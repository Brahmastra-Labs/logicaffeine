//! Where a lane's arrays live. Every array the program allocates goes on the device while they fit the memory it is
//! given; past it, constant matrices move to pinned host memory the device maps, in the order that costs a step least:
//! a table only ever gathered from (a row a token), then each expert stack from the last layer's back (a step reads its
//! chosen experts' rows), then the other matrices from the last read back — until the rest fits. A kernel reads a mapped
//! matrix through the same pointer it reads one in device memory, so where a matrix lives changes no value. Matrices
//! that read the same checkpoint rows (a tied head and the embedding table it is) share one upload.
//!
//! A matrix in host memory that a launch reads whole streams to the device a launch at a time when it can: the copy
//! engine copies it into one of two device slots while the launch before it runs, and the launch reads the slot — a
//! chunk of many rows reads it over the link once, not once a tile, and at the copy engine's rate. A table only gathered
//! from and an expert stack stay where they are (a step reads their rows), and so does a matrix past [`SLOT_CAP`], which
//! leaves the device after every one that streams.
//!
//! The compiler fixes the order ([`plan`]); the program decides how far down it to go when it starts, from the device
//! memory free once its context is up (or the `--device-bytes` it is given), less its [`Plan::spare`] — [`place`] is
//! that decision, as the program makes it. Where a moved matrix streams, the program's slots and its copies' graph
//! memory ([`staging`]) come out of the free memory before it places, and sit beside a budget it is given. The arena is an allocation of its own; every other array the program keeps
//! on the device is carved from one pool, and every matrix it moves from one mapped host pool, each at [`POOL_ALIGN`],
//! so the bytes counted are the bytes the device gives up.

use std::collections::{BTreeMap, BTreeSet};

use super::{arena, kernels};
use super::plan::{quant_in_place, Fill, Launch, Touch, QBLOCK};
use super::weights::{Placement, Rows};
use super::{Form, FormKind, LaneError};
use crate::codegen_cuda::qwen_emit::Q8MatKind;
use crate::model::quant::GgmlType;

/// The device memory a chip loses before the program allocates: what the driver holds back from every program (379 MiB
/// of the RTX 3060's 12 GiB under driver 580) and the program's context with its modules loaded (113 MiB).
pub const CONTEXT_RESERVE: usize = 512 << 20;

/// What the program's graphs, streams and events, and the rounding of its three allocations to the driver's 2 MiB
/// granule, take of the device beside its arrays, past [`SPARE_A_LAUNCH`] for each node its graphs capture.
pub const SPARE: usize = 32 << 20;

/// The device memory each node a graph captures takes (`emit::captured_nodes`: a kernel, a memory set or copy): the 21.2
/// KiB a node takes on the RTX 3060 under driver 580 (Llama-3.2-1B's batch lane: 69.2 MB over 3,192 nodes, the free
/// memory before and after its graphs; Qwen3.8-27B's 18,590 nodes outran 16 KiB each) and an eighth more. The program
/// prints what its graphs took (`GRAPH_BYTES`).
pub const SPARE_A_LAUNCH: usize = 24 << 10;

/// The most bytes one launch's streamed matrices take: a dense layer's widest launch of a 27B model streams
/// (Qwen3.8-27B's gated pair, 160 MB); a matrix past it (a 152K-row head) stays on the device.
pub const SLOT_CAP: usize = 256 << 20;

/// The alignment of every array carved from a pool: `cudaMalloc`'s, the arena's.
pub const POOL_ALIGN: usize = arena::ALIGN;

/// The bytes an array of `n` bytes takes of its pool.
pub fn pooled(n: usize) -> usize {
    n.next_multiple_of(POOL_ALIGN)
}

/// What a lane's program allocates, before it decides where its matrices live.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// The arena's bytes, an allocation of its own.
    pub arena: usize,
    /// The bytes of the device pool's arrays that stay on the device whatever the budget: the buffers, scratch,
    /// computed constants, and the generation's position, run, count and prompt.
    pub fixed: usize,
    /// Every matrix the program uploads and the bytes it takes, in the order they leave the device.
    pub order: Vec<(String, usize)>,
    /// The matrices that read the rows another matrix uploads, each with the name of the one that uploads them.
    pub shares: BTreeMap<String, String>,
    /// The device memory the program takes beside its arrays ([`SPARE`], [`SPARE_A_LAUNCH`]).
    pub spare: usize,
    /// The matrices a launch streams to the device when they are in host memory, each with its arrays' bytes in a slot
    /// (a Q8_0 matrix's levels and scales, any other format's rows), each at [`POOL_ALIGN`].
    pub streamed: BTreeMap<String, Vec<usize>>,
    /// A staging slot: the most bytes one launch's streamed matrices take.
    pub slot: usize,
    /// The copies the graphs capture where every streamed matrix streams: an array of a matrix a launch reads, each.
    pub copies: usize,
}

impl Plan {
    /// Every array on the device: the arena, the fixed arrays and every matrix.
    pub fn whole(&self) -> usize {
        self.arena + self.fixed + self.order.iter().map(|(_, b)| b).sum::<usize>()
    }
}

/// Where the program's matrices live on a budget.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Residence {
    /// The matrices in mapped host memory.
    pub host: BTreeSet<String>,
    /// How many of the plan's order moved: the first.
    pub moved: usize,
    /// The bytes the program allocates on the device: its arena and its pool.
    pub device_bytes: usize,
    /// The bytes of its mapped host pool.
    pub host_bytes: usize,
    /// Whether a moved matrix streams: the program holds its two slots and its copies' graph memory ([`staging`]).
    pub streaming: bool,
}

/// The constant buffers every form shares, allocated once: the constants, and the carried state the decode step holds.
pub fn shared_names(forms: &[Form]) -> BTreeSet<String> {
    let decode = forms.iter().find(|f| f.rows == 1);
    forms
        .iter()
        .flat_map(|f| f.schedule.buffers.values())
        .filter(|b| match b.fill {
            Fill::Constant(_) => true,
            Fill::Zero => decode.is_some_and(|d| d.schedule.buffers.contains_key(&b.name)),
            _ => false,
        })
        .map(|b| b.name.clone())
        .collect()
}

/// The bytes a matrix kept as rows takes of its pool: a Q8_0 matrix's levels and its half scales, each an array; any
/// other format's rows as the device keeps them ([`kernels::row_bytes`]) and the bytes a staged read runs past them
/// ([`kernels::tail_bytes`]).
pub fn matrix_bytes(rows: usize, cols: usize, ty: GgmlType) -> usize {
    match ty {
        GgmlType::Q8_0 => pooled(rows * cols) + pooled(rows * (cols / QBLOCK) * 2),
        _ => pooled(rows * kernels::row_bytes(ty, cols) + kernels::tail_bytes(ty)),
    }
}

/// The arrays a matrix kept as rows uploads, each its bytes of its pool ([`matrix_bytes`] their sum).
pub fn matrix_arrays(rows: usize, cols: usize, ty: GgmlType) -> Vec<usize> {
    match ty {
        GgmlType::Q8_0 => vec![pooled(rows * cols), pooled(rows * (cols / QBLOCK) * 2)],
        _ => vec![matrix_bytes(rows, cols, ty)],
    }
}

/// The checkpoint rows a matrix uploads, whatever name and orientation declare them: their format, width and runs.
fn upload(r: &Rows) -> (u32, usize, Vec<(u64, usize)>) {
    (r.ty.id(), r.cols, r.runs.iter().map(|run| (run.offset, run.rows)).collect())
}

/// What the program of `forms` allocates, its caches `ctx` positions long: the arena, the arrays it keeps on the device,
/// its matrices in the order they leave it, and the spare its graphs' `nodes` take (`emit::captured_nodes`).
pub fn plan(forms: &[Form], ctx: usize, nodes: usize) -> Plan {
    let two = forms[0].schedule.numerics == Q8MatKind::TwoPassInt8;
    let shared = shared_names(forms);
    let mut out = Plan { arena: forms.iter().map(|f| f.arena.bytes).max().unwrap_or(0), ..Plan::default() };
    // The position, the generated run, its count and the prompt; a batch's every sequence's position, run and count.
    out.fixed = pooled(4) + pooled(ctx * 4) + pooled(4) + pooled(ctx * 4);
    if let Some(b) = forms.iter().find(|f| f.kind == FormKind::Batch) {
        out.fixed += pooled(b.rows * 4) + pooled(b.rows * ctx * 4) + pooled(b.rows * 4);
    }
    let mut matrices: BTreeMap<String, usize> = BTreeMap::new();
    let mut arrays: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut uploads: BTreeMap<(u32, usize, Vec<(u64, usize)>), String> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for f in forms {
        for buf in f.schedule.buffers.values() {
            if f.schedule.io.positions().contains(&buf.name.as_str()) || (shared.contains(&buf.name) && !seen.insert(buf.name.clone())) {
                continue;
            }
            match &buf.fill {
                Fill::Constant(Placement::Rows(r)) => match uploads.get(&upload(r)) {
                    Some(of) => {
                        out.shares.insert(buf.name.clone(), of.clone());
                    }
                    None => {
                        uploads.insert(upload(r), buf.name.clone());
                        matrices.insert(buf.name.clone(), matrix_bytes(r.rows, r.cols, r.ty));
                        arrays.insert(buf.name.clone(), matrix_arrays(r.rows, r.cols, r.ty));
                    }
                },
                Fill::Constant(Placement::Values(v)) => out.fixed += pooled(v.len() * 4),
                Fill::Alias { offset, .. } if buf.quantized && quant_in_place(*offset) => continue,
                Fill::Alias { .. } => {}
                Fill::Param | Fill::Zero | Fill::Computed => {
                    if buf.plain && f.arena.at(&buf.name, Touch::Plain).is_none() {
                        out.fixed += pooled(buf.len * buf.dtype.bytes());
                    }
                }
            }
            if buf.quantized && f.arena.at(&buf.name, Touch::Quant).is_none() {
                out.fixed += pooled(buf.len) + pooled(buf.len / QBLOCK * 4) + if two { pooled(buf.len) } else { 0 };
            }
        }
        for (i, l) in f.schedule.launches.iter().enumerate() {
            for sc in arena::scratch(l, f.schedule.numerics) {
                if sc.counter || f.arena.scratch_at(i, sc.tag).is_none() {
                    out.fixed += pooled(sc.bytes);
                }
            }
        }
    }
    out.streamed = streamed(forms, &arrays, &out.shares);
    out.order = eviction_order(forms, &matrices, &out.shares, &out.streamed).into_iter().map(|m| {
        let bytes = matrices[&m];
        (m, bytes)
    }).collect();
    out.slot = forms
        .iter()
        .flat_map(|f| f.schedule.launches.iter())
        .map(|l| read_matrices(l, &out.shares).filter_map(|m| out.streamed.get(m)).flatten().sum::<usize>())
        .max()
        .unwrap_or(0);
    out.spare = SPARE + nodes * SPARE_A_LAUNCH;
    out
}

/// The distinct matrices launch `l` reads, every part of it, each by the name its upload goes by.
pub fn read_matrices<'a>(l: &'a Launch, shares: &'a BTreeMap<String, String>) -> impl Iterator<Item = &'a str> {
    let mut seen = BTreeSet::new();
    l.parts().into_iter().flat_map(|p| p.reads()).map(move |t| shares.get(t).map_or(t, String::as_str)).filter(move |t| seen.insert(*t))
}

/// The matrices that stream when in host memory, with their arrays' bytes: read whole by some launch (not only gathered
/// from, not an expert stack) and within [`SLOT_CAP`].
fn streamed(forms: &[Form], arrays: &BTreeMap<String, Vec<usize>>, shares: &BTreeMap<String, String>) -> BTreeMap<String, Vec<usize>> {
    let mut whole: BTreeSet<&str> = BTreeSet::new();
    let mut sparse: BTreeSet<&str> = BTreeSet::new();
    for l in forms.iter().flat_map(|f| f.schedule.launches.iter().flat_map(|l| l.parts())) {
        let experts: Vec<&str> = match l {
            Launch::Moe { wg, wu, wd, .. } => vec![wg, wu, wd],
            _ => Vec::new(),
        };
        for t in l.reads() {
            let t = shares.get(t).map_or(t, String::as_str);
            if !arrays.contains_key(t) {
                continue;
            }
            if matches!(l, Launch::Gather { .. }) || experts.contains(&t) {
                sparse.insert(t);
            } else {
                whole.insert(t);
            }
        }
    }
    arrays
        .iter()
        .filter(|(m, a)| whole.contains(m.as_str()) && !sparse.contains(m.as_str()) && a.iter().sum::<usize>() <= SLOT_CAP)
        .map(|(m, a)| (m.clone(), a.clone()))
        .collect()
}

/// Where `plan`'s matrices live on `budget` bytes of device memory: as many of its order as must move to mapped host
/// memory for the rest to fit, and the bytes each side takes — the decision the program makes when it starts. Refused
/// when the arrays but the matrices outrun the budget.
pub fn place(plan: &Plan, budget: usize) -> Result<Residence, LaneError> {
    let fixed = plan.arena + plan.fixed;
    if fixed > budget {
        return Err(LaneError::Memory { needed: fixed, budget });
    }
    let down = |budget: usize| {
        let mut out = Residence { device_bytes: plan.whole(), ..Residence::default() };
        for (name, bytes) in &plan.order {
            if out.device_bytes <= budget {
                break;
            }
            out.device_bytes -= bytes;
            out.host_bytes += bytes;
            out.host.insert(name.clone());
            out.moved += 1;
        }
        out
    };
    let mut out = down(budget);
    out.streaming = out.host.iter().any(|m| plan.streamed.contains_key(m));
    Ok(out)
}

/// The device memory a program whose moved matrices stream takes beside its arrays: its two slots and its copies'
/// graph memory. On the memory it finds free it takes them out before it places; given `--device-bytes`, beside them.
pub fn staging(plan: &Plan) -> usize {
    2 * plan.slot + plan.copies * SPARE_A_LAUNCH
}

/// The matrices in the order they leave the device: those only gathered from, then the expert stacks, then the rest,
/// then those read whole that are too wide to stream — each class from the decode step's last read back. A read of a
/// matrix that `shares` another's upload reads that one.
fn eviction_order(forms: &[Form], matrices: &BTreeMap<String, usize>, shares: &BTreeMap<String, String>, streamed: &BTreeMap<String, Vec<usize>>) -> Vec<String> {
    let decode = forms.iter().find(|f| f.rows == 1).expect("a lane has a decode step");
    let mut class: BTreeMap<&str, u8> = BTreeMap::new();
    let mut last: BTreeMap<&str, usize> = BTreeMap::new();
    for f in forms {
        for l in f.schedule.launches.iter().flat_map(|l| l.parts()) {
            let experts: Vec<&str> = match l {
                Launch::Moe { wg, wu, wd, .. } => vec![wg, wu, wd],
                _ => Vec::new(),
            };
            for t in l.reads() {
                let c = if matches!(l, Launch::Gather { .. }) {
                    0
                } else if experts.contains(&t) {
                    1
                } else {
                    2
                };
                let t = shares.get(t).map_or(t, String::as_str);
                if matrices.contains_key(t) {
                    let e = class.entry(t).or_insert(c);
                    *e = (*e).max(c);
                }
            }
        }
    }
    for (i, l) in decode.schedule.launches.iter().enumerate() {
        for t in l.reads() {
            let t = shares.get(t).map_or(t, String::as_str);
            if matrices.contains_key(t) {
                last.insert(t, i);
            }
        }
    }
    let mut order: Vec<&String> = matrices.keys().collect();
    // A matrix read whole that cannot stream (past a slot) leaves after every one that can.
    let class_of = |m: &str| match class.get(m).copied().unwrap_or(2) {
        2 if !streamed.contains_key(m) && matrices[m] > SLOT_CAP => 3,
        c => c,
    };
    order.sort_by_key(|m| (class_of(m.as_str()), std::cmp::Reverse(last.get(m.as_str()).copied().unwrap_or(0)), m.as_str()));
    order.into_iter().cloned().collect()
}
