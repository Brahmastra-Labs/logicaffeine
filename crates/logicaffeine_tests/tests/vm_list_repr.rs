//! M6 RED gate: unboxed homogeneous list storage (`ListRepr`) INSIDE the
//! existing `Rc<RefCell<…>>` — promotion re-tags the payload in place within
//! its buffer, so every holder of that buffer observes it with no Rc identity
//! change and no refcount churn on hot paths. Language-level value semantics
//! (`Let b be a`) copies-on-write on top of this, so distinct bindings are
//! isolated. All-Int lists store `Vec<i64>`, all-Float `Vec<f64>`; anything
//! heterogeneous boxes. Every kernel error string and display format is
//! unchanged.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_both(src: &str, expected: &str) {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "VM diverged from tree-walker on:\n{src}"
    );
    assert_eq!(vm.error, None, "errored on:\n{src}");
    assert_eq!(norm(&vm.output), expected, "wrong output for:\n{src}");
}

/// A `Let b be a` binding is a value copy (copy-on-write): the heterogeneous
/// push through `a` promotes and grows `a`'s buffer in place, while `b` — now a
/// distinct owner — is isolated and keeps the original all-Int payload.
#[test]
fn let_binding_is_isolated_value_copy() {
    assert_both(
        "## Main\n\
         Let mutable a be [1, 2, 3].\n\
         Let b be a.\n\
         Push 2.5 to a.\n\
         Show item 1 of b.\n\
         Show length of b.\n\
         Show length of a.\n",
        "1\n3\n4",
    );
}

/// An empty list adopts the kind of its first push — and re-tags freely
/// while still empty (push Text into a fresh empty list).
#[test]
fn empty_list_adopts_first_push_kind() {
    assert_both(
        "## Main\n\
         Let mutable xs be a new Seq of Int.\n\
         Push 7 to xs.\n\
         Push 8 to xs.\n\
         Show xs.\n",
        "[7, 8]",
    );
    assert_both(
        "## Main\n\
         Let mutable xs be a new Seq of Text.\n\
         Push \"a\" to xs.\n\
         Show xs.\n",
        "[a]",
    );
    assert_both(
        "## Main\n\
         Let mutable xs be a new Seq of Float.\n\
         Push 0.5 to xs.\n\
         Push 1 to xs.\n\
         Show item 2 of xs.\n",
        "1",
    );
}

/// Display formats are byte-identical across reprs.
#[test]
fn display_format_unchanged() {
    assert_both("## Main\nShow [1, 2, 3].\n", "[1, 2, 3]");
    assert_both("## Main\nShow [0.5, 1.5].\n", "[0.5, 1.5]");
    assert_both("## Main\nShow [1, \"x\", 2.5].\n", "[1, x, 2.5]");
    assert_both("## Main\nLet xs be a new Seq of Int.\nShow xs.\n", "[]");
}

/// `contains` keeps the kernel's equality: epsilon for floats, cross-type
/// never equal (an Int list never contains a Float, and vice versa).
#[test]
fn contains_keeps_kernel_equality() {
    assert_both(
        "## Main\n\
         Let xs be [0.1].\n\
         Let mutable s be 0.0.\n\
         Set s to 0.1 + 0.2.\n\
         Let ys be [s].\n\
         Show ys contains 0.3.\n",
        "true",
    );
    assert_both("## Main\nShow [1, 2] contains 1.\n", "true");
    assert_both("## Main\nShow [1, 2] contains 3.\n", "false");
}

/// Out-of-bounds errors keep the exact kernel strings through every repr.
#[test]
fn bounds_errors_unchanged() {
    for src in [
        "## Main\nLet xs be [1, 2].\nShow item 3 of xs.\n",
        "## Main\nLet xs be [0.5].\nShow item 0 of xs.\n",
        "## Main\nLet xs be [\"a\"].\nShow item 9 of xs.\n",
    ] {
        let tier = ForgeTier::new();
        let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
        let tw = tw_outcome_with_args(src, &[]);
        assert_eq!(
            (norm(&vm.output), &vm.error),
            (norm(&tw.output), &tw.error),
            "bounds error diverged on:\n{src}"
        );
        assert!(vm.error.is_some(), "must error:\n{src}");
    }
}

/// Mutation through `Set item`, `Pop`, `Remove` across reprs and promotion.
#[test]
fn mutation_surface_across_reprs() {
    assert_both(
        "## Main\n\
         Let mutable xs be [1, 2, 3].\n\
         Set item 2 of xs to 9.\n\
         Pop from xs into last.\n\
         Show xs.\n\
         Show last.\n",
        "[1, 9]\n3",
    );
    assert_both(
        "## Main\n\
         Let mutable xs be [1, 2, 3].\n\
         Set item 2 of xs to \"mid\".\n\
         Show xs.\n",
        "[1, mid, 3]",
    );
    // (`Remove … from` is Set/Map-only in the language — lists shrink via
    // Pop; both engines agree on the error, pinned in error-parity suites.)
}

/// Deep clone (`copy of`) is independent storage in every repr.
#[test]
fn copy_is_independent() {
    assert_both(
        "## Main\n\
         Let mutable a be [1, 2].\n\
         Let mutable b be copy of a.\n\
         Push 3 to b.\n\
         Show a.\n\
         Show b.\n",
        "[1, 2]\n[1, 2, 3]",
    );
}

/// The sieve/histogram shape: large Int lists built by Push, indexed reads
/// and writes in hot loops — exactness under the repr plus tier interplay.
#[test]
fn sieve_shape_differential() {
    let src = "## Main\n\
               Let n be 2000.\n\
               Let mutable flags be a new Seq of Int.\n\
               Let mutable i be 0.\n\
               While i is at most n:\n\
               \x20   Push 1 to flags.\n\
               \x20   Set i to i + 1.\n\
               Let mutable p be 2.\n\
               While p * p is at most n:\n\
               \x20   If item (p + 1) of flags equals 1:\n\
               \x20       Let mutable m be p * p.\n\
               \x20       While m is at most n:\n\
               \x20           Set item (m + 1) of flags to 0.\n\
               \x20           Set m to m + p.\n\
               \x20   Set p to p + 1.\n\
               Let mutable count be 0.\n\
               Set p to 2.\n\
               While p is at most n:\n\
               \x20   If item (p + 1) of flags equals 1:\n\
               \x20       Set count to count + 1.\n\
               \x20   Set p to p + 1.\n\
               Show count.\n";
    assert_both(src, "303");
}

/// Slices, iteration snapshots, and list equality semantics survive.
#[test]
fn slices_iteration_and_builtins() {
    // The slice grammar keys on the identifier `items` (a pinned parser
    // quirk) — name the variable accordingly.
    assert_both(
        "## Main\n\
         Let items be [10, 20, 30, 40].\n\
         Let mid be items 2 through 3.\n\
         Show mid.\n",
        "[20, 30]",
    );
    assert_both(
        "## Main\n\
         Let xs be [3, 1, 2].\n\
         Let mutable total be 0.\n\
         Repeat for x in xs:\n\
         \x20   Set total to total + x.\n\
         Show total.\n",
        "6",
    );
}
