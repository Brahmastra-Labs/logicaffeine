//! Wave 1 — the same program must MEAN the same thing on every tier.
//!
//! The v1.0 readiness audit found that a handful of ordinary programs produce DIFFERENT answers
//! depending on which tier runs them: a struct field defaults to `nothing` on the interpreter and
//! `0` on AOT; a missing map key is a catchable error on one tier, a Rust panic on another, and a
//! silently zeroed value on a third. Each of those is a wrong answer rather than an error, which is
//! the worst failure a language can have.
//!
//! THIS LOCK ASSERTS AGREEMENT, NOT A CHOSEN ANSWER. Several of these probes sit on genuine open
//! language questions (should `length of` count bytes or characters? should `1 + "a"` be a type
//! error?), and those are Tristen's rulings, not the compiler's. But whichever way each is ruled,
//! every tier must give the SAME answer — so agreement can be locked today, without waiting on a
//! single ruling, and it is the property that actually protects users.
//!
//! HOW IT STAYS HONEST. Probes that disagree today are listed in `KNOWN_DIVERGENCES` with the
//! evidence. The assertion is an EXACT SET MATCH, not a subset: a probe that starts diverging fails
//! the lock (a regression), and a probe that stops diverging ALSO fails it (a stale entry that must
//! be deleted). A ledger that only shrinks can still lie by omission; one that must match exactly
//! cannot. Nothing here is `#[ignore]`d to hide a red — the fast test runs on every push.

use logicaffeine_compile::compile::{
    compile_to_program_optimized, program_outcome, run_logos_source, tw_outcome, vm_outcome,
};

/// One probe: a minimal program whose cross-tier behaviour the audit questioned.
struct Probe {
    /// Stable name — this is the ledger key, so renaming one is a deliberate ledger edit.
    name: &'static str,
    src: &'static str,
    /// Why this program is interesting. Not asserted; it is what a reader needs to judge the ledger.
    question: &'static str,
}

/// The probe corpus. Each program is the smallest thing that exercises its question.
const PROBES: &[Probe] = &[
    Probe {
        name: "struct_field_default",
        src: "## A Body has:\n    A mass: Real.\n\n## Main\nLet b be a new Body.\nShow b's mass.\n",
        question: "A field with no `with` value: type-zero, or an absent value?",
    },
    Probe {
        name: "map_missing_key",
        src: "## Main\nLet prices be a new Map of Text to Int.\nShow item \"b\" of prices.\n",
        question: "Reading an absent key: catchable error, panic, or a zeroed value?",
    },
    Probe {
        name: "divide_by_zero",
        src: "## Main\nShow 10 / 0.\n",
        question: "Integer division by zero: one LOGOS error on every tier, or a host panic?",
    },
    Probe {
        name: "seq_index_out_of_bounds",
        src: "## Main\nLet mutable xs be a new Seq of Int.\nPush 1 to xs.\nShow item 5 of xs.\n",
        question: "Indexing past the end: error, or unchecked read?",
    },
    Probe {
        name: "text_length_counts",
        src: "## Main\nShow length of \"h\u{e9}llo\".\n",
        question: "`length of` a Text: bytes (6) or characters (5)? Indexing is by character.",
    },
    Probe {
        name: "int_plus_text",
        src: "## Main\nShow 1 + \"a\".\n",
        question: "Mixed-type `+`: a type error, or silent Int-to-Text coercion?",
    },
    Probe {
        name: "option_some_is_transparent",
        src: "## Main\nLet x be some 42.\nShow x.\n",
        question: "Is `some 42` a distinct value, or erased to the payload?",
    },
    Probe {
        name: "declared_return_type_ignored",
        src: "## To f (n: Int) -> Int:\n    Return \"text\".\n\n## Main\nShow f(1).\n",
        question: "A body contradicting its declared return type: rejected, or run anyway?",
    },
    Probe {
        name: "float_divide_by_zero",
        src: "## Main\nShow 1.0 / 0.0.\n",
        question: "Float division by zero: infinity, an error, or something tier-specific?",
    },
    Probe {
        name: "empty_seq_first_item",
        src: "## Main\nLet xs be a new Seq of Int.\nShow item 1 of xs.\n",
        question: "First item of an empty sequence: error, or a default?",
    },
];

/// Probes whose INTERPRETER-FAMILY tiers (tree-walker, raw VM, AOT-lattice VM, CLI `--interpret`)
/// do not agree today. EXACT SET — see the module header. Empty is the goal; every entry needs the
/// evidence line that put it here.
const KNOWN_DIVERGENCES: &[&str] = &[];

/// Probes whose AOT-Rust answer differs from the tree-walker oracle today. EXACT SET.
///
/// Every entry below was MEASURED by this lock, not assumed, and each is a wrong answer rather than
/// an error — the program runs and produces something different depending on how you invoked it.
/// They fall into three families, which is what makes them fixable rather than a list of oddities:
///
///   1. A LOGOS ERROR BECOMES A HOST PANIC — FIXED, and the family is gone from the ledger. The AOT
///      `main` wrapper installs a panic hook that prints the LOGOS message alone and exits 1, and the
///      runtime words a missing map key and an index past the end as the tree-walker does, so
///      `divide_by_zero`, `map_missing_key`, `seq_index_out_of_bounds` and `empty_seq_first_item`
///      fail in the same words with the same status on every tier.
///
///   2. AN UNINITIALISED FIELD READS DIFFERENTLY: `struct_field_default` — `nothing` on the
///      tree-walker, `0` on AOT. This one needs a RULING (type-zero everywhere, or require `with`),
///      because both tiers are self-consistent and only the choice is missing.
///
///   3. THE TIERS DISAGREE ON WHETHER A PROGRAM IS EVEN LEGAL:
///      `declared_return_type_ignored` — AOT REJECTS `-> Int` with a `Return "text"` body at compile
///      time; the tree-walker runs it and prints `text`. Here AOT is right and the interpreter is
///      permissive, which is the same gap Wave 2 exists to close.
///      `float_divide_by_zero` — NOT IN THE ORIGINAL AUDIT, found by this lock: `1.0 / 0.0` is
///      `inf` on AOT (IEEE 754, what a float user expects) but a hard "Division by zero" ERROR on
///      the tree-walker. The interpreter is applying integer division's rule to floats.
const KNOWN_AOT_DIVERGENCES: &[&str] = &[
    "declared_return_type_ignored",
    "float_divide_by_zero",
    "struct_field_default",
];

/// A tier's answer, reduced to the thing a user actually observes: what was printed, or how it
/// failed — the message AND the exit status, because a user scripting around a LOGOS program sees
/// both. Error TEXT is part of the semantics: "Division by zero" and a Rust backtrace are not the
/// same answer even though both are failures.
///
/// An in-process tier has no exit status of its own; a LOGOS error from it means the CLI exits 1,
/// which is the convention every tier must meet, so it is written explicitly here rather than left
/// implicit and uncompared.
fn signature(output: &str, error: Option<&str>) -> String {
    match error {
        Some(e) => format!("ERROR: [exit 1] {}", e.trim()),
        None => format!("OK: {}", output.trim()),
    }
}

/// The AOT tier's answer, in the same shape — but its exit status is a real process status.
fn aot_signature(result: Result<String, String>) -> String {
    match result {
        Ok(out) => format!("OK: {}", out.trim()),
        Err(e) => format!("ERROR: {}", compact(&e)),
    }
}

/// The AOT tier reports a failure by handing back the whole cargo invocation, so a raw error is
/// either a build log (hundreds of rustc warning lines from the runtime crates) or the runtime shape
/// `Execution failed (status=… code=Some(N)):\nstderr: …\nstdout: …`. Reduce it to the two things
/// that carry semantics — the process exit code and the message the program actually printed —
/// because those are exactly what a user sees and what must match the other tiers.
///
/// An unreadable failure message is a lock nobody acts on, so this is not cosmetic.
fn compact(error: &str) -> String {
    let code = error
        .split_once("code=Some(")
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(code, _)| code.to_string());

    let Some(code) = code else {
        // A build failure, not a run: keep the first line, which names the rustc error.
        return error
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .take(160)
            .collect();
    };

    // The program's own stderr sits between the `stderr:` and `stdout:` markers. With the AOT panic
    // hook installed it holds exactly one line — the LOGOS message.
    let stderr = error
        .split_once("stderr: ")
        .map(|(_, rest)| rest.split("\nstdout: ").next().unwrap_or(rest))
        .unwrap_or("");
    let message = stderr
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .next_back()
        .unwrap_or("(no message)");
    format!(
        "[exit {code}] {}",
        message.chars().take(120).collect::<String>()
    )
}

/// Run one probe on every interpreter-family tier, returning `(tier name, signature)` per tier.
fn interpreter_family(src: &str) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();

    let tw = tw_outcome(src);
    out.push(("tree-walker", signature(&tw.output, tw.error.as_deref())));

    let vm = vm_outcome(src);
    out.push(("vm", signature(&vm.output, vm.error.as_deref())));

    // The AOT lattice, executed on the VM: what `largo build` optimizes, without a cargo build.
    match compile_to_program_optimized(src) {
        Ok(program) => {
            let opt = program_outcome(&program);
            out.push(("vm-optimized", signature(&opt.output, opt.error.as_deref())));
        }
        Err(e) => out.push(("vm-optimized", format!("ERROR: {e:?}"))),
    }

    // The run-path lattice — a different pass list from the AOT one, and the path `largo run
    // --interpret` actually takes.
    let cli = logicaffeine_compile::ui_bridge::interpret_for_ui_sync_with_args(src, &[]);
    out.push((
        "cli-interpret",
        signature(&cli.lines.join("\n"), cli.error.as_deref()),
    ));

    out
}

/// Which probes disagree across the tiers they were run on, with a readable report of the split.
fn split_report(tiers: &[(&'static str, String)]) -> Option<String> {
    let first = &tiers[0].1;
    if tiers.iter().all(|(_, sig)| sig == first) {
        return None;
    }
    Some(
        tiers
            .iter()
            .map(|(name, sig)| format!("      {name:>14} => {sig}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Compare an observed divergence set against a ledger, and produce the actionable message.
fn assert_ledger_matches(observed: Vec<(&'static str, String)>, ledger: &[&str], what: &str) {
    let observed_names: std::collections::BTreeSet<&str> =
        observed.iter().map(|(n, _)| *n).collect();
    let declared: std::collections::BTreeSet<&str> = ledger.iter().copied().collect();

    let new_divergences: Vec<&str> = observed_names.difference(&declared).copied().collect();
    let fixed: Vec<&str> = declared.difference(&observed_names).copied().collect();

    if new_divergences.is_empty() && fixed.is_empty() {
        return;
    }

    let mut msg = format!("\n{what} cross-tier agreement changed.\n");
    if !new_divergences.is_empty() {
        msg.push_str(
            "\n  NEW DIVERGENCE — a program now means different things on different tiers.\n  \
             This is a wrong answer, not an error: fix the tier that is wrong, do not list it here.\n",
        );
        for &name in &new_divergences {
            let detail = observed
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, d)| d.as_str())
                .unwrap_or("");
            let question = PROBES
                .iter()
                .find(|p| p.name == name)
                .map(|p| p.question)
                .unwrap_or("");
            msg.push_str(&format!("\n    {name}  ({question})\n{detail}\n"));
        }
    }
    if !fixed.is_empty() {
        msg.push_str(
            "\n  FIXED — these agree now. Delete them from the ledger; it must match exactly, so a \
             stale entry is as wrong as a missing one.\n",
        );
        for name in &fixed {
            msg.push_str(&format!("    {name}\n"));
        }
    }
    panic!("{msg}");
}

/// The always-on half: every probe must mean the same thing on all four interpreter-family tiers.
/// No cargo build, so this runs on every push.
#[test]
fn every_probe_means_the_same_thing_across_the_interpreter_family() {
    let mut diverging = Vec::new();
    for probe in PROBES {
        let tiers = interpreter_family(probe.src);
        if let Some(report) = split_report(&tiers) {
            diverging.push((probe.name, report));
        }
    }
    assert_ledger_matches(diverging, KNOWN_DIVERGENCES, "The interpreter family's");
}

/// The corpus itself cannot silently shrink: a probe deleted to make the lock pass is the easiest
/// way to fake agreement, so the count is pinned and the names must be unique.
#[test]
fn the_probe_corpus_only_grows() {
    const FLOOR: usize = 10;
    assert!(
        PROBES.len() >= FLOOR,
        "the semantic probe corpus fell to {} (floor {FLOOR}). Deleting a probe hides a divergence \
         instead of fixing it — add probes, never remove them.",
        PROBES.len()
    );
    let mut names: Vec<&str> = PROBES.iter().map(|p| p.name).collect();
    names.sort_unstable();
    let unique = names.len();
    names.dedup();
    assert_eq!(
        names.len(),
        unique,
        "probe names are ledger keys and must be unique"
    );
}

/// The heavy half: the AOT-Rust tier must agree with the tree-walker oracle. `#[ignore]`d because
/// it runs a full cargo build per probe; the gate runs it via `--run-ignored all`.
#[test]
#[ignore = "heavy: full cargo build per probe (run_logos_source) — runs in run-all-tests-fast.sh"]
fn every_probe_means_the_same_thing_on_the_aot_rust_tier() {
    if std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("SKIP semantic probes on AOT: cargo absent");
        return;
    }

    let mut diverging = Vec::new();
    for probe in PROBES {
        let tw = tw_outcome(probe.src);
        let oracle = signature(&tw.output, tw.error.as_deref());
        let aot = aot_signature(run_logos_source(probe.src));
        if aot != oracle {
            diverging.push((
                probe.name,
                format!(
                    "      {:>14} => {oracle}\n      {:>14} => {aot}",
                    "tree-walker", "aot-rust"
                ),
            ));
        }
    }
    assert_ledger_matches(diverging, KNOWN_AOT_DIVERGENCES, "The AOT-Rust tier's");
}

/// A dynamic VM result stays a value even when no static proof can justify
/// versioning the body. C still requires a consistent static signature.
#[test]
fn optional_vm_proofs_do_not_remove_typed_backend_validation() {
    for source in [
        "## To f (n: Int) -> Int:\n    Return \"text\".\n\n## Main\nShow f(1).\n",
        "## To f (n: Int) -> Int:\n    If n is greater than 0:\n        Return \"text\".\n    Return 0.\n\n## Main\nShow f(1).\n",
    ] {
        let reference = vm_outcome(source);
        assert_eq!(reference.error, None);
        assert_eq!(reference.output.lines().collect::<Vec<_>>(), ["text"]);
        let program = compile_to_program_optimized(source).expect("VM optimization preserves dynamic results");
        let optimized = program_outcome(&program);
        assert_eq!(optimized.error, None);
        assert_eq!(optimized.output, reference.output);
        let error = logicaffeine_compile::compile::compile_to_c_via_bytecode(source)
            .expect_err("typed C must still reject an inconsistent declared return");
        assert!(matches!(
            error.kind,
            logicaffeine_compile::error::ParseErrorKind::TypeMismatchDetailed { .. }
        ), "typed C must retain its static diagnostic: {error:?}");
    }
}
