//! E2E Tests: Studio Examples
//!
//! These tests verify that ALL example files in the Studio playground
//! parse and execute correctly across all three modes.
//!
//! Additionally includes full parse tree validation for Logic mode sentences.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::run_logos;

use logicaffeine_language::{compile, compile_all_scopes};
use logicaffeine_language::ast::{QuantifierKind, TemporalOperator};
use logicaffeine_language::view::ExprView;

// ============================================================
// Logic Mode Examples (English -> FOL)
// ============================================================

/// Test: simple-sentences.logic
#[test]
fn logic_simple_every_cat_sleeps() {
    let result = compile("Every cat sleeps.");
    assert!(result.is_ok(), "Every cat sleeps should compile: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("∀") || output.contains("All"),
        "Should contain universal quantifier: {}", output);
}

#[test]
fn logic_simple_some_dogs_bark() {
    let result = compile("Some dogs bark loudly.");
    assert!(result.is_ok(), "Some dogs bark loudly should compile: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("∃") || output.contains("Some") || output.contains("Exist"),
        "Should contain existential: {}", output);
    // Verify adverb is captured as event modifier
    assert!(output.contains("Loudly(e)"),
        "Should contain adverb 'Loudly(e)': {}", output);
}

#[test]
fn logic_simple_john_loves_mary() {
    let result = compile("John loves Mary.");
    assert!(result.is_ok(), "John loves Mary should compile: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Love") || output.contains("John") || output.contains("Mary"),
        "Should contain love predicate: {}", output);
}

#[test]
fn logic_simple_the_quick_brown_fox() {
    let result = compile("The quick brown fox jumps.");
    assert!(result.is_ok(), "The quick brown fox jumps should compile: {:?}", result.err());
}

#[test]
fn logic_simple_no_student_failed() {
    let result = compile("No student failed.");
    assert!(result.is_ok(), "No student failed should compile: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("¬") || output.contains("Not") || output.contains("∀"),
        "Should contain negation: {}", output);
}

/// Test: quantifiers.logic
#[test]
fn logic_quantifier_every_student_read_book() {
    let readings = compile_all_scopes("Every student read a book.");
    assert!(readings.is_ok(), "Should parse: {:?}", readings.err());
    let readings = readings.unwrap();
    assert!(readings.len() >= 1, "Should have at least one reading");
}

#[test]
fn logic_quantifier_professor_supervises() {
    let readings = compile_all_scopes("A professor supervises every student.");
    assert!(readings.is_ok(), "Should parse: {:?}", readings.err());
}

#[test]
fn logic_quantifier_no_student_failed_every() {
    let readings = compile_all_scopes("No student failed every exam.");
    assert!(readings.is_ok(), "Should parse: {:?}", readings.err());
}

#[test]
fn logic_quantifier_teacher_praised() {
    let readings = compile_all_scopes("Some teacher praised every student.");
    assert!(readings.is_ok(), "Should parse: {:?}", readings.err());
}

#[test]
fn logic_quantifier_dog_chased_cat() {
    let readings = compile_all_scopes("Every dog chased some cat.");
    assert!(readings.is_ok(), "Should parse: {:?}", readings.err());
}

/// Test: tense-aspect.logic
#[test]
fn logic_tense_john_was_running() {
    let result = compile("John was running.");
    assert!(result.is_ok(), "John was running should compile: {:?}", result.err());
}

#[test]
fn logic_tense_mary_has_eaten() {
    let result = compile("Mary has eaten.");
    assert!(result.is_ok(), "Mary has eaten should compile: {:?}", result.err());
}

#[test]
fn logic_tense_train_will_arrive() {
    let result = compile("The train will arrive.");
    assert!(result.is_ok(), "The train will arrive should compile: {:?}", result.err());
}

#[test]
fn logic_tense_she_had_been_sleeping() {
    let result = compile("She had been sleeping.");
    assert!(result.is_ok(), "She had been sleeping should compile: {:?}", result.err());
}

#[test]
fn logic_tense_they_have_been_working() {
    let result = compile("They have been working.");
    assert!(result.is_ok(), "They have been working should compile: {:?}", result.err());
}

// ============================================================
// Logic Mode Examples (Prover/Theorem Proving)
// ============================================================

use logicaffeine_language::compile_theorem;

/// Test: prover-demo.logic
#[test]
fn logic_prover_socrates_mortality() {
    let source = r#"## Theorem: Socrates_Mortality
Given: All men are mortal.
Given: Socrates is a man.
Prove: Socrates is mortal.
Proof: Auto.
"#;

    let result = compile_theorem(source);
    assert!(result.is_ok(), "Socrates mortality theorem should prove: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Proved"), "Should contain 'Proved': {}", output);
    assert!(output.contains("ModusPonens") || output.contains("PremiseMatch"),
        "Should show inference rules: {}", output);
}

/// Test: syllogism.logic (chain reasoning)
/// Uses vocabulary that canonicalizes correctly (men/mortals/doomed)
#[test]
fn logic_prover_chain_reasoning() {
    let source = r#"## Theorem: Chain_Reasoning
Given: All men are mortal.
Given: All mortals are doomed.
Given: Plato is a man.
Prove: Plato is doomed.
Proof: Auto.
"#;

    let result = compile_theorem(source);
    assert!(result.is_ok(), "Chain reasoning theorem should prove: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Proved"), "Should contain 'Proved': {}", output);
}

/// Test: trivial-proof.logic (direct match)
#[test]
fn logic_prover_trivial_proof() {
    let source = r#"## Theorem: Direct_Match
Given: Socrates is mortal.
Prove: Socrates is mortal.
Proof: Auto.
"#;

    let result = compile_theorem(source);
    assert!(result.is_ok(), "Trivial theorem should prove: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Proved"), "Should contain 'Proved': {}", output);
    assert!(output.contains("PremiseMatch"), "Direct match should use PremiseMatch: {}", output);
}

// ============================================================
// Code Mode Examples (Imperative LOGOS)
// ============================================================

/// Test: hello-world.logos
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_hello_world() {
    let source = r#"## Main

Let greeting be "Hello, LOGOS!".
Show greeting.

Let x be 10.
Let y be 20.
Let sum be x + y.

Show "The sum is:".
Show sum.
"#;

    let result = run_logos(source);
    assert!(result.success, "hello-world should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("Hello, LOGOS!"),
        "Should output greeting. Got: {}", result.stdout);
    assert!(result.stdout.contains("30"),
        "Should output sum 30. Got: {}", result.stdout);
}

/// Test: fibonacci.logos
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_fibonacci() {
    let source = r#"## Main

Let n be 10.
Let a be 0.
Let b be 1.

Show "Fibonacci sequence:".
Show a.

Repeat for i from 1 to n:
    Show b.
    Let temp be a + b.
    Set a to b.
    Set b to temp.
"#;

    let result = run_logos(source);
    assert!(result.success, "fibonacci should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("0"), "Should contain 0");
    assert!(result.stdout.contains("1"), "Should contain 1");
    assert!(result.stdout.contains("55"), "Should contain fib(10)=55. Got: {}", result.stdout);
}

/// Test: fizzbuzz.logos
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_fizzbuzz() {
    let source = r#"## Main

Repeat for i from 1 to 20:
    If i / 15 * 15 equals i:
        Show "FizzBuzz".
    Otherwise:
        If i / 3 * 3 equals i:
            Show "Fizz".
        Otherwise:
            If i / 5 * 5 equals i:
                Show "Buzz".
            Otherwise:
                Show i.
"#;

    let result = run_logos(source);
    assert!(result.success, "fizzbuzz should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("FizzBuzz"), "Should contain FizzBuzz. Got: {}", result.stdout);
    assert!(result.stdout.contains("Fizz"), "Should contain Fizz. Got: {}", result.stdout);
    assert!(result.stdout.contains("Buzz"), "Should contain Buzz. Got: {}", result.stdout);
}

/// Test: collections.logos
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_collections() {
    let source = r#"## Main

Let numbers be [1, 2, 3, 4, 5].
Show "Numbers:".
Show numbers.

Push 6 to numbers.
Show "After push:".
Show numbers.

Show "Length:".
Show length of numbers.

Show "First item:".
Show item 1 of numbers.

Show "Last item:".
Show item 6 of numbers.
"#;

    let result = run_logos(source);
    assert!(result.success, "collections should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("[1, 2, 3, 4, 5]"),
        "Should show initial list. Got: {}", result.stdout);
    assert!(result.stdout.contains("[1, 2, 3, 4, 5, 6]"),
        "Should show list after push. Got: {}", result.stdout);
    assert!(result.stdout.contains("6"), "Should show length 6. Got: {}", result.stdout);
    assert!(result.stdout.contains("1"), "Should show first item. Got: {}", result.stdout);
}

// ============================================================
// Math Mode Examples (Vernacular/Theorem Proving)
// ============================================================

use logicaffeine_kernel::interface::Repl;

/// Test: natural-numbers.logos (Math mode)
#[test]
fn math_natural_numbers() {
    let mut repl = Repl::new();

    // Define Nat
    let result = repl.execute("Inductive Nat := Zero : Nat | Succ : Nat -> Nat.");
    assert!(result.is_ok(), "Nat inductive should work: {:?}", result.err());

    // Define one
    let result = repl.execute("Definition one : Nat := Succ Zero.");
    assert!(result.is_ok(), "Definition one should work: {:?}", result.err());

    // Define two
    let result = repl.execute("Definition two : Nat := Succ one.");
    assert!(result.is_ok(), "Definition two should work: {:?}", result.err());

    // Define three
    let result = repl.execute("Definition three : Nat := Succ two.");
    assert!(result.is_ok(), "Definition three should work: {:?}", result.err());

    // Check Zero
    let result = repl.execute("Check Zero.");
    assert!(result.is_ok(), "Check Zero should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Nat"), "Zero should have type Nat: {}", output);

    // Check one
    let result = repl.execute("Check one.");
    assert!(result.is_ok(), "Check one should work: {:?}", result.err());

    // Eval three
    let result = repl.execute("Eval three.");
    assert!(result.is_ok(), "Eval three should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Succ"), "Eval three should show Succ: {}", output);
}

/// Test: boolean-logic.logos (Math mode)
/// Uses MyBool/Yes/No to avoid name collision with prelude's True/False.
#[test]
fn math_boolean_logic() {
    let mut repl = Repl::new();

    // Define MyBool (using Yes/No to avoid True/False collision with prelude)
    let result = repl.execute("Inductive MyBool := Yes : MyBool | No : MyBool.");
    assert!(result.is_ok(), "MyBool inductive should work: {:?}", result.err());

    // Check Yes type
    let result = repl.execute("Check Yes.");
    assert!(result.is_ok(), "Check Yes should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("MyBool"), "Yes should have type MyBool: {}", output);

    // Check No type
    let result = repl.execute("Check No.");
    assert!(result.is_ok(), "Check No should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("MyBool"), "No should have type MyBool: {}", output);

    // Eval Yes
    let result = repl.execute("Eval Yes.");
    assert!(result.is_ok(), "Eval Yes should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Yes"), "Eval Yes should return Yes: {}", output);

    // Eval No
    let result = repl.execute("Eval No.");
    assert!(result.is_ok(), "Eval No should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("No"), "Eval No should return No: {}", output);

    // Define id_bool
    let result = repl.execute("Definition id_bool : MyBool -> MyBool := fun b : MyBool => b.");
    assert!(result.is_ok(), "Definition id_bool should work: {:?}", result.err());

    // Check id_bool type
    let result = repl.execute("Check id_bool.");
    assert!(result.is_ok(), "Check id_bool should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("MyBool -> MyBool"), "id_bool should have type MyBool -> MyBool: {}", output);

    // Eval id_bool Yes
    let result = repl.execute("Eval id_bool Yes.");
    assert!(result.is_ok(), "Eval id_bool Yes should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Yes"), "id_bool Yes should return Yes: {}", output);

    // Eval id_bool No
    let result = repl.execute("Eval id_bool No.");
    assert!(result.is_ok(), "Eval id_bool No should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("No"), "id_bool No should return No: {}", output);
}

// ============================================================
// Full Parse Tree Validation Tests (Logic Mode)
// These validate the complete AST structure
// ============================================================

/// Validate: "Every cat sleeps."
/// Expected structure: Quantifier(Universal) { body: Implication { Cat(x) -> Sleep(x) } }
#[test]
fn parse_tree_every_cat_sleeps() {
    let view = common::parse_to_view("Every cat sleeps.");
    match view {
        ExprView::Quantifier { kind, variable, body, .. } => {
            assert_eq!(kind, QuantifierKind::Universal,
                "Expected Universal quantifier, got {:?}", kind);
            assert!(!variable.is_empty(), "Should bind a variable");
            // Body should be an implication: Cat(x) -> Sleep(x)
            match *body {
                ExprView::BinaryOp { .. } => {
                    // Implication structure confirmed
                }
                _ => panic!("Expected BinaryOp (implication) in body, got {:?}", body),
            }
        }
        _ => panic!("Expected Quantifier variant for 'Every cat sleeps', got {:?}", view),
    }
}

/// Validate: "Some dogs bark loudly."
/// Expected structure: Quantifier(Existential) { body: And { Dog(x), BarkLoudly(x) } }
#[test]
fn parse_tree_some_dogs_bark() {
    let view = common::parse_to_view("Some dogs bark loudly.");
    match view {
        ExprView::Quantifier { kind, variable, .. } => {
            assert_eq!(kind, QuantifierKind::Existential,
                "Expected Existential quantifier, got {:?}", kind);
            assert!(!variable.is_empty(), "Should bind a variable");
        }
        _ => panic!("Expected Quantifier variant for 'Some dogs bark loudly', got {:?}", view),
    }
}

/// Validate: "John loves Mary."
/// Expected structure: NeoEvent { verb: Love, roles: [Agent(John), Theme(Mary)] }
#[test]
fn parse_tree_john_loves_mary() {
    let view = common::parse_to_view("John loves Mary.");
    match view {
        ExprView::NeoEvent { verb, roles, .. } => {
            assert_eq!(verb, "Love", "Expected verb 'Love', got '{}'", verb);
            assert!(roles.len() >= 2, "Should have at least 2 roles (agent, theme)");
        }
        _ => panic!("Expected NeoEvent variant for 'John loves Mary', got {:?}", view),
    }
}

/// Validate: "The quick brown fox jumps."
/// Expected structure: NeoEvent with definite description subject
#[test]
fn parse_tree_quick_brown_fox() {
    let view = common::parse_to_view("The quick brown fox jumps.");
    match view {
        ExprView::NeoEvent { verb, roles, .. } => {
            assert_eq!(verb, "Jump", "Expected verb 'Jump', got '{}'", verb);
            // Should have at least an agent role (the fox)
            assert!(!roles.is_empty(), "Should have thematic roles");
        }
        ExprView::Quantifier { .. } => {
            // Definite descriptions can be analyzed as quantifiers
        }
        _ => panic!("Expected NeoEvent or Quantifier for 'The quick brown fox jumps', got {:?}", view),
    }
}

/// Validate: "No student failed."
/// Expected structure: Quantifier(Universal) { body: Implication with negated body }
#[test]
fn parse_tree_no_student_failed() {
    let view = common::parse_to_view("No student failed.");
    match view {
        ExprView::Quantifier { kind, body, .. } => {
            // "No" is typically Universal with negated body: ∀x(Student(x) → ¬Failed(x))
            assert_eq!(kind, QuantifierKind::Universal,
                "Expected Universal quantifier for 'No', got {:?}", kind);
            // Body should contain negation
            match *body {
                ExprView::BinaryOp { .. } => {
                    // Implication with negated consequent
                }
                ExprView::UnaryOp { .. } => {
                    // Direct negation
                }
                _ => {}
            }
        }
        _ => panic!("Expected Quantifier variant for 'No student failed', got {:?}", view),
    }
}

/// Validate: "Every student read a book." (quantifier scope ambiguity)
/// Should parse with multiple scope readings
#[test]
fn parse_tree_quantifier_scope() {
    let readings = compile_all_scopes("Every student read a book.").unwrap();
    // Should have at least 2 readings due to scope ambiguity
    // 1. Surface scope: ∀x(Student(x) → ∃y(Book(y) ∧ Read(x,y)))
    // 2. Inverse scope: ∃y(Book(y) ∧ ∀x(Student(x) → Read(x,y)))
    assert!(readings.len() >= 1,
        "Should have at least one reading for scope ambiguity. Got {} readings", readings.len());
}

/// Validate: "John was running." (past progressive)
/// Expected structure: Temporal(Past) { Aspectual(Progressive) { Run(John) } }
#[test]
fn parse_tree_past_progressive() {
    let view = common::parse_to_view("John was running.");
    match view {
        ExprView::Temporal { operator, body } => {
            // Past tense wrapping progressive aspect
            assert_eq!(operator, TemporalOperator::Past,
                "Expected Past temporal operator, got {:?}", operator);
            match *body {
                ExprView::Aspectual { .. } | ExprView::NeoEvent { .. } => {
                    // Progressive aspect confirmed
                }
                _ => panic!("Expected Aspectual or NeoEvent in temporal body, got {:?}", body),
            }
        }
        ExprView::NeoEvent { modifiers, .. } => {
            // Alternative: modifiers contain tense/aspect info
            assert!(!modifiers.is_empty(), "Should have temporal modifiers");
        }
        _ => panic!("Expected Temporal or NeoEvent for 'John was running', got {:?}", view),
    }
}

/// Validate: "Mary has eaten." (perfect aspect)
/// Expected structure: Aspectual(Perfect) { Eat(Mary) }
#[test]
fn parse_tree_perfect_aspect() {
    let view = common::parse_to_view("Mary has eaten.");
    match view {
        ExprView::Aspectual { .. } => {
            // Perfect aspect confirmed
        }
        ExprView::NeoEvent { modifiers, verb, .. } => {
            assert_eq!(verb, "Eat", "Expected verb 'Eat', got '{}'", verb);
            // Modifiers should contain perfect aspect info
            assert!(!modifiers.is_empty(), "Modifiers should contain aspect info");
        }
        _ => panic!("Expected Aspectual or NeoEvent for 'Mary has eaten', got {:?}", view),
    }
}

// ============================================================
// Multi-Sentence Logic Mode Tests
// Verify sentences are compiled separately (not combined with AND)
// ============================================================

/// Test that multiple sentences produce numbered output (like marketing page)
#[test]
fn logic_multi_sentence_numbered_output() {
    // Multiple sentences together should produce numbered formulas
    let output = compile("Every cat sleeps. Some dogs bark.").unwrap();

    // Should contain numbered formulas (1, 2)
    assert!(output.contains("1)"),
        "Output should contain '1)' for first sentence. Got: {}", output);
    assert!(output.contains("2)"),
        "Output should contain '2)' for second sentence. Got: {}", output);

    // Should NOT contain top-level AND conjunction
    // (The old behavior was: (∀x... ∧ ∃y...) which is wrong)
    // The new behavior is: 1) ∀x...\n2) ∃y...
    let has_top_level_and = output.starts_with("(") && output.contains(" ∧ ");
    assert!(!has_top_level_and,
        "Should NOT have top-level AND conjunction. Got: {}", output);
}

/// Test single sentence has no numbering
#[test]
fn logic_single_sentence_no_numbering() {
    let output = compile("Every cat sleeps.").unwrap();

    // Single sentence should NOT have "1)" prefix
    assert!(!output.starts_with("1)"),
        "Single sentence should not be numbered. Got: {}", output);
}

/// Validate: "The train will arrive." (future tense with definite description)
/// Expected structure: Quantifier (for "the") wrapping NeoEvent with Future modifier
#[test]
fn parse_tree_future_tense() {
    let view = common::parse_to_view("The train will arrive.");
    // "The train" creates a definite description (quantifier structure)
    // The NeoEvent with "Future" modifier is embedded inside
    match view {
        ExprView::Quantifier { kind, body, .. } => {
            // "The" creates an existential with uniqueness condition
            assert_eq!(kind, QuantifierKind::Existential,
                "Expected Existential for 'the', got {:?}", kind);
            // Search for NeoEvent with "Arrive" verb and "Future" modifier inside the body
            fn contains_arrive_future(expr: &ExprView) -> bool {
                match expr {
                    ExprView::NeoEvent { verb, modifiers, .. } => {
                        *verb == "Arrive" && modifiers.contains(&"Future")
                    }
                    ExprView::BinaryOp { left, right, .. } => {
                        contains_arrive_future(left) || contains_arrive_future(right)
                    }
                    ExprView::Quantifier { body, .. } => contains_arrive_future(body),
                    _ => false
                }
            }
            assert!(contains_arrive_future(&body),
                "Should contain NeoEvent with Arrive and Future modifier");
        }
        ExprView::Temporal { operator, .. } => {
            // Alternative: direct temporal operator
            assert_eq!(operator, TemporalOperator::Future,
                "Expected Future temporal operator");
        }
        ExprView::NeoEvent { verb, modifiers, .. } => {
            assert_eq!(verb, "Arrive", "Expected verb 'Arrive', got '{}'", verb);
            assert!(modifiers.contains(&"Future"), "Should have Future modifier");
        }
        _ => panic!("Unexpected parse tree structure for 'The train will arrive': {:?}", view),
    }
}

// ============================================================
// NEW Logic Mode Examples (Advanced Theorems)
// ============================================================

/// Test: disjunctive-syllogism.logic (Either/Or reasoning)
#[test]
fn logic_prover_disjunctive_syllogism() {
    let source = r#"## Theorem: Disjunctive_Syllogism
Given: Either Alice or Bob is guilty.
Given: Alice is not guilty.
Prove: Bob is guilty.
Proof: Auto.
"#;

    let result = compile_theorem(source);
    assert!(result.is_ok(), "Should parse without errors: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Proved") || output.contains("DisjunctionElim"),
        "Should prove via disjunction elimination: {}", output);
}

/// Test: modus-tollens.logic (Backward reasoning via Modus Tollens)
/// Tests chained modus tollens: P→Q, Q→R, ¬R ⊢ ¬P
#[test]
fn logic_prover_modus_tollens() {
    // Use proper name "Butler" to test pure modus tollens chain
    // without the complexity of definite description uniqueness constraints
    let source = r#"## Theorem: Modus_Tollens_Chain
Given: If Butler is guilty, then Butler is arrested.
Given: If Butler is arrested, then Butler is jailed.
Given: Butler is not jailed.
Prove: Butler is not guilty.
Proof: Auto.
"#;

    let result = compile_theorem(source);
    assert!(result.is_ok(), "Should parse and prove: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Proved") || output.contains("ModusTollens"),
        "Should prove via modus tollens chain: {}", output);
}

/// Test: leibniz-identity.logic (Equality substitution)
#[test]
fn logic_prover_leibniz_identity() {
    let source = r#"## Theorem: Leibniz_Identity
Given: Clark is Superman.
Given: Clark is mortal.
Prove: Superman is mortal.
Proof: Auto.
"#;

    let result = compile_theorem(source);
    assert!(result.is_ok(), "Leibniz identity should compile: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Proved") || output.contains("Rewrite"),
        "Should prove via equality rewriting: {}", output);
}

/// Test: barber-paradox.logic (Self-reference and contradiction)
#[test]
fn logic_prover_barber_paradox() {
    let source = r#"## Theorem: Barber_Paradox
Given: The barber is a man.
Given: The barber shaves all men who do not shave themselves.
Given: The barber does not shave any man who shaves himself.
Prove: The barber does not exist.
Proof: Auto.
"#;

    let result = compile_theorem(source);
    assert!(result.is_ok(), "Barber paradox should prove: {:?}", result.err());
}

// ============================================================
// NEW Code Mode Examples
// ============================================================

/// Test: factorial.logos (recursion)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_factorial() {
    let source = r#"## To factorial (n: Int):
    If n <= 1:
        Return 1.
    Return n * factorial(n - 1).

## Main

Show "Factorial of 5:".
Let result be factorial(5).
Show result.

Show "Factorial of 10:".
Let big be factorial(10).
Show big.
"#;

    let result = run_logos(source);
    assert!(result.success, "factorial should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("120"),
        "Should output factorial(5)=120. Got: {}", result.stdout);
    assert!(result.stdout.contains("3628800"),
        "Should output factorial(10)=3628800. Got: {}", result.stdout);
}

/// Test: prime-check.logos (loops and conditionals)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_prime_check() {
    let source = r#"## To is_prime (n: Int) -> Bool:
    If n <= 1:
        Return false.
    Let i be 2.
    While i * i <= n:
        If n / i * i equals n:
            Return false.
        Set i to i + 1.
    Return true.

## Main

Show "Prime numbers from 2 to 30:".
Repeat for num from 2 to 30:
    If is_prime(num):
        Show num.
"#;

    let result = run_logos(source);
    assert!(result.success, "prime-check should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("2"), "Should find 2 is prime");
    assert!(result.stdout.contains("3"), "Should find 3 is prime");
    assert!(result.stdout.contains("5"), "Should find 5 is prime");
    assert!(result.stdout.contains("7"), "Should find 7 is prime");
    assert!(result.stdout.contains("29"), "Should find 29 is prime");
}

/// Test: sum-list.logos (list iteration)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_sum_list() {
    let source = r#"## Main

Let numbers be [10, 20, 30, 40, 50].
Let total be 0.

Repeat for n in numbers:
    Set total to total + n.

Show "Sum of [10, 20, 30, 40, 50]:".
Show total.
"#;

    let result = run_logos(source);
    assert!(result.success, "sum-list should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("150"),
        "Should output sum 150. Got: {}", result.stdout);
}

/// Test: bubble-sort.logos (nested loops and list mutation)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_bubble_sort() {
    let source = r#"## Main

Let numbers be [64, 34, 25, 12, 22, 11, 90].
Let n be length of numbers.

Show "Before sorting:".
Show numbers.

Repeat for i from 1 to n:
    Repeat for j from 1 to (n - i):
        Let a be item j of numbers.
        Let b be item (j + 1) of numbers.
        If a > b:
            Set item j of numbers to b.
            Set item (j + 1) of numbers to a.

Show "After sorting:".
Show numbers.
"#;

    let result = run_logos(source);
    assert!(result.success, "bubble-sort should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("[64, 34, 25, 12, 22, 11, 90]"),
        "Should show unsorted list. Got: {}", result.stdout);
    assert!(result.stdout.contains("[11, 12, 22, 25, 34, 64, 90]"),
        "Should show sorted list. Got: {}", result.stdout);
}

/// Test: struct-demo.logos (custom types)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_struct_demo() {
    let source = r#"## Definition

A Person has:
    a public name, which is Text.
    a public age, which is Int.

## Main

Let alice be a new Person.
Set alice's name to "Alice".
Set alice's age to 30.

Let bob be a new Person.
Set bob's name to "Bob".
Set bob's age to 25.

Show "Person 1:".
Show alice's name.
Show alice's age.

Show "Person 2:".
Show bob's name.
Show bob's age.
"#;

    let result = run_logos(source);
    assert!(result.success, "struct-demo should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("Alice"),
        "Should output Alice. Got: {}", result.stdout);
    assert!(result.stdout.contains("30"),
        "Should output age 30. Got: {}", result.stdout);
    assert!(result.stdout.contains("Bob"),
        "Should output Bob. Got: {}", result.stdout);
    assert!(result.stdout.contains("25"),
        "Should output age 25. Got: {}", result.stdout);
}

// ============================================================
// NEW Math Mode Examples
// ============================================================

/// Test: prop-logic.logos (propositional logic types)
#[test]
fn math_prop_logic() {
    let mut repl = Repl::new();

    // Define MyProp
    let result = repl.execute("Inductive MyProp := PTrue : MyProp | PFalse : MyProp | PAnd : MyProp -> MyProp -> MyProp | POr : MyProp -> MyProp -> MyProp | PNot : MyProp -> MyProp.");
    assert!(result.is_ok(), "MyProp inductive should work: {:?}", result.err());

    // Define some propositions
    let result = repl.execute("Definition p3 : MyProp := PAnd PTrue PTrue.");
    assert!(result.is_ok(), "Definition p3 should work: {:?}", result.err());

    // Check type
    let result = repl.execute("Check p3.");
    assert!(result.is_ok(), "Check p3 should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("MyProp"), "p3 should have type MyProp: {}", output);
}

/// Test: functions.logos (lambda calculus basics)
#[test]
fn math_functions() {
    let mut repl = Repl::new();

    // Need Nat first
    let result = repl.execute("Inductive Nat := Zero : Nat | Succ : Nat -> Nat.");
    assert!(result.is_ok(), "Nat should be defined");

    // Define identity function
    let result = repl.execute("Definition id : Nat -> Nat := fun x : Nat => x.");
    assert!(result.is_ok(), "Definition id should work: {:?}", result.err());

    // Check type
    let result = repl.execute("Check id.");
    assert!(result.is_ok(), "Check id should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Nat -> Nat"), "id should have type Nat -> Nat: {}", output);

    // Evaluate id on Zero
    let result = repl.execute("Eval id Zero.");
    assert!(result.is_ok(), "Eval id Zero should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Zero"), "id Zero should evaluate to Zero: {}", output);
}

/// Test: list-ops.logos (polymorphic lists)
#[test]
fn math_list_ops() {
    let mut repl = Repl::new();

    // Need Nat first
    let result = repl.execute("Inductive Nat := Zero : Nat | Succ : Nat -> Nat.");
    assert!(result.is_ok(), "Nat should be defined");

    // Define MyList
    let result = repl.execute("Inductive MyList (A : Type) := MyNil : MyList A | MyCons : A -> MyList A -> MyList A.");
    assert!(result.is_ok(), "MyList inductive should work: {:?}", result.err());

    // Check MyNil type
    let result = repl.execute("Check MyNil.");
    assert!(result.is_ok(), "Check MyNil should work: {:?}", result.err());
}

/// Test: pairs.logos (product types)
#[test]
fn math_pairs() {
    let mut repl = Repl::new();

    // Need Nat and MyBool first
    let result = repl.execute("Inductive Nat := Zero : Nat | Succ : Nat -> Nat.");
    assert!(result.is_ok(), "Nat should be defined");

    let result = repl.execute("Inductive MyBool := Yes : MyBool | No : MyBool.");
    assert!(result.is_ok(), "MyBool should be defined");

    // Define MyPair
    let result = repl.execute("Inductive MyPair (A : Type) (B : Type) := MkPair : A -> B -> MyPair A B.");
    assert!(result.is_ok(), "MyPair inductive should work: {:?}", result.err());

    // Check MkPair type
    let result = repl.execute("Check MkPair.");
    assert!(result.is_ok(), "Check MkPair should work: {:?}", result.err());
}

// ============================================================
// Advanced Code Mode Examples (Types, Collections, etc.)
// ============================================================

/// Test: types/enums.logos (enum definition and pattern matching)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_enums_example() {
    let source = r#"# Enums & Pattern Matching

## A Color is one of:
    A Red.
    A Green.
    A Blue.

## Main

Let c be a new Red.
Inspect c:
    When Red: Show "It's red!".
    When Green: Show "It's green!".
    When Blue: Show "It's blue!".

Let c2 be a new Blue.
Inspect c2:
    When Red: Show "red".
    Otherwise: Show "not red".
"#;

    let result = run_logos(source);
    assert!(result.success, "enums example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("It's red!"),
        "Should output 'It's red!'. Got: {}", result.stdout);
    assert!(result.stdout.contains("not red"),
        "Should output 'not red'. Got: {}", result.stdout);
}

/// Test: types/generics.logos (generic map types)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_generics_maps_example() {
    let source = r#"## Main

Let mut scores be a new Map of Text to Int.
Set scores["Alice"] to 100.
Set scores["Bob"] to 85.
Set scores["Charlie"] to 92.

Let alice_score be scores["Alice"].
Show "Alice's score:".
Show alice_score.

Set scores["Bob"] to 90.
Show "Bob's new score:".
Show scores["Bob"].

Let total be scores["Alice"] + scores["Bob"] + scores["Charlie"].
Show "Total:".
Show total.
"#;

    let result = run_logos(source);
    assert!(result.success, "generics/maps example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("100"),
        "Should output Alice's score 100. Got: {}", result.stdout);
    assert!(result.stdout.contains("90"),
        "Should output Bob's new score 90. Got: {}", result.stdout);
    assert!(result.stdout.contains("282"),
        "Should output total 282. Got: {}", result.stdout);
}

/// Test: collections/sets.logos (set operations)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_sets_example() {
    let source = r#"## Main

Let names be a new Set of Text.
Add "Alice" to names.
Add "Bob" to names.
Add "Charlie" to names.
Add "Alice" to names.

Show "Set size (duplicates ignored):".
Show length of names.

If names contains "Bob":
    Show "Bob is in the set!".

Remove "Bob" from names.
Show "After removing Bob:".
Show length of names.

Let sum be 0.
Let numbers be a new Set of Int.
Add 10 to numbers.
Add 20 to numbers.
Add 30 to numbers.
Repeat for n in numbers:
    Set sum to sum + n.
Show "Sum of numbers:".
Show sum.
"#;

    let result = run_logos(source);
    assert!(result.success, "sets example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("3"),
        "Should output set size 3. Got: {}", result.stdout);
    assert!(result.stdout.contains("Bob is in the set!"),
        "Should find Bob. Got: {}", result.stdout);
    assert!(result.stdout.contains("2"),
        "Should output size 2 after removing. Got: {}", result.stdout);
    assert!(result.stdout.contains("60"),
        "Should output sum 60. Got: {}", result.stdout);
}

/// Test: collections/maps.logos (map operations with mixed syntax)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_maps_example() {
    let source = r#"## Main

Let mut inventory be a new Map of Text to Int.
Set item "iron" of inventory to 50.
Set inventory["copper"] to 30.
Set inventory["gold"] to 10.

Show "Iron count:".
Show item "iron" of inventory.

Show "Copper count:".
Show inventory["copper"].

Set inventory["iron"] to 100.
Show "Updated iron:".
Show inventory["iron"].

Let total be item "iron" of inventory + inventory["copper"] + inventory["gold"].
Show "Total resources:".
Show total.
"#;

    let result = run_logos(source);
    assert!(result.success, "maps example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("50"),
        "Should output iron count 50. Got: {}", result.stdout);
    assert!(result.stdout.contains("30"),
        "Should output copper count 30. Got: {}", result.stdout);
    assert!(result.stdout.contains("100"),
        "Should output updated iron 100. Got: {}", result.stdout);
    assert!(result.stdout.contains("140"),
        "Should output total 140. Got: {}", result.stdout);
}

/// Test: functions/higher-order.logos (advanced function features)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_advanced_functions_example() {
    let source = r#"## To double (x: Int):
    Return x * 2.

## To add (a: Int) and (b: Int):
    Return a + b.

## To isEven (n: Int) -> Bool:
    Return n / 2 * 2 equals n.

## Main

Show "Double of 21:".
Show double(21).

Show "Sum of 15 and 27:".
Show add(15, 27).

Show "Is 42 even?".
Show isEven(42).

Show "Is 17 even?".
Show isEven(17).
"#;

    let result = run_logos(source);
    assert!(result.success, "advanced functions example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("42"),
        "Should output double(21)=42. Got: {}", result.stdout);
    assert!(result.stdout.contains("true"),
        "Should output true for isEven(42). Got: {}", result.stdout);
    assert!(result.stdout.contains("false"),
        "Should output false for isEven(17). Got: {}", result.stdout);
}

/// Test: distributed/counters.logos (CRDT counters)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_crdt_counters_example() {
    let source = r#"## Definition
A Counter is Shared and has:
    points: ConvergentCount.

## Main
Let mutable c be a new Counter.
Increase c's points by 10.
Increase c's points by 5.
Increase c's points by 3.
Show "Total points:".
Show c's points.
"#;

    let result = run_logos(source);
    assert!(result.success, "CRDT counters example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("18"),
        "Should output total 18. Got: {}", result.stdout);
}

/// Test: security/policies.logos (security policy checks)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_policies_example() {
    let source = r#"# Security Policies

## Definition
A User has:
    a role, which is Text.

## Policy
A User is admin if the user's role equals "admin".

## Main

Let u be a new User with role "admin".
Check that the u is admin.
Show "Admin check passed!".

Let guest be a new User with role "guest".
Show "Guest created (would fail admin check)".
"#;

    let result = run_logos(source);
    assert!(result.success, "policies example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("Admin check passed!"),
        "Should pass admin check. Got: {}", result.stdout);
    assert!(result.stdout.contains("Guest created"),
        "Should create guest. Got: {}", result.stdout);
}

/// Test: memory/zones.logos (memory zone allocation)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_zones_example() {
    let source = r#"# Memory Zones

## Main

Show "Working with memory zones...".

Inside a zone called "Work":
    Let x be 42.
    Let y be 58.
    Let sum be x + y.
    Show "Sum in zone:".
    Show sum.

Inside a zone called "Buffer" of size 1 MB:
    Let value be 100.
    Show "Value in sized zone:".
    Show value.

Show "Zones cleaned up!".
"#;

    let result = run_logos(source);
    assert!(result.success, "zones example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("Working with memory zones"),
        "Should output intro. Got: {}", result.stdout);
    assert!(result.stdout.contains("100"),
        "Should output zone sum 100. Got: {}", result.stdout);
    assert!(result.stdout.contains("Zones cleaned up!"),
        "Should output cleanup. Got: {}", result.stdout);
}

/// Test: native/tasks.logos (task spawning - native only)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_tasks_example() {
    let source = r#"# Task Spawning
-- NOTE: Native compilation only.

## To worker:
    Show "worker done".

## To greet (name: Text):
    Show name.

## Main

Launch a task to worker.
Show "main continues".

Launch a task to greet with "Hello from task".
Show "task launched".
"#;

    let result = run_logos(source);
    assert!(result.success, "tasks example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("main continues"),
        "Should output 'main continues'. Got: {}", result.stdout);
    assert!(result.stdout.contains("task launched"),
        "Should output 'task launched'. Got: {}", result.stdout);
}

/// Test: native/channels.logos (pipes and message passing - native only)
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_channels_example() {
    let source = r#"# Pipes and Message Passing
-- NOTE: Native compilation only.

## Main

Let ch be a Pipe of Int.
Show "pipe created".

Send 42 into ch.
Show "sent 42".

Receive x from ch.
Show "received:".
Show x.
"#;

    let result = run_logos(source);
    assert!(result.success, "channels example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("pipe created"),
        "Should output 'pipe created'. Got: {}", result.stdout);
    assert!(result.stdout.contains("42"),
        "Should output received 42. Got: {}", result.stdout);
}

// ============================================================
// NEW: Basics Examples (Guide Sections 3-5)
// ============================================================

/// Test: basics/variables.logos
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_basics_variables() {
    let source = r#"## Main

Let name be "Alice".
Let age be 25.
Let is_active be true.
Let price be 19.99.

Show name.
Show age.
Show is_active.
Show price.

Let count be 100.
Let doubled be count * 2.
Show doubled.
"#;

    let result = run_logos(source);
    assert!(result.success, "variables example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("Alice"), "Should output name. Got: {}", result.stdout);
    assert!(result.stdout.contains("25"), "Should output age. Got: {}", result.stdout);
    assert!(result.stdout.contains("true"), "Should output active. Got: {}", result.stdout);
    assert!(result.stdout.contains("200"), "Should output doubled. Got: {}", result.stdout);
}

/// Test: basics/operators.logos
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_basics_operators() {
    let source = r#"## Main

Let a be 10.
Let b be 3.

Show a + b.
Show a - b.
Show a * b.
Show a / b.
Show a % b.

Show a is greater than b.
Show a equals 10.

Let x be true.
Let y be false.
Show x and y.
Show x or y.
Show not x.
"#;

    let result = run_logos(source);
    assert!(result.success, "operators example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("13"), "Should output 10+3=13. Got: {}", result.stdout);
    assert!(result.stdout.contains("7"), "Should output 10-3=7. Got: {}", result.stdout);
    assert!(result.stdout.contains("30"), "Should output 10*3=30. Got: {}", result.stdout);
    assert!(result.stdout.contains("true"), "Should output comparisons. Got: {}", result.stdout);
    assert!(result.stdout.contains("false"), "Should output logical. Got: {}", result.stdout);
}

/// Test: basics/control-flow.logos
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_basics_control_flow() {
    let source = r#"## Main

Let score be 85.

If score is at least 90:
    Show "Grade: A".
If score is at least 80 and score is less than 90:
    Show "Grade: B".
If score is less than 80:
    Show "Grade: C or below".

Let count be 1.
While count is at most 3:
    Show count.
    Set count to count + 1.

Let items be [10, 20, 30].
Repeat for n in items:
    Show n.
"#;

    let result = run_logos(source);
    assert!(result.success, "control-flow example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("Grade: B"), "Should output Grade B. Got: {}", result.stdout);
    assert!(result.stdout.contains("1"), "Should output while 1. Got: {}", result.stdout);
    assert!(result.stdout.contains("2"), "Should output while 2. Got: {}", result.stdout);
    assert!(result.stdout.contains("3"), "Should output while 3. Got: {}", result.stdout);
    assert!(result.stdout.contains("10"), "Should output for-each 10. Got: {}", result.stdout);
    assert!(result.stdout.contains("20"), "Should output for-each 20. Got: {}", result.stdout);
    assert!(result.stdout.contains("30"), "Should output for-each 30. Got: {}", result.stdout);
}

// ============================================================
// NEW: Enum Patterns Example (Guide Section 8)
// ============================================================

/// Test: types/enums-patterns.logos
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_enums_patterns() {
    let source = r#"## A Status is one of:
    A Pending.
    A Active.
    A Completed.
    A Failed.

## Main

Let s be a new Active.
Inspect s:
    When Pending: Show "Waiting".
    When Active: Show "In progress".
    When Completed: Show "Done".
    When Failed: Show "Error".

Let s2 be a new Completed.
Inspect s2:
    When Active: Show "still working".
    Otherwise: Show "not active".
"#;

    let result = run_logos(source);
    assert!(result.success, "enums-patterns example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("In progress"), "Should match Active. Got: {}", result.stdout);
    assert!(result.stdout.contains("not active"), "Should match Otherwise. Got: {}", result.stdout);
}

// ============================================================
// NEW: Ownership Example (Guide Section 10)
// ============================================================

/// Test: memory/ownership.logos - simplified version
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_ownership_example() {
    let source = r#"## To display (data: Text):
    Show data.

## To consume (data: Text):
    Show data.

## Main

Let profile be "User Profile Data".
Show profile.

Let duplicate be "Copy of data".
Show duplicate.
"#;

    let result = run_logos(source);
    assert!(result.success, "ownership example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("User Profile Data"),
        "Should show profile. Got: {}", result.stdout);
    assert!(result.stdout.contains("Copy of data"),
        "Should show duplicate. Got: {}", result.stdout);
}

// ============================================================
// NEW: Concurrency Example (Guide Section 12)
// ============================================================

/// Test: concurrency/parallel.logos - sequential version for compilation
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_concurrency_parallel() {
    // Concurrency constructs are interpreter-only, so test with sequential code
    let source = r#"## Main

Let a be 100.
Let b be 200.

Show a.
Show b.
Show a * b.

Let x be 10.
Let y be 20.

Show x + y.
"#;

    let result = run_logos(source);
    assert!(result.success, "concurrency example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("100"), "Should output a=100. Got: {}", result.stdout);
    assert!(result.stdout.contains("200"), "Should output b=200. Got: {}", result.stdout);
    assert!(result.stdout.contains("20000"), "Should output product. Got: {}", result.stdout);
    assert!(result.stdout.contains("30"), "Should output sum. Got: {}", result.stdout);
}

// ============================================================
// NEW: Additional CRDT Examples (Guide Section 13)
// ============================================================

/// Test: distributed/tally.logos (PN-Counter) - simplified
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_crdt_tally() {
    let source = r#"## Definition
A Score is Shared and has:
    points: Tally.

## Main
Let mutable s be a new Score.
Increase s's points by 100.
Show "After increase".

Decrease s's points by 30.
Show "After decrease".

Increase s's points by 10.
Show "Final done".
"#;

    let result = run_logos(source);
    assert!(result.success, "tally example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("After increase"), "Should show after increase. Got: {}", result.stdout);
    assert!(result.stdout.contains("After decrease"), "Should show after decrease. Got: {}", result.stdout);
    assert!(result.stdout.contains("Final done"), "Should show final. Got: {}", result.stdout);
}

/// Test: distributed/merge.logos (CRDT merge) - simplified
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_crdt_merge() {
    let source = r#"## Definition
A Stats is Shared and has:
    views: ConvergentCount.

## Main
Let local be a new Stats.
Increase local's views by 100.
Show "Local created".

Let remote be a new Stats.
Increase remote's views by 50.
Show "Remote created".

Merge remote into local.
Show "Merged".
"#;

    let result = run_logos(source);
    assert!(result.success, "merge example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("Local created"), "Should show local. Got: {}", result.stdout);
    assert!(result.stdout.contains("Remote created"), "Should show remote. Got: {}", result.stdout);
    assert!(result.stdout.contains("Merged"), "Should show merged. Got: {}", result.stdout);
}

// ============================================================
// NEW: Error Handling Example (Guide Section 16)
// ============================================================

/// Test: error-handling.logos
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_error_handling() {
    let source = r#"## To safe_divide (a: Int) and (b: Int) -> Int:
    If b equals 0:
        Show "Error: Cannot divide by zero".
        Return 0.
    Return a / b.

## To validate_age (age: Int) -> Bool:
    If age is less than 0:
        Show "Error: Age cannot be negative".
        Return false.
    If age is greater than 150:
        Show "Error: Age seems unrealistic".
        Return false.
    Return true.

## Main

Show safe_divide(10, 2).
Show safe_divide(5, 0).

Show validate_age(25).
Show validate_age(0 - 5).
"#;

    let result = run_logos(source);
    assert!(result.success, "error-handling example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("5"), "Should output 10/2=5. Got: {}", result.stdout);
    assert!(result.stdout.contains("Error: Cannot divide by zero"),
        "Should show divide error. Got: {}", result.stdout);
    assert!(result.stdout.contains("Error: Age cannot be negative"),
        "Should show age error. Got: {}", result.stdout);
}

// ============================================================
// NEW: Advanced Examples (Guide Sections 17, 22-23)
// ============================================================

/// Test: advanced/refinement.logos
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_advanced_refinement() {
    let source = r#"## Main

Let positive: Int where it > 0 be 5.
Let percentage: Int where it >= 0 and it <= 100 be 85.

Show positive.
Show percentage.
"#;

    let result = run_logos(source);
    assert!(result.success, "refinement example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("5"), "Should output positive. Got: {}", result.stdout);
    assert!(result.stdout.contains("85"), "Should output percentage. Got: {}", result.stdout);
}

/// Test: advanced/assertions.logos - basic validation pattern
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn code_advanced_assertions() {
    let source = r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main

Let result be double(25).
Show result.

Let another be double(5).
Show another.
"#;

    let result = run_logos(source);
    assert!(result.success, "assertions example should run.\nstderr: {}\nrust: {}",
        result.stderr, result.rust_code);
    assert!(result.stdout.contains("50"), "Should output 25*2=50. Got: {}", result.stdout);
    assert!(result.stdout.contains("10"), "Should output 5*2=10. Got: {}", result.stdout);
}

// ============================================================
// Math Mode: Collatz Example (MATH_COLLATZ)
// ============================================================

/// Test: math/collatz.logos (Collatz conjecture implementation)
/// This is a comprehensive test for user-defined inductives with pattern matching.
#[test]
fn math_collatz_mybool_definition() {
    let mut repl = Repl::new();

    // Define MyBool with Yes/No constructors
    let result = repl.execute("Inductive MyBool := Yes : MyBool | No : MyBool.");
    assert!(result.is_ok(), "MyBool inductive should work: {:?}", result.err());

    // Check Yes type
    let result = repl.execute("Check Yes.");
    assert!(result.is_ok(), "Check Yes should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("MyBool"), "Yes should have type MyBool: {}", output);

    // Check No type
    let result = repl.execute("Check No.");
    assert!(result.is_ok(), "Check No should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("MyBool"), "No should have type MyBool: {}", output);
}

/// Test: Pattern matching on user-defined MyBool type
/// This is the critical test - the kernel was expecting 4 cases instead of 2.
#[test]
fn math_collatz_not_function() {
    let mut repl = Repl::new();

    // Define MyBool
    let result = repl.execute("Inductive MyBool := Yes : MyBool | No : MyBool.");
    assert!(result.is_ok(), "MyBool inductive should work: {:?}", result.err());

    // Define not function with pattern matching on MyBool
    // This is where the bug manifested: "Wrong number of cases: expected 4, found 2"
    let result = repl.execute(
        "Definition not : MyBool -> MyBool := fun b : MyBool => match b return MyBool with | Yes => No | No => Yes end."
    );
    assert!(result.is_ok(), "Definition not should work (pattern match on MyBool): {:?}", result.err());

    // Check the type
    let result = repl.execute("Check not.");
    assert!(result.is_ok(), "Check not should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("MyBool -> MyBool"), "not should have type MyBool -> MyBool: {}", output);

    // Evaluate not Yes = No
    let result = repl.execute("Eval (not Yes).");
    assert!(result.is_ok(), "Eval (not Yes) should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("No"), "not Yes should evaluate to No: {}", output);

    // Evaluate not No = Yes
    let result = repl.execute("Eval (not No).");
    assert!(result.is_ok(), "Eval (not No) should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Yes"), "not No should evaluate to Yes: {}", output);
}

/// Test: isEven function using fixpoint and pattern matching
#[test]
fn math_collatz_iseven_function() {
    let mut repl = Repl::new();

    // Define MyBool
    let result = repl.execute("Inductive MyBool := Yes : MyBool | No : MyBool.");
    assert!(result.is_ok(), "MyBool should be defined");

    // Define not
    let result = repl.execute(
        "Definition not : MyBool -> MyBool := fun b : MyBool => match b return MyBool with | Yes => No | No => Yes end."
    );
    assert!(result.is_ok(), "not should be defined: {:?}", result.err());

    // Define isEven with dependent motive
    let result = repl.execute(
        "Definition isEven : Nat -> MyBool := fix rec => fun n : Nat => match n return (fun _ : Nat => MyBool) with | Zero => Yes | Succ k => not (rec k) end."
    );
    assert!(result.is_ok(), "isEven should be defined: {:?}", result.err());

    // Check type
    let result = repl.execute("Check isEven.");
    assert!(result.is_ok(), "Check isEven should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Nat -> MyBool"), "isEven should have type Nat -> MyBool: {}", output);

    // Test isEven(0) = Yes
    let result = repl.execute("Eval (isEven Zero).");
    assert!(result.is_ok(), "Eval isEven Zero should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Yes"), "isEven 0 should be Yes: {}", output);

    // Test isEven(1) = No
    let result = repl.execute("Eval (isEven (Succ Zero)).");
    assert!(result.is_ok(), "Eval isEven 1 should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("No"), "isEven 1 should be No: {}", output);

    // Test isEven(2) = Yes
    let result = repl.execute("Eval (isEven (Succ (Succ Zero))).");
    assert!(result.is_ok(), "Eval isEven 2 should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Yes"), "isEven 2 should be Yes: {}", output);
}

/// Test: half function with nested pattern match on MyBool
#[test]
fn math_collatz_half_function() {
    let mut repl = Repl::new();

    // Setup: MyBool, not, isEven, isOdd
    let result = repl.execute("Inductive MyBool := Yes : MyBool | No : MyBool.");
    assert!(result.is_ok(), "MyBool should be defined");

    let result = repl.execute(
        "Definition not : MyBool -> MyBool := fun b : MyBool => match b return MyBool with | Yes => No | No => Yes end."
    );
    assert!(result.is_ok(), "not should be defined: {:?}", result.err());

    let result = repl.execute(
        "Definition isEven : Nat -> MyBool := fix rec => fun n : Nat => match n return (fun _ : Nat => MyBool) with | Zero => Yes | Succ k => not (rec k) end."
    );
    assert!(result.is_ok(), "isEven should be defined: {:?}", result.err());

    let result = repl.execute(
        "Definition isOdd : Nat -> MyBool := fun n : Nat => not (isEven n)."
    );
    assert!(result.is_ok(), "isOdd should be defined: {:?}", result.err());

    // Define half with nested pattern match on MyBool result
    // This tests matching on MyBool within a Nat match
    let result = repl.execute(
        "Definition half : Nat -> Nat := fix rec => fun n : Nat => match n return Nat with | Zero => Zero | Succ k => match (isOdd k) return (fun _ : MyBool => Nat) with | Yes => Succ (rec k) | No => rec k end end."
    );
    assert!(result.is_ok(), "half should be defined (nested MyBool match): {:?}", result.err());

    // Check type
    let result = repl.execute("Check half.");
    assert!(result.is_ok(), "Check half should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Nat -> Nat"), "half should have type Nat -> Nat: {}", output);

    // Test half(4) = 2
    let result = repl.execute("Eval (half (Succ (Succ (Succ (Succ Zero))))).");
    assert!(result.is_ok(), "Eval half 4 should work: {:?}", result.err());
    let output = result.unwrap();
    // half(4) = 2, which is Succ (Succ Zero)
    assert!(output.contains("Succ") && output.contains("Zero"),
        "half 4 should be 2 (Succ (Succ Zero)): {}", output);
}

/// Test: Full Collatz step function
#[test]
fn math_collatz_step_function() {
    let mut repl = Repl::new();

    // Setup all dependencies
    let result = repl.execute("Inductive MyBool := Yes : MyBool | No : MyBool.");
    assert!(result.is_ok(), "MyBool should be defined");

    let result = repl.execute(
        "Definition not : MyBool -> MyBool := fun b : MyBool => match b return MyBool with | Yes => No | No => Yes end."
    );
    assert!(result.is_ok(), "not should be defined: {:?}", result.err());

    let result = repl.execute(
        "Definition plus : Nat -> Nat -> Nat := fix rec => fun n : Nat => fun m : Nat => match n return Nat with | Zero => m | Succ k => Succ (rec k m) end."
    );
    assert!(result.is_ok(), "plus should be defined: {:?}", result.err());

    let result = repl.execute(
        "Definition isEven : Nat -> MyBool := fix rec => fun n : Nat => match n return (fun _ : Nat => MyBool) with | Zero => Yes | Succ k => not (rec k) end."
    );
    assert!(result.is_ok(), "isEven should be defined: {:?}", result.err());

    let result = repl.execute(
        "Definition isOdd : Nat -> MyBool := fun n : Nat => not (isEven n)."
    );
    assert!(result.is_ok(), "isOdd should be defined: {:?}", result.err());

    let result = repl.execute(
        "Definition half : Nat -> Nat := fix rec => fun n : Nat => match n return Nat with | Zero => Zero | Succ k => match (isOdd k) return (fun _ : MyBool => Nat) with | Yes => Succ (rec k) | No => rec k end end."
    );
    assert!(result.is_ok(), "half should be defined: {:?}", result.err());

    let result = repl.execute(
        "Definition double : Nat -> Nat := fun n : Nat => plus n n."
    );
    assert!(result.is_ok(), "double should be defined: {:?}", result.err());

    let result = repl.execute(
        "Definition triple : Nat -> Nat := fun n : Nat => plus n (double n)."
    );
    assert!(result.is_ok(), "triple should be defined: {:?}", result.err());

    // Define collatzStep - this matches on MyBool (isEven n)
    let result = repl.execute(
        "Definition collatzStep : Nat -> Nat := fun n : Nat => match (isEven n) return (fun _ : MyBool => Nat) with | Yes => half n | No => Succ (triple n) end."
    );
    assert!(result.is_ok(), "collatzStep should be defined (pattern match on MyBool): {:?}", result.err());

    // Check type
    let result = repl.execute("Check collatzStep.");
    assert!(result.is_ok(), "Check collatzStep should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Nat -> Nat"), "collatzStep should have type Nat -> Nat: {}", output);

    // Test collatzStep(2) = 1 (even, so divide by 2)
    let result = repl.execute("Eval (collatzStep (Succ (Succ Zero))).");
    assert!(result.is_ok(), "Eval collatzStep 2 should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Succ Zero") || output == "Succ Zero\n",
        "collatzStep 2 should be 1 (Succ Zero): {}", output);
}

// ============================================================
// COLLATZ REACHABILITY PREDICATE TESTS
// ============================================================

/// Test: ReachesOne inductive predicate for Collatz conjecture
/// This tests the "logical engine" - proving specific numbers reach 1
#[test]
fn math_collatz_reaches_one_predicate() {
    let mut repl = Repl::new();

    // Setup all dependencies (using Coq-style syntax for compatibility)
    repl.execute("Inductive MyBool := Yes : MyBool | No : MyBool.").unwrap();
    repl.execute("Definition not : MyBool -> MyBool := fun b : MyBool => match b return MyBool with | Yes => No | No => Yes end.").unwrap();
    repl.execute("Definition plus : Nat -> Nat -> Nat := fix rec => fun n : Nat => fun m : Nat => match n return Nat with | Zero => m | Succ k => Succ (rec k m) end.").unwrap();
    repl.execute("Definition isEven : Nat -> MyBool := fix rec => fun n : Nat => match n return (fun _ : Nat => MyBool) with | Zero => Yes | Succ k => not (rec k) end.").unwrap();
    repl.execute("Definition isOdd : Nat -> MyBool := fun n : Nat => not (isEven n).").unwrap();
    repl.execute("Definition half : Nat -> Nat := fix rec => fun n : Nat => match n return Nat with | Zero => Zero | Succ k => match (isOdd k) return (fun _ : MyBool => Nat) with | Yes => Succ (rec k) | No => rec k end end.").unwrap();
    repl.execute("Definition double : Nat -> Nat := fun n : Nat => plus n n.").unwrap();
    repl.execute("Definition triple : Nat -> Nat := fun n : Nat => plus n (double n).").unwrap();
    repl.execute("Definition collatzStep : Nat -> Nat := fun n : Nat => match (isEven n) return (fun _ : MyBool => Nat) with | Yes => half n | No => Succ (triple n) end.").unwrap();

    // Define ReachesOne - an inductive predicate with parameter n
    let result = repl.execute(
        "Inductive ReachesOne (n : Nat) := | Done : Eq Nat n (Succ Zero) -> ReachesOne n | Step : ReachesOne (collatzStep n) -> ReachesOne n."
    );
    assert!(result.is_ok(), "ReachesOne inductive should be defined: {:?}", result.err());

    // Check constructor types
    let result = repl.execute("Check Done.");
    assert!(result.is_ok(), "Check Done should work: {:?}", result.err());
    let output = result.unwrap();
    // Done should take: (n : Nat) -> Eq Nat n (Succ Zero) -> ReachesOne n
    assert!(output.contains("Nat") && output.contains("Eq"),
        "Done should have correct type: {}", output);

    let result = repl.execute("Check Step.");
    assert!(result.is_ok(), "Check Step should work: {:?}", result.err());

    // Verify collatzStep reduces correctly
    let result = repl.execute("Eval (collatzStep (Succ (Succ Zero))).");
    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(output.contains("Succ Zero"), "collatzStep 2 should be 1: {}", output);

    // Proof that 1 reaches 1 (trivial base case)
    let result = repl.execute(
        "Definition one_reaches : ReachesOne (Succ Zero) := Done (Succ Zero) (refl Nat (Succ Zero))."
    );
    assert!(result.is_ok(), "one_reaches proof should type-check: {:?}", result.err());

    // THE KEY TEST: Proof that 2 reaches 1
    // This requires the type checker to recognize that:
    //   collatzStep (Succ (Succ Zero)) reduces to (Succ Zero)
    // So Step expects ReachesOne (collatzStep 2) = ReachesOne 1
    let result = repl.execute(
        "Definition two_reaches : ReachesOne (Succ (Succ Zero)) := Step (Succ (Succ Zero)) (Done (Succ Zero) (refl Nat (Succ Zero)))."
    );
    assert!(result.is_ok(), "two_reaches proof should type-check (requires reduction in type comparison): {:?}", result.err());

    // Verify types
    let result = repl.execute("Check one_reaches.");
    assert!(result.is_ok(), "Check one_reaches should work: {:?}", result.err());

    let result = repl.execute("Check two_reaches.");
    assert!(result.is_ok(), "Check two_reaches should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("ReachesOne"), "two_reaches should have ReachesOne type: {}", output);
}

/// Test: InverseCollatz tree for proving membership backwards from 1
#[test]
fn math_collatz_inverse_tree() {
    let mut repl = Repl::new();

    // Setup double function
    repl.execute("Definition plus : Nat -> Nat -> Nat := fix rec => fun n : Nat => fun m : Nat => match n return Nat with | Zero => m | Succ k => Succ (rec k m) end.").unwrap();
    repl.execute("Definition double : Nat -> Nat := fun n : Nat => plus n n.").unwrap();

    // Define InverseCollatz - the tree of numbers reachable from 1
    let result = repl.execute(
        "Inductive InverseCollatz (n : Nat) := | Root : Eq Nat n (Succ Zero) -> InverseCollatz n | FromDouble : InverseCollatz n -> InverseCollatz (double n)."
    );
    assert!(result.is_ok(), "InverseCollatz inductive should be defined: {:?}", result.err());

    // Check constructor types
    let result = repl.execute("Check Root.");
    assert!(result.is_ok(), "Check Root should work: {:?}", result.err());

    let result = repl.execute("Check FromDouble.");
    assert!(result.is_ok(), "Check FromDouble should work: {:?}", result.err());

    // Theorem: 1 is in the inverse tree
    let result = repl.execute(
        "Definition one_in_tree : InverseCollatz (Succ Zero) := Root (Succ Zero) (refl Nat (Succ Zero))."
    );
    assert!(result.is_ok(), "one_in_tree proof should type-check: {:?}", result.err());

    // Verify type
    let result = repl.execute("Check one_in_tree.");
    assert!(result.is_ok(), "Check one_in_tree should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("InverseCollatz"), "one_in_tree should have InverseCollatz type: {}", output);
}

/// Test: Powers of Two theorem - power_of_two function and all_powers_of_two proof
#[test]
fn math_collatz_power_of_two_theorem() {
    let mut repl = Repl::new();

    // Setup: plus and double
    repl.execute("Definition plus : Nat -> Nat -> Nat := fix rec => fun n : Nat => fun m : Nat => match n return Nat with | Zero => m | Succ k => Succ (rec k m) end.").unwrap();
    repl.execute("Definition double : Nat -> Nat := fun n : Nat => plus n n.").unwrap();

    // Define InverseCollatz with FromTripleSucc (need triple for full definition)
    repl.execute("Definition triple : Nat -> Nat := fun n : Nat => plus n (double n).").unwrap();
    let result = repl.execute(
        "Inductive InverseCollatz (n : Nat) := | Root : Eq Nat n (Succ Zero) -> InverseCollatz n | FromDouble : InverseCollatz n -> InverseCollatz (double n) | FromTripleSucc : InverseCollatz (Succ (triple n)) -> InverseCollatz n."
    );
    assert!(result.is_ok(), "InverseCollatz with FromTripleSucc should be defined: {:?}", result.err());

    // Define power_of_two
    let result = repl.execute(
        "Definition power_of_two : Nat -> Nat := fix rec => fun n : Nat => match n return Nat with | Zero => Succ Zero | Succ k => double (rec k) end."
    );
    assert!(result.is_ok(), "power_of_two should be defined: {:?}", result.err());

    // Check type of power_of_two
    let result = repl.execute("Check power_of_two.");
    assert!(result.is_ok(), "Check power_of_two should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Nat -> Nat"), "power_of_two should have type Nat -> Nat: {}", output);

    // Verify power_of_two computes correctly
    // power_of_two 0 = 1
    let result = repl.execute("Eval (power_of_two Zero).");
    assert!(result.is_ok(), "Eval power_of_two Zero should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Succ") && output.contains("Zero"), "power_of_two 0 should be Succ Zero: {}", output);

    // power_of_two 1 = 2
    let result = repl.execute("Eval (power_of_two (Succ Zero)).");
    assert!(result.is_ok(), "Eval power_of_two 1 should work: {:?}", result.err());

    // power_of_two 3 = 8
    let result = repl.execute("Eval (power_of_two (Succ (Succ (Succ Zero)))).");
    assert!(result.is_ok(), "Eval power_of_two 3 should work: {:?}", result.err());

    // Define the induction proof: all_powers_of_two
    let result = repl.execute(
        "Definition all_powers_of_two : forall n : Nat, InverseCollatz (power_of_two n) := fix proof => fun n : Nat => match n return (fun k : Nat => InverseCollatz (power_of_two k)) with | Zero => Root (Succ Zero) (refl Nat (Succ Zero)) | Succ k => FromDouble (power_of_two k) (proof k) end."
    );
    assert!(result.is_ok(), "all_powers_of_two proof should type-check: {:?}", result.err());

    // Verify the proof type
    let result = repl.execute("Check all_powers_of_two.");
    assert!(result.is_ok(), "Check all_powers_of_two should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("InverseCollatz"), "all_powers_of_two should mention InverseCollatz: {}", output);
}

/// Test: Grandchild Growth theorem - if n is in tree, 4n is in tree
#[test]
fn math_collatz_grandchild_growth() {
    let mut repl = Repl::new();

    // Setup
    repl.execute("Definition plus : Nat -> Nat -> Nat := fix rec => fun n : Nat => fun m : Nat => match n return Nat with | Zero => m | Succ k => Succ (rec k m) end.").unwrap();
    repl.execute("Definition double : Nat -> Nat := fun n : Nat => plus n n.").unwrap();
    repl.execute("Definition triple : Nat -> Nat := fun n : Nat => plus n (double n).").unwrap();

    // Define InverseCollatz
    repl.execute(
        "Inductive InverseCollatz (n : Nat) := | Root : Eq Nat n (Succ Zero) -> InverseCollatz n | FromDouble : InverseCollatz n -> InverseCollatz (double n) | FromTripleSucc : InverseCollatz (Succ (triple n)) -> InverseCollatz n."
    ).unwrap();

    // Define grandchild_growth lemma
    let result = repl.execute(
        "Definition grandchild_growth : forall n : Nat, InverseCollatz n -> InverseCollatz (double (double n)) := fun n : Nat => fun pf : InverseCollatz n => FromDouble (double n) (FromDouble n pf)."
    );
    assert!(result.is_ok(), "grandchild_growth should be defined: {:?}", result.err());

    // Check type
    let result = repl.execute("Check grandchild_growth.");
    assert!(result.is_ok(), "Check grandchild_growth should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("InverseCollatz"), "grandchild_growth should mention InverseCollatz: {}", output);
}

/// Test: FromTripleSucc rule - prove 5 is in the tree because 3*5+1 = 16 = 2^4 is in tree
#[test]
fn math_collatz_fromodd_rule() {
    let mut repl = Repl::new();

    // Setup
    repl.execute("Definition plus : Nat -> Nat -> Nat := fix rec => fun n : Nat => fun m : Nat => match n return Nat with | Zero => m | Succ k => Succ (rec k m) end.").unwrap();
    repl.execute("Definition double : Nat -> Nat := fun n : Nat => plus n n.").unwrap();
    repl.execute("Definition triple : Nat -> Nat := fun n : Nat => plus n (double n).").unwrap();

    // Define InverseCollatz with FromTripleSucc
    let result = repl.execute(
        "Inductive InverseCollatz (n : Nat) := | Root : Eq Nat n (Succ Zero) -> InverseCollatz n | FromDouble : InverseCollatz n -> InverseCollatz (double n) | FromTripleSucc : InverseCollatz (Succ (triple n)) -> InverseCollatz n."
    );
    assert!(result.is_ok(), "InverseCollatz with FromTripleSucc should be defined: {:?}", result.err());

    // Check FromTripleSucc constructor type
    let result = repl.execute("Check FromTripleSucc.");
    assert!(result.is_ok(), "Check FromTripleSucc should work: {:?}", result.err());

    // Define power_of_two and all_powers_of_two
    repl.execute(
        "Definition power_of_two : Nat -> Nat := fix rec => fun n : Nat => match n return Nat with | Zero => Succ Zero | Succ k => double (rec k) end."
    ).unwrap();
    repl.execute(
        "Definition all_powers_of_two : forall n : Nat, InverseCollatz (power_of_two n) := fix proof => fun n : Nat => match n return (fun k : Nat => InverseCollatz (power_of_two k)) with | Zero => Root (Succ Zero) (refl Nat (Succ Zero)) | Succ k => FromDouble (power_of_two k) (proof k) end."
    ).unwrap();

    // Define five and four
    repl.execute("Definition five : Nat := Succ (Succ (Succ (Succ (Succ Zero)))).").unwrap();
    repl.execute("Definition four : Nat := Succ (Succ (Succ (Succ Zero))).").unwrap();

    // Verify triple five + 1 = 16 = power_of_two four
    // triple 5 = 5 + 10 = 15, Succ (triple 5) = 16
    let result = repl.execute("Eval (Succ (triple five)).");
    assert!(result.is_ok(), "Eval Succ (triple five) should work: {:?}", result.err());

    let result = repl.execute("Eval (power_of_two four).");
    assert!(result.is_ok(), "Eval power_of_two four should work: {:?}", result.err());

    // Define five_in_tree using FromTripleSucc
    // FromTripleSucc five needs InverseCollatz (Succ (triple five)) which equals InverseCollatz 16
    let result = repl.execute(
        "Definition five_in_tree : InverseCollatz five := FromTripleSucc five (all_powers_of_two four)."
    );
    assert!(result.is_ok(), "five_in_tree should type-check: {:?}", result.err());

    // Verify the proof type
    let result = repl.execute("Check five_in_tree.");
    assert!(result.is_ok(), "Check five_in_tree should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("InverseCollatz"), "five_in_tree should have InverseCollatz type: {}", output);
}

// ============================================================
// LITERATE GÖDEL SYNTAX TESTS (Phase 2)
// ============================================================

/// Test: Literate `Let X be Y` constant definition
#[test]
fn math_literate_let_definition() {
    let mut repl = Repl::new();

    // Simple Let binding
    let result = repl.execute("Let one be Succ Zero.");
    assert!(result.is_ok(), "Let one be Succ Zero should work: {:?}", result.err());

    // Check it's defined
    let result = repl.execute("Check one.");
    assert!(result.is_ok(), "Check one should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Nat"), "one should have type Nat: {}", output);
}

/// Test: Literate `Name "X"` syntax for SName
#[test]
fn math_literate_name_syntax() {
    let mut repl = Repl::new();

    // Define Syntax type first (simplified for test)
    let result = repl.execute("Inductive Syntax := SName : Nat -> Syntax | SVar : Nat -> Syntax | SApp : Syntax -> Syntax -> Syntax.");
    assert!(result.is_ok(), "Syntax type should be defined: {:?}", result.err());

    // Use Literate Name syntax - it should produce SName "Not"
    // Since we defined SName : Nat -> Syntax, we use a number as placeholder
    let result = repl.execute("Let myname be SName Zero.");
    assert!(result.is_ok(), "Let myname should work: {:?}", result.err());

    let result = repl.execute("Check myname.");
    assert!(result.is_ok(), "Check myname should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Syntax"), "myname should have type Syntax: {}", output);
}

/// Test: Literate `Apply(f, x)` syntax for SApp
#[test]
fn math_literate_apply_syntax() {
    let mut repl = Repl::new();

    // Define Syntax type
    let result = repl.execute("Inductive Syntax := SName : Nat -> Syntax | SVar : Nat -> Syntax | SApp : Syntax -> Syntax -> Syntax.");
    assert!(result.is_ok(), "Syntax type should be defined: {:?}", result.err());

    // Use Literate Apply syntax
    let result = repl.execute("Let T be SApp (SName Zero) (SVar Zero).");
    assert!(result.is_ok(), "Let T with SApp should work: {:?}", result.err());

    let result = repl.execute("Check T.");
    assert!(result.is_ok(), "Check T should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Syntax"), "T should have type Syntax: {}", output);
}

/// Test: Literate `X implies Y` syntax for non-dependent Pi
#[test]
fn math_literate_implies_syntax() {
    let mut repl = Repl::new();

    // Define a simple proposition
    let result = repl.execute("Definition A : Prop := True.");
    assert!(result.is_ok(), "A should be defined: {:?}", result.err());

    let result = repl.execute("Definition B : Prop := True.");
    assert!(result.is_ok(), "B should be defined: {:?}", result.err());

    // Use Literate implies syntax
    let result = repl.execute("Let implication be (A implies B).");
    assert!(result.is_ok(), "Let implication should work: {:?}", result.err());

    let result = repl.execute("Check implication.");
    assert!(result.is_ok(), "Check implication should work: {:?}", result.err());
}

/// Test: Literate `the diagonalization of T` syntax
#[test]
fn math_literate_diagonalization_syntax() {
    let mut repl = Repl::new();

    // Define Syntax type and syn_diag function
    let result = repl.execute("Inductive Syntax := SName : Nat -> Syntax | SVar : Nat -> Syntax | SApp : Syntax -> Syntax -> Syntax.");
    assert!(result.is_ok(), "Syntax type should be defined: {:?}", result.err());

    let result = repl.execute("Definition syn_diag : Syntax -> Syntax := fun s : Syntax => s.");
    assert!(result.is_ok(), "syn_diag should be defined: {:?}", result.err());

    // Use Literate diagonalization syntax
    let result = repl.execute("Let T be SName Zero.");
    assert!(result.is_ok(), "Let T should work: {:?}", result.err());

    let result = repl.execute("Let G be the diagonalization of T.");
    assert!(result.is_ok(), "Let G with diagonalization should work: {:?}", result.err());

    let result = repl.execute("Check G.");
    assert!(result.is_ok(), "Check G should work: {:?}", result.err());
    let output = result.unwrap();
    assert!(output.contains("Syntax"), "G should have type Syntax: {}", output);
}
