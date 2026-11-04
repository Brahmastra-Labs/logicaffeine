//! Weak-head reduction of values: ι (recursors, with K and struct-η), quotients, projections,
//! validated `Nat` literal operations, and δ through constant heads.
//!
//! β is already done by evaluation, so a value's weak head can only be blocked by a recursor or
//! quotient eliminator, a stuck projection, a literal operation, or a definition. Every step is
//! remembered on the value that took it: the unfolding of a definition application, the
//! `whnf_core` and `whnf` of any value.

use super::value::{Head, Kind, Sp, Val, V};
use super::{reject, Sem};
use crate::dag::env::{Kind as CKind, QuotKind, RecInfo};
use crate::dag::error::KResult;
use crate::dag::expr::StrId;
use crate::dag::level::{LevelId, LevelsId};
use crate::dag::name::NameId;
use crate::dag::nat::Nat;

impl Sem<'_> {
    /// The constant heading a neutral, with its arguments.
    #[inline]
    pub(crate) fn const_app(&self, v: V) -> Option<(NameId, LevelsId, Kind, Sp)> {
        match self.st.arena.get(v) {
            Val::Neu { kind, head, spine } => match self.st.arena.head(head) {
                Head::Const { name, levels } => Some((name, levels, kind, spine)),
                _ => None,
            },
            _ => None,
        }
    }

    #[inline]
    pub(crate) fn is_const_app_of(&self, v: V, n: NameId, arity: usize) -> bool {
        match self.const_app(v) {
            Some((m, _, _, sp)) => m == n && self.st.arena.spine_len(sp) == arity,
            None => false,
        }
    }

    /// Weak-head normal form without δ. With `cheap_proj`, a projection's structure is only
    /// brought to `whnf_core` (not unfolded), as Lean's `whnfCore` does inside conversion.
    #[inline]
    pub fn whnf_core(&mut self, v: V, cheap_proj: bool) -> KResult<V> {
        let Val::Neu { kind, .. } = self.st.arena.get(v) else {
            return Ok(v);
        };
        if !matches!(kind, Kind::Rec | Kind::Quot | Kind::Proj) {
            return Ok(v);
        }
        match self.st.arena.whnf_core_memo(v) {
            Some(r) => Ok(r),
            None => self.whnf_core_miss(v, kind, cheap_proj),
        }
    }

    /// [`Sem::whnf_core`] of a value headed by a recursor, a quotient eliminator or a projection,
    /// reduced and (unless projections were only brought to `whnf_core`) remembered on it.
    #[inline(never)]
    fn whnf_core_miss(&mut self, v: V, kind: Kind, cheap_proj: bool) -> KResult<V> {
        self.enter()?;
        let r = self.whnf_core_step(v, kind, cheap_proj);
        self.leave();
        let r = r?;
        if !cheap_proj {
            self.st.arena.set_whnf_core_memo(v, r);
        }
        Ok(r)
    }

    fn whnf_core_step(&mut self, v: V, kind: Kind, cheap_proj: bool) -> KResult<V> {
        let reduced = match kind {
            Kind::Rec => self.inductive_reduce_rec(v)?,
            Kind::Quot => {
                if self.k.env.quot_init {
                    self.quot_reduce_rec(v)?
                } else {
                    None
                }
            }
            Kind::Proj => self.reduce_proj_head(v, cheap_proj)?,
            _ => None,
        };
        match reduced {
            Some(r) => self.whnf_core(r, cheap_proj),
            None => Ok(v),
        }
    }

    /// A stuck projection at the head of `v`, reduced if its structure now reduces to a
    /// constructor; the arguments applied to the projection are applied to the field.
    fn reduce_proj_head(&mut self, v: V, cheap_proj: bool) -> KResult<Option<V>> {
        let Val::Neu { head, spine, .. } = self.st.arena.get(v) else {
            return Ok(None);
        };
        let Head::Proj { idx, x, .. } = self.st.arena.head(head) else {
            return Ok(None);
        };
        let Some(field) = self.reduce_proj(idx, x, cheap_proj)? else {
            return Ok(None);
        };
        let args = self.st.arena.args(spine);
        self.apply_all(field, &args).map(Some)
    }

    /// Field `idx` of `x` if `x` reduces to a constructor application.
    pub(crate) fn reduce_proj(&mut self, idx: u32, x: V, cheap_proj: bool) -> KResult<Option<V>> {
        let c = if cheap_proj {
            self.whnf_core(x, true)?
        } else {
            self.whnf(x)?
        };
        self.proj_field_of(idx, c)
    }

    /// Field `idx` of `c` if `c` is a constructor application (a string literal counting as the
    /// constructor application it abbreviates).
    pub(crate) fn proj_field_of(&mut self, idx: u32, mut c: V) -> KResult<Option<V>> {
        if let Val::Str(s) = self.st.arena.get(c) {
            let e = self.str_to_ctor(s)?;
            c = self.whnf(e)?;
        }
        Ok(self.ctor_field(c, idx))
    }

    /// Field `idx` of a constructor application.
    fn ctor_field(&self, c: V, idx: u32) -> Option<V> {
        let (mk, _, kind, spine) = self.const_app(c)?;
        if kind != Kind::Ctor {
            return None;
        }
        let ci = self.k.env.ctor(mk)?;
        self.st.arena.arg(spine, (ci.num_params + idx) as usize)
    }

    /// `x.idx` as evaluation builds it: the field at once if `x` is a constructor application,
    /// otherwise a stuck projection (the way a `proj` term waits for `whnf_core`).
    pub(crate) fn proj(&mut self, s: NameId, idx: u32, x: V) -> KResult<V> {
        if let Some(f) = self.ctor_field(x, idx) {
            return Ok(f);
        }
        Ok(self
            .st
            .arena
            .head_value(Kind::Proj, Head::Proj { s, idx, x }))
    }

    // ---- δ ----

    /// A definition's body at universe levels, as a value (evaluated once per session).
    pub(crate) fn const_body(&mut self, n: NameId, ls: LevelsId) -> KResult<Option<V>> {
        if let Some(v) = self.st.const_body.get(&(n.0, ls.0)) {
            return Ok(Some(V(v)));
        }
        let Some(c) = self.k.env.get(n) else {
            return Ok(None);
        };
        let Some(body) = c.delta_value() else {
            return Ok(None);
        };
        if self.k.s.levels.get_list(ls).len() != c.lparams.len() {
            return Ok(None);
        }
        let base = self.instance_base(n, ls)?;
        let v = self.eval(base, body)?;
        self.st.const_body.insert((n.0, ls.0), v.0);
        Ok(Some(v))
    }

    /// One δ-step: the definition at the head of `v` unfolded and applied to the arguments.
    pub fn unfold(&mut self, v: V) -> KResult<Option<V>> {
        let Some((n, ls, Kind::Delta, spine)) = self.const_app(v) else {
            return Ok(None);
        };
        if let Some(r) = self.st.arena.delta_memo(v) {
            return Ok(Some(r));
        }
        let Some(body) = self.const_body(n, ls)? else {
            return Ok(None);
        };
        let args = self.st.arena.args(spine);
        let r = self.apply_all(body, &args)?;
        self.st.arena.set_delta_memo(v, r);
        Ok(Some(r))
    }

    /// Full weak-head normal form.
    #[inline]
    pub fn whnf(&mut self, v: V) -> KResult<V> {
        if !matches!(self.st.arena.get(v), Val::Neu { .. }) {
            return Ok(v);
        }
        match self.st.arena.whnf_memo(v) {
            Some(r) => Ok(r),
            None => self.whnf_miss(v),
        }
    }

    /// The weak-head normal form of a neutral, reduced and remembered on it.
    #[inline(never)]
    fn whnf_miss(&mut self, v: V) -> KResult<V> {
        self.enter()?;
        let r = self.whnf_loop(v);
        self.leave();
        let r = r?;
        self.st.arena.set_whnf_memo(v, r);
        Ok(r)
    }

    fn whnf_loop(&mut self, v: V) -> KResult<V> {
        let mut t = v;
        loop {
            t = self.whnf_core(t, false)?;
            if let Some(r) = self.reduce_nat(t)? {
                return Ok(r);
            }
            match self.unfold(t)? {
                Some(u) => t = u,
                None => return Ok(t),
            }
        }
    }

    // ---- literals ----

    pub(crate) fn nat_value(&mut self, n: Nat) -> V {
        let id = self.k.s.nat_id(n);
        self.st.arena.nat(id)
    }

    /// `0` as `Nat.zero`, `n + 1` as `Nat.succ n`.
    pub(crate) fn nat_to_ctor(&mut self, n: &Nat) -> KResult<V> {
        if n.is_zero() {
            return self.const0(self.k.wk.nat_zero);
        }
        let succ = self.const0(self.k.wk.nat_succ)?;
        let pred = self.nat_value(n.sub(&Nat::from_u64(1)));
        Ok(self.st.arena.neutral_app(succ, pred))
    }

    /// A string literal as `String.ofList [Char.ofNat c₀, …]`.
    pub(crate) fn str_to_ctor(&mut self, s: StrId) -> KResult<V> {
        let text = self.k.s.string(s);
        let zero = self.k.s.levels.list(&[LevelId::ZERO]);
        let ch = self.const0(self.k.wk.char_)?;
        let nil_c = self.const_value(self.k.wk.list_nil, zero)?;
        let nil = self.st.arena.neutral_app(nil_c, ch);
        let cons_c = self.const_value(self.k.wk.list_cons, zero)?;
        let cons = self.st.arena.neutral_app(cons_c, ch);
        let of_nat = self.const0(self.k.wk.char_of_nat)?;
        let mut list = nil;
        for c in text.chars().rev() {
            let lit = self.nat_value(Nat::from_u64(c as u64));
            let chr = self.st.arena.neutral_app(of_nat, lit);
            let cell = self.st.arena.neutral_app(cons, chr);
            list = self.st.arena.neutral_app(cell, list);
        }
        let of_list = self.const0(self.k.wk.string_of_list)?;
        Ok(self.st.arena.neutral_app(of_list, list))
    }

    /// `Nat.zero` or a literal, as a number.
    pub(crate) fn raw_nat(&self, v: V) -> Option<Nat> {
        match self.st.arena.get(v) {
            Val::Nat(n) => Some(self.k.s.nat(n).clone()),
            _ => match self.const_app(v) {
                Some((n, ls, _, sp))
                    if n == self.k.wk.nat_zero && ls == LevelsId::EMPTY && sp == Sp::NIL =>
                {
                    Some(Nat::zero())
                }
                _ => None,
            },
        }
    }

    fn bool_value(&mut self, b: bool) -> KResult<V> {
        let n = if b {
            self.k.wk.bool_true
        } else {
            self.k.wk.bool_false
        };
        self.const0(n)
    }

    /// Lean's `reduceNat`: `Nat.succ` and the validated binary operations on literals.
    pub(crate) fn reduce_nat(&mut self, v: V) -> KResult<Option<V>> {
        if !self.k.nat_literal_rules() {
            return Ok(None);
        }
        let Some((n, ls, _, spine)) = self.const_app(v) else {
            return Ok(None);
        };
        let nargs = self.st.arena.spine_len(spine);
        if nargs == 1 {
            if n != self.k.wk.nat_succ || ls != LevelsId::EMPTY {
                return Ok(None);
            }
            let a = self.st.arena.spine_arg(spine, 0);
            let w = self.whnf(a)?;
            return Ok(self.raw_nat(w).map(|x| self.nat_value(x.succ())));
        }
        if nargs != 2 {
            return Ok(None);
        }
        let w = &self.k.wk;
        let op = if n == w.nat_add {
            0
        } else if n == w.nat_sub {
            1
        } else if n == w.nat_mul {
            2
        } else if n == w.nat_pow {
            3
        } else if n == w.nat_gcd {
            4
        } else if n == w.nat_mod {
            5
        } else if n == w.nat_div {
            6
        } else if n == w.nat_beq {
            7
        } else if n == w.nat_ble {
            8
        } else if n == w.nat_land {
            9
        } else if n == w.nat_lor {
            10
        } else if n == w.nat_xor {
            11
        } else if n == w.nat_shift_left {
            12
        } else if n == w.nat_shift_right {
            13
        } else {
            return Ok(None);
        };
        if !self.k.validated_primitives.contains(&n) {
            return Ok(None);
        }
        let (a0, a1) = (
            self.st.arena.spine_arg(spine, 0),
            self.st.arena.spine_arg(spine, 1),
        );
        let wa = self.whnf(a0)?;
        let Some(x) = self.raw_nat(wa) else {
            return Ok(None);
        };
        let wb = self.whnf(a1)?;
        let Some(y) = self.raw_nat(wb) else {
            return Ok(None);
        };
        let r = match op {
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
            7 => return self.bool_value(x == y).map(Some),
            8 => return self.bool_value(x <= y).map(Some),
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
        Ok(Some(self.nat_value(r)))
    }

    // ---- quotients ----

    fn quot_reduce_rec(&mut self, v: V) -> KResult<Option<V>> {
        let Some((n, _, _, spine)) = self.const_app(v) else {
            return Ok(None);
        };
        let (mk_pos, arg_pos) = match self.k.env.get(n).map(|c| &c.kind) {
            Some(CKind::Quot(QuotKind::Lift)) => (5, 3),
            Some(CKind::Quot(QuotKind::Ind)) => (4, 3),
            _ => return Ok(None),
        };
        let args = self.st.arena.args(spine);
        if mk_pos >= args.len() {
            return Ok(None);
        }
        let Some(q) = self
            .k
            .env
            .quotients
            .iter()
            .find(|q| q.lift == n || q.ind == n)
            .copied()
        else {
            return Ok(None);
        };
        let mk = self.whnf(args[mk_pos])?;
        if !self.is_const_app_of(mk, q.mk, 3) {
            return Ok(None);
        }
        let Val::Neu { spine: mks, .. } = self.st.arena.get(mk) else {
            unreachable!()
        };
        let mk_arg = self.st.arena.spine_arg(mks, 2);
        let r = self.apply(args[arg_pos], mk_arg)?;
        self.apply_all(r, &args[mk_pos + 1..]).map(Some)
    }

    // ---- recursors ----

    fn inductive_reduce_rec(&mut self, v: V) -> KResult<Option<V>> {
        let Some((rec, ls, _, spine)) = self.const_app(v) else {
            return Ok(None);
        };
        let Some(c) = self.k.env.get(rec).cloned() else {
            return Ok(None);
        };
        let CKind::Rec(info) = &c.kind else {
            return Ok(None);
        };
        let args = self.st.arena.args(spine);
        let major_idx = info.major_idx();
        let Some(&major0) = args.get(major_idx) else {
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
                match self.st.arena.get(wm) {
                    Val::Nat(n) if self.k.reads_nat_literal(self.k.s.nat(n)) => {
                        let x = self.k.s.nat(n).clone();
                        self.nat_to_ctor(&x)?
                    }
                    Val::Str(s) => {
                        let e = self.str_to_ctor(s)?;
                        self.whnf(e)?
                    }
                    _ => self.to_ctor_when_struct(info.major_induct, wm)?,
                }
            }
        };
        let Some((ctor, _, Kind::Ctor, mspine)) = self.const_app(major) else {
            return Ok(None);
        };
        let Some(rule_idx) = info.rules.iter().position(|r| r.ctor == ctor) else {
            return Ok(None);
        };
        let rule = &info.rules[rule_idx];
        let nfields = rule.nfields as usize;
        let margs = self.st.arena.args(mspine);
        if nfields > margs.len() {
            return Ok(None);
        }
        if self.k.s.levels.get_list(ls).len() != c.lparams.len() {
            return Ok(None);
        }
        let rhs = match self.st.rule_rhs.get(&(rec.0, rule_idx as u32, ls.0)) {
            Some(r) => V(r),
            None => {
                let base = self.instance_base(rec, ls)?;
                let r = self.eval(base, rule.rhs)?;
                self.st.rule_rhs.insert((rec.0, rule_idx as u32, ls.0), r.0);
                r
            }
        };
        let mut r = self.apply_all(rhs, &args[..info.first_index_idx()])?;
        r = self.apply_all(r, &margs[margs.len() - nfields..])?;
        r = self.apply_all(r, &args[major_idx + 1..])?;
        Ok(Some(r))
    }

    /// The inductive's only constructor applied to the parameters of `ty`.
    fn mk_nullary_ctor(&mut self, ty: V, nparams: usize) -> KResult<Option<V>> {
        let Some((d, ls, _, sp)) = self.const_app(ty) else {
            return Ok(None);
        };
        let Some(ctor) = self
            .k
            .env
            .inductive(d)
            .and_then(|i| i.ctors.first().copied())
        else {
            return Ok(None);
        };
        let c = self.const_value(ctor, ls)?;
        let args = self.st.arena.args(sp);
        self.apply_all(c, &args[..nparams.min(args.len())])
            .map(Some)
    }

    fn to_ctor_when_k(&mut self, info: &RecInfo, e: V) -> KResult<V> {
        let et = self.type_of(e)?;
        let app_type = self.whnf(et)?;
        let Some((ti, _, _, _)) = self.const_app(app_type) else {
            return Ok(e);
        };
        if ti != info.major_induct {
            return Ok(e);
        }
        let Some(new_ctor) = self.mk_nullary_ctor(app_type, info.num_params as usize)? else {
            return Ok(e);
        };
        let nt = self.type_of(new_ctor)?;
        if !self.is_def_eq(app_type, nt)? {
            return Ok(e);
        }
        Ok(new_ctor)
    }

    fn is_constructor_app(&self, v: V) -> bool {
        matches!(self.const_app(v), Some((_, _, Kind::Ctor, _)))
    }

    /// The major premise of a structure's recursor as its constructor applied to its projections, taken before the
    /// major is reduced; `None` where that does not apply. By η a structure's value is that application, so the rule
    /// fires without the major's weak head normal form, which a proof need not have
    /// (`corner-cases/eta-struct-major-no-whnf`). It applies to a structure in a universe that is never `Prop`, and to
    /// one in `Prop` whose own recursor eliminates into every universe (`And`), whose fields are all proofs.
    fn eta_major_first(&mut self, info: &RecInfo, major: V) -> KResult<Option<V>> {
        if !self.k.env.is_non_rec_structure(info.major_induct)
            || !matches!(self.st.arena.get(major), Val::Neu { .. })
            || self.is_constructor_app(major)
        {
            return Ok(None);
        }
        let et = self.type_of(major)?;
        let ety = self.whnf(et)?;
        let Some((i, ls, _, sp)) = self.const_app(ety) else {
            return Ok(None);
        };
        if i != info.major_induct {
            return Ok(None);
        }
        let tt = self.type_of(ety)?;
        let sort = self.whnf(tt)?;
        let Val::Sort(u) = self.st.arena.get(sort) else {
            return Ok(None);
        };
        let levels = &self.k.s.levels;
        if !(levels.is_never_zero(u) || (levels.is_always_zero(u) && self.k.env.eliminates_into_every_sort(info.major_induct))) {
            return Ok(None);
        }
        let args = self.st.arena.args(sp);
        self.expand_eta_struct(i, ls, &args, major).map(Some)
    }

    fn to_ctor_when_struct(&mut self, induct: NameId, e: V) -> KResult<V> {
        if !self.k.env.is_non_rec_structure(induct) || self.is_constructor_app(e) {
            return Ok(e);
        }
        let et = self.type_of(e)?;
        let ety = self.whnf(et)?;
        let Some((i, ls, _, sp)) = self.const_app(ety) else {
            return Ok(e);
        };
        if i != induct {
            return Ok(e);
        }
        let tt = self.type_of(ety)?;
        let sort = self.whnf(tt)?;
        let Val::Sort(u) = self.st.arena.get(sort) else {
            return Ok(e);
        };
        if !self.k.s.levels.is_never_zero(u) {
            return Ok(e);
        }
        let args = self.st.arena.args(sp);
        self.expand_eta_struct(i, ls, &args, e)
    }

    /// `mk params (x.1) … (x.n)` for a structure value `x`.
    pub(crate) fn expand_eta_struct(
        &mut self,
        i: NameId,
        ls: LevelsId,
        args: &[V],
        x: V,
    ) -> KResult<V> {
        let ctor = self
            .k
            .env
            .inductive(i)
            .ok_or_else(|| reject("not an inductive type"))?
            .ctors[0];
        let ci = self
            .k
            .env
            .ctor(ctor)
            .ok_or_else(|| reject("not a constructor"))?
            .clone();
        let c = self.const_value(ctor, ls)?;
        let params = &args[..(ci.num_params as usize).min(args.len())];
        let mut all: smallvec::SmallVec<[V; 16]> = smallvec::SmallVec::from_slice(params);
        for f in 0..ci.num_fields {
            all.push(self.proj(i, f, x)?);
        }
        self.apply_all(c, &all)
    }
}
