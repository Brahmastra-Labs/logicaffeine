//! Focused pipeline controls for refusal explanations. Synthetic exports exercise graph semantics;
//! they make no claim that a Lean compiler was run. Original-root refusals supply that evidence.
mod lean_package_support;

use lean_package_support::Export;
use logicaffeine_kernel::interface::lean::pipeline::{self, Checked, LeanEvidence, Limits};
use logicaffeine_kernel::interface::lean::receipt::StageStatus;
use logicaffeine_kernel::interface::lean::{DiagnosticCode, Outcome};
use std::collections::BTreeSet;

struct Graph {
    export: Export,
    namespace: u64,
    proposition: u64,
    leaf: u64,
    combine: u64,
}

impl Graph {
    fn new() -> Self {
        let mut export = Export::new();
        export.lines[0] = include_str!("fixtures/lean_package/W05.clean.ndjson")
            .lines()
            .next()
            .unwrap()
            .to_string();
        let namespace = export.name("Taint", 0);
        let p = export.name("P", namespace);
        let leaf_name = export.name("ALeaf", namespace);
        let combine_name = export.name("combine", namespace);
        let x = export.name("x", 0);
        let y = export.name("y", 0);
        let prop = export.sort(0);
        export.axiom(p, prop, &[]);
        let proposition = export.constant(p, &[]);
        export.axiom(leaf_name, proposition, &[]);
        let leaf = export.constant(leaf_name, &[]);
        let last = export.forall(y, proposition, proposition);
        let combine_type = export.forall(x, proposition, last);
        export.axiom(combine_name, combine_type, &[]);
        let combine = export.constant(combine_name, &[]);
        Self {
            export,
            namespace,
            proposition,
            leaf,
            combine,
        }
    }

    fn definition(&mut self, short: &str, body: u64) -> u64 {
        let name = self.export.name(short, self.namespace);
        self.export.def(name, self.proposition, body);
        self.export.constant(name, &[])
    }

    fn pair(&mut self, first: u64, second: u64) -> u64 {
        let applied = self.export.app(self.combine, first);
        self.export.app(applied, second)
    }

    fn root(&mut self, body: u64) {
        let root = self.export.name("root", self.namespace);
        self.export.thm(root, self.proposition, body);
    }

    fn checked(&self) -> Checked {
        pipeline::check_export(
            &self.export.text(),
            "Taint.root",
            &LeanEvidence::NotObserved,
            &Limits::default(),
        )
    }
}

fn assert_refusal(checked: &Checked, paths: &[&[&str]]) {
    let result = &checked.result;
    let describe = || serde_json::to_string_pretty(&result.to_json()).unwrap();
    assert_eq!(
        result.outcome(),
        Outcome::DisallowedAssumptions,
        "{}",
        describe()
    );
    assert_eq!(result.exit_code(), 3);
    assert!(checked.package.is_none() && result.package.is_none());
    assert!(result.target_logos.is_none());
    for (stage, expected) in [
        ("lean_accepted", StageStatus::NotRun),
        ("translated", StageStatus::Passed),
        ("primary_check", StageStatus::Passed),
        ("independent_recheck", StageStatus::Passed),
        ("assumption_policy", StageStatus::Failed),
        ("emission_reparsed", StageStatus::NotRun),
        ("fresh_replay", StageStatus::NotRun),
    ] {
        assert_eq!(result.status(stage), expected, "{stage}: {}", describe());
    }
    let expected_axioms: BTreeSet<_> = paths
        .iter()
        .map(|path| path.last().unwrap().to_string())
        .collect();
    assert_eq!(
        result.assumptions.iter().cloned().collect::<BTreeSet<_>>(),
        expected_axioms
    );
    assert_eq!(result.assumptions.len(), paths.len());
    assert_eq!(
        result.diagnostics.first().map(|d| d.code),
        Some(DiagnosticCode::DisallowedAssumption)
    );
    assert!(
        !result.diagnostics[0].message.starts_with("taint path to "),
        "the original policy diagnostic stays first"
    );
    let expected: BTreeSet<String> = paths
        .iter()
        .map(|path| {
            format!(
                "taint path to {}: {}",
                path.last().unwrap(),
                path.join(" → ")
            )
        })
        .collect();
    let actual: Vec<&str> = result
        .diagnostics
        .iter()
        .filter(|d| d.message.starts_with("taint path to "))
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(
        actual.len(),
        expected.len(),
        "one explanation per assumption: {}",
        describe()
    );
    assert_eq!(
        actual
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>(),
        expected,
        "{}",
        describe()
    );
    for diagnostic in &result.diagnostics {
        assert_eq!(diagnostic.stage, "assumption_policy");
        assert_eq!(diagnostic.code, DiagnosticCode::DisallowedAssumption);
        assert_eq!(diagnostic.declaration.as_deref(), Some("Taint.root"));
    }
}

#[test]
fn an_axiom_is_a_leaf_even_when_its_type_mentions_another_assumption() {
    let mut graph = Graph::new();
    let alias = graph.export.name("ZAlias", graph.namespace);
    let prop = graph.export.sort(0);
    graph.export.def(alias, prop, graph.proposition);
    let alias_type = graph.export.constant(alias, &[]);
    let root = graph.export.name("root", graph.namespace);
    graph.export.thm(root, alias_type, graph.leaf);
    // ALeaf's type mentions P, but CollectAxioms stops at ALeaf. P must be reached through ZAlias.
    assert_refusal(
        &graph.checked(),
        &[
            &["Taint.root", "Taint.ALeaf"],
            &["Taint.root", "Taint.ZAlias", "Taint.P"],
        ],
    );
}

#[test]
fn equal_length_paths_use_lean_name_order_not_record_or_application_order() {
    let mut graph = Graph::new();
    let z = graph.definition("ZBranch", graph.leaf);
    let a = graph.definition("ABranch", graph.leaf);
    let body = graph.pair(z, a);
    graph.root(body);
    assert_refusal(
        &graph.checked(),
        &[
            &["Taint.root", "Taint.ABranch", "Taint.ALeaf"],
            &["Taint.root", "Taint.P"],
            &["Taint.root", "Taint.combine"],
        ],
    );
}

#[test]
fn the_shortest_path_wins_over_an_alphabetically_earlier_longer_path() {
    let mut graph = Graph::new();
    let middle = graph.definition("Middle", graph.leaf);
    let a = graph.definition("ABranch", middle);
    let z = graph.definition("ZBranch", graph.leaf);
    let body = graph.pair(a, z);
    graph.root(body);
    assert_refusal(
        &graph.checked(),
        &[
            &["Taint.root", "Taint.ZBranch", "Taint.ALeaf"],
            &["Taint.root", "Taint.P"],
            &["Taint.root", "Taint.combine"],
        ],
    );
}

#[test]
fn an_opaque_dependency_uses_its_retained_checked_body() {
    let mut graph = Graph::new();
    let opaque = graph.export.name("sealed", graph.namespace);
    graph.export.raw(format!(
        r#"{{"opaque":{{"name":{opaque},"levelParams":[],"type":{},"value":{},"isUnsafe":false,"all":[{opaque}]}}}}"#,
        graph.proposition, graph.leaf
    ));
    let body = graph.export.constant(opaque, &[]);
    graph.root(body);
    assert_refusal(
        &graph.checked(),
        &[
            &["Taint.root", "Taint.sealed", "Taint.ALeaf"],
            &["Taint.root", "Taint.P"],
        ],
    );
}

#[test]
fn sharing_helpers_are_traversed_without_appearing_as_lean_declarations() {
    let mut graph = Graph::new();
    let mut repeated = graph.leaf;
    // Seventeen doublings: 2^17 leaves as a tree, seventeen `combine` nodes as a DAG. The tree
    // outgrows the importer's per-declaration budget (100,000), which is what makes it share.
    for _ in 0..17 {
        repeated = graph.pair(repeated, repeated);
    }
    graph.root(repeated);
    let checked = graph.checked();
    let audit = checked
        .result
        .audit
        .as_ref()
        .expect("the import produced its declaration audit");
    let helpers = audit["internal_helpers"]
        .as_array()
        .expect("reported sharing helpers");
    assert!(
        !helpers.is_empty(),
        "this control must actually exercise importer sharing"
    );
    assert_refusal(
        &checked,
        &[
            &["Taint.root", "Taint.ALeaf"],
            &["Taint.root", "Taint.P"],
            &["Taint.root", "Taint.combine"],
        ],
    );
    for helper in helpers {
        let name = helper["name"].as_str().unwrap();
        assert!(checked
            .result
            .diagnostics
            .iter()
            .all(|d| !d.message.contains(name)));
    }
}
