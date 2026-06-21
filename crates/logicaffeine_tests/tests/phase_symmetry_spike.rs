//! B0 spike — de-risk the symmetry-breaking optimization BEFORE building the
//! kernel bitvector theory (plan Part B, Phase 0).
//!
//! Pure-Rust validation that the N-Queens left-right reflection rewrite is
//! arithmetically CORRECT: the per-first-column subcounts are mirror-symmetric
//! (`subcount(c) == subcount(n-1-c)`), so the optimized form — enumerate the
//! left half, double, add the odd-`n` middle — equals the naive count. This is
//! the correctness premise the kernel proof (B1) will discharge for all `n`.
//! If this gate fails, the symmetry approach is wrong and no kernel work should
//! proceed. A negative control proves the mirror test is not vacuous.
//!
//! Sizes capped at n≤10 (full search trees run in debug) to stay quick; larger
//! `n` is covered later by the pass's differential tests.

/// The exact N-Queens bitboard search the benchmark uses (`main.lg`), in Rust.
fn solve(row: i64, cols: i64, d1: i64, d2: i64, n: i64) -> i64 {
    if row == n {
        return 1;
    }
    let all = (1i64 << n) - 1;
    let mut available = all & !(cols | d1 | d2);
    let mut count = 0;
    while available != 0 {
        let bit = available & (-available);
        available ^= bit;
        count += solve(row + 1, cols | bit, (d1 | bit) << 1, (d2 | bit) >> 1, n);
    }
    count
}

fn naive(n: i64) -> i64 {
    solve(0, 0, 0, 0, n)
}

/// Solution count with the first-row queen fixed in column `c`.
fn subcount(c: i64, n: i64) -> i64 {
    let bit = 1i64 << c;
    solve(1, bit, bit << 1, bit >> 1, n)
}

/// The optimized form: enumerate first-row columns in `[0, n/2)`, double, and
/// add the middle column once when `n` is odd.
fn symmetric(n: i64) -> i64 {
    let half = n / 2;
    let mut total: i64 = 0;
    for c in 0..half {
        total += subcount(c, n);
    }
    total *= 2;
    if n % 2 == 1 {
        total += subcount(half, n);
    }
    total
}

/// THE GATE: per-first-column subcounts are mirror-symmetric. This is exactly
/// the invariance the kernel proof must establish for all `n`.
#[test]
fn reflection_subcount_is_mirror_symmetric() {
    for n in 1..=10 {
        for c in 0..n {
            assert_eq!(
                subcount(c, n),
                subcount(n - 1 - c, n),
                "reflection broken at n={n}, column {c} vs {}",
                n - 1 - c
            );
        }
    }
}

/// The optimized rewrite equals the naive count (and the known N-Queens series).
#[test]
fn symmetric_rewrite_matches_naive() {
    // index n: solutions to n-queens (n=0 unused here).
    let expected: [i64; 11] = [1, 1, 0, 0, 2, 10, 4, 40, 92, 352, 724];
    for n in 1..=10 {
        assert_eq!(naive(n), expected[n as usize], "naive count wrong at n={n}");
        assert_eq!(
            symmetric(n),
            naive(n),
            "symmetric rewrite diverged from naive at n={n}"
        );
    }
}

/// Negative control: a BOGUS rotation "symmetry" (`subcount(c) == subcount((c+1) mod n)`)
/// must be violated somewhere — otherwise the mirror test above is vacuous.
#[test]
fn bogus_rotation_symmetry_is_rejected() {
    let mut violated = false;
    for n in 4..=10 {
        for c in 0..n {
            if subcount(c, n) != subcount((c + 1) % n, n) {
                violated = true;
            }
        }
    }
    assert!(
        violated,
        "a bogus rotation symmetry held everywhere — the spike is not testing anything"
    );
}
