//! A projection's family is read from the weak-head normal form of its value's type.
//!
//! Lean's kernel types `Expr.proj S i e` by `whnf (infer_type e)`: it unfolds the type's head until
//! the structure `S` appears and never normalizes the rest of the type. The reader fully normalized
//! that type instead. `Nat.log2_terminates` projects out of `Nat.below (motive := …) (n + 4)`, whose
//! head reaches `PProd` only through `Nat.rec`, and whose full normal form does not finish: every
//! root depending on `Nat.log2` (blocker `B-FALSE-REJECT-3f6719b2`) stalled in `body.parse` of
//! `Nat.log2_terminates` for more than 600 s. The reader now exposes the head exactly as the
//! kernel's own `match` typing does (the type itself, else its bounded head reduction) and keeps
//! the full normal form only as the fallback for a head the bounded probe cannot reach. The
//! lowered `match` is kernel-checked as before.
//!
//! Fixture: `fixtures/lean_scale/LG_Proj.lean`, exported with the pinned Lean 4.14.0 and the
//! pinned lean4export (structural expression keys). `firstOfAlias` and `firstOfDeep` elaborate
//! to `Prod.fst p`; the controls below rewrite that one application into Lean's `Expr.proj` on the
//! same variable, so its type (`PairAlias`, `Deep 300`) only unfolds to `Prod`. A projection out
//! of a family that is not a structure keeps its Lean-original refusals in `lean_arena_projection`
//! (`checks_088_projnotstruct`, `checks_proj_non_structure`).
#![allow(dead_code)]

#[path = "harvest_support/checker.rs"]
mod checker;
#[path = "lean_scale_support/witness.rs"]
mod witness;

use checker::{check_in, Root};
use logicaffeine_kernel::interface::lean::reference::Reference;
use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode, Verdict};

const STEM: &str = "LG_Proj";
const ROOTS: [&str; 3] = [
    "LG_PJ.firstOfAlias",
    "LG_PJ.firstOfDeep",
    "LG_PJ.usesLog2Terminates",
];

fn export() -> String {
    witness::witness_with_axioms(STEM, &ROOTS, &[&[], &[], &["propext"]]).0
}

#[test]
fn leans_own_log2_terminates_imports_and_both_kernels_check_its_statement() {
    let export = export();
    let reference = Reference::read(&export).expect("the reference translator reads the export");
    let (ctx, report) =
        witness::imported(&export, &["LG_PJ.usesLog2Terminates"]).unwrap_or_else(|e| {
            panic!("the production reader must accept Lean's Nat.log2_terminates: {e}")
        });
    let mut failures = Vec::new();
    for (lean, kind, axioms) in [
        ("LG_PJ.firstOfAlias", "def", &[][..]),
        ("LG_PJ.firstOfDeep", "def", &[][..]),
        ("LG_PJ.usesLog2Terminates", "theorem", &["propext"][..]),
    ] {
        if let Err(why) = check_in(
            &export,
            &ctx,
            &report,
            &reference,
            &Root { lean, kind, axioms },
        ) {
            failures.push(format!("{lean}: {why}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The export's name index of a Lean name.
fn name_index(export: &str, lean: &str) -> u64 {
    let mut names = std::collections::HashMap::from([(0u64, String::new())]);
    for line in export.lines().skip(1) {
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        if let (Some(i), Some(part)) = (
            row["in"].as_u64(),
            row.get("str").or_else(|| row.get("num")),
        ) {
            let pre = &names[&part["pre"].as_u64().unwrap()];
            let component = part["str"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| part["i"].to_string());
            names.insert(
                i,
                if pre.is_empty() {
                    component
                } else {
                    format!("{pre}.{component}")
                },
            );
        }
    }
    *names
        .iter()
        .find(|(_, n)| n.as_str() == lean)
        .unwrap_or_else(|| panic!("no name {lean}"))
        .0
}

/// The witness with `@Prod.fst Nat Nat p` — the one body both `firstOfAlias` and `firstOfDeep`
/// share — replaced by the projection `p.<idx>` read as a field of `family`, and `family`'s name
/// records moved to the front of the export, so the projection names a family the reader already
/// knows and the only thing wrong with the export is the projection itself.
fn with_projection(family: &str, idx: u64) -> String {
    let export = with_projection_in_place(family, idx);
    let family = name_index(&export, family);
    let mut records: Vec<&str> = export.lines().collect();
    let parse = |line: &str| serde_json::from_str::<serde_json::Value>(line).unwrap();
    let mut chain = Vec::new();
    let mut next = family;
    while next != 0 {
        let at = records
            .iter()
            .position(|l| parse(l)["in"].as_u64() == Some(next))
            .expect("the name is defined");
        let record = records.remove(at);
        let row = parse(record);
        next = row
            .get("str")
            .or_else(|| row.get("num"))
            .and_then(|part| part["pre"].as_u64())
            .expect("a name part");
        chain.push(record);
    }
    for record in chain {
        records.insert(1, record);
    }
    records.join("\n") + "\n"
}

/// [`with_projection`] leaving `family`'s name records where the exporter wrote them.
fn with_projection_in_place(family: &str, idx: u64) -> String {
    let export = export();
    let fst = name_index(&export, "Prod.fst");
    let rows: Vec<serde_json::Value> = export
        .lines()
        .skip(1)
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let by_id = |id: u64| rows.iter().find(|r| r["ie"].as_u64() == Some(id)).unwrap();
    let head = |mut id: u64| {
        while let Some(f) = by_id(id)["app"]["fn"].as_u64() {
            id = f;
        }
        by_id(id)["const"]["name"].as_u64()
    };
    let applications: Vec<&serde_json::Value> = rows
        .iter()
        .filter(|r| {
            r.get("app").is_some()
                && head(r["ie"].as_u64().unwrap()) == Some(fst)
                && by_id(r["app"]["arg"].as_u64().unwrap())
                    .get("bvar")
                    .is_some()
        })
        .collect();
    assert_eq!(
        applications.len(),
        1,
        "one shared `@Prod.fst Nat Nat p` application"
    );
    let target = applications[0];
    let (id, variable) = (
        target["ie"].as_u64().unwrap(),
        target["app"]["arg"].as_u64().unwrap(),
    );
    let original = serde_json::to_string(target).unwrap();
    let line = export
        .lines()
        .find(|l| serde_json::from_str::<serde_json::Value>(l).unwrap() == *target)
        .unwrap();
    assert_eq!(export.matches(line).count(), 1, "{original}");
    let family = name_index(&export, family);
    export.replacen(
        line,
        &format!(r#"{{"ie":{id},"proj":{{"typeName":{family},"idx":{idx},"struct":{variable}}}}}"#),
        1,
    )
}

#[test]
fn a_projection_out_of_a_type_that_only_unfolds_to_its_structure_is_accepted() {
    // `PairAlias` unfolds to `Prod Nat Nat` in one δ-step; `Deep 300` needs three hundred levels
    // of its own recursion, past the bounded head probe, so it also exercises the full-normal-form
    // fallback. Both are what Lean's `whnf` reaches.
    let input = with_projection("Prod", 0);
    let (ctx, _) = adapter::import_audited(&input)
        .unwrap_or_else(|e| panic!("p.1 on an alias of Prod: {e:?}"));
    for root in ["firstOfAlias", "firstOfDeep"] {
        let name = witness::kernel_spelling(&format!("LG_PJ.{root}"));
        let body = ctx
            .get_definition_body(&name)
            .unwrap_or_else(|| panic!("{root} has a checked body"));
        assert!(
            format!("{body}").contains("match"),
            "{root}: the projection is lowered to a checked match: {body}"
        );
        logicaffeine_kernel::recheck(&ctx, body)
            .unwrap_or_else(|e| panic!("{root}: independent kernel: {e:?}"));
    }
}

fn rejected_as_invalid(input: &str, why: &str) {
    match adapter::check_str(input) {
        Err(verdict @ Verdict::Reject(_)) => {
            assert_eq!(
                verdict.reason().code(),
                DiagnosticCode::InvalidProof,
                "{verdict:?}"
            );
            assert!(
                format!("{verdict:?}").contains(why),
                "refused for the right reason ({why}): {verdict:?}"
            );
        }
        other => panic!("expected an invalid-proof rejection ({why}), got {other:?}"),
    }
}

#[test]
fn a_projection_naming_another_parent_is_refused() {
    rejected_as_invalid(&with_projection("PProd", 0), "projection parent mismatch");
}

/// A record naming a name the export defines only later is not a well-formed export: the reader
/// resolves names as it meets them.
#[test]
fn a_projection_naming_a_family_defined_later_is_declined_as_a_malformed_export() {
    match adapter::check_str(&with_projection_in_place("PProd", 0)) {
        Err(verdict @ Verdict::Decline(_)) => {
            assert_eq!(
                verdict.reason().code(),
                DiagnosticCode::MalformedExport,
                "{verdict:?}"
            );
            assert!(
                format!("{verdict:?}").contains("unknown name index"),
                "{verdict:?}"
            );
        }
        other => panic!("expected a malformed-export decline, got {other:?}"),
    }
}

#[test]
fn a_projection_out_of_range_is_refused() {
    rejected_as_invalid(&with_projection("Prod", 2), "projection index out of range");
}
