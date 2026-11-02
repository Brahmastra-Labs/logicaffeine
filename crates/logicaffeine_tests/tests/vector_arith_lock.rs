//! vector_arith_lock — `LoopCell::VectorArith` is a FIRST-CLASS placeable fabric cell.
//!
//! The packed-SIMD tile is now a real cell, not a standalone module: it evaluates in the cell reducer, PLACES
//! onto the CGRA grid (`route_loop_body` → `CellKind::Simd`) and evaluates there identically, emits combinational
//! RTL that matches under iverilog, and is CAPABILITY-GATED by `target.simd` (a program needing a lane type the
//! target lacks is refused with a typed error). Four locks, one per property.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(vector_arith_lock)'`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::fabric_grid::{eval_grid, route_loop_body};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    emit_decision_fabric_testbench_dummy, emit_decision_fabric_verilog, eval_decision_fabric,
    DecisionFabric, LoopCell, Src,
};
use logicaffeine_compile::codegen_sva::fabric_target::{
    place_with_target, Capability, FabricTarget, Unplaceable,
};
use logicaffeine_compile::codegen_sva::simd::{eval_lanewise_add, LaneTy, VecOp};

/// Packed 64-bit operand patterns that stress lane boundaries and sign bits.
const SAMPLES: &[u64] = &[
    0,
    u64::MAX,
    0x0101_0101_0101_0101,
    0x8080_8080_8080_8080,
    0x7FFF_0001_FFFF_FFFF,
    0x1234_5678_9ABC_DEF0,
    0xDEAD_BEEF_CAFE_F00D,
    0x0000_0001_0000_0001,
];

fn vec_cell(op: VecOp, ty: LaneTy, a: Src, b: Src) -> LoopCell {
    LoopCell::VectorArith { op, ty, a, b }
}

/// The lane-wise reference for a `VecOp` (the tile's spec = N scalar ops).
fn spec(op: VecOp, ty: LaneTy, a: u64, b: u64) -> u64 {
    match op {
        VecOp::Add => eval_lanewise_add(ty, a, b),
        VecOp::And => a & b,
        VecOp::Or => a | b,
        VecOp::Xor => a ^ b,
    }
}

const OPS: &[VecOp] = &[VecOp::Add, VecOp::And, VecOp::Or, VecOp::Xor];
const TYS: &[LaneTy] = &[LaneTy::I8x8, LaneTy::I16x4, LaneTy::I32x2];

#[test]
fn vector_arith_evaluates_lane_wise_in_the_cell_reducer() {
    for &op in OPS {
        for &ty in TYS {
            let df = DecisionFabric {
                n_in: 2,
                cells: vec![vec_cell(op, ty, Src::In(0), Src::In(1))],
                result: Src::Val(0),
                escaped_first_return: false,
            };
            for &a in SAMPLES {
                for &b in SAMPLES {
                    let got = eval_decision_fabric(&df, &[a as i64, b as i64]) as u64;
                    assert_eq!(
                        got,
                        spec(op, ty, a, b),
                        "{op:?} {ty:?} eval mismatch for {a:#x},{b:#x}"
                    );
                }
            }
        }
    }
}

#[test]
fn vector_arith_places_onto_the_grid_and_evaluates_identically() {
    // A two-cell body: v0 = a `op` b (packed), v1 = v0 `xor` a — exercises routing a Simd tile's Val operand.
    for &op in OPS {
        for &ty in TYS {
            let cells = vec![
                vec_cell(op, ty, Src::In(0), Src::In(1)),
                vec_cell(VecOp::Xor, ty, Src::Val(0), Src::In(0)),
            ];
            let g = route_loop_body(&cells, 2, &[Src::Val(1)]);
            for &a in SAMPLES {
                for &b in SAMPLES {
                    let v0 = spec(op, ty, a, b);
                    let want = spec(VecOp::Xor, ty, v0, a);
                    let got = eval_grid(&g, &[a as i64, b as i64]) as u64;
                    assert_eq!(
                        got, want,
                        "placed grid {op:?} {ty:?} mismatch for {a:#x},{b:#x}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_emitted_vector_arith_rtl_matches_eval_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = std::env::temp_dir().join("logos_vecarith_iverilog");
    std::fs::create_dir_all(&dir).unwrap();

    for &op in OPS {
        for &ty in TYS {
            let df = DecisionFabric {
                n_in: 2,
                cells: vec![vec_cell(op, ty, Src::In(0), Src::In(1))],
                result: Src::Val(0),
                escaped_first_return: false,
            };
            let name = format!("{op:?}_{ty:?}").to_lowercase();
            let (mut inhex, mut outhex) = (String::new(), String::new());
            for &a in SAMPLES {
                for &b in SAMPLES {
                    inhex.push_str(&format!("{a:016x}\n{b:016x}\n"));
                    outhex.push_str(&format!(
                        "{:016x}\n",
                        eval_decision_fabric(&df, &[a as i64, b as i64]) as u64
                    ));
                }
            }
            let n = SAMPLES.len() * SAMPLES.len();
            let rtl = emit_decision_fabric_verilog(&name, &df);
            let tb = emit_decision_fabric_testbench_dummy(&name, 2, n);
            let srcv = dir.join(format!("decision_{name}.v"));
            let tbv = dir.join(format!("tb_dec_{name}.v"));
            let sim = dir.join(format!("sim_{name}.vvp"));
            std::fs::write(&srcv, &rtl).unwrap();
            std::fs::write(&tbv, &tb).unwrap();
            std::fs::write(dir.join(format!("dec_{name}_in.hex")), &inhex).unwrap();
            std::fs::write(dir.join(format!("dec_{name}_out.hex")), &outhex).unwrap();

            let build = std::process::Command::new("iverilog")
                .arg("-g2012")
                .arg("-o")
                .arg(&sim)
                .arg(&srcv)
                .arg(&tbv)
                .current_dir(&dir)
                .output()
                .expect("run iverilog");
            assert!(
                build.status.success(),
                "iverilog failed for {name}: {}\n--- RTL ---\n{rtl}",
                String::from_utf8_lossy(&build.stderr)
            );
            let run = std::process::Command::new("vvp")
                .arg(&sim)
                .current_dir(&dir)
                .output()
                .expect("vvp");
            let stdout = String::from_utf8_lossy(&run.stdout);
            assert!(
                stdout.contains("PASS:"),
                "VectorArith {name} RTL != eval under iverilog: {stdout}"
            );
        }
    }
    println!("VectorArith: emitted RTL ≡ eval under iverilog for every VecOp × LaneTy");
}

#[test]
fn vector_arith_is_capability_gated_by_target_simd() {
    let cell = vec![vec_cell(VecOp::Add, LaneTy::I32x2, Src::In(0), Src::In(1))];

    // wearable carries simd = Some(I32x2) → it places.
    assert!(
        place_with_target(&cell, 2, &[Src::Val(0)], &FabricTarget::wearable()).is_ok(),
        "wearable (simd I32x2) must place a matching packed-SIMD cell"
    );

    // tiny has no SIMD → typed refusal naming the missing lane capability.
    match place_with_target(&cell, 2, &[Src::Val(0)], &FabricTarget::tiny()) {
        Err(Unplaceable::MissingCapability { needed, .. }) => {
            assert_eq!(
                needed,
                Capability::Simd(LaneTy::I32x2),
                "refusal must name the missing Simd(I32x2)"
            );
        }
        other => {
            panic!("tiny must refuse a packed-SIMD cell with MissingCapability, got {other:?}")
        }
    }

    // A target with the WRONG lane type refuses too (lane types must match exactly).
    let wrong = FabricTarget {
        simd: Some(LaneTy::I16x4),
        ..FabricTarget::wearable()
    };
    assert!(
        matches!(
            place_with_target(&cell, 2, &[Src::Val(0)], &wrong),
            Err(Unplaceable::MissingCapability {
                needed: Capability::Simd(LaneTy::I32x2),
                ..
            })
        ),
        "a target providing only I16x4 must refuse an I32x2 cell"
    );
}
