//! The LOGOS Int spec: integer arithmetic is EXACT in every engine and build
//! profile — on i64 overflow it PROMOTES to an arbitrary-precision integer
//! rather than silently wrapping (the CWE-190 footgun). Results that fit i64
//! downsize back automatically. Each program runs through BOTH the tree-walker
//! and the bytecode VM and must produce the same exact output. (Wrapping is no
//! longer the default; it is opt-in via a `wrapping` marker — CRDT counters.)

use logicaffeine_compile::compile::{interpret_program, vm_run_source};

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_both_engines(src: &str, expected: &str) {
    let tw = interpret_program(src)
        .unwrap_or_else(|e| panic!("tree-walker failed on:\n{}\n{:?}", src, e));
    let vm = vm_run_source(src).unwrap_or_else(|e| panic!("vm failed on:\n{}\n{}", src, e));
    assert_eq!(norm(&tw), expected, "tree-walker output wrong for:\n{}", src);
    assert_eq!(norm(&vm), expected, "vm output wrong for:\n{}", src);
}

#[test]
fn add_at_i64_max_promotes_exactly() {
    // i64::MAX + 1 = 2^63 — the exact value, never the wrapped i64::MIN.
    assert_both_engines(
        "## Main\n\
         Let mutable x be 9223372036854775807.\n\
         Set x to x + 1.\n\
         Show x.\n",
        "9223372036854775808",
    );
}

#[test]
fn promote_then_subtract_downsizes_back_to_i64() {
    // Crossing the boundary up and back returns the exact narrow value (2^63 - 1).
    assert_both_engines(
        "## Main\n\
         Let mutable x be 9223372036854775807.\n\
         Set x to x + 1.\n\
         Set x to x - 1.\n\
         Show x.\n",
        "9223372036854775807",
    );
}

#[test]
fn mul_overflow_promotes_exactly() {
    // i64::MAX * 2 = 18446744073709551614 — exact, not the wrapped -2.
    assert_both_engines(
        "## Main\n\
         Let mutable x be 9223372036854775807.\n\
         Set x to x * 2.\n\
         Show x.\n",
        "18446744073709551614",
    );
}

#[test]
fn divide_a_promoted_value_that_fits_downsizes() {
    // 2^63 / -1 = -2^63 = i64::MIN, which fits i64 → exact and narrow.
    assert_both_engines(
        "## Main\n\
         Let mutable x be 9223372036854775807.\n\
         Set x to x + 1.\n\
         Let y be 0 - 1.\n\
         Set x to x / y.\n\
         Show x.\n",
        "-9223372036854775808",
    );
}

#[test]
fn modulo_of_a_promoted_value_is_exact() {
    // 2^63 % -1 = 0.
    assert_both_engines(
        "## Main\n\
         Let mutable x be 9223372036854775807.\n\
         Set x to x + 1.\n\
         Let y be 0 - 1.\n\
         Set x to x % y.\n\
         Show x.\n",
        "0",
    );
}
