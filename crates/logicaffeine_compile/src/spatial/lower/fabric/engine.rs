//! A fabric engine: one micro-program's sequencer, driving one of each datapath.
//!
//! An engine runs its micro-ops one after another, so it needs one datapath of each kind, not one per op. The
//! sequencer walks the micro-program as the twin does — `pc`, the phase `ph`, the loop counters — and each fixed op's
//! state sets, as constants, the parameters of the unit it runs on:
//!
//! - the tile walk, which steps a tile read's or write's element offset;
//! - the accumulator walk, which steps an element of the accumulator block through its row and column;
//! - the Generic unit: its axes, one offset per operand slot, the body's shape class, the contraction and the fold;
//! - the matmul unit: its load, weight, stream and output phases;
//! - the unary unit: its input and operand reads and its one multiplier.
//!
//! Those parameters are the op table, a register loaded on the clock that enters a state, so no decode of the state
//! stands in front of a datapath. Each state's successors are constants and the conditions choosing among them read
//! registers, so the table loads the row of the successor taken from a decode of the registered state, with those
//! conditions selecting among a few rows; the next state the sequencer computes from the same conditions is decoded by
//! nothing. Each unit holds the counters its addresses advance by, so no address divides, and keeps the cycle the op's
//! own datapath kept, so the twin's count is still the RTL's.
//!
//! A Generic body is a pipeline: each node registered once its operands are, a product twice, a floor division through
//! a divider a stage per quotient bit. A point enters it in its body cycle with a control word — whether it writes,
//! where, how it folds — that travels beside its value, so the contraction, the fold and the write happen where the
//! value leaves, as deep as its class. An op takes its points' periods and that depth (`Fixed::Generic::depth`).
//!
//! The accumulator is banked by column: bank `c` holds column `c` of every row of the block, so a matmul writes a
//! whole output row in one cycle, one bank each. Where the site's memories are banked, the walks move runs: a tile walk
//! a contiguous row's run a cycle, an acquire, a pack and a matmul's reload a run of a row's column banks, a matmul's A
//! and B loads a run of an operand row — each lane landing in the bank or register of its residue, so no lane is
//! selected by more than the run's start. Every other access is a single element at its row and column.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    addr_v, bits_for, buffer_uses, fold_step, from_phase, gather_lanes, index_read, item_coord_names, lit64, log2_banks, loop_vars,
    mcast_sends, mcast_valids, ph_less, resize_lanes, sem_incs, sems_local, tensor_width, tensors_read, tensors_written,
    var_name, Ctx, Emit, EnginePlan, Fixed, GenUnit, Guard, Micro, UnaryKind,
};
use crate::codegen_sva::systolic::pipelined_latency;
use crate::spatial::ir::*;
use crate::spatial::lower::SpatialLowerError;
use crate::spatial::passes::{LV_ITEM, LV_SUB_H, LV_SUB_W};
use crate::spatial::target::precision_int_bits;
use crate::spatial::value_range::{signed_bits, Interval};
use crate::tensor_recognize::ElementwiseOp;

/// `log2(n)` of a power of two.
fn log2(n: usize) -> u32 {
    n.trailing_zeros()
}

/// `v`, `width` bits wide, as a signed operand of a product kept at `keep` bits: its low `keep` bits where it is wider,
/// since no bit of an operand above the product's width reaches the product's bits.
fn fit(v: &str, width: u32, keep: u32) -> String {
    if width > keep {
        format!("$signed({v}[{}:0])", keep - 1)
    } else {
        format!("$signed({v})")
    }
}

/// The accumulator block every op of an acquired region works in, as tiles per row: a matmul's output block, else one
/// row of the region's tiles. Element `e = (i·cols + j)·te + r·tc + cc` of the block, tile-major, is the accumulator's
/// row `i·tr + r` and column `j·tc + cc`.
pub(super) fn acc_regions(micro: &[Micro]) -> Vec<Option<usize>> {
    let mut out = vec![None; micro.len()];
    let mut i = 0;
    while i < micro.len() {
        if let Micro::Fixed(Fixed::AcquireAcc { tiles, .. }) = micro[i] {
            let end = (i..micro.len())
                .find(|&k| matches!(micro[k], Micro::Fixed(Fixed::ReleaseAcc) | Micro::Then(Fixed::ReleaseAcc)))
                .unwrap_or(micro.len());
            let cols = micro[i..end]
                .iter()
                .find_map(|m| match m {
                    Micro::Fixed(Fixed::Matmul { block, .. }) => Some(block.osw),
                    _ => None,
                })
                .unwrap_or(tiles);
            for slot in out.iter_mut().take(end).skip(i) {
                *slot = Some(cols);
            }
            i = end;
        }
        i += 1;
    }
    out
}

/// A Generic body with its constants lifted out: ops of one engine whose bodies differ only in constants share one
/// datapath, their constants its parameters.
struct BodyClass {
    key: String,
    /// The first op of the class: its body and its axes, which every op of the class shares.
    unit: GenUnit,
}

/// The shape key of `b` (structure, operand slots, axis roles, comparison kinds) and its constants in walk order.
fn body_shape(b: &TensorBody, role: &dyn Fn(LoopVar) -> &'static str, params: &mut Vec<i64>) -> Result<String, SpatialLowerError> {
    let two = |tag: &str, x: &TensorBody, y: &TensorBody, params: &mut Vec<i64>| -> Result<String, SpatialLowerError> {
        Ok(format!("{tag}({},{})", body_shape(x, role, params)?, body_shape(y, role, params)?))
    };
    Ok(match b {
        TensorBody::Operand(i) => format!("o{i}"),
        TensorBody::Const(bits) => {
            params.push(f64::from_bits(*bits) as i64);
            "k".into()
        }
        TensorBody::Add(x, y) => two("add", x, y, params)?,
        TensorBody::Sub(x, y) => two("sub", x, y, params)?,
        TensorBody::Mul(x, y) => two("mul", x, y, params)?,
        TensorBody::Min(x, y) => two("min", x, y, params)?,
        TensorBody::Max(x, y) => two("max", x, y, params)?,
        TensorBody::FloorDiv(x, y) => two("fdiv", x, y, params)?,
        TensorBody::Shr { arg, by } => {
            let a = body_shape(arg, role, params)?;
            params.push(*by as i64);
            format!("shr({a},k)")
        }
        TensorBody::Select { cond, then, otherwise } => {
            let rhs = match cond.rhs() {
                PredRhs::Const(k) => {
                    params.push(k as i64);
                    "k".to_string()
                }
                PredRhs::Operand(i) => format!("o{i}"),
            };
            let cmp = match cond {
                Pred::AxisLt(..) => "lt",
                Pred::AxisGe(..) => "ge",
            };
            let t = body_shape(then, role, params)?;
            let o = body_shape(otherwise, role, params)?;
            format!("sel({cmp},{},{rhs},{t},{o})", role(cond.var()))
        }
        TensorBody::Unary { .. } => {
            return Err(SpatialLowerError::Unsupported("a transcendental in an integer body"))
        }
    })
}

/// Cycles from a pipelined floor divider's operands to its quotient: a stage for the magnitudes and the quotient's
/// sign, a stage per quotient bit — the `wq` bits the proof bounds the quotient to, never more than the dividend's `wa` —
/// and a stage for the floor.
pub fn floor_divider_latency(wa: u32, wq: u32) -> u64 {
    wa.min(wq) as u64 + 2
}

/// A pipelined signed floor divider `q = ⌊a / b⌋` — `a` of `wa` bits, `b` of `wb`, `q` of `wq` — taking a pair a cycle
/// and returning each quotient [`floor_divider_latency`] cycles after its operands.
///
/// It divides the magnitudes by restoring division, a quotient bit a stage, then floors: where the signs differ the
/// quotient is negated, and one less again where a remainder is left. A quotient proven to fit `wq` bits has no bit above
/// them, so the dividend's bits above them start the remainder and the stages walk only the `wq` below. A divisor of zero
/// is the evaluator's error; its quotient is whatever the stages make of it.
pub fn floor_divider_verilog(name: &str, wa: u32, wb: u32, wq: u32) -> String {
    let n = wa.min(wq);
    let mut e = Emit { s: String::new() };
    e.l(format!(
        "module {name}(input clk, input signed [{}:0] a, input signed [{}:0] b, output reg signed [{}:0] q);",
        wa - 1,
        wb - 1,
        wq - 1
    ));
    e.l(format!("  wire [{}:0] am = a[{}] ? -a : a;", wa - 1, wa - 1));
    e.l(format!("  wire [{}:0] bm = b[{}] ? -b : b;", wb - 1, wb - 1));
    e.l(format!("  reg [{}:0] r0, d0; reg [{}:0] x0; reg s0;", wb - 1, n - 1));
    let top = if wa > n { format!("am[{}:{n}]", wa - 1) } else { "0".to_string() };
    e.l(format!(
        "  always @(posedge clk) begin r0 <= {top}; x0 <= am[{}:0]; d0 <= bm; s0 <= a[{}] ^ b[{}]; end",
        n - 1,
        wa - 1,
        wb - 1
    ));
    for k in 1..=n {
        let p = k - 1;
        e.l(format!("  wire [{wb}:0] t{k} = {{r{p}, x{p}[{}]}};", n - 1));
        e.l(format!("  wire g{k} = t{k} >= {{1'b0, d{p}}};"));
        e.l(format!("  reg [{}:0] r{k}, d{k}; reg [{}:0] x{k}; reg s{k};", wb - 1, n - 1));
        let shift = if n == 1 { format!("g{k}") } else { format!("{{x{p}[{}:0], g{k}}}", n - 2) };
        e.l(format!(
            "  always @(posedge clk) begin r{k} <= g{k} ? t{k} - {{1'b0, d{p}}} : t{k}; x{k} <= {shift}; d{k} <= d{p}; s{k} <= s{p}; end"
        ));
    }
    e.l(format!(
        "  always @(posedge clk) q <= s{n} ? ((r{n} != 0) ? -$signed({{1'b0, x{n}}}) - 1 : -$signed({{1'b0, x{n}}})) : $signed({{1'b0, x{n}}});"
    ));
    e.l("endmodule");
    e.s
}

/// A body node's value in the pipeline: its wire, its width, and the stage it is ready at — counted from the body's
/// cycle, `None` for a parameter, which every op holds fixed.
struct Staged {
    wire: String,
    width: u32,
    ready: Option<u32>,
}

/// A class body printed as a pipeline, one wire `gn{class}_{j}{suffix}` per node `j` of [`TensorBody::post_order`] at its
/// width in `widths`. An operand leaf reads its slot's operand (`operand`) in the body's cycle (stage 0) and a constant its
/// parameter, in walk order; every other node is computed from its operands brought to one stage — a value ready
/// earlier through a register a stage — and registered: once, twice for a product, through a pipelined divider for a
/// floor division. A select's predicate is compared in the body's cycle, against the point's own axis. A lane of a
/// vector unit is a pipeline of its own, its wires suffixed with the lane, its parameters the class's.
///
/// ★ A SUM A PRODUCT WIDER THAN ONE MULTIPLIER SLICE READS STAYS OUT OF THE SLICE'S PRE-ADDER (`(* keep *)` on its
/// register). yosys 0.33 folds a registered sum into the pre-adder of a product's DSP slice; where the product is wider
/// than one slice ([`SiteSpec::multiplier`](crate::spatial::target::SiteSpec::multiplier)) and it splits it over a
/// cascade, it folds the sum into the top slice alone and wires the bottom slice's A operand to `x`: the bitstream then
/// multiplies a constant there. MEASURED on chip 0 of the split step: `(o_a_c0 + o_a_c1) · 144532`, two 19-bit operands
/// past the slice's 18-bit B port, computed `hi` wrong on the board while its RTL was the evaluator's. A product one
/// slice holds keeps its pre-adder: the sum costs no adder of its own.
struct Pipe<'a> {
    class: usize,
    axis: &'a dyn Fn(LoopVar) -> String,
    operand: &'a dyn Fn(usize) -> String,
    suffix: &'a str,
    widths: &'a [u32],
    /// The prefix of each divider module's name, unique to the engine.
    prefix: &'a str,
    /// The widest product one multiplier slice takes, `(wide, narrow)` signed operand bits.
    slice: Option<(u32, u32)>,
    next_param: usize,
    node: usize,
    lines: Vec<String>,
    modules: Vec<String>,
}

impl Pipe<'_> {
    fn param(&mut self) -> String {
        let n = self.next_param;
        self.next_param += 1;
        format!("gp{}_{n}", self.class)
    }

    fn take(&mut self) -> (usize, String, u32) {
        let j = self.node;
        self.node += 1;
        (j, format!("gn{}_{j}{}", self.class, self.suffix), self.widths[j])
    }

    /// The register of node wire `wire`, kept: synthesis leaves it a register of its own.
    fn keep(&mut self, wire: &str) {
        let decl = format!(" {wire}; always @(posedge clk) {wire} <= ");
        let line = self.lines.iter_mut().find(|l| l.starts_with("  reg signed [") && l.contains(&decl)).expect("a node's register");
        line.insert_str(2, "(* keep *) ");
    }

    /// `v` brought to stage `to` through a register a stage; a parameter as it is.
    fn at(&mut self, v: &Staged, to: u32) -> String {
        let Some(from) = v.ready else { return v.wire.clone() };
        let mut cur = v.wire.clone();
        for s in from + 1..=to {
            let d = format!("{}_d{s}", v.wire);
            self.lines.push(format!("  reg signed [{}:0] {d}; always @(posedge clk) {d} <= {cur};", v.width - 1));
            cur = d;
        }
        cur
    }

    fn walk(&mut self, b: &TensorBody) -> Staged {
        match b {
            TensorBody::Operand(i) => {
                let (_, wire, width) = self.take();
                self.lines.push(format!("  wire signed [{}:0] {wire} = $signed({});", width - 1, (self.operand)(*i)));
                Staged { wire, width, ready: Some(0) }
            }
            TensorBody::Const(_) => {
                let p = self.param();
                let (_, wire, width) = self.take();
                self.lines.push(format!("  wire signed [{}:0] {wire} = $signed({p});", width - 1));
                Staged { wire, width, ready: None }
            }
            TensorBody::Add(xb, yb) | TensorBody::Sub(xb, yb) | TensorBody::Mul(xb, yb) | TensorBody::Min(xb, yb) | TensorBody::Max(xb, yb) => {
                let (x, y) = (self.walk(xb), self.walk(yb));
                let (j, wire, width) = self.take();
                let s = x.ready.max(y.ready).unwrap_or(0);
                let (a, c) = (self.at(&x, s), self.at(&y, s));
                let w = width - 1;
                let (expr, ready) = match b {
                    TensorBody::Add(..) => (format!("$signed({a}) + $signed({c})"), s + 1),
                    TensorBody::Sub(..) => (format!("$signed({a}) - $signed({c})"), s + 1),
                    TensorBody::Mul(..) => {
                        let (wa, wb) = (x.width.min(width), y.width.min(width));
                        if self.slice.is_some_and(|(wide, narrow)| wa.max(wb) > wide || wa.min(wb) > narrow) {
                            for (child, operand) in [(xb, &x), (yb, &y)] {
                                if matches!(**child, TensorBody::Add(..) | TensorBody::Sub(..)) {
                                    self.keep(&operand.wire);
                                }
                            }
                        }
                        // The product is taken at its node's width: the bits of an operand past it reach none of the
                        // node's.
                        let m = format!("gm{}_{j}{}", self.class, self.suffix);
                        let (a, c) = (fit(&a, x.width, width), fit(&c, y.width, width));
                        self.lines.push(format!("  reg signed [{w}:0] {m}; always @(posedge clk) {m} <= {a} * {c};"));
                        (m, s + 2)
                    }
                    TensorBody::Min(..) => (format!("($signed({a}) < $signed({c})) ? $signed({a}) : $signed({c})"), s + 1),
                    _ => (format!("($signed({a}) > $signed({c})) ? $signed({a}) : $signed({c})"), s + 1),
                };
                self.lines.push(format!("  reg signed [{w}:0] {wire}; always @(posedge clk) {wire} <= {expr};"));
                Staged { wire, width, ready: Some(ready) }
            }
            TensorBody::FloorDiv(x, y) => {
                let (x, y) = (self.walk(x), self.walk(y));
                let (j, wire, width) = self.take();
                let s = x.ready.max(y.ready).unwrap_or(0);
                let (a, c) = (self.at(&x, s), self.at(&y, s));
                let module = format!("{}_fd{}_{j}", self.prefix, self.class);
                self.modules.push(floor_divider_verilog(&module, x.width, y.width, width));
                self.lines.push(format!("  wire signed [{}:0] {wire};", width - 1));
                self.lines.push(format!("  {module} u_{module}(.clk(clk), .a({a}), .b({c}), .q({wire}));"));
                Staged { wire, width, ready: Some(s + floor_divider_latency(x.width, width) as u32) }
            }
            TensorBody::Shr { arg, .. } => {
                let x = self.walk(arg);
                let p = self.param();
                let (_, wire, width) = self.take();
                let s = x.ready.unwrap_or(0);
                let a = self.at(&x, s);
                self.lines.push(format!("  reg signed [{}:0] {wire}; always @(posedge clk) {wire} <= $signed({a}) >>> {p};", width - 1));
                Staged { wire, width, ready: Some(s + 1) }
            }
            TensorBody::Select { cond, then, otherwise } => {
                let lhs = format!("$signed({{32'd0, {}}})", (self.axis)(cond.var()));
                let rhs = match cond.rhs() {
                    PredRhs::Const(_) => format!("$signed({})", self.param()),
                    PredRhs::Operand(i) => format!("$signed({})", (self.operand)(i)),
                };
                let op = match cond {
                    Pred::AxisLt(..) => "<",
                    Pred::AxisGe(..) => ">=",
                };
                let (t, o) = (self.walk(then), self.walk(otherwise));
                let (j, wire, width) = self.take();
                let pred = Staged { wire: format!("gs{}_{j}{}", self.class, self.suffix), width: 1, ready: Some(0) };
                self.lines.push(format!("  wire {} = {lhs} {op} {rhs};", pred.wire));
                let s = t.ready.max(o.ready).unwrap_or(0);
                let (p, a, c) = (self.at(&pred, s), self.at(&t, s), self.at(&o, s));
                self.lines.push(format!(
                    "  reg signed [{}:0] {wire}; always @(posedge clk) {wire} <= {p} ? $signed({a}) : $signed({c});",
                    width - 1
                ));
                Staged { wire, width, ready: Some(s + 1) }
            }
            TensorBody::Unary { .. } => unreachable!("refused by body_shape"),
        }
    }
}

/// The operand register of slot `s` in a one-lane unit.
fn scalar_operand(s: usize) -> String {
    format!("g_op{s}")
}

/// A class body's pipeline: its lines, its divider modules, its root, and its depth — the cycles from the body's cycle
/// to the root's value.
#[allow(clippy::too_many_arguments)]
fn class_pipeline(
    b: &TensorBody,
    class: usize,
    axis: &dyn Fn(LoopVar) -> String,
    operand: &dyn Fn(usize) -> String,
    suffix: &str,
    widths: &[u32],
    prefix: &str,
    slice: Option<(u32, u32)>,
) -> (Vec<String>, Vec<String>, String, u64) {
    let mut pipe = Pipe { class, axis, operand, suffix, widths, prefix, slice, next_param: 0, node: 0, lines: Vec::new(), modules: Vec::new() };
    let root = pipe.walk(b);
    (pipe.lines, pipe.modules, root.wire, root.ready.unwrap_or(0) as u64)
}

/// An engine's Generic bodies by class: ops whose bodies differ only in constants share one class.
struct Classes {
    classes: Vec<BodyClass>,
    /// Each Generic op's class and its parameter values, by micro-op index.
    op_class: BTreeMap<usize, (usize, Vec<i64>)>,
}

impl Classes {
    fn of(micro: &[Micro]) -> Result<Classes, SpatialLowerError> {
        let mut out = Classes { classes: Vec::new(), op_class: BTreeMap::new() };
        for (i, m) in micro.iter().enumerate() {
            let Micro::Fixed(Fixed::Generic { unit, .. }) = m else { continue };
            let role = axis_role(unit);
            let mut params = Vec::new();
            let key = body_shape(&unit.body, &role, &mut params)?;
            let class = match out.classes.iter().position(|c| c.key == key) {
                Some(c) => c,
                None => {
                    out.classes.push(BodyClass { key, unit: (**unit).clone() });
                    out.classes.len() - 1
                }
            };
            out.op_class.insert(i, (class, params));
        }
        Ok(out)
    }

    fn params_of(&self, class: usize) -> usize {
        self.op_class.values().filter(|(c, _)| *c == class).map(|(_, p)| p.len()).max().unwrap_or(0)
    }
}

/// The cycles a Generic point spends past its body: the inner fold, then the outer fold and the write, each a stage of
/// its own, so the contraction, the two folds and the write never share a cycle.
pub(super) const FOLD_STAGES: u64 = 2;

/// Each engine class's Generic ops given their depth: their body's pipeline, their lane tree's levels and the folds. The
/// class of every plan of an engine class is the representative's, its widths the join over the class's plans — what
/// [`print_engine`] prints.
pub(super) fn set_generic_depths(ctx: &Ctx, plans: &mut [EnginePlan]) -> Result<(), SpatialLowerError> {
    let engine_classes: BTreeSet<usize> = plans.iter().map(|p| p.class).collect();
    for ec in engine_classes {
        let rep = plans.iter().position(|p| p.class == ec).expect("a plan of the class");
        let cls = Classes::of(&plans[rep].micro)?;
        let depths: Vec<u64> = {
            let class_plans: Vec<&EnginePlan> = plans.iter().filter(|p| p.class == ec).collect();
            let wd = Widths::of(ctx, &class_plans, &cls);
            cls.classes
                .iter()
                .enumerate()
                .map(|(c, bc)| class_pipeline(&bc.unit.body, c, &|_| String::new(), &scalar_operand, "", &wd.nodes[c], "", ctx.multiplier).3)
                .collect()
        };
        for plan in plans.iter_mut().filter(|p| p.class == ec) {
            for (i, m) in plan.micro.iter_mut().enumerate() {
                if let Micro::Fixed(Fixed::Generic { unit, depth, .. }) = m {
                    *depth = depths[cls.op_class[&i].0] + unit.tree_levels() + FOLD_STAGES;
                }
            }
        }
    }
    Ok(())
}

/// What every unit of an engine is sized to: the accumulator's banks and rows, the Generic registers, axes and
/// classes, the matmul's array and operand rows.
struct Shape {
    banks: usize,
    rows: usize,
    slots: usize,
    /// The bits of a Generic axis counter: its extents, its initiation interval.
    axis_bits: usize,
    /// The deepest of the Generic ops' schedules: each slot's issue after its point's start, each slot's hold from its
    /// landing to its point's body cycle, and the body cycle itself.
    issue_taps: Vec<usize>,
    hold_taps: Vec<usize>,
    body_tap: usize,
    cls: Classes,
    array: Option<(usize, usize, u32)>,
    a_rows: usize,
    gathers: BTreeSet<usize>,
    /// The slots a vector op gathers through, each of whose lanes reads through a port of its own ([`gather_lanes`]).
    lane_gathers: BTreeSet<usize>,
    /// The widest run a Generic op takes a point: the unit's lanes, one where no op takes more.
    lanes: usize,
    /// Each body class's lanes: the widest run any of its ops takes.
    class_lanes: Vec<usize>,
    /// The levels of the lane tree: the most any op's lanes climb.
    tree_levels: usize,
    /// What the engine's ops whose lanes each keep their own column use past lane 0: the pools their folds fold by,
    /// whether any contracts, and whether any writes a parallel point (kind 0) or a fold's result (kind 2).
    lane_pools: [bool; 3],
    lane_contracts: bool,
    lane_writes_points: bool,
    lane_writes_folds: bool,
    /// Whether a tile walk of the engine fills a page whose rows keep a pitch ([`Ctx::page_pitch`]).
    pitched_fill: bool,
}

fn axis_role(g: &GenUnit) -> impl Fn(LoopVar) -> &'static str + '_ {
    move |v: LoopVar| {
        if v == g.row.var {
            "r"
        } else if v == g.col.var {
            "c"
        } else if g.k.is_some_and(|k| k.var == v) {
            "x"
        } else {
            "0"
        }
    }
}

/// An axis as lane `l` of a vector unit's body cycle sees it: its own coordinate along the lane axis, the point's along
/// any other.
fn lane_axis_wire(role: &'static str, l: usize) -> String {
    match role {
        "r" | "c" | "x" => format!("gb_{role}_l{l}"),
        _ => "32'd0".into(),
    }
}

/// An axis as the body cycle of a point sees it: the point's own coordinate, carried from its start.
fn axis_wire(role: &'static str) -> String {
    match role {
        "r" => "gb_r".into(),
        "c" => "gb_c".into(),
        "x" => "gb_x".into(),
        _ => "32'd0".into(),
    }
}

impl Shape {
    fn of(ctx: &Ctx, micro: &[Micro], regions: &[Option<usize>]) -> Result<Shape, SpatialLowerError> {
        let (te, tr, tc) = (ctx.te, ctx.tr, ctx.tc);
        let mut sh = Shape {
            banks: 1,
            rows: 1,
            slots: 0,
            axis_bits: 1,
            issue_taps: Vec::new(),
            hold_taps: Vec::new(),
            body_tap: 0,
            cls: Classes::of(micro)?,
            array: None,
            a_rows: 1,
            gathers: BTreeSet::new(),
            lane_gathers: BTreeSet::new(),
            lanes: 1,
            class_lanes: Vec::new(),
            tree_levels: 0,
            lane_pools: [false; 3],
            lane_contracts: false,
            lane_writes_points: false,
            lane_writes_folds: false,
            pitched_fill: micro.iter().any(|m| {
                matches!(m, Micro::Fixed(Fixed::ReadTiles { into, .. }) if ctx.page_pitch.contains_key(&ctx.template(*into)))
            }),
        };
        sh.class_lanes = vec![1; sh.cls.classes.len()];
        let need = |cols_t: Option<usize>, elems: usize, sh: &mut Shape| -> Result<(), SpatialLowerError> {
            let cols_t = cols_t.ok_or(SpatialLowerError::Unsupported("an accumulator op outside an acquired region"))?;
            let width = cols_t.max(1) * tc;
            sh.banks = sh.banks.max(width);
            sh.rows = sh.rows.max(elems.div_ceil(cols_t.max(1) * te) * tr);
            Ok(())
        };
        for (i, m) in micro.iter().enumerate() {
            let Micro::Fixed(f) = m else { continue };
            match f {
                Fixed::AcquireAcc { tiles, .. } | Fixed::Unary { tiles, .. } => need(regions[i], tiles * te, &mut sh)?,
                Fixed::PackAcc { tiles, .. } => need(regions[i], tiles * te, &mut sh)?,
                Fixed::Matmul { block, operand, array, reload, .. } => {
                    need(regions[i], block.osh * block.osw * te, &mut sh)?;
                    sh.banks = sh.banks.max(array.1);
                    let _ = reload;
                    let bits = precision_int_bits(*operand).unwrap_or(64);
                    let widest = sh.array.map_or(bits, |a| a.2.max(bits));
                    sh.array = Some((array.0, array.1, widest));
                    sh.a_rows = sh.a_rows.max(block.osh * tr);
                }
                Fixed::Generic { unit, tiles, .. } => {
                    if tr != 1 || tc != 1 {
                        return Err(SpatialLowerError::Unsupported("a Generic on a tile of more than one element"));
                    }
                    need(regions[i], tiles * te, &mut sh)?;
                    sh.slots = sh.slots.max(unit.slots());
                    for s in 0..unit.slots() {
                        if unit.gathers(s) {
                            sh.gathers.insert(s);
                            if unit.lanes > 1 {
                                sh.lane_gathers.insert(s);
                            }
                        }
                    }
                    sh.lanes = sh.lanes.max(unit.lanes);
                    let class = sh.cls.op_class[&i].0;
                    sh.class_lanes[class] = sh.class_lanes[class].max(unit.lanes);
                    sh.tree_levels = sh.tree_levels.max(unit.tree_levels() as usize);
                    if unit.lanes > 1 && !unit.tree() {
                        sh.lane_contracts |= unit.k.is_some();
                        match (unit.row.kind, unit.col.kind) {
                            (IterKind::Reduce { pool }, IterKind::Parallel) => {
                                sh.lane_pools[pool_index(pool)] = true;
                                sh.lane_writes_folds = true;
                            }
                            _ => sh.lane_writes_points = true,
                        }
                    }
                    let is = &unit.issue;
                    let most = [unit.row.extent, unit.col.extent, unit.k_extent(), is.ii].into_iter().max().unwrap_or(1);
                    sh.axis_bits = sh.axis_bits.max(bits_for(most + 1));
                    sh.body_tap = sh.body_tap.max(is.body);
                    sh.issue_taps.resize(sh.slots, 0);
                    sh.hold_taps.resize(sh.slots, 0);
                    for (s, at) in is.at.iter().enumerate() {
                        sh.issue_taps[s] = sh.issue_taps[s].max(*at);
                        sh.hold_taps[s] = sh.hold_taps[s].max(is.body - at - ctx.lat as usize - 1);
                    }
                }
                _ => {}
            }
        }
        Ok(sh)
    }
}

/// A Generic slot's offset walk: its base plus its walk along the axes — a delta as each one advances, `d0` along the
/// contraction, `d1` to the next middle point, `d2` to the next outer one — plus a gather's value times its stride,
/// read through the page as one row of its width, whose `lim` elements are the page.
struct SlotWalk {
    d0: i64,
    d1: i64,
    d2: i64,
    lim: i64,
}

impl SlotWalk {
    /// The walk in points: a step along the lane axis moves a run of the lanes.
    fn of(ctx: &Ctx, unit: &GenUnit, ins: &[BufferId], sl: usize) -> SlotWalk {
        let m = &unit.maps[sl];
        let (vr, vc, vk) = (unit.row.var, unit.col.var, unit.k.map(|k| k.var));
        let lv = unit.lane_var();
        let stride = |v: Option<LoopVar>| -> i64 {
            v.map_or(0, |v| m.terms.iter().filter(|(t, _)| *t == v).map(|(_, s)| *s).sum::<i64>() * if v == lv { unit.lanes as i64 } else { 1 })
        };
        let (sx, sr, scol) = (stride(vk), stride(Some(vr)), stride(Some(vc)));
        let (smid, sout) = if unit.rows_inner() { (sr, scol) } else { (scol, sr) };
        let nmid = if unit.rows_inner() { unit.points_along(vr) } else { unit.points_along(vc) };
        let kx1 = vk.map_or(1, |v| unit.points_along(v)) as i64 - 1;
        let width = (ctx.page_elems(ins[sl]) / ctx.te) * ctx.tr;
        SlotWalk {
            d0: sx,
            d1: smid - sx * kx1,
            d2: sout - smid * (nmid as i64 - 1) - sx * kx1,
            lim: (ctx.tr * width) as i64,
        }
    }

    /// Every offset the slot's walk takes: its map over the iteration space, a gather's term over every index `index`
    /// holds — and the walk itself, from zero.
    fn offsets(unit: &GenUnit, sl: usize, index: Option<Interval>) -> (Interval, Interval) {
        let m = &unit.maps[sl];
        let extent = |v: LoopVar| {
            if v == unit.row.var {
                unit.row.extent
            } else if v == unit.col.var {
                unit.col.extent
            } else if unit.k.is_some_and(|k| k.var == v) {
                unit.k_extent()
            } else {
                1
            }
        };
        // The lanes of a last point past the lane axis's end address past it too.
        let lv = unit.lane_var();
        let (mut lo, mut hi) = (0i128, 0i128);
        for (v, s) in &m.terms {
            let reach = if *v == lv { unit.points_along(*v) * unit.lanes } else { extent(*v) };
            let span = *s as i128 * (reach as i128 - 1);
            lo += span.min(0);
            hi += span.max(0);
        }
        let lin = (lo, hi);
        let (mut lo, mut hi) = (lo + m.base as i128, hi + m.base as i128);
        if let (Some(ind), true, Some(ix)) = (m.indirect, unit.gathers(sl), index) {
            let (a, b) = (ind.stride as i128 * ix.0 as i128, ind.stride as i128 * ix.1 as i128);
            lo += a.min(b);
            hi += a.max(b);
        }
        let fit = |(lo, hi): (i128, i128)| -> Interval {
            if lo < i64::MIN as i128 || hi > i64::MAX as i128 {
                (i64::MIN, i64::MAX)
            } else {
                (lo as i64, hi as i64)
            }
        };
        (fit(lin), fit((lo, hi)))
    }
}

/// A reduction pool's place among the three, as the fold units and the op table number them.
fn pool_index(p: ReducePool) -> usize {
    match p {
        ReducePool::Sum => 0,
        ReducePool::Max => 1,
        ReducePool::ArgMax => 2,
    }
}

/// What each register and wire of an engine class holds, as a signed width: the join, over every site the class runs
/// on, of the proven interval ([`crate::spatial::value_range::program_ranges`]) of every value it carries.
struct Widths {
    /// The accumulator banks, and every value written into them or read out of them.
    acc: u32,
    /// The Generic unit's operand registers: what each slot reads, or zero outside its page.
    slot: u32,
    /// The Generic unit's value: a body's result, a contraction's partial sums, a fold's registers, the write.
    value: u32,
    /// Each body class's nodes, in [`TensorBody::post_order`].
    nodes: Vec<Vec<u32>>,
    /// Each body class's parameters, by the constants the class's ops give them.
    params: Vec<Vec<u32>>,
    /// The unary unit: its input, its second operand (a bias, an elementwise operand), its product or sum, its result,
    /// and its multiplier.
    un_in: u32,
    un_b: u32,
    un_mul: u32,
    un_out: u32,
    un_m: u32,
    /// The Generic unit's offset walks: each slot's base, deltas, page bound, gather stride, walk and offset.
    addr: u32,
}

impl Widths {
    fn of(ctx: &Ctx, plans: &[&EnginePlan], cls: &Classes) -> Widths {
        let r = &ctx.ranges;
        let mut w = Widths {
            acc: 1,
            slot: 1,
            value: 1,
            nodes: cls.classes.iter().map(|c| vec![1; c.unit.body.post_order().len()]).collect(),
            params: (0..cls.classes.len()).map(|c| vec![1; cls.params_of(c)]).collect(),
            un_in: 1,
            un_b: 1,
            un_mul: 1,
            un_out: 1,
            un_m: 1,
            addr: 1,
        };
        let grow = |slot: &mut u32, iv: Option<Interval>| {
            if let Some(iv) = iv {
                *slot = (*slot).max(signed_bits(iv));
            }
        };
        let buf = |b: BufferId| r.buffers[b.idx()];
        // What a compute stage keeps — the whole of i64 where nothing proves less.
        let held = |s: StageId| Some(r.stages[s.idx()].held.unwrap_or((i64::MIN, i64::MAX)));
        let with_zero = |iv: Option<Interval>| iv.map(|(lo, hi)| (lo.min(0), hi.max(0)));
        for plan in plans {
            for (i, m) in plan.micro.iter().enumerate() {
                let Micro::Fixed(f) = m else { continue };
                match f {
                    Fixed::Matmul { stage, reload, .. } => {
                        grow(&mut w.acc, held(*stage));
                        grow(&mut w.acc, reload.and_then(buf));
                    }
                    Fixed::Unary { kind, ins, stage, .. } => {
                        let st = &r.stages[stage.idx()];
                        grow(&mut w.un_in, buf(ins[0]));
                        if let Some(b) = ins.get(1) {
                            grow(&mut w.un_b, buf(*b));
                        }
                        match kind {
                            UnaryKind::Requant { m, .. } => {
                                grow(&mut w.un_m, Some((*m, *m)));
                                grow(&mut w.un_mul, st.nodes.first().copied());
                            }
                            UnaryKind::Bias => grow(&mut w.un_mul, st.nodes.first().copied()),
                            UnaryKind::Elementwise(_) => grow(&mut w.un_mul, st.nodes.last().copied()),
                            UnaryKind::Relu => {}
                            UnaryKind::Scalar(_) => {
                                for x in [&mut w.un_in, &mut w.un_b, &mut w.un_mul, &mut w.un_out] {
                                    *x = 64;
                                }
                            }
                        }
                        grow(&mut w.un_out, held(*stage));
                        grow(&mut w.acc, held(*stage));
                    }
                    Fixed::Generic { unit, ins, stage, .. } => {
                        let st = &r.stages[stage.idx()];
                        for b in ins {
                            grow(&mut w.slot, with_zero(buf(*b)));
                        }
                        for (sl, m) in unit.maps.iter().enumerate() {
                            let walk = SlotWalk::of(ctx, unit, ins, sl);
                            let index = m.indirect.and_then(|ind| with_zero(buf(ins[ind.operand])));
                            let (lin, off) = SlotWalk::offsets(unit, sl, index);
                            for iv in [lin, off, (m.base, m.base), (walk.d0, walk.d0), (walk.d1, walk.d1), (walk.d2, walk.d2), (walk.lim, walk.lim)] {
                                grow(&mut w.addr, Some(iv));
                            }
                            if let Some(ind) = m.indirect {
                                grow(&mut w.addr, Some((ind.stride, ind.stride)));
                            }
                        }
                        let (class, _) = &cls.op_class[&i];
                        for (j, iv) in st.nodes.iter().enumerate() {
                            grow(&mut w.nodes[*class][j], Some(*iv));
                        }
                        grow(&mut w.value, held(*stage));
                        grow(&mut w.value, st.nodes.last().copied());
                        // A lane tree that sums holds the sum of a run of up to its lanes' values: no prefix of the fold,
                        // so none the fold's proof bounds.
                        if unit.tree() && unit.tree_sums() {
                            grow(&mut w.value, st.nodes.last().map(|iv| run_sum(*iv, unit.lanes)));
                        }
                        grow(&mut w.acc, held(*stage));
                    }
                    _ => {}
                }
            }
        }
        for (class, values) in cls.op_class.values() {
            for (j, v) in values.iter().enumerate() {
                grow(&mut w.params[*class][j], Some((*v, *v)));
            }
        }
        w
    }
}

/// Every sum of one to `n` values of `iv`, and zero: `[min(lo, n·lo, 0), max(hi, n·hi, 0)]`, the whole of i64 past it.
fn run_sum((lo, hi): Interval, n: usize) -> Interval {
    let fit = |v: i128| v.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
    let (lo, hi, n) = (lo as i128, hi as i128, n as i128);
    (fit(lo.min(n * lo).min(0)), fit(hi.max(n * hi).max(0)))
}

/// A table parameter's width that is the engine's address width, `AW`: the walks' offsets, bounds, strides and phases.
const AW_WIDE: usize = 0;

/// The engine class's Verilog module, printed from the representative plan.
pub(super) fn print_engine(
    ctx: &Ctx,
    plan: &EnginePlan,
    class_plans: &[&EnginePlan],
    name: &str,
    grid_names: &BTreeMap<usize, String>,
) -> Result<String, SpatialLowerError> {
    let p = ctx.p;
    let (te, tr, tc, lat) = (ctx.te, ctx.tr, ctx.tc, ctx.lat);
    if !te.is_power_of_two() || !tr.is_power_of_two() || !tc.is_power_of_two() {
        return Err(SpatialLowerError::Unsupported("a fabric tile whose sides are not powers of two"));
    }
    ctx.phase_flags.borrow_mut().clear();
    let micro = &plan.micro;
    let uses = buffer_uses(micro);
    let bufs: BTreeSet<BufferId> = uses.all();
    let tp = |b: BufferId| ctx.template(b).idx();
    let gathered = gather_lanes(ctx, micro);
    let t_rd = tensors_read(micro);
    let t_wr = tensors_written(micro);
    let s_local = sems_local(micro);
    let s_inc = sem_incs(micro);
    let m_valid = mcast_valids(micro);
    let m_send = mcast_sends(micro);
    let lvs = loop_vars(micro);
    let regions = acc_regions(micro);
    let sh = Shape::of(ctx, micro, &regions)?;
    let wd = Widths::of(ctx, class_plans, &sh.cls);
    let mut e = Emit { s: String::new() };
    // Each Generic class's pipeline — its lines, its dividers, its root and its depth — the dividers printed ahead of the
    // engine. Every op of a class was given the class's depth with its plan. A vector unit (`vl` lanes) prints a class's
    // pipeline once per lane its widest op takes, each against its lane's operands and coordinates.
    let vl = sh.lanes;
    let vec = vl > 1;
    let lane_pipes: Vec<Vec<(Vec<String>, Vec<String>, String, u64)>> = sh
        .cls
        .classes
        .iter()
        .enumerate()
        .map(|(c, cl)| {
            let role = axis_role(&cl.unit);
            if vec {
                (0..sh.class_lanes[c])
                    .map(|l| {
                        let axis = |v: LoopVar| lane_axis_wire(role(v), l);
                        let operand = |s: usize| format!("g_op{s}_l{l}");
                        class_pipeline(&cl.unit.body, c, &axis, &operand, &format!("_l{l}"), &wd.nodes[c], &format!("floor_div_{name}_l{l}"), ctx.multiplier)
                    })
                    .collect()
            } else {
                let axis = |v: LoopVar| axis_wire(role(v));
                vec![class_pipeline(&cl.unit.body, c, &axis, &scalar_operand, "", &wd.nodes[c], &format!("floor_div_{name}"), ctx.multiplier)]
            }
        })
        .collect();
    let pipes: Vec<&(Vec<String>, Vec<String>, String, u64)> = lane_pipes.iter().map(|lanes| &lanes[0]).collect();
    for (i, (c, _)) in &sh.cls.op_class {
        if let Micro::Fixed(Fixed::Generic { unit, depth, .. }) = &micro[*i] {
            debug_assert_eq!(*depth, pipes[*c].3 + unit.tree_levels() + FOLD_STAGES, "op {i} is as deep as its class's pipeline, its lane tree and the folds");
        }
    }
    for lanes in &lane_pipes {
        for (_, modules, _, _) in lanes {
            for m in modules {
                e.s.push_str(m);
            }
        }
    }

    // ── Ports ───────────────────────────────────────────────────────────────────────────────────────────────
    let mut ports: Vec<String> = vec!["input clk".into(), "input rst".into(), "input start".into(), "output reg done".into()];
    for b in &bufs {
        let pe = ctx.page_words(*b);
        let (sw, ew, cw) = (bits_for(ctx.depth(*b)), bits_for(pe), bits_for(ctx.depth(*b) + 1));
        let k = tp(*b);
        if uses.produces.contains(b) {
            ports.push(format!("output reg f{k}_reserve"));
            ports.push(format!("output reg [{cw}:0] f{k}_reserve_n"));
            ports.push(format!("output reg f{k}_push"));
            ports.push(format!("output reg [{cw}:0] f{k}_push_n"));
            ports.push(format!("input [{cw}:0] f{k}_free"));
        }
        if uses.consumes.contains(b) {
            ports.push(format!("output reg f{k}_pop"));
            ports.push(format!("output reg [{cw}:0] f{k}_pop_n"));
            ports.push(format!("input [{cw}:0] f{k}_count"));
        }
        let hi = (ctx.banks as u32 - 1) * ctx.buffer_width(*b);
        if uses.writes.contains(b) {
            ports.push(format!("output reg f{k}_wr_en"));
            ports.push(format!("output reg [{sw}:0] f{k}_wr_slot"));
            ports.push(format!("output reg [{ew}:0] f{k}_wr_elem"));
            ports.push(format!("output reg [63:0] f{k}_wr_data"));
            if ctx.banks > 1 {
                ports.push(format!("output reg [{}:0] f{k}_wr_n", log2_banks(ctx.banks)));
                ports.push(format!("output reg [3:0] f{k}_wr_lw"));
                ports.push(format!("output reg [{}:0] f{k}_wr_hi", hi - 1));
            }
            ports.push(format!("input [{sw}:0] f{k}_fill_slot0"));
        }
        if uses.reads.contains(b) {
            ports.push(format!("output reg [{sw}:0] f{k}_rd_slot"));
            ports.push(format!("output reg [{ew}:0] f{k}_rd_elem"));
            ports.push(format!("input [63:0] f{k}_rd_data"));
            for l in 1..gathered.get(&ctx.template(*b)).copied().unwrap_or(1) {
                ports.push(format!("output reg [{sw}:0] f{k}_l{l}_rd_slot"));
                ports.push(format!("output reg [{ew}:0] f{k}_l{l}_rd_elem"));
                ports.push(format!("input [63:0] f{k}_l{l}_rd_data"));
            }
            if ctx.banks > 1 {
                ports.push(format!("input [{}:0] f{k}_rd_hi", hi - 1));
            }
            ports.push(format!("input [{sw}:0] f{k}_rd_slot0"));
            ports.push(format!("input [{sw}:0] f{k}_fill_slot0_r"));
        }
    }
    let t_hi = |t: TensorId| (ctx.banks as u32 - 1) * tensor_width(p.tensor(t), &ctx.ranges);
    for t in &t_rd {
        ports.push(format!("output reg [31:0] t{}_rd_addr", t.idx()));
        ports.push(format!("input [63:0] t{}_rd_data", t.idx()));
        if ctx.banks > 1 {
            ports.push(format!("input [{}:0] t{}_rd_hi", t_hi(*t) - 1, t.idx()));
        }
    }
    for t in &t_wr {
        ports.push(format!("output reg t{}_wr_en", t.idx()));
        ports.push(format!("output reg [31:0] t{}_wr_addr", t.idx()));
        ports.push(format!("output reg [63:0] t{}_wr_data", t.idx()));
        if ctx.banks > 1 {
            ports.push(format!("output reg [{}:0] t{}_wr_n", log2_banks(ctx.banks), t.idx()));
            ports.push(format!("output reg [3:0] t{}_wr_lw", t.idx()));
            ports.push(format!("output reg [{}:0] t{}_wr_hi", t_hi(*t) - 1, t.idx()));
        }
    }
    for s in &s_local {
        ports.push(format!("input [15:0] s{}_count", s.idx()));
        ports.push(format!("output reg s{}_set", s.idx()));
        ports.push(format!("output reg [15:0] s{}_set_val", s.idx()));
    }
    for s in &s_inc {
        ports.push(format!("output reg s{}_inc_remote", s.idx()));
        ports.push(format!("output reg [15:0] s{}_inc_n", s.idx()));
    }
    for (s, g) in &m_valid {
        ports.push(format!("output reg mv{}_g{}", s.idx(), g.idx()));
    }
    for g in &m_send {
        ports.push(format!("output reg mc{}_send", g.idx()));
        ports.push(format!("output reg [79:0] mc{}_din", g.idx()));
    }
    // A chip program's root engine halts for the host: the continues the host has made, and where it waits.
    let links: Vec<(usize, usize, usize)> = micro
        .iter()
        .enumerate()
        .filter_map(|(i, m)| match m {
            Micro::Guarded(Guard::Link { point, ordinal }) => Some((i, *point, *ordinal)),
            _ => None,
        })
        .collect();
    if !links.is_empty() {
        ports.push("input [7:0] link_n".into());
        ports.push("output halted".into());
        ports.push("output [15:0] halt_at".into());
    }
    e.l(format!("module {name} #(parameter ITEM_START = 0, parameter ITEM_COUNT = 1) ("));
    e.l(format!("  {}", ports.join(",\n  ")));
    e.l(");");
    // The walks' width, one bit past the largest value any walk takes (computed once the op table's rows exist).
    e.l("  localparam AW = __AW__;");

    // ── Sequencer state ─────────────────────────────────────────────────────────────────────────────────────
    let pcw = bits_for(micro.len() + 1);
    e.l(format!("  reg [{pcw}:0] pc, pc_n;"));
    // The phase counts no further than the longest op lasts, and at least the sixteen bits a multicast's element takes.
    let longest = micro.iter().filter_map(|m| if let Micro::Fixed(f) = m { Some(ctx.duration(f)) } else { None }).max().unwrap_or(1);
    let phw = bits_for(longest as usize + 2).max(16);
    e.l(format!("  reg [{}:0] ph, ph_n;", phw - 1));
    // The op's last cycle, a register: its duration less one loaded as the op is entered and counted down, so nothing
    // decodes the state or compares the phase in front of the many units it resets.
    e.l(format!("  reg op_last; reg [{}:0] op_left;", phw - 1));
    e.l("  reg running, running_n, done_n;");
    if !links.is_empty() {
        // Halted while it waits at a halt the host has not continued it past; the halt's link, while it does.
        let waits: Vec<String> = links.iter().map(|(i, _, o)| format!("(pc == {i} && !(link_n > 8'd{o}))")).collect();
        e.l(format!("  assign halted = running && ({});", waits.join(" || ")));
        let at: String = links.iter().map(|(i, point, _)| format!("(pc == {i}) ? 16'd{point} : ")).collect();
        e.l(format!("  assign halt_at = {at}16'd0;"));
    }
    for v in &lvs {
        e.l(format!("  reg [31:0] {0}, {0}_n;", var_name(*v)));
    }
    let or_zero = |v: LoopVar| if lvs.contains(&v) { var_name(v) } else { "32'd0".into() };
    e.l(format!("  wire [31:0] lv5_or_zero = {};", or_zero(LV_ITEM)));
    e.l("  wire [31:0] item_id = ITEM_START + lv5_or_zero;");
    e.l(format!("  wire [31:0] oy = item_id / {};", ctx.grid_nx.max(1)));
    e.l(format!("  wire [31:0] ox = item_id % {};", ctx.grid_nx.max(1)));
    let sections_addressed: BTreeSet<usize> = micro
        .iter()
        .filter_map(|m| match m {
            Micro::Fixed(Fixed::ReadTiles { section, .. } | Fixed::WriteTiles { section, .. }) => *section,
            _ => None,
        })
        .collect();
    if !sections_addressed.is_empty() {
        let items = p.site_items();
        for k in &sections_addressed {
            let coords: Vec<(usize, usize)> =
                items.at(Some(*k), plan.site).iter().map(|it| p.work.items[it.idx()].coord).collect();
            for (axis, pick) in [("y", 0usize), ("x", 1usize)] {
                e.l(format!("  function [31:0] item_{axis}_s{k}(input [31:0] lv);"));
                let mut c = String::from("    case (lv)");
                for (n, xy) in coords.iter().enumerate() {
                    c.push_str(&format!(" {n}: item_{axis}_s{k} = {};", if pick == 0 { xy.0 } else { xy.1 }));
                }
                c.push_str(&format!(" default: item_{axis}_s{k} = 0; endcase"));
                e.l(c);
                e.l("  endfunction");
            }
        }
    }

    // ── Parameters ───────────────────────────────────────────────────────────────────────────────────────────
    // What moves with the phase or carries a port's data: driven by the decode of the current state.
    let g = sh.slots;
    let ax = sh.axis_bits;
    // The deepest tap of the control word: a class's pipeline, and the lane tree an op of it climbs.
    let dmax = sh
        .cls
        .op_class
        .iter()
        .filter_map(|(i, (c, _))| match &micro[*i] {
            Micro::Fixed(Fixed::Generic { unit, .. }) => Some(pipes[*c].3 + unit.tree_levels()),
            _ => None,
        })
        .chain(pipes.iter().map(|p| p.3))
        .max()
        .unwrap_or(0);
    let mut live: Vec<(String, usize, bool)> = vec![
        ("aw_step".into(), 1, false),
        ("sw_we".into(), 1, false),
        ("az_we".into(), 1, false),
        ("sw_d".into(), wd.acc as usize, true),
        ("unary_in".into(), wd.un_in as usize, false),
        ("unary_bias".into(), wd.un_b as usize, false),
        ("mu_ra".into(), sh.array.map_or(1, |a| a.2) as usize, false),
        ("mu_rb".into(), sh.array.map_or(1, |a| a.2) as usize, false),
        ("mu_rr".into(), wd.acc as usize, false),
    ];
    for s in 0..sh.slots {
        if vec {
            for l in 0..vl {
                live.push((format!("gu_ld{s}_l{l}"), wd.slot as usize, false));
            }
        } else {
            live.push((format!("gu_ld{s}"), wd.slot as usize, false));
        }
        live.push((format!("gu_pp{s}"), 1, false));
    }
    if let (Some((_, _, bits)), true) = (sh.array, ctx.runs()) {
        for j in 0..ctx.banks {
            live.push((format!("mu_rl{j}"), wd.acc as usize, true));
            live.push((format!("mu_al{j}"), bits as usize, true));
            live.push((format!("mu_bl{j}"), bits as usize, true));
        }
    }
    // The op table: every unit parameter an op holds fixed, loaded on the clock that enters the op's state from the
    // state it enters, so no decode of the state stands in front of the datapath it configures.
    let mut table: Vec<(String, usize, bool)> = vec![
        // a fixed op's state
        ("fx".into(), 1, false),
        // tile walk
        ("tw_on".into(), 1, false),
        ("tw_from".into(), AW_WIDE, false),
        ("tw_bw".into(), AW_WIDE, false),
        ("tw_d1".into(), AW_WIDE, true),
        ("tw_d2".into(), AW_WIDE, true),
        ("tw_vec".into(), 1, false),
        ("tw_rs".into(), AW_WIDE, false),
        ("tw_lw".into(), 4, false),
        ("tw_n0".into(), log2_banks(ctx.banks) as usize + 1, false),
    ];
    if sh.pitched_fill {
        // the padding past the end of a page row the walk fills
        table.push(("tw_pp".into(), AW_WIDE, false));
    }
    table.extend([
        // accumulator walk, and its run walk
        ("aw_cols".into(), AW_WIDE, false),
        ("ar_on".into(), 1, false),
        ("ar_e".into(), AW_WIDE, false),
        ("ar_dly".into(), 1, false),
        ("ar_lw".into(), 4, false),
        ("ar_n0".into(), log2_banks(ctx.banks) as usize + 1, false),
        ("pk_tr".into(), 1, false),
        // unary
        ("un_on".into(), 1, false),
        ("un_pe".into(), AW_WIDE, false),
        ("un_pb".into(), AW_WIDE, false),
        ("un_kind".into(), 4, false),
        ("un_m".into(), wd.un_m as usize, true),
        ("un_s".into(), 8, false),
        // multicast send
        ("mc_on".into(), 1, false),
        ("mc_pe".into(), AW_WIDE, false),
        // matmul
        ("mu_on".into(), 1, false),
        ("mu_p0".into(), AW_WIDE, false),
        ("mu_p1".into(), AW_WIDE, false),
        ("mu_p2".into(), AW_WIDE, false),
        ("mu_p3".into(), AW_WIDE, false),
        ("mu_rows".into(), AW_WIDE, false),
        ("mu_kop".into(), AW_WIDE, false),
        ("mu_nop".into(), AW_WIDE, false),
        ("mu_osw".into(), AW_WIDE, false),
        ("mu_ibw".into(), AW_WIDE, false),
        ("mu_bwrap".into(), AW_WIDE, false),
        ("mu_bjump".into(), AW_WIDE, false),
        ("mu_pn".into(), AW_WIDE, false),
        ("mr_on".into(), 1, false),
        ("mu_mv".into(), 1, false),
        ("mr_lw".into(), 4, false),
        ("mr_n0o".into(), log2_banks(ctx.banks) as usize + 1, false),
        ("mr_n0i".into(), log2_banks(ctx.banks) as usize + 1, false),
        // Generic
        ("gu_on".into(), 1, false),
        ("gu_ii".into(), ax, false),
        ("gu_kx".into(), ax, false),
        ("gu_nmid".into(), ax, false),
        ("gu_nout".into(), ax, false),
        ("gu_rinner".into(), 1, false),
        ("gu_cls".into(), 8, false),
        ("gu_kind".into(), 2, false),
        ("gu_pool".into(), 2, false),
        ("gu_rpool".into(), 2, false),
        ("gu_rinner_cx".into(), ax, false),
        ("gu_rx".into(), ax, false),
        ("gu_dep".into(), bits_for(dmax as usize + 1), false),
        ("gu_iss".into(), AW_WIDE, false),
        ("gu_body".into(), bits_for(sh.body_tap + 1), false),
    ]);
    if vec {
        // The op's lanes as a shift and as a count, the axis they run along (0 the row, 1 the column, 2 the contraction),
        // its last point along that axis and the lanes that point holds;
        // the level of the lane tree its value leaves at and the pool it folds by; the tap of the control word its lanes'
        // validity is read at, its class's depth; and the shift of the columns its lanes write, zero where one lane
        // writes.
        for (n, w) in [
            ("gu_lsh", 4),
            ("gu_lax", 2),
            ("gu_lanes", bits_for(vl + 1)),
            ("gu_llast", ax),
            ("gu_ltail", bits_for(vl + 1)),
            ("gu_tl", bits_for(sh.tree_levels + 1)),
            ("gu_tpool", 2),
            ("gu_cdep", bits_for(dmax as usize + 1)),
            ("gu_wsh", 4),
        ] {
            table.push((n.into(), w, false));
        }
    }
    for s in 0..g {
        let aw = wd.addr as usize;
        let (at, hold) = (bits_for(sh.issue_taps[s] + 1), bits_for(sh.hold_taps[s] + 1));
        for (n, w, sg) in [("base", aw, true), ("d0", aw, true), ("d1", aw, true), ("d2", aw, true), ("lim", aw, true), ("gon", 1, false), ("gst", aw, true), ("gop", 8, false), ("at", at, false), ("dl", hold, false)] {
            table.push((format!("gu_{n}{s}"), w, sg));
        }
        if vec {
            // Whether the slot reads one element a point, every lane's.
            table.push((format!("gu_bc{s}"), 1, false));
        }
    }
    for c in 0..sh.cls.classes.len() {
        for j in 0..sh.cls.params_of(c) {
            table.push((format!("gp{c}_{j}"), wd.params[c][j] as usize, true));
        }
    }
    for (n, w, sg) in live.iter().chain(&table) {
        let s = if *sg { "signed " } else { "" };
        let range = if *w == AW_WIDE { "[AW-1:0]".to_string() } else { format!("[{}:0]", w - 1) };
        e.l(format!("  reg {s}{range} {n};"));
    }

    // ── Units: registers ─────────────────────────────────────────────────────────────────────────────────────
    e.l("  // tile walk: element offset of a tile read or write");
    e.l("  reg [AW-1:0] tw_lo, tw_w, tw_off;");
    e.l("  // accumulator walk: the element's row and column in the accumulator block");
    e.l("  reg [AW-1:0] aw_cc, aw_r, aw_j, aw_i;");
    e.l(format!("  wire [AW-1:0] aw_row = (aw_i << {}) + aw_r;", log2(tr)));
    e.l(format!("  wire [AW-1:0] aw_col = (aw_j << {}) + aw_cc;", log2(tc)));
    e.l("  // unary and multicast walks");
    e.l("  reg [AW-1:0] un_e, un_e2, un_bk, un_bt, mc_e, mc_pg;");
    e.l(format!("  reg [{}:0] unary_out;", wd.un_out - 1));
    e.l("  wire mu_reload;");

    // The accumulator banks.
    let (banks, rows) = (sh.banks, sh.rows);
    e.l(format!("  // accumulator: {banks} banks of {rows} rows"));
    // A bank is a register file, written by the unit an op runs on and read whole by the matmul: never a LUT memory.
    for c in 0..banks {
        e.l(format!("  (* ram_style = \"registers\" *) reg signed [{}:0] acc_b{c} [0:{}];", wd.acc - 1, rows - 1));
    }
    // The single read port, at the accumulator walk's element.
    e.l(format!("  reg signed [{}:0] sr_d;", wd.acc - 1));
    {
        let mut r = String::from("  always @* begin sr_d = 0; case (aw_col)");
        for c in 0..banks {
            r.push_str(&format!(" {c}: sr_d = acc_b{c}[aw_row];"));
        }
        r.push_str(" default: ; endcase end");
        e.l(r);
    }
    // The run walk: a run of a row's column banks a cycle — its row, its first column, its place in the page and its
    // length — and the same `lat` cycles on, where an accumulating pack writes what it read.
    if ctx.runs() {
        let (l, lg) = (ctx.banks, log2_banks(ctx.banks));
        // A run's length and the banks it zeroes are registers, set a cycle ahead from where the walk goes next; at the
        // walk's start they are the op's own (`ar_n0`), so nothing between the walk and the banks is arithmetic.
        let banks = sh.banks;
        e.l("  reg [AW-1:0] ar_i, ar_j, ar_p, ar_r, ar_rr_r, ar_rp_r; reg ar_first;");
        e.l(format!("  reg [{lg}:0] ar_n_r; reg [{}:0] ar_z_r;", banks - 1));
        e.l("  wire [AW-1:0] ar_l = 32'd1 << ar_lw;");
        e.l(format!("  wire [{lg}:0] ar_n = ar_first ? ar_n0 : ar_n_r;"));
        let z0: Vec<String> = (0..banks).rev().map(|c| format!("(32'd{c} < ar_n0)")).collect();
        e.l(format!("  wire [{}:0] ar_z = ar_first ? {{{}}} : ar_z_r;", banks - 1, z0.join(", ")));
        // What is left of the row (`aw_cols - ar_j`) and of the page (`ar_e - ar_p`) are registers, the op's own at its
        // first cycle, so the walk advances on a compare of the run's width with a remainder, never on a sum. A run
        // starts at a multiple of its width, so its index in the row (`ar_r`) is its column over the width.
        e.l("  wire [AW-1:0] ar_rr = ar_first ? aw_cols : ar_rr_r, ar_rp = ar_first ? ar_e : ar_rp_r;");
        e.l("  wire ar_adv = (ar_l < ar_rr);");
        e.l("  wire [AW-1:0] ar_jn = ar_adv ? ar_j + ar_l : 32'd0, ar_pn = ar_p + ar_n;");
        e.l("  wire [AW-1:0] ar_rln = ar_adv ? ar_rr - ar_l : aw_cols, ar_ln = ar_rp - ar_n, ar_rn = ar_adv ? ar_r + 1 : 32'd0;");
        e.l("  wire [AW-1:0] ar_mn = (ar_rln < ar_ln) ? ar_rln : ar_ln;");
        e.l(format!("  wire [{lg}:0] ar_nn = (ar_mn < ar_l) ? ar_mn[{lg}:0] : ar_l[{lg}:0];"));
        // Bank `c` is in the next run when its word is the run's and its lane is below the run's length; a lane is
        // always below the run's width, so it is below the length exactly when it is below both next remainders. Each
        // side of the advance is decided from registers, and the advance only chooses between them.
        let zn: Vec<String> = (0..banks)
            .rev()
            .map(|c| {
                let (word, lane) = (format!("(32'd{c} >> ar_lw)"), format!("(32'd{c} & (ar_l - 1))"));
                format!(
                    "(ar_adv ? ({word} == ar_r + 1 && {lane} < ar_rr - ar_l && {lane} < ar_ln) : ({word} == 0 && {lane} < aw_cols && {lane} < ar_ln))"
                )
            })
            .collect();
        e.l(format!("  wire [{}:0] ar_zn = {{{}}};", banks - 1, zn.join(", ")));
        e.l("  always @(posedge clk)");
        e.l("    if (rst || !(running && ar_on) || op_last) begin ar_i <= 0; ar_j <= 0; ar_p <= 0; ar_r <= 0; ar_first <= 1'b1; end");
        e.l("    else begin ar_first <= 1'b0; ar_n_r <= ar_nn; ar_z_r <= ar_zn; ar_p <= ar_pn; ar_j <= ar_jn; ar_r <= ar_rn; ar_rr_r <= ar_rln; ar_rp_r <= ar_ln; if (!ar_adv) ar_i <= ar_i + 1; end");
        // Under simulation, in every cycle the walk runs: no run passes its page, and the banks a run zeroes are its own,
        // its word's lanes below its length. Past the page nothing reads what a run writes or zeroes, so no output
        // shows either broken; these checks do.
        let own: Vec<String> = (0..banks)
            .rev()
            .map(|c| format!("((32'd{c} >> ar_lw) == (ar_j >> ar_lw) && ((32'd{c} & (ar_l - 1)) < ar_n))"))
            .collect();
        e.l("  // synthesis translate_off");
        e.l("  always @(posedge clk) if (!rst && running && ar_on && ar_p + ar_n > ar_e) begin $display(\"FATAL: a run of %0d at %0d passes its page of %0d\", ar_n, ar_p, ar_e); $finish; end");
        e.l(format!(
            "  always @(posedge clk) if (!rst && running && ar_on && ar_z != {{{}}}) begin $display(\"FATAL: the banks a run zeroes are not its own: %b at column %0d, length %0d\", ar_z, ar_j, ar_n); $finish; end",
            own.join(", ")
        ));
        e.l("  // synthesis translate_on");
        if lat == 0 {
            e.l(format!("  wire [AW-1:0] ar_wi = ar_i, ar_wj = ar_j, ar_wp = ar_p; wire [{lg}:0] ar_wn = ar_n;"));
        } else {
            for d in 1..=lat {
                let src = |x: &str| if d == 1 { x.to_string() } else { format!("{x}{}", d - 1) };
                e.l(format!(
                    "  reg [AW-1:0] ar_i{d}, ar_j{d}, ar_p{d}; reg [{lg}:0] ar_n{d}; always @(posedge clk) begin ar_i{d} <= {}; ar_j{d} <= {}; ar_p{d} <= {}; ar_n{d} <= {}; end",
                    src("ar_i"),
                    src("ar_j"),
                    src("ar_p"),
                    src("ar_n")
                ));
            }
            e.l(format!(
                "  wire [AW-1:0] ar_wi = ar_dly ? ar_i{lat} : ar_i, ar_wj = ar_dly ? ar_j{lat} : ar_j, ar_wp = ar_dly ? ar_p{lat} : ar_p; wire [{lg}:0] ar_wn = ar_dly ? ar_n{lat} : ar_n;"
            ));
        }
        // Lane `j` of the run: column `ar_wj + j`, as an I32 plan keeps it. A run that reaches lane `j` is wider than `j`,
        // so it starts at a multiple of `j`'s next power of two: the lane chooses among those columns only.
        let acc = wd.acc;
        for j in 0..l {
            let step = (j + 1).next_power_of_two();
            let mut r = format!("  reg signed [{}:0] pk_a{j}; always @* case (ar_wj >> {})", acc - 1, step.trailing_zeros());
            for m in 0..banks.div_ceil(step) {
                if m * step + j < banks {
                    r.push_str(&format!(" {m}: pk_a{j} = acc_b{}[ar_wi];", m * step + j));
                }
            }
            r.push_str(&format!(" default: pk_a{j} = 0; endcase"));
            e.l(r);
            let t = if acc > 32 {
                format!("pk_tr ? {{{{{}{{pk_a{j}[31]}}}}, pk_a{j}[31:0]}} : pk_a{j}", acc - 32)
            } else {
                format!("pk_a{j}")
            };
            e.l(format!("  wire signed [{}:0] pk_t{j} = {t};", acc - 1));
        }
        // Each page a pack writes: the run's lanes at the page's width, an accumulating pack's added to what it read.
        let packed: BTreeSet<BufferId> = micro
            .iter()
            .filter_map(|m| match m {
                Micro::Fixed(Fixed::PackAcc { to, .. }) => Some(ctx.template(*to)),
                _ => None,
            })
            .collect();
        let read: BTreeSet<BufferId> = uses.reads.iter().map(|b| ctx.template(*b)).collect();
        for tpl in &packed {
            let (k, w) = (tpl.idx(), ctx.buffer_width(*tpl));
            for j in 0..l {
                let page = if !read.contains(tpl) {
                    "0".to_string()
                } else if j == 0 {
                    format!("(ar_dly ? $signed(f{k}_rd_data) : 0)")
                } else {
                    format!("(ar_dly ? $signed(f{k}_rd_hi[{}:{}]) : 0)", j as u32 * w - 1, (j as u32 - 1) * w)
                };
                e.l(format!("  wire signed [{}:0] pk{k}_v{j} = $signed(pk_t{j}) + {page};", w - 1));
            }
        }
    }

    // The PE array of a matmul engine.
    let pe = sh.array;
    if let Some((k, n, bits)) = pe {
        e.l("  reg pe_w_load, pe_w_load_r;");
        e.l(format!("  reg [{}:0] pe_w_row;", bits_for(k) - 1));
        e.l(format!("  reg [{}:0] pe_w_data;", n as u32 * bits - 1));
        for r in 0..k {
            e.l(format!("  reg signed [63:0] pe_a{r};"));
        }
        for c in 0..n {
            e.l(format!("  wire signed [63:0] pe_o{c};"));
        }
        let mut conn = vec![
            ".clk(clk)".to_string(),
            ".rst(rst)".to_string(),
            ".w_load(pe_w_load)".to_string(),
            ".w_row(pe_w_row)".to_string(),
            ".w_data(pe_w_data)".to_string(),
        ];
        conn.extend((0..k).map(|r| format!(".a{r}(pe_a{r})")));
        conn.extend((0..n).map(|c| format!(".o{c}(pe_o{c})")));
        e.l(format!("  pe_{name} u_pe({});", conn.join(", ")));
        e.l("  always @* pe_w_load = pe_w_load_r;");
        // Operand rows: the A rows the array streams and the B rows it loads as weights, each element at its width.
        let ar = sh.a_rows;
        for s in 0..ar {
            for x in 0..k {
                e.l(format!("  reg signed [{}:0] a_r{s}_{x};", bits - 1));
            }
        }
        for r in 0..k {
            e.l(format!("  reg [{}:0] b_r{r};", n as u32 * bits - 1));
        }
        e.l("  reg [AW-1:0] ma_xx, ma_r, ma_kk, ma_i, mb_cc, mb_x, mb_j, mb_kk, mbi_t, mbi_off;");
    }

    // The Generic unit: the walk that starts a point every `gu_ii` cycles; each slot's read on its schedule, its value
    // latched as it lands and held to its point's body cycle; each class's pipeline; and what the fold and the write
    // need of a point, carried down the pipeline beside its value.
    if g > 0 {
        let axb = ax - 1;
        let aw = wd.addr;
        // A value carried `taps` cycles in a register a cycle, read at the tap an op's parameter names.
        let delay = |e: &mut Emit, name: &str, width: usize, taps: usize, sel: &str, first: &str| {
            for d in 1..=taps {
                let prev = if d == 1 { first.to_string() } else { format!("{name}_{}", d - 1) };
                e.l(format!("  reg [{}:0] {name}_{d}; always @(posedge clk) {name}_{d} <= {prev};", width - 1));
            }
            let mut t = format!("  reg [{}:0] {name}; always @* case ({sel}) 0: {name} = {first};", width - 1);
            for d in 1..=taps {
                t.push_str(&format!(" {d}: {name} = {name}_{d};"));
            }
            t.push_str(&format!(" default: {name} = {first}; endcase"));
            e.l(t);
        };
        e.l(format!("  reg signed [{}:0] g_kacc, g_fold, g_ofold;", wd.value - 1));
        e.l(format!("  reg [{axb}:0] g_fidx, g_oidx;"));
        e.l(format!("  reg [{axb}:0] gu_sub, gu_x, gu_mid, gu_out;"));
        e.l(format!("  wire [{axb}:0] gu_r = gu_rinner ? gu_mid : gu_out;"));
        e.l(format!("  wire [{axb}:0] gu_c = gu_rinner ? gu_out : gu_mid;"));
        // A point starts: within the op's issue phases, at the head of each interval.
        e.l("  wire gs_v = gu_on && (ph < gu_iss) && (gu_sub == 0);");
        // The point's coordinates, carried to its body cycle — and in a vector unit, above them, which of its lanes are
        // the op's: every lane below the op's lanes, and of the last point along the lane axis those below its tail. The
        // point's own counters decide it at its start, so its body cycle compares nothing.
        let lane_start = if vec {
            e.l(format!("  wire [{axb}:0] gu_li = (gu_lax == 2'd0) ? gu_r : (gu_lax == 2'd1) ? gu_c : gu_x;"));
            e.l("  wire gu_lat_last = (gu_li == gu_llast);");
            let bits: Vec<String> = (0..vl).rev().map(|l| format!("(gu_lat_last ? ({l} < gu_ltail) : ({l} < gu_lanes))")).collect();
            // The run's first coordinate along the lane axis, a multiple of the op's lanes.
            e.l(format!("  wire [{}:0] gs_lbase = {{4'd0, gu_li}} << gu_lsh;", ax + 3));
            format!("gs_lbase, {}, ", bits.join(", "))
        } else {
            String::new()
        };
        let gbw = 3 * ax + 1 + if vec { vl + ax + 4 } else { 0 };
        e.l(format!("  wire [{}:0] gb0 = {{{lane_start}gu_c, gu_r, gu_x, gs_v}};", gbw - 1));
        delay(&mut e, "gbo", gbw, sh.body_tap, "gu_body", "gb0");
        e.l("  wire gb_v = gu_on && gbo[0] && (ph >= gu_body);");
        e.l(format!("  wire [{axb}:0] gb_x = gbo[{}:1], gb_r = gbo[{}:{}], gb_c = gbo[{}:{}];", ax, 2 * ax, ax + 1, 3 * ax, 2 * ax + 1));
        // A vector unit's lanes: each lane's coordinates — its point's, and along the lane axis the run's first
        // coordinate with the lane in its low bits, which a run's start, a multiple of the op's lanes, leaves clear for
        // every lane of the op — and whether it is one of the op's, as its point carries it.
        let cwd = ax + 4;
        if vec {
            e.l(format!("  wire [{}:0] gb_lbase = gbo[{}:{}];", cwd - 1, 3 * ax + vl + cwd, 3 * ax + 1 + vl));
            for l in 0..vl {
                for (role, code, src) in [("r", 0, "gb_r"), ("c", 1, "gb_c"), ("x", 2, "gb_x")] {
                    e.l(format!("  wire [{0}:0] gb_{role}_l{l} = (gu_lax == 2'd{code}) ? (gb_lbase | {cwd}'d{l}) : {{4'd0, {src}}};", cwd - 1));
                }
                e.l(format!("  wire gb_lv{l} = gbo[{}];", 3 * ax + 1 + l));
            }
        }
        for s in 0..g {
            e.l(format!("  reg signed [{}:0] gu_lin{s};", aw - 1));
            if vec {
                for l in 0..vl {
                    e.l(format!("  reg signed [{}:0] gv{s}_l{l};", wd.slot - 1));
                }
            } else {
                e.l(format!("  reg signed [{}:0] gv{s};", wd.slot - 1));
            }
        }
        for s in 0..g {
            // The slot's read: its point's start, the select its point takes where the slot shares a read, and its walk
            // offset, `gu_at` cycles on.
            e.l(format!("  wire [{}:0] gi{s}_0 = {{gs_v, gu_pp{s}, gu_lin{s}}};", aw + 1));
            delay(&mut e, &format!("gi{s}"), aw as usize + 2, sh.issue_taps[s], &format!("gu_at{s}"), &format!("gi{s}_0"));
            e.l(format!("  wire gi_v{s} = gu_on && gi{s}[{}] && (ph >= gu_at{s}), gi_pp{s} = gi{s}[{aw}];", aw + 1));
            // A gather adds the value its index slot latched, times its stride — in an op where it gathers: an operand
            // register no op has written yet holds no value, and no stride of zero makes one. The index and the product
            // are nodes of the offset's width, which is all of either the offset keeps. The product is a register, so
            // the slot reads a cycle after its index latched (`Issue::of`) and no multiply stands in front of its
            // address.
            let gather = if sh.gathers.contains(&s) {
                let mut idx = String::new();
                for j in 0..g {
                    let gv = if vec { format!("gv{j}_l0") } else { format!("gv{j}") };
                    idx.push_str(&format!("(gu_gop{s} == 8'd{j}) ? $signed({gv}) : "));
                }
                idx.push('0');
                e.l(format!("  wire signed [{0}:0] gu_gx{s} = {idx}; reg signed [{0}:0] gu_gm{s};", aw - 1));
                e.l(format!("  always @(posedge clk) gu_gm{s} <= gu_gst{s} * gu_gx{s};"));
                format!(" + (gu_gon{s} ? gu_gm{s} : 0)")
            } else {
                String::new()
            };
            e.l(format!("  wire signed [{}:0] gu_off{s} = gu_base{s} + $signed(gi{s}[{}:0]){gather};", aw - 1, aw - 1));
            e.l(format!("  wire gu_in{s} = (gu_off{s} >= 0) && (gu_off{s} < gu_lim{s});"));
            let el = if aw > 32 { format!("gu_off{s}[31:0]") } else { format!("gu_off{s}") };
            e.l(format!("  wire [31:0] gu_el{s} = {el};"));
            // Its value lands `lat` cycles after the read, zero where the read fell outside the page.
            if !vec {
                if lat == 0 {
                    e.l(format!("  wire gl_v{s} = gi_v{s}, gl_in{s} = gu_in{s};"));
                } else {
                    for d in 1..=lat {
                        let prev = if d == 1 { format!("gi_v{s}, gu_in{s}") } else { format!("gl_v{s}_{0}, gl_in{s}_{0}", d - 1) };
                        e.l(format!("  reg gl_v{s}_{d}, gl_in{s}_{d}; always @(posedge clk) {{gl_v{s}_{d}, gl_in{s}_{d}}} <= {{{prev}}};"));
                    }
                    e.l(format!("  wire gl_v{s} = gl_v{s}_{lat}, gl_in{s} = gl_in{s}_{lat};"));
                }
                e.l(format!("  always @(posedge clk) if (gl_v{s}) gv{s} <= gl_in{s} ? $signed(gu_ld{s}) : 0;"));
                // Held to the point's body cycle.
                delay(&mut e, &format!("g_op{s}"), wd.slot as usize, sh.hold_taps[s], &format!("gu_dl{s}"), &format!("gv{s}"));
                continue;
            }
            // A run's lane `l` reads element `l` past the run's start: inside the page or zero, as one element is. A slot
            // that reads one element a point gives every lane that element. A slot a vector op gathers through reads, in
            // lane `l`, the element its index slot's lane `l` names: its own offset, through a port of its own.
            let lane_gather = sh.lane_gathers.contains(&s);
            for l in 1..vl {
                if lane_gather {
                    let mut idx = String::new();
                    for j in 0..g {
                        idx.push_str(&format!("(gu_gop{s} == 8'd{j}) ? $signed(gv{j}_l{l}) : "));
                    }
                    idx.push('0');
                    e.l(format!("  wire signed [{0}:0] gu_gx{s}_l{l} = {idx}; reg signed [{0}:0] gu_gm{s}_l{l};", aw - 1));
                    e.l(format!("  always @(posedge clk) gu_gm{s}_l{l} <= gu_gst{s} * gu_gx{s}_l{l};"));
                    e.l(format!(
                        "  wire signed [{}:0] gu_off{s}_l{l} = gu_base{s} + $signed(gi{s}[{}:0]) + (gu_gon{s} ? gu_gm{s}_l{l} : 0);",
                        aw - 1,
                        aw - 1
                    ));
                    let el = if aw > 32 { format!("gu_off{s}_l{l}[31:0]") } else { format!("gu_off{s}_l{l}") };
                    e.l(format!("  wire [31:0] gu_el{s}_l{l} = {el};"));
                    e.l(format!(
                        "  wire gu_in{s}_l{l} = gu_gon{s} ? ((gu_off{s}_l{l} >= 0) && (gu_off{s}_l{l} < gu_lim{s})) : gu_bc{s} ? gu_in{s} : ((gu_off{s} + {l} >= 0) && (gu_off{s} + {l} < gu_lim{s}));"
                    ));
                } else {
                    e.l(format!("  wire gu_in{s}_l{l} = gu_bc{s} ? gu_in{s} : ((gu_off{s} + {l} >= 0) && (gu_off{s} + {l} < gu_lim{s}));"));
                }
            }
            let lane_in = |l: usize| if l == 0 { format!("gu_in{s}") } else { format!("gu_in{s}_l{l}") };
            let ins: Vec<String> = (0..vl).rev().map(lane_in).collect();
            e.l(format!("  wire [{}:0] gu_ins{s} = {{{}}};", vl - 1, ins.join(", ")));
            if lat == 0 {
                e.l(format!("  wire gl_v{s} = gi_v{s}; wire [{}:0] gl_in{s} = gu_ins{s};", vl - 1));
            } else {
                for d in 1..=lat {
                    let prev = if d == 1 { format!("gi_v{s}, gu_ins{s}") } else { format!("gl_v{s}_{0}, gl_in{s}_{0}", d - 1) };
                    e.l(format!("  reg gl_v{s}_{d}; reg [{}:0] gl_in{s}_{d}; always @(posedge clk) {{gl_v{s}_{d}, gl_in{s}_{d}}} <= {{{prev}}};", vl - 1));
                }
                e.l(format!("  wire gl_v{s} = gl_v{s}_{lat}; wire [{}:0] gl_in{s} = gl_in{s}_{lat};", vl - 1));
            }
            for l in 0..vl {
                e.l(format!("  always @(posedge clk) if (gl_v{s}) gv{s}_l{l} <= gl_in{s}[{l}] ? $signed(gu_ld{s}_l{l}) : 0;"));
                delay(&mut e, &format!("g_op{s}_l{l}"), wd.slot as usize, sh.hold_taps[s], &format!("gu_dl{s}"), &format!("gv{s}_l{l}"));
            }
        }
        for lanes in &lane_pipes {
            for (lines, _, _, _) in lanes {
                for l in lines {
                    e.l(l.clone());
                }
            }
        }
        e.l("  wire gu_pd = (gu_kx == 1) || (gb_x == gu_kx - 1);");
        e.l(format!("  wire [{axb}:0] gu_ia = gu_kind[0] ? gb_c : gb_r;"));
        // The control word of the point: its column to write, its row, the folded axis's index; whether it writes, folds
        // the outer axis, starts each fold, finishes and starts its contraction; whether it is a point at all — and in a
        // vector unit, above them, which of its lanes are.
        let cs = 7 + 3 * ax;
        let cw = if vec { cs + vl } else { cs };
        let lane_bits = if vec { format!("{}, ", (0..vl).rev().map(|l| format!("gb_lv{l}")).collect::<Vec<_>>().join(", ")) } else { String::new() };
        e.l(format!(
            "  wire [{}:0] gc0 = {{{lane_bits}(gu_kind == 2'd0 || gu_kind == 2'd2) ? gb_c : {ax}'d0, gb_r, gu_ia, gu_pd && ((gu_kind == 2'd0) || (gu_kind == 2'd1 && gb_c == gu_rinner_cx) || (gu_kind == 2'd2 && gb_r == gu_rx - 1) || (gu_kind == 2'd3 && gb_c == gu_rinner_cx && gb_r == gu_rx - 1)), gu_kind == 2'd3 && gu_pd && gb_c == gu_rinner_cx, gb_r == 0, gu_ia == 0, gu_pd, gu_kx == 1 || gb_x == 0, gb_v}};",
            cw - 1
        ));
        for d in 1..=dmax {
            e.l(format!("  reg [{}:0] gc{d}; always @(posedge clk) gc{d} <= gc{};", cw - 1, d - 1));
        }
        // The point whose value leaves the op's pipeline now: the control word as many cycles back as its class and its
        // lane tree are deep, and never one from before the op began.
        let taps: BTreeSet<u64> = sh
            .cls
            .op_class
            .iter()
            .filter_map(|(i, (c, _))| match &micro[*i] {
                Micro::Fixed(Fixed::Generic { unit, .. }) => Some(pipes[*c].3 + unit.tree_levels()),
                _ => None,
            })
            .collect();
        let mut tap = format!("  reg [{}:0] gco; always @* case (gu_dep)", cw - 1);
        for d in &taps {
            tap.push_str(&format!(" {d}: gco = gc{d};"));
        }
        tap.push_str(" default: gco = gc0; endcase");
        e.l(tap);
        if vec {
            // The control word of the point whose lanes leave their class's pipeline now, ahead of the lane tree: which
            // lanes are, and the folded axis's index their coordinates count from.
            let class_taps: BTreeSet<u64> = pipes.iter().map(|p| p.3).collect();
            let mut tap = format!("  reg [{}:0] gcd; always @* case (gu_cdep)", cw - 1);
            for d in &class_taps {
                tap.push_str(&format!(" {d}: gcd = gc{d};"));
            }
            tap.push_str(" default: gcd = gc0; endcase");
            e.l(tap);
            e.l(format!("  wire [{}:0] gq_lv = gco[{}:{cs}], gd_lv = gcd[{}:{cs}];", vl - 1, cw - 1, cw - 1));
            e.l(format!("  wire [{axb}:0] gd_ia = gcd[{}:7];", 6 + ax));
        }
        e.l("  wire gq_v = gu_on && gco[0] && (ph >= gu_dep);");
        e.l("  wire gq_kfirst = gco[1], gq_pd = gco[2], gq_ia0 = gco[3], gq_r0 = gco[4], gq_ofold = gco[5], gq_we = gco[6];");
        e.l(format!(
            "  wire [{axb}:0] gq_ia = gco[{}:7], gq_r = gco[{}:{}], gq_wcol = gco[{}:{}];",
            6 + ax,
            6 + 2 * ax,
            7 + ax,
            6 + 3 * ax,
            7 + 2 * ax
        ));
        // Under simulation: every point that leaves the pipeline is a point of the op it leaves in.
        e.l("  // synthesis translate_off");
        e.l("  always @(posedge clk) if (gq_v && (gq_r >= gu_rx || gq_wcol > gu_rinner_cx)) begin $display(\"FATAL: a point of row %0d, column %0d leaves the pipeline of an op of %0d rows and %0d columns\", gq_r, gq_wcol, gu_rx, gu_rinner_cx + 1); $finish; end");
        e.l("  // synthesis translate_on");
        let vw = wd.value - 1;
        if vec {
            // Each lane's value as it leaves its class's pipeline.
            for l in 0..vl {
                let mut v = format!("  wire signed [{vw}:0] gu_v_l{l} = ");
                for (c, lanes) in lane_pipes.iter().enumerate() {
                    if let Some((_, _, root, _)) = lanes.get(l) {
                        v.push_str(&format!("(gu_cls == 8'd{c}) ? $signed({root}) : "));
                    }
                }
                v.push_str("0;");
                e.l(v);
            }
            // ★ THE LANE TREE: a run's lanes folded pairwise, a registered level a pair, each node the fold of the lanes
            // below it — their sum, or the larger and on a tie the lower lane, so the first maximum — over the lanes that
            // are the op's and inside its extent, a node of none none. An op of `2^t` lanes takes level `t`'s first node.
            if sh.tree_levels > 0 {
                for j in 0..vl {
                    e.l(format!("  wire signed [{vw}:0] tv0_{j} = gu_v_l{j}; wire tval0_{j} = gd_lv[{j}]; wire [{axb}:0] tix0_{j} = (gd_ia << gu_lsh) | {ax}'d{j};"));
                }
                for lv in 1..=sh.tree_levels {
                    for j in 0..(vl >> lv) {
                        let (a, b) = (2 * j, 2 * j + 1);
                        let p = lv - 1;
                        e.l(format!(
                            "  wire tpick{lv}_{j} = !tval{p}_{a} || (tval{p}_{b} && (gu_tpool != 2'd0) && ($signed(tv{p}_{b}) > $signed(tv{p}_{a})));"
                        ));
                        e.l(format!("  reg signed [{vw}:0] tv{lv}_{j}; reg tval{lv}_{j}; reg [{axb}:0] tix{lv}_{j};"));
                        e.l(format!(
                            "  always @(posedge clk) begin tval{lv}_{j} <= tval{p}_{a} || tval{p}_{b}; tix{lv}_{j} <= tpick{lv}_{j} ? tix{p}_{b} : tix{p}_{a}; tv{lv}_{j} <= (gu_tpool == 2'd0 && tval{p}_{a} && tval{p}_{b}) ? $signed(tv{p}_{a}) + $signed(tv{p}_{b}) : tpick{lv}_{j} ? tv{p}_{b} : tv{p}_{a}; end"
                        ));
                    }
                }
                let mut v = format!("  reg signed [{vw}:0] gu_v; reg [{axb}:0] gu_tix; always @* case (gu_tl)");
                v.push_str(" 0: begin gu_v = gu_v_l0; gu_tix = gq_ia; end");
                for lv in 1..=sh.tree_levels {
                    v.push_str(&format!(" {lv}: begin gu_v = tv{lv}_0; gu_tix = tix{lv}_0; end"));
                }
                v.push_str(" default: begin gu_v = gu_v_l0; gu_tix = gq_ia; end endcase");
                e.l(v);
            } else {
                e.l(format!("  wire signed [{vw}:0] gu_v = gu_v_l0; wire [{axb}:0] gu_tix = gq_ia;"));
            }
        } else {
            let mut v = format!("  wire signed [{}:0] gu_v = ", wd.value - 1);
            for (c, (_, _, root, _)) in pipes.iter().enumerate() {
                v.push_str(&format!("(gu_cls == 8'd{c}) ? $signed({root}) : "));
            }
            v.push_str("0;");
            e.l(v);
        }
        // Stage A, the body's cycle: the contraction.
        e.l(format!("  wire signed [{vw}:0] gu_pv = gq_kfirst ? gu_v : ($signed(g_kacc) + gu_v);"));
        // Stage B: the fold of the innermost reduced axis, over the contraction a cycle on. A point's control rides
        // beside its value, and an op's last stages land within its own cycles, so a valid bit never crosses ops.
        e.l(format!("  reg fb_v, fb_pd, fb_ia0, fb_r0, fb_ofold, fb_we; reg [{axb}:0] fb_ia, fb_r, fb_wcol; reg signed [{vw}:0] fb_pv;"));
        let fb_ia = if vec { "gu_tix" } else { "gq_ia" };
        e.l(format!("  always @(posedge clk) begin fb_v <= gq_v && !op_last; fb_pd <= gq_pd; fb_ia0 <= gq_ia0; fb_r0 <= gq_r0; fb_ofold <= gq_ofold; fb_we <= gq_we; fb_ia <= {fb_ia}; fb_r <= gq_r; fb_wcol <= gq_wcol; fb_pv <= gu_pv; end"));
        for (pool, tag) in [(ReducePool::Sum, 0), (ReducePool::Max, 1), (ReducePool::ArgMax, 2)] {
            let (fv, fi) = fold_step(pool, "fb_ia0", "g_fold", "g_fidx", "fb_pv", "fb_ia");
            e.l(format!("  wire signed [{vw}:0] gu_fv{tag} = {fv}; wire [{axb}:0] gu_fi{tag} = {fi};"));
        }
        e.l(format!("  wire signed [{vw}:0] gu_fv = (gu_pool == 2'd2) ? gu_fv2 : (gu_pool == 2'd1) ? gu_fv1 : gu_fv0;"));
        e.l(format!("  wire [{axb}:0] gu_fi = (gu_pool == 2'd2) ? gu_fi2 : (gu_pool == 2'd1) ? gu_fi1 : gu_fi0;"));
        e.l(format!("  wire signed [{vw}:0] gu_ir = (gu_pool == 2'd2) ? $signed({{1'b0, gu_fi}}) : gu_fv;"));
        // Stage C: the outer fold of a full reduction, and the write.
        e.l(format!("  reg fc_v, fc_r0, fc_ofold, fc_we; reg [{axb}:0] fc_r, fc_wcol; reg signed [{vw}:0] fc_pv, fc_ir;"));
        e.l("  always @(posedge clk) begin fc_v <= fb_v && !op_last; fc_r0 <= fb_r0; fc_ofold <= fb_ofold; fc_we <= fb_we; fc_r <= fb_r; fc_wcol <= fb_wcol; fc_pv <= fb_pv; fc_ir <= gu_ir; end");
        for (pool, tag) in [(ReducePool::Sum, 0), (ReducePool::Max, 1), (ReducePool::ArgMax, 2)] {
            let (ov, oi) = fold_step(pool, "fc_r0", "g_ofold", "g_oidx", "fc_ir", "fc_r");
            e.l(format!("  wire signed [{vw}:0] gu_ov{tag} = {ov}; wire [{axb}:0] gu_oi{tag} = {oi};"));
        }
        e.l(format!("  wire signed [{vw}:0] gu_ov = (gu_rpool == 2'd2) ? gu_ov2 : (gu_rpool == 2'd1) ? gu_ov1 : gu_ov0;"));
        e.l(format!("  wire [{axb}:0] gu_oi = (gu_rpool == 2'd2) ? gu_oi2 : (gu_rpool == 2'd1) ? gu_oi1 : gu_oi0;"));
        e.l(format!("  wire signed [{vw}:0] gu_or = (gu_rpool == 2'd2) ? $signed({{1'b0, gu_oi}}) : gu_ov;"));
        if vec {
            // Each further lane of an op whose lanes each keep their own column: its contraction, its fold of the axis
            // walked inside, and its write, beside lane 0's, on lane 0's control.
            // Only what the engine's ops of that kind use: a contraction where one contracts, the folds of their pools.
            e.l(format!("  reg [{}:0] fb_lv, fc_lv; always @(posedge clk) begin fb_lv <= gq_lv; fc_lv <= fb_lv; end", vl - 1));
            let pools: Vec<(ReducePool, u8)> =
                [(ReducePool::Sum, 0u8), (ReducePool::Max, 1), (ReducePool::ArgMax, 2)].into_iter().filter(|(p, _)| sh.lane_pools[pool_index(*p)]).collect();
            for l in 1..vl {
                e.l(format!("  reg signed [{vw}:0] fb_pv_l{l}, fc_pv_l{l};"));
                if sh.lane_contracts {
                    e.l(format!("  reg signed [{vw}:0] g_kacc_l{l};"));
                    e.l(format!("  wire signed [{vw}:0] gu_pv_l{l} = gq_kfirst ? gu_v_l{l} : ($signed(g_kacc_l{l}) + gu_v_l{l});"));
                } else {
                    e.l(format!("  wire signed [{vw}:0] gu_pv_l{l} = gu_v_l{l};"));
                }
                e.l(format!("  always @(posedge clk) begin fb_pv_l{l} <= gu_pv_l{l}; fc_pv_l{l} <= fb_pv_l{l}; end"));
                if pools.is_empty() {
                    continue;
                }
                e.l(format!("  reg signed [{vw}:0] g_fold_l{l}, fc_ir_l{l}; reg [{axb}:0] g_fidx_l{l};"));
                for (pool, tag) in &pools {
                    let (fv, fi) = fold_step(*pool, "fb_ia0", &format!("g_fold_l{l}"), &format!("g_fidx_l{l}"), &format!("fb_pv_l{l}"), "fb_ia");
                    e.l(format!("  wire signed [{vw}:0] gu_fv{tag}_l{l} = {fv}; wire [{axb}:0] gu_fi{tag}_l{l} = {fi};"));
                }
                let pick = |what: &str| -> String {
                    let (last, rest) = pools.split_last().expect("a pool");
                    let mut x = String::new();
                    for (_, tag) in rest {
                        x.push_str(&format!("(gu_pool == 2'd{tag}) ? {what}{tag}_l{l} : "));
                    }
                    format!("{x}{what}{}_l{l}", last.1)
                };
                e.l(format!("  wire signed [{vw}:0] gu_fv_l{l} = {};", pick("gu_fv")));
                e.l(format!("  wire [{axb}:0] gu_fi_l{l} = {};", pick("gu_fi")));
                let ir = if sh.lane_pools[pool_index(ReducePool::ArgMax)] {
                    format!("(gu_pool == 2'd2) ? $signed({{1'b0, gu_fi_l{l}}}) : gu_fv_l{l}")
                } else {
                    format!("gu_fv_l{l}")
                };
                e.l(format!("  wire signed [{vw}:0] gu_ir_l{l} = {ir};"));
                e.l(format!("  always @(posedge clk) fc_ir_l{l} <= gu_ir_l{l};"));
            }
        }
    }
    // Scalar grid.
    let needs_grid = micro.iter().any(|m| matches!(m, Micro::Fixed(Fixed::Unary { kind: UnaryKind::Scalar(_), .. })));
    if needs_grid {
        let gname = grid_names.get(&plan.class).cloned().unwrap_or_else(|| format!("g{}", plan.class));
        e.l("  wire [63:0] grid_result;");
        e.l(format!("  grid_{gname} u_grid(.in0(unary_in), .result(grid_result));"));
    } else {
        e.l("  wire grid_result = 1'b0;");
    }

    // ── Comb: per-state port drives and live values ──────────────────────────────────────────────────────────
    let mut comb = Emit { s: String::new() };
    comb.l("  always @* begin");
    for b in &bufs {
        let k = tp(*b);
        if uses.produces.contains(b) {
            comb.l(format!("    f{k}_reserve = 1'b0; f{k}_reserve_n = 0; f{k}_push = 1'b0; f{k}_push_n = 0;"));
        }
        if uses.consumes.contains(b) {
            comb.l(format!("    f{k}_pop = 1'b0; f{k}_pop_n = 0;"));
        }
        if uses.writes.contains(b) {
            comb.l(format!("    f{k}_wr_en = 1'b0; f{k}_wr_slot = 0; f{k}_wr_elem = 0; f{k}_wr_data = 64'd0;"));
            if ctx.banks > 1 {
                comb.l(format!("    f{k}_wr_n = 1; f{k}_wr_lw = 0; f{k}_wr_hi = 0;"));
            }
        }
        if uses.reads.contains(b) {
            comb.l(format!("    f{k}_rd_slot = 0; f{k}_rd_elem = 0;"));
            for l in 1..gathered.get(&ctx.template(*b)).copied().unwrap_or(1) {
                comb.l(format!("    f{k}_l{l}_rd_slot = 0; f{k}_l{l}_rd_elem = 0;"));
            }
        }
    }
    for t in &t_rd {
        comb.l(format!("    t{}_rd_addr = 32'd0;", t.idx()));
    }
    for t in &t_wr {
        comb.l(format!("    t{0}_wr_en = 1'b0; t{0}_wr_addr = 32'd0; t{0}_wr_data = 64'd0;", t.idx()));
        if ctx.banks > 1 {
            comb.l(format!("    t{0}_wr_n = 1; t{0}_wr_lw = 0; t{0}_wr_hi = 0;", t.idx()));
        }
    }
    for s in &s_local {
        comb.l(format!("    s{0}_set = 1'b0; s{0}_set_val = 16'd0;", s.idx()));
    }
    for s in &s_inc {
        comb.l(format!("    s{0}_inc_remote = 1'b0; s{0}_inc_n = 16'd0;", s.idx()));
    }
    for (s, gr) in &m_valid {
        comb.l(format!("    mv{}_g{} = 1'b0;", s.idx(), gr.idx()));
    }
    for gr in &m_send {
        comb.l(format!("    mc{0}_send = 1'b0; mc{0}_din = 80'd0;", gr.idx()));
    }
    {
        let mut d = String::from("   ");
        for (n, _, _) in &live {
            d.push_str(&format!(" {n} = 0;"));
        }
        comb.l(d);
    }
    comb.l("    if (running) case (pc)");

    // ── The sequencer: the next state, then its register ────────────────────────────────────────────────────────
    let mut seq = Emit { s: String::new() };
    seq.l("  always @* begin");
    {
        let mut hold = String::from("    pc_n = pc; ph_n = ph; running_n = running; done_n = done;");
        for v in &lvs {
            hold.push_str(&format!(" {0}_n = {0};", var_name(*v)));
        }
        seq.l(hold);
    }
    seq.l("    if (!running && start && !done) begin running_n = 1'b1; pc_n = 0; ph_n = 0; end");
    seq.l("    else if (running) case (pc)");
    // Each running state's successor, and each fixed op's row of the op table and load of the cycle counter, which the
    // two print as the rows of the successors the state can take.
    let mut nexts: Vec<(usize, Next)> = Vec::new();
    let mut rows: BTreeMap<usize, String> = BTreeMap::new();
    let mut counts: BTreeMap<usize, String> = BTreeMap::new();

    let mut bound: BTreeSet<LoopVar> = BTreeSet::new();
    for (i, m) in micro.iter().enumerate() {
        let next = i + 1;
        match m {
            Micro::LoopStart { var, count, end } => {
                bound.insert(*var);
                let c = if *var == LV_ITEM { "ITEM_COUNT".to_string() } else { count.to_string() };
                let to = Next::when(format!("({c} == 0)"), Next::To(end + 1), Next::To(next));
                seq.l(format!("      {i}: begin {}_n = 0; pc_n = {}; end", var_name(*var), to.expr()));
                nexts.push((i, to));
            }
            Micro::LoopEnd { var, count, start } => {
                bound.remove(var);
                // A loop's end is reached with its counter below its count, so it runs again while the counter is not the
                // last: an equality with a constant, no carry.
                let last = if *var == LV_ITEM { "(ITEM_COUNT - 1)".to_string() } else { count.saturating_sub(1).to_string() };
                let v = var_name(*var);
                let to = Next::when(format!("({v} != {last})"), Next::To(start + 1), Next::To(next));
                seq.l(format!("      {i}: begin {v}_n = {v} + 1; pc_n = {}; end", to.expr()));
                nexts.push((i, to));
            }
            // A folded effect is printed with the op it is folded into; it has no state.
            Micro::Then(_) => {}
            Micro::Guarded(gd) => {
                let cond = guard_cond(ctx, gd);
                if let Guard::Reserve { buf, pages } = gd {
                    let k = ctx.template(*buf).idx();
                    comb.l(format!("      {i}: begin f{k}_reserve = {cond}; f{k}_reserve_n = {pages}; end"));
                }
                let to = Next::when(cond, Next::To(next), Next::To(i));
                seq.l(format!("      {i}: pc_n = {};", to.expr()));
                nexts.push((i, to));
            }
            Micro::Fixed(f) => {
                let d = ctx.duration(f);
                // Through the loop bound its last cycle takes itself, then past the guards of the branch it checks early.
                let exit = super::exit_of(micro, i, plan.site);
                let check = |e: &super::Early| -> String {
                    use super::Early;
                    match *e {
                        Early::Count(_, n) | Early::Free(_, n) | Early::AtLeast(_, n) if n <= 0 => "1'b1".into(),
                        Early::Count(b, n) => format!("(f{}_count >= {n})", ctx.template(b).idx()),
                        Early::Free(b, n) => format!("(f{}_free >= {n})", ctx.template(b).idx()),
                        Early::AtLeast(sem, n) => format!("(s{}_count >= {n})", sem.idx()),
                        Early::Equals(sem, n) => format!("(s{}_count == {n})", sem.idx()),
                    }
                };
                let branch = |(at, checks): &(usize, Vec<(usize, super::Early)>)| -> (Next, Option<String>) {
                    match checks.last() {
                        Some((last, _)) => {
                            let all = checks.iter().map(|(_, e)| check(e)).collect::<Vec<_>>().join(" && ");
                            (Next::when(format!("({all})"), Next::To(last + 1), Next::To(*at)), Some(all))
                        }
                        None => (Next::To(*at), None),
                    }
                };
                let count = |var: LoopVar, count: usize| if var == LV_ITEM { "ITEM_COUNT".to_string() } else { count.to_string() };
                let final_count = |var: LoopVar, count: usize| if var == LV_ITEM { "(ITEM_COUNT - 1)".to_string() } else { count.saturating_sub(1).to_string() };
                let (to, steps, taken): (Next, String, Vec<String>) = match exit.bound {
                    None => (branch(&exit.branches[0]).0, String::new(), vec!["1'b1".into()]),
                    Some(super::Bound::End { var, count: c, .. }) => {
                        let v = var_name(var);
                        let runs = format!("({v} != {})", final_count(var, c));
                        (
                            Next::when(runs.clone(), branch(&exit.branches[0]).0, branch(&exit.branches[1]).0),
                            format!(" {v}_n = {v} + 1;"),
                            vec![runs.clone(), format!("!{runs}")],
                        )
                    }
                    Some(super::Bound::Start { var, count: c, .. }) => {
                        let empty = format!("({} == 0)", count(var, c));
                        (
                            Next::when(empty.clone(), branch(&exit.branches[1]).0, branch(&exit.branches[0]).0),
                            format!(" {}_n = 0;", var_name(var)),
                            vec![format!("!{empty}"), empty.clone()],
                        )
                    }
                };
                seq.l(format!("      {i}: if (op_last) begin ph_n = 0;{steps} pc_n = {}; end else ph_n = ph + 1;", to.expr()));
                counts.insert(i, format!(" op_left <= {}; op_last <= {};", d.max(1) - 1, if d <= 1 { "1'b1" } else { "1'b0" }));
                let mut c = format!("      {i}: begin");
                // A branch whose guards all hold takes their reservations in the op's last cycle.
                for (b, taken) in exit.branches.iter().zip(&taken) {
                    let Some(all) = branch(b).1 else { continue };
                    let mut takes = String::new();
                    for (g, _) in &b.1 {
                        if let Micro::Guarded(Guard::Reserve { buf, pages }) = &micro[*g] {
                            let k = ctx.template(*buf).idx();
                            takes.push_str(&format!(" f{k}_reserve = 1'b1; f{k}_reserve_n = {pages};"));
                        }
                    }
                    if !takes.is_empty() {
                        c.push_str(&format!(" if (op_last && {taken} && {all}) begin{takes} end"));
                    }
                }
                let after = super::after_effects(micro, i);
                let mut entries: Vec<(String, String)> = vec![("fx".into(), "1'b1".into())];
                print_op(ctx, &sh, &wd, &regions, i, f, &bound, &mut entries, &mut c)?;
                // The effects folded into the op commit in its last cycle.
                for (j, m) in micro.iter().enumerate().take(after).skip(i + 1) {
                    if let Micro::Then(e) = m {
                        let mut drives = String::new();
                        let mut none = Vec::new();
                        print_op(ctx, &sh, &wd, &regions, j, e, &bound, &mut none, &mut drives)?;
                        debug_assert!(none.is_empty(), "an effect sets no unit parameter");
                        if !drives.is_empty() {
                            c.push_str(&format!(" if (op_last) begin{drives} end"));
                        }
                    }
                }
                c.push_str(" end");
                comb.l(c);
                rows.insert(i, entries.iter().map(|(n, v)| format!(" {n} <= {v};")).collect());
                nexts.push((i, Next::when("(op_last)".into(), to, Next::To(i))));
            }
        }
    }
    let fin = micro.len();
    seq.l(format!("      {fin}: begin running_n = 1'b0; done_n = 1'b1; end"));
    seq.l("      default: pc_n = 0;");
    seq.l("    endcase");
    seq.l("  end");
    {
        let mut reset = String::from("  always @(posedge clk) if (rst) begin pc <= 0; ph <= 0; running <= 1'b0; done <= 1'b0;");
        let mut r = String::from(" end else begin pc <= pc_n; ph <= ph_n; running <= running_n; done <= done_n;");
        for v in &lvs {
            reset.push_str(&format!(" {} <= 0;", var_name(*v)));
            r.push_str(&format!(" {0} <= {0}_n;", var_name(*v)));
        }
        r.push_str(" end");
        seq.l(format!("{reset}{r}"));
    }
    comb.l("      default: ;");
    comb.l("    endcase");
    comb.l("  end");

    // ── The op table, loaded on the clock that enters a state ───────────────────────────────────────────────────
    // Each running state loads the row of the successor it takes, chosen by the conditions its next state is, so the
    // decode in front of the table is of the registered state and the conditions select among a few constant rows. The
    // final state loads none, as the run ends there; a state the sequencer never holds loads the first's, as it restarts.
    let row = |s: usize| rows.get(&s).cloned().unwrap_or_default();
    let mut tab = Emit { s: String::new() };
    tab.l("  always @(posedge clk) begin");
    {
        let mut d = String::from("   ");
        for (n, _, _) in &table {
            d.push_str(&format!(" {n} <= 0;"));
        }
        tab.l(d);
    }
    tab.l(format!("    if (rst) ; else if (!running) begin if (start && !done) begin{} end end", row(0)));
    tab.l("    else case (pc)");
    for (i, nx) in &nexts {
        tab.l(format!("      {i}: begin{} end", nx.load(&row)));
    }
    tab.l(format!("      {fin}: ;"));
    tab.l(format!("      default: begin{} end", row(0)));
    tab.l("    endcase");
    tab.l("  end");
    // A state's first cycle — after a fixed op's last, in any other state, or the run's first — loads the cycle counter
    // for the state entered, as the table does. An engine that is not running holds no count.
    let zero = " op_left <= 0; op_last <= 1'b0;".to_string();
    let count = |s: usize| counts.get(&s).cloned().unwrap_or_else(|| zero.clone());
    let mut last = Emit { s: String::new() };
    last.l("  wire fresh = !running || !fx || op_last;");
    last.l("  always @(posedge clk)");
    last.l("    if (rst) begin op_left <= 0; op_last <= 1'b0; end");
    last.l(format!("    else if (!running) begin if (start && !done) begin{} end else begin{zero} end end", count(0)));
    last.l("    else if (fresh) case (pc)");
    for (i, nx) in &nexts {
        last.l(format!("      {i}: begin{} end", nx.load(&count)));
    }
    last.l(format!("      {fin}: begin{zero} end"));
    last.l(format!("      default: begin{} end", count(0)));
    last.l("    endcase");
    last.l("    else begin op_left <= op_left - 1; op_last <= (op_left == 1); end");

    let units = units(ctx, &sh, &wd, te, tr, tc, lat);
    // The flags of the phase constants the blocks above compare with: `ph_ge{k}` holds `ph >= k` in every cycle. The
    // phase's next value is only ever 0 (a start, a fixed op's last cycle), `ph + 1` (a fixed op counting) or `ph`, so
    // the flags follow it as a shift chosen by registers: clear, each from the one below (`ph + 1 >= k` is `ph >= k - 1`),
    // or hold. They run dense from 1 so every flag's predecessor is a flag, and no decode of the next state or comparator
    // stands in front of them.
    let top = ctx.phase_flags.borrow().iter().copied().max().unwrap_or(0);
    let ks: Vec<u64> = (1..=top).collect();
    if !ks.is_empty() {
        let flags: Vec<String> = ks.iter().map(|k| format!("ph_ge{k}")).collect();
        e.l(format!("  reg {};", flags.join(", ")));
    }
    e.s.push_str(&comb.s);
    e.s.push_str(&tab.s);
    e.s.push_str(&last.s);
    e.s.push_str(&seq.s);
    if !ks.is_empty() {
        let clear: String = ks.iter().map(|k| format!(" ph_ge{k} <= 1'b0;")).collect();
        let shift: String = ks.iter().map(|k| if *k == 1 { " ph_ge1 <= 1'b1;".to_string() } else { format!(" ph_ge{k} <= ph_ge{};", k - 1) }).collect();
        e.l(format!(
            "  always @(posedge clk) if (rst || (running && fx && op_last) || (!running && start && !done)) begin{clear} end else if (running && fx) begin{shift} end"
        ));
    }
    // Under simulation, in every cycle out of reset: each flag is the comparison it stands for, and an engine that is not
    // running holds no row of its table. Neither shows in any output (a flag is read only while the engine runs, and a
    // parameter only where a state drives its unit), so only these checks see them broken.
    e.l("  // synthesis translate_off");
    for k in &ks {
        e.l(format!("  always @(posedge clk) if (!rst && ph_ge{k} != (ph >= {k})) begin $display(\"FATAL: ph_ge{k} is %0d at phase %0d\", ph_ge{k}, ph); $finish; end"));
    }
    let held: Vec<&str> = table.iter().map(|(n, _, _)| n.as_str()).collect();
    if !held.is_empty() {
        e.l(format!(
            "  always @(posedge clk) if (!rst && !running && (|{{{}}})) begin $display(\"FATAL: an idle engine holds a row of its op table\"); $finish; end",
            held.join(", ")
        ));
    }
    e.l("  // synthesis translate_on");
    e.s.push_str(&units);
    e.l("endmodule");
    // The address width: the largest tensor's elements, every literal the rows load into a walk parameter, and the
    // longest op's phase, with a bit of headroom so a sum of two of them never wraps and a difference below zero reads
    // above every one.
    let at_aw: BTreeSet<&str> = table.iter().filter(|(_, w, _)| *w == AW_WIDE).map(|(n, _, _)| n.as_str()).collect();
    let mut bound = p.tensors.iter().map(|t| (t.elems.rows * t.elems.cols) as u64).max().unwrap_or(1).max(longest + 2);
    for row in rows.values() {
        for stmt in row.split(';') {
            if let Some((n, v)) = stmt.split_once("<=") {
                if let (true, Ok(v)) = (at_aw.contains(n.trim()), v.trim().parse::<i64>()) {
                    bound = bound.max(v.unsigned_abs());
                }
            }
        }
    }
    let aw = 64 - bound.leading_zeros() + 1;
    Ok(e.s.replacen("__AW__", &aw.to_string(), 1))
}

/// Where a running state goes on the next clock: a state, or the choice a condition on registered values makes between
/// two. The sequencer prints it as `pc_n`; the op table and the cycle counter print it as the rows of the states it can
/// reach, chosen by the same conditions, so each loads the state entered without decoding `pc_n`.
enum Next {
    To(usize),
    If(String, Box<Next>, Box<Next>),
}

impl Next {
    fn when(cond: String, then: Next, other: Next) -> Next {
        Next::If(cond, Box::new(then), Box::new(other))
    }

    /// The state taken, as the expression `pc_n` is set to.
    fn expr(&self) -> String {
        match self {
            Next::To(s) => s.to_string(),
            Next::If(c, a, b) => format!("({c} ? {} : {})", a.expr(), b.expr()),
        }
    }

    /// Statements loading, for the state taken, what `row` prints for that state.
    fn load(&self, row: &dyn Fn(usize) -> String) -> String {
        match self {
            Next::To(s) => row(*s),
            Next::If(c, a, b) => format!(" if {c} begin{} end else begin{} end", a.load(row), b.load(row)),
        }
    }
}

/// A guard's condition on its FIFO's or semaphore's registered state.
fn guard_cond(ctx: &Ctx, g: &Guard) -> String {
    match g {
        Guard::Reserve { buf, pages } => format!("(f{}_free >= {pages})", ctx.template(*buf).idx()),
        Guard::Wait { buf, pages } => format!("(f{}_count >= {pages})", ctx.template(*buf).idx()),
        Guard::SemWait { sem, until } => match until {
            SemCond::Equals(n) => format!("(s{}_count == {n})", sem.idx()),
            SemCond::AtLeast(n) => format!("(s{}_count >= {n})", sem.idx()),
        },
        Guard::Link { ordinal, .. } => format!("(link_n > 8'd{ordinal})"),
    }
}

/// One fixed op's state: the parameters of the unit it runs on, into `table` as `(name, value)`, and the ports and live
/// values it drives as the phase moves, into `c`.
#[allow(clippy::too_many_arguments)]
fn print_op(
    ctx: &Ctx,
    sh: &Shape,
    wd: &Widths,
    regions: &[Option<usize>],
    i: usize,
    f: &Fixed,
    bound: &BTreeSet<LoopVar>,
    table: &mut Vec<(String, String)>,
    c: &mut String,
) -> Result<(), SpatialLowerError> {
    let (te, tc, lat) = (ctx.te, ctx.tc, ctx.lat);
    let cols = regions[i].unwrap_or(1);
    let mut set = |n: &str, v: String| table.push((n.to_string(), v));
    match f {
        Fixed::Push { buf, pages } => {
            let k = ctx.template(*buf).idx();
            c.push_str(&format!(" f{k}_push = 1'b1; f{k}_push_n = {pages};"));
        }
        Fixed::Pop { buf, pages } => {
            let k = ctx.template(*buf).idx();
            c.push_str(&format!(" f{k}_pop = 1'b1; f{k}_pop_n = {pages};"));
        }
        Fixed::Barrier | Fixed::ReleaseAcc => {}
        Fixed::SemSet { sem, value } => {
            let v = match value {
                SemValue::Invalid => 0,
                SemValue::Valid => 1,
                SemValue::Count(n) => *n,
            };
            c.push_str(&format!(" s{0}_set = 1'b1; s{0}_set_val = {v};", sem.idx()));
        }
        Fixed::SemInc { sem, by, .. } => {
            c.push_str(&format!(" s{0}_inc_remote = 1'b1; s{0}_inc_n = {by};", sem.idx()));
        }
        Fixed::McastSetValid { sem, group } => {
            c.push_str(&format!(" mv{}_g{} = 1'b1;", sem.idx(), group.idx()));
        }
        Fixed::McastSend { buf, pages, group } => {
            let k = ctx.template(*buf).idx();
            let pe = ctx.page_elems(*buf);
            let total = pages * pe;
            let depth = ctx.depth(*buf);
            let gr = group.idx();
            set("mc_on", "1'b1".into());
            set("mc_pe", pe.to_string());
            // The page this element is in, after the front: its slot wraps once at most.
            let ph_lt_total = ctx.ph_lt(total as u64);
            c.push_str(&format!(
                " if ({ph_lt_total}) begin f{k}_rd_slot = (f{k}_fill_slot0_r + mc_pg >= {depth}) ? f{k}_fill_slot0_r + mc_pg - {depth} : f{k}_fill_slot0_r + mc_pg; f{k}_rd_elem = mc_e; end"
            ));
            let elem = if lat == 0 { "ph[15:0]".to_string() } else { format!("ph[15:0] - 16'd{lat}") };
            let live = format!("{} && {}", ctx.ph_ge(lat), ctx.ph_lt(total as u64 + lat));
            c.push_str(&format!(" if ({live}) begin mc{gr}_send = 1'b1; mc{gr}_din = {{{elem}, f{k}_rd_data}}; end"));
        }
        Fixed::ReadTiles { tensor, at, block_w, stride_w, stride_h, into, section, .. } => {
            let k = ctx.template(*into).idx();
            let addr = addr_v(at, bound, item_coord_names(*section).as_tuple(), ctx);
            let gather = index_read(ctx, at);
            // Element `e`'s address goes out at phase `skew + e` — a gather's waits for its index — and its data lands in
            // the page `lat` phases later.
            let skew = ctx.gather_skew(at);
            let (d1, d2) = walk_deltas(te, *block_w, *stride_w, *stride_h);
            let t = tensor.idx();
            let cp = ph_less(skew + lat);
            let land = from_phase(ctx, 
                skew + lat,
                &format!("f{k}_wr_en = 1'b1; f{k}_wr_slot = f{k}_fill_slot0; f{k}_wr_elem = {cp}; f{k}_wr_data = t{t}_rd_data;"),
            );
            set("tw_on", "1'b1".into());
            set("tw_from", skew.to_string());
            set("tw_bw", block_w.to_string());
            if let Some((_, pitch)) = ctx.page_pitch.get(&ctx.template(*into)) {
                // The page's rows lie `pitch` apart: the walk steps over each row's padding as it ends the row.
                set("tw_pp", (pitch - block_w).to_string());
            }
            if ctx.walks_runs(f) {
                // A run of the row a cycle, from `skew` on; it lands in the page `lat` later, where the walk had it.
                let (wt, wf) = (tensor_width(ctx.p.tensor(*tensor), &ctx.ranges), ctx.buffer_width(*into));
                let hi = resize_lanes(&format!("t{t}_rd_hi"), wt, wf, ctx.banks - 1);
                let land = from_phase(ctx, 
                    skew + lat,
                    &format!("f{k}_wr_en = 1'b1; f{k}_wr_slot = f{k}_fill_slot0; f{k}_wr_elem = tw_pd; f{k}_wr_n = tw_nd; f{k}_wr_lw = tw_lw; f{k}_wr_data = t{t}_rd_data; f{k}_wr_hi = {hi};"),
                );
                set("tw_vec", "1'b1".into());
                set("tw_lw", ctx.run(f).trailing_zeros().to_string());
                set("tw_n0", (*block_w).min(ctx.run(f)).to_string());
                set("tw_rs", stride_h.to_string());
                c.push_str(&format!("{gather} t{t}_rd_addr = ({addr}) + tw_vo; {land}"));
            } else {
                set("tw_d1", d1.to_string());
                set("tw_d2", d2.to_string());
                c.push_str(&format!("{gather} t{t}_rd_addr = ({addr}) * {te} + tw_off; {land}"));
            }
        }
        Fixed::WriteTiles { from, tensor, at, block_w, stride_w, stride_h, section, .. } => {
            let k = ctx.template(*from).idx();
            if at.indirect.is_some_and(|ind| ctx.template(ind.index) == ctx.template(*from)) {
                return Err(SpatialLowerError::Unsupported(
                    "a tile scatter whose index sits in the buffer it drains — one read port cannot read both in a cycle",
                ));
            }
            let addr = addr_v(at, bound, item_coord_names(*section).as_tuple(), ctx);
            let gather = index_read(ctx, at);
            // Element `e` is read at phase `e` and stored `lat` phases later, when its data — and a scatter's index,
            // read at every phase — have landed.
            let (d1, d2) = walk_deltas(te, *block_w, *stride_w, *stride_h);
            let t = tensor.idx();
            set("tw_on", "1'b1".into());
            set("tw_bw", block_w.to_string());
            if ctx.run(f) > 1 {
                // A run of the page a cycle from phase 0, stored `lat` later where the walk had it in the tensor.
                let (wt, wf) = (tensor_width(ctx.p.tensor(*tensor), &ctx.ranges), ctx.buffer_width(*from));
                let hi = resize_lanes(&format!("f{k}_rd_hi"), wf, wt, ctx.banks - 1);
                let store = from_phase(ctx, 
                    lat,
                    &format!("t{t}_wr_en = 1'b1; t{t}_wr_addr = ({addr}) + tw_vod; t{t}_wr_n = tw_nd; t{t}_wr_lw = tw_lw; t{t}_wr_data = f{k}_rd_data; t{t}_wr_hi = {hi};"),
                );
                set("tw_from", "0".into());
                set("tw_vec", "1'b1".into());
                set("tw_lw", ctx.run(f).trailing_zeros().to_string());
                set("tw_n0", (*block_w).min(ctx.run(f)).to_string());
                set("tw_rs", stride_h.to_string());
                c.push_str(&format!("{gather} f{k}_rd_slot = f{k}_rd_slot0; f{k}_rd_elem = tw_p; {store}"));
            } else {
                let store = from_phase(ctx, lat, &format!("t{t}_wr_en = 1'b1; t{t}_wr_addr = ({addr}) * {te} + tw_off; t{t}_wr_data = f{k}_rd_data;"));
                set("tw_from", lat.to_string());
                set("tw_d1", d1.to_string());
                set("tw_d2", d2.to_string());
                c.push_str(&format!("{gather} f{k}_rd_slot = f{k}_rd_slot0; f{k}_rd_elem = ph; {store}"));
            }
        }
        Fixed::AcquireAcc { tiles, .. } => {
            set("aw_cols", cols.to_string());
            if ctx.acc_run(f) > 1 {
                // A run of the row's banks zeroed a cycle.
                set("ar_on", "1'b1".into());
                set("ar_lw", ctx.acc_run(f).trailing_zeros().to_string());
                set("ar_e", (tiles * te).to_string());
                set("ar_n0", (tiles * te).min(cols).min(ctx.acc_run(f)).to_string());
                c.push_str(" az_we = 1'b1;");
            } else {
                c.push_str(" aw_step = 1'b1; sw_we = 1'b1; sw_d = 0;");
            }
        }
        Fixed::PackAcc { to, tiles, trunc32, accumulate, .. } => {
            let k = ctx.template(*to).idx();
            // Accumulating into the page reads each element before it writes it back, `lat` phases later.
            let skew = if *accumulate { lat } else { 0 };
            set("aw_cols", cols.to_string());
            if ctx.acc_run(f) > 1 {
                // A run of the row's banks a cycle, into the page where the walk had it.
                set("ar_on", "1'b1".into());
                set("ar_lw", ctx.acc_run(f).trailing_zeros().to_string());
                set("ar_e", (tiles * te).to_string());
                set("ar_n0", (tiles * te).min(cols).min(ctx.acc_run(f)).to_string());
                set("ar_dly", (*accumulate as u8).to_string());
                set("pk_tr", (*trunc32 as u8).to_string());
                if *accumulate {
                    c.push_str(&format!(" f{k}_rd_slot = f{k}_fill_slot0; f{k}_rd_elem = ar_p;"));
                }
                let lanes: Vec<String> = (1..ctx.banks).rev().map(|l| format!("pk{k}_v{l}")).collect();
                let store = from_phase(ctx, 
                    skew,
                    &format!(
                        "f{k}_wr_en = 1'b1; f{k}_wr_slot = f{k}_fill_slot0; f{k}_wr_elem = ar_wp; f{k}_wr_n = ar_wn; f{k}_wr_lw = ar_lw; f{k}_wr_data = $signed(pk{k}_v0); f{k}_wr_hi = {{{}}};",
                        lanes.join(", ")
                    ),
                );
                c.push_str(&format!(" {store}"));
            } else {
                let cp = ph_less(skew);
                // An accumulator past 32 bits keeps the low 32 of an I32 plan; one within them holds its value whole.
                let v = if *trunc32 && wd.acc > 32 { "{{32{sr_d[31]}}, sr_d[31:0]}".to_string() } else { "sr_d".to_string() };
                let base = if *accumulate { format!("$signed(f{k}_rd_data) + ") } else { String::new() };
                if *accumulate {
                    c.push_str(&format!(" f{k}_rd_slot = f{k}_fill_slot0; f{k}_rd_elem = ph;"));
                }
                let store = from_phase(ctx, 
                    skew,
                    &format!("aw_step = 1'b1; f{k}_wr_en = 1'b1; f{k}_wr_slot = f{k}_fill_slot0; f{k}_wr_elem = {cp}; f{k}_wr_data = {base}$signed({v});"),
                );
                c.push_str(&format!(" {store}"));
            }
        }
        Fixed::Unary { kind, ins, .. } => {
            let k = ctx.template(ins[0]).idx();
            let pe = ctx.page_elems(ins[0]);
            set("un_on", "1'b1".into());
            set("un_pe", pe.to_string());
            c.push_str(&format!(" f{k}_rd_slot = f{k}_rd_slot0; f{k}_rd_elem = un_e; unary_in = f{k}_rd_data;"));
            let kind_code = match kind {
                UnaryKind::Requant { m, s } => {
                    set("un_m", lit64(*m));
                    set("un_s", s.to_string());
                    0
                }
                UnaryKind::Bias => {
                    let kb = ctx.template(ins[1]).idx();
                    set("un_pb", ctx.page_elems(ins[1]).to_string());
                    c.push_str(&format!(
                        " f{kb}_rd_slot = f{kb}_rd_slot0; f{kb}_rd_elem = un_bt + (un_bk & {}); unary_bias = f{kb}_rd_data;",
                        tc - 1
                    ));
                    1
                }
                UnaryKind::Relu => 2,
                UnaryKind::Elementwise(op) => {
                    let kb = ctx.template(ins[1]).idx();
                    set("un_pb", ctx.page_elems(ins[1]).to_string());
                    c.push_str(&format!(" f{kb}_rd_slot = f{kb}_rd_slot0; f{kb}_rd_elem = un_e2; unary_bias = f{kb}_rd_data;"));
                    match op {
                        ElementwiseOp::Add => 4,
                        ElementwiseOp::Mul => 5,
                    }
                }
                UnaryKind::Scalar(_) => 6,
            };
            let store = from_phase(ctx, lat, "aw_step = 1'b1; sw_we = 1'b1;");
            set("un_kind", kind_code.to_string());
            set("aw_cols", cols.to_string());
            c.push_str(&format!(" sw_d = unary_out; {store}"));
        }
        Fixed::Generic { unit, ins, depth, .. } => {
            let (class, values) = &sh.cls.op_class[&i];
            let is = &unit.issue;
            // The walk counts points: along the lane axis, runs of the lanes.
            let (rx, cx, kx) = (unit.points_along(unit.row.var), unit.points_along(unit.col.var), unit.k.map_or(1, |k| unit.points_along(k.var)));
            let (nmid, nout) = if unit.rows_inner() { (rx, cx) } else { (cx, rx) };
            use IterKind::{Parallel, Reduce};
            let (kind, pool, rpool) = match (unit.row.kind, unit.col.kind) {
                (Parallel, Parallel) => (0, ReducePool::Sum, ReducePool::Sum),
                (Parallel, Reduce { pool }) => (1, pool, ReducePool::Sum),
                (Reduce { pool }, Parallel) => (2, pool, ReducePool::Sum),
                (Reduce { pool: rp }, Reduce { pool }) => (3, pool, rp),
            };
            let code = |p: ReducePool| match p {
                ReducePool::Sum => 0,
                ReducePool::Max => 1,
                ReducePool::ArgMax => 2,
            };
            for (n, v) in [
                ("gu_on", "1'b1".to_string()),
                ("gu_ii", is.ii.to_string()),
                ("gu_kx", kx.to_string()),
                ("gu_nmid", nmid.to_string()),
                ("gu_nout", nout.to_string()),
                ("gu_rinner", (unit.rows_inner() as u8).to_string()),
                ("gu_cls", class.to_string()),
                ("gu_kind", kind.to_string()),
                ("gu_pool", code(pool).to_string()),
                ("gu_rpool", code(rpool).to_string()),
                ("gu_rinner_cx", (cx - 1).to_string()),
                ("gu_rx", rx.to_string()),
                ("gu_dep", (depth - FOLD_STAGES).to_string()),
                ("gu_iss", (unit.points() * is.ii as u64).to_string()),
                ("gu_body", is.body.to_string()),
            ] {
                set(n, v);
            }
            for (j, v) in values.iter().enumerate() {
                set(&format!("gp{class}_{j}"), lit64(*v));
            }
            let vec = sh.lanes > 1;
            let lsh = unit.lanes.trailing_zeros();
            if vec {
                let lv = unit.lane_var();
                let lax = if lv == unit.row.var { 0 } else if lv == unit.col.var { 1 } else { 2 };
                set("gu_lsh", lsh.to_string());
                set("gu_lax", lax.to_string());
                let last = unit.points_along(lv) - 1;
                set("gu_lanes", unit.lanes.to_string());
                set("gu_llast", last.to_string());
                set("gu_ltail", (unit.lane_extent() - last * unit.lanes).to_string());
                set("gu_tl", unit.tree_levels().to_string());
                set("gu_tpool", code(unit.tree_pool()).to_string());
                set("gu_cdep", (depth - FOLD_STAGES - unit.tree_levels()).to_string());
                if unit.lanes > 1 && !unit.tree() {
                    set("gu_wsh", lsh.to_string());
                }
            }
            for (sl, m) in unit.maps.iter().enumerate() {
                let w = SlotWalk::of(ctx, unit, ins, sl);
                set(&format!("gu_base{sl}"), lit64(m.base));
                set(&format!("gu_d0{sl}"), lit64(w.d0));
                set(&format!("gu_d1{sl}"), lit64(w.d1));
                set(&format!("gu_d2{sl}"), lit64(w.d2));
                set(&format!("gu_lim{sl}"), lit64(w.lim));
                if let Some(ind) = m.indirect.filter(|_| unit.gathers(sl)) {
                    set(&format!("gu_gon{sl}"), "1'b1".into());
                    set(&format!("gu_gst{sl}"), lit64(ind.stride));
                    set(&format!("gu_gop{sl}"), ind.operand.to_string());
                }
                set(&format!("gu_at{sl}"), is.at[sl].to_string());
                set(&format!("gu_dl{sl}"), (is.body - is.at[sl] - lat as usize - 1).to_string());
                if vec && unit.broadcast(sl) {
                    set(&format!("gu_bc{sl}"), "1'b1".into());
                }
                // The slot reads its buffer's page through the port of its FIFO on its schedule, and takes the port's data
                // as it lands; of two that share a read, the one its point's select takes.
                let k = ctx.template(ins[sl]).idx();
                let role = axis_role(unit);
                // A shared read's select decides on its point's first coordinate: along the lane axis, the run's.
                let start_axis = |v: LoopVar| -> String {
                    let wire = match role(v) {
                        "r" => "gu_r",
                        "c" => "gu_c",
                        "x" => "gu_x",
                        _ => return "32'd0".into(),
                    };
                    if lsh > 0 && v == unit.lane_var() {
                        format!("({wire} << {lsh})")
                    } else {
                        wire.into()
                    }
                };
                let addr = match is.shares.iter().find(|sh| sh.then == sl || sh.otherwise == sl) {
                    Some(share) if share.then == sl => {
                        let PredRhs::Const(bound) = share.pred.rhs() else { unreachable!("a shared read's select has a constant bound") };
                        let cmp = match share.pred {
                            Pred::AxisLt(..) => "<",
                            Pred::AxisGe(..) => ">=",
                        };
                        c.push_str(&format!(" gu_pp{sl} = ({{32'd0, {}}} {cmp} 32'd{bound});", start_axis(share.pred.var())));
                        Some(format!("gi_pp{sl} ? gu_el{sl} : gu_el{}", share.otherwise))
                    }
                    Some(_) => None,
                    None => Some(format!("gu_el{sl}")),
                };
                if let Some(addr) = addr {
                    c.push_str(&format!(" if (gi_v{sl}) begin f{k}_rd_slot = f{k}_rd_slot0; f{k}_rd_elem = {addr}; end"));
                }
                // A gather of a vector op: lane `l` reads the element its own index names through its own port.
                let lane_gather = unit.lanes > 1 && unit.gathers(sl);
                if lane_gather {
                    for l in 1..unit.lanes {
                        c.push_str(&format!(" if (gi_v{sl}) begin f{k}_l{l}_rd_slot = f{k}_rd_slot0; f{k}_l{l}_rd_elem = gu_el{sl}_l{l}; end"));
                    }
                }
                if vec {
                    // Lane `l` takes element `l` of the run the port returns, or every lane the one element it reads, or a
                    // gather's lane the element its own port returns.
                    let w = ctx.buffer_width(ins[sl]);
                    for l in 0..sh.lanes {
                        let src = if lane_gather && l > 0 && l < unit.lanes {
                            format!("f{k}_l{l}_rd_data")
                        } else if l == 0 || unit.broadcast(sl) || l >= unit.lanes {
                            format!("f{k}_rd_data")
                        } else {
                            format!("$signed(f{k}_rd_hi[{}:{}])", l as u32 * w - 1, (l as u32 - 1) * w)
                        };
                        c.push_str(&format!(" gu_ld{sl}_l{l} = {src};"));
                    }
                } else {
                    c.push_str(&format!(" gu_ld{sl} = f{k}_rd_data;"));
                }
            }
        }
        Fixed::Matmul { block, per_site_n, sub_rows, ins, reload, .. } => {
            let (osh, osw, ibw) = (block.osh, block.osw, block.ibw);
            let (k, n) = (ibw * tc, osw * tc);
            let b_row = ctx.page_row(ins[1], *per_site_n);
            let run = ctx.matmul_run(block, b_row, *per_site_n, *sub_rows);
            let (rl, la, lb) = ctx.matmul_loads(block, reload.is_some(), run);
            let (rl, la, lb) = (rl as usize, la as usize, lb as usize);
            let (p0, p1, p2, p3) = (rl, rl + la, rl + la + lb, rl + la + lb + k);
            let rows = osh * ctx.tr;
            let (ka, kb) = (ctx.template(ins[0]).idx(), ctx.template(ins[1]).idx());
            // The sub-block this op multiplies: a loop's counter where a loop over sub-blocks encloses the op, else the
            // first — a counter an earlier loop left behind is not this op's.
            let sub = |v: LoopVar| if bound.contains(&v) { var_name(v) } else { "32'd0".to_string() };
            let (shv, swv) = (sub(LV_SUB_H), sub(LV_SUB_W));
            for (name, v) in [
                ("mu_on", "1'b1".to_string()),
                ("mu_mv", if ctx.matvec(block) { "1'b1" } else { "1'b0" }.to_string()),
                ("mu_p0", p0.to_string()),
                ("mu_p1", p1.to_string()),
                ("mu_p2", p2.to_string()),
                ("mu_p3", p3.to_string()),
                ("mu_rows", rows.to_string()),
                ("mu_kop", k.to_string()),
                ("mu_nop", n.to_string()),
                ("mu_osw", osw.to_string()),
                ("mu_ibw", ibw.to_string()),
                ("mu_bwrap", (osw * te).to_string()),
                ("mu_bjump", ((b_row as i64 - osw as i64) * te as i64 + 1).to_string()),
                ("aw_cols", cols.to_string()),
            ] {
                set(name, v);
            }
            if run > 1 {
                // Each load a row's runs a cycle: the reloaded partial, the A rows, the B rows of the site's columns.
                set("mr_on", "1'b1".into());
                set("mr_lw", run.trailing_zeros().to_string());
                set("mr_n0o", (osw * te).min(run).to_string());
                set("mr_n0i", (ibw * te).min(run).to_string());
                set("mu_pn", b_row.to_string());
                let lanes = |k: usize, b: BufferId, into: &str| -> String {
                    let w = ctx.buffer_width(b);
                    (0..ctx.banks)
                        .map(|j| {
                            let src = if j == 0 { format!("f{k}_rd_data") } else { format!("f{k}_rd_hi[{}:{}]", j as u32 * w - 1, (j as u32 - 1) * w) };
                            format!(" {into}{j} = $signed({src});")
                        })
                        .collect()
                };
                if let Some(r) = reload {
                    let kr = ctx.template(*r).idx();
                    c.push_str(&format!(" if ({}) begin f{kr}_rd_slot = f{kr}_rd_slot0; f{kr}_rd_elem = mr_pos; end{}", ctx.ph_lt(p0 as u64), lanes(kr, *r, "mu_rl")));
                }
                c.push_str(&format!(
                    " if ({} && {}) begin f{ka}_rd_slot = f{ka}_rd_slot0; f{ka}_rd_elem = {shv} * {} + mr_pos; end{}",
                    ctx.ph_ge(p0 as u64),
                    ctx.ph_lt(p1 as u64),
                    osh * ibw,
                    lanes(ka, ins[0], "mu_al")
                ));
                c.push_str(&format!(
                    " if ({} && {}) begin f{kb}_rd_slot = f{kb}_rd_slot0; f{kb}_rd_elem = {swv} * {osw} + mr_pos; end{}",
                    ctx.ph_ge(p1 as u64),
                    ctx.ph_lt(p2 as u64),
                    lanes(kb, ins[1], "mu_bl")
                ));
                return Ok(());
            }
            if let Some(r) = reload {
                let kr = ctx.template(*r).idx();
                c.push_str(&format!(" if ({}) begin f{kr}_rd_slot = f{kr}_rd_slot0; f{kr}_rd_elem = ph; end mu_rr = f{kr}_rd_data;", ctx.ph_lt(p0 as u64)));
            }
            // A page tiles (sh·osh + i, kk), whole and in order; B page tiles (kk, sw·osw + j) of the site's columns.
            c.push_str(&format!(
                " if ({} && {}) begin f{ka}_rd_slot = f{ka}_rd_slot0; f{ka}_rd_elem = {shv} * {} + (ph - {p0}); end mu_ra = f{ka}_rd_data;",
                ctx.ph_ge(p0 as u64),
                ctx.ph_lt(p1 as u64),
                osh * ibw * te
            ));
            c.push_str(&format!(
                " if ({} && {}) begin f{kb}_rd_slot = f{kb}_rd_slot0; f{kb}_rd_elem = {swv} * {} + mbi_off; end mu_rb = f{kb}_rd_data;",
                ctx.ph_ge(p1 as u64),
                ctx.ph_lt(p2 as u64),
                osw * te
            ));
        }
    }
    Ok(())
}

/// A tile walk's offset deltas: within a tile row of `te` elements one, then `d1` to the next tile of the row, `d2` to
/// the first tile of the next block row.
fn walk_deltas(te: usize, block_w: usize, stride_w: usize, stride_h: usize) -> (i64, i64) {
    let (te, bw, sw, shh) = (te as i64, block_w as i64, stride_w as i64, stride_h as i64);
    (sw * te - (te - 1), shh * te - (bw - 1) * sw * te - (te - 1))
}

/// The units' registered logic.
fn units(ctx: &Ctx, sh: &Shape, wd: &Widths, te: usize, tr: usize, tc: usize, lat: u64) -> String {
    let mut e = Emit { s: String::new() };
    let lat32 = format!("32'd{lat}");
    // Tile walk.
    e.l("  always @(posedge clk)");
    e.l("    if (rst || !(running && tw_on) || op_last) begin tw_lo <= 0; tw_w <= 0; tw_off <= 0; end");
    e.l(format!(
        "    else if (ph >= tw_from) begin if (tw_lo != {}) begin tw_lo <= tw_lo + 1; tw_off <= tw_off + 1; end else begin tw_lo <= 0; if (tw_w != tw_bw - 1) begin tw_w <= tw_w + 1; tw_off <= tw_off + tw_d1; end else begin tw_w <= 0; tw_off <= tw_off + tw_d2; end end end",
        te - 1
    ));
    // Run walk: a run of a contiguous row a cycle, `2^tw_lw` elements wide — its offset in the tensor, its place in the
    // page, its length — and the same `lat` cycles on, where its data lands.
    if ctx.banks > 1 {
        let lg = log2_banks(ctx.banks);
        // A run's offset in the tensor and its length are registers, set a cycle ahead from where the walk goes next; at
        // the walk's start they are its first run's (offset 0, length `tw_n0`), so no addition stands between the walk
        // and the memory it addresses.
        e.l("  reg [AW-1:0] tw_c, tw_rb, tw_p, tw_vo_r; reg tw_first;");
        e.l(format!("  reg [{lg}:0] tw_n_r;"));
        e.l("  wire [AW-1:0] tw_l = 32'd1 << tw_lw;");
        e.l(format!("  wire [{lg}:0] tw_n = tw_first ? tw_n0 : tw_n_r;"));
        e.l("  wire [AW-1:0] tw_vo = tw_first ? 32'd0 : tw_vo_r;");
        e.l("  wire tw_adv = (tw_c + tw_l < tw_bw);");
        e.l("  wire [AW-1:0] tw_cn = tw_adv ? tw_c + tw_l : 32'd0, tw_rbn = tw_adv ? tw_rb : tw_rb + tw_rs, tw_leftn = tw_bw - tw_cn;");
        e.l(format!("  wire [{lg}:0] tw_nn = (tw_leftn < tw_l) ? tw_leftn[{lg}:0] : tw_l[{lg}:0];"));
        e.l("  always @(posedge clk)");
        e.l("    if (rst || !(running && tw_on && tw_vec) || op_last) begin tw_c <= 0; tw_rb <= 0; tw_p <= 0; tw_first <= 1'b1; end");
        let step = if sh.pitched_fill { "tw_p + tw_n + (tw_adv ? 32'd0 : tw_pp)" } else { "tw_p + tw_n" };
        e.l(format!("    else if (ph >= tw_from) begin tw_first <= 1'b0; tw_p <= {step}; tw_c <= tw_cn; tw_rb <= tw_rbn; tw_vo_r <= tw_rbn + tw_cn; tw_n_r <= tw_nn; end"));
        if lat == 0 {
            e.l(format!("  wire [AW-1:0] tw_vod = tw_vo, tw_pd = tw_p; wire [{lg}:0] tw_nd = tw_n;"));
        } else {
            for d in 1..=lat {
                let (vo, p, n) = if d == 1 { ("tw_vo".to_string(), "tw_p".to_string(), "tw_n".to_string()) } else { (format!("tw_vo{}", d - 1), format!("tw_p{}", d - 1), format!("tw_n{}", d - 1)) };
                e.l(format!(
                    "  reg [AW-1:0] tw_vo{d}, tw_p{d}; reg [{lg}:0] tw_n{d}; always @(posedge clk) begin tw_vo{d} <= {vo}; tw_p{d} <= {p}; tw_n{d} <= {n}; end"
                ));
            }
            e.l(format!("  wire [AW-1:0] tw_vod = tw_vo{lat}, tw_pd = tw_p{lat}; wire [{lg}:0] tw_nd = tw_n{lat};"));
        }
    }
    // Accumulator walk.
    e.l("  always @(posedge clk)");
    e.l("    if (rst || !running || op_last) begin aw_cc <= 0; aw_r <= 0; aw_j <= 0; aw_i <= 0; end");
    e.l(format!(
        "    else if (aw_step || mu_reload) begin if (aw_cc != {}) aw_cc <= aw_cc + 1; else begin aw_cc <= 0; if (aw_r != {}) aw_r <= aw_r + 1; else begin aw_r <= 0; if (aw_j != aw_cols - 1) aw_j <= aw_j + 1; else begin aw_j <= 0; aw_i <= aw_i + 1; end end end end",
        tc - 1,
        tr - 1
    ));
    // Unary walks: the input element wraps at its page, the bias walks one tile's elements per tile row.
    e.l("  always @(posedge clk)");
    e.l("    if (rst || !(running && un_on) || op_last) begin un_e <= 0; un_e2 <= 0; un_bk <= 0; un_bt <= 0; end");
    e.l(format!(
        "    else begin un_e <= (un_e == un_pe - 1) ? 0 : un_e + 1; un_e2 <= (un_e2 == un_pb - 1) ? 0 : un_e2 + 1; if (un_bk == {}) begin un_bk <= 0; un_bt <= (un_bt + {te} >= un_pb) ? un_bt + {te} - un_pb : un_bt + {te}; end else un_bk <= un_bk + 1; end",
        te - 1
    ));
    // Unary datapath: one multiplier for a requant or a product, taken at its node's width.
    let um = wd.un_mul.max(wd.un_out);
    e.l(format!(
        "  wire signed [{0}:0] un_y = (un_kind == 4'd5) ? $signed(unary_bias) : un_m; wire signed [{1}:0] un_mul = {2} * un_y;",
        wd.un_b.max(wd.un_m).min(um) - 1,
        um - 1,
        fit("unary_in", wd.un_in, um)
    ));
    e.l("  always @* case (un_kind)");
    e.l("    4'd0: unary_out = un_mul >>> un_s;");
    e.l("    4'd1: unary_out = $signed(unary_in) + $signed(unary_bias);");
    e.l("    4'd2: unary_out = ($signed(unary_in) < 0) ? 0 : unary_in;");
    e.l("    4'd4: unary_out = $signed(unary_in) + $signed(unary_bias);");
    e.l("    4'd5: unary_out = un_mul;");
    e.l("    4'd6: unary_out = grid_result;");
    e.l("    default: unary_out = 0;");
    e.l("  endcase");
    // Multicast walk.
    e.l("  always @(posedge clk)");
    e.l("    if (rst || !(running && mc_on) || op_last) begin mc_e <= 0; mc_pg <= 0; end");
    e.l("    else if (mc_e == mc_pe - 1) begin mc_e <= 0; mc_pg <= mc_pg + 1; end else mc_e <= mc_e + 1;");

    // Matmul unit, on the timeline its reads land on.
    if let Some((k, n, bits)) = sh.array {
        let l = sh.array.map(|(a, b, _)| pipelined_latency(a, b)).unwrap_or(0);
        e.l(format!("  wire mu_live = mu_on && {};", ctx.ph_ge(lat)));
        e.l(format!("  wire [AW-1:0] mu_q = ph - {lat32};"));
        // B issue walk: the page offset, one past each element, `bjump` past each row of the site's tiles.
        e.l("  always @(posedge clk)");
        e.l("    if (rst || !(running && mu_on) || op_last) begin mbi_t <= 0; mbi_off <= 0; end");
        e.l("    else if (ph >= mu_p1 && ph < mu_p2) begin if (mbi_t == mu_bwrap - 1) begin mbi_t <= 0; mbi_off <= mbi_off + mu_bjump; end else begin mbi_t <= mbi_t + 1; mbi_off <= mbi_off + 1; end end");
        // A and B consume walks: each element's row and lane in the operand rows.
        e.l("  always @(posedge clk)");
        e.l("    if (rst || !(running && mu_on) || op_last) begin ma_xx <= 0; ma_r <= 0; ma_kk <= 0; ma_i <= 0; mb_cc <= 0; mb_x <= 0; mb_j <= 0; mb_kk <= 0; end");
        e.l("    else if (mu_live) begin");
        e.l(format!(
            "      if (mu_q >= mu_p0 && mu_q < mu_p1) begin if (ma_xx != {}) ma_xx <= ma_xx + 1; else begin ma_xx <= 0; if (ma_r != {}) ma_r <= ma_r + 1; else begin ma_r <= 0; if (ma_kk != mu_ibw - 1) ma_kk <= ma_kk + 1; else begin ma_kk <= 0; ma_i <= ma_i + 1; end end end end",
            tc - 1,
            tr - 1
        ));
        e.l(format!(
            "      if (mu_q >= mu_p1 && mu_q < mu_p2) begin if (mb_cc != {}) mb_cc <= mb_cc + 1; else begin mb_cc <= 0; if (mb_x != {}) mb_x <= mb_x + 1; else begin mb_x <= 0; if (mb_j != mu_osw - 1) mb_j <= mb_j + 1; else begin mb_j <= 0; mb_kk <= mb_kk + 1; end end end end",
            tc - 1,
            tr - 1
        ));
        e.l("    end");
        e.l(format!("  wire [AW-1:0] ma_s = (ma_i << {}) + ma_r;", log2(tr)));
        e.l(format!("  wire [AW-1:0] ma_x = (ma_kk << {}) + ma_xx;", log2(tc)));
        e.l(format!("  wire [AW-1:0] mb_r = (mb_kk << {}) + mb_x;", log2(tc)));
        e.l(format!("  wire [AW-1:0] mb_c = (mb_j << {}) + mb_cc;", log2(tc)));
        // Operand rows, an element a cycle. A load by runs lands its own way (below): each operand register has one writer
        // block, which takes whichever landing the op loads by.
        let mut a_elem = String::new();
        for s in 0..sh.a_rows {
            for x in 0..k {
                a_elem.push_str(&format!("      if (ma_s == {s} && ma_x == {x}) a_r{s}_{x} <= mu_ra[{}:0];\n", bits - 1));
            }
        }
        let mut b_elem = String::new();
        for r in 0..k {
            for c in 0..n {
                b_elem.push_str(&format!(
                    "      if (mb_r == {r} && mb_c == {c}) b_r{r}[{}:{}] <= mu_rb[{}:0];\n",
                    (c as u32 + 1) * bits - 1,
                    c as u32 * bits,
                    bits - 1
                ));
            }
        }
        if !ctx.runs() {
            e.l("  always @(posedge clk) if (mu_live && mu_q >= mu_p0 && mu_q < mu_p1) begin");
            e.s.push_str(&a_elem);
            e.l("  end");
            e.l("  always @(posedge clk) if (mu_live && mu_q >= mu_p1 && mu_q < mu_p2) begin");
            e.s.push_str(&b_elem);
            e.l("  end");
        }
        // The columns past this op's are loaded with zero weights.
        let mut mask = format!("  wire [{}:0] wmask = {{", n as u32 * bits - 1);
        let parts: Vec<String> =
            (0..n).rev().map(|c| format!("(({c} < mu_nop) ? {{{bits}{{1'b1}}}} : {{{bits}{{1'b0}}}})")).collect();
        mask.push_str(&parts.join(", "));
        mask.push_str("};");
        e.l(mask);
        // Weight load, row stream, and the output rows the array returns `l` cycles on.
        e.l("  wire [AW-1:0] mu_w = mu_q - mu_p2;");
        e.l("  wire [AW-1:0] mu_s = mu_q - mu_p3;");
        e.l(format!("  wire [AW-1:0] mu_o = mu_q - mu_p3 - {};", l + 1));
        e.l("  always @(posedge clk) begin");
        e.l("    pe_w_load_r <= 1'b0;");
        e.l("    if (mu_live && !mu_mv && mu_q >= mu_p2 && mu_q < mu_p3) begin");
        e.l("      pe_w_load_r <= 1'b1; pe_w_row <= mu_w;");
        let mut wr = String::from("      case (mu_w)");
        for r in 0..k {
            wr.push_str(&format!(" {r}: pe_w_data <= b_r{r} & wmask;"));
        }
        wr.push_str(" default: pe_w_data <= 0; endcase");
        e.l(wr);
        e.l("    end");
        e.l("    if (mu_live && mu_q >= mu_p3) begin");
        for x in 0..k {
            let mut a = format!("      pe_a{x} <= (mu_s < mu_rows && {x} < mu_kop) ? (");
            for s in 0..sh.a_rows {
                a.push_str(&format!("(mu_s == {s}) ? $signed(a_r{s}_{x}) : "));
            }
            a.push_str("64'sd0) : 64'sd0;");
            e.l(a);
        }
        e.l("    end");
        e.l("  end");
        // The array's output rows: row `s` lands `l + 1` cycles after it streamed, every column into its bank.
        e.l(format!("  wire mw_we = mu_live && (mu_mv ? (mu_q == mu_p2 + 1) : (mu_q >= mu_p3 + {0} && mu_q < mu_p3 + {0} + mu_rows));", l + 1));
        e.l("  wire [AW-1:0] mw_row = mu_mv ? 32'd0 : mu_o, mw_n = mu_nop;");
        // The reload phase writes the accumulator through its walk.
        e.l("  assign mu_reload = mu_live && !mr_on && mu_q < mu_p0;");
        if ctx.runs() {
            // Each load phase a row's runs a cycle: its row, its block of columns, its place in the page, its length; and
            // the same `lat` cycles on, where its data lands in the operand rows or the accumulator.
            let lg = log2_banks(ctx.banks);
            // The load phase, a run's place in its page and its length are registers, set a cycle ahead from where the
            // walk goes next; at each phase's start they are its first run's (place 0, length `mr_n0o` over a row of
            // `osw`, `mr_n0i` over a row of `ibw`), so no addition stands between the walk and the page it reads.
            e.l("  reg [AW-1:0] mr_row, mr_cb, mr_base, mr_pos_r, mr_rem_r; reg mr_first; reg [1:0] mr_ph_r;");
            e.l(format!("  reg [{lg}:0] mr_n_r;"));
            e.l("  wire [AW-1:0] mr_l = 32'd1 << mr_lw;");
            // An op's first cycle takes its phase from its own table; every later one from the register set the cycle
            // before, within the op.
            e.l("  always @(posedge clk) mr_ph_r <= (ph + 1 < mu_p0) ? 2'd0 : (ph + 1 < mu_p1) ? 2'd1 : 2'd2;");
            e.l(format!("  wire [1:0] mr_ph = ({}) ? ((mu_p0 != 0) ? 2'd0 : (mu_p1 != 0) ? 2'd1 : 2'd2) : mr_ph_r;", ctx.ph_lt(1)));
            e.l("  wire [AW-1:0] mr_w = (mr_ph == 2'd1) ? mu_ibw : mu_osw;");
            e.l("  wire [AW-1:0] mr_rs = (mr_ph == 2'd0) ? mu_osw : (mr_ph == 2'd1) ? mu_ibw : mu_pn;");
            e.l("  wire [AW-1:0] mr_pos = mr_first ? 32'd0 : mr_pos_r;");
            e.l(format!("  wire [{lg}:0] mr_n = mr_first ? ((mr_ph == 2'd1) ? mr_n0i : mr_n0o) : mr_n_r;"));
            // What is left of the row from this run's start is a register, the phase's whole row at its first cycle, so
            // the walk advances on a compare of the run's width with it: another run starts at `(cb + 1)·l` exactly when
            // that is inside the row.
            e.l("  wire [AW-1:0] mr_rem = mr_first ? mr_w : mr_rem_r;");
            e.l("  wire mr_adv = (mr_l < mr_rem);");
            e.l("  wire [AW-1:0] mr_cbn = mr_adv ? mr_cb + 1 : 32'd0, mr_basen = mr_adv ? mr_base : mr_base + mr_rs;");
            e.l("  wire [AW-1:0] mr_leftn = mr_adv ? mr_rem - mr_l : mr_w;");
            e.l(format!("  wire [{lg}:0] mr_nn = (mr_leftn < mr_l) ? mr_leftn[{lg}:0] : mr_l[{lg}:0];"));
            e.l("  always @(posedge clk)");
            e.l("    if (rst || !(running && mu_on && mr_on) || op_last || ph + 1 == mu_p0 || ph + 1 == mu_p1) begin mr_row <= 0; mr_cb <= 0; mr_base <= 0; mr_first <= 1'b1; end");
            e.l("    else begin mr_first <= 1'b0; mr_cb <= mr_cbn; mr_base <= mr_basen; if (!mr_adv) mr_row <= mr_row + 1; mr_pos_r <= mr_basen + (mr_cbn << mr_lw); mr_n_r <= mr_nn; mr_rem_r <= mr_leftn; end");
            if lat == 0 {
                e.l(format!("  wire [AW-1:0] mr_rowd = mr_row, mr_cbd = mr_cb; wire [{lg}:0] mr_nd = mr_n;"));
            } else {
                for d in 1..=lat {
                    let src = |x: &str| if d == 1 { x.to_string() } else { format!("{x}{}", d - 1) };
                    e.l(format!(
                        "  reg [AW-1:0] mr_row{d}, mr_cb{d}; reg [{lg}:0] mr_n{d}; always @(posedge clk) begin mr_row{d} <= {}; mr_cb{d} <= {}; mr_n{d} <= {}; end",
                        src("mr_row"),
                        src("mr_cb"),
                        src("mr_n")
                    ));
                }
                e.l(format!("  wire [AW-1:0] mr_rowd = mr_row{lat}, mr_cbd = mr_cb{lat}; wire [{lg}:0] mr_nd = mr_n{lat};"));
            }
            // Lane `j` of a landing run of `2^mr_lw` fills the operand at column `mr_cbd·2^mr_lw + j` of its row: column `x`
            // takes lane `x mod 2^mr_lw` of run `x >> mr_lw`.
            let landing = |x: usize, dst: &str, lane: &str| -> String {
                let mut r = String::from("case (mr_lw)");
                for w in 1..=lg {
                    let j = x & ((1 << w) - 1);
                    r.push_str(&format!(" {w}: if (mr_cbd == {} && {j} < mr_nd) {dst} <= {lane}{j}[{}:0];", x >> w, bits - 1));
                }
                r.push_str(" default: ; endcase");
                r
            };
            e.l("  always @(posedge clk) if (mu_live && mu_q >= mu_p0 && mu_q < mu_p1) begin");
            e.l("    if (mr_on) begin");
            for s in 0..sh.a_rows {
                for x in 0..k {
                    e.l(format!("      if (mr_rowd == {s}) {}", landing(x, &format!("a_r{s}_{x}"), "mu_al")));
                }
            }
            e.l("    end else begin");
            e.s.push_str(&a_elem);
            e.l("    end");
            e.l("  end");
            e.l("  always @(posedge clk) if (mu_live && mu_q >= mu_p1 && mu_q < mu_p2) begin");
            e.l("    if (mr_on) begin");
            for r in 0..k {
                for c in 0..n {
                    e.l(format!("      if (mr_rowd == {r}) {}", landing(c, &format!("b_r{r}[{}:{}]", (c as u32 + 1) * bits - 1, c as u32 * bits), "mu_bl")));
                }
            }
            e.l("    end else begin");
            e.s.push_str(&b_elem);
            e.l("    end");
            e.l("  end");
            e.l("  wire mr_reload = mu_live && mr_on && mu_q < mu_p0;");
        }
        // The matrix-vector product of a one-row matmul ([`Ctx::matvec`]): column `c` multiplies each B element of its
        // column by the A element of the B element's row as it lands, and sums the products a cycle later. A run lands
        // the column's element in lane `c mod 2^mr_lw` of run `c >> mr_lw`; an element lands at its own column.
        let land_b = "mu_live && mu_q >= mu_p1 && mu_q < mu_p2";
        let kidx = if ctx.runs() { "(mr_on ? mr_rowd : mb_r)" } else { "mb_r" };
        let mut ak = format!("  reg signed [{}:0] mv_a; always @* case ({kidx})", bits - 1);
        for x in 0..k {
            ak.push_str(&format!(" {x}: mv_a = $signed(a_r0_{x});"));
        }
        ak.push_str(" default: mv_a = 0; endcase");
        e.l(ak);
        let lgb = log2_banks(ctx.banks) as usize;
        for c in 0..n {
            let element = format!("(!mr_on && mb_c == {c}) ? {{1'b1, mu_rb[{}:0]}} : ", bits - 1);
            let runs: String = if ctx.runs() {
                (1..=lgb)
                    .map(|w| {
                        let j = c & ((1 << w) - 1);
                        format!("(mr_on && mr_lw == {w} && mr_cbd == {} && {j} < mr_nd) ? {{1'b1, mu_bl{j}[{}:0]}} : ", c >> w, bits - 1)
                    })
                    .collect()
            } else {
                String::new()
            };
            e.l(format!("  wire [{bits}:0] mv_b{c} = {runs}{element}{}'d0;", bits + 1));
            // The product is as wide as its operands' product — a multiplication takes the width of what it is assigned
            // to, and the accumulator's would make it that wide a multiplier.
            e.l(format!(
                "  wire signed [{2}:0] mv_m{c} = mv_a * $signed(mv_b{c}[{1}:0]); reg mv_q{c}; reg signed [{0}:0] mv_p{c}, mv{c}; always @(posedge clk) begin mv_q{c} <= mu_mv && {land_b} && mv_b{c}[{bits}]; mv_p{c} <= mv_m{c}; end",
                wd.acc - 1,
                bits - 1,
                2 * bits - 1
            ));
            e.l(format!("  always @(posedge clk) if (rst || !(running && mu_on) || op_last) mv{c} <= 0; else if (mv_q{c}) mv{c} <= mv{c} + mv_p{c};"));
        }
    } else {
        e.l("  assign mu_reload = 1'b0;");
    }

    // Generic unit: the axes and slot walks through the issue phases, a point every interval, and — where a point's
    // value leaves its class's pipeline — the contraction, the fold and its write.
    if sh.slots > 0 {
        let g = sh.slots;
        e.l("  always @(posedge clk)");
        e.l("    if (rst || !(running && gu_on) || op_last) begin gu_sub <= 0; gu_x <= 0; gu_mid <= 0; gu_out <= 0;");
        for s in 0..g {
            e.l(format!("      gu_lin{s} <= 0;"));
        }
        e.l("    end else if (ph < gu_iss) begin");
        e.l("      if (gu_sub != gu_ii - 1) gu_sub <= gu_sub + 1;");
        e.l("      else begin gu_sub <= 0;");
        e.l("        if (gu_x != gu_kx - 1) begin gu_x <= gu_x + 1;");
        for s in 0..g {
            e.l(format!("          gu_lin{s} <= gu_lin{s} + gu_d0{s};"));
        }
        e.l("        end else begin gu_x <= 0;");
        e.l("          if (gu_mid != gu_nmid - 1) begin gu_mid <= gu_mid + 1;");
        for s in 0..g {
            e.l(format!("            gu_lin{s} <= gu_lin{s} + gu_d1{s};"));
        }
        e.l("          end else begin gu_mid <= 0;");
        e.l("            if (gu_out != gu_nout - 1) begin gu_out <= gu_out + 1;");
        for s in 0..g {
            e.l(format!("              gu_lin{s} <= gu_lin{s} + gu_d2{s};"));
        }
        e.l("            end else begin gu_out <= 0;");
        for s in 0..g {
            e.l(format!("              gu_lin{s} <= 0;"));
        }
        e.l("            end");
        e.l("          end");
        e.l("        end");
        e.l("      end");
        e.l("    end");
        e.l("  always @(posedge clk) if (gq_v && gu_kx != 1) g_kacc <= gu_pv;");
        e.l("  always @(posedge clk) if (fb_v && gu_kind != 2'd0 && fb_pd) begin g_fold <= gu_fv; g_fidx <= gu_fi; end");
        e.l("  always @(posedge clk) if (fc_v && fc_ofold) begin g_ofold <= gu_ov; g_oidx <= gu_oi; end");
        let vl = sh.lanes;
        for l in 1..vl {
            if sh.lane_contracts {
                e.l(format!("  always @(posedge clk) if (gq_v && gu_kx != 1) g_kacc_l{l} <= gu_pv_l{l};"));
            }
            if sh.lane_pools.contains(&true) {
                e.l(format!("  always @(posedge clk) if (fb_v && gu_kind != 2'd0 && fb_pd) begin g_fold_l{l} <= gu_fv_l{l}; g_fidx_l{l} <= gu_fi_l{l}; end"));
            }
        }
        // The fold's write, at stage C: a parallel point at its column, a reduced column at column 0, a reduced row at
        // its column, a full reduction at column 0 once its last row is folded. An op whose lanes each keep their column
        // writes a run of columns from its point's (`gu_wsh` its shift), each lane that is the op's its own.
        if vl > 1 {
            e.l("  wire gu_we = fc_v && fc_we && (gu_wsh == 4'd0);");
            e.l("  wire gv_we = fc_v && fc_we && (gu_wsh != 4'd0);");
        } else {
            e.l("  wire gu_we = fc_v && fc_we;");
        }
        e.l("  wire [31:0] gu_wcol = fc_wcol;");
        e.l(format!("  wire signed [{}:0] gu_wd = (gu_kind == 2'd0) ? fc_pv : (gu_kind == 2'd3) ? gu_or : fc_ir;", wd.value - 1));
        if vl > 1 {
            // ★ A RUN ROTATES INTO FIXED SLOTS. A run of a parallel op's lanes starts at column `fc_wcol << gu_wsh`, a
            // multiple of its lanes: its lane `j` writes column `first + j`. Slot `s` of the vector unit's output holds
            // the lane bound for the columns `≡ s mod vl` — lane `(s − first) mod vl` — so bank `c` takes slot `c mod vl`
            // alone, and only where the run's word of `vl` columns is the bank's and the slot's lane is one of the op's.
            let lg = vl.trailing_zeros();
            let mvw = wd.value - 1;
            let pick = |pv: &str, ir: &str| -> String {
                match (sh.lane_writes_points, sh.lane_writes_folds) {
                    (true, true) => format!("((gu_kind == 2'd0) ? {pv} : {ir})"),
                    (false, true) => ir.to_string(),
                    _ => pv.to_string(),
                }
            };
            for j in 0..vl {
                let (pv, ir) = if j == 0 { ("fc_pv".to_string(), "fc_ir".to_string()) } else { (format!("fc_pv_l{j}"), format!("fc_ir_l{j}")) };
                e.l(format!("  wire signed [{mvw}:0] gw_lane{j} = {};", pick(&pv, &ir)));
            }
            let ax = sh.axis_bits;
            e.l(format!("  wire [{}:0] gw_first = {{4'd0, fc_wcol}} << gu_wsh;", ax + 3));
            e.l(format!("  wire [{}:0] gw_rot = gw_first[{}:0];", lg - 1, lg - 1));
            for sl in 0..vl {
                e.l(format!("  wire [{}:0] gw_li{sl} = {lg}'d{sl} - gw_rot;", lg - 1));
                let mut m = format!("  reg signed [{mvw}:0] gw_slot{sl}; always @* case (gw_li{sl})");
                for j in 0..vl {
                    m.push_str(&format!(" {j}: gw_slot{sl} = gw_lane{j};"));
                }
                m.push_str(&format!(" default: gw_slot{sl} = gw_lane0; endcase"));
                e.l(m);
                e.l(format!("  wire gw_sv{sl} = fc_lv[gw_li{sl}];"));
            }
            for c in 0..sh.banks {
                e.l(format!(
                    "  wire gw_e{c} = gv_we && (gw_first[{}:{lg}] == {}) && gw_sv{}; wire signed [{mvw}:0] gw_d{c} = gw_slot{};",
                    ax + 3,
                    c >> lg,
                    c % vl,
                    c % vl
                ));
            }
        }
    } else {
        e.l("  wire gu_we = 1'b0; wire [31:0] gu_wcol = 32'd0; wire signed gu_wd = 1'b0;");
    }

    // Accumulator banks: the matmul's whole row, else the single write — the walk's element, a reload, a fold.
    e.l("  wire acc_we = sw_we || mu_reload || gu_we;");
    e.l("  wire [31:0] acc_row = gu_we ? 32'd0 : aw_row;");
    e.l("  wire [31:0] acc_col = gu_we ? gu_wcol : aw_col;");
    e.l(format!("  wire signed [{}:0] acc_d = gu_we ? gu_wd : mu_reload ? $signed(mu_rr) : sw_d;", wd.acc - 1));
    e.l("  always @(posedge clk) begin");
    let pe_cols = sh.array.map_or(0, |a| a.1);
    // An acquire's run zeroes the row's banks it covers; a matmul's reload run writes them, lane by its residue.
    let reload_run = sh.array.is_some() && ctx.runs();
    let zero = |c: usize| {
        if ctx.runs() {
            // A reload run of `2^mr_lw` writes column `c` from lane `c mod 2^mr_lw` of run `c >> mr_lw`.
            let reload = if reload_run {
                let lands: Vec<String> = (1..=log2_banks(ctx.banks) as usize)
                    .map(|w| format!("(mr_lw == {w} && mr_cbd == {} && {} < mr_nd) ? 1'b1 : ", c >> w, c & ((1 << w) - 1)))
                    .collect();
                let lane: Vec<String> = (1..=log2_banks(ctx.banks) as usize)
                    .map(|w| format!("(mr_lw == {w}) ? mu_rl{} : ", c & ((1 << w) - 1)))
                    .collect();
                format!(
                    "if (mr_reload && ({}1'b0)) acc_b{c}[mr_rowd] <= {}mu_rl0; else ",
                    lands.concat(),
                    lane.concat()
                )
            } else {
                String::new()
            };
            format!("{reload}if (az_we && ar_z[{c}]) acc_b{c}[ar_i] <= 0; else ")
        } else {
            String::new()
        }
    };
    // A vector unit's lanes write their run of columns, each bank from its own lane.
    let lane_write = |c: usize| if sh.slots > 0 && sh.lanes > 1 { format!(" else if (gw_e{c}) acc_b{c}[0] <= gw_d{c};") } else { String::new() };
    for c in 0..sh.banks {
        if c < pe_cols {
            e.l(format!(
                "    if (mw_we && {c} < mw_n) acc_b{c}[mw_row] <= acc_b{c}[mw_row] + (mu_mv ? mv{c} : pe_o{c}); else {}if (acc_we && acc_col == {c}) acc_b{c}[acc_row] <= acc_d;{}",
                zero(c),
                lane_write(c)
            ));
        } else {
            e.l(format!("    {}if (acc_we && acc_col == {c}) acc_b{c}[acc_row] <= acc_d;{}", zero(c), lane_write(c)));
        }
    }
    e.l("  end");
    let _ = (ctx, tr);
    e.s
}
