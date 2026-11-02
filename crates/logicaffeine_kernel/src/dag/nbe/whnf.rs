//! Weak-head reduction of values: ι (recursors, with K and struct-η), quotients, projections,
//! validated `Nat` literal operations, and δ through glued heads.

use super::value::{ConstKind, Elim, EnvId, Head, Sp, VId, Val};
use super::Nbe;
use crate::dag::env::{Kind, QuotKind, RecInfo};
use crate::dag::error::{KResult, KernelFailure};
use crate::dag::level::{LevelId, LevelsId};
use crate::dag::name::NameId;
use crate::dag::nat::Nat;

impl Nbe<'_> {
    /// The constant heading a neutral or glued value, with its arguments if the spine is all
    /// applications.
    pub(crate) fn const_app(&self, v: VId) -> Option<(NameId, LevelsId, Sp)> {
        match self.st.arena.get(v) {
            Val::Neutral {
                head: Head::Const(_, n, ls),
                spine,
            } => Some((n, ls, spine)),
            Val::Glued {
                name,
                levels,
                spine,
            } => Some((name, levels, spine)),
            _ => None,
        }
    }

    pub(crate) fn is_ctor_app(&self, v: VId) -> bool {
        matches!(
            self.st.arena.get(v),
            Val::Neutral {
                head: Head::Const(ConstKind::Ctor, ..),
                ..
            }
        )
    }

    /// Weak-head normal form without δ: ι, quotient and projection steps only.
    pub fn whnf_core(&mut self, v: VId) -> KResult<VId> {
        let v = self.force(v)?;
        if !matches!(
            self.st.arena.get(v),
            Val::Neutral {
                head: Head::Const(ConstKind::Rec | ConstKind::Quot, ..),
                ..
            }
        ) {
            return Ok(v);
        }
        if let Some(r) = self.st.whnf_core_cache.get(&v) {
            return Ok(*r);
        }
        self.enter()?;
        self.trace("whnf_core", v);
        let r = self.whnf_core_step(v);
        self.leave();
        let r = r?;
        self.st.whnf_core_cache.insert(v, r);
        Ok(r)
    }

    fn whnf_core_step(&mut self, v: VId) -> KResult<VId> {
        let mut cur = v;
        loop {
            let reduced = match self.st.arena.get(cur) {
                Val::Neutral {
                    head: Head::Const(ConstKind::Rec, n, ls),
                    spine,
                } => self.iota(n, ls, spine)?,
                Val::Neutral {
                    head: Head::Const(ConstKind::Quot, n, _),
                    spine,
                } => self.quot_reduce(n, spine)?,
                _ => None,
            };
            match reduced {
                Some(r) => cur = self.force(r)?,
                None => return Ok(cur),
            }
        }
    }

    /// Full weak-head normal form.
    pub fn whnf(&mut self, v: VId) -> KResult<VId> {
        let v = self.force(v)?;
        match self.st.arena.get(v) {
            Val::Sort(_)
            | Val::Pi { .. }
            | Val::PiOfLam { .. }
            | Val::Lam { .. }
            | Val::Nat(_)
            | Val::Str(_) => return Ok(v),
            Val::Neutral {
                head: Head::Local(_) | Head::FVar(_),
                ..
            } => return Ok(v),
            _ => {}
        }
        if let Some(r) = self.st.whnf_cache.get(&v) {
            return Ok(*r);
        }
        self.enter()?;
        self.trace("whnf", v);
        let r = self.whnf_loop(v);
        self.leave();
        let r = r?;
        self.st.whnf_cache.insert(v, r);
        Ok(r)
    }

    fn whnf_loop(&mut self, v: VId) -> KResult<VId> {
        let mut t = v;
        loop {
            t = self.whnf_core(t)?;
            if let Some(r) = self.reduce_nat(t)? {
                return Ok(r);
            }
            match self.unfold(t)? {
                Some(u) => t = u,
                None => return Ok(t),
            }
        }
    }

    /// One δ-step: a glued value's unfolding (memoized on the value).
    pub fn unfold(&mut self, v: VId) -> KResult<Option<VId>> {
        let Val::Glued {
            name,
            levels,
            spine,
        } = self.st.arena.get(v)
        else {
            return Ok(None);
        };
        if let Some(r) = self.st.arena.memo(v) {
            return Ok(Some(r));
        }
        let c = self
            .k
            .env
            .get(name)
            .cloned()
            .ok_or_else(|| KernelFailure::reject("unknown constant"))?;
        let Some(body) = c.delta_value() else {
            return Ok(None);
        };
        let us = self.k.s.levels.get_list(levels);
        if us.len() != c.lparams.len() {
            return Ok(None);
        }
        let head = self.const_body(name, levels, c.lparams.clone(), body)?;
        let elims = self.st.arena.spine_elims(spine);
        let r = self.apply_elims(head, &elims)?;
        self.st.arena.set_memo(v, r);
        Ok(Some(r))
    }

    fn const_body(
        &mut self,
        name: NameId,
        levels: LevelsId,
        ps: std::rc::Rc<[NameId]>,
        body: crate::dag::expr::ExprId,
    ) -> KResult<VId> {
        let key = (name, levels);
        if let Some(v) = self.st.const_body.get(&key) {
            return Ok(*v);
        }
        let env = if ps.is_empty() {
            EnvId::EMPTY
        } else {
            self.st.arena.with_lsub(ps, levels)
        };
        let v = self.eval(env, body)?;
        self.st.const_body.insert(key, v);
        Ok(v)
    }

    /// The projection `v.idx` as evaluation builds it: taken at once from a constructor
    /// application, otherwise left pending on the value's spine until something demands its weak
    /// head (as the classic engine leaves a `proj` node until `whnf_core` reaches it).
    pub fn proj_lazy(&mut self, s: NameId, idx: u32, v: VId) -> KResult<VId> {
        let v = self.force(v)?;
        if let Some(f) = self.ctor_field(v, idx)? {
            return Ok(f);
        }
        match self.st.arena.get(v) {
            Val::Neutral { head, spine }
                if !matches!(head, Head::Const(ConstKind::Rec | ConstKind::Quot, ..)) =>
            {
                let spine = self.st.arena.snoc(spine, Elim::Proj(s, idx));
                Ok(self.st.arena.alloc(Val::Neutral { head, spine }))
            }
            Val::Glued {
                name,
                levels,
                spine,
            } => {
                let spine = self.st.arena.snoc(spine, Elim::Proj(s, idx));
                Ok(self.st.arena.alloc(Val::Glued {
                    name,
                    levels,
                    spine,
                }))
            }
            _ => self.proj(s, idx, v),
        }
    }

    fn ctor_field(&mut self, w: VId, idx: u32) -> KResult<Option<VId>> {
        if let Val::Neutral {
            head: Head::Const(ConstKind::Ctor, c, _),
            spine,
        } = self.st.arena.get(w)
        {
            if let (Some(ci), Some(args)) =
                (self.k.env.ctor(c).cloned(), self.st.arena.spine_args(spine))
            {
                return Ok(args.get((ci.num_params + idx) as usize).copied());
            }
        }
        Ok(None)
    }

    /// Projection `v.idx` of structure `s`, reducing `v` to weak-head normal form.
    pub fn proj(&mut self, s: NameId, idx: u32, v: VId) -> KResult<VId> {
        let mut w = self.whnf(v)?;
        if let Val::Str(sid) = self.st.arena.get(w) {
            let e = self.str_to_ctor(sid)?;
            w = self.whnf(e)?;
        }
        if let Val::Neutral {
            head: Head::Const(ConstKind::Ctor, c, _),
            spine,
        } = self.st.arena.get(w)
        {
            if let (Some(ci), Some(args)) =
                (self.k.env.ctor(c).cloned(), self.st.arena.spine_args(spine))
            {
                if let Some(a) = args.get((ci.num_params + idx) as usize) {
                    return Ok(*a);
                }
            }
        }
        match self.st.arena.get(w) {
            Val::Neutral { head, spine } => {
                let spine = self.st.arena.snoc(spine, Elim::Proj(s, idx));
                Ok(self.st.arena.alloc(Val::Neutral { head, spine }))
            }
            Val::Glued {
                name,
                levels,
                spine,
            } => {
                let spine = self.st.arena.snoc(spine, Elim::Proj(s, idx));
                Ok(self.st.arena.alloc(Val::Glued {
                    name,
                    levels,
                    spine,
                }))
            }
            _ => Err(KernelFailure::reject(
                "projection of a value that is not a structure",
            )),
        }
    }

    // ---- literals ----

    pub(crate) fn nat_value(&mut self, n: Nat) -> VId {
        let e = self.k.s.nat_lit(n);
        let crate::dag::expr::Node::NatLit(id) = self.k.s.node(e) else {
            unreachable!()
        };
        self.st.arena.alloc(Val::Nat(id))
    }

    /// A natural-number literal as `Nat.zero` / `Nat.succ n`.
    pub(crate) fn nat_to_ctor(&mut self, n: &Nat) -> KResult<VId> {
        if n.is_zero() {
            return self.const0(self.k.wk.nat_zero);
        }
        let succ = self.const0(self.k.wk.nat_succ)?;
        let pred = self.nat_value(n.sub(&Nat::from_u64(1)));
        self.apply(succ, pred)
    }

    /// A string literal as `String.ofList [Char.ofNat c₀, …]`.
    pub(crate) fn str_to_ctor(&mut self, sid: crate::dag::expr::StrId) -> KResult<VId> {
        let e = self.k.str_lit_to_ctor(sid);
        self.eval(EnvId::EMPTY, e)
    }

    /// `Nat.zero` or a literal, as a number.
    pub(crate) fn raw_nat(&self, v: VId) -> Option<Nat> {
        match self.st.arena.get(v) {
            Val::Nat(n) => Some(self.k.s.nat(n).clone()),
            Val::Neutral {
                head: Head::Const(ConstKind::Ctor, n, ls),
                spine: Sp::NIL,
            } if n == self.k.wk.nat_zero && ls == LevelsId::EMPTY => Some(Nat::zero()),
            _ => None,
        }
    }

    fn bool_value(&mut self, b: bool) -> KResult<VId> {
        let n = if b {
            self.k.wk.bool_true
        } else {
            self.k.wk.bool_false
        };
        self.const0(n)
    }

    /// `Nat.succ` and the validated binary operations on literals (Lean's `reduceNat`).
    pub(crate) fn reduce_nat(&mut self, v: VId) -> KResult<Option<VId>> {
        let Some((n, ls, spine)) = self.const_app(v) else {
            return Ok(None);
        };
        let Some(args) = self.st.arena.spine_args(spine) else {
            return Ok(None);
        };
        let w = &self.k.wk;
        if args.len() == 1 && n == w.nat_succ && ls == LevelsId::EMPTY {
            let a = self.whnf(args[0])?;
            return Ok(match self.raw_nat(a) {
                Some(x) => Some(self.nat_value(x.succ())),
                None => None,
            });
        }
        if args.len() != 2 || !self.k.validated_primitives.contains(&n) {
            return Ok(None);
        }
        let w = &self.k.wk;
        let op = [
            w.nat_add,
            w.nat_sub,
            w.nat_mul,
            w.nat_pow,
            w.nat_gcd,
            w.nat_mod,
            w.nat_div,
            w.nat_beq,
            w.nat_ble,
            w.nat_land,
            w.nat_lor,
            w.nat_xor,
            w.nat_shift_left,
            w.nat_shift_right,
        ]
        .iter()
        .position(|x| *x == n);
        let Some(op) = op else { return Ok(None) };
        let a = self.whnf(args[0])?;
        let Some(x) = self.raw_nat(a) else {
            return Ok(None);
        };
        let b = self.whnf(args[1])?;
        let Some(y) = self.raw_nat(b) else {
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

    fn quot_reduce(&mut self, n: NameId, spine: Sp) -> KResult<Option<VId>> {
        let kind = match self.k.env.get(n).map(|c| c.kind.clone()) {
            Some(Kind::Quot(k)) => k,
            _ => return Ok(None),
        };
        let (mk_pos, arg_pos) = match kind {
            QuotKind::Lift => (5, 3),
            QuotKind::Ind => (4, 3),
            _ => return Ok(None),
        };
        let elims = self.st.arena.spine_elims(spine);
        let n_apps = elims
            .iter()
            .take_while(|e| matches!(e, Elim::App(_)))
            .count();
        if n_apps <= mk_pos {
            return Ok(None);
        }
        let arg = |i: usize| match elims[i] {
            Elim::App(a) => a,
            Elim::Proj(..) => unreachable!(),
        };
        let mk = self.whnf(arg(mk_pos))?;
        let Val::Neutral {
            head: Head::Const(ConstKind::Quot, mkn, _),
            spine: mks,
        } = self.st.arena.get(mk)
        else {
            return Ok(None);
        };
        if !matches!(
            self.k.env.get(mkn).map(|c| &c.kind),
            Some(Kind::Quot(QuotKind::Ctor))
        ) {
            return Ok(None);
        }
        let Some(mkargs) = self.st.arena.spine_args(mks) else {
            return Ok(None);
        };
        if mkargs.len() != 3 {
            return Ok(None);
        }
        let r = self.apply(arg(arg_pos), mkargs[2])?;
        self.apply_elims(r, &elims[mk_pos + 1..]).map(Some)
    }

    // ---- recursors ----

    fn iota(&mut self, rec: NameId, ls: LevelsId, spine: Sp) -> KResult<Option<VId>> {
        let Some(c) = self.k.env.get(rec).cloned() else {
            return Ok(None);
        };
        let Kind::Rec(info) = &c.kind else {
            return Ok(None);
        };
        let elims = self.st.arena.spine_elims(spine);
        let n_apps = elims
            .iter()
            .take_while(|e| matches!(e, Elim::App(_)))
            .count();
        let major_idx = info.major_idx();
        if n_apps <= major_idx {
            return Ok(None);
        }
        let args: Vec<VId> = elims[..n_apps]
            .iter()
            .map(|e| match e {
                Elim::App(a) => *a,
                Elim::Proj(..) => unreachable!(),
            })
            .collect();
        let mut major = args[major_idx];
        if info.k {
            major = self.to_ctor_when_k(info, major)?;
        }
        major = match self.eta_major_first(info, major)? {
            Some(expanded) => expanded,
            None => {
                let wm = self.whnf(major)?;
                match self.st.arena.get(wm) {
                    Val::Nat(n) => {
                        let x = self.k.s.nat(n).clone();
                        self.nat_to_ctor(&x)?
                    }
                    Val::Str(sid) => {
                        let e = self.str_to_ctor(sid)?;
                        self.whnf(e)?
                    }
                    _ => self.to_ctor_when_struct(info.major_induct, wm)?,
                }
            }
        };
        let Val::Neutral {
            head: Head::Const(ConstKind::Ctor, ctor, _),
            spine: ms,
        } = self.st.arena.get(major)
        else {
            return Ok(None);
        };
        let Some(rule) = info.rules.iter().find(|r| r.ctor == ctor).cloned() else {
            return Ok(None);
        };
        let Some(margs) = self.st.arena.spine_args(ms) else {
            return Ok(None);
        };
        let nfields = rule.nfields as usize;
        if nfields > margs.len() {
            return Ok(None);
        }
        if self.k.s.levels.get_list(ls).len() != c.lparams.len() {
            return Ok(None);
        }
        let env = if c.lparams.is_empty() {
            EnvId::EMPTY
        } else {
            self.st.arena.with_lsub(c.lparams.clone(), ls)
        };
        let mut r = self.eval(env, rule.rhs)?;
        r = self.apply_all(r, &args[..info.first_index_idx()])?;
        r = self.apply_all(r, &margs[margs.len() - nfields..])?;
        r = self.apply_all(r, &args[major_idx + 1..])?;
        self.apply_elims(r, &elims[n_apps..]).map(Some)
    }

    fn to_ctor_when_k(&mut self, info: &RecInfo, e: VId) -> KResult<VId> {
        let et = self.type_of(e)?;
        let ty = self.whnf(et)?;
        let Some((ti, tls, tspine)) = self.const_app(ty) else {
            return Ok(e);
        };
        if ti != info.major_induct {
            return Ok(e);
        }
        let Some(ctor) = self
            .k
            .env
            .inductive(ti)
            .and_then(|i| i.ctors.first().copied())
        else {
            return Ok(e);
        };
        let Some(targs) = self.st.arena.spine_args(tspine) else {
            return Ok(e);
        };
        let cv = self.const_value(ctor, tls)?;
        let np = (info.num_params as usize).min(targs.len());
        let new_ctor = self.apply_all(cv, &targs[..np])?;
        let nt = self.type_of(new_ctor)?;
        if !self.conv(ty, nt)? {
            return Ok(e);
        }
        Ok(new_ctor)
    }

    /// The major premise of a structure's recursor as its constructor applied to its projections, taken before the
    /// major is reduced; `None` where that does not apply. By η a structure's value is that application, so the rule
    /// fires without the major's weak head normal form, which a proof need not have
    /// (`corner-cases/eta-struct-major-no-whnf`). It applies to a structure in a universe that is never `Prop`, and to
    /// one in `Prop` whose own recursor eliminates into every universe (`And`), whose fields are all proofs.
    fn eta_major_first(&mut self, info: &RecInfo, major: VId) -> KResult<Option<VId>> {
        if !self.k.env.is_non_rec_structure(info.major_induct) {
            return Ok(None);
        }
        let major = self.force(major)?;
        if !matches!(self.st.arena.get(major), Val::Neutral { .. } | Val::Glued { .. }) || self.is_ctor_app(major) {
            return Ok(None);
        }
        let et = self.type_of(major)?;
        let ty = self.whnf(et)?;
        let Some((i, ls, tspine)) = self.const_app(ty) else {
            return Ok(None);
        };
        if i != info.major_induct {
            return Ok(None);
        }
        let tt = self.type_of(ty)?;
        let sort = self.whnf(tt)?;
        let Val::Sort(u) = self.st.arena.get(sort) else {
            return Ok(None);
        };
        let levels = &self.k.s.levels;
        if !(levels.is_never_zero(u) || (levels.is_always_zero(u) && self.k.env.eliminates_into_every_sort(info.major_induct))) {
            return Ok(None);
        }
        let Some(targs) = self.st.arena.spine_args(tspine) else {
            return Ok(None);
        };
        let ctor = self.k.env.inductive(i).unwrap().ctors[0];
        let ci = self.k.env.ctor(ctor).unwrap().clone();
        let cv = self.const_value(ctor, ls)?;
        let np = (ci.num_params as usize).min(targs.len());
        let mut r = self.apply_all(cv, &targs[..np])?;
        for f in 0..ci.num_fields {
            let p = self.proj_lazy(i, f, major)?;
            r = self.apply(r, p)?;
        }
        Ok(Some(r))
    }

    fn to_ctor_when_struct(&mut self, induct: NameId, e: VId) -> KResult<VId> {
        if !self.k.env.is_non_rec_structure(induct) || self.is_ctor_app(e) {
            return Ok(e);
        }
        let et = self.type_of(e)?;
        let ty = self.whnf(et)?;
        let Some((i, ls, tspine)) = self.const_app(ty) else {
            return Ok(e);
        };
        if i != induct {
            return Ok(e);
        }
        let tt = self.type_of(ty)?;
        let sort = self.whnf(tt)?;
        let Val::Sort(u) = self.st.arena.get(sort) else {
            return Ok(e);
        };
        if !self.k.s.levels.is_never_zero(u) {
            return Ok(e);
        }
        let Some(targs) = self.st.arena.spine_args(tspine) else {
            return Ok(e);
        };
        self.expand_eta_struct(i, ls, &targs, e)
    }

    pub(crate) fn expand_eta_struct(
        &mut self,
        i: NameId,
        ls: LevelsId,
        targs: &[VId],
        e: VId,
    ) -> KResult<VId> {
        let ctor = self.k.env.inductive(i).unwrap().ctors[0];
        let ci = self.k.env.ctor(ctor).unwrap().clone();
        let cv = self.const_value(ctor, ls)?;
        let np = (ci.num_params as usize).min(targs.len());
        let mut r = self.apply_all(cv, &targs[..np])?;
        for f in 0..ci.num_fields {
            let p = self.proj(i, f, e)?;
            r = self.apply(r, p)?;
        }
        Ok(r)
    }

    /// Whether `l` is `0` under every assignment.
    pub(crate) fn level_is_zero(&mut self, l: LevelId) -> bool {
        self.k.s.levels.is_always_zero(l)
    }
}
