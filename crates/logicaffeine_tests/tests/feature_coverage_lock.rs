//! ★ LOCK — no feature-gated test is dark. ★
//!
//! A test compiled only under a cargo feature exists in exactly the runs that turn the feature on.
//! The baseline (`scripts/run-all-tests-fast.sh`) builds the workspace with one feature set; a test
//! behind any other feature is not filtered out of it — it is never compiled, and every report stays
//! green. That is how the kernel's NbE suites, the io_uring engine battery, the Cap'n Proto
//! head-to-heads (which no longer even compiled under their own feature), `phase_ffi_requires`' end-to-end
//! runs and the narrow VM value's unit row sat unrun.
//!
//! The invariant: every `cfg` gate that mentions a feature, on a test binary, on anything inside one, on a
//! unit-test module or on anything inside one, holds in at least one invocation of a named runner — the
//! baseline, or an on-demand `scripts/run-*-tests.sh`. The gates are read with the shared module resolver
//! (`common/module_graph.rs`), every path from a crate root or a test binary's root to a file carrying
//! its own gates; the runners' `cargo nextest run` / `cargo test` invocations are read from the scripts
//! themselves, and each invocation's per-crate feature set is what `cargo tree` resolves for exactly those
//! arguments, so feature unification (a dev-dependency turning on `lean`, `full` turning on `relay`) is
//! cargo's, not a restatement. A gate is evaluated in three values: a feature is on or off in an
//! invocation, `test` holds, and every other atom (target, `unix`, `debug_assertions`, …) is left
//! undecided, so a platform branch never reads as dark here — only a feature combination no invocation
//! turns on does.
//!
//! The on-demand opt-in runner is held the other way too: each binary and library it names must carry a
//! gate that it reaches and the baseline does not, so it cannot fill up with runs that test nothing new.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/module_graph.rs"]
mod module_graph;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use module_graph::{normalize, top_level_args};

const GATE: &str = "scripts/run-all-tests-fast.sh";
const OPT_IN_RUNNER: &str = "scripts/run-opt-in-feature-tests.sh";
const TESTS_PACKAGE: &str = "logicaffeine-tests";

fn repo_root() -> PathBuf {
    normalize(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn rel(p: &Path) -> String {
    p.strip_prefix(repo_root())
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

// ─── the runners' invocations ──────────────────────────────────────────────────────────────────

/// Which targets of the selected packages an invocation builds.
#[derive(Clone, Debug, Default)]
struct Targets {
    /// No `--lib`/`--test`/`--doc` restriction: every target.
    all: bool,
    lib: bool,
    tests: BTreeSet<String>,
    doc: bool,
}

/// One `cargo nextest run` / `cargo test` line of a runner script.
#[derive(Clone, Debug)]
struct Invocation {
    script: String,
    line: usize,
    workspace: bool,
    packages: Vec<String>,
    features: Vec<String>,
    no_default_features: bool,
    all_features: bool,
    target: Option<String>,
    targets: Targets,
    /// The `-E` / `--filterset` arguments, as written.
    filters: Vec<String>,
}

impl Invocation {
    fn describe(&self) -> String {
        format!("{}:{}", self.script, self.line)
    }

    fn selects(&self, package: &str) -> bool {
        self.workspace || self.packages.iter().any(|p| p == package)
    }

    fn builds_lib(&self) -> bool {
        self.targets.all || self.targets.lib
    }

    fn builds_bins(&self) -> bool {
        self.targets.all
    }

    fn builds_test(&self, binary: &str) -> bool {
        self.targets.all || self.targets.tests.contains(binary)
    }
}

/// A script's logical lines (each `\`-continued run joined), with the 1-based line each starts on.
fn logical_lines(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut start = 0;
    for (i, line) in src.lines().enumerate() {
        if cur.is_empty() {
            start = i + 1;
        }
        match line.strip_suffix('\\') {
            Some(continued) => {
                cur.push_str(continued);
                cur.push(' ');
            }
            None => {
                cur.push_str(line);
                out.push((start, std::mem::take(&mut cur)));
            }
        }
    }
    if !cur.is_empty() {
        out.push((start, cur));
    }
    out
}

/// Shell words: quotes, `$(…)` and `${…}` stay one word, quotes removed only where they delimit.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    cur.push(chars[i]);
                    i += 1;
                }
            }
            '"' => {
                in_word = true;
                i += 1;
                let mut depth = 0i32;
                while i < chars.len() && !(chars[i] == '"' && depth == 0) {
                    if chars[i] == '$'
                        && i + 1 < chars.len()
                        && (chars[i + 1] == '(' || chars[i + 1] == '{')
                    {
                        depth += 1;
                        cur.push(chars[i]);
                        cur.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    if depth > 0 && (chars[i] == ')' || chars[i] == '}') {
                        depth -= 1;
                    }
                    cur.push(chars[i]);
                    i += 1;
                }
            }
            '$' if i + 1 < chars.len() && (chars[i + 1] == '(' || chars[i + 1] == '{') => {
                in_word = true;
                let (open, close) = if chars[i + 1] == '(' {
                    ('(', ')')
                } else {
                    ('{', '}')
                };
                cur.push('$');
                cur.push(open);
                i += 2;
                let mut depth = 1;
                while i < chars.len() && depth > 0 {
                    if chars[i] == open {
                        depth += 1;
                    } else if chars[i] == close {
                        depth -= 1;
                    }
                    cur.push(chars[i]);
                    i += 1;
                }
                continue;
            }
            _ => {
                in_word = true;
                cur.push(c);
            }
        }
        i += 1;
    }
    if in_word {
        words.push(cur);
    }
    words
}

/// The single-quoted value of a top-level shell assignment `NAME='…'`.
fn shell_var(src: &str, var: &str) -> Option<String> {
    let needle = format!("\n{var}='");
    let start = src.find(&needle)? + needle.len();
    let end = src[start..].find('\'')?;
    Some(src[start..start + end].to_string())
}

/// The binaries named by `binary(x)` in a filterset.
fn filter_binaries(filterset: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = filterset;
    while let Some(i) = rest.find("binary(") {
        let after = &rest[i + "binary(".len()..];
        let end = after.find(')').unwrap_or(after.len());
        out.insert(after[..end].trim().to_string());
        rest = &after[end..];
    }
    out
}

/// Flags whose value is the next word and says nothing about what is built.
const VALUED_FLAGS: &[&str] = &[
    "-E",
    "--filterset",
    "--profile",
    "--cargo-profile",
    "--run-ignored",
    "--partition",
    "--message-format",
    "--status-level",
    "--final-status-level",
    "--exclude",
    "-j",
    "--jobs",
    "--test-threads",
    "--retries",
    "--archive-file",
    "--workspace-remap",
    "--manifest-path",
];

/// Every test invocation in one runner script.
fn invocations(script: &str) -> Vec<Invocation> {
    let root = repo_root();
    let src = read(&root.join(script));
    let mut out = Vec::new();
    for (line_no, line) in logical_lines(&src) {
        let words = shell_words(line.trim_start());
        let mut w = words.as_slice();
        if w.first().is_some_and(|x| x == "exec") {
            w = &w[1..];
        }
        let rest = match w {
            [cargo, nextest, run, rest @ ..]
                if cargo == "cargo" && nextest == "nextest" && run == "run" =>
            {
                rest
            }
            [cargo, test, rest @ ..] if cargo == "cargo" && test == "test" => rest,
            _ => continue,
        };
        let mut inv = Invocation {
            script: script.to_string(),
            line: line_no,
            workspace: false,
            packages: Vec::new(),
            features: Vec::new(),
            no_default_features: false,
            all_features: false,
            target: None,
            targets: Targets::default(),
            filters: Vec::new(),
        };
        let mut restricted = false;
        let mut i = 0;
        while i < rest.len() {
            let word = rest[i].as_str();
            let value = rest.get(i + 1).cloned();
            if word == "--"
                || word.starts_with('|')
                || word.starts_with('>')
                || word.starts_with("2>")
                || word.starts_with('&')
                || word.starts_with(';')
            {
                break;
            }
            match word {
                "--workspace" | "--all" => inv.workspace = true,
                "-p" | "--package" => {
                    inv.packages.push(value.expect("-p takes a package"));
                    i += 1;
                }
                "--features" | "-F" => {
                    inv.features.extend(
                        value
                            .expect("--features takes a list")
                            .split([',', ' '])
                            .filter(|f| !f.is_empty())
                            .map(String::from),
                    );
                    i += 1;
                }
                "--no-default-features" => inv.no_default_features = true,
                "--all-features" => inv.all_features = true,
                "--target" => {
                    inv.target = value;
                    i += 1;
                }
                "--lib" => {
                    restricted = true;
                    inv.targets.lib = true;
                }
                "--doc" => {
                    restricted = true;
                    inv.targets.doc = true;
                }
                "--test" => {
                    restricted = true;
                    inv.targets
                        .tests
                        .insert(value.expect("--test takes a binary"));
                    i += 1;
                }
                "${TEST_ARGS[@]}" => {
                    restricted = true;
                    let list = shell_var(&src, "BINARIES").unwrap_or_else(|| {
                        panic!("{script}:{line_no} passes TEST_ARGS but assigns no BINARIES")
                    });
                    inv.targets
                        .tests
                        .extend(list.split_whitespace().map(String::from));
                }
                w if w.starts_with("--features=") => {
                    inv.features.extend(
                        w["--features=".len()..]
                            .split(',')
                            .filter(|f| !f.is_empty())
                            .map(String::from),
                    );
                }
                w if w.starts_with("$(") && w.contains("wasm-jit-targets.sh") => {
                    // The helper prints `--test` for every `binary(…)` of the filter file beside it.
                    restricted = true;
                    let at = w.find("scripts/").unwrap_or_else(|| {
                        panic!("{script}:{line_no}: `{w}` names no scripts/ path")
                    });
                    let helper = &w
                        [at..w.find("wasm-jit-targets.sh").unwrap() + "wasm-jit-targets.sh".len()];
                    let filter = root.join(helper).with_file_name("wasm-jit-filter.txt");
                    inv.targets.tests.extend(filter_binaries(&read(&filter)));
                }
                "-E" | "--filterset" => {
                    inv.filters.push(value.expect("-E takes a filterset"));
                    i += 1;
                }
                "${EXCLUDE_ARGS[@]}"
                | "${NEXTEST_IGNORED_ARGS[@]}"
                | "${DOCTEST_IGNORED_ARGS[@]}" => {}
                w if w.starts_with("$(") || w.starts_with("${") => {
                    panic!("{script}:{line_no}: `{w}` expands to arguments this lock cannot read — name the targets and features literally")
                }
                w if VALUED_FLAGS.contains(&w) => i += 1,
                _ => {}
            }
            i += 1;
        }
        inv.targets.all = !restricted;
        assert!(
            inv.workspace || !inv.packages.is_empty(),
            "{script}:{line_no}: a test invocation with neither --workspace nor -p builds the default members, which \
             this lock does not model — select the packages explicitly"
        );
        out.push(inv);
    }
    out
}

/// The baseline and every on-demand runner: `scripts/run-all-tests-fast.sh` and every other
/// `scripts/run-*-tests.sh` (the slow cargo-test baseline, `run-all-tests.sh`, is a strict subset of the
/// fast one's feature set and is not a separate home for anything).
fn runner_scripts() -> Vec<String> {
    let mut out = vec![GATE.to_string()];
    let mut others: Vec<String> = std::fs::read_dir(repo_root().join("scripts"))
        .expect("read scripts/")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("run-") && n.ends_with("-tests.sh") && n != "run-all-tests.sh")
        .map(|n| format!("scripts/{n}"))
        .collect();
    others.sort();
    out.extend(others);
    out
}

// ─── what cargo turns on ───────────────────────────────────────────────────────────────────────

/// Package name → the features cargo enables for it under this invocation's arguments, and package
/// name → its directory, for every path package in the build.
struct Resolved {
    features: BTreeMap<String, BTreeSet<String>>,
    dirs: BTreeMap<String, PathBuf>,
}

fn resolve(inv: &Invocation) -> Resolved {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(repo_root()).args([
        "tree",
        "--offline",
        "--locked",
        "-e",
        "normal,dev",
        "--prefix",
        "none",
        "-f",
        "{p}|{f}",
    ]);
    if inv.workspace {
        cmd.arg("--workspace");
    }
    for p in &inv.packages {
        cmd.args(["-p", p]);
    }
    if !inv.features.is_empty() {
        cmd.args(["--features", &inv.features.join(",")]);
    }
    if inv.no_default_features {
        cmd.arg("--no-default-features");
    }
    if inv.all_features {
        cmd.arg("--all-features");
    }
    if let Some(t) = &inv.target {
        cmd.args(["--target", t]);
    }
    let out = cmd.output().expect("run cargo tree");
    assert!(
        out.status.success(),
        "cargo tree for {} failed:\n{}",
        inv.describe(),
        String::from_utf8_lossy(&out.stderr)
    );
    let mut features: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut dirs = BTreeMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let line = line.trim_end_matches(" (*)");
        let Some((pkg, feats)) = line.rsplit_once('|') else {
            continue;
        };
        let Some(open) = pkg.find(" (") else { continue };
        let Some(path) = pkg[open + 2..].strip_suffix(')') else {
            continue;
        };
        if !path.starts_with('/') {
            continue;
        }
        let name = pkg.split_whitespace().next().unwrap().to_string();
        dirs.insert(name.clone(), normalize(Path::new(path)));
        features
            .entry(name)
            .or_default()
            .extend(feats.split(',').filter(|f| !f.is_empty()).map(String::from));
    }
    assert!(
        features.contains_key(TESTS_PACKAGE) || !inv.selects(TESTS_PACKAGE),
        "cargo tree for {} did not list {TESTS_PACKAGE} — its output format changed",
        inv.describe()
    );
    Resolved { features, dirs }
}

// ─── the gates ─────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Truth {
    Yes,
    No,
    Unknown,
}

/// A compacted cfg predicate under a feature set: features decide, `test` holds, anything else is open.
fn eval(pred: &str, features: &BTreeSet<String>) -> Truth {
    if let Some(rest) = pred.strip_prefix("all(") {
        let v: Vec<Truth> = top_level_args(rest)
            .iter()
            .map(|a| eval(a, features))
            .collect();
        return if v.contains(&Truth::No) {
            Truth::No
        } else if v.iter().all(|t| *t == Truth::Yes) {
            Truth::Yes
        } else {
            Truth::Unknown
        };
    }
    if let Some(rest) = pred.strip_prefix("any(") {
        let v: Vec<Truth> = top_level_args(rest)
            .iter()
            .map(|a| eval(a, features))
            .collect();
        return if v.contains(&Truth::Yes) {
            Truth::Yes
        } else if v.iter().all(|t| *t == Truth::No) {
            Truth::No
        } else {
            Truth::Unknown
        };
    }
    if let Some(rest) = pred.strip_prefix("not(") {
        let args = top_level_args(rest);
        assert_eq!(args.len(), 1, "not() takes one predicate: {pred}");
        return match eval(&args[0], features) {
            Truth::Yes => Truth::No,
            Truth::No => Truth::Yes,
            Truth::Unknown => Truth::Unknown,
        };
    }
    if let Some(name) = pred.strip_prefix("feature=") {
        return if features.contains(name.trim_matches('"')) {
            Truth::Yes
        } else {
            Truth::No
        };
    }
    if pred == "test" {
        return Truth::Yes;
    }
    Truth::Unknown
}

/// Whether a compacted predicate has a `feature="…"` atom (`target_feature` is a target property).
fn has_feature_atom(pred: &str) -> bool {
    pred.match_indices("feature=")
        .any(|(at, _)| at == 0 || matches!(pred.as_bytes()[at - 1], b'(' | b','))
}

fn mentions_feature(preds: &[String]) -> bool {
    preds.iter().any(|p| has_feature_atom(p))
}

/// Where a gate is compiled: into a test binary of the tests crate, or into the library or a binary of
/// the crate whose unit tests hold it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Home {
    TestBinary(String),
    Lib(String),
    Bin(String),
}

/// One way a gate is compiled: into this home, when all these predicates hold (the gates on the path
/// from the home's root to the file, then the item's own).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Route {
    home: Home,
    preds: Vec<String>,
}

/// A gate in test code that mentions a feature: a file whose path carries one (line 0), or a gated item.
/// It compiles when any one of its paths holds.
#[derive(Clone, Debug)]
struct Site {
    file: PathBuf,
    line: usize,
    paths: Vec<Route>,
}

impl Site {
    fn at(&self) -> String {
        if self.line == 0 {
            format!("{} (the whole file)", rel(&self.file))
        } else {
            format!("{}:{}", rel(&self.file), self.line)
        }
    }
}

/// Every path of gates from the given roots to every file they reach: file → (root, predicates).
fn chains(
    graph: &module_graph::ModuleGraph,
    roots: &[PathBuf],
) -> BTreeMap<PathBuf, Vec<(PathBuf, Vec<String>)>> {
    let files: BTreeMap<&Path, &module_graph::SourceFile> =
        graph.files.iter().map(|f| (f.path.as_path(), f)).collect();
    let mut out: BTreeMap<PathBuf, Vec<(PathBuf, Vec<String>)>> = BTreeMap::new();
    let mut seen: BTreeSet<(PathBuf, PathBuf, Vec<String>)> = BTreeSet::new();
    let mut work: Vec<(PathBuf, PathBuf, Vec<String>, usize)> = Vec::new();
    for root in roots {
        let Some(f) = files.get(root.as_path()) else {
            continue;
        };
        work.push((root.clone(), root.clone(), f.inner_cfg.clone(), 0));
    }
    while let Some((root, file, chain, depth)) = work.pop() {
        assert!(
            depth < 64,
            "module nesting from {} runs past 64 levels at {} — a `#[path]` cycle",
            rel(&root),
            rel(&file)
        );
        if !seen.insert((root.clone(), file.clone(), chain.clone())) {
            continue;
        }
        out.entry(file.clone())
            .or_default()
            .push((root.clone(), chain.clone()));
        let Some(f) = files.get(file.as_path()) else {
            continue;
        };
        for m in &f.mods {
            let Some(child) = files.get(m.target.as_path()) else {
                continue;
            };
            let mut c = chain.clone();
            c.extend(m.cfg.iter().cloned());
            c.extend(child.inner_cfg.iter().cloned());
            work.push((root.clone(), m.target.clone(), c, depth + 1));
        }
    }
    out
}

/// The feature-mentioning gates of the files `keep` accepts: the file itself (line 0) when a path to it
/// carries a feature, and every gated item in it whose own predicate mentions one — each with every path
/// that compiles it, the root of a path named by `home`.
fn sites_of(
    graph: &module_graph::ModuleGraph,
    reach: &BTreeMap<PathBuf, Vec<(PathBuf, Vec<String>)>>,
    keep: impl Fn(&Path) -> bool,
    home: impl Fn(&Path) -> Option<Home>,
) -> Vec<Site> {
    let mut out: BTreeMap<(PathBuf, usize), BTreeSet<Route>> = BTreeMap::new();
    for f in &graph.files {
        if !keep(&f.path) {
            continue;
        }
        let Some(paths) = reach.get(&f.path) else {
            continue;
        };
        let whole_file_gated = paths.iter().any(|(_, chain)| mentions_feature(chain));
        for (root, chain) in paths {
            let Some(home) = home(root) else { continue };
            if whole_file_gated {
                out.entry((f.path.clone(), 0)).or_default().insert(Route {
                    home: home.clone(),
                    preds: chain.clone(),
                });
            }
            for site in &f.cfg_sites {
                if site.cfg.last().is_some_and(|own| has_feature_atom(own)) {
                    let mut preds = chain.clone();
                    preds.extend(site.cfg.iter().cloned());
                    out.entry((f.path.clone(), site.line))
                        .or_default()
                        .insert(Route {
                            home: home.clone(),
                            preds,
                        });
                }
            }
        }
    }
    out.into_iter()
        .map(|((file, line), paths)| Site {
            file,
            line,
            paths: paths.into_iter().collect(),
        })
        .collect()
}

struct Census {
    invocations: Vec<(Invocation, Resolved)>,
    /// Every feature-mentioning gate in test code.
    sites: Vec<Site>,
    /// Each test binary of the tests crate and the `#![cfg]` predicates of its root file.
    binary_gates: BTreeMap<String, Vec<String>>,
}

impl Census {
    fn take() -> Census {
        let root = repo_root();
        let mut all = Vec::new();
        for script in runner_scripts() {
            for inv in invocations(&script) {
                if inv.targets.doc
                    && !inv.targets.all
                    && !inv.targets.lib
                    && inv.targets.tests.is_empty()
                {
                    continue;
                }
                let resolved = resolve(&inv);
                all.push((inv, resolved));
            }
        }

        let mut sites = Vec::new();

        let tests_crate = root.join("crates/logicaffeine_tests");
        // cargo's integration-test autodiscovery: `tests/<name>.rs` and `tests/<name>/main.rs`.
        let mut binary_roots: Vec<PathBuf> = std::fs::read_dir(tests_crate.join("tests"))
            .expect("read tests/")
            .flatten()
            .map(|e| e.path())
            .filter_map(|p| match () {
                _ if p.extension().is_some_and(|x| x == "rs") => Some(p),
                _ if p.join("main.rs").is_file() => Some(p.join("main.rs")),
                _ => None,
            })
            .map(|p| normalize(&p))
            .collect();
        binary_roots.sort();
        let graph = module_graph::walk_roots(&tests_crate, binary_roots.clone());
        let reach = chains(&graph, &binary_roots);
        let tests_dir = normalize(&tests_crate.join("tests"));
        let binary_name = |r: &Path| {
            let named = if r.ends_with("main.rs") {
                r.parent().unwrap().file_name()
            } else {
                r.file_stem()
            };
            named.unwrap().to_string_lossy().into_owned()
        };
        let binary_gates: BTreeMap<String, Vec<String>> = graph
            .files
            .iter()
            .filter(|f| binary_roots.contains(&f.path))
            .map(|f| (binary_name(&f.path), f.inner_cfg.clone()))
            .collect();
        sites.extend(sites_of(
            &graph,
            &reach,
            |p| p.starts_with(&tests_dir),
            |r| Some(Home::TestBinary(binary_name(r))),
        ));

        let unit_root = normalize(&tests_crate.join("unit"));
        let mut dirs: BTreeMap<String, PathBuf> = BTreeMap::new();
        for (_, resolved) in &all {
            dirs.extend(resolved.dirs.iter().map(|(n, d)| (n.clone(), d.clone())));
        }
        for (package, dir) in &dirs {
            let unit_dir = unit_root.join(dir.file_name().unwrap());
            if package == TESTS_PACKAGE || !unit_dir.is_dir() {
                continue;
            }
            let graph = module_graph::walk(dir);
            let roots = module_graph::roots(dir)
                .into_iter()
                .map(|r| normalize(&r))
                .collect::<Vec<_>>();
            let reach = chains(&graph, &roots);
            let lib = normalize(&dir.join("src/lib.rs"));
            sites.extend(sites_of(
                &graph,
                &reach,
                |p| p.starts_with(&unit_dir),
                |r| match () {
                    _ if r.ends_with("build.rs") => None,
                    _ if r == lib => Some(Home::Lib(package.clone())),
                    _ => Some(Home::Bin(package.clone())),
                },
            ));
        }
        Census {
            invocations: all,
            sites,
            binary_gates,
        }
    }

    /// Whether this invocation compiles this path of a gate.
    fn reaches(inv: &Invocation, resolved: &Resolved, path: &Route) -> bool {
        let (package, built) = match &path.home {
            Home::TestBinary(b) => (TESTS_PACKAGE, inv.builds_test(b)),
            Home::Lib(p) => (p.as_str(), inv.builds_lib()),
            Home::Bin(p) => (p.as_str(), inv.builds_bins()),
        };
        if !built || !inv.selects(package) {
            return false;
        }
        let Some(features) = resolved.features.get(package) else {
            return false;
        };
        path.preds.iter().all(|p| eval(p, features) != Truth::No)
    }

    /// Whether some invocation `pick` accepts compiles this path.
    fn reached_by(&self, path: &Route, pick: impl Fn(&Invocation) -> bool) -> bool {
        self.invocations
            .iter()
            .filter(|(inv, _)| pick(inv))
            .any(|(inv, resolved)| Census::reaches(inv, resolved, path))
    }
}

fn census() -> &'static Census {
    static CENSUS: std::sync::OnceLock<Census> = std::sync::OnceLock::new();
    CENSUS.get_or_init(Census::take)
}

// ─── the rows ──────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_census_reads_the_baseline_and_the_opt_in_runner() {
    let c = census();
    let gate: Vec<&Invocation> = c
        .invocations
        .iter()
        .map(|(i, _)| i)
        .filter(|i| i.script == GATE)
        .collect();
    assert!(
        gate.iter().any(|i| i.workspace
            && i.targets.all
            && i.features.iter().any(|f| f == "verification")),
        "the baseline's workspace pass was not read from {GATE}: {gate:#?}"
    );
    assert!(
        gate.iter()
            .any(|i| i.features.iter().any(|f| f == "wasm-jit")
                && i.targets.lib
                && !i.targets.tests.is_empty()),
        "the baseline's wasm-jit pass was not read from {GATE}: {gate:#?}"
    );
    let workspace = &c
        .invocations
        .iter()
        .find(|(i, _)| i.script == GATE && i.workspace)
        .unwrap()
        .1;
    for (package, feature) in [
        ("logicaffeine-kernel", "lean"),
        ("logicaffeine-system", "relay"),
        (TESTS_PACKAGE, "verification"),
    ] {
        assert!(
            workspace.features.get(package).is_some_and(|f| f.contains(feature)),
            "cargo tree no longer reports `{package}/{feature}` on in the baseline — the resolution this lock rests \
             on changed: {:?}",
            workspace.features.get(package)
        );
    }
    let opt_in = c
        .invocations
        .iter()
        .filter(|(i, _)| i.script == OPT_IN_RUNNER)
        .count();
    assert!(
        opt_in >= 5,
        "only {opt_in} invocations read from {OPT_IN_RUNNER}"
    );

    // The gates this lock exists for are found where they are written — a census that stopped seeing
    // them would pass on nothing.
    let seen = |file: &str, pred: &str| {
        c.sites.iter().any(|s| {
            rel(&s.file) == file
                && s.paths
                    .iter()
                    .any(|p| p.preds.iter().any(|q| q.contains(pred)))
        })
    };
    for (file, pred) in [
        ("crates/logicaffeine_tests/tests/dag_nbe_checker.rs", "feature=\"nbe\""),
        ("crates/logicaffeine_tests/tests/http_engine_uring.rs", "feature=\"io-uring\""),
        ("crates/logicaffeine_tests/tests/phase_ffi_requires.rs", "feature=\"ffi-link-tests\""),
        ("crates/logicaffeine_tests/unit/logicaffeine_compile/vm/value/fast_path_kernel_differential.rs", "feature=\"narrow-value\""),
        ("crates/logicaffeine_tests/unit/logicaffeine_system/fs/tests.rs", "feature=\"io-uring\""),
    ] {
        assert!(seen(file, pred), "the census no longer finds the `{pred}` gate in {file}");
    }
}

#[test]
fn every_feature_gated_test_is_compiled_by_some_runner() {
    let c = census();
    let mut dark = Vec::new();
    for site in &c.sites {
        if !site.paths.iter().any(|p| c.reached_by(p, |_| true)) {
            let gates: Vec<String> = site
                .paths
                .iter()
                .map(|p| format!("{:?}: {}", p.home, p.preds.join(" ∧ ")))
                .collect();
            dark.push(format!("{}\n      {}", site.at(), gates.join("\n      ")));
        }
    }
    assert!(
        dark.is_empty(),
        "{} feature-gated test site(s) that no runner invocation compiles — they never run anywhere:\n  {}\n\n\
         Turn the feature on for them in an invocation of scripts/run-opt-in-feature-tests.sh (one invocation per \
         feature, naming its --test binaries or --lib), or in the baseline if the feature belongs in every run.",
        dark.len(),
        dark.join("\n  ")
    );
}

#[test]
fn every_opt_in_invocation_compiles_a_gate_the_baseline_cannot() {
    let c = census();
    let mut idle = Vec::new();
    for (inv, resolved) in c
        .invocations
        .iter()
        .filter(|(i, _)| i.script == OPT_IN_RUNNER)
    {
        assert!(
            !inv.targets.all,
            "{}: an opt-in invocation must name its targets (--test / --lib)",
            inv.describe()
        );
        let mut named: Vec<Home> = inv
            .targets
            .tests
            .iter()
            .map(|b| Home::TestBinary(b.clone()))
            .collect();
        if inv.targets.lib {
            named.extend(
                inv.packages
                    .iter()
                    .filter(|p| *p != TESTS_PACKAGE)
                    .map(|p| Home::Lib(p.clone())),
            );
        }
        assert!(!named.is_empty(), "{}: names no target", inv.describe());
        for home in named {
            let earns = c.sites.iter().flat_map(|s| &s.paths).any(|path| {
                path.home == home
                    && Census::reaches(inv, resolved, path)
                    && !c.reached_by(path, |i| i.script == GATE)
            });
            if !earns {
                idle.push(format!("{} {home:?}", inv.describe()));
            }
        }
    }
    assert!(
        idle.is_empty(),
        "these opt-in runs compile no gate the baseline does not already compile, so they test nothing new:\n  {}",
        idle.join("\n  ")
    );
}

/// A runner that names a test binary compiles it. A binary whose root file is gated (`#![cfg(feature =
/// "nbe")]`) builds to an empty harness under any other feature set, so a runner naming it without the
/// feature reports its selection run while running none of it. Every `--test` (and every binary a
/// runner's `BINARIES` passes as one) must be compiled by the features its own invocation resolves to.
#[test]
fn every_binary_a_runner_names_is_compiled_by_it() {
    let c = census();
    let mut empty = Vec::new();
    for (inv, resolved) in &c.invocations {
        if !inv.selects(TESTS_PACKAGE) {
            continue;
        }
        let features = resolved
            .features
            .get(TESTS_PACKAGE)
            .cloned()
            .unwrap_or_default();
        for bin in &inv.targets.tests {
            let gates = c.binary_gates.get(bin).unwrap_or_else(|| {
                panic!(
                    "{} names `{bin}`, which is not a test binary of {TESTS_PACKAGE}",
                    inv.describe()
                )
            });
            if let Some(off) = gates.iter().find(|g| eval(g, &features) == Truth::No) {
                empty.push(format!(
                    "{} names `{bin}`, compiled only when `{off}`",
                    inv.describe()
                ));
            }
        }
    }
    assert!(
        empty.is_empty(),
        "runner invocations that name a test binary their own features compile to nothing — its tests never run \
         there:\n  {}\n\nTurn the gate's feature on in that invocation.",
        empty.join("\n  ")
    );
}

/// The opt-in pass keeps the baseline's exclusion classes. A test `run-all-tests-fast.sh` leaves to an
/// on-demand runner (a whole-Mathlib case, a memory-pathological proof) would otherwise run here as well,
/// unbudgeted, under an unrelated feature. The runner builds its filter from the gate's own `*_EXCLUDE`
/// assignments, and every invocation filters by it.
#[test]
fn every_opt_in_invocation_keeps_the_baseline_exclusions() {
    let src = read(&repo_root().join(OPT_IN_RUNNER));
    assert!(
        src.contains("done < <(grep -E \"^[A-Z_]+_EXCLUDE='\" scripts/run-all-tests-fast.sh)"),
        "{OPT_IN_RUNNER} must build BASELINE_FILTER from every `*_EXCLUDE` assignment of {GATE}"
    );
    let c = census();
    let unfiltered: Vec<String> = c
        .invocations
        .iter()
        .map(|(inv, _)| inv)
        .filter(|inv| inv.script == OPT_IN_RUNNER && inv.filters != ["$BASELINE_FILTER"])
        .map(|inv| format!("{} filters by {:?}", inv.describe(), inv.filters))
        .collect();
    assert!(
        unfiltered.is_empty(),
        "opt-in invocations that do not filter by exactly the baseline's exclusions (`-E \"$BASELINE_FILTER\"`):\n  {}",
        unfiltered.join("\n  ")
    );
}
