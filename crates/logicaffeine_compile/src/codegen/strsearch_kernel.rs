// SIMD substring counter: counts overlapping occurrences of `needle` in
// `haystack[start..end]` (an empty needle counts zero).
#[allow(dead_code)]
pub(crate) fn __logos_count_window_matches(
    haystack: &[u8],
    needle: &[u8],
    start: usize,
    end: usize,
) -> i64 {
    let m = needle.len();
    if m == 0 || haystack.len() < m {
        return 0;
    }
    let last = haystack.len() - m; // last valid start position (inclusive)
    let lo = start.min(last + 1);
    let hi = end.min(last + 1);
    if lo >= hi {
        return 0;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            return unsafe { __logos_cwm_avx2(haystack, needle, lo, hi) };
        }
        return unsafe { __logos_cwm_sse2(haystack, needle, lo, hi) };
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        __logos_cwm_scalar(haystack, needle, lo, hi)
    }
}

// Verify the full window at `p`. The caller guarantees `p <= haystack.len() -
// needle.len()`, so reading `p..p+needle.len()` is in bounds.
#[allow(dead_code)]
#[inline(always)]
fn __logos_cwm_verify(haystack: &[u8], needle: &[u8], p: usize) -> bool {
    haystack[p..p + needle.len()] == *needle
}

// Scalar scan over clamped start positions `[lo, hi)`; used on non-x86 targets.
#[cfg(not(target_arch = "x86_64"))]
#[allow(dead_code)]
fn __logos_cwm_scalar(haystack: &[u8], needle: &[u8], lo: usize, hi: usize) -> i64 {
    let first = needle[0];
    let mut count = 0i64;
    for p in lo..hi {
        if haystack[p] == first && __logos_cwm_verify(haystack, needle, p) {
            count += 1;
        }
    }
    count
}

// AVX2 first-byte scan, 32 candidate positions per vector compare. `hi <=
// haystack.len() - needle.len() + 1`, so every counted position verifies a
// fully in-bounds window.
#[cfg(target_arch = "x86_64")]
#[allow(dead_code)]
#[target_feature(enable = "avx2")]
unsafe fn __logos_cwm_avx2(haystack: &[u8], needle: &[u8], lo: usize, hi: usize) -> i64 {
    use std::arch::x86_64::*;
    let n = haystack.len();
    let splat = _mm256_set1_epi8(needle[0] as i8);
    let mut count = 0i64;
    let mut p = lo;
    while p < hi {
        if p + 32 <= n {
            let v = _mm256_loadu_si256(haystack.as_ptr().add(p) as *const __m256i);
            let mut mask = _mm256_movemask_epi8(_mm256_cmpeq_epi8(v, splat)) as u32;
            while mask != 0 {
                let pos = p + mask.trailing_zeros() as usize;
                if pos >= hi {
                    break; // bits ascend; once past hi every later bit is too
                }
                if __logos_cwm_verify(haystack, needle, pos) {
                    count += 1;
                }
                mask &= mask - 1;
            }
            p += 32;
        } else {
            if *haystack.get_unchecked(p) == needle[0] && __logos_cwm_verify(haystack, needle, p) {
                count += 1;
            }
            p += 1;
        }
    }
    count
}

// SSE2 first-byte scan, 16 candidate positions per vector compare. SSE2 is
// guaranteed on x86_64, so this is the no-AVX2 fast path.
#[cfg(target_arch = "x86_64")]
#[allow(dead_code)]
#[target_feature(enable = "sse2")]
unsafe fn __logos_cwm_sse2(haystack: &[u8], needle: &[u8], lo: usize, hi: usize) -> i64 {
    use std::arch::x86_64::*;
    let n = haystack.len();
    let splat = _mm_set1_epi8(needle[0] as i8);
    let mut count = 0i64;
    let mut p = lo;
    while p < hi {
        if p + 16 <= n {
            let v = _mm_loadu_si128(haystack.as_ptr().add(p) as *const __m128i);
            let mut mask = _mm_movemask_epi8(_mm_cmpeq_epi8(v, splat)) as u32;
            while mask != 0 {
                let pos = p + mask.trailing_zeros() as usize;
                if pos >= hi {
                    break;
                }
                if __logos_cwm_verify(haystack, needle, pos) {
                    count += 1;
                }
                mask &= mask - 1;
            }
            p += 16;
        } else {
            if *haystack.get_unchecked(p) == needle[0] && __logos_cwm_verify(haystack, needle, p) {
                count += 1;
            }
            p += 1;
        }
    }
    count
}
