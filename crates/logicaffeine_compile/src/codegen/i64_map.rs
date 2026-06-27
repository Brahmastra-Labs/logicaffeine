//! De-Rc for `Map of Int to Int`: select the specialized open-addressing
//! `LogosI64Map` over `LogosMap<i64, i64>` for the maps where it is *provably*
//! safe — non-aliased, non-escaping locals used only through the value-semantic
//! whitelist (construct / insert / get / contains).
//!
//! `LogosMap` is `Rc<RefCell<FxHashMap>>` with reference semantics: a clone
//! shares the table. `LogosI64Map` owns two flat `Vec`s with value semantics.
//! Swapping one for the other is correct ONLY when the program never relies on
//! the shared-reference behaviour — i.e. the map is never aliased, captured,
//! returned, passed, stored, or otherwise allowed to escape its declaring
//! scope. This analysis is the gate that proves that, conservatively: any use
//! it does not recognize as safe disqualifies the map (it stays `LogosMap`).
//!
//! The whitelist of safe positions is exactly three:
//!   * `Stmt::SetIndex { collection: m, .. }`  — insert (`m.insert(k, v)`)
//!   * `Expr::Index { collection: m, .. }`     — get (`m.get(&k)`)
//!   * `Expr::Contains { collection: m, .. }`  — contains (`m.logos_contains(&k)`)
//! plus the defining `Let`. A bare `m` reached in any other position — an
//! assignment RHS (alias), a call argument or `Return` (escape), a `Push`/field
//! store (escape into a container), `length of m`, even a `Show m` — disqualifies
//! it. Exotic statements (concurrency / CRDT / networking / IO) that this pass
//! does not model conservatively disqualify *every* candidate.

use crate::ast::stmt::{Block, ClosureBody, Expr, MatchArm, Stmt, StringPart, TypeExpr};
use super::context::RefinementContext;
use super::types::codegen_type_expr;
use logicaffeine_base::intern::{Interner, Symbol};
use std::collections::{HashMap, HashSet};

/// The Rust type string for a `Map<K, V>` local: the specialized open-addressing
/// `LogosI64Map` when both type arguments are `i64` and the alias analysis
/// cleared `var`, otherwise the reference-semantic `LogosMap<K, V>`. The single
/// decision point shared by every Map type-registration site.
pub(super) fn map_rust_type(k: &str, v: &str, var: Symbol, ctx: &RefinementContext) -> String {
    if k == "i64" && v == "i64" {
        // Densest tier first: a proven bounded key domain → direct-addressed array.
        if let Some(kind) = ctx.dense_kind(var) {
            return dense_type_name(kind).to_string();
        }
        // Then the i32-narrowed hash tier (proven i32-range keys/values).
        if ctx.is_i32_set(var) {
            return "LogosI32Set".to_string();
        }
        if ctx.is_i32_map(var) {
            return "LogosI32Map".to_string();
        }
        if ctx.is_i64_set(var) {
            return "LogosI64Set".to_string();
        }
        if ctx.is_i64_map(var) {
            return "LogosI64Map".to_string();
        }
    }
    format!("LogosMap<{}, {}>", k, v)
}

/// Which direct-addressed representation a proven-dense `Map of Int to Int` lowers
/// to. `Map` keeps a presence bitset (correct `None` for absent in-range keys);
/// `MapNoPresence` is the presence-elided form chosen only when every queried key
/// is proven inserted (`get` is a bare array load); `Set` is the keys-only bitset.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DenseKind {
    Map,
    MapNoPresence,
    Set,
}

/// The proof results for one dense map: its window offset `lo` (so `data[key-lo]`)
/// and which representation to emit. The window width is the map's `with capacity`
/// hint, re-derived at the constructor site from the `WithCapacity` initializer.
#[derive(Clone, Copy, Debug)]
pub struct DenseMapInfo {
    pub lo: i64,
    pub kind: DenseKind,
}

/// The runtime type name for a dense representation.
pub(super) fn dense_type_name(kind: DenseKind) -> &'static str {
    match kind {
        DenseKind::Set => "LogosDenseI64Set",
        DenseKind::MapNoPresence => "LogosDenseI64MapNoPresence",
        DenseKind::Map => "LogosDenseI64Map",
    }
}

/// The set of `Map of Int to Int` locals in `stmts` that can be lowered to
/// `LogosI64Map`. Runs per scope (the top-level body, then each function body
/// separately — it never descends into nested `FunctionDef`s).
pub fn detect_i64_maps(stmts: &[Stmt], interner: &Interner) -> I64MapClasses {
    // Pass 1 — candidates: `Int → Int` maps bound by *exactly one* `Let` in this
    // scope (so a shadowed/rebound name can never be half-converted).
    let mut let_counts: HashMap<Symbol, usize> = HashMap::new();
    let mut int_map_lets: HashSet<Symbol> = HashSet::new();
    collect_candidates(stmts, interner, &mut let_counts, &mut int_map_lets);
    let mut candidates: HashSet<Symbol> = int_map_lets
        .into_iter()
        .filter(|s| let_counts.get(s) == Some(&1))
        .collect();
    if candidates.is_empty() {
        return I64MapClasses::default();
    }

    // Pass 2 — disqualify any candidate used outside the whitelist; record which
    // candidates are inserted into (a read-only map is degenerate, and requiring
    // an insert guarantees the binding is mutated so `let mut` warns nothing);
    // and record which are GOT (read by value via `item k of m`). A qualified
    // candidate that is never GOT is used purely as a set → keys-only.
    let mut walk = Disq {
        candidates: &candidates,
        disq: HashSet::new(),
        inserted: HashSet::new(),
        got: HashSet::new(),
        has_contains: HashSet::new(),
        key_sites: HashMap::new(),
        value_sites: HashMap::new(),
    };
    walk.block(stmts);
    let Disq { disq, inserted, got, has_contains, key_sites, value_sites, .. } = walk;
    candidates.retain(|s| !disq.contains(s) && inserted.contains(s));
    let sets: HashSet<Symbol> = candidates.iter().copied().filter(|s| !got.contains(s)).collect();
    I64MapClasses { maps: candidates, sets, has_contains, key_sites, value_sites }
}

/// The arena address of an expression — the identity the oracle keys its
/// per-occurrence facts by. Used to match a key site against the dense-bound
/// proof; the argument coerces `&&Expr → &Expr` so it yields the inner node's
/// address regardless of match-binding depth (the same value the oracle stored).
fn expr_addr(e: &Expr) -> usize {
    e as *const Expr as usize
}

/// The `Map of Int to Int` locals that lower to a specialized open-addressing
/// representation, split by whether the value is ever read back: `sets` (only
/// `contains` + `insert`, value never read → keys-only `LogosI64Set`) and the
/// rest of `maps` (value read via `item k of m` → `LogosI64Map`). `sets ⊆ maps`.
/// `key_sites` records, per candidate, the arena address of every insert/get/
/// contains KEY expression — the dense gate checks each against the bound proof.
#[derive(Default)]
pub struct I64MapClasses {
    pub maps: HashSet<Symbol>,
    pub sets: HashSet<Symbol>,
    /// Maps with ≥1 `contains` use — ineligible for presence elision (the
    /// presence-free representation cannot answer membership).
    pub has_contains: HashSet<Symbol>,
    pub key_sites: HashMap<Symbol, Vec<usize>>,
    pub value_sites: HashMap<Symbol, Vec<usize>>,
}

/// Select the subset of `classes.maps` whose ENTIRE key domain the oracle proved
/// bounded within the map's `with capacity` hint, so the map lowers to a
/// direct-addressed flat array (`LogosDenseI64Map`/`LogosDenseI64Set`) instead of
/// the open-addressing hash map. A map qualifies iff (a) it has a recorded
/// invariant capacity, (b) it has ≥1 key site, and (c) EVERY key site was proven
/// `0 <= key <= capacity` (`dense_map_key_proven`). Any unproven or unrecorded
/// site disqualifies it — it stays `LogosI64Map`, never a miscompile. Forced off
/// by `LOGOS_DENSE_MAP=0`.
pub fn detect_dense_i64_maps(
    classes: &I64MapClasses,
    oracle: &crate::optimize::OracleFacts,
    interner: &Interner,
) -> HashMap<Symbol, DenseMapInfo> {
    let mut out = HashMap::new();
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::DenseMap) {
        return out;
    }
    for &m in &classes.maps {
        if !oracle.has_dense_map_capacity(m) {
            continue;
        }
        // The capacity must render to a Rust expression so the constructor can
        // size the direct-addressed array (`with_bounds(0, cap + 1)`); a
        // non-integer/unrenderable cap declines dense (the map stays a hash).
        if oracle
            .map_cap_lin(m)
            .and_then(|l| crate::optimize::lin_to_rust(l, interner))
            .is_none()
        {
            continue;
        }
        let sites = match classes.key_sites.get(&m) {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        if sites.iter().all(|&addr| oracle.dense_map_key_proven_addr(addr, m)) {
            // Offset 0: the proof establishes `0 <= key <= capacity`, so the
            // array (sized `capacity + 1` at the constructor) indexes `data[key]`.
            let kind = if classes.sets.contains(&m) {
                DenseKind::Set
            } else if !classes.has_contains.contains(&m)
                && oracle.dense_map_has_full_coverage(m)
                && sites.iter().all(|&addr| oracle.dense_key_covered_addr(addr, m))
            {
                // Presence elision: a value-read map with no `contains`, whose
                // inserts fully cover a contiguous range and whose every key is
                // proven inside it — so every queried key was definitely written.
                // `get` is a bare array load; the presence bitset is dropped.
                DenseKind::MapNoPresence
            } else {
                DenseKind::Map
            };
            out.insert(m, DenseMapInfo { lo: 0, kind });
            crate::optimize::mark_fired(crate::optimization::Opt::DenseMap);
        }
    }
    out
}

/// Select the NON-dense `Map of Int to Int` locals whose every key (insert / get
/// / contains) — and, for value-read maps, every stored value — provably fits
/// `i32`, lowering them to the half-width `LogosI32Map` or quarter-width
/// `LogosI32Set`. A fallback that halves a hash map's memory traffic (its
/// dominant cost) where the dense gate cannot apply. Returns `(maps, sets)`.
/// Sound: an unproven (or non-finite) key/value interval disqualifies the map —
/// it keeps full `i64` width — so the boundary `as i32` cast is always lossless.
/// Forced off by `LOGOS_NARROW_MAP=0`.
pub fn detect_i32_maps(
    classes: &I64MapClasses,
    dense: &HashMap<Symbol, DenseMapInfo>,
    oracle: &crate::optimize::OracleFacts,
) -> (HashSet<Symbol>, HashSet<Symbol>) {
    let mut maps = HashSet::new();
    let mut sets = HashSet::new();
    // On by default — a sound, lossless general optimization (proven i32-range
    // keys/values; the `as i32` cast cannot lose data and values round-trip back
    // to i64 on read). `LOGOS_NARROW_MAP=0` forces it off for A/B measurement.
    if !crate::optimize::active_config().is_on(crate::optimization::Opt::NarrowMap) {
        return (maps, sets);
    }
    let fits_i32 = |addr: usize| match oracle.expr_int_range_addr(addr) {
        Some((lo, hi)) => lo >= i32::MIN as i64 && hi <= i32::MAX as i64,
        None => false,
    };
    for &m in &classes.maps {
        if dense.contains_key(&m) {
            // Dense is strictly better — it wins, and NarrowMap is its fallback.
            crate::optimize::mark_preempted(
                crate::optimization::Opt::DenseMap,
                crate::optimization::Opt::NarrowMap,
            );
            continue;
        }
        let keys = match classes.key_sites.get(&m) {
            Some(s) if !s.is_empty() => s,
            _ => continue,
        };
        if !keys.iter().all(|&a| fits_i32(a)) {
            continue;
        }
        if classes.sets.contains(&m) {
            sets.insert(m);
        } else {
            // A value-read map also stores values — they must fit i32 too.
            let vals = classes.value_sites.get(&m).map(|v| v.as_slice()).unwrap_or(&[]);
            if vals.iter().all(|&a| fits_i32(a)) {
                maps.insert(m);
            }
        }
    }
    if !maps.is_empty() || !sets.is_empty() {
        crate::optimize::mark_fired(crate::optimization::Opt::NarrowMap);
    }
    (maps, sets)
}

/// Recognize the type string a `Map` codegen site would produce for this map,
/// returning whether both type arguments are `i64`.
fn is_int_int_map(ty: Option<&TypeExpr>, value: &Expr, interner: &Interner) -> bool {
    let both_i64 = |k: &TypeExpr, v: &TypeExpr| {
        codegen_type_expr(k, interner) == "i64" && codegen_type_expr(v, interner) == "i64"
    };
    if let Some(TypeExpr::Generic { base, params }) = ty {
        if matches!(interner.resolve(*base), "Map" | "HashMap") && params.len() >= 2 {
            return both_i64(&params[0], &params[1]);
        }
    }
    match value {
        Expr::New { type_name, type_args, .. } => {
            matches!(interner.resolve(*type_name), "Map" | "HashMap")
                && type_args.len() >= 2
                && both_i64(&type_args[0], &type_args[1])
        }
        Expr::WithCapacity { value: inner, .. } => {
            if let Expr::New { type_name, type_args, .. } = inner {
                matches!(interner.resolve(*type_name), "Map" | "HashMap")
                    && type_args.len() >= 2
                    && both_i64(&type_args[0], &type_args[1])
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Pass 1: tally `Let` bindings per symbol and flag the `Int → Int` map
/// constructions. Descends into nested blocks but NOT into `FunctionDef` bodies
/// (those are analysed in their own scope) or closures.
fn collect_candidates(
    stmts: &[Stmt],
    interner: &Interner,
    let_counts: &mut HashMap<Symbol, usize>,
    int_map_lets: &mut HashSet<Symbol>,
) {
    for stmt in stmts {
        if let Stmt::Let { var, ty, value, .. } = stmt {
            *let_counts.entry(*var).or_insert(0) += 1;
            if is_int_int_map(*ty, value, interner) {
                int_map_lets.insert(*var);
            }
        }
        for block in child_blocks(stmt) {
            collect_candidates(block, interner, let_counts, int_map_lets);
        }
    }
}

/// The child statement blocks of `stmt` that share its variable scope. Excludes
/// `FunctionDef` (a fresh scope) so a function-local name never contaminates the
/// enclosing analysis.
fn child_blocks<'a>(stmt: &Stmt<'a>) -> Vec<Block<'a>> {
    match stmt {
        Stmt::If { then_block, else_block, .. } => {
            let mut v = vec![*then_block];
            if let Some(eb) = else_block {
                v.push(*eb);
            }
            v
        }
        Stmt::While { body, .. }
        | Stmt::Repeat { body, .. }
        | Stmt::Zone { body, .. } => vec![*body],
        Stmt::Concurrent { tasks } | Stmt::Parallel { tasks } => vec![*tasks],
        Stmt::Inspect { arms, .. } => arms.iter().map(|a: &MatchArm<'a>| a.body).collect(),
        _ => Vec::new(),
    }
}

/// Pass 2: the conservative escape/alias walker.
struct Disq<'c> {
    candidates: &'c HashSet<Symbol>,
    disq: HashSet<Symbol>,
    inserted: HashSet<Symbol>,
    /// Candidates read by VALUE (`item k of m`) — these need a value array, so
    /// they stay `LogosI64Map`; those never here are keys-only sets.
    got: HashSet<Symbol>,
    /// Candidates with ≥1 `contains` use — disqualified from presence elision.
    has_contains: HashSet<Symbol>,
    /// Per candidate, the arena address of every insert/get/contains KEY
    /// expression — fed to the dense gate's per-site bound check.
    key_sites: HashMap<Symbol, Vec<usize>>,
    /// Per candidate, the arena address of every insert VALUE expression — fed to
    /// the i32-narrowing gate (the stored value must also fit i32).
    value_sites: HashMap<Symbol, Vec<usize>>,
}

impl<'c> Disq<'c> {
    fn mark_disq(&mut self, sym: Symbol) {
        if self.candidates.contains(&sym) {
            self.disq.insert(sym);
        }
    }

    /// Disqualify EVERY candidate — used for statements this pass cannot model.
    fn disq_all(&mut self) {
        for &s in self.candidates {
            self.disq.insert(s);
        }
    }

    /// A whitelisted map-collection position: a bare candidate identifier here
    /// is safe (read/insert in place). Anything else recurses as a value.
    fn coll(&mut self, e: &Expr) -> Option<Symbol> {
        if let Expr::Identifier(s) = e {
            if self.candidates.contains(s) {
                return Some(*s);
            }
        }
        self.expr(e);
        None
    }

    fn block(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            // The defining (or any) `Let`: the bound name is not a use; only its
            // initializer is. Shadowing is already excluded in pass 1.
            Stmt::Let { value, .. } => self.expr(value),

            // `Set m to e` rebinds the map variable itself — disqualify the map,
            // then analyse the value.
            Stmt::Set { target, value } => {
                self.mark_disq(*target);
                self.expr(value);
            }

            Stmt::Call { args, .. } => {
                for a in args {
                    self.expr(a);
                }
            }

            Stmt::If { cond, then_block, else_block } => {
                self.expr(cond);
                self.block(then_block);
                if let Some(eb) = else_block {
                    self.block(eb);
                }
            }
            Stmt::While { cond, body, decreasing } => {
                self.expr(cond);
                self.block(body);
                if let Some(d) = decreasing {
                    self.expr(d);
                }
            }
            Stmt::Repeat { iterable, body, .. } => {
                self.expr(iterable);
                self.block(body);
            }
            Stmt::Return { value } => {
                if let Some(v) = value {
                    self.expr(v);
                }
            }
            Stmt::Break => {}

            // Static proof obligations — erased in the runtime build, so they do
            // not alias or escape the map's representation.
            Stmt::Assert { .. } | Stmt::Trust { .. } => {}
            Stmt::RuntimeAssert { condition, .. } => self.expr(condition),

            Stmt::Give { object, recipient } => {
                self.expr(object);
                self.expr(recipient);
            }
            Stmt::Show { object, recipient } => {
                self.expr(object);
                self.expr(recipient);
            }
            Stmt::SetField { object, value, .. } => {
                self.expr(object);
                self.expr(value);
            }
            Stmt::StructDef { .. } => {}
            // A nested function is its own scope (analysed separately); the map
            // is a local of THIS scope and cannot be referenced inside it.
            Stmt::FunctionDef { .. } => {}

            Stmt::Inspect { target, arms, .. } => {
                self.expr(target);
                for a in arms {
                    self.block(a.body);
                }
            }

            Stmt::Push { value, collection } => {
                self.expr(value);
                self.expr(collection);
            }
            Stmt::Pop { collection, into } => {
                self.expr(collection);
                if let Some(i) = into {
                    self.mark_disq(*i);
                }
            }
            Stmt::Add { value, collection } | Stmt::Remove { value, collection } => {
                self.expr(value);
                self.expr(collection);
            }

            // The one writable whitelist position: `m.insert(index, value)`.
            Stmt::SetIndex { collection, index, value } => {
                if let Some(m) = self.coll(collection) {
                    self.inserted.insert(m);
                    self.key_sites.entry(m).or_default().push(expr_addr(index));
                    self.value_sites.entry(m).or_default().push(expr_addr(value));
                }
                self.expr(index);
                self.expr(value);
            }

            // Anything else — concurrency, CRDT, networking, IO, agents — is not
            // modelled here; refuse to convert any map in its presence.
            _ => self.disq_all(),
        }
    }

    /// Walk a value-position expression. A bare candidate identifier reached
    /// here is a non-whitelisted use → disqualify.
    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Identifier(s) => self.mark_disq(*s),
            Expr::Literal(_) | Expr::OptionNone | Expr::Escape { .. } => {}

            // Whitelisted read positions.
            Expr::Index { collection, index } => {
                if let Some(m) = self.coll(collection) {
                    self.got.insert(m);
                    self.key_sites.entry(m).or_default().push(expr_addr(index));
                }
                self.expr(index);
            }
            Expr::Contains { collection, value } => {
                if let Some(m) = self.coll(collection) {
                    self.has_contains.insert(m);
                    self.key_sites.entry(m).or_default().push(expr_addr(value));
                }
                self.expr(value);
            }

            Expr::BinaryOp { left, right, .. }
            | Expr::Union { left, right }
            | Expr::Intersection { left, right }
            | Expr::Range { start: left, end: right } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Not { operand } => self.expr(operand),
            Expr::Call { args, .. } => {
                for a in args {
                    self.expr(a);
                }
            }
            Expr::CallExpr { callee, args } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            Expr::Slice { collection, start, end } => {
                self.expr(collection);
                self.expr(start);
                self.expr(end);
            }
            Expr::Copy { expr } => self.expr(expr),
            Expr::Give { value } => self.expr(value),
            // `length of m` is read-only but not part of the LogosI64Map call
            // surface we emit — treat it as a value use (conservative).
            Expr::Length { collection } => self.expr(collection),
            Expr::ManifestOf { zone } => self.expr(zone),
            Expr::ChunkAt { index, zone } => {
                self.expr(index);
                self.expr(zone);
            }
            Expr::List(items) | Expr::Tuple(items) => {
                for i in items {
                    self.expr(i);
                }
            }
            Expr::FieldAccess { object, .. } => self.expr(object),
            Expr::New { init_fields, .. } => {
                for (_, v) in init_fields {
                    self.expr(v);
                }
            }
            Expr::NewVariant { fields, .. } => {
                for (_, v) in fields {
                    self.expr(v);
                }
            }
            Expr::OptionSome { value } => self.expr(value),
            Expr::WithCapacity { value, capacity } => {
                self.expr(value);
                self.expr(capacity);
            }
            // A closure could capture the map by reference → escape; walk its body
            // so any reference disqualifies.
            Expr::Closure { body, .. } => match body {
                ClosureBody::Expression(e) => self.expr(e),
                ClosureBody::Block(b) => self.block(b),
            },
            Expr::InterpolatedString(parts) => {
                for p in parts {
                    if let StringPart::Expr { value, .. } = p {
                        self.expr(value);
                    }
                }
            }
        }
    }
}

/// `true` for a registered map type string that names any map/set flavour — used
/// at the get/insert/contains/equality codegen sites so `LogosMap`, the
/// specialized `LogosI64Map`, and the keys-only `LogosI64Set` share one emission
/// path (their insert/contains call shapes are identical).
pub(super) fn is_logos_map_type(t: &str) -> bool {
    t.starts_with("LogosMap")
        || t.starts_with("LogosI64Map")
        || t.starts_with("LogosI64Set")
        || t.starts_with("LogosI32Map")
        || t.starts_with("LogosI32Set")
        || t.starts_with("LogosDenseI64Map")
        || t.starts_with("LogosDenseI64Set")
}
