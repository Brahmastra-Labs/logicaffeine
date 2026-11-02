//! Type inference and checking over values.
//!
//! A term is walked with an environment (the values of its bound variables) and a context (their
//! types), and its type is produced as a value. An application's type is its function type's
//! closure instantiated with the argument's value; crossing a binder binds a fresh variable; a
//! lambda's type is a closure whose codomain is inferred only if something asks for it. With
//! `check`, every rule of the substitution checker is enforced — universe parameters, safety,
//! argument types, `let` values, the restrictions on projecting out of a proposition.

use super::value::{CtxId, EnvId, Val, DEAD, V};
use super::{reject, Sem};
use crate::dag::env::Safety;
use crate::dag::error::{KResult, KernelFailure};
use crate::dag::expr::{ExprId, Node};
use crate::dag::level::{LevelId, LevelsId};
use crate::dag::name::NameId;

impl Sem<'_> {
    /// The type of `e`, whose bound variables have the values `env` and the types `ctx` (an
    /// empty context means "the types of the values").
    pub fn infer(&mut self, env: EnvId, ctx: CtxId, e: ExprId, check: bool) -> KResult<V> {
        self.note_expr(e);
        let env = self.env_for(env, e);
        let key = (env.0, e.0);
        let lparam_free = !self.k.s.has_lparam(e);
        let safe = self.k.safety == Safety::Safe;
        let scoped = (self.st.scope, env.0, e.0);
        let cached = if !check {
            self.st.infer_cache.get(&key)
        } else {
            let lasting = if lparam_free {
                self.st.safe_checked.get(&key)
            } else if safe {
                self.st.scoped_checked.get(&scoped)
            } else {
                None
            };
            lasting.or_else(|| self.st.check_cache.get(&key))
        };
        if let Some(t) = cached {
            return Ok(V(t));
        }
        self.enter()?;
        let r = self.infer_uncached(env, ctx, e, check);
        self.leave();
        let t = r?;
        if check {
            if safe && lparam_free {
                self.st.safe_checked.insert(key, t.0);
            } else if safe {
                self.st.scoped_checked.insert(scoped, t.0);
            } else {
                self.st.check_cache.insert(key, t.0);
            }
        }
        if !self.st.infer_cache.contains(&key) {
            self.st.infer_cache.insert(key, t.0);
        }
        Ok(t)
    }

    fn check_level(&self, l: LevelId) -> KResult<()> {
        if let Some(n) = self.k.s.levels.undef_param(l, &self.k.lparams) {
            return Err(reject(format!(
                "invalid reference to undefined universe level parameter '{}'",
                self.k.s.names.display(n)
            )));
        }
        Ok(())
    }

    fn infer_constant(&mut self, n: NameId, ls: LevelsId, check: bool) -> KResult<V> {
        let Some(c) = self.k.env.get(n).cloned() else {
            return Err(reject(format!(
                "unknown constant '{}'",
                self.k.s.names.display(n)
            )));
        };
        let us = self.k.s.levels.get_list(ls).to_vec();
        if c.lparams.len() != us.len() {
            return Err(reject(format!(
                "incorrect number of universe levels for '{}': {} expected, {} provided",
                self.k.s.names.display(n),
                c.lparams.len(),
                us.len()
            )));
        }
        if check {
            if c.is_unsafe() && self.k.safety != Safety::Unsafe {
                return Err(reject(format!(
                    "invalid declaration, it uses unsafe declaration '{}'",
                    self.k.s.names.display(n)
                )));
            }
            if c.is_partial() && self.k.safety == Safety::Safe {
                return Err(reject(format!(
                    "invalid declaration, safe declaration must not contain partial declaration '{}'",
                    self.k.s.names.display(n)
                )));
            }
            for l in &us {
                self.check_level(*l)?;
            }
        }
        self.const_type(n, ls)
    }

    /// Weak-head normal form, required to be a sort.
    fn ensure_sort(&mut self, t: V, what: &str) -> KResult<LevelId> {
        match self.sort_of(t)? {
            Some(l) => Ok(l),
            None => Err(reject(format!(
                "type expected, {what} has type {}",
                self.show(t)
            ))),
        }
    }

    fn is_eager_arg(&self, a: ExprId) -> bool {
        match self.k.s.const_head(a) {
            Some((n, _)) => n == self.k.wk.eager_reduce && self.k.s.app_num_args(a) == 2,
            None => false,
        }
    }

    fn infer_uncached(&mut self, env: EnvId, ctx: CtxId, e: ExprId, check: bool) -> KResult<V> {
        match self.k.s.node(e) {
            Node::BVar(i) => match self.st.arena.ctx_lookup(ctx, i) {
                Some(t) => Ok(t),
                None => {
                    let v = self.st.arena.lookup(env, i).ok_or_else(|| reject("unexpected bound variable"))?;
                    self.type_of(v)
                }
            },
            Node::FVar(_) => Err(reject("the value engine met a free variable")),
            Node::Sort(l) => {
                if check {
                    self.check_level(l)?;
                }
                let l = self.level_in(env, l);
                let sl = self.k.s.levels.succ(l);
                Ok(self.st.arena.sort(sl))
            }
            Node::Const(n, ls) => {
                let ls = self.levels_in(env, ls);
                self.infer_constant(n, ls, check)
            }
            Node::App(..) => self.infer_app(env, ctx, e, check),
            Node::Lam(dom, body) => {
                if check {
                    let dt = self.infer(env, ctx, dom, true)?;
                    self.ensure_sort(dt, "a binder type")?;
                }
                let d = self.eval(env, dom)?;
                if check {
                    let ctx2 = self.st.arena.ctx_extend(ctx, d);
                    let bt = self.under(d, |sem, x| {
                        let env2 = sem.st.arena.extend(env, x);
                        sem.infer(env2, ctx2, body, true)
                    })?;
                    // A body type that mentions no local is the codomain at every argument: the
                    // λ's type is the plain `Π (_ : d). bt`, which an application instantiates
                    // without inferring the body again.
                    if !self.st.arena.is_open(bt) {
                        return Ok(self.constant_pi(d, bt));
                    }
                }
                Ok(self.st.arena.pi_lam(d, env, ctx, body))
            }
            Node::Pi(dom, body) => {
                let dt = self.infer(env, ctx, dom, check)?;
                let l1 = self.ensure_sort(dt, "a binder type")?;
                let d = self.eval(env, dom)?;
                let ctx2 = self.st.arena.ctx_extend(ctx, d);
                let l2 = self.under(d, |sem, x| {
                    let env2 = sem.st.arena.extend(env, x);
                    let bt = sem.infer(env2, ctx2, body, check)?;
                    sem.ensure_sort(bt, "a Π codomain")
                })?;
                let l = self.k.s.levels.imax(l1, l2);
                Ok(self.st.arena.sort(l))
            }
            Node::Let(ty, val, body) => {
                if check {
                    let tt = self.infer(env, ctx, ty, true)?;
                    self.ensure_sort(tt, "a let type")?;
                }
                let tv = self.eval(env, ty)?;
                if check {
                    let vt = self.infer(env, ctx, val, true)?;
                    if !self.is_def_eq(vt, tv)? {
                        return Err(reject("let value type mismatch").with_detail(format!(
                            "in `{}`\n  expected type: {}\n  but the value has type: {}",
                            self.k.show(e),
                            self.show(tv),
                            self.show(vt)
                        )));
                    }
                }
                let v = self.eval(env, val)?;
                let env2 = self.st.arena.extend(env, v);
                let ctx2 = self.st.arena.ctx_extend(ctx, tv);
                self.infer(env2, ctx2, body, check)
            }
            Node::NatLit(_) => {
                if check && !self.k.env.contains(self.k.wk.nat) {
                    return Err(reject("natural number literal used before 'Nat' is declared"));
                }
                self.const0(self.k.wk.nat)
            }
            Node::StrLit(_) => {
                if check && !(self.k.env.contains(self.k.wk.char_of_nat) && self.k.env.contains(self.k.wk.string_of_list))
                {
                    return Err(reject("string literal used before 'String.ofList' and 'Char.ofNat' are declared"));
                }
                self.const0(self.k.wk.string)
            }
            Node::Proj(s, idx, x) => {
                let xt = self.infer(env, ctx, x, check)?;
                let xv = self.eval(env, x)?;
                self.proj_type(s, idx, xv, xt, check)
            }
            Node::Match(..) | Node::Fix(_) | Node::MutualFix(..) | Node::Lit(_) | Node::Seq(..) | Node::Nil => {
                Err(reject("the value engine checks Lean's theory; LOGOS's is checked by the substitution checker"))
            }
        }
    }

    /// `Π (_ : dom). cod` for a codomain that mentions no local: a closure whose body reads `cod`
    /// from its one-entry environment (`BVar 1` under the binder).
    fn constant_pi(&mut self, dom: V, cod: V) -> V {
        let (body, _) = self.k.s.with_persistent(|s| s.bvar(1));
        let env = self.st.arena.extend(EnvId::EMPTY, cod);
        self.st.arena.pi(dom, env, body)
    }

    fn infer_app(&mut self, env: EnvId, ctx: CtxId, e: ExprId, check: bool) -> KResult<V> {
        let (f, args) = self.k.s.app_args(e);
        let mut fty = self.infer(env, ctx, f, check)?;
        for &a in args.iter() {
            let w = self.whnf(fty)?;
            let dom = match self.st.arena.get(w) {
                Val::Pi { dom, .. } | Val::PiLam { dom, .. } => dom,
                _ => {
                    return Err(reject(format!("function expected, found {}", self.show(w))));
                }
            };
            if check {
                let at = self.infer(env, ctx, a, true)?;
                let ok = if self.is_eager_arg(a) && !self.st.eager {
                    self.st.eager = true;
                    let r = self.is_def_eq(dom, at);
                    self.st.eager = false;
                    r?
                } else {
                    self.is_def_eq(dom, at)?
                };
                if !ok {
                    return Err(
                        KernelFailure::reject("application type mismatch").with_detail(format!(
                            "in `{}`\n  expected type: {}\n  but the argument has type: {}",
                            self.k.show(e),
                            self.show(dom),
                            self.show(at)
                        )),
                    );
                }
            }
            let av = if self.codomain_ignores_binder(w) {
                DEAD
            } else {
                self.eval(env, a)?
            };
            fty = self.instantiate(w, av)?;
        }
        Ok(fty)
    }

    /// Whether a Π-value's codomain provably never mentions its bound variable: then it can be
    /// instantiated without evaluating the argument (the placeholder is never looked up).
    #[inline]
    pub(crate) fn codomain_ignores_binder(&self, pi: V) -> bool {
        match self.st.arena.get(pi) {
            Val::Pi { body, .. } | Val::PiLam { body, .. } => {
                let (n, mask) = self.k.s.loose_bvars(body);
                n == 0 || mask.is_some_and(|m| m & 1 == 0)
            }
            _ => false,
        }
    }
}
