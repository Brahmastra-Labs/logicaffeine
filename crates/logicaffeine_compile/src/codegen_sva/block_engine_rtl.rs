//! block_engine_rtl — Stage 5 of the self-synthesizing bytecode XPU: the on-chip EXECUTE engine for a
//! decoded block. Stage 4's reconfigurable fabric runs a block's certified SPATIAL placement (host-computed).
//! This engine takes a block's DECODED OPS DIRECTLY — no host placement step — and runs them on a universal
//! reconfigurable datapath: one hardware array executes ANY bounded straight-line block by loading its op
//! list as config. This is the unit the LGB1 decoder feeds: bytecode → ops → this engine → result, all
//! on-chip.
//!
//! Each cell `i` is `v[i] = op_i(operand_a, operand_b)` where an operand is a global input, a constant, or an
//! earlier cell's value `v[j]` (j<i) — selected by config muxes (a full-interconnect "crossbar" datapath, so
//! there is no routing to place: the ops ARE the config). Feed-forward (`v[i]` reads only `v[<i]`) → combinational.
//! The engine reproduces `eval_fabric` exactly, so it is proven ≡ the VM under iverilog by `block_engine_lock`.
//!
//! Correct-first scope: this is the full-interconnect realization (trivial placement, larger interconnect).
//! The AREA-OPTIMAL frontier is running these same ops through `route_loop_body`'s nearest-neighbour SPATIAL
//! placement on the Stage-4 fabric with an on-chip placer — the two meet when the placement decode moves into
//! silicon.

use super::fabric_rtl::{AluOp, FabricOp, Src};

/// A decoded block transcribed into the execute engine's config: one 32-bit control word per cell (op + the
/// two operand sources) plus two 64-bit constant slots per cell, and `result` (which cell's value is the
/// block result).
pub struct BlockConfig {
    pub ctrl: Vec<u32>,
    pub ac: Vec<i64>,
    pub bc: Vec<i64>,
    pub result: usize,
}

fn alu_code(op: AluOp) -> u32 {
    match op {
        AluOp::Add => 0,
        AluOp::Sub => 1,
        AluOp::Mul => 2,
        AluOp::And => 3,
        AluOp::Or => 4,
        AluOp::Xor => 5,
        AluOp::Shl => 6,
        AluOp::Shr => 7,
        AluOp::UShr => 8,
    }
}

/// Encode an operand as `(mode, ref, const)` — mode 0=In(ref), 1=Const(const), 2=Val(ref).
fn enc(s: Src) -> (u32, u32, i64) {
    match s {
        Src::In(k) => (0, k as u32, 0),
        Src::Const(c) => (1, 0, c),
        Src::Val(j) => (2, j as u32, 0),
    }
}

/// Transcribe a decoded block (`FabricOp` list + result index) into the engine's config, padded to `k` cells
/// (unused cells are inert — their value is never referenced).
pub fn serialize_block(ops: &[FabricOp], result: usize, k: usize) -> BlockConfig {
    assert!(
        ops.len() <= k,
        "block has {} cells > engine capacity {k}",
        ops.len()
    );
    let mut ctrl = vec![0u32; k];
    let mut ac = vec![0i64; k];
    let mut bc = vec![0i64; k];
    for (i, o) in ops.iter().enumerate() {
        let (am, ar, acv) = enc(o.a);
        let (bm, br, bcv) = enc(o.b);
        ctrl[i] = alu_code(o.op) | (am << 4) | (ar << 6) | (bm << 10) | (br << 12);
        ac[i] = acv;
        bc[i] = bcv;
    }
    BlockConfig {
        ctrl,
        ac,
        bc,
        result,
    }
}

/// A ternary chain selecting `in{k}` (0..n_in) by the 4-bit `field`.
fn in_mux(field: &str, n_in: usize) -> String {
    let mut acc = "64'sd0".to_string();
    for k in (0..n_in).rev() {
        acc = format!("({field}==4'd{k}) ? in{k} : ({acc})");
    }
    acc
}

/// A ternary chain selecting `v_{j}` (0..upto) by the 4-bit `field` — the prior-cell operand mux.
fn v_mux(field: &str, upto: usize) -> String {
    let mut acc = "64'sd0".to_string();
    for j in (0..upto).rev() {
        acc = format!("({field}==4'd{j}) ? v_{j} : ({acc})");
    }
    acc
}

/// Emit the universal block execute engine: a `k`-cell reconfigurable datapath over `n_in` inputs. Each cell
/// decodes its control word into an op + two operand selects (input / constant / earlier cell) and computes
/// combinationally; loading a decoded block's op list runs it. Config is read once via `$readmemh`.
pub fn emit_block_engine_verilog(name: &str, k: usize, n_in: usize) -> String {
    let in_ports: String = (0..n_in)
        .map(|j| format!("input signed [63:0] in{j},\n    "))
        .collect();

    let mut body = String::new();
    for i in 0..k {
        body.push_str(&format!("  wire [31:0] ct_{i} = ctrl[{i}];\n"));
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
        body.push_str(&format!(
            "  wire signed [63:0] opa_{i} = (ct_{i}[5:4]==2'd0) ? ({ain}) : (ct_{i}[5:4]==2'd1) ? ac[{i}] : ({av});\n"
        ));
        body.push_str(&format!(
            "  wire signed [63:0] opb_{i} = (ct_{i}[11:10]==2'd0) ? ({bin}) : (ct_{i}[11:10]==2'd1) ? bc[{i}] : ({bv});\n"
        ));
        body.push_str(&format!(
            "  wire signed [63:0] v_{i} = alu_f(ct_{i}[3:0], opa_{i}, opb_{i});\n"
        ));
    }
    // The result mux: v[result_ref] selected at runtime.
    let mut res_sel = "64'sd0".to_string();
    for i in (0..k).rev() {
        res_sel = format!("(res=={rw}'d{i}) ? v_{i} : ({res_sel})", rw = 16);
    }

    format!(
        "// bexec_{name} — a UNIVERSAL {k}-cell block EXECUTE engine over {n_in} inputs. Each cell's op and\n\
         // operand sources are RUNTIME config ($readmemh); loading a decoded block's op list runs it on ONE\n\
         // reconfigurable datapath. Feed-forward → combinational. Reproduces eval_fabric; proven ≡ the VM.\n\
         module bexec_{name}(\n    {in_ports}output signed [63:0] result\n);\n\
         \x20 reg [31:0] ctrl [0:{km1}];\n\
         \x20 reg signed [63:0] ac [0:{km1}];\n\
         \x20 reg signed [63:0] bc [0:{km1}];\n\
         \x20 reg [15:0] res; reg [63:0] rhdr [0:0];\n\
         \x20 initial begin\n\
         \x20   $readmemh(\"bexec_{name}_ctrl.hex\", ctrl);\n\
         \x20   $readmemh(\"bexec_{name}_ac.hex\", ac);\n\
         \x20   $readmemh(\"bexec_{name}_bc.hex\", bc);\n\
         \x20   $readmemh(\"bexec_{name}_res.hex\", rhdr);\n\
         \x20   res = rhdr[0][15:0];\n\
         \x20 end\n\
         \x20 function signed [63:0] alu_f(input [3:0] op, input signed [63:0] a, input signed [63:0] b);\n\
         \x20   case (op)\n\
         \x20     4'd0: alu_f = a + b; 4'd1: alu_f = a - b; 4'd2: alu_f = a * b;\n\
         \x20     4'd3: alu_f = a & b; 4'd4: alu_f = a | b; 4'd5: alu_f = a ^ b;\n\
         \x20     4'd6: alu_f = a <<< b; 4'd7: alu_f = a >>> b; 4'd8: alu_f = $unsigned(a) >> b; default: alu_f = 64'sd0;\n\
         \x20   endcase\n\
         \x20 endfunction\n\
         {body}  assign result = {res_sel};\n\
         endmodule\n",
        km1 = k - 1,
    )
}

/// A self-checking testbench: drive each input vector, compare `result` to the VM oracle in
/// `bexec_{name}_{pin,eout}.hex` (the engine loads its config from `bexec_{name}_{ctrl,ac,bc,res}.hex`).
pub fn emit_block_engine_testbench(name: &str, n_in: usize, n_vectors: usize) -> String {
    let decls: String = (0..n_in)
        .map(|k| format!("  reg signed [63:0] in{k};\n"))
        .collect();
    let ports: String = {
        let mut v: Vec<String> = (0..n_in).map(|k| format!(".in{k}(in{k})")).collect();
        v.push(".result(result)".to_string());
        v.join(", ")
    };
    let pin = if n_in > 0 {
        format!("  reg signed [63:0] pin [0:{}];\n", n_in * n_vectors - 1)
    } else {
        String::new()
    };
    let readpin = if n_in > 0 {
        format!("    $readmemh(\"bexec_{name}_pin.hex\", pin);\n")
    } else {
        String::new()
    };
    let drive: String = (0..n_in)
        .map(|k| format!("      in{k} = pin[i*{n_in}+{k}];\n"))
        .collect();

    format!(
        r#"// tb_bexec_{name} — the block execute engine (config = a decoded op list) vs the RUNNING VM.
`timescale 1ns/1ps
module tb_bexec_{name};
{decls}{pin}  reg signed [63:0] eout [0:{olast}];
  wire signed [63:0] result;
  integer i, errors;
  bexec_{name} dut({ports});
  initial begin
    errors = 0;
{readpin}    $readmemh("bexec_{name}_eout.hex", eout);
    for (i = 0; i < {n_vectors}; i = i + 1) begin
{drive}      #1;
      if (result !== eout[i]) begin errors = errors + 1; $display("FAIL case %0d: got %0d exp %0d", i, result, eout[i]); end
    end
    if (errors == 0) $display("PASS: {n_vectors} block-engine evaluations match the VM");
    else $display("FAIL: %0d block-engine mismatches", errors);
    $finish;
  end
endmodule
"#,
        olast = n_vectors - 1,
    )
}
