//! sequencer_rtl — Stage 1 of the self-synthesizing bytecode XPU: the AUTONOMOUS block-walk sequencer as
//! synthesizable RTL. The CONTROL PLANE that `run_program_cfg_on_grid` modelled in software now exists in
//! silicon: a register file that survives block reconfiguration, per-block placed grids, a block program-
//! counter, a terminator FSM (Jump/Branch/Return), and a cycle counter — a hardwired micro-sequencer that
//! walks the whole function itself, one block per clock, with no host help.
//!
//! It is the RTL of exactly `run_program_cfg_on_grid`'s loop (`fabric_grid.rs`): each basic block's
//! straight-line body is PLACED on a fresh tile array by `route_loop_body` (the block's live-in registers as
//! inputs; its register writes then its terminator operand as the grid's outputs, in that order — the
//! `all_outs()` order the placer guarantees). Every block's grid is a combinational island wired to the
//! CURRENT register file; on each clock the sequencer latches the ACTIVE block's writes back into the file
//! and follows its terminator: a `Jump` sets the next block, a `Branch` lets the block's predicate output
//! pick the successor, a `Return` captures the operand and halts. So the whole branchy program runs as
//! spatial dataflow driven by a tiny sequencer — proven bit-identical to `run_program_cfg_on_grid`
//! (≡ `eval_program_cfg_fabric` ≡ the VM) under iverilog by `fabric_sequencer_lock`.
//!
//! The block grids are emitted by the existing `emit_grid_verilog` (multi-output form); this module adds
//! only the register file + sequencing FSM around them. Blocks are Arith/Select/Func islands (the stateless
//! control plane); the heap state plane is the Stage-2 follow-on (`sequencer_rtl` gains a shared BRAM there).

use super::fabric_grid::{emit_grid_verilog, eval_grid_all_with_heap, route_loop_body, Grid};
use super::fabric_rtl::{ProgramCfgFabric, Src, Term};

/// Evaluate a whole-function CFG fabric on the sequencer's model and return `(result, block_steps)`. This IS
/// `run_program_cfg_on_grid` — the same place-per-block (`route_loop_body`) + `eval_grid_all` walk over a
/// register file — with a step counter added, so the returned value is identical to the grid walk (≡ the CFG
/// model ≡ the VM) and `block_steps` is the exact number of block executions the RTL must take (the
/// trajectory, not merely the answer). The reference the iverilog differential checks against.
pub fn eval_sequencer(pf: &ProgramCfgFabric, params: &[i64]) -> (i64, u64) {
    use std::collections::HashMap;
    let mut file: HashMap<u16, i64> = (0..pf.n_params).map(|p| (p as u16, params[p])).collect();
    // The shared state-plane heap, threaded across every block exactly as `run_program_cfg_on_grid` does.
    let mut heap: Vec<i64> = Vec::new();
    let mut blk = 0usize;
    let mut steps = 0u64;
    for _ in 0..10_000_000 {
        steps += 1;
        let b = &pf.blocks[blk];
        let inputs: Vec<i64> = b
            .inputs
            .iter()
            .map(|r| *file.get(r).unwrap_or(&0))
            .collect();
        let mut needed: Vec<Src> = b.writes.iter().map(|(_, s)| *s).collect();
        let term_off = needed.len();
        match &b.term {
            Term::Jump(_) => {}
            Term::Branch { cond, .. } => needed.push(*cond),
            Term::Return(s) => needed.push(*s),
        }
        let g = route_loop_body(&b.body, b.inputs.len(), &needed);
        let outs = eval_grid_all_with_heap(&g, &inputs, &mut heap);
        for (i, (r, _)) in b.writes.iter().enumerate() {
            file.insert(*r, outs[i]);
        }
        match &b.term {
            Term::Jump(t) => blk = *t,
            Term::Branch {
                then_blk, else_blk, ..
            } => {
                blk = if outs[term_off] != 0 {
                    *then_blk
                } else {
                    *else_blk
                }
            }
            Term::Return(_) => return (outs[term_off], steps),
        }
    }
    panic!("sequencer: controller did not return within the step budget (a back-edge slipped through?)")
}

/// The register-file width: one 64-bit word per distinct register the program touches (live-ins + writes),
/// never fewer than the parameter count (params occupy `file[0..n_params]`). At least one word.
fn n_regs(pf: &ProgramCfgFabric) -> usize {
    let mut max_reg = 0u16;
    for b in &pf.blocks {
        for r in &b.inputs {
            max_reg = max_reg.max(*r);
        }
        for (r, _) in &b.writes {
            max_reg = max_reg.max(*r);
        }
    }
    (max_reg as usize + 1).max(pf.n_params).max(1)
}

/// Smallest bit width that can index `n` blocks (≥ 1).
fn clog2(n: usize) -> usize {
    let mut b = 1;
    while (1usize << b) < n {
        b += 1;
    }
    b
}

/// The `next`/output list for a block: each register write's `Src`, then (last) the terminator's
/// predicate/return operand — exactly the order `run_program_cfg_on_grid` places as the grid's outputs, so
/// `all_outs()[i]` is write `i` and `all_outs()[term_off]` is the terminator operand.
fn block_needed(b: &super::fabric_rtl::CfgBlock) -> (Vec<Src>, usize) {
    let mut needed: Vec<Src> = b.writes.iter().map(|(_, s)| *s).collect();
    let term_off = needed.len();
    match &b.term {
        Term::Jump(_) => {}
        Term::Branch { cond, .. } => needed.push(*cond),
        Term::Return(s) => needed.push(*s),
    }
    (needed, term_off)
}

/// Emit the autonomous block-walk sequencer for a whole-function CFG fabric: the per-block placed-grid
/// modules followed by a `seq_{name}` top that carries the register file + block-PC + terminator FSM +
/// cycle counter. One block step per clock; `done` pulses high with the return value on `result` and the
/// block-step count on `cycles`.
pub fn emit_sequencer_verilog(name: &str, pf: &ProgramCfgFabric) -> String {
    let nb = pf.blocks.len();
    assert!(nb >= 1, "sequencer: a program needs at least one block");
    let nreg = n_regs(pf);
    let pcw = clog2(nb);

    // Per-block placed grid + its output arity.
    let mut grids: Vec<(Grid, usize, usize)> = Vec::with_capacity(nb); // (grid, term_off, n_out)
    let mut modules = String::new();
    for (k, b) in pf.blocks.iter().enumerate() {
        let (needed, term_off) = block_needed(b);
        assert!(
            !needed.is_empty(),
            "sequencer: block {k} of `{name}` has no outputs (no register writes and an unconditional Jump) — \
             the honest bound (route_loop_body needs ≥1 output tap); a real block writes a register or ends in \
             a conditional/return terminator"
        );
        let g = route_loop_body(&b.body, b.inputs.len(), &needed);
        let n_out = g.all_outs().len();
        modules.push_str(&emit_grid_verilog(&format!("seq_{name}_b{k}"), &g));
        modules.push('\n');
        grids.push((g, term_off, n_out));
    }

    // seq_{name} port list.
    let mut ports: Vec<String> = vec!["input clk".into(), "input rst".into()];
    for p in 0..pf.n_params {
        ports.push(format!("input [63:0] p{p}"));
    }
    ports.push("output [63:0] result".into());
    ports.push("output done".into());
    ports.push("output [63:0] cycles".into());

    let mut body = String::new();
    body.push_str(&format!("  reg [63:0] file [0:{}];\n", nreg - 1));
    body.push_str(&format!("  reg [{}:0] pc;\n", pcw - 1));
    body.push_str("  reg done_r;\n  reg [63:0] result_r;\n  reg [63:0] cyc_r;\n");
    body.push_str(
        "  assign done = done_r;\n  assign result = result_r;\n  assign cycles = cyc_r;\n\n",
    );

    // Continuous reads of the register file — clean nets for the block-grid input ports.
    for r in 0..nreg {
        body.push_str(&format!("  wire [63:0] fr_{r} = file[{r}];\n"));
    }
    body.push('\n');

    // Per-block combinational grid instances, wired from the current register file.
    for (k, b) in pf.blocks.iter().enumerate() {
        let (_g, _term_off, n_out) = &grids[k];
        for o in 0..*n_out {
            body.push_str(&format!("  wire [63:0] b{k}_o{o};\n"));
        }
        let mut conns: Vec<String> = Vec::new();
        for (j, r) in b.inputs.iter().enumerate() {
            conns.push(format!(".in{j}(fr_{r})"));
        }
        if *n_out == 1 {
            conns.push(format!(".result(b{k}_o0)"));
        } else {
            for o in 0..*n_out {
                conns.push(format!(".out{o}(b{k}_o{o})"));
            }
        }
        body.push_str(&format!(
            "  grid_seq_{name}_b{k} u_b{k} ({});\n\n",
            conns.join(", ")
        ));
    }

    // The sequencing FSM.
    body.push_str("  always @(posedge clk) begin\n");
    body.push_str("    if (rst) begin\n");
    body.push_str(&format!("      pc <= {pcw}'d0;\n      done_r <= 1'b0;\n      result_r <= 64'd0;\n      cyc_r <= 64'd0;\n"));
    for r in 0..nreg {
        if r < pf.n_params {
            body.push_str(&format!("      file[{r}] <= p{r};\n"));
        } else {
            body.push_str(&format!("      file[{r}] <= 64'd0;\n"));
        }
    }
    body.push_str("    end else if (!done_r) begin\n");
    body.push_str("      cyc_r <= cyc_r + 64'd1;\n");
    body.push_str("      case (pc)\n");
    for (k, b) in pf.blocks.iter().enumerate() {
        let (_g, term_off, _n_out) = &grids[k];
        body.push_str(&format!("        {pcw}'d{k}: begin\n"));
        for (i, (r, _)) in b.writes.iter().enumerate() {
            body.push_str(&format!("          file[{r}] <= b{k}_o{i};\n"));
        }
        match &b.term {
            Term::Jump(t) => {
                body.push_str(&format!("          pc <= {pcw}'d{t};\n"));
            }
            Term::Branch {
                then_blk, else_blk, ..
            } => {
                body.push_str(&format!(
                    "          pc <= (b{k}_o{term_off} != 64'd0) ? {pcw}'d{then_blk} : {pcw}'d{else_blk};\n"
                ));
            }
            Term::Return(_) => {
                body.push_str(&format!(
                    "          result_r <= b{k}_o{term_off};\n          done_r <= 1'b1;\n"
                ));
            }
        }
        body.push_str("        end\n");
    }
    body.push_str("        default: done_r <= 1'b1;\n");
    body.push_str("      endcase\n");
    body.push_str("    end\n  end\n");

    format!(
        "// seq_{name} — the AUTONOMOUS block-walk sequencer: a register file + block program-counter +\n\
         // terminator FSM (Jump/Branch/Return) + cycle counter around {nb} per-block placed grids. Walks the\n\
         // whole function itself, one block per clock, ≡ run_program_cfg_on_grid ≡ the VM.\n\
         {modules}\
         module seq_{name}(\n    {ports}\n);\n{body}endmodule\n",
        ports = ports.join(",\n    "),
    )
}

/// A self-checking testbench for the sequencer: for each vector, load the params, pulse reset, run the FSM
/// to `done`, and compare BOTH `result` and `cycles` to the VM oracle in `seq_{name}_{out,cyc}.hex`
/// (inputs in `seq_{name}_in.hex`). The step-count check pins the trajectory, not just the answer.
pub fn emit_sequencer_testbench(name: &str, n_params: usize, n_vectors: usize) -> String {
    let mut decls = String::new();
    if n_params > 0 {
        decls.push_str(&format!(
            "  reg [63:0] pin [0:{}];\n",
            n_params * n_vectors - 1
        ));
    }
    decls.push_str(&format!("  reg [63:0] oout [0:{}];\n", n_vectors - 1));
    decls.push_str(&format!("  reg [63:0] ocyc [0:{}];\n", n_vectors - 1));
    for p in 0..n_params {
        decls.push_str(&format!("  reg [63:0] p{p};\n"));
    }

    let mut dut_ports: Vec<String> = vec![".clk(clk)".into(), ".rst(rst)".into()];
    for p in 0..n_params {
        dut_ports.push(format!(".p{p}(p{p})"));
    }
    dut_ports.push(".result(result)".into());
    dut_ports.push(".done(done)".into());
    dut_ports.push(".cycles(cycles)".into());

    let readin = if n_params > 0 {
        format!("    $readmemh(\"seq_{name}_in.hex\", pin);\n")
    } else {
        String::new()
    };
    let mut load = String::new();
    for p in 0..n_params {
        load.push_str(&format!("      p{p} = pin[i*{n_params}+{p}];\n"));
    }

    format!(
        r#"// tb_seq_{name} — the autonomous sequencer vs the RUNNING VM (result + exact block-step count).
`timescale 1ns/1ps
module tb_seq_{name};
  reg clk = 1'b0;
  reg rst;
{decls}  wire [63:0] result;
  wire done;
  wire [63:0] cycles;
  integer i, errors, guard;
  seq_{name} dut({ports});
  always #5 clk = ~clk;
  initial begin
    errors = 0;
{readin}    $readmemh("seq_{name}_out.hex", oout);
    $readmemh("seq_{name}_cyc.hex", ocyc);
    for (i = 0; i < {n_vectors}; i = i + 1) begin
{load}      rst = 1'b1; @(posedge clk); #1;
      rst = 1'b0;
      guard = 0;
      while (done !== 1'b1 && guard < 100000) begin @(posedge clk); #1; guard = guard + 1; end
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL case %0d: result got %0d exp %0d", i, result, oout[i]); end
      if (cycles !== ocyc[i]) begin errors = errors + 1; $display("FAIL case %0d: cycles got %0d exp %0d", i, cycles, ocyc[i]); end
    end
    if (errors == 0) $display("PASS: {n_vectors} sequencer runs match the VM (result + cycle count)");
    else $display("FAIL: %0d sequencer mismatches", errors);
    $finish;
  end
endmodule
"#,
        ports = dut_ports.join(", "),
    )
}
