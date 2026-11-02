//! `stale_rows` in isolation, against synthetic ledgers — no real page needs to parse or
//! refuse to prove the exemption rule; a scripted `found`/`unparsed_pages`/`refused_at_zero`
//! makes every case exact and mutation-provable.
use super::stale_rows;
use std::collections::{BTreeMap, BTreeSet};

fn row(id: &str, kind: &str, at: usize, cause: &str) -> ((String, String, usize), String) {
    ((id.to_string(), kind.to_string(), at), cause.to_string())
}

#[test]
fn a_row_whose_scenario_actually_agreed_is_stale() {
    let known = BTreeMap::from([row("p.lg::t", "document", 3, "was different")]);
    let found = BTreeMap::new(); // the run reached statement 3 and found no divergence there
    let stale = stale_rows(&known, &found, &[], &BTreeSet::new());
    assert_eq!(
        stale,
        vec![("p.lg::t".to_string(), "document".to_string(), 3)]
    );
}

#[test]
fn a_row_whose_page_did_not_parse_is_not_stale() {
    let known = BTreeMap::from([row("p.lg::t", "document", 3, "was different")]);
    let found = BTreeMap::new();
    let stale = stale_rows(&known, &found, &["p.lg".to_string()], &BTreeSet::new());
    assert!(
        stale.is_empty(),
        "a page that cannot even be parsed proves nothing about statement 3: {stale:?}"
    );
}

#[test]
fn a_row_whose_scenario_refused_at_statement_zero_is_not_stale() {
    let known = BTreeMap::from([row("p.lg::t", "document", 3, "was different")]);
    let found = BTreeMap::from([row("p.lg::t", "refused", 0, "no component named App")]);
    let refused = BTreeSet::from(["p.lg::t".to_string()]);
    let stale = stale_rows(&known, &found, &[], &refused);
    assert!(
        stale.is_empty(),
        "refused before statement 0 ran, statement 3 was never reached: {stale:?}"
    );
}

#[test]
fn a_row_whose_scenario_refused_at_a_later_statement_can_still_be_stale() {
    // Refused at statement 5 means statements 0..5 DID run — a row at statement 3 was
    // genuinely checked and may genuinely have started agreeing.
    let known = BTreeMap::from([row("p.lg::t", "document", 3, "was different")]);
    let found = BTreeMap::from([row("p.lg::t", "refused", 5, "some later gap")]);
    let stale = stale_rows(&known, &found, &[], &BTreeSet::new());
    assert_eq!(
        stale,
        vec![("p.lg::t".to_string(), "document".to_string(), 3)]
    );
}

#[test]
fn an_unparsed_page_does_not_exempt_a_different_pages_row_with_a_shared_prefix() {
    // "p.lg" must not accidentally match "p.lg.bak::t" — a real different page's row.
    let known = BTreeMap::from([row("p.lg.bak::t", "document", 3, "was different")]);
    let found = BTreeMap::new();
    let stale = stale_rows(&known, &found, &["p.lg".to_string()], &BTreeSet::new());
    assert_eq!(
        stale,
        vec![("p.lg.bak::t".to_string(), "document".to_string(), 3)],
        "a prefix match must land on a `::` boundary, not any shared characters"
    );
}

#[test]
fn a_refused_at_zero_scenario_does_not_exempt_a_different_scenario_of_the_same_page() {
    let known = BTreeMap::from([row("p.lg::other test", "document", 3, "was different")]);
    let found = BTreeMap::new();
    let refused = BTreeSet::from(["p.lg::t".to_string()]);
    let stale = stale_rows(&known, &found, &[], &refused);
    assert_eq!(
        stale,
        vec![("p.lg::other test".to_string(), "document".to_string(), 3)],
        "only the scenario that actually refused at 0 is exempt, not its whole page"
    );
}
