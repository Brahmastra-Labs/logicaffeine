//! ★ THE CONCURRENCY-LOWERING KNOB LOCK ★ — pins the trade-off knob for how the C-family backends lower
//! LOGOS's deterministic concurrency (ucontext stackful vs stackless state machine), mirroring the
//! discipline of `cuda_knobs_lock`: every knob names a REAL trade-off (a regime it wins AND a regime it
//! loses — otherwise it would be baked in, not a knob), and the capability-forced points are honest.
//!
//! The one hard invariant this guards: a target with no `ucontext` (a GPU kernel, WASM) is FORCED to the
//! stackless lowering — so when the concurrency scheduler lands, the CUDA tier cannot silently pick the
//! ucontext path it physically cannot compile.

use logicaffeine_compile::codegen_c::concurrency::{CTarget, ConcurrencyLowering};

#[test]
fn every_setting_documents_a_real_tradeoff() {
    let rows = ConcurrencyLowering::tradeoffs();
    assert_eq!(
        rows.len(),
        2,
        "the knob has exactly two settings (Ucontext / Stackless)"
    );
    for r in rows {
        assert!(
            !r.wins.is_empty(),
            "`{}` must name where it wins",
            r.setting
        );
        assert!(
            !r.loses.is_empty(),
            "`{}` must name where it loses — else it would be baked in, not a knob",
            r.setting
        );
        // A genuine trade-off: the two clauses must differ (not a rubber-stamp).
        assert_ne!(
            r.wins, r.loses,
            "`{}`: wins and loses must be distinct regimes",
            r.setting
        );
    }
    // Both settings are covered — no setting lacks a trade-off row.
    let settings: Vec<&str> = rows.iter().map(|r| r.setting).collect();
    assert!(
        settings.contains(&"Ucontext") && settings.contains(&"Stackless"),
        "both settings documented: {settings:?}"
    );
}

#[test]
fn targets_without_ucontext_are_forced_stackless() {
    // The capability-forced invariant: no ucontext ⟺ stackless is the only expressible lowering.
    for t in [
        CTarget::HostC,
        CTarget::RiscV,
        CTarget::Tenstorrent,
        CTarget::Cuda,
        CTarget::Wasm,
    ] {
        let chosen = ConcurrencyLowering::for_target(t);
        if t.has_ucontext() {
            assert_eq!(
                chosen,
                ConcurrencyLowering::Ucontext,
                "{t:?} has ucontext → defaults to the simpler stackful lowering"
            );
        } else {
            assert_eq!(
                chosen,
                ConcurrencyLowering::Stackless,
                "{t:?} has NO ucontext → MUST be stackless (device/WASM cannot compile ucontext)"
            );
        }
    }
}

#[test]
fn cuda_is_stackless_and_riscv_is_ucontext() {
    // The two anchor points: CUDA (device, no ucontext) and RISC-V (qemu+glibc, has ucontext).
    assert_eq!(
        ConcurrencyLowering::for_target(CTarget::Cuda),
        ConcurrencyLowering::Stackless
    );
    assert_eq!(
        ConcurrencyLowering::for_target(CTarget::RiscV),
        ConcurrencyLowering::Ucontext
    );
    assert!(!CTarget::Cuda.has_ucontext());
    assert!(CTarget::RiscV.has_ucontext());
}
