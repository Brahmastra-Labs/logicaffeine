//! Types of values, for the parts of conversion that look at types: proof irrelevance, unit-like
//! types, struct-η and K. Values reaching conversion are well-typed, so this is inference only —
//! it reads a head's type and follows the spine, never re-checking.

use super::value::{Elim, EnvId, Head, Sp, VId, Val};
use super::Nbe;
use crate::dag::error::{KResult, KernelFailure};
use crate::dag::level::{LevelId, LevelsId};
use crate::dag::name::NameId;

impl Nbe<'_> {
    /// A constant's type at the given universe arguments.
    pub(crate) fn const_type(&mut self, n: NameId, ls: LevelsId) -> KResult<VId> {
        if let Some(v) = self.st.const_ty.get(&(n, ls)) {
            return Ok(*v);
        }
        let c = self
            .k
            .env
            .get(n)
            .cloned()
            .ok_or_else(|| KernelFailure::reject("unknown constant"))?;
        let env = if c.lparams.is_empty() {
            EnvId::EMPTY
        } else {
            self.st.arena.with_lsub(c.lparams.clone(), ls)
        };
        let v = self.eval(env, c.ty)?;
        self.st.const_ty.insert((n, ls), v);
        Ok(v)
    }

    fn head_type(&mut self, h: Head) -> KResult<VId> {
        match h {
            Head::Local(i) => Ok(self.st.arena.locals[i as usize].ty),
            Head::FVar(i) => {
                let ty = self.k.lctx[i as usize].ty;
                self.eval(EnvId::EMPTY, ty)
            }
            Head::Const(_, n, ls) => self.const_type(n, ls),
        }
    }

    /// The type of `v`.
    pub fn type_of(&mut self, v: VId) -> KResult<VId> {
        let v = self.force(v)?;
        if let Some(t) = self.st.type_cache.get(&v) {
            return Ok(*t);
        }
        self.enter()?;
        self.trace("type_of", v);
        let r = self.type_of_uncached(v);
        self.leave();
        let r = r?;
        self.st.type_cache.insert(v, r);
        Ok(r)
    }

    fn type_of_uncached(&mut self, v: VId) -> KResult<VId> {
        match self.st.arena.get(v) {
            Val::Sort(l) => {
                let sl = self.k.s.levels.succ(l);
                Ok(self.st.arena.alloc(Val::Sort(sl)))
            }
            Val::Nat(_) => self.const0(self.k.wk.nat),
            Val::Str(_) => self.const0(self.k.wk.string),
            Val::Neutral { head, spine } => {
                let ht = self.head_type(head)?;
                let base = self.st.arena.alloc(Val::Neutral {
                    head,
                    spine: Sp::NIL,
                });
                self.spine_type(base, ht, spine)
            }
            Val::Glued {
                name,
                levels,
                spine,
            } => {
                let ht = self.const_type(name, levels)?;
                let base = self.const_value(name, levels)?;
                self.spine_type(base, ht, spine)
            }
            Val::Lam { dom, env, body } => {
                let d = self.eval(env, dom)?;
                Ok(self.st.arena.alloc(Val::PiOfLam { dom: d, env, body }))
            }
            Val::Pi { dom, .. } | Val::PiOfLam { dom, .. } => {
                let ld = self.sort_level(dom)?;
                let lb = self.under(dom, |n, x| {
                    let cod = n.pi_codomain(v, x)?;
                    n.sort_level(cod)
                })?;
                let l = self.k.s.levels.imax(ld, lb);
                Ok(self.st.arena.alloc(Val::Sort(l)))
            }
            Val::Thunk { .. } => unreachable!("forced above"),
        }
    }

    /// The codomain of a Π-value at argument `a`.
    pub(crate) fn pi_codomain(&mut self, pi: VId, a: VId) -> KResult<VId> {
        match self.st.arena.get(pi) {
            Val::Pi { env, body, .. } => self.instantiate_closure(env, body, a),
            Val::PiOfLam { env, body, .. } => {
                let b = self.instantiate_closure(env, body, a)?;
                self.type_of(b)
            }
            _ => Err(KernelFailure::reject("expected a function type")),
        }
    }

    pub(crate) fn pi_domain(&mut self, pi: VId) -> Option<VId> {
        match self.st.arena.get(pi) {
            Val::Pi { dom, .. } | Val::PiOfLam { dom, .. } => Some(dom),
            _ => None,
        }
    }

    fn spine_type(&mut self, base: VId, mut ty: VId, spine: Sp) -> KResult<VId> {
        let mut prefix = base;
        for e in self.st.arena.spine_elims(spine) {
            match e {
                Elim::App(a) => {
                    let t = self.whnf(ty)?;
                    ty = self.pi_codomain(t, a)?;
                    prefix = self.apply(prefix, a)?;
                }
                Elim::Proj(s, i) => {
                    ty = self.proj_type(prefix, ty, s, i)?;
                    prefix = self.proj(s, i, prefix)?;
                }
            }
        }
        Ok(ty)
    }

    /// The type of field `idx` of structure value `x : struct_ty`.
    fn proj_type(&mut self, x: VId, struct_ty: VId, s: NameId, idx: u32) -> KResult<VId> {
        let fail = || KernelFailure::reject("ill-typed projection in conversion");
        let t = self.whnf(struct_ty)?;
        let (i, ls, tspine) = self.const_app(t).ok_or_else(fail)?;
        if i != s {
            return Err(fail());
        }
        let ind = self.k.env.inductive(i).cloned().ok_or_else(fail)?;
        let ctor = *ind.ctors.first().ok_or_else(fail)?;
        let targs = self.st.arena.spine_args(tspine).ok_or_else(fail)?;
        let mut r = self.const_type(ctor, ls)?;
        for p in targs.iter().take(ind.num_params as usize) {
            let w = self.whnf(r)?;
            r = self.pi_codomain(w, *p)?;
        }
        for f in 0..idx {
            let w = self.whnf(r)?;
            let pf = self.proj(s, f, x)?;
            r = self.pi_codomain(w, pf)?;
        }
        let w = self.whnf(r)?;
        self.pi_domain(w).ok_or_else(fail)
    }

    /// The universe level of a type.
    pub(crate) fn sort_level(&mut self, ty: VId) -> KResult<LevelId> {
        let tt = self.type_of(ty)?;
        let s = self.whnf(tt)?;
        match self.st.arena.get(s) {
            Val::Sort(l) => Ok(l),
            _ => Err(KernelFailure::reject("expected a type in conversion")),
        }
    }

    /// Whether the type `ty` is a proposition (lives in `Prop`). Like the classic engine, this
    /// reads the sort of `ty` by inference and never reduces `ty` itself; a Π already in hand is
    /// decided by its codomain, which is equivalent (`imax u 0 = 0`) and cheaper.
    pub(crate) fn is_prop(&mut self, ty: VId) -> KResult<bool> {
        let w = self.force(ty)?;
        match self.st.arena.get(w) {
            Val::Pi { dom, .. } | Val::PiOfLam { dom, .. } => self.under(dom, |n, x| {
                let cod = n.pi_codomain(w, x)?;
                n.is_prop(cod)
            }),
            Val::Sort(_) => Ok(false),
            _ => {
                let l = self.sort_level(w)?;
                Ok(self.level_is_zero(l))
            }
        }
    }
}
