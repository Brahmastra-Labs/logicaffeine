//! Evaluation: terms to values, application, and the values of constants.
//!
//! Evaluation is call-by-value and performs β only. An application of a variable, a constant or
//! a stuck projection becomes a neutral; δ and ι are left to weak-head reduction. So evaluating a
//! term costs time proportional to the part of it outside every binder — a lambda is a closure
//! whose body is not looked at until the closure is applied.
//!
//! An expression is evaluated in its environment *pruned* to the entries it can see (the others
//! are replaced by a placeholder), so that two environments agreeing on those entries give the
//! same key for the evaluation cache and the same closure, and a closure is open exactly when
//! something it can see is.
//!
//! A universe-polymorphic constant's type, body and recursor rules are evaluated from its own
//! terms under the substitution of one use's levels for its parameters, which the environment
//! carries: sorts and constants take their levels through it as evaluation reaches them, and
//! nothing is instantiated in advance. A term that mentions no universe parameter is read under
//! the identity, so its value is shared by every instantiation.

use super::value::{EnvId, Head, Kind, Val, DEAD, V};
use super::{reject, Sem};
use crate::dag::env::Kind as CKind;
use crate::dag::error::KResult;
use crate::dag::expr::{ExprId, Node, SHORT_SPINE};
use crate::dag::level::{LevelId, LevelsId, SubstId};
use crate::dag::name::NameId;

impl Sem<'_> {
    /// `env` pruned for `e`: only the entries `e` can observe (its loose variables). A closed
    /// term is read under its environment's substitution only if it mentions a universe
    /// parameter, so its value is shared by every instantiation; an open term keeps the
    /// substitution (re-rooting its entries under the identity costs more than it shares).
    #[inline]
    pub(crate) fn env_for(&mut self, env: EnvId, e: ExprId) -> EnvId {
        let scope = self.k.s.scope_of(e);
        self.env_in_scope(env, e, scope)
    }

    /// [`Sem::env_for`] given `e`'s scope as [`Store::scope_of`] reads it.
    ///
    /// [`Store::scope_of`]: crate::dag::expr::Store::scope_of
    #[inline]
    pub(crate) fn env_in_scope(
        &mut self,
        env: EnvId,
        e: ExprId,
        (n, mask, lparams): (u32, Option<u64>, bool),
    ) -> EnvId {
        let node = self.st.arena.env(env);
        if n == 0 {
            return self.st.arena.lsub_base(if lparams {
                node.lsub
            } else {
                SubstId::IDENTITY
            });
        }
        let lsub = node.lsub;
        let Some(mask) = mask else {
            return self.prune_wide(env, e, lsub);
        };
        if n == node.len && node.live == mask {
            return env;
        }
        if let Some(r) = self.st.arena.last_prune(env, mask) {
            return r;
        }
        self.st.arena.prune(env, n, mask)
    }

    /// [`Sem::env_in_scope`] where the environment itself answers, with nothing to build and no
    /// table to fill: the base of a closed term's substitution, the environment when the term
    /// sees all of it, or one of its two latest prunings when that was to the term's mask. `None`
    /// otherwise.
    #[cfg_attr(debug_assertions, inline)]
    #[cfg_attr(not(debug_assertions), inline(always))]
    pub(crate) fn known_env(
        &self,
        env: EnvId,
        (n, mask, lparams): (u32, Option<u64>, bool),
    ) -> Option<EnvId> {
        let node = self.st.arena.env(env);
        if n == 0 {
            return self.st.arena.find_lsub_base(if lparams {
                node.lsub
            } else {
                SubstId::IDENTITY
            });
        }
        let mask = mask?;
        if n == node.len && node.live == mask {
            return Some(env);
        }
        self.st.arena.last_prune(env, mask)
    }

    /// `env` pruned for an expression whose loose variables are not all below 64 (or whose mask
    /// is not exact): by its exact set of loose variables, so that closures over deeply nested
    /// terms still capture only what they can see and stay one value per meaning. An environment
    /// the expression sees entirely — no entry dead, every index loose — is its own pruning, as
    /// in the narrow case; copying it would cost its length at every level of a deep nest.
    #[cold]
    #[inline(never)]
    fn prune_wide(&mut self, env: EnvId, e: ExprId, lsub: SubstId) -> EnvId {
        let set = self.loose_set(e);
        let node = self.st.arena.env(env);
        if node.dense && set.len() == node.len as usize && node.lsub == lsub {
            return env;
        }
        let key = (env.0, e.0);
        if let Some(r) = self.st.wide_prune_cache.get(&key) {
            return EnvId(r);
        }
        let Some(&last) = set.last() else {
            return self.st.arena.lsub_base(lsub);
        };
        let mut vals: smallvec::SmallVec<[V; 16]> = smallvec::SmallVec::new();
        self.st.arena.select_indices(env, &set, &mut vals);
        let r = if last < 64 {
            let mask = set.iter().fold(0u64, |m, &i| m | (1 << i));
            self.st.arena.frame(last + 1, mask, &vals, lsub)
        } else {
            self.st.arena.wide_frame(last + 1, &set, &vals, lsub)
        };
        self.st.wide_prune_cache.insert(key, r.0);
        r
    }

    /// The loose de Bruijn indices of `e`, exactly, ascending (for expressions the store's
    /// 64-bit mask cannot describe). Computed once per persistent expression.
    pub(crate) fn loose_set(&mut self, e: ExprId) -> std::rc::Rc<[u32]> {
        if let (n, Some(mask)) = self.k.s.loose_bvars(e) {
            debug_assert!(n <= 64);
            let mut v = Vec::with_capacity(mask.count_ones() as usize);
            let mut m = mask;
            while m != 0 {
                v.push(m.trailing_zeros());
                m &= m - 1;
            }
            return v.into();
        }
        if let Some(r) = self.st.loose_sets.get(&e) {
            return r.clone();
        }
        let r: std::rc::Rc<[u32]> = match self.k.s.node(e) {
            Node::BVar(i) => vec![i].into(),
            Node::App(f, a) => {
                let (x, y) = (self.loose_set(f), self.loose_set(a));
                union_shifted(&x, &y, 0)
            }
            Node::Lam(d, b) | Node::Pi(d, b) => {
                let (x, y) = (self.loose_set(d), self.loose_set(b));
                union_shifted(&x, &y, 1)
            }
            Node::Let(t, v, b) => {
                let (x, y, z) = (self.loose_set(t), self.loose_set(v), self.loose_set(b));
                union_shifted(&union_shifted(&x, &y, 0), &z, 1)
            }
            Node::Proj(_, _, x) => self.loose_set(x),
            _ => Vec::new().into(),
        };
        if !e.is_scratch() {
            self.st.loose_sets.insert(e, r.clone());
        }
        r
    }

    /// How a constant behaves as a head.
    fn const_kind(&mut self, n: NameId, ls: LevelsId) -> KResult<Kind> {
        let Some(c) = self.k.env.get(n) else {
            return Err(reject(format!(
                "unknown constant '{}'",
                self.k.s.names.display(n)
            )));
        };
        let kind = match &c.kind {
            CKind::Defn { .. } | CKind::Thm { .. } => {
                if self.k.s.levels.get_list(ls).len() == c.lparams.len() {
                    Kind::Delta
                } else {
                    Kind::Opaque
                }
            }
            CKind::Ctor(_) => Kind::Ctor,
            CKind::Rec(_) => Kind::Rec,
            CKind::Quot(_) => Kind::Quot,
            CKind::Axiom { .. } | CKind::Opaque { .. } | CKind::Induct(_) => Kind::Opaque,
        };
        if !self.k.env.is_settled(n) {
            self.st.scratch_seen = true;
            self.st.counters.scratch_unsettled += 1;
        }
        Ok(kind)
    }

    /// The value of a constant (with no arguments).
    pub(crate) fn const_value(&mut self, n: NameId, ls: LevelsId) -> KResult<V> {
        let h = Head::Const {
            name: n,
            levels: ls,
        };
        if let Some(v) = self.st.arena.find_head(&h) {
            return Ok(v);
        }
        let kind = self.const_kind(n, ls)?;
        Ok(self.st.arena.head_value(kind, h))
    }

    pub(crate) fn const0(&mut self, n: NameId) -> KResult<V> {
        self.const_value(n, LevelsId::EMPTY)
    }

    /// The environment constant `c`'s own terms are read under at the universe levels `ls`: the
    /// base of the substitution of `ls` for its parameters. A constant that is only provisionally
    /// declared may be withdrawn with its declaration, so the session must end with it.
    pub(crate) fn instance_base(&mut self, c: NameId, ls: LevelsId) -> KResult<EnvId> {
        let Some(k) = self.k.env.get(c) else {
            return Err(reject(format!(
                "unknown constant '{}'",
                self.k.s.names.display(c)
            )));
        };
        let ps = k.lparams.clone();
        if !self.k.env.is_settled(c) {
            self.st.scratch_seen = true;
            self.st.counters.scratch_instance += 1;
        }
        let s = self.k.s.levels.substitution(&ps, ls);
        Ok(self.st.arena.lsub_base(s))
    }

    /// The type of constant `n` at universe levels `ls`.
    pub fn const_type(&mut self, n: NameId, ls: LevelsId) -> KResult<V> {
        if let Some(v) = self.st.const_type.get(&(n.0, ls.0)) {
            return Ok(V(v));
        }
        let base = self.instance_base(n, ls)?;
        let ty = self.k.env.get(n).expect("a known constant").ty;
        let v = self.eval(base, ty)?;
        self.st.const_type.insert((n.0, ls.0), v.0);
        Ok(v)
    }

    /// Level `l` of a term read in `env`.
    #[inline]
    pub(crate) fn level_in(&mut self, env: EnvId, l: LevelId) -> LevelId {
        let s = self.st.arena.lsub_of(env);
        if s == SubstId::IDENTITY {
            l
        } else {
            self.k.s.levels.subst(l, s)
        }
    }

    /// The universe levels `ls` of a term read in `env`.
    #[inline]
    pub(crate) fn levels_in(&mut self, env: EnvId, ls: LevelsId) -> LevelsId {
        let s = self.st.arena.lsub_of(env);
        if s == SubstId::IDENTITY {
            ls
        } else {
            self.k.s.levels.subst_list(ls, s)
        }
    }

    /// Evaluates `e` in `env`. A variable is read in place; a value remembered for anything
    /// else — an atom's under its substitution, a compound's in an environment
    /// [`Sem::known_env`] prunes — is found in the caller's frame. Computing one, pruning an
    /// environment first, or building a closure is a call, and a call that only finds where to
    /// compute returns before the computation recurses.
    #[cfg_attr(debug_assertions, inline)]
    #[cfg_attr(not(debug_assertions), inline(always))]
    pub fn eval(&mut self, env: EnvId, e: ExprId) -> KResult<V> {
        let node = self.k.s.node(e);
        match node {
            Node::BVar(i) => match self.st.arena.lookup(env, i) {
                Some(v) => Ok(v),
                None => Err(loose_bound_variable()),
            },
            Node::Sort(_) | Node::Const(..) => {
                let s = self.atom_subst(env, e);
                match self.st.atom_memo(e, s) {
                    Some(v) => Ok(v),
                    None => self.eval_atom(e, s),
                }
            }
            Node::Lam(..) | Node::NatLit(_) | Node::StrLit(_) | Node::FVar(_) => {
                self.eval_leaf(env, e, node)
            }
            _ => {
                let known = if e.is_scratch() {
                    None
                } else {
                    self.known_env(env, self.k.s.scope_of(e))
                };
                let (env, found) = match known {
                    Some(pruned) => (pruned, self.remembered_value(pruned, e)),
                    None => self.pruned_value(env, e),
                };
                match found {
                    Some(v) => Ok(v),
                    None => self.eval_miss(env, e),
                }
            }
        }
    }

    /// A lambda's closure, a literal, or a free variable's refusal.
    #[inline(never)]
    fn eval_leaf(&mut self, env: EnvId, e: ExprId, node: Node) -> KResult<V> {
        match node {
            Node::Lam(dom, body) => {
                self.note_expr(e);
                let env = self.env_for(env, e);
                Ok(self.st.arena.lam(dom, env, body))
            }
            Node::NatLit(n) => Ok(self.st.arena.nat(n)),
            Node::StrLit(s) => Ok(self.st.arena.str(s)),
            Node::FVar(_) => Err(reject("the value engine met a free variable")),
            _ => unreachable!("only lambdas, literals and free variables are leaves"),
        }
    }

    /// The environment the compound `e` is evaluated in, pruned from `env`, and the value
    /// remembered for it there: for a term [`Sem::eval`] could not place with no call — one of
    /// the scratch region (which the session must not outlive) or one whose environment must be
    /// pruned first.
    #[inline(never)]
    fn pruned_value(&mut self, env: EnvId, e: ExprId) -> (EnvId, Option<V>) {
        self.note_expr(e);
        let env = self.env_for(env, e);
        (env, self.remembered_value(env, e))
    }

    /// A sort or constant read under the substitution `s`, remembered for the next read.
    #[inline(never)]
    fn eval_atom(&mut self, e: ExprId, s: SubstId) -> KResult<V> {
        let v = match self.k.s.node(e) {
            Node::Sort(l) => {
                let l = self.k.s.levels.subst(l, s);
                self.st.arena.sort(l)
            }
            Node::Const(n, ls) => {
                let ls = self.k.s.levels.subst_list(ls, s);
                self.const_value(n, ls)?
            }
            _ => unreachable!("only sorts and constants are atoms"),
        };
        self.st.note_atom(e, s, v);
        Ok(v)
    }

    /// The value remembered for the compound `e` in its pruned `env`.
    #[inline]
    fn remembered_value(&self, env: EnvId, e: ExprId) -> Option<V> {
        self.st.eval_cache.get(&(env.0, e.0)).map(V)
    }

    /// The value of the compound `e` in its pruned environment, computed and remembered.
    #[inline(never)]
    fn eval_miss(&mut self, env: EnvId, e: ExprId) -> KResult<V> {
        self.enter()?;
        let r = self.eval_node(env, e);
        self.leave();
        let v = r?;
        self.st.eval_cache.insert((env.0, e.0), v.0);
        Ok(v)
    }

    /// The substitution a sort or constant `e` is read under in `env`: the identity unless it
    /// mentions a universe parameter.
    #[inline]
    fn atom_subst(&self, env: EnvId, e: ExprId) -> SubstId {
        let s = self.st.arena.lsub_of(env);
        if s != SubstId::IDENTITY && self.k.s.has_lparam(e) {
            s
        } else {
            SubstId::IDENTITY
        }
    }

    /// A term of the current declaration's scratch region may not key anything that outlives it.
    #[inline]
    pub(crate) fn note_expr(&mut self, e: ExprId) {
        if e.is_scratch() {
            self.st.scratch_seen = true;
            self.st.counters.scratch_expr += 1;
        }
    }

    fn eval_node(&mut self, env: EnvId, e: ExprId) -> KResult<V> {
        match self.k.s.node(e) {
            Node::App(..) => {
                let mut short = [ExprId(0); SHORT_SPINE];
                match self.k.s.short_app_args(e, &mut short) {
                    Some((f, n)) => self.eval_app(env, f, &short[..n]),
                    None => {
                        let (f, args) = self.k.s.app_args(e);
                        self.eval_app(env, f, &args)
                    }
                }
            }
            Node::Pi(dom, body) => {
                let d = self.eval(env, dom)?;
                Ok(self.st.arena.pi(d, env, body))
            }
            Node::Let(_, val, body) => {
                let v = self.eval(env, val)?;
                let env2 = self.st.arena.extend(env, v);
                self.eval(env2, body)
            }
            Node::Proj(s, idx, x) => {
                let xv = self.eval(env, x)?;
                self.proj(s, idx, xv)
            }
            _ => unreachable!("atoms are evaluated directly"),
        }
    }

    /// The application of `f` to `args`, each read in `env`: β by extending a closure's
    /// environment with every argument its nested lambdas take at once.
    #[inline]
    fn eval_app(&mut self, env: EnvId, f: ExprId, args: &[ExprId]) -> KResult<V> {
        let mut fv = self.eval(env, f)?;
        let mut i = 0;
        while i < args.len() {
            let fval = self.st.arena.get(fv);
            if let Val::Lam {
                env: cenv, body, ..
            } = fval
            {
                let a = self.eval(env, args[i])?;
                let mut benv = self.st.arena.extend(cenv, a);
                let mut b = body;
                i += 1;
                while i < args.len() {
                    let Node::Lam(_, inner) = self.k.s.node(b) else {
                        break;
                    };
                    let a = self.eval(env, args[i])?;
                    benv = self.st.arena.extend(benv, a);
                    b = inner;
                    i += 1;
                }
                fv = self.eval(benv, b)?;
            } else {
                let a = self.eval(env, args[i])?;
                if !matches!(fval, Val::Neu { .. }) {
                    return Err(not_a_function());
                }
                let n = args.len() - i;
                if n <= SHORT_SPINE {
                    let mut rest = [DEAD; SHORT_SPINE];
                    rest[0] = a;
                    for (r, &arg) in rest[1..n].iter_mut().zip(&args[i + 1..]) {
                        *r = self.eval(env, arg)?;
                    }
                    return Ok(self.st.arena.neutral_apply(fv, &rest[..n]));
                }
                let mut rest = Vec::with_capacity(n);
                rest.push(a);
                for &arg in &args[i + 1..] {
                    rest.push(self.eval(env, arg)?);
                }
                return Ok(self.st.arena.neutral_apply(fv, &rest));
            }
        }
        Ok(fv)
    }

    /// `f a`.
    pub fn apply(&mut self, f: V, a: V) -> KResult<V> {
        match self.st.arena.get(f) {
            Val::Lam { env, body, .. } => {
                let env2 = self.st.arena.extend(env, a);
                self.eval(env2, body)
            }
            Val::Neu { .. } => Ok(self.st.arena.neutral_app(f, a)),
            _ => Err(not_a_function()),
        }
    }

    pub(crate) fn apply_all(&mut self, mut f: V, args: &[V]) -> KResult<V> {
        let mut i = 0;
        while i < args.len() {
            if let Val::Lam { env, body, .. } = self.st.arena.get(f) {
                let mut benv = self.st.arena.extend(env, args[i]);
                let mut b = body;
                i += 1;
                while i < args.len() {
                    let Node::Lam(_, inner) = self.k.s.node(b) else {
                        break;
                    };
                    benv = self.st.arena.extend(benv, args[i]);
                    b = inner;
                    i += 1;
                }
                f = self.eval(benv, b)?;
            } else if matches!(self.st.arena.get(f), Val::Neu { .. }) {
                return Ok(self.st.arena.neutral_apply(f, &args[i..]));
            } else {
                return Err(not_a_function());
            }
        }
        Ok(f)
    }

    /// The domain of a lambda value.
    pub(crate) fn lam_domain(&mut self, v: V) -> KResult<V> {
        match self.st.arena.get(v) {
            Val::Lam { dom, env, .. } => {
                if let Some(d) = self.st.arena.dom_memo(v) {
                    return Ok(d);
                }
                let d = self.eval(env, dom)?;
                self.st.arena.set_dom_memo(v, d);
                Ok(d)
            }
            Val::Pi { dom, .. } | Val::PiLam { dom, .. } => Ok(dom),
            _ => Err(reject("expected a function")),
        }
    }

    /// The body of a closure applied to `x`: a lambda's body, a Π's codomain.
    pub fn instantiate(&mut self, clo: V, x: V) -> KResult<V> {
        match self.st.arena.get(clo) {
            Val::Lam { env, body, .. } | Val::Pi { env, body, .. } => {
                let env2 = self.st.arena.extend(env, x);
                self.eval(env2, body)
            }
            Val::PiLam {
                dom,
                env,
                ctx,
                body,
            } => {
                let env2 = self.st.arena.extend(env, x);
                let ctx2 = self.st.arena.ctx_extend(ctx, dom);
                self.infer(env2, ctx2, body, false)
            }
            _ => Err(reject("expected a function or function type")),
        }
    }
}

#[cold]
fn loose_bound_variable() -> crate::dag::error::KernelFailure {
    reject("evaluation met a loose bound variable")
}

#[cold]
fn not_a_function() -> crate::dag::error::KernelFailure {
    reject("application of a value that is not a function")
}

/// `a ∪ { i - shift | i ∈ b, i ≥ shift }`, both sets ascending, in one walk. A union that adds
/// nothing to `a` is `a` itself, its allocation shared: down a deep nest of binders the set of a
/// body is usually its domain's plus nothing, or its body's.
fn union_shifted(
    a: &std::rc::Rc<[u32]>,
    whole: &std::rc::Rc<[u32]>,
    shift: u32,
) -> std::rc::Rc<[u32]> {
    let b = &whole[whole.partition_point(|&i| i < shift)..];
    if b.is_empty() {
        return a.clone();
    }
    if a.is_empty() && shift == 0 {
        return whole.clone();
    }
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        let (x, y) = (a[i], b[j] - shift);
        match x.cmp(&y) {
            std::cmp::Ordering::Less => {
                out.push(x);
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                out.push(y);
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                out.push(x);
                i += 1;
                j += 1;
            }
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend(b[j..].iter().map(|&y| y - shift));
    if out.len() == a.len() {
        return a.clone();
    }
    out.into()
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_kernel/dag/sem/eval/union_controls.rs"]
mod union_controls;
