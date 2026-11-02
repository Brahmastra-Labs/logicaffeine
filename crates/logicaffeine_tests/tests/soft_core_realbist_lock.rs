//! soft_core_realbist_lock — the CPU twin of `decision_hft_realbist`: the routable scalar LOGOS core
//! running the SAME HFT decision over the SAME realistic input ranges, so a CPU and the decision fabric
//! can be compared on one basis — each block's gate-level switching power over a window of realistic
//! decisions, times the clocks it spends per decision.
//!
//! WHY. The comparison the silicon ledgers carried divided a soft-core power taken at the tool's
//! DEFAULT switching activity (on a core whose ROM held an 11-word counting loop) by the fabric's
//! VCD-MEASURED power, and multiplied by a cycle count measured on a different core (the full
//! `soft_core.v`). Measured activity on the fabric came out 4.9x below the default assumption, so the
//! mixed basis could inflate the gap by the same factor. A CPU number is only comparable when it is
//! measured the way the fabric's is: a hardened, signoff-clean netlist, simulated at gate level at its
//! signoff clock, running the decision on realistic inputs.
//!
//! What this file locks, before any silicon is built from it:
//!   1. the decision the CPU runs IS the fabric's decision — the shared source, inlined mechanically,
//!      agrees with the called function on the VM and with an independent Rust reference;
//!   2. the program's stimulus draws exactly the realbist's input ranges;
//!   3. the program is chip-runnable: it encodes to the function-free scalar subset and fits the ROM;
//!   4. on the VM its Show stream is the reference's, decision for decision;
//!   5. under iverilog, `soft_core_scalar.v` running it as its boot ROM shows the same stream;
//!   6. the block artifacts in `benchmarks/silicon/` are exactly what the emitter produces.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::decision_workload::{
    inline_call, soft_core_realbist_program, soft_core_realbist_source, soft_core_realbist_verilog,
    DECIDE_FN, DECIDE_NAME, LCG_A, LCG_C, LCG_MASK, SOFT_CORE_REALBIST_ITERATIONS,
};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, eval_decision_fabric,
};
use logicaffeine_compile::codegen_sva::scalar_asm::{compile_to_scalar_words, to_hex};
use logicaffeine_compile::compile::vm_outcome;
use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// The HFT decision, written independently of the LOGOS source: +buy / -sell / 0.
#[allow(clippy::too_many_arguments)]
fn decide_ref(
    micro: i64,
    fair: i64,
    edge: i64,
    base: i64,
    max_order: i64,
    inventory: i64,
    pos_cap: i64,
    book_ok: i64,
) -> (i64, i64) {
    let (mut side, mut size) = (0, 0);
    if book_ok == 1 {
        if micro > fair + edge {
            side = 2;
            size = base;
        } else if micro < fair - edge {
            side = 1;
            size = base;
        }
    }
    size = size.min(max_order);
    let signed = match side {
        1 => size,
        2 => -size,
        _ => 0,
    };
    let projected = inventory + signed;
    let risk_ok = projected <= pos_cap && projected >= -pos_cap;
    (if risk_ok { signed } else { 0 }, side)
}

/// The realbist's input ranges, drawn from three 15-bit LCG outputs.
struct Inputs {
    micro: i64,
    edge: i64,
    base: i64,
    max_order: i64,
    inventory: i64,
    book_ok: i64,
}

fn lcg(s: i64) -> i64 {
    (s * LCG_A + LCG_C) & LCG_MASK
}

fn draw(s: &mut i64) -> Inputs {
    let r1 = lcg(*s);
    let r2 = lcg(r1);
    let r3 = lcg(r2);
    *s = r3;
    Inputs {
        micro: 10000 + (r1 >> 8),
        edge: (r1 >> 4) & 15,
        base: 1 + (r2 >> 9),
        max_order: 10 + ((r2 >> 4) & 31),
        inventory: (r3 >> 7) - 128,
        book_ok: (r3 >> 6) & 1,
    }
}

/// The reference Show stream: one value per decision, `signed_order * 4 + side`.
fn reference(iterations: usize) -> Vec<i64> {
    let mut s = 1;
    (0..iterations)
        .map(|_| {
            let x = draw(&mut s);
            let (d, side) = decide_ref(
                x.micro,
                10050,
                x.edge,
                x.base,
                x.max_order,
                x.inventory,
                100,
                x.book_ok,
            );
            d * 4 + side
        })
        .collect()
}

fn vm_shows(src: &str) -> Vec<i64> {
    let out = vm_outcome(src);
    assert!(out.error.is_none(), "VM error: {:?}", out.error);
    out.output
        .split_whitespace()
        .map(|v| v.parse().expect("an integer Show"))
        .collect()
}

#[test]
fn the_inlined_decision_is_the_called_decision() {
    let args = ["m", "f", "e", "b", "mo", "inv", "cap", "ok"];
    let mut s = 7;
    for _ in 0..64 {
        let x = draw(&mut s);
        let vals = [
            x.micro,
            10050,
            x.edge,
            x.base,
            x.max_order,
            x.inventory,
            100,
            x.book_ok,
        ];
        let lets: String = args
            .iter()
            .zip(vals)
            .map(|(a, v)| format!("    Let {a} be {v}.\n"))
            .collect();
        let called = format!(
            "{DECIDE_FN}## Main\n{lets}    Show {DECIDE_NAME}({}).\n",
            args.join(", ")
        );
        let inline = format!(
            "## Main\n{lets}    Let mutable out be 0.\n{}    Show out.\n",
            inline_call(DECIDE_FN, &args, "out", 4).expect("the decision inlines")
        );
        let want = decide_ref(
            vals[0], vals[1], vals[2], vals[3], vals[4], vals[5], vals[6], vals[7],
        )
        .0;
        assert_eq!(
            vm_shows(&called),
            vec![want],
            "the called decision is not the reference at {vals:?}"
        );
        assert_eq!(
            vm_shows(&inline),
            vec![want],
            "the inlined decision is not the called one at {vals:?}"
        );
    }
}

/// The fabric's lowering of the same source is the same decision: the CPU twin and the decision fabric
/// run one decision, not two that happen to share a name.
#[test]
fn the_fabric_lowers_the_same_decision_the_cpu_runs() {
    let df = decision_fabric_from_source(DECIDE_FN, DECIDE_NAME);
    let mut s = 11;
    for _ in 0..4096 {
        let x = draw(&mut s);
        let vals = [
            x.micro,
            10050,
            x.edge,
            x.base,
            x.max_order,
            x.inventory,
            100,
            x.book_ok,
        ];
        let want = decide_ref(
            vals[0], vals[1], vals[2], vals[3], vals[4], vals[5], vals[6], vals[7],
        )
        .0;
        assert_eq!(
            eval_decision_fabric(&df, &vals),
            want,
            "the fabric's decision is not the CPU's at {vals:?}"
        );
    }
}

#[test]
fn the_inliner_refuses_what_it_cannot_inline_faithfully() {
    let early =
        "## To f (x: Int) -> Int:\n    If x is greater than 0:\n        Return 1.\n    Return 0.\n";
    assert!(
        inline_call(early, &["a"], "r", 4).is_err(),
        "a Return before the last line cannot be inlined"
    );
    assert!(
        inline_call(DECIDE_FN, &["a"], "r", 4).is_err(),
        "an argument count that is not the arity is refused"
    );
}

#[test]
fn the_stimulus_spans_exactly_the_realbist_ranges() {
    let mut s = 1;
    let (mut lo, mut hi) = ([i64::MAX; 6], [i64::MIN; 6]);
    for _ in 0..32768 {
        let x = draw(&mut s);
        for (k, v) in [x.micro, x.edge, x.base, x.max_order, x.inventory, x.book_ok]
            .into_iter()
            .enumerate()
        {
            lo[k] = lo[k].min(v);
            hi[k] = hi[k].max(v);
        }
    }
    assert_eq!(
        lo,
        [10000, 0, 1, 10, -128, 0],
        "every input reaches the bottom of its realbist range"
    );
    assert_eq!(
        hi,
        [10127, 15, 64, 41, 127, 1],
        "every input reaches the top of its realbist range"
    );
    let mix = reference(4000);
    for (name, pred) in [
        ("buy", 0),
        ("sell", 1),
        ("risk-blocked", 2),
        ("no-trade", 3),
    ] {
        let n = mix
            .iter()
            .filter(|&&v| match pred {
                0 => v >= 4,
                1 => v < 0,
                2 => v == 1 || v == 2,
                _ => v == 0,
            })
            .count();
        assert!(n > 0, "the stimulus never makes a {name} decision");
    }
}

#[test]
fn the_program_runs_on_the_scalar_core_and_fits_its_rom() {
    let words = compile_to_scalar_words(&soft_core_realbist_source(SOFT_CORE_REALBIST_ITERATIONS))
        .unwrap_or_else(|e| panic!("the CPU twin's program is outside the scalar subset: {e}"));
    assert!(
        words.len() <= 256,
        "{} words exceed the 256-word boot ROM",
        words.len()
    );
    assert_eq!(
        words,
        soft_core_realbist_program(),
        "the emitted program is the compiled source"
    );
}

#[test]
fn on_the_vm_the_program_is_the_reference_decision_for_decision() {
    let n = 1500;
    assert_eq!(
        vm_shows(&soft_core_realbist_source(n)),
        reference(n as usize)
    );
}

#[test]
fn the_scalar_core_rtl_runs_it_as_its_boot_rom() {
    if !common::require_tool("iverilog") {
        return;
    }
    let n: usize = 300;
    let dir = std::env::temp_dir().join(format!("logos_soft_core_realbist_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let words = compile_to_scalar_words(&soft_core_realbist_source(n as u32)).unwrap();
    std::fs::write(dir.join("program.hex"), to_hex(&words)).unwrap();
    std::fs::write(dir.join("tb.v"), format!("`timescale 1ns/1ps
module tb;
  reg clk = 0, rst = 1;
  wire [31:0] out; wire out_valid, halted;
  soft_core_realbist dut(.clk(clk), .rst(rst), .out(out), .out_valid(out_valid), .halted(halted));
  always #5 clk = ~clk;
  integer shows = 0;
  always @(posedge clk) if (out_valid) begin $display(\"SHOW %0d\", $signed(out)); shows = shows + 1; end
  initial begin
    repeat (3) @(posedge clk); @(negedge clk) rst = 0;
    wait (halted); repeat (2) @(posedge clk);
    $display(\"HALTED %0d\", shows); $finish;
  end
  initial begin #{} $display(\"TIMEOUT\"); $finish; end
endmodule
", n * 400 * 10)).unwrap();
    let sil = root().join("benchmarks/silicon");
    let build = Command::new("iverilog")
        .current_dir(&dir)
        .args(["-g2012", "-o", "sim.vvp", "tb.v"])
        .arg(sil.join("soft_core_realbist.v"))
        .arg(sil.join("soft_core_scalar.v"))
        .arg(sil.join("alu.v"))
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "iverilog: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new("vvp")
        .current_dir(&dir)
        .arg("sim.vvp")
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&run.stdout);
    let got: Vec<i64> = out
        .lines()
        .filter_map(|l| l.strip_prefix("SHOW "))
        .map(|v| v.parse().unwrap())
        .collect();
    assert!(
        out.contains("HALTED"),
        "the core never halted:\n{}",
        &out[out.len().saturating_sub(400)..]
    );
    assert_eq!(
        got,
        reference(n),
        "the scalar core RTL diverged from the reference"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_block_artifacts_are_the_emitted_ones() {
    let sil = root().join("benchmarks/silicon");
    let rtl = std::fs::read_to_string(sil.join("soft_core_realbist.v")).expect(
        "benchmarks/silicon/soft_core_realbist.v — run the emit_soft_core_realbist example",
    );
    assert_eq!(
        rtl,
        soft_core_realbist_verilog(),
        "soft_core_realbist.v is not what the emitter writes"
    );
    let hex = std::fs::read_to_string(sil.join("soft_core_realbist.hex")).expect(
        "benchmarks/silicon/soft_core_realbist.hex — run the emit_soft_core_realbist example",
    );
    assert_eq!(
        hex,
        to_hex(&soft_core_realbist_program()),
        "soft_core_realbist.hex is not the compiled program"
    );
}
