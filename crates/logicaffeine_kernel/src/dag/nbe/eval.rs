//! Evaluation: expressions to values, and application of values.

use super::value::{ConstKind, Elim, EnvId, Head, Sp, VId, Val};
use super::Nbe;
use crate::dag::env::Kind;
use crate::dag::error::{KResult, KernelFailure};
use crate::dag::expr::{ExprId, Node};
use crate::dag::level::{LevelId, LevelsId};
use crate::dag::name::NameId;

impl Nbe<'_> {
    fn lsub_of(&self, env: EnvId) -> u32 {
        self.st.arena.env(env).lsub
    }

    pub(crate) fn subst_level(&mut self, env: EnvId, l: LevelId) -> LevelId {
        let li = self.lsub_of(env);
        if li == 0 {
            return l;
        }
        let ls = self.st.arena.lsubs[li as usize].clone();
        let us = self.k.s.levels.get_list(ls.levels).to_vec();
        self.k.s.levels.instantiate(l, &ls.params, &us)
    }

    pub(crate) fn subst_levels(&mut self, env: EnvId, ls: LevelsId) -> LevelsId {
        if self.lsub_of(env) == 0 || ls == LevelsId::EMPTY {
            return ls;
        }
        let old = self.k.s.levels.get_list(ls).to_vec();
        let new: Vec<LevelId> = old.iter().map(|l| self.subst_level(env, *l)).collect();
        self.k.s.levels.list(&new)
    }

    /// The value of a constant as a head (before any argument).
    pub(crate) fn const_value(&mut self, n: NameId, ls: LevelsId) -> KResult<VId> {
        if let Some(v) = self.st.const_val.get(&(n, ls)) {
            return Ok(*v);
        }
        let c = self.k.env.get(n).ok_or_else(|| {
            KernelFailure::reject(format!("unknown constant '{}'", self.k.s.names.display(n)))
        })?;
        let val = match c.kind {
            Kind::Defn { .. } | Kind::Thm { .. } => Val::Glued {
                name: n,
                levels: ls,
                spine: Sp::NIL,
            },
            Kind::Ctor(_) => Val::Neutral {
                head: Head::Const(ConstKind::Ctor, n, ls),
                spine: Sp::NIL,
            },
            Kind::Rec(_) => Val::Neutral {
                head: Head::Const(ConstKind::Rec, n, ls),
                spine: Sp::NIL,
            },
            Kind::Quot(_) => Val::Neutral {
                head: Head::Const(ConstKind::Quot, n, ls),
                spine: Sp::NIL,
            },
            Kind::Axiom { .. } | Kind::Opaque { .. } | Kind::Induct(_) => Val::Neutral {
                head: Head::Const(ConstKind::Opaque, n, ls),
                spine: Sp::NIL,
            },
        };
        let v = self.st.arena.alloc(val);
        self.st.const_val.insert((n, ls), v);
        Ok(v)
    }

    /// `env` as `e` sees it (see [`super::value::Arena::scope`]).
    #[inline]
    pub(crate) fn env_for(&mut self, env: EnvId, e: ExprId) -> EnvId {
        let (n, mask) = self.k.s.loose_bvars(e);
        self.st.arena.scope(env, n, mask)
    }

    /// Evaluates `e` in `env`. Results are cached on `(env as e sees it, e)`.
    pub fn eval(&mut self, env: EnvId, e: ExprId) -> KResult<VId> {
        if let Node::BVar(i) = self.k.s.node(e) {
            return self
                .st
                .arena
                .lookup(env, i)
                .ok_or_else(|| KernelFailure::reject("evaluation met a loose bound variable"));
        }
        let env = self.env_for(env, e);
        if let Some(v) = self.st.eval_cache.get(&(env, e)) {
            return Ok(*v);
        }
        self.enter()?;
        if super::tracing_on() {
            let probe = self.st.arena.thunk(env, e);
            self.trace("eval", probe);
        }
        let r = self.eval_uncached(env, e);
        self.leave();
        let r = r?;
        self.st.eval_cache.insert((env, e), r);
        Ok(r)
    }

    fn eval_uncached(&mut self, env: EnvId, e: ExprId) -> KResult<VId> {
        match self.k.s.node(e) {
            Node::BVar(i) => self
                .st
                .arena
                .lookup(env, i)
                .ok_or_else(|| KernelFailure::reject("evaluation met a loose bound variable")),
            Node::FVar(i) => match self.k.lctx.get(i as usize).map(|d| d.val) {
                Some(Some(val)) => self.eval(EnvId::EMPTY, val),
                Some(None) => Ok(self.st.arena.alloc(Val::Neutral { head: Head::FVar(i), spine: Sp::NIL })),
                None => Err(KernelFailure::reject("evaluation met an unknown free variable")),
            },
            Node::Sort(l) => {
                let l = self.subst_level(env, l);
                Ok(self.st.arena.alloc(Val::Sort(l)))
            }
            Node::Const(n, ls) => {
                let ls = self.subst_levels(env, ls);
                self.const_value(n, ls)
            }
            Node::App(f, a) => {
                let fv = self.eval(env, f)?;
                let av = self.eval_arg(env, a)?;
                self.apply(fv, av)
            }
            Node::Lam(dom, body) => Ok(self.st.arena.alloc(Val::Lam { dom, env, body })),
            Node::Pi(dom, body) => {
                let d = self.eval_arg(env, dom)?;
                Ok(self.st.arena.alloc(Val::Pi { dom: d, env, body }))
            }
            Node::Let(_, val, body) => {
                let v = self.eval_arg(env, val)?;
                let env2 = self.st.arena.extend(env, v);
                self.eval(env2, body)
            }
            Node::NatLit(n) => Ok(self.st.arena.alloc(Val::Nat(n))),
            Node::StrLit(s) => Ok(self.st.arena.alloc(Val::Str(s))),
            Node::Proj(s, i, x) => {
                let xv = self.eval(env, x)?;
                self.proj_lazy(s, i, xv)
            }
            Node::Match(..) | Node::Fix(_) | Node::MutualFix(..) | Node::Lit(_) | Node::Seq(..) | Node::Nil => {
                Err(KernelFailure::reject("the closure engine evaluates Lean's theory; LOGOS's is checked by the substitution checker"))
            }
        }
    }

    /// An argument: variables and atoms directly, anything else as a thunk.
    pub(crate) fn eval_arg(&mut self, env: EnvId, e: ExprId) -> KResult<VId> {
        match self.k.s.node(e) {
            Node::BVar(_)
            | Node::Sort(_)
            | Node::Const(..)
            | Node::NatLit(_)
            | Node::StrLit(_)
            | Node::FVar(_) => self.eval(env, e),
            _ => {
                let env = self.env_for(env, e);
                if let Some(v) = self.st.eval_cache.get(&(env, e)) {
                    return Ok(*v);
                }
                Ok(self.st.arena.thunk(env, e))
            }
        }
    }

    /// Forces a thunk (at most once); any other value is returned as is.
    pub fn force(&mut self, mut v: VId) -> KResult<VId> {
        loop {
            match self.st.arena.get(v) {
                Val::Thunk { env, expr } => {
                    if let Some(r) = self.st.arena.memo(v) {
                        v = r;
                        continue;
                    }
                    let r = self.eval(env, expr)?;
                    self.st.arena.set_memo(v, r);
                    v = r;
                }
                _ => return Ok(v),
            }
        }
    }

    /// Applies a function value to an argument.
    pub fn apply(&mut self, f: VId, a: VId) -> KResult<VId> {
        let f = self.force(f)?;
        match self.st.arena.get(f) {
            Val::Lam { env, body, .. } => {
                let env2 = self.st.arena.extend(env, a);
                self.eval(env2, body)
            }
            Val::Neutral { head, spine } => Ok(self.st.arena.app_value(f, a, |ar| Val::Neutral {
                head,
                spine: ar.snoc(spine, Elim::App(a)),
            })),
            Val::Glued {
                name,
                levels,
                spine,
            } => Ok(self.st.arena.app_value(f, a, |ar| Val::Glued {
                name,
                levels,
                spine: ar.snoc(spine, Elim::App(a)),
            })),
            _ => Err(KernelFailure::reject("application of a non-function value")),
        }
    }

    pub(crate) fn apply_all(&mut self, mut f: VId, args: &[VId]) -> KResult<VId> {
        for a in args {
            f = self.apply(f, *a)?;
        }
        Ok(f)
    }

    /// Re-applies a spine's eliminations to `v`.
    pub(crate) fn apply_elims(&mut self, mut v: VId, elims: &[Elim]) -> KResult<VId> {
        for e in elims {
            v = match *e {
                Elim::App(a) => self.apply(v, a)?,
                Elim::Proj(s, i) => self.proj(s, i, v)?,
            };
        }
        Ok(v)
    }

    /// Runs `f` under a binder of type `ty`: with the local of that type at the current binder
    /// depth, one level deeper for the length of `f` (see [`super::value`]).
    pub(crate) fn under<T>(
        &mut self,
        ty: VId,
        f: impl FnOnce(&mut Self, VId) -> KResult<T>,
    ) -> KResult<T> {
        let x = self.st.arena.local(self.st.level, ty);
        self.st.level += 1;
        let r = f(self, x);
        self.st.level -= 1;
        r
    }

    /// Instantiates a closure `(env, body)` with `a` for its bound variable.
    pub(crate) fn instantiate_closure(&mut self, env: EnvId, body: ExprId, a: VId) -> KResult<VId> {
        let env2 = self.st.arena.extend(env, a);
        self.eval(env2, body)
    }

    /// The nullary-level value of a constant with no universe arguments.
    pub(crate) fn const0(&mut self, n: NameId) -> KResult<VId> {
        self.const_value(n, LevelsId::EMPTY)
    }
}
