//! Lock for `Opt::ByteSliceReturn` — the zero-copy byte-slice return pass (the bytes-trim family).
//!
//! The bytes analog of `TextSliceReturn`. A fn that takes a `Seq of Int` (byte) param, computes a
//! contiguous `lo..=hi` range by scans, then copies `item lo..hi of xs` into a fresh `out` Seq and
//! `Return out`, lowers to a borrowed `&'a [i64]` sub-slice (`fn f<'a>(xs: &'a [i64], …) -> &'a [i64]
//! { … &xs[lo-1..hi] }`) — Go's O(1) `bytes.Trim` reslice technique instead of the O(n) build-copy.
//! A readonly Seq param is ALREADY `&[i64]`, so the sub-slice shares the param's lifetime with no
//! RefCell hop inside the fn. Gates off under `## No byteslicereturn`; BAILS to the owned path when a
//! caller lets the result escape (bound/returned/stored) — a borrowed `&[i64]` must stay inline.

use logicaffeine_compile::compile::{
    compile_to_program, compile_to_rust, program_outcome, run_logos_source,
};
use logicaffeine_compile::optimization::decorate_source;

/// The VM tier runs the real (unchanged) trim fn — the correctness ORACLE.
fn vm_output(src: &str) -> String {
    program_outcome(&compile_to_program(src).expect("VM compiles"))
        .output
        .trim()
        .to_string()
}
/// The AOT Rust tier — where ByteSliceReturn actually fires.
fn aot_output(src: &str) -> String {
    run_logos_source(src)
        .expect("AOT compiles and runs")
        .trim()
        .to_string()
}

/// `bytesTrimByte` (the real stdlib shape): a `Seq of Int` param `xs`, scans `lo` up / `hi` down past
/// byte `c`, builds the `lo..=hi` slice into `out`, `Return out`. Called INLINE by `length of` (the
/// result never escapes) so the caller-gate keeps the borrowed return. Runtime-built `xs`, no args.
const TRIM: &str = "\
## To trimByte (xs: Seq of Int) and (c: Int) -> Seq of Int:
    Let n be length of xs.
    Let mutable lo be 1.
    Let mutable go be true.
    While go is equal to true:
        If lo is greater than n:
            Set go to false.
        Otherwise:
            If item lo of xs is equal to c:
                Set lo to lo + 1.
            Otherwise:
                Set go to false.
    Let mutable hi be n.
    Set go to true.
    While go is equal to true:
        If hi is less than lo:
            Set go to false.
        Otherwise:
            If item hi of xs is equal to c:
                Set hi to hi - 1.
            Otherwise:
                Set go to false.
    Let out be a new Seq of Int.
    Let mutable i be lo.
    While i is at most hi:
        Push item i of xs to out.
        Set i to i + 1.
    Return out.

## Main
Let mutable xs be a new Seq of Int.
Let mutable k be 0.
While k is less than 64:
    Push (k % 3) to xs.
    Set k to k + 1.
Let mutable checksum be 0.
Let mutable r be 0.
While r is less than 40:
    Set item ((r % 64) + 1) of xs to r % 3.
    Set checksum to checksum + length of trimByte(xs, 0).
    Set r to r + 1.
Show checksum.
";

/// A caller BINDS the result (`Let t be trimByte(xs, 0)`) — a borrowed `&[i64]` must not escape into
/// an owned binding, so the caller-gate must drop `trimByte` back to the owned-Seq path.
const STORED: &str = "\
## To trimByte (xs: Seq of Int) and (c: Int) -> Seq of Int:
    Let n be length of xs.
    Let mutable lo be 1.
    Let mutable hi be n.
    Let out be a new Seq of Int.
    Let mutable i be lo.
    While i is at most hi:
        Push item i of xs to out.
        Set i to i + 1.
    Return out.

## To useIt (xs: Seq of Int) -> Seq of Int:
    Let t be trimByte(xs, 0).
    Return t.

## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Show length of useIt(xs).
";

fn fires(rust: &str) -> bool {
    // Borrowed sub-slice signature + the bounds-safe borrowed slice helper.
    rust.contains("&'a [i64]") && rust.contains("return xs.logos_run_through(")
}

#[test]
fn byte_slice_return_lowers_to_borrowed_slice() {
    let rust = compile_to_rust(TRIM).expect("TRIM compiles");
    assert!(
        fires(&rust),
        "the bytes-trim fn must gain a borrowed `&'a [i64]` signature returning the borrowed slice helper; got:\n{rust}"
    );
}

#[test]
fn byte_slice_return_output_byte_identical_all_tiers() {
    let on = aot_output(TRIM);
    let off = aot_output(&decorate_source(TRIM, &["byteslicereturn"]));
    let vm = vm_output(TRIM);
    assert_eq!(
        on, vm,
        "AOT with ByteSliceReturn must match the interpreter (correct trim)"
    );
    assert_eq!(on, off, "ByteSliceReturn must be byte-identical ON vs OFF");
}

#[test]
fn byte_slice_return_gates_off() {
    let rust =
        compile_to_rust(&decorate_source(TRIM, &["byteslicereturn"])).expect("disabled compiles");
    assert!(
        !fires(&rust),
        "`## No byteslicereturn` must keep the owned-Seq build path; got:\n{rust}"
    );
}

#[test]
fn byte_slice_return_bails_on_escape() {
    // The bound-result caller must force `trimByte` back to the owned path (no borrowed signature).
    let rust = compile_to_rust(STORED).expect("STORED compiles");
    assert!(
        !rust.contains("&'a [i64]"),
        "a caller that binds the result must drop the borrowed slice return; got:\n{rust}"
    );
}
