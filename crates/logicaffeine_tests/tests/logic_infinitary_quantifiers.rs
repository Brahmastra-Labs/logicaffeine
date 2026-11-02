//! The infinitary quantifiers of Logic mode: `infinitely many` (∃^∞, Mostowski's quantifier),
//! `finitely many` (∃^<∞) and `all but finitely many` (∀^∞, the cofinite quantifier), with the
//! temporal `infinitely often` (G F) and the relation `infinitely close to`. Each is read from
//! English in every construction a determiner heads, printed in every output format, and lowered
//! to the proof engine over the order of the naturals: `∃^∞x φ` is `∀b ∃x (x > b ∧ φ)`, `∀^∞x φ`
//! is `∃b ∀x (x > b → φ)`, and `∃^<∞x φ` is `∃b ∀x (x > b → ¬φ)`.

use logicaffeine_compile::theorem_proof_exprs;
use logicaffeine_language::token::TokenType;
use logicaffeine_language::{
    compile, compile_kripke, compile_simple, compile_theorem, compile_with_options, CompileOptions,
    Interner, Lexer, OutputFormat,
};

fn latex(sentence: &str) -> String {
    compile_with_options(
        sentence,
        CompileOptions {
            format: OutputFormat::LaTeX,
            pragmatic: false,
        },
    )
    .unwrap_or_else(|e| panic!("{sentence} does not parse: {e:?}"))
}

fn unicode(sentence: &str) -> String {
    compile(sentence).unwrap_or_else(|e| panic!("{sentence} does not parse: {e:?}"))
}

fn tokens(sentence: &str) -> Vec<TokenType> {
    let mut interner = Interner::new();
    Lexer::new(sentence, &mut interner)
        .tokenize()
        .into_iter()
        .map(|t| t.kind)
        .collect()
}

fn goal(sentence: &str) -> String {
    let (_, goal) =
        theorem_proof_exprs(&format!("## Theorem: T\nProve: {sentence}\nProof: Auto.\n"))
            .unwrap_or_else(|e| panic!("{sentence} does not parse as a goal: {e:?}"));
    goal.to_string()
}

#[test]
fn each_infinitary_determiner_is_one_token() {
    assert!(matches!(
        tokens("Infinitely many dogs bark.")[0],
        TokenType::InfinitelyMany
    ));
    assert!(matches!(
        tokens("infinitely many dogs bark.")[0],
        TokenType::InfinitelyMany
    ));
    assert!(matches!(
        tokens("Finitely many dogs bark.")[0],
        TokenType::FinitelyMany
    ));
    assert!(matches!(
        tokens("Only finitely many dogs bark.")[0],
        TokenType::FinitelyMany
    ));
    assert!(matches!(
        tokens("All but finitely many dogs bark.")[0],
        TokenType::AllButFinitelyMany
    ));
    assert!(matches!(
        tokens("all but finitely many dogs bark.")[0],
        TokenType::AllButFinitelyMany
    ));
    assert_eq!(
        tokens("All but finitely many dogs bark.").len(),
        tokens("Most dogs bark.").len(),
        "the three words are one determiner"
    );
}

#[test]
fn the_words_alone_keep_their_ordinary_reading() {
    assert!(!tokens("The light flashes infinitely.")
        .iter()
        .any(|t| matches!(t, TokenType::InfinitelyMany)));
    assert!(matches!(tokens("All dogs bark.")[0], TokenType::All));
    assert!(matches!(tokens("Many dogs bark.")[0], TokenType::Many));
    assert_eq!(
        unicode("Many dogs bark."),
        "MANY x((Dogs(x) ∧ ∃e(Bark(e) ∧ Agent(e, x))))"
    );
    assert_eq!(
        unicode("All dogs bark."),
        "∀x((Dogs(x) → ∃e(Bark(e) ∧ Agent(e, x))))"
    );
}

#[test]
fn infinitely_many_prints_as_mostowskis_quantifier() {
    assert_eq!(
        unicode("Infinitely many dogs bark."),
        "∃^∞x((Dogs(x) ∧ ∃e(Bark(e) ∧ Agent(e, x))))"
    );
    assert_eq!(
        unicode("Infinitely many primes are odd."),
        "∃^∞x((Primes(x) ∧ Odd(x)))"
    );
}

#[test]
fn finitely_many_prints_as_its_negation_shape() {
    assert_eq!(
        unicode("Finitely many dogs bark."),
        "∃^<∞x((Dogs(x) ∧ ∃e(Bark(e) ∧ Agent(e, x))))"
    );
    assert_eq!(
        unicode("Only finitely many dogs bark."),
        unicode("Finitely many dogs bark.")
    );
    assert_eq!(
        unicode("Finitely many primes are odd."),
        "∃^<∞x((Primes(x) ∧ Odd(x)))"
    );
}

#[test]
fn all_but_finitely_many_restricts_by_implication() {
    assert_eq!(
        unicode("All but finitely many dogs bark."),
        "∀^∞x((Dogs(x) → ∃e(Bark(e) ∧ Agent(e, x))))"
    );
    assert_eq!(
        unicode("All but finitely many primes are odd."),
        "∀^∞x((Primes(x) → Odd(x)))"
    );
}

#[test]
fn an_infinitary_object_scopes_under_its_verb() {
    assert_eq!(
        unicode("Every dog chases infinitely many cats."),
        "∀x((((Dog(x) ∧ Animal(x)) ∧ Mammal(x)) → ∃^∞y((Cats(y) ∧ ∃e(Chase(e) ∧ Agent(e, x) ∧ Theme(e, y))))))"
    );
    assert_eq!(
        unicode("Every dog chases all but finitely many cats."),
        "∀x((((Dog(x) ∧ Animal(x)) ∧ Mammal(x)) → ∀^∞y((Cats(y) → ∃e(Chase(e) ∧ Agent(e, x) ∧ Theme(e, y))))))"
    );
}

#[test]
fn latex_writes_the_superscript_infinity() {
    assert_eq!(
        latex("Infinitely many dogs bark."),
        "\\exists^{\\infty} x((D(x) \\cdot \\exists e(Bark(e) \\cdot Agent(e, x))))"
    );
    assert_eq!(
        latex("Finitely many dogs bark."),
        "\\exists^{<\\infty} x((D(x) \\cdot \\exists e(Bark(e) \\cdot Agent(e, x))))"
    );
    assert_eq!(
        latex("All but finitely many dogs bark."),
        "\\forall^{\\infty} x((D(x) \\supset \\exists e(Bark(e) \\cdot Agent(e, x))))"
    );
}

#[test]
fn simple_and_kripke_formats_name_the_quantifier() {
    assert_eq!(
        compile_simple("Infinitely many dogs bark.").unwrap(),
        "∃^∞x((Dogs(x) ∧ Bark(x)))"
    );
    assert_eq!(
        compile_simple("Finitely many dogs bark.").unwrap(),
        "∃^<∞x((Dogs(x) ∧ Bark(x)))"
    );
    assert_eq!(
        compile_simple("All but finitely many dogs bark.").unwrap(),
        "∀^∞x((Dogs(x) → Bark(x)))"
    );
    assert_eq!(
        compile_kripke("Infinitely many dogs bark.").unwrap(),
        "Exists^inf x((Dogs(x, w0)  And  Exists e(Bark(e, w0)  And  Agent(e, x, w0))))"
    );
    assert_eq!(
        compile_kripke("Finitely many dogs bark.").unwrap(),
        "Exists^<inf x((Dogs(x, w0)  And  Exists e(Bark(e, w0)  And  Agent(e, x, w0))))"
    );
    assert_eq!(
        compile_kripke("All but finitely many dogs bark.").unwrap(),
        "ForAll^inf x((Dogs(x, w0)  Implies  Exists e(Bark(e, w0)  And  Agent(e, x, w0))))"
    );
}

#[test]
fn the_math_layer_binds_infinitary_quantifiers() {
    assert_eq!(
        unicode("For infinitely many n, n plus 0 equals n."),
        "∃^∞n(Add(n, 0) = n)"
    );
    assert_eq!(
        unicode("For finitely many n, n plus 1 equals n."),
        "∃^<∞n(Add(n, 1) = n)"
    );
    assert_eq!(
        unicode("For only finitely many n, n plus 1 equals n."),
        "∃^<∞n(Add(n, 1) = n)"
    );
    assert_eq!(
        unicode("For all but finitely many n, n plus 0 equals n."),
        "∀^∞n(Add(n, 0) = n)"
    );
    assert_eq!(
        latex("For all but finitely many n, n plus 0 equals n."),
        "\\forall^{\\infty} n(A(n, 0) = n)"
    );
}

#[test]
fn infinitely_many_lowers_to_unbounded_existence() {
    let some = goal("Some dogs bark.");
    let body = some
        .strip_prefix("∃x ")
        .unwrap_or_else(|| panic!("`some` lowers to ∃x: {some}"));
    assert_eq!(
        goal("Infinitely many dogs bark."),
        format!("∀x_bound ∃x (Gt(x, x_bound) ∧ {body})")
    );
}

#[test]
fn finitely_many_lowers_to_a_bound_past_which_none_hold() {
    let some = goal("Some dogs bark.");
    let body = some
        .strip_prefix("∃x ")
        .unwrap_or_else(|| panic!("`some` lowers to ∃x: {some}"));
    assert_eq!(
        goal("Finitely many dogs bark."),
        format!("∃x_bound ∀x (Gt(x, x_bound) → ¬{body})")
    );
}

#[test]
fn all_but_finitely_many_lowers_to_a_bound_past_which_all_hold() {
    let all = goal("All dogs bark.");
    let body = all
        .strip_prefix("∀x ")
        .unwrap_or_else(|| panic!("`all` lowers to ∀x: {all}"));
    assert_eq!(
        goal("All but finitely many dogs bark."),
        format!("∃x_bound ∀x (Gt(x, x_bound) → {body})")
    );
}

#[test]
fn the_math_layer_lowers_over_the_naturals() {
    assert_eq!(
        goal("For infinitely many n, n plus 0 equals n."),
        "∀n_bound ∃n (Gt(n, n_bound) ∧ Add(n, 0) = n)"
    );
    assert_eq!(
        goal("For all but finitely many n, n plus 0 equals n."),
        "∃n_bound ∀n (Gt(n, n_bound) → Add(n, 0) = n)"
    );
}

#[test]
fn finitely_many_witnesses_never_prove_infinitely_many() {
    for premises in [
        "Given: Rex is a dog.\nGiven: Rex barks.\n",
        "Given: Some dogs bark.\n",
        "Given: Three dogs bark.\n",
        "Given: Finitely many dogs bark.\n",
    ] {
        let theorem =
            format!("## Theorem: T\n{premises}Prove: Infinitely many dogs bark.\nProof: Auto.\n");
        let proved = compile_theorem(&theorem);
        assert!(
            !matches!(&proved, Ok(text) if text.contains("Proved")),
            "{premises} must not prove that infinitely many dogs bark: {proved:?}"
        );
    }
}

#[test]
fn infinitely_often_is_always_eventually() {
    let always_eventually = unicode("Always, eventually, the light flashes.");
    assert!(always_eventually.starts_with("G(F("), "{always_eventually}");
    assert_eq!(
        unicode("Infinitely often, the light flashes."),
        always_eventually
    );
    assert_eq!(
        unicode("The light flashes infinitely often."),
        always_eventually
    );
    assert_eq!(
        latex("The light flashes infinitely often."),
        latex("Always, eventually, the light flashes.")
    );
}

#[test]
fn eventually_always_is_the_dual() {
    assert!(unicode("Eventually, always, the light flashes.").starts_with("F(G("));
}

#[test]
fn infinitely_close_to_is_a_relation() {
    assert_eq!(
        unicode("The cat is infinitely close to the dog."),
        unicode("The cat is near the dog.").replace("Near(", "InfinitelyClose(")
    );
    assert!(unicode("The cat is infinitely close to the dog.").contains("InfinitelyClose(x, Dog)"));
}
