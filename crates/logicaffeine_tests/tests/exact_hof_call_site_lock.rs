//! ★ EXACT-HOF CALL-SITE LOCK — the selective higher-order exactification
//! (`codegen/bigint_promote::exact_hof_fns`) is refined per call site, and every
//! branch of that refinement must keep the Rust tier == the VM:
//!
//!   - a site whose arguments are PROVABLY within `i64` keeps the plain `i64`
//!     signatures (exact already, no bignum tax) — the shape `phase24_codegen`
//!     pins textually is also semantically right;
//!   - a NAMED closure (a `Let`-bound lambda passed by name, possibly also
//!     called directly) must compile and run — its `Let` emitted the `i64`
//!     form, so the HOF must not demand `LogosInt` from it;
//!   - an UNPROVABLE site (a runtime-computed argument) with inline closures
//!     stays exactified, so an overflow promotes to a bignum == the VM (the
//!     literal-argument twin lives in `higher_order_bignum_lock.rs`).
#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{compile_to_program, program_outcome, run_logos_source};

const APPLY_HOF: &str =
    "## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:\n    Return f(x).\n\n";

fn vm_output(src: &str) -> String {
    let vm = program_outcome(&compile_to_program(src).expect("VM compiles"));
    vm.output.trim().to_string()
}

fn rust_matches_vm(src: &str, want: &str) {
    assert_eq!(
        vm_output(src),
        want,
        "VM oracle mismatch — fixture expectation is stale"
    );
    let got = run_logos_source(src).expect("Rust tier compiles and runs");
    assert_eq!(got.trim(), want, "Rust tier != VM");
}

#[test]
fn a_provably_bounded_inline_site_keeps_plain_i64_signatures_and_the_exact_result() {
    let src = format!("{APPLY_HOF}## Main\nShow apply((n: Int) -> n * 2, 5).\n");
    let rust = logicaffeine_compile::compile_to_rust(&src).expect("compiles");
    assert!(
        rust.contains("impl Fn(i64) -> i64"),
        "a bounded site must not pay the bignum tax:\n{rust}"
    );
    rust_matches_vm(&src, "10");
}

#[test]
fn a_named_closure_flows_through_a_scalar_hof() {
    let src =
        format!("{APPLY_HOF}## Main\nLet square be (n: Int) -> n * n.\nShow apply(square, 6).\n");
    rust_matches_vm(&src, "36");
}

#[test]
fn mixed_named_and_inline_sites_both_compile_and_run() {
    let src = format!(
        "{APPLY_HOF}## Main\nLet square be (n: Int) -> n * n.\nShow apply(square, 6).\nShow apply((n: Int) -> n + 1, 41).\n"
    );
    rust_matches_vm(&src, "36\n42");
}

#[test]
fn a_named_closure_passed_to_a_hof_stays_directly_callable() {
    let src = format!(
        "{APPLY_HOF}## Main\nLet triple be (n: Int) -> n * 3.\nShow triple(5).\nShow apply(triple, 7).\n"
    );
    rust_matches_vm(&src, "15\n21");
}

#[test]
fn an_unprovable_runtime_argument_still_promotes_through_the_inline_closure() {
    // The argument is a variable, not a literal — the boundedness proof must refuse,
    // the HOF stays exactified, and the doubled i64::MAX promotes to a bignum == the VM.
    let src = format!(
        "{APPLY_HOF}## Main\nLet big be 9223372036854775807.\nShow apply((n: Int) -> n * 2, big).\n"
    );
    rust_matches_vm(&src, "18446744073709551614");
}
