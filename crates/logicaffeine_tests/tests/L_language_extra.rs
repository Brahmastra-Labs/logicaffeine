//! Lane L_language — the tests that lock the ENGLISH SURFACE fixes shut.
//!
//! The corpus tier file (`lean_conformance_t12_vernacular.rs`) asserts one case each. This file
//! covers what those fixes TOUCH but the corpus never asks about: the neighbouring thing
//! each change must still refuse, the rung above and the rung below every computed shape,
//! and the differential between the two Surface A drivers (the string door
//! `compile_theorem` and the structured door `compile_theorem_for_ui`), which have no
//! reason to agree except that someone tested it.
//!
//! Three fixes are locked here:
//!
//! 1. **The math layer** (`parser/math.rs`) — a term grammar and a `For all` binder on the
//!    English theorem surface, so `∀ n, n + 0 = n` is statable and provable in English.
//! 2. **`ProofStrategy::ByRule`** — a one-word `Proof:` line is classified as naming a
//!    single proof step, and BOTH drivers have a real arm for it.
//! 3. **The proof SKETCH reading** — an English `Proof:` line names the method and the
//!    residual obligations are discharged by the same certified automation, reported.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile_theorem_for_ui;
use logicaffeine_language::ast::theorem::ProofStrategy;
use logicaffeine_language::ast::Stmt;
use logicaffeine_language::compile_theorem;
use logicaffeine_language::{
    analysis, mwe, Arena, AstContext, Interner, Lexer, Parser, WorldState,
};
use logicaffeine_proof::development::prove_development;

/// The kernel-certified marker. `"Proved"` is weaker — the uncertified arm returns `Ok` too.
const CERTIFIED: &str = "[kernel-verified]";

fn surface_a(src: &str) -> Result<String, String> {
    compile_theorem(src).map_err(|e| format!("{e:?}"))
}

fn certified(src: &str) -> bool {
    matches!(surface_a(src), Ok(s) if s.contains(CERTIFIED))
}

/// A one-line `Prove:`-only theorem block under `Proof: Auto.`
fn goal_only(name: &str, goal: &str) -> String {
    format!("## Theorem: {name}\nProve: {goal}\nProof: Auto.\n")
}

fn strategy_of(src: &str) -> ProofStrategy {
    let mut interner = Interner::new();
    let tokens = Lexer::new(src, &mut interner).tokenize();
    let trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &trie, &mut interner);
    let types = analysis::DiscoveryPass::new(&tokens, &mut interner).run();
    let (expr, term, np, sym, role, pp) = (
        Arena::new(),
        Arena::new(),
        Arena::new(),
        Arena::new(),
        Arena::new(),
        Arena::new(),
    );
    let ctx = AstContext::new(&expr, &term, &np, &sym, &role, &pp);
    let mut world = WorldState::new();
    let mut parser = Parser::new(tokens, &mut world, &mut interner, ctx, types);
    let statements = parser.parse_program().unwrap_or_else(|e| {
        panic!("the theorem block must parse before its strategy is read: {e:?}")
    });
    statements
        .iter()
        .find_map(|s| match s {
            Stmt::Theorem(t) => Some(t.strategy.clone()),
            _ => None,
        })
        .expect("the document declares a `## Theorem` block")
}

// ══════════════════ THE MATH LAYER — POSITIVES ══════════════════

/// The ring vocabulary generalises. One theorem proving is a special case; the whole
/// quantified-arithmetic fragment proving is the capability.
#[test]
fn the_english_math_layer_proves_the_ring_identities_lean_states_in_seven_characters() {
    for (name, english) in [
        ("add_zero", "For all n, n plus 0 equals n."),
        ("add_comm", "For all a b, a plus b equals b plus a."),
        ("mul_one", "For all n, n times 1 equals n."),
        ("mul_comm", "For all a b, a times b equals b times a."),
        ("sub_add", "For all n, n plus 1 minus 1 equals n."),
        (
            "distributivity",
            "For all a b c, a times (b plus c) equals a times b plus a times c.",
        ),
        (
            "assoc",
            "For all a b c, a plus b plus c equals a plus (b plus c).",
        ),
    ] {
        let block = goal_only(name, english);
        assert!(
            certified(&block),
            "`{english}` must state and PROVE on the English theorem surface — it is the \
             quantifier × function-term intersection the corpus reported empty: {:?}",
            surface_a(&block)
        );
    }
}

/// The English surface must be NO WEAKER than the FORMAL one on the same proposition. They
/// share `canonical_arithmetic_fn` and the uppercase/lowercase constant-vs-variable
/// convention on purpose, and the headline row is the case's whole content: `∀ n, n + 0 = n`
/// is the theorem the corpus reported unstatable on any vernacular surface.
///
/// One formal row only — `prove_certify_check` over this goal is orders of magnitude slower
/// through `prove_development` than through the English door (see the lane report), so a
/// seven-row cross-check would turn a millisecond test into a multi-minute one. The rest of
/// the fragment is locked on the English side, which is the side this lane owns.
#[test]
fn the_english_surface_is_never_weaker_than_the_formal_one() {
    let english = certified(&goal_only("Parity", "For all n, n plus 0 equals n."));
    let formal = prove_development("Theorem parity: prove for all n, Eq(Add(n, 0), n).")
        .expect("the formal development parses")
        .into_iter()
        .map(|(_, r)| r.verified)
        .next()
        .expect("one theorem");
    assert!(
        formal,
        "`∀ n, Eq(Add(n, 0), n)` must certify on the FORMAL surface — this is T12-011's own \
         refutation of the audit's central claim, and the English row below mirrors it"
    );
    assert!(
        english,
        "…and the ENGLISH surface must prove the same proposition. The English theorem \
         surface must never be the weaker of the two vernaculars: {:?}",
        surface_a(&goal_only("Parity", "For all n, n plus 0 equals n."))
    );
}

/// The NEGATIVES. A positive-only table is satisfiable by a prover that says yes to
/// everything; these are the neighbouring FALSE statements of the same shapes, and every one
/// of them must be refused.
#[test]
fn the_english_arithmetic_fragment_refuses_the_false_statements_of_the_same_shape() {
    for english in [
        "For all n, n plus 1 equals n.",
        "For all a b, a plus b times 2 equals (a plus b) times 2.",
        "For all a b c, a minus b minus c equals a minus (b minus c).",
        "For all a, a divided by 2 equals 2 divided by a.",
        "For all n, f(n) equals n.",
    ] {
        assert!(
            !certified(&goal_only("False", english)),
            "`{english}` is FALSE and must not certify — the surface is only worth having if \
             it refuses as reliably as it proves: {:?}",
            surface_a(&goal_only("False", english))
        );
    }
}

/// `zero` is the English name of `0`. The lexicon's own number table starts at `one`, so
/// without this the sentence the whole layer exists for — "adding zero" — would read `zero`
/// as a free VARIABLE and quietly fail to prove.
#[test]
fn the_english_names_of_small_numbers_are_numerals_in_a_math_term() {
    for english in [
        "For all n, n plus zero equals n.",
        "For all n, n times one equals n.",
        "For all n, n plus two minus two equals n.",
    ] {
        let block = goal_only("NumberWord", english);
        assert!(
            certified(&block),
            "`{english}` must prove — the English number word is the numeral: {:?}",
            surface_a(&block)
        );
    }
}

/// A function SYMBOL in term position — the other half of the intersection the corpus
/// reported empty. Asserted structurally on the rendered FOL (a function term either IS a
/// term on this surface or is not, independently of whether the prover can discharge it)
/// and then semantically where the ring's own symbols are involved.
#[test]
fn a_function_application_is_a_term_on_the_english_surface() {
    // The RENDERER capitalises a function symbol (`f` → `F`), so the shape is compared
    // case-insensitively: what is being asserted is that the application is a TERM, not how
    // the transpiler spells it.
    let applied = logicaffeine_language::compile("For all n, f(n) plus 0 equals f(n).")
        .expect("an application in term position compiles")
        .to_lowercase();
    assert!(
        applied.contains("add(f(n), 0)"),
        "a function APPLICATION must be a TERM on the English theorem surface — a quantifier \
         whose body applies a function symbol is precisely the intersection claimed empty: \
         {applied}"
    );

    let nested = logicaffeine_language::compile("For all n, f(g(n)) equals n.")
        .expect("a nested application compiles")
        .to_lowercase();
    assert!(
        nested.contains("f(g(n))"),
        "and the term grammar RECURSES through an application: {nested}"
    );

    // The ring's own symbols, applied explicitly, reach the same canonical `Add` the infix
    // spelling does — and PROVE.
    let explicit = goal_only("Explicit", "For all n, add(n, 0) equals n.");
    assert!(
        certified(&explicit),
        "`add(n, 0)` must canonicalise to the same `Add` that `n plus 0` builds: {:?}",
        surface_a(&explicit)
    );

    // An UNINTERPRETED symbol must prove nothing about its values.
    let false_about_f = goal_only("FalseAboutF", "For all n, f(n) equals n.");
    assert!(
        !certified(&false_about_f),
        "an uninterpreted function symbol must stay uninterpreted: {:?}",
        surface_a(&false_about_f)
    );
}

// ══════════════════ THE MATH LAYER — BOUNDARIES ══════════════════

/// Operator precedence, tested through the PROVER rather than through an AST dump: if
/// `times` did not bind tighter than `plus`, the first of each pair would be false and the
/// second true. One rung above and one rung below, for both precedence and associativity.
#[test]
fn math_term_precedence_and_associativity_are_the_arithmetic_ones() {
    for (must_prove, must_not) in [
        (
            "For all a b, a plus b times 2 equals a plus (b times 2).",
            "For all a b, a plus b times 2 equals (a plus b) times 2.",
        ),
        (
            "For all a b c, a minus b minus c equals (a minus b) minus c.",
            "For all a b c, a minus b minus c equals a minus (b minus c).",
        ),
        (
            "For all a b, a minus b plus b equals (a minus b) plus b.",
            "For all a b, a minus b plus b equals a minus (b plus b).",
        ),
    ] {
        let good = goal_only("Precedence", must_prove);
        let bad = goal_only("Precedence", must_not);
        assert!(
            certified(&good),
            "`{must_prove}` must prove — `times` binds tighter than `plus`, and `minus` is \
             LEFT-associative: {:?}",
            surface_a(&good)
        );
        assert!(
            !certified(&bad),
            "`{must_not}` must NOT prove — a term grammar that got precedence wrong would \
             certify this and the one above, and nothing downstream could tell: {:?}",
            surface_a(&bad)
        );
    }
}

/// A `For all a b,` prefix must bind BOTH variables. Asserted STRUCTURALLY, on the rendered
/// FOL, because the prover cannot tell you: a free variable in a goal is a metavariable the
/// search may instantiate, so `∀a. a + b = b + a` proves either way and a one-binder bug
/// would hide behind it.
#[test]
fn every_variable_in_a_binder_prefix_is_actually_bound() {
    let joint = logicaffeine_language::compile("For all a b, a plus b equals b plus a.")
        .expect("a two-binder math sentence compiles");
    assert_eq!(
        joint.matches('∀').count(),
        2,
        "a `For all a b,` prefix must bind BOTH variables, not just the first: {joint}"
    );

    let nested = logicaffeine_language::compile("For all a, For all b, a plus b equals b plus a.")
        .expect("a nested math sentence compiles");
    assert_eq!(
        nested.matches('∀').count(),
        2,
        "the nested spelling must build the same two binders: {nested}"
    );
    assert_eq!(
        joint, nested,
        "`For all a b,` and `For all a, For all b,` are the same proposition and must render \
         identically"
    );

    let both = goal_only("Both", "For all a b, a plus b equals b plus a.");
    assert!(
        certified(&both),
        "and it must prove: {:?}",
        surface_a(&both)
    );

    // A variable OUTSIDE the prefix stays free: `∀a∀b. a + b = b + c` is not
    // commutativity, and must not close.
    let free = goal_only("Free", "For all a b, a plus b equals b plus c.");
    assert!(
        !certified(&free),
        "a variable outside the binder prefix must not be quantified silently — the goal is a \
         different proposition: {:?}",
        surface_a(&free)
    );

    // And the case convention is shared with the formal surface: an uppercase-leading name
    // in the body is a CONSTANT, never one of the binders. Asserted on the rendering, which
    // is where the distinction lives.
    let constant = logicaffeine_language::compile("For all a b, a plus b equals b plus C.")
        .expect("an uppercase name in a math term compiles");
    assert_eq!(
        constant.matches('∀').count(),
        2,
        "`C` must NOT have become a third binder — an uppercase-leading name is a constant \
         (`formula::ident_to_term`'s rule, shared): {constant}"
    );
    assert!(
        constant.contains("Add(b, C)"),
        "and it must survive as the constant it is: {constant}"
    );
}

/// A SORT is not representable on this surface — neither `LogicExpr::Quantifier` nor
/// `ProofExpr::ForAll` carries one. Accepting `: Nat` and dropping it would attest a
/// proposition about `Nat` while proving the ring one, which is exactly the silent-success
/// trap the tier file guards elsewhere. Refusing is the loud answer.
#[test]
fn a_sorted_binder_is_refused_rather_than_silently_unsorted() {
    let sorted = goal_only("Sorted", "For all n : Nat, n plus 0 equals n.");
    assert!(
        surface_a(&sorted).is_err(),
        "`For all n : Nat,` must not be accepted with the sort dropped — the surface carries \
         no sort and must say so: {:?}",
        surface_a(&sorted)
    );
    // And the unsorted spelling, which IS representable, proves.
    assert!(
        certified(&goal_only("Unsorted", "For all n, n plus 0 equals n.")),
        "the spelling the surface CAN represent must still work"
    );
}

/// The math production is strictly ADDITIVE: it rewinds unless it consumes the whole
/// sentence, so no English sentence changes meaning. These are the sentences nearest the
/// new grammar's vocabulary.
#[test]
fn the_math_layer_never_steals_an_english_sentence() {
    let syllogism = "## Theorem: Untouched\n\
                     Given: All men are mortal.\n\
                     Given: Socrates is a man.\n\
                     Prove: Socrates is mortal.\n\
                     Proof: Auto.\n";
    assert!(
        certified(syllogism),
        "the canonical syllogism must be unaffected by the math layer: {:?}",
        surface_a(syllogism)
    );

    // A sentence carrying the math layer's OWN relation word but no arithmetic and no
    // binder is left to the English parser — the pre-scan requires both.
    let fol_premise = "## Theorem: StillEnglish\n\
                       Given: forall x, Man(x) -> Mortal(x).\n\
                       Prove: Socrates is mortal.\n\
                       Proof: Auto.\n";
    assert!(
        surface_a(fol_premise).is_err(),
        "formal FOL in a `Given:` line must still be refused — the math layer parses TERMS, \
         not formulas, and must not have made this look writable: {:?}",
        surface_a(fol_premise)
    );

    // Ordinary compilation of English is untouched.
    let every = logicaffeine_language::compile("Every man is mortal.")
        .expect("ordinary English still compiles");
    assert!(
        every.contains('∀') || every.to_lowercase().contains("forall"),
        "a categorical must still be a categorical: {every}"
    );
}

/// A math sentence is a legal PREMISE, not only a goal — the two lines go through the same
/// sentence parser and must not come apart.
#[test]
fn a_math_sentence_is_a_premise_as_well_as_a_goal() {
    let with_premise = "## Theorem: FromPremise\n\
                        Given: For all n, f(n) plus 0 equals f(n).\n\
                        Prove: For all n, f(n) plus 0 equals f(n).\n\
                        Proof: Auto.\n";
    assert!(
        certified(with_premise),
        "a quantified arithmetic PREMISE must reach the prover — the `Given:` and `Prove:` \
         lines go through the SAME sentence parser and must not come apart: {:?}",
        surface_a(with_premise)
    );
}

/// Robustness to the point of absurdity: the term grammar is recursive and the binder
/// prefix is unbounded, so both are exercised past anything a human would write. A
/// grammar that quietly caps out is a grammar that will cap out in front of an audience.
#[test]
fn the_math_grammar_survives_depth_width_and_a_rewind_in_the_middle_of_a_document() {
    // Six binders, each one used in the body. The body is the same sum on both sides, so
    // this is a GRAMMAR test closed by reflexivity rather than a search benchmark; the
    // binder COUNT is checked structurally next to it.
    let six = "For all a b c d e f, a plus b plus c plus d plus e plus f equals \
               a plus b plus c plus d plus e plus f.";
    let wide = goal_only("Wide", six);
    assert!(
        certified(&wide),
        "a six-variable binder prefix must parse and close: {:?}",
        surface_a(&wide)
    );
    let rendered = logicaffeine_language::compile(six).expect("six binders compile");
    assert_eq!(
        rendered.matches('∀').count(),
        6,
        "all six variables must be BOUND, not just the first: {rendered}"
    );

    // Eight levels of parentheses around one variable.
    let deep = goal_only("Deep", "For all n, ((((((((n)))))))) plus 0 equals n.");
    assert!(
        certified(&deep),
        "a deeply parenthesised term is still the term it wraps: {:?}",
        surface_a(&deep)
    );

    // A twenty-term sum, each side identical — pure grammar, closed by reflexivity.
    let terms = ('a'..='t').map(|c| c.to_string()).collect::<Vec<_>>();
    let binders = terms.join(" ");
    let sum = terms.join(" plus ");
    let wide_sum = goal_only(
        "WideSum",
        &format!("For all {binders}, {sum} equals {sum}."),
    );
    assert!(
        certified(&wide_sum),
        "a twenty-term sum under twenty binders must parse and close: {:?}",
        surface_a(&wide_sum)
    );

    // A REWIND in the middle of a block: an English premise, a math goal. The math
    // production rewinds on the premise and must leave the parser exactly as it found it.
    let mixed = "## Theorem: Mixed\n\
                 Given: All men are mortal.\n\
                 Given: Socrates is a man.\n\
                 Prove: For all n, n plus 0 equals n.\n\
                 Proof: Auto.\n";
    assert!(
        certified(mixed),
        "a math goal after English premises must prove — the rewind must not disturb the \
         premises it declined: {:?}",
        surface_a(mixed)
    );

    // …and the reverse: a math premise followed by an English goal the premises entail.
    let reverse = "## Theorem: Reverse\n\
                   Given: For all n, n plus 0 equals n.\n\
                   Given: All men are mortal.\n\
                   Given: Socrates is a man.\n\
                   Prove: Socrates is mortal.\n\
                   Proof: Auto.\n";
    assert!(
        certified(reverse),
        "an English syllogism must still close with a math premise sitting beside it: {:?}",
        surface_a(reverse)
    );
}

/// Two residues of the math layer, each refused BY NAME rather than handed to the English
/// parser (which reports a missing content word and never mentions the construct that broke).
/// A limit a user can read is a work queue; a limit that reports the wrong thing is a bug
/// report nobody can act on.
#[test]
fn an_unsupported_math_relation_is_refused_by_name_and_english_is_left_alone() {
    for order in [
        "For all n, n plus 1 is greater than n.",
        "For all n, n plus 1 is less than n.",
    ] {
        let err = surface_a(&goal_only("Order", order))
            .expect_err("an order between arithmetic TERMS has no production here yet");
        assert!(
            err.contains("equals"),
            "`{order}` must be refused by naming the relation this surface DOES have, not by \
             reporting a missing content word: {err}"
        );
    }

    // The refusal must not reach an English sentence that merely uses the same comparative.
    // No arithmetic operator ⇒ no math production ⇒ English keeps the sentence, whatever it
    // makes of it.
    for english in [
        "For all cats, the score is greater than five.",
        "The score is greater than five.",
        "Simon is taller than Mary.",
    ] {
        if let Err(e) = logicaffeine_language::compile(english) {
            let rendered = format!("{e:?}");
            assert!(
                !rendered.contains("relates TERMS"),
                "`{english}` carries no arithmetic, so the math layer must never have looked \
                 at it: {rendered}"
            );
        }
    }
}

/// The EXISTENTIAL half of the binder. The corpus's claim was about EXPRESSIBILITY, and a
/// surface that can say `∀ n, n + 0 = n` but not `∃ n, n + 1 = 2` has closed half the gap.
#[test]
fn the_math_binder_carries_the_existential_quantifier_too() {
    let exists = logicaffeine_language::compile("For some n, n plus 1 equals 2.")
        .expect("`For some n,` compiles");
    assert!(
        exists.contains('∃') || exists.to_lowercase().contains("exists"),
        "`For some n,` must build an EXISTENTIAL binder over the term equation: {exists}"
    );
    assert!(
        exists.contains("Add(n, 1)"),
        "and its body is still a term equation: {exists}"
    );

    let mixed = logicaffeine_language::compile("For all a, For some b, a plus b equals a.")
        .expect("a mixed prefix compiles");
    assert_eq!(
        mixed.matches('∀').count(),
        1,
        "a mixed prefix keeps each quantifier as written: {mixed}"
    );
    assert!(
        mixed.contains('∃') || mixed.to_lowercase().contains("exists"),
        "…including the existential one: {mixed}"
    );
}

// ══════════════════ ProofStrategy::ByRule ══════════════════

/// The classification is by SHAPE — one word is one step, more than one word is a script —
/// so it can never drift against the inference-rule enum or the tactic table.
#[test]
fn a_one_word_proof_line_is_a_rule_and_a_many_word_one_is_a_script() {
    assert_eq!(
        strategy_of("## Theorem: T\nProve: Bob is happy.\nProof: ModusPonens.\n"),
        ProofStrategy::ByRule("ModusPonens".to_string()),
        "a one-word `Proof:` line names a single step"
    );
    assert_eq!(
        strategy_of("## Theorem: T\nProve: Bob is happy.\nProof: Assumption.\n"),
        ProofStrategy::ByRule("Assumption".to_string()),
        "the shape decides, not whether the word happens to be a live tactic"
    );
    assert_eq!(
        strategy_of("## Theorem: T\nProve: Bob is happy.\nProof: Induction on n.\n"),
        ProofStrategy::Script("Induction on n .".to_string()),
        "more than one word is a SCRIPT — `Induction on n.` must not be reclassified"
    );
    assert_eq!(
        strategy_of("## Theorem: T\nProve: Bob is happy.\nProof: Assume h. By assumption.\n"),
        ProofStrategy::Script("Assume h . By assumption .".to_string()),
        "a multi-step script is a SCRIPT"
    );
    assert_eq!(
        strategy_of("## Theorem: T\nProve: Bob is happy.\nProof: Auto.\n"),
        ProofStrategy::Auto,
        "`Auto` is still `Auto` — the rule classification sits BELOW it"
    );
}

/// `ByRule` is not a dead variant with a live parser: both drivers run it, and a one-word
/// line that names a real proof step proves the theorem.
#[test]
fn a_one_word_proof_line_that_names_a_real_step_proves_the_theorem() {
    let by_assumption = "## Theorem: ByAssumption\n\
                         Given: Bob is happy.\n\
                         Prove: Bob is happy.\n\
                         Proof: Assumption.\n";
    assert!(
        certified(by_assumption),
        "`Proof: Assumption.` reaches `ByRule` and must still PROVE — a classification that \
         broke the proof would be worse than the dead variant: {:?}",
        surface_a(by_assumption)
    );
    assert!(
        compile_theorem_for_ui(by_assumption).verified,
        "and the structured door must agree — a `_ => None` arm there reported a `ByRule` \
         theorem as unproved with error: None"
    );
}

/// The diagnostic TEACHES. An inference-rule name is not a tactic, and the message says
/// which namespace the word belongs to instead of leaving the author to guess.
#[test]
fn naming_an_inference_rule_reports_the_namespace_it_belongs_to() {
    let by_rule = "## Theorem: ByModusPonens\n\
                   Given: If Bob is happy then Bob is sad.\n\
                   Given: Bob is happy.\n\
                   Prove: Bob is sad.\n\
                   Proof: ModusPonens.\n";
    let err = surface_a(by_rule).expect_err("a rule name is not a proof step");
    assert!(
        err.contains("unknown tactic"),
        "the failure must stay LOUD and keep naming the category: {err}"
    );
    assert!(
        err.contains("INFERENCE RULE"),
        "and it must say what the word actually is, so the author is not left reading \
         `unknown tactic` about a rule that plainly exists: {err}"
    );

    // Both doors report the same refusal — the structured one must not quietly return
    // `verified: false, error: None`.
    let ui = compile_theorem_for_ui(by_rule);
    assert!(!ui.verified, "the structured door must refuse it too");
    assert!(
        ui.verification_error
            .as_deref()
            .is_some_and(|e| e.contains("unknown tactic")),
        "and it must carry the same reason, not silence: {:?}",
        ui.verification_error
    );

    // The theorem itself is provable — the failure is the SPELLING, not the prover.
    assert!(
        certified(&by_rule.replace("Proof: ModusPonens.", "Proof: Auto.")),
        "the same theorem under `Auto` must certify"
    );
}

// ══════════════════ THE PROOF SKETCH READING ══════════════════

/// An English `Proof:` line names the METHOD; the routine obligations it leaves are
/// discharged by the same certified automation, and the answer SAYS how many.
#[test]
fn a_proof_sketch_discharges_its_residual_obligations_and_reports_them() {
    let sketch = "## Theorem: Sketch\n\
                  Given: Bob is happy.\n\
                  Prove: If Bob is sad then Bob is happy.\n\
                  Proof: Assume h.\n";
    let out = surface_a(sketch).expect("the sketch proves");
    assert!(
        out.contains(CERTIFIED),
        "a sketch that names the method must still reach a KERNEL CERTIFICATE: {out}"
    );
    assert!(
        out.contains("1 residual obligation discharged by automation"),
        "and it must SAY that the author left an obligation to the machine — an implicit \
         close that does not report itself is the dishonest version of this feature: {out}"
    );

    // A COMPLETE script says nothing, so a full tactic proof reads exactly as before.
    let complete = "## Theorem: Complete\n\
                    Given: Bob is happy.\n\
                    Prove: If Bob is sad then Bob is happy.\n\
                    Proof: Assume h. By assumption.\n";
    let complete_out = surface_a(complete).expect("the complete script proves");
    assert!(
        complete_out.contains(CERTIFIED),
        "the complete script still certifies"
    );
    assert!(
        !complete_out.contains("residual obligation"),
        "a complete proof must carry no residue note: {complete_out}"
    );
}

/// The sweep is not a licence. A step that ERRORS is never swept, and a residue the
/// automation cannot close still fails loudly — those are the two ways an implicit
/// goal-closing pass turns into a proof of something nobody proved.
#[test]
fn a_proof_sketch_never_rescues_a_broken_step_or_an_unreachable_goal() {
    // (1) A step that fails is reported as itself, not proved around.
    let bad_step = "## Theorem: BadStep\n\
                    Given: Bob is happy.\n\
                    Prove: Bob is happy.\n\
                    Proof: Exact Or.\n";
    assert!(
        surface_a(bad_step).is_err_and(|e| e.contains("NoSuchHypothesis")),
        "a failing step must surface as that step's failure — the sweep runs only after a \
         clean script: {:?}",
        surface_a(bad_step)
    );

    // (2) A residue the automation cannot discharge stays a failure, and says so.
    let unreachable = "## Theorem: Unreachable\n\
                       Given: Bob is happy.\n\
                       Prove: If Bob is happy then Bob is sad.\n\
                       Proof: Assume h.\n";
    let err = surface_a(unreachable).expect_err("an unprovable residue must not certify");
    assert!(
        err.contains("obligation"),
        "the failure must name the obligations the sketch left open: {err}"
    );

    // (3) And the structured door agrees on both.
    assert!(!compile_theorem_for_ui(bad_step).verified);
    assert!(!compile_theorem_for_ui(unreachable).verified);
}

/// THE DIFFERENTIAL. The string door and the structured door are two independent
/// implementations of the same surface; every shape this lane touched must get the same
/// verdict from both.
#[test]
fn both_surface_a_doors_reach_the_same_verdict_on_every_shape_this_lane_touched() {
    for (label, src) in [
        ("math goal", goal_only("MathGoal", "For all n, n plus 0 equals n.")),
        ("false math goal", goal_only("FalseMath", "For all n, n plus 1 equals n.")),
        (
            "math premise",
            "## Theorem: MathPremise\nGiven: For all n, f(n) plus 0 equals f(n).\nProve: For all n, f(n) plus 0 equals f(n).\nProof: Auto.\n"
                .to_string(),
        ),
        (
            "by-rule that is a step",
            "## Theorem: R1\nGiven: Bob is happy.\nProve: Bob is happy.\nProof: Assumption.\n"
                .to_string(),
        ),
        (
            "by-rule that is a rule name",
            "## Theorem: R2\nGiven: Bob is happy.\nProve: Bob is happy.\nProof: ModusPonens.\n"
                .to_string(),
        ),
        (
            "sketch with residue",
            "## Theorem: R3\nGiven: Bob is happy.\nProve: If Bob is sad then Bob is happy.\nProof: Assume h.\n"
                .to_string(),
        ),
        (
            "complete script",
            "## Theorem: R4\nGiven: Bob is happy.\nProve: If Bob is sad then Bob is happy.\nProof: Assume h. By assumption.\n"
                .to_string(),
        ),
    ] {
        let string_door = certified(&src);
        let structured_door = compile_theorem_for_ui(&src).verified;
        assert_eq!(
            string_door, structured_door,
            "`{label}` must get ONE answer from Surface A, not one per door: string={string_door} \
             structured={structured_door} — {:?}",
            surface_a(&src)
        );
    }
}

// ══════════════ EVERY DOOR PROVES BY THE DECLARED METHOD ══════════════

/// A NEW defect, not in the corpus. `verify_theorem`, `prove_theorem_trace` and
/// `theorem_certificate` ran automatic proof search unconditionally and never read the
/// `Proof:` line at all. So a theorem whose declared proof is a BROKEN script still got a
/// kernel term — and `theorem_certificate` still minted a portable CERTIFICATE — while
/// `compile_theorem` failed loudly on the very same input. Two doors, opposite verdicts,
/// and nothing in the suite between them.
///
/// `Proof: Exact Or.` is the witness: the script looks up a hypothesis named `Or`, which
/// does not exist, so there IS no proof by the declared method — but the theorem is
/// trivially provable by search, which is what made the divergence silent.
#[test]
fn a_certificate_is_never_minted_for_a_proof_the_author_did_not_write() {
    let broken = "## Theorem: Broken_Script\n\
                  Given: Bob is happy.\n\
                  Prove: Bob is happy.\n\
                  Proof: Exact Or.\n";

    // The string door has always been honest about this one.
    assert!(
        surface_a(broken).is_err_and(|e| e.contains("NoSuchHypothesis")),
        "the string door must refuse a broken script: {:?}",
        surface_a(broken)
    );

    assert!(
        logicaffeine_compile::verify_theorem(broken).is_err(),
        "`verify_theorem` must NOT hand back a kernel term for a proof the declared script \
         never found — running automatic search instead of the declared method is how a door \
         certifies something nobody wrote"
    );

    let trace = logicaffeine_compile::prove_theorem_trace(broken)
        .expect("the block parses, so a trace is returned");
    assert!(
        !trace.verified,
        "`prove_theorem_trace` must report the DECLARED proof's verdict: {:?}",
        trace.error
    );

    assert!(
        logicaffeine_compile::theorem_certificate(broken)
            .expect("the block parses")
            .is_none(),
        "no portable CERTIFICATE may be minted for a theorem whose declared proof failed — a \
         certificate is the artifact a third party re-checks, and it must attest the proof \
         that was written"
    );

    // …and the same theorem under `Auto` still certifies through every door, so the refusal
    // above is about the METHOD and not about the theorem.
    let auto = broken.replace("Proof: Exact Or.", "Proof: Auto.");
    assert!(
        certified(&auto),
        "the same theorem under `Auto` must certify"
    );
    assert!(logicaffeine_compile::verify_theorem(&auto).is_ok());
    assert!(logicaffeine_compile::theorem_certificate(&auto)
        .expect("parses")
        .is_some());
}

/// The four kernel-certified Surface A doors must agree on EVERY shape — that is the
/// property, not any one of them being right.
#[test]
fn all_four_kernel_doors_agree_about_what_was_proved() {
    for (label, src) in [
        (
            "auto",
            "## Theorem: D1\nGiven: Bob is happy.\nProve: Bob is happy.\nProof: Auto.\n".to_string(),
        ),
        (
            "complete script",
            "## Theorem: D2\nGiven: Bob is happy.\nProve: If Bob is sad then Bob is happy.\nProof: Assume h. By assumption.\n"
                .to_string(),
        ),
        (
            "broken script",
            "## Theorem: D3\nGiven: Bob is happy.\nProve: Bob is happy.\nProof: Exact Or.\n"
                .to_string(),
        ),
        (
            "rule name, not a step",
            "## Theorem: D4\nGiven: Bob is happy.\nProve: Bob is happy.\nProof: ModusPonens.\n"
                .to_string(),
        ),
        (
            "math goal",
            goal_only("D5", "For all n, n plus 0 equals n."),
        ),
        (
            "false math goal",
            goal_only("D6", "For all n, n plus 1 equals n."),
        ),
    ] {
        let string_door = certified(&src);
        let structured_door = compile_theorem_for_ui(&src).verified;
        let term_door = logicaffeine_compile::verify_theorem(&src).is_ok();
        let certificate_door = logicaffeine_compile::theorem_certificate(&src)
            .map(|c| c.is_some())
            .unwrap_or(false);
        assert_eq!(
            [string_door, structured_door, term_door, certificate_door],
            [string_door; 4],
            "`{label}` must get ONE answer from Surface A: string={string_door} \
             structured={structured_door} term={term_door} certificate={certificate_door} — {:?}",
            surface_a(&src)
        );
    }
}

// ══════════════ WAVE 2 — THE TWO NEW TERM DEFECTS, AT THE KERNEL DOOR ══════════════

/// Two NEW defects, neither in the corpus, and they are the same failure in different
/// clothes: a TERM that is not the one the sentence wrote. Both came back
/// `[kernel-verified]` on a FALSE statement, which is the exact shape this campaign was
/// convened to hunt — the kernel was never wrong, it was lied to.
///
/// 1. **The binder shadowed by the numeral table.** `For all one, one times one equals one.`
///    bound `one` and then read the body's occurrences of it as the literal `1`, so
///    `∀one(Mul(1, 1) = 1)` reached the kernel, certified, and stood in for `∀x(x × x = x)`.
/// 2. **Two numbers that are one `f64`.** A numeral too large for an `i64` was widened to a
///    float and rendered back through `f64::to_string`, so `99999999999999999999` and
///    `100000000000000000000` arrived as the same term and `n + a = n + b` certified.
///
/// The positives sit next to the negatives on purpose: a blanket refusal of either shape
/// would satisfy a negatives-only test, and would be its own regression.
#[test]
fn the_two_new_term_defects_no_longer_certify_a_false_statement() {
    for shadowed in [
        "For all one, one times one equals one.",
        "For all zero, zero plus zero equals zero.",
        "For all two, two plus two equals two.",
    ] {
        let block = goal_only("Shadowed", shadowed);
        assert!(
            !certified(&block),
            "`{shadowed}` is FALSE — its binder binds a real variable and the sentence says \
             so. Before the fix the body had become closed arithmetic over the numeral the \
             binder's own name happened to spell, and it came back kernel-certified: {:?}",
            surface_a(&block)
        );
    }

    for collision in [
        "For all n, n plus 99999999999999999999 equals n plus 100000000000000000000.",
        "For all n, n plus 0.1 equals n plus 0.10000000000000000001.",
    ] {
        let block = goal_only("Collision", collision);
        assert!(
            !certified(&block),
            "`{collision}` is FALSE — two numerals written differently are two different \
             terms, and a surface that silently renumbers one of them proves whatever the \
             other says: {:?}",
            surface_a(&block)
        );
    }

    // …and the TRUE statements of both shapes still prove, so neither fix is a blanket
    // refusal of the shape it repairs.
    for still_true in [
        "For all one, one plus 0 equals one.",
        "For all zero, zero plus 0 equals zero.",
    ] {
        let block = goal_only("StillTrue", still_true);
        assert!(
            certified(&block),
            "`{still_true}` is the ring identity under a binder that happens to be named for \
             a number, and it must still certify: {:?}",
            surface_a(&block)
        );
    }

    // THE DIFFERENTIAL. Both Surface A doors must reach the same verdict on every one of
    // these, because they are two independent implementations of one surface.
    for src in [
        goal_only("D1", "For all one, one times one equals one."),
        goal_only("D2", "For all one, one plus 0 equals one."),
        goal_only(
            "D3",
            "For all n, n plus 99999999999999999999 equals n plus 100000000000000000000.",
        ),
    ] {
        assert_eq!(
            certified(&src),
            compile_theorem_for_ui(&src).verified,
            "the string door and the structured door must agree: {:?}",
            surface_a(&src)
        );
    }
}

/// T12-005's LANGUAGE side, end to end, on a goal whose ∀ prefix actually binds the variable.
///
/// `Proof: Induction on n.` reaches `ProofStrategy::Script("Induction on n .")` — T12-005's
/// own first assertion pins that — `run_script` canonicalises `Induction` → `induction`,
/// skips `on` as filler and calls `ProofState::induction_on("n")`, and
/// `compile::run_proof_sketch` sweeps whatever the two induction subgoals leave open with the
/// same certified automation `Proof: Auto.` uses. Every link of that chain is exercised here,
/// and the corpus case's own `@evidence` for T12-005 — *"`induction` is a ZERO-ARGUMENT
/// primitive, so `on` and `n` are left over and the run dies on trailing tokens"* — is STALE
/// against today's source: the documented spelling RUNS.
///
/// What is not exercised here is `induction_on` over a variable the goal's ∀ prefix does not
/// bind at all. That is the one thing still red in T12-005 and it lives in
/// `logicaffeine_proof::tactic`; this lane owns no file there. See the lane report.
#[test]
fn the_documented_induction_spelling_runs_on_a_goal_that_binds_its_variable() {
    let inductive = "## Theorem: Induct\n\
                     Prove: For all n, n plus 0 equals n.\n\
                     Proof: Induction on n.\n";
    assert!(
        surface_a(inductive).is_ok(),
        "`Proof: Induction on n.` must RUN on a goal whose ∀ prefix binds `n` — no unknown \
         tactic, no trailing tokens, no inapplicable rule: {:?}",
        surface_a(inductive)
    );

    // THE NEGATIVE. The same spelling over a FALSE goal must not come back as a proof — the
    // sketch sweep is not a licence, and `induction` is not a way around the prover.
    let false_goal = "## Theorem: InductFalse\n\
                      Prove: For all n, n plus 1 equals n.\n\
                      Proof: Induction on n.\n";
    assert!(
        surface_a(false_goal).is_err(),
        "…and it must not prove a FALSE one: {:?}",
        surface_a(false_goal)
    );

    // The spelling is still classified as a SCRIPT, not reclassified into the dead
    // `Induction` variant — pinned here as well as in the tier file, because the two halves
    // of T12-005 are only consistent while both hold.
    assert_eq!(
        strategy_of(inductive),
        ProofStrategy::Script("Induction on n .".to_string()),
        "the documented induction spelling stays raw script text"
    );
}

/// A sentence that is unambiguously arithmetic does not also have to CONTAIN arithmetic —
/// asserted through the PROVER, where it matters. `2 equals 2.` had no rendering on the
/// surface whose headline theorem is `∀ n, n + 0 = n`.
#[test]
fn the_smallest_true_statements_of_arithmetic_prove_on_the_english_surface() {
    for english in ["2 equals 2.", "2 plus 2 equals 4.", "0 equals 0."] {
        let block = goal_only("Smallest", english);
        assert!(
            certified(&block),
            "`{english}` must state and prove on the English theorem surface: {:?}",
            surface_a(&block)
        );
    }

    // THE NEGATIVE, one rung over: the false closed statements of the same shape.
    for english in ["2 equals 3.", "2 plus 2 equals 5."] {
        let block = goal_only("SmallestFalse", english);
        assert!(
            !certified(&block),
            "`{english}` is FALSE and must be refused: {:?}",
            surface_a(&block)
        );
    }

    // An APPLICATION opens a sentence English does not write, so it needs no operator
    // either — and an uninterpreted symbol still proves nothing about its values.
    let from_premise = "## Theorem: FromPremise\n\
                        Given: f(3) equals 3.\n\
                        Prove: f(3) equals 3.\n\
                        Proof: Auto.\n";
    assert!(
        certified(from_premise),
        "an application equation must be statable as a PREMISE and as a GOAL: {:?}",
        surface_a(from_premise)
    );
    assert!(
        !certified(&goal_only("Uninterpreted", "f(3) equals 3.")),
        "…and it must prove nothing on its own: {:?}",
        surface_a(&goal_only("Uninterpreted", "f(3) equals 3."))
    );
}

/// A measure's UNIT is part of its identity on every arm of `NumberKind`, including the
/// symbolic one — and `proof_convert::term_to_proof_term` used to drop it there.
///
/// This matters only now that a numeral too large for an `i64` takes the symbolic arm instead
/// of silently becoming `0`. `Integer` and `Real` both append the unit to the
/// `ProofTerm::Constant` they build; `Symbolic` did not, so the same unheld numeral in two
/// different units would have reached the kernel as one constant — the same
/// two-things-become-one-term failure the numeral fix exists to close, one layer down.
///
/// Asserted on the ProofExpr itself rather than through the prover, because the prover cannot
/// see it: the enclosing `∃`/uniqueness structure of `The price is …` does not close either
/// way, so a proof-level assertion here would pass whether the unit survived or not.
#[test]
fn a_measure_carries_its_unit_onto_the_symbolic_arm_too() {
    let huge = goal_as_proof_expr(
        "## Theorem: Huge\nProve: The price is 99999999999999999999 dollars.\nProof: Auto.\n",
    );
    assert!(
        huge.contains("99999999999999999999dollars"),
        "an unheld numeral must reach the kernel WITH its unit, exactly as a held one does: \
         {huge}"
    );

    // The two held arms, for the boundary either side of the symbolic one.
    let small =
        goal_as_proof_expr("## Theorem: Small\nProve: The price is 5 dollars.\nProof: Auto.\n");
    assert!(
        small.contains("5dollars"),
        "an `i64` measure keeps its unit: {small}"
    );
    let real =
        goal_as_proof_expr("## Theorem: Real\nProve: The weight is 2.5 kilograms.\nProof: Auto.\n");
    assert!(
        real.contains("2.5kilograms"),
        "a `Real` measure keeps its unit: {real}"
    );
}

/// The goal of a `## Theorem` block, lowered through `proof_convert` and rendered with
/// `Debug`. The rendering is the only door onto a `ProofExpr` this crate has that does not
/// also run the prover, and what these tests are about is the TERM, not the search.
fn goal_as_proof_expr(src: &str) -> String {
    let mut interner = Interner::new();
    let tokens = Lexer::new(src, &mut interner).tokenize();
    let trie = mwe::build_mwe_trie();
    let tokens = mwe::apply_mwe_pipeline(tokens, &trie, &mut interner);
    let types = analysis::DiscoveryPass::new(&tokens, &mut interner).run();
    let (expr, term, np, sym, role, pp) = (
        Arena::new(),
        Arena::new(),
        Arena::new(),
        Arena::new(),
        Arena::new(),
        Arena::new(),
    );
    let ctx = AstContext::new(&expr, &term, &np, &sym, &role, &pp);
    let mut world = WorldState::new();
    let mut parser = Parser::new(tokens, &mut world, &mut interner, ctx, types);
    let statements = parser
        .parse_program()
        .unwrap_or_else(|e| panic!("the theorem block must parse: {e:?}"));
    let theorem = statements
        .iter()
        .find_map(|s| match s {
            Stmt::Theorem(t) => Some(t),
            _ => None,
        })
        .expect("the document declares a `## Theorem` block");
    format!(
        "{:?}",
        logicaffeine_language::proof_convert::logic_expr_to_proof_expr(theorem.goal, &interner)
    )
}

// ═══════════ THE POSITIVE FORM OF T12-011 — ALL THREE SURFACES, NOT ONE ═══════════

/// The corpus's routing policy rests on one sentence: *"The intersection `quantifier AND
/// function term` is empty. `∀ n : Nat, n + 0 = n` — the first theorem anyone writes — is
/// expressible on NO vernacular surface. It is two lines on K."*
///
/// It is expressible on **all three**, and each of them PROVES it, kernel-certified, from no
/// axioms. Surface B was already the refutation (`formula::parse_term` recurses into
/// `ProofTerm::Function`, and `parse_atom` routes an arithmetic `Eq` to `ProofExpr::Identity`);
/// this campaign gave Surface A a term layer and a bare-variable binder, and Surface C a
/// `forall`. Routing this theorem to the raw kernel is now a CHOICE on every surface, not a
/// necessity on any.
///
/// The tier file's own `the_quantifier_and_function_term_intersection_is_not_empty` still
/// asserts the two NEGATIVES — that Surface A and Surface C cannot do this — on input that is
/// byte-equivalent to the two capability cases it shares the file with (T12-002 and T12-009,
/// both green). Those assertions are the case's evidence, not its thesis, and they are now
/// false. This lane may not edit that file, so the thesis is locked here instead, positively,
/// where it cannot rot: if any surface loses the capability, this goes red and names which one.
#[test]
fn every_vernacular_surface_states_and_proves_that_adding_zero_changes_nothing() {
    // SURFACE A — the English theorem block. English quantifier, English arithmetic.
    let surface_a_block = goal_only("AddZeroA", "For all n, n plus 0 equals n.");
    assert!(
        certified(&surface_a_block),
        "Surface A must state and prove `∀ n, n + 0 = n` in English: {:?}",
        surface_a(&surface_a_block)
    );

    // SURFACE B — the formal FOL development. `for all` binder over a function term.
    let surface_b: Vec<(String, bool)> =
        prove_development("Theorem add_zero_right: prove for all n, Eq(Add(n, 0), n).")
            .expect("the formal development parses")
            .into_iter()
            .map(|(_, r)| (r.name, r.verified))
            .collect();
    assert_eq!(
        surface_b,
        vec![("add_zero_right".to_string(), true)],
        "Surface B must certify the same proposition — this was the original refutation"
    );

    // SURFACE C — the literate kernel theorem. A real `forall` binder over real terms.
    let surface_c = logicaffeine_kernel::interface::Repl::new().execute(
        "## Theorem: AddZeroC\n    Statement: forall n : Nat, (add n 0) equals n.\n    Proof: ring.",
    );
    assert!(
        surface_c.is_ok(),
        "Surface C must bind its quantified variable and prove the same proposition: {:?}",
        surface_c.map(|_| ())
    );

    // THE NEGATIVE, one rung over, on every surface that has arithmetic: the neighbouring
    // FALSE statement must be refused, or "all three can state it" would be worth nothing.
    assert!(
        !certified(&goal_only("AddOneA", "For all n, n plus 1 equals n.")),
        "Surface A must refuse `∀ n, n + 1 = n`: {:?}",
        surface_a(&goal_only("AddOneA", "For all n, n plus 1 equals n."))
    );
    assert_eq!(
        prove_development("Theorem add_one_right: prove for all n, Eq(Add(n, 1), n).")
            .expect("parses")
            .into_iter()
            .map(|(_, r)| r.verified)
            .collect::<Vec<_>>(),
        vec![false],
        "…and so must Surface B"
    );
    assert!(
        logicaffeine_kernel::interface::Repl::new()
            .execute(
                "## Theorem: AddOneC\n    Statement: forall n : Nat, (add n 1) equals n.\n    Proof: ring."
            )
            .is_err(),
        "…and so must Surface C"
    );
}

// ══════════════════ WAVE 3 — A PROOF LINE THAT NAMES ITS INFERENCE RULE ══════════════════

/// **The capability T12-012 is named after.** `ProofState::by_rule` gave the proof layer a way
/// to apply a kernel inference rule BY NAME and certify the node before reporting success;
/// this is the English spelling of that door, and until now no Surface A line could reach it.
///
/// `By <Rule>` rather than the bare `<Rule>`, because a rule is not a proof STEP — Lean answers
/// `by modus_ponens` with "unknown tactic" and wants `apply` too. The one-word line keeps its
/// own meaning (a tactic, pinned by `a_one_word_proof_line_is_a_rule_and_a_many_word_one_is_a_script`),
/// and the refusal now names this spelling instead of leaving the author to guess.
#[test]
fn an_english_proof_line_applies_the_inference_rule_it_names() {
    let by_modus_ponens = "## Theorem: ByModusPonens\n\
                           Given: If Bob is happy then Bob is sad.\n\
                           Given: Bob is happy.\n\
                           Prove: Bob is sad.\n\
                           Proof: By ModusPonens.\n";

    // WHY THE LOWERING EXISTS, asserted rather than assumed: `by` is a FILLER in the tactic
    // script vocabulary, so the prose reaches the engine as the bare rule name and dies. If
    // someone deletes `lower_rule_applications`, this line stays true and the next one goes red.
    let raw = logicaffeine_proof::tactic_script::parse_script("By ModusPonens .");
    assert!(
        raw.is_err_and(|e| e.to_string().contains("unknown tactic")),
        "the tactic namespace does not carry a rule name — the vernacular must LOWER it"
    );

    let out = surface_a(&by_modus_ponens).expect("`Proof: By ModusPonens.` must prove");
    assert!(
        out.contains(CERTIFIED),
        "a proof line that names its inference rule must reach a KERNEL CERTIFICATE: {out}"
    );
    assert!(
        out.contains("ModusPonens"),
        "and the rule the author NAMED must be the one in the derivation — `by_rule` is one \
         step, not a search that happens to succeed: {out}"
    );

    // The lowering is observable in the AST, not hidden in a driver.
    assert_eq!(
        strategy_of(by_modus_ponens),
        ProofStrategy::Script("by_rule ModusPonens .".to_string()),
        "`By <Rule>` lowers to the tactic namespace's own `by_rule` atom — and only that \
         pair: the sentence period is the step separator and stays exactly where it was"
    );

    // Both Surface A doors, as always.
    assert!(
        compile_theorem_for_ui(by_modus_ponens).verified,
        "the structured door must agree"
    );

    // It GENERALISES — one rule proving is a special case, the namespace is the capability.
    let conjunction = "## Theorem: ByConjunctionIntro\n\
                       Given: Bob is happy.\n\
                       Given: Bob is sad.\n\
                       Prove: Bob is happy and Bob is sad.\n\
                       Proof: By ConjunctionIntro.\n";
    assert!(
        certified(conjunction),
        "∧-introduction must be nameable too: {:?}",
        surface_a(conjunction)
    );
    let snake = conjunction.replace("By ConjunctionIntro.", "By conjunction_intro.");
    assert!(
        certified(&snake),
        "…under the snake_case spelling a translator emits as readily as the variant name: {:?}",
        surface_a(&snake)
    );
}

/// **The precedence, and it is the whole reason this is safe.** `Contradiction` and
/// `Reflexivity` are BOTH inference-rule names and tactic words. The vernacular's own
/// vocabulary wins: `By contradiction.` is still `by_contra` and `By reflexivity.` is still
/// `rfl`, because the lowering fires only for a word the TACTIC TABLE does not carry — and it
/// asks that table rather than copying a word list that would drift.
#[test]
fn the_tactic_namespace_outranks_the_rule_namespace_in_an_english_proof_line() {
    // Every one of these words is a live tactic, so every line must stay VERBATIM.
    for (line, expected) in [
        ("By contradiction.", "By contradiction ."),
        ("By reflexivity.", "By reflexivity ."),
        ("By assumption.", "By assumption ."),
        ("By automation.", "By automation ."),
        ("By cases on h.", "By cases on h ."),
        ("By induction.", "By induction ."),
        ("Assume h. By assumption.", "Assume h . By assumption ."),
        (
            "By cases on h. Right, by assumption. Left, by assumption.",
            "By cases on h . Right , by assumption . Left , by assumption .",
        ),
    ] {
        assert_eq!(
            strategy_of(&format!(
                "## Theorem: T\nProve: Bob is happy.\nProof: {line}\n"
            )),
            ProofStrategy::Script(expected.to_string()),
            "`{line}` names a TACTIC and must reach the engine exactly as written"
        );
    }

    // And the words still MEAN what they meant — a classification that changed the text
    // without changing the proof would be the silent version of this bug.
    let reflexive = "## Theorem: Refl\nProve: Bob is Bob.\nProof: By reflexivity.\n";
    let script_reflexive = "## Theorem: Refl\nProve: Bob is Bob.\nProof: rfl.\n";
    assert_eq!(
        surface_a(reflexive),
        surface_a(script_reflexive),
        "`By reflexivity.` is the `rfl` tactic, not `by_rule Reflexivity`"
    );

    // A `by` that is PROSE inside a step, not the head of one, is never rewritten either.
    assert_eq!(
        strategy_of("## Theorem: T\nProve: Bob is happy.\nProof: Have h by ModusPonens.\n"),
        ProofStrategy::Script("Have h by ModusPonens .".to_string()),
        "the rewrite is anchored at a STEP boundary — a mid-step `by` belongs to `have`"
    );
}

/// **The negatives.** `by_rule` is one step and not a search, so naming the WRONG rule must
/// fail, naming a rule with no certifier arm must report that, and naming nothing at all must
/// say so. A `Proof:` line that names a rule can never become a licence to prove.
#[test]
fn a_named_rule_that_does_not_discharge_the_goal_is_refused() {
    let wrong_rule = "## Theorem: WrongRule\n\
                      Given: If Bob is happy then Bob is sad.\n\
                      Given: Bob is happy.\n\
                      Prove: Bob is sad.\n\
                      Proof: By ConjunctionIntro.\n";
    let err = surface_a(wrong_rule).expect_err("∧-introduction cannot reach a non-conjunction");
    assert!(
        err.contains("ConjunctionIntro"),
        "the refusal must name the rule the author chose: {err}"
    );
    // …and the same theorem proves under the RIGHT rule, so the failure is the choice.
    assert!(
        certified(&wrong_rule.replace("ConjunctionIntro", "ModusPonens")),
        "the theorem itself is not at fault"
    );

    // A rule that is DECLARED but has no certifier arm reports that, rather than failing as
    // if the goal were the problem.
    let dead_rule = wrong_rule.replace("ConjunctionIntro", "TemporalTransitivity");
    let err = surface_a(&dead_rule).expect_err("a rule with no certifier arm cannot discharge");
    assert!(
        err.contains("no certifier arm"),
        "a declared-but-unimplemented rule must say which kind of dead it is: {err}"
    );

    // THE ONE THAT MATTERS. A named rule must never certify a FALSE statement.
    let false_goal = "## Theorem: FalseByRule\n\
                      Given: Bob is happy.\n\
                      Prove: Bob is sad.\n\
                      Proof: By ModusPonens.\n";
    assert!(
        surface_a(&false_goal).is_err(),
        "naming a rule is not a licence — `by_rule` certifies the node before it succeeds: {:?}",
        surface_a(&false_goal)
    );

    // THE `sorry` SHAPE. `Axiom` is a nameable rule, so `Proof: By Axiom.` is a spelling an
    // author will try — and it must NOT be a licence to assume the goal. The certifier
    // resolves an `Axiom` leaf against the hypotheses and declarations actually in scope, so
    // a goal that is not one of them is refused; a `Given:` that IS the goal is an ordinary
    // assumption step and proves, which is the only reading that is sound.
    let assumed = "## Theorem: ByAxiom\n\
                   Prove: Bob is sad.\n\
                   Proof: By Axiom.\n";
    assert!(
        surface_a(assumed).is_err(),
        "`By Axiom.` must never close a goal nothing in scope proves — that is `sorry` with a \
         kernel certificate on it: {:?}",
        surface_a(assumed)
    );
    assert!(
        certified(
            "## Theorem: ByAxiomGiven\nGiven: Bob is sad.\nProve: Bob is sad.\nProof: By Axiom.\n"
        ),
        "…while a proposition that IS in scope is an ordinary assumption and must still prove"
    );

    // A word that is neither a tactic nor a rule stays an unknown tactic; the lowering never
    // invents a rule out of a typo.
    let nonsense = wrong_rule.replace("ConjunctionIntro", "ModusPonenz");
    assert!(
        surface_a(&nonsense).is_err_and(|e| e.contains("unknown tactic")),
        "a misspelt rule name must fail as loudly as any other unknown step: {:?}",
        surface_a(&nonsense)
    );
}

/// **A defect this lane found outside the corpus.** The one-word `Proof:` diagnostic told
/// EVERY unrecognised word that it "names a kernel INFERENCE RULE, not a proof step" — a
/// statement that is simply false for `Proof: Frobnicate.`, and one that sends the author
/// looking for a rule that does not exist. The message now asks
/// `InferenceRule::from_name` and says which of the two kinds of failure this is.
#[test]
fn a_one_word_proof_line_is_told_which_kind_of_word_it_used() {
    let base = "## Theorem: OneWord\n\
                Given: If Bob is happy then Bob is sad.\n\
                Given: Bob is happy.\n\
                Prove: Bob is sad.\n";

    // A REAL rule name: still refused (a rule is not a step), and now told the spelling that
    // applies it.
    let rule = format!("{base}Proof: ModusPonens.\n");
    let err = surface_a(&rule).expect_err("a rule name is not a proof step");
    assert!(
        err.contains("unknown tactic"),
        "the failure stays loud: {err}"
    );
    assert!(
        err.contains("INFERENCE RULE"),
        "and still names the namespace the word belongs to: {err}"
    );
    assert!(
        err.contains("Proof: By ModusPonens."),
        "and must now TEACH the spelling that applies it — the capability existed with no \
         way for the author to find it: {err}"
    );

    // A word that is NEITHER must not be told it is a rule.
    let neither = format!("{base}Proof: Frobnicate.\n");
    let err = surface_a(&neither).expect_err("`Frobnicate` is not a step of any kind");
    assert!(err.contains("unknown tactic"), "still loud: {err}");
    assert!(
        !err.contains("names a kernel INFERENCE RULE"),
        "`Frobnicate` names NO inference rule — asserting it does is a false statement to \
         the author, and the one that sends them hunting for a rule that never existed: {err}"
    );
    assert!(
        err.contains("neither a proof step nor a kernel inference rule"),
        "…and it must say what it actually is: {err}"
    );

    // Both doors carry the same reason, as everywhere else on this surface.
    assert!(
        compile_theorem_for_ui(&neither)
            .verification_error
            .as_deref()
            .is_some_and(|e| e.contains("neither a proof step")),
        "the structured door must not go silent on the same input"
    );
}

/// **`ProofStrategy::Induction` is no longer a variant nothing builds.** Wave 1 gave it a real
/// arm in BOTH drivers and no site in the workspace constructed it — the mirror image of a
/// variant that is built and never driven, and just as invisible.
///
/// `Proof: By induction on n.` is its construction site: the whole line naming a method and
/// its argument. It is the SAME proof the script spelling reaches — asserted here byte for
/// byte, so the structured variant can never drift into a second, divergent engine.
#[test]
fn the_structured_induction_strategy_is_constructed_and_proves_what_the_script_does() {
    let structured = "## Theorem: Induct\n\
                      Prove: For all n, n plus 0 equals n.\n\
                      Proof: By induction on n.\n";
    let script = "## Theorem: Induct\n\
                  Prove: For all n, n plus 0 equals n.\n\
                  Proof: Induction on n.\n";

    assert_eq!(
        strategy_of(structured),
        ProofStrategy::Induction("n".to_string()),
        "`Proof: By induction on n.` must reach the structured variant — it had no \
         construction site anywhere in the workspace"
    );
    assert_eq!(
        surface_a(structured),
        surface_a(script),
        "…and must be the SAME proof the script spelling produces, step for step: one engine, \
         two spellings, no divergence"
    );
    assert!(
        surface_a(structured).is_ok(),
        "and it must actually run: {:?}",
        surface_a(structured)
    );
    assert_eq!(
        compile_theorem_for_ui(structured).verified,
        compile_theorem_for_ui(script).verified,
        "the structured door must not reach a different verdict either"
    );

    // THE NEGATIVE. A method name is not a licence: the FALSE statement one rung over is
    // refused under the structured spelling exactly as under the script one.
    let false_goal = structured.replace("n plus 0 equals n", "n plus 1 equals n");
    assert!(
        surface_a(&false_goal).is_err(),
        "`By induction on n.` must not prove `∀n. n + 1 = n`: {:?}",
        surface_a(&false_goal)
    );

    // THE BOUNDARY, one rung either side of the shape. More than the whole line is a SCRIPT —
    // collapsing it would silently discard every step after the first.
    assert_eq!(
        strategy_of(
            "## Theorem: T\nProve: Bob is happy.\nProof: By induction on n. Automatically.\n"
        ),
        ProofStrategy::Script("By induction on n . Automatically .".to_string()),
        "a two-step script stays a script"
    );
    assert_eq!(
        strategy_of("## Theorem: T\nProve: Bob is happy.\nProof: By induction.\n"),
        ProofStrategy::Script("By induction .".to_string()),
        "…and induction with no named variable is not the structured variant, which carries one"
    );
}

/// **The pit-of-success lock for the whole enum.** `ProofStrategy` is public and four-way, and
/// TWO of its four variants used to be unreachable — one built and never driven, one driven
/// and never built. Neither is visible to a compiler, a reviewer or a type. This test is what
/// makes it visible: every variant is named with the vernacular line that builds it, so a
/// fifth variant added without a spelling, or a spelling deleted, goes red here.
#[test]
fn every_proof_strategy_variant_has_a_vernacular_spelling_that_builds_it() {
    let block = |proof: &str| format!("## Theorem: Every\nProve: Bob is happy.\nProof: {proof}\n");
    let built = [
        ("Auto.", ProofStrategy::Auto),
        (
            "By induction on n.",
            ProofStrategy::Induction("n".to_string()),
        ),
        (
            "ModusPonens.",
            ProofStrategy::ByRule("ModusPonens".to_string()),
        ),
        (
            "Assume h. By assumption.",
            ProofStrategy::Script("Assume h . By assumption .".to_string()),
        ),
    ];
    for (line, expected) in &built {
        assert_eq!(
            &strategy_of(&block(line)),
            expected,
            "`Proof: {line}` must build {expected:?} — a variant with no construction site is \
             a dead branch that every consumer of this public enum still has to handle"
        );
    }

    // EXHAUSTIVE, by construction: the match below is what fails to compile if a variant is
    // added, and the loop above is what fails at run time if the new variant has no spelling.
    let mut seen = [false; 4];
    for (_, strategy) in &built {
        let slot = match strategy {
            ProofStrategy::Auto => 0,
            ProofStrategy::Induction(_) => 1,
            ProofStrategy::ByRule(_) => 2,
            ProofStrategy::Script(_) => 3,
        };
        assert!(
            !seen[slot],
            "two spellings collapsed onto one variant: {strategy:?}"
        );
        seen[slot] = true;
    }
    assert!(
        seen.iter().all(|s| *s),
        "a `ProofStrategy` variant has no vernacular spelling"
    );
}

/// **The class-level lock, not the instance.** `By <Rule>` opened a door onto the WHOLE
/// inference-rule enum, so testing it with `ModusPonens` and calling it covered would leave
/// thirty-seven rules unexamined. Two invariants are asserted over every nameable rule there
/// is, so a rule added to `InferenceRule::NAMEABLE` is carried into both automatically:
///
/// 1. **No named rule can mint a certificate.** `by_rule` certifies before it succeeds, so a
///    goal that follows from nothing in scope must be refused whatever rule is named — and
///    the scope here is deliberately rich enough for a rule to find premises to chew on.
/// 2. **The precedence holds enum-wide.** A rule name the TACTIC table also carries keeps its
///    tactic meaning; every other one lowers. Both halves are read from the tactic table
///    itself, so this cannot drift into a hand-maintained exception list.
#[test]
fn the_english_rule_door_is_sound_and_consistently_classified_over_every_nameable_rule() {
    use logicaffeine_proof::InferenceRule;

    let block = |proof: &str| {
        format!(
            "## Theorem: RuleDoor\n\
             Given: Bob is happy.\n\
             Given: If Bob is happy then Bob is sad.\n\
             Prove: Mary is happy.\n\
             Proof: {proof}\n"
        )
    };
    let is_tactic_word = |w: &str| match logicaffeine_proof::tactic_script::parse_script(w) {
        Ok(_) => true,
        Err(e) => !e.to_string().contains("unknown tactic"),
    };

    let mut minted: Vec<String> = Vec::new();
    let mut misclassified: Vec<String> = Vec::new();
    for rule in InferenceRule::NAMEABLE {
        let name = rule.canonical_name();

        // (1) SOUNDNESS. `Mary is happy.` follows from nothing here.
        for spelling in [format!("By {name}."), format!("{name}.")] {
            if certified(&block(&spelling)) {
                minted.push(spelling);
            }
        }

        // (2) CLASSIFICATION, over the whole enum.
        let expected = if is_tactic_word(&name) {
            ProofStrategy::Script(format!("By {name} ."))
        } else {
            ProofStrategy::Script(format!("by_rule {name} ."))
        };
        let got = strategy_of(&block(&format!("By {name}.")));
        if got != expected {
            misclassified.push(format!("`By {name}.` → {got:?}, expected {expected:?}"));
        }
    }

    assert!(
        minted.is_empty(),
        "an English proof line that NAMES a rule minted a kernel certificate for a goal \
         nothing in scope proves — `by_rule` must certify before it succeeds: {minted:#?}"
    );
    assert!(
        misclassified.is_empty(),
        "the tactic namespace must outrank the rule namespace for every rule, not just the \
         two this lane happened to check: {misclassified:#?}"
    );

    // …and the same over the snake_case spelling a translator emits, which `from_name`
    // accepts and which must therefore behave identically.
    let mut snake_minted: Vec<String> = Vec::new();
    for rule in InferenceRule::NAMEABLE {
        let name = rule.canonical_name();
        let snake: String = name
            .chars()
            .enumerate()
            .fold(String::new(), |mut acc, (i, c)| {
                if c.is_uppercase() && i > 0 {
                    acc.push('_');
                }
                acc.extend(c.to_lowercase());
                acc
            });
        if certified(&block(&format!("By {snake}."))) {
            snake_minted.push(snake);
        }
    }
    assert!(
        snake_minted.is_empty(),
        "the snake_case spelling is the same door and must be just as shut: {snake_minted:#?}"
    );
}

/// **Both Surface A doors must disclose the same AMOUNT, not merely reach the same verdict.**
///
/// The vernacular `Proof:` line is a proof SKETCH: the author names the method and the same
/// certified automation `Proof: Auto.` uses closes the routine obligations it leaves. That is
/// a deliberate reading of the surface — and it is only honest if the answer SAYS how much
/// the author left to the machine. The prose door has always said it —
/// `"Proved! [kernel-verified] (2 residual obligations discharged by automation)"` — and the
/// STRUCTURED door, which is what the studio and every programmatic consumer read, computed
/// the same number and dropped it: `verified: true`, and no way to tell a complete proof from
/// a one-word gesture at one.
///
/// Nothing about trust moves either way. Every swept obligation is assembled into the SAME
/// derivation and certified through `qed` → `check_derivation` → the kernel; this is
/// disclosure, and `the_trust_door_separates_a_kernel_certificate_from_a_bare_derivation`
/// owns the trust half.
#[test]
fn both_surface_a_doors_disclose_how_much_of_the_proof_the_automation_did() {
    /// The count the PROSE door reports, read back out of its own sentence — the independent
    /// model, since it is produced by a different function in a different crate.
    fn noted(src: &str) -> usize {
        let Ok(text) = surface_a(src) else { return 0 };
        let Some((head, _)) = text.split_once(" residual obligation") else {
            return 0;
        };
        head.rsplit('(')
            .next()
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or_else(|| panic!("the prose note must carry a count: {text}"))
    }

    // THE ANCHOR. A declared METHOD that closes nothing: `n` occurs nowhere in the goal, so
    // `induction on n` generalises over a fresh Nat and leaves both constructor cases open.
    // The author wrote one word of proof; the automation wrote the rest.
    let gestured = "## Theorem: Gestured\n\
                    Given: Bob is happy.\n\
                    Prove: Bob is happy.\n\
                    Proof: By induction on n.\n";
    assert!(
        certified(&gestured.to_string()),
        "the sketch must still certify: {:?}",
        surface_a(gestured)
    );
    assert_eq!(
        noted(gestured),
        2,
        "the prose door reports both open cases: {:?}",
        surface_a(gestured)
    );
    let ui = compile_theorem_for_ui(gestured);
    assert!(
        ui.verified,
        "and the structured door must agree it is proved"
    );
    assert_eq!(
        ui.discharged, 2,
        "…and must say the automation closed 2 obligations. It used to return `verified: \
         true` with no way at all to tell this from a proof the author actually wrote"
    );

    // THE NEGATIVE, and the reason the anchor is not vacuous: a COMPLETE proof discloses
    // nothing, because there was nothing to disclose.
    let complete = "## Theorem: Complete\n\
                    Given: Bob is happy.\n\
                    Prove: Bob is happy.\n\
                    Proof: By assumption.\n";
    assert!(
        certified(&complete.to_string()),
        "{:?}",
        surface_a(complete)
    );
    assert!(
        !surface_a(complete).unwrap().contains("residual"),
        "an author's complete proof must not be annotated: {:?}",
        surface_a(complete)
    );
    assert_eq!(compile_theorem_for_ui(complete).discharged, 0);

    // `Auto` IS the automation, so it has nothing to report: the count is 0 by construction,
    // never by accident.
    assert_eq!(
        compile_theorem_for_ui(&goal_only("Auto", "For all n, n plus 0 equals n.")).discharged,
        0
    );

    // A theorem that does NOT prove discloses nothing either — a count is a fact about a
    // proof, and there is no proof.
    let refused = "## Theorem: Refused\n\
                   Given: Bob is happy.\n\
                   Prove: Mary is happy.\n\
                   Proof: By induction on n.\n";
    let bad = compile_theorem_for_ui(refused);
    assert!(
        !bad.verified,
        "an unentailed goal must not certify: {:?}",
        bad.verification_error
    );
    assert_eq!(
        bad.discharged, 0,
        "a failed sketch has no discharged count to report"
    );

    // THE CLASS. Over every strategy variant and every sketch shape this lane touches, the
    // two doors must report the same number — not just on the anchor above.
    let blocks = [
        gestured.to_string(),
        complete.to_string(),
        refused.to_string(),
        goal_only("Auto", "For all n, n plus 0 equals n."),
        "## Theorem: Ind\nProve: For all n, n plus 0 equals n.\nProof: By induction on n.\n".to_string(),
        "## Theorem: Scr\nProve: For all n, n plus 0 equals n.\nProof: Induction on n.\n".to_string(),
        "## Theorem: Rule\nGiven: Bob is happy.\nProve: Bob is happy.\nProof: Assumption.\n".to_string(),
        "## Theorem: Half\nGiven: Bob is happy.\nGiven: Bob is tall.\nProve: Bob is happy and Bob is tall.\nProof: Split.\n".to_string(),
    ];
    let mut drift: Vec<String> = Vec::new();
    for src in &blocks {
        let prose = noted(src);
        let structured = compile_theorem_for_ui(src).discharged;
        if prose != structured {
            drift.push(format!(
                "{prose} (prose) vs {structured} (structured) for {src:?}"
            ));
        }
    }
    assert!(
        drift.is_empty(),
        "the two Surface A doors compute the residual count from the same driver and must \
         report it identically: {drift:#?}"
    );
}

/// **Our own doc comments are a spec, and this is the instrument that makes them one.**
///
/// T12-005 exists because `ProofStrategy::Induction`'s doc comment taught `Proof: Induction on
/// n.` as its worked example and the surface refused it — *"the documentation teaches a
/// spelling that does not work, which is worse than an undocumented gap"*. Fixing that one
/// spelling fixes one instance. This fixes the CLASS: every `Proof:` spelling quoted anywhere
/// in `ast/theorem.rs` is read OUT OF THE FILE and run against the surface, and every
/// ` ```text ` block in it that is a theorem must certify.
///
/// It found a live second instance the moment it was written. `ProofStrategy::Script` — the
/// one open-ended variant, the one an implementer is likeliest to copy — taught
/// `Proof: Assume h. By cases on h, right, by assumption. Left, by assumption.`, which wants a
/// goal that is BOTH an implication (for `Assume`) and a disjunction (for `cases on h`). This
/// surface has no sentence for that shape (`Prove: If Bob is happy or Bob is sad then …` does
/// not parse), so the sole worked example of the sole free-form variant could be run on
/// nothing at all. The doc comment now names its block, and this test runs it.
#[test]
fn every_proof_spelling_our_own_doc_comments_teach_is_checked_against_the_surface() {
    const THEOREM_AST: &str = include_str!("../../logicaffeine_language/src/ast/theorem.rs");

    /// Every doc line of the file, stripped of its `///`/`//!` marker, in order.
    fn doc_lines(src: &str) -> Vec<String> {
        src.lines()
            .map(str::trim_start)
            .filter(|l| l.starts_with("///") || l.starts_with("//!"))
            .map(|l| l[3..].trim().to_string())
            .collect()
    }

    // ============================ HALF ONE — the worked BLOCKS ============================
    // A ```text block that opens with `## Theorem` is a complete example, so it is run whole.
    let lines = doc_lines(THEOREM_AST);
    let mut blocks: Vec<String> = Vec::new();
    let mut open: Option<Vec<String>> = None;
    for line in &lines {
        match (&mut open, line.as_str()) {
            (None, "```text") => open = Some(Vec::new()),
            (Some(buf), "```") => {
                blocks.push(format!("{}\n", buf.join("\n")));
                open = None;
            }
            (Some(buf), other) => buf.push(other.to_string()),
            (None, _) => {}
        }
    }
    assert!(
        open.is_none(),
        "an unterminated ```text block in ast/theorem.rs"
    );
    let theorem_blocks: Vec<&String> = blocks
        .iter()
        .filter(|b| b.starts_with("## Theorem"))
        .collect();
    assert_eq!(
        theorem_blocks.len(),
        2,
        "a worked theorem block was added to or removed from ast/theorem.rs without updating \
         this lock: {blocks:#?}"
    );
    for block in &theorem_blocks {
        assert!(
            certified(block.as_str()),
            "a complete `## Theorem` block written in our own documentation must be \
             kernel-certified, verbatim, with no edit at all:\n{block}\n{:?}",
            surface_a(block.as_str())
        );
    }

    // ========================= HALF TWO — every quoted SPELLING =========================
    // Joined across doc lines first: a backticked example may wrap, and a lock that missed
    // the wrapped ones would be exactly the blind spot it exists to remove.
    let flat = lines.join(" ");
    let mut quoted: Vec<String> = Vec::new();
    let mut rest = flat.as_str();
    while let Some(start) = rest.find("`Proof:") {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('`') else { break };
        let spelling: String = rest[..end].split_whitespace().collect::<Vec<_>>().join(" ");
        if spelling != "Proof:" {
            quoted.push(spelling);
        }
        rest = &rest[end + 1..];
    }
    quoted.sort();
    quoted.dedup();

    // The blocks each spelling is documented FOR. A proof step means nothing without a goal.
    let trivial = |proof: &str| {
        format!("## Theorem: Trivial\nGiven: Bob is happy.\nProve: Bob is happy.\nProof: {proof}\n")
    };
    let modus = |proof: &str| {
        format!(
            "## Theorem: Modus\n\
             Given: If Bob is happy then Bob is sad.\n\
             Given: Bob is happy.\n\
             Prove: Bob is sad.\n\
             Proof: {proof}\n"
        )
    };
    let disjunction = |proof: &str| {
        format!(
            "## Theorem: Or_Commutes\n\
             Given (h): Bob is happy or Bob is sad.\n\
             Prove: Bob is sad or Bob is happy.\n\
             Proof: {proof}\n"
        )
    };

    /// What the surface must do with a documented spelling. `Refused` is a legitimate
    /// documented outcome — `Proof: ModusPonens.` is taught precisely as the spelling that is
    /// REPORTED rather than applied — but it must be refused for the documented reason.
    enum Taught {
        Certified,
        Refused(&'static str),
    }
    use Taught::{Certified, Refused};

    #[allow(clippy::type_complexity)]
    let table: Vec<(&str, ProofStrategy, String, Taught)> = vec![
        (
            "Proof: Assumption.",
            ProofStrategy::ByRule("Assumption".to_string()),
            trivial("Assumption."),
            Certified,
        ),
        (
            "Proof: ModusPonens.",
            ProofStrategy::ByRule("ModusPonens".to_string()),
            modus("ModusPonens."),
            Refused("By ModusPonens."),
        ),
        (
            "Proof: By ModusPonens.",
            ProofStrategy::Script("by_rule ModusPonens .".to_string()),
            modus("By ModusPonens."),
            Certified,
        ),
        (
            "Proof: By cases on h. Right, by assumption. Left, by assumption.",
            ProofStrategy::Script(
                "By cases on h . Right , by assumption . Left , by assumption .".to_string(),
            ),
            disjunction("By cases on h. Right, by assumption. Left, by assumption."),
            Certified,
        ),
        (
            "Proof: By induction on n.",
            ProofStrategy::Induction("n".to_string()),
            trivial("By induction on n."),
            Certified,
        ),
        (
            "Proof: Induction on n.",
            ProofStrategy::Script("Induction on n .".to_string()),
            trivial("Induction on n."),
            Certified,
        ),
    ];

    let documented: Vec<&str> = table.iter().map(|(s, ..)| *s).collect();
    let mut expected: Vec<String> = documented.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(
        quoted, expected,
        "a `Proof:` spelling was added to or removed from a doc comment in \
         `ast/theorem.rs` without a row here. Every spelling our documentation teaches must \
         be one this test RUNS — that is the whole point of the instrument"
    );

    for (spelling, strategy, block, taught) in &table {
        let proof_line = spelling
            .strip_prefix("Proof: ")
            .expect("a `Proof:` spelling");
        assert!(
            block.contains(proof_line),
            "the row for `{spelling}` must exercise the spelling verbatim: {block}"
        );
        assert_eq!(
            &strategy_of(block),
            strategy,
            "`{spelling}` must reach the `ProofStrategy` its own doc comment classifies it as"
        );
        match taught {
            Certified => assert!(
                certified(block),
                "`{spelling}` is taught by our documentation and must therefore WORK: {:?}",
                surface_a(block)
            ),
            Refused(instead) => {
                let err =
                    surface_a(block).expect_err("a spelling documented as refused must not prove");
                assert!(
                    err.contains("unknown tactic") && err.contains(*instead),
                    "`{spelling}` is documented as reported-not-applied, and the report must \
                     name `{instead}`, the spelling that DOES apply it: {err}"
                );
                // …and that named alternative is itself in the table, certified — so the
                // diagnostic can never point at a spelling nobody runs.
                assert!(
                    documented.iter().any(|d| d.contains(*instead)),
                    "the diagnostic names `{instead}`, which no row of this table runs"
                );
            }
        }
    }
}
