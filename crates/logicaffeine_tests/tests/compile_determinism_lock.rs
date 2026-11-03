//! A program compiles to the same optimized bytecode every time.
//!
//! Every program the tree holds (`support/program_corpus.rs`, discovered at run time) is compiled by
//! `compile_to_program_optimized` in two separate processes running side by side; the sample is compiled twice
//! in each, so state one compile leaves for the next shows too. Each process seeds its hash maps
//! differently, and each map a process builds is seeded differently again. Every rendering must be
//! identical, byte for byte:
//!
//! - every field of the `CompiledProgram`;
//! - its maps in key order;
//! - its ops one per line, so a difference names the first op that differs.
//!
//! A difference means some pass emits in the iteration order of an unordered map. That makes the bytecode,
//! and everything lowered from it, a function of the process instead of the program.
//!
//! The default row compiles a seeded sample of the corpus, and the canaries: loops that read many arrays, so
//! a map of hoisted accesses has many orders to fall into. The ignored row compiles all of it, once in each
//! process.

#[path = "support/rust_literals.rs"]
mod rust_literals;
#[path = "support/program_corpus.rs"]
mod program_corpus;

use std::collections::BTreeMap;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::io::Write;

use logicaffeine_compile::compile::compile_to_program_optimized;
use logicaffeine_compile::vm::CompiledProgram;
use program_corpus::{content_id, corpus, sample, Program};

const SAMPLE: usize = 300;
const SEED: u64 = 0x5EED_D373_2026_0930;
/// Threads per child process: `LOGOS_DETERMINISM_THREADS`, else half the host's, so the two children together use
/// what nextest grants the whole-corpus row (`threads-required` in `.config/nextest.toml`).
fn threads() -> usize {
    std::env::var("LOGOS_DETERMINISM_THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(2, |n| n.get()).div_ceil(2))
        .clamp(1, 32)
}

/// A function whose loop reads `width` sequence parameters up to a bound it cannot prove: the compiler hoists
/// one region-entry bounds guard per sequence, from one map of the accesses.
fn canary(width: usize) -> Program {
    let names: Vec<String> = (0..width).map(|k| format!("a{k}")).collect();
    let params: Vec<String> = names.iter().map(|n| format!("{n}: Seq of Int")).collect();
    let reads: String = names.iter().map(|n| format!(" + item i of {n}")).collect();
    let args: Vec<&str> = names.iter().map(|_| "xs").collect();
    let src = format!(
        "## To native args () -> Seq of Text\n## To native parseInt (s: Text) -> Int\n\n## To total ({}, n: Int) -> Int:\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is at most n:\n        Set s to s{reads}.\n        Set i to i + 1.\n    Return s.\n\n## Main\n    Let n be parseInt(item 2 of args()).\n    Let mutable xs be a new Seq of Int.\n    Let mutable k be 0.\n    While k is less than n:\n        Push k to xs.\n        Set k to k + 1.\n    Show total({}, n).\n",
        params.join(", "),
        args.join(", ")
    );
    Program { id: content_id(&src), origin: format!("canary with {width} sequences"), source: src }
}

fn canaries() -> Vec<Program> {
    [2, 4, 8].into_iter().map(canary).collect()
}

/// The program, every field of it, maps in key order, ops one per line.
fn render(p: &CompiledProgram) -> String {
    let mut rest = p.clone();
    let code = std::mem::take(&mut rest.code);
    let fn_index: BTreeMap<_, _> = std::mem::take(&mut rest.fn_index).into_iter().collect();
    let loop_locals: BTreeMap<_, _> = std::mem::take(&mut rest.loop_locals).into_iter().collect();
    let mut out = String::new();
    for (i, op) in code.iter().enumerate() {
        out.push_str(&format!("op {i} {op:?}\n"));
    }
    out.push_str(&format!("fn_index {fn_index:?}\nloop_locals {loop_locals:?}\n{rest:#?}\n"));
    out
}

/// How a compile went: the rendering, or why there is none.
fn compiled(source: &str) -> String {
    match std::panic::catch_unwind(|| compile_to_program_optimized(source)) {
        Ok(Ok(p)) => render(&p),
        Ok(Err(_)) => "does not compile\n".to_string(),
        Err(_) => "the optimizer panicked\n".to_string(),
    }
}

/// The child: compile every program in `LOGOS_DETERMINISM_IN` twice and write both renderings, program by program,
/// into the directory `LOGOS_DETERMINISM_OUT` — file `<n>` for the n-th program, its two renderings split by `\u{1f}\n`.
/// Each rendering leaves the process as soon as it is made, so the child holds at most one program per thread, never the
/// corpus. Without those variables it is not a child and does nothing.
#[test]
#[ignore]
fn determinism_child() {
    let (Ok(input), Ok(output)) = (std::env::var("LOGOS_DETERMINISM_IN"), std::env::var("LOGOS_DETERMINISM_OUT")) else {
        return;
    };
    let text = std::fs::read_to_string(&input).expect("the program list");
    let programs: Vec<&str> = text
        .split("\n\u{1e}\n")
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| chunk.split_once('\n').expect("an id line").1)
        .collect();
    let output = std::path::PathBuf::from(output);
    std::fs::create_dir_all(&output).expect("the renderings directory");
    let twice = std::env::var("LOGOS_DETERMINISM_TWICE").as_deref() != Ok("0");
    std::panic::set_hook(Box::new(|_| {}));
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..threads() {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::SeqCst);
                let Some(source) = programs.get(i) else { break };
                let first = compiled(source);
                let second = if twice { compiled(source) } else { String::new() };
                let mut file = std::io::BufWriter::new(std::fs::File::create(output.join(i.to_string())).expect("a rendering file"));
                file.write_all(first.as_bytes()).expect("write a rendering");
                file.write_all("\u{1f}\n".as_bytes()).expect("write a rendering");
                file.write_all(if twice { second.as_bytes() } else { first.as_bytes() }).expect("write a rendering");
                file.flush().expect("write a rendering");
            });
        }
    });
}

/// The program list the children read.
fn write_programs(programs: &[Program], dir: &std::path::Path) {
    let mut list = String::new();
    for p in programs {
        list.push_str(&format!("{}\n{}\n\u{1e}\n", p.id, p.source));
    }
    std::fs::write(dir.join("programs"), list).expect("write the program list");
}

/// Run the child over the program list in a fresh process; the directory its renderings are in (see
/// `determinism_child`), two per program: the second a recompile when `twice`, else the first again.
fn child(dir: &std::path::Path, tag: &str, twice: bool) -> std::path::PathBuf {
    let input = dir.join("programs");
    let output = dir.join(format!("renderings-{tag}"));
    let status = Command::new(std::env::current_exe().expect("this test binary"))
        .args(["determinism_child", "--exact", "--ignored", "--test-threads", "1"])
        .env("LOGOS_DETERMINISM_IN", &input)
        .env("LOGOS_DETERMINISM_OUT", &output)
        .env("LOGOS_DETERMINISM_TWICE", if twice { "1" } else { "0" })
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("the child starts");
    assert!(status.success(), "the child process {tag} finished");
    output
}

/// The n-th program's two renderings in a child's directory, if the child wrote them.
fn renderings(dir: &std::path::Path, n: usize) -> Option<(String, String)> {
    let text = std::fs::read_to_string(dir.join(n.to_string())).ok()?;
    let (first, second) = text.split_once("\u{1f}\n").expect("two renderings");
    Some((first.to_string(), second.to_string()))
}

/// A rendering cut into its sections: the ops, the two maps, and each top-level field of the rest.
fn sections(r: &str) -> BTreeMap<String, Vec<&str>> {
    let mut out: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut current = "code".to_string();
    for line in r.lines() {
        if line.starts_with("fn_index ") {
            current = "fn_index".to_string();
        } else if line.starts_with("loop_locals ") {
            current = "loop_locals".to_string();
        } else if let Some(field) = line.strip_prefix("    ").and_then(|l| l.split_once(": ")).map(|(f, _)| f) {
            if !field.starts_with(' ') && field.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                current = field.to_string();
            }
        }
        out.entry(current.clone()).or_default().push(line);
    }
    out
}

/// Each section on which `a` and `b` differ, with its first differing line, both sides shown.
fn differences(a: &str, b: &str) -> Vec<(String, String)> {
    let (sa, sb) = (sections(a), sections(b));
    let mut out = Vec::new();
    for name in sa.keys().chain(sb.keys()).collect::<std::collections::BTreeSet<_>>() {
        let (x, y) = (sa.get(name).cloned().unwrap_or_default(), sb.get(name).cloned().unwrap_or_default());
        if x != y {
            let first = x
                .iter()
                .zip(&y)
                .find(|(l, r)| l != r)
                .map(|(l, r)| format!("`{}` vs `{}`", l.trim(), r.trim()))
                .unwrap_or_else(|| format!("{} vs {} lines", x.len(), y.len()));
            out.push((name.clone(), first));
        }
    }
    out
}

/// `twice`: each process also compiles every program a second time, which catches state one compile leaves
/// for the next; the two processes alone catch every per-process order.
fn assert_deterministic(programs: Vec<Program>, name: &str, twice: bool) {
    let dir = std::env::temp_dir().join(format!("compile-determinism-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    write_programs(&programs, &dir);
    let (a, b) = std::thread::scope(|s| {
        let a = s.spawn(|| child(&dir, "a", twice));
        let b = s.spawn(|| child(&dir, "b", twice));
        (a.join().expect("process a"), b.join().expect("process b"))
    });
    let origin: BTreeMap<&str, &str> = programs.iter().map(|p| (p.id.as_str(), p.origin.as_str())).collect();
    let mut bad = Vec::new();
    let mut by_section: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut compiled_count = 0;
    for (n, p) in programs.iter().enumerate() {
        let (Some((a1, a2)), Some((b1, b2))) = (renderings(&a, n), renderings(&b, n)) else {
            bad.push(format!("{} ({}): a child wrote no rendering", p.id, origin[p.id.as_str()]));
            continue;
        };
        if !a1.starts_with("does not compile") {
            compiled_count += 1;
        }
        for (what, x, y) in [("within process a", &a1, &a2), ("within process b", &b1, &b2), ("across processes", &a1, &b1)] {
            let diffs = differences(x, y);
            if !diffs.is_empty() {
                for (section, first) in &diffs {
                    by_section.entry(section.clone()).or_default().push(p.id.clone());
                    bad.push(format!("{} ({}) differs {what} in `{section}`: {first}", p.id, origin[p.id.as_str()]));
                }
                break;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(compiled_count > 0, "the corpus compiled nothing");
    let tally: Vec<String> = by_section.iter().map(|(s, ids)| format!("`{s}`: {} programs", ids.len())).collect();
    assert!(
        bad.is_empty(),
        "programs compile to different bytecode ({} of {}), by section: {}\n{}",
        by_section.values().flatten().collect::<std::collections::BTreeSet<_>>().len(),
        programs.len(),
        tally.join(", "),
        bad.join("\n")
    );
}

#[test]
fn a_sample_of_the_corpus_and_the_canaries_compile_identically_in_every_process() {
    let mut programs = sample(&corpus(), SAMPLE, SEED);
    programs.extend(canaries());
    assert_deterministic(programs, "sample", true);
}

#[test]
#[ignore]
fn the_whole_corpus_compiles_identically_in_every_process() {
    let mut programs = corpus();
    programs.extend(canaries());
    assert_deterministic(programs, "all", false);
}
