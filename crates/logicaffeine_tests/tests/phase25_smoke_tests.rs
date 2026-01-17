use logicaffeine_language::{compile, compile_all_scopes};

/// SMOKE TEST 1: Adverb Scope (Montague)
/// "John almost killed Mary."
///
/// CURRENT (WRONG):  ∃e(Kill(e) ∧ Agent(e, J) ∧ Theme(e, M) ∧ Almost(e))
///                   This implies the killing happened!
///
/// REQUIRED (CORRECT): Almost(∃e(Kill(e) ∧ Agent(e, J) ∧ Theme(e, M)))
///                     The "Almost" operator must wrap the existential quantifier
#[test]
fn test_smoke_01_adverb_scope_almost() {
    let input = "John almost killed Mary.";
    let output = compile(input).expect("Should parse");

    println!("Output: {}", output);

    assert!(
        output.starts_with("Almost("),
        "Scopal adverb must wrap the entire expression. Got: {}",
        output
    );
    assert!(
        !output.contains("∧ Almost"),
        "Scopal adverb must NOT be a flat event predicate. Got: {}",
        output
    );
}

/// SMOKE TEST 2: Negation Scope (Shakespeare/Council)
/// "Every student is not happy."
///
/// CURRENT (SURFACE ONLY): ∀x(Student(x) → ¬Happy(x))
///                         "No student is happy"
///
/// REQUIRED (BOTH READINGS):
///   - Surface: ∀x(Student(x) → ¬Happy(x))
///   - Inverse: ¬∀x(Student(x) → Happy(x)) "Not every student is happy"
#[test]
fn test_smoke_02_negation_scope_ambiguity() {
    let input = "Every student is not happy.";
    let results = compile_all_scopes(input).expect("Should parse");

    println!("Results: {:?}", results);

    assert!(
        results.len() >= 2,
        "Should produce at least 2 scope readings (surface and inverse). Got {} reading(s): {:?}",
        results.len(),
        results
    );

    let has_surface = results.iter().any(|r| {
        let forall_pos = r.find('∀').unwrap_or(999);
        let not_pos = r.find('¬').unwrap_or(0);
        forall_pos < not_pos
    });

    let has_inverse = results.iter().any(|r| {
        let forall_pos = r.find('∀').unwrap_or(0);
        let not_pos = r.find('¬').unwrap_or(999);
        not_pos < forall_pos
    });

    assert!(has_surface, "Should include surface reading (∀ before ¬): {:?}", results);
    assert!(has_inverse, "Should include inverse reading (¬ before ∀): {:?}", results);
}

/// SMOKE TEST 3: Donkey Sentences / Dynamic Binding (Russell)
/// "Every farmer who owns a donkey beats it."
///
/// REQUIRED: ∀x∀y((Farmer(x) ∧ Donkey(y) ∧ Own(x, y)) → Beat(x, y))
///           Requires DRT or scope hoisting for 'it' to bind to 'donkey'
#[test]
fn test_smoke_03_donkey_anaphora() {
    let input = "Every farmer who owns a donkey beats it.";
    let output = compile(input).expect("Should parse");

    println!("Output: {}", output);

    assert!(
        !output.contains("?"),
        "Anaphora resolution failed - contains unbound variable marker. Got: {}",
        output
    );
    assert!(
        output.contains("→") || output.contains("->"),
        "Should produce an implication structure. Got: {}",
        output
    );
}

/// SMOKE TEST 4: Intensional Identity (Frege)
/// "John seeks a unicorn and Mary seeks it."
///
/// REQUIRED: Seek(J, ^Unicorn) ∧ Seek(M, ^Unicorn)
///           Mary seeks the same *concept* of a unicorn, not a realized object
#[test]
fn test_smoke_04_intensional_identity() {
    let input = "John seeks a unicorn and Mary seeks it.";
    let output = compile(input).expect("Should parse");

    println!("Output: {}", output);

    assert!(
        output.contains("^Unicorn") || output.contains("^unicorn"),
        "Should use intensional notation (^) for unicorn in de dicto reading. Got: {}",
        output
    );
}

/// SMOKE TEST 5: Performatives (Austin)
/// "I promise to come."
///
/// REQUIRED: SpeechAct or explicit performative marking
///           This utterance *creates* the promise, not describes it
#[test]
fn test_smoke_05_performative() {
    let input = "I promise to come.";
    let output = compile(input).expect("Should parse");

    println!("Output: {}", output);

    let is_performative = output.contains("SpeechAct")
        || output.contains("Promising(")
        || output.contains("promise(speaker");

    assert!(
        is_performative,
        "Should recognize performative verb usage. Got: {}",
        output
    );
}

/// SMOKE TEST 6: Distanced Phrasal Verbs (Council Collective)
/// "John gave the book up."
///
/// REQUIRED: The phrasal verb "give up" (= surrender) should be recognized
///           even when the particle "up" is separated from the verb by the object
#[test]
fn test_smoke_06_distanced_phrasal_verb() {
    // First check contiguous version works
    let contiguous = compile("John gave up the book.").expect("Contiguous should parse");
    println!("Contiguous: {}", contiguous);

    // Now check distanced version
    let input = "John gave the book up.";
    let output = compile(input).expect("Should parse");

    println!("Distanced: {}", output);

    let recognizes_phrasal = output.contains("Surrender")
        || output.contains("GiveUp")
        || output.contains("Give_up");

    assert!(
        recognizes_phrasal,
        "Should collapse 'gave ... up' into phrasal verb meaning. Got: {}",
        output
    );
}

/// SMOKE TEST 7: Double Focus (Rooth)
/// "Only John eats only rice."
///
/// REQUIRED: Only(John, λx. Only(Rice, λy. Eat(x, y)))
///           Both focus operators should be represented with proper nesting
#[test]
fn test_smoke_07_double_focus() {
    let input = "Only John eats only rice.";
    let output = compile(input).expect("Should parse");

    println!("Output: {}", output);

    let focus_count = output.matches("Only").count();

    assert!(
        focus_count >= 2,
        "Should detect two focus operators. Found {} 'Only' occurrences in: {}",
        focus_count,
        output
    );
}

/// Additional test: Verify scopal adverb works with intransitive verbs too
#[test]
fn test_smoke_01b_adverb_scope_intransitive() {
    let input = "John almost died.";
    let output = compile(input).expect("Should parse");

    println!("Output: {}", output);

    assert!(
        output.starts_with("Almost("),
        "Scopal adverb must wrap intransitive event too. Got: {}",
        output
    );
}

/// Additional test: "barely" is also a scopal adverb
#[test]
fn test_smoke_01c_adverb_scope_barely() {
    let input = "Mary barely escaped.";
    let output = compile(input).expect("Should parse");

    println!("Output: {}", output);

    assert!(
        output.starts_with("Barely("),
        "Scopal adverb 'barely' must wrap the expression. Got: {}",
        output
    );
}
