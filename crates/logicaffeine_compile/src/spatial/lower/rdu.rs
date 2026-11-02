//! The SambaNova RDU slot: the placed+scheduled IR as a certified neutral placement — the
//! `logos.spatial.neutral` v2 envelope — honestly SDK-gated for the RDA config bytes.
//!
//! ONE envelope serves both RDU lanes. The spatial lane ([`RduSpatialBackend`]) carries the whole IR
//! under `program` beside its recomputed choreography certificate; the placed-grid lane
//! (`codegen_sva::backend::RduBackend`) carries its deterministic dump under `grid`. Both name the lane,
//! both derive the `target` block from the same [`RduChipFacts`] — the chip, its published numbers, the
//! unit model with every value's provenance, which precisions are witnessed and which are chip-supported
//! but witness-pending, what is modeled — and both are gated by the ONE sentence [`GATED_REASON`].
//!
//! `sections`, `state`, `gathers` and `ii_report` are present and empty: the composed (multi-section,
//! stateful) form of a program fills them; a reader written today already sees their keys.

use serde::Serialize;
use serde_json::Value;

use super::{
    require_verified, Census, NeutralArtifact, SpatialArtifact, SpatialBackend, SpatialLowerError,
};
use crate::codegen_sva::fabric_target::{MemHierarchy, Precision, SiliconSpec};
use crate::spatial::certificate::{certify_choreography, CertifyFault, ChoreographyCertificate};
use crate::spatial::eval::ChoreoFault;
use crate::spatial::ir::*;
use crate::spatial::rdu_chip::{Fact, RduChip, RduChipFacts, RduUnitModel, ScaleUp};
use crate::spatial::target::{MeshShape, SpatialTarget};

pub const NEUTRAL_SCHEMA: &str = "logos.spatial.neutral";
pub const NEUTRAL_VERSION: u32 = 2;
/// The versions [`parse_neutral`] reads: v1 (target block + program + certificate, already shipped) and v2.
pub const NEUTRAL_VERSIONS_READ: &[u32] = &[1, 2];

/// The one sentence both RDU lanes gate on. Written here once; `RduBackend::GATED_REASON` and
/// [`RduSpatialBackend::GATED_REASON`] re-export it.
pub const GATED_REASON: &str = "RDA/SambaFlow config emission is gated on the vendor SDK; the certified placement and schedule are ready to bridge";

pub struct RduSpatialBackend {
    pub target: SpatialTarget,
    pub facts: RduChipFacts,
}

impl RduSpatialBackend {
    /// The SN40L lane.
    pub fn new() -> Self {
        Self::for_chip(RduChip::Sn40l)
    }
    /// The lane for one RDU generation, its target derived from that chip's facts.
    pub fn for_chip(chip: RduChip) -> Self {
        Self::with_facts(chip.facts())
    }
    /// The lane for facts with numbers dropped in (an SDK reading, a probe).
    pub fn with_facts(facts: RduChipFacts) -> Self {
        RduSpatialBackend {
            target: facts.spatial_target(),
            facts,
        }
    }
    pub const GATED_REASON: &'static str = GATED_REASON;
}

impl Default for RduSpatialBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SpatialBackend for RduSpatialBackend {
    fn name(&self) -> &str {
        self.target.name
    }
    fn target(&self) -> &SpatialTarget {
        &self.target
    }
    fn lower(&self, p: &SpatialProgram) -> Result<SpatialArtifact, SpatialLowerError> {
        require_verified(p)?;
        self.target
            .scalar_target()
            .ok_or(SpatialLowerError::Unsupported(
                "an RDU target without its scalar descriptor",
            ))?;
        // A lowering ships only what it proved itself, never a certificate on the program's word: the one the program
        // carries RECHECKED — every obligation re-derived from the program and held to what the certificate states —
        // or, when it carries none that rechecks, one derived here.
        let (certificate, rechecked) = match p.certificate.as_ref().filter(|c| c.recheck(p)) {
            Some(c) => (c.clone(), true),
            None => {
                let mut fresh = p.clone();
                fresh.certificate = None;
                (
                    certify_choreography(&fresh)
                        .map_err(|f| SpatialLowerError::Uncertified(certify_fault_as_choreo(f)))?,
                    false,
                )
            }
        };
        let json = neutral_envelope(
            &self.target,
            &self.facts,
            NeutralBody::Program(p),
            Some(&certificate),
        );
        Ok(SpatialArtifact::SdkGated(NeutralArtifact {
            json,
            pretty: pretty_neutral(p),
            reason: GATED_REASON,
            certificate,
            rechecked,
            census: Census::of(p),
        }))
    }
}

fn certify_fault_as_choreo(f: CertifyFault) -> ChoreoFault {
    match f {
        CertifyFault::Eval(e) => e,
        other => ChoreoFault::Unsupported(format!("certificate: {other:?}")),
    }
}

// ── The envelope ─────────────────────────────────────────────────────────────────────────────────

/// Which RDU lane produced an artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NeutralLane {
    /// The whole spatial IR: stages, buffers, semaphores, edges, placement, schedule.
    Spatial,
    /// A placed scalar grid (`codegen_sva::fabric_grid::Grid`), as its deterministic dump.
    Grid,
}

impl NeutralLane {
    pub fn as_str(self) -> &'static str {
        match self {
            NeutralLane::Spatial => "spatial",
            NeutralLane::Grid => "grid",
        }
    }
}

/// The grid lane's body: the deterministic dump(s) of the placed grid(s).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GridBody {
    pub name: String,
    /// The dump — one grid, or every grid of a reconfigurable array joined by `---\n`.
    pub pretty: String,
    /// One dump per grid.
    pub grids: Vec<String>,
    /// The configuration-select width of a reconfigurable array; `None` for a single grid.
    pub cfg_width: Option<u32>,
}

/// What an envelope carries under its lane.
pub enum NeutralBody<'a> {
    Program(&'a SpatialProgram),
    Grid(GridBody),
}

impl NeutralBody<'_> {
    pub fn lane(&self) -> NeutralLane {
        match self {
            NeutralBody::Program(_) => NeutralLane::Spatial,
            NeutralBody::Grid(_) => NeutralLane::Grid,
        }
    }
}

/// The precisions block: what the descriptors advertise, what the chip supports that no witness has shipped
/// for, and the integer every index in the program rides — with its provenance, since a vendor engineer
/// reading the artifact must see whether that width is published for this part or inherited.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NeutralPrecisions {
    pub witnessed: Vec<Precision>,
    pub chip_supported_witness_pending: Vec<Precision>,
    pub index: Fact<Precision>,
}

/// The target block: the mesh and the published numbers, the unit model with every value's provenance,
/// and the modeled ledgers — the descriptor's and the facts'.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NeutralTarget {
    pub name: &'static str,
    pub chip: RduChip,
    pub part: &'static str,
    pub sites: MeshShape,
    pub silicon: Option<SiliconSpec>,
    pub mem: MemHierarchy,
    /// The descriptor's modeled fields.
    pub modeled: &'static [&'static str],
    /// The facts' modeled values, by dotted path.
    pub facts_modeled: Vec<String>,
    pub unit_model: RduUnitModel,
    pub precisions: NeutralPrecisions,
    pub peak_bf16_tflops: u32,
    pub peak_fp8_tflops: Option<u32>,
    pub node_nm: u32,
    pub dies: u32,
    pub chips_per_node: usize,
    pub scale_up: Option<ScaleUp>,
    pub cites: &'static [&'static str],
}

impl NeutralTarget {
    pub fn of(target: &SpatialTarget, facts: &RduChipFacts) -> NeutralTarget {
        NeutralTarget {
            name: target.name,
            chip: facts.chip,
            part: facts.part,
            sites: target.sites,
            silicon: target.scalar_target().and_then(|s| s.silicon),
            mem: target.dram,
            modeled: target.modeled,
            facts_modeled: facts.modeled_fields(),
            unit_model: facts.unit_model,
            precisions: NeutralPrecisions {
                witnessed: facts.witnessed.to_vec(),
                chip_supported_witness_pending: facts.witness_pending.to_vec(),
                index: facts.index_datatype,
            },
            peak_bf16_tflops: facts.peak_bf16_tflops.value,
            peak_fp8_tflops: facts.peak_fp8_tflops.map(|f| f.value),
            node_nm: facts.node_nm.value,
            dies: facts.dies.value,
            chips_per_node: facts.chips_per_node.value,
            scale_up: facts.scale_up.map(|f| f.value),
            cites: facts.cites,
        }
    }
}

#[derive(Serialize)]
struct NeutralEnvelope<'a> {
    schema: &'static str,
    version: u32,
    lane: NeutralLane,
    target: NeutralTarget,
    sections: Vec<Value>,
    state: Vec<Value>,
    gathers: Vec<Value>,
    ii_report: serde_json::Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    program: Option<&'a SpatialProgram>,
    #[serde(skip_serializing_if = "Option::is_none")]
    grid: Option<GridBody>,
    certificate: Option<Value>,
}

/// The certificate summary a reader checks before opening the program: the digest that binds it, the
/// marked graph's size, the liveness witness, every buffer balance, every semaphore count, every compute
/// certificate kind, and the numerics claim.
pub fn certificate_summary(cert: &ChoreographyCertificate) -> Value {
    let mut v = serde_json::json!({
        "schedule_digest": cert.schedule_digest,
        "places": cert.places(),
        "transitions": cert.transitions(),
        "liveness_witness": cert.proof.witnesses().first().map(|w| w.topo_order.clone()).unwrap_or_default(),
        "balances": cert.balances.iter().map(|b| serde_json::json!({
            "buffer": b.buf.idx(), "depth_pages": b.depth_pages, "reserved": b.reserved, "pushed": b.pushed, "popped": b.popped, "high_water": b.high_water,
        })).collect::<Vec<_>>(),
        "sem_counts": cert.sem_counts.iter().map(|(s, n, w)| serde_json::json!([s.idx(), n, w])).collect::<Vec<_>>(),
        "compute": cert.compute.iter().map(|(s, c)| serde_json::json!([s.idx(), format!("{c:?}")])).collect::<Vec<_>>(),
        "numerics": cert.numerics,
    });
    // A per-section proof carries one order per section over that section's OWN numbering, so the flat
    // array above is only readable with the cuts beside it. The whole form has no cuts and no key.
    // The per-section form carries a TABLE of distinct witnesses and, per section, which one it uses.
    // `liveness_witness` stays the whole form's single order, so a single-section artifact is unchanged.
    if let (Some(uses), Some(obj)) = (cert.proof.witness_of_section(), v.as_object_mut()) {
        let table: Vec<&Vec<usize>> = cert
            .proof
            .witnesses()
            .iter()
            .map(|w| &w.topo_order)
            .collect();
        obj.remove("liveness_witness");
        obj.insert("liveness_witnesses".into(), serde_json::json!(table));
        obj.insert("liveness_sections".into(), serde_json::json!(uses));
    }
    v
}

/// `logos.spatial.neutral` v2: the lane, the target block derived from the facts, the empty composed-form
/// sections, the lane's body (the IR itself, every field declaration-ordered — the certificate travels
/// beside it, never inside — or the grid dump), and the certificate summary when the lane proved one.
pub fn neutral_envelope(
    target: &SpatialTarget,
    facts: &RduChipFacts,
    body: NeutralBody<'_>,
    certificate: Option<&ChoreographyCertificate>,
) -> String {
    let lane = body.lane();
    let (program, grid) = match body {
        NeutralBody::Program(p) => (Some(p), None),
        NeutralBody::Grid(g) => (None, Some(g)),
    };
    let doc = NeutralEnvelope {
        schema: NEUTRAL_SCHEMA,
        version: NEUTRAL_VERSION,
        lane,
        target: NeutralTarget::of(target, facts),
        sections: Vec::new(),
        state: Vec::new(),
        gathers: Vec::new(),
        ii_report: serde_json::Map::new(),
        program,
        grid,
        certificate: certificate.map(certificate_summary),
    };
    serde_json::to_string(&doc).expect("the envelope serializes")
}

/// One line per object, in id order: what a vendor engineer reads before opening the JSON.
pub fn pretty_neutral(p: &SpatialProgram) -> String {
    let mut out = String::new();
    let line = |out: &mut String, s: String| {
        out.push_str(&s);
        out.push('\n');
    };
    line(
        &mut out,
        format!(
            "program {} tile {}x{} phase {:?}",
            p.name,
            p.tile.rows,
            p.tile.cols,
            p.phase()
        ),
    );
    for t in &p.tensors {
        line(
            &mut out,
            format!(
                "tensor {} {:?} {}x{} {:?} tiles {}x{}",
                t.name, t.role, t.elems.rows, t.elems.cols, t.dtype, t.tiles.rows_t, t.tiles.cols_t
            ),
        );
    }
    let pl = p.placement.as_ref();
    for s in &p.stages {
        let site = pl
            .map(|pl| pl.site_of[s.id.idx()])
            .map(|si| format!("({},{})", si.x, si.y))
            .unwrap_or_else(|| "-".into());
        let engine = pl
            .map(|pl| format!("{:?}", pl.engine_of[s.id.idx()].engine))
            .unwrap_or_else(|| "-".into());
        // The placed name carries an `@x,y` suffix that the `site` column on this very line already
        // states. A vendor engineer reads this dump to see WHICH STAGE ran WHERE, so print the stage's
        // own name once and let `site` say where — the suffix was the same fact spelled twice.
        let stage_name = s
            .name
            .split_once('@')
            .map(|(base, _)| base)
            .unwrap_or(&s.name);
        line(
            &mut out,
            format!(
                "stage {} {:?} site {site} engine {engine} ins {:?} outs {:?}",
                stage_name,
                s.role,
                s.ins.iter().map(|b| b.idx()).collect::<Vec<_>>(),
                s.outs.iter().map(|b| b.idx()).collect::<Vec<_>>()
            ),
        );
    }
    for b in &p.buffers {
        line(
            &mut out,
            format!(
                "buffer {} {:?} {:?} site {:?}",
                b.name,
                b.kind,
                b.dtype,
                b.site.map(|s| (s.x, s.y))
            ),
        );
    }
    for s in &p.semaphores {
        line(
            &mut out,
            format!(
                "semaphore {} init {} {:?} operand {:?} sites {}",
                s.id.idx(),
                s.init,
                s.role,
                s.operand,
                s.sites.len()
            ),
        );
    }
    for e in &p.edges {
        line(
            &mut out,
            format!(
                "edge stage {} -> {:?} via buffer {} {:?}",
                e.from.idx(),
                e.to.iter().map(|t| t.idx()).collect::<Vec<_>>(),
                e.buffer.idx(),
                e.handshake.kind()
            ),
        );
    }
    if let Some(pl) = pl {
        for g in &pl.groups {
            line(
                &mut out,
                format!(
                    "group {} {:?} {:?} sender ({},{}) rect ({},{})-({},{}) noc {:?}",
                    g.id.idx(),
                    g.operand,
                    g.direction,
                    g.sender.x,
                    g.sender.y,
                    g.rect.x0,
                    g.rect.y0,
                    g.rect.x1,
                    g.rect.y1,
                    g.noc
                ),
            );
        }
        for (site, items) in &pl.items_of_site {
            line(
                &mut out,
                format!(
                    "site ({},{}) items {:?}",
                    site.x,
                    site.y,
                    items.iter().map(|i| i.idx()).collect::<Vec<_>>()
                ),
            );
        }
    }
    if let Some(sch) = &p.schedule {
        for prog in &sch.programs {
            line(
                &mut out,
                format!(
                    "engine site ({},{}) {:?} ops {}",
                    prog.site.x,
                    prog.site.y,
                    prog.engine,
                    prog.ops.len()
                ),
            );
        }
    }
    out
}

/// Parse a `logos.spatial.neutral` artifact (v1 or v2, spatial lane) back into the program it describes.
pub fn parse_neutral(json: &str) -> Result<SpatialProgram, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let schema = v.get("schema").and_then(Value::as_str).ok_or("no schema")?;
    if schema != NEUTRAL_SCHEMA {
        return Err(format!("schema `{schema}` is not `{NEUTRAL_SCHEMA}`"));
    }
    let version = v
        .get("version")
        .and_then(Value::as_u64)
        .ok_or("no version")?;
    if !NEUTRAL_VERSIONS_READ
        .iter()
        .any(|k| u64::from(*k) == version)
    {
        return Err(format!("{NEUTRAL_SCHEMA} version {version} is not one this reader knows ({NEUTRAL_VERSIONS_READ:?})"));
    }
    if version >= 2 {
        let lane = v
            .get("lane")
            .and_then(Value::as_str)
            .ok_or("a v2 artifact names its lane")?;
        if lane != NeutralLane::Spatial.as_str() {
            return Err(format!(
                "a `{lane}`-lane artifact carries no spatial program (its body is `{lane}`)"
            ));
        }
    }
    let program = v.get("program").ok_or("no program")?;
    serde_json::from_value(program.clone()).map_err(|e| e.to_string())
}
