//! dae — Decoupled Access-Execute for the spatial fabric (`FabricTarget::agu`).
//!
//! HiVec keeps its PE array fed by splitting address generation out of the compute path. Our fabric's analog:
//! a straight-line body's ADDRESS-generation cells — the untainted arithmetic that feeds a memory tile's
//! address, whose dependency cone never crosses a load — are hoisted into a parallel **AGU lane**, while the
//! compute + memory tiles stay in the **exec lane** in program order. The AGU runs concurrently (a decoupling
//! queue hides the load), so the exec lane's critical path no longer includes the address arithmetic:
//! `decoupled_critical_depth = max(agu_lane_depth, exec_lane_depth) ≤ coupled_critical_depth`, strictly less
//! whenever an address chain fed the compute path. The transform is a pure topological re-layout of the SAME
//! cells (memory ops keep their relative order), so it is semantics-preserving by construction — proven by
//! `dae_equiv_lock` (eval ≡ the coupled body) and measured by `dae_perf_lock` (the depth ratchet).

use super::fabric_rtl::{
    emit_state_plane_verilog, eval_decision_fabric_heap, DecisionFabric, LoopCell, Src,
};
use super::fabric_target::FabricTarget;

/// A decoupled schedule: the SAME cells re-laid-out as `[agu lane .. | exec lane ..]` (AGU cells first), with
/// every `Src::Val` remapped to the new positions. `agu_len` cells form the address-generation lane; the rest
/// are the exec lane (compute + memory tiles, memory ops preserving program order).
#[derive(Clone, Debug)]
pub struct DaeSchedule {
    pub n_in: usize,
    pub layout: Vec<LoopCell>,
    pub result: Src,
    /// The number of leading `layout` cells that form the AGU lane (`0` ⇒ nothing decoupled).
    pub agu_len: usize,
    /// For each `layout` cell, whether it belongs to the AGU lane (`true`) or the exec lane (`false`).
    pub is_agu: Vec<bool>,
}

impl DaeSchedule {
    /// Every AGU-lane cell must be a PURE address computation — never a heap tile. The structural heart of the
    /// decoupling: the address lane has no side effects, so hoisting it cannot reorder memory.
    pub fn agu_is_pure(&self) -> bool {
        self.layout
            .iter()
            .zip(&self.is_agu)
            .filter(|(_, a)| **a)
            .all(|(c, _)| !is_heap_cell(c))
    }

    /// The exec lane's critical-path depth, treating every AGU-lane output as depth 0 (supplied by the parallel
    /// lane through the decoupling queue). This is the compute path the clock actually sees.
    pub fn exec_lane_depth(&self) -> usize {
        let mut depth = vec![0usize; self.layout.len()];
        let mut ans = 0usize;
        for i in 0..self.layout.len() {
            if self.is_agu[i] {
                continue; // AGU cells contribute through the parallel lane, not the exec path
            }
            let d = 1 + self.operand_exec_depth(&self.layout[i], &depth);
            depth[i] = d;
            ans = ans.max(d);
        }
        ans
    }

    /// The AGU lane's own critical-path depth (runs concurrently with the exec lane).
    pub fn agu_lane_depth(&self) -> usize {
        let mut depth = vec![0usize; self.layout.len()];
        let mut ans = 0usize;
        for i in 0..self.layout.len() {
            if !self.is_agu[i] {
                continue;
            }
            let d = 1 + cell_operands(&self.layout[i])
                .iter()
                .map(|s| match s {
                    Src::Val(k) => depth[*k],
                    _ => 0,
                })
                .max()
                .unwrap_or(0);
            depth[i] = d;
            ans = ans.max(d);
        }
        ans
    }

    /// The throughput-limiting depth of the decoupled schedule: the two lanes run concurrently, so the clock
    /// sees the DEEPER of them — not their sum.
    pub fn decoupled_critical_depth(&self) -> usize {
        self.agu_lane_depth().max(self.exec_lane_depth())
    }

    fn operand_exec_depth(&self, cell: &LoopCell, depth: &[usize]) -> usize {
        cell_operands(cell)
            .iter()
            .map(|s| match s {
                // An AGU output is precomputed in the parallel lane ⇒ free to the exec path.
                Src::Val(k) if self.is_agu[*k] => 0,
                Src::Val(k) => depth[*k],
                _ => 0,
            })
            .max()
            .unwrap_or(0)
    }
}

/// True iff the cell touches the state plane (a load/store/alloc) — the side-effecting tiles the exec lane
/// must keep in program order.
pub fn is_heap_cell(cell: &LoopCell) -> bool {
    matches!(
        cell,
        LoopCell::HeapRead { .. }
            | LoopCell::HeapWrite { .. }
            | LoopCell::HeapAlloc { .. }
            | LoopCell::MemRead { .. }
    )
}

/// Every `Src` operand a cell reads (compute + address + value, in a stable order).
fn cell_operands(cell: &LoopCell) -> Vec<Src> {
    match cell {
        LoopCell::Arith { a, b, .. }
        | LoopCell::Func { a, b, .. }
        | LoopCell::VectorArith { a, b, .. } => vec![*a, *b],
        LoopCell::Select { ca, cb, t, e, .. } => vec![*ca, *cb, *t, *e],
        LoopCell::HeapRead { addr } | LoopCell::MemRead { addr } => vec![*addr],
        LoopCell::HeapWrite { addr, val } => vec![*addr, *val],
        LoopCell::HeapAlloc { size } => vec![*size],
    }
}

/// The ADDRESS-class operands of a cell (what an AGU would precompute): a memory tile's address / alloc size.
/// Non-memory cells have none.
fn address_operands(cell: &LoopCell) -> Vec<Src> {
    match cell {
        LoopCell::HeapRead { addr } | LoopCell::MemRead { addr } => vec![*addr],
        LoopCell::HeapWrite { addr, .. } => vec![*addr],
        LoopCell::HeapAlloc { size } => vec![*size],
        _ => vec![],
    }
}

/// The coupled body's critical-path depth: the longest dependency chain feeding `result` (every operand
/// counts — address arithmetic sits on the compute path).
pub fn coupled_critical_depth(cells: &[LoopCell], result: Src) -> usize {
    let mut depth = vec![0usize; cells.len()];
    for i in 0..cells.len() {
        depth[i] = 1 + cell_operands(&cells[i])
            .iter()
            .map(|s| match s {
                Src::Val(k) => depth[*k],
                _ => 0,
            })
            .max()
            .unwrap_or(0);
    }
    match result {
        Src::Val(k) => depth[k],
        _ => depth.iter().copied().max().unwrap_or(0),
    }
}

/// Remap a `Src` through the old→new cell-index table (leaves `In`/`Const` untouched).
fn remap(src: Src, newpos: &[usize]) -> Src {
    match src {
        Src::Val(k) => Src::Val(newpos[k]),
        other => other,
    }
}

/// Rewrite a cell's operand `Src`s through `newpos`.
fn remap_cell(cell: &LoopCell, newpos: &[usize]) -> LoopCell {
    match *cell {
        LoopCell::Arith { op, a, b } => LoopCell::Arith {
            op,
            a: remap(a, newpos),
            b: remap(b, newpos),
        },
        LoopCell::Func { unit, a, b } => LoopCell::Func {
            unit,
            a: remap(a, newpos),
            b: remap(b, newpos),
        },
        LoopCell::VectorArith { op, ty, a, b } => LoopCell::VectorArith {
            op,
            ty,
            a: remap(a, newpos),
            b: remap(b, newpos),
        },
        LoopCell::Select { cmp, ca, cb, t, e } => LoopCell::Select {
            cmp,
            ca: remap(ca, newpos),
            cb: remap(cb, newpos),
            t: remap(t, newpos),
            e: remap(e, newpos),
        },
        LoopCell::HeapRead { addr } => LoopCell::HeapRead {
            addr: remap(addr, newpos),
        },
        LoopCell::MemRead { addr } => LoopCell::MemRead {
            addr: remap(addr, newpos),
        },
        LoopCell::HeapWrite { addr, val } => LoopCell::HeapWrite {
            addr: remap(addr, newpos),
            val: remap(val, newpos),
        },
        LoopCell::HeapAlloc { size } => LoopCell::HeapAlloc {
            size: remap(size, newpos),
        },
    }
}

/// Decouple a straight-line body into an AGU lane + exec lane. The AGU lane is the set of cells that
/// transitively feed a memory tile's address AND whose dependency cone never crosses a load (untainted) — the
/// address arithmetic that can be precomputed off the compute path. Everything else (compute + the memory
/// tiles + any pointer-chase address whose cone reads memory) stays in the exec lane, in program order. The
/// two are concatenated AGU-first with every `Src::Val` remapped, so the layout is a valid topological
/// reordering of the SAME cells — semantics-preserving by construction.
pub fn decouple_access(cells: &[LoopCell], result: Src, n_in: usize) -> DaeSchedule {
    let n = cells.len();

    // taint[i]: cell i's cone contains a load/store/alloc (so it cannot be hoisted onto a pure address lane).
    let mut taint = vec![false; n];
    for i in 0..n {
        taint[i] = is_heap_cell(&cells[i])
            || cell_operands(&cells[i])
                .iter()
                .any(|s| matches!(s, Src::Val(k) if taint[*k]));
    }

    // feeds_addr[i]: cell i transitively feeds some memory tile's address operand. Seed from address operands,
    // then close under predecessors (operand indices are strictly smaller, so one reverse sweep suffices).
    let mut feeds_addr = vec![false; n];
    for cell in cells {
        for s in address_operands(cell) {
            if let Src::Val(k) = s {
                feeds_addr[k] = true;
            }
        }
    }
    for i in (0..n).rev() {
        if feeds_addr[i] {
            for s in cell_operands(&cells[i]) {
                if let Src::Val(k) = s {
                    feeds_addr[k] = true;
                }
            }
        }
    }

    // The AGU lane: untainted address-feeding cells (pure by construction, and predecessor-closed).
    let is_agu_orig: Vec<bool> = (0..n).map(|i| feeds_addr[i] && !taint[i]).collect();

    // Concatenate AGU-first (ascending original order preserves dependencies within each lane), remapping Vals.
    let order: Vec<usize> = (0..n)
        .filter(|&i| is_agu_orig[i])
        .chain((0..n).filter(|&i| !is_agu_orig[i]))
        .collect();
    let mut newpos = vec![0usize; n];
    for (new_idx, &old) in order.iter().enumerate() {
        newpos[old] = new_idx;
    }
    let layout: Vec<LoopCell> = order
        .iter()
        .map(|&old| remap_cell(&cells[old], &newpos))
        .collect();
    let is_agu: Vec<bool> = order.iter().map(|&old| is_agu_orig[old]).collect();
    let agu_len = is_agu_orig.iter().filter(|&&a| a).count();

    DaeSchedule {
        n_in,
        layout,
        result: remap(result, &newpos),
        agu_len,
        is_agu,
    }
}

/// Schedule a body FOR a described target: decouple its address lane iff the target carries an AGU
/// (`target.agu`), else leave it coupled (identity). This is the descriptor knob — a fabric without an AGU
/// cannot exploit decoupling, so the transform only fires when the target can realize it. Either way the
/// schedule is semantics-preserving (a proven reordering, or the identity).
pub fn schedule_for_target(
    cells: &[LoopCell],
    result: Src,
    n_in: usize,
    target: &FabricTarget,
) -> DaeSchedule {
    if target.agu {
        decouple_access(cells, result, n_in)
    } else {
        DaeSchedule {
            n_in,
            layout: cells.to_vec(),
            result,
            agu_len: 0,
            is_agu: vec![false; cells.len()],
        }
    }
}

/// Evaluate a decoupled schedule against `inputs` + a shared `heap`, exactly as the coupled body would. Reuses
/// the decision-fabric heap reduction over the re-laid-out cells — so agreement with the coupled eval is the
/// semantics-preservation certificate.
pub fn eval_dae(sched: &DaeSchedule, inputs: &[i64], heap: &mut Vec<i64>) -> i64 {
    let df = DecisionFabric {
        n_in: sched.n_in,
        cells: sched.layout.clone(),
        result: sched.result,
        escaped_first_return: false,
    };
    eval_decision_fabric_heap(&df, inputs, heap)
}

/// Render a `Src` as a Verilog operand (`in{k}` / `v{k}` / a signed constant).
fn src_v(s: &Src) -> String {
    match s {
        Src::In(k) => format!("in{k}"),
        Src::Val(k) => format!("v{k}"),
        Src::Const(c) if *c < 0 => format!("-64'sd{}", (*c as i128).unsigned_abs()),
        Src::Const(c) => format!("64'sd{c}"),
    }
}

/// Emit a decoupled schedule as COMBINATIONAL Verilog: the AGU lane (address arithmetic) and the exec lane
/// (compute + loads) are emitted as two labelled regions, the loads reading a `depth`-word heap. Bit-identical
/// to [`eval_dae`] (out-of-range loads read 0, matching the model) — proven under iverilog by `dae_rtl_lock`.
/// LOAD-ONLY: `HeapWrite`/`HeapAlloc`/`Func` are the sequenced-state follow-on and panic here (the honest
/// boundary), so the corpus is streaming reads with computed addresses — the canonical DAE case.
pub fn emit_dae_verilog(name: &str, sched: &DaeSchedule, depth: usize) -> String {
    let idxbits = (usize::BITS - (depth - 1).leading_zeros()) as usize; // clog2(depth)
    let inputs = (0..sched.n_in)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut body = String::new();
    for (i, cell) in sched.layout.iter().enumerate() {
        if i == 0 && sched.agu_len > 0 {
            body.push_str("  // --- AGU lane (address generation, runs in parallel) ---\n");
        }
        if i == sched.agu_len {
            body.push_str("  // --- exec lane (compute + loads) ---\n");
        }
        match cell {
            LoopCell::Arith { op, a, b } => {
                body.push_str(&format!(
                    "  wire [63:0] v{i} = {};\n",
                    op.verilog(&src_v(a), &src_v(b))
                ));
            }
            LoopCell::Select { cmp, ca, cb, t, e } => {
                body.push_str(&format!(
                    "  wire [63:0] v{i} = ($signed({}) {} $signed({})) ? {} : {};\n",
                    src_v(ca),
                    cmp.verilog(),
                    src_v(cb),
                    src_v(t),
                    src_v(e),
                ));
            }
            LoopCell::HeapRead { addr } => {
                body.push_str(&format!(
                    "  wire signed [63:0] addr{i} = {};\n",
                    src_v(addr)
                ));
                body.push_str(&format!(
                    "  wire [63:0] v{i} = (addr{i} >= 0 && addr{i} < {depth}) ? heap[addr{i}[{hi}:0]] : 64'd0;\n",
                    hi = idxbits - 1,
                ));
            }
            other => panic!(
                "emit_dae_verilog: {other:?} is the sequenced-state follow-on (load-only today)"
            ),
        }
    }
    format!(
        "// dae_{name} — a decoupled ACCESS/EXECUTE datapath: the AGU lane computes addresses in parallel with\n\
         // the exec lane; loads read a {depth}-word heap. Combinational, ≡ eval_dae under iverilog.\n\
         module dae_{name}({inputs}, output [63:0] result);\n\
         \x20 reg [63:0] heap [0:{maxd}];\n\
         \x20 initial $readmemh(\"dae_{name}_heap.hex\", heap);\n\
         {body}  assign result = {res};\n\
         endmodule\n",
        maxd = depth - 1,
        res = src_v(&sched.result),
    )
}

/// Emit a decoupled schedule that MUTATES the heap (`HeapWrite`/`HeapAlloc`, not just loads) as a SEQUENCED
/// synthesizable datapath: the decoupled layout (AGU cells hoisted first) is run on the proven state-plane
/// micro-sequencer (`emit_state_plane_verilog`), which commits heap writes in program order over one shared
/// heap + a bump allocator. Module `state_{name}` (clk/rst/inN/result/done). Bit-identical to [`eval_dae`]
/// (≡ the coupled body ≡ the VM); proven under iverilog by `dae_rtl_lock`. Distinct from the combinational
/// load-only [`emit_dae_verilog`], which realizes the parallel AGU lane for streaming reads.
pub fn emit_dae_stateful_verilog(name: &str, sched: &DaeSchedule, heap_words: usize) -> String {
    let df = DecisionFabric {
        n_in: sched.n_in,
        cells: sched.layout.clone(),
        result: sched.result,
        escaped_first_return: false,
    };
    emit_state_plane_verilog(name, &df, heap_words)
}

/// A self-checking combinational testbench for `dae_{name}`: drive `n` input vectors (each `n_in` words) from
/// `dae_{name}_in.hex`, compare the settled `result` against the `eval_dae` oracle in `dae_{name}_out.hex`.
pub fn emit_dae_testbench(name: &str, n_in: usize, n: usize) -> String {
    let decls = (0..n_in)
        .map(|k| format!("  reg [63:0] in{k};\n"))
        .collect::<String>();
    let ports = (0..n_in)
        .map(|k| format!(".in{k}(in{k}), "))
        .collect::<String>();
    let drive = (0..n_in)
        .map(|k| format!("      in{k} = oin[i * {n_in} + {k}];\n"))
        .collect::<String>();
    format!(
        r#"`timescale 1ns/1ps
module tb_dae_{name};
{decls}  reg [63:0] oin  [0:{in_last}];
  reg [63:0] oout [0:{last}];
  wire [63:0] result;
  integer i, errors = 0;
  dae_{name} dut({ports}.result(result));
  initial begin
    $readmemh("dae_{name}_in.hex", oin);
    $readmemh("dae_{name}_out.hex", oout);
    for (i = 0; i < {n}; i = i + 1) begin
{drive}      #1;
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL vec %0d: got %0d", i, $signed(result)); end
    end
    if (errors == 0) $display("PASS: {n} decoupled-datapath evals match eval_dae (≡ the coupled body ≡ VM)");
    else $display("FAIL: %0d decoupled-datapath mismatches", errors);
    $finish;
  end
endmodule
"#,
        in_last = n_in * n - 1,
        last = n - 1,
    )
}
