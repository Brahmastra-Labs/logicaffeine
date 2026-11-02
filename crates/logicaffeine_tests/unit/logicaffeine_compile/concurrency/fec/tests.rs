use super::*;
use logicaffeine_base::gf256::div;

struct R(u64);
impl R {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[test]
fn gf_mul_div_are_inverse_across_the_field() {
    for a in 1u8..=255 {
        for b in 1u8..=255 {
            assert_eq!(div(mul(a, b), b), a, "div(mul(a,b),b)=a for a={a} b={b}");
        }
    }
}

#[test]
fn systematic_first_k_shards_are_the_data() {
    let data: Vec<u8> = (0..40u8).collect();
    let (k, n) = (5usize, 8usize);
    let (_, shards) = encode(&data, k, n).unwrap();
    let shard_len = 8;
    for j in 0..k {
        assert_eq!(
            &shards[j][..],
            &data[j * shard_len..(j + 1) * shard_len],
            "data shard {j} must pass through unchanged (systematic)"
        );
    }
}

#[test]
fn reconstructs_from_any_k_of_n_sliding_window() {
    let mut rng = R(0x1234_5678);
    for &(k, n) in &[
        (4usize, 6usize),
        (6, 10),
        (3, 5),
        (8, 12),
        (1, 3),
        (10, 16),
        (2, 9),
    ] {
        let len = (rng.next() % 600) as usize + 1;
        let data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let (orig, shards) = encode(&data, k, n).unwrap();
        // Every window of (n-k) consecutive (mod n) lost shards must still reconstruct.
        for drop_start in 0..n {
            let dropped: Vec<usize> = (0..(n - k)).map(|d| (drop_start + d) % n).collect();
            let have: Vec<(usize, Vec<u8>)> = (0..n)
                .filter(|i| !dropped.contains(i))
                .map(|i| (i, shards[i].clone()))
                .collect();
            let got = decode(orig, k, n, &have).expect("reconstruct from k survivors");
            assert_eq!(got, data, "k={k} n={n} dropped={dropped:?}");
        }
    }
}

#[test]
fn reconstructs_from_random_k_subsets() {
    let mut rng = R(0xC0FF_EE00);
    for &(k, n) in &[(5usize, 9usize), (7, 11), (4, 13)] {
        let len = (rng.next() % 400) as usize + 1;
        let data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let (orig, shards) = encode(&data, k, n).unwrap();
        for _ in 0..40 {
            // Pick a random subset of exactly k surviving indices.
            let mut all: Vec<usize> = (0..n).collect();
            // Fisher-Yates the first k slots.
            for s in 0..k {
                let pick = s + (rng.next() as usize) % (n - s);
                all.swap(s, pick);
            }
            let have: Vec<(usize, Vec<u8>)> =
                all[0..k].iter().map(|&i| (i, shards[i].clone())).collect();
            let got = decode(orig, k, n, &have).expect("reconstruct from random k-subset");
            assert_eq!(got, data, "k={k} n={n} survivors={:?}", &all[0..k]);
        }
    }
}

#[test]
fn fewer_than_k_shards_is_unrecoverable() {
    let data: Vec<u8> = (0..30u8).collect();
    let (orig, shards) = encode(&data, 5, 8).unwrap();
    let have: Vec<(usize, Vec<u8>)> = (0..4).map(|i| (i, shards[i].clone())).collect();
    assert!(
        decode(orig, 5, 8, &have).is_none(),
        "k-1 shards must not reconstruct"
    );
}

#[test]
fn reordered_and_duplicated_shards_still_reconstruct() {
    let data: Vec<u8> = (0..100u8).collect();
    let (k, n) = (6usize, 10usize);
    let (orig, shards) = encode(&data, k, n).unwrap();
    // Hand the decoder shards out of order, with duplicates, missing a few.
    let mut have: Vec<(usize, Vec<u8>)> = vec![
        (9, shards[9].clone()),
        (2, shards[2].clone()),
        (2, shards[2].clone()), // duplicate
        (7, shards[7].clone()),
        (0, shards[0].clone()),
        (5, shards[5].clone()),
        (5, shards[5].clone()), // duplicate
        (3, shards[3].clone()),
    ];
    have.reverse();
    let got = decode(orig, k, n, &have).expect("reorder + dup tolerant");
    assert_eq!(got, data);
}

#[test]
fn degenerate_parameters_are_rejected() {
    assert!(encode(b"x", 0, 3).is_none());
    assert!(encode(b"x", 4, 2).is_none()); // n < k
    assert!(encode(b"x", 1, 257).is_none()); // n > 256
}

// ---- G6 phase-2: the self-describing `redundant` framing layer -----------------

#[test]
fn framed_redundant_reconstructs_from_any_k_after_loss() {
    let payload: Vec<u8> = (0..250u8).cycle().take(777).collect();
    let (k, n) = (5usize, 8usize);
    let shards = frame_redundant(0xABCD, &payload, k, n).unwrap();
    assert_eq!(shards.len(), n, "one framed shard per n");
    // Deliver only k of them (drop 3), out of order.
    let delivered: Vec<Vec<u8>> = vec![
        shards[7].clone(),
        shards[1].clone(),
        shards[4].clone(),
        shards[0].clone(),
        shards[6].clone(),
    ];
    let (id, got) = reconstruct_redundant(&delivered).expect("reconstruct from k framed shards");
    assert_eq!(id, 0xABCD);
    assert_eq!(got, payload);
}

#[test]
fn framed_redundant_below_k_does_not_reconstruct() {
    let payload = b"the quick brown fox".to_vec();
    let shards = frame_redundant(7, &payload, 4, 7).unwrap();
    let delivered: Vec<Vec<u8>> = shards[0..3].to_vec(); // only 3 < 4
    assert!(reconstruct_redundant(&delivered).is_none());
}

#[test]
fn framed_redundant_ignores_other_messages_and_garbage() {
    let a = frame_redundant(1, b"message A payload here", 3, 5).unwrap();
    let b = frame_redundant(2, b"message B totally different", 3, 5).unwrap();
    // 2 shards of A (not enough), all of B, plus garbage. Only B reconstructs.
    let mut bag = vec![a[0].clone(), a[1].clone(), vec![0u8, 1, 2, 3], Vec::new()];
    bag.extend(b.iter().cloned());
    let (id, got) = reconstruct_redundant(&bag).unwrap();
    assert_eq!(id, 2);
    assert_eq!(got, b"message B totally different");
}

#[test]
fn framed_redundant_tolerates_duplicate_shards() {
    let payload: Vec<u8> = (0..200u8).collect();
    let shards = frame_redundant(99, &payload, 6, 10).unwrap();
    // 5 distinct (with duplicates) is still < k=6 — duplicates must not count.
    let mut bag = vec![
        shards[0].clone(),
        shards[0].clone(),
        shards[1].clone(),
        shards[1].clone(),
        shards[2].clone(),
        shards[3].clone(),
        shards[4].clone(),
    ];
    assert!(
        reconstruct_redundant(&bag).is_none(),
        "5 distinct (with dups) < k=6"
    );
    bag.push(shards[8].clone()); // the 6th distinct shard
    let (_, got) = reconstruct_redundant(&bag).expect("6 distinct shards reconstruct");
    assert_eq!(got, payload);
}
