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
    v.iter().enumerate().filter(|&(t, _)| t != i).map(|(_, &x)| x).collect()
}

/// The face map `dᵢ : Γ_n → Γ_{n-1}` over `A = Z/modulus`. A degree-`n` simplex is an `A`-labeling of
/// `S_n`; `dᵢ` sends basis `σ` to `σ∘δᵢ` when that stays surjective, else to `0`, summing collisions.
fn face(modulus: usize, n: usize, i: usize, x: &[usize]) -> Vec<usize> {
    let sn = surjections(n);
    let sn1 = surjections(n - 1);
    let idx: HashMap<Vec<u8>, usize> = sn1.iter().cloned().enumerate().map(|(k, v)| (v, k)).collect();
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
        .filter(|y| positions(n, k).iter().all(|&p| &face(modulus, n, p, y) == &horn[&p]))
        .count()
}

/// `|ker hₖ|` — degree-`n` simplices whose faces `i ≠ k` are all zero. Equals the number of fillers of
/// the *trivial* horn; `> 1` means a horn fills non-uniquely (genuine higher cells in that degree).
fn kernel_size(modulus: usize, n: usize, k: usize) -> usize {
    gamma(modulus, n)
        .iter()
        .filter(|y| positions(n, k).iter().all(|&p| face(modulus, n, p, y).iter().all(|&c| c == 0)))
        .count()
}

/// `|im hₖ|` — the number of distinct face-tuples `(dᵢ y)_{i≠k}` realized by some degree-`n` simplex.
fn image_size(modulus: usize, n: usize, k: usize) -> usize {
    let mut images: HashSet<Vec<Vec<usize>>> = HashSet::new();
    for y in gamma(modulus, n) {
        images.insert(positions(n, k).iter().map(|&p| face(modulus, n, p, &y)).collect());
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
            pos[ai + 1..].iter().all(|&b| face(modulus, n - 1, a, &horn[&b]) == face(modulus, n - 1, b - 1, &horn[&a]))
        });
        if ok {
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trivial_horn(n: usize, k: usize, modulus: usize) -> HashMap<usize, Vec<usize>> {
        // the horn whose given faces are all zero (lives in Γ_{n-1})
        let zero = vec![0usize; surjections(n - 1).len()];
        positions(n, k).iter().map(|&p| (p, zero.clone())).collect()
    }

    #[test]
    fn the_2_type_has_a_nonunique_inner_filler_so_pi2_is_nonzero() {
        // π₂ ≠ 0, INSIDE THE OBJECT. The inner horn Λ²₁ has |A| fillers — the 2-cells — not the unique
        // filler a 1-type forces. This is precisely the structure kan_complex proved K(G,1) lacks:
        // there every inner horn filled uniquely; here a whole copy of A fills the same horn.
        for modulus in [2usize, 3, 4] {
            let fillers = filler_count(modulus, 2, 1, &trivial_horn(2, 1, modulus));
            assert_eq!(fillers, modulus, "Λ²₁ has |A| = {modulus} fillers — π₂ = A, genuinely 2-dimensional");
            assert!(fillers > 1, "non-unique inner filler ⇒ NOT 1-truncated (this is what K(G,1) cannot do)");
        }
    }

    #[test]
    fn the_2_type_is_a_kan_complex_every_horn_fills() {
        // It IS a Kan complex (an ∞-groupoid): the face-tuple map hₖ is SURJECTIVE onto the compatible
        // horns in every degree we check (2, 3, 4) and position — so every horn fills. A simplicial
        // abelian group is Kan; we confirm it by counting, not by appeal.
        let modulus = 2;
        for (n, ks) in [(2usize, vec![0, 1, 2]), (3, vec![0, 1, 2, 3])] {
            for k in ks {
                assert_eq!(
                    image_size(modulus, n, k),
                    compatible_horn_count(modulus, n, k),
                    "Λ^{n}_{k} fills: image of hₖ = all compatible horns"
                );
            }
        }
        // degree 4, inner horns: still surjective (Kan)
        for k in 1..4 {
            assert_eq!(image_size(modulus, 4, k), compatible_horn_count(modulus, 4, k), "Λ⁴_{k} fills (Kan)");
        }
    }

    #[test]
    fn the_2_type_realizes_the_crossed_modules_admitted_pi2_admit_equals_have() {
        // SYMMETRY BREAKING, one level up — and the campaign's loop CLOSES. two_group's crossed module
        // A → 1 (g = trivial, h = A, ∂ = 0) ADMITS π₂ = ker∂ = A, algebraically. This K(A,2) Kan complex
        // HAS π₂ = A geometrically: the inner horn Λ²₁ has exactly |A| fillers. They are the SAME A — the
        // 2-group that admits the higher symmetry is realized by the object that carries it: admit = have.
        // And the |A| fillers form an A-TORSOR: breaking that symmetry (choosing the canonical filler) is
        // the very π₀ orbit-collapse this whole campaign began with, now acting on 2-cells. Symmetry
        // breaking runs from π₀ all the way up the tower.
        use crate::two_group::{CrossedModule, FiniteGroup};
        for m in [2usize, 3, 4] {
            let cm = CrossedModule { g: FiniteGroup::trivial(), h: FiniteGroup::cyclic(m), partial: vec![0; m] };
            let admitted = cm.pi_two().len(); // algebra: ker ∂ = A
            let had = filler_count(m, 2, 1, &trivial_horn(2, 1, m)); // geometry: # fillers of Λ²₁
            assert_eq!(admitted, had, "crossed module ADMITS π₂ = A ⟺ K(A,2) HAS π₂ = A (|A| fillers)");
            assert_eq!(admitted, m, "and both equal |A| = {m}");
        }
    }

    #[test]
    fn the_2_type_is_exactly_two_truncated() {
        // EXACTLY 2-truncated. At degree 2 the inner horn fills NON-uniquely (π₂ lives there: kernel = A).
        // By degree 4 the inner horns fill UNIQUELY again (kernel trivial) — there is no π₃, the higher
        // homotopy stops at level 2. So this object is a genuine 2-type: more than K(G,1), and no more.
        let modulus = 2;
        assert_eq!(kernel_size(modulus, 2, 1), modulus, "degree 2 inner: |ker| = |A| ⇒ π₂ = A");
        for k in 1..4 {
            assert_eq!(kernel_size(modulus, 4, k), 1, "degree 4 inner Λ⁴_{k}: unique filler ⇒ no π₃ (2-truncated)");
        }
    }
}
