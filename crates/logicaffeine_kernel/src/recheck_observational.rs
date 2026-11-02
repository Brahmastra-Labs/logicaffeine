//! The re-checker's own reading of observational equality (`observational`): the same rules on
//! de Bruijn terms, with this kernel's weak-head reduction, typing and substitution.
//!
//! `Obs.{u} A a b` steps on the weak-head normal form of `A`: a function type compares pointwise;
//! `SProp` and `Prop` compare by logical equivalence, squashed for `Prop`; any other universe
//! compares types that are not propositions by their formers; an inductive family that holds no proposition
//! compares values by their constructors. `Obs_cast.{u} A B e t` steps where that equality of types
//! computes: along one universe to `t`, along two function types to the function that carries its
//! argument back and its result forth, along one family without indices to the same constructor
//! with each field carried.

use super::*;
use crate::observational::Kind;

pub(super) fn reduce(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> Option<Db> {
    genv.observational.as_ref()?;
    let (head, args) = spine_ref(t);
    let Db::Const { name, levels } = head else {
        return None;
    };
    if levels.len() != 1 {
        return None;
    }
    match (crate::observational::kind(genv, name)?, args.as_slice()) {
        (Kind::Obs, [ty, a, b]) => equality(genv, lctx, ty, a, b),
        (Kind::Cast, [from, to, proof, value]) => carry(genv, lctx, from, to, proof, value),
        _ => None,
    }
}

fn equality(genv: &Context, lctx: &dyn LocalTypes, ty: &Db, a: &Db, b: &Db) -> Option<Db> {
    match whnf_in(genv, lctx, ty) {
        Db::Pi(domain, codomain) => {
            pointwise(genv, lctx, domain.into_db(), codomain.into_db(), a, b)
        }
        Db::Sort(Universe::SProp) => Some(apply(global("SAnd"), [arrow(a, b), arrow(b, a)])),
        Db::Sort(level) if level.equiv(&Universe::Prop) => {
            let squash = |p: Db| {
                apply(
                    Db::Const {
                        name: "SBox".to_string(),
                        levels: vec![Universe::Prop],
                    },
                    [p],
                )
            };
            Some(apply(
                global("SAnd"),
                [squash(arrow(a, b)), squash(arrow(b, a))],
            ))
        }
        Db::Sort(_) => {
            let (a, b) = (whnf_in(genv, lctx, a), whnf_in(genv, lctx, b));
            type_equality(genv, lctx, &a, &b).map(TypeEquality::into_db)
        }
        ty => values(genv, lctx, &ty, a, b),
    }
}

/// `Π D. Obs.{v} C (a↑ #0) (b↑ #0)`, `v` the codomain's level under the new binder.
fn pointwise(
    genv: &Context,
    lctx: &dyn LocalTypes,
    domain: Db,
    codomain: Db,
    a: &Db,
    b: &Db,
) -> Option<Db> {
    let mut inside = extend_local(lctx);
    inside.push(domain.clone());
    let level = level_of(genv, &inside, &codomain)?;
    let at = |side: &Db| apply(shift(side, 1, 0), [Db::Var(0)]);
    Some(Db::Pi(
        DbBox::new(domain),
        DbBox::new(obs(level, codomain, at(a), at(b))),
    ))
}

enum TypeEquality {
    Holds,
    Fails,
    /// One quotient former: the carriers' equality first, then the relations'.
    Quotient {
        parts: Vec<Db>,
    },
    /// `SSigma domains codomains`: the domains' equality at level `domain`, and the family of the
    /// codomains' equalities over it.
    Products {
        domain: Universe,
        domains: Db,
        codomains: Db,
    },
    Family {
        parts: Vec<Db>,
        dependent: bool,
    },
}

impl TypeEquality {
    fn into_db(self) -> Db {
        match self {
            TypeEquality::Holds => global("STrue"),
            TypeEquality::Fails => global("SFalse"),
            TypeEquality::Products {
                domains, codomains, ..
            } => apply(global("SSigma"), [domains, codomains]),
            TypeEquality::Family { parts, .. } | TypeEquality::Quotient { parts } => {
                conjunction(parts)
            }
        }
    }
}

/// Equality of two weak-head-normal types that are not propositions, by their formers. A side
/// whose own sort may be `Prop` is stuck: a proposition seen as a type is compared by `Prop`'s
/// rule, never by its former.
fn type_equality(genv: &Context, lctx: &dyn LocalTypes, a: &Db, b: &Db) -> Option<TypeEquality> {
    let (left, right) = (former(genv, a)?, former(genv, b)?);
    for side in [a, b] {
        if !infer_sort(genv, lctx, side).ok()?.is_never_prop() {
            return None;
        }
    }
    match (left, right) {
        (Former::Sort(s), Former::Sort(t)) => {
            if s.equiv(t) {
                Some(TypeEquality::Holds)
            } else if never_equal(s, t) {
                Some(TypeEquality::Fails)
            } else {
                None
            }
        }
        (Former::Pi(d, c), Former::Pi(e, k)) => products(genv, lctx, (d, c), (e, k)),
        (Former::Family(i, li, xs), Former::Family(j, lj, ys)) => {
            if i != j {
                return Some(TypeEquality::Fails);
            }
            families(genv, lctx, a, (i, li, &xs), (lj, &ys))
        }
        (Former::Quotient(h, xs), Former::Quotient(k, ys)) => {
            if !same_quotient(h, k)? {
                return Some(TypeEquality::Fails);
            }
            quotients(genv, lctx, h, &xs, &ys)
        }
        _ => Some(TypeEquality::Fails),
    }
}

enum Former<'a> {
    Sort(&'a Universe),
    Pi(&'a Db, &'a Db),
    Family(&'a str, &'a [Universe], Vec<&'a Db>),
    Quotient(&'a Db, Vec<&'a Db>),
}

fn former<'a>(genv: &Context, t: &'a Db) -> Option<Former<'a>> {
    match t {
        Db::Sort(level) => Some(Former::Sort(level)),
        Db::Pi(domain, codomain) => Some(Former::Pi(domain, codomain)),
        _ => {
            let (head, args) = spine_ref(t);
            match head {
                Db::Global(name) if genv.is_inductive(name) => {
                    Some(Former::Family(name, &[], args))
                }
                Db::Const { name, levels } if genv.is_inductive(name) => {
                    Some(Former::Family(name, levels, args))
                }
                _ if quotient_part(genv, head) == Some(QuotientPart::Former) => {
                    Some(Former::Quotient(head, args))
                }
                _ => None,
            }
        }
    }
}

#[derive(PartialEq)]
enum QuotientPart {
    Former,
    Class,
}

fn quotient_part(genv: &Context, head: &Db) -> Option<QuotientPart> {
    use crate::quotient::{Kind, PreludeKind};
    match head {
        Db::Global(name) => match crate::quotient::prelude_kind(genv, name)? {
            PreludeKind::Type => Some(QuotientPart::Former),
            PreludeKind::Constructor => Some(QuotientPart::Class),
            _ => None,
        },
        Db::Const { name, levels } => match crate::quotient::kind(genv, name, levels)? {
            Kind::Type => Some(QuotientPart::Former),
            Kind::Constructor => Some(QuotientPart::Class),
            _ => None,
        },
        _ => None,
    }
}

/// `Quot_mk A r x ~ Quot_mk A r y` is `SBox (EqvGen A r x y)`.
fn classes(
    genv: &Context,
    lctx: &dyn LocalTypes,
    ty: &Db,
    carrier: &[&Db],
    a: &Db,
    b: &Db,
) -> Option<Db> {
    if carrier.len() != 2 || !infer_sort(genv, lctx, ty).ok()?.is_never_prop() {
        return None;
    }
    let (a, b) = (whnf_in(genv, lctx, a), whnf_in(genv, lctx, b));
    let ((c, xs), (d, ys)) = (spine_ref(&a), spine_ref(&b));
    if quotient_part(genv, c) != Some(QuotientPart::Class)
        || quotient_part(genv, d) != Some(QuotientPart::Class)
    {
        return None;
    }
    let ([_, _, x], [_, _, y]) = (xs.as_slice(), ys.as_slice()) else {
        return None;
    };
    let level = level_of(genv, lctx, carrier[0])?;
    let closure = apply(
        Db::Const {
            name: "EqvGen".to_string(),
            levels: vec![level],
        },
        [
            carrier[0].clone(),
            carrier[1].clone(),
            (*x).clone(),
            (*y).clone(),
        ],
    );
    Some(apply(
        Db::Const {
            name: "SBox".to_string(),
            levels: vec![Universe::Prop],
        },
        [closure],
    ))
}

/// Two quotients by one former, compared as a family's arguments.
fn quotients(
    genv: &Context,
    lctx: &dyn LocalTypes,
    head: &Db,
    xs: &[&Db],
    ys: &[&Db],
) -> Option<TypeEquality> {
    if xs.len() != 2 || ys.len() != 2 {
        return None;
    }
    let level = infer_sort(genv, lctx, xs[0]).ok()?;
    let arity = match head {
        Db::Global(name) => {
            let header = genv.get_global(name)?;
            let [u] = crate::term::free_level_vars(header).try_into().ok()?;
            let instantiated =
                crate::term::instantiate_universes(header, &[(u, level)].into_iter().collect());
            to_db(&instantiated, &mut Vec::new()).ok()?
        }
        _ => infer(genv, lctx, head).ok()?,
    };
    // Both arguments of a quotient that is not a proposition are small by `Quot`'s own signature.
    let (parts, _) = componentwise(genv, lctx, arity, xs, ys, &|_| true)?;
    Some(TypeEquality::Quotient { parts })
}

fn same_quotient(h: &Db, k: &Db) -> Option<bool> {
    match (h, k) {
        (Db::Global(m), Db::Global(n)) => Some(m == n),
        (Db::Const { name: m, levels: l }, Db::Const { name: n, levels: k }) => {
            if m != n {
                return Some(false);
            }
            (l.len() == k.len() && l.iter().zip(k).all(|(a, b)| a.equiv(b))).then_some(true)
        }
        _ => Some(false),
    }
}

/// Two levels equal under no instantiation: one strictly below the other everywhere, or two
/// different levels without variables.
fn never_equal(s: &Universe, t: &Universe) -> bool {
    fn ground(level: &Universe) -> bool {
        match level {
            Universe::SProp | Universe::Prop | Universe::Type(_) => true,
            Universe::Var(_) => false,
            Universe::Succ(l) => ground(l),
            Universe::Max(a, b) | Universe::IMax(a, b) => ground(a) && ground(b),
        }
    }
    s.succ().is_subtype_of(t)
        || t.succ().is_subtype_of(s)
        || (ground(s) && ground(t) && !s.equiv(t))
}

/// `(Π D. C) ~ (Π E. K)` is `SSigma (E ~ D) (λ e. Π E. C[Obs_cast E D e #0] ~ K)`.
fn products(
    genv: &Context,
    lctx: &dyn LocalTypes,
    (d, c): (&Db, &Db),
    (e, k): (&Db, &Db),
) -> Option<TypeEquality> {
    let domain = level_of(genv, lctx, d)?;
    if !domain.equiv(&level_of(genv, lctx, e)?) {
        return None;
    }
    let domains = obs(
        domain.succ(),
        Db::Sort(domain.clone()),
        e.clone(),
        d.clone(),
    );
    // Under `λ e` and `Π (x : E)`: `#1` is the proof, `#0` the argument.
    let carried = apply(
        cast_at(domain.clone()),
        [shift(e, 2, 0), shift(d, 2, 0), Db::Var(1), Db::Var(0)],
    );
    let left = beta_open(&shift(c, 2, 1), &carried);
    let right = shift(k, 1, 1);
    let mut inside = extend_local(lctx);
    inside.push(domains.clone());
    inside.push(shift(e, 1, 0));
    let codomain = level_of(genv, &inside, &left)?;
    if !codomain.equiv(&level_of(genv, &inside, &right)?) {
        return None;
    }
    let codomains_equal = Db::Pi(
        DbBox::new(shift(e, 1, 0)),
        DbBox::new(obs(codomain.succ(), Db::Sort(codomain), left, right)),
    );
    let codomains = Db::Lam(DbBox::new(domains.clone()), DbBox::new(codomains_equal));
    Some(TypeEquality::Products {
        domain,
        domains,
        codomains,
    })
}

/// `I p̄ ~ I q̄`: the arguments' equalities, when each argument is small enough for the family's
/// universe to store; otherwise stuck.
fn families(
    genv: &Context,
    lctx: &dyn LocalTypes,
    a: &Db,
    (family, levels, xs): (&str, &[Universe], &[&Db]),
    (other_levels, ys): (&[Universe], &[&Db]),
) -> Option<TypeEquality> {
    if levels.len() != other_levels.len()
        || levels.iter().zip(other_levels).any(|(l, m)| !l.equiv(m))
    {
        return None;
    }
    let universe = infer_sort(genv, lctx, a).ok()?;
    let arity = infer(genv, lctx, &family_head(family, levels)).ok()?;
    let small = |t: &Db| small(genv, lctx, t, &universe);
    let (parts, dependent) = componentwise(genv, lctx, arity, xs, ys, &small)?;
    Some(TypeEquality::Family { parts, dependent })
}

fn family_head(family: &str, levels: &[Universe]) -> Db {
    if levels.is_empty() {
        global(family)
    } else {
        Db::Const {
            name: family.to_string(),
            levels: levels.to_vec(),
        }
    }
}

/// An element of a type in `universe`, a universe below it, or a family of those over an element
/// type.
fn small(genv: &Context, lctx: &dyn LocalTypes, t: &Db, universe: &Universe) -> bool {
    if infer_sort(genv, lctx, t).is_ok_and(|sort| sort.is_subtype_of(universe)) {
        return true;
    }
    match whnf_in(genv, lctx, t) {
        Db::Sort(level) => level.is_subtype_of(universe),
        Db::Pi(domain, codomain) => {
            if !infer_sort(genv, lctx, &domain).is_ok_and(|sort| sort.is_subtype_of(universe)) {
                return false;
            }
            let mut inside = extend_local(lctx);
            inside.push(domain.into_db());
            small(genv, &inside, &codomain, universe)
        }
        _ => false,
    }
}

/// Equality of two values of a family that is not a proposition, by their constructors.
fn values(genv: &Context, lctx: &dyn LocalTypes, ty: &Db, a: &Db, b: &Db) -> Option<Db> {
    if let Some(Former::Quotient(_, carrier)) = former(genv, ty) {
        return classes(genv, lctx, ty, &carrier, a, b);
    }
    let (family, arguments) = spine_ref(ty);
    let (Db::Global(family) | Db::Const { name: family, .. }) = family else {
        return None;
    };
    if !genv.is_inductive(family) || !infer_sort(genv, lctx, ty).ok()?.is_never_prop() {
        return None;
    }
    let (a, b) = (
        constructor_form(genv, lctx, a)?,
        constructor_form(genv, lctx, b)?,
    );
    let ((left, xs), (right, ys)) = (spine_ref(&a), spine_ref(&b));
    let (Db::Global(c) | Db::Const { name: c, .. }) = left else {
        return None;
    };
    let (Db::Global(d) | Db::Const { name: d, .. }) = right else {
        return None;
    };
    if genv.constructor_inductive(c) != Some(family)
        || genv.constructor_inductive(d) != Some(family)
    {
        return None;
    }
    if c != d {
        return Some(global("SFalse"));
    }
    let parameters = genv.inductive_num_params(family);
    if arguments.len() < parameters || xs.len() < parameters || xs.len() != ys.len() {
        return None;
    }
    let telescope = peel(infer(genv, lctx, left).ok()?, &arguments[..parameters])?;
    let (parts, _) = componentwise(
        genv,
        lctx,
        telescope,
        &xs[parameters..],
        &ys[parameters..],
        &|_| true,
    )?;
    Some(conjunction(parts))
}

/// The weak-head value of `t` as an explicit constructor application, a numeral read as one layer.
fn constructor_form(genv: &Context, lctx: &dyn LocalTypes, t: &Db) -> Option<Db> {
    let value = whnf_in(genv, lctx, t);
    if let Some(layer) = db_int_constructor(genv, &value) {
        return Some(layer);
    }
    if let Db::Lit(Literal::Nat(n)) = &value {
        return crate::int_inductive::reads_nat_literal(genv, n).then(|| db_nat_peano_step(&value));
    }
    matches!(spine_head(&value), Db::Global(n) | Db::Const { name: n, .. } if genv.is_constructor(n)).then_some(value)
}

/// The equalities of `xs` and `ys` along one telescope, and whether any is heterogeneous:
/// homogeneous at a position whose type mentions no earlier position, heterogeneous where it does
/// (read from the telescope itself, at depth `j` for the `j`-th position); `admit` vets each side's
/// type; the telescope is used up.
fn componentwise(
    genv: &Context,
    lctx: &dyn LocalTypes,
    telescope: Db,
    xs: &[&Db],
    ys: &[&Db],
    admit: &dyn Fn(&Db) -> bool,
) -> Option<(Vec<Db>, bool)> {
    if xs.len() != ys.len() {
        return None;
    }
    let (mut generic, mut left, mut right) = (telescope.clone(), telescope.clone(), telescope);
    let mut parts = Vec::with_capacity(xs.len());
    let mut any_dependent = false;
    for (j, (x, y)) in xs.iter().zip(ys).enumerate() {
        let Db::Pi(shape, rest) = generic else {
            return None;
        };
        let Db::Pi(lt, lb) = left else { return None };
        let Db::Pi(rt, rb) = right else { return None };
        if !admit(&lt) || !admit(&rt) {
            return None;
        }
        let dependent = (0..j).any(|k| mentions_index(&shape, k));
        any_dependent |= dependent;
        parts.push(if dependent {
            heterogeneous(genv, lctx, &lt, &rt, x, y)?
        } else {
            obs(
                level_of(genv, lctx, &lt)?,
                lt.into_db(),
                (*x).clone(),
                (*y).clone(),
            )
        });
        generic = rest.into_db();
        left = beta_open(&lb, x);
        right = beta_open(&rb, y);
    }
    if matches!(left, Db::Pi(..)) || matches!(right, Db::Pi(..)) {
        return None;
    }
    Some((parts, any_dependent))
}

/// `SSigma (T ~ T') (λ e. Obs_cast T T' e x ~[T'] y)`.
fn heterogeneous(
    genv: &Context,
    lctx: &dyn LocalTypes,
    from: &Db,
    to: &Db,
    x: &Db,
    y: &Db,
) -> Option<Db> {
    let level = level_of(genv, lctx, from)?;
    if !level.equiv(&level_of(genv, lctx, to)?) {
        return None;
    }
    let types_equal = obs(
        level.succ(),
        Db::Sort(level.clone()),
        from.clone(),
        to.clone(),
    );
    let up = |t: &Db| shift(t, 1, 0);
    let carried = apply(
        cast_at(level.clone()),
        [up(from), up(to), Db::Var(0), up(x)],
    );
    let body = obs(level, up(to), carried, up(y));
    Some(apply(
        global("SSigma"),
        [
            types_equal.clone(),
            Db::Lam(DbBox::new(types_equal), DbBox::new(body)),
        ],
    ))
}

/// `Obs_cast.{u} A B e t` where `A ~ B` computes: along one universe, `t`; along two function types,
/// `λ (x : E). Obs_cast C[back] K (SSigma_snd … e x) (t back)` with `back = Obs_cast E D (SSigma_fst … e) x`;
/// along one family without indices and with independent parameters, the constructor of `t` at
/// `B`'s parameters with each independent field carried. Everything else is stuck.
fn carry(
    genv: &Context,
    lctx: &dyn LocalTypes,
    from: &Db,
    to: &Db,
    proof: &Db,
    value: &Db,
) -> Option<Db> {
    let (from, to) = (whnf_in(genv, lctx, from), whnf_in(genv, lctx, to));
    match type_equality(genv, lctx, &from, &to)? {
        TypeEquality::Holds => Some(value.clone()),
        TypeEquality::Products {
            domain,
            domains,
            codomains,
        } => {
            let (Db::Pi(d, c), Db::Pi(e, k)) = (&from, &to) else {
                return None;
            };
            let up = |t: &Db| shift(t, 1, 0);
            let first = apply(
                global("SSigma_fst"),
                [up(&domains), up(&codomains), up(proof)],
            );
            let back = apply(cast_at(domain), [up(e), up(d), first, Db::Var(0)]);
            let left = beta_open(&shift(c, 1, 1), &back);
            let right = k.as_ref().clone();
            let mut inside = extend_local(lctx);
            inside.push(e.as_ref().clone());
            let codomain = level_of(genv, &inside, &left)?;
            if !codomain.equiv(&level_of(genv, &inside, &right)?) {
                return None;
            }
            let second = apply(
                global("SSigma_snd"),
                [up(&domains), up(&codomains), up(proof), Db::Var(0)],
            );
            let image = Db::App(DbBox::new(up(value)), DbBox::new(back));
            let body = apply(cast_at(codomain), [left, right, second, image]);
            Some(Db::Lam(DbBox::new(e.as_ref().clone()), DbBox::new(body)))
        }
        TypeEquality::Family {
            parts,
            dependent: false,
        } => carry_value(genv, lctx, &from, &to, &parts, proof, value),
        TypeEquality::Quotient { parts } => {
            carry_class(genv, lctx, &from, &to, &parts, proof, value)
        }
        TypeEquality::Fails | TypeEquality::Family { .. } => None,
    }
}

/// `Obs_cast (Quot X R) (Quot Y S) e (Quot_mk X R x)` is `Quot_mk Y S (Obs_cast X Y e₀ x)`.
fn carry_class(
    genv: &Context,
    lctx: &dyn LocalTypes,
    from: &Db,
    to: &Db,
    parts: &[Db],
    proof: &Db,
    value: &Db,
) -> Option<Db> {
    let (Former::Quotient(_, xs), Former::Quotient(_, ys)) =
        (former(genv, from)?, former(genv, to)?)
    else {
        return None;
    };
    let value = whnf_in(genv, lctx, value);
    let (constructor, arguments) = spine_ref(&value);
    let [_, _, x] = arguments.as_slice() else {
        return None;
    };
    if quotient_part(genv, constructor) != Some(QuotientPart::Class) {
        return None;
    }
    let carriers = components(parts, proof).into_iter().next()?;
    let representative = apply(
        cast_at(level_of(genv, lctx, xs[0])?),
        [xs[0].clone(), ys[0].clone(), carriers, (*x).clone()],
    );
    Some(apply(
        constructor.clone(),
        [ys[0].clone(), ys[1].clone(), representative],
    ))
}

/// `Obs_cast (I p̄) (I q̄) e (C p̄ x̄)` is `C q̄ (Obs_cast T₀[p̄] T₀[q̄] h₀ x₀) …`, each `hₖ` moving `Tₖ`'s
/// parameters from `p̄` to `q̄` one `Obs_transp` at a time along `e`'s components.
fn carry_value(
    genv: &Context,
    lctx: &dyn LocalTypes,
    from: &Db,
    to: &Db,
    parts: &[Db],
    proof: &Db,
    value: &Db,
) -> Option<Db> {
    let (Former::Family(family, levels, ps), Former::Family(_, _, qs)) =
        (former(genv, from)?, former(genv, to)?)
    else {
        return None;
    };
    if genv.inductive_num_indices(family) != 0 || ps.len() != parts.len() || qs.len() != ps.len() {
        return None;
    }
    let value = constructor_form(genv, lctx, value)?;
    let (constructor, arguments) = spine_ref(&value);
    let (Db::Global(c) | Db::Const { name: c, .. }) = constructor else {
        return None;
    };
    if genv.constructor_inductive(c) != Some(family) || arguments.len() < ps.len() {
        return None;
    }
    let fields = &arguments[ps.len()..];
    let telescope = infer(genv, lctx, constructor).ok()?;
    if !fields_independent(&telescope, ps.len()) {
        return None;
    }
    let arity = infer(genv, lctx, &family_head(family, levels)).ok()?;
    let parameter_types = telescope_types(arity, &ps)?;
    let components = components(parts, proof);
    let mut carried = Vec::with_capacity(fields.len());
    for k in 0..fields.len() {
        let source = field_type(&telescope, &ps, fields, k)?;
        let target = field_type(&telescope, &qs, fields, k)?;
        let level = level_of(genv, lctx, &source)?;
        if !level.equiv(&level_of(genv, lctx, &target)?) {
            return None;
        }
        let mut evidence = apply(
            Db::Const {
                name: "Obs_refl".to_string(),
                levels: vec![level.succ()],
            },
            [Db::Sort(level.clone()), source.clone()],
        );
        for (i, parameter_type) in parameter_types.iter().enumerate() {
            let parameter = level_of(genv, lctx, parameter_type)?;
            // Under `λ (b : Pᵢ)` and `λ (h : pᵢ ~ b)`: `#1` is `b`.
            let up2 = |t: &Db| shift(t, 2, 0);
            let mixed: Vec<Db> = qs[..i]
                .iter()
                .map(|q| up2(q))
                .chain([Db::Var(1)])
                .chain(ps[i + 1..].iter().map(|p| up2(p)))
                .collect();
            let shifted_fields: Vec<Db> = fields.iter().map(|x| up2(x)).collect();
            let moved = field_type(
                &telescope,
                &mixed.iter().collect::<Vec<_>>(),
                &shifted_fields.iter().collect::<Vec<_>>(),
                k,
            )?;
            let hypothesis = obs(
                parameter.clone(),
                shift(parameter_type, 1, 0),
                shift(ps[i], 1, 0),
                Db::Var(0),
            );
            let motive = Db::Lam(
                DbBox::new(parameter_type.clone()),
                DbBox::new(Db::Lam(
                    DbBox::new(hypothesis),
                    DbBox::new(obs(
                        level.succ(),
                        Db::Sort(level.clone()),
                        up2(&source),
                        moved,
                    )),
                )),
            );
            evidence = apply(
                Db::Const {
                    name: "Obs_transp".to_string(),
                    levels: vec![parameter],
                },
                [
                    parameter_type.clone(),
                    ps[i].clone(),
                    motive,
                    evidence,
                    qs[i].clone(),
                    components[i].clone(),
                ],
            );
        }
        carried.push(apply(
            cast_at(level),
            [source, target, evidence, fields[k].clone()],
        ));
    }
    Some(apply(
        constructor.clone(),
        qs.iter().map(|q| (*q).clone()).chain(carried),
    ))
}

/// Whether no field of a constructor's telescope (after its `parameters`) mentions an earlier field.
fn fields_independent(telescope: &Db, parameters: usize) -> bool {
    let mut cur = telescope;
    for _ in 0..parameters {
        let Db::Pi(_, rest) = cur else { return false };
        cur = rest;
    }
    let mut j = 0;
    while let Db::Pi(domain, rest) = cur {
        if (0..j).any(|k| mentions_index(domain, k)) {
            return false;
        }
        cur = rest;
        j += 1;
    }
    true
}

/// The types of the first `values.len()` positions of `telescope`, each at the earlier values.
fn telescope_types(telescope: Db, values: &[&Db]) -> Option<Vec<Db>> {
    let mut cur = telescope;
    let mut types = Vec::with_capacity(values.len());
    for value in values {
        let Db::Pi(domain, rest) = cur else {
            return None;
        };
        types.push(domain.into_db());
        cur = beta_open(&rest, value);
    }
    Some(types)
}

/// `telescope` past its first positions, instantiated at `values`.
fn peel(telescope: Db, values: &[&Db]) -> Option<Db> {
    let mut cur = telescope;
    for value in values {
        let Db::Pi(_, rest) = cur else { return None };
        cur = beta_open(&rest, value);
    }
    Some(cur)
}

/// The type of field `k` of a constructor `telescope`, at `parameters` and the earlier `fields`.
fn field_type(telescope: &Db, parameters: &[&Db], fields: &[&Db], k: usize) -> Option<Db> {
    let cur = peel(peel(telescope.clone(), parameters)?, &fields[..k])?;
    let Db::Pi(domain, _) = cur else { return None };
    Some(domain.into_db())
}

/// Each component of `proof : SAnd parts₀ (SAnd parts₁ …)`.
fn components(parts: &[Db], proof: &Db) -> Vec<Db> {
    let mut out = Vec::with_capacity(parts.len());
    let mut rest = proof.clone();
    for (j, part) in parts.iter().enumerate() {
        if j + 1 == parts.len() {
            out.push(rest);
            break;
        }
        let tail = conjunction(parts[j + 1..].to_vec());
        out.push(apply(
            global("SAnd_left"),
            [part.clone(), tail.clone(), rest.clone()],
        ));
        rest = apply(global("SAnd_right"), [part.clone(), tail, rest]);
    }
    out
}

fn conjunction(mut parts: Vec<Db>) -> Db {
    let Some(last) = parts.pop() else {
        return global("STrue");
    };
    parts
        .into_iter()
        .rev()
        .fold(last, |rest, part| apply(global("SAnd"), [part, rest]))
}

fn level_of(genv: &Context, lctx: &dyn LocalTypes, ty: &Db) -> Option<Universe> {
    infer_sort(genv, lctx, ty)
        .ok()
        .map(|sort| sort.least_level())
}

fn obs(level: Universe, ty: Db, a: Db, b: Db) -> Db {
    apply(
        Db::Const {
            name: "Obs".to_string(),
            levels: vec![level],
        },
        [ty, a, b],
    )
}

fn cast_at(level: Universe) -> Db {
    Db::Const {
        name: "Obs_cast".to_string(),
        levels: vec![level],
    }
}

/// `a → b`: `b` lifted under the new binder.
fn arrow(a: &Db, b: &Db) -> Db {
    Db::Pi(DbBox::new(a.clone()), DbBox::new(shift(b, 1, 0)))
}

fn global(name: &str) -> Db {
    Db::Global(name.to_string())
}

fn apply(f: Db, args: impl IntoIterator<Item = Db>) -> Db {
    args.into_iter()
        .fold(f, |f, x| Db::App(DbBox::new(f), DbBox::new(x)))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/recheck/observational/observational_controls.rs"]
mod observational_controls;
