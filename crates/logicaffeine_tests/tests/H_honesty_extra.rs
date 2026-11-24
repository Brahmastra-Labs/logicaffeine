//! THE SCOREBOARD MUST NOT BE ABLE TO LIE — the locks around `conformance-verify-verdicts.sh`.
//!
//! `lean_parity_matrix.rs` is the Lean-parity scoreboard. It proves every one of the 248
//! corpus cases is classified, that no two share a decider, that every DIVERGE/OUT carries an
//! argument, and that `TIER_FLOORS` never falls. It reads the manifest and the verdict JSONs as
//! TEXT and never runs a corpus test, so `"status": "Native"` is an unverified *assertion* about a
//! capability and `TIER_FLOORS` is a ratchet on a number nobody measured.
//!
//! `scripts/lean/conformance-verify-verdicts.sh` closes that: it runs every corpus binary and joins the
//! real results against the verdicts by test name. This file is what keeps THAT honest.
//!
//! ## Why a committed results file would be worse than the hole it replaces
//!
//! The obvious shape is "commit a results JSON, read it from a Rust lock with `include_str!`".
//! Do not. Such a file is stale the instant anyone edits kernel source, and nothing in the repo can
//! detect it: it still parses, the lock still reads it, the lock still goes green. It is a
//! photograph of a machine that no longer exists, and a green light nobody can tell apart from a
//! measurement — the exact dishonesty the script exists to close, rebuilt one layer up.
//!
//! So results are produced in the same invocation that reports them, always, and the five locks
//! here pin the only failure modes that remain:
//!
//!   1. `the_verdict_gate_is_wired_into_the_baseline` — someone deletes the gate from
//!      `run-all-tests-fast.sh`. That is the one staleness that can actually hurt you, so deleting
//!      it is itself a RED test.
//!   2. `no_test_reads_a_recorded_verdict_verification_result` — someone reintroduces the
//!      committed-results pattern. The trap is shut before anyone falls into it.
//!   3. `every_corpus_binary_a_verdict_names_exists_on_disk` +
//!      `the_gates_binary_discovery_agrees_with_an_independent_resolver` — a renamed binary makes
//!      the gate collect the wrong set. Caught in milliseconds, without a build, by resolving the
//!      binaries a SECOND time here and requiring the two answers to agree. The same lock refuses an
//!      AMBIGUOUS name: verdicts address a decider by bare binary name, and eight test-file stems are
//!      duplicated across this workspace, so a first-match resolver could satisfy a verdict with a
//!      same-named test in an unrelated crate.
//!   4. `the_verdict_gate_catches_every_planted_lie` — the detector stops detecting. REG-C is why
//!      this is not optional: `proof_test_honesty_lock::is_assert_true` quietly stopped matching
//!      its own synthetic violation, its self-test went red, and the anti-vacuity ratchet was
//!      protecting nothing. A guard nobody has lied to is a guard nobody has tested.
//!   5. `the_gates_selftest_plants_every_finding_the_report_can_emit` — a NEW detector is added to
//!      the gate and nobody plants a lie for it, so it never fires and nobody notices. This makes
//!      the planting structural: the gate's `SUMMARY` format declares one slot per finding, and the
//!      dishonest fixture must be non-zero in every slot while its honest twin is zero in every one.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/scratch.rs"]
mod scratch;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn gate_path() -> PathBuf {
    root().join("scripts/lean/conformance-verify-verdicts.sh")
}

fn read(p: &Path) -> String {
    fs::read_to_string(p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

// ============================================================================================
// 1. THE GATE IS WIRED IN — deleting it must be RED.
// ============================================================================================

/// The baseline must actually INVOKE the gate and must actually FOLD ITS EXIT CODE IN. Naming the
/// script in a comment is not wiring; a call whose status is discarded is not a gate.
fn baseline_invokes_the_gate(script: &str) -> bool {
    script.contains("scripts/lean/conformance-verify-verdicts.sh")
        || script.contains("conformance-verify-verdicts.sh")
}

fn baseline_folds_in_the_gates_exit(script: &str) -> bool {
    // The convention every other pass in that script follows: capture into a *_EXIT variable and
    // let it reach EXIT_CODE.
    script.contains("VERDICTS_EXIT") && script.contains("EXIT_CODE=\"$VERDICTS_EXIT\"")
}

#[test]
fn the_verdict_gate_is_wired_into_the_baseline() {
    let gate = gate_path();
    assert!(
        gate.exists(),
        "scripts/lean/conformance-verify-verdicts.sh is missing — the Lean-parity scoreboard is back to \
         being an unverified claim"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&gate)
            .expect("stat the gate")
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "the verdict gate is not executable (mode {mode:o}); the baseline invokes it directly"
        );
    }

    let baseline = read(&root().join("scripts/run-all-tests-fast.sh"));
    assert!(
        baseline_invokes_the_gate(&baseline),
        "run-all-tests-fast.sh does not run scripts/lean/conformance-verify-verdicts.sh. Without it the \
         baseline can go green while a case recorded Native has a FAILING test — a capability we \
         claim and do not have. That is exactly what the gate exists to stop."
    );
    assert!(
        baseline_folds_in_the_gates_exit(&baseline),
        "run-all-tests-fast.sh runs the verdict gate but does not fold its exit code into \
         EXIT_CODE. A gate whose result is discarded is decoration."
    );
}

/// The corpus ships in the tree, so the gate runs on every checkout: the `if` chain around its invocation tests no
/// file or directory, and no branch of it settles `VERDICTS_EXIT=0` without running the gate.
fn gate_runs_unconditionally(script: &str) -> Result<(), String> {
    let call = script
        .find("\"$VERDICT_GATE\" ")
        .ok_or("the baseline never invokes \"$VERDICT_GATE\"")?;
    let open = script[..call]
        .rfind("\nif ")
        .ok_or("the gate's invocation sits in no if-chain")?;
    let close = call
        + script[call..]
            .find("\nfi\n")
            .ok_or("the gate's if-chain never closes")?;
    let chain = &script[open..close];
    for test in ["-d ", "-f ", "-e ", "-s "] {
        if chain.contains(test) {
            return Err(format!(
                "the gate runs only when a path exists (`{}`)",
                test.trim()
            ));
        }
    }
    if chain.contains("VERDICTS_EXIT=0") {
        return Err("a branch settles VERDICTS_EXIT=0 without running the gate".into());
    }
    Ok(())
}

#[test]
fn the_verdict_gate_runs_on_every_checkout() {
    let baseline = read(&root().join("scripts/run-all-tests-fast.sh"));
    if let Err(why) = gate_runs_unconditionally(&baseline) {
        panic!(
            "run-all-tests-fast.sh can skip the verdict gate: {why}. The corpus it scores is in the tree, so a \
             checkout without the gate's input does not exist, and a skipped gate is a green scoreboard nobody checked."
        );
    }
    let skipped_when_absent = "VERDICT_GATE=\"$SCRIPT_DIR/lean/conformance-verify-verdicts.sh\"\n\
        if [[ -d \"$REPO_ROOT/work/corpus/verdicts\" && ! -x \"$VERDICT_GATE\" ]]; then\n  VERDICTS_EXIT=1\n\
        elif [[ -d \"$REPO_ROOT/work/corpus/verdicts\" ]]; then\n  \"$VERDICT_GATE\" --features x\n  VERDICTS_EXIT=\"$?\"\n\
        else\n  VERDICTS_EXIT=0\nfi\n";
    assert!(
        gate_runs_unconditionally(skipped_when_absent).is_err(),
        "a gate behind a directory test is caught"
    );
    let settled = "if [[ ! -x \"$VERDICT_GATE\" ]]; then\n  VERDICTS_EXIT=0\nelse\n  \"$VERDICT_GATE\" --features x\n  \
        VERDICTS_EXIT=\"$?\"\nfi\n";
    assert!(
        gate_runs_unconditionally(&format!("\n{settled}")).is_err(),
        "a branch that settles green is caught"
    );
    let honest = "\nif [[ ! -x \"$VERDICT_GATE\" ]]; then\n  VERDICTS_EXIT=1\nelse\n  \"$VERDICT_GATE\" --features x\n  \
        VERDICTS_EXIT=\"$?\"\nfi\n";
    assert_eq!(
        gate_runs_unconditionally(honest),
        Ok(()),
        "a gate that fails when missing and otherwise runs is honest"
    );
}

// ============================================================================================
// 2. NOBODY MAY READ A RECORDED RESULT — the staleness trap, shut in advance.
// ============================================================================================

/// The artifact basenames the gate writes. The SOURCE verdicts under `crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/`
/// are a different thing entirely — they are the claim, hand-written and reviewed, and
/// `lean_parity_matrix.rs` reads them on purpose. What must never be read is a RESULT.
const RESULT_ARTIFACTS: &[&str] = &[
    "latest-verdicts",
    "conformance-verdicts-",
    "verdict-results",
];

fn reads_a_recorded_result(line: &str) -> bool {
    let reads = line.contains("include_str!")
        || line.contains("include_bytes!")
        || line.contains("read_to_string")
        || line.contains("File::open");
    reads && RESULT_ARTIFACTS.iter().any(|a| line.contains(a))
}

fn rust_sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if name == "target" || name == "node_modules" || name.starts_with('.') {
                    continue;
                }
                walk(&p, out);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for base in ["crates", "apps"] {
        walk(&root().join(base), &mut out);
    }
    out.sort();
    out
}

#[test]
fn no_test_reads_a_recorded_verdict_verification_result() {
    let mut offenders = Vec::new();
    for path in rust_sources() {
        // This file NAMES the artifacts in order to ban them; that is not reading one.
        if path.file_name().and_then(|s| s.to_str()) == Some("H_honesty_extra.rs") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or(line);
            if reads_a_recorded_result(code) {
                offenders.push(format!(
                    "{}:{}  {}",
                    path.strip_prefix(root()).unwrap_or(&path).display(),
                    i + 1,
                    line.trim()
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a Rust source reads a RECORDED verdict-verification result. That file is stale the moment \
         anyone edits kernel source and nothing can tell — it still parses, the lock still reads \
         it, the lock still goes green. Run scripts/lean/conformance-verify-verdicts.sh instead; it is \
         the only honest answer:\n  {}",
        offenders.join("\n  ")
    );
}

// ============================================================================================
// 3. DISCOVERY — resolved twice, independently, and the two must agree.
// ============================================================================================

/// This file's OWN resolver for "which test binary decides this case", written from the schema in
/// `crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts/README.md` rather than by reading the shell script. A verdict's `test`
/// is either a bare fn (decided by its tier's `test_binary`) or a qualified `binary::fn` — 11 of
/// the 248 use the qualified form, and a resolver that trusts `test_binary` alone mis-reports every
/// one of them as a missing test.
fn deciders() -> BTreeSet<(String, String)> {
    let dir = root().join("crates/logicaffeine_tests/tests/fixtures/lean_conformance/verdicts");
    let mut out = BTreeSet::new();
    let entries =
        fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw = read(&path);
        let v: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));
        let fallback = v["test_binary"].as_str().unwrap_or("").to_string();
        for case in v["cases"].as_array().into_iter().flatten() {
            let test = case["test"].as_str().unwrap_or("").trim().to_string();
            let (binary, name) = match test.rsplit_once("::") {
                Some((head, f)) => (
                    head.rsplit("::").next().unwrap_or(head).to_string(),
                    f.to_string(),
                ),
                None => (fallback.clone(), test.clone()),
            };
            if !binary.is_empty() && !name.is_empty() {
                out.insert((binary, name));
            }
        }
    }
    out
}

fn binaries_a_verdict_names() -> BTreeSet<String> {
    deciders().into_iter().map(|(b, _)| b).collect()
}

/// EVERY source file a binary name could resolve to — not the first one. A verdict names a test
/// binary by BARE NAME (`lean_conformance_t04_recursion`), and test-binary names are NOT unique across the
/// workspace: measured 2026-09-15, eight stems are duplicated (`L_language_extra`, `order_axioms`,
/// `parser_fixes_test`, `replay_lock`, `skip_budget_lock`, `tool_preflight_lock`,
/// `wasm_concurrency`, `web_theme_lock`). A resolver that returns the FIRST match would let a
/// verdict be satisfied by a same-named test in the wrong crate — the scoreboard reading green off
/// a test that has nothing to do with the case.
fn locate_binary_all(binary: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for base in ["crates", "apps"] {
        let b = root().join(base);
        let Ok(entries) = fs::read_dir(&b) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path().join("tests").join(format!("{binary}.rs"));
            if p.is_file() {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_corpus_binary_a_verdict_names_exists_on_disk() {
    let named = binaries_a_verdict_names();
    assert!(
        named.len() >= 13,
        "the corpus has 13 tiers; only {} binaries resolved — the resolver or the verdict schema \
         changed: {named:?}",
        named.len()
    );
    let missing: Vec<&String> = named
        .iter()
        .filter(|b| locate_binary_all(b.as_str()).is_empty())
        .collect();
    assert!(
        missing.is_empty(),
        "a verdict names a test binary with no source file. The gate would collect nothing for it \
         and report every one of its cases as a missing test — which reads as a lying scoreboard \
         when the real cause is a rename: {missing:?}"
    );

    // …and AMBIGUOUS is as bad as absent. `--test <name>` selects by name across the whole
    // workspace, so two files with one stem are two binaries the gate cannot tell apart, and a case
    // could be decided by a same-named test in an unrelated crate.
    let ambiguous: Vec<String> = named
        .iter()
        .map(|b| (b, locate_binary_all(b.as_str())))
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(b, paths)| {
            format!(
                "{b} -> {:?}",
                paths
                    .iter()
                    .map(|p| p.strip_prefix(root()).unwrap_or(p).display().to_string())
                    .collect::<Vec<_>>()
            )
        })
        .collect();
    assert!(
        ambiguous.is_empty(),
        "a verdict names a test binary that exists in more than one crate. Verdicts carry the BARE \
         name, so the gate cannot tell the two apart and a case could be decided by a same-named \
         test in an unrelated crate. Rename one, or qualify the verdict:\n  {}",
        ambiguous.join("\n  ")
    );
}

#[test]
fn the_gates_binary_discovery_agrees_with_an_independent_resolver() {
    let out = Command::new("bash")
        .arg(gate_path())
        .arg("--print-binaries")
        .current_dir(root())
        .output()
        .expect("run the verdict gate in --print-binaries mode");
    assert!(
        out.status.success(),
        "scripts/lean/conformance-verify-verdicts.sh --print-binaries failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let theirs: BTreeSet<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let mine = binaries_a_verdict_names();
    assert_eq!(
        theirs, mine,
        "the gate and this lock disagree about which binaries decide the corpus. Two independent \
         resolvers agreeing is the whole point — if one drifts, the gate silently stops measuring \
         part of the scoreboard"
    );
}

// ============================================================================================
// 4. THE DETECTOR IS FED LIES — and must catch every one.
// ============================================================================================

/// Milliseconds, no cargo, no corpus: pure fixtures in which every lie the gate exists to catch is
/// present at once, plus an honest twin in which none is. Runs in every baseline on purpose — REG-C
/// is the proof that a guard's self-test going quietly red is how a ratchet stops protecting
/// anything while still reading green.
#[test]
fn the_verdict_gate_catches_every_planted_lie() {
    let out = Command::new("bash")
        .arg(gate_path())
        .arg("--selftest")
        .current_dir(root())
        .output()
        .expect("run the verdict gate in --selftest mode");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "the verdict gate's own self-test FAILED, so the scoreboard's lie-detector is not \
         detecting:\n{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("SELFTEST: OK"),
        "expected `SELFTEST: OK` from the gate, got:\n{stdout}"
    );
}

// ============================================================================================
// 4b. EVERY DETECTOR IS FED A LIE — structurally, not by habit.
// ============================================================================================

/// Each `SUMMARY: …` line in the gate, flattened to one string: the `printf` FORMAT (which carries
/// one `%d` per finding the report can emit) and the two fixtures' EXPECTED lines.
fn summary_windows(script: &str) -> Vec<String> {
    const MARK: &str = "SUMMARY: ";
    const END: &str = "floor-above-truth";
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(i) = script[from..].find(MARK) {
        let start = from + i;
        let mut e = (start + 600).min(script.len());
        while !script.is_char_boundary(e) {
            e -= 1;
        }
        let raw = &script[start..e];
        let cut = raw.find(END).map(|k| k + END.len()).unwrap_or(raw.len());
        out.push(
            raw[..cut]
                .chars()
                .filter(|c| *c != '"' && *c != '\n')
                .collect(),
        );
        from = start + MARK.len();
    }
    out
}

fn counters(window: &str) -> Vec<usize> {
    window
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().expect("a digit run parses"))
        .collect()
}

/// A detector nobody has lied to is a detector nobody has tested — and that is not a slogan here,
/// it is REG-C: `proof_test_honesty_lock::is_assert_true` quietly stopped matching its own synthetic
/// violation, its self-test went red, and the anti-vacuity ratchet was protecting nothing.
///
/// So make "every finding is planted in the fixture" STRUCTURAL rather than a habit: the gate's
/// SUMMARY format carries one `%d` per finding it can report, the dishonest fixture's expected line
/// must have a NON-ZERO count in every one of those slots, and the honest twin's must have zero in
/// every one. Add a finding to the report and forget to plant it, and this goes red — long before
/// the new detector has a chance to silently never fire.
#[test]
fn the_gates_selftest_plants_every_finding_the_report_can_emit() {
    let script = read(&gate_path());
    let windows = summary_windows(&script);
    assert_eq!(
        windows.len(),
        3,
        "expected exactly three SUMMARY lines in the gate (the printf format, the dishonest \
         fixture's expectation, and the honest twin's); found {}:\n{windows:#?}",
        windows.len()
    );

    let format: Vec<&String> = windows.iter().filter(|w| w.contains("%d")).collect();
    assert_eq!(
        format.len(),
        1,
        "exactly one SUMMARY window is the printf format; found {}",
        format.len()
    );
    // A ratchet, like every other number in this campaign: the gate may learn to report MORE
    // kinds of lie and may never quietly report fewer. Raised to 11 in wave 2 by MALFORMED-VERDICT
    // (a `status` outside Native/Gap/Refused used to read OK whatever its test did), and to 12 in
    // wave 4 by OUT-BOOKED-AS-NATIVE (an `OUT` case whose refusal WORKS, booked `Native` instead of
    // `Refused`, so a clean decline holds a rung of that tier's TIER_FLOORS — a lie no pass/fail
    // check can see, because the test passes and passes for the right reason).
    const FINDINGS_FLOOR: usize = 12;
    let slots = format[0].matches("%d").count();
    assert!(
        slots >= FINDINGS_FLOOR,
        "the gate reports {slots} findings, floor {FINDINGS_FLOOR} — a detector was removed, or a \
         finding stopped being counted in the summary line"
    );

    let fixtures: Vec<&String> = windows.iter().filter(|w| !w.contains("%d")).collect();
    assert_eq!(
        fixtures.len(),
        2,
        "two fixture expectations: the dishonest one and its honest twin"
    );

    let mut planted = None;
    let mut silent = None;
    for w in fixtures {
        let c = counters(w);
        assert_eq!(
            c.len(),
            slots,
            "a fixture expectation must cover all {slots} findings the report emits; this one \
             covers {}:\n  {w}",
            c.len()
        );
        if c.iter().all(|n| *n == 0) {
            silent = Some(w.clone());
        } else if c.iter().all(|n| *n >= 1) {
            planted = Some(w.clone());
        } else {
            panic!(
                "a fixture expectation must be ALL non-zero (every finding planted) or ALL zero \
                 (the honest twin). This one is neither, so at least one detector is never fed a \
                 lie:\n  {w}"
            );
        }
    }
    assert!(
        planted.is_some(),
        "no fixture plants every finding — some detector in the gate has never been shown to fire"
    );
    assert!(
        silent.is_some(),
        "no honest twin — without it a detector that fires on EVERYTHING would pass the selftest, \
         which is exactly as useless as one that never fires"
    );
}

// ============================================================================================
// 3b. NO CASE MAY BE DECIDED BY A TEST THAT DOES NOT RUN.
// ============================================================================================

/// Every `#[ignore]`d `#[test]` in `path`, by function name. An attribute may sit several lines
/// above its `fn`, with other attributes (`#[should_panic]`, `#[cfg(…)]`) between.
fn ignored_tests_in(path: &Path) -> BTreeSet<String> {
    let text = read(path);
    let lines: Vec<&str> = text.lines().collect();
    let mut out = BTreeSet::new();
    for (i, line) in lines.iter().enumerate() {
        let code = line.split("//").next().unwrap_or(line).trim();
        if !code.starts_with("#[ignore") {
            continue;
        }
        for follow in lines.iter().skip(i + 1).take(12) {
            let f = follow.split("//").next().unwrap_or(follow).trim();
            if let Some(rest) = f.strip_prefix("fn ") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    out.insert(name);
                }
                break;
            }
            if !f.starts_with('#') && !f.is_empty() && !f.starts_with("pub ") {
                break;
            }
        }
    }
    out
}

/// A case whose decider is `#[ignore]`d is **unclassified**, not Native — nothing ran, so nothing
/// was proven either way. The gate reports that as `IGNORED-DECIDER`, but only after a full corpus
/// build and run; this catches it in milliseconds, in every baseline, without cargo.
///
/// Deliberately scoped to functions a verdict actually NAMES. An unrelated slow test elsewhere in
/// the same binary is somebody else's business and must not turn this red; a decider is the one
/// thing that cannot be silenced, because silencing it moves a case off the scoreboard while
/// leaving its `status` behind.
///
/// Measured 2026-09-15: zero across all 16 corpus binaries. This is a trap shut before anyone falls
/// into it. If a decider genuinely must stop running, the honest move is to change that case's
/// `status` — the gate's `IGNORED-DECIDER` row says exactly that, and this lock is the place to
/// argue the exception.
#[test]
fn no_corpus_case_is_decided_by_an_ignored_test() {
    let mut offenders = Vec::new();
    let all = deciders();
    let mut binaries: BTreeSet<String> = BTreeSet::new();
    for (b, _) in &all {
        binaries.insert(b.clone());
    }
    for binary in &binaries {
        for path in locate_binary_all(binary) {
            let ignored = ignored_tests_in(&path);
            for (b, test) in &all {
                if b == binary && ignored.contains(test) {
                    offenders.push(format!(
                        "{} :: {test}  (decides a corpus case)",
                        path.strip_prefix(root()).unwrap_or(&path).display()
                    ));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "a corpus case is decided by a test that never runs. The case is UNCLASSIFIED, not \
         Native — an ignored test proves nothing in either direction, and its `status` keeps \
         claiming otherwise:\n  {}",
        offenders.join("\n  ")
    );
}

// ============================================================================================
// 3c. A CASE CANNOT BE DECIDED BY A TEST THAT CANNOT FAIL.
// ============================================================================================

/// The source of one top-level `fn <name>` in `src`: from its signature to the next top-level item.
///
/// Bounded by the next line that starts a top-level item at column 0 — `#[`, `fn `, or `pub ` —
/// never by brace matching: a brace counter has to lex Rust to avoid tripping over
/// `format!("{x}")`, and a lock that can be confused by a string literal is the thing this file
/// exists to prevent. All three boundaries are separately pinned by
/// `the_falsifiability_scanner_reads_one_function_and_stops_at_the_next`; a fourth rule for the
/// files' `// ====` banners was dropped because no fixture could distinguish it from the three,
/// and an untested branch in a lock is the lock's own blind spot.
fn function_source<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let sig = format!("\nfn {name}(");
    let at = src.find(&sig)? + 1;
    let rest = &src[at..];
    let mut end = rest.len();
    let mut lines = rest.split_inclusive('\n');
    // `offset` is always the byte where the line about to be examined BEGINS, so the signature
    // line must be counted before the loop starts — skipping it without counting it reports every
    // window as starting at zero.
    let mut offset = lines.next().map_or(0, str::len);
    for line in lines {
        let starts_item =
            line.starts_with("#[") || line.starts_with("fn ") || line.starts_with("pub ");
        if starts_item {
            end = offset;
            break;
        }
        offset += line.len();
    }
    Some(&rest[..end])
}

/// Anything in a test body that can actually make it fail.
const FALSIFIABLE: &[&str] = &[
    "assert!",
    "assert_eq!",
    "assert_ne!",
    ".expect(",
    ".unwrap(",
    "panic!",
    "unreachable!",
    "debug_assert",
];

/// The dual of `NATIVE-BUT-RED`, and the one the gate structurally cannot see: a case whose test
/// **passes because it asserts nothing**. The join reads `PASS` and records the capability as
/// proven; `proof_test_honesty_lock` bans three specific vacuous SHAPES but has nothing to say
/// about a decider that simply never asserts.
///
/// Measured 2026-09-15: all 248 deciders resolve to a function, and every one contains at least one
/// falsifiable construct — 0 offenders across 311 corpus test functions. So this is a trap shut
/// before anyone falls in, and it is the cheapest possible defence of a 248-case scoreboard: the
/// easiest way to turn a `Gap` into a `Native` is to write a test that cannot fail, and nothing
/// else in this campaign would notice.
#[test]
fn every_corpus_decider_contains_something_that_can_fail() {
    let mut unfalsifiable = Vec::new();
    let mut unresolved = Vec::new();
    let mut sources: BTreeMap<PathBuf, String> = BTreeMap::new();
    for (binary, test) in deciders() {
        let paths = locate_binary_all(&binary);
        let Some(path) = paths.first() else {
            unresolved.push(format!("{binary}::{test}  (no source file for the binary)"));
            continue;
        };
        let src = sources.entry(path.clone()).or_insert_with(|| read(path));
        match function_source(src, &test) {
            None => unresolved.push(format!(
                "{binary}::{test}  (no `fn {test}(` in {})",
                path.strip_prefix(root()).unwrap_or(path).display()
            )),
            Some(body) => {
                if !FALSIFIABLE.iter().any(|f| body.contains(f)) {
                    unfalsifiable.push(format!("{binary}::{test}"));
                }
            }
        }
    }
    assert!(
        unresolved.is_empty(),
        "a verdict names a decider this lock cannot find in the source. Either the test was \
         renamed and the verdict left behind, or the decider is not a plain top-level `fn` and \
         this lock needs to learn its shape — do not leave it unchecked:\n  {}",
        unresolved.join("\n  ")
    );
    assert!(
        unfalsifiable.is_empty(),
        "a corpus case is decided by a test with no assertion, no `expect`, no `unwrap` and no \
         `panic!` — it passes unconditionally, so recording its case as `Native` proves nothing. \
         This is the dual of NATIVE-BUT-RED and the one the verdict gate cannot see: the join \
         reads PASS and believes it:\n  {}",
        unfalsifiable.join("\n  ")
    );
}

// ============================================================================================
// 4a-bis. THE GATE SHARES A LOG WITH THE BASELINE SUMMARY — and must not speak its language.
// ============================================================================================

/// PASS 3 of `run-all-tests-fast.sh` tees into the SAME log `scripts/lib/nextest-summary.sh`
/// slices into passes, and that slicer's section terminator (`^ PASS [12]|^ PASS 1b`) does not
/// match " PASS 3" — so everything this gate prints lands inside the DOCTEST section.
///
/// Measured, not reasoned about, by running the summarizer over hand-cut logs:
///
/// * a `^error: could not compile …` there changes nothing — `build_failed_in` is only consulted
///   for the two passes that have a `<FAILED TO BUILD>` sentinel row, and the doctest row has none;
/// * a `test result:` line there is counted AS DOCTESTS — one planted
///   `test result: FAILED. 0 passed; 99 failed;` produced ` doctests : 7 passed, 99 failed`;
/// * a `FAIL [` row there is listed under FAILED TESTS and then trips the RECONCILIATION warning.
///
/// The last two are impossible today only because the gate sends every per-test row to a JSON file
/// instead of the terminal. That is load-bearing, so it is pinned: delete a flag and this is red,
/// instead of the baseline quietly reporting doctest failures that never happened.
#[test]
fn the_verdict_gate_never_writes_test_rows_into_the_shared_log() {
    let script = read(&gate_path());
    for flag in [
        "--status-level none",
        "--final-status-level none",
        "--message-format libtest-json-plus",
    ] {
        assert!(
            script.contains(flag),
            "the verdict gate must keep `{flag}` on its `cargo nextest run`. Without it nextest \
             prints human per-test rows, PASS 3 tees them into the baseline log, and the shared \
             summarizer counts them as DOCTESTS — measured: 99 doctest failures that do not exist."
        );
    }
    // And it must not reach for a plain `cargo test`, whose `test result:` line is exactly the
    // shape the doctest counter consumes.
    for (i, line) in script.lines().enumerate() {
        let code = line.split('#').next().unwrap_or(line);
        assert!(
            !code.contains("cargo test "),
            "line {} runs `cargo test`, whose `test result:` line the baseline summarizer counts \
             as doctests: {}",
            i + 1,
            line.trim()
        );
    }
}

// ============================================================================================
// 4c. A FINDING MUST BE ABLE TO FAIL THE GATE BY ITSELF.
// ============================================================================================

/// The text between two markers, or `None`.
fn between<'a>(hay: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = hay.find(start)? + start.len();
    let j = hay[i..].find(end)? + i;
    Some(&hay[i..j])
}

/// Every `len(<name>)` in a slice of the gate's Python, in order.
fn len_calls(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = src;
    while let Some(i) = rest.find("len(") {
        let tail = &rest[i + 4..];
        let name: String = tail
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            out.insert(name);
        }
        rest = tail;
    }
    out
}

/// A gate that PRINTS a finding and exits 0 is a gate that reports and does not gate — and that is
/// not hypothetical: measured in wave 2, dropping `len(malformed)` from the exit total left the
/// whole self-test green, because the dishonest fixture's other nine findings carried the exit code
/// on their own. Only a scoreboard whose ONLY lie is the new one can catch it dynamically, and only
/// a reader who remembers to write that fixture gets it.
///
/// So pin it structurally as well: every finding the SUMMARY line counts must also appear in the
/// expression that decides `VERDICT: OK`. Add a counter to the summary and forget the exit total,
/// and this is red before anyone has to notice the gate went quiet.
#[test]
fn every_finding_the_summary_counts_also_decides_the_verdict() {
    let script = read(&gate_path());
    let args = between(&script, "%d floor-above-truth\"", "print(\"VERDICT:")
        .expect("the SUMMARY printf's argument tuple, up to the VERDICT line");
    let total =
        between(&script, "bad_total = (", "print(\"VERDICT:").expect("the bad_total expression");

    let counted = len_calls(args);
    let deciding = len_calls(total);
    assert!(
        !counted.is_empty(),
        "no `len(...)` findings were found in the summary arguments — this lock has stopped \
         reading the gate: {args:?}"
    );
    let orphans: Vec<&String> = counted.difference(&deciding).collect();
    assert!(
        orphans.is_empty(),
        "the gate COUNTS {orphans:?} in its summary line but does not let them reach the exit \
         total, so a scoreboard whose only lie is one of those prints the finding and still exits \
         VERDICT: OK. A finding that cannot fail the gate by itself is decoration.\n  summary \
         args: {args}\n  exit total: {total}"
    );

    // …and the `counts` dict must be folded WHOLE, not cherry-picked, so a new category added to
    // `order` cannot be reported-but-not-counted either.
    assert!(
        total.contains("sum(v for k, v in counts.items()"),
        "the exit total must fold the whole `counts` dict, not selected keys: {total}"
    );
}

// ============================================================================================
// 5. THE LIVE JOIN — the real thing, on demand.
// ============================================================================================

/// Shells out to the gate, which rebuilds and runs all 16 corpus binaries. Excluded from the
/// baseline (`HEAVY_ONDEMAND_EXCLUDE` in run-all-tests-fast.sh) for the same reason
/// `marketing_test_count_floor_is_live` is: the baseline already runs the script DIRECTLY as its
/// own pass, so this would be a nested cargo invocation duplicating work the run just did. It is
/// here so `cargo nextest run -p logicaffeine-tests -- --ignored` can check the scoreboard without
/// anyone having to remember the script's name.
#[test]
#[ignore = "shells out to scripts/lean/conformance-verify-verdicts.sh, which rebuilds and runs the whole \
            corpus; the fast-suite run covers it as its own pass"]
fn every_verdict_agrees_with_reality() {
    let out = Command::new("bash")
        .arg(gate_path())
        .current_dir(root())
        .output()
        .expect("run the verdict gate");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "the Lean-parity scoreboard disagrees with what the corpus actually does:\n{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ============================================================================================
// GUARD SELF-TESTS — every predicate above is fed a synthetic violation and must fire.
// ============================================================================================

#[cfg(test)]
mod guard_selftests {
    use super::*;

    #[test]
    fn the_wiring_guard_fires_on_a_baseline_that_dropped_the_gate() {
        let wired = "cargo nextest run ...\nbash \"$SCRIPT_DIR/conformance-verify-verdicts.sh\"\n\
                     VERDICTS_EXIT=$?\nEXIT_CODE=\"$VERDICTS_EXIT\"\n";
        let unwired = "cargo nextest run ...\ncargo test --doc\n";
        let mentioned_only = "# see scripts/lean/conformance-verify-verdicts.sh\nVERDICTS_EXIT=0\n";
        assert!(baseline_invokes_the_gate(wired));
        assert!(baseline_folds_in_the_gates_exit(wired));
        assert!(
            !baseline_invokes_the_gate(unwired),
            "a baseline that never names the gate must be reported as unwired"
        );
        assert!(
            !baseline_folds_in_the_gates_exit(mentioned_only),
            "capturing a status without letting it reach EXIT_CODE is not folding it in"
        );
    }

    #[test]
    fn the_stale_result_guard_fires_on_a_committed_results_file() {
        assert!(reads_a_recorded_result(
            r#"const R: &str = include_str!("../../../logs/latest-verdicts.json");"#
        ));
        assert!(reads_a_recorded_result(
            r#"fs::read_to_string(root().join("logs/verdict-results.json"))"#
        ));
        assert!(reads_a_recorded_result(
            r#"let f = File::open("logs/conformance-verdicts-20260915-010101.json");"#
        ));
        // The SOURCE verdicts are the claim under review, not a recorded measurement.
        // `lean_parity_matrix.rs` reads them on purpose and must never be flagged.
        assert!(
            !reads_a_recorded_result(
                r#"const MANIFEST_SRC: &str = include_str!("fixtures/lean_conformance/manifest.json");"#
            ),
            "the corpus manifest is an input, not a recorded result"
        );
        assert!(
            !reads_a_recorded_result(r#"fs::read_to_string(verdict_dir().join("t00_terms.json"))"#),
            "the hand-written verdict files are the claim being checked, not a measurement of it"
        );
        // Naming an artifact without reading it is not a violation.
        assert!(!reads_a_recorded_result(
            r#"eprintln!("wrote logs/latest-verdicts.json");"#
        ));
    }

    #[test]
    fn the_falsifiability_scanner_reads_one_function_and_stops_at_the_next() {
        // A raw string, so what this fixture says is exactly what the scanner reads: the items
        // after `empty_test` MUST sit at column 0, because that is the boundary the window stops
        // at. An escaped literal with line continuations silently indents them and the fixture
        // stops testing the thing it names — measured, that is what the first version of this
        // fixture did.
        //
        // The three boundaries are here on purpose and in this order, so that dropping ANY ONE of
        // them is observable: `pub fn helper_pub` stops `empty_test`'s window before an `.unwrap()`,
        // `fn helper_plain` stops it before an `assert_eq!`, and `#[test]` stops `helper_plain`'s
        // window before `assert!(ok)`. Mutation-tested: a scanner that stops at only one of the
        // three fails at least one of the assertions below.
        const SRC: &str = r##"
fn empty_test() {
    let _ = compile("x");
}

pub fn helper_pub() -> String {
    std::env::var("X").unwrap()
}

fn helper_plain() {
    assert_eq!(1, 1);
}

#[test]
fn real_test() {
    assert!(ok);
}
"##;
        let empty = function_source(SRC, "empty_test").expect("found");
        assert!(
            empty.contains("compile("),
            "the window must contain its OWN body: {empty:?}"
        );
        assert!(
            !FALSIFIABLE.iter().any(|f| empty.contains(f)),
            "the window must STOP at the next top-level item, or a test looks falsifiable because \
             its NEIGHBOUR is — which is how a test that asserts nothing reads as one that does: \
             {empty:?}"
        );

        let real = function_source(SRC, "real_test").expect("found");
        assert!(
            real.contains("assert!("),
            "…and a real one must be read whole: {real:?}"
        );

        // Stops at an ATTRIBUTE, not only at the `fn` under it.
        let plain = function_source(SRC, "helper_plain").expect("found");
        assert!(plain.contains("assert_eq!"), "its own body: {plain:?}");
        assert!(
            !plain.contains("assert!(ok)"),
            "a window must close at the `#[test]` that opens the next item, not run through it: \
             {plain:?}"
        );

        // A `pub fn` is not a corpus decider and must not resolve at all — the signature is
        // anchored to the start of a line, so `pub fn helper_pub` is invisible to it.
        assert!(
            function_source(SRC, "helper_pub").is_none(),
            "the resolver must anchor at column 0; a `pub fn` is not a `#[test] fn`"
        );
        assert!(
            function_source(SRC, "no_such_fn").is_none(),
            "an absent fn resolves to None"
        );
    }

    #[test]
    fn the_ignore_scanner_finds_an_ignored_test_and_only_that() {
        // Raw string again, and for the same reason: the attributes have to be at column 0 and the
        // `= "a stopwatch"` has to survive verbatim. Every line here is load-bearing and was chosen
        // by mutation: the comment between `#[ignore]` and its `fn` fails a scanner that does not
        // strip comments; `const BANNED_ATTRIBUTE` fails one that looks for the attribute text
        // ANYWHERE in a line instead of anchoring it; and `only_mentions_it_in_a_comment` fails one
        // that reads a commented-out attribute as a live one.
        const FIXTURE: &str = r##"
#[test]
fn runs_normally() {}

#[test]
#[ignore = "a stopwatch"]
fn is_ignored_with_a_reason() {}

#[ignore]
// a comment sitting between the attribute and its function
#[test]
fn is_ignored_bare() {}

// #[ignore]
#[test]
fn only_mentions_it_in_a_comment() {}

const BANNED_ATTRIBUTE: &str = "#[ignore]";

#[test]
fn comes_after_a_line_that_merely_names_it() {}
"##;
        let path = scratch::dir("H_honesty_ignore_selftest").join("fixture.rs");
        fs::write(&path, FIXTURE).expect("write fixture");
        let found = ignored_tests_in(&path);
        let _ = fs::remove_file(&path);

        assert!(
            found.contains("is_ignored_with_a_reason"),
            "the message form: {found:?}"
        );
        assert!(
            found.contains("is_ignored_bare"),
            "the bare form, attribute first, with a COMMENT between it and its fn: {found:?}"
        );
        assert!(
            !found.contains("runs_normally"),
            "a plain test is not ignored: {found:?}"
        );
        assert!(
            !found.contains("only_mentions_it_in_a_comment"),
            "a commented-out attribute is not an attribute: {found:?}"
        );
        assert!(
            !found.contains("comes_after_a_line_that_merely_names_it"),
            "a line that merely CONTAINS the attribute's text — a const, or a test asserting on \
             generated source — must not silence the next function: {found:?}"
        );
        assert_eq!(found.len(), 2, "exactly the two real ones: {found:?}");
    }

    #[test]
    fn the_resolver_reads_a_qualified_test_name_as_another_binary() {
        // Pinned against the live corpus: T11-021's decider lives in `automation_extra`, not in
        // its tier's `lean_conformance_t11_adversarial`. A resolver that trusts `test_binary` alone
        // reports it, and ten others like it, as tests that do not exist.
        let named = binaries_a_verdict_names();
        assert!(
            named.contains("automation_extra"),
            "a qualified `binary::test` verdict must contribute ITS binary, not its tier's: {named:?}"
        );
        assert!(
            named.contains("lean_conformance_t11_adversarial"),
            "the tier's own binary must still be discovered: {named:?}"
        );
    }
}

// ============================================================================================
// 6. THE SECTION BOUNDARY — a pass the slicer cannot see is a pass whose output is attributed
//    to the pass before it.
// ============================================================================================

/// `scripts/lib/nextest-summary.sh` slices the shared baseline log into passes, and a section runs
/// until the next line that opens one. Wave 2 measured what happens when a pass's marker is not in
/// that terminator: ` PASS 3` was not, so everything the verdict gate printed landed inside the
/// DOCTEST section and was counted as doctests — a planted
/// `test result: FAILED. 0 passed; 99 failed;` produced ` doctests : 7 passed, 99 failed` and a
/// totals line reading `99 failed` for a run in which nothing failed.
///
/// `the_verdict_gate_never_writes_test_rows_into_the_shared_log` pins the flags that keep those
/// lines out of the log today. This pins the STRUCTURE instead, and it is the stronger of the two:
/// the summarizer is fed a hand-cut log in which PASS 3 speaks the doctest counter's own language,
/// and the doctest row must still report the three doctests that really ran. A reporter whose
/// arithmetic depends on a downstream pass staying quiet is a reporter that will be wrong the first
/// time it does not.
///
/// Written against the REAL summarizer over a temp file, never against the script's text: a grep
/// for `PASS [123]` would go green on a terminator that had been broken in some other way.
#[test]
fn a_later_pass_cannot_be_counted_as_doctests() {
    let log = scratch::dir("H_honesty_section_boundary").join("summary.log");
    // Built line-by-line and joined, never as one `\`-continued literal: a backslash-newline in a
    // Rust string swallows the following line's LEADING WHITESPACE, and every marker here begins
    // with exactly one space. A fixture that quietly loses it tests nothing — measured, on the
    // first draft of this test, which reported both nextest passes as "did not finish".
    let fixture = [
        " PASS 1/2 — cargo nextest run (unit + integration, ignored included)",
        "     Summary [   1.000s] 3 tests run: 3 passed, 0 skipped",
        " PASS 1b — WASM JIT tier + AOT backend + parity lock",
        "     Summary [   0.500s] 2 tests run: 2 passed, 0 skipped",
        " PASS 2/2 — cargo test --doc (doctests)",
        "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out",
        " PASS 3 — Lean-parity verdicts vs reality (scripts/lean/conformance-verify-verdicts.sh)",
        "test result: FAILED. 0 passed; 99 failed; 0 ignored; 0 measured; 0 filtered out",
        "    FAIL [   0.010s] (    1/248) logicaffeine-kernel::lean_conformance_t00_terms a_planted_row",
        "SUMMARY: 0 native-red, 0 gap-green, 0 refused-red, 0 missing, 0 ignored, 0 not-observed, \
0 shared-decider, 0 wrong-tier, 0 ambiguous, 0 malformed, 0 floor-above-truth",
        "VERDICT: OK",
        "",
    ]
    .join("\n");
    assert!(
        fixture.contains("\n PASS 3 —"),
        "the fixture must keep the single leading space that makes a pass header a pass header"
    );
    fs::write(&log, &fixture).expect("write the fixture log");

    let summarizer = root().join("scripts/lib/nextest-summary.sh");
    let out = Command::new("bash")
        .arg(&summarizer)
        .arg(&log)
        .output()
        .expect("run the summarizer");
    let s = String::from_utf8_lossy(&out.stdout).into_owned();
    let _ = fs::remove_file(&log);

    assert!(
        s.contains(" doctests : 3 passed, 0 failed, 0 ignored"),
        "PASS 3's own `test result:` line must not be counted as doctests. The doctest pass ran \
         THREE doctests and none failed; anything else means the section terminator does not \
         recognise the marker that opens PASS 3, so the doctest section runs to EOF:\n{s}"
    );
    assert!(
        s.contains(" totals   : 8 tests run, 0 failed, 0 timed out, 0 flaky"),
        "…and the totals must stay 3 + 2 + 3 with nothing failing:\n{s}"
    );
    assert!(
        !s.contains("RECONCILIATION"),
        "a green run must not raise the reconciliation warning — a `FAIL [` row printed by a pass \
         the slicer cannot see is scraped as a failure the nextest Summary lines never counted, \
         which is the warning firing on the reporter's own blind spot:\n{s}"
    );
    assert!(
        !s.contains("a_planted_row"),
        "…and a row belonging to a later pass must not be listed under FAILED TESTS:\n{s}"
    );
}

/// The terminator must recognise every pass marker the gate script actually writes, and this is
/// the half a fixture cannot cover: a pass added tomorrow gets no fixture until someone remembers
/// to write one, and the failure mode is silent misattribution rather than a crash.
///
/// So the two files are joined directly — every ` PASS <n>` header `run-all-tests-fast.sh` prints
/// must be matched by the summarizer's own terminator, evaluated by `awk` rather than by a regex
/// this test re-implements. Add " PASS 4" to the gate and forget the slicer, and this is red before
/// the first misattributed number reaches anyone.
#[test]
fn the_slicer_recognises_every_pass_marker_the_gate_prints() {
    let gate = read(&root().join("scripts/run-all-tests-fast.sh"));
    let summarizer_src = read(&root().join("scripts/lib/nextest-summary.sh"));

    let terminator = summarizer_src
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("armed &&")
                .map(|r| r.trim().to_string())
        })
        .expect("the section() awk program must carry an `armed && /…/ { exit }` terminator");
    let pattern = between(&terminator, "/", "/")
        .expect("a /…/ regex in the terminator")
        .to_string();

    // Every ` PASS …` header the gate echoes, as the LOG will carry it.
    let markers: BTreeSet<String> = gate
        .lines()
        .filter_map(|l| {
            let t = l.trim_start();
            let rest = t.strip_prefix("echo \"")?;
            rest.strip_prefix(" PASS ").map(|_| {
                let body = rest.trim_end_matches('"');
                format!(" PASS {}", body.trim_start_matches(" PASS "))
            })
        })
        .collect();
    assert!(
        markers.len() >= 4,
        "expected the gate to print at least four ` PASS …` headers; found {markers:?}"
    );

    let mut unseen = Vec::new();
    for m in &markers {
        let matched = Command::new("awk")
            .arg(format!("/{pattern}/ {{ print \"HIT\" }}"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin
                    .as_mut()
                    .expect("stdin")
                    .write_all(format!("{m}\n").as_bytes())?;
                c.wait_with_output()
            })
            .expect("run awk");
        if !String::from_utf8_lossy(&matched.stdout).contains("HIT") {
            unseen.push(m.clone());
        }
    }
    assert!(
        unseen.is_empty(),
        "scripts/lib/nextest-summary.sh's section terminator (/{pattern}/) does not match a pass \
         header scripts/run-all-tests-fast.sh actually prints. Everything that pass writes is then \
         attributed to the pass BEFORE it — measured once already, as 99 doctest failures that did \
         not exist. Unmatched: {unseen:?}"
    );
}

// ============================================================================================
// 7. THE ASSERTION A VACUOUS ONE STOOD IN FOR — measured here so a handoff is a fact, not a guess.
// ============================================================================================

/// `phase_hw_cdc::cdc_report_domains_listed` asserts
/// `report.crossings.len() + report.violations.len() >= 0`, which is true for every possible input
/// because both are `usize`. Its NAME promises the report lists the domains. Rule 4 forbids this
/// lane editing that assertion, and wave 2 correctly refused to guess a replacement, noting that
/// `CdcReport` has no `domains` field and that settling it "needs a run of `analyze_cdc`".
///
/// This is that run, on exactly that input, and it is a real test rather than a probe: it asserts
/// what the report actually says about the domains a crossing runs between. Whoever owns
/// `phase_hw_cdc.rs` can paste the body in with confidence, and until they do, the behaviour that
/// file's test only pretended to cover is covered here.
///
/// Traced through `analyze_cdc` before running it, so a disagreement is a finding either way: with
/// domains `a`/`b` and one signal `a_x`, `is_in_domain` matches the `a_` prefix, `detect_pattern`
/// finds no `a_x_sync1`, no `req`, no `gray` and no `fifo` and returns `None`, so the crossing is
/// unsafe and raises exactly one `MissingSynchronizer` violation.
#[test]
fn the_cdc_report_names_the_domains_a_crossing_runs_between() {
    use logicaffeine_compile::codegen_sva::cdc::*;
    use logicaffeine_compile::codegen_sva::rtl_extract::{RtlModule, RtlSignal, SignalType};

    let rtl = RtlModule {
        name: "test_module".into(),
        ports: vec![],
        signals: vec![RtlSignal {
            name: "a_x".into(),
            signal_type: SignalType::Wire,
            width: 1,
        }],
        params: vec![],
        clocks: vec!["clk_a".into(), "clk_b".into()],
    };
    let domains = vec![
        CdcClockDomain {
            name: "a".into(),
            clock_signal: "clk_a".into(),
        },
        CdcClockDomain {
            name: "b".into(),
            clock_signal: "clk_b".into(),
        },
    ];
    let report = analyze_cdc(&rtl, &domains);

    let crossing = report
        .crossings
        .iter()
        .find(|c| c.signal == "a_x")
        .unwrap_or_else(|| panic!("`a_x` crosses clk_a to clk_b and must be reported: {report:?}"));
    assert_eq!(
        (
            crossing.source_domain.as_str(),
            crossing.dest_domain.as_str()
        ),
        ("a", "b"),
        "the report must name the domains a crossing runs between: {report:?}"
    );
    assert!(
        !crossing.safe && crossing.pattern.is_none(),
        "`a_x` has no synchronizer sibling, so the crossing is unsafe and carries no pattern: {crossing:?}"
    );
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.signal == "a_x" && v.violation_type == CdcViolationType::MissingSynchronizer),
        "an unsafe crossing must raise MissingSynchronizer: {report:?}"
    );
    // The negative rung: a report is not a bag that says yes to everything. Nothing crosses that
    // was not asked about, and a single-bit signal earns no gray-code finding.
    assert!(
        report.crossings.iter().all(|c| c.signal == "a_x"),
        "only the signals actually present may appear as crossings: {report:?}"
    );
    assert!(
        report.patterns.is_empty(),
        "no synchronizer pattern exists here: {report:?}"
    );
}

// ============================================================================================
// 8. THE TWO RATCHETS MUST NOT BE ABLE TO CANCEL EACH OTHER OUT.
// ============================================================================================

/// Every file path either vacuity ledger in `proof_test_honesty_lock.rs` names.
///
/// Read as TEXT, because the ledgers are `const`s in a different test binary and there is no way to
/// import them. The alternative — duplicating the list here — is two lists that drift, which is the
/// failure mode this whole file exists to prevent.
/// A path under the repo root, spelled the way the vacuity ledgers spell one. `root()` is
/// `CARGO_MANIFEST_DIR/../..`, so a joined path carries `crates/logicaffeine_tests/../..` in the
/// middle and `strip_prefix` alone will not remove it; both sides are canonicalized first.
fn repo_relative(p: &Path) -> String {
    let base = root().canonicalize().unwrap_or_else(|_| root());
    let abs = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    abs.strip_prefix(&base)
        .unwrap_or(&abs)
        .to_string_lossy()
        .replace('\\', "/")
}

fn vacuity_ledger_files() -> BTreeSet<String> {
    let src = read(&root().join("crates/logicaffeine_tests/tests/proof_test_honesty_lock.rs"));
    let mut out = BTreeSet::new();
    for (start, end) in [
        ("const GRANDFATHERED", "];"),
        ("const SURFACED_BY_A_WIDER_DETECTOR", "];"),
    ] {
        let Some(block) = between(&src, start, end) else {
            continue;
        };
        let mut rest = block;
        while let Some(i) = rest.find('"') {
            let tail = &rest[i + 1..];
            let Some(j) = tail.find('"') else { break };
            let lit = &tail[..j];
            if (lit.starts_with("crates/") || lit.starts_with("apps/")) && lit.ends_with(".rs") {
                out.insert(lit.to_string());
            }
            rest = &tail[j + 1..];
        }
    }
    assert!(
        !out.is_empty(),
        "the ledgers were parsed and yielded no file at all — the shape changed and this lock is \
         now checking nothing, which is worse than the hole it guards"
    );
    out
}

/// **A corpus decider may never live in a file that is allowed to hold a vacuous assertion.**
///
/// This is the one way the two ratchets in this campaign can cancel each other out, and it would be
/// silent. `proof_test_honesty_lock` permits a known vacuous site in a listed FILE; the verdict gate
/// reads `PASS` and records a capability as proven. Put those together in one file and a `Native`
/// claim can rest on an assertion that cannot fail — a capability we would assert in a head-to-head
/// against Lean and be shown not to have, with two green ratchets and a green scoreboard above it.
///
/// Neither lock can see this on its own: the vacuity ledger does not know what a decider is, and the
/// gate does not know what a vacuous assertion is. The join is the only place it is visible.
///
/// Measured 2026-09-15: zero overlap. All twelve ledgered files are `phase*`/`analysis` files and no
/// corpus case is decided in any of them, so this is a trap shut before anyone falls into it — which
/// is exactly when a trap is cheap to shut.
#[test]
fn no_corpus_decider_lives_in_a_file_that_may_hold_a_vacuous_assertion() {
    let ledgered = vacuity_ledger_files();
    let mut overlap = Vec::new();
    for (binary, test) in deciders() {
        for path in locate_binary_all(&binary) {
            // `locate_binary_all` returns paths built from `root()`, which is ABSOLUTE. The ledgers
            // spell a path relative to the repo root. Comparing the two directly is a test that is
            // always false and therefore always green — caught here before it shipped, which is the
            // entire failure mode `the_ledger_reader_finds_the_files…` below exists to re-check.
            let rel = repo_relative(&path);
            if ledgered.contains(&rel) {
                overlap.push(format!("{rel} decides {binary}::{test}"));
            }
        }
    }
    assert!(
        overlap.is_empty(),
        "a corpus case is decided by a test in a file the anti-vacuity ratchet has granted an \
         exception to. A `Native` verdict there can be satisfied by an assertion that cannot fail, \
         and every ratchet involved stays green while it happens. Either fix the vacuous site or \
         move the decider:\n  {}",
        overlap.join("\n  ")
    );
}

/// …and the guard itself must be capable of seeing an overlap, or the test above is a green light
/// over an unread list. The ledger parser is fed the real file and must come back with the files
/// that are really in it, spelled the way `locate_binary_all` spells a path.
#[test]
fn the_ledger_reader_finds_the_files_and_spells_them_like_the_resolver() {
    let ledgered = vacuity_ledger_files();
    assert!(
        ledgered.len() >= 10,
        "the two ledgers name a dozen files between them; the parser found {}: {ledgered:?}",
        ledgered.len()
    );
    assert!(
        ledgered.contains("crates/logicaffeine_tests/tests/phase_hw_cdc.rs"),
        "the surfaced ledger's entries must be found too, not just the grandfather list: {ledgered:?}"
    );
    for f in &ledgered {
        let joined = root().join(f);
        assert!(
            joined.exists(),
            "a ledgered path must resolve from the repo root: {f}"
        );
        // …and the two spellings must MEET. This is the assertion that would have caught the
        // absolute-vs-relative mismatch above: without it, the overlap test compares two strings
        // that can never be equal and reports "no overlap" forever.
        assert_eq!(
            repo_relative(&joined),
            *f,
            "a ledgered path must round-trip through the same normalisation the decider resolver \
             goes through, or the overlap test above is always false and always green"
        );
    }
    // The join must be live at both ends: a decider resolver that returns nothing would make the
    // overlap test green for the other reason.
    let resolved: usize = deciders()
        .iter()
        .map(|(b, _)| locate_binary_all(b).len())
        .sum();
    assert!(
        resolved >= 240,
        "nearly every one of the 248 corpus cases must resolve to a source file for the overlap \
         test to mean anything — a resolver that returns nothing makes it green for the wrong \
         reason. Deliberately not pinned at exactly 248: two cases sharing one decider is a \
         separate finding the gate reports as SHARED-DECIDER, and it must not break this. \
         {resolved} resolved"
    );
}
