//! emit_decision_fabric — lower the HFT DECISION to a combinational select-DAG circuit and write the
//! tape-ready artifacts into benchmarks/silicon/:
//!   * decision_hft.v      — the combinational module (every `If` a predicated Select tile, no clock)
//!   * decision_hft_seq.v  — a registered wrapper so the RTL-to-GDSII flow can time the datapath
//!   * decision_in.hex / decision_exp.hex — N vectors (inputs + expected, from `eval_decision_fabric`,
//!     which `hft_decision_fabric_lock` proves ≡ the running VM)
//!   * tb_decision_hft.v   — self-checking testbench (RTL == fabric == VM)
//!
//! The decision (given the microprice) is pure compare/arith/branch — no multiply, no divide — so it
//! lowers to a small, fast, single-cycle datapath. Run: `cargo run -p logicaffeine-compile --example
//! emit_decision_fabric`.

use logicaffeine_compile::codegen_sva::decision_workload::{DECIDE_FN, DECIDE_NAME};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, emit_decision_fabric_verilog, eval_decision_fabric,
};
use std::io::Write;

fn main() {
    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    let df = decision_fabric_from_source(DECIDE_FN, DECIDE_NAME);
    let n_in = df.n_in;

    // 1. the combinational circuit
    let comb = emit_decision_fabric_verilog("hft", &df);
    write(&out_dir.join("decision_hft.v"), &comb);

    // 2. registered wrapper (input + output flops) so STA reports a clock — the reg→reg path IS the
    //    single-cycle latency of the fetch/decode-free decision.
    let ins = (0..n_in)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let inr = (0..n_in)
        .map(|k| format!("  reg [63:0] in{k}_r;"))
        .collect::<Vec<_>>()
        .join("\n");
    let conn = (0..n_in)
        .map(|k| format!(".in{k}(in{k}_r)"))
        .collect::<Vec<_>>()
        .join(", ");
    let latch = (0..n_in)
        .map(|k| format!("    in{k}_r <= in{k};"))
        .collect::<Vec<_>>()
        .join("\n");
    let wrapper = format!(
        "// decision_hft_seq — the combinational decision (decision_hft) in input/output registers so\n\
         // the RTL-to-GDSII flow can TIME it. The reg-to-reg path is the decision's single-cycle latency:\n\
         // no PC, no fetch, no dispatch, no loop counter — the whole branchy decision settles in ONE clock.\n\
         module decision_hft_seq(input clk, {ins}, output reg [63:0] result);\n\
         {inr}\n\
         \x20 wire [63:0] y;\n\
         \x20 decision_hft u_dec({conn}, .result(y));\n\
         \x20 always @(posedge clk) begin\n\
         {latch}\n\
         \x20   result <= y;\n\
         \x20 end\n\
         endmodule\n"
    );
    write(&out_dir.join("decision_hft_seq.v"), &wrapper);

    // 3. vectors: realistic + adversarial market states; expected via eval_decision_fabric (≡ VM).
    let n = 256usize;
    let mut state: u64 = 0xD_EC_151_0F_u64;
    let mut next = |m: u64| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) % m
    };
    let mut inhex = String::new();
    let mut exphex = String::new();
    for _ in 0..n {
        let p: [i64; 8] = [
            10000 + next(120) as i64, // micro100 straddling the fair±edge band
            10050,                    // fair100
            next(20) as i64,          // edge100
            1 + next(80) as i64,      // base_size (past max_order → clamp)
            10 + next(60) as i64,     // max_order
            next(260) as i64 - 130,   // inventory (straddles ±pos_cap)
            100,                      // pos_cap
            next(2) as i64,           // book_ok
        ];
        for v in p.iter() {
            inhex.push_str(&format!("{:016x}\n", *v as u64));
        }
        exphex.push_str(&format!("{:016x}\n", eval_decision_fabric(&df, &p) as u64));
    }
    write(&out_dir.join("decision_in.hex"), &inhex);
    write(&out_dir.join("decision_exp.hex"), &exphex);

    // 4. self-checking testbench: RTL == fabric(==VM).
    let port = (0..n_in)
        .map(|k| format!(".in{k}(iw[{k}])"))
        .collect::<Vec<_>>()
        .join(", ");
    let tb = format!(
        "// tb_decision_hft — the emitted decision circuit vs the fabric oracle (== the running VM,\n\
         // proven by hft_decision_fabric_lock). {n} market states.\n\
         `timescale 1ns/1ps\n\
         module tb_decision_hft;\n\
         \x20 reg  [63:0] mem [0:{inlast}];\n\
         \x20 reg  [63:0] exp [0:{elast}];\n\
         \x20 reg  [63:0] iw [0:{ninlast}];\n\
         \x20 wire [63:0] result;\n\
         \x20 integer i, j, errors = 0;\n\
         \x20 decision_hft dut({port}, .result(result));\n\
         \x20 initial begin\n\
         \x20   $readmemh(\"decision_in.hex\", mem);\n\
         \x20   $readmemh(\"decision_exp.hex\", exp);\n\
         \x20   for (i = 0; i < {n}; i = i + 1) begin\n\
         \x20     for (j = 0; j < {n_in}; j = j + 1) iw[j] = mem[i*{n_in}+j];\n\
         \x20     #1;\n\
         \x20     if (result !== exp[i]) begin errors = errors + 1; $display(\"FAIL vec %0d: got %0d exp %0d\", i, $signed(result), $signed(exp[i])); end\n\
         \x20   end\n\
         \x20   if (errors == 0) $display(\"PASS: {n} decision evaluations match (RTL == fabric == VM)\");\n\
         \x20   else $display(\"FAIL: %0d mismatches\", errors);\n\
         \x20   $finish;\n\
         \x20 end\n\
         endmodule\n",
        inlast = n * n_in - 1,
        elast = n - 1,
        ninlast = n_in - 1,
    );
    write(&out_dir.join("tb_decision_hft.v"), &tb);

    let selects = df
        .cells
        .iter()
        .filter(|c| {
            matches!(
                c,
                logicaffeine_compile::codegen_sva::fabric_rtl::LoopCell::Select { .. }
            )
        })
        .count();
    let arith = df.cells.len() - selects;
    println!(
        "decision fabric: {n_in} inputs, {} cells ({selects} Select tiles + {arith} arith), {n} vectors",
        df.cells.len()
    );
    println!("wrote decision_hft.v, decision_hft_seq.v, tb_decision_hft.v, decision_{{in,exp}}.hex → benchmarks/silicon/");
}

fn write(path: &std::path::Path, s: &str) {
    let mut f = std::fs::File::create(path).unwrap_or_else(|e| panic!("create {path:?}: {e}"));
    f.write_all(s.as_bytes()).unwrap();
}
