//! fabric_grid — a TRUE reconfigurable CGRA: place a fabric's dataflow onto a 2-D array of
//! identical tiles and route it through a LOCAL switchbox interconnect. The same silicon runs any
//! fabric by reconfiguration — not a bespoke circuit per program.
//!
//! Interconnect: each tile drives two INDEPENDENT channels — a vertical `vs` (to its south
//! neighbor) and a horizontal `he` (to its east neighbor) — so a value flowing down and a value
//! routed across CROSS without conflict (the standard CGRA switchbox). Dataflow is feed-forward
//! (south/east only), so the array is a combinational DAG. A tile reads its north neighbor's `vs`
//! and its west neighbor's `he`; it may pass either through, TURN one into the other, source a
//! global input / constant, or COMPUTE (an `Alu`/`Select` tapping those inputs) and drive the
//! result onto a channel.
//!
//! The point (proven in `fabric_grid`'s S3 tests): whatever the placer/router did, the placed +
//! routed array computes EXACTLY the abstract datapath — certified exhaustively by our own SAT.
//! That is the property a trustworthy CGRA compiler needs: place-and-route is semantics-preserving.

use super::fabric_rtl::{AluOp, CmpKind, FuncUnit};
use super::simd::{LaneTy, VecOp};

/// A value source visible to a tile: its north/west channel inputs, a global input, a constant,
/// or zero (an unused channel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Src2 {
    N,
    W,
    In(usize),
    Const(i64),
    Zero,
}

/// A tile's optional compute: an arithmetic op or a predicated select over its visible sources.
#[derive(Clone, Copy, Debug)]
pub enum CellKind {
    Arith(AluOp, Src2, Src2),
    Select(CmpKind, Src2, Src2, Src2, Src2),
    /// A FUNCTIONAL-UNIT tile — division/mod/floor-div, magic-reciprocal high-multiply, power, or a float op
    /// (`FuncUnit`). Combinational at the RTL level (a synthesized divider / the FPU's combinational form), so
    /// it settles in the same 1-clock feed-forward pass as `Arith` — but it is NOT SAT-lowered (bitblast has
    /// no `bvsdiv`/`bvsrem`), so a Func-bearing grid is proven by `eval_grid ≡ VM` + iverilog, exactly as the
    /// decision fabric proves division (`emit_decision_fabric_verilog` + `microprice_rtl_matches_vm_under_iverilog`).
    Func(FuncUnit, Src2, Src2),
    /// A packed-SIMD tile — a `VecOp` (add / bitwise) over two operands, computed lane-wise for the given
    /// `LaneTy` (`Add` = the carry-isolated SWAR form). Combinational, so it settles in the same 1-clock pass
    /// as `Arith`; but it is NOT SAT-lowered here (proven ≡ lane-wise by `simd_tile_lock` + iverilog), so a
    /// Simd-bearing grid is combinational-for-emission yet refused by the reconfig SAT (like `Func`).
    Simd(VecOp, LaneTy, Src2, Src2),
    /// A MEMORY (state-plane) tile — a read/write/alloc against ONE shared heap that all `Mem` tiles access
    /// (the `dmem`-style block RAM the soft-core already carries). The heap mirrors the VM/model layout
    /// (`[len, e0..eN]`), so collections/structs/text/CRDT compose from `Mem` + `Arith` tiles exactly as the
    /// whole-program model (`fabric_rtl.rs` exec_ops) does. Stateful (a synchronous BRAM), so it is NOT
    /// SAT-lowered and is NOT combinational — proven by `eval_grid_with_heap ≡ the model ≡ VM` + iverilog.
    /// The tile's two `Src2` operands are (A, B): `HeapRead` reads `heap[A]`, `HeapWrite` does `heap[A] = B`
    /// and drives `B`, `HeapAlloc` grows the heap by `A` words and drives the new base pointer.
    Mem(MemOp, Src2, Src2),
}

/// The three state-plane operations a `Mem` tile performs against the shared heap (see [`CellKind::Mem`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemOp {
    /// `out = heap[A]` (0 if out of bounds — mirrors the model's `heap.get(a).unwrap_or(0)`).
    HeapRead,
    /// `heap[A] = B`; the tile also drives `B` onto its output channel (mirrors the model's write returning `v`).
    HeapWrite,
    /// Allocate `A` fresh (zero) words at the top of the heap; `out = the new base pointer` (`heap.len()`).
    HeapAlloc,
}

/// What drives one of a tile's output channels: a routed source, or the tile's compute result.
#[derive(Clone, Copy, Debug)]
pub enum Drive {
    Pass(Src2),
    Alu,
}

/// One configured CGRA tile.
#[derive(Clone, Copy, Debug)]
pub struct Tile {
    pub alu: Option<CellKind>,
    pub vs: Drive,
    pub he: Drive,
}

impl Tile {
    /// An idle tile: both channels zero, no compute.
    pub const IDLE: Tile = Tile {
        alu: None,
        vs: Drive::Pass(Src2::Zero),
        he: Drive::Pass(Src2::Zero),
    };
}

/// Which output channel of a tile carries the fabric's result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Chan {
    Vs,
    He,
}

/// A placed-and-routed CGRA: a `rows × cols` array of tiles plus the tile/channel that carries the
/// result. Feed-forward (dataflow goes south/east), so it is a combinational DAG.
#[derive(Clone, Debug)]
pub struct Grid {
    pub rows: usize,
    pub cols: usize,
    pub n_in: usize,
    pub tiles: Vec<Vec<Tile>>,
    /// The primary output (a single-output fabric's result = `out0`).
    pub out: (usize, usize, Chan),
    /// Additional outputs (a loop body has one next-state per accumulator). Total outputs =
    /// `[out] ++ extra_outs`, named `result` when there is one, else `out0..outK`.
    pub extra_outs: Vec<(usize, usize, Chan)>,
}

impl Grid {
    pub fn new(rows: usize, cols: usize, n_in: usize) -> Grid {
        Grid {
            rows,
            cols,
            n_in,
            tiles: vec![vec![Tile::IDLE; cols]; rows],
            out: (0, 0, Chan::Vs),
            extra_outs: Vec::new(),
        }
    }
    /// All output taps in order: the primary followed by the extras.
    pub fn all_outs(&self) -> Vec<(usize, usize, Chan)> {
        std::iter::once(self.out)
            .chain(self.extra_outs.iter().copied())
            .collect()
    }
    pub fn set(&mut self, r: usize, c: usize, t: Tile) {
        self.tiles[r][c] = t;
    }
    /// Manhattan wire length = sum over tiles of the channel spans actually used (a rough area/
    /// routing-cost proxy; every used channel is one nearest-neighbor hop).
    pub fn used_channels(&self) -> usize {
        let mut n = 0;
        for row in &self.tiles {
            for t in row {
                if !matches!(t.vs, Drive::Pass(Src2::Zero)) {
                    n += 1;
                }
                if !matches!(t.he, Drive::Pass(Src2::Zero)) {
                    n += 1;
                }
            }
        }
        n
    }
}

/// Read a `Src2` given a tile's channel inputs and the global inputs (64-bit wrapping).
fn read2(s: Src2, n_in: i64, w_in: i64, inputs: &[i64]) -> i64 {
    match s {
        Src2::N => n_in,
        Src2::W => w_in,
        Src2::In(k) => inputs[k],
        Src2::Const(c) => c,
        Src2::Zero => 0,
    }
}

fn eval_cell(k: CellKind, n_in: i64, w_in: i64, inputs: &[i64]) -> i64 {
    let r = |s| read2(s, n_in, w_in, inputs);
    match k {
        CellKind::Arith(op, a, b) => op.eval(r(a), r(b)),
        CellKind::Func(unit, a, b) => unit.eval(r(a), r(b)),
        CellKind::Simd(op, ty, a, b) => op.eval(ty, r(a) as u64, r(b) as u64) as i64,
        CellKind::Mem(..) => panic!(
            "eval_cell: a Mem (state-plane) tile needs the shared heap — use eval_grid_with_heap, not the \
             stateless eval_grid"
        ),
        CellKind::Select(cmp, ca, cb, t, e) => {
            if cmp.eval(r(ca), r(cb)) {
                r(t)
            } else {
                r(e)
            }
        }
    }
}

/// Simulate the grid (row-major, so every tile's north/west inputs are already resolved — the
/// feed-forward invariant) and return the result value the output tile/channel carries.
fn grid_channels(g: &Grid, inputs: &[i64]) -> (Vec<Vec<i64>>, Vec<Vec<i64>>) {
    let mut vs = vec![vec![0i64; g.cols]; g.rows];
    let mut he = vec![vec![0i64; g.cols]; g.rows];
    for r in 0..g.rows {
        for c in 0..g.cols {
            let n_in = if r == 0 { 0 } else { vs[r - 1][c] };
            let w_in = if c == 0 { 0 } else { he[r][c - 1] };
            let t = g.tiles[r][c];
            let alu = t.alu.map(|k| eval_cell(k, n_in, w_in, inputs)).unwrap_or(0);
            let drive = |d: Drive| match d {
                Drive::Pass(s) => read2(s, n_in, w_in, inputs),
                Drive::Alu => alu,
            };
            vs[r][c] = drive(t.vs);
            he[r][c] = drive(t.he);
        }
    }
    (vs, he)
}

fn tap(vs: &[Vec<i64>], he: &[Vec<i64>], o: (usize, usize, Chan)) -> i64 {
    let (r, c, ch) = o;
    match ch {
        Chan::Vs => vs[r][c],
        Chan::He => he[r][c],
    }
}

/// Simulate the grid (feed-forward, row-major) and return the PRIMARY output value.
pub fn eval_grid(g: &Grid, inputs: &[i64]) -> i64 {
    let (vs, he) = grid_channels(g, inputs);
    tap(&vs, &he, g.out)
}

/// Simulate the grid and return ALL output values in `all_outs` order (for multi-output loop bodies).
pub fn eval_grid_all(g: &Grid, inputs: &[i64]) -> Vec<i64> {
    let (vs, he) = grid_channels(g, inputs);
    g.all_outs().into_iter().map(|o| tap(&vs, &he, o)).collect()
}

/// Simulate a STATE-PLANE grid — one that carries `Mem` (heap read/write/alloc) tiles — threading ONE shared
/// heap through the tiles in row-major (= placement = program) order, exactly as the model's `eval_cells`
/// threads its heap. Non-`Mem` tiles evaluate combinationally (`eval_cell`); a `Mem` tile touches the heap and
/// drives its result onto its output channel. Returns the primary output. This is how Stage X2 proves
/// collections/structs/text run on the grid: `eval_grid_with_heap ≡ the whole-program model ≡ the VM`.
pub fn eval_grid_with_heap(g: &Grid, inputs: &[i64]) -> i64 {
    let mut heap: Vec<i64> = Vec::new();
    let (vs, he) = grid_channels_with_heap(g, inputs, &mut heap);
    tap(&vs, &he, g.out)
}

/// Simulate a (possibly state-plane) grid, threading a SHARED heap through its `Mem` tiles in row-major
/// (= placement = program) order — the heap layout that mirrors the VM's `[len, e1..eN]`. Factored so both
/// the single-output `eval_grid_with_heap` and the multi-output `eval_grid_all_with_heap` share ONE
/// heap-threading pass; a heap-free grid never touches `heap`, so this equals the stateless `grid_channels`.
fn grid_channels_with_heap(
    g: &Grid,
    inputs: &[i64],
    heap: &mut Vec<i64>,
) -> (Vec<Vec<i64>>, Vec<Vec<i64>>) {
    let mut vs = vec![vec![0i64; g.cols]; g.rows];
    let mut he = vec![vec![0i64; g.cols]; g.rows];
    for r in 0..g.rows {
        for c in 0..g.cols {
            let n = if r == 0 { 0 } else { vs[r - 1][c] };
            let w = if c == 0 { 0 } else { he[r][c - 1] };
            let t = g.tiles[r][c];
            let alu = match t.alu {
                Some(CellKind::Mem(op, a, b)) => {
                    let av = read2(a, n, w, inputs);
                    match op {
                        MemOp::HeapRead => heap.get(av.max(0) as usize).copied().unwrap_or(0),
                        MemOp::HeapWrite => {
                            let bv = read2(b, n, w, inputs);
                            let ai = av.max(0) as usize;
                            if ai >= heap.len() {
                                heap.resize(ai + 1, 0);
                            }
                            heap[ai] = bv;
                            bv
                        }
                        MemOp::HeapAlloc => {
                            let ptr = heap.len() as i64;
                            heap.resize(heap.len() + av.max(0) as usize, 0);
                            ptr
                        }
                    }
                }
                Some(k) => eval_cell(k, n, w, inputs),
                None => 0,
            };
            vs[r][c] = match t.vs {
                Drive::Pass(s) => read2(s, n, w, inputs),
                Drive::Alu => alu,
            };
            he[r][c] = match t.he {
                Drive::Pass(s) => read2(s, n, w, inputs),
                Drive::Alu => alu,
            };
        }
    }
    (vs, he)
}

/// Like [`eval_grid_all`] but threads a SHARED heap (for a whole-program block walk with a state plane):
/// every `Mem` tile touches `heap` (persisting across blocks), and ALL outputs are returned in `all_outs()`
/// order so the block-sequencing controller can commit the block's register writes + read its terminator
/// operand. For a heap-free grid this equals `eval_grid_all` (the heap stays empty).
pub fn eval_grid_all_with_heap(g: &Grid, inputs: &[i64], heap: &mut Vec<i64>) -> Vec<i64> {
    let (vs, he) = grid_channels_with_heap(g, inputs, heap);
    g.all_outs().into_iter().map(|o| tap(&vs, &he, o)).collect()
}

/// Run a whole (acyclic-control-flow) function on the reconfigurable GRID (Stage X4 — the whole-program XPU).
/// Each basic block's straight-line body is PLACED on a fresh tile array (`route_loop_body`, the block's
/// live-in registers as inputs) with its downstream `Src`s as the grid's outputs — every register write, then
/// the terminator's predicate/return operand; `eval_grid_all` reads them off the placed array, the register
/// file is updated, and a block-sequencing controller walks the terminator graph (Jump/Branch/Return),
/// reconfiguring the array to the next block. So a BRANCHY whole program runs as spatial dataflow on ONE array
/// (each block a reconfiguration), proven ≡ `eval_program_cfg_fabric` ≡ the VM. A block may carry `Mem`
/// (heap read/write/alloc) tiles: ONE heap is threaded across every reconfiguration (`eval_grid_all_with_heap`),
/// so collections/structs persist across blocks — the state plane. (Loops are the separate loop fabric.)
pub fn run_program_cfg_on_grid(pf: &super::fabric_rtl::ProgramCfgFabric, params: &[i64]) -> i64 {
    use super::fabric_rtl::{Src as FSrc, Term};
    use std::collections::HashMap;
    let mut file: HashMap<u16, i64> = (0..pf.n_params).map(|p| (p as u16, params[p])).collect();
    // ONE shared heap threaded across every block's reconfiguration — the state plane. A stateless program
    // (no `Mem` tiles) never touches it, so this walk stays identical to the stateless `eval_grid_all` path.
    let mut heap: Vec<i64> = Vec::new();
    let mut blk = 0usize;
    for _ in 0..10_000_000 {
        let b = &pf.blocks[blk];
        let inputs: Vec<i64> = b
            .inputs
            .iter()
            .map(|r| *file.get(r).unwrap_or(&0))
            .collect();
        // The `Src`s this block needs downstream: each register write's value, then (last) the terminator's
        // predicate/return operand — placed as the block-grid's outputs and read back by `eval_grid_all`.
        let mut needed: Vec<FSrc> = b.writes.iter().map(|(_, s)| *s).collect();
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
            Term::Return(_) => return outs[term_off],
        }
    }
    panic!(
        "cfg grid: controller did not return within the step budget (a back-edge slipped through?)"
    )
}

/// Render a `Src2` in Verilog, given the north/west channel wire names for this tile.
fn src2_v(s: Src2, n: &str, w: &str) -> String {
    match s {
        Src2::N => n.to_string(),
        Src2::W => w.to_string(),
        Src2::In(k) => format!("in{k}"),
        Src2::Const(c) => {
            if c < 0 {
                format!("-64'sd{}", (c as i128).unsigned_abs())
            } else {
                format!("64'sd{c}")
            }
        }
        Src2::Zero => "64'sd0".to_string(),
    }
}

fn cell_v(k: CellKind, n: &str, w: &str) -> String {
    let s = |x| src2_v(x, n, w);
    match k {
        CellKind::Arith(op, a, b) => op.verilog(&s(a), &s(b)),
        CellKind::Func(unit, a, b) => unit.verilog(&s(a), &s(b)),
        CellKind::Simd(op, ty, a, b) => op.verilog(ty, &s(a), &s(b)),
        CellKind::Mem(..) => panic!(
            "cell_v: Mem-tile RTL (a shared BRAM read/write + a sequencing controller) is the X2b follow-on; \
             X2a proves the state plane at the eval level via eval_grid_with_heap"
        ),
        CellKind::Select(cmp, ca, cb, t, e) => {
            format!("($signed({}) {} $signed({})) ? {} : {}", s(ca), cmp.verilog(), s(cb), s(t), s(e))
        }
    }
}

/// Emit the tile array as combinational RTL: two channel wires (`vs`/`he`) per tile, each tile
/// wired only to its north/west neighbours — a genuine nearest-neighbour spatial fabric.
pub fn emit_grid_verilog(name: &str, g: &Grid) -> String {
    let inputs = (0..g.n_in)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let inputs = if inputs.is_empty() {
        String::new()
    } else {
        format!("{inputs}, ")
    };
    let mut body = String::new();
    for r in 0..g.rows {
        for c in 0..g.cols {
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
            let t = g.tiles[r][c];
            if let Some(k) = t.alu {
                body.push_str(&format!(
                    "  wire [63:0] alu_{r}_{c} = {};\n",
                    cell_v(k, &nn, &ww)
                ));
            }
            let drv = |d: Drive| match d {
                Drive::Pass(s) => src2_v(s, &nn, &ww),
                Drive::Alu => format!("alu_{r}_{c}"),
            };
            body.push_str(&format!("  wire [63:0] vs_{r}_{c} = {};\n", drv(t.vs)));
            body.push_str(&format!("  wire [63:0] he_{r}_{c} = {};\n", drv(t.he)));
        }
    }
    let outs = g.all_outs();
    let out_wire = |o: (usize, usize, Chan)| match o.2 {
        Chan::Vs => format!("vs_{}_{}", o.0, o.1),
        Chan::He => format!("he_{}_{}", o.0, o.1),
    };
    // One output → `result`; several (a loop body's next-state) → `out0..outK`.
    let (ports, assigns) = if outs.len() == 1 {
        (
            "output [63:0] result".to_string(),
            format!("  assign result = {};\n", out_wire(outs[0])),
        )
    } else {
        let ports = (0..outs.len())
            .map(|j| format!("output [63:0] out{j}"))
            .collect::<Vec<_>>()
            .join(", ");
        let assigns = outs
            .iter()
            .enumerate()
            .map(|(j, o)| format!("  assign out{j} = {};\n", out_wire(*o)))
            .collect::<String>();
        (ports, assigns)
    };
    format!(
        "// grid_{name} — a fabric placed on a {rows}×{cols} CGRA tile array with a two-channel\n\
         // switchbox interconnect (independent vertical `vs` + horizontal `he` wires per tile,\n\
         // nearest-neighbour only). Feed-forward south/east, so combinational. The placed+routed\n\
         // array is certified == the abstract datapath by our own SAT.\n\
         module grid_{name}({inputs}{ports});\n\
         {body}{assigns}\
         endmodule\n",
        rows = g.rows,
        cols = g.cols,
    )
}

/// The PRIMARY output's `BoundedExpr` (output 0) with the default `in{k}` input names.
pub fn grid_to_bounded(g: &Grid, width: u32) -> crate::codegen_sva::sva_to_verify::BoundedExpr {
    grid_output_to_bounded(g, 0, width, &|k| format!("in{k}"))
}

// ── One datapath vocabulary, two lowerings ──────────────────────────────────────────────
//
// A placed grid and the cell body it is supposed to equal are lowered by the SAME walk into two
// different forms: the `BoundedExpr` tree the SMT/printer surfaces speak, and the shared gate DAG
// the certifier refutes. Writing that walk twice would be a soundness hole wearing the costume of
// a style choice — a tile the tree path spells one way and the DAG path another is a mapping we
// would "prove" against the wrong specification. So there is ONE walk over a `Datapath`
// vocabulary, and the two forms are its two instances.

/// The datapath vocabulary a placed grid — or a straight-line cell body — is lowered through.
pub trait Datapath {
    /// The lowered value: a `BoundedExpr` for the tree form, a `Word` for the shared gate DAG.
    type V: Clone;
    /// The global circuit input `k`.
    fn input(&mut self, k: usize) -> Self::V;
    /// A constant, truncated to the datapath width.
    fn constant(&mut self, c: i64) -> Self::V;
    /// An ALU operation over two lowered operands.
    fn arith(&mut self, op: AluOp, a: Self::V, b: Self::V) -> Self::V;
    /// The mux tile: `(ca cmp cb) ? t : e`.
    fn select(&mut self, cmp: CmpKind, ca: Self::V, cb: Self::V, t: Self::V, e: Self::V)
        -> Self::V;
}

/// The bitvector op an `AluOp` blasts as. `Shr` is ARITHMETIC (LOGOS's `shifted right by` is
/// signed) and `UShr` is the logical one — named once here so no lowering can disagree.
fn alu_bounded_op(op: AluOp) -> crate::codegen_sva::sva_to_verify::BitVecBoundedOp {
    use crate::codegen_sva::sva_to_verify::BitVecBoundedOp as B;
    match op {
        AluOp::Add => B::Add,
        AluOp::Sub => B::Sub,
        AluOp::Mul => B::Mul,
        AluOp::And => B::And,
        AluOp::Or => B::Or,
        AluOp::Xor => B::Xor,
        AluOp::Shl => B::Shl,
        AluOp::Shr => B::AShr,
        AluOp::UShr => B::Shr,
    }
}

/// How a `CmpKind` mux resolves into a signed/equality predicate and the ARM ORDER that predicate
/// selects: `Ge`/`Le`/`Ne` are the complements of `Lt`/`Gt`/`Eq`, spelled by swapping the arms
/// rather than negating the comparator. One table, read by both lowerings.
fn select_form(
    cmp: CmpKind,
) -> (
    crate::codegen_sva::sva_to_verify::BitVecBoundedOp,
    bool,
    bool,
) {
    use crate::codegen_sva::sva_to_verify::BitVecBoundedOp as B;
    // (predicate op, swap the comparison operands, swap the arms)
    match cmp {
        CmpKind::Lt => (B::SLt, false, false),
        CmpKind::Gt => (B::SLt, true, false),
        CmpKind::Ge => (B::SLt, false, true),
        CmpKind::Le => (B::SLt, true, true),
        CmpKind::Eq => (B::Eq, false, false),
        CmpKind::Ne => (B::Eq, false, true),
    }
}

/// The `BoundedExpr` instance of [`Datapath`] — the tree form.
pub struct BoundedBuild<'a> {
    width: u32,
    name_in: &'a dyn Fn(usize) -> String,
}

impl<'a> BoundedBuild<'a> {
    /// Lower at `width`, naming global inputs through `name_in`.
    pub fn new(width: u32, name_in: &'a dyn Fn(usize) -> String) -> Self {
        BoundedBuild { width, name_in }
    }
}

impl Datapath for BoundedBuild<'_> {
    type V = crate::codegen_sva::sva_to_verify::BoundedExpr;
    fn input(&mut self, k: usize) -> Self::V {
        crate::codegen_sva::sva_to_verify::BoundedExpr::BitVecVar((self.name_in)(k), self.width)
    }
    fn constant(&mut self, c: i64) -> Self::V {
        let mask = if self.width >= 64 {
            u64::MAX
        } else {
            (1u64 << self.width) - 1
        };
        crate::codegen_sva::sva_to_verify::BoundedExpr::BitVecConst {
            width: self.width,
            value: (c as u64) & mask,
        }
    }
    fn arith(&mut self, op: AluOp, a: Self::V, b: Self::V) -> Self::V {
        crate::codegen_sva::sva_to_verify::BoundedExpr::BitVecBinary {
            op: alu_bounded_op(op),
            left: Box::new(a),
            right: Box::new(b),
        }
    }
    fn select(
        &mut self,
        cmp: CmpKind,
        ca: Self::V,
        cb: Self::V,
        t: Self::V,
        e: Self::V,
    ) -> Self::V {
        use crate::codegen_sva::sva_to_verify::BoundedExpr;
        let (op, swap_cmp, swap_arms) = select_form(cmp);
        let (l, r) = if swap_cmp { (cb, ca) } else { (ca, cb) };
        let (then, els) = if swap_arms { (e, t) } else { (t, e) };
        BoundedExpr::BitVecIte {
            cond: Box::new(BoundedExpr::BitVecBinary {
                op,
                left: Box::new(l),
                right: Box::new(r),
            }),
            then: Box::new(then),
            else_: Box::new(els),
        }
    }
}

/// The shared-gate-DAG instance of [`Datapath`] — the form the certifier refutes, linear in the
/// circuit instead of in its unfolding.
pub struct GateBuild<'a> {
    width: u32,
    name_in: &'a dyn Fn(usize) -> String,
}

impl<'a> GateBuild<'a> {
    /// Blast at `width`, naming global inputs through `name_in`.
    pub fn new(width: u32, name_in: &'a dyn Fn(usize) -> String) -> Self {
        GateBuild { width, name_in }
    }
}

impl Datapath for GateBuild<'_> {
    type V = crate::codegen_sva::bitblast::Word;
    fn input(&mut self, k: usize) -> Self::V {
        crate::codegen_sva::bitblast::word_var(&(self.name_in)(k), self.width)
    }
    fn constant(&mut self, c: i64) -> Self::V {
        crate::codegen_sva::bitblast::word_const(self.width, c as u64)
    }
    fn arith(&mut self, op: AluOp, a: Self::V, b: Self::V) -> Self::V {
        crate::codegen_sva::bitblast::word_binary(alu_bounded_op(op), &a, &b)
            .expect("every operand of a grid tile carries the grid's one width")
    }
    fn select(
        &mut self,
        cmp: CmpKind,
        ca: Self::V,
        cb: Self::V,
        t: Self::V,
        e: Self::V,
    ) -> Self::V {
        let (op, swap_cmp, swap_arms) = select_form(cmp);
        let (l, r) = if swap_cmp { (cb, ca) } else { (ca, cb) };
        let (then, els) = if swap_arms { (e, t) } else { (t, e) };
        let pred = crate::codegen_sva::bitblast::word_compare(op, &l, &r)
            .expect("every operand of a grid tile carries the grid's one width");
        crate::codegen_sva::bitblast::word_select(&pred, &then, &els)
            .expect("every operand of a grid tile carries the grid's one width")
    }
}

/// The message a tile with no propositional lowering refuses with — one string, so the tree and
/// DAG lowerings decline identically.
const NO_BITBLAST_TILE: &str =
    "grid SAT: a Func (division/float), Mem (state-plane), or Simd (packed) tile has no bitblast \
     lowering here — such a grid is proven by eval ≡ VM + iverilog, not by our SAT";

/// One tile's two channel outputs `(vs, he)` given its north/west channel inputs. `None` when the
/// tile has no propositional lowering (`Func`/`Mem`/`Simd`).
fn tile_outputs<B: Datapath>(b: &mut B, t: Tile, n: &B::V, w: &B::V) -> Option<(B::V, B::V)> {
    let src2 = |b: &mut B, s: Src2| -> B::V {
        match s {
            Src2::N => n.clone(),
            Src2::W => w.clone(),
            Src2::In(k) => b.input(k),
            Src2::Const(c) => b.constant(c),
            Src2::Zero => b.constant(0),
        }
    };
    let alu = match t.alu {
        None => None,
        Some(CellKind::Arith(op, x, y)) => {
            let (x, y) = (src2(b, x), src2(b, y));
            Some(b.arith(op, x, y))
        }
        Some(CellKind::Select(cmp, ca, cb, tt, ee)) => {
            let (ca, cb, tt, ee) = (src2(b, ca), src2(b, cb), src2(b, tt), src2(b, ee));
            Some(b.select(cmp, ca, cb, tt, ee))
        }
        Some(CellKind::Func(..)) | Some(CellKind::Mem(..)) | Some(CellKind::Simd(..)) => {
            return None
        }
    };
    let drive = |b: &mut B, d: Drive| -> B::V {
        match d {
            Drive::Pass(s) => src2(b, s),
            Drive::Alu => alu.clone().expect("Drive::Alu on a tile with no compute"),
        }
    };
    let vs = drive(b, t.vs);
    let he = drive(b, t.he);
    Some((vs, he))
}

/// Lower output `which` (in `all_outs` order) through the datapath vocabulary `b`, resolving the
/// array row-major so every tile's north/west inputs are already lowered (the feed-forward
/// invariant). `None` when any live tile has no propositional lowering.
pub fn grid_output_datapath<B: Datapath>(b: &mut B, g: &Grid, which: usize) -> Option<B::V> {
    let zero = b.constant(0);
    let mut vs: Vec<Vec<B::V>> = vec![vec![zero.clone(); g.cols]; g.rows];
    let mut he: Vec<Vec<B::V>> = vec![vec![zero.clone(); g.cols]; g.rows];
    for r in 0..g.rows {
        for c in 0..g.cols {
            let n = if r == 0 {
                zero.clone()
            } else {
                vs[r - 1][c].clone()
            };
            let w = if c == 0 {
                zero.clone()
            } else {
                he[r][c - 1].clone()
            };
            let (v, h) = tile_outputs(b, g.tiles[r][c], &n, &w)?;
            vs[r][c] = v;
            he[r][c] = h;
        }
    }
    let (or, oc, ch) = g.all_outs()[which];
    Some(match ch {
        Chan::Vs => vs[or][oc].clone(),
        Chan::He => he[or][oc].clone(),
    })
}

/// Lower output `which` STRAIGHT INTO THE SHARED GATE DAG at `width` — the certifier's path. Same
/// walk, same tiles, same circuit as [`grid_output_to_bounded`]; it just never materialises the
/// tree, which for a reconvergent datapath is exponentially larger than the array that computes
/// it. `None` when a tile has no propositional lowering.
pub fn grid_output_to_gates(
    g: &Grid,
    which: usize,
    width: u32,
    name_in: &dyn Fn(usize) -> String,
) -> Option<crate::codegen_sva::bitblast::Word> {
    grid_output_datapath(&mut GateBuild::new(width, name_in), g, which)
}

/// Lower a straight-line `LoopCell` body through the datapath vocabulary `b` — the placer's own
/// lowering vocabulary, and the SPECIFICATION side of the placement obligation. `None` for a cell
/// with no propositional lowering (a functional unit, a memory tile, a packed-SIMD tile).
pub fn cells_datapath<B: Datapath>(
    b: &mut B,
    cells: &[super::fabric_rtl::LoopCell],
    result: super::fabric_rtl::Src,
) -> Option<B::V> {
    use super::fabric_rtl::{LoopCell, Src as FSrc};
    fn src<B: Datapath>(b: &mut B, s: FSrc, vals: &[B::V]) -> Option<B::V> {
        Some(match s {
            FSrc::In(k) => b.input(k),
            FSrc::Val(k) => vals.get(k)?.clone(),
            FSrc::Const(c) => b.constant(c),
        })
    }
    let mut vals: Vec<B::V> = Vec::with_capacity(cells.len());
    for c in cells {
        let v = match c {
            LoopCell::Arith { op, a, b: rhs } => {
                let (x, y) = (src(b, *a, &vals)?, src(b, *rhs, &vals)?);
                b.arith(*op, x, y)
            }
            LoopCell::Select { cmp, ca, cb, t, e } => {
                let (ca, cb, t, e) = (
                    src(b, *ca, &vals)?,
                    src(b, *cb, &vals)?,
                    src(b, *t, &vals)?,
                    src(b, *e, &vals)?,
                );
                b.select(*cmp, ca, cb, t, e)
            }
            _ => return None,
        };
        vals.push(v);
    }
    src(b, result, &vals)
}

/// An ALU/select cell body lowered STRAIGHT INTO THE SHARED GATE DAG at `width`. Linear in the
/// CELLS: a cell read by three later cells is built once and shared by pointer, where the tree
/// form copies it three times (and their readers copy those, and so on — `fib.lg`'s 75-cell body
/// unfolds to 1 657 763 tree nodes from a few hundred).
pub fn cells_to_gates(
    cells: &[super::fabric_rtl::LoopCell],
    result: super::fabric_rtl::Src,
    width: u32,
) -> Option<crate::codegen_sva::bitblast::Word> {
    cells_datapath(
        &mut GateBuild::new(width, &|k| format!("in{k}")),
        cells,
        result,
    )
}

/// How many operand channels the switchbox must route into ONE tile for `cell`.
///
/// A global input or a constant operand is TAPPED at the tile for free (inputs fan out at no
/// cost); every other operand is another cell's output and has to arrive on one of the tile's two
/// routed channels — its north or its west. Two ports that read the SAME producer share one
/// channel, which is why `clamp`'s `If r is greater than 1000` (the comparison's left operand and
/// the else arm are both `r`) needs ONE channel and routes, while `pick`'s `If bit is 1` (a
/// computed predicate over two computed arms) needs THREE and does not.
pub fn tile_channel_demand(cell: &super::fabric_rtl::LoopCell) -> usize {
    routed_producers(cell).len()
}

/// A mux tile's operand channels: its north and its west. The interconnect gives each tile two.
pub const TILE_OPERAND_CHANNELS: usize = 2;

/// The distinct producer cells a tile's operands must have routed to it, in PORT ORDER — the
/// order the router assigns channels in, so `[0]` takes the north and `[1]` the west.
fn routed_producers(cell: &super::fabric_rtl::LoopCell) -> Vec<usize> {
    use super::fabric_rtl::{LoopCell, Src as FSrc};
    let ports: Vec<super::fabric_rtl::Src> = match *cell {
        LoopCell::Arith { a, b, .. } | LoopCell::Func { a, b, .. } => vec![a, b],
        LoopCell::VectorArith { a, b, .. } => vec![a, b],
        LoopCell::Select { ca, cb, t, e, .. } => vec![ca, cb, t, e],
        LoopCell::HeapRead { addr } => vec![addr],
        LoopCell::HeapWrite { addr, val } => vec![addr, val],
        LoopCell::HeapAlloc { size } => vec![size],
        LoopCell::MemRead { addr } => vec![addr],
    };
    let mut out: Vec<usize> = Vec::new();
    for p in ports {
        if let FSrc::Val(j) = p {
            if !out.contains(&j) {
                out.push(j);
            }
        }
    }
    out
}

/// The first cell of `cells` the router cannot place, as `(cell index, channels it demands)`.
/// `None` when every cell fits the interconnect. The gate a certifier consults BEFORE calling
/// [`route_loop_body`], so an unplaceable body is declined by name instead of panicking inside
/// the placer.
pub fn unroutable_cell(cells: &[super::fabric_rtl::LoopCell]) -> Option<(usize, usize)> {
    cells.iter().enumerate().find_map(|(i, c)| {
        let d = tile_channel_demand(c);
        (d > TILE_OPERAND_CHANNELS).then_some((i, d))
    })
}

/// Lower output `which` (in `all_outs` order) to a `BoundedExpr` at `width`, naming global inputs
/// via `name_in` — so the placed+routed array can be proven EQUAL to a reference expressed in the
/// SAME variable namespace (e.g. a loop body's `s0/iv/inv0`), exhaustively.
pub fn grid_output_to_bounded(
    g: &Grid,
    which: usize,
    width: u32,
    name_in: &dyn Fn(usize) -> String,
) -> crate::codegen_sva::sva_to_verify::BoundedExpr {
    grid_output_datapath(&mut BoundedBuild::new(width, name_in), g, which).expect(NO_BITBLAST_TILE)
}

/// **The placer + router.** Map a straight-line fabric (`FabricOp` SSA, from the real compiled
/// residual) onto the tile array. Placement: cell `i` computes on the diagonal at `(2(i+1), i)` —
/// its own column `i` carries its result downward. Routing: an operand that is a global input or
/// constant is TAPPED directly (inputs fan out for free); an operand that is another cell's value
/// is routed via a Manhattan path — forked EAST off its column (`he←N`, which coexists with the
/// column's downward `vs←N`, so fan-out costs nothing) and turned back into the consumer's north
/// (`vs←W`) or read from its west. Correct by construction; the S3 SAT proof catches any slip.
pub fn route_straight_line(
    cells: &[super::fabric_rtl::FabricOp],
    n_in: usize,
    result: usize,
) -> Grid {
    use super::fabric_rtl::Src as FSrc;
    let m = cells.len().max(1);
    let cols = m;
    let rows = 2 * m + 2;
    let r_of = |i: usize| 2 * (i + 1);
    let mut g = Grid::new(rows, cols, n_in);

    // 1) Place the compute tiles + start each result flowing down its own column.
    let operand = |s: FSrc| -> (Src2, Option<usize>) {
        match s {
            FSrc::In(k) => (Src2::In(k), None),
            FSrc::Const(c) => (Src2::Const(c), None),
            FSrc::Val(j) => (Src2::Zero, Some(j)), // placeholder src; filled by the route below
        }
    };
    for (i, cell) in cells.iter().enumerate() {
        let (ri, ci) = (r_of(i), i);
        let (mut sa, route_a) = operand(cell.a);
        let (mut sb, route_b) = operand(cell.b);
        // A routed operand arrives on the north (A) / west (B) channel of the compute tile.
        if route_a.is_some() {
            sa = Src2::N;
        }
        if route_b.is_some() {
            sb = Src2::W;
        }
        g.tiles[ri][ci].alu = Some(CellKind::Arith(cell.op, sa, sb));
        g.tiles[ri][ci].vs = Drive::Alu;

        // 2) Route operand A into the tile NORTH of the cell (row ri-1, col ci):
        //    fork cell `a`'s value east off column `a` at row ri-1, run it to column ci, turn south.
        if let Some(a) = route_a {
            let rr = ri - 1;
            g.tiles[rr][a].he = Drive::Pass(Src2::N); // fork east off column a (coexists with vs↓)
            for c in a + 1..ci {
                g.tiles[rr][c].he = Drive::Pass(Src2::W); // run east
            }
            g.tiles[rr][ci].vs = Drive::Pass(Src2::W); // turn south into the cell's north input
        }
        // 3) Route operand B into the tile WEST of the cell (row ri, col ci-1): fork east + run.
        if let Some(b) = route_b {
            let rr = ri;
            g.tiles[rr][b].he = Drive::Pass(Src2::N);
            for c in b + 1..ci {
                g.tiles[rr][c].he = Drive::Pass(Src2::W);
            }
        }
    }

    // 4) Every cell's value flows DOWN its column so later (lower) cells can fork it east.
    for i in 0..cells.len() {
        let (ri, ci) = (r_of(i), i);
        for r in ri + 1..rows {
            // Preserve any he route already placed here; only (re)assert the vertical channel.
            if matches!(g.tiles[r][ci].vs, Drive::Pass(Src2::Zero)) {
                g.tiles[r][ci].vs = Drive::Pass(Src2::N);
            }
        }
    }

    g.out = (r_of(result), result, Chan::Vs);
    g
}

/// Route a producer cell's value into the tile NORTH of a consumer at `(ri, ci)`: fork east off
/// the producer's column at row `ri-1`, run east, and turn south into the consumer's north input.
fn route_north(g: &mut Grid, from: Option<usize>, ci: usize, ri: usize) {
    if let Some(a) = from {
        let rr = ri - 1;
        g.tiles[rr][a].he = Drive::Pass(Src2::N);
        for c in a + 1..ci {
            g.tiles[rr][c].he = Drive::Pass(Src2::W);
        }
        g.tiles[rr][ci].vs = Drive::Pass(Src2::W);
    }
}

/// Route a producer cell's value onto the WEST input of a consumer at `(ri, ci)`: fork east off the
/// producer's column at row `ri`, run east to the consumer's west neighbour.
fn route_west(g: &mut Grid, from: Option<usize>, ci: usize, ri: usize) {
    if let Some(b) = from {
        let rr = ri;
        g.tiles[rr][b].he = Drive::Pass(Src2::N);
        for c in b + 1..ci {
            g.tiles[rr][c].he = Drive::Pass(Src2::W);
        }
    }
}

/// **Map a LOOP BODY onto the tile array.** Same diagonal placement + Manhattan routing as the
/// straight-line case, but each `LoopCell` may be a `Select` (a mux tile — the grid already models
/// it), and the body produces MULTIPLE outputs (one per loop-carried next-state register, in
/// `next` order). Global-input / constant operands are tapped directly; cell-value operands route
/// (each distinct producer takes one of the tile's two channels — a computed PREDICATE routes
/// exactly like a computed data arm, and two ports reading one producer share a channel). This
/// grid is the combinational BODY datapath of a loop-on-a-grid — the controller (counter / fabric
/// predicate) latches its outputs, exactly as with the hand-wired body.
pub fn route_loop_body(
    cells: &[super::fabric_rtl::LoopCell],
    n_in: usize,
    next: &[super::fabric_rtl::Src],
) -> Grid {
    use super::fabric_rtl::{LoopCell, Src as FSrc};
    let m = cells.len().max(1);
    let n_src_out = next.iter().filter(|s| !matches!(s, FSrc::Val(_))).count();
    let cols = m + n_src_out.max(0);
    let rows = 2 * m + 2;
    let r_of = |i: usize| 2 * (i + 1);
    let mut g = Grid::new(rows, cols.max(1), n_in);
    let operand = |s: FSrc| -> (Src2, Option<usize>) {
        match s {
            FSrc::In(k) => (Src2::In(k), None),
            FSrc::Const(c) => (Src2::Const(c), None),
            FSrc::Val(j) => (Src2::Zero, Some(j)),
        }
    };
    for (i, cell) in cells.iter().enumerate() {
        let (ri, ci) = (r_of(i), i);
        match *cell {
            LoopCell::Arith { op, a, b } => {
                let (mut sa, ra) = operand(a);
                let (mut sb, rb) = operand(b);
                if ra.is_some() {
                    sa = Src2::N;
                }
                if rb.is_some() {
                    sb = Src2::W;
                }
                g.tiles[ri][ci].alu = Some(CellKind::Arith(op, sa, sb));
                g.tiles[ri][ci].vs = Drive::Alu;
                route_north(&mut g, ra, ci, ri);
                route_west(&mut g, rb, ci, ri);
            }
            // A FUNCTIONAL-UNIT tile (division/mod/floor-div/pow/magic-high-mul/float) — placed exactly like an
            // Arith tile (its two operands route to north/west), but the tile drives a `FuncUnit` compute. The
            // emitted RTL is the FuncUnit's combinational form (`emit_decision_fabric_verilog`'s divider `a/b`),
            // so it settles in the same 1-clock pass; proven ≡ VM by eval + iverilog (NOT SAT — no `bvsdiv`).
            LoopCell::Func { unit, a, b } => {
                let (mut sa, ra) = operand(a);
                let (mut sb, rb) = operand(b);
                if ra.is_some() {
                    sa = Src2::N;
                }
                if rb.is_some() {
                    sb = Src2::W;
                }
                g.tiles[ri][ci].alu = Some(CellKind::Func(unit, sa, sb));
                g.tiles[ri][ci].vs = Drive::Alu;
                route_north(&mut g, ra, ci, ri);
                route_west(&mut g, rb, ci, ri);
            }
            // A packed-SIMD tile — placed exactly like an Arith tile (its two operands route north/west), but
            // the tile drives a `Simd` compute (lane-wise add/bitwise). Combinational; proven ≡ lane-wise + VM
            // by eval + iverilog (like `Func`, not SAT).
            LoopCell::VectorArith { op, ty, a, b } => {
                let (mut sa, ra) = operand(a);
                let (mut sb, rb) = operand(b);
                if ra.is_some() {
                    sa = Src2::N;
                }
                if rb.is_some() {
                    sb = Src2::W;
                }
                g.tiles[ri][ci].alu = Some(CellKind::Simd(op, ty, sa, sb));
                g.tiles[ri][ci].vs = Drive::Alu;
                route_north(&mut g, ra, ci, ri);
                route_west(&mut g, rb, ci, ri);
            }
            // STATE-PLANE tiles (Stage X2): heap read/write/alloc against ONE shared heap. Placed like an
            // Arith tile (operands route to north/west), but the tile drives a `Mem` compute that touches the
            // shared heap. Proven by `eval_grid_with_heap ≡ the model ≡ VM` (stateful → NOT SAT, NOT the
            // stateless eval_grid); the BRAM+controller RTL is the X2b follow-on (cell_v panics).
            LoopCell::HeapRead { addr } => {
                let (mut sa, ra) = operand(addr);
                if ra.is_some() {
                    sa = Src2::N;
                }
                g.tiles[ri][ci].alu = Some(CellKind::Mem(MemOp::HeapRead, sa, Src2::Zero));
                g.tiles[ri][ci].vs = Drive::Alu;
                route_north(&mut g, ra, ci, ri);
            }
            LoopCell::HeapWrite { addr, val } => {
                let (mut sa, ra) = operand(addr);
                let (mut sb, rb) = operand(val);
                if ra.is_some() {
                    sa = Src2::N;
                }
                if rb.is_some() {
                    sb = Src2::W;
                }
                g.tiles[ri][ci].alu = Some(CellKind::Mem(MemOp::HeapWrite, sa, sb));
                g.tiles[ri][ci].vs = Drive::Alu;
                route_north(&mut g, ra, ci, ri);
                route_west(&mut g, rb, ci, ri);
            }
            LoopCell::HeapAlloc { size } => {
                let (mut sa, ra) = operand(size);
                if ra.is_some() {
                    sa = Src2::N;
                }
                g.tiles[ri][ci].alu = Some(CellKind::Mem(MemOp::HeapAlloc, sa, Src2::Zero));
                g.tiles[ri][ci].vs = Drive::Alu;
                route_north(&mut g, ra, ci, ri);
            }
            // A loop-SEQUENCE read (the P2 scratchpad, induction-counter-addressed) is a distinct mechanism —
            // proven via eval_loop_fabric_seq, not placed as a state-plane tile here.
            LoopCell::MemRead { .. } => panic!(
                "grid: a loop-sequence MemRead is the induction-counter scratchpad (P2, eval_loop_fabric_seq), \
                 distinct from the Stage-X2 shared-heap state plane"
            ),
            // A mux tile reads FOUR operands, but the switchbox gives every tile the same TWO
            // routed channels (its north and its west). An input or a constant is tapped at the
            // tile for free; a COMPUTED operand — including a computed PREDICATE, which is an
            // ordinary dataflow edge and gets the ordinary treatment — must land on a channel,
            // and two ports reading the same producer share one. `unroutable_cell` prices this
            // from the cells before the placer is called, so a body that demands a third channel
            // is declined by name rather than caught here.
            LoopCell::Select { cmp, ca, cb, t, e } => {
                let routed = routed_producers(cell);
                assert!(
                    routed.len() <= TILE_OPERAND_CHANNELS,
                    "grid: a tile has {TILE_OPERAND_CHANNELS} operand channels (north and west) \
                     but this mux needs {} distinct computed values routed to it — check \
                     `unroutable_cell` before placing",
                    routed.len()
                );
                let chan = |s: FSrc| -> Src2 {
                    match s {
                        FSrc::In(k) => Src2::In(k),
                        FSrc::Const(c) => Src2::Const(c),
                        FSrc::Val(j) => {
                            if routed[0] == j {
                                Src2::N
                            } else {
                                Src2::W
                            }
                        }
                    }
                };
                g.tiles[ri][ci].alu =
                    Some(CellKind::Select(cmp, chan(ca), chan(cb), chan(t), chan(e)));
                g.tiles[ri][ci].vs = Drive::Alu;
                route_north(&mut g, routed.first().copied(), ci, ri);
                route_west(&mut g, routed.get(1).copied(), ci, ri);
            }
        }
    }
    for i in 0..cells.len() {
        let (ri, ci) = (r_of(i), i);
        for r in ri + 1..rows {
            if matches!(g.tiles[r][ci].vs, Drive::Pass(Src2::Zero)) {
                g.tiles[r][ci].vs = Drive::Pass(Src2::N);
            }
        }
    }
    // Outputs: one per next-state register. A `Val` reads its cell's tile; a pass-through state
    // (`In`/`Const`) gets a source tile in a spare column.
    let mut extra_col = m;
    let mut outs: Vec<(usize, usize, Chan)> = Vec::new();
    for s in next {
        match s {
            FSrc::Val(k) => outs.push((r_of(*k), *k, Chan::Vs)),
            FSrc::In(k) => {
                g.tiles[0][extra_col].vs = Drive::Pass(Src2::In(*k));
                outs.push((0, extra_col, Chan::Vs));
                extra_col += 1;
            }
            FSrc::Const(c) => {
                g.tiles[0][extra_col].vs = Drive::Pass(Src2::Const(*c));
                outs.push((0, extra_col, Chan::Vs));
                extra_col += 1;
            }
        }
    }
    // A SIDE-EFFECT-ONLY block (a heap write with no register write and an unconditional Jump) has no output
    // tap. The body's `Mem` tiles still execute (every tile is evaluated), but the `Grid` needs a primary
    // output; a dummy tap on the idle tile (0,0) suffices — the caller has no register write or branch cond
    // to read, so the value is never consumed. Non-empty `next` is unaffected.
    if outs.is_empty() {
        outs.push((0, 0, Chan::Vs));
    }
    g.out = outs[0];
    g.extra_outs = outs[1..].to_vec();
    g
}

// ── Time-multiplexing: one physical array runs MULTIPLE fabrics by reconfiguration ─────────────

/// The BoundedExpr for a single tile's two channel outputs `(vs, he)`, given its north/west channel
/// inputs — the shared per-tile evaluation used by both the single-grid and time-muxed lowerings.
fn tile_bounded(
    t: Tile,
    n: &crate::codegen_sva::sva_to_verify::BoundedExpr,
    w: &crate::codegen_sva::sva_to_verify::BoundedExpr,
    width: u32,
    name_in: &dyn Fn(usize) -> String,
) -> (
    crate::codegen_sva::sva_to_verify::BoundedExpr,
    crate::codegen_sva::sva_to_verify::BoundedExpr,
) {
    tile_outputs(&mut BoundedBuild::new(width, name_in), t, n, w).expect(NO_BITBLAST_TILE)
}

/// The combined output of a TIME-MULTIPLEXED array: one physical grid whose every tile is muxed by
/// a `ctx` selector between fabric `a`'s config (ctx=1) and fabric `b`'s config (ctx=0). Because
/// the neighbour channels are themselves ctx-muxed, when ctx=1 the whole array carries `a`'s
/// dataflow and when ctx=0 it carries `b`'s — proven in the P3 test. `a` and `b` must share dims.
pub fn time_mux_bounded(
    a: &Grid,
    b: &Grid,
    width: u32,
) -> crate::codegen_sva::sva_to_verify::BoundedExpr {
    use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
    assert!(
        a.rows == b.rows && a.cols == b.cols,
        "time-mux requires equal grid dimensions"
    );
    let name_in = |k: usize| format!("in{k}");
    let konst = |c: i64| BoundedExpr::BitVecConst {
        width,
        value: c as u64,
    };
    let is_a = BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Eq,
        left: Box::new(BoundedExpr::BitVecVar("ctx".to_string(), 1)),
        right: Box::new(BoundedExpr::BitVecConst { width: 1, value: 1 }),
    };
    let mux = |va: BoundedExpr, vb: BoundedExpr| BoundedExpr::BitVecIte {
        cond: Box::new(is_a.clone()),
        then: Box::new(va),
        else_: Box::new(vb),
    };
    let zero = konst(0);
    let mut vs = vec![vec![zero.clone(); a.cols]; a.rows];
    let mut he = vec![vec![zero.clone(); a.cols]; a.rows];
    for r in 0..a.rows {
        for c in 0..a.cols {
            let n = if r == 0 {
                zero.clone()
            } else {
                vs[r - 1][c].clone()
            };
            let w = if c == 0 {
                zero.clone()
            } else {
                he[r][c - 1].clone()
            };
            let (a_vs, a_he) = tile_bounded(a.tiles[r][c], &n, &w, width, &name_in);
            let (b_vs, b_he) = tile_bounded(b.tiles[r][c], &n, &w, width, &name_in);
            vs[r][c] = mux(a_vs, b_vs);
            he[r][c] = mux(a_he, b_he);
        }
    }
    let tap = |o: (usize, usize, Chan)| match o.2 {
        Chan::Vs => vs[o.0][o.1].clone(),
        Chan::He => he[o.0][o.1].clone(),
    };
    mux(tap(a.out), tap(b.out))
}

/// The RECONFIGURABLE array (Phase 5): ONE tile grid runs ANY of `grids` selected by a CONFIG WORD — the
/// generalization of [`time_mux_bounded`] from a 1-bit `ctx` (2 fabrics) to a `cfg_width`-bit word (N
/// fabrics). Every tile computes all N fabrics' drives over the SHARED muxed neighbour channels, then
/// selects by `config`; the output tap muxes likewise. This is "the fabric IS the interpreter": the config
/// word (streamed per block from the `LGB1` frame) reconfigures the SAME silicon to be any block's
/// datapath. `config` is fixed to a concrete value here so the lock proves, per value `k`, that the
/// reconfigured array is EXACTLY the bespoke island `grids[k]` (`config == k ⇒ array ≡ grids[k]`). All
/// grids must share dimensions (same-shape fabrics place identically).
pub fn time_mux_n_bounded(
    grids: &[&Grid],
    config: u64,
    width: u32,
    cfg_width: u32,
) -> crate::codegen_sva::sva_to_verify::BoundedExpr {
    use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
    assert!(!grids.is_empty(), "time-mux-n needs at least one fabric");
    let rows = grids[0].rows;
    let cols = grids[0].cols;
    for g in grids {
        assert!(
            g.rows == rows && g.cols == cols,
            "time-mux-n requires equal grid dimensions"
        );
    }
    let name_in = |k: usize| format!("in{k}");
    let konst = |c: i64| BoundedExpr::BitVecConst {
        width,
        value: c as u64,
    };
    // `config == k`, with `config` a concrete constant (the proof fixes it).
    let cfg_is = |k: usize| BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Eq,
        left: Box::new(BoundedExpr::BitVecConst {
            width: cfg_width,
            value: config,
        }),
        right: Box::new(BoundedExpr::BitVecConst {
            width: cfg_width,
            value: k as u64,
        }),
    };
    // A right-folded N-way mux: `config==0 ? v0 : config==1 ? v1 : … : v_{last}`.
    let mux_n = |vs: &[BoundedExpr]| -> BoundedExpr {
        let mut acc = vs[vs.len() - 1].clone();
        for k in (0..vs.len() - 1).rev() {
            acc = BoundedExpr::BitVecIte {
                cond: Box::new(cfg_is(k)),
                then: Box::new(vs[k].clone()),
                else_: Box::new(acc),
            };
        }
        acc
    };
    let zero = konst(0);
    let mut vs = vec![vec![zero.clone(); cols]; rows];
    let mut he = vec![vec![zero.clone(); cols]; rows];
    for r in 0..rows {
        for c in 0..cols {
            let n = if r == 0 {
                zero.clone()
            } else {
                vs[r - 1][c].clone()
            };
            let w = if c == 0 {
                zero.clone()
            } else {
                he[r][c - 1].clone()
            };
            let mut dvs = Vec::with_capacity(grids.len());
            let mut dhe = Vec::with_capacity(grids.len());
            for g in grids {
                let (gvs, ghe) = tile_bounded(g.tiles[r][c], &n, &w, width, &name_in);
                dvs.push(gvs);
                dhe.push(ghe);
            }
            vs[r][c] = mux_n(&dvs);
            he[r][c] = mux_n(&dhe);
        }
    }
    let tap = |o: (usize, usize, Chan)| match o.2 {
        Chan::Vs => vs[o.0][o.1].clone(),
        Chan::He => he[o.0][o.1].clone(),
    };
    let taps: Vec<BoundedExpr> = grids.iter().map(|g| tap(g.out)).collect();
    mux_n(&taps)
}

/// Certify the reconfigurable array END TO END (Phase 5): build one grid per source function (they must be
/// same-shape residuals so they place identically), then for EACH config value `k` prove via our certified
/// SAT that `config == k` reconfigures the ONE array to be EXACTLY the bespoke island `grids[k]`, for ALL
/// inputs. Returns the per-config verdicts — all `true` ⇒ the config word (streamed per block from the
/// bytecode frame) losslessly selects any block's datapath on one physical array: "the fabric IS the
/// interpreter". Each source is `(program, fn_name)` with a straight-line residual body.
pub fn certify_reconfig(sources: &[(&str, &str)], width: u32, cfg_width: u32) -> Vec<bool> {
    use crate::codegen_sva::bitblast::lower_bool;
    use crate::codegen_sva::fabric_rtl::residual_from_source;
    use crate::codegen_sva::sva_to_verify::BoundedExpr;
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    let grids: Vec<Grid> = sources
        .iter()
        .map(|(src, name)| {
            let (ops, n_in, res) = residual_from_source(src, name);
            route_straight_line(&ops, n_in, res)
        })
        .collect();
    let (r0, c0) = (grids[0].rows, grids[0].cols);
    for g in &grids {
        assert!(
            g.rows == r0 && g.cols == c0,
            "reconfig: all fabrics must place to the same array dimensions (use same-shape residuals)"
        );
    }
    let refs: Vec<&Grid> = grids.iter().collect();
    (0..grids.len())
        .map(|k| {
            let combined = time_mux_n_bounded(&refs, k as u64, width, cfg_width);
            let island = grid_to_bounded(&grids[k], width);
            // config == k  ⇒  array ≡ island k, for ALL inputs: prove ¬(array == island) is UNSAT.
            let eq = lower_bool(&BoundedExpr::Eq(Box::new(combined), Box::new(island)))
                .expect("bitvec equality lowers to a boolean");
            prove_unsat(&ProofExpr::Not(Box::new(eq))) == UnsatOutcome::Refuted
        })
        .collect()
}

/// Pad a placed grid to a larger `target_rows × target_cols` array: copy its tiles into the TOP-LEFT
/// corner and leave the rest IDLE. This RESOLVES the same-shape constraint the reconfigurable-array proof
/// needs (`time_mux_n_bounded` requires equal dimensions) — grids of DIFFERENT shapes co-reside on ONE
/// array without touching any datapath. The output taps keep their positions (valid since the array only
/// GREW), and the added IDLE tiles drive zero; because dataflow is feed-forward (south/east only), no
/// downstream idle tile ever feeds an upstream one, so the padded grid computes EXACTLY the original
/// (`eval_grid(pad_grid(g, R, C)) == eval_grid(g)`). Panics if asked to shrink.
pub fn pad_grid(g: &Grid, target_rows: usize, target_cols: usize) -> Grid {
    assert!(
        target_rows >= g.rows && target_cols >= g.cols,
        "pad_grid only grows an array (target {target_rows}x{target_cols} < {}x{})",
        g.rows,
        g.cols
    );
    let mut p = Grid::new(target_rows, target_cols, g.n_in);
    for r in 0..g.rows {
        for c in 0..g.cols {
            p.tiles[r][c] = g.tiles[r][c];
        }
    }
    p.out = g.out;
    p.extra_outs = g.extra_outs.clone();
    p
}

/// True iff every placed tile is `Arith`/`Select` (or pure routing) — i.e. the grid is fully COMBINATIONAL
/// and SAT-lowerable. A `Func` (division/float) tile has no bitblast lowering, so such a grid is proven by
/// eval ≡ VM + iverilog, not by `certify_reconfig`/`certify_config_table`; those refuse a non-combinational
/// grid rather than panic opaquely inside the SAT lowering.
pub fn grid_is_combinational(g: &Grid) -> bool {
    g.tiles.iter().flatten().all(|t| {
        !matches!(
            t.alu,
            Some(CellKind::Func(..)) | Some(CellKind::Mem(..)) | Some(CellKind::Simd(..))
        )
    })
}

/// Emit the time-multiplexed array as combinational RTL: every tile computes BOTH fabrics' drives
/// and muxes each channel on `ctx`; the result muxes the two fabrics' output taps. One silicon
/// array, two programs — selected by a configuration bit.
pub fn emit_time_multiplexed(name: &str, a: &Grid, b: &Grid) -> String {
    assert!(
        a.rows == b.rows && a.cols == b.cols,
        "time-mux requires equal grid dimensions"
    );
    let n_in = a.n_in.max(b.n_in);
    let inputs = (0..n_in)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let inputs = if inputs.is_empty() {
        String::new()
    } else {
        format!("{inputs}, ")
    };
    let mut body = String::new();
    for r in 0..a.rows {
        for c in 0..a.cols {
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
            for (tag, g) in [("a", a), ("b", b)] {
                let t = g.tiles[r][c];
                if let Some(k) = t.alu {
                    body.push_str(&format!(
                        "  wire [63:0] alu{tag}_{r}_{c} = {};\n",
                        cell_v(k, &nn, &ww)
                    ));
                }
                let drv = |d: Drive| match d {
                    Drive::Pass(s) => src2_v(s, &nn, &ww),
                    Drive::Alu => format!("alu{tag}_{r}_{c}"),
                };
                body.push_str(&format!("  wire [63:0] vs{tag}_{r}_{c} = {};\n", drv(t.vs)));
                body.push_str(&format!("  wire [63:0] he{tag}_{r}_{c} = {};\n", drv(t.he)));
            }
            body.push_str(&format!(
                "  wire [63:0] vs_{r}_{c} = ctx ? vsa_{r}_{c} : vsb_{r}_{c};\n"
            ));
            body.push_str(&format!(
                "  wire [63:0] he_{r}_{c} = ctx ? hea_{r}_{c} : heb_{r}_{c};\n"
            ));
        }
    }
    let out_wire = |g: &Grid| match g.out.2 {
        Chan::Vs => format!("vs_{}_{}", g.out.0, g.out.1),
        Chan::He => format!("he_{}_{}", g.out.0, g.out.1),
    };
    format!(
        "// tmux_{name} — ONE {rows}×{cols} CGRA tile array running TWO fabrics by reconfiguration:\n\
         // every tile is muxed on `ctx` between fabric A (ctx=1) and fabric B (ctx=0). Certified by\n\
         // our SAT: ctx=1 ⇒ the array ≡ fabric A, ctx=0 ⇒ ≡ fabric B.\n\
         module tmux_{name}({inputs}input ctx, output [63:0] result);\n\
         {body}  assign result = ctx ? {oa} : {ob};\n\
         endmodule\n",
        rows = a.rows,
        cols = a.cols,
        oa = out_wire(a),
        ob = out_wire(b),
    )
}

/// Emit the N-CONFIG reconfigurable array as combinational RTL — the synthesizable twin of
/// [`time_mux_n_bounded`]. ONE `rows×cols` tile grid runs ANY of `grids` selected by a `cfg`-bit config word:
/// every tile computes all N fabrics' drives over the SHARED (config-muxed) neighbour channels and selects by
/// `cfg`; the result muxes the N output taps. `grids` must share dimensions (pad with [`pad_grid`] first).
/// This is what a `fabric_soc` flashes: send `cfg` (the block index from the bytecode frame) + the inputs and
/// the ONE array becomes that block's datapath — proven semantics-preserving by `certify_config_table`.
pub fn emit_config_grid_verilog(name: &str, grids: &[&Grid], cfg_width: u32) -> String {
    assert!(!grids.is_empty(), "config grid needs at least one fabric");
    let rows = grids[0].rows;
    let cols = grids[0].cols;
    for g in grids {
        assert!(
            g.rows == rows && g.cols == cols,
            "config grid requires equal dims (pad_grid first)"
        );
    }
    let n_in = grids.iter().map(|g| g.n_in).max().unwrap();
    let inputs = (0..n_in)
        .map(|k| format!("input [63:0] in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let inputs = if inputs.is_empty() {
        String::new()
    } else {
        format!("{inputs}, ")
    };
    // A right-folded N-way mux: `cfg==0 ? v0 : cfg==1 ? v1 : … : v_{last}`.
    let mux_n = |vals: &[String]| -> String {
        let mut acc = vals[vals.len() - 1].clone();
        for k in (0..vals.len() - 1).rev() {
            acc = format!("(cfg == {cfg_width}'d{k}) ? {} : ({acc})", vals[k]);
        }
        acc
    };
    let mut body = String::new();
    for r in 0..rows {
        for c in 0..cols {
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
            let mut vsk = Vec::with_capacity(grids.len());
            let mut hek = Vec::with_capacity(grids.len());
            for (gi, g) in grids.iter().enumerate() {
                let t = g.tiles[r][c];
                if let Some(k) = t.alu {
                    body.push_str(&format!(
                        "  wire [63:0] alu{gi}_{r}_{c} = {};\n",
                        cell_v(k, &nn, &ww)
                    ));
                }
                let drv = |d: Drive| match d {
                    Drive::Pass(s) => src2_v(s, &nn, &ww),
                    Drive::Alu => format!("alu{gi}_{r}_{c}"),
                };
                body.push_str(&format!("  wire [63:0] vs{gi}_{r}_{c} = {};\n", drv(t.vs)));
                body.push_str(&format!("  wire [63:0] he{gi}_{r}_{c} = {};\n", drv(t.he)));
                vsk.push(format!("vs{gi}_{r}_{c}"));
                hek.push(format!("he{gi}_{r}_{c}"));
            }
            body.push_str(&format!("  wire [63:0] vs_{r}_{c} = {};\n", mux_n(&vsk)));
            body.push_str(&format!("  wire [63:0] he_{r}_{c} = {};\n", mux_n(&hek)));
        }
    }
    let out_wire = |g: &Grid| match g.out.2 {
        Chan::Vs => format!("vs_{}_{}", g.out.0, g.out.1),
        Chan::He => format!("he_{}_{}", g.out.0, g.out.1),
    };
    let taps: Vec<String> = grids.iter().map(|g| out_wire(g)).collect();
    // The emitted provenance must name the proof this table ACTUALLY has. `certify_config_table`
    // asserts out on a Func (division/float) tile — it has no bitblast lowering — so a Func-bearing
    // table is never SAT-certified, and a header saying otherwise is a claim nothing backs. Gate the
    // sentence on the same predicate the certifier asserts on, so the two can never diverge.
    let provenance = if grids.iter().all(|g| grid_is_combinational(g)) {
        "// Certified by our SAT (certify_config_table): cfg==k ⇒ the array ≡ fabric k.\n"
            .to_string()
    } else {
        "// Proven by eval ≡ VM + the iverilog differential. NOT SAT-certified: a Func (division/float)\n\
         // tile has no bitblast lowering, so the SAT reconfig certifier refuses a table with one in it.\n"
            .to_string()
    };
    format!(
        "// config_{name} — ONE {rows}×{cols} CGRA array running {n} fabrics by RECONFIGURATION: every tile is\n\
         // muxed on the `cfg` word among all {n} fabrics over the shared switchbox channels; the result muxes\n\
         // the {n} output taps.\n\
         {provenance}\
         // Feed-forward south/east, so combinational — the answer settles in ONE clock.\n\
         module config_{name}({inputs}input [{w}:0] cfg, output [63:0] result);\n\
         {body}  assign result = {res};\n\
         endmodule\n",
        n = grids.len(),
        w = cfg_width - 1,
        res = mux_n(&taps),
    )
}

/// The grid's area footprint: `(rows, cols, used_tiles)` — a tile is USED if it computes or drives
/// a non-zero channel. Used to compare placements.
pub fn grid_area(g: &Grid) -> (usize, usize, usize) {
    let mut used = 0;
    for row in &g.tiles {
        for t in row {
            let live = t.alu.is_some()
                || !matches!(t.vs, Drive::Pass(Src2::Zero))
                || !matches!(t.he, Drive::Pass(Src2::Zero));
            if live {
                used += 1;
            }
        }
    }
    (g.rows, g.cols, used)
}

/// **AREA-MINIMIZING placer (level scheduling).** The naive [`route_straight_line`] gives every
/// cell two rows (a routing row + a compute row). But a cell whose operands are all global inputs
/// or constants (topological LEVEL 0) needs NO routing row — its operands are tapped directly — so
/// all level-0 cells share ONE compute row (row 0), packed across columns. Deeper cells still get
/// their own routing+compute rows. For DAGs with input-parallelism this collapses the row count
/// (a chain saves a little; a wide fan-in tree saves a lot). Correctness is unchanged — the S3 SAT
/// proof validates the compact placement exactly as it did the naive one.
pub fn route_leveled(cells: &[super::fabric_rtl::FabricOp], n_in: usize, result: usize) -> Grid {
    use super::fabric_rtl::Src as FSrc;
    let m = cells.len().max(1);
    let cols = m;
    let has_val = |c: &super::fabric_rtl::FabricOp| {
        matches!(c.a, FSrc::Val(_)) || matches!(c.b, FSrc::Val(_))
    };
    // Level-0 (no Val operand) cells share row 0; every other cell gets a routing+compute pair.
    let mut compute_row = vec![0usize; cells.len()];
    let mut next = 1usize;
    for i in 0..cells.len() {
        if has_val(&cells[i]) {
            compute_row[i] = next + 1;
            next += 2;
        } else {
            compute_row[i] = 0;
        }
    }
    let rows = next.max(1);
    let mut g = Grid::new(rows, cols, n_in);
    let operand = |s: FSrc| -> (Src2, Option<usize>) {
        match s {
            FSrc::In(k) => (Src2::In(k), None),
            FSrc::Const(c) => (Src2::Const(c), None),
            FSrc::Val(j) => (Src2::Zero, Some(j)),
        }
    };
    for i in 0..cells.len() {
        let (ri, ci) = (compute_row[i], i);
        let (mut sa, ra) = operand(cells[i].a);
        let (mut sb, rb) = operand(cells[i].b);
        if ra.is_some() {
            sa = Src2::N;
        }
        if rb.is_some() {
            sb = Src2::W;
        }
        g.tiles[ri][ci].alu = Some(CellKind::Arith(cells[i].op, sa, sb));
        g.tiles[ri][ci].vs = Drive::Alu;
        // A routed operand's column carries its value down (via the vertical-continue below), so the
        // fork at the consumer's routing row / west neighbour resolves it — the helpers are reused.
        route_north(&mut g, ra, ci, ri);
        route_west(&mut g, rb, ci, ri);
    }
    for i in 0..cells.len() {
        let (ri, ci) = (compute_row[i], i);
        for r in ri + 1..rows {
            if matches!(g.tiles[r][ci].vs, Drive::Pass(Src2::Zero)) {
                g.tiles[r][ci].vs = Drive::Pass(Src2::N);
            }
        }
    }
    g.out = (compute_row[result], result, Chan::Vs);
    g
}

/// A self-checking testbench for the (combinational) tile array: drive each input vector, compare
/// `result` to the VM oracle from `grid_{name}_{in,out}.hex`.
pub fn emit_grid_testbench(name: &str, n_in: usize, n: usize) -> String {
    let decls = (0..n_in)
        .map(|k| format!("  reg  [63:0] in{k};"))
        .collect::<Vec<_>>()
        .join("\n");
    let ports = (0..n_in)
        .map(|k| format!(".in{k}(in{k})"))
        .collect::<Vec<_>>()
        .join(", ");
    let ports = if ports.is_empty() {
        String::new()
    } else {
        format!("{ports}, ")
    };
    let drive = (0..n_in)
        .map(|k| format!("in{k} = oin[i*{n_in}+{k}];"))
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        r#"// tb_grid_{name} — the placed+routed CGRA tile array vs the RUNNING VM's output.
`timescale 1ns/1ps
module tb_grid_{name};
{decls}
  reg  [63:0] oin [0:{ilast}];
  reg  [63:0] oout [0:{olast}];
  wire [63:0] result;
  integer i, errors = 0;
  grid_{name} dut({ports}.result(result));
  initial begin
    $readmemh("grid_{name}_in.hex", oin);
    $readmemh("grid_{name}_out.hex", oout);
    for (i = 0; i < {n}; i = i + 1) begin
      {drive} #1;
      if (result !== oout[i]) begin errors = errors + 1; $display("FAIL case %0d: got %0d exp %0d", i, result, oout[i]); end
    end
    if (errors == 0) $display("PASS: {n} tile-array evaluations match the VM (placed+routed CGRA)");
    else $display("FAIL: %0d grid mismatches", errors);
    $finish;
  end
endmodule
"#,
        ilast = (n * n_in).saturating_sub(1),
        olast = n - 1,
    )
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/fabric_grid/tests.rs"]
mod tests;
