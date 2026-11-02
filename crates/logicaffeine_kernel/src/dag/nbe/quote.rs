//! Read-back: a value as a term again (weak-head normal at every level), for diagnostics and for
//! comparing the two engines' reductions in tests.

use super::value::{Elim, Head, VId, Val};
use super::Nbe;
use crate::dag::error::KResult;
use crate::dag::expr::ExprId;

impl Nbe<'_> {
    /// Reads `v` back as a term. Variables bound during read-back become de Bruijn indices;
    /// `base` is the binder depth read-back starts at (a local bound outside it is rejected).
    pub fn quote(&mut self, v: VId) -> KResult<ExprId> {
        let base = self.st.level;
        self.quote_at(v, base, 0)
    }

    fn quote_at(&mut self, v: VId, base: u32, depth: u32) -> KResult<ExprId> {
        self.enter()?;
        let r = self.quote_inner(v, base, depth);
        self.leave();
        r
    }

    fn quote_inner(&mut self, v: VId, base: u32, depth: u32) -> KResult<ExprId> {
        let v = self.whnf(v)?;
        Ok(match self.st.arena.get(v) {
            Val::Sort(l) => self.k.s.sort(l),
            Val::Nat(n) => {
                let x = self.k.s.nat(n).clone();
                self.k.s.nat_lit(x)
            }
            Val::Str(s) => {
                let st = self.k.s.string(s);
                self.k.s.str_lit(&st)
            }
            Val::Lam { dom, env, body } => {
                let d = self.eval(env, dom)?;
                let dq = self.quote_at(d, base, depth)?;
                let bq = self.under(d, |n, x| {
                    let b = n.instantiate_closure(env, body, x)?;
                    n.quote_at(b, base, depth + 1)
                })?;
                self.k.s.lam(dq, bq)
            }
            Val::Pi { dom, .. } | Val::PiOfLam { dom, .. } => {
                let dq = self.quote_at(dom, base, depth)?;
                let cq = self.under(dom, |n, x| {
                    let c = n.pi_codomain(v, x)?;
                    n.quote_at(c, base, depth + 1)
                })?;
                self.k.s.pi(dq, cq)
            }
            Val::Neutral { head, spine } => {
                let mut e = match head {
                    Head::Local(i) => {
                        let level = self.st.arena.locals[i as usize].level;
                        if level < base || level >= base + depth {
                            return Err(crate::dag::error::KernelFailure::reject(
                                "read-back met a foreign local",
                            ));
                        }
                        self.k.s.bvar(depth - 1 - (level - base))
                    }
                    Head::FVar(i) => self.k.s.fvar(i),
                    Head::Const(_, n, ls) => self.k.s.cnst(n, ls),
                };
                for el in self.st.arena.spine_elims(spine) {
                    e = match el {
                        Elim::App(a) => {
                            let aq = self.quote_at(a, base, depth)?;
                            self.k.s.app(e, aq)
                        }
                        Elim::Proj(s, i) => self.k.s.proj(s, i, e),
                    };
                }
                e
            }
            Val::Glued {
                name,
                levels,
                spine,
            } => {
                let mut e = self.k.s.cnst(name, levels);
                for el in self.st.arena.spine_elims(spine) {
                    e = match el {
                        Elim::App(a) => {
                            let aq = self.quote_at(a, base, depth)?;
                            self.k.s.app(e, aq)
                        }
                        Elim::Proj(s, i) => self.k.s.proj(s, i, e),
                    };
                }
                e
            }
            Val::Thunk { .. } => unreachable!("whnf forces thunks"),
        })
    }
}
