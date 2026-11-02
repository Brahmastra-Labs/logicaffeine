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

// GF(2^8) field arithmetic (mul/div/pow, matrix invert/matmul; primitive polynomial 0x11d) is the
// shared substrate in `logicaffeine_base::gf256` — the same field the multivariate crypto and the
// XL cryptanalysis engine build on. Reed-Solomon uses only its multiplicative structure.
use logicaffeine_base::gf256::{invert, matmul, mul, pow};

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

fn frame_shard(
    msg_id: u64,
    k: usize,
    n: usize,
    orig_len: usize,
    index: usize,
    shard: &[u8],
) -> Vec<u8> {
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
        let Some((h, shard)) = parse_shard(bytes) else {
            continue;
        };
        let entry = groups
            .entry(h.msg_id)
            .or_insert((h.k, h.n, h.orig_len, Vec::new()));
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
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/concurrency/fec/tests.rs"]
mod tests;
