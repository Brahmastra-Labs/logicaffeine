//! Nullstellensatz over **any prime field `GF(p)` and over `GF(4)`** — the characteristic axis of the
//! algebraic proof system that [`crate::polycalc`] fixes at `GF(2)`.
//!
//! Each characteristic is a genuinely *different* proof system: the mod-`p` counting principles fall to
//! degree 1 over `GF(p)` yet carry growing degree over `GF(q)` for every other prime `q` — the pairwise
//! incomparability that makes "which field?" a real dial, not a convention. Extension fields, by
//! contrast, add nothing: a certificate over `GF(pᵏ)` projects coefficient-wise through any `GF(p)`-linear
//! functional fixing `1`, so NS degree depends only on the characteristic (the `GF(4)` engine here exists
//! to *prove* that collapse, constructively). The `GF(2)` engine stays as the specialized fast path — its
//! coefficient-free bitset representation packs 64 basis columns per word — and this general engine is
//! pinned to it by a `p = 2` differential test.
//!
//! Everything [`crate::polycalc`] certifies has its analogue here, now with explicit coefficients:
//! signed clause false-indicators (`1 − x`, which characteristic 2 silently conflates with `1 + x`),
//! the field-generic partition of unity (`(1 − x) + x = 1` holds in every field, so constructive
//! completeness is not a `GF(2)` artifact), degree-`d` refutation decisions, and dual-witness degree
//! lower bounds with zero-trust re-checking.

use crate::cdcl::Lit;
pub use crate::polycalc::Mono;
use std::collections::BTreeMap;

/// The coefficient field of the general engine. Elements are `u64`-encoded: `Prime(p)` uses the residues
/// `0..p`; `Gf4` uses 2-bit pairs `a + b·ω` in the basis `{1, ω}` with `ω² = ω + 1` (value `a | b<<1`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NsField {
    Prime(u64),
    Gf4,
}

/// A multilinear polynomial over an [`NsField`]: monomial → nonzero coefficient (absent = 0). The
/// coefficient-explicit generalization of [`crate::polycalc::Poly`], whose `GF(2)` monomial-set form is
/// the special case "every present coefficient is 1".
pub type GfpPoly = BTreeMap<Mono, u64>;

impl NsField {
    /// The number of field elements (`p`, resp. 4).
    pub fn order(self) -> u64 {
        match self {
            NsField::Prime(p) => p,
            NsField::Gf4 => 4,
        }
    }

    /// The field characteristic (`p`, resp. 2) — the only parameter NS degree actually depends on.
    pub fn characteristic(self) -> u64 {
        match self {
            NsField::Prime(p) => p,
            NsField::Gf4 => 2,
        }
    }

    pub fn add(self, a: u64, b: u64) -> u64 {
        match self {
            NsField::Prime(p) => (a % p + b % p) % p,
            NsField::Gf4 => (a ^ b) & 3,
        }
    }

    pub fn neg(self, a: u64) -> u64 {
        match self {
            NsField::Prime(p) => (p - a % p) % p,
            NsField::Gf4 => a & 3,
        }
    }

    pub fn sub(self, a: u64, b: u64) -> u64 {
        self.add(a, self.neg(b))
    }

    pub fn mul(self, a: u64, b: u64) -> u64 {
        match self {
            NsField::Prime(p) => (a % p) * (b % p) % p,
            NsField::Gf4 => {
                // (a0 + a1ω)(b0 + b1ω) with ω² = ω + 1.
                let (a0, a1, b0, b1) = (a & 1, (a >> 1) & 1, b & 1, (b >> 1) & 1);
                let c0 = (a0 & b0) ^ (a1 & b1);
                let c1 = (a0 & b1) ^ (a1 & b0) ^ (a1 & b1);
                c0 | (c1 << 1)
            }
        }
    }

    /// Multiplicative inverse of a nonzero element (Fermat over `Prime(p)`; the 4-entry table over `Gf4`).
    pub fn inv(self, a: u64) -> u64 {
        match self {
            NsField::Prime(p) => {
                let a = a % p;
                assert!(a != 0, "the zero element has no inverse");
                let (mut base, mut e, mut r) = (a, p - 2, 1u64 % p);
                while e > 0 {
                    if e & 1 == 1 {
                        r = r * base % p;
                    }
                    base = base * base % p;
                    e >>= 1;
                }
                r
            }
            NsField::Gf4 => {
                assert!(a & 3 != 0, "the zero element has no inverse");
                [0, 1, 3, 2][(a & 3) as usize] // ω·(ω+1) = ω² + ω = 1
            }
        }
    }

    /// The image of an integer under the unique ring map `ℤ → F` — `n mod characteristic`. This is how
    /// group orders and counting arguments enter the field (`|G| = 0` in `F` iff `char | |G|`).
    pub fn embed_int(self, n: u128) -> u64 {
        (n % self.characteristic() as u128) as u64
    }
}

/// Add `c · m` into `p`, dropping the monomial when its coefficient cancels to zero — the coefficient
/// analogue of the `GF(2)` engine's `toggle`.
fn add_term(f: NsField, p: &mut GfpPoly, m: Mono, c: u64) {
    let c = f.add(c, 0);
    if c == 0 {
        return;
    }
    let e = p.entry(m).or_insert(0);
    *e = f.add(*e, c);
    if *e == 0 {
        p.remove(&m);
    }
}

/// Multilinear product: `x·x = x` collapses variable sets by OR, and — the correctness subtlety `GF(2)`
/// hides — colliding monomials **add coefficients**.
fn poly_mul(f: NsField, a: &GfpPoly, b: &GfpPoly) -> GfpPoly {
    let mut r = GfpPoly::new();
    for (&ma, &ca) in a {
        for (&mb, &cb) in b {
            add_term(f, &mut r, ma | mb, f.mul(ca, cb));
        }
    }
    r
}

/// The degree of a multilinear polynomial: its largest monomial's popcount (0 for the zero polynomial).
pub fn gfp_poly_degree(p: &GfpPoly) -> usize {
    p.keys().map(|m| m.count_ones() as usize).max().unwrap_or(0)
}

/// The clause polynomial over `f` — the **signed** false-indicator: a positive literal `x` contributes
/// `1 − x` (over `GF(3)`: `1 + 2x`, *not* `1 + x` — over `GF(2)` the two coincide, which is exactly the
/// sign the general engine must make explicit), a negative literal `¬x` contributes `x`. The product is
/// `1` exactly on the clause's falsifying corners and `0` elsewhere, over every field.
pub fn clause_polynomial_gfp(f: NsField, clause: &[Lit]) -> GfpPoly {
    let mut p: GfpPoly = [(0u64, 1u64)].into_iter().collect();
    for l in clause {
        let bit = 1u64 << l.var();
        let indicator: GfpPoly = if l.is_positive() {
            [(0u64, 1u64), (bit, f.neg(1))].into_iter().collect() // 1 − x
        } else {
            [(bit, 1u64)].into_iter().collect() // x
        };
        p = poly_mul(f, &p, &indicator);
    }
    p
}

/// The single-point indicator `δ_a = Π_{a_i=1} x_i · Π_{a_i=0} (1 − x_i)`: expanded, `Σ_{T ⊆ zeros}
/// (−1)^{|T|} x^{ones ∪ T}` — the signed subset walk whose `GF(2)` shadow is sign-free.
fn point_indicator_gfp(f: NsField, a: u64, num_vars: usize) -> GfpPoly {
    let mask = (1u64 << num_vars).wrapping_sub(1);
    let ones = a & mask;
    let zeros = !a & mask;
    let mut p = GfpPoly::new();
    let mut sub = zeros;
    loop {
        let sign = if sub.count_ones() % 2 == 0 { 1 } else { f.neg(1) };
        p.insert(ones | sub, sign); // masks are distinct (T ⊆ zeros, disjoint from ones) — no cancellation
        if sub == 0 {
            break;
        }
        sub = (sub - 1) & zeros;
    }
    p
}

/// The partition-of-unity **atom** on variable `v`: `(1 − x_v) + x_v`, the constant `1` in **every**
/// field — the identity is characteristic-free, so the completeness construction it powers is not a
/// `GF(2)` artifact.
pub fn pou_atom_gfp(f: NsField, v: usize) -> GfpPoly {
    let mut atom: GfpPoly = [(0u64, 1u64), (1u64 << v, f.neg(1))].into_iter().collect();
    add_term(f, &mut atom, 1u64 << v, 1);
    atom
}

/// The partition of unity `Σ_{a ∈ {0,1}ⁿ} δ_a` over `f`, by direct summation — the constant `1` at every
/// `n` over every field ([`pou_atom_gfp`] is the closed-form reason).
pub fn partition_of_unity_gfp(f: NsField, n: usize) -> GfpPoly {
    let mut sum = GfpPoly::new();
    for a in 0..(1u64 << n) {
        for (m, c) in point_indicator_gfp(f, a, n) {
            add_term(f, &mut sum, m, c);
        }
    }
    sum
}

/// Solve a linear system `Σ aᵢ·x_{vᵢ} = rhs` over `f` by incremental-echelon Gaussian elimination with
/// modular-inverse pivot normalization. Each equation is a sparse coefficient vector over `nvars`
/// unknowns plus its right-hand side. Returns a solution (free variables 0) or `None` on inconsistency.
/// The field-general sibling of the `GF(2)` engine's bit-packed `gf2_solve`: rows join an echelon basis
/// one at a time (memory stays `O(rank · nvars)` however many equations stream through), each stored row
/// is pivot-normalized with every entry strictly below its pivot column, so back-substitution in
/// increasing pivot order reads a solution off directly.
pub fn gfp_solve(f: NsField, equations: &[(Vec<(usize, u64)>, u64)], nvars: usize) -> Option<Vec<u64>> {
    let mut basis: Vec<(Vec<u64>, u64)> = Vec::new(); // pivot-normalized rows
    let mut pivot_of_col: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let lead = |row: &[u64]| row.iter().rposition(|&c| c != 0);
    for (coeffs, rhs) in equations {
        let mut row = vec![0u64; nvars];
        for &(v, c) in coeffs {
            row[v] = f.add(row[v], c);
        }
        let mut rhs = f.add(*rhs, 0);
        // Reduce the leading column against the basis until it is fresh or the row dies.
        while let Some(col) = lead(&row) {
            let Some(&bi) = pivot_of_col.get(&col) else { break };
            let factor = row[col];
            let (brow, brhs) = &basis[bi];
            for v in 0..=col {
                row[v] = f.sub(row[v], f.mul(factor, brow[v]));
            }
            rhs = f.sub(rhs, f.mul(factor, *brhs));
        }
        match lead(&row) {
            None => {
                if rhs != 0 {
                    return None; // 0 = nonzero — inconsistent
                }
            }
            Some(col) => {
                let factor = f.inv(row[col]);
                for v in 0..=col {
                    row[v] = f.mul(row[v], factor);
                }
                rhs = f.mul(rhs, factor);
                pivot_of_col.insert(col, basis.len());
                basis.push((row, rhs));
            }
        }
    }
    // Back-substitution in increasing pivot order: every non-pivot entry of a stored row sits strictly
    // below its own pivot, so each referenced unknown is already fixed (free variables stay 0).
    let mut x = vec![0u64; nvars];
    let mut cols: Vec<usize> = pivot_of_col.keys().copied().collect();
    cols.sort_unstable();
    for col in cols {
        let (row, rhs) = &basis[pivot_of_col[&col]];
        let mut val = *rhs;
        for v in 0..col {
            if row[v] != 0 {
                val = f.sub(val, f.mul(row[v], x[v]));
            }
        }
        x[col] = val;
    }
    Some(x)
}

/// Multilinear product by a single monomial: variable sets OR together, colliding images add.
fn poly_mul_mono_gfp(f: NsField, p: &GfpPoly, m: Mono) -> GfpPoly {
    let mut r = GfpPoly::new();
    for (&t, &c) in p {
        add_term(f, &mut r, t | m, c);
    }
    r
}

/// Does a **degree-`d` Nullstellensatz refutation over `f`** exist for an arbitrary polynomial generator
/// system — is `1` in the `f`-span of `{ m·g : deg(m·g) ≤ d }`? The general-field sibling of
/// [`crate::polycalc::ns_refutes_polys`], answered by incremental-echelon span membership (memory
/// `O(rank · basis)` however many generator products stream through). Degree-bounded enumeration —
/// `num_vars ≤ 63`.
pub fn ns_refutes_polys_gfp(f: NsField, num_vars: usize, gens: &[GfpPoly], degree: usize) -> bool {
    let basis = crate::polycalc::monomials_up_to_degree(num_vars, degree);
    let index: std::collections::HashMap<Mono, usize> =
        basis.iter().enumerate().map(|(i, &m)| (m, i)).collect();
    let nb = basis.len();
    let mut echelon: Vec<Vec<u64>> = Vec::new();
    let mut pivot_of_col: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let lead = |row: &[u64]| row.iter().rposition(|&c| c != 0);
    let mut insert = |mut row: Vec<u64>| {
        while let Some(col) = lead(&row) {
            let Some(&bi) = pivot_of_col.get(&col) else { break };
            let factor = row[col];
            for v in 0..=col {
                row[v] = f.sub(row[v], f.mul(factor, echelon[bi][v]));
            }
        }
        if let Some(col) = lead(&row) {
            let factor = f.inv(row[col]);
            for v in 0..=col {
                row[v] = f.mul(row[v], factor);
            }
            pivot_of_col.insert(col, echelon.len());
            echelon.push(row);
        }
    };
    for g in gens {
        if g.is_empty() {
            continue; // the zero polynomial generates nothing
        }
        for &m in &basis {
            let prod = poly_mul_mono_gfp(f, g, m);
            if !prod.is_empty() && gfp_poly_degree(&prod) <= degree {
                let mut row = vec![0u64; nb];
                for (t, c) in prod {
                    row[index[&t]] = c;
                }
                insert(row);
            }
        }
    }
    // Reduce the target — the constant polynomial 1 — and check it vanishes.
    let mut target = vec![0u64; nb];
    target[index[&0u64]] = 1;
    while let Some(col) = lead(&target) {
        let Some(&bi) = pivot_of_col.get(&col) else { break };
        let factor = target[col];
        for v in 0..=col {
            target[v] = f.sub(target[v], f.mul(factor, echelon[bi][v]));
        }
    }
    lead(&target).is_none()
}

/// [`ns_refutes_polys_gfp`] for a CNF: the generators are the signed clause false-indicators. An empty
/// clause is `1 = 0` outright.
pub fn ns_refutes_gfp(f: NsField, num_vars: usize, clauses: &[Vec<Lit>], degree: usize) -> bool {
    if clauses.iter().any(|c| c.is_empty()) {
        return true;
    }
    let gens: Vec<GfpPoly> = clauses.iter().map(|c| clause_polynomial_gfp(f, c)).collect();
    ns_refutes_polys_gfp(f, num_vars, &gens, degree)
}

/// A **constructive Nullstellensatz certificate over `f`**: one coefficient polynomial `g_C` per input
/// clause with `Σ_C p_C · g_C = 1` in the multilinear ring over `f` — the coefficient-explicit
/// generalization of [`crate::polycalc::NsCertificate`].
#[derive(Clone, Debug)]
pub struct NsCertificateGfp {
    field: NsField,
    num_vars: usize,
    /// `coeffs[i]` is `g_{C_i}` for the `i`-th input clause (parallel indexing to the clause list).
    coeffs: Vec<GfpPoly>,
}

impl NsCertificateGfp {
    pub fn field(&self) -> NsField {
        self.field
    }

    pub fn num_vars(&self) -> usize {
        self.num_vars
    }

    /// The maximum monomial degree among the coefficient polynomials.
    pub fn degree(&self) -> usize {
        self.coeffs.iter().map(gfp_poly_degree).max().unwrap_or(0)
    }

    /// **Re-check against the original clauses** (zero trust in the producer), twice over: recompute
    /// `Σ_C p_C · g_C` in the multilinear ring over `field` and confirm it is the constant `1`; then the
    /// engine-independent corner check — at every assignment `a` of the cube (up to 20 variables),
    /// `Σ_C p_C(a) · g_C(a) = 1` as field elements. Fails closed on a clause-count mismatch.
    pub fn verify(&self, clauses: &[Vec<Lit>]) -> bool {
        if self.coeffs.len() != clauses.len() {
            return false;
        }
        let f = self.field;
        let mut sum = GfpPoly::new();
        for (c, g) in clauses.iter().zip(&self.coeffs) {
            if g.is_empty() {
                continue;
            }
            for (m, co) in poly_mul(f, &clause_polynomial_gfp(f, c), g) {
                add_term(f, &mut sum, m, co);
            }
        }
        if !(sum.len() == 1 && sum.get(&0u64) == Some(&1)) {
            return false;
        }
        if self.num_vars <= 20 {
            let eval = |p: &GfpPoly, a: u64| -> u64 {
                p.iter().fold(0u64, |acc, (&m, &c)| if m & !a == 0 { f.add(acc, c) } else { acc })
            };
            for a in 0u64..(1u64 << self.num_vars) {
                let total = clauses.iter().zip(&self.coeffs).fold(0u64, |acc, (c, g)| {
                    f.add(acc, f.mul(eval(&clause_polynomial_gfp(f, c), a), eval(g, a)))
                });
                if total != 1 {
                    return false;
                }
            }
        }
        true
    }
}

/// **The uniform completeness construction over `f`** — the partition-of-unity charging of
/// [`crate::polycalc::build_ns_certificate`], field-generic because its every ingredient is: the signed
/// point indicators sum to `1` in any field ([`partition_of_unity_gfp`]), and `p_C · δ_a = δ_a` whenever
/// `p_C(a) = 1` (multilinear representations on the cube are unique over any field). Returns a
/// constructive [`NsCertificateGfp`] proving UNSAT or a satisfying assignment proving SAT. Bounded to
/// `num_vars ≤ 20` (the explicit-corner construction).
pub fn build_ns_certificate_gfp(
    f: NsField,
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Result<NsCertificateGfp, Vec<bool>> {
    assert!(num_vars <= 20, "the explicit-corner construction is bounded to num_vars ≤ 20");
    let mut coeffs: Vec<GfpPoly> = vec![GfpPoly::new(); clauses.len()];
    for a in 0u64..(1u64 << num_vars) {
        let sel = clauses
            .iter()
            .position(|c| !c.iter().any(|l| ((a >> l.var()) & 1 == 1) == l.is_positive()));
        match sel {
            None => return Err((0..num_vars).map(|i| (a >> i) & 1 == 1).collect()),
            Some(ci) => {
                for (m, c) in point_indicator_gfp(f, a, num_vars) {
                    add_term(f, &mut coeffs[ci], m, c);
                }
            }
        }
    }
    Ok(NsCertificateGfp { field: f, num_vars, coeffs })
}

/// A **degree-`d` pseudo-expectation over `f`** for an arbitrary generator system: a functional `L` on
/// the degree-`≤ d` monomials with `L(1) = 1` and `L(m·g) = 0` for every admitted generator product,
/// returned as its nonzero values `(monomial, value)`. `Some(L)` certifies `NS-degree > d` over `f`
/// (re-checkable by [`check_ns_lower_bound_polys_gfp`], zero trust in the solver); `None` means a
/// degree-`d` refutation exists — the exact witness↔refutation duality of the `GF(2)` engine, at every
/// characteristic. Degree-bounded enumeration — `num_vars ≤ 63`.
pub fn ns_lower_bound_witness_polys_gfp(
    f: NsField,
    num_vars: usize,
    gens: &[GfpPoly],
    degree: usize,
) -> Option<Vec<(Mono, u64)>> {
    let basis = crate::polycalc::monomials_up_to_degree(num_vars, degree);
    let index: std::collections::HashMap<Mono, usize> =
        basis.iter().enumerate().map(|(i, &m)| (m, i)).collect();
    let mut eqs: Vec<(Vec<(usize, u64)>, u64)> = Vec::new();
    for g in gens {
        if g.is_empty() {
            continue;
        }
        for &m in &basis {
            let prod = poly_mul_mono_gfp(f, g, m);
            if !prod.is_empty() && gfp_poly_degree(&prod) <= degree {
                eqs.push((prod.iter().map(|(t, &c)| (index[t], c)).collect(), 0)); // ⟨L, m·g⟩ = 0
            }
        }
    }
    eqs.push((vec![(index[&0u64], 1)], 1)); // L(1) = 1
    let l = gfp_solve(f, &eqs, basis.len())?;
    Some(basis.iter().enumerate().filter(|&(i, _)| l[i] != 0).map(|(i, &m)| (m, l[i])).collect())
}

/// [`ns_lower_bound_witness_polys_gfp`] for a CNF (signed clause false-indicators as generators). An
/// empty clause is an immediate refutation — no lower bound at any degree.
pub fn ns_lower_bound_witness_gfp(
    f: NsField,
    num_vars: usize,
    clauses: &[Vec<Lit>],
    degree: usize,
) -> Option<Vec<(Mono, u64)>> {
    if clauses.iter().any(|c| c.is_empty()) {
        return None;
    }
    let gens: Vec<GfpPoly> = clauses.iter().map(|c| clause_polynomial_gfp(f, c)).collect();
    ns_lower_bound_witness_polys_gfp(f, num_vars, &gens, degree)
}

/// [`ns_lower_bound_witness_polys_gfp`] restricted to a **sub-basis**: the functional `L` is sought only
/// on monomials passing `in_basis` (`L = 0` elsewhere), while the constraints `⟨L, m·g⟩ = 0` still range
/// over *all* admitted generator products — so any `Some` is a fully valid,
/// [`check_ns_lower_bound_polys_gfp`]-verifiable witness, and `None` means only "no witness on this
/// sub-basis". The structure probe of the `GF(2)` engine, carried to every characteristic: which
/// candidate supports hold a family's lower bound, field by field. Degree-bounded enumeration —
/// `num_vars ≤ 63`.
pub fn ns_lower_bound_witness_on_basis_gfp(
    f: NsField,
    num_vars: usize,
    gens: &[GfpPoly],
    degree: usize,
    in_basis: &dyn Fn(Mono) -> bool,
) -> Option<Vec<(Mono, u64)>> {
    let all = crate::polycalc::monomials_up_to_degree(num_vars, degree);
    let basis: Vec<Mono> = all.iter().copied().filter(|&m| in_basis(m)).collect();
    let index: std::collections::HashMap<Mono, usize> =
        basis.iter().enumerate().map(|(i, &m)| (m, i)).collect();
    index.get(&0u64)?; // the empty monomial must be in the sub-basis for L(1) = 1
    let mut eqs: Vec<(Vec<(usize, u64)>, u64)> = Vec::new();
    for g in gens {
        if g.is_empty() {
            continue;
        }
        for &m in &all {
            let prod = poly_mul_mono_gfp(f, g, m);
            if !prod.is_empty() && gfp_poly_degree(&prod) <= degree {
                let coeffs: Vec<(usize, u64)> = prod
                    .iter()
                    .filter_map(|(t, &c)| index.get(t).map(|&i| (i, c)))
                    .collect();
                eqs.push((coeffs, 0));
            }
        }
    }
    eqs.push((vec![(index[&0u64], 1)], 1));
    let l = gfp_solve(f, &eqs, basis.len())?;
    Some(basis.iter().enumerate().filter(|&(i, _)| l[i] != 0).map(|(i, &m)| (m, l[i])).collect())
}

/// Re-check a [`ns_lower_bound_witness_polys_gfp`] certificate (zero trust in the producer): `L(1) = 1`
/// and `⟨L, m·g⟩ = 0` over `f` for every admitted generator product. `true` ⟹ the system genuinely has
/// no degree-`d` Nullstellensatz refutation over `f`. Degree-bounded enumeration — `num_vars ≤ 63`.
pub fn check_ns_lower_bound_polys_gfp(
    f: NsField,
    num_vars: usize,
    gens: &[GfpPoly],
    degree: usize,
    witness: &[(Mono, u64)],
) -> bool {
    let mut l: BTreeMap<Mono, u64> = BTreeMap::new();
    for &(m, v) in witness {
        add_term(f, &mut l, m, v);
    }
    if l.get(&0u64) != Some(&1) {
        return false; // L(1) must be 1
    }
    let value = |m: &Mono| l.get(m).copied().unwrap_or(0);
    for g in gens {
        if g.is_empty() {
            continue;
        }
        for &m in &crate::polycalc::monomials_up_to_degree(num_vars, degree) {
            let prod = poly_mul_mono_gfp(f, g, m);
            if !prod.is_empty() && gfp_poly_degree(&prod) <= degree {
                let pairing =
                    prod.iter().fold(0u64, |acc, (t, &c)| f.add(acc, f.mul(c, value(t))));
                if pairing != 0 {
                    return false; // ⟨L, m·g⟩ must be 0
                }
            }
        }
    }
    true
}

/// The **linear encoding of exactly-one constraints over `f`**: for each group `G` the degree-1
/// generator `(Σ_{v∈G} x_v) − 1` plus the pairwise products `x_u·x_v`, deduplicated — the signed
/// generalization of [`crate::polycalc::exactly_one_linear_generators`] (over `GF(2)` the two coincide,
/// since `−1 = 1`). This is the encoding the modular-counting degree bounds are stated against; over
/// `GF(p)` the point generators telescope — `Σ_i P_i = −n` when every edge meets exactly `p` points — so
/// a counting family with `p ∤ n` collapses at degree 1 over its **own** characteristic.
pub fn exactly_one_linear_generators_gfp(f: NsField, groups: &[Vec<u32>]) -> Vec<GfpPoly> {
    let mut gens: Vec<GfpPoly> = Vec::new();
    for g in groups {
        let mut lin: GfpPoly = [(0u64, f.neg(1))].into_iter().collect();
        for &v in g {
            assert!(v < 63, "the u64 monomial mask carries ≤ 63 variables");
            add_term(f, &mut lin, 1u64 << v, 1);
        }
        gens.push(lin);
    }
    let mut pairs: std::collections::BTreeSet<Mono> = std::collections::BTreeSet::new();
    for g in groups {
        for (i, &u) in g.iter().enumerate() {
            for &v in &g[i + 1..] {
                pairs.insert((1u64 << u) | (1u64 << v));
            }
        }
    }
    gens.extend(pairs.into_iter().map(|m| [(m, 1u64)].into_iter().collect::<GfpPoly>()));
    gens
}

/// A **degree-`d` certificate EXTRACTION over `f`**: where [`ns_refutes_polys_gfp`] only decides span
/// membership, this runs the same incremental echelon with **provenance** — each basis row remembers the
/// combination of original generator products it is — and reads the certificate `g_C = Σ λ_{C,m}·m` off
/// the target's reduction, exactly the technique of [`crate::modp::solve`]'s refutation combinations.
/// Returns `None` when no degree-`d` refutation exists. The extracted certificate is what the
/// `GF(4) → GF(2)` projection ([`project_gf4_certificate_to_gf2`]) operates on. Provenance rows cost
/// `O(rank · products)` memory — meant for the small-census scale, not the witness frontier.
pub fn ns_certificate_at_degree_gfp(
    f: NsField,
    num_vars: usize,
    clauses: &[Vec<Lit>],
    degree: usize,
) -> Option<NsCertificateGfp> {
    if clauses.iter().any(|c| c.is_empty()) {
        return None; // 1 = 0 needs no polynomial certificate; this extractor works over generators
    }
    let gens: Vec<GfpPoly> = clauses.iter().map(|c| clause_polynomial_gfp(f, c)).collect();
    let basis = crate::polycalc::monomials_up_to_degree(num_vars, degree);
    let index: std::collections::HashMap<Mono, usize> =
        basis.iter().enumerate().map(|(i, &m)| (m, i)).collect();
    let nb = basis.len();
    type Prov = BTreeMap<(usize, Mono), u64>; // (clause, multiplier) → λ
    let mut echelon: Vec<(Vec<u64>, Prov)> = Vec::new();
    let mut pivot_of_col: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    let lead = |row: &[u64]| row.iter().rposition(|&c| c != 0);
    let prov_axpy = |f: NsField, dst: &mut Prov, factor: u64, src: &Prov, negate: bool| {
        for (&k, &v) in src {
            let delta = if negate { f.neg(f.mul(factor, v)) } else { f.mul(factor, v) };
            let e = dst.entry(k).or_insert(0);
            *e = f.add(*e, delta);
            if *e == 0 {
                dst.remove(&k);
            }
        }
    };
    for (ci, g) in gens.iter().enumerate() {
        if g.is_empty() {
            continue;
        }
        for &m in &basis {
            let prod = poly_mul_mono_gfp(f, g, m);
            if prod.is_empty() || gfp_poly_degree(&prod) > degree {
                continue;
            }
            let mut row = vec![0u64; nb];
            for (t, c) in prod {
                row[index[&t]] = c;
            }
            let mut prov: Prov = [((ci, m), 1u64)].into_iter().collect();
            while let Some(col) = lead(&row) {
                let Some(&bi) = pivot_of_col.get(&col) else { break };
                let factor = row[col];
                let (brow, bprov) = &echelon[bi];
                for v in 0..=col {
                    row[v] = f.sub(row[v], f.mul(factor, brow[v]));
                }
                let bprov = bprov.clone();
                prov_axpy(f, &mut prov, factor, &bprov, true);
            }
            if let Some(col) = lead(&row) {
                let factor = f.inv(row[col]);
                for v in 0..=col {
                    row[v] = f.mul(row[v], factor);
                }
                let scaled: Prov = prov.iter().map(|(&k, &v)| (k, f.mul(v, factor))).collect();
                pivot_of_col.insert(col, echelon.len());
                echelon.push((row, scaled));
            }
        }
    }
    // Reduce the target `1`, accumulating the combination that produced it.
    let mut target = vec![0u64; nb];
    target[index[&0u64]] = 1;
    let mut comb: Prov = Prov::new();
    while let Some(col) = lead(&target) {
        let Some(&bi) = pivot_of_col.get(&col) else { break };
        let factor = target[col];
        let (brow, bprov) = &echelon[bi];
        for v in 0..=col {
            target[v] = f.sub(target[v], f.mul(factor, brow[v]));
        }
        let bprov = bprov.clone();
        prov_axpy(f, &mut comb, factor, &bprov, false);
    }
    if lead(&target).is_some() {
        return None; // 1 is not in the degree-d span
    }
    let mut coeffs: Vec<GfpPoly> = vec![GfpPoly::new(); clauses.len()];
    for ((ci, m), lambda) in comb {
        add_term(f, &mut coeffs[ci], m, lambda);
    }
    Some(NsCertificateGfp { field: f, num_vars, coeffs })
}

/// The **`GF(4) → GF(2)` coefficient projection** — the constructive half of the extension-field
/// collapse. `λ : a + b·ω ↦ a` is the `GF(2)`-linear functional fixing `1`; because every clause
/// polynomial has prime-subfield (0/1) coefficients, applying `λ` coefficient-wise to a `GF(4)`
/// certificate identity `Σ_C p_C · g_C = 1` commutes with the clause factors and lands on a `GF(2)`
/// certificate of no larger degree. So nothing proved over `GF(4)` needed the extension: **NS degree
/// depends only on the characteristic**. Returns `None` unless the certificate is over `Gf4`.
pub fn project_gf4_certificate_to_gf2(cert: &NsCertificateGfp) -> Option<NsCertificateGfp> {
    if cert.field != NsField::Gf4 {
        return None;
    }
    let coeffs: Vec<GfpPoly> = cert
        .coeffs
        .iter()
        .map(|g| g.iter().filter(|&(_, &c)| c & 1 == 1).map(|(&m, _)| (m, 1u64)).collect())
        .collect();
    Some(NsCertificateGfp { field: NsField::Prime(2), num_vars: cert.num_vars, coeffs })
}

/// The **unnormalized group-sum symmetrization** `Σ_{g∈G} g·L` of a functional over `f` — coefficients
/// genuinely ADD, where the `GF(2)` engine could only toggle. On the constant monomial it evaluates to
/// `|G| · L(1)`, which is the whole dichotomy: zero (annihilation) iff `char | |G|`. Pass the full
/// closed group, not a generating set.
pub fn symmetrize_gfp(
    f: NsField,
    l: &[(Mono, u64)],
    group: &[crate::proof::Perm],
) -> Vec<(Mono, u64)> {
    let mut sym: BTreeMap<Mono, u64> = BTreeMap::new();
    for &(m, v) in l {
        for g in group {
            add_term(f, &mut sym, crate::polycalc::apply_perm_to_mono(g, m), v);
        }
    }
    sym.into_iter().collect()
}

/// The **Reynolds operator** `L ↦ |G|⁻¹ · Σ_{g∈G} g·L` over `f` — the characteristic-0 averaging trick,
/// available exactly when `|G|` is invertible, i.e. `gcd(|G|, char) = 1`. Returns `None` when
/// `char | |G|` (the annihilation branch: the group-sum kills `L(1)` and no normalization can restore
/// it), and the averaged functional otherwise. Averaging a valid pseudo-expectation of a `G`-invariant
/// generator system yields a valid, `G`-invariant one — the branch the `GF(2)` engine can never take on
/// an even group. Pass the full closed group.
pub fn reynolds_gfp(
    f: NsField,
    l: &[(Mono, u64)],
    group: &[crate::proof::Perm],
) -> Option<Vec<(Mono, u64)>> {
    let order = f.embed_int(group.len() as u128);
    if order == 0 {
        return None; // char | |G| — the annihilation branch
    }
    let scale = f.inv(order);
    Some(symmetrize_gfp(f, l, group).into_iter().map(|(m, v)| (m, f.mul(v, scale))).collect())
}

/// [`check_ns_lower_bound_polys_gfp`] for a CNF. An empty clause admits no lower bound at any degree.
pub fn check_ns_lower_bound_gfp(
    f: NsField,
    num_vars: usize,
    clauses: &[Vec<Lit>],
    degree: usize,
    witness: &[(Mono, u64)],
) -> bool {
    if clauses.iter().any(|c| c.is_empty()) {
        return false;
    }
    let gens: Vec<GfpPoly> = clauses.iter().map(|c| clause_polynomial_gfp(f, c)).collect();
    check_ns_lower_bound_polys_gfp(f, num_vars, &gens, degree, witness)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/polycalc_gfp/tests.rs"]
mod tests;
