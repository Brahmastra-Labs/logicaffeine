use super::*;

/// SECURITY/UB regression (CRITIQUE #6). `logos_rt_memmem` cast its `i64` lengths
/// straight to `usize` for `from_raw_parts`, so a negative `needle_len` or
/// `haystack_len` became a ~2^64 slice length — immediate undefined behavior
/// (`from_raw_parts` requires `len <= isize::MAX`), reachable on program-controlled
/// input. The guard must bail to the recoverable deopt sentinel before constructing
/// any slice. (The pre-fix failure is UB, so it is established by inspection of the
/// `from_raw_parts` calls rather than executed as a crashing test.)
#[test]
fn out_of_range_lengths_bail_safely_without_constructing_an_invalid_slice() {
    let haystack = b"abcabc";
    let needle = b"bc";
    let hp = haystack.as_ptr() as i64;
    let np = needle.as_ptr() as i64;
    unsafe {
        // negative needle_len → ~2^64 slice length pre-fix; must bail.
        assert_eq!(
            logos_rt_memmem(hp, 6, np, 2, -1, 1, 0),
            LOGOS_MEMMEM_DEOPT,
            "negative needle_len must bail to deopt, never reach from_raw_parts"
        );
        assert_eq!(
            logos_rt_memmem(hp, -1, np, 2, 2, 1, 0),
            LOGOS_MEMMEM_DEOPT,
            "negative haystack_len must bail"
        );
        assert_eq!(
            logos_rt_memmem(hp, 6, np, -1, 2, 1, 0),
            LOGOS_MEMMEM_DEOPT,
            "negative needle_buf_len must bail"
        );
    }
}

/// Regression: valid inputs still search correctly after the guard is added.
/// `"bc"` occurs twice in `"abcabc"` (0-based offsets 1 and 4).
#[test]
fn valid_search_still_counts_matches() {
    let haystack = b"abcabc";
    let needle = b"bc";
    let count = unsafe {
        logos_rt_memmem(
            haystack.as_ptr() as i64,
            6,
            needle.as_ptr() as i64,
            2,
            2,
            1,
            0,
        )
    };
    assert_eq!(count, 2, "the guard must not change correct search results");
}
