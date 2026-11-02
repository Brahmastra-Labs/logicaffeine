//! csa_multiplier_lock — the pipelined compressor-tree multiplier every wide product shares.
//!
//! `emit_csa_multiplier_verilog` writes one parameterized module: the AW×BW partial-product rows reduced by 3:2
//! compressors, registered after CUT levels and (with FINAL) once more, the last two words resolved by a Kogge-Stone
//! add. The BigInt limb multiplier and the FPU's significand multiplier both instantiate it under their own names.
//!
//! - A: every product of every operand pair, for shapes from one row to seven, every register placement.
//! - B: random 53×53 and 64×64 products, at the cuts the FPU and the limb multiplier use.
//! - C: `fpu.v` (both copies) carries the generator's `fp_csa_mul` byte for byte, so no copy drifts from the one
//!   source.
//! - D: the target picks the engine's multiplier. The Caravel engine defaults `MUL_TREE` to the tree and every
//!   board engine to the DSP multiplies, and yosys agrees: the Caravel macro elaborates no variable-by-variable
//!   multiply, a board engine exactly its DSP ones. (coproc_pipeline_lock holds the coprocessor the same way.)
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test csa_multiplier_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

#[path = "common/multiplier_targets.rs"]
mod multiplier_targets;

use logicaffeine_compile::codegen_sva::fabric_pipe::emit_csa_multiplier_verilog;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_verilog_hw, emit_wprog_engine_soc_verilog_synth,
};
use multiplier_targets::{header_default, variable_multiplies};

fn repo(path: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Simulate `csa_under_test` instances, one per shape `(aw, bw, cut, final)`, over `pairs` per shape: each pair is
/// presented, the clock runs past the deepest pipeline, and the product is printed.
fn products(shapes: &[(u32, u32, u32, u32)], pairs: &[Vec<(u128, u128)>]) -> Vec<Vec<u128>> {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("csa.v"), emit_csa_multiplier_verilog("csa_under_test")).unwrap();
    let mut tb = String::from("module tb;\n  reg clk = 0;\n");
    for (k, &(aw, bw, cut, fin)) in shapes.iter().enumerate() {
        tb.push_str(&format!(
            "  reg [{am}:0] a{k}; reg [{bm}:0] b{k}; wire [{pm}:0] p{k};\n  \
             csa_under_test #(.AW({aw}), .BW({bw}), .CUT({cut}), .FINAL({fin})) m{k}(.clk(clk), .a(a{k}), .b(b{k}), .p(p{k}));\n",
            am = aw - 1,
            bm = bw - 1,
            pm = aw + bw - 1,
        ));
    }
    tb.push_str("  initial begin\n");
    for (k, list) in pairs.iter().enumerate() {
        for &(a, b) in list {
            tb.push_str(&format!(
                "    a{k} = 'h{a:x}; b{k} = 'h{b:x}; #1 clk = 1; #1 clk = 0; #1 clk = 1; #1 clk = 0; \
                 $display(\"P{k} %0h\", p{k});\n"
            ));
        }
    }
    tb.push_str("    $finish;\n  end\nendmodule\n");
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .current_dir(dir.path())
        .args(["-g2012", "-o", "sim.vvp", "tb.v", "csa.v"])
        .output()
        .expect("run iverilog");
    assert!(build.status.success(), "iverilog failed: {}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new("vvp").current_dir(dir.path()).arg("sim.vvp").output().expect("vvp");
    let mut out = vec![Vec::new(); shapes.len()];
    for line in String::from_utf8_lossy(&run.stdout).lines() {
        if let Some(rest) = line.strip_prefix('P') {
            let mut it = rest.split_whitespace();
            let k: usize = it.next().unwrap().parse().unwrap();
            out[k].push(u128::from_str_radix(it.next().unwrap(), 16).expect("hex product"));
        }
    }
    out
}

// ── A. small shapes, every pair ─────────────────────────────────────────────────────────────────────

#[test]
fn every_small_shape_multiplies_every_operand_pair_at_every_register_placement() {
    if !common::require_tool("iverilog") {
        return;
    }
    let shapes: Vec<(u32, u32, u32, u32)> = vec![
        (5, 1, 0, 0),
        (4, 2, 0, 1),
        (6, 3, 1, 0),
        (3, 7, 2, 1),
        (7, 5, 3, 1),
        (6, 6, 1, 1),
        (6, 6, 9, 0),
        (2, 6, 4, 0),
    ];
    let pairs: Vec<Vec<(u128, u128)>> = shapes
        .iter()
        .map(|&(aw, bw, _, _)| {
            let mut v = Vec::new();
            for a in 0..1u128 << aw {
                for b in 0..1u128 << bw {
                    v.push((a, b));
                }
            }
            v
        })
        .collect();
    let got = products(&shapes, &pairs);
    for (k, list) in pairs.iter().enumerate() {
        assert_eq!(got[k].len(), list.len(), "shape {:?}: one product per pair", shapes[k]);
        for (i, &(a, b)) in list.iter().enumerate() {
            assert_eq!(got[k][i], a * b, "shape {:?}: {a} × {b}", shapes[k]);
        }
    }
}

// ── B. the widths in use ────────────────────────────────────────────────────────────────────────────

#[test]
fn the_fpu_and_limb_widths_multiply_random_operands_exactly() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut state = 0x5EED_C5A0u64;
    let mut next = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let shapes = vec![(53, 53, 4, 1), (64, 64, 5, 1), (64, 64, 3, 0)];
    let mut pairs = Vec::new();
    for &(aw, bw, _, _) in &shapes {
        let am = if aw == 64 { u64::MAX } else { (1u64 << aw) - 1 };
        let bm = if bw == 64 { u64::MAX } else { (1u64 << bw) - 1 };
        let mut v: Vec<(u128, u128)> = vec![(0, 0), (am as u128, bm as u128), (1, bm as u128), (am as u128, 1)];
        v.extend((0..300).map(|_| ((next() & am) as u128, (next() & bm) as u128)));
        pairs.push(v);
    }
    let got = products(&shapes, &pairs);
    for (k, list) in pairs.iter().enumerate() {
        for (i, &(a, b)) in list.iter().enumerate() {
            assert_eq!(got[k][i], a * b, "shape {:?}: {a:#x} × {b:#x}", shapes[k]);
        }
    }
}

// ── C. one source ───────────────────────────────────────────────────────────────────────────────────

#[test]
fn every_fpu_copy_of_the_tree_is_the_generators() {
    let fp = emit_csa_multiplier_verilog("fp_csa_mul");
    for path in ["crates/logicaffeine_compile/src/codegen_sva/fpu.v", "benchmarks/silicon/fpu.v"] {
        let text = repo(path);
        assert_eq!(text.matches("module fp_csa_mul").count(), 1, "{path} defines fp_csa_mul once");
        assert!(
            text.contains(fp.trim_end()),
            "{path} carries an fp_csa_mul that is not emit_csa_multiplier_verilog(\"fp_csa_mul\")"
        );
    }
}

// ── D. the target picks the engine's multiplier ─────────────────────────────────────────────────────

#[test]
fn the_caravel_engine_defaults_to_the_tree_and_every_board_engine_to_the_dsp_multiply() {
    let caravel = repo("benchmarks/silicon/wpe_user_project.v");
    assert_eq!(
        header_default(&caravel, "wpe_soc_caravel", "MUL_TREE"),
        "1",
        "the Caravel engine is a sky130 build: its multiplies take the tree"
    );
    let instance = caravel.lines().find(|l| l.contains(" engine (")).expect("the engine's instance");
    assert!(
        !instance.contains("MUL_TREE"),
        "the user macro instantiates its engine on the baked default, never an override: {instance}"
    );
    assert_eq!(
        caravel.matches("fp_mul    #(.MUL_TREE(MUL_TREE)) u_fml").count(),
        1,
        "the float multiply follows the engine's choice"
    );
    for (label, rtl) in [
        ("synth", emit_wprog_engine_soc_verilog_synth("t")),
        ("hw", emit_wprog_engine_soc_verilog_hw("t")),
    ] {
        assert_eq!(
            header_default(&rtl, "wpe_soc_t", "MUL_TREE"),
            "0",
            "the {label} board engine takes the DSP multiply"
        );
    }
}

#[test]
fn the_caravel_engine_elaborates_no_variable_multiply_and_a_board_engine_exactly_its_dsp_ones() {
    if !common::require_tool("yosys") {
        return;
    }
    let caravel = repo("benchmarks/silicon/wpe_user_project.v");
    let board = emit_wprog_engine_soc_verilog_synth("t");
    let (asic, fpga) = std::thread::scope(|s| {
        let asic = s.spawn(|| variable_multiplies(&[("wpe_user_project.v", caravel.clone())], "wpe_user_project"));
        let fpga = s.spawn(|| variable_multiplies(&[("engine.v", board.clone())], "wpe_soc_t"));
        (asic.join().expect("yosys on the Caravel macro"), fpga.join().expect("yosys on the board engine"))
    });
    assert_eq!(asic, vec![], "the Caravel user macro elaborates variable multiplies {asic:?}");
    assert_eq!(fpga, vec![(53, 53), (64, 64)], "the board engine elaborates variable multiplies {fpga:?}");
}
