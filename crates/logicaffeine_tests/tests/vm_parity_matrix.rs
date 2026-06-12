//! The parity matrix: every binary operator crossed with a panel of operand
//! values of every scalar type, plus collection-op edges — each cell runs
//! through BOTH engines and the outcome (output AND error text) must match.
//! Cells that error are as load-bearing as cells that succeed.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_parity(src: &str) {
    let tw = tw_outcome(src);
    let vm = vm_outcome(src);
    assert_eq!(
        norm(&vm.output),
        norm(&tw.output),
        "output diverged for:\n{src}\nvm: {vm:?}\ntw: {tw:?}"
    );
    assert_eq!(vm.error, tw.error, "error diverged for:\n{src}");
}

/// Literal panel per type, as source fragments.
const INTS: &[&str] = &["0", "1", "7", "9223372036854775807"];
const FLOATS: &[&str] = &["0.0", "0.5", "2.5"];
const BOOLS: &[&str] = &["true", "false"];
const TEXTS: &[&str] = &["\"\"", "\"hi\""];

#[test]
fn parity_arith_ops_across_type_pairs() {
    // +, -, *, /, % over every pair drawn from Int/Float/Bool/Text panels.
    let mut operands: Vec<&str> = Vec::new();
    operands.extend_from_slice(INTS);
    operands.extend_from_slice(FLOATS);
    operands.extend_from_slice(BOOLS);
    operands.extend_from_slice(TEXTS);

    for op in ["+", "-", "*", "/", "%"] {
        for a in &operands {
            for b in &operands {
                assert_parity(&format!("## Main\nLet r be {a} {op} {b}.\nShow r.\n"));
            }
        }
    }
}

#[test]
fn parity_comparison_ops_across_type_pairs() {
    let mut operands: Vec<&str> = Vec::new();
    operands.extend_from_slice(INTS);
    operands.extend_from_slice(FLOATS);
    operands.extend_from_slice(BOOLS);

    for op in ["is less than", "is greater than", "is at most", "is at least", "equals"] {
        for a in &operands {
            for b in &operands {
                assert_parity(&format!(
                    "## Main\nIf {a} {op} {b}:\n    Show 1.\nOtherwise:\n    Show 0.\n"
                ));
            }
        }
    }
}

#[test]
fn parity_wrapping_edges() {
    for src in [
        "## Main\nLet mutable x be 9223372036854775807.\nSet x to x + 1.\nShow x.\n",
        "## Main\nLet mutable x be 9223372036854775807.\nSet x to x * 3.\nShow x.\n",
        "## Main\nLet mutable x be 9223372036854775807.\nSet x to x + 1.\nSet x to x - 1.\nShow x.\n",
        "## Main\nLet y be 0 - 9223372036854775807.\nShow y - 2.\n",
    ] {
        assert_parity(src);
    }
}

#[test]
fn parity_index_edges_across_collections() {
    // List/Text indexing at {0, 1, len, len+1, negative}.
    for idx in ["0", "1", "3", "4", "0 - 1"] {
        assert_parity(&format!(
            "## Main\nLet xs be [10, 20, 30].\nLet i be {idx}.\nShow item i of xs.\n"
        ));
        assert_parity(&format!(
            "## Main\nLet s be \"abc\".\nLet i be {idx}.\nShow item i of s.\n"
        ));
    }
}

#[test]
fn parity_aliasing_vs_copy_matrix() {
    // Lists alias through Let; copies do not; struct assignment copies.
    assert_parity(
        "## Main\n\
         Let mutable xs be [1].\n\
         Let mutable ys be xs.\n\
         Push 2 to ys.\n\
         Show length of xs.\n",
    );
    assert_parity(
        "## Main\n\
         Let mutable xs be [1].\n\
         Let ys be copy(xs).\n\
         Push 2 to xs.\n\
         Show length of ys.\n",
    );
}

#[test]
fn parity_shadowing_matrix() {
    // Block shadowing: the inner Let dies with the block in both engines.
    assert_parity(
        "## Main\n\
         Let mutable x be 1.\n\
         If true:\n\
         \x20   Let x be 99.\n\
         \x20   Show x.\n\
         Show x.\n",
    );
    // Mutation (Set) inside the block persists.
    assert_parity(
        "## Main\n\
         Let mutable x be 1.\n\
         If true:\n\
         \x20   Set x to 99.\n\
         Show x.\n",
    );
}

#[test]
fn parity_text_unicode_quirks() {
    // length of Text is BYTES; indexing is CHARS — both pinned.
    assert_parity("## Main\nLet s be \"héllo\".\nShow length of s.\nShow item 2 of s.\n");
}

#[test]
fn parity_division_error_text_and_partial_output() {
    for src in [
        "## Main\nShow 1.\nShow 1 / 0.\n",
        "## Main\nShow 1.\nShow 1 % 0.\n",
        "## Main\nShow 2.5 / 0.0.\n",
        "## Main\nShow 1 / 0.0.\n",
    ] {
        assert_parity(src);
    }
}

#[test]
fn parity_builtin_matrix() {
    for call in [
        "abs(0 - 7)",
        "sqrt(9)",
        "min(3, 2)",
        "max(3, 2)",
        "floor(2.9)",
        "ceil(2.1)",
        "round(2.5)",
        "pow(2, 10)",
        "chr(65)",
        "length(\"hello\")",
        "parseInt(\" 42 \")",
        "parseFloat(\"2.5\")",
    ] {
        assert_parity(&format!("## Main\nShow {call}.\n"));
    }
    // Error cells.
    for call in ["chr(0 - 1)", "parseInt(\"zz\")", "sqrt(true)", "abs(1, 2)"] {
        assert_parity(&format!("## Main\nShow {call}.\n"));
    }
}
