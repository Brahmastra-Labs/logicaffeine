//! A MIXED DOCUMENT'S KERNEL MATH IS THE KERNEL'S, AND ITS REFUSAL IS THE PROGRAM'S.
//!
//! A mixed document is split into two streams (`partition_mixed`): the kernel's vernacular —
//! Coq-style sentences, and literate `## Theorem:` blocks whose name is followed by the kernel's
//! `Statement:` — and everything else, which the language reads. The kernel runs its stream and
//! extracts it into `mod proven`; a statement it refuses refuses the program on both compile doors,
//! naming the statement and quoting the kernel.
//!
//! The language's own theorem forms never reach the kernel: an English `## Theorem:` (`Given:` /
//! `Prove:`) is the proof engine's, and the body of a block the language reads as text (`## Theory`,
//! `## Axiom`) is the language's even where one of its lines opens with a kernel keyword.

use logicaffeine_compile::{compile_to_rust, partition_mixed};
use logicaffeine_language::error::ParseErrorKind;

const REFUSED: &str = "\
Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.
Definition bad_eq : Eq MyNat MZero (MSucc MZero) := refl MyNat MZero.

## Main
Require that check_bad_eq().
Show \"unreachable\".
";

const ACCEPTED: &str = "\
Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.
Definition good_eq : Eq MyNat (MSucc MZero) (MSucc MZero) := refl MyNat (MSucc MZero).

## Main
Require that check_good_eq().
Show \"holds\".
";

/// The proof engine's theorem: English premises and goal.
const ENGLISH_THEOREM: &str = "\
## Theorem: Socrates_Mortality
Given: Socrates is a man.
Given: Every man is mortal.
Prove: Socrates is mortal.
";

/// A formal development: its `Axiom` and `Theorem` lines are the language's theory notation.
const THEORY: &str = "\
## Theory Tarski

Axiom pseudo_reflexivity: for all a b, Cong(a, b, b, a).
Axiom inner_transitivity: for all a b c d e f, if Cong(a, b, c, d) and Cong(a, b, e, f) then Cong(c, d, e, f).

Theorem reflexivity: prove for all a b, Cong(a, b, a, b).
";

/// The kernel's literate theorem, stating a falsehood.
const LITERATE_REFUSED: &str = "\
## Theorem: FiveLessThanTwo
    Statement: 5 < 2.
    Proof: lia.

## Main
Show \"unreachable\".
";

fn message(kind: &ParseErrorKind) -> String {
    match kind {
        ParseErrorKind::Custom(m) => m.clone(),
        other => format!("{other:?}"),
    }
}

fn assert_kernel_words(what: &str, kind: &ParseErrorKind) {
    let m = message(kind);
    assert!(
        m.contains("bad_eq"),
        "{what}: the refusal names the statement: {m}"
    );
    assert!(
        m.contains("Type mismatch"),
        "{what}: the refusal is the kernel's own: {m}"
    );
    assert!(
        !m.contains("Unknown function"),
        "{what}: not a symptom three layers away: {m}"
    );
}

/// `source` holds no kernel math: the split hands the language all of it, unchanged, and both
/// compile doors build it.
fn assert_the_languages(what: &str, source: &str) {
    let (imperative, math) = partition_mixed(source);
    assert_eq!(
        math, None,
        "{what}: nothing here is the kernel's:\n{source}"
    );
    assert_eq!(
        imperative, source,
        "{what}: the language reads the whole document"
    );
    for (door, built) in [
        ("compile_to_rust", compile_to_rust(source)),
        (
            "generate_rust_code",
            logicaffeine_compile::generate_rust_code(source),
        ),
    ] {
        let rust =
            built.unwrap_or_else(|e| panic!("{what}: {door} builds it: {}", message(&e.kind)));
        assert!(
            !rust.contains("compile_error!"),
            "{what}: {door} builds it without a refusal:\n{rust}"
        );
    }
}

#[test]
fn a_refused_math_statement_refuses_the_program_on_the_aot_door() {
    let refused = compile_to_rust(REFUSED).expect_err("the math half does not check");
    assert_kernel_words("compile_to_rust", &refused.kind);
}

#[test]
fn a_refused_math_statement_refuses_the_program_on_the_studio_door() {
    let refused = logicaffeine_compile::generate_rust_code(REFUSED)
        .expect_err("the math half does not check");
    assert_kernel_words("generate_rust_code", &refused.kind);
}

#[test]
fn a_mixed_document_whose_math_checks_still_compiles() {
    let rust = compile_to_rust(ACCEPTED).expect("the math half checks");
    assert!(rust.contains("pub fn check_good_eq"), "{rust}");
    let rust = logicaffeine_compile::generate_rust_code(ACCEPTED).expect("the math half checks");
    assert!(rust.contains("pub fn check_good_eq"), "{rust}");
}

#[test]
fn an_english_theorem_is_the_proof_engines_and_never_reaches_the_kernel() {
    assert_the_languages("an English theorem", ENGLISH_THEOREM);
    assert_the_languages(
        "an English theorem with a proof line",
        "## Theorem: Add_Zero\nProve: For all n, n plus 0 equals n.\nProof: Auto.\n",
    );
    assert_the_languages(
        "an English theorem beside a program",
        &format!("{ENGLISH_THEOREM}\n## Main\nShow \"ran\".\n"),
    );
}

#[test]
fn a_theory_development_is_the_languages_and_never_reaches_the_kernel() {
    assert_the_languages("a ## Theory development", THEORY);
}

/// The blocks the language reads as raw text: each `parse_<word>_block` whose body calls
/// `collect_block_text()`, read from the parser's own source so a block that starts reading its
/// body as text is held here the day it does.
fn language_text_blocks() -> Vec<String> {
    let parser = include_str!("../../logicaffeine_language/src/parser/mod.rs");
    let mut words = Vec::new();
    for piece in parser.split("\n    fn parse_").skip(1) {
        let Some((word, rest)) = piece.split_once("_block(") else {
            continue;
        };
        if !word.chars().all(|c| c.is_ascii_lowercase()) {
            continue;
        }
        let body = rest.split("\n    fn ").next().unwrap_or(rest);
        if body.contains("collect_block_text()") {
            words.push(word.to_string());
        }
    }
    words
}

#[test]
fn every_block_the_language_reads_as_text_is_closed_to_the_kernel() {
    let blocks = language_text_blocks();
    for expected in ["axiom", "theory"] {
        assert!(
            blocks.iter().any(|b| b == expected),
            "the census reaches `## {expected}`; it found {blocks:?}"
        );
    }
    for word in &blocks {
        let header = format!("{}{}", word[..1].to_ascii_uppercase(), &word[1..]);
        let source = format!(
            "## {header} Probe\n\n\
             Axiom a: for all x, P(x).\n\
             Theorem t: prove for all x, P(x).\n\
             Definition d : Nat := Zero.\n\
             Inductive I := C : I.\n\
             Fixpoint f (n : Nat) : Nat := n.\n\
             Lemma l: prove for all x, P(x).\n"
        );
        let (imperative, math) = partition_mixed(&source);
        assert_eq!(
            math, None,
            "`## {header}` reads its body as text; none of it is the kernel's:\n{source}"
        );
        assert_eq!(
            imperative, source,
            "`## {header}`: the language reads the whole block"
        );
    }
}

#[test]
fn a_literate_kernel_theorem_reaches_the_kernel_and_its_refusal_is_the_programs() {
    let (imperative, math) = partition_mixed(LITERATE_REFUSED);
    let math =
        math.expect("a `## Theorem:` whose name is followed by `Statement:` is the kernel's");
    assert!(
        math.contains("Statement: 5 < 2."),
        "the kernel's stream holds the theorem whole:\n{math}"
    );
    assert!(
        !imperative.contains("FiveLessThanTwo"),
        "the language never sees it:\n{imperative}"
    );
    for (door, built) in [
        ("compile_to_rust", compile_to_rust(LITERATE_REFUSED)),
        (
            "generate_rust_code",
            logicaffeine_compile::generate_rust_code(LITERATE_REFUSED),
        ),
    ] {
        let refused = built.expect_err("5 < 2 does not check");
        let m = message(&refused.kind);
        assert!(
            m.contains("the kernel refused") && m.contains("FiveLessThanTwo"),
            "{door}: the kernel's refusal, naming the theorem: {m}"
        );
    }
    let one_line = "## Theorem: FiveLessThanTwo Statement: 5 < 2. Proof: lia.\n\n## Main\nShow \"unreachable\".\n";
    let (_, math) = partition_mixed(one_line);
    assert!(
        math.is_some_and(|m| m.contains("Statement: 5 < 2.")),
        "the literate form written on one line is the kernel's too"
    );
}

#[test]
fn an_english_theorem_beside_a_refused_kernel_statement_leaves_the_refusal_the_kernels() {
    let source = format!("{ENGLISH_THEOREM}\n{REFUSED}");
    let (imperative, math) = partition_mixed(&source);
    let math = math.expect("the Coq-style sentences are the kernel's");
    assert!(
        math.contains("bad_eq") && !math.contains("Socrates"),
        "only the vernacular reaches the kernel:\n{math}"
    );
    assert!(
        imperative.contains("## Theorem: Socrates_Mortality")
            && imperative.contains("Prove: Socrates is mortal."),
        "the English theorem stays whole in the language's stream:\n{imperative}"
    );
    let refused = compile_to_rust(&source).expect_err("the math half does not check");
    assert_kernel_words("compile_to_rust beside an English theorem", &refused.kind);
}
