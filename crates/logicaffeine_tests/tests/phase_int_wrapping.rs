//! The LOGOS Int spec: wrapping i64 arithmetic, identical in every build
//! profile and in every engine. Each program runs through BOTH the tree-walker
//! and the bytecode VM and must produce the same output — including at the
//! overflow edges, where unwrapped native arithmetic would panic in debug
//! builds.

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
fn treewalk_and_vm_add_wraps_at_i64_max() {
    assert_both_engines(
        "## Main\n\
         Let mutable x be 9223372036854775807.\n\
         Set x to x + 1.\n\
         Show x.\n",
        "-9223372036854775808",
    );
}

#[test]
fn treewalk_and_vm_sub_wraps_at_i64_min() {
    // MIN is reached by wrapping (the lexer has no negative literal that large).
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
fn treewalk_and_vm_mul_wraps() {
    assert_both_engines(
        "## Main\n\
         Let mutable x be 9223372036854775807.\n\
         Set x to x * 2.\n\
         Show x.\n",
        "-2",
    );
}

#[test]
fn treewalk_and_vm_div_min_by_neg_one_wraps() {
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
fn treewalk_and_vm_mod_min_by_neg_one_is_zero() {
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
