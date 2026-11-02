//! A genuinely **2-truncated** ∞-groupoid, had as an object: the minimal `K(A, 2)`.
//!
//! `kan_complex` built the nerve `BG = K(G, 1)` and proved it 1-truncated (inner horns fill *uniquely*).
//! This is the first object *beyond* that — a Kan complex with real `π₂ ≠ 0` living inside it, not merely
//! admitted by a crossed module or glimpsed as homology. It is the Eilenberg–MacLane space `K(A, 2)`:
//! `π₂ = A`, every other `πₙ = 0`.
//!
//! The model is Dold–Kan `Γ(A[2])` for the chain complex with `A` in degree 2. Its `n`-simplices are the
//! `A`-linear combinations of order-preserving **surjections `[n] ↠ [2]`** (since the complex is
//! concentrated in degree 2, a face that fails to stay surjective dies — `C₁ = C₃ = 0`). Concretely an
//! `n`-simplex is an `A`-labeling of `S_n = {surjections [n] ↠ [2]}`, and `dᵢ` pulls back along the
//! `i`-th coface, keeping only the still-surjective terms. It is a simplicial *abelian group*, hence
//! automatically a Kan complex — and we verify, by enumeration:
//!
//! - **`π₂ ≠ 0`**: the inner horn `Λ²₁` has `|A|` fillers (the `2`-cells), *not* the unique filler of a
//!   1-type. This is the structure `K(G,1)` provably lacked.
//! - **Kan**: every horn fills (the face-tuple map onto the compatible-horn object is surjective), checked
//!   in degrees 2, 3, 4.
//! - **exactly 2-truncated**: degree-4 inner horns fill *uniquely* again (no `π₃`), so the higher homotopy
//!   stops at level 2.

use std::collections::{HashMap, HashSet};

/// All order-preserving surjections `[n] ↠ [2]`, as nondecreasing length-`(n+1)` value vectors hitting
/// `0, 1, 2`. `S_n` — the basis of the degree-`n` part. Empty for `n < 2` (so `Γ₀ = Γ₁ = 0`).
fn surjections(n: usize) -> Vec<Vec<u8>> {
    fn rec(len: usize, pos: usize, last: u8, cur: &mut Vec<u8>, out: &mut Vec<Vec<u8>>) {
        if pos == len {
            if cur.contains(&0) && cur.contains(&1) && cur.contains(&2) {
                out.push(cur.clone());
            }
            return;
        }
        for v in last..=2 {
            cur.push(v);
            rec(len, pos + 1, v, cur, out);
            cur.pop();
        }
    }
    let mut out = Vec::new();
    rec(n + 1, 0, 0, &mut Vec::new(), &mut out);
    out
}

fn is_surjective(v: &[u8]) -> bool {
    v.contains(&0) && v.contains(&1) && v.contains(&2)
}

/// `σ ∘ δᵢ` — precompose a surjection with the `i`-th coface, i.e. drop coordinate `i`.
fn drop_coord(v: &[u8], i: usize) -> Vec<u8> {
    v.iter()
        .enumerate()
        .filter(|&(t, _)| t != i)
        .map(|(_, &x)| x)
        .collect()
}

/// The face map `dᵢ : Γ_n → Γ_{n-1}` over `A = Z/modulus`. A degree-`n` simplex is an `A`-labeling of
/// `S_n`; `dᵢ` sends basis `σ` to `σ∘δᵢ` when that stays surjective, else to `0`, summing collisions.
fn face(modulus: usize, n: usize, i: usize, x: &[usize]) -> Vec<usize> {
    let sn = surjections(n);
    let sn1 = surjections(n - 1);
    let idx: HashMap<Vec<u8>, usize> = sn1
        .iter()
        .cloned()
        .enumerate()
        .map(|(k, v)| (v, k))
        .collect();
    let mut out = vec![0usize; sn1.len()];
    for (k, sigma) in sn.iter().enumerate() {
        let d = drop_coord(sigma, i);
        if is_surjective(&d) {
            let t = idx[&d];
            out[t] = (out[t] + x[k]) % modulus;
        }
    }
    out
}

/// Every degree-`n` simplex — all `A`-labelings of `S_n` (`|A|^{|S_n|}` of them).
fn gamma(modulus: usize, n: usize) -> Vec<Vec<usize>> {
    let size = surjections(n).len();
    let mut out: Vec<Vec<usize>> = vec![vec![]];
    for _ in 0..size {
        let mut next = Vec::with_capacity(out.len() * modulus);
        for t in &out {
            for e in 0..modulus {
                let mut u = t.clone();
                u.push(e);
                next.push(u);
            }
        }
        out = next;
    }
    out
}

fn positions(n: usize, k: usize) -> Vec<usize> {
    (0..=n).filter(|&i| i != k).collect()
}

/// The number of degree-`n` simplices filling the horn `Λⁿ_k` whose given faces are all the chosen
/// `horn[i]` (`i ≠ k`).
fn filler_count(modulus: usize, n: usize, k: usize, horn: &HashMap<usize, Vec<usize>>) -> usize {
    gamma(modulus, n)
        .iter()
        .filter(|y| {
            positions(n, k)
                .iter()
                .all(|&p| &face(modulus, n, p, y) == &horn[&p])
        })
        .count()
}

/// `|ker hₖ|` — degree-`n` simplices whose faces `i ≠ k` are all zero. Equals the number of fillers of
/// the *trivial* horn; `> 1` means a horn fills non-uniquely (genuine higher cells in that degree).
fn kernel_size(modulus: usize, n: usize, k: usize) -> usize {
    gamma(modulus, n)
        .iter()
        .filter(|y| {
            positions(n, k)
                .iter()
                .all(|&p| face(modulus, n, p, y).iter().all(|&c| c == 0))
        })
        .count()
}

/// `|im hₖ|` — the number of distinct face-tuples `(dᵢ y)_{i≠k}` realized by some degree-`n` simplex.
fn image_size(modulus: usize, n: usize, k: usize) -> usize {
    let mut images: HashSet<Vec<Vec<usize>>> = HashSet::new();
    for y in gamma(modulus, n) {
        images.insert(
            positions(n, k)
                .iter()
                .map(|&p| face(modulus, n, p, &y))
                .collect(),
        );
    }
    images.len()
}

/// The number of **compatible horns** `Λⁿ_k`: tuples `(x_i)_{i≠k}` of degree-`(n-1)` simplices satisfying
/// the simplicial identities `d_a x_b = d_{b-1} x_a` (`a < b`). The horn maps `hₖ` is onto these iff Kan.
fn compatible_horn_count(modulus: usize, n: usize, k: usize) -> usize {
    let pos = positions(n, k);
    let lower = gamma(modulus, n - 1);
    let base = lower.len();
    let total: u128 = (base as u128).pow(pos.len() as u32);
    let mut count = 0;
    for idx in 0..total {
        let mut x = idx;
        let horn: HashMap<usize, Vec<usize>> = pos
            .iter()
            .map(|&p| {
                let a = (x % base as u128) as usize;
                x /= base as u128;
                (p, lower[a].clone())
            })
            .collect();
        // simplicial identity d_a x_b = d_{b-1} x_a for every pair a < b of given positions
        let ok = pos.iter().enumerate().all(|(ai, &a)| {
            pos[ai + 1..].iter().all(|&b| {
                face(modulus, n - 1, a, &horn[&b]) == face(modulus, n - 1, b - 1, &horn[&a])
            })
        });
        if ok {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/two_type/tests.rs"]
mod tests;
