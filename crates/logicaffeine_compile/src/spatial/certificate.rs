//! `ChoreographyCertificate` — what a verified spatial program PROVES about itself, re-checkable by a checker
//! that trusts only a ~100-line PR/RUP refutation checker, a ~40-line topological-order verifier, and integer
//! arithmetic — never the passes that produced the schedule.
//!
//! * **Liveness** — the schedule is unrolled into its event graph: one transition per executed op, one place
//!   per token (`Data{buf,k}` = the k-th page pushed, `Space{buf,k}` = the k-th reservation's slot, returned by
//!   the pop `depth` pages earlier, `Ready`/`Valid` = the k-th round of a multicast semaphore pair, `McastData`
//!   = the k-th block landing at a receiver, `Pc` = program order). Every place has one producer and one
//!   consumer (a marked graph); by Commoner's theorem the net is live iff every directed cycle carries a
//!   token, i.e. iff the token-free arcs form a DAG — the witness is a topological order of the transitions.
//!   A consumed place with neither a producer nor an initial token is a STARVED transition (a dropped
//!   increment, flag or pop) and earns no certificate.
//! * **Bounds** — per-buffer counting balance (reserve == push == pop over the run) and the evaluated
//!   high-water occupancy against the depth.
//! * **PE datapaths** — a SAT [`PlacementCertificate`] for every integer compute stage (`psum + a·w` at the
//!   8-bit bound the fabric's placer certification uses) and for ALU/select scalar bodies; float tensor
//!   datapaths are honestly `Uncertified` with the reason.
//! * **Numerics** — the exactness claim carried from the front end.

use rustc_hash::FxHashMap;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::eval::{eval_choreography_dry, eval_choreography_traced, ChoreoFault, Inputs, Value};
use super::ir::*;
use crate::codegen_sva::bitblast::{word_size, BlastRefusal};
use crate::codegen_sva::fabric_grid::{
    cells_datapath, cells_to_gates, grid_output_to_bounded, grid_output_to_gates, route_loop_body,
    unroutable_cell, BoundedBuild, TILE_OPERAND_CHANNELS,
};
use crate::codegen_sva::fabric_rtl::{AluOp, FabricOp, LoopCell, Src};
use crate::codegen_sva::fabric_target::Precision;
use crate::codegen_sva::bitblast::BLAST_BUDGET_NODES;
use crate::codegen_sva::pnr_certificate::{
    gates_obligation_within, mapping_obligation_within, refute, Obligation, PlacementCertificate, GATE_BUDGET_NODES,
};
use crate::codegen_sva::sva_to_verify::BoundedExpr;
use crate::spatial::target::precision_is_int;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PlaceKind {
    Space {
        buf: BufferId,
        k: usize,
    },
    Data {
        buf: BufferId,
        k: usize,
    },
    Ready {
        sem: SemId,
        receiver: Site,
        k: usize,
    },
    Valid {
        sem: SemId,
        receiver: Site,
        k: usize,
    },
    McastData {
        group: GroupId,
        receiver: Site,
        k: usize,
    },
    Pc {
        site: Site,
        engine: Engine,
        k: usize,
    },
    /// The `k`-th post landed at `at` on a RENDEZVOUS semaphore. A rendezvous is not a hand-off: a post is
    /// not consumed by one reader, because every engine program at the site that waits reads the same
    /// count. So this place has one producer (the post) and as many consumers as the site has waiters —
    /// one for a gather (its root), every program at the site for a release — and a wait for `c` consumes
    /// the first `c` posts at its own site. The liveness proof is the same DAG argument either way.
    ///
    /// ★ WHY NOT `Ready`. A ready counter is modelled per RECEIVER SITE and paired one round at a time
    /// with the sender's wait; a rendezvous is waited on by several engines of one site at once, and that
    /// pairing does not describe it. Posts are keyed by the site they LAND on, which is exactly the
    /// counter a wait there reads.
    Arrival {
        sem: SemId,
        at: Site,
        k: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    pub kind: PlaceKind,
    pub marking: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transition {
    pub site: Site,
    pub engine: Engine,
    pub op_index: usize,
    pub consumes: Vec<usize>,
    pub produces: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkedGraph {
    pub places: Vec<Place>,
    pub transitions: Vec<Transition>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LivenessWitness {
    pub topo_order: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferBalance {
    pub buf: BufferId,
    pub depth_pages: usize,
    pub reserved: u64,
    pub pushed: u64,
    pub popped: u64,
    pub high_water: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ComputeStageCert {
    Sat(PlacementCertificate),
    Uncertified { reason: String },
}

impl PartialEq for ComputeStageCert {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ComputeStageCert::Sat(a), ComputeStageCert::Sat(b)) => {
                a.len() == b.len() && a.num_vars() == b.num_vars()
            }
            (
                ComputeStageCert::Uncertified { reason: a },
                ComputeStageCert::Uncertified { reason: b },
            ) => a == b,
            _ => false,
        }
    }
}

/// The liveness proof of ONE section: its graph's shape, a digest that binds the witness to that exact
/// graph, and the topological order over the section's own transition numbering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionProof {
    pub section: usize,
    pub places: usize,
    pub transitions: usize,
    /// FNV-1a over the section graph's serialization — the stand-in for carrying the graph itself.
    pub digest: u64,
    /// Which entry of the proof's witness table orders this section's transitions.
    pub witness: usize,
    /// The section's graph at every count of its trips' windows ([`trip_window`]); empty when it runs to no trip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cases: Vec<TripCase>,
}

/// ★ ONE COUNT OF A TRIP, PROVED. The graph a part of the program unrolls to when each trip it runs to counts
/// `counts[i]` (its trips in id order), carried like a section's: its shape, its digest, and the witness-table entry
/// that orders it. `recheck` rebuilds every one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TripCase {
    pub counts: Vec<usize>,
    pub places: usize,
    pub transitions: usize,
    pub digest: u64,
    pub witness: usize,
}

/// HOW THE LIVENESS PROOF IS CARRIED.
///
/// ★ WHY THERE ARE TWO FORMS. The unrolled event graph of a four-layer Qwen3.8 forward is 196,211,828
/// places and 102,936,753 transitions — 35 GB, two thirds of peak, and the reason the sixty-four-layer
/// model did not fit on a 125 GB box. The graph is not irreducibly that big: it is a DISJOINT UNION of
/// one subgraph per section joined only by the program-counter chain, because on the composed forward
/// NO buffer, semaphore or multicast group is touched by two sections. Proving one section at a time
/// and keeping only the witnesses turns a 35 GB artifact into per-section orders, and peak from the sum
/// of the sections into the largest of them.
///
/// ★ WHY THE CUT IS SOUND, AND WHERE IT REFUSES. Commoner's condition asks that every directed cycle
/// carry a token. Cutting the program-counter arcs between sections only REMOVES arcs, so it can invent
/// no cycle; it could only HIDE one that runs backwards from a later section to an earlier one — and a
/// backwards arc is exactly a place produced in one section and consumed in another. `section_ranges`
/// checks for that and hands back `None` when it finds one, so a program that does not decompose is
/// proved whole, at the old cost, rather than proved wrongly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphProof {
    /// One graph for the whole choreography. The form every single-section program takes, and the
    /// fallback whenever a place crosses a section boundary. The graph itself is carried, so `recheck`
    /// compares it structurally.
    Whole {
        graph: MarkedGraph,
        liveness: LivenessWitness,
        /// The graph at every count of the program's trips' windows, ordered by `case_witnesses`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        cases: Vec<TripCase>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        case_witnesses: Vec<LivenessWitness>,
    },
    /// One proof per section, plus the totals. The graphs are rebuilt by `recheck` from the schedule and
    /// matched against the carried digests.
    ///
    /// ★ THE WITNESSES ARE A TABLE, BECAUSE A DECODER REPEATS ITSELF. Sections with the same dataflow
    /// build the same graph up to resource names, and so the same topological order. Measured on the
    /// composed Qwen3.8 forward: 117 sections carry 11 DISTINCT witnesses at 4 layers, and 232 sections
    /// carry the SAME 11 at 8 — the table is constant in depth, while one witness per section grows with
    /// it (3,923 entries at 4 layers, 7,759 at 8; the real-width certificate was 810 MB at 2 layers).
    /// Sharing is safe by construction: `recheck` still rebuilds EVERY section's own graph and verifies
    /// the witness it references against that graph, so a shared witness that does not fit a section is
    /// a refusal, never an acceptance.
    PerSection {
        sections: Vec<SectionProof>,
        witnesses: Vec<LivenessWitness>,
        places: usize,
        transitions: usize,
    },
}

impl GraphProof {
    pub fn places(&self) -> usize {
        match self {
            GraphProof::Whole { graph, .. } => graph.places.len(),
            GraphProof::PerSection { places, .. } => *places,
        }
    }
    pub fn transitions(&self) -> usize {
        match self {
            GraphProof::Whole { graph, .. } => graph.transitions.len(),
            GraphProof::PerSection { transitions, .. } => *transitions,
        }
    }
    /// The whole graph, when one is carried. `None` is not "no proof": it is the per-section form, whose
    /// graphs are rebuilt on demand by [`ChoreographyCertificate::recheck`].
    pub fn whole_graph(&self) -> Option<&MarkedGraph> {
        match self {
            GraphProof::Whole { graph, .. } => Some(graph),
            GraphProof::PerSection { .. } => None,
        }
    }
    /// The whole graph, mutably — the seam a mutation probe needs to corrupt a carried graph.
    pub fn whole_graph_mut(&mut self) -> Option<&mut MarkedGraph> {
        match self {
            GraphProof::Whole { graph, .. } => Some(graph),
            GraphProof::PerSection { .. } => None,
        }
    }
    /// Every witness this proof carries, in section order (one entry for the whole form).
    pub fn witnesses(&self) -> Vec<&LivenessWitness> {
        match self {
            GraphProof::Whole { liveness, .. } => vec![liveness],
            GraphProof::PerSection { witnesses, .. } => witnesses.iter().collect(),
        }
    }
    /// For the per-section form, the witness-table index each section uses, in section order.
    pub fn witness_of_section(&self) -> Option<Vec<usize>> {
        match self {
            GraphProof::Whole { .. } => None,
            GraphProof::PerSection { sections, .. } => {
                Some(sections.iter().map(|s| s.witness).collect())
            }
        }
    }
}

/// A choreography's certificate, whole: what [`ChoreographyCertificate::recheck`] reads, and all it reads — so it travels
/// beside its program (`certificate.json`'s `whole`) and a reader rechecks it there, re-deriving nothing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChoreographyCertificate {
    pub schedule_digest: u64,
    pub proof: GraphProof,
    pub balances: Vec<BufferBalance>,
    /// `(semaphore, increments or flag sets, wait target)`.
    pub sem_counts: Vec<(SemId, u64, u64)>,
    pub compute: Vec<(StageId, ComputeStageCert)>,
    pub numerics: NumericsClaim,
}

impl ChoreographyCertificate {
    /// Total places across the proof, whichever form it takes.
    pub fn places(&self) -> usize {
        self.proof.places()
    }
    /// Total transitions across the proof, whichever form it takes.
    pub fn transitions(&self) -> usize {
        self.proof.transitions()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CertifyFault {
    NotScheduled,
    Invalid(IrViolation),
    /// A transition consumes a place nobody produces and that carries no initial token.
    Starved {
        transition: usize,
        place: PlaceKind,
    },
    /// A token-free cycle: the ring of waits that could never all fire, each named by the op that waits
    /// and the place it waits on. A deadlock is only actionable if you can see WHICH hand-off closed the
    /// ring, so the fault carries the ring rather than an index list.
    Deadlock {
        cycle: Vec<DeadlockNode>,
    },
    Bounds {
        buffer: BufferId,
    },
    /// An ACTIVATION is not ordered: the section that writes it is not strictly before every section
    /// that reads it, or a barrier between them is missing. Without that order the reader takes
    /// whatever is in the tensor, which on a hundred-section forward is the whole answer.
    ActivationOrder {
        tensor: TensorId,
        writer: Option<usize>,
        reader: usize,
        reason: &'static str,
    },
    /// STATE is not ordered: more than one section writes it, one section both reads and writes it, or
    /// a barrier between the reader and the writer is missing. Unlike an activation a state tensor is
    /// FULL at the start of a run, so an unordered read yields last step's value instead of faulting —
    /// which is exactly the silent wrong answer this obligation exists to make impossible.
    StateOrder {
        tensor: TensorId,
        reader: Option<usize>,
        writer: Option<usize>,
        reason: &'static str,
    },
    /// A TENSOR-PARALLEL SHARD REACHES AN OUTPUT WITHOUT BEING SUMMED. Each chip of a row-parallel shard
    /// computes a partial over its slice of the contraction — correctly shaped, fully written, and
    /// missing every term the other chips hold. Nothing about that partial looks wrong: the program runs,
    /// the store lands, the run reports success, and the number is `1/chips` of the answer. This is the
    /// one obligation that cannot be recovered by inspecting the output, which is why it is checked here.
    UnreducedShard {
        tensor: TensorId,
        chips: usize,
    },
    /// A part of the program runs to trips whose windows multiply past what one proof may hold.
    TripCasesExceed {
        limit: usize,
    },
    Eval(ChoreoFault),
}

/// FNV-1a over the schedule's serialization — binds a certificate to one choreography.
pub fn schedule_digest(p: &SpatialProgram) -> u64 {
    fnv_json(&p.schedule)
}

/// FNV-1a over a value's JSON serialization, hashed as it streams: the digest of a composed program's
/// schedule never materializes the text.
fn fnv_json<T: Serialize>(v: &T) -> u64 {
    struct Fnv(u64);
    impl std::io::Write for Fnv {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            for x in buf {
                self.0 ^= u64::from(*x);
                self.0 = self.0.wrapping_mul(0x100000001b3);
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut f = Fnv(0xcbf29ce484222325);
    let _ = serde_json::to_writer(&mut f, v);
    f.0
}

/// An executed op with the loop environment it ran under.
///
/// ★ IT BORROWS. The graph only READS the choreography, and a `ChoreoOp::Compute` carries a whole
/// `ComputeOp` — its iteration space, one access map per operand, and a body tree. Copying that per op,
/// and again per unrolled round, made the CERTIFICATE cost more memory than the program it certifies:
/// two thirds of peak on a four-layer Qwen3.8 (52.6 GB, against 18.0 GB for the composed program), which
/// is what put the 64-layer model out of reach of a 125 GB box. The schedule outlives the graph, so a
/// reference is all this ever needed.
#[derive(Clone, Debug)]
struct Event<'a> {
    op: &'a ChoreoOp,
}

/// The item-loop window the event graph unrolls. Work items are rounds of one choreography whose state
/// (FIFO occupancy, semaphore rounds) carries over with a dependence distance bounded by the deepest FIFO;
/// the unrolled graph is periodic beyond that, so a token-free cycle exists in the full unrolling iff it
/// exists within `deepest FIFO + 2` consecutive rounds. Every engine of a site (and every member of a
/// multicast group) loops the same count, so the cap keeps producer/consumer pairings consistent.
pub fn item_window(p: &SpatialProgram) -> usize {
    p.buffers
        .iter()
        .map(|b| b.kind.depth_pages())
        .max()
        .unwrap_or(1)
        + 2
}

/// Unroll `ops` into events, each loop run to a trip counted `counts[trip]`.
fn unroll<'a>(ops: &'a [TaggedOp], out: &mut Vec<Event<'a>>, window: usize, counts: &[usize]) {
    for t in ops {
        match &t.op {
            ChoreoOp::Loop { var, count, body } => {
                // ★ ONLY THE ITEM LOOP MAY BE CAPPED, AND THIS WAS MEASURED THE HARD WAY.
                //
                // Fully unrolling counted loops is what makes the graph enormous — 196,211,828 places
                // and 102,936,753 transitions for a FOUR-layer Qwen3.8, 35 GB and two thirds of peak,
                // which is why the 64-layer model does not fit. Capping every loop at the same window
                // looks like the same periodicity argument, and it is NOT: with the K-block loop
                // truncated, 39 of 100 statements that certify today stop certifying. The item loop is
                // special because every engine of a site loops the SAME count, so truncation keeps
                // producer/consumer pairings consistent; a K-block loop's balance lives INSIDE the loop
                // and truncation breaks it. A smaller graph has to come from somewhere else.
                let n = if *var == crate::spatial::passes::LV_ITEM {
                    count.at(counts).min(window)
                } else {
                    count.at(counts)
                };
                // The body is walked in place; `body_ops` would copy it once per round.
                for _ in 0..n {
                    unroll(body, out, window, counts);
                }
            }
            other => out.push(Event { op: other }),
        }
    }
}

/// The graph under construction. A place is interned by its kind — the first request creates it, every later one
/// finds it — and the lookup is hashed: a layer's graphs hold millions of places, each requested by its producer and
/// again by its consumer.
struct Builder {
    places: Vec<Place>,
    index: FxHashMap<PlaceKind, usize>,
    transitions: Vec<Transition>,
}

impl Builder {
    fn place(&mut self, kind: PlaceKind, marking: usize) -> usize {
        if let Some(i) = self.index.get(&kind) {
            return *i;
        }
        let i = self.places.len();
        self.places.push(Place { kind, marking });
        self.index.insert(kind, i);
        i
    }
}

/// Build the unrolled event graph of the whole schedule, every trip at its greatest count.
pub fn build_marked_graph(p: &SpatialProgram) -> Result<MarkedGraph, CertifyFault> {
    build_graph_over(p, &GraphContext::of(p), None, &p.bound_counts())
}

/// A section's share of the schedule: each program that holds ops of the section, with the half-open range of its
/// top-level ops that do, in program order.
type Selection = Vec<(usize, std::ops::Range<usize>)>;

/// What every graph of one program reads, derived once rather than once per section graph — each is linear in the
/// whole program, and a composed layer builds thousands of section graphs.
struct GraphContext {
    /// The item-loop cap, [`item_window`].
    window: usize,
    /// Each buffer's depth in pages.
    depth: Vec<usize>,
    /// The multicast group feeding each receive buffer.
    recv_group: BTreeMap<BufferId, GroupId>,
    /// Each trip's lead bound, [`trip_spreads`].
    spreads: Vec<usize>,
}

impl GraphContext {
    fn of(p: &SpatialProgram) -> Self {
        let recv_group = p
            .stages
            .iter()
            .filter_map(|s| match (&s.kind, s.outs.first()) {
                (StageKind::Recv { group }, Some(o)) => Some((*o, *group)),
                _ => None,
            })
            .collect();
        GraphContext {
            window: item_window(p),
            depth: p.buffers.iter().map(|bf| bf.kind.depth_pages()).collect(),
            recv_group,
            spreads: trip_spreads(p),
        }
    }
}

/// Build the unrolled event graph of the schedule, or of ONE section of it.
///
/// `sel` names the programs holding the section's ops and each one's half-open range of top-level ops; `None`
/// takes every op of every program. Places are keyed by resource and round, and a section that owns its
/// buffers, semaphores and groups outright (which is what [`section_ranges`] checks before handing any
/// selection back) restarts those rounds at one — so the section's subgraph is exactly the part of the
/// whole graph its ops induce, program-counter arcs across the cut aside. The work is linear in the ops
/// selected: a program with no op in the section contributes nothing and is not visited.
///
/// Each loop run to a trip runs `counts[trip]` times.
fn build_graph_over(
    p: &SpatialProgram,
    cx: &GraphContext,
    sel: Option<&[(usize, std::ops::Range<usize>)]>,
    counts: &[usize],
) -> Result<MarkedGraph, CertifyFault> {
    let sched = p.schedule.as_ref().ok_or(CertifyFault::NotScheduled)?;
    let pl = p.placement.as_ref().ok_or(CertifyFault::NotScheduled)?;
    let mut b = Builder {
        places: Vec::new(),
        index: FxHashMap::default(),
        transitions: Vec::new(),
    };
    let chosen: Vec<(&EngineProgram, &[TaggedOp])> = match sel {
        Some(r) => r
            .iter()
            .map(|(pi, ops)| (&sched.programs[*pi], &sched.programs[*pi].ops[ops.clone()]))
            .collect(),
        None => sched
            .programs
            .iter()
            .map(|prog| (prog, prog.ops.as_slice()))
            .collect(),
    };
    // Per-program unrolled events (item loops within the provable window).
    let mut events: Vec<Vec<Event>> = Vec::new();
    for (_, ops) in &chosen {
        let mut out = Vec::new();
        unroll(ops, &mut out, cx.window, counts);
        events.push(out);
    }
    // Global transition ids: (program, event) → id.
    let mut base = Vec::new();
    let mut total = 0;
    for ev in &events {
        base.push(total);
        total += ev.len();
    }
    for (ci, (prog, _)) in chosen.iter().enumerate() {
        for (ei, _) in events[ci].iter().enumerate() {
            b.transitions.push(Transition {
                site: prog.site,
                engine: prog.engine,
                op_index: ei,
                consumes: Vec::new(),
                produces: Vec::new(),
            });
        }
    }
    // Program order through Pc places.
    for (ci, (prog, _)) in chosen.iter().enumerate() {
        for ei in 0..events[ci].len() {
            let t = base[ci] + ei;
            let pc_in = b.place(
                PlaceKind::Pc {
                    site: prog.site,
                    engine: prog.engine,
                    k: ei,
                },
                if ei == 0 { 1 } else { 0 },
            );
            b.transitions[t].consumes.push(pc_in);
            let pc_out = b.place(
                PlaceKind::Pc {
                    site: prog.site,
                    engine: prog.engine,
                    k: ei + 1,
                },
                0,
            );
            b.transitions[t].produces.push(pc_out);
        }
    }
    // Buffer tokens: per buffer, walk producer and consumer programs in order. The counters are sparse — a
    // section's graph touches only its own buffers.
    let depth = &cx.depth;
    let (mut reserved, mut pushed, mut waited_hi, mut popped): (
        FxHashMap<usize, usize>,
        FxHashMap<usize, usize>,
        FxHashMap<usize, usize>,
        FxHashMap<usize, usize>,
    ) = Default::default();
    // Semaphore rounds: per (sem, site) counters of incs / waits / sets.
    let mut inc_round: FxHashMap<(SemId, Site), usize> = FxHashMap::default();
    let mut wait_round: FxHashMap<(SemId, Site), usize> = FxHashMap::default();
    // Rendezvous posts, keyed by the site they land on (see `PlaceKind::Arrival`).
    let mut arrival_round: FxHashMap<(SemId, Site), usize> = FxHashMap::default();
    // A shared valid flag is set per GROUP: each sender's flag multicasts count their own rounds.
    let mut valid_set_round: FxHashMap<(SemId, GroupId), usize> = FxHashMap::default();
    let mut mcast_round: FxHashMap<GroupId, usize> = FxHashMap::default();
    let mut recv_push_round: FxHashMap<(GroupId, Site), usize> = FxHashMap::default();
    let recv_group = &cx.recv_group;
    // Events must be processed in an order where producers precede consumers per buffer — program order
    // within a program suffices for pairing indices, so we walk programs sequentially.
    for (ci, (prog, _)) in chosen.iter().enumerate() {
        for (ei, ev) in events[ci].iter().enumerate() {
            let t = base[ci] + ei;
            match ev.op {
                ChoreoOp::Reserve { buf, pages } => {
                    for _ in 0..*pages {
                        let k = bump(&mut reserved, buf.idx());
                        let initial = k <= depth[buf.idx()];
                        let pl_id = b.place(
                            PlaceKind::Space { buf: *buf, k },
                            if initial { 1 } else { 0 },
                        );
                        b.transitions[t].consumes.push(pl_id);
                    }
                }
                ChoreoOp::Push { buf, pages } => {
                    for _ in 0..*pages {
                        let k = bump(&mut pushed, buf.idx());
                        let pl_id = b.place(PlaceKind::Data { buf: *buf, k }, 0);
                        b.transitions[t].produces.push(pl_id);
                    }
                    if let Some(g) = recv_group.get(buf) {
                        let r = *recv_push_round.entry((*g, prog.site)).or_insert(0);
                        recv_push_round.insert((*g, prog.site), r + 1);
                        let pl_id = b.place(
                            PlaceKind::McastData {
                                group: *g,
                                receiver: prog.site,
                                k: r + 1,
                            },
                            0,
                        );
                        b.transitions[t].consumes.push(pl_id);
                    }
                }
                ChoreoOp::Wait { buf, pages } => {
                    let need = popped.get(&buf.idx()).copied().unwrap_or(0) + *pages;
                    while waited_hi.get(&buf.idx()).copied().unwrap_or(0) < need {
                        let k = bump(&mut waited_hi, buf.idx());
                        let pl_id = b.place(PlaceKind::Data { buf: *buf, k }, 0);
                        b.transitions[t].consumes.push(pl_id);
                    }
                }
                ChoreoOp::Pop { buf, pages } => {
                    for _ in 0..*pages {
                        let k = bump(&mut popped, buf.idx()) + depth[buf.idx()];
                        let pl_id = b.place(PlaceKind::Space { buf: *buf, k }, 0);
                        b.transitions[t].produces.push(pl_id);
                    }
                    let done = popped.get(&buf.idx()).copied().unwrap_or(0);
                    let hi = waited_hi.entry(buf.idx()).or_insert(0);
                    *hi = (*hi).max(done);
                }
                ChoreoOp::SemInc { sem, at, by: _ }
                    if matches!(p.semaphores[sem.idx()].role, SemRole::Barrier { .. }) =>
                {
                    let r = *arrival_round.entry((*sem, *at)).or_insert(0);
                    arrival_round.insert((*sem, *at), r + 1);
                    let pl_id = b.place(
                        PlaceKind::Arrival {
                            sem: *sem,
                            at: *at,
                            k: r + 1,
                        },
                        0,
                    );
                    b.transitions[t].produces.push(pl_id);
                }
                ChoreoOp::SemInc { sem, at, by: _ } => {
                    let r = *inc_round.entry((*sem, prog.site)).or_insert(0);
                    inc_round.insert((*sem, prog.site), r + 1);
                    let _ = at;
                    let pl_id = b.place(
                        PlaceKind::Ready {
                            sem: *sem,
                            receiver: prog.site,
                            k: r + 1,
                        },
                        0,
                    );
                    b.transitions[t].produces.push(pl_id);
                }
                // A rendezvous wait for `c` consumes the first `c` posts AT THIS SITE — the gather's root
                // every arrival, a release waiter its site's one release — and it is what a dropped post
                // starves.
                ChoreoOp::SemWait { sem, until }
                    if matches!(p.semaphores[sem.idx()].role, SemRole::Barrier { .. }) =>
                {
                    let (SemCond::AtLeast(c) | SemCond::Equals(c)) = *until;
                    for k in 1..=c {
                        let pl_id = b.place(
                            PlaceKind::Arrival {
                                sem: *sem,
                                at: prog.site,
                                k,
                            },
                            0,
                        );
                        b.transitions[t].consumes.push(pl_id);
                    }
                }
                ChoreoOp::SemWait { sem, .. } => {
                    let r = *wait_round.entry((*sem, prog.site)).or_insert(0);
                    wait_round.insert((*sem, prog.site), r + 1);
                    match p.semaphores[sem.idx()].role {
                        SemRole::Barrier { .. } => {
                            unreachable!("a rendezvous wait is matched above")
                        }
                        SemRole::ReadyCounter { .. } => {
                            // The sender waits for every receiver's (r+1)-th increment.
                            let sem_ref = &p.semaphores[sem.idx()];
                            let own_group = sem_ref.operand.and_then(|op| {
                                pl.groups
                                    .iter()
                                    .find(|g| g.operand == op && g.sender == prog.site)
                            });
                            let receivers: Vec<Site> = match own_group {
                                Some(gg) => gg
                                    .rect
                                    .sites()
                                    .into_iter()
                                    .filter(|s| *s != gg.sender)
                                    .collect(),
                                None => sem_ref
                                    .sites
                                    .iter()
                                    .copied()
                                    .filter(|s| *s != prog.site)
                                    .collect(),
                            };
                            for rcv in receivers {
                                let pl_id = b.place(
                                    PlaceKind::Ready {
                                        sem: *sem,
                                        receiver: rcv,
                                        k: r + 1,
                                    },
                                    0,
                                );
                                b.transitions[t].consumes.push(pl_id);
                            }
                        }
                        SemRole::ValidFlag => {
                            let pl_id = b.place(
                                PlaceKind::Valid {
                                    sem: *sem,
                                    receiver: prog.site,
                                    k: r + 1,
                                },
                                0,
                            );
                            b.transitions[t].consumes.push(pl_id);
                        }
                    }
                }
                ChoreoOp::McastSetValid { sem, group } => {
                    let r = *valid_set_round.entry((*sem, *group)).or_insert(0);
                    valid_set_round.insert((*sem, *group), r + 1);
                    let g = &pl.groups[group.idx()];
                    for rcv in g.rect.sites().into_iter().filter(|s| *s != g.sender) {
                        let pl_id = b.place(
                            PlaceKind::Valid {
                                sem: *sem,
                                receiver: rcv,
                                k: r + 1,
                            },
                            0,
                        );
                        b.transitions[t].produces.push(pl_id);
                    }
                }
                ChoreoOp::McastSend { group, .. } => {
                    let r = *mcast_round.entry(*group).or_insert(0);
                    mcast_round.insert(*group, r + 1);
                    let g = &pl.groups[group.idx()];
                    for rcv in g.rect.sites().into_iter().filter(|s| *s != g.sender) {
                        let pl_id = b.place(
                            PlaceKind::McastData {
                                group: *group,
                                receiver: rcv,
                                k: r + 1,
                            },
                            0,
                        );
                        b.transitions[t].produces.push(pl_id);
                    }
                }
                _ => {}
            }
        }
    }
    Ok(MarkedGraph {
        places: b.places,
        transitions: b.transitions,
    })
}

/// Advance a sparse counter and return its new value.
fn bump(counts: &mut FxHashMap<usize, usize>, key: usize) -> usize {
    let c = counts.entry(key).or_insert(0);
    *c += 1;
    *c
}

/// One transition in a token-free cycle: the op that waits, and the empty place it waits on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeadlockNode {
    pub site: Site,
    pub engine: Engine,
    pub op_index: usize,
    pub waiting_on: PlaceKind,
}

/// Find a real back edge in the stuck sub-graph and return the ring it closes.
///
/// A greedy walk is not enough here: program order chains every op of an engine to the next, so following
/// successors marches to the end of a program without ever revisiting. Only a depth-first search that keeps
/// its recursion stack can tell a forward edge from the back edge that actually closes a cycle, and the back
/// edge is the whole diagnostic — it names the two hand-offs waiting on each other.
fn token_free_cycle(
    g: &MarkedGraph,
    indeg: &[usize],
    succ: &[Vec<usize>],
    producer: &[Option<usize>],
) -> Vec<DeadlockNode> {
    let n = g.transitions.len();
    let stuck = |i: usize| indeg[i] > 0;
    // 0 = unvisited, 1 = on the stack, 2 = done.
    let mut color = vec![0u8; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut ring: Vec<usize> = Vec::new();
    for root in (0..n).filter(|i| stuck(*i)) {
        if color[root] != 0 {
            continue;
        }
        // Iterative DFS: (node, index of the next successor to try).
        let mut work: Vec<(usize, usize)> = vec![(root, 0)];
        color[root] = 1;
        stack.push(root);
        while !work.is_empty() {
            let (node, idx) = work.last().map(|t| (t.0, t.1)).expect("non-empty");
            match succ[node].get(idx).copied() {
                Some(next) => {
                    work.last_mut().expect("non-empty").1 += 1;
                    if !stuck(next) {
                        continue;
                    }
                    if color[next] == 1 {
                        let at = stack.iter().position(|&x| x == next).unwrap_or(0);
                        ring = stack[at..].to_vec();
                        break;
                    }
                    if color[next] == 0 {
                        color[next] = 1;
                        stack.push(next);
                        work.push((next, 0));
                    }
                }
                None => {
                    color[node] = 2;
                    stack.pop();
                    work.pop();
                }
            }
        }
        if !ring.is_empty() {
            break;
        }
    }
    // No back edge means the sweep stalled on a starved arc rather than a cycle; report the stuck head.
    if ring.is_empty() {
        ring = (0..n).filter(|i| stuck(*i)).take(1).collect();
    }
    ring.iter()
        .take(12)
        .map(|&ti| {
            let t = &g.transitions[ti];
            // The arc that holds this op is the empty place whose producer is also stuck.
            let waiting_on = t
                .consumes
                .iter()
                .find(|pl| {
                    g.places[**pl].marking == 0 && producer[**pl].map(stuck).unwrap_or(false)
                })
                .or_else(|| t.consumes.iter().find(|pl| g.places[**pl].marking == 0))
                .map(|pl| g.places[*pl].kind)
                .unwrap_or(g.places[0].kind);
            DeadlockNode {
                site: t.site,
                engine: t.engine,
                op_index: t.op_index,
                waiting_on,
            }
        })
        .collect()
}

/// Commoner's condition on the unrolled net: a starved transition or a token-free cycle is a fault; else the
/// topological order over token-free arcs is the witness.
pub fn liveness_witness(g: &MarkedGraph) -> Result<LivenessWitness, CertifyFault> {
    let n = g.transitions.len();
    let mut producer = vec![None; g.places.len()];
    for (ti, t) in g.transitions.iter().enumerate() {
        for pl in &t.produces {
            producer[*pl] = Some(ti);
        }
    }
    let mut indeg = vec![0usize; n];
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (ti, t) in g.transitions.iter().enumerate() {
        for pl in &t.consumes {
            if g.places[*pl].marking > 0 {
                continue;
            }
            match producer[*pl] {
                Some(pr) => {
                    succ[pr].push(ti);
                    indeg[ti] += 1;
                }
                None => {
                    return Err(CertifyFault::Starved {
                        transition: ti,
                        place: g.places[*pl].kind,
                    })
                }
            }
        }
    }
    // The lowest-numbered ready transition first, so the witness is a function of the graph alone.
    let mut order = Vec::with_capacity(n);
    let mut ready: std::collections::BinaryHeap<std::cmp::Reverse<usize>> = (0..n)
        .filter(|i| indeg[*i] == 0)
        .map(std::cmp::Reverse)
        .collect();
    while let Some(std::cmp::Reverse(t)) = ready.pop() {
        order.push(t);
        for s in &succ[t] {
            indeg[*s] -= 1;
            if indeg[*s] == 0 {
                ready.push(std::cmp::Reverse(*s));
            }
        }
    }
    if order.len() != n {
        return Err(CertifyFault::Deadlock {
            cycle: token_free_cycle(g, &indeg, &succ, &producer),
        });
    }
    Ok(LivenessWitness { topo_order: order })
}

fn verify_witness(g: &MarkedGraph, w: &LivenessWitness) -> bool {
    let n = g.transitions.len();
    if w.topo_order.len() != n {
        return false;
    }
    let mut pos = vec![usize::MAX; n];
    for (i, t) in w.topo_order.iter().enumerate() {
        if *t >= n || pos[*t] != usize::MAX {
            return false;
        }
        pos[*t] = i;
    }
    let mut producer = vec![None; g.places.len()];
    for (ti, t) in g.transitions.iter().enumerate() {
        for pl in &t.produces {
            producer[*pl] = Some(ti);
        }
    }
    for (ti, t) in g.transitions.iter().enumerate() {
        for pl in &t.consumes {
            if g.places[*pl].marking > 0 {
                continue;
            }
            match producer[*pl] {
                Some(pr) => {
                    if pos[pr] >= pos[ti] {
                        return false;
                    }
                }
                None => return false,
            }
        }
    }
    true
}

/// Counting balances from the schedule (loop multiplicities), high-water from the dry run.
fn balances(p: &SpatialProgram, high_water: &BTreeMap<BufferId, usize>) -> Vec<BufferBalance> {
    let sched = p.schedule.as_ref().unwrap();
    let mut reserved = vec![0u64; p.buffers.len()];
    let mut pushed = vec![0u64; p.buffers.len()];
    let mut popped = vec![0u64; p.buffers.len()];
    for prog in &sched.programs {
        crate::spatial::ir::walk_tagged_ops(&prog.ops, 1, &mut |op, m| match op {
            ChoreoOp::Reserve { buf, pages } => reserved[buf.idx()] += m * *pages as u64,
            ChoreoOp::Push { buf, pages } => pushed[buf.idx()] += m * *pages as u64,
            ChoreoOp::Pop { buf, pages } => popped[buf.idx()] += m * *pages as u64,
            _ => {}
        });
    }
    p.buffers
        .iter()
        .map(|b| BufferBalance {
            buf: b.id,
            depth_pages: b.kind.depth_pages(),
            reserved: reserved[b.id.idx()],
            pushed: pushed[b.id.idx()],
            popped: popped[b.id.idx()],
            high_water: high_water.get(&b.id).copied().unwrap_or(0),
        })
        .collect()
}

fn sem_counts(p: &SpatialProgram) -> Vec<(SemId, u64, u64)> {
    let sched = p.schedule.as_ref().unwrap();
    let mut incs = vec![0u64; p.semaphores.len()];
    let mut waits = vec![0u64; p.semaphores.len()];
    for prog in &sched.programs {
        crate::spatial::ir::walk_tagged_ops(&prog.ops, 1, &mut |op, m| match op {
            ChoreoOp::SemInc { sem, by, .. } => incs[sem.idx()] += m * *by as u64,
            ChoreoOp::McastSetValid { sem, .. } => incs[sem.idx()] += m,
            ChoreoOp::SemWait { sem, until } => {
                let n = match (p.semaphores[sem.idx()].role, until) {
                    (SemRole::ValidFlag, _) => 1,
                    (_, SemCond::Equals(n)) | (_, SemCond::AtLeast(n)) => *n as u64,
                };
                waits[sem.idx()] += m * n;
            }
            _ => {}
        });
    }
    p.semaphores
        .iter()
        .map(|s| (s.id, incs[s.id.idx()], waits[s.id.idx()]))
        .collect()
}

/// Lower an ALU/select cell body to the bounded fragment (the placer's own lowering vocabulary).
///
/// This is the TREE form: a cell read by two later cells is copied into both, and their readers
/// copy those, so the body's `BoundedExpr` is the cell DAG UNFOLDED — `fib.lg`'s 75 cells unfold
/// to 1 657 763 nodes. Callers that speak the tree (printers, SMT surfaces) want exactly that;
/// the certifier does not, and uses [`cells_to_gates`](crate::codegen_sva::fabric_grid::cells_to_gates)
/// to build the same datapath into the shared gate DAG instead, through the SAME walk, so the two
/// forms cannot drift.
pub fn cells_to_bounded(cells: &[LoopCell], result: Src, width: u32) -> Option<BoundedExpr> {
    cells_datapath(
        &mut BoundedBuild::new(width, &|k| format!("in{k}")),
        cells,
        result,
    )
}

/// The PE datapath of an integer tile matmul: `psum + a·w`, certified at the 8-bit operand bound.
fn pe_certificate(bits: u32) -> ComputeStageCert {
    match pe_obligation(bits).and_then(|o| refute(o).map_err(|_| PE_ESCAPED.to_string())) {
        Ok(c) => ComputeStageCert::Sat(c),
        Err(reason) => ComputeStageCert::Uncertified { reason },
    }
}

const PE_ESCAPED: &str = "the PE obligation escaped the propositional fragment";

/// What an integer tile matmul's PE certificate refutes: the placed PE grid against `psum + a·w`, at the 8-bit operand
/// bound.
fn pe_obligation(bits: u32) -> Result<Obligation, String> {
    let width = bits.min(8);
    let pe = vec![
        FabricOp {
            op: AluOp::Mul,
            a: Src::In(0),
            b: Src::In(2),
        },
        FabricOp {
            op: AluOp::Add,
            a: Src::In(1),
            b: Src::Val(0),
        },
    ];
    let cells: Vec<LoopCell> = pe
        .iter()
        .map(|o| LoopCell::Arith {
            op: o.op,
            a: o.a,
            b: o.b,
        })
        .collect();
    let g = route_loop_body(&cells, 3, &[Src::Val(1)]);
    let placed = grid_output_to_bounded(&g, 0, width, &|k| format!("in{k}"));
    let spec = crate::codegen_sva::fabric_rtl::fabric_to_bounded(&pe, 1, width);
    mapping_obligation_within(placed, spec, BLAST_BUDGET_NODES).map_err(|_| PE_ESCAPED.to_string())
}

/// The per-PE proof of a scalar ALU/select stage: the ROUTED grid proven equal to the body's own
/// datapath, for every 8-bit input, with the refutation kept.
pub fn scalar_certificate(body: &ScalarBody) -> ComputeStageCert {
    scalar_certificate_within(body, GATE_BUDGET_NODES)
}

/// [`scalar_certificate`] against an EXPLICIT gate ceiling — the seam a lock shrinks to prove the
/// budget is live code and not a constant nobody reads.
pub fn scalar_certificate_within(body: &ScalarBody, gate_budget: u64) -> ComputeStageCert {
    match scalar_obligation_within(body, gate_budget).and_then(|o| refute(o).map_err(|r| r.reason())) {
        Ok(c) => ComputeStageCert::Sat(c),
        Err(reason) => ComputeStageCert::Uncertified { reason },
    }
}

/// What a scalar body's certificate refutes: its ROUTED grid against the body's own datapath, every 8-bit input, under
/// `gate_budget` — or why the body has none.
pub fn scalar_obligation_within(body: &ScalarBody, gate_budget: u64) -> Result<Obligation, String> {
    let combinational = body
        .cells
        .iter()
        .all(|c| matches!(c, LoopCell::Arith { .. } | LoopCell::Select { .. }));
    if !combinational {
        return Err("a functional-unit or memory cell has no bit-blast lowering".into());
    }
    let Src::Val(_) = body.result else {
        return Err("the body's result is not a cell value".into());
    };
    // Every tile on the array has the same two routed operand channels (its north and its west).
    // A computed PREDICATE is an ordinary dataflow edge and takes one of them like any other
    // operand — but a mux whose comparison AND both arms read three different cells wants a third
    // channel the interconnect does not have. That is the one shape still declined, and it is
    // declined by name, from the cells, before the placer is asked to route it.
    if let Some((cell, demand)) = unroutable_cell(&body.cells) {
        return Err(format!(
            "cell {cell} needs {demand} distinct computed values routed to one tile, and a tile has \
             {TILE_OPERAND_CHANNELS} operand channels (north and west) — a predicated select that \
             compares a computed value AND selects between two more has no placed form"
        ));
    }
    let width = 8;
    // Both sides of the obligation are built STRAIGHT INTO THE SHARED GATE DAG. The tree form of
    // this same datapath is the cell DAG UNFOLDED (`fib.lg`: 75 cells → 1 657 763 `BoundedExpr`
    // nodes → a 10^14-node propositional formula, which is not a thing any machine holds); the
    // circuit is linear in the cells, and the Tseitin encoding of the circuit is linear in the
    // circuit. So the certificate is priced in GATES, and the tree is never built at all.
    //
    // The SPEC is built and priced FIRST because it is the cheap side: it costs one pass over the
    // cells, where the routed grid is a (2m+2)×m tile array. A body past the ceiling is therefore
    // refused without ever allocating the array that would place it.
    let Some(spec) = cells_to_gates(&body.cells, body.result, width) else {
        return Err("the body escaped the bounded fragment".into());
    };
    let size = word_size(&spec);
    if size.dag_nodes > gate_budget {
        let refusal = BlastRefusal::GateBudget {
            dag_nodes: size.dag_nodes,
            budget: gate_budget,
        };
        return Err(refusal.reason());
    }
    let g = route_loop_body(&body.cells, body.n_in, &[body.result]);
    let Some(placed) = grid_output_to_gates(&g, 0, width, &|k| format!("in{k}")) else {
        return Err("the placed grid escaped the bounded fragment".into());
    };
    gates_obligation_within(&placed, &spec, gate_budget).map_err(|r| r.reason())
}

/// How large a datapath tree the scalar certificate will pay to build.
///
/// The unit is `BoundedExpr` nodes in the UNFOLDED body spec — the cheapest quantity that is both
/// exact (it is what `cells_to_bounded` allocates) and knowable without allocating. Every ALU body
/// that certifies today prices in the tens (`mix` 12, one fib doubling block 29, two blocks 277),
/// so 50 000 is over two orders of magnitude of headroom; the six-block fib doubling chain prices
/// at 1 657 763 — refused before a single node is allocated, in microseconds. The second,
/// exact ceiling is [`crate::codegen_sva::bitblast::BLAST_BUDGET_NODES`] on the materialised
/// formula: this one stops the runaway, that one stops the merely-enormous.
pub const SCALAR_DATAPATH_BUDGET_NODES: u128 = 50_000;

/// The `BoundedExpr` node count `cells_to_bounded` will materialise for `body`, computed from the
/// cell DAG alone — one pass, no allocation, saturating so an astronomical body is priced rather
/// than overflowing. Mirrors `cells_to_bounded`'s shapes exactly: an `Arith` cell is
/// `1 + |a| + |b|`, a `Select` is the comparison (`1 + |ca| + |cb|`) plus both arms plus the ite
/// node, and a `Val(k)` operand costs a full copy of cell `k`.
pub fn bounded_datapath_nodes(body: &ScalarBody) -> u128 {
    fn src(s: &Src, vals: &[u128]) -> u128 {
        match s {
            Src::In(_) | Src::Const(_) => 1,
            Src::Val(k) => vals.get(*k).copied().unwrap_or(1),
        }
    }
    let mut vals: Vec<u128> = Vec::with_capacity(body.cells.len());
    for c in &body.cells {
        let v = match c {
            LoopCell::Arith { a, b, .. } => src(a, &vals)
                .saturating_add(src(b, &vals))
                .saturating_add(1),
            LoopCell::Select { ca, cb, t, e, .. } => src(ca, &vals)
                .saturating_add(src(cb, &vals))
                .saturating_add(1)
                .saturating_add(src(t, &vals))
                .saturating_add(src(e, &vals))
                .saturating_add(1),
            // Not combinational — `scalar_certificate` has already declined by name above; price it
            // as a leaf so this function is total.
            _ => 1,
        };
        vals.push(v);
    }
    src(&body.result, &vals)
}

/// The stages a certificate carries a compute certificate for: every tile matmul and every scalar body.
fn has_compute_certificate(s: &Stage) -> bool {
    matches!(
        s.kind,
        StageKind::Compute(ComputeOp::MatmulTiles { .. }) | StageKind::Scalar(_)
    )
}

/// What a compute stage's certificate is of, by the one thing it depends on — a tile matmul's operand precision, a
/// scalar stage's body: stages alike share it.
fn compute_key(s: &Stage) -> Option<String> {
    match &s.kind {
        StageKind::Compute(ComputeOp::MatmulTiles { operand, .. }) => Some(format!("mm:{operand:?}")),
        StageKind::Scalar(body) => Some(format!("scalar:{}", serde_json::to_string(body).unwrap_or_default())),
        _ => None,
    }
}

/// The formula a compute stage's SAT certificate must refute, rebuilt from the stage itself: an integer tile matmul's PE
/// obligation, a scalar body's routed-grid obligation; `Err` with why for a stage that has none — a float datapath, which
/// is certified by evaluation, or a body outside the bounded fragment.
pub fn compute_obligation(s: &Stage) -> Result<Obligation, String> {
    match &s.kind {
        StageKind::Compute(ComputeOp::MatmulTiles { operand, .. }) if precision_is_int(*operand) => {
            pe_obligation(crate::spatial::target::precision_int_bits(*operand).unwrap_or(64))
        }
        StageKind::Compute(ComputeOp::MatmulTiles { operand, .. }) => {
            Err(format!("a {operand:?} tile datapath is certified by evaluation, and has no SAT obligation"))
        }
        StageKind::Scalar(body) => scalar_obligation_within(body, GATE_BUDGET_NODES),
        _ => Err("the stage has no compute certificate".into()),
    }
}

fn compute_certificates(p: &SpatialProgram) -> Vec<(StageId, ComputeStageCert)> {
    let mut by_origin: BTreeMap<String, ComputeStageCert> = BTreeMap::new();
    let mut out = Vec::new();
    for s in p.stages.iter().filter(|s| has_compute_certificate(s)) {
        let make = || match &s.kind {
            StageKind::Compute(ComputeOp::MatmulTiles { operand, .. }) if precision_is_int(*operand) => {
                pe_certificate(crate::spatial::target::precision_int_bits(*operand).unwrap_or(64))
            }
            StageKind::Compute(ComputeOp::MatmulTiles { operand, .. }) => ComputeStageCert::Uncertified {
                reason: format!("float tensor datapath ({operand:?} operands, fp32 accumulate) cannot be bit-blasted; equivalence is by evaluation"),
            },
            StageKind::Scalar(body) => scalar_certificate(body),
            _ => unreachable!("a stage with a compute certificate"),
        };
        if let Some(key) = compute_key(s) {
            let cert = by_origin.entry(key).or_insert_with(make).clone();
            out.push((s.id, cert));
        }
    }
    out
}

/// Inputs for a dry run: deterministic fills where recorded, zeros for external data.
pub fn dry_inputs(p: &SpatialProgram) -> Inputs {
    let mut inputs = Inputs::default();
    for t in &p.tensors {
        if t.role == TensorRole::Output {
            continue;
        }
        let n = t.elems.rows * t.elems.cols;
        let vals: Vec<i64> = (0..n).map(|i| t.init.element(i).unwrap_or(0)).collect();
        inputs.tensors.insert(t.id, Value::Int(vals));
    }
    inputs
}

/// ★ THE ORDERING OBLIGATIONS READ ONE INDEX OF THE PROGRAM, built in one pass: each stage's section, each tensor's
/// loads and stores, each stage's dataflow successors, and a prefix count of the barriers. The obligations ask their
/// questions — which sections load and store a tensor, whether a barrier stands between two sections, whether a store
/// depends on a load — per carried tensor and per pair of sections, and a batched tensor-parallel layer has thousands
/// of each; from the index every answer costs the size of what it asks about, not the size of the program.
struct OrderIndex<'a> {
    p: &'a SpatialProgram,
    /// The section of each stage (0 for a program without sections).
    sec_of: Vec<usize>,
    loads: BTreeMap<TensorId, Vec<StageId>>,
    stores: BTreeMap<TensorId, Vec<StageId>>,
    /// Each stage's dataflow successors.
    succ: Vec<Vec<StageId>>,
    /// `barriers_before[k]`: how many of sections `0..k` end in a barrier.
    barriers_before: Vec<usize>,
}

impl<'a> OrderIndex<'a> {
    fn new(p: &'a SpatialProgram) -> Self {
        let mut sec_of = vec![0usize; p.stages.len()];
        for (k, sp) in p.sections.iter().enumerate() {
            for slot in sec_of
                .iter_mut()
                .take(sp.stages.1.min(p.stages.len()))
                .skip(sp.stages.0)
            {
                *slot = k;
            }
        }
        let (mut loads, mut stores): (
            BTreeMap<TensorId, Vec<StageId>>,
            BTreeMap<TensorId, Vec<StageId>>,
        ) = (BTreeMap::new(), BTreeMap::new());
        for s in &p.stages {
            match &s.kind {
                StageKind::Load { tensor, .. } => loads.entry(*tensor).or_default().push(s.id),
                StageKind::Store { tensor, .. } => stores.entry(*tensor).or_default().push(s.id),
                _ => {}
            }
        }
        let mut succ = vec![Vec::new(); p.stages.len()];
        for e in &p.edges {
            if let Some(out) = succ.get_mut(e.from.idx()) {
                out.extend(e.to.iter().copied());
            }
        }
        let mut barriers_before = Vec::with_capacity(p.sections.len() + 1);
        barriers_before.push(0);
        for sp in &p.sections {
            barriers_before.push(
                barriers_before.last().copied().unwrap_or(0)
                    + usize::from(sp.barrier_after.is_some()),
            );
        }
        OrderIndex {
            p,
            sec_of,
            loads,
            stores,
            succ,
            barriers_before,
        }
    }

    /// The sections a tensor is loaded in, and the sections it is stored in.
    fn touch_sections(&self, t: TensorId) -> (Vec<usize>, Vec<usize>) {
        let secs = |stages: Option<&Vec<StageId>>| -> Vec<usize> {
            let mut v: Vec<usize> = stages
                .map(|s| s.iter().map(|id| self.sec_of[id.idx()]).collect())
                .unwrap_or_default();
            v.sort_unstable();
            v.dedup();
            v
        };
        (secs(self.loads.get(&t)), secs(self.stores.get(&t)))
    }

    /// Whether some section in `[from, to)` ends in a barrier — the rendezvous that makes "section `from` happens
    /// before section `to`" a fact about the schedule rather than a convention. ONE suffices: `validate_rendezvous`
    /// places every op of a section after the last barrier before it and before the first barrier at or after it, so
    /// a barrier after section `b` orders every section up to `b` before every section after it. Sections with no
    /// barrier between them run concurrently, and must share no data.
    fn barriers_between(&self, from: usize, to: usize) -> bool {
        let at = |k: usize| {
            self.barriers_before
                .get(k.min(self.barriers_before.len() - 1))
                .copied()
                .unwrap_or(0)
        };
        from < to && at(to) > at(from)
    }

    /// Whether `to` is DOWNSTREAM of `from` in the buffer dataflow — the order a section already has
    /// inside itself, because a stage cannot run before whatever fills its operands.
    ///
    /// ★ THE SAME-SECTION READ-THEN-WRITE IS REAL, NOT A LOOPHOLE. A conv ring is `CS' = shift(CS) ← x`:
    /// one section that reads the memory and writes it, and the read is ordered before the write by the
    /// FIFO between the load and the store — the same hand-off every operand in the IR travels on. Demanding
    /// a barrier there would force a pointless extra section; accepting it WITHOUT this check would let a
    /// store and a load of the same state sit side by side on two engines with nothing between them. So the
    /// exception is exactly "the write is downstream of the read", walked here rather than assumed.
    fn reaches(&self, from: StageId, to: StageId) -> bool {
        let mut seen = vec![false; self.p.stages.len()];
        let mut queue = vec![from];
        seen[from.idx()] = true;
        while let Some(s) = queue.pop() {
            if s == to {
                return true;
            }
            for t in &self.succ[s.idx()] {
                if !seen[t.idx()] {
                    seen[t.idx()] = true;
                    queue.push(*t);
                }
            }
        }
        false
    }

    /// A section that reads and writes `t` must make every write DEPEND on a read — PER SITE.
    ///
    /// ★ THE DEPENDENCE IS PER SITE, AND THE ALL-PAIRS FORM WAS AN ARTIFACT. This once demanded that every
    /// `Load` of `t` reach every `Store` of `t`, which is right while a section has ONE work item and exactly
    /// one copy of each stage. `place` clones a section's stages per site, so a section with 16 items has 16
    /// loads and 16 stores on 16 different sites — and site 0's load can never reach site 1's store, because
    /// there is no edge between them and no reason there should be: they touch DISJOINT TILES. Under the old
    /// form any blocked row op that carried state was unsatisfiable by construction. (Measured: a 512-wide
    /// blocked map lowers to `items=16 loads=16 stores=16 sites=16`.)
    ///
    /// What the obligation actually protects is that a write is never unordered with respect to the read it
    /// supersedes — otherwise the step could clobber the state before reading it. That is a statement about
    /// one site's dataflow, so it is checked there: every store must be downstream of a load ON ITS OWN SITE.
    /// An unplaced section (no sites yet) keeps the whole-section form, which is the same thing when there is
    /// one copy.
    fn read_before_write_in_section(&self, t: TensorId, section: usize) -> bool {
        let in_section = |ids: Option<&Vec<StageId>>| -> Vec<&Stage> {
            ids.map(|v| {
                v.iter()
                    .filter(|id| self.sec_of[id.idx()] == section)
                    .map(|id| self.p.stage(*id))
                    .collect()
            })
            .unwrap_or_default()
        };
        let (loads, stores) = (
            in_section(self.loads.get(&t)),
            in_section(self.stores.get(&t)),
        );
        if loads.is_empty() || stores.is_empty() {
            return false;
        }
        // Every store is downstream of a load that shares its site (or, unplaced, of some load).
        stores.iter().all(|w| {
            loads
                .iter()
                .filter(|l| w.site.is_none() || l.site == w.site)
                .any(|l| self.reaches(l.id, w.id))
        })
    }
}

/// ★ THE TWO OBLIGATIONS A COMPOSITION ADDS, AND WHY THEY ARE NOT THE LIVENESS PROOF.
///
/// Liveness says the choreography never wedges. It says nothing about WHICH section wrote the bytes a
/// later section reads: a program whose sections all ran in the wrong order is perfectly live. These
/// two say the data dependences the composer recorded are realized by barriers in the schedule.
fn ordering_obligations(p: &SpatialProgram) -> Result<(), CertifyFault> {
    let index = std::cell::OnceCell::new();
    let ix = || index.get_or_init(|| OrderIndex::new(p));
    for t in &p.tensors {
        match t.role {
            // ★ AN ACTIVATION MAY BE WRITTEN IN PARTS — a matrix a tile row at a time, a chunk's rows by the chunk's section
            // — each tile by ONE section (the program's coverage proof counts every tile's stores across all sections).
            // What a reader needs is the whole of it: every reader comes after EVERY writer, separated from each by a
            // barrier chain.
            TensorRole::Activation => {
                let (readers, writers) = ix().touch_sections(t.id);
                if writers.is_empty() {
                    return Err(CertifyFault::ActivationOrder {
                        tensor: t.id,
                        writer: None,
                        reader: 0,
                        reason: "no section writes it",
                    });
                }
                for &w in &writers {
                    for &r in &readers {
                        if r <= w {
                            return Err(CertifyFault::ActivationOrder { tensor: t.id, writer: Some(w), reader: r, reason: "a section reads the activation at or before a section that writes it" });
                        }
                        if !ix().barriers_between(w, r) {
                            return Err(CertifyFault::ActivationOrder { tensor: t.id, writer: Some(w), reader: r, reason: "no barrier chain separates a writing section from the reading one" });
                        }
                    }
                }
            }
            // ★ A CARRY IS WHAT THE STEP UPDATES IN PLACE. It must be written — a state nothing stores is an input —
            // and every access to it is ORDERED against every write: a read against each write (it sees the writes
            // before it and none after), and a write against each other write (the later one's value is the one
            // that stands). One writer is the common case — a cache append, a memory update; several are a table
            // written row by row, a stacked batch, each row by its own section — and the rule is the same pairwise
            // one. A state the step writes but does not read (a row appended to a cache the NEXT step reads) is a
            // carry all the same.
            TensorRole::State => {
                let (readers, writers) = ix().touch_sections(t.id);
                if writers.is_empty() {
                    return Err(CertifyFault::StateOrder {
                        tensor: t.id,
                        reader: readers.first().copied(),
                        writer: None,
                        reason: "a carried tensor the step never writes is not carried",
                    });
                }
                for &w in &writers {
                    for &r in &readers {
                        if r == w {
                            if !ix().read_before_write_in_section(t.id, r) {
                                return Err(CertifyFault::StateOrder {
                                    tensor: t.id,
                                    reader: Some(r),
                                    writer: Some(w),
                                    reason: "one section reads and writes the carried state with nothing ordering the two — the write must be downstream of the read in the section's own dataflow, or the halves must be separate sections",
                                });
                            }
                            continue;
                        }
                        let (a, b) = if r < w { (r, w) } else { (w, r) };
                        if !ix().barriers_between(a, b) {
                            return Err(CertifyFault::StateOrder { tensor: t.id, reader: Some(r), writer: Some(w), reason: "no barrier chain separates the reading section from the writing one" });
                        }
                    }
                    for &other in writers.iter().filter(|o| **o > w) {
                        if !ix().barriers_between(w, other) {
                            return Err(CertifyFault::StateOrder { tensor: t.id, reader: None, writer: Some(other), reason: "no barrier chain separates two sections that write the same carried state" });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    shard_obligation(p)?;
    Ok(())
}

/// ★ THE OBLIGATION TENSOR PARALLELISM ADDS.
///
/// A program is SHARDED when its sites do not all sit on one chip. A row-parallel chip holds a PARTIAL of the
/// contraction, and the partials are the answer only once summed. Unlike a missing barrier — which shows up as a
/// wedge or a stale read — a missing reduce produces a complete, correctly shaped, entirely plausible wrong number,
/// so nothing downstream can catch it.
///
/// ★ THE RULE IS ABOUT DATAFLOW, NOT ABOUT WHICH TENSOR IS THE OUTPUT. A tensor-parallel LAYER reduces its partials
/// in the middle and computes on after — the residual, the norm, the next layer — so "every output is an all-reduce's
/// tensor" is false of every correct layer. What is true of every correct program is this: a value derived from a
/// chip's own tensors (a `chip: Some` tensor — a weight slice, a slice of the carried state, a chip's activation)
/// reaches the REPLICATED spine (`chip: None`) only through an all-reduce that folds every operand it is handed and
/// draws them from EVERY chip of the program. So chip-local-ness flows forward through every stage and buffer; a
/// complete all-reduce is the one stage that ends it; and a chip-local value stored into a replicated tensor is an
/// unreduced shard. What the rule cannot follow — a multicast receive — counts as chip-local, so it refuses rather
/// than passes.
fn shard_obligation(p: &SpatialProgram) -> Result<(), CertifyFault> {
    use std::collections::BTreeSet;
    let chips: BTreeSet<u16> = p
        .stages
        .iter()
        .filter_map(|s| s.site.map(|x| x.chip))
        .collect();
    if chips.len() <= 1 {
        return Ok(());
    }
    // ★ PRESENCE IS NOT ENOUGH. A collective that is THERE but folds only some of its operands, or only some chips'
    // partials, is the same wrong answer as one that is missing — `Add(op0, op1)` over four partials returns half the
    // contraction, and nothing about the program's shape looks off.
    let complete = |s: &Stage| -> bool {
        let StageKind::AllReduce { combine, .. } = &s.kind else {
            return false;
        };
        let folds_all = match combine.as_ref() {
            ComputeOp::Generic { body, .. } => body.max_operand() == s.ins.len().checked_sub(1),
            _ => false,
        };
        let from: BTreeSet<u16> = s
            .ins
            .iter()
            .filter_map(|b| p.buffer(*b).site.map(|x| x.chip))
            .collect();
        folds_all && from == chips
    };
    let chip_local_tensor = |t: TensorId| p.tensor(t).chip.is_some();
    let mut local: BTreeSet<BufferId> = BTreeSet::new();
    loop {
        let before = local.len();
        for s in &p.stages {
            let derived = match &s.kind {
                StageKind::Load { tensor, .. } => {
                    chip_local_tensor(*tensor) || s.ins.iter().any(|b| local.contains(b))
                }
                StageKind::Recv { .. } => true,
                StageKind::AllReduce { .. } => !complete(s),
                StageKind::Store { .. } => false,
                _ => s.ins.iter().any(|b| local.contains(b)),
            };
            if derived {
                local.extend(s.outs.iter().copied());
            }
        }
        if local.len() == before {
            break;
        }
    }
    for s in &p.stages {
        let StageKind::Store { tensor, .. } = &s.kind else {
            continue;
        };
        if !chip_local_tensor(*tensor) && s.ins.first().is_some_and(|b| local.contains(b)) {
            return Err(CertifyFault::UnreducedShard {
                tensor: *tensor,
                chips: chips.len(),
            });
        }
    }
    Ok(())
}

/// Build the certificate for a scheduled program, or refuse.
/// FNV-1a over a graph's serialization — what a per-section proof carries INSTEAD of the graph, so that
/// `recheck` still binds a witness to one exact graph without the certificate having to hold it.
fn graph_digest(g: &MarkedGraph) -> u64 {
    fnv_json(g)
}

/// The places a single op puts a claim on. Only buffers, semaphores and multicast groups become places;
/// a TENSOR does not, which is why activations may cross a section boundary and these may not.
fn op_resources(op: &ChoreoOp, out: &mut Vec<(u8, usize)>) {
    const BUF: u8 = 0;
    const SEM: u8 = 1;
    const GRP: u8 = 2;
    match op {
        ChoreoOp::Reserve { buf, .. }
        | ChoreoOp::Push { buf, .. }
        | ChoreoOp::Wait { buf, .. }
        | ChoreoOp::Pop { buf, .. } => out.push((BUF, buf.idx())),
        ChoreoOp::ReadTiles { into, .. } => out.push((BUF, into.idx())),
        ChoreoOp::WriteTiles { from, .. } => out.push((BUF, from.idx())),
        ChoreoOp::McastSend { buf, group, .. } => {
            out.push((BUF, buf.idx()));
            out.push((GRP, group.idx()));
        }
        ChoreoOp::McastSetValid { sem, group } => {
            out.push((SEM, sem.idx()));
            out.push((GRP, group.idx()));
        }
        ChoreoOp::SemSet { sem, .. }
        | ChoreoOp::SemInc { sem, .. }
        | ChoreoOp::SemWait { sem, .. } => out.push((SEM, sem.idx())),
        ChoreoOp::Compute {
            ins, acc_reload, ..
        } => {
            for b in ins {
                out.push((BUF, b.idx()));
            }
            if let Some(b) = acc_reload {
                out.push((BUF, b.idx()));
            }
        }
        ChoreoOp::PackAcc { to, .. } => out.push((BUF, to.idx())),
        ChoreoOp::Barrier { group } => out.push((GRP, group.idx())),
        _ => {}
    }
}

/// Does this choreography's liveness proof decompose into one proof per section?
///
/// The precondition `section_ranges` checks, exposed so a caller — and the lock that probes it — can
/// see which form a certificate will take and why, instead of inferring it from the certificate's size.
pub fn decomposes_by_section(p: &SpatialProgram) -> bool {
    section_ranges(p).is_some()
}

/// The per-section slice of every engine program, or `None` when the choreography does not decompose.
///
/// Four things must hold, and every one is CHECKED rather than assumed — a wrong `Some` here would prove
/// a program live that deadlocks across a section boundary, which is the one failure this whole structure
/// exists to make impossible:
///   1. every stage belongs to exactly one section, and each top-level op's stages — loop bodies included
///      — all fall in the SAME one; an op straddling a boundary has no section to be proved in;
///   2. each program's ops run in non-decreasing section order, so a section is a contiguous slice;
///   3. no buffer, semaphore or multicast group is touched from two sections;
///   4. a receive buffer and the group that feeds it live in the same section, since the multicast token
///      is produced by the sender's `McastSend` and consumed by the receiver's `Push`.
///
/// Under (3) and (4) the only arcs the cut removes are program-counter arcs, which run forward in section
/// order and can close no cycle.
fn section_ranges(p: &SpatialProgram) -> Option<Vec<Selection>> {
    macro_rules! refuse {
        ($($a:tt)*) => {{
            if std::env::var_os("LOGOS_SECTION_DEBUG").is_some() {
                eprintln!("section_ranges refuses: {}", format!($($a)*));
            }
            return None;
        }};
    }
    let sched = p.schedule.as_ref()?;
    if p.sections.len() < 2 {
        refuse!("only {} section(s)", p.sections.len());
    }
    let mut sec_of_stage = vec![usize::MAX; p.stages.len()];
    for (si, sp) in p.sections.iter().enumerate() {
        let Some(slots) = sec_of_stage.get_mut(sp.stages.0..sp.stages.1.min(p.stages.len())) else {
            refuse!(
                "section {si} span {:?} is out of range of {} stages",
                sp.stages,
                p.stages.len()
            );
        };
        for slot in slots {
            if *slot != usize::MAX {
                refuse!("section {si} overlaps section {} on a stage", *slot);
            }
            *slot = si;
        }
    }
    let mut ranges: Vec<Selection> = vec![Vec::new(); p.sections.len()];
    let mut owner: BTreeMap<(u8, usize), usize> = BTreeMap::new();
    let mut res: Vec<(u8, usize)> = Vec::new();
    for (pi, prog) in sched.programs.iter().enumerate() {
        let mut cur: Option<(usize, usize)> = None;
        for (oi, t) in prog.ops.iter().enumerate() {
            let mut sec: Option<usize> = None;
            let mut straddles = false;
            res.clear();
            walk_tagged(std::slice::from_ref(t), 1, &mut |tt, _| {
                op_resources(&tt.op, &mut res);
                match sec_of_stage.get(tt.stage.idx()).copied() {
                    None | Some(usize::MAX) => straddles = true,
                    Some(x) => match sec {
                        None => sec = Some(x),
                        Some(a) if a != x => straddles = true,
                        Some(_) => {}
                    },
                }
            });
            let Some(sec) = sec else {
                refuse!("program {pi} op {oi} tags no stage")
            };
            if straddles {
                refuse!("program {pi} op {oi} straddles sections (or names a sectionless stage)");
            }
            for r in &res {
                match owner.entry(*r) {
                    std::collections::btree_map::Entry::Vacant(v) => {
                        v.insert(sec);
                    }
                    std::collections::btree_map::Entry::Occupied(o) => {
                        if *o.get() != sec {
                            refuse!("program {pi} op {oi}: resource {:?} is touched from sections {} and {sec}", r, *o.get());
                        }
                    }
                }
            }
            match cur {
                Some((s, _)) if s == sec => {}
                Some((s, st)) => {
                    if sec < s {
                        refuse!(
                            "program {pi} op {oi} (stage {}): section {sec} follows section {s}",
                            t.stage.idx()
                        );
                    }
                    ranges[s].push((pi, st..oi));
                    cur = Some((sec, oi));
                }
                None => cur = Some((sec, oi)),
            }
        }
        if let Some((s, st)) = cur {
            ranges[s].push((pi, st..prog.ops.len()));
        }
    }
    for st in &p.stages {
        if let (StageKind::Recv { group }, Some(o)) = (&st.kind, st.outs.first()) {
            match (owner.get(&(0, o.idx())), owner.get(&(2, group.idx()))) {
                (Some(a), Some(b)) if a != b => {
                    refuse!("recv buffer in section {a} fed by a group in section {b}")
                }
                _ => {}
            }
        }
    }
    Some(ranges)
}

pub fn certify_choreography(p: &SpatialProgram) -> Result<ChoreographyCertificate, CertifyFault> {
    if p.schedule.is_none() {
        return Err(CertifyFault::NotScheduled);
    }
    p.validate().map_err(CertifyFault::Invalid)?;
    ordering_obligations(p)?;
    let proof = build_proof(p)?;
    let trace = eval_choreography_dry(p).map_err(CertifyFault::Eval)?;
    let balances = balances(p, &trace.high_water);
    for bl in &balances {
        if bl.high_water > bl.depth_pages || bl.reserved != bl.pushed || bl.pushed != bl.popped {
            return Err(CertifyFault::Bounds { buffer: bl.buf });
        }
    }
    Ok(ChoreographyCertificate {
        schedule_digest: schedule_digest(p),
        proof,
        balances,
        sem_counts: sem_counts(p),
        compute: compute_certificates(p),
        numerics: p.claim.clone(),
    })
}

/// ★ THE TRIP WINDOW — A FEW COUNTS PROVE EVERY COUNT. A loop run to a trip unrolls to a different graph at each
/// count, and a decode fold at a long context admits thousands of them. The certificate proves the counts
/// `lo ..= min(hi, max(lo, s + 1))`, where `s` bounds how far, in rounds of the trip's loops, one engine can run ahead
/// of another it trades pages with — and that proves them all, by the shape the IR holds a trip loop to (a program
/// input for its count, no semaphore or multicast in its body, no trip loop in another, every buffer it touches
/// balanced per round — `validate_trips` and the per-round counting check):
///   • per-round balance means advancing EVERY engine of a coupled set by one round leaves every buffer's occupancy
///     where it was;
///   • so a stall with every engine of a set strictly inside its rounds cannot be the run's final state — the same
///     stall one round earlier is a smaller final state, and a marked graph has one;
///   • a stall with an engine still at the start of its rounds has every engine of its set within `s` rounds of
///     the start, and is the same stall at every count past `s + 1`: proved at `s + 1`, it is refused there;
///   • a stall with an engine past its rounds has every engine within `s` rounds of the end, and the same stall one
///     round shorter is a stall of the next count down — down to `s + 1`, which is proved.
/// `s` is the most any path of per-round page traffic lets one engine lead another: over each set of engines that
/// trade pages inside the trip's loops, the sum of each such buffer's depth, the pages it moves outside them, and one.
pub fn trip_window(p: &SpatialProgram, trip: TripId) -> std::ops::RangeInclusive<usize> {
    window_of(p.trip(trip), trip_spreads(p)[trip.idx()])
}

/// The lead bound `s` of [`trip_window`] for every trip, in trip order, from ONE walk of the schedule. Engines are
/// COUPLED by a buffer one of them pushes and another pops inside a trip's loops — a page that moves every round; a
/// page the rounds only wait on moves once per item and lets no engine run a round ahead. Each coupled set's bound is
/// the sum over its buffers of depth, pages moved outside the loops, and one; a trip's `s` is its largest set's.
fn trip_spreads(p: &SpatialProgram) -> Vec<usize> {
    let mut out = vec![0usize; p.trips.len()];
    let Some(sched) = p.schedule.as_ref() else {
        return out;
    };
    let n = p.buffers.len();
    let (mut producer, mut consumer) = (vec![None; n], vec![None; n]);
    let mut outside = vec![0usize; n];
    let mut moved_in: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
    struct Walk<'a> {
        prog: usize,
        producer: &'a mut [Option<usize>],
        consumer: &'a mut [Option<usize>],
        moved_in: &'a mut [BTreeSet<usize>],
        outside: &'a mut [usize],
    }
    fn walk(w: &mut Walk, ops: &[TaggedOp], trip: Option<usize>, mult: usize) {
        for t in ops {
            let (buf, pages, produces) = match &t.op {
                ChoreoOp::Loop { var, count, body } => {
                    match count.trip() {
                        Some(id) => walk(w, body, Some(id.idx()), mult),
                        None if *var == crate::spatial::passes::LV_ITEM => {
                            walk(w, body, trip, mult)
                        }
                        None => walk(w, body, trip, mult * count.bound()),
                    }
                    continue;
                }
                ChoreoOp::Push { buf, pages } => (*buf, *pages, true),
                ChoreoOp::Pop { buf, pages } => (*buf, *pages, false),
                _ => continue,
            };
            let i = buf.idx();
            if produces {
                w.producer[i] = Some(w.prog);
            } else {
                w.consumer[i] = Some(w.prog);
            }
            match trip {
                Some(id) => {
                    w.moved_in[i].insert(id);
                }
                None => w.outside[i] += mult * pages,
            }
        }
    }
    for (prog, q) in sched.programs.iter().enumerate() {
        let mut w = Walk {
            prog,
            producer: &mut producer,
            consumer: &mut consumer,
            moved_in: &mut moved_in,
            outside: &mut outside,
        };
        walk(&mut w, &q.ops, None, 1);
    }
    // Per trip, the coupling buffers and a union-find over just the engine programs they join.
    let mut coupling: Vec<Vec<(usize, usize, usize)>> = vec![Vec::new(); p.trips.len()];
    for i in 0..n {
        let (Some(a), Some(b)) = (producer[i], consumer[i]) else {
            continue;
        };
        if a == b {
            continue;
        }
        for &trip in &moved_in[i] {
            coupling[trip].push((a, b, p.buffers[i].kind.depth_pages() + outside[i] + 1));
        }
    }
    fn root(parent: &mut rustc_hash::FxHashMap<usize, usize>, mut x: usize) -> usize {
        while let Some(&up) = parent.get(&x).filter(|up| **up != x) {
            let grand = parent.get(&up).copied().unwrap_or(up);
            parent.insert(x, grand);
            x = up;
        }
        x
    }
    for (trip, edges) in coupling.iter().enumerate() {
        let mut parent: rustc_hash::FxHashMap<usize, usize> = rustc_hash::FxHashMap::default();
        for (a, b, _) in edges {
            let (ra, rb) = (root(&mut parent, *a), root(&mut parent, *b));
            if ra != rb {
                parent.insert(ra, rb);
            }
        }
        let mut per_set: rustc_hash::FxHashMap<usize, usize> = rustc_hash::FxHashMap::default();
        for (a, _, lead) in edges {
            *per_set.entry(root(&mut parent, *a)).or_default() += lead;
        }
        out[trip] = per_set.values().copied().max().unwrap_or(0);
    }
    out
}

/// The window of a trip whose lead bound is `spread`.
fn window_of(t: &Trip, spread: usize) -> std::ops::RangeInclusive<usize> {
    t.lo()..=t.hi().min(t.lo().max(spread + 1))
}

/// The trips the loops of a part of the schedule run to — `sel`'s ops, or every op — in id order.
fn part_trips(p: &SpatialProgram, sel: Option<&[(usize, std::ops::Range<usize>)]>) -> Vec<TripId> {
    let Some(sched) = p.schedule.as_ref() else {
        return Vec::new();
    };
    let mut found = std::collections::BTreeSet::new();
    let mut visit = |ops: &[TaggedOp]| {
        walk_tagged(ops, 1, &mut |t, _| {
            if let ChoreoOp::Loop { count, .. } = &t.op {
                found.extend(count.trip());
            }
        })
    };
    match sel {
        Some(r) => r
            .iter()
            .for_each(|(pi, ops)| visit(&sched.programs[*pi].ops[ops.clone()])),
        None => sched.programs.iter().for_each(|q| visit(&q.ops)),
    }
    found.into_iter().collect()
}

/// The most cases one part's proof may hold: the windows of the trips it runs to, multiplied.
const TRIP_CASES_LIMIT: usize = 4096;

/// Every count a part must be proved at: each trip it runs to at every count of its window, the rest at their
/// greatest — the part's counts (its trips in id order) beside the whole program's.
fn part_cases(
    p: &SpatialProgram,
    cx: &GraphContext,
    sel: Option<&[(usize, std::ops::Range<usize>)]>,
) -> Result<Vec<(Vec<usize>, Vec<usize>)>, CertifyFault> {
    let trips = part_trips(p, sel);
    if trips.is_empty() {
        return Ok(Vec::new());
    }
    let windows: Vec<Vec<usize>> = trips
        .iter()
        .map(|t| window_of(p.trip(*t), cx.spreads[t.idx()]).collect())
        .collect();
    let cases = windows
        .iter()
        .try_fold(1usize, |n, w| n.checked_mul(w.len()))
        .filter(|n| *n <= TRIP_CASES_LIMIT);
    let Some(cases) = cases else {
        return Err(CertifyFault::TripCasesExceed {
            limit: TRIP_CASES_LIMIT,
        });
    };
    let bound = p.bound_counts();
    Ok((0..cases)
        .map(|mut k| {
            let mut full = bound.clone();
            let mut own = vec![0; trips.len()];
            for (i, w) in windows.iter().enumerate().rev() {
                own[i] = w[k % w.len()];
                k /= w.len();
                full[trips[i].idx()] = own[i];
            }
            (own, full)
        })
        .collect())
}

/// Prove a part at every count of its trips' windows, adding each witness to the table.
fn prove_cases(
    p: &SpatialProgram,
    cx: &GraphContext,
    sel: Option<&[(usize, std::ops::Range<usize>)]>,
    witnesses: &mut Vec<LivenessWitness>,
    index_of: &mut BTreeMap<Vec<usize>, usize>,
) -> Result<Vec<TripCase>, CertifyFault> {
    let mut out = Vec::new();
    for (counts, full) in part_cases(p, cx, sel)? {
        let g = build_graph_over(p, cx, sel, &full)?;
        let liveness = liveness_witness(&g)?;
        let witness = *index_of
            .entry(liveness.topo_order.clone())
            .or_insert_with(|| {
                witnesses.push(liveness);
                witnesses.len() - 1
            });
        out.push(TripCase {
            counts,
            places: g.places.len(),
            transitions: g.transitions.len(),
            digest: graph_digest(&g),
            witness,
        });
    }
    Ok(out)
}

/// The carried cases of a part are exactly the counts its trips' windows require, in order, and each one's rebuilt
/// graph has its shape, its digest, and an order in the table that the graph verifies.
fn recheck_cases(
    p: &SpatialProgram,
    cx: &GraphContext,
    sel: Option<&[(usize, std::ops::Range<usize>)]>,
    cases: &[TripCase],
    witnesses: &[LivenessWitness],
) -> bool {
    let Ok(want) = part_cases(p, cx, sel) else {
        return false;
    };
    if want.len() != cases.len() {
        return false;
    }
    want.iter().zip(cases).all(|((counts, full), case)| {
        if *counts != case.counts {
            return false;
        }
        let Ok(g) = build_graph_over(p, cx, sel, full) else {
            return false;
        };
        let Some(w) = witnesses.get(case.witness) else {
            return false;
        };
        g.places.len() == case.places
            && g.transitions.len() == case.transitions
            && graph_digest(&g) == case.digest
            && verify_witness(&g, w)
    })
}

/// Prove liveness in whichever form the choreography admits: one section at a time when it decomposes,
/// whole otherwise. Each section's graph is dropped the moment its witness exists — holding one section
/// instead of the sum of them is the entire point.
fn build_proof(p: &SpatialProgram) -> Result<GraphProof, CertifyFault> {
    let cx = GraphContext::of(p);
    let bound = p.bound_counts();
    let Some(ranges) = section_ranges(p) else {
        let graph = build_graph_over(p, &cx, None, &bound)?;
        let liveness = liveness_witness(&graph)?;
        let (mut case_witnesses, mut index_of) = (Vec::new(), BTreeMap::new());
        let cases = prove_cases(p, &cx, None, &mut case_witnesses, &mut index_of)?;
        return Ok(GraphProof::Whole {
            graph,
            liveness,
            cases,
            case_witnesses,
        });
    };
    let mut sections = Vec::with_capacity(ranges.len());
    let mut witnesses: Vec<LivenessWitness> = Vec::new();
    let mut index_of: BTreeMap<Vec<usize>, usize> = BTreeMap::new();
    let (mut places, mut transitions) = (0usize, 0usize);
    for (si, r) in ranges.iter().enumerate() {
        let g = build_graph_over(p, &cx, Some(r), &bound)?;
        let liveness = liveness_witness(&g)?;
        places += g.places.len();
        transitions += g.transitions.len();
        let witness = *index_of
            .entry(liveness.topo_order.clone())
            .or_insert_with(|| {
                witnesses.push(liveness);
                witnesses.len() - 1
            });
        let (digest, section_places, section_transitions) =
            (graph_digest(&g), g.places.len(), g.transitions.len());
        drop(g);
        let cases = prove_cases(p, &cx, Some(r), &mut witnesses, &mut index_of)?;
        sections.push(SectionProof {
            section: si,
            places: section_places,
            transitions: section_transitions,
            digest,
            witness,
            cases,
        });
    }
    Ok(GraphProof::PerSection {
        sections,
        witnesses,
        places,
        transitions,
    })
}

/// Why a certificate does not recheck against a program: the first of its claims the program does not bear out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecheckFault {
    /// The program is unscheduled (`program: None`), or its schedule's digest is not the one the certificate binds.
    Digest { carried: u64, program: Option<u64> },
    /// The program breaks an invariant of the IR or one of its ordering obligations.
    Program(String),
    /// The carried liveness proof does not hold of the program's schedule.
    Liveness,
    /// The buffer balances are not the program's, or one overflows or does not balance.
    Balances,
    /// The semaphore counts are not the program's.
    Semaphores,
    /// The numerics claim is not the program's.
    Numerics,
    /// The compute certificates do not name the program's compute stages, in order.
    ComputeStages,
    /// A stage's SAT certificate carries a formula that is not the stage's obligation as the program rebuilds it.
    ForeignFormula { stage: usize, name: String, why: String },
    /// A stage's SAT certificate does not refute the formula it carries.
    Refutation { stage: usize, name: String },
}

impl std::fmt::Display for RecheckFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecheckFault::Digest { carried, program: Some(p) } => {
                write!(f, "the certificate binds schedule {carried:016x}, and the program's is {p:016x}")
            }
            RecheckFault::Digest { carried, program: None } => {
                write!(f, "the certificate binds schedule {carried:016x}, and the program is unscheduled")
            }
            RecheckFault::Program(why) => write!(f, "the program does not hold: {why}"),
            RecheckFault::Liveness => f.write_str("the liveness proof does not hold of the program's schedule"),
            RecheckFault::Balances => f.write_str("the buffer balances are not the program's, or do not balance"),
            RecheckFault::Semaphores => f.write_str("the semaphore counts are not the program's"),
            RecheckFault::Numerics => f.write_str("the numerics claim is not the program's"),
            RecheckFault::ComputeStages => f.write_str("the compute certificates do not name the program's compute stages in order"),
            RecheckFault::ForeignFormula { stage, name, why } => write!(
                f,
                "stage {stage} `{name}`: its SAT certificate refutes a formula that is not the stage's obligation ({why})"
            ),
            RecheckFault::Refutation { stage, name } => {
                write!(f, "stage {stage} `{name}`: its SAT certificate does not refute the formula it carries")
            }
        }
    }
}

impl ChoreographyCertificate {
    /// Independent re-check: the digest binds the certificate to `p`; the graph is rebuilt from `p`'s schedule
    /// and compared; the witness is verified against the token-free arcs; balances and semaphore counts are
    /// recomputed; every SAT certificate's formula is held to its stage's obligation rebuilt from `p`, and its
    /// refutation re-run through the PR/RUP checker.
    pub fn recheck(&self, p: &SpatialProgram) -> bool {
        self.checked(p).is_ok()
    }

    /// [`Self::recheck`], the first claim `p` does not bear out named.
    pub fn checked(&self, p: &SpatialProgram) -> Result<(), RecheckFault> {
        match p.schedule.as_ref().map(|_| schedule_digest(p)) {
            Some(d) if d == self.schedule_digest => {}
            program => return Err(RecheckFault::Digest { carried: self.schedule_digest, program }),
        }
        p.validate().map_err(|e| RecheckFault::Program(format!("{e:?}")))?;
        ordering_obligations(p).map_err(|e| RecheckFault::Program(format!("{e:?}")))?;
        if !self.recheck_proof(p) {
            return Err(RecheckFault::Liveness);
        }
        let hw: BTreeMap<BufferId, usize> = self
            .balances
            .iter()
            .map(|b| (b.buf, b.high_water))
            .collect();
        if balances(p, &hw) != self.balances
            || self
                .balances
                .iter()
                .any(|b| b.high_water > b.depth_pages || b.reserved != b.pushed || b.pushed != b.popped)
        {
            return Err(RecheckFault::Balances);
        }
        if sem_counts(p) != self.sem_counts {
            return Err(RecheckFault::Semaphores);
        }
        // ★ A CERTIFICATE STATES WHAT IT CERTIFIES. Its numerics claim is the program's, and its compute certificates
        // name exactly the program's compute stages, in order — a `Sat` one rechecked below, an `Uncertified` one a claim
        // of less than a derivation might make, never of more.
        if self.numerics != p.claim {
            return Err(RecheckFault::Numerics);
        }
        if self.compute.iter().map(|(s, _)| *s).ne(p
            .stages
            .iter()
            .filter(|s| has_compute_certificate(s))
            .map(|s| s.id))
        {
            return Err(RecheckFault::ComputeStages);
        }
        // ★ A REFUTATION PROVES ONLY THE FORMULA IT REFUTES. A SAT certificate carries its formula, and its replay says
        // that formula is unsatisfiable, whatever formula it is; it proves the stage only when the formula is the stage's
        // obligation. So every obligation is rebuilt here from the stage — once for stages alike — and a carried formula
        // that is not it is refused before its refutation is read.
        let mut rebuilt: BTreeMap<String, Result<Obligation, String>> = BTreeMap::new();
        for (id, c) in &self.compute {
            let ComputeStageCert::Sat(pc) = c else { continue };
            let s = &p.stages[id.idx()];
            let foreign = |why: String| RecheckFault::ForeignFormula { stage: id.idx(), name: s.name.clone(), why };
            let key = compute_key(s).expect("a stage with a compute certificate has a key");
            match rebuilt.entry(key).or_insert_with(|| compute_obligation(s)) {
                Err(why) => return Err(foreign(format!("the stage has no SAT obligation: {why}"))),
                Ok(obligation) if !pc.carries(obligation) => {
                    return Err(foreign(format!(
                        "the formula it carries, over {} variables, is not the obligation rebuilt from the stage, {} clauses over {} variables",
                        pc.num_vars(),
                        obligation.clauses(),
                        obligation.num_vars()
                    )))
                }
                Ok(_) => {}
            }
            if !pc.recheck() {
                return Err(RecheckFault::Refutation { stage: id.idx(), name: s.name.clone() });
            }
        }
        Ok(())
    }

    /// Re-derive the liveness proof from `p` and check the carried one against it.
    ///
    /// The whole form is compared structurally, graph and all, exactly as it always was. The per-section
    /// form rebuilds each section's graph, matches its shape and its digest, and verifies that section's
    /// witness against it — the digest standing in for the graph comparison, and the witness check being
    /// the same Commoner argument restated once per section.
    fn recheck_proof(&self, p: &SpatialProgram) -> bool {
        match &self.proof {
            GraphProof::Whole {
                graph,
                liveness,
                cases,
                case_witnesses,
            } => {
                let Ok(rebuilt) = build_marked_graph(p) else {
                    return false;
                };
                rebuilt == *graph
                    && verify_witness(graph, liveness)
                    && recheck_cases(p, &GraphContext::of(p), None, cases, case_witnesses)
            }
            GraphProof::PerSection {
                sections,
                witnesses,
                places,
                transitions,
            } => {
                let Some(ranges) = section_ranges(p) else {
                    return false;
                };
                if ranges.len() != sections.len() {
                    return false;
                }
                let cx = GraphContext::of(p);
                let (mut tp, mut tt) = (0usize, 0usize);
                for (si, sp) in sections.iter().enumerate() {
                    if sp.section != si {
                        return false;
                    }
                    let Ok(g) = build_graph_over(p, &cx, Some(&ranges[si]), &p.bound_counts())
                    else {
                        return false;
                    };
                    if g.places.len() != sp.places || g.transitions.len() != sp.transitions {
                        return false;
                    }
                    let Some(w) = witnesses.get(sp.witness) else {
                        return false;
                    };
                    if graph_digest(&g) != sp.digest || !verify_witness(&g, w) {
                        return false;
                    }
                    if !recheck_cases(p, &cx, Some(&ranges[si]), &sp.cases, witnesses) {
                        return false;
                    }
                    tp += sp.places;
                    tt += sp.transitions;
                }
                tp == *places && tt == *transitions
            }
        }
    }

    /// A tampered copy: the liveness witness permuted so it is no longer a topological order — must fail
    /// `recheck`. This is the probe that proves the witness is load-bearing rather than decorative, so it
    /// has to bite in BOTH proof forms.
    ///
    /// Whole form: swap one real producer/consumer pair, the most surgical break there is. Per-section
    /// form: reverse the first section long enough to have an arc — the graph is not carried there, so a
    /// pair cannot be picked out, and reversing violates EVERY token-free arc at once instead of one.
    #[must_use]
    pub fn with_witness_tampered(&self) -> Self {
        let mut c = self.clone();
        match &mut c.proof {
            GraphProof::Whole {
                graph, liveness, ..
            } => {
                let n = liveness.topo_order.len();
                if n >= 2 {
                    let g: &MarkedGraph = graph;
                    let mut swapped = false;
                    for t in 0..n {
                        for pl in &g.transitions[t].consumes {
                            if g.places[*pl].marking == 0 {
                                if let Some(pr) =
                                    g.transitions.iter().position(|x| x.produces.contains(pl))
                                {
                                    let (ip, ic) = (
                                        liveness.topo_order.iter().position(|x| *x == pr).unwrap(),
                                        liveness.topo_order.iter().position(|x| *x == t).unwrap(),
                                    );
                                    liveness.topo_order.swap(ip, ic);
                                    swapped = true;
                                    break;
                                }
                            }
                        }
                        if swapped {
                            break;
                        }
                    }
                    if !swapped {
                        liveness.topo_order.swap(0, 1);
                    }
                }
            }
            GraphProof::PerSection { witnesses, .. } => {
                if let Some(w) = witnesses.iter_mut().find(|w| w.topo_order.len() >= 2) {
                    w.topo_order.reverse();
                }
            }
        }
        c
    }
}

/// Import-and-certify at the spatial level: an EXTERNAL placement+schedule of the SAME computation as
/// `source` earns a certificate iff it computes the same thing (structure and evaluation) and its
/// choreography certifies. `None` is fail-closed.
pub fn certify_external_spatial(
    external: &SpatialProgram,
    source: &SpatialProgram,
) -> Option<ChoreographyCertificate> {
    if external.tensors != source.tensors
        || external.work.k_tiles != source.work.k_tiles
        || external.numerics != source.numerics
    {
        return None;
    }
    let kinds = |p: &SpatialProgram| -> Vec<String> {
        let mut v: Vec<String> = p
            .stages
            .iter()
            .map(|s| match &s.kind {
                StageKind::Load { tensor, .. } | StageKind::Store { tensor, .. } => {
                    format!("{:?}:{}", std::mem::discriminant(&s.kind), tensor.idx())
                }
                StageKind::Recv { .. } => "load".into(),
                StageKind::AllReduce { pool, tensor, .. } => {
                    format!("allreduce:{pool:?}:{}", tensor.idx())
                }
                StageKind::Compute(op) => format!("{:?}", std::mem::discriminant(op)),
                StageKind::Scalar(b) => {
                    format!("scalar:{}", serde_json::to_string(b).unwrap_or_default())
                }
            })
            .collect();
        v.sort();
        v.dedup();
        v
    };
    if kinds(external) != kinds(source) {
        return None;
    }
    let inputs = dry_inputs(source);
    let (a, _) = eval_choreography_traced(external, &inputs).ok()?;
    let (b, _) = eval_choreography_traced(source, &inputs).ok()?;
    if a.tensors != b.tensors {
        return None;
    }
    let mut ext = external.clone();
    ext.certificate = None;
    certify_choreography(&ext).ok()
}

/// A precision's role in a PE certificate (kept for lowerings that report it).
pub fn certifiable_operand(p: Precision) -> bool {
    precision_is_int(p)
}
