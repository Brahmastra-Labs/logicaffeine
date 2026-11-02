//! ★ LOCK — a component test is legible as one in the claim inventory. ★
//!
//! `largo test --json` classifies every `## Test` by KIND, so a green run is legible about what it
//! actually proved rather than just how many things passed. A `For all` is a `property` with its
//! domain disclosed; a `## Theorem` is a `theorem`; anything else was an `example`.
//!
//! ★ BUT A COMPONENT TEST IS NOT AN EXAMPLE. It MOUNTS a page, drives it with clicks and
//! keystrokes, and asserts about a rendered document — a different kind of evidence, that fails in
//! different ways and covers a different risk. Reporting it as an `example` told a reader that a
//! run of twenty examples had checked twenty values, when some of them had checked a PAGE.
//!
//! The distinction is not cosmetic: it is what lets somebody ask "does this project test its UI at
//! all?" and get an answer, which is exactly the question a claim inventory exists to answer.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr, stdout};

/// Run `largo test --json` over a source file and hand back the inventory document.
fn inventory(source: &str) -> serde_json::Value {
    let dir = tempfile::tempdir().expect("temp dir");
    let out = largo_in(dir.path(), &["new", "proj"]);
    assert!(out.status.success(), "largo new: {}", stderr(&out));
    let root = dir.path().join("proj");
    std::fs::write(root.join("src/main.lg"), source).expect("write source");
    let out = largo_in(&root, &["test", "--json"]);
    let text = stdout(&out);
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("the inventory is JSON: {e}\n{text}\n{}", stderr(&out)))
}

fn kinds(inv: &serde_json::Value) -> Vec<(String, String)> {
    inv["inventory"]
        .as_array()
        .expect("the inventory lists items")
        .iter()
        .map(|i| {
            (
                i["name"].as_str().unwrap_or_default().to_string(),
                i["kind"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

const PAGE: &str = "\
## Component Counter
State:
    Let clicks be 0.
Actions:
    To bump:
        Set clicks to clicks + 1.
View:
    <p id=\"n\">{clicks}</p>
    <button id=\"b\" on-click={bump}>+</button>
";

#[test]
fn a_test_that_mounts_a_page_is_a_component_claim() {
    // ★ THE DISTINCTION. This test drives a DOM; the one below checks a value. An inventory that
    // called both `example` cannot answer "is the UI tested?".
    let inv = inventory(&format!(
        "{PAGE}\n## Test: clicking counts\n    Mount Counter.\n    Click \"#b\".\n    \
         Expect that the text of \"#n\" is equal to \"1\".\n"
    ));
    let items = kinds(&inv);
    assert!(
        items
            .iter()
            .any(|(n, k)| n.contains("clicking") && k == "component"),
        "a test that mounts a page is a `component` claim: {items:?}"
    );
}

#[test]
fn a_plain_value_test_is_still_an_example() {
    // The other half — the new kind must not swallow the old one. A test asserting a value is an
    // example, and calling it anything else would make the inventory less legible, not more.
    let inv = inventory("## Test: arithmetic\n    Expect that 2 + 2 is equal to 4.\n");
    let items = kinds(&inv);
    assert!(
        items
            .iter()
            .any(|(n, k)| n.contains("arithmetic") && k == "example"),
        "a value test is still an `example`: {items:?}"
    );
}

#[test]
fn the_two_kinds_coexist_in_one_run() {
    // A real project has both, and the inventory has to tell them apart IN THE SAME DOCUMENT —
    // which is where a classification that keyed on the run rather than the test would break.
    let inv = inventory(&format!(
        "{PAGE}\n## Test: clicking counts\n    Mount Counter.\n    Click \"#b\".\n    \
         Expect that the text of \"#n\" is equal to \"1\".\n\n\
         ## Test: arithmetic\n    Expect that 2 + 2 is equal to 4.\n"
    ));
    let items = kinds(&inv);
    assert!(
        items
            .iter()
            .any(|(n, k)| n.contains("clicking") && k == "component"),
        "the component claim: {items:?}"
    );
    assert!(
        items
            .iter()
            .any(|(n, k)| n.contains("arithmetic") && k == "example"),
        "…and the example beside it: {items:?}"
    );
}

#[test]
fn a_component_test_is_finite_testing_and_never_a_proof() {
    // The honesty every kind in this inventory carries. Mounting a page and clicking it checks the
    // cases you drove and nothing else — a page that works for one input is not a page that works.
    let inv = inventory(&format!(
        "{PAGE}\n## Test: clicking counts\n    Mount Counter.\n    Click \"#b\".\n    \
         Expect that the text of \"#n\" is equal to \"1\".\n"
    ));
    let item = inv["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"].as_str().unwrap_or_default().contains("clicking"))
        .expect("the component claim is listed");
    assert_eq!(
        item["proof"],
        serde_json::json!(false),
        "never a proof: {item}"
    );
    assert_eq!(
        item["finite_enumeration"],
        serde_json::json!(true),
        "it checked the cases it drove: {item}"
    );
}

#[test]
fn a_component_test_that_asserts_nothing_is_still_flagged_vacuous() {
    // ★ THE NEW KIND MUST NOT BECOME A HIDING PLACE. The vacuous-green gate is what stops a test
    // that mounts a page, clicks nothing and asserts nothing from counting as evidence — and a new
    // classification that skipped that check would be a way to launder exactly those.
    let inv = inventory(&format!(
        "{PAGE}\n## Test: mounts and asks nothing\n    Mount Counter.\n"
    ));
    let item = inv["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"].as_str().unwrap_or_default().contains("nothing"))
        .expect("the claim is listed");
    assert_eq!(
        item["vacuous"],
        serde_json::json!(true),
        "asserting nothing is vacuous: {item}"
    );
}
