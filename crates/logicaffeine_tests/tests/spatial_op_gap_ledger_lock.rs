//! spatial_op_gap_ledger_lock — WHICH tensor ops reach the SN50 RDU's certified neutral artifact, as a
//! shrink-only ledger over (op × stage) cells, so "the spatial IR encompasses everything the model needs"
//! is a count that can only fall.
//!
//! Four stages on `SpatialTarget::sn50_rdu()`, each requiring the one before it — every one asked the way the spatial
//! compiler takes a program: the whole-program walk, the composer, the certificate, the RDU lane.
//!   Recognized — the walk (`recognize_stages`) lifts the program as exactly ONE stage of the kind the probe
//!                names (`OpProbe::kind`). A fragment of the desugar lifted as another op is NOT recognition
//!                (see `MISRECOGNIZED`).
//!   Lowered    — `compile_program_to_spatial` lowers every statement the walk lifted.
//!   Verified   — the composition certifies (`Phase::Verified`), and one of its sections is the op's own program
//!                family (`OpProbe::family`).
//!   Neutral    — `RduSpatialBackend::for_chip(Sn50).lower` is `SdkGated`, the shipped certificate rechecks
//!                against the program, and `eval_choreography` ≡ `vm_outcome` under the rounding rule of
//!                `spatial_front_end_lock` (the target's float datapath width, one ulp of the ×1e6 rounding).
//!
//! The probes are `common/tensor_probes.rs::OP_PROBES` (D4) — the SAME programs the seven-backend coverage
//! matrix runs — so the two ratchets can never disagree about what an op is.
//!
//! The teeth:
//!   • `every_op_reaches_the_rdu_neutral_artifact_or_is_a_listed_gap` and `no_gap_masks_a_cell_that_works`
//!     together make `SPATIAL_OP_GAPS` EXACTLY the live set of failing cells — a regression fails RED, and a
//!     cell that starts working demands its row be deleted;
//!   • `the_ledger_only_shrinks` freezes the row count (`FROZEN_GAP_COUNT`; re-freeze downward only);
//!   • `a_stage_implies_its_predecessors` — a gap drags every later stage with it, in the table itself;
//!   • `the_verdict_function_has_teeth` — the Neutral criterion is fed a wrong answer and must refuse it;
//!   • `the_front_end_never_lowers_a_fragment_as_the_whole_program` — no probe is lifted with a stage of another
//!     kind: the fragment miscompiles this ledger found (`gelu`/`silu` → rmsnorm, `moe` → softmax) were a
//!     shrink-only table of their own; it is empty since the recognizers key on whole desugar shapes, and it stays
//!     as the trap for the next one;
//!   • `the_probe_set_covers_every_parser_sugar` — the parser's `fn desugar_tensor_*` definitions are
//!     scanned (comments stripped) against `SUGAR_TO_PROBE`, no sugar unprobed, no stale row.
//!
//!
//! ★ MUTATION PROBE (each run once — break, observe red, restore):
//!   • delete the `("transpose", Stage::Neutral, …)` row → `every_op_reaches_…_or_is_a_listed_gap` red,
//!     naming the cell;
//!   • add a `("matmul", Stage::Neutral, "x")` row → `no_gap_masks_a_cell_that_works` red: "delete its row";
//!   • make `neutral` return `Ok(())` unconditionally → `the_verdict_function_has_teeth` red;
//!   • let `recognize_fused_row`'s flat fallback accept any divisor again (drop `binds_rms_denominator`) →
//!     `the_front_end_never_lowers_a_fragment_as_the_whole_program` red, naming a `FusedRow` stage for gelu;
//!   • add a `("bogus", "matmul")` row to `SUGAR_TO_PROBE` → `the_probe_set_covers_every_parser_sugar` red
//!     as a stale row;
//!   • delete the `("sigmoid", Stage::Lowered, …)` row → `a_stage_implies_its_predecessors` red.
//!
//!   cargo nextest run -p logicaffeine-tests --test spatial_op_gap_ledger_lock
#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, Outputs, Value};
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram};
use logicaffeine_compile::spatial::lower::rdu::RduSpatialBackend;
use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend};
use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, ProgramLowering};
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::target::SpatialTarget;
use tensor_probes::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Recognized,
    Lowered,
    Verified,
    Neutral,
}

const STAGES: [Stage; 4] = [
    Stage::Recognized,
    Stage::Lowered,
    Stage::Verified,
    Stage::Neutral,
];

fn target() -> SpatialTarget {
    SpatialTarget::sn50_rdu()
}

fn backend() -> RduSpatialBackend {
    RduSpatialBackend::for_chip(RduChip::Sn50)
}

/// SHRINK-ONLY: every (op, stage) cell that does not work on the SN50 lane today, with the piece it is
/// waiting on. A stage that fails lists every later stage too (`a_stage_implies_its_predecessors`), so the
/// ledger is total over the cells that fail and the count is the honest distance to "everything".
const SPATIAL_OP_GAPS: &[(&str, Stage, &str)] = &[
    // ── compositions whose bf16 answer lands past the one-ulp rule: the ops lower, certify and ship; the chip's
    //    rounding through their chain of stages is more than one ulp of the ×1e6 rounding ────────────────────
    ("rope_partial", Stage::Neutral, "the rotation's two bf16 products and their difference round past one ulp (Show #6: 7625000 vs VM 7600000)"),
    ("full_attention", Stage::Neutral, "scores → softmax → weighted sum round past one ulp in bf16 (Show #4: 5312500 vs VM 5339523)"),
    ("gated_delta", Stage::Neutral, "the predict / update / read chain rounds past one ulp in bf16 (Show #4: 4000000 vs VM 3991800)"),
    ("attention", Stage::Neutral, "scores → softmax → weighted sum round past one ulp in bf16 (Show #4: 5312500 vs VM 5339523)"),
];

/// Today's row count. Re-freeze DOWNWARD when cells start working; never up.
const FROZEN_GAP_COUNT: usize = 4;

/// SHRINK-ONLY: programs the front end lifts as a DIFFERENT op than the one they name — a fragment of the
/// desugar matched as the whole program, which then goes through tiling, placement, choreography and a
/// certificate as a Verified program computing the wrong function. This ledger's ≡VM check found three
/// (`gelu`/`silu` → rmsnorm, `moe` → softmax); every recognizer now keys on its whole desugar shape (the flat
/// reduction fallback checks the divisor it claims, and a Push-based lift inside a desugar block must target
/// that block's own binder), so the table is empty. A new row is a regression of that rule, not a fact to
/// record. `(op, the kind the walk lifts it as, why)`.
const MISRECOGNIZED: &[(&str, &str, &str)] = &[];

#[derive(Debug, PartialEq)]
enum Verdict {
    Works,
    Fails,
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

fn norm(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

/// The kinds of stage the walk lifts the probe as, in order.
fn lifted_kinds(p: &OpProbe) -> Result<Vec<&'static str>, String> {
    let rp = recognize_stages(p.program).map_err(|e| format!("walk: {e:?}"))?;
    Ok(rp.stages.iter().map(|s| s.op.kind()).collect())
}

/// Recognized: the walk lifts the program as exactly one stage of the probe's own kind.
fn recognized(p: &OpProbe) -> Result<(), String> {
    let kinds = lifted_kinds(p)?;
    if kinds == [p.kind] {
        Ok(())
    } else {
        Err(format!("lifted as {kinds:?}, not as one `{}`", p.kind))
    }
}

/// Lowered: every statement the walk lifted lowers.
fn lowered(p: &OpProbe) -> Result<ProgramLowering, String> {
    recognized(p)?;
    let pl =
        compile_program_to_spatial(p.program, &target()).map_err(|e| format!("walk: {e:?}"))?;
    if pl.lowered != pl.statements {
        return Err(format!(
            "lowered {} of {} statements: {}",
            pl.lowered,
            pl.statements,
            pl.refusal()
        ));
    }
    Ok(pl)
}

/// Verified: the composition certifies, and a section of it is the op's own program.
fn verified(p: &OpProbe) -> Result<SpatialProgram, String> {
    let pl = lowered(p)?;
    let sp = pl.whole().ok_or_else(|| pl.refusal())?.clone();
    if sp.phase() != Phase::Verified {
        return Err(format!("phase {:?}, not Verified", sp.phase()));
    }
    if !sp.sections.iter().any(|x| x.name.starts_with(p.family)) {
        return Err(format!(
            "no section is a `{}` program: {:?}",
            p.family,
            sp.sections.iter().map(|x| &x.name).collect::<Vec<_>>()
        ));
    }
    Ok(sp)
}

/// The Neutral criterion for a verified program and an evaluation of it — SdkGated ∧ the certificate rechecks
/// ∧ the evaluation shows what the VM shows. Takes the evaluation as an argument so
/// `the_verdict_function_has_teeth` can hand it a wrong one.
fn neutral(p: &OpProbe, sp: &SpatialProgram, outputs: &Outputs) -> Result<(), String> {
    let artifact = backend().lower(sp).map_err(|e| format!("lower: {e:?}"))?;
    let SpatialArtifact::SdkGated(n) = artifact else {
        return Err("the RDU lane did not produce the SDK-gated neutral artifact".into());
    };
    if !n.certificate.recheck(sp) {
        return Err("the shipped certificate does not recheck against the program".into());
    }
    let vm = vm_outcome(p.program);
    if let Some(e) = vm.error.as_deref() {
        return Err(format!("VM: {}", first_line(e)));
    }
    agrees_with_oracle(p.program, sp, outputs, &vm.output)
}

/// Why a cell fails, or `Ok` — the verdict with its reason.
fn cell(p: &OpProbe, stage: Stage) -> Result<(), String> {
    match stage {
        Stage::Recognized => recognized(p),
        Stage::Lowered => lowered(p).map(drop),
        Stage::Verified => verified(p).map(drop),
        Stage::Neutral => {
            let sp = verified(p)?;
            let inputs = inputs_from_fills(&sp)?;
            let out = eval_choreography(&sp, &inputs).map_err(|e| format!("eval: {e:?}"))?;
            neutral(p, &sp, &out)
        }
    }
}

fn verdict(p: &OpProbe, stage: Stage) -> Verdict {
    match cell(p, stage) {
        Ok(()) => Verdict::Works,
        Err(_) => Verdict::Fails,
    }
}

fn is_gap(op: &str, stage: Stage) -> bool {
    SPATIAL_OP_GAPS
        .iter()
        .any(|(o, s, _)| *o == op && *s == stage)
}

fn is_pending_sugar(op: &str) -> bool {
    SPATIAL_OP_GAPS
        .iter()
        .any(|(o, s, r)| *o == op && *s == Stage::Recognized && *r == PENDING_SUGAR_REASON)
}

// ── the ledger ────────────────────────────────────────────────────────────────────────────────────

#[test]
fn every_op_reaches_the_rdu_neutral_artifact_or_is_a_listed_gap() {
    let mut violations = Vec::new();
    for p in OP_PROBES {
        for stage in STAGES {
            if is_gap(p.op, stage) {
                continue;
            }
            if let Err(why) = cell(p, stage) {
                violations.push(format!(
                    "{} at {stage:?}: FAILS but is not a listed gap — {why}",
                    p.op
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "spatial op coverage regressed on {}:\n{}",
        target().name,
        violations.join("\n")
    );
}

#[test]
fn no_gap_masks_a_cell_that_works() {
    let mut fixed = Vec::new();
    for &(op, stage, _) in SPATIAL_OP_GAPS {
        if verdict(probe(op), stage) == Verdict::Works {
            fixed.push(format!("{op} at {stage:?} now WORKS — delete its row (gaps only shrink) and re-freeze FROZEN_GAP_COUNT"));
        }
    }
    assert!(
        fixed.is_empty(),
        "stale gaps mask real coverage:\n{}",
        fixed.join("\n")
    );
}

#[test]
fn the_ledger_only_shrinks() {
    assert!(
        SPATIAL_OP_GAPS.len() <= FROZEN_GAP_COUNT,
        "the ledger GREW: {} rows > FROZEN_GAP_COUNT {FROZEN_GAP_COUNT} — an op lost a stage on the RDU lane; fix it, do not raise the freeze",
        SPATIAL_OP_GAPS.len()
    );
}

#[test]
fn gaps_reference_real_ops_and_stages() {
    let mut seen = std::collections::BTreeSet::new();
    for &(op, stage, reason) in SPATIAL_OP_GAPS {
        assert!(
            OP_PROBES.iter().any(|p| p.op == op),
            "SPATIAL_OP_GAPS names unknown op `{op}`"
        );
        assert!(
            STAGES.contains(&stage),
            "SPATIAL_OP_GAPS names unknown stage {stage:?}"
        );
        assert!(
            !reason.trim().is_empty(),
            "{op} at {stage:?}: a gap needs a reason naming the missing piece"
        );
        assert!(
            seen.insert((op, stage)),
            "{op} at {stage:?} is listed twice"
        );
    }
}

#[test]
fn a_stage_implies_its_predecessors() {
    // Reaching a stage implies reaching every stage before it; so a gap at one stage makes every later
    // stage unreachable, and the table must say so — a `Verified` gap without a `Neutral` gap is a ledger
    // that claims an unverified program reached the artifact.
    let mut holes = Vec::new();
    for &(op, stage, _) in SPATIAL_OP_GAPS {
        for later in STAGES.iter().filter(|s| **s > stage) {
            if !is_gap(op, *later) {
                holes.push(format!(
                    "{op}: a gap at {stage:?} makes {later:?} unreachable too — list it"
                ));
            }
        }
    }
    assert!(
        holes.is_empty(),
        "the ledger is inconsistent:\n{}",
        holes.join("\n")
    );
}

#[test]
fn every_probe_actually_computes_something_on_the_oracle() {
    // TEETH FOR THE ORACLE ITSELF. Every stage verdict ends in "≡ the VM's stdout", so a probe that fails to
    // PARSE would print nothing and a broken op could read as agreed-with. A probe must really run — no VM
    // error, exactly one value per `Show` — unless the ledger says its sugar does not exist yet, in which
    // case the day the parser accepts it the row must go.
    let mut bad = Vec::new();
    for p in OP_PROBES {
        let out = vm_outcome(p.program);
        let want = p.program.matches("Show ").count();
        let computes = match out.error.as_deref() {
            Some(_) => false,
            None => norm(&out.output).len() == want,
        };
        match (computes, is_pending_sugar(p.op)) {
            (true, false) => {}
            (false, true) => {}
            (false, false) => match out.error.as_deref() {
                Some(err) => bad.push(format!("{}: VM errored: {}", p.op, first_line(err))),
                None => bad.push(format!("{}: VM printed {} values but the program has {want} `Show`s", p.op, norm(&out.output).len())),
            },
            (true, true) => bad.push(format!("{}: the parser accepts it now — delete its `{PENDING_SUGAR_REASON}` rows and re-freeze", p.op)),
        }
    }
    assert!(
        bad.is_empty(),
        "probes that do not actually execute on the oracle:\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_oracle_computes_the_known_answers() {
    // Independent correctness anchor for the exact ops: the oracle itself is right, not just self-consistent.
    for p in OP_PROBES {
        if let Some(expect) = p.expect {
            assert_eq!(
                norm(&vm_outcome(p.program).output),
                expect,
                "{} on the VM oracle",
                p.op
            );
        }
    }
}

#[test]
fn the_verdict_function_has_teeth() {
    // A program whose every shown value is 1, and a hand-built evaluation that says 2: the Neutral criterion
    // must refuse it — and accept the true evaluation, so the refusal is discrimination, not a constant.
    const ONES: &str = concat!(
        "## Main\n",
        "Let A be a 2 by 2 tensor of Int.\n",
        "Set item 1 of A to 1.\nSet item 2 of A to 1.\nSet item 3 of A to 1.\nSet item 4 of A to 1.\n",
        "Let B be a 2 by 2 tensor of Int.\n",
        "Let C be A plus B.\n",
        "Show item 1 of C.\nShow item 2 of C.\nShow item 3 of C.\nShow item 4 of C.\n",
    );
    let ones = OpProbe {
        op: "ones",
        family: "elementwise_+",
        kind: "Elementwise",
        program: ONES,
        shows: 4,
        needs: &[],
        expect: Some(&["1", "1", "1", "1"]),
    };
    assert_eq!(
        norm(&vm_outcome(ONES).output),
        ["1", "1", "1", "1"],
        "the VM shows 1 four times"
    );
    let sp = verified(&ones).expect("a 2×2 elementwise plus verifies on the RDU lane");
    let out = output_id(&sp);
    let truth = eval_choreography(&sp, &inputs_from_fills(&sp).unwrap()).unwrap();
    assert_eq!(
        neutral(&ones, &sp, &truth),
        Ok(()),
        "the true evaluation is Neutral"
    );

    let mut wrong = truth.clone();
    wrong.tensors.insert(out, Value::Int(vec![2, 2, 2, 2]));
    let refused = neutral(&ones, &sp, &wrong);
    assert!(
        refused.is_err(),
        "an evaluation of 2 where the VM shows 1 must be refused"
    );
    assert!(
        refused.as_ref().unwrap_err().contains("spatial 2 vs VM 1"),
        "{refused:?}"
    );

    let mut short = truth.clone();
    short.tensors.insert(out, Value::Int(vec![1, 1, 1]));
    assert!(
        neutral(&ones, &sp, &short).is_err(),
        "fewer values than the program shows is not agreement"
    );

    let mut missing = truth.clone();
    missing.tensors.remove(&out);
    assert!(
        neutral(&ones, &sp, &missing).is_err(),
        "no value for the output is not agreement"
    );

    // The float rule tolerates one ulp of the ×1e6 rounding at the datapath's width — not a real difference.
    let rms = probe("rmsnorm");
    let sp = verified(rms).expect("rmsnorm verifies on the RDU lane");
    let out = output_id(&sp);
    let truth = eval_choreography(&sp, &inputs_from_fills(&sp).unwrap()).unwrap();
    assert_eq!(neutral(rms, &sp, &truth), Ok(()));
    let Value::Float(mut v) = truth.tensors[&out].clone() else {
        panic!("rmsnorm is float")
    };
    v[0] += 0.01;
    let mut nudged = truth.clone();
    nudged.tensors.insert(out, Value::Float(v));
    assert!(
        neutral(rms, &sp, &nudged).is_err(),
        "a 0.01 error in a unit-scale float is 10000 units of the ×1e6 rounding — refused"
    );
}

// ── the fragment miscompiles this ledger found ─────────────────────────────────────────────────────

#[test]
fn the_front_end_never_lowers_a_fragment_as_the_whole_program() {
    let mut bad = Vec::new();
    for p in OP_PROBES {
        let foreign: Option<&str> = lifted_kinds(p)
            .unwrap_or_default()
            .into_iter()
            .find(|k| *k != p.kind);
        let listed = MISRECOGNIZED.iter().find(|(op, _, _)| *op == p.op);
        match (foreign, listed) {
            (Some(kind), None) => bad.push(format!("{}: the walk lifted a `{kind}` stage — a fragment of its desugar was taken for an op; refuse it or recognize it by name", p.op)),
            (Some(kind), Some((_, as_, _))) if kind != *as_ => bad.push(format!("{}: MISRECOGNIZED says `{as_}` but the walk lifts a `{kind}` — correct the row", p.op)),
            (None, Some(_)) => bad.push(format!("{}: the walk no longer lifts a fragment — delete its MISRECOGNIZED row", p.op)),
            _ => {}
        }
    }
    for (op, _, _) in MISRECOGNIZED {
        assert!(
            OP_PROBES.iter().any(|p| p.op == *op),
            "MISRECOGNIZED names unknown op `{op}`"
        );
        assert!(
            is_gap(op, Stage::Recognized),
            "{op} is mis-recognized, so it is not Recognized — the ledger must say so"
        );
    }
    assert!(
        bad.is_empty(),
        "the front end lifts fragments:\n{}",
        bad.join("\n")
    );
}

// ── the probe set itself ───────────────────────────────────────────────────────────────────────────

#[test]
fn the_probe_set_covers_every_parser_sugar() {
    let sugars = desugar_sugars_in(&parser_source());
    assert!(
        sugars.contains("matmul") && sugars.len() >= 10,
        "the parser scan found {sugars:?} — the scanner or the path is wrong"
    );
    let mut bad = Vec::new();
    for sugar in &sugars {
        let rows: Vec<&str> = SUGAR_TO_PROBE
            .iter()
            .filter(|(s, _)| s == sugar)
            .map(|(_, op)| *op)
            .collect();
        if rows.is_empty() {
            bad.push(format!("parser sugar `desugar_tensor_{sugar}` has no probe — add a SUGAR_TO_PROBE row and a probe program"));
        }
    }
    for (sugar, op) in SUGAR_TO_PROBE {
        if !sugars.contains(*sugar) {
            bad.push(format!("SUGAR_TO_PROBE row (`{sugar}`, `{op}`) is stale — the parser has no `fn desugar_tensor_{sugar}`"));
        }
        if !OP_PROBES.iter().any(|p| p.op == *op) {
            bad.push(format!(
                "SUGAR_TO_PROBE row (`{sugar}`, `{op}`) names a probe that does not exist"
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "the probe set and the parser disagree:\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_sugar_scanner_flags_a_definition_and_ignores_a_comment() {
    // Non-vacuity of the scan: a definition is found, a mention in a line or block comment is not, and a
    // name without the call parenthesis is not a definition.
    let src = "// fn desugar_tensor_commented(\n/* fn desugar_tensor_blocked( */\nfn desugar_tensor_bogus(&mut self) {}\nlet desugar_tensor_named = 1;\nfn desugar_tensor_two (x) {}\n";
    let found = desugar_sugars_in(src);
    assert_eq!(
        found.iter().map(String::as_str).collect::<Vec<_>>(),
        ["bogus"],
        "{found:?}"
    );
    assert!(!desugar_sugars_in(&parser_source()).contains("bogus"));
}

#[test]
fn the_probe_tables_partition_the_universe() {
    let names = |t: &[OpProbe]| {
        t.iter()
            .map(|p| p.op)
            .collect::<std::collections::BTreeSet<_>>()
    };
    let all = names(OP_PROBES);
    assert_eq!(
        all.len(),
        OP_PROBES.len(),
        "duplicate op names in OP_PROBES"
    );
    let (implemented, pending, matrix) = (
        names(IMPLEMENTED_PROBES),
        names(PENDING_SUGAR_PROBES),
        names(BACKEND_MATRIX_PROBES),
    );
    assert!(
        implemented.is_disjoint(&pending),
        "a probe is both implemented and pending: {:?}",
        implemented.intersection(&pending).collect::<Vec<_>>()
    );
    assert_eq!(
        implemented
            .union(&pending)
            .copied()
            .collect::<std::collections::BTreeSet<_>>(),
        all,
        "IMPLEMENTED ∪ PENDING must be every probe"
    );
    assert!(
        matrix.is_subset(&implemented),
        "the backend matrix may only run implemented sugars"
    );
    for op in implemented.difference(&matrix) {
        assert!(MISRECOGNIZED.iter().any(|(o, _, _)| o == op), "`{op}` is implemented but kept out of the backend matrix without being a listed fragment miscompile");
    }
    for op in &pending {
        for stage in STAGES {
            assert!(
                is_gap(op, stage),
                "pending sugar `{op}` must be a listed gap at {stage:?}"
            );
        }
        assert!(
            is_pending_sugar(op),
            "pending sugar `{op}`'s Recognized row must carry PENDING_SUGAR_REASON"
        );
    }
    for op in &implemented {
        assert!(
            !is_pending_sugar(op),
            "`{op}` is implemented but its ledger row says the sugar is pending"
        );
    }
}

#[test]
fn every_probe_is_self_consistent() {
    for p in OP_PROBES {
        assert_eq!(
            p.shows,
            p.program.matches("Show ").count(),
            "{}: `shows` must count the program's Show lines",
            p.op
        );
        assert_eq!(
            shows_of(p.program).len(),
            p.shows,
            "{}: every Show line must parse as a projection",
            p.op
        );
        for need in p.needs {
            assert!(
                PRIMITIVES.contains(need),
                "{}: `needs` names `{need}`, not a primitive in PRIMITIVES",
                p.op
            );
        }
        if let Some(e) = p.expect {
            assert_eq!(
                e.len(),
                p.shows,
                "{}: `expect` must have one value per Show",
                p.op
            );
        }
        assert!(
            p.program.starts_with("## Main\n"),
            "{}: a probe is a whole program",
            p.op
        );
        assert!(!p.family.is_empty());
    }
}
