use super::*;
use crate::codegen_sva::unified_placement::Row;

fn lcg(seed: u64) -> impl FnMut() -> u64 {
    let mut x = seed;
    move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }
}

fn from_planes(planes: &[Row], lanes: usize) -> Vec<u64> {
    (0..lanes)
        .map(|j| {
            let mut v = 0u64;
            for (i, p) in planes.iter().enumerate() {
                if (p[0] >> j) & 1 == 1 {
                    v |= 1u64 << i;
                }
            }
            v
        })
        .collect()
}

/// Per-lane popcount over K fuzzed planes equals the reference bit count, across all 64 lanes.
#[test]
fn popcount_counts_set_bits_per_lane() {
    let k = 20usize;
    let out_width = 5usize; // holds 0..=31 ≥ 20
    let lanes = 64usize;
    let mut rng = lcg(0x9001_c0de_7712_1234);
    for _ in 0..30 {
        let data: Vec<u64> = (0..k).map(|_| rng()).collect();
        let mut df = DramDataflow::new(k + 1);
        let plane_idx: Vec<usize> = (0..k).collect();
        let zero = k;
        let cnt_idx = popcount(&mut df, &plane_idx, out_width, zero);

        let mut inputs: Vec<Row> = data.iter().map(|&w| vec![w]).collect();
        inputs.push(vec![0u64]);
        let heap = df.run(&inputs, 1);

        let cnt_planes: Vec<Row> = cnt_idx.iter().map(|&r| heap[r].clone()).collect();
        let got = from_planes(&cnt_planes, lanes);
        for lane in 0..lanes {
            let expected = (0..k).filter(|&j| (data[j] >> lane) & 1 == 1).count() as u64;
            assert_eq!(got[lane], expected, "lane {lane}");
        }
    }
}

/// Hamming distance = popcount(a XOR b), per lane, equals the reference across 64 lanes — the
/// similarity-search / binary-net kernel.
#[test]
fn hamming_distance_equals_reference() {
    let bits = 16usize;
    let out_width = 5usize; // holds 0..=31 ≥ 16
    let lanes = 64usize;
    let mut rng = lcg(0x4a33_1c0d_e999_0001);
    for _ in 0..30 {
        let acode: Vec<u64> = (0..lanes).map(|_| rng() % (1 << bits)).collect();
        let bcode: Vec<u64> = (0..lanes).map(|_| rng() % (1 << bits)).collect();
        // bit-serial layout: plane j bit i = bit j of code[i]
        let to_planes = |codes: &[u64]| -> Vec<Row> {
            (0..bits)
                .map(|j| {
                    let mut w = 0u64;
                    for (i, &c) in codes.iter().enumerate() {
                        if (c >> j) & 1 == 1 {
                            w |= 1u64 << i;
                        }
                    }
                    vec![w]
                })
                .collect()
        };
        let mut df = DramDataflow::new(2 * bits + 1);
        let a_idx: Vec<usize> = (0..bits).collect();
        let b_idx: Vec<usize> = (bits..2 * bits).collect();
        let zero = 2 * bits;
        let d_idx = hamming_distance(&mut df, &a_idx, &b_idx, out_width, zero);

        let mut inputs: Vec<Row> = Vec::new();
        inputs.extend(to_planes(&acode));
        inputs.extend(to_planes(&bcode));
        inputs.push(vec![0u64]);
        let heap = df.run(&inputs, 1);

        let d_planes: Vec<Row> = d_idx.iter().map(|&r| heap[r].clone()).collect();
        let got = from_planes(&d_planes, lanes);
        for lane in 0..lanes {
            let expected = (acode[lane] ^ bcode[lane]).count_ones() as u64;
            assert_eq!(
                got[lane], expected,
                "lane {lane}: hamming({}, {})",
                acode[lane], bcode[lane]
            );
        }
    }
}

/// Agreement count = popcount(a XNOR b) = K − hamming, per lane — the 1-bit-LLM dot-product half.
#[test]
fn agreement_count_is_the_binary_dot_product() {
    let bits = 16usize;
    let out_width = 5usize;
    let lanes = 64usize;
    let mut rng = lcg(0xd07_b17_9999_0001);
    for _ in 0..30 {
        let acode: Vec<u64> = (0..lanes).map(|_| rng() % (1 << bits)).collect();
        let bcode: Vec<u64> = (0..lanes).map(|_| rng() % (1 << bits)).collect();
        let to_planes = |codes: &[u64]| -> Vec<Row> {
            (0..bits)
                .map(|j| {
                    let mut w = 0u64;
                    for (i, &c) in codes.iter().enumerate() {
                        if (c >> j) & 1 == 1 {
                            w |= 1u64 << i;
                        }
                    }
                    vec![w]
                })
                .collect()
        };
        let mut df = DramDataflow::new(2 * bits + 1);
        let a_idx: Vec<usize> = (0..bits).collect();
        let b_idx: Vec<usize> = (bits..2 * bits).collect();
        let zero = 2 * bits;
        let g_idx = agreement_count(&mut df, &a_idx, &b_idx, out_width, zero);

        let mut inputs: Vec<Row> = Vec::new();
        inputs.extend(to_planes(&acode));
        inputs.extend(to_planes(&bcode));
        inputs.push(vec![0u64]);
        let heap = df.run(&inputs, 1);

        let g_planes: Vec<Row> = g_idx.iter().map(|&r| heap[r].clone()).collect();
        let got = from_planes(&g_planes, lanes);
        for lane in 0..lanes {
            // agreements = #bits equal = K − hamming, over the `bits`-wide code
            let disagree = (acode[lane] ^ bcode[lane]).count_ones() as u64;
            assert_eq!(got[lane], bits as u64 - disagree, "lane {lane}");
        }
    }
}

/// BitNet ternary·binary dot product = popcount(Wp&X) − popcount(Wn&X), per lane, equals the
/// reference Σ w[k]·x[k] (two's complement) — a 1.58-bit LLM matmul row with no multiply.
#[test]
fn ternary_binary_dot_equals_reference_matmul_row() {
    let k = 24usize; // vector dimension (planes)
    let out_width = 7usize; // holds signed [-64,63] ⊇ [-24,24]
    let lanes = 64usize;
    let outmod = 1u64 << out_width;
    let mut rng = lcg(0xb17_9e7_c0de_0001);
    for _ in 0..30 {
        // per plane: wp and wn are disjoint (a weight is +1, −1, or 0)
        let wp_d: Vec<u64> = (0..k).map(|_| rng()).collect();
        let wn_d: Vec<u64> = (0..k).map(|j| rng() & !wp_d[j]).collect(); // disjoint from wp
        let x_d: Vec<u64> = (0..k).map(|_| rng()).collect();

        let mut df = DramDataflow::new(3 * k + 2);
        let wp_i: Vec<usize> = (0..k).collect();
        let wn_i: Vec<usize> = (k..2 * k).collect();
        let x_i: Vec<usize> = (2 * k..3 * k).collect();
        let zero = 3 * k;
        let ones = 3 * k + 1;
        let dot_i = ternary_binary_dot(&mut df, &wp_i, &wn_i, &x_i, out_width, zero, ones);

        let mut inputs: Vec<Row> = Vec::new();
        inputs.extend(wp_d.iter().map(|&w| vec![w]));
        inputs.extend(wn_d.iter().map(|&w| vec![w]));
        inputs.extend(x_d.iter().map(|&w| vec![w]));
        inputs.push(vec![0u64]); // zero
        inputs.push(vec![!0u64]); // ones (carry-in for the subtract)
        let heap = df.run(&inputs, 1);

        let dot_planes: Vec<Row> = dot_i.iter().map(|&r| heap[r].clone()).collect();
        let got = from_planes(&dot_planes, lanes);
        for lane in 0..lanes {
            let pos = (0..k)
                .filter(|&j| (wp_d[j] >> lane) & 1 == 1 && (x_d[j] >> lane) & 1 == 1)
                .count() as i64;
            let neg = (0..k)
                .filter(|&j| (wn_d[j] >> lane) & 1 == 1 && (x_d[j] >> lane) & 1 == 1)
                .count() as i64;
            let net = pos - neg; // in [-k, k]
            let expected = net.rem_euclid(outmod as i64) as u64; // two's complement
            assert_eq!(got[lane], expected, "lane {lane}: pos={pos} neg={neg}");
        }
    }
}
