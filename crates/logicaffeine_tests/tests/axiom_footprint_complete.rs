use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{Context, Term, Universe};
use logicaffeine_proof::verify::VerifiedProof;

fn global(name: &str) -> Term {
    Term::Global(name.into())
}

/// A context in which one assumption, `Assumed`, is reachable only through a definition's TYPE
/// and only through a constructor's TYPE. Every fixture name is fresh: the prelude owns `witness`
/// (the `Ex` constructor its own theorems rest on), so a fixture that re-declared it would be
/// refused by the stable-meaning gate and would silently stop being the context it describes.
fn context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.add_declaration("Assumed", Term::Sort(Universe::Prop));
    ctx.add_declaration("assumedWitness", global("Assumed"));
    ctx.add_definition(
        "throughType".into(),
        global("Assumed"),
        global("assumedWitness"),
    );
    ctx.add_inductive("Wrapped", Term::Sort(Universe::Prop));
    ctx.add_constructor(
        "wrap",
        "Wrapped",
        Term::Pi {
            param: "_".into(),
            param_type: Box::new(global("Assumed")),
            body_type: Box::new(global("Wrapped")),
        },
    );
    assert!(
        ctx.refused_declarations().is_empty(),
        "the fixture must register exactly as written, or the footprint is measured on a \
         different context than the one each test names: {:?}",
        ctx.refused_declarations()
    );
    ctx
}

fn introduced(ctx: Context, term: Term) -> Vec<String> {
    VerifiedProof {
        derivation: None,
        proof_term: Some(term),
        kernel_ctx: ctx,
        verified: true,
        verification_error: None,
    }
    .axiom_footprint()
    .introduced
}

#[test]
fn definition_types_contribute_to_the_proof_footprint() {
    assert_eq!(
        introduced(context(), global("throughType")),
        vec!["Assumed", "assumedWitness"]
    );
}

/// Why the fixture cannot use the name `witness`: the prelude's `Ex` constructor carries it, and a
/// name the prelude's theorems rest on is never re-declared under them. The refusal is what keeps
/// the footprint of every prelude theorem that cites `witness` meaning what it meant.
#[test]
fn the_fixture_names_are_its_own_and_the_preludes_witness_is_not_rebound() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    assert!(
        ctx.is_constructor("witness"),
        "`witness` is the prelude's `Ex` constructor"
    );
    let before = ctx.get_global("witness").cloned();
    ctx.add_declaration("Assumed", Term::Sort(Universe::Prop));
    ctx.add_declaration("witness", global("Assumed"));
    assert!(
        ctx.refused_declarations()
            .iter()
            .any(|(name, why)| name == "witness" && why.starts_with("Stable meaning violation")),
        "re-declaring the prelude's `witness` must be refused as a change of meaning: {:?}",
        ctx.refused_declarations()
    );
    assert_eq!(
        ctx.get_global("witness").cloned(),
        before,
        "the refused declaration must leave the constructor's type exactly as it was"
    );
    for name in [
        "Assumed",
        "assumedWitness",
        "throughType",
        "Wrapped",
        "wrap",
    ] {
        let mut prelude = Context::new();
        StandardLibrary::register(&mut prelude);
        assert!(
            prelude.get_global(name).is_none(),
            "`{name}` must be a fixture name the prelude does not own"
        );
    }
}

#[test]
fn constructor_types_contribute_to_the_proof_footprint() {
    assert_eq!(introduced(context(), global("wrap")), vec!["Assumed"]);
}

#[test]
fn inductive_names_include_their_constructor_dependencies() {
    assert_eq!(introduced(context(), global("Wrapped")), vec!["Assumed"]);
}

#[test]
fn unresolved_names_are_reported_as_introduced() {
    assert_eq!(introduced(context(), global("missing")), vec!["missing"]);
}
