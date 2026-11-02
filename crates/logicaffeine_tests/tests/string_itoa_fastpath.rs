//! CZ4 — integer string-building uses a fast itoa path, not the `fmt::Display`
//! vtable. `Set result to result + i + " "` used to lower to
//! `write!(result, "{}{}", i, " ")`, routing every integer through the formatter
//! machinery. It now lowers to `result.push_i64(i); result.push(' ');` — the same
//! bytes, ~1.5× faster on integer-heavy string building (measured 20.7→13.6ms on a
//! 2M-int loop). These lock the codegen shape AND the byte-identical output,
//! including negatives (the sign path is the soundness risk).

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::fs;

use tempfile::tempdir;

use logicaffeine_compile::compile::{compile_project, compile_to_c, compile_to_c_via_bytecode};

/// Compile a single-file program and run it; return `(generated_rust, trimmed_stdout)`.
fn build_and_run(src: &str) -> (String, String) {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, src).unwrap();
    let out = compile_project(&main).expect("should lower to Rust");
    let r = common::run_rust_project(&out.rust_code, &out.dependencies, &[]);
    assert!(
        r.success,
        "generated program must compile+run.\nstderr: {}\n{}",
        r.stderr, out.rust_code
    );
    (out.rust_code, r.stdout.trim().to_string())
}

/// Build and run a generated C translation unit as a release C build does.
fn build_and_run_c(c: &str) -> String {
    let dir = tempdir().unwrap();
    let source = dir.path().join("main.c");
    let binary = dir.path().join("main");
    fs::write(&source, c).unwrap();
    let build = common::build_c_release(&source, &binary).expect("run the C compiler");
    assert!(
        build.status.success(),
        "generated C must compile.\nstderr: {}\n{c}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new(&binary)
        .output()
        .expect("run generated C");
    assert!(
        run.status.success(),
        "generated C must run.\nstderr: {}\n{c}",
        String::from_utf8_lossy(&run.stderr)
    );
    String::from_utf8_lossy(&run.stdout).trim().to_string()
}

const BUILD_DIGITS: &str = "## Main\n\
    Let mutable result be \"\" with capacity 60.\n\
    Let mutable i be 0.\n\
    While i is less than 10:\n\
    \x20   Set result to result + i + \" \".\n\
    \x20   Set i to i + 1.\n\
    Show result.\n";

/// The integer append lowers to the itoa fast path, and the `write!`/`Display`
/// vtable is gone for the integer operand.
#[test]
fn integer_append_uses_itoa_fast_path() {
    let (rust, out) = build_and_run(BUILD_DIGITS);
    assert!(
        rust.contains(".push_i64("),
        "integer string append must use the `.push_i64` itoa fast path:\n{rust}"
    );
    assert!(
        !rust.contains("write!(result"),
        "the integer operand must not route through `write!`/`Display`:\n{rust}"
    );
    // Byte-identical to what `write!` produced.
    assert_eq!(out, "0 1 2 3 4 5 6 7 8 9");
}

/// Negatives and the largest representable-magnitude value must be byte-identical
/// — the sign branch in `push_i64` is where a hand itoa most easily goes wrong.
/// (LOGOS caps `Int` literals at ±i64::MAX, so i64::MIN can't be a literal; the
/// `unsigned_abs` MIN edge is locked directly in `ops.rs`.) A computed 0 is
/// covered by `integer_append_uses_itoa_fast_path` (the `i=0` iteration).
#[test]
fn negative_append_is_byte_identical() {
    let (_rust, out) = build_and_run(
        "## Main\n\
         Let mutable s be \"\".\n\
         Set s to s + 7 + \",\".\n\
         Set s to s + (0 - 5) + \",\".\n\
         Set s to s + (0 - 9223372036854775807) + \",\".\n\
         Set s to s + 42 + \".\".\n\
         Show s.\n",
    );
    assert_eq!(out, "7,-5,-9223372036854775807,42.");
}

/// The additive-identity fold `x + 0 → x` is arithmetic-only. When `x` is a string
/// and `+` is concatenation, the `0` operand is a literal to append, not an identity
/// to drop: `"x" + 0` yields `"x0"`. Both fold surfaces are guarded — `fold_expr`'s
/// identity rewrite and the constant-propagation pass that re-folds after substitution
/// (`propagate.rs`), which is fed the scope's Text-symbol set so it refuses the rewrite
/// on string operands.
#[test]
fn string_plus_zero_literal_should_not_drop_the_zero() {
    let (_rust, out) =
        build_and_run("## Main\nLet mutable s be \"x\".\nSet s to s + 0.\nShow s.\n");
    assert_eq!(
        out, "x0",
        "`\"x\" + 0` must append '0', not identity-fold it away"
    );
}

/// The direct and bytecode C tiers use the same allocation-free decimal
/// rendering as Rust without changing bytes for zero, signs, or large values.
#[test]
fn c_backends_integer_append_is_byte_identical() {
    let source = "## Main\n\
        Let mutable s be \"\" with capacity 80.\n\
        Set s to s + 0 + \",\".\n\
        Set s to s + 7 + \",\".\n\
        Set s to s + (0 - 5) + \",\".\n\
        Set s to s + ((0 - 9223372036854775807) - 1) + \",\".\n\
        Set s to s + 9223372036854775807 + \".\".\n\
        Show s.\n";
    let expected = "0,7,-5,-9223372036854775808,9223372036854775807.";

    let direct = compile_to_c(source).expect("direct C lowers integer text appends");
    assert_eq!(
        build_and_run_c(&direct),
        expected,
        "direct C preserves decimal text"
    );

    let bytecode =
        compile_to_c_via_bytecode(source).expect("bytecode C lowers integer text appends");
    assert_eq!(
        build_and_run_c(&bytecode),
        expected,
        "bytecode C preserves decimal text"
    );
}

/// Borrowing text bytes must preserve the exact unsigned UTF-8 octets. This
/// covers both the zero-copy pointer loop and bytes above 127, where signed
/// C char promotion would otherwise corrupt the Int values.
#[test]
fn c_backends_text_bytes_view_preserves_unsigned_octets() {
    let source = "## To sumBytes (s: Text) -> Int:\n\
        \x20   Let bytes be text_bytes(s).\n\
        \x20   Let mutable total be 0.\n\
        \x20   Repeat for b in bytes:\n\
        \x20       Set total to total + b.\n\
        \x20   Return total.\n\
        \n\
        ## Main\n\
        Let raw be a new Seq of Int.\n\
        Push 104 to raw.\n\
        Push 195 to raw.\n\
        Push 169 to raw.\n\
        Let sample be text_from_bytes(raw).\n\
        Show sumBytes(sample).\n";
    // The byte sequence 68 C3 A9 encodes h followed by e-acute.
    let expected = "468";

    let direct = compile_to_c(source).expect("direct C lowers borrowed text bytes");
    assert_eq!(
        build_and_run_c(&direct),
        expected,
        "direct C widens unsigned UTF-8 bytes"
    );

    let bytecode =
        compile_to_c_via_bytecode(source).expect("bytecode C lowers borrowed text bytes");
    assert_eq!(
        build_and_run_c(&bytecode),
        expected,
        "bytecode C widens unsigned UTF-8 bytes"
    );
}
