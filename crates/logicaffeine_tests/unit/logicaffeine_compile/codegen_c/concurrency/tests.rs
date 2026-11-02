use super::*;

#[test]
fn cuda_and_wasm_are_forced_stackless() {
    assert_eq!(
        ConcurrencyLowering::for_target(CTarget::Cuda),
        ConcurrencyLowering::Stackless
    );
    assert_eq!(
        ConcurrencyLowering::for_target(CTarget::Wasm),
        ConcurrencyLowering::Stackless
    );
    assert!(!CTarget::Cuda.has_ucontext());
    assert!(!CTarget::Wasm.has_ucontext());
}

#[test]
fn ucontext_targets_default_to_ucontext() {
    for t in [CTarget::HostC, CTarget::RiscV, CTarget::Tenstorrent] {
        assert!(t.has_ucontext());
        assert_eq!(
            ConcurrencyLowering::for_target(t),
            ConcurrencyLowering::Ucontext
        );
    }
}

#[test]
fn default_scheduler_knobs_mirror_the_oracle_config() {
    // The AOT scheduler's DEFAULT must be the exact point `run_vm_concurrent` pins:
    // Fifo / workers=1 / Logical / seed=0 — so a default-knob AOT run == the VM oracle.
    let k = SchedulerKnobs::default();
    assert_eq!(k.config.policy, SchedulePolicy::Fifo);
    assert_eq!(k.config.workers, 1);
    assert_eq!(k.config.clock, ClockMode::Logical);
    assert_eq!(k.seed, 0);
    let d = k.emit_defines();
    assert!(
        d.contains("#define LOGOS_SCHED_POLICY LOGOS_SCHED_FIFO"),
        "{d}"
    );
    assert!(d.contains("#define LOGOS_SCHED_WORKERS 1"), "{d}");
    assert!(
        d.contains("#define LOGOS_SCHED_CLOCK LOGOS_CLOCK_LOGICAL"),
        "{d}"
    );
    assert!(d.contains("#define LOGOS_SCHED_SEED 0ULL"), "{d}");
}

#[test]
fn every_policy_emits_a_distinct_define() {
    use std::collections::BTreeSet;
    let policies = [
        SchedulePolicy::Fifo,
        SchedulePolicy::Lifo,
        SchedulePolicy::RoundRobin,
        SchedulePolicy::Random,
        SchedulePolicy::Priority,
    ];
    let lines: BTreeSet<String> = policies
        .iter()
        .map(|p| {
            let k = SchedulerKnobs {
                config: SchedulerConfig::default().with_policy(*p),
                seed: 0,
            };
            k.emit_defines()
                .lines()
                .find(|l| l.contains("LOGOS_SCHED_POLICY"))
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(
        lines.len(),
        5,
        "each SchedulePolicy must map to a distinct emitted define: {lines:?}"
    );
}
