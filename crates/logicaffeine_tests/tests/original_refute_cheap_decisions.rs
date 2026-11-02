//! A conversion refuted at one cheap component is a REJECTION, whatever the other component costs.
//!
//! Lost decision (lane LG, W06 quiet run, `W06_SCALE_20260923.md`): Arena `perf/refute-cheap-first`
//! and `perf/refute-cheap-last` were rejected in 0.1 s by the 2026-09-21 production checker and
//! timed out at their 50 s deadline on 2026-09-23. Cause: after a conversion said "not equal", the
//! reader asked whether that answer was "really" decided by normalizing BOTH WHOLE SIDES again —
//! `(false, count #200)` against `(true, count #201)` — although the conversion had refuted at the
//! booleans without ever evaluating `count`. The audit cost more than the conversion it audited, and
//! it could not see what the conversion compared. The conversion now reports its own resource stops
//! (`observe_resource_stops`), and only those make a refusal undecided.
//!
//! Original witnesses (`fixtures/lean_semantics/LF_RefuteCheap.lean`, Lean 4.14.0, pinned
//! lean4export): the Arena pair at `n = 2000`, where `count #n` needs about four million reductions —
//! more than any normalization budget here covers. A whole-side audit can only call these undecided;
//! the conversion that ran decides them. Lean's kernel rejects `cheapFirst` (recorded), and proves
//! both claims false (`cheapFirst_false`, `cheapLast_false`, accepted by Lean's kernel and by both
//! kernels here).
use logicaffeine_kernel::interface::lean::{self as adapter, DiagnosticCode, Verdict};
use logicaffeine_kernel::{infer_type, is_subtype, recheck, Context};
use serde_json::Value;

const SOURCE: &str = include_str!("fixtures/lean_semantics/LF_RefuteCheap.lean");
const SOURCE_SHA: &str = "c4f14b5e4c21e69400c65ed19f7f441fa5b355516b1f9e70df259b950faa7d88";
const FIRST: &str = include_str!("fixtures/lean_semantics/LF_RefuteCheap.first.ndjson");
const FIRST_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_RefuteCheap.first.provenance.json");
const LAST: &str = include_str!("fixtures/lean_semantics/LF_RefuteCheap.last.ndjson");
const LAST_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_RefuteCheap.last.provenance.json");
const REFUTATIONS: &str = include_str!("fixtures/lean_semantics/LF_RefuteCheap.refutations.ndjson");
const REFUTATIONS_PROVENANCE: &str =
    include_str!("fixtures/lean_semantics/LF_RefuteCheap.refutations.provenance.json");
// The Arena's own inputs (byte-identical to the official artifact named in `payloads-214.json`).
const ARENA_FIRST: &str = include_str!("fixtures/lean_arena/refute-cheap-first.ndjson");
const ARENA_LAST: &str = include_str!("fixtures/lean_arena/refute-cheap-last.ndjson");

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}

fn pinned(provenance: &str, export_sha: &str, input: &str, roots: &[&str]) -> Value {
    let p: Value = serde_json::from_str(provenance).unwrap();
    assert_eq!(
        p["source"],
        "crates/logicaffeine_tests/tests/fixtures/lean_semantics/LF_RefuteCheap.lean"
    );
    assert_eq!(p["source_sha256"], SOURCE_SHA);
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(p["expression_equality"], "structural");
    assert_eq!(
        (p["compiler_exit"].as_i64(), p["exporter_exit"].as_i64()),
        (Some(0), Some(0))
    );
    assert_eq!(p["export_sha256"], export_sha);
    assert_eq!(p["records"], input.lines().count());
    assert_eq!(p["export_header"]["meta"]["lean"]["version"], "4.14.0");
    let declared: Vec<&str> = p["declarations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap())
        .collect();
    assert_eq!(declared, roots);
    p
}

fn rejected_as_invalid(input: &str, what: &str) {
    let verdict = adapter::check_str(input).expect_err("a false claim is refused");
    assert!(matches!(verdict, Verdict::Reject(_)),
        "{what}: the conversion refuted it, so it is REJECTED — never declined as undecided: {verdict:?}");
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::InvalidProof,
        "{what}: {verdict:?}"
    );
}

#[test]
fn a_refutation_at_the_cheap_first_component_is_a_rejection() {
    let p = pinned(
        FIRST_PROVENANCE,
        "0f9632bc68748a0f72b3c126c0ceb78d73d68b1d3f772f20057198d5b4e1a44e",
        FIRST,
        &["LF_RC.cheapFirst"],
    );
    let verdicts: Vec<&str> = p["lean_kernel_verdicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        verdicts.iter().any(|v| v
            .starts_with("LF_RC.cheapFirst: kernel REJECTED: (kernel) declaration type mismatch")),
        "Lean 4.14.0's kernel refused cheapFirst: {verdicts:?}"
    );
    assert!(SOURCE.contains("let n := 2000"));
    rejected_as_invalid(FIRST, "LF_RC.cheapFirst");
}

#[test]
fn a_refutation_at_the_cheap_last_component_is_a_rejection() {
    pinned(
        LAST_PROVENANCE,
        "2f6d7d446315e3c927afe478278ef42f3160de871e7bd3e3f7eeb81aafb1e046",
        LAST,
        &["LF_RC.cheapLast"],
    );
    rejected_as_invalid(LAST, "LF_RC.cheapLast");
}

/// Lean's kernel proves both claims false; both kernels here accept those proofs too, so a checker
/// that accepted either claim would be proving `False`.
#[test]
fn leans_refutations_of_both_claims_are_accepted_by_both_kernels() {
    let roots = ["LF_RC.cheapFirst_false", "LF_RC.cheapLast_false"];
    let p = pinned(
        REFUTATIONS_PROVENANCE,
        "88c5413212b8f526eb801942525a6ee6f8ac6a0242aa15b9f0463cb034a29d2e",
        REFUTATIONS,
        &roots,
    );
    let axioms: Vec<&str> = p["lean_print_axioms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    let (ctx, report) = adapter::import_audited(REFUTATIONS)
        .unwrap_or_else(|v| panic!("Lean's kernel accepts both refutations: {v:?}"));
    for root in roots {
        assert!(
            axioms.contains(&format!("'{root}' does not depend on any axioms").as_str()),
            "{axioms:?}"
        );
        let audit = report
            .declarations
            .iter()
            .find(|d| d.name == n(root))
            .unwrap_or_else(|| panic!("{root} is audited"));
        assert_eq!(audit.kind, "thm", "{root}");
        assert!(audit.axioms.is_empty(), "{root}: {:?}", audit.axioms);
        both_kernels_check_the_published_proof(&ctx, &n(root));
    }
}

fn both_kernels_check_the_published_proof(ctx: &Context, name: &str) {
    let statement = ctx
        .get_global(name)
        .unwrap_or_else(|| panic!("{name} is published"))
        .clone();
    let proof = ctx
        .get_definition_body(name)
        .unwrap_or_else(|| panic!("{name} keeps its proof"))
        .clone();
    let main = infer_type(ctx, &proof).unwrap_or_else(|e| panic!("{name}: primary kernel: {e:?}"));
    let independent =
        recheck(ctx, &proof).unwrap_or_else(|e| panic!("{name}: independent kernel: {e:?}"));
    for inferred in [&main, &independent] {
        assert!(
            is_subtype(ctx, inferred, &statement) && is_subtype(ctx, &statement, inferred),
            "{name}: the proof proves exactly the declared statement"
        );
    }
}

/// The Arena inputs themselves (`n = 200`): rejected, and the refusal names the conversion.
#[test]
fn the_arena_pair_is_rejected_as_an_invalid_proof() {
    for (input, what, name) in [
        (
            ARENA_FIRST,
            "perf/refute-cheap-first",
            "kernel_refute_cheap_first",
        ),
        (
            ARENA_LAST,
            "perf/refute-cheap-last",
            "kernel_refute_cheap_last",
        ),
    ] {
        assert!(
            input.contains(&format!("{name:?}")),
            "{what} is the Arena's export of {name}"
        );
        rejected_as_invalid(input, what);
    }
}
