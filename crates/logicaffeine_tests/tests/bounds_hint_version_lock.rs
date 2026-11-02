//! A loop whose bounds hint holds runs a copy that assumes it; the copy that assumes nothing runs otherwise.
//!
//! A counted loop that reads `item i of xs` up to a limit the Oracle cannot compare with `xs`'s length takes a bounds
//! hint: the length and the hint's truth hoisted before the loop (`__{xs}_bhl`, `__{xs}_bh_ok`). The assumption sat
//! inside the loop behind the flag, which LLVM had to unswitch to read the loop unchecked, and for counting_sort's
//! checksum loop it did not. The generated Rust versions the loop on the flags itself: `if <every flag> { the loop,
//! assuming each hint } else { the loop, assuming nothing }`. A loop that does index past the end runs the copy that
//! assumes nothing and fails where every other tier fails. Only a loop with no hinted loop inside is versioned.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test bounds_hint_version_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted, compile_to_rust, run_all_tiers};

fn benchmark(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../benchmarks/programs/{name}/main.lg"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every tier prints the same, compiled Rust prints the interpreter's output, and C prints it too.
fn answers_alike(source: &str) -> String {
    let agreed = run_all_tiers(source);
    assert!(agreed.error.is_none(), "{:?}\n{source}", agreed.error);
    assert_c_output(source, agreed.output.trim());
    assert_compiled_equals_interpreted(source);
    agreed.output.trim().to_string()
}

#[test]
fn the_counting_sort_checksum_loop_runs_a_copy_that_assumes_its_hint() {
    let rust = compile_to_rust(&benchmark("counting_sort")).unwrap();
    let versioned = rust
        .find("if __sorted_bh_ok {\n")
        .unwrap_or_else(|| panic!("the checksum loop is versioned on its hint:\n{rust}"));
    let (assuming, checked) = rust[versioned..]
        .split_once("} else {\n")
        .unwrap_or_else(|| panic!("a copy that assumes nothing follows:\n{rust}"));
    assert!(
        assuming.contains("unsafe { std::hint::assert_unchecked(")
            && !assuming.contains("if __sorted_bh_ok { unsafe"),
        "the first copy assumes the hint outright:\n{assuming}"
    );
    let checked = &checked[..checked.find("\n}\n").unwrap_or(checked.len())];
    assert!(
        !checked.contains("__sorted_bhl); }"),
        "the second copy assumes nothing about `sorted`:\n{checked}"
    );
}

#[test]
fn counting_sort_prints_alike_at_every_size() {
    for n in [1, 2, 3, 10, 999, 1000, 1001, 5000] {
        let source = benchmark("counting_sort").replace(
            "Let n be parseInt(item 2 of arguments).",
            &format!("Let n be {n}."),
        );
        answers_alike(&source);
    }
}

/// A loop reading past its list's end runs the copy that assumes nothing, so the compiled program fails at the same
/// item, in the interpreter's words.
#[test]
fn a_loop_past_the_end_fails_where_every_tier_fails() {
    let source = "## To native parseInt (s: Text) -> Int\n\n\
                  ## Main\n\
                  Let n be parseInt(\"5\").\n\
                  Let m be parseInt(\"3\").\n\
                  Let mutable xs be a new Seq of Int.\n\
                  Let mutable k be 1.\n\
                  While k is at most m:\n    Push k * 10 to xs.\n    Set k to k + 1.\n\
                  Let mutable total be 0.\n\
                  Let mutable i be 1.\n\
                  While i is at most n:\n    Set total to total + item i of xs.\n    Set i to i + 1.\n\
                  Show total.\n";
    let rust = compile_to_rust(source).unwrap();
    assert!(rust.contains("_bh_ok"), "the loop takes a hint:\n{rust}");
    let agreed = run_all_tiers(source);
    let want = agreed.error.expect("item 4 of a three-item list fails");
    let compiled = common::run_logos(source);
    assert!(
        !compiled.success,
        "{}\n{}",
        compiled.stdout, compiled.rust_code
    );
    assert_eq!(compiled.stdout.trim(), agreed.output.trim());
    assert!(
        compiled.stderr.contains(&want),
        "the compiled failure {:?} is not the interpreter's {want:?}\n{}",
        compiled.stderr,
        compiled.rust_code
    );
}

/// An outer hinted loop around an inner one keeps its conditional hint; only the innermost is versioned, so nesting
/// never doubles the code at every level. The lists are not affine, so the optimizer cannot read them as formulas and
/// both loops index them, and each item is pushed from one arm of a branch, so the Oracle cannot prove either length
/// and both loops keep a check for their hint to discharge. Both arms push the same value. Both print alike.
#[test]
fn only_the_innermost_hinted_loop_is_versioned() {
    let source = "## To native parseInt (s: Text) -> Int\n\n\
                  ## Main\n\
                  Let n be parseInt(\"6\").\n\
                  Let mutable xs be a new Seq of Int.\n\
                  Let mutable ys be a new Seq of Int.\n\
                  Let mutable k be 0.\n\
                  While k is less than n:\n    \
                      If k % 2 equals 0:\n        Push (k * k) % 11 to xs.\n        Push (k * k * 7) % 5 to ys.\n    \
                      Otherwise:\n        Push (k * k + 11) % 11 to xs.\n        Push (k * k * 7 + 5) % 5 to ys.\n    \
                      Set k to k + 1.\n\
                  Let mutable total be 0.\n\
                  Let mutable i be 1.\n\
                  While i is at most n:\n    \
                      Let mutable j be 1.\n    \
                      While j is at most n:\n        Set total to total + item j of ys.\n        Set j to j + 1.\n    \
                      Set total to total + item i of xs.\n    \
                      Set i to i + 1.\n\
                  Show total.\n";
    let rust = compile_to_rust(source).unwrap();
    assert!(
        rust.contains("let __xs_bh_ok") && rust.contains("let __ys_bh_ok"),
        "both loops take a hint (lists the optimizer cannot close into a formula):\n{rust}"
    );
    let versions = rust.matches("_bh_ok {\n").count();
    assert_eq!(
        versions, 1,
        "the innermost hinted loop alone is versioned:\n{rust}"
    );
    assert_eq!(answers_alike(source), "82");
}
