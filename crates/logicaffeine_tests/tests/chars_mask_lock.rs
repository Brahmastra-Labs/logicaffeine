//! Lock for `Opt::AsciiClassMask` — the ASCII-classification bitmask pass.
//!
//! A call to a PURE `Int -> Bool` predicate whose true-set lies entirely in the byte domain
//! `[0, 256)` (the `chars.lg` `charIs*` family — range/equality checks over `c`) lowers to a
//! branchless 256-bit mask test `(0<=c<256) && (MASK[(c>>6)&3] >> (c&63)) & 1 != 0`, built by
//! CTFE-evaluating the predicate over `0..256` at compile time. This beats C's `ctype` table
//! (the mask is an immediate/static — no runtime memory load) and is fully general: any user
//! predicate over a small integer domain collapses the same way. Gates off under
//! `## No asciiclassmask`; byte-identical across all tiers (incl. out-of-[0,256) inputs).

use logicaffeine_compile::compile::{
    compile_to_program, compile_to_rust, program_outcome, run_logos_source,
};
use logicaffeine_compile::optimization::decorate_source;

fn vm_output(src: &str) -> String {
    program_outcome(&compile_to_program(src).expect("VM compiles"))
        .output
        .trim()
        .to_string()
}
fn aot_output(src: &str) -> String {
    run_logos_source(src)
        .expect("AOT compiles and runs")
        .trim()
        .to_string()
}

/// Counts digits over a domain that includes OUT-OF-RANGE bytes (`i % 300` hits values ≥256, where
/// `charIsDigit` must still be false) — pins the soundness of the `(0<=c<256)` guard, not just the
/// in-range mask. The loop bound (3000) is large enough that comptime won't unroll it, so `i` stays
/// a runtime value and the `charIsDigit(i % 300)` call survives to codegen. Digits per 300-cycle:
/// c in 48..=57 → 10; 3000/300 = 10 cycles → 100.
const PROG: &str = "\
## Main
Let mutable s be 0.
Let mutable i be 0.
While i is less than 3000:
    If charIsDigit(i % 300):
        Set s to s + 1.
    Set i to i + 1.
Show s.
";

#[test]
fn ascii_class_mask_lowers_to_flat_table() {
    let rust = compile_to_rust(PROG).expect("PROG compiles");
    // The predicate BODY must become a flat 256-byte table lookup (`__T[__cc as usize]`) — the
    // technique that beats C's ctype LUT, not the slower compact bitmask. (The call itself stays —
    // LLVM inlines the tiny fn.)
    assert!(
        rust.contains("[u8; 256]") && rust.contains("__T[__cc as usize]"),
        "the charIsDigit predicate must lower to a flat 256-byte table lookup; got:\n{rust}"
    );
}

#[test]
fn ascii_class_mask_gates_off() {
    let off = decorate_source(PROG, &["asciiclassmask"]);
    let rust = compile_to_rust(&off).expect("disabled PROG compiles");
    assert!(
        !rust.contains("__cc"),
        "`## No asciiclassmask` must keep the comparison-chain predicate (no mask); got:\n{rust}"
    );
}

/// PIT-OF-SUCCESS correctness lock: the mask must be BYTE-IDENTICAL to the VM oracle (which runs
/// the real predicate) AND to the AOT path with the opt disabled — over a domain that includes
/// out-of-[0,256) inputs (i%300), catching any guard/edge error (negative, ≥256, boundary 47/48/57/58).
#[test]
fn ascii_class_mask_is_byte_identical_all_tiers_and_toggle() {
    let vm = vm_output(PROG);
    let on = aot_output(PROG);
    let off = aot_output(&decorate_source(PROG, &["asciiclassmask"]));
    assert_eq!(
        on, vm,
        "AOT mask diverged from the VM oracle (guard/boundary bug?)"
    );
    assert_eq!(
        on, off,
        "AsciiClassMask ON changed output vs OFF — not behavior-preserving"
    );
}
