use logicaffeine_kernel::interface::lean as adapter;

#[test]
fn checks_the_official_char_declaration_without_evaluating_its_validity_predicate() {
    // The pinned export's Char block ends at line 3350. Keep the upstream file
    // unchanged and test the self-contained prefix as a reduced reproducer.
    let input = include_str!("fixtures/lean_arena/init-prelude.ndjson")
        .lines()
        .take(3350)
        .collect::<Vec<_>>()
        .join("\n");
    adapter::check_str(&input).unwrap();
}
