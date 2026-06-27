//! Mixed-document linking: imperative LOGOS code uses compiled math/logic objects.
//!
//! One source may contain `## To`/`## Main` imperative code AND `Definition`/
//! `Inductive`/`## Theorem:` math; the compiler routes each block to the right
//! backend, extracts the math/logic into a named `mod proven`, and bundles it into
//! the imperative output so a bare `double(21)` resolves into it.
//!
//! Phase 0 locks the bundling MECHANISM (pure-string codegen shape). The
//! compile-AND-run e2e tests arrive with the mixed router (Phase 2).

use logicaffeine_compile::{
    compile_to_rust, compile_to_rust_with_proven, extract_logic_module,
    extract_math_module_from_source, partition_mixed,
};

const PROVEN: &str = "pub fn double(x: i64) -> i64 { x + x }\n";

#[test]
fn proven_module_is_injected() {
    let rust = compile_to_rust_with_proven("## Main\nShow 1.", PROVEN).expect("compiles");
    assert!(rust.contains("pub mod proven {"), "missing proven module:\n{rust}");
    assert!(rust.contains("use proven::*;"), "missing glob re-export:\n{rust}");
    assert!(
        rust.contains("pub fn double(x: i64) -> i64"),
        "proven fn not bundled:\n{rust}"
    );
}

#[test]
fn proven_module_precedes_main() {
    let rust = compile_to_rust_with_proven("## Main\nShow 1.", PROVEN).expect("compiles");
    let mp = rust.find("mod proven").expect("has proven module");
    let mu = rust
        .find("fn main")
        .or_else(|| rust.find("_logos_main"))
        .expect("has main");
    assert!(mp < mu, "proven module must precede main:\n{rust}");
}

#[test]
fn empty_proven_is_a_no_op() {
    // A blank proven module must not be emitted — byte-identical to the bare compile.
    let with_blank = compile_to_rust_with_proven("## Main\nShow 1.", "   \n").expect("compiles");
    let bare = compile_to_rust("## Main\nShow 1.").expect("compiles");
    assert_eq!(with_blank, bare, "blank proven module should be a no-op");
}

// --- Phase 1: main-less extraction variants ----------------------------------

#[test]
fn math_module_has_pub_defs_but_no_main() {
    // `add` is the kernel's `Int -> Int -> Int` builtin (surface `+` is not kernel
    // vernacular); extraction maps it to the Rust `+` operator.
    let m = extract_math_module_from_source("Definition double : Int -> Int := fun n : Int => add n n.");
    assert!(m.contains("pub fn double"), "double extracted as pub fn:\n{m}");
    assert!(m.contains(" + "), "add builtin → Rust `+` operator:\n{m}");
    assert!(!m.contains("fn main"), "module must NOT carry a demo main:\n{m}");
}

#[test]
fn logic_module_has_pub_holds_but_no_main() {
    let m = extract_logic_module("Every dog runs.").expect("compiles");
    assert!(m.contains("pub fn holds"), "holds present and public:\n{m}");
    assert!(!m.contains("fn main"), "module must NOT carry a demo main:\n{m}");
}

// --- Phase 2: mixed-document router ------------------------------------------

/// One document: a proven `double` defined in math, called from imperative `## Main`.
const MIXED_HERO: &str = "\
Definition double : Int -> Int := fun n : Int => add n n.

## Main
Show double(21).
";

#[test]
fn mixed_source_partitions_math_and_imperative() {
    let (imp, math) = partition_mixed(MIXED_HERO);
    let math = math.expect("math stream present");
    assert!(math.contains("Definition double"), "math stream has the definition:\n{math}");
    assert!(!imp.contains("Definition double"), "imperative stream must not keep the math:\n{imp}");
    assert!(imp.contains("Show double(21)"), "imperative stream keeps the call:\n{imp}");
}

#[test]
fn pure_imperative_partition_is_noop() {
    let src = "## Main\nLet x be 5.\nShow x.";
    let (imp, math) = partition_mixed(src);
    assert_eq!(imp, src, "pure imperative source must be returned unchanged");
    assert!(math.is_none(), "no math stream for a pure imperative program");
}

#[test]
fn mixed_compile_bundles_proven_and_wires_the_call() {
    // The mechanism (no cargo): the proven module is bundled and the imperative call
    // is emitted to resolve into it. The compile-AND-run proof is the #[ignore] test.
    let rust = compile_to_rust(MIXED_HERO).expect("mixed source compiles");
    assert!(rust.contains("pub mod proven {"), "proven module bundled:\n{rust}");
    assert!(rust.contains("pub fn double"), "proven double bundled:\n{rust}");
    assert!(rust.contains("use proven::*;"), "proven items in scope:\n{rust}");
    assert!(rust.contains("double("), "imperative call to double emitted:\n{rust}");
}

// --- Phase 5: vacuous proofs → honest note, construction still extracts -------

#[test]
fn vacuous_theorem_noted_but_construction_extracted() {
    // `triv : True` is a proof of a proposition — proof-irrelevant, no runnable form.
    // The constructive `double` still extracts; `triv` gets an honest note.
    let src = "Definition double : Int -> Int := fun n : Int => add n n.\n\
               Definition triv : True := I.";
    let m = extract_math_module_from_source(src);
    assert!(m.contains("pub fn double"), "constructive def still extracts:\n{m}");
    assert!(m.contains("// note:") && m.contains("triv"), "vacuous theorem gets a note:\n{m}");
    assert!(m.contains("proof-irrelevant"), "note explains why:\n{m}");
}

// --- Phase 6: enum type-unification (construct/pass/show proven enum values) --

/// Imperative code builds proven `MyNat` values by name, passes them to a proven
/// function, and displays the result — the full enum bridge.
const MIXED_ENUM: &str = "\
Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.
Definition my_add : MyNat -> MyNat -> MyNat := fix rec => fun n : MyNat => fun m : MyNat => match n return (fun _ : MyNat => MyNat) with | MZero => m | MSucc k => MSucc (rec k m).

## Main
Let x be MSucc(MSucc(MZero)).
Let y be MSucc(MSucc(MSucc(MZero))).
Let sum be my_add(x, y).
Show sum.
";

#[test]
fn proven_enum_has_constructor_api_and_display() {
    let m = extract_math_module_from_source(
        "Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.",
    );
    assert!(m.contains("pub enum MyNat"), "enum emitted:\n{m}");
    assert!(m.contains("pub const MZero: MyNat"), "nullary ctor → const:\n{m}");
    assert!(
        m.contains("pub fn MSucc(") && m.contains("Box::new("),
        "recursive ctor → boxed wrapper fn:\n{m}"
    );
    assert!(
        m.contains("impl std::fmt::Display for MyNat"),
        "Display impl so `Show` works on proven values:\n{m}"
    );
}

#[test]
fn mixed_enum_construction_wires_through() {
    let rust = compile_to_rust(MIXED_ENUM).expect("mixed enum doc compiles");
    assert!(rust.contains("pub fn MSucc("), "ctor wrapper bundled:\n{rust}");
    assert!(rust.contains("pub fn my_add"), "proven fn bundled:\n{rust}");
    // imperative code calls the bundled constructors + function by name
    assert!(rust.contains("MSucc("), "imperative builds proven value:\n{rust}");
    assert!(rust.contains("my_add("), "imperative calls proven fn:\n{rust}");
    assert!(
        rust.contains("impl Showable for MyNat"),
        "proven enum bridged to the Show verb:\n{rust}"
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[ignore = "compiles + runs a full Cargo project (slow); run in the full suite"]
#[test]
fn e2e_imperative_builds_and_uses_proven_enum() {
    use logicaffeine_compile::compile::compile_and_run;
    let dir = std::env::temp_dir().join("logos_mixed_enum_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    let out = compile_and_run(MIXED_ENUM, &dir).expect("mixed enum doc compiles and runs");
    // my_add(2, 3) = 5, shown via the Display(=Debug) impl on the proven enum.
    assert_eq!(
        out.trim(),
        "MSucc(MSucc(MSucc(MSucc(MSucc(MZero)))))",
        "imperative-built proven enum, summed by proven fn, displayed; got: {out:?}"
    );
}

// --- Phase 3: proven invariants ---------------------------------------------

#[test]
fn require_statement_emits_hard_assert() {
    // `Require that` is an ENFORCED invariant — a hard `assert!` that survives release
    // (unlike `Assert that` → `debug_assert!`, stripped in release).
    let rust = compile_to_rust("## Main\nLet x be 5.\nRequire that x is greater than 0.").expect("compiles");
    assert!(rust.contains("assert!("), "Require → assert!:\n{rust}");
    assert!(!rust.contains("debug_assert!("), "Require must NOT be debug_assert:\n{rust}");
}

#[test]
fn assert_statement_stays_debug_assert() {
    let rust = compile_to_rust("## Main\nLet x be 5.\nAssert that x is greater than 0.").expect("compiles");
    assert!(rust.contains("debug_assert!("), "Assert stays debug_assert:\n{rust}");
}

/// A proven theorem becomes a runtime contract: `Require that check_<thm>()` calls
/// the bundled closed property check and enforces it at runtime.
const MIXED_INVARIANT: &str = "\
Inductive MyNat := MZero : MyNat | MSucc : MyNat -> MyNat.
Definition my_add : MyNat -> MyNat -> MyNat := fix rec => fun n : MyNat => fun m : MyNat => match n return (fun _ : MyNat => MyNat) with | MZero => m | MSucc k => MSucc (rec k m).
Definition add_zero : Eq MyNat (my_add MZero MZero) MZero := refl MyNat MZero.

## Main
Require that check_add_zero().
Show \"invariant holds\".
";

#[test]
fn run_tier_enforces_require() {
    // The Studio's ▶ Run uses the interpreter/VM, not AOT — a `Require` must be
    // enforced there too, or a proven invariant would silently pass when run.
    use logicaffeine_compile::compile::interpret_program;
    assert!(
        interpret_program("## Main\nLet x be 5.\nRequire that x is greater than 0.\nShow \"ok\".").is_ok(),
        "a satisfied Require runs fine"
    );
    assert!(
        interpret_program("## Main\nLet x be 5.\nRequire that x is greater than 9.").is_err(),
        "a violated Require must fail when run (interpreter/VM)"
    );
}

#[test]
fn mixed_invariant_bundles_check_and_requires_it() {
    let rust = compile_to_rust(MIXED_INVARIANT).expect("mixed invariant compiles");
    assert!(rust.contains("pub fn check_add_zero"), "closed check bundled:\n{rust}");
    assert!(rust.contains("assert!("), "Require enforces it as a hard assert:\n{rust}");
    assert!(rust.contains("check_add_zero("), "Require calls the proven check:\n{rust}");
}

#[cfg(not(target_arch = "wasm32"))]
#[ignore = "compiles + runs a full Cargo project (slow); run in the full suite"]
#[test]
fn e2e_proven_invariant_holds_at_runtime() {
    use logicaffeine_compile::compile::compile_and_run;
    let dir = std::env::temp_dir().join("logos_mixed_invariant_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    let out = compile_and_run(MIXED_INVARIANT, &dir).expect("proven invariant compiles and runs");
    assert_eq!(out.trim(), "invariant holds", "passing invariant runs; got: {out:?}");
}

#[cfg(not(target_arch = "wasm32"))]
#[ignore = "compiles + runs a full Cargo project (slow); run in the full suite"]
#[test]
fn e2e_violated_require_panics() {
    use logicaffeine_compile::compile::{compile_and_run, CompileError};
    let dir = std::env::temp_dir().join("logos_require_violation_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    let src = "## Main\nLet x be 1.\nRequire that x is greater than 2.\nShow \"unreachable\".";
    match compile_and_run(src, &dir) {
        Err(CompileError::Runtime(_)) => {} // the hard assert fired
        other => panic!("a violated Require must panic at runtime; got: {other:?}"),
    }
}

// --- Phase 3b: function Requires/Ensures contracts ---------------------------

#[test]
fn requires_clause_checks_at_entry() {
    let src = "## To clamp (x: Int) -> Int:\n    Requires x is greater than 0.\n    Return x.\n## Main\nShow clamp(5).";
    let rust = compile_to_rust(src).expect("compiles");
    assert!(rust.contains("fn clamp"), "function emitted:\n{rust}");
    assert!(rust.contains("assert!("), "precondition → hard assert!:\n{rust}");
}

#[test]
fn ensures_clause_checks_every_return_path() {
    // Two exits: an early return inside If, and the fallthrough return — the
    // postcondition must guard BOTH (no silent corner on early returns).
    let src = "## To pick (x: Int) -> Int:\n    Ensures x is greater than 0.\n    If x is greater than 10:\n        Return x.\n    Return x.\n## Main\nShow pick(5).";
    let rust = compile_to_rust(src).expect("compiles");
    let n = rust.matches("assert!(").count();
    assert!(n >= 2, "postcondition must guard BOTH exits (>=2 assert!); found {n}:\n{rust}");
}

#[cfg(not(target_arch = "wasm32"))]
#[ignore = "compiles + runs a full Cargo project (slow); run in the full suite"]
#[test]
fn e2e_violated_precondition_panics() {
    use logicaffeine_compile::compile::{compile_and_run, CompileError};
    let dir = std::env::temp_dir().join("logos_precond_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    let src = "## To pos (x: Int) -> Int:\n    Requires x is greater than 0.\n    Return x.\n## Main\nShow pos(0).";
    match compile_and_run(src, &dir) {
        Err(CompileError::Runtime(_)) => {} // entry precondition fired
        other => panic!("a violated precondition must panic; got: {other:?}"),
    }
}

/// The real bar: compile the mixed document to a Cargo project and RUN it. Heavy
/// (builds the imperative runtime), so #[ignore] in the quick loop — the fast runner
/// (`run-all-tests-fast.sh`) executes #[ignore]d tests, so it is still gated in CI.
#[cfg(not(target_arch = "wasm32"))]
#[ignore = "compiles + runs a full Cargo project (slow); run in the full suite"]
#[test]
fn e2e_mixed_imperative_calls_proven_double_prints_42() {
    use logicaffeine_compile::compile::compile_and_run;
    let dir = std::env::temp_dir().join("logos_mixed_hero_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    let out = compile_and_run(MIXED_HERO, &dir).expect("mixed document compiles and runs");
    assert_eq!(out.trim(), "42", "proven double(21) should print 42; got: {out:?}");
}
