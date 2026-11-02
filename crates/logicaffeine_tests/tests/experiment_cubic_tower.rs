//! # RUNG 5 (the climb) — does the tower continue? degree-`d` char-`d` → `Λ^{d+1}`, `d`-torsion?
//!
//! The degree-2 UOV tangent obstruction is `Λ³` (triangles, `C(o,3)`), 2-torsion (§32–§35). The conjecture
//! this seeds: a degree-`d` form's oil-isotropy tangent, in characteristic `d`, has obstruction `Λ^{d+1}`
//! (`(d+1)`-cliques, `C(o,d+1)`), `d`-torsion. We test it *honestly* by building the natural degree-`d`
//! tangent system and reading its kernel across characteristics.
//!
//! Setup. The linear-in-`φ` part of `Q_i(Σ x_c(b_c+φ_c))` (with the oil-oil-…-oil block zero) collects, per
//! oil monomial, the coupling `D_i(φ_c, y_1,…,y_{d-1})` = `T[c][{i,y_1,…,y_{d-1}}]` under the natural coupling
//! (vinegar indexed by degree-`d` oil monomials, the analog of `e_{{i,j}}`). The condition indexed by
//! `(form i, oil d-multiset M)` is the coefficient of that monomial: `Σ` over ordered `(c,y_1,…,y_{d-1})` with
//! multiset `M`, of `T[c][sort({i,y_1,…,y_{d-1}})]`. For `d=2` this is exactly the polar+Dickson system
//! (kernel `C(o,3)`) — so the construction is validated before we climb.

fn mod_inv(a: i64, p: i64) -> i64 {
    let mut r = 1i64;
    let mut b = ((a % p) + p) % p;
    let mut e = p - 2;
    while e > 0 {
        if e & 1 == 1 {
            r = r * b % p;
        }
        b = b * b % p;
        e >>= 1;
    }
    r
}
/// Rank of an integer matrix reduced modulo the prime `p`.
fn rank_gfp(rows: &[Vec<i64>], ncols: usize, p: i64) -> usize {
    let mut m: Vec<Vec<i64>> = rows
        .iter()
        .map(|r| r.iter().map(|&x| x.rem_euclid(p)).collect())
        .collect();
    let mut rank = 0;
    for c in 0..ncols {
        if rank >= m.len() {
            break;
        }
        let Some(piv) = (rank..m.len()).find(|&i| m[i][c].rem_euclid(p) != 0) else {
            continue;
        };
        m.swap(rank, piv);
        let inv = mod_inv(m[rank][c], p);
        for x in m[rank].iter_mut() {
            *x = (*x * inv).rem_euclid(p);
        }
        for i in 0..m.len() {
            if i != rank && m[i][c].rem_euclid(p) != 0 {
                let f = m[i][c];
                for cc in 0..ncols {
                    m[i][cc] = (m[i][cc] - f * m[rank][cc]).rem_euclid(p);
                }
            }
        }
        rank += 1;
    }
    rank
}

fn binom(n: usize, k: usize) -> usize {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut num = 1u128;
    let mut den = 1u128;
    for i in 0..k {
        num *= (n - i) as u128;
        den *= (i + 1) as u128;
    }
    (num / den) as usize
}

/// All nondecreasing length-`d` sequences from `{0,…,o-1}` — the degree-`d` oil monomials (multisets).
fn d_multisets(o: usize, d: usize) -> Vec<Vec<usize>> {
    fn rec(o: usize, d: usize, start: usize, cur: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if cur.len() == d {
            out.push(cur.clone());
            return;
        }
        for x in start..o {
            cur.push(x);
            rec(o, d, x, cur, out);
            cur.pop();
        }
    }
    let mut out = Vec::new();
    rec(o, d, 0, &mut Vec::new(), &mut out);
    out
}

/// All length-`d` sequences from `{0,…,o-1}` (`o^d` of them).
fn all_tuples(o: usize, d: usize) -> Vec<Vec<usize>> {
    let mut out = vec![vec![]];
    for _ in 0..d {
        let mut next = Vec::new();
        for t in &out {
            for x in 0..o {
                let mut nt = t.clone();
                nt.push(x);
                next.push(nt);
            }
        }
        out = next;
    }
    out
}

/// The natural degree-`d` oil-isotropy tangent system as an integer matrix (rows = conditions, cols = the
/// `o · C(o+d-1,d)` variables `T[c][μ]`, `c` oil, `μ` a degree-`d` oil monomial).
fn degree_system(o: usize, d: usize) -> (Vec<Vec<i64>>, usize) {
    let ms = d_multisets(o, d);
    let index = |m: &[usize]| ms.iter().position(|x| x == m).unwrap();
    let nvar = o * ms.len();
    let var = |c: usize, mu: usize| mu * o + c;
    let tuples = all_tuples(o, d);
    let mut rows = Vec::new();
    for i in 0..o {
        for cond in &ms {
            let mut row = vec![0i64; nvar];
            for t in &tuples {
                let mut sorted = t.clone();
                sorted.sort_unstable();
                if &sorted == cond {
                    let c = t[0];
                    let mut vmono = vec![i];
                    vmono.extend_from_slice(&t[1..]);
                    vmono.sort_unstable();
                    row[var(c, index(&vmono))] += 1;
                }
            }
            rows.push(row);
        }
    }
    (rows, nvar)
}

fn kernel_dim(o: usize, d: usize, p: i64) -> usize {
    let (rows, nvar) = degree_system(o, d);
    nvar - rank_gfp(&rows, nvar, p)
}

#[test]
fn degree_two_reproduces_the_triangle_kernel_and_is_two_torsion() {
    // Validation: the construction at d=2 is exactly the polar+Dickson system — kernel C(o,3) in
    // characteristic 2, and 0 in every odd characteristic (the triangles are 2-torsion, §35).
    for o in 3..=6usize {
        assert_eq!(
            kernel_dim(o, 2, 2),
            binom(o, 3),
            "o={o}: deg-2 char-2 kernel = C(o,3) = Λ³"
        );
        for &p in &[3i64, 5, 7] {
            assert_eq!(
                kernel_dim(o, 2, p),
                0,
                "o={o}: deg-2 kernel = 0 over GF({p}) — the triangles are 2-torsion"
            );
        }
    }
}

#[test]
fn the_degree_three_tangent_is_rigid_above_three_with_torsion_at_two_and_three() {
    // THE HONEST TOWER LAW (degree 3). The naive conjecture (char-3 kernel = C(o,4) = Λ⁴ exactly) is FALSE;
    // the truth is richer and, once seen, cleaner:
    //   * characteristic > 3 (> degree):  kernel = 0   — the tangent is RIGID (generic-fiber generalizes);
    //   * characteristic 3 (= degree):     kernel = C(o,4) + (o−1)·C(o,2)  — the tetrahedra Λ⁴=C(o,4) ARE a
    //                                       summand (the tower continues), plus a char-3 term;
    //   * characteristic 2 (< degree):     kernel = o·C(o,3)   — a distinct small-prime degeneracy.
    // So a degree-d tangent carries torsion at EVERY prime p ≤ d, and the char-d piece contains Λ^{d+1}.
    for o in 4..=7usize {
        for &p in &[5i64, 7, 11] {
            assert_eq!(
                kernel_dim(o, 3, p),
                0,
                "o={o}: deg-3 RIGID over GF({p}) (char > degree)"
            );
        }
        assert_eq!(
            kernel_dim(o, 3, 2),
            o * binom(o, 3),
            "o={o}: deg-3 char-2 kernel = o·C(o,3) = {}",
            o * binom(o, 3)
        );
        let char3 = binom(o, 4) + (o - 1) * binom(o, 2);
        assert_eq!(
            kernel_dim(o, 3, 3),
            char3,
            "o={o}: deg-3 char-3 kernel = C(o,4) + (o−1)·C(o,2) = {char3} (Λ⁴ is a summand)"
        );
    }
}
