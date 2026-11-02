//! emit_systolic_tile — emit an 8×8 weight-stationary systolic GEMM tile (Phase 1) + a self-contained BIST
//! harness (on-chip LFSR stimulus + MISR signature, only clk/rst/sig cross the boundary) so it routes on
//! Sky130 with no pad distortion. Feeds `scripts/openlane-run.sh systolic_tile_realbist clk 15 ...` +
//! `scripts/measure-power-vcd.sh systolic_tile_realbist` — the weight-stationary energy/MAC measurement.
//!
//!   cargo run -q -p logicaffeine-compile --example emit_systolic_tile
use std::fs;

use logicaffeine_compile::codegen_sva::systolic::{emit_systolic_verilog, Systolic};

fn main() {
    let (k, n) = (8usize, 8usize);
    // Deterministic int8 weights (a fixed model's baked GEMM block).
    let weights: Vec<i64> = (0..k * n)
        .map(|i| {
            let mut x = (i as u64).wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(1);
            x ^= x >> 29;
            ((x % 255) as i64) - 127
        })
        .collect();
    let sys = Systolic::new(k, n, weights);
    let array = emit_systolic_verilog("systolic8", &sys);

    // BIST harness: LFSR drives the 8 int8 activation lanes; the 8 registered outputs fold into a MISR.
    let a_wires: String = (0..k)
        .map(|r| {
            format!(
                "  wire signed [63:0] a{r} = $signed(lfsr[{}:{}]);\n",
                r * 8 + 7,
                r * 8
            )
        })
        .collect();
    let o_wires: String = (0..n)
        .map(|c| format!("  wire signed [63:0] o{c};\n"))
        .collect();
    let aconn: Vec<String> = (0..k).map(|r| format!(".a{r}(a{r})")).collect();
    let oconn: Vec<String> = (0..n).map(|c| format!(".o{c}(o{c})")).collect();
    let ofold: String = (0..n)
        .map(|c| format!("o{c}"))
        .collect::<Vec<_>>()
        .join(" ^ ");
    let wrapper = format!(
        "module systolic_tile_realbist(input clk, input rst, output sig);\n\
         \x20 reg [63:0] lfsr;\n  reg [63:0] misr;\n{a_wires}{o_wires}\
         \x20 systolic8 arr(.clk(clk), .rst(rst), {aconn}, {oconn});\n\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin\n      lfsr <= 64'hD1CEF00D12345678;\n      misr <= 64'd0;\n    end else begin\n\
         \x20     lfsr <= {{lfsr[62:0], lfsr[63]^lfsr[62]^lfsr[60]^lfsr[59]}};\n\
         \x20     misr <= {{misr[62:0], misr[63]}} ^ ({ofold});\n    end\n  end\n\
         \x20 assign sig = ^misr;\nendmodule\n",
        aconn = aconn.join(", "),
        oconn = oconn.join(", "),
    );

    let sil = "benchmarks/silicon";
    fs::write(format!("{sil}/systolic8.v"), &array).unwrap();
    fs::write(format!("{sil}/systolic_tile_realbist.v"), &wrapper).unwrap();
    println!("emitted {sil}/systolic8.v (8x8 weight-stationary tile) + systolic_tile_realbist.v (BIST harness)");
    println!("tape:  OUT_JSON={sil}/systolic_signoff.json scripts/openlane-run.sh systolic_tile_realbist clk 15 systolic_tile_realbist.v systolic8.v");
    println!("power: scripts/measure-power-vcd.sh systolic_tile_realbist");
}
