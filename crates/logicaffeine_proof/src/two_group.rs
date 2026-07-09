//! The **2-group frontier** — climbing past `K(G,1)` to genuine 2-types, honestly.
//!
//! The tower named its 1-truncation: a *discrete* symmetry group `G` gives the ∞-groupoid
//! `BG = K(G,1)` — `π₁ = G`, `πₙ = 0` for `n ≥ 2`. To populate `π₂` you need a **2-group** (a symmetry
//! whose symmetries themselves have symmetries), modeled by a **crossed module** `∂: H → G`:
//!
//! - `π₁ = coker(∂) = G / ∂(H)` — the symmetries that survive modulo the higher cells, and
//! - `π₂ = ker(∂)` — the genuinely higher homotopy.
//!
//! This module builds that machinery and is honest about where our SAT structure sits in it: our
//! symmetry is the *degenerate* crossed module `1 → G` (so `π₂ = 0`, `BG = K(G,1)`), and a genuine
//! 2-group `Z/m → 1` gives `π₂ = Z/m ≠ 0` — a `K(Z/m, 2)`. The machinery reads off both; our case is
//! the 1-type. Climbing further (`π₃, …`) is the same construction one categorical level up — the
//! frontier, named, not claimed.

/// A finite group by its (0-indexed) multiplication table.
#[derive(Clone)]
pub struct FiniteGroup {
    /// `mul[a][b] = a · b`.
    pub mul: Vec<Vec<usize>>,
    /// The identity element.
    pub id: usize,
}

impl FiniteGroup {
    pub fn order(&self) -> usize {
        self.mul.len()
    }

    pub fn inverse(&self, a: usize) -> usize {
        (0..self.order()).find(|&b| self.mul[a][b] == self.id).expect("a group element has an inverse")
    }

    /// The trivial group `1`.
    pub fn trivial() -> FiniteGroup {
        FiniteGroup { mul: vec![vec![0]], id: 0 }
    }

    /// The cyclic group `Z/m` (elements `0..m`, addition mod `m`).
    pub fn cyclic(m: usize) -> FiniteGroup {
        let mul = (0..m).map(|a| (0..m).map(|b| (a + b) % m).collect()).collect();
        FiniteGroup { mul, id: 0 }
    }

    /// The symmetric group `S_n` (all `n!` permutations, indexed; multiplication = composition). This
    /// is the shape of a pigeonhole symmetry group (pigeon/hole permutations).
    pub fn symmetric(n: usize) -> FiniteGroup {
        // enumerate permutations of 0..n (Heap-free: lexicographic via Steinhaus–Johnson or simple
        // recursive build)
        let mut perms: Vec<Vec<usize>> = Vec::new();
        let mut cur: Vec<usize> = (0..n).collect();
        permute(&mut cur, 0, &mut perms);
        let index: std::collections::HashMap<Vec<usize>, usize> =
            perms.iter().cloned().enumerate().map(|(i, p)| (p, i)).collect();
        let compose = |p: &[usize], q: &[usize]| -> Vec<usize> { (0..n).map(|i| p[q[i]]).collect() };
        let mul: Vec<Vec<usize>> = perms
            .iter()
            .map(|p| perms.iter().map(|q| index[&compose(p, q)]).collect())
            .collect();
        let id = index[&(0..n).collect::<Vec<_>>()];
        FiniteGroup { mul, id }
    }
}

fn permute(arr: &mut Vec<usize>, k: usize, out: &mut Vec<Vec<usize>>) {
    if k == arr.len() {
        out.push(arr.clone());
        return;
    }
    for i in k..arr.len() {
        arr.swap(k, i);
        permute(arr, k + 1, out);
        arr.swap(k, i);
    }
}

/// A **crossed module** `∂: H → G` — the algebraic model of a 2-group. `partial[h]` is `∂(h) ∈ G`,
/// a group homomorphism. Its homotopy groups are `π₁ = coker(∂)` and `π₂ = ker(∂)`.
pub struct CrossedModule {
    pub g: FiniteGroup,
    pub h: FiniteGroup,
    /// `partial[x] = ∂(x)` for `x ∈ H`.
    pub partial: Vec<usize>,
}

impl CrossedModule {
    /// `∂` must be a group homomorphism: `∂(x · y) = ∂(x) · ∂(y)`.
    pub fn is_homomorphism(&self) -> bool {
        (0..self.h.order()).all(|x| {
            (0..self.h.order()).all(|y| {
                self.partial[self.h.mul[x][y]] == self.g.mul[self.partial[x]][self.partial[y]]
            })
        })
    }

    /// `π₂ = ker(∂)` — the elements of `H` mapping to the identity of `G`. The genuine higher homotopy.
    pub fn pi_two(&self) -> Vec<usize> {
        (0..self.h.order()).filter(|&x| self.partial[x] == self.g.id).collect()
    }

    /// `|im(∂)|` — the size of `∂`'s image in `G`.
    pub fn image_size(&self) -> usize {
        let mut img: Vec<usize> = (0..self.h.order()).map(|x| self.partial[x]).collect();
        img.sort_unstable();
        img.dedup();
        img.len()
    }

    /// `|π₁| = |coker(∂)| = |G| / |im(∂)|` (valid when `im(∂)` is normal, which holds for our examples).
    pub fn pi_one_order(&self) -> usize {
        self.g.order() / self.image_size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_groups_are_well_formed() {
        for g in [FiniteGroup::trivial(), FiniteGroup::cyclic(5), FiniteGroup::symmetric(4)] {
            let n = g.order();
            // identity acts as identity, and every element has an inverse
            for a in 0..n {
                assert_eq!(g.mul[g.id][a], a);
                assert_eq!(g.mul[a][g.id], a);
                let _ = g.inverse(a);
            }
        }
        assert_eq!(FiniteGroup::symmetric(4).order(), 24, "|S_4| = 24");
    }

    #[test]
    fn discrete_symmetry_is_a_one_type_pi_two_vanishes() {
        // Our SAT symmetry is DISCRETE: the degenerate crossed module 1 → G. π₂ = ker(∂) = 1, π₁ = G.
        // So the ∞-groupoid is BG = K(G,1), a 1-TYPE — exactly the tower's named top, now via the
        // 2-group machinery. (G = S_4, the pigeon symmetry of PHP(5).)
        let g = FiniteGroup::symmetric(4);
        let cm = CrossedModule { g, h: FiniteGroup::trivial(), partial: vec![0] };
        assert!(cm.is_homomorphism(), "∂ : 1 → G is a homomorphism");
        assert_eq!(cm.pi_two().len(), 1, "π₂ = ker(∂) = trivial ⇒ a 1-type (discrete symmetry)");
        assert_eq!(cm.pi_one_order(), 24, "π₁ = G = S_4");
    }

    #[test]
    fn a_genuine_2_group_populates_pi_two() {
        // A GENUINE 2-group (the frontier): the crossed module Z/m → 1 with ∂ = 0. Now π₂ = ker(∂) =
        // Z/m ≠ 0 — a K(Z/m, 2), a genuine 2-TYPE. This is what a symmetry-with-internal-symmetry
        // would contribute; our SAT structure does not provide it, but the machinery reads it off.
        for m in [2usize, 3, 5] {
            let cm = CrossedModule { g: FiniteGroup::trivial(), h: FiniteGroup::cyclic(m), partial: vec![0; m] };
            assert!(cm.is_homomorphism(), "∂ = 0 : Z/m → 1 is a homomorphism");
            assert_eq!(cm.pi_two().len(), m, "π₂ = ker(∂) = Z/{m} — genuine higher homotopy");
            assert_eq!(cm.pi_one_order(), 1, "π₁ = coker = trivial");
        }
    }

    #[test]
    fn an_isomorphism_crossed_module_is_contractible() {
        // ∂ = id : Z/m → Z/m kills both homotopy groups (π₁ = coker = 1, π₂ = ker = 1) — a contractible
        // 2-group. The machinery spans the range: from contractible, through K(G,1), to K(Z/m,2).
        let m = 6;
        let cm = CrossedModule {
            g: FiniteGroup::cyclic(m),
            h: FiniteGroup::cyclic(m),
            partial: (0..m).collect(), // ∂ = identity
        };
        assert!(cm.is_homomorphism());
        assert_eq!(cm.pi_two().len(), 1, "π₂ = ker(id) = trivial");
        assert_eq!(cm.pi_one_order(), 1, "π₁ = coker(id) = trivial");
    }
}
