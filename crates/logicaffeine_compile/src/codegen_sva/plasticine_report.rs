//! The **Plasticine report** — P4 of the Plasticine lane: the artifact an RDU engineer reads, generated from
//! the model so no number in it is typed by hand.
//!
//! One row per (workload, target): PCU/PMU used vs total, FU utilization, the initiation interval of every
//! metapipeline stage with the critical stage flagged, the pipeline II, N-buffer depths and PMU counts, hop
//! counts and static-routing occupancy against the Spade link budget, iterations, fill + steady-state
//! cycles, the DRAM roofline (bytes moved, channel bandwidth, the bound), throughput at the II, and for
//! GEMM GFLOPS and % of the published fp32 peak. A counted-loop row also carries its VM anchor (the VM's
//! answer == the fabric's == the cycle-timed pipelined run at the modeled II, hazards one cycle below) and,
//! for an ALU-only body, the re-checkable SAT certificate of its routed datapath.
//!
//! Provenance is `modeled` on every row: the numbers are the paper-parameterized descriptors' ([`PlasticineTarget`])
//! under Spatial's II definition ([`super::metapipeline`]), the PMU banking model ([`super::pmu`]) and the
//! checkerboard placer ([`super::plasticine_place`]). Nothing here ran on a Plasticine simulator or silicon;
//! a Tungsten run lands beside these rows under its own provenance (P5).
//!
//! `benchmarks/silicon/plasticine_report.json` is written ONLY by the private place-and-route engine's report
//! emitter (`emit_plasticine_report`), which installs the engine's certified negotiated router before placing
//! anything, and pinned byte-equal by `plasticine_report_lock`.

use std::panic::{catch_unwind, AssertUnwindSafe};

use serde_json::{json, Value};

use super::choreography::{synthesize, Choreography};
use super::fabric_grid::route_loop_body;
use super::fabric_rtl::{
    eval_loop_fabric, loop_fabric_from_source, FabricOp, LoopCell, LoopFabric, Src,
};
use super::fusion::{share_operand, OperandStream, Sharing};
use super::metapipeline::{eval_loop_fabric_at_ii, loop_stage, Metapipeline, Stage, UnitModel};
use super::plasticine::PlasticineTarget;
use super::plasticine_place::{
    max_par, max_routable_par_within, place_unrolled, words_per_pmu, LinkBudget, Placement,
};
use super::pmu::{address_datapath_stages, memory_ii, strided_banking_for, AffineAccess, PmuSpec};
use super::pnr_certificate::{certify_external_placement, PlacementCertificate};
use super::spatial_emit::{
    emit_spatial_counted_loop, emit_spatial_counted_loop_test, emit_spatial_gda_test, emit_spatial_gemm_test,
    emit_spatial_kmeans_test, emit_spatial_outer_test, emit_spatial_zip_map_test, emit_spatial_zip_sum_test, GdaColumn,
    GdaPar, KmeansPar, MapColumn, MapPar, OuterPar, ZipColumn, ZipPar,
};
use crate::tensor_recognize::{Axis, ZipOp, ZipSum, ZipTerm};
use crate::compile::vm_outcome;

/// The provenance every row carries: a paper-parameterized model, never a simulation or silicon.
pub const PROVENANCE_MODELED: &str = "modeled";
/// Where the generated report lives (repo-relative).
pub const REPORT_PATH: &str = "benchmarks/silicon/plasticine_report.json";
/// The bit width the routed-datapath certificate is proven at (the bounded fragment).
pub const CERTIFICATE_WIDTH: u32 = 4;
/// The public toolchain's ledger (written only by `scripts/accel/plasticine-gate-ledger.py`): Tungsten simulations.
pub const TOOLCHAIN_LEDGER_PATH: &str = "benchmarks/silicon/plasticine_toolchain.json";
/// The calibration file joining each toolchain row with our MODELED counterpart (generated with the report).
pub const CALIBRATION_PATH: &str = "benchmarks/silicon/plasticine_calibration.json";
/// The provenance of a toolchain-simulated row.
pub const PROVENANCE_SIMULATED: &str = "simulated_tungsten";

/// What a workload is.
#[derive(Clone, Debug, PartialEq)]
pub enum WorkloadKind {
    /// A LOGOS counted-loop function `fn_name(params…) -> Int` in `source`, lowered through
    /// [`loop_fabric_from_source`] and modeled as one metapipeline stage.
    CountedLoop {
        source: String,
        fn_name: String,
        params: Vec<i64>,
    },
    /// A tiled fp32 GEMM `C[m×n] = A[m×k] · B[k×n]`: `tile` (None = the largest square tile whose
    /// double-buffered A+B panel fits one PMU) and `par` (None = the widest unroll that fits the mesh).
    Gemm {
        m: usize,
        n: usize,
        k: usize,
        tile: Option<usize>,
        par: Option<usize>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Workload {
    pub name: String,
    pub kind: WorkloadKind,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct StageRow {
    pub name: String,
    /// `pcu` (a PCU chain) or `ag` (a perimeter address generator: no PCU).
    pub host: String,
    pub resource_ii: usize,
    pub recurrence_ii: usize,
    pub memory_ii: usize,
    pub ii: usize,
    pub latency: usize,
    pub cells: usize,
    pub pcus: usize,
    pub critical: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BufferRow {
    pub writer: usize,
    pub reader: usize,
    pub depth: usize,
    pub words: usize,
    pub pmus: usize,
}

/// One operand of a GEMM and how the compiler decided to feed the unrolled instances.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OperandRow {
    pub name: String,
    /// `replicated` or `broadcast`.
    pub sharing: String,
    pub consumers: usize,
    pub bytes_per_copy: usize,
    /// Copies actually fetched from memory.
    pub copies: usize,
    pub bytes_moved: usize,
    pub dram_cycles_saved: usize,
    pub fabric_cycles_added: usize,
    pub why: String,
}

/// One channel's control plane: the credit solve beside Spatial's N-buffer rule, and which one binds.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChannelRow {
    pub writer: usize,
    pub reader: usize,
    pub hops: usize,
    pub forward_latency: usize,
    pub backward_latency: usize,
    /// Spatial's published writer-to-reader rule.
    pub n_buffer_depth: usize,
    /// The bandwidth-delay solve: `ceil((forward + backward) / II)`.
    pub credit_depth: usize,
    /// What ships: the deeper requirement.
    pub depth: usize,
    /// `credits` when the rate solve binds, `n_buffer` when the liveness rule does.
    pub binding: String,
}

/// The control plane as a whole: two nets per channel, budgeted on the scalar network.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ControlRow {
    pub nets: usize,
    pub hops: usize,
    pub max_scalar_occupancy: usize,
    pub scalar_link_budget: usize,
    pub routed_statically: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct VmAnchor {
    pub params: Vec<i64>,
    pub vm: i64,
    pub fabric: i64,
    pub pipelined_at_ii: i64,
    pub hazards_at_ii: usize,
    /// Hazards one cycle below the modeled II (`None` when the II is 1 — nothing below it).
    pub hazards_below_ii: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CertificateRow {
    pub steps: usize,
    pub vars: usize,
    pub proves: String,
    pub rechecked: bool,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct GemmRow {
    pub m: usize,
    pub n: usize,
    pub k: usize,
    pub tile_m: usize,
    pub tile_n: usize,
    pub col_tiles: usize,
    pub row_tiles: usize,
    pub k_chunks: usize,
    pub iters_per_tile: usize,
    pub lanes: usize,
    /// The reduction chunk the operands stream in: Spatial's `ts` along K, or the whole K for a tile whose
    /// panels are streamed whole.
    #[serde(default)]
    pub k_tile: usize,
    /// Reduction chunks run side by side (`loop_kk`, a dot product's `op`).
    #[serde(default)]
    pub kk: usize,
    /// One chunk's operand load, at the reduce's lane width, and one chunk's MAC iterations.
    #[serde(default)]
    pub chunk_load_cycles: usize,
    #[serde(default)]
    pub chunk_compute_cycles: usize,
    /// The chunks one instance runs one after another, over every tile it owns.
    #[serde(default)]
    pub chunks_in_sequence: usize,
}

/// One (workload, target) row of the report.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WorkloadRow {
    pub name: String,
    /// `counted_loop` or `gemm`.
    pub kind: String,
    pub target: String,
    pub provenance: String,
    /// Unrolled instances placed (a counted loop's recurrence forbids unrolling: 1).
    pub par: usize,
    /// The widest unroll the mesh's PCU/PMU capacity allows — `par` is at most this; when it is less, the
    /// static vector network, not capacity, bounded the unroll.
    pub par_capacity: usize,
    /// The private place-and-route engine's rung that produced the routes (`pnr-negotiated`, `pnr-exact`), or why
    /// none did.
    pub router: String,
    pub stages: Vec<StageRow>,
    pub pipeline_ii: usize,
    pub critical_stage: String,
    pub buffers: Vec<BufferRow>,
    pub pcu_used: usize,
    pub pcu_total: usize,
    pub pcu_utilization_pct: f64,
    pub pmu_used: usize,
    pub pmu_total: usize,
    pub pmu_utilization_pct: f64,
    pub fu_utilization_pct: f64,
    /// Address-generator stages across every unrolled instance.
    pub ag_stages: usize,
    pub total_hops: usize,
    pub max_hops: usize,
    pub max_link_occupancy: usize,
    pub vector_links: usize,
    pub routed_statically: bool,
    /// Iterations one unrolled instance runs.
    pub iterations: usize,
    pub fill_cycles: usize,
    pub compute_cycles: usize,
    /// Cycles the MAC waits for operands on top of `compute_cycles`: a K-chunked reduction's first chunk loads
    /// before any compute starts, and a chunk whose load outlasts its compute stalls for the difference.
    #[serde(default)]
    pub operand_stall_cycles: usize,
    pub dram_bytes: usize,
    pub dram_cycles: usize,
    pub modeled_cycles: usize,
    /// `compute` or `dram`.
    pub bound: String,
    pub throughput_iters_per_cycle: f64,
    pub gflops: Option<f64>,
    pub pct_of_peak_fp32: Option<f64>,
    /// Per operand: how the compiler decided to feed the unrolled instances, and what it cost.
    #[serde(default)]
    pub operands: Option<Vec<OperandRow>>,
    /// The synthesized control plane: one entry per inter-stage channel.
    #[serde(default)]
    pub channels: Vec<ChannelRow>,
    /// The control nets those channels need, and whether they route.
    #[serde(default)]
    pub control: Option<ControlRow>,
    pub certificate: Option<CertificateRow>,
    pub certificate_omitted: Option<String>,
    pub vm_anchor: Option<VmAnchor>,
    pub gemm: Option<GemmRow>,
}

/// The placed, certified artifact of one counted loop on a Plasticine target — what `largo emit plasticine`
/// writes and [`crate::compile::compile_to_plasticine`] returns.
#[derive(Debug)]
pub struct PlasticineArtifact {
    pub fn_name: String,
    pub target: &'static str,
    pub loop_fabric: LoopFabric,
    pub metapipeline: Metapipeline,
    pub placement: Placement,
    pub row: WorkloadRow,
    pub certificate: Option<PlacementCertificate>,
    /// Spatial source for the open toolchain.
    pub spatial: String,
    /// The same loop as a Spatial regression-harness app (`class Logos_<fn> extends SpatialTest`) at the
    /// VM anchor's inputs, checking the VM's answer — what the toolchain gate runs.
    pub spatial_test: String,
}

impl PlasticineArtifact {
    /// The placement as pretty JSON (trailing newline).
    pub fn placement_json(&self) -> String {
        format!(
            "{}\n",
            serde_json::to_string_pretty(&self.placement).expect("a placement serializes")
        )
    }
    /// The report row as pretty JSON (trailing newline).
    pub fn row_json(&self) -> String {
        format!(
            "{}\n",
            serde_json::to_string_pretty(&self.row).expect("a row serializes")
        )
    }
}

fn r3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn pct(x: f64) -> f64 {
    r3(x * 100.0)
}

fn ceil_div(a: usize, b: usize) -> usize {
    (a + b.max(1) - 1) / b.max(1)
}

/// Run `f`, turning a panic into an `Err` carrying its message, with the default panic printer silenced for
/// the call (the fabric lowerings panic on shapes outside their family — that is their honest boundary, and
/// here it becomes a typed refusal instead of noise).
fn quietly<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = catch_unwind(AssertUnwindSafe(f));
    std::panic::set_hook(prev);
    r.map_err(|p| {
        p.downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "the lowering panicked".to_string())
    })
}

/// `source` with its `## Main` section removed (up to the next `## ` header), so the VM anchor can append
/// its own `Main` that shows the function under test.
fn without_main(source: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in source.lines() {
        if line.trim() == "## Main" {
            skipping = true;
            continue;
        }
        if skipping && line.starts_with("## ") {
            skipping = false;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn vm_value(source: &str, fn_name: &str, params: &[i64]) -> Result<i64, String> {
    let args = params
        .iter()
        .map(|p| {
            if *p < 0 {
                format!("(0 - {})", -p)
            } else {
                p.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let prog = format!(
        "{}## Main\n    Show {fn_name}({args}).\n",
        without_main(source)
    );
    let out = vm_outcome(&prog);
    if let Some(e) = out.error {
        return Err(format!("the VM errored running `{fn_name}({args})`: {e:?}"));
    }
    out.output
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .ok_or_else(|| {
            format!(
                "the VM printed no integer for `{fn_name}({args})`: {:?}",
                out.output
            )
        })
}

/// Certify the routed body datapath of `lf` (output 0 ≡ the next value of state register 0) at
/// [`CERTIFICATE_WIDTH`] bits — only an ALU-only body whose next-state-0 is a body value lies in the bounded
/// fragment; anything else is reported as omitted, with the reason.
fn certify_body(
    lf: &LoopFabric,
) -> (
    Option<PlacementCertificate>,
    Option<CertificateRow>,
    Option<String>,
) {
    let mut ops = Vec::with_capacity(lf.body.len());
    for cell in &lf.body {
        match cell {
            LoopCell::Arith { op, a, b } => ops.push(FabricOp { op: *op, a: *a, b: *b }),
            LoopCell::Select { .. } => return (None, None, Some("the body has a predicated select — outside the bounded ALU fragment the certificate covers".into())),
            LoopCell::Func { unit, .. } => return (None, None, Some(format!("the body has a `{unit:?}` functional unit — outside the bounded ALU fragment the certificate covers"))),
            _ => return (None, None, Some("the body has memory tiles — outside the bounded ALU fragment the certificate covers".into())),
        }
    }
    let Some(Src::Val(result)) = lf.next.first().copied() else {
        return (None, None, Some("next-state 0 is not a body value (the register holds or copies an input) — nothing to certify".into()));
    };
    let n_body_in = lf.n_state + 1 + lf.invariants.len();
    let grid = route_loop_body(&lf.body, n_body_in, &lf.next);
    match certify_external_placement(&grid, &ops, result, CERTIFICATE_WIDTH) {
        Some(cert) => {
            let rechecked = cert.recheck();
            let row = CertificateRow {
                steps: cert.len(),
                vars: cert.num_vars(),
                proves: format!(
                    "the routed body grid's output 0 ≡ the next-state-0 datapath ({} ALU cells) for every {CERTIFICATE_WIDTH}-bit input: the negation is SAT-refuted (PR/RUP), re-checkable without the placer",
                    ops.len()
                ),
                rechecked,
            };
            (Some(cert), Some(row), None)
        }
        None => (None, None, Some("the placement-equivalence obligation escaped the propositional fragment (fail-closed)".into())),
    }
}

fn row_from_placement(
    name: &str,
    kind: &str,
    t: &PlasticineTarget,
    mp: &Metapipeline,
    ag_stages: &[usize],
    par: usize,
    par_capacity: usize,
    placement: &Placement,
    iterations: usize,
    dram_bytes: usize,
    operand_stall_cycles: usize,
) -> WorkloadRow {
    let ii = mp.ii();
    let fill: usize = mp.stages.iter().map(|s| s.latency).sum();
    let compute_cycles = mp.cycles(iterations);
    let dram_cycles = if dram_bytes == 0 {
        0
    } else {
        (dram_bytes as f64 * t.clock_ghz / t.dram.gbps).ceil() as usize
    };
    let (modeled_cycles, bound) = if dram_cycles > compute_cycles + operand_stall_cycles {
        (dram_cycles, "dram")
    } else {
        (compute_cycles + operand_stall_cycles, "compute")
    };
    let stages: Vec<StageRow> = mp
        .stages
        .iter()
        .enumerate()
        .map(|(i, s)| StageRow {
            name: s.name.clone(),
            host: if ag_stages.contains(&i) { "ag" } else { "pcu" }.to_string(),
            resource_ii: s.resource_ii,
            recurrence_ii: s.recurrence_ii,
            memory_ii: s.memory_ii,
            ii: s.ii(),
            latency: s.latency,
            cells: s.cells,
            pcus: placement.alloc.stages[i].pcus,
            critical: s.ii() == ii,
        })
        .collect();
    let critical_stage = mp
        .stages
        .iter()
        .find(|s| s.ii() == ii)
        .map(|s| s.name.clone())
        .unwrap_or_default();
    let buffers: Vec<BufferRow> = placement
        .alloc
        .buffers
        .iter()
        .take(mp.edges.len())
        .map(|b| BufferRow {
            writer: b.edge.0,
            reader: b.edge.1,
            depth: b.depth,
            words: b.words,
            pmus: b.pmus,
        })
        .collect();
    let fu = if placement.pcu_used() == 0 {
        0.0
    } else {
        placement.fu_utilization(t.pcu_stages)
    };
    // The control plane the pipeline needs on this placement: credit depth from the routed distance.
    let hops: Vec<usize> = mp
        .edges
        .iter()
        // Instance 0's own route: `place_unrolled` offsets replicated edges by the stage count, so an
        // unreplicated edge index names the first instance's route directly.
        .map(|e| {
            placement
                .routes
                .iter()
                .find(|r| r.edge == *e)
                .map(|r| r.hops)
                .unwrap_or(0)
        })
        .collect();
    let choreo: Choreography = synthesize(mp, &hops, t, placement.budget);
    let channels: Vec<ChannelRow> = choreo
        .channels
        .iter()
        .map(|ch| ChannelRow {
            writer: ch.edge.0,
            reader: ch.edge.1,
            hops: ch.hops,
            forward_latency: ch.forward_latency,
            backward_latency: ch.backward_latency,
            n_buffer_depth: ch.n_buffer_depth,
            credit_depth: ch.credit_depth,
            depth: ch.depth,
            binding: if ch.credit_depth >= ch.n_buffer_depth {
                "credits"
            } else {
                "n_buffer"
            }
            .to_string(),
        })
        .collect();
    let control = ControlRow {
        nets: choreo.nets.len(),
        hops: choreo.control_hops,
        max_scalar_occupancy: choreo.max_scalar_occupancy,
        scalar_link_budget: choreo.scalar_link_budget,
        routed_statically: choreo.control_routed_statically,
    };
    WorkloadRow {
        name: name.to_string(),
        kind: kind.to_string(),
        target: t.name.to_string(),
        provenance: PROVENANCE_MODELED.to_string(),
        par,
        par_capacity,
        router: placement.router.to_string(),
        stages,
        pipeline_ii: ii,
        critical_stage,
        buffers,
        pcu_used: placement.pcu_used(),
        pcu_total: placement.pcu_total,
        pcu_utilization_pct: pct(placement.pcu_utilization()),
        pmu_used: placement.pmu_used(),
        pmu_total: placement.pmu_total,
        pmu_utilization_pct: pct(placement.pmu_utilization()),
        fu_utilization_pct: pct(fu),
        ag_stages: ag_stages.len() * par,
        total_hops: placement.total_hops(),
        max_hops: placement.max_hops(),
        max_link_occupancy: placement.max_link_occupancy,
        vector_links: placement.budget.vector_links,
        routed_statically: placement.routed_statically,
        iterations,
        fill_cycles: fill,
        compute_cycles,
        operand_stall_cycles,
        dram_bytes,
        dram_cycles,
        modeled_cycles,
        bound: bound.to_string(),
        throughput_iters_per_cycle: r3(mp.throughput()),
        gflops: None,
        pct_of_peak_fp32: None,
        operands: None,
        channels,
        control: Some(control),
        certificate: None,
        certificate_omitted: None,
        vm_anchor: None,
        gemm: None,
    }
}

/// The counted-loop shape the single-stage metapipeline model takes — the FIRST gate of
/// [`lower_counted_loop`], on its own so a lane can ask the question without placing: `fn_name` of `source`
/// as a [`LoopFabric`] (a numeric `Repeat for i from lo to hi` over scalar carried state), or the typed
/// reason it is not one.
pub fn counted_loop_fabric(source: &str, fn_name: &str) -> Result<LoopFabric, String> {
    let lf = quietly(|| loop_fabric_from_source(source, fn_name))
        .map_err(|why| format!("`{fn_name}` is not a counted range loop the metapipeline model takes (Spatial `Foreach` over `lo..=hi`): {why}"))?;
    if lf.seq_input.is_some() || lf.seq_heap_ptr.is_some() {
        return Err("sequence loops (`Repeat for x in xs`): the DRAM → PMU tile-load stage is modeled for GEMM only so far".into());
    }
    if !lf.epilogue.is_empty() || !lf.prologue_heap.is_empty() {
        return Err(
            "a post-loop epilogue / prologue heap island is outside the single-stage loop model"
                .into(),
        );
    }
    Ok(lf)
}

/// Lower the counted loop `fn_name` of `source` onto `t`: one metapipeline stage under the ideal pipelined
/// PCU model, placed on the checkerboard, VM-anchored at `params`, certified when its body is ALU-only,
/// with Spatial source — or a typed reason it cannot be.
pub fn lower_counted_loop(
    source: &str,
    fn_name: &str,
    params: &[i64],
    t: &PlasticineTarget,
) -> Result<PlasticineArtifact, String> {
    let lf = counted_loop_fabric(source, fn_name)?;
    if lf.n_in != params.len() {
        return Err(format!(
            "`{fn_name}` takes {} parameters; {} sample values were given",
            lf.n_in,
            params.len()
        ));
    }
    // The PCU as Tungsten simulates it: a loop-carried register pays the pipeline depth per iteration.
    let model = UnitModel::plasticine_pcu(t.pcu_stages);
    let stage = loop_stage(&lf, &model, fn_name);
    let mp = Metapipeline {
        stages: vec![stage],
        edges: vec![],
    };
    let placement = place_unrolled(&mp, &[], &[], 1, t, LinkBudget::spade_default())
        .map_err(|e| format!("{e:?}"))?;
    let empty: [i64; 0] = [];
    let lo = lf.lo.eval(params, &empty);
    let hi = lf.hi.eval(params, &empty);
    let iterations = if hi < lo { 0 } else { (hi - lo + 1) as usize };
    let ii = mp.ii();
    let fabric = eval_loop_fabric(&lf, params);
    let at_ii = eval_loop_fabric_at_ii(&lf, params, ii, &model);
    let below = if ii > 1 {
        Some(eval_loop_fabric_at_ii(&lf, params, ii - 1, &model).hazards)
    } else {
        None
    };
    let vm = vm_value(source, fn_name, params)?;
    let (certificate, cert_row, omitted) = certify_body(&lf);
    let spatial = emit_spatial_counted_loop(&lf, fn_name)?;
    let spatial_test =
        emit_spatial_counted_loop_test(&lf, &format!("Logos_{fn_name}"), params, vm)?;
    let mut row = row_from_placement(
        fn_name,
        "counted_loop",
        t,
        &mp,
        &[],
        1,
        1,
        &placement,
        iterations,
        0,
        0,
    );
    row.vm_anchor = Some(VmAnchor {
        params: params.to_vec(),
        vm,
        fabric,
        pipelined_at_ii: at_ii.value,
        hazards_at_ii: at_ii.hazards,
        hazards_below_ii: below,
    });
    row.certificate = cert_row;
    row.certificate_omitted = omitted;
    Ok(PlasticineArtifact {
        fn_name: fn_name.to_string(),
        target: t.name,
        loop_fabric: lf,
        metapipeline: mp,
        placement,
        row,
        certificate,
        spatial,
        spatial_test,
    })
}

fn pmu_spec(t: &PlasticineTarget) -> PmuSpec {
    PmuSpec {
        banks: t.pmu_banks,
        words_per_bank: words_per_pmu(t) / t.pmu_banks.max(1),
        n_buffers: 2,
        stages: t.pmu_stages,
    }
}

/// The largest square tile (a multiple of 8, at least 1) whose double-buffered A row-panel + B column-panel
/// (`2 · tile · k` words) fits ONE PMU on `t`.
pub fn gemm_tile_for(k: usize, t: &PlasticineTarget) -> usize {
    let per_buffer = words_per_pmu(t) / 2;
    let max_tile = per_buffer / (2 * k.max(1));
    let by8 = max_tile / 8 * 8;
    by8.max(1).min(max_tile.max(1))
}

/// The per-tile GEMM metapipeline on `t`: load (perimeter AGs: A row-panel + B column-panel, DRAM → PMU) →
/// mac-reduce (a PCU: `lanes`-wide multiply, the `log2(lanes)+1`-stage reduction tree, the accumulator
/// fused into its last stage — ISCA'17 §3.1) → store (AG: the C tile). Returns the metapipeline, the words
/// on each edge (`[A+B panel, C tile]`) and the AG-hosted stage indices.
pub fn gemm_metapipeline(
    k: usize,
    tile: usize,
    t: &PlasticineTarget,
) -> (Metapipeline, Vec<usize>, Vec<usize>) {
    let lanes = t.pcu_lanes.max(1);
    // The reduction tree is ⌈log2(lanes)⌉ adder levels deep plus the accumulator: a lane count that is not a
    // power of two (Spatial's own apps run 5) still reduces pairwise, with one level for the odd lane.
    let tree = (usize::BITS - (lanes - 1).leading_zeros()) as usize + 1;
    let spec = pmu_spec(t);
    let a_read = AffineAccess {
        base: 0,
        stride: 1,
        lanes,
    };
    let b_read = AffineAccess {
        base: 0,
        stride: tile,
        lanes,
    };
    let mem_ii = memory_ii(&[a_read], strided_banking_for(1), &spec).max(memory_ii(
        &[b_read],
        strided_banking_for(tile),
        &spec,
    ));
    let agu = address_datapath_stages(2, true);
    let load = Stage {
        name: "load: A row-panel + B column-panel, DRAM → PMU (perimeter AGs)".into(),
        resource_ii: 1,
        recurrence_ii: 0,
        latency: agu,
        cells: 2,
        memory_ii: 1,
    };
    let mac = Stage {
        name: format!(
            "mac-reduce: {lanes}-lane multiply → {tree}-stage reduction tree → accumulator"
        ),
        resource_ii: 1,
        recurrence_ii: 1,
        latency: 1 + tree,
        cells: 1 + tree,
        memory_ii: mem_ii,
    };
    let store = Stage {
        name: "store: C tile, PMU → DRAM (perimeter AG)".into(),
        resource_ii: 1,
        recurrence_ii: 0,
        latency: agu,
        cells: 1,
        memory_ii: 1,
    };
    let mp = Metapipeline {
        stages: vec![load, mac, store],
        edges: vec![(0, 1), (1, 2)],
    };
    (mp, vec![tile * k + k * tile, tile * tile], vec![0, 2])
}

/// The GEMM row: the tile (or the PMU-fitting default), the unroll (or the widest that fits AND routes
/// statically; `par_capacity` records the capacity bound), the placement,
/// and the roofline — compute cycles at the pipeline II vs the DRAM channel bandwidth over the bytes the
/// tiling moves (B column-panels once, A re-streamed per column panel, C once).
/// A GEMM row with mesh replication only — the intra-tile loops run one pass each.
pub fn gemm_row(
    name: &str,
    m: usize,
    n: usize,
    k: usize,
    tile: Option<usize>,
    par: Option<usize>,
    t: &PlasticineTarget,
) -> Result<WorkloadRow, String> {
    gemm_row_unrolled(name, m, n, k, tile, par, 1, 1, None, t)
}

/// `intra` is the product of the unrolls INSIDE one tile's chunk (the emitted nest's row and column loops) and
/// `kk` the reduction chunks run side by side (`loop_kk`, a dot product's `op`): both divide the work per tile,
/// where `par` divides the number of column panels — different axes, and the model needs each to reproduce an
/// unrolled program. `k_tile` is the reduction chunk the operands stream in (Spatial's `ts` along K; `None` streams
/// a tile's panels whole).
#[allow(clippy::too_many_arguments)]
pub fn gemm_row_unrolled(
    name: &str,
    m: usize,
    n: usize,
    k: usize,
    tile: Option<usize>,
    par: Option<usize>,
    intra: usize,
    kk: usize,
    k_tile: Option<usize>,
    t: &PlasticineTarget,
) -> Result<WorkloadRow, String> {
    let (intra, kk) = (intra.max(1), kk.max(1));
    let k_tile = k_tile.unwrap_or(k).max(1).min(k.max(1));
    // A tile never exceeds the matrix (a 1×1×K dot product tiles at 1 whatever `ts` the source app used).
    let tile = tile
        .unwrap_or_else(|| gemm_tile_for(k, t))
        .max(1)
        .min(m.max(1))
        .min(n.max(1));
    let (mp, words, ags) = gemm_metapipeline(k, tile, t);
    let budget = LinkBudget::spade_default();
    let capacity = max_par(&mp, &words, &ags, t).map_err(|e| format!("{e:?}"))?;
    let lanes = t.pcu_lanes;
    let col_tiles = ceil_div(n, tile);
    let row_tiles = ceil_div(m, tile);
    // The outer loop hands one column panel to each instance: unrolling wider than the panels leaves
    // instances idle, so the automatic unroll is the routable maximum capped at the work available.
    let par = match par {
        Some(p) => p.max(1),
        None => {
            max_routable_par_within(&mp, &words, &ags, t, budget, col_tiles)
                .map_err(|e| format!("{e:?}"))?
                .0
        }
    };
    let placement =
        place_unrolled(&mp, &words, &ags, par, t, budget).map_err(|e| format!("{e:?}"))?;
    let k_chunks = ceil_div(k, lanes);
    // Three axes of parallelism, kept apart. `par` replicates the whole tile pipeline across the mesh and divides
    // the column panels. `intra` widens the row and column loops inside a tile and divides a chunk's compute (a
    // chunk is at least one pass). `kk` runs reduction chunks side by side, so a tile takes ⌈chunks / kk⌉ rounds of
    // one chunk each — a whole chunk's time even when fewer than `kk` chunks are left for the round. The operands
    // stream chunk by chunk, each load at the reduce's lane width (`par ip` on Spatial's loads): nothing computes
    // until the first chunk is in, and a chunk whose load outlasts its compute stalls the MAC for the difference.
    // Measured on Tungsten: a one-lane dot product of four 16,384-element chunks runs (4 + 1) x 16,384 cycles, and
    // three five-lane chunk pipelines over four chunks run (2 + 1) rounds.
    let chunks_in_sequence_per_tile = ceil_div(ceil_div(k, k_tile), kk);
    let chunk_load_cycles = ceil_div(k_tile * tile, lanes);
    let chunk_compute_cycles = ceil_div(tile * tile * ceil_div(k_tile, lanes), intra).max(1);
    let iters_per_tile = chunks_in_sequence_per_tile * chunk_compute_cycles;
    let tiles_in_sequence = ceil_div(col_tiles, par) * row_tiles;
    let iterations = tiles_in_sequence * iters_per_tile;
    let chunks_in_sequence = tiles_in_sequence * chunks_in_sequence_per_tile;
    let operand_stall_cycles =
        chunk_load_cycles + chunks_in_sequence * chunk_load_cycles.saturating_sub(chunk_compute_cycles);
    // How each operand reaches the `par` instances is a COST DECISION, not an assumption: the B column-panel
    // set and the A row-panel set are each either streamed per instance or streamed once and fanned out. The
    // roofline then moves exactly the bytes the decision chose.
    let hops = placement.routes.first().map(|r| r.hops).unwrap_or(1).max(1);
    // B is PARTITIONED, not shared: instance `i` owns its own column panels, so every byte of B is fetched
    // exactly once however the work is spread. There is no sharing question to ask about it.
    let b_bytes = 4 * k * n;
    // A is genuinely SHARED: every column panel sweeps all of A. Replicated, A is re-streamed once per column
    // panel; broadcast to the `par` instances at once, once per ROUND of panels. That is the decision.
    let a_bytes = 4 * m * k;
    let a_stream = OperandStream {
        name: "A".to_string(),
        bytes: a_bytes,
        consumers: par,
        broadcast_hops: hops,
    };
    let a_dec = share_operand(&a_stream, t);
    let a_copies = if a_dec.sharing == Sharing::Broadcast {
        ceil_div(col_tiles, par)
    } else {
        col_tiles
    };
    let operands = vec![
        OperandRow {
            name: "B".to_string(),
            sharing: "partitioned".to_string(),
            consumers: par,
            bytes_per_copy: b_bytes,
            copies: 1,
            bytes_moved: b_bytes,
            dram_cycles_saved: 0,
            fabric_cycles_added: 0,
            why: "each instance owns disjoint column panels: every byte is fetched once, nothing to share".to_string(),
        },
        OperandRow {
            name: "A".to_string(),
            sharing: a_dec.sharing.label().to_string(),
            consumers: par,
            bytes_per_copy: a_bytes,
            copies: a_copies,
            bytes_moved: a_copies * a_bytes,
            dram_cycles_saved: a_dec.dram_cycles_saved,
            fabric_cycles_added: a_dec.fabric_cycles_added,
            why: a_dec.why,
        },
    ];
    let dram_bytes = b_bytes + a_copies * a_bytes + 4 * m * n;
    let mut row = row_from_placement(
        name,
        "gemm",
        t,
        &mp,
        &ags,
        par,
        capacity,
        &placement,
        iterations,
        dram_bytes,
        operand_stall_cycles,
    );
    row.operands = Some(operands);
    let flops = 2.0 * m as f64 * n as f64 * k as f64;
    let gflops = r3(flops * t.clock_ghz / row.modeled_cycles.max(1) as f64);
    row.gflops = Some(gflops);
    row.pct_of_peak_fp32 = Some(r3(gflops / (t.peak_fp32_tflops * 1000.0) * 100.0));
    row.gemm = Some(GemmRow {
        m,
        n,
        k,
        tile_m: tile,
        tile_n: tile,
        col_tiles,
        row_tiles,
        k_chunks,
        iters_per_tile,
        lanes,
        k_tile,
        kk,
        chunk_load_cycles,
        chunk_compute_cycles,
        chunks_in_sequence,
    });
    Ok(row)
}

/// The row for any workload on `t`.
pub fn workload_row(w: &Workload, t: &PlasticineTarget) -> Result<WorkloadRow, String> {
    match &w.kind {
        WorkloadKind::CountedLoop {
            source,
            fn_name,
            params,
        } => {
            let mut row = lower_counted_loop(source, fn_name, params, t)?.row;
            row.name = w.name.clone();
            Ok(row)
        }
        WorkloadKind::Gemm { m, n, k, tile, par } => gemm_row(&w.name, *m, *n, *k, *tile, *par, t),
    }
}

const SELECT_SRC: &str = "## To f (n: Int) -> Int:\n    Let mutable s be 1.\n    Repeat for i from 1 to n:\n        If s is greater than 10:\n            Set s to s - i.\n        Else:\n            Set s to s + i * 2.\n    Return s.\n";

/// One round of the MoE router's masked argmax over 128 synthetic expert scores (the inner loop of the
/// `route128` routing proven ≡VM on the fabric): a counted loop with two carried registers and a
/// data-dependent select.
const ARGMAX_SRC: &str = "## To argmax128 (x: Int) -> Int:\n    Let mutable bv be 0.\n    Let mutable bi be 1.\n    Repeat for j from 1 to 128:\n        Let v be (((x * j * 7) + (j * 13) + 5) % 101).\n        If v is greater than bv:\n            Set bv to v.\n            Set bi to j.\n    Return bi.\n";

/// The canonical workloads: the metapipeline corpus, the MoE argmax round, a dot product, and the two GEMM
/// shapes the Tenstorrent ledger measures.
pub fn canonical_workloads() -> Vec<Workload> {
    fn loop_prog(body: &str) -> String {
        format!("## To f (n: Int) -> Int:\n    Let mutable s be 1.\n    Repeat for i from 1 to n:\n        Set s to {body}.\n    Return s.\n")
    }
    // Trip counts keep every carried value inside i64: the VM promotes to BigInt on overflow while the
    // fabric wraps, and the anchor must compare like with like (3^30, 6^20, 30^12 all fit).
    let counted = |name: &str, source: String, n: i64| Workload {
        name: name.to_string(),
        kind: WorkloadKind::CountedLoop {
            source,
            fn_name: "f".to_string(),
            params: vec![n],
        },
    };
    vec![
        counted("loop-sum", loop_prog("s + i"), 1000),
        counted("loop-affine", loop_prog("s * 3 + i"), 30),
        counted("loop-affine-deep", loop_prog("(s * 3 + i) * 2"), 20),
        counted("loop-mul-chain", loop_prog("((s * 2) * 3) * 5 + i"), 12),
        counted("loop-no-carry", loop_prog("i * 7 + 1"), 1000),
        counted("loop-select", SELECT_SRC.to_string(), 1000),
        Workload {
            name: "moe-argmax-round-128".to_string(),
            kind: WorkloadKind::CountedLoop {
                source: ARGMAX_SRC.to_string(),
                fn_name: "argmax128".to_string(),
                params: vec![3],
            },
        },
        Workload {
            name: "dot-product-1024".to_string(),
            kind: WorkloadKind::Gemm {
                m: 1,
                n: 1,
                k: 1024,
                tile: Some(1),
                par: Some(1),
            },
        },
        Workload {
            name: "gemm-640x640x640".to_string(),
            kind: WorkloadKind::Gemm {
                m: 640,
                n: 640,
                k: 640,
                tile: None,
                par: None,
            },
        },
        Workload {
            name: "gemm-2048x1024x512".to_string(),
            kind: WorkloadKind::Gemm {
                m: 2048,
                n: 1024,
                k: 512,
                tile: None,
                par: None,
            },
        },
    ]
}

/// The two published configurations.
pub fn canonical_targets() -> Vec<PlasticineTarget> {
    vec![PlasticineTarget::isca17(), PlasticineTarget::sara_20x20()]
}

/// What the model does NOT cover yet, named with the reason — the honest boundary printed in the report.
pub fn not_modeled() -> Vec<(String, String)> {
    let s = |a: &str, b: &str| (a.to_string(), b.to_string());
    vec![
        s(
            "data-dependent `while` loops (Collatz, the data_dependent_control corpus)",
            "a while loop's trip count is a runtime predicate; the metapipeline model takes counted loops (Spatial `Foreach`). These programs are proven ≡VM on the fabric's decision / loop mechanisms; a stream-controller (Spatial `Stream`/FSM) II model is the P5 follow-on",
        ),
        s(
            "route128 top-8-of-128 MoE routing (whole function)",
            "three nested while loops over a heap-resident Seq — proven ≡VM on the fabric call-stack mechanism; the nested counter-chain controller model (Spatial `Foreach` nests with PMU-resident state) is P5. The argmax round it repeats eight times is modeled above",
        ),
        s(
            "tiny transformer block / Qwen3-30B MoE routing at real scale",
            "nested loops with heap-resident tensors — same nested-controller gap as route128",
        ),
        s(
            "scalar sequence loops (`Repeat for x in xs`)",
            "the DRAM → PMU tile-load stage is modeled for GEMM only; a parameter sequence's load stage comes with the nested-controller model",
        ),
    ]
}

fn target_summary(t: &PlasticineTarget) -> Value {
    json!({
        "name": t.name,
        "mesh": format!("{}x{} checkerboard", t.pcu_rows, t.pcu_cols),
        "pcus": t.pcus(),
        "pmus": t.pmus(),
        "pcu_lanes": t.pcu_lanes,
        "pcu_stages": t.pcu_stages,
        "pcu_regs_per_stage": t.pcu_regs_per_stage,
        "pmu_banks": t.pmu_banks,
        "pmu_kib": t.pmu_kib,
        "pmu_stages": t.pmu_stages,
        "scratchpad_kib": t.scratchpad_kib(),
        "dram": { "channels": t.dram.channels, "kind": t.dram.kind, "gbps": t.dram.gbps },
        "clock_ghz": t.clock_ghz,
        "peak_fp32_tflops": r3(t.peak_fp32_tflops),
        "area_mm2": t.area_mm2,
        "power_w": t.power_w,
        "cite": t.cite,
    })
}

/// The whole report as JSON — deterministic in the workloads' and targets' order. Every workload must lower
/// (a canonical workload the model cannot take is a bug, not a silent omission).
pub fn plasticine_report(workloads: &[Workload], targets: &[PlasticineTarget]) -> Value {
    let mut rows = Vec::with_capacity(workloads.len() * targets.len());
    for t in targets {
        for w in workloads {
            let row = workload_row(w, t)
                .unwrap_or_else(|e| panic!("canonical workload `{}` on {}: {e}", w.name, t.name));
            rows.push(serde_json::to_value(row).expect("a row serializes"));
        }
    }
    let not_modeled: Vec<Value> = not_modeled()
        .into_iter()
        .map(|(name, reason)| json!({ "name": name, "reason": reason }))
        .collect();
    json!({
        "_generated_by": "the private place-and-route engine's report emitter, emit_plasticine_report (codegen_sva::plasticine_report) — never hand-edited; plasticine_report_lock regenerates it byte-equal",
        "provenance": PROVENANCE_MODELED,
        "honesty": "Every number is the paper-parameterized model's (ISCA'17 / SARA descriptors, Spatial's II definition, the PMU banking model, the checkerboard placer). Nothing here ran on a Plasticine simulator or on silicon; Tungsten rows land beside these under provenance `simulated_tungsten`.",
        "model": {
            "ii": "Spatial (Koeplinger et al., PLDI 2018, §4): stage II = max(resource II, recurrence II, memory II); pipeline II = the slowest stage; N-buffer depth = reader − writer + 1",
            "pcu": "ISCA'17 §3.1: a stage of depth D chains ceil(D / 6) PCUs, each 16 lanes wide; the reduction tree is log2(lanes)+1 stages",
            "pmu": "ISCA'17 §3.2: 16 banks × 16 KB; strided banking at the access stride is conflict-free for 16 lanes; N-buffers partition the address space",
            "control": "every inter-stage channel carries a data-valid net forward and a space-available net backward (codegen_sva::choreography); its credit depth is the bandwidth-delay product ceil((forward + backward latency) / II) and the shipped depth is the deeper of that and Spatial's writer-to-reader N-buffer rule; control nets are budgeted on the SCALAR network against the PCU's control ports, separately from the vector network the data crosses. `plasticine_choreography_lock` proves the depth tight: at it the pipeline runs at exactly 1/II, and one credit less on any channel loses throughput",
            "placement": "row-balanced checkerboard placement; static routing by the private place-and-route engine over the mesh's routing-resource graph (one wire per channel of every DIRECTED link at Spade's vlink 2, unit vector ports, perimeter DRAM ports): the negotiated-congestion rung, the exact SAT rung on a stall, and the independent legality checker as the verdict; address-generator stages attach at the left/right perimeter switches (a port counts one hop)",
            "dram": "cycles = bytes × clock_GHz / (GB/s); a GEMM tiling moves B column-panels once, A once per column panel, C once",
            "cycles": "fill (Σ stage latency) + (iterations − 1) × II per unrolled instance; modeled cycles = max(compute, DRAM)",
        },
        "targets": targets.iter().map(target_summary).collect::<Vec<_>>(),
        "rows": rows,
        "not_modeled": not_modeled,
        "published_reference": {
            "note": "Spatial PLDI'18 Table 4 (Plasticine column) — the paper's numbers, not ours: the credibility target the Tungsten lane reproduces before our rows are compared to it",
            "spatial_pldi18_table4_plasticine": {
                "GEMM": { "pcu_pct": 96.8, "pmu_pct": 64.1, "ag_pct": 11.7, "time_ms": 15.98 },
                "BlackScholes": { "pcu_pct": 73.4, "pmu_pct": 10.9, "ag_pct": 20.6 },
                "GDA": { "pcu_pct": 95.3, "pmu_pct": 73.4, "ag_pct": 38.2 },
                "KMeans": { "pcu_pct": 89.1, "pmu_pct": 57.8, "ag_pct": 17.6 },
                "TQ6": { "pcu_pct": 29.7, "pmu_pct": 37.5, "ag_pct": 70.6 },
            },
        },
    })
}

/// The descriptor a toolchain row's `rows × cols` mesh calibrates against: the 20x20 mesh is PIR's default in
/// front of Tungsten's DDR4 memory (`PlasticineTarget::tungsten_20x20_ddr4`, the system the chain simulates);
/// the 16x8 mesh is the ISCA'17 preset.
pub fn target_for_mesh(rows: usize, cols: usize) -> Option<PlasticineTarget> {
    if (rows, cols) == (20, 20) {
        return Some(PlasticineTarget::tungsten_20x20_ddr4());
    }
    canonical_targets()
        .into_iter()
        .find(|t| t.pcu_rows == rows && t.pcu_cols == cols)
}

/// One toolchain row joined with our model: the same app shape, tile (`ts`) and a single instance (its loop
/// pars are 1), on the same mesh — modeled cycles vs Tungsten's, the per-cent difference, and the unit
/// counts side by side (PIR's PCU count includes the control/address units its partitioner splits off; our
/// row counts compute PCUs and perimeter address-generator stages separately — printed, not conflated).
/// A dot product is the `1 × 1 × N` GEMM: its `ts` the K-chunk, its `ip` the reduce's lanes, and its `op`
/// (how many tiles are reduced at once) the GEMM's `loop_kk`. Rows of `kind: "dot"` are modeled as that GEMM.
pub fn calibration_row(t: &PlasticineTarget, row: &Value) -> Result<Value, String> {
    if row["kind"].as_str() == Some("counted_loop") {
        return calibration_row_loop(t, row);
    }
    if let Some(why) = unmodeled(row) {
        return Err(format!("{}: {why}", row["key"]));
    }
    if let Some(why) = routed_failure(row) {
        return Err(format!("{}: {why}", row["key"]));
    }
    let dot = row["kind"].as_str() == Some("dot");
    let par = |p: &str| {
        let key = if dot && p == "loop_kk" { "op" } else { p };
        row["params"][key].as_u64().unwrap_or(1)
    };
    let shape = &row["shape"];
    let (m, n, k) = (
        shape["m"].as_u64().ok_or("shape.m")? as usize,
        shape["n"].as_u64().ok_or("shape.n")? as usize,
        shape["k"].as_u64().ok_or("shape.k")? as usize,
    );
    let ts = row["params"]["ts"].as_u64().ok_or("params.ts")? as usize;
    let ip = row["params"]["ip"].as_u64().ok_or("params.ip")? as usize;
    // The axes, kept apart: the outer tile loops replicate the pipeline across the mesh; the row and column loops
    // widen the work inside one chunk; `loop_kk` runs reduction chunks side by side.
    let mesh_par: u64 = ["loop_ii", "loop_jj"].iter().map(|p| par(p)).product();
    let intra: u64 = ["loop_i", "loop_j"].iter().map(|p| par(p)).product();
    let kk = par("loop_kk");
    // Model the app on the vector width the app actually runs. `ip` is the innermost `par` of the Spatial
    // source — the lanes the reduce issues across — and it is a property of the PROGRAM, not of the part: the
    // same mesh runs `ip=1` and `ip=16` apps. The model read the target's lane count instead, so an app
    // configured narrower than the descriptor was credited with the descriptor's parallelism. That is how a
    // 32^3 GEMM at `ip=1` came out at 1,035 modelled cycles against 9,128 measured — a 16-lane machine doing
    // one-lane work. Widths above the descriptor are not granted: the part cannot issue more than it has; widths
    // below it need not be powers of two (Spatial's own DotProduct_6 runs five).
    let mut app_t = t.clone();
    if ip >= 1 {
        app_t.pcu_lanes = ip.min(t.pcu_lanes);
    }
    let modeled = gemm_row_unrolled(
        row["app"].as_str().unwrap_or("app"),
        m,
        n,
        k,
        Some(ts),
        Some(mesh_par.max(1) as usize),
        intra.max(1) as usize,
        kk.max(1) as usize,
        Some(ts),
        &app_t,
    )?;
    let simulated = row["hybrid_network"]["cycles"]
        .as_u64()
        .ok_or("hybrid_network.cycles")? as usize;
    let ideal = row["ideal_network"]["cycles"]
        .as_u64()
        .ok_or("ideal_network.cycles")? as usize;
    let ratio = modeled.modeled_cycles as f64 / simulated as f64;
    Ok(json!({
        "key": row["key"],
        "app": row["app"],
        "kind": if dot { "dot" } else { "gemm" },
        "target": t.name,
        "shape": { "m": m, "n": n, "k": k, "tile": ts, "lanes": ip },
        "modeled": {
            "cycles": modeled.modeled_cycles,
            "bound": modeled.bound,
            "pipeline_ii": modeled.pipeline_ii,
            "lanes": modeled.gemm.as_ref().map(|g| g.lanes),
            "operand_stall_cycles": modeled.operand_stall_cycles,
            "compute_pcus": modeled.pcu_used,
            "pmus": modeled.pmu_used,
            "ag_stages": modeled.ag_stages,
            "provenance": PROVENANCE_MODELED,
        },
        "simulated": {
            "cycles_hybrid_network": simulated,
            "cycles_ideal_network": ideal,
            "pir_pcus": row["pir"]["PCU"]["used"],
            "pir_pmus": row["pir"]["PMU"]["used"],
            "pir_dags": row["pir"]["DAG"]["used"],
            "pir_mcs": row["pir"]["MC"]["used"],
            "plastiroute_best_score": row["plastiroute"]["best_score"],
            "pass": row["hybrid_network"]["pass"],
            "provenance": PROVENANCE_SIMULATED,
        },
        "modeled_over_simulated": r3(ratio),
        "difference_pct": r3((ratio - 1.0).abs() * 100.0),
        "lanes_agree": modeled.gemm.as_ref().map(|g| g.lanes == ip).unwrap_or(false),
    }))
}

/// A toolchain row for one of OUR emitted counted loops (the ledger carries the LOGOS source, the function and
/// the runtime args), joined with the model's row for the same loop on the same mesh: modeled cycles (fill +
/// (iterations − 1) × II under the ideal pipelined PCU) vs Tungsten's.
fn calibration_row_loop(t: &PlasticineTarget, row: &Value) -> Result<Value, String> {
    let source = row["logos_source"].as_str().ok_or("logos_source")?;
    let fn_name = row["fn_name"].as_str().ok_or("fn_name")?;
    let params: Vec<i64> = row["params"]
        .as_array()
        .ok_or("params")?
        .iter()
        .map(|v| v.as_i64().ok_or("params: i64"))
        .collect::<Result<_, _>>()?;
    let art = lower_counted_loop(source, fn_name, &params, t)?;
    let modeled = &art.row;
    let simulated = row["hybrid_network"]["cycles"]
        .as_u64()
        .ok_or("hybrid_network.cycles")? as usize;
    let ideal = row["ideal_network"]["cycles"]
        .as_u64()
        .ok_or("ideal_network.cycles")? as usize;
    let ratio = modeled.modeled_cycles as f64 / simulated as f64;
    let anchor = modeled.vm_anchor.as_ref().ok_or("vm anchor")?;
    Ok(json!({
        "key": row["key"],
        "app": row["app"],
        "kind": "counted_loop",
        "target": t.name,
        "loop": { "fn": fn_name, "params": params, "iterations": modeled.iterations, "vm_answer": anchor.vm },
        "modeled": {
            "cycles": modeled.modeled_cycles,
            "pipeline_ii": modeled.pipeline_ii,
            "fill_cycles": modeled.fill_cycles,
            "compute_pcus": modeled.pcu_used,
            "provenance": PROVENANCE_MODELED,
        },
        "simulated": {
            "cycles_hybrid_network": simulated,
            "cycles_ideal_network": ideal,
            "pir_pcus": row["pir"]["PCU"]["used"],
            "pir_pmus": row["pir"]["PMU"]["used"],
            "pass": row["hybrid_network"]["pass"],
            "provenance": PROVENANCE_SIMULATED,
        },
        "modeled_over_simulated": r3(ratio),
        "difference_pct": r3((ratio - 1.0).abs() * 100.0),
        "simulated_cycles_per_iteration": r3(simulated as f64 / modeled.iterations.max(1) as f64),
    }))
}

/// The kinds of toolchain row the model has no counterpart for, each with the reason — listed in the
/// calibration as uncalibrated, never dropped and never joined with a model that does not describe them.
pub const UNMODELED_KINDS: &[(&str, &str)] = &[
    (
        "outer",
        "the model has no outer-product row: its pipeline computes a ts1 x ts2 product tile and stores it whole, a \
         store-bound pipeline the GEMM metapipeline (a MemReduce over K) does not describe",
    ),
    (
        "tpchq6",
        "the model has no guarded-sum row: TPC-H Q6 streams four columns and selects per element, which the two-stream \
         1 x 1 x N GEMM a dot product is modeled as does not describe",
    ),
    (
        "gda",
        "the model has no scatter-accumulation row: GDA centres each row and adds its outer product to a C x C tile \
         under two nested MemReduces, a pipeline the GEMM metapipeline does not describe",
    ),
    (
        "blackscholes",
        "the model has no elementwise-map row: Black-Scholes streams six columns through FIFOs into a chain of \
         several hundred scalar operations per element, a dataflow graph the GEMM metapipeline does not describe",
    ),
    (
        "kmeans",
        "the model has no clustering row: k-means runs epochs of a distance reduce per point and centroid, a min and an \
         index reduce, and a one-hot MemReduce into the centroids, a pipeline the GEMM metapipeline does not describe",
    ),
    (
        "spatial_app",
        "one of Spatial's own apps whose twin the compiler does not print yet: the model describes the shapes the \
         compiler lowers (GEMMs, dot products, counted loops), not an arbitrary Spatial program",
    ),
];

/// Why `row` has no modeled counterpart, if its kind is one the model does not describe.
/// Why a toolchain row's routed (hybrid) simulation is no calibration point: what it came to when it did not finish
/// clean (the recorder's `why`: deadlocked, threw, failed its gold check). The model is calibrated against routed
/// cycles, and a routed run that did not finish has none to compare.
pub fn routed_failure(row: &Value) -> Option<String> {
    row["hybrid_network"]["why"].as_str().map(|why| format!("its routed-network simulation {why}"))
}

pub fn unmodeled(row: &Value) -> Option<&'static str> {
    let kind = row["kind"].as_str()?;
    UNMODELED_KINDS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, why)| *why)
}

/// The calibration document: every toolchain row on a mesh we model, joined with the model, and every row of
/// a kind the model does not describe, or whose routed run did not finish clean, listed as uncalibrated with the
/// reason.
pub fn calibration(toolchain_ledger: &Value) -> Result<Value, String> {
    let mut rows = Vec::new();
    let mut uncalibrated = Vec::new();
    for row in toolchain_ledger["rows"]
        .as_array()
        .ok_or("toolchain ledger has no rows array")?
    {
        if let Some(why) = unmodeled(row) {
            uncalibrated.push(json!({ "key": row["key"], "app": row["app"], "kind": row["kind"], "why": why }));
            continue;
        }
        if let Some(why) = routed_failure(row) {
            uncalibrated.push(json!({ "key": row["key"], "app": row["app"], "kind": row["kind"], "why": why }));
            continue;
        }
        let mesh = &row["mesh"];
        let (r, c) = (
            mesh["rows"].as_u64().ok_or("mesh.rows")? as usize,
            mesh["cols"].as_u64().ok_or("mesh.cols")? as usize,
        );
        let t = target_for_mesh(r, c).ok_or_else(|| {
            format!(
                "toolchain row {} ran on a {r}x{c} mesh we have no descriptor for",
                row["key"]
            )
        })?;
        rows.push(calibration_row(&t, row)?);
    }
    Ok(json!({
        "_generated_by": "the private place-and-route engine's report emitter, emit_plasticine_report (codegen_sva::plasticine_report::calibration) — joins benchmarks/silicon/plasticine_toolchain.json with the model; never hand-edited",
        "what": "For each app the public Plasticine toolchain simulated (Spatial → PIR → plastiroute → Tungsten), our MODELED row for the same shape, tile and unroll on the same mesh — the calibration an RDU engineer asks for before trusting a model. `simulated` numbers are the toolchain's (a cycle-accurate simulation of the paper's architecture, not silicon); `modeled` numbers are ours.",
        "tolerance_pct": CALIBRATION_TOLERANCE_PCT,
        "rows": rows,
        "uncalibrated": uncalibrated,
    }))
}

/// The per-cent difference the calibration lock allows between modeled and simulated cycles.
pub const CALIBRATION_TOLERANCE_PCT: f64 = 15.0;

/// Pretty JSON with a trailing newline — the exact bytes of [`REPORT_PATH`].
pub fn render_report(v: &Value) -> String {
    format!(
        "{}\n",
        serde_json::to_string_pretty(v).expect("the report serializes")
    )
}

/// A canonical LOGOS matmul `C[m×n] = A[m×k] · B[k×n]` as the triple loop the tensor recognizer reads (the
/// same program the Tenstorrent lane lowers).
pub fn logos_gemm_program(m: usize, n: usize, k: usize) -> String {
    format!(
        "## Main\nLet rows be {m}.\nLet cols be {n}.\nLet mid be {k}.\nLet mutable a be a new Seq of Int.\nLet mutable b be a new Seq of Int.\nLet mutable c be a new Seq of Int.\nLet mutable p be 0.\nWhile p is less than rows * mid:\n    Push 1 to a.\n    Set p to p + 1.\nSet p to 0.\nWhile p is less than mid * cols:\n    Push 1 to b.\n    Set p to p + 1.\nSet p to 0.\nWhile p is less than rows * cols:\n    Push 0 to c.\n    Set p to p + 1.\nLet mutable i be 0.\nWhile i is less than rows:\n    Let mutable k be 0.\n    While k is less than mid:\n        Let mutable j be 0.\n        While j is less than cols:\n            Let idx be i * cols + j + 1.\n            Set item idx of c to item idx of c + item (i * mid + k + 1) of a * item (k * cols + j + 1) of b.\n            Set j to j + 1.\n        Set k to k + 1.\n    Set i to i + 1.\nShow item 1 of c.\n"
    )
}

/// A LOGOS matmul, recognized (`tensor_recognize::recognize_matmul`) and emitted as Spatial's GEMM regression
/// app with tiles `ts`/`its` and an `ip`-lane reduce: `(m, n, k, class name, Spatial source)`, or a typed
/// reason (not a matmul; tiles that do not divide the shape).
pub fn lower_gemm_source(
    source: &str,
    ts: usize,
    its: usize,
    ip: usize,
) -> Result<(usize, usize, usize, String, String), String> {
    let (m, n, k) = crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.map_err(|e| format!("parse: {e:?}"))?;
        let spec = crate::tensor_recognize::recognize_matmul(stmts, interner)
            .map_err(|e| format!("not a matmul the tensor recognizer reads: {e:?}"))?;
        Ok::<_, String>((spec.m, spec.n, spec.k))
    })?;
    let class = format!("Logos_gemm_{m}x{n}x{k}");
    let spatial = emit_spatial_gemm_test(m, n, k, ts, its, ip, &class)?;
    Ok((m, n, k, class, spatial))
}

/// A canonical LOGOS dot product `Σ_{t<n} a[t]·b[t]` as the counted loop the tensor recognizer reads.
pub fn logos_dot_program(n: usize) -> String {
    format!(
        "## Main\nLet n be {n}.\nLet mutable a be a new Seq of Int.\nLet mutable b be a new Seq of Int.\nLet mutable p be 0.\nWhile p is less than n:\n    Push p % 7 to a.\n    Push p % 5 to b.\n    Set p to p + 1.\nLet mutable s be 0.\nLet mutable i be 0.\nWhile i is less than n:\n    Set s to s + item (i + 1) of a * item (i + 1) of b.\n    Set i to i + 1.\nShow s.\n"
    )
}

/// A canonical LOGOS outer product `c[i·cols + j] = a[i]·b[j]` (`rows × cols`) as the nest the tensor
/// recognizer reads; it shows `Σ (t + 1)·c[t]`, a checksum a transposed store does not reproduce.
pub fn logos_outer_program(rows: usize, cols: usize) -> String {
    format!(
        "## Main\nLet rows be {rows}.\nLet cols be {cols}.\nLet mutable a be a new Seq of Int.\nLet mutable b be a new Seq of Int.\nLet mutable c be a new Seq of Int.\nLet mutable p be 0.\nWhile p is less than rows:\n    Push p % 7 to a.\n    Set p to p + 1.\nSet p to 0.\nWhile p is less than cols:\n    Push p % 5 to b.\n    Set p to p + 1.\nSet p to 0.\nWhile p is less than rows * cols:\n    Push 0 to c.\n    Set p to p + 1.\nLet mutable i be 0.\nWhile i is less than rows:\n    Let mutable j be 0.\n    While j is less than cols:\n        Let idx be i * cols + j + 1.\n        Set item idx of c to item (i + 1) of a * item (j + 1) of b.\n        Set j to j + 1.\n    Set i to i + 1.\nLet mutable sum be 0.\nSet p to 0.\nWhile p is less than rows * cols:\n    Set sum to sum + item (p + 1) of c * (p + 1).\n    Set p to p + 1.\nShow sum.\n"
    )
}

/// A recognized zip sum printed as Spatial's reduce-over-tiles app: each column named as the program names it
/// and filled as the program fills it. The class name carries the family, the length and every knob, in
/// Spatial's parameter order, so each variant's twin has its own generated directory.
fn emit_zip_twin(
    stmts: &[logicaffeine_language::ast::stmt::Stmt],
    interner: &crate::intern::Interner,
    zip: &ZipSum,
    family: &str,
    p: ZipPar,
) -> Result<(usize, String, String), String> {
    let columns: Vec<ZipColumn> = zip
        .operands
        .iter()
        .map(|o| ZipColumn {
            name: interner.resolve(*o).to_string(),
            fill: crate::tensor_recognize::recognize_fill(stmts, *o),
        })
        .collect();
    let class = format!("Logos_{family}_{}_ts{}_ip{}_op{}", zip.n, p.ts, p.ip, p.op);
    let spatial = emit_spatial_zip_sum_test(zip.n, &columns, &zip.term, p, &class)?;
    Ok((zip.n, class, spatial))
}

/// A LOGOS dot product, recognized (`tensor_recognize::recognize_dot`) and emitted as Spatial's DotProduct
/// regression app at the knobs `p`: `(n, class name, Spatial source)`, or a typed reason (not a dot product; a
/// tile that does not divide the length; a sum the app's 32-bit `Int` would wrap).
pub fn lower_dot_source(source: &str, p: ZipPar) -> Result<(usize, String, String), String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.map_err(|e| format!("parse: {e:?}"))?;
        let dot = crate::tensor_recognize::recognize_dot(stmts, interner)
            .map_err(|e| format!("not a dot product the tensor recognizer reads: {e:?}"))?;
        let operands = if dot.a == dot.b { vec![dot.a] } else { vec![dot.a, dot.b] };
        let read = |s| ZipTerm::Read(operands.iter().position(|o| *o == s).expect("an operand"));
        let zip = ZipSum {
            n: dot.n,
            term: ZipTerm::Op(ZipOp::Mul, Box::new(read(dot.a)), Box::new(read(dot.b))),
            operands,
            acc: dot.acc,
        };
        emit_zip_twin(stmts, interner, &zip, "dot", p)
    })
}

/// The query of Spatial's `TPCHQ6` regression app (`pir/regression/TPCHQ6.scala`), transcribed: over the
/// columns `dates`, `discts`, `quants`, `prices` (read in that order), the revenue term
/// `date > MIN_DATE && date < MAX_DATE && disct >= MIN_DISC && disct <= MAX_DISC && quant < 24 ? price·disct : 0`
/// with its `MIN_DATE = 0`, `MAX_DATE = 9999`, `MIN_DISC = 0`, `MAX_DISC = 9999`.
pub fn tpchq6_query() -> ZipTerm {
    let (date, disct, quant, price) = (|| ZipTerm::Read(0), || ZipTerm::Read(1), || ZipTerm::Read(2), || ZipTerm::Read(3));
    let op = |o, l, r| ZipTerm::Op(o, Box::new(l), Box::new(r));
    let valid = [
        op(ZipOp::Gt, date(), ZipTerm::Const(0)),
        op(ZipOp::Lt, date(), ZipTerm::Const(9999)),
        op(ZipOp::Ge, disct(), ZipTerm::Const(0)),
        op(ZipOp::Le, disct(), ZipTerm::Const(9999)),
        op(ZipOp::Lt, quant(), ZipTerm::Const(24)),
    ]
    .into_iter()
    .reduce(|a, b| op(ZipOp::And, a, b))
    .expect("five conditions");
    ZipTerm::Select(Box::new(valid), Box::new(op(ZipOp::Mul, price(), disct())), Box::new(ZipTerm::Const(0)))
}

/// TPC-H Q6 in LOGOS over `n` records — the four columns filled as Spatial's TPCHQ6 app fills them
/// (`i % 256`) and the revenue summed under the query's predicate.
pub fn logos_tpchq6_program(n: usize) -> String {
    format!(
        "## Main\nLet n be {n}.\nLet mutable dates be a new Seq of Int.\nLet mutable quants be a new Seq of Int.\nLet mutable discts be a new Seq of Int.\nLet mutable prices be a new Seq of Int.\nLet mutable p be 0.\nWhile p is less than n:\n    Push p % 256 to dates.\n    Push p % 256 to quants.\n    Push p % 256 to discts.\n    Push p % 256 to prices.\n    Set p to p + 1.\nLet mutable revenue be 0.\nLet mutable i be 0.\nWhile i is less than n:\n    Let date be item (i + 1) of dates.\n    Let disct be item (i + 1) of discts.\n    Let quant be item (i + 1) of quants.\n    If date is greater than 0 and date is less than 9999 and disct is at least 0 and disct is at most 9999 and quant is less than 24:\n        Set revenue to revenue + item (i + 1) of prices * disct.\n    Set i to i + 1.\nShow revenue.\n"
    )
}

/// A LOGOS TPC-H Q6 — the zip sum whose term is [`tpchq6_query`] — emitted as Spatial's TPCHQ6 regression app
/// at the knobs `p`: `(n, class name, Spatial source)`, or a typed reason.
pub fn lower_tpchq6_source(source: &str, p: ZipPar) -> Result<(usize, String, String), String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.map_err(|e| format!("parse: {e:?}"))?;
        let query = tpchq6_query();
        let zip = crate::tensor_recognize::recognize_zip_sum_where(stmts, interner, &|z| z.term == query)
            .map_err(|e| format!("not TPC-H Q6 as the tensor recognizer reads it: {e:?}"))?;
        emit_zip_twin(stmts, interner, &zip, "tpchq6", p)
    })
}

/// The covariance step of Spatial's `GDA` regression app (`pir/regression/GDA.scala`), transcribed: over the arrays
/// `y` (at the row), `x` (row-major), `mu1` and `mu0` (at the column), read in that order, the row term
/// `x(rr, cc) - mux(y(rr) == 1, mu1(cc), mu0(cc))`, each row's outer product with itself summed into `sigma`.
pub fn gda_query() -> (Vec<Axis>, ZipTerm) {
    let op = |o, l, r| ZipTerm::Op(o, Box::new(l), Box::new(r));
    let label = op(ZipOp::Eq, ZipTerm::Read(0), ZipTerm::Const(1));
    let mean = ZipTerm::Select(Box::new(label), Box::new(ZipTerm::Read(2)), Box::new(ZipTerm::Read(3)));
    (vec![Axis::Row, Axis::RowCol, Axis::Col, Axis::Col], op(ZipOp::Sub, ZipTerm::Read(1), mean))
}

/// Gaussian discriminant analysis's covariance in LOGOS over `rows` samples of `cols` features — the data filled as
/// Spatial's GDA app fills it (`x` row-major `i % 256`, `y` `i % 256`, both means `i % 2`), each row centred on the
/// mean its label picks and added to `sigma` as a rank-1 update. It shows `Σ (t + 1)·sigma[t]`, a checksum a
/// transposed or partial covariance does not reproduce.
pub fn logos_gda_program(rows: usize, cols: usize) -> String {
    format!(
        "## Main\nLet rows be {rows}.\nLet cols be {cols}.\nLet mutable x be a new Seq of Int.\nLet mutable y be a new Seq of Int.\nLet mutable mu0 be a new Seq of Int.\nLet mutable mu1 be a new Seq of Int.\nLet mutable sigma be a new Seq of Int.\nLet mutable sub be a new Seq of Int.\nLet mutable p be 0.\nWhile p is less than rows * cols:\n    Push p % 256 to x.\n    Set p to p + 1.\nSet p to 0.\nWhile p is less than rows:\n    Push p % 256 to y.\n    Set p to p + 1.\nSet p to 0.\nWhile p is less than cols:\n    Push p % 2 to mu0.\n    Push p % 2 to mu1.\n    Push 0 to sub.\n    Set p to p + 1.\nSet p to 0.\nWhile p is less than cols * cols:\n    Push 0 to sigma.\n    Set p to p + 1.\nLet mutable r be 0.\nWhile r is less than rows:\n    Let label be item (r + 1) of y.\n    Let mutable c be 0.\n    While c is less than cols:\n        If label is 1:\n            Set item (c + 1) of sub to item (r * cols + c + 1) of x - item (c + 1) of mu1.\n        Otherwise:\n            Set item (c + 1) of sub to item (r * cols + c + 1) of x - item (c + 1) of mu0.\n        Set c to c + 1.\n    Let mutable i be 0.\n    While i is less than cols:\n        Let mutable j be 0.\n        While j is less than cols:\n            Let idx be i * cols + j + 1.\n            Set item idx of sigma to item idx of sigma + item (i + 1) of sub * item (j + 1) of sub.\n            Set j to j + 1.\n        Set i to i + 1.\n    Set r to r + 1.\nLet mutable sum be 0.\nSet p to 0.\nWhile p is less than cols * cols:\n    Set sum to sum + item (p + 1) of sigma * (p + 1).\n    Set p to p + 1.\nShow sum.\n"
    )
}

/// A LOGOS GDA covariance — the scatter accumulation whose row term and reading axes are [`gda_query`] — emitted as
/// Spatial's GDA regression app at the knobs `p`: `(rows, cols, class name, Spatial source)`, or a typed reason. The
/// class name carries the shape and every knob in Spatial's parameter order.
pub fn lower_gda_source(source: &str, p: GdaPar) -> Result<(usize, usize, String, String), String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.map_err(|e| format!("parse: {e:?}"))?;
        let spec = crate::tensor_recognize::recognize_scatter(stmts, interner)
            .map_err(|e| format!("not a scatter accumulation the tensor recognizer reads: {e:?}"))?;
        let (axes, query) = gda_query();
        if spec.term != query || spec.operands.iter().map(|(_, a)| *a).collect::<Vec<_>>() != axes {
            return Err("a scatter accumulation, but not GDA's covariance step".to_string());
        }
        let columns: Vec<GdaColumn> = spec
            .operands
            .iter()
            .map(|(o, axis)| GdaColumn {
                name: interner.resolve(*o).to_string(),
                axis: *axis,
                fill: crate::tensor_recognize::recognize_fill(stmts, *o),
            })
            .collect();
        let class = format!(
            "Logos_gda_{}x{}_ts{}_op{}_mp{}x{}_ip{}",
            spec.rows, spec.cols, p.ts, p.op, p.mp1, p.mp2, p.ip
        );
        let spatial = emit_spatial_gda_test(spec.rows, spec.cols, &columns, &spec.term, p, &class)?;
        Ok((spec.rows, spec.cols, class, spatial))
    })
}

/// The option price of Spatial's `BlackScholes` regression app (`pir/regression/BlackScholes.scala`,
/// `BlkSchlsEqEuroNoDiv`, with the Taylor approximations of `spatial.lang.api.MathAPI` its `CNDF` and the pricing
/// call), transcribed: over the columns `prices`, `strikes`, `rates`, `vols`, `times` (`Real`) and `types` (`Int`),
/// read in that order. Spatial's `-x` is LOGOS's `0 - x`.
pub fn blackscholes_query() -> ZipTerm {
    let r = ZipTerm::real;
    let op = |o, l: ZipTerm, r: ZipTerm| ZipTerm::Op(o, Box::new(l), Box::new(r));
    let (add, sub) = (|a, b| op(ZipOp::Add, a, b), |a, b| op(ZipOp::Sub, a, b));
    let (mul, div) = (|a, b| op(ZipOp::Mul, a, b), |a, b| op(ZipOp::Div, a, b));
    let lt = |a, b| op(ZipOp::Lt, a, b);
    let sq = |a| op(ZipOp::Pow, a, ZipTerm::Const(2));
    let neg = |a| sub(ZipTerm::Const(0), a);
    let sel = |c, a, b| ZipTerm::Select(Box::new(c), Box::new(a), Box::new(b));
    let exp_taylor = |x: ZipTerm| {
        let poly = add(
            add(
                add(add(add(r(1.0), x.clone()), div(mul(x.clone(), x.clone()), r(2.0))), div(mul(mul(x.clone(), x.clone()), x.clone()), r(6.0))),
                div(mul(mul(mul(x.clone(), x.clone()), x.clone()), x.clone()), r(24.0)),
            ),
            div(mul(mul(mul(mul(x.clone(), x.clone()), x.clone()), x.clone()), x.clone()), r(120.0)),
        );
        sel(lt(x.clone(), r(-3.5)), r(0.0), sel(lt(x.clone(), r(-1.2)), add(mul(x, r(0.1)), r(0.35)), poly))
    };
    let log_taylor = |x: ZipTerm| {
        let xx = sub(x, r(1.0));
        sub(
            add(sub(xx.clone(), div(mul(xx.clone(), xx.clone()), r(2.0))), div(mul(mul(xx.clone(), xx.clone()), xx.clone()), r(3.0))),
            div(mul(mul(mul(xx.clone(), xx.clone()), xx.clone()), xx), r(4.0)),
        )
    };
    let sqrt_approx = |x: ZipTerm| {
        let m1 = || sub(x.clone(), r(1.0));
        let taylor = add(sub(add(r(1.0), div(m1(), r(2.0))), div(mul(m1(), m1()), r(8.0))), div(mul(mul(m1(), m1()), m1()), r(16.0)));
        let line = |a: f64, b: f64| add(mul(x.clone(), r(a)), r(b));
        sel(
            lt(x.clone(), r(2.0)),
            taylor,
            sel(
                lt(x.clone(), r(10.0)),
                line(0.22, 1.0),
                sel(
                    lt(x.clone(), r(100.0)),
                    line(0.08, 2.5),
                    sel(lt(x.clone(), r(1000.0)), line(0.028, 8.0), sel(lt(x.clone(), r(10000.0)), line(0.008, 20.0), line(0.0002, 300.0))),
                ),
            ),
        )
    };
    let cndf = |x: ZipTerm| {
        let ax = ZipTerm::Abs(Box::new(x.clone()));
        let n_prime = mul(exp_taylor(mul(sq(ax.clone()), r(-0.05))), r(0.39894228040143270286));
        let k = div(r(1.0), add(mul(ax, r(0.2316419)), r(1.0)));
        let k2 = sq(k.clone());
        let k3 = mul(k2.clone(), k.clone());
        let k4 = mul(k3.clone(), k.clone());
        let k5 = mul(k4.clone(), k.clone());
        let l1 = add(
            add(add(add(mul(k2, r(-0.356563782)), mul(k3, r(1.781477937))), mul(k4, r(-1.821255978))), mul(k5, r(1.330274429))),
            mul(k, r(0.31938153)),
        );
        let l0 = mul(l1, n_prime);
        sel(lt(x, r(0.0)), l0.clone(), add(neg(l0), r(1.0)))
    };
    let (price, strike, rate, vol, time, otype) =
        (ZipTerm::Read(0), ZipTerm::Read(1), ZipTerm::Read(2), ZipTerm::Read(3), ZipTerm::Read(4), ZipTerm::Read(5));
    let log_term = log_taylor(div(price.clone(), strike.clone()));
    let power_term = mul(sq(vol.clone()), r(0.5));
    let num = add(mul(add(rate.clone(), power_term), time.clone()), log_term);
    let den = mul(vol, sqrt_approx(time.clone()));
    let d = div(num, sq(den.clone()));
    let (nd1, nd2) = (cndf(d.clone()), cndf(sub(d, den)));
    let future = mul(strike, exp_taylor(mul(neg(rate), time)));
    let (neg_nd1, neg_nd2) = (add(neg(nd1.clone()), r(1.0)), add(neg(nd2.clone()), r(1.0)));
    let price1 = sub(mul(price.clone(), nd1), mul(future.clone(), nd2));
    let price2 = sub(mul(future, neg_nd2), mul(price, neg_nd1));
    sel(op(ZipOp::Eq, otype, ZipTerm::Const(0)), price2, price1)
}

/// Black-Scholes in LOGOS over `n` options, as Spatial's BlackScholes app prices them — its Taylor approximations
/// written as LOGOS functions, the six columns declared in the app's order and filled with residues (types `p % 2`,
/// the prices, strikes, rates, volatilities and times scaled into ranges the approximations are taken over), every
/// option priced by one counted loop. It shows `Σ (t + 1)·out[t]`, a checksum a permuted or partial result does not
/// reproduce.
pub fn logos_blackscholes_program(n: usize) -> String {
    format!(
        "## To expTaylor (x: Real) -> Real:\n    If x is less than -3.5:\n        Return 0.0.\n    If x is less than -1.2:\n        Return x * 0.1 + 0.35.\n    Return 1.0 + x + x * x / 2.0 + x * x * x / 6.0 + x * x * x * x / 24.0 + x * x * x * x * x / 120.0.\n\n\
## To logTaylor (x: Real) -> Real:\n    Let xx be x - 1.0.\n    Return xx - xx * xx / 2.0 + xx * xx * xx / 3.0 - xx * xx * xx * xx / 4.0.\n\n\
## To sqrtApprox (x: Real) -> Real:\n    If x is less than 2.0:\n        Return 1.0 + (x - 1.0) / 2.0 - (x - 1.0) * (x - 1.0) / 8.0 + (x - 1.0) * (x - 1.0) * (x - 1.0) / 16.0.\n    If x is less than 10.0:\n        Return x * 0.22 + 1.0.\n    If x is less than 100.0:\n        Return x * 0.08 + 2.5.\n    If x is less than 1000.0:\n        Return x * 0.028 + 8.0.\n    If x is less than 10000.0:\n        Return x * 0.008 + 20.0.\n    Return x * 0.0002 + 300.0.\n\n\
## To cndf (x: Real) -> Real:\n    Let ax be abs(x).\n    Let xNPrimeofX be expTaylor(ax ** 2 * -0.05) * 0.3989422804014327.\n    Let xK2 be 1.0 / (ax * 0.2316419 + 1.0).\n    Let xK2_2 be xK2 ** 2.\n    Let xK2_3 be xK2_2 * xK2.\n    Let xK2_4 be xK2_3 * xK2.\n    Let xK2_5 be xK2_4 * xK2.\n    Let xLocal_10 be xK2 * 0.31938153.\n    Let xLocal_20 be xK2_2 * -0.356563782.\n    Let xLocal_30 be xK2_3 * 1.781477937.\n    Let xLocal_31 be xK2_4 * -1.821255978.\n    Let xLocal_32 be xK2_5 * 1.330274429.\n    Let xLocal_21 be xLocal_20 + xLocal_30.\n    Let xLocal_22 be xLocal_21 + xLocal_31.\n    Let xLocal_23 be xLocal_22 + xLocal_32.\n    Let xLocal_1 be xLocal_23 + xLocal_10.\n    Let xLocal0 be xLocal_1 * xNPrimeofX.\n    Let xLocal be 0 - xLocal0 + 1.0.\n    If x is less than 0.0:\n        Return xLocal0.\n    Return xLocal.\n\n\
## To blackScholes (sptprice: Real, strike: Real, rate: Real, volatility: Real, time: Real, otype: Int) -> Real:\n    Let xLogTerm be logTaylor(sptprice / strike).\n    Let xPowerTerm be volatility ** 2 * 0.5.\n    Let xNum be (rate + xPowerTerm) * time + xLogTerm.\n    Let xDen be volatility * sqrtApprox(time).\n    Let xDiv be xNum / (xDen ** 2).\n    Let nofXd1 be cndf(xDiv).\n    Let nofXd2 be cndf(xDiv - xDen).\n    Let futureValueX be strike * expTaylor((0 - rate) * time).\n    Let negNofXd1 be 0 - nofXd1 + 1.0.\n    Let negNofXd2 be 0 - nofXd2 + 1.0.\n    Let optionPrice1 be sptprice * nofXd1 - futureValueX * nofXd2.\n    Let optionPrice2 be futureValueX * negNofXd2 - sptprice * negNofXd1.\n    If otype is 0:\n        Return optionPrice2.\n    Return optionPrice1.\n\n\
## Main\nLet n be {n}.\nLet mutable types be a new Seq of Int.\nLet mutable prices be a new Seq of Real.\nLet mutable strikes be a new Seq of Real.\nLet mutable rates be a new Seq of Real.\nLet mutable vols be a new Seq of Real.\nLet mutable times be a new Seq of Real.\nLet mutable out be a new Seq of Real.\nLet mutable p be 0.\nWhile p is less than n:\n    Push p % 2 to types.\n    Push (p % 97 + 50) / 100.0 to prices.\n    Push (p % 89 + 50) / 100.0 to strikes.\n    Push (p % 13 + 1) / 100.0 to rates.\n    Push (p % 41 + 10) / 100.0 to vols.\n    Push (p % 23 + 2) / 4.0 to times.\n    Push 0.0 to out.\n    Set p to p + 1.\nLet mutable i be 0.\nWhile i is less than n:\n    Set item (i + 1) of out to blackScholes(item (i + 1) of prices, item (i + 1) of strikes, item (i + 1) of rates, item (i + 1) of vols, item (i + 1) of times, item (i + 1) of types).\n    Set i to i + 1.\nLet mutable sum be 0.0.\nSet p to 0.\nWhile p is less than n:\n    Set sum to sum + item (p + 1) of out * (p + 1).\n    Set p to p + 1.\nShow sum.\n"
    )
}

/// A LOGOS Black-Scholes — the zip map whose term is [`blackscholes_query`] — emitted as Spatial's BlackScholes
/// regression app at the knobs `p`, with the variant's PIR arguments (`pir_args`, what its `pirArgs` override
/// appends): `(n, class name, Spatial source)`, or a typed reason. The columns are loaded in the order the program
/// declares them, as the app loads its own. The class name carries the length and every knob in Spatial's parameter
/// order.
pub fn lower_blackscholes_source(source: &str, p: MapPar, pir_args: Option<&str>) -> Result<(usize, String, String), String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.map_err(|e| format!("parse: {e:?}"))?;
        let query = blackscholes_query();
        let map = crate::tensor_recognize::recognize_zip_map_where(stmts, interner, &|m| m.term == query)
            .map_err(|e| format!("not Black-Scholes as the tensor recognizer reads it: {e:?}"))?;
        let columns: Vec<MapColumn> = map
            .operands
            .iter()
            .zip(&map.types)
            .map(|(o, ty)| MapColumn {
                name: interner.resolve(*o).to_string(),
                ty: *ty,
                fill: crate::tensor_recognize::recognize_fill(stmts, *o),
            })
            .collect();
        let declared = declaration_order(stmts, &map.operands)?;
        let class = format!("Logos_bs_{}_ts{}_op{}_ip{}", map.n, p.ts, p.op, p.ip);
        let out = interner.resolve(map.out).to_string();
        let spatial = emit_spatial_zip_map_test(map.n, &columns, &declared, &map.term, &out, p, pir_args, &class)?;
        Ok((map.n, class, spatial))
    })
}

/// The arrays `arrays` in the order the program declares them (its top-level `Let`s): each one's position in
/// `arrays`.
fn declaration_order(stmts: &[logicaffeine_language::ast::stmt::Stmt], arrays: &[crate::intern::Symbol]) -> Result<Vec<usize>, String> {
    let at = |a: &crate::intern::Symbol| {
        stmts
            .iter()
            .position(|s| matches!(s, logicaffeine_language::ast::stmt::Stmt::Let { var, .. } if var == a))
            .ok_or_else(|| "a column the program reads is not declared at its top level".to_string())
    };
    let mut order: Vec<(usize, usize)> = arrays.iter().enumerate().map(|(k, a)| Ok((at(a)?, k))).collect::<Result<_, String>>()?;
    order.sort_unstable();
    Ok(order.into_iter().map(|(_, k)| k).collect())
}

/// The distance a feature adds in Spatial's `Kmeans` regression app (`pir/regression/Kmeans.scala`), transcribed:
/// `(pts(pt,d) - cts(ct,d)) ** 2` over the point's feature and the centroid's, read in that order.
pub fn kmeans_distance_query() -> ZipTerm {
    let sub = ZipTerm::Op(ZipOp::Sub, Box::new(ZipTerm::Read(0)), Box::new(ZipTerm::Read(1)));
    ZipTerm::Op(ZipOp::Pow, Box::new(sub), Box::new(ZipTerm::Const(2)))
}

/// K-means clustering in LOGOS — Lloyd's algorithm as Spatial's Kmeans app runs it: `n` points of `f` features
/// (row-major, filled `q % 97` over the flat index), `k` centroids starting as the first `k` points, `iters` epochs
/// each assigning every point to its nearest centroid by the squared distance (the last of equally near ones, as the
/// app's accelerator), summing and counting each centroid's points, and setting every centroid to the integer mean of
/// its points (0 for one with none). It shows `Σ (t + 1)·centroids[t]`, a checksum a permuted or partial result does
/// not reproduce.
pub fn logos_kmeans_program(n: usize, k: usize, f: usize, iters: usize) -> String {
    format!(
        "## Main\nLet n be {n}.\nLet k be {k}.\nLet f be {f}.\nLet iters be {iters}.\nLet mutable pts be a new Seq of Int.\nLet mutable cts be a new Seq of Int.\nLet mutable sums be a new Seq of Int.\nLet mutable counts be a new Seq of Int.\nLet mutable q be 0.\nWhile q is less than n * f:\n    Push q % 97 to pts.\n    Set q to q + 1.\nSet q to 0.\nWhile q is less than k * f:\n    Push item (q + 1) of pts to cts.\n    Push 0 to sums.\n    Set q to q + 1.\nSet q to 0.\nWhile q is less than k:\n    Push 0 to counts.\n    Set q to q + 1.\nLet mutable epoch be 0.\nWhile epoch is less than iters:\n    Set q to 0.\n    While q is less than k * f:\n        Set item (q + 1) of sums to 0.\n        Set q to q + 1.\n    Set q to 0.\n    While q is less than k:\n        Set item (q + 1) of counts to 0.\n        Set q to q + 1.\n    Let mutable p be 0.\n    While p is less than n:\n        Let mutable best be 0.\n        Let mutable bestDist be 0.\n        Let mutable c be 0.\n        While c is less than k:\n            Let mutable dist be 0.\n            Let mutable d be 0.\n            While d is less than f:\n                Set dist to dist + (item (p * f + d + 1) of pts - item (c * f + d + 1) of cts) ** 2.\n                Set d to d + 1.\n            If c is 0 or dist is at most bestDist:\n                Set best to c.\n                Set bestDist to dist.\n            Set c to c + 1.\n        Set q to 0.\n        While q is less than f:\n            Set item (best * f + q + 1) of sums to item (best * f + q + 1) of sums + item (p * f + q + 1) of pts.\n            Set q to q + 1.\n        Set item (best + 1) of counts to item (best + 1) of counts + 1.\n        Set p to p + 1.\n    Let mutable j be 0.\n    While j is less than k:\n        Let cnt be item (j + 1) of counts.\n        Set q to 0.\n        While q is less than f:\n            If cnt is 0:\n                Set item (j * f + q + 1) of cts to 0.\n            Otherwise:\n                Set item (j * f + q + 1) of cts to item (j * f + q + 1) of sums / cnt.\n            Set q to q + 1.\n        Set j to j + 1.\n    Set epoch to epoch + 1.\nLet mutable sum be 0.\nSet q to 0.\nWhile q is less than k * f:\n    Set sum to sum + item (q + 1) of cts * (q + 1).\n    Set q to q + 1.\nShow sum.\n"
    )
}

/// A LOGOS k-means clustering — whose distance is [`kmeans_distance_query`] — emitted as Spatial's Kmeans regression
/// app at the knobs `p`: `(n, k, features + 1, epochs, class name, Spatial source)` (the app's `N`, `K`, `D` and
/// runtime `I`), or a typed reason. The class name carries the shape, the epochs and every knob in Spatial's parameter
/// order.
pub fn lower_kmeans_source(source: &str, p: KmeansPar) -> Result<(usize, usize, usize, usize, String, String), String> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.map_err(|e| format!("parse: {e:?}"))?;
        let spec = crate::tensor_recognize::recognize_kmeans(stmts, interner)
            .map_err(|e| format!("not k-means as the tensor recognizer reads it: {e:?}"))?;
        if spec.distance != kmeans_distance_query() {
            return Err("k-means, but not by the distance Spatial's Kmeans app sums".to_string());
        }
        let points = crate::tensor_recognize::recognize_fill(stmts, spec.points);
        let class = format!(
            "Logos_kmeans_{}x{}_k{}_i{}_ts{}_ip{}_op{}_mp{}x{}x{}",
            spec.n, spec.f + 1, spec.k, spec.iters, p.ts, p.ip, p.op, p.mp1, p.mp2, p.mp3
        );
        let spatial = emit_spatial_kmeans_test(&spec, &points, p, &class)?;
        Ok((spec.n, spec.k, spec.f + 1, spec.iters, class, spatial))
    })
}

/// A LOGOS outer product, recognized (`tensor_recognize::recognize_outer`) and emitted as Spatial's
/// OuterProduct regression app at the knobs `p`: `(m, n, class name, Spatial source)`, or a typed reason. The
/// class name carries the shape and every knob in Spatial's parameter order (`ts1 x ts2`, `ip x ip1 x ip2`,
/// `op1 x op2`).
pub fn lower_outer_source(source: &str, p: OuterPar) -> Result<(usize, usize, String, String), String> {
    let (m, n) = crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.map_err(|e| format!("parse: {e:?}"))?;
        crate::tensor_recognize::recognize_outer(stmts, interner)
            .map(|spec| (spec.m, spec.n))
            .map_err(|e| format!("not an outer product the tensor recognizer reads: {e:?}"))
    })?;
    let class = format!(
        "Logos_outer_{m}x{n}_ts{}x{}_ip{}x{}x{}_op{}x{}",
        p.ts1, p.ts2, p.ip, p.ip1, p.ip2, p.op1, p.op2
    );
    let spatial = emit_spatial_outer_test(m, n, p, &class)?;
    Ok((m, n, class, spatial))
}
