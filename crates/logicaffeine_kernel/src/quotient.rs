//! Certified universe-polymorphic quotient primitives for kernel importers.
use crate::{Context, KernelError, Term, Universe};

mod prelude;
pub(crate) use prelude::{kind as prelude_kind, same_base as same_prelude_base};
pub(crate) use prelude::{PreludeCertificate, PreludeKind};
pub(crate) fn register_prelude(ctx: &mut Context) -> Result<(), KernelError> {
    prelude::register(ctx)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Type,
    Constructor,
    Lift,
    Induction,
}
impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Type => "kernel.quot.type",
            Self::Constructor => "kernel.quot.mk",
            Self::Lift => "kernel.quot.lift",
            Self::Induction => "kernel.quot.ind",
        }
    }
    pub fn arity(self) -> usize {
        if self == Self::Lift {
            2
        } else {
            1
        }
    }
}
#[derive(Debug, Clone)]
pub(crate) struct Certificate {
    pub kind: Kind,
    pub dependencies: Vec<String>,
    /// The lift's: the equality its respect hypothesis is stated with.
    pub equality: Option<String>,
}
fn v(n: &str) -> Term {
    Term::Var(format!("quot.{n}"))
}
fn pi(n: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: format!("quot.{n}"),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn app(f: Term, args: impl IntoIterator<Item = Term>) -> Term {
    args.into_iter()
        .fold(f, |f, a| Term::App(Box::new(f), Box::new(a)))
}
fn constant(name: &str, levels: Vec<Universe>) -> Term {
    Term::Const {
        name: name.into(),
        levels,
    }
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn error(message: &str) -> KernelError {
    KernelError::CertificationError(message.into())
}

/// The canonical Lean signature, with explicit universe instances throughout.
pub fn signature(kind: Kind, levels: &[Universe], equality: &str) -> Result<Term, KernelError> {
    if levels.len() != kind.arity() {
        return Err(error("quotient universe arity mismatch"));
    }
    let u = levels[0].clone();
    let carrier = || Term::Sort(u.clone());
    let relation = || pi("x", v("A"), pi("y", v("A"), prop()));
    let quotient = || {
        app(
            constant(Kind::Type.name(), vec![u.clone()]),
            [v("A"), v("r")],
        )
    };
    let mk = |x| {
        app(
            constant(Kind::Constructor.name(), vec![u.clone()]),
            [v("A"), v("r"), x],
        )
    };
    let body = match kind {
        Kind::Type => carrier(),
        Kind::Constructor => pi("a", v("A"), quotient()),
        Kind::Induction => pi(
            "P",
            pi("q", quotient(), prop()),
            pi(
                "f",
                pi("a", v("A"), app(v("P"), [mk(v("a"))])),
                pi("q", quotient(), app(v("P"), [v("q")])),
            ),
        ),
        Kind::Lift => {
            let level_v = levels[1].clone();
            let respect = pi(
                "a",
                v("A"),
                pi(
                    "b",
                    v("A"),
                    pi(
                        "h",
                        app(v("r"), [v("a"), v("b")]),
                        app(
                            constant(equality, vec![level_v.clone()]),
                            [v("B"), app(v("f"), [v("a")]), app(v("f"), [v("b")])],
                        ),
                    ),
                ),
            );
            pi(
                "B",
                Term::Sort(level_v),
                pi(
                    "f",
                    pi("a", v("A"), v("B")),
                    pi("respect", respect, pi("q", quotient(), v("B"))),
                ),
            )
        }
    };
    Ok(pi("A", carrier(), pi("r", relation(), body)))
}

fn canonical_signature(actual: &Term, expected: &Term) -> bool {
    let mut store = crate::term_store::TermStore::new();
    let actual = store.intern(actual);
    let expected = store.intern(expected);
    store.alpha_same(actual, expected)
}

fn equality_dependencies(ctx: &Context, name: &str) -> Result<Vec<String>, KernelError> {
    let Some((params, ty)) = ctx.poly_declaration(name) else {
        return Err(error(
            "quotient lift requires a polymorphic equality inductive",
        ));
    };
    let constructors = ctx.get_constructors(name);
    if params.len() != 1
        || !ctx.is_inductive(name)
        || ctx.inductive_num_params(name) != 2
        || ctx.inductive_num_indices(name) != 1
        || constructors.len() != 1
    {
        return Err(error("quotient lift requires canonical equality"));
    }
    let u = Universe::Var(params[0].clone());
    let expected = pi(
        "A",
        Term::Sort(u.clone()),
        pi("a", v("A"), pi("b", v("A"), prop())),
    );
    if !canonical_signature(ty, &expected) {
        return Err(error(
            "quotient equality type differs from canonical equality",
        ));
    }
    let (ctor, _) = constructors[0];
    let Some((cp, ct)) = ctx.poly_declaration(ctor) else {
        return Err(error(
            "quotient equality constructor lacks universe telescope",
        ));
    };
    if cp.len() != 1 {
        return Err(error("quotient equality constructor universe arity"));
    }
    let cu = Universe::Var(cp[0].clone());
    let expected_ctor = pi(
        "A",
        Term::Sort(cu.clone()),
        pi(
            "a",
            v("A"),
            app(constant(name, vec![cu]), [v("A"), v("a"), v("a")]),
        ),
    );
    if !canonical_signature(ct, &expected_ctor) {
        return Err(error(
            "quotient equality constructor differs from reflexivity",
        ));
    }
    Ok(vec![name.into(), ctor.into()])
}

/// Install only a canonical primitive, never an arbitrary trusted input type.
/// The importer separately compares its serialized signature with this one.
pub fn install(
    ctx: &mut Context,
    kind: Kind,
    params: Vec<String>,
    equality: &str,
) -> Result<(), KernelError> {
    let levels = params
        .iter()
        .cloned()
        .map(Universe::Var)
        .collect::<Vec<_>>();
    let ty = signature(kind, &levels, equality)?;
    let mut dependencies = vec![kind.name().into()];
    if kind != Kind::Type {
        if ctx
            .quotient_primitives
            .get(Kind::Type.name())
            .map(|c| c.kind)
            != Some(Kind::Type)
        {
            return Err(error(
                "quotient type must be installed before its primitives",
            ));
        }
        dependencies.push(Kind::Type.name().into());
    }
    if kind == Kind::Induction {
        if ctx
            .quotient_primitives
            .get(Kind::Constructor.name())
            .map(|c| c.kind)
            != Some(Kind::Constructor)
        {
            return Err(error("quotient constructor must precede induction"));
        }
        dependencies.push(Kind::Constructor.name().into());
    }
    if kind == Kind::Lift {
        dependencies.extend(equality_dependencies(ctx, equality)?);
    }
    ctx.add_universe_poly_declaration(kind.name(), params, ty)?;
    let equality = (kind == Kind::Lift).then(|| equality.to_string());
    ctx.record(
        |c| &mut c.quotient_primitives,
        kind.name().into(),
        Certificate {
            kind,
            dependencies,
            equality,
        },
    );
    Ok(())
}

/// Resolve a certified identity only at a well-formed universe instance.
pub(crate) fn kind(ctx: &Context, name: &str, levels: &[Universe]) -> Option<Kind> {
    let certificate = ctx.quotient_primitives.get(name)?;
    let (params, ty) = ctx.poly_declaration(name)?;
    crate::term::resolve_const_levels(name, Some(params), ty, levels).ok()?;
    Some(certificate.kind)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/quotient/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/quotient/prelude_identity_controls.rs"]
mod prelude_identity_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/quotient/recursor_definition_controls.rs"]
mod recursor_definition_controls;
