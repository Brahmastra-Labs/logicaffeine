//! ★ LOCK — a variable a `Let` copies (`Let twin be row.`) is MOVED at its last use, and copied otherwise. ★
//!
//! A `Let` of a variable binds a value of its own: the variable stays the program's to read after the binding, so a
//! binding of a variable read again copies it. A binding of a variable the program never reads again takes its value
//! (a move, free). The Rust backend copied every list, map and text a `Let` named. A copied list shares its buffer with
//! the variable, which lives to the end of its scope, so the first write into the copy copied the whole list: a
//! function that binds its list parameter to a local it then writes copied the list on every call.
//!
//! The decision is the one a call argument's and a store's are (`call_argument_move_lock`, `store_move_lock`): the
//! statement names the variable only there and either reassigns it or is the last statement that uses it. A
//! statement that runs more than once (a loop's) moves nothing. The rows build and run the Rust, and read it, so a fix
//! that copied everything would fail them.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test let_move_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::compile_to_rust;

const ARGS: &str = "## To native args () -> Seq of Text\n\n";

/// `src` run on AOT Rust with the argument `hi` prints `expected`, as the tree-walker does; the Rust it built from.
fn run(src: &str, expected: &str) -> String {
    let tw = logicaffeine_compile::compile::tw_outcome_with_args(src, &["prog".to_string(), "hi".to_string()]);
    assert_eq!(tw.error, None, "tree-walker:\n{src}");
    assert_eq!(tw.output.trim_end(), expected, "the reference tier must agree with the test:\n{src}");
    let r = common::run_logos_with_args(src, &["hi"]);
    assert!(r.success, "the generated Rust must build and run:\n{}\n--- EMITTED ---\n{}", r.stderr, r.rust_code);
    assert_eq!(r.stdout.trim_end(), expected, "AOT Rust:\n{}", r.rust_code);
    compile_to_rust(src).expect("Rust for the program")
}

/// The Rust of the function `name`.
fn function<'c>(rust: &'c str, name: &str) -> &'c str {
    let start = rust.find(&format!("fn {name}(")).unwrap_or_else(|| panic!("no `fn {name}` in:\n{rust}"));
    let end = rust[start..].find("\n}\n").map_or(rust.len(), |e| start + e + 3);
    &rust[start..end]
}

const ROW: &str = "Let s be item 2 of args().\nLet mutable row be a new Seq of Text.\nPush s to row.\n";

/// A list a `Let` names at its last use is moved into the binding, and the binding's writes are its own.
#[test]
fn a_list_bound_at_its_last_use_is_moved() {
    let src = format!("{ARGS}## Main\n{ROW}Let mutable twin be row.\nPush \"x\" to twin.\nShow twin.\n");
    let main = function(&run(&src, "[hi, x]"), "_logos_main").to_string();
    assert!(main.contains("= row;"), "the binding does not take `row`:\n{main}");
    assert!(!main.contains("row.clone()"), "`row` is copied at its last use:\n{main}");
}

/// A list read after the `Let` is copied into it, and keeps its value when the copy is written.
#[test]
fn a_list_read_after_the_binding_is_copied() {
    let src = format!("{ARGS}## Main\n{ROW}Let mutable twin be row.\nPush \"x\" to twin.\nShow twin.\nShow row.\n");
    let main = function(&run(&src, "[hi, x]\n[hi]"), "_logos_main").to_string();
    assert!(main.contains("row.clone()"), "`row` is moved before its last use:\n{main}");
}

/// A list bound in a loop is copied every time: the statement runs again.
#[test]
fn a_list_bound_in_a_loop_is_copied_each_time() {
    let src = format!(
        "{ARGS}## Main\n{ROW}Repeat for i from 1 to 2:\n    Let mutable twin be row.\n    Push \"x\" to twin.\n    Show twin.\n"
    );
    let main = function(&run(&src, "[hi, x]\n[hi, x]"), "_logos_main").to_string();
    assert!(main.contains("row.clone()"), "`row` is moved inside a loop:\n{main}");
}

/// A Text a `Let` names at its last use is moved into the binding.
#[test]
fn a_text_bound_at_its_last_use_is_moved() {
    let src = format!("{ARGS}## Main\nLet s be item 2 of args().\nLet t be s.\nShow t.\n");
    let main = function(&run(&src, "hi"), "_logos_main").to_string();
    assert!(!main.contains("s.clone()"), "`s` is copied at its last use:\n{main}");
}

/// A function that binds its list parameter to a local and writes the local takes the parameter's value at its last
/// use; the caller's list keeps the value it had.
#[test]
fn a_parameter_bound_at_its_last_use_is_moved_and_the_callers_list_is_its_own() {
    let src = format!(
        "{ARGS}## To bumped (xs: Seq of Text) -> Seq of Text:\n    Let mutable out be xs.\n    Set item 1 of out to \"x\".\n    \
         Return out.\n\n## Main\n{ROW}Let ys be bumped(row).\nShow ys.\nShow row.\n"
    );
    let rust = run(&src, "[x]\n[hi]");
    let bumped = function(&rust, "bumped");
    assert!(!bumped.contains("xs.clone()"), "the parameter is copied at its last use:\n{bumped}");
}
