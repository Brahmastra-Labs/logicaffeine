use super::ProofStatus;

#[test]
fn an_uncertified_derivation_is_not_ready_and_not_found() {
    let (class, text) = ProofStatus::Uncertified("kernel rejected it".into()).line();
    assert!(text.contains("not certified"), "{text}");
    assert!(!text.contains("Ready") && !text.contains("found"), "{text}");
    assert_eq!(class, "proof-status uncertified");
}

#[test]
fn each_status_has_its_own_line() {
    let lines = [
        ProofStatus::Idle.line(),
        ProofStatus::Proving.line(),
        ProofStatus::Success.line(),
        ProofStatus::Uncertified(String::new()).line(),
        ProofStatus::Failed("no".into()).line(),
    ];
    for (i, a) in lines.iter().enumerate() {
        for b in &lines[i + 1..] {
            assert_ne!(a.0, b.0, "two statuses share a class");
            assert_ne!(a.1, b.1, "two statuses share a line");
        }
    }
}
