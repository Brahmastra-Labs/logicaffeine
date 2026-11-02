//! G14 — the SKIP-BUDGET ratchet. The honest-preflight sweep (`require_tool`) is
//! only durable if a NEW dishonest skip cannot be added. This lock scans every
//! source file in this test crate for the old SKIP-as-PASS idioms and FAILS if any
//! appears outside a tiny, JUSTIFIED allowlist — so `common::require_tool` stays the
//! single sanctioned skip channel forever.
//!
//! Two dishonest idioms are hunted:
//!   1. `eprintln!("SKIP…")` / `println!("SKIP…")` — a print-then-`return` that
//!      reports PASS to cargo for a test that never actually ran.
//!   2. A raw external-tool probe used as a GATE — `Command::new("<tool>").arg("-V"|
//!      "--version")` whose result decides a `return;` — which is exactly what
//!      `require_tool` replaces (and which, unlike `require_tool`, cannot fail loud
//!      under `LOGOS_REQUIRE_TOOLS`).
//!
//! The sanctioned channel is `common::require_tool(<tool>)`, which on a missing tool
//! emits the machine-readable `LOGOS_TOOL_SKIP: <tool>` marker and returns false
//! (dev box), or PANICS under `LOGOS_REQUIRE_TOOLS` (the release gate). See
//! `tool_preflight_lock.rs` for the seam's own contract.
//!
//! ── MIGRATED DEBT ─────────────────────────────────────────────────────────────
//! Every crate's tests moved into this one. The CLI suites arrived already swept (their own copy of
//! this lock held them to the same allowlist), but compile's arrived with ~200 SKIP-as-PASS sites
//! under a crate-wide ceiling of 210 — its lock counted rather than converted: nvcc, gcc, ngspice,
//! RISC-V cross toolchains and multi-gigabyte checkpoints are real gates on a box without them. That
//! debt is frozen PER FILE in `goldens/skip_debt.txt`: a listed file may keep at most its listed
//! counts, a count may only fall (lower it when a site moves to `common::require_tool`), and no
//! file may join the list. Everywhere else the allowlist rule below holds unchanged.
//!
//! ⚠️  YOU DO NOT GET TO WEAKEN THIS LOCK OR PAD ITS ALLOWLIST TO GET GREEN. ⚠️
//! A new external-tool gate must go through `require_tool`; if you are tempted to
//! allowlist a file, you are adding a false-green. Stop and use the seam instead.
//!
//! ── RELEASE-GATE OPERATOR NOTES ────────────────────────────────────────────────
//! * `LOGOS_REQUIRE_TOOLS` turns every honest skip into a loud failure. A release
//!   gate that must establish the FULL external-tool matrix sets it to `1` (or
//!   `all`) — then a missing python3/node/nvcc/iverilog/yosys/gcc PANICS instead of
//!   skipping. To force a SUBSET, pass a comma/space list, e.g.
//!   `LOGOS_REQUIRE_TOOLS="python3 gcc iverilog"`. A plain `cargo test` (unset)
//!   stays runnable on any box, honestly recording which validators it lacked.
//! * HTTP PARITY HONESTY. The HTTP cross-tier differential that runs by DEFAULT
//!   (`http_serve_cross_tier.rs`) proves VM ≡ interpreter only — both legs run
//!   in-process (`run_vm_io_async` vs `interpret_for_ui`), so a default `cargo test`
//!   establishes NO AOT/HTTP parity (no HTTP program is Logos→Rust→rustc→run in the
//!   default suite). Any test that WOULD compile an HTTP server through the AOT tier
//!   is slow (a full rustc build, minutes) and therefore belongs behind `#[ignore]`,
//!   run on purpose by the release gate with `-- --ignored`, never on the fast
//!   default. `http_default_differential_is_vm_vs_interpreter_only` below pins that
//!   honesty: the default HTTP differential must stay in-process (it must NOT quietly
//!   grow an AOT build and start claiming a parity the fast default never actually
//!   ran).

#![allow(unused_imports, dead_code)]
#[path = "common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const DEBT: &str = "goldens/skip_debt.txt";

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// Every `.rs` under `tests/` (recursing into `common/`), returned as (relative
/// name, contents). The scanner reads source text, never executes anything.
fn all_test_sources() -> Vec<(String, String)> {
    let root = tests_dir();
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                let rel = p
                    .strip_prefix(&root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .replace('\\', "/");
                let text = std::fs::read_to_string(&p).unwrap_or_default();
                out.push((rel, text));
            }
        }
    }
    out.sort();
    out
}

/// The JUSTIFIED allowlist — the ONLY places a `SKIP`-shaped print may remain. Each
/// entry is a `(file, reason)` pair; a reason is REQUIRED so an addition is a
/// deliberate, reviewed act, never a silent pad. NONE of these is an external-tool
/// gate (those must use `require_tool`); they gate on something `LOGOS_REQUIRE_TOOLS`
/// cannot force — a physical board, an optional private artifact, an optional
/// third-party dataset, a per-datum size bound, or a per-iteration fuzz boundary.
const SKIP_PRINT_ALLOWLIST: &[(&str, &str)] = &[
    (
        "skip_budget_lock.rs",
        "this lock — its doc comment and code literally quote the forbidden idioms",
    ),
    (
        "tool_preflight_lock.rs",
        "the seam's own contract test — it quotes SKIP in prose and drives the marker",
    ),
    (
        "cli_tool_preflight_lock.rs",
        "the CLI suites' seam contract test — it quotes SKIP in prose and drives the markers",
    ),
    (
        "test_runner_lock.rs",
        "asserts on the test RUNNER's own SKIP line format — SKIP is data under test, not a gate",
    ),
    (
        "phase_hw_benchmark.rs",
        "gates on an OPTIONAL third-party dataset (FVEval CSVs), not a tool — not forceable via LOGOS_REQUIRE_TOOLS",
    ),
    (
        "wprog_all_benchmarks_lane.rs",
        "per-benchmark size skip (demo heap exceeds the sim cap) — a data-size bound, not a tool gate",
    ),
    (
        "wpe_postsynth_silicon_lock.rs",
        "None-arm reports after synth_and_gatelevel_result (which itself require_tool-gates yosys+iverilog); \
         the tool contract is enforced inside the helper, these are secondary yosys-datadir/#[ignore] notes",
    ),
    (
        "wprog_engine_rtl_fuzz.rs",
        "per-iteration lowering-boundary print inside a fuzz loop that still asserts a `ran >= 10` floor — \
         not a whole-test SKIP-as-PASS; its tool gate uses require_tool",
    ),
];

/// Files whose leftover raw `Command::new(<tool>).arg(-V|--version)` is NOT a skip
/// gate (it launches the real work / is the seam itself), so the raw-probe scan
/// must not flag them.
const RAW_PROBE_ALLOWLIST: &[(&str, &str)] = &[
    (
        "common/mod.rs",
        "the seam itself — `tool_available` IS the one place that probes a tool",
    ),
    (
        "cli_common/mod.rs",
        "the CLI suites' seam — its `tool_available` is the one place those suites probe a tool",
    ),
];

/// The migrated debt, per file: `(SKIP prints, raw tool probes)` it may still carry.
fn debt() -> BTreeMap<String, (usize, usize)> {
    let text = std::fs::read_to_string(tests_dir().join(DEBT))
        .unwrap_or_else(|e| panic!("read {DEBT}: {e}"));
    text.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            assert_eq!(
                cols.len(),
                3,
                "{DEBT}: `{l}` is not `<file>\\t<prints>\\t<probes>`"
            );
            (
                cols[0].to_string(),
                (
                    cols[1].parse().expect("prints"),
                    cols[2].parse().expect("probes"),
                ),
            )
        })
        .collect()
}

fn allowlisted(list: &[(&str, &str)], file: &str) -> bool {
    list.iter().any(|(f, _)| file.ends_with(f))
}

/// A `SKIP`-shaped print is the classic SKIP-as-PASS: `eprintln!`/`println!` whose
/// first string arg starts with `SKIP` or `skip`. (`SKIPPED`/`skipping` count too —
/// the same false-green shape.)
fn skip_print_lines(text: &str) -> Vec<(usize, String)> {
    let mut hits = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start();
        let is_skip_macro = (t.starts_with("eprintln!(\"") || t.starts_with("println!(\"")) && {
            // The literal begins right after the opening quote.
            let after = &t[t.find('"').unwrap() + 1..];
            after.starts_with("SKIP") || after.starts_with("skip")
        };
        if is_skip_macro {
            hits.push((i + 1, line.trim().to_string()));
        }
    }
    hits
}

/// A raw external-tool probe used as a gate: `Command::new("<tool>")` on a line
/// whose surrounding window contains `.arg("-V")` or `.arg("--version")`. This is
/// the exact shape `require_tool` replaces; the ONLY sanctioned prober is the seam.
fn raw_tool_probe_lines(text: &str) -> Vec<(usize, String)> {
    const TOOLS: &[&str] = &[
        "python3",
        "python",
        "node",
        "nvcc",
        "iverilog",
        "yosys",
        "gcc",
        "riscv64-linux-gnu-gcc",
        "qemu-riscv64",
        "verilator",
    ];
    let lines: Vec<&str> = text.lines().collect();
    let mut hits = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        // A comment (doc or line) that QUOTES the pattern is prose, not a gate — this
        // lock's own header does exactly that. Only real code counts.
        if line.trim_start().starts_with("//") {
            continue;
        }
        let names_a_tool = TOOLS
            .iter()
            .any(|t| line.contains(&format!("Command::new(\"{t}\")")));
        if !names_a_tool {
            continue;
        }
        // A probe gate spells `-V`/`--version` within a few lines of the Command::new.
        let lo = i.saturating_sub(2);
        let hi = (i + 4).min(lines.len());
        let window = lines[lo..hi].join("\n");
        let is_probe = window.contains(".arg(\"-V\")") || window.contains(".arg(\"--version\")");
        if is_probe {
            hits.push((i + 1, line.trim().to_string()));
        }
    }
    hits
}

/// THE RATCHET: no dishonest SKIP-as-PASS print survives outside the justified
/// allowlist. A new one added anywhere else fails this test — forcing the author to
/// the `require_tool` seam.
#[test]
fn no_dishonest_skip_prints_outside_the_allowlist() {
    let debt = debt();
    let mut violations = Vec::new();
    for (file, text) in all_test_sources() {
        if allowlisted(SKIP_PRINT_ALLOWLIST, &file) {
            continue;
        }
        let hits = skip_print_lines(&text);
        let owed = debt.get(&file).map(|d| d.0).unwrap_or(0);
        if hits.len() <= owed {
            continue;
        }
        for (ln, src) in hits {
            violations.push(format!(
                "{file}:{ln}  SKIP-as-PASS print — a test that prints SKIP and returns reports \
                 PASS to cargo though it never ran. Use `common::require_tool(<tool>)` (honest \
                 marker + loud panic under LOGOS_REQUIRE_TOOLS) instead.\n      {src}"
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "{} dishonest SKIP-as-PASS print(s) found — the honest-preflight sweep is rotting:\n  - {}",
        violations.len(),
        violations.join("\n  - ")
    );
}

/// THE RATCHET, second axis: no raw external-tool probe is used as a gate outside
/// the seam. A reintroduced `Command::new("iverilog").arg("-V")` skip gate fails
/// here even if it is silent (no SKIP print) — because a silent tool gate is a
/// false-green that `LOGOS_REQUIRE_TOOLS` can never turn loud.
#[test]
fn no_raw_tool_probe_gates_outside_the_seam() {
    let debt = debt();
    let mut violations = Vec::new();
    for (file, text) in all_test_sources() {
        if allowlisted(RAW_PROBE_ALLOWLIST, &file) {
            continue;
        }
        let hits = raw_tool_probe_lines(&text);
        let owed = debt.get(&file).map(|d| d.1).unwrap_or(0);
        if hits.len() <= owed {
            continue;
        }
        for (ln, src) in hits {
            violations.push(format!(
                "{file}:{ln}  raw tool-probe gate — probe an external tool through \
                 `common::require_tool(<tool>)`, not a hand-rolled `Command::new(..).arg(\"-V\")` \
                 check (only that seam fails loud under LOGOS_REQUIRE_TOOLS).\n      {src}"
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "{} raw tool-probe gate(s) found outside the require_tool seam:\n  - {}",
        violations.len(),
        violations.join("\n  - ")
    );
}

/// Every allowlist entry must carry a non-empty reason AND still match a real file
/// under `tests/` — so the allowlist cannot rot into stale, unjustified pads. (A
/// file that no longer exists, or an empty reason, fails here.)
#[test]
fn the_allowlist_is_justified_and_live() {
    let files: Vec<String> = all_test_sources().into_iter().map(|(f, _)| f).collect();
    let matches = |name: &str| files.iter().any(|f| f.ends_with(name));
    for (name, reason) in SKIP_PRINT_ALLOWLIST.iter().chain(RAW_PROBE_ALLOWLIST) {
        assert!(
            !reason.trim().is_empty(),
            "allowlist entry `{name}` has no reason"
        );
        assert!(
            matches(name),
            "allowlist entry `{name}` matches no file under tests/ — remove the stale pad"
        );
    }
}

/// The sanctioned channel is present and used: `common::require_tool` must be the
/// vocabulary the sweep speaks. This asserts the seam is wired into real gates (not
/// just defined) — a floor on how many files route their tool preflight through it,
/// so a regression that rips `require_tool` back out for raw probes is caught.
#[test]
fn require_tool_is_the_live_skip_channel() {
    let mut users = 0usize;
    for (file, text) in all_test_sources() {
        if file.ends_with("common/mod.rs") {
            continue;
        }
        if text.contains("common::require_tool(") {
            users += 1;
        }
    }
    assert!(
        users >= 30,
        "only {users} test files route their external-tool preflight through \
         `common::require_tool` — the honest-preflight sweep has regressed (floor 30)"
    );
}

/// CI VISIBILITY: emit a machine-readable line naming exactly which external
/// validators are present (and would ACTUALLY run) vs absent (and would honestly
/// skip) on this box. A CI run greps `LOGOS_VALIDATORS_RAN:` to report the matrix,
/// so a green run's coverage is legible: `python3=yes node=no nvcc=no …` tells you
/// which validators the green actually exercised. Under `LOGOS_REQUIRE_TOOLS` a
/// `no` for a required tool would already have PANICKED elsewhere; this line is the
/// human/CI-facing summary of that same truth. Always passes — it is a REPORT, not
/// a gate — the gating lives in `require_tool`.
#[test]
fn report_which_validators_ran() {
    const VALIDATORS: &[&str] = &[
        "python3",
        "node",
        "nvcc",
        "iverilog",
        "yosys",
        "gcc",
        "cc",
        "riscv64-linux-gnu-gcc",
        "qemu-riscv64",
        "verilator",
    ];
    let mut parts = Vec::with_capacity(VALIDATORS.len());
    for t in VALIDATORS {
        let yn = if common::tool_available(t) {
            "yes"
        } else {
            "no"
        };
        parts.push(format!("{t}={yn}"));
    }
    let require = std::env::var("LOGOS_REQUIRE_TOOLS").unwrap_or_default();
    let require = if require.trim().is_empty() {
        "(unset)".to_string()
    } else {
        require
    };
    println!("LOGOS_VALIDATORS_RAN: {}", parts.join(" "));
    println!("LOGOS_REQUIRE_TOOLS: {require}");
    // A report, never a gate: it must always pass so it never masks a real failure.
    assert!(!parts.is_empty());
}

/// HTTP PARITY HONESTY (see the module header). The default HTTP cross-tier
/// differential proves VM ≡ interpreter ONLY — both legs run in-process, so a
/// default `cargo test` establishes no AOT/HTTP parity. This pins that: the default
/// differential must NOT quietly grow an AOT compile-and-run leg (which would be
/// slow AND would make the fast default claim a parity it never ran); a real
/// AOT/HTTP proof is slow and belongs behind `#[ignore]`, run by the release gate
/// with `-- --ignored`.
#[test]
fn http_default_differential_is_vm_vs_interpreter_only() {
    let path = tests_dir().join("http_serve_cross_tier.rs");
    let Ok(text) = std::fs::read_to_string(&path) else {
        // File owned by another workstream / renamed — the honesty note in the module
        // header still stands; nothing to pin here.
        return;
    };
    // The default legs are the in-process engines. If an AOT build appears without an
    // #[ignore], the fast default has silently taken on a slow parity claim.
    let aot_markers = [
        "run_logos",
        "compile_program_full",
        "run_rust_project",
        "compile_logos",
    ];
    let does_aot = aot_markers.iter().any(|m| text.contains(m));
    if does_aot {
        assert!(
            text.contains("#[ignore]"),
            "http_serve_cross_tier.rs grew an AOT compile-and-run leg but is not #[ignore]d — a \
             slow AOT/HTTP parity proof must not ride the fast default; gate it behind #[ignore] \
             and run it in the release gate with `-- --ignored`"
        );
    }
    // Positively affirm the default legs are the in-process VM + interpreter (the
    // honest scope of what a default `cargo test` proves for HTTP).
    assert!(
        text.contains("run_vm_io_async") && text.contains("interpret_for_ui"),
        "the default HTTP differential must remain the in-process VM≡interpreter parity \
         (run_vm_io_async vs interpret_for_ui) — that is the exact scope a default `cargo test` \
         establishes for HTTP; anything more (AOT) is slow and belongs behind #[ignore]"
    );
}

/// ★ THE RATCHET on the migrated debt (compile's crate-wide ceiling, now per file): no listed file
/// carries more than its listed counts, and no file outside the list carries any — the two tests
/// above enforce both per site. This one holds the totals and names the heaviest files.
#[test]
fn the_skip_budget_only_ever_shrinks() {
    let debt = debt();
    let mut prints = 0usize;
    let mut probes = 0usize;
    let mut over = Vec::new();
    for (file, text) in all_test_sources() {
        let (p, r) = (
            skip_print_lines(&text).len(),
            raw_tool_probe_lines(&text).len(),
        );
        if let Some(&(dp, dr)) = debt.get(&file) {
            prints += p;
            probes += r;
            if p > dp || r > dr {
                over.push(format!(
                    "{file}: {p} print(s) / {r} probe(s), frozen at {dp} / {dr}"
                ));
            }
        }
    }
    let (dp, dr): (usize, usize) = debt.values().fold((0, 0), |a, d| (a.0 + d.0, a.1 + d.1));
    assert!(
        over.is_empty() && prints <= dp && probes <= dr,
        "the migrated SKIP debt grew ({prints} prints / {probes} probes against {dp} / {dr}):\n  {}\n\n\
         Do not raise a count in {DEBT}. Gate the test through `common::require_tool(<tool>)`, which \
         prints `LOGOS_TOOL_SKIP: <tool>` and panics under `LOGOS_REQUIRE_TOOLS`.",
        over.join("\n  ")
    );
}

/// A skip that names nothing is a skip nobody can act on: every SKIP print left anywhere in the
/// crate says what was missing (a `{}` placeholder counts — the call site chooses the value).
#[test]
fn every_skip_says_what_it_needed() {
    let mut anonymous = Vec::new();
    for (file, text) in all_test_sources() {
        if allowlisted(SKIP_PRINT_ALLOWLIST, &file) {
            continue;
        }
        for (ln, src) in skip_print_lines(&text) {
            let after = &src[src.find('"').map(|i| i + 1).unwrap_or(0)..];
            let printed = after.split('"').next().unwrap_or("");
            let named = printed
                .trim_start_matches(|c: char| c.is_ascii_alphabetic())
                .trim_start_matches([':', ' ', '-', '—']);
            let names_something = named.chars().filter(|c| c.is_ascii_alphanumeric()).count() >= 3
                || printed.contains('{');
            if !names_something {
                anonymous.push(format!("{file}:{ln}"));
            }
        }
    }
    assert!(
        anonymous.is_empty(),
        "{} skip(s) print `SKIP` and say nothing about what was missing:\n  {}\n\n\
         Name the tool, the device or the file — one phrase is enough (`gcc absent`, `needs nvcc + \
         /dev/nvidia0`) — or better, gate through `common::require_tool`.",
        anonymous.len(),
        anonymous.join("\n  ")
    );
}

/// The ledger stays honest in the other direction: a file that shed sites must have its counts
/// lowered (and a file that shed all of them must leave the list), so the ground that was won
/// stays won.
#[test]
fn the_ceiling_has_not_gone_slack() {
    let sources: BTreeMap<String, String> = all_test_sources().into_iter().collect();
    let mut slack = Vec::new();
    for (file, (dp, dr)) in debt() {
        let (p, r) = match sources.get(&file) {
            Some(text) => (
                skip_print_lines(text).len(),
                raw_tool_probe_lines(text).len(),
            ),
            None => (0, 0),
        };
        if p < dp || r < dr {
            slack.push(format!("{file}: now {p} / {r}, listed {dp} / {dr}"));
        }
    }
    assert!(
        slack.is_empty(),
        "lower these entries in {DEBT} (the debt only shrinks):\n  {}",
        slack.join("\n  ")
    );
}
