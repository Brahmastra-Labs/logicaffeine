//! samba_exam_agility_lock — the rows of the Samba book's exam E10 (section 10.10 of `benchmarks/PREPARE_FOR_SAMBA.md`,
//! a new op, first run then fused): an op nobody has built yet, written in LOGOS as the VM's own loop, runs on the SN50
//! on its first compile, as the VM computes it — no sugar, no kernel, no registration.
//!
//!   • E10.1 — novel activations: squareplus through a function the program defines, and a piecewise activation that
//!     branches on its input and takes the library's `exp`; each lifts as ONE map and lowers as ONE section of the general
//!     form;
//!   • E10.2 — novel attention masks over a hand-written attention: a dilated causal mask and Llama 4's chunked-local
//!     one, each a map of the scores by their positions, between the product, the softmax and the product;
//!   • E10.3 — a novel recurrence: a channel decaying by a gate it computes from its own input, written turn by turn; it
//!     runs as a section that takes its turns in order, and lowers chunked: a carry nonlinear in the state still runs
//!     turn by turn;
//!   • E10.4 — the time from the written op to its fused, certified form: one compile, inside a minute, and the
//!     ledger's measured row for it (`benchmarks/silicon/samba_cells.json`, cell `C-SN50-SQUAREPLUS-AGILITY`) inside a
//!     minute too; `LOGOS_RECORD_SAMBA_CELLS=1` on the run measures five compiles and records the row;
//!   • E10.5 — a novel routing function: biased, sharpened sigmoid scores, the top two by two arg-maxes, their scores
//!     normalized into the combine.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_agility_lock`
//! Record E10.4's row: `LOGOS_RECORD_SAMBA_CELLS=1 cargo nextest run -p logicaffeine-tests --cargo-profile test-opt
//! --test samba_exam_agility_lock -E 'test(the_time_to_a_fused_new_op_is_recorded)'`

#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
#[allow(dead_code)]
mod tensor_probes;
#[path = "common/written_programs.rs"]
#[allow(dead_code)]
mod written_programs;
#[path = "common/samba_cells.rs"]
#[allow(dead_code)]
mod samba_cells;

use std::path::{Path, PathBuf};
use std::time::Instant;

use logicaffeine_compile::spatial::certificate::certify_choreography;
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::lower::rdu::pretty_neutral;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_kernel::interface::lean::package::sha256_hex;
use serde::Serialize;
use serde_json::{json, Value};
use written_programs::{filled, lifted, lowered, on_the_sn50, seeded, shows, special_compute, walk};

/// Sixty-four inputs in `[-3, 3]`, every one exact in bf16.
fn inputs() -> String {
    filled("X", 1, 64, &|i| seeded("X", i) * 3.0)
}

/// `Y = f(X)` element by element, `turn` the loop's body; every element of `Y` shown.
fn activation(functions: &str, turn: &str) -> String {
    format!("{functions}## Main\n{}Let Y be a 1 by 64 tensor of Real.\nRepeat for i from 1 to 64:\n{turn}{}", inputs(), shows("Y", 64))
}

/// Squareplus, `(x + √(x² + 4)) / 2`, through a function the program defines.
fn squareplus() -> String {
    activation(
        "## To squareplus (x: Real) -> Real:\n    Return (x + sqrt(x * x + 4.0)) / 2.0.\n\n",
        "    Set item i of Y to squareplus(item i of X).\n",
    )
}

/// The section the SN50 runs a novel op as: one, of the general form alone.
fn one_general_section(sp: &logicaffeine_compile::spatial::ir::SpatialProgram, what: &str) {
    assert_eq!(sp.sections.len(), 1, "{what}: one section, not {}", sp.sections.len());
    assert!(special_compute(sp).is_empty(), "{what}: every compute stage is the general form: {:?}", special_compute(sp));
}

#[test]
fn a_new_activation_runs_on_its_first_compile() {
    let piecewise = activation(
        "",
        "    Let x be item i of X.\n    If x is greater than 0.0:\n        Set item i of Y to x / (1.0 + x).\n    Otherwise:\n        Set item i of Y to x * exp(x).\n",
    );
    for (src, what) in [(squareplus(), "squareplus"), (piecewise, "a piecewise activation")] {
        assert_eq!(lifted(&src).iter().map(|l| l.0).collect::<Vec<_>>(), vec!["Map"], "{what}: the loop lifts as one map");
        let sp = on_the_sn50(&src, what, false);
        one_general_section(&sp, what);
    }
}

/// A hand-written attention over 8 positions of 4 dimensions: `S = Q·Kᵀ`, the scores `S` masked into `M` by `keep`
/// (a condition on the row `r` and the column `c`) and scaled by 1/√4, `P = softmax(M)` row by row, `O = P·V`.
fn attention(keep: &str) -> String {
    format!(
        "## Main\n{}{}{}Let Kt be transpose of K.\nLet S be Q matmul Kt.\nLet M be a 8 by 8 tensor of Real.\n\
         Repeat for r from 1 to 8:\n    Repeat for c from 1 to 8:\n        Let s be item ((r - 1) * 8 + c) of S.\n\
         \x20       If {keep}:\n            Set item ((r - 1) * 8 + c) of M to s * 0.5.\n\
         \x20       Otherwise:\n            Set item ((r - 1) * 8 + c) of M to -1000000000.0.\n\
         Let P be softmax of M.\nLet O be P matmul V.\n{}",
        filled("Q", 8, 4, &|i| seeded("Q", i)),
        filled("K", 8, 4, &|i| seeded("K", i)),
        filled("V", 8, 4, &|i| seeded("V", i)),
        shows("O", 32)
    )
}

#[test]
fn a_new_attention_mask_runs_on_its_first_compile() {
    for (keep, what) in [
        ("c is at most r and (r - c) % 3 is not 1", "a dilated causal mask"),
        ("c is at most r and (c - 1) / 4 equals (r - 1) / 4", "a chunked-local mask of 4"),
    ] {
        let src = attention(keep);
        let kinds: Vec<&str> = lifted(&src).iter().map(|l| l.0).collect();
        assert_eq!(kinds, vec!["Transpose", "Matmul", "Map", "FusedRow", "Matmul"], "{what}: the mask lifts as one map between the products");
        on_the_sn50(&src, what, false);
    }
}

/// A channel recurrence over 16 turns of 8 channels, written turn by turn: `H[t] = step` from the gate `a = σ(A[t])`, the
/// input `x = X[t]` and the carried `h = H[t − 1]`; the first turn starts from zero.
fn recurrence(step: &str) -> String {
    format!(
        "## Main\n{}{}Let H be a 16 by 8 tensor of Real.\n\
         Repeat for t from 1 to 16:\n    Repeat for d from 1 to 8:\n\
         \x20       Let a be 1.0 / (1.0 + exp(0.0 - item ((t - 1) * 8 + d) of A)).\n\
         \x20       Let x be item ((t - 1) * 8 + d) of X.\n\
         \x20       If t equals 1:\n            Set item ((t - 1) * 8 + d) of H to (1.0 - a) * x.\n\
         \x20       Otherwise:\n            Let h be item ((t - 2) * 8 + d) of H.\n            Set item ((t - 1) * 8 + d) of H to {step}.\n{}",
        filled("A", 16, 8, &|i| seeded("A", i) * 2.0),
        filled("X", 16, 8, &|i| seeded("X", i)),
        shows("H", 128)
    )
}

#[test]
fn a_new_recurrence_runs_then_chunks() {
    // A carry under a nonlinearity takes its turns in order: one stage a turn, in one section.
    let nonlinear = recurrence("tanh(a * h + x)");
    assert_eq!(lifted(&nonlinear).iter().map(|l| l.0).collect::<Vec<_>>(), vec!["Scan"], "the loop lifts as one recurrence");
    let sp = on_the_sn50(&nonlinear, "a recurrence nonlinear in its carry", false);
    let turns = sp.stages.iter().filter(|s| s.name.contains("scan_turn") && !s.name.contains("difference")).count();
    assert_eq!(turns, 16, "the nonlinear recurrence runs turn by turn, a stage a turn");
    // Linear in the carry, the turns chunk: every chunk's own recurrence at once, then the carries across the chunks.
    let linear = recurrence("a * h + (1.0 - a) * x");
    assert_eq!(lifted(&linear).iter().map(|l| l.0).collect::<Vec<_>>(), vec!["Scan"], "the loop lifts as one recurrence");
    let sp = on_the_sn50(&linear, "a gated channel recurrence", false);
    let stages: Vec<&str> = sp.stages.iter().map(|s| s.name.as_str()).collect();
    for part in ["scan_decays", "scan_inputs", "scan_chunk_sum", "scan_chunk_decay", "scan_carry"] {
        assert!(stages.iter().any(|s| s.contains(part)), "the linear recurrence lowers chunked, with `{part}`: {stages:?}");
    }
    assert!(!stages.iter().any(|s| s.contains("scan_turn")), "the linear recurrence is never taken turn by turn: {stages:?}");
}

/// E10.4's cell of the ledger, the column its row records, and the budget it is held to.
const AGILITY_CELL: &str = "C-SN50-SQUAREPLUS-AGILITY";
const AGILITY_MEASURE: &str = "agility_s";
const AGILITY_BUDGET_S: f64 = 60.0;
/// The compiles a recording times; the row's value is the slowest.
const AGILITY_RUNS: usize = 5;
/// The ledger, and the log a recording writes beside it, under the repository.
const CELLS: &str = "benchmarks/silicon/samba_cells.json";
const AGILITY_LOG: &str = "logs/samba/agility-e10_4.log";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Whether this run records E10.4's row into the committed ledger: the explicit `LOGOS_RECORD_SAMBA_CELLS=1`.
fn recording() -> bool {
    std::env::var_os("LOGOS_RECORD_SAMBA_CELLS").is_some_and(|v| v == "1")
}

/// One compile of `src` from its source to its fused section for the SN50, certified: the program, and the wall seconds
/// the compile took. The program is one section of the general form whose certificate rechecks from the program alone.
fn fused_and_certified(src: &str, what: &str) -> (SpatialProgram, f64) {
    let start = Instant::now();
    let sp = lowered(src, &SpatialTarget::sn50_rdu()).unwrap_or_else(|why| panic!("{what} lowers: {why}"));
    let seconds = start.elapsed().as_secs_f64();
    one_general_section(&sp, what);
    let shipped = sp.certificate.clone().expect("the lowering ships a certificate");
    let mut bare = sp.clone();
    bare.certificate = None;
    let again = certify_choreography(&bare).unwrap_or_else(|e| panic!("{what} certifies on its own: {e:?}"));
    assert_eq!(again.schedule_digest, shipped.schedule_digest, "{what}: the certificate is the program's");
    (sp, seconds)
}

/// `YYYY-MM-DD` of the UTC day `days` after 1970-01-01.
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The first line of a file of this host's kernel, which a recording names its machine by.
fn kernel_fact(path: &str) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("a recording names this host by {path}: {e}"));
    text.lines().next().unwrap_or("").trim().to_string()
}

/// The ledger with `row` in place of any row of its cell and side: only the `rows` key is rewritten, every column kept
/// as it is written.
fn with_row(ledger: &str, row: Value) -> String {
    let current: Value = serde_json::from_str(ledger).expect("the ledger is JSON");
    let mut rows: Vec<Value> = current["rows"].as_array().expect("the ledger has rows").clone();
    rows.retain(|r| !(r["cell"] == row["cell"] && r["side"] == row["side"]));
    rows.push(row);
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(b" "));
    json!({ "rows": rows }).serialize(&mut ser).expect("the rows serialize");
    let tail = String::from_utf8(out).expect("JSON is UTF-8");
    let at = ledger.rfind("\n \"rows\": ").expect("the ledger's rows are its last key");
    format!("{}{}\n", &ledger[..at], &tail[1..])
}

/// E10.4's row, measured: `runs` the wall seconds of compiles of `src`, the slowest its value; the program's identity
/// (the source, the op the front end lifted, the neutral program it lowered to), this host's boot and the seconds its
/// front end takes to read the source; the run's log written beside the ledger.
fn record_agility(src: &str, sp: &SpatialProgram, runs: &[f64]) {
    let root = repo();
    let load = {
        let start = Instant::now();
        walk(src);
        start.elapsed().as_secs_f64()
    };
    let lifted_op = format!("{:?}", walk(src).stages.iter().map(|s| &s.op).collect::<Vec<_>>());
    let (source_sha, spec_digest, bundle_digest) =
        (sha256_hex(src.as_bytes()), sha256_hex(lifted_op.as_bytes()), sha256_hex(pretty_neutral(sp).as_bytes()));
    let slowest = runs.iter().copied().fold(0.0f64, f64::max);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("a clock after 1970");
    let date = civil_date((now.as_secs() / 86_400) as i64);
    let (boot, host) = (kernel_fact("/proc/sys/kernel/random/boot_id"), kernel_fact("/proc/sys/kernel/hostname"));
    let log = format!(
        "E10.4 squareplus, {date}, {host}, boot {boot}, load average {}\nsource sha256 {source_sha}\nlifted op sha256 {spec_digest}\n\
         neutral program sha256 {bundle_digest}\nfront end {load:.6} s\ncompiles (s): {}\nslowest {slowest:.6} s of a {AGILITY_BUDGET_S} s budget\n",
        kernel_fact("/proc/loadavg"),
        runs.iter().map(|r| format!("{r:.6}")).collect::<Vec<_>>().join(" ")
    );
    std::fs::create_dir_all(root.join(AGILITY_LOG).parent().expect("the log has a directory")).expect("the log's directory");
    std::fs::write(root.join(AGILITY_LOG), log).expect("the recording's log");
    let row = json!({
        "cell": AGILITY_CELL,
        "side": "ours",
        "provenance": "measured",
        "date": date,
        "values": { AGILITY_MEASURE: slowest },
        "runs": { AGILITY_MEASURE: runs },
        "conditions": "squareplus over 64 Reals, written as a loop and a function of the program's own: one compile from \
                       the source to its certified SN50 section, the slowest of five, on this host",
        "won": false,
        "program": { "spec_digest": spec_digest, "cut": "agility:squareplus", "source_sha256": source_sha },
        "bundle_digest": bundle_digest,
        "boot": { "id": boot, "host": host, "load_seconds": load },
        "log": AGILITY_LOG
    });
    let path = root.join(CELLS);
    let ledger = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    std::fs::write(&path, with_row(&ledger, row)).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

#[test]
fn the_time_to_a_fused_new_op_is_recorded() {
    // From the op as written to its fused, certified form is one compile, inside the budget.
    let src = squareplus();
    let (sp, seconds) = fused_and_certified(&src, "squareplus");
    eprintln!("RECEIPT E10.4 squareplus: written to fused and certified in {seconds:.3} s");
    assert!(seconds <= AGILITY_BUDGET_S, "a novel op is fused and certified in {seconds:.1} s, past {AGILITY_BUDGET_S} s");
    if recording() {
        let runs: Vec<f64> =
            std::iter::once(seconds).chain((1..AGILITY_RUNS).map(|_| fused_and_certified(&src, "squareplus").1)).collect();
        record_agility(&src, &sp, &runs);
    }
    // The ledger holds the row, measured, of this program, inside the budget.
    let path = repo().join(CELLS);
    let ledger: samba_cells::Ledger =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
            .expect("the ledger reads under schema 1");
    let row = ledger.rows.iter().find(|r| r.cell == AGILITY_CELL && r.side == "ours").unwrap_or_else(|| {
        panic!(
            "{CELLS} holds no row of {AGILITY_CELL}; record it: LOGOS_RECORD_SAMBA_CELLS=1 cargo nextest run -p logicaffeine-tests \
             --cargo-profile test-opt --test samba_exam_agility_lock -E 'test(the_time_to_a_fused_new_op_is_recorded)'"
        )
    });
    assert_eq!(row.provenance, samba_cells::Provenance::Measured, "{AGILITY_CELL}'s row is measured");
    let program = row.program.as_ref().expect("the row names its program");
    assert_eq!(program.source_sha256, sha256_hex(src.as_bytes()), "{AGILITY_CELL}'s row is of another program: re-record it");
    let recorded = *row.values.get(AGILITY_MEASURE).unwrap_or_else(|| panic!("{AGILITY_CELL}'s row has no `{AGILITY_MEASURE}`"));
    let runs = row.runs.get(AGILITY_MEASURE).unwrap_or_else(|| panic!("{AGILITY_CELL}'s row has no runs of `{AGILITY_MEASURE}`"));
    assert!(runs.len() >= AGILITY_RUNS, "{AGILITY_CELL}'s row stands on {} runs", runs.len());
    assert_eq!(recorded, runs.iter().copied().fold(0.0f64, f64::max), "{AGILITY_CELL}'s value is its slowest run");
    assert!(recorded <= AGILITY_BUDGET_S, "{AGILITY_CELL} was recorded at {recorded:.1} s, past {AGILITY_BUDGET_S} s");
}

/// A token of 8 routed over 6 experts by a router nobody has a kernel for: scores `σ(2·(x·Wr + B))`, the top two by two
/// arg-maxes, their scores normalized, and the experts' rows of `We` mixed by them.
fn routing() -> String {
    format!(
        "## Main\n{}{}{}{}{}\
         Let z be x matmul Wr.\nLet S be a 1 by 6 tensor of Real.\n\
         Repeat for e from 1 to 6:\n    Set item e of S to 1.0 / (1.0 + exp(0.0 - (item e of z + item e of B) * 2.0)).\n\
         Let e1 be argmax of S.\nLet S2 be a 1 by 6 tensor of Real.\n\
         Repeat for e from 1 to 6:\n    If e equals e1:\n        Set item e of S2 to -1.0.\n    Otherwise:\n        Set item e of S2 to item e of S.\n\
         Let e2 be argmax of S2.\nLet G be a 1 by 6 tensor of Real.\n\
         Repeat for e from 1 to 6:\n    If e equals e1 or e equals e2:\n        Set item e of G to item e of S.\n    Otherwise:\n        Set item e of G to 0.0.\n\
         Let total be G matmul Ones.\nLet W be a 1 by 6 tensor of Real.\n\
         Repeat for e from 1 to 6:\n    Set item e of W to item e of G / item 1 of total.\n\
         Let out be W matmul We.\n{}",
        filled("x", 1, 8, &|i| seeded("x", i)),
        filled("Wr", 8, 6, &|i| seeded("Wr", i)),
        filled("B", 1, 6, &|i| seeded("B", i) / 4.0),
        filled("Ones", 6, 1, &|_| 1.0),
        filled("We", 6, 8, &|i| seeded("We", i)),
        shows("out", 8)
    )
}

#[test]
fn a_new_routing_function_runs() {
    let src = routing();
    let kinds: Vec<&str> = lifted(&src).iter().map(|l| l.0).collect();
    assert_eq!(
        kinds,
        vec!["Matmul", "Map", "ArgMax", "Map", "ArgMax", "Map", "Matmul", "Map", "Matmul"],
        "the router's scores, its masks and its weights each lift as a map"
    );
    on_the_sn50(&src, "a biased, sharpened sigmoid router choosing 2 of 6", false);
}
