//! ★ LOCK — a variable stored into a place (pushed onto a list, stored as an item or a map's value, set as a field) is
//! MOVED at its last use, and copied otherwise. ★
//!
//! A store keeps a value of its own: the variable stored stays the program's to read after the store, so a store of a
//! variable read again copies it. A store of a variable the program never reads again hands the value over (a move,
//! free). The Rust backend copied every variable it stored. For a list the copy shares its buffer with the variable,
//! and the variable lives to the end of its scope, so the first write into the stored list copied the whole list.
//!
//! The decision is the one a call argument's is (`call_argument_move_lock`): the statement names the variable only
//! there and either reassigns it or is the last statement that uses it. A statement that runs more than once (a
//! loop's) moves nothing. The rows build and run the Rust, and read it, so a fix that copied everything would fail them.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test store_move_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::compile_to_rust;

const ARGS: &str = "## To native args () -> Seq of Text\n\n";

/// `src` run on AOT Rust with the argument `hi` prints `expected`, as the tree-walker does; the Rust of its `Main`.
fn run(src: &str, expected: &str) -> String {
    let tw = logicaffeine_compile::compile::tw_outcome_with_args(src, &["prog".to_string(), "hi".to_string()]);
    assert_eq!(tw.error, None, "tree-walker:\n{src}");
    assert_eq!(tw.output.trim_end(), expected, "the reference tier must agree with the test:\n{src}");
    let r = common::run_logos_with_args(src, &["hi"]);
    assert!(r.success, "the generated Rust must build and run:\n{}\n--- EMITTED ---\n{}", r.stderr, r.rust_code);
    assert_eq!(r.stdout.trim_end(), expected, "AOT Rust:\n{}", r.rust_code);
    let rust = compile_to_rust(src).expect("Rust for the program");
    let main = rust.find("fn _logos_main(").expect("a main");
    rust[main..].to_string()
}

/// A Text pushed at its last use is moved onto the list.
#[test]
fn a_text_pushed_at_its_last_use_is_moved() {
    let src = format!("{ARGS}## Main\nLet s be item 2 of args().\nLet mutable words be a new Seq of Text.\nPush s to words.\nShow words.\n");
    let main = run(&src, "[hi]");
    assert!(main.contains("words.push(s);"), "the push does not take `s`:\n{main}");
}

/// A Text read after the push is copied onto the list, and keeps its value.
#[test]
fn a_text_read_after_the_push_is_copied() {
    let src = format!(
        "{ARGS}## Main\nLet s be item 2 of args().\nLet mutable words be a new Seq of Text.\nPush s to words.\nShow words.\nShow s.\n"
    );
    let main = run(&src, "[hi]\nhi");
    assert!(main.contains("words.push(s.clone());"), "`s` is moved before its last use:\n{main}");
}

/// A Text pushed in a loop is copied every time: the statement runs again.
#[test]
fn a_text_pushed_in_a_loop_is_copied_each_time() {
    let src = format!(
        "{ARGS}## Main\nLet s be item 2 of args().\nLet mutable words be a new Seq of Text.\nRepeat for i from 1 to 2:\n    Push s to words.\nShow words.\n"
    );
    let main = run(&src, "[hi, hi]");
    assert!(main.contains("words.push(s.clone());"), "`s` is moved inside a loop:\n{main}");
}

/// A list pushed onto a list of lists at its last use is moved there, and a store into the row it became changes that
/// row alone.
#[test]
fn a_list_pushed_at_its_last_use_is_moved_and_its_row_is_the_grids() {
    let src = format!(
        "{ARGS}## Main\nLet s be item 2 of args().\nLet mutable row be a new Seq of Text.\nPush s to row.\n\
         Let mutable grid be a new Seq of Seq of Text.\nPush row to grid.\nSet item 1 of (item 1 of grid) to \"x\".\nShow grid.\n"
    );
    let main = run(&src, "[[x]]");
    assert!(main.contains("grid.push(row);"), "the push does not take `row`:\n{main}");
}

/// A Text stored as an item of a list at its last use is moved into it.
#[test]
fn a_text_stored_as_an_item_at_its_last_use_is_moved() {
    let src = format!(
        "{ARGS}## Main\nLet s be item 2 of args().\nLet mutable words be [\"a\", \"b\"].\nSet item 2 of words to s.\nShow words.\n"
    );
    let main = run(&src, "[a, hi]");
    assert!(!main.contains("s.clone()"), "`s` is copied at its last use:\n{main}");
}

/// A Text stored as a map's value at its last use is moved into the map; one read after is copied.
#[test]
fn a_text_stored_in_a_map_moves_at_its_last_use_only() {
    let last = format!(
        "{ARGS}## Main\nLet s be item 2 of args().\nLet mutable m be a new Map of Text to Text.\nSet item \"k\" of m to s.\nShow item \"k\" of m.\n"
    );
    let main = run(&last, "hi");
    assert!(!main.contains("s.clone()"), "`s` is copied at its last use:\n{main}");
    let read = format!(
        "{ARGS}## Main\nLet s be item 2 of args().\nLet mutable m be a new Map of Text to Text.\nSet item \"k\" of m to s.\nShow item \"k\" of m.\nShow s.\n"
    );
    let main = run(&read, "hi\nhi");
    assert!(main.contains("s.clone()"), "`s` is moved before its last use:\n{main}");
}
