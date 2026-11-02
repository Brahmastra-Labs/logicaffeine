// Compile the exact kernel text that ships into generated programs.
include!("../../../../../logicaffeine_compile/src/codegen/strsearch_kernel.rs");

/// Independent brute-force oracle over the same clamped `[lo, hi)` start
/// range and the same empty-needle convention as the kernel.
fn reference(haystack: &[u8], needle: &[u8], start: usize, end: usize) -> i64 {
    let m = needle.len();
    if m == 0 || haystack.len() < m {
        return 0;
    }
    let last = haystack.len() - m;
    let lo = start.min(last + 1);
    let hi = end.min(last + 1);
    let mut c = 0i64;
    for p in lo..hi {
        if &haystack[p..p + m] == needle {
            c += 1;
        }
    }
    c
}

/// Deterministic xorshift64* PRNG — no external dependency, reproducible.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

#[test]
fn overlapping_matches_are_all_counted() {
    assert_eq!(
        __logos_count_window_matches(b"aaaaaa", b"aa", 0, usize::MAX),
        5
    );
    assert_eq!(
        __logos_count_window_matches(b"aaaaa", b"aaa", 0, usize::MAX),
        3
    );
}

#[test]
fn the_benchmark_pattern() {
    let mut text = Vec::new();
    let mut pos = 0usize;
    let n = 100_000usize;
    while pos < n {
        if pos > 0 && pos % 1000 == 0 && pos + 5 <= n {
            text.extend_from_slice(b"XXXXX");
            pos += 5;
        } else {
            text.push(b'a' + (pos % 5) as u8);
            pos += 1;
        }
    }
    let got = __logos_count_window_matches(&text, b"XXXXX", 0, usize::MAX);
    assert_eq!(got, reference(&text, b"XXXXX", 0, usize::MAX));
    assert_eq!(got, 99);
}

#[test]
fn edge_cases() {
    assert_eq!(__logos_count_window_matches(b"", b"a", 0, usize::MAX), 0);
    assert_eq!(__logos_count_window_matches(b"abc", b"", 0, usize::MAX), 0);
    assert_eq!(
        __logos_count_window_matches(b"ab", b"abc", 0, usize::MAX),
        0
    );
    assert_eq!(
        __logos_count_window_matches(b"abc", b"abc", 0, usize::MAX),
        1
    );
    assert_eq!(__logos_count_window_matches(b"a", b"a", 0, usize::MAX), 1);
    let mut h = vec![b'z'; 40];
    h.extend_from_slice(b"needle");
    assert_eq!(
        __logos_count_window_matches(&h, b"needle", 0, usize::MAX),
        1
    );
    assert_eq!(__logos_count_window_matches(b"aaaaaa", b"aa", 2, 4), 2);
    assert_eq!(__logos_count_window_matches(b"aaaaaa", b"aa", 100, 200), 0);
}

#[test]
fn single_byte_needle_dense() {
    let h = b"XaXbXcXdXeXfXX";
    assert_eq!(
        __logos_count_window_matches(h, b"X", 0, usize::MAX),
        reference(h, b"X", 0, usize::MAX)
    );
}

#[test]
fn differential_fuzz_full_range() {
    let mut rng = Rng(0x1234_5678_9abc_def0);
    for _ in 0..40_000 {
        let hlen = rng.below(140); // spans <16, 16..32, >32 to cover all SIMD paths
        let alphabet = 1 + rng.below(4); // tiny alphabet -> frequent overlapping matches
        let haystack: Vec<u8> = (0..hlen)
            .map(|_| b'a' + (rng.below(alphabet) as u8))
            .collect();
        let nlen = rng.below(6); // includes 0
        let needle: Vec<u8> = (0..nlen)
            .map(|_| b'a' + (rng.below(alphabet) as u8))
            .collect();
        let got = __logos_count_window_matches(&haystack, &needle, 0, usize::MAX);
        let want = reference(&haystack, &needle, 0, usize::MAX);
        assert_eq!(got, want, "haystack={haystack:?} needle={needle:?}");
    }
}

#[test]
fn differential_fuzz_subranges_and_full_byte_domain() {
    let mut rng = Rng(0xfeed_face_dead_beef);
    for _ in 0..40_000 {
        let hlen = rng.below(200);
        let haystack: Vec<u8> = (0..hlen).map(|_| rng.next() as u8).collect();
        let nlen = 1 + rng.below(8);
        let needle: Vec<u8> = (0..nlen)
            .map(|_| {
                if hlen > 0 && rng.below(2) == 0 {
                    haystack[rng.below(hlen)]
                } else {
                    rng.next() as u8
                }
            })
            .collect();
        let start = rng.below(hlen + 5);
        let end = start + rng.below(hlen + 5);
        let got = __logos_count_window_matches(&haystack, &needle, start, end);
        let want = reference(&haystack, &needle, start, end);
        assert_eq!(
            got, want,
            "hlen={hlen} needle={needle:?} start={start} end={end}"
        );
    }
}
