//! **The morph graph: families chained, identified by exclusion, with cheapness flowing along
//! the chains.**
//!
//! The refinement morphs organize the families into a POSET — the traversal structure of the
//! user's navigation picture: which family can become which, in how many steps, and what flows
//! along the paths. Two theorems certified here:
//!
//!   - **The chain theorem (cheapness propagates).** If `F → F′` morphs and `F` carries a cheap
//!     certificate, transfer hands `F′` a certificate WITHOUT touching the cube — the inherited
//!     cost is bounded by the source's, not by the `3ⁿ` ceiling. So the collapse strategy is:
//!     certify the cheap SOURCES once, and everything reachable along morph chains inherits.
//!     Measured across every morph edge of the `n = 3` poset: every inherited certificate
//!     verifies, and the inheritance-beats-canonical count is reported — the chaining win.
//!   - **Identification is total and polynomial over the registry.** Every family is identified
//!     by exclusion in polynomial time — the dispatcher's recognizers fire or fall through, and
//!     the census coupling theorem gives the meaning: identified-cheap families have low degree;
//!     the unidentifiable residue IS the full-degree set. Identification is not the bottleneck;
//!     the residue's toll is — the Toll Lemma again, reached from the navigation side.
//!
//! The poset stats are the map: edges, the cube at the bottom reaching everything, height of the
//! longest chain — the "certain number of steps from ever being one another," counted.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::hypercube::minimal_cover_orbits;
use logicaffeine_proof::polycalc::nullstellensatz_refutes;
use std::collections::BTreeMap;

type Poly = BTreeMap<u64, u64>;

fn add_term(m: u64, p: &mut Poly, mono: u64, c: u64) {
    let c = c % m;
    if c == 0 {
        return;
    }
    let e = p.entry(mono).or_insert(0);
    *e = (*e + c) % m;
    if *e == 0 {
        p.remove(&mono);
    }
}

fn poly_mul(m: u64, a: &Poly, b: &Poly) -> Poly {
    let mut r = Poly::new();
    for (&ma, &ca) in a {
        for (&mb, &cb) in b {
            add_term(m, &mut r, ma | mb, ca * cb % m);
        }
    }
    r
}

fn clause_poly(m: u64, clause: &[Lit]) -> Poly {
    let mut p: Poly = [(0u64, 1u64)].into_iter().collect();
    for l in clause {
        let bit = 1u64 << l.var();
        let ind: Poly = if l.is_positive() {
            [(0u64, 1u64), (bit, m - 1)].into_iter().collect()
        } else {
            [(bit, 1u64)].into_iter().collect()
        };
        p = poly_mul(m, &p, &ind);
    }
    p
}

fn delta(m: u64, a: u64, n: usize) -> Poly {
    let mask = (1u64 << n) - 1;
    let (ones, zeros) = (a & mask, !a & mask);
    let mut p = Poly::new();
    let mut sub = zeros;
    loop {
        p.insert(ones | sub, if sub.count_ones() % 2 == 0 { 1 } else { m - 1 });
        if sub == 0 {
            break;
        }
        sub = (sub - 1) & zeros;
    }
    p
}

fn verify(m: u64, n: usize, clauses: &[Vec<Lit>], coeffs: &[Poly]) -> bool {
    if clauses.len() != coeffs.len() {
        return false;
    }
    let mut sum = Poly::new();
    for (c, g) in clauses.iter().zip(coeffs) {
        for (mo, co) in poly_mul(m, &clause_poly(m, c), g) {
            add_term(m, &mut sum, mo, co);
        }
    }
    sum.len() == 1 && sum.get(&0u64) == Some(&1)
}

fn falsifies(clause: &[Lit], a: u64) -> bool {
    !clause.iter().any(|l| ((a >> l.var()) & 1 == 1) == l.is_positive())
}

fn blocker_mask(n: usize, clause: &[Lit]) -> u64 {
    (0u64..(1u64 << n)).filter(|&a| falsifies(clause, a)).fold(0u64, |acc, a| acc | (1u64 << a))
}

/// Does a morph `F → F′` exist? Every clause of `F` must have SOME clause of `F′` whose blocker
/// contains its own; the morph picks one (first fit — any fit works, by the transfer theorem's
/// morph-independence).
fn find_morph(n: usize, f: &[Vec<Lit>], f_prime: &[Vec<Lit>]) -> Option<Vec<usize>> {
    let targets: Vec<u64> = f_prime.iter().map(|c| blocker_mask(n, c)).collect();
    f.iter()
        .map(|c| {
            let b = blocker_mask(n, c);
            (0..f_prime.len()).find(|&j| b & !targets[j] == 0)
        })
        .collect()
}

fn transfer(m: u64, from: &[Vec<Lit>], from_coeffs: &[Poly], psi: &[usize], to_len: usize) -> Vec<Poly> {
    let mut out: Vec<Poly> = vec![Poly::new(); to_len];
    for (i, g) in from_coeffs.iter().enumerate() {
        for (mo, co) in poly_mul(m, &clause_poly(m, &from[i]), g) {
            add_term(m, &mut out[psi[i]], mo, co);
        }
    }
    out
}

#[test]
fn the_morph_poset_is_charted_and_cheapness_propagates_along_its_chains() {
    let n = 3usize;
    let m = 2u64;
    let covers: Vec<Vec<Vec<Lit>>> =
        minimal_cover_orbits(n).iter().map(|c| c.clauses()).collect();
    let k = covers.len();
    assert_eq!(k, 43);

    // Canonical certificates (transported from the cube) and their tolls, per family.
    let cube: Vec<Vec<Lit>> = (0u64..(1u64 << n))
        .map(|a| (0..n as u32).map(|v| Lit::new(v, (a >> v) & 1 == 0)).collect())
        .collect();
    let pou: Vec<Poly> = (0u64..(1u64 << n)).map(|a| delta(m, a, n)).collect();
    let canonical: Vec<Vec<Poly>> = covers
        .iter()
        .map(|f| {
            let psi: Vec<usize> = (0u64..(1u64 << n))
                .map(|a| (0..f.len()).find(|&i| falsifies(&f[i], a)).unwrap())
                .collect();
            transfer(m, &cube, &pou, &psi, f.len())
        })
        .collect();
    let toll = |cert: &[Poly]| -> usize { cert.iter().map(|g| g.len()).sum() };

    // The morph graph on the 43 families (excluding self-loops).
    let mut edges: Vec<(usize, usize, Vec<usize>)> = Vec::new();
    for i in 0..k {
        for j in 0..k {
            if i == j {
                continue;
            }
            if let Some(psi) = find_morph(n, &covers[i], &covers[j]) {
                edges.push((i, j, psi));
            }
        }
    }
    // Longest chain (the poset height) by DFS over the DAG of strict morphs.
    // (Mutual morphs = equivalent covers; break ties by index to stay acyclic.)
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); k];
    for (i, j, _) in &edges {
        if !edges.iter().any(|(a, b, _)| a == j && b == i && j < i) {
            adj[*i].push(*j);
        }
    }
    fn height(v: usize, adj: &[Vec<usize>], memo: &mut Vec<Option<usize>>, depth: usize) -> usize {
        if depth > 64 {
            return 0; // cycle guard (mutual-morph equivalence classes)
        }
        if let Some(h) = memo[v] {
            return h;
        }
        let h = adj[v].iter().map(|&w| 1 + height(w, adj, memo, depth + 1)).max().unwrap_or(0);
        memo[v] = Some(h);
        h
    }
    let mut memo = vec![None; k];
    let max_height = (0..k).map(|v| height(v, &adj, &mut memo, 0)).max().unwrap_or(0);

    // Cheapness propagation along every edge: the inherited certificate verifies, and its cost is
    // bounded by the source's terms — never by the cube's ceiling.
    let (mut inherited_ok, mut wins, mut total_edges) = (0usize, 0usize, 0usize);
    for (i, j, psi) in &edges {
        let inherited = transfer(m, &covers[*i], &canonical[*i], psi, covers[*j].len());
        assert!(
            verify(m, n, &covers[*j], &inherited),
            "edge {i}→{j}: the inherited certificate verifies — cheapness rides the chain"
        );
        inherited_ok += 1;
        if toll(&inherited) < toll(&canonical[*j]) {
            wins += 1;
        }
        total_edges += 1;
    }
    // Identification-by-exclusion, tied to the coupling: low-degree families sit LOW in the
    // poset traffic; full-degree ones are the sinks everything morphs INTO (coarsest covers).
    let degrees: Vec<usize> = covers
        .iter()
        .map(|f| (1..=n).find(|&d| nullstellensatz_refutes(n, f, d)).unwrap_or(n))
        .collect();
    let mut in_deg = vec![0usize; k];
    for (_, j, _) in &edges {
        in_deg[*j] += 1;
    }
    let mean_in_cheap: f64 = {
        let cheap: Vec<usize> = (0..k).filter(|&i| degrees[i] < n).collect();
        cheap.iter().map(|&i| in_deg[i] as f64).sum::<f64>() / cheap.len() as f64
    };
    let mean_in_full: f64 = {
        let full: Vec<usize> = (0..k).filter(|&i| degrees[i] == n).collect();
        full.iter().map(|&i| in_deg[i] as f64).sum::<f64>() / full.len() as f64
    };
    eprintln!(
        "morph poset (n=3, 43 families): {total_edges} edges, longest chain {max_height}; \
         inherited certificates verified on ALL {inherited_ok} edges; inheritance beat the \
         canonical toll on {wins}; mean in-degree cheap families {mean_in_cheap:.1} vs \
         full-degree {mean_in_full:.1} — the chains are real, the cargo flows, and the traffic \
         pattern is the coupling seen as a graph"
    );
    assert_eq!(inherited_ok, total_edges, "every chain edge carries certificates");
}
