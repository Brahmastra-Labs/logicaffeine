//! ★ LOCK — a RISC-V program holds only the memory it can still reach. ★
//!
//! Every block the RISC-V tier allocates comes from the C tier's collector (`codegen_c::runtime::C_GC_COLLECTOR`,
//! emitted ahead of the RISC-V runtime): a program's memory is bounded by what it still holds, as on the VM, which
//! frees a value when its last reference goes. Before, the runtime allocated and never freed, so binary_trees at
//! depth 16 reached 1.18 GB under qemu and died at the corpus differential's 512 MB address space.
//!
//! Two locks: a program that allocates far more in all than the run's address space but holds little at any time runs
//! to the VM's output, and every program here — trees, records, maps, text, closures, tasks that hold a tree on their
//! own stacks across a yield — prints the VM's output with a collection before every allocation
//! (`LOGOS_GC_STRESS=1`), so any block reachable only from a place the collector does not scan would be freed while
//! still in use and change the output or crash the run.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test riscv_heap_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "support/riscv_runner.rs"]
mod riscv_runner;

use logicaffeine_compile::compile::{compile_to_riscv, vm_outcome};
use logicaffeine_compile::run_vm_concurrent_seeded;

const TREES: &str = r#"## A Tree is either:
    A Leaf.
    A Node with left Tree and right Tree.

## To build (depth: Int) -> Tree:
    If depth equals 0:
        Return a new Leaf.
    Return a new Node with left build(depth - 1) and right build(depth - 1).

## To count (tree: Tree) -> Int:
    Inspect tree:
        When Leaf: Return 1.
        When Node (left, right): Return 1 + count(left) + count(right).

"#;

/// Trees built and dropped: about 2,000 depth-12 trees, near 1.3 GB allocated in all, one tree held at a time.
fn churn() -> String {
    format!(
        r#"{TREES}## Main
Let kept be build(10).
Let mutable total be 0.
Let mutable i be 0.
While i is less than 2000:
    Set total to total + count(build(12)).
    Set i to i + 1.
Show total.
Show count(kept).
"#
    )
}

/// Programs whose every live block must survive a collection at each allocation.
fn programs() -> Vec<(&'static str, String)> {
    vec![
        (
            "trees",
            format!(
                "{TREES}## Main\nLet kept be build(8).\nLet mutable total be 0.\nLet mutable i be 0.\nWhile i is less than 20:\n    Set total to total + count(build(6)) + count(kept).\n    Set i to i + 1.\nShow total.\n"
            ),
        ),
        (
            "records_in_a_list",
            "## A Point has:\n    an x, which is Int.\n    a y, which is Int.\n\n## Main\nLet mutable points be a new Seq of Point.\nLet mutable i be 0.\nWhile i is less than 300:\n    Push a new Point with x i and y i * i to points.\n    Set i to i + 1.\nLet mutable sum be 0.\nRepeat for p in points:\n    Set sum to sum + p's x + p's y.\nShow sum.\nLet last be item 300 of points.\nShow last's x.\n"
                .to_string(),
        ),
        (
            "maps_of_text",
            "## Main\nLet mutable m be a new Map of Text to Int.\nLet mutable i be 0.\nWhile i is less than 200:\n    Let key be \"k{i}\".\n    Set item key of m to i * 3.\n    Set i to i + 1.\nShow length of m.\nShow item \"k7\" of m.\nShow item \"k199\" of m.\n"
                .to_string(),
        ),
        (
            "text_built_by_appending",
            "## Main\nLet mutable s be \"\".\nLet mutable i be 0.\nWhile i is less than 400:\n    Set s to s + \"ab\" + i.\n    Set i to i + 1.\nShow length of s.\n"
                .to_string(),
        ),
        (
            "nested_lists",
            "## Main\nLet mutable rows be a new Seq of Seq of Int.\nLet mutable r be 0.\nWhile r is less than 40:\n    Let mutable row be a new Seq of Int.\n    Let mutable c be 0.\n    While c is less than 40:\n        Push r * c to row.\n        Set c to c + 1.\n    Push row to rows.\n    Set r to r + 1.\nShow item 39 of item 40 of rows.\nShow length of rows.\n"
                .to_string(),
        ),
        (
            "tasks_hold_trees_across_yields",
            format!(
                "{TREES}## To grower (depth: Int, out: Int):\n    Let kept be build(depth).\n    Let mutable i be 0.\n    While i is less than 3:\n        Sleep 1.\n        Let v be count(build(depth)) + count(kept).\n        Send v into out.\n        Set i to i + 1.\n\n## Main\nLet out be a Pipe of Int.\nLaunch a task to grower with 6 and out.\nLaunch a task to grower with 7 and out.\nLet mutable total be 0.\nLet mutable k be 0.\nWhile k is less than 6:\n    Receive v from out.\n    Set total to total + v.\n    Set k to k + 1.\nShow total.\n"
            ),
        ),
    ]
}

/// The VM's output for `source`, which runs to its end; a program that launches tasks runs under the VM's scheduler
/// at seed 0, the seed the RISC-V run reads from `LOGOS_SEED`.
fn vm(label: &str, source: &str) -> String {
    if source.contains("Launch a task") {
        let run = run_vm_concurrent_seeded(source, 0);
        assert!(run.error.is_none(), "{label}: the VM fails: {:?}", run.error);
        return run.lines.iter().map(|line| format!("{line}\n")).collect();
    }
    let outcome = vm_outcome(source);
    assert!(outcome.error.is_none(), "{label}: the VM fails: {:?}", outcome.error);
    outcome.output
}

fn tools() -> bool {
    common::require_tool("riscv64-linux-gnu-gcc") && common::require_tool("qemu-riscv64")
}

#[test]
fn memory_churned_past_the_address_space_is_reclaimed() {
    let source = churn();
    let expected = vm("churn", &source);
    let asm = compile_to_riscv(&source).unwrap_or_else(|e| panic!("churn: {e:?}"));
    if !tools() {
        return;
    }
    let linked = riscv_runner::link(&asm, "churn");
    let run = riscv_runner::run(&linked, &[], &[("LOGOS_HEAP_REPORT", "1".to_string())]);
    assert_eq!(run.code, Some(0), "churn: {}", run.describe());
    assert_eq!(run.stdout, expected, "churn: RISC-V disagrees with the VM");
    let report = run
        .stderr
        .lines()
        .find_map(|l| l.strip_prefix("logos heap: "))
        .unwrap_or_else(|| panic!("churn: no heap report on stderr:\n{}", run.stderr));
    let field = |name: &str| -> u64 {
        report
            .split_whitespace()
            .find_map(|kv| kv.strip_prefix(&format!("{name}=")))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("churn: no `{name}` in `{report}`"))
    };
    let (allocated, high) = (field("allocated"), field("high"));
    let cap = riscv_runner::RUN_CAP_MB << 20;
    assert!(
        allocated > cap,
        "churn: the program allocates {allocated} bytes in all, more than the {cap}-byte address space it runs in"
    );
    assert!(
        high < cap / 4,
        "churn: at most {high} bytes held at once: the collector keeps the heap near what the program reaches"
    );
}

#[test]
fn every_reachable_block_survives_a_collection_at_each_allocation() {
    let mut bad = Vec::new();
    for (label, source) in programs() {
        let expected = vm(label, &source);
        let asm = compile_to_riscv(&source).unwrap_or_else(|e| panic!("{label}: {e:?}"));
        if !tools() {
            return;
        }
        let linked = riscv_runner::link(&asm, label);
        let run = riscv_runner::run(
            &linked,
            &[],
            &[("LOGOS_GC_STRESS", "1".to_string()), ("LOGOS_SEED", "0".to_string())],
        );
        if run.code != Some(0) || run.stdout != expected || !run.stderr.is_empty() {
            bad.push(format!(
                "{label}: printed {:?}, the VM {:?} ({})",
                run.stdout,
                expected,
                run.describe()
            ));
        }
    }
    assert!(bad.is_empty(), "under LOGOS_GC_STRESS=1:\n{}", bad.join("\n"));
}
