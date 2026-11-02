//! eda_tools_seam_lock — every test that reads the machine-local EDA tree asks the input seam first.
//!
//! The silicon evidence lives under `tools/`: OpenLane with this repository's hardened designs and their run
//! directories, the Caravel checkouts, the assembled submissions. A clone does not carry it (CI has none, the
//! fresh-clone gate links it). A test that reads it without asking either fails in a clone
//! (`rtl_provenance_lock`'s anti-vacuity assert did, in CI) or passes having checked nothing (a `return` or
//! `continue` when a path is missing). Through `common::require_tool("eda-tools")` a clone records a counted
//! `LOGOS_TOOL_SKIP: eda-tools`, and a gate that names the tree in `LOGOS_REQUIRE_TOOLS` fails loud without it.
//!
//! The census reads every test source for the ways a test reaches the tree — a `"tools/…"` or `"tools"`
//! literal, a join of a Caravel project's `submission_dir`, a run of a script that reads the tree — and holds
//! each one to the seam: asked earlier in the same function, or, in a helper, asked by every caller before it
//! calls. The readers it finds are the `READERS` row, exactly. A path under `tools/` that a narrower probe answers
//! for (`NARROW_SEAMS`: an environment one fetcher installs there, not the EDA tree) is held to that probe alone.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};

const SEAM: &str = "common::require_tool(\"eda-tools\")";

/// Probes narrower than the tree's, each with the path under `tools/` it answers for: an environment one fetcher
/// installs there, which the tree's probe does not check. A read under one is held to its probe, and does not make
/// its binary a reader of the tree.
const NARROW_SEAMS: &[(&str, &str)] = &[
    ("litex", "tools/qor/litex-venv"),
    ("openxc7-source", "tools/openxc7/nextpnr-xilinx"),
    ("opae-sdk", "tools/d5005/opae-sdk"),
    ("verilator", "tools/verilator"),
    ("d5005-pim", "tools/d5005"),
];

/// The seam a read reaching the tree at column `col` of `line` must ask: the narrow probe that answers for the path
/// its literal names, the tree's otherwise.
fn seam_for(line: &str, col: usize) -> String {
    let literal = line[col..].strip_prefix('"').and_then(|rest| rest.split('"').next());
    literal
        .and_then(|l| {
            NARROW_SEAMS
                .iter()
                .find(|(_, under)| l == *under || l.starts_with(&format!("{under}/")))
        })
        .map_or(SEAM.to_string(), |(probe, _)| format!("common::require_tool(\"{probe}\")"))
}

/// The test binaries that read the EDA tree.
const READERS: &[&str] = &[
    "caravel_bringup_lock",
    "caravel_project_lock",
    "flow_config_lock",
    "flow_script_guard_lock",
    "openlane_overlay_lock",
    "openlane_tcl_lever_lock",
    "openroad_build_lock",
    "rtl_provenance_lock",
    "signoff_period_lock",
    "sky130_provenance_lock",
    "synth_strategy_registry_lock",
];

/// Files that name the tree without reading it, each with its reason.
const MENTION_FILES: &[(&str, &str)] = &[
    (
        "common/mod.rs",
        "the seam itself: the named probe asks whether the tree is there",
    ),
    (
        "common/public_manifest.rs",
        "names the gitignored tree in the ignore oracle; reads nothing under it",
    ),
    (
        "mcp_e2e.rs",
        "MCP method names (`tools/list`, `tools/call`) and result keys, not paths",
    ),
    (
        "mcp_protocol.rs",
        "MCP method names (`tools/list`, `tools/call`) and result keys, not paths",
    ),
    (
        "mcp_tools.rs",
        "MCP method names (`tools/list`, `tools/call`) and result keys, not paths",
    ),
    (
        "signal_blast_radius_lock.rs",
        "names the machine-local root directories its scan skips; reads nothing under them",
    ),
    (
        "common/sky130_ledgers.rs",
        "names the run root a registry entry without one implies; reads nothing under it",
    ),
];

/// Functions that name the tree without reading it, each with its reason.
const MENTION_FNS: &[(&str, &str, &str)] = &[
    (
        "caravel_project_lock.rs",
        "every_submission_directory_is_ignored",
        "the .gitignore matcher's own cases: paths as data, nothing read",
    ),
    (
        "caravel_project_lock.rs",
        "a_staged_submission_is_checked_where_it_stands_and_recorded_where_it_lands",
        "names the landed submission directory only as the path the recorder records; the recorder reads the staged copy in a temporary directory",
    ),
    (
        "flow_script_guard_lock.rs",
        "each_knob_is_forwarded_or_refused_when_the_runner_runs",
        "builds its own `tools/OpenLane/designs` inside a temporary sandbox and reads only that",
    ),
    (
        "flow_script_guard_lock.rs",
        "the_prebuilt_emitter_is_where_cargo_builds_it",
        "runs the runner in a temporary sandbox with its own `tools/OpenLane/designs`, and reads only that",
    ),
    (
        "flow_script_guard_lock.rs",
        "a_bench_run_or_a_control_names_its_image_and_records_only_an_anchored_ir",
        "runs the runner in a temporary sandbox with its own `tools/OpenLane`, `tools/qor/openlane-bench` and `tools/qor/openlane-stock`, and reads only those",
    ),
    (
        "openroad_build_lock.rs",
        "an_ir_replay_runs_in_the_tree_and_the_image_its_run_flowed_in",
        "replays runs it builds in a temporary sandbox's own `tools/` trees, and reads only those",
    ),
    (
        "openlane_overlay_lock.rs",
        "only_a_control_on_the_stock_tree_at_the_overlays_base_runs_without_the_overlay",
        "runs the runner in a temporary sandbox with its own `tools/qor/openlane-stock`, and reads only that",
    ),
    (
        "openlane_overlay_lock.rs",
        "the_overlay_is_laid_over_the_tree_it_is_given_at_the_overlays_base",
        "lays the overlay over trees built in a temporary sandbox, and reads only those",
    ),
    (
        "openlane_overlay_lock.rs",
        "a_bench_run_flows_in_the_bench_tree_at_the_overlays_base",
        "runs the runner in a temporary sandbox with its own `tools/qor/openlane-bench`, and reads only that",
    ),
    (
        "sky130_ledger_registry_lock.rs",
        "the_recorder_refuses_a_ledger_the_registry_does_not_name",
        "runs the recorder at an unregistered ledger, which it refuses before it reads any run",
    ),
    (
        "sky130_ledger_registry_lock.rs",
        "the_recorder_writes_a_ledger_whole_under_its_lock",
        "imports the recorder to call its ledger write on a scratch ledger; it reads no run",
    ),
    (
        "sky130_ledger_registry_lock.rs",
        "the_registry_names_every_block_ledger_on_disk_and_only_those",
        "holds each entry's run root to a tree under `tools/` by its text; reads nothing there",
    ),
    (
        "sky130_ledger_registry_lock.rs",
        "no_comparative_ledger_runs_in_the_shared_openlane_tree",
        "holds each comparative entry's run root to the bench's trees by its text; reads nothing there",
    ),
    (
        "sky130_area_lock.rs",
        "areas",
        "runs the recorder's area readers on a run directory built in a temporary sandbox, and reads only that",
    ),
    (
        "sky130_area_lock.rs",
        "every_synthesis_finding_is_counted_by_kind",
        "loads the recorder as a module and reads a run directory built in a temporary sandbox, and only that",
    ),
];

/// Scripts whose run reads the tree.
const TREE_READING_SCRIPTS: &[&str] = &["record-sky130-signoff.py", "resign-block.sh"];

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// Every `.rs` under `tests/`, as (path relative to `tests/`, source).
fn test_sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![tests_dir()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let rel = p
                    .strip_prefix(tests_dir())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// Does this line reach the tree? Byte offset of the reaching token in the line, if so.
fn reaches_tree(line: &str) -> Option<usize> {
    let code = line.trim_start();
    if code.starts_with("//") {
        return None;
    }
    let mut hits = Vec::new();
    for pat in ["\"tools/", "\"tools\""] {
        hits.extend(line.find(pat));
    }
    if let Some(j) = line.find("join(") {
        if line[j..].contains("submission_dir") {
            hits.push(j);
        }
    }
    if line.contains("Command::new") || line.contains(".arg(") {
        hits.extend(TREE_READING_SCRIPTS.iter().filter_map(|s| line.find(s)));
    }
    hits.into_iter().min()
}

/// A function of a source file: its name, whether it is a `#[test]`, and its byte span.
struct Func {
    name: String,
    test: bool,
    span: (usize, usize),
}

/// The functions of `src`, each spanning from its `fn` keyword to the brace that closes its body.
fn functions(src: &str) -> Vec<Func> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = src[at..].find("fn ").map(|i| i + at) {
        at = i + 3;
        let boundary = i == 0 || matches!(bytes[i - 1], b' ' | b'\n' | b'\t');
        let name: String = src[i + 3..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !boundary
            || name.is_empty()
            || !name.starts_with(|c: char| c.is_alphabetic() || c == '_')
        {
            continue;
        }
        let line_start = src[..i].rfind('\n').map_or(0, |p| p + 1);
        let before = &src[line_start..i];
        if before.contains("//") || before.matches('"').count() % 2 == 1 {
            continue;
        }
        let Some(open) = src[i..].find(['{', ';']).map(|o| o + i) else {
            continue;
        };
        if bytes[open] == b';' {
            continue;
        }
        let end = body_end(src, open);
        let test = src[..line_start]
            .lines()
            .rev()
            .take_while(|l| {
                let t = l.trim_start();
                t.starts_with("#[") || t.starts_with("///")
            })
            .any(|l| l.trim() == "#[test]");
        out.push(Func {
            test,
            name,
            span: (i, end),
        });
    }
    out
}

/// The byte just past the brace that closes the block opening at `open`. Braces inside string, raw-string and
/// char literals and inside comments do not count: a test that builds `"}"` must not end where its string does.
fn body_end(src: &str, open: usize) -> usize {
    let b = src.as_bytes();
    let mut depth = 0i64;
    let mut k = open;
    while k < b.len() {
        match b[k] {
            b'/' if b.get(k + 1) == Some(&b'/') => {
                k = src[k..].find('\n').map_or(b.len(), |n| k + n);
                continue;
            }
            b'/' if b.get(k + 1) == Some(&b'*') => {
                k = src[k + 2..].find("*/").map_or(b.len(), |n| k + 2 + n + 2);
                continue;
            }
            b'r' if matches!(b.get(k + 1), Some(b'"') | Some(b'#'))
                && (k == 0 || !(b[k - 1].is_ascii_alphanumeric() || b[k - 1] == b'_')) =>
            {
                let hashes = b[k + 1..].iter().take_while(|&&c| c == b'#').count();
                if b.get(k + 1 + hashes) == Some(&b'"') {
                    let close = format!("\"{}", "#".repeat(hashes));
                    k = src[k + 2 + hashes..]
                        .find(&close)
                        .map_or(b.len(), |n| k + 2 + hashes + n + close.len());
                    continue;
                }
            }
            b'"' => {
                k += 1;
                while k < b.len() && b[k] != b'"' {
                    k += if b[k] == b'\\' { 2 } else { 1 };
                }
                k += 1;
                continue;
            }
            b'\'' => {
                let lit = if b.get(k + 1) == Some(&b'\\') {
                    src[k + 3..]
                        .find('\'')
                        .filter(|&n| n <= 8)
                        .map(|n| k + 3 + n + 1)
                } else if b.get(k + 2) == Some(&b'\'') {
                    Some(k + 3)
                } else {
                    None
                };
                if let Some(next) = lit {
                    k = next;
                    continue;
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return k + 1;
                }
            }
            _ => {}
        }
        k += 1;
    }
    b.len()
}

/// The innermost function whose body contains byte offset `at`.
fn enclosing(funcs: &[Func], at: usize) -> Option<&Func> {
    funcs
        .iter()
        .filter(|f| f.span.0 <= at && at < f.span.1)
        .min_by_key(|f| f.span.1 - f.span.0)
}

/// Is the read at byte offset `at` reached only after `seam`: asked earlier in its function, or, in a helper, asked
/// by every caller before its call? `why` collects the paths that are not.
fn seamed(src: &str, funcs: &[Func], at: usize, seam: &str, depth: usize, why: &mut Vec<String>) -> bool {
    let Some(f) = enclosing(funcs, at) else {
        why.push("at item level, outside any function".into());
        return false;
    };
    if src[f.span.0..at].contains(seam) {
        return true;
    }
    if f.test || depth > 8 {
        why.push(format!(
            "in `{}`, which does not ask `{seam}` before it",
            f.name
        ));
        return false;
    }
    let calls: Vec<usize> = src
        .match_indices(&format!("{}(", f.name))
        .map(|(i, _)| i)
        .filter(|&i| !(f.span.0..f.span.0 + 3 + f.name.len()).contains(&i))
        .filter(|&i| {
            i == 0
                || !src.as_bytes()[i - 1].is_ascii_alphanumeric() && src.as_bytes()[i - 1] != b'_'
        })
        .collect();
    if calls.is_empty() {
        why.push(format!("in helper `{}`, which nothing calls", f.name));
        return false;
    }
    calls
        .into_iter()
        .all(|c| seamed(src, funcs, c, seam, depth + 1, why))
}

/// The reading binaries the census finds in the test sources, and the reads that skip the seam.
fn census() -> (Vec<String>, Vec<String>) {
    census_of(test_sources())
}

/// The reading binaries the census finds in `sources` (path relative to `tests/`, text), and the reads that skip the
/// seam.
fn census_of(sources: Vec<(String, String)>) -> (Vec<String>, Vec<String>) {
    let this_file = file!().rsplit(['/', '\\']).next().unwrap();
    let mut readers = Vec::new();
    let mut violations = Vec::new();
    for (file, src) in sources {
        if file == this_file || MENTION_FILES.iter().any(|(f, _)| *f == file) {
            continue;
        }
        let funcs = functions(&src);
        let mut offset = 0;
        for (n, line) in src.split_inclusive('\n').enumerate() {
            let start = offset;
            offset += line.len();
            let Some(col) = reaches_tree(line) else {
                continue;
            };
            let at = start + col;
            if let Some(f) = enclosing(&funcs, at) {
                if MENTION_FNS
                    .iter()
                    .any(|(mf, name, _)| *mf == file && *name == f.name)
                {
                    continue;
                }
            }
            let seam = seam_for(line, col);
            let mut why = Vec::new();
            if seamed(&src, &funcs, at, &seam, 0, &mut why) {
                if seam == SEAM && !file.contains('/') {
                    readers.push(file.trim_end_matches(".rs").to_string());
                }
            } else {
                violations.push(format!(
                    "{file}:{}: reaches tools/ {}",
                    n + 1,
                    why.join("; ")
                ));
            }
        }
    }
    readers.sort();
    readers.dedup();
    (readers, violations)
}

#[test]
fn every_read_of_the_eda_tree_asks_the_seam_first() {
    let (readers, violations) = census();
    assert!(
        violations.is_empty(),
        "a clone without tools/ would fail these, or pass them having checked nothing:\n  {}",
        violations.join("\n  ")
    );
    assert_eq!(
        readers, READERS,
        "the census found these readers of the EDA tree; READERS must name exactly them"
    );
    for (file, reason) in MENTION_FILES {
        assert!(!reason.trim().is_empty(), "`{file}` needs its reason");
        let src = std::fs::read_to_string(tests_dir().join(file)).unwrap_or_default();
        assert!(
            src.lines().any(|l| reaches_tree(l).is_some()),
            "`{file}` no longer names tools/: drop its entry"
        );
    }
    for (file, name, reason) in MENTION_FNS {
        assert!(
            !reason.trim().is_empty(),
            "`{file}::{name}` needs its reason"
        );
        let src = std::fs::read_to_string(tests_dir().join(file)).unwrap();
        let funcs = functions(&src);
        let f = funcs
            .iter()
            .find(|f| f.name == *name)
            .unwrap_or_else(|| panic!("`{file}` has no fn `{name}`: drop its entry"));
        assert!(
            src[f.span.0..f.span.1]
                .lines()
                .any(|l| reaches_tree(l).is_some()),
            "`{file}::{name}` no longer names tools/: drop its entry"
        );
    }
}

/// The census rules on the shapes they guard.
#[test]
fn the_census_rules_fire_on_the_shapes_they_guard() {
    let check = |src: &str| -> Vec<String> {
        let funcs = functions(src);
        let mut bad = Vec::new();
        let mut offset = 0;
        for line in src.split_inclusive('\n') {
            let start = offset;
            offset += line.len();
            if let Some(col) = reaches_tree(line) {
                let mut why = Vec::new();
                if !seamed(src, &funcs, start + col, &seam_for(line, col), 0, &mut why) {
                    bad.push(why.join("; "));
                }
            }
        }
        bad
    };
    let seamed_test = "#[test]\nfn t() {\n    if !common::require_tool(\"eda-tools\") {\n        return;\n    }\n    let d = root().join(\"tools/OpenLane\");\n}\n";
    assert!(
        check(seamed_test).is_empty(),
        "a read after the seam is held"
    );
    let unseamed = "#[test]\nfn t() {\n    let d = root().join(\"tools/OpenLane\");\n}\n";
    assert_eq!(check(unseamed).len(), 1, "a read with no seam is caught");
    let late = "#[test]\nfn t() {\n    let d = root().join(\"tools/OpenLane\");\n    if !common::require_tool(\"eda-tools\") {\n        return;\n    }\n}\n";
    assert_eq!(check(late).len(), 1, "a read BEFORE the seam is caught");
    let helper_ok = "fn h() -> PathBuf {\n    root().join(\"tools/caravel\")\n}\n\n#[test]\nfn t() {\n    if !common::require_tool(\"eda-tools\") {\n        return;\n    }\n    h();\n}\n";
    assert!(
        check(helper_ok).is_empty(),
        "a helper every caller seams is held"
    );
    let helper_bad = format!("{helper_ok}\n#[test]\nfn u() {{\n    h();\n}}\n");
    assert_eq!(
        check(&helper_bad).len(),
        1,
        "a helper one caller does not seam is caught"
    );
    let item = "const P: &str = \"tools/OpenLane/designs\";\n";
    assert_eq!(check(item).len(), 1, "an item-level path is caught");
    let sub = "#[test]\nfn t() {\n    let s = root().join(DRAM_PROJECT.submission_dir);\n}\n";
    assert_eq!(check(sub).len(), 1, "a submission directory is the tree");
    let script = "#[test]\nfn t() {\n    Command::new(\"python3\").arg(r.join(\"scripts/record-sky130-signoff.py\"));\n}\n";
    assert_eq!(
        check(script).len(),
        1,
        "a run of a tree-reading script is a read"
    );
    let text = "#[test]\nfn t() {\n    let s = std::fs::read_to_string(root().join(\"scripts/resign-block.sh\")).unwrap();\n}\n";
    assert!(
        check(text).is_empty(),
        "reading a script's text is not running it"
    );
    assert!(
        check("// root().join(\"tools/OpenLane\")\n").is_empty(),
        "a comment reads nothing"
    );
    let braces_in_literals = "#[test]\nfn t() {\n    let a = s.replace(\"}\", \"x\");\n    let b = '}';\n    let c = '\\'';\n    let d = r#\"}\"#;\n    // }\n    if !common::require_tool(\"eda-tools\") {\n        return;\n    }\n    let p = root().join(\"tools/OpenLane\");\n}\n";
    assert!(
        check(braces_in_literals).is_empty(),
        "a brace in a string, char, raw string or comment does not end the function"
    );
    let narrow = |probe: &str, path: &str| {
        format!("#[test]\nfn t() {{\n    if !common::require_tool(\"{probe}\") {{\n        return;\n    }}\n    let p = root().join(\"{path}\");\n}}\n")
    };
    assert!(
        check(&narrow("litex", "tools/qor/litex-venv/bin/python")).is_empty(),
        "a read under a narrow probe's path, after that probe, is held"
    );
    assert_eq!(
        check(&narrow("eda-tools", "tools/qor/litex-venv/bin/python")).len(),
        1,
        "the tree's probe does not answer for an environment it does not check"
    );
    assert_eq!(
        check(&narrow("litex", "tools/OpenLane/designs")).len(),
        1,
        "a narrow probe answers for its own path only"
    );
    assert_eq!(
        check(&narrow("litex", "tools/qor/litex-venv-old")).len(),
        1,
        "a narrow probe's path is a directory, not a name prefix"
    );
    let (readers, violations) = census_of(vec![
        ("tree_reader.rs".into(), narrow("eda-tools", "tools/OpenLane/designs")),
        ("venv_reader.rs".into(), narrow("litex", "tools/qor/litex-venv/bin/python")),
        ("common/helper.rs".into(), narrow("eda-tools", "tools/caravel")),
    ]);
    assert!(violations.is_empty(), "{violations:?}");
    assert_eq!(
        readers,
        ["tree_reader"],
        "a binary that reads the tree is a reader; one that reads a narrow probe's path, or a shared helper, is not"
    );
    let narrow_helper = "fn py() -> PathBuf {\n    root().join(\"tools/qor/litex-venv/bin/python\")\n}\n\n#[test]\nfn t() {\n    if !common::require_tool(\"litex\") {\n        return;\n    }\n    py();\n}\n";
    assert!(check(narrow_helper).is_empty(), "a helper every caller narrows is held");
    assert_eq!(
        check(&format!("{narrow_helper}\n#[test]\nfn u() {{\n    if !common::require_tool(\"eda-tools\") {{\n        return;\n    }}\n    py();\n}}\n")).len(),
        1,
        "a helper under a narrow path, called after the tree's probe alone, is caught"
    );
}

/// Every narrow probe is a named probe of `common::tool_available` (not a search of PATH, which would answer for no
/// path under `tools/`), its path under `tools/`, and the probe reads that path.
#[test]
fn every_narrow_probe_is_a_named_probe_of_the_path_it_answers_for() {
    let probes = std::fs::read_to_string(tests_dir().join("common/mod.rs")).unwrap();
    for (probe, under) in NARROW_SEAMS {
        assert!(under.starts_with("tools/") && !under.ends_with('/'), "{probe}: {under} is a path under tools/");
        let arm = probes
            .find(&format!("\"{probe}\" =>"))
            .unwrap_or_else(|| panic!("{probe} is no named probe of common::tool_available"));
        let body = &probes[arm..];
        let body = &body[..body.find("\n        }").expect("the probe's arm ends")];
        assert!(body.contains(under), "the {probe} probe does not read {under}:\n{body}");
    }
}

#[test]
fn the_eda_tree_probe_answers_whether_the_tree_is_there() {
    let full = |root: &Path| {
        std::fs::create_dir_all(root.join("tools/OpenLane/designs/logos_x")).unwrap();
        std::fs::create_dir_all(root.join("tools/caravel_user_project")).unwrap();
        std::fs::create_dir_all(root.join("tools/caravel")).unwrap();
    };
    let checkout = tempfile::tempdir().unwrap();
    assert!(
        !common::eda_tools_at(checkout.path()),
        "a checkout without tools/ does not have the tree"
    );
    full(checkout.path());
    assert!(
        common::eda_tools_at(checkout.path()),
        "OpenLane with a logos_ design and both Caravel checkouts is the tree"
    );
    for missing in ["tools/caravel", "tools/caravel_user_project"] {
        let partial = tempfile::tempdir().unwrap();
        full(partial.path());
        std::fs::remove_dir_all(partial.path().join(missing)).unwrap();
        assert!(
            !common::eda_tools_at(partial.path()),
            "without {missing} the tree is incomplete"
        );
    }
    let no_designs = tempfile::tempdir().unwrap();
    full(no_designs.path());
    std::fs::remove_dir_all(no_designs.path().join("tools/OpenLane/designs/logos_x")).unwrap();
    std::fs::create_dir_all(no_designs.path().join("tools/OpenLane/designs/spm")).unwrap();
    assert!(
        !common::eda_tools_at(no_designs.path()),
        "OpenLane without this repository's designs is not the tree"
    );
    let file_design = tempfile::tempdir().unwrap();
    full(file_design.path());
    std::fs::remove_dir_all(file_design.path().join("tools/OpenLane/designs/logos_x")).unwrap();
    std::fs::write(
        file_design.path().join("tools/OpenLane/designs/logos_x"),
        "",
    )
    .unwrap();
    assert!(
        !common::eda_tools_at(file_design.path()),
        "a file named like a design is not a design"
    );
    #[cfg(unix)]
    {
        let elsewhere = tempfile::tempdir().unwrap();
        full(elsewhere.path());
        let linked = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path().join("tools"), linked.path().join("tools"))
            .unwrap();
        assert!(
            common::eda_tools_at(linked.path()),
            "a linked tree (a private copy, the fresh-clone gate) is the tree"
        );
    }
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(
        common::tool_available("eda-tools"),
        common::eda_tools_at(&repo),
        "the named probe asks about this checkout"
    );
}

#[test]
fn the_full_runner_and_every_runner_of_a_reader_require_the_tree() {
    let scripts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts");
    let full = std::fs::read_to_string(scripts.join("run-all-tests-fast.sh")).unwrap();
    let default = common::required_by_runner(&full, "run-all-tests-fast.sh")
        .expect("the full runner sets LOGOS_REQUIRE_TOOLS");
    assert!(
        common::tool_required_by(&default, "eda-tools"),
        "the full runner must fail when the EDA tree is missing — its LOGOS_REQUIRE_TOOLS default is {default:?}"
    );
    for e in std::fs::read_dir(&scripts).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !(name.starts_with("run-") && name.ends_with(".sh")) || name == "run-all-tests-fast.sh" {
            continue;
        }
        let script = std::fs::read_to_string(e.path()).unwrap();
        let names_a_reader = script
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .any(|l| {
                l.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .any(|w| READERS.contains(&w))
            });
        if names_a_reader {
            let required = common::required_by_runner(&script, &name).unwrap_or_default();
            assert!(
                common::tool_required_by(&required, "eda-tools"),
                "{name} runs a binary that reads the EDA tree and must require it \
                 (`export LOGOS_REQUIRE_TOOLS=\"${{LOGOS_REQUIRE_TOOLS:+$LOGOS_REQUIRE_TOOLS,}}eda-tools\"`); it requires {required:?}"
            );
        }
    }
}
