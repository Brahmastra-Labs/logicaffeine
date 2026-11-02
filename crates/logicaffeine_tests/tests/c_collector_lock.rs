//! The C tier's one allocator and its collector.
//!
//! Every block the bytecode-C walker's program and its runtime allocate goes through `lc_gc_*`
//! (`codegen_c::runtime`, `C_GC_COLLECTOR`), and the collector reclaims what the program can no longer reach: it
//! marks from the stack, the registers, the data and bss segments and every block it reaches, finding a block from
//! any word that points into it, with the low bit of an exact integer's tagged word ignored.
//! - **One allocator.** No `malloc`, `calloc`, `realloc`, `strdup`, `strndup` or `free` appears in the walker's C
//!   outside the allocator's own sections: a block the collector does not know of could hold the only pointer to
//!   one it does.
//! - **Every pointer form is known.** The only way the walker hides a pointer is the exact integer's tag, `p | 1`,
//!   which the collector strips. A new form is RED here until the collector learns it.
//! - **Memory is bounded by what the program holds.** A loop that builds and drops a list many times runs in the
//!   same peak memory as one that does it a few times.
//! - **Device code never links the collector.** The CUDA and Tenstorrent programs carry the C library's calls.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use logicaffeine_compile::compile::compile_to_c_via_bytecode;

/// Programs that bring in every runtime the walker emits: collections, texts, maps, sets, exact integers, closures,
/// files, offline networking, CRDT counters.
const RUNTIME_CORPUS: &[(&str, &str)] = &[
    ("collections", "## Main\n    Let mutable xs be [3, 1, 2].\n    Push 4 to xs.\n    Let mutable m be a new Map of Text to Int.\n    Set item \"a\" of m to 1.\n    Let mutable s be a new Set of Int.\n    Add 5 to s.\n    Show xs.\n    Show m.\n    Show s.\n"),
    ("texts_and_exact_integers", "## Main\n    Let mutable t be \"\".\n    Repeat for i from 1 to 5:\n        Set t to t + \"ab\".\n    Let mutable p be 1.\n    Repeat for i from 1 to 80:\n        Set p to p * 3.\n    Show t.\n    Show p.\n"),
    ("closures", "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n## Main\n    Let k be 10.\n    Show apply((n: Int) -> n + k, 5).\n"),
    ("files", "## Main\nWrite \"hello\" to file \"read.txt\".\nRead contents from file \"read.txt\".\nShow contents.\n"),
    ("network", "## Main\nListen on \"/ip4/0.0.0.0/tcp/0\".\nShow 1.\n"),
    ("crdt", "## Definition\nA Tally has:\n    n: a Counter.\n## Main\n    Let mutable state be a new Tally.\n    Increase state's n by 3.\n    Show state's n.\n"),
];

/// The spans of `c` inside the allocator's own sections, where the C library's allocator is called.
fn allocator_sections(c: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    for (open, close) in [
        ("/* LC_GC_COLLECTOR_BEGIN */", "/* LC_GC_COLLECTOR_END */"),
        ("/* LC_GC_BEGIN", "/* LC_GC_END */"),
    ] {
        let mut from = 0;
        while let Some(at) = c[from..].find(open) {
            let lo = from + at;
            let hi = lo
                + c[lo..]
                    .find(close)
                    .unwrap_or_else(|| panic!("`{open}` with no `{close}`"));
            spans.push((lo, hi));
            from = hi;
        }
    }
    spans
}

/// Each call of the C library's allocator in `c` outside the allocator's sections, as the line it is on.
fn stray_allocations(c: &str) -> Vec<String> {
    let sections = allocator_sections(c);
    let mut found = Vec::new();
    for name in ["malloc", "calloc", "realloc", "strdup", "strndup", "free"] {
        let pat = format!("{name}(");
        let mut from = 0;
        while let Some(at) = c[from..].find(&pat) {
            let pos = from + at;
            from = pos + pat.len();
            let before = c[..pos].chars().next_back();
            if before.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
                continue;
            }
            if sections.iter().any(|(lo, hi)| (*lo..*hi).contains(&pos)) {
                continue;
            }
            let line_start = c[..pos].rfind('\n').map_or(0, |i| i + 1);
            let line_end = c[pos..].find('\n').map_or(c.len(), |i| pos + i);
            found.push(c[line_start..line_end].trim().to_string());
        }
    }
    found
}

#[test]
fn the_walker_allocates_only_through_the_collector() {
    let mut stray = Vec::new();
    for (label, src) in RUNTIME_CORPUS {
        let c = compile_to_c_via_bytecode(src)
            .unwrap_or_else(|e| panic!("[{label}] the walker refused it: {e:?}"));
        assert!(
            c.contains("/* LC_GC_COLLECTOR_BEGIN */"),
            "[{label}] the walker's C does not carry the collector"
        );
        for line in stray_allocations(&c) {
            stray.push(format!("[{label}] {line}"));
        }
    }
    stray.sort();
    stray.dedup();
    assert!(
        stray.is_empty(),
        "the C library's allocator is called outside the collector:\n{}",
        stray.join("\n")
    );
}

/// Every place the walker's C, outside the allocator's own sections (which compute addresses), turns a pointer into a
/// word other than as itself: a cast to an integer followed by an arithmetic or bitwise operator and a constant.
fn pointer_forms(c: &str) -> Vec<String> {
    let sections = allocator_sections(c);
    let mut forms = Vec::new();
    for cast in ["(intptr_t)", "(uintptr_t)"] {
        let mut from = 0;
        while let Some(at) = c[from..].find(cast) {
            let pos = from + at + cast.len();
            from = pos;
            if sections.iter().any(|(lo, hi)| (*lo..*hi).contains(&pos)) {
                continue;
            }
            let rest = &c[pos..];
            let operand: usize = rest
                .char_indices()
                .take_while(|(_, ch)| {
                    ch.is_ascii_alphanumeric()
                        || matches!(ch, '_' | '(' | '[' | ']' | '>' | '-' | '.')
                })
                .map(|(i, ch)| i + ch.len_utf8())
                .last()
                .unwrap_or(0);
            let after = rest[operand..].trim_start();
            let op = after.chars().next();
            if matches!(op, Some('|' | '^' | '+')) && !after.starts_with("||") {
                let tail = after[1..].trim_start();
                if tail.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
                    let digits: String = tail
                        .chars()
                        .take_while(|ch| ch.is_ascii_alphanumeric())
                        .collect();
                    forms.push(format!(
                        "{cast}{} {} {digits}",
                        &rest[..operand],
                        op.unwrap()
                    ));
                }
            }
        }
    }
    forms.sort();
    forms.dedup();
    forms
}

#[test]
fn every_tagged_pointer_form_is_one_the_collector_strips() {
    let mut forms = Vec::new();
    for (label, src) in RUNTIME_CORPUS {
        let c = compile_to_c_via_bytecode(src)
            .unwrap_or_else(|e| panic!("[{label}] the walker refused it: {e:?}"));
        forms.extend(pointer_forms(&c));
    }
    forms.sort();
    forms.dedup();
    // The exact integer's box, tagged in `lc_xput` and untagged in `lc_xget`: bit 0, which the collector strips.
    let known = ["(intptr_t)(w ^ 1", "(intptr_t)b | 1"];
    let unknown: Vec<&String> = forms
        .iter()
        .filter(|f| !known.iter().any(|k| f.starts_with(k)))
        .collect();
    assert!(
        unknown.is_empty(),
        "a pointer is hidden in a form the collector does not strip (it strips bit 0 only); teach `lc_gc_consider` the form, then list it here:\n{}",
        unknown.iter().map(|f| f.as_str()).collect::<Vec<_>>().join("\n")
    );
    for k in known {
        assert!(
            forms.iter().any(|f| f.starts_with(k)),
            "the known form `{k}` is gone; remove it from the list: {forms:?}"
        );
    }
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("logos_collector_{}_{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn build(c: &str, dir: &Path) -> PathBuf {
    let (cfile, bin) = (dir.join("prog.c"), dir.join("prog"));
    std::fs::write(&cfile, c).expect("write C");
    let out = Command::new("gcc")
        .args(["-O2", "-o"])
        .arg(&bin)
        .arg(&cfile)
        .arg("-lm")
        .output()
        .expect("gcc runs");
    assert!(
        out.status.success(),
        "gcc failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    bin
}

/// The run's stdout and its peak resident memory in KiB.
fn run_measured(bin: &Path, env: &[(&str, &str)]) -> (String, i64) {
    let mut cmd = Command::new(bin);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit());
    for (k, v) in env {
        cmd.env(k, v);
    }
    let child = cmd.spawn().expect("the program runs");
    let pid = child.id() as libc::pid_t;
    let mut stdout = child.stdout.expect("stdout piped");
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut stdout, &mut s).expect("read stdout");
        s
    });
    let mut status = 0;
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let waited = unsafe { libc::wait4(pid, &mut status, 0, &mut usage) };
    assert_eq!(waited, pid, "wait4 failed");
    assert!(
        libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0,
        "the program failed: status {status}"
    );
    (reader.join().expect("reader"), usage.ru_maxrss)
}

/// A loop that builds a list of `n` texts and drops it, `rounds` times: it holds one list at a time.
fn churn(rounds: u32) -> String {
    format!(
        "## Main\n    Let mutable total be 0.\n    Repeat for r from 1 to {rounds}:\n        Let mutable xs be a new Seq of Text.\n        Repeat for i from 1 to 20000:\n            Push \"item \" + i to xs.\n        Set total to total + length of xs.\n    Show total.\n"
    )
}

#[test]
fn memory_is_bounded_by_what_the_program_holds() {
    if !common::require_tool("gcc") {
        return;
    }
    let mut peaks = Vec::new();
    for rounds in [20u32, 320] {
        let c =
            compile_to_c_via_bytecode(&churn(rounds)).expect("the walker lowers the churn loop");
        let dir = scratch(&format!("churn_{rounds}"));
        let bin = build(&c, &dir);
        let (out, peak) = run_measured(&bin, &[]);
        assert_eq!(
            out,
            format!("{}\n", rounds as u64 * 20000),
            "the churn loop's total"
        );
        peaks.push((rounds, peak));
        let _ = std::fs::remove_dir_all(&dir);
    }
    let (few, many) = (peaks[0].1, peaks[1].1);
    // Sixteen times the garbage, the same live set: the peak stays within the collector's slack of the small run's.
    assert!(
        many <= few * 3 / 2 + 16 * 1024,
        "peak memory grows with the garbage, not with what the program holds: {peaks:?} (rounds, peak KiB)"
    );
}

#[test]
fn device_programs_never_link_the_collector() {
    let src = "## Main\n    Let mutable s be 0.\n    Repeat for i from 1 to 10:\n        Set s to s + i.\n    Show s.\n";
    let cuda =
        logicaffeine_compile::compile::compile_to_cuda(src).expect("the CUDA tier lowers the sum");
    let tt = logicaffeine_compile::compile::compile_to_tenstorrent(src)
        .expect("the Tenstorrent tier lowers the sum");
    for (name, text) in [
        ("CUDA kernel", cuda.kernel_cu.as_str()),
        ("Tenstorrent kernel", tt.kernel_cpp.as_str()),
    ] {
        for marker in ["LC_GC_COLLECTOR_BEGIN", "lc_gc_collect", "LC_GC_COLLECTS 1"] {
            assert!(
                !text.contains(marker),
                "the {name} links the collector (`{marker}`)"
            );
        }
    }
}

/// Every benchmark program, by name, with its source, fold fusion off: these locks are about what the programs
/// allocate, and binary_trees fused builds no tree.
fn benchmarks() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/programs");
    let mut found: Vec<(String, String)> = std::fs::read_dir(&dir)
        .expect("benchmarks/programs exists")
        .flatten()
        .filter_map(|e| {
            Some((
                e.file_name().to_string_lossy().into_owned(),
                logicaffeine_compile::optimization::decorate_source(
                    &std::fs::read_to_string(e.path().join("main.lg")).ok()?,
                    &["foldfuse"],
                ),
            ))
        })
        .collect();
    found.sort();
    found
}

/// The size every benchmark runs at here: small enough that a collection at every allocation finishes.
const SMALL: &str = "8";

/// What a run of `bin` at `SMALL` shows, with `env` set and its sanitizer reports, if any.
fn run_small(bin: &Path, env: &[(&str, &str)]) -> (std::process::Output, Vec<String>) {
    let out = Command::new(bin)
        .arg(SMALL)
        .envs(env.iter().copied())
        .output()
        .expect("the program runs");
    let reports = String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| {
            l.contains("ERROR: LeakSanitizer")
                || l.contains("ERROR: AddressSanitizer")
                || l.starts_with("SUMMARY: ")
        })
        .map(str::to_string)
        .collect();
    (out, reports)
}

#[test]
fn every_benchmark_frees_everything_it_allocates() {
    if !common::require_tool("gcc") {
        return;
    }
    let mut reports = Vec::new();
    for (name, src) in benchmarks() {
        let c = compile_to_c_via_bytecode(&src)
            .unwrap_or_else(|e| panic!("[{name}] the walker refused it: {e:?}"));
        let dir = scratch(&format!("asan_{name}"));
        std::fs::write(dir.join("prog.c"), &c).expect("write C");
        let built = Command::new("gcc")
            .args([
                "-O1",
                "-g",
                "-fsanitize=address",
                "-fno-omit-frame-pointer",
                "-o",
                "prog",
                "prog.c",
                "-lm",
            ])
            .current_dir(&dir)
            .output()
            .expect("gcc runs");
        assert!(
            built.status.success(),
            "[{name}] gcc failed:\n{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let (out, found) = run_small(
            &dir.join("prog"),
            &[("ASAN_OPTIONS", "detect_leaks=1:exitcode=99")],
        );
        if !out.status.success() {
            reports.push(format!(
                "[{name}] failed {:?}: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        if !found.is_empty() {
            reports.push(format!("[{name}] {}", found.join(" | ")));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(
        reports.is_empty(),
        "benchmarks leak or touch memory they do not own:\n{}",
        reports.join("\n")
    );
}

/// Each named benchmark at `SMALL` prints the same with the collector collecting before every allocation as
/// without it.
fn assert_stressed_benchmarks_match(names: Option<&[&str]>) {
    if !common::require_tool("gcc") {
        return;
    }
    let mut differences = Vec::new();
    for (name, src) in benchmarks()
        .into_iter()
        .filter(|(n, _)| names.map_or(true, |ns| ns.contains(&n.as_str())))
    {
        let c = compile_to_c_via_bytecode(&src)
            .unwrap_or_else(|e| panic!("[{name}] the walker refused it: {e:?}"));
        let dir = scratch(&format!("stress_{name}"));
        let bin = build(&c, &dir);
        let (plain, _) = run_small(&bin, &[]);
        let (stressed, _) = run_small(&bin, &[("LOGOS_GC_STRESS", "1")]);
        if (plain.status.code(), &plain.stdout, &plain.stderr)
            != (stressed.status.code(), &stressed.stdout, &stressed.stderr)
        {
            differences.push(format!(
                "[{name}] plain {:?} {:?}\n  stressed {:?} {:?}",
                plain.status.code(),
                String::from_utf8_lossy(&plain.stdout),
                stressed.status.code(),
                String::from_utf8_lossy(&stressed.stdout)
            ));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(
        differences.is_empty(),
        "benchmarks run differently with a collection at every allocation:\n{}",
        differences.join("\n")
    );
}

/// A sample of the benchmarks — trees, strings, maps, sorting — under a collection at every allocation.
#[test]
fn a_sample_of_the_benchmarks_runs_the_same_with_a_collection_at_every_allocation() {
    assert_stressed_benchmarks_match(Some(&[
        "binary_trees",
        "strings",
        "two_sum",
        "mergesort",
        "graph_bfs",
    ]));
}

#[test]
#[ignore = "heavy: every benchmark collects before each allocation"]
fn every_benchmark_runs_the_same_with_a_collection_at_every_allocation() {
    assert_stressed_benchmarks_match(None);
}

/// Each iteration consumes one tree and discards it. The explicit allocation
/// counter prevents an optimizer that removes the workload from passing this
/// memory check without exercising enum reclamation.
#[test]
fn discarded_enum_trees_keep_peak_memory_bounded() {
    assert!(
        common::require_tool("gcc"),
        "enum collector regression requires gcc"
    );
    const TREE: &str = r#"## A Tree is either:
    A Leaf.
    A Node with left Tree and right Tree.

## To buildTree (depth: Int) -> Tree:
    If depth equals 0:
        Return a new Leaf.
    Return a new Node with left buildTree(depth - 1) and right buildTree(depth - 1).

## To countTree (tree: Tree) -> Int:
    Inspect tree:
        When Leaf: Return 1.
        When Node (left, right): Return 1 + countTree(left) + countTree(right).

"#;
    let mut peaks = Vec::new();
    for rounds in [64u64, 4096] {
        let source = format!("{TREE}## Main\nLet mutable total be 0.\nRepeat for i from 1 to {rounds}:\n    Set total to total + countTree(buildTree(10)).\nShow total.\n");
        let source = logicaffeine_compile::optimization::decorate_source(
            &source,
            &[
                "memo",
                "comptime",
                "inline",
                "unfold",
                "supercompile",
                "specialize",
                "loophoist",
                "foldfuse",
            ],
        );
        let c = compile_to_c_via_bytecode(&source).expect("the walker lowers enum churn");
        let signature = "lc_enum_alloc(size_t n){";
        assert_eq!(c.matches(signature).count(), 1, "one enum allocation entry");
        let c = format!(
            "#include <stdio.h>\nstatic unsigned long long enum_allocations;\n\
             __attribute__((destructor)) static void report_enum_allocations(void) {{ \
             printf(\"enum_allocations=%llu\\n\", enum_allocations); }}\n{}",
            c.replacen(signature, "lc_enum_alloc(size_t n){ ++enum_allocations;", 1),
        );
        let dir = scratch(&format!("enum_churn_{rounds}"));
        let bin = build(&c, &dir);
        let (output, peak) = run_measured(&bin, &[]);
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(
            lines.len(),
            2,
            "program output plus allocation census: {output}"
        );
        assert_eq!(
            lines[0],
            (rounds * 2047).to_string(),
            "every constructed tree remains intact"
        );
        let allocations: u64 = lines[1]
            .strip_prefix("enum_allocations=")
            .expect("allocation census")
            .parse()
            .expect("allocation count");
        assert!(
            allocations >= rounds * 1023,
            "the test must allocate every non-leaf node: {allocations}"
        );
        peaks.push(peak);
        std::fs::remove_dir_all(dir).expect("remove enum fixture");
    }
    assert!(
        peaks[1] <= peaks[0] * 3 / 2 + 16 * 1024,
        "discarded enum trees retain heap across 64 times the work: {peaks:?} KiB"
    );
}

/// Trace only allocated payloads. Free-list links and reused bytes are not
/// references owned by the program, while a stored child pointer is.
#[test]
fn tracing_ignores_allocator_links_and_reclaimed_payloads() {
    assert!(
        common::require_tool("gcc"),
        "collector traversal requires gcc"
    );
    let c = compile_to_c_via_bytecode("## Main\nShow 1.\n").expect("compile collector carrier");
    let marker = "/* LC_GC_COLLECTOR_END */";
    let end = c.find(marker).expect("collector must be emitted") + marker.len();
    let c = format!(
        "{}\n{}",
        &c[..end],
        include_str!("support/c_collector_traversal.c")
    );
    let dir = scratch("traversal");
    let bin = build(&c, &dir);
    for mode in 0..5 {
        let out = Command::new(&bin)
            .arg(mode.to_string())
            .output()
            .expect("run traversal");
        assert!(
            out.status.success(),
            "mode {mode}: {} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    std::fs::remove_dir_all(dir).expect("remove traversal fixture");
}

#[test]
fn collector_rejects_size_overflow_and_initializes_traced_storage() {
    assert!(
        common::require_tool("gcc"),
        "collector allocation contract requires gcc"
    );
    assert!(
        common::require_tool("clang"),
        "checked collector contract requires clang"
    );
    let c = compile_to_c_via_bytecode("## Main\nShow 1.\n").expect("compile collector carrier");
    let marker = "/* LC_GC_COLLECTOR_END */";
    let end = c.find(marker).expect("collector emitted") + marker.len();
    let c = format!(
        "{}\n{}",
        &c[..end],
        include_str!("support/c_collector_allocation.c")
    );
    let dir = scratch("allocation_contract");
    let bin = build(&c, &dir);
    for mode in 0..4 {
        let out = Command::new(&bin)
            .arg(mode.to_string())
            .output()
            .expect("run allocation contract");
        if mode < 3 {
            assert_eq!(out.status.code(), Some(1), "overflow mode {mode}: {out:?}");
            assert_eq!(
                out.stderr, b"memory allocation failed\n",
                "overflow fails before allocation"
            );
        } else {
            assert!(
                out.status.success(),
                "traced payload must be initialized: {out:?}"
            );
        }
    }
    let checked = dir.join("checked");
    let built = Command::new("clang")
        .args(["-O1", "-g", "-fsanitize=address", "-fno-omit-frame-pointer"])
        .arg(dir.join("prog.c"))
        .arg("-o")
        .arg(&checked)
        .output()
        .expect("clang runs");
    assert!(
        built.status.success(),
        "clang: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let out = Command::new(checked)
        .arg("4")
        .env("ASAN_OPTIONS", "detect_leaks=1:abort_on_error=1")
        .output()
        .expect("run checked allocator");
    assert!(
        out.status.success(),
        "Clang must use exact allocation bounds: {out:?}"
    );
    std::fs::remove_dir_all(dir).expect("remove allocation fixtures");
}

#[test]
fn removed_container_values_do_not_remain_tracing_roots() {
    assert!(
        common::require_tool("gcc"),
        "vacated-slot contract requires gcc"
    );
    let c = compile_to_c_via_bytecode("## Main\nShow 1.\n").expect("compile runtime carrier");
    let prefix = c.split_once("int main(").expect("generated main entry").0;
    let c = format!(
        "{prefix}\n{}",
        include_str!("support/c_collector_vacated_slots.c")
    );
    let dir = scratch("vacated_slots");
    let bin = build(&c, &dir);
    for mode in 0..7 {
        let out = Command::new(&bin)
            .arg(mode.to_string())
            .output()
            .expect("run container tracing");
        assert!(out.status.success(), "vacated-slot mode {mode}: {out:?}");
    }
    std::fs::remove_dir_all(dir).expect("remove container fixture");
}

#[test]
fn growing_traced_storage_preserves_values_and_clears_new_bytes() {
    assert!(common::require_tool("gcc"), "collector resize requires gcc");
    assert!(
        common::require_tool("clang"),
        "checked collector resize requires clang"
    );
    let c = compile_to_c_via_bytecode("## Main\nShow 1.\n").expect("compile collector carrier");
    let marker = "/* LC_GC_COLLECTOR_END */";
    let end = c.find(marker).expect("collector emitted") + marker.len();
    let c = format!(
        "{}\n{}",
        &c[..end],
        include_str!("support/c_collector_resize.c")
    );
    let dir = scratch("resize");
    let bin = build(&c, &dir);
    let out = Command::new(bin).output().expect("run paged heap resize");
    assert!(
        out.status.success(),
        "resize must clear newly traced bytes and preserve retained bytes: {out:?}"
    );
    let checked = dir.join("checked");
    let built = Command::new("clang")
        .args(["-O1", "-g", "-fsanitize=address", "-fno-omit-frame-pointer"])
        .arg(dir.join("prog.c"))
        .arg("-o")
        .arg(&checked)
        .output()
        .expect("clang runs");
    assert!(
        built.status.success(),
        "clang: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let out = Command::new(checked)
        .env("ASAN_OPTIONS", "detect_leaks=1:abort_on_error=1")
        .output()
        .expect("run checked heap resize");
    assert!(out.status.success(), "checked heap resize: {out:?}");
    std::fs::remove_dir_all(dir).expect("remove resize fixtures");
}

#[test]
fn retired_scheduler_entries_stop_retaining_tasks_and_payloads() {
    assert!(
        common::require_tool("gcc"),
        "scheduler tracing requires gcc"
    );
    let c = compile_to_c_via_bytecode("## Main\nShow 1.\n").expect("compile collector carrier");
    let marker = "/* LC_GC_COLLECTOR_END */";
    let end = c.find(marker).expect("collector emitted") + marker.len();
    let c = format!(
        "{}\n\
        #define LOGOS_FN static __attribute__((unused))\n\
        #define LOGOS_VAR static\n\
        #define LOGOS_TASK_TIER\n\
        #define logos_alloc(n) lc_gc_calloc(1,(n))\n\
        #define logos_release(p) lc_gc_free(p)\n{}\n{}",
        &c[..end],
        include_str!("../../logicaffeine_compile/src/codegen_c/sched_core.c"),
        include_str!("support/c_collector_scheduler_retention.c")
    );
    let dir = scratch("scheduler_retention");
    let bin = build(&c, &dir);
    for mode in 0..6 {
        let out = Command::new(&bin)
            .arg(mode.to_string())
            .output()
            .expect("run scheduler tracing");
        assert!(
            out.status.success(),
            "scheduler retention mode {mode}: {out:?}"
        );
    }
    std::fs::remove_dir_all(dir).expect("remove scheduler fixture");
}
