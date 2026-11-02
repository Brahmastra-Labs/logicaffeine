//! The vernacular surface, beyond what the corpus asks about.
//!
//! `literate_proof_soundness.rs` is the model this file follows: **every capability is asserted
//! together with the thing it must still REFUSE.** A positive-only test is satisfiable by a weak
//! implementation — `induction` that always succeeds, a `forall` that drops its binder, a document
//! door that reports success for the blocks it happened to reach — and this surface is the
//! soundness gate, so a weak implementation here is a false theorem everywhere.
//!
//! Four capabilities are locked, each with its negative:
//!
//! 1. **The document door.** `Repl::execute` is a SINGLE-command door and refuses a prefix;
//!    `Repl::execute_document` reads a whole document and is fail-early. The negative that matters
//!    is not "two blocks are both proved" — it is "a FALSE block anywhere is refused, and nothing
//!    after it is registered". Concatenation used to walk straight around the `concludes` gate:
//!    `5 < 2` by `lia` is refused alone and was accepted by silence when it followed a true
//!    theorem.
//! 2. **`forall` in a `Statement:`.** The binder must bind, the sort must be mandatory, and a
//!    FALSE body under a binder must be refused exactly as it is without one.
//! 3. **Literate `induction`.** Each bullet closes ITS OWN constructor's goal. The negatives are a
//!    wrong bullet count, a false motive, and — the one that catches a weak fix — a motive that
//!    holds for some constructors and not others.
//! 4. **One name grammar, both doors.** Every name the declaration door accepts is a name a term
//!    can spell; anything else is refused at the declaration rather than keyed at a string no
//!    `Check` could ever name.

use logicaffeine_kernel::interface::{InterfaceError, ParseError, Repl};
use logicaffeine_kernel::{double_check, infer_type, DoubleCheck};

// ============================================================================================
// HELPERS
// ============================================================================================

/// One command against a fresh context.
fn once(src: &str) -> Result<String, InterfaceError> {
    Repl::new().execute(src)
}

/// A whole document against a fresh context.
fn document(src: &str) -> Result<Vec<String>, InterfaceError> {
    Repl::new().execute_document(src)
}

/// Did the single-command door refuse this because something FOLLOWED the command?
fn refused_as_trailing_input(src: &str) -> bool {
    matches!(
        once(src),
        Err(InterfaceError::Parse(ParseError::TrailingInput { .. }))
    )
}

const TRUE_RING: &str =
    "## Theorem: TrueRing\n    Statement: 48 equals (mul 6 8).\n    Proof: ring.";
const TRUE_LIA: &str = "## Theorem: TrueLia\n    Statement: 2 < 5.\n    Proof: lia.";
const FALSE_LIA: &str = "## Theorem: FalseLia\n    Statement: 5 < 2.\n    Proof: lia.";

// ============================================================================================
// 1 · THE DOCUMENT DOOR — a prefix is never accepted, and a document is never half-proved
// ============================================================================================

/// `execute` reads ONE command and requires it to be the whole input. Every door leaks the same
/// way if it does not: the literate theorem parser stopped after the first `Proof:` line and
/// returned `Ok`, so the caller could not tell from the result that a block had vanished.
#[test]
fn the_single_command_door_refuses_anything_after_the_command() {
    for (what, src) in [
        (
            "two literate theorems",
            format!("{TRUE_RING}\n\n{TRUE_LIA}"),
        ),
        (
            "two Coq-style definitions",
            "Definition a : Nat := Zero.\n\nDefinition b : Nat := Zero.".to_string(),
        ),
        (
            "a theorem then a definition",
            format!("{TRUE_RING}\n\nDefinition a : Nat := Zero."),
        ),
        (
            "a definition then prose",
            "Definition a : Nat := Zero.\n\n/-- a note -/".to_string(),
        ),
        (
            "a theorem then junk",
            format!("{TRUE_RING}\n\nwhat is this"),
        ),
        (
            "a literate inductive then a theorem",
            format!("A Flag is either Up or Down.\n\n{TRUE_RING}"),
        ),
    ] {
        assert!(
            refused_as_trailing_input(&src),
            "the single-command door must refuse {what}, naming what followed: {:?}",
            once(&src)
        );
    }

    // …and the refusal must not over-reach. Whitespace, a trailing newline and a full stop are
    // punctuation, not a second command.
    for src in [
        "Definition a : Nat := Zero.",
        "Definition a : Nat := Zero.\n",
        "  Definition a : Nat := Zero.  \n\n  ",
        "## To double (n: Int) -> Int:\n    Yield (add n n).",
        "## To double (n: Int) -> Int:\n    Yield (add n n).\n",
        "A Flag is either Up or Down.\n",
    ] {
        assert!(
            once(src).is_ok(),
            "trailing whitespace and the sentence's own full stop are not input: {src:?} gave {:?}",
            once(src)
        );
    }
}

/// The other half of the door: a document is READ WHOLE. Every block is proved, and a caller that
/// counted its inputs gets one result per block.
#[test]
fn a_document_proves_every_block_it_was_handed() {
    let mut repl = Repl::new();
    let results = repl
        .execute_document(&format!("{TRUE_RING}\n\n{TRUE_LIA}"))
        .expect("a document of true theorems must be accepted");
    assert_eq!(
        results.len(),
        2,
        "one result per block, so nothing can vanish unnoticed"
    );
    for name in ["TrueRing", "TrueLia"] {
        assert!(
            repl.context().get_global(name).is_some(),
            "`{name}` must be registered — a document proves every block, not its first"
        );
    }

    // Prose between blocks is documentation, and documentation is not a block boundary problem.
    let mut annotated = Repl::new();
    annotated
        .execute_document(&format!(
            "/-- The first one. -/\n{TRUE_RING}\n-- and the second\n{TRUE_LIA}\n"
        ))
        .expect("prose between theorems must not disturb them");
    for name in ["TrueRing", "TrueLia"] {
        assert!(
            annotated.context().get_global(name).is_some(),
            "`{name}` must survive the prose around it"
        );
    }
}

/// ★ THE NEGATIVE THAT MATTERS. A false block ANYWHERE in a document is refused, and nothing
/// after it is registered — the run stops rather than reporting a success that covered fewer
/// blocks than it was handed.
#[test]
fn a_false_block_anywhere_in_a_document_is_refused_and_stops_the_run() {
    // The baseline each block meets alone: this is the gate concatenation must not walk around.
    assert!(
        once(TRUE_RING).is_ok(),
        "`48 = 6 * 8` by `ring` certifies alone"
    );
    assert!(
        once(FALSE_LIA).is_err(),
        "`5 < 2` by `lia` is refused alone"
    );

    for (position, src) in [
        ("first", format!("{FALSE_LIA}\n\n{TRUE_RING}\n\n{TRUE_LIA}")),
        (
            "middle",
            format!("{TRUE_RING}\n\n{FALSE_LIA}\n\n{TRUE_LIA}"),
        ),
        ("last", format!("{TRUE_RING}\n\n{TRUE_LIA}\n\n{FALSE_LIA}")),
    ] {
        assert!(
            document(&src).is_err(),
            "a document whose {position} block is FALSE must not report success: {:?}",
            document(&src)
        );
    }

    // And the run STOPS: a theorem after the refused one is never registered, so no declaration
    // rides in behind a failure.
    let mut repl = Repl::new();
    assert!(repl
        .execute_document(&format!("{TRUE_RING}\n\n{FALSE_LIA}\n\n{TRUE_LIA}"))
        .is_err());
    assert!(
        repl.context().get_global("TrueRing").is_some(),
        "the blocks before the failure ran"
    );
    assert!(
        repl.context().get_global("FalseLia").is_none(),
        "the false theorem itself is never registered"
    );
    assert!(
        repl.context().get_global("TrueLia").is_none(),
        "and nothing AFTER the failure is registered — a fail-early door does not keep going"
    );
}

/// A document is read whole or not at all: an unparseable block anywhere aborts the split before
/// a single command runs.
#[test]
fn an_unparseable_block_aborts_the_document_before_anything_runs() {
    let mut repl = Repl::new();
    let r = repl.execute_document(&format!("{TRUE_RING}\n\nInductive : Colour.\n\n{TRUE_LIA}"));
    assert!(
        r.is_err(),
        "a malformed block must refuse the document: {r:?}"
    );
    assert!(
        repl.context().get_global("TrueRing").is_none(),
        "the split runs before any command does, so a later parse error means NOTHING ran"
    );
}

// ============================================================================================
// 2 · PROSE — accepted beside a declaration, and never data
// ============================================================================================

#[test]
fn documentation_is_accepted_declares_nothing_and_cannot_swallow_a_command() {
    for src in [
        "/-- A documented definition. -/",
        "/-! A module-level doc section. -/",
        "/- an ordinary block comment -/",
        "/- a /- nested -/ block comment -/",
        "-- an ordinary line comment",
    ] {
        assert!(
            once(src).is_ok(),
            "prose must be accepted: {src:?} gave {:?}",
            once(src)
        );
    }

    // An UNTERMINATED block comment is an error. Swallowing the remainder would let one missing
    // `-/` eat a whole document's declarations in silence.
    assert!(
        once("/-- where is my terminator").is_err(),
        "an unterminated documentation block must be refused, not run to end of input"
    );

    // Prose declares nothing — there is no `Literal::Text` global hiding behind it.
    let mut repl = Repl::new();
    repl.execute("/-- A documented definition. -/").unwrap();
    for phantom in ["doc", "documented_doc", "A documented definition."] {
        assert!(
            repl.context().get_global(phantom).is_none(),
            "a docstring must never reach the namespace as data: `{phantom}`"
        );
    }

    // A comment does not consume the command beside it: in a document both are seen.
    let mut annotated = Repl::new();
    annotated
        .execute_document("/-- doc -/\nDefinition documented : Int -> Int := (fun n : Int => ((add n) 1)).\n-- a closing note\n")
        .expect("a documented declaration must still be made");
    assert_eq!(
        annotated.execute("Eval (documented 1).").ok(),
        Some("2".to_string()),
        "a documented declaration must translate exactly as an undocumented one does"
    );
}

// ============================================================================================
// 3 · `forall` IN A `Statement:` — the binder binds, the sort is mandatory, falsehood is refused
// ============================================================================================

#[test]
fn a_universally_quantified_statement_proves_exactly_what_its_unquantified_twin_does() {
    // The capability, in both spellings.
    for src in [
        "## Theorem: AddZeroAscii\n    Statement: forall n : Nat, (add n 0) equals n.\n    Proof: ring.",
        "## Theorem: AddZeroUnicode\n    Statement: \u{2200} n : Nat, (add n 0) equals n.\n    Proof: ring.",
        "## Theorem: AddCommTwoBinders\n    Statement: forall a : Nat, forall b : Nat, (add a b) equals (add b a).\n    Proof: ring.",
        "## Theorem: TwoLtFive\n    Statement: forall n : Nat, 2 < 5.\n    Proof: lia.",
    ] {
        assert!(once(src).is_ok(), "a quantified statement must prove: {src:?} gave {:?}", once(src));
    }

    // ★ THE NEGATIVE. A binder is not a licence: a FALSE body under a `forall` is refused exactly
    // as it is without one. A quantifier that made its body unreachable would pass a
    // positive-only test and certify anything.
    for src in [
        "## Theorem: AddOneIsIdentity\n    Statement: forall n : Nat, (add n 1) equals n.\n    Proof: ring.",
        "## Theorem: FiveLtTwoQuantified\n    Statement: forall n : Nat, 5 < 2.\n    Proof: lia.",
        "## Theorem: NIsZero\n    Statement: forall n : Nat, n equals 0.\n    Proof: ring.",
        "## Theorem: DifferentNames\n    Statement: forall n : Nat, n equals m.\n    Proof: ring.",
    ] {
        assert!(once(src).is_err(), "a false quantified statement must be refused: {src:?} gave {:?}", once(src));
    }
}

/// The boundary of the binder grammar. Each of these is one token away from the accepted form and
/// must be a PARSE error — not a silent reading of something else.
#[test]
fn a_malformed_binder_is_a_parse_error_rather_than_a_different_proposition() {
    for (what, src) in [
        (
            "no sort",
            "## Theorem: B1\n    Statement: forall n, (add n 0) equals n.\n    Proof: ring.",
        ),
        (
            "no colon",
            "## Theorem: B2\n    Statement: forall n Nat, (add n 0) equals n.\n    Proof: ring.",
        ),
        (
            "no comma",
            "## Theorem: B3\n    Statement: forall n : Nat (add n 0) equals n.\n    Proof: ring.",
        ),
        (
            "no body",
            "## Theorem: B4\n    Statement: forall n : Nat.\n    Proof: ring.",
        ),
        (
            "no binder name",
            "## Theorem: B5\n    Statement: forall : Nat, (add n 0) equals n.\n    Proof: ring.",
        ),
    ] {
        assert!(
            matches!(once(src), Err(InterfaceError::Parse(_))),
            "a `forall` with {what} must fail in the PARSER, where the mistake is legible: {:?}",
            once(src)
        );
    }

    // The existential is still refused. Dropping a binder is right for a prenex ∀ and catastrophic
    // for an ∃ — it turns it into an implicit ∀, an unsound strengthening that still typechecks.
    assert!(
        once("## Theorem: E\n    Statement: exists x : Nat, x > 0.\n    Proof: lia.").is_err(),
        "an existential must be refused rather than read as a statement about a free variable"
    );
}

// ============================================================================================
// 4 · LITERATE `induction` — one bullet per constructor, each closing ITS OWN goal
// ============================================================================================

/// The capability, over an inductive whose constructors are all non-recursive: `induction c`
/// binds the variable, takes the inductive type FROM THE BINDER, and hands bullet `i` the goal
/// the kernel derives for the `i`-th constructor.
#[test]
fn induction_closes_one_case_per_constructor_over_the_binders_own_type() {
    let mut repl = Repl::new();
    repl.execute("Inductive Colour := red : Colour | green : Colour | blue : Colour.")
        .expect("the inductive must declare");

    assert!(
        repl.execute(
            "## Theorem: ColourRefl\n    Statement: forall c : Colour, (Eq Colour c c).\n    \
             Proof: induction c.\n    - refl.\n    - refl.\n    - refl."
        )
        .is_ok(),
        "three bullets over a three-constructor inductive must certify — the type comes from the \
         binder, not from a hard-coded `Nat`"
    );

    // The proof object it stored is ORDINARY, and BOTH kernels agree about it. A derivation the
    // door accepted but the second kernel cannot re-check would be a certificate in name only.
    let stored = repl
        .context()
        .get_definition_body("ColourRefl")
        .cloned()
        .expect("an accepted theorem stores its derivation");
    infer_type(repl.context(), &stored).expect("the stored derivation must re-check");
    assert_eq!(
        double_check(repl.context(), &stored),
        DoubleCheck::Agreed,
        "the second kernel must agree about the derivation the elimination built"
    );

    // ★ THE NEGATIVES, one per thing a weak implementation would get away with.

    // (a) The bullet COUNT is checked against the real constructor list.
    for (name, what, proof) in [
        (
            "TooFew",
            "too few",
            "Proof: induction c.\n    - refl.\n    - refl.",
        ),
        (
            "TooMany",
            "too many",
            "Proof: induction c.\n    - refl.\n    - refl.\n    - refl.\n    - refl.",
        ),
        ("None", "no", "Proof: induction c."),
    ] {
        let src = format!(
            "## Theorem: {name}\n    Statement: forall c : Colour, (Eq Colour c c).\n    {proof}"
        );
        assert!(
            repl.execute(&src).is_err(),
            "{what} bullets must be refused: {:?}",
            repl.execute(&src)
        );
    }

    // (b) A FALSE motive is refused however many bullets it has.
    assert!(
        repl.execute(
            "## Theorem: ColourFalse\n    Statement: forall c : Colour, (Eq Int 1 2).\n    \
             Proof: induction c.\n    - refl.\n    - refl.\n    - refl."
        )
        .is_err(),
        "a false goal must be refused — the cases are checked, not counted"
    );

    // (c) ★ The one that catches a fix which hands every bullet the same goal: a motive that holds
    // for ONE constructor and fails for the others. If the bullets were all given the whole
    // statement, `c = red` would be neither provable nor refutable case-by-case and the shape of
    // the defect would be invisible.
    assert!(
        repl.execute(
            "## Theorem: ColourAllRed\n    Statement: forall c : Colour, (Eq Colour c red).\n    \
             Proof: induction c.\n    - refl.\n    - refl.\n    - refl."
        )
        .is_err(),
        "`c = red` is true in the `red` case and false in the other two — a per-case goal is the \
         only thing that can tell those apart"
    );

    // (d) The variable token is required even when the statement quantifies nothing.
    assert!(
        Repl::new()
            .execute("## Theorem: NoVar\n    Statement: x equals x.\n    Proof: induction.")
            .is_err(),
        "`induction` with no variable must not parse"
    );
}

// ============================================================================================
// 5 · ONE NAME GRAMMAR, BOTH DOORS
// ============================================================================================

/// Every name the declaration door accepts, a term can spell — and a name it cannot spell is
/// refused at the declaration rather than keyed at a string no `Check` could ever name.
#[test]
fn a_declarable_name_is_always_a_referenceable_one() {
    // A hierarchical name is one of them: the term door reads `Inner.value` as the one dotted
    // name the declaration door keys, so it declares, spells, and denotes what was declared.
    for name in [
        "h",
        "h'",
        "h''",
        "prime'd_name",
        "\u{3b1}",
        "\u{393}",
        "\u{2115}plus",
        "x\u{2081}",
        "\u{ab}a name with spaces\u{bb}",
        "\u{ab}weird?!name\u{bb}",
        "Inner.value",
    ] {
        let mut repl = Repl::new();
        repl.execute(&format!("Definition {name} : Nat := Zero."))
            .unwrap_or_else(|e| panic!("`{name}` must declare: {e:?}"));
        repl.execute(&format!("Check {name}.")).unwrap_or_else(|e| {
            panic!("`{name}` declares, so a term must be able to spell it: {e:?}")
        });
        assert_eq!(
            repl.execute(&format!("Eval {name}."))
                .unwrap_or_default()
                .trim(),
            "Zero",
            "`{name}` must denote what was declared, not merely parse"
        );
    }

    // ★ THE NEGATIVE. A name a term cannot spell is refused AT THE DECLARATION. These used to be
    // accepted as a raw text slice up to the first `:`, keying the context at a write-only name.
    for name in [
        "\u{ab}unclosed",
        "1abc",
        "a b",
        "Inner.\u{ab}a.b\u{bb}",
        "Inner.",
        ".value",
    ] {
        let mut repl = Repl::new();
        let declared = repl.execute(&format!("Definition {name} : Nat := Zero."));
        assert!(
            declared.is_err(),
            "`{name}` is not a name a term can spell, so the declaration door must refuse it: \
             {declared:?}"
        );
    }

    // Two names that differ only by a prime are two names.
    let mut repl = Repl::new();
    repl.execute("Definition h : Nat := Zero.").unwrap();
    repl.execute("Definition h' : Nat := (Succ Zero).").unwrap();
    assert_ne!(
        repl.execute("Eval h.").ok(),
        repl.execute("Eval h'.").ok(),
        "`h` and `h'` are different names and must hold different values"
    );
}

// ============================================================================================
// 6 · `deriving` — refused, never absorbed
// ============================================================================================

/// A `deriving` handler is a `CommandElabM` program and has nowhere to live in a pure-CoC kernel,
/// so the clause is refused. What must NOT happen is the old behaviour: the clause was absorbed
/// into the constructor's type, registering `red : ((Colour deriving) BEq)` — a declaration whose
/// type names two globals that are not in the context at all.
#[test]
fn a_deriving_clause_is_refused_and_leaves_nothing_behind() {
    for src in [
        "Inductive Colour := red : Colour deriving BEq.",
        "Inductive Colour := red : Colour | green : Colour deriving BEq, Repr.",
        "Inductive Colour deriving BEq := red : Colour.",
    ] {
        assert!(
            matches!(
                once(src),
                Err(InterfaceError::Parse(ParseError::Unsupported { .. }))
            ),
            "a `deriving` clause must be refused BY NAME: {src:?} gave {:?}",
            once(src)
        );

        let mut repl = Repl::new();
        let _ = repl.execute(src);
        for phantom in ["Colour", "red", "deriving", "BEq", "Colour_rec"] {
            assert!(
                repl.context().get_global(phantom).is_none(),
                "a refused `deriving` must leave NOTHING behind, and `{phantom}` appeared"
            );
        }
    }

    // The refusal must not over-reach: the same inductive without the clause declares, and gets
    // its dependent recursor.
    let mut repl = Repl::new();
    repl.execute("Inductive Colour := red : Colour | green : Colour.")
        .expect("the inductive itself is ordinary");
    assert!(
        repl.context().get_global("Colour_rec").is_some(),
        "declaring an inductive still derives its recursor"
    );
}

// ============================================================================================
// 7 · THE SURFACE — notation, fixity, macros, and the things they must still refuse
// ============================================================================================

/// A parser-extension command extends the way things are SPELLED. The negatives are the whole
/// value: a token means nothing before it is declared, a section-local notation dies with its
/// section, and a malformed declaration is refused rather than half-installed.
#[test]
fn a_notation_means_nothing_before_it_is_declared_and_nothing_after_its_section_closes() {
    let mut repl = Repl::new();
    repl.execute(
        "Definition op : Int -> Int -> Int := (fun a : Int => fun b : Int => ((add a) b)).",
    )
    .expect("the function must define");

    // Before the declaration the token is not lexable.
    assert!(
        repl.execute("Eval (3 \u{229e} 4).").is_err(),
        "an undeclared token must not parse — notation is declared, never guessed"
    );

    repl.execute("Section Scoped.")
        .expect("a section must open");
    repl.execute("Local Notation \"a \u{229e} b\" := ((op a) b).")
        .expect("a section-local notation must declare");
    assert_eq!(
        repl.execute("Eval (3 \u{229e} 4).").ok(),
        repl.execute("Eval ((op 3) 4).").ok(),
        "inside the section the token means `op`"
    );
    repl.execute("End Scoped.").expect("the section must close");

    // ★ THE NEGATIVE. The notation died with its section.
    assert!(
        repl.execute("Eval (3 \u{229e} 4).").is_err(),
        "a section-local notation must be gone once its section closes: {:?}",
        repl.execute("Eval (3 \u{229e} 4).")
    );

    // A section boundary is checked, not assumed.
    assert!(
        Repl::new().execute("End Nothing.").is_err(),
        "`End` with no open section must be refused"
    );
    let mut mismatched = Repl::new();
    mismatched.execute("Section Outer.").unwrap();
    assert!(
        mismatched.execute("End Inner.").is_err(),
        "`End` must name the section it closes — sections close innermost first"
    );
}

/// A malformed surface declaration is refused. Each of these is one field away from a valid
/// command and would otherwise install a notation that behaves in an unstated way.
#[test]
fn a_malformed_surface_declaration_is_refused_rather_than_half_installed() {
    for (what, src) in [
        ("an infix with no precedence", "Infixl \"\u{22c4}\" := f."),
        ("a fixity with no operator", "Infixl 65 := f."),
        ("a notation with no expansion", "Notation \"a \u{2295} b\"."),
        (
            "a notation with an unterminated pattern",
            "Notation \"a \u{2295} b := ((op a) b).",
        ),
        (
            "a pattern that is all holes",
            "Notation \"a b\" := ((op a) b).",
        ),
        (
            "a macro pattern beginning with a hole",
            "Macro \"$x bump\" := ((add $x) 1).",
        ),
        ("a `$` with no hole name", "Macro \"bump $\" := 1."),
    ] {
        assert!(
            once(src).is_err(),
            "{what} must be refused: {src:?} gave {:?}",
            once(src)
        );
    }
}

/// A macro whose recursion never bottoms out FAILS, at a stated limit. An expander that spins is
/// indistinguishable from a kernel that is thinking hard, and this surface is the one a user
/// drives interactively.
#[test]
fn a_non_terminating_macro_fails_loudly_instead_of_spinning() {
    let mut repl = Repl::new();
    repl.execute("MacroRules \"loop![$x]\" := loop![$x].")
        .expect("the rule itself declares — a macro's termination is not decidable at declaration");
    assert!(
        matches!(
            repl.execute("Eval loop![1]."),
            Err(InterfaceError::Parse(ParseError::MacroDepth { .. }))
        ),
        "a macro that never reaches a base case must be refused by the step budget: {:?}",
        repl.execute("Eval loop![1].")
    );
}

/// Hygiene has two directions, and the corpus only asks about one. A template binder must not
/// capture a call-site name (that is T09-003), AND the template must not be able to see a name the
/// argument binds.
#[test]
fn macro_hygiene_holds_in_both_directions() {
    let mut repl = Repl::new();
    repl.execute("Macro \"twice $e\" := (fun x : Int => ((add ((add $e) $e)) x)).")
        .expect("the macro must declare");

    // Direction 1: the template's `x` must not capture the call site's `x`.
    repl.execute("Definition captured : Int -> Int := (fun x : Int => ((twice x) 0)).")
        .expect("the definition must check");
    assert_eq!(
        repl.execute("Eval (captured 4).").ok(),
        Some("8".to_string()),
        "the macro's own binder must not capture the call site's `x` — an unhygienic expansion \
         answers 0 here, and answering 0 is not a type error"
    );

    // Direction 2: a name the ARGUMENT binds is not visible to the template. The template's `x` is
    // its own; substituting a lambda must leave that lambda's binder alone.
    repl.execute("Macro \"applyTo $f\" := ($f 5).")
        .expect("the second macro must declare");
    repl.execute("Definition shadowed : Int := (applyTo (fun x : Int => ((add x) 1))).")
        .expect("the definition must check");
    assert_eq!(
        repl.execute("Eval shadowed.").ok(),
        Some("6".to_string()),
        "the argument's own binder is its own: `(fun x => x + 1) 5` is 6 whatever the template \
         happens to call its variables"
    );
}

/// Declaring notation must not change the meaning of anything already written. The control is a
/// term with no notation in it at all, evaluated before and after four declarations.
#[test]
fn declaring_notation_leaves_every_existing_spelling_alone() {
    let control = "Eval ((add 2) ((mul 3) 4)).";
    let mut repl = Repl::new();
    let before = repl.execute(control).expect("the control term evaluates");

    repl.execute(
        "Definition op : Int -> Int -> Int := (fun a : Int => fun b : Int => ((add a) b)).",
    )
    .unwrap();
    repl.execute("Notation \"a \u{2295} b\" := ((op a) b).")
        .unwrap();
    repl.execute("Infixl 70 \"\u{22c4}\" := op.").unwrap();
    repl.execute("Prefix \"\u{223d}\" := Succ.").unwrap();
    repl.execute("MacroRules \"bump $x\" := ((add $x) 1).")
        .unwrap();

    assert_eq!(
        repl.execute(control).ok(),
        Some(before),
        "an extension point that changes what existing text means is not an extension point"
    );

    // And none of the four put anything in the flat namespace.
    for phantom in [
        "op\u{2295}",
        "\u{2295}",
        "\u{22c4}",
        "\u{223d}",
        "bump",
        "Notation",
        "Infixl",
        "Prefix",
        "MacroRules",
        "Section",
        "Local",
    ] {
        assert!(
            repl.context().get_global(phantom).is_none(),
            "a parser-extension command must declare NOTHING, and `{phantom}` appeared"
        );
    }
}

/// A string literal is DATA. A macro head inside one must be left exactly as written — rewriting
/// it would change a VALUE rather than a spelling, which is the one thing an expander may not do.
#[test]
fn a_macro_head_inside_a_string_literal_is_left_alone() {
    let mut repl = Repl::new();
    let plain = repl
        .execute("Eval \"bump me\".")
        .expect("a string literal evaluates to itself");
    repl.execute("MacroRules \"bump $x\" := ((add $x) 1).")
        .expect("the macro must declare");
    assert_eq!(
        repl.execute("Eval \"bump me\".").ok(),
        Some(plain),
        "declaring a macro must not rewrite the inside of a string literal"
    );
    // …and it still fires outside one.
    assert_eq!(
        repl.execute("Eval (bump 5).").ok(),
        repl.execute("Eval ((add 5) 1).").ok(),
        "the macro still expands where it is code"
    );
}

/// ★ A `forall` binder is an EIGENVARIABLE, and ∀-introduction is only sound when that constant is
/// FRESH. A binder that shadows a name the reflection layer interprets must not let the tactic
/// read the bound variable as that operator: `forall add : Nat, (add 1 2) equals 3` is a statement
/// about an opaque `Nat`, not about addition, and there is no such theorem.
#[test]
fn a_forall_binder_is_a_fresh_constant_not_the_operator_it_shadows() {
    for (what, src) in [
        ("add", "## Theorem: ShadowAdd\n    Statement: forall add : Nat, (add 1 2) equals 3.\n    Proof: ring."),
        ("mul", "## Theorem: ShadowMul\n    Statement: forall mul : Nat, (mul 2 3) equals 6.\n    Proof: ring."),
        ("sub", "## Theorem: ShadowSub\n    Statement: forall sub : Nat, (sub 5 1) equals 4.\n    Proof: simp."),
        ("lt", "## Theorem: ShadowLt\n    Statement: forall lt : Nat, (lt 2 5).\n    Proof: lia."),
    ] {
        assert!(
            once(src).is_err(),
            "a binder named `{what}` must be a fixed-but-arbitrary constant, never the operator \
             it shadows: {:?}",
            once(src)
        );
    }

    // The unshadowed statements those shadow ARE provable, so the refusals above are about the
    // binder and not about the arithmetic.
    for src in [
        "## Theorem: RealAdd\n    Statement: (add 1 2) equals 3.\n    Proof: ring.",
        "## Theorem: RealMul\n    Statement: (mul 2 3) equals 6.\n    Proof: ring.",
    ] {
        assert!(
            once(src).is_ok(),
            "the unshadowed arithmetic must still prove: {:?}",
            once(src)
        );
    }

    // …and a binder with an ordinary name is unaffected.
    assert!(
        once("## Theorem: Ordinary\n    Statement: forall n : Nat, (add n 0) equals n.\n    Proof: ring.")
            .is_ok(),
        "marking the eigenvariable must not disturb a binder that shadows nothing"
    );
}
