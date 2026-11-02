//! Thermal-map replay — the independent judge of a claimed steady-state temperature map.
//!
//! The crown unification: the thermal solve `G·ΔT = P` IS the linear-system mathematics the
//! relaxation lane certifies ([`crate::relax_replay`]) — one trust pattern, two physical
//! planes. The claim is NEUTRAL data: a conductance edge list, ground (boundary)
//! conductances, power injections, and the claimed per-node temperature rise — all exact
//! rationals. [`check_thermal_claim`] reassembles the Laplacian ITSELF from the primitive
//! edges (it never trusts an assembled matrix), re-multiplies exactly, and demands
//! `G·ΔT = P` as an EQUALITY — the producer solved exactly, so nothing less certifies.
//! Fail-closed throughout: malformed shapes, non-positive conductances, negative injections,
//! a node with no path to any ground (heat with nowhere to go), or a negative claimed rise
//! are rejections, never a verdict.

use logicaffeine_base::numeric::Rational;

/// The neutral thermal claim a stack's thermal solve emits.
#[derive(Clone, Debug, PartialEq)]
pub struct ThermalClaim {
    /// Node count (ΔT is per node, kelvin above ambient).
    pub n: usize,
    /// Undirected conductance edges `(a, b, g)` in W/K, `g > 0`.
    pub edges: Vec<(usize, usize, Rational)>,
    /// Boundary conductances to ambient `(node, g)` in W/K, `g > 0`.
    pub grounds: Vec<(usize, Rational)>,
    /// Power injections `(node, watts)`, non-negative.
    pub injections: Vec<(usize, Rational)>,
    /// The claimed steady-state rise above ambient, per node.
    pub delta_t: Vec<Rational>,
}

/// What a TRUE map says. The junction-limit judgment belongs to the cooling phase's
/// consumer — the verdict reports the physics, it does not editorialize.
#[derive(Clone, Debug, PartialEq)]
pub struct ThermalVerdict {
    pub hottest_node: usize,
    pub hottest_delta_t: Rational,
}

/// THE judge. `Some(verdict)` iff the claim is well-formed, physically coherent, and the
/// reassembled `G·ΔT` equals `P` EXACTLY at every node.
pub fn check_thermal_claim(claim: &ThermalClaim) -> Option<ThermalVerdict> {
    let n = claim.n;
    if n == 0 || claim.delta_t.len() != n {
        return None;
    }
    for (a, b, g) in &claim.edges {
        if *a >= n || *b >= n || a == b || !g.is_positive() {
            return None;
        }
    }
    for (node, g) in &claim.grounds {
        if *node >= n || !g.is_positive() {
            return None;
        }
    }
    if claim.grounds.is_empty() {
        return None; // heat with nowhere to go — no steady state exists
    }
    for (node, w) in &claim.injections {
        if *node >= n || w.is_negative() {
            return None;
        }
    }
    // Positivity: with non-negative injections, no node sits below ambient.
    if claim.delta_t.iter().any(|t| t.is_negative()) {
        return None;
    }

    // Connectivity: every node must reach a grounded node through the edge list.
    let mut adj = vec![Vec::new(); n];
    for (a, b, _) in &claim.edges {
        adj[*a].push(*b);
        adj[*b].push(*a);
    }
    let mut reached = vec![false; n];
    let mut queue: Vec<usize> = claim.grounds.iter().map(|(g, _)| *g).collect();
    for g in &queue {
        reached[*g] = true;
    }
    while let Some(v) = queue.pop() {
        for w in &adj[v] {
            if !reached[*w] {
                reached[*w] = true;
                queue.push(*w);
            }
        }
    }
    if reached.iter().any(|r| !r) {
        return None;
    }

    // Reassemble the Laplacian row-action INDEPENDENTLY and demand exact balance:
    //   Σ_edges g·(ΔT_i − ΔT_j) + g_ground·ΔT_i = P_i  at every node i.
    let mut balance: Vec<Rational> = vec![Rational::zero(); n];
    for (a, b, g) in &claim.edges {
        let diff = claim.delta_t[*a].sub(&claim.delta_t[*b]);
        balance[*a] = balance[*a].add(&g.mul(&diff));
        balance[*b] = balance[*b].sub(&g.mul(&diff));
    }
    for (node, g) in &claim.grounds {
        balance[*node] = balance[*node].add(&g.mul(&claim.delta_t[*node]));
    }
    let mut p: Vec<Rational> = vec![Rational::zero(); n];
    for (node, w) in &claim.injections {
        p[*node] = p[*node].add(w);
    }
    if balance.iter().zip(&p).any(|(bi, pi)| bi != pi) {
        return None;
    }

    let hottest = (0..n).max_by(|a, b| claim.delta_t[*a].cmp(&claim.delta_t[*b]))?;
    Some(ThermalVerdict {
        hottest_node: hottest,
        hottest_delta_t: claim.delta_t[hottest].clone(),
    })
}
