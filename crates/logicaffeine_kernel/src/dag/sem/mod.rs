//! The value engine: type checking and definitional equality by evaluation into closures.
//!
//! The substitution checker ([`crate::dag::tc`]) crosses a binder by substituting a free variable
//! into the body and β-reduces by substituting an argument into a body — each a copy of
//! everything the variable is under. Here nothing is ever substituted. A term is *evaluated* in
//! an environment that gives each bound variable a value: β extends the environment, a binder
//! is crossed by binding a fresh variable, δ evaluates a definition's body once per universe
//! instantiation and applies it to the arguments. Evaluation performs β only; δ and ι wait in
//! *neutral* values (a head and its arguments) until conversion or inference demands a weak head,
//! and each such step is remembered on the value that asked for it.
//!
//! Types are values too: inferring an application instantiates its function type's closure with
//! the argument's value, and a lambda's type is a closure whose codomain is inferred only when
//! someone asks for it. The judgment is the substitution checker's rule for rule, and conversion
//! is Lean's algorithm (lazy δ by definitional height, proof irrelevance, η, struct-η, unit-like
//! types, K, quotients, `Nat`/`String` literals) — only the objects differ.
//!
//! Values live for a *session* of many declarations: a constant's type, its unfolded body, the
//! weak-head normal form of a closed application are computed once and shared by every later
//! declaration, until the session's arena outgrows its budget or the environment changes the
//! meaning of something already evaluated (see [`Kernel::sem_epoch`]).
//!
//! A variable is identified by its binder depth and its type, so the variable a check binds and
//! the variable a later comparison binds at the same depth with the same type are one value, and
//! everything known about one is known about the other. That identification is sound exactly
//! when no two variables in scope share a depth — the one invariant `Sem::bind` keeps.

pub mod conv;
pub mod eval;
pub mod infer;
pub mod table;
pub mod types;
pub mod value;
pub mod whnf;

use super::error::{KResult, KernelFailure};
use super::expr::ExprId;
use super::level::{LevelId, SubstId};
use super::tc::Kernel;
use table::{StampedTable, Table};
use value::{Arena, CtxId, EnvId, V};

/// Recursion bound for evaluation, reduction, inference and conversion (the checker thread's
/// stack is sized for it).
const MAX_DEPTH: u32 = 200_000;

/// A session ends, and every value is dropped, once the arena holds this many values.
const SESSION_VALUES: usize = 1 << 23;

/// One session's values and caches.
#[derive(Clone)]
pub struct State {
    pub arena: Arena,
    epoch: u64,
    const_type: Table<(u32, u32)>,
    const_body: Table<(u32, u32)>,
    rule_rhs: Table<(u32, u32, u32)>,
    eval_cache: Table<(u32, u32)>,
    prune_cache: Table<(u32, u64)>,
    /// Pruning for expressions beyond the 64-bit masks, keyed on (environment, expression).
    wide_prune_cache: Table<(u32, u32)>,
    /// Exact loose-variable sets of such expressions. A function of the (persistent) expression
    /// alone, so it outlives sessions.
    loose_sets: rustc_hash::FxHashMap<ExprId, std::rc::Rc<[u32]>>,
    /// Types inferred without checking, keyed on the environment and the expression. A checked
    /// inference is also an inferred one, so check results land here too.
    infer_cache: Table<(u32, u32)>,
    /// Types inferred WITH checking. Checking depends on the declaration's universe parameters
    /// and safety, so this is cleared at every declaration.
    check_cache: StampedTable<(u32, u32)>,
    /// Types inferred with checking in a SAFE declaration, of expressions that mention no
    /// universe parameter: such a check holds in every later declaration (no level parameter
    /// can be undeclared, and safe is the strictest safety), so these outlive the declaration.
    safe_checked: Table<(u32, u32)>,
    /// Types inferred with checking in a SAFE declaration, of expressions that do mention
    /// universe parameters, keyed also on the declaration's parameter list (`scope`): the check
    /// holds in every later safe-or-weaker declaration with exactly those parameters.
    scoped_checked: Table<(u32, u32, u32)>,
    /// The current declaration's universe parameters, as an interned list.
    scope: u32,
    /// Pairs of values proven definitionally equal.
    proven: Table<(u32, u32)>,
    /// Pairs of applications of one definition whose arguments were found to differ.
    failure: Table<(u32, u32)>,
    depth: u32,
    /// The binder depth: the level the next bound variable gets.
    level: u32,
    /// Lean's `eagerReduce`.
    eager: bool,
    /// The conversion steps a speculative probe may still take (conv.rs), while one runs.
    pub(crate) probe: Option<u32>,
    /// A value or cache key of this session refers to a term of the current declaration's
    /// scratch region (ids recycled when the declaration ends), or to a declaration that is only
    /// provisionally in the environment: the session must not outlive the declaration.
    scratch_seen: bool,
    pub counters: Counters,
    /// The values of recently evaluated constants and sorts, by expression id and the universe
    /// substitution they were read under, direct-mapped: a hash-consed constant is one id, so its
    /// value is found without substituting its levels or hashing its head. Only persistent ids: a
    /// scratch id names another term in the next declaration.
    atoms: Vec<(u32, u32, u32)>,
}

const ATOM_MEMO: usize = 1 << 16;

/// How the session's caches are doing (reported by `logos_kernel --stats`).
#[derive(Default, Clone, Debug)]
pub struct Counters {
    pub sessions: u64,
    pub resets_epoch: u64,
    pub resets_scratch: u64,
    pub resets_budget: u64,
    pub scratch_unsettled: u64,
    pub scratch_instance: u64,
    pub scratch_expr: u64,
    /// Values whose proof-ness neither their shape nor their head's signature decided, so that
    /// their type was inferred.
    pub proof_probes: u64,
}

impl Default for State {
    fn default() -> Self {
        State {
            arena: Arena::new(),
            epoch: 0,
            const_type: Table::new(),
            const_body: Table::new(),
            rule_rhs: Table::new(),
            eval_cache: Table::new(),
            prune_cache: Table::new(),
            wide_prune_cache: Table::new(),
            loose_sets: Default::default(),
            infer_cache: Table::new(),
            check_cache: StampedTable::new(),
            safe_checked: Table::new(),
            scoped_checked: Table::new(),
            scope: 0,
            proven: Table::new(),
            failure: Table::new(),
            depth: 0,
            level: 0,
            eager: false,
            probe: None,
            scratch_seen: false,
            counters: Counters::default(),
            atoms: vec![(u32::MAX, u32::MAX, u32::MAX); ATOM_MEMO],
        }
    }
}

impl State {
    fn begin_decl(&mut self, epoch: u64) {
        if self.epoch != epoch || self.scratch_seen || self.arena.len() > SESSION_VALUES {
            let c = &mut self.counters;
            c.sessions += 1;
            c.resets_epoch += (self.epoch != epoch) as u64;
            c.resets_scratch += self.scratch_seen as u64;
            c.resets_budget += (self.arena.len() > SESSION_VALUES) as u64;
            self.end_session(epoch);
        }
        self.check_cache.clear();
        self.depth = 0;
        self.level = 0;
        self.eager = false;
        self.probe = None;
    }

    #[inline]
    fn atom_slot(e: ExprId, s: SubstId) -> usize {
        (e.0 ^ s.0.wrapping_mul(0x9E37_79B9)) as usize & (ATOM_MEMO - 1)
    }

    #[inline]
    pub(crate) fn atom_memo(&self, e: ExprId, s: SubstId) -> Option<V> {
        let (k, ks, v) = self.atoms[Self::atom_slot(e, s)];
        (k == e.0 && ks == s.0).then_some(V(v))
    }

    #[inline]
    pub(crate) fn note_atom(&mut self, e: ExprId, s: SubstId, v: V) {
        if !e.is_scratch() {
            self.atoms[Self::atom_slot(e, s)] = (e.0, s.0, v.0);
        }
    }

    /// Drops every value and everything remembered about one, keeping the tables' storage.
    fn end_session(&mut self, epoch: u64) {
        self.arena.reset();
        self.atoms.fill((u32::MAX, u32::MAX, u32::MAX));
        self.epoch = epoch;
        self.const_type.clear();
        self.const_body.clear();
        self.rule_rhs.clear();
        self.eval_cache.clear();
        self.prune_cache.clear();
        self.wide_prune_cache.clear();
        self.infer_cache.clear();
        self.check_cache.clear();
        self.safe_checked.clear();
        self.scoped_checked.clear();
        self.proven.clear();
        self.failure.clear();
        self.scratch_seen = false;
    }

    /// Bytes held by each part of the session (capacity: what the process pays).
    pub fn memory_report(&self) -> Vec<(&'static str, usize)> {
        let mut r = self.arena.memory_report();
        r.push(("value engine: evaluation cache", self.eval_cache.bytes()));
        r.push((
            "value engine: pruning cache",
            self.prune_cache.bytes() + self.wide_prune_cache.bytes(),
        ));
        r.push((
            "value engine: inference caches",
            self.infer_cache.bytes()
                + self.check_cache.bytes()
                + self.safe_checked.bytes()
                + self.scoped_checked.bytes(),
        ));
        r.push((
            "value engine: conversion caches",
            self.proven.bytes() + self.failure.bytes(),
        ));
        r.push((
            "value engine: constant caches",
            self.const_type.bytes() + self.const_body.bytes() + self.rule_rhs.bytes(),
        ));
        r
    }

    pub fn bytes(&self) -> usize {
        self.memory_report().iter().map(|(_, b)| b).sum()
    }
}

/// The engine: the kernel (environment, stores, well-known names) and the session state.
pub struct Sem<'a> {
    pub k: &'a mut Kernel,
    pub st: &'a mut State,
}

fn reject(m: impl Into<String>) -> KernelFailure {
    KernelFailure::reject(m)
}

impl<'a> Sem<'a> {
    pub fn new(k: &'a mut Kernel, st: &'a mut State) -> Sem<'a> {
        Sem { k, st }
    }

    #[inline]
    pub(crate) fn enter(&mut self) -> KResult<()> {
        if self.st.depth >= MAX_DEPTH {
            return Err(KernelFailure::resource("recursion depth limit reached"));
        }
        self.st.depth += 1;
        Ok(())
    }

    #[inline]
    pub(crate) fn leave(&mut self) {
        self.st.depth -= 1;
    }

    /// A fresh variable of type `ty` at the current binder depth, which is entered: every use
    /// must be matched by [`Sem::unbind`] before a sibling binder is crossed.
    #[inline]
    pub(crate) fn bind(&mut self, ty: V) -> V {
        let level = self.st.level;
        self.st.level += 1;
        self.st
            .arena
            .head_value(value::Kind::Local, value::Head::Local { level, ty })
    }

    #[inline]
    pub(crate) fn unbind(&mut self) {
        self.st.level -= 1;
    }

    /// Runs `f` under a fresh variable of type `ty`.
    pub fn under<T>(&mut self, ty: V, f: impl FnOnce(&mut Self, V) -> KResult<T>) -> KResult<T> {
        let x = self.bind(ty);
        let r = f(self, x);
        self.unbind();
        r
    }

    // ---- declaration entry points ----

    /// Checks that the closed term `ty` is a type; its universe level.
    pub fn check_type(&mut self, ty: ExprId) -> KResult<LevelId> {
        let t = self.infer(EnvId::EMPTY, CtxId::EMPTY, ty, true)?;
        match self.sort_of(t)? {
            Some(l) => Ok(l),
            None => Err(reject(format!("type expected, found {}", self.k.show(ty)))),
        }
    }

    /// Checks the closed term `value` against the closed, already checked type `ty`.
    pub fn check_value(&mut self, value: ExprId, ty: ExprId) -> KResult<()> {
        let vt = self.infer(EnvId::EMPTY, CtxId::EMPTY, value, true)?;
        let tv = self.eval(EnvId::EMPTY, ty)?;
        if !self.is_def_eq(vt, tv)? {
            return Err(reject("declaration type mismatch").with_detail(format!(
                "the value has type\n    {}\n  but is declared with type\n    {}",
                self.show(vt),
                self.show(tv)
            )));
        }
        Ok(())
    }

    /// The level of a sort value after weak-head normalisation.
    pub(crate) fn sort_of(&mut self, t: V) -> KResult<Option<LevelId>> {
        let w = self.whnf(t)?;
        Ok(match self.st.arena.get(w) {
            value::Val::Sort(l) => Some(l),
            _ => None,
        })
    }

    /// A one-line description of a value, for diagnostics.
    pub fn show(&mut self, v: V) -> String {
        use value::{Head, Val};
        match self.st.arena.get(v) {
            Val::Sort(l) => format!("Sort {l:?}"),
            Val::Nat(n) => self.k.s.nat(n).to_decimal(),
            Val::Str(s) => format!("{:?}", self.k.s.string(s)),
            Val::Lam { body, .. } => format!("fun _ => {}", self.k.show(body)),
            Val::Pi { dom, env, body } => match self.k.s.node(body) {
                // A codomain read from the closure's environment: show the value it reads.
                crate::dag::expr::Node::BVar(i) if i > 0 => {
                    match self.st.arena.lookup(env, i - 1) {
                        Some(cod) => format!("({}) → {}", self.show(dom), self.show(cod)),
                        None => format!("({}) → {}", self.show(dom), self.k.show(body)),
                    }
                }
                _ => format!("({}) → {}", self.show(dom), self.k.show(body)),
            },
            Val::PiLam { dom, body, .. } => format!("({}) → {}", self.show(dom), self.k.show(body)),
            Val::Neu { head, spine, .. } => {
                let h = match self.st.arena.head(head) {
                    Head::Local { level, .. } => format!("x{level}"),
                    Head::Const { name, .. } => self.k.s.names.display(name),
                    Head::Proj { idx, x, .. } => format!("({}).{}", self.show(x), idx + 1),
                };
                let n = self.st.arena.spine_len(spine);
                let args: Vec<String> = self
                    .st
                    .arena
                    .args(spine)
                    .iter()
                    .take(6)
                    .map(|a| self.show_atom(*a))
                    .collect();
                if n == 0 {
                    h
                } else if n > 6 {
                    format!("{h} {} …", args.join(" "))
                } else {
                    format!("{h} {}", args.join(" "))
                }
            }
        }
    }

    fn show_atom(&mut self, v: V) -> String {
        use value::{Head, Val};
        match self.st.arena.get(v) {
            Val::Neu { head, spine, .. } if spine == value::Sp::NIL => {
                match self.st.arena.head(head) {
                    Head::Local { level, .. } => format!("x{level}"),
                    Head::Const { name, .. } => self.k.s.names.display(name),
                    Head::Proj { idx, .. } => format!("_.{}", idx + 1),
                }
            }
            Val::Nat(n) => self.k.s.nat(n).to_decimal(),
            Val::Sort(_) => "Sort _".into(),
            _ => "(…)".into(),
        }
    }
}

impl Kernel {
    /// Runs the value engine on this kernel with its session state.
    pub fn with_sem<T>(&mut self, f: impl FnOnce(&mut Sem) -> KResult<T>) -> KResult<T> {
        let mut st = self.sem.take().unwrap_or_default();
        st.begin_decl(self.sem_epoch());
        let ps = self.lparams.clone();
        st.scope = self.const_levels_of(&ps).0;
        let r = {
            let mut sem = Sem::new(self, &mut st);
            f(&mut sem)
        };
        self.sem = Some(st);
        r
    }

    /// Changes whenever something already evaluated may now mean something else: the quotient
    /// reductions were switched on, or a primitive was validated (its literal extension switched
    /// on). The value engine's session never spans two epochs.
    ///
    /// A withdrawn declaration needs no epoch: only a provisional declaration is ever withdrawn,
    /// and a session that touches one is marked (`scratch_seen`) and ends with its declaration.
    pub fn sem_epoch(&self) -> u64 {
        ((self.validated_primitives.len() as u64) << 1) | self.env.quot_init as u64
    }
}
