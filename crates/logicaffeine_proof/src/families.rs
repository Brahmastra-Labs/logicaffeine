//! Parametric generators for symmetry-rich SAT families — the canonical hard cases where
//! symmetry breaking earns its keep. They are programmatic (reproducible, offline, parametric)
//! rather than vendored `.cnf` files, and each is pinned to its known verdict so the solver and
//! the certified pipeline can be tested against ground truth.

use crate::cdcl::Lit;
use crate::dimacs::DimacsCnf;

/// The known verdict of a generated instance, for test oracles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedVerdict {
    Sat,
    Unsat,
}

/// The pigeonhole principle PHP(n): `n` pigeons into `n-1` holes — unsatisfiable, and the
/// textbook symmetry / resolution-hard family (any resolution refutation is exponential, while
/// breaking the `S_n × S_{n-1}` symmetry collapses it). The variable for "pigeon `p` sits in
/// hole `h`" lives at index `p*(n-1) + h`, so pigeons index rows and holes index columns.
pub fn php(n: usize) -> (DimacsCnf, ExpectedVerdict) {
    let holes = n.saturating_sub(1);
    let num_vars = n * holes;
    let var = |p: usize, h: usize| Lit::pos((p * holes + h) as u32);
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    // Each pigeon occupies at least one hole (an empty disjunction when holes == 0).
    for p in 0..n {
        clauses.push((0..holes).map(|h| var(p, h)).collect());
    }
    // No two pigeons share a hole.
    for h in 0..holes {
        for p in 0..n {
            for q in (p + 1)..n {
                clauses.push(vec![var(p, h).negated(), var(q, h).negated()]);
            }
        }
    }
    (DimacsCnf { num_vars, clauses }, ExpectedVerdict::Unsat)
}

/// **Coupled exactly-one + parity** — a scalable MIXED family that needs BOTH structures at once. The
/// selectors `x₀…x_{n-1}` are constrained to *exactly one* true (an at-most-one clique + an at-least-one
/// clause — a cardinality covering) AND to *even* total parity `⊕ xᵢ = 0` (a GF(2) system, encoded by a
/// width-3 prefix-XOR chain with auxiliaries `sᵢ = ⊕_{j≤i} xⱼ` plus the unit `s_{n-1} = 0`). Exactly-one
/// forces an ODD selector count, the parity forces EVEN — **UNSAT**, yet neither substructure alone is
/// (the parity chain is satisfiable, exactly-one is satisfiable). The canonical shape for the fused
/// parity+cardinality route ([`crate::lyapunov::fused_parity_cardinality_decide`]); `2n` variables.
pub fn parity_exactly_one(n: usize) -> (DimacsCnf, ExpectedVerdict) {
    assert!(n >= 2, "need at least two selectors");
    let x = |i: usize| i as u32; // selectors 0..n-1
    let s = |i: usize| (n + i) as u32; // prefix-XOR auxiliaries n..2n-1
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    // exactly-one of the selectors: at-least-one ∨, then pairwise at-most-one.
    clauses.push((0..n).map(|i| Lit::pos(x(i))).collect());
    for i in 0..n {
        for j in (i + 1)..n {
            clauses.push(vec![Lit::neg(x(i)), Lit::neg(x(j))]);
        }
    }
    // Prefix-XOR chain: s₀ = x₀, sᵢ = s_{i-1} ⊕ xᵢ, s_{n-1} = 0 — each a small XOR gadget.
    let gadget = |vars: &[u32], out: &mut Vec<Vec<Lit>>| {
        let k = vars.len();
        for mask in 0u32..(1 << k) {
            if mask.count_ones() % 2 == 1 {
                out.push(
                    (0..k)
                        .map(|t| Lit::new(vars[t], (mask >> t) & 1 == 0))
                        .collect(),
                );
            }
        }
    };
    gadget(&[s(0), x(0)], &mut clauses);
    for i in 1..n {
        gadget(&[s(i), s(i - 1), x(i)], &mut clauses);
    }
    clauses.push(vec![Lit::neg(s(n - 1))]);
    (
        DimacsCnf {
            num_vars: 2 * n,
            clauses,
        },
        ExpectedVerdict::Unsat,
    )
}

/// Graph `k`-coloring of the complete graph `K_n` — the canonical *color-permutation* symmetry
/// family (the color group `S_k` acts, on top of the vertex group `S_n`). It is unsatisfiable
/// exactly when `k < n` (a clique of size `n` needs `n` colors). The variable "vertex `v` takes
/// color `c`" lives at index `v*k + c`.
pub fn clique_coloring(n: usize, k: usize) -> (DimacsCnf, ExpectedVerdict) {
    let num_vars = n * k;
    let var = |v: usize, c: usize| Lit::pos((v * k + c) as u32);
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    // Every vertex takes at least one color.
    for v in 0..n {
        clauses.push((0..k).map(|c| var(v, c)).collect());
    }
    // Adjacent vertices (every pair, in K_n) must differ in color.
    for u in 0..n {
        for w in (u + 1)..n {
            for c in 0..k {
                clauses.push(vec![var(u, c).negated(), var(w, c).negated()]);
            }
        }
    }
    let verdict = if k < n {
        ExpectedVerdict::Unsat
    } else {
        ExpectedVerdict::Sat
    };
    (DimacsCnf { num_vars, clauses }, verdict)
}

/// The mutilated chessboard: an `n×n` board with two OPPOSITE corners removed, asking for a perfect
/// domino tiling. Every domino covers one black and one white square, but the two removed corners
/// share a colour (for even `n`), so the black/white counts differ by two and no tiling exists —
/// UNSAT, and a textbook resolution-exponential family. The infeasibility is a bipartite-matching
/// (Hall) obstruction, which our covering route certifies in polynomial time. Variables are dominoes
/// (grid edges between adjacent surviving squares); each surviving square must be covered by exactly
/// one. `n` must be even (so the parity argument bites).
pub fn mutilated_chessboard(n: usize) -> (DimacsCnf, ExpectedVerdict) {
    assert!(
        n >= 4 && n % 2 == 0,
        "the parity argument needs an even board ≥ 4"
    );
    let removed = |r: usize, c: usize| (r == 0 && c == 0) || (r == n - 1 && c == n - 1);
    let sq = |r: usize, c: usize| r * n + c;
    // Edges: horizontal (r,c)–(r,c+1) and vertical (r,c)–(r+1,c), both endpoints surviving.
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for r in 0..n {
        for c in 0..n {
            if removed(r, c) {
                continue;
            }
            if c + 1 < n && !removed(r, c + 1) {
                edges.push((sq(r, c), sq(r, c + 1)));
            }
            if r + 1 < n && !removed(r + 1, c) {
                edges.push((sq(r, c), sq(r + 1, c)));
            }
        }
    }
    let num_vars = edges.len();
    let mut incident: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for (e, &(a, b)) in edges.iter().enumerate() {
        incident.entry(a).or_default().push(e);
        incident.entry(b).or_default().push(e);
    }
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    for r in 0..n {
        for c in 0..n {
            if removed(r, c) {
                continue;
            }
            let inc = incident.get(&sq(r, c)).cloned().unwrap_or_default();
            clauses.push(inc.iter().map(|&e| Lit::pos(e as u32)).collect()); // covered ≥ once
            for i in 0..inc.len() {
                for j in (i + 1)..inc.len() {
                    clauses.push(vec![
                        Lit::pos(inc[i] as u32).negated(),
                        Lit::pos(inc[j] as u32).negated(),
                    ]); // ≤ once
                }
            }
        }
    }
    (DimacsCnf { num_vars, clauses }, ExpectedVerdict::Unsat)
}

/// The linear ordering principle GT(n): no finite strict total order lacks a maximal element — yet
/// this formula asserts exactly that, so it is UNSAT. `x[i][j]` (index `i*n + j`) reads "i < j".
/// Antisymmetry + totality + transitivity force a linear order, and the "every element has a greater
/// one" clauses then contradict finiteness. A canonical resolution-hard family — it stresses the
/// CDCL core / cutting-plane reasoning rather than a matching or parity specialist.
pub fn ordering_principle(n: usize) -> (DimacsCnf, ExpectedVerdict) {
    assert!(n >= 2);
    let var = |i: usize, j: usize| Lit::pos((i * n + j) as u32);
    let num_vars = n * n;
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            clauses.push(vec![var(i, j), var(j, i)]); // totality
            clauses.push(vec![var(i, j).negated(), var(j, i).negated()]); // antisymmetry
        }
    }
    for i in 0..n {
        for j in 0..n {
            for k in 0..n {
                if i != j && j != k && i != k {
                    clauses.push(vec![var(i, j).negated(), var(j, k).negated(), var(i, k)]);
                    // transitivity
                }
            }
        }
    }
    for i in 0..n {
        clauses.push((0..n).filter(|&j| j != i).map(|j| var(i, j)).collect()); // i has a greater element
    }
    (DimacsCnf { num_vars, clauses }, ExpectedVerdict::Unsat)
}

/// A random 3-SAT instance: `num_clauses` clauses of three distinct variables with random signs,
/// over `vars` variables, from a seeded SplitMix64 stream (reproducible — no wall-clock). Near the
/// clause/variable ratio 4.26 these are the canonical *hard, non-symmetric* benchmarks — the
/// general-instance control where raw CDCL quality (not symmetry breaking) decides the race.
pub fn random_3sat(vars: usize, num_clauses: usize, seed: u64) -> DimacsCnf {
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let mut clauses: Vec<Vec<Lit>> = Vec::with_capacity(num_clauses);
    while clauses.len() < num_clauses {
        let mut vs: Vec<u32> = Vec::with_capacity(3);
        while vs.len() < 3 {
            let v = (next() as usize % vars) as u32;
            if !vs.contains(&v) {
                vs.push(v);
            }
        }
        clauses.push(vs.iter().map(|&v| Lit::new(v, next() & 1 == 0)).collect());
    }
    DimacsCnf {
        num_vars: vars,
        clauses,
    }
}

/// A random **k-SAT** instance: `num_clauses` clauses of `k` distinct variables with random signs, over
/// `vars` variables, from a seeded SplitMix64 stream (reproducible — no wall-clock). Generalizes
/// [`random_3sat`]. The satisfiability threshold climbs with `k` roughly as `α_k ≈ 2ᵏ ln 2` (≈ 4.27 for
/// k=3, ≈ 9.93 for k=4, ≈ 21.1 for k=5). Requires `k ≤ vars`.
pub fn random_ksat(k: usize, vars: usize, num_clauses: usize, seed: u64) -> DimacsCnf {
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let mut clauses: Vec<Vec<Lit>> = Vec::with_capacity(num_clauses);
    while clauses.len() < num_clauses {
        let mut vs: Vec<u32> = Vec::with_capacity(k);
        while vs.len() < k {
            let v = (next() as usize % vars) as u32;
            if !vs.contains(&v) {
                vs.push(v);
            }
        }
        clauses.push(vs.iter().map(|&v| Lit::new(v, next() & 1 == 0)).collect());
    }
    DimacsCnf {
        num_vars: vars,
        clauses,
    }
}

/// The CNF clauses encoding the parity constraint `v0 ⊕ v1 ⊕ … = rhs` over any arity (forbid every
/// wrong-parity assignment of the listed variables — `2^{|vs|-1}` clauses, each of width `|vs|`).
fn xor_clauses(vs: &[u32], rhs: bool) -> Vec<Vec<Lit>> {
    let mut out = Vec::new();
    for mask in 0u32..(1 << vs.len()) {
        let odd = mask.count_ones() % 2 == 1;
        if odd != rhs {
            // Forbid this assignment: a literal is true exactly when it differs from the mask bit.
            out.push(
                vs.iter()
                    .enumerate()
                    .map(|(i, &v)| Lit::new(v, (mask >> i) & 1 == 0))
                    .collect(),
            );
        }
    }
    out
}

/// Fold a freshly sampled `k`-subset's coefficient row into the running GF(2) echelon basis (rows keyed
/// by pivot = least set variable). Returns `true` and extends the basis if the row is linearly
/// independent of those seen so far, `false` if it lies in their span. This is the rank meter that lets
/// [`random_kxor`] build a *guaranteed*-inconsistent system rather than a merely-probable one.
fn gf2_absorb(basis: &mut std::collections::HashMap<u32, Vec<u32>>, vars: &[u32]) -> bool {
    let mut row: std::collections::BTreeSet<u32> = vars.iter().copied().collect();
    while let Some(&pivot) = row.iter().next() {
        match basis.get(&pivot) {
            Some(b) => {
                for &v in b {
                    if !row.remove(&v) {
                        row.insert(v); // symmetric difference: row ⊕ basis[pivot]
                    }
                }
            }
            None => {
                basis.insert(pivot, row.iter().copied().collect());
                return true;
            }
        }
    }
    false
}

/// A **guaranteed-unsatisfiable** random **k-XOR** (parity) system over `n` variables: at least `m`
/// equations of `k` distinct variables, all consistent with a planted assignment `p` except one whose
/// right-hand side is flipped. Generalizes [`parity_unsat`] (the `k = 3` case).
///
/// The flip alone does **not** force inconsistency — it does so only once the flipped row lies in the
/// span of the others, which a naive "flip the last equation" does not guarantee near `m ≈ n` (it
/// silently yields *satisfiable* instances). So we build the consistent rows until the coefficient
/// matrix reaches its **maximum achievable GF(2) rank** (`n` for odd `k`; `n − 1` for even `k`, whose
/// even-weight rows can never span the all-ones functional) and *at least* `m` rows total, then append
/// one more equation with a flipped right-hand side. At maximal rank that final row is redundant, so the
/// system's solution set — `{p}` for odd `k`, `{p, p̄}` for even `k` — is pinned, and *every* pinned
/// solution violates the flipped row: **the system is inconsistent by construction**, for any seed.
/// Full-rank `k`-XOR above the XOR-SAT threshold is also exponentially hard for resolution
/// (Ben-Sasson–Wigderson 2001) — hence for every CDCL solver — yet linear for Gaussian elimination over
/// GF(2). Each equation expands to `2^{k-1}` width-`k` clauses, so keep `k` modest. `m` is a floor on the
/// equation count (full rank may need a few more). Returns the XOR equations and the equisatisfiable CNF.
pub fn random_kxor(
    k: usize,
    n: usize,
    m: usize,
    seed: u64,
) -> (Vec<crate::xorsat::XorEquation>, DimacsCnf) {
    assert!((1..=n).contains(&k) && m >= 1, "need 1 ≤ k ≤ n and m ≥ 1");
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let planted: Vec<bool> = (0..n).map(|_| next() & 1 == 0).collect();
    let mut draw = || {
        let mut vs: Vec<u32> = Vec::with_capacity(k);
        while vs.len() < k {
            let v = (next() as usize % n) as u32;
            if !vs.contains(&v) {
                vs.push(v);
            }
        }
        vs
    };
    let consistent_rhs = |vs: &[u32]| vs.iter().fold(false, |a, &v| a ^ planted[v as usize]);

    // Phase 1: consistent rows until the coefficient matrix is at maximal rank AND there are ≥ m of them.
    let target_rank = if k % 2 == 1 { n } else { n.saturating_sub(1) };
    let cap = 8 * n + 64; // generous guard; random k-subsets reach maximal rank far inside this.
    let mut basis: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    let mut rows: Vec<(Vec<u32>, bool)> = Vec::new();
    let mut rank = 0usize;
    while (rank < target_rank || rows.len() < m) && rows.len() < cap {
        let vs = draw();
        if gf2_absorb(&mut basis, &vs) {
            rank += 1;
        }
        let rhs = consistent_rhs(&vs);
        rows.push((vs, rhs));
    }
    // Phase 2: one more equation, right-hand side FLIPPED. At maximal rank it is redundant, so the pinned
    // solution set is unchanged by it — yet no pinned solution satisfies it ⇒ the system is UNSAT.
    let vs = draw();
    let rhs = !consistent_rhs(&vs);
    rows.push((vs, rhs));

    let mut eqs = Vec::new();
    let mut clauses = Vec::new();
    for (vs, rhs) in &rows {
        eqs.push(crate::xorsat::XorEquation::new(
            vs.iter().map(|&v| v as usize).collect::<Vec<_>>(),
            *rhs,
        ));
        clauses.extend(xor_clauses(vs, *rhs));
    }
    (
        eqs,
        DimacsCnf {
            num_vars: n,
            clauses,
        },
    )
}

/// A **guaranteed-unsatisfiable** random 3-XOR (parity) system — the `k = 3` case of [`random_kxor`]:
/// at least `m` 3-XOR equations over `n` variables, all consistent with a planted assignment except one
/// flipped row, built up to maximal GF(2) rank so the inconsistency holds for *any* seed (see
/// [`random_kxor`] for why the naive single flip does not suffice). Full-rank 3-XOR above the XOR-SAT
/// threshold is **exponentially hard for resolution** (Ben-Sasson–Wigderson 2001), hence for every CDCL
/// solver — yet linear for Gaussian elimination over GF(2). Returns the XOR equations (for
/// [`crate::xorsat`]) and the equisatisfiable CNF (for a resolution solver).
pub fn parity_unsat(n: usize, m: usize, seed: u64) -> (Vec<crate::xorsat::XorEquation>, DimacsCnf) {
    random_kxor(3, n, m, seed)
}

/// A random 3-regular graph on `n` (even) vertices via the configuration model with self-loop /
/// multi-edge rejection — an expander with high probability, which is what makes the Tseitin
/// formula on it *exponentially* hard for resolution (Urquhart 1987; Ben-Sasson–Wigderson 2001).
fn random_3regular(n: usize, seed: u64) -> Vec<(usize, usize)> {
    let mut state = seed ^ 0x9E3779B97F4A7C15;
    let mut next = move || {
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    for _ in 0..4000 {
        let mut stubs: Vec<usize> = (0..n).flat_map(|v| [v, v, v]).collect();
        for i in (1..stubs.len()).rev() {
            let j = (next() as usize) % (i + 1);
            stubs.swap(i, j);
        }
        let mut edges = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut ok = true;
        for c in stubs.chunks(2) {
            let (a, b) = (c[0].min(c[1]), c[0].max(c[1]));
            if a == b || !seen.insert((a, b)) {
                ok = false;
                break;
            }
            edges.push((a, b));
        }
        if ok {
            return edges;
        }
    }
    panic!("could not build a simple 3-regular graph on {n} vertices");
}

/// The **Tseitin formula on a random 3-regular expander** with odd total charge — UNSATISFIABLE,
/// exponentially hard for resolution (so every CDCL solver blows up), yet solved in polynomial time
/// by Gaussian elimination over GF(2). A *non-pigeonhole* hard family: the hardness is parity /
/// graph expansion, not covering symmetry. One Boolean per edge; per vertex, the XOR of its incident
/// edges equals that vertex's charge (vertex 0 charged, the rest not — an odd sum, hence
/// inconsistent). Returns the XOR system (for [`crate::xorsat`]) and the equisatisfiable CNF.
pub fn tseitin_expander(
    n: usize,
    seed: u64,
) -> (Vec<crate::xorsat::XorEquation>, DimacsCnf, ExpectedVerdict) {
    assert!(
        n % 2 == 0 && n >= 4,
        "a 3-regular graph needs an even vertex count ≥ 4"
    );
    let edges = random_3regular(n, seed);
    let m = edges.len();
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (e, &(a, b)) in edges.iter().enumerate() {
        incident[a].push(e);
        incident[b].push(e);
    }
    let mut eqs = Vec::new();
    let mut clauses = Vec::new();
    for v in 0..n {
        let inc = &incident[v];
        let r = v == 0; // odd total charge ⇒ UNSAT
        eqs.push(crate::xorsat::XorEquation::new(inc.clone(), r));
        let d = inc.len();
        for mask in 0u32..(1u32 << d) {
            if ((mask.count_ones() % 2) == 1) != r {
                clauses.push(
                    (0..d)
                        .map(|i| Lit::new(inc[i] as u32, (mask >> i) & 1 == 0))
                        .collect(),
                );
            }
        }
    }
    (
        eqs,
        DimacsCnf {
            num_vars: m,
            clauses,
        },
        ExpectedVerdict::Unsat,
    )
}

/// The **Tseitin formula on a `w × n` grid** with odd total charge — UNSATISFIABLE, and the sharpest
/// indictment of resolution search there is. A grid has treewidth exactly `w` (a fixed constant,
/// independent of the length `n`), so a POLYNOMIAL-size, bounded-width resolution refutation provably
/// EXISTS. Yet CDCL solvers without Gaussian reasoning blow up on the parity regardless — they cannot
/// find the short proof that is known to exist — while Gaussian elimination over GF(2) decides it in
/// near-linear time. One Boolean per grid edge; per vertex, the XOR of its incident edges equals its
/// charge (vertex 0 charged, the rest not — an odd sum, hence inconsistent). Returns the XOR system
/// (for [`crate::xorsat`]) and the equisatisfiable CNF.
pub fn grid_tseitin(
    w: usize,
    n: usize,
) -> (Vec<crate::xorsat::XorEquation>, DimacsCnf, ExpectedVerdict) {
    assert!(w >= 2 && n >= 2, "a grid needs both dimensions ≥ 2");
    let vid = |i: usize, j: usize| i * n + j;
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); w * n];
    let mut num_edges = 0usize;
    for i in 0..w {
        for j in 0..n {
            let v = vid(i, j);
            if j + 1 < n {
                let e = num_edges;
                num_edges += 1;
                incident[v].push(e);
                incident[vid(i, j + 1)].push(e);
            }
            if i + 1 < w {
                let e = num_edges;
                num_edges += 1;
                incident[v].push(e);
                incident[vid(i + 1, j)].push(e);
            }
        }
    }
    let mut eqs = Vec::new();
    let mut clauses = Vec::new();
    for v in 0..(w * n) {
        let inc = &incident[v];
        let r = v == 0; // odd total charge ⇒ UNSAT
        eqs.push(crate::xorsat::XorEquation::new(inc.clone(), r));
        let d = inc.len();
        for mask in 0u32..(1u32 << d) {
            if ((mask.count_ones() % 2) == 1) != r {
                clauses.push(
                    (0..d)
                        .map(|i| Lit::new(inc[i] as u32, (mask >> i) & 1 == 0))
                        .collect(),
                );
            }
        }
    }
    (
        eqs,
        DimacsCnf {
            num_vars: num_edges,
            clauses,
        },
        ExpectedVerdict::Unsat,
    )
}

/// The **mod-`p` Tseitin obstruction on a random 3-regular expander** — the parity crush carried to
/// every prime. Each edge carries a `GF(p)` flow variable; orient each edge and impose, at every
/// vertex, the signed divergence `Σ_out − Σ_in ≡ charge (mod p)`. Summing all vertices telescopes the
/// edge variables to `0`, so the system is inconsistent exactly when the total charge `≢ 0 (mod p)`.
/// We charge two vertices (total `2`): **inconsistent over `GF(p)` for every odd prime `p`, yet
/// consistent over `GF(2)`** (`2 ≡ 0`) — a family the parity cut is structurally blind to, decided
/// instantly by Gaussian elimination over the *right* `GF(p)`, and (on an expander) exponentially hard
/// for resolution. Returns the `GF(p)` system (for [`crate::modp`]), the equisatisfiable one-hot
/// Boolean CNF (for resolution solvers), and the `GF(p)` verdict.
pub fn mod_p_tseitin_expander(
    n: usize,
    p: u64,
    seed: u64,
) -> (Vec<crate::modp::ModpEquation>, DimacsCnf, ExpectedVerdict) {
    assert!(
        p >= 3,
        "the mod-p obstruction needs an odd prime (p=2 is the parity case, consistent here)"
    );
    mod_tseitin_expander_core(n, p, seed)
}

/// The **composite-modulus** sibling of [`mod_p_tseitin_expander`]: the same total-charge-2 divergence
/// obstruction over `ℤ/m` for a squarefree composite `m` (e.g. `m = 6`), with a mixed-radix one-hot
/// Boolean encoding (`m` values per edge). By CRT `ℤ/m ≅ ∏ GF(pᵢ)`, so the system is inconsistent over
/// `ℤ/m` exactly when it is inconsistent over some prime factor — total charge `2 ≢ 0 (mod m)` for any
/// `m ≥ 3` that does not divide `2` — and [`crate::modm::solve`] decides it through that factor with a
/// solver-free re-checkable certificate ([`crate::modm::is_refutation`]). This drives the **ring**
/// route (`ℤ/m` Gaussian via CRT), not the prime-field route, on a CNF that resolution still walls on.
pub fn mod_m_tseitin_expander(
    n: usize,
    m: u64,
    seed: u64,
) -> (Vec<crate::modp::ModpEquation>, DimacsCnf, ExpectedVerdict) {
    assert!(
        m >= 3,
        "the composite obstruction needs a modulus ≥ 3 (m=2 is the parity case, consistent here)"
    );
    mod_tseitin_expander_core(n, m, seed)
}

/// Shared construction for the total-charge-2 divergence obstruction over `ℤ/modulus` on a random
/// 3-regular expander, used by both the prime-field ([`mod_p_tseitin_expander`]) and composite-ring
/// ([`mod_m_tseitin_expander`]) families. Returns the `ℤ/modulus` divergence system, the equisatisfiable
/// one-hot Boolean CNF (`modulus` values per edge), and the UNSAT verdict. Charging two vertices makes
/// the total `2`, which is `≢ 0 (mod modulus)` for every `modulus ≥ 3` yet `≡ 0 (mod 2)` — so the parity
/// cut is blind to it and only Gaussian elimination over the right characteristic decides it.
fn mod_tseitin_expander_core(
    n: usize,
    modulus: u64,
    seed: u64,
) -> (Vec<crate::modp::ModpEquation>, DimacsCnf, ExpectedVerdict) {
    use crate::cdcl::Lit;
    use crate::modp::ModpEquation;
    assert!(
        n % 2 == 0 && n >= 4,
        "a 3-regular graph needs an even vertex count ≥ 4"
    );
    let edges = random_3regular(n, seed); // each (a, b) with a < b; orient a → b (tail a, head b)
    let ne = edges.len();
    let charge = |v: usize| u64::from(v == 0 || v == 1); // total charge 2: ≢0 mod modulus (≥3), ≡0 mod 2

    // The ℤ/modulus divergence system: +x_e at the tail, −x_e (= (modulus−1)·x_e) at the head.
    let mut eqs: Vec<ModpEquation> = Vec::new();
    for v in 0..n {
        let mut coeffs: Vec<(usize, u64)> = Vec::new();
        for (e, &(a, b)) in edges.iter().enumerate() {
            if a == v {
                coeffs.push((e, 1));
            } else if b == v {
                coeffs.push((e, modulus - 1));
            }
        }
        eqs.push(ModpEquation::new(coeffs, charge(v)));
    }

    // The equisatisfiable Boolean CNF: one-hot bit b(e, val) = "edge e takes value val" (var e*modulus+val).
    let bvar = |e: usize, val: u64| (e * modulus as usize + val as usize) as u32;
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    for e in 0..ne {
        clauses.push((0..modulus).map(|val| Lit::pos(bvar(e, val))).collect()); // ≥1 value
        for v1 in 0..modulus {
            for v2 in (v1 + 1)..modulus {
                clauses.push(vec![Lit::neg(bvar(e, v1)), Lit::neg(bvar(e, v2))]);
                // ≤1 value
            }
        }
    }
    // Per vertex, forbid every assignment of its incident edge-values whose signed sum ≢ charge.
    for v in 0..n {
        let incident: Vec<(usize, i64)> = edges
            .iter()
            .enumerate()
            .filter_map(|(e, &(a, b))| {
                if a == v {
                    Some((e, 1i64))
                } else if b == v {
                    Some((e, -1i64))
                } else {
                    None
                }
            })
            .collect();
        let d = incident.len();
        let want = charge(v) as i64;
        let pi = modulus as i64;
        for idx in 0..modulus.pow(d as u32) {
            let mut x = idx;
            let mut combo = vec![0u64; d];
            for slot in combo.iter_mut() {
                *slot = x % modulus;
                x /= modulus;
            }
            let s = incident
                .iter()
                .zip(&combo)
                .fold(0i64, |acc, (&(_, sign), &val)| acc + sign * val as i64);
            if (s.rem_euclid(pi)) != (want.rem_euclid(pi)) {
                clauses.push(
                    incident
                        .iter()
                        .zip(&combo)
                        .map(|(&(e, _), &val)| Lit::neg(bvar(e, val)))
                        .collect(),
                );
            }
        }
    }

    (
        eqs,
        DimacsCnf {
            num_vars: ne * modulus as usize,
            clauses,
        },
        ExpectedVerdict::Unsat,
    )
}

/// A **satisfiable** sibling of [`mod_p_tseitin_expander`] over the same 3-regular graph and the same
/// one-hot Boolean encoding, but with a charge distribution whose total `≡ 0 (mod p)`. The divergence
/// system `Σ_out x − Σ_in x ≡ charge(v)` is consistent exactly when the charges sum to zero (summing all
/// vertex equations telescopes the left side to `0`), so this instance is SAT — the control for the GF(p)
/// route's model-returning path. Charge: `+1` at vertex 0, `−1 (= p−1)` at vertex 1, zero elsewhere.
pub fn mod_p_consistent_onehot(
    n: usize,
    p: u64,
    seed: u64,
) -> (Vec<crate::modp::ModpEquation>, DimacsCnf, ExpectedVerdict) {
    use crate::cdcl::Lit;
    use crate::modp::ModpEquation;
    assert!(
        n % 2 == 0 && n >= 4,
        "a 3-regular graph needs an even vertex count ≥ 4"
    );
    assert!(p >= 3, "the mod-p one-hot encoding needs an odd prime");
    let edges = random_3regular(n, seed);
    let ne = edges.len();
    let charge = |v: usize| -> u64 {
        match v {
            0 => 1,
            1 => p - 1,
            _ => 0,
        }
    }; // total charge ≡ 0 (mod p) ⟹ consistent

    let mut eqs: Vec<ModpEquation> = Vec::new();
    for v in 0..n {
        let mut coeffs: Vec<(usize, u64)> = Vec::new();
        for (e, &(a, b)) in edges.iter().enumerate() {
            if a == v {
                coeffs.push((e, 1));
            } else if b == v {
                coeffs.push((e, p - 1));
            }
        }
        eqs.push(ModpEquation::new(coeffs, charge(v)));
    }

    let bvar = |e: usize, val: u64| (e * p as usize + val as usize) as u32;
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    for e in 0..ne {
        clauses.push((0..p).map(|val| Lit::pos(bvar(e, val))).collect());
        for v1 in 0..p {
            for v2 in (v1 + 1)..p {
                clauses.push(vec![Lit::neg(bvar(e, v1)), Lit::neg(bvar(e, v2))]);
            }
        }
    }
    for v in 0..n {
        let incident: Vec<(usize, i64)> = edges
            .iter()
            .enumerate()
            .filter_map(|(e, &(a, b))| {
                if a == v {
                    Some((e, 1i64))
                } else if b == v {
                    Some((e, -1i64))
                } else {
                    None
                }
            })
            .collect();
        let d = incident.len();
        let want = charge(v) as i64;
        let pi = p as i64;
        for idx in 0..p.pow(d as u32) {
            let mut x = idx;
            let mut combo = vec![0u64; d];
            for slot in combo.iter_mut() {
                *slot = x % p;
                x /= p;
            }
            let s = incident
                .iter()
                .zip(&combo)
                .fold(0i64, |acc, (&(_, sign), &val)| acc + sign * val as i64);
            if (s.rem_euclid(pi)) != (want.rem_euclid(pi)) {
                clauses.push(
                    incident
                        .iter()
                        .zip(&combo)
                        .map(|(&(e, _), &val)| Lit::neg(bvar(e, val)))
                        .collect(),
                );
            }
        }
    }

    (
        eqs,
        DimacsCnf {
            num_vars: ne * p as usize,
            clauses,
        },
        ExpectedVerdict::Sat,
    )
}

/// The **first-moment (counting) upper bound** on the random k-SAT satisfiability threshold:
/// `α*(k) = ln 2 / ln(2ᵏ / (2ᵏ − 1))`. Above this clause density the expected number of satisfying
/// assignments `E[X] = 2ⁿ(1 − 2⁻ᵏ)^{αn} = (2·(1 − 2⁻ᵏ)^α)ⁿ` has per-variable base `< 1`, so `E[X] → 0`
/// and the instance is UNSAT with high probability (Markov) — a **rigorous** upper bound on the true
/// threshold, exact and closed-form (no sampling). The sequence climbs as `2ᵏ ln 2 − (ln 2)/2`,
/// asymptotically **doubling per k**: `2.41, 5.19, 10.74, 21.83, 44.02, …` for `k = 2, 3, 4, 5, 6, …`.
/// The *true* thresholds (≈ 1, 4.27, 9.93, 21.1, …) lie below it; the gap → ½ as k → ∞ (the difference
/// between the first-moment bound `2ᵏln2 − ½ln2` and the sharp value `2ᵏln2 − (1+ln2)/2`).
pub fn ksat_threshold_first_moment_upper(k: u32) -> f64 {
    let pow = (1u64 << k) as f64;
    std::f64::consts::LN_2 / (pow / (pow - 1.0)).ln()
}

/// All `q`-element subsets of `{0, …, n−1}` in lexicographic order (the standard combinatorial
/// odometer). Returns empty when `q > n`. Used to build the hyperedge / clique families below; keep
/// `n` and `q` small, the count is `C(n, q)`.
fn combinations(n: usize, q: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    if q == 0 || q > n {
        return out;
    }
    let mut idx: Vec<usize> = (0..q).collect();
    loop {
        out.push(idx.clone());
        let mut i = q;
        loop {
            if i == 0 {
                return out;
            }
            i -= 1;
            if idx[i] != i + n - q {
                break;
            }
        }
        idx[i] += 1;
        for j in (i + 1)..q {
            idx[j] = idx[j - 1] + 1;
        }
    }
}

/// The **weak pigeonhole principle** `PHP^{holes}_{pigeons}`: `pigeons` pigeons into `holes` holes —
/// unsatisfiable exactly when `pigeons > holes`. Generalizes [`php`] (which is the tight
/// `holes = pigeons − 1` case) to any hole count, including the *weak* regime (`holes = 2·pigeons`,
/// `pigeons²`, …) used to probe how far symmetry breaking scales as the holes-to-pigeons ratio grows.
/// Same variable layout as [`php`] — "pigeon `p` in hole `h`" at index `p*holes + h` — so
/// `weak_php(n, n−1)` is byte-for-byte `php(n)`.
pub fn weak_php(pigeons: usize, holes: usize) -> (DimacsCnf, ExpectedVerdict) {
    let num_vars = pigeons * holes;
    let var = |p: usize, h: usize| Lit::pos((p * holes + h) as u32);
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    for p in 0..pigeons {
        clauses.push((0..holes).map(|h| var(p, h)).collect()); // each pigeon in ≥ 1 hole
    }
    for h in 0..holes {
        for p in 0..pigeons {
            for q in (p + 1)..pigeons {
                clauses.push(vec![var(p, h).negated(), var(q, h).negated()]); // ≤ 1 pigeon per hole
            }
        }
    }
    let verdict = if pigeons > holes {
        ExpectedVerdict::Unsat
    } else {
        ExpectedVerdict::Sat
    };
    (DimacsCnf { num_vars, clauses }, verdict)
}

/// The **functional pigeonhole principle** FPHP(n): `php(n)` strengthened so each pigeon sits in *at
/// most* one hole (the placement is a function, not a relation). Adds the "no pigeon in two holes"
/// clauses on top of PHP; still UNSAT, still `S_n × S_{n−1}`-symmetric, and the standard strengthening
/// used to check that symmetry breaking survives the extra functional clauses. Same layout as [`php`].
pub fn functional_php(n: usize) -> (DimacsCnf, ExpectedVerdict) {
    let holes = n.saturating_sub(1);
    let (mut cnf, _) = php(n);
    let var = |p: usize, h: usize| Lit::pos((p * holes + h) as u32);
    for p in 0..n {
        for h1 in 0..holes {
            for h2 in (h1 + 1)..holes {
                cnf.clauses
                    .push(vec![var(p, h1).negated(), var(p, h2).negated()]); // ≤ 1 hole per pigeon
            }
        }
    }
    (cnf, ExpectedVerdict::Unsat)
}

/// The **onto (bijective) pigeonhole principle** onto-FPHP(n): `functional_php(n)` further forced to
/// be *onto* — every hole receives at least one pigeon. The placement is now a bijection `n → n−1`,
/// which cannot exist; UNSAT. This is the hardest standard PHP variant for symmetry reasoning (it pins
/// both the pigeon and the hole side), and the maximal clause set over the shared [`php`] layout.
pub fn onto_php(n: usize) -> (DimacsCnf, ExpectedVerdict) {
    let holes = n.saturating_sub(1);
    let (mut cnf, _) = functional_php(n);
    let var = |p: usize, h: usize| Lit::pos((p * holes + h) as u32);
    for h in 0..holes {
        cnf.clauses.push((0..n).map(|p| var(p, h)).collect()); // each hole gets ≥ 1 pigeon (onto)
    }
    (cnf, ExpectedVerdict::Unsat)
}

/// The **modular counting principle** `Count_q(n)`: can the `n`-element set `{0,…,n−1}` be exactly
/// partitioned into blocks of size `q`? A Boolean per `q`-subset (hyperedge); clauses force every
/// element to lie in ≥ 1 chosen block and forbid any two *overlapping* blocks — so the chosen blocks
/// are a perfect `q`-cover (an exact partition). That exists iff `q ∣ n`, hence the formula is
/// **UNSAT exactly when `q ∤ n`**. This is the canonical *modular* family — the obstruction is a
/// counting argument mod `q` that resolution cannot make at low width (Ajtai; Beame–Pitassi), the
/// natural target of the GF(q) / mod-`m` rung. `q = 2` is perfect matching on the complete graph `K_n`
/// (UNSAT for odd `n`). The hyperedge count is `C(n, q)`, so keep `n`, `q` small.
pub fn mod_counting(n: usize, q: usize) -> (DimacsCnf, ExpectedVerdict) {
    assert!(q >= 2 && n >= q, "need a block size q ≥ 2 with n ≥ q");
    let edges = combinations(n, q);
    let num_vars = edges.len();
    let mut incident: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (e, edge) in edges.iter().enumerate() {
        for &v in edge {
            incident[v].push(e);
        }
    }
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    for inc in &incident {
        clauses.push(inc.iter().map(|&e| Lit::pos(e as u32)).collect()); // every element covered ≥ once
    }
    for e in 0..edges.len() {
        for f in (e + 1)..edges.len() {
            if edges[e].iter().any(|v| edges[f].contains(v)) {
                clauses.push(vec![Lit::neg(e as u32), Lit::neg(f as u32)]); // overlapping blocks are exclusive
            }
        }
    }
    let verdict = if n % q == 0 {
        ExpectedVerdict::Sat
    } else {
        ExpectedVerdict::Unsat
    };
    (DimacsCnf { num_vars, clauses }, verdict)
}

/// The exactly-one groups of [`mod_counting`]'s **linear encoding**: for each point, the variables of
/// its covering clause — the incident `q`-subsets, in the same variable order as [`mod_counting`].
/// Feed to `polycalc::exactly_one_linear_generators` for the degree-1 point generators plus overlap
/// pairs (the encoding the modular-counting degree lower bounds are stated against).
pub fn mod_counting_groups(n: usize, q: usize) -> Vec<Vec<u32>> {
    let (cnf, _) = mod_counting(n, q);
    cnf.clauses
        .iter()
        .filter(|c| c.iter().all(|l| l.is_positive()))
        .map(|c| c.iter().map(|l| l.var()).collect())
        .collect()
}

/// The edge layout of [`mod_counting`]: variable `e` is the `q`-subset `mod_counting_edges(n, q)[e]`
/// of the point set `{0,…,n−1}` — the map a witness-support predicate needs to read a monomial as a
/// set of blocks.
pub fn mod_counting_edges(n: usize, q: usize) -> Vec<Vec<usize>> {
    combinations(n, q)
}

/// The known two-colour Ramsey numbers `R(s, t)` for the small `(s, t)` we can actually build CNFs for
/// (symmetric: `R(s, t) = R(t, s)`); `None` when the value is still an open problem. `R(s, t)` is the
/// least `n` such that every red/blue colouring of `K_n`'s edges contains a red `K_s` or a blue `K_t`.
pub fn ramsey_number(s: usize, t: usize) -> Option<usize> {
    let (a, b) = (s.min(t), s.max(t));
    match (a, b) {
        (1, _) => Some(1),
        (2, b) => Some(b),
        (3, 3) => Some(6),
        (3, 4) => Some(9),
        (3, 5) => Some(14),
        (3, 6) => Some(18),
        (3, 7) => Some(23),
        (3, 8) => Some(28),
        (3, 9) => Some(36),
        (4, 4) => Some(18),
        (4, 5) => Some(25),
        _ => None,
    }
}

/// The **Ramsey formula** `Ramsey(s, t; n)`: 2-colour the edges of the complete graph `K_n` (one
/// Boolean per edge — true = red, false = blue) avoiding every red `K_s` and every blue `K_t`. For each
/// `s`-clique a clause forbids all its edges being red; for each `t`-clique a clause forbids all blue.
/// Such a colouring exists iff `n < R(s, t)`, so the formula is **UNSAT exactly when `n ≥ R(s, t)`**.
/// A genuinely different geometry from pigeonhole/parity — clique structure, not covering or counting —
/// and a classic CDCL stress family (`Ramsey(3,3;6)` is the smallest UNSAT case). Panics for `(s, t)`
/// whose Ramsey number is unknown (so the verdict is never guessed); see [`ramsey_number`].
pub fn ramsey(s: usize, t: usize, n: usize) -> (DimacsCnf, ExpectedVerdict) {
    assert!(s >= 2 && t >= 2 && n >= 2);
    let r = ramsey_number(s, t).expect("Ramsey number R(s,t) must be known to pin the verdict");
    let mut edge_id = std::collections::HashMap::new();
    for (e, pair) in combinations(n, 2).iter().enumerate() {
        edge_id.insert((pair[0], pair[1]), e as u32);
    }
    let num_vars = edge_id.len();
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    let clique_edges = |clique: &[usize]| -> Vec<u32> {
        combinations(clique.len(), 2)
            .iter()
            .map(|p| edge_id[&(clique[p[0]], clique[p[1]])])
            .collect()
    };
    for clique in combinations(n, s) {
        clauses.push(clique_edges(&clique).into_iter().map(Lit::neg).collect());
        // not all red
    }
    for clique in combinations(n, t) {
        clauses.push(clique_edges(&clique).into_iter().map(Lit::pos).collect());
        // not all blue
    }
    let verdict = if n >= r {
        ExpectedVerdict::Unsat
    } else {
        ExpectedVerdict::Sat
    };
    (DimacsCnf { num_vars, clauses }, verdict)
}

/// The **pebbling contradiction** on the pyramid DAG of the given `height` — the canonical resolution
/// *space* family (Ben-Sasson–Wigderson; Nordström). The pyramid has rows `0..=height` with row `r`
/// holding `r+1` nodes; each node `(r, i)` for `r < height` has the two predecessors `(r+1, i)` and
/// `(r+1, i+1)` below it. One Boolean per node "is pebbled": every source (bottom row) is asserted
/// pebbled (unit), pebbling propagates up (`pebbled(a) ∧ pebbled(b) → pebbled(parent)`), and the apex
/// `(0,0)` is asserted *un*pebbled — a contradiction, so UNSAT. Refutation size is linear (unit
/// propagation alone closes it), but refuting it in small *space* provably is not — the axis pigeonhole
/// and parity do not touch.
pub fn pebbling_pyramid(height: usize) -> (DimacsCnf, ExpectedVerdict) {
    let node = |r: usize, i: usize| (r * (r + 1) / 2 + i) as u32; // triangular id, apex = node(0,0) = 0
    let num_vars = (height + 1) * (height + 2) / 2;
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    for i in 0..=height {
        clauses.push(vec![Lit::pos(node(height, i))]); // sources (bottom row) are pebbled
    }
    for r in 0..height {
        for i in 0..=r {
            clauses.push(vec![
                Lit::neg(node(r + 1, i)),
                Lit::neg(node(r + 1, i + 1)),
                Lit::pos(node(r, i)),
            ]); // both children pebbled ⇒ this node pebbled
        }
    }
    clauses.push(vec![Lit::neg(node(0, 0))]); // the apex is asserted unpebbled — the contradiction
    (DimacsCnf { num_vars, clauses }, ExpectedVerdict::Unsat)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/families/tests.rs"]
mod tests;
