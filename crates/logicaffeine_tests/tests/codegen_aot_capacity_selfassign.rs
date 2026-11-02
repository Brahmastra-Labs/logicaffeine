//! AOT-native codegen regression locks for two use-before-def / invalid-Rust bugs
//! the stdlib `strings.lg` surfaced (they were invisible to the tree-walker≡VM
//! differential — the interpreter ignores the capacity hint and a self-assign is a
//! runtime no-op — so only the AOT tier miscompiled). Both go through
//! `assert_compiled_equals_interpreted`, which builds the generated Rust with rustc:
//! before the fix each of these failed to *compile*, so the harness went RED.
//!
//! Bug 1 — a `Set X to X` self-assignment lowered to `x = x;`, which is invalid when
//!   X is an immutable binding (a `for`-loop variable). Fixed by eliding self-assigns.
//! Bug 2 — the vec-presize peephole hoisted `with_capacity(<loop bound>)` to a Seq's
//!   declaration even when the bound is a local declared BETWEEN the Seq and the loop
//!   (`Let out …` `Let hi …` `While i ≤ hi`), emitting a use-before-def. Fixed by
//!   bailing (plain constructor) when the capacity operands aren't yet in scope.
#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::assert_compiled_equals_interpreted;

/// `Set X to X` on a `for`-loop variable must compile on the AOT tier (self-assign
/// elision). Mirrors `strings.lg`'s `isSpaceText`.
#[test]
fn self_assign_to_loop_variable_compiles_aot() {
    assert_compiled_equals_interpreted(
        "## To allSpaces (xs: Seq of Int) -> Bool:\n\
        \x20   Repeat for bt in xs:\n\
        \x20       If bt is equal to 32:\n\
        \x20           Set bt to bt.\n\
        \x20       Otherwise:\n\
        \x20           Return false.\n\
        \x20   Return true.\n\
        ## Main\n\
        Show allSpaces([32, 32, 32]).\n\
        Show allSpaces([32, 1, 32]).\n",
    );
}

/// A `new Seq` pre-sized by a loop whose bound is declared AFTER the Seq must
/// compile on the AOT tier (the capacity hint is dropped rather than hoisted into a
/// use-before-def). The flat shape.
#[test]
fn forward_referenced_capacity_bound_compiles_aot() {
    assert_compiled_equals_interpreted(
        "## To tailFrom (xs: Seq of Int) -> Seq of Int:\n\
        \x20   Let n be length of xs.\n\
        \x20   Let out be a new Seq of Int.\n\
        \x20   Let hi be n - 1.\n\
        \x20   Let mutable i be 1.\n\
        \x20   While i is at most hi:\n\
        \x20       Push item i of xs to out.\n\
        \x20       Set i to i + 1.\n\
        \x20   Return out.\n\
        ## Main\n\
        Show tailFrom([10, 20, 30, 40]).\n",
    );
}

/// Same forward-referenced capacity, but the Seq + fill loop live inside an `If`
/// block (the exact shape of `strings.lg`'s `removeSuffix`) — the peephole must
/// still bail when nested.
#[test]
fn forward_referenced_capacity_inside_if_compiles_aot() {
    assert_compiled_equals_interpreted(
        "## To dropLast (xs: Seq of Int) -> Seq of Int:\n\
        \x20   If (length of xs) is greater than 0:\n\
        \x20       Let n be length of xs.\n\
        \x20       Let out be a new Seq of Int.\n\
        \x20       Let hi be n - 1.\n\
        \x20       Let mutable i be 1.\n\
        \x20       While i is at most hi:\n\
        \x20           Push item i of xs to out.\n\
        \x20           Set i to i + 1.\n\
        \x20       Return out.\n\
        \x20   Return xs.\n\
        ## Main\n\
        Show dropLast([10, 20, 30, 40]).\n\
        Show dropLast([]).\n",
    );
}

/// A capacity bound declared BEFORE the Seq stays pre-sized (the fix must not
/// over-bail): `removePrefix`'s shape, where the loop limit `ns` precedes `out`.
#[test]
fn in_scope_capacity_bound_still_compiles_aot() {
    assert_compiled_equals_interpreted(
        "## To fromIndex (xs: Seq of Int) and (k: Int) -> Seq of Int:\n\
        \x20   Let ns be length of xs.\n\
        \x20   Let out be a new Seq of Int.\n\
        \x20   Let mutable i be k.\n\
        \x20   While i is at most ns:\n\
        \x20       Push item i of xs to out.\n\
        \x20       Set i to i + 1.\n\
        \x20   Return out.\n\
        ## Main\n\
        Show fromIndex([10, 20, 30, 40], 2).\n",
    );
}
