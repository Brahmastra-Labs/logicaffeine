use super::*;

#[cfg(target_arch = "x86_64")]
fn hw_available() -> bool {
    std::is_x86_feature_detected!("sha")
        && std::is_x86_feature_detected!("ssse3")
        && std::is_x86_feature_detected!("sse4.1")
}

/// Drive random inputs through both the software op and the real intrinsic; they must be equal —
/// this is the proof that the software spec IS the hardware.
#[cfg(target_arch = "x86_64")]
#[test]
fn software_sha_ops_equal_the_hardware_intrinsics() {
    if !hw_available() {
        eprintln!("skipping: no SHA-NI on this CPU");
        return;
    }
    use core::arch::x86_64::*;

    #[target_feature(enable = "sha,sse2,ssse3,sse4.1")]
    unsafe fn check(a: [u32; 4], b: [u32; 4]) {
        let va = _mm_loadu_si128(a.as_ptr() as *const __m128i);
        let vb = _mm_loadu_si128(b.as_ptr() as *const __m128i);
        let store = |v: __m128i| {
            let mut o = [0u32; 4];
            _mm_storeu_si128(o.as_mut_ptr() as *mut __m128i, v);
            o
        };
        assert_eq!(
            store(_mm_sha1msg1_epu32(va, vb)),
            sha1msg1(a, b),
            "sha1msg1"
        );
        assert_eq!(
            store(_mm_sha1msg2_epu32(va, vb)),
            sha1msg2(a, b),
            "sha1msg2"
        );
        assert_eq!(
            store(_mm_sha1nexte_epu32(va, vb)),
            sha1nexte(a, b),
            "sha1nexte"
        );
        assert_eq!(
            store(_mm_sha1rnds4_epu32(va, vb, 0)),
            sha1rnds4(a, b, 0),
            "rnds4 f0"
        );
        assert_eq!(
            store(_mm_sha1rnds4_epu32(va, vb, 1)),
            sha1rnds4(a, b, 1),
            "rnds4 f1"
        );
        assert_eq!(
            store(_mm_sha1rnds4_epu32(va, vb, 2)),
            sha1rnds4(a, b, 2),
            "rnds4 f2"
        );
        assert_eq!(
            store(_mm_sha1rnds4_epu32(va, vb, 3)),
            sha1rnds4(a, b, 3),
            "rnds4 f3"
        );
    }

    let mut s: u64 = 0x0123_4567_89ab_cdef;
    let mut next = || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 32) as u32
    };
    for _ in 0..20_000 {
        let a = [next(), next(), next(), next()];
        let b = [next(), next(), next(), next()];
        unsafe { check(a, b) };
    }
}
