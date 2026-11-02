//! fabric_lanes_lock — a fabric's Generic unit is a vector unit as wide as its memories' banks.
//!
//! A banked site moves a run of consecutive elements a cycle ([`SiteSpec::mem_banks`]), and its Generic unit takes a
//! run a point: one axis of the op's space is its lane axis, cut into points of `L` consecutive coordinates, `L` a power
//! of two no wider than the site's vector lanes ([`SiteSpec::vector_lanes`]), themselves no wider than the banks. Every operand slot reads that axis at unit stride — a run, `L` lanes of it — or not at
//! all — one element, the same in every lane. The run of each point starts at a multiple of `L` in its page, so every
//! base and every stride along the other axes is a multiple of `L`; a slot read through a gather reads, in each lane, the
//! element its own index names, through a port of its own; a body that divides keeps its op at one lane, its divider
//! already taking a pair a cycle. A reduced lane axis folds its lanes through a tree before the op's own fold; a parallel one keeps a fold per
//! lane and writes `L` columns a point. A select whose two sides share a read keeps sharing only where its bound is a
//! multiple of `L`, so that every lane of a point takes the same side. Of the lane axes and widths an op allows, it takes
//! the one that issues its points in the fewest cycles. A fabric of one bank keeps every op at one lane. This lock holds:
//!
//! - Generic ops of every kind — a map, a fold of each pool, a contraction, a fold of both axes, the attention passes of
//!   both orientations — over unit-stride and broadcast slots, with tails past the last full run, selects decided along
//!   the lane axis, a run whose last lanes read past their page and so read zero, ties for the maximum in different lanes, partial sums no prefix reaches, runs from bases at every
//!   alignment, folds of every pool kept a lane apart, a gather and a shared read, run in RTL to the evaluator's answer in exactly the twin's cycles, at 1, 2,
//!   4, 8 and 16 banks;
//! - an op whose only run would be along the axis walked outside a fold of both axes keeps one lane: the outer axis is
//!   a lane axis only as a parallel column;
//! - each op takes the lanes its slots keep aligned: the banks where nothing narrows them, the alignment of a base or a
//!   stride where one does, one where it reads the lane axis at a stride — and of two widths that issue its points in as many cycles, its lane
//!   tree's levels counted, the narrower, so a fold of 16 to 24 takes eight lanes where sixteen would be no faster;
//! - an op of `L` lanes issues `⌈n / L⌉` points along its lane axis, where one lane issues `n`;
//! - a site's vector lanes bound every op's, however many banks its memories have, and its banks bound them where the
//!   vector unit is the wider;
//! - a fabric of one bank keeps every op of the decode step at one lane;
//! - on both boards' fabrics, each of the decode step's 384-point attention passes takes a point per run of eight;
//! - a gather takes the site's lanes, each lane reading the element its own index names through a read port of the
//!   table's FIFO of its own — indices from a run or shared, beside a run slot, with a tail, from an offset into the table
//!   — in RTL to the evaluator's answer in exactly the twin's cycles;
//! - a page a gather's lanes read through ports of their own shares a pool only with pages read through as many: every
//!   member of every pool reads through every one of its ports, on each gather and on both boards' decode steps.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test fabric_lanes_lock` (iverilog-gated)

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/fabric_rtl.rs"]
mod fabric_rtl;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::int8_model::Int8Model;
use logicaffeine_compile::spatial::ir::{
    AccPolicy, ComputeOp, ElemAccess, IndirectTerm, IterKind, IterSpec, LoopVar, NumericsClaim, Pred, PredRhs, ReducePool, SpatialProgram,
    StageKind, TensorBody, TensorInit, TensorRole, TileRect, WorkItem, WorkItemId,
};
use logicaffeine_compile::spatial::passes::from_tensor_op::Template;
use logicaffeine_compile::spatial::pipeline::run_passes;
use logicaffeine_compile::spatial::lower::fabric::{fabric_cycle_model, fabric_micro_listing, FabricBackend};
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::passes::{LV_GEN_COL, LV_GEN_K, LV_GEN_ROW};
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, compile_program_to_spatial_returning};
use logicaffeine_compile::spatial::target::SpatialTarget;
use spatial_support::{elem_map, inputs_from_fills, parallel};

const ROW: LoopVar = LV_GEN_ROW;
const COL: LoopVar = LV_GEN_COL;
const K: LoopVar = LV_GEN_K;

/// The XC7A100T's fabric with its memories in `banks` banks and a vector unit as wide.
fn banked(banks: usize) -> SpatialTarget {
    let mut t = SpatialTarget::xc7a100t_fabric(16, 64);
    t.site.mem_banks = banks;
    t.site.vector_lanes = banks;
    t
}

fn op(i: usize) -> Box<TensorBody> {
    Box::new(TensorBody::Operand(i))
}

fn k(v: i64) -> Box<TensorBody> {
    Box::new(TensorBody::Const((v as f64).to_bits()))
}

fn mul(a: Box<TensorBody>, b: Box<TensorBody>) -> Box<TensorBody> {
    Box::new(TensorBody::Mul(a, b))
}

fn add(a: Box<TensorBody>, b: Box<TensorBody>) -> Box<TensorBody> {
    Box::new(TensorBody::Add(a, b))
}

fn below(axis: LoopVar, bound: usize, then: Box<TensorBody>, otherwise: Box<TensorBody>) -> TensorBody {
    TensorBody::Select { cond: Pred::AxisLt(axis, PredRhs::Const(bound)), then, otherwise }
}

fn fold(var: LoopVar, extent: usize, pool: ReducePool) -> IterSpec {
    IterSpec { var, extent, kind: IterKind::Reduce { pool } }
}

fn spread(n: usize, lo: i64, hi: i64, seed: u64) -> Vec<i64> {
    (0..n as u64).map(|i| lo + ((i.wrapping_add(seed).wrapping_mul(2_654_435_761) >> 7) % (hi - lo + 1) as u64) as i64).collect()
}

/// A scheduled program of ONE integer Generic over `ins` — each a tensor one tile tall of the given width and values,
/// declared at the hull of its values — slot `s` reading input `slots[s]`, writing a `cols`-wide output through `out`.
#[allow(clippy::too_many_arguments)]
fn one_generic_over(
    target: &SpatialTarget,
    ins: &[(&str, usize, Vec<i64>)],
    slots: &[usize],
    cols: usize,
    iters: Vec<IterSpec>,
    maps: Vec<ElemAccess>,
    out: ElemAccess,
    body: TensorBody,
) -> SpatialProgram {
    let t = target.tile.rows;
    let mut tp = Template::new(target).expect("a template");
    let (mut loads, mut bufs, mut ids) = (Vec::new(), Vec::new(), Vec::new());
    for (name, w, vals) in ins {
        let id = tp.tensor(name, TensorRole::Input, t, *w, Precision::Int64, TensorInit::Values(vals.clone()));
        let cols_t = w.div_ceil(t);
        let b = tp.buffer(&format!("in_{name}"), cols_t, Precision::Int64);
        loads.push((tp.load(&format!("load_{name}"), id, vec![], cols_t, b), b));
        bufs.push(b);
        ids.push((id, vals.iter().fold((i64::MAX, i64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)))));
    }
    let y = tp.tensor("y", TensorRole::Output, t, cols, Precision::Int64, TensorInit::Zero);
    let out_ct = cols.div_ceil(t);
    let ob = tp.buffer("out", out_ct, Precision::Int64);
    let g = tp.stage("generic", StageKind::Compute(ComputeOp::Generic { iters, ins: maps, out, body }), slots.iter().map(|i| bufs[*i]).collect(), vec![ob]);
    let st = tp.store("store_y", y, vec![], out_ct, ob);
    for (l, b) in loads {
        tp.edge(l, b, g);
    }
    tp.edge(g, ob, st);
    let items = vec![WorkItem { id: WorkItemId::new(0), coord: (0, 0), out: TileRect { row0_t: 0, col0_t: 0, h_t: 1, w_t: out_ct } }];
    let mut p = tp
        .finish("one_generic_over".into(), items, AccPolicy::I64, NumericsClaim::Approx { reason: "a hand-built lane probe".into() }, target)
        .expect("the program validates");
    for (id, hull) in ids {
        p.tensors[id.idx()].range = Some(hull);
    }
    run_passes(p, target).expect("the program schedules")
}

/// One Generic op, as [`one_generic_over`] builds it, with the lanes this lock expects it to take at `banks` banks.
struct Shape {
    label: &'static str,
    ins: Vec<(&'static str, usize, Vec<i64>)>,
    /// The input each slot reads.
    slots: Vec<usize>,
    cols: usize,
    iters: Vec<IterSpec>,
    maps: Vec<ElemAccess>,
    out: ElemAccess,
    body: TensorBody,
    /// The lanes the op takes at a bank count, where the rule decides them alone; `None` where a cost comparison does.
    lanes: fn(usize) -> Option<usize>,
}

impl Shape {
    fn program(&self, target: &SpatialTarget) -> SpatialProgram {
        one_generic_over(target, &self.ins, &self.slots, self.cols, self.iters.clone(), self.maps.clone(), self.out.clone(), self.body.clone())
    }
}

/// Two heads of eight over 24 cached positions: the scores `s[h·24 + j] = Σ_e q[8h + e]·k[16j + 8h + e]`, the fold over
/// the head dimension walked inside the positions, each head's slots chosen by the column's band.
fn scores() -> Shape {
    let maps = (0..2)
        .flat_map(|h: i64| [elem_map(8 * h, &[(ROW, 1)]), elem_map(8 * h - h * 24 * 16, &[(COL, 16), (ROW, 1)])])
        .collect();
    Shape {
        label: "two-head scores",
        ins: vec![("q", 16, spread(16, -128, 127, 41)), ("kc", 384, spread(384, -128, 127, 42))],
        slots: vec![0, 1, 0, 1],
        cols: 48,
        iters: vec![fold(ROW, 8, ReducePool::Sum), parallel(COL, 48)],
        maps,
        out: elem_map(0, &[(COL, 1)]),
        body: below(COL, 24, mul(op(0), op(1)), mul(op(2), op(3))),
        lanes: |b| Some(b.min(8)),
    }
}

/// The weighted sum of two heads over 24 positions: `n[8h + e] = Σ_j w[24h + j]·v[16j + 8h + e]` — a fold over the
/// positions whose weights read along it and whose values read across it.
fn context() -> Shape {
    let maps = (0..2)
        .flat_map(|h: i64| [elem_map(24 * h, &[(ROW, 1)]), elem_map(0, &[(ROW, 16), (COL, 1)])])
        .collect();
    Shape {
        label: "two-head weighted sum",
        ins: vec![("w", 48, spread(48, 0, 32768, 43)), ("vc", 384, spread(384, -128, 127, 44))],
        slots: vec![0, 1, 0, 1],
        cols: 16,
        iters: vec![fold(ROW, 24, ReducePool::Sum), parallel(COL, 16)],
        maps,
        out: elem_map(0, &[(COL, 1)]),
        body: below(COL, 8, mul(op(0), op(1)), mul(op(2), op(3))),
        lanes: |b| Some(b.min(8)),
    }
}

fn shapes() -> Vec<Shape> {
    let row = |n| vec![parallel(ROW, 1), parallel(COL, n)];
    let unit = |base| elem_map(base, &[(COL, 1)]);
    let one = elem_map(0, &[]);
    let requant = TensorBody::Min(
        Box::new(TensorBody::Max(Box::new(TensorBody::Shr { arg: mul(op(0), k(104_593)), by: 24 }), k(-128))),
        k(127),
    );
    let mut ties = spread(19, -400, 300, 45);
    // Two maxima in one run of eight, so the lane tree breaks a tie, and two more in later runs, so the fold does.
    for at in [2, 5, 11, 17] {
        ties[at] = 399;
    }
    let mut v = vec![
        Shape {
            label: "requant and clamp of 13",
            ins: vec![("x", 13, spread(13, -258_064, 258_064, 46))],
            slots: (0..1).collect(),
            cols: 13,
            iters: row(13),
            maps: vec![unit(0)],
            out: unit(0),
            body: requant,
            lanes: |b| Some(b),
        },
        Shape {
            label: "sum of two rows of 16",
            ins: vec![("x", 16, spread(16, -900, 900, 47)), ("y", 16, spread(16, -900, 900, 48))],
            slots: (0..2).collect(),
            cols: 16,
            iters: row(16),
            maps: vec![unit(0), unit(0)],
            out: unit(0),
            body: *add(op(0), op(1)),
            lanes: |b| Some(b),
        },
        Shape {
            label: "a row of 16 times a scalar",
            ins: vec![("x", 16, spread(16, -900, 900, 49)), ("s", 1, vec![-37])],
            slots: (0..2).collect(),
            cols: 16,
            iters: row(16),
            maps: vec![unit(0), one.clone()],
            out: unit(0),
            body: *mul(op(0), op(1)),
            lanes: |b| Some(b),
        },
        Shape {
            label: "sum of a row of 24",
            ins: vec![("x", 24, spread(24, -900, 900, 50))],
            slots: (0..1).collect(),
            cols: 1,
            iters: vec![parallel(ROW, 1), fold(COL, 24, ReducePool::Sum)],
            maps: vec![unit(0)],
            out: one.clone(),
            body: TensorBody::Operand(0),
            lanes: |b| Some(b.min(8)),
        },
        Shape {
            label: "maximum of a row of 21, past 13 its first",
            ins: vec![("x", 21, spread(21, -900, 900, 51)), ("f", 1, vec![-901])],
            slots: (0..2).collect(),
            cols: 1,
            iters: vec![parallel(ROW, 1), fold(COL, 21, ReducePool::Max)],
            maps: vec![unit(0), one.clone()],
            out: one.clone(),
            body: below(COL, 13, op(0), op(1)),
            lanes: |b| Some(b.min(8)),
        },
        Shape {
            label: "first maximum of a row of 19 with ties",
            ins: vec![("x", 19, ties)],
            slots: (0..1).collect(),
            cols: 1,
            iters: vec![parallel(ROW, 1), fold(COL, 19, ReducePool::ArgMax)],
            maps: vec![unit(0)],
            out: one.clone(),
            body: TensorBody::Operand(0),
            lanes: |b| Some(b.min(8)),
        },
        Shape {
            label: "one more than each of a row of 21, summed",
            ins: vec![("x", 21, spread(21, -900, 900, 66))],
            slots: (0..1).collect(),
            cols: 1,
            iters: vec![parallel(ROW, 1), fold(COL, 21, ReducePool::Sum)],
            maps: vec![unit(0)],
            out: one.clone(),
            body: *add(op(0), k(1)),
            lanes: |b| Some(b.min(8)),
        },
        Shape {
            label: "alternating extremes summed",
            ins: vec![("x", 16, (0..16).map(|i| if i % 2 == 0 { 1 << 40 } else { -(1 << 40) }).collect())],
            slots: (0..1).collect(),
            cols: 1,
            iters: vec![parallel(ROW, 1), fold(COL, 16, ReducePool::Sum)],
            maps: vec![unit(0)],
            out: one.clone(),
            body: TensorBody::Operand(0),
            lanes: |b| Some(b.min(8)),
        },
        Shape {
            label: "a contraction whose weights read across it",
            ins: vec![("x", 8, spread(8, -128, 127, 52)), ("w", 96, spread(96, -128, 127, 53))],
            slots: (0..2).collect(),
            cols: 12,
            iters: vec![parallel(ROW, 1), parallel(COL, 12), fold(K, 8, ReducePool::Sum)],
            maps: vec![elem_map(0, &[(K, 1)]), elem_map(0, &[(K, 12), (COL, 1)])],
            out: unit(0),
            body: *mul(op(0), op(1)),
            lanes: |b| Some(b.min(4)),
        },
        Shape {
            label: "a contraction whose weights read along it",
            ins: vec![("x", 8, spread(8, -128, 127, 54)), ("w", 96, spread(96, -128, 127, 55))],
            slots: (0..2).collect(),
            cols: 12,
            iters: vec![parallel(ROW, 1), parallel(COL, 12), fold(K, 8, ReducePool::Sum)],
            maps: vec![elem_map(0, &[(K, 1)]), elem_map(0, &[(COL, 8), (K, 1)])],
            out: unit(0),
            body: *mul(op(0), op(1)),
            lanes: |b| Some(b.min(8)),
        },
        Shape {
            label: "a total over 3 rows of 10",
            ins: vec![("x", 30, spread(30, -900, 900, 56))],
            slots: (0..1).collect(),
            cols: 1,
            iters: vec![fold(ROW, 3, ReducePool::Sum), fold(COL, 10, ReducePool::Sum)],
            maps: vec![elem_map(0, &[(ROW, 10), (COL, 1)])],
            out: one.clone(),
            body: TensorBody::Operand(0),
            lanes: |b| Some(b.min(2)),
        },
        Shape {
            label: "a gather through a table",
            ins: vec![("ix", 16, spread(16, 0, 7, 57)), ("e", 8, vec![32768, 28672, 25088, 21952, 19208, 16807, 14706, 12868])],
            slots: (0..2).collect(),
            cols: 16,
            iters: row(16),
            maps: vec![unit(0), ElemAccess { base: 0, terms: Vec::new(), indirect: Some(IndirectTerm { operand: 0, stride: 1, modulus: 8 }) }],
            out: unit(0),
            body: TensorBody::Operand(1),
            lanes: |b| Some(b),
        },
        Shape {
            label: "a row of 12 read as 16, its last four past its page",
            ins: vec![("x", 12, spread(12, -900, 900, 68))],
            slots: (0..1).collect(),
            cols: 16,
            iters: row(16),
            maps: vec![unit(0)],
            out: unit(0),
            body: *add(op(0), k(5)),
            lanes: |b| Some(b),
        },
        Shape {
            label: "a total over 4 rows of 5 read down its columns",
            ins: vec![("x", 20, spread(20, -900, 900, 69))],
            slots: (0..1).collect(),
            cols: 1,
            iters: vec![fold(ROW, 4, ReducePool::Sum), fold(COL, 5, ReducePool::Sum)],
            maps: vec![elem_map(0, &[(ROW, 1), (COL, 4)])],
            out: one.clone(),
            body: TensorBody::Operand(0),
            lanes: |_| Some(1),
        },
        Shape {
            label: "the largest of 5 rows in each of 16 columns",
            ins: vec![("x", 80, spread(80, -900, 900, 70))],
            slots: (0..1).collect(),
            cols: 16,
            iters: vec![fold(ROW, 5, ReducePool::Max), parallel(COL, 16)],
            maps: vec![elem_map(0, &[(ROW, 16), (COL, 1)])],
            out: unit(0),
            body: TensorBody::Operand(0),
            lanes: |b| Some(b),
        },
        Shape {
            label: "the row of the first largest of 5 in each of 16 columns",
            ins: vec![("x", 80, { let mut v = spread(80, -900, 900, 71); v[16 + 3] = 950; v[48 + 3] = 950; v[32 + 9] = 950; v })],
            slots: (0..1).collect(),
            cols: 16,
            iters: vec![fold(ROW, 5, ReducePool::ArgMax), parallel(COL, 16)],
            maps: vec![elem_map(0, &[(ROW, 16), (COL, 1)])],
            out: unit(0),
            body: TensorBody::Operand(0),
            lanes: |b| Some(b),
        },
        Shape {
            label: "every other element of a row of 32",
            ins: vec![("x", 32, spread(32, -900, 900, 67))],
            slots: (0..1).collect(),
            cols: 16,
            iters: row(16),
            maps: vec![elem_map(0, &[(COL, 2)])],
            out: unit(0),
            body: *add(op(0), k(3)),
            lanes: |_| Some(1),
        },
        Shape {
            label: "a row of 16 divided by a scalar",
            ins: vec![("x", 16, spread(16, -9000, 9000, 59)), ("d", 1, vec![-37])],
            slots: (0..2).collect(),
            cols: 16,
            iters: row(16),
            maps: vec![unit(0), one.clone()],
            out: unit(0),
            body: TensorBody::FloorDiv(op(0), op(1)),
            lanes: |_| Some(1),
        },
        Shape {
            label: "a select over one input",
            ins: vec![("a", 24, spread(24, -500, 500, 58))],
            slots: vec![0, 0, 0],
            cols: 8,
            iters: row(8),
            maps: vec![unit(0), unit(8), unit(16)],
            out: unit(0),
            body: below(COL, 4, add(op(0), op(1)), add(op(0), op(2))),
            lanes: |_| None,
        },
        scores(),
        context(),
    ];
    // A run from a base at each alignment: the largest power of two dividing it, at most the banks.
    for (base, label) in [(1, "a run from base 1"), (2, "a run from base 2"), (3, "a run from base 3"), (4, "a run from base 4"), (6, "a run from base 6"), (8, "a run from base 8")] {
        v.push(Shape {
            label,
            ins: vec![("x", 24, spread(24, -900, 900, 60 + base as u64))],
            slots: (0..1).collect(),
            cols: 16,
            iters: row(16),
            maps: vec![unit(base)],
            out: unit(0),
            body: *add(op(0), k(1)),
            lanes: match base {
                1 | 3 => |_| Some(1),
                2 | 6 => |b: usize| Some(b.min(2)),
                4 => |b: usize| Some(b.min(4)),
                _ => |b: usize| Some(b.min(8)),
            },
        });
    }
    v
}

/// `p` on `target` under iverilog: every output and state tensor the evaluator's, in exactly the twin's cycles.
fn holds(label: &str, p: &SpatialProgram, target: &SpatialTarget) {
    let inputs = inputs_from_fills(p);
    let want = eval_choreography(p, &inputs).unwrap_or_else(|e| panic!("{label}: the evaluator: {e:?}"));
    let art = match FabricBackend::new(target.clone()).lower(p) {
        Ok(SpatialArtifact::Verilog(v)) => v,
        other => panic!("{label}: the fabric lowers it: {other:?}"),
    };
    let dir = std::env::temp_dir().join(format!("fabric_lanes_{}_{}_{}", label.replace(' ', "_"), target.site.mem_banks, std::process::id()));
    let run = fabric_rtl::simulate(&dir, p, &art, &inputs);
    assert!(run.done, "{label}: the design finishes\n{}", run.log);
    for t in p.tensors.iter().filter(|t| matches!(t.role, TensorRole::Output | TensorRole::State)) {
        let Some(Value::Int(w)) = want.tensors.get(&t.id) else { continue };
        let got = run.out.get(&t.id.idx()).unwrap_or_else(|| panic!("{label}: the testbench prints {}", t.name));
        assert_eq!(got, w, "{label}: {} in RTL is the evaluator's", t.name);
    }
    assert_eq!(run.cycles, fabric_cycle_model(p, target), "{label}: the RTL's cycles are the twin's");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The field `name: <usize>` of a micro listing line.
fn field(line: &str, name: &str) -> Option<usize> {
    let at = line.find(&format!("{name}: "))? + name.len() + 2;
    line[at..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

/// Every Generic op of `p` on `target`: its cycles, its lanes, its issue interval and the points it issues.
fn generic_ops(p: &SpatialProgram, target: &SpatialTarget) -> Vec<(u64, usize, usize, usize)> {
    fabric_micro_listing(p, target)
        .expect("the fabric lowers it")
        .into_iter()
        .flat_map(|(_, _, lines)| lines)
        .filter(|l| l.starts_with("generic ["))
        .map(|l| {
            let cycles = l[l.find('[').unwrap() + 1..l.find(']').unwrap()].parse().unwrap();
            let lanes = field(&l, "lanes").unwrap_or_else(|| panic!("a Generic op names its lanes: {l}"));
            let ii = field(&l, "ii").expect("its issue interval");
            let points = field(&l, "points").unwrap_or_else(|| panic!("a Generic op names the points it issues: {l}"));
            (cycles, lanes, ii, points)
        })
        .collect()
}

#[test]
fn every_shape_runs_on_every_bank_count_to_the_evaluator_s_answer_in_the_twin_s_cycles() {
    if !common::require_tool("iverilog") {
        return;
    }
    for banks in [1, 2, 4, 8, 16] {
        let target = banked(banks);
        for s in shapes() {
            holds(&format!("{} at {banks} banks", s.label), &s.program(&target), &target);
        }
    }
}

#[test]
fn each_op_takes_the_lanes_its_slots_keep_aligned() {
    for banks in [1, 2, 4, 8, 16] {
        let target = banked(banks);
        for s in shapes() {
            let ops = generic_ops(&s.program(&target), &target);
            assert_eq!(ops.len(), 1, "{}: one Generic op", s.label);
            if let Some(want) = (s.lanes)(banks) {
                assert_eq!(ops[0].1, want, "{} at {banks} banks: its lanes", s.label);
            }
            assert!(ops[0].1.is_power_of_two() && ops[0].1 <= banks, "{} at {banks} banks: {} lanes, a power of two no wider than the banks", s.label, ops[0].1);
        }
    }
}

/// The points an op issues: its lane axis's extent cut into runs of its lanes, times every other axis's extent.
#[test]
fn an_op_of_l_lanes_issues_a_point_per_run_of_its_lane_axis() {
    // Each shape's extents and the axis its lanes run along, where its lanes are more than one.
    let lane_axis = |label: &str| -> Option<(usize, usize)> {
        Some(match label {
            "requant and clamp of 13" => (13, 1),
            "sum of two rows of 16" | "a row of 16 times a scalar" | "a row of 12 read as 16, its last four past its page" => (16, 1),
            "the largest of 5 rows in each of 16 columns" | "the row of the first largest of 5 in each of 16 columns" => (16, 5),
            "sum of a row of 24" => (24, 1),
            "maximum of a row of 21, past 13 its first" | "one more than each of a row of 21, summed" => (21, 1),
            "first maximum of a row of 19 with ties" => (19, 1),
            "alternating extremes summed" => (16, 1),
            "a contraction whose weights read across it" => (12, 8),
            "a contraction whose weights read along it" => (8, 12),
            "a total over 3 rows of 10" => (10, 3),
            "two-head scores" => (8, 48),
            "two-head weighted sum" => (16, 24),
            l if l.starts_with("a run from base") => (16, 1),
            _ => return None,
        })
    };
    for banks in [1, 2, 4, 8, 16] {
        let target = banked(banks);
        for s in shapes() {
            let Some((along, others)) = lane_axis(s.label) else { continue };
            let (_, lanes, _, points) = generic_ops(&s.program(&target), &target)[0];
            assert_eq!(points, along.div_ceil(lanes) * others, "{} at {banks} banks: {lanes} lanes issue a point per run", s.label);
        }
    }
}

#[test]
fn a_site_s_vector_lanes_bound_every_op_s_lanes_whatever_its_banks() {
    for (banks, lanes) in [(16, 4), (16, 1), (8, 2), (4, 4), (4, 16), (2, 8)] {
        let mut target = banked(banks);
        target.site.vector_lanes = lanes;
        let widest = lanes.min(banks);
        for s in shapes() {
            let (_, got, _, _) = generic_ops(&s.program(&target), &target)[0];
            assert!(got <= widest, "{} at {banks} banks and {lanes} vector lanes: {got} lanes", s.label);
            if let Some(want) = (s.lanes)(widest) {
                assert_eq!(got, want, "{} at {banks} banks and {lanes} vector lanes: the lanes it takes at {widest}", s.label);
            }
        }
    }
}

fn decode_step() -> String {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let m = Int8Model::from_json(&j).expect("the model");
    m.decode_step_lg(j["golden_trace"]["prompt_tokens"][0].as_u64().unwrap() as usize, 0, &[], &[])
}

#[test]
fn a_fabric_of_one_bank_keeps_every_op_of_the_decode_step_at_one_lane() {
    for target in [SpatialTarget::sky_fabric(16, 64), banked(1)] {
        let p = compile_program_to_spatial(&decode_step(), &target).unwrap().program.expect("the step lowers whole");
        for (cycles, lanes, _, _) in generic_ops(&p, &target) {
            assert_eq!(lanes, 1, "{}: an op of {cycles} cycles takes one lane", target.name);
        }
    }
}

/// Each of the decode step's two 384-point attention passes — the scores and the weighted sum — issues a point per run
/// of eight on both boards' fabrics: under a sixth of its points in cycles.
#[test]
fn each_attention_pass_of_the_decode_step_takes_a_point_per_run_of_eight_on_both_boards() {
    for target in [SpatialTarget::xc7a100t_fabric(16, 64), SpatialTarget::xc7a200t_fabric(16, 64)] {
        let p = compile_program_to_spatial(&decode_step(), &target).unwrap().program.expect("the step lowers whole");
        let passes: Vec<(u64, usize, usize, usize)> = generic_ops(&p, &target).into_iter().filter(|op| op.1 * op.3 == 384).collect();
        assert_eq!(passes.len(), 2, "{}: the step's two 384-point attention passes, each at eight lanes", target.name);
        for (cycles, lanes, ii, points) in passes {
            assert_eq!((lanes, ii, points), (8, 1, 48), "{}: eight lanes, a point a cycle", target.name);
            assert!(cycles < 384 / 6, "{}: a 384-point pass takes {cycles} cycles", target.name);
        }
    }
}

/// Gathers through a table, each lane of a point reading its own element of the table through a read port of its own:
/// indices from a run, an index every lane shares, a gather beside a slot read as a run, a tail past the last full run,
/// a table read from an offset.
fn gathers() -> Vec<(&'static str, Shape)> {
    let row = |n| vec![parallel(ROW, 1), parallel(COL, n)];
    let unit = |base| elem_map(base, &[(COL, 1)]);
    let table = vec![32768, 28672, 25088, 21952, 19208, 16807, 14706, 12868, 11259, 9852, 8620, 7543];
    let through = |base: i64, operand: usize, modulus: usize| ElemAccess { base, terms: Vec::new(), indirect: Some(IndirectTerm { operand, stride: 1, modulus }) };
    vec![
        (
            "sixteen indices into a table of eight",
            Shape {
                label: "a gather of sixteen through a table of eight",
                ins: vec![("ix", 16, spread(16, 0, 7, 71)), ("e", 8, table[..8].to_vec())],
                slots: (0..2).collect(),
                cols: 16,
                iters: row(16),
                maps: vec![unit(0), through(0, 0, 8)],
                out: unit(0),
                body: TensorBody::Operand(1),
                lanes: |_| None,
            },
        ),
        (
            "one index every lane shares",
            Shape {
                label: "a gather by one shared index",
                ins: vec![("ix", 1, vec![5]), ("e", 8, table[..8].to_vec())],
                slots: (0..2).collect(),
                cols: 16,
                iters: row(16),
                maps: vec![elem_map(0, &[]), through(0, 0, 8)],
                out: unit(0),
                body: TensorBody::Operand(1),
                lanes: |_| None,
            },
        ),
        (
            "a gather beside a run",
            Shape {
                label: "a gather plus a run",
                ins: vec![("ix", 16, spread(16, 0, 11, 72)), ("e", 12, table.clone()), ("x", 16, spread(16, -900, 900, 73))],
                slots: (0..3).collect(),
                cols: 16,
                iters: row(16),
                maps: vec![unit(0), through(0, 0, 12), unit(0)],
                out: unit(0),
                body: *add(op(1), op(2)),
                lanes: |_| None,
            },
        ),
        (
            "a tail past the last full run",
            Shape {
                label: "a gather of thirteen",
                ins: vec![("ix", 13, spread(13, 0, 7, 74)), ("e", 8, table[..8].to_vec())],
                slots: (0..2).collect(),
                cols: 13,
                iters: row(13),
                maps: vec![unit(0), through(0, 0, 8)],
                out: unit(0),
                body: TensorBody::Operand(1),
                lanes: |_| None,
            },
        ),
        (
            "a table read from an offset",
            Shape {
                label: "a gather from the table's fourth entry",
                ins: vec![("ix", 16, spread(16, 0, 7, 75)), ("e", 12, table.clone())],
                slots: (0..2).collect(),
                cols: 16,
                iters: row(16),
                maps: vec![unit(0), through(4, 0, 8)],
                out: unit(0),
                body: TensorBody::Operand(1),
                lanes: |_| None,
            },
        ),
    ]
}

/// ★ A GATHER TAKES LANES. Each lane of a point reads its own element of the table — at the index its own lane of the
/// index slot read — through a read port of the table's FIFO of its own, so a gather issues a point per run of its lane
/// axis like any other op: as many lanes as the site's vector unit has, `⌈n / L⌉` points, and in RTL the evaluator's answer
/// in exactly the twin's cycles at every bank count.
#[test]
fn a_gather_takes_a_lane_per_read_port_of_its_table() {
    for banks in [1usize, 2, 4, 8, 16] {
        let target = banked(banks);
        for (what, s) in gathers() {
            let p = s.program(&target);
            let ops = generic_ops(&p, &target);
            assert_eq!(ops.len(), 1, "{what}: one Generic op");
            let (_, lanes, _, points) = ops[0];
            assert_eq!(lanes, banks, "{what} at {banks} banks: a lane per bank");
            assert_eq!(points, s.cols.div_ceil(banks), "{what} at {banks} banks: a point per run of {banks}");
            if common::require_tool("iverilog") {
                holds(&format!("{} at {banks} banks", s.label), &p, &target);
            }
        }
    }
}

/// Each pool of the RTL as `(pool, members, ports)`: the buffers whose writes its `pool<i>_we` merges and the read ports
/// its `pool<i>_ra<r>` addresses, with, for each buffer, the ports it reads through (`assign f<b>_rd<r>_data =
/// pool<i>_rd<r>;`).
fn pools_of(rtl: &str) -> Vec<(usize, Vec<(usize, std::collections::BTreeSet<usize>)>, usize)> {
    let mut out = Vec::new();
    for i in 0.. {
        let we = format!("wire pool{i}_we = ");
        let Some(line) = rtl.lines().find(|l| l.trim_start().starts_with(&we)) else { break };
        let members: Vec<usize> = line
            .split(" | ")
            .filter_map(|t| t.trim().trim_start_matches(&we).trim_end_matches(';').strip_prefix('f')?.strip_suffix("_wr_en")?.parse().ok())
            .collect();
        let ports = (0..).take_while(|r| rtl.contains(&format!("wire [31:0] pool{i}_ra{r} = "))).count();
        let reads = members
            .iter()
            .map(|b| (*b, (0..ports).filter(|r| rtl.contains(&format!("assign f{b}_rd{r}_data = pool{i}_rd{r};"))).collect()))
            .collect();
        out.push((i, reads, ports));
    }
    out
}

/// ★ A GATHER'S LANE PORTS COPY ONLY WHAT READS THROUGH THEM. Every read port of a pool is a copy of the whole pool, so a
/// page a gather's lanes read through ports of their own shares a pool only with pages read through as many: every member
/// of every pool reads through every one of its ports. Held on each gather at every bank count and on both boards' decode
/// steps as their images lower them — Board A's at eight lanes, Board B's at four — where the gathered exp table would
/// otherwise take the site's largest pool's ports eight and four times over.
#[test]
fn a_gather_s_lane_ports_copy_only_the_pages_read_through_them() {
    let mut cases: Vec<(String, SpatialProgram, SpatialTarget)> = Vec::new();
    for banks in [2usize, 4, 8, 16] {
        let target = banked(banks);
        for (what, s) in gathers() {
            cases.push((format!("{what} at {banks} banks"), s.program(&target), target.clone()));
        }
    }
    for (board, mut target, lanes) in [("Board A", SpatialTarget::xc7a200t_fabric(16, 64), 8), ("Board B", SpatialTarget::xc7a100t_fabric(16, 64), 4)] {
        target.site.vector_lanes = lanes;
        let l = compile_program_to_spatial_returning(&decode_step(), &target, &Int8Model::step_reads()).expect("the decode step");
        cases.push((format!("{board}'s decode step at {lanes} lanes"), l.program.expect("the step lowers whole"), target));
    }
    for (label, p, target) in &cases {
        let rtl = match FabricBackend::new(target.clone()).lower(p) {
            Ok(SpatialArtifact::Verilog(v)) => v.rtl,
            other => panic!("{label}: lowers: {other:?}"),
        };
        let pools = pools_of(&rtl);
        assert!(pools.iter().any(|(_, _, ports)| *ports > 1), "{label}: a gather's lanes read through ports of their own");
        for (i, members, ports) in pools {
            for (b, reads) in &members {
                assert_eq!(reads.len(), ports, "{label}: f{b} reads through {reads:?} of pool{i}'s {ports} ports");
            }
        }
    }
}

/// A gather whose table read moves along the lane axis — `y[c] = e[c + ix[c]]` — keeps its op at one lane: a lane's
/// element would be its index plus its own coordinate, and a gather's lanes read where their indices alone say. It runs
/// in RTL to the evaluator's answer in the twin's cycles at every bank count.
#[test]
fn a_gather_that_moves_along_its_lanes_keeps_one_lane() {
    let s = Shape {
        label: "a gather of ix[c] past column c",
        ins: vec![("ix", 16, spread(16, 0, 7, 76)), ("e", 24, spread(24, -5000, 5000, 77))],
        slots: (0..2).collect(),
        cols: 16,
        iters: vec![parallel(ROW, 1), parallel(COL, 16)],
        maps: vec![
            elem_map(0, &[(COL, 1)]),
            ElemAccess { base: 0, terms: vec![(COL, 1)], indirect: Some(IndirectTerm { operand: 0, stride: 1, modulus: 8 }) },
        ],
        out: elem_map(0, &[(COL, 1)]),
        body: TensorBody::Operand(1),
        lanes: |_| Some(1),
    };
    for banks in [1usize, 2, 4, 8, 16] {
        let target = banked(banks);
        let p = s.program(&target);
        let ops = generic_ops(&p, &target);
        assert_eq!(ops.len(), 1, "one Generic op");
        assert_eq!(ops[0].1, 1, "{} at {banks} banks: one lane", s.label);
        if common::require_tool("iverilog") {
            holds(&format!("{} at {banks} banks", s.label), &p, &target);
        }
    }
}
