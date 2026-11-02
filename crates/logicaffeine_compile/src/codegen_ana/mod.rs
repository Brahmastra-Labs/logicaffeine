//! LOGOS → **analog spatial fabric** (FPAA-style mixed-signal coprocessor) — the analog lane.
//!
//! Analog computing is inherently spatial: there is no program counter to run through continuous
//! physics, so a computation is laid out as a topology and signals flow through it. This module
//! lowers the float-dataflow region of a LOGOS program onto an analog execution plane (Kirchhoff
//! current-summing adds, transconductance multiplies, OTA-C integrators) framed by a digital
//! control plane (DAC/ADC strobes, sample-and-hold ticks).
//!
//! ## The trust model
//! Everything except one arrow stays **exact**: the digital control plane, the gain-schedule
//! rationals, calibration fixed-point, and the DAC/ADC quantizer functions. The single
//! analog-uncertain arrow — DAC-out to ADC-in — is *bounded* (an `ErrorBudget` the compiler
//! emits), *certified non-clipping* (a range certificate replayed by an independent checker),
//! and *sampled/simulated* (fixed-seed Monte-Carlo, SPICE). ε is a compiler output derived from
//! the artifact, never a hand-tuned test tolerance; `logos_approx_eq` is the ideal-tier floor
//! and is never widened.
//!
//! ## Proof-regime firewall
//! [`crate::codegen_sva`]'s value is exact ≡VM (SAT bitblast, byte-identical eval, iverilog
//! diff). The analog lane is tolerance-≡ by physics, so it lives in its OWN ADL
//! ([`target::AnalogTarget`]) and imports the digital fabric's extraction IR read-only —
//! `codegen_sva` itself is untouched.
//!
//! ## Totality discipline (mirrors the Tenstorrent / RISC-V tiers)
//! Every program gets a correct artifact or a NAMED refusal, never a silent miscompile:
//! concurrency/networking → [`AnaLowerError::Unsupported`]; no float dataflow →
//! [`AnaLowerError::NoAnalogRegion`]; float dataflow without declared input ranges →
//! [`AnaLowerError::UnboundedInput`] (gain scaling without bounds would be a guess, and this
//! lane does not guess). The `analog_op_coverage_lock` ratchet classifies all VM ops four ways
//! (OnPlane / Boundary / ControlPlane / OffPlane) so nothing is silently dropped.

pub mod calibrate;
pub mod cell;
pub mod control;
pub mod dims;
pub mod dynamics;
pub mod effects;
pub mod error_budget;
pub mod eval;
pub mod gain;
pub mod int_lane;
pub mod integrator;
pub mod lower;
pub mod nonideal;
pub mod ontology;
pub mod place;
pub mod reduction;
pub mod spice_emit;
pub mod stack;
pub mod stack_place;
pub mod target;
pub mod thermal;
pub mod wta;
pub mod xbar;

pub use calibrate::{
    apply_calibration, calibrate, error_budget_calibrated, v1_trim, CalibrationTable, TrimSpec,
};
pub use cell::{ASrc, AnalogCell, AnalogNetlist};
pub use control::{ControlProgram, CtrlStep};
pub use error_budget::{
    error_budget, error_budget_chopped, error_budget_nonideal, ErrorSource, ErrorTerm, OutputBudget,
};
pub use eval::{eval_analog_dc, eval_analog_quantized, ConverterGrid};
pub use gain::{scale_pass, to_range_claim, GainSchedule, MathBound, NetBound, RangeCertificate};
pub use int_lane::analog_int_netlist_from_source;
pub use integrator::{
    analog_integrator_from_source, eval_analog_integrated, eval_analog_integrated_quantized,
    integrator_error_budget, integrator_scale_pass, AnalogIntegrator, InitSrc,
};
pub use lower::analog_netlist_from_source;
pub use nonideal::{
    eval_analog_instance, eval_analog_instance_chopped, sample_instance, sky130_nonideal,
    InstanceSample, NonIdealityModel, KSIGMA,
};
pub use place::{place_analog, place_blind, AnalogPlacement};
pub use reduction::{
    analog_int_reduction_from_source, analog_reduction_from_source, eval_analog_sampled,
    eval_analog_sampled_quantized, reduction_error_budget, reduction_scale_pass, AnalogReduction,
};
pub use spice_emit::{emit_spice_dc, SPICE_ABSTOL, SPICE_RELTOL, SPICE_VNTOL};
pub use target::{
    AnaUnplaceable, AnalogCapability, AnalogCellKind, AnalogTarget, ALL_ANALOG_CELL_KINDS,
};
pub use xbar::{fold_crossbar, fold_crossbar_for};

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use crate::intern::Interner;
use crate::vm::{CompiledProgram, Constant};

/// The mixed-signal compilation artifact: the lowered region, its physical schedule and
/// placement, the certificates and the budget, and the behavioral SPICE netlist (emitted at the
/// declared input box's midpoint).
#[derive(Debug, Clone, PartialEq)]
pub struct AnalogArtifact {
    pub fn_name: String,
    pub netlist: AnalogNetlist,
    pub schedule: GainSchedule,
    pub cert: RangeCertificate,
    pub budget: OutputBudget,
    pub placement: AnalogPlacement,
    pub spice: String,
}

/// The contract-free twin of a source: the `Assume` range-check lines stripped. The checks
/// belong to the CONTROL PLANE (the digital side validates each word before the DAC fires, and
/// runs them as ordinary bytecode in the digital fallback), so the region DATAFLOW both lanes
/// lower is this twin — exposed for the mixed-signal composition to build its fallback configs
/// from the same text the analog lane extracts.
pub fn contract_free_twin(src: &str) -> String {
    lower::source_without_range_contracts(src)
}

/// Which certified lane a region compiled through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionLane {
    /// ε-certified float dataflow.
    Float,
    /// ½-grid-certified EXACT integer dataflow (calibrated budget).
    Int,
}

/// Try to compile ONE region function on `target`, deciding the lane from its parameter
/// family: float params → the ε-certified pipeline; else the certified-exact integer pipeline.
/// PANIC-SAFE: extraction shapes outside the supported family become a typed refusal, never a
/// crash — this is the entry the mixed-signal partition (and `compile_to_analog`) dispatches
/// through for arbitrary programs.
pub fn try_region(
    src: &str,
    fn_name: &str,
    target: &AnalogTarget,
) -> Result<(RegionLane, AnalogArtifact), AnaLowerError> {
    let guard = |f: &dyn Fn() -> Result<AnalogArtifact, AnaLowerError>| {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| {
            Err(AnaLowerError::UnloweredOp(
                "region outside the supported extraction family".to_string(),
            ))
        })
    };
    match guard(&|| analog_artifact_from_source(src, fn_name, target)) {
        Ok(art) => Ok((RegionLane::Float, art)),
        // Not a float region — the integer lane decides (its own gates refuse non-int shapes).
        Err(AnaLowerError::NoAnalogRegion) => {
            guard(&|| analog_int_artifact_from_source(src, fn_name, target))
                .map(|a| (RegionLane::Int, a))
        }
        Err(e) => Err(e),
    }
}

/// The INTEGER pipeline for one region function: lower through the int lane, schedule, place,
/// take the CALIBRATED physical budget (the product configuration — trims burned at final
/// test), and PROVE exactness through the ½-grid gate before any artifact exists. An integer
/// artifact is therefore certified-exact by construction: its budget is the proof.
pub fn analog_int_artifact_from_source(
    src: &str,
    fn_name: &str,
    target: &AnalogTarget,
) -> Result<AnalogArtifact, AnaLowerError> {
    let netlist = int_lane::analog_int_netlist_from_source(src, fn_name, target)?;
    let ranges = int_lane::int_region_param_ranges(src, fn_name)?;
    let (schedule, cert) = scale_pass(&netlist, &ranges, target)?;
    let placement = place_analog(&schedule.quantized, target)?;
    let model = sky130_nonideal();
    let spec = calibrate::v1_trim(target);
    let budget = error_budget_calibrated(
        &netlist, &cert, &schedule, target, &model, &placement, &spec,
    );
    exactness_gate(&budget)?;
    let mid: Vec<f64> = ranges
        .iter()
        .map(|r| ((r.lo + r.hi) / 2.0).round())
        .collect();
    let spice = spice_emit::emit_spice_dc(&schedule, target, &mid, "out.txt");
    Ok(AnalogArtifact {
        fn_name: fn_name.to_string(),
        netlist,
        schedule,
        cert,
        budget,
        placement,
        spice,
    })
}

/// The full DAG-lane pipeline for one region function on one target: lower → schedule/certify →
/// place → budget (physical, with the sourced non-ideality model) → behavioral SPICE.
pub fn analog_artifact_from_source(
    src: &str,
    fn_name: &str,
    target: &AnalogTarget,
) -> Result<AnalogArtifact, AnaLowerError> {
    let netlist = analog_netlist_from_source(src, fn_name, target)?;
    let ranges = lower::region_param_ranges(src, fn_name)?;
    let (schedule, cert) = scale_pass(&netlist, &ranges, target)?;
    let placement = place_analog(&schedule.quantized, target)?;
    let model = sky130_nonideal();
    let budget = error_budget_nonideal(&netlist, &cert, &schedule, target, &model, &placement);
    let mid: Vec<f64> = ranges.iter().map(|r| (r.lo + r.hi) / 2.0).collect();
    let spice = spice_emit::emit_spice_dc(&schedule, target, &mid, "out.txt");
    Ok(AnalogArtifact {
        fn_name: fn_name.to_string(),
        netlist,
        schedule,
        cert,
        budget,
        placement,
        spice,
    })
}

/// Why the analog tier declines a program — a SOUND refusal, never wrong silicon.
#[derive(Debug, Clone, PartialEq)]
pub enum AnaLowerError {
    /// The program uses an op outside the mixed-signal SoC model (concurrency / networking).
    Unsupported(&'static str),
    /// The program has no float dataflow to map onto the analog plane.
    NoAnalogRegion,
    /// Float dataflow is present but an input has no declared range — refused, because gain
    /// scaling without bounds would be a guess.
    UnboundedInput(String),
    /// An op inside the region has no witnessed analog tile — a named lowering gap, never a
    /// silent drop.
    UnloweredOp(String),
    /// The integer lane's exactness proof failed: the error budget reaches or exceeds half an
    /// integer grid step, so the rounded readout cannot be guaranteed to be the VM's integer.
    /// Shrink the ranges/depth, use more converter bits, or calibrate.
    ExactnessUnprovable { eps_math: f64 },
    /// A declared range is empty (`lo > hi`) — no value can satisfy it, so no scaling or
    /// certificate built on it can mean anything. Refused for bare and dimensioned contracts
    /// alike (the runtime twin simply fires the impossible assert when reached).
    ContradictoryContract { var: String, lo: f64, hi: f64 },
    /// The relaxation lane's admission gate: no cited convergence theorem covers this system
    /// (asymmetric, indefinite, malformed) — refused, never settled into a wrong answer.
    ConvergenceUnprovable { reason: String },
    /// The stack's thermal network has heat with nowhere to go (a floating island / no
    /// cooling boundary) — no steady state exists, so no temperature map is emitted.
    ThermalModelDisconnected,
    /// The CERTIFIED temperature map's hottest junction exceeds the stack's declared cooling
    /// budget — the same program may compile under a stronger cooling class; under this one
    /// it refuses. "Meets its budget under forced-air cooling, proven" is the checker's
    /// verdict, not the producer's word.
    ThermalBudgetExceeded {
        node: usize,
        tier: usize,
        t_junction_c: f64,
        t_max_c: f64,
        cooling: &'static str,
    },
    /// The settled answer's EXACT residual exceeds the demanded certificate budget — the run
    /// is under-settled (or the demand unrealistic); no certificate is emitted.
    ResidualExceedsBudget { linf: f64, required: f64 },
    /// The netlist cannot be realized on the requested target.
    Unplaceable(AnaUnplaceable),
}

impl std::fmt::Display for AnaLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnaLowerError::Unsupported(what) => write!(f, "unsupported {what}"),
            AnaLowerError::NoAnalogRegion => {
                write!(f, "no analog region: the program has no float dataflow to map onto the analog plane")
            }
            AnaLowerError::UnboundedInput(msg) => write!(f, "unbounded input: {msg}"),
            AnaLowerError::UnloweredOp(what) => write!(f, "unlowered op: {what}"),
            AnaLowerError::ExactnessUnprovable { eps_math } => write!(
                f,
                "exactness unprovable: the error budget ε = {eps_math} reaches half an integer \
                 grid step — shrink ranges/depth, add converter bits, or calibrate"
            ),
            AnaLowerError::ContradictoryContract { var, lo, hi } => write!(
                f,
                "contradictory contract: `{var}` is bounded to [{lo}, {hi}], an empty range — \
                 no value can satisfy it"
            ),
            AnaLowerError::ConvergenceUnprovable { reason } => {
                write!(f, "convergence unprovable: {reason}")
            }
            AnaLowerError::ThermalModelDisconnected => write!(
                f,
                "thermal model disconnected: heat has nowhere to go (floating island or no \
                 cooling boundary) — no steady state exists"
            ),
            AnaLowerError::ThermalBudgetExceeded {
                node,
                tier,
                t_junction_c,
                t_max_c,
                cooling,
            } => write!(
                f,
                "thermal budget exceeded: node {node} (tier {tier}) reaches {t_junction_c} degC \
                 against the declared {t_max_c} degC junction ceiling under {cooling} cooling — \
                 use a stronger cooling class or a cooler layout"
            ),
            AnaLowerError::ResidualExceedsBudget { linf, required } => write!(
                f,
                "residual exceeds budget: ‖A·x̂ − b‖∞ = {linf} > required {required} — \
                 under-settled or the demand is unrealistic; no certificate emitted"
            ),
            AnaLowerError::Unplaceable(u) => write!(f, "{u}"),
        }
    }
}

impl std::error::Error for AnaLowerError {}

/// Returns `Some(reason)` if `program` uses an op with no home anywhere in the mixed-signal SoC
/// model — concurrency (the analog control plane is a frame sequencer, not a thread pool) and
/// networking (host-only IO). Everything else either maps to the plane, frames its boundary, or
/// runs digitally VM-identical on the control side.
pub fn unsupported_reason(program: &CompiledProgram) -> Option<&'static str> {
    use crate::aot_unsupported::Offload;
    crate::aot_unsupported::first(program).map(|o| match o {
        Offload::Channel => "channel op (no channels in the mixed-signal SoC model)",
        Offload::Task => {
            "task spawn/await (the analog control plane is a frame sequencer, not a thread pool)"
        }
        Offload::VirtualTime => "virtual-time / select (needs a cooperative scheduler)",
        Offload::Net => "networking op (host-only IO)",
        Offload::Dom => "DOM / browser op (no document in the mixed-signal SoC model)",
    })
}

/// Whether the program carries any float dataflow at all — the analog-region existence probe.
/// A float constant in the pool is the witness; the region extractor sharpens this to the real
/// dataflow graph when the lowering lands.
fn has_float_dataflow(program: &CompiledProgram) -> bool {
    program
        .constants
        .iter()
        .any(|c| matches!(c, Constant::Float(_)))
}

/// The fail-closed refusal gate every analog compilation runs FIRST. `None` means the program
/// clears the gate (an analog region exists, with declared ranges, inside the SoC model).
pub fn analog_refusal(program: &CompiledProgram, ranges: &[AssumedRange]) -> Option<AnaLowerError> {
    if let Some(reason) = unsupported_reason(program) {
        return Some(AnaLowerError::Unsupported(reason));
    }
    if !has_float_dataflow(program) {
        return Some(AnaLowerError::NoAnalogRegion);
    }
    if ranges.is_empty() {
        return Some(AnaLowerError::UnboundedInput(
            "float dataflow is present but no input has a declared range (`Assume x is between A and B.`)"
                .to_string(),
        ));
    }
    if let Some(r) = ranges.iter().find(|r| r.lo > r.hi) {
        return Some(AnaLowerError::ContradictoryContract {
            var: r.var.clone(),
            lo: r.lo,
            hi: r.hi,
        });
    }
    None
}

/// The integer lane's ½-grid exactness proof: integer inputs on a grid-preserving region read
/// back EXACTLY through the rounding ADC iff the compiler's own error budget stays under half
/// a grid step. `Ok(())` is the exactness CERTIFICATE'S admission condition; the refusal
/// carries the budget that broke so the diagnostic names the cost.
pub fn exactness_gate(budget: &OutputBudget) -> Result<(), AnaLowerError> {
    if budget.eps_math < 0.5 {
        Ok(())
    } else {
        Err(AnaLowerError::ExactnessUnprovable {
            eps_math: budget.eps_math,
        })
    }
}

/// A declared input range: `Assume x is between lo and hi.` (or the `Require that` spelling —
/// the range IS the enforced `RuntimeAssert` shape, not the head keyword). The gain-scaling
/// pass's input: a variable whose runtime value is CONTRACT-bounded to `[lo, hi]` on every tier.
#[derive(Debug, Clone, PartialEq)]
pub struct AssumedRange {
    pub var: String,
    pub lo: f64,
    pub hi: f64,
    /// The contract's physical dimension — recovered from the parser's quantity-bound fold
    /// (`Assume v is between 0.2 volts and 0.7 volts.`). Bare numeric contracts carry
    /// `Dimension::DIMENSIONLESS`. Metadata + coherence in this phase: the artifact bits are
    /// identical to the bare twin's; netlist dimension propagation is a later phase.
    pub dim: logicaffeine_base::dimension::Dimension,
}

/// Recover every declared range from a program's source — the source-level surface of
/// [`assumed_ranges_in`], for callers outside the compile pipeline.
pub fn assumed_ranges(source: &str) -> Vec<AssumedRange> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| match parsed {
        Ok((stmts, _types, _pol)) => assumed_ranges_in(stmts, interner),
        Err(_) => Vec::new(),
    })
}

/// Recover every declared range from parsed statements: each enforced (`hard`) `RuntimeAssert`
/// whose condition is the `between` desugar shape `lo <= x and x <= hi` with literal bounds.
/// Walks function bodies and nested blocks — a contract is a contract wherever it is stated.
pub fn assumed_ranges_in(stmts: &[Stmt<'_>], interner: &Interner) -> Vec<AssumedRange> {
    let mut out = Vec::new();
    collect_ranges(stmts, interner, &mut out);
    out
}

fn collect_ranges(stmts: &[Stmt<'_>], interner: &Interner, out: &mut Vec<AssumedRange>) {
    for stmt in stmts {
        match stmt {
            Stmt::RuntimeAssert {
                condition,
                hard: true,
                dim,
                reason: _,
            } => {
                if let Some((var, lo, hi)) = match_between(condition) {
                    out.push(AssumedRange {
                        var: interner.resolve(var).to_string(),
                        lo,
                        hi,
                        dim: dim.unwrap_or(logicaffeine_base::dimension::Dimension::DIMENSIONLESS),
                    });
                }
            }
            Stmt::FunctionDef { body, .. } => collect_ranges(body, interner, out),
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_ranges(then_block, interner, out);
                if let Some(e) = else_block {
                    collect_ranges(e, interner, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                collect_ranges(body, interner, out)
            }
            _ => {}
        }
    }
}

/// A declared COLLECTION-LENGTH range: `Assume length of xs is between A and B.` — the streamed
/// lane's frame bound (the control plane schedules `hi` frames; the runtime check enforces the
/// contract on every tier, so a longer sequence fails loudly before the DAC ever fires).
#[derive(Debug, Clone, PartialEq)]
pub struct AssumedLenRange {
    pub var: String,
    pub lo: i64,
    pub hi: i64,
}

/// Recover every declared length range from parsed statements (the `between` desugar over a
/// `length of <ident>` operand, in an enforced `RuntimeAssert`).
pub fn assumed_len_ranges_in(stmts: &[Stmt<'_>], interner: &Interner) -> Vec<AssumedLenRange> {
    let mut out = Vec::new();
    collect_len_ranges(stmts, interner, &mut out);
    out
}

fn collect_len_ranges(stmts: &[Stmt<'_>], interner: &Interner, out: &mut Vec<AssumedLenRange>) {
    for stmt in stmts {
        match stmt {
            Stmt::RuntimeAssert {
                condition,
                hard: true,
                dim: _,
                reason: _,
            } => {
                if let Some((var, lo, hi)) = match_len_between(condition) {
                    out.push(AssumedLenRange {
                        var: interner.resolve(var).to_string(),
                        lo,
                        hi,
                    });
                }
            }
            Stmt::FunctionDef { body, .. } => collect_len_ranges(body, interner, out),
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                collect_len_ranges(then_block, interner, out);
                if let Some(e) = else_block {
                    collect_len_ranges(e, interner, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } => {
                collect_len_ranges(body, interner, out)
            }
            _ => {}
        }
    }
}

/// Match `lo <= length of xs and length of xs <= hi` with integer literal bounds.
fn match_len_between(e: &Expr<'_>) -> Option<(crate::intern::Symbol, i64, i64)> {
    let Expr::BinaryOp {
        op: BinaryOpKind::And,
        left,
        right,
    } = e
    else {
        return None;
    };
    let Expr::BinaryOp {
        op: BinaryOpKind::LtEq,
        left: lo,
        right: lv,
    } = left
    else {
        return None;
    };
    let Expr::BinaryOp {
        op: BinaryOpKind::LtEq,
        left: rv,
        right: hi,
    } = right
    else {
        return None;
    };
    let (Expr::Length { collection: ca }, Expr::Length { collection: cb }) = (lv, rv) else {
        return None;
    };
    let (Expr::Identifier(a), Expr::Identifier(b)) = (ca, cb) else {
        return None;
    };
    if a != b {
        return None;
    }
    let int_lit = |e: &Expr<'_>| -> Option<i64> {
        match e {
            Expr::Literal(Literal::Number(n)) => Some(*n),
            _ => None,
        }
    };
    Some((*a, int_lit(lo)?, int_lit(hi)?))
}

/// Match the parser's `is between` desugar: `(lo <= x) and (x <= hi)` with literal numeric bounds.
fn match_between(e: &Expr<'_>) -> Option<(crate::intern::Symbol, f64, f64)> {
    let Expr::BinaryOp {
        op: BinaryOpKind::And,
        left,
        right,
    } = e
    else {
        return None;
    };
    let Expr::BinaryOp {
        op: BinaryOpKind::LtEq,
        left: lo,
        right: lv,
    } = left
    else {
        return None;
    };
    let Expr::BinaryOp {
        op: BinaryOpKind::LtEq,
        left: rv,
        right: hi,
    } = right
    else {
        return None;
    };
    let (Expr::Identifier(a), Expr::Identifier(b)) = (lv, rv) else {
        return None;
    };
    if a != b {
        return None;
    }
    Some((*a, numeric_literal(lo)?, numeric_literal(hi)?))
}

/// A literal numeric bound, including the negated spelling.
fn numeric_literal(e: &Expr<'_>) -> Option<f64> {
    match e {
        Expr::Literal(Literal::Float(f)) => Some(*f),
        Expr::Literal(Literal::Number(n)) => Some(*n as f64),
        Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left,
            right,
        } => {
            // `-1.0` may parse as `0 - 1.0`; accept the zero-minus-literal spelling.
            match (numeric_literal(left), numeric_literal(right)) {
                (Some(l), Some(r)) if l == 0.0 => Some(-r),
                _ => None,
            }
        }
        _ => None,
    }
}
