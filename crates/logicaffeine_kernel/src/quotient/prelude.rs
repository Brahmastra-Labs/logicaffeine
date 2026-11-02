//! The prelude's quotient family, registered as Lean's `init_quot` registers it: four primitive
//! identities (`Quot`, `Quot_mk`, `Quot_lift`, `Quot_ind`), the `Quot_sound` axiom, and the
//! dependent eliminator `Quot_rec` DEFINED from them. Registration owns the fixed signatures;
//! copying a spelling or signature through an assumption door grants no identity.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PreludeKind {
    Type,
    Constructor,
    Lift,
    Induction,
}
#[derive(Debug)]
pub(crate) struct PreludeCertificate {
    pub(crate) dependencies: BTreeSet<String>,
    headers: BTreeMap<String, Term>,
}
impl PreludeCertificate {
    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.headers.keys().map(String::as_str)
    }
}
pub(crate) fn kind(ctx: &Context, name: &str) -> Option<PreludeKind> {
    let certificate = ctx.prelude_quotient.as_ref()?;
    // The record is minted only by register below and revoked on dependencies.
    certificate.headers.get(name)?;
    match name {
        "Quot" => Some(PreludeKind::Type),
        "Quot_mk" => Some(PreludeKind::Constructor),
        "Quot_lift" => Some(PreludeKind::Lift),
        "Quot_ind" => Some(PreludeKind::Induction),
        // `Quot_sound` is an axiom; `Quot_rec` is a checked definition and has no header here.
        _ => None,
    }
}
pub(crate) fn same_base(a: &Context, b: &Context) -> bool {
    match (&a.prelude_quotient, &b.prelude_quotient) {
        (Some(a), Some(b)) => a.headers == b.headers && a.dependencies == b.dependencies,
        _ => false,
    }
}
fn gl(name: &str) -> Term {
    Term::Global(name.into())
}
fn apps_(f: Term, args: &[Term]) -> Term {
    app(f, args.iter().cloned())
}
fn lam_(param: &str, param_type: Term, body: Term) -> Term {
    Term::Lambda {
        param: param.into(),
        param_type: Box::new(param_type),
        body: Box::new(body),
    }
}
fn declare(ctx: &mut Context, headers: &mut BTreeMap<String, Term>, name: &str, ty: Term) {
    ctx.add_declaration(name, ty.clone());
    headers.insert(name.into(), ty);
}
fn attest(ctx: &mut Context, headers: BTreeMap<String, Term>) -> Result<(), KernelError> {
    // These expected headers were constructed by the private fixed registration
    // code, never supplied by a caller or read from untrusted declarations.
    for (name, expected) in &headers {
        if ctx.get_global(name) != Some(expected)
            || ctx.get_definition_body(name).is_some()
            || ctx.poly_declaration(name).is_some()
            || ctx.is_inductive(name)
            || ctx.is_constructor(name)
            || ctx.has_pending_dependency(&gl(name))
        {
            return Err(error(
                "prelude quotient registration did not install its exact declaration",
            ));
        }
        crate::type_checker::infer_sort(ctx, expected)?;
    }
    let u = Universe::Var("u".into());
    let expected_eq = pi(
        "A",
        Term::Sort(u.clone()),
        pi("a", v("A"), pi("b", v("A"), prop())),
    );
    let expected_refl = pi(
        "A",
        Term::Sort(u),
        pi("a", v("A"), app(gl("Eq"), [v("A"), v("a"), v("a")])),
    );
    let constructors = ctx.get_constructors("Eq");
    if !ctx.is_inductive("Eq")
        || ctx.poly_declaration("Eq").is_some()
        || ctx.inductive_num_params("Eq") != 2
        || ctx.inductive_num_indices("Eq") != 1
        || constructors.len() != 1
        || constructors[0].0 != "refl"
        || !ctx
            .get_global("Eq")
            .is_some_and(|ty| canonical_signature(ty, &expected_eq))
        || !canonical_signature(constructors[0].1, &expected_refl)
        || ctx.get_definition_body("Eq_rec_dep").is_none()
        || ctx.has_pending_dependency(&gl("Eq_rec_dep"))
    {
        return Err(error(
            "prelude quotient registration requires its checked equality interface",
        ));
    }
    // The usual dependency tracer follows actual definition bodies, declared
    // types and constructor inventories. Keep all discovered names for revocation.
    let mut dependencies = BTreeSet::from(["Eq".into(), "refl".into(), "Eq_rec_dep".into()]);
    dependencies.extend(headers.keys().cloned());
    for ty in headers.values() {
        let trace = crate::prelude::checked_opaque_dependencies(ctx, ty, ty);
        if !trace.checked.usage.unresolved.is_empty() {
            return Err(error(
                "prelude quotient interface has an unresolved dependency",
            ));
        }
        dependencies.extend(trace.checked.dependencies);
        for (name, edges) in trace.graph {
            dependencies.insert(name);
            dependencies.extend(edges);
        }
    }
    ctx.certify_prelude_quotient(PreludeCertificate {
        dependencies,
        headers,
    });
    Ok(())
}

pub(super) fn register(ctx: &mut Context) -> Result<(), KernelError> {
    let mut headers = BTreeMap::new();
    // The CARRIER's sort is a universe VARIABLE, as Lean's `Quot {α : Sort u}` is. A quotient
    // of a `Type 0` carrier lives in `Type 1`, so the codomain carries the same `u` — pinning
    // either to `Type 0` makes quotienting a type (a `Setoid` on `Type`, a quotient category,
    // `Cardinal`) unstateable rather than merely unprovable.
    //
    // `Quot_lift`'s target is a SECOND level variable, Lean's `{β : Sort v}`. The dependent
    // eliminator is `Quot_lift` into a dependent pair over the quotient, and that pair lives in
    // `Sort (max 1 u v)`: with the carrier a variable, no fixed target can hold it.
    let sort_u = || Term::Sort(Universe::Var("u".to_string()));
    let sort_v = || Term::Sort(Universe::Var("v".to_string()));
    let prop = || Term::Sort(Universe::Prop);
    let g = |s: &str| Term::Global(s.to_string());
    let v = |s: &str| Term::Var(s.to_string());
    let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
    let pi = |p: &str, t: Term, b: Term| Term::Pi {
        param: p.to_string(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    };
    let arrow = |t: Term, b: Term| pi("_", t, b);
    // r : A → A → Prop
    let rel = || arrow(v("A"), arrow(v("A"), prop()));
    let quot = |a: Term, r: Term| ap(ap(g("Quot"), a), r);
    let mk = |a: Term, r: Term, x: Term| ap(ap(ap(g("Quot_mk"), a), r), x);
    let eq3 = |t: Term, x: Term, y: Term| ap(ap(ap(g("Eq"), t), x), y);

    // Quot.{u} : Π(A : Sort u). (A → A → Prop) → Sort u
    declare(
        ctx,
        &mut headers,
        "Quot",
        pi("A", sort_u(), arrow(rel(), sort_u())),
    );
    // Quot_mk.{u} : Π(A : Sort u). Π(r:A→A→Prop). A → Quot A r
    declare(
        ctx,
        &mut headers,
        "Quot_mk",
        pi(
            "A",
            sort_u(),
            pi("r", rel(), arrow(v("A"), quot(v("A"), v("r")))),
        ),
    );
    // Quot_lift.{u,v} : Π(A : Sort u). Π(r). Π(B : Sort v). Π(f:A→B).
    //   Π(h: Π(a b:A). r a b → Eq B (f a) (f b)). Quot A r → B
    let resp = pi(
        "a",
        v("A"),
        pi(
            "b",
            v("A"),
            arrow(
                ap(ap(v("r"), v("a")), v("b")),
                eq3(v("B"), ap(v("f"), v("a")), ap(v("f"), v("b"))),
            ),
        ),
    );
    declare(
        ctx,
        &mut headers,
        "Quot_lift",
        pi(
            "A",
            sort_u(),
            pi(
                "r",
                rel(),
                pi(
                    "B",
                    sort_v(),
                    pi(
                        "f",
                        arrow(v("A"), v("B")),
                        pi("h", resp, arrow(quot(v("A"), v("r")), v("B"))),
                    ),
                ),
            ),
        ),
    );
    // Quot_sound : Π(A). Π(r). Π(a b:A). r a b → Eq (Quot A r) (Quot_mk A r a) (Quot_mk A r b)
    declare(
        ctx,
        &mut headers,
        "Quot_sound",
        pi(
            "A",
            sort_u(),
            pi(
                "r",
                rel(),
                pi(
                    "a",
                    v("A"),
                    pi(
                        "b",
                        v("A"),
                        arrow(
                            ap(ap(v("r"), v("a")), v("b")),
                            eq3(
                                quot(v("A"), v("r")),
                                mk(v("A"), v("r"), v("a")),
                                mk(v("A"), v("r"), v("b")),
                            ),
                        ),
                    ),
                ),
            ),
        ),
    );
    // Quot_ind : Π(A). Π(r). Π(P: Quot A r → Prop).
    //   (Π(a:A). P (Quot_mk A r a)) → Π(q: Quot A r). P q
    declare(
        ctx,
        &mut headers,
        "Quot_ind",
        pi(
            "A",
            sort_u(),
            pi(
                "r",
                rel(),
                pi(
                    "P",
                    arrow(quot(v("A"), v("r")), prop()),
                    arrow(
                        pi("a", v("A"), ap(v("P"), mk(v("A"), v("r"), v("a")))),
                        pi("q", quot(v("A"), v("r")), ap(v("P"), v("q"))),
                    ),
                ),
            ),
        ),
    );
    attest(ctx, headers)?;
    define_recursor(ctx)
}

/// `Quot_rec`'s statement — the DEPENDENT eliminator into `Type 0`, with the respect obligation
/// stated up to TRANSPORT:
///
/// ```text
/// Quot_rec : Π(A : Sort u)(r). Π(motive : Quot A r → Type 0).
///            Π(f : Π(a:A). motive (Quot_mk A r a)).
///            Π(h : Π(a b:A). Π(p : r a b).
///                  Eq (motive (Quot_mk A r b))
///                     (transport (Quot_sound A r a b p) (f a))
///                     (f b)).
///            Π(q : Quot A r). motive q
/// ```
///
/// The respect obligation cannot be `Eq ? (f a) (f b)`: `f a : motive (Quot_mk A r a)` and
/// `f b : motive (Quot_mk A r b)` live in DIFFERENT types, so there is no carrier to state the
/// equation at until `f a` has been transported along `Quot_sound`. The transport is the
/// prelude's derived `Eq_rec_dep`, so the obligation says exactly what Lean's does.
pub(super) fn recursor_statement() -> Term {
    let sort_u = || Term::Sort(Universe::Var("u".to_string()));
    let prop = || Term::Sort(Universe::Prop);
    let v = |s: &str| Term::Var(s.to_string());
    let ap = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
    let pi = |p: &str, t: Term, b: Term| Term::Pi {
        param: p.to_string(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    };
    let arrow = |t: Term, b: Term| pi("_", t, b);
    let rel = || arrow(v("A"), arrow(v("A"), prop()));
    let quot = || apps_(gl("Quot"), &[v("A"), v("r")]);
    let mk = |x: Term| apps_(gl("Quot_mk"), &[v("A"), v("r"), x]);
    let eq3 = |t: Term, x: Term, y: Term| apps_(gl("Eq"), &[t, x, y]);
    let motive_at = |q: Term| ap(v("motive"), q);
    let transported = apps_(
        gl("Eq_rec_dep"),
        &[
            quot(),
            mk(v("a")),
            lam_(
                "q",
                quot(),
                lam_("_", eq3(quot(), mk(v("a")), v("q")), motive_at(v("q"))),
            ),
            ap(v("f"), v("a")),
            mk(v("b")),
            apps_(gl("Quot_sound"), &[v("A"), v("r"), v("a"), v("b"), v("p")]),
        ],
    );
    let respect = pi(
        "a",
        v("A"),
        pi(
            "b",
            v("A"),
            pi(
                "p",
                ap(ap(v("r"), v("a")), v("b")),
                eq3(motive_at(mk(v("b"))), transported, ap(v("f"), v("b"))),
            ),
        ),
    );
    pi(
        "A",
        sort_u(),
        pi(
            "r",
            rel(),
            pi(
                "motive",
                arrow(quot(), Term::Sort(Universe::Type(0))),
                pi(
                    "f",
                    pi("a", v("A"), motive_at(mk(v("a")))),
                    pi("h", respect, pi("q", quot(), motive_at(v("q")))),
                ),
            ),
        ),
    )
}

/// `Quot_rec` — DEFINED, as Lean defines `Quot.rec` (`Init/Core.lean:1581-1611`): `Quot_lift`
/// into the dependent pair `PSigma (Quot A r) motive`, `Quot_ind` for the pair's first
/// projection, and a transport along it ([`crate::recursor::derive_quot_rec`]). The registration
/// gateway checks the derived body against the fixed statement above, so the eliminator adds no
/// assumption: its axiom closure is `Quot_sound`, read through the body, as Lean's
/// `#print axioms Quot.rec` is. It computes by δ, `Quot_lift`'s ι-rule, the pair's projections
/// and `Eq_rec_dep` at `refl` — no rule of its own.
///
/// The name then joins the family's revocation set with everything its body reads. The
/// certificate attests this registration as a unit, so displacing the derived eliminator revokes
/// the primitive identities exactly as displacing a primitive does.
fn define_recursor(ctx: &mut Context) -> Result<(), KernelError> {
    let statement = recursor_statement();
    let body = crate::recursor::quot_rec_body(ctx)?;
    ctx.declare_checked_auxiliary("Quot_rec", statement.clone(), body.clone())?;
    let trace = crate::prelude::checked_opaque_dependencies(ctx, &statement, &body);
    if !trace.checked.usage.unresolved.is_empty() {
        return Err(error(
            "the derived quotient eliminator has an unresolved dependency",
        ));
    }
    let certificate = ctx.prelude_quotient.clone().ok_or_else(|| {
        error("the derived quotient eliminator revoked the family it is built from")
    })?;
    let mut dependencies = certificate.dependencies.clone();
    dependencies.insert("Quot_rec".into());
    dependencies.extend(trace.checked.dependencies);
    for (name, edges) in trace.graph {
        dependencies.insert(name);
        dependencies.extend(edges);
    }
    ctx.certify_prelude_quotient(PreludeCertificate {
        dependencies,
        headers: certificate.headers.clone(),
    });
    Ok(())
}
