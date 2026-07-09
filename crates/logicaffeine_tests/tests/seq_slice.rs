//! Native sequence slicing `items START through END of COLLECTION` (1-based, inclusive). The
//! bounds may be literal numbers OR parenthesized/computed expressions — `items (off + 1) through
//! (off + len) of s` must slice correctly (this is the zero-copy `&s[a..b]` form the crypto stdlib
//! uses for byte-string subranges). Byte-identical across tree-walker == VM == AOT.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn norm(s: &str) -> Vec<String> {
    s.lines().map(|l| l.trim_end().to_string()).filter(|l| !l.is_empty()).collect()
}

// [10,20,30,40,50] sliced 1-based-inclusive 2..=4 = items 2,3,4 = 20,30,40.
const BARE: &str = "## Main\nLet s be [10, 20, 30, 40, 50].\nLet r be items 2 through 4 of s.\nRepeat for x in r:\n    Show x.\n";
const PAREN_VAR: &str = "## Main\nLet s be [10, 20, 30, 40, 50].\nLet lo be 2.\nLet hi be 4.\nLet r be items (lo) through (hi) of s.\nRepeat for x in r:\n    Show x.\n";
const COMPUTED: &str = "## Main\nLet s be [10, 20, 30, 40, 50].\nLet off be 1.\nLet r be items (off + 1) through (off + 3) of s.\nRepeat for x in r:\n    Show x.\n";

#[test]
fn slice_bare_bounds() {
    let r = tw_outcome(BARE);
    assert_eq!(r.error, None, "bare slice compiles: {:?}", r.error);
    assert_eq!(norm(&r.output), vec!["20", "30", "40"], "items 2 through 4 of s");
}

#[test]
fn slice_parenthesized_variable_bounds() {
    let r = tw_outcome(PAREN_VAR);
    assert_eq!(r.error, None, "(lo)..(hi) slice compiles: {:?}", r.error);
    assert_eq!(norm(&r.output), vec!["20", "30", "40"], "items (lo) through (hi) of s");
}

#[test]
fn slice_computed_bounds() {
    let r = tw_outcome(COMPUTED);
    assert_eq!(r.error, None, "`items (off + 1) through (off + 3) of s` must slice: {:?}", r.error);
    assert_eq!(norm(&r.output), vec!["20", "30", "40"], "computed-bound slice");
}

#[test]
fn slice_tw_vm_byte_identical() {
    for (name, prog) in [("bare", BARE), ("paren_var", PAREN_VAR), ("computed", COMPUTED)] {
        let tw = tw_outcome(prog);
        let vm = vm_outcome(prog);
        assert_eq!(tw.error, None, "{name}: tw clean: {:?}", tw.error);
        assert_eq!(vm.error, None, "{name}: vm clean: {:?}", vm.error);
        assert_eq!(norm(&tw.output), norm(&vm.output), "{name}: tw == vm for slicing");
    }
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT slice gate"]
fn slice_aot_matches_treewalker() {
    fn norm1(s: &str) -> String {
        s.lines().map(|l| l.trim_end()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
    }
    for prog in [BARE, PAREN_VAR, COMPUTED] {
        let tw = tw_outcome(prog);
        assert_eq!(tw.error, None, "tw clean: {:?}", tw.error);
        let aot = common::run_logos_with_args(prog, &[]);
        assert!(aot.success, "AOT failed:\n{}\n{}", aot.stderr, aot.rust_code);
        assert_eq!(norm1(&aot.stdout), norm1(&tw.output), "AOT == tw for slicing");
    }
}
