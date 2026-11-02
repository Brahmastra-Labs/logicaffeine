use super::*;

#[test]
fn classify_recognizes_block_families() {
    assert_eq!(classify("To double (n: Int) -> Int:"), BlockKind::Function);
    assert_eq!(classify("A Point has"), BlockKind::TypeDef);
    assert_eq!(classify("An Agent has"), BlockKind::TypeDef);
    assert_eq!(classify("Note"), BlockKind::Note);
    assert_eq!(classify("Example"), BlockKind::Example);
    assert_eq!(classify("Definition"), BlockKind::Definition);
    assert_eq!(classify("Theorem pythagoras"), BlockKind::Formal);
    assert_eq!(classify("Main"), BlockKind::Skip);
    assert_eq!(classify("Requires"), BlockKind::Skip);
    assert_eq!(classify("Policy access"), BlockKind::Other);
}

#[test]
fn extract_blocks_keeps_order_and_bodies() {
    let src = "# Title\n\nintro prose\n\n## To f:\n    Return 1.\n\n## Note\n\nhello\n";
    let (preamble, blocks) = extract_blocks(src);
    assert_eq!(preamble, "intro prose");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].kind, BlockKind::Function);
    assert!(blocks[0].body.contains("Return 1."));
    assert_eq!(blocks[1].kind, BlockKind::Note);
    assert!(blocks[1].body.contains("hello"));
}

#[test]
fn render_omits_main_and_fences_examples() {
    let src = "## Example\n\n    Show 1.\n\n## Main\n\n    Show 99.\n";
    let md = render_markdown("demo", None, src);
    assert!(md.contains("```logos\nShow 1.\n```"));
    assert!(!md.contains("Show 99."));
}

#[test]
fn import_declarations_are_omitted_from_the_preamble() {
    // The assembled unit carries the entry's `Import …` line in its abstract; it
    // is build plumbing (the module is already spliced in), so it must NOT render
    // as documentation prose — while genuine abstract prose is kept.
    let src =
        "# App\n\nImport Util.\n\nThis app greets you.\n\n## To greet ():\n    Show \"hi\".\n";
    let (preamble, blocks) = extract_blocks(src);
    assert!(
        !preamble.contains("Import Util."),
        "the import line is omitted: {preamble:?}"
    );
    assert!(
        preamble.contains("This app greets you."),
        "real prose is kept: {preamble:?}"
    );
    assert_eq!(blocks.len(), 1, "the greet function is documented");

    // The full render carries no stray import directive either.
    let md = render_markdown("app", None, src);
    assert!(
        !md.contains("Import Util."),
        "no import directive leaks into the docs:\n{md}"
    );
    assert!(
        md.contains("### To greet"),
        "the function still documents:\n{md}"
    );
}

#[test]
fn an_import_inside_a_fenced_example_survives() {
    // An `Import` line INSIDE a fenced code example is content, not a directive.
    let src = "# App\n\n```\nImport Util.\n```\n\n## Note\n\ndocs\n";
    let (preamble, _) = extract_blocks(src);
    assert!(
        preamble.contains("Import Util."),
        "fenced example content is preserved: {preamble:?}"
    );
}

#[test]
fn fenced_code_never_starts_blocks() {
    // A `## Main` INSIDE a fenced example is content, not a header.
    let src = "## Note\n\nUsage:\n\n```\n## Main\nShow 1.\n```\n\nafter the fence.\n";
    let (_, blocks) = extract_blocks(src);
    assert_eq!(blocks.len(), 1, "one Note block only: {blocks:?}");
    assert!(
        blocks[0].body.contains("## Main"),
        "fence content preserved"
    );
    assert!(blocks[0].body.contains("after the fence."));
}
