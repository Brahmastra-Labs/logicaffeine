//! LOGOS's Calculus of Inductive Constructions in the core: the rules [`Theory::Logos`] adds to
//! the substitution checker.
//!
//! The judgments are the `Term` kernel's (`type_checker.rs`, `reduction.rs`), stated over the
//! core's hash-consed DAG:
//!
//! * a literal's type is LOGOS's carrier (`Int`, `Nat`, `Float`, `Text`, `Duration`, `Date`,
//!   `Moment`);
//! * `match d return m with cs` takes the inductive family from `d`'s type, accepts a motive that
//!   is a family over that type and its indices (or a bare type, read as the constant family),
//!   owes a case for every constructor or for exactly those `d`'s indices allow, checks each case
//!   against the constructor's fields, refuses an elimination out of `SProp` into a relevant sort
//!   and a large elimination of a family that is a proposition at some instance of its levels
//!   (unless it is a subsingleton), and has the motive applied to `d`'s indices and `d` as its
//!   type. It ι-reduces once `d` is in constructor form (a `Nat` literal is `Zero` or `Succ`, an
//!   `Int` literal `Int_ofNat` or `Int_negSucc`), and a single-case match on a proof of a K family
//!   or a value of a unit-like type reads its scrutinee as the constructor;
//! * `fix f => body` has the type read off `body`'s binders and the motive of its innermost
//!   `match`; in a checking position it is checked against the expected type instead, with `f`
//!   bound at that type. It unrolls only when the argument at the structural column the guard
//!   chose is in constructor form;
//! * conversion relates stuck `match`es, `fix`es and mutual `fix`es by congruence, an `Int`
//!   literal with its constructor form, and two instances of a cumulative inductive that differ
//!   only in irrelevant universe parameters;
//! * a term of type `A` stands where `B` is expected when `A ≤ B` (cumulative universes,
//!   covariant in a function type's codomain).

//!
//! The declaration-time checks of LOGOS's own design (the termination guard, strict positivity)
//! and the computation rules of LOGOS's builtins (integer and text operations, the validated
//! `natLe`/`natLt`, the reflection embedding) are the `Term` kernel's, applied to [`lift`]ed
//! terms through `LogosState::ctx`, and so are the certificates of the programs computed natively
//! on literals (an arithmetic operation, a `Nat` order or equality, a division program such as
//! Lean's well-founded `Nat.div._unary`): one implementation of each, so the two kernels cannot
//! disagree about them.

use super::env::Kind;
use super::error::{KResult, KernelFailure};
use super::expr::{ExprId, Lit, Node};
use super::level::LevelsId;
use super::logos::{lift, lower_open};
use super::tc::{Kernel, Theory};
use crate::context::Context;
use crate::reduction::{CertifiedProgram, ProgramShape};
use crate::term::Term;
use rustc_hash::FxHashMap as FxMap;
use std::rc::Rc;

/// What checking LOGOS's theory needs beyond the store and the environment.
#[derive(Clone)]
pub struct LogosState {
    /// The LOGOS context the kernel's declarations come from.
    pub(crate) ctx: Context,
    /// The structural column the guard chose for each fixpoint body (`None`: no column, so the
    /// fixpoint never unrolls).
    columns: FxMap<ExprId, Option<usize>>,
    /// The builtin computation rules' answers, by the application they rewrote.
    builtins: FxMap<ExprId, Option<ExprId>>,
    /// Whether `Nat` and `Int` are the canonical families, so that a literal is read as its
    /// constructor form (`Context::canonical_numerals`).
    numerals: crate::context::CanonicalNumerals,
    /// Each inductive's and constructor's irrelevant universe parameters, as a mask over its
    /// level list (see `logos_env::irrelevant_levels`).
    irrelevant: FxMap<super::name::NameId, u64>,
    /// The context's certified observational equality, by the names its rules fire for.
    pub(super) certified_obs: Option<super::logos_obs::Certified>,
    /// What the `Term` kernel's evaluator certifies each constant to compute at an arity, and how
    /// its operands are passed (`reduction::program_shape`, `reduction::certified_program`): asked
    /// once per context, `None` for a constant that computes nothing natively.
    programs: FxMap<(super::name::NameId, usize), Option<Rc<(ProgramShape, CertifiedProgram)>>>,
}

impl LogosState {
    /// A declaration is done: what its scratch region held is gone, so are the answers about it.
    pub(crate) fn end_decl(&mut self) {
        self.columns.retain(|k, _| !k.is_scratch());
        self.builtins
            .retain(|k, v| !k.is_scratch() && !v.is_some_and(ExprId::is_scratch));
    }
}

macro_rules! bail {
    ($($arg:tt)*) => {
        return Err(KernelFailure::reject(format!($($arg)*)))
    };
}

impl Kernel {
    /// A kernel for LOGOS's theory over the declarations of `ctx`.
    pub fn logos(ctx: &Context) -> Kernel {
        let mut k = Kernel::new();
        k.theory = Theory::Logos;
        k.wk = super::wellknown::WellKnown::logos(&mut k.s.names);
        let certified_obs = super::logos_obs::Certified::of(ctx, &mut k.s.names);
        k.logos = Some(Box::new(LogosState {
            ctx: ctx.clone(),
            columns: FxMap::default(),
            builtins: FxMap::default(),
            numerals: ctx.canonical_numerals(),
            irrelevant: FxMap::default(),
            certified_obs,
            programs: FxMap::default(),
        }));
        k
    }

    /// Move a LOGOS kernel to `ctx`, a later state of the lineage its declarations came from: the
    /// guard, positivity, index coverage and the builtin evaluator read the context they check in.
    pub(crate) fn follow(&mut self, ctx: &Context) {
        let certified_obs = super::logos_obs::Certified::of(ctx, &mut self.s.names);
        let state = self
            .logos
            .as_mut()
            .expect("a LOGOS kernel carries its context");
        state.ctx = ctx.clone();
        state.certified_obs = certified_obs;
        state.programs.clear();
    }

    /// Whether this kernel's `Nat` and `Int` are the canonical families its literal rules read.
    pub(crate) fn numerals(&self) -> crate::context::CanonicalNumerals {
        self.logos
            .as_ref()
            .expect("a LOGOS kernel carries its context")
            .numerals
    }

    /// Whether the `Nat` literal `n` reads as the constructor it names (`int_inductive::reads_nat_literal`):
    /// always in Lean's theory, whose `Nat` the primitive checks hold to its canonical form; in
    /// LOGOS's, `0` as `Zero` while `Zero : Nat` is `Nat`'s constructor and `n + 1` as `Succ n`
    /// while `Succ : Nat → Nat` is too.
    pub(crate) fn reads_nat_literal(&self, n: &super::nat::Nat) -> bool {
        match &self.logos {
            None => true,
            Some(state) if n.is_zero() => state.numerals.zero,
            Some(state) => state.numerals.succ,
        }
    }

    /// Whether `Succ` of a literal is the next one and the arithmetic on literals computes: the
    /// successor reading, [`Kernel::reads_nat_literal`] past `0`.
    pub(crate) fn nat_literal_rules(&self) -> bool {
        self.logos
            .as_ref()
            .map_or(true, |state| state.numerals.succ)
    }

    fn logos_ctx(&self) -> &Context {
        &self
            .logos
            .as_ref()
            .expect("a LOGOS kernel carries its context")
            .ctx
    }

    fn global(&mut self, name: &str) -> ExprId {
        let n = self.s.names.str(super::name::NameId::ANON, name);
        self.s.cnst(n, LevelsId::EMPTY)
    }

    pub(crate) fn infer_logos(&mut self, e: ExprId, infer_only: bool) -> KResult<ExprId> {
        if self.theory != Theory::Logos {
            bail!("a LOGOS construct outside LOGOS's theory");
        }
        match self.s.node(e) {
            Node::Lit(l) => {
                let carrier = match self.s.lit_value(l) {
                    Lit::Int(_) | Lit::BigInt(_) => "Int",
                    Lit::Float(_) => "Float",
                    Lit::Duration(_) => "Duration",
                    Lit::Date(_) => "Date",
                    Lit::Moment(_) => "Moment",
                };
                Ok(self.global(carrier))
            }
            Node::NatLit(_) => Ok(self.global("Nat")),
            Node::StrLit(_) => Ok(self.global("Text")),
            Node::Match(d, m, cs) => self.infer_match(d, m, cs, infer_only),
            Node::Fix(body) => self.infer_fix(e, body, infer_only),
            Node::MutualFix(ds, i) => self.infer_mutual_fix(ds, i, infer_only),
            other => unreachable!("not a LOGOS construct: {other:?}"),
        }
    }

    /// The inductive family a type is an application of, with the application's arguments.
    fn inductive_app(
        &mut self,
        ty: ExprId,
    ) -> KResult<(super::name::NameId, LevelsId, Vec<ExprId>)> {
        let w = self.whnf(ty)?;
        let (head, args) = self.s.app_args(w);
        if let Node::Const(n, ls) = self.s.node(head) {
            if let Some(c) = self.env.get(n) {
                if matches!(c.kind, Kind::Induct(_)) {
                    return Ok((n, ls, args.to_vec()));
                }
            }
        }
        bail!(
            "match on a value whose type {} is not an inductive family",
            self.show(ty)
        )
    }

    fn infer_match(
        &mut self,
        d: ExprId,
        m: ExprId,
        cs: ExprId,
        infer_only: bool,
    ) -> KResult<ExprId> {
        let dt = self.infer(d, infer_only)?;
        let (ind, ls, args) = self.inductive_app(dt)?;
        let info = match &self.env.get(ind).expect("checked above").kind {
            Kind::Induct(i) => i.clone(),
            _ => unreachable!(),
        };
        let dtw = self.whnf(dt)?;
        if info.num_indices > 0 {
            return self.infer_indexed_match(d, m, cs, ind, ls, &args, &info, dtw, infer_only);
        }
        let mt = self.infer(m, infer_only)?;
        let mtw = self.whnf(mt)?;
        let motive = match self.s.node(mtw) {
            // Inferring only, the type is the motive at the discriminant, as for the indexed
            // `match`; the motive is held to the discriminant's type where the `match` is checked.
            Node::Pi(..) if infer_only => m,
            Node::Pi(dom, cod) => {
                if !self.is_def_eq(dom, dtw)? {
                    bail!(
                        "motive parameter {} doesn't match discriminant type {}",
                        self.show(dom),
                        self.show(dtw)
                    );
                }
                let x = self.push_local(dom, None);
                let codw = self.s.instantiate1(cod, x);
                let codw = self.whnf(codw)?;
                if !matches!(self.s.node(codw), Node::Sort(_)) {
                    bail!("motive type codomain is not a sort");
                }
                m
            }
            Node::Sort(_) => {
                let body = self.s.lift_loose(m, 0, 1);
                self.s.lam(dtw, body)
            }
            _ => bail!("motive {} is not a function or type", self.show(m)),
        };
        let cases = self.s.seq_items(cs);
        if cases.len() != info.ctors.len() {
            bail!(
                "wrong number of cases: expected {}, found {}",
                info.ctors.len(),
                cases.len()
            );
        }
        let params = &args[..info.num_params as usize];
        for (case, ctor) in cases.iter().zip(info.ctors.iter()) {
            if infer_only {
                continue;
            }
            let expected = self.case_type(*ctor, ls, params, motive)?;
            if !self.check_fix_against(*case, expected)? {
                let ct = self.infer(*case, false)?;
                if !self.conforms(ct, expected)? {
                    bail!(
                        "case for '{}' has type {} where {} is expected",
                        self.s.names.display(*ctor),
                        self.show(ct),
                        self.show(expected)
                    );
                }
            }
        }
        let applied = self.s.app(motive, d);
        let ret = self.whnf_core(applied, false)?;
        if !infer_only {
            self.check_elimination(ind, dtw, ret)?;
        }
        Ok(ret)
    }

    /// `Π fields. motive (C params fields)` for constructor `ctor` of a family at `params`.
    fn case_type(
        &mut self,
        ctor: super::name::NameId,
        ls: LevelsId,
        params: &[ExprId],
        motive: ExprId,
    ) -> KResult<ExprId> {
        let c = self
            .env
            .get(ctor)
            .expect("an inductive's constructor is declared")
            .clone();
        let us: Vec<_> = self.s.levels.get_list(ls).to_vec();
        let mut ty = self.s.instantiate_lparams(c.ty, &c.lparams, &us);
        for p in params {
            let w = self.whnf(ty)?;
            let Node::Pi(_, b) = self.s.node(w) else {
                bail!("constructor type has fewer parameters than its family")
            };
            ty = self.s.instantiate1(b, *p);
        }
        let head = self.s.cnst(ctor, ls);
        let head = self.s.mk_app_n(head, params);
        let mut fields = Vec::new();
        loop {
            let w = self.whnf(ty)?;
            match self.s.node(w) {
                Node::Pi(a, b) => {
                    let x = self.push_local(a, None);
                    fields.push(x);
                    ty = self.s.instantiate1(b, x);
                }
                _ => break,
            }
        }
        let value = self.s.mk_app_n(head, &fields);
        let w = self.whnf(ty)?;
        let (_, result_args) = self.s.app_args(w);
        let indices: Vec<ExprId> = result_args.iter().skip(params.len()).copied().collect();
        let body = self.s.mk_app_n(motive, &indices);
        let body = self.s.app(body, value);
        let body = self.whnf_core(body, false)?;
        Ok(self.mk_pi(&fields, body))
    }

    /// `match` on an indexed family: the motive abstracts the indices and then the scrutinee; a
    /// case is owed for every constructor, or exactly for the constructors the scrutinee's indices
    /// allow (`type_checker::constructors_the_indices_allow`); each case is the motive at the
    /// constructor's result indices and value; the result is the motive at the scrutinee's indices
    /// and the scrutinee.
    #[allow(clippy::too_many_arguments)]
    fn infer_indexed_match(
        &mut self,
        d: ExprId,
        m: ExprId,
        cs: ExprId,
        ind: super::name::NameId,
        ls: LevelsId,
        args: &[ExprId],
        info: &super::env::InductiveInfo,
        dtw: ExprId,
        infer_only: bool,
    ) -> KResult<ExprId> {
        let np = info.num_params as usize;
        let (params, indices) = args.split_at(np);
        if !infer_only {
            self.validate_indexed_motive(ind, ls, params, indices.len(), m)?;
        }
        let cases = self.s.seq_items(cs);
        let owed: Vec<super::name::NameId> = if cases.len() == info.ctors.len() {
            info.ctors.to_vec()
        } else {
            let allowed = self.allowed_constructors(ind, np, args);
            if cases.len() != allowed.len() {
                bail!(
                    "wrong number of cases: expected {}, found {}",
                    allowed.len(),
                    cases.len()
                );
            }
            allowed.iter().map(|i| info.ctors[*i]).collect()
        };
        for (case, ctor) in cases.iter().zip(owed.iter()) {
            if infer_only {
                continue;
            }
            let expected = self.case_type(*ctor, ls, params, m)?;
            if !self.check_fix_against(*case, expected)? {
                let ct = self.infer(*case, false)?;
                if !self.conforms(ct, expected)? {
                    bail!(
                        "case for '{}' has type {} where {} is expected",
                        self.s.names.display(*ctor),
                        self.show(ct),
                        self.show(expected)
                    );
                }
            }
        }
        let ret = self.s.mk_app_n(m, indices);
        let ret = self.s.app(ret, d);
        let ret = self.whnf_core(ret, false)?;
        if !infer_only {
            let rs = self.infer(ret, true)?;
            let rs = self.whnf(rs)?;
            if !matches!(self.s.node(rs), Node::Sort(_)) {
                bail!("indexed match on '{}' has a non-type result: the motive is not a family into a sort", self.s.names.display(ind));
            }
            self.check_elimination(ind, dtw, ret)?;
        }
        Ok(ret)
    }

    /// The motive's telescope is the family's index telescope (at the scrutinee's parameters), then
    /// the family at those indices, then a sort.
    fn validate_indexed_motive(
        &mut self,
        ind: super::name::NameId,
        ls: LevelsId,
        params: &[ExprId],
        num_indices: usize,
        m: ExprId,
    ) -> KResult<()> {
        let c = self.env.get(ind).expect("an inductive").clone();
        let us: Vec<_> = self.s.levels.get_list(ls).to_vec();
        let mut arity = self.s.instantiate_lparams(c.ty, &c.lparams, &us);
        for p in params {
            let w = self.whnf(arity)?;
            let Node::Pi(_, b) = self.s.node(w) else {
                bail!("inductive has fewer parameters than its discriminant")
            };
            arity = self.s.instantiate1(b, *p);
        }
        let head = self.s.cnst(ind, ls);
        let mut family = self.s.mk_app_n(head, params);
        let mut mt = self.infer(m, false)?;
        for _ in 0..num_indices {
            let aw = self.whnf(arity)?;
            let Node::Pi(ad, ab) = self.s.node(aw) else {
                bail!("inductive index telescope is shorter than its discriminant")
            };
            let mw = self.whnf(mt)?;
            let Node::Pi(md, mb) = self.s.node(mw) else {
                bail!("indexed motive lacks an index binder")
            };
            if !self.is_def_eq(md, ad)? {
                bail!("indexed motive index domain differs from the inductive telescope");
            }
            let x = self.push_local(ad, None);
            arity = self.s.instantiate1(ab, x);
            mt = self.s.instantiate1(mb, x);
            family = self.s.app(family, x);
        }
        let aw = self.whnf(arity)?;
        if !matches!(self.s.node(aw), Node::Sort(_)) {
            bail!("discriminant does not instantiate the full inductive index telescope");
        }
        let mw = self.whnf(mt)?;
        let Node::Pi(md, mb) = self.s.node(mw) else {
            bail!("indexed motive lacks its major-premise binder")
        };
        if !self.is_def_eq(md, family)? {
            bail!("indexed motive major domain differs from the generic indexed family");
        }
        let x = self.push_local(family, None);
        let cod = self.s.instantiate1(mb, x);
        let cod = self.whnf(cod)?;
        if !matches!(self.s.node(cod), Node::Sort(_)) {
            bail!("indexed motive must return a sort after its major-premise binder");
        }
        Ok(())
    }

    /// The positions of the constructors whose declared result indices can equal the indices among
    /// `args`, the family's type arguments with its `np` parameters first.
    fn allowed_constructors(
        &mut self,
        ind: super::name::NameId,
        np: usize,
        args: &[ExprId],
    ) -> Vec<usize> {
        let name = self.s.names.display(ind);
        let lifted: Vec<Term> = args.iter().map(|i| lift(&self.s, *i)).collect();
        crate::type_checker::constructors_the_indices_allow(self.logos_ctx(), &name, np, &lifted)
    }

    /// The elimination restrictions of `match` (`type_checker::check_elimination`'s rule): an
    /// `SProp` family eliminates only into `SProp`, and a family that is a proposition at some
    /// instance of its levels eliminates into a larger sort only when it is a subsingleton or, for
    /// a family whose level may be `Prop`, the result's level is at most its own.
    fn check_elimination(
        &mut self,
        ind: super::name::NameId,
        disc_ty: ExprId,
        ret: ExprId,
    ) -> KResult<()> {
        let ds = self.infer(disc_ty, true)?;
        let ds = self.whnf(ds)?;
        let rs = self.infer(ret, true)?;
        let rs = self.whnf(rs)?;
        let (Node::Sort(dl), Node::Sort(rl)) = (self.s.node(ds), self.s.node(rs)) else {
            bail!("a match whose scrutinee or result is not a type");
        };
        let levels = &mut self.s.levels;
        if levels.is_sprop(dl) && !levels.is_sprop(rl) {
            bail!(
                "elimination of the definitionally-irrelevant proposition '{}' into a relevant sort is not allowed",
                self.s.names.display(ind)
            );
        }
        let zero = super::level::LevelId::ZERO;
        let one = levels.succ(zero);
        let large = if levels.is_sprop(dl) {
            false
        } else if levels.equiv(dl, zero) {
            !levels.equiv(rl, zero) && !levels.is_sprop(rl)
        } else if !levels.leq(one, dl) {
            !levels.leq(rl, dl)
        } else {
            false
        };
        if large {
            let name = self.s.names.display(ind);
            if !crate::type_checker::is_subsingleton_prop(self.logos_ctx(), &name).unwrap_or(false)
            {
                bail!(
                    "large elimination of proposition '{name}' into a larger sort is not allowed"
                );
            }
        }
        Ok(())
    }

    /// A fixpoint in a checking position — a definition's value, an argument, a `let` value, a
    /// `match` case — checked against the type that position expects: bound at that type, a
    /// terminating fixpoint's body must have it (`type_checker::check_type`'s rule). It types a recursion whose result type no `match` motive states,
    /// such as a `match` applied to further arguments. `false` hands the value to inference;
    /// `expected` is a type, as every caller has already established.
    pub(crate) fn check_fix_against(&mut self, value: ExprId, expected: ExprId) -> KResult<bool> {
        let Node::Fix(body) = self.s.node(value) else {
            return Ok(false);
        };
        let Term::Fix { name, body: b } = lift(&self.s, value) else {
            unreachable!()
        };
        if crate::termination::check_termination(self.logos_ctx(), &name, &b).is_err() {
            return Ok(false);
        }
        let x = self.push_local(expected, None);
        let opened = self.s.instantiate1(body, x);
        match self.infer(opened, false) {
            Ok(got) => self.conforms(got, expected),
            Err(_) => Ok(false),
        }
    }

    /// The type of `fix f => body`, read off `body` as the `Term` kernel reads it; checked, the
    /// guard admits the recursion and `body`, with `f` bound at that type, has it.
    fn infer_fix(&mut self, fix: ExprId, body: ExprId, infer_only: bool) -> KResult<ExprId> {
        let ty = self.fix_type(body)?;
        if !infer_only {
            let Term::Fix { name, body: b } = lift(&self.s, fix) else {
                unreachable!()
            };
            crate::termination::check_termination(self.logos_ctx(), &name, &b)
                .map_err(|e| KernelFailure::reject(e.to_string()))?;
            let x = self.push_local(ty, None);
            let opened = self.s.instantiate1(body, x);
            let got = self.infer(opened, false)?;
            if !self.is_def_eq(got, ty)? {
                bail!(
                    "a fixpoint's body has type {} where its binders and motive read {}",
                    self.show(got),
                    self.show(ty)
                );
            }
        }
        Ok(ty)
    }

    fn infer_mutual_fix(&mut self, ds: ExprId, index: u32, infer_only: bool) -> KResult<ExprId> {
        let bodies = self.s.seq_items(ds);
        if bodies.is_empty() || index as usize >= bodies.len() {
            bail!("mutual fixpoint with an empty block or out-of-range index");
        }
        // A member's type is read off its own binders and motive with the block's names unbound,
        // as the `Term` kernel reads it: a sibling mentioned there is a loose variable, and refused.
        let mut types = Vec::with_capacity(bodies.len());
        let n = bodies.len() as u32;
        for b in bodies.iter() {
            if self.s.loose_bvar_range(*b) > n {
                bail!("a mutual fixpoint member mentions a variable outside its block");
            }
            types.push(self.fix_type(*b)?);
        }
        if !infer_only {
            let whole = self.s.intern(Node::MutualFix(ds, index));
            let Term::MutualFix { defs, .. } = lift(&self.s, whole) else {
                unreachable!()
            };
            crate::termination::check_termination_mutual(self.logos_ctx(), &defs)
                .map_err(|e| KernelFailure::reject(e.to_string()))?;
            let xs: Vec<ExprId> = types.iter().map(|t| self.push_local(*t, None)).collect();
            for b in bodies.iter() {
                let opened = self.s.instantiate_rev(*b, &xs);
                self.infer(opened, false)?;
            }
        }
        Ok(types[index as usize])
    }

    /// `infer_fix_type_structurally`: a fixpoint's type from its body's binders and the motive of
    /// the innermost `match` (applied to the discriminant's indices and the discriminant), read
    /// through an application spine, so a convoy `(match v … ) w` has the motive's type applied to
    /// `w`. The fixpoint's own binders stay loose in `body`, so a type that mentions one is refused.
    fn fix_type(&mut self, body: ExprId) -> KResult<ExprId> {
        let mut locals = Vec::new();
        let mut cur = body;
        while let Node::Lam(a, b) = self.s.node(cur) {
            let a = self.s.instantiate_rev(a, &locals);
            self.infer(a, true)?;
            let x = self.push_local(a, None);
            locals.push(x);
            cur = b;
        }
        let inner = self.s.instantiate_rev(cur, &locals);
        let (head, args) = self.s.app_args(inner);
        let result = match self.s.node(head) {
            Node::Match(d, m, _) => {
                let mut ty = self.match_result_type(d, m)?;
                for a in args.iter() {
                    let w = self.whnf(ty)?;
                    let Node::Pi(_, b) = self.s.node(w) else {
                        bail!(
                            "a convoy applies a match whose type {} is not a function",
                            self.show(w)
                        )
                    };
                    ty = self.s.instantiate1(b, *a);
                }
                ty
            }
            _ => self.infer(inner, true)?,
        };
        Ok(self.mk_pi(&locals, result))
    }

    /// A `match`'s type as a fixpoint's type is read off it: a motive that is itself a type, or
    /// the motive applied to the scrutinee's indices and the scrutinee.
    fn match_result_type(&mut self, d: ExprId, m: ExprId) -> KResult<ExprId> {
        let mt = self.infer(m, true)?;
        let mtw = self.whnf(mt)?;
        if matches!(self.s.node(mtw), Node::Sort(_)) {
            return Ok(m);
        }
        let dt = self.infer(d, true)?;
        let indices = match self.inductive_app(dt) {
            Ok((ind, _, args)) => match &self.env.get(ind).expect("an inductive").kind {
                Kind::Induct(i) => args[i.num_params as usize..].to_vec(),
                _ => Vec::new(),
            },
            Err(_) => Vec::new(),
        };
        let applied = self.s.mk_app_n(m, &indices);
        let applied = self.s.app(applied, d);
        self.whnf_core(applied, false)
    }

    /// ι for `match`: the case of the discriminant's constructor, applied to its fields.
    pub(crate) fn reduce_match(&mut self, e: ExprId) -> KResult<Option<ExprId>> {
        if self.theory != Theory::Logos {
            return Ok(None);
        }
        let Node::Match(d, m, cs) = self.s.node(e) else {
            unreachable!()
        };
        let dw = self.whnf(d)?;
        let cases = self.s.seq_items(cs);
        let Some((ctor, fields)) = self.constructor_view(dw)? else {
            if cases.len() == 1 {
                if let Some(k) = self.k_constructor_form(dw)? {
                    return Ok(Some(self.s.intern(Node::Match(k, m, cs))));
                }
            }
            return Ok(None);
        };
        let Some(pos) = self.case_position(dw, ctor, cases.len())? else {
            return Ok(None);
        };
        let Some(case) = cases.get(pos).copied() else {
            return Ok(None);
        };
        Ok(Some(self.s.mk_app_n(case, &fields)))
    }

    /// Which case a constructor's value takes: its constructor's position, or, in a case list
    /// pruned to the constructors the scrutinee's indices allow, its position among those.
    fn case_position(
        &mut self,
        v: ExprId,
        ctor: super::name::NameId,
        num_cases: usize,
    ) -> KResult<Option<usize>> {
        let Some(c) = self.env.get(ctor).cloned() else {
            return Ok(None);
        };
        let Kind::Ctor(ci) = &c.kind else {
            return Ok(None);
        };
        let Some(ic) = self.env.get(ci.induct).cloned() else {
            return Ok(None);
        };
        let Kind::Induct(info) = &ic.kind else {
            return Ok(None);
        };
        if num_cases == info.ctors.len() {
            return Ok(Some(ci.cidx as usize));
        }
        // The scrutinee's indices: the constructor's declared result at its actual arguments.
        let (head, actual) = self.s.app_args(v);
        let Node::Const(_, ls) = self.s.node(head) else {
            return Ok(None);
        };
        let us: Vec<_> = self.s.levels.get_list(ls).to_vec();
        let mut ty = self.s.instantiate_lparams(c.ty, &c.lparams, &us);
        for a in actual.iter() {
            let w = self.whnf(ty)?;
            let Node::Pi(_, b) = self.s.node(w) else {
                return Ok(None);
            };
            ty = self.s.instantiate1(b, *a);
        }
        let w = self.whnf(ty)?;
        let (_, result) = self.s.app_args(w);
        let np = info.num_params as usize;
        let allowed = self.allowed_constructors(ci.induct, np, &result);
        if allowed.len() != num_cases {
            return Ok(None);
        }
        Ok(allowed.iter().position(|i| *i == ci.cidx as usize))
    }

    /// The K-like rule of LOGOS's ι: a proof of a K family (`Context::k_constructor`, a
    /// one-constructor proposition whose constructor fixes its indices) or a value of a unit-like
    /// type (`Context::unit_constructor`) is its constructor at the parameters, when that
    /// constructor has the proof's type.
    fn k_constructor_form(&mut self, proof: ExprId) -> KResult<Option<ExprId>> {
        let pt = match self.infer(proof, true) {
            Ok(t) => self.whnf(t)?,
            Err(_) => return Ok(None),
        };
        let (head, args) = self.s.app_args(pt);
        let Node::Const(ind, ls) = self.s.node(head) else {
            return Ok(None);
        };
        let name = self.s.names.display(ind);
        let ctx = self.logos_ctx();
        let unit = ctx.unit_constructor(&name).map(str::to_string);
        let Some(ctor) = ctx
            .k_constructor(&name)
            .map(str::to_string)
            .or(unit.clone())
        else {
            return Ok(None);
        };
        let np = ctx.inductive_num_params(&name);
        if unit.is_none() {
            let sort = self.infer(pt, true)?;
            let sort = self.whnf(sort)?;
            let Node::Sort(l) = self.s.node(sort) else {
                return Ok(None);
            };
            if self.s.levels.is_sprop(l) || l != super::level::LevelId::ZERO {
                return Ok(None);
            }
        }
        let c = self.s.names.str(super::name::NameId::ANON, &ctor);
        let value = self.s.cnst(c, ls);
        let value = self.s.mk_app_n(value, &args[..np.min(args.len())]);
        let vt = match self.infer(value, true) {
            Ok(t) => t,
            Err(_) => return Ok(None),
        };
        Ok(self.is_def_eq(pt, vt)?.then_some(value))
    }

    /// A value in constructor form: the constructor and its fields (parameters dropped). A `Nat`
    /// literal is `Zero` or `Succ` of the literal below it.
    fn constructor_view(
        &mut self,
        v: ExprId,
    ) -> KResult<Option<(super::name::NameId, Vec<ExprId>)>> {
        if let Some(c) = self.int_constructor_form(v) {
            return self.constructor_view(c);
        }
        if let Node::NatLit(n) = self.s.node(v) {
            let n = self.s.nat(n).clone();
            if !self.reads_nat_literal(&n) {
                return Ok(None);
            }
            if n.is_zero() {
                let z = self.s.names.str(super::name::NameId::ANON, "Zero");
                return Ok(Some((z, Vec::new())));
            }
            let pred = self.s.nat_lit(n.sub(&super::nat::Nat::from_u64(1)));
            let succ = self.s.names.str(super::name::NameId::ANON, "Succ");
            return Ok(Some((succ, vec![pred])));
        }
        let (head, args) = self.s.app_args(v);
        let Node::Const(n, _) = self.s.node(head) else {
            return Ok(None);
        };
        let Some(c) = self.env.get(n) else {
            return Ok(None);
        };
        let Kind::Ctor(ci) = &c.kind else {
            return Ok(None);
        };
        let np = ci.num_params as usize;
        if args.len() < np {
            return Ok(None);
        }
        Ok(Some((n, args[np..].to_vec())))
    }

    /// `whnf_core` of a stuck application under LOGOS's ι, out of line: the Lean theory's
    /// `whnf_core` reaches it only through a test of [`Theory`] and keeps its own recursor step
    /// inline.
    #[inline(never)]
    pub(crate) fn whnf_core_logos_app(&mut self, e: ExprId, cheap_proj: bool) -> KResult<ExprId> {
        match self.reduce_logos_app(e)? {
            Some(r) => self.whnf_core(r, cheap_proj),
            None => Ok(e),
        }
    }

    /// The application rules of LOGOS's ι: the quotient's (`Quot_lift`/`Quot_ind` of a
    /// `Quot_mk`), and `fix` unrolling — an application of a fixpoint whose structural argument
    /// is in constructor form becomes its body with the fixpoint substituted, applied to the
    /// arguments.
    pub(crate) fn reduce_logos_app(&mut self, e: ExprId) -> KResult<Option<ExprId>> {
        if self.env.quot_init {
            if let Some(r) = self.quot_reduce_rec(e)? {
                return Ok(Some(r));
            }
        }
        if let Some(r) = self.reduce_observational(e)? {
            return Ok(Some(r));
        }
        let (head, args) = self.s.app_args(e);
        match self.s.node(head) {
            Node::Fix(body) => {
                let Some(col) = self.structural_column(head)? else {
                    return Ok(None);
                };
                let Some(arg) = args.get(col).copied() else {
                    return Ok(None);
                };
                let aw = self.whnf(arg)?;
                if self.constructor_view(aw)?.is_none() {
                    return Ok(None);
                }
                let unrolled = self.s.instantiate1(body, head);
                Ok(Some(self.s.mk_app_n(unrolled, &args)))
            }
            Node::MutualFix(ds, index) => {
                // A mutual block unrolls on its first argument, as the `Term` kernel's does.
                let Some(arg) = args.first().copied() else {
                    return Ok(None);
                };
                let aw = self.whnf(arg)?;
                if self.constructor_view(aw)?.is_none() {
                    return Ok(None);
                }
                let bodies = self.s.seq_items(ds);
                let members: Vec<ExprId> = (0..bodies.len() as u32)
                    .map(|i| self.s.intern(Node::MutualFix(ds, i)))
                    .collect();
                let unrolled = self.s.instantiate_rev(bodies[index as usize], &members);
                Ok(Some(self.s.mk_app_n(unrolled, &args)))
            }
            _ => Ok(None),
        }
    }

    /// The column the guard recursed on for this fixpoint.
    fn structural_column(&mut self, fix: ExprId) -> KResult<Option<usize>> {
        if let Some(c) = self.logos.as_ref().and_then(|l| l.columns.get(&fix)) {
            return Ok(*c);
        }
        let Term::Fix { body, .. } = lift(&self.s, fix) else {
            unreachable!("a single fixpoint")
        };
        let col = crate::termination::structural_columns(self.logos_ctx(), &body)
            .first()
            .copied();
        self.logos
            .as_mut()
            .expect("LOGOS state")
            .columns
            .insert(fix, col);
        Ok(col)
    }

    /// A builtin computation rule of LOGOS's (an integer or text operation, the validated `Nat`
    /// order, the reflection embedding), applied by the `Term` kernel's reducer to the lifted
    /// application.
    pub(crate) fn reduce_builtin(&mut self, e: ExprId) -> KResult<Option<ExprId>> {
        let Some((n, _)) = self.s.const_head(e) else {
            return Ok(None);
        };
        if self.s.app_num_args(e) == 0 {
            return Ok(None);
        }
        let name = self.s.names.display(n);
        let ctx = self.logos_ctx();
        if !(crate::reduction::computes_by_name(&name)
            || crate::reduction::accelerates_nat_order(ctx, &name))
        {
            return Ok(None);
        }
        if let Some(r) = self.logos.as_ref().and_then(|l| l.builtins.get(&e)) {
            return Ok(*r);
        }
        let term = lift(&self.s, e);
        let reduced = crate::reduction::normalize(self.logos_ctx(), &term);
        let r = if reduced == term {
            None
        } else {
            Some(lower_open(&mut self.s, &reduced).map_err(|e| {
                KernelFailure::reject(format!("a builtin's answer is not a kernel term: {e:?}"))
            })?)
        };
        self.logos
            .as_mut()
            .expect("LOGOS state")
            .builtins
            .insert(e, r);
        Ok(r)
    }

    /// A program the `Term` kernel's evaluator certifies — an arithmetic operation, a `Nat` order
    /// or equality, a division program on a pair (`reduction::certified_program`) — applied to
    /// operands whose weak-head normal forms are literals is its value, computed by the evaluator's
    /// own arithmetic (`reduction::certified_application`). The operands are this kernel's to read;
    /// the certificate is the evaluator's, so the two kernels compute exactly the same programs. A
    /// program is certified only over the canonical `Nat` (`reduction::program_shape`), so the
    /// literals read here are its numerals.
    pub(crate) fn reduce_certified_program(&mut self, e: ExprId) -> KResult<Option<ExprId>> {
        let Some((name, levels)) = self.s.const_head(e) else {
            return Ok(None);
        };
        let arity = self.s.app_num_args(e);
        if levels != LevelsId::EMPTY || !(1..=2).contains(&arity) {
            return Ok(None);
        }
        let Some(certified) = self.certified_program_of(name, arity) else {
            return Ok(None);
        };
        let (shape, program) = &*certified;
        let (_, args) = self.s.app_args(e);
        let (x, y) = match shape {
            ProgramShape::Binary => (args[0], args[1]),
            ProgramShape::Pair { pair, parameters } => {
                let parameters = *parameters;
                let observed = self.whnf(args[0])?;
                let (constructor, fields) = self.s.app_args(observed);
                let Node::Const(c, _) = self.s.node(constructor) else {
                    return Ok(None);
                };
                if fields.len() != parameters + 2 || self.s.names.display(c) != *pair {
                    return Ok(None);
                }
                (fields[parameters], fields[parameters + 1])
            }
        };
        let Some((x, y)) = self.nat_bin_args(x, y)? else {
            return Ok(None);
        };
        let Some(value) =
            crate::reduction::certified_application(program, &x.to_bigint(), &y.to_bigint())
        else {
            return Ok(None);
        };
        lower_open(&mut self.s, &value).map(Some).map_err(|e| {
            KernelFailure::reject(format!(
                "a certified program's value is not a kernel term: {e:?}"
            ))
        })
    }

    /// What `name` applied to `arity` operands is certified to compute, and how its operands are
    /// passed: a division program takes one pair, every other program two literals.
    fn certified_program_of(
        &mut self,
        name: super::name::NameId,
        arity: usize,
    ) -> Option<Rc<(ProgramShape, CertifiedProgram)>> {
        if let Some(known) = self
            .logos
            .as_ref()
            .and_then(|l| l.programs.get(&(name, arity)))
        {
            return known.clone();
        }
        let spelled = self.s.names.display(name);
        let ctx = self.logos_ctx();
        let known = crate::reduction::program_shape(ctx, &spelled, arity).and_then(|shape| {
            let program = crate::reduction::certified_program(ctx, &spelled)?;
            let division = matches!(program, CertifiedProgram::Division { .. });
            (division == matches!(shape, ProgramShape::Pair { .. }))
                .then(|| Rc::new((shape, program)))
        });
        self.logos
            .as_mut()
            .expect("LOGOS state")
            .programs
            .insert((name, arity), known.clone());
        known
    }

    pub(crate) fn record_irrelevant_levels(&mut self, name: super::name::NameId, mask: u64) {
        if mask != 0 {
            self.logos
                .as_mut()
                .expect("LOGOS state")
                .irrelevant
                .insert(name, mask);
        }
    }

    /// Cumulative inductive types: two applications of one inductive or constructor, its
    /// parameters supplied, at instances that differ only in irrelevant universe parameters,
    /// are equal when their arguments are.
    fn cumulative_instances(&mut self, t: ExprId, s: ExprId) -> KResult<bool> {
        let (Some((n, tl)), Some((m, sl))) = (self.s.const_head(t), self.s.const_head(s)) else {
            return Ok(false);
        };
        if n != m || tl == sl {
            return Ok(false);
        }
        let Some(mask) = self
            .logos
            .as_ref()
            .and_then(|l| l.irrelevant.get(&n))
            .copied()
        else {
            return Ok(false);
        };
        let num_params = match self.env.get(n).map(|c| &c.kind) {
            Some(Kind::Induct(info)) => info.num_params,
            Some(Kind::Ctor(ci)) => ci.num_params,
            _ => return Ok(false),
        };
        let (a, b) = (
            self.s.levels.get_list(tl).to_vec(),
            self.s.levels.get_list(sl).to_vec(),
        );
        if a.len() != b.len() {
            return Ok(false);
        }
        for (i, (u, v)) in a.iter().zip(&b).enumerate() {
            let irrelevant = i < 64 && (mask >> i) & 1 == 1;
            if !irrelevant && !self.s.levels.equiv(*u, *v) {
                return Ok(false);
            }
        }
        let (_, targs) = self.s.app_args(t);
        let (_, sargs) = self.s.app_args(s);
        if targs.len() != sargs.len() || targs.len() < num_params as usize {
            return Ok(false);
        }
        for (x, y) in targs.iter().zip(&sargs) {
            if !self.is_def_eq(*x, *y)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// An `Int` literal's constructor form, `Int_ofNat n` or `Int_negSucc n`, when `Int` is the
    /// canonical family: a literal is a compact spelling of a constructor value, not a second
    /// carrier.
    pub(super) fn int_constructor_form(&mut self, e: ExprId) -> Option<ExprId> {
        use logicaffeine_base::BigInt;
        let Node::Lit(l) = self.s.node(e) else {
            return None;
        };
        let value = match self.s.lit_value(l) {
            Lit::Int(i) => BigInt::from_i64(*i),
            Lit::BigInt(b) => b.clone(),
            _ => return None,
        };
        if !self.logos.as_ref()?.numerals.int {
            return None;
        }
        let (ctor, field) = if value.is_negative() {
            ("Int_negSucc", value.abs().sub(&BigInt::from_i64(1)))
        } else {
            ("Int_ofNat", value)
        };
        let c = self.global(ctor);
        let n = self.s.nat_lit(super::nat::Nat::from_bigint(&field));
        Some(self.s.app(c, n))
    }

    /// LOGOS's conversion rules beyond the shared algorithm, on two weak-head normal forms it
    /// could not relate: an `Int` literal is its constructor form, and a stuck `match`, a `fix`
    /// and a mutual `fix` are related by congruence. `false` is "not shown equal", never a
    /// refutation.
    /// Structure η as LOGOS states it (`type_checker::try_structure_eta`): `s`, a fully applied
    /// constructor of a structure the context registers, against `t`, not headed by that constructor,
    /// field by field against the structure's projection applied to the parameters and `t`. LOGOS
    /// writes a projection as a definition by `match`, never as a projection node.
    pub(crate) fn logos_eta_struct(&mut self, t: ExprId, s: ExprId) -> KResult<bool> {
        let Some((f, ls)) = self.s.const_head(s) else {
            return Ok(false);
        };
        if self.s.const_head(t).is_some_and(|(g, _)| g == f) {
            return Ok(false);
        }
        let constructor = super::logos::component(&self.s, f);
        let Some(state) = self.logos.as_ref() else {
            return Ok(false);
        };
        let Some((_, info)) = state.ctx.struct_of_constructor(&constructor) else {
            return Ok(false);
        };
        let (num_params, projections) = (info.num_params, info.projections.clone());
        let (_, args) = self.s.app_args(s);
        if args.len() != num_params + projections.len() {
            return Ok(false);
        }
        for (i, projection) in projections.iter().enumerate() {
            let p = self.s.names.str(super::name::NameId::ANON, projection);
            if self.env.get(p).is_none() {
                return Ok(false);
            }
            let head = self.s.cnst(p, ls);
            let applied = self.s.mk_app_n(head, &args[..num_params]);
            let applied = self.s.app(applied, t);
            if !self.is_def_eq(args[num_params + i], applied)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(crate) fn logos_def_eq(&mut self, t: ExprId, s: ExprId) -> KResult<bool> {
        if self.cumulative_instances(t, s)? {
            return Ok(true);
        }
        if let Some(tc) = self.int_constructor_form(t) {
            let sc = self.int_constructor_form(s).unwrap_or(s);
            return self.is_def_eq(tc, sc);
        }
        if let Some(sc) = self.int_constructor_form(s) {
            return self.is_def_eq(t, sc);
        }
        match (self.s.node(t), self.s.node(s)) {
            (Node::Match(d1, m1, c1), Node::Match(d2, m2, c2)) => {
                let (cs1, cs2) = (self.s.seq_items(c1), self.s.seq_items(c2));
                if cs1.len() != cs2.len() || !self.is_def_eq(d1, d2)? || !self.is_def_eq(m1, m2)? {
                    return Ok(false);
                }
                for (a, b) in cs1.iter().zip(&cs2) {
                    if !self.is_def_eq(*a, *b)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            (Node::Fix(b1), Node::Fix(b2)) => {
                let ty = self.infer(t, true)?;
                let x = self.push_local(ty, None);
                let (o1, o2) = (self.s.instantiate1(b1, x), self.s.instantiate1(b2, x));
                self.is_def_eq(o1, o2)
            }
            (Node::MutualFix(ds1, i1), Node::MutualFix(ds2, i2)) if i1 == i2 => {
                let (bs1, bs2) = (self.s.seq_items(ds1), self.s.seq_items(ds2));
                if bs1.len() != bs2.len() {
                    return Ok(false);
                }
                let mut xs = Vec::with_capacity(bs1.len());
                for b in bs1.iter() {
                    let ty = self.fix_type(*b)?;
                    xs.push(self.push_local(ty, None));
                }
                for (a, b) in bs1.iter().zip(&bs2) {
                    let (oa, ob) = (
                        self.s.instantiate_rev(*a, &xs),
                        self.s.instantiate_rev(*b, &xs),
                    );
                    if !self.is_def_eq(oa, ob)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// `a ≤ b`: cumulative universes, covariant in a function type's codomain, otherwise
    /// definitional equality.
    pub(crate) fn is_subtype(&mut self, a: ExprId, b: ExprId) -> KResult<bool> {
        if a == b {
            return Ok(true);
        }
        let (aw, bw) = (self.whnf(a)?, self.whnf(b)?);
        match (self.s.node(aw), self.s.node(bw)) {
            (Node::Sort(u), Node::Sort(v)) => Ok(self.s.levels.leq(u, v)),
            (Node::Pi(d1, c1), Node::Pi(d2, c2)) => {
                if !self.is_def_eq(d1, d2)? {
                    return Ok(false);
                }
                let x = self.push_local(d2, None);
                let (c1, c2) = (self.s.instantiate1(c1, x), self.s.instantiate1(c2, x));
                self.is_subtype(c1, c2)
            }
            _ => self.is_def_eq(aw, bw),
        }
    }
}
