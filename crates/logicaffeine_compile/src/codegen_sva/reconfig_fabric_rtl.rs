//! reconfig_fabric_rtl — Stage 4 of the self-synthesizing bytecode XPU: the placer IN SILICON as a
//! RECONFIGURABLE fabric. Stage 1 HARDWIRED a grid per block; Stage 3 proved `route_loop_body` correct for
//! every bounded body. This stage makes the fabric CONFIG-DRIVEN: ONE fixed R×C tile array whose every
//! tile's operation and channel routing are RUNTIME inputs, so loading the certified placement of ANY block
//! runs it on the same physical silicon — the reconfigurable array the on-chip placer configures.
//!
//! A tile's config is a 32-bit control word (alu op + the two operand sources + the vs/he channel drives)
//! plus two 64-bit constant slots (for `Const` operands). `serialize_grid_config` transcribes a placed
//! `Grid` (from `route_loop_body`, padded to the array size) into that config; the emitted fabric decodes it
//! per tile and computes the same feed-forward dataflow the hardwired grid does. Proven ≡ the VM under
//! iverilog by `reconfig_fabric_lock`: for a corpus of straight-line programs, the certified placement loaded
//! into ONE universal array computes exactly what the VM computes.
//!
//! Correct-first scope: Arith-tile grids (the Stage-3 certified family) with a single `Val` output — the
//! spatial place-and-route the array runs is `route_loop_body`'s canonical placement. Select/Func/Mem tiles
//! and the on-chip placement of the config (the placement decode moving from host to chip) are the
//! follow-ons; here the certified placement is loaded, and the ARRAY is proven universal + reconfigurable.

use super::fabric_grid::{CellKind, Chan, Drive, Grid, Src2};
use super::fabric_rtl::AluOp;

/// A placed grid transcribed into the reconfigurable fabric's config: one 32-bit control word per tile, two
/// 64-bit constant slots per tile (operand `a`/`b` `Const` values), and the output tap (`out_sel` = flat tile
/// index, `out_chan` = 0 for the vertical channel / 1 for the horizontal).
pub struct ReconfigConfig {
    pub ctrl: Vec<u32>,
    pub sac: Vec<i64>,
    pub sbc: Vec<i64>,
    pub out_sel: usize,
    pub out_chan: u8,
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

/// Encode an operand source as `(mode, in_index, const)` — mode 0=Zero, 1=N, 2=W, 3=In, 4=Const.
fn enc_src2(s: Src2) -> (u32, u32, i64) {
    match s {
        Src2::Zero => (0, 0, 0),
        Src2::N => (1, 0, 0),
        Src2::W => (2, 0, 0),
        Src2::In(k) => (3, k as u32, 0),
        Src2::Const(c) => (4, 0, c),
    }
}

/// Encode a channel drive as a 2-bit mode: 0=Zero, 1=N, 2=W, 3=Alu. (Arith-body grids with a `Val` output
/// drive their channels only with these — a routed input/const in a channel is the In/Const-output case,
/// excluded from the Stage-4 correct-first scope.)
fn enc_drive(d: Drive) -> u32 {
    match d {
        Drive::Alu => 3,
        Drive::Pass(Src2::Zero) => 0,
        Drive::Pass(Src2::N) => 1,
        Drive::Pass(Src2::W) => 2,
        Drive::Pass(other) => panic!(
            "reconfig fabric: channel drive Pass({other:?}) is outside the correct-first scope \
             (Arith-body grids drive channels with Zero/N/W/Alu only)"
        ),
    }
}

/// Transcribe a placed `Grid` (Arith tiles, single `Val` output) into the reconfigurable fabric's config,
/// laid out row-major over `rows*cols` tiles (the grid must already be padded to `rows`×`cols`).
pub fn serialize_grid_config(g: &Grid) -> ReconfigConfig {
    let rc = g.rows * g.cols;
    let mut ctrl = vec![0u32; rc];
    let mut sac = vec![0i64; rc];
    let mut sbc = vec![0i64; rc];
    for r in 0..g.rows {
        for c in 0..g.cols {
            let idx = r * g.cols + c;
            let t = g.tiles[r][c];
            let (op, sam, sai, sac_v, sbm, sbi, sbc_v) = match t.alu {
                None => (0u32, 0u32, 0u32, 0i64, 0u32, 0u32, 0i64),
                Some(CellKind::Arith(op, a, b)) => {
                    let (am, ai, ac) = enc_src2(a);
                    let (bm, bi, bc) = enc_src2(b);
                    (alu_code(op), am, ai, ac, bm, bi, bc)
                }
                Some(other) => panic!(
                    "reconfig fabric: tile {other:?} is outside the Stage-4 correct-first scope (Arith grids)"
                ),
            };
            let vs = enc_drive(t.vs);
            let he = enc_drive(t.he);
            ctrl[idx] =
                op | (sam << 4) | (sai << 7) | (sbm << 11) | (sbi << 14) | (vs << 18) | (he << 20);
            sac[idx] = sac_v;
            sbc[idx] = sbc_v;
        }
    }
    let (or, oc, och) = g.out;
    ReconfigConfig {
        ctrl,
        sac,
        sbc,
        out_sel: or * g.cols + oc,
        out_chan: if matches!(och, Chan::He) { 1 } else { 0 },
    }
}

/// The ternary chain selecting `in{k}` by the 4-bit `in_index` field `field` of a tile's control word.
fn in_mux(field: &str, n_in: usize) -> String {
    if n_in == 0 {
        return "64'sd0".to_string();
    }
    let mut acc = "64'sd0".to_string();
    for k in (0..n_in).rev() {
        acc = format!("({field}==4'd{k}) ? in{k} : ({acc})");
    }
    acc
}

/// Emit the universal reconfigurable fabric: a fixed `rows`×`cols` config-driven CGRA. Each tile decodes its
/// control word (loaded via `$readmemh`) into an operation + channel routing and computes the same
/// feed-forward dataflow the hardwired grid does; loading `route_loop_body`'s certified placement runs any
/// bounded Arith block on this ONE array. Combinational (feed-forward south/east) — the answer settles in
/// zero clocks; the config is read once at elaboration.
pub fn emit_reconfig_fabric_verilog(name: &str, rows: usize, cols: usize, n_in: usize) -> String {
    let rc = rows * cols;
    let in_ports: String = (0..n_in)
        .map(|k| format!("input signed [63:0] in{k},\n    "))
        .collect();

    let mut body = String::new();
    for r in 0..rows {
        for c in 0..cols {
            let idx = r * cols + c;
            let nn = if r == 0 {
                "64'sd0".to_string()
            } else {
                format!("vs_{}_{c}", r - 1)
            };
            let ww = if c == 0 {
                "64'sd0".to_string()
            } else {
                format!("he_{r}_{}", c - 1)
            };
            body.push_str(&format!("  wire [31:0] ct_{r}_{c} = ctrl[{idx}];\n"));
            let ain = in_mux(&format!("ct_{r}_{c}[10:7]"), n_in);
            let bin = in_mux(&format!("ct_{r}_{c}[17:14]"), n_in);
            body.push_str(&format!(
                "  wire signed [63:0] opa_{r}_{c} = (ct_{r}_{c}[6:4]==3'd0) ? 64'sd0 : (ct_{r}_{c}[6:4]==3'd1) ? {nn} : (ct_{r}_{c}[6:4]==3'd2) ? {ww} : (ct_{r}_{c}[6:4]==3'd3) ? ({ain}) : sac[{idx}];\n"
            ));
            body.push_str(&format!(
                "  wire signed [63:0] opb_{r}_{c} = (ct_{r}_{c}[13:11]==3'd0) ? 64'sd0 : (ct_{r}_{c}[13:11]==3'd1) ? {nn} : (ct_{r}_{c}[13:11]==3'd2) ? {ww} : (ct_{r}_{c}[13:11]==3'd3) ? ({bin}) : sbc[{idx}];\n"
            ));
            body.push_str(&format!(
                "  wire signed [63:0] al_{r}_{c} = alu_f(ct_{r}_{c}[3:0], opa_{r}_{c}, opb_{r}_{c});\n"
            ));
            body.push_str(&format!(
                "  wire signed [63:0] vs_{r}_{c} = drive_f(ct_{r}_{c}[19:18], {nn}, {ww}, al_{r}_{c});\n"
            ));
            body.push_str(&format!(
                "  wire signed [63:0] he_{r}_{c} = drive_f(ct_{r}_{c}[21:20], {nn}, {ww}, al_{r}_{c});\n"
            ));
        }
    }

    // The output tap: a mux over all tiles selected by the runtime out_sel + out_chan.
    let mut vs_sel = "64'sd0".to_string();
    let mut he_sel = "64'sd0".to_string();
    for r in (0..rows).rev() {
        for c in (0..cols).rev() {
            let idx = r * cols + c;
            vs_sel = format!(
                "(out_sel=={rc_w}'d{idx}) ? vs_{r}_{c} : ({vs_sel})",
                rc_w = 16
            );
            he_sel = format!(
                "(out_sel=={rc_w}'d{idx}) ? he_{r}_{c} : ({he_sel})",
                rc_w = 16
            );
        }
    }

    format!(
        "// reconfig_{name} — a UNIVERSAL config-driven CGRA ({rows}x{cols}). Every tile's operation and\n\
         // channel routing are RUNTIME config (loaded via $readmemh); loading route_loop_body's certified\n\
         // placement (Stage 3) runs ANY bounded Arith block on ONE physical array. Feed-forward → combinational.\n\
         module reconfig_{name}(\n    {in_ports}output signed [63:0] result\n);\n\
         \x20 reg [31:0] ctrl [0:{rcm1}];\n\
         \x20 reg signed [63:0] sac [0:{rcm1}];\n\
         \x20 reg signed [63:0] sbc [0:{rcm1}];\n\
         \x20 reg [63:0] ohdr [0:1];\n\
         \x20 reg [15:0] out_sel; reg out_chan;\n\
         \x20 initial begin\n\
         \x20   $readmemh(\"reconfig_{name}_ctrl.hex\", ctrl);\n\
         \x20   $readmemh(\"reconfig_{name}_sac.hex\", sac);\n\
         \x20   $readmemh(\"reconfig_{name}_sbc.hex\", sbc);\n\
         \x20   $readmemh(\"reconfig_{name}_out.hex\", ohdr);\n\
         \x20   out_sel = ohdr[0][15:0]; out_chan = ohdr[1][0];\n\
         \x20 end\n\
         \x20 function signed [63:0] alu_f(input [3:0] op, input signed [63:0] a, input signed [63:0] b);\n\
         \x20   case (op)\n\
         \x20     4'd0: alu_f = a + b; 4'd1: alu_f = a - b; 4'd2: alu_f = a * b;\n\
         \x20     4'd3: alu_f = a & b; 4'd4: alu_f = a | b; 4'd5: alu_f = a ^ b;\n\
         \x20     4'd6: alu_f = a <<< b; 4'd7: alu_f = a >>> b; 4'd8: alu_f = $unsigned(a) >> b; default: alu_f = 64'sd0;\n\
         \x20   endcase\n\
         \x20 endfunction\n\
         \x20 function signed [63:0] drive_f(input [1:0] m, input signed [63:0] nn, input signed [63:0] ww, input signed [63:0] al);\n\
         \x20   case (m) 2'd0: drive_f = 64'sd0; 2'd1: drive_f = nn; 2'd2: drive_f = ww; 2'd3: drive_f = al; default: drive_f = 64'sd0; endcase\n\
         \x20 endfunction\n\
         {body}  assign result = out_chan ? ({he_sel}) : ({vs_sel});\n\
         endmodule\n",
        rcm1 = rc - 1,
    )
}

/// A self-checking testbench for the reconfigurable fabric: drive each input vector and compare `result`
/// to the VM oracle in `reconfig_{name}_{pin,eout}.hex` (the fabric loads its config from
/// `reconfig_{name}_{ctrl,sac,sbc,out}.hex`). One block's config + vectors are written per run; the fabric
/// RTL is fixed, so it compiles once and re-runs per block.
pub fn emit_reconfig_testbench(name: &str, n_in: usize, n_vectors: usize) -> String {
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
        format!("    $readmemh(\"reconfig_{name}_pin.hex\", pin);\n")
    } else {
        String::new()
    };
    let drive: String = (0..n_in)
        .map(|k| format!("      in{k} = pin[i*{n_in}+{k}];\n"))
        .collect();

    format!(
        r#"// tb_reconfig_{name} — the reconfigurable fabric (config = a certified placement) vs the RUNNING VM.
`timescale 1ns/1ps
module tb_reconfig_{name};
{decls}{pin}  reg signed [63:0] eout [0:{olast}];
  wire signed [63:0] result;
  integer i, errors;
  reconfig_{name} dut({ports});
  initial begin
    errors = 0;
{readpin}    $readmemh("reconfig_{name}_eout.hex", eout);
    for (i = 0; i < {n_vectors}; i = i + 1) begin
{drive}      #1;
      if (result !== eout[i]) begin errors = errors + 1; $display("FAIL case %0d: got %0d exp %0d", i, result, eout[i]); end
    end
    if (errors == 0) $display("PASS: {n_vectors} reconfigurable-fabric evaluations match the VM");
    else $display("FAIL: %0d reconfig mismatches", errors);
    $finish;
  end
endmodule
"#,
        olast = n_vectors - 1,
    )
}
