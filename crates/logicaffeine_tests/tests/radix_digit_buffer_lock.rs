//! Lock for `Opt::RadixDigitBuffer` — the itoa stack-buffer pass.
//!
//! The canonical base-N integer→text idiom writes digits least-significant-first into one boxed
//! `Seq of Int` (`digits`), then a SECOND boxed `Seq` (`out`) reverses them, then
//! `text_from_bytes(out)` builds the String. That is two `Rc<RefCell<Vec<i64>>>` allocations, a
//! reversal copy, and a `String::from_utf8` revalidation — all to format a handful of digits.
//!
//! The pass recognizes that shape and lowers it to ONE fixed-size stack buffer `[u8; N]` written
//! BACK-TO-FRONT (the back-to-front write IS the reversal, so the reverse loop + second Seq vanish),
//! then one `String` from the buffer tail. Digits are ASCII (`0-9`/`a-z`), so the result is valid
//! UTF-8. SOUND only when the loop counter is a NATIVE i64 (never a bignum-promotable Int) so the
//! digit count is bounded (≤64 for base ≥ 2) and cannot overflow the buffer.
//!
//! Matches on SHAPE, never on the name `toBase`: any well-written pure-LOGOS itoa benefits. Gates
//! off under `## No radixdigitbuffer`, restoring the byte-identical boxed-Seq round-trip.

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

/// The `toBase` definition + a RUNNABLE Main (loop-driven inputs so nothing const-folds and no
/// `args()` to trap `run_logos_source`), exercising bases 2/8/10/16 over 0..=299 including the
/// `n == 0` path. The VM tier (which never applies the pass) is the correctness oracle.
const TO_BASE_RUN: &str = "\
## To toBase (n: Int) and (base: Int) -> Text:
    If n is equal to 0:
        Return \"0\".
    Let mutable m be n.
    Let digits be a new Seq of Int.
    While m is greater than 0:
        Let d be m % base.
        Let mutable code be 48 + d.
        If d is at least 10:
            Set code to 97 + (d - 10).
        Push code to digits.
        Set m to m / base.
    Let out be a new Seq of Int.
    Let k be length of digits.
    Let mutable i be k.
    While i is at least 1:
        Push item i of digits to out.
        Set i to i - 1.
    Return text_from_bytes(out).

## Main
Let mutable n be 0.
While n is at most 299:
    Show toBase(n, 16).
    Show toBase(n, 2).
    Show toBase(n, 8).
    Show toBase(n, 10).
    Set n to n + 1.
";

/// The canonical base-N itoa (Int params, driven by a RUNTIME arg so nothing constant-folds away):
/// generate digits LSB-first into `digits`, reverse into `out`, `text_from_bytes(out)`.
const TO_BASE: &str = "\
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To toBase (n: Int) and (base: Int) -> Text:
    If n is equal to 0:
        Return \"0\".
    Let mutable m be n.
    Let digits be a new Seq of Int.
    While m is greater than 0:
        Let d be m % base.
        Let mutable code be 48 + d.
        If d is at least 10:
            Set code to 97 + (d - 10).
        Push code to digits.
        Set m to m / base.
    Let out be a new Seq of Int.
    Let k be length of digits.
    Let mutable i be k.
    While i is at least 1:
        Push item i of digits to out.
        Set i to i - 1.
    Return text_from_bytes(out).

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show toBase(n, 16).
";

/// NEGATIVE: `digits` is also read mid-loop for a running total (not a pure itoa buffer), so the
/// bounded-back-to-front rewrite is unsound — the pass must bail and keep the boxed-Seq form.
const DIGITS_READ_MIDLOOP: &str = "\
## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To weird (n: Int) and (base: Int) -> Text:
    Let mutable m be n.
    Let digits be a new Seq of Int.
    While m is greater than 0:
        Let d be m % base.
        Push (48 + d) to digits.
        Push (length of digits) to digits.
        Set m to m / base.
    Let out be a new Seq of Int.
    Let k be length of digits.
    Let mutable i be k.
    While i is at least 1:
        Push item i of digits to out.
        Set i to i - 1.
    Return text_from_bytes(out).

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show weird(n, 16).
";

fn fires(rust: &str) -> bool {
    // Distinctive markers of the lowered form: a fixed-size stack byte buffer and a tail-slice String.
    rust.contains("__radix_buf") && rust.contains("[u8; 64]")
}

#[test]
fn radix_digit_buffer_lowers_itoa_to_stack_buffer() {
    let rust = compile_to_rust(TO_BASE).expect("TO_BASE compiles");
    assert!(
        fires(&rust),
        "the itoa idiom must lower to a stack `[u8; 64]` buffer `__radix_buf`; got:\n{rust}"
    );
    // The two boxed digit Seqs are gone: no `text_from_bytes` round-trip remains on this path.
    assert!(
        !rust.contains("text_from_bytes"),
        "the boxed-Seq reversal + text_from_bytes round-trip must be eliminated; got:\n{rust}"
    );
}

#[test]
fn radix_digit_buffer_bails_when_digits_read_midloop() {
    let rust = compile_to_rust(DIGITS_READ_MIDLOOP).expect("DIGITS_READ_MIDLOOP compiles");
    assert!(
        !fires(&rust),
        "must NOT fire when `digits` is read mid-generation (unsound); got:\n{rust}"
    );
}

#[test]
fn radix_digit_buffer_fires_on_runnable_program() {
    // The runnable program (loop-driven, no args) must still trigger the lowering.
    let rust = compile_to_rust(TO_BASE_RUN).expect("TO_BASE_RUN compiles");
    assert!(
        fires(&rust),
        "the pass must fire on the runnable itoa program; got:\n{rust}"
    );
}

#[test]
fn radix_digit_buffer_output_byte_identical_all_tiers() {
    // The VM tier (pass never applies) is the correctness oracle; AOT with the pass ON, AOT with it
    // OFF, and the VM must all produce the SAME digits for every value/base.
    let on = aot_output(TO_BASE_RUN);
    let off = aot_output(&decorate_source(TO_BASE_RUN, &["radixdigitbuffer"]));
    let vm = vm_output(TO_BASE_RUN);
    assert_eq!(
        on, vm,
        "AOT with RadixDigitBuffer must match the interpreter (correct itoa)"
    );
    assert_eq!(on, off, "RadixDigitBuffer must be byte-identical ON vs OFF");
    // Spot-check a few known values are actually present (255 -> ff, 256 -> 100000000 base2, etc.).
    assert!(on.contains("\nff\n"), "255 in hex must be 'ff'; got:\n{on}");
    assert!(
        on.contains("\n100000000\n"),
        "256 in binary must be '100000000'; got:\n{on}"
    );
}

#[test]
fn radix_digit_buffer_gates_off() {
    let src = decorate_source(TO_BASE, &["radixdigitbuffer"]);
    let rust = compile_to_rust(&src).expect("gated-off compiles");
    assert!(
        !fires(&rust),
        "under `## No radixdigitbuffer` the pass must not fire; got:\n{rust}"
    );
    assert!(
        rust.contains("let mut digits: Vec<i64>")
            && rust.contains("let mut out: Vec<u8>")
            && rust.contains("return String::from_utf8(out)"),
        "gated-off must keep the heap digit and output buffers; got:\n{rust}"
    );
}
