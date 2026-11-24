//! ★ LOCK — a recursion whose step is an affine map of its value one below is answered in closed form by the shared
//! optimizer, the same on every backend. ★
//!
//! `f(0) = C; f(n) = k + f(n − 1) + f(n − 1)` is `(C + k)·2^n − k`. binary_trees' fused `check(bottomUpTree(d))` is
//! that recursion (`1 + f(d − 1) + f(d − 1)`). `optimize::closed_form` answers it behind a guard `0 <= n <= N`, `N` the
//! largest `n` whose answer fits a word, over the recursion as written, which answers every other argument as the
//! reference tiers do. AST C answered the fused check by its double recursion (354 ms at depth 20, where AOT Rust
//! took under 1 ms), and AOT Rust answered it with a closed form of its own, written at codegen: one that wrapped past
//! the word (`pow2(63)` printed -9223372036854775808) and ran under `## No closedform` too. A function the closed form
//! answers is not cached: it answers its range without a call.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test closed_form_recursion_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_c_output, run_all_tiers, run_logos};
use logicaffeine_compile::compile::{compile_to_c, compile_to_rust};
use logicaffeine_compile::optimization::decorate_source;

fn benchmark(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../benchmarks/programs/{name}/main.lg"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The text of the function `name` the code defines, from the line that opens its body to the end of it.
fn function<'c>(code: &'c str, name: &str) -> &'c str {
    let mut at = 0;
    let start = loop {
        let found = code[at..].find(&format!("{name}(")).unwrap_or_else(|| panic!("no definition of `{name}` in:\n{code}"));
        let line_start = code[..at + found].rfind('\n').map_or(0, |n| n + 1);
        let line_end = code[at + found..].find('\n').map_or(code.len(), |n| at + found + n);
        if code[line_start..line_end].trim_end().ends_with('{') {
            break line_start;
        }
        at = line_end;
    };
    let end = code[start..].find("\n}\n").map_or(code.len(), |e| start + e + 3);
    &code[start..end]
}

/// Whether `code` shifts.
fn shifts(code: &str) -> bool {
    code.contains("shl(") || code.contains("<<")
}

/// `pow2` doubled by recursion, shown at `d`.
fn pow2_at(d: i64) -> String {
    format!(
        "## To pow2 (d: Int) -> Int:\n    If d equals 0:\n        Return 1.\n    Return pow2(d - 1) + pow2(d - 1).\n\n## Main\nShow pow2({d}).\n"
    )
}

/// AST C answers binary_trees' fused check by the closed form: the depths up to 61 without a call.
#[test]
fn ast_c_answers_the_fused_binary_trees_check_in_closed_form() {
    let c = compile_to_c(&benchmark("binary_trees")).expect("C for binary_trees");
    let check = function(&c, "check_of_bottomUpTree");
    assert!(
        check.contains("61") && shifts(check),
        "the fused check is not answered in closed form on AST C:\n{check}"
    );
    let body = &check[check.find('{').expect("a body") + 1..];
    assert!(!body.contains("check_of_bottomUpTree("), "the closed form calls itself on AST C:\n{check}");
}

/// AOT Rust answers the fused check by the same closed form and caches nothing in front of it. The closed form calls
/// no function of its own (the recursion it falls to is one of its own), so the compiler inlines it into the loop that
/// checks one depth many times and lifts it out.
#[test]
fn aot_rust_answers_the_fused_check_in_closed_form_and_caches_nothing() {
    let rust = compile_to_rust(&benchmark("binary_trees")).expect("Rust for binary_trees");
    let check = function(&rust, "fn check_of_bottomUpTree");
    assert!(check.contains("61") && shifts(check), "the fused check is not answered in closed form:\n{check}");
    assert!(!check.contains("__MEMO_"), "a function the closed form answers is cached:\n{check}");
    let body = &check[check.find('{').expect("a body") + 1..];
    assert!(!body.contains("check_of_bottomUpTree("), "the closed form calls itself:\n{check}");
}

/// Recursions doubled with a constant, shifted and scaled, every tier and both compiled backends print alike at every
/// depth from 0 to 12: the closed form where it fires, the recursion where it does not.
#[test]
fn every_tier_agrees_on_doubled_recursions() {
    for (step, base) in [
        ("f(d - 1) + f(d - 1)", 1),
        ("1 + f(d - 1) + f(d - 1)", 1),
        ("f(d - 1) + f(d - 1) + 3", 0),
        ("2 * f(d - 1) + 1", 4),
        ("f(d - 1) - 2 + f(d - 1)", 5),
        ("f(d - 1) + 7", 2),
    ] {
        let src = format!(
            "## To f (d: Int) -> Int:\n    If d equals 0:\n        Return {base}.\n    Return {step}.\n\n## Main\nRepeat for d from 0 to 12:\n    Show f(d).\n"
        );
        let agreed = run_all_tiers(&src);
        assert!(agreed.error.is_none(), "`{step}`: {:?}", agreed.error);
        let expected = agreed.output.trim().to_string();
        let aot = run_logos(&src);
        assert!(aot.success, "`{step}` on AOT Rust:\n{}\n{}", aot.stderr, aot.rust_code);
        assert_eq!(aot.stdout.trim(), expected, "`{step}` on AOT Rust:\n{}", aot.rust_code);
        assert_c_output(&src, &expected);
    }
}

/// Past the word the recursion answers: `pow2(63)` is 2^63. AOT Rust and AST C print it or stop with a sentence;
/// neither prints a wrapped number.
#[test]
fn past_the_word_no_backend_prints_a_wrapped_number() {
    let src = pow2_at(63);
    let exact = "9223372036854775808";
    let aot = run_logos(&src);
    assert!(
        (aot.success && aot.stdout.trim() == exact) || (!aot.success && aot.stdout.trim().is_empty()),
        "AOT Rust printed {:?} (stderr {:?}); pow2(63) is {exact}:\n{}",
        aot.stdout,
        aot.stderr,
        aot.rust_code
    );
    let c = compile_to_c(&src).expect("C for pow2");
    let dir = tempfile::tempdir().expect("a temp dir");
    let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_path, &c).expect("write the C");
    let built = common::build_c_release(&c_path, &bin).expect("run the C compiler");
    assert!(built.status.success(), "the C does not build:\n{}", String::from_utf8_lossy(&built.stderr));
    let run = std::process::Command::new(&bin).output().expect("run the C");
    let out = String::from_utf8_lossy(&run.stdout);
    assert!(
        (run.status.success() && out.trim() == exact) || (!run.status.success() && out.trim().is_empty()),
        "AST C printed {out:?} (stderr {:?}); pow2(63) is {exact}",
        String::from_utf8_lossy(&run.stderr)
    );
}

/// `## No closedform`, for the program or for the function alone, keeps the recursion as written on AOT Rust.
#[test]
fn no_closedform_keeps_the_recursion() {
    let program_off = decorate_source(&pow2_at(10), &["closedform"]);
    let function_off = format!("## No closedform\n{}", pow2_at(10));
    for (what, src) in [("the program", program_off), ("the function", function_off)] {
        let rust = compile_to_rust(&src).expect("Rust for pow2");
        let pow2 = function(&rust, "fn pow2");
        assert!(
            !shifts(pow2),
            "`## No closedform` on {what} still answers pow2 in closed form:\n{pow2}"
        );
        let run = run_logos(&src);
        assert_eq!(run.stdout.trim(), "1024", "{what}:\n{}\n{}", run.stderr, run.rust_code);
    }
}

/// A recursion whose step is not an affine map of its value one below keeps its recursion: `fib` stays cached, a
/// square of the value below stays as written.
#[test]
fn a_step_outside_the_shape_keeps_its_recursion() {
    let fib = "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\nShow fib(20).\n";
    let rust = compile_to_rust(fib).expect("Rust for fib");
    let f = function(&rust, "fn fib");
    assert!(f.contains("__MEMO_"), "fib is no longer cached:\n{f}");
    assert!(!shifts(f), "fib is answered in a closed form it does not have:\n{f}");
    let square = "## To sq (d: Int) -> Int:\n    If d equals 0:\n        Return 2.\n    Return sq(d - 1) * sq(d - 1).\n\n## Main\nShow sq(4).\n";
    let rust = compile_to_rust(square).expect("Rust for sq");
    let f = function(&rust, "fn sq");
    assert!(!shifts(f), "a square is answered as a doubling:\n{f}");
    let agreed = run_all_tiers(square);
    assert_eq!(agreed.output.trim(), "65536");
}
