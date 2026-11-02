//! fabric_soc — Phase E: the FLASHABLE reconfigurable-fabric SoC.
//!
//! Phase D proved (our SAT) that a diverse program corpus projects onto ONE reconfigurable tile array under a
//! config word. This module makes that array real hardware: a synthesizable `fabric_soc` wraps the
//! `config_<name>` array ([`emit_config_grid_verilog`]) with registered I/O and a CYCLE COUNTER, so a host
//! presents `(cfg, inputs)` and reads back `(result, cycles)`. Because the array is feed-forward
//! (combinational), a decision settles in ONE clock — the honest cycles/op the interpreter pays hundreds of
//! fetch-decode-execute cycles for. The self-checking testbench drives the SoC over a vector battery and
//! `$display`s `(result, cycles)`; `fabric_soc_lock` runs it under iverilog and checks every result against
//! the VM and every cycle count against the 1-clock settle. The RTL, its testbench, and the constraints are
//! drift-locked (regenerate with `FABRIC_SOC_RECORD=1`), and the config-grid ≡ VM is the eval side of
//! `fabric_whole_program_lock` (Phase D).

use super::fabric_config::{compile_to_config_table, ConfigTable};
use super::fabric_grid::emit_config_grid_verilog;
use super::fabric_rtl::{
    decision_fabric_from_source, emit_float_datapath_verilog, emit_state_plane_verilog,
    STATE_PLANE_HEAP_WORDS,
};

/// The fabric SoC's program corpus — the datapaths the ONE reconfigurable array can become, selected by the
/// config word. Single-final-`Return` + conditional-`Set` style so branches if-convert to select tiles.
pub const FABRIC_SOURCES: &[(&str, &str)] = &[
    ("## To affine (x: Int) -> Int:\n    Return 3 * x + 7.\n", "affine"),
    ("## To clamp (x: Int, lo: Int, hi: Int) -> Int:\n    Let result be x.\n    If x is less than lo:\n        Set result to lo.\n    If x is greater than hi:\n        Set result to hi.\n    Return result.\n", "clamp"),
    ("## To sign (n: Int) -> Int:\n    Let result be 0.\n    If n is less than 0:\n        Set result to 0 - 1.\n    If n is greater than 0:\n        Set result to 1.\n    Return result.\n", "sign"),
    ("## To min2 (a: Int, b: Int) -> Int:\n    Let result be b.\n    If a is less than b:\n        Set result to a.\n    Return result.\n", "min2"),
    // Stage X1 (XPU): a DIVISION datapath — a Func (functional-unit) tile on the fabric, not just Arith/Select.
    ("## To ratio (a: Int, b: Int) -> Int:\n    Return a / b.\n", "ratio"),
];

/// The datapath / config-word widths (8-bit datapath for the SAT proof; 3-bit config indexes ≤ 8 datapaths).
pub const WIDTH: u32 = 8;
pub const CFG_WIDTH: u32 = 3;
/// The SoC's input-port count — the max arity over the corpus (`clamp` has three). Functions with fewer
/// inputs ignore the extra ports (their tiles never reference them).
pub const N_IN: usize = 3;

/// The compiled config table (padded islands on one array). The single source of truth for the RTL and the
/// testbench's expected values — both derive from it, never hand-typed.
pub fn table() -> ConfigTable {
    compile_to_config_table(FABRIC_SOURCES, WIDTH, CFG_WIDTH)
}

/// The reconfigurable array RTL (`config_fabric`): ONE tile grid becoming any corpus datapath under `cfg`.
pub fn emit_config_grid() -> String {
    let t = table();
    let refs: Vec<&super::fabric_grid::Grid> = t.grids.iter().collect();
    emit_config_grid_verilog("fabric", &refs, CFG_WIDTH)
}

/// The flashable SoC: registered `(cfg, inputs)` in, the combinational `config_fabric`, a `cycles` counter,
/// and registered `(result, cycles, done)` out. On `rst` release the array's output settles in one clock; the
/// SoC latches it, sets `cycles = 1` (the honest cost of a spatial decision), and raises `done`. Synthesizable
/// as-is — the inputs wire to switches / a UART loader and the outputs to LEDs / a UART readback for bring-up.
pub fn emit_fabric_soc() -> String {
    let ins = (0..N_IN)
        .map(|k| format!("  input signed [63:0] in{k},\n"))
        .collect::<String>();
    let conns = (0..N_IN)
        .map(|k| format!(".in{k}(in{k}), "))
        .collect::<String>();
    format!(
        "// fabric_soc — a flashable wrapper around config_fabric (the reconfigurable CGRA of Phase D). Present\n\
         // `cfg` + inputs, pulse `rst`, and after ONE settle clock `result`/`cycles` are valid and `done` is\n\
         // high. `cycles == 1`: a spatial decision settles in one clock, the metric the interpreter pays\n\
         // hundreds of fetch-decode-execute cycles for. Proven ≡ the VM under iverilog by `fabric_soc_lock`.\n\
         module fabric_soc (\n\
         \x20 input        clk,\n\
         \x20 input        rst,          // active-high synchronous reset / start\n\
         \x20 input  [{cw}:0] cfg,          // the config word: which datapath the array becomes\n\
         {ins}\
         \x20 output reg signed [63:0] result,\n\
         \x20 output reg [15:0] cycles,   // clocks from start to a valid result (the honest cycles/op)\n\
         \x20 output reg        done\n\
         );\n\
         \x20 wire signed [63:0] fab_out;\n\
         \x20 config_fabric u_fab({conns}.cfg(cfg), .result(fab_out));\n\
         \x20 reg running;\n\
         \x20 always @(posedge clk) begin\n\
         \x20\x20 if (rst) begin\n\
         \x20\x20\x20 result <= 64'sd0; cycles <= 16'd0; done <= 1'b0; running <= 1'b1;\n\
         \x20\x20 end else if (running) begin\n\
         \x20\x20\x20 // the combinational array has settled; latch it. One clock = one decision.\n\
         \x20\x20\x20 result <= fab_out; cycles <= 16'd1; done <= 1'b1; running <= 1'b0;\n\
         \x20\x20 end\n\
         \x20 end\n\
         endmodule\n",
        cw = CFG_WIDTH - 1,
    )
}

/// The self-checking testbench: drive the SoC over `(cfg, inputs)` vectors and `$display` `cfg result cycles`
/// per vector. `fabric_soc_lock` parses these and checks each result ≡ the VM and each cycle count == 1.
pub fn emit_fabric_soc_tb() -> String {
    let ins = (0..N_IN)
        .map(|k| format!("  reg signed [63:0] in{k};\n"))
        .collect::<String>();
    let conns = (0..N_IN)
        .map(|k| format!(".in{k}(in{k}), "))
        .collect::<String>();
    let set0 = (0..N_IN).map(|k| format!("in{k}=0; ")).collect::<String>();
    // One task drives a vector: reset, present cfg+inputs, clock until done, print.
    format!(
        "`timescale 1ns/1ps\n\
         module tb_fabric_soc;\n\
         \x20 reg clk = 0, rst = 0;\n\
         \x20 reg [{cw}:0] cfg;\n\
         {ins}\
         \x20 wire signed [63:0] result;\n\
         \x20 wire [15:0] cycles;\n\
         \x20 wire done;\n\
         \x20 fabric_soc dut(.clk(clk), .rst(rst), .cfg(cfg), {conns}.result(result), .cycles(cycles), .done(done));\n\
         \x20 always #5 clk = ~clk;\n\
         \x20 task run(input [{cw}:0] c, input signed [63:0] a0, input signed [63:0] a1, input signed [63:0] a2);\n\
         \x20 begin\n\
         \x20\x20 @(negedge clk); cfg=c; {set0} in0=a0; in1=a1; in2=a2; rst=1;\n\
         \x20\x20 @(negedge clk); rst=0;\n\
         \x20\x20 wait (done);\n\
         \x20\x20 @(negedge clk);\n\
         \x20\x20 $display(\"%0d %0d %0d\", c, $signed(result), cycles);\n\
         \x20 end\n\
         \x20 endtask\n\
         \x20 initial begin\n\
         {vectors}\
         \x20\x20 $finish;\n\
         \x20 end\n\
         endmodule\n",
        cw = CFG_WIDTH - 1,
        vectors = tb_vectors(),
    )
}

/// The test vectors the testbench drives — a battery per datapath, spanning each function's cases. Kept in
/// sync with `fabric_soc_lock`'s expected computation (both iterate this same list).
pub fn vectors() -> Vec<(u64, [i64; 3])> {
    vec![
        (0, [0, 0, 0]),
        (0, [3, 0, 0]),
        (0, [-5, 0, 0]),
        (1, [5, 0, 10]),
        (1, [-3, 0, 10]),
        (1, [20, 0, 10]),
        (2, [-7, 0, 0]),
        (2, [0, 0, 0]),
        (2, [9, 0, 0]),
        (3, [3, 8, 0]),
        (3, [8, 3, 0]),
        // cfg 4 = ratio (a / b, truncating toward zero) — the DIVISION Func tile. Non-zero divisors.
        (4, [10, 3, 0]),
        (4, [100, 7, 0]),
        (4, [-20, 4, 0]),
    ]
}

fn tb_vectors() -> String {
    vectors()
        .iter()
        .map(|(c, a)| {
            format!(
                "  run({cw}'d{c}, {}, {}, {});\n",
                a[0],
                a[1],
                a[2],
                cw = CFG_WIDTH
            )
        })
        .collect()
}

/// Emit the BOARD-FLASHABLE top (`nexys_fabric_soc`) for the Nexys A7-100T: the reconfigurable `config_fabric`
/// array, a baked-in vector ROM, and a settle sweep that measures on the chip how many clocks each datapath
/// takes. For every vector the sweep loads a prime into the launch register, lets it settle, launches the vector
/// and captures the array's output `L` clocks later, for `L` from `LMAX - 1` down to 1, under two primes (the
/// vector's inputs inverted, and the next vector), `REPS` times. The settled value is the capture at `2·LMAX`; a
/// result is settled inside the window only when the capture at `LMAX` already equals it under both primes. The
/// streamed cycle count is the smallest `L` from which every capture equals the settled value, the worst over the
/// repetitions, and 0 when the result did not settle inside the window. Each record goes out the UART TX as
/// `(sync, cfg, result[8], cycles[2])`; a host checks each result against the VM. 100 MHz board clock, 115200
/// baud (CLKS_PER_BIT = 868). Pins in `nexys_fabric_soc.xdc`.
pub fn emit_nexys_fabric_soc() -> String {
    let vs = vectors();
    let nvec = vs.len();
    let cw = CFG_WIDTH;
    let cwm = cw - 1;
    let rw = cw + 191;
    let pad = 8 - cw;
    let word = |n: i64| format!("64'h{:016x}", n as u64);
    let mut rom = String::new();
    for (i, (c, a)) in vs.iter().enumerate() {
        rom.push_str(&format!(
            "      {i}: rom = {{{cw}'d{c}, {}, {}, {}}};\n",
            word(a[0]),
            word(a[1]),
            word(a[2]),
        ));
    }
    format!(
        "// nexys_fabric_soc — the Phase-D reconfigurable fabric on a Nexys A7-100T, measuring its own settle. A\n\
         // baked-in vector ROM steps the ONE config_fabric array through every corpus datapath. For each vector a\n\
         // settle sweep primes the launch register, launches the vector, and captures the array's output L clocks\n\
         // later, for L from LMAX-1 down to 1, under two primes (the inputs inverted; the next vector), REPS times.\n\
         // The settled value is the capture at 2*LMAX, and it counts only when the capture at LMAX already equals it.\n\
         // Each record streams out the UART TX as {{0xA5, cfg, result[63:0] MSB-first, cycles[15:0]}}: cycles is\n\
         // the smallest L from which every capture equals the settled result, the worst over the repetitions, and\n\
         // 0 when the result did not settle inside the window. 100 MHz board clock; 8N1 115200 (CLKS_PER_BIT=868).\n\
         // Host: scripts/fabric-host.py checks each result against the VM.\n\
         module nexys_fabric_soc #(parameter CLKS_PER_BIT = 868, parameter LMAX = 255, parameter REPS = 8) (\n\
         \x20 input        clk,\n\
         \x20 input        rst,\n\
         \x20 output reg   uart_tx,\n\
         \x20 output [15:0] led\n\
         );\n\
         \x20 localparam NVEC = {nvec};\n\
         \x20 // vector ROM: {{cfg, in0, in1, in2}} of vector i.\n\
         \x20 function [{rw}:0] rom(input [5:0] i);\n\
         \x20\x20 case (i)\n\
         {rom}\
         \x20\x20\x20 default: rom = 0;\n\
         \x20\x20 endcase\n\
         \x20 endfunction\n\
         \x20 reg  [5:0] vec;\n\
         \x20 wire [5:0] nxt_vec = (vec == NVEC - 1) ? 6'd0 : vec + 6'd1;\n\
         \x20 wire [{rw}:0] tgt = rom(vec);\n\
         \x20 wire [{rw}:0] nxt = rom(nxt_vec);\n\
         \x20 // the launch register: the array computes from these.\n\
         \x20 reg [{cwm}:0] l_cfg; reg [63:0] l_in0, l_in1, l_in2;\n\
         \x20 wire [63:0] fab_out;\n\
         \x20 config_fabric u_fab(.in0(l_in0), .in1(l_in1), .in2(l_in2), .cfg(l_cfg), .result(fab_out));\n\
         \x20 localparam M_PRIME=4'd0, M_SETTLE=4'd1, M_LAUNCH=4'd2, M_WAIT=4'd3, M_CHECK=4'd4,\n\
         \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20\x20 M_LOAD=4'd5, M_START=4'd6, M_DATA=4'd7, M_STOP=4'd8;\n\
         \x20 reg [3:0]  m;\n\
         \x20 reg [15:0] cnt;      // clocks since the launch register was last loaded\n\
         \x20 reg [15:0] lat;      // the capture latency under test\n\
         \x20 reg [15:0] settle;   // the worst settle over the repetitions so far\n\
         \x20 reg [15:0] rep;\n\
         \x20 reg        scan;     // 0: the reference captures; 1: the descending scan\n\
         \x20 reg        prime_b;  // 0: the inverted inputs prime the launch; 1: the next vector does\n\
         \x20 reg        stable;   // every reference agreed: the result settled inside the window\n\
         \x20 reg        have_ref; // result holds the first repetition's settled value\n\
         \x20 reg [63:0] cap;      // the capture at LMAX (reference)\n\
         \x20 reg [63:0] cap2;     // the capture at 2*LMAX (reference) or at lat (scan)\n\
         \x20 reg [63:0] ref_v;    // this repetition's settled value\n\
         \x20 reg [63:0] result;   // the settled value the record streams\n\
         \x20 wire [15:0] want = scan ? lat : 16'd2 * LMAX;\n\
         \x20 wire [15:0] miss = lat + 16'd1;\n\
         \x20 wire [15:0] cycles_out = stable ? settle : 16'd0;\n\
         \x20 wire        last_rep = (rep == REPS - 1);\n\
         \x20 // UART TX byte sequencer: 12 bytes a record.\n\
         \x20 reg [15:0] clk_cnt;\n\
         \x20 reg [2:0]  bit_idx;\n\
         \x20 reg [7:0]  tx_byte;\n\
         \x20 reg [3:0]  pos;\n\
         \x20 assign led = {{m, vec, settle[5:0]}};\n\
         \x20 wire [7:0] res_byte = result[(9 - pos) * 8 +: 8];\n\
         \x20 always @(posedge clk) begin\n\
         \x20\x20 if (rst) begin\n\
         \x20\x20\x20 m<=M_PRIME; uart_tx<=1'b1; clk_cnt<=16'd0; bit_idx<=3'd0; pos<=4'd0; tx_byte<=8'd0; vec<=6'd0;\n\
         \x20\x20\x20 cnt<=16'd0; lat<=16'd0; settle<=16'd0; rep<=16'd0; scan<=1'b0; prime_b<=1'b0; stable<=1'b1; have_ref<=1'b0;\n\
         \x20\x20\x20 l_cfg<={cw}'d0; l_in0<=64'd0; l_in1<=64'd0; l_in2<=64'd0; cap<=64'd0; cap2<=64'd0; ref_v<=64'd0; result<=64'd0;\n\
         \x20\x20 end else case (m)\n\
         \x20\x20\x20 M_PRIME: begin\n\
         \x20\x20\x20\x20 if (!prime_b) begin\n\
         \x20\x20\x20\x20\x20 l_cfg<=tgt[{rw}:192]; l_in0<=~tgt[191:128]; l_in1<=~tgt[127:64]; l_in2<=~tgt[63:0];\n\
         \x20\x20\x20\x20 end else begin\n\
         \x20\x20\x20\x20\x20 l_cfg<=nxt[{rw}:192]; l_in0<=nxt[191:128]; l_in1<=nxt[127:64]; l_in2<=nxt[63:0];\n\
         \x20\x20\x20\x20 end\n\
         \x20\x20\x20\x20 cnt<=16'd1; m<=M_SETTLE;\n\
         \x20\x20\x20 end\n\
         \x20\x20\x20 M_SETTLE: if (cnt == 16'd2 * LMAX) m<=M_LAUNCH; else cnt<=cnt+16'd1;\n\
         \x20\x20\x20 M_LAUNCH: begin\n\
         \x20\x20\x20\x20 l_cfg<=tgt[{rw}:192]; l_in0<=tgt[191:128]; l_in1<=tgt[127:64]; l_in2<=tgt[63:0];\n\
         \x20\x20\x20\x20 cnt<=16'd1; m<=M_WAIT;\n\
         \x20\x20\x20 end\n\
         \x20\x20\x20 M_WAIT: begin\n\
         \x20\x20\x20\x20 if (!scan && cnt == LMAX) cap<=fab_out;\n\
         \x20\x20\x20\x20 if (cnt == want) begin cap2<=fab_out; m<=M_CHECK; end else cnt<=cnt+16'd1;\n\
         \x20\x20\x20 end\n\
         \x20\x20\x20 M_CHECK: begin\n\
         \x20\x20\x20\x20 if (!scan) begin\n\
         \x20\x20\x20\x20\x20 if (cap != cap2) stable<=1'b0;\n\
         \x20\x20\x20\x20\x20 if (!prime_b) begin ref_v<=cap2; prime_b<=1'b1; m<=M_PRIME; end\n\
         \x20\x20\x20\x20\x20 else begin\n\
         \x20\x20\x20\x20\x20\x20 if (cap2 != ref_v) stable<=1'b0;\n\
         \x20\x20\x20\x20\x20\x20 if (!have_ref) begin result<=ref_v; have_ref<=1'b1; end else if (ref_v != result) stable<=1'b0;\n\
         \x20\x20\x20\x20\x20\x20 scan<=1'b1; prime_b<=1'b0; lat<=LMAX - 16'd1; m<=M_PRIME;\n\
         \x20\x20\x20\x20\x20 end\n\
         \x20\x20\x20\x20 end else if (cap2 != ref_v || (prime_b && lat == 16'd1)) begin\n\
         \x20\x20\x20\x20\x20 if (cap2 != ref_v) begin if (miss > settle) settle<=miss; end\n\
         \x20\x20\x20\x20\x20 else if (settle == 16'd0) settle<=16'd1;\n\
         \x20\x20\x20\x20\x20 if (last_rep) begin pos<=4'd0; m<=M_LOAD; end\n\
         \x20\x20\x20\x20\x20 else begin rep<=rep+16'd1; scan<=1'b0; prime_b<=1'b0; m<=M_PRIME; end\n\
         \x20\x20\x20\x20 end else if (!prime_b) begin prime_b<=1'b1; m<=M_PRIME; end\n\
         \x20\x20\x20\x20 else begin prime_b<=1'b0; lat<=lat-16'd1; m<=M_PRIME; end\n\
         \x20\x20\x20 end\n\
         \x20\x20\x20 M_LOAD: begin\n\
         \x20\x20\x20\x20 case (pos)\n\
         \x20\x20\x20\x20\x20 4'd0: tx_byte<=8'hA5;\n\
         \x20\x20\x20\x20\x20 4'd1: tx_byte<={{{pad}'d0, tgt[{rw}:192]}};\n\
         \x20\x20\x20\x20\x20 4'd10: tx_byte<=cycles_out[15:8];\n\
         \x20\x20\x20\x20\x20 4'd11: tx_byte<=cycles_out[7:0];\n\
         \x20\x20\x20\x20\x20 default: tx_byte<=res_byte;\n\
         \x20\x20\x20\x20 endcase\n\
         \x20\x20\x20\x20 clk_cnt<=16'd0; bit_idx<=3'd0; m<=M_START;\n\
         \x20\x20\x20 end\n\
         \x20\x20\x20 M_START: begin uart_tx<=1'b0; if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; m<=M_DATA; end else clk_cnt<=clk_cnt+16'd1; end\n\
         \x20\x20\x20 M_DATA: begin uart_tx<=tx_byte[bit_idx];\n\
         \x20\x20\x20\x20 if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; if (bit_idx==3'd7) m<=M_STOP; else bit_idx<=bit_idx+3'd1; end else clk_cnt<=clk_cnt+16'd1;\n\
         \x20\x20\x20 end\n\
         \x20\x20\x20 M_STOP: begin uart_tx<=1'b1;\n\
         \x20\x20\x20\x20 if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0;\n\
         \x20\x20\x20\x20\x20 if (pos==4'd11) begin\n\
         \x20\x20\x20\x20\x20\x20 // the next vector, measured afresh; the stream loops so a host can lock onto any lap.\n\
         \x20\x20\x20\x20\x20\x20 vec<=nxt_vec; rep<=16'd0; settle<=16'd0; stable<=1'b1; have_ref<=1'b0; scan<=1'b0; prime_b<=1'b0; m<=M_PRIME;\n\
         \x20\x20\x20\x20\x20 end else begin pos<=pos+4'd1; m<=M_LOAD; end\n\
         \x20\x20\x20\x20 end else clk_cnt<=clk_cnt+16'd1;\n\
         \x20\x20\x20 end\n\
         \x20\x20\x20 default: m<=M_PRIME;\n\
         \x20\x20 endcase\n\
         \x20 end\n\
         endmodule\n",
    )
}

/// The board pin constraints for `nexys_fabric_soc` (Nexys A7-100T): 100 MHz clock (E3), reset (J15),
/// UART TX (D4), status LEDs. No `uart_rx` — the vector ROM is baked in, so the SoC only streams out.
pub fn emit_nexys_fabric_xdc() -> String {
    let mut s = String::from(
        "## Nexys A7-100T pin constraints for nexys_fabric_soc (100 MHz, UART TX readback only).\n\
         set_property -dict { PACKAGE_PIN E3  IOSTANDARD LVCMOS33 } [get_ports { clk }]\n\
         create_clock -add -name sys_clk_pin -period 10.00 -waveform {0 5} [get_ports { clk }]\n\
         set_property -dict { PACKAGE_PIN J15 IOSTANDARD LVCMOS33 } [get_ports { rst }]\n\
         set_property -dict { PACKAGE_PIN D4  IOSTANDARD LVCMOS33 } [get_ports { uart_tx }]\n",
    );
    const LED_PINS: [&str; 16] = [
        "H17", "K15", "J13", "N14", "R18", "V17", "U17", "U16", "V16", "T15", "U14", "T16", "V15",
        "V14", "V12", "V11",
    ];
    for (i, pin) in LED_PINS.iter().enumerate() {
        s.push_str(&format!(
            "set_property -dict {{ PACKAGE_PIN {pin} IOSTANDARD LVCMOS33 }} [get_ports {{ led[{i}] }}]\n"
        ));
    }
    s
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// STAGE X2c (XPU): the STATE PLANE on real silicon. Unlike the combinational config_fabric (one array
// muxed across datapaths, 1 clock/op), a state-plane kernel is a SEQUENCED datapath (`state_<name>`, a
// LUTRAM heap + a micro-sequencer). So its SoC drives ONE kernel over a vector battery: it pulses the
// datapath's reset, WAITS for `done` (counting the MULTI-cycle cost — the honest state-plane latency,
// not 1), captures the result, and streams the 12-byte record. This puts collections/structs on the
// fabric CHIP, matching X1's silicon bar.

/// The flashed state-plane kernel: `grade(a,b,c)` is a WHOLE MULTI-OP-CLASS program — it builds the list
/// `[a,b,c]` on the heap (COLLECTIONS), sums its elements (ARITHMETIC), and clamps the total to `[0, 100]`
/// (CONTROL FLOW — two if-converted branches). So the flashed datapath exercises three distinct op-classes at
/// once — Mem (HeapAlloc/Write/Read) + Arith (adds) + Select (the clamps) — all in ONE sequenced datapath on
/// silicon: the fabric running a real program, not a single op.
pub const STATE_SOC_SRC: &str = "## To grade (a: Int, b: Int, c: Int) -> Int:\n    Let xs be [a, b, c].\n    Let total be item 1 of xs + item 2 of xs + item 3 of xs.\n    Let result be total.\n    If total is greater than 100:\n        Set result to 100.\n    If total is less than 0:\n        Set result to 0.\n    Return result.\n";
pub const STATE_SOC_NAME: &str = "grade";

/// The input battery the state SoC runs on silicon (each a `(a,b,c)` triple → `clamp(a+b+c, 0, 100)`). Chosen
/// to exercise the pass-through, the UPPER clamp (sum > 100), and the LOWER clamp (sum < 0) — so the control
/// flow is genuinely taken on hardware, not just present.
pub fn state_soc_vectors() -> Vec<[i64; 3]> {
    vec![
        [10, 20, 30],    // 60  (pass-through)
        [1, 2, 3],       // 6   (pass-through)
        [40, 40, 40],    // 120 -> 100 (upper clamp)
        [-40, -40, -40], // -120 -> 0 (lower clamp)
        [33, 33, 33],    // 99  (pass-through, near the ceiling)
        [0, 0, 0],       // 0
        [70, 20, 5],     // 95  (pass-through)
    ]
}

/// The sequenced state-plane datapath RTL (`state_grade`) — the LUTRAM-heap micro-sequencer for the kernel.
pub fn emit_state_soc_module() -> String {
    let df = decision_fabric_from_source(STATE_SOC_SRC, STATE_SOC_NAME);
    emit_state_plane_verilog(STATE_SOC_NAME, &df, STATE_PLANE_HEAP_WORDS)
}

/// The SIMPLE state SoC (`state_soc`) for the iverilog proof: it wraps `state_<name>` with a CYCLE COUNTER
/// (cycles from reset-release to `done`) and exposes `result`/`cycles`/`done`. A host/testbench presents
/// `(a,b,c)`, pulses `rst`, and reads back the multi-cycle result — the board-independent ≡VM proof surface.
pub fn emit_state_soc() -> String {
    format!(
        "// state_soc — the sequenced state-plane datapath under a cycle counter (Stage X2c iverilog surface).\n\
         // On `rst` the datapath restarts; the counter runs until `done`, so `cycles` is the HONEST multi-cycle\n\
         // state-plane latency (a heap program is NOT a 1-clock decision). `result` is valid when `done` rises.\n\
         module state_soc(\n\
         \x20 input               clk,\n\
         \x20 input               rst,\n\
         \x20 input  signed [63:0] in0,\n\
         \x20 input  signed [63:0] in1,\n\
         \x20 input  signed [63:0] in2,\n\
         \x20 output signed [63:0] result,\n\
         \x20 output [15:0]       cycles,\n\
         \x20 output              done\n\
         );\n\
         \x20 wire signed [63:0] dp_out;\n\
         \x20 wire dp_done;\n\
         \x20 reg [15:0] cyc; reg counting;\n\
         \x20 state_{name} u_dp(.clk(clk), .rst(rst), .in0(in0), .in1(in1), .in2(in2), .result(dp_out), .done(dp_done));\n\
         \x20 always @(posedge clk) begin\n\
         \x20\x20 if (rst) begin cyc <= 16'd0; counting <= 1'b1; end\n\
         \x20\x20 else if (counting) begin if (dp_done) counting <= 1'b0; else cyc <= cyc + 16'd1; end\n\
         \x20 end\n\
         \x20 assign result = dp_out;\n\
         \x20 assign cycles = cyc;\n\
         \x20 assign done = dp_done;\n\
         endmodule\n",
        name = STATE_SOC_NAME,
    )
}

/// A self-checking testbench for `state_soc`: drive each `(a,b,c)` vector, pulse `rst`, run until `done`, and
/// `$display(\"result cycles\")`. `state_soc_lock` runs it under iverilog and checks each result ≡ the VM and
/// each cycle count > 1 (a sequenced state program, not a 1-clock combinational decision).
pub fn emit_state_soc_tb() -> String {
    let vs = state_soc_vectors();
    let lit = |n: i64| {
        if n < 0 {
            format!("-64'sd{}", -n)
        } else {
            format!("64'sd{n}")
        }
    };
    let mut runs = String::new();
    for a in &vs {
        runs.push_str(&format!(
            "    run({}, {}, {});\n",
            lit(a[0]),
            lit(a[1]),
            lit(a[2])
        ));
    }
    format!(
        "// tb_state_soc — the sequenced state plane vs the RUNNING VM over the list-sum battery.\n\
         `timescale 1ns/1ps\n\
         module tb_state_soc;\n\
         \x20 reg clk = 0, rst;\n\
         \x20 reg signed [63:0] in0, in1, in2;\n\
         \x20 wire signed [63:0] result;\n\
         \x20 wire [15:0] cycles;\n\
         \x20 wire done;\n\
         \x20 integer c;\n\
         \x20 state_soc dut(.clk(clk), .rst(rst), .in0(in0), .in1(in1), .in2(in2), .result(result), .cycles(cycles), .done(done));\n\
         \x20 always #5 clk = ~clk;\n\
         \x20 task run(input signed [63:0] a0, input signed [63:0] a1, input signed [63:0] a2);\n\
         \x20 begin\n\
         \x20\x20 @(negedge clk); in0 = a0; in1 = a1; in2 = a2; rst = 1;\n\
         \x20\x20 @(negedge clk); rst = 0;\n\
         \x20\x20 c = 0; while (!done && c < 200) begin @(negedge clk); c = c + 1; end\n\
         \x20\x20 $display(\"%0d %0d\", result, cycles);\n\
         \x20 end\n\
         \x20 endtask\n\
         \x20 initial begin\n\
         {runs}\
         \x20\x20 $finish;\n\
         \x20 end\n\
         endmodule\n",
    )
}

/// The FLASHABLE state SoC (`nexys_state_soc`) for the Nexys A7-100T: a baked-in `(a,b,c)` vector ROM drives
/// `state_<name>` — pulse the datapath reset, wait `done` (counting the multi-cycle latency), capture the
/// result, and stream the 12-byte record `{0xA5, vec, result[63:0] MSB-first, cycles[15:0]}` out the UART TX,
/// looping forever so a host can frame-lock at any time. 100 MHz; 8N1 115200 (CLKS_PER_BIT=868).
pub fn emit_nexys_state_soc() -> String {
    let vs = state_soc_vectors();
    let nvec = vs.len();
    let mut rom = String::new();
    for (i, a) in vs.iter().enumerate() {
        let lit = |n: i64| {
            if n < 0 {
                format!("-64'sd{}", -n)
            } else {
                format!("64'sd{n}")
            }
        };
        rom.push_str(&format!(
            "      {i}: begin v_in0 = {}; v_in1 = {}; v_in2 = {}; end\n",
            lit(a[0]),
            lit(a[1]),
            lit(a[2]),
        ));
    }
    format!(
        "// nexys_state_soc — the sequenced STATE PLANE on a Nexys A7-100T (Stage X2c). A baked-in vector ROM\n\
         // drives state_{name} (a LUTRAM-heap micro-sequencer) over the list-sum battery; each result streams\n\
         // out the UART TX as a 12-byte record {{0xA5, vec, result[63:0] MSB-first, cycles[15:0]}}. cycles is\n\
         // the HONEST multi-cycle state-plane latency (not the combinational 1). 100 MHz; 8N1 115200.\n\
         module nexys_state_soc #(parameter CLKS_PER_BIT = 868) (\n\
         \x20 input        clk,\n\
         \x20 input        rst,\n\
         \x20 output reg   uart_tx,\n\
         \x20 output [15:0] led\n\
         );\n\
         \x20 localparam NVEC = {nvec};\n\
         \x20 reg [5:0] vec;\n\
         \x20 reg signed [63:0] v_in0, v_in1, v_in2;\n\
         \x20 always @(*) begin\n\
         \x20\x20 v_in0 = 64'sd0; v_in1 = 64'sd0; v_in2 = 64'sd0;\n\
         \x20\x20 case (vec)\n\
         {rom}\
         \x20\x20\x20 default: ;\n\
         \x20\x20 endcase\n\
         \x20 end\n\
         \x20 // the sequenced datapath + a run controller.\n\
         \x20 reg dp_rst; wire dp_done; wire signed [63:0] dp_out;\n\
         \x20 state_{name} u_dp(.clk(clk), .rst(dp_rst), .in0(v_in0), .in1(v_in1), .in2(v_in2), .result(dp_out), .done(dp_done));\n\
         \x20 reg signed [63:0] cap_out; reg [15:0] cap_cyc;\n\
         \x20 localparam S_RESET=4'd0, S_DRST=4'd1, S_DWAIT=4'd2, S_CAP=4'd3, S_LOAD=4'd4, S_START=4'd5, S_DATA=4'd6, S_STOP=4'd7, S_DONE=4'd8;\n\
         \x20 reg [3:0]  state;\n\
         \x20 reg [15:0] clk_cnt;\n\
         \x20 reg [2:0]  bit_idx;\n\
         \x20 reg [7:0]  tx_byte;\n\
         \x20 reg [3:0]  pos;\n\
         \x20 reg [3:0]  rst_cnt;\n\
         \x20 reg [3:0]  dprst_cnt;\n\
         \x20 assign led = {{state, vec[4:0], 7'd0}};\n\
         \x20 wire [7:0] res_byte = cap_out[(9 - pos) * 8 +: 8];\n\
         \x20 always @(posedge clk) begin\n\
         \x20\x20 if (rst) begin\n\
         \x20\x20\x20 state<=S_RESET; uart_tx<=1'b1; clk_cnt<=16'd0; bit_idx<=3'd0; vec<=6'd0; pos<=4'd0; rst_cnt<=4'd0;\n\
         \x20\x20\x20 tx_byte<=8'd0; dp_rst<=1'b1; dprst_cnt<=4'd0; cap_out<=64'sd0; cap_cyc<=16'd0;\n\
         \x20\x20 end else case (state)\n\
         \x20\x20\x20 S_RESET: if (rst_cnt==4'd8) begin state<=S_DRST; dp_rst<=1'b1; dprst_cnt<=4'd0; end else rst_cnt<=rst_cnt+4'd1;\n\
         \x20\x20\x20 // pulse the datapath reset so it latches inputs + zeroes its heap, then run.\n\
         \x20\x20\x20 S_DRST: begin dp_rst<=1'b1; if (dprst_cnt==4'd3) begin state<=S_DWAIT; dp_rst<=1'b0; cap_cyc<=16'd0; end else dprst_cnt<=dprst_cnt+4'd1; end\n\
         \x20\x20\x20 S_DWAIT: begin dp_rst<=1'b0; if (dp_done) begin cap_out<=dp_out; state<=S_CAP; end else cap_cyc<=cap_cyc+16'd1; end\n\
         \x20\x20\x20 S_CAP: begin pos<=4'd0; state<=S_LOAD; end\n\
         \x20\x20\x20 S_LOAD: begin\n\
         \x20\x20\x20\x20 case (pos)\n\
         \x20\x20\x20\x20\x20 4'd0: tx_byte<=8'hA5;\n\
         \x20\x20\x20\x20\x20 4'd1: tx_byte<={{2'd0, vec}};\n\
         \x20\x20\x20\x20\x20 4'd10: tx_byte<=cap_cyc[15:8];\n\
         \x20\x20\x20\x20\x20 4'd11: tx_byte<=cap_cyc[7:0];\n\
         \x20\x20\x20\x20\x20 default: tx_byte<=res_byte;\n\
         \x20\x20\x20\x20 endcase\n\
         \x20\x20\x20\x20 clk_cnt<=16'd0; bit_idx<=3'd0; state<=S_START;\n\
         \x20\x20\x20 end\n\
         \x20\x20\x20 S_START: begin uart_tx<=1'b0; if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; state<=S_DATA; end else clk_cnt<=clk_cnt+16'd1; end\n\
         \x20\x20\x20 S_DATA: begin uart_tx<=tx_byte[bit_idx];\n\
         \x20\x20\x20\x20 if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; if (bit_idx==3'd7) state<=S_STOP; else bit_idx<=bit_idx+3'd1; end else clk_cnt<=clk_cnt+16'd1;\n\
         \x20\x20\x20 end\n\
         \x20\x20\x20 S_STOP: begin uart_tx<=1'b1;\n\
         \x20\x20\x20\x20 if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0;\n\
         \x20\x20\x20\x20\x20 if (pos==4'd11) begin\n\
         \x20\x20\x20\x20\x20\x20 pos<=4'd0;\n\
         \x20\x20\x20\x20\x20\x20 if (vec==NVEC-1) vec<=6'd0; else vec<=vec+6'd1;\n\
         \x20\x20\x20\x20\x20\x20 state<=S_DRST; dp_rst<=1'b1; dprst_cnt<=4'd0;\n\
         \x20\x20\x20\x20\x20 end else begin pos<=pos+4'd1; state<=S_LOAD; end\n\
         \x20\x20\x20\x20 end else clk_cnt<=clk_cnt+16'd1;\n\
         \x20\x20\x20 end\n\
         \x20\x20\x20 S_DONE: uart_tx<=1'b1;\n\
         \x20\x20\x20 default: state<=S_DONE;\n\
         \x20\x20 endcase\n\
         \x20 end\n\
         endmodule\n",
        name = STATE_SOC_NAME,
    )
}

/// The Nexys A7-100T pin constraints for `nexys_state_soc` — identical ports to the fabric SoC (100 MHz clk,
/// reset, UART TX, LEDs), so the same pin map.
pub fn emit_nexys_state_xdc() -> String {
    emit_nexys_fabric_xdc().replace("nexys_fabric_soc", "nexys_state_soc")
}

// ══════════════════════════════════════════════════════════════════════════════════════════════════
// STAGE X3b (XPU): the FLOAT LANE on real silicon. A float kernel is a SEQUENCED datapath binding fpu.v
// (multi-cycle IEEE-754 units). This SoC drives ONE float kernel over a battery of float inputs: it pulses
// the datapath reset, WAITS for `done` (counting the FPU pipeline cycles), captures the f64 RESULT BITS, and
// streams the 12-byte record. The host decodes the f64 and floors it to the integer hypotenuse. This puts
// IEEE-754 floating point on the fabric CHIP, matching X1/X2c's silicon bar.

/// The flashed float kernel: `fhypf(a,b) = sqrt(a*a + b*b)` — the Euclidean hypotenuse as a Real. Exercises the
/// full float lane: `fp_mul` (twice) + `fp_addsub` + `fp_sqrt`. It returns a FLOAT (not floored) so the SoC
/// streams the raw f64 bits — the host floors to the integer hypotenuse (no float→int RTL needed on-chip).
pub const FLOAT_SOC_SRC: &str =
    "## To fhypf (a: Float, b: Float) -> Real:\n    Return sqrt(a * a + b * b).\n";
pub const FLOAT_SOC_NAME: &str = "fhypf";

/// The `(a, b)` float pairs the float SoC runs on silicon → each hypotenuse (mostly Pythagorean integers).
pub fn float_soc_vectors() -> Vec<(f64, f64)> {
    vec![
        (3.0, 4.0),  // 5
        (6.0, 8.0),  // 10
        (5.0, 12.0), // 13
        (8.0, 15.0), // 17
        (0.0, 7.0),  // 7
        (2.5, 6.0),  // 6.5 -> floor 6
        (9.0, 12.0), // 15
    ]
}

/// The sequenced float datapath RTL (`float_fhypf`) — the fpu.v-bound micro-sequencer for the kernel.
pub fn emit_float_soc_module() -> String {
    let df = decision_fabric_from_source(FLOAT_SOC_SRC, FLOAT_SOC_NAME);
    emit_float_datapath_verilog(FLOAT_SOC_NAME, &df)
}

/// The SIMPLE float SoC (`float_soc`) for the iverilog proof: `float_<name>` under a CYCLE COUNTER, exposing
/// the f64 `result` bits / `cycles` / `done`. A testbench presents float bit-patterns and reads the result.
pub fn emit_float_soc() -> String {
    format!(
        "// float_soc — the sequenced float datapath (binding fpu.v) under a cycle counter (X3b iverilog surface).\n\
         // `cycles` is the FPU pipeline latency (fp_mul/fp_addsub/fp_sqrt in sequence); `result` is the f64\n\
         // bit-pattern, valid when `done` rises.\n\
         module float_soc(\n\
         \x20 input               clk,\n\
         \x20 input               rst,\n\
         \x20 input  [63:0]       in0,\n\
         \x20 input  [63:0]       in1,\n\
         \x20 output [63:0]       result,\n\
         \x20 output [15:0]       cycles,\n\
         \x20 output              done\n\
         );\n\
         \x20 wire [63:0] dp_out; wire dp_done;\n\
         \x20 reg [15:0] cyc; reg counting;\n\
         \x20 float_{name} u_dp(.clk(clk), .rst(rst), .in0(in0), .in1(in1), .result(dp_out), .done(dp_done));\n\
         \x20 always @(posedge clk) begin\n\
         \x20\x20 if (rst) begin cyc <= 16'd0; counting <= 1'b1; end\n\
         \x20\x20 else if (counting) begin if (dp_done) counting <= 1'b0; else cyc <= cyc + 16'd1; end\n\
         \x20 end\n\
         \x20 assign result = dp_out;\n\
         \x20 assign cycles = cyc;\n\
         \x20 assign done = dp_done;\n\
         endmodule\n",
        name = FLOAT_SOC_NAME,
    )
}

/// A self-checking testbench for `float_soc`: drive each float pair (bit-patterns), run to `done`, and
/// `$display(\"result_hex cycles\")`. `float_soc_lock` runs it under iverilog (with fpu.v) and checks each
/// result's f64 bits ≡ the model/Rust and each cycle count > 1.
pub fn emit_float_soc_tb() -> String {
    let vs = float_soc_vectors();
    let mut runs = String::new();
    for (a, b) in &vs {
        runs.push_str(&format!(
            "    run(64'h{:016x}, 64'h{:016x});\n",
            a.to_bits(),
            b.to_bits()
        ));
    }
    format!(
        "// tb_float_soc — the sequenced float lane (fpu.v) vs the f64 reference over the hypotenuse battery.\n\
         `timescale 1ns/1ps\n\
         module tb_float_soc;\n\
         \x20 reg clk = 0, rst;\n\
         \x20 reg [63:0] in0, in1;\n\
         \x20 wire [63:0] result;\n\
         \x20 wire [15:0] cycles;\n\
         \x20 wire done;\n\
         \x20 integer c;\n\
         \x20 float_soc dut(.clk(clk), .rst(rst), .in0(in0), .in1(in1), .result(result), .cycles(cycles), .done(done));\n\
         \x20 always #5 clk = ~clk;\n\
         \x20 task run(input [63:0] a, input [63:0] b);\n\
         \x20 begin\n\
         \x20\x20 @(negedge clk); in0 = a; in1 = b; rst = 1;\n\
         \x20\x20 @(negedge clk); rst = 0;\n\
         \x20\x20 c = 0; while (!done && c < 5000) begin @(negedge clk); c = c + 1; end\n\
         \x20\x20 $display(\"%h %0d\", result, cycles);\n\
         \x20 end\n\
         \x20 endtask\n\
         \x20 initial begin\n\
         {runs}\
         \x20\x20 $finish;\n\
         \x20 end\n\
         endmodule\n",
    )
}

/// The FLASHABLE float SoC (`nexys_float_soc`) for the Nexys A7-100T: a baked-in `(a,b)` float ROM drives
/// `float_<name>` — pulse the datapath reset, wait `done` (counting the FPU pipeline cycles), capture the f64
/// result, and stream the 12-byte record `{0xA5, vec, result[63:0] MSB-first, cycles[15:0]}` out the UART TX,
/// looping forever. The host decodes the f64 and floors it to the integer hypotenuse. 100 MHz; 8N1 115200.
pub fn emit_nexys_float_soc() -> String {
    let vs = float_soc_vectors();
    let nvec = vs.len();
    let mut rom = String::new();
    for (i, (a, b)) in vs.iter().enumerate() {
        rom.push_str(&format!(
            "      {i}: begin v_in0 = 64'h{:016x}; v_in1 = 64'h{:016x}; end\n",
            a.to_bits(),
            b.to_bits(),
        ));
    }
    format!(
        "// nexys_float_soc — the sequenced FLOAT LANE on a Nexys A7-100T (Stage X3b). A baked-in float ROM\n\
         // drives float_{name} (an fpu.v-bound micro-sequencer) over the hypotenuse battery; each f64 result\n\
         // streams out the UART TX as a 12-byte record {{0xA5, vec, result[63:0] MSB-first, cycles[15:0]}}.\n\
         // cycles = the FPU pipeline latency (fp_mul/fp_addsub/fp_sqrt in sequence).\n\
         // The multi-cycle FPU has deep combinational shifter stages (nextpnr Fmax ~13 MHz), so the 100 MHz\n\
         // board clock is DIVIDED by 16 to a ~6.25 MHz core clock the FPU meets timing on — running it at the\n\
         // full 100 MHz latches wrong intermediates (proven on silicon). 8N1 115200 at 6.25 MHz (CLKS_PER_BIT=54).\n\
         module nexys_float_soc #(parameter CLKS_PER_BIT = 54) (\n\
         \x20 input        clk,\n\
         \x20 input        rst,\n\
         \x20 output reg   uart_tx,\n\
         \x20 output [15:0] led\n\
         );\n\
         \x20 // Divide the 100 MHz board clock to ~6.25 MHz so the multi-cycle FPU meets timing on real silicon.\n\
         \x20 reg [3:0] clkdiv = 4'd0;\n\
         \x20 always @(posedge clk) clkdiv <= clkdiv + 4'd1;\n\
         \x20 wire coreclk = clkdiv[3];\n\
         \x20 localparam NVEC = {nvec};\n\
         \x20 reg [5:0] vec;\n\
         \x20 reg [63:0] v_in0, v_in1;\n\
         \x20 always @(*) begin\n\
         \x20\x20 v_in0 = 64'd0; v_in1 = 64'd0;\n\
         \x20\x20 case (vec)\n\
         {rom}\
         \x20\x20\x20 default: ;\n\
         \x20\x20 endcase\n\
         \x20 end\n\
         \x20 reg dp_rst; wire dp_done; wire [63:0] dp_out;\n\
         \x20 float_{name} u_dp(.clk(coreclk), .rst(dp_rst), .in0(v_in0), .in1(v_in1), .result(dp_out), .done(dp_done));\n\
         \x20 reg [63:0] cap_out; reg [15:0] cap_cyc;\n\
         \x20 localparam S_RESET=4'd0, S_DRST=4'd1, S_DWAIT=4'd2, S_CAP=4'd3, S_LOAD=4'd4, S_START=4'd5, S_DATA=4'd6, S_STOP=4'd7, S_DONE=4'd8;\n\
         \x20 reg [3:0]  state;\n\
         \x20 reg [15:0] clk_cnt;\n\
         \x20 reg [2:0]  bit_idx;\n\
         \x20 reg [7:0]  tx_byte;\n\
         \x20 reg [3:0]  pos;\n\
         \x20 reg [3:0]  rst_cnt;\n\
         \x20 reg [3:0]  dprst_cnt;\n\
         \x20 assign led = {{state, vec[4:0], 7'd0}};\n\
         \x20 wire [7:0] res_byte = cap_out[(9 - pos) * 8 +: 8];\n\
         \x20 always @(posedge coreclk) begin\n\
         \x20\x20 if (rst) begin\n\
         \x20\x20\x20 state<=S_RESET; uart_tx<=1'b1; clk_cnt<=16'd0; bit_idx<=3'd0; vec<=6'd0; pos<=4'd0; rst_cnt<=4'd0;\n\
         \x20\x20\x20 tx_byte<=8'd0; dp_rst<=1'b1; dprst_cnt<=4'd0; cap_out<=64'd0; cap_cyc<=16'd0;\n\
         \x20\x20 end else case (state)\n\
         \x20\x20\x20 S_RESET: if (rst_cnt==4'd8) begin state<=S_DRST; dp_rst<=1'b1; dprst_cnt<=4'd0; end else rst_cnt<=rst_cnt+4'd1;\n\
         \x20\x20\x20 S_DRST: begin dp_rst<=1'b1; if (dprst_cnt==4'd3) begin state<=S_DWAIT; dp_rst<=1'b0; cap_cyc<=16'd0; end else dprst_cnt<=dprst_cnt+4'd1; end\n\
         \x20\x20\x20 S_DWAIT: begin dp_rst<=1'b0; if (dp_done) begin cap_out<=dp_out; state<=S_CAP; end else cap_cyc<=cap_cyc+16'd1; end\n\
         \x20\x20\x20 S_CAP: begin pos<=4'd0; state<=S_LOAD; end\n\
         \x20\x20\x20 S_LOAD: begin\n\
         \x20\x20\x20\x20 case (pos)\n\
         \x20\x20\x20\x20\x20 4'd0: tx_byte<=8'hA5;\n\
         \x20\x20\x20\x20\x20 4'd1: tx_byte<={{2'd0, vec}};\n\
         \x20\x20\x20\x20\x20 4'd10: tx_byte<=cap_cyc[15:8];\n\
         \x20\x20\x20\x20\x20 4'd11: tx_byte<=cap_cyc[7:0];\n\
         \x20\x20\x20\x20\x20 default: tx_byte<=res_byte;\n\
         \x20\x20\x20\x20 endcase\n\
         \x20\x20\x20\x20 clk_cnt<=16'd0; bit_idx<=3'd0; state<=S_START;\n\
         \x20\x20\x20 end\n\
         \x20\x20\x20 S_START: begin uart_tx<=1'b0; if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; state<=S_DATA; end else clk_cnt<=clk_cnt+16'd1; end\n\
         \x20\x20\x20 S_DATA: begin uart_tx<=tx_byte[bit_idx];\n\
         \x20\x20\x20\x20 if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0; if (bit_idx==3'd7) state<=S_STOP; else bit_idx<=bit_idx+3'd1; end else clk_cnt<=clk_cnt+16'd1;\n\
         \x20\x20\x20 end\n\
         \x20\x20\x20 S_STOP: begin uart_tx<=1'b1;\n\
         \x20\x20\x20\x20 if (clk_cnt==CLKS_PER_BIT-1) begin clk_cnt<=16'd0;\n\
         \x20\x20\x20\x20\x20 if (pos==4'd11) begin\n\
         \x20\x20\x20\x20\x20\x20 pos<=4'd0;\n\
         \x20\x20\x20\x20\x20\x20 if (vec==NVEC-1) vec<=6'd0; else vec<=vec+6'd1;\n\
         \x20\x20\x20\x20\x20\x20 state<=S_DRST; dp_rst<=1'b1; dprst_cnt<=4'd0;\n\
         \x20\x20\x20\x20\x20 end else begin pos<=pos+4'd1; state<=S_LOAD; end\n\
         \x20\x20\x20\x20 end else clk_cnt<=clk_cnt+16'd1;\n\
         \x20\x20\x20 end\n\
         \x20\x20\x20 S_DONE: uart_tx<=1'b1;\n\
         \x20\x20\x20 default: state<=S_DONE;\n\
         \x20\x20 endcase\n\
         \x20 end\n\
         endmodule\n",
        name = FLOAT_SOC_NAME,
    )
}

/// The Nexys A7-100T pin constraints for `nexys_float_soc` — same ports as the fabric SoC, same pin map.
pub fn emit_nexys_float_xdc() -> String {
    emit_nexys_fabric_xdc().replace("nexys_fabric_soc", "nexys_float_soc")
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/fabric_soc/tests.rs"]
mod tests;

// ── the placed-and-routed arrays as data, for the technical drawings ─────────────────────────────────────

fn drawing_src2(s: super::fabric_grid::Src2) -> String {
    use super::fabric_grid::Src2;
    match s {
        Src2::N => "N".into(),
        Src2::W => "W".into(),
        Src2::In(k) => format!("In{k}"),
        Src2::Const(c) => format!("C{c}"),
        Src2::Zero => "Zero".into(),
    }
}

fn drawing_drive(d: super::fabric_grid::Drive) -> String {
    use super::fabric_grid::Drive;
    match d {
        Drive::Alu => "Alu".into(),
        Drive::Pass(s) => drawing_src2(s),
    }
}

fn drawing_cell(k: super::fabric_grid::CellKind) -> serde_json::Value {
    use super::fabric_grid::CellKind;
    use serde_json::json;
    match k {
        CellKind::Arith(op, a, b) => {
            json!({ "kind": "Arith", "op": format!("{op:?}"), "a": drawing_src2(a), "b": drawing_src2(b) })
        }
        CellKind::Select(cmp, ca, cb, t, e) => json!({
            "kind": "Select", "cmp": format!("{cmp:?}"), "ca": drawing_src2(ca), "cb": drawing_src2(cb),
            "t": drawing_src2(t), "e": drawing_src2(e)
        }),
        CellKind::Func(u, a, b) => {
            json!({ "kind": "Func", "op": format!("{u:?}"), "a": drawing_src2(a), "b": drawing_src2(b) })
        }
        CellKind::Simd(op, lane, a, b) => json!({
            "kind": "Simd", "op": format!("{op:?}"), "lane": format!("{lane:?}"), "a": drawing_src2(a), "b": drawing_src2(b)
        }),
        CellKind::Mem(op, a, b) => {
            json!({ "kind": "Mem", "op": format!("{op:?}"), "a": drawing_src2(a), "b": drawing_src2(b) })
        }
    }
}

fn drawing_out(o: (usize, usize, super::fabric_grid::Chan)) -> serde_json::Value {
    serde_json::json!([
        o.0,
        o.1,
        if o.2 == super::fabric_grid::Chan::Vs {
            "vs"
        } else {
            "he"
        }
    ])
}

/// One placed grid as JSON: its shape, the tap that carries the result, and every tile's compute + the source
/// each of its two channels passes (`N`/`W` neighbour, `In<k>` input, `C<v>` constant, `Zero`, or `Alu`).
fn drawing_grid(name: &str, source: &str, g: &super::fabric_grid::Grid) -> serde_json::Value {
    let (rows, cols, used) = super::fabric_grid::grid_area(g);
    serde_json::json!({
        "name": name,
        "source": source,
        "rows": rows,
        "cols": cols,
        "n_in": g.n_in,
        "used_tiles": used,
        "used_channels": g.used_channels(),
        "out": drawing_out(g.out),
        "extra_outs": g.extra_outs.iter().map(|o| drawing_out(*o)).collect::<Vec<_>>(),
        "tiles": g.tiles.iter().map(|row| row.iter().map(|t| serde_json::json!({
            "alu": t.alu.map(drawing_cell),
            "vs": drawing_drive(t.vs),
            "he": drawing_drive(t.he),
        })).collect::<Vec<_>>()).collect::<Vec<_>>(),
    })
}

/// The placed-and-routed tile arrays as JSON — the source of truth for every technical drawing of the fabric,
/// so a drawing of a tile array is the placer's own output, never hand-drawn.
/// Contains the five `config_fabric` islands (the 8×3 array that runs 14/14 vectors on both boards) and the
/// Horner cubic residual on both placers (the diagonal `route_straight_line` behind `grid_horner.v`, and the
/// area-minimizing `route_leveled`). Pinned to `benchmarks/silicon/fabric_grids.json` by `fabric_grids_lock`.
pub fn fabric_grids_json() -> serde_json::Value {
    use super::fabric_grid::{route_leveled, route_straight_line};
    use super::fabric_rtl::{poly_program, residual_from_source};
    use serde_json::json;
    let t = table();
    let islands: Vec<serde_json::Value> = t
        .grids
        .iter()
        .zip(FABRIC_SOURCES)
        .enumerate()
        .map(|(cfg, (g, (src, name)))| {
            let mut v = drawing_grid(name, src, g);
            v["cfg"] = json!(cfg);
            v
        })
        .collect();
    let poly_src = poly_program("Show poly(4).");
    let (ops, result, n_in) = residual_from_source(&poly_src, "poly");
    let cells: Vec<serde_json::Value> = ops
        .iter()
        .enumerate()
        .map(|(i, c)| json!({ "i": i, "op": format!("{:?}", c.op), "a": format!("{:?}", c.a), "b": format!("{:?}", c.b) }))
        .collect();
    json!({
        "config_fabric": {
            "rows": t.rows, "cols": t.cols, "cfg_width": t.cfg_width, "width": t.width,
            "islands": islands,
        },
        "horner": {
            "source": poly_src,
            "cells": cells,
            "result": result,
            "n_in": n_in,
            "diagonal": drawing_grid("horner_diagonal", &poly_src, &route_straight_line(&ops, n_in, result)),
            "leveled": drawing_grid("horner_leveled", &poly_src, &route_leveled(&ops, n_in, result)),
        },
    })
}
