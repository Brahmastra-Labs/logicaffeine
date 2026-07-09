//! Mutable Value Semantics — cross-tier (VM + tree-walker) agreement.
//!
//! Each test enables `LOGOS_VALUE_SEMANTICS` and runs through `interpret_for_ui`,
//! which executes the program on the VM AND, in debug, runs a shadow tree-walker
//! that `assert_eq!`s the two. So a PASS proves BOTH engines isolate identically
//! — the shadow oracle panics on any divergence. This is the cross-tier proof
//! that the copy-on-write flip agrees on the tree-walker and the VM.
//!
//! Scope: the ISOLATION cases (binding/plain-param/index/map), which both
//! engines handle via COW. The `mutable`-parameter PROPAGATION case is omitted —
//! the VM does not yet carry param-mutability in bytecode (the remaining
//! VM-compiler work); adding it here would diverge until that lands.
//!
//! Run with: `cargo nextest run --test value_semantics` (process-per-test, so
//! the process-global flag never races other suites).

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::{assert_compiled_equals_interpreted, assert_interpreter_output};
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
    assert_eq!(vm.output.trim(), "0\n1", "aliased array did not isolate under value semantics");
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
