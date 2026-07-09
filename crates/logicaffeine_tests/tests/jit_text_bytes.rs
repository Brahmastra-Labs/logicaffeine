//! Text-as-bytes JIT representation: `item i of text` on an ASCII string must
//! lower to a pinned BYTE load (char index == byte index for ASCII) and char
//! equality must lower to integer byte comparison, so the string-scanning hot
//! loops (string_search, strings) stop running 100% on bytecode.
//!
//! SOUNDNESS — the differential gate is sacred (the JIT must be BIT-IDENTICAL to
//! the tree-walker):
//!   * LOGOS text indexing is 1-based by CHARACTER (Unicode scalar). The pinned
//!     byte representation is only equivalent for ASCII text (where char index ==
//!     byte index AND `length of text` in bytes == char count). A region that
//!     pins a Text MUST guard at entry that the observed Text is ASCII; a
//!     non-ASCII Text deopts to bytecode so the per-char decode path runs.
//!   * `item i of text` returns a 1-char `RuntimeValue::Text` in the VM; the
//!     equality lowering must agree with `compare::values_equal` for the cases it
//!     handles (single ASCII chars compare equal iff their bytes are equal).
//!   * A Text register that can be REASSIGNED inside the region (string growth via
//!     `Set text to text + ...`) must NOT be pinned stale — such a region bails or
//!     guards.
//!
//! Every test pins VM output bit-identically to the tree-walker; the differential
//! IS the spec. Tiering assertions (region successes) are separate, secondary
//! checks: correctness never depends on whether the region tiers.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run through the tiered VM + the tree-walker, assert they agree, and return
/// `(normalized output, error, region_attempts, region_successes)`.
fn tiered(src: &str) -> (String, Option<String>, u32, u32) {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on:\n{src}"
    );
    let (attempts, successes) = tier.region_counts();
    (norm(&vm.output), vm.error, attempts, successes)
}

/// A hot character-scanning loop over an ASCII text: count occurrences of a
/// single character by indexing the text and comparing the extracted 1-char
/// Text against a one-char literal. The build loop grows the string with
/// `AddAssign` (so `text` is mutated EARLIER), but the scan loop reads it
/// read-only — only the scan loop region is a candidate for the TextBytes pin.
#[test]
fn ascii_char_scan_count_tiers_and_matches() {
    let src = "## Main\n\
               Let mutable text be \"\".\n\
               Let mutable i be 0.\n\
               While i is less than 4000:\n\
               \x20   Set text to text + \"a\".\n\
               \x20   Set text to text + \"b\".\n\
               \x20   Set i to i + 1.\n\
               Let textLen be length of text.\n\
               Let mutable count be 0.\n\
               Let mutable j be 1.\n\
               While j is at most textLen:\n\
               \x20   If item j of text equals \"a\":\n\
               \x20       Set count to count + 1.\n\
               \x20   Set j to j + 1.\n\
               Show count.\n";
    let (out, err, attempts, successes) = tiered(src);
    assert_eq!(err, None);
    // 4000 'a' chars (one per iteration).
    assert_eq!(out, "4000");
    assert!(attempts >= 1, "the hot scan loop must be attempted as a region");
    assert!(
        successes >= 1,
        "the ASCII char-scan loop must tier with a TextBytes pin \
         (attempts={attempts} successes={successes})"
    );
}

/// The string_search hot loop shape: a naive substring search comparing
/// `item (i + j) of text` against `item (j + 1) of needle` with `is not`
/// (NotEq) between two single-char Texts. BOTH sides are pinned TextBytes, so
/// the char inequality lowers to an integer byte comparison.
#[test]
fn naive_substring_search_tiers_and_matches() {
    let src = "## Main\n\
               Let mutable text be \"\".\n\
               Let mutable pos be 0.\n\
               While pos is less than 2000:\n\
               \x20   If pos % 100 equals 0:\n\
               \x20       Set text to text + \"XXXXX\".\n\
               \x20   Set text to text + \"a\".\n\
               \x20   Set pos to pos + 1.\n\
               Let needle be \"XXXXX\".\n\
               Let needleLen be 5.\n\
               Let textLen be length of text.\n\
               Let mutable count be 0.\n\
               Let mutable i be 1.\n\
               While i is at most textLen - needleLen + 1:\n\
               \x20   Let mutable match be 1.\n\
               \x20   Let mutable j be 0.\n\
               \x20   While j is less than needleLen:\n\
               \x20       If item (i + j) of text is not item (j + 1) of needle:\n\
               \x20           Set match to 0.\n\
               \x20           Set j to needleLen.\n\
               \x20       Set j to j + 1.\n\
               \x20   If match equals 1:\n\
               \x20       Set count to count + 1.\n\
               \x20   Set i to i + 1.\n\
               Show count.\n";
    let (out, err, attempts, successes) = tiered(src);
    assert_eq!(err, None);
    // "XXXXX" is appended 20 times (pos % 100 == 0 for pos in {0,100,...,1900}),
    // each occurrence found once by the naive search.
    assert_eq!(out, "20");
    assert!(attempts >= 1, "the search loop must be attempted as a region");
    assert!(
        successes >= 1,
        "the ASCII substring search loop must tier \
         (attempts={attempts} successes={successes})"
    );
}

/// Char equality between two extracted ASCII chars (NOT against a literal):
/// `item i of a equals item i of b`. Both pin as TextBytes; equality lowers to
/// integer compare and must agree with the tree-walker's `Text == Text`.
#[test]
fn char_equality_between_two_texts_matches() {
    let src = "## Main\n\
               Let a be \"abcabcabcabc\".\n\
               Let b be \"abxabcabxabc\".\n\
               Let n be length of a.\n\
               Let mutable same be 0.\n\
               Let mutable i be 1.\n\
               Let mutable pass be 0.\n\
               While pass is less than 1000:\n\
               \x20   Set i to 1.\n\
               \x20   While i is at most n:\n\
               \x20       If item i of a equals item i of b:\n\
               \x20           Set same to same + 1.\n\
               \x20       Set i to i + 1.\n\
               \x20   Set pass to pass + 1.\n\
               Show same.\n";
    let (out, err, _attempts, _successes) = tiered(src);
    assert_eq!(err, None);
    // a and b differ at positions 3 and 9 (1-based); 10 of 12 match each pass,
    // over 1000 passes = 10000.
    assert_eq!(out, "10000");
}

/// Out-of-bounds index parity: an ASCII text indexed past its end must raise
/// the SAME error (and produce the same partial output) on the VM as on the
/// tree-walker — the pinned byte load is bounds-checked exactly like the
/// per-char path. The `equals` here is left side of an If, so on the faulting
/// access the whole program errors identically.
#[test]
fn oob_text_index_parity() {
    let src = "## Main\n\
               Let text be \"hello\".\n\
               Let mutable count be 0.\n\
               Let mutable i be 1.\n\
               While i is at most 100:\n\
               \x20   If item i of text equals \"l\":\n\
               \x20       Set count to count + 1.\n\
               \x20   Set i to i + 1.\n\
               Show count.\n";
    // The VM and tree-walker must AGREE: both error at index 6 (past "hello").
    let (_out, err, _attempts, _successes) = tiered(src);
    assert!(
        err.is_some(),
        "indexing past the end of an ASCII text must error on both engines"
    );
}

/// Non-ASCII text must stay CORRECT: char index != byte index, so the byte
/// representation would diverge. The region must guard ASCII at entry and bail
/// (deopt to bytecode) on a non-ASCII Text — but the OUTPUT must still match the
/// tree-walker bit-for-bit. The é (2 bytes) at the start makes byte index 2
/// land mid-character; the per-char path counts the correct number of "é".
#[test]
fn non_ascii_text_stays_correct() {
    // "éaéaéa..." — 'é' is 2 bytes, 'a' is 1 byte: char index != byte index.
    let src = "## Main\n\
               Let mutable text be \"\".\n\
               Let mutable i be 0.\n\
               While i is less than 50:\n\
               \x20   Set text to text + \"é\".\n\
               \x20   Set text to text + \"a\".\n\
               \x20   Set i to i + 1.\n\
               Let n be 100.\n\
               Let mutable count be 0.\n\
               Let mutable j be 1.\n\
               Let mutable pass be 0.\n\
               While pass is less than 500:\n\
               \x20   Set j to 1.\n\
               \x20   While j is at most n:\n\
               \x20       If item j of text equals \"é\":\n\
               \x20           Set count to count + 1.\n\
               \x20       Set j to j + 1.\n\
               \x20   Set pass to pass + 1.\n\
               Show count.\n";
    let (out, err, _attempts, _successes) = tiered(src);
    assert_eq!(err, None);
    // 50 'é' chars per pass (char-indexed), over 500 passes = 25000. A byte-pin
    // without an ASCII guard would miscount; the differential assert in `tiered`
    // already guarantees agreement, this pins the absolute value too.
    assert_eq!(out, "25000");
}

/// The `strings` benchmark shape: comparing an extracted ASCII char against a
/// ONE-CHARACTER text LITERAL (`item i of result equals " "`). The constant
/// lowers to its single byte and the comparison is an integer byte compare —
/// the const→byte path that unblocks `strings`. The build loop grows the string
/// (so `result` is mutated earlier), but the scan loop reads it read-only.
#[test]
fn ascii_char_vs_literal_tiers_and_matches() {
    let src = "## Main\n\
               Let mutable result be \"\".\n\
               Let mutable i be 0.\n\
               While i is less than 3000:\n\
               \x20   Set result to result + \"x\".\n\
               \x20   Set result to result + \" \".\n\
               \x20   Set i to i + 1.\n\
               Let mutable count be 0.\n\
               Let mutable j be 1.\n\
               While j is at most length of result:\n\
               \x20   If item j of result equals \" \":\n\
               \x20       Set count to count + 1.\n\
               \x20   Set j to j + 1.\n\
               Show count.\n";
    let (out, err, attempts, successes) = tiered(src);
    assert_eq!(err, None);
    // One space appended per iteration over 3000 iterations.
    assert_eq!(out, "3000");
    assert!(attempts >= 1, "the scan loop must be attempted as a region");
    assert!(
        successes >= 1,
        "the char-vs-literal scan loop must tier with a TextBytes pin + \
         const-byte lowering (attempts={attempts} successes={successes})"
    );
}
