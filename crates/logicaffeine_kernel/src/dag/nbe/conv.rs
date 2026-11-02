//! Conversion: Lean's definitional-equality algorithm on values.
//!
//! The order of attempts is the classic kernel's — quick structural cases, proof irrelevance,
//! lazy δ-reduction by definitional height (comparing two applications of the same definition
//! argument by argument, last to first, before unfolding either), then rigid spines first to
//! last, η, struct-η, string literals and unit-like types — so both engines give the same
//! verdict. Only the objects differ: a β-step is an environment extension, a δ-step is a
//! remembered evaluation, and a binder is crossed by applying both closures to the local of its
//! depth and type.

use super::value::{ConstKind, Elim, Head, Sp, VId, Val};
use super::Nbe;
use crate::dag::env::Hint;
use crate::dag::error::KResult;
use crate::dag::level::LevelsId;
use crate::dag::nat::Nat;

#[inline]
fn key(a: VId, b: VId) -> (VId, VId) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

enum Step {
    Continue(VId, VId),
    Unknown(VId, VId),
    Done(bool),
}

impl Nbe<'_> {
    /// `a ≡ b`, in the current `eagerReduce` mode.
    pub fn conv(&mut self, a: VId, b: VId) -> KResult<bool> {
        if a == b || self.known_equal(a, b, 8) {
            return Ok(true);
        }
        let k = key(a, b);
        let eager = self.st.eager;
        let stamp = self.st.proven_generation;
        if self.st.refuted.get(&(k.0, k.1, eager)) == Some(&stamp) {
            return Ok(false);
        }
        self.enter()?;
        self.trace("conv", a);
        self.st.conv_steps += 1;
        let r = self.conv_core(a, b);
        self.leave();
        let r = r?;
        if r {
            if self.st.proven.insert(k) {
                self.st.proven_generation += 1;
            }
        } else {
            self.st.refuted.insert((k.0, k.1, eager), stamp);
        }
        Ok(r)
    }

    #[inline]
    fn is_proven(&self, a: VId, b: VId) -> bool {
        self.st.proven.contains(&key(a, b))
    }

    /// Equal, proven equal, or built alike from equal or proven-equal parts — decided without
    /// evaluating anything: two values built from the same expression in environments whose
    /// entries (as far as the expression can see) are pairwise known equal are equal, and so are
    /// two neutrals of one head whose spines are. This is Lean's equivalence manager — congruence
    /// over the pairs already proven — and what makes comparing `f a₁ … aₙ` with `f b₁ … bₙ` cost
    /// nothing once each `aᵢ ≡ bᵢ` is known.
    pub(crate) fn known_equal(&self, a: VId, b: VId, fuel: u32) -> bool {
        if a == b || self.is_proven(a, b) {
            return true;
        }
        if fuel == 0 {
            return false;
        }
        let (a, b) = (self.peek_forced(a), self.peek_forced(b));
        if a == b || self.is_proven(a, b) {
            return true;
        }
        match (self.st.arena.get(a), self.st.arena.get(b)) {
            (Val::Thunk { env: e1, expr: x1 }, Val::Thunk { env: e2, expr: x2 })
            | (
                Val::Lam {
                    env: e1, body: x1, ..
                },
                Val::Lam {
                    env: e2, body: x2, ..
                },
            ) => {
                let dom_ok = match (self.st.arena.get(a), self.st.arena.get(b)) {
                    (Val::Lam { dom: d1, .. }, Val::Lam { dom: d2, .. }) => d1 == d2,
                    _ => true,
                };
                dom_ok && x1 == x2 && self.envs_agree(e1, e2, self.env_reach(a), fuel - 1)
            }
            (
                Val::Pi {
                    dom: d1,
                    env: e1,
                    body: x1,
                },
                Val::Pi {
                    dom: d2,
                    env: e2,
                    body: x2,
                },
            ) => {
                x1 == x2
                    && self.known_equal(d1, d2, fuel - 1)
                    && self.envs_agree(
                        e1,
                        e2,
                        self.k.s.loose_bvar_range(x1).saturating_sub(1),
                        fuel - 1,
                    )
            }
            (Val::Nat(x), Val::Nat(y)) => x == y,
            (Val::Str(x), Val::Str(y)) => x == y,
            (Val::Sort(x), Val::Sort(y)) => x == y,
            (
                Val::Neutral {
                    head: h1,
                    spine: s1,
                },
                Val::Neutral {
                    head: h2,
                    spine: s2,
                },
            ) => h1 == h2 && self.spines_known_equal(s1, s2, fuel - 1),
            (
                Val::Glued {
                    name: n1,
                    levels: l1,
                    spine: s1,
                },
                Val::Glued {
                    name: n2,
                    levels: l2,
                    spine: s2,
                },
            ) => n1 == n2 && l1 == l2 && self.spines_known_equal(s1, s2, fuel - 1),
            _ => false,
        }
    }

    /// The forced value of a thunk if it has been forced, else the thunk itself.
    fn peek_forced(&self, mut v: VId) -> VId {
        while let Val::Thunk { .. } = self.st.arena.get(v) {
            match self.st.arena.memo(v) {
                Some(r) => v = r,
                None => break,
            }
        }
        v
    }

    /// How many environment entries a thunk or lambda's expression can see.
    fn env_reach(&self, v: VId) -> u32 {
        match self.st.arena.get(v) {
            Val::Thunk { expr, .. } => self.k.s.loose_bvar_range(expr),
            Val::Lam { dom, body, .. } => self
                .k
                .s
                .loose_bvar_range(dom)
                .max(self.k.s.loose_bvar_range(body).saturating_sub(1)),
            _ => 0,
        }
    }

    fn envs_agree(
        &self,
        e1: super::value::EnvId,
        e2: super::value::EnvId,
        n: u32,
        fuel: u32,
    ) -> bool {
        if e1 == e2 {
            return true;
        }
        let (n1, n2) = (self.st.arena.env(e1), self.st.arena.env(e2));
        if n1.lsub != n2.lsub {
            let (l1, l2) = (
                &self.st.arena.lsubs[n1.lsub as usize],
                &self.st.arena.lsubs[n2.lsub as usize],
            );
            if l1.params != l2.params || l1.levels != l2.levels {
                return false;
            }
        }
        for i in 0..n {
            match (self.st.arena.lookup(e1, i), self.st.arena.lookup(e2, i)) {
                (Some(x), Some(y)) => {
                    if !self.known_equal(x, y, fuel) {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        true
    }

    fn spines_known_equal(&self, mut s1: Sp, mut s2: Sp, fuel: u32) -> bool {
        loop {
            if s1 == s2 {
                return true;
            }
            if s1 == Sp::NIL || s2 == Sp::NIL {
                return false;
            }
            let (p1, e1) = self.st.arena.spine_node(s1);
            let (p2, e2) = self.st.arena.spine_node(s2);
            let same = match (e1, e2) {
                (Elim::App(x), Elim::App(y)) => self.known_equal(x, y, fuel),
                (Elim::Proj(_, i), Elim::Proj(_, j)) => i == j,
                _ => false,
            };
            if !same {
                return false;
            }
            s1 = p1;
            s2 = p2;
        }
    }

    fn quick(&mut self, a: VId, b: VId) -> KResult<Option<bool>> {
        if a == b || self.known_equal(a, b, 8) {
            return Ok(Some(true));
        }
        Ok(match (self.st.arena.get(a), self.st.arena.get(b)) {
            (Val::Sort(x), Val::Sort(y)) => Some(self.k.s.levels.equiv(x, y)),
            (Val::Nat(x), Val::Nat(y)) => Some(x == y),
            (Val::Str(x), Val::Str(y)) => Some(x == y),
            (Val::Nat(_), Val::Str(_)) | (Val::Str(_), Val::Nat(_)) => Some(false),
            (Val::Lam { .. }, Val::Lam { .. }) => Some(self.conv_lams(a, b)?),
            (Val::Pi { .. } | Val::PiOfLam { .. }, Val::Pi { .. } | Val::PiOfLam { .. }) => {
                Some(self.conv_pis(a, b)?)
            }
            _ => None,
        })
    }

    fn lam_parts(
        &mut self,
        v: VId,
    ) -> KResult<(VId, super::value::EnvId, crate::dag::expr::ExprId)> {
        let Val::Lam { dom, env, body } = self.st.arena.get(v) else {
            unreachable!()
        };
        let d = self.eval(env, dom)?;
        Ok((d, env, body))
    }

    fn conv_lams(&mut self, a: VId, b: VId) -> KResult<bool> {
        let (da, ea, ba) = self.lam_parts(a)?;
        let (db, eb, bb) = self.lam_parts(b)?;
        if !self.conv(da, db)? {
            return Ok(false);
        }
        self.under(db, |n, x| {
            let va = n.instantiate_closure(ea, ba, x)?;
            let vb = n.instantiate_closure(eb, bb, x)?;
            n.conv(va, vb)
        })
    }

    fn conv_pis(&mut self, a: VId, b: VId) -> KResult<bool> {
        let da = self.pi_domain(a).unwrap();
        let db = self.pi_domain(b).unwrap();
        if !self.conv(da, db)? {
            return Ok(false);
        }
        self.under(db, |n, x| {
            let ca = n.pi_codomain(a, x)?;
            let cb = n.pi_codomain(b, x)?;
            n.conv(ca, cb)
        })
    }

    /// Whether `v` mentions a variable — a bound variable of conversion or a free variable of the
    /// local context. The classic engine's `has_fvar`: literal arithmetic and the `≡ true`
    /// shortcut are only attempted on closed values, where they cannot unfold arithmetic over a
    /// variable into a unary recursion.
    pub(crate) fn has_locals(&mut self, v: VId) -> bool {
        if let Some(r) = self.st.locals_cache.get(&v) {
            return *r;
        }
        let r = match self.st.arena.get(v) {
            Val::Sort(_) | Val::Nat(_) | Val::Str(_) => false,
            Val::Neutral {
                head: Head::Local(_) | Head::FVar(_),
                ..
            } => true,
            Val::Neutral { spine, .. } | Val::Glued { spine, .. } => self.spine_has_locals(spine),
            Val::Lam { dom, env, body } => {
                self.k.s.has_fvar(dom) || self.k.s.has_fvar(body) || self.env_has_locals(env)
            }
            Val::Pi { dom, env, body } | Val::PiOfLam { dom, env, body } => {
                self.has_locals(dom) || self.k.s.has_fvar(body) || self.env_has_locals(env)
            }
            Val::Thunk { env, expr } => match self.st.arena.memo(v) {
                Some(f) => self.has_locals(f),
                None => self.k.s.has_fvar(expr) || self.env_has_locals(env),
            },
        };
        self.st.locals_cache.insert(v, r);
        r
    }

    fn spine_has_locals(&mut self, mut s: Sp) -> bool {
        while s != Sp::NIL {
            let (p, e) = self.st.arena.spine_node(s);
            if let Elim::App(a) = e {
                if self.has_locals(a) {
                    return true;
                }
            }
            s = p;
        }
        false
    }

    fn env_has_locals(&mut self, mut e: super::value::EnvId) -> bool {
        loop {
            let n = self.st.arena.env(e);
            if n.len == 0 {
                return false;
            }
            if self.has_locals(n.val) {
                return true;
            }
            e = n.parent;
        }
    }

    fn is_bool_true(&self, v: VId) -> bool {
        matches!(self.st.arena.get(v), Val::Neutral { head: Head::Const(ConstKind::Ctor, n, _), spine: Sp::NIL } if n == self.k.wk.bool_true)
    }

    fn conv_core(&mut self, a: VId, b: VId) -> KResult<bool> {
        let a = self.force(a)?;
        let b = self.force(b)?;
        if let Some(r) = self.quick(a, b)? {
            return Ok(r);
        }
        if self.is_bool_true(b) && (self.st.eager || !self.has_locals(a)) {
            let w = self.whnf(a)?;
            if self.is_bool_true(w) {
                return Ok(true);
            }
        }
        let tn = self.whnf_core(a)?;
        let sn = self.whnf_core(b)?;
        if tn != a || sn != b {
            if let Some(r) = self.quick(tn, sn)? {
                return Ok(r);
            }
        }
        let tt = self.type_of(tn)?;
        if self.is_prop(tt)? {
            let st = self.type_of(sn)?;
            return self.conv(tt, st);
        }
        let (tn, sn) = match self.lazy_delta(tn, sn)? {
            Step::Done(r) => return Ok(r),
            Step::Unknown(x, y) => (x, y),
            Step::Continue(..) => unreachable!(),
        };
        if self.conv_rigid(tn, sn)? {
            return Ok(true);
        }
        if self.try_eta(tn, sn)? || self.try_eta(sn, tn)? {
            return Ok(true);
        }
        if self.try_eta_struct(tn, sn)? || self.try_eta_struct(sn, tn)? {
            return Ok(true);
        }
        match self.try_string_lit(tn, sn)? {
            Some(r) => return Ok(r),
            None => {
                if let Some(r) = self.try_string_lit(sn, tn)? {
                    return Ok(r);
                }
            }
        }
        self.unit_like(tn, sn)
    }

    // ---- lazy δ ----

    fn glued_hint(&self, v: VId) -> Option<Hint> {
        let Val::Glued { name, levels, .. } = self.st.arena.get(v) else {
            return None;
        };
        let c = self.k.env.get(name)?;
        (c.delta_value().is_some() && self.k.s.levels.get_list(levels).len() == c.lparams.len())
            .then(|| c.hint())
    }

    fn delta(&mut self, v: VId) -> KResult<VId> {
        let u = self.unfold(v)?.expect("δ on a value that does not unfold");
        self.whnf_core(u)
    }

    fn nat_pred(&mut self, v: VId) -> Option<VId> {
        match self.st.arena.get(v) {
            Val::Nat(n) => {
                let x = self.k.s.nat(n).clone();
                (!x.is_zero()).then(|| self.nat_value(x.sub(&Nat::from_u64(1))))
            }
            Val::Neutral {
                head: Head::Const(ConstKind::Ctor, n, _),
                spine,
            } if n == self.k.wk.nat_succ => {
                let (prev, e) = self.st.arena.spine_node(spine);
                match (prev, e) {
                    (Sp::NIL, Elim::App(x)) if spine != Sp::NIL => Some(x),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn is_nat_zero(&self, v: VId) -> bool {
        match self.st.arena.get(v) {
            Val::Nat(n) => self.k.s.nat(n).is_zero(),
            Val::Neutral {
                head: Head::Const(ConstKind::Ctor, n, ls),
                spine: Sp::NIL,
            } => n == self.k.wk.nat_zero && ls == LevelsId::EMPTY,
            _ => false,
        }
    }

    fn lazy_delta(&mut self, mut tn: VId, mut sn: VId) -> KResult<Step> {
        loop {
            if self.is_nat_zero(tn) && self.is_nat_zero(sn) {
                return Ok(Step::Done(true));
            }
            if let (Some(x), Some(y)) = (self.nat_pred(tn), self.nat_pred(sn)) {
                return Ok(Step::Done(self.conv(x, y)?));
            }
            if self.st.eager || (!self.has_locals(tn) && !self.has_locals(sn)) {
                if let Some(t2) = self.reduce_nat(tn)? {
                    return Ok(Step::Done(self.conv(t2, sn)?));
                }
                if let Some(s2) = self.reduce_nat(sn)? {
                    return Ok(Step::Done(self.conv(tn, s2)?));
                }
            }
            let step = match (self.glued_hint(tn), self.glued_hint(sn)) {
                (None, None) => return Ok(Step::Unknown(tn, sn)),
                (Some(_), None) => {
                    let t2 = self.delta(tn)?;
                    self.after_step(t2, sn)?
                }
                (None, Some(_)) => {
                    let s2 = self.delta(sn)?;
                    self.after_step(tn, s2)?
                }
                (Some(ht), Some(hs)) => {
                    if ht.lt(hs) {
                        let s2 = self.delta(sn)?;
                        self.after_step(tn, s2)?
                    } else if hs.lt(ht) {
                        let t2 = self.delta(tn)?;
                        self.after_step(t2, sn)?
                    } else {
                        if let (
                            Val::Glued {
                                name: n1,
                                levels: l1,
                                spine: s1,
                            },
                            Val::Glued {
                                name: n2,
                                levels: l2,
                                spine: s2,
                            },
                        ) = (self.st.arena.get(tn), self.st.arena.get(sn))
                        {
                            let pair = key(tn, sn);
                            if n1 == n2
                                && matches!(ht, Hint::Regular(_))
                                && s1 != Sp::NIL
                                && s2 != Sp::NIL
                                && !self.st.failure.contains(&pair)
                            {
                                if self.k.s.levels.equiv_lists(l1, l2)
                                    && self.conv_spines(s1, s2)?
                                {
                                    return Ok(Step::Done(true));
                                }
                                self.st.failure.insert(pair);
                            }
                        }
                        let t2 = self.delta(tn)?;
                        let s2 = self.delta(sn)?;
                        self.after_step(t2, s2)?
                    }
                }
            };
            match step {
                Step::Continue(x, y) => {
                    tn = x;
                    sn = y;
                }
                done => return Ok(done),
            }
        }
    }

    fn after_step(&mut self, tn: VId, sn: VId) -> KResult<Step> {
        Ok(match self.quick(tn, sn)? {
            Some(r) => Step::Done(r),
            None => Step::Continue(tn, sn),
        })
    }

    /// The arguments of two applications of one definition, last to first (Lean's
    /// `is_def_eq_args`).
    fn conv_spines(&mut self, s1: Sp, s2: Sp) -> KResult<bool> {
        let (mut x, mut y) = (s1, s2);
        loop {
            match (x == Sp::NIL, y == Sp::NIL) {
                (true, true) => return Ok(true),
                (true, false) | (false, true) => return Ok(false),
                _ => {}
            }
            let (px, ex) = self.st.arena.spine_node(x);
            let (py, ey) = self.st.arena.spine_node(y);
            let ok = match (ex, ey) {
                (Elim::App(a), Elim::App(b)) => self.conv(a, b)?,
                (Elim::Proj(_, i), Elim::Proj(_, j)) => i == j,
                _ => false,
            };
            if !ok {
                return Ok(false);
            }
            x = px;
            y = py;
        }
    }

    // ---- rigid comparison ----

    /// Two rigid values: one head, then the eliminations first to last (Lean's `is_def_eq_app`
    /// order — the first differing argument decides, before any later one is looked at).
    fn conv_rigid(&mut self, tn: VId, sn: VId) -> KResult<bool> {
        let (s1, s2) = match (self.st.arena.get(tn), self.st.arena.get(sn)) {
            (
                Val::Neutral {
                    head: h1,
                    spine: s1,
                },
                Val::Neutral {
                    head: h2,
                    spine: s2,
                },
            ) => {
                let heads = match (h1, h2) {
                    (Head::Local(i), Head::Local(j)) => i == j,
                    (Head::FVar(i), Head::FVar(j)) => i == j,
                    (Head::Const(_, n1, l1), Head::Const(_, n2, l2)) => {
                        n1 == n2 && self.k.s.levels.equiv_lists(l1, l2)
                    }
                    _ => false,
                };
                if !heads {
                    return Ok(false);
                }
                (s1, s2)
            }
            (
                Val::Glued {
                    name: n1,
                    levels: l1,
                    spine: s1,
                },
                Val::Glued {
                    name: n2,
                    levels: l2,
                    spine: s2,
                },
            ) if n1 == n2 && self.k.s.levels.equiv_lists(l1, l2) => (s1, s2),
            _ => return Ok(false),
        };
        let e1 = self.st.arena.spine_elims(s1);
        let e2 = self.st.arena.spine_elims(s2);
        if e1.len() != e2.len() {
            return Ok(false);
        }
        for (x, y) in e1.into_iter().zip(e2) {
            let ok = match (x, y) {
                (Elim::App(a), Elim::App(b)) => self.conv(a, b)?,
                (Elim::Proj(_, i), Elim::Proj(_, j)) => i == j,
                _ => false,
            };
            if !ok {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// η: `t` a lambda, `s` not — compare `t x` with `s x` under a binder of `s`'s domain.
    fn try_eta(&mut self, t: VId, s: VId) -> KResult<bool> {
        if !matches!(self.st.arena.get(t), Val::Lam { .. })
            || matches!(self.st.arena.get(s), Val::Lam { .. })
        {
            return Ok(false);
        }
        let st = self.type_of(s)?;
        let sty = self.whnf(st)?;
        let Some(sdom) = self.pi_domain(sty) else {
            return Ok(false);
        };
        let (tdom, env, body) = self.lam_parts(t)?;
        if !self.conv(tdom, sdom)? {
            return Ok(false);
        }
        self.under(sdom, |n, x| {
            let tb = n.instantiate_closure(env, body, x)?;
            let sx = n.apply(s, x)?;
            n.conv(tb, sx)
        })
    }

    /// Struct-η: `s` a full constructor application of a non-recursive structure, `t` compared
    /// field by field through projections.
    fn try_eta_struct(&mut self, t: VId, s: VId) -> KResult<bool> {
        let Val::Neutral {
            head: Head::Const(ConstKind::Ctor, c, _),
            spine,
        } = self.st.arena.get(s)
        else {
            return Ok(false);
        };
        let Some(ci) = self.k.env.ctor(c).cloned() else {
            return Ok(false);
        };
        let Some(args) = self.st.arena.spine_args(spine) else {
            return Ok(false);
        };
        if args.len() != (ci.num_params + ci.num_fields) as usize
            || !self.k.env.is_non_rec_structure(ci.induct)
        {
            return Ok(false);
        }
        let tt = self.type_of(t)?;
        let st = self.type_of(s)?;
        if !self.conv(tt, st)? {
            return Ok(false);
        }
        for i in ci.num_params as usize..args.len() {
            let p = self.proj(ci.induct, (i - ci.num_params as usize) as u32, t)?;
            if !self.conv(p, args[i])? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn try_string_lit(&mut self, t: VId, s: VId) -> KResult<Option<bool>> {
        let Val::Str(sid) = self.st.arena.get(t) else {
            return Ok(None);
        };
        let Some((n, ls, spine)) = self.const_app(s) else {
            return Ok(None);
        };
        if n != self.k.wk.string_of_list
            || ls != LevelsId::EMPTY
            || self.st.arena.spine_len(spine) != 1
        {
            return Ok(None);
        }
        let x = self.str_to_ctor(sid)?;
        Ok(Some(self.conv(x, s)?))
    }

    fn unit_like(&mut self, t: VId, s: VId) -> KResult<bool> {
        let tt = self.type_of(t)?;
        let tty = self.whnf(tt)?;
        let Some((i, _, _)) = self.const_app(tty) else {
            return Ok(false);
        };
        let Some(ind) = self.k.env.inductive(i) else {
            return Ok(false);
        };
        if ind.is_rec || ind.ctors.len() != 1 || ind.num_indices != 0 {
            return Ok(false);
        }
        if self.k.env.ctor(ind.ctors[0]).map(|c| c.num_fields) != Some(0) {
            return Ok(false);
        }
        let st = self.type_of(s)?;
        self.conv(tty, st)
    }
}
