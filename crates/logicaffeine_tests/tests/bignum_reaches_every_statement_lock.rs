//! LOCK — which integers the Rust tier stores as a `LogosInt` is decided over EVERY statement a
//! program can write, and a value the analysis promotes arrives promoted wherever it goes.
//!
//! The tree-walker and the VM hold one integer type; the Rust tier holds two (`i64` and `LogosInt`)
//! and `bigint_promote` decides, per slot, which one. Each question it asks — where is a record built,
//! where does a function return, where is a name assigned, where is a bignum handed to a call — is
//! answered by one exhaustive walk of a scope (`ast_depth::for_each_node_in_scope`), never by a walk
//! that names the statement kinds it knows and waves the rest through: a bignum built in a `Show`, a
//! `Push` or an interpolation is as real as one built in a `Let`, and each of those positions was once
//! a program the Rust tier refused to build while every other tier answered it.

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::compile::tw_outcome;

/// 2 ** 70, and twice it.
const BIG: &str = "1180591620717411303424";
const TWICE_BIG: &str = "2361183241434822606848";

/// `src` on the tree-walker and as compiled Rust, both answering `expected`; answers the Rust the
/// tier compiled, so a row can read which type a slot was given.
fn both(src: &str, expected: &[&str]) -> String {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker runs it:\n{src}");
    let lines: Vec<&str> = tw.output.lines().collect();
    assert_eq!(lines, expected, "the tree-walker's answer:\n{src}");
    let compiled = common::run_logos(src);
    assert!(
        compiled.success,
        "the Rust tier runs it:\n{src}\n\nGenerated Rust:\n{}\n\nstderr: {}",
        compiled.rust_code, compiled.stderr
    );
    let lines: Vec<&str> = compiled.stdout.trim().lines().map(str::trim).collect();
    assert_eq!(
        lines, expected,
        "the Rust tier's answer:\n{src}\n\nGenerated Rust:\n{}",
        compiled.rust_code
    );
    compiled.rust_code
}

/// The signature line the Rust tier gave function `name`.
fn signature_of<'r>(rust: &'r str, name: &str) -> &'r str {
    let head = format!("fn {name}(");
    rust.lines()
        .find(|line| line.trim_start().starts_with(&head))
        .unwrap_or_else(|| panic!("the Rust tier declares `{head}`:\n{rust}"))
}

#[test]
fn a_bignum_built_in_a_show_is_stored_exact() {
    both(
        "## Main\nShow jsonSerialize(a new JNum with n (2 ** 70)).\n",
        &[BIG],
    );
}

#[test]
fn a_bignum_built_in_a_push_is_stored_exact() {
    both(
        "## Main\nLet mutable xs be a new Seq of Json.\nPush a new JNum with n (2 ** 70) to xs.\n\
         Show jsonSerialize(item 1 of xs).\n",
        &[BIG],
    );
}

#[test]
fn a_bignum_built_in_an_interpolation_is_stored_exact() {
    both(
        "## Main\nShow \"[{jsonSerialize(a new JNum with n (2 ** 70))}]\".\n",
        &[&format!("[{BIG}]")],
    );
}

#[test]
fn a_bignum_built_into_a_binding_is_not_narrowed() {
    both(
        "## Main\nShow jsonSerialize(jsonParse(\"42\")).\nLet big be a new JNum with n (2 ** 70).\n\
         Show jsonSerialize(big).\n",
        &["42", BIG],
    );
}

#[test]
fn a_bignum_built_as_an_inspected_value_is_not_narrowed() {
    both(
        "## Main\nShow jsonSerialize(jsonParse(\"42\")).\nInspect a new JNum with n (2 ** 70):\n\
         \x20   When JNum (v):\n        Show v.\n    Otherwise:\n        Show \"none\".\n",
        &["42", BIG],
    );
}

/// A field stored as a bignum holds whatever the program builds it with: a machine word read out of
/// a binding or written as a literal is widened on the way in, for a record and a variant alike.
#[test]
fn a_machine_word_built_into_a_bignum_record_field_is_widened() {
    both(
        "## A Cell has:\n    a v, which is Int.\n\n\
         ## Main\nLet big be a new Cell with v (2 ** 70).\nLet k be 5.\nLet small be a new Cell with v k.\n\
         Let lit be a new Cell with v 7.\nShow big's v.\nShow small's v.\nShow lit's v.\n",
        &[BIG, "5", "7"],
    );
}

#[test]
fn a_machine_word_built_into_a_bignum_variant_field_is_widened() {
    both(
        "## Main\nShow jsonSerialize(a new JNum with n (2 ** 70)).\nLet k be 5.\n\
         Show jsonSerialize(a new JNum with n k).\nShow jsonSerialize(a new JNum with n 7).\n",
        &[BIG, "5", "7"],
    );
}

#[test]
fn a_bignum_handed_to_a_call_in_a_show_reaches_the_callee() {
    both(
        "## To tag (v: Int) and (k: Int) -> Text:\n    Return \"{v}/{k}\".\n\n\
         ## Main\nShow jsonSerialize(jsonParse(\"42\")).\nInspect a new JNum with n (2 ** 70):\n\
         \x20   When JNum (v):\n        Show tag(v, 2).\n    Otherwise:\n        Show \"none\".\n",
        &["42", &format!("{BIG}/2")],
    );
}

#[test]
fn a_bignum_handed_to_a_call_in_an_interpolation_reaches_the_callee() {
    both(
        "## To tag (v: Int) -> Text:\n    Return \"<{v}>\".\n\n\
         ## Main\nShow jsonSerialize(jsonParse(\"42\")).\nInspect a new JNum with n (2 ** 70):\n\
         \x20   When JNum (v):\n        Show \"got {tag(v)}\".\n    Otherwise:\n        Show \"none\".\n",
        &["42", &format!("got <{BIG}>")],
    );
}

/// `tag` is handed a bignum at `v`, so every integer parameter it takes is exact; `j` starts as a
/// machine word read out of a list and is then set from `k`.
#[test]
fn a_promoted_parameter_set_into_a_local_promotes_the_local() {
    both(
        "## To tag (v: Int) and (k: Int) -> Int:\n    Let mutable j be item k of [5, 6, 7].\n    If v > 0:\n\
         \x20       Set j to k.\n    Return j.\n\n\
         ## Main\nInspect jsonParse(\"42\"):\n    When JNum (v):\n        Let t be tag(v, 2).\n        Show t.\n\
         \x20   Otherwise:\n        Show \"none\".\n",
        &["2"],
    );
}

/// A value that crosses two calls and comes back out of an `Inspect` arm is still the bignum it was.
#[test]
fn a_function_answering_a_bignum_from_an_arm_answers_it_exact() {
    both(
        "## To twice (x: Int) -> Int:\n    Return x * 2.\n\n\
         ## To tag (v: Int) -> Int:\n    Return twice(v).\n\n\
         ## To doubled (j: Json) -> Int:\n    Inspect j:\n        When JNum (v):\n            Let t be tag(v).\n\
         \x20           Return t.\n        Otherwise:\n            Return 0.\n\n\
         ## Main\nShow doubled(a new JNum with n (2 ** 70)).\n",
        &[TWICE_BIG],
    );
}

/// A `Return` inside a closure answers the closure, not the function it is written in: a closure
/// block that returns one of the function's bignum locals does not make the function
/// bignum-returning — its own answer is the machine word it declares.
#[test]
fn a_closure_return_is_not_the_functions_return() {
    let rust = both(
        "## To count (xs: Seq of Int) -> Int:\n    Let big be 2 ** 70.\n    Let grow be (n: Int) ->:\n\
         \x20       Return big.\n    Show grow(2).\n    Return length of xs.\n\n\
         ## Main\nShow count([1, 2, 3]).\n",
        &[BIG, "3"],
    );
    let count = signature_of(&rust, "count");
    assert!(
        count.contains("-> i64"),
        "`count` answers a machine word: {count}"
    );
    assert!(
        !count.contains("LogosInt"),
        "`count` is not bignum-returning: {count}"
    );
}

/// The walks are the ones the lock's name promises: `bigint_promote.rs` walks statements only through
/// `ast_depth`'s scope walk. A `match` over statements with a wildcard arm, or a hand descent into an
/// `If`'s blocks, is a walk that knows some statement kinds and silently skips the rest.
#[test]
fn bigint_promote_walks_every_statement() {
    let source = include_str!("../../logicaffeine_compile/src/codegen/bigint_promote.rs");
    let code: String = source
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        code.contains("for_each_node_in_scope"),
        "bigint_promote walks through the scope walk"
    );
    for descent in ["then_block", "else_block", "arm.body"] {
        assert!(
            !code.contains(descent),
            "bigint_promote.rs descends into `{descent}` by hand"
        );
    }
    for (at, block) in match_blocks(&code) {
        if block.contains("Stmt::") && block.contains("_ =>") {
            panic!(
                "bigint_promote.rs matches statements with a wildcard arm at byte {at}:\n{block}"
            );
        }
    }
}

/// Every `match … { … }` in `code`, brace-balanced, with where it starts.
fn match_blocks(code: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(at) = code[from..].find("match ").map(|i| from + i) {
        from = at + 6;
        let Some(open) = code[at..].find('{').map(|i| at + i) else {
            break;
        };
        let mut depth = 0usize;
        let mut end = open;
        for (i, c) in code[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        out.push((at, code[at..end].to_string()));
    }
    out
}
