//! SHADOWING, EVERYWHERE A NAME IS READ — the reach of the 2026-09-24 ruling beyond the spec's own
//! rows (`shadowing.rs`).
//!
//! A shadow redirects a WRITTEN NAME to a fresh identity, and every door that reads a name reads
//! it through the one resolution point (`surface::Env::resolve`). So the rows here are the doors:
//! a derived constant's name, a namespace and an `open`, a literate pattern and a literate clause
//! body, the proof vocabulary the literate door builds a derivation from, a mutual block, a
//! `Protected` declaration — and the consumer that interprets names by spelling, the ring oracle,
//! which must never read a shadow as the environment's arithmetic.

use logicaffeine_kernel::interface::{InterfaceError, Repl};
use logicaffeine_kernel::{Context, KernelError, Term};
use logicaffeine_proof::arith::prove_int_eq;

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

fn refused(repl: &mut Repl, command: &str) -> InterfaceError {
    match repl.execute(command) {
        Ok(out) => panic!("`{command}` must be refused, but printed {out:?}"),
        Err(e) => e,
    }
}

fn shadow_key(repl: &Repl, written: &str) -> String {
    repl.shadows()
        .into_iter()
        .find(|(w, _)| w == written)
        .map(|(_, key)| key)
        .unwrap_or_else(|| panic!("`{written}` is not shadowed: {:?}", repl.shadows()))
}

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}

fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}

fn op(f: &str, a: Term, b: Term) -> Term {
    app(app(g(f), a), b)
}

fn nat(n: usize) -> String {
    (0..n).fold("Zero".to_string(), |acc, _| format!("(Succ {acc})"))
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The ring oracle
// ─────────────────────────────────────────────────────────────────────────────────────────────

const FIRST: &str = "Definition add : Int -> Int -> Int := fun a : Int => fun b : Int => a.";

const COMMUTES: &str = "## Theorem: add_flips
    Statement: forall x: Int, forall y: Int, add(x, y) equals add(y, x).
    Proof: ring.";

/// A document that rebinds `add` over `Int` — the prelude's own signature — rebinds the NAME. A
/// ring proof of `add x y = add y x` is then a claim about `λ a b. a`, and it is refused; the
/// oracle, asked about the shadow's identity, answers nothing. The environment's `add` is still
/// the ring's: the same statement over it, reached through `_root_`, is proved.
#[test]
fn a_rebound_arithmetic_name_never_inherits_the_ring_interpretation() {
    let mut pristine = Repl::new();
    ok(&mut pristine, COMMUTES);

    let mut repl = session(FIRST);
    let key = shadow_key(&repl, "add");
    refused(&mut repl, COMMUTES);
    assert!(
        repl.context().get_definition_body("add_flips").is_none(),
        "nothing was registered"
    );

    let ctx: &Context = repl.context();
    let (x, y) = (g("x"), g("y"));
    let mut with_atoms = ctx.clone();
    with_atoms.add_declaration("x", g("Int"));
    with_atoms.add_declaration("y", g("Int"));
    assert!(
        prove_int_eq(
            &with_atoms,
            &op(&key, x.clone(), y.clone()),
            &op(&key, y.clone(), x.clone())
        )
        .is_none(),
        "the oracle never reads the shadow's identity as ring addition"
    );
    assert!(
        prove_int_eq(
            &with_atoms,
            &op("add", x.clone(), y.clone()),
            &op("add", y, x)
        )
        .is_some(),
        "…and still proves the environment's `add` commutative"
    );
    let goal = "SApp (SApp (SApp (SName \"Eq\") (SName \"Int\")) (SApp (SApp (SName \"add\") (SLit 2)) (SLit 3))) (SLit 5)";
    ok(
        &mut repl,
        &format!("Definition root_goal : Syntax := {goal}."),
    );
    ok(
        &mut repl,
        "Definition root_ring : Derivation := try_ring root_goal.",
    );
    assert_eq!(
        ok(&mut repl, "Eval concludes root_ring."),
        ok(&mut repl, "Eval root_goal.")
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Names derived from a shadowing declaration
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A recursor is spelled from its inductive, so it follows the inductive's shadow: `B2_rec` is the
/// recursor of the `B2` the name now denotes, and `_root_.B2_rec` is the environment's.
#[test]
fn a_derived_name_follows_the_shadow_of_its_declaration() {
    let mut repl = session(
        "Inductive B2 := b1 : B2.
         Definition only_b1 : B2 -> Nat := fun x : B2 => match x return (fun _ : B2 => Nat) with | b1 => Zero end.
         Inductive B2 := b1 : B2 | b2 : B2.",
    );
    let fresh = shadow_key(&repl, "B2");
    let fresh_rec = format!("{fresh}_rec");
    assert!(
        repl.context().get_global(&fresh_rec).is_some(),
        "the shadow has its own recursor"
    );
    let written = ok(&mut repl, "Check B2_rec.");
    let root = ok(&mut repl, "Check _root_.B2_rec.");
    assert!(written.starts_with("B2_rec : "), "{written}");
    assert!(root.starts_with("_root_.B2_rec : "), "{root}");
    assert!(
        written.contains("b2"),
        "`B2_rec` eliminates the two-constructor `B2`: {written}"
    );
    assert!(
        !root.contains("b2"),
        "`_root_.B2_rec` eliminates the one-constructor `B2`: {root}"
    );
    assert!(
        !written.contains('\u{27e8}') && !root.contains('\u{27e8}'),
        "no identity is printed"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Namespaces and opens
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A shadow is keyed by the ABSOLUTE name, so it is reached however the name is written: qualified,
/// short inside the namespace, short under an `open` — and `_root_` reaches the declaration beneath.
#[test]
fn a_shadowed_name_in_a_namespace_resolves_through_every_spelling() {
    let mut repl = session(
        "Namespace M.
         Definition f : Nat := Zero.
         End M.
         Definition uses_f : Nat := M.f.
         Namespace M.
         Definition f : Nat := Succ Zero.
         End M.",
    );
    shadow_key(&repl, "M.f");
    assert_eq!(ok(&mut repl, "Eval M.f."), nat(1));
    assert_eq!(
        ok(&mut repl, "Eval uses_f."),
        "Zero",
        "the dependent keeps the first `M.f`"
    );
    assert_eq!(ok(&mut repl, "Eval _root_.M.f."), "Zero");
    assert_eq!(ok(&mut repl, "Check _root_.M.f."), "_root_.M.f : Nat");
    ok(&mut repl, "Namespace M.");
    assert_eq!(
        ok(&mut repl, "Eval f."),
        nat(1),
        "the short name inside the namespace"
    );
    ok(&mut repl, "End M.");
    ok(&mut repl, "Open M.");
    assert_eq!(
        ok(&mut repl, "Eval f."),
        nat(1),
        "the short name under an `open`"
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// The literate door
// ─────────────────────────────────────────────────────────────────────────────────────────────

const TWO_B2: &str = "Inductive B2 := b1 : B2.
     Definition only_b1 : B2 -> Nat := fun x : B2 => match x return (fun _ : B2 => Nat) with | b1 => Zero end.
     Inductive B2 := b1 : B2 | b2 : B2.";

/// A clause is compiled against the constructors of the inductive its discriminant really has, so
/// a pattern name must mean what it means here: `b1` is the shadowing `B2`'s first constructor.
#[test]
fn a_literate_pattern_names_the_constructor_its_name_denotes() {
    let mut repl = session(TWO_B2);
    repl.execute_document(
        "## To pick (x: B2) -> Nat:
    Consider x:
        When b1: Yield Zero.
        When b2: Yield Succ Zero.
",
    )
    .expect("an equation set over the shadowing `B2`");
    assert_eq!(ok(&mut repl, "Eval pick b2."), nat(1));
    assert_eq!(ok(&mut repl, "Eval pick b1."), "Zero");
}

/// An equation set's bodies are elaborated while the door reads them, so they are read with the
/// names they are written under: `add(n, 1)` against the session's `Nat` addition takes `1` as a
/// `Nat`, not as the `Int` the environment's `add` would ask for.
#[test]
fn a_literate_clause_body_is_elaborated_against_the_names_it_is_written_under() {
    let mut repl = session(
        "## To add (n: Nat) and (m: Nat) -> Nat:
    Consider n:
        When Zero: Yield m.
        When Succ k: Yield Succ (add(k, m)).

## To bump (n: Nat) -> Nat:
    Consider n:
        When Zero: Yield Zero.
        When Succ k: Yield add(n, 1).
",
    );
    assert_eq!(ok(&mut repl, "Check bump."), "bump : Nat -> Nat");
    ok(
        &mut repl,
        &format!(
            "Theorem bump_two : Eq Nat (bump {}) 3 := refl Nat 3.",
            nat(2)
        ),
    );
    refused(
        &mut repl,
        &format!(
            "Theorem bump_two_wrong : Eq Nat (bump {}) 2 := refl Nat 2.",
            nat(2)
        ),
    );
}

/// The literate door builds an induction proof out of the prelude's own vocabulary — `Zero` and
/// `Succ` index the case being closed. A session whose `Zero` is its own must not change what that
/// vocabulary means: what the door hands on is spelled so the REPL reads back exactly the constants
/// the door built with.
#[test]
fn the_proof_vocabulary_the_literate_door_builds_with_is_the_preludes() {
    let mut repl = session(
        "Inductive Mark := Zero : Mark | Tick : Mark -> Mark.
         Definition zero_mark : Mark := Zero.",
    );
    shadow_key(&repl, "Zero");
    ok(
        &mut repl,
        "## Theorem: every_nat_is_itself
    Statement: forall h : Nat, h equals h.
    Proof: induction h.
    - refl.
    - refl.",
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Blocks, qualifiers, failure
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A mutual block declared again with different constructors, under a dependent, shadows the
/// members it redeclares — the block is registered at fresh identities as one block.
#[test]
fn a_mutual_block_redeclared_under_a_dependent_is_shadowed_whole() {
    let mut repl = session(
        "Inductive Tree := leaf : Tree | node : Forest -> Tree.
         Inductive Forest := empty : Forest | grow : Tree -> Forest -> Forest.
         Definition sprout : Tree := node empty.",
    );
    repl.execute_document(
        "Inductive Tree := leaf : Tree | node : Forest -> Tree | pair : Tree -> Tree -> Tree.
         Inductive Forest := empty : Forest | grow : Tree -> Forest -> Forest.",
    )
    .expect("the wider block, under `sprout`");
    let tree = shadow_key(&repl, "Tree");
    assert_eq!(
        repl.context().get_constructors("Tree").len(),
        2,
        "the first `Tree` keeps its constructors"
    );
    assert_eq!(repl.context().get_constructors(&tree).len(), 3);
    assert_eq!(
        ok(&mut repl, "Check pair leaf leaf."),
        "((pair leaf) leaf) : Tree"
    );
    assert_eq!(ok(&mut repl, "Check sprout."), "sprout : _root_.Tree");
}

/// `Protected` changes which short name reaches a declaration; on a shadowing declaration it
/// protects the declaration the written name now denotes.
#[test]
fn a_protected_declaration_can_shadow() {
    let mut repl = session(
        "Namespace P.
         Definition v : Nat := Zero.
         End P.
         Definition uses_v : Nat := P.v.",
    );
    ok(&mut repl, "Namespace P.");
    ok(&mut repl, "Protected Definition v : Nat := Succ Zero.");
    refused(&mut repl, "Eval v.");
    ok(&mut repl, "End P.");
    assert_eq!(ok(&mut repl, "Eval P.v."), nat(1));
    assert_eq!(ok(&mut repl, "Eval uses_v."), "Zero");
}

/// A shadowing declaration that does not check leaves nothing behind: no identity, no shadow, no
/// partial registration — and the stable-meaning refusal is not what the user is told, the real
/// reason is.
#[test]
fn a_shadowing_declaration_that_does_not_check_leaves_nothing_behind() {
    let mut repl = session(
        "Definition two : Nat := Succ (Succ Zero).
         Definition uses_two : Nat := two.",
    );
    let before: Vec<String> = repl
        .context()
        .iter_definitions()
        .map(|(k, _, _)| k.to_string())
        .collect();
    let why = refused(&mut repl, "Definition two : Nat := true.");
    assert!(
        !matches!(
            why,
            InterfaceError::Kernel(KernelError::StableMeaningViolation { .. })
        ),
        "the refusal is the ill-typed body, not the gate: {why}"
    );
    assert!(
        repl.shadows().is_empty(),
        "no shadow survives a refused declaration: {:?}",
        repl.shadows()
    );
    let after: Vec<String> = repl
        .context()
        .iter_definitions()
        .map(|(k, _, _)| k.to_string())
        .collect();
    assert_eq!(after, before, "no identity was registered");
    assert_eq!(ok(&mut repl, "Eval two."), nat(2));
}

/// A name nobody depends on is replaced in place even in a session that shadows other names, and an
/// identical redeclaration of a shadow is kept as it is: neither mints.
#[test]
fn only_a_change_of_meaning_under_a_dependent_mints() {
    let mut repl = session(FIRST);
    let key = shadow_key(&repl, "add");
    ok(&mut repl, FIRST);
    assert_eq!(
        shadow_key(&repl, "add"),
        key,
        "the identical declaration is the same identity"
    );
    ok(
        &mut repl,
        "Definition add : Int -> Int -> Int := fun a : Int => fun b : Int => b.",
    );
    assert_eq!(
        shadow_key(&repl, "add"),
        key,
        "nothing depends on the shadow, so it is replaced in place"
    );
    assert_eq!(repl.shadows().len(), 1);
    assert_eq!(ok(&mut repl, "Eval add 2 3."), "3");
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// What shadowing costs a document that does not shadow
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A command runs under the shadowing snapshot only when it redeclares a name the environment
/// already holds. The snapshot keeps a second reference to every table, so while it lives the
/// command's first write to each table copies that whole table: a document of fresh declarations
/// must take none. Counted, never timed.
#[test]
fn a_document_of_fresh_declarations_takes_no_snapshot() {
    const DEFINITIONS: usize = 1960;
    const INDUCTIVES: usize = 40;
    let mut document = String::new();
    for i in 0..INDUCTIVES {
        document.push_str(&format!(
            "Inductive Fresh{i} := first{i} : Fresh{i} | second{i} : Fresh{i}.\n"
        ));
    }
    for i in 0..DEFINITIONS {
        document.push_str(&format!("Definition fresh{i} : Nat := Succ Zero.\n"));
    }
    let repl = session(&document);
    assert_eq!(
        repl.shadow_snapshots(),
        0,
        "{} fresh declarations took a snapshot",
        DEFINITIONS + INDUCTIVES
    );
    assert!(repl.shadows().is_empty());
    assert!(repl
        .context()
        .get_definition_body(&format!("fresh{}", DEFINITIONS - 1))
        .is_some());
}

/// …and a command that does redeclare takes exactly one, whatever the gate then decides: a shadow,
/// a replacement in place, or an identical redeclaration kept as it is.
#[test]
fn each_redeclaring_command_takes_exactly_one_snapshot() {
    let mut repl = session(
        "Definition base0 : Nat := Zero.
         Definition use0 : Nat := base0.
         Definition base1 : Nat := Zero.
         Definition use1 : Nat := base1.
         Definition base2 : Nat := Zero.
         Definition use2 : Nat := base2.
         Definition scratch0 : Nat := Zero.
         Definition scratch1 : Nat := Zero.
         Definition same0 : Nat := Zero.
         Definition same1 : Nat := Zero.
         Inductive Hue := red : Hue | green : Hue.
         Definition fav : Hue := red.",
    );
    assert_eq!(
        repl.shadow_snapshots(),
        0,
        "the fresh declarations took none"
    );
    let redeclarations = [
        "Definition base0 : Nat := Succ Zero.",
        "Definition base1 : Nat := Succ Zero.",
        "Definition base2 : Nat := Succ Zero.",
        "Definition scratch0 : Nat := Succ Zero.",
        "Definition scratch1 : Nat := Succ Zero.",
        "Definition same0 : Nat := Zero.",
        "Definition same1 : Nat := Zero.",
        "Definition add : Nat -> Nat -> Nat := fun a : Nat => fun b : Nat => a.",
        "Inductive Hue := red : Hue | green : Hue | blue : Hue.",
    ];
    for (taken, command) in redeclarations.iter().enumerate() {
        ok(&mut repl, command);
        assert_eq!(
            repl.shadow_snapshots(),
            taken + 1,
            "`{command}` takes exactly one snapshot"
        );
        ok(
            &mut repl,
            &format!("Definition after{taken} : Nat := Zero."),
        );
        assert_eq!(
            repl.shadow_snapshots(),
            taken + 1,
            "a fresh declaration after it takes none"
        );
    }
    let shadowed: Vec<String> = repl
        .shadows()
        .into_iter()
        .map(|(written, _)| written)
        .collect();
    assert_eq!(
        shadowed,
        ["base0", "base1", "base2", "add", "Hue", "red", "green"],
        "only the relied-on names mint"
    );
}

/// An inductive redeclared under a dependent mints — its name, then its constructor — and only then
/// fails on an ill-typed constructor. Nothing of the minting survives the refusal: no shadow, so the
/// environment's `b1` is still written `b1` and is still what `use_b1` computes to; no identity is
/// registered; and the refusal is the constructor's, not the gate's.
#[test]
fn an_inductive_that_mints_and_then_fails_leaves_nothing_behind() {
    let mut repl = session(
        "Inductive B := b1 : B.
         Definition use_b1 : B := b1.",
    );
    let before: Vec<String> = repl
        .context()
        .iter_definitions()
        .map(|(k, _, _)| k.to_string())
        .collect();
    let why = refused(&mut repl, "Inductive B := b1 : B | b2 : Nat.");
    assert!(
        matches!(
            why,
            InterfaceError::Kernel(KernelError::InvalidDeclaration(_))
        ),
        "the refusal is the constructor's result type, not the gate: {why}"
    );
    assert!(
        repl.shadows().is_empty(),
        "no shadow survives the refused inductive: {:?}",
        repl.shadows()
    );
    let after: Vec<String> = repl
        .context()
        .iter_definitions()
        .map(|(k, _, _)| k.to_string())
        .collect();
    assert_eq!(after, before, "no identity was registered");
    assert_eq!(ok(&mut repl, "Eval use_b1."), "b1");
    assert_eq!(ok(&mut repl, "Check b1."), "b1 : B");
}
