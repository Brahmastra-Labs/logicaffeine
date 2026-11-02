//! ★ `Theorem`, `Opaque` AND `Axiom` — three declarations the kernel had and the vernacular could
//! not spell.
//!
//! * `Theorem name : P := proof.` — the statement must be a PROPOSITION, the proof is checked by
//!   both kernels against it EXACTLY (conversion both ways, not cumulative subtyping), and the name
//!   is then published as Lean publishes a theorem: definitional equality may unfold it.
//! * `Opaque name : T := value.` — the same checks for data (Lean's `opaque`), never unfolded.
//! * `Axiom name : T.` — an explicit assumption. It is LISTED: `Print Axioms` names it on itself and
//!   on everything that rests on it.
//!
//! Each is checked by the `Repl` through the kernel door the Lean importer uses —
//! `Context::declare_theorem_checked`, `Context::declare_opaque_checked` and
//! `Context::add_universe_poly_declaration` — so a document
//! and an import cannot disagree about what a declaration means.

use logicaffeine_kernel::interface::printer::print_command;
use logicaffeine_kernel::interface::{parse_command, Command, Repl};
use logicaffeine_kernel::{Term, Universe};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn eq_nat(a: Term, b: Term) -> Term {
    app(app(app(g("Eq"), g("Nat")), a), b)
}

fn refused(r: &mut Repl, text: &str, name: &str) -> String {
    let out = r.execute(text);
    let err = match out {
        Err(e) => format!("{e}"),
        Ok(o) => panic!("{text:?} must be refused, got Ok({o:?})"),
    };
    assert!(
        r.context().get_global(name).is_none() && r.context().poly_declaration(name).is_none(),
        "a refused `{name}` must leave nothing behind"
    );
    err
}

#[test]
fn a_theorem_is_checked_and_then_unfolds_like_leans() {
    let mut r = Repl::new();
    r.execute("Theorem zz : Eq Nat Zero Zero := refl Nat Zero.")
        .expect("a true statement, proved");
    let ctx = r.context();
    assert_eq!(
        ctx.get_global("zz"),
        Some(&eq_nat(g("Zero"), g("Zero"))),
        "its statement is its type"
    );
    let proof = app(app(g("refl"), g("Nat")), g("Zero"));
    assert_eq!(
        ctx.get_definition_body("zz"),
        Some(&proof),
        "a theorem keeps its checked proof as its body"
    );
    assert!(
        ctx.get_universe_poly("zz").is_some(),
        "published through the checked theorem gateway"
    );
    assert_eq!(
        r.execute("Check zz.").unwrap(),
        format!("zz : {}", eq_nat(g("Zero"), g("Zero")))
    );
    assert_eq!(
        r.execute("Eval zz.").unwrap(),
        r.execute("Eval refl Nat Zero.").unwrap(),
        "evaluation unfolds a theorem as Lean's kernel does"
    );
    // Conversion sees through it, where an `Opaque` stays sealed (see the next test).
    r.execute("Definition zz_is_refl : Eq (Eq Nat Zero Zero) zz (refl Nat Zero) := refl (Eq Nat Zero Zero) zz.")
        .expect("a theorem is definitionally its proof");
}

#[test]
fn an_opaque_value_never_unfolds_in_conversion() {
    // The control: a transparent definition DOES unfold, so `refl` proves the equation…
    let mut r = Repl::new();
    r.execute("Definition kt : Nat := Zero.").unwrap();
    r.execute("Definition kt_is_zero : Eq Nat kt Zero := refl Nat Zero.")
        .expect("a transparent constant is its body");
    // …and the same equation about an OPAQUE constant is refused, because nothing may look inside.
    r.execute("Opaque ko : Nat := Zero.")
        .expect("an opaque value");
    assert!(r.context().get_definition_body("ko").is_none());
    let err = refused(
        &mut r,
        "Definition ko_is_zero : Eq Nat ko Zero := refl Nat Zero.",
        "ko_is_zero",
    );
    assert!(!err.is_empty());
    assert_eq!(r.execute("Eval ko.").unwrap(), "ko");
}

#[test]
fn a_theorem_whose_proof_has_another_type_is_refused() {
    let mut r = Repl::new();
    refused(
        &mut r,
        "Theorem wrong : Eq Nat Zero (Succ Zero) := refl Nat Zero.",
        "wrong",
    );
    refused(
        &mut r,
        "Theorem notaproof : Eq Nat Zero Zero := Zero.",
        "notaproof",
    );
    refused(&mut r, "Theorem holey : Eq Nat Zero Zero := _.", "holey");
    refused(&mut r, "Opaque wrongdata : Nat := I.", "wrongdata");
    refused(
        &mut r,
        "Theorem unbound : Eq Nat Zero Zero := nonexistent.",
        "unbound",
    );
}

#[test]
fn a_theorem_states_a_proposition_and_data_is_opaque_instead() {
    let mut r = Repl::new();
    let err = refused(&mut r, "Theorem data : Nat := Zero.", "data");
    assert!(
        err.contains("proposition") && err.contains("Opaque"),
        "the refusal says what to write: {err}"
    );
    r.execute("Opaque data : Nat := Zero.")
        .expect("data is declared opaque");
}

#[test]
fn an_axiom_is_an_explicit_listed_assumption() {
    let mut r = Repl::new();
    r.execute("Axiom lem : (p : Prop) -> Or p (Not p).")
        .expect("an assumption");
    assert!(r.context().get_definition_body("lem").is_none());
    let own = r.execute("Print Axioms lem.").unwrap();
    assert!(own.contains("lem"), "an axiom rests on itself: {own}");
    r.execute("Theorem uses : (p : Prop) -> Or p (Not p) := lem.")
        .expect("a theorem citing it");
    let report = r.execute("Print Axioms uses.").unwrap();
    assert!(
        report.contains("depends on axioms") && report.contains("lem"),
        "{report}"
    );
    r.execute("Theorem clean : Eq Nat Zero Zero := refl Nat Zero.")
        .unwrap();
    let report = r.execute("Print Axioms clean.").unwrap();
    assert!(report.contains("depends on no axioms"), "{report}");
    // A definition that rests on the axiom through a theorem is tainted too.
    r.execute("Definition downstream : (p : Prop) -> Or p (Not p) := uses.")
        .unwrap();
    let report = r.execute("Print Axioms downstream.").unwrap();
    assert!(report.contains("lem"), "the taint is transitive: {report}");
}

#[test]
fn an_axiom_must_state_a_type_and_cannot_replace_a_declaration() {
    let mut r = Repl::new();
    refused(&mut r, "Axiom notatype : Zero.", "notatype");
    let before = r.context().get_global("Nat").cloned();
    assert!(
        r.execute("Axiom Nat : Type.").is_err(),
        "an axiom cannot redeclare the prelude's Nat"
    );
    assert_eq!(r.context().get_global("Nat").cloned(), before);
    assert!(r.context().is_inductive("Nat"));
    r.execute("Theorem once : True := I.").unwrap();
    assert!(
        r.execute("Axiom once : False.").is_err(),
        "nor a theorem already proved"
    );
    assert_eq!(r.context().get_global("once"), Some(&g("True")));
}

#[test]
fn universe_polymorphic_theorems_and_axioms_carry_their_telescope() {
    let mut r = Repl::new();
    r.execute("Axiom pchoice.{u} : (A : Sort u) -> A.")
        .expect("a polymorphic assumption");
    let (params, _) = r
        .context()
        .poly_declaration("pchoice")
        .expect("a telescope");
    assert_eq!(params, ["u".to_string()]);
    r.execute(
        "Theorem pmono.{u} : (A : Sort u) -> (P : A -> Prop) -> (a : A) -> P a -> P a := \
         fun (A : Sort u) (P : A -> Prop) (a : A) (h : P a) => h.",
    )
    .expect("a polymorphic theorem");
    let (params, _) = r.context().poly_declaration("pmono").expect("a telescope");
    assert_eq!(params, ["u".to_string()]);
    r.execute("Check pmono.{1} Nat.").expect("instantiated");
    refused(
        &mut r,
        "Theorem stray.{u} : (A : Sort v) -> A -> A := fun (A : Sort v) (a : A) => a.",
        "stray",
    );
    refused(&mut r, "Axiom astray.{u} : Sort v.", "astray");
}

#[test]
fn theorem_opaque_and_axiom_read_and_print_back() {
    for text in [
        "Theorem zz : Eq Nat Zero Zero := refl Nat Zero.",
        "Theorem pmono.{u} : (A : Sort u) -> (P : A -> Prop) -> (a : A) -> P a -> P a := fun (A : Sort u) (P : A -> Prop) (a : A) (h : P a) => h.",
        "Opaque ko : Nat := Zero.",
        "Axiom lem : (p : Prop) -> Or p (Not p).",
        "Axiom pchoice.{u, v} : (A : Sort u) -> Sort v.",
        "Protected Theorem Alpha.t : True := I.",
        "Protected Axiom Alpha.a : True.",
    ] {
        let cmd = parse_command(text).unwrap_or_else(|e| panic!("{text}: {e:?}"));
        let printed = print_command(&cmd).unwrap_or_else(|e| panic!("{text}: {e}"));
        let again = parse_command(&printed).unwrap_or_else(|e| panic!("{printed}: {e:?}"));
        assert_eq!(print_command(&again).unwrap(), printed, "{text}");
    }
    let Command::Theorem { name, ty, proof } =
        parse_command("Theorem zz : Eq Nat Zero Zero := refl Nat Zero.").unwrap()
    else {
        panic!("a theorem")
    };
    assert_eq!(name, "zz");
    assert_eq!(ty, eq_nat(g("Zero"), g("Zero")));
    assert_eq!(
        proof,
        app(app(g("refl"), g("Nat")), g("Zero")),
        "the proof is carried as evidence"
    );
    let Command::Universes { params, inner } = parse_command("Axiom a.{u} : Sort u.").unwrap()
    else {
        panic!("a polymorphic axiom")
    };
    assert_eq!(params, vec!["u".to_string()]);
    let Command::Axiom { name, ty } = *inner else {
        panic!("an axiom")
    };
    assert_eq!(name, "a");
    assert_eq!(ty, Term::Sort(Universe::Var("u".into())));
}

#[test]
fn a_document_checks_axioms_theorems_and_their_audit_together() {
    let mut r = Repl::new();
    let out = r
        .execute_document(
            "Axiom ax : Eq Nat Zero (Succ Zero).\n\
             Theorem from_ax : Eq Nat Zero (Succ Zero) := ax.\n\
             Print Axioms from_ax.\n",
        )
        .expect("a document of new declarations");
    assert!(out.last().unwrap().contains("ax"), "{out:?}");
    // One refused theorem stops the document, and nothing after it is registered.
    let mut r = Repl::new();
    let refused = r.execute_document(
        "Theorem bad : Eq Nat Zero (Succ Zero) := refl Nat Zero.\n\
         Theorem after : True := I.\n",
    );
    assert!(refused.is_err());
    assert!(r.context().get_global("bad").is_none() && r.context().get_global("after").is_none());
}

#[test]
fn protected_applies_to_the_new_declarations() {
    let mut r = Repl::new();
    r.execute("Protected Theorem Alpha.t : True := I.")
        .expect("a protected theorem");
    r.execute("Protected Axiom Alpha.a : True.")
        .expect("a protected axiom");
    assert!(
        r.context().get_global("Alpha.t").is_some() && r.context().get_global("Alpha.a").is_some()
    );
    r.execute("Open Alpha.").unwrap();
    assert!(
        r.execute("Check t.").is_err(),
        "a protected name does not answer to its short name"
    );
    r.execute("Check Alpha.t.")
        .expect("…but does to its full one");
}
