//! ═══════════════════════════════════════════════════════════════════════
//! TT GEMM LEDGER TOOL LOCK — the ONLY writer of `benchmarks/silicon/tenstorrent_gemm.json`
//! (`scripts/tt-gemm-ledger.py`) is itself locked, so a Tenstorrent cycle number can never again
//! be typed by hand, quoted without its math fidelity, or recorded without a PASS.
//!
//! A. A row is refused (exit 3, `REFUSING:`) without a PASS line, on a `LOGOS_MM_FAIL`, on any ttsim
//!    `[N] ERROR:` line, or on a non-zero exit code — nothing is written.
//! B. The knobs the PROGRAM printed (fidelity / fp32 / L1-acc) must equal the knobs the harness
//!    REQUESTED, else refused — the fidelity confound (LoFi-with-config vs HiFi2 default vs our
//!    HiFi4) can never be recorded silently again.
//! C. A legacy bare `LOGOS_MM_PASS` is accepted only with `--knobs-from-source`, and the row says so.
//! D. Every row carries its committed ttsim log EXCERPT (banner + CFG + PASS + the exit heartbeat)
//!    and a sha256 of the program sources; re-adding the same key supersedes the older row.
//! E. Comparisons are DERIVED (never typed): matched-numerics pairs become `comparisons`,
//!    cross-knob pairs become `unmatched_comparisons` with the knob diff spelled out.
//! F. `verify` re-derives everything and re-parses every excerpt — a hand-edited cycle count fails.
//! G. A tt-metal example row may only cite a shape that example's SOURCE can run (the 25,180
//!    "640^3" mislabel of a 2048-class run is structurally impossible).
//!
//! Honest boundary: these are FIXTURE logs (hand-made, tiny) — they lock the TOOL, not any
//! measurement. Measured rows come only from `scripts/tt-matmul-measure.sh` on ttsim.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(tt_gemm_ledger_tool_lock)'`
//! ═══════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "common/mod.rs"]
mod common;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("tt_ledger")
}

const SO_SHA: &str = "590f6d38aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa4194";
const TT_METAL_SHA: &str = "9aa797dec5ed247251aa142deb3cea0af2df4321";

fn tool(args: &[&str]) -> Output {
    Command::new("python3")
        .arg(repo_root().join("scripts/tt-gemm-ledger.py"))
        .args(args)
        .current_dir(repo_root())
        .output()
        .expect("spawn python3 scripts/tt-gemm-ledger.py")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

fn ledger(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("read ledger"))
        .expect("parse ledger")
}

fn s(p: &Path) -> String {
    p.to_str().unwrap().to_string()
}

/// A logos_emitted `add` over `log`, with the fixture's two program sources, at `now`.
fn add_logos(led: &Path, log: &str, now: &str, extra: &[&str]) -> Output {
    let fx = fixtures();
    let mut args: Vec<String> = vec![
        "add".into(),
        "--ledger".into(), s(led),
        "--stack".into(), "logos_emitted".into(),
        "--log".into(), s(&fx.join(log)),
        "--exit-code".into(), "0".into(),
        "--program".into(), "logos_matmul".into(),
        "--program-origin".into(), "compiler-emitted host + compute kernel; stock tt-metal dataflow kernels".into(),
        "--generated".into(), "compute,host".into(),
        "--shape".into(), "640".into(), "640".into(), "640".into(),
        "--chip".into(), "wormhole_b0".into(),
        "--reproduce".into(), "scripts/tt-matmul-measure.sh logos --shape 640 640 640 --chip wormhole --strategy simple --fidelity HiFi4 --fp32 0 --l1acc 0".into(),
        "--now".into(), now.into(),
        "--program-src".into(), s(&fx.join("src/logos_matmul.cpp")), s(&fx.join("src/logos_mm.cpp")),
    ];
    args.extend(extra.iter().map(|x| x.to_string()));
    let a: Vec<&str> = args.iter().map(|x| x.as_str()).collect();
    tool(&a)
}

/// A ttnn_production `add` over `log` (origin comes from the CFG line the runner printed).
fn add_ttnn(led: &Path, log: &str, now: &str) -> Output {
    let fx = fixtures();
    let led_s = s(led);
    let log_s = s(&fx.join(log));
    let src_s = s(&fx.join("src/logos_mm.cpp"));
    tool(&[
        "add", "--ledger", &led_s, "--stack", "ttnn_production", "--log", &log_s, "--exit-code", "0",
        "--program", "ttnn_matmul", "--shape", "640", "640", "640", "--chip", "wormhole_b0",
        "--reproduce", "scripts/tt-matmul-measure.sh ttnn --shape 640 640 640 --chip wormhole --config ours --fidelity HiFi4 --fp32 0 --l1acc 0",
        "--now", now, "--program-src", &src_s,
    ])
}

#[test]
fn the_tool_refuses_a_row_without_a_pass_or_with_a_ttsim_error() {
    if !common::require_tool("python3") {
        return;
    }
    for (log, why) in [
        ("no_pass.log", "no PASS line"),
        ("fail.log", "LOGOS_MM_FAIL"),
        (
            "error.log",
            "a ttsim [N] ERROR: line even though a PASS line follows",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let led = dir.path().join("tenstorrent_gemm.json");
        let o = add_logos(&led, log, "2026-09-02T00:00:00Z", &[]);
        assert_eq!(
            o.status.code(),
            Some(3),
            "{log} ({why}) must be refused with exit 3: {}",
            stderr(&o)
        );
        assert!(
            stderr(&o).starts_with("REFUSING:"),
            "{log}: stderr must start with REFUSING:, got {}",
            stderr(&o)
        );
        assert!(!led.exists(), "{log}: a refused row must write NOTHING");
    }
    // A non-zero exit code is refused even when the log looks fine.
    let dir = tempfile::tempdir().unwrap();
    let led = dir.path().join("tenstorrent_gemm.json");
    let fx = fixtures();
    let led_s = s(&led);
    let log_s = s(&fx.join("pass.log"));
    let src_s = s(&fx.join("src/logos_mm.cpp"));
    let o = tool(&[
        "add",
        "--ledger",
        &led_s,
        "--stack",
        "logos_emitted",
        "--log",
        &log_s,
        "--exit-code",
        "1",
        "--program",
        "logos_matmul",
        "--program-origin",
        "x",
        "--generated",
        "compute,host",
        "--shape",
        "640",
        "640",
        "640",
        "--chip",
        "wormhole_b0",
        "--reproduce",
        "x",
        "--program-src",
        &src_s,
    ]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "non-zero exit code must be refused: {}",
        stderr(&o)
    );
    assert!(stderr(&o).starts_with("REFUSING:"));
    assert!(!led.exists());
}

#[test]
fn the_tool_refuses_when_the_program_reports_different_knobs_than_requested() {
    if !common::require_tool("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let led = dir.path().join("tenstorrent_gemm.json");
    // The program printed HiFi2 but the harness asked for HiFi4 — the confound must be caught here.
    let o = add_logos(
        &led,
        "knob_hifi2.log",
        "2026-09-02T00:00:00Z",
        &["--requested-knobs", "HiFi4,0,0"],
    );
    assert_eq!(
        o.status.code(),
        Some(3),
        "knob mismatch must be refused: {}",
        stderr(&o)
    );
    let e = stderr(&o);
    assert!(
        e.starts_with("REFUSING:") && e.contains("HiFi2") && e.contains("HiFi4"),
        "stderr must name both: {e}"
    );
    assert!(!led.exists());
    // Matching knobs are accepted.
    let o = add_logos(
        &led,
        "knob_hifi2.log",
        "2026-09-02T00:00:00Z",
        &["--requested-knobs", "HiFi2,0,0"],
    );
    assert!(
        o.status.success(),
        "matching knobs must be accepted: {}",
        stderr(&o)
    );
    assert_eq!(ledger(&led)["rows"][0]["knobs"]["fidelity"], "HiFi2");
}

#[test]
fn the_tool_refuses_a_legacy_bare_pass_unless_knobs_come_from_source() {
    if !common::require_tool("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let led = dir.path().join("tenstorrent_gemm.json");
    let o = add_logos(&led, "legacy_pass.log", "2026-09-02T00:00:00Z", &[]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "a bare LOGOS_MM_PASS carries no knobs — refused without --knobs-from-source: {}",
        stderr(&o)
    );
    assert!(stderr(&o).starts_with("REFUSING:"));
    let src = s(&fixtures().join("src/logos_matmul.cpp"));
    let o = add_logos(
        &led,
        "legacy_pass.log",
        "2026-09-02T00:00:00Z",
        &["--knobs-from-source", &src],
    );
    assert!(
        o.status.success(),
        "bare PASS + knobs-from-source must be accepted: {}",
        stderr(&o)
    );
    let row = &ledger(&led)["rows"][0];
    // The fixture host hardcodes HiFi4 + fp32_dest_acc_en = true and never mentions packer_l1_acc.
    assert_eq!(row["knobs"]["fidelity"], "HiFi4");
    assert_eq!(row["knobs"]["fp32_dest_acc"], true);
    assert_eq!(row["knobs"]["packer_l1_acc"], false);
    let ks = row["knobs_source"].as_str().unwrap();
    assert!(
        ks.contains("logos_matmul.cpp") && ks.contains("HiFi4"),
        "knobs_source must say where the knobs came from: {ks}"
    );
    assert_eq!(
        row["pcc"], 0.9992974,
        "pcc comes from the `-- PCC =` line when the PASS line is bare"
    );
    assert_eq!(row["cycles"], 9394);
    assert_eq!(row["strategy"], "ReuseMcast");
}

#[test]
fn the_tool_writes_a_row_and_its_excerpt_from_a_pass_log() {
    if !common::require_tool("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let led = dir.path().join("tenstorrent_gemm.json");
    let o = add_logos(
        &led,
        "pass.log",
        "2026-09-02T01:23:45Z",
        &["--requested-knobs", "HiFi4,0,0"],
    );
    assert!(o.status.success(), "add must succeed: {}", stderr(&o));
    let d = ledger(&led);
    assert_eq!(d["schema_version"], 1);
    assert_eq!(d["provenance"], "sim_functional_not_timed");
    assert_eq!(d["generated_by"], "scripts/tt-gemm-ledger.py");
    assert!(d["_comment"].as_str().unwrap().contains("NEVER hand-edit"));
    let rows = d["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r["stack"], "logos_emitted");
    assert_eq!(r["program"], "logos_matmul");
    assert_eq!(
        r["program_origin"],
        "compiler-emitted host + compute kernel; stock tt-metal dataflow kernels"
    );
    assert_eq!(
        r["generated"],
        serde_json::json!({"compute": true, "dataflow": false, "host": true})
    );
    assert_eq!(
        r["shape"],
        serde_json::json!({"m": 640, "n": 640, "k": 640})
    );
    assert_eq!(r["chip"], "wormhole_b0");
    assert_eq!(r["dtype"], "bf16");
    assert_eq!(r["strategy"], "Simple");
    assert_eq!(
        r["cfg"],
        serde_json::json!({"grid": "5x5", "per_core": "4x4", "out_block": "4x4", "in0_block_w": 20, "subblock": "2x4"})
    );
    assert_eq!(
        r["knobs"],
        serde_json::json!({"fidelity": "HiFi4", "fp32_dest_acc": false, "packer_l1_acc": false, "math_approx_mode": false})
    );
    assert_eq!(
        r["knobs_source"],
        "LOGOS_MM_PASS line printed by the program"
    );
    assert_eq!(r["cycles"], 9735);
    assert_eq!(r["pcc"], 0.99929);
    assert_eq!(r["pcc_floor"], 0.99);
    assert_eq!(r["pass"], true);
    assert_eq!(
        r["pass_line"],
        "LOGOS_MM_PASS fidelity=HiFi4 fp32=0 l1acc=0 pcc=0.99929 floor=0.99"
    );
    assert_eq!(r["measured_at"], "2026-09-02T01:23:45Z");
    assert_eq!(r["provenance"], "sim_functional_not_timed");
    assert_eq!(r["current"], true);
    assert!(r["superseded_by"].is_null());
    assert_eq!(r["ttsim"]["so"], "libttsim_wh.so");
    assert_eq!(r["ttsim"]["so_sha256"], SO_SHA);
    assert_eq!(r["ttsim"]["version"], "v1.10.2");
    assert_eq!(r["tt_metal_sha"], TT_METAL_SHA);
    assert!(r["reproduce"]
        .as_str()
        .unwrap()
        .starts_with("scripts/tt-matmul-measure.sh"));
    let key = r["key"].as_str().unwrap();
    assert_eq!(
        key,
        "wormhole_b0/640x640x640/logos_emitted/logos_matmul/Simple/HiFi4-fp32_0-l1acc_0"
    );
    assert_eq!(r["id"], format!("{key}/2026-09-02T01:23:45Z"));
    // The program sha is real (64 hex) and a function of the sources.
    let sha = r["program_sha256"].as_str().unwrap().to_string();
    assert_eq!(sha.len(), 64);
    assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    // The committed excerpt lives beside the ledger and re-parses to the same facts.
    let log_rel = r["log"].as_str().unwrap();
    let log_path = dir.path().join(log_rel);
    assert!(
        log_path.exists(),
        "excerpt {log_rel} must be written next to the ledger"
    );
    assert!(log_rel.starts_with("ttsim_logs/") && log_rel.ends_with(".log"));
    let ex = std::fs::read_to_string(&log_path).unwrap();
    for needle in [
        "LOGOS_TT_ENV chip=wormhole_b0",
        "LOGOS_MM_CFG strategy=Simple",
        "LOGOS_MM_PASS fidelity=HiFi4",
        "[9735] 12.3 seconds (0.8 KHz)",
    ] {
        assert!(ex.contains(needle), "excerpt must keep `{needle}`");
    }
    assert_eq!(r["log_full_sha256"].as_str().unwrap().len(), 64);

    // Changing a program source changes the sha (the sha is over the sources, not a constant).
    let alt = dir.path().join("alt_src");
    std::fs::create_dir_all(&alt).unwrap();
    std::fs::write(alt.join("logos_matmul.cpp"), "// different program\n").unwrap();
    std::fs::write(alt.join("logos_mm.cpp"), "// different kernel\n").unwrap();
    let fx = fixtures();
    let led_s = s(&led);
    let log_s = s(&fx.join("pass.log"));
    let a1 = s(&alt.join("logos_matmul.cpp"));
    let a2 = s(&alt.join("logos_mm.cpp"));
    let o = tool(&[
        "add",
        "--ledger",
        &led_s,
        "--stack",
        "logos_emitted",
        "--log",
        &log_s,
        "--exit-code",
        "0",
        "--program",
        "logos_matmul",
        "--program-origin",
        "x",
        "--generated",
        "compute,host",
        "--shape",
        "640",
        "640",
        "640",
        "--chip",
        "wormhole_b0",
        "--reproduce",
        "scripts/tt-matmul-measure.sh x",
        "--now",
        "2026-09-02T02:00:00Z",
        "--program-src",
        &a1,
        &a2,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let d = ledger(&led);
    // ★ SELECT THE ROW WE JUST WROTE, BY ITS TIMESTAMP — not "the first current row".
    //
    // Both rows are legitimately current here, and the tool is right to keep them so. The first add's
    // `--reproduce` names `--strategy simple`, which makes its `plan_source` **pinned**; this one's does
    // not, so it is **compiler**. `tt-gemm-ledger.py` only supersedes rows at the same key that share a
    // plan_source, because a pinned run and a compiler-chosen run are different EXPERIMENTS — one shows
    // what the machine can do when told exactly what to do, the other what the compiler decides unaided,
    // and the tool's own comment records that letting the last one win used to erase the evidence every
    // "beats the vendor" claim rests on.
    //
    // So `find(current)` returned the OLDER row and compared the fixture's sha against itself. The
    // assertion below is about hashing, and it has to read the row whose sources it changed.
    let newest = d["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["measured_at"] == "2026-09-02T02:00:00Z")
        .expect("the row just added must be in the ledger");
    assert_eq!(
        newest["current"], true,
        "a compiler-plan row at a fresh plan_source is current"
    );
    assert_ne!(
        newest["program_sha256"].as_str().unwrap(),
        sha,
        "different sources must hash differently"
    );
}

#[test]
fn the_tool_regenerates_comparisons_and_supersedes_older_rows() {
    if !common::require_tool("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let led = dir.path().join("tenstorrent_gemm.json");
    let o = add_logos(&led, "pass.log", "2026-09-02T01:00:00Z", &["--headline"]);
    assert!(o.status.success(), "{}", stderr(&o));
    // Without a second stack there is nothing to compare — but the headline key is recorded.
    let d = ledger(&led);
    assert_eq!(
        d["headline_key"],
        serde_json::json!({"shape": "640x640x640", "chip": "wormhole_b0", "knobs": "HiFi4-fp32_0-l1acc_0"})
    );
    assert_eq!(d["comparisons"].as_array().unwrap().len(), 0);

    // A MATCHED ttnn row (HiFi4, fp32 off, l1acc off — the same knobs) yields one comparison.
    let o = add_ttnn(&led, "ttnn_hifi4.log", "2026-09-02T01:10:00Z");
    assert!(o.status.success(), "{}", stderr(&o));
    let d = ledger(&led);
    let cmp = d["comparisons"].as_array().unwrap();
    assert_eq!(
        cmp.len(),
        1,
        "one matched-numerics group: {}",
        serde_json::to_string_pretty(&d["comparisons"]).unwrap()
    );
    let c = &cmp[0];
    assert_eq!(c["id"], "wormhole_b0/640x640x640/HiFi4-fp32_0-l1acc_0");
    assert_eq!(c["matched_numerics"], true);
    assert_eq!(c["subject_cycles"], 9735);
    let vs = &c["vs"]["ttnn_production_our_config"];
    assert_eq!(vs["cycles"], 7000);
    let ratio = vs["ratio"].as_f64().unwrap();
    assert!(
        (ratio - 9735.0 / 7000.0).abs() < 1e-9,
        "ratio must be subject/other = {ratio}"
    );
    assert_eq!(vs["verdict"], "more_cycles");
    assert!(
        c["vs"]["ttnn_production_default"].is_null(),
        "no default-heuristic row at these knobs yet"
    );
    assert_eq!(c["same_simulator"], true);
    // The headline now resolves to that comparison and its subject.
    assert_eq!(
        d["headline"]["comparison"],
        "wormhole_b0/640x640x640/HiFi4-fp32_0-l1acc_0"
    );
    assert_eq!(d["headline"]["subject"], "wormhole_b0/640x640x640/logos_emitted/logos_matmul/Simple/HiFi4-fp32_0-l1acc_0/2026-09-02T01:00:00Z");

    // An UNMATCHED ttnn row (LoFi + l1acc) never becomes a comparison; it becomes an unmatched entry
    // that spells out exactly which knobs differ.
    let o = add_ttnn(&led, "ttnn_lofi.log", "2026-09-02T01:20:00Z");
    assert!(o.status.success(), "{}", stderr(&o));
    let d = ledger(&led);
    assert_eq!(
        d["comparisons"].as_array().unwrap().len(),
        1,
        "a LoFi row must NOT create a matched comparison"
    );
    let un = d["unmatched_comparisons"].as_array().unwrap();
    assert_eq!(
        un.len(),
        1,
        "{}",
        serde_json::to_string_pretty(&d["unmatched_comparisons"]).unwrap()
    );
    let u = &un[0];
    assert_eq!(u["matched_numerics"], false);
    assert_eq!(
        u["why_unmatched"],
        serde_json::json!(["fidelity: HiFi4 vs LoFi", "packer_l1_acc: false vs true"])
    );
    assert!((u["ratio"].as_f64().unwrap() - 9735.0 / 6154.0).abs() < 1e-9);
    assert!(u["note"].as_str().unwrap().contains("NOT a head-to-head"));
    // The default-heuristic row at HiFi2 is likewise unmatched against the HiFi4 subject.
    let o = add_ttnn(&led, "ttnn_default.log", "2026-09-02T01:30:00Z");
    assert!(o.status.success(), "{}", stderr(&o));
    let d = ledger(&led);
    assert_eq!(d["unmatched_comparisons"].as_array().unwrap().len(), 2);
    // ttnn default rows carry no cfg (ttnn picks the program factory itself) and its origin.
    let def = d["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["program_origin"] == "ttnn.matmul default heuristic")
        .unwrap();
    assert!(def["cfg"].is_null());
    assert_eq!(
        def["strategy"], "vendor_default",
        "the two ttnn origins are distinct programs and must never share a key"
    );
    let ours = d["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["program_origin"] == "ttnn.matmul with LOGOS program config")
        .unwrap();
    assert_eq!(ours["strategy"], "vendor_our_config");
    assert_ne!(ours["key"], def["key"]);

    // Re-adding the same key supersedes: the old row stays (history), only the new one is current.
    let o = add_logos(&led, "pass.log", "2026-09-02T03:00:00Z", &[]);
    assert!(o.status.success(), "{}", stderr(&o));
    let d = ledger(&led);
    let same_key: Vec<&serde_json::Value> = d["rows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| {
            r["key"]
                == "wormhole_b0/640x640x640/logos_emitted/logos_matmul/Simple/HiFi4-fp32_0-l1acc_0"
        })
        .collect();
    assert_eq!(same_key.len(), 2);
    let old = same_key
        .iter()
        .find(|r| r["measured_at"] == "2026-09-02T01:00:00Z")
        .unwrap();
    let new = same_key
        .iter()
        .find(|r| r["measured_at"] == "2026-09-02T03:00:00Z")
        .unwrap();
    assert_eq!(old["current"], false);
    assert_eq!(old["superseded_by"], new["id"]);
    assert_eq!(new["current"], true);
    assert_eq!(
        d["comparisons"].as_array().unwrap().len(),
        1,
        "comparisons use current rows only"
    );
    assert_eq!(d["comparisons"][0]["subject"], new["id"]);

    // `show` renders the matched table without error.
    let led_s = s(&led);
    let o = tool(&["show", "--ledger", &led_s]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        out.contains("9735") && out.contains("7000") && out.contains("HiFi4"),
        "show must print the matched pair: {out}"
    );
}

#[test]
fn verify_fails_on_a_hand_edited_cycle_count() {
    if !common::require_tool("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let led = dir.path().join("tenstorrent_gemm.json");
    assert!(
        add_logos(&led, "pass.log", "2026-09-02T01:00:00Z", &["--headline"])
            .status
            .success()
    );
    assert!(add_ttnn(&led, "ttnn_hifi4.log", "2026-09-02T01:10:00Z")
        .status
        .success());
    let led_s = s(&led);
    let o = tool(&["verify", "--ledger", &led_s]);
    assert!(
        o.status.success(),
        "a freshly written ledger must verify: {}",
        stderr(&o)
    );

    // Hand-edit the subject's cycles: the excerpt still says 9735, so verify must fail loudly.
    let text = std::fs::read_to_string(&led).unwrap();
    let edited = text.replacen("\"cycles\": 9735", "\"cycles\": 6000", 1);
    assert_ne!(text, edited, "fixture must contain the cycle count to edit");
    std::fs::write(&led, edited).unwrap();
    let o = tool(&["verify", "--ledger", &led_s]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "a hand-edited cycle count must fail verify: {}",
        stderr(&o)
    );
    assert!(
        stderr(&o).contains("9735") && stderr(&o).contains("6000"),
        "verify must name the drift: {}",
        stderr(&o)
    );

    // Hand-editing a derived ratio is caught too.
    std::fs::write(&led, &text).unwrap();
    let d = ledger(&led);
    let mut d2 = d.clone();
    d2["comparisons"][0]["vs"]["ttnn_production_our_config"]["ratio"] = serde_json::json!(0.5);
    std::fs::write(&led, serde_json::to_string_pretty(&d2).unwrap()).unwrap();
    let o = tool(&["verify", "--ledger", &led_s]);
    assert_eq!(
        o.status.code(),
        Some(1),
        "a hand-edited ratio must fail verify: {}",
        stderr(&o)
    );
}

#[test]
fn the_tool_refuses_an_example_row_at_a_shape_the_example_cannot_run() {
    if !common::require_tool("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let led = dir.path().join("tenstorrent_gemm.json");
    let fx = fixtures();
    let led_s = s(&led);
    let log_s = s(&fx.join("example_pass.log"));
    let tree = s(&fx.join("examples_tree"));
    let add_example = |program: &str, shape: [&str; 3], now: &str| {
        tool(&[
            "add",
            "--ledger",
            &led_s,
            "--stack",
            "tt_metal_example",
            "--log",
            &log_s,
            "--exit-code",
            "0",
            "--program",
            program,
            "--shape",
            shape[0],
            shape[1],
            shape[2],
            "--chip",
            "wormhole_b0",
            "--reproduce",
            "scripts/tt-matmul-measure.sh example --name matmul_multi_core --chip wormhole",
            "--now",
            now,
        ])
    };
    // Before the examples are scanned, no example row can be recorded at all.
    let o = add_example(
        "matmul_multi_core",
        ["640", "640", "640"],
        "2026-09-02T01:00:00Z",
    );
    assert_eq!(
        o.status.code(),
        Some(3),
        "an example row needs scan-examples first: {}",
        stderr(&o)
    );
    assert!(stderr(&o).starts_with("REFUSING:") && stderr(&o).contains("scan-examples"));

    let o = tool(&[
        "scan-examples",
        "--ledger",
        &led_s,
        "--tt-metal-home",
        &tree,
    ]);
    assert!(
        o.status.success(),
        "scan-examples must succeed on the fixture tree: {}",
        stderr(&o)
    );
    let d = ledger(&led);
    let ves = &d["vendor_example_shapes"];
    assert_eq!(
        ves["matmul_multi_core"]["shapes"],
        serde_json::json!(["640x640x640"])
    );
    assert_eq!(
        ves["matmul_multicore_reuse_mcast"]["shapes"],
        serde_json::json!(["2048x1024x512", "3584x3072x768"])
    );
    assert!(
        ves["matmul_multi_core"]["source"]
            .as_str()
            .unwrap()
            .contains("matmul_multi_core.cpp:"),
        "source must cite path:line"
    );
    assert_eq!(
        ves["matmul_multi_core"]["knobs"],
        serde_json::json!({"fidelity": "HiFi4", "fp32_dest_acc": false, "packer_l1_acc": false, "math_approx_mode": false})
    );
    assert_eq!(ves["matmul_multi_core"]["pcc_floor"], 0.97);
    assert_eq!(ves["matmul_multicore_reuse"]["pcc_floor"], 0.99);

    // The 25,180 mislabel: the mcast example cannot run 640^3, and multi_core cannot run 2048-class.
    let o = add_example(
        "matmul_multi_core",
        ["2048", "1024", "512"],
        "2026-09-02T01:00:00Z",
    );
    assert_eq!(
        o.status.code(),
        Some(3),
        "shape the example cannot run must be refused: {}",
        stderr(&o)
    );
    assert!(stderr(&o).contains("2048x1024x512") && stderr(&o).contains("640x640x640"));
    let o = add_example(
        "matmul_multicore_reuse_mcast",
        ["640", "640", "640"],
        "2026-09-02T01:00:00Z",
    );
    assert_eq!(o.status.code(), Some(3), "{}", stderr(&o));

    // A legal example row derives its knobs, pcc floor, origin, and sources from the scan.
    let o = add_example(
        "matmul_multi_core",
        ["640", "640", "640"],
        "2026-09-02T01:00:00Z",
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let d = ledger(&led);
    let r = &d["rows"][0];
    assert_eq!(r["stack"], "tt_metal_example");
    assert_eq!(
        r["program_origin"],
        "tt-metal programming example matmul_multi_core"
    );
    assert_eq!(r["knobs"]["fidelity"], "HiFi4");
    assert_eq!(r["knobs"]["packer_l1_acc"], false);
    assert!(r["knobs_source"]
        .as_str()
        .unwrap()
        .contains("matmul_multi_core.cpp"));
    assert_eq!(r["pcc"], 0.99985);
    assert_eq!(r["pcc_floor"], 0.97);
    assert_eq!(r["pass_line"], "Test Passed");
    assert_eq!(r["cycles"], 9735);
    assert!(r["cfg"].is_null());
    assert_eq!(r["strategy"], "example");
    assert!(r["generated"].is_null());
    assert_eq!(r["program_sha256"].as_str().unwrap().len(), 64);
    let led_s2 = s(&led);
    let o = tool(&["verify", "--ledger", &led_s2]);
    assert!(o.status.success(), "{}", stderr(&o));
}

#[test]
fn the_tool_refuses_a_run_whose_kernels_were_jit_compiled_during_the_measurement() {
    // MEASURED on ttsim: the same emitted program recorded 9,735 cycles with every kernel in the JIT cache and
    // 13,543 with three kernels compiled during the run — the device clock ticks while the host compiles. A
    // row therefore requires `JIT cache stats: N/N hits`; anything colder is refused with the fix spelled out.
    if !common::require_tool("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let led = dir.path().join("tenstorrent_gemm.json");
    let o = add_logos(&led, "cold_jit.log", "2026-09-02T00:00:00Z", &[]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "a cold-JIT run must be refused: {}",
        stderr(&o)
    );
    let e = stderr(&o);
    assert!(
        e.starts_with("REFUSING:") && e.contains("10/13") && e.to_lowercase().contains("warm"),
        "the refusal names the cache state and the remedy: {e}"
    );
    assert!(!led.exists());
    // A warm run records the cache state on the row.
    let o = add_logos(&led, "pass.log", "2026-09-02T00:00:00Z", &[]);
    assert!(o.status.success(), "{}", stderr(&o));
    let r = &ledger(&led)["rows"][0];
    assert_eq!(r["jit_cache"], serde_json::json!({"hits": 13, "total": 13}));
    // A log without the JIT line at all is refused too — the cache state is part of the measurement.
    let fx = fixtures();
    let nojit = dir.path().join("nojit.log");
    let text = std::fs::read_to_string(fx.join("pass.log"))
        .unwrap()
        .lines()
        .filter(|l| !l.contains("JIT cache stats"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&nojit, text).unwrap();
    let led2 = dir.path().join("l2.json");
    let led2_s = s(&led2);
    let nojit_s = s(&nojit);
    let src_s = s(&fx.join("src/logos_mm.cpp"));
    let o = tool(&[
        "add",
        "--ledger",
        &led2_s,
        "--stack",
        "logos_emitted",
        "--log",
        &nojit_s,
        "--exit-code",
        "0",
        "--program",
        "logos_matmul",
        "--program-origin",
        "x",
        "--generated",
        "compute,host",
        "--shape",
        "640",
        "640",
        "640",
        "--chip",
        "wormhole_b0",
        "--reproduce",
        "scripts/tt-matmul-measure.sh x",
        "--program-src",
        &src_s,
    ]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "a log without the JIT cache line must be refused: {}",
        stderr(&o)
    );
}

/// ★ A ROW FROM ONE INSTRUMENT MAY NOT SUPERSEDE A ROW FROM ANOTHER.
///
/// Superseding is per (key, plan_source): a later run of the same program at the same knobs replaces the
/// earlier one. That is right when both were measured the same way, and wrong across INSTRUMENTS, because
/// the instrument decides what a row can even say. The instrumented build carries the work meter —
/// `matmul_retired`, `noc_txns`, `noc_bytes`, `tensix_inst` — and the stock release carries none of it.
///
/// This is not hypothetical. Bringing Blackhole up, the same vendor example was run on the instrumented
/// build and then on the stock one to prove the patch moves no clock (it does not: 309,062 cycles under
/// both). The stock run, being newer, SUPERSEDED the instrumented one — so the ledger's current
/// Blackhole example row lost its work meter, and the only head-to-head on that chip was left comparing a
/// row that has work against a row that does not.
///
/// The instrument is part of the measurement, exactly as the pinned-vs-compiler plan source is. Two runs
/// under two builds are two experiments, and `rederive` already knows this — it retires rows measured on a
/// build the ledger has replaced. Superseding has to agree with it, or the two fight and a key can end up
/// with no current row at all.
///
/// NON-VACUITY: the same two adds with the SAME simulator sha DO supersede (asserted below), so this is a
/// property of the instrument and not of adding twice.
#[test]
fn a_row_from_a_different_simulator_build_never_supersedes_one_from_the_current_build() {
    if !common::require_tool("python3") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let led = dir.path().join("tenstorrent_gemm.json");
    assert!(add_logos(&led, "pass.log", "2026-09-02T01:00:00Z", &[])
        .status
        .success());

    // A later run of the SAME program on a DIFFERENT simulator build.
    let o = add_logos(&led, "pass_other_sim.log", "2026-09-02T02:00:00Z", &[]);
    assert!(o.status.success(), "{}", stderr(&o));
    let d = ledger(&led);
    let rows = d["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    let first = rows
        .iter()
        .find(|r| r["measured_at"] == "2026-09-02T01:00:00Z")
        .unwrap();
    assert_eq!(
        first["current"], true,
        "the row measured on the current instrument must stay live when a DIFFERENT build is added — the \
         instrument decides what a row can say, and the newer one here says less"
    );
    assert!(
        first["superseded_by"].is_null(),
        "and it must not be marked superseded by it"
    );

    // Non-vacuity: two runs on the SAME build still supersede, so this is about the instrument.
    let led2 = dir.path().join("same_build.json");
    assert!(add_logos(&led2, "pass.log", "2026-09-02T01:00:00Z", &[])
        .status
        .success());
    assert!(add_logos(&led2, "pass.log", "2026-09-02T02:00:00Z", &[])
        .status
        .success());
    let d2 = ledger(&led2);
    let old = d2["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["measured_at"] == "2026-09-02T01:00:00Z")
        .unwrap();
    assert_eq!(
        old["current"], false,
        "a re-run on the SAME build does supersede"
    );
}

/// ── A CHECKED-IN EXCERPT NAMES NO ONE'S HOME. A ttsim run log is full of this machine's paths — the tt-metal tree it
/// built in, the repository, the JIT cache under the home directory — and the excerpt the ledger commits is written
/// with each spelled portably (`$TT_METAL_HOME`, `$LOGOS_ROOT`, `~`) AT WRITE TIME, so no run, a gate's included,
/// can put a home path in the tree. The row it records is the one the raw log gives.
///
/// ★ MUTATION PROBE (2026-09-28, `logs/sn50/probe_portable_noop.txt`): `portable()` returns its text unchanged → this
/// test reds: "the excerpt names the home directory".
#[test]
fn the_committed_excerpt_names_no_home_directory() {
    if !common::require_tool("python3") {
        return;
    }
    let home = std::env::var("HOME").expect("a home directory to spell");
    let tt = std::env::var("TT_METAL_HOME").unwrap_or_else(|_| format!("{home}/tt-metal"));
    let repo = repo_root().canonicalize().expect("repo root");
    let dir = tempfile::tempdir().unwrap();
    let raw = std::fs::read_to_string(fixtures().join("pass.log")).expect("the pass fixture");
    let paths = format!(
        "Building {tt}/build_Release/programming_examples/matmul/logos_matmul\nJIT cache: {home}/.cache/tt-metal-cache/1234\nEmitted from {}/target/tt/logos_spatial\n",
        repo.display()
    );
    let log = dir.path().join("pass_with_paths.log");
    std::fs::write(&log, format!("{paths}{raw}")).unwrap();
    let led = dir.path().join("tenstorrent_gemm.json");
    let fx = fixtures();
    let o = tool(&[
        "add", "--ledger", &s(&led), "--stack", "logos_emitted", "--log", &s(&log), "--exit-code", "0",
        "--program", "logos_matmul", "--program-origin", "compiler-emitted host + compute kernel; stock tt-metal dataflow kernels",
        "--generated", "compute,host", "--shape", "640", "640", "640", "--chip", "wormhole_b0",
        "--reproduce", "scripts/tt-matmul-measure.sh logos --shape 640 640 640 --chip wormhole --strategy simple --fidelity HiFi4 --fp32 0 --l1acc 0",
        "--now", "2026-09-02T01:23:45Z", "--program-src", &s(&fx.join("src/logos_matmul.cpp")), &s(&fx.join("src/logos_mm.cpp")),
        "--requested-knobs", "HiFi4,0,0",
    ]);
    assert!(o.status.success(), "add must succeed: {}", stderr(&o));
    let d = ledger(&led);
    let r = &d["rows"][0];
    assert_eq!(r["cycles"], 9735, "the row is the raw log's");
    let ex = std::fs::read_to_string(dir.path().join(r["log"].as_str().unwrap())).unwrap();
    for (what, path) in [
        ("the home directory", home.as_str()),
        ("the tt-metal tree", tt.as_str()),
        ("the repository", &repo.display().to_string()),
    ] {
        assert!(!ex.contains(path), "the excerpt names {what} ({path})");
    }
    for spelled in [
        "$TT_METAL_HOME/build_Release",
        "~/.cache/tt-metal-cache",
        "$LOGOS_ROOT/target/tt",
    ] {
        assert!(
            ex.contains(spelled),
            "the excerpt keeps the line, spelled `{spelled}…`"
        );
    }
    let o = tool(&["verify", "--ledger", &s(&led)]);
    assert!(
        o.status.success(),
        "the portable excerpt re-parses to the row: {}",
        stderr(&o)
    );
}
