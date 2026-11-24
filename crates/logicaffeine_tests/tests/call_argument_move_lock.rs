//! ★ LOCK — a variable handed to a function that takes it by value is MOVED only at its last use, and copied otherwise. ★
//!
//! A parameter a function owns either takes the value handed to it (a move, free) or a copy. Handing it over is right
//! only when the program never reads that variable again: `Let loud be shout(s). Show s.` must leave `s` usable. The
//! Rust backend copied every variable it handed to such a parameter, even at its last mention, so a `Text` or a list
//! passed once was copied for nothing.
//!
//! The decision rests on liveness, per statement, as a construction's does (`construction_move_lock`): a variable is
//! moved into a call when the statement names it only there and either reassigns it or is the last statement that
//! uses it. A statement that runs more than once (a loop's) moves nothing. The rows build and run the Rust, and read
//! it, so a fix that copied everything would fail them.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test call_argument_move_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::compile_to_rust;

const SHOUT: &str = "## To native args () -> Seq of Text\n\n## To shout (t: Text) -> Text:\n    Let mutable out be t.\n    Set out to out + \"!\".\n    Return out.\n\n";

/// `src` run on AOT Rust with the argument `hi` prints `expected`; the Rust of its `Main`.
fn run(src: &str, expected: &str) -> String {
    let r = common::run_logos_with_args(src, &["hi"]);
    assert!(r.success, "the generated Rust must build and run:\n{}\n--- EMITTED ---\n{}", r.stderr, r.rust_code);
    assert_eq!(r.stdout.trim_end(), expected, "AOT Rust:\n{}", r.rust_code);
    let rust = compile_to_rust(src).expect("Rust for the program");
    let main = rust.find("fn _logos_main(").expect("a main");
    rust[main..].to_string()
}

/// A Text handed over at its last use is moved: the call takes it, with no copy.
#[test]
fn a_text_handed_over_at_its_last_use_is_moved() {
    let src = format!("{SHOUT}## Main\nLet s be item 2 of args().\nLet loud be shout(s).\nShow loud.\n");
    let main = run(&src, "hi!");
    assert!(main.contains("shout(s)"), "the call does not take `s`:\n{main}");
    assert!(!main.contains("shout(s.clone())"), "`s` is copied at its last use:\n{main}");
}

/// A Text read after the call is copied into it, and keeps its value.
#[test]
fn a_text_read_after_the_call_is_copied() {
    let src = format!("{SHOUT}## Main\nLet s be item 2 of args().\nLet loud be shout(s).\nShow loud.\nShow s.\n");
    let main = run(&src, "hi!\nhi");
    assert!(main.contains("shout(s.clone())"), "`s` is moved before its last use:\n{main}");
}

/// A Text handed over in a loop is copied every time: the statement runs again.
#[test]
fn a_text_handed_over_in_a_loop_is_copied_each_time() {
    let src = format!("{SHOUT}## Main\nLet s be item 2 of args().\nRepeat for i from 1 to 2:\n    Show shout(s).\n");
    let main = run(&src, "hi!\nhi!");
    assert!(main.contains("shout(s.clone())"), "`s` is moved inside a loop:\n{main}");
}

/// A Text named twice in the statement that hands it over is copied: the other mention reads it.
#[test]
fn a_text_named_twice_in_one_statement_is_copied() {
    let src = format!("{SHOUT}## Main\nLet s be item 2 of args().\nLet both be shout(s) + s.\nShow both.\n");
    let main = run(&src, "hi!hi");
    assert!(main.contains("shout(s.clone())"), "`s` is moved while the statement still reads it:\n{main}");
}
