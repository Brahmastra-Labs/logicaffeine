//! Error-string parity: the Err text and the partial output emitted before the
//! error are part of the LOGOS spec. Every program here runs through BOTH
//! engines via the SAME front-end; outcome (output + error) must be identical.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The differential contract: output AND error must match.
fn assert_outcome_parity(src: &str) {
    let tw = tw_outcome(src);
    let vm = vm_outcome(src);
    assert_eq!(
        norm(&vm.output),
        norm(&tw.output),
        "partial output diverged for:\n{}\nvm: {:?}\ntw: {:?}",
        src,
        vm,
        tw
    );
    assert_eq!(vm.error, tw.error, "error diverged for:\n{}", src);
}

#[test]
fn err_parity_div_by_zero() {
    assert_outcome_parity("## Main\nLet x be 1 / 0.\nShow x.\n");
}

#[test]
fn err_parity_mod_by_zero() {
    assert_outcome_parity("## Main\nLet x be 1 % 0.\nShow x.\n");
}

#[test]
fn err_parity_index_out_of_bounds() {
    assert_outcome_parity("## Main\nLet xs be [1, 2].\nShow item 5 of xs.\n");
}

#[test]
fn err_parity_index_zero() {
    assert_outcome_parity("## Main\nLet xs be [1, 2].\nShow item 0 of xs.\n");
}

#[test]
fn err_parity_negative_index_prints_wrapped_usize() {
    // A negative index wraps through `as usize`; both engines print the
    // wrapped number — pinned behavior.
    assert_outcome_parity(
        "## Main\nLet xs be [1, 2].\nLet i be 0 - 1.\nShow item i of xs.\n",
    );
}

#[test]
fn err_parity_set_undefined_variable() {
    // The tree-walker fails the assignment at RUNTIME, after evaluating the
    // value and after earlier statements produced output.
    assert_outcome_parity("## Main\nShow 1.\nSet ghost to 5.\nShow 2.\n");
}

#[test]
fn err_parity_partial_output_before_error() {
    assert_outcome_parity(
        "## Main\nShow \"a\".\nShow \"b\".\nLet x be 1 / 0.\nShow \"c\".\n",
    );
}

#[test]
fn outcome_parity_policy_check_passes_and_fails() {
    // Predicate passes.
    assert_outcome_parity(
        "## Definition\n\
         A User has:\n\
         \x20   a role, which is Text.\n\
         \n\
         ## Policy\n\
         A User is admin if the user's role equals \"admin\".\n\
         \n\
         ## Main\n\
         Let u be a new User with role \"admin\".\n\
         Check that the u is admin.\n\
         Show \"passed\".\n",
    );
    // Predicate fails: Security Check Failed + partial output preserved.
    assert_outcome_parity(
        "## Definition\n\
         A User has:\n\
         \x20   a role, which is Text.\n\
         \n\
         ## Policy\n\
         A User is admin if the user's role equals \"admin\".\n\
         \n\
         ## Main\n\
         Show \"before\".\n\
         Let u be a new User with role \"guest\".\n\
         Check that the u is admin.\n\
         Show \"after\".\n",
    );
}

#[test]
fn outcome_parity_registry_struct_defaults() {
    // The struct definition lives in the discovery registry (## Definition),
    // not in a Stmt::StructDef — default-fill must still work in the VM.
    assert_outcome_parity(
        "## Definition\n\
         A Point has:\n\
         \x20   an x, which is an Int.\n\
         \x20   a y, which is an Int.\n\
         \n\
         ## Main\n\
         Let p be a new Point with x 3.\n\
         Show p's x.\n\
         Show p's y.\n",
    );
}

#[test]
fn outcome_parity_read_console_and_file_without_vfs() {
    // Console reads yield empty Text in both engines.
    assert_outcome_parity("## Main\nRead input from the console.\nShow input.\nShow 7.\n");
    // File reads without a VFS error identically (after the partial output).
    assert_outcome_parity(
        "## Main\nShow \"start\".\nRead data from file \"data.txt\".\nShow data.\n",
    );
}

#[test]
fn err_parity_division_in_loop_partial_output() {
    // The error fires mid-loop; both engines keep the iterations already shown.
    assert_outcome_parity(
        "## Main\n\
         Let mutable i be 3.\n\
         While i is at least 0:\n\
         \x20   Show 6 / i.\n\
         \x20   Set i to i - 1.\n",
    );
}

#[test]
fn outcome_parity_push_to_struct_field() {
    // Positive: push into a struct's List field through the shared allocation.
    assert_outcome_parity(
        "## Definition\n\
         A Box has:\n\
         \x20   a items, which is a List of Int.\n\
         \n\
         ## Main\n\
         Let xs be [1].\n\
         Let b be a new Box with items xs.\n\
         Push 2 to b's items.\n\
         Show length of b's items.\n",
    );
    // Field is not a List.
    assert_outcome_parity(
        "## Definition\n\
         A Box has:\n\
         \x20   a n, which is an Int.\n\
         \n\
         ## Main\n\
         Let b be a new Box with n 1.\n\
         Push 2 to b's n.\n\
         Show 9.\n",
    );
}

#[test]
fn outcome_parity_collection_stmt_shape_errors() {
    // Each collection statement's non-identifier error string is the spec.
    assert_outcome_parity(
        "## Definition\nA Box has:\n\x20   a m, which is a Map of Text to Int.\n\n## Main\nLet mm be a new Map of Text to Int.\nLet b be a new Box with m mm.\nSet item \"k\" of b's m to 5.\nShow 1.\n",
    );
    assert_outcome_parity(
        "## Definition\nA Box has:\n\x20   a s, which is a Set of Int.\n\n## Main\nLet ss be a new Set of Int.\nLet b be a new Box with s ss.\nAdd 5 to b's s.\nShow 1.\n",
    );
}

#[test]
fn outcome_parity_struct_field_set_via_index_syntax() {
    // `Set item "field" of structVar to v` — the index-syntax struct-field
    // write (the decompiler's CMapSet rendering), with VALUE semantics.
    assert_outcome_parity(
        "## Definition\n\
         A Box has:\n\
         \x20   a x, which is an Int.\n\
         \n\
         ## Main\n\
         Let b be a new Box with x 1.\n\
         Set item \"x\" of b to 9.\n\
         Show item \"x\" of b.\n\
         Show b's x.\n",
    );
    // Through a global mutated inside a function (write-back required).
    assert_outcome_parity(
        "## Definition\n\
         A Box has:\n\
         \x20   a x, which is an Int.\n\
         \n\
         ## To poke:\n\
         \x20   Set item \"x\" of b to 7.\n\
         \n\
         ## Main\n\
         Let b be a new Box with x 1.\n\
         poke().\n\
         Show b's x.\n",
    );
}
