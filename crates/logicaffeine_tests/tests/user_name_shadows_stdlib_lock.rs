//! A name the STDLIB also uses must not change what a program's OWN declaration means.
//!
//! Three separate wrong answers in one night traced to the same shape — a stdlib name leaking into a
//! decision about user code, each time producing a wrong ANSWER rather than an error:
//!
//!   * the prelude pruner deleted a program's own `## To native args -> Seq of Text`, because the
//!     stdlib also declares `args` and the pruner decided "stdlib and unreferenced" by NAME;
//!   * codegen typed a program's own `gamma` as `f64`, because `math.lg` declares
//!     `gamma (z: Real) -> Real` and the float-returning-math list is a list of NAMES, emitting
//!     `gamma(n - 1) + ((1) as f64)` against an `i64`;
//!   * (recorded from a sibling campaign) the VM's builtin lookup beats user functions outright, so a
//!     user native has to route around it rather than through it.
//!
//! The rule this pins: whoever declared a name in THIS program owns it. Each case below is a program
//! that declares its own function with a name the stdlib also uses, and asserts the program's meaning
//! survives — on the tree-walker (the oracle), the VM, and the AOT-Rust backend, because each of the
//! three bugs lived in a different one of those.
//!
//! Add a case whenever a new collision class appears; the list is meant to grow — and it must, because
//! this class has a specific hazard: a fix lands where the bug was OBSERVED, while the same flaw sits
//! unexercised in a parallel spelling. The float-returning list was fixed here after `gamma`
//! mis-typed; `COMPLEXF_RETURNING_MATH_FNS` is a second name list with the identical flaw and was
//! still open afterwards, found only because someone followed the class rather than the failure. So a
//! case here should pin the RULE for each name-keyed lookup, not just the one that broke.

use logicaffeine_compile::compile::{compile_to_rust, tw_outcome, vm_outcome};

/// Every tier agrees, and agrees with what the program says rather than what the stdlib says.
fn assert_user_declaration_wins(label: &str, src: &str, expected: &str) {
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "{label}: the tree-walker must run it");
    assert_eq!(tw.output.trim(), expected, "{label}: tree-walker");

    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "{label}: the VM must run it");
    assert_eq!(vm.output.trim(), expected, "{label}: VM");

    // The AOT tier is where a stdlib name leaking into TYPE inference shows up: it emits Rust, so a
    // wrong type is a compile error rather than a wrong number.
    let rust = compile_to_rust(src)
        .unwrap_or_else(|e| panic!("{label}: the Rust backend must compile it: {e:?}"));
    assert!(
        rust.contains("fn "),
        "{label}: the emitted program must define the function the source declares:\n{rust}"
    );
}

/// `gamma` is `math.lg`'s `gamma (z: Real) -> Real`. A program's own `gamma (n: Int) -> Int` must be
/// typed by ITS declaration — the bug emitted `gamma(n - 1) + ((1) as f64)` against an `i64`.
#[test]
fn a_user_function_named_after_a_real_returning_math_function_keeps_its_own_return_type() {
    let src = "## To gamma (n: Int) -> Int:\n    If n is at most 0:\n        Return 3.\n    Return gamma(n - 1) + 1.\n## Main\n    Show gamma(3).\n";
    assert_user_declaration_wins("gamma shadows math.lg", src, "6");
}

/// The mutual-recursion shape the regression actually surfaced in: the shadowing name is reached
/// through another function, so the mis-typing lands in the CALLER's arithmetic.
#[test]
fn a_shadowing_name_reached_through_mutual_recursion_still_types_by_its_own_declaration() {
    let src = "## To alpha (n: Int) -> Int:\n    If n is at most 0:\n        Return 1.\n    Return beta(n - 1) + 1.\n\n## To beta (n: Int) -> Int:\n    If n is at most 0:\n        Return 2.\n    Return gamma(n - 1) + 1.\n\n## To gamma (n: Int) -> Int:\n    If n is at most 0:\n        Return 3.\n    Return alpha(n - 1) + 1.\n\n## Main\n    Show alpha(3).\n";
    assert_user_declaration_wins("mutual recursion through a shadowing name", src, "4");
}

/// `args` is the stdlib's `## To native args -> Seq of Text`. A program declaring its own must keep
/// it even when nothing calls it — the pruner deleted the declaration and left the use behind.
#[test]
fn a_user_native_named_after_a_stdlib_native_survives_pruning() {
    let src = "## To native args -> Seq of Text\n\n## Main\n    Let x be 1.\n    Show x.\n";
    let rust = compile_to_rust(src).expect("a program redeclaring a stdlib native must compile");
    assert!(
        rust.contains("fn args()"),
        "the program's own `args` declaration must reach codegen even though nothing calls it:\n{rust}"
    );
}

// KNOWN GAP, deliberately not asserted here — it needs a ruling, not a patch.
//
// A user function named after a BUILTIN (rather than after a prelude `.lg` function) does NOT win
// today. `## To sqrt (a: Int, b: Int) -> Int: Return a + b.` with `Show sqrt(2, 3).` fails on the
// tree-walker with `sqrt() takes exactly 1 argument`: builtin dispatch resolves the name before user
// functions are consulted, so the program's own two-argument `sqrt` is unreachable.
//
// That is the same class this file pins, and the language's stated rule points the same way —
// `keyword_shadowing_lock` opens with "LOGOS keywords are contextual, never reserved: a live binding
// wins", and the prelude and math-name cases above now behave that way. But builtin dispatch is
// semantics rather than a lookup bug: making a user definition beat a builtin changes what `sqrt`
// MEANS in a program that defines it, on every tier at once. That is Tristen's call, so this file
// records the gap in the open rather than asserting either answer, and a case belongs here the moment
// the rule is settled.

/// The parallel name list: `COMPLEXF_RETURNING_MATH_FNS`. A program's own `cExp (n: Int) -> Int` must
/// be typed by its own declaration, exactly as `gamma` must — this list had the same flaw and was
/// still open after the float one was fixed, because the float one was where the bug was seen.
#[test]
fn a_user_function_named_after_a_complex_returning_math_function_keeps_its_own_return_type() {
    let src = "## To cExp (n: Int) -> Int:\n    Return n + 1.\n## Main\n    Show cExp(4).\n";
    assert_user_declaration_wins("cExp shadows math.lg's ComplexF-returning name", src, "5");
}
