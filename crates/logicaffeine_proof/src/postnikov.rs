//! The **Postnikov `k`-invariant** — the gluing data of a 2-type, and the obstruction to breaking it
//! into a product.
//!
//! `two_type` built `K(A,2)` and `kan_complex` built `K(G,1)`. A general 2-truncated ∞-groupoid is *not*
//! just a `π₁` and a `π₂` sitting side by side: by Mac Lane–Whitehead / Sinh's theorem it is classified
//! by `(π₁ = G, π₂ = A, the G-action, k ∈ H³(G; A))`. The class `k` — the **Postnikov invariant** — is
//! the associator of the corresponding 2-group, and it says how the two levels are *twisted*:
//!
//! - `k = 0` ⟺ the 2-type is the **product** `K(G,1) × K(A,2)` — the levels decouple.
//! - `k ≠ 0` ⟺ a genuinely **entangled** 2-type, provably *not* a product.
//!
//! So `k` is "the single object carrying all `πₙ` *with their interactions*" reduced to one checkable
//! cohomology class. And it is, once more, **a symmetry-breaking obstruction**: splitting the 2-type into
//! a product is the ultimate symmetry break (decoupling `π₁` from `π₂`), and `k ≠ 0` is exactly the
//! obstruction to it — the higher sibling of the deadlock `β₁`. We compute group cohomology directly
//! (`A = Z/modulus`, trivial action) and crush the canonical case `H³(Z/2; Z/2) = Z/2`: the cup-cube
//! cocycle is a 3-cocycle that is *not* a coboundary, so its 2-type is no product.

use crate::two_group::FiniteGroup;

fn idx(order: usize, t: &[usize]) -> usize {
    t.iter().fold(0usize, |a, &x| a * order + x)
}

fn tuples(order: usize, n: usize) -> Vec<Vec<usize>> {
    let mut out: Vec<Vec<usize>> = vec![vec![]];
    for _ in 0..n {
        let mut next = Vec::with_capacity(out.len() * order);
        for t in &out {
            for e in 0..order {
                let mut u = t.clone();
                u.push(e);
                next.push(u);
            }
        }
        out = next;
    }
    out
}

/// Every `n`-cochain `Gⁿ → A` (`A = Z/modulus`): `modulus^{order^n}` of them.
fn all_cochains(order: usize, modulus: usize, n: usize) -> Vec<Vec<usize>> {
    let len = order.pow(n as u32);
    let mut out: Vec<Vec<usize>> = vec![vec![]];
    for _ in 0..len {
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

/// The group-cohomology coboundary `δⁿ : Cⁿ(G; A) → Cⁿ⁺¹(G; A)`, with `A = Z/modulus` and **trivial**
/// `G`-action. `f` is a length-`order^n` table; the result is length `order^{n+1}`.
pub fn coboundary(g: &FiniteGroup, modulus: usize, n: usize, f: &[usize]) -> Vec<usize> {
    let order = g.order();
    let mut out = vec![0usize; order.pow((n + 1) as u32)];
    for t in tuples(order, n + 1) {
        // (δf)(g_1,…,g_{n+1}) = f(g_2,…,g_{n+1}) + Σ_{i=1}^n (−1)^i f(…,g_i g_{i+1},…) + (−1)^{n+1} f(g_1,…,g_n)
        let mut val: i64 = f[idx(order, &t[1..])] as i64;
        for i in 1..=n {
            let mut merged = t[..i - 1].to_vec();
            merged.push(g.mul[t[i - 1]][t[i]]);
            merged.extend_from_slice(&t[i + 1..]);
            let sign = if i % 2 == 0 { 1 } else { -1 };
            val += sign * f[idx(order, &merged)] as i64;
        }
        let sign = if (n + 1) % 2 == 0 { 1 } else { -1 };
        val += sign * f[idx(order, &t[..n])] as i64;
        out[idx(order, &t)] = val.rem_euclid(modulus as i64) as usize;
    }
    out
}

/// Is `f` an `n`-cocycle? (`δf = 0`)
pub fn is_cocycle(g: &FiniteGroup, modulus: usize, n: usize, f: &[usize]) -> bool {
    coboundary(g, modulus, n, f).iter().all(|&x| x == 0)
}

/// Is `f` an `n`-coboundary? (`f = δβ` for some `(n-1)`-cochain `β`)
pub fn is_coboundary(g: &FiniteGroup, modulus: usize, n: usize, f: &[usize]) -> bool {
    all_cochains(g.order(), modulus, n - 1)
        .iter()
        .any(|b| coboundary(g, modulus, n - 1, b) == f)
}

/// `|Hⁿ(G; A)|` — cocycles modulo coboundaries (both are subgroups of `Cⁿ`, so the order is the index).
pub fn cohomology_size(g: &FiniteGroup, modulus: usize, n: usize) -> usize {
    let cocycles = all_cochains(g.order(), modulus, n)
        .into_iter()
        .filter(|f| is_cocycle(g, modulus, n, f))
        .count();
    let mut coboundaries: std::collections::HashSet<Vec<usize>> = std::collections::HashSet::new();
    for b in all_cochains(g.order(), modulus, n - 1) {
        coboundaries.insert(coboundary(g, modulus, n - 1, &b));
    }
    cocycles / coboundaries.len()
}

/// The **cup product** `Cᵖ × Cᵠ → Cᵖ⁺ᵠ` (Alexander–Whitney; `Z/modulus` coefficients, trivial action,
/// no signs): `(f ∪ h)(a₁,…,a_{p+q}) = f(a₁,…,a_p) · h(a_{p+1},…,a_{p+q})`. It makes the obstruction
/// groups a graded ring `H*(G; A)` — the multiplicative algebra binding all the k-invariant levels.
pub fn cup(
    g: &FiniteGroup,
    modulus: usize,
    p: usize,
    q: usize,
    f: &[usize],
    h: &[usize],
) -> Vec<usize> {
    let order = g.order();
    let mut out = vec![0usize; order.pow((p + q) as u32)];
    for t in tuples(order, p + q) {
        out[idx(order, &t)] = (f[idx(order, &t[..p])] * h[idx(order, &t[p..])]) % modulus;
    }
    out
}

/// The **cup-square** cohomology operation `x ↦ x ∪ x : Hⁿ(−; A) → H²ⁿ(−; A)`. Over `Z/2` it is the TOP
/// Steenrod square `Sqⁿ` — the first genuine *cohomology operation* (a natural transformation of the
/// functor `Hⁿ(−;A)`), the secondary structure carried by the cohomology the Eilenberg–MacLane spectrum
/// represents.
pub fn cup_square(g: &FiniteGroup, modulus: usize, n: usize, f: &[usize]) -> Vec<usize> {
    cup(g, modulus, n, n, f, f)
}

/// The **Bockstein** `β : Hⁿ(−; Z/2) → Hⁿ⁺¹(−; Z/2)` — the connecting homomorphism of the short exact
/// sequence `0 → Z/2 →(×2) Z/4 → Z/2 → 0`. By a theorem of Steenrod, `β = Sq¹`. Computed honestly: lift
/// the `Z/2`-cochain to `Z/4`, take `δ` over `Z/4` (which is even on a `Z/2`-cocycle), divide by 2, and
/// reduce mod 2. The lowest genuine Steenrod square, reached as a connecting map rather than a cup-square.
pub fn bockstein(g: &FiniteGroup, n: usize, c: &[usize]) -> Vec<usize> {
    let lifted: Vec<usize> = c.iter().map(|&v| v % 4).collect();
    let d = coboundary(g, 4, n, &lifted); // δ over Z/4
    d.iter().map(|&v| (v / 2) % 2).collect()
}

/// The cup-power `xⁿ` of `Z/2` with `Z/2` coefficients: `α(g_1,…,g_n) = g_1·g_2·⋯·g_n` (1 iff every
/// argument is 1). As the `n`-fold cup product of the nonzero 1-cocycle, it is an `n`-cocycle and the
/// generator of `Hⁿ(Z/2; Z/2) = Z/2` — the explicit nonzero obstruction at level `n`.
pub fn cup_power_z2(n: usize) -> Vec<usize> {
    let len = 2usize.pow(n as u32);
    let mut f = vec![0usize; len];
    f[len - 1] = 1; // only the all-ones tuple (1,1,…,1) has product 1
    f
}

/// The canonical nontrivial 3-cocycle (the cup-cube `x³`), generator of `H³(Z/2; Z/2) = Z/2`.
pub fn cup_cube_z2() -> Vec<usize> {
    cup_power_z2(3)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/postnikov/tests.rs"]
mod tests;
