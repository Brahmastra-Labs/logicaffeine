//! SHADOWING — a declaration that would change what a name already means is registered under a
//! FRESH kernel identity, and the name resolves to it from then on (Tristen's ruling, 2026-09-24,
//! COMPETITION_RULES.md).
//!
//! A document that writes `Definition add …` has not asked to change the prelude's `add`: it has
//! introduced ITS `add`. The stable-meaning gate stays absolute — no registered declaration ever
//! changes meaning under what depends on it — and the session's NAME RESOLUTION does the rest: the
//! written name `add` denotes the user's declaration for the rest of the session, every surface
//! agrees on that, and `_root_.add` still names the environment's own declaration beneath the
//! shadow, as Lean's `_root_` reaches a global a local shadows.
//!
//! What is pinned here:
//! * the fresh identity is not spellable and never collides;
//! * the prelude's declarations keep their meaning, and a prelude lemma still types against the
//!   prelude's `add`; the prelude's tactics keep reading the prelude's `add`;
//! * a second redefinition shadows the first and leaves the first's dependents as they were;
//! * re-running an identical document mints nothing;
//! * an unobserved user redefinition still replaces in place — the notebook convenience;
//! * every closed-`False` redeclaration route stays closed;
//! * `Check`/`Eval` print names as written, the literate surface reflects what the name resolves
//!   to, a shadowing document prints back to itself, and a document the Lean emitter writes never
//!   shadows anything.

use logicaffeine_kernel::interface::lean::{self as adapter, emit};
use logicaffeine_kernel::interface::{
    parse_command_prefix_in, print_command, print_term, Repl, Surface,
};
use logicaffeine_kernel::{infer_type, recheck, Context, Term};
use std::collections::BTreeSet;

const NUM_ADD: &str = "Inductive Num := Z : Num | S : Num -> Num.
Definition add : Num -> Num -> Num := fix rec => fun n : Num => fun m : Num =>
  match n return (fun _ : Num => Num) with | Z => m | S k => S (rec k m).
Definition one : Num := S Z.";

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}

fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}

fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}

fn pristine() -> Repl {
    Repl::new()
}

fn session(document: &str) -> Repl {
    let mut repl = Repl::new();
    repl.execute_document(document)
        .unwrap_or_else(|e| panic!("the document must be accepted: {e}\n{document}"));
    repl
}

fn ok(repl: &mut Repl, command: &str) -> String {
    repl.execute(command)
        .unwrap_or_else(|e| panic!("`{command}` must be accepted: {e}"))
}

fn refused(repl: &mut Repl, command: &str) {
    if let Ok(out) = repl.execute(command) {
        panic!("`{command}` must be refused, but printed {out:?}");
    }
}

/// The kernel key the written name currently denotes, when it is shadowed.
fn shadow_key(repl: &Repl, written: &str) -> String {
    repl.shadows()
        .into_iter()
        .find(|(w, _)| w == written)
        .map(|(_, key)| key)
        .unwrap_or_else(|| panic!("`{written}` is not shadowed: {:?}", repl.shadows()))
}

/// Every key the context holds a declaration at.
fn all_keys(ctx: &Context) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    keys.extend(ctx.iter_declarations().map(|(k, _)| k.to_string()));
    keys.extend(ctx.iter_definitions().map(|(k, _, _)| k.to_string()));
    for (k, _) in ctx.iter_inductives() {
        keys.insert(k.to_string());
        keys.extend(
            ctx.get_constructors(k)
                .into_iter()
                .map(|(c, _)| c.to_string()),
        );
    }
    keys
}

fn same_entry(a: &Context, b: &Context, key: &str) -> bool {
    a.get_global(key) == b.get_global(key)
        && a.get_definition_body(key) == b.get_definition_body(key)
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The shadow and its identity
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_document_definition_of_a_prelude_name_shadows_it_under_a_fresh_identity() {
    let prelude = pristine();
    let mut repl = session(NUM_ADD);
    assert_eq!(
        ok(&mut repl, "Eval add one one."),
        "(S (S Z))",
        "the name computes with the user's `add`"
    );
    assert_eq!(ok(&mut repl, "Check add."), "add : Num -> Num -> Num");

    let key = shadow_key(&repl, "add");
    assert_ne!(
        key, "add",
        "the user's `add` is not keyed at the prelude's key"
    );
    assert_eq!(
        repl.shadows().len(),
        1,
        "exactly one name is shadowed: {:?}",
        repl.shadows()
    );
    let ctx = repl.context();
    assert_eq!(
        ctx.get_global(&key),
        Some(&arrow(g("Num"), arrow(g("Num"), g("Num")))),
        "the user's `add` is registered at its fresh key"
    );
    assert!(ctx.get_definition_body(&key).is_some(), "…as a definition");
    assert!(
        same_entry(ctx, prelude.context(), "add"),
        "the prelude's `add` keeps its meaning at its own key"
    );
}

#[test]
fn the_fresh_identity_cannot_be_written_and_never_collides() {
    let repl = session(NUM_ADD);
    let key = shadow_key(&repl, "add");

    // No surface spells it: not bare, not quoted, not as a declaration name, and the printer
    // refuses to invent a spelling for it.
    let mut probe = session(NUM_ADD);
    refused(&mut probe, &format!("Check {key}."));
    refused(&mut probe, &format!("Check \u{ab}{key}\u{bb}."));
    refused(
        &mut probe,
        &format!("Definition \u{ab}{key}\u{bb} : Nat := Zero."),
    );
    assert!(
        print_term(&g(&key)).is_err(),
        "the printer has no spelling for a fresh identity"
    );

    // A key already taken is stepped past, never reused: the context is given the very key the
    // first session minted, and the document still shadows — elsewhere — and leaves it alone.
    let mut crowded = Repl::new();
    crowded
        .context_mut()
        .declare(&key, g("Nat"), g("Zero"))
        .expect("the key, taken in advance");
    crowded.execute_document(NUM_ADD).expect("the document");
    let minted = shadow_key(&crowded, "add");
    assert_ne!(
        minted, key,
        "a fresh identity never collides with a registered key"
    );
    assert_eq!(
        crowded.context().get_global(&key),
        Some(&g("Nat")),
        "the occupant is untouched"
    );
}

#[test]
fn root_names_the_environments_declaration_beneath_the_shadow() {
    let mut repl = session(NUM_ADD);
    assert_eq!(
        ok(&mut repl, "Check _root_.add."),
        "_root_.add : Int -> Int -> Int"
    );
    assert_eq!(ok(&mut repl, "Eval _root_.add 2 3."), "5");
}

/// The adversarial control the ruling names: a prelude lemma about `add` is still a lemma about
/// the prelude's `add`, and cannot be passed off as one about the user's.
#[test]
fn a_prelude_lemma_about_add_still_types_against_the_preludes_add() {
    let prelude = pristine();
    let mut repl = session(NUM_ADD);
    let lemma = g("IntLaw_add_neg_right");
    let before = infer_type(prelude.context(), &lemma).expect("the prelude lemma");
    assert_eq!(
        infer_type(repl.context(), &lemma).expect("still typed"),
        before,
        "the same statement, about the same `add`"
    );
    assert_eq!(
        ok(&mut repl, "Check IntLaw_add_neg_right."),
        "IntLaw_add_neg_right : \u{3a0}(a:Int). (((Eq Int) ((_root_.add a) (Int_neg a))) (Int_ofNat Zero))",
        "and it prints as a statement about the environment's `add`"
    );
    refused(
        &mut repl,
        "Definition stolen : forall a : Num, Eq Num (add a a) Z := IntLaw_add_neg_right.",
    );
}

#[test]
fn the_preludes_tactics_keep_reading_the_preludes_add() {
    let goal = "SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) (SApp (SApp (SName \"add\") (SLit 2)) (SLit 3))) (SLit 5)";
    let conclusion = |repl: &mut Repl| {
        ok(repl, &format!("Definition goal : Syntax := {goal}."));
        ok(repl, "Definition d : Derivation := try_ring goal.");
        (ok(repl, "Eval concludes d."), ok(repl, "Eval goal."))
    };
    let (proved, stated) = conclusion(&mut pristine());
    assert_eq!(proved, stated, "`ring` proves `add 2 3 = 5` in the prelude");
    let (proved, stated) = conclusion(&mut session(NUM_ADD));
    assert_eq!(
        proved, stated,
        "…and still does once a document shadows `add`: the tactic reads the prelude's"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Redefinition, idempotence, the notebook convenience
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_second_redefinition_shadows_the_first_and_keeps_its_dependents() {
    let mut repl = session(
        "Definition add : Nat -> Nat -> Nat := fun a : Nat => fun b : Nat => Succ a.
         Definition twice : Nat -> Nat := fun n : Nat => add n n.",
    );
    let first = shadow_key(&repl, "add");
    ok(
        &mut repl,
        "Definition add : Nat -> Nat -> Nat := fun a : Nat => fun b : Nat => Zero.",
    );
    let second = shadow_key(&repl, "add");
    assert!(
        first != second && first != "add" && second != "add",
        "three identities: {first} {second}"
    );
    assert_eq!(
        ok(&mut repl, "Eval twice (Succ Zero)."),
        "(Succ (Succ Zero))",
        "`twice` keeps the first `add`"
    );
    assert_eq!(
        ok(&mut repl, "Eval add (Succ Zero) (Succ Zero)."),
        "Zero",
        "the name now means the second"
    );
    assert!(
        repl.context().get_definition_body(&first).is_some(),
        "the first is still registered"
    );
}

#[test]
fn rerunning_an_identical_document_mints_nothing() {
    let document = format!(
        "{NUM_ADD}
         Definition two : Num := add one one.
         Theorem two_is : Eq Num two (S (S Z)) := refl Num (S (S Z)).
         Eval add two one."
    );
    let once = session(&document);
    let mut twice = session(&document);
    let second_run = twice
        .execute_document(&document)
        .expect("the identical document, again");
    assert_eq!(
        twice.shadows(),
        once.shadows(),
        "the second run shadows nothing new"
    );
    assert_eq!(
        all_keys(twice.context()),
        all_keys(once.context()),
        "and registers no new key"
    );
    assert_eq!(second_run.last().map(String::as_str), Some("(S (S (S Z)))"));
}

#[test]
fn an_unobserved_user_redefinition_replaces_in_place() {
    let mut repl = Repl::new();
    ok(&mut repl, "Definition scratch : Nat := Zero.");
    ok(&mut repl, "Definition scratch : Nat := Succ Zero.");
    assert!(
        repl.shadows().is_empty(),
        "nothing depended on `scratch`: {:?}",
        repl.shadows()
    );
    assert_eq!(ok(&mut repl, "Eval scratch."), "(Succ Zero)");
}

#[test]
fn a_user_name_with_dependents_is_shadowed_rather_than_refused() {
    let mut repl = Repl::new();
    ok(&mut repl, "Definition two : Nat := Succ (Succ Zero).");
    ok(&mut repl, "Definition usesTwo : Nat := two.");
    ok(&mut repl, "Definition two : Nat := Zero.");
    assert_eq!(
        ok(&mut repl, "Eval usesTwo."),
        "(Succ (Succ Zero))",
        "the dependent keeps what it was checked against"
    );
    assert_eq!(ok(&mut repl, "Eval two."), "Zero");
    assert_eq!(
        repl.context().get_definition_body("two"),
        Some(&app(g("Succ"), app(g("Succ"), g("Zero"))))
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The closed-False routes stay closed
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// `myAdd 1 1 = 2` by `refl`, then `myAdd := λ_ _. Zero`: the redefinition is a NEW `myAdd`, and
/// the theorem is still about the old one — so it is no proof of `0 = 2`, in either kernel.
#[test]
fn the_redefinition_route_to_false_stays_closed() {
    let mut repl = Repl::new();
    ok(
        &mut repl,
        "Definition myAdd : Nat -> Nat -> Nat := fun m : Nat => fun n : Nat => Nat_add m n.",
    );
    ok(
        &mut repl,
        "Theorem thm : Eq Nat (myAdd 1 1) 2 := refl Nat 2.",
    );
    ok(
        &mut repl,
        "Definition myAdd : Nat -> Nat -> Nat := fun m : Nat => fun n : Nat => Zero.",
    );

    refused(&mut repl, "Check nat_zs_ne 1 thm.");
    refused(&mut repl, "Definition zero_is_two : Eq Nat Zero 2 := thm.");
    refused(
        &mut repl,
        "Definition restated : Eq Nat (myAdd 1 1) 2 := thm.",
    );
    assert_eq!(
        ok(&mut repl, "Check thm."),
        "thm : (((Eq Nat) ((_root_.myAdd 1) 1)) 2)"
    );

    let falsity = app(
        app(
            g("nat_zs_ne"),
            Term::Lit(logicaffeine_kernel::Literal::Nat(
                logicaffeine_kernel::BigInt::from_u64(1),
            )),
        ),
        g("thm"),
    );
    assert!(
        infer_type(repl.context(), &falsity).is_err(),
        "no closed proof of `False`"
    );
    assert!(
        recheck(repl.context(), &falsity).is_err(),
        "…in the second kernel either"
    );
}

/// A theorem that eliminated `B2` exhaustively with one constructor never meets a second one: the
/// wider `B2` is a new inductive, with a new `b1`.
#[test]
fn the_constructor_set_route_to_false_stays_closed() {
    let mut repl = Repl::new();
    ok(&mut repl, "Inductive B2 := b1 : B2.");
    ok(&mut repl, "Definition all_b1 : forall x : B2, Eq B2 x b1 := fun x : B2 => match x return (fun y : B2 => Eq B2 y b1) with | b1 => refl B2 b1 end.");
    ok(&mut repl, "Inductive B2 := b1 : B2 | b2 : B2.");
    refused(&mut repl, "Check all_b1 b2.");
    refused(&mut repl, "Check all_b1 b1.");
    ok(&mut repl, "Check all_b1 _root_.b1.");
    assert_eq!(
        repl.context().get_constructors("B2").len(),
        1,
        "the original `B2` keeps its one constructor"
    );
}

/// A constructor may not take the name of a definition other declarations use: it takes a fresh
/// identity, and the definition keeps computing.
#[test]
fn a_constructor_named_like_a_used_definition_takes_a_fresh_identity() {
    let mut repl = Repl::new();
    ok(&mut repl, "Inductive Tok := Nat_add : Tok.");
    assert_eq!(ok(&mut repl, "Check Nat_add."), "Nat_add : Tok");
    assert_eq!(
        ok(&mut repl, "Check _root_.Nat_add."),
        "_root_.Nat_add : Nat -> Nat -> Nat"
    );
    assert_eq!(
        ok(&mut repl, "Eval Nat_mul 2 3."),
        "(Succ (Succ (Succ (Succ (Succ (Succ Zero))))))"
    );
    assert!(
        repl.context().get_definition_body("Nat_add").is_some(),
        "`Nat_add` is still the definition"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Every surface agrees
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn check_and_eval_print_every_name_as_written() {
    let mut repl = session(NUM_ADD);
    ok(
        &mut repl,
        "Definition twice : Num -> Num := fun n : Num => add n n.",
    );
    let key = shadow_key(&repl, "add");
    for command in [
        "Check add.",
        "Check twice.",
        "Eval add one.",
        "Check add one.",
        "Eval twice one.",
    ] {
        let out = ok(&mut repl, command);
        assert!(
            !out.contains(&key),
            "`{command}` printed the fresh identity: {out}"
        );
    }
    assert_eq!(ok(&mut repl, "Check add one."), "(add one) : Num -> Num");

    // A shadow that is itself shadowed has no written name left: it prints as Lean prints an
    // inaccessible name, and never as its kernel key.
    ok(
        &mut repl,
        "Theorem add_one_one : Eq Num (add one one) (S one) := refl Num (S one).",
    );
    ok(
        &mut repl,
        "Definition add : Num -> Num -> Num := fun n : Num => fun m : Num => Z.",
    );
    assert_eq!(
        ok(&mut repl, "Check add_one_one."),
        "add_one_one : (((Eq Num) ((add\u{271d} one) one)) (S one))"
    );
    assert_eq!(ok(&mut repl, "Check twice."), "twice : Num -> Num");
}

/// A literate `+` is sugar for the name `add`, and the literate surface resolves what it reflects:
/// a theorem about `add` in a document that shadows `add` is a theorem about the user's `add`.
#[test]
fn the_literate_surface_resolves_the_names_it_reads_and_reflects() {
    let mut repl = session(
        "## To add (n: Nat) and (m: Nat) -> Nat:
    Consider n:
        When Zero: Yield m.
        When Succ k: Yield Succ (add(k, m)).

Let three be (Succ Zero) + (Succ (Succ Zero)).

## Theorem: add_zero_left
    Statement: forall n: Nat, add(Zero, n) equals n.
    Proof: simp.
",
    );
    assert_eq!(
        ok(&mut repl, "Eval three."),
        "(Succ (Succ (Succ Zero)))",
        "`+` is the user's `add` here"
    );
    let key = shadow_key(&repl, "add");
    let proof = ok(&mut repl, "Eval concludes add_zero_left.");
    assert!(
        proof.contains(&format!("\"{key}\"")),
        "the reflected statement names the user's `add`: {proof}"
    );
    assert!(
        !proof.contains("\"add\""),
        "and never the prelude's: {proof}"
    );
}

/// print∘parse ≡ id for a shadowing document: every command prints and reads back as itself, and
/// the printed document, run in a fresh session, is the same session.
#[test]
fn a_shadowing_document_prints_back_to_itself() {
    let document = format!(
        "{NUM_ADD}
         Definition twice : Num -> Num := fun n : Num => add n n.
         Definition add : Num -> Num -> Num := fun n : Num => fun m : Num => m.
         Theorem twice_one : Eq Num (twice one) (S one) := refl Num (S one).
         Inductive Tok := Nat_add : Tok."
    );
    let surface = Surface::default();
    let mut rest = document.as_str();
    let mut printed = String::new();
    while !rest.trim().is_empty() {
        let (command, used) = parse_command_prefix_in(&surface, rest).expect("a command");
        let text = print_command(&command).unwrap_or_else(|e| panic!("{command:?} prints: {e}"));
        let (again, _) = parse_command_prefix_in(&surface, &text).expect("the print reads back");
        assert_eq!(
            format!("{again:?}"),
            format!("{command:?}"),
            "print∘parse is the identity on `{text}`"
        );
        printed.push_str(&text);
        printed.push('\n');
        rest = &rest[used..];
    }
    let original = session(&document);
    let reprinted = session(&printed);
    assert_eq!(
        reprinted.shadows(),
        original.shadows(),
        "the same names are shadowed, by the same identities"
    );
    let keys = all_keys(original.context());
    assert_eq!(all_keys(reprinted.context()), keys);
    for key in &keys {
        assert!(
            same_entry(original.context(), reprinted.context(), key),
            "`{key}` means the same in both"
        );
    }
}

/// The Lean emitter relocates every Lean root the prelude already uses, so the documents it writes
/// never shadow anything and each correspondence name IS the kernel key it lands at. The control
/// beside it shows the check can fail.
#[test]
fn a_document_the_lean_emitter_writes_shadows_nothing() {
    const LE_EMIT: &str = include_str!("fixtures/lean_emission/LE_Emit.ndjson");
    let target: String = ["collisions"].iter().map(|p| format!("/s:{p:?}")).collect();
    let intake = adapter::intake::read(LE_EMIT, &adapter::intake::STAGE_A).expect("intake");
    let closure = intake.closure(&target).expect("closure");
    let imported = adapter::import_retaining_closure(LE_EMIT, &target, &closure)
        .expect("import")
        .0;
    let doc = emit::emit_document(&imported, &target).expect("emission");
    assert!(
        doc.source.contains("add"),
        "the fixture exercises the prelude-colliding Lean `add`"
    );
    let reader = session(&doc.source);
    assert!(
        reader.shadows().is_empty(),
        "an emitted document shadowed {:?}",
        reader.shadows()
    );
    for entry in &doc.correspondence {
        assert!(!entry.logos.contains('\u{27e8}'), "{}", entry.logos);
    }

    let control = session(NUM_ADD);
    assert!(
        !control.shadows().is_empty(),
        "the same check sees a document that does shadow"
    );
}
