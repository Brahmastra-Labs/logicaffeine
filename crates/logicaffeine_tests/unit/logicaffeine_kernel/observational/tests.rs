//! The observational equality's certificate: what revokes it, what an impostor gets, how its
//! constants are accounted.
use super::*;
use crate::interface::TermParser;
use crate::prelude::{axiom_dependencies, StandardLibrary};
use std::collections::BTreeSet;

fn term(text: &str) -> Term {
    TermParser::parse(text).unwrap_or_else(|e| panic!("`{text}` parses: {e:?}"))
}

fn prop() -> Term {
    Term::Sort(Universe::Prop)
}

fn certified() -> Context {
    let mut ctx = StandardLibrary::context().clone();
    register(&mut ctx).expect("registers");
    ctx
}

const FUNEXT: &str = "(fun (wanted : forall (A : Type) (B : Type) (f : A -> B) (g : A -> B), \
                      (forall (x : A), Obs.{1} B (f x) (g x)) -> Obs.{1} (A -> B) f g) => wanted) \
                      (fun (A : Type) (B : Type) (f : A -> B) (g : A -> B) \
                      (h : forall (x : A), Obs.{1} B (f x) (g x)) => h)";

/// Whether each kernel accepts function extensionality by conversion.
fn funext(ctx: &Context) -> (bool, bool) {
    let probe = term(FUNEXT);
    (
        crate::infer_type(ctx, &probe).is_ok(),
        crate::recheck(ctx, &probe).is_ok(),
    )
}

/// Every constant declared at its exact signature, the certificate never installed: the same
/// spelling without the certificate's door.
fn impostor() -> Context {
    let mut ctx = StandardLibrary::context().clone();
    declare(&mut ctx).expect("declares");
    ctx
}

#[test]
fn the_certified_context_computes_in_both_kernels() {
    assert_eq!(funext(&certified()), (true, true));
}

#[test]
fn replacing_any_name_the_certificate_rests_on_revokes_it() {
    let dependencies = certified()
        .observational
        .clone()
        .expect("certified")
        .dependencies
        .clone();
    for name in &dependencies {
        let mut ctx = certified();
        ctx.permit_meaning_changes_for_cache_tests();
        ctx.add_declaration(name, prop());
        assert!(
            ctx.observational.is_none(),
            "replacing `{name}` revokes the certificate"
        );
        assert_eq!(
            kind(&ctx, "Obs"),
            None,
            "replacing `{name}`: `Obs` is no longer certified"
        );
    }
}

#[test]
fn a_constructor_added_to_an_empty_strict_proposition_revokes_the_certificate() {
    let mut ctx = certified();
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.add_constructor("sfalse_intro", "SFalse", Term::Global("SFalse".into()));
    assert!(ctx.observational.is_none());
    assert_eq!(
        funext(&ctx),
        (false, false),
        "nothing computes once the certificate is revoked"
    );
}

#[test]
fn constants_of_the_same_spelling_and_signature_without_the_certificate_do_not_compute() {
    let ctx = impostor();
    assert!(ctx.observational.is_none());
    assert_eq!(kind(&ctx, "Obs"), None);
    assert_eq!(
        funext(&ctx),
        (false, false),
        "an uncertified `Obs` is an opaque assumption in both kernels"
    );
}

#[test]
fn revoking_one_contexts_certificate_leaves_a_clones_intact() {
    let original = certified();
    let mut changed = original.clone();
    changed.permit_meaning_changes_for_cache_tests();
    changed.add_declaration("SAnd", prop());
    assert!(changed.observational.is_none());
    assert!(
        original.observational.is_some(),
        "the clone's revocation is its own"
    );
    assert_eq!(funext(&original), (true, true));
}

#[test]
fn malformed_universe_instances_have_no_computation() {
    let ctx = certified();
    let function = term("Nat -> Nat");
    let (f, g) = (term("fun (n : Nat) => n"), term("fun (n : Nat) => n"));
    for levels in [vec![], vec![Universe::Type(0), Universe::Type(0)]] {
        let head = Term::Const {
            name: "Obs".into(),
            levels,
        };
        assert_eq!(
            reduce(&ctx, &head, &[&function, &f, &g]),
            None,
            "`Obs` at {} levels",
            head
        );
    }
    let well_formed = Term::Const {
        name: "Obs".into(),
        levels: vec![Universe::Type(0)],
    };
    assert!(
        reduce(&ctx, &well_formed, &[&function, &f, &g]).is_some(),
        "the control computes"
    );
}

#[test]
fn certified_constants_are_primitives_and_an_impostor_is_an_axiom() {
    let probe = term("Obs_refl.{1} Nat Zero");
    let ctx = certified();
    assert_eq!(
        axiom_dependencies(&ctx, &probe).axioms,
        BTreeSet::new(),
        "a certified constant is part of the theory"
    );
    assert_eq!(
        axiom_dependencies(&impostor(), &probe).axioms,
        ["Obs_refl".to_string()].into_iter().collect(),
        "an uncertified constant of the same spelling is an assumption"
    );
}

#[test]
fn replacing_a_projection_revokes_the_certificate() {
    for projection in PROJECTIONS {
        let mut ctx = certified();
        ctx.permit_meaning_changes_for_cache_tests();
        ctx.add_declaration(projection, prop());
        assert!(
            ctx.observational.is_none(),
            "replacing `{projection}` revokes the certificate"
        );
    }
}

/// A clone that inherits the certified context's caches must not read a verdict the certificate
/// earned once the certificate is revoked in the clone.
#[test]
fn a_revoked_certificate_is_not_read_back_from_a_cache_the_certified_context_filled() {
    let original = certified();
    assert_eq!(
        funext(&original),
        (true, true),
        "fills whatever the context caches"
    );
    let mut changed = original.clone();
    changed.permit_meaning_changes_for_cache_tests();
    changed.add_declaration("SAnd", prop());
    assert_eq!(funext(&changed), (false, false));
}

/// One step of the term `text`, as this kernel's rule takes it.
fn step(ctx: &Context, text: &str) -> Option<Term> {
    let t = term(text);
    let mut head = &t;
    let mut args = Vec::new();
    while let Term::App(f, a) = head {
        args.push(a.as_ref());
        head = f;
    }
    args.reverse();
    reduce(ctx, head, &args)
}

/// The certified context with what the seams name, each asserted registered: a refused setup would
/// leave its seam's term over an unknown name, which no rule reduces, and the seam would hold for
/// that reason alone. Shared with the re-checker's controls.
pub(crate) fn seam_context() -> Context {
    let mut ctx = certified();
    let var = |name: &str| Term::Var(name.to_string());
    ctx.add_indexed_inductive("Hur", term("(Type -> Type) -> Type"), 1);
    ctx.add_constructor("hur", "Hur", term("forall (F : Type -> Type), Hur F"));
    ctx.add_structure(
        "Sized",
        &[],
        &[
            ("n", term("Nat")),
            (
                "bits",
                Term::App(Box::new(term("BVec")), Box::new(var("n"))),
            ),
        ],
    );
    ctx.add_structure("Dep", &[], &[("A", term("Type")), ("a", var("A"))]);
    ctx.add_indexed_inductive("Tagged", term("Nat -> Type"), 0);
    ctx.add_constructor("tag", "Tagged", term("Bool -> Tagged Zero"));
    ctx.add_declaration("absurd", term("Obs.{2} Type Nat Bool"));
    ctx.add_declaration("e", term("Obs.{1} Prop True True"));
    for name in [
        "Hur", "hur", "Sized", "Sized_mk", "Dep", "Dep_mk", "Tagged", "tag", "absurd", "e",
    ] {
        assert!(
            ctx.get_global(name).is_some(),
            "the seams' context declares `{name}`"
        );
    }
    ctx
}

/// Each guard of the rules, one input it must leave stuck beside one it must reduce.
#[test]
fn each_guard_leaves_its_seam_stuck_beside_a_control_that_steps() {
    let ctx = seam_context();
    for (seam, stuck, control) in SEAMS {
        assert_eq!(step(&ctx, stuck), None, "{seam}: `{stuck}` is stuck");
        assert!(
            step(&ctx, control).is_some(),
            "{seam}: the control `{control}` steps"
        );
    }
}

/// `(seam, an input the guard keeps stuck, a control of the same rule that steps)`, shared with
/// the re-checker's controls.
pub(crate) const SEAMS: [(&str, &str, &str); 10] = [
    (
        "a proposition's constructors",
        "Obs.{0} (Or True True) (left True True I) (right True True I)",
        "Obs.{1} Nat Zero (Succ Zero)",
    ),
    (
        "a proposition seen as a type",
        "Obs.{2} Type (Nat -> False) (Bool -> False)",
        "Obs.{2} Type (Nat -> Nat) (Bool -> Nat)",
    ),
    (
        "a family over a large argument",
        "Obs.{2} Type (Hur (fun (X : Type) => X)) (Hur (fun (X : Type) => X -> X))",
        "Obs.{2} Type (TList (Nat -> Nat)) (TList (Nat -> Bool))",
    ),
    (
        "a dependent field",
        "Obs_cast.{1} Sized Sized (Obs_refl.{2} Type Sized) (Sized_mk (Succ Zero) (BVCons B0 Zero BVNil))",
        "Obs_cast.{1} (TList Nat) (TList Nat) (Obs_refl.{2} Type (TList Nat)) (TCons Nat Zero (TNil Nat))",
    ),
    (
        "a field whose type is an earlier field",
        "Obs_cast.{2} Dep Dep (Obs_refl.{3} (Sort 2) Dep) (Dep_mk Nat Zero)",
        "Obs_cast.{1} (TList Nat) (TList Nat) (Obs_refl.{2} Type (TList Nat)) (TNil Nat)",
    ),
    (
        "an indexed family",
        "Obs_cast.{1} (BVec (Succ Zero)) (BVec (Succ Zero)) (Obs_refl.{2} Type (BVec (Succ Zero))) (BVCons B0 Zero BVNil)",
        "Obs_cast.{1} Nat Nat (Obs_refl.{2} Type Nat) (Succ Zero)",
    ),
    (
        "an indexed family whose fields are independent",
        "Obs_cast.{1} (Tagged Zero) (Tagged Zero) (Obs_refl.{2} Type (Tagged Zero)) (tag true)",
        "Obs_cast.{1} Bool Bool (Obs_refl.{2} Type Bool) true",
    ),
    ("evidence of SFalse", "Obs_cast.{1} Nat Bool absurd Zero", "Obs_cast.{2} Type Type (Obs_refl.{3} (Sort 2) Type) Nat"),
    ("two propositions", "Obs_cast.{0} True True e I", "Obs_cast.{1} (Nat -> Nat) (Nat -> Nat) (Obs_refl.{2} Type (Nat -> Nat)) Succ"),
    (
        "a quotient of a proposition",
        "Obs.{0} (Quot True (fun (x : True) (y : True) => True)) (Quot_mk True (fun (x : True) (y : True) => True) I) \
         (Quot_mk True (fun (x : True) (y : True) => True) I)",
        "Obs.{1} (Quot Nat (fun (x : Nat) (y : Nat) => True)) (Quot_mk Nat (fun (x : Nat) (y : Nat) => True) Zero) \
         (Quot_mk Nat (fun (x : Nat) (y : Nat) => True) Zero)",
    ),
];

/// The cache keys on the certificate: a context certified after a verdict was cached without it does
/// not read that verdict back.
#[test]
fn a_verdict_cached_before_certification_is_not_read_back_after_it() {
    let mut ctx = StandardLibrary::context().clone();
    let certificate = declare(&mut ctx).expect("declares");
    assert_eq!(
        funext(&ctx),
        (false, false),
        "uncertified, nothing computes, whatever the context caches"
    );
    ctx.certify_observational(certificate);
    assert_eq!(
        funext(&ctx),
        (true, true),
        "certified, both kernels compute"
    );
}
