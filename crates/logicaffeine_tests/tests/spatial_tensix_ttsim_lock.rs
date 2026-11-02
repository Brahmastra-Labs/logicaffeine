//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! TENSIX ttsim LOCK — the IR-lowered Tensix program, MEASURED on Tenstorrent's simulator, ties or beats
//! every program we ever emitted and every tt-metal example at matched numerics — and the compiler's own
//! plan choice is the measured winner.
//!
//! Every number here is a row of the ttsim GEMM ledger (`benchmarks/silicon/tenstorrent_gemm.json`),
//! recorded by `scripts/tt-matmul-measure.sh spatial …` from a real run log. The harness runs THROUGH the
//! measure script under an exclusive lock (one simulator, one build tree). This lock proves, per shape:
//!   A. the compiler's chosen program PASSES (PCC ≥ its floor) and its recorded plan is the plan the
//!      compiler emits today, and the recorded program hash IS the hash of today's emitted bundle — no
//!      number can be recorded from a different program;
//!   B. NOT DOMINATED on the WORK meter — NOC bytes, transactions and instructions — against every other
//!      LOGOS-emitted row at the same shape, knobs and RETIRED ARITHMETIC (a comparison at unequal retired
//!      work is not a comparison). It used to require the fewest CYCLES, which `ttsim_instrument_audit.json`
//!      shows is not a function of the program at all — 8x the reduction depth moves that counter 0.08%.
//!   C. the DECISION was right: of the calibration candidates (streamed / fused / unfused) measured at the
//!      same knobs, none that the compiler passed over is better than its choice on every axis at once;
//!   D. every kernel of the measured program was generated (`compute,dataflow,host`), no vendor kernel.
//!
//! Gated on the simulator (`require_tool("ttsim")`; `LOGOS_REQUIRE_TOOLS=ttsim` makes absence fail loud).
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(spatial_tensix_ttsim_lock)'`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/spatial.rs"]
mod spatial_support;
#[path = "common/tt_ledger.rs"]
mod tt_ledger;

use common::ttsim::{
    measure_spatial, program_sha256, recorded, recording_ledger, Knobs, COMMITTED_LEDGER,
};
use logicaffeine_compile::spatial::ir::NumericsPolicy;
use logicaffeine_compile::spatial::lower::tensix::{bundle_files, TensixBackend};
use logicaffeine_compile::spatial::lower::SpatialBackend;
use logicaffeine_compile::spatial::pipeline::{
    calibration_candidates, compile_to_spatial_with, CandidateKind,
};
use logicaffeine_compile::spatial::target::SpatialTarget;
use spatial_support::gemm_program;

const CHIP: &str = "wormhole_b0";

/// Whether a ledger row records that EVERY kernel of the measured program was generated.
///
/// ★ THE FIELD HAS TWO SHAPES AND THE OLDER READER SAW NEITHER. Today's rows store an object —
/// `{"compute": true, "dataflow": true, "host": true}` (140 of them) — while the check read
/// `generated["parts"]` or `generated` AS A STRING and asked whether it contained `"dataflow"`.
/// Both `as_str()` calls answer `None` on an object, `unwrap_or("")` turns that into the empty
/// string, and `"".contains("dataflow")` is false. So the assertion whose message says "every
/// kernel generated" was measuring whether a string that does not exist contains a substring.
///
/// ★ AND IT COULD NOT TELL THREE DIFFERENT SITUATIONS APART: a format it does not understand, a
/// row with no `generated` field at all (62 rows are `null`), and a program that genuinely used a
/// vendor kernel all produced the same `false`. That is the failure this lock exists to catch,
/// hidden behind the same answer as a schema change.
fn every_kernel_generated(generated: &serde_json::Value) -> Result<(), String> {
    // The current shape: one bool per kernel kind, all of which must be true.
    if let Some(map) = generated.as_object() {
        let missing: Vec<&str> = ["compute", "dataflow", "host"]
            .into_iter()
            .filter(|k| map.get(*k).and_then(serde_json::Value::as_bool) != Some(true))
            .collect();
        return if missing.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "a vendor kernel was used for: {missing:?} — {generated}"
            ))
        };
    }
    // The legacy shape: a comma-separated list, either bare or under `parts`.
    let listed = generated
        .get("parts")
        .and_then(serde_json::Value::as_str)
        .or_else(|| generated.as_str());
    match listed {
        Some(text) => {
            let missing: Vec<&str> = ["compute", "dataflow", "host"]
                .into_iter()
                .filter(|k| !text.contains(*k))
                .collect();
            if missing.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "a vendor kernel was used for: {missing:?} — {text:?}"
                ))
            }
        }
        // ★ NOT `false`. A row that does not say cannot be read as a row that says no — that is the
        // conflation which let a schema change masquerade as a vendor-kernel regression.
        None => Err(format!(
            "this row does not record which kernels were generated (`generated` is {generated}) — \
             re-measure it through scripts/tt-matmul-measure.sh rather than reading the silence \
             as either answer"
        )),
    }
}

fn shape_key(shape: (usize, usize, usize)) -> String {
    format!("{}x{}x{}", shape.0, shape.1, shape.2)
}

/// Current ledger rows at (shape, chip, knobs).
fn rows_at(shape: (usize, usize, usize), knobs: Knobs) -> Vec<serde_json::Value> {
    let d = recorded();
    tt_ledger::current_rows(&d)
        .into_iter()
        .filter(|r| {
            r["chip"] == CHIP
                && tt_ledger::shape_str(r) == shape_key(shape)
                && tt_ledger::knobs_str(&r["knobs"]) == knobs.ledger_key()
        })
        .collect()
}

fn spatial_row(rows: &[serde_json::Value], candidate: &str) -> Option<serde_json::Value> {
    rows.iter()
        .find(|r| r["program"] == "logos_spatial" && r["strategy"] == candidate)
        .cloned()
}

/// The row the ledger names as the compiler's own choice at (shape, knobs): `frontier[].compiler_chosen`, the
/// most recent current row measured with nothing pinned. A strategy label is not an identity — one knob set can
/// hold the compiler's row and a pinned calibration row of the same arrangement.
fn compiler_chosen_row(shape: (usize, usize, usize), knobs: Knobs) -> Option<serde_json::Value> {
    let d = recorded();
    let entry = d["frontier"].as_array()?.iter().find(|f| {
        f["chip"] == CHIP
            && f["shape"] == shape_key(shape).as_str()
            && f["knobs"] == knobs.ledger_key().as_str()
    })?;
    let id = entry["compiler_chosen"]["row"].as_str()?;
    tt_ledger::current_rows(&d)
        .into_iter()
        .find(|r| r["id"] == id)
}

fn numerics(knobs: Knobs) -> NumericsPolicy {
    if knobs.fp32 {
        NumericsPolicy::Exact
    } else {
        NumericsPolicy::AllowSpillRounding
    }
}

/// Measure every calibration candidate at `shape` under `knobs`, then assert A–D.
fn measure_and_assert(shape: (usize, usize, usize), knobs: Knobs) {
    if !common::require_tool("ttsim") {
        return;
    }
    let wh = SpatialTarget::wormhole_ttsim();
    let src = gemm_program(shape.0, shape.1, shape.2);
    let chosen = compile_to_spatial_with(&src, &wh, numerics(knobs)).unwrap();
    let chosen_label = chosen.summary().arrangement_label();
    let candidates = calibration_candidates(&src, &wh, numerics(knobs)).unwrap();
    assert!(
        candidates.iter().any(|(k, _)| *k == CandidateKind::Best),
        "the compiler's choice is among the candidates"
    );

    measure_spatial(shape, "wormhole", knobs).unwrap_or_else(|e| panic!("{e}"));
    let rows = rows_at(shape, knobs);
    let best = compiler_chosen_row(shape, knobs).unwrap_or_else(|| panic!("the ledger names no compiler-chosen row at {shape:?} {knobs:?} (`{chosen_label}`): {rows:?}"));
    assert_eq!(
        best["strategy"], chosen_label,
        "the compiler-chosen row is today's arrangement"
    );

    // A. pass, recorded plan == today's plan, recorded hash == today's bundle.
    assert_eq!(best["pass"], true, "{best}");
    assert!(
        best["pcc"].as_f64().unwrap() >= best["pcc_floor"].as_f64().unwrap(),
        "{best}"
    );
    let art = match TensixBackend::new(wh.clone()).lower(&chosen).unwrap() {
        logicaffeine_compile::spatial::lower::SpatialArtifact::Tensix(a) => a,
        other => panic!("{other:?}"),
    };
    let files = bundle_files(&art);
    assert_eq!(
        best["program_sha256"],
        program_sha256(&files),
        "the ledger row must be the hash of TODAY's emitted bundle"
    );
    let s = chosen.summary();
    let cfg = &best["cfg"];
    assert_eq!(
        cfg["grid"],
        format!("{}x{}", s.grid.1, s.grid.0),
        "recorded grid == emitted grid (x×y)"
    );
    assert_eq!(
        cfg["per_core"],
        format!("{}x{}", s.per_site_block.0, s.per_site_block.1)
    );
    every_kernel_generated(&best["generated"]).unwrap_or_else(|why| {
        panic!("every kernel of the measured program must be generated: {why}")
    });

    // B + C. ★ ON THE WORK METER, NOT THE CLOCK. These used to require the chosen program to have the
    // fewest `cycles` of every measured alternative. That counter is not a function of the program —
    // `ttsim_instrument_audit.json` sweeps the reduction depth 8x and watches it move 0.08% — so the
    // obligation was "agree with a coin", and at 512^3 the coin named the plan that issues 2.9x the
    // instructions. What the simulator CAN report is what the machine did, and that is what a compiler
    // may be held to.
    //
    // The form is DOMINANCE, for the same reason the frontier exists: a plan that moves fewer bytes by
    // pushing everything through two sites has not beaten one that spreads across sixty-four. What the
    // compiler may never do is pick a plan another measured plan beats on every axis at once.
    let work_of_row = |r: &serde_json::Value| -> Option<(u64, u64, u64, u64)> {
        let w = r.get("work")?;
        Some((
            w["matmul_retired"].as_u64()?,
            w["noc_bytes"].as_u64()?,
            w["noc_txns"].as_u64()?,
            w["tensix_inst"].as_u64()?,
        ))
    };
    let Some((mine_ret, mine_b, mine_t, mine_i)) = work_of_row(&best) else {
        panic!("{shape:?} {knobs:?}: the chosen row carries no work meter — re-measure on the instrumented simulator");
    };
    let mut compared = 0;
    for r in &rows {
        if r["id"] == best["id"] || r["stack"] == "ttnn_production" {
            continue;
        }
        let Some((ret, b, t, i)) = work_of_row(r) else {
            continue;
        };
        if ret != mine_ret {
            continue;
        }
        compared += 1;
        assert!(
            !(b < mine_b && t < mine_t && i < mine_i),
            "{shape:?} {knobs:?}: the IR-lowered program is beaten on EVERY axis by {} at identical retired work ({ret}) — bytes {b} vs {mine_b}, transactions {t} vs {mine_t}, instructions {i} vs {mine_i}",
            r["id"]
        );
    }
    // And the compiler's own choice must not be dominated by a candidate it passed over.
    for (kind, _) in &candidates {
        let label = kind.label();
        if label == chosen_label {
            continue;
        }
        let Some(r) = spatial_row(&rows, label) else {
            continue;
        };
        let Some((ret, b, t, i)) = work_of_row(&r) else {
            continue;
        };
        if ret != mine_ret {
            continue;
        }
        compared += 1;
        assert!(
            !(b < mine_b && t < mine_t && i < mine_i),
            "{shape:?} {knobs:?}: the compiler chose `{chosen_label}` but the `{label}` candidate it passed over is better on every axis — bytes {b} vs {mine_b}, transactions {t} vs {mine_t}, instructions {i} vs {mine_i}"
        );
    }
    eprintln!("{shape:?} {knobs:?}: `{chosen_label}` not dominated by any of {compared} measured alternatives (bytes {mine_b}, txns {mine_t}, instrs {mine_i}, retired {mine_ret})");
}

#[test]
fn streamed_640_cubed_exact_ties_or_beats_every_recorded_program() {
    measure_and_assert(
        (640, 640, 640),
        Knobs {
            fidelity: "HiFi4",
            fp32: true,
            l1acc: false,
        },
    );
}

#[test]
fn bf16_packing_640_cubed_ties_or_beats_every_recorded_program() {
    measure_and_assert(
        (640, 640, 640),
        Knobs {
            fidelity: "HiFi4",
            fp32: false,
            l1acc: false,
        },
    );
}

#[test]
fn multicast_2048x1024x512_exact_ties_or_beats_every_recorded_program() {
    measure_and_assert(
        (2048, 1024, 512),
        Knobs {
            fidelity: "HiFi4",
            fp32: true,
            l1acc: false,
        },
    );
}

#[test]
fn multicast_2048x2048x512_exact_ties_or_beats_every_recorded_program() {
    measure_and_assert(
        (2048, 2048, 512),
        Knobs {
            fidelity: "HiFi4",
            fp32: true,
            l1acc: false,
        },
    );
}

#[test]
fn multicast_2048x2048x1024_exact_with_a_spill_ties_or_beats_every_recorded_program() {
    // The exact reduction splits K and finishes in the output's own page, which is the packer's accumulate.
    measure_and_assert(
        (2048, 2048, 1024),
        Knobs {
            fidelity: "HiFi4",
            fp32: true,
            l1acc: true,
        },
    );
}

#[test]
fn the_ledger_holds_no_vendor_kernel_program_for_the_spatial_stack() {
    let d = tt_ledger::ledger();
    for r in tt_ledger::current_rows(&d) {
        if r["program"] == "logos_spatial" {
            let origin = r["program_origin"].as_str().unwrap_or("");
            assert!(
                origin.contains("all kernels generated"),
                "{}: {origin}",
                r["id"]
            );
            assert!(!origin.contains("stock"), "{}: {origin}", r["id"]);
        }
    }
}

/// A test run records into its own copy of the ledger, seeded from the committed one, and never rewrites the
/// committed file: recording there is the explicit `LOGOS_RECORD_TT_LEDGER=1`. The copy verifies where it lies —
/// the recorder re-verifies the whole ledger after every add, each row against the excerpt it cites beside the
/// ledger, so a copy without its excerpts refuses every measurement.
#[test]
fn a_test_run_records_into_its_own_copy_of_the_ledger() {
    if std::env::var_os("LOGOS_RECORD_TT_LEDGER").is_some_and(|v| v == "1") {
        return;
    }
    let committed = common::ttsim::repo_root().join(COMMITTED_LEDGER);
    let recording = recording_ledger();
    assert_ne!(
        recording, committed,
        "a test run must not record into the committed ledger"
    );
    assert!(
        recording.starts_with(common::ttsim::repo_root().join("logs")),
        "{}",
        recording.display()
    );
    let ids = |d: &serde_json::Value| -> std::collections::BTreeSet<String> {
        d["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap_or("").to_string())
            .collect()
    };
    let committed_rows =
        ids(&serde_json::from_str(&std::fs::read_to_string(&committed).unwrap()).unwrap());
    let recorded_rows = ids(&recorded());
    let missing: Vec<_> = committed_rows.difference(&recorded_rows).collect();
    assert!(
        missing.is_empty(),
        "the recording ledger is seeded with every committed row: missing {missing:?}"
    );
    let verify = std::process::Command::new(common::python_for_tools())
        .arg(common::ttsim::repo_root().join("scripts/tt-gemm-ledger.py"))
        .args(["verify", "--ledger"])
        .arg(&recording)
        .output()
        .expect("run scripts/tt-gemm-ledger.py verify");
    assert!(
        verify.status.success(),
        "the recording ledger must verify where it lies, as the recorder re-verifies it after each add:\n{}{}",
        String::from_utf8_lossy(&verify.stdout),
        String::from_utf8_lossy(&verify.stderr)
    );
}
