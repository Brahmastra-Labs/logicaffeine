use super::*;

#[test]
fn test_session_basic() {
    let mut session = Session::new();
    let out = session.eval("John walked.").unwrap();
    assert!(out.contains("Walk"), "Should have Walk predicate");
}

#[test]
fn test_session_multiple_sentences() {
    let mut session = Session::new();
    session.eval("John walked.").unwrap();
    session.eval("Mary ran.").unwrap();

    assert_eq!(session.turn_count(), 2);

    let history = session.history();
    assert!(history.contains("Walk"));
    assert!(history.contains("Run"));
    assert!(history.contains("Precedes"));
}
