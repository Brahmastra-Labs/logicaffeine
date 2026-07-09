//! Actually **having** an ∞-groupoid — not its invariants, the object itself, with its defining
//! property machine-verified.
//!
//! Everything in `progress_complex`/`cubical` computed *invariants* (Betti numbers) of *spaces*. That is
//! not the same as possessing the ∞-groupoid: by the homotopy hypothesis the simplicial model of an
//! ∞-groupoid is a **Kan complex** — a simplicial set in which every *horn* (a simplex with one face
//! missing) can be filled. Here we build such an object directly: the **nerve of a group**, whose
//! degree-`n` simplices are `Gⁿ` (chains of composable arrows), and we *verify the Kan condition* —
//! every horn fills — in degrees 2 and 3. The nerve of `Aut(F)` is exactly `BG = K(Aut(F),1)`, the
//! classifying space the symmetry tower named, so this is that ∞-groupoid, now had as an object.
//!
//! It is honestly the **1-truncated** one: inner horns fill *uniquely*, the hallmark of the nerve of a
//! 1-groupoid (no genuine higher cells). A non-truncated ∞-groupoid — nontrivial `π_{≥2}`, inner horns
//! with *several* fillers, the homology ladder's higher `πₙ` assembled into one coherent complex with
//! its k-invariants — is the frontier this does NOT yet reach, and we say so in the test that proves the
//! uniqueness.

use crate::two_group::FiniteGroup;

/// The `i`-th face `d_i` of a degree-`n` simplex `(g_1, …, g_n) ∈ Gⁿ` of the nerve: drop the first arrow
/// (`i=0`), drop the last (`i=n`), or compose the adjacent pair (`0 < i < n`).
fn face(g: &FiniteGroup, s: &[usize], i: usize) -> Vec<usize> {
    let n = s.len();
    if i == 0 {
        return s[1..].to_vec();
    }
    if i == n {
        return s[..n - 1].to_vec();
    }
    let mut out = s.to_vec();
    out[i - 1] = g.mul[s[i - 1]][s[i]];
    out.remove(i);
    out
}

/// Every degree-`n` simplex of the nerve — all of `Gⁿ`.
fn all_simplices(g: &FiniteGroup, n: usize) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = vec![vec![]];
    for _ in 0..n {
        let mut next = Vec::with_capacity(out.len() * g.order());
        for s in &out {
            for e in 0..g.order() {
                let mut t = s.clone();
                t.push(e);
                next.push(t);
            }
        }
        out = next;
    }
    out
}

/// Enumerate every **compatible horn** `Λⁿ_k` — an assignment of a degree-`(n-1)` simplex to each face
/// position `i ≠ k` satisfying the simplicial identities `d_i x_j = d_{j-1} x_i` (`i < j`) — and call
/// `each(filler_count)` with how many degree-`n` simplices fill it.
fn for_each_horn(g: &FiniteGroup, n: usize, k: usize, mut each: impl FnMut(usize)) {
    let lower = all_simplices(g, n - 1);
    let tops = all_simplices(g, n);
    let positions: Vec<usize> = (0..=n).filter(|&i| i != k).collect();
    let base = lower.len();
    let total: u128 = (base as u128).pow(positions.len() as u32);
    for idx in 0..total {
        // decode the assignment of a lower simplex to each position
        let mut x = idx;
        let horn: Vec<(usize, &Vec<usize>)> = positions
            .iter()
            .map(|&p| {
                let a = (x % base as u128) as usize;
                x /= base as u128;
                (p, &lower[a])
            })
            .collect();
        // compatibility: d_i x_j = d_{j-1} x_i for i < j
        let compatible = horn.iter().enumerate().all(|(a, &(i, xi))| {
            horn[a + 1..].iter().all(|&(j, xj)| face(g, xj, i) == face(g, xi, j - 1))
        });
        if !compatible {
            continue;
        }
        let fillers = tops.iter().filter(|y| horn.iter().all(|&(p, xp)| &face(g, y, p) == xp)).count();
        each(fillers);
    }
}

/// Does the nerve satisfy the **Kan condition** for horns `Λⁿ_k` — does every compatible horn fill?
pub fn kan_fills(g: &FiniteGroup, n: usize, k: usize) -> bool {
    let mut ok = true;
    for_each_horn(g, n, k, |fillers| ok &= fillers >= 1);
    ok
}

/// Does every compatible **inner** horn (`0 < k < n`) fill *uniquely*? Unique inner fillers are the
/// signature of a 1-truncated nerve — the ∞-groupoid is exactly `K(G,1) = BG`, no genuine higher cells.
pub fn inner_horns_fill_uniquely(g: &FiniteGroup, n: usize, k: usize) -> bool {
    assert!(0 < k && k < n, "inner horns only");
    let mut unique = true;
    for_each_horn(g, n, k, |fillers| unique &= fillers == 1);
    unique
}

/// All `A`-valued cochains on `BG_k` — functions `BG_k = Gᵏ → A = Z/modulus`.
fn nerve_cochains(g: &FiniteGroup, modulus: usize, k: usize) -> Vec<Vec<usize>> {
    let size = all_simplices(g, k).len();
    let mut out: Vec<Vec<usize>> = vec![vec![]];
    for _ in 0..size {
        let mut next = Vec::with_capacity(out.len() * modulus);
        for t in &out {
            for v in 0..modulus {
                let mut u = t.clone();
                u.push(v);
                next.push(u);
            }
        }
        out = next;
    }
    out
}

/// Simplicial coboundary `δ : Cᵏ(BG; A) → Cᵏ⁺¹` from the **nerve's own faces**:
/// `(δφ)(σ) = Σᵢ (−1)ⁱ φ(dᵢ σ)`.
fn nerve_coboundary(g: &FiniteGroup, modulus: usize, k: usize, phi: &[usize]) -> Vec<usize> {
    let sk = all_simplices(g, k);
    let idx: std::collections::HashMap<Vec<usize>, usize> =
        sk.iter().cloned().enumerate().map(|(i, s)| (s, i)).collect();
    let skp1 = all_simplices(g, k + 1);
    let mut out = vec![0usize; skp1.len()];
    for (j, sigma) in skp1.iter().enumerate() {
        let mut val = 0i64;
        for i in 0..=(k + 1) {
            let f = face(g, sigma, i);
            let s = if i % 2 == 0 { 1 } else { -1 };
            val += s * phi[idx[&f]] as i64;
        }
        out[j] = val.rem_euclid(modulus as i64) as usize;
    }
    out
}

/// The simplicial cohomology `|Hⁿ(BG; A)|` of the nerve `BG = K(G,1)`, computed from the nerve's own
/// face maps. By the Eilenberg–MacLane representing property `Hⁿ(BG; A) = [BG, K(A,n)]`; we confirm it
/// equals the algebraic **group cohomology** `Hⁿ(G; A)`, so the spectrum represents — on the symmetry's
/// classifying space — exactly group cohomology, the home of the Postnikov k-invariants.
pub fn nerve_cohomology_size(g: &FiniteGroup, modulus: usize, n: usize) -> usize {
    let cocycles = nerve_cochains(g, modulus, n)
        .into_iter()
        .filter(|phi| nerve_coboundary(g, modulus, n, phi).iter().all(|&x| x == 0))
        .count();
    let mut coboundaries: std::collections::HashSet<Vec<usize>> = std::collections::HashSet::new();
    for psi in nerve_cochains(g, modulus, n - 1) {
        coboundaries.insert(nerve_coboundary(g, modulus, n - 1, &psi));
    }
    cocycles / coboundaries.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spectrum_represents_cohomology_nerve_cohomology_is_group_cohomology() {
        // K(A,n) REPRESENTS cohomology: Hⁿ(X;A) = [X, K(A,n)]. Instantiated on the symmetry's classifying
        // space X = BG = K(G,1): Hⁿ(BG;A) = [BG, K(A,n)]. We compute Hⁿ(BG;A) from the NERVE's own face
        // maps and confirm it equals the algebraic group cohomology Hⁿ(G;A) — so what the Eilenberg–
        // MacLane spectrum represents on BG is exactly group cohomology, the home of the k-invariants.
        for n in 2..=3 {
            assert_eq!(
                nerve_cohomology_size(&FiniteGroup::cyclic(2), 2, n),
                crate::postnikov::cohomology_size(&FiniteGroup::cyclic(2), 2, n),
                "Hⁿ(BG; Z/2) from the nerve = group cohomology Hⁿ(Z/2; Z/2)"
            );
        }
        // a second group: H²(BZ/3; Z/2) from the nerve = group cohomology H²(Z/3; Z/2)
        assert_eq!(
            nerve_cohomology_size(&FiniteGroup::cyclic(3), 2, 2),
            crate::postnikov::cohomology_size(&FiniteGroup::cyclic(3), 2, 2),
            "the representing property holds for BZ/3 too"
        );
    }

    #[test]
    fn the_nerve_is_a_kan_complex_so_we_actually_have_an_infinity_groupoid() {
        // We stop computing invariants and build the ∞-GROUPOID OBJECT: the nerve of a group, a
        // simplicial set, and VERIFY its defining property — the Kan horn-filling condition — in degrees
        // 2 and 3, for EVERY horn (inner and outer). A Kan complex IS an ∞-groupoid (homotopy
        // hypothesis). The nerve of Aut(F) is BG = K(Aut(F),1), so this is the very ∞-groupoid the
        // symmetry tower named — now possessed as an object, not merely measured by its Betti numbers.
        for g in [FiniteGroup::cyclic(2), FiniteGroup::cyclic(3), FiniteGroup::symmetric(3)] {
            for n in 2..=3 {
                for k in 0..=n {
                    assert!(kan_fills(&g, n, k), "the nerve fills every horn Λ^n_k — it is a Kan complex");
                }
            }
        }
    }

    #[test]
    fn it_is_exactly_one_truncated_so_honestly_its_only_K_G_1_for_now() {
        // HONEST SCOPE — the test that keeps us truthful. The ∞-groupoid we now have is precisely the
        // 1-TRUNCATED one: every inner horn fills UNIQUELY, the hallmark of the nerve of a 1-groupoid
        // (no genuine higher cells, π_{≥2} = 0). A non-truncated ∞-groupoid would have inner horns with
        // MULTIPLE fillers. Assembling the homology ladder's higher πₙ into one coherent Kan complex with
        // its k-invariants — a genuinely 2+-truncated object — is the frontier we do NOT yet hold.
        for g in [FiniteGroup::cyclic(2), FiniteGroup::cyclic(3), FiniteGroup::symmetric(3)] {
            for n in 2..=3 {
                for k in 1..n {
                    assert!(inner_horns_fill_uniquely(&g, n, k), "inner horns unique ⇒ 1-truncated = K(G,1)");
                }
            }
        }
    }
}
