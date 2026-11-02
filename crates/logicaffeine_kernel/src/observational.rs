//! Observational equality: `Obs.{u} A a b : SProp`, an equality over every type that computes on
//! the weak-head normal form of `A`, with `Obs_refl`, `Obs_transp` (its elimination, into `SProp` only)
//! and `Obs_cast` (a value carried along an equality of types), after Pujet and Tabareau's CC^obs.
//!
//! [`register`](crate::observational::register) installs them beside the standard library, with the `SProp` connectives the rules
//! produce (`STrue`, `SFalse`, `SAnd`, `SSigma`, `SBox` and their projections) and `EqvGen`, the
//! equivalence closure of a relation, and certifies them: the kernels' computation rules fire only
//! for the constants the certificate names, and displacing any name it rests on revokes it, as the
//! standard quotient's certificate is.
//!
//! `Obs.{u} A a b`, by the weak-head normal form of `A`:
//!
//! | `A` | `a ~ b` is |
//! |---|---|
//! | `Π (x : D). C` | `Π (x : D). a x ~[C] b x` |
//! | `SProp` | `SAnd (a → b) (b → a)` |
//! | `Prop` | `SAnd (SBox (a → b)) (SBox (b → a))` |
//! | another universe, `a` and `b` types that are not propositions | by their formers: two universes equal when their levels are, two function types `SSigma (D' ~ D) (λ e. Π (x' : D'). C[Obs_cast D' D e x'] ~ C'[x'])`, one inductive family or quotient former the conjunction of its arguments' equalities, two different formers `SFalse` |
//! | an inductive family that is not a proposition, `a` and `b` constructors | `SFalse` for two constructors, else the conjunction of the fields' equalities |
//! | a quotient that is not a proposition, `a` and `b` classes | `SBox (EqvGen A r x y)` of their representatives |
//!
//! A conjunction is `STrue` for none and right-nested `SAnd` otherwise; a position whose type mentions
//! an earlier position compares heterogeneously, `SSigma (T ~ T') (λ e. Obs_cast T T' e x ~ y)`, which
//! positions are dependent read off the telescope. Three guards keep the rules sound, each locked by
//! a test that derives `SFalse` without it: two proofs are never compared by their constructors (they
//! are one by irrelevance); a proposition seen as a type is never compared by its former (`Nat →
//! False` and `Bool → False` are one proposition); and a family's arguments are compared only when
//! its universe can store them (Hur: an injective `I : (Type → Type) → Type` refutes excluded
//! middle). Everything else is stuck.
//!
//! `Obs_cast.{u} A B e t` computes where `A ~ B` does: along one universe to `t`; along two function
//! types to `λ (x' : D'). Obs_cast C[back] C'[x'] (SSigma_snd … e x') (t back)` with `back` the argument
//! carried back along the domains' equality; along one family without indices whose parameters and
//! fields depend on no earlier one to the same constructor at the target parameters, each field
//! carried along an equality of its types built by `Obs_transp`; along one quotient former to the class
//! of the carried representative. A cast between propositions, along evidence of `SFalse`, along a
//! stuck type or of a value that is not yet a constructor is stuck.

use crate::context::{Context, MutualInductive};
use crate::error::KernelResult;
use crate::term::{Term, Universe};
use std::collections::BTreeSet;

/// What a certified constant is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Obs,
    Refl,
    Transp,
    Cast,
}

/// The constants the computation rules fire for, and every name they rest on.
#[derive(Debug)]
pub(crate) struct Certificate {
    pub(crate) constants: Vec<(String, Kind)>,
    pub(crate) dependencies: BTreeSet<String>,
}

impl Certificate {
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.constants.iter().map(|(n, _)| n.as_str())
    }
}

/// What `name` is to the certified observational equality of `ctx`, if anything.
pub(crate) fn kind(ctx: &Context, name: &str) -> Option<Kind> {
    ctx.observational
        .as_ref()?
        .constants
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, k)| *k)
}

fn g(name: &str) -> Term {
    Term::Global(name.to_string())
}

fn v(name: &str) -> Term {
    Term::Var(name.to_string())
}

fn app(f: Term, args: impl IntoIterator<Item = Term>) -> Term {
    args.into_iter()
        .fold(f, |f, a| Term::App(Box::new(f), Box::new(a)))
}

fn pi(param: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: param.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}

fn sort(u: Universe) -> Term {
    Term::Sort(u)
}

fn obs_at(u: Universe) -> Term {
    Term::Const {
        name: "Obs".to_string(),
        levels: vec![u],
    }
}

/// The `SProp` connectives and the four constants, certified.
pub fn register(ctx: &mut Context) -> KernelResult<()> {
    let certificate = declare(ctx)?;
    ctx.certify_observational(certificate);
    Ok(())
}

/// The connectives, their projections and the four constants, declared; the certificate that would
/// make them compute, not yet installed.
fn declare(ctx: &mut Context) -> KernelResult<Certificate> {
    let sprop = || sort(Universe::SProp);
    let u = || Universe::Var("u".to_string());
    ctx.add_mutual_inductives(&[MutualInductive {
        name: "STrue".to_string(),
        sort: sprop(),
        num_params: 0,
        constructors: vec![("sI".to_string(), g("STrue"))],
    }])?;
    ctx.add_mutual_inductives(&[MutualInductive {
        name: "SFalse".to_string(),
        sort: sprop(),
        num_params: 0,
        constructors: vec![],
    }])?;
    ctx.add_mutual_inductives(&[MutualInductive {
        name: "SAnd".to_string(),
        sort: pi("A", sprop(), pi("B", sprop(), sprop())),
        num_params: 2,
        constructors: vec![(
            "spair".to_string(),
            pi(
                "A",
                sprop(),
                pi(
                    "B",
                    sprop(),
                    pi(
                        "a",
                        v("A"),
                        pi("b", v("B"), app(g("SAnd"), [v("A"), v("B")])),
                    ),
                ),
            ),
        )],
    }])?;
    ctx.add_mutual_inductives(&[MutualInductive {
        name: "SSigma".to_string(),
        sort: pi("A", sprop(), pi("B", pi("_", v("A"), sprop()), sprop())),
        num_params: 2,
        constructors: vec![(
            "sexists".to_string(),
            pi(
                "A",
                sprop(),
                pi(
                    "B",
                    pi("_", v("A"), sprop()),
                    pi(
                        "a",
                        v("A"),
                        pi(
                            "b",
                            app(v("B"), [v("a")]),
                            app(g("SSigma"), [v("A"), v("B")]),
                        ),
                    ),
                ),
            ),
        )],
    }])?;
    let sbox = || Term::Const {
        name: "SBox".to_string(),
        levels: vec![u()],
    };
    ctx.add_universe_poly_inductives(
        &["u".to_string()],
        &[MutualInductive {
            name: "SBox".to_string(),
            sort: pi("A", sort(u()), sprop()),
            num_params: 1,
            constructors: vec![(
                "sbox".to_string(),
                pi("A", sort(u()), pi("a", v("A"), app(sbox(), [v("A")]))),
            )],
        }],
    )?;
    register_projections(ctx)?;
    register_closure(ctx)?;
    let levels = vec!["u".to_string()];
    ctx.add_universe_poly_declaration(
        "Obs",
        levels.clone(),
        pi("A", sort(u()), pi("a", v("A"), pi("b", v("A"), sprop()))),
    )?;
    let obs = |ty: Term, a: Term, b: Term| app(obs_at(u()), [ty, a, b]);
    ctx.add_universe_poly_declaration(
        "Obs_refl",
        levels.clone(),
        pi("A", sort(u()), pi("a", v("A"), obs(v("A"), v("a"), v("a")))),
    )?;
    let refl = Term::Const {
        name: "Obs_refl".to_string(),
        levels: vec![u()],
    };
    ctx.add_universe_poly_declaration(
        "Obs_transp",
        levels.clone(),
        pi(
            "A",
            sort(u()),
            pi(
                "a",
                v("A"),
                pi(
                    "P",
                    pi("b", v("A"), pi("_", obs(v("A"), v("a"), v("b")), sprop())),
                    pi(
                        "h",
                        app(v("P"), [v("a"), app(refl, [v("A"), v("a")])]),
                        pi(
                            "b",
                            v("A"),
                            pi(
                                "e",
                                obs(v("A"), v("a"), v("b")),
                                app(v("P"), [v("b"), v("e")]),
                            ),
                        ),
                    ),
                ),
            ),
        ),
    )?;
    let types_equal = app(obs_at(u().succ()), [sort(u()), v("A"), v("B")]);
    ctx.add_universe_poly_declaration(
        "Obs_cast",
        levels,
        pi(
            "A",
            sort(u()),
            pi(
                "B",
                sort(u()),
                pi("e", types_equal, pi("_", v("A"), v("B"))),
            ),
        ),
    )?;
    let constants = vec![
        ("Obs".to_string(), Kind::Obs),
        ("Obs_refl".to_string(), Kind::Refl),
        ("Obs_transp".to_string(), Kind::Transp),
        ("Obs_cast".to_string(), Kind::Cast),
    ];
    let mut dependencies: BTreeSet<String> = constants.iter().map(|(n, _)| n.clone()).collect();
    for name in [
        "STrue", "sI", "SFalse", "SAnd", "spair", "SSigma", "sexists", "SBox", "sbox",
    ]
    .into_iter()
    .chain(PROJECTIONS)
    .chain(CLOSURE)
    {
        dependencies.insert(name.to_string());
    }
    Ok(Certificate {
        constants,
        dependencies,
    })
}

/// The projections out of the `SProp` pairs, which the rules for `Obs_cast` read an equality's
/// components with.
const PROJECTIONS: [&str; 4] = ["SAnd_left", "SAnd_right", "SSigma_fst", "SSigma_snd"];

fn register_projections(ctx: &mut Context) -> KernelResult<()> {
    let sprop = || sort(Universe::SProp);
    let and = || app(g("SAnd"), [v("A"), v("B")]);
    let family = || pi("_", v("A"), sprop());
    let sigma = || app(g("SSigma"), [v("A"), v("B")]);
    let out_of = |discriminant: &str, owner: Term, motive: Term, case: Term| Term::Match {
        discriminant: Box::new(v(discriminant)),
        motive: Box::new(lam(discriminant, owner, motive)),
        cases: vec![case],
    };
    for (name, field, ty) in [("SAnd_left", "a", v("A")), ("SAnd_right", "b", v("B"))] {
        let body = out_of(
            "p",
            and(),
            ty.clone(),
            lam("a", v("A"), lam("b", v("B"), v(field))),
        );
        ctx.declare(
            name,
            pi("A", sprop(), pi("B", sprop(), pi("p", and(), ty))),
            lam("A", sprop(), lam("B", sprop(), lam("p", and(), body))),
        )?;
    }
    let fields = |chosen: &str| lam("a", v("A"), lam("b", app(v("B"), [v("a")]), v(chosen)));
    let fst = out_of("s", sigma(), v("A"), fields("a"));
    ctx.declare(
        "SSigma_fst",
        pi("A", sprop(), pi("B", family(), pi("s", sigma(), v("A")))),
        lam("A", sprop(), lam("B", family(), lam("s", sigma(), fst))),
    )?;
    let second = || app(v("B"), [app(g("SSigma_fst"), [v("A"), v("B"), v("s")])]);
    let snd = out_of("s", sigma(), second(), fields("b"));
    ctx.declare(
        "SSigma_snd",
        pi("A", sprop(), pi("B", family(), pi("s", sigma(), second()))),
        lam("A", sprop(), lam("B", family(), lam("s", sigma(), snd))),
    )
}

/// The equivalence closure of a relation, which the equality of two quotient classes is.
const CLOSURE: [&str; 5] = [
    "EqvGen",
    "EqvGen_rel",
    "EqvGen_refl",
    "EqvGen_symm",
    "EqvGen_trans",
];

/// `EqvGen.{u} (A : Sort u) (r : A → A → Prop) : A → A → Prop`, the least equivalence containing `r`.
fn register_closure(ctx: &mut Context) -> KernelResult<()> {
    let u = || Universe::Var("u".to_string());
    let relation = || pi("_", v("A"), pi("_", v("A"), sort(Universe::Prop)));
    let closure = |x: &str, y: &str| {
        app(
            Term::Const {
                name: "EqvGen".to_string(),
                levels: vec![u()],
            },
            [v("A"), v("r"), v(x), v(y)],
        )
    };
    let over = |body: Term| pi("A", sort(u()), pi("r", relation(), body));
    let point = |name: &str, body: Term| pi(name, v("A"), body);
    ctx.add_universe_poly_inductives(
        &["u".to_string()],
        &[MutualInductive {
            name: "EqvGen".to_string(),
            sort: over(point("x", point("y", sort(Universe::Prop)))),
            num_params: 2,
            constructors: vec![
                (
                    "EqvGen_rel".to_string(),
                    over(point(
                        "x",
                        point(
                            "y",
                            pi("_", app(v("r"), [v("x"), v("y")]), closure("x", "y")),
                        ),
                    )),
                ),
                (
                    "EqvGen_refl".to_string(),
                    over(point("x", closure("x", "x"))),
                ),
                (
                    "EqvGen_symm".to_string(),
                    over(point(
                        "x",
                        point("y", pi("_", closure("x", "y"), closure("y", "x"))),
                    )),
                ),
                (
                    "EqvGen_trans".to_string(),
                    over(point(
                        "x",
                        point(
                            "y",
                            point(
                                "z",
                                pi(
                                    "_",
                                    closure("x", "y"),
                                    pi("_", closure("y", "z"), closure("x", "z")),
                                ),
                            ),
                        ),
                    )),
                ),
            ],
        }],
    )
}

/// One step of a certified constant applied to all its arguments: `Obs` on the shape of its type,
/// `Obs_cast` on the shapes of its two types and its argument. The reducer brings the arguments to
/// their normal forms, so a rule that does not fire yet fires once they are reduced.
pub(crate) fn reduce(ctx: &Context, head: &Term, args: &[&Term]) -> Option<Term> {
    let Term::Const { name, levels } = head else {
        return None;
    };
    if levels.len() != 1 {
        return None;
    }
    match (kind(ctx, name)?, args) {
        (Kind::Obs, [ty, a, b]) => equality(ctx, ty, a, b),
        (Kind::Cast, [from, to, proof, value]) => carry(ctx, from, to, proof, value),
        _ => None,
    }
}

/// `Obs.{u} A a b`, by `A`:
///
/// - a function type: pointwise equality (function extensionality);
/// - `SProp` or `Prop`: logical equivalence (propositional extensionality), squashed into `SProp`
///   for `Prop`;
/// - any other universe: equality of types that are not propositions, by their formers;
/// - an inductive family that holds no proposition: equality of values, by their constructors.
///
/// Every other `A` is stuck, among them a family that may be a proposition and any type that is
/// not yet a former; so is an equality of propositions at a universe that may be `Prop`.
fn equality(ctx: &Context, ty: &Term, a: &Term, b: &Term) -> Option<Term> {
    match ty {
        Term::Pi {
            param,
            param_type,
            body_type,
        } => pointwise(ctx, param, param_type, body_type, a, b),
        Term::Sort(Universe::SProp) => Some(app(g("SAnd"), [arrow(a, b), arrow(b, a)])),
        Term::Sort(level) if level.equiv(&Universe::Prop) => {
            let squash = |p: Term| {
                app(
                    Term::Const {
                        name: "SBox".to_string(),
                        levels: vec![Universe::Prop],
                    },
                    [p],
                )
            };
            Some(app(g("SAnd"), [squash(arrow(a, b)), squash(arrow(b, a))]))
        }
        Term::Sort(_) => type_equality(ctx, a, b).map(TypeEquality::into_term),
        _ => values(ctx, ty, a, b),
    }
}

/// `Π (x : D). Obs.{v} (C x) (a x) (b x)`, `v` the level of the codomain.
fn pointwise(
    ctx: &Context,
    param: &str,
    domain: &Term,
    codomain: &Term,
    a: &Term,
    b: &Term,
) -> Option<Term> {
    let x = fresh(param, &[a, b, domain, &scoped(param, codomain)]);
    let codomain = if x == param {
        codomain.clone()
    } else {
        substitute(codomain, param, &v(&x))
    };
    let level = level_of(&ctx.extend(&x, domain.clone()), &codomain)?;
    let at = |t: &Term| Term::App(Box::new(t.clone()), Box::new(v(&x)));
    Some(pi(&x, domain.clone(), obs(level, codomain, at(a), at(b))))
}

/// What the equality of two types that are not propositions computes to, read on their formers.
enum TypeEquality {
    /// One universe.
    Holds,
    /// Two different formers, or two universes no instantiation makes one.
    Fails,
    /// Two function types: `SSigma domains codomains`, `domains` the equality of the domains at
    /// level `domain` and `codomains` the family over it.
    Products {
        domain: Universe,
        domains: Term,
        codomains: Term,
    },
    /// One family: its arguments' equalities, and whether any is heterogeneous.
    Family { parts: Vec<Term>, dependent: bool },
    /// One quotient former: the carriers' equality first, then the relations'.
    Quotient { parts: Vec<Term> },
}

impl TypeEquality {
    fn into_term(self) -> Term {
        match self {
            TypeEquality::Holds => g("STrue"),
            TypeEquality::Fails => g("SFalse"),
            TypeEquality::Products {
                domains, codomains, ..
            } => app(g("SSigma"), [domains, codomains]),
            TypeEquality::Family { parts, .. } | TypeEquality::Quotient { parts } => {
                conjunction(parts)
            }
        }
    }
}

/// Equality of two types that are not propositions, by their formers: universes by their levels,
/// function types by their domains and codomains, the same inductive family by its arguments, and
/// two different formers never. A side whose own sort may be `Prop` is stuck: a proposition seen
/// as a type is compared by `Prop`'s rule, never by its former (`Nat → False` and `Bool → False`
/// are the same proposition).
fn type_equality(ctx: &Context, a: &Term, b: &Term) -> Option<TypeEquality> {
    let (left, right) = (former(ctx, a)?, former(ctx, b)?);
    for side in [a, b] {
        if !crate::type_checker::infer_sort(ctx, side)
            .ok()?
            .is_never_prop()
        {
            return None;
        }
    }
    match (left, right) {
        (Former::Sort(s), Former::Sort(t)) => {
            if s.equiv(t) {
                Some(TypeEquality::Holds)
            } else if distinct_levels(s, t) {
                Some(TypeEquality::Fails)
            } else {
                None
            }
        }
        (Former::Pi(p, d, c), Former::Pi(q, e, k)) => products(ctx, (p, d, c), (q, e, k)),
        (Former::Family(i, li, xs), Former::Family(j, lj, ys)) => {
            if i != j {
                return Some(TypeEquality::Fails);
            }
            families(ctx, a, (i, li, &xs), (lj, &ys))
        }
        (Former::Quotient(h, xs), Former::Quotient(k, ys)) => {
            if !same_quotient(h, k)? {
                return Some(TypeEquality::Fails);
            }
            quotients(ctx, h, &xs, &ys)
        }
        _ => Some(TypeEquality::Fails),
    }
}

/// What a type is built by, read on its head.
enum Former<'a> {
    Sort(&'a Universe),
    Pi(&'a str, &'a Term, &'a Term),
    Family(&'a str, &'a [Universe], Vec<&'a Term>),
    /// A certified quotient former (the prelude's or a kernel-installed one) and its arguments.
    Quotient(&'a Term, Vec<&'a Term>),
}

fn former<'a>(ctx: &Context, t: &'a Term) -> Option<Former<'a>> {
    match t {
        Term::Sort(level) => Some(Former::Sort(level)),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => Some(Former::Pi(param, param_type, body_type)),
        _ => {
            let (head, args) = spine(t);
            match head {
                Term::Global(name) if ctx.is_inductive(name) => {
                    Some(Former::Family(name, &[], args))
                }
                Term::Const { name, levels } if ctx.is_inductive(name) => {
                    Some(Former::Family(name, levels, args))
                }
                _ if quotient_part(ctx, head) == Some(QuotientPart::Former) => {
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

/// What a certified quotient constant `head` is, if it is one.
fn quotient_part(ctx: &Context, head: &Term) -> Option<QuotientPart> {
    use crate::quotient::{Kind, PreludeKind};
    match head {
        Term::Global(name) => match crate::quotient::prelude_kind(ctx, name)? {
            PreludeKind::Type => Some(QuotientPart::Former),
            PreludeKind::Constructor => Some(QuotientPart::Class),
            _ => None,
        },
        Term::Const { name, levels } => match crate::quotient::kind(ctx, name, levels)? {
            Kind::Type => Some(QuotientPart::Former),
            Kind::Constructor => Some(QuotientPart::Class),
            _ => None,
        },
        _ => None,
    }
}

/// `Quot_mk A r x ~[Quot A r] Quot_mk A r y` is `SBox (EqvGen A r x y)`: two classes are one exactly
/// when their representatives are related by the equivalence `r` generates.
fn classes(ctx: &Context, ty: &Term, carrier: &[&Term], a: &Term, b: &Term) -> Option<Term> {
    if carrier.len() != 2
        || !crate::type_checker::infer_sort(ctx, ty)
            .ok()?
            .is_never_prop()
    {
        return None;
    }
    let ((c, xs), (d, ys)) = (spine(a), spine(b));
    if quotient_part(ctx, c) != Some(QuotientPart::Class)
        || quotient_part(ctx, d) != Some(QuotientPart::Class)
    {
        return None;
    }
    let ([_, _, x], [_, _, y]) = (xs.as_slice(), ys.as_slice()) else {
        return None;
    };
    let level = level_of(ctx, carrier[0])?;
    let closure = app(
        Term::Const {
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
    Some(app(
        Term::Const {
            name: "SBox".to_string(),
            levels: vec![Universe::Prop],
        },
        [closure],
    ))
}

/// Two quotients by one former: the equalities of their carriers and relations, as a family's
/// arguments compare (the relation's type mentions the carrier, so it compares heterogeneously).
fn quotients(ctx: &Context, head: &Term, xs: &[&Term], ys: &[&Term]) -> Option<TypeEquality> {
    if xs.len() != 2 || ys.len() != 2 {
        return None;
    }
    let level = crate::type_checker::infer_sort(ctx, xs[0]).ok()?;
    let arity = match head {
        Term::Global(name) => {
            let header = ctx.get_global(name)?;
            let [u] = crate::term::free_level_vars(header).try_into().ok()?;
            crate::term::instantiate_universes(header, &[(u, level)].into_iter().collect())
        }
        _ => crate::type_checker::infer_type(ctx, head).ok()?,
    };
    // `Quot.{u} : Π (A : Sort u). (A → A → Prop) → Sort u`: in a quotient that is not a proposition
    // (`u ≥ 1`), the carrier is an element of the universe and the relation a family of propositions
    // over it, both small, so no argument is vetted.
    let (parts, _) = componentwise(ctx, arity, xs, ys, &|_| true)?;
    Some(TypeEquality::Quotient { parts })
}

/// One certified quotient head and another: the same constant at levels that agree.
fn same_quotient(h: &Term, k: &Term) -> Option<bool> {
    match (h, k) {
        (Term::Global(m), Term::Global(n)) => Some(m == n),
        (Term::Const { name: m, levels: l }, Term::Const { name: n, levels: k }) => {
            if m != n {
                return Some(false);
            }
            (l.len() == k.len() && l.iter().zip(k).all(|(a, b)| a.equiv(b))).then_some(true)
        }
        _ => Some(false),
    }
}

/// Two levels no instantiation of their variables makes equal: one strictly below the other
/// everywhere, or two different closed levels.
fn distinct_levels(s: &Universe, t: &Universe) -> bool {
    s.succ().is_subtype_of(t)
        || t.succ().is_subtype_of(s)
        || (closed(s) && closed(t) && !s.equiv(t))
}

fn closed(level: &Universe) -> bool {
    match level {
        Universe::SProp | Universe::Prop | Universe::Type(_) => true,
        Universe::Var(_) => false,
        Universe::Succ(l) => closed(l),
        Universe::Max(a, b) | Universe::IMax(a, b) => closed(a) && closed(b),
    }
}

/// `(Π (x : D). C) ~ (Π (x' : D'). C')` is
/// `SSigma (D' ~ D) (λ e. Π (x' : D'). C[x := Obs_cast D' D e x'] ~ C'[x'])`: the domains are equal,
/// and the codomains are equal at every argument carried across.
fn products(
    ctx: &Context,
    (p, d, c): (&str, &Term, &Term),
    (q, e, k): (&str, &Term, &Term),
) -> Option<TypeEquality> {
    let domain = level_of(ctx, d)?;
    if !domain.equiv(&level_of(ctx, e)?) {
        return None;
    }
    let (c_scoped, k_scoped) = (scoped(p, c), scoped(q, k));
    let proof = fresh("e", &[d, e, &c_scoped, &k_scoped]);
    let x = fresh(q, &[d, e, &c_scoped, &k_scoped, &v(&proof)]);
    let domains = obs(
        domain.succ(),
        Term::Sort(domain.clone()),
        e.clone(),
        d.clone(),
    );
    let carried = app(
        cast_at(domain.clone()),
        [e.clone(), d.clone(), v(&proof), v(&x)],
    );
    let left = substitute(c, p, &carried);
    let right = if x == q {
        k.clone()
    } else {
        substitute(k, q, &v(&x))
    };
    let inside = ctx.extend(&proof, domains.clone()).extend(&x, e.clone());
    let codomain = level_of(&inside, &left)?;
    if !codomain.equiv(&level_of(&inside, &right)?) {
        return None;
    }
    let codomains_equal = pi(
        &x,
        e.clone(),
        obs(codomain.succ(), Term::Sort(codomain), left, right),
    );
    let codomains = lam(&proof, domains.clone(), codomains_equal);
    Some(TypeEquality::Products {
        domain,
        domains,
        codomains,
    })
}

/// `I p̄ ~ I q̄` is the conjunction of the arguments' equalities, when every argument is small
/// enough to be stored in the family's own universe: an element of a type in it, a universe it
/// contains, or a family of those over an element type. A family over a larger argument (Hur's
/// `I : (Type → Type) → Type`) would make `I` an injection from that larger type into its
/// universe, which classical logic refutes; such an equality is stuck.
fn families(
    ctx: &Context,
    a: &Term,
    (family, levels, xs): (&str, &[Universe], &[&Term]),
    (other_levels, ys): (&[Universe], &[&Term]),
) -> Option<TypeEquality> {
    if levels.len() != other_levels.len()
        || levels.iter().zip(other_levels).any(|(l, m)| !l.equiv(m))
    {
        return None;
    }
    let universe = crate::type_checker::infer_sort(ctx, a).ok()?;
    let arity = crate::type_checker::infer_type(ctx, &family_head(family, levels)).ok()?;
    let small = |t: &Term| small(ctx, t, &universe);
    let (parts, dependent) = componentwise(ctx, arity, xs, ys, &small)?;
    Some(TypeEquality::Family { parts, dependent })
}

fn family_head(family: &str, levels: &[Universe]) -> Term {
    if levels.is_empty() {
        g(family)
    } else {
        Term::Const {
            name: family.to_string(),
            levels: levels.to_vec(),
        }
    }
}

/// An element of a type in `universe`, a universe below it, or a family of those over an element
/// type.
fn small(ctx: &Context, t: &Term, universe: &Universe) -> bool {
    if crate::type_checker::infer_sort(ctx, t).is_ok_and(|sort| sort.is_subtype_of(universe)) {
        return true;
    }
    match crate::reduction::normalize(ctx, t) {
        Term::Sort(level) => level.is_subtype_of(universe),
        Term::Pi {
            param,
            param_type,
            body_type,
        } => {
            crate::type_checker::infer_sort(ctx, &param_type)
                .is_ok_and(|sort| sort.is_subtype_of(universe))
                && small(&ctx.extend(&param, *param_type), &body_type, universe)
        }
        _ => false,
    }
}

/// Equality of two values of a family that is not a proposition: two different constructors
/// never, the same constructor by its fields. A numeral is read as its constructor.
fn values(ctx: &Context, ty: &Term, a: &Term, b: &Term) -> Option<Term> {
    if let Some(Former::Quotient(_, carrier)) = former(ctx, ty) {
        return classes(ctx, ty, &carrier, a, b);
    }
    let (family, arguments) = spine(ty);
    let (Term::Global(family) | Term::Const { name: family, .. }) = family else {
        return None;
    };
    if !ctx.is_inductive(family)
        || !crate::type_checker::infer_sort(ctx, ty)
            .ok()?
            .is_never_prop()
    {
        return None;
    }
    let (a, b) = (
        crate::reduction::constructor_form(ctx, a)?,
        crate::reduction::constructor_form(ctx, b)?,
    );
    let ((left, xs), (right, ys)) = (spine(&a), spine(&b));
    let (Term::Global(c) | Term::Const { name: c, .. }) = left else {
        return None;
    };
    let (Term::Global(d) | Term::Const { name: d, .. }) = right else {
        return None;
    };
    if ctx.constructor_inductive(c) != Some(family) || ctx.constructor_inductive(d) != Some(family)
    {
        return None;
    }
    if c != d {
        return Some(g("SFalse"));
    }
    let parameters = ctx.inductive_num_params(family);
    if arguments.len() < parameters || xs.len() < parameters || xs.len() != ys.len() {
        return None;
    }
    let telescope = peel(
        crate::type_checker::infer_type(ctx, left).ok()?,
        &arguments[..parameters],
    )?;
    let (parts, _) = componentwise(
        ctx,
        telescope,
        &xs[parameters..],
        &ys[parameters..],
        &|_| true,
    )?;
    Some(conjunction(parts))
}

/// The equalities of `xs` and `ys` read along one telescope, each at its own type, and whether any
/// is heterogeneous: a position whose type mentions no earlier position compares at that type;
/// one whose type does compares heterogeneously, `SSigma (T ~ T') (λ e. Obs_cast T T' e x ~ y)`, its
/// type taken at each side's own earlier positions. Which positions are dependent is read from
/// the telescope, never from the two instances, so the answer does not change as the arguments
/// reduce. `admit` vets each position's type on both sides; the telescope must be used up exactly.
fn componentwise(
    ctx: &Context,
    telescope: Term,
    xs: &[&Term],
    ys: &[&Term],
    admit: &dyn Fn(&Term) -> bool,
) -> Option<(Vec<Term>, bool)> {
    if xs.len() != ys.len() {
        return None;
    }
    let (mut generic, mut left, mut right) = (telescope.clone(), telescope.clone(), telescope);
    let mut earlier: Vec<String> = Vec::new();
    let mut parts = Vec::with_capacity(xs.len());
    let mut any_dependent = false;
    for (x, y) in xs.iter().zip(ys) {
        let Term::Pi {
            param: name,
            param_type: shape,
            body_type: rest,
        } = generic
        else {
            return None;
        };
        let Term::Pi {
            param: lp,
            param_type: lt,
            body_type: lb,
        } = left
        else {
            return None;
        };
        let Term::Pi {
            param: rp,
            param_type: rt,
            body_type: rb,
        } = right
        else {
            return None;
        };
        if !admit(&lt) || !admit(&rt) {
            return None;
        }
        let dependent = earlier
            .iter()
            .any(|n| crate::type_checker::occurs_free(&shape, n));
        any_dependent |= dependent;
        parts.push(if dependent {
            heterogeneous(ctx, &lt, &rt, x, y)?
        } else {
            obs(level_of(ctx, &lt)?, *lt, (*x).clone(), (*y).clone())
        });
        earlier.push(name);
        generic = *rest;
        left = substitute(&lb, &lp, x);
        right = substitute(&rb, &rp, y);
    }
    if matches!(left, Term::Pi { .. }) || matches!(right, Term::Pi { .. }) {
        return None;
    }
    Some((parts, any_dependent))
}

/// `SSigma (T ~ T') (λ e. Obs_cast T T' e x ~[T'] y)`.
fn heterogeneous(ctx: &Context, from: &Term, to: &Term, x: &Term, y: &Term) -> Option<Term> {
    let level = level_of(ctx, from)?;
    if !level.equiv(&level_of(ctx, to)?) {
        return None;
    }
    let types_equal = obs(
        level.succ(),
        Term::Sort(level.clone()),
        from.clone(),
        to.clone(),
    );
    let e = fresh("e", &[from, to, x, y]);
    let carried = app(
        cast_at(level.clone()),
        [from.clone(), to.clone(), v(&e), x.clone()],
    );
    Some(app(
        g("SSigma"),
        [
            types_equal.clone(),
            lam(&e, types_equal, obs(level, to.clone(), carried, y.clone())),
        ],
    ))
}

/// `Obs_cast.{u} A B e t`, for two types that are not propositions whose equality computes:
///
/// - along one universe, `t`;
/// - along two function types, the function that carries its argument back along the domains'
///   equality and its result forth along the codomains';
/// - along one family that has no indices and whose parameters depend on no earlier parameter,
///   with `t` a constructor whose fields depend on no earlier field, the same constructor at
///   `B`'s parameters with each field carried.
///
/// Every other cast is stuck: a cast along an equality of types that does not compute, along
/// evidence of `SFalse`, between propositions (a proof needs no carrying), or of a value that is
/// not yet a constructor.
fn carry(ctx: &Context, from: &Term, to: &Term, proof: &Term, value: &Term) -> Option<Term> {
    match type_equality(ctx, from, to)? {
        TypeEquality::Holds => Some(value.clone()),
        TypeEquality::Products {
            domain,
            domains,
            codomains,
        } => {
            let (
                Term::Pi {
                    param: p,
                    param_type: d,
                    body_type: c,
                },
                Term::Pi {
                    param: q,
                    param_type: e,
                    body_type: k,
                },
            ) = (from, to)
            else {
                return None;
            };
            let x = fresh(
                q,
                &[
                    d,
                    e,
                    &scoped(p, c),
                    &scoped(q, k),
                    proof,
                    value,
                    &domains,
                    &codomains,
                ],
            );
            let first = app(
                g("SSigma_fst"),
                [domains.clone(), codomains.clone(), proof.clone()],
            );
            let back = app(
                cast_at(domain),
                [(**e).clone(), (**d).clone(), first, v(&x)],
            );
            let left = substitute(c, p, &back);
            let right = if x == *q {
                (**k).clone()
            } else {
                substitute(k, q, &v(&x))
            };
            let inside = ctx.extend(&x, (**e).clone());
            let codomain = level_of(&inside, &left)?;
            if !codomain.equiv(&level_of(&inside, &right)?) {
                return None;
            }
            let second = app(g("SSigma_snd"), [domains, codomains, proof.clone(), v(&x)]);
            let image = Term::App(Box::new(value.clone()), Box::new(back));
            Some(lam(
                &x,
                (**e).clone(),
                app(cast_at(codomain), [left, right, second, image]),
            ))
        }
        TypeEquality::Family {
            parts,
            dependent: false,
        } => carry_value(ctx, from, to, &parts, proof, value),
        TypeEquality::Quotient { parts } => carry_class(ctx, from, to, &parts, proof, value),
        TypeEquality::Fails | TypeEquality::Family { .. } => None,
    }
}

/// `Obs_cast (Quot X R) (Quot Y S) e (Quot_mk X R x)` is `Quot_mk Y S (Obs_cast X Y e₀ x)`, `e₀` the carriers'
/// equality: the class of the carried representative.
fn carry_class(
    ctx: &Context,
    from: &Term,
    to: &Term,
    parts: &[Term],
    proof: &Term,
    value: &Term,
) -> Option<Term> {
    let (Former::Quotient(_, xs), Former::Quotient(_, ys)) = (former(ctx, from)?, former(ctx, to)?)
    else {
        return None;
    };
    let (constructor, arguments) = spine(value);
    let [_, _, x] = arguments.as_slice() else {
        return None;
    };
    if quotient_part(ctx, constructor) != Some(QuotientPart::Class) {
        return None;
    }
    let carriers = components(parts, proof).into_iter().next()?;
    let representative = app(
        cast_at(level_of(ctx, xs[0])?),
        [xs[0].clone(), ys[0].clone(), carriers, (*x).clone()],
    );
    Some(app(
        constructor.clone(),
        [ys[0].clone(), ys[1].clone(), representative],
    ))
}

/// `Obs_cast (I p̄) (I q̄) e (C p̄ x̄)` is `C q̄ (Obs_cast T₀[p̄] T₀[q̄] h₀ x₀) …`, each `hₖ : Tₖ[p̄] ~ Tₖ[q̄]`
/// built from `e`'s components by one `Obs_transp` per parameter, moving `Tₖ`'s parameters from `p̄` to
/// `q̄` one at a time.
fn carry_value(
    ctx: &Context,
    from: &Term,
    to: &Term,
    parts: &[Term],
    proof: &Term,
    value: &Term,
) -> Option<Term> {
    let (Former::Family(family, levels, ps), Former::Family(_, _, qs)) =
        (former(ctx, from)?, former(ctx, to)?)
    else {
        return None;
    };
    if ctx.inductive_num_indices(family) != 0 || ps.len() != parts.len() || qs.len() != ps.len() {
        return None;
    }
    let value = crate::reduction::constructor_form(ctx, value)?;
    let (constructor, arguments) = spine(&value);
    let (Term::Global(c) | Term::Const { name: c, .. }) = constructor else {
        return None;
    };
    if ctx.constructor_inductive(c) != Some(family) || arguments.len() < ps.len() {
        return None;
    }
    let fields = &arguments[ps.len()..];
    let telescope = crate::type_checker::infer_type(ctx, constructor).ok()?;
    if !fields_independent(&telescope, ps.len()) {
        return None;
    }
    let arity = crate::type_checker::infer_type(ctx, &family_head(family, levels)).ok()?;
    let parameter_types = telescope_types(arity, &ps)?;
    let components = components(parts, proof);
    let owned = |xs: &[&Term]| xs.iter().map(|x| (*x).clone()).collect::<Vec<Term>>();
    let (ps, qs) = (owned(&ps), owned(&qs));
    let mut carried = Vec::with_capacity(fields.len());
    for k in 0..fields.len() {
        let source = field_type(&telescope, &ps, fields, k)?;
        let target = field_type(&telescope, &qs, fields, k)?;
        let level = level_of(ctx, &source)?;
        if !level.equiv(&level_of(ctx, &target)?) {
            return None;
        }
        let mut evidence = app(
            Term::Const {
                name: "Obs_refl".to_string(),
                levels: vec![level.succ()],
            },
            [Term::Sort(level.clone()), source.clone()],
        );
        for (i, parameter_type) in parameter_types.iter().enumerate() {
            let mut avoid: Vec<&Term> =
                ps.iter().chain(&qs).chain(fields.iter().copied()).collect();
            avoid.push(&telescope);
            let b = fresh("b", &avoid);
            let h = fresh("h", &[&avoid[..], &[&v(&b)]].concat());
            let mixed: Vec<Term> = qs[..i]
                .iter()
                .cloned()
                .chain([v(&b)])
                .chain(ps[i + 1..].iter().cloned())
                .collect();
            let moved = field_type(&telescope, &mixed, fields, k)?;
            let parameter = level_of(ctx, parameter_type)?;
            let motive = lam(
                &b,
                parameter_type.clone(),
                lam(
                    &h,
                    obs(
                        parameter.clone(),
                        parameter_type.clone(),
                        ps[i].clone(),
                        v(&b),
                    ),
                    obs(
                        level.succ(),
                        Term::Sort(level.clone()),
                        source.clone(),
                        moved,
                    ),
                ),
            );
            evidence = app(
                Term::Const {
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
        carried.push(app(
            cast_at(level),
            [source, target, evidence, fields[k].clone()],
        ));
    }
    Some(app(constructor.clone(), qs.into_iter().chain(carried)))
}

/// Whether no field of a constructor's telescope (after its `parameters`) mentions an earlier field.
fn fields_independent(telescope: &Term, parameters: usize) -> bool {
    let mut cur = telescope;
    for _ in 0..parameters {
        let Term::Pi { body_type, .. } = cur else {
            return false;
        };
        cur = body_type;
    }
    let mut earlier: Vec<&str> = Vec::new();
    while let Term::Pi {
        param,
        param_type,
        body_type,
    } = cur
    {
        if earlier
            .iter()
            .any(|n| crate::type_checker::occurs_free(param_type, n))
        {
            return false;
        }
        earlier.push(param);
        cur = body_type;
    }
    true
}

/// The types of the first `values.len()` positions of `telescope`, each at the earlier values.
fn telescope_types(telescope: Term, values: &[&Term]) -> Option<Vec<Term>> {
    let mut cur = telescope;
    let mut types = Vec::with_capacity(values.len());
    for value in values {
        let Term::Pi {
            param,
            param_type,
            body_type,
        } = cur
        else {
            return None;
        };
        types.push(*param_type);
        cur = substitute(&body_type, &param, value);
    }
    Some(types)
}

/// `telescope` past its first positions, instantiated at `values`.
fn peel(telescope: Term, values: &[&Term]) -> Option<Term> {
    let mut cur = telescope;
    for value in values {
        let Term::Pi {
            param, body_type, ..
        } = cur
        else {
            return None;
        };
        cur = substitute(&body_type, &param, value);
    }
    Some(cur)
}

/// The type of field `k` of a constructor `telescope`, at `parameters` and the earlier `fields`.
fn field_type(telescope: &Term, parameters: &[Term], fields: &[&Term], k: usize) -> Option<Term> {
    let parameters: Vec<&Term> = parameters.iter().collect();
    let cur = peel(peel(telescope.clone(), &parameters)?, &fields[..k])?;
    let Term::Pi { param_type, .. } = cur else {
        return None;
    };
    Some(*param_type)
}

/// Each component of `proof : SAnd parts₀ (SAnd parts₁ …)`, read out by `SAnd_left`/`SAnd_right`.
fn components(parts: &[Term], proof: &Term) -> Vec<Term> {
    let mut out = Vec::with_capacity(parts.len());
    let mut rest = proof.clone();
    for (j, part) in parts.iter().enumerate() {
        if j + 1 == parts.len() {
            out.push(rest);
            break;
        }
        let tail = conjunction(parts[j + 1..].to_vec());
        out.push(app(
            g("SAnd_left"),
            [part.clone(), tail.clone(), rest.clone()],
        ));
        rest = app(g("SAnd_right"), [part.clone(), tail, rest]);
    }
    out
}

/// `STrue` for none, the one for one, else right-nested `SAnd`.
fn conjunction(mut parts: Vec<Term>) -> Term {
    let Some(last) = parts.pop() else {
        return g("STrue");
    };
    parts
        .into_iter()
        .rev()
        .fold(last, |rest, part| app(g("SAnd"), [part, rest]))
}

/// The level a type lives at, as a universe argument (`SProp` read as `Prop`).
fn level_of(ctx: &Context, ty: &Term) -> Option<Universe> {
    crate::type_checker::infer_sort(ctx, ty)
        .ok()
        .map(|sort| sort.least_level())
}

fn obs(level: Universe, ty: Term, a: Term, b: Term) -> Term {
    app(obs_at(level), [ty, a, b])
}

fn cast_at(level: Universe) -> Term {
    Term::Const {
        name: "Obs_cast".to_string(),
        levels: vec![level],
    }
}

fn arrow(a: &Term, b: &Term) -> Term {
    pi(&fresh("_", &[b]), a.clone(), b.clone())
}

fn lam(param: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: param.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}

fn substitute(body: &Term, var: &str, replacement: &Term) -> Term {
    crate::type_checker::substitute(body, var, replacement)
}

/// The head and argument spine of an application.
fn spine(t: &Term) -> (&Term, Vec<&Term>) {
    let mut args = Vec::new();
    let mut cur = t;
    while let Term::App(f, a) = cur {
        args.push(a.as_ref());
        cur = f.as_ref();
    }
    args.reverse();
    (cur, args)
}

/// `body` under a binder for `param`, so a fresh-name search sees every name free in `body` except
/// `param` itself.
fn scoped(param: &str, body: &Term) -> Term {
    lam(param, Term::Sort(Universe::Prop), body.clone())
}

/// `base`, or `base` with a numeral, free in none of `terms`.
fn fresh(base: &str, terms: &[&Term]) -> String {
    let taken = |n: &str| terms.iter().any(|t| crate::type_checker::occurs_free(t, n));
    if !taken(base) {
        return base.to_string();
    }
    (1..)
        .map(|i| format!("{base}{i}"))
        .find(|n| !taken(n))
        .expect("a fresh name")
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/observational/tests.rs"]
pub(crate) mod tests;
