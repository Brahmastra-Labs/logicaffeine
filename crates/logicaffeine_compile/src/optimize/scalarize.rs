//! Fixed-size float/int `Seq` scalarization for the RUN path (the interpreter's
//! analog of LLVM's SROA).
//!
//! Motivation (nbody): the kernel keeps its five bodies in seven fixed-size
//! (length-5) `Seq of Float`s, each element read as `item k of arr` — a
//! bounds-checked, reference-counted heap load. After the run-path unroller
//! turns every index into a compile-time constant, those Seqs are touched only
//! at constant offsets, so they carry no information the array representation
//! adds: each element is just a named scalar. This pass replaces such a Seq with
//! `N` scalar locals (`arr$1 … arr$N`), turning every `item k of arr` into a
//! plain variable read the JIT's register allocator can keep in an XMM register.
//! Unrolling alone was a measured loss (a bigger region over the same heap
//! loads); scalarize is the half that pays — it removes the loads.
//!
//! **Conservative by construction.** It reuses `collect_scalarizable_seqs`,
//! which already proves a Seq is push-built, fixed-size, never resized, aliased,
//! escaped, or passed anywhere — its only uses are index reads, `SetIndex`
//! writes, length queries, and its initial pushes. On top of that this pass adds
//! the scalarization-specific requirement that *every* index is a CONSTANT in
//! `1..=N` and that the Seq's length is never queried (a scalar set has no
//! length). Any Seq that fails either check is left entirely untouched.
//!
//! **Value-preserving.** Element `k` (1-based) maps to the value of the `k`-th
//! `Push v to arr` — exactly the position the 1-based `item k of arr` would
//! read. No arithmetic is reassociated; reads and writes keep their order.
//!
//! **A swap exchanges elements.** `Swap a and b.` of two scalarized arrays of one
//! length becomes `Swap a$k and b$k.` for every `k`, the same reversible register
//! exchange on each element.
//!
//! **No array survives its rewrite.** After rewriting, the optimizer's exhaustive
//! walk ([`super::Mentions`]) must find no scalarized array's name anywhere in the
//! program. A form this rewriter has no case for, or a swap whose partner was not
//! scalarized (its length is queried, or it is read at a computed position),
//! leaves the name behind; that array is then kept as a `Seq`, and the rewrite is
//! redone without it until nothing is left behind.

use std::collections::{HashMap, HashSet};

use crate::arena::Arena;
use crate::ast::stmt::{Expr, MatchArm, Stmt};
use crate::codegen::detection::collect_scalarizable_seqs;
use crate::intern::{Interner, Symbol};

use super::Mentions;

/// Replace qualifying fixed-size `Seq` locals with `N` scalar variables.
/// Returns the (possibly) rewritten statements and whether anything changed.
/// When nothing qualifies the ORIGINAL statements are returned untouched, so
/// the pass is a guaranteed no-op on programs without a scalarizable Seq.
pub fn scalarize_seqs<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    let (out, changed, _left_behind) = scalarize_reporting(stmts, expr_arena, stmt_arena, interner);
    (out, changed)
}

/// [`scalarize_seqs`], also naming the arrays the check after the rewrite sent back to being a `Seq`.
fn scalarize_reporting<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool, HashSet<Symbol>) {
    let cands = collect_scalarizable_seqs(&stmts, interner);
    if cands.is_empty() {
        return (stmts, false, HashSet::new());
    }

    // Among the detection candidates keep only those whose EVERY index is a
    // constant in range and whose length is never queried — the extra
    // requirements scalarization (but not the `[T;N]` codegen) imposes.
    let mut disqualified: HashSet<Symbol> = HashSet::new();
    let lens: HashMap<Symbol, usize> = cands.iter().map(|(s, c)| (*s, c.len)).collect();
    scan_index_constraints(&stmts, &lens, &mut disqualified);

    let qualified: HashMap<Symbol, usize> = lens
        .iter()
        .filter(|(s, _)| !disqualified.contains(s))
        .map(|(s, n)| (*s, *n))
        .collect();
    rewrite_until_clean(stmts, qualified, expr_arena, stmt_arena, interner)
}

/// Rewrite `qualified`'s arrays into scalars, then check the result: an array whose name the rewrite left anywhere
/// in the program is kept as a `Seq`, and the rewrite is redone from the original statements without it. Returns
/// the program, whether anything was scalarized, and the arrays the check sent back.
fn rewrite_until_clean<'a>(
    stmts: Vec<Stmt<'a>>,
    mut qualified: HashMap<Symbol, usize>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool, HashSet<Symbol>) {
    let mut left_behind: HashSet<Symbol> = HashSet::new();
    loop {
        if qualified.is_empty() {
            return (stmts, false, left_behind);
        }
        // Intern the `arr$k` scalar names (1-based, matching `item k of arr`).
        let mut scalar_names: HashMap<Symbol, Vec<Symbol>> = HashMap::new();
        for (&sym, &n) in &qualified {
            let base = interner.resolve(sym).to_string();
            let names: Vec<Symbol> = (1..=n)
                .map(|k| interner.intern(&format!("{base}${k}")))
                .collect();
            scalar_names.insert(sym, names);
        }
        let out = rewrite_block(
            stmts.clone(),
            &qualified,
            &scalar_names,
            expr_arena,
            stmt_arena,
        );
        let residue = Mentions::of_block(&out);
        let survivors: Vec<Symbol> = qualified
            .keys()
            .copied()
            .filter(|s| residue.touches(interner.resolve(*s), interner))
            .collect();
        if survivors.is_empty() {
            return (out, true, left_behind);
        }
        for s in survivors {
            qualified.remove(&s);
            left_behind.insert(s);
        }
    }
}

/// The element `k` (1-based) of `arr` is defined by the `k`-th `Push v to arr`.
/// `push_counter` tracks, per array, how many pushes have been rewritten so far
/// so the next push targets the next scalar.
struct Rewriter<'a, 'q> {
    qualified: &'q HashMap<Symbol, usize>,
    scalar_names: &'q HashMap<Symbol, Vec<Symbol>>,
    push_counter: HashMap<Symbol, usize>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
}

fn rewrite_block<'a>(
    stmts: Vec<Stmt<'a>>,
    qualified: &HashMap<Symbol, usize>,
    scalar_names: &HashMap<Symbol, Vec<Symbol>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
) -> Vec<Stmt<'a>> {
    let mut rw = Rewriter {
        qualified,
        scalar_names,
        push_counter: HashMap::new(),
        expr_arena,
        stmt_arena,
    };
    rw.rewrite_stmts(stmts)
}

impl<'a, 'q> Rewriter<'a, 'q> {
    fn rewrite_stmts(&mut self, stmts: Vec<Stmt<'a>>) -> Vec<Stmt<'a>> {
        let mut out: Vec<Stmt<'a>> = Vec::with_capacity(stmts.len());
        for stmt in stmts {
            self.rewrite_stmt(stmt, &mut out);
        }
        out
    }

    fn rewrite_block_ref(&mut self, block: &'a [Stmt<'a>]) -> &'a [Stmt<'a>] {
        let v = self.rewrite_stmts(block.to_vec());
        self.stmt_arena.alloc_slice(v)
    }

    fn rewrite_stmt(&mut self, stmt: Stmt<'a>, out: &mut Vec<Stmt<'a>>) {
        match stmt {
            // `Let mutable arr be a new Seq …` is dropped: the element scalars
            // are introduced directly at the array's pushes (below), so the
            // empty-Seq declaration has nothing left to bind.
            Stmt::Let { var, .. } if self.qualified.contains_key(&var) => {}
            // `Push v to arr` → `Let mutable arr$k be v` (k = push order). The
            // detection proves every read of element k follows its push, so the
            // defining `Let` always precedes — element k is in scope and carries
            // its real initial value (no separate placeholder/`Set`, which a
            // later CSE pass over the now-scalar reads would value-number against
            // its initializer and fold across a loop-carried mutation).
            Stmt::Push { value, collection } if matches!(collection, Expr::Identifier(s) if self.qualified.contains_key(s)) =>
            {
                let sym = match collection {
                    Expr::Identifier(s) => *s,
                    _ => unreachable!(),
                };
                let k = self.push_counter.entry(sym).or_insert(0);
                let target = self.scalar_names[&sym][*k];
                *k += 1;
                let rewritten = self.rewrite_expr(value);
                out.push(Stmt::Let {
                    var: target,
                    ty: None,
                    value: rewritten,
                    mutable: true,
                });
            }
            // `Set item k of arr to x` → `Set arr$k to x`.
            Stmt::SetIndex {
                collection,
                index,
                value,
            } if self.scalar_target(collection, index).is_some() => {
                let target = self.scalar_target(collection, index).unwrap();
                let rewritten = self.rewrite_expr(value);
                out.push(Stmt::Set {
                    target,
                    value: rewritten,
                });
            }
            // `Swap a and b` of two scalarized arrays of one length → `Swap a$k and b$k`
            // for every element; a swap of an array with itself changes nothing.
            Stmt::Swap { a, b }
                if self.qualified.contains_key(&a) && self.qualified.contains_key(&b) =>
            {
                if a != b && self.qualified[&a] == self.qualified[&b] {
                    let pairs = self.scalar_names[&a].iter().zip(&self.scalar_names[&b]);
                    out.extend(pairs.map(|(x, y)| Stmt::Swap { a: *x, b: *y }));
                } else if a != b {
                    out.push(Stmt::Swap { a, b });
                }
            }
            Stmt::Splice { body } => {
                out.push(Stmt::Splice {
                    body: self.rewrite_block_ref(body),
                });
            }
            Stmt::SetField {
                object,
                field,
                value,
            } => {
                out.push(Stmt::SetField {
                    object: self.rewrite_expr(object),
                    field,
                    value: self.rewrite_expr(value),
                });
            }
            Stmt::Give { object, recipient } => {
                out.push(Stmt::Give {
                    object: self.rewrite_expr(object),
                    recipient: self.rewrite_expr(recipient),
                });
            }
            Stmt::Let {
                var,
                ty,
                value,
                mutable,
            } => {
                out.push(Stmt::Let {
                    var,
                    ty,
                    value: self.rewrite_expr(value),
                    mutable,
                });
            }
            Stmt::Set { target, value } => {
                out.push(Stmt::Set {
                    target,
                    value: self.rewrite_expr(value),
                });
            }
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => {
                out.push(Stmt::SetIndex {
                    collection: self.rewrite_expr(collection),
                    index: self.rewrite_expr(index),
                    value: self.rewrite_expr(value),
                });
            }
            Stmt::Push { value, collection } => {
                out.push(Stmt::Push {
                    value: self.rewrite_expr(value),
                    collection: self.rewrite_expr(collection),
                });
            }
            Stmt::Show { object, recipient } => {
                out.push(Stmt::Show {
                    object: self.rewrite_expr(object),
                    recipient: self.rewrite_expr(recipient),
                });
            }
            Stmt::Return { value } => {
                out.push(Stmt::Return {
                    value: value.map(|v| self.rewrite_expr(v)),
                });
            }
            Stmt::RuntimeAssert {
                condition,
                hard,
                dim,
                reason,
            } => {
                let reason = reason.map(|r| self.rewrite_expr(r));
                out.push(Stmt::RuntimeAssert {
                    condition: self.rewrite_expr(condition),
                    hard,
                    dim,
                    reason,
                });
            }
            Stmt::Call { function, args } => {
                let args = args.into_iter().map(|a| self.rewrite_expr(a)).collect();
                out.push(Stmt::Call { function, args });
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                out.push(Stmt::If {
                    cond: self.rewrite_expr(cond),
                    then_block: self.rewrite_block_ref(then_block),
                    else_block: else_block.map(|b| self.rewrite_block_ref(b)),
                });
            }
            Stmt::While {
                cond,
                body,
                decreasing,
            } => {
                out.push(Stmt::While {
                    cond: self.rewrite_expr(cond),
                    body: self.rewrite_block_ref(body),
                    decreasing: decreasing.map(|d| self.rewrite_expr(d)),
                });
            }
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => {
                out.push(Stmt::Repeat {
                    pattern,
                    iterable: self.rewrite_expr(iterable),
                    body: self.rewrite_block_ref(body),
                });
            }
            Stmt::Inspect {
                target,
                arms,
                has_otherwise,
            } => {
                let arms = arms
                    .into_iter()
                    .map(|a| MatchArm {
                        enum_name: a.enum_name,
                        variant: a.variant,
                        bindings: a.bindings,
                        body: self.rewrite_block_ref(a.body),
                    })
                    .collect();
                out.push(Stmt::Inspect {
                    target: self.rewrite_expr(target),
                    arms,
                    has_otherwise,
                });
            }
            Stmt::Zone {
                name,
                capacity,
                source_file,
                body,
            } => {
                out.push(Stmt::Zone {
                    name,
                    capacity,
                    source_file,
                    body: self.rewrite_block_ref(body),
                });
            }
            other => out.push(other),
        }
    }

    /// If `collection`/`index` is a constant index `item k of arr` on a
    /// qualified array, the target scalar `arr$k`; else None.
    fn scalar_target(&self, collection: &Expr<'a>, index: &Expr<'a>) -> Option<Symbol> {
        let sym = match collection {
            Expr::Identifier(s) if self.qualified.contains_key(s) => *s,
            _ => return None,
        };
        let k = const_index(index)?;
        let n = self.qualified[&sym];
        if k >= 1 && k <= n as i64 {
            Some(self.scalar_names[&sym][(k - 1) as usize])
        } else {
            None
        }
    }

    fn rewrite_expr(&self, expr: &'a Expr<'a>) -> &'a Expr<'a> {
        match expr {
            // `item k of arr` (constant k) → `arr$k`.
            Expr::Index { collection, index } => {
                if let Some(target) = self.scalar_target(collection, index) {
                    return self.expr_arena.alloc(Expr::Identifier(target));
                }
                self.expr_arena.alloc(Expr::Index {
                    collection: self.rewrite_expr(collection),
                    index: self.rewrite_expr(index),
                })
            }
            Expr::BinaryOp { op, left, right } => self.expr_arena.alloc(Expr::BinaryOp {
                op: *op,
                left: self.rewrite_expr(left),
                right: self.rewrite_expr(right),
            }),
            Expr::Not { operand } => self.expr_arena.alloc(Expr::Not {
                operand: self.rewrite_expr(operand),
            }),
            Expr::Call { function, args } => self.expr_arena.alloc(Expr::Call {
                function: *function,
                args: args.iter().map(|a| self.rewrite_expr(a)).collect(),
            }),
            Expr::CallExpr { callee, args } => self.expr_arena.alloc(Expr::CallExpr {
                callee: self.rewrite_expr(callee),
                args: args.iter().map(|a| self.rewrite_expr(a)).collect(),
            }),
            Expr::Slice {
                collection,
                start,
                end,
            } => self.expr_arena.alloc(Expr::Slice {
                collection: self.rewrite_expr(collection),
                start: self.rewrite_expr(start),
                end: self.rewrite_expr(end),
            }),
            Expr::Length { collection } => self.expr_arena.alloc(Expr::Length {
                collection: self.rewrite_expr(collection),
            }),
            Expr::Copy { expr } => self.expr_arena.alloc(Expr::Copy {
                expr: self.rewrite_expr(expr),
            }),
            Expr::Give { value } => self.expr_arena.alloc(Expr::Give {
                value: self.rewrite_expr(value),
            }),
            Expr::Contains { collection, value } => self.expr_arena.alloc(Expr::Contains {
                collection: self.rewrite_expr(collection),
                value: self.rewrite_expr(value),
            }),
            Expr::Union { left, right } => self.expr_arena.alloc(Expr::Union {
                left: self.rewrite_expr(left),
                right: self.rewrite_expr(right),
            }),
            Expr::Intersection { left, right } => self.expr_arena.alloc(Expr::Intersection {
                left: self.rewrite_expr(left),
                right: self.rewrite_expr(right),
            }),
            Expr::Range { start, end } => self.expr_arena.alloc(Expr::Range {
                start: self.rewrite_expr(start),
                end: self.rewrite_expr(end),
            }),
            Expr::FieldAccess { object, field } => self.expr_arena.alloc(Expr::FieldAccess {
                object: self.rewrite_expr(object),
                field: *field,
            }),
            Expr::List(items) => self.expr_arena.alloc(Expr::List(
                items.iter().map(|i| self.rewrite_expr(i)).collect(),
            )),
            Expr::Tuple(items) => self.expr_arena.alloc(Expr::Tuple(
                items.iter().map(|i| self.rewrite_expr(i)).collect(),
            )),
            Expr::OptionSome { value } => self.expr_arena.alloc(Expr::OptionSome {
                value: self.rewrite_expr(value),
            }),
            Expr::New {
                type_name,
                type_args,
                init_fields,
            } => self.expr_arena.alloc(Expr::New {
                type_name: *type_name,
                type_args: type_args.clone(),
                init_fields: init_fields
                    .iter()
                    .map(|(f, v)| (*f, self.rewrite_expr(v)))
                    .collect(),
            }),
            Expr::NewVariant {
                enum_name,
                variant,
                fields,
            } => self.expr_arena.alloc(Expr::NewVariant {
                enum_name: *enum_name,
                variant: *variant,
                fields: fields
                    .iter()
                    .map(|(f, v)| (*f, self.rewrite_expr(v)))
                    .collect(),
            }),
            Expr::WithCapacity { value, capacity } => self.expr_arena.alloc(Expr::WithCapacity {
                value: self.rewrite_expr(value),
                capacity: self.rewrite_expr(capacity),
            }),
            Expr::InterpolatedString(parts) => {
                let parts = parts
                    .iter()
                    .map(|p| match p {
                        crate::ast::stmt::StringPart::Expr {
                            value,
                            format_spec,
                            debug,
                        } => crate::ast::stmt::StringPart::Expr {
                            value: self.rewrite_expr(value),
                            format_spec: *format_spec,
                            debug: *debug,
                        },
                        crate::ast::stmt::StringPart::Literal(s) => {
                            crate::ast::stmt::StringPart::Literal(*s)
                        }
                    })
                    .collect();
                self.expr_arena.alloc(Expr::InterpolatedString(parts))
            }
            // Leaves, and forms that hold no qualified index: the detection
            // disqualifies every array a closure, foreign code or a page form
            // touches. Anything left behind is caught by `rewrite_until_clean`.
            other => other,
        }
    }
}

/// The constant value of an index expression (1-based), if it folds to an
/// integer literal — including the `k - 1` shapes the unroller can leave.
fn const_index(expr: &Expr) -> Option<i64> {
    crate::loop_shape::const_eval_i64(expr)
}

// ---------------------------------------------------------------------------
// Scalarization-specific disqualification: non-constant / out-of-range index,
// or any length query.
// ---------------------------------------------------------------------------

fn scan_index_constraints(
    stmts: &[Stmt],
    lens: &HashMap<Symbol, usize>,
    disq: &mut HashSet<Symbol>,
) {
    for s in stmts {
        scan_stmt(s, lens, disq);
    }
}

fn scan_stmt(s: &Stmt, lens: &HashMap<Symbol, usize>, disq: &mut HashSet<Symbol>) {
    match s {
        Stmt::SetIndex {
            collection,
            index,
            value,
        } => {
            check_index(collection, index, lens, disq);
            scan_expr(index, lens, disq);
            scan_expr(value, lens, disq);
        }
        Stmt::Let { value, .. } | Stmt::Set { value, .. } => scan_expr(value, lens, disq),
        Stmt::Push { value, collection } => {
            scan_expr(value, lens, disq);
            scan_expr(collection, lens, disq);
        }
        Stmt::Show { object, recipient } => {
            scan_expr(object, lens, disq);
            scan_expr(recipient, lens, disq);
        }
        Stmt::Give { object, recipient } => {
            scan_expr(object, lens, disq);
            scan_expr(recipient, lens, disq);
        }
        Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
            scan_expr(value, lens, disq);
            scan_expr(collection, lens, disq);
        }
        Stmt::SetField { object, value, .. } => {
            scan_expr(object, lens, disq);
            scan_expr(value, lens, disq);
        }
        Stmt::RuntimeAssert { condition, .. } => scan_expr(condition, lens, disq),
        Stmt::Return { value } => {
            if let Some(v) = value {
                scan_expr(v, lens, disq);
            }
        }
        Stmt::Call { args, .. } => {
            for a in args {
                scan_expr(a, lens, disq);
            }
        }
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            scan_expr(cond, lens, disq);
            scan_index_constraints(then_block, lens, disq);
            if let Some(eb) = else_block {
                scan_index_constraints(eb, lens, disq);
            }
        }
        Stmt::While {
            cond,
            body,
            decreasing,
        } => {
            scan_expr(cond, lens, disq);
            if let Some(d) = decreasing {
                scan_expr(d, lens, disq);
            }
            scan_index_constraints(body, lens, disq);
        }
        Stmt::Repeat { iterable, body, .. } => {
            scan_expr(iterable, lens, disq);
            scan_index_constraints(body, lens, disq);
        }
        Stmt::Inspect { target, arms, .. } => {
            scan_expr(target, lens, disq);
            for a in arms {
                scan_index_constraints(a.body, lens, disq);
            }
        }
        Stmt::Zone { body, .. } | Stmt::Splice { body } => scan_index_constraints(body, lens, disq),
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => {
            scan_index_constraints(tasks, lens, disq)
        }
        _ => {}
    }
}

/// A read/write at `item index of collection`: if `collection` is a candidate
/// array, its index must be a constant in `1..=N`, else disqualify.
fn check_index(
    collection: &Expr,
    index: &Expr,
    lens: &HashMap<Symbol, usize>,
    disq: &mut HashSet<Symbol>,
) {
    if let Expr::Identifier(sym) = collection {
        if let Some(&n) = lens.get(sym) {
            match const_index(index) {
                Some(k) if k >= 1 && k <= n as i64 => {}
                _ => {
                    disq.insert(*sym);
                }
            }
        }
    }
}

fn scan_expr(e: &Expr, lens: &HashMap<Symbol, usize>, disq: &mut HashSet<Symbol>) {
    match e {
        Expr::Index { collection, index } => {
            check_index(collection, index, lens, disq);
            scan_expr(collection, lens, disq);
            scan_expr(index, lens, disq);
        }
        // A scalar set has no length — a `length of arr` query cannot be
        // honored, so any candidate it touches is disqualified.
        Expr::Length { collection } => {
            if let Expr::Identifier(sym) = collection {
                if lens.contains_key(sym) {
                    disq.insert(*sym);
                }
            }
            scan_expr(collection, lens, disq);
        }
        Expr::Slice {
            collection,
            start,
            end,
        } => {
            scan_expr(collection, lens, disq);
            scan_expr(start, lens, disq);
            scan_expr(end, lens, disq);
        }
        Expr::BinaryOp { left, right, .. }
        | Expr::Union { left, right }
        | Expr::Intersection { left, right }
        | Expr::Range {
            start: left,
            end: right,
        } => {
            scan_expr(left, lens, disq);
            scan_expr(right, lens, disq);
        }
        Expr::Contains { collection, value } => {
            scan_expr(collection, lens, disq);
            scan_expr(value, lens, disq);
        }
        Expr::Not { operand } => scan_expr(operand, lens, disq),
        Expr::FieldAccess { object, .. } => scan_expr(object, lens, disq),
        Expr::Copy { expr } => scan_expr(expr, lens, disq),
        Expr::Give { value } | Expr::OptionSome { value } => scan_expr(value, lens, disq),
        Expr::WithCapacity { value, capacity } => {
            scan_expr(value, lens, disq);
            scan_expr(capacity, lens, disq);
        }
        Expr::Call { args, .. } | Expr::CallExpr { args, .. } => {
            for a in args {
                scan_expr(a, lens, disq);
            }
        }
        Expr::List(items) | Expr::Tuple(items) => {
            for i in items {
                scan_expr(i, lens, disq);
            }
        }
        Expr::New {
            init_fields: fields,
            ..
        }
        | Expr::NewVariant { fields, .. } => {
            for (_, v) in fields {
                scan_expr(v, lens, disq);
            }
        }
        Expr::InterpolatedString(parts) => {
            for p in parts {
                if let crate::ast::stmt::StringPart::Expr { value, .. } = p {
                    scan_expr(value, lens, disq);
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/scalarize/tests.rs"]
mod tests;
