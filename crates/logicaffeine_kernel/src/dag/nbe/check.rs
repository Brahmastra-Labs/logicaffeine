//! Type checking on values: every rule of the classic checker, with types as values.
//!
//! The classic checker walks a term by substituting a free variable for each binder it crosses
//! and building every type as a term. Here the term is walked with an *environment* instead —
//! crossing a binder pushes the local of its depth and type — and every type is a value: an
//! application's type is its function type's closure applied to the argument, a lambda's type is
//! the closure whose codomain is the body's type. Nothing is ever substituted into a term, so
//! checking a term costs time proportional to the term, and the types it builds share everything
//! the evaluator shares.
//!
//! A term is a DAG: one subterm can occur many times. Each is checked once per context it can
//! see — the values and the types of the variables it mentions — and its type is remembered, as
//! the classic checker remembers the type of each (hash-consed) term.
//!
//! The judgment is the classic one rule for rule — the same level, safety and literal checks, the
//! same application, `let` and projection rules (including the restrictions on projecting out of a
//! proposition) — and conversion is [`super::conv`].

use super::value::{CtxId, EnvId, VId, Val};
use super::Nbe;
use crate::dag::env::Safety;
use crate::dag::error::{KResult, KernelFailure};
use crate::dag::expr::{ExprId, Node};
use crate::dag::level::{LevelId, LevelsId};
use crate::dag::name::NameId;

/// The checking context: the environment binding each de Bruijn index to a value, and the
/// context giving each one's type.
pub(crate) struct Cx {
    pub env: EnvId,
    pub ctx: CtxId,
}

impl Cx {
    pub fn empty() -> Cx {
        Cx {
            env: EnvId::EMPTY,
            ctx: CtxId::EMPTY,
        }
    }
}

fn reject(m: impl Into<String>) -> KernelFailure {
    KernelFailure::reject(m)
}

impl Nbe<'_> {
    fn check_level(&self, l: LevelId) -> KResult<()> {
        if let Some(n) = self.k.s.levels.undef_param(l, &self.k.lparams) {
            return Err(reject(format!(
                "invalid reference to undefined universe level parameter '{}'",
                self.k.s.names.display(n)
            )));
        }
        Ok(())
    }

    fn check_constant(&mut self, n: NameId, ls: LevelsId) -> KResult<()> {
        let c =
            self.k.env.get(n).cloned().ok_or_else(|| {
                reject(format!("unknown constant '{}'", self.k.s.names.display(n)))
            })?;
        let us = self.k.s.levels.get_list(ls).to_vec();
        if c.lparams.len() != us.len() {
            return Err(reject(format!(
                "incorrect number of universe levels for '{}': {} expected, {} provided",
                self.k.s.names.display(n),
                c.lparams.len(),
                us.len()
            )));
        }
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
        for l in us {
            self.check_level(l)?;
        }
        Ok(())
    }

    /// Weak-head normal form, required to be a sort; its level.
    fn expect_sort(&mut self, ty: VId, what: &str) -> KResult<LevelId> {
        let w = self.whnf(ty)?;
        match self.st.arena.get(w) {
            Val::Sort(l) => Ok(l),
            _ => Err(reject(format!(
                "type expected, {what} has type {}",
                self.describe(w)
            ))),
        }
    }

    /// Checks `e` in context `cx` and returns its type.
    pub(crate) fn infer_check(&mut self, cx: &mut Cx, e: ExprId) -> KResult<VId> {
        let (n, mask) = self.k.s.loose_bvars(e);
        let seen = (
            self.st.arena.scope(cx.env, n, mask),
            self.st.arena.ctx_scope(cx.ctx, n, mask),
            e,
        );
        if let Some(t) = self.st.check_cache.get(&seen) {
            return Ok(*t);
        }
        self.enter()?;
        let r = self.infer_check_inner(cx, e);
        self.leave();
        let t = r?;
        self.st.check_cache.insert(seen, t);
        Ok(t)
    }

    fn infer_check_inner(&mut self, cx: &mut Cx, e: ExprId) -> KResult<VId> {
        match self.k.s.node(e) {
            Node::BVar(i) => self.st.arena.ctx_lookup(cx.ctx, i).ok_or_else(|| reject("loose bound variable")),
            Node::FVar(i) => {
                let ty = self.k.lctx.get(i as usize).ok_or_else(|| reject("unknown free variable"))?.ty;
                self.eval(EnvId::EMPTY, ty)
            }
            Node::Sort(l) => {
                self.check_level(l)?;
                let sl = self.k.s.levels.succ(l);
                Ok(self.st.arena.alloc(Val::Sort(sl)))
            }
            Node::Const(n, ls) => {
                self.check_constant(n, ls)?;
                self.const_type(n, ls)
            }
            Node::App(..) => self.infer_app(cx, e),
            Node::Lam(..) => self.infer_lam(cx, e),
            Node::Pi(..) => self.infer_pi(cx, e),
            Node::Let(ty, val, body) => {
                let tt = self.infer_check(cx, ty)?;
                self.expect_sort(tt, "a let type")?;
                let tyv = self.eval(cx.env, ty)?;
                let vt = self.infer_check(cx, val)?;
                if !self.conv(vt, tyv)? {
                    return Err(reject("let value type mismatch").with_detail(format!(
                        "declared {}\n  value has {}",
                        self.describe(tyv),
                        self.describe(vt)
                    )));
                }
                let v = self.eval_arg(cx.env, val)?;
                let saved = (cx.env, cx.ctx);
                cx.env = self.st.arena.extend(cx.env, v);
                cx.ctx = self.st.arena.ctx_extend(cx.ctx, tyv);
                let r = self.infer_check(cx, body);
                (cx.env, cx.ctx) = saved;
                r
            }
            Node::NatLit(_) => {
                if !self.k.env.contains(self.k.wk.nat) {
                    return Err(reject("natural number literal used before 'Nat' is declared"));
                }
                self.const0(self.k.wk.nat)
            }
            Node::StrLit(_) => {
                if !(self.k.env.contains(self.k.wk.char_of_nat) && self.k.env.contains(self.k.wk.string_of_list)) {
                    return Err(reject("string literal used before 'String.ofList' and 'Char.ofNat' are declared"));
                }
                self.const0(self.k.wk.string)
            }
            Node::Proj(s, idx, x) => {
                let xt = self.infer_check(cx, x)?;
                let xv = self.eval(cx.env, x)?;
                self.infer_proj(s, idx, xv, xt)
            }
            Node::Match(..) | Node::Fix(_) | Node::MutualFix(..) | Node::Lit(_) | Node::Seq(..) | Node::Nil => {
                Err(reject("the closure engine checks Lean's theory; LOGOS's is checked by the substitution checker"))
            }
        }
    }

    fn infer_app(&mut self, cx: &mut Cx, e: ExprId) -> KResult<VId> {
        let (f, args) = self.k.s.app_args(e);
        let mut fty = self.infer_check(cx, f)?;
        for &a in args.iter() {
            let w = self.whnf(fty)?;
            let Some(dom) = self.pi_domain(w) else {
                return Err(reject("function expected")
                    .with_detail(format!("its type is {}", self.describe(w))));
            };
            let at = self.infer_check(cx, a)?;
            let eager = self.is_eager_reduce(a);
            let saved = self.st.eager;
            if eager {
                self.st.eager = true;
            }
            let ok = self.conv(dom, at);
            self.st.eager = saved;
            if !ok? {
                return Err(reject("application type mismatch").with_detail(format!(
                    "in `{}`\n  expected type: {}\n  but the argument has type: {}",
                    self.k.show(e),
                    self.describe(dom),
                    self.describe(at)
                )));
            }
            let av = self.eval_arg(cx.env, a)?;
            fty = self.pi_codomain(w, av)?;
        }
        Ok(fty)
    }

    fn is_eager_reduce(&self, a: ExprId) -> bool {
        match self.k.s.const_head(a) {
            Some((n, _)) => n == self.k.wk.eager_reduce && self.k.s.app_num_args(a) == 2,
            None => false,
        }
    }

    fn infer_lam(&mut self, cx: &mut Cx, e: ExprId) -> KResult<VId> {
        let Node::Lam(dom, body) = self.k.s.node(e) else {
            unreachable!()
        };
        let dt = self.infer_check(cx, dom)?;
        self.expect_sort(dt, "a binder type")?;
        let d = self.eval(cx.env, dom)?;
        self.infer_under(cx, d, body)?;
        let env = self.env_for(cx.env, e);
        Ok(self.st.arena.alloc(Val::PiOfLam { dom: d, env, body }))
    }

    fn infer_pi(&mut self, cx: &mut Cx, e: ExprId) -> KResult<VId> {
        let Node::Pi(dom, body) = self.k.s.node(e) else {
            unreachable!()
        };
        let dt = self.infer_check(cx, dom)?;
        let l1 = self.expect_sort(dt, "a binder type")?;
        let d = self.eval(cx.env, dom)?;
        let bt = self.infer_under(cx, d, body)?;
        let l2 = self.expect_sort(bt, "a Π codomain")?;
        let l = self.k.s.levels.imax(l1, l2);
        Ok(self.st.arena.alloc(Val::Sort(l)))
    }

    /// Checks `body` under a binder of type `d` and returns its type.
    fn infer_under(&mut self, cx: &mut Cx, d: VId, body: ExprId) -> KResult<VId> {
        self.under(d, |n, x| {
            let saved = (cx.env, cx.ctx);
            cx.env = n.st.arena.extend(cx.env, x);
            cx.ctx = n.st.arena.ctx_extend(cx.ctx, d);
            let r = n.infer_check(cx, body);
            (cx.env, cx.ctx) = saved;
            r
        })
    }

    /// The type of `x.idx`, `x : xt` (Lean's `inferProj`).
    fn infer_proj(&mut self, s: NameId, idx: u32, x: VId, xt: VId) -> KResult<VId> {
        let fail = |k: &Nbe| {
            reject(format!(
                "invalid projection .{} of '{}'",
                idx + 1,
                k.k.s.names.display(s)
            ))
        };
        let ty = self.whnf(xt)?;
        let Some((i, ls, tspine)) = self.const_app(ty) else {
            return Err(fail(self));
        };
        if i != s || !matches!(self.st.arena.get(ty), Val::Neutral { .. }) {
            return Err(fail(self));
        }
        let Some(ind) = self.k.env.inductive(i).cloned() else {
            return Err(fail(self));
        };
        let Some(targs) = self.st.arena.spine_args(tspine) else {
            return Err(fail(self));
        };
        if ind.ctors.len() != 1 || targs.len() != (ind.num_params + ind.num_indices) as usize {
            return Err(fail(self));
        }
        let mut r = self.const_type(ind.ctors[0], ls)?;
        for p in targs.iter().take(ind.num_params as usize) {
            let w = self.whnf(r)?;
            if self.pi_domain(w).is_none() {
                return Err(fail(self));
            }
            r = self.pi_codomain(w, *p)?;
        }
        let sl = self.sort_level(ty)?;
        let maybe_prop = !self.k.s.levels.is_never_zero(sl);
        for f in 0..idx {
            let w = self.whnf(r)?;
            let Some(dom) = self.pi_domain(w) else {
                return Err(fail(self));
            };
            if self.codomain_depends(w) {
                if maybe_prop && !self.is_prop(dom)? {
                    return Err(fail(self));
                }
                let pf = self.proj_lazy(s, f, x)?;
                r = self.pi_codomain(w, pf)?;
            } else {
                r = self.under(dom, |n, dummy| n.pi_codomain(w, dummy))?;
            }
        }
        let w = self.whnf(r)?;
        let Some(dom) = self.pi_domain(w) else {
            return Err(fail(self));
        };
        if maybe_prop && !self.is_prop(dom)? {
            return Err(fail(self));
        }
        Ok(dom)
    }

    /// Whether a Π-value's codomain mentions its bound variable.
    fn codomain_depends(&self, pi: VId) -> bool {
        match self.st.arena.get(pi) {
            Val::Pi { body, .. } | Val::PiOfLam { body, .. } => self.mentions_bvar(body, 0),
            _ => true,
        }
    }

    fn mentions_bvar(&self, e: ExprId, i: u32) -> bool {
        if self.k.s.loose_bvar_range(e) <= i {
            return false;
        }
        match self.k.s.node(e) {
            Node::BVar(j) => j == i,
            Node::App(f, a) => self.mentions_bvar(f, i) || self.mentions_bvar(a, i),
            Node::Lam(t, b) | Node::Pi(t, b) => {
                self.mentions_bvar(t, i) || self.mentions_bvar(b, i + 1)
            }
            Node::Let(t, v, b) => {
                self.mentions_bvar(t, i) || self.mentions_bvar(v, i) || self.mentions_bvar(b, i + 1)
            }
            Node::Proj(_, _, x) => self.mentions_bvar(x, i),
            _ => false,
        }
    }

    // ---- declaration-level entry points ----

    /// Checks that `e` (closed) is a type; its universe level.
    pub fn check_type(&mut self, e: ExprId) -> KResult<LevelId> {
        let mut cx = Cx::empty();
        let t = self.infer_check(&mut cx, e)?;
        self.expect_sort(t, "the declared type")
    }

    /// Checks `value` (closed) against the type `ty` (closed, already checked).
    pub fn check_value(&mut self, value: ExprId, ty: ExprId) -> KResult<()> {
        let mut cx = Cx::empty();
        let vt = self.infer_check(&mut cx, value)?;
        let tv = self.eval(EnvId::EMPTY, ty)?;
        if !self.conv(vt, tv)? {
            return Err(reject("declaration type mismatch").with_detail(format!(
                "the value has type\n    {}\n  but is declared with type\n    {}",
                self.describe(vt),
                self.describe(tv)
            )));
        }
        Ok(())
    }

    /// Whether the closed type `ty` is a proposition.
    pub fn expr_is_prop(&mut self, ty: ExprId) -> KResult<bool> {
        let tv = self.eval(EnvId::EMPTY, ty)?;
        self.is_prop(tv)
    }
}
