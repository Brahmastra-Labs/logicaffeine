//! fabric_sobel_lock — a full fixed-point SOBEL EDGE DETECTOR on the spatial fabric.
//!
//! Two gradients (Sobel-X `[-1,0,1;-2,0,2;-1,0,1]`, Sobel-Y `[-1,-2,-1;0,0,0;1,2,1]`) plus the
//! hardware-standard L1 edge magnitude `|Gx| + |Gy|` — with the branch-free absolute value
//! `|x| = (x ⊕ (x>>>63)) − (x>>>63)` (the Xor tile, newly unlocked in `residual_from_source`). The
//! whole detector stays STRAIGHT-LINE, so it lowers to one dispatch-free dataflow circuit, proven
//! ≡ the VM by eval, placed-grid eval, and iverilog gate-simulation, then synthesized to Artix-7.

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::fabric_grid::{eval_grid, grid_area, route_leveled};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    emit_fabric_verilog, eval_fabric, residual_from_source, AluOp, FabricOp,
};
use logicaffeine_compile::compile::vm_outcome;

const DECL: &str = "## To sobel3 (p1: Int, p2: Int, p3: Int, p4: Int, p5: Int, p6: Int, p7: Int, p8: Int, p9: Int) -> Int:\n\
    \x20   Let gx be (0 - p1) + p3 - 2 * p4 + 2 * p6 - p7 + p9.\n\
    \x20   Let gy be (0 - p1) - 2 * p2 - p3 + p7 + 2 * p8 + p9.\n\
    \x20   Let mx be gx shifted right by 63.\n\
    \x20   Let ax be (gx xor mx) - mx.\n\
    \x20   Let my be gy shifted right by 63.\n\
    \x20   Let ay be (gy xor my) - my.\n\
    \x20   Return ax + ay.";

fn source_for(px: &[i64; 9]) -> String {
    let args = px
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{DECL}\n## Main\n    Show sobel3({args}).\n")
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
    let src = format!("{DECL}\n## Main\n    Show sobel3(0, 0, 0, 0, 0, 0, 0, 0, 0).");
    residual_from_source(&src, "sobel3")
}

fn windows() -> Vec<[i64; 9]> {
    vec![
        [0, 0, 0, 0, 0, 0, 0, 0, 0],                   // flat → no edge
        [128, 128, 128, 128, 128, 128, 128, 128, 128], // flat → no edge
        [0, 0, 0, 128, 128, 128, 255, 255, 255],       // horizontal edge → strong Gy
        [0, 128, 255, 0, 128, 255, 0, 128, 255],       // vertical edge → strong Gx
        [10, 20, 30, 40, 50, 60, 70, 80, 90],          // ramp
        [255, 0, 0, 0, 0, 0, 0, 0, 0],                 // corner spike
        [17, 240, 3, 199, 88, 46, 130, 7, 251],        // noisy
        [50, 100, 150, 50, 100, 150, 50, 100, 150],    // vertical gradient
    ]
}

#[test]
fn sobel_residual_lowers_to_a_fabric_circuit_eq_vm() {
    let (ops, result, n_in) = residual();
    assert_eq!(n_in, 9, "nine pixel inputs");
    assert!(
        ops.iter().any(|o| matches!(o.op, AluOp::Xor)),
        "abs uses a Xor tile"
    );
    assert!(
        ops.iter().any(|o| matches!(o.op, AluOp::Shr)),
        "abs uses an arithmetic Shr tile"
    );
    for w in windows() {
        assert_eq!(
            eval_fabric(&ops, &w, result),
            vm_oracle(&w),
            "sobel circuit != VM on {w:?}"
        );
    }
}

#[test]
fn sobel_places_on_the_spatial_grid_eq_vm() {
    let (ops, result, n_in) = residual();
    let g = route_leveled(&ops, n_in, result);
    for w in windows() {
        assert_eq!(
            eval_grid(&g, &w),
            vm_oracle(&w),
            "placed sobel grid != VM on {w:?}"
        );
    }
    let (rows, cols, used) = grid_area(&g);
    eprintln!("sobel3 placed on a {rows}×{cols} tile array, {used} tiles used");
}

#[test]
fn sobel_rtl_matches_vm_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (ops, result, n_in) = residual();
    let module = emit_fabric_verilog("sobel3", n_in, &ops, result);
    let ws = windows();
    let mut tb = String::from("module tb;\n  reg [63:0] ");
    tb.push_str(
        &(0..n_in)
            .map(|k| format!("in{k}"))
            .collect::<Vec<_>>()
            .join(","),
    );
    tb.push_str(";\n  wire [63:0] result;\n  fabric_sobel3 dut(");
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
    let dir = std::env::temp_dir().join("logos_fabric_sobel_iverilog");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let modv = dir.join("sobel3.v");
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
        assert_eq!(*g, vm_oracle(w), "RTL sobel ≠ VM on {w:?} (rtl {g})");
    }
}
