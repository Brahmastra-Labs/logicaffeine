//! The types of values, for the parts of conversion and reduction that look at types — proof
//! irrelevance, η, struct-η, unit-like types, K — and for projections. Values reaching here are
//! well-typed, so this is inference without checking: a head's type followed along the spine.

use super::value::{CtxId, Head, Sp, Val, V};
use super::{reject, Sem};
use crate::dag::error::KResult;
use crate::dag::level::{LevelId, LevelsId};
use crate::dag::name::NameId;
use crate::dag::relevance::{Signature, MAX_TRACKED};

impl Sem<'_> {
    /// The type of `v`.
    #[inline]
    pub fn type_of(&mut self, v: V) -> KResult<V> {
        match self.st.arena.type_memo(v) {
            Some(t) => Ok(t),
            None => self.type_of_miss(v),
        }
    }

    /// The type of `v`, inferred and remembered on it.
    #[inline(never)]
    fn type_of_miss(&mut self, v: V) -> KResult<V> {
        self.enter()?;
        let r = self.type_of_uncached(v);
        self.leave();
        let r = r?;
        self.st.arena.set_type_memo(v, r);
        Ok(r)
    }

    fn type_of_uncached(&mut self, v: V) -> KResult<V> {
        match self.st.arena.get(v) {
            Val::Sort(l) => {
                let sl = self.k.s.levels.succ(l);
                Ok(self.st.arena.sort(sl))
            }
            Val::Nat(_) => self.const0(self.k.wk.nat),
            Val::Str(_) => self.const0(self.k.wk.string),
            Val::Neu { head, spine, .. } => {
                let mut ty = match self.st.arena.head(head) {
                    Head::Local { ty, .. } => ty,
                    Head::Const { name, levels } => self.const_type(name, levels)?,
                    Head::Proj { s, idx, x } => {
                        let xt = self.type_of(x)?;
                        self.proj_type(s, idx, x, xt, false)?
                    }
                };
                let n = self.st.arena.spine_len(spine);
                for i in 0..n {
                    let a = self.st.arena.spine_arg(spine, i);
                    let w = self.whnf(ty)?;
                    ty = self.instantiate(w, a)?;
                }
                Ok(ty)
            }
            Val::Lam { env, body, .. } => {
                let d = self.lam_domain(v)?;
                Ok(self.st.arena.pi_lam(d, env, CtxId::EMPTY, body))
            }
            Val::Pi { dom, .. } | Val::PiLam { dom, .. } => {
                let ld = self.sort_level(dom)?;
                let lb = self.under(dom, |s, x| {
                    let cod = s.instantiate(v, x)?;
                    s.sort_level(cod)
                })?;
                let l = self.k.s.levels.imax(ld, lb);
                Ok(self.st.arena.sort(l))
            }
        }
    }

    /// The universe level of a type.
    pub(crate) fn sort_level(&mut self, ty: V) -> KResult<LevelId> {
        let tt = self.type_of(ty)?;
        match self.sort_of(tt)? {
            Some(l) => Ok(l),
            None => Err(reject(format!("type expected, found {}", self.show(ty)))),
        }
    }

    /// Whether the type `ty` lives in `Prop`.
    pub fn is_prop(&mut self, ty: V) -> KResult<bool> {
        let l = self.sort_level(ty)?;
        Ok(self.k.s.levels.is_always_zero(l))
    }

    /// Whether `v` is a proof (its type is a proposition): decided by its shape or its head's
    /// [`Signature`] where they decide it, by inferring its type otherwise, and remembered on the
    /// value.
    #[inline]
    pub(crate) fn is_proof(&mut self, v: V) -> KResult<bool> {
        match self.st.arena.proof_memo(v) {
            Some(p) => Ok(p),
            None => self.decide_proof(v),
        }
    }

    /// Whether `v` is a proof, decided and remembered on it.
    #[inline(never)]
    fn decide_proof(&mut self, v: V) -> KResult<bool> {
        let p = match self.static_proof(v)? {
            Some(p) => p,
            None => {
                self.st.counters.proof_probes += 1;
                let t = self.type_of(v)?;
                self.is_prop(t)?
            }
        };
        self.st.arena.set_proof_memo(v, p);
        Ok(p)
    }

    fn static_proof(&mut self, v: V) -> KResult<Option<bool>> {
        Ok(match self.st.arena.get(v) {
            Val::Sort(_) | Val::Pi { .. } | Val::PiLam { .. } | Val::Nat(_) | Val::Str(_) => {
                Some(false)
            }
            Val::Lam { .. } => None,
            Val::Neu { head, spine, .. } => match self.st.arena.head(head) {
                Head::Const { name, levels } => {
                    let k = self.st.arena.spine_len(spine) as usize;
                    self.signature(name, levels)?.and_then(|s| s.is_proof(k))
                }
                Head::Local { ty, .. } if spine == Sp::NIL => self.static_proposition(ty)?,
                _ => None,
            },
        })
    }

    /// Whether the type `ty` is a proposition, where its shape or its head's signature decides it.
    fn static_proposition(&mut self, ty: V) -> KResult<Option<bool>> {
        Ok(match self.st.arena.get(ty) {
            Val::Sort(_) => Some(false),
            Val::Neu { head, spine, .. } => match self.st.arena.head(head) {
                Head::Const { name, levels } => {
                    let k = self.st.arena.spine_len(spine) as usize;
                    self.signature(name, levels)?
                        .and_then(|s| s.is_proposition(k))
                }
                _ => None,
            },
            _ => None,
        })
    }

    /// The [`Signature`] of `c.{ls}`, computed once per settled constant and instantiation and
    /// shared by every later declaration; `None` for a constant only provisionally in the
    /// environment, or one given the wrong number of universes, whose applications' types must be
    /// inferred.
    pub fn signature(&mut self, c: NameId, ls: LevelsId) -> KResult<Option<Signature>> {
        if let Some(s) = self.k.known_signature(c, ls) {
            return Ok(Some(s));
        }
        if !self.k.env.is_settled(c) {
            return Ok(None);
        }
        match self.k.env.get(c) {
            Some(k) if k.lparams.len() == self.k.s.levels.get_list(ls).len() => {}
            _ => return Ok(None),
        }
        if !self.k.relevance.computing.insert((c, ls)) {
            return Ok(None);
        }
        let r = self
            .const_type(c, ls)
            .and_then(|ty| self.telescope_signature(ty));
        self.k.relevance.computing.remove(&(c, ls));
        let s = r?;
        self.k.note_signature(c, ls, s);
        Ok(Some(s))
    }

    /// The signature of the type `ty`: its telescope walked under fresh variables, each binder
    /// type's universe tested for `Prop`, and the result's universe deciding every partial
    /// application (see [`Signature`]).
    pub(crate) fn telescope_signature(&mut self, ty: V) -> KResult<Signature> {
        let mut proof_args = 0u64;
        let mut bound = 0usize;
        let r = (|| -> KResult<Signature> {
            let mut cur = ty;
            let result = loop {
                let w = self.whnf(cur)?;
                let dom = match self.st.arena.get(w) {
                    Val::Pi { dom, .. } | Val::PiLam { dom, .. } => dom,
                    _ => break w,
                };
                if bound == MAX_TRACKED {
                    return Ok(Signature::new(proof_args, 0, 0, None));
                }
                let l = self.sort_level(dom)?;
                if self.k.s.levels.is_always_zero(l) {
                    proof_args |= 1 << bound;
                }
                let x = self.bind(dom);
                bound += 1;
                cur = self.instantiate(w, x)?;
            };
            let n = bound;
            if n >= MAX_TRACKED {
                return Ok(Signature::new(proof_args, 0, 0, None));
            }
            let proposition = match self.st.arena.get(result) {
                Val::Sort(l) => Some(self.k.s.levels.is_always_zero(l)),
                _ => None,
            };
            let known = u64::MAX >> (MAX_TRACKED - 1 - n);
            let level = self.sort_level(result)?;
            let proofs = if self.k.s.levels.is_always_zero(level) {
                known
            } else {
                0
            };
            Ok(Signature::new(
                proof_args,
                proofs,
                known,
                Some((n as u32, proposition)),
            ))
        })();
        for _ in 0..bound {
            self.unbind();
        }
        r
    }

    /// The type of field `idx` of the structure value `x : xt` (Lean's `inferProj`). With `check`,
    /// the projection's side conditions are enforced: a structure in `Prop` may only be projected
    /// to its proof fields (and past a data field only if nothing later depends on it).
    pub(crate) fn proj_type(
        &mut self,
        sname: NameId,
        idx: u32,
        x: V,
        xt: V,
        check: bool,
    ) -> KResult<V> {
        let fail = |s: &mut Sem| {
            reject(format!(
                "invalid projection .{} of '{}'",
                idx + 1,
                s.k.s.names.display(sname)
            ))
        };
        let ty = self.whnf(xt)?;
        let Some((iname, ils, _, tsp)) = self.const_app(ty) else {
            return Err(fail(self));
        };
        if iname != sname {
            return Err(fail(self));
        }
        let (ctor0, nctors, num_params, num_indices) = {
            let Some(ind) = self.k.env.inductive(iname) else {
                return Err(fail(self));
            };
            (
                ind.ctors.first().copied(),
                ind.ctors.len(),
                ind.num_params,
                ind.num_indices,
            )
        };
        let targs = self.st.arena.args(tsp);
        if nctors != 1 || targs.len() != (num_params + num_indices) as usize {
            return Err(fail(self));
        }
        let ctor0 = ctor0.expect("a single-constructor inductive has its constructor");
        let mut r = self.const_type(ctor0, ils)?;
        for arg in targs.iter().take(num_params as usize) {
            let w = self.whnf(r)?;
            if !matches!(self.st.arena.get(w), Val::Pi { .. } | Val::PiLam { .. }) {
                return Err(fail(self));
            }
            r = self.instantiate(w, *arg)?;
        }
        let maybe_prop = if check {
            let sl = self.sort_level(ty)?;
            !self.k.s.levels.is_never_zero(sl)
        } else {
            false
        };
        for i in 0..idx {
            let w = self.whnf(r)?;
            let (dom, body) = match self.st.arena.get(w) {
                Val::Pi { dom, body, .. } | Val::PiLam { dom, body, .. } => (dom, body),
                _ => return Err(fail(self)),
            };
            let depends = self.k.s.loose_bvar_mask(body).is_none_or(|m| m & 1 == 1);
            if maybe_prop && depends && !self.is_prop(dom)? {
                return Err(fail(self));
            }
            let p = self.proj(iname, i, x)?;
            r = self.instantiate(w, p)?;
        }
        let w = self.whnf(r)?;
        let dom = match self.st.arena.get(w) {
            Val::Pi { dom, .. } | Val::PiLam { dom, .. } => dom,
            _ => return Err(fail(self)),
        };
        if maybe_prop && !self.is_prop(dom)? {
            return Err(fail(self));
        }
        Ok(dom)
    }
}
