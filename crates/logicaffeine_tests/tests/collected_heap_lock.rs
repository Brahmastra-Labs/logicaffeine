//! ★ LOCK — on every collected tier, a program's heap holds what the program can still reach, and no more. ★
//!
//! The compiled tiers whose runtime carries the shared collector (`C_GC_COLLECTOR`): RISC-V, the bytecode→C
//! walker and the C tier. Every row is a loop that makes and drops `n` values of one kind: a list, a list grown
//! by pushes, a list of lists, a map, a set, a record and its list field, Text by concatenation and by
//! appending, a closure, an element popped out of a list, the locals of a called function, a copy. Each row runs
//! at `n` and at eight times `n` with `LOGOS_HEAP_REPORT` set, and the runtime's report line,
//! `logos heap: live=<bytes> high=<bytes> collections=<count> allocated=<bytes> freed=<bytes>`, gives the most
//! heap it ever held at once. That high-water must not grow with `n`: a value nothing can reach goes back to the
//! allocator. Each run's `n` is large enough that the collector runs at least twice, so the small run measures
//! the collected regime and not the heap before its first collection. Each run prints what the VM prints, so a
//! collector that frees a value the program still holds fails here too.
//!
//! Gated on each tier's toolchain (`riscv64-linux-gnu-gcc` + `qemu-riscv64`, `gcc`): an absent tool is a
//! recorded skip, and a failure where `LOGOS_REQUIRE_TOOLS` names it.

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "support/riscv_runner.rs"]
mod riscv_runner;

use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::compile::{compile_to_c, compile_to_c_via_bytecode, compile_to_riscv, vm_outcome};

/// How far the large run's high-water may exceed the small run's: half again, plus the allocator's rounding.
fn within_slack(small: u64, large: u64) -> bool {
    large <= small + small / 2 + 64 * 1024
}

/// `(label, loop count of the small run, program)`: a program whose `{n}` is the loop count.
const ROWS: &[(&str, usize, &str)] = &[
    (
        "list_literal",
        400_000,
        "## Main\nLet mutable total be 0.\nRepeat for i from 1 to {n}:\n    Let xs be [i, i + 1, i + 2].\n    \
         Set total to total + length of xs + item 2 of xs.\nShow total.\n",
    ),
    (
        "list_grown_by_pushes",
        60_000,
        "## Main\nLet mutable total be 0.\nRepeat for i from 1 to {n}:\n    Let mutable xs be a new Seq of Int.\n    \
         Repeat for j from 1 to 20:\n        Push i + j to xs.\n    Set total to total + item 20 of xs.\nShow total.\n",
    ),
    (
        "list_of_lists",
        150_000,
        "## Main\nLet mutable total be 0.\nRepeat for i from 1 to {n}:\n    \
         Let mutable outer be a new Seq of Seq of Int.\n    \
         Push [i] to outer.\n    Push [i, i] to outer.\n    Let inner be item 2 of outer.\n    \
         Set total to total + length of inner.\nShow total.\n",
    ),
    (
        "map",
        150_000,
        "## Main\nLet mutable total be 0.\nRepeat for i from 1 to {n}:\n    \
         Let mutable m be a new Map of Int to Int.\n    \
         Set item i of m to i.\n    Set item (i + 1) of m to 2.\n    Set total to total + item i of m.\nShow total.\n",
    ),
    (
        "set",
        250_000,
        "## Main\nLet mutable total be 0.\nRepeat for i from 1 to {n}:\n    Let mutable s be a new Set of Int.\n    \
         Add i to s.\n    Add i + 1 to s.\n    Set total to total + length of s.\nShow total.\n",
    ),
    (
        "record_with_a_list_field",
        150_000,
        "## A Bag has:\n    An items: Seq of Int.\n    A label: Text.\n\n## Main\nLet mutable total be 0.\n\
         Repeat for i from 1 to {n}:\n    Let mutable b be a new Bag with items [i, i] and label \"x\".\n    \
         Push i to b's items.\n    Set total to total + length of b's items.\nShow total.\n",
    ),
    (
        "text_by_concatenation",
        500_000,
        "## Main\nLet mutable total be 0.\nRepeat for i from 1 to {n}:\n    Let t be \"value \" + i.\n    \
         Set total to total + length of t.\nShow total.\n",
    ),
    (
        "text_by_appending",
        150_000,
        "## Main\nLet mutable total be 0.\nRepeat for i from 1 to {n}:\n    Let mutable s be \"\".\n    \
         Repeat for j from 1 to 8:\n        Set s to s + \"ab\".\n    Set total to total + length of s.\nShow total.\n",
    ),
    (
        "closure",
        600_000,
        "## Main\nLet mutable total be 0.\nRepeat for i from 1 to {n}:\n    Let f be (x: Int) -> x + i.\n    \
         Set total to total + f(1).\nShow total.\n",
    ),
    (
        "popped_element",
        150_000,
        "## Main\nLet mutable total be 0.\nRepeat for i from 1 to {n}:\n    Let mutable xs be [[i], [i, i]].\n    \
         Pop from xs into v.\n    Set total to total + length of v.\nShow total.\n",
    ),
    (
        "locals_of_a_called_function",
        100_000,
        "## To build (i: Int) -> Int:\n    Let mutable xs be a new Seq of Int.\n    \
         Repeat for j from 1 to 10:\n        \
         Push i to xs.\n    Let t be \"n\" + i.\n    Return length of xs + length of t.\n\n## Main\n\
         Let mutable total be 0.\n\
         Repeat for i from 1 to {n}:\n    Set total to total + build(i).\nShow total.\n",
    ),
    (
        "copy",
        200_000,
        "## A Bag has:\n    An items: Seq of Int.\n\n## Main\nLet mutable total be 0.\n\
         Let b be a new Bag with items [1, 2, 3].\n\
         Repeat for i from 1 to {n}:\n    Let mutable c be copy of b.\n    Push i to c's items.\n    \
         Set total to total + length of c's items.\nShow total.\n",
    ),
];

fn program(template: &str, n: usize) -> String {
    template.replace("{n}", &n.to_string())
}

/// A tier whose runtime carries the shared collector.
#[derive(Clone, Copy, Debug)]
enum Tier {
    RiscV,
    BytecodeC,
    C,
}

const TIERS: [Tier; 3] = [Tier::RiscV, Tier::BytecodeC, Tier::C];

impl Tier {
    fn available(self) -> bool {
        match self {
            Tier::RiscV => common::require_tool("riscv64-linux-gnu-gcc") && common::require_tool("qemu-riscv64"),
            Tier::BytecodeC | Tier::C => common::require_tool("gcc"),
        }
    }

    fn compile(self, src: &str) -> Result<String, String> {
        match self {
            Tier::RiscV => compile_to_riscv(src),
            Tier::BytecodeC => compile_to_c_via_bytecode(src),
            Tier::C => compile_to_c(src),
        }
        .map_err(|e| format!("does not compile: {e:?}"))
    }

    /// The program's stdout and stderr, with the heap report on, and whether it exited 0.
    fn run(self, label: &str, code: &str) -> Result<(String, String, bool), String> {
        match self {
            Tier::RiscV => {
                let env = [("LOGOS_HEAP_REPORT", "1".to_string())];
                let run = riscv_runner::run(&riscv_runner::link(code, label), &[], &env);
                Ok((run.stdout, run.stderr, run.success))
            }
            Tier::BytecodeC | Tier::C => {
                let dir = Scratch::new(label);
                let (c, bin) = (dir.0.join("prog.c"), dir.0.join("prog"));
                std::fs::write(&c, code).map_err(|e| e.to_string())?;
                let built = Command::new("gcc")
                    .args(["-O2", "-o"])
                    .arg(&bin)
                    .arg(&c)
                    .arg("-lm")
                    .output()
                    .map_err(|e| e.to_string())?;
                if !built.status.success() {
                    return Err(format!("gcc failed: {}", String::from_utf8_lossy(&built.stderr)));
                }
                let out = Command::new(&bin).env("LOGOS_HEAP_REPORT", "1").output().map_err(|e| e.to_string())?;
                let text = |b: &[u8]| String::from_utf8_lossy(b).to_string();
                Ok((text(&out.stdout), text(&out.stderr), out.status.success()))
            }
        }
    }
}

/// A temp directory of its own, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Scratch {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("logos_collected_heap_{}_{n}_{label}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The report line's `key=` value.
fn report_value(stderr: &str, key: &str) -> Result<u64, String> {
    let lines: Vec<&str> = stderr.lines().filter(|l| l.starts_with("logos heap: ")).collect();
    let [line] = lines[..] else {
        return Err(format!("the runtime reports {} heap lines, not one: {stderr:?}", lines.len()));
    };
    line.split_whitespace()
        .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('=')?.parse().ok())
        .ok_or_else(|| format!("the heap report has no `{key}=`: {line:?}"))
}

/// One run of `src` on `tier`, held to the VM's output: its high-water and its collection count.
fn measure(tier: Tier, label: &str, src: &str, expected: &str) -> Result<(u64, u64), String> {
    let code = tier.compile(src)?;
    let (stdout, stderr, ok) = tier.run(label, &code)?;
    if !ok || stdout.trim() != expected.trim() {
        return Err(format!("wrote {stdout:?} / {stderr:?} (success {ok}); the VM wrote {expected:?}"));
    }
    Ok((report_value(&stderr, "high")?, report_value(&stderr, "collections")?))
}

/// Why `row` keeps what it drops on `tier`, if it does.
fn row_verdict(tier: Tier, (label, n, template): &(&str, usize, &str)) -> Option<String> {
    let (small_src, large_src) = (program(template, *n), program(template, 8 * n));
    let at = |src: &str| -> Result<(u64, u64), String> {
        let expected = vm_outcome(src).output;
        measure(tier, label, src, &expected)
    };
    match (at(&small_src), at(&large_src)) {
        (Err(why), _) | (_, Err(why)) => Some(why),
        (Ok((_, collections)), _) if collections < 2 => Some(format!(
            "the collector runs {collections} times at {n} iterations: raise the row's loop count until it runs twice"
        )),
        (Ok((small, _)), Ok((large, _))) if !within_slack(small, large) => Some(format!(
            "the heap's high-water grows with the loop count: {small} bytes at {n} iterations, {large} at {}",
            8 * n
        )),
        _ => None,
    }
}

/// Every call of the C library's allocator in `c` outside the collector's own section
/// (`LC_GC_COLLECTOR_BEGIN` … `LC_GC_COLLECTOR_END`), as the line it is on: a block the collector does not know
/// of could hold the only pointer to one it does, and a block it does know of must never reach `free`.
fn stray_allocations(c: &str) -> Vec<String> {
    let (open, close) = ("/* LC_GC_COLLECTOR_BEGIN */", "/* LC_GC_COLLECTOR_END */");
    let section = c.find(open).zip(c.find(close)).map(|(lo, hi)| (lo, hi + close.len()));
    let mut found = Vec::new();
    for name in ["malloc", "calloc", "realloc", "strdup", "strndup", "free"] {
        let call = format!("{name}(");
        for (at, _) in c.match_indices(&call) {
            let named_alone = !c[..at].chars().next_back().is_some_and(|ch| ch.is_alphanumeric() || ch == '_');
            if named_alone && !section.is_some_and(|(lo, hi)| (lo..hi).contains(&at)) {
                let line = c[..at].rfind('\n').map_or(0, |i| i + 1);
                let end = c[at..].find('\n').map_or(c.len(), |i| at + i);
                found.push(c[line..end].trim().to_string());
            }
        }
    }
    found
}

#[test]
fn the_riscv_runtime_allocates_only_through_the_collector() {
    let runtime = logicaffeine_compile::codegen_riscv::RISCV_RUNTIME_C;
    assert!(runtime.contains("/* LC_GC_COLLECTOR_BEGIN */"), "the RISC-V runtime does not carry the shared collector");
    let stray = stray_allocations(runtime);
    assert!(
        stray.is_empty(),
        "the RISC-V runtime calls the C library's allocator {} times outside the collector:\n  {}",
        stray.len(),
        stray.join("\n  ")
    );
}

#[test]
fn every_heap_row_compiles_on_every_collected_tier() {
    let mut refused = Vec::new();
    for tier in TIERS {
        for (label, n, template) in ROWS {
            if let Err(why) = tier.compile(&program(template, *n)) {
                refused.push(format!("{tier:?} {label}: {why}"));
            }
        }
    }
    assert!(refused.is_empty(), "{} heap rows do not compile:\n  {}", refused.len(), refused.join("\n  "));
}

#[test]
fn a_dropped_value_goes_back_to_the_allocator_on_every_collected_tier() {
    let jobs: Vec<(Tier, &(&str, usize, &str))> =
        TIERS.into_iter().filter(|t| t.available()).flat_map(|t| ROWS.iter().map(move |r| (t, r))).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let verdicts: Vec<std::sync::Mutex<Option<String>>> = jobs.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(&(tier, row)) = jobs.get(i) else { break };
                let verdict = row_verdict(tier, row).map(|why| format!("{tier:?} {}: {why}", row.0));
                *verdicts[i].lock().unwrap() = verdict;
            });
        }
    });
    let failed: Vec<String> = verdicts.into_iter().filter_map(|v| v.into_inner().unwrap()).collect();
    assert!(
        failed.is_empty(),
        "{} of {} heap rows keep what they dropped:\n  {}",
        failed.len(),
        jobs.len(),
        failed.join("\n  ")
    );
}
