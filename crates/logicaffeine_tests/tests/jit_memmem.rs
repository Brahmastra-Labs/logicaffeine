//! The naive substring-search idiom (string_search benchmark) lowered to a
//! single `logos_rt_memmem` runtime-helper call instead of the per-byte
//! `ArrLoad` nested loop.
//!
//! SOUNDNESS — the differential gate is sacred: the tiered VM (with the memmem
//! recognizer) must be BIT-IDENTICAL to the tree-walker. The helper must count
//! OVERLAPPING occurrences with the SAME semantics as the LOGOS nested loop:
//!   * a full match at 1-based position `i` (text[i..i+needleLen-1] equals
//!     needle[1..needleLen]) is counted; positions overlap;
//!   * the outer loop ranges `i` over `[start, textLen - needleLen + 1]`
//!     inclusive (1-based);
//!   * an empty needle (needleLen == 0) matches at EVERY outer position;
//!   * a needle longer than the haystack yields zero matches (the outer bound
//!     is non-positive);
//!   * a single-char needle and a match at the very last position are counted;
//!   * any access the nest would take out of bounds (a checked needle index)
//!     must side-exit to bytecode so the EXACT error/behavior is reproduced.
//!
//! Every search test pins VM output bit-identically to the tree-walker; the
//! differential IS the spec. Tiering assertions (region success + the helper
//! actually firing) are secondary — correctness never depends on whether the
//! region tiers. A direct unit test of `logos_rt_memmem` pins the helper itself
//! against a plain reference count.

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

/// Build the string_search nested-loop program over a literal `text` and a
/// literal `needle` of declared length `needle_len`, scanned `passes` times so
/// the outer loop goes hot and tiers.
fn search_program(text: &str, needle: &str, needle_len: usize, passes: usize) -> String {
    format!(
        "## Main\n\
         Let text be \"{text}\".\n\
         Let needle be \"{needle}\".\n\
         Let needleLen be {needle_len}.\n\
         Let textLen be length of text.\n\
         Let mutable count be 0.\n\
         Let mutable pass be 0.\n\
         While pass is less than {passes}:\n\
         \x20   Set count to 0.\n\
         \x20   Let mutable i be 1.\n\
         \x20   While i is at most textLen - needleLen + 1:\n\
         \x20       Let mutable match be 1.\n\
         \x20       Let mutable j be 0.\n\
         \x20       While j is less than needleLen:\n\
         \x20           If item (i + j) of text is not item (j + 1) of needle:\n\
         \x20               Set match to 0.\n\
         \x20               Set j to needleLen.\n\
         \x20           Set j to j + 1.\n\
         \x20       If match equals 1:\n\
         \x20           Set count to count + 1.\n\
         \x20       Set i to i + 1.\n\
         \x20   Set pass to pass + 1.\n\
         Show count.\n"
    )
}

/// Plain reference count of OVERLAPPING ASCII occurrences of `needle` in
/// `text`, matching the LOGOS nest's semantics exactly.
fn ref_count(text: &str, needle: &str, needle_len: usize) -> i64 {
    let t = text.as_bytes();
    let n = needle.as_bytes();
    let text_len = t.len() as i64;
    let nl = needle_len as i64;
    let bound = text_len - nl + 1;
    let mut count = 0i64;
    let mut i = 1i64; // 1-based
    while i <= bound {
        let mut m = true;
        for j in 0..nl {
            // text[i+j] 1-based, needle[j+1] 1-based
            if t[(i + j - 1) as usize] != n[j as usize] {
                m = false;
                break;
            }
        }
        if m {
            count += 1;
        }
        i += 1;
    }
    count
}

// ---------------------------------------------------------------------------
// The benchmark shape at a small size: many "XXXXX" needles in a long text.
// ---------------------------------------------------------------------------

#[test]
fn benchmark_shape_small_matches_and_tiers() {
    // Build a text shaped like the benchmark: blocks of "XXXXX" sprinkled in a
    // sea of other letters, scanned for the "XXXXX" needle.
    let mut text = String::new();
    for k in 0..40 {
        if k % 7 == 0 {
            text.push_str("XXXXX");
        }
        text.push_str("abcde");
    }
    let needle = "XXXXX";
    let expected = ref_count(&text, needle, 5);
    let src = search_program(&text, needle, 5, 60);
    let (out, err, attempts, successes) = tiered(&src);
    assert_eq!(err, None);
    assert_eq!(out, expected.to_string());
    assert!(attempts >= 1, "the outer search loop must be attempted as a region");
    assert!(
        successes >= 1,
        "the naive-search nest must tier (attempts={attempts} successes={successes})"
    );
}

// ---------------------------------------------------------------------------
// Overlapping matches: "aa" in "aaaaaa" → 5 (positions 1..5).
// ---------------------------------------------------------------------------

#[test]
fn overlapping_matches_counted() {
    let text = "aaaaaa";
    let needle = "aa";
    assert_eq!(ref_count(text, needle, 2), 5);
    let src = search_program(text, needle, 2, 120);
    let (out, err, _attempts, _successes) = tiered(&src);
    assert_eq!(err, None);
    assert_eq!(out, "5");
}

/// The canonical overlap case from the task brief: "aa" in "aaaa" → 3.
#[test]
fn overlap_aa_in_aaaa_is_three() {
    let src = search_program("aaaa", "aa", 2, 120);
    let (out, err, _attempts, _successes) = tiered(&src);
    assert_eq!(err, None);
    assert_eq!(out, "3");
}

// ---------------------------------------------------------------------------
// Needle not found.
// ---------------------------------------------------------------------------

#[test]
fn needle_not_found_is_zero() {
    let text = "abcdefghijabcdefghij";
    let needle = "zzz";
    assert_eq!(ref_count(text, needle, 3), 0);
    let src = search_program(text, needle, 3, 120);
    let (out, err, _attempts, _successes) = tiered(&src);
    assert_eq!(err, None);
    assert_eq!(out, "0");
}

// ---------------------------------------------------------------------------
// Needle longer than haystack: bound non-positive → zero.
// ---------------------------------------------------------------------------

#[test]
fn needle_longer_than_haystack_is_zero() {
    let text = "abc";
    let needle = "abcdef";
    assert_eq!(ref_count(text, needle, 6), 0);
    let src = search_program(text, needle, 6, 120);
    let (out, err, _attempts, _successes) = tiered(&src);
    assert_eq!(err, None);
    assert_eq!(out, "0");
}

// ---------------------------------------------------------------------------
// Single-char needle.
// ---------------------------------------------------------------------------

#[test]
fn single_char_needle_counts_all_occurrences() {
    let text = "abababababab";
    let needle = "a";
    assert_eq!(ref_count(text, needle, 1), 6);
    let src = search_program(text, needle, 1, 120);
    let (out, err, _attempts, _successes) = tiered(&src);
    assert_eq!(err, None);
    assert_eq!(out, "6");
}

// ---------------------------------------------------------------------------
// Match at the very end of the haystack.
// ---------------------------------------------------------------------------

#[test]
fn match_at_the_very_end() {
    let text = "qqqqqqqqqXYZ";
    let needle = "XYZ";
    assert_eq!(ref_count(text, needle, 3), 1);
    let src = search_program(text, needle, 3, 120);
    let (out, err, _attempts, _successes) = tiered(&src);
    assert_eq!(err, None);
    assert_eq!(out, "1");
}

/// Match at the start AND end, plus interior, all counted.
#[test]
fn matches_at_both_ends() {
    let text = "abXYabYYabZZab"; // "ab" at 1,5,9,13
    let needle = "ab";
    assert_eq!(ref_count(text, needle, 2), 4);
    let src = search_program(text, needle, 2, 120);
    let (out, err, _attempts, _successes) = tiered(&src);
    assert_eq!(err, None);
    assert_eq!(out, "4");
}

// ---------------------------------------------------------------------------
// Deopt parity: a declared needleLen LONGER than the needle buffer makes the
// nest's CHECKED `item (j+1) of needle` index run past the needle and error.
// The memmem helper must NOT swallow this — it returns the deopt sentinel and
// the region replays the exact nest on bytecode, raising the identical error.
// ---------------------------------------------------------------------------

#[test]
fn needle_len_past_buffer_errors_identically() {
    // needle is "ab" (length 2) but needleLen is declared 4: scanning the text
    // "abababab" reaches `item 3 of needle` (out of bounds) and must error the
    // SAME way on both engines. `bound = textLen - needleLen + 1 = 8 - 4 + 1 = 5`
    // so the outer loop runs and the inner loop indexes the needle past its end.
    let src = search_program("abababab", "ab", 4, 120);
    let (_out, err, _attempts, _successes) = tiered(&src);
    assert!(
        err.is_some(),
        "indexing the needle past its declared length must error on both engines"
    );
}

/// The helper's deopt sentinel is returned (not a guessed count) when the
/// needle index would run past the needle buffer.
#[test]
fn helper_deopts_when_needle_len_exceeds_buffer() {
    let text = "abababab";
    let needle = "ab";
    let t = text.as_bytes();
    let n = needle.as_bytes();
    let h_len = t.len() as i64;
    let n_buf_len = n.len() as i64; // 2
    let needle_len = 4i64; // declared longer than the buffer
    let bound = h_len - needle_len + 1;
    let got = unsafe {
        logicaffeine_jit::logos_rt_memmem(
            t.as_ptr() as i64,
            h_len,
            n.as_ptr() as i64,
            n_buf_len,
            needle_len,
            1,
            bound,
        )
    };
    assert_eq!(got, logicaffeine_jit::LOGOS_MEMMEM_DEOPT);
}

// ---------------------------------------------------------------------------
// Direct unit test of the helper itself vs the reference count.
// ---------------------------------------------------------------------------

/// `logos_rt_memmem(haystack, h_len, needle, n_len, needle_len, start, bound)`
/// counting overlapping matches must equal the plain reference over a grid of
/// cases (overlap, not-found, single-char, end-anchored, full-haystack match).
#[test]
fn helper_matches_reference_over_grid() {
    let cases: &[(&str, &str)] = &[
        ("aaaaaa", "aa"),
        ("aaaa", "aa"),
        ("abababab", "ab"),
        ("abcdefg", "xyz"),
        ("hello world", "o"),
        ("mississippi", "issi"),
        ("xxxxxx", "x"),
        ("the end is XYZ", "XYZ"),
        ("ABCABCABC", "ABC"),
        ("a", "a"),
        ("", "a"),
        ("abc", ""),
    ];
    for &(text, needle) in cases {
        let t = text.as_bytes();
        let n = needle.as_bytes();
        let needle_len = n.len();
        let h_len = t.len() as i64;
        let n_buf_len = n.len() as i64;
        let bound = h_len - needle_len as i64 + 1;
        let expected = ref_count(text, needle, needle_len);
        let got = unsafe {
            logicaffeine_jit::logos_rt_memmem(
                t.as_ptr() as i64,
                h_len,
                n.as_ptr() as i64,
                n_buf_len,
                needle_len as i64,
                1, // 1-based start
                bound,
            )
        };
        assert_eq!(
            got, expected,
            "memmem({text:?}, {needle:?}) = {got}, expected {expected}"
        );
    }
}

/// Empty needle: matches at every outer position (needleLen == 0 → inner loop
/// never runs, match stays 1). bound = h_len + 1, so positions 1..=h_len+1.
#[test]
fn helper_empty_needle_matches_everywhere() {
    let text = "abcd";
    let t = text.as_bytes();
    let h_len = t.len() as i64;
    let bound = h_len - 0 + 1; // = 5
    let expected = ref_count(text, "", 0); // 5
    let got = unsafe {
        logicaffeine_jit::logos_rt_memmem(
            t.as_ptr() as i64,
            h_len,
            text.as_ptr() as i64,
            0,
            0,
            1,
            bound,
        )
    };
    assert_eq!(got, expected);
    assert_eq!(got, 5);
}

// ---------------------------------------------------------------------------
// End-to-end: the REAL string_search benchmark program (its main.lg), run at a
// small size, must match the tree-walker AND tier the search nest as a region.
// ---------------------------------------------------------------------------

#[test]
fn real_benchmark_program_matches_and_tiers() {
    let src = include_str!("../../../benchmarks/programs/string_search/main.lg");
    let args = vec!["string_search".to_string(), "3000".to_string()];
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &args, Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &args);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "tiered VM diverged from tree-walker on the real string_search benchmark"
    );
    assert_eq!(vm.error, None);
    let (attempts, successes) = tier.region_counts();
    assert!(attempts >= 1, "the search loop must be attempted as a region");
    assert!(
        successes >= 1,
        "the real string_search search nest must tier via the memmem collapse \
         (attempts={attempts} successes={successes})"
    );
}

/// A starting position other than 1 (mid-scan) is honored.
#[test]
fn helper_honors_start_offset() {
    let text = "aaaaaa";
    let t = text.as_bytes();
    let n = b"aa";
    let h_len = t.len() as i64;
    let needle_len = 2i64;
    let bound = h_len - needle_len + 1; // 5
    // start at i=3 → positions 3,4,5 all match → 3
    let got = unsafe {
        logicaffeine_jit::logos_rt_memmem(
            t.as_ptr() as i64,
            h_len,
            n.as_ptr() as i64,
            2,
            needle_len,
            3,
            bound,
        )
    };
    assert_eq!(got, 3);
}
