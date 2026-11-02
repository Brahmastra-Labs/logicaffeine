//! Lane V_vernacular — the namespace/`open` surface, and the locks that keep the vernacular door
//! honest once it grew one.
//!
//! Corpus cases T02-011 and T02-019 ask for the one thing without which no real Lean development can
//! be transcribed at all: a hierarchical `Name` and namespace-relative resolution. This file is the
//! half the corpus does NOT ask for — the neighbouring things the new resolver must still REFUSE,
//! the boundaries one rung above and below each rule, and the two REPL soundness gates
//! (conformance DEFECT-4 and DEFECT-7) that live on this same surface and would rot unpinned.
//!
//! # Where namespaces live, and why that is following Lean rather than diverging from it
//!
//! Lean's kernel `Environment` maps ABSOLUTE `Name`s to constants. `namespace`/`end`/`open` are
//! elaborator `Scope` state, and `resolveGlobalName` runs to completion BEFORE anything reaches the
//! kernel — which is exactly what T02-019's own title says ("name resolution completes before
//! translation begins"). So LOGOS puts the scope state on the `Surface` and runs resolution in the
//! `Repl`, the elaboration driver, and leaves `Context` a flat absolute map. That preserves the
//! Milner invariant (the kernel knows no surface) and matches Lean's staging exactly.

#![allow(clippy::bool_assert_comparison)]

use logicaffeine_kernel::interface::Repl;

// =============================================================================================
// THE T12-011 SOUNDNESS PROBE — is Surface C's new binder REAL, or dropped before `ring` runs?
//
// `AUDIT-T12-011` says the corpus guard `surface_c("… Statement: forall n : Nat, (add n 0) equals
// n. Proof: ring.").is_err()` now returns Ok. On a TRUE theorem, "the binder parsed" and "the
// binder was silently dropped and `ring` proved the open identity" are indistinguishable. The one
// experiment that separates them is a FALSE quantified statement: a dropped binder leaves `ring`
// an open identity it still cannot prove, but so does a real binder — so the decisive probe is
// whether the gate REFUSES falsehood at all, under the binder.
// =============================================================================================

fn surface_c(src: &str) -> Result<(), String> {
    Repl::new()
        .execute(src)
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}

/// A FALSE universally quantified statement must not certify on Surface C, whatever the binder
/// does. `∀ n : Nat, n + 1 = n` is false for every `n`; `ring` is a sound decision procedure and
/// must not close it. If this ever passes, the REPL's conclusion gate (`repl.rs`, the `proof_goal`
/// arm) is admitting an arbitrary proposition and the binder question is moot.
#[test]
fn a_false_quantified_statement_is_refused_on_the_literate_surface() {
    let false_thm = "## Theorem: FalseQuantified\n    \
                     Statement: forall n : Nat, (add n 1) equals n.\n    Proof: ring.";
    assert!(
        surface_c(false_thm).is_err(),
        "`∀ n : Nat, n + 1 = n` is FALSE and must be refused: {:?}",
        surface_c(false_thm)
    );

    // The unquantified twin, to show the refusal is about the CLAIM and not about the binder
    // form: with the binder erased the same falsehood must still be refused.
    let false_open = "## Theorem: FalseOpen\n    Statement: (add n 1) equals n.\n    Proof: ring.";
    assert!(
        surface_c(false_open).is_err(),
        "`n + 1 = n` is FALSE and must be refused with or without a binder: {:?}",
        surface_c(false_open)
    );

    // …and the paired positive, so a gate that refuses EVERYTHING cannot satisfy the two above.
    let true_thm = "## Theorem: TrueQuantified\n    \
                    Statement: forall n : Nat, (add n 0) equals n.\n    Proof: ring.";
    assert!(
        surface_c(true_thm).is_ok(),
        "`∀ n : Nat, n + 0 = n` is TRUE and `ring` decides it — a gate that refuses this is \
         refusing by reflex, not by checking: {:?}",
        surface_c(true_thm)
    );
}

// =============================================================================================
// DEFECT-4, AT THE SURFACE — a DECLARED type is a CLAIM, and a claim is checked.
// =============================================================================================

/// `Definition oops : False := I.` must be refused. `infer_type` alone only says the body is
/// well-formed SOMEHOW; without comparing against the declared type the REPL stamps any term with
/// any type, and a closed inhabitant of `False` is reachable from the ordinary user surface.
#[test]
fn a_declared_type_is_checked_against_the_body() {
    let mut repl = Repl::new();
    let oops = repl.execute("Definition oops : False := Zero.");
    assert!(
        oops.is_err(),
        "`Definition oops : False := Zero.` must be REFUSED — `Zero : Nat`, not `False`. \
         From a proof of False every proposition follows: {oops:?}"
    );
    assert!(
        repl.context().get_global("oops").is_none(),
        "a refused definition must not be registered at all: {:?}",
        repl.context().get_global("oops")
    );

    // The paired control — a correctly typed definition must still land and still unfold.
    let mut ok = Repl::new();
    ok.execute("Definition one : Nat := (Succ Zero).")
        .expect("a well-typed definition registers");
    assert_eq!(
        ok.execute("Eval one.").unwrap().trim(),
        "(Succ Zero)",
        "a well-typed definition must still δ-unfold"
    );
}

// =============================================================================================
// DEFECT-7, AT THE SURFACE — a failed tactic yields a well-TYPED error derivation, so type
// checking alone accepts a false theorem. The conclusion gate is what refuses it.
// =============================================================================================

/// A literate theorem whose statement is FALSE must be refused, and its name must not be
/// registered. `5 < 2` is the corpus's own example.
#[test]
fn a_false_literate_theorem_is_refused_and_leaves_no_global() {
    let mut repl = Repl::new();
    let bad = repl.execute("## Theorem: FiveLessThanTwo\n    Statement: 5 < 2.\n    Proof: lia.");
    assert!(
        bad.is_err(),
        "`5 < 2` must be refused by the conclusion gate: {bad:?}"
    );
    assert!(
        repl.context().get_global("FiveLessThanTwo").is_none(),
        "a refused theorem must leave NO global behind — a name in the context is a citable \
         lemma: {:?}",
        repl.context().get_global("FiveLessThanTwo")
    );

    // The paired positive: the same gate must still ADMIT a true one.
    let mut good = Repl::new();
    let ok = good.execute("## Theorem: TwoLessThanFive\n    Statement: 2 < 5.\n    Proof: lia.");
    assert!(ok.is_ok(), "`2 < 5` is true and `lia` decides it: {ok:?}");
    assert!(
        good.context().get_global("TwoLessThanFive").is_some(),
        "an admitted theorem must be registered so later proofs can cite it"
    );
}

// =============================================================================================
// HIERARCHICAL NAMES — the half the corpus asks for, and the boundaries it does not
// =============================================================================================

/// Three Lean names that differ only in where the separator falls must be three distinct keys, and
/// each must be REFERABLE and not merely storable. The corpus pins the storage half; this pins that
/// every one of them evaluates back to what it was declared as.
#[test]
fn a_dotted_name_is_declared_stored_and_referred_to() {
    let mut repl = Repl::new();
    repl.execute("Definition Outer.value : Nat := (Succ Zero).")
        .expect("dotted");
    repl.execute("Definition Outer.Inner.value : Nat := (Succ (Succ Zero)).")
        .expect("nested");
    repl.execute("Definition Outer_value : Nat := (Succ (Succ (Succ Zero))).")
        .expect("flat twin");

    assert_eq!(
        repl.execute("Eval Outer.value.").unwrap().trim(),
        "(Succ Zero)"
    );
    assert_eq!(
        repl.execute("Eval Outer.Inner.value.").unwrap().trim(),
        "(Succ (Succ Zero))",
        "a THREE-component name must resolve as one name, not as a projection of a projection"
    );
    assert_eq!(
        repl.execute("Eval Outer_value.").unwrap().trim(),
        "(Succ (Succ (Succ Zero)))"
    );

    // A `Check` reports the name the user WROTE resolved to its absolute key, so the answer names
    // the constant that was actually used.
    let checked = repl.execute("Check Outer.Inner.value.").expect("check");
    assert!(
        checked.starts_with("Outer.Inner.value :"),
        "the resolved absolute key must be what gets reported: {checked}"
    );
}

/// THE NEGATIVE. A dotted spelling that denotes no declared constant must still FAIL. Making
/// `a.b.c` resolvable must not make it resolve to something else, and must not make it silently
/// succeed as a projection of a name nothing declared.
#[test]
fn a_dotted_name_that_denotes_nothing_is_refused() {
    let mut repl = Repl::new();
    repl.execute("Definition Outer.value : Nat := (Succ Zero).")
        .expect("dotted");

    for missing in [
        "Eval Outer.missing.",
        "Eval Nowhere.value.",
        "Eval Outer.value.deeper.",
    ] {
        assert!(
            repl.execute(missing).is_err(),
            "`{missing}` names nothing and must be refused, not resolved to a neighbour: {:?}",
            repl.execute(missing)
        );
    }

    // And the PROJECTION reading is still reachable: `Zero.field` is not a name, so it is read as a
    // field access and fails as one rather than as an unknown global. The resolver must not have
    // eaten the projection grammar on its way past.
    let projected = repl.execute("Eval Zero.field.");
    assert!(
        projected.is_err(),
        "a field access on a non-structure must still fail: {projected:?}"
    );
}

/// THE BOUNDARY, both rungs. `Namespace Outer.Inner.` makes the short name `value` mean
/// `Outer.Inner.value` when there is one, and `Outer.value` when there is not — an inner namespace
/// SHADOWS an outer one, and dropping OUT of it changes the answer back.
#[test]
fn an_inner_namespace_shadows_an_outer_one_and_the_shadow_lifts() {
    let mut repl = Repl::new();
    repl.execute("Definition Outer.value : Nat := (Succ Zero).")
        .expect("Outer.value");
    repl.execute("Definition Outer.Inner.value : Nat := (Succ (Succ Zero)).")
        .expect("inner");
    repl.execute("Definition Outer.only : Nat := (Succ (Succ (Succ Zero))).")
        .expect("Outer.only");

    // ONE RUNG DOWN — inside `Outer.Inner`, `value` is the inner one and `only` falls back to the
    // outer namespace, which is the prefix walk being LONGEST-first rather than first-hit.
    repl.execute("Namespace Outer.Inner.")
        .expect("enter Outer.Inner");
    repl.execute("Definition fromInner : Nat := value.")
        .expect("inner value");
    repl.execute("Definition fellBack : Nat := only.")
        .expect("outer only");
    assert_eq!(
        repl.execute("Eval Outer.Inner.fromInner.").unwrap().trim(),
        "(Succ (Succ Zero))",
        "inside Outer.Inner the short name `value` is Outer.Inner.value, not Outer.value"
    );
    assert_eq!(
        repl.execute("Eval Outer.Inner.fellBack.").unwrap().trim(),
        "(Succ (Succ (Succ Zero)))",
        "`only` exists only in the OUTER namespace and must be found by walking out one step"
    );
    repl.execute("End Outer.Inner.").expect("leave Outer.Inner");

    // ONE RUNG UP — inside `Outer`, the same `value` is the outer one. The shadow was the scope's,
    // not the name's.
    repl.execute("Namespace Outer.").expect("enter Outer");
    repl.execute("Definition fromOuter : Nat := value.")
        .expect("outer value");
    repl.execute("End Outer.").expect("leave Outer");
    assert_eq!(
        repl.execute("Eval Outer.fromOuter.").unwrap().trim(),
        "(Succ Zero)",
        "inside Outer the short name `value` is Outer.value — the inner shadow is gone with the scope"
    );

    // AT THE ROOT — neither is in scope, so the short name denotes nothing at all.
    assert!(
        repl.execute("Definition atRoot : Nat := value.").is_err(),
        "at the root `value` names nothing; resolving it to one of the two would be a guess"
    );
}

/// A namespace must CLOSE. If `End` left the scope open, every later declaration would silently be
/// written into it — the most expensive kind of silence, because the file still checks.
#[test]
fn a_namespace_closes_and_its_declarations_stop_being_prefixed() {
    let mut repl = Repl::new();
    repl.execute("Namespace Box.").expect("enter");
    repl.execute("Definition inside : Nat := Zero.")
        .expect("declare inside");
    repl.execute("End Box.").expect("leave");
    repl.execute("Definition outside : Nat := Zero.")
        .expect("declare outside");

    assert!(
        repl.context().get_global("Box.inside").is_some(),
        "a declaration inside `Namespace Box` is keyed Box.inside"
    );
    assert!(
        repl.context().get_global("inside").is_none(),
        "…and NOT at the bare short name, or the namespace bought nothing"
    );
    assert!(
        repl.context().get_global("outside").is_some(),
        "a declaration after `End Box` is keyed at the root: {:?}",
        repl.context().get_global("Box.outside")
    );
    assert!(
        repl.context().get_global("Box.outside").is_none(),
        "…and NOT inside the closed namespace"
    );

    // A mismatched `End` must be refused rather than pop the wrong scope.
    repl.execute("Namespace Deep.").expect("enter Deep");
    assert!(
        repl.execute("End Shallow.").is_err(),
        "`End Shallow` must not close `Namespace Deep`"
    );
    repl.execute("End Deep.")
        .expect("the right End still closes it");

    // `Namespace` declares NOTHING. It is a way of spelling, like every other surface command.
    assert!(
        repl.context().get_global("Box").is_none(),
        "a namespace command must not put a phantom global in the context"
    );
}

/// `End` serves sections and namespaces both, exactly as Lean's does, and must pick by which scope
/// is innermost rather than by guessing.
#[test]
fn one_end_keyword_closes_whichever_scope_is_innermost() {
    let mut repl = Repl::new();
    repl.execute("Section Sec.").expect("open a section");
    repl.execute("Namespace Ns.")
        .expect("open a namespace inside it");
    // The namespace is innermost: `End Sec.` must not reach past it.
    assert!(
        repl.execute("End Sec.").is_err(),
        "the innermost open scope is the namespace Ns — `End Sec` must not reach past it"
    );
    repl.execute("End Ns.").expect("the namespace closes");
    repl.execute("End Sec.").expect("and then the section does");
    // Back at the root: a declaration is unprefixed.
    repl.execute("Definition plain : Nat := Zero.")
        .expect("plain");
    assert!(repl.context().get_global("plain").is_some());
}

// =============================================================================================
// `Open` — and the ambiguity that is REPORTED rather than picked
// =============================================================================================

/// `Open Alpha.` makes `Alpha`'s short names denote `Alpha`'s constants.
#[test]
fn open_brings_a_namespaces_short_names_into_scope() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");
    assert!(
        repl.execute("Definition beforeOpen : Nat := token.")
            .is_err(),
        "without the `open`, `token` names nothing"
    );
    repl.execute("Open Alpha.").expect("open Alpha");
    repl.execute("Definition afterOpen : Nat := token.")
        .expect("token is Alpha.token now");
    assert_eq!(
        repl.execute("Eval afterOpen.").unwrap().trim(),
        "(Succ Zero)",
        "under `Open Alpha`, `token` denotes Alpha.token"
    );
}

/// ★ THE NEGATIVE THAT MATTERS. Two `open`s, one short name, two constants: REPORTED, and the
/// message names BOTH candidates.
///
/// This is the failure mode the audit called the dangerous one. A resolver that picks silently
/// produces a proof about a different proposition than the one written, and both readings
/// type-check. Lean errors here; we error here and say which two.
#[test]
fn a_short_name_under_two_opens_is_reported_as_ambiguous_naming_both() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");
    repl.execute("Definition Beta.token : Nat := (Succ (Succ Zero)).")
        .expect("Beta.token");
    repl.execute("Open Alpha.").expect("open Alpha");
    repl.execute("Open Beta.").expect("open Beta");

    let ambiguous = repl.execute("Definition which : Nat := token.");
    let message = match &ambiguous {
        Err(e) => e.to_string(),
        Ok(_) => panic!(
            "one short name answering to Alpha.token AND Beta.token must be REFUSED, not picked — \
             both readings type-check and they are different numbers"
        ),
    };
    assert!(
        message.contains("Alpha.token") && message.contains("Beta.token"),
        "the diagnosis must name BOTH candidates — `ambiguous` without the list is a puzzle: {message}"
    );
    assert!(
        repl.context().get_global("which").is_none(),
        "a refused declaration must leave no global"
    );

    // The disambiguation must WORK, or the error is a dead end.
    repl.execute("Definition fromAlpha : Nat := Alpha.token.")
        .expect("full name disambiguates");
    assert_eq!(
        repl.execute("Eval fromAlpha.").unwrap().trim(),
        "(Succ Zero)"
    );
    repl.execute("Definition fromBeta : Nat := Beta.token.")
        .expect("full name disambiguates");
    assert_eq!(
        repl.execute("Eval fromBeta.").unwrap().trim(),
        "(Succ (Succ Zero))"
    );
}

/// The SAME report when the second candidate is the PRELUDE's. This is the exact shape the audit
/// described: `open Alpha` where `Alpha.Zero` exists means the short name `Zero` answers to two
/// constants, and resolving it to the prelude's constructor "changes what the proof is about".
/// `_root_.Zero` is the escape, and it must denote the prelude's.
#[test]
fn a_short_name_shadowing_the_prelude_is_reported_and_root_escapes_it() {
    let mut repl = Repl::new();
    // The body says `_root_.Zero` because `Zero` inside `Alpha` IS the name being declared — see
    // `a_declarations_own_short_name_is_a_self_reference_inside_its_namespace` below.
    repl.execute("Definition Alpha.Zero : Nat := (Succ _root_.Zero).")
        .expect("Alpha.Zero");
    repl.execute("Open Alpha.").expect("open Alpha");

    let shadowed = repl.execute("Definition ambiguousZero : Nat := Zero.");
    let message = match &shadowed {
        Err(e) => e.to_string(),
        Ok(_) => panic!(
            "under `Open Alpha`, `Zero` answers to Alpha.Zero AND the prelude's Zero. Picking the \
             prelude's silently is the audit's own example of a change nothing reports"
        ),
    };
    assert!(
        message.contains("Alpha.Zero") && message.contains("`Zero`"),
        "both readings must be named: {message}"
    );

    // `_root_` is the way to say "the top-level one", and it must consult nothing else.
    repl.execute("Definition preludeZero : Nat := _root_.Zero.")
        .expect("_root_ escapes the open");
    assert_eq!(
        repl.execute("Eval preludeZero.").unwrap().trim(),
        "Zero",
        "`_root_.Zero` is the prelude's Zero, whatever is open"
    );
    repl.execute("Definition alphasZero : Nat := Alpha.Zero.")
        .expect("full name works too");
    assert_eq!(
        repl.execute("Eval alphasZero.").unwrap().trim(),
        "(Succ Zero)"
    );
}

/// `Open Alpha in <command>` scopes the opening to EXACTLY one command. If it leaked, every later
/// short name would read in a scope nobody wrote — and the file would still check.
#[test]
fn a_scoped_open_covers_exactly_one_command() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");

    repl.execute("Open Alpha in Definition scoped : Nat := token.")
        .expect("`Open Alpha in <command>` must scope the opening to that command");
    assert_eq!(
        repl.execute("Eval scoped.").unwrap().trim(),
        "(Succ Zero)",
        "the bracketed command sees Alpha's short names"
    );

    assert!(
        repl.execute("Definition after : Nat := token.").is_err(),
        "…and the NEXT command must not. A scoped open that leaks is worse than none"
    );

    // It must be wound back on FAILURE too — a refused command cannot leave its scope behind.
    assert!(repl
        .execute("Open Alpha in Definition bad : Nat := nosuchname.")
        .is_err());
    assert!(
        repl.execute("Definition afterFailure : Nat := token.")
            .is_err(),
        "a FAILED scoped command must still wind its opening back"
    );
}

/// An `Open` written inside a namespace dies with that namespace — scope is a stack, not a pile.
#[test]
fn an_open_inside_a_namespace_dies_with_it() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");
    repl.execute("Namespace Host.").expect("enter Host");
    repl.execute("Open Alpha.")
        .expect("open inside the namespace");
    repl.execute("Definition uses : Nat := token.")
        .expect("visible inside");
    repl.execute("End Host.").expect("leave Host");
    assert_eq!(
        repl.execute("Eval Host.uses.").unwrap().trim(),
        "(Succ Zero)"
    );
    assert!(
        repl.execute("Definition afterEnd : Nat := token.").is_err(),
        "the `Open Alpha` written inside `Namespace Host` must not outlive it"
    );
}

/// THE NEGATIVE for `Open` itself: a namespace nothing has ever declared into does not exist, and
/// opening it must be refused. An `open` that brings in no names is indistinguishable from one that
/// worked, and every short name under it then quietly denotes the root's constant.
#[test]
fn opening_a_namespace_that_does_not_exist_is_refused() {
    let mut repl = Repl::new();
    let typo = repl.execute("Open Alpah.");
    assert!(
        typo.is_err(),
        "`Open Alpah.` names no namespace and must be refused, not accepted as a no-op: {typo:?}"
    );
    assert!(
        repl.execute("Open Alpha in Definition x : Nat := Zero.")
            .is_err(),
        "the scoped form must check the same thing"
    );

    // …and once something IS declared there, the same `Open` works. A check that refuses
    // everything is not a check.
    repl.execute("Definition Alpha.token : Nat := Zero.")
        .expect("Alpha.token");
    repl.execute("Open Alpha.").expect("now Alpha exists");

    // A REFUSED declaration must not bring a namespace into being.
    let mut fresh = Repl::new();
    assert!(
        fresh.execute("Definition Ghost.x : Nat := True.").is_err(),
        "mistyped body"
    );
    assert!(
        fresh.execute("Open Ghost.").is_err(),
        "a namespace only exists once something actually landed in it"
    );
}

// =============================================================================================
// EVERY OTHER DOOR — the resolver runs BEFORE elaboration, so it has to hold for all of them
// =============================================================================================

/// An inductive declared inside a namespace is keyed there, its constructors are keyed there, its
/// constructor types resolve to the prefixed type, and its recursor is derived. Registration is not
/// the test — the projections must COMPUTE.
#[test]
fn an_inductive_inside_a_namespace_registers_and_computes() {
    let mut repl = Repl::new();
    repl.execute("Namespace Col.").expect("enter Col");
    repl.execute("Inductive Hue := Red : Hue | Green : Hue.")
        .expect("inductive inside");
    repl.execute("Definition favourite : Hue := Red.")
        .expect("short ctor name resolves");
    repl.execute("End Col.").expect("leave Col");

    assert!(
        repl.context().is_inductive("Col.Hue"),
        "the type is keyed in the namespace"
    );
    assert!(
        repl.context().is_constructor("Col.Red"),
        "and so is its constructor"
    );
    assert!(
        !repl.context().is_inductive("Hue"),
        "…and the bare short name is NOT also registered"
    );
    assert_eq!(
        repl.context().constructor_inductive("Col.Red"),
        Some("Col.Hue"),
        "the constructor's type must have resolved to the PREFIXED inductive, not to a stray `Hue`"
    );
    assert_eq!(
        repl.execute("Eval Col.favourite.").unwrap().trim(),
        "Col.Red",
        "and it must compute, by the absolute key"
    );
    assert!(
        repl.context().get_global("Col.Hue_rec").is_some(),
        "the recursor is derived for the namespaced inductive too — declaring a type gives you \
         induction for free wherever it is declared"
    );
}

/// A recursive definition inside a namespace still finds ITSELF by its short name — the key it is
/// being given resolves before the context holds it, which is the one thing a resolver that only
/// consults the environment cannot do.
#[test]
fn a_recursive_definition_inside_a_namespace_binds_its_own_fix() {
    let mut repl = Repl::new();
    repl.execute("Namespace Arith.").expect("enter");
    repl.execute(
        "Definition double : Nat -> Nat := fun n : Nat => match n with          | Zero => Zero | Succ k => (Succ (Succ (double k))) end.",
    )
    .expect("a recursive definition must find itself by its short name inside its namespace");
    repl.execute("End Arith.").expect("leave");
    assert_eq!(
        repl.execute("Eval (Arith.double (Succ (Succ Zero))).")
            .unwrap()
            .trim(),
        "(Succ (Succ (Succ (Succ Zero))))",
        "…and it must actually recurse"
    );
}

/// The whole-document door reads the namespace commands as commands, in order — a `Namespace` in a
/// document is not a special case of anything.
#[test]
fn a_document_carries_its_namespace_scope_through() {
    let mut repl = Repl::new();
    let doc = "Namespace Doc.\n\
               Definition one : Nat := (Succ Zero).\n\
               Definition two : Nat := (Succ one).\n\
               End Doc.\n\
               Definition three : Nat := (Succ Doc.two).\n";
    repl.execute_document(doc)
        .expect("the document must read whole");
    assert_eq!(
        repl.execute("Eval Doc.two.").unwrap().trim(),
        "(Succ (Succ Zero))"
    );
    assert_eq!(
        repl.execute("Eval three.").unwrap().trim(),
        "(Succ (Succ (Succ Zero)))",
        "a name reached from outside the namespace by its absolute key"
    );
    assert!(
        repl.context().get_global("two").is_none(),
        "nothing in the document was declared at the root"
    );
}

/// A mutual block inside a namespace registers as a mutual block, prefixed, with its cross
/// references resolved — the two things that could each independently be missed.
#[test]
fn a_mutual_block_inside_a_namespace_registers_prefixed() {
    let mut repl = Repl::new();
    let doc = "Namespace Woods.\n\
               Inductive Tree := Leaf : Tree | Node : Forest -> Tree.\n\
               Inductive Forest := Nil2 : Forest | Grow : Tree -> Forest -> Forest.\n\
               End Woods.\n";
    repl.execute_document(doc)
        .expect("the mutual block must register");
    assert!(repl.context().is_inductive("Woods.Tree"), "Woods.Tree");
    assert!(repl.context().is_inductive("Woods.Forest"), "Woods.Forest");
    assert!(
        repl.context().mutual_block_of("Woods.Tree").is_some(),
        "a genuine cycle must still be recorded as a mutual block when it is namespaced"
    );
    assert_eq!(
        repl.context().constructor_inductive("Woods.Node"),
        Some("Woods.Tree"),
        "Node's constructor must be keyed in the namespace"
    );
    // `Node : Forest -> Tree` must have resolved `Forest` to `Woods.Forest`, not left a dangling
    // root-level name that happens to type-check because nothing looked.
    let node_ty = repl
        .context()
        .get_global("Woods.Node")
        .expect("Woods.Node")
        .to_string();
    assert!(
        node_ty.contains("Woods.Forest"),
        "the constructor's argument type must be the PREFIXED sibling: {node_ty}"
    );
}

/// `Definition _root_.top` declares the top-level name from inside a namespace, and elaborates at
/// the root — the escape has to work in the declaring direction too.
#[test]
fn a_root_escaped_declaration_lands_at_the_root() {
    let mut repl = Repl::new();
    repl.execute("Namespace Inner.").expect("enter");
    repl.execute("Definition _root_.top : Nat := (Succ Zero).")
        .expect("_root_ declaration");
    repl.execute("End Inner.").expect("leave");
    assert!(
        repl.context().get_global("top").is_some(),
        "`Definition _root_.top` inside a namespace declares the top-level `top`"
    );
    assert!(
        repl.context().get_global("Inner.top").is_none()
            && repl.context().get_global("Inner._root_.top").is_none(),
        "…and nothing inside the namespace"
    );
    assert_eq!(repl.execute("Eval top.").unwrap().trim(), "(Succ Zero)");
}

/// BOTH KERNELS. A namespaced definition is an ordinary term with a longer key, so the independent
/// re-checker must agree about it. Resolution happening before the kernel sees anything is exactly
/// what makes that true, and a test is the only thing that says so.
#[test]
fn both_kernels_agree_about_a_namespaced_definition() {
    use logicaffeine_kernel::{double_check, DoubleCheck, Term};
    let mut repl = Repl::new();
    repl.execute("Definition Outer.value : Nat := (Succ Zero).")
        .expect("dotted");
    repl.execute("Namespace Outer.").expect("enter");
    repl.execute("Definition uses : Nat := (Succ value).")
        .expect("relative reference");
    repl.execute("End Outer.").expect("leave");

    for key in ["Outer.value", "Outer.uses"] {
        assert_eq!(
            double_check(repl.context(), &Term::Global(key.to_string())),
            DoubleCheck::Agreed,
            "both kernels must agree about `{key}` — a namespace is a spelling, and a spelling \
             cannot reach the kernel at all"
        );
    }
}

/// The identity guarantee, stated as a test. With no namespace and no `open`, the resolver must
/// change NOTHING: every session that never writes `Namespace` or `Open` sees byte-identical
/// behaviour, which is what makes the subsystem free rather than opt-out.
#[test]
fn an_empty_scope_resolves_to_exactly_what_was_written() {
    let mut repl = Repl::new();
    repl.execute("Definition n : Nat := (Succ Zero).")
        .expect("plain");
    assert_eq!(repl.execute("Eval n.").unwrap().trim(), "(Succ Zero)");
    assert_eq!(repl.execute("Check Zero.").unwrap().trim(), "Zero : Nat");
    assert_eq!(
        repl.execute("Eval (Succ n).").unwrap().trim(),
        "(Succ (Succ Zero))"
    );
    // An unknown name is still reported as the name the user wrote, not as a candidate they did not.
    let unknown = repl.execute("Eval nosuchthing.").expect_err("unknown");
    assert!(
        unknown.to_string().contains("nosuchthing"),
        "an unresolved name must be reported at the spelling that was written: {unknown}"
    );
}

/// FOUND WHILE BUILDING THIS, and worth pinning on its own: inside `Alpha`, the short name `Zero`
/// IS `Alpha.Zero` — including in the body of `Definition Alpha.Zero` itself, where it is a
/// RECURSIVE self-reference and not the prelude's constructor.
///
/// Lean answers the same way (`def f := f` is a recursion Lean then fails to show terminating, not
/// an unknown identifier), so this is conformance and not an accident. It is pinned because the
/// alternative reading — the prelude's `Zero` — would make `Definition Alpha.Zero : Nat := Zero.`
/// quietly declare the identity of a name that shadows it, which is the exact silent substitution
/// the ambiguity report exists to prevent. `_root_.Zero` is how you say the other thing.
#[test]
fn a_declarations_own_short_name_is_a_self_reference_inside_its_namespace() {
    let mut repl = Repl::new();
    let recursive = repl.execute("Definition Alpha.Zero : Nat := (Succ Zero).");
    assert!(
        recursive.is_err(),
        "inside `Alpha`, the `Zero` in the body of `Alpha.Zero` is that very declaration — a \
         non-terminating self-reference, which must be refused rather than silently read as the \
         prelude's constructor: {recursive:?}"
    );
    assert!(
        repl.context().get_global("Alpha.Zero").is_none(),
        "and nothing may be registered from a refused declaration"
    );

    // THE ESCAPE, and the paired positive: say which one you meant and it lands.
    repl.execute("Definition Alpha.Zero : Nat := (Succ _root_.Zero).")
        .expect("`_root_.Zero` is unambiguously the prelude's, whatever is being declared");
    assert_eq!(
        repl.execute("Eval Alpha.Zero.").unwrap().trim(),
        "(Succ Zero)"
    );

    // A recursion that GENUINELY terminates is still accepted through the same channel, so the
    // refusal above is the termination guard doing its job and not a blanket ban on self-reference.
    let mut ok = Repl::new();
    ok.execute("Namespace Rec.").expect("enter");
    ok.execute(
        "Definition down : Nat -> Nat := fun n : Nat => match n with \
         | Zero => Zero | Succ k => (down k) end.",
    )
    .expect("a structurally decreasing self-reference by short name must be accepted");
    ok.execute("End Rec.").expect("leave");
    assert_eq!(
        ok.execute("Eval (Rec.down (Succ (Succ Zero))).")
            .unwrap()
            .trim(),
        "Zero"
    );
}

// =============================================================================================
// THE RESOLVER, DIRECTLY — the negatives the vernacular has no spelling for yet
//
// `p.fst` on a structure is reachable from the TERM parser but not from any vernacular command
// (there is no `Structure` command), so the one way to pin what the resolver does to a projection
// chain is to hand it one. These are the tests that caught a real corruption: the generic term walk
// descended into a projection and handed the FIELD name to the resolver.
// =============================================================================================

/// ★ A FIELD IS A LABEL, NOT A REFERENCE. Inside `Namespace Foo` with a `Foo.fst` declared, the
/// projection `p.fst` must stay `p.fst` — resolving the field would silently produce a projection by
/// `Foo.fst`, a field name no structure has, from source that never said it.
#[test]
fn a_projections_field_name_is_never_resolved() {
    use logicaffeine_kernel::interface::surface::{resolve_names, Name, Namespaces};
    use logicaffeine_kernel::{Term, DOT_MARKER};

    let proj = |base: Term, field: &str| {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global(DOT_MARKER.to_string())),
                Box::new(base),
            )),
            Box::new(Term::Global(field.to_string())),
        )
    };

    // `Foo` is current, and BOTH `Foo.fst` (a would-be field collision) and `Foo.p` exist.
    let mut ns = Namespaces::new();
    ns.record_declaration("Foo.fst");
    ns.record_declaration("Foo.p");
    ns.enter(&Name::parse("Foo").unwrap(), 1);
    let known = |k: &str| k == "Foo.fst" || k == "Foo.p";

    // `p.fst`: the receiver resolves to `Foo.p`, the field stays `fst`.
    let term = proj(Term::Global("p".to_string()), "fst");
    let out = resolve_names(&ns, &known, &mut Vec::new(), &term).expect("resolves");
    assert_eq!(
        *out,
        proj(Term::Global("Foo.p".to_string()), "fst"),
        "the RECEIVER resolves and the FIELD does not — `Foo.p.Foo.fst` would be a projection by a \
         field name nothing declared: {:?}",
        *out
    );

    // A receiver that is a LOCAL must not resolve either: `Var` is already bound, and a `Global`
    // shielded as a local (an inductive's type parameter) is bound too.
    let local = proj(Term::Global("p".to_string()), "fst");
    let out = resolve_names(&ns, &known, &mut vec!["p".to_string()], &local).expect("resolves");
    assert_eq!(
        *out, local,
        "a receiver that is BOUND here is not a global reference and must be left alone"
    );

    // And the whole-name reading still wins when the chain IS a name: `fst` under `Foo` is the
    // constant `Foo.fst`, reached as a bare global with no projection left over.
    let as_name = Term::Global("fst".to_string());
    let out = resolve_names(&ns, &known, &mut Vec::new(), &as_name).expect("resolves");
    assert_eq!(*out, Term::Global("Foo.fst".to_string()));
}

/// The resolution ORDER, stated directly, one rung at a time — the thing every namespace bug is
/// really a bug in. Lean's order is: `_root_` alone; then the enclosing namespace longest-prefix
/// first (a SHADOW); then every `open` together with the root at ONE priority (an AMBIGUITY).
#[test]
fn the_resolution_order_is_leans_order() {
    use logicaffeine_kernel::interface::surface::{Name, Namespaces, Resolution};

    let n = |s: &str| Name::parse(s).expect("a name");

    // Prefixes, longest first: `Outer.Inner` shadows `Outer` shadows the root — a shadow, never an
    // ambiguity, because Lean returns at the first prefix level that resolves.
    let mut ns = Namespaces::new();
    ns.record_declaration("Outer.Inner.x");
    ns.record_declaration("Outer.x");
    ns.enter(&n("Outer"), 1);
    ns.enter(&n("Inner"), 1);
    let all = |k: &str| ["Outer.Inner.x", "Outer.x", "x"].contains(&k);
    assert_eq!(
        ns.resolve(&n("x"), &all),
        Resolution::Found("Outer.Inner.x".into())
    );
    let no_inner = |k: &str| ["Outer.x", "x"].contains(&k);
    assert_eq!(
        ns.resolve(&n("x"), &no_inner),
        Resolution::Found("Outer.x".into()),
        "one rung out when the inner namespace has no such name"
    );
    let root_only = |k: &str| k == "x";
    assert_eq!(
        ns.resolve(&n("x"), &root_only),
        Resolution::Found("x".into()),
        "…and all the way out to the root"
    );
    assert_eq!(
        ns.resolve(&n("x"), &|_| false),
        Resolution::Unresolved("x".into()),
        "an unresolved name keeps the spelling that was WRITTEN — not a candidate nobody typed"
    );

    // `_root_` consults the enclosing namespace not at all, even when the relative reading exists.
    assert_eq!(
        ns.resolve(&n("_root_.x"), &all),
        Resolution::Found("x".into()),
        "`_root_.x` is the top-level x, whatever is current"
    );

    // Opens and the root sit at ONE priority, so two readings is an error and not a shadow.
    let mut opened = Namespaces::new();
    opened.record_declaration("Alpha.t");
    opened.record_declaration("Beta.t");
    opened.open(n("Alpha")).expect("Alpha exists");
    opened.open(n("Beta")).expect("Beta exists");
    let two = |k: &str| ["Alpha.t", "Beta.t"].contains(&k);
    match opened.resolve(&n("t"), &two) {
        Resolution::Ambiguous {
            written,
            candidates,
        } => {
            assert_eq!(written, "t");
            assert!(
                candidates.contains(&"Alpha.t".to_string())
                    && candidates.contains(&"Beta.t".to_string()),
                "both readings must be carried in the report: {candidates:?}"
            );
        }
        other => panic!("two opens answering to one short name must be AMBIGUOUS: {other:?}"),
    }
    // ONE open plus the root is the same group, so it is ambiguous too — this is the audit's
    // prelude-shadowing case, and the reason it is now reported instead of picked.
    let mut one = Namespaces::new();
    one.record_declaration("Alpha.Zero");
    one.open(n("Alpha")).expect("Alpha exists");
    assert!(
        matches!(
            one.resolve(&n("Zero"), &|k| ["Alpha.Zero", "Zero"].contains(&k)),
            Resolution::Ambiguous { .. }
        ),
        "an `open` name and a ROOT name at one priority is Lean's rule, and the silent pick it \
         replaces is the audit's own example of a change nothing reports"
    );
    // …and with only one reading available it resolves cleanly, so the check is not a blanket ban.
    assert_eq!(
        one.resolve(&n("Zero"), &|k| k == "Alpha.Zero"),
        Resolution::Found("Alpha.Zero".into())
    );
    assert_eq!(
        one.resolve(&n("Zero"), &|k| k == "Zero"),
        Resolution::Found("Zero".into())
    );
}

/// A `«guillemet name»` is ONE component even when it contains a dot — the escape that keeps a name
/// with a `.` in it writable, and the boundary the dotted grammar must not eat.
#[test]
fn a_guillemet_name_is_one_component_even_with_a_dot_in_it() {
    use logicaffeine_kernel::interface::surface::Name;
    let quoted = Name::parse("\u{ab}a.b\u{bb}").expect("a quoted name parses");
    assert_eq!(quoted.parts(), ["a.b"], "the quotes make it ONE component");
    let mixed = Name::parse("Outer.\u{ab}a.b\u{bb}").expect("mixed");
    assert_eq!(
        mixed.parts(),
        ["Outer", "a.b"],
        "two components, one of them quoted"
    );
    assert_eq!(
        Name::parse("Outer.Inner.value").unwrap().parts(),
        ["Outer", "Inner", "value"]
    );
    // A trailing separator is punctuation, not an empty component.
    assert_eq!(
        Name::scan("Outer.value.").map(|(n, l)| (n.key(), l)),
        Some(("Outer.value".into(), 11))
    );
    assert!(
        Name::parse("").is_none() && Name::parse(".x").is_none() && Name::parse("1bad").is_none()
    );
}

// =============================================================================================
// THE LITERATE DOOR, UNDER A NAMESPACE — the shape a transcribed Lean development actually has
// =============================================================================================

/// A literate `## Theorem … Statement … Proof:` block written inside a namespace must be keyed
/// there, must still be GATED, and must still be citable by its absolute name. This is the shape a
/// real transcribed development has — `namespace Nat … theorem add_zero …` — so it is the one that
/// decides whether namespaces are usable or merely present.
#[test]
fn a_literate_theorem_inside_a_namespace_is_keyed_there_and_still_gated() {
    let mut repl = Repl::new();
    repl.execute("Namespace Arith.").expect("enter Arith");
    repl.execute(
        "## Theorem: AddZero\n    Statement: forall n : Nat, (add n 0) equals n.\n    Proof: ring.",
    )
    .expect("a true literate theorem must certify inside a namespace");
    // THE NEGATIVE, in the same scope: the conclusion gate is not relaxed by being inside one.
    let false_thm = repl.execute(
        "## Theorem: AddOne\n    Statement: forall n : Nat, (add n 1) equals n.\n    Proof: ring.",
    );
    assert!(
        false_thm.is_err(),
        "a FALSE theorem inside a namespace must be refused exactly as at the root: {false_thm:?}"
    );
    repl.execute("End Arith.").expect("leave Arith");

    assert!(
        repl.context().get_global("Arith.AddZero").is_some(),
        "the theorem is keyed in its namespace, so a Lean `Nat.add_zero` has somewhere to live: {:?}",
        repl.context().get_global("AddZero")
    );
    assert!(
        repl.context().get_global("AddZero").is_none()
            && repl.context().get_global("Arith.AddOne").is_none(),
        "nothing at the root, and nothing from the refused one"
    );
}

/// A literate inductive inside a namespace — the other half of a transcribed development.
#[test]
fn a_literate_inductive_inside_a_namespace_is_keyed_there() {
    let mut repl = Repl::new();
    repl.execute("Namespace Logic.").expect("enter");
    repl.execute("A Verdict is either Yes or No.")
        .expect("literate inductive");
    repl.execute("End Logic.").expect("leave");
    assert!(
        repl.context().is_inductive("Logic.Verdict"),
        "the type is keyed in the namespace"
    );
    assert!(
        repl.context().is_constructor("Logic.Yes"),
        "and so are its constructors"
    );
    assert_eq!(
        repl.context().constructor_inductive("Logic.Yes"),
        Some("Logic.Verdict"),
        "the constructor's type resolved to the PREFIXED inductive"
    );
    assert!(
        !repl.context().is_inductive("Verdict"),
        "and the bare short name is not also registered"
    );
}

/// Nesting: `Open A in Open B in <command>` must layer, and BOTH layers must come off.
#[test]
fn nested_scoped_opens_layer_and_both_come_off() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.a : Nat := Zero.")
        .expect("Alpha.a");
    repl.execute("Definition Beta.b : Nat -> Nat := fun n : Nat => (Succ n).")
        .expect("Beta.b");
    repl.execute("Open Alpha in Open Beta in Definition both : Nat := (b a).")
        .expect("both openings must be in scope for the bracketed command");
    assert_eq!(
        repl.execute("Eval both.").unwrap().trim(),
        "(Succ Zero)",
        "`b a` — each short name denoted its OWN namespace's constant, from two layers of opening"
    );
    assert!(
        repl.execute("Definition afterA : Nat := a.").is_err(),
        "Alpha's opening came off"
    );
    assert!(
        repl.execute("Definition afterB : Nat -> Nat := b.")
            .is_err(),
        "Beta's did too"
    );
}

/// A `Namespace` inside a `Namespace` composes into one current namespace, and `End` peels one
/// layer at a time. The boundary is the KEY the declarations land at, at each depth.
#[test]
fn namespaces_nest_and_end_peels_one_layer() {
    let mut repl = Repl::new();
    repl.execute("Namespace A.").expect("A");
    repl.execute("Definition atA : Nat := Zero.").expect("atA");
    repl.execute("Namespace B.").expect("B");
    repl.execute("Definition atAB : Nat := Zero.")
        .expect("atAB");
    repl.execute("End B.").expect("close B");
    repl.execute("Definition backAtA : Nat := Zero.")
        .expect("backAtA");
    repl.execute("End A.").expect("close A");
    for key in ["A.atA", "A.B.atAB", "A.backAtA"] {
        assert!(
            repl.context().get_global(key).is_some(),
            "{key} must be the key"
        );
    }
    for absent in ["atA", "B.atAB", "A.atAB", "A.B.backAtA"] {
        assert!(
            repl.context().get_global(absent).is_none(),
            "{absent} must NOT be a key"
        );
    }
    // `Namespace A.B.` in one command is the same place as two nested commands.
    let mut oneshot = Repl::new();
    oneshot.execute("Namespace A.B.").expect("A.B in one go");
    oneshot
        .execute("Definition atAB : Nat := Zero.")
        .expect("atAB");
    assert!(oneshot.context().get_global("A.B.atAB").is_some());
    assert!(
        oneshot.execute("End B.").is_err(),
        "`Namespace A.B.` is ONE frame of two components — `End B` does not close half of it"
    );
    oneshot.execute("End A.B.").expect("and `End A.B` does");
}

// =============================================================================================
// `Export` — the last of the four things T02-011's own docstring says do not exist
//
// "there is no namespace stack, no `open`, no `export`, no `_root_`." All four exist now, and each
// one is pinned with the neighbouring thing it must still REFUSE.
// =============================================================================================

/// `Export Alpha (token).` makes `token` denote `Alpha.token` in the CURRENT namespace — not merely
/// while an `open` is in scope, which is the whole difference between the two.
#[test]
fn export_aliases_a_name_into_the_current_namespace() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");
    repl.execute("Namespace Host.").expect("enter Host");
    repl.execute("Export Alpha (token).")
        .expect("export it into Host");
    repl.execute("Definition uses : Nat := token.")
        .expect("`token` denotes Alpha.token here");
    assert_eq!(
        repl.execute("Eval Host.uses.").unwrap().trim(),
        "(Succ Zero)"
    );

    // The alias is reachable by its EXPORTED name too — that is what an alias is.
    repl.execute("Definition alsoUses : Nat := Host.token.")
        .expect("the alias answers at its own absolute key");
    assert_eq!(
        repl.execute("Eval Host.alsoUses.").unwrap().trim(),
        "(Succ Zero)"
    );

    // …and it dies with its namespace, exactly as an `open` does.
    repl.execute("End Host.").expect("leave Host");
    assert!(
        repl.execute("Definition afterEnd : Nat := token.").is_err(),
        "an `Export` written inside a namespace must not outlive it"
    );
    // An alias declares NOTHING: no global was created at the alias key.
    assert!(
        repl.context().get_global("Host.token").is_none(),
        "an alias is a spelling — it must not put a second copy in the context: {:?}",
        repl.context().get_global("Host.token")
    );
}

/// THE NEGATIVES. Every way an `Export` can be wrong is refused LOUDLY, because a silent one is an
/// alias nobody can see pointing at nothing.
#[test]
fn a_bad_export_is_refused_rather_than_aliased_to_nothing() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");

    // (a) an unknown source namespace
    assert!(
        repl.execute("Export Alpah (token).").is_err(),
        "unknown namespace"
    );
    // (b) a name the source namespace does not declare — the alias would point nowhere
    assert!(
        repl.execute("Export Alpha (absent).").is_err(),
        "`Alpha` declares no `absent`, so the alias would point nowhere"
    );
    // (c) no list at all — a namespace's members cannot be enumerated, so a bare Export could only
    //     be a silent no-op, and Lean requires the list for the same reason.
    assert!(
        repl.execute("Export Alpha.").is_err(),
        "the name list is required"
    );
    // (d) a collision with a name the current namespace ALREADY declares: two readings of one key.
    repl.execute("Namespace Host.").expect("enter Host");
    repl.execute("Definition token : Nat := Zero.")
        .expect("Host.token declared for real");
    let collide = repl.execute("Export Alpha (token).");
    let message = match &collide {
        Err(e) => e.to_string(),
        Ok(_) => panic!("`Host.token` already exists — aliasing over it leaves two readings"),
    };
    assert!(
        message.contains("Host.token") && message.contains("Alpha.token"),
        "the collision must name both keys: {message}"
    );
    repl.execute("End Host.").expect("leave");

    // AND THE PAIRED POSITIVE, so the checks are not a blanket refusal.
    repl.execute("Namespace Clean.").expect("enter");
    repl.execute("Export Alpha (token).")
        .expect("a clean export must still land");
    repl.execute("Definition uses : Nat := token.")
        .expect("and be usable");
    assert_eq!(
        repl.execute("Eval Clean.uses.").unwrap().trim(),
        "(Succ Zero)"
    );
}

/// Two exports of one short name from two different namespaces leave one key with two readings.
/// Reported, not resolved by recency — the same rule the ambiguous-`open` report follows, because it
/// is the same mistake wearing a different keyword.
#[test]
fn two_exports_of_one_name_are_reported_not_silently_the_later_one() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");
    repl.execute("Definition Beta.token : Nat := (Succ (Succ Zero)).")
        .expect("Beta.token");
    repl.execute("Namespace Host.").expect("enter");
    repl.execute("Export Alpha (token).")
        .expect("the first export lands");
    let second = repl.execute("Export Beta (token).");
    let message = match &second {
        Err(e) => e.to_string(),
        Ok(_) => panic!(
            "the second export gives `Host.token` a second reading — letting recency decide is the \
             silent substitution the ambiguity report exists to prevent"
        ),
    };
    assert!(
        message.contains("Alpha.token") && message.contains("Beta.token"),
        "both readings must be named: {message}"
    );
    // The FIRST export still stands: a refused second one must change nothing.
    repl.execute("Definition uses : Nat := token.")
        .expect("Alpha's export is intact");
    assert_eq!(
        repl.execute("Eval Host.uses.").unwrap().trim(),
        "(Succ Zero)"
    );
    repl.execute("End Host.").expect("leave");

    // Re-exporting the SAME target twice is idempotent, not a clash — there is only one reading.
    repl.execute("Namespace Idem.").expect("enter");
    repl.execute("Export Alpha (token).").expect("once");
    repl.execute("Export Alpha (token).")
        .expect("twice — same target, still one reading");
    repl.execute("Definition uses : Nat := token.")
        .expect("still usable");
    assert_eq!(
        repl.execute("Eval Idem.uses.").unwrap().trim(),
        "(Succ Zero)"
    );
}

/// `Namespaces::is_trivial` is the claim the whole subsystem rests on: in the empty scope, every
/// name resolves to itself and every unresolvable name keeps its spelling, so a session that never
/// writes `Namespace`, `Open` or `Export` behaves byte-identically to one with no resolver at all.
/// The claim needs ALL THREE absent, and a stale `is_trivial` that forgot one would make it a lie.
#[test]
fn the_empty_scope_is_trivial_and_each_of_the_three_ends_it() {
    use logicaffeine_kernel::interface::surface::{Name, Namespaces, Resolution};
    let n = |s: &str| Name::parse(s).expect("a name");
    let known = |k: &str| ["x", "Alpha.x"].contains(&k);

    let empty = Namespaces::new();
    assert!(empty.is_trivial(), "a fresh scope is trivial");
    assert_eq!(
        empty.resolve(&n("x"), &known),
        Resolution::Found("x".into())
    );
    assert_eq!(
        empty.resolve(&n("q"), &known),
        Resolution::Unresolved("q".into())
    );

    let mut with_ns = Namespaces::new();
    with_ns.enter(&n("Alpha"), 1);
    assert!(!with_ns.is_trivial(), "a current namespace ends triviality");
    assert_eq!(
        with_ns.resolve(&n("x"), &known),
        Resolution::Found("Alpha.x".into()),
        "…and it is no longer the identity"
    );

    let mut with_open = Namespaces::new();
    with_open.record_declaration("Alpha.x");
    with_open.open(n("Alpha")).expect("Alpha exists");
    assert!(!with_open.is_trivial(), "an open ends triviality");

    let mut with_alias = Namespaces::new();
    with_alias.record_declaration("Alpha.x");
    with_alias.record_declaration("Alpha.y");
    // Exporting `Alpha.x` as the root `x` collides with the root's own `x` — two readings of one
    // key — and a REFUSED export must leave the scope exactly as it was.
    with_alias
        .export(&n("Alpha"), &[n("x")], &known)
        .expect_err("`x` already denotes the root's constant");
    assert!(
        with_alias.is_trivial(),
        "a REFUSED export must leave the scope trivial"
    );
    with_alias
        .export(&n("Alpha"), &[n("y")], &|k| k == "Alpha.y")
        .expect("`y` is free at the root");
    assert!(!with_alias.is_trivial(), "an alias ends triviality");
    assert_eq!(
        with_alias.resolve(&n("y"), &|k| k == "Alpha.y"),
        Resolution::Found("Alpha.y".into()),
        "…and `y` now denotes what was exported"
    );
}

// =============================================================================================
// WAVE 2 · ONE NAME GRAMMAR, AND THE KEY IT LANDS AT
//
// Wave 1 gave the declaration door and the term door one grammar (`Name::scan`). That closes the
// half `vernacular_extra::a_declarable_name_is_always_a_referenceable_one` was written for — a
// name accepted as a raw text slice and keyed where no `Check` could name it. It opens a second
// half that grammar alone cannot see: the kernel `Context` is keyed by a FLAT `String`, so two
// DIFFERENT names can land on one key. `«a.b»` is one component and `a.b` is two; both key at
// `a.b`, and the later declaration silently answers to the earlier one's spelling.
//
// That is the same defect class this lane exists for — a short name quietly denoting a constant
// nobody wrote — wearing the guillemet escape instead of an `open`. The rule below is the general
// one and not a ban on quotes: a declaration name is refused exactly when its flat key READS BACK
// as a different name.
// =============================================================================================

/// Every name the declaration door takes must be spellable by a term, AND must land at a key that
/// reads back as the name that was written. The table is the grammar's whole span: plain,
/// primed, non-ASCII, quoted, and hierarchical at three depths.
#[test]
fn a_declarable_name_lands_at_a_key_that_reads_back_as_itself() {
    for (spelling, key) in [
        ("h", "h"),
        ("h'", "h'"),
        ("prime'd_name", "prime'd_name"),
        ("\u{3b1}", "\u{3b1}"),
        ("\u{2115}plus", "\u{2115}plus"),
        ("x\u{2081}", "x\u{2081}"),
        ("\u{ab}a name with spaces\u{bb}", "a name with spaces"),
        ("\u{ab}weird?!name\u{bb}", "weird?!name"),
        ("\u{ab}a b\u{bb}.value", "a b.value"),
        ("Inner.value", "Inner.value"),
        ("Outer.Inner.value", "Outer.Inner.value"),
    ] {
        let mut repl = Repl::new();
        repl.execute(&format!("Definition {spelling} : Nat := Zero."))
            .unwrap_or_else(|e| panic!("`{spelling}` must declare: {e:?}"));
        assert!(
            repl.context().get_global(key).is_some(),
            "`{spelling}` must land at the key `{key}` — a declaration keyed anywhere else is \
             unreachable by every door there is"
        );
        repl.execute(&format!("Check {spelling}."))
            .unwrap_or_else(|e| panic!("`{spelling}` declares, so a term must spell it: {e:?}"));
        assert_eq!(
            repl.execute(&format!("Eval {spelling}."))
                .unwrap_or_default()
                .trim(),
            "Zero",
            "…and spelling it must denote what was declared, not merely parse"
        );
    }
}

/// ★ THE OTHER NEGATIVE, and a defect the corpus never asked about. A name the TERM door cannot
/// spell must be refused at the declaration, whatever the component grammar says about it.
///
/// `Name::scan` gives both doors one COMPONENT grammar, and that is not enough: the term door
/// reserves its own keywords. `Definition end : Nat := Zero.` scanned clean and keyed the context
/// at a name no `Check` could ever write — a write-only global, which is the exact defect
/// `vernacular_extra::a_declarable_name_is_always_a_referenceable_one` exists to forbid, arriving
/// by a route that test's list could not see. `Outer.«a b»` was the same defect through the
/// dot-postfix; the term door now reads a component after a `.` with the declaration door's own
/// grammar, so that name is spellable and declares, and the boundary is a quoted component whose
/// flat key would read back as a different name.
#[test]
fn a_name_the_term_door_cannot_spell_is_refused_at_the_declaration() {
    // A word the term door reads as SYNTAX is not a name, and declaring one keyed the context at
    // something no `Check` could ever write.
    for word in [
        "fun", "forall", "fix", "match", "let", "Type", "Prop", "Type0",
    ] {
        let mut repl = Repl::new();
        let declared = repl.execute(&format!("Definition {word} : Nat := Zero."));
        assert!(
            declared.is_err(),
            "`{word}` is syntax to the term door, so declaring it keys a name nothing can ever \
             spell: {declared:?}"
        );
        assert!(
            repl.context().get_global(word).is_none(),
            "…and a refused name must leave nothing behind: `{word}` appeared"
        );
    }

    // …and the INVARIANT itself over the whole reserved list, which is the claim that matters and
    // does not depend on guessing which words the term door happens to read as atoms: every one of
    // them either is refused, or is a name a term can spell. There is no third answer.
    for word in [
        "fun", "forall", "fix", "match", "let", "in", "with", "return", "end", "by", "Type",
        "Prop", "Type0", "Sort", "then", "else", "do", "deriving",
    ] {
        let mut repl = Repl::new();
        if repl
            .execute(&format!("Definition {word} : Nat := Zero."))
            .is_err()
        {
            assert!(
                repl.context().get_global(word).is_none(),
                "a refused `{word}` must leave nothing behind"
            );
            continue;
        }
        repl.execute(&format!("Check {word}.")).unwrap_or_else(|e| {
            panic!("`{word}` declared, so a term must be able to spell it: {e:?}")
        });
    }

    // A quoted component AFTER a `.` is the same invariant from the other direction. The term
    // door's dot-postfix reads a component with the declaration door's own name grammar, so
    // `Outer.«a b»` is spellable, declares, and is reachable at exactly the key it took…
    let mut quoted = Repl::new();
    quoted
        .execute("Definition Outer.\u{ab}a b\u{bb} : Nat := Zero.")
        .expect("a quoted component after a `.` is a name a term can spell, so it declares");
    quoted
        .execute("Check Outer.\u{ab}a b\u{bb}.")
        .expect("…and the term door spells it");
    assert!(
        quoted.context().get_global("Outer.a b").is_some(),
        "the key is the hierarchical one, `Outer` \u{b7} `a b`"
    );
    // …while a quoted component with a `.` in it is refused, because its flat key would read back
    // as three components, and a refused name leaves nothing behind.
    let mut dotted = Repl::new();
    let collides = dotted.execute("Definition Outer.\u{ab}a.b\u{bb} : Nat := Zero.");
    assert!(
        collides.is_err(),
        "`Outer.\u{ab}a.b\u{bb}` keys at `Outer.a.b`, which reads back as a different name: \
         {collides:?}"
    );
    assert!(
        dotted.context().get_global("Outer.a.b").is_none(),
        "and nothing may be registered under the key it would have taken"
    );

    // THE BOUNDARY, so the rule is not a ban on quoting or on depth: the same quoted component in
    // HEAD position is spellable and stays legal, and so does an ordinary hierarchical name.
    let mut fine = Repl::new();
    fine.execute("Definition \u{ab}a b\u{bb}.value : Nat := Zero.")
        .expect("a quoted HEAD is spellable, so it is still a name");
    fine.execute("Check \u{ab}a b\u{bb}.value.")
        .expect("…and a term can spell it");
    fine.execute("Definition Outer.Inner.value : Nat := Zero.")
        .expect("depth is not the problem");
    fine.execute("Check Outer.Inner.value.")
        .expect("…and it is spellable at every depth");

    // A word that merely CONTAINS a reserved one is a name like any other.
    let mut near = Repl::new();
    for word in ["ending", "fund", "matcher", "letter", "index", "Typeface"] {
        near.execute(&format!("Definition {word} : Nat := Zero."))
            .unwrap_or_else(|e| panic!("`{word}` is not a keyword and must declare: {e:?}"));
        near.execute(&format!("Check {word}."))
            .unwrap_or_else(|e| panic!("`{word}` must be spellable: {e:?}"));
    }
}

/// ★ THE NEGATIVE. A name whose FLAT KEY reads back as a different name is refused at the
/// declaration, because the context cannot hold the difference. `«a.b»` is Lean's one-component
/// name with a dot in it; `a.b` is two components. They key identically here, so accepting the
/// first would make `a.b` — a spelling nobody wrote for it — denote it, and a later real
/// `Namespace a. Definition b` would answer to the earlier one's spelling with nothing reported.
///
/// The refusal is precise, not a ban on quoting: a quoted component with no dot in it keys at a
/// string no term-level dotted spelling can produce, so it stays legal at every depth.
#[test]
fn a_name_whose_flat_key_reads_back_as_another_name_is_refused() {
    for spelling in [
        "\u{ab}a.b\u{bb}",
        "Outer.\u{ab}a.b\u{bb}",
        "\u{ab}a.b\u{bb}.value",
        "\u{ab}Outer.Inner\u{bb}.value",
    ] {
        let mut repl = Repl::new();
        let declared = repl.execute(&format!("Definition {spelling} : Nat := Zero."));
        assert!(
            declared.is_err(),
            "`{spelling}` keys at a string that reads back as a DIFFERENT name, so the \
             declaration door must refuse it rather than alias one name onto another: {declared:?}"
        );
        // …and it must leave NOTHING behind under either reading.
        for phantom in [
            "a.b",
            "a",
            "b",
            "Outer.a.b",
            "value",
            "a.b.value",
            "Outer.Inner.value",
        ] {
            assert!(
                repl.context().get_global(phantom).is_none(),
                "a refused name must register nothing, and `{phantom}` appeared for `{spelling}`"
            );
        }
    }

    // The boundary, one rung either side: a quoted component with no dot is fine, and the same
    // characters WITHOUT quotes are a hierarchical name and also fine.
    let mut fine = Repl::new();
    fine.execute("Definition \u{ab}a b\u{bb} : Nat := Zero.")
        .expect("a space is not a dot");
    fine.execute("Definition a.b : Nat := Zero.")
        .expect("two components, unquoted");
    assert!(
        fine.context().get_global("a b").is_some() && fine.context().get_global("a.b").is_some()
    );
}

/// The same rule at the `Name` level, where it is decidable without a REPL: a name is holdable
/// exactly when re-reading its flat key gives the name back. Quoted components that cannot be
/// re-read AT ALL (`a name with spaces`) are holdable — no term-level spelling produces that
/// string either, so there is nothing for them to collide with.
#[test]
fn a_names_key_reads_back_as_itself_or_not_at_all() {
    use logicaffeine_kernel::interface::surface::Name;
    for (spelling, holdable) in [
        ("h", true),
        ("Outer.Inner.value", true),
        ("\u{ab}a name with spaces\u{bb}", true),
        ("\u{ab}weird?!name\u{bb}", true),
        ("Outer.\u{ab}a b\u{bb}", true),
        ("\u{ab}a.b\u{bb}", false),
        ("Outer.\u{ab}a.b\u{bb}", false),
        ("\u{ab}a.b\u{bb}.value", false),
    ] {
        let name = Name::parse(spelling).unwrap_or_else(|| panic!("`{spelling}` parses"));
        assert_eq!(
            name.key_reads_back(),
            holdable,
            "`{spelling}` keys at {:?}, which reads back as {:?}",
            name.key(),
            Name::parse(&name.key()).map(|n| n.parts().to_vec())
        );
    }
}

// =============================================================================================
// WAVE 2 · A DOCUMENT IS BALANCED
//
// `Namespace N.` with no `End N.` is the silent failure this lane exists to close, in its purest
// form: every later declaration is written into `N`, nothing is reported, and the file still
// checks. Lean refuses a file that ends inside a scope. So does this door — and the rule is
// stated as BALANCE, not as "the scope is empty at the end", so a session that entered a
// namespace before handing the door a document is not punished for it.
// =============================================================================================

/// A document must leave the scope stack exactly as it found it.
#[test]
fn a_document_must_close_every_scope_it_opens() {
    let mut repl = Repl::new();
    let unclosed = repl.execute_document("Namespace N.\nDefinition x : Nat := Zero.\n");
    assert!(
        unclosed.is_err(),
        "a document that ends inside `Namespace N` must be refused: every later declaration \
         would be written into N and the file would still check: {unclosed:?}"
    );
    // …and the door must not leave the caller inside it either.
    repl.execute("Definition afterwards : Nat := Zero.")
        .expect("declare at the root");
    assert!(
        repl.context().get_global("afterwards").is_some()
            && repl.context().get_global("N.afterwards").is_none(),
        "a refused document must not leave its namespace open"
    );

    // A SECTION is the other stack and gets the same rule.
    let mut sect = Repl::new();
    assert!(
        sect.execute_document("Section S.\nDefinition y : Nat := Zero.\n")
            .is_err(),
        "a document that ends inside `Section S` must be refused too"
    );

    // An `End` with nothing open is the mirror error and was already refused; a document carrying
    // one must fail rather than unwind past its own entry scope.
    let mut under = Repl::new();
    under
        .execute("Namespace Host.")
        .expect("the CALLER is inside Host");
    assert!(
        under
            .execute_document("End Host.\nDefinition z : Nat := Zero.\n")
            .is_err(),
        "a document may not close a scope it did not open"
    );
    under
        .execute("Definition inside : Nat := Zero.")
        .expect("still inside Host");
    assert!(
        under.context().get_global("Host.inside").is_some(),
        "…and the caller is still where it was"
    );
    under
        .execute("End Host.")
        .expect("the caller closes its own scope");
}

/// The positive half: a BALANCED document runs, at the root and nested inside a caller's scope.
/// A rule that refused every document with a namespace in it would not be a rule.
#[test]
fn a_balanced_document_runs_at_the_root_and_inside_a_callers_scope() {
    let mut repl = Repl::new();
    repl.execute_document("Namespace D.\nDefinition x : Nat := Zero.\nEnd D.\n")
        .expect("a balanced document must run");
    assert!(repl.context().get_global("D.x").is_some());

    let mut nested = Repl::new();
    nested.execute("Namespace Host.").expect("enter Host");
    nested
        .execute_document("Namespace D.\nDefinition x : Nat := Zero.\nEnd D.\n")
        .expect("balance is relative to where the document STARTED, not to the root");
    assert!(
        nested.context().get_global("Host.D.x").is_some(),
        "…and the document's namespace nests inside the caller's"
    );
    nested
        .execute("End Host.")
        .expect("the caller's scope survived the document");
}

// =============================================================================================
// WAVE 2 · T02-019's CAPABILITY, UNDER THE TEXT THE CASE MEANT
//
// `lean_conformance_t02_declarations::the_same_short_name_under_two_opens_cannot_denote_two_constants`
// executes NO `open` at all (its comments describe `open Alpha in def …`; the strings passed to
// `Repl::execute` carry no such clause), so as written it asks two byte-identical inputs in one
// session to denote two different constants — which no resolution rule can satisfy. This is the
// same three blocks WITH the clause its comments describe, so the capability the case is about is
// pinned whatever happens to the case.
// =============================================================================================

/// One short name, two namespaces, two `open`s, two constants — each `open` deciding its own
/// command. The third block is the one the case calls dangerous, and our answer is Lean's: with
/// `Alpha.Zero` open beside the prelude's `Zero`, the short name is AMBIGUOUS and is reported,
/// rather than silently taking either reading.
#[test]
fn the_same_short_name_under_two_opens_denotes_two_constants() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");
    repl.execute("Definition Beta.token : Nat := (Succ (Succ Zero)).")
        .expect("Beta.token");

    repl.execute("Open Alpha in Definition fromAlpha : Nat := token.")
        .expect("`Open Alpha in …` must make the short name `token` denote Alpha.token");
    assert_eq!(
        repl.execute("Eval fromAlpha.").unwrap().trim(),
        "(Succ Zero)"
    );

    repl.execute("Open Beta in Definition fromBeta : Nat := token.")
        .expect("…and the identical text under `Open Beta` must denote Beta.token");
    assert_eq!(
        repl.execute("Eval fromBeta.").unwrap().trim(),
        "(Succ (Succ Zero))"
    );

    // Without either opening the short name denotes NOTHING — the reading that makes the two
    // above genuinely scope-dependent rather than a flat lookup that happened to work.
    assert!(
        repl.execute("Definition bare : Nat := token.").is_err(),
        "`token` is not a root name; if it resolved here the two readings above proved nothing"
    );

    // The dangerous block. `Alpha.Zero` beside the prelude's `Zero` is two readings of one short
    // name, and Lean reports exactly this ("ambiguous, possible interpretations").
    let mut shadow = Repl::new();
    shadow
        .execute("Definition Alpha.Zero : Nat := (Succ _root_.Zero).")
        .expect("Alpha.Zero");
    let ambiguous = shadow.execute("Open Alpha in Definition shadowed : Nat := Zero.");
    let report = format!("{ambiguous:?}");
    assert!(
        ambiguous.is_err() && report.contains("Alpha.Zero") && report.contains("Zero"),
        "under `Open Alpha`, `Zero` answers to Alpha.Zero AND the prelude's Zero; picking either \
         in silence changes what the proof is about, so BOTH must be named: {report}"
    );
    // …and each reading is reachable by saying which one is meant.
    shadow
        .execute("Open Alpha in Definition explicit : Nat := Alpha.Zero.")
        .expect("the qualified spelling is never ambiguous");
    shadow
        .execute("Open Alpha in Definition rooted : Nat := _root_.Zero.")
        .expect("and `_root_` is the escape");
    assert_eq!(
        shadow.execute("Eval explicit.").unwrap().trim(),
        "(Succ Zero)"
    );
    assert_eq!(shadow.execute("Eval rooted.").unwrap().trim(), "Zero");
}

/// `Open` names a NAMESPACE, and a declared constant is not one. Without this an `Open Zero.`
/// would be a no-op that reads exactly like an opening that worked.
#[test]
fn open_refuses_a_name_that_is_a_constant_rather_than_a_namespace() {
    let mut repl = Repl::new();
    repl.execute("Definition Zed : Nat := Zero.")
        .expect("Zed is a constant");
    assert!(
        repl.execute("Open Zed.").is_err(),
        "`Zed` is a constant, not a namespace — opening it brings in nothing"
    );
    assert!(
        repl.execute("Open Nat.").is_err(),
        "a prelude INDUCTIVE is not a namespace either"
    );
    // …and the namespace that a constant's own prefix names IS openable, which is the boundary.
    repl.execute("Definition Zed2.inner : Nat := Zero.")
        .expect("Zed2.inner");
    repl.execute("Open Zed2.")
        .expect("`Zed2` is a namespace because something landed in it");
}

/// The ambiguity report is not confined to a definition BODY: a type, a `Check` and an `Eval` all
/// run the resolver, and each must report rather than pick.
#[test]
fn an_ambiguous_short_name_is_reported_at_every_door() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.Carrier : Type := Nat.")
        .expect("Alpha.Carrier");
    repl.execute("Definition Beta.Carrier : Type := Bool.")
        .expect("Beta.Carrier");
    repl.execute("Open Alpha.").expect("open Alpha");
    repl.execute("Open Beta.").expect("open Beta");

    for door in [
        "Definition inType : Carrier := Zero.",
        "Check Carrier.",
        "Eval Carrier.",
        "Inductive Box := Wrap : Carrier -> Box.",
    ] {
        let out = repl.execute(door);
        let report = format!("{out:?}");
        assert!(
            out.is_err() && report.contains("Alpha.Carrier") && report.contains("Beta.Carrier"),
            "`{door}` must report BOTH readings of the ambiguous `Carrier`: {report}"
        );
    }
    assert!(
        repl.context().get_global("Box").is_none() && repl.context().get_global("inType").is_none(),
        "and a command refused for ambiguity registers nothing"
    );
}

// =============================================================================================
// WAVE 2 · A NARROWED `open` — the tool for AVOIDING an ambiguity rather than reporting one
//
// Wave 1 made an ambiguous short name an ERROR, which is Lean's answer and the right one. On its
// own that makes `Open` unusable against a prelude that owns forty short names: every opening of a
// namespace with a common member name is a wall. Lean's answer to that is not to weaken the report
// but to let the opening say what it brings in — `open Foo (bar)` and `open Foo hiding bar`. Both
// are here, and both are checked against the ENVIRONMENT, because a clause naming a member the
// namespace does not declare narrows the opening to nothing and reads exactly like one that worked.
// =============================================================================================

/// `Open Alpha (token).` brings in exactly the names it lists, and the ambiguity the wholesale
/// opening would have caused is AVOIDED rather than reported.
#[test]
fn a_selective_open_brings_in_only_the_names_it_lists() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");
    repl.execute("Definition Alpha.other : Nat := (Succ (Succ Zero)).")
        .expect("Alpha.other");
    repl.execute("Definition Alpha.Zero : Nat := _root_.Zero.")
        .expect("Alpha.Zero, shadowing the prelude");

    repl.execute("Open Alpha (token).")
        .expect("open just `token`");
    repl.execute("Definition usesToken : Nat := token.")
        .expect("`token` is in scope");
    assert_eq!(
        repl.execute("Eval usesToken.").unwrap().trim(),
        "(Succ Zero)"
    );

    assert!(
        repl.execute("Definition usesOther : Nat := other.")
            .is_err(),
        "`other` was NOT listed, so the opening must not bring it in"
    );
    repl.execute("Definition stillPrelude : Nat := Zero.")
        .expect("`Zero` was not listed either, so it is unambiguously the prelude's");
    assert_eq!(
        repl.execute("Eval stillPrelude.").unwrap().trim(),
        "Zero",
        "a narrowed opening is the way to AVOID the ambiguity a wholesale one reports"
    );

    // A list of several, and the boundary: the clause matches the WHOLE written name, so it says
    // nothing about a longer one that happens to end in the same component.
    let mut many = Repl::new();
    many.execute("Definition Beta.a : Nat := Zero.")
        .expect("Beta.a");
    many.execute("Definition Beta.b : Nat -> Nat := fun n : Nat => (Succ n).")
        .expect("Beta.b");
    many.execute("Definition Beta.Inner.a : Nat := (Succ (Succ Zero)).")
        .expect("Beta.Inner.a");
    many.execute("Open Beta (a b).").expect("two names");
    many.execute("Definition both : Nat := (b a).")
        .expect("both listed names are in scope");
    assert_eq!(many.execute("Eval both.").unwrap().trim(), "(Succ Zero)");
    assert!(
        many.execute("Definition deep : Nat := Inner.a.").is_err(),
        "`Open Beta (a)` lists the name `a`, not every name whose last component is `a`"
    );
}

/// `Open Alpha hiding (Zero).` is the other half: everything EXCEPT the named members.
#[test]
fn a_hiding_open_brings_in_everything_but_the_names_it_hides() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");
    repl.execute("Definition Alpha.Zero : Nat := _root_.Zero.")
        .expect("Alpha.Zero");

    repl.execute("Open Alpha hiding (Zero).")
        .expect("hide the clashing member");
    repl.execute("Definition usesToken : Nat := token.")
        .expect("everything else is in scope");
    assert_eq!(
        repl.execute("Eval usesToken.").unwrap().trim(),
        "(Succ Zero)"
    );
    repl.execute("Definition stillPrelude : Nat := Zero.")
        .expect("the hidden member leaves the prelude's `Zero` unambiguous");
    assert_eq!(repl.execute("Eval stillPrelude.").unwrap().trim(), "Zero");

    // Lean's unparenthesised spelling reads the same.
    let mut bare = Repl::new();
    bare.execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");
    bare.execute("Definition Alpha.Zero : Nat := _root_.Zero.")
        .expect("Alpha.Zero");
    bare.execute("Open Alpha hiding Zero.")
        .expect("`hiding a b` without parentheses");
    bare.execute("Definition u : Nat := token.")
        .expect("in scope");
    bare.execute("Definition p : Nat := Zero.").expect("hidden");
    assert_eq!(bare.execute("Eval p.").unwrap().trim(), "Zero");
}

/// ★ THE NEGATIVE. A clause that names a member the namespace does not declare narrows the opening
/// to NOTHING and is indistinguishable from one that worked, so it is refused — the same rule the
/// unknown-namespace and bad-`Export` checks follow, because it is the same mistake.
#[test]
fn an_open_clause_naming_an_absent_member_is_refused() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");

    for bad in [
        "Open Alpha (tokne).",
        "Open Alpha hiding (tokne).",
        "Open Alpha (token tokne).",
        "Open Alpha (tokne) in Definition x : Nat := Zero.",
    ] {
        let out = repl.execute(bad);
        assert!(
            out.is_err() && format!("{out:?}").contains("Alpha.tokne"),
            "`{bad}` names a member Alpha does not declare, and the report must say which: {out:?}"
        );
    }

    // The refusal must not leave half a scope behind, and must not over-reach: the same clause
    // with the real name works.
    assert!(
        repl.execute("Definition afterFailure : Nat := token.")
            .is_err(),
        "a refused `Open` opens nothing at all"
    );
    repl.execute("Open Alpha (token).")
        .expect("the spelled-correctly clause works");
    repl.execute("Definition afterSuccess : Nat := token.")
        .expect("and now it is in scope");

    // An unknown NAMESPACE is still refused ahead of the member check, so the message names the
    // thing that is actually missing.
    let unknown = Repl::new().execute("Open Alpah (token).");
    assert!(
        format!("{unknown:?}").contains("Alpah"),
        "the report must name the missing namespace, not a member of it: {unknown:?}"
    );
}

/// The narrowed forms compose with everything the wholesale form does: the scoped `in`, several
/// clauses in one command, and the namespace stack.
#[test]
fn narrowed_openings_compose_with_scoping_and_with_each_other() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.token : Nat := Zero.")
        .expect("Alpha.token");
    repl.execute("Definition Beta.step : Nat -> Nat := fun n : Nat => (Succ n).")
        .expect("Beta.step");

    // Two clauses in one command, one of them narrowed.
    repl.execute("Open Alpha (token) Beta in Definition mixed : Nat := (step token).")
        .expect("a narrowed clause and a wholesale one in one `Open … in`");
    assert_eq!(repl.execute("Eval mixed.").unwrap().trim(), "(Succ Zero)");
    assert!(
        repl.execute("Definition leaked : Nat := token.").is_err(),
        "the scoped form still scopes to exactly one command"
    );

    // …and inside a namespace, a narrowed opening dies with it exactly as a wholesale one does.
    repl.execute("Namespace Host.").expect("enter Host");
    repl.execute("Open Alpha (token).")
        .expect("narrowed open inside a namespace");
    repl.execute("Definition uses : Nat := token.")
        .expect("visible inside");
    repl.execute("End Host.").expect("leave Host");
    assert!(
        repl.execute("Definition afterEnd : Nat := token.").is_err(),
        "a narrowed `Open` written inside a namespace must not outlive it either"
    );
}

// =============================================================================================
// WAVE 2 · AUDIT-T12-011, THE DISCRIMINATOR — AND WHAT IT FOUND
//
// Wave 1's probe settled the SOUNDNESS half: a false quantified statement is refused, so the
// literate conclusion gate is not admitting arbitrary propositions under a binder. It did not
// settle the CAPABILITY half — on a TRUE theorem, "the binder parsed" and "the binder was dropped
// and `ring` proved the open identity" agree on every observation. The experiment that separates
// them needs no false theorem: it needs a name the binder does NOT bind.
//
// ★ IT DOES NOT REPORT ONE. `Statement: forall n : Nat, (add m 0) equals m.` by `ring` CERTIFIES,
// with `m` bound by nothing. So an unbound name in a `Statement:` is read as a fresh universally
// quantified variable where Lean reports `unknown identifier 'm'`. That is a DIAGNOSTIC divergence
// and not a soundness one — a `ring` identity over free variables is true at every value of them —
// and the fix belongs to the literate door (`interface/literate_parser.rs`), which this lane does
// not own. It is recorded in the report and the soundness half is pinned here, stated so that it
// holds under EITHER reading and therefore stays green when that door grows the report.
// =============================================================================================

/// A name the binder does not bind can never make a FALSE statement certify.
///
/// Every body below is false under both readings of an unbound `m` — as a fresh universally
/// quantified variable (`∀ n m, …`), and as an unknown identifier — so every one must be refused.
/// That is the property that is about soundness rather than about diagnostics, and it does not
/// depend on which reading the literate door settles on.
#[test]
fn a_name_the_binder_does_not_bind_can_never_make_a_false_statement_certify() {
    // The control. If this did not certify, the refusals below would prove nothing.
    surface_c(
        "## Theorem: Bound\n    Statement: forall n : Nat, (add n 0) equals n.\n    Proof: ring.",
    )
    .expect("the control must certify");

    for statement in [
        "forall n : Nat, (add m 1) equals m",
        "forall n : Nat, (add m 0) equals n",
        "forall n : Nat, (add n 1) equals m",
    ] {
        let out = surface_c(&format!(
            "## Theorem: NotAnIdentity\n    Statement: {statement}.\n    Proof: ring."
        ));
        assert!(
            out.is_err(),
            "`{statement}` is no identity under any reading of its unbound name, so it must be \
             refused: {out:?}"
        );
    }

    // …and the binder's SORT is not decoration either: a binder with no sort is refused, so the
    // `forall` that does bind cannot be satisfied by a spelling that says nothing about its domain.
    let sortless = surface_c(
        "## Theorem: Sortless\n    Statement: forall n, (add n 0) equals n.\n    Proof: ring.",
    );
    assert!(
        sortless.is_err(),
        "a `forall` in a `Statement:` must name its domain: {sortless:?}"
    );
}

// =============================================================================================
// WAVE 3 · `Section` … `End` MUST BE A KERNEL SCOPE TOO
//
// `Context::push_scope`/`pop_scope` revert the four ATTRIBUTE databases — instances, coercions,
// simp rules, hints — and never a declaration. The vernacular's `Section`/`End` moved the SURFACE
// scope (notation) and left the kernel's at zero, so a `section` in a document scoped how things
// were SPELLED and nothing about what was in force inside it. The two depths are mirrored rather
// than restated: the REPL reads the surface's own count before and after, so a scope form added
// later cannot forget to be one.
// =============================================================================================

/// A `Section`/`End` pair opens and closes a kernel scope, and so does `Namespace`/`End` — Lean
/// scopes `local` at both. The BOUNDARY is asserted on each side: depth 0 before, 1 inside, 0
/// after, and nesting reaches 2.
#[test]
fn a_section_and_a_namespace_are_each_a_kernel_scope() {
    let mut repl = Repl::new();
    assert_eq!(
        repl.context().scope_depth(),
        0,
        "a fresh REPL is at the permanent scope"
    );

    repl.execute("Section Alpha.").expect("a section opens");
    assert_eq!(
        repl.context().scope_depth(),
        1,
        "`Section` must open a kernel scope"
    );
    repl.execute("Namespace Inner.").expect("a namespace opens");
    assert_eq!(
        repl.context().scope_depth(),
        2,
        "`Namespace` is a scope for `local` too"
    );
    repl.execute("End Inner.").expect("the namespace closes");
    assert_eq!(
        repl.context().scope_depth(),
        1,
        "one `End` peels exactly one scope"
    );
    repl.execute("End Alpha.").expect("the section closes");
    assert_eq!(
        repl.context().scope_depth(),
        0,
        "and the pair balances back to the root"
    );
}

/// A `local` registration made inside a section LEAVES at the `End`, and a DECLARATION made inside
/// it does not. That pairing is the whole point of a scope: reverting the attribute databases is
/// not the same as restoring a snapshot, which would delete the section's real content with them.
#[test]
fn closing_a_section_reverts_a_local_attribute_and_keeps_a_declaration() {
    use logicaffeine_kernel::{Reducibility, Term};

    let mut repl = Repl::new();
    repl.execute("Definition sealedZero : Nat := Zero.")
        .expect("a definition to mark");
    repl.execute("Section Opaque.").expect("a section opens");
    repl.context_mut()
        .set_reducibility("sealedZero", Reducibility::Irreducible);
    repl.execute("Definition insideSection : Nat := (Succ Zero).")
        .expect("a declaration inside");
    assert!(
        repl.context().is_irreducible("sealedZero"),
        "the attribute is in force inside its own section"
    );

    repl.execute("End Opaque.").expect("the section closes");
    assert!(
        !repl.context().is_irreducible("sealedZero"),
        "a `local` attribute must LEAVE the database when its scope closes — that is what the \
         kernel's scope axis is for, and the vernacular has to actually open one"
    );
    assert!(
        repl.context().is_definition("insideSection"),
        "…and the declaration written inside it survives: a scope reverts ATTRIBUTES, never a \
         declaration"
    );
    assert_eq!(
        repl.execute("Eval insideSection.")
            .expect("it still evaluates")
            .trim(),
        "(Succ Zero)",
        "and it still means what it meant"
    );
    let _ = Term::Hole; // the import is the point: this test is about the kernel's own types
}

/// A document that opens a scope and never closes it is REFUSED, and the refusal must not leave a
/// kernel scope behind either — the next document would then register its `local`s at a depth
/// nobody wrote and nothing would revert them.
#[test]
fn a_refused_document_leaves_no_kernel_scope_behind() {
    let mut repl = Repl::new();
    let unbalanced = repl.execute_document("Section Dangling.\nDefinition d : Nat := Zero.\n");
    assert!(
        unbalanced.is_err(),
        "a document that ends inside a scope is refused: {unbalanced:?}"
    );
    assert_eq!(
        repl.context().scope_depth(),
        0,
        "a refused document must wind the KERNEL scope back too, not only the surface's"
    );

    // …and the balanced form leaves it at zero the ordinary way.
    repl.execute_document("Section Fine.\nDefinition e : Nat := Zero.\nEnd Fine.\n")
        .expect("a balanced document runs");
    assert_eq!(
        repl.context().scope_depth(),
        0,
        "a balanced document is balanced in both stacks"
    );
}

// =============================================================================================
// WAVE 3 · A DEFAULT ARGUMENT IS A SPELLING
//
// `Definition f : Int -> (m : Int := 3) -> Int := …` used to be refused at the SPLIT: the sentence
// was cut at the first `:=` in the text, which is the DEFAULT's, and the term parser was handed the
// truncated `Int -> (m : Int` and reported `Expected ')' found EOF`. Two things had to become true.
// The split is a TOP-LEVEL scan (the term grammar's own knowledge of brackets, strings and
// «quoted names»), and the declared type parts into the kernel type the context is keyed at and the
// per-parameter surface a CALL SITE reads — because `withDefault 1` is a SPELLING of
// `withDefault 1 3`, exactly as `token` under `open Alpha` is a spelling of `Alpha.token`.
// =============================================================================================

/// The historical failure, pinned from the outside: a `:=` inside the TYPE does not end the
/// sentence, and the declaration registers at the type WITHOUT the default in it.
#[test]
fn a_default_inside_a_type_does_not_split_the_sentence() {
    let mut repl = Repl::new();
    repl.execute(
        "Definition withDefault : Int -> (m : Int := 3) -> Int := \
         fun n : Int => fun m : Int => add n m.",
    )
    .expect("a parameter's default must be expressible in a declaration");

    let shown = repl
        .execute("Check withDefault.")
        .expect("the declaration is there");
    assert!(
        !shown.contains("default"),
        "the KERNEL type carries no marker — a default is surface, not type: {shown}"
    );
    assert!(
        shown.starts_with("withDefault : Int -> ") && shown.ends_with("Int"),
        "the type the context is keyed at is the one with the default ERASED: {shown}"
    );

    // …and structurally, because a printer is not a witness. Two binders, both `Int`, no marker.
    use logicaffeine_kernel::Term;
    let ty = repl
        .context()
        .get_global("withDefault")
        .cloned()
        .expect("it is registered");
    let Term::Pi {
        param_type: first,
        body_type: rest,
        ..
    } = &ty
    else {
        panic!("the declared type is a Pi: {ty}")
    };
    let Term::Pi {
        param_type: second,
        body_type: result,
        ..
    } = rest.as_ref()
    else {
        panic!("…of two binders: {ty}")
    };
    for part in [first.as_ref(), second.as_ref(), result.as_ref()] {
        assert_eq!(
            part,
            &Term::Global("Int".to_string()),
            "every position is the bare `Int` the user wrote, with nothing wrapped round it: {ty}"
        );
    }
}

/// Omitting the default produces the SAME TERM as writing the argument out, not a shorter spine.
/// Asserted on the registered bodies rather than on the printed answer, because two different
/// terms can normalise to one value and only one of them is what Lean elaborates.
#[test]
fn an_omitted_default_is_the_same_term_as_the_argument_written_out() {
    use logicaffeine_kernel::{double_check, DoubleCheck};

    let mut repl = Repl::new();
    repl.execute(
        "Definition withDefault : Int -> (m : Int := 3) -> Int := \
         fun n : Int => fun m : Int => add n m.",
    )
    .expect("declare");
    repl.execute("Definition omitted : Int := withDefault 1.")
        .expect("the default is filled");
    repl.execute("Definition written : Int := withDefault 1 3.")
        .expect("the argument written out");

    let a = repl
        .context()
        .get_definition_body("omitted")
        .cloned()
        .expect("omitted");
    let b = repl
        .context()
        .get_definition_body("written")
        .cloned()
        .expect("written");
    assert_eq!(
        a, b,
        "an omitted default is inlined, so the two spellings are ONE term"
    );
    assert_eq!(
        repl.execute("Eval omitted.").expect("it evaluates").trim(),
        "4",
        "withDefault 1 = withDefault 1 3 = 4"
    );

    // BOTH KERNELS. The filled term is an ordinary application and the second, de Bruijn kernel
    // must certify it without knowing a default was ever involved.
    assert!(
        matches!(double_check(repl.context(), &a), DoubleCheck::Agreed),
        "the independent kernel must certify the filled term: {:?}",
        double_check(repl.context(), &a)
    );
}

/// A BARE NAME IS NOT A CALL SITE. `Check withDefault.` reports the function, and a definition
/// whose body is the bare name registers exactly that name — filling there would change what an
/// unapplied constant MEANS.
#[test]
fn a_bare_name_is_not_a_call_site() {
    use logicaffeine_kernel::Term;

    let mut repl = Repl::new();
    repl.execute(
        "Definition withDefault : Int -> (m : Int := 3) -> Int := \
         fun n : Int => fun m : Int => add n m.",
    )
    .expect("declare");
    repl.execute("Definition asFunction : Int -> Int -> Int := withDefault.")
        .expect("the unapplied function is the function it already was");
    assert_eq!(
        repl.context().get_definition_body("asFunction"),
        Some(&Term::Global("withDefault".to_string())),
        "an unapplied name must be registered verbatim — no argument was omitted, because none \
         was being supplied"
    );
}

/// Filling STOPS at the first parameter with no default, and it fills only at the END of a
/// written spine. `mid 1` against `(a) (m := 3) (c)` is `mid 1 3`, still awaiting `c`; `mid 1 10`
/// is POSITIONAL — `m` is `10` — which is the reading Lean gives the same unparenthesised spine.
///
/// ★ THE ONE DIVERGENCE, STATED RATHER THAN DISCOVERED. Lean distinguishes `(mid 1) 10` from
/// `mid 1 10` because its `paren` node survives into elaboration; `Term::App` has no such node, so
/// the two are ONE term here and both read positionally. It matters only for a default in a
/// NON-FINAL position — which is the shape Lean itself calls a mistake, since no call site can
/// then omit it — and for a trailing default, the overwhelmingly common one, the two agree
/// exactly. Filling per `App` NODE instead would be the wrong repair: `mid 1 10 100` would become
/// `mid 1 3 10 100`, four arguments to a three-parameter function, so an explicit middle argument
/// could never be written at all.
#[test]
fn filling_stops_at_the_first_parameter_with_no_default() {
    let mut repl = Repl::new();
    repl.execute(
        "Definition mid : Int -> (m : Int := 3) -> Int -> Int := \
         fun a : Int => fun b : Int => fun c : Int => add (add a b) c.",
    )
    .expect("a default in the MIDDLE of a telescope");

    let one = repl
        .execute("Check (mid 1).")
        .expect("one argument supplied");
    assert!(
        one.contains("Int -> Int") || one.contains("Π"),
        "exactly ONE argument was filled, so one explicit parameter is still owed: {one}"
    );
    // `add` is `Int`'s checked arithmetic, defined by cases on `Int_ofNat`/`Int_negSucc`, so the
    // normal form unfolds it; the claim is compared against the same function written out.
    let filled = repl
        .execute("Eval (mid 1).")
        .expect("the filled partial application");
    assert_eq!(
        filled.trim(),
        repl.execute("Eval (fun c : Int => add 4 c).")
            .expect("the function written out")
            .trim(),
        "`mid 1` is `mid 1 3`, a function still awaiting `c` — the default was filled and the \
         parameter with none was not"
    );
    assert!(
        filled.trim().starts_with("λ(c:Int)."),
        "and it still awaits `c`: {filled}"
    );
    assert_ne!(
        filled.trim(),
        repl.execute("Eval (fun c : Int => add 1 c).")
            .expect("the unfilled reading")
            .trim(),
        "the default `3` was added in: `mid 1` is not `λc. add 1 c`"
    );
    assert_eq!(
        repl.execute("Eval (mid 1 10 100).")
            .expect("every argument written out")
            .trim(),
        "111",
        "a spine that supplies the defaulted position reads it POSITIONALLY: m = 10, not 3"
    );
}

/// A default that MENTIONS an earlier parameter is closed with that parameter's ACTUAL argument.
/// The failure mode this forbids is an OPEN TERM — a free `n` inlined at a call site that never
/// bound one — which type-checks nowhere and is reported far from here.
#[test]
fn a_dependent_default_is_closed_with_the_actual_argument() {
    let mut repl = Repl::new();
    repl.execute(
        "Definition sized : Int -> (m : Int := mul n 2) -> Int := \
         fun n : Int => fun m : Int => add n m.",
    )
    .expect("a dependent default is expressible");
    assert_eq!(
        repl.execute("Eval (sized 5).")
            .expect("the dependent default is substituted")
            .trim(),
        "15",
        "sized 5 = sized 5 10 — the default is `n * 2` with n := 5, never an open `n`"
    );
    assert_eq!(
        repl.execute("Eval (sized 7).")
            .expect("a second call site")
            .trim(),
        "21",
        "…and a second call site inlines ITS argument, not the first one's"
    );
}

/// A default is filled at EVERY application spine in a term, not only at its top.
#[test]
fn a_default_is_filled_at_every_spine_in_a_term() {
    let mut repl = Repl::new();
    repl.execute(
        "Definition withDefault : Int -> (m : Int := 3) -> Int := \
         fun n : Int => fun m : Int => add n m.",
    )
    .expect("declare");
    assert_eq!(
        repl.execute("Eval (add (withDefault 1) (withDefault 2)).")
            .expect("nested")
            .trim(),
        "9",
        "both spines are filled: (1+3) + (2+3)"
    );
    repl.execute("Definition inALambda : Int -> Int := fun k : Int => withDefault k.")
        .expect("a spine under a binder");
    assert_eq!(
        repl.execute("Eval (inALambda 10).")
            .expect("it computes")
            .trim(),
        "13",
        "a spine inside a lambda body is filled too"
    );
}

/// A REDECLARATION replaces what the surface says about a name, and one that drops the default
/// drops the filling. A stale default would supply an argument the current declaration never
/// offered — silently, and at the wrong arity.
#[test]
fn a_redeclaration_without_a_default_stops_filling() {
    let mut repl = Repl::new();
    repl.execute(
        "Definition withDefault : Int -> (m : Int := 3) -> Int := \
         fun n : Int => fun m : Int => add n m.",
    )
    .expect("declare with a default");
    assert_eq!(
        repl.execute("Eval (withDefault 1).")
            .expect("filled")
            .trim(),
        "4",
        "the default is in force"
    );

    repl.execute(
        "Definition withDefault : Int -> Int -> Int := fun n : Int => fun m : Int => add n m.",
    )
    .expect("redeclare WITHOUT a default");
    let after = repl
        .execute("Check (withDefault 1).")
        .expect("still applies");
    assert!(
        after.contains("Int -> Int"),
        "the default is GONE with the declaration that carried it — `withDefault 1` is a partial \
         application again: {after}"
    );
}

/// A default cannot smuggle an ILL-TYPED argument past the kernel. The filled term is an ordinary
/// application and is type-checked exactly as one written by hand.
#[test]
fn a_default_that_does_not_typecheck_is_refused_at_the_call_site() {
    let mut repl = Repl::new();
    repl.execute(
        "Definition badDefault : Int -> (m : Int := True) -> Int := \
         fun n : Int => fun m : Int => n.",
    )
    .expect("the declaration itself is about the kernel type, which is fine");
    let used = repl.execute("Eval (badDefault 1).");
    assert!(
        used.is_err(),
        "filling `True` into an `Int` position must be REFUSED — a spelling may not change what \
         type-checks: {used:?}"
    );
}

/// A default is SOURCE, and reads in the namespace scope it is written in.
#[test]
fn a_default_resolves_in_the_namespace_it_is_written_in() {
    let mut repl = Repl::new();
    repl.execute_document(
        "Namespace Cfg.\n\
         Definition step : Int := 4.\n\
         Definition scaled : Int -> (m : Int := step) -> Int := \
           fun n : Int => fun m : Int => add n m.\n\
         End Cfg.\n",
    )
    .expect("a namespaced declaration whose default names a sibling by its SHORT name");
    assert_eq!(
        repl.execute("Eval (Cfg.scaled 1).").expect("the default is filled from outside too").trim(),
        "5",
        "`step` in the default meant `Cfg.step`, and it still does at a call site written elsewhere"
    );
}

// =============================================================================================
// WAVE 3 · THE SENTENCE SPLIT IS THE TERM GRAMMAR'S JOB
// =============================================================================================

/// A `:=` and a `:` inside a «quoted name» are part of the NAME. Under a raw `find`, both split
/// the sentence at a position inside the name and the declaration was refused for a reason that
/// named neither.
#[test]
fn a_delimiter_inside_a_quoted_name_is_part_of_the_name() {
    let mut repl = Repl::new();
    repl.execute("Definition «a := b» : Nat := Zero.")
        .expect("a `:=` inside the NAME");
    assert!(
        repl.context().get_global("a := b").is_some(),
        "the declaration is keyed at the whole quoted name"
    );
    assert_eq!(
        repl.execute("Eval «a := b».")
            .expect("and a term can spell it")
            .trim(),
        "Zero"
    );

    repl.execute("Definition «c : d» := (Succ Zero).")
        .expect("a `:` inside the NAME");
    assert_eq!(
        repl.execute("Eval «c : d».").expect("it evaluates").trim(),
        "(Succ Zero)",
        "the colon inside the quotes did not open a type ascription"
    );
}

/// The same rule at the INDUCTIVE door: a constructor whose name is quoted and contains a colon,
/// and a `|` that belongs to a constructor's type rather than to the alternation.
#[test]
fn the_inductive_door_splits_at_top_level_too() {
    let mut repl = Repl::new();
    repl.execute("Inductive Quirk := «a:b» : Quirk | plain : Quirk.")
        .expect("a quoted constructor name containing a colon");
    assert_eq!(
        repl.execute("Eval «a:b».")
            .expect("the constructor is there")
            .trim(),
        "a:b",
        "the constructor is keyed at its whole quoted name"
    );
    assert!(
        repl.context().get_global("plain").is_some(),
        "and the alternation still split at the `|` that separates the constructors"
    );
}

/// The NEGATIVE half: a sentence with no top-level `:=` at all is still refused, and it is refused
/// for the missing `:=` rather than for whatever the scan found inside a bracket.
#[test]
fn a_definition_with_no_top_level_assignment_is_refused() {
    let mut repl = Repl::new();
    let bare = repl.execute("Definition f.");
    assert!(
        bare.is_err(),
        "a definition must assign something: {bare:?}"
    );
    let hidden = repl.execute("Definition g : (m : Int := 3) -> Int.");
    assert!(
        hidden.is_err(),
        "a `:=` that lives INSIDE the type is not the sentence's assignment, so this is still a \
         definition with no body: {hidden:?}"
    );
}

// =============================================================================================
// WAVE 3 · `Print Axioms <name>.` — OUR `#print axioms`
//
// The report that makes an admission VISIBLE. Lean warns on the declaration that CONTAINS a
// `sorry` and says nothing about its dependants, so a theorem three lemmas downstream of one looks
// finished; this reads the transitive closure of the ELABORATED environment, so the taint is a
// property of the proof and cannot be lost by renaming, re-exporting or citing at a distance.
// One implementation, three surfaces: the command reads `axiom_dependencies_of`, which is what the
// re-check gate and the trusted-base census already read.
// =============================================================================================

/// The transitive case, which no warning can reach: a downstream theorem whose proof term never
/// mentions the admission must still be reported as resting on it — and a clean theorem beside it
/// must not be.
#[test]
fn print_axioms_follows_the_dependency_graph_and_the_taint_is_exact() {
    use logicaffeine_kernel::{Term, Universe};

    // `sorryAx : Π(A : Type 0). A` — one declaration that proves every proposition, which is what
    // makes the report necessary rather than decorative.
    let sorry_ax = Term::Pi {
        param: "A".to_string(),
        param_type: Box::new(Term::Sort(Universe::Type(0))),
        body_type: Box::new(Term::Var("A".to_string())),
    };
    let mut repl = Repl::new();
    repl.context_mut().add_declaration("sorryAx", sorry_ax);
    repl.execute("Definition taintedLemma : Eq Int 2 3 := sorryAx (Eq Int 2 3).")
        .expect("an admitted lemma registers — that is the hazard");
    repl.execute("Definition taintedUser : Eq Int 2 3 := taintedLemma.")
        .expect("a downstream theorem cites the lemma by name");
    repl.execute("Definition cleanThm : Eq Int 3 3 := refl Int 3.")
        .expect("a clean theorem");

    let tainted = repl
        .execute("Print Axioms taintedUser.")
        .expect("the report exists");
    assert!(
        tainted.contains("sorryAx"),
        "the report follows the graph: `taintedUser` contains no `sorry` of its own: {tainted}"
    );
    assert!(
        tainted.contains("ADMITTED"),
        "and it says what it means — an admission is named FIRST, not buried in a list: {tainted}"
    );
    let clean = repl
        .execute("Print Axioms cleanThm.")
        .expect("it works for a clean theorem too");
    assert!(
        !clean.contains("sorryAx"),
        "the taint is EXACT, not file-wide: `cleanThm` rests on nothing: {clean}"
    );
}

/// A name nothing declares is REFUSED. A report that answered "depends on no axioms" for a
/// misspelling would be the most dangerous possible answer: it says "proved" about nothing.
#[test]
fn print_axioms_refuses_a_name_that_denotes_nothing() {
    let mut repl = Repl::new();
    let missing = repl.execute("Print Axioms noSuchTheorem.");
    assert!(
        missing.is_err(),
        "an unknown subject must be refused, never reported as depending on nothing: {missing:?}"
    );
    assert!(
        format!("{missing:?}").contains("noSuchTheorem"),
        "…and the refusal names what was asked for: {missing:?}"
    );
}

/// `Print` is a FAMILY in both Coq and Lean, so the subject is required rather than assumed. An
/// unknown subject is refused with the one spelling that exists.
#[test]
fn print_requires_the_report_it_is_asking_for() {
    let mut repl = Repl::new();
    let vague = repl.execute("Print cleanThm.");
    assert!(vague.is_err(), "a bare `Print` is not a command: {vague:?}");
    assert!(
        format!("{vague:?}").contains("Print Axioms"),
        "the refusal names the report that DOES exist: {vague:?}"
    );
}

/// The subject is a REFERENCE, so it reads in the namespace scope it is written in — the same
/// resolution a term's name gets, asked of the same door.
#[test]
fn print_axioms_reads_its_subject_in_the_current_scope() {
    let mut repl = Repl::new();
    repl.execute("Namespace Audit.").expect("open a namespace");
    repl.execute("Definition here : Nat := Zero.")
        .expect("a declaration inside it");
    let short = repl
        .execute("Print Axioms here.")
        .expect("the short name resolves inside");
    assert!(
        short.contains("Audit.here"),
        "the subject resolved to its ABSOLUTE key, and the report names that key: {short}"
    );
    repl.execute("End Audit.").expect("close it");
    assert!(
        repl.execute("Print Axioms here.").is_err(),
        "…and outside the namespace the short name denotes nothing again"
    );
    assert!(
        repl.execute("Print Axioms Audit.here.").is_ok(),
        "the hierarchical name always works"
    );
}

// =============================================================================================
// WAVE 3 · DECLARING AN INDUCTIVE AT THE DOOR BRINGS ITS WHOLE FAMILY
//
// `add_constructor_checked` synthesises the declaration-time family Lean generates for every
// inductive. The REPL used to re-derive `{I}_rec` by hand afterwards and register it with
// `add_definition`, paying for a second derivation of a constant that already existed AND
// disowning the generated one — after which `{I}_below` and `{I}_brecOn` were the only members of
// the family the user-facing door did not hand back.
// =============================================================================================

/// Every member of the family is reachable from the vernacular, on the Coq-style door and the
/// literate one alike.
#[test]
fn declaring_an_inductive_at_the_door_brings_its_whole_family() {
    for source in [
        "Inductive Tally := Nought : Tally | Tick : Tally -> Tally.",
        "A Tally is either Nought or a Tick with pred: Tally.",
    ] {
        let mut repl = Repl::new();
        repl.execute(source)
            .unwrap_or_else(|e| panic!("`{source}` must register: {e:?}"));
        for aux in [
            "Tally_rec",
            "Tally_below",
            "Tally_brecOn",
            "Tally_noConfusion",
        ] {
            let r = repl.execute(&format!("Check {aux}."));
            assert!(
                r.is_ok(),
                "`{aux}` must exist the moment the declaration does, via `{source}`: {r:?}"
            );
        }
    }
}

/// …and a MUTUAL block gets one per member. The mutual door registers through the BULK
/// constructor door, which queues synthesis rather than paying per constructor, so the block has
/// to ask — and if it forgot, only a mutual declaration would be missing its eliminator.
#[test]
fn a_mutual_block_at_the_door_brings_a_family_per_member() {
    let mut repl = Repl::new();
    repl.execute_document(
        "Inductive Grove := aLeaf : Grove | aBranch : Copse -> Grove.\n\
         Inductive Copse := noMore : Copse | more : Grove -> Copse -> Copse.\n",
    )
    .expect("a genuine mutual cycle registers as one block");
    for aux in [
        "Grove_rec",
        "Copse_rec",
        "Grove_noConfusion",
        "Copse_noConfusion",
        "Grove_noConfusionType",
        "aBranch_inj",
    ] {
        let r = repl.execute(&format!("Check {aux}."));
        assert!(
            r.is_ok(),
            "`{aux}` must be derived for its member of the block: {r:?}"
        );
    }
    // The SCOPE BOUNDARY, recorded rather than discovered later. `derive_auxiliaries` is
    // best-effort and omits what this kernel cannot state for a shape; for a mutual cycle that is
    // the course-of-values pair, because `below` is stated over ONE inductive's own constructor
    // telescope and a mutual member's recursive fields land in its sibling. `aLeaf_inj` is absent
    // for the ordinary reason: a constructor with no fields has nothing to injure.
    for absent in ["Grove_below", "Grove_brecOn", "aLeaf_inj"] {
        assert!(
            repl.context().get_global(absent).is_none(),
            "`{absent}` is NOT derivable for a mutual member today — if it has become derivable \
             that is a win and this list is what says so out loud"
        );
    }
}

// =============================================================================================
// WAVE 3 · THE EQUATION COMPILER REACHES THE VERNACULAR
//
// `Consider`'s equation set cannot be read from TEXT ALONE: the constructor list, its REGISTRATION
// ORDER and its arities live in the kernel context. The REPL read its commands through the
// text-only doors, so every clause landed in the slot its POSITION hit, a nested pattern was a
// parse error, and a `When _:` catch-all had nowhere to fan out to. All three doors now carry the
// context.
// =============================================================================================

/// A clause lands in the slot of the constructor it NAMES. `Bool` is registered `true` then
/// `false`, so writing the clauses the other way round used to compute the other function —
/// silently, for every equal-arity inductive.
#[test]
fn a_clause_lands_in_the_constructor_it_names_not_the_slot_it_sits_in() {
    let mut repl = Repl::new();
    repl.execute(
        "## To flip (b: Bool) -> Bool: Consider b: When false: Yield true. When true: Yield false.",
    )
    .expect("the clauses are well-formed whichever order they are written in");
    assert_eq!(
        repl.execute("Eval flip true.").expect("flip true").trim(),
        "false",
        "the `When true:` clause decides what `flip true` is, wherever it was written"
    );
    assert_eq!(
        repl.execute("Eval flip false.").expect("flip false").trim(),
        "true",
        "…and so does the other one"
    );
}

/// A NESTED constructor pattern compiles to a tree of positional matches, and a `_` catch-all
/// fans out over every constructor with no clause of its own.
#[test]
fn a_nested_pattern_and_a_catch_all_are_both_writable_at_the_door() {
    let mut repl = Repl::new();
    repl.execute(
        "## To classify (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ Zero: Yield (Succ Zero). When Succ (Succ k): Yield k.",
    )
    .expect("a nested pattern must compile");
    for (arg, want) in [
        ("Zero", "Zero"),
        ("(Succ Zero)", "(Succ Zero)"),
        ("(Succ (Succ Zero))", "Zero"),
    ] {
        assert_eq!(
            repl.execute(&format!("Eval classify {arg}."))
                .expect("it computes")
                .trim(),
            want,
            "classify {arg}"
        );
    }

    repl.execute("A Hue is either Ruby or Jade or Slate.")
        .expect("three constructors");
    repl.execute(
        "## To isRuby (h: Hue) -> Bool: Consider h: When Ruby: Yield true. When _: Yield false.",
    )
    .expect("two clauses over three constructors — the catch-all must fan out");
    for (arg, want) in [("Ruby", "true"), ("Jade", "false"), ("Slate", "false")] {
        assert_eq!(
            repl.execute(&format!("Eval isRuby {arg}."))
                .expect("it computes")
                .trim(),
            want,
            "the expansion agrees with Lean on EVERY constructor, not only the enumerated one"
        );
    }
}

/// THE NEGATIVE HALF, and it is the one that matters: a clause set that misses a case is REFUSED
/// and the refusal NAMES the missing case. Relaxing exhaustiveness would make the compiler invent
/// a body for the uncovered constructor, which is a silent wrong answer for every input that
/// reaches it.
#[test]
fn an_equation_set_that_misses_a_case_is_refused_and_names_it() {
    let mut repl = Repl::new();
    let gappy = repl.execute(
        "## To halfBaked (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ (Succ k): Yield k.",
    );
    let report = format!("{gappy:?}");
    assert!(
        gappy.is_err(),
        "`Succ Zero` is covered by no clause, so this is not a function"
    );
    assert!(
        report.contains("Succ Zero"),
        "…and the refusal names the case that is missing: {report}"
    );
    assert!(
        repl.context().get_global("halfBaked").is_none(),
        "a refused definition registers nothing"
    );
}

/// A DOCUMENT containing a nested pattern can be SPLIT into blocks at all. The splitter finds a
/// block's extent by parsing it, and the text-only literate door refuses a nested pattern rather
/// than measuring it — so without a context in the splitter, a file containing one could not even
/// be read, whatever the rest of it said.
#[test]
fn a_document_containing_a_nested_pattern_can_be_split_into_blocks() {
    let mut repl = Repl::new();
    let out = repl.execute_document(
        "## To classify (n: Nat) -> Nat: Consider n: When Zero: Yield Zero. \
         When Succ Zero: Yield (Succ Zero). When Succ (Succ k): Yield k.\n\
         Definition three : Nat := (Succ (Succ (Succ Zero))).\n",
    );
    assert!(out.is_ok(), "the document must split and run: {out:?}");
    assert_eq!(
        repl.execute("Eval classify three.")
            .expect("both blocks registered")
            .trim(),
        "(Succ Zero)",
        "the block AFTER the nested pattern ran too — a splitter that could not measure the \
         first block would have discarded the rest of the file"
    );
}

/// ★ THE HANDOFF LANDED, FROM THE OTHER SIDE. Wave 2's declaration door REFUSED `Outer.«a b»` —
/// truthfully, because the term door's dot-postfix read a plain identifier and no term could spell
/// it — and the note handed to the term-parser lane asked for `scan_name` there instead. It landed,
/// so the invariant "a declarable name is a referenceable one" is now satisfied the other way: the
/// name is ACCEPTED because a term can spell it.
///
/// This is added rather than folded into the wave-2 refusal test because that test's quoted block
/// now asserts the opposite of what holds; the exact edit is in this lane's `test_edits_needed`,
/// and this lock means no coverage is lost while it waits.
#[test]
fn a_quoted_component_after_a_dot_is_declarable_and_spellable() {
    let mut repl = Repl::new();
    repl.execute("Definition Outer.\u{ab}a b\u{bb} : Nat := Zero.")
        .expect("a quoted component after a `.` is a name a term can spell, so it declares");
    repl.execute("Check Outer.\u{ab}a b\u{bb}.")
        .expect("…and the term door spells it");
    assert_eq!(
        repl.execute("Eval Outer.\u{ab}a b\u{bb}.")
            .expect("it evaluates")
            .trim(),
        "Zero",
        "the declaration is reachable at exactly the key it took — which is the whole invariant"
    );
    assert!(
        repl.context().get_global("Outer.a b").is_some(),
        "and the key is the hierarchical one, `Outer` \u{b7} `a b`"
    );

    // THE BOUNDARY on the other side, unchanged: a quoted component containing a `.` is still
    // refused, because ITS flat key reads back as a different name and the context cannot hold the
    // difference.
    let mut dotted = Repl::new();
    let collides = dotted.execute("Definition Outer.\u{ab}a.b\u{bb} : Nat := Zero.");
    assert!(
        collides.is_err(),
        "a quoted component with a `.` in it keys at `Outer.a.b`, which reads back as THREE \
         components: {collides:?}"
    );
}

/// An AUTO-PARAM `(h : P := by tac)` is the default's sibling — a different proof term at every
/// call site rather than one term inlined per omission — and LOGOS does not run one yet. What must
/// already be true is that the declaration registers at the ERASED type: the marker the parser
/// rides the tactic name on is surface, and letting it reach the kernel registers a type whose
/// parameter is an application of a global nobody declared.
///
/// The unfilled obligation is a PARTIAL APPLICATION, which is a type error downstream and never a
/// silent wrong answer. When the tactic does run at the call site this lock is what says so.
#[test]
fn an_auto_param_declaration_registers_at_the_erased_type() {
    let mut repl = Repl::new();
    repl.execute(
        "Definition guarded : Int -> (h : Eq Int 1 1 := by decide) -> Int := \
         fun n : Int => fun h : Eq Int 1 1 => n.",
    )
    .expect("an auto-param declaration must register, at the type with the tactic ERASED");

    let shown = repl.execute("Check guarded.").expect("it is there");
    assert!(
        !shown.contains("autoparam") && !shown.contains("decide"),
        "the tactic is SURFACE: no marker and no tactic name may reach the registered type: {shown}"
    );
    let applied = repl.execute("Check (guarded 1).").expect("it applies");
    assert!(
        applied.contains("Eq") || applied.contains('\u{3a0}'),
        "the obligation is not discharged yet, so `guarded 1` is a function still awaiting the \
         proof — a partial application, never a silently completed one: {applied}"
    );
}

/// A default and a leading IMPLICIT coexist: the implicit is still inserted and inferred, and the
/// default is still filled, because the two live on different axes — one is what the elaborator
/// solves, the other is what the surface expands.
#[test]
fn an_implicit_and_a_default_coexist_on_one_declaration() {
    let mut repl = Repl::new();
    repl.execute(
        "Definition tagged : {A : Type} -> A -> (m : Int := 7) -> Int := \
         fun A : Type => fun a : A => fun m : Int => m.",
    )
    .expect("a leading implicit followed by a defaulted parameter");
    assert_eq!(
        repl.execute("Eval (tagged Zero).")
            .expect("both are supplied for me")
            .trim(),
        "7",
        "`A` was inferred from `Zero` and `m` was filled from the declaration"
    );
    assert_eq!(
        repl.execute("Eval (tagged Zero 9).")
            .expect("the default written out")
            .trim(),
        "9",
        "…and writing the argument still overrides it"
    );
}

/// `Print Axioms` is an ordinary command, so a scoped `Open … in` brackets it like any other.
#[test]
fn print_axioms_can_be_scoped_by_open_in() {
    let mut repl = Repl::new();
    repl.execute("Definition Audit.clean : Nat := Zero.")
        .expect("a namespaced declaration");
    let scoped = repl
        .execute("Open Audit in Print Axioms clean.")
        .expect("the opening scopes the report's own subject");
    assert!(
        scoped.contains("Audit.clean"),
        "the short name resolved under the opening, and the report names the key: {scoped}"
    );
    assert!(
        repl.execute("Print Axioms clean.").is_err(),
        "…and the opening covered exactly that one command"
    );
}

/// A DEFAULT ACROSS AN IMPLICIT, both ways. The positive: a default that names an EXPLICIT earlier
/// parameter is closed with that parameter's argument even when an implicit sits in front of it.
/// The negative: a default that names the IMPLICIT is not filled at all, because the one thing we
/// could put there is a placeholder — and a `Term::Hole` in an argument position matches ANY type.
#[test]
fn a_default_across_an_implicit_is_closed_or_refused_never_guessed() {
    let mut repl = Repl::new();
    repl.execute(
        "Definition echo : {A : Type} -> (a : A) -> (b : A := a) -> A := \
         fun A : Type => fun a : A => fun b : A => b.",
    )
    .expect("a default naming an explicit parameter, behind an implicit one");
    assert_eq!(
        repl.execute("Eval (echo (Succ Zero)).")
            .expect("the default is closed")
            .trim(),
        "(Succ Zero)",
        "`b` defaults to `a`, and `a` is the argument that WAS written — the implicit in front of \
         it does not shift the alignment"
    );

    repl.execute(
        "Definition carrier : {A : Type} -> A -> (t : Type := A) -> Type := \
         fun A : Type => fun a : A => fun t : Type => t.",
    )
    .expect("a default naming the IMPLICIT parameter");
    let unfilled = repl
        .execute("Check (carrier Zero).")
        .expect("it still applies");
    assert!(
        !unfilled.contains("?") && !unfilled.contains("_"),
        "nothing was guessed into the argument position: {unfilled}"
    );
    assert!(
        unfilled.contains("Type"),
        "the default names a parameter the elaborator has yet to solve, so it is NOT filled — \
         `carrier Zero` is a function still awaiting `t`: {unfilled}"
    );
}

/// The remaining raw-search splits at the inductive door, from both directions. A «quoted name»
/// containing a `(` is a NAME, not the start of a parameter list; a «quoted» parameter name
/// containing a `:` is a name, not the ascription.
#[test]
fn an_inductive_header_and_its_parameters_split_at_the_name_grammar() {
    let mut repl = Repl::new();
    repl.execute("Inductive \u{ab}odd(name\u{bb} := only : \u{ab}odd(name\u{bb}.")
        .expect("a quoted inductive name containing a `(` is a name, not a parameter list");
    assert!(
        repl.context().get_global("odd(name").is_some(),
        "the inductive is keyed at the whole quoted name"
    );

    let mut params = Repl::new();
    params
        .execute("Inductive Box (\u{ab}a:b\u{bb} : Type) := wrap : \u{ab}a:b\u{bb} -> Box \u{ab}a:b\u{bb}.")
        .expect("a quoted PARAMETER name containing a `:` splits at its own ascription");
    assert!(
        params.context().get_global("Box").is_some(),
        "the parametric inductive registers"
    );

    // …and the ordinary spellings are untouched, which is what makes this a split rule rather than
    // a new grammar.
    let mut plain = Repl::new();
    plain
        .execute("Inductive Pair (A : Type) (B : Type) := both : A -> B -> Pair A B.")
        .expect("two ordinary parameters still parse");
    plain
        .execute("Check Pair.")
        .expect("and the header is the name it always was");
}

// ============================================================================================
// LEAN'S `resolveExact` — A DOTTED NAME THAT EXISTS AT THE ROOT IS NOT AMBIGUOUS
//
// `Lean/ResolveName.lean:163-186` is one `match` deep and the middle arm is the one this lane had
// missing. After the enclosing-namespace walk fails, Lean tries `resolveExact`
// (`ResolveName.lean:121-131`), which returns `none` for an ATOMIC name and, for a dotted one,
// the exact top-level constant if the environment holds it — and a `some` there RETURNS, so the
// `open` decls at line 180 are never consulted. Only an atomic name reaches the branch where the
// root and the opens sit at one priority and two readings are an ambiguity.
//
// Measured against lean 4.14.0 on this box, not inferred:
//   `Alpha.Inner.x` + `Inner.x`, `open Alpha`, `#eval Inner.x`  ⇒ 2   (the ROOT's, no error)
//   `Alpha.y` + `y`,             `open Alpha`, `#eval y`        ⇒ error: ambiguous
// ============================================================================================

/// ★ THE DEFECT. A dotted name the root declares resolved as AMBIGUOUS here whenever an `open`
/// offered a second reading — a FALSE error, refusing a program Lean accepts. `Inner.x` is not a
/// short name; it names one constant exactly, and Lean's `resolveExact` says so before any `open`
/// gets a vote.
#[test]
fn a_dotted_name_the_root_declares_is_not_made_ambiguous_by_an_open() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.Inner.x : Nat := (Succ Zero).")
        .expect("Alpha.Inner.x");
    repl.execute("Definition Inner.x : Nat := (Succ (Succ Zero)).")
        .expect("Inner.x");
    repl.execute("Open Alpha.").expect("Alpha exists");

    let checked = repl.execute("Check Inner.x.");
    assert!(
        checked.is_ok(),
        "`Inner.x` is an EXACT top-level name, so Lean's `resolveExact` answers before the opens \
         are consulted and there is no ambiguity to report: {checked:?}"
    );
    assert_eq!(
        repl.execute("Eval Inner.x.").unwrap().trim(),
        "(Succ (Succ Zero))",
        "…and the reading is the ROOT's constant, exactly as lean 4.14.0 evaluates it to 2"
    );

    // ★ THE NEGATIVE, one rung down: the ATOMIC counterpart of the same shape stays AMBIGUOUS,
    // because `resolveExact` refuses to answer for an atomic name (`ResolveName.lean:122`).
    let mut atomic = Repl::new();
    atomic
        .execute("Definition Alpha.y : Nat := (Succ Zero).")
        .expect("Alpha.y");
    atomic
        .execute("Definition y : Nat := (Succ (Succ Zero)).")
        .expect("y");
    atomic.execute("Open Alpha.").expect("Alpha exists");
    let report = format!("{:?}", atomic.execute("Check y."));
    assert!(
        report.contains("Ambiguous") && report.contains("Alpha.y"),
        "an ATOMIC name answered by both an open and the root is Lean's ambiguity, and must stay \
         one: {report}"
    );
}

/// The BOUNDARY on the other side, so the new rung is a short circuit and not a blanket
/// root-preference: with nothing at the root to be exact ABOUT, a dotted name still goes to the
/// opens, and two opens answering it are still an ambiguity.
#[test]
fn a_dotted_name_with_no_root_reading_still_goes_to_the_opens() {
    // One open, one reading: it resolves THROUGH the open.
    let mut one = Repl::new();
    one.execute("Definition Alpha.Inner.x : Nat := (Succ Zero).")
        .expect("Alpha.Inner.x");
    one.execute("Open Alpha.").expect("Alpha exists");
    assert_eq!(
        one.execute("Eval Inner.x.").unwrap().trim(),
        "(Succ Zero)",
        "with no `Inner.x` at the root, the open is the only reading and must be taken"
    );

    // Two opens, two readings, still no root one: AMBIGUOUS — lean 4.14.0 reports
    // `ambiguous, possible interpretations Q.I.z, P.I.z` for exactly this shape.
    let mut two = Repl::new();
    two.execute("Definition P.I.z : Nat := (Succ Zero).")
        .expect("P.I.z");
    two.execute("Definition Q.I.z : Nat := (Succ (Succ Zero)).")
        .expect("Q.I.z");
    two.execute("Open P.").expect("P exists");
    two.execute("Open Q.").expect("Q exists");
    let report = format!("{:?}", two.execute("Check I.z."));
    assert!(
        report.contains("Ambiguous") && report.contains("P.I.z") && report.contains("Q.I.z"),
        "a dotted name with no exact root reading is still resolved through the opens, and two \
         of them is an ambiguity that must name BOTH: {report}"
    );
}

/// …and the rung ABOVE it is untouched: the enclosing namespace is walked BEFORE `resolveExact`
/// (`ResolveName.lean:171-174`), so a relative reading inside a namespace still beats the exact
/// top-level one. lean 4.14.0 evaluates the same program to 5, not 2.
#[test]
fn the_enclosing_namespace_is_walked_before_the_exact_root_reading() {
    let mut repl = Repl::new();
    repl.execute("Definition Inner.x : Nat := (Succ (Succ Zero)).")
        .expect("root Inner.x");
    repl.execute("Definition N.Inner.x : Nat := (Succ (Succ (Succ Zero))).")
        .expect("N.Inner.x");
    repl.execute("Namespace N.").expect("enter N");
    repl.execute("Definition useRelative : Nat := Inner.x.")
        .expect("relative dotted reference");
    repl.execute("End N.").expect("leave N");
    assert_eq!(
        repl.execute("Eval N.useRelative.").unwrap().trim(),
        "(Succ (Succ (Succ Zero)))",
        "inside `N`, `Inner.x` is `N.Inner.x` — the namespace walk runs first and returns at the \
         first prefix that resolves"
    );

    // BOTH KERNELS, because a resolver that picked a different constant would produce a term that
    // still type-checks and denotes something else — the failure mode with no symptom.
    use logicaffeine_kernel::{double_check, DoubleCheck, Term};
    assert_eq!(
        double_check(repl.context(), &Term::Global("N.useRelative".to_string())),
        DoubleCheck::Agreed,
        "both kernels must agree about the constant the relative reading picked"
    );
}

// ============================================================================================
// LEAN'S EXPLICIT `open` IS A PREFIX, NOT A WHOLE NAME — AND `renaming` IS THE SAME MECHANISM
//
// `Lean/Elab/Open.lean:73-79` elaborates `open Foo (x)` into `OpenDecl.explicit x Foo.x`, and
// `Lean/ResolveName.lean:142-154` resolves such a decl in TWO ways: `openedId == id`, and
// `openedId.isPrefixOf id`, in which case the prefix is REPLACED. So `open Foo (x)` brings in
// `x` AND everything under it. `Lean/Elab/Open.lean:89-96` builds `renaming` out of the very same
// `OpenDecl.explicit`, with the local spelling on the left — which is why a renamed name carries
// its sub-names along too, and why a `renaming` clause brings in NOTHING else.
//
// Measured against lean 4.14.0 on this box:
//   `def Foo.x := 1`, `def Foo.x.y := 3`, `open Foo (x)` ⇒ `x` = 1, `x.y` = 3, `other` unknown
//   `open Baz renaming orig → ali`        ⇒ `ali` = 42, `ali.deep` = 43, `orig` and the rest unknown
//   `open Bar hiding a`                   ⇒ `a` unknown, but `a.deep` STILL resolves (exact match only)
// ============================================================================================

/// ★ THE DEFECT. A narrowed opening compared the WHOLE written name against its list, so
/// `Open Foo (x).` brought in `x` and refused `x.y` — a name Lean resolves, because its explicit
/// open replaces a PREFIX.
#[test]
fn a_narrowed_opening_carries_the_names_underneath_the_ones_it_lists() {
    let mut repl = Repl::new();
    repl.execute("Definition Foo.x : Nat := (Succ Zero).")
        .expect("Foo.x");
    repl.execute("Definition Foo.x.y : Nat := (Succ (Succ Zero)).")
        .expect("Foo.x.y");
    repl.execute("Definition Foo.other : Nat := Zero.")
        .expect("Foo.other");
    repl.execute("Open Foo (x).").expect("a narrowed opening");

    assert_eq!(
        repl.execute("Eval x.").unwrap().trim(),
        "(Succ Zero)",
        "the listed name itself"
    );
    assert_eq!(
        repl.execute("Eval x.y.").unwrap().trim(),
        "(Succ (Succ Zero))",
        "`x` is a PREFIX of `x.y`, so Lean's explicit open replaces it and `Foo.x.y` is reached — \
         lean 4.14.0 evaluates this exact program to 3"
    );

    // ★ THE NEGATIVE, and the whole point of narrowing: a sibling the clause did NOT list stays
    // out, so the prefix rule is a rule about the listed names and not a wholesale opening.
    let other = repl.execute("Check Foo.other.");
    assert!(
        other.is_ok(),
        "the absolute name is always reachable: {other:?}"
    );
    let short = repl.execute("Check other.");
    assert!(
        short.is_err(),
        "`other` was not listed, so a narrowed opening must not bring it in: {short:?}"
    );
}

/// The BOUNDARY on the `hiding` side, which is NOT prefix-matched — Lean's `resolveOpenDecls`
/// tests `exs.contains id`, an exact membership (`ResolveName.lean:136-137`). The asymmetry is
/// Lean's, measured: `open Bar hiding a` leaves `a` unknown and `a.deep` resolving.
#[test]
fn a_hiding_open_hides_exactly_the_name_it_names_and_not_what_is_under_it() {
    let mut repl = Repl::new();
    repl.execute("Definition Bar.a : Nat := (Succ Zero).")
        .expect("Bar.a");
    repl.execute("Definition Bar.b : Nat := (Succ (Succ Zero)).")
        .expect("Bar.b");
    repl.execute("Definition Bar.a.deep : Nat := Zero.")
        .expect("Bar.a.deep");
    repl.execute("Open Bar hiding (a).")
        .expect("a hiding opening");

    assert_eq!(
        repl.execute("Eval b.").unwrap().trim(),
        "(Succ (Succ Zero))",
        "the rest comes in"
    );
    assert_eq!(
        repl.execute("Eval a.deep.").unwrap().trim(),
        "Zero",
        "`hiding a` hides the NAME `a`, not the namespace under it — lean 4.14.0 evaluates this \
         exact program to 7 for `a.deep`"
    );
    assert!(
        repl.execute("Check a.").is_err(),
        "…and the name it does hide is genuinely gone"
    );
}

/// `Open Baz renaming orig -> ali.` — Lean's fourth `open` form, which this surface did not have.
/// It is the one way to bring in a name whose short spelling already means something else here,
/// so its absence is not cosmetic: without it a clash can only be worked around by writing every
/// occurrence absolutely.
#[test]
fn a_renaming_open_gives_one_name_a_local_spelling_and_brings_in_nothing_else() {
    let mut repl = Repl::new();
    repl.execute("Definition Baz.orig : Nat := (Succ Zero).")
        .expect("Baz.orig");
    repl.execute("Definition Baz.orig.deep : Nat := (Succ (Succ Zero)).")
        .expect("Baz.orig.deep");
    repl.execute("Definition Baz.untouched : Nat := Zero.")
        .expect("Baz.untouched");
    repl.execute("Open Baz renaming orig -> ali.")
        .expect("a renaming opening");

    assert_eq!(
        repl.execute("Eval ali.").unwrap().trim(),
        "(Succ Zero)",
        "the new spelling denotes it"
    );
    assert_eq!(
        repl.execute("Eval ali.deep.").unwrap().trim(),
        "(Succ (Succ Zero))",
        "a renaming is an explicit open, so the local spelling is a PREFIX too — lean 4.14.0 \
         evaluates `ali.deep` to 43"
    );

    // ★ THE NEGATIVES, both of them. The OLD spelling is not brought in, and neither is anything
    // the clause did not rename: lean 4.14.0 reports `unknown identifier` for both.
    assert!(
        repl.execute("Check orig.").is_err(),
        "a renaming replaces the spelling; the original short name is NOT also brought in"
    );
    assert!(
        repl.execute("Check untouched.").is_err(),
        "a renaming clause is an explicit opening — it brings in exactly what it renames"
    );

    // …and the absolute name is untouched by any of it.
    assert_eq!(repl.execute("Eval Baz.untouched.").unwrap().trim(), "Zero");

    // The arrow is written either way, as Lean writes it either way.
    let mut unicode = Repl::new();
    unicode
        .execute("Definition Baz.orig : Nat := (Succ Zero).")
        .expect("Baz.orig");
    unicode
        .execute("Open Baz renaming orig \u{2192} ali.")
        .expect("the unicode arrow");
    assert_eq!(unicode.execute("Eval ali.").unwrap().trim(), "(Succ Zero)");
}

/// A renaming clause naming something the namespace does not declare is REFUSED, exactly as the
/// other narrowed forms are and exactly as Lean refuses it (`unknown constant 'Baz.nosuch'`). A
/// clause that renamed nothing would read exactly like one that worked.
#[test]
fn a_renaming_of_an_absent_member_is_refused() {
    let mut repl = Repl::new();
    repl.execute("Definition Baz.orig : Nat := (Succ Zero).")
        .expect("Baz.orig");
    let refused = repl.execute("Open Baz renaming nosuch -> ali.");
    assert!(
        refused.is_err() && format!("{refused:?}").contains("Baz.nosuch"),
        "a renaming must name a member that exists, and the report must name it: {refused:?}"
    );
    // …and nothing is left behind: the clause did not half-apply.
    assert!(
        repl.execute("Check ali.").is_err(),
        "a refused clause must bring in no spelling at all"
    );
    assert!(
        repl.execute("Check orig.").is_err(),
        "…and must not have opened the namespace wholesale on the way to failing"
    );
    // The BOUNDARY: the same clause with a member that DOES exist is accepted.
    repl.execute("Open Baz renaming orig -> ali.")
        .expect("the member exists");
    assert_eq!(repl.execute("Eval ali.").unwrap().trim(), "(Succ Zero)");
}

/// Two renamings in one clause, and a renaming scoped by `in` — the composition checks, because a
/// list is where an off-by-one lives and `in` is where a scope fails to come off.
#[test]
fn renamings_compose_in_one_clause_and_come_off_with_the_scope() {
    let mut repl = Repl::new();
    repl.execute("Definition Baz.a : Nat := (Succ Zero).")
        .expect("Baz.a");
    repl.execute("Definition Baz.b : Nat := (Succ (Succ Zero)).")
        .expect("Baz.b");
    repl.execute("Open Baz renaming a -> one, b -> two.")
        .expect("two renamings");
    assert_eq!(repl.execute("Eval one.").unwrap().trim(), "(Succ Zero)");
    assert_eq!(
        repl.execute("Eval two.").unwrap().trim(),
        "(Succ (Succ Zero))"
    );

    let mut scoped = Repl::new();
    scoped
        .execute("Definition Baz.a : Nat := (Succ Zero).")
        .expect("Baz.a");
    scoped
        .execute("Open Baz renaming a -> one in Definition uses : Nat := (Succ one).")
        .expect("a scoped renaming");
    assert_eq!(
        scoped.execute("Eval uses.").unwrap().trim(),
        "(Succ (Succ Zero))"
    );
    assert!(
        scoped.execute("Check one.").is_err(),
        "a scoped opening covers exactly one command, renaming or not"
    );
}

// ============================================================================================
// `Section.` / `End.` — LEAN'S ANONYMOUS SECTION, AND THE `End` THAT MUST NOT CROSS ONE
//
// `section` with no name is ordinary Lean and is how a `local` is scoped without inventing a
// label. The keyword is shared with namespaces, so what an `End` closes is decided by which scope
// is INNERMOST — and that decision has to be made before the namespace stack is touched, not
// after. Measured against lean 4.14.0:
//   `section … end`                        ⇒ accepted
//   `namespace N … end`                    ⇒ invalid 'end', name is missing (expected N)
//   `section … end Foo`                    ⇒ invalid 'end', name mismatch (expected nothing)
//   `namespace N; section S; … end N`      ⇒ invalid 'end', name mismatch (expected S)
// ============================================================================================

/// An anonymous section opens and a bare `End` closes it — and closes ONLY it.
#[test]
fn an_anonymous_section_opens_and_only_a_bare_end_closes_it() {
    let mut repl = Repl::new();
    repl.execute("Section.")
        .expect("`section` with no name is ordinary Lean");
    repl.execute("Definition inSection : Nat := Zero.")
        .expect("a declaration inside it");
    assert_eq!(
        repl.context().scope_depth(),
        1,
        "an anonymous section is a kernel scope like any other — that is what it is FOR"
    );
    repl.execute("End.").expect("a bare `End` closes it");
    assert_eq!(repl.context().scope_depth(), 0, "and the pair balances");
    assert!(
        repl.context().is_definition("inSection"),
        "a section scopes attributes, never declarations"
    );

    // ★ THE NEGATIVE. A bare `End` closes an anonymous section and nothing else: a NAMED section
    // and a namespace each need their name back, or a file could lose track of which scope it is
    // in without a word. lean 4.14.0 reports `name is missing (expected N)` for the namespace.
    let mut named = Repl::new();
    named.execute("Section Named.").expect("a named section");
    assert!(
        named.execute("End.").is_err(),
        "a bare `End` must not close a NAMED section"
    );
    named.execute("End Named.").expect("its own name does");

    let mut ns = Repl::new();
    ns.execute("Namespace N.").expect("a namespace");
    assert!(ns.execute("End.").is_err(), "…nor a namespace");
    ns.execute("End N.").expect("its own name does");

    // …and the mirror: a NAMED `End` must not close an anonymous section. lean 4.14.0 reports
    // `name mismatch (expected nothing)`.
    let mut anon = Repl::new();
    anon.execute("Section.").expect("an anonymous section");
    assert!(
        anon.execute("End Whatever.").is_err(),
        "an anonymous section is closed by a bare `End`, not by a name nobody wrote"
    );
    anon.execute("End.")
        .expect("and the bare form still works after the refusal");
}

/// ★ THE DEFECT. `End N.` closed the namespace `N` straight THROUGH an open section: the namespace
/// stack was consulted FIRST and popped on a name match, so `Namespace N. Section S. End N.` left
/// `S` open and `N` closed. Every later declaration was then keyed OUTSIDE the namespace its
/// author was writing in, and the file still checked. lean 4.14.0 refuses that exact program.
#[test]
fn an_end_cannot_close_a_namespace_through_an_open_section() {
    let mut repl = Repl::new();
    repl.execute("Namespace N.").expect("a namespace");
    repl.execute("Section S.").expect("a section inside it");
    let crossed = repl.execute("End N.");
    assert!(
        crossed.is_err(),
        "the innermost open scope is the section `S`, so `End N` must not reach past it — \
         lean 4.14.0 says `invalid 'end', name mismatch (expected S)`: {crossed:?}"
    );

    // …and nothing moved: a refused `End` must leave BOTH stacks exactly where they were, or the
    // refusal is worse than the mistake.
    repl.execute("Definition stillInside : Nat := Zero.")
        .expect("a declaration");
    assert!(
        repl.context().get_global("N.stillInside").is_some(),
        "the namespace is still current after the refused `End`"
    );
    assert_eq!(
        repl.context().scope_depth(),
        2,
        "and both scopes are still open"
    );

    // The BOUNDARY, closing in the right order: it all comes off.
    repl.execute("End S.").expect("the section closes first");
    repl.execute("End N.").expect("and then the namespace");
    repl.execute("Definition outside : Nat := Zero.")
        .expect("at the root again");
    assert!(repl.context().get_global("outside").is_some());
    assert_eq!(repl.context().scope_depth(), 0);
}

// ============================================================================================
// `Protected` — LEAN'S `protected def`, WHICH IS A RESOLUTION FACT AND NOT A PERMISSION
//
// Corpus case T02-012 names this in its own guard: "Catches assuming `protected` is inert. It is
// inert *logically*, but it changes which short name resolves at a use site." The rule is one
// conjunct of `resolveQualifiedName` (`Lean/ResolveName.lean:104`):
// `!id.isAtomic || !isProtected env resolvedId` — so a protected declaration is dropped exactly
// when the name WRITTEN is atomic, on the two paths that go through that function (the enclosing
// namespace walk, and a wholesale `open`) and on no other.
//
// Every one of the following was run against lean 4.14.0 on this box:
//   `protected def Foo.x`, `open Foo`, `#check x`          ⇒ unknown identifier 'x'
//   … `#check Foo.x`                                        ⇒ Foo.x : Nat
//   … `#eval _root_.Foo.x`                                  ⇒ 1
//   inside `namespace Foo.Bar`, `def y := x`                ⇒ unknown identifier 'x'
//   `open Foo (x)` / `open Foo renaming x → q`              ⇒ 1  (the EXPLICIT arm does not check)
//   `protected inductive Foo.Col`, `open Foo`, `#check Col` ⇒ unknown identifier 'Col'
// ============================================================================================

/// A protected declaration stops answering to its SHORT name and keeps answering to its full one.
#[test]
fn a_protected_declaration_stops_answering_to_its_short_name() {
    let mut repl = Repl::new();
    repl.execute("Protected Definition Foo.x : Nat := (Succ Zero).")
        .expect("an ordinary constant");
    repl.execute("Definition Foo.y : Nat := (Succ (Succ Zero)).")
        .expect("an unprotected sibling");
    repl.execute("Open Foo.").expect("Foo exists");

    assert_eq!(
        repl.execute("Eval y.").unwrap().trim(),
        "(Succ (Succ Zero))",
        "the unprotected sibling comes in, so this is not an `open` that failed"
    );
    let short = repl.execute("Check x.");
    assert!(
        short.is_err(),
        "`protected` drops the declaration exactly when the written name is ATOMIC: {short:?}"
    );
    assert_eq!(
        repl.execute("Eval Foo.x.").unwrap().trim(),
        "(Succ Zero)",
        "…and the qualified name is untouched — `protected` is not a permission"
    );
    assert_eq!(
        repl.execute("Eval _root_.Foo.x.").unwrap().trim(),
        "(Succ Zero)",
        "…nor is the `_root_` escape refused by it"
    );

    // It is a DECLARATION like any other, and both kernels agree about it.
    use logicaffeine_kernel::{double_check, DoubleCheck, Term};
    assert_eq!(
        double_check(repl.context(), &Term::Global("Foo.x".to_string())),
        DoubleCheck::Agreed,
        "`protected` is a spelling rule; a spelling rule cannot reach the kernel at all"
    );
}

/// The ENCLOSING-NAMESPACE walk consults it too, which is the half that is easy to miss: a sibling
/// namespace writing the short name does not reach a protected declaration either.
#[test]
fn a_protected_declaration_is_shielded_from_its_own_enclosing_namespace() {
    let mut repl = Repl::new();
    repl.execute("Protected Definition Foo.x : Nat := (Succ Zero).")
        .expect("protected");
    repl.execute("Namespace Foo.").expect("enter Foo");
    repl.execute("Namespace Bar.")
        .expect("and Foo.Bar inside it");
    let relative = repl.execute("Definition y : Nat := x.");
    assert!(
        relative.is_err(),
        "inside `Foo.Bar`, the short name `x` must NOT reach the protected `Foo.x` — lean 4.14.0 \
         reports `unknown identifier 'x'` for exactly this: {relative:?}"
    );
    // …and the qualified spelling inside the same namespace does reach it.
    repl.execute("Definition z : Nat := Foo.x.")
        .expect("the qualified name still resolves");
    repl.execute("End Bar.").expect("leave");
    repl.execute("End Foo.").expect("leave");
    assert_eq!(
        repl.execute("Eval Foo.Bar.z.").unwrap().trim(),
        "(Succ Zero)"
    );
}

/// The BOUNDARY, and it is the one a guess would get wrong: an EXPLICIT opening reaches a
/// protected declaration. Lean's `protected` check lives in `resolveQualifiedName`, which the
/// explicit arm of `resolveOpenDecls` never calls — a name written out in the clause was asked for
/// by name, so there is nothing left to protect it from.
#[test]
fn protected_does_not_shield_a_name_an_open_clause_writes_out() {
    for clause in ["Open Foo (x).", "Open Foo renaming x -> q."] {
        let spelling = if clause.contains("renaming") {
            "q"
        } else {
            "x"
        };
        let mut repl = Repl::new();
        repl.execute("Protected Definition Foo.x : Nat := (Succ Zero).")
            .expect("protected");
        repl.execute(clause)
            .unwrap_or_else(|e| panic!("`{clause}` must be accepted: {e:?}"));
        assert_eq!(
            repl.execute(&format!("Eval {spelling}."))
                .unwrap_or_default()
                .trim(),
            "(Succ Zero)",
            "`{clause}` names the declaration explicitly, so `protected` does not refuse it — \
             lean 4.14.0 evaluates both of these to 1"
        );
    }

    // …and the WHOLESALE opening of the same namespace still does refuse it, so the distinction is
    // between the two `OpenDecl` constructors and not a hole in the check.
    let mut wholesale = Repl::new();
    wholesale
        .execute("Protected Definition Foo.x : Nat := (Succ Zero).")
        .expect("protected");
    wholesale.execute("Open Foo.").expect("Foo exists");
    assert!(
        wholesale.execute("Check x.").is_err(),
        "a wholesale `Open` is still refused"
    );
}

/// `Protected` on an inductive shields the TYPE's short name, and on a command that declares
/// nothing it is refused by name rather than quietly ignored.
#[test]
fn protected_covers_the_inductive_door_and_refuses_what_declares_nothing() {
    let mut repl = Repl::new();
    repl.execute("Protected Inductive Foo.Col := red : Col.")
        .expect("an ordinary inductive");
    repl.execute("Open Foo.").expect("Foo exists");
    assert!(
        repl.execute("Check Col.").is_err(),
        "lean 4.14.0 answers `unknown identifier 'Col'` after `protected inductive Foo.Col`"
    );
    repl.execute("Check Foo.Col.")
        .expect("…and the qualified name is ordinary");

    // ★ THE NEGATIVE. `Protected` on something that declares no name has nothing to protect, and a
    // qualifier that silently did nothing is the quietest kind of lie.
    for src in [
        "Protected Check Zero.",
        "Protected Open Foo.",
        "Protected Section S.",
    ] {
        let refused = Repl::new().execute(src);
        assert!(
            refused.is_err(),
            "`{src}` protects nothing, so it must be REFUSED rather than accepted as a no-op: \
             {refused:?}"
        );
    }

    // …and a protected declaration that does not CHECK protects nothing either: the mark is
    // applied after the inner command succeeds.
    let mut failed = Repl::new();
    assert!(failed
        .execute("Protected Definition Foo.bad : Nat := Nat.")
        .is_err());
    failed
        .execute("Definition Foo.bad : Nat := Zero.")
        .expect("the name is free");
    failed.execute("Open Foo.").expect("Foo exists");
    assert_eq!(
        failed.execute("Eval bad.").unwrap().trim(),
        "Zero",
        "a refused `Protected Definition` must leave no protection behind"
    );
}

/// ★ AN `export` OUTLIVES ITS NAMESPACE; AN `open` DOES NOT. The two are both "spellings written
/// inside a scope", and treating them as one kind of thing made `Export` useless for the one job
/// it has: publishing a name from inside the namespace that is to answer to it. Lean keeps them
/// apart by construction — `addAlias` (`Lean/ResolveName.lean:70-71`) is a persistent ENVIRONMENT
/// extension and an `OpenDecl` is elaborator `Scope` state — and lean 4.14.0 prints 5 for
/// `namespace B export A (x) end B` followed by `#eval B.x`.
#[test]
fn an_export_survives_the_end_of_the_namespace_it_was_written_in() {
    let mut repl = Repl::new();
    repl.execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");
    repl.execute("Namespace Host.").expect("enter Host");
    repl.execute("Export Alpha (token).")
        .expect("export it into Host");
    repl.execute("End Host.").expect("leave Host");

    assert_eq!(
        repl.execute("Eval Host.token.").unwrap_or_default().trim(),
        "(Succ Zero)",
        "the alias is an environment fact and must still answer after its namespace closed"
    );
    // …and it is reachable through an `open` of the namespace that published it, which is the
    // combination the whole mechanism exists for.
    repl.execute("Open Host.").expect("Host exists");
    assert_eq!(
        repl.execute("Eval token.").unwrap_or_default().trim(),
        "(Succ Zero)",
        "`Open Host.` brings in the exported spelling exactly as it brings in a declaration"
    );

    // ★ THE NEGATIVE, which is what an `open` still does NOT do: an `open` written inside a
    // namespace dies with it. The two must not have been collapsed into one rule.
    let mut opened = Repl::new();
    opened
        .execute("Definition Alpha.token : Nat := (Succ Zero).")
        .expect("Alpha.token");
    opened.execute("Namespace Host.").expect("enter Host");
    opened
        .execute("Open Alpha.")
        .expect("open inside the namespace");
    opened
        .execute("Definition inside : Nat := token.")
        .expect("the opening is in force here");
    opened.execute("End Host.").expect("leave Host");
    assert!(
        opened.execute("Check token.").is_err(),
        "an `Open` written inside a namespace must still die with it"
    );

    // …and an alias still declares NOTHING: no second copy landed in the context.
    assert!(
        repl.context().get_global("Host.token").is_none(),
        "an alias is a spelling — it must never put a second constant in the context"
    );
}

// ============================================================================================
// THE DIFFERENTIAL: OUR RESOLVER AGAINST AN INDEPENDENT TRANSCRIPTION OF LEAN'S OWN FUNCTION
//
// Every rung this lane added — the enclosing-namespace walk, `resolveExact`, the root-and-opens
// group, the explicit-open prefix rule, `protected` — is a branch of ONE function in Lean
// (`ResolveName.lean:163-186`, `resolveGlobalName`), and a resolver that gets four of the five
// right is a resolver that silently denotes the wrong constant. So the rungs are not checked one
// at a time here: `lean_candidates` below is that function transcribed from the Lean source in
// LEAN'S order and LEAN'S shape — the namespace walk, then `resolveExact`, then the root seeded
// into `resolveOpenDecls` — and the grid runs both over every combination.
//
// It is an independent model in the sense that matters: it is written from the .lean file rather
// than from `surface.rs`, it is structured as Lean structures it (our `resolve` returns early on
// `_root_`, where Lean reaches the same answer through `resolveExact`), and it computes a LIST of
// candidates where ours computes a verdict — so "one candidate" versus "an ambiguity" is derived
// on both sides rather than copied.
// ============================================================================================

/// One `open` in the model, in Lean's own two constructors (`Lean/Data/OpenDecl.lean`).
#[derive(Clone)]
enum ModelOpen {
    /// `OpenDecl.simple ns exs` — `open Foo` / `open Foo hiding a`.
    Simple(&'static str, &'static [&'static str]),
    /// `OpenDecl.explicit openedId resolvedId` — `open Foo (x)` and `open Foo renaming a -> x`.
    Explicit(&'static str, &'static str),
}

/// `Lean/ResolveName.lean:163-186`, transcribed for the fragment this surface implements: no
/// private names, no macro scopes, no reserved-name predicates, no aliases.
fn lean_candidates(
    env: &[&str],
    protected: &[&str],
    current: &[&str],
    opens: &[ModelOpen],
    written: &str,
) -> Vec<String> {
    let parts: Vec<&str> = written.split('.').collect();
    let atomic = parts.len() == 1; // Lean/Data/Name.lean:154-158
    let contains = |k: &str| env.contains(&k);
    // resolveQualifiedName, ResolveName.lean:100-110 — the ONE place `isProtected` is consulted.
    let qualified = |ns: &str, id: &str| -> Option<String> {
        let key = if ns.is_empty() {
            id.to_string()
        } else {
            format!("{ns}.{id}")
        };
        let shielded = atomic && protected.contains(&key.as_str());
        (contains(&key) && !shielded).then_some(key)
    };

    // 1 · resolveUsingNamespace (lines 112-118): the prefixes of the current namespace, longest
    //     first, returning at the FIRST that resolves. `.anonymous` returns [], so the root is not
    //     one of them.
    for depth in (1..=current.len()).rev() {
        if let Some(key) = qualified(&current[..depth].join("."), written) {
            return vec![key];
        }
    }

    // 2 · resolveExact (lines 120-131): `none` for an atomic name; otherwise the exact top-level
    //     constant, with `_root_` stripped. A `some` here RETURNS.
    if !atomic {
        let exact = written.strip_prefix("_root_.").unwrap_or(written);
        if contains(exact) {
            return vec![exact.to_string()];
        }
    }

    // 3 · the root reading, seeded into resolveOpenDecls (lines 177-181). One priority, so two
    //     answers is an ambiguity and not a shadow.
    let mut ids: Vec<String> = if contains(written) {
        vec![written.to_string()]
    } else {
        Vec::new()
    };
    // resolveOpenDecls (lines 134-154) walks the decl list head-first, most recent first.
    for open in opens.iter().rev() {
        match open {
            ModelOpen::Simple(ns, exs) => {
                if exs.contains(&written) {
                    continue;
                }
                if let Some(key) = qualified(ns, written) {
                    ids.insert(0, key);
                }
            }
            ModelOpen::Explicit(opened, resolved) => {
                if *opened == written {
                    ids.insert(0, resolved.to_string());
                } else if let Some(tail) = written.strip_prefix(&format!("{opened}.")) {
                    let candidate = format!("{resolved}.{tail}");
                    if contains(&candidate) {
                        ids.insert(0, candidate);
                    }
                }
            }
        }
    }
    ids.dedup();
    ids
}

/// The grid. Every scope shape this lane can build, crossed with every written name that could
/// distinguish two rungs — and the adversarial ones are in it on purpose: the dotted name that
/// exists at the root AND under an open, the atomic one that does, the protected name reached
/// four different ways, and the explicit open whose listed name is a PREFIX of what is written.
#[test]
fn the_resolver_agrees_with_lean_over_the_whole_resolution_grid() {
    use logicaffeine_kernel::interface::surface::{Name, Namespaces, OpenClause, Resolution};

    // The environment every row is resolved against: deliberately full of collisions.
    const ENV: &[&str] = &[
        "x",
        "Inner.x",
        "y",
        "Alpha.x",
        "Alpha.y",
        "Alpha.Inner.x",
        "Alpha.p",
        "Alpha.p.q",
        "Alpha.secret",
        "Beta.x",
        "Beta.Inner.x",
        "N.x",
        "N.Inner.x",
        "N.M.x",
    ];
    const PROTECTED: &[&str] = &["Alpha.secret"];

    let scopes: Vec<(&str, Vec<&str>, Vec<ModelOpen>)> = vec![
        ("the empty scope", vec![], vec![]),
        ("inside N", vec!["N"], vec![]),
        ("inside N.M", vec!["N", "M"], vec![]),
        ("open Alpha", vec![], vec![ModelOpen::Simple("Alpha", &[])]),
        (
            "open Alpha and Beta",
            vec![],
            vec![
                ModelOpen::Simple("Alpha", &[]),
                ModelOpen::Simple("Beta", &[]),
            ],
        ),
        (
            "inside N, open Alpha",
            vec!["N"],
            vec![ModelOpen::Simple("Alpha", &[])],
        ),
        (
            "open Alpha hiding x",
            vec![],
            vec![ModelOpen::Simple("Alpha", &["x"])],
        ),
        (
            "open Alpha (p)",
            vec![],
            vec![ModelOpen::Explicit("p", "Alpha.p")],
        ),
        (
            "open Alpha renaming secret -> r",
            vec![],
            vec![ModelOpen::Explicit("r", "Alpha.secret")],
        ),
        (
            "open Alpha (secret)",
            vec![],
            vec![ModelOpen::Explicit("secret", "Alpha.secret")],
        ),
    ];
    let names = [
        "x",
        "y",
        "secret",
        "Inner.x",
        "p",
        "p.q",
        "M.x",
        "_root_.x",
        "_root_.Inner.x",
        "absent",
        "Inner.absent",
    ];

    for (label, current, opens) in &scopes {
        // Build the same scope in the real resolver.
        let mut ns = Namespaces::new();
        for key in ENV {
            ns.record_declaration(key);
        }
        for key in PROTECTED {
            ns.record_protected(key);
        }
        for part in current {
            ns.enter(&Name::parse(part).expect("a namespace name"), 1);
        }
        let known = |k: &str| ENV.contains(&k);
        for open in opens {
            let clause = match open {
                ModelOpen::Simple(space, hidden) => OpenClause {
                    namespace: Name::parse(space).expect("a namespace"),
                    only: None,
                    hiding: hidden
                        .iter()
                        .map(|h| Name::parse(h).expect("a name"))
                        .collect(),
                    renaming: Vec::new(),
                },
                // `open Foo (x)` and `open Foo renaming a -> x` are the same Lean constructor, so
                // the model carries one shape and the surface's two forms both map onto it.
                ModelOpen::Explicit(local, target) => {
                    let target = Name::parse(target).expect("a name");
                    let space = Name::from_parts(target.enclosing()).expect("a namespace");
                    let member = Name::parse(target.short()).expect("the member");
                    let local = Name::parse(local).expect("the local spelling");
                    if member == local {
                        OpenClause {
                            namespace: space,
                            only: Some(vec![member]),
                            hiding: Vec::new(),
                            renaming: Vec::new(),
                        }
                    } else {
                        OpenClause {
                            namespace: space,
                            only: None,
                            hiding: Vec::new(),
                            renaming: vec![(member, local)],
                        }
                    }
                }
            };
            ns.open_clause(clause, &known)
                .unwrap_or_else(|e| panic!("{label}: {e:?}"));
        }

        for written in names {
            let expected = lean_candidates(ENV, PROTECTED, current, opens, written);
            let got = ns.resolve(&Name::parse(written).expect("a name"), &known);
            let ours: Vec<String> = match &got {
                Resolution::Found(key) => vec![key.clone()],
                Resolution::Unresolved(_) => Vec::new(),
                Resolution::Ambiguous { candidates, .. } => candidates.clone(),
            };
            let mut a = expected.clone();
            let mut b = ours.clone();
            a.sort();
            b.sort();
            assert_eq!(
                a, b,
                "`{written}` in [{label}]: Lean's own procedure gives {expected:?} and we gave \
                 {ours:?} ({got:?})"
            );
            // …and the VERDICT is derived the same way on both sides: one candidate resolves,
            // two is an ambiguity, none is unresolved.
            let verdict_matches = match (&got, expected.len()) {
                (Resolution::Found(_), 1) => true,
                (Resolution::Unresolved(_), 0) => true,
                (Resolution::Ambiguous { .. }, n) => n >= 2,
                _ => false,
            };
            assert!(
                verdict_matches,
                "`{written}` in [{label}]: Lean's procedure found {} candidate(s) {expected:?}, so \
                 the verdict must match, and ours was {got:?}",
                expected.len()
            );
        }
    }
}

/// ★ THE LOCK ON THE PRELUDE CACHE. `Repl::new` hands out a CLONE of one process-wide prelude
/// context instead of registering the standard library again each time. That is only sound because
/// `Context`'s global maps are copy-on-write behind `Arc`, so this test is the property that makes
/// the cache legitimate: two REPLs must share nothing a write can reach.
///
/// It is written as a grid over every mutating door the vernacular has — a definition, an
/// inductive with its whole derived family, a namespace, an export, a protected mark and a kernel
/// scope — because "they are independent" is only worth asserting against the writers that exist.
#[test]
fn two_repls_built_from_the_cached_prelude_are_independent() {
    let mut a = Repl::new();
    let mut b = Repl::new();

    // Both start from the same prelude, and it is actually THERE.
    for repl in [&a, &b] {
        assert!(
            repl.context().get_global("Nat").is_some(),
            "the prelude is loaded"
        );
        assert!(repl.context().get_global("Succ").is_some());
    }

    a.execute("Definition onlyInA : Nat := Zero.")
        .expect("a definition in A");
    a.execute("Inductive Hue := Red : Hue | Green : Hue.")
        .expect("an inductive in A");
    a.execute("Definition Space.member : Nat := Zero.")
        .expect("a namespace in A");
    a.execute("Protected Definition Space.hidden : Nat := Zero.")
        .expect("a protected mark in A");
    a.execute("Namespace Space.").expect("A enters a namespace");
    a.execute("Export Space (member).").expect("an export in A");
    a.execute("Section S.").expect("A opens a kernel scope");

    for absent in [
        "onlyInA",
        "Hue",
        "Red",
        "Hue_rec",
        "Space.member",
        "Space.hidden",
    ] {
        assert!(
            b.context().get_global(absent).is_none() && !b.context().is_inductive(absent),
            "`{absent}` was declared in A and must not be visible in B — a shared prelude must \
             share nothing a write can reach"
        );
    }
    assert_eq!(b.context().scope_depth(), 0, "A's open scope is not B's");
    assert!(
        b.execute("Definition onlyInA : Nat := (Succ Zero).")
            .is_ok(),
        "…and the key is genuinely free in B"
    );
    assert_eq!(
        b.execute("Eval onlyInA.").unwrap().trim(),
        "(Succ Zero)",
        "B's declaration is B's"
    );
    assert_eq!(
        a.execute("Eval _root_.onlyInA.").unwrap().trim(),
        "Zero",
        "…and A's is still A's, unchanged by anything B did"
    );

    // A THIRD REPL made after all that mutation still gets the pristine prelude, which is the
    // failure mode a cache actually has: the cached value drifting because somebody wrote to it.
    let c = Repl::new();
    for absent in ["onlyInA", "Hue", "Space.member"] {
        assert!(
            c.context().get_global(absent).is_none(),
            "a REPL built AFTER two others mutated theirs must still see the pristine prelude, \
             and `{absent}` leaked into it"
        );
    }
    assert!(
        c.context().get_global("Nat").is_some(),
        "…and the prelude is still all there"
    );
    assert_eq!(c.context().scope_depth(), 0);
}

/// `Private` and `Protected` are not one qualifier with two spellings, and corpus case T02-012's
/// guard is about exactly that confusion — "Catches assuming `protected` is inert. It is inert
/// *logically*, but it changes which short name resolves at a use site."
///
/// `private` IS inert here, and that is Lean's answer rather than a shrug: Lean mangles a private
/// name with the defining module's hash and then makes the mangling transparent to that same
/// module (`Lean/ResolveName.lean:107-110` and `:127-131` both fall back to the private form), so
/// within one module every spelling still reaches it. Measured on lean 4.14.0: `private def
/// M.hidden`, and then `M.hidden`, `M.user` and — under `open M` — the bare `hidden` all print 3.
/// A session is one module, so accepting it is the transcription that is CORRECT here.
#[test]
fn private_is_inert_in_one_module_and_protected_is_not() {
    let mut repl = Repl::new();
    repl.execute("Private Definition M.hidden : Nat := (Succ (Succ (Succ Zero))).")
        .expect("`Private` is accepted");
    repl.execute("Definition M.user : Nat := M.hidden.")
        .expect("a public user of it");
    repl.execute("Open M.").expect("M exists");

    assert_eq!(
        repl.execute("Eval M.hidden.").unwrap().trim(),
        "(Succ (Succ (Succ Zero)))"
    );
    assert_eq!(
        repl.execute("Eval M.user.").unwrap().trim(),
        "(Succ (Succ (Succ Zero)))"
    );
    assert_eq!(
        repl.execute("Eval hidden.").unwrap().trim(),
        "(Succ (Succ (Succ Zero)))",
        "the short name under `Open M.` still reaches it — `private` is transparent to its own \
         module, and lean 4.14.0 prints 3 for this exact program"
    );

    // ★ THE CONTRAST, which is the whole point. The same declaration written `Protected` stops
    // answering to its short name, and everything else about it is unchanged.
    let mut guarded = Repl::new();
    guarded
        .execute("Protected Definition M.hidden : Nat := (Succ (Succ (Succ Zero))).")
        .expect("`Protected` is accepted too");
    guarded.execute("Open M.").expect("M exists");
    assert!(
        guarded.execute("Check hidden.").is_err(),
        "`protected` is NOT inert — it is a resolution fact in every module, including this one"
    );
    assert_eq!(
        guarded.execute("Eval M.hidden.").unwrap().trim(),
        "(Succ (Succ (Succ Zero)))"
    );

    // ★ THE NEGATIVE. Visibility is exactly ONE qualifier: lean 4.14.0 refuses `private protected
    // def` in BOTH orders ("unexpected token 'protected'; expected … 'def' …"), because
    // `declModifiers` has one visibility field. Accepting the pair would be inventing a Lean a
    // reader does not have, and would leave two qualifiers whose interaction nothing defines.
    for src in [
        "Private Protected Definition M.hidden : Nat := Zero.",
        "Protected Private Definition M.hidden : Nat := Zero.",
    ] {
        let mut pair = Repl::new();
        let refused = pair.execute(src);
        assert!(
            refused.is_err(),
            "`{src}` writes two visibility qualifiers, which Lean has no field for: {refused:?}"
        );
        assert!(
            pair.context().get_global("M.hidden").is_none(),
            "…and a refused declaration leaves nothing behind"
        );
    }
}
