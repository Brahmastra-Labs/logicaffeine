//! `compose` — N verified sections become ONE `SpatialProgram`: the persistent-decoder form.
//!
//! ★ THE UNIT OF LOWERING IS A SECTION; THE UNIT OF DEPLOYMENT IS A PROGRAM. Each section arrives here
//! already tiled, placed, sized, scheduled and verified on its own — which is what makes each one
//! checkable — and this pass joins them without re-deriving any of it: ids are offset, tensors are
//! unified BY NAME, engine programs are concatenated per `(site, engine)` in section order, and every
//! boundary gets a rendezvous every engine passes through. What was `Output` in one section and `Input`
//! in a later one becomes an `Activation`; what the front end named as carried becomes `State`.
//!
//! ★ WHAT THE BARRIER BUYS, AND WHY NOTHING CHEAPER WORKS. Concatenating per engine already orders a
//! section against the next ON THAT ENGINE. It says nothing across engines: the movement engine that
//! loads section k+1's operand runs concurrently with the one that stores section k's result, and
//! without a rendezvous it will read the tensor before it is written. For an activation that is a
//! `ReadOutOfRange` — loud. For carried state it is last token's value — silent, and plausible. So the
//! barrier is not an optimization knob; it is the thing that makes the composition mean anything.
//!
//! ★ THE RENDEZVOUS IS A GATHER AND A RELEASE. Every one of the `P` participating programs posts one
//! arrival at the root (the first participant in site order); the root waits for all `P`, posts one
//! release at each of the `S` participating sites, and every program waits for its own site's release.
//! That is `2P + S + 1` ops and `P + S` marked-graph places per boundary, where posting every arrival at
//! every site would be `P·S` of each — the difference between a real-width layer's certificate being
//! its sections' size and being ten times it. `spatial_rendezvous_lock` pins the shape and breaks each
//! of its ops to show the IR refuses the result.

use std::collections::{BTreeMap, BTreeSet};

use super::recognize_stages::Link;
use super::SpatialReject;
use crate::codegen_sva::fabric_target::Precision;
use crate::spatial::ir::*;
use crate::spatial::target::{precision_bytes, precision_is_int, SpatialTarget};

/// The wider of two floats when one holds every value the other does: fp32 over bf16 or fp16. Two 16-bit floats of
/// different kinds, or an integer, have no wider.
fn wider_float(a: Precision, b: Precision) -> Option<Precision> {
    if precision_is_int(a) || precision_is_int(b) {
        return None;
    }
    match precision_bytes(a).cmp(&precision_bytes(b)) {
        std::cmp::Ordering::Greater => Some(a),
        std::cmp::Ordering::Less => Some(b),
        std::cmp::Ordering::Equal => None,
    }
}

/// Where the composer refuses — always by name, never by dropping a section.
#[derive(Clone, Debug, PartialEq)]
pub enum ComposeReject {
    /// Nothing to compose.
    NoSections,
    /// Two sections give one tensor two different shapes or element types. Composition is only
    /// meaningful when producer and consumer agree about where element `p` lives.
    TensorShapeConflict {
        name: String,
        first: (usize, usize, Precision),
        second: (usize, usize, Precision),
    },
    /// A section's schedule is missing — `compose` joins VERIFIED sections, never templates.
    SectionNotScheduled { section: usize },
    /// The joined program breaks a structural invariant.
    Invalid(IrViolation),
    /// A pass refused the joined program.
    Reject(Box<SpatialReject>),
    /// A phase's sections share a site and, laid out one past another, need more of its memory than it has.
    PhaseOverflowsMemory { bytes: u32, capacity: usize },
}

impl From<IrViolation> for ComposeReject {
    fn from(v: IrViolation) -> Self {
        ComposeReject::Invalid(v)
    }
}

impl From<SpatialReject> for ComposeReject {
    fn from(v: SpatialReject) -> Self {
        ComposeReject::Reject(Box::new(v))
    }
}

/// How the sections are spread over the mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionPlacement {
    /// Every section keeps the sites its own placement chose — sections run in sequence and REUSE the
    /// same local memory, which is the persistent-decoder form and the only one that fits a hundred-op
    /// forward in one site's L1.
    Stacked,
    /// Section `k` is shifted onto mesh site `k mod |mesh|`, so a mesh with room holds several sections
    /// at once. Still ordered by the same barriers; this only changes WHERE, never WHEN.
    RoundRobin,
}

/// The semaphores a section boundary adds: its rendezvous' gather and release.
const RENDEZVOUS_SEMS: usize = 2;

/// The id offsets one section is shifted by.
struct Shift {
    tensor: Vec<TensorId>,
    stage: usize,
    buffer: usize,
    edge: usize,
    sem: usize,
    item: usize,
    group: usize,
    trip: usize,
}

impl Shift {
    fn t(&self, id: TensorId) -> TensorId {
        self.tensor[id.idx()]
    }
    fn s(&self, id: StageId) -> StageId {
        StageId::new(self.stage + id.idx())
    }
    fn b(&self, id: BufferId) -> BufferId {
        BufferId::new(self.buffer + id.idx())
    }
    fn e(&self, id: EdgeId) -> EdgeId {
        EdgeId::new(self.edge + id.idx())
    }
    fn m(&self, id: SemId) -> SemId {
        SemId::new(self.sem + id.idx())
    }
    fn i(&self, id: WorkItemId) -> WorkItemId {
        WorkItemId::new(self.item + id.idx())
    }
    fn g(&self, id: GroupId) -> GroupId {
        GroupId::new(self.group + id.idx())
    }
    fn r(&self, id: TripId) -> TripId {
        TripId::new(self.trip + id.idx())
    }
    fn count(&self, c: LoopCount) -> LoopCount {
        match c {
            LoopCount::Const(n) => LoopCount::Const(n),
            LoopCount::Trip { trip, max } => LoopCount::Trip {
                trip: self.r(trip),
                max,
            },
        }
    }
    fn addr(&self, a: &TileAddr) -> TileAddr {
        TileAddr {
            base: a.base,
            steps: a.steps.clone(),
            indirect: a.indirect.map(|i| IndirectTile {
                index: self.b(i.index),
                ..i
            }),
        }
    }
    fn pattern(&self, a: &AccessPattern) -> AccessPattern {
        AccessPattern {
            origin: self.addr(&a.origin),
            ..a.clone()
        }
    }
    fn compute(&self, op: &ComputeOp) -> ComputeOp {
        match op {
            ComputeOp::MatmulTiles { interm, .. } => {
                let mut o = op.clone();
                if let ComputeOp::MatmulTiles { interm: i, .. } = &mut o {
                    *i = interm.map(|b| self.b(b));
                }
                o
            }
            ComputeOp::Bias { bias } => ComputeOp::Bias {
                bias: self.b(*bias),
            },
            other => other.clone(),
        }
    }
    fn handshake(&self, h: &Handshake) -> Handshake {
        match h {
            Handshake::Semaphore {
                ready,
                valid,
                ready_count,
            } => Handshake::Semaphore {
                ready: self.m(*ready),
                valid: self.m(*valid),
                ready_count: *ready_count,
            },
            Handshake::Barrier { group } => Handshake::Barrier {
                group: self.g(*group),
            },
            other => other.clone(),
        }
    }
    fn kind(&self, k: &StageKind) -> StageKind {
        match k {
            StageKind::Load {
                tensor,
                pattern,
                num_blocks,
                multicast,
            } => StageKind::Load {
                tensor: self.t(*tensor),
                pattern: self.pattern(pattern),
                num_blocks: *num_blocks,
                multicast: multicast.map(|g| self.g(g)),
            },
            StageKind::Store {
                tensor,
                pattern,
                sub_blocks,
            } => StageKind::Store {
                tensor: self.t(*tensor),
                pattern: self.pattern(pattern),
                sub_blocks: *sub_blocks,
            },
            StageKind::Recv { group } => StageKind::Recv {
                group: self.g(*group),
            },
            StageKind::AllReduce {
                pool,
                tensor,
                combine,
            } => StageKind::AllReduce {
                pool: *pool,
                tensor: self.t(*tensor),
                combine: Box::new(self.compute(combine)),
            },
            StageKind::Compute(op) => StageKind::Compute(self.compute(op)),
            StageKind::Scalar(b) => StageKind::Scalar(b.clone()),
        }
    }
    /// Every id a choreography op names — the one place a missed field would silently point a composed
    /// section at another section's buffer.
    fn op(&self, o: &ChoreoOp) -> ChoreoOp {
        match o {
            ChoreoOp::Reserve { buf, pages } => ChoreoOp::Reserve {
                buf: self.b(*buf),
                pages: *pages,
            },
            ChoreoOp::Push { buf, pages } => ChoreoOp::Push {
                buf: self.b(*buf),
                pages: *pages,
            },
            ChoreoOp::Wait { buf, pages } => ChoreoOp::Wait {
                buf: self.b(*buf),
                pages: *pages,
            },
            ChoreoOp::Pop { buf, pages } => ChoreoOp::Pop {
                buf: self.b(*buf),
                pages: *pages,
            },
            ChoreoOp::ReadTiles {
                tensor,
                at,
                block_h,
                block_w,
                stride_w,
                stride_h,
                into,
            } => ChoreoOp::ReadTiles {
                tensor: self.t(*tensor),
                at: self.addr(at),
                block_h: *block_h,
                block_w: *block_w,
                stride_w: *stride_w,
                stride_h: *stride_h,
                into: self.b(*into),
            },
            ChoreoOp::WriteTiles {
                from,
                tensor,
                at,
                block_h,
                block_w,
                stride_w,
                stride_h,
            } => ChoreoOp::WriteTiles {
                from: self.b(*from),
                tensor: self.t(*tensor),
                at: self.addr(at),
                block_h: *block_h,
                block_w: *block_w,
                stride_w: *stride_w,
                stride_h: *stride_h,
            },
            ChoreoOp::McastSend {
                buf,
                pages,
                group,
                hold_path,
            } => ChoreoOp::McastSend {
                buf: self.b(*buf),
                pages: *pages,
                group: self.g(*group),
                hold_path: *hold_path,
            },
            ChoreoOp::McastSetValid { sem, group } => ChoreoOp::McastSetValid {
                sem: self.m(*sem),
                group: self.g(*group),
            },
            ChoreoOp::SemSet { sem, value } => ChoreoOp::SemSet {
                sem: self.m(*sem),
                value: *value,
            },
            ChoreoOp::SemInc { sem, at, by } => ChoreoOp::SemInc {
                sem: self.m(*sem),
                at: *at,
                by: *by,
            },
            ChoreoOp::SemWait { sem, until } => ChoreoOp::SemWait {
                sem: self.m(*sem),
                until: *until,
            },
            ChoreoOp::Compute {
                op,
                ins,
                acc_reload,
            } => ChoreoOp::Compute {
                op: self.compute(op),
                ins: ins.iter().map(|b| self.b(*b)).collect(),
                acc_reload: acc_reload.map(|b| self.b(b)),
            },
            ChoreoOp::PackAcc {
                to,
                tiles,
                convert,
                accumulate,
            } => ChoreoOp::PackAcc {
                to: self.b(*to),
                tiles: *tiles,
                convert: *convert,
                accumulate: *accumulate,
            },
            ChoreoOp::Barrier { group } => ChoreoOp::Barrier {
                group: self.g(*group),
            },
            ChoreoOp::Loop { var, count, body } => ChoreoOp::Loop {
                var: *var,
                count: self.count(*count),
                body: body.iter().map(|t| self.tagged(t)).collect(),
            },
            other => other.clone(),
        }
    }
    fn tagged(&self, t: &TaggedOp) -> TaggedOp {
        TaggedOp {
            stage: self.s(t.stage),
            op: self.op(&t.op),
        }
    }
}

/// The role a merged tensor takes: what every section does with it, reconciled.
#[derive(Default)]
struct Usage {
    written_in: Vec<usize>,
    read_in: Vec<usize>,
    declared: Option<TensorRole>,
}

/// ★ A CONSTANT TWO SECTIONS DECLARE DIFFERENTLY IS TWO CONSTANTS. A pass names a table it generates after the stage that
/// reads it (a column block's first columns and bounds, `write_row__columns`), so two sections of one program can name
/// different tables alike — two cache writes of different widths, two rotations of different head counts. The
/// composition unifies tensors by name, which would hand one section the other's table, or refuse the program for
/// their shapes; a compile-time parameter declared again with other contents is renamed for its section instead
/// (`…__s{k}`). One declared alike stays one tensor, and so does every tensor bound at run time — a weight included.
fn distinct_constants(sections: &mut [SpatialProgram]) {
    let mut first: BTreeMap<String, (ElemShape, TensorInit)> = BTreeMap::new();
    for (k, sec) in sections.iter_mut().enumerate() {
        for td in &mut sec.tensors {
            // A weight is a parameter too, declared `ConstF(0)` until its file binds it: only a table of values is a
            // section's own. Two widths of one table are one tensor, held at the wider (below).
            let constant = td.role == TensorRole::Param
                && matches!(td.init, TensorInit::Values(_) | TensorInit::FloatValues(_) | TensorInit::Pattern { .. });
            match first.get(&td.name) {
                None => {
                    first.insert(td.name.clone(), (td.elems, td.init.clone()));
                }
                Some((elems, init)) if constant && (*elems != td.elems || *init != td.init) => {
                    td.name = format!("{}__s{k}", td.name);
                }
                Some(_) => {}
            }
        }
    }
}

/// Join verified sections into one program.
///
/// `links` is the front end's dataflow (advisory — the roles are derived from what the SECTIONS do, so
/// the sub-sections an op expands into are handled by the same rule as the statements themselves), and
/// `state` names the tensors the step carries across tokens.
pub fn compose(
    sections: Vec<SpatialProgram>,
    links: &[Link],
    state: &[String],
    placement: SectionPlacement,
    target: &SpatialTarget,
) -> Result<SpatialProgram, ComposeReject> {
    compose_phased(sections, links, state, placement, None, target)
}

/// [`compose`] with the barriers named: `barrier_after[k]` closes section `k` with a rendezvous, and a section left
/// open runs CONCURRENTLY with the one after it — sections on disjoint chips with no data between them, a phase of a
/// tensor-parallel step. `None` closes every section but the last.
///
/// ★ ONE BARRIER ORDERS EVERYTHING ACROSS IT. A rendezvous after section `b` holds every op of every section after
/// `b` until every op of every section up to `b` is done, so two sections are ordered by ANY barrier between them —
/// which is what the certificate's ordering obligations check — and a boundary with no data across it needs none.
pub fn compose_phased(
    sections: Vec<SpatialProgram>,
    links: &[Link],
    state: &[String],
    placement: SectionPlacement,
    barrier_after: Option<&[bool]>,
    target: &SpatialTarget,
) -> Result<SpatialProgram, ComposeReject> {
    let _ = links;
    let mut sections = sections;
    distinct_constants(&mut sections);
    let closes = |k: usize| {
        k + 1 < sections.len() && barrier_after.is_none_or(|b| b.get(k).copied().unwrap_or(true))
    };
    if sections.is_empty() {
        return Err(ComposeReject::NoSections);
    }
    for (k, s) in sections.iter().enumerate() {
        if s.schedule.is_none() || s.placement.is_none() || s.memory.is_none() {
            return Err(ComposeReject::SectionNotScheduled { section: k });
        }
    }

    // ── 1. Unify the tensors BY NAME and record what each section does with each.
    //
    // ★ A MERGED TENSOR HOLDS WHAT ITS PRODUCER WROTE. Two sections can legitimately DECLARE the same
    // activation at different element types — a matmul writes an fp32 result from its fp32
    // accumulators, and the bf16 elementwise datapath that reads it next declares the operand it can
    // take. Only one of those is a fact about the bytes in memory: the writer's. So the producing
    // section's declaration wins, the reader narrows on the way in (`ReadTiles` rounds to the buffer it
    // lands in, exactly as the hardware does), and a disagreement with NO producer to settle it — or
    // between two producers — is still refused by name, because then nothing decides.
    let producer_decl = {
        let mut chosen: BTreeMap<String, (TensorDecl, bool)> = BTreeMap::new();
        for sec in sections.iter() {
            let stored: Vec<TensorId> = sec
                .stages
                .iter()
                .filter_map(|s| match &s.kind {
                    StageKind::Store { tensor, .. } => Some(*tensor),
                    _ => None,
                })
                .collect();
            for td in &sec.tensors {
                let writes = stored.contains(&td.id);
                match chosen.get_mut(&td.name) {
                    None => {
                        chosen.insert(td.name.clone(), (td.clone(), writes));
                    }
                    Some((have, had_writer)) => {
                        if have.elems != td.elems {
                            return Err(ComposeReject::TensorShapeConflict {
                                name: td.name.clone(),
                                first: (have.elems.rows, have.elems.cols, have.dtype),
                                second: (td.elems.rows, td.elems.cols, td.dtype),
                            });
                        }
                        if writes && !*had_writer {
                            // ★ THE PRODUCER DECIDES THE BYTES, NOT THE STARTING VALUE. A section always
                            // declares its own output `Zero` — that is a placeholder for "this section
                            // fills it", not a claim about what the tensor holds when the program starts.
                            // For a CARRIED STATE that distinction is the whole thing: the GDN memory, the
                            // conv ring and the KV cache are read before they are written, and their
                            // initial value is a fact only the SOURCE declaration knows. Letting the
                            // writer's placeholder win silently zeroed all twelve of Qwen3.8's state
                            // tensors, which a single forward pass hides inside its tolerance and a decode
                            // loop would not.
                            let keep_init = matches!(td.init, TensorInit::Zero)
                                && !matches!(have.init, TensorInit::Zero);
                            let prior = have.init.clone();
                            *have = td.clone();
                            if keep_init {
                                have.init = prior;
                            }
                            *had_writer = true;
                        } else if have.dtype != td.dtype && (writes == *had_writer) {
                            // ★ READERS ALONE MAY DISAGREE IN A FLOAT'S WIDTH. Nothing in the program writes the
                            // tensor and one reader takes it wider than another — an fp32 island beside the bf16
                            // datapath. It is held at the wider width, what the bytes from outside can carry, and
                            // the narrower reader rounds it on the way in, as it does below a wider producer. Two
                            // producers, or a float against an integer, still have nothing to decide them.
                            match (writes, wider_float(have.dtype, td.dtype)) {
                                (false, Some(wider)) => have.dtype = wider,
                                _ => {
                                    return Err(ComposeReject::TensorShapeConflict {
                                        name: td.name.clone(),
                                        first: (have.elems.rows, have.elems.cols, have.dtype),
                                        second: (td.elems.rows, td.elems.cols, td.dtype),
                                    })
                                }
                            }
                        }
                    }
                }
            }
        }
        chosen
    };
    let mut tensors: Vec<TensorDecl> = Vec::new();
    let mut by_name: BTreeMap<String, TensorId> = BTreeMap::new();
    let mut usage: Vec<Usage> = Vec::new();
    let mut shifts: Vec<Shift> = Vec::new();
    let (
        mut stage_at,
        mut buffer_at,
        mut edge_at,
        mut sem_at,
        mut item_at,
        mut group_at,
        mut trip_at,
    ) = (0, 0, 0, 0, 0, 0, 0);
    for (k, sec) in sections.iter().enumerate() {
        let mut tmap = Vec::with_capacity(sec.tensors.len());
        for td in &sec.tensors {
            let id = match by_name.get(&td.name) {
                Some(id) => *id,
                None => {
                    let id = TensorId::new(tensors.len());
                    let mut decl = producer_decl
                        .get(&td.name)
                        .map(|(d, _)| d.clone())
                        .unwrap_or_else(|| td.clone());
                    decl.id = id;
                    tensors.push(decl);
                    usage.push(Usage::default());
                    by_name.insert(td.name.clone(), id);
                    id
                }
            };
            // The first DECLARED role a name gets is the one a plain input or weight keeps.
            let u = &mut usage[id.idx()];
            if u.declared.is_none() && matches!(td.role, TensorRole::Input | TensorRole::Param) {
                u.declared = Some(td.role);
            }
            tmap.push(id);
        }
        for s in &sec.stages {
            match &s.kind {
                // Sections are walked in order, so a section already recorded is the last one recorded.
                StageKind::Load { tensor, .. } => {
                    let id = tmap[tensor.idx()];
                    let u = &mut usage[id.idx()];
                    if u.read_in.last() != Some(&k) {
                        u.read_in.push(k);
                    }
                }
                StageKind::Store { tensor, .. } => {
                    let id = tmap[tensor.idx()];
                    let u = &mut usage[id.idx()];
                    if u.written_in.last() != Some(&k) {
                        u.written_in.push(k);
                    }
                }
                _ => {}
            }
        }
        shifts.push(Shift {
            tensor: tmap,
            stage: stage_at,
            buffer: buffer_at,
            edge: edge_at,
            sem: sem_at,
            item: item_at,
            group: group_at,
            trip: trip_at,
        });
        stage_at += sec.stages.len();
        buffer_at += sec.buffers.len();
        edge_at += sec.edges.len();
        sem_at += sec.semaphores.len() + if closes(k) { RENDEZVOUS_SEMS } else { 0 };
        item_at += sec.work.items.len();
        group_at += sec.placement.as_ref().map(|p| p.groups.len()).unwrap_or(0);
        trip_at += sec.trips.len();
    }

    // ── 2. Decide the role of every merged tensor.
    //
    // ★ THE ROLE IS WHAT THE PROGRAM DOES, NOT WHAT A SECTION CALLED IT. A section is lowered in
    // isolation and calls its result `Output` because from its own point of view it is one. Reading
    // the sections TOGETHER is what tells an activation (written here, read later) from the program's
    // actual answer (written and never read) — and the front end's carried-state list is what tells
    // both from a memory that outlives the step.
    let carried: BTreeSet<&str> = state.iter().map(String::as_str).collect();
    for (i, t) in tensors.iter_mut().enumerate() {
        let u = &usage[i];
        let written = !u.written_in.is_empty();
        let read = !u.read_in.is_empty();
        if carried.contains(t.name.as_str()) {
            t.role = TensorRole::State;
        } else if written && read {
            t.role = TensorRole::Activation;
        } else if written {
            t.role = TensorRole::Output;
        } else {
            t.role = u.declared.unwrap_or(TensorRole::Input);
        }
    }

    // ── 3. Shift every id space, section by section.
    let mut stages: Vec<Stage> = Vec::new();
    let mut buffers: Vec<Buffer> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut semaphores: Vec<Semaphore> = Vec::new();
    let mut items: Vec<WorkItem> = Vec::new();
    let mut groups: Vec<Group> = Vec::new();
    let mut trips: Vec<Trip> = Vec::new();
    let mut site_of: Vec<Site> = Vec::new();
    let mut engine_of: Vec<EngineSlot> = Vec::new();
    let mut buffer_map: Vec<BufferAt> = Vec::new();
    let mut items_of_site: BTreeMap<Site, Vec<WorkItemId>> = BTreeMap::new();
    let mut requires = Requirements::default();
    let mut acc_tiles = 0usize;
    let mut exact = true;
    let mut approx_reasons: Vec<String> = Vec::new();
    let mesh = target.sites;
    let mesh_sites: Vec<Site> = (0..mesh.y)
        .flat_map(|y| (0..mesh.x).map(move |x| Site::new(x, y)))
        .collect();

    // The rendezvous participants: every `(site, engine)` that runs any section, in site order. The root
    // of every rendezvous is the first of them.
    let order: Vec<(Site, Engine)> = {
        let mut seen: BTreeSet<(Site, Engine)> = BTreeSet::new();
        let mut v: Vec<(Site, Engine)> = Vec::new();
        for (k, sec) in sections.iter().enumerate() {
            for prog in &sec.schedule.as_ref().expect("checked").programs {
                let site = match placement {
                    SectionPlacement::Stacked => prog.site,
                    SectionPlacement::RoundRobin => *mesh_sites
                        .get(k % mesh_sites.len().max(1))
                        .unwrap_or(&prog.site),
                };
                if seen.insert((site, prog.engine)) {
                    v.push((site, prog.engine));
                }
            }
        }
        v.sort_by_key(|(s, e)| (s.y, s.x, *e));
        v
    };
    let participants = order.len();
    let root = order.first().copied();
    let rendezvous_sites: Vec<Site> = {
        let mut v: Vec<Site> = order.iter().map(|(s, _)| *s).collect();
        v.sort_by_key(|s| (s.y, s.x, s.chip));
        v.dedup();
        v
    };
    let rendezvous: Vec<Option<Rendezvous>> = (0..sections.len())
        .map(|k| {
            closes(k).then(|| {
                let base = shifts[k].sem + sections[k].semaphores.len();
                Rendezvous {
                    gather: SemId::new(base),
                    release: SemId::new(base + 1),
                }
            })
        })
        .collect();

    for (k, sec) in sections.iter().enumerate() {
        let sh = &shifts[k];
        // `RoundRobin` shifts a whole section onto one mesh site; `Stacked` keeps the section's own
        // placement, so consecutive sections share local memory.
        let move_site = |s: Site| -> Site {
            match placement {
                SectionPlacement::Stacked => s,
                SectionPlacement::RoundRobin => {
                    *mesh_sites.get(k % mesh_sites.len().max(1)).unwrap_or(&s)
                }
            }
        };
        let pl = sec.placement.as_ref().expect("checked above");
        let mm = sec.memory.as_ref().expect("checked above");
        for s in &sec.stages {
            stages.push(Stage {
                id: sh.s(s.id),
                name: format!("s{k}.{}", s.name),
                kind: sh.kind(&s.kind),
                role: s.role,
                ins: s.ins.iter().map(|b| sh.b(*b)).collect(),
                outs: s.outs.iter().map(|b| sh.b(*b)).collect(),
                site: s.site.map(move_site),
                item: s.item.map(|i| sh.i(i)),
                origin: s.origin.map(|o| sh.s(o)),
            });
        }
        for b in &sec.buffers {
            buffers.push(Buffer {
                id: sh.b(b.id),
                name: format!("s{k}.{}", b.name),
                kind: b.kind,
                dtype: b.dtype,
                space: b.space,
                site: b.site.map(move_site),
                origin: b.origin.map(|o| sh.b(o)),
                alias_of: b.alias_of.map(|a| sh.b(a)),
            });
            let at = mm.buffer_at[b.id.idx()];
            buffer_map.push(BufferAt {
                site: move_site(at.site),
                ..at
            });
        }
        for e in &sec.edges {
            edges.push(Edge {
                id: sh.e(e.id),
                from: sh.s(e.from),
                buffer: sh.b(e.buffer),
                to: e.to.iter().map(|t| sh.s(*t)).collect(),
                handshake: sh.handshake(&e.handshake),
            });
        }
        for s in &sec.semaphores {
            semaphores.push(Semaphore {
                id: sh.m(s.id),
                init: s.init,
                role: s.role,
                operand: s.operand,
                sites: s.sites.iter().map(|x| move_site(*x)).collect(),
            });
        }
        if let (Some(r), Some((root_site, _))) = (rendezvous[k], root) {
            let n = participants;
            semaphores.push(Semaphore {
                id: r.gather,
                init: 0,
                role: SemRole::Barrier {
                    n,
                    phase: BarrierPhase::Gather,
                },
                operand: None,
                sites: vec![root_site],
            });
            semaphores.push(Semaphore {
                id: r.release,
                init: 0,
                role: SemRole::Barrier {
                    n,
                    phase: BarrierPhase::Release,
                },
                operand: None,
                sites: rendezvous_sites.clone(),
            });
        }
        for it in &sec.work.items {
            items.push(WorkItem {
                id: sh.i(it.id),
                coord: it.coord,
                out: it.out,
            });
        }
        // A trip reads a program input, and the merged tensor of that name is the input it reads.
        trips.extend(sec.trips.iter().map(|t| Trip {
            tensor: sh.t(t.tensor),
            ..t.clone()
        }));
        for g in &pl.groups {
            groups.push(Group {
                id: sh.g(g.id),
                rect: g.rect,
                sender: move_site(g.sender),
                direction: g.direction,
                operand: g.operand,
                noc: g.noc,
            });
        }
        for s in &sec.stages {
            site_of.push(move_site(pl.site_of[s.id.idx()]));
            let slot = &pl.engine_of[s.id.idx()];
            engine_of.push(EngineSlot {
                site: move_site(slot.site),
                engine: slot.engine,
                noc: slot.noc,
            });
        }
        for (site, ids) in &pl.items_of_site {
            items_of_site
                .entry(move_site(*site))
                .or_default()
                .extend(ids.iter().map(|i| sh.i(*i)));
        }
        requires
            .handshakes
            .extend(sec.requires.handshakes.iter().copied());
        requires
            .interconnect
            .extend(sec.requires.interconnect.iter().copied());
        acc_tiles = acc_tiles.max(sec.acc_tiles_used);
        match &sec.claim {
            NumericsClaim::Exact => {}
            NumericsClaim::Approx { reason } => {
                exact = false;
                if !approx_reasons.contains(reason) {
                    approx_reasons.push(reason.clone());
                }
            }
        }
    }

    // ── 4. Join the schedules per `(site, engine)`, with a rendezvous at every boundary.
    //
    // Section by section: each section's programs append to the participants that run them, and a closing section's
    // rendezvous to every participant — the only part of the join that touches a participant with nothing to do in
    // the section, and one it cannot skip, since every program passes every barrier.
    let participant: BTreeMap<(Site, Engine), usize> =
        order.iter().enumerate().map(|(i, p)| (*p, i)).collect();
    let mut ops: Vec<Vec<TaggedOp>> = vec![Vec::new(); participants];
    let mut stage_ids: Vec<Vec<StageId>> = vec![Vec::new(); participants];
    for (k, sec) in sections.iter().enumerate() {
        let sh = &shifts[k];
        let own = &sec.schedule.as_ref().expect("checked").programs;
        // The program each participant runs in section k: its own under `Stacked`; under `RoundRobin` the section's
        // program on its engine, wherever the section placed it.
        let runs: Vec<(usize, &EngineProgram)> = match placement {
            SectionPlacement::Stacked => own
                .iter()
                .filter_map(|q| participant.get(&(q.site, q.engine)).map(|i| (*i, q)))
                .collect(),
            SectionPlacement::RoundRobin => order
                .iter()
                .enumerate()
                .filter_map(|(i, (_, engine))| {
                    own.iter().find(|q| q.engine == *engine).map(|q| (i, q))
                })
                .collect(),
        };
        // The rendezvous that closes section k is tagged with a stage of section k, so the schedule attributes it to
        // the section it closes.
        let mut anchor: BTreeMap<usize, StageId> = BTreeMap::new();
        for (i, prog) in runs {
            for s in &prog.stages {
                let id = sh.s(*s);
                stage_ids[i].push(id);
                anchor.entry(i).or_insert(id);
            }
            ops[i].extend(prog.ops.iter().map(|t| sh.tagged(t)));
        }
        if let (Some(r), Some((root_site, _))) = (rendezvous[k], root) {
            let fallback = sec
                .stages
                .first()
                .map(|st| sh.s(st.id))
                .unwrap_or(StageId::new(0));
            for (i, (site, engine)) in order.iter().enumerate() {
                let tag = anchor.get(&i).copied().unwrap_or(fallback);
                let ops = &mut ops[i];
                // Arrive at the root; the root gathers every arrival and releases each site; every program then
                // waits for its own site's release.
                ops.push(TaggedOp {
                    stage: tag,
                    op: ChoreoOp::SemInc {
                        sem: r.gather,
                        at: root_site,
                        by: 1,
                    },
                });
                if root == Some((*site, *engine)) {
                    ops.push(TaggedOp {
                        stage: tag,
                        op: ChoreoOp::SemWait {
                            sem: r.gather,
                            until: SemCond::AtLeast(participants),
                        },
                    });
                    for s in &rendezvous_sites {
                        ops.push(TaggedOp {
                            stage: tag,
                            op: ChoreoOp::SemInc {
                                sem: r.release,
                                at: *s,
                                by: 1,
                            },
                        });
                    }
                }
                ops.push(TaggedOp {
                    stage: tag,
                    op: ChoreoOp::SemWait {
                        sem: r.release,
                        until: SemCond::AtLeast(1),
                    },
                });
            }
        }
    }
    let mut programs: Vec<EngineProgram> = Vec::new();
    for (((site, engine), ops), mut stage_ids) in order.iter().zip(ops).zip(stage_ids) {
        if ops.is_empty() {
            continue;
        }
        stage_ids.sort();
        stage_ids.dedup();
        programs.push(EngineProgram {
            site: *site,
            engine: *engine,
            stages: stage_ids,
            ops,
        });
    }

    // ── 5. The spans, in order.
    let mut spans: Vec<SectionSpan> = Vec::with_capacity(sections.len());
    {
        let (mut st, mut bf, mut ed, mut sm, mut it) = (0usize, 0usize, 0usize, 0usize, 0usize);
        for (k, sec) in sections.iter().enumerate() {
            let sem_end = shifts[k].sem
                + sec.semaphores.len()
                + if rendezvous[k].is_some() {
                    RENDEZVOUS_SEMS
                } else {
                    0
                };
            // What the section's work writes, by the one definition an uncomposed program uses: its `Output`, or — an
            // in-place update — the first state it stores, in declaration order.
            let out = sec.output_tensor().map(|t| shifts[k].t(t.id));
            spans.push(SectionSpan {
                name: sec.name.clone(),
                stages: (st, st + sec.stages.len()),
                buffers: (bf, bf + sec.buffers.len()),
                edges: (ed, ed + sec.edges.len()),
                semaphores: (sm, sem_end),
                items: (it, it + sec.work.items.len()),
                plan: sec.work.plan,
                k_tiles: sec.work.k_tiles,
                output: out,
                barrier_after: rendezvous[k],
            });
            st += sec.stages.len();
            bf += sec.buffers.len();
            ed += sec.edges.len();
            sm = sem_end;
            it += sec.work.items.len();
        }
    }

    let last = sections.last().expect("non-empty");
    let out_name = last
        .tensors
        .iter()
        .find(|t| t.role == TensorRole::Output)
        .map(|t| t.name.clone())
        .unwrap_or_else(|| "result".into());
    let claim = if exact {
        NumericsClaim::Exact
    } else {
        NumericsClaim::Approx {
            reason: approx_reasons.join("; "),
        }
    };
    let mut p = SpatialProgram {
        // ★ ONE SECTION KEEPS ITS OWN NAME. `matmul_2x2x2` says what the program IS; `C_1_sections`
        // says only how it was built. The section count earns its place in the name exactly when there
        // is more than one section to account for — `logits_117_sections` is the whole forward, and that
        // IS the fact worth carrying.
        name: if sections.len() == 1 {
            sections[0].name.clone()
        } else {
            format!("{out_name}_{}_sections", sections.len())
        },
        tile: target.tile,
        tensors,
        stages,
        buffers,
        edges,
        semaphores,
        sections: spans,
        trips,
        links: Vec::new(),
        work: WorkList {
            items,
            k_tiles: last.work.k_tiles,
            plan: last.work.plan,
            k_trip: last.work.k_trip.map(|t| shifts[sections.len() - 1].r(t)),
        },
        numerics: if sections.iter().all(|s| s.numerics == NumericsPolicy::Exact) {
            NumericsPolicy::Exact
        } else {
            NumericsPolicy::AllowSpillRounding
        },
        claim,
        acc_tiles_used: acc_tiles,
        mem_banks: target.site.mem_banks,
        requires,
        placement: Some(Placement {
            mesh,
            site_of,
            engine_of,
            groups,
            items_of_site: items_of_site.into_iter().collect(),
        }),
        memory: Some(MemoryMap {
            buffer_at: buffer_map,
        }),
        schedule: Some(Schedule { programs }),
        certificate: None,
    };
    let high = separate_phase_memory(&mut p);
    if high as usize > target.max_local_mem_bytes() {
        return Err(ComposeReject::PhaseOverflowsMemory {
            bytes: high,
            capacity: target.max_local_mem_bytes(),
        });
    }
    // Where the tensors live is decided once the program's phases and its buffers' layout are: its intermediates over
    // their live ranges, then its carried state as the deployment of this one program. A deployment of many programs
    // — a bundle's layer kinds, each run by many layers — plans its state again over all of them.
    crate::spatial::residency::place_activations(&mut p, target);
    crate::spatial::residency::plan_deployment(&mut [("program", 1, &mut p, None)], target);
    p.validate()?;
    Ok(p)
}

/// ★ A PHASE'S SECTIONS RUN AT ONCE, SO THEY DO NOT SHARE A BYTE. Each section lays its buffers out over its sites'
/// local memory from the same base; sections a barrier separates reuse those bytes, sections of one phase may not.
/// A section of a phase that shares a site with an earlier one of the phase has its buffers there moved past the
/// earlier ones' (per memory space) — sections on disjoint sites keep their layout. Returns the highest byte any site
/// now uses, for the caller to hold against the site's memory.
pub fn separate_phase_memory(p: &mut SpatialProgram) -> u32 {
    let Some(mm) = p.memory.as_mut() else {
        return 0;
    };
    let mut end: BTreeMap<(Site, MemSpace), u32> = BTreeMap::new();
    let mut high = 0u32;
    for sp in &p.sections {
        let mut mine: BTreeMap<(Site, MemSpace), u32> = BTreeMap::new();
        for b in sp.buffers.0..sp.buffers.1 {
            let buf = &p.buffers[b];
            if buf.space == MemSpace::Acc {
                continue;
            }
            let at = &mut mm.buffer_at[b];
            at.offset_bytes += end.get(&(at.site, at.space)).copied().unwrap_or(0);
            if buf.alias_of.is_none() {
                let e = mine.entry((at.site, at.space)).or_default();
                *e = (*e).max(at.offset_bytes + at.bytes);
                high = high.max(at.offset_bytes + at.bytes);
            }
        }
        for (key, e) in mine {
            let x = end.entry(key).or_default();
            *x = (*x).max(e);
        }
        if sp.barrier_after.is_some() {
            end.clear();
        }
    }
    high
}
