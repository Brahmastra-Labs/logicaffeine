//! BACKEND FAILURE MATRIX — every backend LOGOS compiles to, and how it keeps the failure contract:
//! a program that stops with a reason (`Fail with …`, a contract's `, otherwise …`, a runtime trap)
//! stops with the SAME reason after the SAME output everywhere (`failure_corpus.in`).
//!
//! `status` is an EXHAUSTIVE match with NO wildcard: a new backend does not compile until it is
//! classified here. A backend is
//!   • `Exact`     — the named test runs the failure corpus on it byte-for-byte (verified to exist);
//!   • `Delegated` — it never executes failure paths itself: they run on the VM (proven `Exact`);
//!   • `Gap`       — a KNOWN shortfall, stated plainly. The gap count is a ratchet that only falls.
//!
//! ⚠ Never reclassify a `Gap` as `Exact` without the test that proves it, and never raise the ceiling.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    TreeWalker,
    Vm,
    AotRust,
    CFromAst,
    CFromBytecode,
    Wasm,
    ForgeJit,
    WireFormat,
    RiscV,
    SoftCore,
    Fabric,
    Cuda,
    Tenstorrent,
    AotNativeTier,
    Dram,
    Spatial,
    Analog,
}

const ALL: &[Backend] = &[
    Backend::TreeWalker,
    Backend::Vm,
    Backend::AotRust,
    Backend::CFromAst,
    Backend::CFromBytecode,
    Backend::Wasm,
    Backend::ForgeJit,
    Backend::WireFormat,
    Backend::RiscV,
    Backend::SoftCore,
    Backend::Fabric,
    Backend::Cuda,
    Backend::Tenstorrent,
    Backend::AotNativeTier,
    Backend::Dram,
    Backend::Spatial,
    Backend::Analog,
];

enum Status {
    /// `(file, test fn)` — the test that proves it.
    Exact(&'static str, &'static str),
    /// Why its failure paths never execute on it, and the test in THIS file that proves it.
    Delegated(&'static str, &'static str),
    /// What is missing.
    Gap(&'static str),
}

fn status(b: Backend) -> Status {
    use Backend::*;
    use Status::*;
    match b {
        TreeWalker | Vm => Exact("failure_all_tiers.rs", "failures_on_the_tree_walker_and_vm"),
        AotRust => Exact("failure_all_tiers.rs", "failures_on_the_rust_tier"),
        CFromAst => Exact("failure_all_tiers.rs", "failures_on_the_c_tier"),
        CFromBytecode => Exact(
            "failure_all_tiers.rs",
            "failures_on_the_c_tier_lowered_from_bytecode",
        ),
        Wasm => Exact(
            "wasm_aot_lock.rs",
            "the_failure_corpus_stops_with_the_same_reason_on_wasm",
        ),
        ForgeJit => Exact(
            "failure_all_tiers.rs",
            "failure_under_the_forge_is_the_vm_failure",
        ),
        WireFormat => Exact(
            "failure_all_tiers.rs",
            "failure_survives_the_binary_wire_format",
        ),
        Dram => Delegated(
            "failure ops classify as ControlPlane: the host runs them on the VM",
            "dram_runs_failures_on_the_host",
        ),
        Spatial => Delegated(
            "numeric tensor choreographies only; a program with a failure path is refused",
            "spatial_refuses_every_failure_program",
        ),
        Analog => Delegated(
            "float dataflow regions only; a program with a failure path is refused",
            "analog_refuses_every_failure_program",
        ),
        RiscV => Exact(
            "riscv_aot_lock.rs",
            "the_failure_corpus_stops_with_the_same_reason_on_riscv",
        ),
        SoftCore => Exact(
            "soft_core_failure_lock.rs",
            "the_failure_corpus_stops_with_the_same_reason_on_the_soft_core_rtl",
        ),
        Fabric => Gap("a failure sets HALTED and suppresses later output; no reason is reported"),
        Cuda => Gap("the device-fault shim strips the reason; only a nonzero exit remains"),
        Tenstorrent => {
            Gap("the on-core kernel has no failure path; the host C oracle's stderr is not checked")
        }
        AotNativeTier => {
            Gap("a failure inside a native function aborts the process; no reason, no deopt")
        }
    }
}

/// The ratchet: the number of `Gap` rows. Lower it as each gap closes; it may never rise.
const GAP_CEILING: usize = 4;

fn test_source(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn every_exact_claim_names_a_real_test() {
    let mut missing = Vec::new();
    for &b in ALL {
        if let Status::Exact(file, test) = status(b) {
            let src = test_source(file);
            if !src.contains(&format!("fn {test}()")) {
                missing.push(format!("{b:?}: `{test}` not found in {file}"));
            }
        }
        if let Status::Delegated(_, test) = status(b) {
            if !test_source("backend_failure_matrix_lock.rs").contains(&format!("fn {test}()")) {
                missing.push(format!("{b:?}: delegation proof `{test}` not found"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "an Exact claim without its proof:\n{}",
        missing.join("\n")
    );
}

#[test]
fn every_backend_is_listed() {
    // `status` is exhaustive over the enum; this keeps `ALL` exhaustive over it too.
    for &b in ALL {
        let _ = status(b);
    }
    let mut seen: Vec<String> = ALL.iter().map(|b| format!("{b:?}")).collect();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), ALL.len(), "a backend is listed twice");
    assert_eq!(
        ALL.len(),
        17,
        "a backend was added to the enum but not to ALL (or removed)"
    );
}

#[test]
fn the_gap_count_only_falls() {
    let gaps: Vec<String> = ALL
        .iter()
        .filter_map(|&b| match status(b) {
            Status::Gap(why) => Some(format!("{b:?}: {why}")),
            _ => None,
        })
        .collect();
    assert!(
        gaps.len() <= GAP_CEILING,
        "{} failure gaps exceed the ceiling {GAP_CEILING}:\n{}",
        gaps.len(),
        gaps.join("\n")
    );
    assert_eq!(
        gaps.len(),
        GAP_CEILING,
        "a gap closed — lower GAP_CEILING to {} to lock it in",
        gaps.len()
    );
}

// ─── Delegation proofs ───────────────────────────────────────────────────────

const FAILURES: &[(&str, &str, &str, &str)] = include!("failure_corpus.in");

#[test]
fn dram_runs_failures_on_the_host() {
    use logicaffeine_compile::codegen_dram::{fragment, DramFragment};
    use logicaffeine_compile::semantics::builtins::BuiltinId;
    use logicaffeine_compile::vm::Op;
    for op in [
        Op::FailWith { msg: 0 },
        Op::CallBuiltin {
            dst: 0,
            builtin: BuiltinId::Fail,
            args_start: 1,
            arg_count: 1,
        },
        Op::CallBuiltin {
            dst: 0,
            builtin: BuiltinId::FailReason,
            args_start: 1,
            arg_count: 1,
        },
    ] {
        assert_eq!(
            fragment(&op),
            DramFragment::ControlPlane,
            "{op:?} must run on the host VM"
        );
    }
}

#[test]
fn spatial_refuses_every_failure_program() {
    let target = logicaffeine_compile::spatial::target::SpatialTarget::wormhole();
    for (name, src, _, _) in FAILURES {
        assert!(
            logicaffeine_compile::spatial::pipeline::template_from_source(src, &target).is_err(),
            "{name}: a failure program must never lower to a spatial choreography"
        );
    }
}

#[test]
fn analog_refuses_every_failure_program() {
    for (name, src, _, _) in FAILURES {
        let program = logicaffeine_compile::compile::compile_to_program(src).expect("compiles");
        assert!(
            logicaffeine_compile::codegen_ana::analog_refusal(&program, &[]).is_some(),
            "{name}: a failure program must never lower to the analog fabric"
        );
    }
}
