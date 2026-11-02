//! loop_pipeline_lock — the counted-loop CGRA with its accumulators in carry-save form and their terms a stage ahead.
//!
//! The GEMM array's clock was eight chained 64-bit accumulate adds behind eight int8 multiplies. The pipelined loop
//! form computes the terms from the induction counter in stages ahead, absorbs them into each accumulator's sum and
//! carry words through 3:2 compressors, and increments the counter through a parallel-prefix AND, so no recurrence
//! carries a carry chain. This lock holds it to the form it replaces:
//!
//! - A: on dot products of several widths, both datapaths, every loop bound tried and every term budget, the
//!   pipelined CGRA returns the original CGRA's result under iverilog, exactly as many clocks later as it has term
//!   stages whenever the body runs; on the 64-bit datapath both are the model's.
//! - B: accumulations that subtract, add constants or carry two accumulators do the same.
//! - C: a loop that is not a pure accumulation is refused by name, never emitted wrong.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test loop_pipeline_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::fabric_pipe::{
    dot_unrolled_source, GEMM_MUL_BITS, GEMM_TERM_BUDGET,
};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    emit_loop_fabric_verilog, emit_loop_fabric_verilog_int8, emit_loop_fabric_verilog_pipelined, eval_loop_fabric,
    loop_fabric_from_source, LoopFabric,
};

/// Run `cgra_{module}` for each bound in `bounds`: reset one clock with the bound presented, then count clocks to
/// `done`. Returns (result, clocks) per bound.
fn run_loop(verilog: &str, module: &str, n_in: usize, bounds: &[Vec<i64>]) -> Vec<(i64, u64)> {
    let dir = tempfile::tempdir().expect("temp dir");
    let regs = (0..n_in).map(|k| format!("reg [63:0] in{k};\n  ")).collect::<String>();
    let ports = (0..n_in).map(|k| format!(".in{k}(in{k}), ")).collect::<String>();
    let mut drive = String::new();
    for b in bounds {
        let set = (0..n_in).map(|k| format!("in{k} = {};", b[k])).collect::<Vec<_>>().join(" ");
        drive.push_str(&format!(
            "    {set} rst = 1; #1 clk = 1; #1 clk = 0; rst = 0; c = 0;\n    \
             while (!done && c < 100000) begin #1 clk = 1; #1 clk = 0; c = c + 1; end\n    \
             $display(\"R %0d %0d\", $signed(result), c);\n"
        ));
    }
    let tb = format!(
        "module tb;\n  reg clk = 0, rst = 1;\n  {regs}wire [63:0] result; wire done; integer c;\n  \
         cgra_{module} dut(.clk(clk), .rst(rst), {ports}.result(result), .done(done));\n  initial begin\n{drive}    \
         $finish;\n  end\nendmodule\n"
    );
    std::fs::write(dir.path().join("dut.v"), verilog).unwrap();
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let build = std::process::Command::new("iverilog")
        .current_dir(dir.path())
        .args(["-g2012", "-o", "sim.vvp", "tb.v", "dut.v"])
        .output()
        .expect("run iverilog");
    assert!(build.status.success(), "iverilog failed: {}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new("vvp").current_dir(dir.path()).arg("sim.vvp").output().expect("vvp");
    let got: Vec<(i64, u64)> = String::from_utf8_lossy(&run.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("R "))
        .map(|l| {
            let mut it = l.split_whitespace();
            (it.next().unwrap().parse().unwrap(), it.next().unwrap().parse().unwrap())
        })
        .collect();
    assert_eq!(got.len(), bounds.len(), "one result per bound:\n{}", String::from_utf8_lossy(&run.stdout));
    got
}

fn original(name: &str, lf: &LoopFabric, mul_bits: Option<u32>) -> String {
    match mul_bits {
        Some(w) => emit_loop_fabric_verilog_int8(name, lf, w),
        None => emit_loop_fabric_verilog(name, lf),
    }
}

/// The term budgets tried: one stage for all the terms, the GEMM's two, and a tile per stage.
const BUDGETS: [f64; 3] = [1e9, GEMM_TERM_BUDGET, 0.5];

/// The pipelined CGRA, at every budget, returns the original's result on every bound, as many clocks after it as it
/// has term stages whenever the body runs; on the 64-bit datapath both are the model's.
fn same_as_original(src: &str, f: &str, bounds: &[Vec<i64>], mul_bits: Option<u32>) {
    let lf = loop_fabric_from_source(src, f);
    let was = run_loop(&original("p", &lf, mul_bits), "p", lf.n_in, bounds);
    let mut stage_counts = Vec::new();
    for budget in BUDGETS {
        let pipelined = emit_loop_fabric_verilog_pipelined("p", &lf, mul_bits, budget)
            .unwrap_or_else(|why| panic!("{f} is an accumulation, refused: {why}"));
        stage_counts.push(pipelined.term_stages);
        let now = run_loop(&pipelined.verilog, "p", lf.n_in, bounds);
        for (i, b) in bounds.iter().enumerate() {
            assert_eq!(now[i].0, was[i].0, "{f}{b:?} ({mul_bits:?}, budget {budget}): another result");
            let drain = if was[i].1 > 0 { pipelined.term_stages as u64 } else { 0 };
            assert_eq!(
                now[i].1,
                was[i].1 + drain,
                "{f}{b:?} ({mul_bits:?}, budget {budget}): not {} clock(s) longer whenever its body runs",
                pipelined.term_stages
            );
            if mul_bits.is_none() {
                assert_eq!(now[i].0, eval_loop_fabric(&lf, b), "{f}{b:?}: not the model's result");
            }
        }
    }
    assert_eq!(stage_counts[0], 1, "{f}: an unbounded budget keeps every term in one stage");
    assert!(stage_counts.windows(2).all(|w| w[0] <= w[1]), "{f}: a tighter budget never shortens the term pipeline");
}

// ── A. dot products ─────────────────────────────────────────────────────────────────────────────────

#[test]
fn dot_products_of_every_width_return_the_original_result_after_their_term_stages() {
    if !common::require_tool("iverilog") {
        return;
    }
    let bounds: Vec<Vec<i64>> = [0i64, 1, 2, 3, 7, 16, 40, 100, 127].iter().map(|&n| vec![n]).collect();
    for u in [1usize, 2, 3, 5, 8] {
        for mul_bits in [None, Some(GEMM_MUL_BITS)] {
            same_as_original(&dot_unrolled_source(u), "dotU", &bounds, mul_bits);
        }
    }
}

// ── B. other accumulations ──────────────────────────────────────────────────────────────────────────

#[test]
fn accumulations_that_subtract_add_constants_or_carry_two_registers_return_the_original_result() {
    if !common::require_tool("iverilog") {
        return;
    }
    let bounds: Vec<Vec<i64>> = [0i64, 1, 4, 33, 200].iter().map(|&n| vec![n]).collect();
    let cases = [
        "## To f (n: Int) -> Int:\n    Let mutable s be 5.\n    Repeat for k from 1 to n:\n        Set s to s - k * 3 + 7.\n    Return s.\n",
        "## To f (n: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for k from 1 to n:\n        Set s to s + 1.\n    Return s.\n",
        "## To f (n: Int) -> Int:\n    Let mutable s be 0.\n    Repeat for k from 1 to n:\n        Set s to s + k * k - (n - k).\n    Return s.\n",
        "## To f (n: Int) -> Int:\n    Let mutable a be 0.\n    Let mutable b be 1.\n    Repeat for k from 1 to n:\n        Set a to a + k.\n        Set b to b + k * k * k.\n    Return b.\n",
    ];
    for src in cases {
        same_as_original(src, "f", &bounds, None);
    }
}

// ── C. refusals ─────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_loop_that_is_not_a_pure_accumulation_is_refused_by_name() {
    let refused = [
        ("## To f (n: Int) -> Int:\n    Let mutable s be 1.\n    Repeat for k from 1 to n:\n        Set s to s * 2 + k.\n    Return s.\n",
         "not an accumulator"),
        ("## To f (n: Int) -> Int:\n    Let mutable a be 0.\n    Let mutable b be 1.\n    Repeat for k from 1 to n:\n        Set a to a + b.\n        Set b to b + 1.\n    Return a.\n",
         "reads a state register"),
    ];
    for (src, why) in refused {
        let lf = loop_fabric_from_source(src, "f");
        match emit_loop_fabric_verilog_pipelined("p", &lf, None, 1e9) {
            Ok(_) => panic!("a loop that is not a pure accumulation was emitted pipelined:\n{src}"),
            Err(e) => assert!(e.contains(why), "refused for the wrong reason ({e}), wanted `{why}`:\n{src}"),
        }
    }
}
