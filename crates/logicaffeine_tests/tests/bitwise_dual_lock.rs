//! A bit operation over an Int operand the Oracle cannot keep within a machine word runs on machine words, exact at
//! the edge of the word.
//!
//! `available & (0 - available)` isolates the lowest set bit, and `0 - available` overflows only for `available` at
//! the bottom of the word, where its exact value is `2^63` and `&` refuses it beside a negative operand. The generated
//! Rust computes such an operand's chain in checked machine-word arithmetic, one overflow check per step, runs the raw
//! `&`, `|`, `xor`, `shifted left by` or `shifted right by` when every step fit, and the exact form
//! (`logos_band_exact`, `logos_word_operand`) otherwise, out of line, so its big-integer code never crowds the hot
//! function (nqueens unrolls its search eight levels deep, and eight inlined exact forms spilled its registers).
//! nqueens' search stays on machine words, and every program answers alike on every tier: the same value, or the same
//! refusal.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test bitwise_dual_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted, compile_to_rust, run_all_tiers};

fn benchmark(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../benchmarks/programs/{name}/main.lg"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// nqueens with the board size in the source instead of on the command line.
fn nqueens_at(n: i64) -> String {
    benchmark("nqueens").replace(
        "Let n be parseInt(item 2 of arguments).",
        &format!("Let n be {n}."),
    )
}

/// A program whose Ints come from text, so the Oracle knows nothing of their range, and which shows `shown`.
fn over(values: &[(&str, &str)], shown: &str) -> String {
    let mut src = String::from("## To native parseInt (s: Text) -> Int\n\n## Main\n");
    for (name, value) in values {
        src.push_str(&format!("Let {name} be parseInt(\"{value}\").\n"));
    }
    src.push_str(&format!("Show {shown}.\n"));
    src
}

/// Every tier prints the same, compiled Rust prints the interpreter's output, and C prints it too.
fn answers_alike(source: &str) -> String {
    let agreed = run_all_tiers(source);
    assert!(agreed.error.is_none(), "{:?}\n{source}", agreed.error);
    assert_c_output(source, agreed.output.trim());
    assert_compiled_equals_interpreted(source);
    agreed.output.trim().to_string()
}

/// Every tier refuses alike, and the compiled program fails as the interpreter does, in its words.
fn refuses_alike(source: &str) {
    let agreed = run_all_tiers(source);
    let want = agreed
        .error
        .unwrap_or_else(|| panic!("the reference refuses:\n{source}"));
    let compiled = common::run_logos(source);
    assert!(
        !compiled.success,
        "the compiled program must refuse as the interpreter does ({want}); it printed:\n{}\n\n{}",
        compiled.stdout, compiled.rust_code
    );
    assert!(
        compiled.stderr.contains(&want),
        "the compiled refusal {:?} is not the interpreter's {want:?}\n\n{}",
        compiled.stderr,
        compiled.rust_code
    );
}

/// The function `name`'s body in generated Rust.
fn function<'a>(rust: &'a str, name: &str) -> &'a str {
    let start = rust
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("no fn {name} in:\n{rust}"));
    let end = rust[start..]
        .find("\n}\n")
        .map_or(rust.len(), |e| start + e);
    &rust[start..end]
}

#[test]
fn the_nqueens_benchmark_isolates_its_lowest_bit_on_machine_words() {
    let rust = compile_to_rust(&benchmark("nqueens")).unwrap();
    let solve = function(&rust, "solve");
    assert!(
        solve.contains("(0i64).checked_sub(available)?") && solve.contains("=> (available & __bwrv)"),
        "the search's `available & (0 - available)` negates with one overflow check and ands on machine words:\n{solve}"
    );
    let exact: Vec<usize> = solve
        .match_indices("logos_band_exact(")
        .map(|(at, _)| at)
        .collect();
    assert!(
        !exact.is_empty(),
        "the exact form stays for the step that does not fit:\n{solve}"
    );
    for at in exact {
        assert!(
            solve[..at].ends_with("_ => logos_cold_path(move || "),
            "an exact `&` runs outside the out-of-line cold branch:\n{solve}"
        );
    }
}

/// The benchmark as shipped, its board size on the command line (a constant size specializes the search differently).
#[test]
fn the_nqueens_benchmark_as_shipped_counts_on_machine_words() {
    let shipped = common::run_logos_with_args(&benchmark("nqueens"), &["10"]);
    assert!(
        shipped.success,
        "the shipped nqueens compiles and runs:\n{}\n{}",
        shipped.stderr, shipped.rust_code
    );
    assert_eq!(shipped.stdout.trim(), "724");
    let rust = &shipped.rust_code;
    assert!(
        rust.contains(".checked_sub(")
            && rust.matches("logos_band_exact(").count()
                == rust.matches("_ => logos_cold_path(move || logos_band_exact(").count(),
        "its lowest-bit isolation runs checked on machine words, its exact form only out of line:\n{rust}"
    );
}

#[test]
fn the_nqueens_benchmark_counts_alike_at_every_size() {
    for (n, solutions) in [
        (1, "1"),
        (2, "0"),
        (3, "0"),
        (4, "2"),
        (5, "10"),
        (6, "4"),
        (7, "40"),
        (8, "92"),
        (9, "352"),
    ] {
        assert_eq!(answers_alike(&nqueens_at(n)), solutions, "nqueens({n})");
    }
}

/// Bit operations whose operand is arithmetic over unknown Ints, at values well inside a word, at its edge and past it,
/// on both sides of each operator.
fn programs() -> Vec<(String, String)> {
    let mut v = Vec::new();
    let lowest = "x & (0 - x)";
    for x in [
        "0",
        "1",
        "12",
        "96",
        "4611686018427387904",
        "4611686018427387905",
        "9223372036854775807",
        "-8",
        "-9223372036854775807",
    ] {
        v.push((format!("{lowest} at {x}"), over(&[("x", x)], lowest)));
    }
    for (shown, values) in [
        ("x | (y + 1)", &[("x", "8"), ("y", "4")][..]),
        ("x | (y + 1)", &[("x", "8"), ("y", "9223372036854775806")]),
        ("x xor (y * 3)", &[("x", "1023"), ("y", "341")]),
        (
            "x xor (y * 3)",
            &[("x", "-1"), ("y", "3074457345618258602")],
        ),
        ("(a - b) & 255", &[("a", "1000"), ("b", "1")]),
        (
            "(a - b) & 255",
            &[("a", "-9223372036854775807"), ("b", "1")],
        ),
        (
            "(a + b) & (c - d)",
            &[("a", "7"), ("b", "9"), ("c", "100"), ("d", "1")],
        ),
        (
            "(a + b) & (c - d)",
            &[
                ("a", "4611686018427387904"),
                ("b", "4611686018427387903"),
                ("c", "-1"),
                ("d", "0"),
            ],
        ),
        ("(x + y) shifted right by 1", &[("x", "40"), ("y", "2")]),
        ("1 shifted left by (k + 1)", &[("k", "9")]),
        ("(k * 2) shifted left by 3", &[("k", "21")]),
        ("x xor (y * -3)", &[("x", "5"), ("y", "7")]),
        ("(x - 8) & (y + 8)", &[("x", "100"), ("y", "-20")]),
        (
            "x & ((y * 4 + 8) / 2)",
            &[("x", "255"), ("y", "4611686018427387904")],
        ),
        (
            "x & ((1 shifted left by (k / 2)) - 1)",
            &[("x", "1023"), ("k", "11")],
        ),
        ("x xor (-3 * y)", &[("x", "5"), ("y", "7")]),
        ("(-8 - y) & x", &[("x", "255"), ("y", "100")]),
    ] {
        v.push((format!("{shown} at {values:?}"), over(values, shown)));
    }
    v
}

#[test]
fn every_bit_operation_over_a_chain_answers_alike() {
    for (what, source) in programs() {
        let shown = answers_alike(&source);
        assert!(!shown.is_empty(), "{what}");
    }
}

/// Past the word the exact form decides: a big operand beside a low mask keeps its low bits, and any other big
/// operand is refused in the operator's words, compiled as interpreted.
#[test]
fn past_the_word_the_exact_form_answers_or_refuses_as_the_reference_does() {
    for (shown, values, refused) in [
        ("x & (0 - x)", &[("x", "-9223372036854775808")][..], true),
        (
            "x | (y + 1)",
            &[("x", "8"), ("y", "9223372036854775807")],
            true,
        ),
        (
            "x xor (y * 3)",
            &[("x", "1"), ("y", "3074457345618258603")],
            true,
        ),
        ("(x * 4) & 255", &[("x", "4611686018427387905")], false),
        ("(x * 4) & 256", &[("x", "4611686018427387905")], true),
        (
            "x & (y * 4)",
            &[("x", "1023"), ("y", "4611686018427387905")],
            false,
        ),
    ] {
        let source = over(values, shown);
        if refused {
            refuses_alike(&source);
        } else {
            let agreed = run_all_tiers(&source);
            assert!(agreed.error.is_none(), "{:?}\n{source}", agreed.error);
            assert_compiled_equals_interpreted(&source);
        }
    }
}

#[test]
fn a_call_in_an_operand_runs_once_and_in_order() {
    let source = "## To native parseInt (s: Text) -> Int\n\n\
                  ## To next (x: Int) -> Int:\n    Show x.\n    Return x + 1.\n\n\
                  ## Main\n\
                  Let x be parseInt(\"20\").\n\
                  Show next(x) & (0 - next(x + 10)).\n";
    let shown = answers_alike(source);
    assert_eq!(
        shown, "20\n30\n1",
        "the left call runs first, each call once"
    );
}

/// An exact chain's call takes its argument as the parameter declares it: an `Int` parameter is a machine word, so
/// the argument's own arithmetic narrows at the call rather than reaching the callee as an exact integer (which
/// generated Rust that did not compile), while the chain around the call stays exact past the word.
#[test]
fn a_call_inside_an_exact_chain_takes_its_argument_as_its_parameter_declares() {
    let source = "## To native parseInt (s: Text) -> Int\n\n\
                  ## To next (x: Int) -> Int:\n    Show x.\n    Return x + 1.\n\n\
                  ## Main\n\
                  Let x be parseInt(\"5\").\n\
                  Show next(x + 1) + 9223372036854775807.\n\
                  Show (next(x * 2) - 1) & 255.\n";
    let shown = answers_alike(source);
    assert_eq!(shown, "6\n9223372036854775814\n10\n10");
}

/// Every exact branch a guard or a checked step sends a computation to runs out of line in a closure that takes its
/// machine words by value. A closure borrowing a loop counter into the out-of-line call pins the counter to memory for
/// the whole loop: matrix_mult ran five times slower until its index chain's closure moved its words.
#[test]
fn every_cold_path_takes_its_words_by_value() {
    let programs =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/programs");
    let mut cold = 0;
    let mut borrowing = Vec::new();
    for entry in std::fs::read_dir(&programs).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(source) = std::fs::read_to_string(entry.path().join("main.lg")) else {
            continue;
        };
        let rust = compile_to_rust(&source).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        for (at, _) in rust.match_indices("logos_cold_path(") {
            cold += 1;
            if !rust[at..].starts_with("logos_cold_path(move || ") {
                borrowing.push(format!("{name}: {}", &rust[at..rust.len().min(at + 80)]));
            }
        }
    }
    assert!(cold > 0, "the benchmarks take exact branches out of line");
    assert!(
        borrowing.is_empty(),
        "a cold path borrows its words:\n  {}",
        borrowing.join("\n  ")
    );
}
