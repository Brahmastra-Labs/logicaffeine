//! fabric_cycle_breakdown_lock — where a fabric program's cycles go, engine by engine, and that every cycle is
//! accounted for.
//!
//! `fabric_cycle_breakdown` runs the twin that predicts the RTL's cycles and attributes each engine's every cycle: the
//! op kind it ran or the guard it passed (`Busy`), the guard that held it (`Waiting`), loop control, or done while
//! another engine still ran. This lock holds:
//!
//! - the breakdown's total is the twin's cycle count, which the fabric locks hold to the RTL;
//! - every engine accounts for every cycle but the two of the start pulse, on every program it is asked about;
//! - on the int8 decode step the compute engine runs the step's arithmetic — its tile matmuls and Generic bodies — and
//!   the movement engine its tile reads and writes, so the breakdown names the work where it happens.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test fabric_cycle_breakdown_lock`

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::Engine;
use logicaffeine_compile::spatial::lower::fabric::{fabric_cycle_breakdown, fabric_cycle_model, Activity};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;

fn decode_step() -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the model");
    m.decode_step_lg(j["golden_trace"]["prompt_tokens"][0].as_u64().unwrap() as usize, 0, &[], &[])
}

fn programs() -> Vec<(&'static str, String)> {
    vec![
        ("decode step", decode_step()),
        (
            "matmul",
            "## Main\nLet A be a 2 by 16 tensor of Int.\nSet item 1 of A to 7.\nLet W be a 16 by 16 tensor of Int.\nSet item 3 of W to -2.\nLet C be A matmul W.\n".into(),
        ),
        ("clamp", "## Main\nLet X be a 1 by 8 tensor of Int.\nSet item 2 of X to 300.\nLet Y be X clamped between -128 and 127.\n".into()),
    ]
}

#[test]
fn every_engine_accounts_for_every_cycle_of_the_twin() {
    for target in [SpatialTarget::sky_fabric(16, 64), SpatialTarget::xc7a100t_fabric(16, 64), SpatialTarget::xc7a200t_fabric(16, 64)] {
        for (label, src) in programs() {
            let p = compile_program_to_spatial(&src, &target).unwrap().program.unwrap_or_else(|| panic!("{label} lowers whole"));
            let b = fabric_cycle_breakdown(&p, &target).unwrap_or_else(|| panic!("{label}: the fabric lowers it"));
            assert_eq!(b.total, fabric_cycle_model(&p, &target), "{label}@{}: the breakdown's total is the twin's", target.name);
            assert!(!b.engines.is_empty(), "{label}@{}: engines", target.name);
            for e in &b.engines {
                let accounted: u64 = e.cycles.values().sum();
                assert_eq!(accounted, b.total - 2, "{label}@{}: {:?} {:?} accounts for every cycle after the start pulse", target.name, e.site, e.engine);
            }
        }
    }
}

#[test]
fn the_decode_step_s_work_is_named_where_it_runs() {
    let target = SpatialTarget::xc7a100t_fabric(16, 64);
    let p = compile_program_to_spatial(&decode_step(), &target).unwrap().program.unwrap();
    let b = fabric_cycle_breakdown(&p, &target).unwrap();
    let busy = |engine: Engine, kind: &'static str| -> u64 {
        b.engines.iter().filter(|e| e.engine == engine).map(|e| e.cycles.get(&Activity::Busy(kind)).copied().unwrap_or(0)).sum()
    };
    let movement = b.engines.iter().find(|e| e.engine != Engine::Compute).map(|e| e.engine).expect("a movement engine");
    assert!(busy(Engine::Compute, "matmul") > 0 && busy(Engine::Compute, "generic") > 0, "the compute engine runs the arithmetic");
    assert!(busy(movement, "read tiles") > 0 && busy(movement, "write tiles") > 0, "the movement engine moves the tiles");
    assert_eq!(busy(movement, "matmul") + busy(movement, "generic"), 0, "no arithmetic on the movement engine");
    assert_eq!(busy(Engine::Compute, "read tiles") + busy(Engine::Compute, "write tiles"), 0, "no tile traffic on the compute engine");
}
