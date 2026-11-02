//! ═══════════════════════════════════════════════════════════════════════
//! TENSTORRENT ttsim LEDGER LOCK — every Tenstorrent GEMM cycle number the repo quotes is a row of
//! `benchmarks/silicon/tenstorrent_gemm.json`, written only by `scripts/tt-gemm-ledger.py` from a ttsim
//! run, and every comparison is recomputed HERE from the rows — no number is typed, no numerics are
//! conflated, and the honesty labels cannot drift.
//!
//! A. Every row is `sim_functional_not_timed`, self-consistent (key/id/knobs/pcc >= floor), and backed by a
//!    committed ttsim log excerpt that re-parses to the same cycles, knobs, and PCC.
//! B. Every `comparisons` ratio is recomputed from its two rows; matched-numerics only; same simulator.
//! C. PARITY: the compiler-emitted program is at or below tt-metal's EXAMPLE at matched knobs.
//! D. RATCHET: the matched-knobs ratio of the emitted program against ttnn's PRODUCTION op only shrinks
//!    (a regression goes RED; a win past the constant demands tightening it — the constant can never lag).
//! E. ANTI-CONFOUND: cross-knob pairs are never a comparison; they are `unmatched_comparisons` with the
//!    knob diff spelled out (the LoFi-with-config vs HiFi2-default vs our-HiFi4 mistake, made impossible).
//! F. A tt-metal example row can only cite a shape its source can run (no 2048-class run labeled 640^3).
//!
//! Honest boundary: ttsim reports cycles, never watts; nothing here is a silicon measurement.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(tt_sim_lock)'`
//! ═══════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/tt_ledger.rs"]
mod tt_ledger;
use tt_ledger::*;

/// The matched-knobs ratchet against ttnn's PRODUCTION op (subject cycles / ttnn cycles). Set to the
/// measured truth rounded UP to two decimals; a measurement above it is a regression (RED), a measurement
/// more than 0.10 below it means the truth moved ahead of the ratchet (RED: tighten the constant).
// 2026-09-02, 640^3 HiFi4 / fp32-acc off / L1-acc off, every row JIT-warm: the fastest emitted program is the
// forced-tiling multicast host at 9,394 vs ttnn's default heuristic 7,554 (= 1.2436) and ttnn handed our
// config 7,075 (= 1.3278). Rounded UP to two decimals; shrink-only.
const RATCHET_LOGOS_OVER_TTNN_DEFAULT_MAX: f64 = 0.74;
const RATCHET_LOGOS_OVER_TTNN_OUR_CONFIG_MAX: f64 = 0.79;

/// `compiler` when nothing about the PLAN was pinned on the command line, else `pinned`; vendor rows are
/// `vendor`. Mirrors `plan_source` in `scripts/tt-gemm-ledger.py` — the two must agree about what counts
/// as the compiler's own choice, so they are written the same way and named the same thing.
fn plan_source(r: &serde_json::Value) -> &'static str {
    if r["stack"] != "logos_emitted" {
        return "vendor";
    }
    let repro = r["reproduce"].as_str().unwrap_or_default();
    let forced_candidate = ["streamed", "fused", "unfused"]
        .iter()
        .any(|k| repro.contains(&format!("--candidate {k}")));
    if forced_candidate
        || [
            "--block",
            "--per-core",
            "--packing",
            "--tiling",
            "--strategy",
            "--config ours",
        ]
        .iter()
        .any(|f| repro.contains(f))
    {
        "pinned"
    } else {
        "compiler"
    }
}

fn parse_excerpt(
    text: &str,
) -> (
    Option<u64>,
    Option<(String, bool, bool, f64, f64)>,
    Option<(String, String, String)>,
    bool,
    Option<f64>,
    bool,
    Option<f64>,
) {
    // (heartbeat cycles, full PASS (fid, fp32, l1acc, pcc, floor), banner (chip, so_sha, tt_metal_sha), any ERROR,
    //  example PCC, example Test Passed, `-- PCC =` line value)
    let mut cycles = None;
    let mut pass = None;
    let mut banner = None;
    let mut error = false;
    let mut ex_pcc = None;
    let mut ex_pass = false;
    let mut pcc_line = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('[') {
            if let Some((n, tail)) = rest.split_once(']') {
                if tail.trim_start().starts_with("ERROR:") {
                    error = true;
                } else if tail.contains(" seconds (")
                    && (tail.ends_with("MHz)") || tail.ends_with("KHz)"))
                {
                    cycles = n.parse().ok();
                }
            }
        }
        if let Some(rest) = line.strip_prefix("LOGOS_TT_ENV ") {
            let get = |k: &str| {
                rest.split_whitespace()
                    .find_map(|kv| kv.strip_prefix(&format!("{k}=")).map(|v| v.to_string()))
            };
            banner = Some((
                get("chip").unwrap_or_default(),
                get("so_sha256").unwrap_or_default(),
                get("tt_metal_sha").unwrap_or_default(),
            ));
        }
        if let Some(rest) = line.strip_prefix("LOGOS_MM_PASS ") {
            let get = |k: &str| {
                rest.split_whitespace()
                    .find_map(|kv| kv.strip_prefix(&format!("{k}=")).map(|v| v.to_string()))
            };
            if let (Some(f), Some(fp), Some(l1), Some(p), Some(fl)) = (
                get("fidelity"),
                get("fp32"),
                get("l1acc"),
                get("pcc"),
                get("floor"),
            ) {
                pass = Some((
                    f,
                    fp == "1",
                    l1 == "1",
                    p.parse().unwrap(),
                    fl.parse().unwrap(),
                ));
            }
        }
        if let Some(v) = line.strip_prefix("Metalium vs Golden -- PCC = ") {
            ex_pcc = v.trim().parse().ok();
        }
        if line.trim() == "Test Passed" {
            ex_pass = true;
        }
        if let Some(idx) = line.find("-- PCC = ") {
            pcc_line = line[idx + 9..].trim().parse().ok();
        }
    }
    (cycles, pass, banner, error, ex_pcc, ex_pass, pcc_line)
}

#[test]
fn every_ledger_row_is_labelled_for_what_it_measures_and_is_self_consistent() {
    let d = ledger();
    // `sim_cycle_accurate` was the claim until a work sweep showed the simulator's counters do not advance
    // with device execution. It is functionally faithful and temporally blind, and the provenance now says
    // so — see `clock_validity`, and `no_speed_claim_is_derived_from_a_clock_that_does_not_track_work`.
    assert_eq!(d["provenance"], "sim_functional_not_timed");
    assert_eq!(d["generated_by"], "scripts/tt-gemm-ledger.py");
    assert!(d["_comment"].as_str().unwrap().contains("NEVER hand-edit"));
    let all = rows(&d);
    assert!(
        !all.is_empty(),
        "the ledger has no rows — run scripts/tt-matmul-measure.sh all"
    );
    let mut current_keys = std::collections::HashSet::new();
    for r in &all {
        let id = r["id"].as_str().unwrap();
        for k in [
            "key",
            "current",
            "shape",
            "chip",
            "stack",
            "program",
            "program_origin",
            "strategy",
            "knobs",
            "knobs_source",
            "cycles",
            "pcc",
            "pcc_floor",
            "pass",
            "pass_line",
            "reproduce",
            "ttsim",
            "tt_metal_sha",
            "measured_at",
            "program_sha256",
            "log",
            "log_full_sha256",
            "provenance",
        ] {
            assert!(!r[k].is_null() || k == "pass_line", "row {id} lacks `{k}`");
        }
        assert_eq!(r["provenance"], "sim_functional_not_timed", "row {id}");
        assert_eq!(
            r["pass"], true,
            "row {id} must have passed its golden check"
        );
        assert!(
            ["logos_emitted", "tt_metal_example", "ttnn_production"]
                .contains(&r["stack"].as_str().unwrap()),
            "row {id} stack"
        );
        assert!(
            ["LoFi", "HiFi2", "HiFi3", "HiFi4"].contains(&r["knobs"]["fidelity"].as_str().unwrap()),
            "row {id} fidelity"
        );
        assert!(r["cycles"].as_u64().unwrap() > 0, "row {id} cycles");
        assert!(
            r["pcc"].as_f64().unwrap() >= r["pcc_floor"].as_f64().unwrap(),
            "row {id} pcc below floor"
        );
        assert!(
            r["reproduce"]
                .as_str()
                .unwrap()
                .starts_with("scripts/tt-matmul-measure.sh"),
            "row {id} reproduce must be the harness command"
        );
        assert_eq!(
            r["ttsim"]["so_sha256"].as_str().unwrap().len(),
            64,
            "row {id} simulator sha"
        );
        assert_eq!(
            r["tt_metal_sha"].as_str().unwrap().len(),
            40,
            "row {id} tt-metal sha"
        );
        assert_eq!(
            r["program_sha256"].as_str().unwrap().len(),
            64,
            "row {id} program sha"
        );
        let ma = r["measured_at"].as_str().unwrap();
        assert!(
            ma.len() == 20 && ma.ends_with('Z') && ma.as_bytes()[10] == b'T',
            "row {id} measured_at must be ISO-8601 Z: {ma}"
        );
        let key = format!(
            "{}/{}/{}/{}/{}/{}",
            r["chip"].as_str().unwrap(),
            shape_str(r),
            r["stack"].as_str().unwrap(),
            r["program"].as_str().unwrap(),
            r["strategy"].as_str().unwrap(),
            knobs_str(&r["knobs"])
        );
        assert_eq!(
            r["key"], key,
            "row {id} key must be derived from its fields"
        );
        assert_eq!(id, format!("{key}/{ma}"), "row id must be key/measured_at");
        if r["current"] == true {
            // ★ ONE CURRENT ROW PER (KEY, PLAN SOURCE), not per key. A pinned calibration row and the
            // compiler's own unaided choice are different EXPERIMENTS — one shows what the machine can do
            // when told exactly what to do, the other what the compiler decides — and once the planner got
            // good enough that `--candidate best` and `--candidate fused` reach the same arrangement they
            // started colliding on one key, so whichever ran last erased the other. The record that the
            // compiler CHOSE a plan is what every vendor comparison rests on; it may not be overwritten by
            // a calibration run that happens to agree with it.
            assert!(
                current_keys.insert((key.clone(), plan_source(&r))),
                "two current rows share the key {key} as the same plan source"
            );
        }
        if r["cfg"].is_null() {
            assert!(
                r["stack"] == "tt_metal_example" || r["program_origin"] == ORIGIN_TTNN_DEFAULT,
                "row {id}: cfg may be null only for tt-metal examples and ttnn's default heuristic"
            );
        }
        if r["stack"] == "logos_emitted" {
            assert!(
                r["generated"].is_object(),
                "row {id}: logos rows say which parts the compiler generated"
            );
        }
    }
}

#[test]
fn every_row_is_backed_by_its_committed_ttsim_log_excerpt() {
    let d = ledger();
    for r in rows(&d) {
        let id = r["id"].as_str().unwrap();
        let rel = r["log"].as_str().unwrap();
        assert!(
            rel.starts_with("ttsim_logs/"),
            "row {id}: excerpt must live under benchmarks/silicon/ttsim_logs/"
        );
        let text = read(&format!("benchmarks/silicon/{rel}"));
        let (cycles, pass, banner, error, ex_pcc, ex_pass, pcc_line) = parse_excerpt(&text);
        assert!(!error, "row {id}: its excerpt carries a ttsim ERROR line");
        assert_eq!(
            cycles,
            r["cycles"].as_u64(),
            "row {id}: the excerpt's exit heartbeat must equal the ledger cycles"
        );
        let (chip, so, tm) =
            banner.unwrap_or_else(|| panic!("row {id}: excerpt lacks the LOGOS_TT_ENV banner"));
        assert_eq!(chip, r["chip"], "row {id} chip");
        assert_eq!(so, r["ttsim"]["so_sha256"], "row {id} simulator sha");
        assert_eq!(tm, r["tt_metal_sha"], "row {id} tt-metal sha");
        if r["stack"] == "tt_metal_example" {
            assert!(
                ex_pass,
                "row {id}: example excerpt must carry `Test Passed`"
            );
            assert_eq!(
                ex_pcc,
                r["pcc"].as_f64(),
                "row {id}: example PCC must re-parse"
            );
        } else if let Some((f, fp, l1, p, fl)) = pass {
            assert_eq!(f, r["knobs"]["fidelity"], "row {id} fidelity re-parse");
            assert_eq!(fp, r["knobs"]["fp32_dest_acc"], "row {id} fp32 re-parse");
            assert_eq!(l1, r["knobs"]["packer_l1_acc"], "row {id} l1acc re-parse");
            assert_eq!(Some(p), r["pcc"].as_f64(), "row {id} pcc re-parse");
            assert_eq!(Some(fl), r["pcc_floor"].as_f64(), "row {id} floor re-parse");
            assert_eq!(
                r["knobs_source"],
                "LOGOS_MM_PASS line printed by the program"
            );
        } else {
            // A legacy bare PASS: the knobs came from source (and the row says so); the PCC from the `-- PCC =` line.
            assert!(
                text.lines().any(|l| l.trim() == "LOGOS_MM_PASS"),
                "row {id}: excerpt carries no PASS line"
            );
            assert!(
                r["knobs_source"]
                    .as_str()
                    .unwrap()
                    .starts_with("grepped from"),
                "row {id}: bare PASS rows must source knobs from the program source"
            );
            assert_eq!(
                pcc_line,
                r["pcc"].as_f64(),
                "row {id}: pcc must re-parse from the `-- PCC =` line"
            );
        }
    }
}

#[test]
fn every_comparison_ratio_is_recomputed_from_its_rows() {
    let d = ledger();
    let cur = current_rows(&d);
    let comps = d["comparisons"].as_array().cloned().unwrap_or_default();
    assert!(!comps.is_empty(), "no matched-numerics comparison — record a ttnn or example row at the emitted program's knobs");
    for c in &comps {
        let s = row(&d, c["subject"].as_str().unwrap());
        assert_eq!(
            s["current"], true,
            "comparison subject must be a current row"
        );
        assert_eq!(s["stack"], "logos_emitted");
        assert_eq!(c["matched_numerics"], true);
        assert_eq!(c["subject_cycles"], s["cycles"]);
        assert_eq!(c["knobs"], s["knobs"]);
        assert_eq!(
            c["id"],
            format!(
                "{}/{}/{}",
                s["chip"].as_str().unwrap(),
                shape_str(&s),
                knobs_str(&s["knobs"])
            )
        );
        // The subject is the fastest current logos row at this key.
        let best = cur
            .iter()
            .filter(|r| {
                r["stack"] == "logos_emitted"
                    && r["chip"] == s["chip"]
                    && shape_str(r) == shape_str(&s)
                    && knobs_str(&r["knobs"]) == knobs_str(&s["knobs"])
            })
            .map(|r| r["cycles"].as_u64().unwrap())
            .min()
            .unwrap();
        assert_eq!(
            best,
            s["cycles"].as_u64().unwrap(),
            "comparison subject must be the fastest matched logos row"
        );
        let sc = s["cycles"].as_f64().unwrap();
        let mut refs = vec![s.clone()];
        for (name, origin) in [
            ("ttnn_production_default", ORIGIN_TTNN_DEFAULT),
            ("ttnn_production_our_config", ORIGIN_TTNN_OURS),
        ] {
            let v = &c["vs"][name];
            if v.is_null() {
                continue;
            }
            let o = row(&d, v["row"].as_str().unwrap());
            assert_eq!(o["program_origin"], origin);
            assert_eq!(o["current"], true);
            assert_eq!(
                knobs_str(&o["knobs"]),
                knobs_str(&s["knobs"]),
                "{name}: matched numerics only"
            );
            assert_eq!(
                o["knobs"]["math_approx_mode"],
                s["knobs"]["math_approx_mode"]
            );
            let ratio = sc / o["cycles"].as_f64().unwrap();
            assert!(
                (v["ratio"].as_f64().unwrap() - ratio).abs() < 1e-9,
                "{name}: ratio must be subject/other"
            );
            // The verdict names an ORDERING of cycle counts, not a duration: the clock behind these numbers
            // does not track work, so "faster" would be a claim the measurement cannot support.
            let verdict = if ratio < 1.0 {
                "fewer_cycles"
            } else if ratio == 1.0 {
                "equal_cycles"
            } else {
                "more_cycles"
            };
            assert_eq!(v["verdict"], verdict, "{name} verdict");
            assert_eq!(
                v["verdict_is_time_claim"], false,
                "{name}: a cycle ordering is not a time claim"
            );
            refs.push(o);
        }
        if let Some(ex) = c["vs"]["tt_metal_example"].as_object() {
            for (prog, v) in ex {
                let o = row(&d, v["row"].as_str().unwrap());
                assert_eq!(o["program"], prog.as_str());
                assert_eq!(o["stack"], "tt_metal_example");
                assert_eq!(
                    knobs_str(&o["knobs"]),
                    knobs_str(&s["knobs"]),
                    "example {prog}: matched numerics only"
                );
                let ratio = sc / o["cycles"].as_f64().unwrap();
                assert!((v["ratio"].as_f64().unwrap() - ratio).abs() < 1e-9);
                assert_eq!(v["parity"], ratio <= 1.0);
                refs.push(o);
            }
        }
        let same = refs.iter().all(|r| {
            r["ttsim"]["so_sha256"] == s["ttsim"]["so_sha256"]
                && r["tt_metal_sha"] == s["tt_metal_sha"]
        });
        assert_eq!(c["same_simulator"], same);
        assert!(
            same,
            "comparison {}: every row must come from the same simulator build and tt-metal commit",
            c["id"]
        );
    }
    for sp in d["spatial"].as_array().cloned().unwrap_or_default() {
        let single = row(&d, sp["single_core_row"].as_str().unwrap());
        let subj = row(&d, sp["subject"].as_str().unwrap());
        let speedup = single["cycles"].as_f64().unwrap() / subj["cycles"].as_f64().unwrap();
        assert!(
            (sp["speedup"].as_f64().unwrap() - speedup).abs() < 1e-9,
            "spatial speedup must be single/subject"
        );
        assert_eq!(sp["cross_stack"], single["stack"] != "logos_emitted");
        assert_eq!(
            knobs_str(&single["knobs"]),
            knobs_str(&subj["knobs"]),
            "the spatial speedup is a matched-numerics pair"
        );
    }
    let (hc, hs) = headline(&d);
    assert_eq!(hc["subject"], hs["id"]);
}

#[test]
fn the_emitted_program_is_at_parity_or_better_than_the_tt_metal_example_at_matched_knobs() {
    let d = ledger();
    let mut seen = 0;
    for c in d["comparisons"].as_array().unwrap() {
        if let Some(ex) = c["vs"]["tt_metal_example"].as_object() {
            for (prog, v) in ex {
                seen += 1;
                assert!(
                    c["subject_cycles"].as_u64().unwrap() <= v["cycles"].as_u64().unwrap(),
                    "the emitted program ({} cycles) must not be slower than tt-metal example {prog} ({} cycles) at matched knobs {}",
                    c["subject_cycles"], v["cycles"], knobs_str(&c["knobs"])
                );
            }
        }
    }
    assert!(seen > 0, "no example parity pair recorded at matched knobs");
}

/// This was a performance ratchet: it required `emitted / ttnn` to stay under a constant that could only be
/// tightened. It is not one any more, and the reason is the most important thing this ledger records.
///
/// A work sweep — hold M and N, double K — moves no counter reachable from a kernel. Not the whole-program
/// clock, not either movement engine's wall-clock span, not any of the three compute TRISCs. Host wall-clock
/// rises almost linearly over the same sweep and the program provably does the work, so the simulator is
/// functionally faithful and temporally blind. A ratio between two numbers from a blind clock is not a
/// margin, and ratcheting on it would be ratcheting on noise while looking rigorous.
///
/// So the constants are gone rather than loosened. Loosening would have kept the shape of a performance
/// guarantee we cannot make; deleting it and saying why is the honest move. What replaces it is the guard
/// that actually protects us now: the ledger must DECLARE its clock's blindness, and nothing may present a
/// ratio from that clock as a speed result. When the metric becomes real — on silicon, or on a simulator
/// that models time — `tracks_work` flips, and a genuine ratchet can be reintroduced against it.
#[test]
fn no_speed_claim_is_derived_from_a_clock_that_does_not_track_work() {
    let d = ledger();
    let v = &d["clock_validity"];
    assert!(!v.is_null(), "the ledger must state whether its clock tracks work — silence is how this was missed for a whole campaign");

    let tracks = v["tracks_work"]
        .as_bool()
        .expect("clock_validity.tracks_work is a boolean");
    for key in ["evidence", "consequence", "what_survives"] {
        let s = v[key].as_str().unwrap_or_default();
        assert!(
            s.len() > 40,
            "clock_validity.{key} must carry the reasoning, not a bare flag"
        );
    }

    if tracks {
        // The metric became real. Reintroduce a ratchet against it deliberately — do not let this test pass
        // silently on a working clock, because a working clock is exactly when a ratchet earns its keep.
        panic!(
            "clock_validity.tracks_work is now true: the counter measures work again, so this test must be \
             replaced by a real performance ratchet against it (and `provenance` updated from \
             `sim_functional_not_timed`)"
        );
    }

    // While the clock is blind, no comparison may be dressed as a speed result.
    for c in d["comparisons"]
        .as_array()
        .map(|a| a.as_slice())
        .unwrap_or_default()
    {
        for (name, entry) in c["vs"]
            .as_object()
            .map(|o| o.iter().collect::<Vec<_>>())
            .unwrap_or_default()
        {
            if entry["ratio"].is_null() {
                continue;
            }
            let verdict = entry["verdict"].as_str().unwrap_or_default();
            assert!(
                !verdict.contains("faster"),
                "{}/{name} states the verdict {verdict:?}, which is a claim about TIME, from a clock that \
                 does not track work. Ratios may be recorded for identity and ordering; they may not be \
                 called faster or slower until `clock_validity.tracks_work` is true.",
                c["id"].as_str().unwrap_or("?")
            );
        }
    }
}

#[test]
fn no_comparison_mixes_numerics_silently() {
    let d = ledger();
    let cur = current_rows(&d);
    let logos: Vec<_> = cur
        .iter()
        .filter(|r| r["stack"] == "logos_emitted")
        .collect();
    let others: Vec<_> = cur
        .iter()
        .filter(|r| r["stack"] != "logos_emitted")
        .collect();
    let un = d["unmatched_comparisons"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    // Every cross-knob (logos subject, other) pair at a shared shape/chip is listed as unmatched, and labeled.
    for c in d["comparisons"].as_array().unwrap() {
        let s = row(&d, c["subject"].as_str().unwrap());
        for o in &others {
            if o["chip"] != s["chip"]
                || shape_str(o) != shape_str(&s)
                || knobs_str(&o["knobs"]) == knobs_str(&s["knobs"])
            {
                continue;
            }
            let entry = un
                .iter()
                .find(|u| u["subject"] == s["id"] && u["other"] == o["id"])
                .unwrap_or_else(|| {
                    panic!(
                        "cross-knob pair {} vs {} must be listed in unmatched_comparisons",
                        s["id"], o["id"]
                    )
                });
            assert_eq!(entry["matched_numerics"], false);
            let why = entry["why_unmatched"].as_array().unwrap();
            assert!(
                !why.is_empty(),
                "why_unmatched must spell out the knob diff"
            );
            let ratio = s["cycles"].as_f64().unwrap() / o["cycles"].as_f64().unwrap();
            assert!((entry["ratio"].as_f64().unwrap() - ratio).abs() < 1e-9);
            assert!(entry["note"]
                .as_str()
                .unwrap()
                .contains("NOT a head-to-head"));
        }
    }
    for u in &un {
        assert_eq!(u["matched_numerics"], false);
        let s = row(&d, u["subject"].as_str().unwrap());
        let o = row(&d, u["other"].as_str().unwrap());
        assert_ne!(
            knobs_str(&s["knobs"]),
            knobs_str(&o["knobs"]),
            "an unmatched entry must actually differ in knobs"
        );
    }
    let _ = logos;
}

#[test]
fn no_row_cites_a_shape_its_program_cannot_run() {
    let d = ledger();
    let ves = &d["vendor_example_shapes"];
    for r in rows(&d).iter().filter(|r| r["stack"] == "tt_metal_example") {
        let prog = r["program"].as_str().unwrap();
        let e = &ves[prog];
        assert!(
            e.is_object(),
            "example {prog} must have been scanned (source path:line) before a row cites it"
        );
        let shapes: Vec<&str> = e["shapes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        assert!(
            shapes.contains(&shape_str(r).as_str()),
            "example {prog} cannot run {} (source runs {:?})",
            shape_str(r),
            shapes
        );
        assert!(
            e["source"].as_str().unwrap().contains(".cpp:"),
            "example {prog} scan must cite path:line"
        );
        assert_eq!(
            r["knobs"], e["knobs"],
            "example rows carry the knobs their own source hardcodes"
        );
    }
    // The historical mislabel, pinned forever: the mcast example never runs 640^3.
    if let Some(e) = ves["matmul_multicore_reuse_mcast"].as_object() {
        assert!(!e["shapes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s == "640x640x640"));
    }
}
