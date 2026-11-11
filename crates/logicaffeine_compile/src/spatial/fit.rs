//! The **fit report** — what a model or a workload needs from an RDU, sized against ONE source of chip
//! facts ([`RduChipFacts`]) and written down with every number labeled.
//!
//! Two reports and a plan:
//!   * [`gemm_fit`] — an int8 GEMM lowered through the spatial pipeline onto the chip: its arithmetic,
//!     its working set against the SRAM, the VERIFIED program's census, the certificate that rechecks,
//!     and the II view on the chip's unit model.
//!   * [`qwen38_fit`] — Qwen3.8-27B (the forward authored in LOGOS, [`gen_qwen38_lg`]) on the chip: the
//!     weights by storage, per-layer MACs and arithmetic intensity, the KV cache, the Gated-DeltaNet
//!     memory and the conv rings, the peak live set of activations, residency tiers, and tensor-parallel
//!     sharding over a node and a rack of RDUs with the HBM verdict at the model's full context.
//!   * [`vllm_plan`] — the deployment plan the fit implies for the vendor's vLLM backend: model id, TP,
//!     dtype, max-model-len, the op list the backend must support, the state each layer keeps.
//!
//! ★ DERIVED, NEVER TYPED. Every model number is a function of [`Qwen38Cfg`] and of the authored program's
//! own declarations and statements — the tensor inventory is read from `gen_qwen38_lg`'s output, the MACs
//! from its `matmul`/`attention`/`gated delta`/`causal conv` statements, the live set from a liveness walk
//! over its `Let` chain. Every chip number is a [`Fact`] with its provenance. The generator locks grep this
//! file and its examples for bare literals; a number a reader cannot trace to the config, the program or
//! the facts cannot land here.
//!
//! ★ NO TIME. Nothing here ran on an RDU. No field is a duration, a rate of ours, or a speed; the vendor's
//! published peak rates travel only under their `published` label with a citation, and `no_speed_claim`
//! is true on every artifact. The II block spells modeled cycles on the unit model, never latency.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{json, Value};

use super::ir::{SpatialProgram, TensorRole};
use super::lower::rdu::RduSpatialBackend;
use super::lower::{Census, NeutralArtifact, SpatialArtifact, SpatialBackend};
use super::pipeline::compile_to_spatial;
use super::programs::gemm_program;
use super::rdu_chip::{
    rdu_ii_report, Fact, Provenance, RduChip, RduChipFacts, RduIiReport, RduUnitModel, ScaleUp,
};
use super::target::{precision_bytes, CreditLatency, SpatialTarget};
use crate::codegen_cuda::qwen38::{gen_qwen38_lg, Qwen38Cfg};
use crate::codegen_sva::fabric_target::Precision;
use crate::codegen_sva::pmu::{BankingMode, PmuSpec};

pub const FIT_SCHEMA: &str = "logos.rdu.fit";
pub const VLLM_PLAN_SCHEMA: &str = "logos.rdu.vllm_plan";
pub const FIT_VERSION: u32 = 1;

pub const PROVENANCE_MODELED: &str = "modeled";
pub const PROVENANCE_PUBLISHED: &str = "published";
pub const PROVENANCE_DERIVED: &str = "derived";
pub const PROVENANCE_MEASURED: &str = "measured";

/// The HF checkpoint a vLLM run loads (safetensors, bf16).
pub const QWEN38_MODEL_ID: &str = "Qwen/Qwen3.8-27B";
/// The HF `model_type`, and the architecture it inherits in vLLM (the hybrid Gated-DeltaNet + attention).
pub const QWEN38_HF_MODEL_TYPE: &str = "qwen3_5";
pub const QWEN38_HF_INHERITS: &str = "qwen3_next";
/// The GGUF our own CUDA lane runs from — NOT what vLLM loads.
pub const QWEN38_GGUF: &str = "bartowski/Qwen3.8-27B-GGUF (Q8_0)";
/// The dtype the deployment is sized in.
pub const DEPLOYMENT_DTYPE: &str = "bfloat16";

pub const GEMM_FIT_PATH: &str = "benchmarks/silicon/rdu_fit_gemm.json";
pub const QWEN38_FIT_PATH: &str = "benchmarks/silicon/qwen38_sn50.json";
pub const VLLM_PLAN_PATH: &str = "benchmarks/silicon/qwen38_vllm_plan.json";
pub const GEMM_GENERATED_BY: &str = "cargo run -p logicaffeine-compile --example emit_rdu_fit_gemm";
pub const QWEN38_GENERATED_BY: &str = "cargo run -p logicaffeine-compile --example emit_qwen38_rdu";

/// GGUF Q8_0: a block of 32 int8 quants carries one f16 scale.
pub const Q8_0_BLOCK: usize = 32;
pub const Q8_0_SCALE_BYTES: usize = 2;
pub const Q8_0_LAYOUT: &str =
    "GGUF Q8_0: blocks of 32 int8 quants + one f16 scale (34 bytes per 32 elements)";
pub const F32_BYTES: usize = 4;
pub const BF16_BYTES: usize = 2;

/// The GEMM fit's workload, in the chip's native tiles: an 8×8-tile output over 4 k-tiles.
pub const GEMM_FIT_TILES: (usize, usize, usize) = (8, 8, 4);

pub const HONESTY: &str = "Every number is a model's: sizes from the config and the authored program, the chip from its published facts (cited) or its modeled unit model (labeled). Nothing here ran on an RDU; no field is a time and no throughput is claimed.";

const MIB: usize = 1 << 20;
const GIB: usize = 1 << 30;

/// The bytes a Q8_0 tensor of `elems` occupies: the quants plus one f16 scale per (partial) block.
pub fn q8_0_bytes(elems: usize) -> usize {
    elems + elems.div_ceil(Q8_0_BLOCK) * Q8_0_SCALE_BYTES
}

/// Pretty JSON with one trailing newline — the byte form every fit artifact is pinned in.
pub fn render<T: Serialize>(report: &T) -> String {
    format!(
        "{}\n",
        serde_json::to_string_pretty(report).expect("a fit report serializes")
    )
}

/// The tensor-parallel width the plan brings: one SambaRack where the vendor tabulates it, else one node.
pub fn default_tensor_parallel(facts: &RduChipFacts) -> usize {
    facts
        .scale_up
        .map(|s| s.value.chips_per_rack)
        .unwrap_or(facts.chips_per_node.value)
}

/// Why a tensor-parallel width does not split a model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TpRefusal {
    pub tp: usize,
    pub why: String,
}

/// The widths an RDU deployment over `tp` chips takes: every width the author writes. The author shares each head count
/// and the FFN's columns over the chips in balanced runs, so 24 query heads deploy over 16 chips, some holding one more
/// than others; the layers it does not yet split — a latent attention, an expert mixture, an attention with biases or
/// sinks — are refused by its own name for what is missing (`author::check_tp`). A plan for a serving stack keeps the
/// stack's own rule ([`serving_admits_tp`]).
pub fn admits_tp(spec: &crate::model::spec::ModelSpec, tp: usize) -> Result<(), TpRefusal> {
    if tp == 0 {
        return Err(TpRefusal { tp, why: "a tensor-parallel width of zero chips".into() });
    }
    crate::model::author::check_tp(spec, tp).map_err(|e| TpRefusal { tp, why: format!("{} over {tp} chips is not written yet: {}", spec.arch, e.0) })
}

/// ★ A SERVING STACK'S TENSOR-PARALLEL WIDTH DIVIDES THE MODEL. Megatron splits attention and the Gated DeltaNet BY HEAD
/// and the FFN and the vocabulary by column, so `tp` must divide the query heads, both GDN head counts, the FFN width and
/// the vocabulary; the KV heads either split across the chips or replicate over them evenly (`kv % tp == 0` or
/// `tp % kv == 0`). Any other width is refused by the serving stack before a weight loads — vLLM: "Total number of
/// attention heads (24) must be divisible by tensor parallel size (16)" — and the widths the author does not yet write
/// are refused as [`admits_tp`] refuses them.
pub fn serving_admits_tp(spec: &crate::model::spec::ModelSpec, tp: usize) -> Result<(), TpRefusal> {
    use crate::model::spec::{Ffn, Mixer};
    let refuse = |why: String| Err(TpRefusal { tp, why });
    if tp == 0 {
        return refuse("a tensor-parallel width of zero chips".into());
    }
    let counts = |what: &str| -> Vec<usize> {
        let mut v: Vec<usize> = spec
            .layers
            .iter()
            .filter_map(|l| match (what, &l.mixer, &l.ffn) {
                ("query heads", Mixer::Attention(a), _) => Some(a.heads),
                ("Gated DeltaNet key heads", Mixer::GatedDelta(g), _) => Some(g.k_heads),
                ("Gated DeltaNet value heads", Mixer::GatedDelta(g), _) => Some(g.v_heads),
                ("FFN columns", _, Ffn::Dense { dff, .. }) => Some(*dff),
                _ => None,
            })
            .collect();
        v.dedup();
        v
    };
    for what in ["query heads", "Gated DeltaNet key heads", "Gated DeltaNet value heads", "FFN columns"] {
        if let Some(n) = counts(what).into_iter().find(|n| n % tp != 0) {
            return refuse(format!("the {n} {what} do not split over {tp} chips"));
        }
    }
    if spec.vocab % tp != 0 {
        return refuse(format!("the {} vocabulary rows do not split over {tp} chips", spec.vocab));
    }
    for l in &spec.layers {
        if let Mixer::Attention(a) = &l.mixer {
            if a.kv_heads % tp != 0 && tp % a.kv_heads != 0 {
                return refuse(format!(
                    "the {} KV heads neither split over {tp} chips nor replicate over them evenly",
                    a.kv_heads
                ));
            }
        }
    }
    crate::model::author::check_tp(spec, tp).or_else(|e| refuse(format!("{} over {tp} chips is not written yet: {}", spec.arch, e.0)))
}

/// [`serving_admits_tp`] for the Qwen3.8 configuration the fit report and the serving plans size.
pub fn qwen38_admits_tp(cfg: &Qwen38Cfg, tp: usize) -> Result<(), TpRefusal> {
    serving_admits_tp(&cfg.spec(), tp)
}

/// The tensor-parallel width Qwen3.8 is served at on `facts`' chips: the widest the model admits
/// ([`qwen38_admits_tp`]) within the chips' tensor-parallel domain ([`default_tensor_parallel`]); the rest of the
/// domain serves more sequences as data-parallel replicas.
pub fn qwen38_tensor_parallel(cfg: &Qwen38Cfg, facts: &RduChipFacts) -> usize {
    (1..=default_tensor_parallel(facts))
        .rev()
        .find(|tp| qwen38_admits_tp(cfg, *tp).is_ok())
        .unwrap_or(1)
}

/// How Qwen3.8 is served on a chip's scale-up domain at one context length — every choice read off the chips' memory
/// hierarchy and topology and the model's structure, none typed. `provenance: modeled`; no field is a time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ServingPlan {
    /// The chips one tensor-parallel group spans ([`qwen38_tensor_parallel`]).
    pub tensor_parallel: usize,
    /// The chips of the scale-up domain ([`default_tensor_parallel`]) the group does not span, as whole replicas.
    pub data_parallel: usize,
    pub ctx: usize,
    /// One chip's HBM, its slice of the weights, and ONE sequence's carried state on it at `ctx`: the KV cache (sharded
    /// over the KV heads, replicated beyond them), the Gated-DeltaNet memory and the conv rings.
    pub hbm_bytes_per_chip: usize,
    pub weights_per_chip_bytes: usize,
    pub sequence_bytes_per_chip: usize,
    /// The most sequences a replica holds at once: their carried state beside the weights in every chip's HBM.
    pub max_sequences_per_replica: usize,
    pub max_sequences: usize,
    /// One chip's SRAM, one sequence's per-step activation working set, and the most sequences whose working set —
    /// with their recurrent state — stays on-chip for a step.
    pub sram_bytes_per_chip: usize,
    pub activation_bytes_per_sequence: usize,
    pub sequences_resident_in_sram: usize,
    pub provenance: &'static str,
    pub no_speed_claim: bool,
}

/// Qwen3.8's serving plan on `facts`' chips at context `ctx`: the tensor-parallel width the model's heads admit within
/// the scale-up domain, the rest of the domain as data-parallel replicas, and how many sequences a replica holds —
/// HBM-bound by each sequence's carried state beside the chip's weight slice, SRAM-resident as far as each
/// sequence's per-step working set allows. The per-chip bytes are the fit report's own ([`qwen38_fit`]).
pub fn qwen38_serving_plan(
    cfg: &Qwen38Cfg,
    facts: &RduChipFacts,
    ctx: usize,
) -> Result<ServingPlan, String> {
    let tp = qwen38_tensor_parallel(cfg, facts);
    let domain = default_tensor_parallel(facts);
    let FitReport::Qwen38(fit) = qwen38_fit(cfg, facts, tp, ctx)? else {
        return Err("the Qwen3.8 fit is a Qwen3.8 report".into());
    };
    let t = &fit.tensor_parallel;
    let sequence = t.state_bytes_per_chip + t.kv_cache_at_ctx.per_chip_bytes;
    let free = t.hbm_bytes_per_chip.checked_sub(t.weights_per_chip_bytes).ok_or_else(|| {
        format!("a chip's {} bytes of HBM do not hold its {} bytes of weights at tensor parallel {tp}", t.hbm_bytes_per_chip, t.weights_per_chip_bytes)
    })?;
    let per_replica = free / sequence.max(1);
    if per_replica == 0 {
        return Err(format!("a chip's HBM holds its weights but not one sequence's {sequence} bytes of state at context {ctx}"));
    }
    let r = &fit.residency;
    let per_sequence_on_chip =
        r.activations_per_token_bytes + r.gdn_state_bytes_per_chip + r.conv_state_bytes_per_chip;
    Ok(ServingPlan {
        tensor_parallel: tp,
        data_parallel: domain / tp,
        ctx,
        hbm_bytes_per_chip: t.hbm_bytes_per_chip,
        weights_per_chip_bytes: t.weights_per_chip_bytes,
        sequence_bytes_per_chip: sequence,
        max_sequences_per_replica: per_replica,
        max_sequences: per_replica * (domain / tp),
        sram_bytes_per_chip: r.sram_bytes,
        activation_bytes_per_sequence: r.activations_per_token_bytes,
        sequences_resident_in_sram: r.sram_bytes / per_sequence_on_chip.max(1),
        provenance: PROVENANCE_MODELED,
        no_speed_claim: true,
    })
}

/// The GEMM fit's `(m, n, k)` on a target: [`GEMM_FIT_TILES`] in that target's tile geometry.
pub fn gemm_fit_shape(target: &SpatialTarget) -> (usize, usize, usize) {
    let (mt, nt, kt) = GEMM_FIT_TILES;
    (
        mt * target.tile.rows,
        nt * target.tile.cols,
        kt * target.tile.cols,
    )
}

// ── Labeled numbers ─────────────────────────────────────────────────────────────────────────────

/// A chip number with its provenance spelled the way the fit artifacts spell it (`published`/`modeled`).
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Cited<T> {
    pub value: T,
    pub provenance: &'static str,
    pub cite: &'static str,
}

fn label(p: Provenance) -> &'static str {
    match p {
        Provenance::Published => PROVENANCE_PUBLISHED,
        Provenance::Derived => PROVENANCE_DERIVED,
        Provenance::Measured => PROVENANCE_MEASURED,
        Provenance::Modeled => PROVENANCE_MODELED,
    }
}

fn cited<T: Copy>(f: Fact<T>) -> Cited<T> {
    Cited {
        value: f.value,
        provenance: label(f.provenance),
        cite: f.cite,
    }
}

fn cited_map<T: Copy, U>(f: Fact<T>, map: impl Fn(T) -> U) -> Cited<U> {
    Cited {
        value: map(f.value),
        provenance: label(f.provenance),
        cite: f.cite,
    }
}

// ── The chip block ──────────────────────────────────────────────────────────────────────────────

/// The chip as the fit sees it: every vendor number under its own label, the derived per-PCU budget
/// modeled, the descriptor's modeled ledger and the sources.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChipBlock {
    pub part: &'static str,
    pub chip: RduChip,
    pub target: &'static str,
    pub peak_bf16_tflops: Cited<u32>,
    pub peak_fp8_tflops: Option<Cited<u32>>,
    pub sram_bytes: Cited<usize>,
    pub hbm_bytes: Cited<usize>,
    pub ddr_bytes_max: Cited<usize>,
    pub node_nm: Cited<u32>,
    pub dies: Cited<u32>,
    pub chips_per_node: Cited<usize>,
    pub chips_per_rack: Option<Cited<usize>>,
    /// The tensor-parallel width the plan brings: the rack where tabulated, else the node.
    pub tensor_parallel_width: Cited<usize>,
    pub scale_up: Option<Cited<ScaleUp>>,
    pub pcu_count: Cited<usize>,
    pub pmu_count: Cited<usize>,
    pub local_mem_bytes_per_pcu: usize,
    pub witnessed_precisions: Vec<Precision>,
    pub witness_pending_precisions: Vec<Precision>,
    pub modeled_fields: Vec<String>,
    pub cites: &'static [&'static str],
    pub provenance: &'static str,
}

impl ChipBlock {
    pub fn of(facts: &RduChipFacts) -> ChipBlock {
        let rack = facts.scale_up.map(|s| cited_map(s, |v| v.chips_per_rack));
        ChipBlock {
            part: facts.part,
            chip: facts.chip,
            target: facts.chip.target_name(),
            peak_bf16_tflops: cited(facts.peak_bf16_tflops),
            peak_fp8_tflops: facts.peak_fp8_tflops.map(cited),
            sram_bytes: cited_map(facts.sram_mib, |v| v * MIB),
            hbm_bytes: cited_map(facts.hbm_gib, |v| v * GIB),
            ddr_bytes_max: cited_map(facts.ddr_gib_max, |v| v * GIB),
            node_nm: cited(facts.node_nm),
            dies: cited(facts.dies),
            chips_per_node: cited(facts.chips_per_node),
            chips_per_rack: rack,
            tensor_parallel_width: rack.unwrap_or_else(|| cited(facts.chips_per_node)),
            scale_up: facts.scale_up.map(cited),
            pcu_count: cited(facts.pcu_count),
            pmu_count: cited(facts.pmu_count),
            local_mem_bytes_per_pcu: facts.local_mem_bytes_per_pcu(),
            witnessed_precisions: facts.witnessed.to_vec(),
            witness_pending_precisions: facts.witness_pending.to_vec(),
            modeled_fields: facts.modeled_fields(),
            cites: facts.cites,
            provenance: PROVENANCE_MODELED,
        }
    }
}

// ── The II block ────────────────────────────────────────────────────────────────────────────────

/// The unit model with every value under the fit's labels.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UnitModelBlock {
    pub pcu_stages: Cited<usize>,
    pub pcu_lanes: Cited<usize>,
    pub pcu_envelope: Cited<(usize, usize)>,
    pub datapath_bits: Cited<u32>,
    pub acc_tiles: Cited<usize>,
    pub mesh_columns: Cited<usize>,
    pub fusion_window: Cited<usize>,
    pub counter_nest: Cited<usize>,
    pub pmu: Cited<PmuSpec>,
    pub credit: Option<Cited<CreditLatency>>,
}

impl UnitModelBlock {
    pub fn of(um: &RduUnitModel) -> UnitModelBlock {
        UnitModelBlock {
            pcu_stages: cited(um.pcu_stages),
            pcu_lanes: cited(um.pcu_lanes),
            pcu_envelope: cited(um.pcu_envelope),
            datapath_bits: cited(um.datapath_bits),
            acc_tiles: cited(um.acc_tiles),
            mesh_columns: cited(um.mesh_columns),
            fusion_window: cited(um.fusion_window),
            counter_nest: cited(um.counter_nest),
            pmu: cited(um.pmu),
            credit: um.credit.map(cited),
        }
    }
}

/// One compute stage on the unit model: its IIs and the cycles to its result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IiStageRow {
    pub stage: usize,
    pub name: String,
    pub op: &'static str,
    pub reduce: bool,
    pub carried: bool,
    pub lanes: usize,
    pub cells: usize,
    pub resource_ii: usize,
    pub recurrence_ii: usize,
    pub memory_ii: usize,
    /// PCU pipeline depth, plus the reduction tree when the stage folds an axis.
    pub result_cycles: usize,
}

/// One IR edge's credit loop on the unit model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IiCreditRow {
    pub edge: usize,
    pub from: usize,
    pub to: usize,
    pub hops: usize,
    pub forward_cycles: usize,
    pub backward_cycles: usize,
    pub n_buffer_depth: usize,
    pub credit_depth: usize,
    pub depth: usize,
}

/// The II view of a placed program — [`rdu_ii_report`] with cycles spelled as cycles.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IiBlock {
    pub chip: RduChip,
    pub program: String,
    /// What program the block is the view of, and why.
    pub source: String,
    pub unit_model: UnitModelBlock,
    pub banking: BankingMode,
    pub credit_declared: bool,
    pub pipeline_ii: usize,
    pub stages: Vec<IiStageRow>,
    pub buffer_depths: Vec<(usize, usize, usize)>,
    pub credit_depths: Vec<IiCreditRow>,
    pub cycles_are: &'static str,
    pub provenance: &'static str,
}

impl IiBlock {
    pub fn of(report: &RduIiReport, source: String) -> IiBlock {
        IiBlock {
            chip: report.chip,
            program: report.program.clone(),
            source,
            unit_model: UnitModelBlock::of(&report.unit_model),
            banking: report.banking,
            credit_declared: report.credit_declared,
            pipeline_ii: report.pipeline_ii,
            stages: report
                .stages
                .iter()
                .map(|r| IiStageRow {
                    stage: r.stage,
                    name: r.name.clone(),
                    op: r.op,
                    reduce: r.reduce,
                    carried: r.carried,
                    lanes: r.lanes,
                    cells: r.timing.cells,
                    resource_ii: r.timing.resource_ii,
                    recurrence_ii: r.timing.recurrence_ii,
                    memory_ii: r.timing.memory_ii,
                    result_cycles: r.timing.latency,
                })
                .collect(),
            buffer_depths: report.buffer_depths.clone(),
            credit_depths: report
                .credit_depths
                .iter()
                .map(|c| IiCreditRow {
                    edge: c.edge,
                    from: c.from,
                    to: c.to,
                    hops: c.hops,
                    forward_cycles: c.forward_latency,
                    backward_cycles: c.backward_latency,
                    n_buffer_depth: c.n_buffer_depth,
                    credit_depth: c.credit_depth,
                    depth: c.depth,
                })
                .collect(),
            cycles_are: "cycles of the modeled unit model (Plasticine-inherited PCU/PMU), never a measurement of the chip",
            provenance: report.provenance,
        }
    }
}

// ── Program + certificate summaries ─────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProgramSummary {
    pub name: String,
    pub phase: String,
    pub tensors: usize,
    pub stages: usize,
    pub buffers: usize,
    pub semaphores: usize,
    pub edges: usize,
    pub census: Census,
    pub provenance: &'static str,
}

impl ProgramSummary {
    pub fn of(p: &SpatialProgram, census: &Census) -> ProgramSummary {
        ProgramSummary {
            name: p.name.clone(),
            phase: format!("{:?}", p.phase()),
            tensors: p.tensors.len(),
            stages: p.stages.len(),
            buffers: p.buffers.len(),
            semaphores: p.semaphores.len(),
            edges: p.edges.len(),
            census: census.clone(),
            provenance: PROVENANCE_MODELED,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CertificateSummary {
    pub schedule_digest: u64,
    pub digest_hex: String,
    /// Recomputed here against the program the certificate travels with.
    pub rechecked: bool,
    pub places: usize,
    pub transitions: usize,
    pub provenance: &'static str,
}

impl CertificateSummary {
    pub fn of(artifact: &NeutralArtifact, p: &SpatialProgram) -> CertificateSummary {
        let cert = &artifact.certificate;
        CertificateSummary {
            schedule_digest: cert.schedule_digest,
            digest_hex: format!("{:016x}", cert.schedule_digest),
            rechecked: cert.recheck(p),
            places: cert.places(),
            transitions: cert.transitions(),
            provenance: PROVENANCE_MODELED,
        }
    }
}

/// Lower `source` on `facts`' descriptor through the RDU lane: the verified program and its SDK-gated artifact.
fn lower_on(
    source: &str,
    facts: &RduChipFacts,
) -> Result<(SpatialProgram, NeutralArtifact), String> {
    let target = facts.spatial_target();
    let program = compile_to_spatial(source, &target)
        .map_err(|e| format!("the {} cannot host it: {e:?}", target.name))?;
    match RduSpatialBackend::with_facts(facts.clone()).lower(&program) {
        Ok(SpatialArtifact::SdkGated(a)) => Ok((program, a)),
        Ok(other) => Err(format!("the RDU lane is SDK-gated, got {other:?}")),
        Err(e) => Err(format!("the RDU lane refused the verified program: {e:?}")),
    }
}

// ── The GEMM fit ────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GemmTiles {
    pub rows_t: usize,
    pub cols_t: usize,
    pub k_tiles: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GemmWorkload {
    pub kind: &'static str,
    pub m: usize,
    pub n: usize,
    pub k: usize,
    pub macs: usize,
    pub tile: (usize, usize),
    pub tiles: GemmTiles,
    pub source: &'static str,
    pub provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GemmOperand {
    pub name: String,
    pub role: String,
    pub rows: usize,
    pub cols: usize,
    pub dtype: Precision,
    pub bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GemmArithmetic {
    pub operands: Vec<GemmOperand>,
    pub bytes: usize,
    pub arithmetic_intensity_macs_per_byte: f64,
    pub provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GemmResidency {
    pub working_set_bytes: usize,
    pub sram_bytes: usize,
    pub sram_fit: bool,
    pub local_mem_bytes_per_pcu: usize,
    pub fits_one_pcu: bool,
    pub buffer_bytes_on_mesh: usize,
    pub sites_used: usize,
    pub sites_available: usize,
    pub provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GemmFit {
    pub schema: &'static str,
    pub version: u32,
    #[serde(rename = "_generated_by")]
    pub generated_by: &'static str,
    pub provenance: &'static str,
    pub no_speed_claim: bool,
    pub honesty: &'static str,
    pub workload: GemmWorkload,
    pub chip: ChipBlock,
    pub arithmetic: GemmArithmetic,
    pub residency: GemmResidency,
    pub program: ProgramSummary,
    pub certificate: CertificateSummary,
    pub gated: &'static str,
    pub ii_report: IiBlock,
}

/// The int8 GEMM `C(m×n) = A(m×k)·B(k×n)` lowered onto `facts`' chip.
pub fn gemm_fit(m: usize, n: usize, k: usize, facts: &RduChipFacts) -> Result<FitReport, String> {
    let source = gemm_program(m, n, k);
    let (program, artifact) = lower_on(&source, facts)?;
    let target = facts.spatial_target();
    let macs = m * n * k;
    let output = program
        .tensors
        .iter()
        .find(|t| t.role == TensorRole::Output)
        .ok_or("the GEMM has an output tensor")?;
    let operands: Vec<GemmOperand> = program
        .tensors
        .iter()
        .map(|t| GemmOperand {
            name: t.name.clone(),
            role: format!("{:?}", t.role),
            rows: t.elems.rows,
            cols: t.elems.cols,
            dtype: t.dtype,
            bytes: t.elems.rows * t.elems.cols * precision_bytes(t.dtype),
        })
        .collect();
    let bytes: usize = operands.iter().map(|o| o.bytes).sum();
    let buffer_bytes_on_mesh: usize = program
        .buffers
        .iter()
        .map(|b| program.buffer_bytes(b))
        .sum();
    let sites_available = target.sites.x * target.sites.y;
    let ii = rdu_ii_report(&program, facts);
    Ok(FitReport::Gemm(GemmFit {
        schema: FIT_SCHEMA,
        version: FIT_VERSION,
        generated_by: GEMM_GENERATED_BY,
        provenance: PROVENANCE_MODELED,
        no_speed_claim: true,
        honesty: HONESTY,
        workload: GemmWorkload {
            kind: "int8_gemm",
            m,
            n,
            k,
            macs,
            tile: (program.tile.rows, program.tile.cols),
            tiles: GemmTiles { rows_t: output.tiles.rows_t, cols_t: output.tiles.cols_t, k_tiles: k.div_ceil(program.tile.cols) },
            source: "spatial::programs::gemm_program — the canonical LOGOS matmul with deterministic fills, lifted by the tensor recognizer",
            provenance: PROVENANCE_MODELED,
        },
        chip: ChipBlock::of(facts),
        arithmetic: GemmArithmetic { operands, bytes, arithmetic_intensity_macs_per_byte: macs as f64 / bytes as f64, provenance: PROVENANCE_MODELED },
        residency: GemmResidency {
            working_set_bytes: bytes,
            sram_bytes: facts.sram_bytes(),
            sram_fit: bytes <= facts.sram_bytes(),
            local_mem_bytes_per_pcu: facts.local_mem_bytes_per_pcu(),
            fits_one_pcu: bytes <= facts.local_mem_bytes_per_pcu(),
            buffer_bytes_on_mesh,
            sites_used: artifact.census.sites,
            sites_available,
            provenance: PROVENANCE_MODELED,
        },
        program: ProgramSummary::of(&program, &artifact.census),
        certificate: CertificateSummary::of(&artifact, &program),
        gated: artifact.reason,
        ii_report: IiBlock::of(&ii, format!("the GEMM itself ({}), placed on the {}", program.name, target.name)),
    }))
}

// ── Qwen3.8: the authored program, read back ────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Storage {
    #[serde(rename = "q8_0")]
    Q8_0,
    F32,
    Bf16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TensorClass {
    Weight,
    State,
    RopeTable,
    Activation,
}

/// One tensor of the authored forward, as it is declared there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TensorLine {
    pub name: String,
    pub rows: usize,
    pub cols: usize,
    pub storage: Storage,
    pub class: TensorClass,
    pub layer: Option<usize>,
    pub bytes: usize,
}

impl TensorLine {
    pub fn elems(&self) -> usize {
        self.rows * self.cols
    }
    pub fn is_matrix(&self) -> bool {
        self.rows > 1 && self.cols > 1
    }
}

/// `Let V be a R by C tensor of Real.` → `(V, R, C)`.
fn tensor_declaration(line: &str) -> Option<(&str, usize, usize)> {
    let rest = line.strip_prefix("Let ")?;
    let (name, shape) = rest.split_once(" be a ")?;
    let shape = shape.strip_suffix(" tensor of Real.")?;
    let (rows, cols) = shape.split_once(" by ")?;
    Some((name, rows.parse().ok()?, cols.parse().ok()?))
}

/// The layer a name belongs to: its trailing digits (`Wq3` → 3; `Wout`, `x`, `RC` → none).
fn layer_of(name: &str) -> Option<usize> {
    let digits = name.trim_end_matches(|c: char| !c.is_ascii_digit()).len();
    let start = name[..digits]
        .rfind(|c: char| !c.is_ascii_digit())
        .map_or(0, |i| i + 1);
    name[start..digits]
        .parse()
        .ok()
        .filter(|_| digits > 0 && start < digits)
}

fn classify_f32(name: &str) -> TensorClass {
    if name.starts_with("St") || name.starts_with("Cs") {
        TensorClass::State
    } else if name == "RC" || name == "RS" {
        TensorClass::RopeTable
    } else {
        TensorClass::Weight
    }
}

fn bytes_of(storage: Storage, elems: usize) -> usize {
    match storage {
        Storage::Q8_0 => q8_0_bytes(elems),
        Storage::F32 => elems * F32_BYTES,
        Storage::Bf16 => elems * BF16_BYTES,
    }
}

/// Every tensor the authored forward declares — read from [`gen_qwen38_lg`]'s own output and classified by
/// the manifests it returns (Q8_0 matrices vs f32 tensors; the hidden state is the one undeclared-in-the-
/// manifests activation) — plus the embedding table the decode step gathers its input row from.
pub fn qwen38_tensors(cfg: &Qwen38Cfg) -> Vec<TensorLine> {
    let (source, f32w, q8w) = gen_qwen38_lg(cfg, "w");
    let mut out = Vec::new();
    for line in source.lines() {
        let Some((name, rows, cols)) = tensor_declaration(line) else {
            continue;
        };
        let (storage, class) = if q8w.contains_key(name) {
            (Storage::Q8_0, TensorClass::Weight)
        } else if f32w.contains_key(name) {
            (Storage::F32, classify_f32(name))
        } else {
            (Storage::Bf16, TensorClass::Activation)
        };
        out.push(TensorLine {
            name: name.to_string(),
            rows,
            cols,
            storage,
            class,
            layer: layer_of(name),
            bytes: bytes_of(storage, rows * cols),
        });
    }
    out.push(TensorLine {
        name: "emb".to_string(),
        rows: cfg.vocab,
        cols: cfg.d,
        storage: Storage::Q8_0,
        class: TensorClass::Weight,
        layer: None,
        bytes: bytes_of(Storage::Q8_0, cfg.vocab * cfg.d),
    });
    out
}

/// One `Let name be <expr>.` op statement of the authored forward (declarations excluded).
struct OpStmt {
    name: String,
    expr: String,
    layer: Option<usize>,
}

fn op_statements(source: &str) -> Vec<OpStmt> {
    source
        .lines()
        .filter(|l| tensor_declaration(l).is_none())
        .filter_map(|l| {
            let rest = l.strip_prefix("Let ")?;
            let (name, expr) = rest.split_once(" be ")?;
            let expr = expr.strip_suffix('.')?;
            Some(OpStmt {
                name: name.to_string(),
                expr: expr.to_string(),
                layer: layer_of(name),
            })
        })
        .collect()
}

fn idents(expr: &str) -> impl Iterator<Item = &str> {
    expr.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
}

/// The operand after `of`, `over … and`, or the left operand of an infix op — the one whose shape the result
/// takes — except `gated delta`, whose result is as wide as its VALUE operand (after `and`).
fn result_shape(expr: &str, shapes: &BTreeMap<String, (usize, usize)>) -> Option<(usize, usize)> {
    let shape = |name: &str| shapes.get(name).copied();
    if let Some((a, w)) = expr.split_once(" matmul ") {
        return Some((shape(a.trim())?.0, shape(w.trim())?.1));
    }
    if let Some(rest) = expr.strip_prefix("gated delta of ") {
        let (_, after) = rest.split_once(" and ")?;
        return shape(after.split_whitespace().next()?);
    }
    if let Some((_, rest)) = expr.split_once(" of ") {
        return shape(rest.split_whitespace().next()?);
    }
    shape(expr.split_whitespace().next()?)
}

/// The MACs one op statement costs per token, at context position `ctx`, from its operands' shapes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct OpMacs {
    weight: usize,
    attention: usize,
    gdn: usize,
    conv: usize,
}

fn op_macs(expr: &str, shapes: &BTreeMap<String, (usize, usize)>, ctx: usize) -> OpMacs {
    let shape = |name: &str| shapes.get(name).copied().unwrap_or((0, 0));
    let mut m = OpMacs::default();
    if let Some((a, w)) = expr.split_once(" matmul ") {
        let (ar, _) = shape(a.trim());
        let (wr, wc) = shape(w.trim());
        m.weight = ar * wr * wc;
    } else if let Some(rest) = expr.strip_prefix("attention of ") {
        let q = rest.split_whitespace().next().unwrap_or("");
        let (qr, qc) = shape(q);
        m.attention = 2 * qr * qc * ctx;
    } else if let Some(rest) = expr.strip_prefix("gated delta of ") {
        let state = rest
            .split(" state ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .unwrap_or("");
        let (sr, sc) = shape(state);
        m.gdn = 4 * sr * sc;
    } else if let Some(rest) = expr.strip_prefix("causal conv of ") {
        let a = rest.split_whitespace().next().unwrap_or("");
        let taps: usize = rest
            .split(" over ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|t| t.parse().ok())
            .unwrap_or(0);
        let (ar, ac) = shape(a);
        m.conv = taps * ar * ac;
    }
    m
}

/// The peak live set of activations (bf16) per statement index, from a liveness walk over the op chain:
/// an activation is live from its definition to its last use.
fn live_set_peaks(
    stmts: &[OpStmt],
    shapes: &BTreeMap<String, (usize, usize)>,
    hidden: &str,
) -> Vec<usize> {
    let mut size: BTreeMap<&str, usize> = BTreeMap::new();
    let mut def: BTreeMap<&str, usize> = BTreeMap::new();
    let mut last: BTreeMap<&str, usize> = BTreeMap::new();
    if let Some(&(r, c)) = shapes.get(hidden) {
        size.insert(hidden, r * c * BF16_BYTES);
        def.insert(hidden, 0);
        last.insert(hidden, 0);
    }
    for (i, s) in stmts.iter().enumerate() {
        let (r, c) = shapes.get(&s.name).copied().unwrap_or((0, 0));
        size.insert(&s.name, r * c * BF16_BYTES);
        def.insert(&s.name, i);
        last.entry(&s.name).or_insert(i);
        for used in idents(&s.expr) {
            if size.contains_key(used) && used != s.name.as_str() {
                last.insert(used, i);
            }
        }
    }
    (0..stmts.len())
        .map(|i| {
            size.iter()
                .filter(|(n, _)| def[*n] <= i && last[*n] >= i)
                .map(|(_, b)| *b)
                .sum()
        })
        .collect()
}

// ── The Qwen3.8 fit ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CfgBlock {
    pub d: usize,
    pub layers: usize,
    pub gdn_layers: usize,
    pub attn_layers: usize,
    pub full_attn_every: usize,
    pub dff: usize,
    pub vocab: usize,
    pub h: usize,
    pub kv: usize,
    pub hd: usize,
    pub rope_dim: usize,
    pub rope_theta: f64,
    pub rms_eps: f64,
    pub gdn_k_heads: usize,
    pub gdn_v_heads: usize,
    pub gdn_head_dim: usize,
    pub conv_taps: usize,
    pub ctx_len: usize,
    pub provenance: &'static str,
}

impl CfgBlock {
    pub fn of(c: &Qwen38Cfg) -> CfgBlock {
        CfgBlock {
            d: c.d,
            layers: c.layers,
            gdn_layers: c.layers - c.attn_layers(),
            attn_layers: c.attn_layers(),
            full_attn_every: c.full_attn_every,
            dff: c.dff,
            vocab: c.vocab,
            h: c.h,
            kv: c.kv,
            hd: c.hd,
            rope_dim: c.rope_dim,
            rope_theta: c.rope_theta,
            rms_eps: c.rms_eps,
            gdn_k_heads: c.gdn_k_heads,
            gdn_v_heads: c.gdn_v_heads,
            gdn_head_dim: c.gdn_head_dim,
            conv_taps: c.conv_taps,
            ctx_len: c.ctx_len,
            provenance: PROVENANCE_MODELED,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EmbeddingBlock {
    pub rows: usize,
    pub cols: usize,
    pub storage: Storage,
    pub bytes: usize,
    pub note: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WeightsBlock {
    pub params: usize,
    pub tensors: usize,
    pub q8_0_tensors: usize,
    pub f32_tensors: usize,
    /// Σ Q8_0 matrices: elems + one f16 scale per 32.
    pub q8_0_bytes: usize,
    /// Σ f32 tensors × 4.
    pub f32_bytes: usize,
    /// The on-disk Q8_0 checkpoint, minus what is excluded.
    pub gguf_bytes: usize,
    /// The deployment dtype: every parameter × 2.
    pub bf16_bytes: usize,
    pub embedding: EmbeddingBlock,
    pub excluded: Vec<&'static str>,
    pub q8_0_layout: &'static str,
    pub provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelBlock {
    pub id: &'static str,
    pub hf_model_type: &'static str,
    pub inherits: &'static str,
    pub deployment_dtype: &'static str,
    pub gguf: &'static str,
    pub config: CfgBlock,
    pub weights: WeightsBlock,
    pub tensors: Vec<TensorLine>,
    pub provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LayerRow {
    pub layer: usize,
    pub kind: &'static str,
    pub weight_params: usize,
    pub weight_bytes_bf16: usize,
    pub weight_bytes_gguf: usize,
    /// One MAC per weight element of every `matmul`, at batch 1.
    pub weight_macs: usize,
    /// Scores and the weighted V-sum over `ctx` positions, every head.
    pub attention_macs_at_ctx: usize,
    /// Four ghd² passes over every v-head's ghd×ghd memory: decay, S·k, the rank-1 update, S·q.
    pub gdn_recurrence_macs: usize,
    /// Taps × the q|k|v channels.
    pub conv_macs: usize,
    pub macs: usize,
    pub arithmetic_intensity_macs_per_byte: f64,
    pub activations_peak_bytes: usize,
    pub provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LmHeadRow {
    pub weight_params: usize,
    pub weight_bytes_bf16: usize,
    pub weight_bytes_gguf: usize,
    pub macs: usize,
    pub arithmetic_intensity_macs_per_byte: f64,
    pub provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TotalsBlock {
    pub macs_per_token_at_ctx: usize,
    pub weight_macs: usize,
    pub attention_macs_at_ctx: usize,
    pub gdn_recurrence_macs: usize,
    pub conv_macs: usize,
    pub lm_head_macs: usize,
    pub provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KvAtCtx {
    pub ctx: usize,
    pub bytes: usize,
    /// The KV cache shards over kv heads, never wider.
    pub kv_shards: usize,
    /// Chips holding the same shard once TP exceeds the kv heads.
    pub replication: usize,
    pub per_chip_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TpRow {
    pub chips: usize,
    pub sharded_weight_bytes_per_chip: usize,
    pub replicated_weight_bytes: usize,
    pub weights_per_chip_bytes: usize,
    pub state_bytes_per_chip: usize,
    pub kv_per_chip_bytes: usize,
    pub hbm_need_bytes: usize,
    pub hbm_bytes: usize,
    pub hbm_fit: bool,
    pub hbm_headroom_bytes: i64,
    pub max_ctx_in_hbm: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TensorParallelBlock {
    pub chips: usize,
    pub dtype: &'static str,
    pub sharded_weight_bytes_per_chip: usize,
    pub replicated_weight_bytes: usize,
    pub weights_per_chip_bytes: usize,
    pub state_bytes_per_chip: usize,
    pub kv_cache_at_ctx: KvAtCtx,
    pub hbm_bytes_per_chip: usize,
    pub hbm_need_bytes: usize,
    pub hbm_fit: bool,
    pub hbm_headroom_bytes: i64,
    pub max_ctx_in_hbm: usize,
    pub ladder: Vec<TpRow>,
    pub sharding: &'static str,
    pub provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct KvTier {
    pub sram_until_ctx: usize,
    pub then: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Tiers {
    pub weights: &'static str,
    pub activations: &'static str,
    pub gdn_state: &'static str,
    pub conv_state: &'static str,
    pub kv_cache: KvTier,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ResidencyBlock {
    pub sram_bytes: usize,
    /// The largest live set of a layer, bf16 — from a liveness walk over the authored forward.
    pub activations_per_token_bytes: usize,
    pub activations_peak_layer: usize,
    pub activations_peak_layer_kind: &'static str,
    pub activations_fit_sram: bool,
    /// The live set at the LM head — the logits row beside the normed hidden state, bf16 — and its
    /// vocab-parallel share per chip.
    pub lm_head_live_set_bytes: usize,
    pub lm_head_live_set_per_chip_bytes: usize,
    pub gdn_state_bytes: usize,
    pub gdn_state_bytes_per_chip: usize,
    pub conv_state_bytes: usize,
    pub conv_state_bytes_per_chip: usize,
    pub kv_cache_bytes_per_token: usize,
    pub kv_cache_bytes_per_token_per_chip: usize,
    /// Activations + the GDN memory + the conv rings on one chip.
    pub recurrent_working_set_per_chip_bytes: usize,
    /// The recurrent working set, or the LM head's moment, whichever is larger, fits the SRAM.
    pub sram_fit_per_chip: bool,
    pub tiers: Tiers,
    pub provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Qwen38Fit {
    pub schema: &'static str,
    pub version: u32,
    #[serde(rename = "_generated_by")]
    pub generated_by: &'static str,
    pub provenance: &'static str,
    pub no_speed_claim: bool,
    pub honesty: &'static str,
    pub model: ModelBlock,
    pub chip: ChipBlock,
    pub tensor_parallel: TensorParallelBlock,
    pub residency: ResidencyBlock,
    pub per_layer: Vec<LayerRow>,
    pub lm_head: LmHeadRow,
    pub totals: TotalsBlock,
    pub ii_report: IiBlock,
}

fn tp_row(
    chips: usize,
    cfg: &Qwen38Cfg,
    sharded_bf16: usize,
    replicated_bf16: usize,
    gdn_state: usize,
    conv_state: usize,
    kv_per_token: usize,
    ctx: usize,
    hbm: usize,
) -> TpRow {
    let sharded_weight_bytes_per_chip = sharded_bf16 / chips;
    let weights_per_chip_bytes = sharded_weight_bytes_per_chip + replicated_bf16;
    let state_bytes_per_chip = gdn_state / chips.min(cfg.gdn_v_heads) + conv_state / chips;
    let kv_shards = chips.min(cfg.kv);
    let kv_per_chip_bytes = kv_per_token * ctx / kv_shards;
    let hbm_need_bytes = weights_per_chip_bytes + state_bytes_per_chip + kv_per_chip_bytes;
    let resident = weights_per_chip_bytes + state_bytes_per_chip;
    TpRow {
        chips,
        sharded_weight_bytes_per_chip,
        replicated_weight_bytes: replicated_bf16,
        weights_per_chip_bytes,
        state_bytes_per_chip,
        kv_per_chip_bytes,
        hbm_need_bytes,
        hbm_bytes: hbm,
        hbm_fit: hbm_need_bytes <= hbm,
        hbm_headroom_bytes: hbm as i64 - hbm_need_bytes as i64,
        max_ctx_in_hbm: hbm.saturating_sub(resident) / (kv_per_token / kv_shards).max(1),
    }
}

/// The tiny-config RMSNorm chain — the first statement of the authored forward at width `d`, as a standalone
/// program the tensor lane lifts.
fn rmsnorm_chain_source(d: usize) -> String {
    let mut s = String::from("## Main\n");
    s.push_str(&format!("Let x be a 1 by {d} tensor of Real.\n"));
    for i in 1..=d {
        s.push_str(&format!("Set item {i} of x to {i}.0.\n"));
    }
    s.push_str(&format!("Let an0 be a 1 by {d} tensor of Real.\n"));
    for i in 1..=d {
        s.push_str(&format!("Set item {i} of an0 to 1.0.\n"));
    }
    s.push_str("Let xn0 be rmsnorm of x with weight an0.\n");
    for i in 1..=d {
        s.push_str(&format!("Show item {i} of xn0.\n"));
    }
    s
}

/// The II block of the RMSNorm chain: at the real width when the chip hosts it, else at the tiny width with
/// the real width's refusal recorded — the block upgrades itself when composition lands.
fn qwen38_ii_block(cfg: &Qwen38Cfg, facts: &RduChipFacts) -> Result<IiBlock, String> {
    let target = facts.spatial_target();
    match compile_to_spatial(&rmsnorm_chain_source(cfg.d), &target) {
        Ok(p) => Ok(IiBlock::of(&rdu_ii_report(&p, facts), format!("the forward's first statement, rmsnorm of x with weight an0, at the real width 1×{} on the {}", cfg.d, target.name))),
        Err(refusal) => {
            let tiny = Qwen38Cfg::tiny();
            let p = compile_to_spatial(&rmsnorm_chain_source(tiny.d), &target).map_err(|e| format!("the {} cannot host the tiny rmsnorm chain: {e:?}", target.name))?;
            Ok(IiBlock::of(
                &rdu_ii_report(&p, facts),
                format!(
                    "the forward's first statement, rmsnorm of x with weight an0, at the tiny config's width 1×{} on the {} — the real width 1×{} is refused by one PCU's budget ({refusal:?}) until sections compose it",
                    tiny.d, target.name, cfg.d
                ),
            ))
        }
    }
}

/// Qwen3.8 on `facts`' chip, sharded over `tp` RDUs, with the KV cache sized at context `ctx`.
pub fn qwen38_fit(
    cfg: &Qwen38Cfg,
    facts: &RduChipFacts,
    tp: usize,
    ctx: usize,
) -> Result<FitReport, String> {
    if tp == 0 {
        return Err("tensor parallel needs at least one chip".into());
    }
    let tensors = qwen38_tensors(cfg);
    let (source, _, _) = gen_qwen38_lg(cfg, "w");
    let mut shapes: BTreeMap<String, (usize, usize)> = tensors
        .iter()
        .map(|t| (t.name.clone(), (t.rows, t.cols)))
        .collect();
    let stmts = op_statements(&source);
    for s in &stmts {
        let shape = result_shape(&s.expr, &shapes)
            .ok_or_else(|| format!("no shape for `{}` = `{}`", s.name, s.expr))?;
        shapes.insert(s.name.clone(), shape);
    }
    let peaks = live_set_peaks(&stmts, &shapes, "x");

    let weights: Vec<&TensorLine> = tensors
        .iter()
        .filter(|t| t.class == TensorClass::Weight)
        .collect();
    let params: usize = weights.iter().map(|t| t.elems()).sum();
    let q8: Vec<&&TensorLine> = weights
        .iter()
        .filter(|t| t.storage == Storage::Q8_0)
        .collect();
    let f32s: Vec<&&TensorLine> = weights
        .iter()
        .filter(|t| t.storage == Storage::F32)
        .collect();
    let q8_0_bytes_total: usize = q8.iter().map(|t| t.bytes).sum();
    let f32_bytes_total: usize = f32s.iter().map(|t| t.bytes).sum();
    let emb = tensors
        .iter()
        .find(|t| t.name == "emb")
        .expect("the embedding line");
    let sharded_bf16: usize = weights
        .iter()
        .filter(|t| t.is_matrix())
        .map(|t| t.elems() * BF16_BYTES)
        .sum();
    let replicated_bf16: usize = weights
        .iter()
        .filter(|t| !t.is_matrix())
        .map(|t| t.elems() * BF16_BYTES)
        .sum();

    let gdn_state: usize = tensors
        .iter()
        .filter(|t| t.class == TensorClass::State && t.name.starts_with("St"))
        .map(|t| t.bytes)
        .sum();
    let conv_state: usize = tensors
        .iter()
        .filter(|t| t.class == TensorClass::State && t.name.starts_with("Cs"))
        .map(|t| t.bytes)
        .sum();
    let kv_per_token = 2 * cfg.akv() * BF16_BYTES * cfg.attn_layers();

    let mut per_layer = Vec::with_capacity(cfg.layers);
    let mut totals = TotalsBlock {
        macs_per_token_at_ctx: 0,
        weight_macs: 0,
        attention_macs_at_ctx: 0,
        gdn_recurrence_macs: 0,
        conv_macs: 0,
        lm_head_macs: 0,
        provenance: PROVENANCE_MODELED,
    };
    for l in 0..cfg.layers {
        let layer_weights: Vec<&&TensorLine> =
            weights.iter().filter(|t| t.layer == Some(l)).collect();
        let weight_params: usize = layer_weights.iter().map(|t| t.elems()).sum();
        let weight_bytes_gguf: usize = layer_weights.iter().map(|t| t.bytes).sum();
        let mut macs = OpMacs::default();
        let mut peak = 0usize;
        for (i, s) in stmts.iter().enumerate().filter(|(_, s)| s.layer == Some(l)) {
            let m = op_macs(&s.expr, &shapes, ctx);
            macs.weight += m.weight;
            macs.attention += m.attention;
            macs.gdn += m.gdn;
            macs.conv += m.conv;
            peak = peak.max(peaks[i]);
        }
        let total = macs.weight + macs.attention + macs.gdn + macs.conv;
        let weight_bytes_bf16 = weight_params * BF16_BYTES;
        totals.weight_macs += macs.weight;
        totals.attention_macs_at_ctx += macs.attention;
        totals.gdn_recurrence_macs += macs.gdn;
        totals.conv_macs += macs.conv;
        per_layer.push(LayerRow {
            layer: l,
            kind: if cfg.is_attn(l) { "attention" } else { "gdn" },
            weight_params,
            weight_bytes_bf16,
            weight_bytes_gguf,
            weight_macs: macs.weight,
            attention_macs_at_ctx: macs.attention,
            gdn_recurrence_macs: macs.gdn,
            conv_macs: macs.conv,
            macs: total,
            arithmetic_intensity_macs_per_byte: total as f64 / weight_bytes_bf16 as f64,
            activations_peak_bytes: peak,
            provenance: PROVENANCE_MODELED,
        });
    }
    let head: Vec<&&TensorLine> = weights
        .iter()
        .filter(|t| t.layer.is_none() && t.name != "emb")
        .collect();
    let head_params: usize = head.iter().map(|t| t.elems()).sum();
    let head_macs: usize = stmts
        .iter()
        .filter(|s| s.layer.is_none())
        .map(|s| op_macs(&s.expr, &shapes, ctx).weight)
        .sum();
    totals.lm_head_macs = head_macs;
    totals.macs_per_token_at_ctx = totals.weight_macs
        + totals.attention_macs_at_ctx
        + totals.gdn_recurrence_macs
        + totals.conv_macs
        + head_macs;
    let lm_head = LmHeadRow {
        weight_params: head_params,
        weight_bytes_bf16: head_params * BF16_BYTES,
        weight_bytes_gguf: head.iter().map(|t| t.bytes).sum(),
        macs: head_macs,
        arithmetic_intensity_macs_per_byte: head_macs as f64 / (head_params * BF16_BYTES) as f64,
        provenance: PROVENANCE_MODELED,
    };

    let hbm = facts.hbm_gib.value * GIB;
    let ladder: Vec<TpRow> = {
        let mut widths = vec![
            1,
            facts.chips_per_node.value,
            default_tensor_parallel(facts),
        ];
        widths.dedup();
        widths
            .into_iter()
            .map(|w| {
                tp_row(
                    w,
                    cfg,
                    sharded_bf16,
                    replicated_bf16,
                    gdn_state,
                    conv_state,
                    kv_per_token,
                    ctx,
                    hbm,
                )
            })
            .collect()
    };
    let here = tp_row(
        tp,
        cfg,
        sharded_bf16,
        replicated_bf16,
        gdn_state,
        conv_state,
        kv_per_token,
        ctx,
        hbm,
    );
    let kv_shards = tp.min(cfg.kv);
    let tensor_parallel = TensorParallelBlock {
        chips: tp,
        dtype: DEPLOYMENT_DTYPE,
        sharded_weight_bytes_per_chip: here.sharded_weight_bytes_per_chip,
        replicated_weight_bytes: here.replicated_weight_bytes,
        weights_per_chip_bytes: here.weights_per_chip_bytes,
        state_bytes_per_chip: here.state_bytes_per_chip,
        kv_cache_at_ctx: KvAtCtx { ctx, bytes: kv_per_token * ctx, kv_shards, replication: tp / kv_shards, per_chip_bytes: here.kv_per_chip_bytes },
        hbm_bytes_per_chip: hbm,
        hbm_need_bytes: here.hbm_need_bytes,
        hbm_fit: here.hbm_fit,
        hbm_headroom_bytes: here.hbm_headroom_bytes,
        max_ctx_in_hbm: here.max_ctx_in_hbm,
        ladder,
        sharding: "every weight matrix is split over the chips (column/row parallel); norm vectors are replicated; the KV cache shards over kv heads and replicates beyond them; the GDN memory shards over v-heads; the conv rings over channels",
        provenance: PROVENANCE_MODELED,
    };

    let sram = facts.sram_bytes();
    let (peak_layer, act) = per_layer
        .iter()
        .map(|r| (r.layer, r.activations_peak_bytes))
        .max_by_key(|(l, b)| (*b, std::cmp::Reverse(*l)))
        .ok_or("a layer")?;
    let lm_head_live: usize = stmts
        .iter()
        .enumerate()
        .filter(|(_, s)| s.layer.is_none())
        .map(|(i, _)| peaks[i])
        .max()
        .unwrap_or(0);
    let lm_head_live_pc =
        lm_head_live.saturating_sub(cfg.vocab * BF16_BYTES) + cfg.vocab * BF16_BYTES / tp;
    let gdn_pc = gdn_state / tp.min(cfg.gdn_v_heads);
    let conv_pc = conv_state / tp;
    let recurrent = act + gdn_pc + conv_pc;
    let momentary = act.max(lm_head_live_pc) + gdn_pc + conv_pc;
    let kv_per_token_pc = kv_per_token / kv_shards;
    let free = sram.saturating_sub(recurrent);
    let tier = |bytes: usize| if bytes <= sram { "sram" } else { "hbm" };
    let residency = ResidencyBlock {
        sram_bytes: sram,
        activations_per_token_bytes: act,
        activations_peak_layer: peak_layer,
        activations_peak_layer_kind: if cfg.is_attn(peak_layer) {
            "attention"
        } else {
            "gdn"
        },
        activations_fit_sram: act <= sram,
        lm_head_live_set_bytes: lm_head_live,
        lm_head_live_set_per_chip_bytes: lm_head_live_pc,
        gdn_state_bytes: gdn_state,
        gdn_state_bytes_per_chip: gdn_pc,
        conv_state_bytes: conv_state,
        conv_state_bytes_per_chip: conv_pc,
        kv_cache_bytes_per_token: kv_per_token,
        kv_cache_bytes_per_token_per_chip: kv_per_token_pc,
        recurrent_working_set_per_chip_bytes: recurrent,
        sram_fit_per_chip: momentary <= sram,
        tiers: Tiers {
            weights: "hbm",
            activations: tier(act),
            gdn_state: tier(act + gdn_pc),
            conv_state: tier(recurrent),
            kv_cache: KvTier {
                sram_until_ctx: free / kv_per_token_pc.max(1),
                then: "hbm",
            },
        },
        provenance: PROVENANCE_MODELED,
    };

    Ok(FitReport::Qwen38(Qwen38Fit {
        schema: FIT_SCHEMA,
        version: FIT_VERSION,
        generated_by: QWEN38_GENERATED_BY,
        provenance: PROVENANCE_MODELED,
        no_speed_claim: true,
        honesty: HONESTY,
        model: ModelBlock {
            id: QWEN38_MODEL_ID,
            hf_model_type: QWEN38_HF_MODEL_TYPE,
            inherits: QWEN38_HF_INHERITS,
            deployment_dtype: DEPLOYMENT_DTYPE,
            gguf: QWEN38_GGUF,
            config: CfgBlock::of(cfg),
            weights: WeightsBlock {
                params,
                tensors: weights.len(),
                q8_0_tensors: q8.len(),
                f32_tensors: f32s.len(),
                q8_0_bytes: q8_0_bytes_total,
                f32_bytes: f32_bytes_total,
                gguf_bytes: q8_0_bytes_total + f32_bytes_total,
                bf16_bytes: params * BF16_BYTES,
                embedding: EmbeddingBlock {
                    rows: emb.rows,
                    cols: emb.cols,
                    storage: emb.storage,
                    bytes: emb.bytes,
                    note: "not declared by the single-token forward (the step gathers one row of it); resident for decode",
                },
                excluded: vec!["the MTP (nextn) draft block: not used by greedy decode", "the rope tables and the recurrent state: not weights (listed under residency)"],
                q8_0_layout: Q8_0_LAYOUT,
                provenance: PROVENANCE_MODELED,
            },
            tensors,
            provenance: PROVENANCE_MODELED,
        },
        chip: ChipBlock::of(facts),
        tensor_parallel,
        residency,
        per_layer,
        lm_head,
        totals,
        ii_report: qwen38_ii_block(cfg, facts)?,
    }))
}

// ── The two reports, one type ───────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum FitReport {
    Gemm(GemmFit),
    Qwen38(Qwen38Fit),
}

impl FitReport {
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("a fit report serializes")
    }
    pub fn as_qwen38(&self) -> Option<&Qwen38Fit> {
        match self {
            FitReport::Qwen38(q) => Some(q),
            FitReport::Gemm(_) => None,
        }
    }
    pub fn as_gemm(&self) -> Option<&GemmFit> {
        match self {
            FitReport::Gemm(g) => Some(g),
            FitReport::Qwen38(_) => None,
        }
    }
}

// ── The vLLM plan ───────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VllmModel {
    pub id: &'static str,
    pub hf_model_type: &'static str,
    pub inherits: &'static str,
    pub checkpoint: &'static str,
    pub gguf_alternative: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VllmServe {
    pub tensor_parallel_size: usize,
    pub dtype: &'static str,
    pub max_model_len: usize,
    pub command: String,
    pub provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VllmChip {
    pub part: &'static str,
    pub chip: RduChip,
    pub chips_per_node: Cited<usize>,
    pub chips_per_rack: Option<Cited<usize>>,
    pub tensor_parallel_width: Cited<usize>,
    pub provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StatePerLayer {
    pub attention_kv_per_token: usize,
    pub gdn_memory: usize,
    pub conv_rings: usize,
    pub provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FromFit {
    pub weights_bf16_bytes: usize,
    pub weights_per_chip_bytes: usize,
    pub hbm_fit: bool,
    pub kv_cache_at_ctx: KvAtCtx,
    pub max_ctx_in_hbm: usize,
    pub sram_until_ctx: usize,
    pub provenance: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VllmPlan {
    pub schema: &'static str,
    pub version: u32,
    #[serde(rename = "_generated_by")]
    pub generated_by: &'static str,
    pub provenance: &'static str,
    pub no_speed_claim: bool,
    pub honesty: &'static str,
    pub model: VllmModel,
    pub serve: VllmServe,
    pub chip: VllmChip,
    pub ops_the_backend_must_support: Vec<Value>,
    pub state_bytes_per_layer: StatePerLayer,
    pub from_fit: FromFit,
    pub intake_question: &'static str,
    pub fallbacks: Vec<&'static str>,
}

/// The ops the authored forward is made of, with the shapes the backend must support — every number from the config.
pub fn qwen38_op_list(cfg: &Qwen38Cfg) -> Vec<Value> {
    let gdn_layers = cfg.layers - cfg.attn_layers();
    vec![
        json!({ "op": "rmsnorm", "eps": cfg.rms_eps, "width": cfg.d, "per_head_variants": { "q": cfg.hd, "k": cfg.hd }, "provenance": PROVENANCE_MODELED }),
        json!({ "op": "matmul", "dtype": DEPLOYMENT_DTYPE, "widest": { "rows": cfg.d, "cols": cfg.vocab }, "provenance": PROVENANCE_MODELED }),
        json!({ "op": "rope", "rotary_dims": cfg.rope_dim, "head_dim": cfg.hd, "theta": cfg.rope_theta, "note": "partial rotary: only the first rotary_dims of each head rotate", "provenance": PROVENANCE_MODELED }),
        json!({ "op": "gqa_attention", "heads": cfg.h, "kv_heads": cfg.kv, "head_dim": cfg.hd, "layers": cfg.attn_layers(), "causal": true, "provenance": PROVENANCE_MODELED }),
        json!({ "op": "sigmoid_gate", "width": cfg.aq(), "note": "the attention output gate, hadamard with the context", "provenance": PROVENANCE_MODELED }),
        json!({ "op": "causal_conv", "taps": cfg.conv_taps, "channels": 2 * cfg.gdn_qk() + cfg.gdn_v(), "layers": gdn_layers, "provenance": PROVENANCE_MODELED }),
        json!({ "op": "silu", "provenance": PROVENANCE_MODELED }),
        json!({ "op": "l2norm_per_head", "heads": cfg.gdn_k_heads, "head_dim": cfg.gdn_head_dim, "provenance": PROVENANCE_MODELED }),
        json!({ "op": "decay", "heads": cfg.gdn_v_heads, "note": "exp(A · softplus(u + dt_bias)) — the per-head forget gate", "provenance": PROVENANCE_MODELED }),
        json!({ "op": "gated_delta", "v_heads": cfg.gdn_v_heads, "k_heads": cfg.gdn_k_heads, "head_dim": cfg.gdn_head_dim, "layers": gdn_layers, "state_elems_per_layer": cfg.gdn_v() * cfg.gdn_head_dim, "provenance": PROVENANCE_MODELED }),
        json!({ "op": "swiglu", "dff": cfg.dff, "provenance": PROVENANCE_MODELED }),
        json!({ "op": "argmax", "vocab": cfg.vocab, "note": "greedy next token", "provenance": PROVENANCE_MODELED }),
    ]
}

/// The plan the fit implies for the vendor's vLLM backend at the model's full context.
pub fn vllm_plan(cfg: &Qwen38Cfg, facts: &RduChipFacts, tp: usize) -> Result<VllmPlan, String> {
    let fit = qwen38_fit(cfg, facts, tp, cfg.ctx_len)?;
    let q = fit.as_qwen38().ok_or("a Qwen3.8 fit")?;
    let chip = ChipBlock::of(facts);
    let gdn_layers = cfg.layers - cfg.attn_layers();
    Ok(VllmPlan {
        schema: VLLM_PLAN_SCHEMA,
        version: FIT_VERSION,
        generated_by: QWEN38_GENERATED_BY,
        provenance: PROVENANCE_MODELED,
        no_speed_claim: true,
        honesty: HONESTY,
        model: VllmModel {
            id: QWEN38_MODEL_ID,
            hf_model_type: QWEN38_HF_MODEL_TYPE,
            inherits: QWEN38_HF_INHERITS,
            checkpoint: "safetensors, bf16",
            gguf_alternative: QWEN38_GGUF,
        },
        serve: VllmServe {
            tensor_parallel_size: tp,
            dtype: DEPLOYMENT_DTYPE,
            max_model_len: cfg.ctx_len,
            command: format!("vllm serve {QWEN38_MODEL_ID} --tensor-parallel-size {tp} --dtype {DEPLOYMENT_DTYPE} --max-model-len {}", cfg.ctx_len),
            provenance: PROVENANCE_MODELED,
        },
        chip: VllmChip {
            part: chip.part,
            chip: chip.chip,
            chips_per_node: chip.chips_per_node,
            chips_per_rack: chip.chips_per_rack,
            tensor_parallel_width: chip.tensor_parallel_width,
            provenance: PROVENANCE_MODELED,
        },
        ops_the_backend_must_support: qwen38_op_list(cfg),
        state_bytes_per_layer: StatePerLayer {
            attention_kv_per_token: q.residency.kv_cache_bytes_per_token / cfg.attn_layers(),
            gdn_memory: q.residency.gdn_state_bytes / gdn_layers,
            conv_rings: q.residency.conv_state_bytes / gdn_layers,
            provenance: PROVENANCE_MODELED,
        },
        from_fit: FromFit {
            weights_bf16_bytes: q.model.weights.bf16_bytes,
            weights_per_chip_bytes: q.tensor_parallel.weights_per_chip_bytes,
            hbm_fit: q.tensor_parallel.hbm_fit,
            kv_cache_at_ctx: q.tensor_parallel.kv_cache_at_ctx.clone(),
            max_ctx_in_hbm: q.tensor_parallel.max_ctx_in_hbm,
            sram_until_ctx: q.residency.tiers.kv_cache.sram_until_ctx,
            provenance: PROVENANCE_MODELED,
        },
        intake_question: "does the RDU vLLM backend support HF model_type qwen3_5 (Gated DeltaNet: causal_conv + decay + gated_delta with a per-layer ghd×ghd memory per v-head)? If not, which of these ops is missing?",
        fallbacks: vec![
            "SambaFlow: `largo emit rdu --format sambaflow` on the tiny config first, then the real one (PEF compile time is the risk)",
            "RDA/Spatial-IR: `largo emit rdu` neutral.json v2 + certificate; `--format spatial` for the Spatial source",
            "API only: the SambaCloud baseline plus the `largo emit rdu` demo stand on their own",
        ],
    })
}
