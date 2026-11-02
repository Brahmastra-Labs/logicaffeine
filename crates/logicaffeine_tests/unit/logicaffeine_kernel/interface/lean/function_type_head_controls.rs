//! Application typing reads only the HEAD of the function's type, as Lean's `infer_app` does
//! (`ensure_pi (whnf (infer f))`). Normalizing the whole type unfolds every proof embedded in it —
//! each `Fin` numeral carries its instance's `Nat.mod_lt` proof, and theorems are transparent — and
//! that is what turned Arena `perf/magma-list-pair-n21` from an acceptance into a timeout.
//!
//! Original witness `fixtures/lean_semantics/LF_FunctionTypeHead.lean` (Lean 4.14.0, pinned
//! lean4export): three theorems Lean's kernel accepts, each applying a function whose type is an
//! `abbrev`, a `def` or a `¬`, and two corruptions Lean's kernel rejects (recorded in each
//! provenance): the same application claimed at `False` (`idemFalse`), and at the false equation
//! `op 2 2 = op 2 3` (`idemWrong`).
use super::*;
use std::sync::atomic::Ordering;

const SOURCE_SHA: &str = "84ab3f0d64a8b15f901a12593028fc20ef9c57fe4d4fcbad69650762888cf85d";
const POSITIVE: &str =
    include_str!("../../../../tests/fixtures/lean_semantics/LF_FunctionTypeHead.positive.ndjson");
const POSITIVE_PROVENANCE: &str = include_str!(
    "../../../../tests/fixtures/lean_semantics/LF_FunctionTypeHead.positive.provenance.json"
);
const WRONG: &str =
    include_str!("../../../../tests/fixtures/lean_semantics/LF_FunctionTypeHead.wrong.ndjson");
const WRONG_PROVENANCE: &str = include_str!(
    "../../../../tests/fixtures/lean_semantics/LF_FunctionTypeHead.wrong.provenance.json"
);
const FALSE: &str =
    include_str!("../../../../tests/fixtures/lean_semantics/LF_FunctionTypeHead.false.ndjson");
const FALSE_PROVENANCE: &str = include_str!(
    "../../../../tests/fixtures/lean_semantics/LF_FunctionTypeHead.false.provenance.json"
);

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}

fn pinned(provenance: &str, export_sha: &str, input: &str, roots: &[&str]) -> Value {
    let p: Value = serde_json::from_str(provenance).unwrap();
    assert_eq!(
        p["source"],
        "crates/logicaffeine_tests/tests/fixtures/lean_semantics/LF_FunctionTypeHead.lean"
    );
    assert_eq!(p["source_sha256"], SOURCE_SHA);
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(
        (p["compiler_exit"].as_i64(), p["exporter_exit"].as_i64()),
        (Some(0), Some(0))
    );
    assert_eq!(p["export_sha256"], export_sha);
    assert_eq!(p["records"], input.lines().count());
    let declared: Vec<&str> = p["declarations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap())
        .collect();
    assert_eq!(declared, roots);
    p
}

#[test]
fn an_applied_function_type_is_exposed_at_its_head_and_never_normalized_in_full() {
    let roots = ["LF_FT.useAt", "LF_FT.idemAt", "LF_FT.notAt"];
    let p = pinned(
        POSITIVE_PROVENANCE,
        "9820276beef5d5be8c8f6684992e48ded8833b7b6774166d27650ba1db5eefd9",
        POSITIVE,
        &roots,
    );
    let before = FUNCTION_TYPE_NORMALIZATIONS.load(Ordering::Relaxed);
    let (ctx, report) = import_audited(POSITIVE).expect("Lean's kernel accepts every declaration");
    let normalized = FUNCTION_TYPE_NORMALIZATIONS.load(Ordering::Relaxed) - before;
    assert_eq!(
        normalized, 0,
        "every applied function's Π is reached by head reduction; none is normalized in full"
    );
    let axioms: Vec<&str> = p["lean_print_axioms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    for root in roots {
        assert!(
            axioms.contains(&format!("'{root}' depends on axioms: [propext]").as_str()),
            "{axioms:?}"
        );
        let audit = report
            .declarations
            .iter()
            .find(|d| d.name == n(root))
            .unwrap_or_else(|| panic!("{root}"));
        assert_eq!(audit.kind, "thm", "{root}");
        assert_eq!(
            audit.axioms,
            vec![n("propext")],
            "{root}: Lean's #print axioms says [propext]"
        );
        // Both kernels check the published proof, and each infers exactly the declared statement.
        let statement = ctx.get_global(&n(root)).unwrap().clone();
        let proof = ctx
            .get_definition_body(&n(root))
            .expect("theorems keep their proof")
            .clone();
        let main = crate::infer_type(&ctx, &proof)
            .unwrap_or_else(|e| panic!("{root}: primary kernel: {e:?}"));
        let independent = crate::recheck(&ctx, &proof)
            .unwrap_or_else(|e| panic!("{root}: independent kernel: {e:?}"));
        for inferred in [&main, &independent] {
            assert!(
                crate::is_subtype(&ctx, inferred, &statement)
                    && crate::is_subtype(&ctx, &statement, inferred),
                "{root}"
            );
        }
    }
}

fn lean_rejected(p: &Value, name: &str) {
    let verdicts: Vec<&str> = p["lean_kernel_verdicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    assert!(
        verdicts.iter().any(|v| v.starts_with(&format!(
            "{name}: kernel REJECTED: (kernel) declaration type mismatch"
        ))),
        "Lean 4.14.0's kernel refused {name}: {verdicts:?}"
    );
    assert!(
        !verdicts.iter().any(|v| v.contains("kernel ACCEPTED")),
        "{verdicts:?}"
    );
}

fn refused(input: &str, what: &str) {
    let verdict = check_str(input).expect_err(what);
    assert!(matches!(verdict, Verdict::Reject(_)), "{what}: {verdict:?}");
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::InvalidProof,
        "{what}: {verdict:?}"
    );
}

#[test]
fn the_same_application_at_a_false_result_is_refused() {
    let p = pinned(
        FALSE_PROVENANCE,
        "a09f3acbe097f9851a0810df47511d326f88d189d87f1c153c694900e44b622b",
        FALSE,
        &["LF_FT.idemFalse"],
    );
    lean_rejected(&p, "LF_FT.idemFalse");
    refused(FALSE, "h 2 proves an equation, not False");
}

/// The refutation Lean's kernel makes at once — `op 2 2` and `op 2 3` are `Fin.mk` of the distinct
/// numerals 4 and 0 — and the one the primary kernel's conversion does not yet finish (it spends
/// its time in `lazy_delta_conversion`; reported to lane LG in `LF_REQUESTS.md`).
#[test]
fn the_same_application_at_a_false_equation_is_refused() {
    let p = pinned(
        WRONG_PROVENANCE,
        "8c59aa63249c62dd1a2975a73da3f6ac2ce42046a32d01e83014b3c1d34da313",
        WRONG,
        &["LF_FT.idemWrong"],
    );
    lean_rejected(&p, "LF_FT.idemWrong");
    refused(WRONG, "op 2 2 = op 2 3 is false");
}
