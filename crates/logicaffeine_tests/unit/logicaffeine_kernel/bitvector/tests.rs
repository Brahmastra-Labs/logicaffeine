use super::*;

#[test]
fn rev_n_is_an_involution_on_low_bits() {
    for n in 1..=16 {
        let full = (1i64 << n) - 1;
        for v in 0..(1i64 << n.min(12)) {
            assert_eq!(rev_n(rev_n(v, n), n), v & full);
        }
    }
}

#[test]
fn reflection_identities_are_proven() {
    assert!(
        reflection_symmetry_proven(),
        "reflection certificate failed: {:?}",
        reflection_certificate()
    );
}

/// The check is not vacuous: a WRONG reflection (reverse over n+1 bits) must
/// break at least one identity.
#[test]
fn wrong_reflection_is_rejected() {
    fn wrong_rev(x: i64, n: u32) -> i64 {
        super::rev_n(x, n + 1)
    }
    let mut broke = false;
    'outer: for n in 2..=10 {
        let full = (1i64 << n) - 1;
        for v in 0..(1i64 << n) {
            if (full & !wrong_rev(v, n)) != wrong_rev(full & !v, n) {
                broke = true;
                break 'outer;
            }
        }
    }
    assert!(
        broke,
        "a wrong reflection passed L1 — the certificate is vacuous"
    );
}
