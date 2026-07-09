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
mod tests {
    use super::*;

    /// A deterministic LCG for reproducible fuzz corpora inside tests (no `rand`, no wall clock).
    fn lcg(state: &mut u64) -> u64 {
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        *state >> 33
    }

    /// Evaluate a polynomial at a cube corner: monomial `m` contributes its coefficient iff `m ⊆ a`.
    fn eval_at(f: NsField, p: &GfpPoly, a: u64) -> u64 {
        p.iter().fold(0u64, |acc, (&m, &c)| if m & !a == 0 { f.add(acc, c) } else { acc })
    }

    fn falsifies(clause: &[Lit], a: u64) -> bool {
        !clause.iter().any(|l| ((a >> l.var()) & 1 == 1) == l.is_positive())
    }

    /// **The clause polynomial over `GF(p)` is the SIGNED false-indicator, pinned on every corner.** Over
    /// `GF(2)`, `1 − x = 1 + x`, so the sign of the positive-literal factor is invisible; over `GF(3)` it
    /// is not — the factor must be `1 + 2x`, and using `1 + x` silently breaks the indicator semantics.
    /// We pin the sign convention explicitly, then verify the corner semantics (1 exactly on falsifying
    /// corners, 0 elsewhere) on a deterministic random-clause corpus across `p ∈ {2, 3, 5, 7}`.
    #[test]
    fn gf3_clause_polynomial_is_the_signed_false_indicator_on_every_corner() {
        let f3 = NsField::Prime(3);
        // The sign pin: the false-indicator of the positive literal x0 is 1 − x = 1 + 2x over GF(3).
        let pos = clause_polynomial_gfp(f3, &[Lit::pos(0)]);
        let expected: GfpPoly = [(0u64, 1u64), (1u64, 2u64)].into_iter().collect();
        assert_eq!(pos, expected, "positive literal → 1 − x = 1 + 2x over GF(3), not 1 + x");
        let neg = clause_polynomial_gfp(f3, &[Lit::neg(0)]);
        let expected_neg: GfpPoly = [(1u64, 1u64)].into_iter().collect();
        assert_eq!(neg, expected_neg, "negative literal → x");

        let mut seed = 0x00C0_FFEEu64;
        for &p in &[2u64, 3, 5, 7] {
            let f = NsField::Prime(p);
            for _ in 0..40 {
                let n = 2 + (lcg(&mut seed) % 5) as usize; // 2..=6 variables
                let width = 1 + (lcg(&mut seed) % n as u64) as usize;
                let mut vars: Vec<u32> = Vec::new();
                while vars.len() < width {
                    let v = (lcg(&mut seed) % n as u64) as u32;
                    if !vars.contains(&v) {
                        vars.push(v);
                    }
                }
                let clause: Vec<Lit> =
                    vars.iter().map(|&v| Lit::new(v, lcg(&mut seed) & 1 == 1)).collect();
                let poly = clause_polynomial_gfp(f, &clause);
                assert_eq!(gfp_poly_degree(&poly), width, "p={p}: clause polynomial degree = width");
                for a in 0u64..(1u64 << n) {
                    let want = if falsifies(&clause, a) { 1 } else { 0 };
                    assert_eq!(
                        eval_at(f, &poly, a),
                        want,
                        "p={p} clause={clause:?} corner={a:0width$b}",
                        width = n
                    );
                }
            }
        }
    }

    /// **The partition-of-unity atom `(1 − x) + x = 1` holds in EVERY prime field** — the identity behind
    /// constructive NS completeness is characteristic-free, so "no finite randomness" (§3 of the paper) is
    /// not a `GF(2)` artifact. The atom is the constant `1` at `p ∈ {2, 3, 5, 7}`, and the `2ⁿ`-corner sum
    /// `Σ_a δ_a` — now with genuinely signed indicator terms that must cancel — collapses to `1` at every
    /// `n ≤ 10` over every one of those fields.
    #[test]
    fn the_partition_of_unity_atom_is_one_over_every_prime_field() {
        let one: GfpPoly = [(0u64, 1u64)].into_iter().collect();
        for &p in &[2u64, 3, 5, 7] {
            let f = NsField::Prime(p);
            for v in 0..8 {
                assert_eq!(pou_atom_gfp(f, v), one, "p={p}: the atom (1−x{v})+x{v} reduces to 1");
            }
            for n in 0..=10 {
                assert_eq!(partition_of_unity_gfp(f, n), one, "p={p}: Σ_a δ_a = 1 over the {n}-cube");
            }
        }
    }

    /// **The incomparability row, side 1: `Count_3` collapses at degree 1 over its OWN characteristic
    /// and carries certified growing degree over the other.** The linear-encoded point generators
    /// telescope over `GF(3)`: every 3-block meets exactly 3 points, so `Σ_i P_i = −n` — a nonzero
    /// CONSTANT whenever `3 ∤ n`, hence a degree-**1** refutation at every scale (`n = 4, 5, 7`, the last
    /// at 35 variables). The identity is asserted as polynomial algebra, not just its consequence. Over
    /// `GF(2)` the same family has certified exact degree 2 (`n = 4, 5`, both halves) growing to `≥ 3`
    /// (`n = 7`, re-checked dual witness) — the facts of the paper's mismatch row, re-invoked beside
    /// their `GF(3)` foil. One family, two characteristics, opposite complexity.
    #[test]
    fn count_three_falls_to_degree_one_over_gf3_but_has_certified_growing_degree_over_gf2() {
        let f3 = NsField::Prime(3);
        for n in [4usize, 5, 7] {
            let (cnf, _) = crate::families::mod_counting(n, 3);
            let nv = cnf.num_vars;
            let groups = crate::families::mod_counting_groups(n, 3);
            let gens3 = exactly_one_linear_generators_gfp(f3, &groups);
            // The telescoping identity: the n point generators sum to the constant −n over GF(3).
            let mut total = GfpPoly::new();
            for g in gens3.iter().take(n) {
                for (&m, &c) in g {
                    add_term(f3, &mut total, m, c);
                }
            }
            let minus_n = f3.neg(f3.embed_int(n as u128));
            assert_ne!(minus_n, 0, "3 ∤ {n}, so the telescoped constant is nonzero");
            let expected: GfpPoly = [(0u64, minus_n)].into_iter().collect();
            assert_eq!(total, expected, "Count_3({n}): Σ_i P_i = −n over GF(3)");
            assert!(
                ns_refutes_polys_gfp(f3, nv, &gens3, 1),
                "Count_3({n}): a degree-1 GF(3) refutation — the char-matched collapse"
            );
            assert!(
                ns_lower_bound_witness_polys_gfp(f3, nv, &gens3, 1).is_none(),
                "Count_3({n}): duality — no GF(3) pseudo-expectation survives at degree 1"
            );
        }
        // The GF(2) half: exact degree 2 at n = 4, 5; ≥ 3 at n = 7 — strictly above 1, and growing.
        for n in [4usize, 5] {
            let (cnf, _) = crate::families::mod_counting(n, 3);
            let nv = cnf.num_vars;
            let gens2 = crate::polycalc::exactly_one_linear_generators(
                &crate::families::mod_counting_groups(n, 3),
            );
            let w1 = crate::polycalc::ns_lower_bound_witness_polys(nv, &gens2, 1)
                .expect("Count_3: no degree-1 GF(2) refutation");
            assert!(crate::polycalc::check_ns_lower_bound_polys(nv, &gens2, 1, &w1));
            assert!(crate::polycalc::ns_refutes_polys(nv, &gens2, 2), "Count_3({n}): exact GF(2) degree 2");
        }
        let n = 7usize;
        let (cnf, _) = crate::families::mod_counting(n, 3);
        let gens2 = crate::polycalc::exactly_one_linear_generators(
            &crate::families::mod_counting_groups(n, 3),
        );
        let w2 = crate::polycalc::ns_lower_bound_witness_polys(cnf.num_vars, &gens2, 2)
            .expect("Count_3(7): the GF(2) degree exceeds 2");
        assert!(crate::polycalc::check_ns_lower_bound_polys(cnf.num_vars, &gens2, 2, &w2));
        eprintln!("Count_3: GF(3) degree = 1 at n = 4, 5, 7; GF(2) degree = 2, 2, ≥3 — incomparability side 1");
    }

    /// **The incomparability row, side 2 — the exact mirror: `Count_2` collapses at degree 1 over
    /// `GF(2)` and carries certified growing degree over `GF(3)`.** The paper's char-matched collapse
    /// (`Count_2`, odd `n`, degree-1 `GF(2)` refutation) is re-invoked beside the new fact: over `GF(3)`
    /// the same family has exact NS degree **2 at `n = 3` and 3 at `n = 5`** — both halves certified (a
    /// re-checked dual witness below, a refutation at the degree) — strictly growing, strictly above the
    /// `GF(2)` degree of 1. Even `n` is satisfiable and refuted at no probed degree over either field
    /// (the soundness foil). Together with side 1 this machine-certifies that `GF(2)`- and `GF(3)`-NS
    /// are **incomparable proof systems**: each crushes at degree 1 a family the other can only refute
    /// with growing degree.
    #[test]
    fn count_two_falls_to_degree_one_over_gf2_but_has_certified_growing_degree_over_gf3() {
        let f3 = NsField::Prime(3);
        let mut gf3_degrees = Vec::new();
        for (n, exact) in [(3usize, 2usize), (5, 3)] {
            let (cnf, _) = crate::families::mod_counting(n, 2);
            let nv = cnf.num_vars;
            let groups = crate::families::mod_counting_groups(n, 2);
            let gens2 = crate::polycalc::exactly_one_linear_generators(&groups);
            assert!(
                crate::polycalc::ns_refutes_polys(nv, &gens2, 1),
                "Count_2({n}), n odd: GF(2) degree 1 — the char-matched collapse"
            );
            let gens3 = exactly_one_linear_generators_gfp(f3, &groups);
            for d in 1..exact {
                let w = ns_lower_bound_witness_polys_gfp(f3, nv, &gens3, d)
                    .expect("a dual witness exists below the exact degree");
                assert!(
                    check_ns_lower_bound_polys_gfp(f3, nv, &gens3, d, &w),
                    "Count_2({n}): GF(3) NS-degree > {d} re-checks with zero trust"
                );
            }
            assert!(
                ns_refutes_polys_gfp(f3, nv, &gens3, exact),
                "Count_2({n}): GF(3) refuted at degree {exact} — exact"
            );
            gf3_degrees.push(exact);
            eprintln!("Count_2({n}) [{nv} vars]: GF(2) degree 1, certified exact GF(3) degree {exact}");
        }
        assert!(
            gf3_degrees.windows(2).all(|w| w[1] > w[0]),
            "the GF(3) degree grows with n: {gf3_degrees:?}"
        );
        // Soundness foil: even n is SAT (a perfect matching exists) — no refutation over either field.
        let (cnf, _) = crate::families::mod_counting(4, 2);
        let groups = crate::families::mod_counting_groups(4, 2);
        let gens3 = exactly_one_linear_generators_gfp(f3, &groups);
        let gens2 = crate::polycalc::exactly_one_linear_generators(&groups);
        for d in 1..=3 {
            assert!(!ns_refutes_polys_gfp(f3, cnf.num_vars, &gens3, d), "Count_2(4) is SAT (GF(3), d={d})");
            assert!(!crate::polycalc::ns_refutes_polys(cnf.num_vars, &gens2, d), "Count_2(4) is SAT (GF(2), d={d})");
        }
    }

    /// **The exact `GF(3)` degree of `Count_2` at scale: a wide-tread staircase, mirroring the
    /// `GF(2)` side.** `Count_2(7)` (21 variables, basis `C(21,≤3) = 1562`) has exact linear-encoded
    /// `GF(3)` NS degree **3** — certified both halves: a re-checked degree-2 dual witness and a
    /// refutation at 3. So the mismatch-side staircase reads `2 (n=3), 3 (n=5), 3 (n=7)`: the degree
    /// grows out of the dense regime and then holds its tread — precisely the profile of the paper's
    /// `GF(2)`-side `Count_3` measurements (`2, 2, 3, 3` across `n = 4, 5, 7, 8`). The two mismatch
    /// rows are not just qualitatively symmetric; they climb the same wide-tread staircase from
    /// opposite characteristics. (`n = 9` — 36 variables, a `C(36,≤4)` basis — lies past the dense
    /// mod-`p` echelon's frontier.) Dense `GF(3)` Gaussian elimination at this width is release-scale
    /// work, so it lives behind `#[ignore]` like its `GF(2)` siblings.
    #[test]
    #[ignore = "scale measurement — dense GF(3) Gaussian elimination at 1562 columns; run explicitly or via the fast suite"]
    fn count_two_scale_probe_measures_the_gf3_degree_at_scale() {
        let f3 = NsField::Prime(3);
        let (cnf, _) = crate::families::mod_counting(7, 2);
        let nv = cnf.num_vars;
        let gens3 = exactly_one_linear_generators_gfp(f3, &crate::families::mod_counting_groups(7, 2));
        let w = ns_lower_bound_witness_polys_gfp(f3, nv, &gens3, 2)
            .expect("Count_2(7): a degree-2 GF(3) pseudo-expectation exists");
        assert!(
            check_ns_lower_bound_polys_gfp(f3, nv, &gens3, 2, &w),
            "Count_2(7): GF(3) NS-degree ≥ 3 re-checks with zero trust"
        );
        assert!(
            ns_refutes_polys_gfp(f3, nv, &gens3, 3),
            "Count_2(7): a degree-3 GF(3) refutation exists — the exact degree is 3"
        );
        eprintln!("Count_2(7) [{nv} vars]: exact GF(3) NS degree = 3 — the staircase 2, 3, 3");
    }

    /// **Mod-3 Tseitin: degree-1 `GF(3)` reasoning crushes what the whole certified `GF(2)` ladder
    /// cannot even place.** On the 3-regular-expander divergence instance with total charge `2` — `≡ 0
    /// (mod 2)`, so the parity cut is *structurally* blind — the native mod-3 system AND the system
    /// recovered from its opaque one-hot CNF both refute instantly, each with a re-checkable
    /// linear-dependency combination. The certified proof-complexity ladder, probed through NS degree 3
    /// on the 18-variable CNF, reports `BeyondBudget`: not trivial, not counting, not parity, no
    /// low-degree GF(2) certificate. This is the `router_beats_ladder` audit gap measured on a concrete
    /// instance — the datum the census's characteristic rung exists to close.
    #[test]
    fn mod3_tseitin_is_gf3_easy_and_its_gf2_route_is_the_audit_gap() {
        let (eqs, cnf, verdict) = crate::families::mod_p_tseitin_expander(4, 3, 0xC0DE);
        assert_eq!(verdict, crate::families::ExpectedVerdict::Unsat);
        let ne = cnf.num_vars / 3; // one-hot: 3 boolean bits per GF(3) edge variable
        match crate::modp::solve(&eqs, ne, 3) {
            crate::modp::ModpOutcome::Unsat(combo) => {
                assert!(
                    crate::modp::is_refutation(&eqs, ne, 3, &combo),
                    "the native GF(3) refutation re-checks"
                );
            }
            crate::modp::ModpOutcome::Sat(_) => panic!("the charged divergence system is inconsistent"),
        }
        // The opaque one-hot CNF lifts back to the same mod-3 system (recognition, never guessing).
        let rec = crate::modp::recover_from_cnf(cnf.num_vars, &cnf.clauses)
            .expect("the one-hot encoding is recognized");
        assert_eq!(rec.modulus, 3, "the recovered modulus is the group size");
        assert_eq!(rec.num_vars, ne, "one recovered GF(3) variable per one-hot group");
        match crate::modp::solve(&rec.equations, rec.num_vars, 3) {
            crate::modp::ModpOutcome::Unsat(combo) => {
                assert!(
                    crate::modp::is_refutation(&rec.equations, rec.num_vars, 3, &combo),
                    "the recovered-system refutation re-checks"
                );
            }
            crate::modp::ModpOutcome::Sat(_) => panic!("the recovered system is inconsistent"),
        }
        // The audit gap, measured: the certified ladder has no rung that reaches this instance.
        let rung = crate::hypercube::weakest_crushing_rung(cnf.num_vars, &cnf.clauses, 3);
        assert_eq!(
            rung,
            crate::hypercube::ProofRung::BeyondBudget,
            "the GF(2) ladder cannot place what degree-1 GF(3) crushes"
        );
    }

    /// The all-ones hole-injective indicator of the paper's §5.3, as a general-field witness.
    fn hole_injective_indicator(num_vars: usize, holes: usize, degree: usize) -> Vec<(Mono, u64)> {
        (0u64..(1u64 << num_vars))
            .filter(|&mo| {
                mo.count_ones() as usize <= degree
                    && crate::polycalc::php_is_hole_injective(mo, holes)
            })
            .map(|mo| (mo, 1))
            .collect()
    }

    /// **The hole-injective indicator is a pseudo-expectation at EVERY characteristic — the paper's
    /// parity argument is the char-2 shadow of a binomial identity.** With the *signed* clause
    /// false-indicators, a pigeon clause pairs against the indicator as `Σ_{S_A⊆A, S_U⊆U}
    /// (−1)^{|S_A|+|S_U|} = (1−1)^{|A|}·(1−1)^{|U|}` (`A` = holes where the monomial already carries this
    /// pigeon's edge, `U` = untouched holes), which telescopes to `0` in **every** field whenever
    /// `|A| + |U| ≥ 1` — and the degree cap `2m−3` forces exactly that. Over `GF(2)` the signs are
    /// invisible (`−1 = 1`), so the identity *degenerates into* the paper's `Σ 1 = 2^{|U|} ≡ 0 (mod 2)`.
    /// The at-most-one generators vanish on hole collisions at every characteristic alike. Machine-checked
    /// at `m = 3, 4` over `GF(2)` (the paper's checker), `GF(3)`, and `GF(5)` — one closed-form witness,
    /// every prime field, proving `NS-degree(PHP_m) ≥ 2(m−1)` characteristic-free.
    #[test]
    fn the_hole_injective_indicator_is_a_pseudo_expectation_at_every_characteristic() {
        for m in [3usize, 4] {
            let (php, _) = crate::families::php(m);
            let holes = m - 1;
            let d = 2 * holes - 1;
            let w = hole_injective_indicator(php.num_vars, holes, d);
            // GF(2): the paper's fact, re-invoked through the paper's own checker.
            let monos: Vec<u64> = w.iter().map(|&(mo, _)| mo).collect();
            assert!(
                crate::polycalc::check_ns_lower_bound(php.num_vars, &php.clauses, d, &monos),
                "PHP({m}): the indicator is valid over GF(2) — the parity shadow"
            );
            // GF(3), GF(5): the same all-ones indicator, now via the binomial telescoping.
            for &p in &[3u64, 5] {
                assert!(
                    check_ns_lower_bound_gfp(NsField::Prime(p), php.num_vars, &php.clauses, d, &w),
                    "PHP({m}): the indicator is a valid degree-{d} pseudo-expectation over GF({p}) \
                     ⟹ NS-degree ≥ {} at characteristic {p}",
                    2 * holes
                );
            }
        }
    }

    /// **Pigeonhole hardness is characteristic-INVARIANT — the foil to `Count_p`'s characteristic
    /// sensitivity.** `Count_p` flips from degree 1 to growing degree as the field changes; PHP does not
    /// budge: its `GF(3)` NS degree at `m = 3` is **exactly 4 = 2(m−1)**, the same as `GF(2)` — measured
    /// by scan (no refutation through degree 3, refutation at 4), with the lower half certified by a
    /// solver-found dual witness that re-checks, and the duality (`witness at d ⟺ no refutation at d`)
    /// pinned on both sides of the threshold. At `m = 4` the uniform indicator certifies `GF(3)`
    /// NS-degree ≥ 6 = 2(m−1), matching the certified `GF(2)` exact degree. Counting is orthogonal to
    /// linear algebra over **every** field — which is exactly why the pigeonhole group (`Sₘ × Sₘ₋₁`,
    /// permutation symmetry) rather than any field structure is what protects its hardness.
    #[test]
    fn php_gf3_ns_degree_is_measured_and_its_lower_half_certified() {
        let f3 = NsField::Prime(3);
        // m = 3: the exact degree, both halves, plus the duality at the threshold.
        let (php3, _) = crate::families::php(3);
        let nv = php3.num_vars;
        for d in 1..=3 {
            assert!(!ns_refutes_gfp(f3, nv, &php3.clauses, d), "PHP(3): no GF(3) refutation at {d}");
        }
        assert!(ns_refutes_gfp(f3, nv, &php3.clauses, 4), "PHP(3): GF(3) refuted at 4 — exact");
        let w3 = ns_lower_bound_witness_gfp(f3, nv, &php3.clauses, 3)
            .expect("PHP(3): a solver-found GF(3) witness exists at degree 3");
        assert!(
            check_ns_lower_bound_gfp(f3, nv, &php3.clauses, 3, &w3),
            "PHP(3): GF(3) NS-degree > 3 re-checks with zero trust"
        );
        assert!(
            ns_lower_bound_witness_gfp(f3, nv, &php3.clauses, 4).is_none(),
            "PHP(3): duality — no witness survives at the refutation degree"
        );
        // The GF(2) exact degree is 4 (the paper's certified fact) — the characteristics agree.
        assert!(crate::polycalc::nullstellensatz_refutes(nv, &php3.clauses, 4));
        assert!(!crate::polycalc::nullstellensatz_refutes(nv, &php3.clauses, 3));
        // m = 4: the uniform indicator certifies GF(3) NS-degree ≥ 6 = 2(m−1), matching GF(2).
        let (php4, _) = crate::families::php(4);
        let w4 = hole_injective_indicator(php4.num_vars, 3, 5);
        assert!(
            check_ns_lower_bound_gfp(f3, php4.num_vars, &php4.clauses, 5, &w4),
            "PHP(4): GF(3) NS-degree ≥ 6 via the uniform indicator"
        );
        eprintln!("PHP: GF(3) degree = 4 (m=3, exact), ≥ 6 (m=4) — equal to GF(2); characteristic-invariant");
    }

    /// **The witness SUPPORT is where the characteristic bites — and it bites on a THRESHOLD, not just
    /// at 2.** The paper proved the classical (Razborov, char-0) partial-matching support cannot carry
    /// the `GF(2)` witness (a parity obstruction; the hole-injective support is the `GF(2)`-correct
    /// one). The general engine measures the full landscape: at `m = 3` the matching support is rescued
    /// by every odd prime (`GF(3), GF(5), GF(7)` all carry a re-checked witness), but at `m = 4` it
    /// fails over `GF(3)` as well — only `p ≥ 5` rescues it. The measured law across all eight
    /// (prime, m) points: **the classical support survives exactly when the characteristic clears a
    /// threshold growing with the family (`p ≥ m` here), with `GF(2)` merely the deepest failure** —
    /// the classical argument divides by counts that small primes annihilate, the same
    /// small-prime-divides-a-binomial mechanism as the paper's `Count_3` Lucas schedule. Meanwhile the
    /// hole-injective support carries the witness at every characteristic and every `m` (the control,
    /// per [`the_hole_injective_indicator_is_a_pseudo_expectation_at_every_characteristic`]). So the
    /// *bound* is characteristic-invariant while the *witness structure* is characteristic-graded — the
    /// honest refinement of the paper's "characteristic matters" theme: it matters at the support, on a
    /// threshold.
    #[test]
    fn the_php_witness_support_structure_differs_by_characteristic() {
        for m in [3usize, 4] {
            let (php, _) = crate::families::php(m);
            let nv = php.num_vars;
            let holes = m - 1;
            let d = 2 * holes - 1;
            let is_pm = |mo: Mono| crate::polycalc::php_is_partial_matching(mo, holes);
            // GF(2): the partial-matching support fails (the paper's parity-obstruction fact, re-run).
            let pm2 = crate::polycalc::ns_lower_bound_witness_on_basis(nv, &php.clauses, d, &is_pm);
            assert!(pm2.is_none(), "PHP({m}): over GF(2) the partial-matching sub-basis fails");
            for &p in &[3u64, 5, 7] {
                let f = NsField::Prime(p);
                let gens: Vec<GfpPoly> =
                    php.clauses.iter().map(|c| clause_polynomial_gfp(f, c)).collect();
                let pm = ns_lower_bound_witness_on_basis_gfp(f, nv, &gens, d, &is_pm);
                let expected = p >= m as u64; // the measured threshold: the support survives iff p ≥ m
                assert_eq!(
                    pm.is_some(),
                    expected,
                    "PHP({m}) over GF({p}): the partial-matching support carries a witness iff p ≥ m"
                );
                if let Some(w) = pm {
                    assert!(
                        check_ns_lower_bound_polys_gfp(f, nv, &gens, d, &w),
                        "PHP({m}) over GF({p}): the matching-support witness re-checks with zero trust"
                    );
                    assert!(
                        w.iter().all(|&(mo, _)| is_pm(mo)),
                        "PHP({m}) over GF({p}): the witness is genuinely supported on partial matchings"
                    );
                }
                // Control: the hole-injective sub-basis carries the witness at every characteristic.
                let hi = ns_lower_bound_witness_on_basis_gfp(f, nv, &gens, d, &|mo| {
                    crate::polycalc::php_is_hole_injective(mo, holes)
                })
                .expect("the hole-injective sub-basis carries the witness at every prime");
                assert!(check_ns_lower_bound_polys_gfp(f, nv, &gens, d, &hi));
            }
            eprintln!(
                "PHP({m}) d={d}: matching support — GF(2): ✗, GF(3): {}, GF(5): ✓, GF(7): ✓ (threshold p ≥ m)",
                if 3 >= m as u64 { "✓" } else { "✗" }
            );
        }
    }

    /// The variable permutation of PHP(m) induced by a pigeon permutation `sigma` (holes fixed):
    /// `x_{p,h} ↦ x_{sigma(p),h}`, with the [`crate::families::php`] layout `var(p,h) = p·holes + h`.
    fn php_pigeon_perm(m: usize, sigma: &dyn Fn(usize) -> usize) -> crate::proof::Perm {
        let holes = m - 1;
        let images: Vec<Lit> = (0..m * holes)
            .map(|v| {
                let (p, h) = (v / holes, v % holes);
                Lit::pos((sigma(p) * holes + h) as u32)
            })
            .collect();
        crate::proof::Perm::from_images(images)
    }

    /// **The annihilation dichotomy, both branches, both characteristics: symmetrizing kills the
    /// witness EXACTLY when `p` divides `|G|`.** The paper's §5.5 proved the char-2 case (every even
    /// group annihilates over `GF(2)`); the general engine shows the theorem is about `p | |G|`, not
    /// about 2. On PHP(3), take the pigeon transposition group `C₂` (order 2) and the pigeon 3-cycle
    /// group `C₃` (order 3), and cross them with `GF(2)` and `GF(3)`: the group-sum `Σ_g g·L` evaluates
    /// on the constant monomial to `|G| · L(1)`, so it annihilates in precisely two of the four cells —
    /// `C₂` over `GF(2)` and `C₃` over `GF(3)` — and survives (with the Reynolds operator available) in
    /// the other two. The same subgroup that kills a witness at one characteristic averages it perfectly
    /// at the other. Certified with a genuine solver-found witness in every cell.
    #[test]
    fn over_gfp_symmetrizing_annihilates_exactly_when_p_divides_the_group_order() {
        let m = 3usize;
        let (php, _) = crate::families::php(m);
        let nv = php.num_vars;
        let d = 2 * (m - 1) - 1;
        let c2 = crate::polycalc::close_perm_group(
            &[php_pigeon_perm(m, &|p| [1, 0, 2][p])],
            nv,
        );
        let c3 = crate::polycalc::close_perm_group(
            &[php_pigeon_perm(m, &|p| (p + 1) % m)],
            nv,
        );
        assert_eq!(c2.len(), 2, "the pigeon transposition generates C₂");
        assert_eq!(c3.len(), 3, "the pigeon 3-cycle generates C₃");
        for &p in &[2u64, 3] {
            let f = NsField::Prime(p);
            let w = ns_lower_bound_witness_gfp(f, nv, &php.clauses, d)
                .expect("a degree-3 witness exists at every characteristic (PHP degree is 4)");
            assert!(check_ns_lower_bound_gfp(f, nv, &php.clauses, d, &w), "the witness re-checks");
            assert_eq!(w.iter().find(|&&(mo, _)| mo == 0).map(|&(_, v)| v), Some(1), "L(1) = 1");
            for (group, order) in [(&c2, 2u64), (&c3, 3u64)] {
                let symmetrized = symmetrize_gfp(f, &w, group);
                let l1 = symmetrized.iter().find(|&&(mo, _)| mo == 0).map(|&(_, v)| v);
                let annihilates = order % p == 0;
                assert_eq!(
                    l1.is_none(),
                    annihilates,
                    "GF({p}) × C_{order}: the group-sum L(1) = |G|·1 = {order} vanishes iff {p} | {order}"
                );
                assert_eq!(
                    reynolds_gfp(f, &w, group).is_none(),
                    annihilates,
                    "GF({p}) × C_{order}: the Reynolds operator exists iff gcd(|G|, p) = 1"
                );
            }
        }
        // The paper's own case, through the general engine: the FULL pigeonhole group (order 12,
        // divisible by both 2 and 3) annihilates at BOTH characteristics.
        let full = crate::polycalc::close_perm_group(&crate::hypercube::php_perm_symmetries(m), nv);
        assert_eq!(full.len(), 12, "|S₃ × S₂| = 12");
        for &p in &[2u64, 3] {
            let f = NsField::Prime(p);
            let w = ns_lower_bound_witness_gfp(f, nv, &php.clauses, d).expect("witness exists");
            assert!(reynolds_gfp(f, &w, &full).is_none(), "GF({p}): 12 ≡ 0, the full group annihilates");
        }
    }

    /// **The constructive branch the paper could never exhibit: Reynolds averaging produces a valid,
    /// invariant witness whenever the order is invertible.** Over `GF(2)` every even group annihilates,
    /// so the paper had to build its symmetric witness natively (§5.5). The general engine takes the
    /// other branch: averaging a solver-found (unstructured) witness over a group of invertible order
    /// yields a functional that (i) re-checks as a valid pseudo-expectation with zero trust and (ii) is
    /// genuinely `G`-invariant, monomial by monomial. Exhibited in both directions — `C₂` over `GF(3)`
    /// (2 is a unit mod 3) and `C₃` over `GF(2)` (3 is odd) — so the averaging trick classical
    /// proof complexity takes for granted in characteristic 0 is machine-verified to work at every
    /// characteristic that misses the group order.
    #[test]
    fn the_reynolds_operator_produces_a_valid_symmetric_witness_when_the_order_is_invertible() {
        let m = 3usize;
        let (php, _) = crate::families::php(m);
        let nv = php.num_vars;
        let d = 2 * (m - 1) - 1;
        let c2 = crate::polycalc::close_perm_group(&[php_pigeon_perm(m, &|p| [1, 0, 2][p])], nv);
        let c3 = crate::polycalc::close_perm_group(&[php_pigeon_perm(m, &|p| (p + 1) % m)], nv);
        for (p, group, label) in [(3u64, &c2, "C₂ over GF(3)"), (2, &c3, "C₃ over GF(2)")] {
            let f = NsField::Prime(p);
            let w = ns_lower_bound_witness_gfp(f, nv, &php.clauses, d).expect("witness exists");
            let avg = reynolds_gfp(f, &w, group).expect("the order is invertible — Reynolds exists");
            assert!(
                check_ns_lower_bound_gfp(f, nv, &php.clauses, d, &avg),
                "{label}: the averaged witness is a valid pseudo-expectation (zero trust)"
            );
            let value: BTreeMap<Mono, u64> = avg.iter().copied().collect();
            for g in group {
                for &(mo, v) in &avg {
                    let img = crate::polycalc::apply_perm_to_mono(g, mo);
                    assert_eq!(
                        value.get(&img).copied().unwrap_or(0),
                        v,
                        "{label}: the averaged witness is G-invariant monomial-by-monomial"
                    );
                }
            }
            eprintln!("{label}: Reynolds-averaged witness valid + invariant ({} monomials)", avg.len());
        }
    }

    /// **`GF(4)` is a genuine field, checked exhaustively — every axiom over every tuple.** The 2-bit
    /// encoding `a + b·ω` with `ω² = ω + 1`: commutativity and identities over all 16 pairs,
    /// associativity and distributivity over all 64 triples, unique inverses, characteristic 2
    /// (`x + x = 0`), the defining quadratic, and the Frobenius fixed-point identity `x⁴ = x` that pins
    /// `GF(4)` as the 4-element field rather than the (non-field) ring `ℤ/4` — where `2·2 = 0` makes 2 a
    /// zero divisor, the exact reason "mod 4" is not a field and the extension construction is forced.
    #[test]
    fn gf4_arithmetic_satisfies_the_field_axioms_exhaustively() {
        let f = NsField::Gf4;
        let w = 2u64; // ω
        assert_eq!(f.characteristic(), 2);
        assert_eq!(f.order(), 4);
        assert_eq!(f.mul(w, w), f.add(w, 1), "the defining quadratic: ω² = ω + 1");
        for a in 0..4u64 {
            assert_eq!(f.add(a, 0), a, "additive identity");
            assert_eq!(f.mul(a, 1), a, "multiplicative identity");
            assert_eq!(f.add(a, a), 0, "characteristic 2");
            assert_eq!(f.mul(f.mul(f.mul(a, a), a), a), a, "Frobenius: x⁴ = x");
            if a != 0 {
                assert_eq!(f.mul(a, f.inv(a)), 1, "unique multiplicative inverse");
            }
            for b in 0..4u64 {
                assert_eq!(f.add(a, b), f.add(b, a), "commutative addition");
                assert_eq!(f.mul(a, b), f.mul(b, a), "commutative multiplication");
                if a != 0 && b != 0 {
                    assert_ne!(f.mul(a, b), 0, "a field has no zero divisors");
                }
                for c in 0..4u64 {
                    assert_eq!(f.add(f.add(a, b), c), f.add(a, f.add(b, c)), "associative +");
                    assert_eq!(f.mul(f.mul(a, b), c), f.mul(a, f.mul(b, c)), "associative ×");
                    assert_eq!(
                        f.mul(a, f.add(b, c)),
                        f.add(f.mul(a, b), f.mul(a, c)),
                        "distributivity"
                    );
                }
            }
        }
        // The contrast that motivates the extension: ℤ/4 is NOT a field — 2 is a zero divisor there.
        assert_eq!((2u64 * 2) % 4, 0, "in ℤ/4, 2·2 = 0 — the ring mod 4 is not GF(4)");
    }

    /// **The extension-field collapse, measured across the whole small census: `GF(4)` buys NOTHING.**
    /// NS degree depends only on the characteristic, not the field size — so the "field ladder" is
    /// really the PRIME ladder, and shifting `GF(2) → GF(4)` (unlike `GF(2) → GF(3)`) changes no
    /// verdict. Measured exhaustively: every minimal-UNSAT orbit representative at `n = 1, 2, 3` (48
    /// covers), plus PHP(3) and the Count_3(4) CNF, has *identical* minimum NS degree under the `GF(4)`
    /// engine and the specialized `GF(2)` engine — and identical refutation verdicts at every
    /// intermediate degree, not just at the minimum.
    #[test]
    fn ns_degree_over_gf4_equals_ns_degree_over_gf2_across_the_small_census() {
        let f4 = NsField::Gf4;
        let mut corpus: Vec<(usize, Vec<Vec<Lit>>)> = Vec::new();
        for n in 1..=3usize {
            for cover in crate::hypercube::minimal_cover_orbits(n) {
                corpus.push((n, cover.clauses()));
            }
        }
        let (php3, _) = crate::families::php(3);
        corpus.push((php3.num_vars, php3.clauses));
        let (cnt34, _) = crate::families::mod_counting(4, 3);
        corpus.push((cnt34.num_vars, cnt34.clauses));
        let mut checked = 0usize;
        for (nv, clauses) in &corpus {
            let mut min_gf4 = None;
            let mut min_gf2 = None;
            for d in 1..=*nv {
                let v4 = ns_refutes_gfp(f4, *nv, clauses, d);
                let v2 = crate::polycalc::nullstellensatz_refutes(*nv, clauses, d);
                assert_eq!(v4, v2, "n={nv} d={d}: GF(4) and GF(2) verdicts agree everywhere");
                if v4 && min_gf4.is_none() {
                    min_gf4 = Some(d);
                }
                if v2 && min_gf2.is_none() {
                    min_gf2 = Some(d);
                }
            }
            assert_eq!(min_gf4, min_gf2, "n={nv}: the minimum NS degree collapses to characteristic 2");
            assert!(min_gf4.is_some(), "every census cover is UNSAT and refuted by degree n");
            checked += 1;
        }
        assert!(checked >= 50, "the sweep covered the census plus the named families ({checked})");
        eprintln!("GF(4) ≡ GF(2) on all {checked} covers — the field ladder is the prime ladder");
    }

    /// **The collapse is CONSTRUCTIVE: a `GF(4)` certificate projects coefficient-wise to a re-checking
    /// `GF(2)` certificate.** `λ : a + b·ω ↦ a` is `GF(2)`-linear with `λ(1) = 1`, and clause
    /// polynomials have prime-subfield coefficients, so `λ` slides through `Σ_C p_C · g_C = 1`
    /// monomial-by-monomial. Verified end-to-end: extract a provenance-tracked `GF(4)` certificate at
    /// the minimal degree (it re-checks in-ring and corner-wise over `GF(4)`), project it, and the image
    /// re-checks as a `GF(2)` certificate of no larger degree — through BOTH the general engine's
    /// verifier at `Prime(2)` and the corner evaluation. The projection also refuses non-`GF(4)` input
    /// (fail-closed) rather than silently projecting a prime-field certificate.
    #[test]
    fn a_gf4_certificate_projects_coefficientwise_to_a_rechecking_gf2_certificate() {
        let f4 = NsField::Gf4;
        let p = |v: u32| Lit::pos(v);
        let q = |v: u32| Lit::neg(v);
        let xor_core = vec![
            vec![q(0), p(1)], vec![p(0), q(1)],
            vec![q(1), p(2)], vec![p(1), q(2)],
            vec![p(0), p(2)], vec![q(0), q(2)],
        ];
        let (php3, _) = crate::families::php(3);
        let (cnt34, _) = crate::families::mod_counting(4, 3);
        let corpus: Vec<(usize, Vec<Vec<Lit>>)> = vec![
            (3, xor_core),
            (php3.num_vars, php3.clauses),
            (cnt34.num_vars, cnt34.clauses),
        ];
        for (nv, clauses) in &corpus {
            let min_d = (1..=*nv)
                .find(|&d| ns_refutes_gfp(f4, *nv, clauses, d))
                .expect("every corpus formula is UNSAT");
            let cert4 = ns_certificate_at_degree_gfp(f4, *nv, clauses, min_d)
                .expect("a certificate exists exactly where the decision engine refutes");
            assert_eq!(cert4.field(), NsField::Gf4);
            assert!(cert4.verify(clauses), "n={nv}: the GF(4) certificate re-checks (ring + corners)");
            assert!(cert4.degree() <= min_d, "n={nv}: the extracted certificate respects the degree");
            let cert2 = project_gf4_certificate_to_gf2(&cert4).expect("projection of a Gf4 certificate");
            assert_eq!(cert2.field(), NsField::Prime(2));
            assert!(
                cert2.verify(clauses),
                "n={nv}: the λ-projected certificate re-checks over GF(2) — the constructive collapse"
            );
            assert!(cert2.degree() <= cert4.degree(), "n={nv}: projection never raises the degree");
            // Fail-closed: projecting a prime-field certificate is refused, not silently accepted.
            assert!(project_gf4_certificate_to_gf2(&cert2).is_none());
            // And extraction is fail-closed below the minimum degree: no certificate is fabricated.
            if min_d > 1 {
                assert!(ns_certificate_at_degree_gfp(f4, *nv, clauses, min_d - 1).is_none());
            }
            eprintln!(
                "n={nv}: GF(4) certificate at degree {min_d} projects to a GF(2) certificate (degree {})",
                cert2.degree()
            );
        }
    }

    fn sat(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
        (0u64..(1u64 << num_vars)).any(|x| {
            clauses.iter().all(|c| c.iter().any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive()))
        })
    }

    /// **The general engine at `p = 2` IS the specialized `GF(2)` engine — the differential anchor.**
    /// The bitset engine ([`crate::polycalc`]) is coefficient-free XOR; this engine carries explicit
    /// coefficients and signed indicators. At `p = 2` the two must coincide exactly: refutation verdicts
    /// at every degree, witness existence (the duality), and each engine's witness passing the *other*
    /// engine's zero-trust checker. Corpus: pigeonhole, modular counting, a transitive-XOR core, and a
    /// deterministic random-3-CNF sweep. Once this holds, every claim the general engine makes at odd `p`
    /// stands on machinery already pinned to the paper's `GF(2)` results.
    #[test]
    fn the_general_engine_at_p_two_agrees_with_the_specialized_gf2_engine() {
        let f = NsField::Prime(2);
        let mut corpus: Vec<(usize, Vec<Vec<Lit>>)> = Vec::new();
        let (php3, _) = crate::families::php(3);
        corpus.push((php3.num_vars, php3.clauses));
        let (cnt32, _) = crate::families::mod_counting(3, 2);
        corpus.push((cnt32.num_vars, cnt32.clauses));
        let p = |v: u32| Lit::pos(v);
        let q = |v: u32| Lit::neg(v);
        corpus.push((3, vec![
            vec![q(0), p(1)], vec![p(0), q(1)],
            vec![q(1), p(2)], vec![p(1), q(2)],
            vec![p(0), p(2)], vec![q(0), q(2)],
        ]));
        let mut seed = 0xD1FF_A4C4u64;
        for _ in 0..16 {
            let nv = 4 + (lcg(&mut seed) % 3) as usize; // 4..=6 variables
            let nc = 6 + (lcg(&mut seed) % 10) as usize;
            let mut cl = Vec::new();
            for _ in 0..nc {
                let mut vars: Vec<u32> = Vec::new();
                while vars.len() < 3 {
                    let v = (lcg(&mut seed) % nv as u64) as u32;
                    if !vars.contains(&v) {
                        vars.push(v);
                    }
                }
                cl.push(vars.iter().map(|&v| Lit::new(v, lcg(&mut seed) & 1 == 1)).collect());
            }
            corpus.push((nv, cl));
        }

        for (nv, clauses) in &corpus {
            for d in 1..=(*nv).min(4) {
                let gf2_verdict = crate::polycalc::nullstellensatz_refutes(*nv, clauses, d);
                assert_eq!(
                    ns_refutes_gfp(f, *nv, clauses, d),
                    gf2_verdict,
                    "n={nv} d={d}: the general engine at p=2 matches the bitset engine"
                );
                let w_gf2 = crate::polycalc::ns_lower_bound_witness(*nv, clauses, d);
                let w_gen = ns_lower_bound_witness_gfp(f, *nv, clauses, d);
                assert_eq!(
                    w_gf2.is_some(),
                    w_gen.is_some(),
                    "n={nv} d={d}: witness existence agrees across the engines"
                );
                assert_eq!(w_gen.is_none(), gf2_verdict, "n={nv} d={d}: the duality holds");
                if let (Some(wc), Some(wp)) = (w_gf2, w_gen) {
                    let wc_as_pairs: Vec<(Mono, u64)> = wc.iter().map(|&m| (m, 1)).collect();
                    assert!(
                        check_ns_lower_bound_gfp(f, *nv, clauses, d, &wc_as_pairs),
                        "n={nv} d={d}: the bitset engine's witness passes the general checker"
                    );
                    assert!(wp.iter().all(|&(_, v)| v == 1), "p=2 witness values are all 1");
                    let wp_as_monos: Vec<u64> = wp.iter().map(|&(m, _)| m).collect();
                    assert!(
                        crate::polycalc::check_ns_lower_bound(*nv, clauses, d, &wp_as_monos),
                        "n={nv} d={d}: the general engine's witness passes the bitset checker"
                    );
                }
            }
        }
    }

    /// **The completeness construction is field-generic: total, sound, and fail-closed over `GF(3)`.**
    /// The partition-of-unity charging never sees the characteristic — every UNSAT formula gets a
    /// certificate whose signed coefficients re-check (in the ring AND corner-by-corner), every SAT
    /// formula gets a model that satisfies it, and a certificate refuses to verify a clause set it was
    /// not built for. Cross-checked against brute-force satisfiability throughout.
    #[test]
    fn build_ns_certificate_gfp_is_total_sound_and_fail_closed_over_gf3() {
        let f = NsField::Prime(3);
        let mut seed = 0x0BAD_5EEDu64;
        let mut unsat_seen = 0usize;
        for _ in 0..80 {
            let nv = 4 + (lcg(&mut seed) % 3) as usize; // 4..=6 variables
            let nc = nv + (lcg(&mut seed) % (2 * nv as u64)) as usize;
            let clauses: Vec<Vec<Lit>> = (0..nc)
                .map(|_| {
                    let width = 2 + (lcg(&mut seed) % 2) as usize;
                    let mut vars: Vec<u32> = Vec::new();
                    while vars.len() < width {
                        let v = (lcg(&mut seed) % nv as u64) as u32;
                        if !vars.contains(&v) {
                            vars.push(v);
                        }
                    }
                    vars.iter().map(|&v| Lit::new(v, lcg(&mut seed) & 1 == 1)).collect()
                })
                .collect();
            match build_ns_certificate_gfp(f, nv, &clauses) {
                Ok(cert) => {
                    unsat_seen += 1;
                    assert_eq!(cert.field(), f);
                    assert!(cert.verify(&clauses), "n={nv}: the GF(3) certificate re-checks");
                    assert!(cert.degree() <= nv, "n={nv}: certificate degree ≤ n");
                    assert!(!sat(nv, &clauses), "n={nv}: certificates only for genuinely UNSAT formulas");
                    assert!(
                        !cert.verify(&clauses[..clauses.len() - 1]),
                        "n={nv}: a certificate must not verify a different clause set"
                    );
                }
                Err(model) => {
                    assert!(sat(nv, &clauses), "n={nv}: SAT verdicts only for satisfiable formulas");
                    assert!(
                        clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())),
                        "n={nv}: the returned model satisfies every clause"
                    );
                }
            }
        }
        assert!(unsat_seen >= 5, "the corpus exercises the UNSAT branch ({unsat_seen} instances)");
    }

    /// **`GF(3)` degree lower bounds are certifiable and exactly dual to refutation — and the checker is
    /// genuinely zero-trust.** On a deterministic random corpus: a witness exists iff no degree-`d`
    /// refutation does, and every witness re-checks. Then the adversarial half: a witness with its
    /// normalization `L(1)` dropped, one rescaled by `2` (so `L(1) = 2 ≠ 1`), and one perturbed on a
    /// monomial inside an admitted generator product must all be REJECTED. Empty clauses fail closed
    /// across the API.
    #[test]
    fn gf3_degree_lower_bounds_are_certifiable_and_dual_to_refutation() {
        let f = NsField::Prime(3);
        let mut seed = 0xD0A1_0003u64;
        for _ in 0..60 {
            let nv = 3 + (lcg(&mut seed) % 3) as usize; // 3..=5
            let nc = 2 + (lcg(&mut seed) % 8) as usize;
            let clauses: Vec<Vec<Lit>> = (0..nc)
                .map(|_| {
                    let mut c = Vec::new();
                    for v in 0..nv {
                        if lcg(&mut seed) % 2 == 0 {
                            c.push(Lit::new(v as u32, lcg(&mut seed) % 2 == 0));
                        }
                    }
                    if c.is_empty() {
                        c.push(Lit::new((lcg(&mut seed) % nv as u64) as u32, lcg(&mut seed) % 2 == 0));
                    }
                    c
                })
                .collect();
            let gens: Vec<GfpPoly> = clauses.iter().map(|c| clause_polynomial_gfp(f, c)).collect();
            for d in 1..=nv {
                let refutes = ns_refutes_gfp(f, nv, &clauses, d);
                match ns_lower_bound_witness_gfp(f, nv, &clauses, d) {
                    Some(w) => {
                        assert!(!refutes, "a witness exists only when there is NO degree-{d} refutation");
                        assert!(
                            check_ns_lower_bound_gfp(f, nv, &clauses, d, &w),
                            "the GF(3) witness must re-check"
                        );
                        // Corruption 1: drop the normalization L(1) = 1.
                        let no_one: Vec<(Mono, u64)> =
                            w.iter().copied().filter(|&(m, _)| m != 0).collect();
                        assert!(
                            !check_ns_lower_bound_gfp(f, nv, &clauses, d, &no_one),
                            "a witness without L(1) = 1 is rejected"
                        );
                        // Corruption 2: rescale by 2 — every constraint still holds, but L(1) = 2 ≠ 1.
                        let scaled: Vec<(Mono, u64)> =
                            w.iter().map(|&(m, v)| (m, f.mul(v, 2))).collect();
                        assert!(
                            !check_ns_lower_bound_gfp(f, nv, &clauses, d, &scaled),
                            "a rescaled witness breaks the normalization and is rejected"
                        );
                        // Corruption 3: perturb L on a monomial inside an admitted generator product.
                        let target = gens.iter().find_map(|g| {
                            crate::polycalc::monomials_up_to_degree(nv, d).into_iter().find_map(|m| {
                                let prod = poly_mul_mono_gfp(f, g, m);
                                (!prod.is_empty() && gfp_poly_degree(&prod) <= d)
                                    .then(|| *prod.keys().next_back().unwrap())
                            })
                        });
                        if let Some(t) = target {
                            let mut perturbed: Vec<(Mono, u64)> = w
                                .iter()
                                .copied()
                                .filter(|&(m, _)| m != t)
                                .collect();
                            let old = w.iter().find(|&&(m, _)| m == t).map_or(0, |&(_, v)| v);
                            let bumped = f.add(old, 1);
                            if bumped != 0 {
                                perturbed.push((t, bumped));
                            }
                            assert!(
                                !check_ns_lower_bound_gfp(f, nv, &clauses, d, &perturbed),
                                "a witness perturbed on a constrained monomial is rejected"
                            );
                        }
                    }
                    None => assert!(refutes, "no witness ⟹ a degree-{d} refutation exists"),
                }
            }
        }
        // Empty clauses fail closed across the API.
        let with_empty: Vec<Vec<Lit>> = vec![vec![], vec![Lit::pos(0)]];
        assert!(ns_refutes_gfp(f, 1, &with_empty, 1), "an empty clause is 1 = 0 outright");
        assert!(ns_lower_bound_witness_gfp(f, 1, &with_empty, 1).is_none());
        assert!(!check_ns_lower_bound_gfp(f, 1, &with_empty, 1, &[(0, 1)]));
    }

    /// **`gfp_solve` decides linear systems over `GF(p)` and both verdicts re-check.** Planted-solution
    /// systems (rhs generated from a random assignment) come back `Some`, and the returned solution — not
    /// necessarily the planted one — satisfies every equation by direct substitution. Duplicating a row
    /// with a shifted right-hand side manufactures `r = c` ∧ `r = c+1`, which must return `None`. The
    /// degenerate rows behave: `0 = 0` is consistent, `0 = 1` is not.
    #[test]
    fn gfp_solve_solutions_and_inconsistencies_both_recheck() {
        let mut seed = 0x5EED_5017u64;
        for &p in &[2u64, 3, 5, 7, 11] {
            let f = NsField::Prime(p);
            for _ in 0..60 {
                let nvars = 1 + (lcg(&mut seed) % 8) as usize;
                let planted: Vec<u64> = (0..nvars).map(|_| lcg(&mut seed) % p).collect();
                let rows = 1 + (lcg(&mut seed) % 10) as usize;
                let mut eqs: Vec<(Vec<(usize, u64)>, u64)> = Vec::new();
                for _ in 0..rows {
                    let coeffs: Vec<(usize, u64)> = (0..nvars)
                        .filter_map(|v| {
                            let c = lcg(&mut seed) % p;
                            (c != 0).then_some((v, c))
                        })
                        .collect();
                    let rhs =
                        coeffs.iter().fold(0u64, |acc, &(v, c)| f.add(acc, f.mul(c, planted[v])));
                    eqs.push((coeffs, rhs));
                }
                let x = gfp_solve(f, &eqs, nvars).expect("a planted-solution system is consistent");
                for (coeffs, rhs) in &eqs {
                    let lhs = coeffs.iter().fold(0u64, |acc, &(v, c)| f.add(acc, f.mul(c, x[v])));
                    assert_eq!(lhs, *rhs, "p={p}: the returned solution satisfies every equation");
                }
                if let Some((coeffs, rhs)) = eqs.iter().find(|(c, _)| !c.is_empty()).cloned() {
                    let mut bad = eqs.clone();
                    bad.push((coeffs, f.add(rhs, 1)));
                    assert_eq!(
                        gfp_solve(f, &bad, nvars),
                        None,
                        "p={p}: the same row with a shifted rhs is inconsistent"
                    );
                }
            }
            assert!(gfp_solve(f, &[(vec![], 0)], 3).is_some(), "p={p}: 0 = 0 is consistent");
            assert_eq!(gfp_solve(f, &[(vec![], 1)], 3), None, "p={p}: 0 = 1 is a contradiction");
        }
    }
}
