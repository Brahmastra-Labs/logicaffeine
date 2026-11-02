//! closed_form_affine_lock — the closed-form pass collapses a GENERAL AFFINE accumulator.
//!
//! `optimize/closed_form.rs` collapsed `Σ i` (Gauss) and the Faulhaber powers. This extends the
//! family to `acc += a·i + b` → `a·Σi + b·count` (a real, general optimization: real code sums
//! affine sequences). The raw formula is exact because the codegen overflow guard promotes its
//! products to BigInt exactly as the loop would.
//!
//!   cargo nextest run -p logicaffeine-compile -E 'binary(closed_form_affine_lock)'
#![cfg(feature = "codegen")]

use logicaffeine_compile::compile::{
    compile_to_rust, interpret_program, optimizations_fired, vm_run_source,
};
use logicaffeine_compile::optimization::decorate_source;

/// `Σ_{i=1}^{100} (i + 7) = 5050 + 700 = 5750`.
const DIRECT: &str = "\
## Main
Let mutable s be 0.
Let mutable i be 1.
While i is at most 100:
    Set s to s + (i + 7).
    Set i to i + 1.
Show s.
";

/// Same, but through an `opAdd(i, 7)` call — exercises whether inlining exposes the affine form to
/// closedform (the stdlib operator benches use this shape).
const VIA_CALL: &str = "\
## To opAdd (a: Int) and (b: Int) -> Int:
    Return a + b.
## Main
Let mutable s be 0.
Let mutable i be 1.
While i is at most 100:
    Set s to s + opAdd(i, 7).
    Set i to i + 1.
Show s.
";

/// `Σ_{i=1}^{100} (7*i) = 7*5050 = 35350` — the a>1 case (products promote via the codegen guard).
const MUL: &str = "\
## Main
Let mutable s be 0.
Let mutable i be 1.
While i is at most 100:
    Set s to s + (i * 7).
    Set i to i + 1.
Show s.
";

fn no_unroll(src: &str) -> String {
    decorate_source(src, &["unroll"])
}
fn tw(src: &str) -> String {
    interpret_program(src)
        .unwrap_or_else(|e| panic!("tw: {e:?}"))
        .trim_end()
        .to_string()
}
fn vm(src: &str) -> String {
    vm_run_source(src)
        .unwrap_or_else(|e| panic!("vm: {e}"))
        .trim_end()
        .to_string()
}
fn rust(src: &str) -> String {
    compile_to_rust(src).unwrap_or_else(|e| panic!("compile_to_rust: {e:?}"))
}

#[test]
fn affine_direct_fires_and_is_loop_free() {
    let src = no_unroll(DIRECT);
    assert!(
        optimizations_fired(&src).contains(&"closedform"),
        "closedform must fire on Σ(i+7)"
    );
    let f = rust(&src);
    let m = f.find("fn _logos_main").expect("main");
    assert!(
        !f[m..].contains("while "),
        "the Σ(i+7) loop must be collapsed"
    );
}

#[test]
fn affine_semantics_exact() {
    assert_eq!(tw(DIRECT), "5750");
    assert_eq!(vm(DIRECT), "5750");
    assert_eq!(
        vm(&no_unroll(DIRECT)),
        "5750",
        "closedform must preserve Σ(i+7)"
    );
    assert_eq!(
        vm(&decorate_source(DIRECT, &["unroll", "closedform"])),
        "5750"
    );
    assert_eq!(vm(MUL), "35350");
    assert_eq!(
        vm(&no_unroll(MUL)),
        "35350",
        "closedform must preserve Σ(7i)"
    );
}

#[test]
fn affine_via_call_fires_and_is_exact() {
    // closedform reaches THROUGH the canonical opAdd call (body-verified) and collapses the loop.
    assert!(
        optimizations_fired(&no_unroll(VIA_CALL)).contains(&"closedform"),
        "closedform must see through opAdd"
    );
    assert_eq!(vm(VIA_CALL), "5750");
    assert_eq!(vm(&no_unroll(VIA_CALL)), "5750");
    let f = rust(&no_unroll(VIA_CALL));
    let m = f.find("fn _logos_main").expect("main");
    assert!(
        !f[m..].contains("while "),
        "the opAdd loop must be collapsed"
    );
}

/// SOUNDNESS: a user shadows opAdd to a constant — must NOT be reduced as `i + 7`.
#[test]
fn affine_op_shadow_safe() {
    let src = "\
## To opAdd (a: Int) and (b: Int) -> Int:
    Return 999.
## Main
Let mutable s be 0.
Let mutable i be 1.
While i is at most 100:
    Set s to s + opAdd(i, 7).
    Set i to i + 1.
Show s.
";
    // opAdd returns 999; Σ_{1}^{100} 999 = 99900, NOT Σ(i+7) = 5750.
    assert_eq!(tw(src), "99900", "tree-walker (shadowed opAdd)");
    assert_eq!(vm(src), "99900", "VM (shadowed opAdd)");
    assert_eq!(vm(&no_unroll(src)), "99900", "must not become 5750");
}
