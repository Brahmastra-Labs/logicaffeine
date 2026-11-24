//! The importer's declaration audit computes transitive assumptions with LEAN's semantics — Lean's
//! `CollectAxioms`, what `#print axioms` prints: a definition or theorem rests on every axiom its type
//! and value reach; an axiom rests on itself, and its own type is NOT visited.
//!
//! One rule, three witnesses that must agree: the audit (`ImportReport::declarations[*].axioms`), the
//! package's rule (the shared closure `axiom_dependencies` over statement and proof, plus a checked
//! opaque's evidence — `certificate.rs`, `RebuiltEnvironment::axioms`), and Lean's own `#print axioms`
//! as recorded per declaration in the corpus ledger (`crates/logicaffeine_tests/tests/fixtures/lean_conformance/bridge/*.json`, `lean_axioms`).

use logicaffeine_kernel::interface::lean::{self as lean, intake, ImportRequest, RetainSource};
use logicaffeine_kernel::{axiom_dependencies, axiom_dependencies_given, axiom_dependencies_of, Term};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const HEADER: &str = r#"{"meta":{"lean":{"version":"4.14.0","githash":"410fab7284703f41660ca2454218dcca9b2ec896"},"format":{"version":"3.1.0"},"exporter":{"version":"3.1.0","name":"lean4export"}}}"#;

fn axioms_of(report: &lean::ImportReport, name: &str) -> Vec<String> {
    report
        .declarations
        .iter()
        .find(|d| d.name == name)
        .unwrap_or_else(|| panic!("{name} is audited"))
        .axioms
        .clone()
}

/// `axiom A : Prop`, `axiom B : A → Prop`, `axiom a : A`, `def d : Prop := B a`.
#[test]
fn an_axioms_type_does_not_charge_the_axioms_it_mentions() {
    let rows = [
        HEADER,
        r#"{"in":1,"str":{"pre":0,"str":"A"}}"#,
        r#"{"in":2,"str":{"pre":0,"str":"B"}}"#,
        r#"{"in":3,"str":{"pre":0,"str":"a"}}"#,
        r#"{"in":4,"str":{"pre":0,"str":"d"}}"#,
        r#"{"ie":0,"sort":0}"#,
        r#"{"axiom":{"name":1,"levelParams":[],"type":0,"isUnsafe":false}}"#,
        r#"{"ie":1,"const":{"name":1,"us":[]}}"#,
        r#"{"ie":2,"forallE":{"name":0,"type":1,"body":0,"binderInfo":"default"}}"#,
        r#"{"axiom":{"name":2,"levelParams":[],"type":2,"isUnsafe":false}}"#,
        r#"{"axiom":{"name":3,"levelParams":[],"type":1,"isUnsafe":false}}"#,
        r#"{"ie":3,"const":{"name":2,"us":[]}}"#,
        r#"{"ie":4,"const":{"name":3,"us":[]}}"#,
        r#"{"ie":5,"app":{"fn":3,"arg":4}}"#,
        r#"{"def":{"name":4,"levelParams":[],"type":0,"value":5,"hints":"abbrev","safety":"safe","all":[4]}}"#,
    ];
    let export = rows.join("\n") + "\n";
    let (_, report) = lean::import_audited(&export).unwrap();
    let k = |n: &str| intake::kernel_name(n).unwrap();
    assert_eq!(axioms_of(&report, &k("A")), vec![k("A")]);
    assert_eq!(
        axioms_of(&report, &k("B")),
        vec![k("B")],
        "B's type mentions A; Lean does not charge it"
    );
    assert_eq!(
        axioms_of(&report, &k("a")),
        vec![k("a")],
        "a's type is A; Lean does not charge it"
    );
    let mut d = axioms_of(&report, &k("d"));
    d.sort();
    let mut expected = vec![k("B"), k("a")];
    expected.sort();
    assert_eq!(
        d, expected,
        "d reaches B and a; A is reached only through their types"
    );
}

/// A walk given the axioms of the constants already audited takes each answer where it meets the constant and walks
/// nothing below it, so auditing a declaration costs its own terms and not its whole closure again. `axiom A : Prop`,
/// `def d1 : Prop := A`, `def d2 : Prop := d1`, `def d3 : Prop := d2`.
#[test]
fn an_audited_constant_answers_for_everything_below_it() {
    let rows = [
        HEADER,
        r#"{"in":1,"str":{"pre":0,"str":"A"}}"#,
        r#"{"in":2,"str":{"pre":0,"str":"d1"}}"#,
        r#"{"in":3,"str":{"pre":0,"str":"d2"}}"#,
        r#"{"in":4,"str":{"pre":0,"str":"d3"}}"#,
        r#"{"ie":0,"sort":0}"#,
        r#"{"axiom":{"name":1,"levelParams":[],"type":0,"isUnsafe":false}}"#,
        r#"{"ie":1,"const":{"name":1,"us":[]}}"#,
        r#"{"def":{"name":2,"levelParams":[],"type":0,"value":1,"hints":"abbrev","safety":"safe","all":[2]}}"#,
        r#"{"ie":2,"const":{"name":2,"us":[]}}"#,
        r#"{"def":{"name":3,"levelParams":[],"type":0,"value":2,"hints":"abbrev","safety":"safe","all":[3]}}"#,
        r#"{"ie":3,"const":{"name":3,"us":[]}}"#,
        r#"{"def":{"name":4,"levelParams":[],"type":0,"value":3,"hints":"abbrev","safety":"safe","all":[4]}}"#,
    ];
    let export = rows.join("\n") + "\n";
    let (ctx, report) = lean::import_audited(&export).unwrap();
    let k = |n: &str| intake::kernel_name(n).unwrap();
    for d in ["d1", "d2", "d3"] {
        assert_eq!(axioms_of(&report, &k(d)), vec![k("A")], "{d} rests on A");
    }
    let asked = std::cell::RefCell::new(Vec::new());
    let known = |name: &str| {
        asked.borrow_mut().push(name.to_string());
        (name == k("d2")).then(|| vec![k("A")])
    };
    let given = axiom_dependencies_given(&ctx, &Term::Global(k("d3")), &known);
    let full = axiom_dependencies(&ctx, &Term::Global(k("d3")));
    assert_eq!(given.axioms, full.axioms, "the answer at d2 is what the walk below it finds");
    assert!(given.unresolved.is_empty() && full.unresolved.is_empty());
    let asked = asked.into_inner();
    assert!(asked.contains(&k("d2")), "the walk reaches d2: {asked:?}");
    assert!(
        !asked.contains(&k("d1")) && !asked.contains(&k("A")),
        "nothing below d2 is walked: {asked:?}"
    );
}

fn corpus() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<PathBuf> =
        std::fs::read_dir(root.join("tests/fixtures/lean_conformance/cases_lean414"))
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "ndjson"))
            .filter(|p| std::fs::metadata(p).unwrap().len() < 1 << 20)
            .collect();
    files.sort();
    files
}

fn on_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

/// ★ The audit and Lean's `#print axioms` agree on every authored declaration of every pinned export
/// that the ledger records.
#[test]
fn the_audit_agrees_with_leans_print_axioms() {
    on_big_stack(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut compared = 0;
        for path in corpus() {
            let case = path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .split('.')
                .next()
                .unwrap()
                .to_string();
            let Ok(ledger) = std::fs::read_to_string(root.join(format!(
                "crates/logicaffeine_tests/tests/fixtures/lean_conformance/bridge/{case}.json"
            ))) else {
                continue;
            };
            let ledger: Value = serde_json::from_str(&ledger).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            let Ok((_, report)) = lean::import_audited(&text) else {
                continue;
            };
            for row in ledger["declarations"].as_array().unwrap() {
                let Some(lean_axioms) = row["lean_axioms"].as_array() else {
                    continue;
                };
                let kernel = intake::kernel_name(row["lean"].as_str().unwrap()).unwrap();
                let Some(entry) = report.declarations.iter().find(|d| d.name == kernel) else {
                    continue;
                };
                let audited: BTreeSet<String> =
                    entry.axioms.iter().map(|a| intake::lean_name(a)).collect();
                let lean: BTreeSet<String> = lean_axioms
                    .iter()
                    .map(|a| a.as_str().unwrap().to_string())
                    .collect();
                assert_eq!(audited, lean, "{}: {}", path.display(), row["lean"]);
                compared += 1;
            }
        }
        assert!(
            compared >= 30,
            "the ledger is exercised: {compared} declarations"
        );
    });
}

/// ★ The audit and the package's rule — the shared closure over statement and proof, plus a checked
/// opaque's evidence — agree on every audited declaration of every pinned export.
#[test]
fn the_audit_agrees_with_the_package_rule() {
    on_big_stack(|| {
        let mut compared = 0;
        for path in corpus() {
            let text = std::fs::read_to_string(&path).unwrap();
            let request = ImportRequest {
                audit: true,
                retain: RetainSource::EveryProof,
                lean_binder_names: false,
            };
            let Ok((ctx, report)) = lean::import_with(&text, request) else {
                continue;
            };
            for entry in &report.declarations {
                let expected: BTreeSet<String> = if entry.kind == "axiom" {
                    [entry.name.clone()].into_iter().collect()
                } else {
                    let ty = ctx
                        .retained(&entry.name)
                        .map(|(ty, _)| ty.clone())
                        .or_else(|| ctx.get_definition_type(&entry.name).cloned())
                        .or_else(|| ctx.get_global(&entry.name).cloned())
                        .or_else(|| ctx.poly_declaration(&entry.name).map(|(_, ty)| ty.clone()))
                        .unwrap_or_else(|| panic!("{}: {} has a type", path.display(), entry.name));
                    let mut usage = axiom_dependencies(&ctx, &ty);
                    if let Some((_, body)) = ctx.retained(&entry.name) {
                        usage.absorb(axiom_dependencies(&ctx, body));
                    }
                    if let Some(evidence) = axiom_dependencies_of(&ctx, &entry.name) {
                        usage.absorb(evidence);
                    }
                    assert!(
                        usage.unresolved.is_empty(),
                        "{}: {}",
                        path.display(),
                        entry.name
                    );
                    usage.axioms.into_iter().collect()
                };
                let audited: BTreeSet<String> = entry.axioms.iter().cloned().collect();
                assert_eq!(audited, expected, "{}: {}", path.display(), entry.name);
                compared += 1;
            }
        }
        assert!(
            compared >= 100,
            "every audited declaration is compared: {compared}"
        );
    });
}

/// Every name an export spells, as the importer keys it.
fn spelled_names(text: &str) -> BTreeSet<String> {
    let mut encoded = std::collections::HashMap::from([(0u64, String::new())]);
    let mut names = BTreeSet::new();
    for line in text.lines().skip(1) {
        let row: Value = serde_json::from_str(line).unwrap();
        let (Some(id), Some(prefix)) = (
            row["in"].as_u64(),
            row.get("str").or_else(|| row.get("num")),
        ) else {
            continue;
        };
        let Some(pre) = encoded.get(&prefix["pre"].as_u64().unwrap()).cloned() else {
            continue;
        };
        let part = match row.get("str") {
            Some(s) => format!(
                "/s:{}",
                serde_json::to_string(s["str"].as_str().unwrap()).unwrap()
            ),
            None => format!("/n:{}", prefix["i"].as_u64().unwrap()),
        };
        let name = format!("{pre}{part}");
        if let Ok(kernel) = intake::kernel_name(&intake::lean_name(&name)) {
            names.insert(kernel);
        }
        encoded.insert(id, name);
    }
    names
}

/// The kernel's two doors onto a name in `ctx`: typing it (a `Global`, or a `Const` at its own level
/// parameters) and reading its axiom closure. Returns how many typed and how many absent names agreed.
fn closure_doors_agree(
    ctx: &logicaffeine_kernel::Context,
    names: &BTreeSet<String>,
    label: &str,
) -> (usize, usize) {
    use logicaffeine_kernel::{infer_type, Term, Universe};
    let (mut typed, mut absent) = (0, 0);
    for name in names {
        let global = infer_type(ctx, &Term::Global(name.clone())).is_ok();
        let constant = ctx.poly_declaration(name).is_some_and(|(levels, _)| {
            infer_type(
                ctx,
                &Term::Const {
                    name: name.clone(),
                    levels: levels.iter().cloned().map(Universe::Var).collect(),
                },
            )
            .is_ok()
        });
        let closure = axiom_dependencies_of(ctx, name);
        if global || constant {
            let closure = closure.unwrap_or_else(|| {
                panic!("{label}: the kernel types {name} but it has no axiom closure")
            });
            assert!(
                closure.unresolved.is_empty(),
                "{label}: {name} rests on unresolved {:?}",
                closure.unresolved
            );
            typed += 1;
        } else if ctx.get_global(name).is_none()
            && ctx.poly_declaration(name).is_none()
            && ctx.get_universe_poly(name).is_none()
            && ctx.get_definition_body(name).is_none()
        {
            assert!(
                closure.is_none(),
                "{label}: nothing registers {name}, yet it has an axiom closure {closure:?}"
            );
            absent += 1;
        }
    }
    (typed, absent)
}

/// ★ Every name the kernel types has an axiom closure with nothing unresolved, and a name nothing
/// registers has none — over the prelude, every pinned corpus export and the nested-function-field
/// witness (whose export spells `LF_NF.CFn.swap` but never declares it).
#[test]
fn every_name_the_kernel_types_has_an_axiom_closure_and_no_other_name_does() {
    on_big_stack(|| {
        let mut prelude = logicaffeine_kernel::Context::new();
        logicaffeine_kernel::prelude::StandardLibrary::register(&mut prelude);
        let mut names: BTreeSet<String> = prelude
            .iter_declarations()
            .map(|(n, _)| n.to_string())
            .chain(prelude.iter_definitions().map(|(n, _, _)| n.to_string()))
            .chain(prelude.iter_inductives().map(|(n, _)| n.to_string()))
            .collect();
        let constructors: Vec<String> = names
            .iter()
            .flat_map(|n| {
                prelude
                    .get_constructors(n)
                    .into_iter()
                    .map(|(c, _)| c.to_string())
            })
            .collect();
        names.extend(constructors);
        let (mut typed, mut absent) = closure_doors_agree(&prelude, &names, "prelude");
        let mut exports: Vec<(String, String)> = corpus()
            .into_iter()
            .map(|p| {
                (
                    p.display().to_string(),
                    std::fs::read_to_string(&p).unwrap(),
                )
            })
            .collect();
        exports.push((
            "LF_NestedFunctionField".into(),
            include_str!("fixtures/lean_semantics/LF_NestedFunctionField.ndjson").to_string(),
        ));
        for (label, text) in exports {
            let Ok((ctx, report)) = lean::import_audited(&text) else {
                continue;
            };
            let mut names = spelled_names(&text);
            names.extend(report.internal_helpers.iter().map(|h| h.name.clone()));
            let (t, a) = closure_doors_agree(&ctx, &names, &label);
            typed += t;
            absent += a;
            if label == "LF_NestedFunctionField" {
                let swap = intake::kernel_name("LF_NF.CFn.swap").unwrap();
                assert!(
                    names.contains(&swap) && axiom_dependencies_of(&ctx, &swap).is_none(),
                    "the witness spells CFn.swap without declaring it"
                );
            }
        }
        assert!(typed >= 1000, "the kernel's names are exercised: {typed}");
        assert!(absent >= 1, "an absent name is exercised: {absent}");
    });
}
