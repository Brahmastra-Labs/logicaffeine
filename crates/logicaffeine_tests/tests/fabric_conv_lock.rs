//! fabric_conv_lock — a fixed-point COMPUTER-VISION stencil on the spatial fabric.
//!
//! A 3×3 Gaussian-blur convolution `(Σ wᵢ·pᵢ) >> 4` (kernel [1,2,1;2,4,2;1,2,1], ÷16) — the CV
//! workhorse in the quantized fixed-point form FPGAs actually use — is compiled to a dispatch-free
//! residual, lowered to a dataflow circuit, and PLACED on the 2-D switchbox tile array. It is proven
//! ≡ the running VM four ways: the flat circuit by `eval_fabric`, the placed grid by `eval_grid`, and
//! the EMITTED Verilog by a real gate-level simulator (iverilog). This is `vision.lg`'s stencil as
//! certified silicon — the supercompiler→fabric bridge carrying a real kernel, `Op::Shr` included.

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::fabric_grid::{eval_grid, grid_area, route_leveled};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    emit_fabric_verilog, eval_fabric, residual_from_source, AluOp, FabricOp,
};
use logicaffeine_compile::compile::vm_outcome;

// Straight-line fixed-weight Gaussian stencil over 9 scalar pixel inputs (the specialized form the
// supercompiler produces from a fixed kernel — directly fabric-lowerable, no loop, no Seq access).
const DECL: &str = "## To gaussBlur3 (p1: Int, p2: Int, p3: Int, p4: Int, p5: Int, p6: Int, p7: Int, p8: Int, p9: Int) -> Int:\n\
    \x20   Let acc be p1 + 2 * p2 + p3 + 2 * p4 + 4 * p5 + 2 * p6 + p7 + 2 * p8 + p9.\n\
    \x20   Return acc shifted right by 4.";

fn source_for(px: &[i64; 9]) -> String {
    let args = px
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{DECL}\n## Main\n    Show gaussBlur3({args}).\n")
}

fn vm_oracle(px: &[i64; 9]) -> i64 {
    let out = vm_outcome(&source_for(px));
    assert!(out.error.is_none(), "VM error: {:?}", out.error);
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int")
}

fn residual() -> (Vec<FabricOp>, usize, usize) {
    let src = format!("{DECL}\n## Main\n    Show gaussBlur3(0, 0, 0, 0, 0, 0, 0, 0, 0).");
    residual_from_source(&src, "gaussBlur3")
}

// Deterministic 8-bit pixel windows (realistic grayscale 0..255), including flats, edges, ramps.
fn windows() -> Vec<[i64; 9]> {
    vec![
        [0, 0, 0, 0, 0, 0, 0, 0, 0],
        [255, 255, 255, 255, 255, 255, 255, 255, 255],
        [0, 0, 0, 128, 128, 128, 255, 255, 255], // horizontal edge
        [10, 20, 30, 40, 50, 60, 70, 80, 90],    // ramp
        [200, 10, 200, 10, 240, 10, 200, 10, 200], // checker
        [100, 100, 100, 100, 200, 100, 100, 100, 100], // bright center
        [17, 240, 3, 199, 88, 46, 130, 7, 251],
        [64, 64, 64, 64, 64, 64, 64, 64, 64],
    ]
}

#[test]
fn gauss_blur_residual_lowers_to_a_fabric_circuit_eq_vm() {
    let (ops, result, n_in) = residual();
    assert_eq!(n_in, 9, "nine pixel inputs");
    assert!(
        ops.iter().any(|o| matches!(o.op, AluOp::Shr)),
        "the Q-format rescale must lower to a Shr tile"
    );
    for w in windows() {
        assert_eq!(
            eval_fabric(&ops, &w, result),
            vm_oracle(&w),
            "conv circuit != VM on window {w:?}"
        );
    }
}

#[test]
fn gauss_blur_places_on_the_spatial_grid_eq_vm() {
    let (ops, result, n_in) = residual();
    let g = route_leveled(&ops, n_in, result);
    for w in windows() {
        assert_eq!(
            eval_grid(&g, &w),
            vm_oracle(&w),
            "placed grid != VM on {w:?}"
        );
    }
    let (rows, cols, used) = grid_area(&g);
    assert!(
        used > 0 && rows > 0 && cols > 0,
        "the stencil occupies a real tile array"
    );
    eprintln!("gaussBlur3 placed on a {rows}×{cols} tile array, {used} tiles used");
}

#[test]
fn gauss_blur_rtl_matches_vm_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (ops, result, n_in) = residual();
    let module = emit_fabric_verilog("gaussblur3", n_in, &ops, result);

    let ws = windows();
    let mut tb = String::from("module tb;\n  reg [63:0] ");
    tb.push_str(
        &(0..n_in)
            .map(|k| format!("in{k}"))
            .collect::<Vec<_>>()
            .join(","),
    );
    tb.push_str(";\n  wire [63:0] result;\n  fabric_gaussblur3 dut(");
    tb.push_str(
        &(0..n_in)
            .map(|k| format!("in{k}"))
            .collect::<Vec<_>>()
            .join(","),
    );
    tb.push_str(", result);\n  initial begin\n");
    for w in &ws {
        let drive = (0..n_in)
            .map(|k| format!("in{k}={};", w[k]))
            .collect::<Vec<_>>()
            .join(" ");
        tb.push_str(&format!(
            "    {drive} #1 $display(\"%0d\", $signed(result));\n"
        ));
    }
    tb.push_str("    $finish;\n  end\nendmodule\n");

    let dir = std::env::temp_dir().join("logos_fabric_conv_iverilog");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let modv = dir.join("gaussblur3.v");
    let tbv = dir.join("tb.v");
    let sim = dir.join("sim.vvp");
    std::fs::write(&modv, &module).expect("write module");
    std::fs::write(&tbv, &tb).expect("write tb");

    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(&modv)
        .arg(&tbv)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .output()
        .expect("run vvp");
    let got: Vec<i64> = String::from_utf8_lossy(&run.stdout)
        .lines()
        .map(str::trim)
        .filter_map(|l| l.parse::<i64>().ok())
        .collect();
    assert_eq!(got.len(), ws.len(), "one sim line per window");
    for (w, g) in ws.iter().zip(got.iter()) {
        assert_eq!(*g, vm_oracle(w), "RTL conv ≠ VM on {w:?} (rtl {g})");
    }
}

#[test]
fn gauss_blur_synthesizes_to_real_artix7_luts() {
    // The strongest silicon proof short of a bitstream: yosys maps the emitted convolution to actual
    // Xilinx Artix-7 primitives (LUT6/CARRY4/MUXF), and the resource count FITS the Nexys A7-100T
    // (63,400 LUT6). No Vivado — the open yosys flow. Skipped gracefully where yosys is absent.
    if !common::require_tool("yosys") {
        return;
    }
    let (ops, result, n_in) = residual();
    let module = emit_fabric_verilog("gaussblur3", n_in, &ops, result);
    let dir = std::env::temp_dir().join("logos_fabric_conv_yosys");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let modv = dir.join("gaussblur3.v");
    std::fs::write(&modv, &module).expect("write module");

    let out = std::process::Command::new("yosys")
        .arg("-p")
        .arg(format!(
            "read_verilog {}; synth_xilinx -top fabric_gaussblur3; stat",
            modv.display()
        ))
        .output()
        .expect("run yosys");
    assert!(
        out.status.success(),
        "yosys synth failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let log = String::from_utf8_lossy(&out.stdout);

    // Estimated logic cells — the headline resource figure.
    let lcs: usize = log
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("Estimated number of LCs:")
                .map(|s| s.trim().parse().ok())
        })
        .flatten()
        .expect("yosys stat must report an LC estimate");
    assert!(lcs > 0, "the convolution must synthesize to real logic");
    assert!(
        lcs < 63_400,
        "the stencil ({lcs} LCs) must fit the Nexys A7-100T fabric"
    );
    eprintln!(
        "gaussBlur3 synthesizes to {lcs} Artix-7 logic cells (fits the Nexys A7-100T ~{}× over)",
        63_400 / lcs.max(1)
    );
}
