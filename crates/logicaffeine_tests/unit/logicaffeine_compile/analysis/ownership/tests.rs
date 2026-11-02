use super::*;

#[test]
fn test_ownership_checker_basic() {
    let interner = Interner::new();
    let checker = OwnershipChecker::new(&interner);
    assert!(checker.state.is_empty());
}
