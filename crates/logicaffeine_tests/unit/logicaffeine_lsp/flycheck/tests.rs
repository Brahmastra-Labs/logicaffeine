use super::*;

struct NoRunner;
impl FlycheckRunner for NoRunner {
    fn check(&self, _: &str, _: &str) -> Option<Vec<FlycheckFinding>> {
        None
    }
}

#[test]
fn clear_reports_whether_findings_were_dropped() {
    let fc = Flycheck::new(Box::new(NoRunner));
    let uri = Url::parse("file:///x.lg").unwrap();

    // Nothing stored → nothing to retract.
    assert!(!fc.clear(&uri), "no findings → clear reports false");

    // Store a completed run at the current generation.
    let generation = fc.begin_save(&uri);
    let stored = fc.complete(
        &uri,
        generation,
        vec![FlycheckFinding {
            message: "x".into(),
            suggestion: None,
            span: None,
        }],
        "## Main\n",
        &[],
    );
    assert!(stored.is_some(), "a run at the current generation stores");

    // Now clear DOES drop findings → true, and is idempotent afterward.
    assert!(
        fc.clear(&uri),
        "stored findings → clear reports true (retract now)"
    );
    assert!(!fc.clear(&uri), "already cleared → clear reports false");
}
