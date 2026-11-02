//! fabric_fold_lock — a fabric engine spends a cycle only on work or on a wait that is real.
//!
//! A choreography op that only signals — a push of the page an op filled, a pop of the page it read, a semaphore
//! increment telling another engine it is done — commits in the last cycle of the op before it; a barrier, whose reads
//! and writes the op before it has already landed, and an accumulator's release cost nothing; the last cycle of an op
//! also takes the loop bound after it and checks the guards it reaches. A two-head attention pass, whose body selects
//! one head's operands by the point's own column, reads only the head it takes, a point a cycle where both heads' slots
//! share a FIFO's port. Every fabric lock holds the RTL to the evaluator in the twin's cycles; this one holds where the
//! cycles go, on the decode step of each machine:
//!
//! - no engine spends a cycle on a barrier or a release, and no movement engine on a push;
//! - an engine spends at most a few cycles on loop bounds;
//! - each of the step's two 384-point attention passes takes under twice its points: a point a cycle, one head read,
//!   where reading both heads' slots would take two.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test fabric_fold_lock`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::Engine;
use logicaffeine_compile::spatial::lower::fabric::{fabric_cycle_breakdown, fabric_micro_listing, Activity, CycleBreakdown};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;

fn decode_step() -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the model");
    m.decode_step_lg(j["golden_trace"]["prompt_tokens"][0].as_u64().unwrap() as usize, 0, &[], &[])
}

fn fabrics() -> [SpatialTarget; 3] {
    [SpatialTarget::sky_fabric(16, 64), SpatialTarget::xc7a200t_fabric(16, 64), SpatialTarget::xc7a100t_fabric(16, 64)]
}

fn breakdown(target: &SpatialTarget) -> CycleBreakdown {
    let p = compile_program_to_spatial(&decode_step(), target).unwrap().program.expect("the step lowers whole");
    fabric_cycle_breakdown(&p, target).expect("the fabric lowers the step")
}

fn busy(b: &CycleBreakdown, engine: Option<Engine>, activity: Activity) -> u64 {
    b.engines
        .iter()
        .filter(|e| engine.is_none_or(|g| e.engine == g))
        .map(|e| e.cycles.get(&activity).copied().unwrap_or(0))
        .sum()
}

#[test]
fn no_engine_spends_a_cycle_on_a_barrier_a_release_or_a_movement_push() {
    for target in fabrics() {
        let b = breakdown(&target);
        assert_eq!(busy(&b, None, Activity::Busy("barrier")), 0, "{}: a barrier costs nothing", target.name);
        assert_eq!(busy(&b, None, Activity::Busy("release")), 0, "{}: a release costs nothing", target.name);
        for e in b.engines.iter().filter(|e| e.engine != Engine::Compute) {
            assert_eq!(
                e.cycles.get(&Activity::Busy("push")).copied().unwrap_or(0),
                0,
                "{}: {:?} pushes each page in the last cycle of the read that filled it",
                target.name,
                e.engine
            );
        }
    }
}

#[test]
fn an_engine_spends_at_most_a_few_cycles_on_loop_bounds() {
    for target in fabrics() {
        let b = breakdown(&target);
        for e in &b.engines {
            let looping = e.cycles.get(&Activity::Looping).copied().unwrap_or(0);
            assert!(looping <= 24, "{}: {:?} spends {looping} cycles on loop bounds", target.name, e.engine);
        }
    }
}

/// The cycles and the iteration space (`rows × cols`) of every Generic op of the decode step on `target`.
fn generic_ops(target: &SpatialTarget) -> Vec<(u64, u64)> {
    let p = compile_program_to_spatial(&decode_step(), target).unwrap().program.expect("the step lowers whole");
    let extents = |l: &str| -> Vec<u64> {
        l.match_indices("extent: ").map(|(i, _)| l[i + 8..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap()).collect()
    };
    fabric_micro_listing(&p, target)
        .expect("the fabric lowers the step")
        .into_iter()
        .flat_map(|(_, _, lines)| lines)
        .filter(|l| l.starts_with("generic ["))
        .map(|l| {
            let cycles: u64 = l[l.find('[').unwrap() + 1..l.find(']').unwrap()].parse().unwrap();
            let e = extents(&l);
            (cycles, e[0] * e[1])
        })
        .collect()
}

#[test]
fn a_two_head_attention_pass_reads_one_head_a_point() {
    for target in fabrics() {
        let passes: Vec<u64> = generic_ops(&target).into_iter().filter(|(_, points)| *points == 384).map(|(c, _)| c).collect();
        assert_eq!(passes.len(), 2, "{}: the step's two 384-point attention passes", target.name);
        for cycles in passes {
            assert!(cycles < 2 * 384, "{}: a 384-point pass takes {cycles} cycles — a head's slots read twice a point", target.name);
        }
    }
}
