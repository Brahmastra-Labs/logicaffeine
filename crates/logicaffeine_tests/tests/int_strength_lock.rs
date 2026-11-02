//! Integer induction-variable strength reduction (IntStrength) lock.
//!
//! IntStrength turns a counted loop's affine index `coeff*j + base` (base
//! loop-invariant) into a seeded accumulator advanced by `coeff*step`. Because
//! integer arithmetic is exact, the accumulator equals the closed-form index
//! bit-for-bit at every iteration — so the transform is unconditionally
//! semantics-preserving. These locks pin:
//!
//!   1. FIRING — it fires on matmul's inner-loop indices (`i*n+j+1`,
//!      `k*n+j+1`) on the run-path pipeline, and gates off under
//!      `## No intstrength`.
//!   2. CORRECTNESS — matmul's modular checksum (an unforgiving integer)
//!      is identical across tree-walker and VM, and with the optimization on
//!      vs off, over a spread of matrix sizes.
//!   3. NEGATIVE — a counted loop with no affine index does not fire it.

use logicaffeine_compile::compile::{
    compile_to_rust, interpret_program, optimizations_fired_run, vm_run_source,
};
use logicaffeine_compile::optimization::decorate_source;

const MATMUL: &str = include_str!("../../../benchmarks/programs/matrix_mult/main.lg");

/// The benchmark with a FIXED size, so it runs without program args — the
/// optimizing `interpret_program` cannot pass args. The native `args`/`parseInt`
/// declarations are left in (harmless, unused).
fn matmul_fixed(n: i64) -> String {
    MATMUL.replace("Let arguments be args().\n", "").replace(
        "Let n be parseInt(item 2 of arguments).",
        &format!("Let n be {n}."),
    )
}

// `interpret_program` runs the OPTIMIZING VM (the run-path optimizer, which
// includes IntStrength); `vm_run_source` runs the RAW VM (no optimizer). Their
// agreement is the opt-vs-raw execution differential — a miscompiled
// strength-reduction changes `opt` and is caught.
fn opt(src: &str) -> String {
    interpret_program(src)
        .unwrap_or_else(|e| panic!("optimized run failed: {e:?}"))
        .trim_end()
        .to_string()
}
fn raw(src: &str) -> String {
    vm_run_source(src)
        .unwrap_or_else(|e| panic!("raw run failed: {e}"))
        .trim_end()
        .to_string()
}

/// IntStrength fires on the matmul inner loop's affine indices on the run-path
/// pipeline (where it runs before the loop is restructured), and gates off under
/// `## No intstrength`.
#[test]
fn intstrength_fires_and_gates() {
    let run = optimizations_fired_run(MATMUL);
    assert!(
        run.contains(&"intstrength"),
        "run-path must fire intstrength on matmul; got {run:?}"
    );
    let off = optimizations_fired_run(&decorate_source(MATMUL, &["intstrength"]));
    assert!(
        !off.contains(&"intstrength"),
        "`## No intstrength` must remove it; got {off:?}"
    );
}

/// Correctness core: the strength-reduced accumulator equals the closed-form
/// index bit-for-bit, so matmul's modular checksum from the OPTIMIZED run
/// (`interpret_program`, IntStrength applied) equals the RAW run
/// (`vm_run_source`) and the SCCP-off optimized run — over a spread of matrix
/// sizes large enough that the inner loop is not unrolled away.
#[test]
fn intstrength_preserves_matmul_checksum() {
    for n in [4, 7, 17, 20, 32] {
        let src = matmul_fixed(n);
        let off = decorate_source(&src, &["intstrength"]);
        let optimized = opt(&src);
        let rawv = raw(&src);
        let off_optimized = opt(&off);
        assert_eq!(
            optimized, rawv,
            "n={n}: optimized (IntStrength) diverged from raw (miscompile!)"
        );
        assert_eq!(
            optimized, off_optimized,
            "n={n}: IntStrength changed the checksum (unsound!)"
        );
    }
}

/// A counted loop with no affine index must NOT fire intstrength — there is
/// nothing to strength-reduce beyond the bare counter.
#[test]
fn intstrength_noop_without_affine_index() {
    let src = "## Main\nLet n be 20.\nLet mutable sum be 0.\nLet i be 0.\nWhile i is less than n:\n    Set sum to sum + 1.\n    Set i to i + 1.\nShow sum.\n";
    let fired = optimizations_fired_run(src);
    assert!(
        !fired.contains(&"intstrength"),
        "must not fire without an affine index; got {fired:?}"
    );
    assert_eq!(opt(src), "20");
    assert_eq!(raw(src), "20");
}

/// Matrix multiplication filled by the affine index `(i*n + j) % 100`, as `matrix_mult` stood when this gate was
/// written (`fixtures/opt_witness`): a site the compiled path's IntStrength takes.
const AFFINE_MATMUL: &str = include_str!("fixtures/opt_witness/matrix_mult.lg");

/// `## Require intstrength` compiles on matmul and errors where there is no site.
#[test]
fn intstrength_require_gate() {
    let required = AFFINE_MATMUL.replace("## Main", "## Require intstrength\n## Main");
    assert!(
        compile_to_rust(&required).is_ok(),
        "`## Require intstrength` must compile on matmul"
    );
    let no_site = "## Require intstrength\n## Main\nShow 42.\n";
    assert!(
        compile_to_rust(no_site).is_err(),
        "`## Require intstrength` must error with no site"
    );
}
