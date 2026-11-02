//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! TENSOR PARALLELISM FOR A WHOLE PROGRAM — Megatron over RDUs, as ONE spatial program.
//!
//! `passes::shard` splits ONE matmul's contraction over chips and sums the partials. A model layer is a chain of
//! ops, and Megatron splits the chain, not an op: the projections into a layer are split by COLUMN (each chip owns
//! whole heads, whole FFN columns — its outputs are complete slices), the projections out of it by ROW (each chip
//! holds a PARTIAL of the full output), and the partials are summed by an all-reduce, twice a layer. Between the
//! all-reduces every chip works on its own heads alone; the residuals and norms are the replicated spine.
//!
//! ★ THE DECOMPOSITION IS A PROGRAM, SO IT IS WRITTEN AS ONE. A generator (`qwen38::gen_qwen38_tp_decode_*`) writes
//! the tensor-parallel layer as ordinary LOGOS: every chip's slice spelled with its own tensors (`Wq3_c1`), and
//! every all-reduce as the left fold of `plus` it means. That source RUNS on the VM, so the decomposition itself is
//! checked against the unsharded layer before the compiler ever sees it. Alongside it comes a [`TpManifest`]: the
//! chip of every chip-local tensor, the slice of the full checkpoint tensor each one is (the host's loading map), and
//! each collective's partials and fold statements.
//!
//! ★ THE COMPILER THEN LOWERS IT AS IT LOWERS ANYTHING, with two differences. Every section is RELOCATED to the chip
//! of what it computes ([`relocate`]); and a collective's fold statements are not lowered as additions on one chip —
//! ONE [`allreduce_section`] replaces them, reading each chip's partial where it lives and folding them all. The
//! certificate's shard obligation holds the result to the rule that makes the split mean anything: data derived from
//! a chip's own tensors reaches the replicated spine only through an all-reduce over EVERY chip.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::ir::*;
use super::passes::from_qwen_ops::elem_precision;
use super::passes::from_tensor_op::{
    elementwise_int, ident_map, items_per_tile_grid, space, tile_grid_steps, Template,
};
use super::passes::shard::{crossing_handshake, crossing_kind, ShardError};
use super::passes::SpatialReject;
use super::pipeline::{run_passes, run_passes_uncertified, SectionCertification};
use super::target::SpatialTarget;
use crate::codegen_sva::fabric_target::Precision;

/// Which axis of a full tensor a chip's slice cuts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum SliceAxis {
    Rows,
    Cols,
}

/// A chip's tensor as ranges of a full tensor `of` (`of_shape` whole) along one axis, concatenated in order: each
/// `(start, len)` is `len` rows (or columns) from `start`. What a host loads onto the chip, and how a test fills the
/// slice with the full tensor's values.
///
/// ★ ONE RANGE IS THE COMMON CASE, NOT THE RULE. A chip's query heads are one run of columns; its Gated-DeltaNet
/// value heads are not — this checkpoint TILES its keys across the value heads (value head `h` reads key head
/// `h mod HK`), so the value heads that read a chip's key heads recur every `HK` heads, one block per recurrence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TpSlice {
    pub of: String,
    pub of_shape: (usize, usize),
    pub axis: SliceAxis,
    pub ranges: Vec<(usize, usize)>,
}

impl TpSlice {
    /// The slice's extent along its axis.
    pub fn len(&self) -> usize {
        self.ranges.iter().map(|(_, n)| n).sum()
    }

    /// The full tensor's coordinate along the axis of the slice's coordinate `x`.
    fn full_coord(&self, x: usize) -> usize {
        let mut at = x;
        for (start, n) in &self.ranges {
            if at < *n {
                return start + at;
            }
            at -= n;
        }
        panic!(
            "coordinate {x} is past the slice's {} of `{}`",
            self.len(),
            self.of
        )
    }

    /// The full tensor's flat index of the slice's element `(r, c)`.
    pub fn full_index(&self, r: usize, c: usize) -> usize {
        match self.axis {
            SliceAxis::Rows => self.full_coord(r) * self.of_shape.1 + c,
            SliceAxis::Cols => r * self.of_shape.1 + self.full_coord(c),
        }
    }

    /// The slice's own `(rows, cols)`.
    pub fn shape(&self) -> (usize, usize) {
        match self.axis {
            SliceAxis::Rows => (self.len(), self.of_shape.1),
            SliceAxis::Cols => (self.of_shape.0, self.len()),
        }
    }
}

/// One all-reduce: `out = Σ partials`, the partials in chip order (`partials[c]` lives on chip `c`), spelled in the
/// source as the fold statements `folds` — the last of which binds `out`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TpCollective {
    pub out: String,
    pub partials: Vec<String>,
    pub folds: Vec<String>,
}

/// How a tensor-parallel program is spread over `chips` RDUs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct TpManifest {
    pub chips: usize,
    /// Every chip-local tensor, by name. A tensor not named here is replicated: the spine every chip shares.
    pub chip_of: BTreeMap<String, u16>,
    /// Every chip-local tensor that is a slice of a full checkpoint (or carried-state) tensor.
    pub slices: BTreeMap<String, TpSlice>,
    pub collectives: Vec<TpCollective>,
}

impl TpManifest {
    pub fn new(chips: usize) -> Self {
        TpManifest {
            chips,
            ..Default::default()
        }
    }

    /// Record `name` as chip `chip`'s tensor, a slice of `slice.of` when given.
    pub fn place(&mut self, name: &str, chip: usize, slice: Option<TpSlice>) {
        self.chip_of.insert(name.to_string(), chip as u16);
        if let Some(s) = slice {
            self.slices.insert(name.to_string(), s);
        }
    }

    /// The collective whose fold statements include `binder`, if any.
    pub fn collective_folding(&self, binder: &str) -> Option<&TpCollective> {
        self.collectives
            .iter()
            .find(|c| c.folds.iter().any(|f| f == binder))
    }

    /// The chip a statement binding `name` runs on: its tensor's chip, or the root (chip 0) for the replicated spine.
    pub fn chip_running(&self, name: &str) -> u16 {
        self.chip_of.get(name).copied().unwrap_or(0)
    }
}

/// What a chip allows of a tensor-parallel program: what `base` allows, less a fused op over more than one chip.
///
/// ★ A FUSED OP RUNS ON ONE CHIP. Every statement of the program runs where the manifest puts what it binds — a chip's
/// own tensor on that chip, the replicated spine on every chip — and a section is relocated whole to one chip
/// ([`relocate`]). So a fusion whose members bind tensors of different chips, or a chip's tensor and the spine's, would
/// move a chip's work onto another chip; and a fusion holding a collective's fold would lower that fold as a sum on the
/// spine where the program means an all-reduce over every chip. Both are refused here, by the chip the optimizer asks
/// before it fuses; every other fusion is `base`'s to answer.
pub struct TensorParallelShim<'a> {
    pub base: &'a dyn crate::tensor_graph::shim::Shim,
    pub manifest: &'a TpManifest,
}

impl crate::tensor_graph::shim::Shim for TensorParallelShim<'_> {
    fn name(&self) -> String {
        format!("{}:tp{}", self.base.name(), self.manifest.chips)
    }

    fn supports(&self, class: crate::tensor_graph::symbols::ClassId) -> crate::tensor_graph::shim::Support {
        self.base.supports(class)
    }

    fn fits(&self, op: &crate::tensor_graph::graph::Op, g: &crate::tensor_graph::graph::TensorGraph) -> Result<(), String> {
        if matches!(op.sym, crate::tensor_graph::graph::Sym::Fused(_)) {
            let members = crate::tensor_graph::fuse::leaves(op.clone());
            if let Some(m) = members.iter().find(|m| self.manifest.collective_folding(&m.out).is_some()) {
                return Err(format!("`{}` folds a collective, which lowers as its all-reduce", m.out));
            }
            let chip = |m: &crate::tensor_graph::graph::Op| self.manifest.chip_of.get(&m.out).copied();
            if let Some(m) = members.iter().find(|m| chip(m) != chip(&members[0])) {
                return Err(format!("`{}` and `{}` run on different chips", members[0].out, m.out));
            }
        }
        self.base.fits(op, g)
    }

    fn activation(
        &self,
        consumer: &crate::tensor_graph::graph::Op,
        operand: usize,
        g: &crate::tensor_graph::graph::TensorGraph,
    ) -> Option<crate::tensor_graph::graph::QuantFormat> {
        self.base.activation(consumer, operand, g)
    }

    fn rules(&self) -> Vec<(crate::tensor_graph::shim::Position, crate::tensor_graph::fuse::ChipRule<'_>)> {
        self.base.rules()
    }

    fn numerics(&self) -> Option<crate::tensor_graph::numerics::Contract> {
        self.base.numerics()
    }
}

/// A tensor one chip's statement reads and another chip's statement writes: it crosses from the writer's chip to the
/// reader's, once, before the reader's first statement that reads it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ChipTransfer {
    pub tensor: String,
    pub from: u16,
    pub to: u16,
}

/// The tensors that cross between `m`'s chips when each statement runs on [`TpManifest::chip_running`] its binder: for
/// each statement, every operand the latest earlier statement to write it wrote on another chip, in program order, each
/// `(tensor, reader's chip)` once. An operand no statement writes — a table, a host-written input — crosses nothing:
/// each chip that reads it holds its own.
pub fn transfers(stages: &[super::passes::recognize_stages::RecognizedStage], m: &TpManifest) -> Vec<ChipTransfer> {
    let mut writer: BTreeMap<&str, u16> = BTreeMap::new();
    let mut out: Vec<ChipTransfer> = Vec::new();
    for s in stages {
        let here = m.chip_running(&s.out);
        for name in &s.ins {
            let Some(&from) = writer.get(name.as_str()) else {
                continue;
            };
            if from != here && !out.iter().any(|t| t.tensor == *name && t.to == here) {
                out.push(ChipTransfer {
                    tensor: name.clone(),
                    from,
                    to: here,
                });
            }
        }
        writer.insert(&s.out, here);
    }
    out
}

/// Where chip `chip`'s program `p` halts for `transfers`: each tensor it sends, after the last section that stores it;
/// each it receives, before the first section that loads it — `None` for the first section, the host's to put there
/// before the chip starts. Halts at one boundary are one [`LinkPoint`]; they come in program order.
pub fn link_points(p: &SpatialProgram, chip: u16, transfers: &[ChipTransfer]) -> Result<Vec<LinkPoint>, TpReject> {
    let touch = |name: &str| -> (Vec<usize>, Vec<usize>, Option<TensorId>) {
        let (mut stores, mut loads) = (Vec::new(), Vec::new());
        for (k, sec) in p.sections.iter().enumerate() {
            for st in &p.stages[sec.stages.0..sec.stages.1] {
                match &st.kind {
                    StageKind::Store { tensor, .. } if p.tensor(*tensor).name == name => stores.push(k),
                    StageKind::Load { tensor, .. } if p.tensor(*tensor).name == name => loads.push(k),
                    _ => {}
                }
            }
        }
        (stores, loads, p.tensors.iter().find(|t| t.name == name).map(|t| t.id))
    };
    let unlinked = |t: &ChipTransfer, why: &'static str| TpReject::Unlinked {
        tensor: t.tensor.clone(),
        chip,
        why,
    };
    fn point(at: &mut BTreeMap<Option<usize>, LinkPoint>, after: Option<usize>) -> &mut LinkPoint {
        at.entry(after).or_insert(LinkPoint {
            after,
            sends: Vec::new(),
            receives: Vec::new(),
        })
    }
    let mut at: BTreeMap<Option<usize>, LinkPoint> = BTreeMap::new();
    for t in transfers {
        if t.from == chip {
            let (stores, _, id) = touch(&t.tensor);
            let (Some(last), Some(id)) = (stores.last(), id) else {
                return Err(unlinked(t, "it sends a tensor no section of it stores"));
            };
            point(&mut at, Some(*last)).sends.push(id);
        }
        if t.to == chip {
            let (stores, loads, id) = touch(&t.tensor);
            if !stores.is_empty() {
                return Err(unlinked(t, "it receives a tensor a section of its own stores"));
            }
            let (Some(first), Some(id)) = (loads.first(), id) else {
                return Err(unlinked(t, "it receives a tensor no section of it loads"));
            };
            point(&mut at, first.checked_sub(1)).receives.push(id);
        }
    }
    Ok(at.into_values().collect())
}

/// One step of the relay a host runs between chips' programs that halt at their [`LinkPoint`]s.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum RelayStep {
    /// Start chip `chip`, or continue it past the halt it stands at, and let it run to its halt `to` — to its end when `to`
    /// is its count of halts. The host waits on a chip only when it next copies from it or runs it.
    Run { chip: u16, to: usize },
    /// Read `tensor` from chip `from`, which stands at or past the halt that sends it, and write it into chip `to`.
    Copy { tensor: String, from: u16, to: u16 },
}

/// The host's relay over `chips`' halts: round by round, each chip that can is run to its next halt — once the tensors
/// the halt it stands at receives have been sent and copied — so chips whose next work needs nothing of each other run
/// at once. Refused, with where each chip stands, when no chip can move before all have finished.
pub fn relay_schedule(chips: &[SpatialProgram], transfers: &[ChipTransfer]) -> Result<Vec<RelayStep>, String> {
    let named = |p: &SpatialProgram, ids: &[TensorId]| -> Vec<String> { ids.iter().map(|t| p.tensor(*t).name.clone()).collect() };
    let mut at: Vec<Option<usize>> = vec![None; chips.len()];
    let mut sent: BTreeSet<String> = BTreeSet::new();
    let mut copied: BTreeSet<(String, u16)> = BTreeSet::new();
    let mut steps = Vec::new();
    while at.iter().zip(chips).any(|(a, p)| *a != Some(p.links.len())) {
        let mut moved = false;
        for (c, p) in chips.iter().enumerate() {
            if at[c] == Some(p.links.len()) {
                continue;
            }
            let wants: Vec<String> = at[c].map(|i| named(p, &p.links[i].receives)).unwrap_or_default();
            if !wants.iter().all(|t| sent.contains(t) || copied.contains(&(t.clone(), c as u16))) {
                continue;
            }
            for t in wants {
                if copied.insert((t.clone(), c as u16)) {
                    let from = transfers
                        .iter()
                        .find(|x| x.tensor == t && x.to == c as u16)
                        .map(|x| x.from)
                        .ok_or_else(|| format!("chip {c} receives {t}, which no transfer brings it"))?;
                    steps.push(RelayStep::Copy { tensor: t, from, to: c as u16 });
                }
            }
            let next = at[c].map_or(0, |i| i + 1);
            steps.push(RelayStep::Run { chip: c as u16, to: next });
            if let Some(l) = p.links.get(next) {
                sent.extend(named(p, &l.sends));
            }
            at[c] = Some(next);
            moved = true;
        }
        if !moved {
            return Err(format!("no chip can move: each stands at {at:?} of {:?} halts, with {sent:?} sent", chips.iter().map(|p| p.links.len()).collect::<Vec<_>>()));
        }
    }
    Ok(steps)
}

/// Why a tensor-parallel lowering refused.
#[derive(Clone, Debug, PartialEq)]
pub enum TpReject {
    /// A section that multicasts: its groups are rectangles of `(x, y)`, which cannot name a chip.
    Multicast {
        program: String,
    },
    /// No hand-off on the target crosses a chip boundary.
    NoCrossing(ShardError),
    Passes(SpatialReject),
    /// A chip's program cannot halt for a transfer: it sends a tensor no section of it stores, or receives one no section
    /// of it loads, or one a section of its own stores.
    Unlinked {
        tensor: String,
        chip: u16,
        why: &'static str,
    },
}

impl From<SpatialReject> for TpReject {
    fn from(e: SpatialReject) -> Self {
        TpReject::Passes(e)
    }
}

fn on_chip(s: &mut Site, chip: u16) {
    s.chip = chip;
}

/// A scheduled one-chip section of the statement binding `binder`, moved onto the chip it runs on: every site it
/// names — its stages', buffers', placement's, memory map's, semaphores', engine programs' and the remote semaphore
/// posts in them — and every tensor it touches marked with its chip: the one `m` gives it, or, for a tensor the
/// section itself stores on behalf of a chip-local statement (a blocked op's intermediate), that chip; the
/// replicated spine stays unmarked. The section computes exactly what it did; it now does so on another RDU. Its
/// certificate is dropped with the schedule digest it was bound to.
pub fn relocate(
    p: &SpatialProgram,
    binder: &str,
    m: &TpManifest,
) -> Result<SpatialProgram, TpReject> {
    let chip = m.chip_running(binder);
    let chip_local = m.chip_of.contains_key(binder);
    let mut q = p.clone();
    if q.placement.as_ref().is_some_and(|pl| !pl.groups.is_empty()) {
        return Err(TpReject::Multicast {
            program: q.name.clone(),
        });
    }
    super::passes::pack::for_each_site_mut(&mut q, &mut |s| s.chip = chip);
    let stored: BTreeSet<TensorId> = q
        .stages
        .iter()
        .filter_map(|s| match &s.kind {
            StageKind::Store { tensor, .. } => Some(*tensor),
            _ => None,
        })
        .collect();
    // A weight read as it is stored Q8_0 is its values and scales: each lives where the chip's slice of the weight does.
    let chip_of = |name: &str| {
        m.chip_of.get(name).copied().or_else(|| {
            super::passes::from_q8::stored_part(name).and_then(|(w, _)| m.chip_of.get(w).copied())
        })
    };
    for t in &mut q.tensors {
        t.chip = chip_of(&t.name)
            .or((chip_local && stored.contains(&t.id)).then_some(chip))
            .map(ChipId);
    }
    q.certificate = None;
    Ok(q)
}

/// ONE all-reduce as a section: `out = Σ_c partials[c]` over a `rows × cols` tensor, tile by tile — chip `c`'s
/// partial Loaded on chip `c` (where it lives) and handed across the chip boundary, all of them folded by an
/// `AllReduce` on chip 0, the sum stored as the replicated `out`.
///
/// ★ BUILT AS A SUM, THEN SPREAD. The fold is lowered first as what it is on one chip — a `chips`-ary elementwise
/// sum, tiled, placed and scheduled by the pinned passes — and then each operand's Load moves to its chip, its
/// hand-off becomes one that crosses a chip boundary, and the fold becomes the `AllReduce` whose combine it already
/// is. The passes run again from the placement, so the collective is sized and choreographed like any other hand-off,
/// and certified as `certification` says: on its own, or as a section of the composed program. The one-chip sum is
/// only scheduled — its certificate would be dropped with the placement it was bound to.
///
/// ★ AN INTEGER SUM IS EXACT. Float partials (`floats`) are summed in the target's element precision, an approximation
/// whose order the claim names. Integer partials — an int8 model's accumulators — are summed in the widest integer the
/// elementwise datapath holds, exactly, in any order, and their storage is narrowed by `narrow` as every integer
/// section's is before it is tiled.
pub fn allreduce_section(
    col: &TpCollective,
    shape: (usize, usize),
    floats: bool,
    target: &SpatialTarget,
    certification: SectionCertification,
    narrow: &mut dyn FnMut(SpatialProgram) -> SpatialProgram,
) -> Result<SpatialProgram, TpReject> {
    let chips = col.partials.len();
    let (rows, cols) = shape;
    let dt: Precision = if floats {
        elem_precision(target)?
    } else {
        elementwise_int(target).ok_or(TpReject::Passes(SpatialReject::MissingCompute {
            needed: "an integer elementwise datapath",
            target: target.name,
        }))?
    };
    let mut t = Template::new(target)?;
    let parts: Vec<TensorId> = col
        .partials
        .iter()
        .map(|n| t.tensor(n, TensorRole::Input, rows, cols, dt, TensorInit::External))
        .collect();
    let out = t.tensor(
        &col.out,
        TensorRole::Output,
        rows,
        cols,
        dt,
        TensorInit::Zero,
    );
    let (rows_t, cols_t) = (
        rows.div_ceil(target.tile.rows),
        cols.div_ceil(target.tile.cols),
    );
    let steps = tile_grid_steps(rows_t, cols_t);
    let ins: Vec<BufferId> = (0..chips)
        .map(|c| t.buffer(&format!("partial_{c}"), 1, dt))
        .collect();
    let loads: Vec<StageId> = (0..chips)
        .map(|c| {
            t.load(
                &format!("load_{}", col.partials[c]),
                parts[c],
                steps.clone(),
                1,
                ins[c],
            )
        })
        .collect();
    let summed = t.buffer("sum", 1, dt);
    let w = target.tile.cols;
    let combine = ComputeOp::Generic {
        iters: space(target, w, None),
        ins: vec![ident_map(w); chips],
        out: ident_map(w),
        body: (1..chips).fold(TensorBody::Operand(0), |acc, i| {
            TensorBody::Add(Box::new(acc), Box::new(TensorBody::Operand(i)))
        }),
    };
    let fold = t.stage(
        &format!("allreduce_{}", col.out),
        StageKind::Compute(combine),
        ins.clone(),
        vec![summed],
    );
    for (l, b) in loads.iter().zip(&ins) {
        t.edge(*l, *b, fold);
    }
    let st = t.store(&format!("store_{}", col.out), out, steps, 1, summed);
    t.edge(fold, summed, st);
    let (acc, claim) = if floats {
        (
            AccPolicy::F32,
            NumericsClaim::Approx {
                reason: format!("all-reduce of {chips} partials in {dt:?}"),
            },
        )
    } else {
        (AccPolicy::I64, NumericsClaim::Exact)
    };
    let proto = run_passes_uncertified(
        narrow(t.finish(
            format!("allreduce_{}_{chips}_chips", col.out),
            items_per_tile_grid(rows_t, cols_t),
            acc,
            claim,
            target,
        )?),
        target,
    )?;

    // ── Spread: each operand's Load, and the buffer it fills, onto its chip.
    let crossing = crossing_kind(target).map_err(TpReject::NoCrossing)?;
    let mut q = proto;
    let chip_of_tensor: BTreeMap<TensorId, u16> = parts
        .iter()
        .enumerate()
        .map(|(c, t)| (*t, c as u16))
        .collect();
    let mut chip_of_buffer: BTreeMap<BufferId, u16> = BTreeMap::new();
    for s in &mut q.stages {
        match &s.kind {
            StageKind::Load { tensor, .. } if chip_of_tensor.contains_key(tensor) => {
                let c = chip_of_tensor[tensor];
                if let Some(x) = &mut s.site {
                    on_chip(x, c);
                }
                chip_of_buffer.extend(s.outs.iter().map(|b| (*b, c)));
            }
            StageKind::Compute(op @ ComputeOp::Generic { .. }) => {
                s.kind = StageKind::AllReduce {
                    pool: ReducePool::Sum,
                    tensor: out,
                    combine: Box::new(op.clone()),
                };
            }
            _ => {}
        }
    }
    // ★ A PARTIAL THAT LEAVES ITS CHIP IS NOT A LOCAL HAND-OFF. A `Fifo` is held to one producing and one consuming
    // edge on its own site, and an edge that crosses a chip is neither — the receiver owns what lands. `Interm` is the
    // IR's kind for a partial sum, and carries no local pairing (as `passes::shard` spells its partials).
    for b in &mut q.buffers {
        if let (Some(c), Some(x)) = (chip_of_buffer.get(&b.id), &mut b.site) {
            on_chip(x, *c);
            if let (
                true,
                BufferKind::Fifo {
                    depth_pages,
                    page_tiles,
                },
            ) = (*c != 0, b.kind)
            {
                b.kind = BufferKind::Interm {
                    depth_pages,
                    page_tiles,
                };
            }
        }
    }
    for e in &mut q.edges {
        if chip_of_buffer.get(&e.buffer).is_some_and(|c| *c != 0) {
            e.handshake = crossing_handshake(crossing);
        }
    }
    if let Some(pl) = &mut q.placement {
        for (i, s) in q.stages.iter().enumerate() {
            if let Some(x) = s.site {
                pl.site_of[i] = x;
                pl.engine_of[i].site = x;
            }
        }
        let mut hosting: BTreeSet<(Site, Vec<WorkItemId>)> = BTreeSet::new();
        for s in q
            .stages
            .iter()
            .filter_map(|s| s.site)
            .filter(|x| x.chip != 0)
        {
            let home = Site { chip: 0, ..s };
            hosting.insert((s, pl.items_at(home).to_vec()));
        }
        pl.items_of_site.extend(hosting);
    }
    for t in &mut q.tensors {
        t.chip = chip_of_tensor.get(&t.id).map(|c| ChipId(*c));
    }
    q.requires.handshakes.insert(crossing);
    q.semaphores.clear();
    q.memory = None;
    q.schedule = None;
    q.certificate = None;
    Ok(match certification {
        SectionCertification::Composed => run_passes_uncertified(q, target)?,
        SectionCertification::EachSection => run_passes(q, target)?,
    })
}

/// The phases one chip already holds, as a pointer forest: a held phase points past itself, and a lookup follows the
/// pointers to the first free phase and points everything it passed straight at it — amortized constant time, however
/// many phases the chip holds.
#[derive(Default)]
struct FreePhases {
    /// `next[x] == x` for a free phase; every phase at or past the end is free.
    next: Vec<usize>,
}

impl FreePhases {
    /// The first free phase at or after `x`.
    fn first_free(&mut self, x: usize) -> usize {
        let mut root = x;
        while root < self.next.len() && self.next[root] != root {
            root = self.next[root];
        }
        let mut y = x;
        while y < self.next.len() && self.next[y] != y {
            let n = self.next[y];
            self.next[y] = root;
            y = n;
        }
        root
    }

    /// Hold phase `x`, which is free.
    fn take(&mut self, x: usize) {
        if self.next.len() <= x {
            let from = self.next.len();
            self.next.extend(from..=x);
        }
        self.next[x] = x + 1;
    }
}

/// What a section touches, as far as its phase is concerned: the tensors it reads and writes, by name, and its chips.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SectionTouch {
    pub reads: BTreeSet<String>,
    pub writes: BTreeSet<String>,
    pub chips: BTreeSet<u16>,
}

impl SectionTouch {
    pub fn of(p: &SpatialProgram) -> Self {
        let mut t = SectionTouch::default();
        for s in &p.stages {
            match &s.kind {
                StageKind::Load { tensor, .. } => {
                    t.reads.insert(p.tensor(*tensor).name.clone());
                }
                StageKind::Store { tensor, .. } => {
                    t.writes.insert(p.tensor(*tensor).name.clone());
                }
                _ => {}
            }
            if let Some(site) = s.site {
                t.chips.insert(site.chip);
            }
        }
        t
    }
}

/// ★ THE CHIPS RUN AT ONCE. The source writes chip 0's slice of an op, then chip 1's; composed in that order with a
/// barrier after each, chip 1 would wait out chip 0's whole slice. Two sections may share a PHASE — run with no
/// barrier between them — when they are on disjoint chips and neither touches what the other writes. See
/// [`phase_order`].
pub fn phases(sections: &[SpatialProgram]) -> (Vec<usize>, Vec<bool>) {
    phase_order(&sections.iter().map(SectionTouch::of).collect::<Vec<_>>())
}

/// Place each section, in source order, in the earliest phase after every section it depends on (a read of what an
/// earlier one writes, a write of what an earlier one reads or writes — the carried state included) where its chips
/// are free, and return the sections' order, phase by phase, with `barrier_after[k]` closing the last section of each
/// phase.
///
/// A section depends on an earlier one exactly when some name witnesses it, so the phase it must follow is the latest
/// phase any of its names was written in (or, for a name it writes, read in) — the pairwise maximum over the sections
/// before it, taken per name. The first phase from there with its chips free is read off each chip's `FreePhases`:
/// jumping to the latest of the chips' next free phases skips only phases some chip holds, so the first phase every
/// chip agrees on is the earliest free one. Both keep the placement linear in the sections, where a walk phase by
/// phase is quadratic in a batch whose independent sequences all start past the same phase.
pub fn phase_order(touches: &[SectionTouch]) -> (Vec<usize>, Vec<bool>) {
    let (mut written_in, mut read_in): (BTreeMap<&str, usize>, BTreeMap<&str, usize>) =
        (BTreeMap::new(), BTreeMap::new());
    let mut phase_of: Vec<usize> = Vec::with_capacity(touches.len());
    let mut free: BTreeMap<u16, FreePhases> = BTreeMap::new();
    for t in touches {
        let raw = t.reads.iter().filter_map(|n| written_in.get(n.as_str()));
        let war_waw = t
            .writes
            .iter()
            .flat_map(|n| [written_in.get(n.as_str()), read_in.get(n.as_str())])
            .flatten();
        let after = raw.chain(war_waw).map(|ph| ph + 1).max().unwrap_or(0);
        let mut ph = after;
        loop {
            let next = t
                .chips
                .iter()
                .map(|c| free.entry(*c).or_default().first_free(ph))
                .max()
                .unwrap_or(ph);
            if next == ph {
                break;
            }
            ph = next;
        }
        for c in &t.chips {
            free.entry(*c).or_default().take(ph);
        }
        phase_of.push(ph);
        for n in &t.reads {
            let r = read_in.entry(n.as_str()).or_insert(ph);
            *r = (*r).max(ph);
        }
        for n in &t.writes {
            let w = written_in.entry(n.as_str()).or_insert(ph);
            *w = (*w).max(ph);
        }
    }
    let mut order: Vec<usize> = (0..touches.len()).collect();
    order.sort_by_key(|&k| (phase_of[k], k));
    let barrier_after = order
        .iter()
        .enumerate()
        .map(|(pos, &k)| {
            order
                .get(pos + 1)
                .is_some_and(|&next| phase_of[next] != phase_of[k])
        })
        .collect();
    (order, barrier_after)
}
