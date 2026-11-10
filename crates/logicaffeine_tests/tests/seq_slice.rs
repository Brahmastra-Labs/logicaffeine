//! Native sequence slicing `items START through END of COLLECTION` (1-based, inclusive). The
//! bounds may be literal numbers OR parenthesized/computed expressions — `items (off + 1) through
//! (off + len) of s` must slice correctly (this is the zero-copy `&s[a..b]` form the crypto stdlib
//! uses for byte-string subranges). Byte-identical across tree-walker == VM == AOT.
//!
//! A slice is a new list of the items it names, whatever it slices — a binding, a field, a call's
//! result — and wherever it goes: held, returned, passed, stored in a record or a list, walked. It is
//! its own list from then on: writing it leaves the list it came from as it was, and writing that list
//! leaves the slice as it was.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn norm(s: &str) -> Vec<String> {
    s.lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

// [10,20,30,40,50] sliced 1-based-inclusive 2..=4 = items 2,3,4 = 20,30,40.
const BARE: &str = "## Main\nLet s be [10, 20, 30, 40, 50].\nLet r be items 2 through 4 of s.\nRepeat for x in r:\n    Show x.\n";
const PAREN_VAR: &str = "## Main\nLet s be [10, 20, 30, 40, 50].\nLet lo be 2.\nLet hi be 4.\nLet r be items (lo) through (hi) of s.\nRepeat for x in r:\n    Show x.\n";
const COMPUTED: &str = "## Main\nLet s be [10, 20, 30, 40, 50].\nLet off be 1.\nLet r be items (off + 1) through (off + 3) of s.\nRepeat for x in r:\n    Show x.\n";

#[test]
fn slice_bare_bounds() {
    let r = tw_outcome(BARE);
    assert_eq!(r.error, None, "bare slice compiles: {:?}", r.error);
    assert_eq!(
        norm(&r.output),
        vec!["20", "30", "40"],
        "items 2 through 4 of s"
    );
}

#[test]
fn slice_parenthesized_variable_bounds() {
    let r = tw_outcome(PAREN_VAR);
    assert_eq!(r.error, None, "(lo)..(hi) slice compiles: {:?}", r.error);
    assert_eq!(
        norm(&r.output),
        vec!["20", "30", "40"],
        "items (lo) through (hi) of s"
    );
}

#[test]
fn slice_computed_bounds() {
    let r = tw_outcome(COMPUTED);
    assert_eq!(
        r.error, None,
        "`items (off + 1) through (off + 3) of s` must slice: {:?}",
        r.error
    );
    assert_eq!(
        norm(&r.output),
        vec!["20", "30", "40"],
        "computed-bound slice"
    );
}

#[test]
fn slice_tw_vm_byte_identical() {
    for (name, prog) in [
        ("bare", BARE),
        ("paren_var", PAREN_VAR),
        ("computed", COMPUTED),
    ] {
        let tw = tw_outcome(prog);
        let vm = vm_outcome(prog);
        assert_eq!(tw.error, None, "{name}: tw clean: {:?}", tw.error);
        assert_eq!(vm.error, None, "{name}: vm clean: {:?}", vm.error);
        assert_eq!(
            norm(&tw.output),
            norm(&vm.output),
            "{name}: tw == vm for slicing"
        );
    }
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT slice gate"]
fn slice_aot_matches_treewalker() {
    fn norm1(s: &str) -> String {
        s.lines()
            .map(|l| l.trim_end())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }
    for prog in [BARE, PAREN_VAR, COMPUTED] {
        let tw = tw_outcome(prog);
        assert_eq!(tw.error, None, "tw clean: {:?}", tw.error);
        let aot = common::run_logos_with_args(prog, &[]);
        assert!(
            aot.success,
            "AOT failed:\n{}\n{}",
            aot.stderr, aot.rust_code
        );
        assert_eq!(
            norm1(&aot.stdout),
            norm1(&tw.output),
            "AOT == tw for slicing"
        );
    }
}

/// `prog` prints `printed` on the tree-walker, the VM and the generated Rust, built and run.
fn assert_sliced_everywhere(prog: &str, printed: &str) {
    for (tier, run) in [("the tree-walker", tw_outcome(prog)), ("the VM", vm_outcome(prog))] {
        assert_eq!(run.error, None, "{tier} refused:\n{prog}");
        assert_eq!(run.output.trim(), printed, "{tier}:\n{prog}");
    }
    let aot = common::run_logos(prog);
    assert!(aot.success, "the generated Rust must build and run:\n{}\n{prog}", aot.stderr);
    assert_eq!(aot.stdout.trim(), printed, "the generated Rust:\n{prog}");
}

#[test]
fn a_slice_of_a_calls_result_is_read_returned_and_held() {
    assert_sliced_everywhere(
        "## To trio (n: Int) -> Seq of Int:\n    Return [n, n + 1, n + 2].\n\n## To middle (n: Int) -> Seq of Int:\n    Return items 2 through 3 of trio(n).\n\n## Main\nShow items 1 through 2 of trio(1).\nShow middle(5).\nLet ys be items 2 through 3 of trio(7).\nShow ys.\nShow length of (items 1 through 2 of trio(9)).\nShow (items 1 through 3 of trio(9)) contains 10.\nShow (items 1 through 3 of trio(9)) contains 4.\n",
        "[1, 2]\n[6, 7]\n[8, 9]\n2\ntrue\nfalse",
    );
}

#[test]
fn a_slice_is_passed_stored_in_a_record_and_a_list_and_walked() {
    assert_sliced_everywhere(
        "## A Pair has:\n    a left: Seq of Int.\n    a right: Seq of Int.\n\n## To total (xs: Seq of Int) -> Int:\n    Let mutable t be 0.\n    Repeat for x in xs:\n        Set t to t + x.\n    Return t.\n\n## Main\nLet xs be [1, 2, 3, 4].\nShow total(items 2 through 3 of xs).\nLet p be a new Pair with left (items 1 through 2 of xs) and right (items 3 through 4 of xs).\nShow p's left.\nShow p's right.\nLet mutable groups be a new Seq of Seq of Int.\nPush items 1 through 2 of xs to groups.\nPush items 3 through 4 of xs to groups.\nShow groups.\nRepeat for x in items 2 through 4 of xs:\n    Show x.\n",
        "5\n[1, 2]\n[3, 4]\n[[1, 2], [3, 4]]\n2\n3\n4",
    );
}

#[test]
fn a_slice_is_a_list_of_its_own() {
    assert_sliced_everywhere(
        "## To firstTwo (xs: Seq of Int) -> Seq of Int:\n    Return items 1 through 2 of xs.\n\n## Main\nLet mutable xs be [1, 2, 3].\nLet mutable ys be firstTwo(xs).\nPush 9 to ys.\nSet item 1 of ys to 7.\nShow xs.\nShow ys.\nLet zs be items 2 through 3 of xs.\nSet item 2 of xs to 0.\nShow zs.\n",
        "[1, 2, 3]\n[7, 2, 9]\n[2, 3]",
    );
}
