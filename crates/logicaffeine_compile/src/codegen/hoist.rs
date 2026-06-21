//! O1 — borrow hoisting (scoped slice extraction).
//!
//! A loop body that indexes `LogosSeq` handles pays one `RefCell` flag
//! operation per access. When the handles provably keep their identity for
//! the whole loop, the borrow moves to a scope around the loop and the body
//! indexes plain slices:
//!
//! ```text
//! {
//!     let __prev_g = prev.borrow();
//!     let prev = &__prev_g[..];
//!     let mut __curr_g = curr.borrow_mut();
//!     let curr = &mut __curr_g[..];
//!     for w in 0..n { curr[w as usize] = prev[w as usize]; }
//! }
//! ```
//!
//! Shadowing the original name and flipping its tracked type to `&[T]` /
//! `&mut [T]` means every existing emission path (indexing, SetIndex, swap
//! fusion, zero-based lowering, length) works unchanged through its slice
//! arms.
//!
//! SOUNDNESS. A mut-hoisted handle holds a `RefMut` across the whole loop;
//! a shared-hoisted handle holds a `Ref`. Any other access to the same
//! allocation while one is held panics at runtime. A handle is therefore
//! hoisted only when ALL of these hold:
//!
//!  - its tracked type is `LogosSeq<T>` (Maps, Strings, zone arenas, and
//!    already-sliced params are out);
//!  - the body neither rebinds, redeclares, resizes, nor pops it;
//!  - it never appears as a bare value (pushed into a container, passed to
//!    a call, given away, shown whole, interpolated, field-stored);
//!  - the body contains no calls and no opaque/concurrent statements
//!    (anything could touch any handle behind them);
//!  - the alias oracle PROVES it distinct, at this loop's invariant, from
//!    every other handle the loop touches (for mut hoists), or from every
//!    handle the loop mutates (for shared hoists). Read-read aliasing is
//!    RefCell-legal and allowed.
//!
//! Refusal is always the default: no oracle, no snapshot, an unknown
//! statement kind, an unknown expression kind — hoist nothing.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use crate::analysis::types::RustNames;
use crate::ast::stmt::{Expr, Stmt};
use crate::intern::{Interner, Symbol};

use super::context::RefinementContext;
use super::peephole::body_modifies_var;

thread_local! {
    /// Per-thread test override for the borrow-hoist kill switch. Tests set
    /// this on their own thread (race-free), unlike the process-global
    /// `LOGOS_HOIST` env var.
    static FORCE_DISABLE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Force borrow hoisting off (or back on) for the current thread only.
/// Used by tests to verify the kill switch without the env-var data race.
pub fn force_disable_for_test(disabled: bool) {
    FORCE_DISABLE.with(|c| c.set(disabled));
}

/// Is borrow hoisting disabled? True when the thread-local test override is
/// set, or the operational `LOGOS_HOIST=0` env var is present.
pub fn hoisting_disabled() -> bool {
    FORCE_DISABLE.with(|c| c.get())
        || std::env::var("LOGOS_HOIST").map(|v| v == "0").unwrap_or(false)
}

/// Could a value of this tracked type share a Seq's `Rc<RefCell>` — i.e.
/// could it conflict with a hoisted Seq borrow? Only Seq-shaped types can;
/// known scalars, strings, and maps cannot. Unknown types are treated
/// conservatively as "could" so an untyped handle never escapes the check.
fn could_alias_seq(ty: Option<&String>) -> bool {
    let t = match ty {
        Some(t) => t,
        None => return true,
    };
    let base = t.split("|__hl:").next().unwrap_or(t.as_str());
    let definitely_not = matches!(
        base,
        "i64" | "f64" | "bool" | "char" | "u8" | "usize" | "i32" | "u64"
            | "String" | "&str" | "()" | "__zero_based_i64" | "__single_char_u8"
    ) || base.starts_with("LogosMap")
        || base.starts_with("LogosI64Map")
        || base.starts_with("LogosI64Set")
        || base.starts_with("HashMap")
        || base.starts_with("FxHashMap")
        || base.starts_with("std::collections::HashMap")
        || base.starts_with("rustc_hash::FxHashMap");
    !definitely_not
}

/// How a handle is hoisted out of the loop.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum HoistKind {
    /// Read-only: `let __g = x.borrow(); let x = &__g[..];` (`&[T]`).
    Shared,
    /// Indexed read/write, never resized: `&mut __g[..]` (`&mut [T]`).
    MutSlice,
    /// Resized (pushed/popped): hold the `RefMut` as a `&mut Vec` so push
    /// goes through it — `let mut __g = x.borrow_mut(); let x = &mut *__g;`.
    MutVec,
}

/// One handle's hoist decision.
pub(crate) struct HoistEntry {
    pub sym: Symbol,
    pub kind: HoistKind,
    pub elem_ty: String,
    pub old_type: String,
    /// True for a de-Rc'd `Vec<T>`: extract the noalias slice WITHOUT a
    /// `.borrow()` (there is no `RefCell`). The slice still matters — it hands
    /// LLVM the same `&[T]`/`&mut [T]` noalias the LogosSeq path gives, which
    /// is what unlocks bounds-check elision and vectorization.
    pub is_vec: bool,
}

/// A pure scalar builtin that maps to a Rust method call (sqrt, abs, …) —
/// takes scalars, returns a scalar, never touches a Seq's `RefCell`. The
/// name+arity pairs mirror the dispatch in codegen/expr.rs, so a body that
/// only calls these can still be hoisted. Anything else is opaque and bails.
fn is_pure_scalar_builtin(name: &str, argc: usize) -> bool {
    matches!(
        (name, argc),
        ("sqrt", 1) | ("abs", 1) | ("floor", 1) | ("ceil", 1) | ("round", 1)
            | ("pow", 2) | ("min", 2) | ("max", 2)
    )
}

struct AccessRoles<'i> {
    /// Read through a collection position (Index/Slice/Length/Contains).
    read: HashSet<Symbol>,
    /// Written through SetIndex.
    written: HashSet<Symbol>,
    /// Resized in place (Push/Pop/Add/Remove collection target). Hoistable
    /// as a held `&mut Vec` (push goes through the RefMut) when not also
    /// leaked.
    resized: HashSet<Symbol>,
    /// Leaked: bare-value uses, call args, whole-handle Show/Give/Return,
    /// pushed-as-a-value into another collection, Repeat iterables. Never
    /// hoistable; mut-hoists must be proven distinct from these too.
    other: HashSet<Symbol>,
    /// Symbols (re)bound inside the body: `Let`, `Pop into`, Repeat
    /// patterns, Inspect bindings. A guard for these would capture a stale
    /// or undeclared name.
    rebound: HashSet<Symbol>,
    /// Insertion order of first sighting, for deterministic emission.
    order: Vec<Symbol>,
    /// Something opaque appeared — hoist nothing in this loop.
    bail: bool,
    /// For resolving called-function names against the builtin whitelist.
    interner: &'i Interner,
}

impl<'i> AccessRoles<'i> {
    fn new(interner: &'i Interner) -> Self {
        AccessRoles {
            read: HashSet::new(),
            written: HashSet::new(),
            resized: HashSet::new(),
            other: HashSet::new(),
            rebound: HashSet::new(),
            order: Vec::new(),
            bail: false,
            interner,
        }
    }

    fn note(&mut self, sym: Symbol) {
        if !self.order.contains(&sym) {
            self.order.push(sym);
        }
    }
    fn read(&mut self, sym: Symbol) {
        self.note(sym);
        self.read.insert(sym);
    }
    fn written(&mut self, sym: Symbol) {
        self.note(sym);
        self.written.insert(sym);
    }
    fn resized(&mut self, sym: Symbol) {
        self.note(sym);
        self.resized.insert(sym);
    }
    fn other(&mut self, sym: Symbol) {
        self.note(sym);
        self.other.insert(sym);
    }
}

/// Walk an expression in VALUE position: collection positions of the
/// indexing forms count as reads; any bare identifier is an `other` use
/// (it may flow anywhere); opaque forms bail.
fn scan_value_expr(e: &Expr, roles: &mut AccessRoles) {
    match e {
        Expr::Identifier(s) => roles.other(*s),
        Expr::Literal(_) | Expr::OptionNone => {}
        Expr::Index { collection, index } => {
            scan_collection_pos(collection, roles);
            scan_value_expr(index, roles);
        }
        Expr::Slice { collection, start, end } => {
            scan_collection_pos(collection, roles);
            scan_value_expr(start, roles);
            scan_value_expr(end, roles);
        }
        Expr::Length { collection } => scan_collection_pos(collection, roles),
        Expr::Contains { collection, value } => {
            scan_collection_pos(collection, roles);
            scan_value_expr(value, roles);
        }
        Expr::BinaryOp { left, right, .. } => {
            scan_value_expr(left, roles);
            scan_value_expr(right, roles);
        }
        Expr::Not { operand } => scan_value_expr(operand, roles),
        Expr::Range { start, end } => {
            scan_value_expr(start, roles);
            scan_value_expr(end, roles);
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for it in items {
                scan_value_expr(it, roles);
            }
        }
        Expr::New { init_fields, .. } => {
            for (_, v) in init_fields {
                scan_value_expr(v, roles);
            }
        }
        Expr::NewVariant { fields, .. } => {
            for (_, v) in fields {
                scan_value_expr(v, roles);
            }
        }
        Expr::FieldAccess { object, .. } => scan_value_expr(object, roles),
        Expr::Copy { expr } => scan_value_expr(expr, roles),
        Expr::WithCapacity { value, capacity } => {
            scan_value_expr(value, roles);
            scan_value_expr(capacity, roles);
        }
        Expr::OptionSome { value } => scan_value_expr(value, roles),
        Expr::Give { value } => scan_value_expr(value, roles),
        Expr::InterpolatedString(parts) => {
            for p in parts {
                if let crate::ast::stmt::StringPart::Expr { value, .. } = p {
                    scan_value_expr(value, roles);
                }
            }
        }
        Expr::Union { left, right } | Expr::Intersection { left, right } => {
            scan_value_expr(left, roles);
            scan_value_expr(right, roles);
        }
        Expr::Call { function, args } => {
            // A pure scalar builtin (sqrt/abs/min/…) can't touch a Seq's
            // RefCell — scan its args for index reads and carry on. Any
            // other call may alias or mutate any handle: bail.
            if is_pure_scalar_builtin(roles.interner.resolve(*function), args.len()) {
                for a in args {
                    scan_value_expr(a, roles);
                }
            } else {
                roles.bail = true;
            }
        }
        // Closures, escapes, zone accessors, opaque calls — bail.
        _ => roles.bail = true,
    }
}

/// A collection position: a bare identifier here is a READ borrow; any
/// other expression is scanned as a value.
fn scan_collection_pos(e: &Expr, roles: &mut AccessRoles) {
    if let Expr::Identifier(s) = e {
        roles.read(*s);
    } else {
        scan_value_expr(e, roles);
    }
}

fn scan_stmts(stmts: &[Stmt], roles: &mut AccessRoles) {
    for stmt in stmts {
        if roles.bail {
            return;
        }
        match stmt {
            Stmt::Let { var, value, .. } => {
                roles.rebound.insert(*var);
                scan_value_expr(value, roles);
            }
            Stmt::Set { target, value } => {
                // The rebind itself is caught by body_modifies_var; the
                // RHS may leak a handle.
                let _ = target;
                scan_value_expr(value, roles);
            }
            Stmt::SetIndex { collection, index, value } => {
                if let Expr::Identifier(s) = collection {
                    roles.written(*s);
                } else {
                    scan_value_expr(collection, roles);
                }
                scan_value_expr(index, roles);
                scan_value_expr(value, roles);
            }
            Stmt::Push { value, collection }
            | Stmt::Add { value, collection }
            | Stmt::Remove { value, collection } => {
                // The pushed VALUE may leak a handle (a Seq pushed into a
                // Seq-of-Seq escapes); the COLLECTION is resized in place.
                scan_value_expr(value, roles);
                if let Expr::Identifier(s) = collection {
                    roles.resized(*s);
                } else {
                    scan_value_expr(collection, roles);
                }
            }
            Stmt::Pop { collection, into } => {
                if let Expr::Identifier(s) = collection {
                    roles.resized(*s);
                } else {
                    scan_value_expr(collection, roles);
                }
                if let Some(v) = into {
                    roles.rebound.insert(*v);
                }
            }
            Stmt::If { cond, then_block, else_block } => {
                scan_value_expr(cond, roles);
                scan_stmts(then_block, roles);
                if let Some(eb) = else_block {
                    scan_stmts(eb, roles);
                }
            }
            Stmt::While { cond, body, .. } => {
                scan_value_expr(cond, roles);
                scan_stmts(body, roles);
            }
            Stmt::Repeat { pattern, iterable, body } => {
                if let Expr::Identifier(s) = iterable {
                    roles.other(*s);
                } else {
                    scan_value_expr(iterable, roles);
                }
                if let crate::ast::stmt::Pattern::Identifier(s) = pattern {
                    roles.rebound.insert(*s);
                }
                scan_stmts(body, roles);
            }
            Stmt::Show { object, recipient } => {
                // Indexed reads are fine; showing a whole handle is opaque
                // (display borrows it in library code).
                scan_value_expr(object, roles);
                scan_value_expr(recipient, roles);
            }
            Stmt::Return { value } => {
                if let Some(v) = value {
                    scan_value_expr(v, roles);
                }
            }
            Stmt::Break => {}
            Stmt::SetField { object, value, .. } => {
                scan_value_expr(object, roles);
                scan_value_expr(value, roles);
            }
            Stmt::Inspect { target, arms, .. } => {
                scan_value_expr(target, roles);
                for arm in arms {
                    for (_, binding) in &arm.bindings {
                        roles.rebound.insert(*binding);
                    }
                    scan_stmts(arm.body, roles);
                }
            }
            Stmt::RuntimeAssert { condition } => scan_value_expr(condition, roles),
            // Calls, concurrency, zones, escapes, IO — opaque: bail.
            _ => roles.bail = true,
        }
    }
}

/// Decide which handles to hoist around this loop. Empty when anything is
/// uncertain.
pub(crate) fn plan_borrow_hoist<'a>(
    loop_stmt: &Stmt<'a>,
    cond: Option<&Expr<'a>>,
    body: &[Stmt<'a>],
    ctx: &RefinementContext<'a>,
    interner: &Interner,
) -> Vec<HoistEntry> {
    // The kill switch (env var or test thread-local) nulls the oracle in
    // codegen_program, so a present oracle already means hoisting is on.
    let oracle = match ctx.oracle() {
        Some(o) => o,
        None => return Vec::new(),
    };

    let mut roles = AccessRoles::new(interner);
    if let Some(c) = cond {
        scan_value_expr(c, &mut roles);
    }
    scan_stmts(body, &mut roles);
    if roles.bail {
        return Vec::new();
    }

    // Candidates: handles accessed through collection positions whose
    // identity provably survives the loop. A resized (pushed/popped) handle
    // becomes a held `&mut Vec` (MutVec); a written-but-not-resized handle a
    // `&mut [T]` (MutSlice); a read-only handle a `&[T]` (Shared).
    let types = ctx.get_variable_types();
    let mut hoisted: Vec<(Symbol, HoistKind, String, String)> = Vec::new();
    // De-Rc'd `Vec<T>` handles are owned and provably non-aliased (that is the
    // de-Rc precondition), so they bypass the alias gating and always get the
    // noalias slice extraction.
    let mut derc_hoisted: Vec<(Symbol, HoistKind, String, String)> = Vec::new();
    for sym in &roles.order {
        let sym = *sym;
        if roles.other.contains(&sym) || roles.rebound.contains(&sym) {
            continue;
        }
        let resized = roles.resized.contains(&sym);
        let written = roles.written.contains(&sym);
        let read = roles.read.contains(&sym);
        if !resized && !written && !read {
            continue;
        }
        let full_ty = match types.get(&sym) {
            Some(t) => t.clone(),
            None => continue,
        };
        // A length-hoisted handle (`|__hl:` sentinel) already has a `_len`
        // binding the loop uses; shadowing it as a slice would orphan that
        // binding. Leave such handles per-access.
        if full_ty.contains("|__hl:") {
            continue;
        }
        let base_ty = full_ty.split("|__hl:").next().unwrap_or(&full_ty);
        // A LogosSeq hoists with a `.borrow()`; a de-Rc'd `Vec<T>` hoists the
        // slice directly (no borrow) — both give LLVM the noalias `&[T]`.
        // `is_de_rc` is authoritative: the registered type string can lag a
        // de-Rc'd decl, but the emitted variable is the `Vec`.
        let elem_ty = if let Some(e) = base_ty.strip_prefix("LogosSeq<").and_then(|s| s.strip_suffix('>')) {
            e.to_string()
        } else if let Some(e) = base_ty.strip_prefix("Vec<").and_then(|s| s.strip_suffix('>')) {
            e.to_string()
        } else {
            continue;
        };
        let is_vec = ctx.is_de_rc(sym) || base_ty.starts_with("Vec<");
        // A rebind (`Set x to …`) inside the body makes the held borrow
        // stale; resize (push/pop) is fine — that's exactly the MutVec case.
        if body_modifies_var(body, sym) {
            continue;
        }
        let kind = if resized {
            HoistKind::MutVec
        } else if written {
            HoistKind::MutSlice
        } else {
            HoistKind::Shared
        };
        if is_vec {
            derc_hoisted.push((sym, kind, elem_ty, full_ty));
        } else {
            hoisted.push((sym, kind, elem_ty, full_ty));
        }
    }

    // Alias gating to a fixpoint: a dropped handle becomes an unhoisted
    // per-access toucher, which can invalidate remaining ones.
    //
    // Only handles that could share a Seq's `Rc<RefCell>` matter — a scalar,
    // String, or Map can never conflict with a Seq borrow, and demanding the
    // oracle prove `arr ≠ i` for a scalar `i` would needlessly require a loop
    // snapshot. Any REAL aliaser is Seq-typed (codegen propagates the type
    // through `Let`/`Set`, the oracle tracks the edge), so restricting the
    // distinctness checks to Seq-typed touched symbols is sound.
    let all_touched: Vec<Symbol> = roles
        .order
        .iter()
        .copied()
        .filter(|s| could_alias_seq(types.get(s)))
        .collect();
    loop {
        let hoisted_syms: HashSet<Symbol> = hoisted.iter().map(|(s, _, _, _)| *s).collect();
        let mut_hoisted: Vec<Symbol> = hoisted
            .iter()
            .filter(|(_, k, _, _)| *k != HoistKind::Shared)
            .map(|(s, _, _, _)| *s)
            .collect();
        let unhoisted_mut: Vec<Symbol> = all_touched
            .iter()
            .filter(|s| {
                !hoisted_syms.contains(s)
                    && (roles.written.contains(s)
                        || roles.resized.contains(s)
                        || roles.other.contains(s))
            })
            .cloned()
            .collect();
        let keep: Vec<bool> = hoisted
            .iter()
            .map(|(sym, kind, _, _)| {
                if *kind != HoistKind::Shared {
                    // A RefMut must be alone: distinct from every other
                    // touched handle, hoisted or not.
                    all_touched
                        .iter()
                        .filter(|t| **t != *sym)
                        .all(|t| oracle.loop_handles_definitely_distinct(loop_stmt, *sym, *t))
                } else {
                    // A Ref tolerates other Refs; it must be distinct from
                    // every mut-hoisted handle and every unhoisted mutator.
                    mut_hoisted
                        .iter()
                        .filter(|s| **s != *sym)
                        .all(|s| oracle.loop_handles_definitely_distinct(loop_stmt, *sym, *s))
                        && unhoisted_mut
                            .iter()
                            .all(|t| oracle.loop_handles_definitely_distinct(loop_stmt, *sym, *t))
                }
            })
            .collect();
        if keep.iter().all(|k| *k) {
            break;
        }
        let mut it = keep.into_iter();
        hoisted.retain(|_| it.next().unwrap());
    }

    derc_hoisted
        .into_iter()
        .map(|(sym, kind, elem_ty, old_type)| HoistEntry { sym, kind, elem_ty, old_type, is_vec: true })
        .chain(
            hoisted
                .into_iter()
                .map(|(sym, kind, elem_ty, old_type)| HoistEntry { sym, kind, elem_ty, old_type, is_vec: false }),
        )
        .collect()
}

/// Open the hoist scope: guards, shadows, and the slice type flips.
/// Returns nothing to restore beyond what the entries carry — call
/// [`emit_hoist_close`] with the same entries on the way out.
pub(crate) fn emit_hoist_open(
    entries: &[HoistEntry],
    interner: &Interner,
    indent_str: &str,
    ctx: &mut RefinementContext,
    output: &mut String,
) {
    if entries.is_empty() {
        return;
    }
    let names = RustNames::new(interner);
    writeln!(output, "{}{{", indent_str).unwrap();
    for e in entries {
        let n = names.ident(e.sym);
        match (e.kind, e.is_vec) {
            // LogosSeq: a `.borrow()` guard + slice shadow.
            (HoistKind::Shared, false) => {
                writeln!(output, "{}    let __{}_g = {}.borrow();", indent_str, n, n).unwrap();
                writeln!(output, "{}    let {} = &__{}_g[..];", indent_str, n, n).unwrap();
                ctx.register_variable_type(e.sym, format!("&[{}]", e.elem_ty));
            }
            (HoistKind::MutSlice, false) => {
                writeln!(output, "{}    let mut __{}_g = {}.borrow_mut();", indent_str, n, n).unwrap();
                writeln!(output, "{}    let {} = &mut __{}_g[..];", indent_str, n, n).unwrap();
                ctx.register_variable_type(e.sym, format!("&mut [{}]", e.elem_ty));
            }
            (HoistKind::MutVec, false) => {
                // Hold the RefMut as a `&mut Vec` so push/pop go through it
                // without a per-call borrow. Registered as `Vec<T>` to reuse
                // the existing owned-Vec emission (index, push, length all
                // auto-deref through the `&mut Vec` shadow).
                writeln!(output, "{}    let mut __{}_g = {}.borrow_mut();", indent_str, n, n).unwrap();
                writeln!(output, "{}    let {} = &mut *__{}_g;", indent_str, n, n).unwrap();
                ctx.register_variable_type(e.sym, format!("Vec<{}>", e.elem_ty));
            }
            // De-Rc'd Vec: the SAME noalias slice, but no `.borrow()`.
            (HoistKind::Shared, true) => {
                writeln!(output, "{}    let {} = &{}[..];", indent_str, n, n).unwrap();
                ctx.register_variable_type(e.sym, format!("&[{}]", e.elem_ty));
            }
            (HoistKind::MutSlice, true) => {
                writeln!(output, "{}    let {} = &mut {}[..];", indent_str, n, n).unwrap();
                ctx.register_variable_type(e.sym, format!("&mut [{}]", e.elem_ty));
            }
            (HoistKind::MutVec, true) => {
                // Resized in the loop — keep it a `&mut Vec` (a slice can't
                // push) but reborrow so LLVM gets the `&mut` noalias. The
                // binding is `mut` so a NESTED loop can reborrow it again
                // (`let mut a = &mut a;` then inner `let mut a = &mut a;`).
                writeln!(output, "{}    let mut {} = &mut {};", indent_str, n, n).unwrap();
                ctx.register_variable_type(e.sym, format!("Vec<{}>", e.elem_ty));
            }
        }
    }
}

/// Close the hoist scope and restore the handles' tracked types.
pub(crate) fn emit_hoist_close(
    entries: &[HoistEntry],
    indent_str: &str,
    ctx: &mut RefinementContext,
    output: &mut String,
) {
    if entries.is_empty() {
        return;
    }
    for e in entries {
        ctx.register_variable_type(e.sym, e.old_type.clone());
    }
    writeln!(output, "{}}}", indent_str).unwrap();
}
