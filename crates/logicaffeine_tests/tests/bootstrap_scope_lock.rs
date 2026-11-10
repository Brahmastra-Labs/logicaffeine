//! ★ LOCK — bootstrap programs run under reference semantics, and nothing else does. ★
//!
//! The compiler carries programs of its own written in LOGOS: the partial evaluator and its
//! dialects (`pe_source`, `pe_mini`, `pe_bti`), the self-interpreter, the decompiler, and the wire
//! encoder's driver. They thread their state through `mutable` parameters and expect every write to
//! land on the state itself, which is reference semantics. User programs run under value semantics.
//! `semantics::collections::with_reference_semantics` is the bootstrap scope that switches one to
//! the other for a single run.
//!
//! A bootstrap program written to mean the same on either host (the self-interpreter does) may also
//! be run outside the scope by a proof of exactly that — a test that compares it with its run inside
//! the scope. Each such proof is named in [`HOST_AGNOSTIC_PROOFS`]; every other run enters the scope.
//!
//! The halves:
//! - a source census: every function in `compile.rs` that runs or compiles a bootstrap program
//!   does it inside the scope, on every engine (tree-walker, VM, compiled Rust);
//! - the converse: nothing in the workspace enters the scope except those runs;
//! - behaviour: inside the scope the tree-walker, the VM and compiled Rust mean the same program —
//!   a plain binding shares, a `mutable` parameter is the caller's value, `copy of` is a value of
//!   its own — so a bootstrap program means one thing on every engine; the partial evaluator
//!   specializes a known value on the tree-walker and the VM alike; and an ordinary program run
//!   before, after or around any of that sees value semantics, even after a run inside the scope
//!   unwinds.

use logicaffeine_compile::compile::{
    interpret_program, projection1_source_real_fast, projection1_source_real_fast_on_vm,
    run_logos_source, tw_outcome, vm_outcome,
};
use logicaffeine_compile::semantics::collections::{
    reference_scope_active, with_reference_semantics,
};
use std::path::{Path, PathBuf};

/// A call that names one of the compiler's own LOGOS programs, or the types they share.
const BOOTSTRAP_SOURCES: &[&str] = &[
    "pe_source_text()",
    "pe_mini_source_text()",
    "pe_bti_source_text()",
    "decompile_source_text()",
    "core_interp_source()",
    "core_types_for_pe_source()",
    "pe_combined_source(",
    "genuine_p2_on_target_source(",
    "genuine_p3_on_target_source(",
    "pe_native_source()",
    "pe_cdylib_source()",
    "mix_cdylib_source()",
    "self_interpreter_run_source(",
    "textbook_projection1_subject_source(",
    "mix_source_text()",
    "CORE_TYPES_FOR_PE",
];

/// The test functions that run a bootstrap program outside the scope on purpose: each proves the
/// program means the same there as inside it, as `(file, function)`.
const HOST_AGNOSTIC_PROOFS: &[(&str, &str)] = &[
    ("futamura_textbook_lock.rs", "sint_on_the_value_host"),
    ("futamura_textbook_lock.rs", "specializer_on_the_value_host"),
    ("futamura_textbook_lock.rs", "textbook_on_the_value_host"),
];

/// A call that runs a program, or compiles one to Rust, on some engine.
const RUNNERS: &[&str] = &[
    "interpret_program(",
    "interpret_for_ui",
    "vm_run_source(",
    "run_to_outcome(",
    "run_treewalker(",
    "tw_outcome(",
    "vm_outcome(",
    "run_logos_source(",
    "compile_program_full(",
    "compile_program_full_with_wire(",
];

const SCOPE: &str = "with_reference_semantics(";

/// The one in-process entry for a bootstrap program: `interpret_program` inside the scope.
const BOOTSTRAP_ENTRY: &str = "interpret_bootstrap(";

/// Whether a runner line enters the bootstrap scope.
fn in_scope(line: &str) -> bool {
    line.contains(SCOPE) || line.contains(BOOTSTRAP_ENTRY)
}

fn compile_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../logicaffeine_compile/src")
}

/// Every top-level function of `text` as `(name, body lines)`, a body running to the next
/// top-level item.
fn functions(text: &str) -> Vec<(String, Vec<&str>)> {
    let lines: Vec<&str> = text.lines().collect();
    let name_of = |l: &str| -> Option<String> {
        let rest = ["pub fn ", "pub(crate) fn ", "fn "]
            .iter()
            .find_map(|p| l.strip_prefix(p))?;
        Some(
            rest.split(|c: char| c == '(' || c == '<')
                .next()
                .unwrap_or(rest)
                .to_string(),
        )
    };
    let starts: Vec<(usize, String)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| name_of(l).map(|n| (i, n)))
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(k, (start, name))| {
            let end = starts.get(k + 1).map_or(lines.len(), |(s, _)| *s);
            (name.clone(), lines[*start..end].to_vec())
        })
        .collect()
}

fn code(line: &str) -> bool {
    let l = line.trim_start();
    !l.starts_with("//") && !l.starts_with("///")
}

/// The functions of `compile.rs` that name a bootstrap program and run or compile a program, with
/// each runner line of theirs.
fn bootstrap_runners() -> Vec<(String, Vec<String>)> {
    let text = std::fs::read_to_string(compile_src().join("compile.rs")).expect("read compile.rs");
    functions(&text)
        .into_iter()
        .filter(|(_, body)| {
            body.iter()
                .any(|l| code(l) && BOOTSTRAP_SOURCES.iter().any(|m| l.contains(m)))
        })
        .filter_map(|(name, body)| {
            let runs: Vec<String> = body[1..]
                .iter()
                .filter(|l| {
                    code(l)
                        && (RUNNERS.iter().any(|m| l.contains(m)) || l.contains(BOOTSTRAP_ENTRY))
                })
                .map(|l| l.trim().to_string())
                .collect();
            (!runs.is_empty()).then_some((name, runs))
        })
        .collect()
}

#[test]
fn every_run_of_a_bootstrap_program_enters_the_reference_scope() {
    let runners = bootstrap_runners();
    let names: Vec<&str> = runners.iter().map(|(n, _)| n.as_str()).collect();
    for expected in [
        "projection1_source_real",
        "projection1_source_real_fast",
        "projection1_source_real_fast_on_vm",
        "program_to_core_wire_bytes_via_interpreter",
        "build_native_pe_binary",
        "native_pe_cdylib_lib_rs",
        "run_genuine_p2_on_target",
        "run_genuine_p3_on_target",
        "projection2_source_real",
        "projection3_source_real",
    ] {
        assert!(names.contains(&expected), "the census no longer sees `{expected}` run a bootstrap program — the reader is broken: {names:?}");
    }
    let outside: Vec<String> = runners
        .iter()
        .flat_map(|(name, runs)| {
            runs.iter()
                .filter(|l| !in_scope(l))
                .map(move |l| format!("{name}: {l}"))
        })
        .collect();
    assert!(
        outside.is_empty(),
        "bootstrap programs run outside the reference scope — wrap each run in `with_reference_semantics(|| …)`:\n{}",
        outside.join("\n")
    );
}

/// Test-side names that produce a bootstrap program: the sources above, and the helpers that build a
/// projection's compiler (a specialization of the partial evaluator, threading its state the same way).
const TEST_BOOTSTRAP_SOURCES: &[&str] = &[
    "get_p2_compiler()",
    "get_p3_cogen()",
    "pe_mini_source_text()",
    "pe_bti_source_text()",
];

/// The bootstrap programs' own `.logos` sources, as a test file includes one.
const BOOTSTRAP_LOGOS: &[&str] = &[
    "optimize/core_interp.logos",
    "optimize/pe_source.logos",
    "optimize/pe_mini_source.logos",
    "optimize/pe_bti_source.logos",
    "optimize/decompile_source.logos",
    "optimize/mix.logos",
];

/// Every name that produces a bootstrap program in the test file `text`: the names above, and each
/// constant the file binds to a bootstrap program's source with `include_str!` (`phase_futamura`'s
/// `INTERPRETER`, the self-interpreter).
fn bootstrap_names(text: &str) -> Vec<String> {
    let mut names: Vec<String> = BOOTSTRAP_SOURCES
        .iter()
        .chain(TEST_BOOTSTRAP_SOURCES)
        .map(|s| s.to_string())
        .collect();
    for line in text.lines().map(str::trim_start) {
        let Some(rest) = line
            .strip_prefix("const ")
            .or_else(|| line.strip_prefix("static "))
        else {
            continue;
        };
        if line.contains("include_str!(") && BOOTSTRAP_LOGOS.iter().any(|f| line.contains(f)) {
            if let Some(name) = rest.split(':').next() {
                names.push(name.trim().to_string());
            }
        }
    }
    names
}

/// Every `.rs` file under `tests/` but this one.
fn test_files() -> Vec<PathBuf> {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let this = Path::new(file!())
        .file_name()
        .expect("this file has a name")
        .to_owned();
    let mut files = Vec::new();
    let mut stack = vec![tests];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs")
                && p.file_name() != Some(this.as_os_str())
            {
                files.push(p);
            }
        }
    }
    assert!(
        files.len() > 1000,
        "the test walk found only {} files",
        files.len()
    );
    files
}

/// An in-process run a test makes.
const TEST_RUNNERS: &[&str] = &[
    "interpret_program(",
    "interpret_bootstrap_on_the_value_host(",
    "tw_outcome(",
    "vm_outcome(",
    "vm_run_source(",
    "interpret_for_ui",
];

/// Every test function that builds a bootstrap program and runs a program in-process runs at least
/// one of them through the bootstrap entry — a new in-process bootstrap runner outside the scope has
/// none. (The same function may also run the residual it produced, a user program, outside it.)
#[test]
fn every_test_that_runs_a_bootstrap_program_in_process_enters_the_scope() {
    let mut found = 0;
    let mut outside = Vec::new();
    for file in &test_files() {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        let bootstrap = bootstrap_names(&text);
        if !bootstrap.iter().any(|m| text.contains(m.as_str())) {
            continue;
        }
        for (name, body) in functions_indented(&text) {
            let names = body
                .iter()
                .any(|l| code(l) && bootstrap.iter().any(|m| l.contains(m.as_str())));
            let runs = body
                .iter()
                .any(|l| code(l) && (TEST_RUNNERS.iter().any(|m| l.contains(m)) || in_scope(l)));
            if names && runs {
                found += 1;
                let proof = HOST_AGNOSTIC_PROOFS.iter().any(|(f, n)| {
                    file.file_name().and_then(|x| x.to_str()) == Some(*f) && name == *n
                });
                if !proof && !body.iter().any(|l| code(l) && in_scope(l)) {
                    outside.push(format!("{}: fn {name}", file.display()));
                }
            }
        }
    }
    assert!(found >= 4, "the census found only {found} in-process bootstrap runners in the tests — the reader is broken");
    assert!(
        outside.is_empty(),
        "tests that run a bootstrap program in-process outside the scope — run it through `compile::interpret_bootstrap`:\n{}",
        outside.join("\n")
    );
}

/// A test's run of a program compiled to Rust (`tests/common/mod.rs`), or its compile of one.
const TEST_AOT_RUNNERS: &[&str] = &[
    "run_logos(",
    "run_logos_with_args(",
    "assert_output(",
    "assert_exact_output(",
    "assert_output_lines(",
    "compile_program_full(",
];

/// The runs of compiled Rust that compile and run inside the bootstrap scope (`tests/common/mod.rs`).
const TEST_BOOTSTRAP_AOT_RUNNERS: &[&str] =
    &["run_bootstrap_logos(", "assert_bootstrap_exact_output("];

/// The runs of compiled Rust a test that builds a bootstrap program makes of a user program — a
/// residual a projection produced, a target run directly — under value semantics.
const TEST_USER_AOT_RUNNERS: &[&str] = &["run_user_logos(", "assert_user_exact_output("];

/// Every test function that builds a bootstrap program and compiles a program to Rust says, at each
/// compile, which program it is: a bootstrap program compiles and runs inside the bootstrap scope —
/// through a bootstrap runner, or within `with_reference_semantics` — as `compile.rs` compiles it;
/// outside it the partial evaluator's writes to its threaded state land on copies and it
/// specializes nothing. A user program the same test runs — the residual it produced — goes through
/// a user runner, under value semantics.
#[test]
fn every_test_that_runs_a_bootstrap_program_as_compiled_rust_enters_the_scope() {
    let mut found = 0;
    let mut outside = Vec::new();
    for file in &test_files() {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        let bootstrap = bootstrap_names(&text);
        if !bootstrap.iter().any(|m| text.contains(m.as_str())) {
            continue;
        }
        for (name, body) in functions_indented(&text) {
            if !body
                .iter()
                .any(|l| code(l) && bootstrap.iter().any(|m| l.contains(m.as_str())))
            {
                continue;
            }
            let scoped = body.iter().any(|l| code(l) && l.contains(SCOPE));
            for line in body.iter().filter(|l| code(l)) {
                if TEST_BOOTSTRAP_AOT_RUNNERS.iter().any(|m| line.contains(m)) {
                    found += 1;
                } else if !scoped
                    && TEST_AOT_RUNNERS.iter().any(|m| line.contains(m))
                    && !TEST_USER_AOT_RUNNERS.iter().any(|m| line.contains(m))
                {
                    outside.push(format!("{}: fn {name}: {}", file.display(), line.trim()));
                }
            }
        }
    }
    assert!(
        outside.is_empty(),
        "tests that build a bootstrap program and compile a program to Rust without saying which — a bootstrap program runs through `common::run_bootstrap_logos` / `common::assert_bootstrap_exact_output`, a user program through `common::run_user_logos` / `common::assert_user_exact_output`:\n{}",
        outside.join("\n")
    );
    assert!(found >= 10, "the census found only {found} bootstrap runs of compiled Rust in the tests — the reader is broken");
}

/// Every function of `text`, at any indentation (a test file nests helpers in modules), as `(name,
/// body lines)`, a body running to the next function.
fn functions_indented(text: &str) -> Vec<(String, Vec<&str>)> {
    let lines: Vec<&str> = text.lines().collect();
    let name_of = |l: &str| -> Option<String> {
        let t = l.trim_start();
        let rest = ["pub fn ", "pub(crate) fn ", "fn "]
            .iter()
            .find_map(|p| t.strip_prefix(p))?;
        Some(
            rest.split(|c: char| c == '(' || c == '<')
                .next()
                .unwrap_or(rest)
                .to_string(),
        )
    };
    let starts: Vec<(usize, String)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| name_of(l).map(|n| (i, n)))
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(k, (start, name))| {
            let end = starts.get(k + 1).map_or(lines.len(), |(s, _)| *s);
            (name.clone(), lines[*start..end].to_vec())
        })
        .collect()
}

#[test]
fn nothing_but_a_bootstrap_run_enters_the_reference_scope() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    let mut stack: Vec<PathBuf> = ["crates", "apps"]
        .iter()
        .flat_map(|d| {
            std::fs::read_dir(root.join(d))
                .into_iter()
                .flatten()
                .flatten()
        })
        .map(|e| e.path().join("src"))
        .filter(|p| p.is_dir())
        .collect();
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                files.push(p);
            }
        }
    }
    assert!(
        files.len() > 200,
        "the source walk found only {} files",
        files.len()
    );
    let runners: Vec<String> = bootstrap_runners().into_iter().map(|(n, _)| n).collect();
    let compile_rs = compile_src()
        .join("compile.rs")
        .canonicalize()
        .expect("compile.rs");
    let collections_rs = compile_src()
        .join("semantics/collections.rs")
        .canonicalize()
        .expect("collections.rs");
    let mut entries = 0;
    let mut leaks = Vec::new();
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap_or_default();
        if !text.contains(SCOPE) && !text.contains("ReferenceScope::enter(") {
            continue;
        }
        let here = file.canonicalize().unwrap_or_else(|_| file.clone());
        for (name, body) in functions(&text) {
            let enters = body[1..]
                .iter()
                .any(|l| code(l) && (l.contains(SCOPE) || l.contains("ReferenceScope::enter(")));
            if !enters {
                continue;
            }
            entries += 1;
            let sanctioned = (here == compile_rs
                && (runners.contains(&name) || name == "interpret_bootstrap"))
                || (here == collections_rs && name == "with_reference_semantics");
            if !sanctioned {
                leaks.push(format!("{}: fn {name}", file.display()));
            }
        }
    }
    assert!(
        entries >= 10,
        "the census found only {entries} functions entering the scope — the reader is broken"
    );
    assert!(
        leaks.is_empty(),
        "functions that enter the reference scope without running a bootstrap program — user code must never run inside it:\n{}",
        leaks.join("\n")
    );
}

/// `a` and a copy `b` of it: value semantics shows 1 (the push lands on `b` alone), reference
/// semantics shows 2.
const COPY_THEN_PUSH: &str =
    "## Main\nLet mutable a be [1].\nLet mutable b be a.\nPush 2 to b.\nShow length of a.\n";

fn shown(outcome: logicaffeine_compile::compile::RunOutcome) -> String {
    assert_eq!(outcome.error, None, "the probe runs");
    outcome.output.trim().to_string()
}

/// What an ordinary program sees on every engine, with the scope's own flag.
fn ordinary() -> (bool, [String; 3]) {
    let via_entry = interpret_program(COPY_THEN_PUSH)
        .expect("the probe runs")
        .trim()
        .to_string();
    (
        reference_scope_active(),
        [
            shown(tw_outcome(COPY_THEN_PUSH)),
            shown(vm_outcome(COPY_THEN_PUSH)),
            via_entry,
        ],
    )
}

fn value_semantics() -> (bool, [String; 3]) {
    (false, ["1".to_string(), "1".to_string(), "1".to_string()])
}

#[test]
fn the_probe_tells_the_two_semantics_apart_on_every_engine() {
    assert_eq!(
        ordinary(),
        value_semantics(),
        "an ordinary program runs under value semantics"
    );
    let inside = with_reference_semantics(|| {
        (
            reference_scope_active(),
            [
                shown(tw_outcome(COPY_THEN_PUSH)),
                shown(vm_outcome(COPY_THEN_PUSH)),
            ],
        )
    });
    assert_eq!(
        inside,
        (true, ["2".to_string(), "2".to_string()]),
        "inside the scope the copy shares its list"
    );
}

const ST: &str = "## A St has:\n    an env, which is Map of Text to Int.\n    a log, which is Seq of Text.\n\n\
    ## To stEnv (st: St) -> Map of Text to Int:\n    Return st's env.\n\n\
    ## To stBind (st: mutable St) and (k: Text) and (v: Int):\n    Let mutable e be stEnv(st).\n    Set item k of e to v.\n    Set st to a new St with env e and log (st's log).\n\n";

const HOLDER: &str = "## A Holder has:\n    an m, which is Map of Text to Int.\n\n\
    ## To getM (h: Holder) -> Map of Text to Int:\n    Return h's m.\n\n\
    ## To poke (m: mutable Map of Text to Int):\n    Set item \"a\" of m to 9.\n\n";

const TAGGED: &str = "## A Tagged has:\n    a tags, which is Seq of Int.\n\n";

/// What a program means inside the scope, one row per rule: `(row, program, what it shows)`.
fn scope_rows() -> Vec<(&'static str, String, &'static str)> {
    vec![
        ("a plain binding shares its list", COPY_THEN_PUSH.to_string(), "2"),
        (
            "a mutable Int parameter is the caller's",
            "## To bump (n: mutable Int):\n    Set n to n + 1.\n\n## Main\nLet mutable k be 1.\nbump(k).\nShow k.\n".to_string(),
            "2",
        ),
        (
            "a mutable record parameter replaced whole is the caller's",
            "## A Box has:\n    an n, which is Int.\n\n## To bump (b: mutable Box):\n    Set b to a new Box with n (b's n + 1).\n\n\
             ## Main\nLet mutable b be a new Box with n 1.\nbump(b).\nShow b's n.\n".to_string(),
            "2",
        ),
        (
            "a copy of a call's map is a map of its own",
            format!("{HOLDER}## Main\nLet mutable inner be a new Map of Text to Int.\nSet item \"a\" of inner to 1.\n\
                     Let h be a new Holder with m inner.\nLet mutable c be copy of (getM(h)).\nSet item \"a\" of c to 9.\nShow item \"a\" of (getM(h)).\n"),
            "1",
        ),
        (
            "a copy of a call's map lent to a mutable parameter takes the write alone",
            format!("{HOLDER}## Main\nLet mutable inner be a new Map of Text to Int.\nSet item \"a\" of inner to 1.\n\
                     Let h be a new Holder with m inner.\nLet mutable c be copy of (getM(h)).\npoke(c).\nShow item \"a\" of (getM(h)).\nShow item \"a\" of c.\n"),
            "1\n9",
        ),
        (
            "a copy of a record keeps a push onto its field to itself",
            format!("{TAGGED}## Main\nLet mutable a be a new Tagged with tags [1].\nLet mutable b be copy of a.\nPush 9 to b's tags.\nShow a's tags.\nShow b's tags.\n"),
            "[1]\n[1, 9]",
        ),
        (
            "a copy of a nested list keeps a nested write to itself",
            "## Main\nLet mutable g be a new Seq of Seq of Int.\nLet mutable row be a new Seq of Int.\nPush 1 to row.\nPush row to g.\n\
             Let mutable c be copy of g.\nSet item 1 of item 1 of c to 9.\nShow item 1 of item 1 of g.\nShow item 1 of item 1 of c.\n".to_string(),
            "1\n9",
        ),
        (
            "a copy of a nested list has rows of its own",
            "## Main\nLet mutable g be a new Seq of Seq of Int.\nLet mutable row be a new Seq of Int.\nPush 1 to row.\nPush row to g.\n\
             Let mutable c be copy of g.\nLet mutable r be item 1 of c.\nPush 9 to r.\nShow g.\nShow c.\n".to_string(),
            "[[1]]\n[[1, 9]]",
        ),
        (
            "a state threaded through a mutable parameter, and a child built from a copy of its map",
            format!("{ST}## Main\nLet mutable st be a new St with env (a new Map of Text to Int) and log (a new Seq of Text).\nstBind(st, \"x\", 1).\n\
                     Let mutable child be a new St with env (copy of (stEnv(st))) and log (st's log).\nstBind(child, \"x\", 4).\n\
                     Show item \"x\" of (stEnv(st)).\nShow item \"x\" of (stEnv(child)).\n"),
            "1\n4",
        ),
    ]
}

/// Inside the scope every engine runs a program to the same answer: the tree-walker, the VM, and
/// the program compiled to Rust and run — the three ways `compile.rs` runs a bootstrap program.
#[test]
fn the_scope_means_the_same_on_every_engine() {
    let mut wrong = Vec::new();
    for (row, program, expected) in scope_rows() {
        let (tw, vm, aot) = with_reference_semantics(|| {
            (
                shown(tw_outcome(&program)),
                shown(vm_outcome(&program)),
                run_logos_source(&program).map(|out| out.trim().to_string()),
            )
        });
        let answers = [("tree-walker", Ok(tw)), ("VM", Ok(vm)), ("compiled Rust", aot)];
        for (engine, answer) in answers {
            if answer.as_deref() != Ok(expected) {
                wrong.push(format!("{row} — {engine}: {answer:?}, not {expected:?}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "inside the scope an engine means something else:\n{}",
        wrong.join("\n")
    );
    assert_eq!(ordinary(), value_semantics(), "the scope is left behind");
}

#[test]
fn the_partial_evaluator_specializes_a_known_value_on_every_engine() {
    let program = "## Main\nLet x be 5.\nShow x.\n";
    let tw = projection1_source_real_fast("", "", program)
        .expect("the PE runs on the tree-walker and the VM");
    let vm = projection1_source_real_fast_on_vm("", "", program).expect("the PE runs on the VM");
    assert_eq!(
        tw.trim(),
        "## Main\nShow 5.",
        "the residual shows the value the PE knows"
    );
    assert_eq!(
        vm.trim(),
        tw.trim(),
        "the VM's residual is the tree-walker's"
    );
    assert_eq!(
        ordinary(),
        value_semantics(),
        "the programs after a bootstrap run are back under value semantics"
    );
}

#[test]
fn a_failed_bootstrap_run_leaves_the_scope() {
    let failed = with_reference_semantics(|| {
        interpret_program("## Main\nLet xs be [1].\nShow item 5 of xs.\n")
    });
    assert!(failed.is_err(), "the fixture fails inside the scope");
    assert_eq!(
        ordinary(),
        value_semantics(),
        "a failure inside the scope does not keep it entered"
    );
}

#[test]
fn an_unwinding_bootstrap_run_leaves_the_scope() {
    let unwound = std::panic::catch_unwind(|| {
        with_reference_semantics(|| panic!("a bootstrap run that panics"))
    });
    assert!(unwound.is_err(), "the fixture panics inside the scope");
    assert_eq!(
        ordinary(),
        value_semantics(),
        "a panic inside the scope does not keep it entered"
    );
}

/// Every named host-agnosticism proof is still there and still runs its program outside the scope —
/// a renamed or retired proof leaves no stale exemption.
#[test]
fn every_host_agnosticism_proof_runs_outside_the_scope() {
    for (file, name) in HOST_AGNOSTIC_PROOFS {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let functions = functions_indented(&text);
        let body = functions
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b)
            .unwrap_or_else(|| panic!("{file} no longer has `fn {name}` — drop it from HOST_AGNOSTIC_PROOFS"));
        assert!(
            body.iter().any(|l| code(l) && TEST_RUNNERS.iter().any(|m| l.contains(m))),
            "{file}: `fn {name}` no longer runs anything — drop it from HOST_AGNOSTIC_PROOFS"
        );
        assert!(
            !body.iter().any(|l| code(l) && in_scope(l)),
            "{file}: `fn {name}` runs inside the scope — it proves nothing about the other host; drop it from HOST_AGNOSTIC_PROOFS"
        );
    }
}
