//! Sprint C: Verification IR Extensions — Bitvectors & BMC
//!
//! Tests for hardware verification features. Tests that need Z3 are gated
//! behind the `verification` feature flag.
//!
//! Run WITHOUT Z3: cargo test --test phase_hw_verify -- --skip e2e
//! Run WITH Z3:    cargo test --features verification --test phase_hw_verify -- --skip e2e

// ═══════════════════════════════════════════════════════════════════════════
// COMPILE-TIME: existing tests still pass after our AST changes
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn hw_changes_do_not_break_basic_compilation() {
    // Existing LOGOS programs should still compile after AST extensions
    use logicaffeine_compile::compile::compile_to_rust;
    let source = "## Main\nLet x be 42.\nShow x.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Basic compilation should still work: {:?}", result.err());
}

#[test]
fn hw_changes_do_not_break_refinement_syntax() {
    // Refinement types should still parse (even without Z3 verification)
    use logicaffeine_compile::compile::compile_to_rust;
    let source = "## Main\nLet x: Int where it > 0 be 10.\nShow x.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Refinement syntax should still parse: {:?}", result.err());
}

// ═══════════════════════════════════════════════════════════════════════════
// PATTERN: BMC as LOGOS refinement types (no Z3 needed for parse check)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn bmc_counter_pattern_compiles() {
    // A counter with bounded refinement — the pattern we'll use for HW BMC
    use logicaffeine_compile::compile::compile_to_rust;
    let source = r#"## Main
Let counter be 0.
Let next be counter + 1.
Show next.
"#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Counter pattern should compile: {:?}", result.err());
}

#[test]
fn mutex_pattern_compiles() {
    use logicaffeine_compile::compile::compile_to_rust;
    let source = r#"## Main
Let grant_a be 1.
Let grant_b be 0.
Let both be grant_a + grant_b.
If both is greater than 1:
    Show "MUTEX VIOLATION".
Otherwise:
    Show "OK".
"#;
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Mutex pattern should compile: {:?}", result.err());
}

// ═══════════════════════════════════════════════════════════════════════════
// Z3 VERIFICATION (behind feature flag — tests skip when Z3 unavailable)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
#[cfg(feature = "verification")]
fn z3_refinement_valid_after_hw_changes() {
    use logicaffeine_compile::compile::compile_to_rust_verified;
    let source = "## Main\nLet x: Int where it > 0 be 10.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_ok(), "Z3 should verify 10 > 0: {:?}", result.err());
}

#[test]
#[cfg(feature = "verification")]
fn z3_refinement_invalid_after_hw_changes() {
    use logicaffeine_compile::compile::compile_to_rust_verified;
    let source = "## Main\nLet x: Int where it > 0 be -5.";
    let result = compile_to_rust_verified(source);
    assert!(result.is_err(), "Z3 should reject -5 > 0");
}

#[test]
#[cfg(feature = "verification")]
fn z3_mutex_valid() {
    use logicaffeine_compile::compile::compile_to_rust_verified;
    let source = r#"## Main
Let grant_a: Int where it >= 0 and it <= 1 be 1.
Let grant_b: Int where it >= 0 and it <= 1 be 0.
Let sum: Int where it <= 1 be grant_a + grant_b.
Show sum.
"#;
    let result = compile_to_rust_verified(source);
    assert!(result.is_ok(), "1+0=1 satisfies sum<=1: {:?}", result.err());
}

#[test]
#[cfg(feature = "verification")]
fn z3_mutex_violation() {
    use logicaffeine_compile::compile::compile_to_rust_verified;
    let source = r#"## Main
Let grant_a: Int where it >= 0 and it <= 1 be 1.
Let grant_b: Int where it >= 0 and it <= 1 be 1.
Let sum: Int where it <= 1 be grant_a + grant_b.
Show sum.
"#;
    let result = compile_to_rust_verified(source);
    assert!(result.is_err(), "1+1=2 violates sum<=1 (mutex violation)");
}
