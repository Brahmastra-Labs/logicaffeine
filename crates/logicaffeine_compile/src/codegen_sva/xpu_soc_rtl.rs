//! xpu_soc_rtl — Stage 6 of the self-synthesizing bytecode XPU: the SoC that receives a block's BYTECODE OVER
//! THE WIRE, parses it on-chip, and runs it on the reconfigurable execute engine — the north star made
//! physical (in simulation). A block's decoded op list is serialized to a byte stream (the wire format); the
//! SoC clocks the bytes in one at a time into an on-chip buffer, assembles them into the engine's config +
//! inputs, and the engine computes the result — no host help beyond the bytes on the wire.
//!
//! Wire format (big-endian), matching `block_stream_bytes`: for each of `k` cells, `[ctrl:4][a_const:8]
//! [b_const:8]` (20 bytes), then `[result_idx:2]`, then `in[j]:8` for each of `n_in` inputs. The SoC counts
//! bytes into a buffer, raises `done` when the whole frame has arrived, and drives `result` combinationally
//! from the assembled config. This is `nexys_soc.v`'s load-then-run pattern, but the payload configures the
//! spatial execute engine instead of the sequential soft-core.
//!
//! Proven ≡ the VM under iverilog by `xpu_soc_lock`: for a corpus, streaming a program's bytecode bytes makes
//! the SoC compute exactly what the VM computes. (The engine compute mirrors `block_engine_rtl` — a candidate
//! for a shared core once the SoC grows the sequencer + state-plane BRAM.)

use super::block_engine_rtl::BlockConfig;

/// Bytes per cell in the wire frame: 4 (ctrl) + 8 (a const) + 8 (b const).
const CELL_BYTES: usize = 20;

/// Serialize a block's config + one input vector into the SoC's big-endian wire byte stream.
pub fn block_stream_bytes(cfg: &BlockConfig, inputs: &[i64], k: usize, n_in: usize) -> Vec<u8> {
    assert_eq!(cfg.ctrl.len(), k, "config must be padded to k cells");
    let mut out = Vec::with_capacity(k * CELL_BYTES + 2 + n_in * 8);
    for i in 0..k {
        out.extend_from_slice(&cfg.ctrl[i].to_be_bytes()); // 4
        out.extend_from_slice(&(cfg.ac[i] as u64).to_be_bytes()); // 8
        out.extend_from_slice(&(cfg.bc[i] as u64).to_be_bytes()); // 8
    }
    out.extend_from_slice(&(cfg.result as u16).to_be_bytes()); // 2
    for j in 0..n_in {
        let v = if j < inputs.len() { inputs[j] } else { 0 };
        out.extend_from_slice(&(v as u64).to_be_bytes()); // 8
    }
    out
}

/// A big-endian byte-concatenation `{mem[base], mem[base+1], ...}` of `n` bytes → a `[8n-1:0]` value.
fn be_bytes(base: usize, n: usize) -> String {
    let parts: Vec<String> = (0..n).map(|b| format!("mem[{}]", base + b)).collect();
    format!("{{{}}}", parts.join(", "))
}

fn in_mux(field: &str, n_in: usize) -> String {
    let mut acc = "64'sd0".to_string();
    for j in (0..n_in).rev() {
        acc = format!("({field}==4'd{j}) ? iv_{j} : ({acc})");
    }
    acc
}

fn v_mux(field: &str, upto: usize) -> String {
    let mut acc = "64'sd0".to_string();
    for j in (0..upto).rev() {
        acc = format!("({field}==4'd{j}) ? v_{j} : ({acc})");
    }
    acc
}

/// Emit the XPU SoC: a byte-stream loader (`byte_in`/`byte_valid`) filling an on-chip frame buffer, on-chip
/// assembly of the engine's config + inputs, and the `k`-cell execute engine — `done` + `result` when the
/// whole frame has arrived.
pub fn emit_xpu_soc_verilog(name: &str, k: usize, n_in: usize) -> String {
    let total = k * CELL_BYTES + 2 + n_in * 8;
    let res_base = k * CELL_BYTES;
    let in_base = res_base + 2;

    let mut asm = String::new();
    // Per-cell config assembled from the frame buffer (big-endian).
    for i in 0..k {
        let base = i * CELL_BYTES;
        asm.push_str(&format!("  wire [31:0] ct_{i} = {};\n", be_bytes(base, 4)));
        asm.push_str(&format!(
            "  wire signed [63:0] ac_{i} = {};\n",
            be_bytes(base + 4, 8)
        ));
        asm.push_str(&format!(
            "  wire signed [63:0] bc_{i} = {};\n",
            be_bytes(base + 12, 8)
        ));
    }
    asm.push_str(&format!("  wire [15:0] res = {};\n", be_bytes(res_base, 2)));
    for j in 0..n_in {
        asm.push_str(&format!(
            "  wire signed [63:0] iv_{j} = {};\n",
            be_bytes(in_base + j * 8, 8)
        ));
    }

    // The execute engine over the assembled config (mirrors block_engine_rtl).
    let mut comp = String::new();
    for i in 0..k {
        let ain = in_mux(&format!("ct_{i}[9:6]"), n_in);
        let bin = in_mux(&format!("ct_{i}[15:12]"), n_in);
        let av = if i == 0 {
            "64'sd0".to_string()
        } else {
            v_mux(&format!("ct_{i}[9:6]"), i)
        };
        let bv = if i == 0 {
            "64'sd0".to_string()
        } else {
            v_mux(&format!("ct_{i}[15:12]"), i)
        };
        comp.push_str(&format!(
            "  wire signed [63:0] opa_{i} = (ct_{i}[5:4]==2'd0) ? ({ain}) : (ct_{i}[5:4]==2'd1) ? ac_{i} : ({av});\n"
        ));
        comp.push_str(&format!(
            "  wire signed [63:0] opb_{i} = (ct_{i}[11:10]==2'd0) ? ({bin}) : (ct_{i}[11:10]==2'd1) ? bc_{i} : ({bv});\n"
        ));
        comp.push_str(&format!(
            "  wire signed [63:0] v_{i} = alu_f(ct_{i}[3:0], opa_{i}, opb_{i});\n"
        ));
    }
    let mut res_sel = "64'sd0".to_string();
    for i in (0..k).rev() {
        res_sel = format!("(res==16'd{i}) ? v_{i} : ({res_sel})");
    }

    format!(
        "// xpu_{name} — the self-synthesizing bytecode XPU SoC. A block's bytecode is streamed in byte-by-byte\n\
         // (byte_in/byte_valid), buffered on-chip, assembled into the reconfigurable engine's config + inputs,\n\
         // and RUN — bytecode over the wire → result, no host help. (nexys_soc's load-then-run, payload = the\n\
         // spatial execute engine's config instead of soft-core opcodes.) Proven ≡ VM by xpu_soc_lock.\n\
         module xpu_{name}(input clk, input rst, input [7:0] byte_in, input byte_valid,\n\
         \x20   output signed [63:0] result, output done);\n\
         \x20 localparam integer TOTAL = {total};\n\
         \x20 reg [7:0] mem [0:{tm1}];\n\
         \x20 reg [15:0] cnt;\n\
         \x20 reg loaded;\n\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin cnt <= 16'd0; loaded <= 1'b0; end\n\
         \x20   else if (!loaded && byte_valid) begin\n\
         \x20     mem[cnt] <= byte_in;\n\
         \x20     if (cnt == TOTAL-1) loaded <= 1'b1;\n\
         \x20     cnt <= cnt + 16'd1;\n\
         \x20   end\n\
         \x20 end\n\
         \x20 assign done = loaded;\n\
         \x20 function signed [63:0] alu_f(input [3:0] op, input signed [63:0] a, input signed [63:0] b);\n\
         \x20   case (op)\n\
         \x20     4'd0: alu_f = a + b; 4'd1: alu_f = a - b; 4'd2: alu_f = a * b;\n\
         \x20     4'd3: alu_f = a & b; 4'd4: alu_f = a | b; 4'd5: alu_f = a ^ b;\n\
         \x20     4'd6: alu_f = a <<< b; 4'd7: alu_f = a >>> b; default: alu_f = 64'sd0;\n\
         \x20   endcase\n\
         \x20 endfunction\n\
         {asm}{comp}  assign result = {res_sel};\n\
         endmodule\n",
        tm1 = total - 1,
    )
}

/// A self-checking testbench: clock a program's bytecode frame in byte-by-byte, wait for `done`, and compare
/// `result` to the VM oracle. One frame carries the config + one input vector, so a frame is streamed per
/// (program, input) case; `rst` clears the loader between frames.
pub fn emit_xpu_soc_testbench(name: &str, n_cases: usize, frame_len: usize) -> String {
    format!(
        r#"// tb_xpu_{name} — stream each program's bytecode frame in over the wire; result must equal the VM.
`timescale 1ns/1ps
module tb_xpu_{name};
  reg clk = 1'b0;
  reg rst;
  reg [7:0] byte_in;
  reg byte_valid;
  wire signed [63:0] result;
  wire done;
  reg [7:0] frames [0:{flast}];   // n_cases frames concatenated, {frame_len} bytes each
  reg signed [63:0] eout [0:{clast}];
  integer i, b, errors, guard;
  xpu_{name} dut(.clk(clk), .rst(rst), .byte_in(byte_in), .byte_valid(byte_valid), .result(result), .done(done));
  always #5 clk = ~clk;
  initial begin
    errors = 0;
    $readmemh("xpu_{name}_frames.hex", frames);
    $readmemh("xpu_{name}_eout.hex", eout);
    for (i = 0; i < {n_cases}; i = i + 1) begin
      rst = 1'b1; byte_valid = 1'b0; @(posedge clk); #1;
      rst = 1'b0;
      for (b = 0; b < {frame_len}; b = b + 1) begin
        byte_in = frames[i*{frame_len}+b]; byte_valid = 1'b1; @(posedge clk); #1;
      end
      byte_valid = 1'b0;
      guard = 0;
      while (done !== 1'b1 && guard < 100000) begin @(posedge clk); #1; guard = guard + 1; end
      #1;
      if (result !== eout[i]) begin errors = errors + 1; $display("FAIL case %0d: got %0d exp %0d", i, result, eout[i]); end
    end
    if (errors == 0) $display("PASS: {n_cases} bytecode frames run on the XPU SoC match the VM");
    else $display("FAIL: %0d XPU SoC mismatches", errors);
    $finish;
  end
endmodule
"#,
        flast = n_cases * frame_len - 1,
        clast = n_cases - 1,
    )
}
