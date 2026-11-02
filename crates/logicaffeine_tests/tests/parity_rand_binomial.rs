//! Stdlib-parity — `randBinomial` (Python random.binomialvariate): deterministic success count in
//! n Bernoulli trials. Seed 42 CPython-verified; p=0/1 edges. The loop-carried mutable LInt `state`
//! (bignum, because the LCG's `state*2654435761` overflows i32) converts to `double` in `state*1.0`
//! via the codegen_c BigInt×Float64 → `lint_to_f64` coercion. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

const SRC: &str = "## Main\n\
    Show randBinomial(42, 10, 0.5).\n\
    Show randBinomial(42, 10, 0.0).\n\
    Show randBinomial(42, 5, 1.0).\n";
const EXPECTED: &str = "6\n0\n5";

/// Successes in n Bernoulli trials. py: random.binomialvariate
#[test]
fn parity_rand_binomial() {
    assert_all_tiers(SRC, EXPECTED);
    assert_compiled_equals_interpreted(SRC);
    assert_c_output(SRC, EXPECTED);
}
