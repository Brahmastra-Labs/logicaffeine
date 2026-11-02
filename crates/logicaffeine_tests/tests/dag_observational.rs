//! Observational equality in the `dag` core (`Theory::Logos`, `dag::logos_obs`) decides what the
//! `Term` kernel decides (`observational`), at the kernel interface: the registration replays into
//! the core whole, every declaration whose check needs a rule to compute is admitted by both, and
//! every seam a rule must not open is refused by both.

use logicaffeine_kernel::dag::logos_env::{admit, replay, Declaration};
use logicaffeine_kernel::interface::TermParser;
use logicaffeine_kernel::observational;
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{Context, Term};

fn term(text: &str) -> Term {
    TermParser::parse(text).unwrap_or_else(|e| panic!("`{text}` parses: {e:?}"))
}

fn on_a_large_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(f)
        .expect("spawns")
        .join()
        .expect("joins")
}

/// The standard library, the observational registration, and the declarations the seams need.
fn context() -> Context {
    let mut ctx = StandardLibrary::context().clone();
    observational::register(&mut ctx).expect("registers");
    ctx.add_indexed_inductive("Hur", term("(Type -> Type) -> Type"), 1);
    ctx.add_constructor("hur", "Hur", term("forall (F : Type -> Type), Hur F"));
    let bits = Term::App(Box::new(term("BVec")), Box::new(Term::Var("n".to_string())));
    ctx.add_structure("Sized", &[], &[("n", term("Nat")), ("bits", bits)]);
    ctx.add_declaration("absurd", term("Obs.{2} Type Nat Bool"));
    ctx.add_universe_poly_declaration(
        "Obs2",
        vec!["u".to_string()],
        term("forall (A : Sort u) (a : A) (b : A), SProp"),
    )
    .expect("a look-alike of `Obs`");
    ctx.add_universe_poly_declaration(
        "Cast2",
        vec!["u".to_string()],
        term("forall (A : Sort u) (B : Sort u), Obs.{u+1} (Sort u) A B -> A -> B"),
    )
    .expect("a look-alike of `Obs_cast`");
    ctx.add_structure(
        "Dep",
        &[],
        &[("A", term("Type")), ("a", Term::Var("A".to_string()))],
    );
    ctx.add_indexed_inductive("Tagged", term("Nat -> Type"), 0);
    ctx.add_constructor("tag", "Tagged", term("Bool -> Tagged Zero"));
    ctx.add_indexed_inductive("Flag", term("Bool -> Type"), 0);
    ctx.add_constructor("flag", "Flag", term("Bool -> Flag true"));
    // A refused setup leaves the seams' terms over unknown names, which no rule reduces and both
    // kernels refuse for that reason alone.
    for name in [
        "Hur", "hur", "Sized", "Sized_mk", "absurd", "Obs2", "Cast2", "Dep", "Dep_mk", "Tagged",
        "tag", "Flag", "flag",
    ] {
        assert!(
            ctx.get_global(name).is_some(),
            "the setup declares `{name}`"
        );
    }
    ctx
}

/// `(name, type, body)`: a definition put to the `Term` kernel and to the core replaying `context()`.
fn verdicts(
    name: &'static str,
    ty: &'static str,
    body: &'static str,
) -> (Result<(), String>, Result<(), String>) {
    on_a_large_stack(move || {
        let ctx = context();
        let mut checked = ctx.clone();
        // A definition the door accepts as pending (it names something not yet declared) is not
        // admitted.
        let term_verdict = checked
            .declare(name, term(ty), term(body))
            .map_err(|e| e.to_string())
            .and_then(|()| {
                match checked
                    .pending_obligations()
                    .iter()
                    .find(|(pending, _)| *pending == name)
                {
                    Some((_, missing)) => {
                        Err(format!("`{name}` is pending on `{missing}`, not admitted"))
                    }
                    None => Ok(()),
                }
            });
        let (mut k, replayed) = replay(&ctx);
        let refused: Vec<String> = replayed
            .iter()
            .filter_map(|r| r.verdict.as_ref().err().map(|e| format!("{}: {e}", r.name)))
            .collect();
        assert!(
            refused.is_empty(),
            "the core admits the registration and the setup: {refused:#?}"
        );
        let core_verdict = admit(
            &mut k,
            &ctx,
            &[Declaration::Definition {
                name: name.to_string(),
                params: Vec::new(),
                ty: term(ty),
                body: term(body),
            }],
        );
        (term_verdict, core_verdict)
    })
}

/// The harness itself: a definition over a constant nothing declares is one the door holds as
/// pending, and it is counted as refused, never as admitted.
#[test]
fn a_pending_definition_is_not_counted_as_admitted() {
    let (term, core) = verdicts(
        "pending_probe",
        "Undeclared -> Undeclared",
        "fun (x : Undeclared) => x",
    );
    assert!(matches!(&term, Err(e) if e.contains("pending")), "{term:?}");
    assert!(core.is_err(), "{core:?}");
}

#[test]
fn the_core_admits_the_registration_whole() {
    on_a_large_stack(|| {
        let ctx = context();
        let (_, replayed) = replay(&ctx);
        let refused: Vec<String> = replayed
            .iter()
            .filter_map(|r| r.verdict.as_ref().err().map(|e| format!("{}: {e}", r.name)))
            .collect();
        assert!(refused.is_empty(), "{refused:#?}");
        for name in ["Obs", "Obs_refl", "Obs_transp", "Obs_cast"] {
            let entry = replayed
                .iter()
                .find(|r| r.name == name)
                .unwrap_or_else(|| panic!("`{name}` is replayed"));
            assert_eq!(
                entry.kind, "observational",
                "`{name}` is installed with its certificate's constants"
            );
        }
    });
}

/// A declaration whose name sorts before every connective, and whose check builds `SAnd`: the core
/// admits the certified constants after every name their rules' answers are built from, so it is
/// admitted wherever the replay's order puts it.
#[test]
fn a_declaration_the_rules_build_connectives_for_is_admitted_after_them() {
    on_a_large_stack(|| {
        let mut ctx = context();
        ctx.declare(
            "AAA_propext",
            term("forall (P : SProp) (Q : SProp), SAnd (P -> Q) (Q -> P) -> Obs.{1} SProp P Q"),
            term("fun (P : SProp) (Q : SProp) (h : SAnd (P -> Q) (Q -> P)) => h"),
        )
        .expect("the Term kernel admits it");
        // Names no connective: only the rule's answer is a pair to take apart.
        ctx.declare(
            "AAA_forth",
            term("forall (P : SProp) (Q : SProp), Obs.{1} SProp P Q -> P -> Q"),
            term(
                "fun (P : SProp) (Q : SProp) (h : Obs.{1} SProp P Q) => \
                 match h return (fun (s : Obs.{1} SProp P Q) => P -> Q) with fun (f : P -> Q) (g : Q -> P) => f end",
            ),
        )
        .expect("the Term kernel admits it");
        let (_, replayed) = replay(&ctx);
        let refused: Vec<String> = replayed
            .iter()
            .filter_map(|r| r.verdict.as_ref().err().map(|e| format!("{}: {e}", r.name)))
            .collect();
        assert!(refused.is_empty(), "{refused:#?}");
    });
}

const TO_EQ: &str = "(fun (T : Type) (x : T) (y : T) (h : Obs.{1} T x y) => \
                     Obs_cast.{0} (Eq T x x) (Eq T x y) \
                     (Obs_transp.{1} T x (fun (c : T) (hc : Obs.{1} T x c) => Obs.{1} Prop (Eq T x x) (Eq T x c)) \
                     (Obs_refl.{1} Prop (Eq T x x)) y h) \
                     (refl T x))";

/// Each needs a rule to compute; each is admitted by both kernels.
const COMPUTING: &[(&str, &str, &str)] = &[
    (
        "obs_funext",
        "forall (A : Type) (B : Type) (f : A -> B) (g : A -> B), (forall (x : A), Obs.{1} B (f x) (g x)) -> Obs.{1} (A -> B) f g",
        "fun (A : Type) (B : Type) (f : A -> B) (g : A -> B) (h : forall (x : A), Obs.{1} B (f x) (g x)) => h",
    ),
    (
        "propext_strict",
        "forall (P : SProp) (Q : SProp), SAnd (P -> Q) (Q -> P) -> Obs.{1} SProp P Q",
        "fun (P : SProp) (Q : SProp) (h : SAnd (P -> Q) (Q -> P)) => h",
    ),
    (
        "obs_propext",
        "forall (P : Prop) (Q : Prop), SAnd (SBox.{0} (P -> Q)) (SBox.{0} (Q -> P)) -> Obs.{1} Prop P Q",
        "fun (P : Prop) (Q : Prop) (h : SAnd (SBox.{0} (P -> Q)) (SBox.{0} (Q -> P))) => h",
    ),
    ("clash", "Obs.{1} Nat Zero (Succ Zero) -> SFalse", "fun (h : Obs.{1} Nat Zero (Succ Zero)) => h"),
    (
        "injective",
        "forall (n : Nat) (m : Nat), Obs.{1} Nat (Succ n) (Succ m) -> Obs.{1} Nat n m",
        "fun (n : Nat) (m : Nat) (h : Obs.{1} Nat (Succ n) (Succ m)) => h",
    ),
    (
        "pairs",
        "forall (a : Nat) (b : Bool) (c : Nat) (d : Bool), \
         Obs.{1} (Prod Nat Bool) (Prod_mk Nat Bool a b) (Prod_mk Nat Bool c d) -> SAnd (Obs.{1} Nat a c) (Obs.{1} Bool b d)",
        "fun (a : Nat) (b : Bool) (c : Nat) (d : Bool) (h : Obs.{1} (Prod Nat Bool) (Prod_mk Nat Bool a b) (Prod_mk Nat Bool c d)) => h",
    ),
    (
        "dependent_field",
        "forall (k : Nat) (b : Bit) (c : Bit) (v : BVec k) (w : BVec k), \
         Obs.{1} (BVec (Succ k)) (BVCons b k v) (BVCons c k w) -> \
         SAnd (Obs.{1} Bit b c) (SAnd (Obs.{1} Nat k k) (SSigma (Obs.{2} Type (BVec k) (BVec k)) \
         (fun (e : Obs.{2} Type (BVec k) (BVec k)) => Obs.{1} (BVec k) (Obs_cast.{1} (BVec k) (BVec k) e v) w)))",
        "fun (k : Nat) (b : Bit) (c : Bit) (v : BVec k) (w : BVec k) (h : Obs.{1} (BVec (Succ k)) (BVCons b k v) (BVCons c k w)) => h",
    ),
    ("families", "Obs.{2} Type Nat Bool -> SFalse", "fun (h : Obs.{2} Type Nat Bool) => h"),
    (
        "family_arguments",
        "forall (A : Type) (B : Type), Obs.{2} Type (TList A) (TList B) -> Obs.{2} Type A B",
        "fun (A : Type) (B : Type) (h : Obs.{2} Type (TList A) (TList B)) => h",
    ),
    (
        "function_types",
        "Obs.{2} Type (Nat -> Nat) (Nat -> Bool) -> \
         SSigma (Obs.{2} Type Nat Nat) (fun (e : Obs.{2} Type Nat Nat) => forall (x : Nat), Obs.{2} Type Nat Bool)",
        "fun (h : Obs.{2} Type (Nat -> Nat) (Nat -> Bool)) => h",
    ),
    ("universes", "Obs.{3} (Sort 2) Type Prop -> SFalse", "fun (h : Obs.{3} (Sort 2) Type Prop) => h"),
    (
        "canonicity",
        "Eq Nat (Obs_cast.{1} Nat Nat (Obs_refl.{2} Type Nat) (Succ (Succ Zero))) (Succ (Succ Zero))",
        "refl Nat (Succ (Succ Zero))",
    ),
    (
        "carried_function",
        "Eq Nat ((Obs_cast.{1} (Nat -> Nat) (Nat -> Nat) (Obs_refl.{2} Type (Nat -> Nat)) Succ) Zero) (Succ Zero)",
        "refl Nat (Succ Zero)",
    ),
    (
        "carried_list",
        "Eq (TList Nat) (Obs_cast.{1} (TList Nat) (TList Nat) (Obs_refl.{2} Type (TList Nat)) (TCons Nat Zero (TNil Nat))) \
         (TCons Nat Zero (TNil Nat))",
        "refl (TList Nat) (TCons Nat Zero (TNil Nat))",
    ),
    (
        "classes",
        "forall (A : Type) (r : A -> A -> Prop) (a : A) (b : A), \
         Obs.{1} (Quot A r) (Quot_mk A r a) (Quot_mk A r b) -> SBox.{0} (EqvGen.{1} A r a b)",
        "fun (A : Type) (r : A -> A -> Prop) (a : A) (b : A) (h : Obs.{1} (Quot A r) (Quot_mk A r a) (Quot_mk A r b)) => h",
    ),
    (
        "carried_class",
        "Eq (Quot Nat (fun (x : Nat) (y : Nat) => True)) \
         (Obs_cast.{1} (Quot Nat (fun (x : Nat) (y : Nat) => True)) (Quot Nat (fun (x : Nat) (y : Nat) => True)) \
         (Obs_refl.{2} Type (Quot Nat (fun (x : Nat) (y : Nat) => True))) (Quot_mk Nat (fun (x : Nat) (y : Nat) => True) Zero)) \
         (Quot_mk Nat (fun (x : Nat) (y : Nat) => True) Zero)",
        "refl (Quot Nat (fun (x : Nat) (y : Nat) => True)) (Quot_mk Nat (fun (x : Nat) (y : Nat) => True) Zero)",
    ),
    (
        "eq_to_obs",
        "forall (A : Type) (a : A) (b : A), Eq A a b -> Obs.{1} A a b",
        "fun (A : Type) (a : A) (b : A) (h : Eq A a b) => Eq_rec A a (fun (c : A) => Obs.{1} A a c) (Obs_refl.{1} A a) b h",
    ),
];

#[test]
fn every_declaration_a_rule_computes_for_is_admitted_by_both_kernels() {
    let mut disagreements = Vec::new();
    for (name, ty, body) in COMPUTING {
        let (term_verdict, core_verdict) = verdicts(name, ty, body);
        if term_verdict.is_err() || core_verdict.is_err() {
            disagreements.push(format!(
                "{name}: Term {term_verdict:?}, core {core_verdict:?}"
            ));
        }
    }
    assert!(disagreements.is_empty(), "{disagreements:#?}");
}

#[test]
fn quotient_soundness_is_admitted_by_both_kernels_as_a_theorem() {
    let ty = "forall (A : Type) (r : A -> A -> Prop) (a : A) (b : A), r a b -> Eq (Quot A r) (Quot_mk A r a) (Quot_mk A r b)";
    let body: &'static str = Box::leak(
        format!(
            "fun (A : Type) (r : A -> A -> Prop) (a : A) (b : A) (h : r a b) => \
             {TO_EQ} (Quot A r) (Quot_mk A r a) (Quot_mk A r b) (sbox.{{0}} (EqvGen.{{1}} A r a b) (EqvGen_rel A r a b h))"
        )
        .into_boxed_str(),
    );
    let (term_verdict, core_verdict) = verdicts("quot_sound_theorem", ty, body);
    assert_eq!(
        (term_verdict.is_ok(), core_verdict.is_ok()),
        (true, true),
        "Term {term_verdict:?}, core {core_verdict:?}"
    );
}

/// Each would follow if a rule fired where it must not; each is refused by both kernels.
const SEAMS: &[(&str, &str, &str)] = &[
    (
        "two_proofs_clash",
        "Obs.{0} (Or True True) (left True True I) (right True True I) -> SFalse",
        "fun (h : Obs.{0} (Or True True) (left True True I) (right True True I)) => h",
    ),
    (
        "a_proposition_by_its_former",
        "Obs.{2} Type (Nat -> False) (Bool -> False) -> SFalse",
        "fun (h : Obs.{2} Type (Nat -> False) (Bool -> False)) => h",
    ),
    (
        "hur_injective",
        "Obs.{2} Type (Hur (fun (X : Type) => X)) (Hur (fun (X : Type) => X -> X)) -> \
         Obs.{3} (Type -> Type) (fun (X : Type) => X) (fun (X : Type) => X -> X)",
        "fun (h : Obs.{2} Type (Hur (fun (X : Type) => X)) (Hur (fun (X : Type) => X -> X))) => h",
    ),
    ("bool_out_of_nat", "Eq Bool (Obs_cast.{1} Nat Bool absurd Zero) true", "refl Bool true"),
    (
        "cast_of_a_variable",
        "forall (n : Nat), Eq Nat (Obs_cast.{1} Nat Nat (Obs_refl.{2} Type Nat) n) n",
        "fun (n : Nat) => refl Nat n",
    ),
    (
        "indexed_value_carried",
        "Eq (BVec (Succ Zero)) (Obs_cast.{1} (BVec (Succ Zero)) (BVec (Succ Zero)) (Obs_refl.{2} Type (BVec (Succ Zero))) \
         (BVCons B0 Zero BVNil)) (BVCons B0 Zero BVNil)",
        "refl (BVec (Succ Zero)) (BVCons B0 Zero BVNil)",
    ),
    (
        "dependent_field_carried",
        "Eq Sized (Obs_cast.{1} Sized Sized (Obs_refl.{2} Type Sized) (Sized_mk (Succ Zero) (BVCons B0 Zero BVNil))) \
         (Sized_mk (Succ Zero) (BVCons B0 Zero BVNil))",
        "refl Sized (Sized_mk (Succ Zero) (BVCons B0 Zero BVNil))",
    ),
    (
        "a_look_alike_computes",
        "Obs2.{1} Nat Zero (Succ Zero) -> SFalse",
        "fun (h : Obs2.{1} Nat Zero (Succ Zero)) => h",
    ),
    (
        "a_cast_look_alike_carries",
        "Eq Nat (Cast2.{1} Nat Nat (Obs_refl.{2} Type Nat) Zero) Zero",
        "refl Nat Zero",
    ),
    (
        "a_proposition_by_its_formers_components",
        "Obs.{2} Type (Nat -> False) (Bool -> False) -> \
         SSigma (Obs.{2} Type Bool Nat) (fun (e : Obs.{2} Type Bool Nat) => forall (x : Bool), Obs.{1} Prop False False)",
        "fun (h : Obs.{2} Type (Nat -> False) (Bool -> False)) => h",
    ),
    (
        "a_field_typed_by_an_earlier_field_carried",
        "Eq Dep (Obs_cast.{2} Dep Dep (Obs_refl.{3} (Sort 2) Dep) (Dep_mk Nat Zero)) (Dep_mk Nat Zero)",
        "refl Dep (Dep_mk Nat Zero)",
    ),
    (
        "an_index_read_as_a_parameter_equates_two_values",
        "Eq (Flag true) (Obs_cast.{1} (Flag true) (Flag true) (Obs_refl.{2} Type (Flag true)) (flag false)) (flag true)",
        "refl (Flag true) (flag true)",
    ),
    (
        "an_indexed_value_with_independent_fields_carried",
        "Eq (Tagged Zero) (Obs_cast.{1} (Tagged Zero) (Tagged Zero) (Obs_refl.{2} Type (Tagged Zero)) (tag true)) (tag true)",
        "refl (Tagged Zero) (tag true)",
    ),
    (
        "eq_is_obs",
        "forall (a : Nat) (b : Nat), Eq Nat a b -> Obs.{1} Nat a b",
        "fun (a : Nat) (b : Nat) (h : Eq Nat a b) => h",
    ),
];

#[test]
fn every_seam_is_refused_by_both_kernels() {
    let mut opened = Vec::new();
    for (name, ty, body) in SEAMS {
        let (term_verdict, core_verdict) = verdicts(name, ty, body);
        if term_verdict.is_ok() || core_verdict.is_ok() {
            opened.push(format!(
                "{name}: Term {term_verdict:?}, core {core_verdict:?}"
            ));
        }
    }
    assert!(opened.is_empty(), "{opened:#?}");
}

/// The core observes a literal as the constructor it names only while that constructor is `Nat`'s,
/// as the `Term` kernel does (`kernel_observational`): with `Succ` assumed, `sI : 1 ~[Nat] Succ
/// Zero` is refused by both kernels and `sI : 0 ~[Nat] Zero` admitted by both.
#[test]
fn the_core_observes_a_literal_only_as_the_constructors_its_family_has() {
    on_a_large_stack(|| {
        let mut ctx = Context::new();
        ctx.add_inductive("Nat", term("Type"));
        ctx.add_constructor("Zero", "Nat", term("Nat"));
        ctx.add_constructor("Succ", "Nat", term("Nat -> Nat"));
        ctx.add_constructor("Other", "Nat", term("Nat -> Nat"));
        ctx.add_declaration("Succ", term("Nat -> Nat"));
        observational::register(&mut ctx).expect("registers");
        assert!(
            ctx.refused_declarations().is_empty(),
            "the setup is admitted: {:?}",
            ctx.refused_declarations()
        );
        let (mut k, replayed) = replay(&ctx);
        assert!(
            replayed.iter().all(|r| r.verdict.is_ok()),
            "the core admits the setup"
        );
        for (name, ty, admitted) in [
            ("one_is_succ_zero", "Obs.{1} Nat 1n (Succ Zero)", false),
            ("zero_is_zero", "Obs.{1} Nat 0n Zero", true),
        ] {
            let term_verdict = ctx.clone().declare(name, term(ty), term("sI"));
            let core_verdict = admit(
                &mut k,
                &ctx,
                &[Declaration::Definition {
                    name: name.to_string(),
                    params: Vec::new(),
                    ty: term(ty),
                    body: term("sI"),
                }],
            );
            assert_eq!(
                (term_verdict.is_ok(), core_verdict.is_ok()),
                (admitted, admitted),
                "`{name} : {ty}`: the Term kernel {term_verdict:?}, the core {core_verdict:?}"
            );
        }
    })
}

/// The core holds a literal no reading holds for opaque to observation, as the `Term` kernel does:
/// with `Succ` a constructor of another family, `1 ~[Nat] Other Zero` is no clash, `0 ~[Nat] Other
/// Zero` is.
#[test]
fn the_core_holds_a_literal_no_reading_holds_for_opaque() {
    on_a_large_stack(|| {
        let mut ctx = Context::new();
        ctx.add_inductive("Nat", term("Type"));
        ctx.add_constructor("Zero", "Nat", term("Nat"));
        ctx.add_constructor("Other", "Nat", term("Nat -> Nat"));
        ctx.add_inductive("Foo", term("Type"));
        ctx.add_constructor("Succ", "Foo", term("Nat -> Foo"));
        observational::register(&mut ctx).expect("registers");
        assert!(
            ctx.refused_declarations().is_empty(),
            "the setup is admitted: {:?}",
            ctx.refused_declarations()
        );
        let (mut k, replayed) = replay(&ctx);
        assert!(
            replayed.iter().all(|r| r.verdict.is_ok()),
            "the core admits the setup"
        );
        for (name, ty, admitted) in [
            (
                "one_clashes",
                "Obs.{1} Nat 1n (Other Zero) -> SFalse",
                false,
            ),
            (
                "zero_clashes",
                "Obs.{1} Nat 0n (Other Zero) -> SFalse",
                true,
            ),
        ] {
            let body = "fun (h : SFalse) => h";
            let term_verdict = ctx.clone().declare(name, term(ty), term(body));
            let core_verdict = admit(
                &mut k,
                &ctx,
                &[Declaration::Definition {
                    name: name.to_string(),
                    params: Vec::new(),
                    ty: term(ty),
                    body: term(body),
                }],
            );
            assert_eq!(
                (term_verdict.is_ok(), core_verdict.is_ok()),
                (admitted, admitted),
                "`{name} : {ty}`: the Term kernel {term_verdict:?}, the core {core_verdict:?}"
            );
        }
    })
}
