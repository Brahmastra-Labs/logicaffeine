//! LOCK — a function reaches the program's own bindings on the compiled tiers.
//!
//! A `## To` function reads and writes the bindings `## Main` declares: `Set counter to counter + 1`
//! inside `## To bump:` moves the `counter` that `Let mutable counter be 0.` made, and the next
//! `Show counter.` sees it. The tree-walker and the VM keep one binding for the whole program. A
//! compiled tier that makes each of them a local of its `main` hands every such function a name
//! nothing declares, and the program does not build — `largo run` called it a compiler bug. Every
//! lowered `## Session` is this shape (the session, its secret and the payload it arrived with are
//! the program's bindings, read by `csrf()` and by every route), so no server with a session could
//! be shipped as a binary.
//!
//! Each row is held compiled ≡ interpreted AND to its expected answer, so both tiers agreeing on a
//! wrong answer still fails.

mod common;

use common::{
    assert_c_output, assert_compiled_equals_interpreted, assert_compiled_equals_interpreted_eq,
};
use logicaffeine_compile::compile::{compile_to_rust, tw_outcome, vm_outcome};

const COUNTER: &str = "## To bump:\n    Set counter to counter + 1.\n\n\
                       ## Main\nLet mutable counter be 0.\nbump().\nbump().\nShow counter.\n";

const TRANSITIVE: &str = "## To bump:\n    Set counter to counter + 1.\n\n\
                          ## To twice:\n    bump().\n    bump().\n\n\
                          ## Main\nLet mutable counter be 0.\ntwice().\ntwice().\nShow counter.\n";

const TEXT: &str = "## To greet (who: Text):\n    Set log to log + who + \";\".\n\n\
                    ## Main\nLet mutable log be \"\".\ngreet(\"a\").\ngreet(\"b\").\nShow log.\n";

const RECORD: &str = "## A Config has:\n    a name, which is Text.\n\n\
                      ## To rename (n: Text):\n    Set cfg's name to n.\n\n\
                      ## Main\nLet mutable cfg be a new Config with name \"x\".\nrename(\"y\").\nShow cfg's name.\n";

const SEQ: &str = "## To remember (n: Int):\n    Push n to seen.\n\n\
                   ## Main\nLet mutable seen be a new Seq of Int.\nremember(3).\nremember(4).\n\
                   Show length of seen.\nShow item 2 of seen.\n";

const MAP: &str = "## To score (k: Text) and (v: Int):\n    Set item k of scores to v.\n\n\
                   ## Main\nLet mutable scores be a new Map of Text to Int.\nscore(\"a\", 1).\nscore(\"b\", 2).\n\
                   Show item \"b\" of scores.\n";

const READ_ONLY: &str = "## To scaled (x: Int) -> Int:\n    Return x * factor.\n\n\
                         ## Main\nLet factor be 3.\nShow scaled(2).\n";

const SHADOWED: &str = "## To shadow (counter: Int) -> Int:\n    Return counter + 1.\n\n\
                        ## Main\nLet counter be 10.\nShow shadow(1).\nShow counter.\n";

const READ_BESIDE_A_WRITER: &str = "## To next -> Int:\n    Set counter to counter + 1.\n    Return counter.\n\n\
                                    ## Main\nLet mutable counter be 0.\nShow counter + next().\nShow counter.\n";

const PUSHED_BY_A_WRITER: &str = "## To next -> Int:\n    Push 0 to seen.\n    Return length of seen.\n\n\
                                  ## Main\nLet mutable seen be a new Seq of Int.\nPush next() to seen.\n\
                                  Show length of seen.\nShow item 2 of seen.\n";

const READ_BEFORE_ITS_LET: &str = "## To peek -> Int:\n    Return late.\n\n\
                                   ## Main\nShow peek().\nLet late be 1.\n";

/// A function's frame sees its own locals and then the program's bindings — never its caller's
/// locals (the tree-walker's lexical barrier, `Interpreter::push_frame`).
const CALLERS_LOCAL: &str = "## To peek -> Int:\n    Return x.\n\n\
                             ## To outer -> Int:\n    Let x be 7.\n    Return peek().\n\n\
                             ## Main\nLet x be 1.\nShow outer().\nShow x.\n";

/// A binding made inside a block of `## Main` belongs to that block, not to the program.
const BLOCK_OF_MAIN: &str = "## To peek -> Int:\n    Return inner.\n\n\
                             ## Main\nIf true:\n    Let inner be 5.\n    Show peek().\n";

#[test]
fn a_function_moves_a_count_the_program_declares() {
    assert_compiled_equals_interpreted_eq(COUNTER, "2");
}

#[test]
fn a_function_reaches_it_through_another_function() {
    assert_compiled_equals_interpreted_eq(TRANSITIVE, "4");
}

#[test]
fn a_function_extends_the_programs_text() {
    assert_compiled_equals_interpreted_eq(TEXT, "a;b;");
}

#[test]
fn a_function_sets_a_field_of_the_programs_record() {
    assert_compiled_equals_interpreted_eq(RECORD, "y");
}

#[test]
fn a_function_pushes_to_the_programs_list() {
    assert_compiled_equals_interpreted_eq(SEQ, "2\n4");
}

#[test]
fn a_function_writes_an_entry_of_the_programs_map() {
    assert_compiled_equals_interpreted_eq(MAP, "2");
}

#[test]
fn a_function_reads_a_binding_it_never_writes() {
    assert_compiled_equals_interpreted_eq(READ_ONLY, "6");
}

#[test]
fn a_parameter_of_the_same_name_is_the_functions_own() {
    assert_compiled_equals_interpreted_eq(SHADOWED, "2\n10");
}

#[test]
fn a_read_beside_a_call_that_writes_it_agrees_with_the_interpreter() {
    assert_compiled_equals_interpreted(READ_BESIDE_A_WRITER);
}

#[test]
fn a_push_of_a_value_whose_call_writes_the_same_list_agrees_with_the_interpreter() {
    assert_compiled_equals_interpreted(PUSHED_BY_A_WRITER);
}

#[test]
fn a_binding_read_before_its_let_runs_fails_on_every_tier() {
    assert_compiled_equals_interpreted(READ_BEFORE_ITS_LET);
}

#[test]
fn a_function_reads_the_programs_binding_and_never_its_callers_local() {
    let tw = tw_outcome(CALLERS_LOCAL);
    assert_eq!(
        (tw.output.trim(), tw.error.as_deref()),
        ("1\n1", None),
        "the tree-walker"
    );
    let vm = vm_outcome(CALLERS_LOCAL);
    assert_eq!(
        (vm.output.trim(), vm.error.as_deref()),
        ("1\n1", None),
        "the VM"
    );
    assert_compiled_equals_interpreted_eq(CALLERS_LOCAL, "1\n1");
}

#[test]
fn a_binding_inside_a_block_of_main_is_no_functions_to_read() {
    assert!(
        tw_outcome(BLOCK_OF_MAIN).error.is_some(),
        "the tree-walker refuses the read"
    );
    assert!(
        vm_outcome(BLOCK_OF_MAIN).error.is_some(),
        "the VM refuses the read"
    );
    assert_compiled_equals_interpreted(BLOCK_OF_MAIN);
}

#[test]
fn a_program_whose_functions_reach_no_binding_is_emitted_as_it_was() {
    let src =
        "## To twice (x: Int) -> Int:\n    Return x * 2.\n\n## Main\nLet n be 4.\nShow twice(n).\n";
    let rust = compile_to_rust(src).expect("compiles");
    assert!(
        !rust.contains("__Program"),
        "nothing to lift, nothing lifted:\n{rust}"
    );
}

#[test]
fn the_bindings_functions_reach_are_the_fields_of_one_lent_record() {
    let rust = compile_to_rust(TRANSITIVE).expect("compiles");
    assert!(rust.contains("struct __Program"), "{rust}");
    assert!(
        rust.contains("fn bump(__program: "),
        "a function that names the binding takes the record:\n{rust}"
    );
    assert!(
        rust.contains("fn twice(__program: "),
        "and so does one that reaches it through a call:\n{rust}"
    );
}

#[test]
fn a_call_before_the_binding_is_declared_is_refused_by_name() {
    let err = compile_to_rust(READ_BEFORE_ITS_LET).expect_err("refused on this tier");
    let shown = format!("{err:?}");
    assert!(
        shown.contains("`peek` reads the program's `late`"),
        "{shown}"
    );
}

#[test]
fn a_task_on_a_function_that_reaches_the_programs_bindings_is_refused_by_name() {
    let src = "## To bump:\n    Set counter to counter + 1.\n\n## Main\nLet mutable counter be 0.\nLaunch a task to bump.\nShow counter.\n";
    let err = compile_to_rust(src).expect_err("refused on this tier");
    let shown = format!("{err:?}");
    assert!(shown.contains("`bump` is launched as a task"), "{shown}");
}

#[test]
fn the_c_tier_keeps_the_programs_bindings_too() {
    for (src, expected) in [
        (COUNTER, "2"),
        (TRANSITIVE, "4"),
        (TEXT, "a;b;"),
        (RECORD, "y"),
        (SEQ, "2\n4"),
        (MAP, "2"),
        (READ_ONLY, "6"),
        (SHADOWED, "2\n10"),
        (CALLERS_LOCAL, "1\n1"),
    ] {
        assert_c_output(src, expected);
    }
}

/// Text is built left to right: a read of the program's binding written before a call that writes
/// it shows the value from before the call, on the tree-walker, the VM and the compiled binary.
#[test]
fn text_built_around_a_call_that_writes_the_binding_reads_left_to_right() {
    let src = "## To next -> Int:\n    Set counter to counter + 1.\n    Return counter.\n\n\
               ## Main\nLet mutable counter be 0.\nShow \"{counter} {next()} {counter}\".\n\
               Let mutable label be \"a\".\nShow label + next() + label.\n";
    let expected = "0 1 1\na2a";
    for (tier, out) in [("tree-walker", tw_outcome(src)), ("VM", vm_outcome(src))] {
        assert_eq!(
            (out.output.trim(), out.error.as_deref()),
            (expected, None),
            "the {tier}"
        );
    }
    let aot = common::run_logos(src);
    assert!(
        aot.success,
        "the program builds:\n{}\n{}",
        aot.stderr, aot.rust_code
    );
    assert_eq!(aot.stdout.trim(), expected, "the compiled binary");
}

/// A route's page reading the session beside a `csrf()` — which writes the session — builds as a
/// binary: the read is taken before the call, as every tier takes it.
#[test]
fn a_page_reading_the_session_beside_csrf_builds_as_a_binary() {
    let src = "## Session\n    a user, which is Text.\n\n\
               ## Route GET /dashboard -> Response:\n    \
               Return html(\"<h1>\" + the session's user + \"</h1><input value=\\\"\" + csrf() + \"\\\">\" + the session's user).\n\n\
               ## Main\nServe on port 8080.\n";
    let built = common::compile_logos(src);
    assert!(
        built.success,
        "the server builds:\n{}\n{}",
        built.stderr, built.rust_code
    );
}
