//! ★ LOCK — a loop region counts entries, not crossings, and gives up only when it keeps failing. ★
//!
//! A hot loop runs as a native region entered at its back-edge. An entry whose registers hold kinds the region was not
//! compiled for (an `Int` grown past 64 bits, where the region adds machine words) cannot run native, and nothing
//! inside the entry will change that: the region checks again only once a register that missed holds another kind. So
//! one bad entry is one miss, where it was one miss for every crossing of the head and eight of them failed the
//! region for good.
//! A region is demoted when its misses reach the demotion threshold and outnumber the entries it completed, counted
//! over a window that halves both counts as it fills, so a loop that ran native for many entries survives a few bad
//! ones, and a loop that never completes still goes back to bytecode.
//!
//! Partial compilation (Whaley, OOPSLA 2001): while a loop warms, the VM counts which way each of its branches goes.
//! A loop whose body holds an op no region lowers (a `Show`) used to stay on bytecode whole. Now, when that op sits on
//! a path the warm-up never took (a branch direction that never ran while the branch ran at least 32 times the other
//! way), the region is compiled without it: the cold path becomes an exit that hands the VM the loop's registers at
//! the cold code's first op, the VM runs the cold code, and the next crossing of the head enters the region again. A
//! cold path that turns out to run on a large share of the iterations demotes the region, so a phase change never
//! leaves a loop paying an exit per iteration. A branch seen too few times while warming is not cut.
//!
//! The report (`JitBackendOptimizationTrace::regions`, `largo opts --jit`'s `regions`) gives each compiled region's
//! entries, completions, exits, side exits, guard misses, cold entries, cold exits and whether it was demoted. Every
//! program prints exactly what the tree-walker prints.

#![cfg(all(target_arch = "x86_64", not(target_os = "windows")))]

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr, stdout};
use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_jit::{optimization_trace_jit, JitBackendOptimizationTrace, RegionReport};

fn run(source: &str, argument: &str) -> JitBackendOptimizationTrace {
    let args = ["logos".to_string(), argument.to_string()];
    let traced = optimization_trace_jit(source, &args);
    let tree = tw_outcome_with_args(source, &args);
    assert_eq!(traced.outcome.error, tree.error, "the JIT and the tree-walker disagree on the error:\n{source}");
    assert_eq!(
        traced.outcome.output.trim(),
        tree.output.trim(),
        "the JIT and the tree-walker disagree on the output for {argument}:\n{source}"
    );
    traced
}

/// The one region the program compiles: its inner loop (the outer loop shows, which no region lowers).
fn the_region(traced: &JitBackendOptimizationTrace) -> &RegionReport {
    assert_eq!(traced.regions.len(), 1, "one compiled region: {:#?}", traced.regions);
    &traced.regions[0]
}

/// Forty rounds of an inner loop adding `v` n times modulo a prime, `v` an integer past 64 bits (10^22) in the rounds
/// from `from` to `to` and 1 in the others.
fn rounds(from: i64, to: i64) -> String {
    format!(
        "## Main
Let n be parseInt(item 2 of args()).
Let mutable round be 1.
While round is at most 40:
    Let mutable v be 1.
    If round is at least {from} and round is at most {to}:
        Set v to 100000000000 * 100000000000.
    Let mutable acc be 0.
    Let mutable i be 0.
    While i is less than n:
        Set acc to (acc + v) % 1000003.
        Set i to i + 1.
    Show acc.
    Set round to round + 1.
"
    )
}

#[test]
fn an_entry_whose_kinds_missed_is_one_miss_however_many_times_it_crosses_the_head() {
    let traced = run(&rounds(21, 24), "1000");
    let region = the_region(&traced);
    assert_eq!(region.guard_misses, 4, "four rounds hold a wide integer, each one entry: {region:#?}");
    assert!(!region.demoted, "four bad entries among thirty-six good ones: {region:#?}");
    assert!(
        region.completions >= 35,
        "every round after the first compiles runs native, the rounds after the wide ones too: {region:#?}"
    );
}

#[test]
fn a_region_that_completed_many_entries_survives_a_run_of_bad_ones() {
    let traced = run(&rounds(25, 33), "1000");
    let region = the_region(&traced);
    assert_eq!(region.guard_misses, 9, "nine wide rounds: {region:#?}");
    assert!(!region.demoted, "nine misses against twenty-three completions: {region:#?}");
    assert!(region.completions >= 30, "the seven rounds after them run native again: {region:#?}");
}

#[test]
fn a_region_that_never_completes_is_demoted_after_the_threshold() {
    let traced = run(&rounds(2, 40), "1000");
    let region = the_region(&traced);
    assert!(region.demoted, "every entry after the first misses: {region:#?}");
    assert_eq!(
        region.guard_misses,
        logicaffeine_compile::vm::REGION_DEMOTE_AFTER as u64,
        "demoted at the threshold, one miss per entry, and never tried again: {region:#?}"
    );
    assert!(region.completions <= 1, "only the round that compiled it: {region:#?}");
}

#[test]
fn every_entry_is_counted_once() {
    let traced = run(&rounds(21, 24), "1000");
    let region = the_region(&traced);
    assert_eq!(
        region.entries,
        region.completions + region.exits + region.deopts,
        "every native entry ends one way: {region:#?}"
    );
    assert_eq!(region.exits + region.deopts, 0, "a sum of words modulo a prime never leaves early: {region:#?}");
}

#[test]
fn largo_opts_lists_each_region_with_its_counts() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("rounds.lg"), rounds(21, 24)).expect("write source");
    let out = largo_in(dir.path(), &["opts", "rounds.lg", "--jit", "--", "1000"]);
    assert!(out.status.success(), "`largo opts --jit` succeeds:\n{}", stderr(&out));
    let text = stdout(&out);
    let line = text
        .lines()
        .find(|l| l.contains("region at pc") && l.contains("4 guard misses"))
        .unwrap_or_else(|| panic!("a line gives the region's guard misses:\n{text}"));
    assert!(!line.contains("demoted"), "the region was not demoted: {line}");

    let out = largo_in(dir.path(), &["opts", "rounds.lg", "--jit", "--json", "--", "1000"]);
    assert!(out.status.success(), "`largo opts --jit --json` succeeds:\n{}", stderr(&out));
    let report: serde_json::Value = serde_json::from_str(stdout(&out).trim()).expect("the report is JSON");
    let regions = report["jit_native"]["regions"].as_array().expect("a list of compiled regions");
    assert_eq!(regions.len(), 1, "{report}");
    assert_eq!(regions[0]["guard_misses"], 4, "{report}");
    assert_eq!(regions[0]["demoted"], false, "{report}");
    let (entries, completions) = (regions[0]["entries"].as_u64(), regions[0]["completions"].as_u64());
    assert!(entries.is_some_and(|e| e >= 35) && entries == completions, "{report}");
}

#[test]
fn largo_opts_counts_one_of_a_thing_in_the_singular() {
    let dir = tempfile::tempdir().expect("temp dir");
    let report = |name: &str, source: &str, args: &[&str]| -> String {
        std::fs::write(dir.path().join(name), source).expect("write source");
        let mut argv = vec!["opts", name, "--jit", "--"];
        argv.extend_from_slice(args);
        let out = largo_in(dir.path(), &argv);
        assert!(out.status.success(), "`largo opts --jit` succeeds:\n{}", stderr(&out));
        stdout(&out)
    };
    let divisor = report(
        "divisor.lg",
        "## Main\nLet k be parseInt(item 2 of args()).\nLet n be parseInt(item 3 of args()).\nLet mutable sum be 0.\n\
         Let mutable i be 0 - n.\nWhile i is less than n:\n    Set sum to sum + (i * 7919) % k.\n    Set i to i + 1.\nShow sum.\n",
        &["7", "1000000"],
    );
    let spec = divisor.lines().find(|l| l.contains("specialized on")).unwrap_or_else(|| panic!("{divisor}"));
    assert!(spec.contains("; 1 entry, 0 value-guard misses"), "one entry is one entry: {spec}");

    let plain = report(
        "plain.lg",
        "## Main\nLet n be parseInt(item 2 of args()).\nLet mutable acc be 0.\nLet mutable i be 0.\n\
         While i is less than n:\n    Set acc to (acc + i * 3) % 1000003.\n    Set i to i + 1.\nShow acc.\n",
        &["100000"],
    );
    let region = plain.lines().find(|l| l.contains("region at pc")).unwrap_or_else(|| panic!("{plain}"));
    assert!(region.contains(": 1 entry (1 completed, 0 exits, 0 side exits), 0 guard misses"), "{region}");

    let once = report("once.lg", &rare_show("i is 5000"), &["10000"]);
    let region = once.lines().find(|l| l.contains("region at pc")).unwrap_or_else(|| panic!("{once}"));
    assert!(region.contains(" 1 exit,") && region.contains("1 cold entry (1 cold exit)"), "{region}");
    assert!(!format!("{divisor}{plain}{once}").contains("1 entries"), "no count of one in the plural");
}

/// A loop of `n` iterations whose rare path (`taken` decides when) shows a line, which no region lowers.
fn rare_show(taken: &str) -> String {
    format!(
        "## Main
Let n be parseInt(item 2 of args()).
Let mutable acc be 0.
Let mutable i be 0.
While i is less than n:
    Set acc to (acc + i * 7) % 1000003.
    If {taken}:
        Show \"at {{i}}: {{acc}}\".
    Set i to i + 1.
Show acc.
"
    )
}

#[test]
fn a_rare_path_the_jit_cannot_lower_leaves_the_rest_of_the_loop_native() {
    let traced = run(&rare_show("i % 2500 is 2499"), "10000");
    let region = the_region(&traced);
    assert_eq!(region.cold_entries.len(), 1, "the Show's branch is cut: {region:#?}");
    assert_eq!(region.cold_exits, 4, "the cold path ran at 2499, 4999, 7499 and 9999: {region:#?}");
    assert!(!region.demoted, "four exits in ten thousand iterations: {region:#?}");
    assert_eq!(region.entries, region.completions + region.exits + region.deopts, "{region:#?}");
}

#[test]
fn a_cold_path_that_runs_on_most_iterations_demotes_the_region() {
    let traced = run(&rare_show("i is at least 200"), "2000");
    let region = the_region(&traced);
    assert_eq!(region.cold_entries.len(), 1, "cold while the loop warmed (i below 100): {region:#?}");
    assert!(region.demoted, "from i = 200 on every iteration leaves the region: {region:#?}");
    assert!(
        region.cold_exits <= 2 * logicaffeine_compile::vm::REGION_DEMOTE_AFTER as u64,
        "demoted after a handful of exits, not one per remaining iteration: {region:#?}"
    );
}

#[test]
fn a_branch_seen_too_few_times_while_warming_is_not_cut() {
    // The inner branch runs on one iteration in ten, ten times in the hundred crossings before the loop compiles:
    // too little evidence that its taken side is cold, so the Show stays in the region and keeps it on bytecode.
    let source = "## Main
Let n be parseInt(item 2 of args()).
Let mutable acc be 0.
Let mutable i be 0.
While i is less than n:
    Set acc to (acc + i * 7) % 1000003.
    If i % 10 is 0:
        If i is at least 5000:
            Show \"at {i}\".
    Set i to i + 1.
Show acc.
";
    let traced = run(source, "6000");
    assert!(traced.regions.is_empty(), "no region compiles around the uncut Show: {:#?}", traced.regions);
}

#[test]
fn largo_opts_names_the_cold_entries_and_their_exits() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("rare.lg"), rare_show("i % 2500 is 2499")).expect("write source");
    let out = largo_in(dir.path(), &["opts", "rare.lg", "--jit", "--", "10000"]);
    assert!(out.status.success(), "`largo opts --jit` succeeds:\n{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.lines().any(|l| l.contains("region at pc") && l.contains("1 cold entry") && l.contains("4 cold exits")),
        "the region's line names its cold entry and its exits:\n{text}"
    );
    let out = largo_in(dir.path(), &["opts", "rare.lg", "--jit", "--json", "--", "10000"]);
    let report: serde_json::Value = serde_json::from_str(stdout(&out).trim()).expect("the report is JSON");
    let region = &report["jit_native"]["regions"][0];
    assert_eq!(region["cold_entries"].as_array().map(Vec::len), Some(1), "{report}");
    assert_eq!(region["cold_exits"], 4, "{report}");
}

/// A loop whose rare branch shows a line every thousand iterations: a `Show` no region lowers.
const RARE_SHOW: &str = "## Main
Let n be parseInt(item 2 of args()).
Let mutable total be 0.
Let mutable i be 0.
While i is less than n:
    If i % 1000 is 999:
        Show \"tick {i}\".
    Set total to (total + i * 3) % 1000003.
    Set i to i + 1.
Show total.
";

/// Partial method compilation (Whaley, OOPSLA 2001): the region compiles only the edges the loop took while it
/// warmed. The rare branch, never taken in the first hundred iterations, is cut: taking it leaves native code at the
/// branch, the VM runs the `Show`, and the next back-edge enters the region again. The loop runs native, where the
/// `Show` kept the whole loop on bytecode, and each tick is one exit.
#[test]
fn a_rare_path_the_jit_cannot_lower_no_longer_keeps_the_loop_on_bytecode() {
    let traced = run(RARE_SHOW, "10000");
    let region = the_region(&traced);
    assert_eq!(region.exits, 10, "one exit at each of the ten ticks: {region:#?}");
    assert_eq!(region.completions, 1, "the loop ends native once: {region:#?}");
    assert_eq!(region.entries, region.exits + region.completions, "every entry ends one way: {region:#?}");
    assert!(!region.demoted, "a rare exit is no miss: {region:#?}");
}

/// The same loop whose rare branch is never taken at all runs native start to finish: one entry, no exit.
#[test]
fn a_cut_edge_never_taken_costs_nothing() {
    let traced = run(RARE_SHOW, "999");
    let region = the_region(&traced);
    assert_eq!((region.entries, region.exits, region.completions), (1, 0, 1), "{region:#?}");
}

/// A list the loop builds every iteration and reads on its hot path, escaping only on the rare one (Whaley's second
/// optimization: the rare path, not the hot one, pays for the object).
const RARE_ESCAPE: &str = "## Main
Let n be parseInt(item 2 of args()).
Let mutable acc be 0.
Let mutable i be 0.
While i is less than n:
    Let pair be [i, i * 2].
    Set acc to (acc + item 1 of pair + item 2 of pair) % 1000003.
    If i % 2500 is 2499:
        Show pair.
    Set i to i + 1.
Show acc.
";

#[test]
fn a_list_that_escapes_only_on_the_rare_path_is_built_for_it_there() {
    let traced = run(RARE_ESCAPE, "10000");
    let region = the_region(&traced);
    assert_eq!(region.cold_entries.len(), 1, "the Show's branch is cut: {region:#?}");
    assert_eq!(region.cold_exits, 4, "each rare Show is one exit, its list handed to the VM: {region:#?}");
    assert!(!region.demoted, "{region:#?}");
    assert!(traced.outcome.output.contains("[2499, 4998]"), "the list the rare path shows is the loop's own");
}

#[test]
fn a_list_built_every_iteration_compiles_into_one_reused_buffer() {
    let source = RARE_ESCAPE.replace("    If i % 2500 is 2499:\n        Show pair.\n", "");
    let traced = run(&source, "10000");
    let region = the_region(&traced);
    assert!(region.cold_entries.is_empty(), "nothing is cut: the whole loop lowers: {region:#?}");
    assert_eq!((region.exits, region.deopts), (0, 0), "{region:#?}");
}

#[test]
fn a_list_the_rare_path_keeps_is_not_overwritten_by_the_next_iteration() {
    // The rare path keeps the list the region refills in place: the next entry copies it before refilling, so every
    // kept pair is the one its iteration built.
    let source = "## Main
Let n be parseInt(item 2 of args()).
Let mutable acc be 0.
Let mutable kept be a new Seq of (Seq of Int).
Let mutable i be 0.
While i is less than n:
    Let pair be [i, i * 2].
    Set acc to (acc + item 1 of pair + item 2 of pair) % 1000003.
    If i % 2500 is 2499:
        Push pair to kept.
    Set i to i + 1.
Show acc.
Show kept.
";
    let traced = run(source, "10000");
    let region = the_region(&traced);
    assert_eq!(region.cold_exits, 4, "{region:#?}");
    assert!(
        traced.outcome.output.contains("[[2499, 4998], [4999, 9998], [7499, 14998], [9999, 19998]]"),
        "every kept pair is its own iteration's: {}",
        traced.outcome.output
    );
}
