//! Observational equality in [`Theory::Logos`](super::tc::Theory::Logos): the rules of
//! `crate::observational`, on the core's hash-consed terms with its own weak-head reduction,
//! inference and level algebra. They fire only for the constants the LOGOS context's certificate
//! names (`Context::observational`), which the kernel reads when it is built or moved to a later
//! context ([`Kernel::logos`], [`Kernel::follow`]).
//!
//! `Obs.{u} A a b` steps on the weak-head normal form of `A`: a function type compares pointwise;
//! `SProp` and `Prop` compare by logical equivalence, squashed for `Prop`; any other universe
//! compares types that are not propositions by their formers (two universes by their levels, two
//! function types by domains and codomains, one inductive family or one quotient former by its
//! arguments, small ones only, two different formers never); an inductive family that is not a
//! proposition compares values by their constructors, and a quotient its classes by the equivalence
//! closure of its relation. `Obs_cast.{u} A B e t` steps where that equality of types computes: along
//! one universe to `t`, along two function types to the function that carries its argument back
//! and its result forth, along one family without indices whose parameters and fields depend on no
//! earlier one to the same constructor with each field carried, and along one quotient former to
//! the class of the carried representative.

use super::env::{Kind, QuotKind};
use super::error::KResult;
use super::expr::{ExprId, Node};
use super::level::LevelId;
use super::name::NameId;
use super::tc::Kernel;

/// The certified constants' names in this kernel, read off the LOGOS context's certificate.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Certified {
    obs: NameId,
    cast: NameId,
}

impl Certified {
    pub(crate) fn of(
        ctx: &crate::context::Context,
        names: &mut super::name::Names,
    ) -> Option<Certified> {
        let certificate = ctx.observational.as_ref()?;
        let named = |kind: crate::observational::Kind| {
            certificate
                .constants
                .iter()
                .find(|(_, k)| *k == kind)
                .map(|(n, _)| n.as_str())
        };
        let (obs, cast) = (
            named(crate::observational::Kind::Obs)?,
            named(crate::observational::Kind::Cast)?,
        );
        Some(Certified {
            obs: names.str(NameId::ANON, obs),
            cast: names.str(NameId::ANON, cast),
        })
    }
}

enum TypeEquality {
    Holds,
    Fails,
    /// `SSigma domains codomains`, the domains compared at level `domain`.
    Products {
        domain: LevelId,
        domains: ExprId,
        codomains: ExprId,
    },
    Family {
        parts: Vec<ExprId>,
        dependent: bool,
    },
    /// One quotient former: the carriers' equality first, then the relations'.
    Quotient {
        parts: Vec<ExprId>,
    },
}

enum Former {
    Sort(LevelId),
    Pi(ExprId, ExprId),
    Family(NameId, super::level::LevelsId, Vec<ExprId>),
    Quotient(ExprId, Vec<ExprId>),
}

/// Which positions' types are vetted before a family's arguments compare.
#[derive(Clone, Copy)]
enum Vetting {
    Nothing,
    /// Each must be small enough for a family in this universe to store.
    SmallIn(LevelId),
}

impl Kernel {
    /// One step of a certified constant applied to all its arguments, any further arguments applied
    /// to the step's result.
    pub(crate) fn reduce_observational(&mut self, e: ExprId) -> KResult<Option<ExprId>> {
        let Some(certified) = self.logos.as_ref().and_then(|l| l.certified_obs) else {
            return Ok(None);
        };
        let Some((n, ls)) = self.s.const_head(e) else {
            return Ok(None);
        };
        if (n != certified.obs && n != certified.cast) || self.s.levels.get_list(ls).len() != 1 {
            return Ok(None);
        }
        let (_, args) = self.s.app_args(e);
        let arity = if n == certified.obs { 3 } else { 4 };
        if args.len() < arity {
            return Ok(None);
        }
        let (saturated, rest) = args.split_at(arity);
        let step = match (n == certified.obs, saturated) {
            (true, [ty, a, b]) => self.obs_equality(*ty, *a, *b)?,
            (false, [from, to, proof, value]) => self.obs_carry(*from, *to, *proof, *value)?,
            _ => None,
        };
        Ok(step.map(|head| self.s.mk_app_n(head, rest)))
    }

    fn obs_equality(&mut self, ty: ExprId, a: ExprId, b: ExprId) -> KResult<Option<ExprId>> {
        let ty = self.whnf(ty)?;
        match self.s.node(ty) {
            Node::Pi(domain, codomain) => {
                let x = self.push_local(domain, None);
                let codomain_at = self.s.instantiate1(codomain, x);
                let level = self.obs_level_of(codomain_at)?;
                let (ax, bx) = (self.s.app(a, x), self.s.app(b, x));
                let equation = self.obs(level, codomain_at, ax, bx);
                Ok(Some(self.mk_pi(&[x], equation)))
            }
            Node::Sort(l) if self.s.levels.is_sprop(l) => {
                let (forth, back) = (self.s.pi(a, b), self.s.pi(b, a));
                Ok(Some(self.obs_global_app("SAnd", &[forth, back])))
            }
            Node::Sort(l) if self.s.levels.is_always_zero(l) => {
                let (forth, back) = (self.s.pi(a, b), self.s.pi(b, a));
                let squash = |k: &mut Kernel, p: ExprId| {
                    let sbox = k.obs_const("SBox", &[LevelId::ZERO]);
                    k.s.app(sbox, p)
                };
                let (forth, back) = (squash(self, forth), squash(self, back));
                Ok(Some(self.obs_global_app("SAnd", &[forth, back])))
            }
            Node::Sort(_) => {
                let (a, b) = (self.whnf(a)?, self.whnf(b)?);
                match self.obs_type_equality(a, b)? {
                    Some(equality) => Ok(Some(self.obs_type_equality_term(equality))),
                    None => Ok(None),
                }
            }
            _ => self.obs_values(ty, a, b),
        }
    }

    fn obs_type_equality_term(&mut self, equality: TypeEquality) -> ExprId {
        match equality {
            TypeEquality::Holds => self.obs_global("STrue"),
            TypeEquality::Fails => self.obs_global("SFalse"),
            TypeEquality::Products {
                domains, codomains, ..
            } => self.obs_global_app("SSigma", &[domains, codomains]),
            TypeEquality::Family { parts, .. } | TypeEquality::Quotient { parts } => {
                self.obs_conjunction(parts)
            }
        }
    }

    /// Two weak-head-normal types that are not propositions, compared by their formers. A side
    /// whose own sort may be `Prop` is stuck.
    fn obs_type_equality(&mut self, a: ExprId, b: ExprId) -> KResult<Option<TypeEquality>> {
        let (Some(left), Some(right)) = (self.obs_former(a), self.obs_former(b)) else {
            return Ok(None);
        };
        for side in [a, b] {
            if !self.obs_never_prop(side)? {
                return Ok(None);
            }
        }
        match (left, right) {
            (Former::Sort(s), Former::Sort(t)) => Ok(if self.s.levels.equiv(s, t) {
                Some(TypeEquality::Holds)
            } else if self.obs_never_equal(s, t) {
                Some(TypeEquality::Fails)
            } else {
                None
            }),
            (Former::Pi(d, c), Former::Pi(e, k)) => self.obs_products(d, c, e, k),
            (Former::Family(i, li, xs), Former::Family(j, lj, ys)) => {
                if i != j {
                    return Ok(Some(TypeEquality::Fails));
                }
                if !self.s.levels.equiv_lists(li, lj) {
                    return Ok(None);
                }
                let universe = self.get_sort_level(a)?;
                let head = self.s.cnst(i, li);
                let arity = self.infer(head, true)?;
                Ok(self
                    .obs_componentwise(arity, &xs, &ys, Vetting::SmallIn(universe))?
                    .map(|(parts, dependent)| TypeEquality::Family { parts, dependent }))
            }
            (Former::Quotient(h, xs), Former::Quotient(k, ys)) => {
                let (Some((m, lm)), Some((n, ln))) = (self.s.const_head(h), self.s.const_head(k))
                else {
                    return Ok(None);
                };
                if m != n {
                    return Ok(Some(TypeEquality::Fails));
                }
                if !self.s.levels.equiv_lists(lm, ln) || xs.len() != 2 || ys.len() != 2 {
                    return Ok(None);
                }
                let arity = self.infer(h, true)?;
                // Both arguments of a quotient that is not a proposition are small by `Quot`'s own
                // signature, so none is vetted.
                Ok(self
                    .obs_componentwise(arity, &xs, &ys, Vetting::Nothing)?
                    .map(|(parts, _)| TypeEquality::Quotient { parts }))
            }
            _ => Ok(Some(TypeEquality::Fails)),
        }
    }

    fn obs_former(&mut self, t: ExprId) -> Option<Former> {
        match self.s.node(t) {
            Node::Sort(l) => Some(Former::Sort(l)),
            Node::Pi(d, b) => Some(Former::Pi(d, b)),
            _ => {
                let (head, args) = self.s.app_args(t);
                let Node::Const(n, ls) = self.s.node(head) else {
                    return None;
                };
                match &self.env.get(n)?.kind {
                    Kind::Induct(_) => Some(Former::Family(n, ls, args.to_vec())),
                    Kind::Quot(QuotKind::Type) => Some(Former::Quotient(head, args.to_vec())),
                    _ => None,
                }
            }
        }
    }

    /// Two levels equal under no assignment: one strictly below the other everywhere, or two
    /// different levels without parameters.
    fn obs_never_equal(&mut self, s: LevelId, t: LevelId) -> bool {
        let (ss, st) = (self.s.levels.succ(s), self.s.levels.succ(t));
        self.s.levels.leq(ss, t)
            || self.s.levels.leq(st, s)
            || (!self.s.levels.has_param(s)
                && !self.s.levels.has_param(t)
                && !self.s.levels.equiv(s, t))
    }

    /// `(Π D. C) ~ (Π E. K)` is `SSigma (E ~ D) (λ e. Π (x : E). C[Obs_cast E D e x] ~ K[x])`.
    fn obs_products(
        &mut self,
        d: ExprId,
        c: ExprId,
        e: ExprId,
        k: ExprId,
    ) -> KResult<Option<TypeEquality>> {
        let domain = self.obs_level_of(d)?;
        let other = self.obs_level_of(e)?;
        if !self.s.levels.equiv(domain, other) {
            return Ok(None);
        }
        let universe = self.s.sort(domain);
        let above = self.s.levels.succ(domain);
        let domains = self.obs(above, universe, e, d);
        let proof = self.push_local(domains, None);
        let x = self.push_local(e, None);
        let carried = self.obs_cast(domain, e, d, proof, x);
        let left = self.s.instantiate1(c, carried);
        let right = self.s.instantiate1(k, x);
        let codomain = self.obs_level_of(left)?;
        let other = self.obs_level_of(right)?;
        if !self.s.levels.equiv(codomain, other) {
            return Ok(None);
        }
        let universe = self.s.sort(codomain);
        let above = self.s.levels.succ(codomain);
        let equation = self.obs(above, universe, left, right);
        let codomains_equal = self.mk_pi(&[x], equation);
        let codomains = self.mk_lambda(&[proof], codomains_equal);
        Ok(Some(TypeEquality::Products {
            domain,
            domains,
            codomains,
        }))
    }

    /// Small enough to be stored in a family in `universe`: an element of a type in it, a
    /// universe below it, or a family of those over an element type.
    fn obs_small(&mut self, t: ExprId, universe: LevelId) -> KResult<bool> {
        let sort = self.get_sort_level(t)?;
        if self.s.levels.leq(sort, universe) {
            return Ok(true);
        }
        let t = self.whnf(t)?;
        match self.s.node(t) {
            Node::Sort(l) => Ok(self.s.levels.leq(l, universe)),
            Node::Pi(d, b) => {
                let domain = self.get_sort_level(d)?;
                if !self.s.levels.leq(domain, universe) {
                    return Ok(false);
                }
                let x = self.push_local(d, None);
                let body = self.s.instantiate1(b, x);
                self.obs_small(body, universe)
            }
            _ => Ok(false),
        }
    }

    fn obs_admits(&mut self, vetting: Vetting, t: ExprId) -> KResult<bool> {
        match vetting {
            Vetting::Nothing => Ok(true),
            Vetting::SmallIn(universe) => self.obs_small(t, universe),
        }
    }

    /// The equalities of `xs` and `ys` along one telescope, and whether any is heterogeneous: a
    /// position whose type has a loose variable (an earlier position, the telescope read without
    /// instantiating) compares heterogeneously.
    fn obs_componentwise(
        &mut self,
        telescope: ExprId,
        xs: &[ExprId],
        ys: &[ExprId],
        vetting: Vetting,
    ) -> KResult<Option<(Vec<ExprId>, bool)>> {
        if xs.len() != ys.len() {
            return Ok(None);
        }
        let (mut generic, mut left, mut right) = (telescope, telescope, telescope);
        let mut parts = Vec::with_capacity(xs.len());
        let mut any_dependent = false;
        for (x, y) in xs.iter().zip(ys) {
            let (Node::Pi(shape, rest), Node::Pi(lt, lb), Node::Pi(rt, rb)) =
                (self.s.node(generic), self.s.node(left), self.s.node(right))
            else {
                return Ok(None);
            };
            if !self.obs_admits(vetting, lt)? || !self.obs_admits(vetting, rt)? {
                return Ok(None);
            }
            let dependent = self.s.has_loose_bvars(shape);
            any_dependent |= dependent;
            let part = if dependent {
                match self.obs_heterogeneous(lt, rt, *x, *y)? {
                    Some(part) => part,
                    None => return Ok(None),
                }
            } else {
                let level = self.obs_level_of(lt)?;
                self.obs(level, lt, *x, *y)
            };
            parts.push(part);
            generic = rest;
            left = self.s.instantiate1(lb, *x);
            right = self.s.instantiate1(rb, *y);
        }
        if matches!(self.s.node(left), Node::Pi(..)) || matches!(self.s.node(right), Node::Pi(..)) {
            return Ok(None);
        }
        Ok(Some((parts, any_dependent)))
    }

    /// `SSigma (T ~ T') (λ e. Obs_cast T T' e x ~[T'] y)`.
    fn obs_heterogeneous(
        &mut self,
        from: ExprId,
        to: ExprId,
        x: ExprId,
        y: ExprId,
    ) -> KResult<Option<ExprId>> {
        let level = self.obs_level_of(from)?;
        let other = self.obs_level_of(to)?;
        if !self.s.levels.equiv(level, other) {
            return Ok(None);
        }
        let universe = self.s.sort(level);
        let above = self.s.levels.succ(level);
        let types_equal = self.obs(above, universe, from, to);
        let proof = self.push_local(types_equal, None);
        let carried = self.obs_cast(level, from, to, proof, x);
        let equation = self.obs(level, to, carried, y);
        let family = self.mk_lambda(&[proof], equation);
        Ok(Some(self.obs_global_app("SSigma", &[types_equal, family])))
    }

    /// Two values of a family that is not a proposition, by their constructors; two classes of a
    /// quotient that is not a proposition, by the equivalence closure of its relation.
    fn obs_values(&mut self, ty: ExprId, a: ExprId, b: ExprId) -> KResult<Option<ExprId>> {
        let Some(former) = self.obs_former(ty) else {
            return Ok(None);
        };
        if !self.obs_never_prop(ty)? {
            return Ok(None);
        }
        match former {
            Former::Quotient(_, carrier) => self.obs_classes(&carrier, a, b),
            Former::Family(family, _, arguments) => {
                let (Some(a), Some(b)) =
                    (self.obs_constructor_form(a)?, self.obs_constructor_form(b)?)
                else {
                    return Ok(None);
                };
                let ((c, xs), (d, ys)) = (self.s.app_args(a), self.s.app_args(b));
                let (Some(left), Some(right)) = (
                    self.obs_constructor_of(c, family),
                    self.obs_constructor_of(d, family),
                ) else {
                    return Ok(None);
                };
                if left != right {
                    return Ok(Some(self.obs_global("SFalse")));
                }
                let parameters = match &self.env.get(family).map(|c| c.kind.clone()) {
                    Some(Kind::Induct(info)) => info.num_params as usize,
                    _ => return Ok(None),
                };
                if arguments.len() < parameters || xs.len() < parameters || xs.len() != ys.len() {
                    return Ok(None);
                }
                let mut telescope = self.infer(c, true)?;
                for parameter in &arguments[..parameters] {
                    let Node::Pi(_, rest) = self.s.node(telescope) else {
                        return Ok(None);
                    };
                    telescope = self.s.instantiate1(rest, *parameter);
                }
                Ok(self
                    .obs_componentwise(
                        telescope,
                        &xs[parameters..],
                        &ys[parameters..],
                        Vetting::Nothing,
                    )?
                    .map(|(parts, _)| self.obs_conjunction(parts)))
            }
            Former::Sort(_) | Former::Pi(..) => Ok(None),
        }
    }

    /// `Quot_mk A r x ~ Quot_mk A r y` is `SBox (EqvGen A r x y)`.
    fn obs_classes(&mut self, carrier: &[ExprId], a: ExprId, b: ExprId) -> KResult<Option<ExprId>> {
        let [carrier, relation] = carrier else {
            return Ok(None);
        };
        let (a, b) = (self.whnf(a)?, self.whnf(b)?);
        let ((c, xs), (d, ys)) = (self.s.app_args(a), self.s.app_args(b));
        if !self.obs_is_class(c) || !self.obs_is_class(d) {
            return Ok(None);
        }
        let ([_, _, x], [_, _, y]) = (xs.as_slice(), ys.as_slice()) else {
            return Ok(None);
        };
        let level = self.obs_level_of(*carrier)?;
        let closure = self.obs_const("EqvGen", &[level]);
        let closure = self.s.mk_app_n(closure, &[*carrier, *relation, *x, *y]);
        let sbox = self.obs_const("SBox", &[LevelId::ZERO]);
        Ok(Some(self.s.app(sbox, closure)))
    }

    fn obs_is_class(&self, head: ExprId) -> bool {
        matches!(self.s.node(head), Node::Const(n, _) if self.quot_kind(n) == Some(QuotKind::Ctor))
    }

    /// The constructor `head` names, when it is one of `family`'s.
    fn obs_constructor_of(&self, head: ExprId, family: NameId) -> Option<NameId> {
        let Node::Const(n, _) = self.s.node(head) else {
            return None;
        };
        match &self.env.get(n)?.kind {
            Kind::Ctor(info) if info.induct == family => Some(n),
            _ => None,
        }
    }

    /// The weak-head value of `t` as an explicit constructor application, a numeral read as one
    /// constructor layer.
    fn obs_constructor_form(&mut self, t: ExprId) -> KResult<Option<ExprId>> {
        let v = self.whnf(t)?;
        if let Some(c) = self.int_constructor_form(v) {
            return Ok(Some(c));
        }
        if let Node::NatLit(n) = self.s.node(v) {
            let n = self.s.nat(n).clone();
            if !self.reads_nat_literal(&n) {
                return Ok(None);
            }
            if n.is_zero() {
                return Ok(Some(self.obs_global("Zero")));
            }
            let pred = self.s.nat_lit(n.sub(&super::nat::Nat::from_u64(1)));
            return Ok(Some(self.obs_global_app("Succ", &[pred])));
        }
        let head = self.s.app_fn(v);
        let Node::Const(n, _) = self.s.node(head) else {
            return Ok(None);
        };
        Ok(matches!(self.env.get(n).map(|c| &c.kind), Some(Kind::Ctor(_))).then_some(v))
    }

    /// `Obs_cast.{u} A B e t`, where `A ~ B` computes.
    fn obs_carry(
        &mut self,
        from: ExprId,
        to: ExprId,
        proof: ExprId,
        value: ExprId,
    ) -> KResult<Option<ExprId>> {
        let (from, to) = (self.whnf(from)?, self.whnf(to)?);
        let Some(equality) = self.obs_type_equality(from, to)? else {
            return Ok(None);
        };
        match equality {
            TypeEquality::Holds => Ok(Some(value)),
            TypeEquality::Products {
                domain,
                domains,
                codomains,
            } => {
                let (Node::Pi(d, c), Node::Pi(e, k)) = (self.s.node(from), self.s.node(to)) else {
                    return Ok(None);
                };
                let x = self.push_local(e, None);
                let first = self.obs_global_app("SSigma_fst", &[domains, codomains, proof]);
                let back = self.obs_cast(domain, e, d, first, x);
                let left = self.s.instantiate1(c, back);
                let right = self.s.instantiate1(k, x);
                let codomain = self.obs_level_of(left)?;
                let other = self.obs_level_of(right)?;
                if !self.s.levels.equiv(codomain, other) {
                    return Ok(None);
                }
                let second = self.obs_global_app("SSigma_snd", &[domains, codomains, proof, x]);
                let image = self.s.app(value, back);
                let result = self.obs_cast(codomain, left, right, second, image);
                Ok(Some(self.mk_lambda(&[x], result)))
            }
            TypeEquality::Family {
                parts,
                dependent: false,
            } => self.obs_carry_value(from, to, &parts, proof, value),
            TypeEquality::Quotient { parts } => {
                let (Some(Former::Quotient(_, xs)), Some(Former::Quotient(_, ys))) =
                    (self.obs_former(from), self.obs_former(to))
                else {
                    return Ok(None);
                };
                let value = self.whnf(value)?;
                let (constructor, arguments) = self.s.app_args(value);
                let [_, _, x] = arguments.as_slice() else {
                    return Ok(None);
                };
                if !self.obs_is_class(constructor) {
                    return Ok(None);
                }
                let carriers = match self.obs_components(&parts, proof).first() {
                    Some(c) => *c,
                    None => return Ok(None),
                };
                let level = self.obs_level_of(xs[0])?;
                let representative = self.obs_cast(level, xs[0], ys[0], carriers, *x);
                Ok(Some(
                    self.s
                        .mk_app_n(constructor, &[ys[0], ys[1], representative]),
                ))
            }
            TypeEquality::Fails | TypeEquality::Family { .. } => Ok(None),
        }
    }

    /// `Obs_cast (I p̄) (I q̄) e (C p̄ x̄)` is `C q̄ (Obs_cast T₀[p̄] T₀[q̄] h₀ x₀) …`, each `hₖ` moving `Tₖ`'s
    /// parameters from `p̄` to `q̄` one `Obs_transp` at a time along `e`'s components.
    fn obs_carry_value(
        &mut self,
        from: ExprId,
        to: ExprId,
        parts: &[ExprId],
        proof: ExprId,
        value: ExprId,
    ) -> KResult<Option<ExprId>> {
        let (Some(Former::Family(family, levels, ps)), Some(Former::Family(_, _, qs))) =
            (self.obs_former(from), self.obs_former(to))
        else {
            return Ok(None);
        };
        let indices = match self.env.get(family).map(|c| &c.kind) {
            Some(Kind::Induct(info)) => info.num_indices,
            _ => return Ok(None),
        };
        if indices != 0 || ps.len() != parts.len() || qs.len() != ps.len() {
            return Ok(None);
        }
        let Some(value) = self.obs_constructor_form(value)? else {
            return Ok(None);
        };
        let (constructor, arguments) = self.s.app_args(value);
        if self.obs_constructor_of(constructor, family).is_none() || arguments.len() < ps.len() {
            return Ok(None);
        }
        let fields = arguments[ps.len()..].to_vec();
        let telescope = self.infer(constructor, true)?;
        if !self.obs_fields_independent(telescope, &ps) {
            return Ok(None);
        }
        let head = self.s.cnst(family, levels);
        let mut arity = self.infer(head, true)?;
        let mut parameter_types = Vec::with_capacity(ps.len());
        for p in &ps {
            let Node::Pi(domain, rest) = self.s.node(arity) else {
                return Ok(None);
            };
            parameter_types.push(domain);
            arity = self.s.instantiate1(rest, *p);
        }
        let components = self.obs_components(parts, proof);
        let mut carried = Vec::with_capacity(fields.len());
        for k in 0..fields.len() {
            let (Some(source), Some(target)) = (
                self.obs_field_type(telescope, &ps, &fields, k),
                self.obs_field_type(telescope, &qs, &fields, k),
            ) else {
                return Ok(None);
            };
            let level = self.obs_level_of(source)?;
            let other = self.obs_level_of(target)?;
            if !self.s.levels.equiv(level, other) {
                return Ok(None);
            }
            let above = self.s.levels.succ(level);
            let universe = self.s.sort(level);
            let refl = self.obs_const("Obs_refl", &[above]);
            let mut evidence = self.s.mk_app_n(refl, &[universe, source]);
            for (i, parameter_type) in parameter_types.iter().copied().enumerate() {
                let parameter = self.obs_level_of(parameter_type)?;
                let b = self.push_local(parameter_type, None);
                let hypothesis = self.obs(parameter, parameter_type, ps[i], b);
                let h = self.push_local(hypothesis, None);
                let mixed: Vec<ExprId> = qs[..i]
                    .iter()
                    .copied()
                    .chain([b])
                    .chain(ps[i + 1..].iter().copied())
                    .collect();
                let Some(moved) = self.obs_field_type(telescope, &mixed, &fields, k) else {
                    return Ok(None);
                };
                let equation = self.obs(above, universe, source, moved);
                let motive = self.mk_lambda(&[b, h], equation);
                let transp = self.obs_const("Obs_transp", &[parameter]);
                evidence = self.s.mk_app_n(
                    transp,
                    &[
                        parameter_type,
                        ps[i],
                        motive,
                        evidence,
                        qs[i],
                        components[i],
                    ],
                );
            }
            carried.push(self.obs_cast(level, source, target, evidence, fields[k]));
        }
        let target_head = self.s.app_fn(value);
        let result = self.s.mk_app_n(target_head, &qs);
        Ok(Some(self.s.mk_app_n(result, &carried)))
    }

    /// Whether no field of a constructor's telescope, instantiated at `parameters`, has a loose
    /// variable for an earlier field.
    fn obs_fields_independent(&mut self, mut telescope: ExprId, parameters: &[ExprId]) -> bool {
        for p in parameters {
            let Node::Pi(_, rest) = self.s.node(telescope) else {
                return false;
            };
            telescope = self.s.instantiate1(rest, *p);
        }
        while let Node::Pi(domain, rest) = self.s.node(telescope) {
            if self.s.has_loose_bvars(domain) {
                return false;
            }
            telescope = rest;
        }
        true
    }

    /// The type of field `k` of a constructor telescope, at `parameters` and the earlier `fields`.
    fn obs_field_type(
        &mut self,
        mut telescope: ExprId,
        parameters: &[ExprId],
        fields: &[ExprId],
        k: usize,
    ) -> Option<ExprId> {
        for value in parameters.iter().chain(&fields[..k]) {
            let Node::Pi(_, rest) = self.s.node(telescope) else {
                return None;
            };
            telescope = self.s.instantiate1(rest, *value);
        }
        let Node::Pi(domain, _) = self.s.node(telescope) else {
            return None;
        };
        Some(domain)
    }

    /// Each component of `proof : SAnd parts₀ (SAnd parts₁ …)`.
    fn obs_components(&mut self, parts: &[ExprId], proof: ExprId) -> Vec<ExprId> {
        let mut out = Vec::with_capacity(parts.len());
        let mut rest = proof;
        for (j, part) in parts.iter().enumerate() {
            if j + 1 == parts.len() {
                out.push(rest);
                break;
            }
            let tail = self.obs_conjunction(parts[j + 1..].to_vec());
            out.push(self.obs_global_app("SAnd_left", &[*part, tail, rest]));
            rest = self.obs_global_app("SAnd_right", &[*part, tail, rest]);
        }
        out
    }

    fn obs_conjunction(&mut self, mut parts: Vec<ExprId>) -> ExprId {
        let Some(last) = parts.pop() else {
            return self.obs_global("STrue");
        };
        parts.into_iter().rev().fold(last, |rest, part| {
            self.obs_global_app("SAnd", &[part, rest])
        })
    }

    /// A type's level as a universe argument (`SProp` read as `Prop`).
    fn obs_level_of(&mut self, ty: ExprId) -> KResult<LevelId> {
        let l = self.get_sort_level(ty)?;
        Ok(if self.s.levels.is_sprop(l) {
            LevelId::ZERO
        } else {
            l
        })
    }

    fn obs_never_prop(&mut self, ty: ExprId) -> KResult<bool> {
        let l = self.get_sort_level(ty)?;
        let one = self.s.levels.of_nat(1);
        Ok(self.s.levels.leq(one, l))
    }

    fn obs(&mut self, level: LevelId, ty: ExprId, a: ExprId, b: ExprId) -> ExprId {
        let obs = self.obs_const("Obs", &[level]);
        self.s.mk_app_n(obs, &[ty, a, b])
    }

    fn obs_cast(
        &mut self,
        level: LevelId,
        from: ExprId,
        to: ExprId,
        proof: ExprId,
        value: ExprId,
    ) -> ExprId {
        let cast = self.obs_const("Obs_cast", &[level]);
        self.s.mk_app_n(cast, &[from, to, proof, value])
    }

    fn obs_const(&mut self, name: &str, levels: &[LevelId]) -> ExprId {
        let n = self.s.names.str(NameId::ANON, name);
        let ls = self.s.levels.list(levels);
        self.s.cnst(n, ls)
    }

    fn obs_global(&mut self, name: &str) -> ExprId {
        self.obs_const(name, &[])
    }

    fn obs_global_app(&mut self, name: &str, args: &[ExprId]) -> ExprId {
        let head = self.obs_global(name);
        self.s.mk_app_n(head, args)
    }
}
