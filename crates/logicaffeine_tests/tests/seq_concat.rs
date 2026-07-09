//! `a followed by b` — native sequence concatenation: merge two sequences into one, the
//! structural sibling of string `combined with`. `Add x to s` pushes one element; this joins
//! whole sequences. Must be byte-identical across tree-walker == VM == AOT, chainable, and
//! compose with slicing/indexing/length.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn norm(s: &str) -> Vec<String> {
    s.lines().map(|l| l.trim_end().to_string()).filter(|l| !l.is_empty()).collect()
}

const BASIC: &str = "## Main\nLet a be [1, 2, 3].\nLet b be [4, 5, 6].\nLet c be a followed by b.\nRepeat for x in c:\n    Show x.\n";

const CHAINED: &str = "## Main\nLet a be [1, 2].\nLet b be [3, 4].\nLet c be [5, 6].\nLet d be a followed by b followed by c.\nShow length of d.\nRepeat for x in d:\n    Show x.\n";

const EMPTY_LHS: &str = "## Main\nLet a be a new Seq of Int.\nLet b be [7, 8, 9].\nLet c be a followed by b.\nShow length of c.\nRepeat for x in c:\n    Show x.\n";

const COMPOSED: &str = "## Main\nLet a be [10, 20, 30].\nLet b be [40, 50, 60].\nLet c be a followed by b.\nShow item 1 of c.\nShow item 6 of c.\nShow length of c.\n";

#[test]
fn seq_concat_basic_merges_two_sequences() {
    let r = tw_outcome(BASIC);
    assert_eq!(r.error, None, "`followed by` compiles + runs: {:?}", r.error);
    assert_eq!(norm(&r.output), vec!["1", "2", "3", "4", "5", "6"], "a followed by b = merge");
}

#[test]
fn seq_concat_is_chainable() {
    let r = tw_outcome(CHAINED);
    assert_eq!(r.error, None, "chained `followed by`: {:?}", r.error);
    assert_eq!(norm(&r.output), vec!["6", "1", "2", "3", "4", "5", "6"], "a followed by b followed by c");
}

#[test]
fn seq_concat_empty_operand() {
    let r = tw_outcome(EMPTY_LHS);
    assert_eq!(r.error, None, "empty-lhs concat: {:?}", r.error);
    assert_eq!(norm(&r.output), vec!["3", "7", "8", "9"], "empty followed by b = b");
}

#[test]
fn seq_concat_composes_with_index_and_length() {
    let r = tw_outcome(COMPOSED);
    assert_eq!(r.error, None, "concat + index/length: {:?}", r.error);
    assert_eq!(norm(&r.output), vec!["10", "60", "6"], "indexing/length over a concatenation");
}

#[test]
fn seq_concat_tw_vm_byte_identical() {
    for (name, prog) in [("basic", BASIC), ("chained", CHAINED), ("empty", EMPTY_LHS), ("composed", COMPOSED)] {
        let tw = tw_outcome(prog);
        let vm = vm_outcome(prog);
        assert_eq!(tw.error, None, "{name}: tw clean: {:?}", tw.error);
        assert_eq!(vm.error, None, "{name}: vm clean: {:?}", vm.error);
        assert_eq!(norm(&tw.output), norm(&vm.output), "{name}: tw == vm for `followed by`");
    }
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT `followed by` gate"]
fn seq_concat_aot_matches_treewalker() {
    fn norm1(s: &str) -> String {
        s.lines().map(|l| l.trim_end()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
    }
    for prog in [BASIC, CHAINED, EMPTY_LHS, COMPOSED] {
        let tw = tw_outcome(prog);
        assert_eq!(tw.error, None, "tw clean: {:?}", tw.error);
        let aot = common::run_logos_with_args(prog, &[]);
        assert!(aot.success, "AOT failed:\n{}\n{}", aot.stderr, aot.rust_code);
        assert_eq!(norm1(&aot.stdout), norm1(&tw.output), "AOT == tw for `followed by`");
    }
}
