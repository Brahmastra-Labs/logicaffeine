//! The **Steenrod algebra** acting on `H*(BZ/2; Z/2) = Z/2[x]` — the deepest layer of cohomology
//! operations, and a place where homotopy theory turns out to *be* number theory.
//!
//! The total Steenrod square `Sq = Σᵢ Sqⁱ` is the ring endomorphism of `Z/2[x]` determined by
//! `Sq(x) = x + x²` (`Sq⁰x = x`, `Sq¹x = x²` since `|x| = 1`, `Sqⁱx = 0` for `i > 1`) extended
//! multiplicatively by the **Cartan formula** `Sq(ab) = Sq(a)Sq(b)`. Computing it is then pure
//! polynomial arithmetic over `Z/2`, and the answer is striking:
//!
//! ```text
//!   Sqⁱ(xᵏ) = coefficient of x^{k+i} in (x + x²)ᵏ = C(k, i) mod 2
//! ```
//!
//! — the binomial coefficients mod 2, i.e. **Lucas' theorem** (`C(k,i) ≡ 1 ⟺ i AND k = i` bit-wise).
//! The top square `Sqᵏ(xᵏ) = x^{2k}` agrees with the cup-square ([`crate::postnikov::cup_square`]); the
//! whole infinite family of operations is governed by one binomial law.

/// A polynomial over `Z/2`: `p[d]` is the coefficient of `xᵈ` (each `0` or `1`).
fn add(a: &[u8], b: &[u8]) -> Vec<u8> {
    let n = a.len().max(b.len());
    (0..n)
        .map(|i| (a.get(i).copied().unwrap_or(0) + b.get(i).copied().unwrap_or(0)) % 2)
        .collect()
}

fn mul(a: &[u8], b: &[u8]) -> Vec<u8> {
    if a.is_empty() || b.is_empty() {
        return vec![];
    }
    let mut out = vec![0u8; a.len() + b.len() - 1];
    for (i, &ai) in a.iter().enumerate() {
        if ai == 1 {
            for (j, &bj) in b.iter().enumerate() {
                out[i + j] = (out[i + j] + bj) % 2;
            }
        }
    }
    out
}

/// The monomial `xᵏ`.
pub fn monomial(k: usize) -> Vec<u8> {
    let mut p = vec![0u8; k + 1];
    p[k] = 1;
    p
}

fn trim(mut p: Vec<u8>) -> Vec<u8> {
    while p.len() > 1 && *p.last().unwrap() == 0 {
        p.pop();
    }
    p
}

/// The **total Steenrod square** `Sq`, as the ring endomorphism of `Z/2[x]` sending `x ↦ x + x²`
/// (Cartan formula). Returns `Sq(p)` as a polynomial.
pub fn total_square(poly: &[u8]) -> Vec<u8> {
    let x_plus_x2 = [0u8, 1, 1]; // x + x²
    let mut result = vec![0u8];
    let mut power = vec![1u8]; // (x + x²)^0
    for &c in poly {
        if c == 1 {
            result = add(&result, &power);
        }
        power = mul(&power, &x_plus_x2);
    }
    trim(result)
}

/// `Sqⁱ(xᵏ)` on `H*(BZ/2; Z/2)` — the coefficient of `x^{k+i}` in `Sq(xᵏ) = (x + x²)ᵏ`.
pub fn sq(i: usize, k: usize) -> u8 {
    let s = total_square(&monomial(k));
    s.get(k + i).copied().unwrap_or(0)
}

/// `C(k, i)` (binomial coefficient), computed by Pascal's recurrence — independent of the topology, so
/// matching it against [`sq`] is a genuine theorem, not a definition.
fn binom(k: usize, i: usize) -> u64 {
    if i > k {
        return 0;
    }
    let mut row = vec![1u64; 1];
    for r in 1..=k {
        let mut next = vec![1u64; r + 1];
        for c in 1..r {
            next[c] = row[c - 1] + row[c];
        }
        row = next;
    }
    row[i]
}

/// Apply a single `Sqⁱ` to a class in `H*(BZ/2; Z/2) = Z/2[x]`: `Sqⁱ(Σ aₖ xᵏ) = Σ aₖ C(k,i) x^{k+i}`.
pub fn apply_sq(i: usize, poly: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; poly.len() + i];
    for (k, &a) in poly.iter().enumerate() {
        if a == 1 && binom(k, i) % 2 == 1 {
            out[k + i] ^= 1;
        }
    }
    trim(out)
}

/// The Adem relation applied to `SqᵃSqᵇ` (`a < 2b`): `Σ_c C(b-c-1, a-2c) Sq^{a+b-c} Sq^c` over `Z/2`,
/// returned as a list of monomials (`Sq⁰` = identity is dropped).
fn adem(a: usize, b: usize) -> Vec<Vec<usize>> {
    let mut terms = Vec::new();
    for c in 0..=(a / 2) {
        if binom(b - c - 1, a - 2 * c) % 2 == 1 {
            let mut m = vec![a + b - c];
            if c > 0 {
                m.push(c);
            }
            terms.push(m);
        }
    }
    terms
}

/// Is a monomial `Sq^{i₁}…Sq^{iₖ}` **admissible**? (`i_j ≥ 2 i_{j+1}` for all adjacent pairs.)
pub fn is_admissible(m: &[usize]) -> bool {
    m.iter().all(|&i| i >= 1) && m.windows(2).all(|w| w[0] >= 2 * w[1])
}

/// Reduce a Steenrod monomial to its `Z/2` sum of **admissible** monomials via the Adem relations — the
/// canonical form in the Steenrod algebra. A `HashSet` models the `Z/2` linear combination (present =
/// coefficient 1; re-insertion cancels).
pub fn adem_reduce(m: &[usize]) -> std::collections::HashSet<Vec<usize>> {
    let m: Vec<usize> = m.iter().copied().filter(|&i| i > 0).collect();
    if is_admissible(&m) {
        let mut s = std::collections::HashSet::new();
        if !m.is_empty() {
            s.insert(m);
        }
        return s;
    }
    let j = m.windows(2).position(|w| w[0] < 2 * w[1]).unwrap();
    let (a, b) = (m[j], m[j + 1]);
    let mut result: std::collections::HashSet<Vec<usize>> = std::collections::HashSet::new();
    for term in adem(a, b) {
        let mut spliced = m[..j].to_vec();
        spliced.extend_from_slice(&term);
        spliced.extend_from_slice(&m[j + 2..]);
        for adm in adem_reduce(&spliced) {
            if !result.insert(adm.clone()) {
                result.remove(&adm); // XOR over Z/2
            }
        }
    }
    result
}

/// Over `Z/2`, is `target` in the linear span of `generators`? Each "vector" is a set of admissible
/// monomials (its `Z/2` support); XOR is symmetric difference; the pivot is the largest monomial.
fn z2_span_contains(
    generators: Vec<std::collections::HashSet<Vec<usize>>>,
    target: std::collections::HashSet<Vec<usize>>,
) -> bool {
    type Vec2 = std::collections::HashSet<Vec<usize>>;
    fn reduce(mut v: Vec2, basis: &[(Vec<usize>, Vec2)]) -> Vec2 {
        loop {
            if v.is_empty() {
                return v;
            }
            let piv = v.iter().max().unwrap().clone();
            match basis.iter().find(|(p, _)| *p == piv) {
                Some((_, bset)) => {
                    for x in bset {
                        if !v.insert(x.clone()) {
                            v.remove(x);
                        }
                    }
                }
                None => return v,
            }
        }
    }
    let mut basis: Vec<(Vec<usize>, Vec2)> = Vec::new();
    for g in generators {
        let r = reduce(g, &basis);
        if !r.is_empty() {
            let piv = r.iter().max().unwrap().clone();
            basis.push((piv, r));
        }
    }
    reduce(target, &basis).is_empty()
}

/// The `Z/2` rank of a set of "vectors" (sets of admissible monomials) — Gaussian elimination, pivot =
/// largest monomial.
fn z2_rank(generators: Vec<std::collections::HashSet<Vec<usize>>>) -> usize {
    type Vec2 = std::collections::HashSet<Vec<usize>>;
    fn reduce(mut v: Vec2, basis: &[(Vec<usize>, Vec2)]) -> Vec2 {
        loop {
            if v.is_empty() {
                return v;
            }
            let piv = v.iter().max().unwrap().clone();
            match basis.iter().find(|(p, _)| *p == piv) {
                Some((_, bset)) => {
                    for x in bset {
                        if !v.insert(x.clone()) {
                            v.remove(x);
                        }
                    }
                }
                None => return v,
            }
        }
    }
    let mut basis: Vec<(Vec<usize>, Vec2)> = Vec::new();
    for g in generators {
        let r = reduce(g, &basis);
        if !r.is_empty() {
            let piv = r.iter().max().unwrap().clone();
            basis.push((piv, r));
        }
    }
    basis.len()
}

/// The **admissible monomials** of total degree `t` — a basis of the Steenrod algebra `𝒜_t`.
pub fn admissibles_of_degree(t: usize) -> Vec<Vec<usize>> {
    fn rec(remaining: usize, min_left: usize, suffix: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if remaining == 0 {
            out.push(suffix.clone());
            return;
        }
        for i in min_left.max(1)..=remaining {
            suffix.insert(0, i);
            rec(remaining - i, 2 * i, suffix, out);
            suffix.remove(0);
        }
    }
    let mut out = Vec::new();
    rec(t, 1, &mut Vec::new(), &mut out);
    out
}

/// `dim Ext^{1,t}_𝒜(Z/2, Z/2)` — the first line of the Adams `E₂` page = the **indecomposables** of the
/// Steenrod algebra in degree `t`. Equals `dim 𝒜_t − dim(𝒜⁺·𝒜⁺)_t`. By Milnor it is `1` exactly when
/// `t` is a power of 2 (the `h_i = Sq^{2ⁱ}`), and `0` otherwise.
pub fn adams_one_line(t: usize) -> usize {
    let dim_at = admissibles_of_degree(t).len();
    let mut decomposables: Vec<std::collections::HashSet<Vec<usize>>> = Vec::new();
    for d in 1..t {
        for a in admissibles_of_degree(d) {
            for b in admissibles_of_degree(t - d) {
                let mut prod = a.clone();
                prod.extend_from_slice(&b);
                decomposables.push(adem_reduce(&prod));
            }
        }
    }
    dim_at - z2_rank(decomposables)
}

/// Is `Sqⁿ` **decomposable** — a `Z/2` combination of products `SqᵃSqᵇ` (`a, b ≥ 1`) — in the Steenrod
/// algebra? Decided by reducing every such product to the admissible basis (Adem) and testing whether
/// `Sqⁿ` lies in their span.
pub fn is_decomposable_sq(n: usize) -> bool {
    let generators: Vec<_> = (1..n).map(|a| adem_reduce(&[a, n - a])).collect();
    let target: std::collections::HashSet<Vec<usize>> = [vec![n]].into_iter().collect();
    z2_span_contains(generators, target)
}

/// An unbounded `Z/2` bit-vector (the resolution outgrows `u128` once the Steenrod algebra's degree-wise
/// dimension exceeds 128). Word-packed; the pivot is always the lowest set bit.
#[derive(Clone, PartialEq, Eq)]
struct Bits(Vec<u64>);

impl Bits {
    fn new() -> Self {
        Bits(Vec::new())
    }
    fn bit(i: usize) -> Self {
        let mut b = Bits::new();
        b.toggle(i);
        b
    }
    fn from_indices(it: impl Iterator<Item = usize>) -> Self {
        let mut b = Bits::new();
        for i in it {
            b.toggle(i);
        }
        b
    }
    fn toggle(&mut self, i: usize) {
        let w = i / 64;
        if w >= self.0.len() {
            self.0.resize(w + 1, 0);
        }
        self.0[w] ^= 1u64 << (i % 64);
    }
    fn get(&self, i: usize) -> bool {
        let w = i / 64;
        w < self.0.len() && (self.0[w] >> (i % 64)) & 1 == 1
    }
    fn is_zero(&self) -> bool {
        self.0.iter().all(|&w| w == 0)
    }
    /// The lowest set bit — the Gaussian-elimination pivot.
    fn lowest(&self) -> Option<usize> {
        self.0
            .iter()
            .enumerate()
            .find(|(_, &w)| w != 0)
            .map(|(wi, &w)| wi * 64 + w.trailing_zeros() as usize)
    }
}

impl std::ops::BitXorAssign<&Bits> for Bits {
    fn bitxor_assign(&mut self, o: &Bits) {
        if o.0.len() > self.0.len() {
            self.0.resize(o.0.len(), 0);
        }
        for (a, b) in self.0.iter_mut().zip(&o.0) {
            *a ^= b;
        }
    }
}

/// Basis of the `Z/2` nullspace of the linear map whose `j`-th column is `cols[j]` (a bitmask over the
/// codomain): each returned `Bits` is a mask over the *domain* indices summing the columns to zero.
fn z2_nullspace(cols: &[Bits]) -> Vec<Bits> {
    let mut pivots: std::collections::HashMap<usize, (Bits, Bits)> =
        std::collections::HashMap::new();
    let mut kernel = Vec::new();
    for (j, col) in cols.iter().enumerate() {
        let (mut cod, mut trk) = (col.clone(), Bits::bit(j));
        while let Some(lead) = cod.lowest() {
            match pivots.get(&lead) {
                Some((pcod, ptrk)) => {
                    cod ^= pcod;
                    trk ^= ptrk;
                }
                None => break,
            }
        }
        match cod.lowest() {
            None => kernel.push(trk),
            Some(lead) => {
                pivots.insert(lead, (cod, trk));
            }
        }
    }
    kernel
}

/// `Z/2` rank of a set of `Bits` row vectors.
fn z2_rank_u128(rows: Vec<Bits>) -> usize {
    let mut pivots: std::collections::HashMap<usize, Bits> = std::collections::HashMap::new();
    for mut v in rows {
        while let Some(lead) = v.lowest() {
            match pivots.get(&lead) {
                Some(p) => v ^= p,
                None => {
                    pivots.insert(lead, v);
                    break;
                }
            }
        }
    }
    pivots.len()
}

/// Basis of the degree-`s` part of `C₁ = ⊕ᵢ 𝒜·hᵢ` in the minimal resolution: `(i, m)` with `2ⁱ ≤ s` and
/// `m` an admissible of degree `s − 2ⁱ` (the element `m·hᵢ`).
fn c1_basis(s: usize) -> Vec<(usize, Vec<usize>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while (1usize << i) <= s {
        for m in admissibles_of_degree(s - (1 << i)) {
            out.push((i, m));
        }
        i += 1;
    }
    out
}

/// `(basis, kernel)` of `d₁ : C₁ → 𝒜` in degree `s`, where `d₁(m·hᵢ) = m·Sq^{2ⁱ}`. Kernel vectors are
/// masks over the returned `c1_basis(s)`.
fn ker_d1(s: usize) -> (Vec<(usize, Vec<usize>)>, Vec<Bits>) {
    let basis = c1_basis(s);
    let cod_idx: std::collections::HashMap<Vec<usize>, usize> = admissibles_of_degree(s)
        .into_iter()
        .enumerate()
        .map(|(i, m)| (m, i))
        .collect();
    let cols: Vec<Bits> = basis
        .iter()
        .map(|(i, m)| {
            let mut prod = m.clone();
            prod.push(1 << i); // m · Sq^{2^i}
            let mut mask = Bits::new();
            for adm in adem_reduce(&prod) {
                if let Some(&idx) = cod_idx.get(&adm) {
                    mask.toggle(idx);
                }
            }
            mask
        })
        .collect();
    let kernel = z2_nullspace(&cols);
    (basis, kernel)
}

/// `dim Ext^{2,t}_𝒜(Z/2, Z/2)` — the second line of the Adams `E₂` page: the minimal generators of
/// `ker(d₁)` in degree `t`, i.e. `dim ker(d₁)_t − dim(𝒜⁺·ker(d₁))_t`. The relations among the `Sq^{2ⁱ}`.
pub fn adams_two_line(t: usize) -> usize {
    let (_basis_t, ker_t) = ker_d1(t);
    let c1_idx_t: std::collections::HashMap<(usize, Vec<usize>), usize> = c1_basis(t)
        .into_iter()
        .enumerate()
        .map(|(i, b)| (b, i))
        .collect();
    let mut decomposables: Vec<Bits> = Vec::new();
    for k in 1..t {
        let (basis_s, ker_s) = ker_d1(t - k);
        for zmask in &ker_s {
            let mut result = Bits::new();
            for (b, (i, m)) in basis_s.iter().enumerate() {
                if zmask.get(b) {
                    let mut prod = vec![k];
                    prod.extend_from_slice(m);
                    for mprime in adem_reduce(&prod) {
                        if let Some(&idx) = c1_idx_t.get(&(*i, mprime)) {
                            result.toggle(idx);
                        }
                    }
                }
            }
            if !result.is_zero() {
                decomposables.push(result);
            }
        }
    }
    ker_t.len() - z2_rank_u128(decomposables)
}

/// The known `dim Ext^{2,t}` from the chart: `#{(i,j) : i ≤ j, j ≠ i+1, 2ⁱ + 2ʲ = t}` (the `h_i h_j`
/// with the adjacency relation `h_i h_{i+1} = 0`).
fn known_ext2(t: usize) -> usize {
    let mut count = 0;
    let mut i = 0;
    while (1usize << i) <= t {
        let mut j = i;
        while (1usize << i) + (1usize << j) <= t {
            if (1 << i) + (1 << j) == t && j != i + 1 {
                count += 1;
            }
            j += 1;
        }
        i += 1;
    }
    count
}

/// A generator of the minimal free resolution: its internal degree and its boundary `d(g) ∈ C_{s-1}`,
/// a `Z/2` set of `(previous-generator-index, admissible monomial)` pairs (the element `Σ a·g'`).
#[derive(Clone)]
struct ResGen {
    degree: usize,
    boundary: Vec<(usize, Vec<usize>)>,
}

/// An **augmented graded `Z/2`-algebra**, given by its degree-wise basis and product. The minimal-resolution
/// engine (and hence the whole `Ext`/Adams machine) is generic over this — the Steenrod algebra is just one
/// instance. Point it at a different algebra and the same machine computes a different spectrum's Adams chart.
trait Algebra {
    /// The `Z/2`-basis of the algebra in internal degree `degree` (each element encoded as a monomial). For a
    /// finite algebra this is empty above the top degree.
    fn basis(&self, degree: usize) -> Vec<Vec<usize>>;
    /// The product `a · b`, reduced to the basis, as a `Z/2` support set. The unit is the empty monomial.
    fn multiply(&self, a: &[usize], b: &[usize]) -> std::collections::HashSet<Vec<usize>>;
    /// The **algebra generators** (indecomposables) of positive degree up to `max_degree`, as `(degree, key)`.
    /// Multiplying a kernel by just these spans `A⁺·ker` (the kernel is a submodule), so the resolution's
    /// decomposables loop needs only the generators — the key speed symmetry: a handful instead of the whole
    /// algebra per degree.
    fn generators(&self, max_degree: usize) -> Vec<(usize, Vec<usize>)>;
}

/// The full mod-2 Steenrod algebra `𝒜`: basis = admissible monomials, product = concatenate then Adem-reduce.
struct SteenrodAlgebra;
impl Algebra for SteenrodAlgebra {
    fn basis(&self, degree: usize) -> Vec<Vec<usize>> {
        admissibles_of_degree(degree)
    }
    fn multiply(&self, a: &[usize], b: &[usize]) -> std::collections::HashSet<Vec<usize>> {
        let mut prod = a.to_vec();
        prod.extend_from_slice(b);
        reduce_unit(&prod)
    }
    fn generators(&self, max_degree: usize) -> Vec<(usize, Vec<usize>)> {
        let mut out = Vec::new();
        let mut d = 1usize;
        while d <= max_degree {
            out.push((d, vec![d])); // Sq^{2ⁱ}, the indecomposables of 𝒜
            d <<= 1;
        }
        out
    }
}

/// A `Z/2`-element of the Steenrod algebra as the support set of admissible monomials it sums (e.g. the
/// `A(1)` element `Sq²Sq³ = Sq⁵ + Sq⁴Sq¹` is `{[5], [4,1]}`).
type Combo = std::collections::BTreeSet<Vec<usize>>;

/// `Z/2` product of two combinations: `(Σ mᵢ)(Σ nⱼ) = Σ adem_reduce(mᵢ·nⱼ)`.
fn combo_mul(a: &Combo, b: &Combo) -> Combo {
    let mut out = Combo::new();
    for ma in a {
        for mb in b {
            let mut prod = ma.clone();
            prod.extend_from_slice(mb);
            for r in reduce_unit(&prod) {
                if !out.insert(r.clone()) {
                    out.remove(&r);
                }
            }
        }
    }
    out
}

/// A per-degree map from a basis element's **pivot monomial** (its largest monomial) to its index — turns the
/// echelon reduction's inner lookup from an O(basis) scan into O(1).
type PivotIndex = std::collections::HashMap<Vec<usize>, usize>;

/// Express a combination in a degree's **echelon basis** (each basis element pivoted on its largest
/// monomial): returns the indices whose `Z/2` sum is `target`. `target` must lie in the span (always true for
/// products inside a subalgebra). Pivot lookups are O(1) via `pivots`.
fn express_in_basis(target: &Combo, basis: &[Combo], pivots: &PivotIndex) -> Vec<usize> {
    let mut r = target.clone();
    let mut used = Vec::new();
    while let Some(piv) = r.iter().next_back().cloned() {
        match pivots.get(&piv) {
            Some(&idx) => {
                used.push(idx);
                for m in &basis[idx] {
                    if !r.insert(m.clone()) {
                        r.remove(m);
                    }
                }
            }
            None => break, // not in span — only on a closure bug
        }
    }
    used.sort_unstable();
    used
}

/// A **finite subalgebra** of `𝒜`, given a `Z/2`-basis of *combinations* per degree (a subalgebra need not be
/// spanned by single admissibles — e.g. `Sq²Sq³ = Sq⁵+Sq⁴Sq¹ ∈ A(1)`). Basis elements are addressed by the
/// opaque key `[degree, index]`. `A(1) = ⟨Sq¹, Sq²⟩` is the headline instance — its `Ext` is the Adams `E₂`
/// for `ko` (connective real K-theory).
struct SubAlgebra {
    by_degree: Vec<Vec<Combo>>,
    pivots: Vec<PivotIndex>,
    gen_keys: Vec<(usize, Vec<usize>)>,
}

impl SubAlgebra {
    fn dimension(&self) -> usize {
        self.by_degree.iter().map(|v| v.len()).sum()
    }
}

/// Reduce a combination against a degree's echelon basis (pivot = largest monomial), using the O(1) pivot
/// index; returns the residual.
fn reduce_combo(basis: &[Combo], pivots: &PivotIndex, c: &Combo) -> Combo {
    let mut r = c.clone();
    while let Some(piv) = r.iter().next_back().cloned() {
        match pivots.get(&piv) {
            Some(&idx) => {
                for m in &basis[idx] {
                    if !r.insert(m.clone()) {
                        r.remove(m);
                    }
                }
            }
            None => break,
        }
    }
    r
}

/// Reduce `c` against its degree's basis; if it is a NEW independent direction, append the residual to the
/// echelon basis, register its pivot, and queue it. The heart of the fast build: only the basis is ever stored.
fn subalg_add(
    by_degree: &mut [Vec<Combo>],
    pivots: &mut [PivotIndex],
    worklist: &mut Vec<Combo>,
    max_degree: usize,
    c: Combo,
) {
    let d = match c.iter().next() {
        Some(m) => m.iter().sum::<usize>(),
        None => return,
    };
    if d > max_degree {
        return;
    }
    let r = reduce_combo(&by_degree[d], &pivots[d], &c);
    if let Some(piv) = r.iter().next_back().cloned() {
        pivots[d].insert(piv, by_degree[d].len());
        by_degree[d].push(r.clone());
        worklist.push(r);
    }
}

/// Build the subalgebra generated by `gens` (each a single square), by closing under the Adem product up to
/// `max_degree` and extracting an echelon basis of combinations per degree.
fn build_subalgebra(gens: &[Vec<usize>], max_degree: usize) -> SubAlgebra {
    let unit: Combo = std::iter::once(vec![]).collect();
    let gen_combos: Vec<Combo> = gens
        .iter()
        .map(|g| std::iter::once(g.clone()).collect())
        .collect();
    let mut by_degree: Vec<Vec<Combo>> = vec![Vec::new(); max_degree + 1];
    let mut pivots: Vec<PivotIndex> = vec![PivotIndex::new(); max_degree + 1];
    let mut worklist: Vec<Combo> = Vec::new();
    // Seed with the unit and the generators, then BFS: multiply each new basis element by the generators.
    // Every A-element is a word in the generators, so right-multiplying the basis by them reaches all of A —
    // O(dim · #gens) products with O(1) pivot lookups, instead of O(span²) re-scans.
    subalg_add(&mut by_degree, &mut pivots, &mut worklist, max_degree, unit);
    for g in &gen_combos {
        subalg_add(
            &mut by_degree,
            &mut pivots,
            &mut worklist,
            max_degree,
            g.clone(),
        );
    }
    while let Some(b) = worklist.pop() {
        for g in &gen_combos {
            let p = combo_mul(&b, g);
            subalg_add(&mut by_degree, &mut pivots, &mut worklist, max_degree, p);
        }
    }
    // Record the algebra-generator keys: each `Sq^{2ⁱ}` is the largest monomial in its degree, so it is its
    // own pivot and sits as a single basis element addressed by `[degree, index]`.
    let gen_keys: Vec<(usize, Vec<usize>)> = gens
        .iter()
        .map(|g| {
            let d: usize = g.iter().sum();
            (d, vec![d, pivots[d][g]])
        })
        .collect();
    SubAlgebra {
        by_degree,
        pivots,
        gen_keys,
    }
}

impl Algebra for SubAlgebra {
    fn basis(&self, degree: usize) -> Vec<Vec<usize>> {
        (0..self.by_degree.get(degree).map_or(0, |v| v.len()))
            .map(|i| vec![degree, i])
            .collect()
    }
    fn multiply(&self, a: &[usize], b: &[usize]) -> std::collections::HashSet<Vec<usize>> {
        let prod = combo_mul(&self.by_degree[a[0]][a[1]], &self.by_degree[b[0]][b[1]]);
        let pd = a[0] + b[0];
        match self.by_degree.get(pd) {
            Some(basis) if !prod.is_empty() => express_in_basis(&prod, basis, &self.pivots[pd])
                .into_iter()
                .map(|i| vec![pd, i])
                .collect(),
            _ => std::collections::HashSet::new(),
        }
    }
    fn generators(&self, max_degree: usize) -> Vec<(usize, Vec<usize>)> {
        self.gen_keys
            .iter()
            .filter(|(d, _)| *d <= max_degree)
            .cloned()
            .collect()
    }
}

/// Left-multiply a boundary by a monomial over an arbitrary algebra: `m · Σ(g', a) = Σ(g', m·a)` over `Z/2`.
fn act_boundary_over(
    alg: &dyn Algebra,
    m: &[usize],
    boundary: &[(usize, Vec<usize>)],
) -> std::collections::HashSet<(usize, Vec<usize>)> {
    let mut acc: std::collections::HashSet<(usize, Vec<usize>)> = std::collections::HashSet::new();
    for (g, a) in boundary {
        for ap in alg.multiply(m, a) {
            let key = (*g, ap);
            if !acc.insert(key.clone()) {
                acc.remove(&key);
            }
        }
    }
    acc
}

/// The degree-`t` basis of `C_s = ⊕ A·g` over an arbitrary algebra: `(generator index, basis element of
/// degree t−deg(g))`.
fn module_basis_over(alg: &dyn Algebra, gens: &[ResGen], t: usize) -> Vec<(usize, Vec<usize>)> {
    let mut out = Vec::new();
    for (gi, g) in gens.iter().enumerate() {
        if g.degree <= t {
            for m in alg.basis(t - g.degree) {
                out.push((gi, m));
            }
        }
    }
    out
}

/// Left-multiply a boundary by a Steenrod monomial: `m · Σ(g', a) = Σ(g', adem_reduce(m·a))`, over `Z/2`.
fn act_boundary(
    m: &[usize],
    boundary: &[(usize, Vec<usize>)],
) -> std::collections::HashSet<(usize, Vec<usize>)> {
    act_boundary_over(&SteenrodAlgebra, m, boundary)
}

/// The degree-`t` basis of `C_s = ⊕ 𝒜·g`: pairs `(generator index, admissible monomial of degree t−deg(g))`.
fn module_basis(gens: &[ResGen], t: usize) -> Vec<(usize, Vec<usize>)> {
    module_basis_over(&SteenrodAlgebra, gens, t)
}

/// Build the minimal free resolution of `Z/2` over the Steenrod algebra up to homological degree `max_s`
/// and internal degree `max_t`. `Ext^{s,t} = #(generators of C_s in degree t)`.
fn minimal_resolution(max_s: usize, max_t: usize) -> Vec<Vec<ResGen>> {
    minimal_resolution_over(&SteenrodAlgebra, max_s, max_t)
}

/// Build the minimal free resolution of `Z/2` over an **arbitrary augmented algebra** up to homological
/// degree `max_s` and internal degree `max_t`. `Ext^{s,t} = #(generators of C_s in degree t)`. The algorithm
/// is unchanged from the Steenrod case — only the basis and product come from `alg` — which is the whole
/// point: one resolution engine, every algebra.
fn minimal_resolution_over(alg: &dyn Algebra, max_s: usize, max_t: usize) -> Vec<Vec<ResGen>> {
    let mut res: Vec<Vec<ResGen>> = vec![vec![ResGen {
        degree: 0,
        boundary: vec![],
    }]]; // C_0 = A·g₀
    for s in 1..=max_s {
        let mut new_gens: Vec<ResGen> = Vec::new();
        for t in 1..=max_t {
            // Basis of C_{s-1}(t) and an index for it.
            let prev_basis = module_basis_over(alg, &res[s - 1], t);
            let prev_idx: std::collections::HashMap<(usize, Vec<usize>), usize> = prev_basis
                .iter()
                .cloned()
                .enumerate()
                .map(|(i, b)| (b, i))
                .collect();

            // Kernel of d_{s-1} : C_{s-1}(t) → C_{s-2}(t).  (For s=1, d_0 is the augmentation: every
            // positive-degree element is in the kernel, so ker = the whole basis.)
            let kernel: Vec<Bits> = if s == 1 {
                (0..prev_basis.len()).map(Bits::bit).collect()
            } else {
                let cc_basis = module_basis_over(alg, &res[s - 2], t);
                let cc_idx: std::collections::HashMap<(usize, Vec<usize>), usize> = cc_basis
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(i, b)| (b, i))
                    .collect();
                let cols: Vec<Bits> = prev_basis
                    .iter()
                    .map(|(gi, m)| {
                        let img = act_boundary_over(alg, m, &res[s - 1][*gi].boundary);
                        Bits::from_indices(img.into_iter().map(|e| cc_idx[&e]))
                    })
                    .collect();
                z2_nullspace(&cols)
            };

            // Decomposables A⁺·ker = span{ g · z : g a GENERATOR, z ∈ ker(d_{s-1})(t-deg g) }. Because ker is
            // an A-submodule, multiplying by the generators alone spans A⁺·ker — so we loop the handful of
            // generators (Sq^{2ⁱ}) instead of the whole algebra. This is the speed symmetry break.
            let mut decomp: Vec<Bits> = Vec::new();
            for (gdeg, gkey) in alg.generators(t - 1) {
                let lower = kernel_at_over(alg, &res, s - 1, t - gdeg);
                let lower_basis = module_basis_over(alg, &res[s - 1], t - gdeg);
                for z in &lower.0 {
                    let mut v = Bits::new();
                    for (b, (gi, m)) in lower_basis.iter().enumerate() {
                        if z.get(b) {
                            for mp in alg.multiply(&gkey, m) {
                                if let Some(&idx) = prev_idx.get(&(*gi, mp)) {
                                    v.toggle(idx);
                                }
                            }
                        }
                    }
                    if !v.is_zero() {
                        decomp.push(v);
                    }
                }
            }

            // New generators = kernel vectors independent modulo the decomposables.
            let mut pivots: std::collections::HashMap<usize, Bits> =
                std::collections::HashMap::new();
            for mut d in decomp {
                while let Some(lead) = d.lowest() {
                    match pivots.get(&lead) {
                        Some(p) => d ^= p,
                        None => {
                            pivots.insert(lead, d);
                            break;
                        }
                    }
                }
            }
            for kv in &kernel {
                let mut r = kv.clone();
                while let Some(lead) = r.lowest() {
                    match pivots.get(&lead) {
                        Some(p) => r ^= p,
                        None => break,
                    }
                }
                if let Some(lead) = r.lowest() {
                    pivots.insert(lead, r);
                    let boundary: Vec<(usize, Vec<usize>)> = (0..prev_basis.len())
                        .filter(|&b| kv.get(b))
                        .map(|b| prev_basis[b].clone())
                        .collect();
                    new_gens.push(ResGen {
                        degree: t,
                        boundary,
                    });
                }
            }
        }
        res.push(new_gens);
    }
    res
}

/// `(kernel vectors, _)` of `d_{s} : C_{s}(t) → C_{s-1}(t)` — small helper used during decomposable
/// computation. Returns kernel masks over `module_basis(C_s, t)`.
fn kernel_at(res: &[Vec<ResGen>], s: usize, t: usize) -> (Vec<Bits>, ()) {
    kernel_at_over(&SteenrodAlgebra, res, s, t)
}

/// `kernel_at` over an arbitrary algebra.
fn kernel_at_over(alg: &dyn Algebra, res: &[Vec<ResGen>], s: usize, t: usize) -> (Vec<Bits>, ()) {
    let basis = module_basis_over(alg, &res[s], t);
    if s == 0 {
        return ((0..basis.len()).map(Bits::bit).collect(), ());
    }
    let cc_basis = module_basis_over(alg, &res[s - 1], t);
    let cc_idx: std::collections::HashMap<(usize, Vec<usize>), usize> = cc_basis
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, b)| (b, i))
        .collect();
    let cols: Vec<Bits> = basis
        .iter()
        .map(|(gi, m)| {
            Bits::from_indices(
                act_boundary_over(alg, m, &res[s][*gi].boundary)
                    .into_iter()
                    .map(|e| cc_idx[&e]),
            )
        })
        .collect();
    (z2_nullspace(&cols), ())
}

/// `dim Ext^{s,t}_𝒜(Z/2, Z/2)` — the full Adams `E₂` page from the minimal resolution.
pub fn ext(max_s: usize, max_t: usize) -> Vec<Vec<usize>> {
    ext_over(&SteenrodAlgebra, max_s, max_t)
}

/// `dim Ext^{s,t}_A(Z/2, Z/2)` over an arbitrary algebra `A` — the Adams `E₂` page for whatever spectrum `A`
/// resolves. `A = 𝒜` gives the sphere; `A = A(1)` gives `ko`.
fn ext_over(alg: &dyn Algebra, max_s: usize, max_t: usize) -> Vec<Vec<usize>> {
    let res = minimal_resolution_over(alg, max_s, max_t);
    (0..=max_s)
        .map(|s| {
            (0..=max_t)
                .map(|t| res[s].iter().filter(|g| g.degree == t).count())
                .collect()
        })
        .collect()
}

/// `h₀`-multiplication read straight off the resolution boundaries: generator `g` at stage `s` is sent by
/// `h₀ = Sq¹` to the stage-`(s+1)` generators `g'` whose boundary contains `Sq¹·g` (the term `(gi, [1])`).
/// The symmetry is already in the resolution — no Yoneda lifting needed.
fn h0_targets(res: &[Vec<ResGen>], s: usize, gi: usize) -> Vec<usize> {
    if s + 1 >= res.len() {
        return vec![];
    }
    res[s + 1]
        .iter()
        .enumerate()
        .filter(|(_, g)| {
            g.boundary
                .iter()
                .any(|(j, m)| *j == gi && m.as_slice() == [1])
        })
        .map(|(idx, _)| idx)
        .collect()
}

/// The EXACT 2-local stable group of stem `n`, as the sorted multiset of `h₀`-tower lengths (a tower of
/// length `L` is a summand `Z/2^L`). Read from the resolution's `h₀`-structure — the symmetry a dim count
/// ignores. `[3] = Z/8`, `[1,1] = (Z/2)²`, etc.
fn stem_2local_tower_lengths(res: &[Vec<ResGen>], n: usize) -> Vec<usize> {
    let in_stem =
        |s: usize, gi: usize| s < res.len() && gi < res[s].len() && res[s][gi].degree == n + s;
    // every (s+1,t) that is an h₀-image of a stem-n generator
    let mut is_target: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for s in 0..res.len() {
        for gi in 0..res[s].len() {
            if in_stem(s, gi) {
                for t in h0_targets(res, s, gi) {
                    if in_stem(s + 1, t) {
                        is_target.insert((s + 1, t));
                    }
                }
            }
        }
    }
    let mut lengths = Vec::new();
    for s in 0..res.len() {
        for gi in 0..res[s].len() {
            if in_stem(s, gi) && !is_target.contains(&(s, gi)) {
                // tower bottom: walk h₀ upward, counting the height
                let mut len = 1;
                let (mut cs, mut cgi) = (s, gi);
                loop {
                    let ups: Vec<usize> = h0_targets(res, cs, cgi)
                        .into_iter()
                        .filter(|&t| in_stem(cs + 1, t))
                        .collect();
                    if let Some(&t) = ups.first() {
                        len += 1;
                        cs += 1;
                        cgi = t;
                    } else {
                        break;
                    }
                }
                lengths.push(len);
            }
        }
    }
    lengths.sort_unstable();
    lengths
}

/// The action of a Steenrod monomial on a class in `H*(BZ/2) = Z/2[x]` (apply the squares right-to-left).
pub fn act_monomial(m: &[usize], poly: &[u8]) -> Vec<u8> {
    let mut p = poly.to_vec();
    for &i in m.iter().rev() {
        p = apply_sq(i, &p);
    }
    trim(p)
}

/// Adem-reduce a monomial to the admissible basis, but keep the **unit** `Sq⁰ = 1` (the empty monomial) as
/// `[]` rather than dropping it — `adem_reduce` treats `[]` as zero, which is wrong inside the algebra's
/// product/coproduct where `1` must survive.
fn reduce_unit(m: &[usize]) -> std::collections::HashSet<Vec<usize>> {
    let filtered: Vec<usize> = m.iter().copied().filter(|&i| i > 0).collect();
    if filtered.is_empty() {
        return std::iter::once(vec![]).collect();
    }
    adem_reduce(&filtered)
}

/// The **coproduct** on the Steenrod algebra: `ψ(Sqⁿ) = Σ_{a+b=n} Sqᵃ ⊗ Sqᵇ` (the Cartan diagonal),
/// extended as an algebra map `ψ(xy) = ψ(x)ψ(y)` with `(a⊗b)(c⊗d) = ac⊗bd`, each tensor leg reduced to the
/// admissible basis. Returns the `Z/2` support as a set of `(admissible, admissible)` pairs. This is the
/// structure that makes `C ⊗ C` an 𝒜-module and powers the chain-level diagonal.
fn coproduct(m: &[usize]) -> std::collections::HashSet<(Vec<usize>, Vec<usize>)> {
    let mut acc: std::collections::HashSet<(Vec<usize>, Vec<usize>)> =
        std::iter::once((vec![], vec![])).collect(); // ψ(1) = 1⊗1
    for &n in m {
        let mut next: std::collections::HashSet<(Vec<usize>, Vec<usize>)> =
            std::collections::HashSet::new();
        for (lacc, racc) in &acc {
            for a in 0..=n {
                let (mut ll, mut rr) = (lacc.clone(), racc.clone());
                if a > 0 {
                    ll.push(a);
                }
                if n - a > 0 {
                    rr.push(n - a);
                }
                for la in reduce_unit(&ll) {
                    for ra in reduce_unit(&rr) {
                        let key = (la.clone(), ra);
                        if !next.insert(key.clone()) {
                            next.remove(&key);
                        }
                    }
                }
            }
        }
        acc = next;
    }
    acc
}

/// A chain element of `C_s = ⊕ 𝒜·g`: its `Z/2` support, a set of terms `(generator index, admissible
/// Steenrod monomial)` standing for `Σ m·g`.
type Chain = std::collections::HashSet<(usize, Vec<usize>)>;

/// `Z/2`-add `b` into `a` (re-insertion cancels).
fn chain_add(a: &mut Chain, b: Chain) {
    for e in b {
        if !a.insert(e.clone()) {
            a.remove(&e);
        }
    }
}

/// Left-act a Steenrod monomial on a chain element: `m·Σ(g,n) = Σ(g, adem_reduce(m·n))`.
fn act_chain(m: &[usize], elem: &Chain) -> Chain {
    let v: Vec<(usize, Vec<usize>)> = elem.iter().cloned().collect();
    act_boundary(m, &v)
}

/// Solve `∂x = target` for a chain element `x ∈ (C_s)_deg`, given `target ∈ (C_{s-1})_deg`. The minimal
/// resolution is exact, so `target ∈ im ∂` whenever it is a cycle; we pick one preimage by `Z/2` Gaussian
/// elimination — a chosen contracting homotopy. `None` only if `target ∉ im ∂` (a resolution-exactness bug,
/// surfaced rather than papered over).
fn solve_boundary(res: &[Vec<ResGen>], s: usize, deg: usize, target: &Chain) -> Option<Chain> {
    let src_basis = module_basis(&res[s], deg);
    let tgt_basis = module_basis(&res[s - 1], deg);
    let tgt_idx: std::collections::HashMap<(usize, Vec<usize>), usize> = tgt_basis
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, b)| (b, i))
        .collect();
    let mask = |c: &Chain| -> Bits { Bits::from_indices(c.iter().map(|e| tgt_idx[e])) };
    let cols: Vec<Bits> = src_basis
        .iter()
        .map(|(gi, m)| mask(&act_boundary(m, &res[s][*gi].boundary)))
        .collect();
    // Row-reduce the columns, tracking which source columns XOR to each pivot row.
    let mut elim: Vec<(usize, Bits, Bits)> = Vec::new(); // (pivot bit, reduced column, source combination)
    for (j, c) in cols.iter().enumerate() {
        let (mut r, mut src) = (c.clone(), Bits::bit(j));
        for (piv, rc, sm) in &elim {
            if r.get(*piv) {
                r ^= rc;
                src ^= sm;
            }
        }
        if let Some(lead) = r.lowest() {
            elim.push((lead, r, src));
        }
    }
    let (mut tr, mut tsrc) = (mask(target), Bits::new());
    for (piv, rc, sm) in &elim {
        if tr.get(*piv) {
            tr ^= rc;
            tsrc ^= sm;
        }
    }
    if !tr.is_zero() {
        return None;
    }
    Some(
        (0..src_basis.len())
            .filter(|&j| tsrc.get(j))
            .map(|j| src_basis[j].clone())
            .collect(),
    )
}

/// The **Yoneda product** `Ext^{a_s,t_a} ⊗ Ext^{b_s,t_b} → Ext^{a_s+b_s, t_a+t_b}` on the cohomology of the
/// Steenrod algebra, computed from the resolution itself. The cocycle dual to generator `(a_s, a_gi)` is
/// lifted to a comparison chain map `φ_•: C_{a_s+•} → C_•` (`ε·φ_0 = dual`, `∂φ_k = φ_{k-1}∂`, each step a
/// `Z/2` boundary solve), and the product is `dual_b ∘ φ_{b_s}` — read off as the `(b_gi, identity)`
/// coefficient. The comparison map is the diagonal `X → X×X` dualized onto the resolution: the geometric
/// symmetry made algebra. Returns the product's `Z/2` support among the `C_{a_s+b_s}` generators.
pub fn yoneda_product(
    res: &[Vec<ResGen>],
    a_s: usize,
    a_gi: usize,
    b_s: usize,
    b_gi: usize,
) -> Vec<usize> {
    let t_a = res[a_s][a_gi].degree;
    let t_b = res[b_s][b_gi].degree;
    // φ_0 : C_{a_s} → C_0, the cocycle dual to generator a: g_{a_s,i} ↦ δ_{i,a}·g₀.
    let mut phi: Vec<std::collections::HashMap<usize, Chain>> = Vec::new();
    let mut p0 = std::collections::HashMap::new();
    for i in 0..res[a_s].len() {
        let mut c = Chain::new();
        if i == a_gi {
            c.insert((0, vec![]));
        }
        p0.insert(i, c);
    }
    phi.push(p0);
    // Lift along the resolution up to φ_{b_s}.
    for k in 1..=b_s {
        let mut pk: std::collections::HashMap<usize, Chain> = std::collections::HashMap::new();
        for i in 0..res[a_s + k].len() {
            let d_i = res[a_s + k][i].degree;
            if d_i < t_a {
                pk.insert(i, Chain::new()); // below the cocycle's degree φ vanishes; avoids a degree underflow
                continue;
            }
            // r = φ_{k-1}(∂ g_{a_s+k, i})  ∈ C_{k-1}
            let mut r = Chain::new();
            for (j, m) in &res[a_s + k][i].boundary {
                chain_add(&mut r, act_chain(m, &phi[k - 1][j]));
            }
            let x = if r.is_empty() {
                Chain::new()
            } else {
                solve_boundary(res, k, d_i - t_a, &r)
                    .expect("minimal resolution is exact: ∂x = r is solvable")
            };
            pk.insert(i, x);
        }
        phi.push(pk);
    }
    // Pair with dual_b: the (b_gi, identity) coefficient of φ_{b_s}(g) over the (t_a+t_b)-generators.
    (0..res[a_s + b_s].len())
        .filter(|&i| {
            res[a_s + b_s][i].degree == t_a + t_b && phi[b_s][&i].contains(&(b_gi, vec![]))
        })
        .collect()
}

/// A basis term of `C ⊗ C`: `(p, i, m1, q, j, m2)` standing for `(m1·g_{p,i}) ⊗ (m2·g_{q,j})`, with `p`/`q`
/// the homological degrees of the two legs. An element is the `Z/2` support set of such terms.
type TensorTerm = (usize, usize, Vec<usize>, usize, usize, Vec<usize>);
type Tensor = std::collections::HashSet<TensorTerm>;

/// `Z/2`-add `b` into `a`.
fn tensor_add(a: &mut Tensor, b: Tensor) {
    for e in b {
        if !a.insert(e.clone()) {
            a.remove(&e);
        }
    }
}

/// Internal degree of a homogeneous tensor element.
fn tensor_degree(res: &[Vec<ResGen>], t: &Tensor) -> Option<usize> {
    t.iter().next().map(|(p, i, m1, q, j, m2)| {
        res[*p][*i].degree
            + m1.iter().sum::<usize>()
            + res[*q][*j].degree
            + m2.iter().sum::<usize>()
    })
}

/// The boundary on `C ⊗ C`: `∂(a⊗b) = ∂a⊗b + a⊗∂b` (over `Z/2` the Koszul signs vanish).
fn tensor_boundary(res: &[Vec<ResGen>], elem: &Tensor) -> Tensor {
    let mut out = Tensor::new();
    for (p, i, m1, q, j, m2) in elem {
        if *p >= 1 {
            for (i2, m1b) in act_boundary(m1, &res[*p][*i].boundary) {
                let term = (*p - 1, i2, m1b, *q, *j, m2.clone());
                if !out.insert(term.clone()) {
                    out.remove(&term);
                }
            }
        }
        if *q >= 1 {
            for (j2, m2b) in act_boundary(m2, &res[*q][*j].boundary) {
                let term = (*p, *i, m1.clone(), *q - 1, j2, m2b);
                if !out.insert(term.clone()) {
                    out.remove(&term);
                }
            }
        }
    }
    out
}

/// The diagonal 𝒜-action on `C ⊗ C`: `m·(a⊗b) = Σ_{ψ(m)=Σ m'⊗m''} (m'·a) ⊗ (m''·b)`. This is what makes
/// `C ⊗ C` a Steenrod-module and lets the diagonal be lifted 𝒜-linearly.
fn act_tensor(m: &[usize], elem: &Tensor) -> Tensor {
    let psi = coproduct(m);
    let mut out = Tensor::new();
    for (p, i, m1, q, j, m2) in elem {
        for (ml, mr) in &psi {
            let (mut left, mut right) = (ml.clone(), mr.clone());
            left.extend_from_slice(m1);
            right.extend_from_slice(m2);
            for a in reduce_unit(&left) {
                for b in reduce_unit(&right) {
                    let term = (*p, *i, a.clone(), *q, *j, b);
                    if !out.insert(term.clone()) {
                        out.remove(&term);
                    }
                }
            }
        }
    }
    out
}

/// The `Z/2`-basis of `(C ⊗ C)_n` in internal degree `t` — every leg-split `p+q=n`, `t_l+t_r=t`.
fn tensor_basis(res: &[Vec<ResGen>], n: usize, t: usize) -> Vec<TensorTerm> {
    let mut out = Vec::new();
    for p in 0..=n {
        let q = n - p;
        if p >= res.len() || q >= res.len() {
            continue;
        }
        for tl in 0..=t {
            let tr = t - tl;
            for (i, m1) in module_basis(&res[p], tl) {
                for (j, m2) in module_basis(&res[q], tr) {
                    out.push((p, i, m1.clone(), q, j, m2.clone()));
                }
            }
        }
    }
    out
}

/// Solve `∂_{C⊗C} x = target` for `x ∈ (C⊗C)_n` — cap-free `Z/2` Gaussian elimination over tensor basis
/// elements (pivot = the largest term), tracking which source basis elements combine. `None` only if
/// `target ∉ im ∂` (a coherence bug, surfaced not hidden).
fn solve_tensor_boundary(res: &[Vec<ResGen>], n: usize, target: &Tensor) -> Option<Tensor> {
    fn reduce(
        mut col: Tensor,
        mut src: Tensor,
        elim: &[(TensorTerm, Tensor, Tensor)],
    ) -> (Tensor, Tensor) {
        loop {
            let piv = match col.iter().max() {
                Some(p) => p.clone(),
                None => return (col, src),
            };
            match elim.iter().find(|(p, _, _)| *p == piv) {
                Some((_, rc, sm)) => {
                    for x in rc {
                        if !col.insert(x.clone()) {
                            col.remove(x);
                        }
                    }
                    for x in sm {
                        if !src.insert(x.clone()) {
                            src.remove(x);
                        }
                    }
                }
                None => return (col, src),
            }
        }
    }
    let t = match tensor_degree(res, target) {
        Some(t) => t,
        None => return Some(Tensor::new()),
    };
    let mut elim: Vec<(TensorTerm, Tensor, Tensor)> = Vec::new(); // (pivot, reduced column, source combination)
    for b in tensor_basis(res, n, t) {
        let col = tensor_boundary(res, &std::iter::once(b.clone()).collect());
        let src: Tensor = std::iter::once(b).collect();
        let (rc, sm) = reduce(col, src, &elim);
        if let Some(piv) = rc.iter().max().cloned() {
            elim.push((piv, rc, sm));
        }
    }
    let (rc, sm) = reduce(target.clone(), Tensor::new(), &elim);
    rc.is_empty().then_some(sm)
}

/// The chain-level **diagonal** `Δ₀: C → C ⊗ C` — an 𝒜-linear chain map with `Δ₀(g₀) = g₀ ⊗ g₀`, built by
/// lifting up the resolution (`∂_{C⊗C} Δ₀(g) = Δ₀(∂g)`, solved leg-aware). It is the topological diagonal
/// `X → X×X` transported onto the algebraic resolution; its cup-`i` refinements carry the Steenrod
/// operations on Ext. `delta[n][&k] = Δ₀(g_{n,k})`.
fn diagonal(res: &[Vec<ResGen>], max_n: usize) -> Vec<std::collections::HashMap<usize, Tensor>> {
    let mut delta: Vec<std::collections::HashMap<usize, Tensor>> = Vec::new();
    let mut d0 = std::collections::HashMap::new();
    d0.insert(
        0usize,
        std::iter::once((0, 0, vec![], 0, 0, vec![])).collect::<Tensor>(),
    );
    delta.push(d0);
    for n in 1..=max_n {
        let mut dn: std::collections::HashMap<usize, Tensor> = std::collections::HashMap::new();
        for k in 0..res[n].len() {
            let mut rhs = Tensor::new(); // Δ₀(∂ g_{n,k}) = Σ_{(j,mb)∈∂} mb·Δ₀(g_{n-1,j})
            for (j, mb) in &res[n][k].boundary {
                tensor_add(&mut rhs, act_tensor(mb, &delta[n - 1][j]));
            }
            let x = solve_tensor_boundary(res, n, &rhs)
                .expect("diagonal lifts: ∂x = Δ₀(∂g) is solvable");
            dn.insert(k, x);
        }
        delta.push(dn);
    }
    delta
}

/// The leg-swap `T` on `C ⊗ C`: `(a⊗b) ↦ (b⊗a)`.
fn tensor_swap(t: &Tensor) -> Tensor {
    t.iter()
        .map(|(p, i, m1, q, j, m2)| (*q, *j, m2.clone(), *p, *i, m1.clone()))
        .collect()
}

/// The next **cup-`i` coherence homotopy** from the previous one: `Δ_i: C_n → (C⊗C)_{n+i}` satisfying
/// `∂Δ_i + Δ_i∂ = Δ_{i-1} + TΔ_{i-1}`, with `Δ_i(g₀) = 0`. `shift = i` is the homological shift `Δ_i`
/// carries. This single recursion is the whole `E_∞`/`H_∞` ladder: `Δ₀` (the diagonal, `shift 0`) gives the
/// cup product, `Δ₁` (`shift 1`) the `Sq⁰` doubling, `Δ₂` (`shift 2`) the doubling of Ext² classes, and so
/// on up — each measuring the failure of the previous to be symmetric.
fn cup_homotopy(
    res: &[Vec<ResGen>],
    max_n: usize,
    shift: usize,
    prev: &[std::collections::HashMap<usize, Tensor>],
) -> Vec<std::collections::HashMap<usize, Tensor>> {
    let mut d: Vec<std::collections::HashMap<usize, Tensor>> = Vec::new();
    d.push(std::iter::once((0usize, Tensor::new())).collect()); // Δ_i(g₀) = 0
    for n in 1..=max_n {
        let mut dn: std::collections::HashMap<usize, Tensor> = std::collections::HashMap::new();
        for k in 0..res[n].len() {
            let mut rhs = prev[n][&k].clone(); // Δ_{i-1}(g) + TΔ_{i-1}(g) + Δ_i(∂g)
            tensor_add(&mut rhs, tensor_swap(&prev[n][&k]));
            for (j, mb) in &res[n][k].boundary {
                tensor_add(&mut rhs, act_tensor(mb, &d[n - 1][j]));
            }
            let x = solve_tensor_boundary(res, n + shift, &rhs)
                .expect("cup-i homotopy lifts: ∂x = Δ+TΔ+Δ∂");
            dn.insert(k, x);
        }
        d.push(dn);
    }
    d
}

/// The cup-1 homotopy `Δ₁` — the first coherence homotopy of cocommutativity. Pairing a class with itself
/// through `Δ₁` is the `Sq⁰` doubling operation on Ext.
fn diagonal_1(
    res: &[Vec<ResGen>],
    max_n: usize,
    delta: &[std::collections::HashMap<usize, Tensor>],
) -> Vec<std::collections::HashMap<usize, Tensor>> {
    cup_homotopy(res, max_n, 1, delta)
}

/// The algebraic **`Sq⁰` (doubling) operation** on the one-line classes of Ext, computed from the cup-1
/// homotopy: `Sq⁰(h_i) = h_i ⌣_1 h_i = (dual ⊗ dual) ∘ Δ₁` on `C_1`. Returns its `Z/2` support among the
/// degree-`2^{i+1}` generators of `C_1`. The Adams doubling `Sq⁰(h_i) = h_{i+1}` is then a derived fact.
fn sq0_on_h(
    res: &[Vec<ResGen>],
    delta1: &[std::collections::HashMap<usize, Tensor>],
    gi: usize,
) -> Vec<usize> {
    let want = 2 * res[1][gi].degree;
    (0..res[1].len())
        .filter(|&k| {
            res[1][k].degree == want && delta1[1][&k].contains(&(1, gi, vec![], 1, gi, vec![]))
        })
        .collect()
}

/// `Sq⁰` on a single Ext² generator class `gx` (e.g. `h_i²`), via the cup-2 homotopy: `Sq⁰(x) = x ⌣_2 x =
/// (dual ⊗ dual) ∘ Δ₂` on `C_2`, read as the `((2,gx,1),(2,gx,1))` coefficient over the degree-`2t`
/// generators of `C_2`. Used to show the doubling is a ring endomorphism (`Sq⁰(h_i²) = h_{i+1}²`).
fn sq0_on_ext2(
    res: &[Vec<ResGen>],
    delta2: &[std::collections::HashMap<usize, Tensor>],
    gx: usize,
) -> Vec<usize> {
    let want = 2 * res[2][gx].degree;
    (0..res[2].len())
        .filter(|&k| {
            res[2][k].degree == want && delta2[2][&k].contains(&(2, gx, vec![], 2, gx, vec![]))
        })
        .collect()
}

/// The whole tower of cup-`i` diagonals `[Δ₀, Δ₁, …, Δ_max_shift]` built off one resolution, each lifted up
/// to homological degree `max_n`. `Δ₀` is the diagonal; `Δ_i = cup_homotopy(Δ_{i-1})`.
fn cup_diagonals(
    res: &[Vec<ResGen>],
    max_shift: usize,
    max_n: usize,
) -> Vec<Vec<std::collections::HashMap<usize, Tensor>>> {
    let mut out = vec![diagonal(res, max_n)];
    for i in 1..=max_shift {
        let prev = out[i - 1].clone();
        out.push(cup_homotopy(res, max_n, i, &prev));
    }
    out
}

/// The algebraic **`Sq⁰` doubling** applied to *any* Ext class — a single generator `gx ∈ Ext^{s,t}` — via
/// the cup-`s` homotopy: `Sq⁰(x) = x ⌣_s x = (dual ⊗ dual) ∘ Δ_s` on `C_s`, read as the
/// `((s,gx,1),(s,gx,1))` coefficient over the degree-`2t` generators of `C_s`. One uniform operator across
/// the whole chart; `deltas[i]` must be `Δ_i`. This is the automation of the doubling symmetry: feed it any
/// class, harvest `Sq⁰` of it.
fn sq0(
    res: &[Vec<ResGen>],
    deltas: &[Vec<std::collections::HashMap<usize, Tensor>>],
    s: usize,
    gx: usize,
) -> Vec<usize> {
    let want = 2 * res[s][gx].degree;
    let ds = &deltas[s]; // Δ_s
    (0..res[s].len())
        .filter(|&k| {
            res[s][k].degree == want && ds[s][&k].contains(&(s, gx, vec![], s, gx, vec![]))
        })
        .collect()
}

/// One machine-derived fact about the Adams `E₂` chart of the sphere. Every variant is *computed* from the
/// minimal resolution and its chain-level diagonal — none is asserted.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum AdamsFact {
    /// `dim Ext^{s,t}_𝒜(Z/2, Z/2) = dim` — a point of the chart.
    Dimension { s: usize, t: usize, dim: usize },
    /// `lhs · rhs = result` in the cohomology ring of the Steenrod algebra (a named product).
    Product {
        lhs: String,
        rhs: String,
        result: String,
    },
    /// `lhs · rhs = 0` — an Adem-adjacency / ring vanishing.
    Vanishing { lhs: String, rhs: String },
    /// `a = b`: two product-paths landed on the same generator. The ring relation, discovered by collision.
    Relation { a: String, b: String },
    /// `Sq⁰(from) = to` — the algebraic doubling operation on the chart.
    Doubling { from: String, to: String },
    /// `π_n^s ⊗ Z₍₂₎ = ⊕ Z/2^Lᵢ` — the exact 2-local stable stem, as the `h₀`-tower-length multiset. When
    /// `truncated`, an `h₀`-tower runs into the resolution's filtration ceiling, so the group is only a lower
    /// bound (e.g. `π_0 = Z₍₂₎`, an infinite tower the finite resolution cannot close).
    StableGroup {
        stem: usize,
        tower_lengths: Vec<usize>,
        truncated: bool,
    },
    /// `Ext^{s,t}` carries a class that is **not a product of the indecomposables `h_i`** — the product-sweep
    /// cannot name it. It is the shadow of an Adams differential / a Massey product: the first place the
    /// primary (auto-able) structure runs out, and the secondary structure begins.
    Secondary { s: usize, t: usize },
}

impl std::fmt::Display for AdamsFact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdamsFact::Dimension { s, t, dim } => write!(f, "dim Ext^{{{s},{t}}} = {dim}"),
            AdamsFact::Product { lhs, rhs, result } => write!(f, "{lhs} · {rhs} = {result}"),
            AdamsFact::Vanishing { lhs, rhs } => write!(f, "{lhs} · {rhs} = 0"),
            AdamsFact::Relation { a, b } => write!(f, "{a} = {b}"),
            AdamsFact::Doubling { from, to } => write!(f, "Sq⁰({from}) = {to}"),
            AdamsFact::StableGroup {
                stem,
                tower_lengths,
                truncated,
            } => {
                let group = if tower_lengths.is_empty() {
                    "0".to_string()
                } else {
                    tower_lengths
                        .iter()
                        .map(|l| format!("Z/2^{l}"))
                        .collect::<Vec<_>>()
                        .join(" ⊕ ")
                };
                if *truncated {
                    write!(f, "π_{stem}^s ⊗ Z₍₂₎ ⊇ {group} (h₀-tower hits the resolution ceiling — infinite/truncated)")
                } else {
                    write!(f, "π_{stem}^s ⊗ Z₍₂₎ = {group}")
                }
            }
            AdamsFact::Secondary { s, t } => {
                write!(f, "Ext^{{{s},{t}}} has a non-product (secondary/Massey) class — un-auto-able by the product sweep")
            }
        }
    }
}

/// Name every Ext class reachable as a **product of the indecomposables `h_i`**, by multiplying named
/// classes on the right by the `h_i` to a fixpoint. Returns the name map keyed by `(s, generator index)`
/// together with the product / vanishing / relation facts discovered along the way (a relation surfaces
/// whenever two product-paths collide on one generator). Pure ring structure — no chain-level diagonal
/// needed, so it is cheap and runs to high internal degree.
fn name_by_products(
    res: &[Vec<ResGen>],
    max_s: usize,
    max_t: usize,
) -> (
    std::collections::HashMap<(usize, usize), String>,
    Vec<AdamsFact>,
    Vec<(usize, usize)>,
) {
    let mut names: std::collections::HashMap<(usize, usize), String> =
        std::collections::HashMap::new();
    let mut facts: Vec<AdamsFact> = Vec::new();
    let mut secondary: Vec<(usize, usize)> = Vec::new();
    let mut worklist: Vec<(usize, usize)> = Vec::new();
    for gi in 0..res[1].len() {
        let i = res[1][gi].degree.trailing_zeros();
        names.insert((1, gi), format!("h{i}"));
        worklist.push((1, gi));
    }
    worklist.sort_by_key(|&(_, gi)| res[1][gi].degree);
    let h_gens: Vec<usize> = (0..res[1].len()).collect();

    // Drain the worklist (multiply each named class by the indecomposables); when it empties, name the
    // lowest un-named generator as a fresh **secondary generator** `c_k` and keep sweeping. This is how the
    // engine "breaks to the next group": it does not prove what `c_k` is, it recognises a detected
    // indecomposable and lets the verified product symmetry carry on from it. A product landing on another
    // detected secondary class is self-consistent validation — no Massey computation required.
    let mut qi = 0;
    let mut sec = 0;
    loop {
        while qi < worklist.len() {
            let (s, gi) = worklist[qi];
            qi += 1;
            let lname = names[&(s, gi)].clone();
            for &hgi in &h_gens {
                if s + 1 > max_s || res[s][gi].degree + res[1][hgi].degree > max_t {
                    continue;
                }
                let hname = names[&(1, hgi)].clone();
                let prod = yoneda_product(res, s, gi, 1, hgi);
                if prod.is_empty() {
                    facts.push(AdamsFact::Vanishing {
                        lhs: lname.clone(),
                        rhs: hname,
                    });
                } else if prod.len() == 1 {
                    let key = (s + 1, prod[0]);
                    let newname = format!("{lname}{hname}");
                    match names.get(&key) {
                        Some(existing) if *existing != newname => {
                            let (a, b) = if newname < *existing {
                                (newname, existing.clone())
                            } else {
                                (existing.clone(), newname)
                            };
                            facts.push(AdamsFact::Relation { a, b });
                        }
                        Some(_) => {}
                        None => {
                            names.insert(key, newname.clone());
                            worklist.push(key);
                            facts.push(AdamsFact::Product {
                                lhs: lname.clone(),
                                rhs: hname,
                                result: newname,
                            });
                        }
                    }
                }
            }
        }
        let next = (1..=max_s)
            .flat_map(|s| (0..res[s].len()).map(move |gi| (s, gi)))
            .filter(|&(s, gi)| res[s][gi].degree <= max_t && !names.contains_key(&(s, gi)))
            .min_by_key(|&(s, gi)| (res[s][gi].degree, s));
        match next {
            Some((s, gi)) => {
                names.insert((s, gi), format!("c{sec}"));
                sec += 1;
                secondary.push((s, gi));
                worklist.push((s, gi));
            }
            None => break,
        }
    }
    (names, facts, secondary)
}

/// The **un-auto-able classes**: every `(s, t)` in range carrying an Ext generator that the product-sweep
/// cannot name — i.e. not a product of the indecomposables. These are exactly where the secondary (Massey)
/// structure / the Adams differentials live. Cheap (product structure only); the first one is `c₀` at
/// `Ext^{3,11}`, stem 8.
fn secondary_classes(max_s: usize, max_t: usize) -> Vec<(usize, usize)> {
    let res = minimal_resolution(max_s, max_t);
    let (_names, _facts, secondary) = name_by_products(&res, max_s, max_t);
    let mut out: Vec<(usize, usize)> = secondary
        .iter()
        .map(|&(s, gi)| (s, res[s][gi].degree))
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Auto-collect the **primary chart** from a resolution by pure symmetry-breaking — no proving, no
/// chain-level diagonal, so it is cheap and runs to a wide range. Gathers chart dimensions, ring products
/// and the relations that surface by collision, the un-auto-able (secondary/Massey) generators the product
/// sweep cannot name, and the exact 2-local stable stems from the `h₀`-towers. The sorted/deduped catalog.
pub fn harvest_secondary_facts(max_s: usize, max_t: usize) -> Vec<AdamsFact> {
    let res = minimal_resolution(max_s, max_t);
    let (_names, ring_facts, secondary) = name_by_products(&res, max_s, max_t);
    let mut facts = ring_facts;
    for s in 0..=max_s {
        for t in 0..=max_t {
            let dim = res[s].iter().filter(|g| g.degree == t).count();
            if dim > 0 {
                facts.push(AdamsFact::Dimension { s, t, dim });
            }
        }
    }
    for &(s, gi) in &secondary {
        facts.push(AdamsFact::Secondary {
            s,
            t: res[s][gi].degree,
        });
    }
    for stem in 0..=max_t.saturating_sub(max_s) {
        let lengths = stem_2local_tower_lengths(&res, stem);
        if !lengths.is_empty() {
            let truncated = res[max_s].iter().any(|g| g.degree == stem + max_s);
            facts.push(AdamsFact::StableGroup {
                stem,
                tower_lengths: lengths,
                truncated,
            });
        }
    }
    facts.sort();
    facts.dedup();
    facts
}

/// The **self-igniting fact engine**: from a single minimal resolution it auto-collects everything the
/// symmetry breaking nets — the chart dimensions, the ring products (multiplying named classes by the
/// indecomposables to a fixpoint, so ring *relations* surface automatically when two product-paths collide
/// on one generator), the `Sq⁰` doubling across the chart, and the exact 2-local stable stems from the
/// `h₀`-towers. Returns the deduplicated, sorted catalog. Bounded by `(max_s, max_t)`; the doubling reaches
/// through filtration 2 (cup-homotopies `Δ₀..Δ₂`).
pub fn harvest_adams_facts(max_s: usize, max_t: usize) -> Vec<AdamsFact> {
    let res = minimal_resolution(max_s, max_t);
    let dbl_shift = max_s.min(2);
    let deltas = cup_diagonals(&res, dbl_shift, dbl_shift);
    let mut facts: Vec<AdamsFact> = Vec::new();

    // 1. Chart dimensions, straight off the resolution.
    for s in 0..=max_s {
        for t in 0..=max_t {
            let dim = res[s].iter().filter(|g| g.degree == t).count();
            if dim > 0 {
                facts.push(AdamsFact::Dimension { s, t, dim });
            }
        }
    }

    // 2. Name every class reachable as a product of the indecomposables (fixpoint sweep); relations surface
    //    by collision. Then flag every Ext generator the sweep CANNOT name — the secondary/Massey classes.
    let (names, ring_facts, secondary) = name_by_products(&res, max_s, max_t);
    facts.extend(ring_facts);
    for &(s, gi) in &secondary {
        facts.push(AdamsFact::Secondary {
            s,
            t: res[s][gi].degree,
        });
    }

    // 3. The Sq⁰ doubling on every named class (through filtration `dbl_shift`).
    let mut named: Vec<((usize, usize), String)> =
        names.iter().map(|(k, v)| (*k, v.clone())).collect();
    named.sort();
    for ((s, gi), name) in &named {
        if *s == 0 || *s > dbl_shift || 2 * res[*s][*gi].degree > max_t {
            continue;
        }
        let img = sq0(&res, &deltas, *s, *gi);
        if img.len() == 1 {
            let to = names
                .get(&(*s, img[0]))
                .cloned()
                .unwrap_or_else(|| format!("Sq⁰({name})"));
            facts.push(AdamsFact::Doubling {
                from: name.clone(),
                to,
            });
        }
    }

    // 4. The exact 2-local stable stems from the h₀-tower filtration. A stem whose chart reaches the
    //    resolution's top filtration is flagged truncated — its h₀-tower may continue past what we computed
    //    (this is exactly π_0 = Z₍₂₎, an infinite tower the finite resolution cannot close honestly).
    for stem in 0..=max_t.saturating_sub(max_s) {
        let lengths = stem_2local_tower_lengths(&res, stem);
        if !lengths.is_empty() {
            let truncated = res[max_s].iter().any(|g| g.degree == stem + max_s);
            facts.push(AdamsFact::StableGroup {
                stem,
                tower_lengths: lengths,
                truncated,
            });
        }
    }

    facts.sort();
    facts.dedup();
    facts
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/steenrod/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/steenrod/adams_differential_onset.rs"]
mod adams_differential_onset;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/steenrod/ext_ring_from_the_resolution.rs"]
mod ext_ring_from_the_resolution;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/steenrod/steenrod_coproduct.rs"]
mod steenrod_coproduct;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/steenrod/chain_level_diagonal.rs"]
mod chain_level_diagonal;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/steenrod/adams_fact_engine.rs"]
mod adams_fact_engine;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/steenrod/algebra_generic_engine.rs"]
mod algebra_generic_engine;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/steenrod/adams_differential_family.rs"]
mod adams_differential_family;
