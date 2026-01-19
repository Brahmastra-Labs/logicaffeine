//! Phase: Temporal Primitives
//!
//! Tests for kernel-level temporal literals: Duration, Date, Moment.
//! These are the foundational types for the temporal system.

/// Test 1.1: Duration literal exists in kernel
#[test]
fn duration_literal_exists_in_kernel() {
    use logicaffeine_kernel::Literal;

    // Duration stored as nanoseconds (i64) - signed allows negative offsets
    let d = Literal::Duration(500_000_000); // 500ms
    assert!(matches!(d, Literal::Duration(_)));

    // Negative durations are valid ("5 minutes early")
    let early = Literal::Duration(-300_000_000_000); // -5 minutes
    assert!(matches!(early, Literal::Duration(_)));
}

/// Test 1.2: Date literal exists in kernel
#[test]
fn date_literal_exists_in_kernel() {
    use logicaffeine_kernel::Literal;

    // Date as days since Unix epoch (i32) - range: ±5.8 million years
    let d = Literal::Date(20594); // 2026-05-20
    assert!(matches!(d, Literal::Date(_)));
}

/// Test 1.3: Moment literal exists in kernel
#[test]
fn moment_literal_exists_in_kernel() {
    use logicaffeine_kernel::Literal;

    // Moment as nanoseconds since Unix epoch (i64) - matches Duration precision
    let m = Literal::Moment(1779494400_000_000_000); // 2026-05-20T00:00:00Z
    assert!(matches!(m, Literal::Moment(_)));
}

/// Test 1.4: Temporal types registered in prelude
#[test]
fn temporal_types_registered_in_prelude() {
    use logicaffeine_kernel::Context;
    use logicaffeine_kernel::prelude::StandardLibrary;

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Physical time types (kernel primitives)
    assert!(ctx.get_global("Duration").is_some(), "Duration type should be in prelude");
    assert!(ctx.get_global("Date").is_some(), "Date type should be in prelude");
    assert!(ctx.get_global("Moment").is_some(), "Moment type should be in prelude");
}

/// Test 1.5: Duration literal displays correctly
#[test]
fn duration_literal_displays_correctly() {
    use logicaffeine_kernel::Literal;

    let d = Literal::Duration(500_000_000); // 500ms
    let display = format!("{}", d);
    // The display should be human-readable (e.g. "500ms") or at least contain the value
    assert!(display.contains("500") || display.contains("ms"), "Duration display should be readable: {}", display);
}

/// Test 1.6: Date literal displays correctly
#[test]
fn date_literal_displays_correctly() {
    use logicaffeine_kernel::Literal;

    // 2026-05-20 = 20593 days since epoch
    let d = Literal::Date(20593);
    let display = format!("{}", d);
    assert!(display.contains("2026-05-20"), "Date display should show ISO format: {}", display);
}

/// Test 1.7: Moment literal displays correctly
#[test]
fn moment_literal_displays_correctly() {
    use logicaffeine_kernel::Literal;

    // 2026-05-20T00:00:00Z in nanoseconds since epoch
    // 20593 days * 24 * 60 * 60 * 1_000_000_000 = 1779235200000000000
    let m = Literal::Moment(1779235200_000_000_000);
    let display = format!("{}", m);
    assert!(display.contains("2026-05-20"), "Moment display should show date portion: {}", display);
}
