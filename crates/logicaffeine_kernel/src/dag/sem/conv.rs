//! Definitional equality of values: Lean's algorithm, in the classic engine's order of attempts.
//!
//! Quick structural cases and proven pairs first; the `≡ true` shortcut; `whnf_core` without
//! unfolding projections' structures; proof irrelevance; lazy δ-reduction by definitional height
//! (comparing two applications of the same definition argument by argument, skipping the
//! positions its definition cannot distinguish, before unfolding either); then constants,
//! variables and projections; full `whnf_core`; congruence of applications; η; struct-η; string
//! literals; unit-like types. A β-step is an environment extension, a δ-step a remembered
//! unfolding, and a binder is crossed by applying both closures to one fresh variable.
//!
//! Proven pairs are closed under congruence but not transitivity: Lean's definitional equality
//! is not transitive (proof irrelevance), see the classic engine's `ProvenEqual`.

use super::value::{EnvId, Head, Kind, Sp, Val, V};
use super::Sem;
use crate::dag::env::Hint;
use crate::dag::error::{FailKind, KResult, KernelFailure};
use crate::dag::level::LevelsId;
use crate::dag::name::NameId;
use crate::dag::nat::Nat;

enum LBool {
    True,
    False,
    Undef,
}

impl From<bool> for LBool {
    fn from(b: bool) -> LBool {
        if b {
            LBool::True
        } else {
            LBool::False
        }
    }
}

enum Step {
    Continue(V, V),
    Unknown(V, V),
    True,
    False(V, V),
}

#[inline]
fn key(a: V, b: V) -> (u32, u32) {
    if a <= b {
        (a.0, b.0)
    } else {
        (b.0, a.0)
    }
}

impl Sem<'_> {
    /// `t ≡ s`.
    pub fn is_def_eq(&mut self, t: V, s: V) -> KResult<bool> {
        if t == s {
            return Ok(true);
        }
        self.enter()?;
        let r = self.is_def_eq_core(t, s);
        self.leave();
        let r = r?;
        if r {
            self.note_proven(t, s);
        }
        Ok(r)
    }

    /// `t ≡ s`, if it is decided within `steps` conversion steps and the checker's resources;
    /// `None` if not. An attempt is speculation: running out — of steps, or of recursion depth,
    /// which a step does not bound — leaves it undecided, never declined, and nothing is learned
    /// from it but what it proved along the way.
    pub fn is_def_eq_within(&mut self, t: V, s: V, steps: u32) -> KResult<Option<bool>> {
        let outer = self.st.probe.replace(steps);
        let r = self.is_def_eq(t, s);
        self.st.probe = outer;
        match r {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.is_exhausted() || e.kind == FailKind::Decline => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn is_def_eq_core_guarded(&mut self, t: V, s: V) -> KResult<bool> {
        if t == s {
            return Ok(true);
        }
        self.enter()?;
        let r = self.is_def_eq_core(t, s);
        self.leave();
        r
    }

    /// Equal or proven equal.
    #[inline]
    fn known_equal(&self, a: V, b: V) -> bool {
        a == b || self.st.proven.contains(&key(a, b))
    }

    /// Equal, proven equal, or congruent: the same shape with parts pairwise equal, proven equal
    /// or congruent — a walk over what is already known, never a reduction. Closures are compared
    /// by their environments' entries being equal or proven equal, not by a walk into them: an
    /// environment can be long and its entries large, and the full algorithm (which proves such
    /// parts, and remembers them, on its way) decides those.
    fn eqv_is_equiv(&mut self, a: V, b: V) -> bool {
        if self.known_equal(a, b) {
            return true;
        }
        let r = match (self.st.arena.get(a), self.st.arena.get(b)) {
            (
                Val::Neu {
                    head: h1,
                    spine: s1,
                    ..
                },
                Val::Neu {
                    head: h2,
                    spine: s2,
                    ..
                },
            ) => self.heads_equiv(h1, h2) && self.spines_equiv(s1, s2),
            (
                Val::Lam {
                    dom: d1,
                    env: e1,
                    body: b1,
                },
                Val::Lam {
                    dom: d2,
                    env: e2,
                    body: b2,
                },
            ) => d1 == d2 && b1 == b2 && self.envs_equiv(e1, e2),
            (
                Val::Pi {
                    dom: d1,
                    env: e1,
                    body: b1,
                },
                Val::Pi {
                    dom: d2,
                    env: e2,
                    body: b2,
                },
            ) => b1 == b2 && self.eqv_is_equiv(d1, d2) && self.envs_equiv(e1, e2),
            (
                Val::PiLam {
                    dom: d1,
                    env: e1,
                    ctx: c1,
                    body: b1,
                },
                Val::PiLam {
                    dom: d2,
                    env: e2,
                    ctx: c2,
                    body: b2,
                },
            ) => b1 == b2 && c1 == c2 && self.eqv_is_equiv(d1, d2) && self.envs_equiv(e1, e2),
            _ => false,
        };
        if r {
            self.note_proven(a, b);
        }
        r
    }

    #[inline]
    fn note_proven(&mut self, a: V, b: V) {
        let k = key(a, b);
        if !self.st.proven.contains(&k) {
            self.st.proven.insert(k, 1);
        }
    }

    fn heads_equiv(&mut self, h1: super::value::HeadId, h2: super::value::HeadId) -> bool {
        if h1 == h2 {
            return true;
        }
        match (self.st.arena.head(h1), self.st.arena.head(h2)) {
            (Head::Proj { idx: i1, x: x1, .. }, Head::Proj { idx: i2, x: x2, .. }) => {
                i1 == i2 && self.eqv_is_equiv(x1, x2)
            }
            _ => false,
        }
    }

    fn spines_equiv(&mut self, mut s1: Sp, mut s2: Sp) -> bool {
        if self.st.arena.spine_len(s1) != self.st.arena.spine_len(s2) {
            return false;
        }
        while s1 != s2 {
            let (n1, n2) = (self.st.arena.spine_node(s1), self.st.arena.spine_node(s2));
            if !self.eqv_is_equiv(n1.arg, n2.arg) {
                return false;
            }
            s1 = n1.prev;
            s2 = n2.prev;
        }
        true
    }

    /// Two environments of one shape (length and universe substitution) whose entries are
    /// pairwise equal or proven equal.
    fn envs_equiv(&self, e1: EnvId, e2: EnvId) -> bool {
        e1 == e2
            || (self.st.arena.same_shape(e1, e2)
                && self
                    .st
                    .arena
                    .entries(e1)
                    .zip(self.st.arena.entries(e2))
                    .all(|(a, b)| self.known_equal(a, b)))
    }

    fn quick_is_def_eq(&mut self, t: V, s: V) -> KResult<LBool> {
        if self.eqv_is_equiv(t, s) {
            return Ok(LBool::True);
        }
        Ok(match (self.st.arena.get(t), self.st.arena.get(s)) {
            (Val::Lam { .. }, Val::Lam { .. }) => self.is_def_eq_binding(t, s, false)?.into(),
            (Val::Pi { .. } | Val::PiLam { .. }, Val::Pi { .. } | Val::PiLam { .. }) => {
                self.is_def_eq_binding(t, s, true)?.into()
            }
            (Val::Sort(a), Val::Sort(b)) => self.k.s.levels.equiv(a, b).into(),
            (Val::Nat(_) | Val::Str(_), Val::Nat(_) | Val::Str(_)) => (t == s).into(),
            _ => LBool::Undef,
        })
    }

    fn is_binder(&self, v: V, pi: bool) -> bool {
        match self.st.arena.get(v) {
            Val::Lam { .. } => !pi,
            Val::Pi { .. } | Val::PiLam { .. } => pi,
            _ => false,
        }
    }

    /// Two binder chains of the same kind: domains pairwise, then the bodies under one fresh
    /// variable per binder (typed by the second chain's domains, as Lean does).
    fn is_def_eq_binding(&mut self, mut t: V, mut s: V, pi: bool) -> KResult<bool> {
        let mut bound = 0u32;
        let r = (|| -> KResult<bool> {
            while self.is_binder(t, pi) && self.is_binder(s, pi) {
                let td = self.lam_domain(t)?;
                let sd = self.lam_domain(s)?;
                if td != sd && !self.is_def_eq(td, sd)? {
                    return Ok(false);
                }
                let x = self.bind(sd);
                bound += 1;
                t = self.instantiate(t, x)?;
                s = self.instantiate(s, x)?;
            }
            self.is_def_eq(t, s)
        })();
        for _ in 0..bound {
            self.unbind();
        }
        r
    }

    /// [`Sem::is_def_eq_args`] as a probe, before unfolding: each argument pair may take
    /// `Kernel::probe_steps` steps, conversion and δ-unfolding steps alike. Arguments that differ only deep inside can cost
    /// far more to compare than the definitions cost to unfold, and unfolding decides the pair
    /// either way. `Some` when the probe decided (every pair agrees, or one is found unequal);
    /// `None` when a pair ran out of steps (see [`Sem::decide_abandoned_probe`]). Inside a probe,
    /// arguments are compared on the probe's own budget.
    fn probe_args(&mut self, t: V, s: V) -> KResult<Option<bool>> {
        if self.st.probe.is_some() {
            return self.is_def_eq_args(t, s).map(Some);
        }
        let (Some((c, ls, _, ts)), Some((_, _, _, ss))) = (self.const_app(t), self.const_app(s))
        else {
            return Ok(Some(false));
        };
        let n = self.st.arena.spine_len(ts);
        if n != self.st.arena.spine_len(ss) {
            return Ok(Some(false));
        }
        let mask = self.ignorable_args(c, ls)?;
        let steps = self.k.probe_steps;
        let (mut x, mut y) = (ts, ss);
        let mut i = n;
        while x != Sp::NIL {
            i -= 1;
            let (nx, ny) = (self.st.arena.spine_node(x), self.st.arena.spine_node(y));
            let ignorable = i < crate::dag::relevance::MAX_TRACKED && (mask >> i) & 1 == 1;
            if !ignorable {
                match self.is_def_eq_within(nx.arg, ny.arg, steps)? {
                    Some(true) => {}
                    Some(false) => return Ok(Some(false)),
                    None => return Ok(None),
                }
            }
            x = nx.prev;
            y = ny.prev;
        }
        Ok(Some(true))
    }

    /// Two applications of one definition whose argument probe ran out of steps. Unfolding both
    /// is tried on the same budget; should that run out too, the pair is decided as the algorithm
    /// without probes decides it — the arguments compared, then both sides unfolded, with no
    /// budget. So a budget only adds bounded attempts in front of that algorithm: with a budget of
    /// zero it is that algorithm, and no budget ever costs a verdict.
    fn decide_abandoned_probe(&mut self, tn: V, sn: V) -> KResult<Step> {
        let steps = self.k.probe_steps;
        let tn2 = self.delta(tn)?;
        let sn2 = self.delta(sn)?;
        match self.is_def_eq_within(tn2, sn2, steps)? {
            Some(true) => return Ok(Step::True),
            Some(false) => return Ok(Step::False(tn2, sn2)),
            None => {}
        }
        if self.is_def_eq_args(tn, sn)? {
            return Ok(Step::True);
        }
        self.st.failure.insert(key(tn, sn), 1);
        Ok(match self.quick_is_def_eq(tn2, sn2)? {
            LBool::Undef => Step::Continue(tn2, sn2),
            LBool::True => Step::True,
            LBool::False => Step::False(tn2, sn2),
        })
    }

    /// The arguments of two applications of the same definition, last to first, skipping the
    /// positions the definition cannot distinguish.
    fn is_def_eq_args(&mut self, t: V, s: V) -> KResult<bool> {
        let (Some((c, ls, _, ts)), Some((_, _, _, ss))) = (self.const_app(t), self.const_app(s))
        else {
            return Ok(false);
        };
        let n = self.st.arena.spine_len(ts);
        if n != self.st.arena.spine_len(ss) {
            return Ok(false);
        }
        let mask = self.ignorable_args(c, ls)?;
        let (mut x, mut y) = (ts, ss);
        let mut i = n;
        while x != Sp::NIL {
            i -= 1;
            let (nx, ny) = (self.st.arena.spine_node(x), self.st.arena.spine_node(y));
            let ignorable = i < crate::dag::relevance::MAX_TRACKED && (mask >> i) & 1 == 1;
            if !ignorable && !self.is_def_eq(nx.arg, ny.arg)? {
                return Ok(false);
            }
            x = nx.prev;
            y = ny.prev;
        }
        Ok(true)
    }

    /// The argument positions of `c.{ls}` whose arguments need not be compared (see
    /// [`crate::dag::relevance`]), with the proof positions from this engine's [`Signature`] — so
    /// that no substitution-engine work (and none of its scratch terms) happens inside a value
    /// check.
    ///
    /// [`Signature`]: crate::dag::relevance::Signature
    fn ignorable_args(&mut self, c: NameId, ls: LevelsId) -> KResult<u64> {
        let Some(absent) = self.k.absent_mask(c, ls) else {
            return Ok(0);
        };
        Ok(absent | self.signature(c, ls)?.map_or(0, |s| s.proof_args()))
    }

    /// The positions of a Π telescope whose binder types are propositions.
    pub fn proof_positions(&mut self, ty: V) -> KResult<u64> {
        Ok(self.telescope_signature(ty)?.proof_args())
    }

    /// η: `t` a lambda, `s` not — `t`'s body against `s x` for a fresh `x`.
    fn try_eta_expansion_core(&mut self, t: V, s: V) -> KResult<bool> {
        if !matches!(self.st.arena.get(t), Val::Lam { .. })
            || matches!(self.st.arena.get(s), Val::Lam { .. })
        {
            return Ok(false);
        }
        let st = self.type_of(s)?;
        let w = self.whnf(st)?;
        let sdom = match self.st.arena.get(w) {
            Val::Pi { dom, .. } | Val::PiLam { dom, .. } => dom,
            _ => return Ok(false),
        };
        let tdom = self.lam_domain(t)?;
        if tdom != sdom && !self.is_def_eq(tdom, sdom)? {
            return Ok(false);
        }
        self.under(sdom, |sem, x| {
            let tb = sem.instantiate(t, x)?;
            let sx = sem.apply(s, x)?;
            sem.is_def_eq(tb, sx)
        })
    }

    /// Struct-η: `s` a full constructor application of a non-recursive structure, `t` compared to
    /// it field by field. When `t` applies the same constructor, there is nothing left to try:
    /// comparing the types compares the parameters and the projections of `t` reduce to its
    /// fields — exactly the pairs congruence (`is_def_eq_app`, which runs first) found unequal.
    /// Retrying them, in both directions, at every level of a nest of constructors would cost
    /// three to the power of its depth.
    fn try_eta_struct_core(&mut self, t: V, s: V) -> KResult<bool> {
        let Some((f, _, Kind::Ctor, sp)) = self.const_app(s) else {
            return Ok(false);
        };
        if let Some((g, _, Kind::Ctor, tsp)) = self.const_app(t) {
            if g == f && self.st.arena.spine_len(tsp) == self.st.arena.spine_len(sp) {
                return Ok(false);
            }
        }
        let Some(ci) = self.k.env.ctor(f).cloned() else {
            return Ok(false);
        };
        let args = self.st.arena.args(sp);
        if args.len() != (ci.num_params + ci.num_fields) as usize {
            return Ok(false);
        }
        if !self.k.env.is_non_rec_structure(ci.induct) {
            return Ok(false);
        }
        let tt = self.type_of(t)?;
        let st = self.type_of(s)?;
        if !self.is_def_eq(tt, st)? {
            return Ok(false);
        }
        for i in ci.num_params as usize..args.len() {
            let p = self.proj(ci.induct, (i - ci.num_params as usize) as u32, t)?;
            if !self.is_def_eq(p, args[i])? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Two neutrals with non-empty spines of equal length: heads, then arguments first to last.
    fn is_def_eq_app(&mut self, t: V, s: V) -> KResult<bool> {
        let (
            Val::Neu {
                kind: k1,
                head: h1,
                spine: s1,
            },
            Val::Neu {
                kind: k2,
                head: h2,
                spine: s2,
            },
        ) = (self.st.arena.get(t), self.st.arena.get(s))
        else {
            return Ok(false);
        };
        if s1 == Sp::NIL
            || s2 == Sp::NIL
            || self.st.arena.spine_len(s1) != self.st.arena.spine_len(s2)
        {
            return Ok(false);
        }
        let ht = self.bare_head(k1, h1);
        let hs = self.bare_head(k2, h2);
        if !self.is_def_eq(ht, hs)? {
            return Ok(false);
        }
        let proofs = self.shared_proof_args(h1, h2)?;
        let (a1, a2) = (self.st.arena.args(s1), self.st.arena.args(s2));
        for (i, (a, b)) in a1.iter().zip(a2.iter()).enumerate() {
            let proof = i < crate::dag::relevance::MAX_TRACKED && (proofs >> i) & 1 == 1;
            if !proof && !self.is_def_eq(*a, *b)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The proof positions of two applications of one constant: arguments there are proofs of
    /// propositions that agree once the earlier arguments do, so they need not be compared.
    fn shared_proof_args(
        &mut self,
        h1: super::value::HeadId,
        h2: super::value::HeadId,
    ) -> KResult<u64> {
        let (
            Head::Const {
                name: c1,
                levels: l1,
            },
            Head::Const {
                name: c2,
                levels: l2,
            },
        ) = (self.st.arena.head(h1), self.st.arena.head(h2))
        else {
            return Ok(0);
        };
        if c1 != c2 || !self.k.s.levels.equiv_lists(l1, l2) {
            return Ok(0);
        }
        Ok(self.signature(c1, l1)?.map_or(0, |s| s.proof_args()))
    }

    fn bare_head(&mut self, kind: Kind, h: super::value::HeadId) -> V {
        let head = self.st.arena.head(h);
        self.st.arena.head_value(kind, head)
    }

    /// Proof irrelevance: two proofs are equal when their propositions are. A proof and a
    /// non-proof are not (their types live in different universes), which is what comparing their
    /// types would find.
    fn is_def_eq_proof_irrel(&mut self, t: V, s: V) -> KResult<LBool> {
        if !self.is_proof(t)? {
            return Ok(LBool::Undef);
        }
        if !self.is_proof(s)? {
            return Ok(LBool::False);
        }
        let tt = self.type_of(t)?;
        let st = self.type_of(s)?;
        Ok(self.is_def_eq(tt, st)?.into())
    }

    fn is_delta(&self, v: V) -> Option<Hint> {
        let (n, _, kind, _) = self.const_app(v)?;
        if kind != Kind::Delta {
            return None;
        }
        Some(self.k.env.get(n)?.hint())
    }

    fn is_proj_head(&self, v: V) -> bool {
        matches!(
            self.st.arena.get(v),
            Val::Neu {
                kind: Kind::Proj,
                ..
            }
        )
    }

    fn try_unfold_proj_app(&mut self, v: V) -> KResult<Option<V>> {
        if !self.is_proj_head(v) {
            return Ok(None);
        }
        let v2 = self.whnf_core(v, false)?;
        Ok((v2 != v).then_some(v2))
    }

    fn delta(&mut self, v: V) -> KResult<V> {
        let u = self.unfold(v)?.expect("δ on a value that does not unfold");
        self.whnf_core(u, true)
    }

    fn lazy_delta_step(&mut self, tn: V, sn: V) -> KResult<Step> {
        self.spend()?;
        let cont = |sem: &mut Sem, tn: V, sn: V| -> KResult<Step> {
            Ok(match sem.quick_is_def_eq(tn, sn)? {
                LBool::Undef => Step::Continue(tn, sn),
                LBool::True => Step::True,
                LBool::False => Step::False(tn, sn),
            })
        };
        match (self.is_delta(tn), self.is_delta(sn)) {
            (None, None) => Ok(Step::Unknown(tn, sn)),
            (Some(_), None) => {
                if let Some(sn2) = self.try_unfold_proj_app(sn)? {
                    cont(self, tn, sn2)
                } else {
                    let tn2 = self.delta(tn)?;
                    cont(self, tn2, sn)
                }
            }
            (None, Some(_)) => {
                if let Some(tn2) = self.try_unfold_proj_app(tn)? {
                    cont(self, tn2, sn)
                } else {
                    let sn2 = self.delta(sn)?;
                    cont(self, tn, sn2)
                }
            }
            (Some(ht), Some(hs)) => {
                if ht.lt(hs) {
                    let sn2 = self.delta(sn)?;
                    cont(self, tn, sn2)
                } else if hs.lt(ht) {
                    let tn2 = self.delta(tn)?;
                    cont(self, tn2, sn)
                } else {
                    let (Some((n1, l1, _, sp1)), Some((n2, l2, _, sp2))) =
                        (self.const_app(tn), self.const_app(sn))
                    else {
                        unreachable!()
                    };
                    if sp1 != Sp::NIL
                        && sp2 != Sp::NIL
                        && n1 == n2
                        && matches!(ht, Hint::Regular(_))
                        && !self.st.failure.contains(&key(tn, sn))
                    {
                        if self.k.s.levels.equiv_lists(l1, l2) {
                            match self.probe_args(tn, sn)? {
                                Some(true) => return Ok(Step::True),
                                Some(false) => {}
                                None => return self.decide_abandoned_probe(tn, sn),
                            }
                        }
                        self.st.failure.insert(key(tn, sn), 1);
                    }
                    let tn2 = self.delta(tn)?;
                    let sn2 = self.delta(sn)?;
                    cont(self, tn2, sn2)
                }
            }
        }
    }

    fn is_nat_zero(&self, v: V) -> bool {
        match self.st.arena.get(v) {
            Val::Nat(n) => self.k.s.nat(n).is_zero(),
            _ => {
                matches!(self.const_app(v), Some((n, ls, _, Sp::NIL)) if n == self.k.wk.nat_zero && ls == LevelsId::EMPTY)
            }
        }
    }

    fn nat_succ_of(&mut self, v: V) -> Option<V> {
        match self.st.arena.get(v) {
            Val::Nat(n) => {
                let x = self.k.s.nat(n).clone();
                if x.is_zero() {
                    None
                } else {
                    Some(self.nat_value(x.sub(&Nat::from_u64(1))))
                }
            }
            _ => match self.const_app(v) {
                Some((n, _, _, sp))
                    if n == self.k.wk.nat_succ && self.st.arena.spine_len(sp) == 1 =>
                {
                    Some(self.st.arena.spine_node(sp).arg)
                }
                _ => None,
            },
        }
    }

    fn is_def_eq_offset(&mut self, t: V, s: V) -> KResult<LBool> {
        if self.is_nat_zero(t) && self.is_nat_zero(s) {
            return Ok(if self.k.reads_nat_literal(&Nat::zero()) {
                LBool::True
            } else {
                LBool::Undef
            });
        }
        if !self.k.nat_literal_rules() {
            return Ok(LBool::Undef);
        }
        match (self.nat_succ_of(t), self.nat_succ_of(s)) {
            (Some(a), Some(b)) => Ok(self.is_def_eq_core_guarded(a, b)?.into()),
            _ => Ok(LBool::Undef),
        }
    }

    fn lazy_delta_reduction(&mut self, mut tn: V, mut sn: V) -> KResult<Step> {
        loop {
            match self.is_def_eq_offset(tn, sn)? {
                LBool::True => return Ok(Step::True),
                LBool::False => return Ok(Step::False(tn, sn)),
                LBool::Undef => {}
            }
            if (!self.st.arena.is_open(tn) && !self.st.arena.is_open(sn)) || self.st.eager {
                if let Some(tn2) = self.reduce_nat(tn)? {
                    return Ok(if self.is_def_eq_core_guarded(tn2, sn)? {
                        Step::True
                    } else {
                        Step::False(tn2, sn)
                    });
                } else if let Some(sn2) = self.reduce_nat(sn)? {
                    return Ok(if self.is_def_eq_core_guarded(tn, sn2)? {
                        Step::True
                    } else {
                        Step::False(tn, sn2)
                    });
                }
            }
            match self.lazy_delta_step(tn, sn)? {
                Step::Continue(a, b) => {
                    tn = a;
                    sn = b;
                }
                r => return Ok(r),
            }
        }
    }

    fn lazy_delta_proj_reduction(&mut self, mut tn: V, mut sn: V, idx: u32) -> KResult<bool> {
        loop {
            match self.lazy_delta_step(tn, sn)? {
                Step::Continue(a, b) => {
                    tn = a;
                    sn = b;
                }
                Step::True => return Ok(true),
                Step::Unknown(a, b) | Step::False(a, b) => {
                    if let Some(tf) = self.proj_field_of(idx, a)? {
                        if let Some(sf) = self.proj_field_of(idx, b)? {
                            return self.is_def_eq_core_guarded(tf, sf);
                        }
                    }
                    return self.is_def_eq_core_guarded(a, b);
                }
            }
        }
    }

    fn try_string_lit_expansion_core(&mut self, t: V, s: V) -> KResult<LBool> {
        let Val::Str(sid) = self.st.arena.get(t) else {
            return Ok(LBool::Undef);
        };
        match self.const_app(s) {
            Some((n, ls, _, sp))
                if n == self.k.wk.string_of_list
                    && ls == LevelsId::EMPTY
                    && self.st.arena.spine_len(sp) == 1 => {}
            _ => return Ok(LBool::Undef),
        }
        let x = self.str_to_ctor(sid)?;
        Ok(self.is_def_eq_core_guarded(x, s)?.into())
    }

    fn is_def_eq_unit_like(&mut self, t: V, s: V) -> KResult<bool> {
        let tt = self.type_of(t)?;
        let tty = self.whnf(tt)?;
        let Some((i, _, _, _)) = self.const_app(tty) else {
            return Ok(false);
        };
        let Some(ind) = self.k.env.inductive(i) else {
            return Ok(false);
        };
        if ind.is_rec || ind.ctors.len() != 1 || ind.num_indices != 0 {
            return Ok(false);
        }
        let c = ind.ctors[0];
        if self.k.env.ctor(c).map(|ci| ci.num_fields) != Some(0) {
            return Ok(false);
        }
        let st = self.type_of(s)?;
        self.is_def_eq_core_guarded(tty, st)
    }

    fn is_bool_true(&self, v: V) -> bool {
        matches!(self.const_app(v), Some((n, _, _, Sp::NIL)) if n == self.k.wk.bool_true)
    }

    /// A bare head: a constant, a variable or a stuck projection with no arguments.
    fn bare(&self, v: V) -> Option<Head> {
        match self.st.arena.get(v) {
            Val::Neu {
                head,
                spine: Sp::NIL,
                ..
            } => Some(self.st.arena.head(head)),
            _ => None,
        }
    }

    /// One unit of a running probe's budget; none outside a probe. A probe spends one per
    /// conversion step and one per δ-unfolding step: either can run without bound under the other.
    #[inline]
    fn spend(&mut self) -> KResult<()> {
        if let Some(steps) = self.st.probe {
            if steps == 0 {
                return Err(KernelFailure::exhausted());
            }
            self.st.probe = Some(steps - 1);
        }
        Ok(())
    }

    fn is_def_eq_core(&mut self, t: V, s: V) -> KResult<bool> {
        self.spend()?;
        match self.quick_is_def_eq(t, s)? {
            LBool::True => return Ok(true),
            LBool::False => return Ok(false),
            LBool::Undef => {}
        }
        if (!self.st.arena.is_open(t) || self.st.eager) && self.is_bool_true(s) {
            let w = self.whnf(t)?;
            if self.is_bool_true(w) {
                return Ok(true);
            }
        }
        let tn = self.whnf_core(t, true)?;
        let sn = self.whnf_core(s, true)?;
        if tn != t || sn != s {
            match self.quick_is_def_eq(tn, sn)? {
                LBool::True => return Ok(true),
                LBool::False => return Ok(false),
                LBool::Undef => {}
            }
        }
        match self.is_def_eq_proof_irrel(tn, sn)? {
            LBool::True => return Ok(true),
            LBool::False => return Ok(false),
            LBool::Undef => {}
        }
        let (tn, sn) = match self.lazy_delta_reduction(tn, sn)? {
            Step::True => return Ok(true),
            Step::False(..) => return Ok(false),
            Step::Unknown(a, b) => (a, b),
            Step::Continue(..) => unreachable!(),
        };
        match (self.bare(tn), self.bare(sn)) {
            (
                Some(Head::Const {
                    name: tf,
                    levels: tl,
                }),
                Some(Head::Const {
                    name: sf,
                    levels: sl,
                }),
            ) => {
                if tf == sf && self.k.s.levels.equiv_lists(tl, sl) {
                    return Ok(true);
                }
            }
            (Some(Head::Local { level: a, .. }), Some(Head::Local { level: b, .. })) => {
                if a == b {
                    return Ok(true);
                }
            }
            (Some(Head::Proj { idx: ti, x: te, .. }), Some(Head::Proj { idx: si, x: se, .. })) => {
                if ti == si && self.lazy_delta_proj_reduction(te, se, ti)? {
                    return Ok(true);
                }
            }
            _ => {}
        }
        let tnn = self.whnf_core(tn, false)?;
        let snn = self.whnf_core(sn, false)?;
        if tnn != tn || snn != sn {
            return self.is_def_eq_core_guarded(tnn, snn);
        }
        if self.is_def_eq_app(tn, sn)? {
            return Ok(true);
        }
        if self.try_eta_expansion_core(tn, sn)? || self.try_eta_expansion_core(sn, tn)? {
            return Ok(true);
        }
        if self.try_eta_struct_core(tn, sn)? || self.try_eta_struct_core(sn, tn)? {
            return Ok(true);
        }
        match self.try_string_lit_expansion_core(tn, sn)? {
            LBool::True => return Ok(true),
            LBool::False => return Ok(false),
            LBool::Undef => match self.try_string_lit_expansion_core(sn, tn)? {
                LBool::True => return Ok(true),
                LBool::False => return Ok(false),
                LBool::Undef => {}
            },
        }
        if self.is_def_eq_unit_like(tn, sn)? {
            return Ok(true);
        }
        Ok(false)
    }
}
