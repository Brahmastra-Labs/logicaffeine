//! Where a lane's arrays live. Every array the program allocates goes on the device while they fit the memory it is
//! given; past it, constant matrices move to pinned host memory the device maps, in the order that costs a step least:
//! a table only ever gathered from (a row a token), then each expert stack from the last layer's back (a step reads its
//! chosen experts' rows), then the other matrices from the last read back — until the rest fits. A kernel reads a mapped
//! matrix through the same pointer it reads one in device memory, so where a matrix lives changes no value. Matrices
//! that read the same checkpoint rows (a tied head and the embedding table it is) share one upload.
//!
//! The compiler fixes the order ([`plan`]); the program decides how far down it to go when it starts, from the device
//! memory free once its context is up (or the `--device-bytes` it is given), less its [`Plan::spare`] — [`place`] is
//! that decision, as the program makes it. The arena is an allocation of its own; every other array the program keeps
//! on the device is carved from one pool, and every matrix it moves from one mapped host pool, each at [`POOL_ALIGN`],
//! so the bytes counted are the bytes the device gives up.

use std::collections::{BTreeMap, BTreeSet};

use super::arena;
use super::plan::{quant_in_place, Fill, Launch, Touch, QBLOCK};
use super::weights::{Placement, Rows};
use super::{Form, FormKind, LaneError};
use crate::codegen_cuda::qwen_emit::Q8MatKind;
use crate::model::quant::GgmlType;

/// The device memory a chip loses before the program allocates: what the driver holds back from every program (379 MiB
/// of the RTX 3060's 12 GiB under driver 580) and the program's context with its modules loaded (113 MiB).
pub const CONTEXT_RESERVE: usize = 512 << 20;

/// What the program's graphs, streams and events, and the rounding of its three allocations to the driver's 2 MiB
/// granule, take of the device beside its arrays, past [`SPARE_A_LAUNCH`] for each launch it captures.
pub const SPARE: usize = 32 << 20;

/// The device memory each captured launch's graph node takes, at twice the 8 KiB a node of a 1000-launch graph takes
/// on the RTX 3060.
pub const SPARE_A_LAUNCH: usize = 16 << 10;

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
/// other format's blocks as the checkpoint lays them out and the 32 bytes a staged read runs past them.
pub fn matrix_bytes(rows: usize, cols: usize, ty: GgmlType) -> usize {
    match ty {
        GgmlType::Q8_0 => pooled(rows * cols) + pooled(rows * (cols / QBLOCK) * 2),
        _ => pooled(rows * ty.bytes_for(cols) + 32),
    }
}

/// The checkpoint rows a matrix uploads, whatever name and orientation declare them: their format, width and runs.
fn upload(r: &Rows) -> (u32, usize, Vec<(u64, usize)>) {
    (r.ty.id(), r.cols, r.runs.iter().map(|run| (run.offset, run.rows)).collect())
}

/// What the program of `forms` allocates, its caches `ctx` positions long: the arena, the arrays it keeps on the device,
/// its matrices in the order they leave it, and the spare its graphs take.
pub fn plan(forms: &[Form], ctx: usize) -> Plan {
    let two = forms[0].schedule.numerics == Q8MatKind::TwoPassInt8;
    let shared = shared_names(forms);
    let mut out = Plan { arena: forms.iter().map(|f| f.arena.bytes).max().unwrap_or(0), ..Plan::default() };
    // The position, the generated run, its count and the prompt; a batch's every sequence's position, run and count.
    out.fixed = pooled(4) + pooled(ctx * 4) + pooled(4) + pooled(ctx * 4);
    if let Some(b) = forms.iter().find(|f| f.kind == FormKind::Batch) {
        out.fixed += pooled(b.rows * 4) + pooled(b.rows * ctx * 4) + pooled(b.rows * 4);
    }
    let mut matrices: BTreeMap<String, usize> = BTreeMap::new();
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
    out.order = eviction_order(forms, &matrices, &out.shares).into_iter().map(|m| {
        let bytes = matrices[&m];
        (m, bytes)
    }).collect();
    out.spare = SPARE + forms.iter().map(|f| f.schedule.launches.len()).sum::<usize>() * SPARE_A_LAUNCH;
    out
}

/// Where `plan`'s matrices live on `budget` bytes of device memory: as many of its order as must move to mapped host
/// memory for the rest to fit, and the bytes each side takes — the decision the program makes when it starts. Refused
/// when the arrays but the matrices outrun the budget.
pub fn place(plan: &Plan, budget: usize) -> Result<Residence, LaneError> {
    let fixed = plan.arena + plan.fixed;
    if fixed > budget {
        return Err(LaneError::Memory { needed: fixed, budget });
    }
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
    Ok(out)
}

/// The matrices in the order they leave the device: those only gathered from, then the expert stacks, then the rest —
/// each class from the decode step's last read back. A read of a matrix that `shares` another's upload reads that one.
fn eviction_order(forms: &[Form], matrices: &BTreeMap<String, usize>, shares: &BTreeMap<String, String>) -> Vec<String> {
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
    order.sort_by_key(|m| (class.get(m.as_str()).copied().unwrap_or(2), std::cmp::Reverse(last.get(m.as_str()).copied().unwrap_or(0)), m.as_str()));
    order.into_iter().cloned().collect()
}
