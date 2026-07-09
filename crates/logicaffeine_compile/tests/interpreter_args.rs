//! `args()` in the interpreter: program arguments supplied to `largo run
//! --interpret N` must reach the running program exactly as the compiled
//! binary's `env::args()` does — `item 1` is the program name, `item 2` is the
//! first user argument. This is what lets a single `main.lg` (which reads its
//! size from `args()`) run on BOTH the native binary and the bytecode VM/JIT,
//! so the benchmark suite can compare compiled vs interpreted on one source.
//!
//! In debug builds `interpret_for_ui_sync_with_args` also runs the tree-walker
//! shadow oracle and asserts the VM agrees with it, so a single call exercises
//! both engines' `args()` path.

use logicaffeine_compile::interpret_for_ui_sync_with_args;

/// The canonical args-driven program: identical in shape to
/// `benchmarks/programs/fib/main.lg`.
const FIB: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To fib (n: Int) -> Int:
    If n is less than 2:
        Return n.
    Return fib(n - 1) + fib(n - 2).

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show fib(n).
"#;

fn argv(size: &str) -> Vec<String> {
    vec!["bench".to_string(), size.to_string()]
}

#[test]
fn args_supplies_the_size_to_the_interpreter() {
    let r = interpret_for_ui_sync_with_args(FIB, &argv("10"));
    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert_eq!(r.lines, vec!["55".to_string()], "fib(10) = 55");
}

#[test]
fn args_reflects_the_actual_argument_value() {
    // A different size proves the value flows through rather than a constant.
    let r = interpret_for_ui_sync_with_args(FIB, &argv("15"));
    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert_eq!(r.lines, vec!["610".to_string()], "fib(15) = 610");
}

#[test]
fn args_length_counts_argv0_plus_user_args() {
    // `length of args()` must see argv0 ("bench") plus the user argument, i.e.
    // 2 — matching the compiled binary's `env::args()`.
    let src = "## To native args () -> Seq of Text\n\
\n\
## Main\n\
Let arguments be args().\n\
Show length of arguments.\n";
    let r = interpret_for_ui_sync_with_args(src, &argv("99"));
    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert_eq!(r.lines, vec!["2".to_string()]);
}

#[test]
fn args_item_one_is_the_program_name() {
    let src = "## To native args () -> Seq of Text\n\
\n\
## Main\n\
Let arguments be args().\n\
Show item 1 of arguments.\n";
    let r = interpret_for_ui_sync_with_args(src, &argv("99"));
    assert!(r.error.is_none(), "unexpected error: {:?}", r.error);
    assert_eq!(r.lines, vec!["bench".to_string()]);
}
