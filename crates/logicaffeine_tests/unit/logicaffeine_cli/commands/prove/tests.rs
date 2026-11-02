use super::*;

#[test]
fn splitter_finds_each_theorem_block() {
    let src = "## Note\n\nx\n\n## Theorem: A\nGiven: p.\nProve: p.\n\n## Theorem: B\nProve: q.\n\n## Main\nShow 1.\n";
    let blocks = split_theorem_blocks(src);
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].starts_with("## Theorem: A"));
    assert!(blocks[0].contains("Given: p."));
    assert!(!blocks[0].contains("## Theorem: B"));
    assert!(blocks[1].starts_with("## Theorem: B"));
    assert!(
        !blocks[1].contains("Show 1."),
        "Main must not leak into a block"
    );
}

#[test]
fn splitter_ignores_theory_blocks() {
    let src = "## Theory T\n\nAxiom a: for all x, P(x).\nTheorem t: prove for all x, P(x).\n";
    assert!(split_theorem_blocks(src).is_empty());
}

#[test]
fn splitter_requires_a_word_boundary_after_theorem() {
    // `## Theorems overview` is prose, not a theorem block.
    let src = "## Theorems overview\n\nSome prose.\n\n## Theorem: Real\nProve: p.\n";
    let blocks = split_theorem_blocks(src);
    assert_eq!(blocks.len(), 1, "only the real block: {blocks:?}");
    assert!(blocks[0].starts_with("## Theorem: Real"));
}

#[test]
fn theorem_name_extracts_after_colon() {
    assert_eq!(
        theorem_name("## Theorem: Socrates\nProve: x.\n"),
        "Socrates"
    );
    assert_eq!(theorem_name("## Theorem\nProve: x.\n"), "theorem");
}
