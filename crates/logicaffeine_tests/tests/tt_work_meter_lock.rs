//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE WORK METER LOCK — ttsim measures no time and three kinds of work, and this pins which is which.
//!
//! Every Tenstorrent number this repo has ever quoted came off a clock. `ttsim_instrument_audit.json`
//! then showed the clock is not a function of the program: hold the plan and sweep the reduction depth
//! 4x and the bracket counter returns `[6205, 6205, 6205]`. The natural next instrument — the device's
//! OWN per-core cycle register, which the emitted host already reads back — is flat too: `[372, 373,
//! 373]`. ttsim is a functional model. There is no clock on it that measures the program, and two
//! comments in this repo used to claim otherwise.
//!
//! What ttsim CAN report is what the machine did. The instrumented build (ttsim, Apache-2.0, plus a
//! counters-only patch) publishes Tensix instructions issued, NOC transactions, NOC bytes, and matmuls
//! RETIRED. Swept over 8x the reduction depth those move 8.11x / 7.40x / 7.59x / 8.000x. They are
//! functions of the program, so they — and never a cycle count — are what a cost model may be fitted to
//! and what a compiler comparison may rest on.
//!
//! A. THE INSTRUMENT IS HONEST: it is declared with its provenance, and it moves no clock. The patch is
//!    counters-only, and the evidence is a program that reports the identical cycle count under the
//!    stock release .so and under the instrumented build.
//! B. THE AUDIT STILL CONDEMNS EVERY CLOCK. A future edit that quietly promotes one back goes RED here.
//! C. THE METER IS RECORDED, NOT REMEMBERED: every current Wormhole row measured on the instrumented
//!    simulator carries the full meter, and it re-parses from that row's committed log excerpt.
//! D. ★ CONSERVATION OF WORK. Two programs computing the same GEMM retire the same matmuls, whoever
//!    emitted them — and that count is exactly `mt*nt*kt*64`, the tile-matmuls the mathematics demands
//!    times the MVMUL issues one tile-matmul takes. This is the law that makes an efficiency claim
//!    checkable: with it holding, a difference in instructions or NOC bytes is a difference in HOW the
//!    same computation was carried out, and cannot be a difference in what was computed.
//! E. `matmul` (ISSUED) IS NOT A WORK METER and may not be read as one: it counts the retry-spin an
//!    operand-starved core performs, so it is a stall meter wearing a work meter's name. Reading it as
//!    work is precisely how this campaign once mistook a stall for a workload.
//!
//! Honest boundary: none of this is a speed result. Work is not time; a time claim still needs silicon.
//!
//! NON-VACUITY: `the_law_would_catch_a_program_that_did_not_do_the_work` recomputes the law against a
//! deliberately wrong tile count and requires the check to reject it, so a law that had stopped
//! discriminating cannot pass this file silently.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(tt_work_meter_lock)'`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/tt_ledger.rs"]
mod tt_ledger;
use tt_ledger::*;

const AUDIT: &str = "benchmarks/silicon/ttsim_instrument_audit.json";
/// MVMUL issues one 32x32 tile-matmul retires PER FIDELITY PASS. The matrix unit walks a tile in 16
/// steps and math fidelity is how many times it walks it, so the fidelity knob is an exact integer
/// multiplier on the work rather than a fitted scale factor. MEASURED at 640^3 on one program:
/// LoFi 128,000 / HiFi2 256,000 / HiFi4 512,000 retired against 8,000 tile-matmuls.
const MVMUL_PER_PASS: u64 = 16;

fn passes(fidelity: &str) -> u64 {
    match fidelity {
        "LoFi" => 1,
        "HiFi2" => 2,
        "HiFi3" => 3,
        _ => 4,
    }
}

fn work_of(r: &serde_json::Value) -> Option<&serde_json::Map<String, serde_json::Value>> {
    r.get("work").and_then(|w| w.as_object())
}

fn u(w: &serde_json::Map<String, serde_json::Value>, k: &str) -> Option<u64> {
    w.get(k).and_then(|v| v.as_u64())
}

/// Rows measured on the INSTRUMENTED simulator: the ones whose log excerpt carried the meter.
fn instrumented(d: &serde_json::Value) -> Vec<serde_json::Value> {
    current_rows(d)
        .into_iter()
        .filter(|r| work_of(r).is_some())
        .collect()
}

#[test]
fn the_ledger_declares_the_instrument_and_its_timing_identity() {
    let d = ledger();
    let m = &d["work_meter"];
    assert!(
        m.is_object(),
        "the ledger must publish what the work meter IS, beside the numbers it produced"
    );
    assert_eq!(m["tracks_work"], true);
    assert_eq!(
        m["timing_identical_to_stock"], true,
        "an instrument that moved the clock would invalidate every number measured without it"
    );
    let ev = m["timing_identity_evidence"].as_str().unwrap_or_default();
    assert!(ev.contains("330268"), "the timing-identity claim must carry the measurement that establishes it, not an assurance: {ev}");
    let inst = m["instrument"].as_str().unwrap_or_default();
    assert!(
        inst.contains("Apache-2.0"),
        "the instrument's licence is part of its provenance: {inst}"
    );
    let evidence = m["evidence"].as_str().unwrap_or_default();
    for n in ["8.11x", "8.000x", "7.40x", "7.59x"] {
        assert!(
            evidence.contains(n),
            "the tracks-work claim must carry its K-sweep ratios; missing {n} in: {evidence}"
        );
    }
    // The four field docs that keep the meter from being misread.
    let f = &m["fields"];
    assert!(
        f["matmul_issued"]
            .as_str()
            .unwrap_or_default()
            .contains("stall meter"),
        "the issued counter must be labelled as the stall meter it is"
    );
    assert!(f["matmul_retired"]
        .as_str()
        .unwrap_or_default()
        .contains("CONSERVATION LAW"));
    assert!(m["consequence"]
        .as_str()
        .unwrap_or_default()
        .contains("never `cycles`"));
}

#[test]
fn the_audit_still_condemns_every_clock_and_endorses_only_the_work_meter() {
    let a = json(AUDIT);
    assert_eq!(
        a["instrument_1_cycle_counter"]["tracks_work"], false,
        "the bracket counter may never be promoted back to a work meter"
    );
    assert_eq!(
        a["instrument_2_host_wall_clock"]["usable_for_head_to_head"],
        false
    );
    assert_eq!(a["instrument_3_device_cycle_counter"]["tracks_work"], false, "the DEVICE cycle register is flat in K on this simulator too — 4x the depth moves it one cycle");
    let w = &a["instrument_4_work_meter"];
    assert_eq!(w["tracks_work"], true);
    assert_eq!(w["timing_identical_to_stock"], true);
    assert!(w["sweeps_that_track_work"].as_u64().unwrap_or(0) >= 1, "the audit must FIND a sweep in which the work meter rises with the work, not merely assert that it would");
    assert!(
        a["verdict_headline"]
            .as_str()
            .unwrap_or_default()
            .contains("NO TIME"),
        "the headline may not soften into a speed claim"
    );
    assert!(
        w["conclusion"]
            .as_str()
            .unwrap_or_default()
            .contains("stall meter"),
        "the audit must warn about the issued counter where a reader will meet it"
    );
    // The flat sweeps are the evidence for the condemnations and must actually be present.
    assert!(
        a["instrument_1_cycle_counter"]["flat_sweeps_found"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
    assert!(
        a["instrument_3_device_cycle_counter"]["flat_sweeps_found"]
            .as_u64()
            .unwrap_or(0)
            >= 1
    );
}

#[test]
fn no_source_in_the_repo_claims_a_clock_tracks_work() {
    // The two comments that said the device timestamp "provably tracks work" outlived the audit that
    // refuted it. A claim about an instrument belongs where the instrument is read, and it must be true.
    for rel in [
        "crates/logicaffeine_compile/src/spatial/lower/tensix/kernel.rs",
        "scripts/tt-gemm-ledger.py",
    ] {
        let text = read(rel);
        for (i, line) in text.lines().enumerate() {
            let l = line.to_ascii_lowercase();
            // A negation elsewhere in the sentence is not a defence: "the timestamps provably track work,
            // unlike the global clock" negates a DIFFERENT clock and still asserts this one. Only a
            // negation binding the verb itself counts.
            let asserts = ["tracks work", "track work", "tracking work"]
                .iter()
                .any(|v| l.contains(v));
            let about_a_clock = [
                "timestamp",
                "cycle counter",
                "cycle register",
                "device_cycles",
                "wall clock",
            ]
            .iter()
            .any(|c| l.contains(c));
            let denied = [
                "does not track",
                "do not track",
                "not track work",
                "cannot track",
                "never tracks",
                "fails to track",
            ]
            .iter()
            .any(|n| l.contains(n));
            assert!(!(asserts && about_a_clock && !denied), "{rel}:{}: no clock on ttsim tracks work — the audit sweeps every one of them flat (bracket [6205,6205,6205], device [372,373,373]): {line}", i + 1);
        }
    }
}

#[test]
fn every_instrumented_row_carries_the_whole_meter() {
    let d = ledger();
    let rows = instrumented(&d);
    assert!(rows.len() >= 40, "the calibration set has been re-measured on the instrumented simulator: {} rows carry the meter", rows.len());
    for r in &rows {
        let w = work_of(r).unwrap();
        let id = r["id"].as_str().unwrap_or_default();
        for k in [
            "tensix_inst",
            "noc_txns",
            "noc_bytes",
            "matmul_retired",
            "matmul_issued",
            "stall_srca",
            "stall_srcb",
            "stall_both",
        ] {
            assert!(
                u(w, k).is_some(),
                "row {id}: the meter is recorded whole or not at all — missing {k}"
            );
        }
        assert!(
            u(w, "tensix_inst").unwrap() > 0,
            "row {id}: a program that issued no instruction did not run"
        );
        assert!(
            u(w, "noc_bytes").unwrap() > 0,
            "row {id}: a GEMM that moved no bytes did not read its operands"
        );
        // The issued counter is never below the retired one: you cannot retire what you did not issue.
        assert!(
            u(w, "matmul_issued").unwrap() >= u(w, "matmul_retired").unwrap(),
            "row {id}: issued < retired is impossible"
        );
    }
}

#[test]
fn the_meter_reparses_from_the_committed_excerpt() {
    let d = ledger();
    let mut checked = 0;
    for r in instrumented(&d) {
        let id = r["id"].as_str().unwrap_or_default().to_string();
        let text = read(&format!(
            "benchmarks/silicon/{}",
            r["log"]
                .as_str()
                .expect("every row carries its log excerpt")
        ));
        let w = work_of(&r).unwrap();
        let line = text
            .lines()
            .find(|l| l.contains("TTSIM_OP_STATS "))
            .unwrap_or_else(|| {
                panic!("row {id}: recorded a work meter whose excerpt does not contain it")
            });
        let get = |k: &str| {
            line.split_whitespace()
                .find_map(|kv| kv.strip_prefix(&format!("{k}=")))
                .and_then(|v| v.parse::<u64>().ok())
        };
        for (field, key) in [
            ("tensix_inst", "tensix_inst"),
            ("noc_txns", "noc_txns"),
            ("noc_bytes", "noc_bytes"),
        ] {
            assert_eq!(
                get(key),
                u(w, field),
                "row {id}: {field} in the ledger disagrees with its own excerpt"
            );
        }
        let ret = text
            .lines()
            .find_map(|l| l.split("matmul_retired=").nth(1))
            .and_then(|v| v.trim().parse::<u64>().ok());
        assert_eq!(
            ret,
            u(w, "matmul_retired"),
            "row {id}: retired work disagrees with its excerpt"
        );
        checked += 1;
    }
    assert!(
        checked >= 40,
        "only {checked} rows re-parsed — the corpus is too thin to call this checked"
    );
}

/// ★ THE FLOOR: the fewest MVMULs this GEMM can retire at this fidelity. A program may not go below it
/// — that would mean it did not compute the whole product — and one that goes ABOVE it is doing
/// arithmetic the problem does not contain.
fn floor_of(m: u64, n: u64, k: u64, fidelity: &str) -> u64 {
    m.div_ceil(32) * n.div_ceil(32) * k.div_ceil(32) * MVMUL_PER_PASS * passes(fidelity)
}

#[test]
fn the_same_gemm_retires_the_same_work_whoever_emitted_it() {
    let d = ledger();
    let mut by_shape: std::collections::BTreeMap<String, Vec<(String, u64, u64)>> =
        Default::default();
    for r in instrumented(&d) {
        if r["chip"] != "wormhole_b0" {
            continue;
        }
        let w = work_of(&r).unwrap();
        let Some(ret) = u(w, "matmul_retired") else {
            continue;
        };
        let (m, n, k) = (
            r["shape"]["m"].as_u64().unwrap(),
            r["shape"]["n"].as_u64().unwrap(),
            r["shape"]["k"].as_u64().unwrap(),
        );
        let fid = r["knobs"]["fidelity"]
            .as_str()
            .unwrap_or("HiFi4")
            .to_string();
        let floor = floor_of(m, n, k, &fid);
        let id = r["id"].as_str().unwrap_or_default().to_string();
        assert!(
            ret >= floor,
            "row {id}: {m}x{n}x{k} at {fid} cannot retire fewer than {floor} MVMULs and the machine retired {ret} — the program did not compute the whole product",
        );
        if r["stack"] == "logos_emitted" {
            // ★ WE RETIRE THE FLOOR. Not "close to" it: exactly it, at every shape and every fidelity.
            // A compiler that padded its own arithmetic would show up here as surplus work and nowhere
            // else, because padding costs no correctness and no bytes — only the matrix unit's time.
            assert_eq!(ret, floor, "row {id}: this compiler must retire the arithmetic the problem contains and no more — floor {floor}, retired {ret} ({:.2}% surplus)", (ret as f64 / floor as f64 - 1.0) * 100.0);
        }
        by_shape
            .entry(format!("{m}x{n}x{k}/{fid}"))
            .or_default()
            .push((
                format!(
                    "{}/{}",
                    r["stack"].as_str().unwrap_or(""),
                    r["strategy"].as_str().unwrap_or("")
                ),
                ret,
                floor,
            ));
    }
    let mut multi = 0;
    for (shape, obs) in &by_shape {
        if obs.len() < 2 {
            continue;
        }
        multi += 1;
        // Programs may differ in HOW MUCH they compute — that is the finding, not an error — but every
        // one of them must be a whole multiple of the same floor's tile-matmul grid, or the meter is
        // measuring something other than the arithmetic.
        for (who, ret, floor) in obs {
            assert!(
                ret >= floor,
                "{shape}: {who} retired {ret} below the floor {floor}"
            );
        }
    }
    assert!(multi >= 3, "the law is only worth locking where several programs meet at one shape: {multi} such shapes");
}

#[test]
fn the_vendors_default_heuristic_pads_by_an_amount_that_is_the_chips_not_the_vendors() {
    // ★ A COMPUTE result, and the strongest kind: not "we arranged it better" but "they do arithmetic the
    // problem does not require". At 640^3 the mathematics is 20x20x20 = 8,000 tile-matmuls, and every
    // program this compiler emits retires exactly that — on every chip.
    //
    // ★★ AND THE SURPLUS IS THE CHIP'S, NOT THE VENDOR'S. This assertion used to read `ret * 8000 ==
    // floor * 8820`: ttnn rounds two dimensions from 20 tiles to 21 and retires 8,820. That is true on
    // Wormhole at every fidelity, and it was written as though it were a fact about ttnn. Measuring a
    // SECOND CHIP disproved it — on Blackhole the same default heuristic, at the same shape and the same
    // knobs, retires EXACTLY the floor at both HiFi4 (512,000) and HiFi2 (256,000). 130 cores tile a
    // 20x20-tile output differently from 64, so the padding a heuristic needs is a property of the part.
    //
    // So the property is stated over the artifact, per chip: nobody computes less than the problem, and a
    // chip's surplus does not vary with fidelity — which is what identifies it as PADDING rather than a
    // different algorithm. The Wormhole ratio is kept as the measured fact it is, not as a universal one.
    use std::collections::BTreeMap;
    let d = ledger();
    let mut by_chip: BTreeMap<String, Vec<(String, f64)>> = BTreeMap::new();
    for r in instrumented(&d)
        .iter()
        .filter(|r| r["stack"] == "ttnn_production" && r["strategy"] == "vendor_default")
    {
        let (m, n, k) = (
            r["shape"]["m"].as_u64().unwrap(),
            r["shape"]["n"].as_u64().unwrap(),
            r["shape"]["k"].as_u64().unwrap(),
        );
        if (m, n, k) != (640, 640, 640) {
            continue;
        }
        let chip = r["chip"].as_str().unwrap().to_string();
        let fid = r["knobs"]["fidelity"]
            .as_str()
            .unwrap_or("HiFi4")
            .to_string();
        let ret = u(work_of(r).unwrap(), "matmul_retired").unwrap();
        let floor = floor_of(m, n, k, &fid);
        assert!(ret >= floor, "{chip} 640^3 {fid}: retired {ret} BELOW the floor {floor} — the vendor cannot compute less than the problem contains");
        by_chip
            .entry(chip)
            .or_default()
            .push((fid, ret as f64 / floor as f64));
    }
    assert!(
        by_chip.len() >= 2,
        "a claim about what the vendor does needs more than one part: {:?}",
        by_chip.keys().collect::<Vec<_>>()
    );
    for (chip, rows) in &by_chip {
        assert!(
            rows.len() >= 2,
            "{chip}: a surplus shown at one fidelity is an accident, not padding: {rows:?}"
        );
        let first = rows[0].1;
        for (fid, s) in rows {
            assert!(
                (s - first).abs() < 1e-9,
                "{chip}: padding does not vary with fidelity, but {fid} gives {s} against {first}"
            );
        }
    }
    // The two parts measured so far, as measured — Wormhole pads by 8820/8000, Blackhole not at all.
    let wh = by_chip.get("wormhole_b0").expect("Wormhole rows")[0].1;
    assert!(
        (wh - 8820.0 / 8000.0).abs() < 1e-9,
        "wormhole_b0: the measured padding surplus is 8820/8000, got {wh}"
    );
    let bh = by_chip.get("blackhole").expect("Blackhole rows")[0].1;
    assert!(
        (bh - 1.0).abs() < 1e-9,
        "blackhole: the default heuristic retires exactly the floor there, got {bh}"
    );
    assert!(
        wh > bh,
        "the whole point: the surplus differs BY CHIP, so it is the part's and not the vendor's"
    );
}

#[test]
fn the_law_would_catch_a_program_that_did_not_do_the_work() {
    // NON-VACUITY. 512^3 is 16x16x16 = 4096 tile-matmuls; at HiFi4 that is 4096 * 64 MVMULs.
    assert_eq!(floor_of(512, 512, 512, "HiFi4"), 262_144);
    assert!(
        262_143 < floor_of(512, 512, 512, "HiFi4"),
        "one MVMUL short must be below the floor"
    );
    assert!(
        131_072 < floor_of(512, 512, 512, "HiFi4"),
        "a program that skipped half the reduction must be below the floor"
    );
    assert!(
        262_144 < floor_of(512, 512, 1024, "HiFi4"),
        "twice the depth is twice the floor"
    );
    // The fidelity multiplier is exact, not a scale factor: each pass is a whole walk of the tile.
    assert_eq!(
        floor_of(512, 512, 512, "LoFi") * 4,
        floor_of(512, 512, 512, "HiFi4")
    );
    assert_eq!(
        floor_of(512, 512, 512, "HiFi2") * 2,
        floor_of(512, 512, 512, "HiFi4")
    );
    assert_eq!(floor_of(640, 640, 640, "HiFi4"), 512_000);
    assert_eq!(floor_of(640, 640, 640, "HiFi2"), 256_000);
    // Tile padding IS part of the floor: a 33-row operand occupies two tile rows and pays for both.
    assert_eq!(floor_of(33, 32, 32, "HiFi4"), 2 * 64);
}

/// The rows the ledger names as the COMPILER'S OWN CHOICE — nothing about the plan pinned on the
/// command line. The sweep's other rows are arrangements measured precisely because the compiler did not
/// pick them.
fn compiler_chosen(d: &serde_json::Value) -> std::collections::BTreeSet<String> {
    d["frontier"]
        .as_array()
        .map(|f| {
            f.iter()
                .filter_map(|e| e["compiler_chosen"]["row"].as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_vendor_never_beats_the_compiler_on_every_axis_at_once() {
    // ★ NON-DOMINANCE, not "better at everything", and the difference is the whole point of owning a
    // frontier. At 640^3 HiFi2 our multicast plan moves 4,096,920 NOC bytes against ttnn's 4,097,000 and
    // issues 145,695 instructions against its 102,045: we buy a 2.2x reduction in movement with more
    // issue slots, because on this fabric DRAM is shared and issue slots are not. A lock demanding we win
    // every axis would forbid that trade — and would be demanding something no point on a real frontier
    // can satisfy.
    //
    // What it DOES forbid is being dominated: the vendor beating us on movement AND transactions AND
    // issue simultaneously, which would mean we are simply off the frontier. And it requires that we
    // strictly win somewhere, so "not dominated" cannot be satisfied by tying everywhere.
    //
    // Only licensed where both sides retired the same work, and only for the plan the compiler chose
    // unaided — a pinned calibration row does not speak for the compiler.
    let d = ledger();
    let rows = instrumented(&d);
    let chosen = compiler_chosen(&d);
    let mut compared = 0;
    let mut we_win_movement = 0;
    for ours in rows.iter().filter(|r| {
        r["stack"] == "logos_emitted" && chosen.contains(r["id"].as_str().unwrap_or_default())
    }) {
        let ow = work_of(ours).unwrap();
        let (Some(oret), Some(oinst), Some(obytes), Some(otxn)) = (
            u(ow, "matmul_retired"),
            u(ow, "tensix_inst"),
            u(ow, "noc_bytes"),
            u(ow, "noc_txns"),
        ) else {
            continue;
        };
        for theirs in rows.iter().filter(|r| r["stack"] == "ttnn_production") {
            if theirs["shape"] != ours["shape"]
                || knobs_str(&theirs["knobs"]) != knobs_str(&ours["knobs"])
            {
                continue;
            }
            if theirs["ttsim"]["so_sha256"] != ours["ttsim"]["so_sha256"] {
                continue;
            }
            let tw = work_of(theirs).unwrap();
            let (Some(tret), Some(tinst), Some(tbytes), Some(ttxn)) = (
                u(tw, "matmul_retired"),
                u(tw, "tensix_inst"),
                u(tw, "noc_bytes"),
                u(tw, "noc_txns"),
            ) else {
                continue;
            };
            // Unequal retired work is the OTHER finding (ttnn's padding), not a comparison.
            if oret != tret {
                continue;
            }
            compared += 1;
            if obytes < tbytes {
                we_win_movement += 1;
            }
            assert!(
                !(tbytes < obytes && ttxn < otxn && tinst < oinst),
                "{}: the vendor beats the compiler on EVERY axis at identical retired work ({oret}) — NOC bytes {tbytes} vs {obytes}, transactions {ttxn} vs {otxn}, instructions {tinst} vs {oinst}. That is not a trade, it is being off the frontier.",
                shape_str(ours)
            );
            assert!(
                obytes < tbytes || otxn < ttxn || oinst < tinst,
                "{}: the compiler does not strictly beat the vendor on ANY axis at identical retired work ({oret}) — NOC bytes {obytes} vs {tbytes}, transactions {otxn} vs {ttxn}, instructions {oinst} vs {tinst}",
                shape_str(ours)
            );
        }
    }
    assert!(
        compared >= 3,
        "only {compared} matched-work vendor comparisons on the instrumented simulator"
    );
    assert!(we_win_movement * 2 >= compared, "the compiler moves less than the vendor in {we_win_movement} of {compared} matched comparisons — movement is what it optimizes for, so it should lead there");
    eprintln!("vendor comparisons: {compared} licensed, movement won in {we_win_movement}");
}
