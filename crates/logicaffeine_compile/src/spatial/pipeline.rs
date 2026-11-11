//! The front-end bridge and the pinned pass order.

use super::ir::*;
#[allow(unused_imports)]
use super::ir::{BlockPlan, EnginePacking};
use super::link::{dense_tables, link, Agreed, Form, Prelude};
use super::passes::from_tensor_op::{from_matmul, MatmulNames};
use super::passes::recognize_stages::{float_tensors, links, state_tensors, Link, TensorOpSpec};
use super::passes::{
    fuse_generic, fuse_readers, persist, place, size_buffers, synthesize_handshakes, tile, verify,
    SpatialReject,
};
use super::target::SpatialTarget;
use crate::codegen_sva::fabric_target::Precision;
use crate::tensor_graph::graph::{Lifted, Op, Storage, TensorGraph};
use crate::tensor_recognize::{
    recognize_elementwise, recognize_fill, recognize_fused_row, recognize_matmul, recognize_swiglu,
    Fill, MatmulSpec,
};

/// A pass: a pure function of the program and its target.
pub type Pass = fn(&SpatialProgram, &SpatialTarget) -> Result<SpatialProgram, SpatialReject>;

/// The pinned order. Every entry is idempotent on its own output and leaves `validate()` holding.
pub const PASS_ORDER: &[(&str, Pass)] = &[
    // ★ FUSION RUNS FIRST, ON THE TEMPLATE. Legalization split a workload into stages so the split
    // could be CHECKED; this puts back together whatever the destination registers can carry, before
    // any of it is tiled, placed or scheduled. Removing a stage here costs only a dense renumber.
    ("fuse_generic", fuse_generic::fuse_generic),
    ("tile", tile::tile),
    ("place", place::place),
    ("fuse_readers", fuse_readers::fuse_readers),
    ("size_buffers", size_buffers::size_buffers),
    (
        "synthesize_handshakes",
        synthesize_handshakes::synthesize_handshakes,
    ),
    ("persist", persist::persist),
    ("verify", verify::verify),
];

/// ★ A LOWERING COMPUTES ONLY IN DATATYPES ITS CHIP HAS. Every buffer a `Generic` stage reads or writes, every
/// index an indirect tile address waits on, and every tensor moved into or out of one of those buffers must
/// be a precision the target's elementwise datapath computes in (or its fp32 accumulation width). Without
/// this an index lowered for an RDU rode `Int64` — a type the PCU does not have — and evaluated exactly,
/// because nothing asked the chip. It gates what the COMPILER chooses — every program [`super::link::link`] returns; a
/// program built by hand is its author's claim, checked by the passes as it stands.
pub fn check_datatypes(p: &SpatialProgram, target: &SpatialTarget) -> Result<(), SpatialReject> {
    let mut gated: std::collections::BTreeSet<BufferId> = Default::default();
    for s in &p.stages {
        match &s.kind {
            StageKind::Compute(ComputeOp::Generic { .. }) => {
                gated.extend(s.ins.iter().chain(&s.outs).copied())
            }
            StageKind::Load { pattern, .. } | StageKind::Store { pattern, .. } => {
                gated.extend(pattern.origin.indirect.map(|i| i.index))
            }
            _ => {}
        }
    }
    let refuse = |needed: Precision| {
        Err(SpatialReject::MissingPrecision {
            needed,
            target: target.name,
        })
    };
    for b in &gated {
        let d = p.buffer(*b).dtype;
        if !target.generic_holds(d) {
            return refuse(d);
        }
    }
    for s in &p.stages {
        let moved = match &s.kind {
            StageKind::Load { tensor, .. } if s.outs.iter().any(|b| gated.contains(b)) => {
                Some(*tensor)
            }
            StageKind::Store { tensor, .. } if s.ins.first().is_some_and(|b| gated.contains(b)) => {
                Some(*tensor)
            }
            _ => None,
        };
        if let Some(t) = moved {
            let d = p.tensor(t).dtype;
            if !target.generic_holds(d) {
                return refuse(d);
            }
        }
    }
    Ok(())
}

/// Run the pinned passes to a verified program.
pub fn run_passes(
    p: SpatialProgram,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    run_pass_list(p, target, PASS_ORDER)
}

/// The passes through `tile`: a template fused and planned, nothing placed — where two planned sections are joined
/// ([`super::passes::forward`]) before the rest run on the joined one.
pub fn run_passes_through_tile(p: SpatialProgram, target: &SpatialTarget) -> Result<SpatialProgram, SpatialReject> {
    let at = PASS_ORDER.iter().position(|(n, _)| *n == "tile").expect("a tiling pass");
    run_pass_list(p, target, &PASS_ORDER[..=at])
}

/// The passes after `tile`, ending with the certifying one or before it, as [`run_passes`] and
/// [`run_passes_uncertified`] end.
fn run_passes_after_tile(p: SpatialProgram, target: &SpatialTarget, certify: bool) -> Result<SpatialProgram, SpatialReject> {
    let at = PASS_ORDER.iter().position(|(n, _)| *n == "tile").expect("a tiling pass");
    let end = if certify { PASS_ORDER.len() } else { PASS_ORDER.len() - 1 };
    run_pass_list(p, target, &PASS_ORDER[at + 1..end])
}

/// Every pass but the last, which certifies: the program tiled, placed, sized and scheduled — a SECTION of a
/// composition, which is certified as part of the composed program (`compose` drops a section's own certificate).
pub fn run_passes_uncertified(
    p: SpatialProgram,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let (last, before) = PASS_ORDER.split_last().expect("passes");
    assert_eq!(last.0, "verify", "the certifying pass is the last one");
    run_pass_list(p, target, before)
}

/// `passes` over `p` in order, each result validated — through the thread's compile cache
/// ([`super::compile_cache`]), which holds what a list made of a template an earlier compile handed it.
fn run_pass_list(
    p: SpatialProgram,
    target: &SpatialTarget,
    passes: &[(&str, Pass)],
) -> Result<SpatialProgram, SpatialReject> {
    let names: Vec<&str> = passes.iter().map(|(name, _)| *name).collect();
    super::compile_cache::passed(p, target, &names, |p| {
        let mut cur = p;
        let tr = std::env::var("LOGOS_PASS_TIME").is_ok() && cur.name.contains("gd_");
        for (name, pass) in passes {
            let t0 = std::time::Instant::now();
            cur = pass(&cur, target)?;
            let dt = t0.elapsed();
            cur.validate()?;
            if tr {
                eprintln!(
                    "TIME {} {name}: {:?} stages={} buffers={} edges={} items={}",
                    cur.name,
                    dt,
                    cur.stages.len(),
                    cur.buffers.len(),
                    cur.edges.len(),
                    cur.work.items.len()
                );
            }
        }
        Ok(cur)
    })
}

/// The source-level names and recognized fills of ONE lifted statement — the binder, then every operand
/// in [`TensorOpSpec::ins`] order. A fill is [`Fill::Unknown`] for a tensor the program never declares,
/// which is exactly the activations a composition produces and consumes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageNames {
    pub out: String,
    pub ins: Vec<String>,
    pub fills: Vec<Fill>,
}

impl StageNames {
    /// The name and fill of operand `i`.
    pub fn operand(&self, i: usize) -> (&str, &Fill) {
        (self.ins[i].as_str(), &self.fills[i])
    }
}

/// The one op a single-op source is, as the graph of that op over the operands the source declares.
fn recognize_any(source: &str) -> Result<TensorGraph, SpatialReject> {
    crate::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _, _) = parsed.map_err(|e| SpatialReject::Parse(format!("{e:?}")))?;
        if stmts.iter().any(crate::tensor_recognize::holds_an_op_by_hand) {
            return Err(SpatialReject::NotLowered(
                "an op inside a loop or branch the program wrote — the block runs it once a turn or on one arm, so it is not the op",
            ));
        }
        let operand = |s, len: usize| (s, len, recognize_fill(stmts, s));
        let matmul = recognize_matmul(stmts, interner);
        if let Ok(spec) = matmul {
            let ins = vec![operand(spec.a, spec.m * spec.k), operand(spec.b, spec.k * spec.n)];
            let out = spec.c;
            return Ok(TensorGraph::of_op(TensorOpSpec::Matmul(spec), ins, out, interner));
        }
        let elementwise = recognize_elementwise(stmts, interner);
        if let Ok(spec) = elementwise {
            let ins = vec![operand(spec.a, spec.n), operand(spec.b, spec.n)];
            let out = spec.c;
            return Ok(TensorGraph::of_op(TensorOpSpec::Elementwise(spec), ins, out, interner));
        }
        if let Ok(spec) = recognize_swiglu(stmts, interner) {
            let ins = vec![operand(spec.gate, spec.total), operand(spec.up, spec.total)];
            let out = spec.y;
            return Ok(TensorGraph::of_op(TensorOpSpec::SwiGlu(spec), ins, out, interner));
        }
        let reduction = recognize_fused_row(stmts, interner);
        if let Ok(spec) = reduction {
            let (rows, cols) = (spec.rows, spec.cols);
            let ins = std::iter::once(operand(spec.x, rows * cols))
                .chain(spec.weight.map(|s| operand(s, spec.weight_rows * cols)))
                .chain(spec.residual.map(|s| operand(s, rows * cols)))
                .chain(spec.bias.map(|s| operand(s, cols)))
                .collect();
            let out = spec.y;
            return Ok(TensorGraph::of_op(TensorOpSpec::FusedRow(spec), ins, out, interner));
        }
        Err(SpatialReject::Unrecognized {
            matmul: matmul.unwrap_err(),
            elementwise: elementwise.unwrap_err(),
            reduction: reduction.unwrap_err(),
        })
    })
}

/// English → a template (phase `Template`) with the given numerics policy.
pub fn template_from_source_with(
    source: &str,
    target: &SpatialTarget,
    numerics: NumericsPolicy,
) -> Result<SpatialProgram, SpatialReject> {
    let mut programs = link(&recognize_any(source)?, 0, &Prelude::default(), Form::Alone, numerics, target)?;
    if programs.len() != 1 {
        return Err(SpatialReject::NotLowered(
            "a multi-section op as a single template — this shape only exists inside a composition",
        ));
    }
    Ok(programs.remove(0))
}

pub fn template_from_source(
    source: &str,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    template_from_source_with(source, target, NumericsPolicy::Exact)
}

/// A template from a matmul spec directly (external data with declared ranges).
pub fn template_from_spec(
    spec: &MatmulSpec,
    names: &MatmulNames,
    fills: (&Fill, &Fill),
    ranges: (Option<(i64, i64)>, Option<(i64, i64)>),
    numerics: NumericsPolicy,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    from_matmul(spec, names, fills, ranges, numerics, target)
}

/// English → a verified spatial program.
pub fn compile_to_spatial_with(
    source: &str,
    target: &SpatialTarget,
    numerics: NumericsPolicy,
) -> Result<SpatialProgram, SpatialReject> {
    run_passes(template_from_source_with(source, target, numerics)?, target)
}

pub fn compile_to_spatial(
    source: &str,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    compile_to_spatial_with(source, target, NumericsPolicy::Exact)
}

// ── The whole-program front end ───────────────────────────────────────────────────────────────────
//
// ★ A PARTIAL LOWERING IS NEVER A SUCCESS. `template_from_source` lifts the FIRST op it recognizes and
// lowers it AS IF IT WERE THE PROGRAM — which on a hundred-statement transformer forward means emitting
// one q-projection, with a certificate, and exiting zero. That is the single failure this whole front end
// exists to make impossible: [`ProgramLowering`] carries the counts ALWAYS, and hands back a program only
// when every statement the walk saw became a section. A caller that wants an artifact must ask
// [`ProgramLowering::whole`], and the refusal it gets otherwise says how many of how many.

/// One statement the whole-program walk saw and could not turn into a section, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingStage {
    /// Where the statement sits in the source's top-level statement list.
    pub stmt_index: usize,
    /// The tensor it binds (or the loop/branch it is, when no recognizer lifted it).
    pub binder: String,
    /// The recognized op kind, or `unrecognized`.
    pub kind: String,
    /// The refusal, in the front end's own words.
    pub why: String,
}

/// What the whole-program front end made of a source.
///
/// `statements` is every top-level statement the walk considered a tensor op — the recognized ones plus
/// the ones no recognizer lifted. `lowered` is how many became sections. `program` is `Some` only when
/// those two agree AND the composer joined the sections, so `program.is_some()` IS the honest verdict.
#[derive(Clone, Debug)]
pub struct ProgramLowering {
    pub program: Option<SpatialProgram>,
    pub statements: usize,
    pub lowered: usize,
    pub sections: usize,
    pub missing: Vec<MissingStage>,
    /// The state the walk found, by tensor name (first-appearance order) — every table a section updates in place.
    pub state: Vec<String>,
    /// The part of [`Self::state`] a step carries to the next: without the tables the step assembles whole
    /// ([`carried_state`](super::passes::recognize_stages::carried_state)).
    pub carried: Vec<String>,
}

impl ProgramLowering {
    /// The composed program, or `None` when the lowering is partial — there is deliberately no way to
    /// reach a program without passing this check.
    pub fn whole(&self) -> Option<&SpatialProgram> {
        self.program
            .as_ref()
            .filter(|_| self.lowered == self.statements)
    }
    /// [`Self::whole`], by value: the composed program, or the [`Self::refusal`] a partial lowering earns.
    pub fn into_whole(self) -> Result<SpatialProgram, String> {
        if self.whole().is_none() {
            return Err(self.refusal());
        }
        Ok(self.program.expect("a whole lowering carries its program"))
    }
    /// The refusal a partial lowering earns: the fraction, then what is missing, by name.
    pub fn refusal(&self) -> String {
        let mut why: Vec<String> = self
            .missing
            .iter()
            .take(MISSING_NAMED)
            .map(|m| {
                format!(
                    "statement {} (`{}`, {}): {}",
                    m.stmt_index, m.binder, m.kind, m.why
                )
            })
            .collect();
        if self.missing.len() > MISSING_NAMED {
            why.push(format!("… and {} more", self.missing.len() - MISSING_NAMED));
        }
        if self.missing.is_empty() && self.program.is_none() {
            return format!(
                "lifted {} of {} statements into {} sections, and the composer did not join them into one program",
                self.lowered, self.statements, self.sections
            );
        }
        format!(
            "lowered {} of {} statements; not lowered: {}",
            self.lowered,
            self.statements,
            why.join("; ")
        )
    }
}

/// How many missing statements a refusal names before it counts the rest.
const MISSING_NAMED: usize = 6;

/// English → the WHOLE tensor program as sections of one spatial program.
///
/// `Err` only when there is no tensor program to speak of (the source does not parse, or no recognizer
/// sees an op anywhere in it). Everything else comes back as a [`ProgramLowering`] whose counts are the
/// truth, whether or not it carries a program.
pub fn compile_program_to_spatial(
    source: &str,
    target: &SpatialTarget,
) -> Result<ProgramLowering, SpatialReject> {
    compile_program_to_spatial_agreeing(source, target, &std::collections::BTreeSet::new())
}

/// [`compile_program_to_spatial`] for one program of a DEPLOYMENT: `dense` names the tables the deployment's other
/// programs lay out as matrices ([`dense_tables_of`]), so a table one program writes and another reads is laid out
/// alike in both — the agreement `dense_tables` makes inside a program, made across the programs that share it.
pub fn compile_program_to_spatial_agreeing(
    source: &str,
    target: &SpatialTarget,
    dense: &std::collections::BTreeSet<String>,
) -> Result<ProgramLowering, SpatialReject> {
    lower_program(
        source,
        target,
        dense,
        None,
        &std::collections::BTreeSet::new(),
        &Results::EveryNamed,
    )
}

/// [`compile_program_to_spatial`] for a program whose RESULTS are declared: `results` names what outside the program
/// reads — its host, or the next program of a deployment — besides what it shows and the state it carries, and every
/// other tensor only the next statement reads is forwarded ([`super::passes::forward`]).
pub fn compile_program_to_spatial_returning(
    source: &str,
    target: &SpatialTarget,
    results: &std::collections::BTreeSet<String>,
) -> Result<ProgramLowering, SpatialReject> {
    let none = std::collections::BTreeSet::new();
    lower_program(source, target, &none, None, &none, &Results::Only(results.clone()))
}

/// [`compile_program_to_spatial`] for a program whose weights `q8` the deployment STORES Q8_0: a one-row product over
/// one of them reads its int8 values and per-block scales as they are stored, rather than a bf16 copy
/// ([`super::passes::from_q8`]). A product the fold cannot tile reads the weight dense.
pub fn compile_program_to_spatial_stored(
    source: &str,
    target: &SpatialTarget,
    q8: &std::collections::BTreeSet<String>,
) -> Result<ProgramLowering, SpatialReject> {
    lower_program(source, target, &std::collections::BTreeSet::new(), None, q8, &Results::EveryNamed)
}

/// [`compile_program_to_spatial_agreeing`] for a deployment whose weights `q8` are stored Q8_0.
pub fn compile_program_to_spatial_agreeing_stored(
    source: &str,
    target: &SpatialTarget,
    dense: &std::collections::BTreeSet<String>,
    q8: &std::collections::BTreeSet<String>,
) -> Result<ProgramLowering, SpatialReject> {
    lower_program(source, target, dense, None, q8, &Results::EveryNamed)
}

/// [`compile_program_to_spatial`] for a TENSOR-PARALLEL program (see [`super::tensor_parallel`]): every section runs
/// on the chip `manifest` gives what it computes, and each collective's fold statements are one all-reduce section.
pub fn compile_program_to_spatial_tp(
    source: &str,
    target: &SpatialTarget,
    manifest: &super::tensor_parallel::TpManifest,
) -> Result<ProgramLowering, SpatialReject> {
    compile_program_to_spatial_tp_agreeing(
        source,
        target,
        manifest,
        &std::collections::BTreeSet::new(),
    )
}

/// [`compile_program_to_spatial_tp`] for one program of a deployment, agreeing with its other programs on the tables
/// laid out dense (see [`compile_program_to_spatial_agreeing`]).
pub fn compile_program_to_spatial_tp_agreeing(
    source: &str,
    target: &SpatialTarget,
    manifest: &super::tensor_parallel::TpManifest,
    dense: &std::collections::BTreeSet<String>,
) -> Result<ProgramLowering, SpatialReject> {
    compile_program_to_spatial_tp_agreeing_stored(
        source,
        target,
        manifest,
        dense,
        &std::collections::BTreeSet::new(),
    )
}

/// [`compile_program_to_spatial_tp_agreeing`] for a deployment whose chip-local weight slices `q8` are stored Q8_0 — each
/// chip's slice read as its int8 values and scales ([`super::passes::from_q8`]).
pub fn compile_program_to_spatial_tp_agreeing_stored(
    source: &str,
    target: &SpatialTarget,
    manifest: &super::tensor_parallel::TpManifest,
    dense: &std::collections::BTreeSet<String>,
    q8: &std::collections::BTreeSet<String>,
) -> Result<ProgramLowering, SpatialReject> {
    lower_program(source, target, dense, Some(manifest), q8, &Results::EveryNamed)
}

/// [`compile_program_to_spatial_agreeing_stored`] — or, given a tensor-parallel `manifest`,
/// [`compile_program_to_spatial_tp_agreeing_stored`] — and what the program was made from: every lifted op, the shared
/// optimizer's decisions over them on this chip and, when the lowering is whole, the ops each section of the program
/// holds ([`super::lineage::Lineage`]).
pub fn compile_program_to_spatial_traced(
    source: &str,
    target: &SpatialTarget,
    dense: &std::collections::BTreeSet<String>,
    manifest: Option<&super::tensor_parallel::TpManifest>,
    q8: &std::collections::BTreeSet<String>,
) -> Result<(ProgramLowering, super::lineage::Lineage), SpatialReject> {
    lower_program_on(source, target, dense, manifest, q8, None, &Results::EveryNamed)
}

/// Where a composed program's sections are certified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionCertification {
    /// Once, as the composed program: each section is only scheduled, and the composition's certificate — the one that
    /// ships — covers it.
    Composed,
    /// Each section on its own as it is lowered, and then the composition: the path that names every statement whose
    /// section a certificate refuses.
    EachSection,
}

/// ★ CERTIFY ONCE, WHERE IT SHIPS. A section's own certificate never leaves the pipeline — `compose` drops it and the
/// composed program earns its own over the same stages — so certifying each section as it is lowered proves every
/// section twice. The composed certification comes first, and it is the answer whenever the program certifies whole:
/// the same program and the same certificate, since nothing reads a section's. A program that does not certify whole
/// is lowered again certifying each section, so the answer is exactly the one that names what refused.
fn lower_program(
    source: &str,
    target: &SpatialTarget,
    dense: &std::collections::BTreeSet<String>,
    tp: Option<&super::tensor_parallel::TpManifest>,
    q8: &std::collections::BTreeSet<String>,
    results: &Results,
) -> Result<ProgramLowering, SpatialReject> {
    lower_program_on(source, target, dense, tp, q8, None, results).map(|(lowering, _)| lowering)
}

/// [`lower_program`] of the statements `chip` names alone, when it does: composed, and certified each section at a time
/// where the composition does not certify — with the program's lineage.
fn lower_program_on(
    source: &str,
    target: &SpatialTarget,
    dense: &std::collections::BTreeSet<String>,
    tp: Option<&super::tensor_parallel::TpManifest>,
    q8: &std::collections::BTreeSet<String>,
    chip: Option<(u16, &super::tensor_parallel::TpManifest)>,
    results: &Results,
) -> Result<(ProgramLowering, super::lineage::Lineage), SpatialReject> {
    let composed = lower_program_stored(
        source,
        target,
        dense,
        tp,
        SectionCertification::Composed,
        q8,
        chip,
        results,
    )?;
    if composed.0.whole().is_some() {
        return Ok(composed);
    }
    lower_program_stored(
        source,
        target,
        dense,
        tp,
        SectionCertification::EachSection,
        q8,
        chip,
        results,
    )
}

/// A tensor-parallel program lowered for chips that share no fabric: one ordinary program a chip, and the tensors that
/// cross between them ([`compile_program_to_chips`]).
#[derive(Clone, Debug)]
pub struct ChipLowering {
    /// Chip `c`'s program: the statements `manifest` runs on chip `c`, lowered and composed on one site as any one-chip
    /// program is. A tensor another chip writes is an input, holding the interval the whole program proves for it.
    pub chips: Vec<ProgramLowering>,
    pub transfers: Vec<super::tensor_parallel::ChipTransfer>,
}

/// ★ A CHIP BOUNDARY IS A TRANSFER, NOT A SHARED MEMORY. [`compile_program_to_spatial_tp`] lowers a tensor-parallel
/// program as ONE program over its chips, every section followed by a barrier the chips all join — the shape of a
/// machine whose chips share one fabric. Boards joined by a link share nothing: each runs its own program, and a tensor
/// one board writes and the other reads crosses the link between them. So each chip's statements are lowered alone, in
/// order, as an ordinary one-chip program, every tensor narrowed by the interval the WHOLE program proves for it, and
/// what crosses is listed ([`super::tensor_parallel::transfers`]). An all-reduce is then what it is over a link: the
/// partials cross to the chip that folds them, and the fold is that chip's own addition.
pub fn compile_program_to_chips(
    source: &str,
    target: &SpatialTarget,
    manifest: &super::tensor_parallel::TpManifest,
) -> Result<ChipLowering, SpatialReject> {
    lower_chips(source, target, manifest, &Results::EveryNamed)
}

/// [`compile_program_to_chips`] for a program whose results are declared (see
/// [`compile_program_to_spatial_returning`]): on each chip, a tensor only the next statement reads — no statement of
/// any chip, no `Show`, no carried state, nothing `results` names — is forwarded.
pub fn compile_program_to_chips_returning(
    source: &str,
    target: &SpatialTarget,
    manifest: &super::tensor_parallel::TpManifest,
    results: &std::collections::BTreeSet<String>,
) -> Result<ChipLowering, SpatialReject> {
    lower_chips(source, target, manifest, &Results::Only(results.clone()))
}

fn lower_chips(
    source: &str,
    target: &SpatialTarget,
    manifest: &super::tensor_parallel::TpManifest,
    results: &Results,
) -> Result<ChipLowering, SpatialReject> {
    let rp = TensorGraph::lift_source(source, &|_| Storage::Values)?.program;
    let transfers = super::tensor_parallel::transfers(&rp.stages, manifest);
    let none = std::collections::BTreeSet::new();
    let mut chips = (0..manifest.chips)
        .map(|c| lower_program_on(source, target, &none, None, &none, Some((c as u16, manifest)), results).map(|(lowering, _)| lowering))
        .collect::<Result<Vec<_>, _>>()?;
    for (c, pl) in chips.iter_mut().enumerate() {
        if let Some(p) = pl.program.as_mut() {
            match super::tensor_parallel::link_points(p, c as u16, &transfers) {
                Ok(links) => p.links = links,
                Err(e) => {
                    pl.missing.push(MissingStage {
                        stmt_index: 0,
                        binder: format!("chip {c}"),
                        kind: "link".into(),
                        why: format!("{e:?}"),
                    });
                    pl.program = None;
                }
            }
        }
    }
    Ok(ChipLowering { chips, transfers })
}

/// The whole tensor program lowered with its sections certified as `sections` says — `lower_program`'s two paths,
/// public so a lock can hold them to one answer.
pub fn lower_program_with(
    source: &str,
    target: &SpatialTarget,
    dense: &std::collections::BTreeSet<String>,
    tp: Option<&super::tensor_parallel::TpManifest>,
    sections: SectionCertification,
) -> Result<ProgramLowering, SpatialReject> {
    lower_program_stored(
        source,
        target,
        dense,
        tp,
        sections,
        &std::collections::BTreeSet::new(),
        None,
        &Results::EveryNamed,
    )
    .map(|(lowering, _)| lowering)
}

/// ★ ONE VIEW OF A VALUE, HOWEVER MANY PRODUCTS READ IT. A Q8_0 product reads its activation through the `K/32 × 32` view
/// its own lowering reshapes, named for the activation's value ([`super::passes::from_q8::q8_rows_name`]); the
/// projections of one normalized row (q, k, v, the gates) all read the same value, so the reshape is kept only the
/// first time — `made` holds the views already made.
fn share_q8_views(
    sections: Vec<SpatialProgram>,
    view: Option<&String>,
    made: &mut std::collections::BTreeSet<String>,
) -> Vec<SpatialProgram> {
    let Some(view) = view else {
        return sections;
    };
    let makes_view = |p: &SpatialProgram| {
        p.tensors
            .iter()
            .any(|t| t.name == *view && t.role != TensorRole::Input)
    };
    if !sections.iter().any(makes_view) || made.insert(view.clone()) {
        return sections;
    }
    sections.into_iter().filter(|p| !makes_view(p)).collect()
}

/// The lifted ops whose work one section holds ([`super::lineage::Lineage`]): none for a layout bridge.
type Origin = std::collections::BTreeSet<usize>;

/// `next`'s sections after `acc`'s, where the last of `acc` and the first of `next` joined into one section: the joined
/// section holds both's ops.
fn join_origins(acc: &mut Vec<Origin>, next: Vec<Origin>) {
    let mut next = next.into_iter();
    match (acc.last_mut(), next.next()) {
        (Some(last), Some(first)) => last.extend(first),
        (None, Some(first)) => acc.push(first),
        _ => {}
    }
    acc.extend(next);
}

/// The ops each of an op's agreed sections holds ([`Agreed`]): its own sections `own`'s, a layout bridge none.
fn agreed_origins(agreed: &Agreed, own: Vec<Origin>) -> Vec<Origin> {
    let bridges = |n: usize| std::iter::repeat_with(Origin::new).take(n);
    bridges(agreed.before.len()).chain(own).chain(bridges(agreed.after.len())).collect()
}

/// One op's section templates awaiting their passes, the lifted ops it computes, and what names it where it is refused.
struct Held {
    at: usize,
    stmt_index: usize,
    out: String,
    kind: String,
    covers: Origin,
    templates: Vec<SpatialProgram>,
}

/// Statements whose templates joined ([`super::passes::forward`]): their templates as joined, and the lifted ops each
/// holds.
struct Run {
    held: Vec<Held>,
    joined: Vec<SpatialProgram>,
    origins: Vec<Origin>,
}

/// A statement's place in the lowering, in program order: a run of joined statements, or sections already made (an
/// all-reduce, with the ops each holds) or none (a statement that did not lower).
enum Piece {
    Run(Run),
    Done(Vec<SpatialProgram>, Vec<Origin>),
}

/// ★ WHAT OUTSIDE A PROGRAM READS ITS TENSORS, the program's caller says. Its host may read any tensor it is handed, and a
/// program that is one of a deployment's has readers in the next program that its own statements never show. So a
/// tensor only the next statement reads is forwarded ([`super::passes::forward`]) only where the caller has declared
/// what else is read; otherwise every tensor a statement names is stored, as the program wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Results {
    /// Any tensor a statement names may be read: each is stored, and nothing is forwarded.
    EveryNamed,
    /// What these name is read, besides what the program shows and the state it carries: a tensor nothing else reads
    /// is forwarded.
    Only(std::collections::BTreeSet<String>),
}

/// `source` lifted, its weights `q8` read as the deployment stores them (Q8_0), and the shared optimizer's rewrite of it
/// under `target`'s spatial shim — under a tensor-parallel deployment's `manifest`, the shim that keeps every fusion
/// within one chip ([`super::tensor_parallel::TensorParallelShim`]). The lowering links the ops the rewrite leaves, and
/// the rewrite's report is every fusion decided over the program ([`super::lineage::Lineage::fusion`]).
pub fn optimized_on(
    source: &str,
    target: &SpatialTarget,
    q8: &std::collections::BTreeSet<String>,
    manifest: Option<&super::tensor_parallel::TpManifest>,
) -> Result<(Lifted, crate::tensor_graph::fuse::Optimized), SpatialReject> {
    let storage = |t: &str| {
        if q8.contains(t) {
            Storage::Rows { format: crate::model::quant::GgmlType::Q8_0, transposed: true }
        } else {
            Storage::Values
        }
    };
    let lifted = TensorGraph::lift_source(source, &storage)?;
    let spatial = super::shim::SpatialShim { target };
    let optimized = match manifest {
        Some(manifest) => {
            crate::tensor_graph::fuse::optimize(&lifted.graph, &super::tensor_parallel::TensorParallelShim { base: &spatial, manifest }, &[])
        }
        None => crate::tensor_graph::fuse::optimize(&lifted.graph, &spatial, &[]),
    };
    Ok((lifted, optimized))
}

/// [`lower_program_with`] with the weights `q8` the deployment stores Q8_0, and the results `results` its caller reads —
/// and the program's lineage.
#[allow(clippy::too_many_arguments)]
fn lower_program_stored(
    source: &str,
    target: &SpatialTarget,
    dense: &std::collections::BTreeSet<String>,
    tp: Option<&super::tensor_parallel::TpManifest>,
    sections: SectionCertification,
    q8: &std::collections::BTreeSet<String>,
    chip: Option<(u16, &super::tensor_parallel::TpManifest)>,
    results: &Results,
) -> Result<(ProgramLowering, super::lineage::Lineage), SpatialReject> {
    use super::tensor_parallel::{allreduce_section, relocate};
    let certification = sections;
    let passes = |t: SpatialProgram| match certification {
        SectionCertification::Composed => run_passes_uncertified(t, target),
        SectionCertification::EachSection => run_passes(t, target),
    };
    // The ops the shared optimizer leaves this chip, in order: under its shim, each a statement the front end lifted.
    let (Lifted { program: whole, .. }, optimized) = optimized_on(source, target, q8, tp.or(chip.map(|(_, m)| m)))?;
    let crate::tensor_graph::fuse::Optimized { graph: g, report: fusion, .. } = optimized;
    // Each lifted op as the lineage names it: the statement, its binder and its op.
    let lifted_ops: Vec<super::lineage::LiftedOp> = whole
        .stages
        .iter()
        .map(|st| super::lineage::LiftedOp { stmt_index: st.stmt_index, binder: st.out.clone(), op: st.op.kind().to_string() })
        .collect();
    // The statement each op lifts, by which a refusal names it.
    let stmts: Vec<usize> = g.ops.iter().map(|op| whole.stages[op.covers[0]].stmt_index).collect();
    let stmt_of = |k: usize| stmts[k];
    // The statement each lifted op is, by which a region's member names its own.
    let lifted_stmts: Vec<usize> = whole.stages.iter().map(|st| st.stmt_index).collect();
    // The intervals and the float tensors are the whole program's: a tensor another chip writes holds on this chip what
    // it holds there.
    let float_out = float_tensors(&whole);
    let ranges = super::value_range::value_ranges(&whole, &float_out);
    let rp = match chip {
        Some((c, m)) => super::passes::recognize_stages::RecognizedProgram {
            stages: whole.stages.iter().filter(|st| m.chip_running(&st.out) == c).cloned().collect(),
            ..whole.clone()
        },
        None => whole,
    };
    if rp.stages.is_empty() {
        // Nothing lifted at all: hand back the precise three-way reject the single-op path computes, so
        // a front-end regression is never reported as an empty program.
        return Err(recognize_any(source)
            .err()
            .unwrap_or(SpatialReject::NotLowered(
                "a source with no tensor statements",
            )));
    }
    let statements = rp.stages.len() + rp.unrecognized.len() + rp.unlifted_writes.len();
    let mut missing: Vec<MissingStage> = rp
        .unrecognized
        .iter()
        .map(|(i, b)| MissingStage {
            stmt_index: *i,
            binder: b.clone(),
            kind: "unrecognized".into(),
            why: "no recognizer of the whole-program walk lifts this statement".into(),
        })
        .chain(rp.unlifted_writes.iter().map(|(i, t, why)| MissingStage {
            stmt_index: *i,
            binder: t.clone(),
            kind: "write".into(),
            why: (*why).to_string(),
        }))
        .collect();
    // The intervals each section's storage is narrowed by: the front end's, then what each lowered section proves it
    // stores into a tensor the front end never saw.
    let mut known = ranges.clone();
    let ours = |op: &Op| chip.is_none_or(|(c, m)| m.chip_running(&op.out) == c);
    let linked: Vec<usize> = (0..g.ops.len()).filter(|&k| ours(&g.ops[k])).collect();
    let prelude = Prelude::of(&g, &linked, target, dense, ranges.clone(), q8, &|out| tp.and_then(|m| m.chip_of.get(out).copied()));
    let mut twins = std::collections::BTreeSet::new();
    let mut q8_views = std::collections::BTreeSet::new();
    // A collective's links so far, by its result: its one section computes every link of its fold.
    let mut folds: std::collections::BTreeMap<String, Origin> = std::collections::BTreeMap::new();
    let mut lowered = 0usize;
    // ★ A RESULT ONLY THE NEXT STATEMENT READS IS FORWARDED ([`super::passes::forward`]), wherever the caller has
    // declared the program's results ([`Results`]) and the tensor between two statements is read by nothing else — no
    // later statement, no `Show`, no carried state, nothing the caller reads — and its store and load agree. Two levels:
    // - statements whose templates join make a RUN, tiled, placed and scheduled as one section (a map folds into the
    //   map before it);
    // - two neighbouring runs whose planned sections join — a matmul's items meet a row map's one item only once `tile`
    //   has grouped them — are placed and scheduled as one.
    // Whatever the passes refuse joined is lowered as it was before it was joined: a planned join as its two runs, a run
    // as its statements, each as it would have been. A tensor-parallel program relocates each statement's sections to
    // its chips and forwards nothing.
    let carried: std::collections::BTreeSet<String> = state_tensors(&rp.stages).into_iter().collect();
    let touches = g.touches();
    let forwards = |t: &str, writer: usize, reader: usize| {
        let read = match results {
            Results::EveryNamed => return false,
            Results::Only(read) => read,
        };
        tp.is_none()
            && !read.contains(t)
            && !carried.contains(t)
            && g.ops[reader].ins.iter().filter(|x| *x == t).count() == 1
            && touches.read_only_by(t, writer, reader)
    };
    let certify = matches!(certification, SectionCertification::EachSection);
    // A join hands the value to ONE section's compute: a tensor another section of either statement names stays stored.
    let names = |ps: &[SpatialProgram], t: &str| ps.iter().any(|p| p.tensors.iter().any(|x| x.name == t));
    // ★ A JOIN NEVER WIDENS WHAT A VALUE IS PROVEN TO HOLD. A load of the tensor reads it within the interval the front
    // end proves for it; the buffer that carries it once joined holds what the producer's compute proves. Where the
    // front end knows more than any analysis of that compute can see — an attention's context, a weighted mean of its
    // values, computed by a division — the join would hand the next compute a wider value than its load did, and every
    // width sized from it after.
    let keeps = |j: &super::passes::forward::Joint, t: &str, known: &std::collections::BTreeMap<String, super::value_range::Interval>| {
        let Some(&bound) = known.get(t) else { return true };
        let proven = super::value_range::program_ranges(&declare_ranges(j.program.clone(), known));
        proven.buffers[j.carrier.idx()].is_some_and(|iv| bound.0 <= iv.0 && iv.1 <= bound.1)
    };
    // ★ ONE DECISION JOINS TWO SECTIONS, at either level: `a`, the producer's last section (`before` its others), and
    // `b`, the consumer's first (`after` its others), through `t` — where nothing else reads `t`, no other section of
    // either names it, `forward` joins them, and the carrier keeps what `t` is proven to hold.
    let join = |t: &str,
                writer: usize,
                reader: usize,
                (before, a): (&[SpatialProgram], &SpatialProgram),
                (b, after): (&SpatialProgram, &[SpatialProgram]),
                known: &std::collections::BTreeMap<String, super::value_range::Interval>| {
        (forwards(t, writer, reader) && !names(before, t) && !names(after, t))
            .then(|| super::passes::forward::forward(a, b, t))
            .flatten()
            .filter(|j| keeps(j, t, known))
    };
    let mut pieces: Vec<Piece> = Vec::new();
    for &k in &linked {
        let op = &g.ops[k];
        // ★ A COLLECTIVE IS ONE SECTION, NOT ITS FOLD. The source spells an all-reduce as the chain of additions it
        // means, so the VM can check the decomposition; on the chips it is the partials folded where they live. The
        // chain's earlier links compute nothing on their own, and its last is where the all-reduce stands.
        if let Some(c) = tp.and_then(|m| m.collective_folding(&op.out)) {
            folds.entry(c.out.clone()).or_default().extend(op.covers.iter().copied());
            if op.out == c.out {
                let n = match op.prim() {
                    Some(TensorOpSpec::Elementwise(e)) => e.n,
                    _ => 0,
                };
                let floats = float_out.contains(&c.out) || c.partials.iter().any(|x| float_out.contains(x));
                let mut narrow = |t: SpatialProgram| narrow_storage(t, &mut known, target);
                match allreduce_section(c, g.shape_of(&c.out, n), floats, target, certification, &mut narrow) {
                    Ok(p) => pieces.push(Piece::Done(vec![p], vec![folds.remove(&c.out).unwrap_or_default()])),
                    Err(e) => {
                        pieces.push(Piece::Done(Vec::new(), Vec::new()));
                        missing.push(MissingStage {
                            stmt_index: stmt_of(k),
                            binder: op.out.clone(),
                            kind: "AllReduce".into(),
                            why: format!("{e:?}"),
                        })
                    }
                }
            } else {
                pieces.push(Piece::Done(Vec::new(), Vec::new()));
            }
            lowered += 1;
            continue;
        }
        // The float flag a section is lowered with: its result's kind — except an arg-max, whose result is
        // always an index and whose datapath is its INPUT's.
        let floats = match op.prim() {
            Some(TensorOpSpec::ArgMax(_)) => float_out.contains(&op.ins[0]),
            _ => float_out.contains(&op.out),
        };
        // A region is one section. Each member is linked alone, as a statement of its own would be, and the members
        // are joined from the region's result back, each value handed from the member that makes it to the one that
        // reads it ([`super::passes::forward`]) — the shim admits only a region whose every value is read once within it
        // and none written out. A member that lowers to more than one section, or a join the passes refuse, leaves the
        // members' sections apart, each storing what it makes.
        let made = match &op.sym {
            crate::tensor_graph::graph::Sym::Fused(f) if f.class == crate::tensor_graph::symbols::ClassId::ElementwiseRegions => {
                let mut held: Vec<Held> = Vec::new();
                let mut alone: Vec<SpatialProgram> = Vec::new();
                let mut refused = None;
                // One graph of the member alone, its tensors the program's, taken once for the region.
                let mut one = TensorGraph { identity: g.identity.clone(), io: g.io.clone(), tensors: g.tensors.clone(), ops: Vec::new() };
                for m in &f.parts {
                    one.ops = vec![m.clone()];
                    match link(&one, 0, &prelude, Form::Section { floats: float_out.contains(&m.out) }, NumericsPolicy::Exact, target) {
                        Ok(ts) => {
                            alone.extend(ts.iter().cloned());
                            let covers = m.covers.iter().copied().collect();
                            held.push(Held { at: k, stmt_index: lifted_stmts[m.covers[0]], out: m.out.clone(), kind: m.kind().to_string(), covers, templates: ts });
                        }
                        Err(e) => {
                            refused = Some(e);
                            break;
                        }
                    }
                }
                match refused {
                    Some(e) => Err(e),
                    None => {
                        let single = held.iter().all(|h| h.templates.len() == 1) && !matches!(results, Results::EveryNamed) && tp.is_none();
                        let mut joined = single.then(|| held.last().expect("a region has members").templates[0].clone());
                        for h in held.iter().rev().skip(1) {
                            joined = joined.and_then(|acc| {
                                super::passes::forward::forward(&h.templates[0], &acc, &h.out).filter(|j| keeps(j, &h.out, &known)).map(|j| j.program)
                            });
                        }
                        let origins: Vec<Origin> = match joined {
                            Some(_) => vec![held.iter().flat_map(|h| h.covers.iter().copied()).collect()],
                            None => held.iter().flat_map(|h| h.templates.iter().map(|_| h.covers.clone())).collect(),
                        };
                        Ok((held, joined.map_or(alone, |p| vec![p]), origins))
                    }
                }
            }
            _ => link(&g, k, &prelude, Form::Section { floats }, NumericsPolicy::Exact, target)
                .map(|ts| share_q8_views(ts, prelude.views.get(&k), &mut q8_views))
                .map(|ts| {
                    let covers: Origin = op.covers.iter().copied().collect();
                    let origins = vec![covers.clone(); ts.len()];
                    (vec![Held { at: k, stmt_index: stmt_of(k), out: op.out.clone(), kind: op.kind().to_string(), covers, templates: ts.clone() }], ts, origins)
                }),
        };
        match made {
            Ok((held, ts, made_origins)) => {
                let joint = match pieces.last() {
                    Some(Piece::Run(run)) => {
                        let last = run.held.last().expect("a run holds a statement");
                        run.joined
                            .split_last()
                            .zip(ts.split_first())
                            .and_then(|((a, before), (b, after))| join(&last.out, last.at, k, (before, a), (b, after), &known))
                    }
                    _ => None,
                };
                match (joint, pieces.last_mut()) {
                    (Some(j), Some(Piece::Run(run))) => {
                        run.joined.pop();
                        run.joined.push(j.program);
                        run.joined.extend(ts.into_iter().skip(1));
                        join_origins(&mut run.origins, made_origins);
                        run.held.extend(held);
                    }
                    _ => pieces.push(Piece::Run(Run { held, joined: ts, origins: made_origins })),
                }
            }
            Err(e) => {
                pieces.push(Piece::Done(Vec::new(), Vec::new()));
                missing.push(MissingStage {
                    stmt_index: stmt_of(k),
                    binder: op.out.clone(),
                    kind: op.kind().to_string(),
                    why: format!("{e:?}"),
                });
            }
        }
    }
    // A statement's own templates through every pass, its layouts agreed and, in a tensor-parallel program, relocated.
    let alone = |h: &Held,
                 known: &mut std::collections::BTreeMap<String, super::value_range::Interval>,
                 twins: &mut std::collections::BTreeSet<String>|
     -> Result<(Vec<SpatialProgram>, Vec<Origin>), String> {
        let ps = h
            .templates
            .iter()
            .cloned()
            .map(|t| passes(narrow_storage(t, known, target)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("{e:?}"))?;
        let own = vec![h.covers.clone(); ps.len()];
        let agreed = prelude.agree_layouts(&g, ps, twins, &passes, target).map_err(|e| format!("{e:?}"))?;
        let held = agreed_origins(&agreed, own);
        let ps = agreed.into_vec();
        match tp {
            Some(m) => ps.iter().map(|p| relocate(p, &h.out, m).map_err(|e| format!("{e:?}"))).collect::<Result<Vec<_>, _>>().map(|ps| (ps, held)),
            None => Ok((ps, held)),
        }
    };
    // A run's joined templates planned, or `None` where the passes refuse one.
    let plan_run = |run: &Run, known: &mut std::collections::BTreeMap<String, super::value_range::Interval>| -> Option<Vec<SpatialProgram>> {
        if run.held.len() == 1 {
            return None;
        }
        run.joined
            .iter()
            .cloned()
            .map(|t| run_passes_through_tile(narrow_storage(t, known, target), target))
            .collect::<Result<Vec<_>, _>>()
            .ok()
    };
    let place = |ps: Vec<SpatialProgram>, own: Vec<Origin>, twins: &mut std::collections::BTreeSet<String>| -> Option<(Vec<SpatialProgram>, Vec<Origin>)> {
        let ps = ps.into_iter().map(|p| run_passes_after_tile(p, target, certify)).collect::<Result<Vec<_>, _>>().ok()?;
        let agreed = prelude.agree_layouts(&g, ps, twins, &passes, target).ok()?;
        let held = agreed_origins(&agreed, own);
        Some((agreed.into_vec(), held))
    };
    // Sections lowered, and the lifted ops each holds, side by side.
    type Lowered = (Vec<SpatialProgram>, Vec<Origin>);
    let lower_alone = |run: &Run,
                           (sections, origins): &mut Lowered,
                           missing: &mut Vec<MissingStage>,
                           lowered: &mut usize,
                           known: &mut std::collections::BTreeMap<String, super::value_range::Interval>,
                           twins: &mut std::collections::BTreeSet<String>| {
        for h in &run.held {
            match alone(h, known, twins) {
                Ok((mut ps, mut held)) => {
                    *lowered += 1;
                    sections.append(&mut ps);
                    origins.append(&mut held);
                }
                Err(why) => missing.push(MissingStage { stmt_index: h.stmt_index, binder: h.out.clone(), kind: h.kind.clone(), why }),
            }
        }
    };
    // Each run planned — its own templates when it joined nothing — then joined to the run after it where their planned
    // sections join, and placed and scheduled.
    let mut planned: Vec<(Run, Vec<SpatialProgram>)> = Vec::new();
    let flush_planned = |planned: &mut Vec<(Run, Vec<SpatialProgram>)>,
                             out: &mut Lowered,
                             missing: &mut Vec<MissingStage>,
                             lowered: &mut usize,
                             known: &mut std::collections::BTreeMap<String, super::value_range::Interval>,
                             twins: &mut std::collections::BTreeSet<String>| {
        let chain = std::mem::take(planned);
        // The chain cut into units: neighbouring runs whose planned sections join are one unit, its sections as joined;
        // each unit is placed, and its layouts agreed, on its own, so a layout bridge is judged against the roles of the
        // statements it serves, never the whole chain's.
        let mut units: Vec<(Vec<usize>, Vec<SpatialProgram>, Vec<Origin>)> = Vec::new();
        for (k, (run, ps)) in chain.iter().enumerate() {
            let joint = match (k.checked_sub(1), units.last()) {
                (Some(p), Some((_, joined, _))) => {
                    let prev = chain[p].0.held.last().expect("a run holds a statement");
                    chain[p]
                        .1
                        .split_last()
                        .map(|(_, before)| before)
                        .zip(joined.last())
                        .zip(ps.split_first())
                        .and_then(|((before, a), (b, after))| join(&prev.out, prev.at, run.held[0].at, (before, a), (b, after), known))
                }
                _ => None,
            };
            match (joint, units.last_mut()) {
                (Some(j), Some((runs, joined, held))) => {
                    joined.pop();
                    joined.push(j.program);
                    joined.extend(ps.iter().skip(1).cloned());
                    join_origins(held, run.origins.clone());
                    runs.push(k);
                }
                _ => units.push((vec![k], ps.clone(), run.origins.clone())),
            }
        }
        for (runs, ps, held) in units {
            if runs.len() > 1 {
                if let Some((mut ps, mut held)) = place(ps, held, twins) {
                    *lowered += runs.iter().map(|k| chain[*k].0.held.len()).sum::<usize>();
                    out.0.append(&mut ps);
                    out.1.append(&mut held);
                    continue;
                }
            }
            for k in runs {
                let (run, ps) = &chain[k];
                match place(ps.clone(), run.origins.clone(), twins) {
                    Some((mut ps, mut held)) => {
                        *lowered += run.held.len();
                        out.0.append(&mut ps);
                        out.1.append(&mut held);
                    }
                    None => lower_alone(run, out, missing, lowered, known, twins),
                }
            }
        }
    };
    let mut out: Lowered = (Vec::new(), Vec::new());
    for piece in pieces {
        match piece {
            Piece::Done(mut ps, mut held) => {
                flush_planned(&mut planned, &mut out, &mut missing, &mut lowered, &mut known, &mut twins);
                out.0.append(&mut ps);
                out.1.append(&mut held);
            }
            Piece::Run(run) => {
                let ps = if tp.is_some() {
                    None
                } else {
                    plan_run(&run, &mut known).or_else(|| {
                        run.joined
                            .iter()
                            .cloned()
                            .filter(|_| run.held.len() == 1)
                            .map(|t| run_passes_through_tile(narrow_storage(t, &mut known, target), target))
                            .collect::<Result<Vec<_>, _>>()
                            .ok()
                            .filter(|ps| !ps.is_empty())
                    })
                };
                match ps {
                    Some(ps) => planned.push((run, ps)),
                    None => {
                        flush_planned(&mut planned, &mut out, &mut missing, &mut lowered, &mut known, &mut twins);
                        lower_alone(&run, &mut out, &mut missing, &mut lowered, &mut known, &mut twins);
                    }
                }
            }
        }
    }
    flush_planned(&mut planned, &mut out, &mut missing, &mut lowered, &mut known, &mut twins);
    let (sections, origins) = out;
    // ★ NO SECTION READS A RESULT NOTHING STORES. A statement's result a section loads is one some section stores; a
    // load of one nothing stores reads whatever the memory held. Every join is made to keep this; the composition is
    // refused, by the result's name, wherever one did not.
    let stored: std::collections::BTreeSet<&str> = sections
        .iter()
        .flat_map(|p| {
            p.stages.iter().filter_map(move |s| match &s.kind {
                StageKind::Store { tensor, .. } => Some(p.tensor(*tensor).name.as_str()),
                _ => None,
            })
        })
        .collect();
    for p in &sections {
        for s in &p.stages {
            let StageKind::Load { tensor, .. } = &s.kind else { continue };
            let name = &p.tensor(*tensor).name;
            if stored.contains(name.as_str()) || missing.iter().any(|m| m.binder == *name) {
                continue;
            }
            if let Some(st) = rp.stages.iter().find(|st| st.out == *name) {
                missing.push(MissingStage {
                    stmt_index: st.stmt_index,
                    binder: name.clone(),
                    kind: "read".into(),
                    why: format!("section {} loads it and no section stores it", p.name),
                });
            }
        }
    }
    missing.sort_by_key(|m| m.stmt_index);
    let state = state_tensors(&rp.stages);
    let n_sections = sections.len();
    let mut program = None;
    let mut lineage_sections: Vec<Origin> = Vec::new();
    if lowered == rp.stages.len() && missing.is_empty() {
        // A tensor-parallel program's chips run their slices at once: its sections compose phase by phase. On one chip,
        // independent sections run at once on disjoint sites (`pack`).
        let (sections, barriers, order) = match tp {
            Some(_) => {
                let (order, barriers) = super::tensor_parallel::phases(&sections);
                let mut slots: Vec<Option<SpatialProgram>> =
                    sections.into_iter().map(Some).collect();
                (
                    order
                        .iter()
                        .map(|k| slots[*k].take().expect("each section once"))
                        .collect(),
                    Some(barriers),
                    order,
                )
            }
            None => match super::passes::pack::pack_phases(sections, target) {
                Ok(packed) => (packed.sections, Some(packed.barrier_after), packed.order),
                Err(sections) => {
                    let order = (0..sections.len()).collect();
                    (sections, None, order)
                }
            },
        };
        let ordered: Vec<Origin> = order.iter().map(|&k| origins[k].clone()).collect();
        match super::passes::compose::compose_phased(
            sections,
            &links(&rp.stages),
            &state,
            super::passes::compose::SectionPlacement::Stacked,
            barriers.as_deref(),
            target,
        ) {
            // ★ THE COMPOSED PROGRAM EARNS ITS OWN CERTIFICATE. `compose` hands back a scheduled program
            // with `certificate: None` — the section certificates it was built from say nothing about the
            // choreography that now runs them back to back. So the whole thing goes through `verify` once
            // more, and a composition whose barriers, buffers or ordering do not hold is refused HERE
            // rather than shipped as a bundle whose certificate covers one section of many.
            Ok(p) => match super::passes::verify::verify(&declare_ranges(p, &ranges), target) {
                Ok(v) => {
                    assert_eq!(v.sections.len(), ordered.len(), "the composer keeps one span a section, in the order it was handed them");
                    lineage_sections = ordered;
                    program = Some(v)
                }
                Err(e) => missing.push(MissingStage {
                    stmt_index: rp.stages.last().map(|s| s.stmt_index).unwrap_or(0),
                    binder: rp.stages.last().map(|s| s.out.clone()).unwrap_or_default(),
                    kind: "verify".into(),
                    why: format!("the composed program does not certify: {e:?}"),
                }),
            },
            Err(e) => missing.push(MissingStage {
                stmt_index: rp.stages.last().map(|s| s.stmt_index).unwrap_or(0),
                binder: rp.stages.last().map(|s| s.out.clone()).unwrap_or_default(),
                kind: "compose".into(),
                why: format!("{e:?}"),
            }),
        }
    }
    let carried = super::passes::recognize_stages::carried_state(&rp);
    Ok((
        ProgramLowering {
            program,
            statements,
            lowered,
            sections: n_sections,
            missing,
            state,
            carried,
        },
        super::lineage::Lineage { ops: lifted_ops, fusion, sections: lineage_sections },
    ))
}

/// ★ AN INTEGER BUFFER IS STORED AT THE NARROWEST PRECISION THAT HOLDS WHAT IT HOLDS. On a target whose datapath
/// stores narrow integers, a section's buffers take the narrowest of them that holds their proven interval
/// ([`super::value_range::program_ranges`] over the section, its tensors declaring the front end's intervals) — an int8
/// cache is an int8 ring, not an int64 one, which is what lets an attention's cache and its tables fit a GPU block's
/// shared memory. Set before tiling, so the plan sizes the narrower pages. A section with a matmul narrows only the
/// buffers its loads fill: before tiling its contraction is not yet blocked, so what the product holds is not yet
/// known; a load's buffer holds its tensor and, where the tensor ends inside a tile, the padding zeros. A matmul's
/// operand buffer keeps the operand precision its numerics chose.
fn narrow_storage(
    mut t: SpatialProgram,
    ranges: &mut std::collections::BTreeMap<String, super::value_range::Interval>,
    target: &SpatialTarget,
) -> SpatialProgram {
    use super::target::precision_int_bits;
    let storable = target.storable_ints();
    let matmul = |s: &Stage| matches!(s.kind, StageKind::Compute(ComputeOp::MatmulTiles { .. }));
    let operands: std::collections::BTreeSet<BufferId> =
        t.stages.iter().filter(|s| matmul(s)).flat_map(|s| s.ins.iter().copied()).collect();
    let proven = super::value_range::program_ranges(&declare_ranges(t.clone(), ranges));
    let whole = !t.stages.iter().any(matmul);
    // A tensor a whole section stores and the front end did not range — the inside of a decomposed op, a later
    // section's input — holds what the section proves it stores.
    if whole {
        for st in &t.stages {
            if let StageKind::Store { tensor, .. } = &st.kind {
                let name = &t.tensor(*tensor).name;
                if let (false, Some(iv)) = (ranges.contains_key(name), proven.tensors[tensor.idx()]) {
                    if iv != super::value_range::UNBOUNDED {
                        ranges.insert(name.clone(), iv);
                    }
                }
            }
        }
    }
    if storable.iter().all(|p| *p == Precision::Int64) {
        return t;
    }
    let loaded: std::collections::BTreeSet<BufferId> = t
        .stages
        .iter()
        .filter(|s| matches!(s.kind, StageKind::Load { .. }))
        .flat_map(|s| s.outs.iter().copied())
        .collect();
    for b in &mut t.buffers {
        let Some(now) = precision_int_bits(b.dtype) else { continue };
        if operands.contains(&b.id) || !(whole || loaded.contains(&b.id)) {
            continue;
        }
        let Some(iv) = proven.buffers[b.id.idx()] else { continue };
        let fits = |p: &Precision| {
            let bits = precision_int_bits(*p).unwrap_or(64);
            bits >= 64 || (iv.0 >= -(1i64 << (bits - 1)) && iv.1 < (1i64 << (bits - 1)))
        };
        if let Some(&p) = storable.iter().find(|p| fits(p)) {
            if precision_int_bits(p).unwrap_or(64) < now {
                b.dtype = p;
            }
        }
    }
    t
}

/// ★ THE FRONT END'S INTERVALS ARE THE PROGRAM'S DECLARED RANGES. Every integer tensor the walk proved an interval for
/// declares it — met with a range it already declares, both holding every value it takes — so a backend reads, from the
/// program alone, what each tensor can hold ([`super::value_range::program_ranges`]). Set before the composed program
/// is certified, so its certificate covers them.
fn declare_ranges(
    mut p: SpatialProgram,
    ranges: &std::collections::BTreeMap<String, super::value_range::Interval>,
) -> SpatialProgram {
    for t in &mut p.tensors {
        if super::target::precision_int_bits(t.dtype).is_none() {
            continue;
        }
        let Some(&r) = ranges.get(&t.name) else { continue };
        if r == super::value_range::UNBOUNDED {
            continue;
        }
        t.range = Some(match t.range {
            Some(d) if d.0.max(r.0) <= d.1.min(r.1) => (d.0.max(r.0), d.1.min(r.1)),
            _ => r,
        });
    }
    p
}

/// The tables `source` lays out DENSE ([`super::link::dense_tables`]) — what a deployment's other programs must agree with.
pub fn dense_tables_of(
    source: &str,
    target: &SpatialTarget,
) -> Result<std::collections::BTreeSet<String>, SpatialReject> {
    let g = TensorGraph::lift_source(source, &|_| Storage::Values)?.graph;
    let every: Vec<usize> = (0..g.ops.len()).collect();
    Ok(dense_tables(&g, &every, target, &std::collections::BTreeSet::new()))
}

/// The proven interval of every integer tensor of `source` ([`super::value_range::value_ranges`]), by name — the ranges
/// each matmul's operand precision is chosen from.
pub fn value_ranges_of(
    source: &str,
) -> Result<std::collections::BTreeMap<String, super::value_range::Interval>, SpatialReject> {
    let rp = TensorGraph::lift_source(source, &|_| Storage::Values)?.program;
    Ok(super::value_range::value_ranges(&rp, &float_tensors(&rp)))
}

/// The dataflow the walk found, for a caller that wants the links without re-walking.
pub fn program_links(source: &str) -> Result<Vec<Link>, SpatialReject> {
    TensorGraph::lift_source(source, &|_| Storage::Values).map(|l| links(&l.program.stages))
}

/// A calibration candidate: the compiler's own choice, or the best plan of one reader arrangement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateKind {
    Best,
    Streamed,
    Fused,
    Unfused,
}

impl CandidateKind {
    /// The arrangement label a program of this kind prints (the ledger keys on it).
    pub fn label(self) -> &'static str {
        match self {
            CandidateKind::Best => "best",
            CandidateKind::Streamed => "streamed",
            CandidateKind::Fused => "multicast",
            CandidateKind::Unfused => "blocked",
        }
    }
}

impl PlanSummary {
    /// The emergent arrangement, as a label.
    /// The arrangement a measurement names. The movement packing is part of it: two programs that differ
    /// only in which engine carries the store are DIFFERENT arrangements, and a ledger must not let one
    /// supersede the other.
    pub fn arrangement_label(&self) -> &'static str {
        let split = self.engines == EnginePacking::OperandWithStore;
        match (self.streaming, self.multicast_groups > 0, split) {
            (true, _, _) => "streamed",
            (_, true, false) => "multicast",
            (_, true, true) => "multicast-split",
            (_, false, false) => "blocked",
            (_, false, true) => "blocked-split",
        }
    }
}

/// Every calibration candidate of a source on a target — the compiler's own choice plus the best plan of
/// each reader arrangement, each run through the SAME passes. A measurement harness's facility, never a
/// compiler decision.
/// Restrict the plans a candidate search may pick from — the knobs a measurement campaign varies one at a
/// time. A knob left `None` is the compiler's own choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanFilter {
    pub pack_accumulate: Option<bool>,
    pub engines: Option<EnginePacking>,
    /// The per-site output block in tiles.
    pub per_site: Option<(usize, usize)>,
    /// The sub-block and reduction depth the compute engine works in.
    pub block: Option<BlockShape>,
}

impl PlanFilter {
    fn allows(&self, plan: &BlockPlan) -> bool {
        self.pack_accumulate
            .map(|w| plan.pack_accumulate == w)
            .unwrap_or(true)
            && self.engines.map(|w| plan.engines == w).unwrap_or(true)
            && self
                .per_site
                .map(|w| (plan.per_site_m, plan.per_site_n) == w)
                .unwrap_or(true)
            && self.block.map(|w| plan.block == w).unwrap_or(true)
    }
    fn is_empty(&self) -> bool {
        *self == PlanFilter::default()
    }
}

pub fn calibration_candidates(
    source: &str,
    target: &SpatialTarget,
    numerics: NumericsPolicy,
) -> Result<Vec<(CandidateKind, SpatialProgram)>, SpatialReject> {
    calibration_candidates_matching(source, target, numerics, PlanFilter::default())
}

/// The best candidate of each arrangement AMONG the plans the filter allows.
pub fn calibration_candidates_matching(
    source: &str,
    target: &SpatialTarget,
    numerics: NumericsPolicy,
    want: PlanFilter,
) -> Result<Vec<(CandidateKind, SpatialProgram)>, SpatialReject> {
    let template = template_from_source_with(source, target, numerics)?;
    let mut out = Vec::new();
    if want.is_empty() {
        out.push((CandidateKind::Best, run_passes(template.clone(), target)?));
    }
    let cands: Vec<tile::Candidate> =
        tile::candidates_where(&template, target, &|plan| want.allows(plan))
            .into_iter()
            .filter(|c| want.allows(&c.plan))
            .collect();
    // ★ THE PLAN SEARCH'S OWN CHOICE, restricted to each arrangement: `tile::best_among` admits and ranks
    // exactly as the compiler does, so every ledger row measures a plan the compiler would emit given that
    // arrangement, and one ranking serves both.
    let objective = crate::spatial::work::Objective::default();
    let pick = |filter: &dyn Fn(&tile::Candidate) -> bool| {
        let arrangement: Vec<tile::Candidate> =
            cands.iter().filter(|c| filter(c)).cloned().collect();
        tile::best_among(&arrangement, objective).cloned()
    };
    let variants: [(CandidateKind, Option<tile::Candidate>); 4] = [
        (
            CandidateKind::Best,
            if want.is_empty() {
                None
            } else {
                tile::best_among(&cands, objective).cloned()
            },
        ),
        (
            CandidateKind::Streamed,
            pick(&|c| super::cost::is_streaming(&c.plan)),
        ),
        (CandidateKind::Fused, pick(&|c| c.fused)),
        (
            CandidateKind::Unfused,
            pick(&|c| !c.fused && !super::cost::is_streaming(&c.plan)),
        ),
    ];
    for (kind, cand) in variants {
        let Some(c) = cand else { continue };
        out.push((kind, build_candidate(&template, &c, target)?));
    }
    Ok(out)
}

/// The program candidate `c` of `template` builds: its plan applied, placed, fused as the candidate says,
/// sized, handshaken, persisted and verified. What the plan search priced in closed form is what this emits.
pub fn build_candidate(
    template: &SpatialProgram,
    c: &tile::Candidate,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    verify::verify(&build_candidate_uncertified(template, c, target)?, target)
}

/// [`build_candidate`] short of its certificate: the candidate tiled, placed, sized and scheduled — what a cycle
/// twin times.
pub fn build_candidate_uncertified(
    template: &SpatialProgram,
    c: &tile::Candidate,
    target: &SpatialTarget,
) -> Result<SpatialProgram, SpatialReject> {
    let tiled = tile::apply_plan(template, c.plan, target);
    let placed = place::place(&tiled, target)?;
    let fused = fuse_readers::fuse_readers_forced(&placed, target, c.fused)?;
    let sized = size_buffers::size_buffers(&fused, target)?;
    let sched = synthesize_handshakes::synthesize_handshakes(&sized, target)?;
    persist::persist(&sched, target)
}
