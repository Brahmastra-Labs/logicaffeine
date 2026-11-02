//! ★ Deep-check FIDELITY LOCK — `rustc_check` compiles what `largo build` ships. ★
//!
//! `largo build` (`compile_project` = prune ∘ apply_prelude) drops the DEAD stdlib
//! prelude: the whole-module auto-import splices ALL of `strings.lg` when a program
//! mentions any one of its functions, then `prune_unreachable_prelude` removes the
//! ones the program never reaches. Deep check (`rustc_check`) must do the SAME before
//! it parses — otherwise it type-checks stdlib code the binary never contains, and an
//! imperfection (or a parse quirk) in an UNREACHABLE helper surfaces as a finding on
//! code that never runs. This lock pins that fidelity directly on the generated Rust,
//! with no cargo in the loop:
//!   * an UNREACHABLE prelude helper is absent from BOTH the build output and the deep-
//!     check output — deep check prunes exactly like build;
//!   * a REACHABLE prelude helper is present in deep check — pruning never over-reaches
//!     (an over-prune would be a LOUD undefined reference, a far worse finding);
//!   * a program with no prelude is untouched — pruning is a no-op there.

use logicaffeine_compile::compile::{compile_program_full, rustc_check_artifacts};

/// `upper` reaches no string-search code, so `twoWayIndexBytesFrom` (the two-way
/// Crochemore-Perrin core, deep inside `strings.lg`) is dead for this program.
const USES_UPPER: &str = "## Main\nShow upper(\"hi\").\n";

/// `indexOf` transitively calls `twoWayIndexBytesFrom`, so it is REACHABLE here.
const USES_INDEXOF: &str = "## Main\nShow indexOf(\"hello\", \"ll\").\n";

#[test]
fn deep_check_prunes_the_unreachable_prelude_like_build() {
    let check = rustc_check_artifacts(USES_UPPER).expect("upper program lowers for check");
    let build = compile_program_full(USES_UPPER)
        .expect("upper program lowers for build")
        .rust_code;

    // Both keep the reachable function…
    assert!(
        check.rust_code.contains("fn upper"),
        "deep check keeps the reachable `upper`"
    );
    assert!(
        build.contains("fn upper"),
        "build keeps the reachable `upper`"
    );

    // …and BOTH drop the unreachable two-way-search core. Before this fix, deep check
    // spliced the whole `strings.lg` and type-checked `twoWayIndexBytesFrom` — code the
    // built binary never contains — which is exactly how an unreachable-stdlib
    // imperfection leaked out as a deep-check finding.
    assert!(
        !build.contains("twoWayIndexBytesFrom"),
        "build prunes the unreachable two-way search"
    );
    assert!(
        !check.rust_code.contains("twoWayIndexBytesFrom"),
        "deep check must prune the unreachable two-way search EXACTLY like build — \
         otherwise it checks code the binary never ships"
    );
}

#[test]
fn deep_check_never_over_prunes_a_reachable_helper() {
    // The safety direction: an over-prune drops a helper the program DOES reach, turning
    // a valid program into a phantom "cannot find function" finding. A program that
    // reaches the two-way search must still carry it into the deep-check unit.
    let check = rustc_check_artifacts(USES_INDEXOF).expect("indexOf program lowers for check");
    assert!(
        check.rust_code.contains("fn indexOf"),
        "the called function is present"
    );
    assert!(
        check.rust_code.contains("twoWayIndexBytesFrom"),
        "a REACHABLE prelude helper must survive — pruning drops only dead code"
    );
}

#[test]
fn a_program_with_no_prelude_is_untouched_by_the_prune() {
    // No stdlib name is referenced, so there is no prelude to prune: deep check lowers
    // the user's own code exactly, and the mapping stays 1:1 (pruning is a no-op).
    let src = "## Main\nLet x be 2 plus 3.\nShow x.\n";
    let check = rustc_check_artifacts(src).expect("plain program lowers for check");
    assert!(
        check.rust_code.contains("show("),
        "the user's Show is emitted"
    );
    // None of the string helpers are dragged in.
    assert!(
        !check.rust_code.contains("fn upper"),
        "no unreferenced prelude leaks in"
    );
    assert!(
        !check.rust_code.contains("twoWayIndexBytesFrom"),
        "no unreferenced prelude leaks in"
    );
}
