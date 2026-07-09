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
                out.push((0..k).map(|t| Lit::new(vars[t], (mask >> t) & 1 == 0)).collect());
            }
        }
    };
    gadget(&[s(0), x(0)], &mut clauses);
    for i in 1..n {
        gadget(&[s(i), s(i - 1), x(i)], &mut clauses);
    }
    clauses.push(vec![Lit::neg(s(n - 1))]);
    (DimacsCnf { num_vars: 2 * n, clauses }, ExpectedVerdict::Unsat)
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
    let verdict = if k < n { ExpectedVerdict::Unsat } else { ExpectedVerdict::Sat };
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
    assert!(n >= 4 && n % 2 == 0, "the parity argument needs an even board ≥ 4");
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
    let mut incident: std::collections::HashMap<usize, Vec<usize>> = std::collections::HashMap::new();
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
                    clauses.push(vec![Lit::pos(inc[i] as u32).negated(), Lit::pos(inc[j] as u32).negated()]); // ≤ once
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
                    clauses.push(vec![var(i, j).negated(), var(j, k).negated(), var(i, k)]); // transitivity
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
    DimacsCnf { num_vars: vars, clauses }
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
    DimacsCnf { num_vars: vars, clauses }
}

/// The CNF clauses encoding the parity constraint `v0 ⊕ v1 ⊕ … = rhs` over any arity (forbid every
/// wrong-parity assignment of the listed variables — `2^{|vs|-1}` clauses, each of width `|vs|`).
fn xor_clauses(vs: &[u32], rhs: bool) -> Vec<Vec<Lit>> {
    let mut out = Vec::new();
    for mask in 0u32..(1 << vs.len()) {
        let odd = mask.count_ones() % 2 == 1;
        if odd != rhs {
            // Forbid this assignment: a literal is true exactly when it differs from the mask bit.
            out.push(vs.iter().enumerate().map(|(i, &v)| Lit::new(v, (mask >> i) & 1 == 0)).collect());
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
pub fn random_kxor(k: usize, n: usize, m: usize, seed: u64) -> (Vec<crate::xorsat::XorEquation>, DimacsCnf) {
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
        eqs.push(crate::xorsat::XorEquation::new(vs.iter().map(|&v| v as usize).collect::<Vec<_>>(), *rhs));
        clauses.extend(xor_clauses(vs, *rhs));
    }
    (eqs, DimacsCnf { num_vars: n, clauses })
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
pub fn tseitin_expander(n: usize, seed: u64) -> (Vec<crate::xorsat::XorEquation>, DimacsCnf, ExpectedVerdict) {
    assert!(n % 2 == 0 && n >= 4, "a 3-regular graph needs an even vertex count ≥ 4");
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
                clauses.push((0..d).map(|i| Lit::new(inc[i] as u32, (mask >> i) & 1 == 0)).collect());
            }
        }
    }
    (eqs, DimacsCnf { num_vars: m, clauses }, ExpectedVerdict::Unsat)
}

/// The **Tseitin formula on a `w × n` grid** with odd total charge — UNSATISFIABLE, and the sharpest
/// indictment of resolution search there is. A grid has treewidth exactly `w` (a fixed constant,
/// independent of the length `n`), so a POLYNOMIAL-size, bounded-width resolution refutation provably
/// EXISTS. Yet CDCL solvers without Gaussian reasoning blow up on the parity regardless — they cannot
/// find the short proof that is known to exist — while Gaussian elimination over GF(2) decides it in
/// near-linear time. One Boolean per grid edge; per vertex, the XOR of its incident edges equals its
/// charge (vertex 0 charged, the rest not — an odd sum, hence inconsistent). Returns the XOR system
/// (for [`crate::xorsat`]) and the equisatisfiable CNF.
pub fn grid_tseitin(w: usize, n: usize) -> (Vec<crate::xorsat::XorEquation>, DimacsCnf, ExpectedVerdict) {
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
                clauses.push((0..d).map(|i| Lit::new(inc[i] as u32, (mask >> i) & 1 == 0)).collect());
            }
        }
    }
    (eqs, DimacsCnf { num_vars: num_edges, clauses }, ExpectedVerdict::Unsat)
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
    assert!(p >= 3, "the mod-p obstruction needs an odd prime (p=2 is the parity case, consistent here)");
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
    assert!(m >= 3, "the composite obstruction needs a modulus ≥ 3 (m=2 is the parity case, consistent here)");
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
    assert!(n % 2 == 0 && n >= 4, "a 3-regular graph needs an even vertex count ≥ 4");
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
                clauses.push(vec![Lit::neg(bvar(e, v1)), Lit::neg(bvar(e, v2))]); // ≤1 value
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
            let s = incident.iter().zip(&combo).fold(0i64, |acc, (&(_, sign), &val)| acc + sign * val as i64);
            if (s.rem_euclid(pi)) != (want.rem_euclid(pi)) {
                clauses.push(
                    incident.iter().zip(&combo).map(|(&(e, _), &val)| Lit::neg(bvar(e, val))).collect(),
                );
            }
        }
    }

    (eqs, DimacsCnf { num_vars: ne * modulus as usize, clauses }, ExpectedVerdict::Unsat)
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
    assert!(n % 2 == 0 && n >= 4, "a 3-regular graph needs an even vertex count ≥ 4");
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
            let s = incident.iter().zip(&combo).fold(0i64, |acc, (&(_, sign), &val)| acc + sign * val as i64);
            if (s.rem_euclid(pi)) != (want.rem_euclid(pi)) {
                clauses.push(
                    incident.iter().zip(&combo).map(|(&(e, _), &val)| Lit::neg(bvar(e, val))).collect(),
                );
            }
        }
    }

    (eqs, DimacsCnf { num_vars: ne * p as usize, clauses }, ExpectedVerdict::Sat)
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
    let verdict = if pigeons > holes { ExpectedVerdict::Unsat } else { ExpectedVerdict::Sat };
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
                cnf.clauses.push(vec![var(p, h1).negated(), var(p, h2).negated()]); // ≤ 1 hole per pigeon
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
    let verdict = if n % q == 0 { ExpectedVerdict::Sat } else { ExpectedVerdict::Unsat };
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
        combinations(clique.len(), 2).iter().map(|p| edge_id[&(clique[p[0]], clique[p[1]])]).collect()
    };
    for clique in combinations(n, s) {
        clauses.push(clique_edges(&clique).into_iter().map(Lit::neg).collect()); // not all red
    }
    for clique in combinations(n, t) {
        clauses.push(clique_edges(&clique).into_iter().map(Lit::pos).collect()); // not all blue
    }
    let verdict = if n >= r { ExpectedVerdict::Unsat } else { ExpectedVerdict::Sat };
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
mod tests {
    use super::*;
    use crate::cdcl::SolveResult;

    /// **The k-SAT threshold sequence — generalized and pinned down.** The first-moment upper bound
    /// `α*(k) = ln2 / ln(2ᵏ/(2ᵏ−1))` is a rigorous, exact, closed-form sequence: it increases, doubles
    /// per k (→ 2ᵏ ln2 − ½ln2), and genuinely upper-bounds satisfiability — above it the first-moment
    /// base is `< 1`, forcing E[X] → 0 (UNSAT whp by Markov). It brackets the cited true thresholds from
    /// above. No sampling: every claim is arithmetic on the exact formula.
    #[test]
    fn the_ksat_threshold_sequence_first_moment_upper_bound() {
        let ln2 = std::f64::consts::LN_2;
        let ub = ksat_threshold_first_moment_upper;

        // (1) The sequence matches the closed form (k = 2..=5, high precision).
        let known = [(2u32, 2.40942), (3, 5.19089), (4, 10.73970), (5, 21.83239)];
        for (k, want) in known {
            assert!((ub(k) - want).abs() < 5e-3, "α*({k}) = {} want {want}", ub(k));
        }

        // (2) Strictly increasing, and asymptotically doubling per k.
        for k in 2..=20 {
            assert!(ub(k + 1) > ub(k), "increasing at k={k}");
        }
        for k in 4..=20 {
            let ratio = ub(k + 1) / ub(k);
            assert!((1.9..=2.1).contains(&ratio), "ratio ≈ 2 at k={k}: {ratio}");
        }

        // (3) Converges to 2ᵏ ln2 − ½ln2, and stays below the leading term 2ᵏ ln2.
        for k in 2..=20 {
            let lead = (1u64 << k) as f64 * ln2;
            assert!(ub(k) < lead, "below the leading term 2ᵏln2 at k={k}");
            assert!((ub(k) - (lead - ln2 / 2.0)).abs() < 0.05, "→ 2ᵏln2 − ½ln2 at k={k}");
        }

        // (4) RIGOR: α* is exactly where the first-moment base crosses 1 — above it E[X]→0 (UNSAT whp),
        //     below it E[X]→∞. This is *why* it is an upper bound (Markov), not a fitted constant.
        for k in 2..=20 {
            let p = 1.0 - 2f64.powi(-(k as i32));
            let base = |a: f64| 2.0 * p.powf(a);
            assert!((base(ub(k)) - 1.0).abs() < 1e-9, "α*(k={k}) is exactly base = 1");
            assert!(base(ub(k) + 0.5) < 1.0, "above α*(k={k}): E[X] → 0");
            assert!(base(ub(k) - 0.5) > 1.0, "below α*(k={k}): E[X] → ∞");
        }

        // (5) The bound brackets the cited SHARP thresholds from above (the gap → ½ as k → ∞).
        for (k, sharp) in [(2u32, 1.0), (3, 4.267), (4, 9.931), (5, 21.117)] {
            assert!(ub(k) > sharp, "first-moment bound α*({k})={} exceeds the true threshold {sharp}", ub(k));
        }
    }

    /// **Summing the threshold sequence — interesting constants pop out.** The thresholds α*_k grow like
    /// 2ᵏ, so Σα*_k diverges; but their RECIPROCALS telescope, because `1/α*_k = −log₂(1−2⁻ᵏ)`, turning
    /// the sum into the log of a product:
    ///
    /// `Σ_{k≥1} 1/α*_k = −log₂ Π_{k≥1}(1−2⁻ᵏ) = −log₂ φ(½) ≈ 1.79192`,
    ///
    /// where `φ(½) = 0.2887880951` is the Euler function at ½ — *exactly* the asymptotic probability that
    /// a random matrix over **GF(2)** is invertible. So the reciprocal SAT-threshold sum lands on our
    /// PARITY rung's own constant: the counting thresholds and their GF(2) reciprocal-sum meet here. The
    /// ALTERNATING reciprocal sum telescopes likewise to `log₂ Π(1−2⁻ᵏ)^{(−1)ᵏ} ≈ 0.71513`. Both identities
    /// are *exact* (sum of logs = log of product), not numerical coincidences.
    #[test]
    fn reciprocal_threshold_sums_telescope_to_euler_and_gf2_constants() {
        let ln2 = std::f64::consts::LN_2;
        let recip = |k: u32| 1.0 / ksat_threshold_first_moment_upper(k);
        let (mut s, mut p, mut alt, mut palt) = (0.0f64, 1.0f64, 0.0f64, 1.0f64);
        for k in 1..=50u32 {
            let term = 1.0 - 2f64.powi(-(k as i32));
            s += recip(k);
            p *= term;
            // EXACT telescoping: the partial sum equals −log₂ of the partial product.
            assert!((s + p.ln() / ln2).abs() < 1e-9, "Σ1/α* telescopes to −log₂Π(1−2⁻ᵏ) at k={k}");
            let sign = if k % 2 == 1 { 1.0 } else { -1.0 };
            alt += sign * recip(k);
            palt *= term.powf(if k % 2 == 1 { -1.0 } else { 1.0 });
            assert!((alt - palt.ln() / ln2).abs() < 1e-9, "alternating sum telescopes to log₂Π(1−2⁻ᵏ)^((−1)ᵏ) at k={k}");
        }
        // The Euler / GF(2) random-matrix invertibility constant, and the sum it produces.
        let phi_half = 0.288_788_095_1;
        assert!((p - phi_half).abs() < 1e-9, "Π(1−2⁻ᵏ) → φ(½) = the GF(2) invertibility constant 0.28879");
        assert!((s - 1.791_916_824_7).abs() < 1e-6, "Σ 1/α*_k ≈ 1.79192");
        assert!((s + phi_half.ln() / ln2).abs() < 1e-6, "and it equals −log₂ φ(½) exactly");
        // The alternating reciprocal sum's constant.
        assert!((alt - 0.715_131_251_2).abs() < 1e-6, "Σ(−1)ᵏ⁺¹/α*_k ≈ 0.71513");
    }

    #[test]
    fn mutilated_chessboard_and_ordering_are_correctly_unsat() {
        // Both families are UNSAT by construction; the dispatcher must decide them correctly at small
        // n regardless of which route fires. (These are honest measuring families, not yet our
        // crushes — the contract here is correctness, not speed.)
        for n in [4, 6, 8] {
            let (cnf, v) = mutilated_chessboard(n);
            assert_eq!(v, ExpectedVerdict::Unsat);
            assert!(cnf.clauses.iter().all(|c| !c.is_empty()), "chessboard {n} has no empty clause");
            let solved = crate::solve::solve_structured(cnf.num_vars, &cnf.clauses);
            assert!(matches!(solved.answer, crate::solve::Answer::Unsat), "mutilated chessboard {n} must be UNSAT");
        }
        for n in [3, 4, 5, 6] {
            let (cnf, v) = ordering_principle(n);
            assert_eq!(v, ExpectedVerdict::Unsat);
            let solved = crate::solve::solve_structured(cnf.num_vars, &cnf.clauses);
            assert!(matches!(solved.answer, crate::solve::Answer::Unsat), "ordering principle GT({n}) must be UNSAT");
        }
    }

    #[test]
    fn tseitin_expander_is_unsat_and_gaussian_refutes_it() {
        // The XOR engine must refute the expander-Tseitin system with an independently-checkable
        // Gaussian refutation, and the equisatisfiable CNF must agree (UNSAT) by our CDCL solver at
        // a size small enough to still terminate.
        for seed in [1u64, 7, 42] {
            let (eqs, cnf, verdict) = tseitin_expander(10, seed);
            assert_eq!(verdict, ExpectedVerdict::Unsat);
            match crate::xorsat::solve(&eqs, cnf.num_vars) {
                crate::xorsat::XorOutcome::Unsat(refutation) => {
                    assert!(
                        crate::xorsat::is_refutation(&eqs, cnf.num_vars, &refutation),
                        "the Gaussian refutation must independently check"
                    );
                }
                crate::xorsat::XorOutcome::Sat(_) => panic!("expander-Tseitin must be UNSAT"),
            }
            // The CNF encoding is genuinely the same UNSAT problem.
            assert_eq!(cnf.into_solver().solve(), SolveResult::Unsat, "CNF encoding must be UNSAT too");
        }
    }

    #[test]
    fn mod_p_tseitin_is_refuted_over_gf_p_with_a_checkable_certificate() {
        // The parity crush at every prime: the same expander obstruction is UNSAT over GF(p) for every
        // odd prime — Gaussian elimination over the right field refutes it with an independently-
        // checkable certificate — and its Boolean CNF is genuinely the same UNSAT problem.
        for &p in &[3u64, 5, 7] {
            for seed in [1u64, 7] {
                let (eqs, cnf, verdict) = mod_p_tseitin_expander(6, p, seed);
                assert_eq!(verdict, ExpectedVerdict::Unsat);
                let edges = cnf.num_vars / p as usize; // GF(p) variables = one per edge
                match crate::modp::solve(&eqs, edges, p) {
                    crate::modp::ModpOutcome::Unsat(combo) => assert!(
                        crate::modp::is_refutation(&eqs, edges, p, &combo),
                        "the GF({p}) refutation must independently re-check"
                    ),
                    crate::modp::ModpOutcome::Sat(_) => panic!("mod-{p} obstruction must be UNSAT over GF({p})"),
                }
                assert_eq!(cnf.into_solver().solve(), SolveResult::Unsat, "the Boolean CNF must be UNSAT");
            }
        }
    }

    #[test]
    fn mod_p_obstruction_is_invisible_to_gf2() {
        // The "blind" half made explicit: reinterpret the SAME graph + charges as a GF(2) parity
        // system. Total charge 2 is even, so it is SATISFIABLE over GF(2) — the parity cut cannot see
        // the mod-p contradiction. Only the right characteristic decides it.
        let p = 3u64;
        let (eqs, cnf, _) = mod_p_tseitin_expander(6, p, 7);
        let edges = cnf.num_vars / p as usize;
        let gf2: Vec<crate::xorsat::XorEquation> = eqs
            .iter()
            .map(|eq| {
                let vars: Vec<usize> = eq.coeffs.iter().map(|&(v, _)| v).collect();
                crate::xorsat::XorEquation::new(vars, eq.rhs % 2 == 1)
            })
            .collect();
        assert!(
            matches!(crate::xorsat::solve(&gf2, edges), crate::xorsat::XorOutcome::Sat(_)),
            "over GF(2) the even-charge obstruction is satisfiable — the parity engine is blind to it"
        );
    }

    #[test]
    fn mod_m_tseitin_is_refuted_over_the_ring_with_a_checkable_certificate() {
        // The composite-modulus crush: the same total-charge-2 expander obstruction over ℤ/m (m=6=2·3,
        // m=15=3·5) is UNSAT — decided by CRT over the prime factors — with an independently
        // re-checkable certificate, and its mixed-radix one-hot CNF is genuinely the same UNSAT problem.
        for &m in &[6u64, 15] {
            for seed in [1u64, 7] {
                let (eqs, cnf, verdict) = mod_m_tseitin_expander(6, m, seed);
                assert_eq!(verdict, ExpectedVerdict::Unsat);
                let vars = cnf.num_vars / m as usize; // ring variables = one per edge
                match crate::modm::solve(&eqs, vars, m) {
                    Some(crate::modm::ModmOutcome::Unsat { modulus, combo }) => assert!(
                        crate::modm::is_refutation(&eqs, vars, modulus, &combo),
                        "the ℤ/{m} refutation must independently re-check (via its GF({modulus}) factor)"
                    ),
                    other => panic!("mod-{m} obstruction must be UNSAT over ℤ/{m}, got {other:?}"),
                }
                assert_eq!(cnf.into_solver().solve(), SolveResult::Unsat, "the Boolean CNF must be UNSAT");
            }
        }
    }

    /// **TRANSFER: the parity wall.** The same exponential-vs-polynomial separation we measured for
    /// pigeonhole, carried to a DIFFERENT resolution-hard family — Tseitin formulas over expander graphs
    /// (Urquhart 1987: every resolution refutation is `2^Ω(n)`). Our CDCL is pure resolution, so its conflict
    /// count on the CNF explodes. But the formula is just a linear system over GF(2): the `xorsat` Gaussian
    /// reasoner refutes it in polynomial time with an independently-checkable certificate (a subset of
    /// equations summing to `0 = 1`). Different symmetry (parity, not permutation), same collapse — measured.
    #[test]
    #[ignore = "heavy: CDCL on Tseitin is exponential — that's the wall. Charts it vs the Gaussian collapse."]
    fn tseitin_parity_wall_collapses_under_gaussian() {
        use crate::xorsat::{is_refutation, solve, XorOutcome};
        let seed = 42u64;
        let mut rows = vec![
            "  n | edges | CDCL: conflicts / time   | Gaussian     | certified".to_string(),
            "----+-------+--------------------------+--------------+----------".to_string(),
        ];
        for n in [16usize, 24, 32, 40, 48, 56] {
            let (eqs, cnf, verdict) = tseitin_expander(n, seed);
            assert_eq!(verdict, ExpectedVerdict::Unsat);

            let mut solver = cnf.into_solver();
            let ct = std::time::Instant::now();
            assert_eq!(solver.solve(), SolveResult::Unsat, "Tseitin CNF is UNSAT");
            let cdcl_time = ct.elapsed();
            let conflicts = solver.conflicts();

            let t = std::time::Instant::now();
            let out = solve(&eqs, cnf.num_vars);
            let gauss = t.elapsed();
            let certified = match &out {
                XorOutcome::Unsat(r) => is_refutation(&eqs, cnf.num_vars, r),
                XorOutcome::Sat(_) => false,
            };
            assert!(certified, "Gaussian gives a checkable refutation for n={n}");
            rows.push(format!("{n:3} | {:5} | {conflicts:10} {cdcl_time:>10?} | {gauss:>11?} | yes", cnf.num_vars));
        }
        let chart = rows.join("\n");
        eprintln!("\nTSEITIN PARITY WALL — resolution (CDCL) exponential vs Gaussian polynomial+certified\n{chart}\n");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../logs/derived_facts");
        if std::fs::create_dir_all(&dir).is_ok() {
            let _ = std::fs::write(dir.join("tseitin_parity_wall.txt"), format!("TSEITIN PARITY WALL — CDCL (resolution, 2^Ω(n)) vs Gaussian (poly, certified)\n\n{chart}\n"));
        }
    }



    /// **THE TWO GEOMETRIES OF HARDNESS.** The break into geometry: a resolution-hard UNSAT fact is a
    /// region of the Boolean cube that the clause-polytope cannot exclude, and *which* geometry excludes it
    /// is the whole story.
    ///
    /// * **Pigeonhole = convex geometry over ℝ.** The half-point `x ≡ ½` satisfies every *clause's* LP
    ///   relaxation (each clause has ≥ 2 literals, so its value is `len/2 ≥ 1`) — so resolution lives forever
    ///   inside a polytope that still contains `½`. Cutting planes *recovers* the strong cardinality cut
    ///   `Σ_p x_{p,h} ≤ 1` from the exclusion clique (which `½` violates: `n/2 > 1`), sums it against the
    ///   `Σ_h x_{p,h} ≥ 1` rows, and lands on `0 ≥ 1`: a **Farkas separating hyperplane**, the polytope is
    ///   *empty*. `refute_clausal` is exactly this.
    /// * **Parity = affine geometry over GF(2).** `x ≡ ½` *also* satisfies every Tseitin clause's LP — but
    ///   here there is **no** cardinality clique to recover, so the convex reasoner stays blind (the polytope
    ///   is genuinely non-empty; cutting planes is exponential too). The obstruction is a different geometry:
    ///   the all-ones functional lies in the GF(2) row space, which **Gaussian elimination** sees at once.
    ///
    /// Neither reasoner crosses over — convex refutes pigeonhole and not parity; GF(2) refutes parity and is
    /// not even defined on pigeonhole. Two geometries, two obstructions, provably incomparable. That is the
    /// geometric reason there is no single symmetry break for all of NP.
    #[test]
    fn the_two_geometries_of_hardness() {
        // A clause is satisfied by x ≡ ½ in the LP relaxation iff it has ≥ 2 literals (value = len·½ ≥ 1).
        let half_point_satisfies = |cnf: &DimacsCnf| cnf.clauses.iter().all(|c| c.len() >= 2);

        // BOTH clause-polytopes contain the half-point x ≡ ½ — so resolution, which only ever reasons inside
        // the clause polytope, can never exclude EITHER at the clause level. The separation must come from a
        // richer geometry, and which one differs by family.
        let (php_cnf, _) = php(8);
        let (eqs, tse_cnf, _) = tseitin_expander(12, 7);
        assert!(half_point_satisfies(&php_cnf), "x≡½ ∈ the PHP clause-polytope");
        assert!(half_point_satisfies(&tse_cnf), "x≡½ ∈ the Tseitin clause-polytope");

        // PIGEONHOLE — convex geometry over ℝ. Recover the cardinality cut (Σ_p x_{p,h} ≤ 1, which x≡½
        // violates) and sum against the rows (Σ_h x_{p,h} ≥ 1): `n ≤ Σx ≤ n−1`, i.e. 0 ≥ 1. That conic
        // combination IS a Farkas separating hyperplane — the integer (indeed the LP-cardinality) polytope is
        // empty. Our O(1) counting certificate is exactly this hyperplane's content.
        assert!(crate::pigeonhole::certify_pigeonhole_unsat(8, 7).is_some(), "convex/Farkas counting hyperplane refutes PHP(8)");

        // PARITY — affine geometry over GF(2). There is NO cardinality clique to recover (it is not a
        // pigeon/hole problem), so the convex certificate is simply undefined here — convex geometry is blind.
        // The obstruction is that the all-ones functional lies in the GF(2) row space; Gaussian elimination
        // sees it and emits an independently-checkable refutation.
        match crate::xorsat::solve(&eqs, tse_cnf.num_vars) {
            crate::xorsat::XorOutcome::Unsat(r) => assert!(crate::xorsat::is_refutation(&eqs, tse_cnf.num_vars, &r), "GF(2) geometry refutes parity, certified"),
            crate::xorsat::XorOutcome::Sat(_) => panic!("Tseitin must be UNSAT"),
        }
        // Two geometries (convex-ℝ vs affine-GF(2)), two obstructions, neither reasoner crossing over — the
        // geometric reason there is no single symmetry break for all of NP.
    }

    /// **COVERING ≠ COUNTABLE — the exact place the hypercube-cover/counting attack breaks.** SAT is a
    /// covering of the hypercube by subcubes; pigeonhole's covering collapses under counting. The tempting
    /// leap is "so every covering does." It does not, and here is the undeniable witness: **two coverings of
    /// the SAME hypercube, with byte-for-byte identical COUNTING profiles — same clause count, same
    /// clause-length multiset, same per-variable occurrence counts — yet one is SAT and one is UNSAT.**
    /// They are the same Tseitin graph with total parity flipped at one vertex. Every blocker still covers
    /// the same number of vertices; every counting invariant (`footprint_card`, `vertexEnergy`, global
    /// balance — the whole §G toolkit) returns identically on both. So no counting argument can decide them.
    /// The one bit that does — the GF(2) total parity — is invisible to counting and visible to Gaussian
    /// elimination. *Covering, yes. Countable, no.* That is precisely where `ThreeSATInP` cannot close.
    #[test]
    fn counting_is_provably_blind_two_covers_same_counts_opposite_answers() {
        use crate::xorsat::{solve, XorEquation, XorOutcome};
        // Build a Tseitin cover on a fixed 3-regular graph with a chosen per-vertex charge vector.
        let build = |edges: &[(usize, usize)], n: usize, charges: &[bool]| -> (Vec<XorEquation>, DimacsCnf) {
            let mut incident = vec![Vec::new(); n];
            for (e, &(a, b)) in edges.iter().enumerate() {
                incident[a].push(e);
                incident[b].push(e);
            }
            let (mut eqs, mut clauses) = (Vec::new(), Vec::new());
            for v in 0..n {
                let (inc, r, d) = (&incident[v], charges[v], incident[v].len());
                eqs.push(XorEquation::new(inc.clone(), r));
                for mask in 0u32..(1u32 << d) {
                    if ((mask.count_ones() % 2) == 1) != r {
                        clauses.push((0..d).map(|i| Lit::new(inc[i] as u32, (mask >> i) & 1 == 0)).collect());
                    }
                }
            }
            (eqs, DimacsCnf { num_vars: edges.len(), clauses })
        };

        let edges = super::random_3regular(12, 7);
        let (eqs_sat, cnf_sat) = build(&edges, 12, &vec![false; 12]); // total parity even → SAT
        let mut odd = vec![false; 12];
        odd[0] = true; // flip ONE vertex → total parity odd → UNSAT
        let (eqs_uns, cnf_uns) = build(&edges, 12, &odd);

        // The COUNTING profile is identical — every cardinality/footprint invariant is blind to the flip.
        let profile = |cnf: &DimacsCnf| {
            let mut lens: Vec<usize> = cnf.clauses.iter().map(|c| c.len()).collect();
            lens.sort_unstable();
            let mut occ = vec![0usize; cnf.num_vars];
            for c in &cnf.clauses {
                for l in c {
                    occ[l.var() as usize] += 1;
                }
            }
            occ.sort_unstable();
            (cnf.clauses.len(), lens, occ)
        };
        assert_eq!(profile(&cnf_sat), profile(&cnf_uns), "IDENTICAL counting profile — counting cannot tell them apart");

        // Yet GF(2) — the parity invariant — gives OPPOSITE verdicts. One bit, invisible to every count.
        assert!(matches!(solve(&eqs_sat, cnf_sat.num_vars), XorOutcome::Sat(_)), "even total parity ⇒ SAT");
        match solve(&eqs_uns, cnf_uns.num_vars) {
            XorOutcome::Unsat(r) => assert!(crate::xorsat::is_refutation(&eqs_uns, cnf_uns.num_vars, &r), "odd total parity ⇒ UNSAT, certified by GF(2)"),
            XorOutcome::Sat(_) => panic!("odd-parity Tseitin must be UNSAT"),
        }
    }

    #[test]
    fn php_has_the_expected_shape() {
        let (cnf, verdict) = php(4);
        assert_eq!(verdict, ExpectedVerdict::Unsat);
        assert_eq!(cnf.num_vars, 4 * 3);
        // 4 "at least one hole" clauses + 3 holes × C(4,2)=6 conflict clauses.
        assert_eq!(cnf.clauses.len(), 4 + 3 * 6);
    }

    #[test]
    fn php_is_unsatisfiable_for_small_n() {
        for n in 1..=5 {
            let (cnf, _) = php(n);
            assert_eq!(
                cnf.into_solver().solve(),
                SolveResult::Unsat,
                "PHP({n}) must be unsatisfiable"
            );
        }
    }

    #[test]
    #[ignore = "core benchmark on hard random 3-SAT — the general-instance (non-symmetric) baseline"]
    fn bench_core_on_random_3sat() {
        use std::time::Instant;
        for &(vars, ratio) in &[(50usize, 4.26), (75, 4.26), (100, 4.26), (125, 4.26)] {
            let nc = (vars as f64 * ratio) as usize;
            let cnf = random_3sat(vars, nc, 0xC0FFEE_1234);
            let mut s = cnf.into_solver();
            let t = Instant::now();
            let res = s.solve();
            let ms = t.elapsed().as_secs_f64() * 1e3;
            println!(
                "rand3sat(v={vars}, c={nc}): {res:?} in {ms:.1}ms — {} conflicts, {} learned clauses (unbounded!)",
                s.conflicts(),
                s.learned().len()
            );
        }
    }

    #[test]
    #[ignore = "A/B benchmark: LBD clause deletion ON vs OFF on hard random 3-SAT"]
    fn bench_lbd_reduction_ab() {
        use std::time::Instant;
        for &v in &[140usize, 160, 180, 200] {
            let nc = (v as f64 * 4.26) as usize;
            let cnf = random_3sat(v, nc, 0xBADC0DE_99);
            for on in [false, true] {
                let mut s = cnf.into_solver();
                s.set_reduce(on);
                let t = Instant::now();
                let res = s.solve();
                let ms = t.elapsed().as_secs_f64() * 1e3;
                println!(
                    "v={v} reduce={on:5}: {} in {ms:7.0}ms — {} conflicts, {} LIVE learned clauses",
                    if matches!(res, SolveResult::Sat(_)) { "SAT  " } else { "UNSAT" },
                    s.conflicts(),
                    s.live_learned()
                );
            }
        }
    }

    #[test]
    #[ignore = "parity grave-dance: dump expander-XOR CNF + time Gaussian (xorsat), pairs with Kissat loop"]
    fn dump_parity_and_time_xorsat() {
        use std::time::Instant;
        for n in [40usize, 60, 80, 100, 120] {
            let m = (n as f64 * 1.1) as usize;
            let (eqs, cnf) = parity_unsat(n, m, 0x9A2173_5C);
            std::fs::write(format!("/tmp/parity_{n}.cnf"), crate::dimacs::print(&cnf)).unwrap();
            // Sanity: our own CDCL also agrees it's UNSAT (equisatisfiable encoding).
            let t = Instant::now();
            let outcome = crate::xorsat::solve(&eqs, n);
            let us = t.elapsed().as_secs_f64() * 1e6;
            assert!(matches!(outcome, crate::xorsat::XorOutcome::Unsat(_)), "parity(n={n}) must be UNSAT");
            println!(
                "XORSAT parity(n={n}, m={m}): UNSAT in {us:.1}µs via Gaussian elimination  |  {} CNF clauses dumped for Kissat",
                cnf.clauses.len()
            );
        }
    }

    #[test]
    fn parity_unsat_is_genuinely_unsat() {
        // Small instance: both the XOR engine and our CDCL on the CNF must agree it's UNSAT.
        let (eqs, cnf) = parity_unsat(12, 14, 0x9A2173_5C);
        assert!(matches!(crate::xorsat::solve(&eqs, 12), crate::xorsat::XorOutcome::Unsat(_)));
        assert_eq!(cnf.into_solver().solve(), SolveResult::Unsat, "CNF encoding is UNSAT too");
    }

    #[test]
    fn clique_coloring_verdict_tracks_colors_vs_clique() {
        // K_n needs exactly n colors: UNSAT with fewer, SAT with enough.
        for n in 2..=4 {
            let (unsat, v_unsat) = clique_coloring(n, n - 1);
            assert_eq!(v_unsat, ExpectedVerdict::Unsat);
            assert_eq!(unsat.into_solver().solve(), SolveResult::Unsat, "K_{n} with {} colors", n - 1);
            let (sat, v_sat) = clique_coloring(n, n);
            assert_eq!(v_sat, ExpectedVerdict::Sat);
            assert!(matches!(sat.into_solver().solve(), SolveResult::Sat(_)), "K_{n} with {n} colors");
        }
    }

    #[test]
    fn clique_coloring_exposes_color_permutation_symmetry() {
        // The finder must discover the color group (a different symmetry kind than pigeon/hole):
        // swapping two colors across all vertices is an automorphism.
        let (cnf, _) = clique_coloring(3, 2);
        let gens = crate::symmetry_detect::find_generators(cnf.num_vars, &cnf.clauses);
        assert!(!gens.iter().all(|g| g.is_identity()), "color symmetry must be detected");
        for g in &gens {
            assert!(crate::symmetry_detect::perm_is_automorphism(&cnf.clauses, g));
        }
    }

    #[test]
    fn clique_coloring_is_refuted_with_certified_symmetry_breaking() {
        for (n, k) in [(3usize, 2usize), (4, 3)] {
            let (cnf, _) = clique_coloring(n, k);
            let r = crate::sym_certify::certified_unsat_auto(cnf.num_vars, &cnf.clauses);
            assert!(r.refuted, "K_{n} / {k} colors refuted");
            assert!(r.sbp_clauses >= 1, "color symmetry certified-broken");
            assert!(crate::pr::check_pr_refutation(cnf.num_vars, &cnf.clauses, &r.steps));
        }
    }

    /// The dispatcher decides `cnf` exactly as its ground-truth verdict says — and when SAT, the
    /// returned model is independently re-checked against every clause. The single oracle every new
    /// family below is held to.
    fn decides(cnf: &DimacsCnf, verdict: ExpectedVerdict) {
        let solved = crate::solve::solve_structured(cnf.num_vars, &cnf.clauses);
        match verdict {
            ExpectedVerdict::Unsat => {
                assert!(matches!(solved.answer, crate::solve::Answer::Unsat), "expected UNSAT, solver said SAT");
            }
            ExpectedVerdict::Sat => match &solved.answer {
                crate::solve::Answer::Sat(model) => assert!(
                    cnf.clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())),
                    "the returned SAT model must satisfy every clause"
                ),
                crate::solve::Answer::Unsat => panic!("expected SAT, solver said UNSAT"),
            },
        }
    }

    #[test]
    fn weak_php_generalizes_php_and_tracks_the_pigeon_hole_ratio() {
        // The tight case is byte-for-byte the existing PHP — same layout, same clauses.
        for n in 1..=5 {
            let (tight, tv) = weak_php(n, n.saturating_sub(1));
            let (base, bv) = php(n);
            assert_eq!(tight.num_vars, base.num_vars, "weak_php(n,n-1) shares PHP's variable count");
            assert_eq!(tight.clauses, base.clauses, "weak_php(n,n-1) is byte-identical to php(n)");
            assert_eq!(tv, bv);
        }
        // UNSAT iff pigeons > holes, across the tight, weak, and satisfiable regimes — solver-confirmed.
        for &(p, h) in &[(3usize, 2usize), (5, 4), (4, 7), (2, 3), (3, 3), (6, 2)] {
            let (cnf, v) = weak_php(p, h);
            assert_eq!(v, if p > h { ExpectedVerdict::Unsat } else { ExpectedVerdict::Sat }, "PHP^{h}_{p} verdict");
            decides(&cnf, v);
        }
    }

    #[test]
    fn functional_and_onto_php_are_unsat_strengthenings_of_php() {
        for n in 2..=5 {
            let (base, _) = php(n);
            let (func, fv) = functional_php(n);
            let (onto, ov) = onto_php(n);
            assert_eq!(fv, ExpectedVerdict::Unsat);
            assert_eq!(ov, ExpectedVerdict::Unsat);
            // Each strengthening is a clause-superset of the previous (it only ADDS constraints).
            assert!(func.clauses.starts_with(&base.clauses), "FPHP({n}) ⊇ PHP({n})");
            assert!(onto.clauses.starts_with(&func.clauses), "onto-FPHP({n}) ⊇ FPHP({n})");
            decides(&func, ExpectedVerdict::Unsat);
            decides(&onto, ExpectedVerdict::Unsat);
        }
        // Strictly more clauses once a pigeon can reach ≥ 2 holes (n ≥ 3).
        for n in 3..=5 {
            let (base, _) = php(n);
            let (func, _) = functional_php(n);
            let (onto, _) = onto_php(n);
            assert!(func.clauses.len() > base.clauses.len(), "FPHP adds functional clauses at n={n}");
            assert!(onto.clauses.len() > func.clauses.len(), "onto-FPHP adds surjectivity clauses at n={n}");
        }
    }

    #[test]
    fn mod_counting_is_unsat_exactly_when_q_does_not_divide_n() {
        // q=2 is perfect matching on K_n (UNSAT for odd n); q=3 is the mod-3 partition principle.
        for &(n, q) in &[(3usize, 2usize), (4, 2), (5, 2), (6, 2), (4, 3), (6, 3), (7, 3), (8, 4)] {
            let (cnf, v) = mod_counting(n, q);
            assert_eq!(v, if n % q == 0 { ExpectedVerdict::Sat } else { ExpectedVerdict::Unsat }, "Count_{q}({n})");
            assert!(cnf.clauses.iter().all(|c| !c.is_empty()), "Count_{q}({n}) has no empty clause");
            decides(&cnf, v);
        }
        // The variable count is exactly the number of q-subsets, C(n, q).
        let (c52, _) = mod_counting(5, 2);
        assert_eq!(c52.num_vars, 10, "C(5,2) = 10 edges of K_5");
    }

    #[test]
    fn ramsey_tracks_the_known_ramsey_numbers() {
        // The smallest UNSAT Ramsey instance and its satisfiable predecessor, plus an off-diagonal pair —
        // each bracketing R(s,t) from below (SAT) and at the boundary (UNSAT), solver-confirmed.
        for &(s, t, n) in &[(3usize, 3usize, 5usize), (3, 3, 6), (3, 4, 8), (3, 4, 9)] {
            let (cnf, v) = ramsey(s, t, n);
            let r = ramsey_number(s, t).unwrap();
            assert_eq!(v, if n >= r { ExpectedVerdict::Unsat } else { ExpectedVerdict::Sat }, "Ramsey({s},{t};{n}) vs R={r}");
            decides(&cnf, v);
        }
        // Diagonal Ramsey carries the colour-swap symmetry: flipping every edge's colour (negating every
        // literal) maps the clause set onto itself — red-K_3 bans become blue-K_3 bans and vice-versa.
        let (cnf, _) = ramsey(3, 3, 6);
        let key = |c: &Vec<Lit>, flip: bool| {
            let mut k: Vec<(u32, bool)> = c.iter().map(|l| (l.var(), l.is_positive() ^ flip)).collect();
            k.sort_unstable();
            k
        };
        let set: std::collections::HashSet<Vec<(u32, bool)>> = cnf.clauses.iter().map(|c| key(c, false)).collect();
        for c in &cnf.clauses {
            assert!(set.contains(&key(c, true)), "global colour-flip is an automorphism of diagonal Ramsey");
        }
    }

    #[test]
    fn random_kxor_is_guaranteed_unsat_for_every_seed_arity_and_size() {
        // The whole point of the maximal-rank construction: inconsistent for ANY seed (the naive
        // single-flip silently produced SAT instances near m ≈ n). Hammer it — both the GF(2) Gaussian
        // engine and the CDCL solver on the CNF must agree UNSAT on every instance.
        use crate::cdcl::SolveResult;
        for k in [2usize, 3, 4, 5] {
            for n in [8usize, 12, 16] {
                for seed in 0..24u64 {
                    let (eqs, cnf) = random_kxor(k, n, n, seed.wrapping_mul(0x1000193) ^ k as u64);
                    assert!(
                        matches!(crate::xorsat::solve(&eqs, cnf.num_vars), crate::xorsat::XorOutcome::Unsat(_)),
                        "k={k} n={n} seed={seed}: maximal-rank k-XOR must be UNSAT over GF(2)"
                    );
                    assert_eq!(cnf.into_solver().solve(), SolveResult::Unsat, "k={k} n={n} seed={seed}: CNF must be UNSAT");
                }
            }
        }
    }

    #[test]
    fn random_kxor_generalizes_parity_and_gaussian_refutes_every_arity() {
        // k=3 reproduces parity_unsat byte-for-byte (the lift-and-shift is behaviour-preserving).
        for seed in [1u64, 7, 0x9A2173_5C] {
            let (_, a) = parity_unsat(12, 14, seed);
            let (_, b) = random_kxor(3, 12, 14, seed);
            assert_eq!(a.clauses, b.clauses, "parity_unsat is exactly random_kxor(k=3)");
        }
        // Every arity stays UNSAT and the Gaussian (GF(2)) refutation independently checks; the CNF agrees.
        for k in [2usize, 4, 5] {
            let (eqs, cnf) = random_kxor(k, 14, 16, 0xC0FFEE ^ k as u64);
            match crate::xorsat::solve(&eqs, cnf.num_vars) {
                crate::xorsat::XorOutcome::Unsat(r) => assert!(
                    crate::xorsat::is_refutation(&eqs, cnf.num_vars, &r),
                    "{k}-XOR Gaussian refutation must re-check"
                ),
                crate::xorsat::XorOutcome::Sat(_) => panic!("planted-then-flipped {k}-XOR must be UNSAT"),
            }
            assert_eq!(cnf.into_solver().solve(), SolveResult::Unsat, "{k}-XOR CNF encoding is UNSAT");
        }
    }

    #[test]
    fn pebbling_pyramid_is_unsat_with_the_expected_triangular_shape() {
        for h in 1..=5 {
            let (cnf, v) = pebbling_pyramid(h);
            assert_eq!(v, ExpectedVerdict::Unsat);
            let nodes = (h + 1) * (h + 2) / 2;
            assert_eq!(cnf.num_vars, nodes, "pyramid of height {h} has T(h+1) nodes");
            // sources (h+1 units) + propagation (one per non-source node) + 1 apex unit = nodes + 1.
            assert_eq!(cnf.clauses.len(), nodes + 1, "pebbling({h}) clause count = nodes + 1");
            assert!(cnf.clauses.iter().all(|c| !c.is_empty()), "no empty clause");
            decides(&cnf, ExpectedVerdict::Unsat);
        }
    }
}
