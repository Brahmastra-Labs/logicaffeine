//! Task B — the string-build loop fast path.
//!
//! `string_search` builds a multi-million-char haystack with `Set text to
//! text + ch` in a loop. Two costs dominate the per-iteration constant:
//!
//!   1. growing `text` — handled by the sole-owned in-place `add_assign`
//!      append (a `String::push_str` when the register exclusively owns the
//!      `Rc<String>`), so the buffer is amortized-O(1), not O(n) realloc.
//!   2. re-materialising the 1-char literal `ch` — every `Set ch to "a"`
//!      and every `Set text to text + "XXXXX"` reloads a `Constant::Text`.
//!      Rebuilding it from the `Constant` each load allocates a fresh
//!      `String` + `Rc`; sharing one materialised constant turns the reload
//!      into an `Rc` refcount bump (no heap traffic).
//!
//! Both are pure performance moves over the SHARED semantics kernel, so the
//! gate that proves them sound is the differential one: the bytecode VM must
//! produce BIT-IDENTICAL output to the tree-walker on every string-build
//! shape. These RED tests pin that, plus exactness on a build large enough
//! that a quadratic (per-iteration realloc) path would never finish in the
//! suite's budget.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome, tw_outcome_with_args, vm_outcome, vm_outcome_with_args};

fn assert_vm_matches_tw(src: &str) {
    let vm = vm_outcome(src);
    let tw = tw_outcome(src);
    assert_eq!(
        (vm.output.trim(), &vm.error),
        (tw.output.trim(), &tw.error),
        "VM diverged from the tree-walker on:\n{src}"
    );
}

fn assert_vm_matches_tw_args(src: &str, argv: &[String]) {
    let vm = vm_outcome_with_args(src, argv, None);
    let tw = tw_outcome_with_args(src, argv);
    assert_eq!(
        (vm.output.trim(), &vm.error),
        (tw.output.trim(), &tw.error),
        "VM diverged from the tree-walker on:\n{src}\nargv={argv:?}"
    );
}

/// The canonical accumulate shape: a single-char literal appended in a loop.
/// VM and tree-walker must agree on both the built string and its length.
#[test]
fn single_char_literal_append_matches_treewalker() {
    let src = "## Main\n\
               Let mutable text be \"\".\n\
               Let mutable i be 0.\n\
               While i is less than 10:\n\
               \x20   Set text to text + \"a\".\n\
               \x20   Set i to i + 1.\n\
               Show text.\n\
               Show length of text.\n";
    assert_vm_matches_tw(src);
    let vm = vm_outcome(src);
    assert_eq!(vm.output.trim(), "aaaaaaaaaa\n10", "built string wrong:\n{}", vm.output);
}

/// A branch-selected 1-char literal (`string_search`'s `ch`), appended to the
/// accumulator: the rhs is a freshly-loaded constant each iteration.
#[test]
fn branch_selected_char_append_matches_treewalker() {
    let src = "## Main\n\
               Let mutable text be \"\".\n\
               Let mutable i be 0.\n\
               While i is less than 25:\n\
               \x20   Let mutable ch be \"a\".\n\
               \x20   If i % 5 equals 1:\n\
               \x20       Set ch to \"b\".\n\
               \x20   If i % 5 equals 2:\n\
               \x20       Set ch to \"c\".\n\
               \x20   If i % 5 equals 3:\n\
               \x20       Set ch to \"d\".\n\
               \x20   If i % 5 equals 4:\n\
               \x20       Set ch to \"e\".\n\
               \x20   Set text to text + ch.\n\
               \x20   Set i to i + 1.\n\
               Show text.\n";
    assert_vm_matches_tw(src);
    let vm = vm_outcome(src);
    assert_eq!(vm.output.trim(), "abcdeabcdeabcdeabcdeabcde", "built string wrong:\n{}", vm.output);
}

/// A multi-char literal appended (`text + "XXXXX"`) — the in-place path must
/// `push_str` the whole constant, and the shared constant pool must not be
/// mutated by it (the next load of `"XXXXX"` must still see the original).
#[test]
fn multi_char_literal_append_does_not_corrupt_constant() {
    let src = "## Main\n\
               Let mutable text be \"\".\n\
               Let mutable i be 0.\n\
               While i is less than 3:\n\
               \x20   Set text to text + \"XY\".\n\
               \x20   Set i to i + 1.\n\
               Let mutable other be \"\".\n\
               Set other to other + \"XY\".\n\
               Show text.\n\
               Show other.\n";
    assert_vm_matches_tw(src);
    let vm = vm_outcome(src);
    assert_eq!(vm.output.trim(), "XYXYXY\nXY", "constant corrupted:\n{}", vm.output);
}

/// A LARGE build (50_000 single-char appends). With the amortized-O(1)
/// in-place append + shared-constant reload, this is linear; a per-iteration
/// realloc/alloc path would be quadratic and never land in budget. We assert
/// the exact length so a correctness regression cannot hide behind "it ran".
#[test]
fn large_single_char_build_is_correct() {
    let src = "## Main\n\
               Let mutable text be \"\".\n\
               Let mutable i be 0.\n\
               While i is less than 50000:\n\
               \x20   Set text to text + \"a\".\n\
               \x20   Set i to i + 1.\n\
               Show length of text.\n";
    assert_vm_matches_tw(src);
    let vm = vm_outcome(src);
    assert_eq!(vm.output.trim(), "50000", "wrong length:\n{}", vm.output);
}

/// The real string_search program: VM ≡ tree-walker on a real (small) size.
/// This is the end-to-end soundness anchor for the fast path the benchmark
/// exercises millions of times.
#[test]
fn string_search_program_matches_treewalker() {
    let path = format!(
        "{}/../../benchmarks/programs/string_search/main.lg",
        env!("CARGO_MANIFEST_DIR")
    );
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let argv = vec!["bench".to_string(), "1200".to_string()];
    assert_vm_matches_tw_args(&src, &argv);
}

/// A constant Text is also used as a MAP KEY / compared with `equals`: the
/// shared-constant pool must not change observable identity — a literal still
/// equals a separately-built one-char string. (Guards against a Char-vs-Text
/// representation drift if a later change interns literals as `Char`.)
#[test]
fn one_char_literal_equality_and_keying_unchanged() {
    let src = "## Main\n\
               Let a be \"x\".\n\
               Let mutable b be \"\".\n\
               Set b to b + \"x\".\n\
               If a equals b:\n\
               \x20   Show \"equal\".\n\
               Otherwise:\n\
               \x20   Show \"not-equal\".\n\
               Let m be a new Map of Text to Int.\n\
               Set item \"x\" of m to 7.\n\
               Show item b of m.\n";
    assert_vm_matches_tw(src);
    let vm = vm_outcome(src);
    assert_eq!(vm.output.trim(), "equal\n7", "equality/keying drifted:\n{}", vm.output);
}
