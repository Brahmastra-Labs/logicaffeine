//! G13-full — the machine-readable proof/test INVENTORY lock.
//!
//! One structured document classifies EVERY claim a project makes by KIND
//! (example / property / theorem / axiom / admitted-assumption) with an honest
//! per-item status. This lock pins the schema and the honesty gates end to end
//! through the real `largo` binary:
//!
//! - `largo test --json` classifies examples and finite-enumeration properties
//!   (proved-by-exhaustion, falsified-with-counterexample, or held-by-sampling),
//!   flags a vacuous example, and DISCLOSES every property's finite domain bound.
//! - `largo prove --json` classifies theorems (proved / unproved / bounded) and
//!   reports axioms as ASSUMPTIONS — never as proven facts — and REJECTS a
//!   theorem/axiom name collision.
//! - The counts are internally consistent (examples+properties+theorems == the
//!   total claims; proven ⊆ properties+theorems).

#[path = "cli_common/mod.rs"]
mod common;

use common::*;
use serde_json::Value;
use tempfile::tempdir;

fn write_file(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

/// A project whose closure holds one asserting example, one VACUOUS example,
/// one finite property that HOLDS, and one finite property that is FALSIFIED.
const TEST_SOURCE: &str = "\
## Test: addition holds
Let x be 2.
Let y be 3.
Expect that x plus y is 5.

## Test: checks nothing at all
Let x be 1.

## Test: doubling is even
For all naturals n:
    Expect that n plus n is divisible by 2.

## Test: everything is positive
For all integers n:
    Expect that n is greater than 0.
";

/// Grab the inventory document from `largo test --json`.
fn test_inventory(dir: &std::path::Path, src: &str) -> Value {
    scaffold(dir, "inv_test");
    std::fs::write(dir.join("src/main.lg"), src).unwrap();
    let out = largo_in(dir, &["test", "--json"]);
    let doc: Value = serde_json::from_str(&stdout(&out))
        .unwrap_or_else(|e| panic!("test --json must be valid JSON ({e}):\n{}", stdout(&out)));
    doc
}

/// Every item in the inventory of `doc` whose kind == `kind`.
fn items_of_kind<'a>(doc: &'a Value, kind: &str) -> Vec<&'a Value> {
    doc["inventory"]
        .as_array()
        .expect("inventory is an array")
        .iter()
        .filter(|i| i["kind"] == kind)
        .collect()
}

fn item_named<'a>(doc: &'a Value, name_substr: &str) -> &'a Value {
    doc["inventory"]
        .as_array()
        .expect("inventory is an array")
        .iter()
        .find(|i| i["name"].as_str().is_some_and(|n| n.contains(name_substr)))
        .unwrap_or_else(|| panic!("no inventory item named ~{name_substr:?}: {doc}"))
}

/// The asserting example is classified `example` with status `passed`, and is
/// NOT flagged vacuous.
#[test]
fn asserting_example_is_a_passing_example() {
    let dir = tempdir().unwrap();
    let doc = test_inventory(dir.path(), TEST_SOURCE);
    let item = item_named(&doc, "addition holds");
    assert_eq!(
        item["kind"], "example",
        "an Expect-block is an example: {item}"
    );
    assert_eq!(item["status"], "passed", "the assertion holds: {item}");
    assert_eq!(item["vacuous"], false, "it asserted something: {item}");
    // An example is finite testing, never an unrestricted proof.
    assert_eq!(item["proof"], false, "an example is never a proof: {item}");
}

/// The assertion-free example is FLAGGED vacuous in the inventory (surfaced,
/// not just failed) — a green that checked nothing must be legible as such.
#[test]
fn vacuous_example_is_flagged() {
    let dir = tempdir().unwrap();
    let doc = test_inventory(dir.path(), TEST_SOURCE);
    let item = item_named(&doc, "checks nothing");
    assert_eq!(item["kind"], "example");
    assert_eq!(item["vacuous"], true, "no assertion → vacuous: {item}");
    assert_eq!(
        item["status"], "vacuous",
        "vacuous is its own status: {item}"
    );
}

/// The holding property is `property`, marked `finite_enumeration`, its FINITE
/// domain bound is disclosed, and it is NOT reported as an unrestricted proof.
#[test]
fn holding_property_discloses_its_finite_bound() {
    let dir = tempdir().unwrap();
    let doc = test_inventory(dir.path(), TEST_SOURCE);
    let item = item_named(&doc, "doubling is even");
    assert_eq!(item["kind"], "property", "a For-all is a property: {item}");
    assert_eq!(
        item["finite_enumeration"], true,
        "checked by finite enumeration: {item}"
    );
    assert_eq!(
        item["proof"], false,
        "finite enumeration is NOT an unrestricted proof: {item}"
    );
    let bound = item["domain_bound"].as_str().expect("a disclosed bound");
    assert!(
        !bound.is_empty(),
        "the finite bound must be disclosed: {item}"
    );
    // The natural-domain bound must name the actual sampled range.
    assert!(
        bound.contains("natural") || bound.contains("0") || bound.contains("1000"),
        "bound names the naturals range: {item}"
    );
    assert!(
        item["status"] == "held" || item["status"] == "finite_enumeration",
        "a sampled property that held is `held`, not `proved`: {item}"
    );
    assert!(
        item["counterexample"].is_null(),
        "no counterexample when it holds: {item}"
    );
}

/// The falsified property is FALSIFIED and carries the CONCRETE counterexample.
#[test]
fn falsified_property_carries_its_counterexample() {
    let dir = tempdir().unwrap();
    let doc = test_inventory(dir.path(), TEST_SOURCE);
    let item = item_named(&doc, "everything is positive");
    assert_eq!(item["kind"], "property");
    assert_eq!(
        item["status"], "falsified",
        "a property with a counterexample is falsified: {item}"
    );
    let cx = item["counterexample"]
        .as_str()
        .expect("a concrete counterexample");
    assert!(
        cx.contains("n ="),
        "the counterexample binds the domain variable: {item}"
    );
}

/// A property proved by EXHAUSTION (`--prove` over booleans) is reported as
/// proved-by-exhaustion AND still marked finite_enumeration with its bound — a
/// certificate over a finite domain, never an unrestricted mathematical proof.
#[test]
fn exhausted_boolean_property_is_proved_by_exhaustion_but_bounded() {
    let dir = tempdir().unwrap();
    scaffold(dir.path(), "inv_prove");
    let src = "\
## Test: a bool equals itself
For all booleans b:
    Expect that b is b.
";
    std::fs::write(dir.path().join("src/main.lg"), src).unwrap();
    let out = largo_in(dir.path(), &["test", "--prove", "--json"]);
    let doc: Value = serde_json::from_str(&stdout(&out)).expect("valid JSON");
    let item = item_named(&doc, "bool equals itself");
    assert_eq!(item["kind"], "property");
    assert_eq!(
        item["status"], "proved_by_exhaustion",
        "boolean exhaustion is a certificate: {item}"
    );
    assert_eq!(
        item["finite_enumeration"], true,
        "still finite — not unrestricted: {item}"
    );
    assert_eq!(
        item["proof"], false,
        "exhaustion over booleans is NOT an unrestricted math proof: {item}"
    );
    assert!(
        item["domain_bound"].as_str().is_some_and(|b| !b.is_empty()),
        "the exhausted domain is disclosed: {item}"
    );
}

/// The counts are internally consistent: examples+properties+theorems == total,
/// and every property/example appears in the inventory.
#[test]
fn inventory_counts_are_internally_consistent() {
    let dir = tempdir().unwrap();
    let doc = test_inventory(dir.path(), TEST_SOURCE);
    let inv = doc["inventory"].as_array().unwrap();
    let examples = items_of_kind(&doc, "example").len();
    let properties = items_of_kind(&doc, "property").len();
    let theorems = items_of_kind(&doc, "theorem").len();
    let axioms = items_of_kind(&doc, "axiom").len();
    let admitted = items_of_kind(&doc, "admitted_assumption").len();
    assert_eq!(examples, 2, "two examples (one asserting, one vacuous)");
    assert_eq!(properties, 2, "two For-all properties");
    assert_eq!(theorems, 0, "no theorems in a pure test file");
    assert_eq!(
        examples + properties + theorems + axioms + admitted,
        inv.len(),
        "every item is classified into exactly one kind"
    );
    // The summary echoes the same numbers.
    let summary = &doc["summary"];
    assert_eq!(summary["examples"], examples as u64);
    assert_eq!(summary["properties"], properties as u64);
    assert_eq!(
        summary["claims"],
        inv.len() as u64,
        "claims == inventory length"
    );
}

// ============================================================
// `largo prove --json` — theorems, axioms as assumptions, name collision.
// ============================================================

const THEOREM_SOURCE: &str = "\
## Theorem: Socrates
Given: Socrates is a man.
Given: Every man is mortal.
Prove: Socrates is mortal.
Proof: Auto.
";

fn prove_inventory(dir: &std::path::Path, src: &str) -> (std::process::Output, Value) {
    scaffold(dir, "inv_prove2");
    std::fs::write(dir.join("src/main.lg"), src).unwrap();
    let out = largo_in(dir, &["prove", "--json"]);
    let doc: Value = serde_json::from_str(&stdout(&out)).unwrap_or_else(|e| {
        panic!(
            "prove --json must be valid JSON ({e}):\nstdout={}\nstderr={}",
            stdout(&out),
            stderr(&out)
        )
    });
    (out, doc)
}

/// A proved English theorem is `theorem` / `proved`, carries its name, and is
/// reported as an actual proof (kernel-certified).
#[test]
fn proved_theorem_is_classified_as_a_proof() {
    let dir = tempdir().unwrap();
    let (_out, doc) = prove_inventory(dir.path(), THEOREM_SOURCE);
    let item = item_named(&doc, "Socrates");
    assert_eq!(
        item["kind"], "theorem",
        "a Theorem block is a theorem: {item}"
    );
    assert_eq!(item["status"], "proved", "Socrates is proved: {item}");
    assert_eq!(
        item["proof"], true,
        "a kernel-certified theorem IS a proof: {item}"
    );
}

/// An axiom is reported as an ASSUMPTION, explicitly NOT a proof — the key
/// honesty line. An axiom must never be counted among proven facts.
#[test]
fn axiom_is_reported_as_an_assumption_never_a_proof() {
    let dir = tempdir().unwrap();
    let src = "\
## Axiom flip: for all a b, Cong(a, b, b, a).

## Theory Geo

Theorem reflexivity: prove for all a b, Cong(a, b, a, b).
";
    let (_out, doc) = prove_inventory(dir.path(), src);
    let axioms = items_of_kind(&doc, "axiom");
    assert_eq!(axioms.len(), 1, "the one ## Axiom is inventoried: {doc}");
    let axiom = axioms[0];
    assert_eq!(axiom["name"], "flip", "the axiom carries its name: {axiom}");
    assert_eq!(
        axiom["status"], "assumption",
        "an axiom is an assumption: {axiom}"
    );
    assert_eq!(axiom["proof"], false, "an axiom is NEVER a proof: {axiom}");
    // And it must not be double-counted as a theorem.
    assert!(
        !items_of_kind(&doc, "theorem")
            .iter()
            .any(|t| t["name"] == "flip"),
        "the axiom must never masquerade as a theorem: {doc}"
    );
}

/// A theorem and an axiom sharing a NAME is an error — an axiom must never be
/// able to masquerade as a proved theorem. The run fails loudly.
#[test]
fn theorem_axiom_name_collision_is_rejected() {
    let dir = tempdir().unwrap();
    // `flip` is both an admitted axiom AND a theorem name.
    let src = "\
## Axiom flip: for all a b, Cong(a, b, b, a).

## Theory Geo

Theorem flip: prove for all a b, Cong(a, b, a, b).
";
    scaffold(dir.path(), "collide");
    std::fs::write(dir.path().join("src/main.lg"), src).unwrap();
    let out = largo_in(dir.path(), &["prove", "--json"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "a name collision must fail the run"
    );
    let text = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        text.to_lowercase().contains("collision")
            || text.to_lowercase().contains("both a theorem and an axiom")
            || (text.contains("flip") && text.to_lowercase().contains("axiom")),
        "the collision must be explained, naming `flip`:\n{text}"
    );
}
