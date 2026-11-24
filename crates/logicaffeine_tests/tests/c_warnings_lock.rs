//! ★ LOCK — the C a program compiles to builds without a warning. ★
//!
//! One row per warning the C compiler gave the benchmark and feature corpora, each built under that warning alone so a
//! row fails for its own reason. Every C program carries the whole runtime prelude, and the C compiler drops what the
//! program does not call: each function the prelude defines is marked `__attribute__((unused))`, so a program is not
//! reported for the runtime it leaves alone, and the prelude's own code reads as it runs.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test c_warnings_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{compile_to_c, compile_to_c_via_bytecode};

/// A program that uses next to nothing of the runtime: its C is the prelude and a `main`.
const SMALL: &str = "## Main\nShow 1.\n";

/// Whether `line` opens the definition of a file-scope `static` function: `static [inline] <type> <name>(`.
fn defines_a_static_function(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("static ") else {
        return false;
    };
    let Some((head, _)) = rest.split_once('(') else {
        return false;
    };
    !head.contains('=')
        && !head.contains(':')
        && head
            .trim_end()
            .rsplit(|c: char| c == ' ' || c == '*')
            .next()
            .is_some_and(|name| !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
}

/// `c` built under `flags`: whether it built, and what the compiler said.
fn built(c: &str, flags: &[&str]) -> (bool, String) {
    let dir = tempfile::tempdir().expect("temp dir");
    let src = dir.path().join("main.c");
    std::fs::write(&src, c).expect("write the C");
    let out = std::process::Command::new(common::c_compiler())
        .args(flags)
        .arg("-c")
        .arg(&src)
        .arg("-o")
        .arg(dir.path().join("main.o"))
        .output()
        .expect("run the C compiler");
    (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn every_function_the_prelude_defines_may_go_unused() {
    let c = compile_to_c(SMALL).expect("C for a small program");
    assert!(c.contains("lc_gc_alloc"), "the small program carries the prelude:\n{c}");
    let bare: Vec<&str> = c.lines().filter(|l| defines_a_static_function(l)).collect();
    assert!(
        bare.is_empty(),
        "{} prelude functions are not marked `__attribute__((unused))`:\n{}",
        bare.len(),
        bare.iter().take(20).copied().collect::<Vec<_>>().join("\n")
    );
    let (ok, said) = built(&c, &["-std=gnu17", "-O2", "-Wunused-function", "-Werror"]);
    assert!(ok, "a program is reported for the runtime it does not call:\n{said}");
}

#[test]
fn no_prelude_line_reads_as_guarded_when_it_is_not() {
    let c = compile_to_c(SMALL).expect("C for a small program");
    let (ok, said) = built(&c, &["-std=gnu17", "-O2", "-Wmisleading-indentation", "-Werror"]);
    assert!(ok, "the prelude is misleadingly indented:\n{said}");
}

/// A closure takes its environment first whether or not it captures anything (one calling convention for every function
/// value); one that captures nothing says it leaves its environment alone.
#[test]
fn a_closure_that_captures_nothing_builds_without_an_unused_parameter() {
    let src = "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n\
               ## Main\nLet square be (y: Int) -> y * y.\nShow apply(square, 7).\n";
    let c = compile_to_c(src).expect("C for a program with a closure");
    assert!(c.contains("void *__env"), "the closure is lifted to a function taking its environment:\n{c}");
    let (ok, said) = built(&c, &["-std=gnu17", "-O2", "-Wunused-parameter", "-Werror"]);
    assert!(ok, "a closure that captures nothing leaves a parameter unused:\n{said}");
}

/// An `Inspect` arm that names a field it never reads takes nothing for it.
#[test]
fn an_inspect_arm_binds_only_what_it_reads() {
    let src = "## A Shape is one of:\n    A Circle with radius Int.\n    A Square with side Int.\n\n\
               ## To kind (s: Shape) -> Text:\n    Inspect s:\n        When Circle (r):\n            Return \"round\".\n\
               \x20       When Square (d):\n            Return \"square of \" + d.\n\n\
               ## Main\nShow kind(a new Circle with radius 2).\nShow kind(a new Square with side 5).\n";
    let c = compile_to_c(src).expect("C for a program that inspects");
    assert!(c.contains(".data.Square.side"), "the arm that reads its field takes it:\n{c}");
    let (ok, said) = built(&c, &["-std=gnu17", "-O2", "-Wunused-variable", "-Werror"]);
    assert!(ok, "an arm takes a field it never reads:\n{said}");
}

/// Bytecode C carries every function of the compiled program; one no call reaches is dropped by the C compiler, as a
/// prelude function is, and says so.
#[test]
fn bytecode_c_marks_every_function_it_carries_as_one_that_may_go_unused() {
    let src = "## To spare (x: Int) -> Int:\n    Return x * 3.\n\n## Main\nShow 1.\n";
    let c = compile_to_c_via_bytecode(src).expect("bytecode C for a program with an uncalled function");
    assert!(c.contains(" fn_"), "the program's function is carried:\n{c}");
    let (ok, said) = built(&c, &["-std=gnu17", "-O2", "-Wunused-function", "-Werror"]);
    assert!(ok, "a program is reported for a function it does not call:\n{said}");
}

/// The reader of the first row is a check that can fail.
#[test]
fn the_reader_sees_a_bare_static_function_and_passes_the_rest() {
    assert!(defines_a_static_function("static int64_t logos_now(void) {"));
    assert!(defines_a_static_function("static inline LInt lint_of(int64_t v) { return v; }"));
    assert!(defines_a_static_function("static void logos_civil_from_nanos(int64_t nanos, int64_t *y,"));
    assert!(!defines_a_static_function("__attribute__((unused)) static int64_t logos_now(void) {"));
    assert!(!defines_a_static_function("static size_t lc_gc_since, lc_gc_trigger = (size_t)8 << 20;"));
    assert!(!defines_a_static_function("static const uint32_t lc_gc_class_size[LC_GC_CLASSES] = {"));
    assert!(!defines_a_static_function("static lc_gc_range *lc_gc_roots; static size_t lc_gc_roots_n;"));
}
