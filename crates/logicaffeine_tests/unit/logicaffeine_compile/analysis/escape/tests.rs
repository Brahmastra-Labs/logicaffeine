use super::*;

// Note: Full integration tests are in tests/phase85_zones.rs
// These unit tests verify the basic mechanics of the escape checker

#[test]
fn test_escape_checker_basic() {
    use crate::intern::Interner;

    let interner = Interner::new();
    let checker = EscapeChecker::new(&interner);

    // Just verify creation works
    assert_eq!(checker.current_depth, 0);
    assert!(checker.zone_depth.is_empty());
}
