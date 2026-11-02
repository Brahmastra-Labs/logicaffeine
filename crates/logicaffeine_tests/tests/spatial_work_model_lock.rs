//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE WORK MODEL LOCK — the compiler's model of what it makes the machine do is CHECKED, not fitted.
//!
//! `spatial::work::work_of` predicts three things by walking the scheduled choreography: the MVMULs the
//! program retires, the NOC transactions it initiates, and the bytes it moves. Nothing in it is fitted —
//! they are counts of ops the backend prints. This file runs the same programs the ledger measured on
//! the instrumented simulator and requires the prediction to match the machine.
//!
//! ★ WHY THIS EXISTS AND WHAT IT REPLACES. The cost model used to be a regression over ttsim's cycle
//! counter, and `ttsim_instrument_audit.json` showed that counter is not a function of the program: 8x
//! the reduction depth moves it 0.08%, and the device's own per-core cycle register 0.3%. Eighteen
//! constants were being fitted against noise — 13 of them jackknife-unstable, 3 never exercised at all —
//! and the compiler paid for it, ranking the plan that issues 2.9x the instructions FIRST at 512^3.
//!
//! A prediction that can be wrong is worth more than a fit that cannot. When this file goes red the
//! compiler's model of itself is wrong, and the answer is to fix the model — never to widen a tolerance
//! or re-fit a constant, because there is no constant here to re-fit.
//!
//! A. THE LAW IS EXACT. `matmul_retired` is `mt*nt*kt*64` and must match the machine to the MVMUL.
//! B. MOVEMENT IS PREDICTED. NOC bytes and transactions must match within a tolerance that only ever
//!    tightens — the residual is host-side traffic the choreography does not contain, not slack.
//! C. THE PREDICTION DISCRIMINATES PLANS. At one shape, two plans the ledger measured to differ must be
//!    predicted to differ, in the same direction. A model that ranks every plan alike is not a model.
//! D. NON-VACUITY: a deliberately mis-stated plan is predicted wrong and the check catches it.
//!
//! Honest boundary: work is not time. Nothing here is a speed claim.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(spatial_work_model_lock)'`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::cost::is_streaming;
use logicaffeine_compile::spatial::ir::{BlockShape, EnginePacking, NumericsPolicy};
use logicaffeine_compile::spatial::passes::tile::{candidates, Candidate};
use logicaffeine_compile::spatial::pipeline::{
    build_candidate, calibration_candidates_matching, compile_to_spatial_with,
    template_from_source_with, CandidateKind, PlanFilter,
};
use logicaffeine_compile::spatial::target::{Fidelity, MeshShape, SpatialTarget};
use logicaffeine_compile::spatial::work::{gemm_retired_floor, work_of, work_of_plan, Work};
use spatial_support::*;

/// How far a BYTE prediction may sit from the machine. Bytes and DRAM bytes are the objective's leading
/// keys, they are exact from the choreography, and they hold to a fraction of a percent at every shape
/// and every plan family. SHRINK-ONLY: a measurement that needs more than this is a defect in the model.
const BYTE_TOLERANCE: f64 = 0.001;

/// How far a TRANSACTION prediction may sit from the machine.
///
/// ★ AND WHY IT IS LOOSER THAN THE BYTE TOLERANCE, said plainly rather than quietly. On blocked and
/// streaming plans this axis is EXACT — one transaction per tile plus two per site for the host's
/// read-back, at every mesh from 16 to 64 sites. On MULTICAST plans the model OVER-counts by up to 13%,
/// and the excess is localized to the receiver-handshake term: at 128^3 it predicts 120 against the
/// machine's 108, at 512^3 1,152 against 1,096. The closed form and the choreography walk agree with each
/// other exactly there, so the gap is between the IR's handshake ops and what the emitted kernels
/// actually issue — a question about this compiler's own lowering, not about the model's arithmetic, and
/// it is recorded as an open item.
///
/// It is deliberately the objective's LAST key, so no plan is ever chosen on the axis the model knows
/// least about — and it is a RATCHET: when the term is found this constant comes down and never goes up.
const TXN_TOLERANCE: f64 = 0.15;

/// How far the plan-search's closed form may sit from the choreography it summarizes: nowhere. Both sides
/// are the compiler's own arithmetic over the same plan — there is no measurement in between and nothing
/// to excuse a gap. SHRINK-ONLY, and at its floor.
const CLOSED_FORM_TOLERANCE: f64 = 0.0;

/// One measured row, with everything needed to rebuild the program that produced it.
struct Measured {
    id: String,
    shape: (usize, usize, usize),
    candidate: CandidateKind,
    fidelity: Fidelity,
    fp32: bool,
    l1acc: bool,
    filter: PlanFilter,
    work: Work,
}

fn pair(s: &str) -> Option<(usize, usize)> {
    let (a, b) = s.split_once('x')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// Every instrumented `logos_spatial` row, rebuilt from its own reproduce command. Rows measured on
/// another simulator build, or by another stack, are not this model's to predict.
fn measured_rows() -> Vec<Measured> {
    let text =
        std::fs::read_to_string(repo_root().join("benchmarks/silicon/tenstorrent_gemm.json"))
            .expect("the ledger");
    let d: serde_json::Value = serde_json::from_str(&text).expect("the ledger parses");
    let mut out = Vec::new();
    for r in d["rows"].as_array().cloned().unwrap_or_default() {
        if r["current"] != true || r["chip"] != "wormhole_b0" || r["program"] != "logos_spatial" {
            continue;
        }
        let Some(w) = r["work"].as_object() else {
            continue;
        };
        let g = |k: &str| w.get(k).and_then(|v| v.as_u64());
        let (Some(retired), Some(txns), Some(bytes)) =
            (g("matmul_retired"), g("noc_txns"), g("noc_bytes"))
        else {
            continue;
        };
        // ★ REBUILD THE PLAN THE ROW PRINTED, not the one its command line asked for. A row measured with
        // `--candidate best` records whichever arrangement the compiler picked THAT day; predicting
        // today's `best` against it would compare two different programs and call the difference a
        // modelling error. The `cfg` line the program printed is the plan that actually ran.
        let strategy = r["strategy"].as_str().unwrap_or_default().to_string();
        let candidate = if strategy.contains("stream") {
            CandidateKind::Streamed
        } else if strategy.contains("multicast") || strategy.contains("mcast") {
            CandidateKind::Fused
        } else {
            CandidateKind::Unfused
        };
        let cfg = &r["cfg"];
        let fidelity = match r["knobs"]["fidelity"].as_str() {
            Some("LoFi") => Fidelity::LoFi,
            Some("HiFi2") => Fidelity::HiFi2,
            Some("HiFi3") => Fidelity::HiFi3,
            _ => Fidelity::HiFi4,
        };
        let l1acc = r["knobs"]["packer_l1_acc"].as_bool().unwrap_or(false);
        let filter = PlanFilter {
            pack_accumulate: Some(l1acc),
            // `-split` in the printed arrangement IS the engine packing: the operand reads are split
            // across the two movement engines rather than issued together.
            engines: Some(if strategy.contains("split") {
                EnginePacking::OperandWithStore
            } else {
                EnginePacking::LoadsTogether
            }),
            per_site: cfg["per_core"].as_str().and_then(pair),
            block: match (
                cfg["subblock"].as_str().and_then(pair),
                cfg["in0_block_w"].as_u64(),
            ) {
                (Some((osh, osw)), Some(ibw)) => Some(BlockShape {
                    osh,
                    osw,
                    ibw: ibw as usize,
                }),
                _ => None,
            },
        };
        if filter.per_site.is_none() || filter.block.is_none() {
            continue;
        }
        out.push(Measured {
            id: r["id"].as_str().unwrap_or_default().to_string(),
            shape: (
                r["shape"]["m"].as_u64().unwrap() as usize,
                r["shape"]["n"].as_u64().unwrap() as usize,
                r["shape"]["k"].as_u64().unwrap() as usize,
            ),
            candidate,
            fidelity,
            fp32: r["knobs"]["fp32_dest_acc"].as_bool().unwrap_or(false),
            l1acc,
            filter,
            work: Work {
                matmul_retired: retired,
                noc_txns: txns,
                noc_bytes: bytes,
                ..Work::default()
            },
        });
    }
    out
}

/// Rebuild the program a measured row ran, and predict its work.
fn predict(m: &Measured) -> Option<(SpatialTarget, Work)> {
    let target = SpatialTarget {
        sites: MeshShape { x: 8, y: 8 },
        ..SpatialTarget::wormhole_ttsim()
    }
    .at_fidelity(m.fidelity);
    let policy = if m.fp32 {
        NumericsPolicy::Exact
    } else {
        NumericsPolicy::AllowSpillRounding
    };
    let src = gemm_program(m.shape.0, m.shape.1, m.shape.2);
    let cands = calibration_candidates_matching(&src, &target, policy, m.filter).ok()?;
    let (_, program) = cands.into_iter().find(|(k, _)| *k == m.candidate)?;
    let w = work_of(&program, &target);
    Some((target, w))
}

fn rel(pred: u64, meas: u64) -> f64 {
    if meas == 0 {
        return if pred == 0 { 0.0 } else { 1.0 };
    }
    (pred as f64 - meas as f64).abs() / meas as f64
}

#[test]
fn the_conservation_law_is_exact_on_every_measured_program() {
    let rows = measured_rows();
    assert!(
        rows.len() >= 20,
        "the instrumented calibration set is thin: {} rows",
        rows.len()
    );
    for m in &rows {
        let want = gemm_retired_floor(
            m.shape.0,
            m.shape.1,
            m.shape.2,
            SpatialTarget::wormhole_ttsim().tile,
            m.fidelity,
        );
        assert_eq!(
            m.work.matmul_retired, want,
            "{}: {:?} at {:?} must retire exactly the floor {want} MVMULs; the machine retired {}. Below it the program did not compute the whole product; above it the compiler padded arithmetic the problem does not contain.",
            m.id, m.shape, m.fidelity, m.work.matmul_retired
        );
    }
}

#[test]
fn the_model_predicts_the_work_the_machine_actually_did() {
    let rows = measured_rows();
    let mut checked = 0;
    let mut worst = (0.0f64, String::new());
    for m in &rows {
        let Some((_, p)) = predict(m) else { continue };
        checked += 1;
        assert_eq!(
            p.matmul_retired, m.work.matmul_retired,
            "{}: the model predicts {} retired MVMULs and the machine retired {} — the compiler does not know what its own program computes",
            m.id, p.matmul_retired, m.work.matmul_retired
        );
        for (axis, pv, mv, tol) in [
            ("noc_bytes", p.noc_bytes, m.work.noc_bytes, BYTE_TOLERANCE),
            ("noc_txns", p.noc_txns, m.work.noc_txns, TXN_TOLERANCE),
        ] {
            let e = rel(pv, mv);
            if e > worst.0 {
                worst = (e, format!("{} {axis}: predicted {pv} measured {mv}", m.id));
            }
            assert!(
                e <= tol,
                "{}: {axis} predicted {pv} but the machine did {mv} ({:.1}% off). Fix the model — there is no constant here to re-fit.",
                m.id,
                e * 100.0
            );
        }
    }
    assert!(
        checked >= 15,
        "only {checked} measured programs could be rebuilt and predicted"
    );
    eprintln!(
        "work model: {checked} programs, worst movement error {:.2}% ({})",
        worst.0 * 100.0,
        worst.1
    );
}

#[test]
fn the_prediction_tightens_only() {
    // Both tolerances are ratchets: if the worst real error sits far below one, the constant is lagging
    // the truth and must come down. A tolerance that never tightens has stopped being a lock.
    let rows = measured_rows();
    let (mut wb, mut wt) = (0.0f64, 0.0f64);
    for m in &rows {
        let Some((_, p)) = predict(m) else { continue };
        wb = wb.max(rel(p.noc_bytes, m.work.noc_bytes));
        wt = wt.max(rel(p.noc_txns, m.work.noc_txns));
    }
    assert!(
        wb <= BYTE_TOLERANCE,
        "worst byte error {:.3}% exceeds the ratchet",
        wb * 100.0
    );
    assert!(
        wt <= TXN_TOLERANCE,
        "worst transaction error {:.2}% exceeds the ratchet",
        wt * 100.0
    );
    assert!(
        wb >= BYTE_TOLERANCE / 10.0,
        "the worst byte error is {:.4}% against a {:.2}% ratchet — tighten BYTE_TOLERANCE",
        wb * 100.0,
        BYTE_TOLERANCE * 100.0
    );
    assert!(wt >= TXN_TOLERANCE / 4.0, "the worst transaction error is {:.3}% against a {:.1}% ratchet — the missing multicast-handshake term has been found; tighten TXN_TOLERANCE", wt * 100.0, TXN_TOLERANCE * 100.0);
    eprintln!(
        "work model ratchets: bytes {:.4}% / {:.2}%, transactions {:.2}% / {:.1}%",
        wb * 100.0,
        BYTE_TOLERANCE * 100.0,
        wt * 100.0,
        TXN_TOLERANCE * 100.0
    );
}

#[test]
fn the_model_tells_two_measured_plans_apart_in_the_direction_the_machine_did() {
    // C. A model that predicts every plan alike ranks nothing. Where the ledger measured two plans at one
    // shape and they really differ, the prediction must order them the same way.
    let rows = measured_rows();
    let mut pairs = 0;
    for (i, a) in rows.iter().enumerate() {
        for b in rows.iter().skip(i + 1) {
            if a.shape != b.shape || a.work.noc_bytes == b.work.noc_bytes {
                continue;
            }
            // Only a real difference is evidence: within the tolerance the two are the same measurement.
            if rel(a.work.noc_bytes, b.work.noc_bytes) < 2.0 * BYTE_TOLERANCE {
                continue;
            }
            let (Some((_, pa)), Some((_, pb))) = (predict(a), predict(b)) else {
                continue;
            };
            pairs += 1;
            assert_eq!(
                pa.noc_bytes < pb.noc_bytes,
                a.work.noc_bytes < b.work.noc_bytes,
                "at {:?} the machine moved {} bytes for {} and {} for {}, but the model predicts {} and {} — it would rank them backwards",
                a.shape,
                a.work.noc_bytes,
                a.id,
                b.work.noc_bytes,
                b.id,
                pa.noc_bytes,
                pb.noc_bytes
            );
        }
    }
    assert!(pairs >= 2, "the ledger must hold measurably different plans at a shared shape for this to mean anything: {pairs} pairs");
}

#[test]
fn a_misstated_plan_is_predicted_wrong() {
    // D. NON-VACUITY. Predict a program against the WRONG shape and the law must reject it — otherwise
    // the checks above would pass on a model that had stopped reading the program.
    let t = SpatialTarget::wormhole_ttsim();
    assert_eq!(
        gemm_retired_floor(512, 512, 512, t.tile, Fidelity::HiFi4),
        262_144
    );
    assert_ne!(
        gemm_retired_floor(512, 512, 1024, t.tile, Fidelity::HiFi4),
        262_144
    );
    // The fidelity multiplier is exact: MEASURED 16 / 32 / 48 / 64 MVMULs per tile-matmul.
    assert_eq!(
        gemm_retired_floor(512, 512, 512, t.tile, Fidelity::LoFi),
        65_536
    );
    assert_eq!(
        gemm_retired_floor(512, 512, 512, t.tile, Fidelity::HiFi2),
        131_072
    );
    assert_eq!(
        gemm_retired_floor(512, 512, 512, t.tile, Fidelity::HiFi3),
        196_608
    );
    let rows = measured_rows();
    let m = rows
        .iter()
        .find(|m| m.shape == (512, 512, 512))
        .expect("512^3 is measured");
    let wrong = Measured {
        shape: (512, 512, 1024),
        id: m.id.clone(),
        candidate: m.candidate,
        fidelity: m.fidelity,
        fp32: m.fp32,
        l1acc: m.l1acc,
        filter: m.filter,
        work: m.work,
    };
    if let Some((_, p)) = predict(&wrong) {
        assert_ne!(
            p.matmul_retired, m.work.matmul_retired,
            "twice the reduction depth must not predict the same retired work"
        );
        assert!(
            rel(p.noc_bytes, m.work.noc_bytes) > BYTE_TOLERANCE,
            "a program of twice the depth must not be predicted to move the same bytes"
        );
    }
}

#[test]
fn the_closed_form_agrees_with_the_choreography_it_summarizes() {
    // ★ THE MIDDLE LINK. The plan search cannot build its candidates — there are 4,209 at 512^3 — so it
    // ranks them with `work_of_plan`, a closed form over the plan. That form is only trustworthy if it
    // equals what the built choreography actually does, and `work_of` is in turn checked against the
    // machine by `the_model_predicts_the_work_the_machine_actually_did`. Together the three make one
    // chain: closed form == choreography == simulator. Break any link and the planner is guessing.
    let target = SpatialTarget {
        sites: MeshShape { x: 8, y: 8 },
        ..SpatialTarget::wormhole_ttsim()
    };
    let mut checked = 0;
    let mut worst = (0.0f64, String::new());
    for (m, n, k) in [
        (128usize, 128usize, 128usize),
        (256, 256, 256),
        (512, 512, 512),
        (640, 640, 640),
        (2048, 1024, 512),
    ] {
        let src = gemm_program(m, n, k);
        for (kind, built) in calibration_candidates_matching(
            &src,
            &target,
            NumericsPolicy::AllowSpillRounding,
            PlanFilter::default(),
        )
        .expect("the candidates build")
        {
            let walked = work_of(&built, &target);
            let plan = built
                .work
                .plan
                .expect("a built program carries the plan it was tiled with");
            let closed = work_of_plan(&built, &plan, built.summary().multicast_groups > 0, &target)
                .expect("a GEMM is priced");
            assert_eq!(
                closed.matmul_retired, walked.matmul_retired,
                "{m}x{n}x{k} {kind:?}: the closed form and the choreography disagree on retired work — {} vs {}",
                closed.matmul_retired, walked.matmul_retired
            );
            for (axis, c, w) in [
                ("noc_bytes", closed.noc_bytes, walked.noc_bytes),
                ("noc_txns", closed.noc_txns, walked.noc_txns),
                ("dram_bytes", closed.dram_bytes, walked.dram_bytes),
                (
                    "site_noc_bytes",
                    closed.site_noc_bytes,
                    walked.site_noc_bytes,
                ),
                (
                    "engine_noc_bytes",
                    closed.engine_noc_bytes,
                    walked.engine_noc_bytes,
                ),
            ] {
                let e = rel(c, w);
                if e > worst.0 {
                    worst = (
                        e,
                        format!("{m}x{n}x{k} {kind:?} {axis}: closed {c} walked {w}"),
                    );
                }
                assert!(
                    e <= CLOSED_FORM_TOLERANCE,
                    "{m}x{n}x{k} {kind:?}: the closed form says {axis} is {c} and the built choreography does {w} ({:.1}% apart). The planner ranks by the closed form, so a gap here is a plan chosen on a number the compiler does not honour.",
                    e * 100.0
                );
            }
            checked += 1;
        }
    }
    assert!(checked >= 12, "only {checked} candidates compared");
    eprintln!(
        "closed form vs choreography: {checked} candidates, worst {:.2}% ({})",
        worst.0 * 100.0,
        worst.1
    );
}

#[test]
fn the_pipeline_chooses_a_plan_no_measured_plan_dominates() {
    // ★ THE SUCCESSOR to `spatial_cost_model_lock::the_model_reproduces_the_measured_decisions`, stated
    // against a meter that measures the program.
    //
    // Not "the compiler minimizes bytes" — that is the wrong obligation and the ledger says so. At 128^3
    // the fewest-bytes plan the sweep ever measured puts the whole problem on TWO sites: 131,092 bytes,
    // and sixty-two cores idle. Minimizing one axis buys it by giving up another, which is exactly what
    // a frontier means. So the obligation is DOMINANCE: no measured plan may beat the compiler's choice
    // on every axis at once. A plan that wins on bytes by losing on spread has not beaten it, and a
    // compiler that is never dominated is on the frontier.
    let rows = measured_rows();
    let mut by_shape: std::collections::BTreeMap<(usize, usize, usize), Vec<&Measured>> =
        Default::default();
    for m in rows.iter().filter(|m| !m.fp32 && !m.l1acc) {
        by_shape.entry(m.shape).or_default().push(m);
    }
    let mut checked = 0;
    for (shape, ms) in &by_shape {
        if ms.len() < 2 {
            continue;
        }
        let target = SpatialTarget {
            sites: MeshShape { x: 8, y: 8 },
            ..SpatialTarget::wormhole_ttsim()
        };
        let Ok(chosen) = compile_to_spatial_with(
            &gemm_program(shape.0, shape.1, shape.2),
            &target,
            NumericsPolicy::AllowSpillRounding,
        ) else {
            continue;
        };
        let mine = work_of(&chosen, &target);
        checked += 1;
        for m in ms {
            let Some((_, theirs)) = predict(m) else {
                continue;
            };
            // Strictly better on EVERY axis, by more than the model's own accuracy, is domination.
            let beats = |a: u64, b: u64| a * 100 < b * 99;
            assert!(
                !(beats(theirs.dram_bytes, mine.dram_bytes)
                    && beats(theirs.site_noc_bytes, mine.site_noc_bytes)
                    && beats(theirs.noc_txns, mine.noc_txns)),
                "{shape:?}: {} is better than the compiler's choice on EVERY axis — DRAM {} vs {}, busiest site {} vs {}, transactions {} vs {}. The compiler is off the frontier.",
                m.id,
                theirs.dram_bytes,
                mine.dram_bytes,
                theirs.site_noc_bytes,
                mine.site_noc_bytes,
                theirs.noc_txns,
                mine.noc_txns
            );
        }
    }
    assert!(
        checked >= 3,
        "the ledger must hold competing measured plans at several shapes: {checked}"
    );
}

#[test]
fn the_compiler_ranks_plans_in_exactly_one_place() {
    // ★ THE BUG THIS EXISTS TO PREVENT, because it happened. `tile::plan_key` was changed to rank by
    // predicted work, it was verified, and every ledger row kept coming back with the OLD plan — because
    // `pipeline::calibration_candidates_matching` carried its own copy of the ranking, still keyed on
    // `c.cycles`. That is the path `emit_spatial_tensix` takes, so it is the path every measurement takes,
    // and the compiler was quietly ranking by a counter that does not track work for another whole
    // campaign. Two rankings is one too many and the second one is invisible.
    //
    // So: under `spatial/`, the cycle model may be CONSULTED (it still sizes buffers, where a rate model
    // belongs) but it may not ORDER candidates. Ordering goes through `Objective::key`.
    let dir = repo_root().join("crates/logicaffeine_compile/src/spatial");
    let mut ranked_by_cycles = Vec::new();
    let mut ranking_sites = Vec::new();
    let mut walk = vec![dir];
    while let Some(d) = walk.pop() {
        for e in std::fs::read_dir(&d).expect("the spatial module") {
            let p = e.expect("a dir entry").path();
            if p.is_dir() {
                walk.push(p);
                continue;
            }
            if p.extension().and_then(|x| x.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&p).expect("a source file");
            let rel = p
                .strip_prefix(repo_root())
                .unwrap_or(&p)
                .display()
                .to_string();
            for (i, line) in text.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                let orders = code.contains("min_by_key")
                    || code.contains("max_by_key")
                    || code.contains("sort_by_key");
                if orders {
                    ranking_sites.push(format!("{rel}:{}", i + 1));
                }
                // A comparator that reaches for the cycle model is the defect, wherever it lives.
                if orders && (code.contains(".cycles") || code.contains("cycle.cycles")) {
                    ranked_by_cycles.push(format!("{rel}:{}: {}", i + 1, line.trim()));
                }
            }
            // `plan_key` is the one comparator; it must go through the objective.
            if rel.ends_with("passes/tile.rs") {
                assert!(
                    text.contains("objective.key(&c.work)"),
                    "{rel}: the plan comparator must rank by the objective over predicted work"
                );
            }
        }
    }
    assert!(
        ranked_by_cycles.is_empty(),
        "a plan comparator still orders by the cycle model, which ttsim's own audit shows is not a function of the program:\n  {}",
        ranked_by_cycles.join("\n  ")
    );
    // Non-vacuity: the scan must actually be finding comparators, or it proves nothing.
    assert!(ranking_sites.len() >= 3, "the scan found only {} ordering sites under spatial/ — it has stopped looking: {ranking_sites:?}", ranking_sites.len());
}

/// Every count of a work summary, named.
fn counts(w: &Work) -> [(&'static str, u64); 15] {
    [
        ("matmul_retired", w.matmul_retired),
        ("noc_txns", w.noc_txns),
        ("noc_bytes", w.noc_bytes),
        ("payload_bytes", w.payload_bytes),
        ("tensix_inst", w.tensix_inst),
        ("compute_calls", w.compute_calls),
        ("engine_operand_tiles", w.engine_operand_tiles),
        ("sites_used", w.sites_used),
        ("site_noc_bytes", w.site_noc_bytes),
        ("engine_noc_bytes", w.engine_noc_bytes),
        ("site_noc_txns", w.site_noc_txns),
        ("site_matmul_retired", w.site_matmul_retired),
        ("dram_bytes", w.dram_bytes),
        ("dram_tiles", w.dram_tiles),
        ("onchip_bytes", w.onchip_bytes),
    ]
}

/// At most `per` candidates of every arrangement the plan search offers — fused or not, streamed or blocked,
/// ragged or even, its engine packing, its packer accumulate, its sub-block — a stride through each, so the
/// sample is deterministic and spread over the lattice.
fn every_arrangement(cands: &[Candidate], per: usize) -> Vec<&Candidate> {
    let mut groups: std::collections::BTreeMap<_, Vec<&Candidate>> = Default::default();
    for c in cands {
        let b = c.plan.block;
        let key = (
            c.fused,
            is_streaming(&c.plan),
            c.ragged,
            c.plan.engines == EnginePacking::OperandWithStore,
            c.plan.pack_accumulate,
            (b.osh, b.osw, b.ibw),
        );
        groups.entry(key).or_default().push(c);
    }
    groups
        .into_values()
        .flat_map(|g| {
            let stride = g.len().div_ceil(per).max(1);
            g.into_iter().step_by(stride).collect::<Vec<_>>()
        })
        .collect()
}

/// ★ THE MIDDLE LINK, ON EVERY TARGET AND EXACT. The plan search ranks its candidates by `Candidate::work`, the
/// closed form, and `build_candidate` emits the one it picks. Every count the closed form gives must be the
/// count the built choreography makes, to the byte: a gap is a plan chosen on a number the compiler does not
/// honour, and a tolerance hides it. Both numerics, because exact numerics write a wider result.
fn the_closed_form_is_the_built_choreography(t: &SpatialTarget, shapes: &[(usize, usize, usize)]) {
    let mut compared = 0;
    for &(m, n, k) in shapes {
        for numerics in [NumericsPolicy::Exact, NumericsPolicy::AllowSpillRounding] {
            let template = template_from_source_with(&gemm_program(m, n, k), t, numerics)
                .unwrap_or_else(|e| panic!("{} {m}x{n}x{k}: {e:?}", t.name));
            let cands = candidates(&template, t);
            for c in every_arrangement(&cands, 12) {
                let built = build_candidate(&template, c, t).unwrap_or_else(|e| {
                    panic!(
                        "{} {m}x{n}x{k}: the candidate {:?} does not build: {e:?}",
                        t.name, c.plan
                    )
                });
                let walked = work_of(&built, t);
                let apart: Vec<String> = counts(&c.work)
                    .into_iter()
                    .zip(counts(&walked))
                    .filter(|((_, closed), (_, built))| closed != built)
                    .map(|((axis, closed), (_, built))| format!("{axis} {closed} against {built}"))
                    .collect();
                assert!(
                    apart.is_empty(),
                    "{} {m}x{n}x{k} {numerics:?}, fused {}, {:?}: the planner's closed form is not the choreography it builds: {}",
                    t.name,
                    c.fused,
                    c.plan,
                    apart.join(", ")
                );
                compared += 1;
            }
        }
    }
    assert!(
        compared >= 20,
        "{}: only {compared} candidates compared",
        t.name
    );
}

#[test]
fn the_closed_form_is_the_built_choreography_on_wormhole() {
    the_closed_form_is_the_built_choreography(
        &SpatialTarget::wormhole_ttsim(),
        &[(128, 128, 128), (256, 256, 256), (320, 192, 96)],
    );
}

#[test]
fn the_closed_form_is_the_built_choreography_on_blackhole() {
    the_closed_form_is_the_built_choreography(
        &SpatialTarget::blackhole(),
        &[(128, 128, 128), (256, 256, 256)],
    );
}

#[test]
fn the_closed_form_is_the_built_choreography_on_the_rtx3060() {
    the_closed_form_is_the_built_choreography(
        &SpatialTarget::rtx3060(),
        &[(128, 128, 128), (512, 512, 512)],
    );
}

#[test]
fn the_closed_form_is_the_built_choreography_on_the_rdus() {
    the_closed_form_is_the_built_choreography(
        &SpatialTarget::sn50_rdu(),
        &[(128, 128, 128), (256, 256, 256)],
    );
    the_closed_form_is_the_built_choreography(&SpatialTarget::sn40l_rdu(), &[(128, 128, 128)]);
}

#[test]
fn the_closed_form_is_the_built_choreography_on_the_fabric() {
    the_closed_form_is_the_built_choreography(
        &SpatialTarget::sky_fabric(16, 16),
        &[(4, 4, 4), (8, 6, 10), (16, 16, 8), (12, 20, 9)],
    );
}

/// ★ WHAT ONE COMPUTE OP ASKS OF EACH MATRIX ENGINE, by its own sub-block API, at the numbers its source
/// states: Tensix's unpacker (`llk_unpack_AB_matmul`), the fabric's systolic array (its cycle twin), the GPU's
/// `wmma` loop (the CUDA lowering), and a tile pair at a time where an engine has no sub-block API.
#[test]
fn each_matrix_engine_counts_its_calls_and_operand_loads_by_its_own_api() {
    use logicaffeine_compile::spatial::ir::Granularity::{PerSubBlock, PerTile};
    let b = |osh, osw, ibw| BlockShape { osh, osw, ibw };
    let tensix = SpatialTarget::wormhole_ttsim();
    assert_eq!(
        (
            tensix.matmul_calls(b(2, 2, 3), PerSubBlock),
            tensix.operand_loads(b(2, 2, 3), PerSubBlock)
        ),
        (3, 18),
        "Tensix 2x2: a call per inner step, two held tiles and four re-loaded each"
    );
    assert_eq!(
        (
            tensix.matmul_calls(b(1, 4, 3), PerSubBlock),
            tensix.operand_loads(b(1, 4, 3), PerSubBlock)
        ),
        (3, 15),
        "Tensix 1x4: one held, four re-loaded — fewer than the square of the same area"
    );
    assert_eq!(
        tensix.operand_loads(b(4, 1, 3), PerSubBlock),
        15,
        "Tensix holds the shorter side, whichever operand it is"
    );
    let fabric = SpatialTarget::sky_fabric(16, 16);
    assert_eq!(
        (
            fabric.matmul_calls(b(4, 4, 8), PerSubBlock),
            fabric.operand_loads(b(4, 4, 8), PerSubBlock)
        ),
        (1, 64),
        "the fabric's array: one call over the inner block, A's 4x8 panel and B's 8x4"
    );
    assert_eq!(
        (
            fabric.matmul_calls(b(1, 16, 8), PerSubBlock),
            fabric.operand_loads(b(1, 16, 8), PerSubBlock)
        ),
        (1, 136),
        "a 1x16 sub-block of the same area loads more than twice the operands"
    );
    let gpu = SpatialTarget::rtx3060();
    assert_eq!(
        (
            gpu.matmul_calls(b(4, 4, 2), PerSubBlock),
            gpu.operand_loads(b(4, 4, 2), PerSubBlock)
        ),
        (2, 48),
        "the GPU's eight warps split 4x4 on a 2x4 grid: 24 fragments a step"
    );
    assert_eq!(
        gpu.operand_loads(b(8, 4, 1), PerSubBlock),
        32,
        "8x4 on a 4x2 grid: each warp loads two A and two B fragments"
    );
    assert_eq!(
        gpu.operand_loads(b(1, 1, 1), PerSubBlock),
        2,
        "a sub-block no warp grid divides loads two fragments per output tile"
    );
    let rdu = SpatialTarget::sn50_rdu();
    for t in [&tensix, &fabric, &gpu, &rdu] {
        assert_eq!(
            (
                t.matmul_calls(b(2, 3, 4), PerTile),
                t.operand_loads(b(2, 3, 4), PerTile)
            ),
            (24, 48),
            "{}: a tile pair per call",
            t.name
        );
    }
    assert_eq!(
        (
            rdu.matmul_calls(b(2, 3, 4), PerSubBlock),
            rdu.operand_loads(b(2, 3, 4), PerSubBlock)
        ),
        (24, 48),
        "an engine with no sub-block API runs a sub-block a tile pair at a time"
    );
}
