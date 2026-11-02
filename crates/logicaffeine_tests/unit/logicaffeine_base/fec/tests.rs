use super::*;

/// Deterministic splitmix64 — reproducible fuzzing, no external RNG (matches gf256).
struct R(u64);
impl R {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
    fn upto(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn random_shards(rng: &mut R, k: usize, len: usize) -> Vec<Vec<u8>> {
    (0..k)
        .map(|_| (0..len).map(|_| rng.byte()).collect())
        .collect()
}

/// Every size-`m` subset of `0..n`, as index vectors (for exhaustive erasure sweeps).
fn subsets_of_size(n: usize, m: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    for mask in 0u32..(1 << n) {
        if (mask.count_ones() as usize) == m {
            out.push((0..n).filter(|&i| mask & (1 << i) != 0).collect());
        }
    }
    out
}

fn erase(shards: &[Vec<u8>], lost: &[usize]) -> Vec<Option<Vec<u8>>> {
    shards
        .iter()
        .enumerate()
        .map(|(i, s)| {
            if lost.contains(&i) {
                None
            } else {
                Some(s.clone())
            }
        })
        .collect()
}

#[test]
fn new_validates_parameters_and_reports_shard_counts() {
    assert!(
        ReedSolomon::new(0, 2).is_none(),
        "zero data shards is invalid"
    );
    assert!(
        ReedSolomon::new(4, 0).is_none(),
        "zero parity shards is invalid"
    );
    assert!(
        ReedSolomon::new(200, 100).is_none(),
        "n = 300 > 256 field nodes is invalid"
    );
    let rs = ReedSolomon::new(6, 3).expect("6+3 is a valid code");
    assert_eq!(rs.data_shards(), 6);
    assert_eq!(rs.parity_shards(), 3);
    assert_eq!(rs.total_shards(), 9);
    // Largest admissible total.
    assert!(
        ReedSolomon::new(255, 1).is_some(),
        "n = 256 is the boundary and valid"
    );
    assert!(
        ReedSolomon::new(256, 1).is_none(),
        "n = 257 exceeds the field"
    );
}

#[test]
fn encoding_is_systematic_the_first_k_shards_are_the_data() {
    let mut rng = R(0x0000_FEC0_0001);
    for (k, m) in [(1usize, 1usize), (3, 2), (6, 4), (8, 8)] {
        let rs = ReedSolomon::new(k, m).unwrap();
        let data = random_shards(&mut rng, k, 37);
        let shards = rs.encode(&data).expect("well-formed data encodes");
        assert_eq!(shards.len(), k + m, "encode produces n shards");
        assert_eq!(
            &shards[..k],
            &data[..],
            "systematic: first k shards == data (k={k} m={m})"
        );
    }
}

#[test]
fn encode_rejects_wrong_count_or_ragged_lengths() {
    let rs = ReedSolomon::new(4, 2).unwrap();
    assert!(
        rs.encode(&random_shards(&mut R(1), 3, 8)).is_none(),
        "too few shards"
    );
    assert!(
        rs.encode(&random_shards(&mut R(2), 5, 8)).is_none(),
        "too many shards"
    );
    let mut ragged = random_shards(&mut R(3), 4, 8);
    ragged[2].push(0); // one shard longer than the rest
    assert!(
        rs.encode(&ragged).is_none(),
        "ragged shard lengths are rejected"
    );
}

#[test]
fn no_loss_round_trip_reconstructs_every_shard_identically() {
    let mut rng = R(0xABCD_0001);
    for (k, m) in [(1usize, 1usize), (4, 2), (7, 5)] {
        let rs = ReedSolomon::new(k, m).unwrap();
        let data = random_shards(&mut rng, k, 29);
        let encoded = rs.encode(&data).unwrap();
        let all_present: Vec<Option<Vec<u8>>> = encoded.iter().cloned().map(Some).collect();
        let got = rs
            .reconstruct(&all_present)
            .expect("nothing lost reconstructs");
        assert_eq!(
            got, encoded,
            "loss-free reconstruction is the identity (k={k} m={m})"
        );
    }
}

#[test]
fn recovers_from_every_pattern_of_up_to_m_erasures_exhaustively() {
    // For small codes, sweep EVERY subset of ≤ m lost shards and demand byte-exact
    // recovery of both the data shards and the regenerated lost shards.
    let mut rng = R(0x5150_1234_9999);
    for (k, m) in [(2usize, 2usize), (3, 3), (5, 3), (4, 4)] {
        let rs = ReedSolomon::new(k, m).unwrap();
        let n = k + m;
        let data = random_shards(&mut rng, k, 41);
        let encoded = rs.encode(&data).unwrap();
        for lost_count in 0..=m {
            for lost in subsets_of_size(n, lost_count) {
                let received = erase(&encoded, &lost);
                let got = rs
                    .reconstruct(&received)
                    .unwrap_or_else(|| panic!("k={k} m={m}: lost {lost:?} must recover"));
                assert_eq!(
                    got, encoded,
                    "k={k} m={m}: recovering losses {lost:?} must be byte-exact"
                );
                assert_eq!(&got[..k], &data[..], "k={k} m={m}: data shards restored");
            }
        }
    }
}

#[test]
fn declines_when_more_than_m_shards_are_lost() {
    let mut rng = R(0xDEAD_BEEF_0007);
    for (k, m) in [(2usize, 2usize), (4, 3), (5, 4)] {
        let rs = ReedSolomon::new(k, m).unwrap();
        let n = k + m;
        let data = random_shards(&mut rng, k, 17);
        let encoded = rs.encode(&data).unwrap();
        // Every subset of exactly m+1 losses leaves < k survivors: unrecoverable.
        for lost in subsets_of_size(n, m + 1) {
            let received = erase(&encoded, &lost);
            assert!(
                rs.reconstruct(&received).is_none(),
                "k={k} m={m}: losing {} shards (> m) must be undecodable",
                m + 1
            );
        }
    }
}

#[test]
fn mds_property_holds_under_fuzz_across_random_codes_and_loss_patterns() {
    // The heart of the code: for random parameters, data, and any exactly-m loss
    // pattern, recovery is byte-exact; any (m+1) loss pattern is declined.
    let mut rng = R(0x1357_9BDF_2468_ACE0);
    for _ in 0..2000 {
        let k = 1 + rng.upto(8);
        let m = 1 + rng.upto(8);
        let n = k + m;
        let rs = ReedSolomon::new(k, m).unwrap();
        let len = 1 + rng.upto(48);
        let data = random_shards(&mut rng, k, len);
        let encoded = rs.encode(&data).unwrap();

        // A random maximal-but-recoverable loss of exactly m shards.
        let mut idx: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            idx.swap(i, rng.upto(i + 1)); // Fisher-Yates shuffle
        }
        let lost_m = &idx[..m];
        let recovered = rs
            .reconstruct(&erase(&encoded, lost_m))
            .expect("exactly m losses always recover");
        assert_eq!(
            recovered, encoded,
            "k={k} m={m} len={len}: m-loss recovery must be exact"
        );

        // One more loss than the code can bear.
        let lost_m1 = &idx[..m + 1];
        assert!(
            rs.reconstruct(&erase(&encoded, lost_m1)).is_none(),
            "k={k} m={m}: (m+1) losses must be declined"
        );
    }
}

#[test]
fn works_for_single_byte_shards_and_for_long_shards() {
    let mut rng = R(0x0B0B_0B0B);
    let rs = ReedSolomon::new(5, 3).unwrap();
    for len in [1usize, 2, 255, 4096] {
        let data = random_shards(&mut rng, 5, len);
        let encoded = rs.encode(&data).unwrap();
        // Drop the maximum recoverable count (3) at arbitrary positions.
        let received = erase(&encoded, &[0, 4, 7]);
        let got = rs.reconstruct(&received).expect("3 losses recover");
        assert_eq!(
            &got[..5],
            &data[..],
            "len={len}: data recovered across shard lengths"
        );
    }
}

#[test]
fn parity_actually_depends_on_the_data() {
    // A degenerate all-zero generator would "pass" recovery trivially; guard against it
    // by proving parity changes when a single data byte flips.
    let rs = ReedSolomon::new(3, 2).unwrap();
    let base = vec![vec![1u8, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
    let mut flipped = base.clone();
    flipped[0][0] ^= 0xFF;
    let p0 = rs.encode(&base).unwrap();
    let p1 = rs.encode(&flipped).unwrap();
    assert_ne!(
        &p0[3..],
        &p1[3..],
        "parity shards must respond to a data change (non-degenerate code)"
    );
}
