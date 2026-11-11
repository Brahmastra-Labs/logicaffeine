//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! FUSION PARITY — every fusion the leading inference stacks apply, and whether our compiler applies it.
//!
//! `benchmarks/fusion_parity.json` names each fusion class once (a norm with its weight, an add with the norm of
//! the sum, a router's softmax and top-k, an attention writing its quantized output, …), where llama.cpp, vLLM,
//! SGLang, FlashInfer and TensorRT-LLM implement it at a pinned commit, and what we do:
//!
//!   `compiled`          the compiled CUDA forward or the compiled generation lane (`codegen_cuda::lane`) applies it on
//!                       its own, from a program's tensor statements;
//!   `partial`           applied in a weaker form;
//!   `hand_written_only` only a hand-written model lane has it — which does not count;
//!   `gap`               nowhere.
//!
//! This lock holds the ledger to the code. A `compiled` row names a census test (the fused kernel is emitted and
//! the launches it replaces are not) and a values test (the fused program computes what the VM or a reference
//! computes), and both must be real tests through the compiled entry points. The count of `compiled` rows never
//! falls, the inventory is the pinned one, and — on a box that holds the pinned sources — every citation resolves.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// The number of classes the ledger marks `compiled`. It only rises.
const COMPILED_FLOOR: usize = 36;
/// The inventory: fusion classes and serving features, as pinned. Re-inventorying a new stack release changes
/// these on purpose, together with the pins.
const CLASSES: usize = 36;
const SERVING: usize = 7;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ledger() -> Value {
    let path = root().join("benchmarks/fusion_parity.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn rows<'a>(l: &'a Value, section: &str) -> &'a [Value] {
    l[section]
        .as_array()
        .unwrap_or_else(|| panic!("`{section}` is not a list"))
}

fn text<'a>(v: &'a Value, key: &str, what: &str) -> &'a str {
    v[key]
        .as_str()
        .unwrap_or_else(|| panic!("{what}: `{key}` is missing or not text"))
}

/// The source of the test a `binary::function` citation names, and the function's body, if both exist and the
/// function is a `#[test]`.
fn cited_test(citation: &str) -> Result<(String, String), String> {
    let (binary, function) = citation
        .split_once("::")
        .ok_or_else(|| format!("`{citation}` is not `binary::function`"))?;
    let path = root().join(format!("crates/logicaffeine_tests/tests/{binary}.rs"));
    let source = std::fs::read_to_string(&path)
        .map_err(|e| format!("`{citation}`: {}: {e}", path.display()))?;
    let head = format!("fn {function}(");
    let at = source
        .find(&head)
        .ok_or_else(|| format!("`{citation}`: no `{head}` in {}", path.display()))?;
    let before = &source[..at];
    let attrs = before.rsplit_once("\n}").map_or(before, |(_, a)| a);
    if !attrs.contains("#[test]") {
        return Err(format!("`{citation}` is not a #[test]"));
    }
    let body_end = source[at..].find("\n}\n").map_or(source.len(), |e| at + e);
    Ok((source.clone(), source[at..body_end].to_string()))
}

/// Whether the census `citation` compiles through a compiler: its body calls the single-token forward
/// (`compile_to_cuda_forward*`), or it is a test of `lane_compile_lock`, whose every test compiles through
/// `codegen_cuda::lane`.
fn census_compiles(citation: &str) -> Result<(), String> {
    let (source, body) = cited_test(citation)?;
    let through_the_lane =
        citation.starts_with("lane_compile_lock::") && source.contains("use logicaffeine_compile::codegen_cuda::lane::{compile, compile_holding");
    if body.contains("compile_to_cuda_forward") || through_the_lane {
        Ok(())
    } else {
        Err(format!("the census `{citation}` compiles through neither `compile_to_cuda_forward*` nor the lane"))
    }
}

#[test]
fn every_class_is_well_formed() {
    let l = ledger();
    let statuses: BTreeSet<&str> = l["statuses"]
        .as_array()
        .expect("statuses")
        .iter()
        .map(|s| s.as_str().expect("status"))
        .collect();
    assert_eq!(
        statuses,
        BTreeSet::from(["compiled", "partial", "hand_written_only", "gap"])
    );
    let pinned: BTreeSet<&str> = rows(&l, "pins").iter().map(|p| text(p, "stack", "pin")).collect();
    let mut ids = BTreeSet::new();
    for section in ["classes", "serving"] {
        for c in rows(&l, section) {
            let id = text(c, "id", section);
            assert!(ids.insert(id.to_string()), "`{id}` appears twice");
            let status = text(&c["ours"], "status", id);
            assert!(statuses.contains(status), "{id}: unknown status `{status}`");
            let stacks = c["stacks"].as_array().unwrap_or_else(|| panic!("{id}: no stacks"));
            assert!(!stacks.is_empty(), "{id} cites no stack");
            for s in stacks {
                let stack = text(s, "stack", id);
                assert!(pinned.contains(stack), "{id}: `{stack}` is not a pinned stack");
                let cited = s["path"].is_string() || (s["file"].is_string() && s["anchor"].is_string());
                assert!(cited, "{id}: a {stack} citation needs `path`, or `file` and `anchor`");
            }
        }
    }
}

#[test]
fn the_inventory_is_the_pinned_one() {
    let l = ledger();
    assert_eq!(rows(&l, "classes").len(), CLASSES, "fusion classes");
    assert_eq!(rows(&l, "serving").len(), SERVING, "serving features");
    for p in rows(&l, "pins") {
        let commit = text(p, "commit", "pin");
        assert!(
            commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit()),
            "a pin is a full commit hash, got `{commit}`"
        );
    }
}

#[test]
fn the_compiled_count_never_falls() {
    let l = ledger();
    let compiled = rows(&l, "classes")
        .iter()
        .filter(|c| c["ours"]["status"] == "compiled")
        .count();
    assert!(
        compiled >= COMPILED_FLOOR,
        "{compiled} classes are compiled, the floor is {COMPILED_FLOOR}"
    );
}

/// A `compiled` row carries a census and a values test; a `partial` row a census test. Each is a real `#[test]`,
/// and each census goes through a compiler — the single-token forward (`compile_to_cuda_forward*`), or the generation
/// lane, every test of `lane_compile_lock` compiling through `codegen_cuda::lane` — as a hand-written lane proves
/// nothing about the compiler.
#[test]
fn every_claim_names_real_evidence_through_the_compiled_path() {
    let l = ledger();
    let mut problems = Vec::new();
    for c in rows(&l, "classes") {
        let id = text(c, "id", "class");
        let ours = &c["ours"];
        let needs: &[&str] = match ours["status"].as_str() {
            Some("compiled") => &["census", "values"],
            Some("partial") => &["census"],
            _ => &[],
        };
        for key in needs {
            let Some(citation) = ours[key].as_str() else {
                problems.push(format!("{id}: no `{key}` test"));
                continue;
            };
            let checked = if *key == "census" { census_compiles(citation) } else { cited_test(citation).map(|_| ()) };
            if let Err(e) = checked {
                problems.push(format!("{id}: {e}"));
            }
        }
        for key in ["census", "values"] {
            if ours[key].is_string() && needs.is_empty() {
                problems.push(format!(
                    "{id}: a `{}` row cites `{key}` evidence — promote it, or drop the citation",
                    ours["status"]
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// A census counts only through a compiler: a GPU values test, a hand-written lane's test and a test of the lane's
/// runtime are refused as censuses; the forward's and the lane compiler's are accepted.
#[test]
fn a_census_must_compile_through_a_compiler() {
    for refused in [
        "cuda_lane_lock::every_fusion_computes_what_its_launches_compute_apart",
        "qwen38_golden_lock::the_recorded_result_names_exactly_the_golden_stream_this_lock_defines",
        "lane_compile_lock::no_such_test",
    ] {
        assert!(census_compiles(refused).is_err(), "`{refused}` is refused as a census");
    }
    for accepted in [
        "lane_compile_lock::the_dense_steps_fuse_where_their_dataflow_allows",
        "cuda_fusion_census_lock::a_weighted_norm_is_one_row_kernel",
    ] {
        assert_eq!(census_compiles(accepted), Ok(()), "`{accepted}` compiles through a compiler");
    }
}

/// Every citation resolves in the pinned sources: the file holds its anchor, or the path is in the stack's pinned
/// file listing — and the sources on this box are the pinned commits.
#[test]
fn every_citation_resolves_in_the_pinned_sources() {
    if !common::require_tool("competitor-sources") {
        return;
    }
    let l = ledger();
    let dir = root().join("external/competitors");
    let local = std::fs::read_to_string(dir.join("PINS.tsv")).expect("PINS.tsv");
    for p in rows(&l, "pins") {
        let line = format!(
            "{}\t{}\t{}\t{}",
            text(p, "stack", "pin"),
            text(p, "repo", "pin"),
            text(p, "tag", "pin"),
            text(p, "commit", "pin")
        );
        assert!(
            local.lines().any(|l| l == line),
            "the sources on this box are not the pinned `{line}` — refetch them, or re-inventory"
        );
    }
    let mut unresolved = Vec::new();
    for section in ["classes", "serving"] {
        for c in rows(&l, section) {
            let id = text(c, "id", section);
            for s in c["stacks"].as_array().expect("stacks") {
                let stack = text(s, "stack", id);
                let ok = if let Some(path) = s["path"].as_str() {
                    std::fs::read_to_string(dir.join(format!("{stack}.paths.txt")))
                        .map(|listing| listing.lines().any(|l| l == path))
                        .unwrap_or_else(|_| dir.join(stack).join(path).is_file())
                } else {
                    let (file, anchor) = (text(s, "file", id), text(s, "anchor", id));
                    std::fs::read_to_string(dir.join(stack).join(file))
                        .map(|t| t.contains(anchor))
                        .unwrap_or(false)
                };
                if !ok {
                    unresolved.push(format!("{id}: {stack} {s}"));
                }
            }
        }
    }
    assert!(
        unresolved.is_empty(),
        "citations that do not resolve at the pins:\n{}",
        unresolved.join("\n")
    );
}
