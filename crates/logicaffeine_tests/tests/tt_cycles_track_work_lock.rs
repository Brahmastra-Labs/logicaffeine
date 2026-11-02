//! ═══════════════════════════════════════════════════════════════════════
//! CYCLES-TRACK-WORK LOCK — a cycle count that does not move when the work moves is not a cycle count.
//!
//! Tonight the whole Tenstorrent performance story rested on the simulator's whole-program counter, and that
//! counter turned out not to advance with K for our programs: 640x640x320 / 640x640x640 / 640x640x1280 all
//! reported essentially the same number, and the 1024 family reported one number at K of 512, 1024 and 2048.
//! Correlation against a host reference was 1.0 at every point and the emitted compute loop provably scales
//! with K, so the work was real and the counter was not recording it. ttnn's rows on the same counter DID
//! scale, so every margin computed between the two was measuring different things on each side.
//!
//! Nothing in the ledger caught it. The row-level guards all passed, because each row was internally
//! consistent — the defect only appears when rows are compared ACROSS a shape family. So this lock compares
//! them, using the one signal that did track the work: the wall-clock seconds on the same heartbeat line.
//!
//! A. Within a family of our own rows sharing M, N, strategy and knobs, and differing only in K, the cycle
//!    count and the simulator's wall-clock must not disagree in DIRECTION. If wall time rises materially with
//!    K while cycles stay flat or fall, the counter is not measuring the program and no margin from it is
//!    quotable. Families still exhibiting that are listed in `KNOWN_UNTRACKED`, shrink-only, so the defect is
//!    visible and cannot spread.
//! B. `KNOWN_UNTRACKED` is exactly the live set — a family that starts failing must be added deliberately,
//!    and one that is fixed must be removed, so nobody can leave a stale entry hiding a repair.
//!
//! ⚠️ This lock cannot tell you the right number. It makes recording a wrong one loud.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(tt_cycles_track_work_lock)'`
//! ═══════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Families known to exhibit the untracked-counter defect, with the evidence. SHRINK-ONLY.
const KNOWN_UNTRACKED: &[(&str, &str)] = &[
    // 1024x1024 LEFT THIS LIST BY REPAIR, NOT BY EDIT: at the plans the compiler emits today its cycles
    // move with K again, so `the_untracked_ledger_is_exactly_the_live_set` began failing on the entry —
    // which is the shrink-only half working. 640x640 took its place under the same rule.
    (
        "640x640",
        "K 320->1280: simulator wall-clock 3.1s->8.1s (2.6x) while cycles FALL 5810->5681. The counter \
         cannot be fixed here: benchmarks/silicon/ttsim_instrument_audit.json (generated from 403 \
         simulator logs, never hand-edited) finds the bracket cycle counter bit-identical across a wide \
         sweep of the reduction depth — `a work meter must rise with the reduction depth; this one does \
         not` — and the device's own per-core register flat in K as well, because ttsim is a functional \
         model with no per-core timing. So no margin computed from these cycles is quotable as speed; \
         what IS quotable is work (Tensix instructions, NOC transactions, NOC bytes, retired matmuls), \
         which rises with the work and is conserved.",
    ),
];

/// Wall-clock must rise by at least this factor before we call it a real increase in work, so host noise on a
/// loaded box cannot manufacture a failure.
const WALL_RISE: f64 = 1.5;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn ledger() -> serde_json::Value {
    let p = repo_root().join("benchmarks/silicon/tenstorrent_gemm.json");
    serde_json::from_str(
        &std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display())),
    )
    .expect("the ledger parses")
}

fn shape_of(r: &serde_json::Value) -> Option<(u64, u64, u64)> {
    let s = &r["shape"];
    if let Some(t) = s.as_str() {
        let p: Vec<&str> = t.split('x').collect();
        if p.len() == 3 {
            return Some((p[0].parse().ok()?, p[1].parse().ok()?, p[2].parse().ok()?));
        }
        return None;
    }
    Some((s["m"].as_u64()?, s["n"].as_u64()?, s["k"].as_u64()?))
}

/// `"MxN|strategy|knobs" -> [(K, cycles, seconds)]` over our own current rows that carry a wall-clock.
fn families() -> BTreeMap<String, Vec<(u64, f64, f64)>> {
    let d = ledger();
    let mut out: BTreeMap<String, Vec<(u64, f64, f64)>> = BTreeMap::new();
    for r in d["rows"].as_array().expect("rows") {
        if !r["current"].as_bool().unwrap_or(false) || r["stack"].as_str() != Some("logos_emitted")
        {
            continue;
        }
        let (Some((m, n, k)), Some(c), Some(sec)) =
            (shape_of(r), r["cycles"].as_f64(), r["sim_seconds"].as_f64())
        else {
            continue;
        };
        let kn = &r["knobs"];
        let key = format!(
            "{m}x{n}|{}|{}-{}-{}",
            r["strategy"].as_str().unwrap_or("?"),
            kn["fidelity"].as_str().unwrap_or("?"),
            kn["fp32_dest_acc"].as_bool().unwrap_or(false),
            kn["packer_l1_acc"].as_bool().unwrap_or(false)
        );
        out.entry(key).or_default().push((k, c, sec));
    }
    out.retain(|_, v| {
        v.sort_by_key(|t| t.0);
        v.dedup_by_key(|t| t.0);
        v.len() >= 2
    });
    out
}

/// Families whose wall-clock rises materially with K while cycles fail to follow.
fn untracked() -> Vec<(String, String)> {
    let mut bad = Vec::new();
    for (key, pts) in families() {
        let (lo, hi) = (pts.first().unwrap(), pts.last().unwrap());
        if hi.2 < lo.2 * WALL_RISE {
            continue; // wall time did not rise enough to conclude anything
        }
        if hi.1 <= lo.1 {
            bad.push((
                key.split('|').next().unwrap().to_string(),
                format!(
                    "K {}→{}: wall {:.1}s→{:.1}s ({:.1}x) but cycles {}→{}",
                    lo.0,
                    hi.0,
                    lo.2,
                    hi.2,
                    hi.2 / lo.2,
                    lo.1 as u64,
                    hi.1 as u64
                ),
            ));
        }
    }
    bad
}

// ───────────────────────────── A ─────────────────────────────

#[test]
fn a_cycle_count_that_ignores_the_work_is_not_recorded_silently() {
    let bad = untracked();
    let declared: Vec<&str> = KNOWN_UNTRACKED.iter().map(|(f, _)| *f).collect();
    let undeclared: Vec<&(String, String)> = bad
        .iter()
        .filter(|(f, _)| !declared.contains(&f.as_str()))
        .collect();
    assert!(
        undeclared.is_empty(),
        "shape famil(ies) report cycles that do not move while the simulator's own wall-clock says the work \
         grew. A margin computed from those numbers is not a margin:\n  {}\nFix the counter, or add the \
         family to KNOWN_UNTRACKED with its evidence so it cannot be quoted as a speed result.",
        undeclared.iter().map(|(f, w)| format!("{f}: {w}")).collect::<Vec<_>>().join("\n  ")
    );
}

// ───────────────────────────── B ─────────────────────────────

#[test]
fn the_untracked_ledger_is_exactly_the_live_set_and_carries_its_evidence() {
    // Every declared family must still be exhibiting the defect, or the entry is stale and hides a repair.
    // Only checkable once wall-clock is present on those rows; a family with no timed rows yet is skipped
    // rather than treated as fixed, so re-measuring is what clears an entry, not the absence of data.
    let timed: Vec<String> = families()
        .keys()
        .map(|k| k.split('|').next().unwrap().to_string())
        .collect();
    let live: Vec<String> = untracked().into_iter().map(|(f, _)| f).collect();
    for (fam, why) in KNOWN_UNTRACKED {
        assert!(
            why.len() > 30,
            "{fam}: every entry states the evidence, not just the fact"
        );
        if timed.iter().any(|t| t == fam) {
            assert!(
                live.iter().any(|l| l == fam),
                "{fam} no longer shows the defect — remove it from KNOWN_UNTRACKED (shrink-only) and re-quote \
                 its numbers"
            );
        }
    }
}

// ───────────────────────────── C ─────────────────────────────

/// Two metrics will coexist while the device counter is rolled out: the legacy whole-program `cycles`, which
/// does not track work, and `device_cycles`, the slowest core's own counter, which does. A comparison that
/// took one from each side would be meaningless in exactly the way tonight's margins were, and it would look
/// perfectly reasonable in the JSON. So: within any single comparison, both sides must be the same KIND of
/// measurement. This is the same rule as matched numerics, applied to the clock instead of the arithmetic.
#[test]
fn a_comparison_never_mixes_the_two_clocks() {
    let d = ledger();
    let rows: BTreeMap<&str, &serde_json::Value> = d["rows"]
        .as_array()
        .expect("rows")
        .iter()
        .map(|r| (r["id"].as_str().unwrap_or_default(), r))
        .collect();
    // Keyed on the metric the ratio was actually computed from, which each comparison records, NOT on which
    // fields a row happens to carry. A row may hold `device_cycles` from an instrumentation run while the
    // ratio still divides `cycles` by `cycles` — same clock on both sides. Reading presence as use refused
    // four valid comparisons and left a pitch table rendering "—".
    let has_metric =
        |id: &str, metric: &str| -> Option<bool> { rows.get(id).map(|r| !r[metric].is_null()) };
    let mut mixed = Vec::new();
    for c in d["comparisons"]
        .as_array()
        .map(|v| v.as_slice())
        .unwrap_or_default()
    {
        let Some(subj) = c["subject"].as_str() else {
            continue;
        };
        for (name, v) in c["vs"]
            .as_object()
            .map(|o| o.iter().collect::<Vec<_>>())
            .unwrap_or_default()
        {
            let Some(other) = v["id"].as_str().or_else(|| v["row"].as_str()) else {
                continue;
            };
            let metric = v["ratio_metric"].as_str().unwrap_or("cycles");
            let (Some(subj_kind), Some(other_kind)) =
                (has_metric(subj, metric), has_metric(other, metric))
            else {
                continue;
            };
            if !subj_kind || !other_kind {
                mixed.push(format!(
                    "{}: the ratio is computed from `{metric}` but {} lacks it",
                    c["id"].as_str().unwrap_or("?"),
                    if subj_kind {
                        name.as_str()
                    } else {
                        "the subject"
                    }
                ));
            }
        }
    }
    assert!(
        mixed.is_empty(),
        "comparison(s) put a device-cycle row against a whole-program-clock row. Those are different \
         quantities and the ratio between them means nothing:\n  {}\nRe-measure the other side on the same \
         clock, or drop the comparison until you can.",
        mixed.join("\n  ")
    );
}
