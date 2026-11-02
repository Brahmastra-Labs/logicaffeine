//! emit_matmul_array — AUTO-EMIT a P-wide spatial int8 MAC array as synthesizable RTL, FROM LOGOS.
//!
//!   cargo run -q -p logicaffeine-compile --example emit_matmul_array -- <P> [out_dir]
//!
//! This is the generalization that matters: the fabric RTL is NOT hand-written — you
//! write the matmul in LOGOS and the compiler emits the spatial array (`loop_fabric_from_source` →
//! `emit_loop_fabric_verilog`). Widen P to place more multiply tiles (P MACs/clock). The emitted files feed
//! `scripts/synth-fabric-matmul.sh` (yosys → nextpnr → bitstream). Point the same path at any model's matmul.
use std::fs;

use logicaffeine_compile::codegen_sva::fabric_pipe::{dot_unrolled_source, matmul_array_top, GEMM_TERM_BUDGET};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    emit_loop_fabric_verilog, emit_loop_fabric_verilog_int8, emit_loop_fabric_verilog_pipelined, eval_loop_fabric,
    loop_fabric_from_source, AluOp, LoopCell,
};

fn main() {
    let mut args = std::env::args().skip(1);
    let p: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(8);
    let dir = args
        .next()
        .unwrap_or_else(|| format!("/tmp/fabric_matmul/r{p}"));
    fs::create_dir_all(&dir).unwrap();

    // ── 1. the LOGOS source (this is what a bringup engineer writes — English, not RTL) ──
    let src = dot_unrolled_source(p);
    fs::write(format!("{dir}/dotU.lg"), &src).unwrap();

    // ── 2. AUTO-LOWER to the spatial fabric + EMIT synthesizable RTL (no hand-written Verilog) ──
    let lf = loop_fabric_from_source(&src, "dotU");
    let muls = lf
        .body
        .iter()
        .filter(|c| matches!(c, LoopCell::Arith { op: AluOp::Mul, .. }))
        .count();
    let name = format!("arr{p}");
    // MUL_BITS=<n> selects the INT8-NATIVE datapath (n-bit signed multiply operands → 1 DSP48E1 per MAC);
    // unset = the 64-bit datapath (~10 DSPs/MAC). The int8 form is byte-identical for int8-range operands.
    let mul_bits: Option<u32> = std::env::var("MUL_BITS").ok().and_then(|s| s.parse().ok());
    // PIPELINED=1 emits the pipelined form: products a stage ahead and a carry-save accumulator.
    let pipelined = std::env::var("PIPELINED").is_ok_and(|v| v == "1");
    let rtl = match (pipelined, mul_bits) {
        (true, w) => {
            emit_loop_fabric_verilog_pipelined(&name, &lf, w, GEMM_TERM_BUDGET).expect("the dot loop is an accumulation").verilog
        }
        (false, Some(w)) => emit_loop_fabric_verilog_int8(&name, &lf, w),
        (false, None) => emit_loop_fabric_verilog(&name, &lf),
    };
    let dp = mul_bits.map_or("64-bit".to_string(), |w| {
        format!("int8-native ({w}-bit mul → 1 DSP/MAC)")
    });

    // ── 3. a pin-light synthesizable TOP so P&R measures the ARRAY's critical path, not IO ──
    //     (registers a serial input into the length reg, latches the result, XORs to a serial out pin.)
    let top = matmul_array_top(&name);
    fs::write(format!("{dir}/matmul_array.v"), format!("{rtl}\n{top}")).unwrap();

    // ── 4. XDC (clock + a few pins) for the Nexys A7 (xc7a100t), 100 MHz target ──
    let xdc = "set_property -dict { PACKAGE_PIN E3  IOSTANDARD LVCMOS33 } [get_ports clk]\n\
               create_clock -add -name sys_clk -period 10.00 -waveform {0 5} [get_ports clk]\n\
               set_property -dict { PACKAGE_PIN J15 IOSTANDARD LVCMOS33 } [get_ports rst]\n\
               set_property -dict { PACKAGE_PIN C4  IOSTANDARD LVCMOS33 } [get_ports serin]\n\
               set_property -dict { PACKAGE_PIN D4  IOSTANDARD LVCMOS33 } [get_ports serout]\n";
    fs::write(format!("{dir}/matmul_array.xdc"), xdc).unwrap();

    // ── 5. a self-checking testbench (iverilog) so the emitted hardware is verified before synth ──
    let tb = format!(
        "`timescale 1ns/1ps\nmodule tb;\n  reg clk=0, rst; reg [63:0] in0; wire [63:0] result; wire done;\n  integer c;\n\
        \x20 cgra_{name} dut(.clk(clk), .rst(rst), .in0(in0), .result(result), .done(done));\n\
        \x20 always #5 clk=~clk;\n  initial begin\n    in0=64'd0;\n    if (!$value$plusargs(\"N=%d\", in0)) in0=64'd0;\n\
        \x20   @(negedge clk); rst=1; @(negedge clk); rst=0; c=0;\n\
        \x20   while (!done && c<100000) begin @(negedge clk); c=c+1; end\n\
        \x20   $display(\"RESULT: %0d CYCLES=%0d\", $signed(result), c);\n    $finish;\n  end\nendmodule\n"
    );
    fs::write(format!("{dir}/tb_matmul_array.v"), format!("{rtl}\n{tb}")).unwrap();

    // model-level self-check so the emitted artifact is never silently wrong.
    let oracle = |n: i64| {
        (1..=n)
            .map(|k| (0..p as i64).map(|j| (k + j) * (n - k - j)).sum::<i64>())
            .sum::<i64>()
    };
    let m16 = eval_loop_fabric(&lf, &[16]);
    assert_eq!(m16, oracle(16), "emitted array (model) != oracle");

    println!("EMITTED a {p}-wide spatial int8 MAC array FROM LOGOS ({muls} multiply tiles = {p} MACs/clock) [datapath: {dp}]:");
    println!("  LOGOS source : {dir}/dotU.lg");
    println!("  array RTL    : {dir}/matmul_array.v   (module cgra_{name} + matmul_array_top)");
    println!("  constraints  : {dir}/matmul_array.xdc (xc7a100t, 100 MHz target)");
    println!("  testbench    : {dir}/tb_matmul_array.v");
    println!("  model check  : dotU(16) = {m16}  ≡ oracle  ✓");
    println!("→ synth + P&R: bash scripts/synth-fabric-matmul.sh {p}");
}
