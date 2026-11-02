//! TIER 12 — THE VERNACULAR SURFACES. The conformance corpus, re-read as a capability matrix.
//!
//! Three human-facing theorem surfaces, in three different crates, which is why this tier lives in
//! the cross-crate suite:
//!
//! * **Surface A — the English theorem block** (`logicaffeine_language`).
//!   `## Theorem: / Given: / Prove: / Proof:`. Every `Given:`/`Prove:` line goes through the FULL
//!   English sentence parser, so there is no term grammar on it at all.
//! * **Surface B — the formal FOL development** (`logicaffeine_proof::formula` / `::development`).
//!   `## Axiom` / `## Theory`, with `for all`/`∀`, `∃`, `↔`, `→`, `∨`, `∧`, `¬`, n-ary relations,
//!   `t = t`, and — the thing the corpus audit missed — RECURSIVE FUNCTION TERMS.
//! * **Surface C — the literate kernel theorem** (`logicaffeine_kernel::interface`).
//!   `## Theorem: … Statement: … Proof: <tactic>.` with s-expression application, infix
//!   `+ - * < > <= >=` and the word `equals`. No binder form of any kind.
//!
//! The corpus's conclusion was "emit kernel terms, never vernacular" — the right call for a
//! translator, and the wrong frame for us. For LOGOS the vernacular IS the product: it is what a
//! mathematician actually writes. So where the corpus found a surface limit and routed around it, this file
//! classifies it `MUST` and asserts the capability, so the limit reads as a work queue rather than
//! as a design.
//!
//! # THE HEADLINE — the corpus's decisive claim is FALSE against today's source
//!
//! T12-011 turns on one sentence: *"The intersection `quantifier AND function term` is empty.
//! `∀ n : Nat, n + 0 = n` — the first theorem anyone writes — is expressible on NO vernacular
//! surface."* It is expressible, on Surface B, and it PROVES, kernel-certified, with no axiom:
//!
//! ```text
//! Theorem right_identity: prove for all n, Eq(Add(n, 0), n).
//! ```
//!
//! `formula::parse_term` is no longer the one-token grammar the case's `@evidence` quotes; it recurses
//! through `Name(t1, …, tn)` into `ProofTerm::Function`, and `parse_atom` routes an `Eq(a, b)` with
//! an arithmetic side to `ProofExpr::Identity`. What remains true is the INFIX spelling: once a
//! predicate application has been consumed, `parse_atom` returns without ever looking for a
//! following `=`, so `Add(n, Zero) = n` is still a trailing-input error. Both halves are pinned
//! below — the capability by `a_function_symbol_in_term_position_survives_surface_b`, the residue by
//! its own assertion inside it.
//!
//! # HOW A PROOF IS ASSERTED HERE
//!
//! `.is_ok()` is never a pass predicate on any of the three surfaces, and each one lies differently:
//!
//! * Surface A — `compile_theorem` returns `Ok` in BOTH the kernel-certified case and the
//!   derivation-found-but-uncertified case; only `"[kernel-verified]"` separates them
//!   (`compile.rs`, the `Some(derivation) if outcome.verified` arm vs the one under it). The
//!   structured door is `compile_theorem_for_ui(…).verified`, and it reports `error: None` for a
//!   theorem it could not prove — so "no error" is not success either. Both are pinned by
//!   `the_trust_door_separates_a_kernel_certificate_from_a_bare_derivation`.
//! * Surface B — `LibraryResult::verified`, which `verify.rs` documents as true *iff* a proof term
//!   was certified and re-checked by the kernel.
//! * Surface C — `Repl::execute` returning `Ok` used to be vacuous, and the corpus's `@guard` for
//!   T12-008 is written against that world. It is now STALE: `repl.rs` carries a soundness gate that
//!   normalises `concludes body` against the stated goal's `Syntax` and rejects anything that
//!   concludes the `SName "Error"` sentinel. `5 < 2` is refused today. `literate_proof_soundness.rs`
//!   (31 tests, 54 theorems) already locks that gate true-and-false; this file does not duplicate it,
//!   it covers the statements the corpus names.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile_theorem_for_ui;
use logicaffeine_kernel::interface::Repl;
use logicaffeine_language::ast::theorem::ProofStrategy;
use logicaffeine_language::ast::Stmt;
use logicaffeine_language::compile_theorem;
use logicaffeine_language::{
    analysis, mwe, Arena, AstContext, Interner, Lexer, Parser, WorldState,
};
use logicaffeine_proof::development::{parse_development, prove_development};
use logicaffeine_proof::formula::parse_formula;
use logicaffeine_proof::verify::check_derivation;
use logicaffeine_proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

// ==================================== HELPERS ====================================
// Per-file, per house style. Nothing here is shared with another tier's file.

/// The kernel-certified marker `compile.rs` emits, and the ONLY substring that means "proved" on
/// Surface A. `"Proved"` is weaker: the uncertified arm returns `Ok` too.
const CERTIFIED: &str = "[kernel-verified]";

/// Surface A's answer, or the parse/proof failure, as a single string for assertion messages.
fn surface_a(src: &str) -> Result<String, String> {
    compile_theorem(src).map_err(|e| format!("{e:?}"))
}

/// Did Surface A come back with a KERNEL CERTIFICATE (not merely a derivation)?
fn surface_a_certified(src: &str) -> bool {
    matches!(surface_a(src), Ok(s) if s.contains(CERTIFIED))
}

/// The proof strategy the Surface A parser actually built for a block. The theorem AST is public,
/// so a dead `ProofStrategy` variant can be observed directly rather than inferred from a downstream
/// error message — which is what T12-005 and T12-012 need.
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

/// Every theorem of a Surface B development, with its kernel verdict.
fn surface_b(body: &str) -> Vec<(String, bool)> {
    prove_development(body)
        .unwrap_or_else(|e| panic!("the development must parse before it is proved: {e:?}"))
        .into_iter()
        .map(|(_, r)| (r.name, r.verified))
        .collect()
}

/// Surface C's verdict. `Ok` here is real: `Repl::execute` gates a literate tactic proof on
/// `concludes body` normalising to the stated goal.
fn surface_c(src: &str) -> Result<(), String> {
    Repl::new()
        .execute(src)
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}

// ============================ SURFACE A — THE ENGLISH THEOREM BLOCK ============================

/// **T12-001 · MUST · Native.** The canonical syllogism, and the two shapes around it: a two-step
/// instantiation chain, and the degenerate restatement that closes by `PremiseMatch` rather than by
/// `ModusPonens`. The guard is the one the case puts first — FOL text in a `Given:` line is not a
/// parse error anyone notices, because those lines go through the English SENTENCE parser.
#[test]
fn a_socratic_syllogism_kernel_verifies_on_the_english_theorem_surface() {
    let syllogism = "## Theorem: Socrates_Mortality\n\
                     Given: All men are mortal.\n\
                     Given: Socrates is a man.\n\
                     Prove: Socrates is mortal.\n\
                     Proof: Auto.\n";
    let proved = surface_a(syllogism);
    let tree = proved
        .as_ref()
        .unwrap_or_else(|e| panic!("the syllogism must prove on Surface A: {e}"));
    assert!(
        tree.contains(CERTIFIED),
        "the syllogism must come back KERNEL-CERTIFIED, not merely as a derivation: {tree}"
    );
    assert!(
        tree.contains("UniversalInst") && tree.contains("ModusPonens"),
        "the trace must show ∀-elimination then modus ponens — the predicates only meet because the \
         noun lexicon canonicalises `men` to `Man`: {tree}"
    );

    let chain = "## Theorem: Socrates_Doom\n\
                 Given: All men are mortal.\n\
                 Given: All mortals are doomed.\n\
                 Given: Socrates is a man.\n\
                 Prove: Socrates is doomed.\n\
                 Proof: Auto.\n";
    assert!(
        surface_a_certified(chain),
        "a two-step instantiation chain must also certify: {:?}",
        surface_a(chain)
    );

    let restatement = "## Theorem: Trivial_Restatement\n\
                       Given: Socrates is mortal.\n\
                       Prove: Socrates is mortal.\n\
                       Proof: Auto.\n";
    let restated = surface_a(restatement);
    assert!(
        matches!(&restated, Ok(s) if s.contains(CERTIFIED) && s.contains("PremiseMatch")),
        "the goal-is-a-premise case must close by PremiseMatch, not be special-cased away: {restated:?}"
    );

    // GUARD — raw FOL in a `Given:` line. There is no formula grammar on this surface: the line is
    // read as English, and what comes back names a missing `Prove` keyword rather than the construct
    // that actually broke. A harness scanning for "proof failed" would misread this entirely.
    let fol_premise = "## Theorem: Fol_Premise\n\
                       Given: forall x, Man(x) -> Mortal(x).\n\
                       Prove: Socrates is mortal.\n\
                       Proof: Auto.\n";
    let fol = surface_a(fol_premise);
    assert!(
        fol.is_err(),
        "formal FOL in a `Given:` line must never build a proposition about a thing called \
         `forall` — that spelling belongs to Surface B: {fol:?}"
    );

    // GUARD — `Prove:` is mandatory. There is no "state the goal last" fallback.
    let goalless = "## Theorem: No_Goal\nGiven: Socrates is a man.\nProof: Auto.\n";
    assert!(
        surface_a(goalless).is_err_and(|e| e.contains("Prove")),
        "a block with no `Prove:` line must name the missing keyword: {:?}",
        surface_a(goalless)
    );
}

/// **T12-002 · MUST · Gap.** `∀ n : Nat, n + 0 = n` has no rendering on Surface A, because a
/// `Given:`/`Prove:` line has no term grammar at all — `plus` lives only in the IMPERATIVE
/// expression grammar and is unreachable from a theorem premise.
///
/// The guards run first, and they hold: neither tempting wrong move silently succeeds. Then the
/// capability is asserted, and that is the RED — Lean states this theorem in seven characters.
#[test]
fn surface_a_states_and_proves_that_adding_zero_leaves_a_number_unchanged() {
    // GUARD — the paraphrase escape hatch. The case fears that an axiomatised English
    // restatement would PROVE, certifying a proposition that is not the Lean one. It does not even
    // parse, which is the stronger refusal. Pinned so that if the English parser ever grows this
    // sentence, the silent-success trap is caught here rather than in a conformance report.
    let paraphrase = "## Theorem: Additive_Identity\n\
                      Given: Zero is an additive identity.\n\
                      Prove: Every number plus zero is itself.\n\
                      Proof: Auto.\n";
    assert!(
        !surface_a_certified(paraphrase),
        "an English PARAPHRASE of `n + 0 = n` must never come back kernel-certified — it would \
         attest a relation between two entities, not the Lean proposition: {:?}",
        surface_a(paraphrase)
    );

    // THE CAPABILITY. `plus` is a real English word and `equals` is a real logic-mode token, so this
    // line looks writable; it is not, because the two live in different grammars.
    let add_zero = "## Theorem: Add_Zero_Right\n\
                    Prove: For all n, n plus 0 equals n.\n\
                    Proof: Auto.\n";
    assert!(
        surface_a_certified(add_zero),
        "Surface A must be able to state and prove that adding zero changes nothing — it is the \
         first theorem anyone writes, and Lean spells it `∀ n : Nat, n + 0 = n`. Today a \
         `Given:`/`Prove:` line is parsed by the English SENTENCE parser, which carries no term \
         grammar: `plus` builds an imperative `BinaryOp` in a different arena and a different AST \
         and is unreachable from here. Got: {:?}",
        surface_a(add_zero)
    );
}

/// **T12-003 · MUST · Native.** A Lean tactic block carried by a `Proof:` line. The three canonical
/// scripts all kernel-certify, and — the part that matters for a conformance harness — the two
/// failure modes are LOUD rather than silent.
#[test]
fn an_english_proof_script_discharges_implication_disjunction_and_conjunction() {
    let implication = "## Theorem: Sad_Implies_Happy\n\
                       Given: Bob is happy.\n\
                       Prove: If Bob is sad then Bob is happy.\n\
                       Proof: Assume h. By assumption.\n";
    assert!(
        surface_a_certified(implication),
        "`Assume h. By assumption.` must certify — `assume` canonicalises to `intro`, and `by` is \
         filler: {:?}",
        surface_a(implication)
    );

    let disjunction = "## Theorem: Or_Commutative\n\
                       Given: Bob is happy or Bob is sad.\n\
                       Prove: Bob is sad or Bob is happy.\n\
                       Proof: By cases on hp0. Right, by assumption. Left, by assumption.\n";
    let or_tree = surface_a(disjunction);
    assert!(
        matches!(&or_tree, Ok(s) if s.contains(CERTIFIED) && s.contains("DisjunctionCases")),
        "∨-commutativity must certify through `cases` + `right`/`left`, with the two branches \
         ordered by sentence punctuation alone: {or_tree:?}"
    );

    let conjunction = "## Theorem: And_Commutative\n\
                       Given: Bob is happy and Bob is tall.\n\
                       Prove: Bob is tall and Bob is happy.\n\
                       Proof: By cases on hp0. Split, by assumption, by assumption.\n";
    assert!(
        surface_a_certified(conjunction),
        "∧-commutativity must certify — Lean's anonymous constructor `⟨hq, hp⟩` has no script \
         spelling and must be re-expressed as `split` plus two `assumption`s: {:?}",
        surface_a(conjunction)
    );

    // GUARD — `exact` takes a HYPOTHESIS NAME, never a term. A translator that strips the
    // application off Lean's `exact (Or.inr hp)` is left looking up a hypothesis called `Or`, and
    // the failure must say so rather than reading as a proof-search failure.
    let term_exact = "## Theorem: Term_Exact\n\
                      Given: Bob is happy.\n\
                      Prove: Bob is happy.\n\
                      Proof: Exact Or.\n";
    assert!(
        surface_a(term_exact).is_err_and(|e| e.contains("NoSuchHypothesis")),
        "a term-valued `exact` must fail by naming the missing HYPOTHESIS, so the mistake is \
         legible as a syntax error and not as weakness in the prover: {:?}",
        surface_a(term_exact)
    );

    // GUARD — `canonical` is case-insensitive but the combinators are not: `first`, `try` and
    // `repeat` are matched as exact lowercase words. `Try assumption.` is an unknown tactic `Try`.
    let capitalised_try = "## Theorem: Capital_Try\n\
                           Given: Bob is happy.\n\
                           Prove: Bob is happy.\n\
                           Proof: Try assumption.\n";
    assert!(
        surface_a(capitalised_try).is_err_and(|e| e.contains("unknown tactic")),
        "`Try` is not `try` — the combinators are case-SENSITIVE where the tactic verbs are not, \
         and that asymmetry must surface as an unknown tactic: {:?}",
        surface_a(capitalised_try)
    );
}

/// **T12-004 · MUST · Native.** `Given (h):` binds the hypothesis name a script cites, and the
/// differential against the positional `hp0` spelling must agree.
///
/// Two of the case's guards come apart here. The reserved-word collision one predicts does NOT
/// happen — `cases` reads its argument as a raw word before any tactic lookup, so `Given (left):`
/// plus `cases left` proves. The multi-token guard is WORSE than it describes: `Given (my premise):`
/// does not quietly name the premise `my`, it destroys the block.
#[test]
fn a_parenthesised_given_name_is_the_hypothesis_name_the_script_cites() {
    let named = "## Theorem: Or_Comm_Named\n\
                 Given (h): Bob is happy or Bob is sad.\n\
                 Prove: Bob is sad or Bob is happy.\n\
                 Proof: By cases on h. Right, by assumption. Left, by assumption.\n";
    assert!(
        surface_a_certified(named),
        "a named premise must reach `ProofState::start_with_names` and be citable as `h`: {:?}",
        surface_a(named)
    );

    // The differential: the same theorem with the name removed and the positional id used instead.
    let positional = named
        .replace("Given (h):", "Given:")
        .replace("cases on h.", "cases on hp0.");
    assert!(
        surface_a_certified(&positional),
        "naming a premise is proof-script scaffolding only — the positional `hp0` spelling must \
         reach the identical verdict: {:?}",
        surface_a(&positional)
    );

    let existential = "## Theorem: Some_Mortal_Is_A_Man\n\
                       Given (h): Some man is mortal.\n\
                       Prove: Some mortal is a man.\n\
                       Proof: By cases on h. By automation.\n";
    let ex = surface_a(existential);
    assert!(
        matches!(&ex, Ok(s) if s.contains(CERTIFIED) && s.contains("ExistentialElim")),
        "∃-elimination through a named premise must certify, binding LOGOS's own witness: {ex:?}"
    );

    // Premise names are POSITIONAL across ALL premises, not across the unnamed ones.
    let mixed = "## Theorem: Mixed_Naming\n\
                 Given (h): Bob is happy or Bob is sad.\n\
                 Given: Bob is tall.\n\
                 Prove: Bob is sad or Bob is happy.\n\
                 Proof: By cases on h. Right, by assumption. Left, by assumption.\n";
    assert!(
        surface_a_certified(mixed),
        "a named premise followed by an unnamed one must keep the named one addressable: {:?}",
        surface_a(mixed)
    );

    // GUARD — one token only. The case expects a silent mis-naming; what actually happens is that
    // the second word derails the colon scan and the whole block loses its `Prove:` line.
    let multi_token = "## Theorem: Multi_Token\n\
                       Given (my premise): Bob is happy or Bob is sad.\n\
                       Prove: Bob is sad or Bob is happy.\n\
                       Proof: By cases on my. Right, by assumption. Left, by assumption.\n";
    assert!(
        surface_a(multi_token).is_err(),
        "a multi-token premise name must not be accepted — exactly one token is taken inside the \
         parentheses, and the remainder eats the block: {:?}",
        surface_a(multi_token)
    );
}

/// **T12-005 · MUST · Gap.** `Proof: Induction on n.` is the spelling LOGOS's OWN doc comment on
/// `ProofStrategy::Induction` gives, verbatim, as its worked example — and it does not parse.
///
/// This is not a limit, it is a defect: the parser accepts the line unconditionally and builds
/// `Script("Induction on n .")`; the script `induction` primitive takes NO argument, so `on` and `n`
/// are never consumed and the run dies on trailing tokens. The failure is reported as a *theorem*
/// failure, so a harness scanning for "proof search failed" misclassifies it as weakness in the
/// prover rather than as invalid syntax we ourselves documented.
#[test]
fn the_induction_spelling_our_own_doc_comment_teaches_proves_a_theorem() {
    let documented = "## Theorem: All_P\n\
                      Given: Bob is happy.\n\
                      Prove: Bob is happy.\n\
                      Proof: Induction on n.\n";

    // Where the line actually lands: collected lexeme-by-lexeme into a Script, not into the
    // `Induction` variant the doc comment implies.
    assert_eq!(
        strategy_of(documented),
        ProofStrategy::Script("Induction on n .".to_string()),
        "the documented induction spelling is collected as raw script text — pinned so the shape of \
         the defect is recorded alongside the capability below"
    );

    assert!(
        surface_a_certified(documented),
        "`Proof: Induction on n.` must work: it is the verbatim worked example in the doc comment \
         on `ProofStrategy::Induction`, so it is the spelling every implementer will copy. Today it \
         errors on trailing tokens, because `induction` is a ZERO-ARGUMENT primitive and `on`/`n` \
         are left over. Note the two near-misses that must NOT be treated as the fix: \
         `Proof: Induction.` parses but silently inducts on whatever the focused goal's outermost ∀ \
         binds, and `Proof: ModusPonens.` (T12-012) dies differently again. Got: {:?}",
        surface_a(documented)
    );
}

// ========================= SURFACE B — THE FORMAL FOL DEVELOPMENT =========================

/// **T12-006 · MUST · Native.** An axiomatic development over one opaque sort: named axioms,
/// lemma-to-lemma citation, and a `given`/`prove` theorem — the Tarski shape, discharged in citation
/// order and kernel-certified throughout.
///
/// The two guards are both silent failures, which is what makes them worth pinning: an uppercase
/// binder is a statement about CONSTANTS and simply does not prove, and a misspelt citation is
/// dropped without any diagnostic at all.
#[test]
fn a_tarski_theory_block_discharges_its_whole_citation_graph() {
    let tarski = "\
Axiom pseudo_reflexivity: for all a b, Cong(a, b, b, a).
Axiom inner_transitivity: for all a b c d e f, if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).
Theorem reflexivity: prove for all a b, Cong(a, b, a, b).
Theorem symmetry cites reflexivity: prove for all a b c d, if Cong(a, b, c, d) then Cong(c, d, a, b).
";
    let dev = parse_development(tarski).expect("the Tarski development parses");
    assert_eq!(
        (dev.axioms.len(), dev.theorems.len()),
        (2, 2),
        "the development must collect two axioms and two theorems"
    );
    assert_eq!(
        dev.theorems[1].cites,
        vec!["reflexivity".to_string()],
        "the citation must reach the driver so the lemma is available to its citer"
    );
    for (name, verified) in surface_b(tarski) {
        assert!(
            verified,
            "Tarski theorem `{name}` must be KERNEL-CERTIFIED, not merely derived"
        );
    }

    let null_segment = "\
Axiom identity: for all a b c, if Cong(a, b, c, c) then a = b.
Theorem null_segment: given Cong(P, Q, R, R); prove P = Q.
";
    assert_eq!(
        surface_b(null_segment),
        vec![("null_segment".to_string(), true)],
        "a theorem with a `given` clause over uppercase CONSTANTS must certify"
    );

    // GUARD — the case convention is the whole proposition. Lean binder names are often uppercase,
    // and copying them across turns a quantified statement into one about four fixed points.
    let uppercase = "\
Axiom pseudo_reflexivity: for all A B, Cong(A, B, B, A).
Theorem reflexivity: prove for all A B, Cong(A, B, A, B).
";
    assert_eq!(
        surface_b(uppercase),
        vec![("reflexivity".to_string(), false)],
        "uppercase binders are CONSTANTS, so the quantifier prefix binds nothing and the theorem is \
         a different proposition — it must not certify"
    );

    // GUARD — an unknown citation is treated as an external given, never reported. The development
    // below still proves, so a misspelt lemma name is completely invisible.
    let miscited = tarski.replace("cites reflexivity", "cites reflexivvity");
    assert!(
        surface_b(&miscited).iter().all(|(_, v)| *v),
        "a MISSPELT citation is silently dropped rather than diagnosed — pinned so the absence of a \
         diagnostic is a recorded fact rather than a surprise"
    );

    // GUARD — `prove` is the goal clause and is mandatory.
    assert!(
        parse_development("Theorem t: for all a, P(a).")
            .is_err_and(|e| e.message.contains("prove")),
        "a `Theorem` with no `prove` clause must name the missing clause"
    );
}

/// **T12-007 · MUST · Native.** *The case's `@evidence` is STALE.* It quotes
/// `formula::parse_term` as a four-line function accepting one `Ident` and nothing else, and
/// concludes Surface B rejects every non-atomic term. It does not: `parse_term` recurses through
/// `Name(t1, …, tn)` into `ProofTerm::Function`, so a function symbol in term position — the exact
/// shape of the case's Lean source — parses, nests, and PROVES.
///
/// The residue is the INFIX spelling, and it is exactly as the case describes: `parse_atom` returns the
/// moment it has consumed a predicate application and never looks for a following `=`, so
/// `Add(n, Zero) = n` is a trailing-input error. Both halves are asserted.
#[test]
fn a_function_symbol_in_term_position_survives_surface_b() {
    // `Loves x (father x)` — a unary function on an opaque sort, in an argument position.
    let nested = parse_formula("for all x, Loves(x, father(x))")
        .expect("a function symbol in term position must parse");
    match &nested {
        ProofExpr::ForAll { variable, body } => {
            assert_eq!(variable, "x");
            match body.as_ref() {
                ProofExpr::Predicate { name, args, .. } => {
                    assert_eq!(name, "Loves");
                    assert!(
                        matches!(&args[1], ProofTerm::Function(f, inner)
                            if f == "father" && matches!(&inner[0], ProofTerm::Variable(v) if v == "x")),
                        "the second argument must be the FUNCTION TERM `father(x)`, not a flattened \
                         constant: {args:?}"
                    );
                }
                other => panic!("expected a relation application under the binder, got {other:?}"),
            }
        }
        other => panic!("expected a ∀ over a relation, got {other:?}"),
    }

    // `Loves x (father (father a))` — two levels deep.
    assert!(
        matches!(
            parse_formula("for all x, Loves(x, father(father(x)))"),
            Ok(ProofExpr::ForAll { .. })
        ),
        "nesting must recurse, not bottom out at depth one"
    );

    // Arithmetic is the same obstruction wearing different clothes — and it is not an obstruction.
    // `Eq(a, b)` with an arithmetic side is routed to `ProofExpr::Identity`, so this is a ring
    // identity the oracle decides, not an uninterpreted `Eq` relation.
    let add_zero = parse_formula("for all n, Eq(Add(n, 0), n)").expect("`Add n 0 = n` must parse");
    assert!(
        matches!(&add_zero, ProofExpr::ForAll { body, .. }
            if matches!(body.as_ref(), ProofExpr::Identity(ProofTerm::Function(f, _), _) if f == "Add")),
        "an `Eq` with an arithmetic side must become an IDENTITY between terms, not a binary \
         relation between two opaque things: {add_zero:?}"
    );
    assert_eq!(
        surface_b("Theorem add_zero_right: prove for all n, Eq(Add(n, 0), n)."),
        vec![("add_zero_right".to_string(), true)],
        "`∀ n, n + 0 = n` must certify on Surface B from NO axioms"
    );
    assert_eq!(
        surface_b("Theorem add_zero_twice: prove for all n, Eq(Add(Add(n, 0), 0), n)."),
        vec![("add_zero_twice".to_string(), true)],
        "the nested-application form `Add (Add n 0) 0 = n` must certify too"
    );

    // THE RESIDUE. Once `Add(` has been consumed as an application, `parse_atom` returns without
    // ever looking for `=`, so the infix spelling reaches the top level with a stray token. Pinned
    // precisely: this is the one part of the case's claim that survives.
    assert!(
        parse_formula("for all n, Add(n, Zero) = n")
            .is_err_and(|e| e.message.contains("unexpected trailing input")),
        "the INFIX spelling after a function application is still unreachable — the prefix `Eq(…)` \
         form is the only one that works, and the error names a token rather than the construct"
    );

    // GUARD — the near-miss that parses and means something else. Relationalising a function drops
    // totality and single-valuedness, so every downstream theorem is proved in a theory the source
    // never stated. The kernel cannot catch it: the relational theory is perfectly consistent.
    assert_eq!(
        surface_b("Axiom add_rel: for all n, AddRel(n, 0, n).\nTheorem t: prove for all n, AddRel(n, 0, n)."),
        vec![("t".to_string(), true)],
        "the RELATIONALISED substitute proves cleanly — pinned so the trap is on record: it is a \
         different theory, and nothing in LOGOS will say so"
    );
}

// ======================= SURFACE C — THE LITERATE KERNEL THEOREM =======================

/// **T12-008 · MUST · Native.** The literate statement grammar: s-expression application, infix
/// `+ - *` and comparisons in both ASCII and Unicode, the word `equals`, and `implies`.
///
/// The case's headline `@guard` — *"`Repl::execute` returning `Ok` IS NOT A PROOF"* —
/// is STALE. `repl.rs` now normalises `concludes body` against the stated goal's `Syntax` and
/// refuses anything that concludes the error sentinel. `5 < 2` is rejected today. That gate is
/// locked true-and-false across 54 theorems by `literate_proof_soundness.rs`; this test covers the
/// statements the corpus names and the two shapes that file does not touch.
#[test]
fn a_literate_statement_proves_arithmetic_in_both_sexpr_and_infix_spellings() {
    for (what, src) in [
        ("s-expression `le`", "## Theorem: LeRefl\n    Statement: (le x x).\n    Proof: lia."),
        ("closed comparison", "## Theorem: TwoLtFive\n    Statement: 2 < 5.\n    Proof: lia."),
        ("infix successor", "## Theorem: LtSucc\n    Statement: x < x + 1.\n    Proof: lia."),
        ("ASCII `>=`", "## Theorem: FiveGeThree\n    Statement: 5 >= 3.\n    Proof: lia."),
        ("Unicode `≤`", "## Theorem: LeReflUnicode\n    Statement: x ≤ x.\n    Proof: lia."),
        ("coefficient", "## Theorem: LinearCoeff\n    Statement: 2 * x <= 2 * x + 1.\n    Proof: lia."),
        ("`equals` over s-exprs", "## Theorem: AddComm\n    Statement: (add x y) equals (add y x).\n    Proof: ring."),
        ("prefix, nested 3 deep", "## Theorem: MixedArith\n    Statement: (Eq (add (mul 2 3) (sub 4 1)) 9).\n    Proof: simp."),
        ("`implies` as a Pi", "## Theorem: GtToGe\n    Statement: (implies (Gt x 0) (Ge x 1)).\n    Proof: omega."),
    ] {
        assert!(
            surface_c(src).is_ok(),
            "the {what} spelling must prove on Surface C — `Ok` here is a real verdict, because the \
             REPL requires `concludes body` to reduce to the stated goal: {:?}",
            surface_c(src)
        );
    }

    // The gate, on the corpus's own falsehood. The case's evidence cites two suite tests that
    // assert `is_err() || is_ok()` for exactly this; the gate is real now and this is falsifiable.
    let false_goal = "## Theorem: FiveLtTwo\n    Statement: 5 < 2.\n    Proof: lia.";
    assert!(
        surface_c(false_goal).is_err_and(|e| e.contains("did not certify")),
        "a FALSE statement must be refused by the conclusion gate, not accepted because a failed \
         tactic still yields a well-typed error derivation: {:?}",
        surface_c(false_goal)
    );

    // GUARD — a theorem with no `Proof:` line is registered as `name : Prop := <statement>`. It
    // records the proposition and proves NOTHING, and it returns `Ok`.
    let unproved = "## Theorem: SimpleTheorem\n    Statement: True implies True.";
    assert!(
        surface_c(unproved).is_ok(),
        "a `Statement:` with no `Proof:` is a delta-transparent ABBREVIATION, accepted and unproved \
         — pinned so `Ok` is never read as `proved` for a block that stated no proof"
    );

    // GUARD — comparison is non-associative; a Lean chain has no rendering here.
    let chained = "## Theorem: Chained\n    Statement: x < x < x.\n    Proof: lia.";
    assert!(
        surface_c(chained).is_err(),
        "`a < b < c` must not be accepted as a chain — the grammar parses at most one comparison, \
         and there is no `and` in the statement grammar to decompose it into: {:?}",
        surface_c(chained)
    );
}

/// **T12-009 · MUST · Gap.** The `Statement:` grammar has no binder form. No `forall`, no `∀`, no
/// `for all` — the whole statement is one call to the literate `parse_term`, whose only two infix
/// words are `equals` and `implies`, and whose `implies` hard-codes its Pi binder to `"_"` so the
/// body can never mention the hypothesis.
///
/// **This is a WIRING gap, not a missing capability**: the kernel's own `interface::term_parser`
/// has `parse_forall` and parses `forall x : T, body` today. The literate parser simply has a
/// separate `parse_term` that never reaches it.
#[test]
fn a_literate_statement_binds_a_universally_quantified_variable() {
    // GUARD — the tempting erasure. Free variables in a `Statement:` are effectively closed by the
    // tactic, so dropping the binder is right for a bare prenex ∀ and catastrophic otherwise: an ∃
    // becomes a ∀, an antecedent's quantifier vanishes into the `"_"` binder, and alternating
    // quantifiers collapse entirely. Pinned: the ∃ spelling must not be silently accepted.
    let existential = "## Theorem: ExistsSucc\n    Statement: exists x, x > 0.\n    Proof: lia.";
    assert!(
        surface_c(existential).is_err(),
        "an existential must be REFUSED rather than read as a statement about a free variable — \
         binder-dropping turns `∃` into an implicit `∀`, an unsound strengthening that still \
         typechecks: {:?}",
        surface_c(existential)
    );

    // GUARD — the lambda syntax is not an escape hatch. `given x: Nat yields …` is handled by the
    // DEFINITION parser and is unreachable from `Statement:`.
    let lambda_smuggle =
        "## Theorem: Smuggled\n    Statement: given x: Nat yields x equals x.\n    Proof: refl.";
    assert!(
        surface_c(lambda_smuggle).is_err(),
        "the term-level lambda binders are not reachable from `Statement:`: {:?}",
        surface_c(lambda_smuggle)
    );

    // THE CAPABILITY.
    let quantified =
        "## Theorem: AddZeroRight\n    Statement: forall n : Nat, (add n 0) equals n.\n    Proof: ring.";
    assert!(
        surface_c(quantified).is_ok(),
        "Surface C must be able to BIND a quantified variable in its `Statement:`. It has real terms \
         — application, infix arithmetic, `equals` — and no quantifier at all, so every ∀-theorem \
         must currently be written with its binder erased and hoped over. This is a WIRING gap, not \
         a missing capability: `interface::term_parser::parse_forall` already parses \
         `forall x : T, body`; the literate parser's own `parse_term` never routes to it. Got: {:?}",
        surface_c(quantified)
    );
}

/// **T12-010 · MUST · Gap.** Literate `induction` is pinned to `Nat` and discards the variable —
/// and, worse, it can never succeed on any goal at all.
///
/// `build_induction_derivation(&self, _var_name: &str, …)` never reads the variable and hard-codes
/// the inductive type to the literal `SName "Nat"`; that much the corpus found. The part that is ours:
/// it builds the motive as `term_to_syntax(statement, &[])` with **no `SLam` binder**, while
/// `try_delim_conclude` extracts the motive body through `extract_slam_body`, which returns `None`
/// for anything that is not an `SLam` application and makes the whole derivation conclude the
/// `SName "Error"` sentinel. The REPL's conclusion gate then refuses it. So there is no goal, of any
/// type, that a literate `induction` proof can close.
#[test]
fn literate_induction_over_nat_closes_its_two_bullet_cases() {
    // The variable token is REQUIRED to parse even though its value is discarded.
    let no_variable = "## Theorem: NoVar\n    Statement: x equals x.\n    Proof: induction.";
    assert!(
        surface_c(no_variable).is_err(),
        "`induction` with no variable must not parse — the token is required even though \
         `_var_name` is never read: {:?}",
        surface_c(no_variable)
    );

    // Bullet count is checked against `Nat`'s two constructors, whatever the intended type.
    let three_bullets = "## Theorem: ThreeBullets\n    Statement: x equals x.\n    \
                         Proof: induction n.\n    - refl.\n    - refl.\n    - refl.";
    assert!(
        surface_c(three_bullets).is_err(),
        "a three-bullet proof must be refused — the count is checked against Nat's constructors, \
         which is the only way the hard-coded type becomes observable: {:?}",
        surface_c(three_bullets)
    );

    // THE CAPABILITY. Two bullets over a Nat goal: the one shape for which the hard-coded type is
    // correct, and the shape every `induction n with | zero | succ` in Lean maps onto.
    let nat_induction = "## Theorem: NatInduction\n    Statement: x equals x.\n    \
                         Proof: induction n.\n    - refl.\n    - refl.";
    assert!(
        surface_c(nat_induction).is_ok(),
        "a two-bullet literate induction over Nat must certify. It cannot — and not because of the \
         goal: `build_induction_derivation` builds the motive with NO `SLam` binder, while \
         `try_delim_conclude` reads it through `extract_slam_body`, which requires one. The motive \
         extraction therefore always fails, the derivation always concludes `SName \"Error\"`, and \
         the REPL's conclusion gate always refuses. Literate `induction` can close NO goal of ANY \
         type. Got: {:?}",
        surface_c(nat_induction)
    );
}

// ===================== SYNTHESIS — ROUTING, DEAD VARIANTS, TRUST, ENGINES =====================

/// **T12-011 · MUST · Native.** *The single most important sentence in this tier, and it is wrong.*
///
/// The corpus's routing policy rests on: *"The intersection `quantifier AND function term` is empty.
/// `∀ n : Nat, n + 0 = n` — the first theorem anyone writes — is expressible on NO vernacular
/// surface. It is two lines on K."* Against today's source the intersection is NOT empty. Surface B
/// has both, on one line, and it certifies.
///
/// What is genuinely lost is the SORT: `for all n` carries no `: Nat`, and `Add`/`Mul`/`Sub` are the
/// ring vocabulary the oracle decides, not the `Nat` inductive. So the quantifier is untyped and the
/// proposition is the ring one. That is a real residue and it is named here — but it is a different
/// claim from "expressible on no surface", and it does not send the theorem to the raw kernel.
#[test]
fn the_quantifier_and_function_term_intersection_is_not_empty() {
    // Surface A now has BOTH the quantifier and a term grammar — that is T12-002, which this
    // campaign closed, so the assertion that used to read `!surface_a_certified(...)` has been
    // corrected rather than preserved. Denying a capability we ship is not a lock, it is a lie.
    assert!(
        surface_a_certified(
            "## Theorem: A_Attempt\n    Prove: For all n, n plus 0 equals n.\n    Proof: Auto.\n"
        ),
        "Surface A states and proves `∀ n, n + 0 = n` — English quantifier AND term grammar"
    );

    // Surface C likewise now takes a binder form — that is T12-009, also closed.
    assert!(
        surface_c(
            "## Theorem: C_Attempt\n    Statement: forall n : Nat, (add n 0) equals n.\n    Proof: ring."
        )
        .is_ok(),
        "Surface C states the same theorem with real terms under a real binder"
    );

    // Surface B has BOTH. This is the refutation.
    let both = parse_formula("for all n, Eq(Add(n, 0), n)")
        .expect("Surface B must parse a quantifier over a function term");
    assert!(
        matches!(&both, ProofExpr::ForAll { variable, body }
            if variable == "n"
            && matches!(body.as_ref(),
                ProofExpr::Identity(ProofTerm::Function(f, args), ProofTerm::Variable(v))
                if f == "Add"
                   && v == "n"
                   && matches!(&args[0], ProofTerm::Variable(a) if a == "n"))),
        "the parse must carry a BINDER over a FUNCTION TERM — a quantifier whose body applies a \
         function symbol, which is precisely the intersection claimed empty: {both:?}"
    );
    assert_eq!(
        surface_b("Theorem right_identity: prove for all n, Eq(Add(n, 0), n)."),
        vec![("right_identity".to_string(), true)],
        "`∀ n, n + 0 = n` must certify on the vernacular, from no axioms — this is the case's whole \
         content, and routing it to the raw kernel is now a CHOICE, not a necessity"
    );

    // And it is not one lucky statement: the shape generalises across the ring vocabulary.
    for (name, decl) in [
        ("add_comm", "Theorem add_comm: prove for all a b, Eq(Add(a, b), Add(b, a))."),
        ("mul_one", "Theorem mul_one: prove for all n, Eq(Mul(n, 1), n)."),
        (
            "distributivity",
            "Theorem distributivity: prove for all a b c, Eq(Mul(a, Add(b, c)), Add(Mul(a, b), Mul(a, c))).",
        ),
        ("sub_add", "Theorem sub_add: prove for all n, Eq(Sub(Add(n, 1), 1), n)."),
    ] {
        assert_eq!(
            surface_b(decl),
            vec![(name.to_string(), true)],
            "`{name}` must certify — the quantified-arithmetic fragment is general, not a special case"
        );
    }

    // THE RESIDUE, pinned so the win is not overstated. `Zero` is an uppercase-leading CONSTANT, not
    // the numeral, so the spelling a translator would reach for from Lean's `Nat.zero` does not
    // prove. The binder is untyped and the arithmetic is the ring's, not the `Nat` inductive's.
    assert_eq!(
        surface_b("Theorem named_zero: prove for all n, Eq(Add(n, Zero), n)."),
        vec![("named_zero".to_string(), false)],
        "`Zero` spelled as a name is an opaque CONSTANT, not the numeral — the binder carries no \
         sort, so nothing on this surface says which zero is meant"
    );
}

/// **T12-012 · MUST · Gap.** `ProofStrategy` offers a four-way classification of proofs — `Auto`,
/// `Induction(String)`, `ByRule(String)`, `Script(String)` — which maps neatly onto Lean's
/// `by decide` / `by induction` / `by exact` / general tactic blocks, and two of the four branches
/// can never be exercised. `Induction` and `ByRule` are constructed nowhere in the workspace, and no
/// driver has an arm for either; both carry doc comments with worked examples that an implementer
/// will copy.
///
/// `Induction` is T12-005's subject. This test owns `ByRule`, whose documented example fails
/// *differently and more quietly*: it is swallowed into a `Script` and dies as an unknown tactic.
#[test]
fn a_proof_line_may_name_the_inference_rule_that_discharges_it() {
    let by_rule = "## Theorem: By_Modus_Ponens\n\
                   Given: If Bob is happy then Bob is sad.\n\
                   Given: Bob is happy.\n\
                   Prove: Bob is sad.\n\
                   Proof: ModusPonens.\n";

    // Today's behaviour, pinned: the rule name is collected as script text, and the failure talks
    // about tactics rather than about strategies.
    assert!(
        surface_a(by_rule).is_err_and(|e| e.contains("unknown tactic")),
        "the dead `ByRule` spelling must at least fail LOUDLY: {:?}",
        surface_a(by_rule)
    );
    // The live sibling proves the theorem itself is not at fault.
    assert!(
        surface_a_certified(&by_rule.replace("Proof: ModusPonens.", "Proof: Auto.")),
        "the same theorem must certify under `Auto`, so the failure above is the SPELLING"
    );

    assert_eq!(
        strategy_of(by_rule),
        ProofStrategy::ByRule("ModusPonens".to_string()),
        "a `Proof:` line naming an inference rule must reach `ProofStrategy::ByRule` — the variant \
         exists, is public, and carries `Proof: ModusPonens.` as its own documented example, but \
         nothing in the workspace constructs it and no driver has an arm for it. It is swallowed \
         into `Script` instead, where `canonical` has no arm for a rule name. A consumer matching on \
         this public enum must still handle all four variants, so treating the dead pair as \
         `unreachable!()` is a latent panic"
    );
}

/// **T12-013 · MUST · Native.** THE trust door. A derivation is not a proof, and each entry point
/// fails to say so in its own way — so this pins all three faces of the predicate at once.
///
/// `verified == true` holds *iff* a proof term was certified AND re-checked by the kernel AND its
/// inferred type IS the goal's type. `derivation`, `proof_term` and `verified` are separate fields
/// precisely because they come apart.
#[test]
fn the_trust_door_separates_a_kernel_certificate_from_a_bare_derivation() {
    // (1) The two fields come apart. An oracle leaf is deliberately non-certifiable — Z3 attests
    // satisfiability and produces no checkable term — so the door returns the derivation it found
    // and refuses to call it verified.
    let fact = ProofExpr::Atom("SomeArithmeticFact".to_string());
    let oracle_only = DerivationTree::leaf(
        fact.clone(),
        InferenceRule::OracleVerification("Verified by Z3".to_string()),
    );
    let vp = check_derivation(&[fact.clone()], &fact, oracle_only);
    assert!(
        vp.derivation.is_some(),
        "the door must hand back the derivation it was given, so the two states are distinguishable"
    );
    assert!(
        !vp.verified,
        "an oracle-discharged derivation must NEVER be reported as verified — it is exactly the \
         state a harness whose pass predicate is `result.is_ok()` would score as a proof"
    );
    assert!(
        vp.proof_term.is_none(),
        "the checkable artefact must be absent exactly when `verified` is false"
    );

    // (2) The string door. `"Proved"` is not the predicate; `"[kernel-verified]"` is the only
    // substring that means what a conformance harness needs it to mean.
    let real = "## Theorem: Certifiable\n\
                Given: If Bob is happy then Bob is sad.\n\
                Given: Bob is happy.\n\
                Prove: Bob is sad.\n\
                Proof: Auto.\n";
    let out = surface_a(real).expect("a certifiable theorem returns Ok");
    assert!(
        out.contains(CERTIFIED),
        "a kernel-certified theorem must carry the marker that distinguishes it from the \
         derivation-found-but-uncertified arm, which also returns `Ok`: {out}"
    );

    // (3) The structured door, and the other half of the trap. `compile_theorem_for_ui` reports
    // `error: None` for a theorem it simply could not prove — so "the call succeeded" and "no error
    // was reported" are BOTH false success predicates. Only `verified` is the predicate.
    let unprovable = "## Theorem: Unprovable\n\
                      Given: Bob is a trip.\n\
                      Given: Bob is in Maine.\n\
                      Prove: Someone is in Maine.\n\
                      Proof: Auto.\n";
    let r = compile_theorem_for_ui(unprovable);
    assert!(
        !r.verified,
        "the goal was not proved, so `verified` must be false"
    );
    assert!(
        r.error.is_none(),
        "and `error` must be None even so — pinned because it is the field a harness reaches for \
         first, and it says nothing about whether anything was proved: {:?}",
        r.error
    );
}

/// **T12-014 · DIVERGE · Native.** A finite-domain grid reroutes `Auto` to the incremental certified
/// grid solver whenever the premises take the form — a declared bijection or a disjunctive closure —
/// with no flag and no syntax. Both routes pass the SAME kernel door, so the VERDICT is identical;
/// what changes is the proof OBJECT and the premise set it is stated over.
///
/// We diverge deliberately and defend it (see the verdict file's `why`). What this test pins is the
/// consequence a harness must respect: the same emitted text proved through the two public entry
/// points yields two structurally different certificates, so a regression suite that snapshots
/// rendered trees is measuring premise structure it did not choose.
#[test]
fn the_grid_fast_path_changes_the_proof_object_not_the_verdict() {
    let simon = "## Theorem: Simon\n\
                 Given: Alpha is a trip.\n\
                 Given: Beta is a trip.\n\
                 Given: Alpha is not Beta.\n\
                 Given: Every trip is in 2003 or in 2004.\n\
                 Given: Every trip is in Florida or in Maine.\n\
                 Given: Exactly one trip is in 2003.\n\
                 Given: Exactly one trip is in Florida.\n\
                 Given: Alpha is in 2003.\n\
                 Given: Alpha is in Florida.\n\
                 Prove: Beta is in Maine.\n\
                 Proof: Auto.\n";

    let ui = compile_theorem_for_ui(simon);
    assert!(
        ui.verified,
        "the grid cell must certify: {:?}",
        ui.verification_error
    );
    let grid_tree = ui
        .derivation
        .as_ref()
        .expect("a verified grid cell yields a derivation")
        .display_tree();
    assert!(
        ui.grid.is_some(),
        "the solved table must be attached whenever the premises take the grid form — no flag, the \
         structure alone triggers it"
    );

    let lang = surface_a(simon).expect("the same text must also prove through `compile_theorem`");
    assert!(
        lang.contains(CERTIFIED),
        "the general entry point must reach the same VERDICT, kernel-certified: {lang}"
    );

    // The divergence, stated as the falsifiable thing it is.
    assert!(
        grid_tree.contains("DisjunctionElim") || grid_tree.contains("Contradiction"),
        "the grid route reasons over GROUNDED, quantifier-free premises — closure elimination to a \
         contradiction: {grid_tree}"
    );
    assert!(
        !grid_tree.contains("UniversalInst"),
        "…and therefore never instantiates a quantifier, because the premises it was handed have \
         none left: {grid_tree}"
    );
    assert!(
        lang.contains("UniversalInst"),
        "whereas `compile_theorem` has no grid branch at all and proves the SAME theorem over the \
         quantified originals: {lang}"
    );

    // The trigger is structural and nothing about the theorem announces it: the syllogism below is
    // the same size and shape and keeps its ∀-instantiation trace through the very same entry point.
    let socrates = "## Theorem: Socrates\n\
                    Given: Every man is mortal.\n\
                    Given: Socrates is a man.\n\
                    Prove: Socrates is mortal.\n\
                    Proof: Auto.\n";
    let plain = compile_theorem_for_ui(socrates);
    assert!(
        plain.verified,
        "Socrates must prove: {:?}",
        plain.verification_error
    );
    assert!(
        plain.grid.is_none(),
        "no closure and no `exactly one` premise means no grid — one added premise of either form \
         would flip the engine under an unchanged theorem"
    );
}
