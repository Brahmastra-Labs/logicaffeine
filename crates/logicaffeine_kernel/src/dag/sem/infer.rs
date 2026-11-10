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
use crate::dag::expr::{ExprId, Node, SHORT_SPINE};
use crate::dag::level::{LevelId, LevelsId};
use crate::dag::name::NameId;

impl Sem<'_> {
    /// The type of `e`, whose bound variables have the values `env` and the types `ctx` (an
    /// empty context means "the types of the values"). A variable's is read in place, and a type
    /// remembered for anything else in an environment [`Sem::known_env`] prunes is found in the
    /// caller's frame. Inferring one, or pruning an environment first, is a call, and a call that
    /// only finds where to infer returns before the inference recurses.
    #[cfg_attr(debug_assertions, inline)]
    #[cfg_attr(not(debug_assertions), inline(always))]
    pub fn infer(&mut self, env: EnvId, ctx: CtxId, e: ExprId, check: bool) -> KResult<V> {
        if let Node::BVar(i) = self.k.s.node(e) {
            if let Some(t) = self.st.arena.ctx_lookup(ctx, i) {
                return Ok(t);
            }
            return match self.st.arena.lookup(env, i) {
                Some(v) => self.type_of(v),
                None => Err(reject("unexpected bound variable")),
            };
        }
        let scope = self.k.s.scope_of(e);
        let known = if e.is_scratch() {
            None
        } else {
            self.known_env(env, scope)
        };
        let (env, cached) = match known {
            Some(pruned) => (pruned, self.remembered_type(pruned, e, scope.2, check)),
            None => self.pruned_type(env, e, check),
        };
        match cached {
            Some(t) => Ok(V(t)),
            None => self.infer_miss(env, ctx, e, check),
        }
    }

    /// The environment `e` is inferred in, pruned from `env`, and the type remembered for it
    /// there: for a term [`Sem::infer`] could not place with no call — one of the scratch region
    /// (which the session must not outlive) or one whose environment must be pruned first.
    #[inline(never)]
    fn pruned_type(&mut self, env: EnvId, e: ExprId, check: bool) -> (EnvId, Option<u32>) {
        self.note_expr(e);
        let scope = self.k.s.scope_of(e);
        let env = self.env_in_scope(env, e, scope);
        (env, self.remembered_type(env, e, scope.2, check))
    }

    /// The type remembered for `e` in its pruned `env` — checked, with `check` — whose universe
    /// parameters `lparams` says it mentions.
    #[inline]
    fn remembered_type(&self, env: EnvId, e: ExprId, lparams: bool, check: bool) -> Option<u32> {
        if check {
            self.checked_type(env, e, lparams)
        } else {
            self.st.infer_cache.get(&(env.0, e.0))
        }
    }

    /// A type inferred with checking for `e` in its pruned `env` whose check still holds: in a
    /// safe declaration, for an expression mentioning no universe parameter (`lparams` false),
    /// in any later declaration; for one mentioning them, in a later safe declaration with the
    /// same parameters; otherwise in this declaration.
    #[inline]
    fn checked_type(&self, env: EnvId, e: ExprId, lparams: bool) -> Option<u32> {
        let key = (env.0, e.0);
        let lasting = if !lparams {
            self.st.safe_checked.get(&key)
        } else if self.k.safety == Safety::Safe {
            self.st.scoped_checked.get(&(self.st.scope, env.0, e.0))
        } else {
            None
        };
        lasting.or_else(|| self.st.check_cache.get(&key))
    }

    /// The type of the compound `e` in its pruned `env`, inferred and remembered.
    #[inline(never)]
    fn infer_miss(&mut self, env: EnvId, ctx: CtxId, e: ExprId, check: bool) -> KResult<V> {
        let node = self.k.s.node(e);
        let lparams = self.k.s.has_lparam(e);
        self.enter()?;
        let r = self.infer_uncached(env, ctx, e, node, check);
        self.leave();
        let t = r?;
        let key = (env.0, e.0);
        if check {
            if self.k.safety != Safety::Safe {
                self.st.check_cache.insert(key, t.0);
            } else if !lparams {
                self.st.safe_checked.insert(key, t.0);
            } else {
                self.st
                    .scoped_checked
                    .insert((self.st.scope, env.0, e.0), t.0);
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
        let given = self.k.s.levels.get_list(ls).len();
        if c.lparams.len() != given {
            return Err(reject(format!(
                "incorrect number of universe levels for '{}': {} expected, {} provided",
                self.k.s.names.display(n),
                c.lparams.len(),
                given
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
            for i in 0..given {
                let l = self.k.s.levels.get_list(ls)[i];
                self.check_level(l)?;
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
        self.k.s.app_num_args(a) == 2
            && matches!(self.k.s.const_head(a), Some((n, _)) if n == self.k.wk.eager_reduce)
    }

    fn infer_uncached(
        &mut self,
        env: EnvId,
        ctx: CtxId,
        e: ExprId,
        node: Node,
        check: bool,
    ) -> KResult<V> {
        match node {
            Node::BVar(_) => unreachable!("a variable's type is read by infer"),
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
                        return Err(self.mismatch("let value type mismatch", "value", e, tv, vt));
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
        let mut short = [ExprId(0); SHORT_SPINE];
        match self.k.s.short_app_args(e, &mut short) {
            Some((f, n)) => self.infer_spine(env, ctx, e, f, &short[..n], check),
            None => {
                let (f, args) = self.k.s.app_args(e);
                self.infer_spine(env, ctx, e, f, &args, check)
            }
        }
    }

    /// The type of the application `e` of `f` to `args`, each argument checked against its
    /// domain when `check`.
    #[inline]
    fn infer_spine(
        &mut self,
        env: EnvId,
        ctx: CtxId,
        e: ExprId,
        f: ExprId,
        args: &[ExprId],
        check: bool,
    ) -> KResult<V> {
        let mut fty = self.infer(env, ctx, f, check)?;
        for &a in args.iter() {
            let w = self.whnf(fty)?;
            let (dom, body) = match self.st.arena.get(w) {
                Val::Pi { dom, body, .. } | Val::PiLam { dom, body, .. } => (dom, body),
                _ => return Err(self.function_expected(w)),
            };
            if check {
                let at = self.infer(env, ctx, a, true)?;
                if at != dom && !self.arg_conforms(a, dom, at)? {
                    return Err(self.mismatch("application type mismatch", "argument", e, dom, at));
                }
            }
            let av = if self.ignores_binder(body) {
                DEAD
            } else {
                self.eval(env, a)?
            };
            fty = self.instantiate(w, av)?;
        }
        Ok(fty)
    }

    /// The refusal of a function type `w` that is not one.
    #[cold]
    #[inline(never)]
    fn function_expected(&mut self, w: V) -> KernelFailure {
        reject(format!("function expected, found {}", self.show(w)))
    }

    /// The refusal `what` of a term `e` whose `part` (its argument, a `let`'s value) has type
    /// `found` where `expected` is required. Built out of line, so that the recursion it
    /// interrupts carries none of its formatting.
    #[cold]
    #[inline(never)]
    fn mismatch(
        &mut self,
        what: &str,
        part: &str,
        e: ExprId,
        expected: V,
        found: V,
    ) -> KernelFailure {
        KernelFailure::reject(what).with_detail(format!(
            "in `{}`\n  expected type: {}\n  but the {part} has type: {}",
            self.k.show(e),
            self.show(expected),
            self.show(found)
        ))
    }

    /// Whether the argument `a`, of type `at`, may be passed for the domain `dom`: their
    /// definitional equality, decided with Lean's `eagerReduce` when `a` asks for it.
    fn arg_conforms(&mut self, a: ExprId, dom: V, at: V) -> KResult<bool> {
        if self.st.eager || !self.is_eager_arg(a) {
            return self.is_def_eq(dom, at);
        }
        self.st.eager = true;
        let r = self.is_def_eq(dom, at);
        self.st.eager = false;
        r
    }

    /// Whether the body of a Π-value provably never mentions its bound variable: then the Π
    /// can be instantiated without evaluating the argument (the placeholder is never looked up).
    #[inline]
    fn ignores_binder(&self, body: ExprId) -> bool {
        let (n, mask) = self.k.s.loose_bvars(body);
        n == 0 || mask.is_some_and(|m| m & 1 == 0)
    }
}
