use logicaffeine_kernel::interface::lean as adapter;
#[test]
fn checks_the_official_dependent_motive_declaration() {
    // Preserve the complete pinned input; this prefix isolates the first
    // declaration whose motive domain needs beta conversion.
    let input = include_str!("fixtures/lean_arena/init-prelude.ndjson")
        .lines()
        .take(4344)
        .collect::<Vec<_>>()
        .join("\n");
    adapter::check_str(&input).unwrap();
}

#[test]
fn checks_the_official_nat_mod_core_proof() {
    // Includes Nat.modCoreGo_lt and all preceding original declarations.
    // The full pinned input remains in the separate Arena ratchet.
    let input = include_str!("fixtures/lean_arena/init-prelude.ndjson")
        .lines()
        .take(9462)
        .collect::<Vec<_>>()
        .join("\n");
    adapter::check_str(&input).unwrap();
}
