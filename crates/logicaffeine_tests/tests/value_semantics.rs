//! Mutable Value Semantics — the tree-walker, the VM and the compiled binary agree.
//!
//! Each test enables `LOGOS_VALUE_SEMANTICS` and runs through `interpret_for_ui`,
//! which executes the program on the VM AND, in debug, runs a shadow tree-walker
//! that `assert_eq!`s the two; the `vs_aot_` rows also build it as Rust and hold the
//! binary to the same answer.
//!
//! Two rules: a copy is a value of its own (a binding, a plain parameter, an element
//! — isolated by copy-on-write, including across a loop that holds its list's borrow),
//! and a `mutable` parameter is the caller's own value, lent for the call (every write
//! the callee makes is the caller's, and a copy taken before the call keeps its value).
//! Arguments and the operands of text are taken left to right, so a read before a lend
//! sees the value before it.
//!
//! Run with: `cargo nextest run --test value_semantics` (process-per-test, so
//! the process-global flag never races other suites).

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{
    assert_all_tiers, assert_compiled_equals_interpreted, assert_compiled_equals_interpreted_eq,
    assert_interpreter_output,
};
use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

/// Enable value semantics for THIS test's process (nextest isolates processes).
/// The flag is inherited by the rustc-compiled AOT subprocess via the environment.
fn vs() {
    std::env::set_var("LOGOS_VALUE_SEMANTICS", "1");
}

// ---------------------------------------------------------------------------
// AOT tier: the rustc-compiled binary deep-clones `LogosSeq` (it reads the
// inherited flag), so it must AGREE with the interpreter under value semantics.
// This compiles real Rust — proving the third engine end-to-end, not just that
// `let b = a` isolates in the interpreter.
// ---------------------------------------------------------------------------

#[test]
fn vs_aot_agrees_with_interpreter_on_let_alias() {
    vs();
    assert_compiled_equals_interpreted(
        "## Main\nLet mutable a be a new Seq of Int.\nPush 1 to a.\nLet b be a.\nPush 2 to b.\nShow length of a.\n",
    );
}

// The liveness-gate retarget: a PLAIN param must isolate on AOT (no `&mut`
// borrow opt under value semantics — by-value + last-use move + deep-clone).
#[test]
fn vs_aot_plain_param_isolates() {
    vs();
    assert_compiled_equals_interpreted(
        "## To addItem (items: Seq of Int):\n    Push 99 to items.\n\n## Main\nLet mutable items be a new Seq of Int.\nPush 1 to items.\naddItem(items).\nShow length of items.\n",
    );
}

// AOT `mutable`-param propagation via the dedicated `&LogosSeq` ABI: the
// compiled binary passes the caller's collection by shared reference, so the
// callee's push reaches the caller. Must agree with the interpreter.
#[test]
fn vs_aot_mutable_param_propagates() {
    vs();
    assert_compiled_equals_interpreted(
        "## To addItem (items: mutable Seq of Int):\n    Push 99 to items.\n\n## Main\nLet mutable items be a new Seq of Int.\nPush 1 to items.\naddItem(items).\nShow length of items.\n",
    );
}

// A loop that resizes or writes a copy holds one `borrow_mut()` across the whole loop (borrow
// hoisting, codegen/hoist.rs). The copy still shares its buffer with the original until its first
// write, so the held borrow must be taken on the copy's OWN buffer: popping or overwriting the copy
// leaves the original as it was, on every tier, the compiled binary included.
const HOISTED_POP_OF_A_COPY: &str =
    "## Main\nLet a be [5, 1, 4].\nLet mutable b be a.\nWhile length of b is greater than 0:\n    Pop from b.\nShow a.\nShow length of b.\n";

const HOISTED_WRITE_OF_A_COPY: &str = "## Main\nLet a be [5, 1, 4].\nLet mutable b be a.\n\
    Let mutable i be 1.\nWhile i is at most 3:\n    Set item i of b to 0.\n    Set i to i + 1.\nShow a.\nShow b.\n";

// A `mutable` parameter is the one handle a hoisted loop writes THROUGH: the caller's list is the
// callee's, so draining it in a hoisted loop empties the caller's.
const HOISTED_DRAIN_OF_A_MUTABLE_PARAM: &str = "## To drain (xs: mutable Seq of Int):\n    While length of xs is greater than 0:\n        Pop from xs.\n\n\
    ## Main\nLet mutable a be [5, 1, 4].\ndrain(a).\nShow length of a.\n";

// The caller's list is lent, not a value copy of it: `b` was a copy of `a` before the call, and a
// copy keeps its value whatever is later done to `a`, through a `mutable` parameter as much as
// directly.
const DRAIN_OF_A_LIST_WITH_A_COPY: &str = "## To drain (xs: mutable Seq of Int):\n    While length of xs is greater than 0:\n        Pop from xs.\n\n\
    ## Main\nLet mutable a be [5, 1, 4].\nLet b be a.\ndrain(a).\nShow length of a.\nShow b.\n";

#[test]
fn vs_aot_hoisted_pop_loop_leaves_the_original() {
    vs();
    assert_all_tiers(HOISTED_POP_OF_A_COPY, "[5, 1, 4]\n0");
    assert_compiled_equals_interpreted_eq(HOISTED_POP_OF_A_COPY, "[5, 1, 4]\n0");
}

#[test]
fn vs_aot_hoisted_write_loop_leaves_the_original() {
    vs();
    assert_all_tiers(HOISTED_WRITE_OF_A_COPY, "[5, 1, 4]\n[0, 0, 0]");
    assert_compiled_equals_interpreted_eq(HOISTED_WRITE_OF_A_COPY, "[5, 1, 4]\n[0, 0, 0]");
}

#[test]
fn vs_aot_hoisted_drain_of_a_mutable_param_reaches_the_caller() {
    vs();
    assert_all_tiers(HOISTED_DRAIN_OF_A_MUTABLE_PARAM, "0");
    assert_compiled_equals_interpreted_eq(HOISTED_DRAIN_OF_A_MUTABLE_PARAM, "0");
}

#[test]
fn vs_aot_a_mutable_param_call_leaves_a_copy_of_the_argument_alone() {
    vs();
    assert_compiled_equals_interpreted_eq(DRAIN_OF_A_LIST_WITH_A_COPY, "0\n[5, 1, 4]");
    assert_all_tiers(DRAIN_OF_A_LIST_WITH_A_COPY, "0\n[5, 1, 4]");
}

const POP_LAST: &str = "## To popLast (xs: mutable Seq of Int) -> Int:\n    Let v be item (length of xs) of xs.\n    Pop from xs.\n    Return v.\n\n";

/// A lend is a write of the binding wherever the call sits — here a `Let`'s value — and the
/// binding need not be `mutable` to be lent.
#[test]
fn vs_aot_a_lend_inside_an_expression_leaves_a_copy_alone() {
    vs();
    let src = format!("{POP_LAST}## Main\nLet a be [5, 1, 4].\nLet b be a.\nLet n be popLast(a).\nShow n.\nShow a.\nShow b.\n");
    assert_compiled_equals_interpreted_eq(&src, "4\n[5, 1]\n[5, 1, 4]");
    assert_all_tiers(&src, "4\n[5, 1]\n[5, 1, 4]");
}

/// Arguments are values taken left to right: `a` read by the first argument is the list before
/// the second argument's lend pops it.
#[test]
fn vs_aot_an_argument_read_before_a_lend_of_the_same_list_keeps_its_value() {
    vs();
    let src = format!(
        "{POP_LAST}## To joined (xs: Seq of Int, last: Int) -> Seq of Int:\n    Let mutable out be copy of xs.\n    Push last to out.\n    Return out.\n\n\
         ## Main\nLet mutable a be [5, 1, 4].\nShow joined(a, popLast(a)).\nShow a.\n"
    );
    assert_compiled_equals_interpreted_eq(&src, "[5, 1, 4, 4]\n[5, 1]");
    assert_all_tiers(&src, "[5, 1, 4, 4]\n[5, 1]");
}

/// Text is built left to right: a field read before a call that writes the record through a
/// `mutable` parameter shows the field from before the call.
#[test]
fn vs_aot_text_built_around_a_lend_reads_left_to_right() {
    vs();
    let src = "## A Card has:\n    a name, which is Text.\n    a tags, which is Seq of Text.\n\n\
               ## To popTag (c: mutable Card) -> Text:\n    Let v be item 1 of c's tags.\n    \
               Set c's name to v.\n    Return v.\n\n\
               ## Main\nLet mutable c be a new Card with name \"ada\" and tags [\"x\", \"y\"].\n\
               Show c's name + \":\" + popTag(c) + \":\" + c's name.\nShow \"{c's name} {popTag(c)} {length of c's tags}\".\n";
    assert_compiled_equals_interpreted_eq(src, "ada:x:x\nx x 2");
    assert_all_tiers(src, "ada:x:x\nx x 2");
}

/// A statement takes its value — and a store its index, then its value — before it writes, so a
/// value whose call lends the very list being written is taken, lend and all, before the write.
#[test]
fn vs_aot_a_push_whose_value_lends_the_list_takes_the_value_first() {
    vs();
    let src = format!("{POP_LAST}## Main\nLet a be [5, 1, 4].\nPush popLast(a) to a.\nShow a.\n");
    assert_compiled_equals_interpreted_eq(&src, "[5, 1, 4]");
    assert_all_tiers(&src, "[5, 1, 4]");
}

#[test]
fn vs_aot_a_store_whose_value_lends_the_list_takes_the_value_first() {
    vs();
    let src = format!(
        "{POP_LAST}## Main\nLet a be [5, 1, 4].\nSet item 1 of a to popLast(a).\nShow a.\n"
    );
    assert_compiled_equals_interpreted_eq(&src, "[4, 1]");
    assert_all_tiers(&src, "[4, 1]");
}

/// The index is taken first, then the value, then the store — each lend over by the time the
/// store borrows the list or the map.
#[test]
fn vs_aot_a_store_whose_index_lends_the_collection_takes_the_index_first() {
    vs();
    let src = "## To takeFirst (xs: mutable Seq of Int) -> Int:\n    Let v be item 1 of xs.\n    Set item 1 of xs to 0.\n    Return v.\n\n\
               ## To keyAfterMark (m: mutable Map of Text to Int) -> Text:\n    Set item \"k\" of m to 1.\n    Return \"j\".\n\n\
               ## Main\nLet a be [1, 2, 3].\nSet item (takeFirst(a)) of a to 7.\nShow a.\n\
               Let m be a new Map of Text to Int.\nSet item (keyAfterMark(m)) of m to 5.\nShow item \"k\" of m.\nShow item \"j\" of m.\n";
    assert_compiled_equals_interpreted_eq(src, "[7, 2, 3]\n1\n5");
    assert_all_tiers(src, "[7, 2, 3]\n1\n5");
}

#[test]
fn vs_aot_an_add_whose_value_lends_the_set_takes_the_value_first() {
    vs();
    let src = "## To grab (s: mutable Set of Int) -> Int:\n    Add 9 to s.\n    Return 5.\n\n\
               ## Main\nLet s be a new Set of Int.\nAdd grab(s) to s.\nShow length of s.\nShow s contains 5.\n";
    assert_compiled_equals_interpreted_eq(src, "2\ntrue");
    assert_all_tiers(src, "2\ntrue");
}

/// The line that takes `handle`'s hoisted `borrow_mut()`, and the line before it.
fn hoisted_borrow_and_line_before<'r>(rust: &'r str, handle: &str) -> (&'r str, &'r str) {
    let guard = format!("let mut __{handle}_g = {handle}.borrow_mut();");
    let lines: Vec<&str> = rust.lines().collect();
    let at = lines
        .iter()
        .position(|l| l.trim() == guard)
        .unwrap_or_else(|| {
            panic!("the loop over `{handle}` is borrow-hoisted (`{guard}`):\n{rust}")
        });
    assert!(at > 0, "a hoist guard opens a scope first:\n{rust}");
    (lines[at], lines[at - 1])
}

/// Every mut hoist of a shared `LogosSeq` — resized (`&mut Vec`) and written (`&mut [T]`) —
/// copies-on-write the handle immediately before taking the loop's borrow, exactly as the
/// un-hoisted body does before each write.
#[test]
fn vs_aot_every_mut_hoist_of_a_shared_seq_copies_on_write_first() {
    vs();
    for (src, shadow) in [
        (HOISTED_POP_OF_A_COPY, "let b = &mut *__b_g;"),
        (HOISTED_WRITE_OF_A_COPY, "let b = &mut __b_g[..];"),
    ] {
        let rust = common::compile_to_rust(src).expect("compiles to Rust");
        assert!(
            rust.contains(shadow),
            "the hoist takes the expected shape `{shadow}`:\n{rust}"
        );
        let (_, before) = hoisted_borrow_and_line_before(&rust, "b");
        assert_eq!(
            before.trim(),
            "b.cow();",
            "the hoisted borrow is taken on `b`'s own buffer:\n{rust}"
        );
    }
}

/// A `mutable` parameter is hoisted without a copy-on-write: its writes are meant to reach the
/// caller, and the `&LogosSeq` it arrives as cannot give up its buffer.
#[test]
fn vs_aot_a_mutable_params_hoist_writes_through() {
    vs();
    let rust = common::compile_to_rust(HOISTED_DRAIN_OF_A_MUTABLE_PARAM).expect("compiles to Rust");
    let (_, before) = hoisted_borrow_and_line_before(&rust, "xs");
    assert_ne!(
        before.trim(),
        "xs.cow();",
        "a `mutable` parameter writes through to the caller:\n{rust}"
    );
    assert!(
        !rust.contains("xs.cow()"),
        "a `mutable` parameter is never copied on write:\n{rust}"
    );
}

// JIT tier: a hot loop mutating an ALIASED array. Under value semantics the JIT
// declines the in-place-mutation region (deopt), so it runs on the value-semantic
// VM (copy-on-write) — which must isolate `a` from `b` and agree with the
// tree-walker. Engages the forge JIT via ForgeTier (500-iter hot loop).
#[test]
fn vs_jit_aliased_array_isolates() {
    vs();
    let src = "## Main\n\
        Let mutable a be a new Seq of Int.\n\
        Let mutable i be 0.\n\
        While i is less than 500:\n\
        \x20   Push 0 to a.\n\
        \x20   Set i to i + 1.\n\
        Let b be a.\n\
        Set i to 1.\n\
        While i is at most 500:\n\
        \x20   Set item i of a to item i of b + i.\n\
        \x20   Set i to i + 1.\n\
        Show item 500 of b.\n\
        Show item 1 of a.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(vm.error, None, "JIT-tiered VM errored");
    assert_eq!(
        (vm.output.trim(), &vm.error),
        (tw.output.trim(), &tw.error),
        "JIT-tiered VM diverged from the tree-walker under value semantics",
    );
    // Value semantics: `b` keeps the original (0); `a` is isolated, a[1]=b[1]+1=1.
    assert_eq!(
        vm.output.trim(),
        "0\n1",
        "aliased array did not isolate under value semantics"
    );
}

#[test]
fn vs_let_binding_isolates() {
    vs();
    assert_interpreter_output(
        "## Main\nLet mutable a be a new Seq of Int.\nPush 1 to a.\nLet b be a.\nPush 2 to b.\nShow length of a.\n",
        "1",
    );
}

#[test]
fn vs_let_binding_other_side() {
    vs();
    assert_interpreter_output(
        "## Main\nLet mutable a be a new Seq of Int.\nPush 1 to a.\nLet mutable b be a.\nPush 2 to a.\nShow length of b.\n",
        "1",
    );
}

#[test]
fn vs_set_binding_isolates() {
    vs();
    assert_interpreter_output(
        "## Main\nLet mutable a be a new Seq of Int.\nPush 1 to a.\nLet mutable b be a new Seq of Int.\nSet b to a.\nPush 2 to b.\nShow length of a.\n",
        "1",
    );
}

#[test]
fn vs_plain_param_does_not_mutate_caller() {
    vs();
    assert_interpreter_output(
        "## To addItem (items: Seq of Int):\n    Push 99 to items.\n\n## Main\nLet mutable items be a new Seq of Int.\nPush 1 to items.\naddItem(items).\nShow length of items.\n",
        "1",
    );
}

#[test]
fn vs_set_index_through_binding_isolates() {
    vs();
    assert_interpreter_output(
        "## Main\nLet mutable a be a new Seq of Int.\nPush 10 to a.\nPush 20 to a.\nLet b be a.\nSet item 1 of b to 999.\nShow item 1 of a.\n",
        "10",
    );
}

#[test]
fn vs_map_binding_isolates() {
    vs();
    assert_interpreter_output(
        "## Main\nLet mutable a be a new Map of Text to Int.\nSet item \"x\" of a to 1.\nLet b be a.\nSet item \"x\" of b to 99.\nShow item \"x\" of a.\n",
        "1",
    );
}

#[test]
fn vs_copy_of_still_isolates() {
    vs();
    assert_interpreter_output(
        "## Main\nLet mutable a be a new Seq of Int.\nPush 1 to a.\nLet b be copy of a.\nPush 2 to b.\nShow length of a.\n",
        "1",
    );
}

#[test]
fn vs_owner_mutation_still_works() {
    vs();
    assert_interpreter_output(
        "## Main\nLet mutable a be a new Seq of Int.\nPush 1 to a.\nPush 2 to a.\nPush 3 to a.\nShow length of a.\n",
        "3",
    );
}

// ---------------------------------------------------------------------------
// `mutable` parameters PROPAGATE to the caller — the explicit by-reference escape
// hatch. Both engines must skip COW for a mutable-param register; a pass proves
// the VM (CompiledFunction.mutable_param_regs) and the tree-walker
// (is_mutable_param) agree on propagation, not just isolation.
// ---------------------------------------------------------------------------

#[test]
fn vs_mutable_param_mutates_caller() {
    vs();
    assert_interpreter_output(
        "## To addItem (items: mutable Seq of Int):\n    Push 99 to items.\n\n## Main\nLet mutable items be a new Seq of Int.\nPush 1 to items.\naddItem(items).\nShow length of items.\n",
        "2",
    );
}

#[test]
fn vs_mutable_param_set_index_propagates() {
    vs();
    assert_interpreter_output(
        "## To mutate (arr: mutable Seq of Int):\n    Set item 1 of arr to 999.\n\n## Main\nLet mutable arr be a new Seq of Int.\nPush 10 to arr.\nPush 20 to arr.\nmutate(arr).\nShow item 1 of arr.\n",
        "999",
    );
}

#[test]
fn vs_nested_mutable_params_propagate() {
    vs();
    assert_interpreter_output(
        "## To addOne (items: mutable Seq of Int):\n    Push 1 to items.\n\n## To addTwo (items: mutable Seq of Int):\n    addOne(items).\n    addOne(items).\n\n## Main\nLet mutable items be a new Seq of Int.\naddTwo(items).\nShow length of items.\n",
        "2",
    );
}
