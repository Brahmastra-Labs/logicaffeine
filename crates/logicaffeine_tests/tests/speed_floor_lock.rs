//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! EVERY SPEED WE HAVE MEASURED IS A FLOOR WE DO NOT FALL THROUGH.
//!
//! The receipts on record each say how fast something ran: the compiled lane's decode and prefill rates on the RTX 3060
//! and its rates over llama.cpp's and the serving engines' on the same boot (`rtx3060_scoreboard.json`), the D5005
//! card's tokens a second (`card_runs.json`), the Nexys decode boards' (`decode_images.json`), the Lean kernel's
//! instructions on every Arena case and its margin over the best other checker (`lean-kernel-arena/scoreboard.json`),
//! and the certified SAT prover's times and its margin over every rival it beats (`solvers.json`). Each is an
//! achievement, and `benchmarks/results/speed_floors.json` holds a floor for it five per cent short of the best it has
//! reached: a rate or a margin at 95% of it, a time or an instruction count at its 1/0.95.
//!
//!   • `every_achievement_on_record_has_a_floor` — a receipt nobody floored is a speed nothing protects;
//!   • `every_measurement_on_record_holds_its_floor` — a re-measured ledger that fell more than the margin is a
//!     regression, whatever else it fixed;
//!   • `every_floor_is_the_margin_short_of_what_was_achieved` — a floor is derived, never typed;
//!   • `a_measurement_past_its_floor_by_more_than_the_margin_raises_it` — a gain is locked in when it lands, so the
//!     floors only rise;
//!   • `every_floor_names_its_receipt` — the ledger, and when and where it was measured.
//!
//! Recording takes each achievement's best: `LOGOS_UPDATE_FIXTURES=1` raises a floor to the margin under a better
//! measurement and never lowers one. A floor comes down only by hand, with the reason beside it. Record with the one
//! test that writes the ledger of floors, so no other test reads it mid-write:
//!
//!   LOGOS_UPDATE_FIXTURES=1 cargo nextest run -p logicaffeine-tests --test speed_floor_lock -E 'test(every_achievement_on_record_has_a_floor)'
//!
//! A fresh measurement is checked against the floors without touching the ledgers: `LOGOS_SPEED_LEDGERS` maps a
//! ledger to a measurement of the same shape (`benchmarks/results/models/rtx3060_scoreboard.json=logs/x.json`), and
//! `every_measurement_on_record_holds_its_floor` then reads the measurement in its place, judging every floor it
//! measured and naming every floor it did not. `scripts/accel/check-speed-floors.sh` runs the scoreboard into
//! `logs/speed-floors/` and does this.
//!
//! `LOGOS_SPEED_FLOORS` reads the floors from another file in place of the ledger of floors.
//!
//!   cargo nextest run -p logicaffeine-tests --test speed_floor_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// How far under its best an achievement may fall.
const MARGIN: f64 = 0.05;

/// Below this many milliseconds a SAT time is timer noise and process start-up (the ledger's own caveat): such a row is
/// no achievement a floor can hold.
const SAT_RESOLVABLE_MS: f64 = 1.0;

const FLOORS: &str = "benchmarks/results/speed_floors.json";
const SCOREBOARD: &str = "benchmarks/results/models/rtx3060_scoreboard.json";
const CARD_RUNS: &str = "benchmarks/silicon/d5005/card_runs.json";
const DECODE_IMAGES: &str = "benchmarks/silicon/decode_images.json";
const ARENA: &str = "benchmarks/results/lean-kernel-arena/scoreboard.json";
const SOLVERS: &str = "benchmarks/results/solvers.json";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Which way an achievement improves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Better {
    Higher,
    Lower,
}

impl Better {
    fn name(self) -> &'static str {
        match self {
            Better::Higher => "higher",
            Better::Lower => "lower",
        }
    }

    fn parse(s: &str) -> Better {
        match s {
            "higher" => Better::Higher,
            "lower" => Better::Lower,
            other => panic!("a floor is `higher` or `lower` better, not `{other}`"),
        }
    }

    /// The bound `achieved` sets: the margin under a rate, the margin over a time.
    fn bound(self, achieved: f64) -> f64 {
        match self {
            Better::Higher => achieved * (1.0 - MARGIN),
            Better::Lower => achieved / (1.0 - MARGIN),
        }
    }

    fn holds(self, value: f64, bound: f64) -> bool {
        match self {
            Better::Higher => value >= bound,
            Better::Lower => value <= bound,
        }
    }

    fn best(self, a: f64, b: f64) -> f64 {
        match self {
            Better::Higher => a.max(b),
            Better::Lower => a.min(b),
        }
    }

    /// Whether `value` is past `achieved` by more than the margin: a gain to lock in.
    fn gained(self, value: f64, achieved: f64) -> bool {
        match self {
            Better::Higher => value > achieved / (1.0 - MARGIN),
            Better::Lower => value < achieved * (1.0 - MARGIN),
        }
    }
}

/// One achievement as a ledger records it now.
#[derive(Clone, Debug)]
struct Achievement {
    id: String,
    ledger: &'static str,
    better: Better,
    value: f64,
    unit: &'static str,
    receipt: Value,
}

fn mean(v: &Value, what: &str) -> f64 {
    let runs: Vec<f64> = v.as_array().unwrap_or_else(|| panic!("{what}: a list of runs")).iter().map(|x| x.as_f64().unwrap()).collect();
    assert!(!runs.is_empty(), "{what}: no runs");
    runs.iter().sum::<f64>() / runs.len() as f64
}

/// Where each ledger is read from: its own path, or the measurement `LOGOS_SPEED_LEDGERS` puts in its place.
fn overrides() -> BTreeMap<String, PathBuf> {
    std::env::var("LOGOS_SPEED_LEDGERS")
        .unwrap_or_default()
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|pair| {
            let (ledger, at) = pair.split_once('=').unwrap_or_else(|| panic!("LOGOS_SPEED_LEDGERS: `{pair}` is `ledger=measurement`"));
            let at = Path::new(at.trim());
            (ledger.trim().to_string(), if at.is_absolute() { at.to_path_buf() } else { root().join(at) })
        })
        .collect()
}

fn ledger(rel: &str) -> Value {
    let path = overrides().remove(rel).unwrap_or_else(|| root().join(rel));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The keys a scoreboard row carries that the floors read or that carry no speed. A row with any other key holds a
/// measurement no floor reads yet: the lock fails until one does.
const SCOREBOARD_ROW_KEYS: [&str; 12] = [
    "model",
    "file",
    "file_bytes",
    "file_sha256",
    "decode_bytes_per_token",
    "decode_routed",
    "load_average",
    "ours",
    "llama_cpp",
    "vllm",
    "exllamav3",
    "batches",
];

/// The serving engines a scoreboard row may carry, as `model_scoreboard_lock` reads them.
const ENGINES: [&str; 2] = ["vllm", "exllamav3"];

fn scoreboard(l: &Value) -> Vec<Achievement> {
    let receipt = json!({"ledger": SCOREBOARD, "measured_at": l["measured_at"], "boot_id": l["machine"]["boot_id"]});
    let mut out = Vec::new();
    for row in l["rows"].as_array().expect("the scoreboard's rows") {
        let m = row["model"].as_str().expect("a row's model");
        for key in row.as_object().unwrap().keys() {
            assert!(
                SCOREBOARD_ROW_KEYS.contains(&key.as_str()),
                "{SCOREBOARD} {m}: `{key}` is a measurement no floor reads — give it floors in speed_floor_lock before it is quoted"
            );
        }
        let at = |what: &str, value: f64, unit: &'static str| Achievement {
            id: format!("rtx3060/{m}/{what}"),
            ledger: SCOREBOARD,
            better: Better::Higher,
            value,
            unit,
            receipt: receipt.clone(),
        };
        let decode = mean(&row["ours"]["short"]["decode_tok_s"], &format!("{m} ours decode"));
        let prefill = mean(&row["ours"]["long"]["prefill_tok_s"], &format!("{m} ours prefill"));
        out.push(at("decode", decode, "tok/s"));
        out.push(at("prefill", prefill, "tok/s"));
        let tg = mean(&row["llama_cpp"]["tg128"]["tok_s"], &format!("{m} llama.cpp tg128"));
        let pp = mean(&row["llama_cpp"]["pp512"]["tok_s"], &format!("{m} llama.cpp pp512"));
        out.push(at("decode-over-llama.cpp", decode / tg, "×"));
        out.push(at("prefill-over-llama.cpp", prefill / pp, "×"));
        for engine in ENGINES {
            let Some(side) = row.get(engine) else { continue };
            // The engine's rate is the faster of its two clocks: the benefit of the doubt, as model_scoreboard_lock gives it.
            let best = |wall: &str, own: &str| mean(&side[wall], &format!("{m} {engine} {wall}")).max(mean(&side[own], &format!("{m} {engine} {own}")));
            out.push(at(&format!("decode-over-{engine}"), decode / best("decode_tok_s", "decode_engine_tok_s"), "×"));
            out.push(at(&format!("prefill-over-{engine}"), prefill / best("prefill_tok_s", "prefill_engine_tok_s"), "×"));
        }
        // Each batch size the row runs: our aggregate decode rate, and it over vLLM's at the same concurrency where vLLM ran.
        for batch in row["batches"].as_array().into_iter().flatten() {
            let b = batch["sequences"].as_u64().unwrap_or_else(|| panic!("{m}: a batch's sequences"));
            let ours = mean(&batch["ours"]["decode_tok_s"], &format!("{m} ours at {b} sequences"));
            out.push(at(&format!("b{b}/decode"), ours, "tok/s"));
            if let Some(side) = batch.get("vllm") {
                let theirs = mean(&side["decode_tok_s"], &format!("{m} vllm at {b}")).max(mean(&side["decode_engine_tok_s"], &format!("{m} vllm at {b}")));
                out.push(at(&format!("b{b}/decode-over-vllm"), ours / theirs, "×"));
            }
        }
    }
    out
}

fn card_runs(l: &Value) -> Vec<Achievement> {
    let receipt = json!({"ledger": CARD_RUNS, "image_sof_sha256": l["image"]["sof_sha256"]});
    l["runs"]
        .as_array()
        .expect("the card's runs")
        .iter()
        .map(|r| Achievement {
            id: format!("d5005/{}/{}seq", r["model"].as_str().unwrap(), r["sequences"].as_u64().unwrap()),
            ledger: CARD_RUNS,
            better: Better::Higher,
            value: r["tokens_a_second"].as_f64().expect("a run's tokens a second"),
            unit: "tok/s",
            receipt: json!({"ledger": CARD_RUNS, "image_sof_sha256": receipt["image_sof_sha256"], "log": r["log"]}),
        })
        .collect()
}

fn decode_images(l: &Value) -> Vec<Achievement> {
    l["images"]
        .as_array()
        .expect("the decode images")
        .iter()
        .filter(|i| i["measured"].is_object())
        .map(|i| Achievement {
            id: format!("nexys/{}", i["top"].as_str().unwrap()),
            ledger: DECODE_IMAGES,
            better: Better::Higher,
            value: i["measured"]["tokens_per_s"].as_f64().expect("a measured run's tokens a second"),
            unit: "tok/s",
            receipt: json!({
                "ledger": DECODE_IMAGES,
                "bitstream_sha256": i["measured"]["bitstream_sha256"],
                "board": i["measured"]["board"],
                "date": i["measured"]["date"],
            }),
        })
        .collect()
}

fn arena(l: &Value) -> Vec<Achievement> {
    let receipt = json!({"ledger": ARENA, "measured_at": l["measured_at"], "live_revision": l["live_revision"], "checker_sha256": l["checker_sha256"]});
    let mut out = Vec::new();
    for (case, r) in l["results"].as_object().expect("the arena's results") {
        let Some(ours) = r["instructions"].as_f64() else { continue };
        out.push(Achievement {
            id: format!("arena/{case}/instructions"),
            ledger: ARENA,
            better: Better::Lower,
            value: ours,
            unit: "instructions",
            receipt: receipt.clone(),
        });
        if r["rank"].as_u64() == Some(1) {
            if let (Some(best), Some(who)) = (r["best"]["instructions"].as_f64(), r["best"]["checker"].as_str()) {
                out.push(Achievement {
                    id: format!("arena/{case}/over-{who}"),
                    ledger: ARENA,
                    better: Better::Higher,
                    value: best / ours,
                    unit: "×",
                    receipt: receipt.clone(),
                });
            }
        }
    }
    out
}

fn solvers(l: &Value) -> Vec<Achievement> {
    let receipt = json!({"ledger": SOLVERS, "date": l["metadata"]["date"], "cpu": l["metadata"]["cpu"]});
    let mut out = Vec::new();
    for f in l["families"].as_array().expect("the solver families") {
        let family = f["id"].as_str().unwrap();
        for row in f["rows"].as_array().unwrap() {
            let n = row["n"].as_u64().unwrap();
            let Some(ours) = row["ours_ms"].as_f64() else { continue };
            if ours < SAT_RESOLVABLE_MS {
                continue;
            }
            out.push(Achievement { id: format!("sat/{family}/{n}/ms"), ledger: SOLVERS, better: Better::Lower, value: ours, unit: "ms", receipt: receipt.clone() });
            for other in row["others"].as_array().unwrap() {
                let (Some(who), Some(ms)) = (other["solver"].as_str(), other["ms"].as_f64()) else { continue };
                if ms > ours {
                    out.push(Achievement {
                        id: format!("sat/{family}/{n}/over-{who}"),
                        ledger: SOLVERS,
                        better: Better::Higher,
                        value: ms / ours,
                        unit: "×",
                        receipt: receipt.clone(),
                    });
                }
            }
        }
    }
    out
}

/// Every achievement the ledgers record now, by id.
fn achievements() -> BTreeMap<String, Achievement> {
    let mut all = Vec::new();
    all.extend(scoreboard(&ledger(SCOREBOARD)));
    all.extend(card_runs(&ledger(CARD_RUNS)));
    all.extend(decode_images(&ledger(DECODE_IMAGES)));
    all.extend(arena(&ledger(ARENA)));
    all.extend(solvers(&ledger(SOLVERS)));
    let mut by_id = BTreeMap::new();
    for a in all {
        let id = a.id.clone();
        assert!(by_id.insert(id.clone(), a).is_none(), "two achievements are named `{id}`");
    }
    by_id
}

/// One floor as the ledger of floors holds it.
#[derive(Clone, Debug)]
struct Floor {
    ledger: String,
    better: Better,
    achieved: f64,
    bound: f64,
    unit: String,
    receipt: Value,
}

/// The ledger of floors: its own path, or another `LOGOS_SPEED_FLOORS` names (a proposed ledger of floors, read before
/// it replaces this one).
fn floors_path() -> PathBuf {
    std::env::var("LOGOS_SPEED_FLOORS").map_or_else(|_| root().join(FLOORS), |p| {
        let p = Path::new(&p);
        if p.is_absolute() { p.to_path_buf() } else { root().join(p) }
    })
}

fn floors() -> BTreeMap<String, Floor> {
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(floors_path()).unwrap_or_else(|e| {
        panic!("{FLOORS}: {e} — record it with LOGOS_UPDATE_FIXTURES=1 cargo nextest run -p logicaffeine-tests --test speed_floor_lock -E 'test(every_achievement_on_record_has_a_floor)'")
    }))
    .expect("the floors parse");
    assert_eq!(doc["margin"].as_f64(), Some(MARGIN), "{FLOORS}: the margin is the lock's");
    doc["floors"]
        .as_object()
        .expect("the floors, by id")
        .iter()
        .map(|(id, f)| {
            let floor = Floor {
                ledger: f["ledger"].as_str().unwrap_or_else(|| panic!("{id}: its ledger")).to_string(),
                better: Better::parse(f["better"].as_str().unwrap_or_else(|| panic!("{id}: which way is better"))),
                achieved: f["achieved"].as_f64().unwrap_or_else(|| panic!("{id}: what was achieved")),
                bound: f["bound"].as_f64().unwrap_or_else(|| panic!("{id}: its bound")),
                unit: f["unit"].as_str().unwrap_or_default().to_string(),
                receipt: f["receipt"].clone(),
            };
            (id.clone(), floor)
        })
        .collect()
}

/// The ledger of floors, each achievement's best kept: what it records now, raised to every better measurement.
fn record() {
    let path = floors_path();
    let old = if path.exists() { floors() } else { BTreeMap::new() };
    let mut out = serde_json::Map::new();
    for (id, f) in &old {
        out.insert(
            id.clone(),
            json!({"ledger": f.ledger, "better": f.better.name(), "unit": f.unit, "achieved": f.achieved, "bound": f.bound, "receipt": f.receipt}),
        );
    }
    for (id, a) in achievements() {
        let (achieved, receipt) = match old.get(&id) {
            Some(f) if a.better.best(f.achieved, a.value) == f.achieved => (f.achieved, f.receipt.clone()),
            _ => (a.value, a.receipt.clone()),
        };
        out.insert(
            id,
            json!({"ledger": a.ledger, "better": a.better.name(), "unit": a.unit, "achieved": achieved, "bound": a.better.bound(achieved), "receipt": receipt}),
        );
    }
    let doc = json!({
        "_what": "A floor for every speed on record, five per cent short of the best it reached: a rate or a margin over a rival at 95% of it, a time or an instruction count at its 1/0.95. Written by speed_floor_lock under LOGOS_UPDATE_FIXTURES=1, which only raises a floor; held by speed_floor_lock.",
        "schema": 1,
        "margin": MARGIN,
        "floors": out,
    });
    std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap() + "\n").unwrap();
}

fn recording() -> bool {
    std::env::var("LOGOS_UPDATE_FIXTURES").is_ok_and(|v| v == "1")
}

#[test]
fn every_achievement_on_record_has_a_floor() {
    if recording() {
        record();
    }
    let floors = floors();
    let unfloored: Vec<String> = achievements().into_keys().filter(|id| !floors.contains_key(id)).collect();
    assert!(
        unfloored.is_empty(),
        "{} achievements on record have no floor — record them (LOGOS_UPDATE_FIXTURES=1 cargo nextest run -p logicaffeine-tests --test speed_floor_lock -E 'test(every_achievement_on_record_has_a_floor)'):\n  {}",
        unfloored.len(),
        unfloored.join("\n  ")
    );
}

#[test]
fn every_measurement_on_record_holds_its_floor() {
    let now = achievements();
    let fresh: BTreeSet<String> = overrides().into_keys().collect();
    let mut fell = Vec::new();
    let mut unmeasured = Vec::new();
    let mut held = 0;
    for (id, f) in floors() {
        if !fresh.is_empty() && !fresh.contains(&f.ledger) {
            continue;
        }
        let Some(a) = now.get(&id) else {
            unmeasured.push(id);
            continue;
        };
        if f.better.holds(a.value, f.bound) {
            held += 1;
        } else {
            fell.push(format!(
                "{id}: {:.4} {} where its floor is {:.4} ({} is better; the best on record is {:.4}, {:?})",
                a.value,
                f.unit,
                f.bound,
                f.better.name(),
                f.achieved,
                f.receipt
            ));
        }
    }
    if !fresh.is_empty() {
        eprintln!("speed floors against the fresh measurement: {held} held, {} fell, {} not measured in it", fell.len(), unmeasured.len());
        for id in &unmeasured {
            eprintln!("  not measured: {id}");
        }
        assert!(held + fell.len() > 0, "the fresh measurement measured no floored achievement: {fresh:?}");
    } else {
        assert!(
            unmeasured.is_empty(),
            "these floors' achievements are gone from their ledgers — a speed nobody measures any more is a speed nobody protects; \
             drop a floor only by hand, with the reason:\n  {}",
            unmeasured.join("\n  ")
        );
    }
    assert!(fell.is_empty(), "{} speeds fell through their floors:\n  {}", fell.len(), fell.join("\n  "));
}

#[test]
fn every_floor_is_the_margin_short_of_what_was_achieved() {
    for (id, f) in floors() {
        let want = f.better.bound(f.achieved);
        assert!(
            (f.bound - want).abs() <= 1e-9 * want.abs().max(1.0),
            "{id}: its bound {} is not the margin from its best {} ({} is better): a floor is derived, never typed",
            f.bound,
            f.achieved,
            f.better.name()
        );
        assert!(f.achieved.is_finite() && f.achieved > 0.0, "{id}: achieved {}", f.achieved);
    }
}

#[test]
fn a_measurement_past_its_floor_by_more_than_the_margin_raises_it() {
    if !overrides().is_empty() {
        return;
    }
    let floors = floors();
    let gains: Vec<String> = achievements()
        .into_iter()
        .filter_map(|(id, a)| {
            let f = floors.get(&id)?;
            f.better.gained(a.value, f.achieved).then(|| format!("{id}: {:.4} {} against a best on record of {:.4}", a.value, f.unit, f.achieved))
        })
        .collect();
    assert!(
        gains.is_empty(),
        "these measurements beat their best by more than the margin — lock the gains in \
         (LOGOS_UPDATE_FIXTURES=1 cargo nextest run -p logicaffeine-tests --test speed_floor_lock -E 'test(every_achievement_on_record_has_a_floor)'):\n  {}",
        gains.join("\n  ")
    );
}

#[test]
fn every_floor_names_its_receipt() {
    for (id, f) in floors() {
        assert!(root().join(&f.ledger).exists(), "{id}: its ledger {} exists", f.ledger);
        assert_eq!(f.receipt["ledger"].as_str(), Some(f.ledger.as_str()), "{id}: its receipt names its ledger");
        let when = ["measured_at", "date", "image_sof_sha256", "bitstream_sha256"].iter().any(|k| f.receipt[*k].as_str().is_some_and(|s| !s.is_empty()));
        assert!(when, "{id}: its receipt says when, or on what image, it was measured: {}", f.receipt);
    }
}

/// The floor arithmetic, on numbers chosen so each comparison is decided by the margin alone.
#[test]
fn the_margin_judges_both_directions() {
    assert!(Better::Higher.holds(95.0, Better::Higher.bound(100.0)));
    assert!(!Better::Higher.holds(94.9, Better::Higher.bound(100.0)));
    assert!(Better::Lower.holds(105.2, Better::Lower.bound(100.0)));
    assert!(!Better::Lower.holds(105.3, Better::Lower.bound(100.0)));
    assert!(Better::Higher.gained(105.3, 100.0) && !Better::Higher.gained(105.2, 100.0));
    assert!(Better::Lower.gained(94.9, 100.0) && !Better::Lower.gained(95.1, 100.0));
    assert_eq!(Better::Higher.best(3.0, 4.0), 4.0);
    assert_eq!(Better::Lower.best(3.0, 4.0), 3.0);
}
