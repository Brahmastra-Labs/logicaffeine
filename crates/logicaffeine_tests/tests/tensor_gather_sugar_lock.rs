//! The data-indexed tensor sugars — the embedding lookup, the arg-max, the KV-cache append — as LANGUAGE:
//!
//!   `Let x be row tok of Emb.`   a `1 × cols` tensor holding row `tok` (1-based, like `item`) of `Emb`;
//!   `Let t be argmax of X.`      the 1-based index of the FIRST maximal element of `X`, an Int;
//!   `Set row p of Kc to kp.`     row `p` of `Kc` overwritten by `kp` (whose length is `Kc`'s row width).
//!
//! Like every tensor sugar they desugar to ordinary statements, so every tier runs them; these tests pin
//! the meaning on the tree-walker (the reference) and the VM (every other tier's oracle), and the backend
//! matrix (`tensor_op_coverage_lock`) runs the probes on the rest.
//!
//! ★ A ROW INDEX IS CHECKED, NOT TRUSTED. The flat offset of row `r` is `(r − 1)·cols`, and a LOGOS index
//! below 1 is END-RELATIVE (`item -1 of xs` is the last element) — so `row 0 of Emb` on a two-column table
//! would read the table's last elements rather than fail. Every row sugar checks `1 ≤ r ≤ rows` first and
//! fails with one message on every tier.
//!
//! ★ THE WORDS STAY FREE. `row` and `argmax` open a sugar only when the whole shape is there — `row <index>
//! of <a declared tensor>`, `argmax of <a declared tensor>`; otherwise they are ordinary names.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test tensor_gather_sugar_lock`

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Run on the tree-walker and the VM; they must agree exactly; return (output lines, error).
fn run(src: &str) -> (Vec<String>, Option<String>) {
    let (tw, vm) = (tw_outcome(src), vm_outcome(src));
    let lines = |s: &str| {
        s.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        (lines(&tw.output), &tw.error),
        (lines(&vm.output), &vm.error),
        "the VM and the tree-walker disagree on:\n{src}"
    );
    (lines(&tw.output), tw.error)
}

fn ok(src: &str) -> Vec<String> {
    let (out, err) = run(src);
    assert_eq!(err, None, "unexpected error on:\n{src}");
    out
}

fn fails(src: &str) -> String {
    let (_, err) = run(src);
    err.unwrap_or_else(|| panic!("expected a runtime error on:\n{src}"))
}

fn parse_error(src: &str) -> String {
    let out = tw_outcome(src);
    out.error
        .unwrap_or_else(|| panic!("expected a parse error on:\n{src}"))
}

const EMB: &str = "## Main\n\
Let Emb be a 3 by 2 tensor of Real.\n\
Set item 1 of Emb to 1.0.\nSet item 2 of Emb to 2.0.\nSet item 3 of Emb to 3.0.\n\
Set item 4 of Emb to 4.0.\nSet item 5 of Emb to 5.0.\nSet item 6 of Emb to 6.0.\n";

// ── row … of ─────────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_row_of_a_real_table_is_that_row() {
    for (tok, want) in [(1, ["1", "2"]), (2, ["3", "4"]), (3, ["5", "6"])] {
        let src = format!("{EMB}Let tok be {tok}.\nLet x be row tok of Emb.\nShow round(item 1 of x).\nShow round(item 2 of x).\nShow length of x.\n");
        assert_eq!(ok(&src), [want[0], want[1], "2"], "row {tok}");
    }
}

#[test]
fn the_row_index_may_be_a_literal_a_name_or_an_expression() {
    let lit = ok(&format!(
        "{EMB}Let x be row 3 of Emb.\nShow round(item 2 of x).\n"
    ));
    let expr = ok(&format!(
        "{EMB}Let t be 1.\nLet x be row (t + 2) of Emb.\nShow round(item 2 of x).\n"
    ));
    assert_eq!(lit, ["6"]);
    assert_eq!(expr, ["6"]);
}

#[test]
fn a_row_of_an_int_table_is_int() {
    let src = "## Main\n\
Let T be a 2 by 3 tensor of Int.\n\
Set item 4 of T to 7.\nSet item 5 of T to 8.\nSet item 6 of T to 9.\n\
Let x be row 2 of T.\n\
Show item 1 of x + item 3 of x.\n\
Show item 2 of x.\n";
    // Integer arithmetic and an integer rendering — a Real row would show `16.0` / `8.0`.
    assert_eq!(ok(src), ["16", "8"]);
}

#[test]
fn the_row_is_a_copy_not_a_view() {
    let src = format!("{EMB}Let mutable x be row 2 of Emb.\nSet item 1 of x to 99.0.\nShow round(item 3 of Emb).\nShow round(item 1 of x).\n");
    assert_eq!(ok(&src), ["3", "99"]);
}

#[test]
fn a_row_out_of_range_fails_with_one_message_on_every_tier() {
    for tok in ["0", "4", "-1"] {
        let err = fails(&format!(
            "{EMB}Let tok be {tok}.\nLet x be row tok of Emb.\nShow item 1 of x.\n"
        ));
        assert!(
            err.contains("row index out of range") && err.contains("Emb has 3 rows"),
            "row {tok}: {err}"
        );
    }
}

#[test]
fn the_row_index_is_evaluated_once() {
    let src = format!("## To next (c: Int) -> Int:\n    Show c.\n    Return c.\n\n{EMB}Let x be row (next(2)) of Emb.\nShow round(item 1 of x).\n");
    assert_eq!(
        ok(&src),
        ["2", "3"],
        "the index expression runs exactly once"
    );
}

// ── argmax of ────────────────────────────────────────────────────────────────────────────────────────

fn argmax_of(values: &[&str], kind: &str) -> Vec<String> {
    let mut src = format!(
        "## Main\nLet X be a 1 by {} tensor of {kind}.\n",
        values.len()
    );
    for (i, v) in values.iter().enumerate() {
        src += &format!("Set item {} of X to {v}.\n", i + 1);
    }
    src += "Let t be argmax of X.\nShow t.\nShow t + 1.\n";
    ok(&src)
}

#[test]
fn argmax_is_the_one_based_index_of_the_first_maximum() {
    assert_eq!(
        argmax_of(&["0.5", "2.5", "2.5", "1.0"], "Real"),
        ["2", "3"],
        "ties go to the first"
    );
    assert_eq!(
        argmax_of(&["-3.0", "-1.0", "-2.0"], "Real"),
        ["2", "3"],
        "all negative"
    );
    assert_eq!(
        argmax_of(&["9.0", "1.0"], "Real"),
        ["1", "2"],
        "the first element"
    );
    assert_eq!(
        argmax_of(&["1.0", "9.0"], "Real"),
        ["2", "3"],
        "the last element"
    );
    assert_eq!(argmax_of(&["4.0"], "Real"), ["1", "2"], "a single element");
    assert_eq!(
        argmax_of(&["3", "7", "7", "-1"], "Int"),
        ["2", "3"],
        "an Int tensor"
    );
}

#[test]
fn argmax_ranges_over_every_element_of_a_matrix() {
    let src = "## Main\n\
Let M be a 2 by 3 tensor of Int.\n\
Set item 1 of M to 1.\nSet item 5 of M to 50.\nSet item 6 of M to 49.\n\
Let t be argmax of M.\nShow t.\n";
    assert_eq!(ok(src), ["5"]);
}

// ── Set row … of … to ────────────────────────────────────────────────────────────────────────────────

const KC: &str = "## Main\n\
Let Kc be a 3 by 2 tensor of Real.\n\
Let kp be a 1 by 2 tensor of Real.\nSet item 1 of kp to 7.0.\nSet item 2 of kp to 8.0.\n";

fn kc_after(p: &str) -> Vec<String> {
    let mut src = format!("{KC}Let p be {p}.\nSet row p of Kc to kp.\n");
    for i in 1..=6 {
        src += &format!("Show round(item {i} of Kc).\n");
    }
    ok(&src)
}

#[test]
fn set_row_writes_exactly_that_row() {
    assert_eq!(kc_after("1"), ["7", "8", "0", "0", "0", "0"]);
    assert_eq!(kc_after("2"), ["0", "0", "7", "8", "0", "0"]);
    assert_eq!(kc_after("3"), ["0", "0", "0", "0", "7", "8"]);
}

#[test]
fn set_row_copies_the_source() {
    let src = format!(
        "{KC}Set row 2 of Kc to kp.\nSet item 1 of kp to 100.0.\nShow round(item 3 of Kc).\n"
    );
    assert_eq!(
        ok(&src),
        ["7"],
        "a later change to the source row does not reach the table"
    );
}

#[test]
fn set_row_out_of_range_fails_with_one_message_and_writes_nothing() {
    for p in ["0", "4", "-1"] {
        let err = fails(&format!(
            "{KC}Let p be {p}.\nSet row p of Kc to kp.\nShow 1.\n"
        ));
        assert!(
            err.contains("row index out of range") && err.contains("Kc has 3 rows"),
            "row {p}: {err}"
        );
    }
}

#[test]
fn set_row_refuses_a_source_of_the_wrong_width_or_kind_at_parse_time() {
    let wide = "## Main\nLet Kc be a 3 by 2 tensor of Real.\nLet kp be a 1 by 3 tensor of Real.\nSet row 1 of Kc to kp.\n";
    let e = parse_error(wide);
    assert!(e.contains("Set row") && e.contains("2"), "{e}");
    let int = "## Main\nLet Kc be a 3 by 2 tensor of Real.\nLet kp be a 1 by 2 tensor of Int.\nSet row 1 of Kc to kp.\n";
    let e = parse_error(int);
    assert!(e.contains("Set row") && e.contains("Int"), "{e}");
}

// ── a computed tensor carries its operands' element kind ─────────────────────────────────────────────

/// Two 1×2 operands of kind `a` and `b`, combined by `op` into `v`, then written as row 2 of a 3×2 table of
/// kind `table`. Returns the table's six elements, each shown ×100 (rounded when the table is Real).
fn set_row_of_computed(op: &str, a: &str, b: &str, table: &str) -> Result<Vec<String>, String> {
    let lit = |kind: &str, x: i64| {
        if kind == "Real" {
            format!("{x}.5")
        } else {
            format!("{x}")
        }
    };
    let src = format!(
        "## Main\n\
         Let Kc be a 3 by 2 tensor of {table}.\n\
         Let A be a 1 by 2 tensor of {a}.\nSet item 1 of A to {}.\nSet item 2 of A to {}.\n\
         Let B be a {} tensor of {b}.\n{}\
         Let v be A {op} B.\n\
         Set row 2 of Kc to v.\n\
         {}",
        lit(a, 1),
        lit(a, 2),
        if op == "matmul" { "2 by 2" } else { "1 by 2" },
        if op == "matmul" {
            format!("Set item 1 of B to {}.\nSet item 2 of B to {}.\nSet item 3 of B to {}.\nSet item 4 of B to {}.\n", lit(b, 1), lit(b, 0), lit(b, 0), lit(b, 1))
        } else {
            format!(
                "Set item 1 of B to {}.\nSet item 2 of B to {}.\n",
                lit(b, 3),
                lit(b, 4)
            )
        },
        (1..=6)
            .map(|i| if table == "Real" {
                format!("Show round(item {i} of Kc * 100).\n")
            } else {
                format!("Show item {i} of Kc * 100.\n")
            })
            .collect::<String>()
    );
    let out = tw_outcome(&src);
    match out.error {
        Some(e) => Err(e),
        None => Ok(ok(&src)),
    }
}

/// ★ A MATMUL, SUM OR HADAMARD OF REAL TENSORS IS REAL. Every attention layer appends `vr = xn matmul Wv` to its
/// Real value cache (`Set row pos of Vc to vr`); a computed tensor recorded as Int would refuse that at parse
/// time even though every tier computes it in Real.
#[test]
fn a_real_tensor_computed_by_matmul_sum_or_hadamard_lands_in_a_real_table() {
    // A = [1.5, 2.5]; matmul by [[1.5, 0.5], [0.5, 1.5]] = [3.5, 4.5]; plus [3.5, 4.5] = [5, 7]; hadamard = [5.25, 11.25].
    for (op, row) in [
        ("matmul", ["350", "450"]),
        ("plus", ["500", "700"]),
        ("hadamard", ["525", "1125"]),
    ] {
        let got = set_row_of_computed(op, "Real", "Real", "Real")
            .unwrap_or_else(|e| panic!("`{op}` of Real tensors is Real: {e}"));
        assert_eq!(
            got,
            ["0", "0", row[0], row[1], "0", "0"],
            "`{op}`: row 2 holds the computed row"
        );
    }
}

/// An Int op Int stays Int, and lands in an Int table — and a Real table refuses it by name, both ways round.
#[test]
fn an_int_tensor_computed_from_int_operands_stays_int() {
    // A = [1, 2]; matmul by [[1, 0], [0, 1]] = [1, 2]; plus [3, 4] = [4, 6]; hadamard = [3, 8].
    for (op, row) in [
        ("matmul", ["100", "200"]),
        ("plus", ["400", "600"]),
        ("hadamard", ["300", "800"]),
    ] {
        let got = set_row_of_computed(op, "Int", "Int", "Int")
            .unwrap_or_else(|e| panic!("`{op}` of Int tensors is Int: {e}"));
        assert_eq!(
            got,
            ["0", "0", row[0], row[1], "0", "0"],
            "`{op}`: row 2 holds the computed row"
        );
        let e = set_row_of_computed(op, "Int", "Int", "Real")
            .expect_err("an Int row does not land in a Real table");
        assert!(
            e.contains("Set row") && e.contains("holds Real") && e.contains("holds Int"),
            "`{op}`: {e}"
        );
        let e = set_row_of_computed(op, "Real", "Real", "Int")
            .expect_err("a Real row does not land in an Int table");
        assert!(
            e.contains("Set row") && e.contains("holds Int") && e.contains("holds Real"),
            "`{op}`: {e}"
        );
    }
}

/// A Real operand makes the result Real, whichever side it is on.
#[test]
fn one_real_operand_makes_the_computed_tensor_real() {
    for op in ["matmul", "plus", "hadamard"] {
        for (a, b) in [("Real", "Int"), ("Int", "Real")] {
            assert!(
                set_row_of_computed(op, a, b, "Real").is_ok(),
                "`{a} {op} {b}` is Real and lands in a Real table"
            );
            let e = set_row_of_computed(op, a, b, "Int").expect_err("and not in an Int one");
            assert!(e.contains("holds Real"), "`{a} {op} {b}`: {e}");
        }
    }
}

/// A transpose moves elements without computing on them, so it keeps its input's kind: an Int column
/// transposed is an Int row, and it lands in an Int table and not a Real one.
#[test]
fn a_transpose_keeps_its_inputs_element_kind() {
    let src = |kind: &str, table: &str| {
        let (a, b) = if kind == "Int" {
            ("3", "4")
        } else {
            ("3.5", "4.5")
        };
        format!(
            "## Main\nLet Kc be a 2 by 2 tensor of {table}.\nLet X be a 2 by 1 tensor of {kind}.\n\
             Set item 1 of X to {a}.\nSet item 2 of X to {b}.\nLet T be transpose of X.\nSet row 2 of Kc to T.\n\
             Show item 3 of Kc * 10.\nShow item 4 of Kc * 10.\n"
        )
    };
    assert_eq!(
        ok(&src("Int", "Int")),
        ["30", "40"],
        "an Int transpose lands in an Int table"
    );
    let e = parse_error(&src("Int", "Real"));
    assert!(
        e.contains("holds Real") && e.contains("holds Int"),
        "an Int transpose is refused by a Real table: {e}"
    );
    let e = parse_error(&src("Real", "Int"));
    assert!(
        e.contains("holds Int") && e.contains("holds Real"),
        "a Real transpose is refused by an Int table: {e}"
    );
    assert_eq!(
        ok(&src("Real", "Real")).len(),
        2,
        "a Real transpose lands in a Real table"
    );
}

// ── the words stay free ──────────────────────────────────────────────────────────────────────────────

#[test]
fn row_and_argmax_are_still_ordinary_names() {
    let src = "## To argmax (n: Int) -> Int:\n    Return n * 2.\n\n\
## Main\n\
Let row be 5.\n\
Set row to row + 1.\n\
Let x be row.\n\
Show x.\n\
Show argmax(4).\n";
    assert_eq!(ok(src), ["6", "8"]);
}

#[test]
fn a_row_of_something_that_is_not_a_tensor_is_not_the_sugar() {
    // `row` names a list here and `of` would follow nothing tensor-shaped: the ordinary parse stands.
    let src = "## Main\nLet row be [1, 2, 3].\nShow item 2 of row.\n";
    assert_eq!(ok(src), ["2"]);
}
