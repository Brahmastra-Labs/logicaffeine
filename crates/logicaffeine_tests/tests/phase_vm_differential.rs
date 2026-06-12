//! Differential tests for the register bytecode VM against the production
//! tree-walker, over REAL parsed LOGOS source. Every program is executed by both
//! engines and their observable output must be identical. The tree-walker is the
//! independent oracle (VM_PLAN.md / EXODIA two-runtime split).

use logicaffeine_compile::compile::{interpret_program, vm_run_source};

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run `src` through both engines and assert identical output.
fn assert_vm_matches_treewalk(src: &str) {
    let tw = interpret_program(src).unwrap_or_else(|e| panic!("tree-walker failed on:\n{}\n{:?}", src, e));
    let vm = vm_run_source(src).unwrap_or_else(|e| panic!("vm failed on:\n{}\n{}", src, e));
    assert_eq!(norm(&vm), norm(&tw), "VM diverged from tree-walker for:\n{}", src);
}

#[test]
fn vm_diff_arithmetic() {
    assert_vm_matches_treewalk("## Main\nLet x be 5.\nLet y be x + 7.\nShow y.\n");
}

#[test]
fn vm_diff_arithmetic_precedence() {
    assert_vm_matches_treewalk(
        "## Main\nLet a be 6.\nLet b be a * 4.\nLet c be b - 2.\nShow c.\n",
    );
}

#[test]
fn vm_diff_while_loop_sum() {
    assert_vm_matches_treewalk(
        "## Main\n\
         Let mutable total be 0.\n\
         Let mutable i be 1.\n\
         While i is at most 5:\n\
         \x20   Set total to total + i.\n\
         \x20   Set i to i + 1.\n\
         Show total.\n",
    );
}

#[test]
fn vm_diff_if_else() {
    assert_vm_matches_treewalk(
        "## Main\n\
         Let x be 3.\n\
         If x is greater than 5:\n\
         \x20   Show \"big\".\n\
         Otherwise:\n\
         \x20   Show \"small\".\n",
    );
}

#[test]
fn vm_diff_nested_loop() {
    assert_vm_matches_treewalk(
        "## Main\n\
         Let mutable total be 0.\n\
         Let mutable i be 1.\n\
         While i is at most 3:\n\
         \x20   Let mutable j be 1.\n\
         \x20   While j is at most 3:\n\
         \x20       Set total to total + 1.\n\
         \x20       Set j to j + 1.\n\
         \x20   Set i to i + 1.\n\
         Show total.\n",
    );
}

#[test]
fn vm_diff_function_factorial() {
    assert_vm_matches_treewalk(
        "## To factorial (n: Int) -> Int:\n\
         \x20   If n is at most 1:\n\
         \x20       Return 1.\n\
         \x20   Return n * factorial(n - 1).\n\
         \n\
         ## Main\n\
         Show factorial(5).\n",
    );
}

#[test]
fn vm_diff_function_two_args() {
    assert_vm_matches_treewalk(
        "## To add (a: Int, b: Int) -> Int:\n\
         \x20   Return a + b.\n\
         \n\
         ## Main\n\
         Show add(3, 4).\n",
    );
}

#[test]
fn vm_diff_list_literal_index_length() {
    assert_vm_matches_treewalk(
        "## Main\n\
         Let items be [10, 20, 30].\n\
         Show length of items.\n\
         Show item 2 of items.\n",
    );
}

#[test]
fn vm_diff_seq_push_length() {
    assert_vm_matches_treewalk(
        "## Main\n\
         Let mutable items be a new Seq of Int.\n\
         Push 10 to items.\n\
         Push 20 to items.\n\
         Show length of items.\n",
    );
}

#[test]
fn vm_diff_list_set_index() {
    assert_vm_matches_treewalk(
        "## Main\n\
         Let mutable items be [1, 2, 3].\n\
         Set item 2 of items to 99.\n\
         Show item 2 of items.\n",
    );
}

// ---- Corpus-caught regressions: self-referential register hazards ---------
// The PE corpus exposed these: an expression assigned to a variable must be
// fully evaluated BEFORE the target register changes. (`Set result to
// "{result}{s}"` in a loop was silently dropping the accumulated prefix.)

#[test]
fn vm_set_self_referential_interpolation_accumulates() {
    assert_vm_matches_treewalk(
        "## Main\nLet mutable result be \"\".\nRepeat for s in [\"a\", \"b\", \"c\"]:\n    Set result to \"{result}{s}\".\nShow result.\n",
    );
    let out = logicaffeine_compile::compile::vm_outcome(
        "## Main\nLet mutable result be \"\".\nRepeat for s in [\"a\", \"b\", \"c\"]:\n    Set result to \"{result}{s}\".\nShow result.\n",
    );
    assert_eq!(out.output.trim(), "abc");
}

#[test]
fn vm_set_self_referential_short_circuit() {
    // `Set x to y and x` — the lhs write must not clobber x before rhs reads it.
    assert_vm_matches_treewalk(
        "## Main\nLet mutable x be 8.\nLet y be 2.\nSet x to y and x.\nShow x.\n",
    );
}

#[test]
fn vm_let_shadow_reads_outer_binding_in_value() {
    // TW evaluates the value in the OLD environment, then binds the new name.
    assert_vm_matches_treewalk(
        "## Main\nLet x be 5.\nIf true:\n    Let x be x + 1.\n    Show x.\nShow x.\n",
    );
}
