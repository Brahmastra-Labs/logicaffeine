//! Every test the baseline EXCLUDES must have an on-demand home — and the two lists must agree.
//!
//! `scripts/run-all-tests-fast.sh` deliberately drops classes of test from the routine baseline so a
//! normal run stays bounded — the SAT/coNP research campaign (`RESEARCH_EXCLUDE`), the gate-level
//! silicon sims and the nested scoreboard run (`HEAVY_ONDEMAND_EXCLUDE`), the kernel importer specs
//! (`KERNEL_PENDING_EXCLUDE`), the qemu corpus differentials (`QEMU_CORPUS_EXCLUDE`), the whole-Mathlib
//! Arena checks (`ARENA_MATHLIB_EXCLUDE`) and the decode engine's real-checkpoint bars (`MODEL_BARS_EXCLUDE`) —
//! and each class has one companion runner that runs exactly those tests on purpose. One more runner, `scripts/run-opt-in-feature-tests.sh`, owns the suites behind the
//! features the baseline never turns on; `feature_coverage_lock` holds it to the gates it must reach. And
//! `scripts/run-handed-reds-tests.sh` runs the RED suites handed out with the fusion book's tickets, held here to exactly
//! the suites behind `handed-reds`.
//!
//! WHY THIS LOCK EXISTS. Both scripts carried a comment saying the two lists "MUST stay in sync",
//! and nothing enforced it. That is the one failure mode an exclusion can have that is worse than
//! a slow suite: drop a binary from the baseline, forget it in the runner, and the test is DARK —
//! it never runs anywhere, forever, and the suite still reports green. The exclusion is a routing
//! decision, so the invariant to pin is that it routes rather than deletes: whatever the baseline
//! drops, some on-demand runner must pick up, and whatever a runner claims to own, the baseline
//! must actually be dropping. This is the same shape as `wasm_gated_suite_lock`, which stops a
//! `wasm-jit`-gated binary from silently never running.
//!
//! It is a text lock over the scripts and the test sources — no cargo, no build — so it runs in every
//! baseline and catches the drift at the moment someone edits an exclusion or a runner.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Every on-demand runner with a nextest selector, and the shell variable that holds it.
const SELECTOR_RUNNERS: [(&str, &str); 7] = [
    ("run-research-tests.sh", "RESEARCH_ONLY"),
    ("run-opt-audit-tests.sh", "OPT_AUDIT_ONLY"),
    ("run-silicon-tests.sh", "SILICON_ONLY"),
    ("run-kernel-pending-tests.sh", "KERNEL_PENDING_ONLY"),
    ("run-qemu-corpus-tests.sh", "QEMU_CORPUS_ONLY"),
    ("run-arena-mathlib-tests.sh", "ARENA_MATHLIB_ONLY"),
    ("run-model-bars-tests.sh", "MODEL_BARS_ONLY"),
];

/// The on-demand runner for the suites behind the features the baseline leaves off. It selects by
/// feature and `--test`, not by filterset; `feature_coverage_lock` reads its invocations.
const OPT_IN_RUNNER: &str = "run-opt-in-feature-tests.sh";

/// The on-demand runner for the RED suites handed out with the fusion book's tickets, each behind
/// `#![cfg(feature = "handed-reds")]` until its ticket lands. It selects by `--test`, from its `BINARIES`.
const HANDED_REDS_RUNNER: &str = "run-handed-reds-tests.sh";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn script(name: &str) -> String {
    let p = repo_root().join("scripts").join(name);
    assert!(p.exists(), "missing {}", p.display());
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// The single-quoted value of a top-level shell assignment `NAME='…'`.
fn shell_var(src: &str, var: &str) -> String {
    let needle = format!("\n{var}='");
    let start = src
        .find(&needle)
        .unwrap_or_else(|| panic!("`{var}` is not assigned in the script — did it get renamed?"))
        + needle.len();
    let rest = &src[start..];
    let end = rest
        .find('\'')
        .unwrap_or_else(|| panic!("`{var}` has no closing quote"));
    rest[..end].to_string()
}

/// The binary names inside every `binary(/^(a|b|c)$/)` alternation of a nextest filterset.
fn binary_alternation(filterset: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = filterset;
    while let Some(i) = rest.find("binary(/^(") {
        let after = &rest[i + "binary(/^(".len()..];
        let Some(end) = after.find(")$/)") else { break };
        for name in after[..end].split('|') {
            let name = name.trim();
            if !name.is_empty() {
                out.insert(name.to_string());
            }
        }
        rest = &after[end..];
    }
    out
}

#[test]
fn the_research_campaign_the_baseline_drops_is_exactly_what_its_runner_picks_up() {
    let excluded = binary_alternation(&shell_var(
        &script("run-all-tests-fast.sh"),
        "RESEARCH_EXCLUDE",
    ));
    let run = binary_alternation(&shell_var(
        &script("run-research-tests.sh"),
        "RESEARCH_ONLY",
    ));

    assert!(
        !excluded.is_empty(),
        "RESEARCH_EXCLUDE parsed to nothing — the parser or the format changed"
    );

    let dark: Vec<_> = excluded.difference(&run).collect();
    assert!(
        dark.is_empty(),
        "these binaries are excluded from the baseline but NOT run by scripts/run-research-tests.sh, \
         so they run NOWHERE: {dark:?}"
    );
    let phantom: Vec<_> = run.difference(&excluded).collect();
    assert!(
        phantom.is_empty(),
        "scripts/run-research-tests.sh claims these binaries but the baseline does not exclude them, \
         so they run TWICE (and the slow one gates the baseline): {phantom:?}"
    );
}

#[test]
fn the_silicon_sims_the_baseline_drops_are_exactly_what_the_silicon_runner_picks_up() {
    let heavy = shell_var(&script("run-all-tests-fast.sh"), "HEAVY_ONDEMAND_EXCLUDE");
    let silicon = shell_var(&script("run-silicon-tests.sh"), "SILICON_ONLY");

    // The runner's selector is the exact predicate the baseline negates, so neither can drift
    // without the other: `not (<selector>)` in one file, `<selector>` in the other.
    assert!(
        heavy.contains(&format!("not ({silicon})")),
        "HEAVY_ONDEMAND_EXCLUDE must drop exactly what run-silicon-tests.sh runs.\n  \
         baseline drops : {heavy}\n  runner runs    : {silicon}"
    );
}

#[test]
fn the_kernel_pending_specs_the_baseline_drops_are_exactly_what_their_runner_picks_up() {
    let excluded = shell_var(&script("run-all-tests-fast.sh"), "KERNEL_PENDING_EXCLUDE");
    let only = shell_var(
        &script("run-kernel-pending-tests.sh"),
        "KERNEL_PENDING_ONLY",
    );

    // Same shape as the silicon pair: the runner's selector is the exact predicate the baseline
    // negates. These are kernel NORTH-STAR specs for a capability the importer does not have yet
    // (the app-lam original published without expanding its captured source); they run on purpose.
    assert!(
        excluded.contains(&format!("not ({only})")),
        "KERNEL_PENDING_EXCLUDE must drop exactly what run-kernel-pending-tests.sh runs.\n  \
         baseline drops : {excluded}\n  runner runs    : {only}"
    );
}

/// The kernel-pending runner's gap table: each spec it runs, and the stable substring of the panic that names the
/// capability that spec waits for (`KERNEL_PENDING_GAPS`, one `<spec>|<signature>` per line).
fn kernel_pending_gaps() -> BTreeMap<String, String> {
    shell_var(
        &script("run-kernel-pending-tests.sh"),
        "KERNEL_PENDING_GAPS",
    )
    .lines()
    .filter(|l| !l.trim().is_empty())
    .map(|l| {
        let (spec, signature) = l
            .split_once('|')
            .unwrap_or_else(|| panic!("KERNEL_PENDING_GAPS row `{l}` is not `<spec>|<signature>`"));
        (spec.trim().to_string(), signature.trim().to_string())
    })
    .collect()
}

/// The spec names a kernel-pending selector runs: its `test(/::(a|b)$/)` alternation.
fn kernel_pending_specs(only: &str) -> BTreeSet<String> {
    let alts = only.split("test(/::(").nth(1).and_then(|r| r.split(")$/)").next()).unwrap_or_else(|| {
        panic!("KERNEL_PENDING_ONLY is not `… test(/::(a|b)$/)`, so its specs cannot be read: {only}")
    });
    alts.split('|')
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect()
}

/// The runner's verdict is that each gap is exactly its gap, so its table must name exactly the specs its selector
/// runs: a spec with no row is judged against nothing, a row with no spec is a gap nobody checks.
#[test]
fn the_kernel_pending_gap_table_names_exactly_the_specs_its_selector_runs() {
    let only = shell_var(
        &script("run-kernel-pending-tests.sh"),
        "KERNEL_PENDING_ONLY",
    );
    let specs = kernel_pending_specs(&only);
    let table: BTreeSet<String> = kernel_pending_gaps().into_keys().collect();
    assert!(
        !specs.is_empty(),
        "KERNEL_PENDING_ONLY runs no spec: {only}"
    );
    assert_eq!(
        table, specs,
        "KERNEL_PENDING_GAPS must name exactly the specs KERNEL_PENDING_ONLY runs"
    );
}

/// Each gap signature is the start of a string literal the kernel compiles — its own sources, and the unit
/// modules its `#[path]` stubs pull in — so rewording the message that names a gap reddens here, at once, rather
/// than only when the on-demand runner next runs.
#[test]
fn every_kernel_pending_gap_signature_is_a_message_the_kernel_writes() {
    let roots = [
        repo_root().join("crates/logicaffeine_kernel/src"),
        repo_root().join("crates/logicaffeine_tests/unit/logicaffeine_kernel"),
    ];
    let mut sources = Vec::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "rs") {
                sources.push(std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    for (spec, signature) in kernel_pending_gaps() {
        let literal = format!("\"{signature}");
        assert!(
            sources.iter().any(|s| s.contains(&literal)),
            "{spec}: its gap signature `{signature}` begins no string literal in the kernel's sources, so the gap it names \
             cannot be the one the spec fails with"
        );
    }
}

/// The runner judges the report its own profile writes: nextest's JUnit for `--profile kernel-pending`, which lands
/// under the workspace's `target/nextest/kernel-pending/`, is what the verdict reads, and the verdict is the
/// runner's last act, handed cargo's exit.
#[test]
fn the_kernel_pending_runner_judges_the_report_its_profile_writes() {
    let src = script("run-kernel-pending-tests.sh");
    let exec = exec_command(&src);
    assert!(
        exec.contains("--profile kernel-pending"),
        "the runner runs the kernel-pending profile: {exec}"
    );
    let toml = std::fs::read_to_string(repo_root().join(".config/nextest.toml"))
        .expect(".config/nextest.toml");
    assert!(
        toml.contains("\n[profile.kernel-pending.junit]\npath = \"junit.xml\"\n"),
        "the kernel-pending profile writes the JUnit report its verdict reads"
    );
    assert!(
        src.contains("\nJUNIT=\"$REPO_ROOT/target/nextest/kernel-pending/junit.xml\"\nrm -f \"$JUNIT\"\n"),
        "the runner reads the report where nextest writes it, and removes a stale one before the run"
    );
    let last = src
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .unwrap_or_default();
    assert_eq!(
        last, "python3 \"$SCRIPT_DIR/test/kernel-pending-verdict.py\" \"$JUNIT\" \"$KERNEL_PENDING_GAPS\" \"$?\"",
        "the verdict is the runner's last act, handed cargo's exit"
    );
}

/// The verdict over reports of every shape it must tell apart. A green verdict means each spec FAILED with its own
/// gap; a spec that passes is the capability landing and must say to un-ignore it; any other failure, a missing
/// spec, an unknown test, a missing report and an incomplete run are red. Graded by the verdict's own lines, not
/// only its exit code.
#[test]
fn the_kernel_pending_verdict_tells_every_outcome_apart() {
    let verdict = repo_root().join("scripts/test/kernel-pending-verdict.py");
    let gaps = "spec_a|gap alpha exhausted\nspec_b|gap beta budget";
    let failing = |name: &str, err: &str| {
        format!(
            "<testcase name=\"m::{name}\" classname=\"logicaffeine-kernel\"><failure type=\"test failure with exit code \
             101\">thread panicked</failure><system-err>{err}</system-err></testcase>"
        )
    };
    let passing =
        |name: &str| format!("<testcase name=\"m::{name}\" classname=\"logicaffeine-kernel\"/>");
    let dir = tempfile::tempdir().unwrap();
    let run = |cases: &[String], cargo_exit: &str| {
        let junit = dir.path().join("junit.xml");
        std::fs::write(
            &junit,
            format!(
                "<testsuites><testsuite name=\"k\">{}</testsuite></testsuites>",
                cases.concat()
            ),
        )
        .unwrap();
        let out = std::process::Command::new("python3")
            .arg(&verdict)
            .arg(&junit)
            .arg(gaps)
            .arg(cargo_exit)
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    };
    let (a, b) = (
        failing("spec_a", "Decline(&quot;gap alpha exhausted&quot;)"),
        failing("spec_b", "gap beta budget: 769"),
    );

    let (code, out) = run(&[a.clone(), b.clone()], "100");
    assert_eq!(
        code,
        Some(0),
        "every spec failing with its gap is green:\n{out}"
    );
    assert!(out.contains("VERDICT: 2/2 gaps confirmed"), "{out}");

    let (code, out) = run(&[passing("spec_a"), b.clone()], "100");
    assert_ne!(code, Some(0), "a spec that passes is red:\n{out}");
    assert!(
        out.contains("RED spec_a: PASSES — the capability landed. Un-ignore it"),
        "{out}"
    );

    let (code, out) = run(
        &[failing("spec_a", "index out of bounds"), b.clone()],
        "100",
    );
    assert_ne!(code, Some(0), "a spec failing another way is red:\n{out}");
    assert!(
        out.contains("RED spec_a: fails, but not with its gap `gap alpha exhausted`")
            && out.contains("index out of bounds"),
        "{out}"
    );

    let (code, out) = run(&[b.clone()], "100");
    assert_ne!(code, Some(0), "a spec that did not run is red:\n{out}");
    assert!(out.contains("RED spec_a: did not run"), "{out}");

    let (code, out) = run(&[a.clone(), b.clone(), failing("spec_c", "boom")], "100");
    assert_ne!(
        code,
        Some(0),
        "a test the table does not name is red:\n{out}"
    );
    assert!(
        out.contains("RED spec_c: ran, but KERNEL_PENDING_GAPS has no row for it"),
        "{out}"
    );

    let (code, out) = run(&[a.clone(), b.clone()], "101");
    assert_eq!(
        code,
        Some(101),
        "an incomplete run keeps cargo's exit:\n{out}"
    );
    assert!(
        out.contains("RED the run did not complete: cargo nextest exited 101"),
        "{out}"
    );

    std::fs::remove_file(dir.path().join("junit.xml")).ok();
    let out = std::process::Command::new("python3")
        .arg(&verdict)
        .arg(dir.path().join("junit.xml"))
        .arg(gaps)
        .arg("100")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_ne!(out.status.code(), Some(0), "no report is red:\n{text}");
    assert!(text.contains("no JUnit report at"), "{text}");
}

#[test]
fn the_optimization_audit_the_baseline_drops_is_exactly_what_its_runner_picks_up() {
    let excluded = shell_var(&script("run-all-tests-fast.sh"), "OPT_AUDIT_EXCLUDE");
    let only = shell_var(&script("run-opt-audit-tests.sh"), "OPT_AUDIT_ONLY");
    assert!(
        excluded.contains(&format!("not ({only})")),
        "OPT_AUDIT_EXCLUDE must drop exactly what run-opt-audit-tests.sh runs.\n  \
         baseline drops : {excluded}\n  runner runs    : {only}"
    );
}

/// The clauses the baseline drops, one per `not …` term of every `*_EXCLUDE`, with the `not` and any
/// enclosing parentheses taken off: `not (a and b)` is `a and b`, `not binary(x)` is `binary(x)`.
fn excluded_clauses(gate: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in gate.lines() {
        let Some((name, _)) = line.split_once("_EXCLUDE='") else {
            continue;
        };
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
            continue;
        }
        let value = shell_var(gate, &format!("{name}_EXCLUDE"));
        let mut depth = 0i32;
        let mut start = 0;
        let bytes = value.as_bytes();
        let mut terms = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ if depth == 0 && value[i..].starts_with(" and ") => {
                    terms.push(value[start..i].to_string());
                    i += " and ".len();
                    start = i;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        terms.push(value[start..].to_string());
        for term in terms {
            let inner = term.trim().strip_prefix("not ").unwrap_or_else(|| {
                panic!("{name}_EXCLUDE has a term that is not a `not …` exclusion: `{term}`")
            });
            let inner = inner.trim();
            let unwrapped = match inner.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
                Some(body) if balanced(body) => body.to_string(),
                _ => inner.to_string(),
            };
            out.push((format!("{name}_EXCLUDE"), unwrapped));
        }
    }
    out
}

fn balanced(s: &str) -> bool {
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// The one clause the gate runs itself: `H_honesty_extra::every_verdict_agrees_with_reality` shells out
/// to the Lean verdict gate, which the fast gate's PASS 3 runs directly.
const GATE_CLAIMED: [(&str, &str); 1] = [(
    "test(every_verdict_agrees_with_reality)",
    "verify-verdicts.sh",
)];

/// Every clause the baseline drops is claimed by exactly one on-demand runner, whose selector is that
/// clause: a clause no runner claims drops tests that then run nowhere, and a clause two runners claim
/// runs its tests twice. The one exception is the clause the gate claims itself (GATE_CLAIMED), and the
/// gate must still run what claims it.
#[test]
fn every_clause_the_baseline_drops_is_claimed_by_exactly_one_runner() {
    let gate = script("run-all-tests-fast.sh");
    let clauses = excluded_clauses(&gate);
    assert!(
        clauses.len() >= 7,
        "only {} excluded clauses parsed — the parser or the gate changed",
        clauses.len()
    );
    let selectors: Vec<(&str, String)> = SELECTOR_RUNNERS
        .iter()
        .map(|(runner, var)| (*runner, shell_var(&script(runner), var)))
        .collect();
    for (class, clause) in &clauses {
        if let Some((_, claimant)) = GATE_CLAIMED.iter().find(|(c, _)| c == clause) {
            assert!(gate.contains(claimant), "{class}: `{clause}` is claimed by the gate's own `{claimant}`, which it no longer runs");
            continue;
        }
        let claimed_by: Vec<&str> = selectors
            .iter()
            .filter(|(_, only)| only == clause)
            .map(|(runner, _)| *runner)
            .collect();
        assert!(
            claimed_by.len() == 1,
            "{class} drops `{clause}`, claimed by {} runner(s) {claimed_by:?}: every clause the baseline drops is \
             claimed by exactly one runner whose selector is that clause",
            claimed_by.len()
        );
    }
    for (runner, only) in &selectors {
        let dropping: Vec<&str> = clauses
            .iter()
            .filter(|(_, c)| c == only)
            .map(|(class, _)| class.as_str())
            .collect();
        assert!(
            dropping.len() == 1,
            "{runner} selects `{only}`, which {} baseline clause(s) {dropping:?} drop: each runner's tests are \
             dropped from the baseline exactly once",
            dropping.len()
        );
    }
}

#[test]
fn the_qemu_corpus_differentials_the_baseline_drops_are_exactly_what_their_runner_picks_up() {
    let excluded = shell_var(&script("run-all-tests-fast.sh"), "QEMU_CORPUS_EXCLUDE");
    let only = shell_var(&script("run-qemu-corpus-tests.sh"), "QEMU_CORPUS_ONLY");

    // Same shape as the silicon and kernel-pending pairs: the runner's selector is the exact predicate the
    // baseline negates. These are the RISC-V benchmark-corpus differentials — 280 assemble+emulate+VM
    // cycles, measured not to finish inside a 6000s budget on an IDLE box — so they run on purpose.
    assert!(
        excluded.contains(&format!("not ({only})")),
        "QEMU_CORPUS_EXCLUDE must drop exactly what run-qemu-corpus-tests.sh runs.\n  \
         baseline drops : {excluded}\n  runner runs    : {only}"
    );
}

#[test]
fn the_arena_mathlib_cases_the_baseline_drops_are_exactly_what_their_runner_picks_up() {
    let excluded = shell_var(&script("run-all-tests-fast.sh"), "ARENA_MATHLIB_EXCLUDE");
    let only = shell_var(&script("run-arena-mathlib-tests.sh"), "ARENA_MATHLIB_ONLY");

    // Same shape as the other pairs: the runner's selector is the exact predicate the baseline
    // negates. These are the whole-Mathlib checks of the three binaries that share the Arena case
    // table, each longer than the baseline's per-test backstop, so they run on purpose.
    assert!(
        excluded.contains(&format!("not ({only})")),
        "ARENA_MATHLIB_EXCLUDE must drop exactly what run-arena-mathlib-tests.sh runs.\n  \
         baseline drops : {excluded}\n  runner runs    : {only}"
    );
}

#[test]
fn the_model_bars_the_baseline_drops_are_exactly_what_their_runner_picks_up() {
    let excluded = shell_var(&script("run-all-tests-fast.sh"), "MODEL_BARS_EXCLUDE");
    let only = shell_var(&script("run-model-bars-tests.sh"), "MODEL_BARS_ONLY");

    // Same shape as the other pairs: the runner's selector is the exact predicate the baseline negates. These
    // walk a real checkpoint through the decode engine's model at every knob point against llama.cpp's runs,
    // minutes a knob point, so they run on purpose.
    assert!(
        excluded.contains(&format!("not ({only})")),
        "MODEL_BARS_EXCLUDE must drop exactly what run-model-bars-tests.sh runs.\n  \
         baseline drops : {excluded}\n  runner runs    : {only}"
    );
}

#[test]
fn every_heavy_exclusion_names_a_runner_that_exists_and_is_executable() {
    for runner in SELECTOR_RUNNERS
        .iter()
        .map(|(runner, _)| *runner)
        .chain([OPT_IN_RUNNER, HANDED_REDS_RUNNER])
    {
        let p = repo_root().join("scripts").join(runner);
        assert!(
            p.exists(),
            "{runner} is referenced as the on-demand home for excluded tests but is missing"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p)
                .expect("stat runner")
                .permissions()
                .mode();
            assert!(mode & 0o111 != 0, "{runner} is not executable (mode {mode:o}) — the exclusion comment tells people to run it");
        }
    }
    // The baseline must point at every runner, so someone reading an exclusion (or wondering why a
    // feature-gated suite never appears in the baseline) learns where those tests went.
    let gate = script("run-all-tests-fast.sh");
    for runner in SELECTOR_RUNNERS
        .iter()
        .map(|(runner, _)| *runner)
        .chain([OPT_IN_RUNNER, HANDED_REDS_RUNNER])
    {
        assert!(
            gate.contains(&format!("scripts/{runner}")),
            "run-all-tests-fast.sh leaves tests to scripts/{runner} but never names it"
        );
    }
}

/// Every `scripts/run-*-tests.sh` other than the two baselines is a runner this lock knows. A runner
/// added beside them without being registered here would own tests nobody holds it to.
#[test]
fn every_on_demand_runner_script_is_registered() {
    let known: BTreeSet<&str> = SELECTOR_RUNNERS
        .iter()
        .map(|(runner, _)| *runner)
        .chain([OPT_IN_RUNNER, HANDED_REDS_RUNNER])
        .collect();
    let mut found = BTreeSet::new();
    for entry in std::fs::read_dir(repo_root().join("scripts"))
        .expect("read scripts/")
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("run-") && name.ends_with("-tests.sh") && name != "run-all-tests.sh" {
            found.insert(name);
        }
    }
    let unregistered: Vec<_> = found
        .iter()
        .filter(|n| !known.contains(n.as_str()))
        .collect();
    assert!(
        unregistered.is_empty(),
        "{unregistered:?} run tests on demand but ondemand_exclusion_lock does not know them: add each to \
         SELECTOR_RUNNERS (with its selector variable), or make it OPT_IN_RUNNER's or HANDED_REDS_RUNNER's concern"
    );
    let missing: Vec<_> = known.iter().filter(|n| !found.contains(**n)).collect();
    assert!(
        missing.is_empty(),
        "registered runners {missing:?} are not in scripts/"
    );
}

/// The names in every plain `binary(name)` of a filterset (an alternation is `binary(/^(…)$/)`).
fn plain_binaries(filterset: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = filterset;
    while let Some(i) = rest.find("binary(") {
        let after = &rest[i + "binary(".len()..];
        let end = after.find(')').unwrap_or(after.len());
        let name = after[..end].trim();
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            out.insert(name.to_string());
        }
        rest = &after[end..];
    }
    out
}

/// The exact test names a filterset selects: `test(=name)` and every `test(/^(a|b)$/)` alternation.
/// A test regex that is not anchored at both ends names no exact test.
fn exact_test_names(filterset: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = filterset;
    while let Some(i) = rest.find("test(") {
        let after = &rest[i + "test(".len()..];
        let end = after.find(')').unwrap_or(after.len());
        if let Some(name) = after.strip_prefix('=') {
            out.insert(
                name[..name.find(')').unwrap_or(name.len())]
                    .trim()
                    .to_string(),
            );
        } else if let Some(alts) = after.strip_prefix("/^(") {
            if let Some(close) = alts.find(")$/)") {
                out.extend(
                    alts[..close]
                        .split('|')
                        .map(|n| n.trim().to_string())
                        .filter(|n| !n.is_empty()),
                );
                rest = &alts[close..];
                continue;
            }
        }
        rest = &after[end..];
    }
    out
}

/// The `#[test]` functions declared at the top level of one integration-test binary's root file:
/// the only tests an anchored name can select, since nextest names a test inside a module
/// `module::name`.
fn top_level_tests(file: &Path) -> BTreeSet<String> {
    let src =
        std::fs::read_to_string(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
    let ast =
        syn::parse_file(&src).unwrap_or_else(|e| panic!("{} does not parse: {e}", file.display()));
    ast.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(f)
                if f.attrs
                    .iter()
                    .any(|a| a.path().segments.last().is_some_and(|s| s.ident == "test")) =>
            {
                Some(f.sig.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

/// The product crate a runner's command builds the library of alone (`-p <crate> --lib`, no `--test`),
/// or `None` for any other command.
fn lib_runner_package(exec: &str) -> Option<String> {
    let words: Vec<&str> = exec.split_whitespace().collect();
    let packages: Vec<&str> = words
        .windows(2)
        .filter(|w| w[0] == "-p" || w[0] == "--package")
        .map(|w| w[1])
        .collect();
    match packages.as_slice() {
        [package]
            if *package != "logicaffeine-tests"
                && words.contains(&"--lib")
                && !words.contains(&"--test") =>
        {
            Some(package.to_string())
        }
        _ => None,
    }
}

/// The one `cargo nextest run` a runner runs — `exec`ed, or run before a verdict step reads its report — with
/// its `\`-continued lines joined.
fn exec_command(src: &str) -> String {
    let starts: Vec<usize> = src
        .match_indices('\n')
        .map(|(i, _)| i + 1)
        .filter(|&i| {
            src[i..].starts_with("exec cargo nextest run")
                || src[i..].starts_with("cargo nextest run")
        })
        .collect();
    assert_eq!(
        starts.len(),
        1,
        "a runner runs `cargo nextest run` exactly once, at the start of a line: {} found",
        starts.len()
    );
    let start = starts[0];
    let mut cmd = String::new();
    for line in src[start..].lines() {
        match line.strip_suffix('\\') {
            Some(continued) => cmd.push_str(continued),
            None => {
                cmd.push_str(line);
                break;
            }
        }
    }
    cmd
}

/// A runner builds exactly the test binaries its selector can run. `cargo nextest run -p
/// logicaffeine-tests` builds EVERY test binary of the crate, ~2,300 of them, whatever `-E` selects
/// (one such sweep filled 828 GB), so each runner names its binaries in a single-quoted `BINARIES`
/// and passes each as `--test`. The list is held to the selector both ways: a binary the selector
/// reaches that the list omits is never built, so its tests silently never run; a binary the list
/// names that the selector cannot reach is built for nothing.
///
/// A selector that names binaries (`binary(x)`, `binary(/^(a|b)$/)`) must name exactly the list. A
/// selector that names only exact tests (`test(=x)`, `test(/^(a|b)$/)`) must find every one of them
/// declared at the top level of some binary, and the list must be exactly the binaries that declare
/// them.
///
/// A runner over one product crate's unit tests (`-p <crate> --lib`) builds that library alone; its
/// selector must be scoped to `package(<crate>)`.
#[test]
fn every_runner_builds_exactly_the_binaries_its_selector_can_run() {
    let tests_dir = repo_root().join("crates/logicaffeine_tests/tests");
    for (runner, only_var) in SELECTOR_RUNNERS {
        let src = script(runner);
        let only = shell_var(&src, only_var);
        if let Some(package) = lib_runner_package(&exec_command(&src)) {
            assert!(
                only.contains(&format!("package({package})")),
                "{runner}: builds only {package}'s library, so its selector must be scoped to package({package}): {only}"
            );
            let exec = exec_command(&src);
            for (needle, why) in [
                (
                    &format!("-E \"${only_var}\"") as &str,
                    "select with its own selector",
                ),
                (
                    "--cargo-profile test-opt",
                    "build at the baseline's cargo profile, reusing its artifacts",
                ),
            ] {
                assert!(
                    exec.contains(needle),
                    "{runner}: its exec line must {why} (`{needle}`):\n  {exec}"
                );
            }
            continue;
        }
        let listed: BTreeSet<String> = shell_var(&src, "BINARIES")
            .split_whitespace()
            .map(String::from)
            .collect();
        assert!(!listed.is_empty(), "{runner}: BINARIES is empty");
        for bin in &listed {
            assert!(
                tests_dir.join(format!("{bin}.rs")).is_file() || tests_dir.join(bin).join("main.rs").is_file(),
                "{runner}: BINARIES names `{bin}`, which is not a test binary of logicaffeine-tests"
            );
        }

        let mut named = binary_alternation(&only);
        named.extend(plain_binaries(&only));
        let reachable: BTreeSet<String> = if !named.is_empty() {
            named
        } else {
            let names = exact_test_names(&only);
            assert!(
                !names.is_empty(),
                "{runner}: {only_var} names neither a binary nor an exact test, so the binaries it reaches cannot be \
                 derived: {only}"
            );
            let mut declaring: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
            for entry in std::fs::read_dir(&tests_dir)
                .expect("read tests dir")
                .flatten()
            {
                let path = entry.path();
                if path.extension().is_none_or(|x| x != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                if !names.iter().any(|n| text.contains(&format!("fn {n}("))) {
                    continue;
                }
                let bin = path.file_stem().unwrap().to_string_lossy().into_owned();
                for test in top_level_tests(&path).intersection(&names) {
                    declaring
                        .entry(test.clone())
                        .or_default()
                        .insert(bin.clone());
                }
            }
            for name in &names {
                assert!(
                    declaring.contains_key(name),
                    "{runner}: {only_var} selects `{name}`, which no test binary declares at its top level, so the \
                     selector runs nothing for it"
                );
            }
            declaring.into_values().flatten().collect()
        };

        let unbuilt: Vec<_> = reachable.difference(&listed).collect();
        assert!(
            unbuilt.is_empty(),
            "{runner}: its selector reaches {unbuilt:?} but BINARIES does not name them, so they are never built \
             and their tests never run.\n  selector : {only}\n  BINARIES : {listed:?}"
        );
        let stray: Vec<_> = listed.difference(&reachable).collect();
        assert!(
            stray.is_empty(),
            "{runner}: BINARIES names {stray:?}, which its selector cannot reach — built for nothing.\n  \
             selector : {only}\n  BINARIES : {listed:?}"
        );

        assert!(
            src.contains(
                "\nTEST_ARGS=()\nfor bin in $BINARIES; do TEST_ARGS+=(--test \"$bin\"); done\n"
            ),
            "{runner}: TEST_ARGS must be built from BINARIES as `--test \"$bin\"` pairs"
        );
        let exec = exec_command(&src);
        for (needle, why) in [
            (
                "\"${TEST_ARGS[@]}\"",
                "pass the `--test` args, or cargo builds every test binary of the crate",
            ),
            (
                &format!("-E \"${only_var}\"") as &str,
                "select with its own selector",
            ),
            (
                "--cargo-profile test-opt",
                "build at the baseline's cargo profile, reusing its artifacts",
            ),
        ] {
            assert!(
                exec.contains(needle),
                "{runner}: its exec line must {why} (`{needle}`):\n  {exec}"
            );
        }
    }
}

/// CI runs the baseline the fast gate runs. `.github/workflows/test.yml` reads the exclusion classes
/// out of `run-all-tests-fast.sh` at run time instead of restating them, so a class added or edited
/// in the gate is excluded in CI in the same change, and each still has exactly one companion runner.
/// A restated copy is how CI drifted before: it mirrored one class of four, and the nightly could
/// never pass.
#[test]
fn ci_excludes_exactly_what_the_fast_baseline_excludes() {
    let gate = script("run-all-tests-fast.sh");
    let ci = std::fs::read_to_string(repo_root().join(".github/workflows/test.yml"))
        .expect("read .github/workflows/test.yml");

    let defined: BTreeSet<String> = gate
        .lines()
        .filter_map(|l| l.split_once("_EXCLUDE='").map(|(name, _)| name))
        .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_uppercase() || c == '_'))
        .map(String::from)
        .collect();
    assert!(
        defined.len() >= 4,
        "the gate defines only {defined:?} exclusion classes — the parser or the gate changed"
    );

    let eval_prefix = "grep -E '^(";
    let eval_suffix = ")_EXCLUDE=' scripts/run-all-tests-fast.sh";
    let read_by_ci: BTreeSet<String> = ci
        .lines()
        .find_map(|l| {
            let start = l.find(eval_prefix)? + eval_prefix.len();
            let end = l[start..].find(eval_suffix)? + start;
            Some(l[start..end].split('|').map(String::from).collect())
        })
        .unwrap_or_else(|| {
            panic!(
                "test.yml no longer reads the exclusion classes from scripts/run-all-tests-fast.sh"
            )
        });
    assert_eq!(
        read_by_ci, defined,
        "CI must read exactly the exclusion classes the gate defines"
    );

    let filter_line = ci
        .lines()
        .find(|l| l.trim_start().starts_with("BASELINE_FILTER="))
        .expect("test.yml must assemble BASELINE_FILTER from the classes it read");
    for class in &defined {
        assert!(
            filter_line.contains(&format!("${class}_EXCLUDE")),
            "CI's BASELINE_FILTER omits {class}_EXCLUDE: {filter_line}"
        );
    }
    assert!(
        ci.contains("-E \"$BASELINE_FILTER\""),
        "CI's nextest run must filter by $BASELINE_FILTER"
    );
    assert!(
        !ci.contains("binary(/^(") && !ci.contains("not test(/"),
        "test.yml restates a filterset literal — read it from scripts/run-all-tests-fast.sh instead"
    );
}

/// A heavy that drives a prebuilt RELEASE `largo` is given one by the gate. `ntt_logos`'s AOT races
/// time what a user gets from `largo build --release`, reading `target/release/largo`; they are
/// `#[ignore]`d, so only a run with ignored tests reaches them, and a fresh target directory has no
/// such binary. The gate therefore builds it inside its `RUN_IGNORED` branch. Without that the
/// races fail on a fresh tree and pass on a stale one, which measures a binary nobody built from
/// the tree under test.
#[test]
fn every_heavy_that_drives_the_release_largo_is_given_one_by_the_gate() {
    let tests_dir = repo_root().join("crates/logicaffeine_tests/tests");
    let mut drivers = Vec::new();
    for entry in std::fs::read_dir(&tests_dir)
        .expect("read tests dir")
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_some_and(|x| x == "rs") {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            if text.contains("\"target/release/largo\"") {
                drivers.push(path.file_name().unwrap().to_string_lossy().into_owned());
            }
        }
    }
    if drivers.is_empty() {
        return;
    }
    let gate = script("run-all-tests-fast.sh");
    let branch = gate
        .find("if (( RUN_IGNORED )); then")
        .and_then(|at| gate[at..].find("\nfi\n").map(|end| &gate[at..at + end]))
        .filter(|b| b.contains("cargo build -p logicaffeine-cli --release"));
    assert!(
        branch.is_some(),
        "{drivers:?} drive target/release/largo, so run-all-tests-fast.sh must build it \
         (`cargo build -p logicaffeine-cli --release`) inside its `if (( RUN_IGNORED ))` branch"
    );
}

/// ★ A GATE-LEVEL SIM IS A SILICON TEST, WHEREVER IT LIVES. A `postsynth_` test synthesizes a netlist and simulates it gate
/// by gate — an hour where the baseline's backstop is half that — so every one, in whatever suite, is one the baseline
/// drops and the silicon runner runs. `fabric_postsynth_lock`'s ran in neither list and timed out the baseline.
#[test]
fn every_postsynth_test_is_a_silicon_sim_the_baseline_drops() {
    let silicon = shell_var(&script("run-silicon-tests.sh"), "SILICON_ONLY");
    let claimed = binary_alternation(&silicon);
    assert!(silicon.contains("test(/^postsynth_/)"), "the silicon runner selects the `postsynth_` tests: {silicon}");
    let dir = repo_root().join("crates/logicaffeine_tests/tests");
    let mut unclaimed = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        if top_level_tests(&path).iter().any(|t| t.starts_with("postsynth_")) {
            let bin = path.file_stem().unwrap().to_string_lossy().to_string();
            if !claimed.contains(&bin) {
                unclaimed.push(bin);
            }
        }
    }
    unclaimed.sort();
    assert!(unclaimed.is_empty(), "suites with `postsynth_` tests the silicon runner does not claim, so the baseline runs them: {unclaimed:?}\n  SILICON_ONLY: {silicon}");
}

/// The handed-reds runner runs exactly the suites that open with the `handed-reds` gate: one the runner leaves out is
/// compiled by nothing and never runs, and one it names that has dropped the gate has landed and runs in the baseline.
#[test]
fn the_handed_reds_runner_runs_exactly_the_suites_behind_the_feature() {
    let listed: BTreeSet<String> = shell_var(&script(HANDED_REDS_RUNNER), "BINARIES").split_whitespace().map(String::from).collect();
    let tests = repo_root().join("crates/logicaffeine_tests/tests");
    let gated: BTreeSet<String> = std::fs::read_dir(&tests)
        .expect("read the tests directory")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .filter(|p| {
            std::fs::read_to_string(p)
                .unwrap_or_default()
                .lines()
                .any(|l| l == "#![cfg(feature = \"handed-reds\")]")
        })
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(!gated.is_empty() || listed.is_empty(), "the runner names suites while none is gated: {listed:?}");
    assert_eq!(listed, gated, "{HANDED_REDS_RUNNER}'s BINARIES must be exactly the suites behind `handed-reds`");
}

/// What one top-level function of a test file does: whether it is a `#[test]`, the free functions it calls by name, and
/// whether it starts Verilator itself (`common::verilator()`, or the step program's `run-cosim.sh`).
#[derive(Default)]
struct FnFacts {
    test: bool,
    calls: BTreeSet<String>,
    starts_verilator: bool,
}

/// Every top-level function of `src`, parsed: its calls are read from the expressions and from the token streams of
/// the macros it writes (`assert!(step(…))` calls `step`), never from comments or other names that end alike.
fn function_facts(src: &str) -> BTreeMap<String, FnFacts> {
    use proc_macro2::TokenTree;
    use syn::visit::Visit;
    fn tokens(stream: proc_macro2::TokenStream, facts: &mut FnFacts) {
        let trees: Vec<TokenTree> = stream.into_iter().collect();
        for (i, tree) in trees.iter().enumerate() {
            match tree {
                TokenTree::Ident(id) => {
                    if let Some(TokenTree::Group(g)) = trees.get(i + 1) {
                        if g.delimiter() == proc_macro2::Delimiter::Parenthesis
                            && !matches!(i.checked_sub(1).map(|j| &trees[j]), Some(TokenTree::Punct(p)) if p.as_char() == '.')
                        {
                            facts.calls.insert(id.to_string());
                        }
                    }
                }
                TokenTree::Literal(l) if l.to_string().contains("run-cosim.sh") => facts.starts_verilator = true,
                TokenTree::Group(g) => tokens(g.stream(), facts),
                _ => {}
            }
        }
    }
    struct Walk<'f>(&'f mut FnFacts);
    impl<'a> Visit<'a> for Walk<'_> {
        fn visit_expr_call(&mut self, c: &'a syn::ExprCall) {
            if let syn::Expr::Path(p) = &*c.func {
                let segs: Vec<String> = p.path.segments.iter().map(|s| s.ident.to_string()).collect();
                if segs == ["common", "verilator"] {
                    self.0.starts_verilator = true;
                }
                if let [only] = segs.as_slice() {
                    self.0.calls.insert(only.clone());
                }
            }
            syn::visit::visit_expr_call(self, c);
        }
        fn visit_lit_str(&mut self, s: &'a syn::LitStr) {
            if s.value().contains("run-cosim.sh") {
                self.0.starts_verilator = true;
            }
        }
        fn visit_macro(&mut self, m: &'a syn::Macro) {
            let stream = m.tokens.clone();
            if stream.to_string().contains("common :: verilator ()") {
                self.0.starts_verilator = true;
            }
            tokens(stream, self.0);
        }
    }
    let ast = syn::parse_file(src).unwrap_or_else(|e| panic!("the test file does not parse: {e}"));
    let mut out = BTreeMap::new();
    for item in &ast.items {
        if let syn::Item::Fn(f) = item {
            let mut facts = FnFacts {
                test: f.attrs.iter().any(|a| a.path().segments.last().is_some_and(|s| s.ident == "test")),
                ..FnFacts::default()
            };
            Walk(&mut facts).visit_block(&f.block);
            out.insert(f.sig.ident.to_string(), facts);
        }
    }
    out
}

/// The tests of `d5005_engine_card_lock` that run Verilator: each that starts it itself or calls one of the file's
/// functions that does, however deep.
fn card_cosimulations() -> BTreeSet<String> {
    let path = repo_root().join("crates/logicaffeine_tests/tests/d5005_engine_card_lock.rs");
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let fns = function_facts(&src);
    let mut reaching: BTreeSet<String> = fns.iter().filter(|(_, f)| f.starts_verilator).map(|(n, _)| n.clone()).collect();
    loop {
        let more: Vec<String> = fns
            .iter()
            .filter(|(name, f)| !reaching.contains(*name) && f.calls.iter().any(|c| reaching.contains(c)))
            .map(|(name, _)| name.clone())
            .collect();
        if more.is_empty() {
            break;
        }
        reaching.extend(more);
    }
    fns.iter().filter(|(name, f)| f.test && reaching.contains(*name)).map(|(name, _)| name.clone()).collect()
}

/// The `d5005_engine_card_lock` tests a selector names: its `binary(d5005_engine_card_lock) and test(/^(…)$/)` clause.
fn card_tests_named(selector: &str) -> BTreeSet<String> {
    let open = "binary(d5005_engine_card_lock) and test(/^(";
    let start = selector.find(open).unwrap_or_else(|| panic!("the selector has no d5005_engine_card_lock clause: {selector}"));
    let rest = &selector[start + open.len()..];
    let end = rest.find(")$/)").expect("the clause's alternation closes");
    rest[..end].split('|').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// A card co-simulation runs the engine build under Verilator for 600–3,400 s alone (Tristen, 2026-10-23: on demand,
/// beside the silicon sims), so the baseline must not reach one: the silicon runner names exactly the card lock's
/// tests that start Verilator, derived from the file, and a new co-simulation fails here by name rather than running
/// into the fast suite's 30-minute cap.
#[test]
fn every_card_cosimulation_runs_on_demand_and_no_fast_card_test_does() {
    let named = card_tests_named(&shell_var(&script("run-silicon-tests.sh"), "SILICON_ONLY"));
    let cosims = card_cosimulations();
    assert!(cosims.len() >= 20, "found {} card co-simulations: the reader is reading nothing", cosims.len());
    let missing: Vec<&String> = cosims.difference(&named).collect();
    let extra: Vec<&String> = named.difference(&cosims).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "SILICON_ONLY's d5005_engine_card_lock clause must name exactly the tests that start Verilator.\n  \
         co-simulations it leaves to the fast suite: {missing:?}\n  names that start no Verilator: {extra:?}"
    );
}

#[test]
fn the_function_reader_reads_the_shape_it_guards() {
    let src = r#"
fn helper() {
    let _ = std::process::Command::new(common::verilator());
}
fn script() {
    let _ = std::path::Path::new("scripts/d5005/run-cosim.sh");
}
/// doc
#[test]
#[ignore]
fn slow() {
    assert!(helper() == ());
}
#[test]
fn fast() {
    // helper();
    the_helper();
    x.helper();
    assert!(true);
}
"#;
    let fns = function_facts(src);
    assert_eq!(fns.keys().cloned().collect::<Vec<_>>(), ["fast", "helper", "script", "slow"]);
    assert!(fns["slow"].test && fns["fast"].test && !fns["helper"].test);
    assert!(fns["helper"].starts_verilator && fns["script"].starts_verilator && !fns["slow"].starts_verilator);
    assert!(fns["slow"].calls.contains("helper"), "a call inside a macro is read: {:?}", fns["slow"].calls);
    assert!(!fns["fast"].calls.contains("helper"), "a comment, a longer name and a method are not calls: {:?}", fns["fast"].calls);
    let clause = "(binary(d5005_engine_card_lock) and test(/^(a_b|c_d)$/))";
    assert_eq!(card_tests_named(clause), BTreeSet::from(["a_b".to_string(), "c_d".to_string()]));
}
