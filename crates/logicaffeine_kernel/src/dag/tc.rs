//! Type inference, weak-head normalisation and definitional equality over the shared DAG.
//!
//! The algorithm is Lean 4's kernel algorithm — the one `lean4lean` transcribes from the C++ —
//! so that every verdict agrees with Lean's: lazy δ-reduction ordered by definitional height,
//! proof irrelevance, η for functions and structures, unit-like types, K-like and structure
//! recursor reduction, quotient reduction, and `Nat`/`String` literals. What differs is the
//! representation underneath: structural equality is an id compare, and every cache is keyed on
//! ids, so a subterm shared a thousand times is inferred, reduced and compared once.

use super::env::{Const, Env, Hint, Kind, QuotKind, RecInfo, Safety};
use super::error::{KResult, KernelFailure};
use super::expr::{ExprId, Node, Store};
use super::level::{LevelId, LevelNode, LevelsId};
use super::name::NameId;
use super::nat::Nat;
use super::wellknown::WellKnown;
use rustc_hash::{FxHashMap as FxMap, FxHashSet as FxSet};
use std::rc::Rc;

#[derive(Clone, Copy)]
pub struct LocalDecl {
    pub ty: ExprId,
    pub val: Option<ExprId>,
}

/// Pairs of expressions proven definitionally equal, closed under congruence but NOT under
/// transitivity.
///
/// Lean's kernel keeps a union-find here, which closes the relation transitively. But its
/// definitional equality is not transitive: by proof irrelevance `a ≡ b` and `b ≡ c` can hold for
/// proofs whose types make `a ≢ c`. The Arena's `proj-of-stuck-prop`, `rec-missing-ih` and
/// `rec-of-subst-prop` turn exactly that closure into a proof of `False`. The official kernel is
/// exposed only when two hashes collide; a union-find consulted unconditionally would always be.
/// So this cache answers only what was actually proven, plus congruence (two nodes of the same
/// shape whose children are pairwise equal or proven equal), which is the algorithm's own rule.
#[derive(Clone, Default)]
struct ProvenEqual {
    pairs: FxSet<(ExprId, ExprId)>,
    /// How many pairs have been proven: what a remembered refutation is stamped with.
    generation: u64,
}

impl ProvenEqual {
    #[inline]
    fn key(a: ExprId, b: ExprId) -> (ExprId, ExprId) {
        if a <= b {
            (a, b)
        } else {
            (b, a)
        }
    }

    #[inline]
    fn contains(&self, a: ExprId, b: ExprId) -> bool {
        self.pairs.contains(&Self::key(a, b))
    }

    #[inline]
    fn insert(&mut self, a: ExprId, b: ExprId) {
        if a != b && self.pairs.insert(Self::key(a, b)) {
            self.generation += 1;
        }
    }

    fn clear(&mut self) {
        if self.pairs.capacity() > 1 << 14 {
            self.pairs = FxSet::default();
        } else {
            self.pairs.clear();
        }
    }
}

enum LBool {
    True,
    False,
    Undef,
}

impl From<bool> for LBool {
    fn from(b: bool) -> LBool {
        if b {
            LBool::True
        } else {
            LBool::False
        }
    }
}

enum Step {
    Continue(ExprId, ExprId),
    Unknown(ExprId, ExprId),
    True,
    False(ExprId, ExprId),
}

/// Recursion bound for the mutually recursive infer/whnf/defeq calls (checked on a thread whose
/// stack is sized for it). Exceeding it declines instead of overflowing.
const MAX_DEPTH: u32 = 250_000;

/// A kernel is plain data (its only shared cells are immutable: interned strings and admitted
/// constants), so a clone is an independent kernel; `mirror` keeps one as a snapshot to branch from.
#[derive(Clone)]
pub struct Kernel {
    pub s: Store,
    pub env: Env,
    pub wk: WellKnown,
    pub(crate) lctx: Vec<LocalDecl>,
    pub(crate) lparams: Rc<[NameId]>,
    pub(crate) safety: Safety,
    pub(crate) eager: bool,
    depth: u32,
    infer_only_cache: FxMap<ExprId, ExprId>,
    infer_check_cache: FxMap<ExprId, ExprId>,
    whnf_core_cache: FxMap<ExprId, ExprId>,
    whnf_cache: FxMap<ExprId, ExprId>,
    unfold_cache: FxMap<ExprId, ExprId>,
    failure: FxSet<(ExprId, ExprId)>,
    eqv: ProvenEqual,
    /// Pairs refuted in this declaration, keyed with the `eagerReduce` mode (the modes can decide
    /// a pair differently) and stamped with `eqv`'s generation when their comparison began. Asked
    /// again in that mode with nothing proven since, a refuted pair is refuted without re-running
    /// the algorithm — which would retrace the same steps: the terms are closed (a free variable
    /// names an entry of a local context that only grows within the declaration), their levels
    /// are the declaration's own, the environment never changes what a constant already present
    /// means, and the caches consulted are memos of those same steps. A proof reached since can
    /// make the pair equal (by congruence, or deeper, through a sub-comparison), so any new proven
    /// pair retires every stamp. A comparison that exceeds the depth bound declines and records
    /// nothing. Without this, a comparison whose every level re-asks the refutations below it (a
    /// recursion on a literal, stuck on variables that differ) costs the square of its depth.
    refuted: FxMap<(ExprId, ExprId, bool), u64>,
    /// Primitive operations whose definitions were proven to meet their equations, and so are
    /// computed natively on literals.
    pub(crate) validated_primitives: FxSet<NameId>,
    /// A settled constant's type, value and recursor rules at given universe levels depend only
    /// on persistent terms, so they are interned persistently and shared by every declaration.
    global_type: FxMap<(NameId, LevelsId), ExprId>,
    global_value: FxMap<(NameId, LevelsId), ExprId>,
    global_rule: FxMap<(NameId, u32, LevelsId), ExprId>,
    /// Which argument positions each settled definition cannot distinguish.
    pub(crate) relevance: super::relevance::Cache,
    /// The closure-based conversion engine's per-declaration state.
    #[cfg(feature = "nbe")]
    pub(crate) nbe: crate::dag::nbe::State,
    /// Decide definitional equality with the NbE engine instead of the substitution engine.
    #[cfg(feature = "nbe")]
    pub use_nbe: bool,
    /// Decide every definitional equality with BOTH engines; a disagreement declines.
    #[cfg(feature = "nbe")]
    pub dual_conv: bool,
    /// Which judgment admits definitions, theorems, opaques and axioms.
    pub engine: Engine,
    /// The steps the value engine lets one argument pair of a same-head comparison take before it
    /// unfolds the definitions instead (see `sem::conv`): conversion steps and δ-unfolding steps
    /// both spend it. It changes what conversion costs, never what it answers.
    pub probe_steps: u32,
    /// Definitional-equality steps taken so far (entries into the core of the comparison): what
    /// conversion has cost, independent of the machine.
    conv_steps: u64,
    /// δ-unfoldings weak-head reduction has taken so far: what reduction has cost by unfolding,
    /// independent of the machine.
    unfold_steps: u64,
    /// The value engine's session, created on first use (it moves out of the kernel for the
    /// length of every check, so it is boxed and taken rather than rebuilt).
    pub(crate) sem: Option<Box<super::sem::State>>,
    /// The type theory this kernel checks.
    pub theory: Theory,
    /// What checking LOGOS's theory needs beyond the store and the environment (see
    /// [`super::logos_tc`]); present exactly under [`Theory::Logos`].
    pub(crate) logos: Option<Box<super::logos_tc::LogosState>>,
}

/// The type theory a kernel checks.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Theory {
    /// Lean 4's, exactly: the Lean Kernel Arena's checker and the Lean import path.
    #[default]
    Lean,
    /// LOGOS's Calculus of Inductive Constructions: `match`, `fix` and mutual `fix`, cumulative
    /// universes, `SProp`, LOGOS's literals.
    Logos,
}

/// The engine that checks declaration types and values (inductive families, quotients and the
/// primitive validations always use the substitution checker).
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Engine {
    /// Substitution: `tc`.
    #[default]
    Classic,
    /// Evaluation into closures: `sem`.
    Sem,
    /// Both, and they must agree on every declaration.
    Dual,
}

macro_rules! bail {
    ($self:ident, $($arg:tt)*) => {
        return Err(KernelFailure::reject(format!($($arg)*)))
    };
}

impl Kernel {
    pub fn new() -> Kernel {
        let mut s = Store::new();
        let wk = WellKnown::new(&mut s.names);
        Kernel {
            s,
            env: Env::default(),
            wk,
            lctx: Vec::new(),
            lparams: Rc::from(Vec::new()),
            safety: Safety::Safe,
            eager: false,
            depth: 0,
            infer_only_cache: FxMap::default(),
            infer_check_cache: FxMap::default(),
            whnf_core_cache: FxMap::default(),
            whnf_cache: FxMap::default(),
            unfold_cache: FxMap::default(),
            failure: FxSet::default(),
            eqv: ProvenEqual::default(),
            refuted: FxMap::default(),
            validated_primitives: FxSet::default(),
            global_type: FxMap::default(),
            global_value: FxMap::default(),
            global_rule: FxMap::default(),
            relevance: Default::default(),
            #[cfg(feature = "nbe")]
            nbe: Default::default(),
            #[cfg(feature = "nbe")]
            use_nbe: false,
            #[cfg(feature = "nbe")]
            dual_conv: false,
            engine: Engine::Classic,
            probe_steps: 2048,
            conv_steps: 0,
            unfold_steps: 0,
            sem: None,
            theory: Theory::Lean,
            logos: None,
        }
    }

    /// Starts checking one declaration: fresh local context, caches, and scratch region.
    pub fn begin_decl(&mut self, lparams: Rc<[NameId]>, safety: Safety) {
        self.lctx.clear();
        self.lparams = lparams;
        self.safety = safety;
        self.eager = false;
        self.depth = 0;
        self.s.begin_scratch();
    }

    pub fn end_decl(&mut self) {
        self.lctx.clear();
        clear_map(&mut self.infer_only_cache);
        clear_map(&mut self.infer_check_cache);
        clear_map(&mut self.whnf_core_cache);
        clear_map(&mut self.whnf_cache);
        clear_map(&mut self.unfold_cache);
        if self.failure.capacity() > 1 << 14 {
            self.failure = FxSet::default();
        } else {
            self.failure.clear();
        }
        if self.refuted.capacity() > 1 << 14 {
            self.refuted = FxMap::default();
        } else {
            self.refuted.clear();
        }
        self.eqv.clear();
        #[cfg(feature = "nbe")]
        self.nbe.reset();
        if let Some(l) = self.logos.as_mut() {
            l.end_decl();
        }
        self.s.end_scratch();
    }

    /// The store's memory, plus the kernel's own caches and environment.
    pub fn memory_report(&self) -> Vec<(&'static str, usize)> {
        let mut r = self.s.memory_report();
        r.push(("declarations (count)", self.env.count));
        if let Some(st) = self.sem.as_ref() {
            r.extend(st.memory_report());
        }
        let c = self
            .sem
            .as_ref()
            .map(|s| s.counters.clone())
            .unwrap_or_default();
        for (what, n) in [
            ("value engine: sessions", c.sessions),
            ("value engine: resets (epoch)", c.resets_epoch),
            ("value engine: resets (scratch)", c.resets_scratch),
            ("value engine: resets (budget)", c.resets_budget),
            (
                "value engine: scratch from unsettled constants",
                c.scratch_unsettled,
            ),
            (
                "value engine: scratch from tainted instances",
                c.scratch_instance,
            ),
            ("value engine: scratch from scratch terms", c.scratch_expr),
            ("value engine: proof status inferred", c.proof_probes),
        ] {
            r.push((what, n as usize));
        }
        r.push((
            "global type/value/rule cache entries",
            self.global_type.len() + self.global_value.len() + self.global_rule.len(),
        ));
        r
    }

    pub fn set_safety(&mut self, safety: Safety) {
        self.safety = safety;
    }

    // ---- local context ----

    pub fn push_local(&mut self, ty: ExprId, val: Option<ExprId>) -> ExprId {
        let i = self.lctx.len() as u32;
        self.lctx.push(LocalDecl { ty, val });
        self.s.fvar(i)
    }

    pub fn fvar_index(&self, e: ExprId) -> u32 {
        match self.s.node(e) {
            Node::FVar(i) => i,
            _ => unreachable!("not a free variable"),
        }
    }

    /// `mkForall`/`mkLambda` over local declarations (lets become `let`).
    pub fn mk_binding(&mut self, pi: bool, fvars: &[ExprId], body: ExprId) -> ExprId {
        let idx: Vec<u32> = fvars.iter().map(|f| self.fvar_index(*f)).collect();
        let mut r = self.s.abstract_fvars(body, &idx);
        for i in (0..idx.len()).rev() {
            let d = self.lctx[idx[i] as usize];
            let ty = self.s.abstract_fvars(d.ty, &idx[..i]);
            r = match d.val {
                Some(v) => {
                    let v = self.s.abstract_fvars(v, &idx[..i]);
                    self.s.let_(ty, v, r)
                }
                None => {
                    if pi {
                        self.s.pi(ty, r)
                    } else {
                        self.s.lam(ty, r)
                    }
                }
            };
        }
        r
    }

    pub fn mk_pi(&mut self, fvars: &[ExprId], body: ExprId) -> ExprId {
        self.mk_binding(true, fvars, body)
    }

    pub fn mk_lambda(&mut self, fvars: &[ExprId], body: ExprId) -> ExprId {
        self.mk_binding(false, fvars, body)
    }

    fn enter(&mut self) -> KResult<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(KernelFailure::resource("recursion depth limit reached"));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    // ---- constants ----

    pub fn get_const(&self, n: NameId) -> KResult<Rc<Const>> {
        self.env.get(n).cloned().ok_or_else(|| {
            KernelFailure::reject(format!("unknown constant '{}'", self.s.names.display(n)))
        })
    }

    /// `c`'s type at universe arguments `ls`.
    pub fn instantiate_type_lparams(&mut self, c: &Const, ls: LevelsId) -> ExprId {
        if c.lparams.is_empty() {
            return c.ty;
        }
        let settled = self.env.is_settled(c.name);
        if settled {
            if let Some(r) = self.global_type.get(&(c.name, ls)) {
                return *r;
            }
        }
        let us = self.s.levels.get_list(ls).to_vec();
        if settled {
            let (r, persistent) = self
                .s
                .with_persistent(|s| s.instantiate_lparams(c.ty, &c.lparams, &us));
            if persistent {
                self.global_type.insert((c.name, ls), r);
            }
            r
        } else {
            self.s.instantiate_lparams(c.ty, &c.lparams, &us)
        }
    }

    fn check_level(&self, l: LevelId) -> KResult<()> {
        if let Some(n) = self.s.levels.undef_param(l, &self.lparams) {
            bail!(
                self,
                "invalid reference to undefined universe level parameter '{}'",
                self.s.names.display(n)
            );
        }
        Ok(())
    }

    fn infer_constant(&mut self, n: NameId, ls: LevelsId, infer_only: bool) -> KResult<ExprId> {
        let c = self.get_const(n)?;
        let us = self.s.levels.get_list(ls).to_vec();
        if c.lparams.len() != us.len() {
            bail!(
                self,
                "incorrect number of universe levels for '{}': {} expected, {} provided",
                self.s.names.display(n),
                c.lparams.len(),
                us.len()
            );
        }
        if !infer_only {
            if c.is_unsafe() && self.safety != Safety::Unsafe {
                bail!(
                    self,
                    "invalid declaration, it uses unsafe declaration '{}'",
                    self.s.names.display(n)
                );
            }
            if c.is_partial() && self.safety == Safety::Safe {
                bail!(
                    self,
                    "invalid declaration, safe declaration must not contain partial declaration '{}'",
                    self.s.names.display(n)
                );
            }
            for l in &us {
                self.check_level(*l)?;
                if self.s.levels.mentions_sprop(*l) {
                    bail!(
                        self,
                        "'{}' is instantiated at SProp, which is a sort and not a universe level",
                        self.s.names.display(n)
                    );
                }
            }
        }
        Ok(self.instantiate_type_lparams(&c, ls))
    }

    // ---- inference ----

    pub fn infer(&mut self, e: ExprId, infer_only: bool) -> KResult<ExprId> {
        if self.s.has_loose_bvars(e) {
            bail!(self, "type checker does not support loose bound variables");
        }
        let cached = if infer_only {
            self.infer_only_cache.get(&e)
        } else {
            self.infer_check_cache.get(&e)
        };
        if let Some(r) = cached {
            return Ok(*r);
        }
        self.enter()?;
        let r = self.infer_uncached(e, infer_only);
        self.leave();
        let r = r?;
        if infer_only {
            self.infer_only_cache.insert(e, r);
        } else {
            self.infer_check_cache.insert(e, r);
        }
        Ok(r)
    }

    fn infer_uncached(&mut self, e: ExprId, infer_only: bool) -> KResult<ExprId> {
        match self.s.node(e) {
            Node::BVar(_) => bail!(self, "unexpected bound variable"),
            Node::FVar(i) => match self.lctx.get(i as usize) {
                Some(d) => Ok(d.ty),
                None => bail!(self, "unknown free variable"),
            },
            Node::Sort(l) => {
                if !infer_only {
                    self.check_level(l)?;
                }
                let sl = self.s.levels.succ(l);
                Ok(self.s.sort(sl))
            }
            Node::Const(n, ls) => self.infer_constant(n, ls, infer_only),
            Node::Lam(..) => self.infer_lambda(e, infer_only),
            Node::Pi(..) => self.infer_pi(e, infer_only),
            Node::Let(..) => self.infer_let(e, infer_only),
            Node::App(f, a) => {
                if infer_only {
                    self.infer_app(e)
                } else {
                    let ft = self.infer(f, false)?;
                    let ft = self.ensure_pi(ft)?;
                    if self.theory == Theory::Logos {
                        let Node::Pi(dom, body) = self.s.node(ft) else {
                            unreachable!()
                        };
                        if self.check_fix_against(a, dom)? {
                            return Ok(self.s.instantiate1(body, a));
                        }
                    }
                    let at = self.infer(a, false)?;
                    let Node::Pi(dom, body) = self.s.node(ft) else {
                        unreachable!()
                    };
                    let eager_arg = self.is_app_of_arity(a, self.wk.eager_reduce, 2);
                    let ok = if eager_arg && !self.eager {
                        self.eager = true;
                        let r = self.accepts_argument(dom, at);
                        self.eager = false;
                        r?
                    } else {
                        self.accepts_argument(dom, at)?
                    };
                    if !ok {
                        return Err(
                            KernelFailure::reject("application type mismatch".to_string())
                                .with_detail(self.describe_mismatch(e, dom, at)),
                        );
                    }
                    Ok(self.s.instantiate1(body, a))
                }
            }
            Node::NatLit(_) | Node::StrLit(_) if self.theory == Theory::Logos => {
                self.infer_logos(e, infer_only)
            }
            Node::NatLit(_) => {
                if !infer_only && !self.env.contains(self.wk.nat) {
                    bail!(self, "natural number literal used before 'Nat' is declared");
                }
                Ok(self.s.cnst(self.wk.nat, LevelsId::EMPTY))
            }
            Node::StrLit(_) => {
                if !infer_only
                    && !(self.env.contains(self.wk.char_of_nat)
                        && self.env.contains(self.wk.string_of_list))
                {
                    bail!(
                        self,
                        "string literal used before 'String.ofList' and 'Char.ofNat' are declared"
                    );
                }
                Ok(self.s.cnst(self.wk.string, LevelsId::EMPTY))
            }
            Node::Proj(sn, idx, x) => {
                let xt = self.infer(x, infer_only)?;
                self.infer_proj(e, sn, idx, x, xt)
            }
            Node::Match(..) | Node::Fix(_) | Node::MutualFix(..) | Node::Lit(_) => {
                self.infer_logos(e, infer_only)
            }
            Node::Seq(..) | Node::Nil => bail!(self, "a sequence is not a term"),
        }
    }

    fn infer_lambda(&mut self, mut e: ExprId, infer_only: bool) -> KResult<ExprId> {
        let mut fvars = Vec::new();
        while let Node::Lam(t, b) = self.s.node(e) {
            let d = self.s.instantiate_rev(t, &fvars);
            if !infer_only {
                let dt = self.infer(d, false)?;
                self.ensure_sort(dt)?;
            }
            fvars.push(self.push_local(d, None));
            e = b;
        }
        let body = self.s.instantiate_rev(e, &fvars);
        let r = self.infer(body, infer_only)?;
        let r = self.cheap_beta_reduce(r);
        Ok(self.mk_pi(&fvars, r))
    }

    fn infer_pi(&mut self, mut e: ExprId, infer_only: bool) -> KResult<ExprId> {
        let mut fvars = Vec::new();
        let mut us = Vec::new();
        while let Node::Pi(t, b) = self.s.node(e) {
            let d = self.s.instantiate_rev(t, &fvars);
            let dt = self.infer(d, infer_only)?;
            let s = self.ensure_sort(dt)?;
            us.push(self.sort_level(s));
            fvars.push(self.push_local(d, None));
            e = b;
        }
        let body = self.s.instantiate_rev(e, &fvars);
        let r = self.infer(body, infer_only)?;
        let s = self.ensure_sort(r)?;
        let mut l = self.sort_level(s);
        for u in us.into_iter().rev() {
            l = self.s.levels.imax(u, l);
        }
        Ok(self.s.sort(l))
    }

    fn infer_let(&mut self, mut e: ExprId, infer_only: bool) -> KResult<ExprId> {
        let mut fvars = Vec::new();
        while let Node::Let(t, v, b) = self.s.node(e) {
            let ty = self.s.instantiate_rev(t, &fvars);
            let val = self.s.instantiate_rev(v, &fvars);
            if !infer_only {
                let tt = self.infer(ty, false)?;
                self.ensure_sort(tt)?;
                if !(self.theory == Theory::Logos && self.check_fix_against(val, ty)?) {
                    let vt = self.infer(val, false)?;
                    if !self.conforms(vt, ty)? {
                        return Err(KernelFailure::reject("let value type mismatch".to_string())
                            .with_detail(self.describe_mismatch(val, ty, vt)));
                    }
                }
            }
            fvars.push(self.push_local(ty, Some(val)));
            e = b;
        }
        let body = self.s.instantiate_rev(e, &fvars);
        let r = self.infer(body, infer_only)?;
        let r = self.cheap_beta_reduce(r);
        Ok(self.mk_pi(&fvars, r))
    }

    fn infer_app(&mut self, e: ExprId) -> KResult<ExprId> {
        let (f, args) = self.s.app_args(e);
        let mut ft = self.infer(f, true)?;
        let mut j = 0;
        for i in 0..args.len() {
            match self.s.node(ft) {
                Node::Pi(_, body) => ft = body,
                _ => {
                    let t = self.s.instantiate_rev(ft, &args[j..i]);
                    let t = self.ensure_pi(t)?;
                    let Node::Pi(_, body) = self.s.node(t) else {
                        unreachable!()
                    };
                    ft = body;
                    j = i;
                }
            }
        }
        Ok(self.s.instantiate_rev(ft, &args[j..]))
    }

    fn infer_proj(
        &mut self,
        e: ExprId,
        sname: NameId,
        idx: u32,
        x: ExprId,
        xt: ExprId,
    ) -> KResult<ExprId> {
        let ty = self.whnf(xt)?;
        let (head, args) = self.s.app_args(ty);
        let fail = |k: &Kernel| KernelFailure::reject(format!("invalid projection {}", k.show(e)));
        let Node::Const(iname, ils) = self.s.node(head) else {
            return Err(fail(self));
        };
        if iname != sname {
            return Err(fail(self));
        }
        let Some(ind) = self.env.inductive(iname).cloned() else {
            return Err(fail(self));
        };
        if ind.ctors.len() != 1 || args.len() != (ind.num_params + ind.num_indices) as usize {
            return Err(fail(self));
        }
        let ctor = self.get_const(ind.ctors[0])?;
        let mut r = self.instantiate_type_lparams(&ctor, ils);
        for arg in args.iter().take(ind.num_params as usize) {
            let w = self.whnf(r)?;
            let Node::Pi(_, b) = self.s.node(w) else {
                return Err(fail(self));
            };
            r = self.s.instantiate1(b, *arg);
        }
        let sl = self.get_sort_level(ty)?;
        let maybe_prop = !self.s.levels.is_never_zero(sl);
        for i in 0..idx {
            let w = self.whnf(r)?;
            let Node::Pi(dom, b) = self.s.node(w) else {
                return Err(fail(self));
            };
            if self.s.has_loose_bvars(b) {
                if maybe_prop && !self.is_prop(dom)? {
                    return Err(fail(self));
                }
                let p = self.s.proj(iname, i, x);
                r = self.s.instantiate1(b, p);
            } else {
                r = b;
            }
        }
        let w = self.whnf(r)?;
        let Node::Pi(dom, _) = self.s.node(w) else {
            return Err(fail(self));
        };
        if maybe_prop && !self.is_prop(dom)? {
            return Err(fail(self));
        }
        Ok(dom)
    }

    /// Lean's `cheapBetaReduce`.
    fn cheap_beta_reduce(&mut self, e: ExprId) -> ExprId {
        let Node::App(..) = self.s.node(e) else {
            return e;
        };
        let (f, args) = self.s.app_args(e);
        let Node::Lam(..) = self.s.node(f) else {
            return e;
        };
        let mut i = 0;
        let mut body = f;
        while i < args.len() {
            match self.s.node(body) {
                Node::Lam(_, b) => {
                    body = b;
                    i += 1;
                }
                _ => break,
            }
        }
        if !self.s.has_loose_bvars(body) {
            self.s.mk_app_n(body, &args[i..])
        } else if let Node::BVar(n) = self.s.node(body) {
            let n = n as usize;
            if n < i {
                let head = args[i - n - 1];
                self.s.mk_app_n(head, &args[i..])
            } else {
                e
            }
        } else {
            e
        }
    }

    pub fn sort_level(&self, s: ExprId) -> LevelId {
        match self.s.node(s) {
            Node::Sort(l) => l,
            _ => unreachable!("sort_level of a non-sort"),
        }
    }

    pub fn ensure_sort(&mut self, e: ExprId) -> KResult<ExprId> {
        if let Node::Sort(_) = self.s.node(e) {
            return Ok(e);
        }
        let w = self.whnf(e)?;
        if let Node::Sort(_) = self.s.node(w) {
            return Ok(w);
        }
        Err(KernelFailure::reject(format!(
            "type expected, found {}",
            self.show(e)
        )))
    }

    pub fn ensure_pi(&mut self, e: ExprId) -> KResult<ExprId> {
        if let Node::Pi(..) = self.s.node(e) {
            return Ok(e);
        }
        let w = self.whnf(e)?;
        if let Node::Pi(..) = self.s.node(w) {
            return Ok(w);
        }
        Err(KernelFailure::reject(format!(
            "function expected, found {}",
            self.show(e)
        )))
    }

    pub fn get_sort_level(&mut self, e: ExprId) -> KResult<LevelId> {
        let t = self.infer(e, true)?;
        let s = self.ensure_sort(t)?;
        Ok(self.sort_level(s))
    }

    /// `e`'s type is `Prop`.
    pub fn is_prop(&mut self, e: ExprId) -> KResult<bool> {
        let l = self.get_sort_level(e)?;
        Ok(self.s.levels.is_always_zero(l))
    }

    /// Infers `e`'s type with full checking.
    pub fn check(&mut self, e: ExprId) -> KResult<ExprId> {
        self.infer(e, false)
    }

    // ---- reduction ----

    fn is_let_fvar(&self, e: ExprId) -> Option<ExprId> {
        match self.s.node(e) {
            Node::FVar(i) => self.lctx.get(i as usize).and_then(|d| d.val),
            _ => None,
        }
    }

    pub fn whnf_core(&mut self, e: ExprId, cheap_proj: bool) -> KResult<ExprId> {
        match self.s.node(e) {
            Node::BVar(_)
            | Node::Sort(_)
            | Node::Pi(..)
            | Node::Const(..)
            | Node::Lam(..)
            | Node::NatLit(_)
            | Node::StrLit(_)
            | Node::Lit(_)
            | Node::Fix(_)
            | Node::MutualFix(..) => return Ok(e),
            Node::FVar(_) => {
                if self.is_let_fvar(e).is_none() {
                    return Ok(e);
                }
            }
            _ => {}
        }
        if let Some(r) = self.whnf_core_cache.get(&e) {
            return Ok(*r);
        }
        self.enter()?;
        let r = self.whnf_core_uncached(e, cheap_proj);
        self.leave();
        r
    }

    fn whnf_core_uncached(&mut self, e: ExprId, cheap_proj: bool) -> KResult<ExprId> {
        let save = |k: &mut Kernel, r: ExprId| {
            if !cheap_proj {
                k.whnf_core_cache.insert(e, r);
            }
            Ok(r)
        };
        match self.s.node(e) {
            Node::FVar(_) => {
                let v = self.is_let_fvar(e).unwrap();
                self.whnf_core(v, cheap_proj)
            }
            Node::App(..) => {
                let f0 = self.s.app_fn(e);
                let f = self.whnf_core(f0, cheap_proj)?;
                if f == f0 && !matches!(self.s.node(f), Node::Lam(..)) {
                    if self.theory == Theory::Logos {
                        return self.whnf_core_logos_app(e, cheap_proj);
                    }
                    return match self.reduce_recursor(e)? {
                        Some(r) => self.whnf_core(r, cheap_proj),
                        None => Ok(e),
                    };
                }
                let (_, args) = self.s.app_args(e);
                if let Node::Lam(..) = self.s.node(f) {
                    let mut m = 0;
                    let mut body = f;
                    while m < args.len() {
                        match self.s.node(body) {
                            Node::Lam(_, b) => {
                                body = b;
                                m += 1;
                            }
                            _ => break,
                        }
                    }
                    let r = self.s.instantiate_rev(body, &args[..m]);
                    let r = self.s.mk_app_n(r, &args[m..]);
                    let r = self.whnf_core(r, cheap_proj)?;
                    save(self, r)
                } else {
                    let r = self.s.mk_app_n(f, &args);
                    let r = self.whnf_core(r, cheap_proj)?;
                    save(self, r)
                }
            }
            Node::Let(_, v, b) => {
                let r = self.s.instantiate1(b, v);
                let r = self.whnf_core(r, cheap_proj)?;
                save(self, r)
            }
            Node::Proj(_, idx, x) => match self.reduce_proj(idx, x, cheap_proj)? {
                Some(m) => {
                    let r = self.whnf_core(m, cheap_proj)?;
                    save(self, r)
                }
                None => save(self, e),
            },
            Node::Match(..) => match self.reduce_match(e)? {
                Some(m) => {
                    let r = self.whnf_core(m, cheap_proj)?;
                    save(self, r)
                }
                None => save(self, e),
            },
            _ => unreachable!(),
        }
    }

    fn reduce_proj(&mut self, idx: u32, x: ExprId, cheap_proj: bool) -> KResult<Option<ExprId>> {
        let c = if cheap_proj {
            self.whnf_core(x, cheap_proj)?
        } else {
            self.whnf(x)?
        };
        self.reduce_proj_core(idx, c)
    }

    fn reduce_proj_core(&mut self, idx: u32, mut c: ExprId) -> KResult<Option<ExprId>> {
        if let Node::StrLit(sid) = self.s.node(c) {
            let expanded = self.str_lit_to_ctor(sid);
            c = self.whnf(expanded)?;
        }
        let (head, args) = self.s.app_args(c);
        let Node::Const(mk, _) = self.s.node(head) else {
            return Ok(None);
        };
        let Some(ci) = self.env.ctor(mk) else {
            return Ok(None);
        };
        Ok(args.get((ci.num_params + idx) as usize).copied())
    }

    // Inline at its one call site, `whnf_core`'s recursor step: `whnf_core` also carries LOGOS's
    // arms, which put it past the inliner's own budget, and called out of line this step costs the
    // Lean theory's substitution engine about 0.4% of its instructions.
    #[inline(always)]
    fn reduce_recursor(&mut self, e: ExprId) -> KResult<Option<ExprId>> {
        if self.env.quot_init {
            if let Some(r) = self.quot_reduce_rec(e)? {
                return Ok(Some(r));
            }
        }
        self.inductive_reduce_rec(e)
    }

    pub(crate) fn quot_reduce_rec(&mut self, e: ExprId) -> KResult<Option<ExprId>> {
        let Some((fname, _)) = self.s.const_head(e) else {
            return Ok(None);
        };
        let Some(rule) = self.env.quot_rule(fname) else {
            return Ok(None);
        };
        let (_, args) = self.s.app_args(e);
        if rule.major >= args.len() {
            return Ok(None);
        }
        let mk = self.whnf(args[rule.major])?;
        let headed_by_mk =
            matches!(self.s.const_head(mk), Some((m, _)) if self.env.is_quot_mk(&rule, m));
        if !headed_by_mk || self.s.app_num_args(mk) != 3 {
            return Ok(None);
        }
        let Node::App(_, mk_arg) = self.s.node(mk) else {
            unreachable!()
        };
        let mut r = self.s.app(args[rule.fun], mk_arg);
        if rule.major + 1 < args.len() {
            r = self.s.mk_app_n(r, &args[rule.major + 1..]);
        }
        Ok(Some(r))
    }

    pub fn is_app_of_arity(&self, e: ExprId, n: NameId, arity: usize) -> bool {
        match self.s.const_head(e) {
            Some((h, _)) => h == n && self.s.app_num_args(e) == arity,
            None => false,
        }
    }

    fn inductive_reduce_rec(&mut self, e: ExprId) -> KResult<Option<ExprId>> {
        let Some((rec_name, ls)) = self.s.const_head(e) else {
            return Ok(None);
        };
        let Some(c) = self.env.get(rec_name) else {
            return Ok(None);
        };
        let Kind::Rec(_) = &c.kind else {
            return Ok(None);
        };
        let c = c.clone();
        let Kind::Rec(info) = &c.kind else {
            unreachable!()
        };
        let (_, rec_args) = self.s.app_args(e);
        let major_idx = info.major_idx();
        let Some(&major0) = rec_args.get(major_idx) else {
            return Ok(None);
        };
        let mut major = major0;
        if info.k {
            major = self.to_ctor_when_k(info, major)?;
        }
        major = match self.eta_major_first(info, major)? {
            Some(expanded) => expanded,
            None => {
                let wm = self.whnf(major)?;
                match self.s.node(wm) {
                    Node::NatLit(n) if self.reads_nat_literal(self.s.nat(n)) => {
                        let v = self.s.nat(n).clone();
                        self.nat_lit_to_ctor(&v)
                    }
                    Node::StrLit(sid) => {
                        let x = self.str_lit_to_ctor(sid);
                        self.whnf(x)?
                    }
                    _ => self.to_ctor_when_struct(info.major_induct, wm)?,
                }
            }
        };
        let (mhead, margs) = self.s.app_args(major);
        let Node::Const(ctor_name, _) = self.s.node(mhead) else {
            return Ok(None);
        };
        let Some(rule_idx) = info.rules.iter().position(|r| r.ctor == ctor_name) else {
            return Ok(None);
        };
        let rule = &info.rules[rule_idx];
        let nfields = rule.nfields as usize;
        if nfields > margs.len() {
            return Ok(None);
        }
        let us = self.s.levels.get_list(ls).to_vec();
        if us.len() != c.lparams.len() {
            return Ok(None);
        }
        let mut rhs = if c.lparams.is_empty() {
            rule.rhs
        } else if self.env.is_settled(rec_name) {
            match self.global_rule.get(&(rec_name, rule_idx as u32, ls)) {
                Some(r) => *r,
                None => {
                    let (r, persistent) = self
                        .s
                        .with_persistent(|s| s.instantiate_lparams(rule.rhs, &c.lparams, &us));
                    if persistent {
                        self.global_rule.insert((rec_name, rule_idx as u32, ls), r);
                    }
                    r
                }
            }
        } else {
            self.s.instantiate_lparams(rule.rhs, &c.lparams, &us)
        };
        rhs = self.s.mk_app_n(rhs, &rec_args[..info.first_index_idx()]);
        rhs = self.s.mk_app_n(rhs, &margs[margs.len() - nfields..]);
        if major_idx + 1 < rec_args.len() {
            rhs = self.s.mk_app_n(rhs, &rec_args[major_idx + 1..]);
        }
        Ok(Some(rhs))
    }

    fn mk_nullary_ctor(&mut self, ty: ExprId, nparams: usize) -> Option<ExprId> {
        let (head, args) = self.s.app_args(ty);
        let Node::Const(d, ls) = self.s.node(head) else {
            return None;
        };
        let ind = self.env.inductive(d)?;
        let ctor = *ind.ctors.first()?;
        let c = self.s.cnst(ctor, ls);
        Some(self.s.mk_app_n(c, &args[..nparams.min(args.len())]))
    }

    fn to_ctor_when_k(&mut self, info: &RecInfo, e: ExprId) -> KResult<ExprId> {
        let et = self.infer(e, true)?;
        let app_type = self.whnf(et)?;
        let Some((ti, _)) = self.s.const_head(app_type) else {
            return Ok(e);
        };
        if ti != info.major_induct {
            return Ok(e);
        }
        let Some(new_ctor) = self.mk_nullary_ctor(app_type, info.num_params as usize) else {
            return Ok(e);
        };
        let nt = self.infer(new_ctor, true)?;
        if !self.is_def_eq(app_type, nt)? {
            return Ok(e);
        }
        Ok(new_ctor)
    }

    fn is_constructor_app(&self, e: ExprId) -> bool {
        match self.s.const_head(e) {
            Some((n, _)) => self.env.ctor(n).is_some(),
            None => false,
        }
    }

    /// The major premise of a structure's recursor as its constructor applied to its projections, taken before the
    /// major is reduced; `None` where that does not apply. By η a structure's value is that application, so the rule
    /// fires without the major's weak head normal form, which a proof need not have
    /// (`corner-cases/eta-struct-major-no-whnf`). It applies to a structure in a universe that is never `Prop`, and to
    /// one in `Prop` whose own recursor eliminates into every universe (`And`), whose fields are all proofs.
    fn eta_major_first(&mut self, info: &RecInfo, major: ExprId) -> KResult<Option<ExprId>> {
        if !self.env.is_non_rec_structure(info.major_induct)
            || matches!(self.s.node(major), Node::NatLit(_) | Node::StrLit(_))
            || self.is_constructor_app(major)
        {
            return Ok(None);
        }
        let et = self.infer(major, true)?;
        let ety = self.whnf(et)?;
        let (head, args) = self.s.app_args(ety);
        let Node::Const(i, ls) = self.s.node(head) else {
            return Ok(None);
        };
        if i != info.major_induct {
            return Ok(None);
        }
        let tt = self.infer(ety, true)?;
        let sort = self.whnf(tt)?;
        let Node::Sort(u) = self.s.node(sort) else {
            return Ok(None);
        };
        let levels = &self.s.levels;
        if !(levels.is_never_zero(u) || (levels.is_always_zero(u) && self.env.eliminates_into_every_sort(info.major_induct))) {
            return Ok(None);
        }
        Ok(Some(self.expand_eta_struct(i, ls, &args, major)))
    }

    fn to_ctor_when_struct(&mut self, induct: NameId, e: ExprId) -> KResult<ExprId> {
        if !self.env.is_non_rec_structure(induct) || self.is_constructor_app(e) {
            return Ok(e);
        }
        let et = self.infer(e, true)?;
        let ety = self.whnf(et)?;
        let (head, args) = self.s.app_args(ety);
        let Node::Const(i, ls) = self.s.node(head) else {
            return Ok(e);
        };
        if i != induct {
            return Ok(e);
        }
        let tt = self.infer(ety, true)?;
        let sort = self.whnf(tt)?;
        let Node::Sort(u) = self.s.node(sort) else {
            return Ok(e);
        };
        if !self.s.levels.is_never_zero(u) {
            return Ok(e);
        }
        Ok(self.expand_eta_struct(i, ls, &args, e))
    }

    fn expand_eta_struct(&mut self, i: NameId, ls: LevelsId, args: &[ExprId], e: ExprId) -> ExprId {
        let ctor = self.env.inductive(i).unwrap().ctors[0];
        let ci = self.env.ctor(ctor).unwrap().clone();
        let c = self.s.cnst(ctor, ls);
        let mut r = self
            .s
            .mk_app_n(c, &args[..(ci.num_params as usize).min(args.len())]);
        for f in 0..ci.num_fields {
            let p = self.s.proj(i, f, e);
            r = self.s.app(r, p);
        }
        r
    }

    pub fn nat_lit_to_ctor(&mut self, n: &Nat) -> ExprId {
        if n.is_zero() {
            return self.s.cnst(self.wk.nat_zero, LevelsId::EMPTY);
        }
        let succ = self.s.cnst(self.wk.nat_succ, LevelsId::EMPTY);
        let pred = self.s.nat_lit(n.sub(&Nat::from_u64(1)));
        self.s.app(succ, pred)
    }

    pub fn str_lit_to_ctor(&mut self, sid: super::expr::StrId) -> ExprId {
        let st = self.s.string(sid);
        let zero = self.s.levels.list(&[LevelId::ZERO]);
        let ch = self.s.cnst(self.wk.char_, LevelsId::EMPTY);
        let nil_c = self.s.cnst(self.wk.list_nil, zero);
        let nil = self.s.app(nil_c, ch);
        let cons_c = self.s.cnst(self.wk.list_cons, zero);
        let cons = self.s.app(cons_c, ch);
        let of_nat = self.s.cnst(self.wk.char_of_nat, LevelsId::EMPTY);
        let mut list = nil;
        for c in st.chars().rev() {
            let lit = self.s.nat_lit(Nat::from_u64(c as u64));
            let chr = self.s.app(of_nat, lit);
            let cell = self.s.app(cons, chr);
            list = self.s.app(cell, list);
        }
        let of_list = self.s.cnst(self.wk.string_of_list, LevelsId::EMPTY);
        self.s.app(of_list, list)
    }

    /// The head constant of `e` if it δ-unfolds at the given universe arguments.
    fn is_delta(&self, e: ExprId) -> Option<Rc<Const>> {
        let (n, ls) = self.s.const_head(e)?;
        let c = self.env.get(n)?;
        if c.delta_value().is_some() && self.s.levels.get_list(ls).len() == c.lparams.len() {
            Some(c.clone())
        } else {
            None
        }
    }

    fn unfold_definition_core(&mut self, e: ExprId) -> Option<ExprId> {
        let Node::Const(n, ls) = self.s.node(e) else {
            return None;
        };
        let c = self.is_delta(e)?;
        let v = c.delta_value().unwrap();
        if ls == LevelsId::EMPTY || c.lparams.is_empty() {
            return Some(v);
        }
        if !self.env.is_settled(n) {
            if let Some(r) = self.unfold_cache.get(&e) {
                return Some(*r);
            }
            let us = self.s.levels.get_list(ls).to_vec();
            let r = self.s.instantiate_lparams(v, &c.lparams, &us);
            self.unfold_cache.insert(e, r);
            return Some(r);
        }
        if let Some(r) = self.global_value.get(&(n, ls)) {
            return Some(*r);
        }
        let us = self.s.levels.get_list(ls).to_vec();
        let (r, persistent) = self
            .s
            .with_persistent(|s| s.instantiate_lparams(v, &c.lparams, &us));
        if persistent {
            self.global_value.insert((n, ls), r);
        } else {
            self.unfold_cache.insert(e, r);
        }
        Some(r)
    }

    pub fn unfold_definition(&mut self, e: ExprId) -> Option<ExprId> {
        let unfolded = if let Node::App(..) = self.s.node(e) {
            let (f0, args) = self.s.app_args(e);
            let f = self.unfold_definition_core(f0)?;
            Some(self.s.mk_app_n(f, &args))
        } else {
            self.unfold_definition_core(e)
        };
        self.unfold_steps += u64::from(unfolded.is_some());
        unfolded
    }

    fn raw_nat_lit_ext(&self, e: ExprId) -> Option<Nat> {
        match self.s.node(e) {
            Node::NatLit(n) => Some(self.s.nat(n).clone()),
            Node::Const(n, ls) if n == self.wk.nat_zero && ls == LevelsId::EMPTY => {
                Some(Nat::zero())
            }
            _ => None,
        }
    }

    pub(super) fn nat_bin_args(&mut self, a: ExprId, b: ExprId) -> KResult<Option<(Nat, Nat)>> {
        let wa = self.whnf(a)?;
        let Some(x) = self.raw_nat_lit_ext(wa) else {
            return Ok(None);
        };
        let wb = self.whnf(b)?;
        let Some(y) = self.raw_nat_lit_ext(wb) else {
            return Ok(None);
        };
        Ok(Some((x, y)))
    }

    fn bool_lit(&mut self, b: bool) -> ExprId {
        let n = if b {
            self.wk.bool_true
        } else {
            self.wk.bool_false
        };
        self.s.cnst(n, LevelsId::EMPTY)
    }

    /// Lean's `reduceNat`: `Nat.succ` and the GMP-accelerated binary operations on literals.
    pub fn reduce_nat(&mut self, e: ExprId) -> KResult<Option<ExprId>> {
        if !self.nat_literal_rules() {
            return Ok(None);
        }
        let Node::App(f, a1) = self.s.node(e) else {
            return Ok(None);
        };
        if let Node::Const(n, ls) = self.s.node(f) {
            if n == self.wk.nat_succ && ls == LevelsId::EMPTY {
                let w = self.whnf(a1)?;
                return Ok(self
                    .raw_nat_lit_ext(w)
                    .map(|v| v.succ())
                    .map(|v| self.s.nat_lit(v)));
            }
            return Ok(None);
        }
        let Node::App(g, a0) = self.s.node(f) else {
            return Ok(None);
        };
        let Node::Const(op, _) = self.s.node(g) else {
            return Ok(None);
        };
        let w = &self.wk;
        let kind = if op == w.nat_add {
            0
        } else if op == w.nat_sub {
            1
        } else if op == w.nat_mul {
            2
        } else if op == w.nat_pow {
            3
        } else if op == w.nat_gcd {
            4
        } else if op == w.nat_mod {
            5
        } else if op == w.nat_div {
            6
        } else if op == w.nat_beq {
            7
        } else if op == w.nat_ble {
            8
        } else if op == w.nat_land {
            9
        } else if op == w.nat_lor {
            10
        } else if op == w.nat_xor {
            11
        } else if op == w.nat_shift_left {
            12
        } else if op == w.nat_shift_right {
            13
        } else {
            return Ok(None);
        };
        if !self.validated_primitives.contains(&op) {
            return Ok(None);
        }
        let Some((x, y)) = self.nat_bin_args(a0, a1)? else {
            return Ok(None);
        };
        let r = match kind {
            0 => x.add(&y),
            1 => x.sub(&y),
            2 => x.mul(&y),
            3 => match y.to_u64() {
                Some(k) if k <= 1 << 24 => x.pow(k as u32),
                _ => return Ok(None),
            },
            4 => x.gcd(&y),
            5 => x.rem(&y),
            6 => x.div(&y),
            7 => return Ok(Some(self.bool_lit(x == y))),
            8 => return Ok(Some(self.bool_lit(x <= y))),
            9 => x.land(&y),
            10 => x.lor(&y),
            11 => x.xor(&y),
            12 => match y.to_u64() {
                Some(k) if k <= 1 << 24 => x.shl(k),
                _ => return Ok(None),
            },
            13 => match y.to_u64() {
                Some(k) => x.shr(k),
                None => Nat::zero(),
            },
            _ => unreachable!(),
        };
        Ok(Some(self.s.nat_lit(r)))
    }

    /// What the theory computes natively on literals: a validated primitive, in either theory (in
    /// LOGOS's, Lean content's `Nat` operations, checked as in Lean's by `check_primitive_def`), and
    /// in LOGOS's a certified program ([`Kernel::reduce_certified_program`]).
    #[inline]
    fn reduce_native(&mut self, e: ExprId) -> KResult<Option<ExprId>> {
        if let Some(r) = self.reduce_nat(e)? {
            return Ok(Some(r));
        }
        if self.theory == Theory::Logos {
            return self.reduce_certified_program(e);
        }
        Ok(None)
    }

    pub fn whnf(&mut self, e: ExprId) -> KResult<ExprId> {
        match self.s.node(e) {
            Node::BVar(_)
            | Node::Sort(_)
            | Node::Pi(..)
            | Node::NatLit(_)
            | Node::StrLit(_)
            | Node::Lit(_) => return Ok(e),
            Node::FVar(_) => {
                if self.is_let_fvar(e).is_none() {
                    return Ok(e);
                }
            }
            _ => {}
        }
        if let Some(r) = self.whnf_cache.get(&e) {
            return Ok(*r);
        }
        self.enter()?;
        let r = self.whnf_loop(e);
        self.leave();
        let r = r?;
        self.whnf_cache.insert(e, r);
        Ok(r)
    }

    fn whnf_loop(&mut self, e: ExprId) -> KResult<ExprId> {
        let mut t = e;
        loop {
            t = self.whnf_core(t, false)?;
            if self.theory == Theory::Logos {
                if let Some(r) = self.reduce_builtin(t)? {
                    return Ok(r);
                }
            }
            if let Some(r) = self.reduce_native(t)? {
                return Ok(r);
            }
            match self.unfold_definition(t) {
                Some(u) => t = u,
                None => return Ok(t),
            }
        }
    }

    // ---- definitional equality ----

    /// Whether a term of type `actual` may stand where `expected` is required: definitional
    /// equality in Lean's theory, cumulative subtyping in LOGOS's.
    #[inline]
    pub fn conforms(&mut self, actual: ExprId, expected: ExprId) -> KResult<bool> {
        match self.theory {
            Theory::Lean => self.is_def_eq(actual, expected),
            Theory::Logos => self.is_subtype(actual, expected),
        }
    }

    /// [`Kernel::conforms`] for an argument of type `arg` against a function's domain `dom`.
    #[inline]
    fn accepts_argument(&mut self, dom: ExprId, arg: ExprId) -> KResult<bool> {
        match self.theory {
            Theory::Lean => self.is_def_eq(dom, arg),
            Theory::Logos => self.is_subtype(arg, dom),
        }
    }

    pub fn is_def_eq(&mut self, t: ExprId, s: ExprId) -> KResult<bool> {
        if t == s {
            return Ok(true);
        }
        #[cfg(feature = "nbe")]
        if self.dual_conv {
            let classic = self.decide(t, s)?;
            let mut st = std::mem::take(&mut self.nbe);
            st.eager = self.eager;
            let other = crate::dag::nbe::Nbe::new(self, &mut st).is_def_eq(t, s);
            self.nbe = st;
            let other = other?;
            if classic != other {
                return Err(KernelFailure::resource(format!(
                    "the two conversion engines disagree: substitution {classic}, closure {other}"
                ))
                .with_detail(format!("{} =?= {}", self.show(t), self.show(s))));
            }
            return Ok(classic);
        }
        #[cfg(feature = "nbe")]
        if self.use_nbe {
            let mut st = std::mem::take(&mut self.nbe);
            st.eager = self.eager;
            let r = crate::dag::nbe::Nbe::new(self, &mut st).is_def_eq(t, s);
            self.nbe = st;
            return r;
        }
        self.decide(t, s)
    }

    /// `t ≡ s` (`t ≠ s`) by the algorithm, its answer remembered: a proof as a proven pair, a
    /// refutation as a refuted pair of the current mode and proof generation (see `refuted`).
    fn decide(&mut self, t: ExprId, s: ExprId) -> KResult<bool> {
        let key = if t <= s {
            (t, s, self.eager)
        } else {
            (s, t, self.eager)
        };
        let stamp = self.eqv.generation;
        if self.refuted.get(&key) == Some(&stamp) {
            return Ok(false);
        }
        self.enter()?;
        let r = self.is_def_eq_core(t, s);
        self.leave();
        let r = r?;
        if r {
            self.eqv.insert(t, s);
        } else {
            self.refuted.insert(key, stamp);
        }
        Ok(r)
    }

    /// Equal, proven equal, or congruent: same shape with children pairwise equal or proven equal.
    fn eqv_is_equiv(&mut self, a: ExprId, b: ExprId) -> bool {
        if a == b || self.eqv.contains(a, b) {
            return true;
        }
        let r = match (self.s.node(a), self.s.node(b)) {
            (Node::App(f1, a1), Node::App(f2, a2)) => {
                self.eqv_is_equiv(f1, f2) && self.eqv_is_equiv(a1, a2)
            }
            (Node::Lam(d1, b1), Node::Lam(d2, b2)) | (Node::Pi(d1, b1), Node::Pi(d2, b2)) => {
                self.eqv_is_equiv(d1, d2) && self.eqv_is_equiv(b1, b2)
            }
            (Node::Proj(_, i1, e1), Node::Proj(_, i2, e2)) => i1 == i2 && self.eqv_is_equiv(e1, e2),
            (Node::Let(t1, v1, b1), Node::Let(t2, v2, b2)) => {
                self.eqv_is_equiv(t1, t2) && self.eqv_is_equiv(v1, v2) && self.eqv_is_equiv(b1, b2)
            }
            _ => false,
        };
        if r {
            self.eqv.insert(a, b);
        }
        r
    }

    fn quick_is_def_eq(&mut self, t: ExprId, s: ExprId) -> KResult<LBool> {
        if self.eqv_is_equiv(t, s) {
            return Ok(LBool::True);
        }
        Ok(match (self.s.node(t), self.s.node(s)) {
            (Node::Lam(..), Node::Lam(..)) => self.is_def_eq_binding(t, s, false)?.into(),
            (Node::Pi(..), Node::Pi(..)) => self.is_def_eq_binding(t, s, true)?.into(),
            (Node::Sort(a), Node::Sort(b)) => self.s.levels.equiv(a, b).into(),
            (Node::NatLit(_) | Node::StrLit(_), Node::NatLit(_) | Node::StrLit(_)) => {
                (t == s).into()
            }
            _ => LBool::Undef,
        })
    }

    fn is_def_eq_binding(&mut self, mut t: ExprId, mut s: ExprId, pi: bool) -> KResult<bool> {
        let mut subst: Vec<ExprId> = Vec::new();
        loop {
            let (tn, sn) = (self.s.node(t), self.s.node(s));
            let (td, tb, sd, sb) = match (tn, sn) {
                (Node::Lam(td, tb), Node::Lam(sd, sb)) if !pi => (td, tb, sd, sb),
                (Node::Pi(td, tb), Node::Pi(sd, sb)) if pi => (td, tb, sd, sb),
                _ => break,
            };
            let mut s_type = None;
            if td != sd {
                let st = self.s.instantiate_rev(sd, &subst);
                let tt = self.s.instantiate_rev(td, &subst);
                if !self.is_def_eq(tt, st)? {
                    return Ok(false);
                }
                s_type = Some(st);
            }
            if self.s.has_loose_bvars(tb) || self.s.has_loose_bvars(sb) {
                let st = match s_type {
                    Some(st) => st,
                    None => self.s.instantiate_rev(sd, &subst),
                };
                let fv = self.push_local(st, None);
                subst.push(fv);
            } else {
                let dummy = self.s.prop();
                subst.push(dummy);
            }
            t = tb;
            s = sb;
        }
        let ti = self.s.instantiate_rev(t, &subst);
        let si = self.s.instantiate_rev(s, &subst);
        self.is_def_eq(ti, si)
    }

    /// The arguments of two applications of the same head, pairwise, last to first — skipping
    /// the positions the head's definition cannot distinguish (see [`super::relevance`]).
    fn is_def_eq_args(&mut self, t: ExprId, s: ExprId) -> KResult<bool> {
        let n = self.s.app_num_args(t);
        if n != self.s.app_num_args(s) {
            return Ok(false);
        }
        let mask = match self.s.const_head(t) {
            Some((c, ls)) => self.ignorable_args(c, ls)?,
            None => 0,
        };
        let (mut t, mut s) = (t, s);
        let mut i = n;
        while let (Node::App(tf, ta), Node::App(sf, sa)) = (self.s.node(t), self.s.node(s)) {
            i -= 1;
            let ignorable = i < super::relevance::MAX_TRACKED && (mask >> i) & 1 == 1;
            if !ignorable && !self.is_def_eq(ta, sa)? {
                return Ok(false);
            }
            t = tf;
            s = sf;
        }
        Ok(true)
    }

    fn try_eta_expansion_core(&mut self, t: ExprId, s: ExprId) -> KResult<bool> {
        if !matches!(self.s.node(t), Node::Lam(..)) || matches!(self.s.node(s), Node::Lam(..)) {
            return Ok(false);
        }
        let st = self.infer(s, true)?;
        let w = self.whnf(st)?;
        let Node::Pi(dom, _) = self.s.node(w) else {
            return Ok(false);
        };
        let b0 = self.s.bvar(0);
        let body = self.s.app(s, b0);
        let lam = self.s.lam(dom, body);
        self.is_def_eq(t, lam)
    }

    fn try_eta_struct_core(&mut self, t: ExprId, s: ExprId) -> KResult<bool> {
        if self.theory == Theory::Logos {
            return self.logos_eta_struct(t, s);
        }
        let Some((f, _)) = self.s.const_head(s) else {
            return Ok(false);
        };
        let Some(ci) = self.env.ctor(f).cloned() else {
            return Ok(false);
        };
        let (_, args) = self.s.app_args(s);
        if args.len() != (ci.num_params + ci.num_fields) as usize {
            return Ok(false);
        }
        if !self.env.is_non_rec_structure(ci.induct) {
            return Ok(false);
        }
        let tt = self.infer(t, true)?;
        let st = self.infer(s, true)?;
        if !self.is_def_eq(tt, st)? {
            return Ok(false);
        }
        for i in ci.num_params as usize..args.len() {
            let p = self
                .s
                .proj(ci.induct, (i - ci.num_params as usize) as u32, t);
            if !self.is_def_eq(p, args[i])? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn is_def_eq_app(&mut self, t: ExprId, s: ExprId) -> KResult<bool> {
        if !matches!(self.s.node(t), Node::App(..)) || !matches!(self.s.node(s), Node::App(..)) {
            return Ok(false);
        }
        let (tf, targs) = self.s.app_args(t);
        let (sf, sargs) = self.s.app_args(s);
        if targs.len() != sargs.len() {
            return Ok(false);
        }
        if !self.is_def_eq(tf, sf)? {
            return Ok(false);
        }
        for (a, b) in targs.iter().zip(&sargs) {
            if !self.is_def_eq(*a, *b)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn is_def_eq_proof_irrel(&mut self, t: ExprId, s: ExprId) -> KResult<LBool> {
        let tt = self.infer(t, true)?;
        if !self.is_prop(tt)? {
            return Ok(LBool::Undef);
        }
        let st = self.infer(s, true)?;
        Ok(self.is_def_eq(tt, st)?.into())
    }

    fn failed_before(&self, t: ExprId, s: ExprId) -> bool {
        let k = if t <= s { (t, s) } else { (s, t) };
        self.failure.contains(&k)
    }

    fn cache_failure(&mut self, t: ExprId, s: ExprId) {
        let k = if t <= s { (t, s) } else { (s, t) };
        self.failure.insert(k);
    }

    fn try_unfold_proj_app(&mut self, e: ExprId) -> KResult<Option<ExprId>> {
        let f = self.s.app_fn(e);
        if !matches!(self.s.node(f), Node::Proj(..)) {
            return Ok(None);
        }
        let e2 = self.whnf_core(e, false)?;
        Ok((e2 != e).then_some(e2))
    }

    fn delta(&mut self, e: ExprId) -> KResult<ExprId> {
        let u = self
            .unfold_definition(e)
            .expect("delta on a non-unfoldable head");
        self.whnf_core(u, true)
    }

    fn lazy_delta_step(&mut self, tn: ExprId, sn: ExprId) -> KResult<Step> {
        let cont = |k: &mut Kernel, tn: ExprId, sn: ExprId| -> KResult<Step> {
            Ok(match k.quick_is_def_eq(tn, sn)? {
                LBool::Undef => Step::Continue(tn, sn),
                LBool::True => Step::True,
                LBool::False => Step::False(tn, sn),
            })
        };
        match (self.is_delta(tn), self.is_delta(sn)) {
            (None, None) => Ok(Step::Unknown(tn, sn)),
            (Some(_), None) => {
                if let Some(sn2) = self.try_unfold_proj_app(sn)? {
                    cont(self, tn, sn2)
                } else {
                    let tn2 = self.delta(tn)?;
                    cont(self, tn2, sn)
                }
            }
            (None, Some(_)) => {
                if let Some(tn2) = self.try_unfold_proj_app(tn)? {
                    cont(self, tn2, sn)
                } else {
                    let sn2 = self.delta(sn)?;
                    cont(self, tn, sn2)
                }
            }
            (Some(dt), Some(ds)) => {
                let (ht, hs) = (dt.hint(), ds.hint());
                if ht.lt(hs) {
                    let sn2 = self.delta(sn)?;
                    cont(self, tn, sn2)
                } else if hs.lt(ht) {
                    let tn2 = self.delta(tn)?;
                    cont(self, tn2, sn)
                } else {
                    if matches!(self.s.node(tn), Node::App(..))
                        && matches!(self.s.node(sn), Node::App(..))
                        && dt.name == ds.name
                        && matches!(ht, Hint::Regular(_))
                        && !self.failed_before(tn, sn)
                    {
                        let (_, tl) = self.s.const_head(tn).unwrap();
                        let (_, sl) = self.s.const_head(sn).unwrap();
                        if self.s.levels.equiv_lists(tl, sl) && self.is_def_eq_args(tn, sn)? {
                            return Ok(Step::True);
                        }
                        self.cache_failure(tn, sn);
                    }
                    let tn2 = self.delta(tn)?;
                    let sn2 = self.delta(sn)?;
                    cont(self, tn2, sn2)
                }
            }
        }
    }

    fn is_nat_zero(&self, e: ExprId) -> bool {
        match self.s.node(e) {
            Node::Const(n, ls) => n == self.wk.nat_zero && ls == LevelsId::EMPTY,
            Node::NatLit(n) => self.s.nat(n).is_zero(),
            _ => false,
        }
    }

    fn nat_succ_of(&mut self, e: ExprId) -> Option<ExprId> {
        match self.s.node(e) {
            Node::NatLit(n) => {
                let v = self.s.nat(n).clone();
                if v.is_zero() {
                    None
                } else {
                    Some(self.s.nat_lit(v.sub(&Nat::from_u64(1))))
                }
            }
            Node::App(f, a) => match self.s.node(f) {
                Node::Const(n, _) if n == self.wk.nat_succ => Some(a),
                _ => None,
            },
            _ => None,
        }
    }

    fn is_def_eq_offset(&mut self, t: ExprId, s: ExprId) -> KResult<LBool> {
        if self.is_nat_zero(t) && self.is_nat_zero(s) {
            return Ok(if self.reads_nat_literal(&Nat::zero()) {
                LBool::True
            } else {
                LBool::Undef
            });
        }
        if !self.nat_literal_rules() {
            return Ok(LBool::Undef);
        }
        match (self.nat_succ_of(t), self.nat_succ_of(s)) {
            (Some(a), Some(b)) => Ok(self.is_def_eq_core_guarded(a, b)?.into()),
            _ => Ok(LBool::Undef),
        }
    }

    fn is_def_eq_core_guarded(&mut self, t: ExprId, s: ExprId) -> KResult<bool> {
        if t == s {
            return Ok(true);
        }
        self.enter()?;
        let r = self.is_def_eq_core(t, s);
        self.leave();
        r
    }

    fn lazy_delta_reduction(&mut self, mut tn: ExprId, mut sn: ExprId) -> KResult<Step> {
        loop {
            match self.is_def_eq_offset(tn, sn)? {
                LBool::True => return Ok(Step::True),
                LBool::False => return Ok(Step::False(tn, sn)),
                LBool::Undef => {}
            }
            if (!self.s.has_fvar(tn) && !self.s.has_fvar(sn)) || self.eager {
                if let Some(tn2) = self.reduce_native(tn)? {
                    return Ok(if self.is_def_eq_core_guarded(tn2, sn)? {
                        Step::True
                    } else {
                        Step::False(tn2, sn)
                    });
                } else if let Some(sn2) = self.reduce_native(sn)? {
                    return Ok(if self.is_def_eq_core_guarded(tn, sn2)? {
                        Step::True
                    } else {
                        Step::False(tn, sn2)
                    });
                }
            }
            match self.lazy_delta_step(tn, sn)? {
                Step::Continue(a, b) => {
                    tn = a;
                    sn = b;
                }
                r => return Ok(r),
            }
        }
    }

    fn lazy_delta_proj_reduction(
        &mut self,
        mut tn: ExprId,
        mut sn: ExprId,
        idx: u32,
    ) -> KResult<bool> {
        loop {
            match self.lazy_delta_step(tn, sn)? {
                Step::Continue(a, b) => {
                    tn = a;
                    sn = b;
                }
                Step::True => return Ok(true),
                Step::Unknown(a, b) | Step::False(a, b) => {
                    if let Some(tf) = self.reduce_proj_core(idx, a)? {
                        if let Some(sf) = self.reduce_proj_core(idx, b)? {
                            return self.is_def_eq_core_guarded(tf, sf);
                        }
                    }
                    return self.is_def_eq_core_guarded(a, b);
                }
            }
        }
    }

    fn try_string_lit_expansion_core(&mut self, t: ExprId, s: ExprId) -> KResult<LBool> {
        let Node::StrLit(sid) = self.s.node(t) else {
            return Ok(LBool::Undef);
        };
        let Node::App(sf, _) = self.s.node(s) else {
            return Ok(LBool::Undef);
        };
        match self.s.node(sf) {
            Node::Const(n, ls) if n == self.wk.string_of_list && ls == LevelsId::EMPTY => {}
            _ => return Ok(LBool::Undef),
        }
        let x = self.str_lit_to_ctor(sid);
        Ok(self.is_def_eq_core_guarded(x, s)?.into())
    }

    fn is_def_eq_unit_like(&mut self, t: ExprId, s: ExprId) -> KResult<bool> {
        let tt = self.infer(t, true)?;
        let tty = self.whnf(tt)?;
        let Some((i, _)) = self.s.const_head(tty) else {
            return Ok(false);
        };
        let Some(ind) = self.env.inductive(i) else {
            return Ok(false);
        };
        if ind.is_rec || ind.ctors.len() != 1 || ind.num_indices != 0 {
            return Ok(false);
        }
        let c = ind.ctors[0];
        if self.env.ctor(c).map(|ci| ci.num_fields) != Some(0) {
            return Ok(false);
        }
        let st = self.infer(s, true)?;
        self.is_def_eq_core_guarded(tty, st)
    }

    /// The δ-unfoldings this kernel's weak-head reduction has taken: a primitive at literals computed
    /// natively takes none.
    pub fn unfold_steps(&self) -> u64 {
        self.unfold_steps
    }

    /// The definitional-equality steps this kernel has taken (see `conv_steps`).
    pub fn conversion_steps(&self) -> u64 {
        self.conv_steps
    }

    /// Whether `n` is a `true` the `decide` fast path reduces the other side for: `Bool.true`, and
    /// in LOGOS's theory LOGOS's own `true`.
    pub(crate) fn is_true_constant(&self, n: NameId) -> bool {
        n == self.wk.bool_true || self.wk.logos_true == Some(n)
    }

    fn is_def_eq_core(&mut self, t: ExprId, s: ExprId) -> KResult<bool> {
        self.conv_steps += 1;
        match self.quick_is_def_eq(t, s)? {
            LBool::True => return Ok(true),
            LBool::False => return Ok(false),
            LBool::Undef => {}
        }
        if (!self.s.has_fvar(t) || self.eager)
            && matches!(self.s.node(s), Node::Const(n, _) if self.is_true_constant(n))
        {
            let w = self.whnf(t)?;
            if self.s.node(w) == self.s.node(s) {
                return Ok(true);
            }
        }
        let tn = self.whnf_core(t, true)?;
        let sn = self.whnf_core(s, true)?;
        if tn != t || sn != s {
            match self.quick_is_def_eq(tn, sn)? {
                LBool::True => return Ok(true),
                LBool::False => return Ok(false),
                LBool::Undef => {}
            }
        }
        match self.is_def_eq_proof_irrel(tn, sn)? {
            LBool::True => return Ok(true),
            LBool::False => return Ok(false),
            LBool::Undef => {}
        }
        let (tn, sn) = match self.lazy_delta_reduction(tn, sn)? {
            Step::True => return Ok(true),
            Step::False(..) => return Ok(false),
            Step::Unknown(a, b) => (a, b),
            Step::Continue(..) => unreachable!(),
        };
        match (self.s.node(tn), self.s.node(sn)) {
            (Node::Const(tf, tl), Node::Const(sf, sl)) => {
                if tf == sf && self.s.levels.equiv_lists(tl, sl) {
                    return Ok(true);
                }
            }
            (Node::FVar(a), Node::FVar(b)) => {
                if a == b {
                    return Ok(true);
                }
            }
            (Node::Proj(_, ti, te), Node::Proj(_, si, se)) => {
                if ti == si && self.lazy_delta_proj_reduction(te, se, ti)? {
                    return Ok(true);
                }
            }
            _ => {}
        }
        let tnn = self.whnf_core(tn, false)?;
        let snn = self.whnf_core(sn, false)?;
        if tnn != tn || snn != sn {
            return self.is_def_eq_core_guarded(tnn, snn);
        }
        if self.is_def_eq_app(tn, sn)? {
            return Ok(true);
        }
        if self.try_eta_expansion_core(tn, sn)? || self.try_eta_expansion_core(sn, tn)? {
            return Ok(true);
        }
        if self.try_eta_struct_core(tn, sn)? || self.try_eta_struct_core(sn, tn)? {
            return Ok(true);
        }
        match self.try_string_lit_expansion_core(tn, sn)? {
            LBool::True => return Ok(true),
            LBool::False => return Ok(false),
            LBool::Undef => match self.try_string_lit_expansion_core(sn, tn)? {
                LBool::True => return Ok(true),
                LBool::False => return Ok(false),
                LBool::Undef => {}
            },
        }
        if self.theory == Theory::Logos && self.logos_def_eq(tn, sn)? {
            return Ok(true);
        }
        if self.is_def_eq_unit_like(tn, sn)? {
            return Ok(true);
        }
        Ok(false)
    }

    // ---- presentation ----

    pub fn show(&self, e: ExprId) -> String {
        super::pretty::show(self, e)
    }

    fn describe_mismatch(&self, e: ExprId, expected: ExprId, found: ExprId) -> String {
        format!(
            "in `{}`\n  expected type: {}\n  but the argument has type: {}",
            self.show(e),
            self.show(expected),
            self.show(found)
        )
    }

    pub fn level_node(&self, l: LevelId) -> LevelNode {
        self.s.levels.node(l)
    }

    pub fn quot_kind(&self, n: NameId) -> Option<QuotKind> {
        self.env.quot_kind(n)
    }
}

impl Default for Kernel {
    fn default() -> Self {
        Self::new()
    }
}

fn clear_map(m: &mut FxMap<ExprId, ExprId>) {
    if m.capacity() > 1 << 16 {
        *m = FxMap::default();
    } else {
        m.clear();
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_kernel/dag/tc/tests.rs"]
mod tests;
