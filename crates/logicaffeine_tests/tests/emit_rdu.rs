//! `largo emit rdu` — the LOGOS → SambaNova RDU front door: ONE command, the lane chosen by program shape
//! and printed by name, the chip's descriptor chosen by `--chip`, the artifact chosen by `--format`, and
//! the vendor SDK reached through `--bridge`. Everything it emits is a certified NEUTRAL placement,
//! honestly SDK-gated for the vendor config bytes; nothing it prints is a time.
//!
//! ★ MUTATION PROBES (each run once when this lock landed; the lock went red, the impl was restored):
//!   - write `certificate.json` without recomputing `rechecked` (a literal `true`) and drop the digest →
//!     `emit_rdu_writes_the_four_files` goes red on the digest equality;
//!   - ignore `--chip` (always sn50) → `emit_rdu_chip_dial_selects_the_descriptor` goes red on `sram_kib`;
//!   - route a recognized-but-unhostable tensor program on to the next lane instead of refusing →
//!     `emit_rdu_refuses_what_the_chip_cannot_host_by_name` goes red (the refusal loses its name);
//!   - accept `--chip` on `emit c` silently → `emit_rdu_flags_are_refused_on_other_targets` goes red.

#[path = "cli_common/mod.rs"]
mod common;

use common::*;
use tempfile::tempdir;

/// The 2×2 integer matmul sugar — the smallest program the tensor lane lifts (`matmul_2x2x2`).
const MATMUL: &str = "## Main\nLet A be a 2 by 2 tensor of Int.\nSet item 1 of A to 1.\nSet item 2 of A to 2.\nSet item 3 of A to 3.\nSet item 4 of A to 4.\nLet B be a 2 by 2 tensor of Int.\nSet item 1 of B to 5.\nSet item 2 of B to 6.\nSet item 3 of B to 7.\nSet item 4 of B to 8.\nLet C be A matmul B.\nShow item 1 of C.\nShow item 2 of C.\nShow item 3 of C.\nShow item 4 of C.\n";

/// A program NO lane will ever host: a live client socket. The tensor lane does not recognize it, the loop
/// lane has no counted loop to take, and the whole-program lane's fabric ISA keeps `Tcp` on its frontier
/// (a real host socket is interpreter-fallback, `fabric_coverage_lock`). Chosen over `Show 1.` on purpose:
/// a bare `Show` becomes the whole-program lane's business once B-P2c lands, and this lock must keep
/// asserting a TYPED refusal across that landing without moving.
const SOCKET: &str = "## Main\n    Let c be tcpConnect(\"127.0.0.1\", 1).\n    Inspect c:\n        When Ok (conn):\n            Show \"open\".\n        When Err (e):\n            Show e.\n";

/// An RMSNorm at Qwen3.8's real width (1×5120): wider than one PCU's accumulator, hosted anyway by the
/// blocked-row lowering, which splits the row into sections that each fit.
fn rmsnorm_5120() -> String {
    let d = 5120;
    let mut s = String::from("## Main\n");
    s.push_str(&format!("Let X be a 1 by {d} tensor of Real.\n"));
    for i in 1..=8 {
        s.push_str(&format!("Set item {i} of X to {i}.0.\n"));
    }
    s.push_str(&format!("Let W be a 1 by {d} tensor of Real.\nSet item 1 of W to 0.5.\nLet Y be rmsnorm of X with weight W.\nShow item 1 of Y.\n"));
    s
}

/// A 1×5120 softmax: a row too wide for one PCU's accumulator, and — unlike RMSNorm — with no blocked
/// form, so the SN50 genuinely cannot host it today.
fn softmax_5120() -> String {
    let d = 5120;
    let mut s = String::from("## Main\n");
    s.push_str(&format!("Let X be a 1 by {d} tensor of Real.\n"));
    for i in 1..=8 {
        s.push_str(&format!("Set item {i} of X to {i}.0.\n"));
    }
    s.push_str("Let Y be softmax of X.\nShow item 1 of Y.\n");
    s
}

fn write(dir: &std::path::Path, name: &str, source: &str) -> String {
    let file = dir.join(name);
    std::fs::write(&file, source).unwrap();
    file.to_str().unwrap().to_string()
}

fn read_json(path: std::path::PathBuf) -> serde_json::Value {
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{} parses: {e}", path.display()))
}

/// Bare `largo emit rdu FILE` prints the neutral placement text; the lane and the chip go to stderr.
#[test]
fn emit_rdu_prints_the_placement_text() {
    let dir = tempdir().unwrap();
    let file = write(dir.path(), "mm.lg", MATMUL);
    let out = largo_in(dir.path(), &["emit", "rdu", &file]);
    assert_eq!(out.status.code(), Some(0), "emit rdu: {}", stderr(&out));
    let text = stdout(&out);
    for needle in [
        "program matmul_2x2x2",
        "phase Verified",
        "stage ",
        "site (0,0)",
        "engine site (0,0)",
    ] {
        assert!(
            text.contains(needle),
            "the placement text must contain {needle:?}:\n{text}"
        );
    }
    assert!(
        !text.contains("\"schema\""),
        "bare mode prints the placement TEXT, not the JSON"
    );
    let err = strip_ansi(&stderr(&out));
    assert!(
        err.contains("lane: tensor"),
        "the lane is printed by name on stderr: {err}"
    );
    assert!(
        err.contains("chip: sn50"),
        "the chip is printed on stderr: {err}"
    );
    assert!(
        err.to_lowercase().contains("sdk"),
        "the artifact says it is SDK-gated: {err}"
    );
}

/// `-o DIR` writes the four files — the neutral v2 envelope, the placement text, the certificate that
/// rechecks and carries the envelope's digest, and the MODELED report naming the lane — and stdout is quiet.
#[test]
fn emit_rdu_writes_the_four_files() {
    let dir = tempdir().unwrap();
    let file = write(dir.path(), "mm.lg", MATMUL);
    let bundle = dir.path().join("rdu");
    let out = largo_in(
        dir.path(),
        &["emit", "rdu", &file, "-o", bundle.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "emit rdu -o: {}", stderr(&out));
    assert!(
        !stdout(&out).contains("phase Verified"),
        "the placement must not also spill to stdout with -o"
    );

    let neutral = read_json(bundle.join("neutral.json"));
    assert_eq!(neutral["schema"], "logos.spatial.neutral");
    assert_eq!(neutral["version"], 2);
    assert_eq!(neutral["lane"], "spatial");
    assert_eq!(neutral["target"]["name"], "sn50-rdu");
    assert_eq!(neutral["target"]["chip"], "sn50");
    assert!(
        neutral["program"].is_object(),
        "the envelope carries the program"
    );

    let placement = std::fs::read_to_string(bundle.join("placement.txt")).expect("placement.txt");
    assert!(placement.contains("program matmul_2x2x2") && placement.contains("phase Verified"));

    let cert = read_json(bundle.join("certificate.json"));
    assert_eq!(
        cert["schedule_digest"], neutral["certificate"]["schedule_digest"],
        "the certificate file carries the envelope's digest"
    );
    assert_eq!(
        cert["rechecked"], true,
        "the certificate was rechecked against the program before being written"
    );
    assert!(cert["places"].is_number() && cert["transitions"].is_number());

    let report = read_json(bundle.join("report.json"));
    assert_eq!(report["provenance"], "modeled");
    assert_eq!(report["no_speed_claim"], true);
    assert_eq!(report["lane"], "tensor");
    assert_eq!(report["chip"], "sn50");
    assert_eq!(report["target"], "sn50-rdu");
    assert_eq!(report["program"]["name"], "matmul_2x2x2");
    assert_eq!(report["program"]["phase"], "Verified");
    assert_eq!(report["certificate"]["rechecked"], true);
    assert!(
        report["ii_report"]["pipeline_ii"].is_number(),
        "the report carries the modeled II view"
    );
    assert!(report["gated"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("sdk"));
}

/// `--chip sn40l` selects the SN40L descriptor (520 MiB SRAM); the default is the SN50 (432 MB). An unknown
/// chip is a usage error naming the two.
#[test]
fn emit_rdu_chip_dial_selects_the_descriptor() {
    let dir = tempdir().unwrap();
    let file = write(dir.path(), "mm.lg", MATMUL);
    let a = dir.path().join("a");
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            &file,
            "--chip",
            "sn40l",
            "-o",
            a.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "sn40l: {}", stderr(&out));
    let n = read_json(a.join("neutral.json"));
    assert_eq!(n["target"]["name"], "sn40l-rdu");
    assert_eq!(n["target"]["mem"]["sram_kib"], 532480);
    assert_eq!(read_json(a.join("report.json"))["chip"], "sn40l");

    let b = dir.path().join("b");
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            &file,
            "--chip",
            "SN50",
            "-o",
            b.to_str().unwrap(),
        ],
    );
    assert_eq!(out.status.code(), Some(0), "sn50: {}", stderr(&out));
    let n = read_json(b.join("neutral.json"));
    assert_eq!(n["target"]["name"], "sn50-rdu");
    assert_eq!(n["target"]["mem"]["sram_kib"], 442368);

    let out = largo_in(dir.path(), &["emit", "rdu", &file, "--chip", "sn99"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "an unknown chip is a usage error: {}",
        stderr(&out)
    );
    let err = strip_ansi(&stderr(&out));
    assert!(
        err.contains("sn40l") && err.contains("sn50"),
        "the usage error names the chips: {err}"
    );
}

/// A program no lane hosts is refused with a typed `rdu backend:` error that names every lane's reason.
#[test]
fn emit_rdu_refuses_a_non_tensor_program_typed() {
    let dir = tempdir().unwrap();
    let file = write(dir.path(), "sock.lg", SOCKET);
    let out = largo_in(dir.path(), &["emit", "rdu", &file]);
    assert_ne!(
        out.status.code(),
        Some(0),
        "a socket program must be refused"
    );
    assert!(
        stdout(&out).trim().is_empty(),
        "nothing is emitted on a refusal"
    );
    let err = strip_ansi(&stderr(&out));
    assert!(err.contains("rdu backend:"), "typed refusal: {err}");
    assert!(
        err.contains("tensor lane"),
        "the refusal names the tensor lane's verdict: {err}"
    );
    assert!(
        err.contains("loop lane"),
        "the refusal names the loop lane's verdict: {err}"
    );
    assert!(
        err.contains("whole-program lane"),
        "the refusal names the whole-program lane's verdict: {err}"
    );
}

/// A tensor program the lane RECOGNIZES but the chip cannot host is refused by name — the chip, the program,
/// the budget — and never quietly handed to another lane.
///
/// The subject is a 1×5120 SOFTMAX. It used to be a 1×5120 RMSNorm, until the blocked-row lowering made that
/// hostable (see `emit_rdu_hosts_a_row_wider_than_one_accumulator_by_blocking_it`); softmax has no blocked
/// form, so it is still genuinely beyond the chip, and the refusal is still the thing under test.
#[test]
fn emit_rdu_refuses_what_the_chip_cannot_host_by_name() {
    let dir = tempdir().unwrap();
    let file = write(dir.path(), "softmax.lg", &softmax_5120());
    let out = largo_in(dir.path(), &["emit", "rdu", &file]);
    assert_ne!(out.status.code(), Some(0), "a 1×5120 softmax exceeds one PCU's accumulator budget, has no blocked form, and must be refused");
    let err = strip_ansi(&stderr(&out));
    assert!(err.contains("rdu backend:"), "typed refusal: {err}");
    assert!(
        err.contains("sn50-rdu"),
        "the refusal names the chip: {err}"
    );
    assert!(
        err.contains("cannot host"),
        "the refusal says what happened: {err}"
    );
    assert!(
        err.contains("AccTilesExceeded"),
        "the refusal names the budget: {err}"
    );
    assert!(
        !err.contains("loop lane"),
        "a recognized tensor program is the tensor lane's to refuse, not the other lanes': {err}"
    );
}

/// A row wider than one PCU's accumulator is HOSTED when it has a blocked form: the 1×5120 RMSNorm (Qwen3.8's
/// real `d`) splits into sections that each fit, lowers, certifies, and emits the bundle.
#[test]
fn emit_rdu_hosts_a_row_wider_than_one_accumulator_by_blocking_it() {
    let dir = tempdir().unwrap();
    let file = write(dir.path(), "rms.lg", &rmsnorm_5120());
    let bundle = dir.path().join("rms");
    let out = largo_in(
        dir.path(),
        &["emit", "rdu", &file, "-o", bundle.to_str().unwrap()],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "a 1×5120 RMSNorm lowers through the blocked form: {}",
        stderr(&out)
    );
    let cert = read_json(bundle.join("certificate.json"));
    assert_eq!(
        cert["rechecked"], true,
        "the blocked lowering carries a certificate that rechecks"
    );
    let report = read_json(bundle.join("report.json"));
    assert_eq!(report["lane"], "tensor");
    assert!(
        report["coverage"]["sections"].as_u64().unwrap_or(0) > 1,
        "blocking means more than one section: {}",
        report["coverage"]
    );
}

/// `--format spatial` prints the Spatial regression app of the placed program (the B-P2b emitter, wired the
/// moment it landed); `--format sambaflow` prints the PyTorch module, a vendor-ingestion artifact that never
/// runs our compute; `-o DIR` writes each format's files beside the four. An unknown format is a usage error.
#[test]
fn emit_rdu_format_dial_emits_spatial_and_sambaflow() {
    let dir = tempdir().unwrap();
    let file = write(dir.path(), "mm.lg", MATMUL);
    let out = largo_in(dir.path(), &["emit", "rdu", &file, "--format", "spatial"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "--format spatial: {}",
        stderr(&out)
    );
    let scala = stdout(&out);
    for needle in [
        "import spatial.dsl._",
        "@spatial class Logos_matmul_2x2x2 extends SpatialTest",
        "DRAM[",
        "Accel",
    ] {
        assert!(
            scala.contains(needle),
            "the Spatial source must contain {needle:?}:\n{scala}"
        );
    }
    assert!(
        !scala.contains("\"schema\""),
        "bare mode prints the Spatial source alone"
    );

    let out = largo_in(dir.path(), &["emit", "rdu", &file, "--format", "sambaflow"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "--format sambaflow: {}",
        stderr(&out)
    );
    let py = stdout(&out);
    assert!(
        py.contains("vendor-ingestion artifact emitted by LOGOS"),
        "the module says what it is:\n{py}"
    );
    assert!(py.contains("import torch"), "a PyTorch module:\n{py}");

    let bundle = dir.path().join("f");
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            &file,
            "--format",
            "spatial",
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "--format spatial -o: {}",
        stderr(&out)
    );
    let scala = std::fs::read_to_string(bundle.join("spatial.scala")).expect("spatial.scala");
    assert!(scala.contains("@spatial class Logos_matmul_2x2x2"));
    assert!(
        bundle.join("neutral.json").exists() && bundle.join("certificate.json").exists(),
        "the four files come with every format"
    );
    let sf = dir.path().join("s");
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            &file,
            "--format",
            "sambaflow",
            "-o",
            sf.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "--format sambaflow -o: {}",
        stderr(&out)
    );
    for name in ["model.py", "compile.py", "manifest.json"] {
        assert!(sf.join(name).exists(), "{name} is written");
    }
    let compile_py = std::fs::read_to_string(sf.join("compile.py")).unwrap();
    assert!(
        compile_py.contains("TEMPLATE"),
        "the compile scaffold marks the unverified vendor API"
    );
    read_json(sf.join("manifest.json"));

    let out = largo_in(dir.path(), &["emit", "rdu", &file, "--format", "elf"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "an unknown format is a usage error"
    );
}

/// `--format report` prints the MODELED report; `--format vllm` prints the Qwen3.8 deployment plan for the
/// chip (one SambaRack of RDUs, bf16, the model's full context, the op list the backend must support).
#[test]
fn emit_rdu_format_report_and_vllm_print_the_plans() {
    let dir = tempdir().unwrap();
    let file = write(dir.path(), "mm.lg", MATMUL);
    let out = largo_in(dir.path(), &["emit", "rdu", &file, "--format", "report"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "--format report: {}",
        stderr(&out)
    );
    let report: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("the report is JSON");
    assert_eq!(report["lane"], "tensor");
    assert_eq!(report["provenance"], "modeled");
    assert_eq!(report["no_speed_claim"], true);

    let out = largo_in(dir.path(), &["emit", "rdu", &file, "--format", "vllm"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "--format vllm: {}",
        stderr(&out)
    );
    let plan: serde_json::Value = serde_json::from_str(&stdout(&out)).expect("the plan is JSON");
    assert_eq!(plan["model"]["id"], "Qwen/Qwen3.8-27B");
    assert_eq!(plan["serve"]["tensor_parallel_size"], 16);
    assert_eq!(plan["serve"]["dtype"], "bfloat16");
    assert_eq!(plan["chip"]["part"], "SN50");
    let ops: Vec<&str> = plan["ops_the_backend_must_support"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["op"].as_str().unwrap())
        .collect();
    assert!(
        ops.contains(&"gated_delta") && ops.contains(&"gqa_attention"),
        "{ops:?}"
    );

    let bundle = dir.path().join("v");
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            &file,
            "--format",
            "vllm",
            "--chip",
            "sn40l",
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "--format vllm -o: {}",
        stderr(&out)
    );
    let plan = read_json(bundle.join("vllm_plan.json"));
    assert_eq!(plan["chip"]["part"], "SN40L");
    assert_eq!(
        plan["serve"]["tensor_parallel_size"], 16,
        "an SN40L node is sixteen sockets"
    );
    assert!(
        bundle.join("neutral.json").exists() && bundle.join("report.json").exists(),
        "the four files come with every format"
    );
}

/// `--bridge NAME` hands the certified artifact to an SDK bridge by name: `mock` round-trips it (the config
/// IS the neutral bytes, stamped with the certificate digest); `sambaflow` refuses by name until the vendor
/// SDK exists; an unknown name is refused naming the bridges there are.
#[test]
fn emit_rdu_bridge_dispatches_by_name() {
    let dir = tempdir().unwrap();
    let file = write(dir.path(), "mm.lg", MATMUL);
    let out = largo_in(dir.path(), &["emit", "rdu", &file, "--bridge", "mock"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "--bridge mock: {}",
        stderr(&out)
    );
    let config: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("the mock bridge's config is the neutral JSON");
    assert_eq!(config["schema"], "logos.spatial.neutral");
    let err = strip_ansi(&stderr(&out));
    assert!(
        err.contains("bridge: mock") && err.contains("logos.spatial.neutral+json"),
        "the bridge and its format are named on stderr: {err}"
    );
    let digest = format!(
        "{:016x}",
        config["certificate"]["schedule_digest"].as_u64().unwrap()
    );
    assert!(
        err.contains(&digest),
        "the certificate digest the config was stamped with is printed: {err}"
    );

    let bundle = dir.path().join("m");
    let out = largo_in(
        dir.path(),
        &[
            "emit",
            "rdu",
            &file,
            "--bridge",
            "mock",
            "-o",
            bundle.to_str().unwrap(),
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "--bridge mock -o: {}",
        stderr(&out)
    );
    let meta = read_json(bundle.join("bridge.json"));
    assert_eq!(meta["bridge"], "mock");
    assert_eq!(meta["format"], "logos.spatial.neutral+json");
    assert_eq!(meta["certificate_digest"], digest);
    assert_eq!(meta["config_file"], "vendor_config.json");
    let vendor = read_json(bundle.join("vendor_config.json"));
    assert_eq!(
        vendor,
        read_json(bundle.join("neutral.json")),
        "the mock bridge's config is the neutral artifact byte for byte"
    );

    let out = largo_in(dir.path(), &["emit", "rdu", &file, "--bridge", "sambaflow"]);
    assert_ne!(out.status.code(), Some(0));
    let err = strip_ansi(&stderr(&out));
    assert!(
        err.contains("rdu bridge: sambaflow is not implemented"),
        "{err}"
    );
    assert!(err.contains("needs the vendor SDK"), "{err}");

    let out = largo_in(dir.path(), &["emit", "rdu", &file, "--bridge", "nonesuch"]);
    assert_ne!(out.status.code(), Some(0));
    let err = strip_ansi(&stderr(&out));
    assert!(
        err.contains("rdu bridge:")
            && err.contains("nonesuch")
            && err.contains("mock")
            && err.contains("sambaflow"),
        "an unknown bridge names the ones there are: {err}"
    );
}

/// The RDU dials are the RDU's: on any other target they are refused, never silently ignored.
#[test]
fn emit_rdu_flags_are_refused_on_other_targets() {
    let dir = tempdir().unwrap();
    let file = write(dir.path(), "hello.lg", "## Main\nShow 6 * 7.\n");
    for flag in [
        ["--chip", "sn50"],
        ["--format", "report"],
        ["--bridge", "mock"],
    ] {
        let out = largo_in(dir.path(), &["emit", "c", &file, flag[0], flag[1]]);
        assert_ne!(
            out.status.code(),
            Some(0),
            "{} on `emit c` must be refused",
            flag[0]
        );
        let err = strip_ansi(&stderr(&out));
        assert!(
            err.contains(flag[0]) && err.contains("emit rdu"),
            "{}: the refusal names the flag and where it applies: {err}",
            flag[0]
        );
    }
    let out = largo_in(dir.path(), &["emit", "c", &file]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "without the dials `emit c` is untouched: {}",
        stderr(&out)
    );
}
