//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! SN50 SCRIPTS LOCK — the two shell scripts the SN50 day starts with, pinned.
//!
//!   * `scripts/accel/sn50-intake.sh` probes an SDK environment (`import sambaflow`, `snrun`/`samba`/`vllm`/
//!     `sambastack`/`snfadm` on PATH, `vllm --version`, the pip list, `/opt/sambaflow`, `SAMBA*`/`SN*`
//!     variable NAMES, docs dirs) and prints a filled INTAKE CARD with the day's decision tree; `--dry-run`
//!     prints the card without probing anything.
//!   * `scripts/accel/sambacloud-smoke.sh` streams one chat completion from SambaNova Cloud and prints TTFT and
//!     tok/s as one JSON line — the baseline number for the day; `--record` writes it, dated, to
//!     `benchmarks/results/sambacloud_baseline.json`; `--dry-run` prints the request shape with the key
//!     REDACTED and never touches the network.
//!
//! Both are pinned to exist, be executable, answer `--help` with exit 0, refuse an unknown flag, and NEVER
//! print or pass on a command line the value of the API key — the one thing a smoke script must not leak.
//!
//! The smoke's measuring path is exercisable OFFLINE: `SAMBANOVA_STREAM_FILE=<captured SSE>` replays a
//! captured stream through the same program with no key and no network, and the record it writes is stamped
//! `replayed_sse_capture` — never `measured_sambacloud_api`, which only a live request earns. That seam is
//! why TTFT, the token count and the record document are LOCKED here rather than taken on trust: the first
//! version of this script piped curl into `python3 -` under a heredoc, so the heredoc won stdin from the pipe
//! and the stream was never read at all — a silent nothing that no key-less test could see.
//!
//! ★ MUTATION PROBES (each run once, observed red, restored):
//!   - add `echo "$SAMBANOVA_API_KEY"` anywhere in `scripts/accel/sambacloud-smoke.sh` →
//!     `neither_script_ever_echoes_a_key` goes RED on the static scan;
//!   - make `--dry-run` exit 1 in `sn50-intake.sh` → `the_intake_dry_run_prints_the_card_without_probing`;
//!   - stamp a replay `measured_sambacloud_api` in `record.py` (ignore `replayed_from`) →
//!     `the_measuring_path_runs_offline_and_a_replay_is_never_stamped_as_a_measurement`;
//!   - restore the `python3 - <<PY` heredoc form of the measurement → the same test goes RED with
//!     `ttft_s: null` (the stream is never read).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test sn50_scripts_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#[path = "cli_common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root resolves from the CLI crate")
}

fn script(name: &str) -> PathBuf {
    repo_root().join("scripts/accel").join(name)
}

const SCRIPTS: [&str; 2] = ["sn50-intake.sh", "sambacloud-smoke.sh"];

/// Run a script with `args` under an EMPTY key environment plus `env`, from a HOME that holds no key file.
fn run(name: &str, args: &[&str], env: &[(&str, &str)]) -> (Output, tempfile::TempDir) {
    let home = tempfile::tempdir().unwrap();
    let mut cmd = Command::new("bash");
    cmd.arg(script(name))
        .args(args)
        .current_dir(repo_root())
        .env_remove("SAMBANOVA_API_KEY")
        .env("HOME", home.path());
    for (k, v) in env {
        cmd.env(k, v);
    }
    (
        cmd.output().unwrap_or_else(|e| panic!("run {name}: {e}")),
        home,
    )
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

#[test]
fn both_scripts_exist_and_are_executable() {
    use std::os::unix::fs::PermissionsExt;
    for name in SCRIPTS {
        let p = script(name);
        assert!(p.is_file(), "{} exists", p.display());
        let mode = std::fs::metadata(&p).unwrap().permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "{} is executable (mode {mode:o})",
            p.display()
        );
        let head = std::fs::read_to_string(&p).unwrap();
        assert!(
            head.starts_with("#!/usr/bin/env bash"),
            "{name} is a bash script"
        );
        assert!(head.contains("set -"), "{name} sets shell options");
    }
}

#[test]
fn help_exits_zero_and_names_the_usage() {
    for name in SCRIPTS {
        let (o, _h) = run(name, &["--help"], &[]);
        assert_eq!(
            o.status.code(),
            Some(0),
            "{name} --help exits 0: {}",
            text(&o)
        );
        let out = String::from_utf8_lossy(&o.stdout);
        assert!(
            out.contains("Usage") || out.contains("usage"),
            "{name} --help prints a usage: {out}"
        );
        assert!(out.contains(name), "{name} --help names the script: {out}");
        assert!(
            out.contains("--dry-run"),
            "{name} --help documents --dry-run: {out}"
        );
        let (bad, _h) = run(name, &["--no-such-flag"], &[]);
        assert_ne!(bad.status.code(), Some(0), "{name} refuses an unknown flag");
        assert!(
            text(&bad).contains("no-such-flag"),
            "{name} names the flag it refused: {}",
            text(&bad)
        );
    }
}

#[test]
fn the_intake_dry_run_prints_the_card_without_probing() {
    let (o, _h) = run("sn50-intake.sh", &["--dry-run"], &[]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "intake --dry-run exits 0: {}",
        text(&o)
    );
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("SN50 INTAKE CARD"), "the card:\n{out}");
    for needle in [
        "SambaFlow",
        "PEF",
        "vLLM",
        "qwen3_5",
        "Gated DeltaNet",
        "TP",
        "neutral.json",
        "certif",
        "SambaCloud",
        "modeled",
        "not probed",
        "sambaflow",
        "snrun",
        "vllm",
        "sambastack",
        "snfadm",
        "/opt/sambaflow",
        "SAMBA*",
        "0:00",
        "0:30",
    ] {
        assert!(
            out.contains(needle),
            "the dry-run card mentions `{needle}`:\n{out}"
        );
    }
    assert!(
        !out.contains("yes:"),
        "a dry run probes nothing, so no probe answers yes:\n{out}"
    );
    assert!(
        !out.contains("no:") && !out.lines().any(|l| l.trim_end().ends_with(" no")),
        "a dry run probes nothing, so no probe answers no:\n{out}"
    );
}

#[test]
fn the_intake_probes_when_asked_and_labels_every_answer() {
    let (o, _h) = run(
        "sn50-intake.sh",
        &[],
        &[("SAMBA_TEST_ONLY_NAME", "value-that-must-not-print-9f8e7d")],
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "intake exits 0 on a box without the SDK too: {}",
        text(&o)
    );
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("SN50 INTAKE CARD"));
    assert!(!out.contains("not probed"), "a real run probes:\n{out}");
    assert!(
        out.contains("SAMBA_TEST_ONLY_NAME"),
        "environment variable NAMES matching SAMBA*/SN* are listed:\n{out}"
    );
    assert!(
        !out.contains("value-that-must-not-print-9f8e7d"),
        "environment variable VALUES are never printed:\n{out}"
    );
    for probe in [
        "import sambaflow",
        "snrun",
        "samba",
        "vllm",
        "sambastack",
        "snfadm",
        "pip",
        "/opt/sambaflow",
    ] {
        assert!(
            out.contains(probe),
            "the probe `{probe}` is reported:\n{out}"
        );
    }
}

#[test]
fn the_smoke_dry_run_prints_the_request_shape_and_never_the_key() {
    let secret = "sk-test-SECRET-9f8e7d6c";
    let (o, _h) = run(
        "sambacloud-smoke.sh",
        &["--dry-run"],
        &[("SAMBANOVA_API_KEY", secret)],
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "smoke --dry-run exits 0: {}",
        text(&o)
    );
    let all = text(&o);
    for needle in [
        "POST",
        "https://api.sambanova.ai/v1/chat/completions",
        "MiniMax-M2.7",
        "\"stream\"",
        "Bearer <redacted>",
        "Content-Type",
        "dry-run",
    ] {
        assert!(
            all.contains(needle),
            "the dry run prints the request shape with `{needle}`:\n{all}"
        );
    }
    assert!(
        !all.contains(secret),
        "the key value is never printed:\n{all}"
    );
    assert!(!all.contains("SECRET"), "not even a fragment of it:\n{all}");
    // The model is a dial.
    let (o2, _h) = run(
        "sambacloud-smoke.sh",
        &["--dry-run"],
        &[
            ("SAMBANOVA_API_KEY", secret),
            ("SAMBANOVA_MODEL", "DeepSeek-V3.1"),
        ],
    );
    assert!(
        text(&o2).contains("DeepSeek-V3.1"),
        "SAMBANOVA_MODEL selects the model:\n{}",
        text(&o2)
    );
    // A dry run needs no key at all.
    let (o3, _h) = run("sambacloud-smoke.sh", &["--dry-run"], &[]);
    assert_eq!(
        o3.status.code(),
        Some(0),
        "a dry run without a key still prints the shape: {}",
        text(&o3)
    );
}

#[test]
fn the_smoke_refuses_to_run_without_a_key_and_names_where_it_looks() {
    let (o, _h) = run("sambacloud-smoke.sh", &[], &[]);
    assert_eq!(
        o.status.code(),
        Some(3),
        "no key → exit 3 (never a network call): {}",
        text(&o)
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("SAMBANOVA_API_KEY") && err.contains(".sambanova_key"),
        "the refusal names both places a key may live: {err}"
    );
}

#[test]
fn neither_script_ever_echoes_a_key() {
    for name in SCRIPTS {
        let src = std::fs::read_to_string(script(name)).unwrap();
        for (i, line) in src.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with('#') {
                continue;
            }
            let prints = t.starts_with("echo")
                || t.starts_with("printf")
                || t.contains("print(")
                || t.contains(">&2");
            let names_key =
                t.contains("SAMBANOVA_API_KEY") || t.contains("$KEY") || t.contains("${KEY");
            assert!(
                !(prints && names_key)
                    || t.contains("<redacted>")
                    || t.contains("is not set")
                    || t.contains("SAMBANOVA_API_KEY or"),
                "{name}:{}: a print statement names the key: `{line}`",
                i + 1
            );
            // The key must never ride a command line (visible in `ps`): only through a config fd/file.
            assert!(
                !(t.contains("curl")
                    && (t.contains("$KEY")
                        || t.contains("${KEY")
                        || t.contains("SAMBANOVA_API_KEY"))),
                "{name}:{}: the key on curl's command line: `{line}`",
                i + 1
            );
            assert!(
                !t.contains("set -x"),
                "{name}:{}: `set -x` would trace the key: `{line}`",
                i + 1
            );
        }
    }
    let smoke = std::fs::read_to_string(script("sambacloud-smoke.sh")).unwrap();
    assert!(
        smoke.contains("--config") || smoke.contains("-K "),
        "the smoke passes the key to curl through a config file/fd, never argv"
    );
}

/// The document `--record` writes: an ISO date and a provenance, printed by `--dry-run` from the same
/// program that writes it, so the shape shown and the shape written cannot drift.
fn record_shape(out: &str) -> serde_json::Value {
    let marker = "--record would write this shape to";
    let at = out
        .find(marker)
        .unwrap_or_else(|| panic!("the dry run prints the record shape:\n{out}"));
    let rest = &out[at..];
    let start = rest.find('{').expect("a JSON document follows");
    serde_json::from_str(&rest[start..])
        .unwrap_or_else(|e| panic!("the record shape is JSON ({e}):\n{}", &rest[start..]))
}

fn is_iso_date(v: &serde_json::Value) -> bool {
    let s = v.as_str().unwrap_or_default();
    s.len() == 10
        && s.as_bytes()[4] == b'-'
        && s.as_bytes()[7] == b'-'
        && s.chars().filter(char::is_ascii_digit).count() == 8
}

#[test]
fn the_record_document_is_dated_and_its_provenance_is_measured() {
    let (o, _h) = run("sambacloud-smoke.sh", &["--dry-run"], &[]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    let doc = record_shape(&out);
    assert_eq!(
        doc["provenance"], "measured_sambacloud_api",
        "a live record says where the number came from: {doc}"
    );
    assert!(
        is_iso_date(&doc["date"]),
        "the record is dated YYYY-MM-DD: {doc}"
    );
    let obj = doc.as_object().expect("an object");
    for k in [
        "provider",
        "model",
        "ttft_s",
        "completion_tokens",
        "tokens_per_s",
        "stream_span_s",
        "measured_at",
    ] {
        assert!(obj.contains_key(k), "the record carries `{k}`: {doc}");
    }
    for k in ["ttft_s", "completion_tokens", "tokens_per_s"] {
        assert!(
            doc[k].is_null(),
            "a SHAPE measures nothing — `{k}` is null until a request is made: {doc}"
        );
    }
    assert_eq!(doc["provider"], "sambacloud");
    // No time-valued field is invented: the shape shows exactly the keys a measurement fills.
    assert!(!out.contains("\"tok/s\""), "no invented field names: {out}");
}

/// One captured SSE stream, no key, no network: TTFT, the token count and tok/s all the way to the record —
/// and the record says it is a replay. This is the only way to prove the measuring path reads the stream at
/// all on a box with no key.
fn sse(chunks: usize, usage: Option<usize>) -> String {
    let mut s = String::new();
    for i in 0..chunks {
        s.push_str(&format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"tok{i} \"}}}}]}}\n"
        ));
    }
    if let Some(n) = usage {
        s.push_str(&format!("data: {{\"choices\":[],\"usage\":{{\"completion_tokens\":{n},\"prompt_tokens\":11}}}}\n"));
    }
    s.push_str("data: [DONE]\n");
    s
}

#[test]
fn the_measuring_path_runs_offline_and_a_replay_is_never_stamped_as_a_measurement() {
    let dir = tempfile::tempdir().unwrap();
    let stream = dir.path().join("stream.sse");
    let rec = dir.path().join("rec.json");
    std::fs::write(&stream, sse(5, Some(7))).unwrap();
    let (o, _h) = run(
        "sambacloud-smoke.sh",
        &["--record"],
        &[
            ("SAMBANOVA_STREAM_FILE", stream.to_str().unwrap()),
            ("SAMBANOVA_RECORD_PATH", rec.to_str().unwrap()),
        ],
    );
    assert_eq!(
        o.status.code(),
        Some(0),
        "a replay needs no key: {}",
        text(&o)
    );
    let out = String::from_utf8_lossy(&o.stdout).to_string();
    let line = out
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or_else(|| panic!("one JSON line:\n{out}"));
    let v: serde_json::Value = serde_json::from_str(line).expect("the smoke prints one JSON line");
    assert!(
        v["ttft_s"].as_f64().is_some_and(|t| t > 0.0),
        "TTFT is measured from the first `data:` line: {line}"
    );
    assert_eq!(
        v["completion_tokens"], 7,
        "the usage chunk is the token count: {line}"
    );
    assert_eq!(v["token_source"], "usage.completion_tokens", "{line}");
    if !v["stream_span_s"].is_null() {
        assert!(
            v["tokens_per_s"].as_f64().is_some_and(|t| t > 0.0),
            "tok/s over the streamed span: {line}"
        );
    }
    assert!(
        v["published_by"]
            .as_str()
            .unwrap()
            .contains("NOT a live measurement"),
        "the line says what it is: {line}"
    );

    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&rec).expect("--record wrote the file"))
            .unwrap();
    assert_eq!(
        doc["provenance"], "replayed_sse_capture",
        "a replay is NEVER stamped as a measurement: {doc}"
    );
    assert_ne!(doc["provenance"], "measured_sambacloud_api");
    assert!(is_iso_date(&doc["date"]), "the record is dated: {doc}");
    assert_eq!(
        doc["replayed_from"],
        stream.to_str().unwrap(),
        "and names the capture it replayed: {doc}"
    );

    // Without a usage chunk the count falls back to the content chunks, and says so.
    let plain = dir.path().join("plain.sse");
    std::fs::write(&plain, sse(5, None)).unwrap();
    let (o2, _h) = run(
        "sambacloud-smoke.sh",
        &[],
        &[("SAMBANOVA_STREAM_FILE", plain.to_str().unwrap())],
    );
    let out2 = String::from_utf8_lossy(&o2.stdout).to_string();
    let v2: serde_json::Value = serde_json::from_str(
        out2.lines()
            .rev()
            .find(|l| l.trim_start().starts_with('{'))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(v2["completion_tokens"], 5, "five content chunks: {v2}");
    assert_eq!(v2["token_source"], "content chunks", "{v2}");

    // A capture that is not there is refused, never silently measured as zero.
    let (o3, _h) = run(
        "sambacloud-smoke.sh",
        &[],
        &[(
            "SAMBANOVA_STREAM_FILE",
            dir.path().join("nope.sse").to_str().unwrap(),
        )],
    );
    assert_eq!(
        o3.status.code(),
        Some(4),
        "an unreadable capture is refused: {}",
        text(&o3)
    );
    assert!(
        text(&o3).contains("nope.sse"),
        "and names it: {}",
        text(&o3)
    );
}

/// The live baseline: only when a key is present AND the caller opts in (`LOGOS_SAMBACLOUD_LIVE=1`), so the
/// suite never spends the key or the network by accident. Serialized in the `sambacloud-live` nextest group.
#[test]
fn the_live_smoke_prints_ttft_and_tokens_per_second() {
    let key_file = dirs::home_dir()
        .map(|h| h.join(".sambanova_key"))
        .filter(|p| p.is_file());
    let have_key = std::env::var("SAMBANOVA_API_KEY")
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false)
        || key_file.is_some();
    let opted_in = std::env::var("LOGOS_SAMBACLOUD_LIVE")
        .map(|v| v == "1")
        .unwrap_or(false);
    if !have_key || !opted_in {
        if !common::skip_missing_capability(
            "sambacloud-live",
            "a SambaNova Cloud key plus LOGOS_SAMBACLOUD_LIVE=1",
            "the_live_smoke_prints_ttft_and_tokens_per_second",
        ) {
            return;
        }
    }
    let o = Command::new("bash")
        .arg(script("sambacloud-smoke.sh"))
        .current_dir(repo_root())
        .output()
        .unwrap();
    let out = String::from_utf8_lossy(&o.stdout);
    assert_eq!(
        o.status.code(),
        Some(0),
        "the live smoke succeeds: {}",
        text(&o)
    );
    let line = out
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .expect("a JSON line");
    let v: serde_json::Value = serde_json::from_str(line).expect("the smoke prints one JSON line");
    assert_eq!(v["provider"], "sambacloud");
    assert!(
        v["ttft_s"].as_f64().map(|t| t > 0.0).unwrap_or(false),
        "TTFT measured: {line}"
    );
    assert!(
        v["tokens_per_s"].as_f64().map(|t| t > 0.0).unwrap_or(false),
        "tok/s measured: {line}"
    );
    assert!(
        v["completion_tokens"]
            .as_u64()
            .map(|n| n > 0)
            .unwrap_or(false),
        "tokens counted: {line}"
    );
    assert!(
        !out.contains("Bearer ") || out.contains("<redacted>"),
        "no key in the output"
    );
}
