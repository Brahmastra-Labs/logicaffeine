//! ═══════════════════════════════════════════════════════════════════════
//! METAPIPELINE II LOCK — P1 of the Plasticine lane: the initiation-interval model is Spatial's definition,
//! built from the certified loop-fabric IR, and its number MEANS something executable.
//!
//! A. Resource II is 1 for pipelined tiles and the unit's occupancy for a sequential unit (our bit-serial
//!    divider), and the stage II is `max(resource, recurrence, 1)`.
//! B. Recurrence II is the loop-carried read→write path length: `s' = s + i` → 1, `s' = s*3 + i` → 2,
//!    `s' = (s*3 + i)*2` → 3; a register that only copies (`t' = s`) adds nothing; a body that never
//!    reads carried state has recurrence 0.
//! C. NON-VACUOUS: a cycle-timed pipelined execution at the modeled II reproduces the sequential oracle
//!    exactly with zero hazards, and at II−1 it reads stale state (hazards > 0) and computes a DIFFERENT
//!    value on the deep-recurrence programs — the II is neither optimistic nor slack.
//! D. N-buffer depths follow Spatial's writer→reader rule (adjacent = 2, one stage apart = 3).
//! E. The pipeline II is the slowest stage's; cycles = fill + (iters−1)·II; the Plasticine target gates
//!    the depth (6 stages fit a PCU, 7 are refused typed).
//!
//! Honest boundary: MODELED cycles (Spatial's rules over our IR), never a Plasticine simulation.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(metapipeline_lock)'`
//! ═══════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::fabric_rtl::{
    eval_loop_fabric, loop_fabric_from_source, FuncUnit,
};
use logicaffeine_compile::codegen_sva::fabric_target::{Capability, FabricTarget, Unplaceable};
use logicaffeine_compile::codegen_sva::metapipeline::{
    eval_loop_fabric_at_ii, loop_stage, Metapipeline, Stage, UnitModel,
};

fn prog(body: &str) -> String {
    format!("## To f (n: Int) -> Int:\n    Let mutable s be 1.\n    Repeat for i from 1 to n:\n        Set s to {body}.\n    Return s.\n")
}

/// (name, source, expected recurrence II under the ideal pipelined model)
fn corpus() -> Vec<(&'static str, String, usize)> {
    vec![
        ("sum", prog("s + i"), 1),
        ("affine", prog("s * 3 + i"), 2),
        ("affine-deep", prog("(s * 3 + i) * 2"), 3),
        ("mul-chain", prog("((s * 2) * 3) * 5 + i"), 4),
        ("no-carry", prog("i * 7 + 1"), 0),
        ("select", "## To f (n: Int) -> Int:\n    Let mutable s be 1.\n    Repeat for i from 1 to n:\n        If s is greater than 10:\n            Set s to s - i.\n        Else:\n            Set s to s + i * 2.\n    Return s.\n".to_string(), 2),
    ]
}

#[test]
fn recurrence_ii_is_the_loop_carried_read_to_write_path_length() {
    let model = UnitModel::ideal_pipelined();
    for (name, src, want) in corpus() {
        let lf = loop_fabric_from_source(&src, "f");
        let st = loop_stage(&lf, &model, name);
        assert_eq!(st.recurrence_ii, want, "{name}: recurrence II");
        assert_eq!(
            st.resource_ii, 1,
            "{name}: every tile is pipelined under the ideal model"
        );
        assert_eq!(
            st.ii(),
            want.max(1),
            "{name}: stage II = max(resource, recurrence, 1)"
        );
        assert!(
            st.latency >= st.recurrence_ii && st.latency >= 1,
            "{name}: latency covers the recurrence path"
        );
        assert_eq!(st.cells, lf.body.len());
    }
}

#[test]
fn resource_ii_is_the_sequential_units_occupancy() {
    // Our bit-serial divider is sequential: occupancy = latency = bits cycles, so a stage with a divide in it
    // cannot issue faster than that even with no loop-carried dependency.
    let src = prog("i / 3 + 1");
    let lf = loop_fabric_from_source(&src, "f");
    let ideal = loop_stage(&lf, &UnitModel::ideal_pipelined(), "div-ideal");
    let sky = loop_stage(&lf, &UnitModel::sky_fabric(32), "div-sky");
    assert!(
        lf.body.iter().any(|c| matches!(
            c,
            logicaffeine_compile::codegen_sva::fabric_rtl::LoopCell::Func {
                unit: FuncUnit::DivS | FuncUnit::FloorDivS,
                ..
            }
        )),
        "the body contains a divide"
    );
    assert_eq!(ideal.resource_ii, 1);
    assert_eq!(
        sky.resource_ii, 32,
        "a 32-bit bit-serial divide occupies its unit for 32 cycles"
    );
    assert_eq!(sky.ii(), 32);
    assert!(sky.latency >= 32);
}

#[test]
fn pipelined_execution_at_the_modeled_ii_matches_the_oracle_and_one_less_does_not() {
    let model = UnitModel::ideal_pipelined();
    let mut broke_below = 0usize;
    for (name, src, _) in corpus() {
        let lf = loop_fabric_from_source(&src, "f");
        let st = loop_stage(&lf, &model, name);
        for n in [0i64, 1, 2, 3, 5, 8, 13] {
            let want = eval_loop_fabric(&lf, &[n]);
            let at = eval_loop_fabric_at_ii(&lf, &[n], st.ii(), &model);
            assert_eq!(
                at.value,
                want,
                "{name}(n={n}) at II={} must equal the sequential oracle",
                st.ii()
            );
            assert_eq!(
                at.hazards, 0,
                "{name}(n={n}) at the modeled II has no hazards"
            );
            assert_eq!(
                at.iterations as i64,
                n.max(0),
                "{name}: one iteration per induction value"
            );
            if n >= 1 {
                // The result register's last write completes at issue(n-1) + its read→write depth.
                assert_eq!(
                    at.cycles,
                    (n as usize - 1) * st.ii() + st.recurrence_ii,
                    "{name}(n={n}): last write completes at (n-1)*II + depth"
                );
            }
            if st.ii() >= 2 && n >= 3 {
                let below = eval_loop_fabric_at_ii(&lf, &[n], st.ii() - 1, &model);
                assert!(
                    below.hazards > 0,
                    "{name}(n={n}) at II-1 must read stale state"
                );
                if below.value != want {
                    broke_below += 1;
                }
            }
        }
    }
    assert!(broke_below >= 3, "II-1 must actually change the computed value on the deep-recurrence programs (non-vacuity), got {broke_below} cases");
}

#[test]
fn n_buffer_depths_follow_spatials_writer_to_reader_rule() {
    assert_eq!(
        Metapipeline::buffer_depth(0, 1),
        2,
        "adjacent stages double-buffer"
    );
    assert_eq!(Metapipeline::buffer_depth(0, 2), 3);
    assert_eq!(Metapipeline::buffer_depth(1, 4), 4);
    assert_eq!(
        Metapipeline::buffer_depth(3, 3),
        1,
        "a memory read in its own stage needs one copy"
    );
    let mp = Metapipeline {
        stages: (0..3)
            .map(|i| Stage {
                name: format!("s{i}"),
                resource_ii: 1,
                recurrence_ii: 1,
                latency: 2,
                cells: 1,
                memory_ii: 1,
            })
            .collect(),
        edges: vec![(0, 1), (1, 2), (0, 2)],
    };
    assert_eq!(mp.buffer_depths(), vec![(0, 1, 2), (1, 2, 2), (0, 2, 3)]);
}

#[test]
fn pipeline_ii_is_the_slowest_stage_and_the_plasticine_target_gates_the_depth() {
    let model = UnitModel::ideal_pipelined();
    let stages: Vec<Stage> = corpus()
        .into_iter()
        .take(3)
        .map(|(name, src, _)| loop_stage(&loop_fabric_from_source(&src, "f"), &model, name))
        .collect();
    let mp = Metapipeline {
        stages,
        edges: vec![(0, 1), (1, 2)],
    };
    assert_eq!(
        mp.ii(),
        3,
        "sum(1) / affine(2) / affine-deep(3): the slowest stage sets the II"
    );
    assert!((mp.throughput() - 1.0 / 3.0).abs() < 1e-12);
    let fill: usize = mp.stages.iter().map(|s| s.latency).sum();
    assert_eq!(mp.cycles(1), fill);
    assert_eq!(mp.cycles(100), fill + 99 * 3);
    assert_eq!(mp.cycles(0), 0);
    let plasticine = FabricTarget::plasticine_shaped();
    mp.fits(&plasticine)
        .expect("three stages fit a six-stage PCU");
    let six = Metapipeline {
        stages: (0..6).map(|i| mp.stages[i % 3].clone()).collect(),
        edges: vec![],
    };
    six.fits(&plasticine)
        .expect("six stages are exactly the PCU depth");
    let seven = Metapipeline {
        stages: (0..7).map(|i| mp.stages[i % 3].clone()).collect(),
        edges: vec![],
    };
    match seven.fits(&plasticine) {
        Err(Unplaceable::MissingCapability {
            needed: Capability::Metapipeline(7),
            target: "plasticine-isca17",
        }) => {}
        other => panic!("seven stages must be refused typed on a six-stage PCU, got {other:?}"),
    }
    assert!(
        seven.fits(&FabricTarget::wide()).is_err(),
        "the abstract presets sustain a single stage"
    );
}
