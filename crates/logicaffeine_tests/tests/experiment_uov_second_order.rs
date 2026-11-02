//! # SECOND-ORDER: is the char-2 isotropy variety NON-REDUCED at the oil space?
//!
//! §32–§35 found a `C(o,3)`-dimensional tangent (the triangles) at the symmetric coupling. The natural next
//! question — the one an algebraic geometer asks — is whether those tangent directions are *real* (the
//! variety is a smooth `C(o,3)`-dimensional family of equivalent oil spaces) or *phantom* (obstructed at
//! second order, so the oil space is a reduced isolated point carrying a non-reduced, nilpotent scheme
//! structure). The answer is the second fundamental form.
//!
//! Parametrize planes near `O` by `φ ∈ Hom(O, W)` (vinegar `W`); isotropy is the quadratic system
//!   `G_i(φ)(j,k) = B_i(b_j,φ^{(k)}) + B_i(b_k,φ^{(j)}) + Q_i(φ^{(j)},φ^{(k)}) = 0`,
//! whose LINEAR part is the tangent `S` (§32) and whose QUADRATIC part uses the vinegar block `Q_i`. A tangent
//! `Φ ∈ ker S` integrates to 2nd order iff its quadratic image `v_Φ = (Q_i(Φ^{(j)},Φ^{(k)}))` lies in
//! `image(S)`. The **obstruction** is `ob(Φ) = v_Φ mod image(S) ∈ coker S`. We measure the rank of `ob` on the
//! triangle modes: rank `= C(o,3)` means every triangle direction is obstructed — the tangent cone collapses
//! to a point and the scheme is non-reduced.

// ── GF(256), identical engine to experiment_uov_split_proof / generic_fiber ──────────────────────
fn tables() -> (Vec<u16>, Vec<u16>) {
    let mut exp = vec![0u16; 512];
    let mut log = vec![0u16; 256];
    let mut x: u16 = 1;
    for i in 0..255 {
        exp[i] = x;
        log[x as usize] = i as u16;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= 0x11B;
        }
    }
    for i in 255..512 {
        exp[i] = exp[i - 255];
    }
    (exp, log)
}
fn mul(a: u16, b: u16, e: &[u16], l: &[u16]) -> u16 {
    if a == 0 || b == 0 {
        0
    } else {
        e[l[a as usize] as usize + l[b as usize] as usize]
    }
}
fn inv(a: u16, e: &[u16], l: &[u16]) -> u16 {
    e[255 - l[a as usize] as usize]
}
fn grank(mut m: Vec<Vec<u16>>, e: &[u16], l: &[u16]) -> usize {
    let rows = m.len();
    if rows == 0 {
        return 0;
    }
    let cols = m[0].len();
    let mut r = 0;
    for c in 0..cols {
        if r >= rows {
            break;
        }
        let Some(p) = (r..rows).find(|&i| m[i][c] != 0) else {
            continue;
        };
        m.swap(r, p);
        let iv = inv(m[r][c], e, l);
        for x in m[r].iter_mut() {
            *x = mul(*x, iv, e, l);
        }
        for i in 0..rows {
            if i != r && m[i][c] != 0 {
                let f = m[i][c];
                for cc in 0..cols {
                    m[i][cc] ^= mul(f, m[r][cc], e, l);
                }
            }
        }
        r += 1;
    }
    r
}

fn pair_index(o: usize) -> Vec<(usize, usize)> {
    let mut ps = Vec::new();
    for a in 0..o {
        for b in a..o {
            ps.push((a, b));
        }
    }
    ps
}
fn pidx(pairs: &[(usize, usize)], a: usize, b: usize) -> usize {
    pairs
        .iter()
        .position(|&p| p == (a.min(b), a.max(b)))
        .unwrap()
}

/// The linear tangent `S` for the pair coupling `r_i^{(j)} = e_{{i,j}}` (rows: per form, polar `j<k` then
/// Dickson `j`; columns: `Φ[a][c]` at `a*o+c`). Identical layout to the split proof's oil system.
fn tangent(o: usize, v: usize, pairs: &[(usize, usize)]) -> Vec<Vec<u16>> {
    let col = |a: usize, c: usize| a * o + c;
    let mut rows = Vec::new();
    for i in 0..o {
        for j in 0..o {
            for k in j + 1..o {
                let mut row = vec![0u16; v * o];
                // B_i(b_j, φ^{(k)}) = φ^{(k)}[{i,j}] ; B_i(b_k, φ^{(j)}) = φ^{(j)}[{i,k}]
                row[col(pidx(pairs, i, j), k)] ^= 1;
                row[col(pidx(pairs, i, k), j)] ^= 1;
                rows.push(row);
            }
        }
        for j in 0..o {
            let mut row = vec![0u16; v * o];
            row[col(pidx(pairs, i, j), j)] ^= 1; // Dickson: B_i(b_j, φ^{(j)}) = φ^{(j)}[{i,j}]
            rows.push(row);
        }
    }
    rows
}

/// The cyclic triple mode of `{a,b,c}` as columns `Φ^{(k)} ∈ 𝔽^v` (flat index `a*o+c`).
fn triple_mode(o: usize, pairs: &[(usize, usize)], a: usize, b: usize, c: usize) -> Vec<u16> {
    let col = |p: usize, k: usize| p * o + k;
    let mut m = vec![0u16; pairs.len() * o];
    m[col(pidx(pairs, b, c), a)] = 1;
    m[col(pidx(pairs, a, c), b)] = 1;
    m[col(pidx(pairs, a, b), c)] = 1;
    m
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u16 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 40) & 0xFF) as u16
    }
}

/// `o` random upper-triangular vinegar blocks `U_i` (`v×v`); `Q_i(w) = Σ_{a≤b} U_i[a][b] w_a w_b`.
fn vinegar_blocks(o: usize, v: usize, seed: u64) -> Vec<Vec<Vec<u16>>> {
    let mut rng = Lcg(seed);
    (0..o)
        .map(|_| {
            (0..v)
                .map(|a| {
                    (0..v)
                        .map(|b| if b >= a { rng.next() } else { 0 })
                        .collect()
                })
                .collect()
        })
        .collect()
}

/// The second-order quadratic image `v_Φ` of a tangent `Φ`, in the SAME row layout as `tangent`.
/// polar `(i,j<k)`: `B_i(Φ^{(j)},Φ^{(k)}) = Σ_{a<b} U_i[a][b](Φ^{(j)}_a Φ^{(k)}_b + Φ^{(j)}_b Φ^{(k)}_a)`;
/// Dickson `(i,j)`: `Q_i(Φ^{(j)}) = Σ_{a≤b} U_i[a][b] Φ^{(j)}_a Φ^{(j)}_b`.
fn quadratic_image(
    o: usize,
    v: usize,
    u: &[Vec<Vec<u16>>],
    phi: &[u16],
    e: &[u16],
    l: &[u16],
) -> Vec<u16> {
    let comp = |k: usize, a: usize| phi[a * o + k];
    let bil = |i: usize, w: &dyn Fn(usize) -> u16, w2: &dyn Fn(usize) -> u16| {
        let mut s = 0u16;
        for a in 0..v {
            for b in a + 1..v {
                s ^= mul(
                    u[i][a][b],
                    mul(w(a), w2(b), e, l) ^ mul(w(b), w2(a), e, l),
                    e,
                    l,
                );
            }
        }
        s
    };
    let quad = |i: usize, w: &dyn Fn(usize) -> u16| {
        let mut s = 0u16;
        for a in 0..v {
            for b in a..v {
                s ^= mul(u[i][a][b], mul(w(a), w(b), e, l), e, l);
            }
        }
        s
    };
    let mut out = Vec::new();
    for i in 0..o {
        for j in 0..o {
            for k in j + 1..o {
                out.push(bil(i, &|a| comp(j, a), &|a| comp(k, a)));
            }
        }
        for j in 0..o {
            out.push(quad(i, &|a| comp(j, a)));
        }
    }
    out
}

fn transpose(m: &[Vec<u16>]) -> Vec<Vec<u16>> {
    let (r, c) = (m.len(), m[0].len());
    (0..c).map(|j| (0..r).map(|i| m[i][j]).collect()).collect()
}

#[test]
fn the_char2_isotropy_scheme_is_non_reduced_the_triangles_are_phantom() {
    // The result: for a generic vinegar block, EVERY triangle tangent direction is obstructed at second
    // order — the second fundamental form has rank C(o,3) on the triangle modes. So the tangent cone is a
    // single point: the oil space is a reduced isolated point carrying a non-reduced (nilpotent) scheme of
    // tangent dimension C(o,3). The C(o,3) "extra directions" are phantom, entirely 2-torsion nilpotents.
    let (e, l) = tables();
    for o in 3..=6usize {
        let v = o * (o + 1) / 2;
        let pairs = pair_index(o);
        let s = tangent(o, v, &pairs);
        let n = v * o;
        let s_rank = grank(s.clone(), &e, &l);
        assert_eq!(
            n - s_rank,
            c3(o),
            "o={o}: tangent kernel = C(o,3) (the triangles)"
        );

        // image(S) as rows (= column space, via transpose); append each triangle's quadratic image and
        // measure how many are independent modulo image(S) = the rank of the obstruction map.
        let img = transpose(&s); // columns of S span image(S) ⊆ condition space (length n)
        let img_rank = grank(img.clone(), &e, &l);

        // median obstruction rank over several generic vinegar blocks.
        let mut ranks: Vec<usize> = (0..5)
            .map(|sd| {
                let u = vinegar_blocks(o, v, 0xC0FFEE + sd * 97 + (o * 31) as u64);
                let mut aug = img.clone();
                for a in 0..o {
                    for b in a + 1..o {
                        for cc in b + 1..o {
                            aug.push(quadratic_image(
                                o,
                                v,
                                &u,
                                &triple_mode(o, &pairs, a, b, cc),
                                &e,
                                &l,
                            ));
                        }
                    }
                }
                grank(aug, &e, &l) - img_rank // independent obstructions among the C(o,3) modes
            })
            .collect();
        ranks.sort_unstable();
        let ob_rank = ranks[ranks.len() / 2];
        eprintln!(
            "o={o}: C(o,3)={:2}  obstruction rank = {ob_rank}  {}",
            c3(o),
            if ob_rank == c3(o) {
                "→ FULLY OBSTRUCTED (non-reduced, tangent cone = point)"
            } else {
                "→ partially integrates"
            }
        );
        assert_eq!(
            ob_rank,
            c3(o),
            "o={o}: all C(o,3) triangle directions obstructed ⟹ non-reduced scheme"
        );
    }
}

fn c3(o: usize) -> usize {
    o * o.saturating_sub(1) * o.saturating_sub(2) / 6
}
