//! ★ LOCK — the RISC-V tier's emitted code executes exactly the instructions its budget names. ★
//!
//! Each row is a program the tier compiles, run once under `qemu-riscv64` with its translation blocks
//! traced (`-d in_asm,exec,nochain`) and the trace filtered (`-dfilter`) to the address range the link map
//! gives the emitted object's `.text`. The count is the guest instructions the emitted code itself executes:
//! the runtime C and libc belong to the toolchain, and the emitter decides the rest, so the count is the
//! same on every machine that runs the lock.
//!
//! A row that costs more than its budget is a regression of the emitter: find the instructions it added to
//! the path the program runs and take them out (prove the check away, hoist it, or move it to a cold branch),
//! never raise the budget. A row that costs less has its budget lowered to the new count in the same change,
//! so every budget is what the code costs now and the next regression shows at its first instruction.
//!
//! The behavioural half is gated on the RISC-V toolchain (`riscv64-linux-gnu-gcc` + `qemu-riscv64`): an
//! absent tool is a recorded skip, and a failure where `LOGOS_REQUIRE_TOOLS` names it. The static half, that
//! every row compiles, always runs.

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "support/riscv_runner.rs"]
mod riscv_runner;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use logicaffeine_compile::codegen_riscv::RISCV_RUNTIME_C;
use logicaffeine_compile::compile::compile_to_riscv;

fn have_toolchain() -> bool {
    common::require_tool("riscv64-linux-gnu-gcc") && common::require_tool("qemu-riscv64")
}

/// The benchmark programs' root, `benchmarks/programs`.
const BENCHMARK_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/programs");

/// `(benchmark/variant, workload size, instructions)`: a benchmark program run at one size.
const BENCHMARK_BUDGETS: &[(&str, &str, u64)] = &[
    ("ackermann/main", "3", 77788),
    ("ackermann/interp", "8", 77777),
    ("analytics/main", "16", 2132431),
    ("array_fill/main", "16", 1334),
    ("array_reverse/main", "16", 1917),
    ("array_reverse/interp", "16", 1125109),
    ("binary_trees/main", "3", 310126),
    ("bubble_sort/main", "16", 9025),
    ("coins/main", "16", 3748),
    ("coins/interp", "16", 452991),
    ("collatz/main", "16", 6196),
    ("collect/main", "16", 2505),
    ("collect/interp", "16", 151088),
    ("counting_sort/main", "16", 60021),
    ("counting_sort/interp", "16", 1847149),
    ("fannkuch/main", "3", 2997),
    ("fib/main", "16", 83034),
    ("fib_iterative/main", "16", 634),
    ("gcd/main", "16", 14543),
    ("graph_bfs/main", "16", 16337),
    ("graph_bfs/interp", "16", 998360),
    ("heap_sort/main", "16", 9947),
    ("heap_sort/interp", "16", 1545394),
    ("histogram/main", "16", 97598),
    ("histogram/interp", "16", 1006204),
    ("knapsack/main", "16", 83935),
    ("loop_sum/main", "16", 583),
    ("mandelbrot/main", "16", 476196),
    ("matrix_mult/main", "16", 444884),
    ("mergesort/main", "16", 8962),
    ("mergesort/interp", "16", 1076523),
    ("nbody/main", "16", 51886),
    ("nqueens/main", "8", 267538),
    ("pi_leibniz/main", "16", 830),
    ("prefix_sum/main", "16", 2511),
    ("prefix_sum/interp", "16", 1540039),
    ("primes/main", "16", 1132),
    ("quicksort/main", "16", 3908),
    ("quicksort/interp", "16", 538847),
    ("sha256/main", "16", 500232),
    ("sieve/main", "16", 1683),
    ("sieve/interp", "16", 1212861),
    ("spectral_norm/main", "16", 980733),
    ("string_search/main", "16", 3106),
    ("string_search/interp", "16", 2111603),
    ("strings/main", "16", 2700),
    ("strings/interp", "16", 265496),
    ("two_sum/main", "16", 1862),
    ("two_sum/interp", "16", 113368),
    ("vm/main", "16", 4251),
];

/// `(label, program, instructions)`: the hot loops of the operations the benchmarks do not reach.
const LOOP_BUDGETS: &[(&str, &str, u64)] = &[
    (
        "int_accumulator",
        "## Main\nLet mutable t be 0.\nRepeat for i from 1 to 2000:\n    Set t to t + i.\nShow t.\n",
        44020,
    ),
    (
        "text_append",
        "## Main\nLet mutable s be \"\".\nRepeat for i from 1 to 2000:\n    Set s to s + \"ab\".\nShow length of s.\n",
        52026,
    ),
    (
        "calls",
        "## To twice (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nLet mutable t be 0.\n\
         Repeat for i from 1 to 2000:\n    \
         Set t to t + twice(i).\nShow t.\n",
        80020,
    ),
    (
        "recursive_calls",
        "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    \
         Return fib(n - 1) + fib(n - 2).\n\n\
         ## Main\nShow fib(15).\n",
        51303,
    ),
    (
        "option_returning_calls",
        "## To half (n: Int) -> Option of Int:\n    If n % 2 equals 0:\n        Return some n / 2.\n    \
         Return none.\n\n\
         ## Main\nLet mutable t be 0.\nRepeat for i from 1 to 2000:\n    Inspect half(i):\n        \
         When Some (h):\n            \
         Set t to t + h.\n        When None:\n            Set t to t + 1.\nShow t.\n",
        155020,
    ),
    (
        "closure_calls",
        "## Main\nLet f be (x: Int) -> x + 3.\nLet mutable t be 0.\nRepeat for i from 1 to 2000:\n    \
         Set t to t + f(i).\nShow t.\n",
        96027,
    ),
    (
        "list_index",
        "## Main\nLet mutable xs be a new Seq of Int.\nRepeat for i from 1 to 1000:\n    Push i to xs.\n\
         Let mutable t be 0.\n\
         Repeat for i from 1 to 1000:\n    Set t to t + item i of xs.\nShow t.\n",
        50031,
    ),
    (
        "list_iteration",
        "## Main\nLet mutable xs be a new Seq of Int.\nRepeat for i from 1 to 1000:\n    Push i to xs.\n\
         Let mutable t be 0.\n\
         Repeat for x in xs:\n    Set t to t + x.\nShow t.\n",
        44040,
    ),
    (
        "map_update",
        "## Main\nLet mutable m be a new Map of Int to Int.\nRepeat for k from 1 to 50:\n    Set item k of m to 0.\n\
         Repeat for i from 1 to 1000:\n    Let k be (i % 50) + 1.\n    Set item k of m to (item k of m) + i.\n\
         Show item 7 of m.\n",
        49433,
    ),
    (
        "field_update",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## Main\nLet mutable p be a new Point with x 0 and y 0.\n\
         Repeat for i from 1 to 2000:\n    Set p's x to p's x + i.\nShow p's x.\n",
        62045,
    ),
    (
        "merge",
        "## A Tally has:\n    An a: Int.\n    A b: Int.\n    A c: Int.\n    A d: Int.\n\n## Main\n\
         Let mutable w be a new Tally with a 0 and b 0 and c 0 and d 0.\n\
         Let delta be a new Tally with a 1 and b 2 and c 3 and d 4.\n\
         Repeat for i from 1 to 500:\n    Merge delta into w.\nShow w's d.\n",
        20099,
    ),
    (
        "converge",
        "## A Tally has:\n    An a: Int.\n    A b: Int.\n    A c: Int.\n    A d: Int.\n\n## Main\n\
         Let mutable w be a new Tally with a 0 and b 0 and c 0 and d 0.\n\
         Let delta be a new Tally with a 1 and b 2 and c 3 and d 4.\n\
         Repeat for i from 1 to 500:\n    Converge delta into w.\nShow w's d.\n",
        20103,
    ),
    (
        "pop",
        "## Main\nLet mutable xs be a new Seq of Int.\nRepeat for i from 1 to 1000:\n    Push i to xs.\n\
         Let mutable t be 0.\n\
         Repeat for i from 1 to 1000:\n    Pop from xs into v.\n    Set t to t + v.\nShow t.\n",
        50030,
    ),
    (
        "text_iteration",
        "## Main\nLet mutable s be \"\".\nRepeat for i from 1 to 200:\n    Set s to s + \"abcde\".\n\
         Let mutable n be 0.\n\
         Repeat for c in s:\n    If c equals \"c\":\n        Set n to n + 1.\nShow n.\n",
        21434,
    ),
];

/// How many rows are measured at once: each is a link and a traced `qemu-riscv64` run of its own.
const WORKERS: usize = 4;

/// A program the lock measures: its label, its source, and the argument it runs with.
struct Row {
    label: String,
    source: String,
    arg: Option<String>,
    budget: u64,
}

fn rows() -> Vec<Row> {
    let mut rows: Vec<Row> = BENCHMARK_BUDGETS
        .iter()
        .map(|(name, size, budget)| {
            let (dir, variant) = name
                .split_once('/')
                .expect("a benchmark row names `benchmark/variant`");
            let path = Path::new(BENCHMARK_ROOT)
                .join(dir)
                .join(format!("{variant}.lg"));
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            Row {
                label: format!("{name}[{size}]"),
                source,
                arg: Some(size.to_string()),
                budget: *budget,
            }
        })
        .collect();
    rows.extend(LOOP_BUDGETS.iter().map(|(label, source, budget)| Row {
        label: label.to_string(),
        source: source.to_string(),
        arg: None,
        budget: *budget,
    }));
    rows
}

#[test]
fn every_budgeted_program_compiles_to_riscv() {
    let refused: Vec<String> = rows()
        .iter()
        .filter_map(|row| {
            compile_to_riscv(&row.source)
                .err()
                .map(|e| format!("{}: {e:?}", row.label))
        })
        .collect();
    assert!(
        refused.is_empty(),
        "the RISC-V tier refuses {} budgeted programs:\n  {}",
        refused.len(),
        refused.join("\n  ")
    );
}

/// A temp directory of its own, removed when dropped.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(label: &str) -> Scratch {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tag: String = label
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let dir = std::env::temp_dir().join(format!(
        "logos_riscv_budget_{}_{n}_{tag}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    Scratch(dir)
}

fn run(cmd: &mut Command, what: &str) -> Result<std::process::Output, String> {
    let out = cmd.output().map_err(|e| format!("{what}: {e}"))?;
    if out.status.success() {
        Ok(out)
    } else {
        Err(format!(
            "{what} exited {:?}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

/// The address range of the emitted object's `.text` in the linked binary, from the link map's line for it:
/// ` .text  0x<start>  0x<size>  <dir>/main.o`.
fn emitted_text(map: &str, object: &Path) -> Option<(u64, u64)> {
    let object = object.to_string_lossy();
    map.lines().find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        match fields[..] {
            [".text", start, size, path] if path == object => {
                let hex = |s: &str| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok();
                Some((hex(start)?, hex(size)?))
            }
            _ => None,
        }
    })
}

/// The guest instructions a `-d in_asm,exec,nochain` trace executed: each `Trace` line is one execution of
/// the translation block at its guest pc, and each `IN:` block lists that block's instructions.
fn executed_instructions(trace: &str) -> u64 {
    let mut size: HashMap<u64, u64> = HashMap::new();
    let mut block: Option<(u64, u64)> = None;
    let mut total = 0;
    for line in trace.lines() {
        if line.starts_with("IN:") {
            block = None;
        } else if let Some(rest) = line.strip_prefix("0x") {
            let pc = rest
                .split(':')
                .next()
                .and_then(|a| u64::from_str_radix(a, 16).ok());
            if let Some(pc) = pc {
                block = Some(match block {
                    None => (pc, 1),
                    Some((start, n)) => (start, n + 1),
                });
            }
        } else if line.trim().is_empty() {
            if let Some((start, n)) = block.take() {
                size.insert(start, n);
            }
        } else if line.starts_with("Trace ") {
            let pc = line
                .split('[')
                .nth(1)
                .and_then(|f| f.split('/').nth(1))
                .and_then(|a| u64::from_str_radix(a, 16).ok());
            if let Some(pc) = pc {
                total += size.get(&pc).copied().unwrap_or(0);
            }
        }
    }
    total
}

/// The instructions the emitted code of `source` executes when run with `arg`.
fn emitted_instructions(label: &str, source: &str, arg: Option<&str>) -> Result<u64, String> {
    let asm = compile_to_riscv(source).map_err(|e| format!("does not compile to RISC-V: {e:?}"))?;
    let dir = scratch(label);
    let path = |name: &str| dir.0.join(name);
    let (asm_path, object, runtime, bin, map, trace) = (
        path("main.s"),
        path("main.o"),
        path("logos_rt.c"),
        path("main"),
        path("main.map"),
        path("trace.log"),
    );
    std::fs::write(&asm_path, asm).map_err(|e| e.to_string())?;
    std::fs::write(&runtime, RISCV_RUNTIME_C).map_err(|e| e.to_string())?;
    run(
        Command::new("riscv64-linux-gnu-gcc")
            .arg("-c")
            .arg(&asm_path)
            .arg("-o")
            .arg(&object),
        "assembling",
    )?;
    run(
        Command::new("riscv64-linux-gnu-gcc")
            .args(["-O2", "-static", "-o"])
            .arg(&bin)
            .arg(&object)
            .arg(&runtime)
            .arg("-lm")
            .arg(format!("-Wl,-Map={}", map.display())),
        "linking",
    )?;
    let map_text = std::fs::read_to_string(&map).map_err(|e| e.to_string())?;
    let (start, len) = emitted_text(&map_text, &object)
        .ok_or("the link map names no .text for the emitted object")?;
    let mut qemu = Command::new("sh");
    qemu.arg("-c")
        .arg("ulimit -v \"$1\" && t=\"$2\" && shift 2 && exec timeout \"$t\" qemu-riscv64 \"$@\"")
        .arg("sh")
        .arg((riscv_runner::RUN_CAP_MB * 1024).to_string())
        .arg(riscv_runner::RUN_SECONDS.to_string())
        .args(["-d", "in_asm,exec,nochain", "-dfilter"])
        .arg(format!("{start:#x}+{len:#x}"))
        .arg("-D")
        .arg(&trace)
        .arg(&bin)
        .args(arg)
        .env(
            "LOGICAFFEINE_QEMU_MEM_MB",
            riscv_runner::RUN_CAP_MB.to_string(),
        );
    run(&mut qemu, "running under qemu-riscv64")?;
    let trace_text = std::fs::read_to_string(&trace).map_err(|e| e.to_string())?;
    Ok(executed_instructions(&trace_text))
}

/// The runtime never unwinds: a failure prints its sentence and exits, and nothing throws or walks the stack.
/// So its object carries no `.eh_frame`, whose every entry a static binary's startup walks to register the
/// program's frames before `main` runs; its call-frame information goes to `.debug_frame`, which nothing reads
/// at run time.
#[test]
fn the_runtime_carries_no_unwind_tables() {
    if !common::require_tool("riscv64-linux-gnu-gcc")
        || !common::require_tool("riscv64-linux-gnu-objdump")
    {
        return;
    }
    let dir = scratch("runtime");
    let (source, object) = (dir.0.join("logos_rt.c"), dir.0.join("logos_rt.o"));
    std::fs::write(&source, RISCV_RUNTIME_C).unwrap();
    let compile = Command::new("riscv64-linux-gnu-gcc")
        .args(["-O2", "-c"])
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .output()
        .expect("run riscv64-linux-gnu-gcc");
    assert!(
        compile.status.success(),
        "the runtime does not compile:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    let headers = Command::new("riscv64-linux-gnu-objdump")
        .arg("-h")
        .arg(&object)
        .output()
        .expect("run objdump");
    let sections = String::from_utf8_lossy(&headers.stdout).to_string();
    assert!(
        !sections.lines().any(|l| l.split_whitespace().nth(1) == Some(".eh_frame")),
        "the RISC-V runtime's object carries an .eh_frame, which startup walks on every run:\n{sections}"
    );
}

#[test]
fn emitted_code_runs_exactly_its_instruction_budget() {
    if !have_toolchain() {
        return;
    }
    let rows = rows();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let verdicts: Vec<std::sync::Mutex<Option<String>>> =
        rows.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..WORKERS {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(row) = rows.get(i) else { break };
                let verdict = match emitted_instructions(&row.label, &row.source, row.arg.as_deref()) {
                    Err(why) => Some(format!("{}: {why}", row.label)),
                    Ok(n) if n > row.budget => Some(format!(
                        "{}: {n} instructions, over its budget of {} by {} ({:+.2}%) — a regression of the emitter",
                        row.label,
                        row.budget,
                        n - row.budget,
                        (n as f64 - row.budget as f64) * 100.0 / row.budget.max(1) as f64
                    )),
                    Ok(n) if n < row.budget => Some(format!(
                        "{}: {n} instructions, under its budget of {}: lower the budget to {n}",
                        row.label, row.budget
                    )),
                    Ok(_) => None,
                };
                *verdicts[i].lock().unwrap() = verdict;
            });
        }
    });
    let failed: Vec<String> = verdicts
        .into_iter()
        .filter_map(|v| v.into_inner().unwrap())
        .collect();
    assert!(
        failed.is_empty(),
        "{} of {} programs do not run exactly their instruction budget:\n  {}",
        failed.len(),
        rows.len(),
        failed.join("\n  ")
    );
}
