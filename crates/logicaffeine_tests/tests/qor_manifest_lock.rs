//! qor_manifest_lock — every third-party candidate of the comparative-synthesis bench is accounted for.
//!
//! `benchmarks/silicon/qor/candidates.json` is written only by `scripts/qor/build-qor-candidate.py`: one
//! entry per candidate a tool built (its harness binding, the tracked source it read, the exact command,
//! the SHA3-256 of every file it generated) and one refusal per candidate a tool would not build (a
//! crash, an error, a deadline). The generated RTL itself carries the tools' own licences and stays out
//! of the repository (`benchmarks/silicon/qor/rtl/`, regenerated), so this lock holds what a clone can
//! check — the manifest is well formed, every binding fits the ports the tool's netlist declared, every
//! source and tool is real, no candidate is both built and refused — and, wherever the generated RTL is
//! present, that every file still hashes to its record.

mod common;

use logicaffeine_compile::codegen_sva::qor_harness::*;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn manifest() -> Manifest {
    read_manifest(&root()).expect("the manifest reads")
}

/// The tools `scripts/qor/qor-tools.toml` pins.
fn pinned_tools() -> BTreeSet<String> {
    let text = std::fs::read_to_string(root().join("scripts/qor/qor-tools.toml")).expect("the tool pins");
    text.lines()
        .filter_map(|l| l.trim().strip_prefix('[').and_then(|l| l.strip_suffix(']')))
        .map(|t| t.to_string())
        .collect()
}

#[test]
fn the_manifest_is_the_builders_schema() {
    let m = manifest();
    assert_eq!(m.schema, "logos.qor.candidates/1");
    assert!(!m.candidates.is_empty(), "the bench has third-party candidates");
}

#[test]
fn every_candidate_binds_the_ports_its_netlist_declared() {
    for e in manifest().candidates {
        let k = kernel(&e.candidate.kernel).unwrap_or_else(|| panic!("{}: no kernel {}", e.candidate.name, e.candidate.kernel));
        check_ports(&k, &e.candidate).unwrap_or_else(|err| panic!("{}: {err}", e.candidate.name));
    }
}

#[test]
fn every_candidate_and_refusal_names_a_pinned_tool_a_tracked_source_and_its_kernel() {
    let tools = pinned_tools();
    let m = manifest();
    for e in &m.candidates {
        let c = &e.candidate;
        assert!(tools.contains(&e.tool), "{}: tool {} is not pinned in qor-tools.toml", c.name, e.tool);
        assert!(c.name.starts_with(&format!("{}_{}_", c.kernel.to_lowercase(), e.tool)), "{}: <kernel>_<tool>_…", c.name);
        assert!(!e.generate.trim().is_empty() && !e.interface_note.trim().is_empty(), "{}: command and interface note", c.name);
        for s in e.source.split_whitespace() {
            assert!(
                s.starts_with(&format!("benchmarks/silicon/qor/kernels/{}/", c.kernel)) && root().join(s).is_file(),
                "{}: source {s} is a tracked file of its kernel",
                c.name
            );
        }
        assert!(!e.files.is_empty(), "{}: generated files", c.name);
        for f in &e.files {
            assert!(
                f.path.starts_with(&format!("benchmarks/silicon/qor/rtl/{}/{}/", c.kernel, c.name)),
                "{}: {} lives in the candidate's own generated directory",
                c.name,
                f.path
            );
            assert!(f.sha3_256.len() == 64 && f.sha3_256.bytes().all(|b| b.is_ascii_hexdigit()), "{}: {}", c.name, f.path);
        }
    }
    for r in &m.refusals {
        assert!(kernel(&r.kernel).is_some(), "{}: no kernel {}", r.name, r.kernel);
        assert!(tools.contains(&r.tool), "{}: tool {} is not pinned", r.name, r.tool);
        assert!(r.name.starts_with(&format!("{}_{}_", r.kernel.to_lowercase(), r.tool)), "{}: <kernel>_<tool>_…", r.name);
        assert!(!r.reason.trim().is_empty() && !r.generate.trim().is_empty(), "{}: a refusal says what ran and why it failed", r.name);
    }
}

#[test]
fn no_candidate_is_both_built_and_refused_and_every_name_is_unique() {
    let m = manifest();
    let mut seen = BTreeSet::new();
    for name in m.candidates.iter().map(|e| &e.candidate.name).chain(m.refusals.iter().map(|r| &r.name)) {
        assert!(seen.insert(name.clone()), "{name} appears twice");
    }
}

#[test]
fn a_refusal_round_trips_and_a_manifest_without_refusals_reads_as_none() {
    let text = r#"{"schema":"logos.qor.candidates/1","candidates":[]}"#;
    let m: Manifest = serde_json::from_str(text).unwrap();
    assert!(m.refusals.is_empty());
    let r = ManifestRefusal {
        kernel: "K6".into(),
        name: "k6_bambu_pipe_array_a7p10".into(),
        tool: "bambu".into(),
        generate: "bambu bambu_array.c -p=simt_arrays".into(),
        reason: "Bambu 2024.10 exited 139: Segmentation fault".into(),
    };
    let back: ManifestRefusal = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
    assert_eq!(back, r);
}

#[test]
fn every_generated_file_present_hashes_to_its_record() {
    if !common::require_tool("qor-rtl") {
        return;
    }
    for e in manifest().candidates {
        manifest_candidate(&root(), &e.candidate.name).unwrap_or_else(|err| panic!("{}: {err}", e.candidate.name));
    }
}

/// Every string anywhere in `v` that carries a control character other than a newline or a tab: a terminal's colour
/// sequence copied out of a tool's transcript prints as garbage in every report that quotes it.
pub fn unprintable(v: &serde_json::Value, at: &str, out: &mut Vec<String>) {
    match v {
        serde_json::Value::String(s) => {
            if s.chars().any(|c| (c.is_control() && c != '\n' && c != '\t') || c == '\u{7f}') {
                out.push(format!("{at}: {s:?}"));
            }
        }
        serde_json::Value::Array(a) => a.iter().enumerate().for_each(|(i, x)| unprintable(x, &format!("{at}[{i}]"), out)),
        serde_json::Value::Object(o) => o.iter().for_each(|(k, x)| unprintable(x, &format!("{at}.{k}"), out)),
        _ => {}
    }
}

#[test]
fn every_reason_and_note_in_the_manifest_is_printable_text() {
    let text = std::fs::read_to_string(root().join("benchmarks/silicon/qor/candidates.json")).expect("the manifest");
    let mut bad = Vec::new();
    unprintable(&serde_json::from_str(&text).expect("json"), "candidates.json", &mut bad);
    assert!(bad.is_empty(), "control characters in the manifest (rebuild the candidate): {bad:#?}");
}

#[test]
fn a_refusal_keeps_no_terminal_control_sequence() {
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(
            "import importlib.util, sys\n\
             spec = importlib.util.spec_from_file_location('b', sys.argv[1])\n\
             b = importlib.util.module_from_spec(spec); spec.loader.exec_module(b)\n\
             print(repr(b.Refusal('K3', 'x', 'spatial', 'g', 'exited 1: [\\x1b[31merror\\x1b[0m] No target\\x1b[K').reason))",
        )
        .arg(root().join("scripts/qor/build-qor-candidate.py"))
        .output()
        .expect("python3 runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "'exited 1: [error] No target'");
}

