//! Reed-Solomon erasure coding over GF(2^8) — the wire codec's `redundant` knob.
//!
//! Split a payload into `k` data shards plus `n − k` parity shards; a receiver
//! reconstructs the EXACT payload from ANY `k` of the `n` shards. This is the
//! reconstructable axis no general wire format ships: drop, duplicate, or reorder up to
//! `n − k` shards on a lossy link (UDP / multicast / BLE / LoRa) and the message still
//! arrives, with no retransmit and no coordination.
//!
//! The code is **systematic** (the first `k` shards ARE the data chunks, so lossless
//! delivery costs no decode) and **MDS** (any `k` shards suffice). The encoding matrix
//! is a Vandermonde matrix made systematic by multiplying through the inverse of its own
//! top `k×k` block: every `k`-row subset stays invertible, which is exactly the
//! any-`k`-of-`n` guarantee.

use std::sync::OnceLock;

/// GF(2^8) log/exp tables under the primitive polynomial x^8 + x^4 + x^3 + x^2 + 1
/// (0x11d). `exp` is doubled to 512 entries so a `log[a] + log[b]` index never wraps.
struct Gf {
    exp: [u8; 512],
    log: [u8; 256],
}

fn gf() -> &'static Gf {
    static GF: OnceLock<Gf> = OnceLock::new();
    GF.get_or_init(|| {
        let mut exp = [0u8; 512];
        let mut log = [0u8; 256];
        let mut x: u16 = 1;
        for i in 0..255 {
            exp[i] = x as u8;
            log[x as usize] = i as u8;
            x <<= 1;
            if x & 0x100 != 0 {
                x ^= 0x11d;
            }
        }
        for i in 255..512 {
            exp[i] = exp[i - 255];
        }
        Gf { exp, log }
    })
}

fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    let g = gf();
    g.exp[g.log[a as usize] as usize + g.log[b as usize] as usize]
}

/// GF division (`b` must be non-zero — every caller divides by a pivot).
fn div(a: u8, b: u8) -> u8 {
    if a == 0 {
        return 0;
    }
    let g = gf();
    g.exp[g.log[a as usize] as usize + 255 - g.log[b as usize] as usize]
}

/// `base^exp` in GF(2^8). `base^0 == 1` for every base (including 0).
fn pow(base: u8, exp: usize) -> u8 {
    let mut r = 1u8;
    for _ in 0..exp {
        r = mul(r, base);
    }
    r
}

/// Invert a `k×k` matrix over GF(2^8) by Gauss-Jordan elimination; `None` if singular.
fn invert(m: &[Vec<u8>]) -> Option<Vec<Vec<u8>>> {
    let k = m.len();
    // Augment with the identity: [m | I].
    let mut a: Vec<Vec<u8>> = m
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut r = row.clone();
            r.extend((0..k).map(|j| if i == j { 1 } else { 0 }));
            r
        })
        .collect();
    for col in 0..k {
        let mut piv = col;
        while piv < k && a[piv][col] == 0 {
            piv += 1;
        }
        if piv == k {
            return None; // singular
        }
        a.swap(col, piv);
        let inv_p = div(1, a[col][col]);
        for j in 0..2 * k {
            a[col][j] = mul(a[col][j], inv_p);
        }
        for row in 0..k {
            if row == col {
                continue;
            }
            let f = a[row][col];
            if f != 0 {
                for j in 0..2 * k {
                    a[row][j] ^= mul(f, a[col][j]);
                }
            }
        }
    }
    Some(a.iter().map(|r| r[k..2 * k].to_vec()).collect())
}

/// Multiply an `n×k` matrix by a `k×c` matrix over GF(2^8).
fn matmul(a: &[Vec<u8>], b: &[Vec<u8>]) -> Vec<Vec<u8>> {
    let inner = b.len();
    let cols = b[0].len();
    a.iter()
        .map(|row| {
            (0..cols)
                .map(|j| {
                    let mut acc = 0u8;
                    for t in 0..inner {
                        acc ^= mul(row[t], b[t][j]);
                    }
                    acc
                })
                .collect()
        })
        .collect()
}

/// The systematic `n×k` encoding matrix: a Vandermonde (`V[i][j] = i^j`, distinct nodes
/// `0..n` ⇒ any `k` rows invertible) multiplied through the inverse of its top `k×k`
/// block, so the first `k` rows become the identity (data passes through unchanged) while
/// the MDS property is preserved.
fn encode_matrix(n: usize, k: usize) -> Option<Vec<Vec<u8>>> {
    let vander: Vec<Vec<u8>> = (0..n)
        .map(|i| (0..k).map(|j| pow(i as u8, j)).collect())
        .collect();
    let top = vander[0..k].to_vec();
    let inv = invert(&top)?;
    Some(matmul(&vander, &inv))
}

/// Encode `data` into `n` shards (`k` data + `n − k` parity). Returns the original byte
/// length (for un-padding on decode) and the shards. `None` for a degenerate `(k, n)`
/// (`k == 0`, `n < k`, or `n > 256` — GF(2^8) has only 256 distinct nodes).
pub fn encode(data: &[u8], k: usize, n: usize) -> Option<(usize, Vec<Vec<u8>>)> {
    if k == 0 || n < k || n > 256 {
        return None;
    }
    let shard_len = data.len().div_ceil(k).max(1);
    let mut padded = data.to_vec();
    padded.resize(k * shard_len, 0);
    let e = encode_matrix(n, k)?;
    let shards = (0..n)
        .map(|i| {
            (0..shard_len)
                .map(|b| {
                    let mut acc = 0u8;
                    for j in 0..k {
                        acc ^= mul(e[i][j], padded[j * shard_len + b]);
                    }
                    acc
                })
                .collect()
        })
        .collect();
    Some((data.len(), shards))
}

/// Reconstruct the payload from ANY `k` (index, shard) pairs out of the `n`. `None` if
/// fewer than `k` distinct valid shards are present (then the message is unrecoverable).
pub fn decode(orig_len: usize, k: usize, n: usize, have: &[(usize, Vec<u8>)]) -> Option<Vec<u8>> {
    if k == 0 || n < k || n > 256 || have.is_empty() {
        return None;
    }
    let shard_len = have[0].1.len();
    let e = encode_matrix(n, k)?;
    let mut idxs: Vec<usize> = Vec::with_capacity(k);
    let mut rows: Vec<Vec<u8>> = Vec::with_capacity(k);
    let mut vals: Vec<&Vec<u8>> = Vec::with_capacity(k);
    for (idx, shard) in have {
        if *idx >= n || shard.len() != shard_len || idxs.contains(idx) {
            continue;
        }
        idxs.push(*idx);
        rows.push(e[*idx].clone());
        vals.push(shard);
        if idxs.len() == k {
            break;
        }
    }
    if idxs.len() < k {
        return None;
    }
    let minv = invert(&rows)?;
    let mut data = vec![0u8; k * shard_len];
    for b in 0..shard_len {
        for j in 0..k {
            let mut acc = 0u8;
            for r in 0..k {
                acc ^= mul(minv[j][r], vals[r][b]);
            }
            data[j * shard_len + b] = acc;
        }
    }
    if orig_len > data.len() {
        return None;
    }
    data.truncate(orig_len);
    Some(data)
}

// ---- The `redundant` framing layer: self-describing FEC shards ----------------------
//
// Each shard is independently transmittable (its own packet on a lossy link). A fixed
// header carries everything a receiver needs to group shards of the same message and
// reconstruct it — no external state, no ordering assumption.

const FEC_MAGIC: u8 = 0xFE;
// magic(1) + msg_id(8) + k(2) + n(2) + orig_len(4) + index(2)
const FEC_HEADER_LEN: usize = 1 + 8 + 2 + 2 + 4 + 2;

fn frame_shard(msg_id: u64, k: usize, n: usize, orig_len: usize, index: usize, shard: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(FEC_HEADER_LEN + shard.len());
    out.push(FEC_MAGIC);
    out.extend_from_slice(&msg_id.to_le_bytes());
    out.extend_from_slice(&(k as u16).to_le_bytes());
    out.extend_from_slice(&(n as u16).to_le_bytes());
    out.extend_from_slice(&(orig_len as u32).to_le_bytes());
    out.extend_from_slice(&(index as u16).to_le_bytes());
    out.extend_from_slice(shard);
    out
}

struct ShardHeader {
    msg_id: u64,
    k: usize,
    n: usize,
    orig_len: usize,
    index: usize,
}

fn parse_shard(bytes: &[u8]) -> Option<(ShardHeader, &[u8])> {
    if bytes.len() < FEC_HEADER_LEN || bytes[0] != FEC_MAGIC {
        return None;
    }
    let h = ShardHeader {
        msg_id: u64::from_le_bytes(bytes[1..9].try_into().ok()?),
        k: u16::from_le_bytes(bytes[9..11].try_into().ok()?) as usize,
        n: u16::from_le_bytes(bytes[11..13].try_into().ok()?) as usize,
        orig_len: u32::from_le_bytes(bytes[13..17].try_into().ok()?) as usize,
        index: u16::from_le_bytes(bytes[17..19].try_into().ok()?) as usize,
    };
    Some((h, &bytes[FEC_HEADER_LEN..]))
}

/// Peek a framed shard's identity without copying its payload: `(msg_id, k, n)`. `None`
/// if `bytes` is not a FEC shard (wrong magic / too short). Lets a receiver group shards
/// by message and know how many it needs (`k`) before reconstructing.
pub fn shard_header(bytes: &[u8]) -> Option<(u64, usize, usize)> {
    parse_shard(bytes).map(|(h, _)| (h.msg_id, h.k, h.n))
}

/// Split a message into `n` self-describing FEC shards (`k` data + `n − k` parity),
/// each independently transmittable. A receiver reconstructs the exact message from
/// ANY `k` via [`reconstruct_redundant`]. `msg_id` groups a message's shards on the wire.
pub fn frame_redundant(msg_id: u64, payload: &[u8], k: usize, n: usize) -> Option<Vec<Vec<u8>>> {
    let (orig_len, shards) = encode(payload, k, n)?;
    Some(
        shards
            .iter()
            .enumerate()
            .map(|(i, s)| frame_shard(msg_id, k, n, orig_len, i, s))
            .collect(),
    )
}

/// Reconstruct a message from a bag of received FEC shards. Groups by message id and
/// reconstructs the first message that has at least `k` distinct shards present; returns
/// its `(msg_id, payload)`. Shards from other messages, malformed shards, header-
/// inconsistent shards, and duplicate indices are ignored. `None` if no message has
/// reached its `k`-shard threshold.
pub fn reconstruct_redundant(received: &[Vec<u8>]) -> Option<(u64, Vec<u8>)> {
    use std::collections::HashMap;
    // msg_id -> (k, n, orig_len, [(index, shard_bytes)])
    let mut groups: HashMap<u64, (usize, usize, usize, Vec<(usize, Vec<u8>)>)> = HashMap::new();
    for bytes in received {
        let Some((h, shard)) = parse_shard(bytes) else { continue };
        let entry = groups.entry(h.msg_id).or_insert((h.k, h.n, h.orig_len, Vec::new()));
        // Accept only header-consistent shards with a not-yet-seen index.
        if entry.0 == h.k
            && entry.1 == h.n
            && entry.2 == h.orig_len
            && !entry.3.iter().any(|(i, _)| *i == h.index)
        {
            entry.3.push((h.index, shard.to_vec()));
        }
    }
    for (msg_id, (k, n, orig_len, shards)) in groups {
        if shards.len() >= k {
            if let Some(data) = decode(orig_len, k, n, &shards) {
                return Some((msg_id, data));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
        for &(k, n) in &[(4usize, 6usize), (6, 10), (3, 5), (8, 12), (1, 3), (10, 16), (2, 9)] {
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
        assert!(decode(orig, 5, 8, &have).is_none(), "k-1 shards must not reconstruct");
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
        assert!(reconstruct_redundant(&bag).is_none(), "5 distinct (with dups) < k=6");
        bag.push(shards[8].clone()); // the 6th distinct shard
        let (_, got) = reconstruct_redundant(&bag).expect("6 distinct shards reconstruct");
        assert_eq!(got, payload);
    }
}
