//! A program compiles to the same Rust and the same C in every process.
//!
//! Every program the tree holds (`support/program_corpus.rs`, discovered at run time) is compiled by
//! `compile_to_rust` and by `compile_to_c` in two separate processes running side by side; the sample is compiled
//! twice in each, so state one compile leaves for the next shows too. Each process seeds its hash maps differently,
//! and each map a process builds is seeded differently again. Both sources must be identical, byte for byte.
//!
//! A difference means an emitter writes in the iteration order of an unordered map — MD5's constant tables were
//! hoisted in whatever order a `HashMap` yielded them — which makes a build of the program a function of the
//! process: no reproducible build, no content-addressed artifact, no diff of one compiler's output against the
//! next. `compile_determinism_lock` holds the bytecode the same way.
//!
//! The default row compiles a seeded sample of the corpus, and the canaries: MD5 as the fixtures write it, a function
//! whose constant tables the emitter hoists, loops that read many arrays, pairs of functions that tail-call each other
//! and a join whose tasks capture many lists, so a map of hoisted tables, accesses, functions or captures has many
//! orders to fall into. The ignored row compiles all of it, once in each process.

#[path = "support/rust_literals.rs"]
mod rust_literals;
#[path = "support/program_corpus.rs"]
mod program_corpus;

use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use logicaffeine_compile::compile::{compile_to_c, compile_to_rust};
use program_corpus::{content_id, corpus, sample, Program};

const SAMPLE: usize = 300;
const SEED: u64 = 0x5EED_50C0_2026_1119;

/// Threads per child process: `LOGOS_DETERMINISM_THREADS`, else half the host's.
fn threads() -> usize {
    std::env::var("LOGOS_DETERMINISM_THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map_or(2, |n| n.get()).div_ceil(2))
        .clamp(1, 32)
}

/// A function holding `count` constant tables, each built by pushes of constants and read only by a dynamic index,
/// as MD5's shift tables are: the emitter hoists each into a stack array at the top of the body.
fn tables_canary(count: usize) -> Program {
    let mut body = String::new();
    for t in 1..=count {
        body.push_str(&format!("    Let mutable r{t} be a new Seq of Int.\n"));
        for v in 0..4 {
            body.push_str(&format!("    Push {} to r{t}.\n", t * 10 + v));
        }
    }
    let reads: String = (1..=count).map(|t| format!(" + item ((i % 4) + 1) of r{t}")).collect();
    let src = format!(
        "## To mix (n: Int) -> Int:\n{body}    Let mutable s be 0.\n    Let mutable i be 0.\n    While i is less than n:\n        Set s to s{reads}.\n        Set i to i + 1.\n    Return s.\n\n## Main\nShow mix(9).\n"
    );
    Program { id: content_id(&src), origin: format!("canary with {count} constant tables"), source: src }
}

/// A function whose loop reads `width` sequence parameters up to a bound it cannot prove: the emitter hoists one
/// bounds guard per sequence.
fn sequences_canary(width: usize) -> Program {
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

/// `pairs` pairs of functions that tail-call each other: the emitter merges each pair into one loop named for both.
fn mutual_canary(pairs: usize) -> Program {
    let mut src = String::new();
    for k in 0..pairs {
        src.push_str(&format!(
            "## To ping{k} (n: Int) -> Int:\n    If n is less than 1:\n        Return {k}.\n    Return pong{k}(n - 1).\n\n## To pong{k} (n: Int) -> Int:\n    If n is less than 1:\n        Return 0.\n    Return ping{k}(n - 1).\n\n"
        ));
    }
    src.push_str("## Main\n");
    for k in 0..pairs {
        src.push_str(&format!("Show ping{k}(5).\n"));
    }
    Program { id: content_id(&src), origin: format!("canary with {pairs} mutually recursive pairs"), source: src }
}

/// A join of `width` tasks, each reading all of `width` lists: the emitter clones every list each task captures.
fn join_canary(width: usize) -> Program {
    let lists: String = (0..width).map(|k| format!("    Let l{k} be [{k}, {}].\n", k + 1)).collect();
    let reads: Vec<String> = (0..width).map(|k| format!("size(l{k})")).collect();
    let tasks: String = (0..width).map(|k| format!("        Let t{k} be {}.\n", reads.join(" + "))).collect();
    let shown: Vec<String> = (0..width).map(|k| format!("t{k}")).collect();
    let src = format!(
        "## To size (items: Seq of Int) -> Int:\n    Return length of items.\n\n## Main\n{lists}    Attempt all of the following:\n{tasks}    Show {}.\n",
        shown.join(" + ")
    );
    Program { id: content_id(&src), origin: format!("canary with a join of {width} tasks"), source: src }
}

/// MD5 as the fixtures write it, whose round function's four shift tables the emitter hoists.
fn md5_canary() -> Program {
    let src = include_str!("fixtures/md5_logos.lg").to_string();
    Program { id: content_id(&src), origin: "fixtures/md5_logos.lg".to_string(), source: src }
}

fn canaries() -> Vec<Program> {
    let mut all: Vec<Program> = vec![md5_canary()];
    all.extend([2, 4, 8].into_iter().map(tables_canary));
    all.extend([2, 4, 8].into_iter().map(sequences_canary));
    all.extend([2, 8].into_iter().map(mutual_canary));
    all.extend([2, 8].into_iter().map(join_canary));
    all
}

/// One emitter's source for `source`, or why there is none.
fn emitted(emit: fn(&str) -> Result<String, logicaffeine_compile::ParseError>, source: &str) -> String {
    match std::panic::catch_unwind(|| emit(source)) {
        Ok(Ok(code)) => code,
        Ok(Err(_)) => "does not compile\n".to_string(),
        Err(_) => "the emitter panicked\n".to_string(),
    }
}

/// The program's Rust and its C, each under a header line.
fn compiled(source: &str) -> String {
    format!("\u{2}rust\n{}\n\u{2}c\n{}\n", emitted(compile_to_rust, source), emitted(compile_to_c, source))
}

/// The child: compile every program in `LOGOS_DETERMINISM_IN` twice and write both renderings, program by program,
/// into the directory `LOGOS_DETERMINISM_OUT` — file `<n>` for the n-th program, its two renderings split by
/// `\u{1f}\n`. Without those variables it is not a child and does nothing.
#[test]
#[ignore]
fn determinism_child() {
    let (Ok(input), Ok(output)) = (std::env::var("LOGOS_DETERMINISM_IN"), std::env::var("LOGOS_DETERMINISM_OUT"))
    else {
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
                let second = if twice { compiled(source) } else { first.clone() };
                let mut file =
                    std::io::BufWriter::new(std::fs::File::create(output.join(i.to_string())).expect("a rendering file"));
                file.write_all(first.as_bytes()).expect("write a rendering");
                file.write_all("\u{1f}\n".as_bytes()).expect("write a rendering");
                file.write_all(second.as_bytes()).expect("write a rendering");
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

/// Run the child over the program list in a fresh process; the directory its renderings are in.
fn child(dir: &std::path::Path, tag: &str, twice: bool) -> std::path::PathBuf {
    let output = dir.join(format!("renderings-{tag}"));
    let status = Command::new(std::env::current_exe().expect("this test binary"))
        .args(["determinism_child", "--exact", "--ignored", "--test-threads", "1"])
        .env("LOGOS_DETERMINISM_IN", dir.join("programs"))
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

/// The emitter whose source differs first between `a` and `b` (`rust` or `c`), with its first differing line from
/// each side.
fn first_difference(a: &str, b: &str) -> Option<String> {
    let mut emitter = "";
    for (x, y) in a.lines().zip(b.lines()) {
        if let Some(name) = x.strip_prefix('\u{2}') {
            emitter = name;
        }
        if x != y {
            return Some(format!("{emitter}: `{}` vs `{}`", x.trim(), y.trim()));
        }
    }
    (a.lines().count() != b.lines().count()).then(|| format!("{emitter}: {} vs {} lines", a.lines().count(), b.lines().count()))
}

/// `twice`: each process also compiles every program a second time, which catches state one compile leaves for the
/// next; the two processes alone catch every per-process order.
fn assert_deterministic(programs: Vec<Program>, name: &str, twice: bool) {
    let dir = std::env::temp_dir().join(format!("emitted-source-determinism-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    write_programs(&programs, &dir);
    let (a, b) = std::thread::scope(|s| {
        let a = s.spawn(|| child(&dir, "a", twice));
        let b = s.spawn(|| child(&dir, "b", twice));
        (a.join().expect("process a"), b.join().expect("process b"))
    });
    let mut bad = Vec::new();
    let mut emitted_rust = 0;
    for (n, p) in programs.iter().enumerate() {
        let (Some((a1, a2)), Some((b1, b2))) = (renderings(&a, n), renderings(&b, n)) else {
            bad.push(format!("{} ({}): a child wrote no rendering", p.id, p.origin));
            continue;
        };
        if !a1.starts_with("\u{2}rust\ndoes not compile") {
            emitted_rust += 1;
        }
        for (what, x, y) in [("within process a", &a1, &a2), ("within process b", &b1, &b2), ("across processes", &a1, &b1)]
        {
            if let Some(first) = first_difference(x, y) {
                bad.push(format!("{} ({}) differs {what}: {first}", p.id, p.origin));
                break;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(emitted_rust > 0, "the corpus emitted nothing");
    assert!(
        bad.is_empty(),
        "{} of {} programs emit different source in different processes:\n{}",
        bad.len(),
        programs.len(),
        bad.join("\n")
    );
}

/// The canaries reach the code they are written for: each table canary's tables are hoisted, so their order is
/// one the emitter chose.
#[test]
fn the_table_canaries_hoist_their_tables() {
    for count in [2, 4, 8] {
        let rust = compile_to_rust(&tables_canary(count).source).expect("the canary compiles");
        for t in 1..=count {
            assert!(rust.contains(&format!("let r{t}: [i64; 4] = [")), "r{t} is hoisted:\n{rust}");
        }
    }
    let md5 = compile_to_rust(&md5_canary().source).expect("MD5 compiles");
    for t in 1..=4 {
        assert!(md5.contains(&format!("let r{t}: [i64; 4] = [")), "MD5's r{t} is hoisted:\n{md5}");
    }
}

/// The mutual and join canaries reach the code they are written for: every pair is merged into one loop, and every
/// task but the last clones the lists it captures.
#[test]
fn the_mutual_and_join_canaries_reach_their_code() {
    let mutual = compile_to_rust(&mutual_canary(8).source).expect("the mutual canary compiles");
    for k in 0..8 {
        assert!(
            mutual.contains(&format!("fn ping{k}(n: i64) -> i64 {{ __mutual_ping{k}_pong{k}(0, n) }}")),
            "ping{k} and pong{k} are one loop, named in declaration order:\n{mutual}"
        );
    }
    let join = compile_to_rust(&join_canary(8).source).expect("the join canary compiles");
    let clones: String = (0..8).map(|k| format!("let l{k} = l{k}.clone(); ")).collect();
    assert!(join.contains(&format!("{{ {clones}async move")), "each task clones l0..l7 in name order:\n{join}");
}

#[test]
fn a_sample_of_the_corpus_and_the_canaries_emit_identically_in_every_process() {
    let mut programs = sample(&corpus(), SAMPLE, SEED);
    programs.extend(canaries());
    assert_deterministic(programs, "sample", true);
}

#[test]
#[ignore]
fn the_whole_corpus_emits_identically_in_every_process() {
    let mut programs = corpus();
    programs.extend(canaries());
    assert_deterministic(programs, "all", false);
}
