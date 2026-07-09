//! **Complete lex-leader symmetry breaking, driven by the Schreier–Sims backend.**
//!
//! Per-generator symmetry breaking keeps a *superset* of the canonical representatives. With the whole
//! group in hand — enumerated from a BSGS ([`crate::permgroup`]) — we add the COMPLETE lex-leader
//! predicate `a ≤_lex a∘g` for *every* `g ∈ G`, which keeps EXACTLY the lexicographically-least model of
//! each orbit: the maximal sound symmetry break. Satisfiability is preserved (every orbit keeps one
//! representative), and the number of surviving models equals the number of *orbits* of models.
//!
//! This is feasible when `|G|` is small/moderate — the BSGS reports the order, so the caller gates on it
//! before enumerating. Huge-symmetry families (PHP at scale, `|Aut| = n!·(n−1)!`) are left to the
//! dedicated polynomial specialists; complete lex-leader is for the moderate-symmetry instances those do
//! not target. Scope: **variable** permutations (phase-free automorphisms — the symmetry of the
//! covering/colouring families), acting on assignments by `(a∘g)[j] = a[g[j]]`.

use crate::cdcl::{Lit, SolveResult, Solver};
use crate::permgroup::Perm;

/// Is `a` the lexicographic leader of its orbit — `a ≤_lex a∘g` for every `g`? The semantic canonical
/// test; the CNF predicate [`lex_leader_sbp`] accepts exactly these assignments.
pub fn is_lex_leader(group: &[Perm], a: &[bool]) -> bool {
    let lit_group: Vec<Vec<Lit>> = group.iter().map(|g| perm_to_litsym(g)).collect();
    is_lex_leader_lit(&lit_group, a)
}

/// `is_lex_leader` over **literal** symmetries: `aˢ[j]` is the value of the image literal `img[j]` under
/// `a` (a phase flip negates the compared bit).
pub fn is_lex_leader_lit(group: &[Vec<Lit>], a: &[bool]) -> bool {
    let eval = |l: &Lit| if l.is_positive() { a[l.var() as usize] } else { !a[l.var() as usize] };
    group.iter().all(|img| {
        for j in 0..a.len() {
            let (x, y) = (a[j], eval(&img[j]));
            if x != y {
                return !x && y;
            }
        }
        true
    })
}

/// The lex-leader symmetry-breaking predicate as CNF: for every non-identity `g ∈ group`, clauses
/// asserting `a ≤_lex a∘g`. Returns the extra clauses plus the new total variable count (prefix-equality
/// aux variables are appended above `num_vars`). It is satisfiability-preserving for any set of
/// automorphisms (the lex-least model of each orbit always survives). Pass the **whole group** for the
/// COMPLETE break (exactly one model per orbit), or just a **generating set** for a sound POLYNOMIAL
/// PARTIAL break that scales to arbitrarily large groups — both keep at least one representative per orbit.
pub fn lex_leader_sbp(num_vars: usize, group: &[Perm]) -> (Vec<Vec<Lit>>, usize) {
    let lit_group: Vec<Vec<Lit>> = group.iter().map(|g| perm_to_litsym(g)).collect();
    lex_leader_sbp_lit(num_vars, &lit_group)
}

/// The lex-leader SBP over **literal** symmetries (`group[k][j]` = the image literal of variable `j`),
/// which breaks **variable and value/phase** symmetry alike. As with [`lex_leader_sbp`], pass the whole
/// group for the complete break or a generating set for the polynomial partial break.
pub fn lex_leader_sbp_lit(num_vars: usize, group: &[Vec<Lit>]) -> (Vec<Vec<Lit>>, usize) {
    let mut clauses = Vec::new();
    let mut aux = num_vars;
    for img in group {
        if (0..num_vars).all(|j| img[j] == Lit::pos(j as u32)) {
            continue; // identity contributes nothing
        }
        encode_lex_le(num_vars, img, &mut aux, &mut clauses);
    }
    (clauses, aux)
}

/// A variable permutation as a literal symmetry (no phase flips): `imgⱼ = +x_{g[j]}`.
fn perm_to_litsym(g: &Perm) -> Vec<Lit> {
    g.iter().map(|&j| Lit::pos(j as u32)).collect()
}

/// The lex-leader SBP for **affine** maps `α: x ↦ Ax ⊕ b` over GF(2) — the machinery that breaks the affine
/// parity symmetries a variable/literal permutation SBP ([`lex_leader_sbp`]) structurally cannot express (an
/// image bit is an XOR of *several* variables, not one literal). Each `maps[k]` is a per-output spec:
/// `maps[k][j] = (A_j, b_j)` with `α(x)[j] = ⊕_{i∈A_j} x_i ⊕ b_j`. Each non-identity output is Tseitin-encoded
/// as a fresh variable, then the standard prefix-equality chain [`encode_lex_le`] enforces `x ≤_lex α(x)`.
/// Satisfiability-preserving for any model-set affine symmetry (the lex-least model of each orbit survives).
/// Returns the extra clauses and the new total variable count (aux appended above `num_vars`).
pub fn affine_lex_leader_sbp(num_vars: usize, maps: &[Vec<(Vec<usize>, bool)>]) -> (Vec<Vec<Lit>>, usize) {
    let mut clauses = Vec::new();
    let mut aux = num_vars;
    for map in maps {
        let mut img: Vec<Lit> = (0..num_vars).map(|j| Lit::pos(j as u32)).collect();
        for (j, (xset, b)) in map.iter().enumerate() {
            if j >= num_vars {
                break;
            }
            if xset.len() == 1 && xset[0] == j && !b {
                continue; // identity output — `img[j]` stays `+xⱼ`
            }
            img[j] = tseitin_xor(&mut aux, xset, *b, &mut clauses);
        }
        if (0..num_vars).all(|j| img[j] == Lit::pos(j as u32)) {
            continue; // the identity map contributes nothing
        }
        encode_lex_le(num_vars, &img, &mut aux, &mut clauses);
    }
    (clauses, aux)
}

/// Tseitin-encode `c = ⊕_{i∈xset} x_i ⊕ b` and return the literal `c` (a fresh chain variable, negated when
/// `b`). Chains binary XOR gates `t = acc ⊕ v` (four clauses each). `xset` is assumed non-empty (an affine
/// bijection's output bits each depend on ≥ 1 variable).
fn tseitin_xor(aux: &mut usize, xset: &[usize], b: bool, clauses: &mut Vec<Vec<Lit>>) -> Lit {
    let mut fresh = |aux: &mut usize| {
        let v = *aux as u32;
        *aux += 1;
        Lit::pos(v)
    };
    let mut acc = Lit::pos(xset[0] as u32);
    for &v in &xset[1..] {
        let vv = Lit::pos(v as u32);
        let t = fresh(aux);
        clauses.push(vec![t, acc.negated(), vv]);
        clauses.push(vec![t, acc, vv.negated()]);
        clauses.push(vec![t.negated(), acc, vv]);
        clauses.push(vec![t.negated(), acc.negated(), vv.negated()]);
        acc = t;
    }
    if b {
        acc.negated()
    } else {
        acc
    }
}

/// Encode `a ≤_lex aˢ` where `aˢ[j]` is the value of the image literal `img[j]` under `a` (so a phase
/// flip negates the compared bit — this is how **value/phase** symmetry is broken, not just variable
/// symmetry). Prefix-equality Tseitin chain over fresh `aux` variables: at each non-fixed position `j`,
/// `eₚ → (a[j] ≤ img[j])`, and `eₚ` advances only while the prefix stays equal. Positions where
/// `img[j] = +xⱼ` are always equal and are skipped.
fn encode_lex_le(num_vars: usize, img: &[Lit], aux: &mut usize, clauses: &mut Vec<Vec<Lit>>) {
    let mut fresh = |aux: &mut usize| {
        let v = *aux as u32;
        *aux += 1;
        Lit::pos(v)
    };
    let positions: Vec<usize> = (0..num_vars).filter(|&j| img[j] != Lit::pos(j as u32)).collect();
    let mut prev_e: Option<Lit> = None; // None = the constant TRUE (e₀)
    for (k, &j) in positions.iter().enumerate() {
        let aj = Lit::pos(j as u32);
        let cj = img[j];
        // lex constraint at j: prefix-equal ⟹ a[j] ≤ c[j], i.e. (¬a[j] ∨ c[j]).
        match prev_e {
            None => clauses.push(vec![aj.negated(), cj]),
            Some(e) => clauses.push(vec![e.negated(), aj.negated(), cj]),
        }
        if k + 1 == positions.len() {
            break; // last moved position — no need to carry equality further
        }
        // eq ⟺ (a[j] == c[j])
        let eq = fresh(aux);
        clauses.push(vec![eq.negated(), aj.negated(), cj]);
        clauses.push(vec![eq.negated(), aj, cj.negated()]);
        clauses.push(vec![eq, aj.negated(), cj.negated()]);
        clauses.push(vec![eq, aj, cj]);
        // e_next ⟺ prefix-equal ∧ eq
        let e_next = fresh(aux);
        match prev_e {
            None => {
                clauses.push(vec![e_next.negated(), eq]);
                clauses.push(vec![e_next, eq.negated()]);
            }
            Some(e) => {
                clauses.push(vec![e_next.negated(), e]);
                clauses.push(vec![e_next.negated(), eq]);
                clauses.push(vec![e_next, e.negated(), eq.negated()]);
            }
        }
        prev_e = Some(e_next);
    }
}

/// The **variable**-permutation automorphism GENERATORS of a CNF (phase-free symmetries), without
/// enumerating the group — fast, no size cap. `None` if a detected symmetry flips a phase (a value
/// symmetry this variable scheme does not cover). An empty vector means no non-trivial symmetry.
pub fn variable_automorphism_generators(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<Perm>> {
    let lit_gens = crate::symmetry_detect::find_generators(num_vars, clauses);
    let mut var_gens: Vec<Perm> = Vec::new();
    for g in &lit_gens {
        if g.is_identity() {
            continue;
        }
        let mut vp = vec![0usize; num_vars];
        for v in 0..num_vars as u32 {
            let img = g.apply(Lit::pos(v));
            if !img.is_positive() {
                return None; // a phase flip ⟹ not a pure variable symmetry
            }
            vp[v as usize] = img.var() as usize;
        }
        var_gens.push(vp);
    }
    Some(var_gens)
}

/// The **variable**-permutation automorphism group of a CNF, fully enumerated via the Schreier–Sims
/// backend (for *complete* symmetry breaking). `None` if a generator flips a phase, or `|G| > cap` (then
/// the caller should use per-generator *partial* breaking on [`variable_automorphism_generators`] instead,
/// which scales to arbitrarily large groups).
pub fn variable_automorphism_group(num_vars: usize, clauses: &[Vec<Lit>], cap: usize) -> Option<Vec<Perm>> {
    let gens = variable_automorphism_generators(num_vars, clauses)?;
    crate::permgroup::schreier_sims(num_vars, &gens).elements(cap)
}

/// The literal-point index of a literal: `2v` for `+xᵥ`, `2v+1` for `¬xᵥ`.
fn lit_idx(l: Lit) -> usize {
    2 * l.var() as usize + usize::from(!l.is_positive())
}

/// A literal symmetry (`img[j]` = image literal of variable `j`) as a permutation of the `2·num_vars`
/// literal points — for the Schreier–Sims backend (order, enumeration). Negation is respected.
pub fn litsym_to_points(img: &[Lit], num_vars: usize) -> Perm {
    let mut p = vec![0usize; 2 * num_vars];
    for (j, &l) in img.iter().enumerate() {
        p[lit_idx(Lit::pos(j as u32))] = lit_idx(l);
        p[lit_idx(Lit::neg(j as u32))] = lit_idx(l.negated());
    }
    p
}

/// A permutation of the `2·num_vars` literal points back to a literal symmetry (`img[j]` from where `+xⱼ`
/// goes). Inverse of [`litsym_to_points`].
pub fn litsym_from_points(p: &[usize], num_vars: usize) -> Vec<Lit> {
    (0..num_vars)
        .map(|j| {
            let q = p[2 * j];
            Lit::new((q / 2) as u32, q % 2 == 0)
        })
        .collect()
}

/// The **literal**-permutation automorphism GENERATORS — variable AND **value/phase** symmetry — as
/// image-literal vectors (`imgⱼ = σ(+xⱼ)`). Unlike [`variable_automorphism_generators`], phase flips are
/// kept, so this captures the symmetry of formulas invariant under negating variables. Empty if none.
pub fn literal_automorphism_generators(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<Lit>> {
    crate::symmetry_detect::find_generators(num_vars, clauses)
        .iter()
        .filter(|g| !g.is_identity())
        .map(|g| (0..num_vars as u32).map(|v| g.apply(Lit::pos(v))).collect())
        .collect()
}

/// Simplify `clauses` under the partial assignment `fixed` (a list of literals set true): drop clauses
/// satisfied by a fixed literal, and drop falsified literals from the rest. The result is the residual
/// `F|ρ` down the branch where `fixed` holds.
fn simplify_under(clauses: &[Vec<Lit>], fixed: &[Lit]) -> Vec<Vec<Lit>> {
    let true_lits: std::collections::HashSet<(u32, bool)> =
        fixed.iter().map(|l| (l.var(), l.is_positive())).collect();
    let mut out = Vec::new();
    'clause: for c in clauses {
        let mut nc = Vec::new();
        for &l in c {
            if true_lits.contains(&(l.var(), l.is_positive())) {
                continue 'clause; // a literal is true ⟹ the clause is satisfied
            }
            if true_lits.contains(&(l.var(), !l.is_positive())) {
                continue; // a literal is false ⟹ drop it
            }
            nc.push(l);
        }
        out.push(nc);
    }
    out
}

/// **Conditional (local) symmetry** — the symmetry of the RESIDUAL formula after a partial assignment.
/// A formula can be globally asymmetric yet its residual `F|ρ` symmetric: symmetries that emerge only
/// down a branch, invisible to a global automorphism search. Returns the residual's literal-symmetry
/// generators (image-literal form). This is a different symmetry *source* — the basis for **local
/// symmetry breaking** during search, where each decision can unlock fresh symmetry to exploit.
pub fn conditional_symmetry_generators(num_vars: usize, clauses: &[Vec<Lit>], fixed: &[Lit]) -> Vec<Vec<Lit>> {
    literal_automorphism_generators(num_vars, &simplify_under(clauses, fixed))
}

/// The full literal-automorphism group (variable + value symmetry) enumerated via the Schreier–Sims
/// backend on the `2·num_vars` literal points, for the *complete* break. `None` if `|G| > cap` (use the
/// generators for the polynomial partial break instead).
pub fn literal_automorphism_group(num_vars: usize, clauses: &[Vec<Lit>], cap: usize) -> Option<Vec<Vec<Lit>>> {
    let gens = literal_automorphism_generators(num_vars, clauses);
    let point_gens: Vec<Perm> = gens.iter().map(|s| litsym_to_points(s, num_vars)).collect();
    let elems = crate::permgroup::schreier_sims(2 * num_vars, &point_gens).elements(cap)?;
    Some(elems.iter().map(|p| litsym_from_points(p, num_vars)).collect())
}

/// Count the distinct projections onto the first `num_orig` variables among the models of `clauses` over
/// `total_vars`, by CDCL model-enumeration with blocking clauses. Exponential in the number of distinct
/// projections — for small/moderate instances.
fn count_projected_models(total_vars: usize, num_orig: usize, clauses: &[Vec<Lit>]) -> usize {
    let mut seen: std::collections::BTreeSet<Vec<bool>> = std::collections::BTreeSet::new();
    loop {
        let mut s = Solver::new(total_vars);
        for c in clauses {
            s.add_clause(c.clone());
        }
        for proj in &seen {
            s.add_clause((0..num_orig).map(|v| Lit::new(v as u32, !proj[v])).collect());
        }
        match s.solve() {
            SolveResult::Sat(m) => {
                seen.insert((0..num_orig).map(|v| m[v]).collect());
            }
            SolveResult::Unsat => break,
        }
    }
    seen.len()
}

/// **The number of essentially-distinct solutions** — models counted up to the formula's symmetry: the
/// orbit count of the solution set (`#SAT modulo G`). The complete lex-leader keeps exactly one model per
/// orbit, so counting the symmetry-broken formula's models *is* the orbit count. `None` if the symmetry
/// group is too large to enumerate for the complete break. The counting face of symmetry breaking — and,
/// by Burnside, `(1/|G|)·Σ_σ #{models fixed by σ}`.
pub fn count_models_modulo_symmetry(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<usize> {
    let group = literal_automorphism_group(num_vars, clauses, 100_000)?;
    let (sbp, total) = lex_leader_sbp_lit(num_vars, &group);
    let mut broken = clauses.to_vec();
    broken.extend(sbp);
    Some(count_projected_models(total, num_vars, &broken))
}

/// **Hierarchical (block-wise) symmetry breaking.** For an imprimitive symmetry — a grid like PHP or
/// graph colouring — the minimal block system splits the variables into equal blocks (e.g. the rows). The
/// **adjacent block-swaps** (inter-block) and the **uniform adjacent within-block swaps** (intra-block)
/// are STRUCTURED generators; each is verified to actually lie in the group (`Bsgs::contains`), then their
/// lex-leader is the "sorted blocks, sorted within" break — a POLYNOMIAL set of `O(blocks + block-size)`
/// constraints that breaks the wreath/product symmetry for which the complete enumeration would need `|G|`
/// (exponential) clauses. Sound (it only uses verified group elements). `None` if the group is primitive,
/// has no phase-free symmetry, or no structured generator lies in it. Scope: variable (phase-free) grids.
pub fn hierarchical_break(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<(Vec<Vec<Lit>>, usize)> {
    let gens = variable_automorphism_generators(num_vars, clauses)?;
    if gens.is_empty() {
        return None;
    }
    let bsgs = crate::permgroup::schreier_sims(num_vars, &gens);
    let blocks = crate::permgroup::minimal_block_system(num_vars, &gens)?; // None if primitive / intransitive
    let (k, m) = (blocks.len(), blocks[0].len());
    let mut structured: Vec<Vec<Lit>> = Vec::new();

    // Inter-block: adjacent block swaps b_{i,j} ↔ b_{i+1,j} (swap two whole blocks position-wise).
    for i in 0..k.saturating_sub(1) {
        let mut p: Vec<usize> = (0..num_vars).collect();
        for j in 0..m {
            p[blocks[i][j]] = blocks[i + 1][j];
            p[blocks[i + 1][j]] = blocks[i][j];
        }
        if bsgs.contains(&p) {
            structured.push(perm_to_litsym(&p));
        }
    }
    // Intra-block: uniform adjacent within-block swaps b_{i,j} ↔ b_{i,j+1} across every block at once.
    for j in 0..m.saturating_sub(1) {
        let mut p: Vec<usize> = (0..num_vars).collect();
        for b in &blocks {
            p[b[j]] = b[j + 1];
            p[b[j + 1]] = b[j];
        }
        if bsgs.contains(&p) {
            structured.push(perm_to_litsym(&p));
        }
    }
    if structured.is_empty() {
        return None;
    }
    Some(lex_leader_sbp_lit(num_vars, &structured))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdcl::{SolveResult, Solver};
    use std::collections::BTreeSet;

    /// All satisfying assignments of `clauses` over `num_vars`, by brute force.
    fn models(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<Vec<bool>> {
        (0u64..(1u64 << num_vars))
            .filter_map(|x| {
                let a: Vec<bool> = (0..num_vars).map(|i| (x >> i) & 1 == 1).collect();
                clauses
                    .iter()
                    .all(|c| c.iter().any(|l| a[l.var() as usize] == l.is_positive()))
                    .then_some(a)
            })
            .collect()
    }

    /// The number of orbits of `models` under `group` (action `(a∘g)[j] = a[g[j]]`).
    fn orbit_count(group: &[Perm], models: &[Vec<bool>]) -> usize {
        let mut seen: BTreeSet<Vec<bool>> = BTreeSet::new();
        let mut count = 0;
        for m in models {
            if seen.contains(m) {
                continue;
            }
            count += 1;
            for g in group {
                seen.insert((0..m.len()).map(|j| m[g[j]]).collect());
            }
        }
        count
    }

    /// Count the distinct projections onto the first `num_orig` variables among the models of `clauses`
    /// over `total_vars`, by CDCL model-enumeration with blocking clauses.
    fn count_projected_models(total_vars: usize, num_orig: usize, clauses: &[Vec<Lit>]) -> usize {
        let mut seen: BTreeSet<Vec<bool>> = BTreeSet::new();
        loop {
            let mut s = Solver::new(total_vars);
            for c in clauses {
                s.add_clause(c.clone());
            }
            for proj in &seen {
                s.add_clause((0..num_orig).map(|v| Lit::new(v as u32, !proj[v])).collect());
            }
            match s.solve() {
                SolveResult::Sat(m) => {
                    seen.insert((0..num_orig).map(|v| m[v]).collect());
                }
                SolveResult::Unsat => break,
            }
        }
        seen.len()
    }

    fn all_s_n(n: usize) -> Vec<Perm> {
        let mut out = Vec::new();
        let mut p: Perm = (0..n).collect();
        loop {
            out.push(p.clone());
            let Some(i) = (0..n.saturating_sub(1)).rev().find(|&i| p[i] < p[i + 1]) else { break };
            let j = (i + 1..n).rev().find(|&j| p[j] > p[i]).unwrap();
            p.swap(i, j);
            p[i + 1..].reverse();
        }
        out
    }

    /// **The semantic leader test keeps exactly one per orbit.** Under the full `S_n` permuting `n`
    /// coordinates, the orbits of `{0,1}ⁿ` are the Hamming-weight classes (`n+1` of them), and the
    /// lex-leaders are exactly one per orbit.
    #[test]
    fn lex_leaders_are_one_per_orbit_under_s_n() {
        for n in 2..=5usize {
            let group = all_s_n(n);
            let all: Vec<Vec<bool>> = (0u64..(1u64 << n)).map(|x| (0..n).map(|i| (x >> i) & 1 == 1).collect()).collect();
            let leaders = all.iter().filter(|a| is_lex_leader(&group, a)).count();
            assert_eq!(leaders, n + 1, "S_{n} on the cube has n+1 weight-orbits, one leader each");
            assert_eq!(leaders, orbit_count(&group, &all), "leaders == orbit count");
        }
    }

    /// **The CNF predicate accepts exactly the semantic leaders.** Enumerating the models of the SBP
    /// (projected to the original variables) by CDCL reproduces the semantic leader count — the Tseitin
    /// lex encoding is correct.
    #[test]
    fn the_cnf_sbp_accepts_exactly_the_lex_leaders() {
        for n in 2..=4usize {
            let group = all_s_n(n);
            let (sbp, total) = lex_leader_sbp(n, &group);
            let semantic =
                (0u64..(1u64 << n)).filter(|&x| is_lex_leader(&group, &(0..n).map(|i| (x >> i) & 1 == 1).collect::<Vec<_>>())).count();
            assert_eq!(count_projected_models(total, n, &sbp), semantic, "CNF SBP accepts exactly the leaders");
            assert_eq!(semantic, n + 1, "and there are n+1 of them");
        }
    }

    /// **The affine-map SBP encodes the lex predicate exactly.** For an affine map `α` whose image bits are
    /// XORs of *several* variables, the SBP (extended with its Tseitin/prefix-equality aux) is satisfiable
    /// over `x` iff `x ≤_lex α(x)` — the machinery that breaks affine symmetries a permutation SBP cannot.
    #[test]
    fn affine_lex_leader_encodes_the_lex_predicate() {
        let n = 3usize;
        // α(x)[0] = x0 ⊕ x1, α(x)[1] = x1, α(x)[2] = x2 ⊕ x0.
        let map = vec![(vec![0usize, 1], false), (vec![1], false), (vec![2, 0], false)];
        let (sbp, total) = affine_lex_leader_sbp(n, &[map]);
        let alpha = |x: u64| -> u64 {
            let a0 = (x & 1) ^ ((x >> 1) & 1);
            let a1 = (x >> 1) & 1;
            let a2 = ((x >> 2) & 1) ^ (x & 1);
            a0 | (a1 << 1) | (a2 << 2)
        };
        let lex_le = |x: u64, y: u64| -> bool {
            for j in 0..n {
                let (xj, yj) = ((x >> j) & 1, (y >> j) & 1);
                if xj != yj {
                    return xj < yj;
                }
            }
            true
        };
        for x in 0u64..(1 << n) {
            let accepted = (0u64..(1u64 << (total - n))).any(|aux| {
                let full = x | (aux << n);
                sbp.iter().all(|c| c.iter().any(|l| ((full >> l.var()) & 1 == 1) == l.is_positive()))
            });
            assert_eq!(accepted, lex_le(x, alpha(x)), "SBP must accept x={x:03b} iff x ≤_lex α(x)={:03b}", alpha(x));
        }
    }

    /// **Partial (per-generator) breaking is sound but weaker than complete.** Passing only a generating
    /// set keeps a *superset* of the canonical representatives — at least one per orbit (sound,
    /// satisfiability-preserving), but generally more than the complete break's exactly-one. It is the
    /// polynomial, scalable fallback for groups too large to enumerate.
    #[test]
    fn partial_generator_breaking_is_sound_but_weaker_than_complete() {
        let n = 3;
        let full = all_s_n(n); // 6 elements
        let gens: Vec<Perm> = vec![vec![1, 0, 2], vec![1, 2, 0]]; // (0 1) and (0 1 2) generate S_3
        let (complete, ct) = lex_leader_sbp(n, &full);
        let (partial, pt) = lex_leader_sbp(n, &gens);
        let complete_survivors = count_projected_models(ct, n, &complete);
        let partial_survivors = count_projected_models(pt, n, &partial);
        assert_eq!(complete_survivors, n + 1, "complete keeps exactly one per orbit (n+1 weight classes)");
        assert!(
            partial_survivors >= complete_survivors && partial_survivors <= (1 << n),
            "partial keeps a superset (≥ complete, ≤ all): {partial_survivors} vs {complete_survivors}"
        );
        // Soundness on a satisfiable symmetric formula: at-least-one-true keeps a model under both.
        let f = vec![vec![Lit::pos(0), Lit::pos(1), Lit::pos(2)]];
        let mut with_partial = f.clone();
        with_partial.extend(partial);
        assert!(count_projected_models(pt, n, &with_partial) >= 1, "partial preserves satisfiability");
    }

    /// **The stabilizer-chain break sits between complete and generators — stronger than the bare
    /// generators, still polynomial.** Generators ∪ transversal coset reps is a *superset* of the
    /// generators, so its lex-leader break is at least as strong (fewer-or-equal survivors), and it is a
    /// subset of the whole group, so the complete break is at least as strong as it: complete ≤ chain ≤
    /// generators. All are sound (every survivor count ≥ the orbit count).
    #[test]
    fn stabilizer_chain_break_is_between_complete_and_generators() {
        let n = 4;
        let gens: Vec<Perm> = vec![vec![1, 0, 2, 3], vec![1, 2, 3, 0]]; // (0 1), (0 1 2 3) generate S_4
        let bsgs = crate::permgroup::schreier_sims(n, &gens);
        let complete = bsgs.elements(100_000).unwrap();
        let mut chain = gens.clone();
        chain.extend(bsgs.transversal_elements());
        let survivors = |group: &[Perm]| {
            let (sbp, t) = lex_leader_sbp(n, group);
            count_projected_models(t, n, &sbp)
        };
        let (c, ch, g) = (survivors(&complete), survivors(&chain), survivors(&gens));
        assert_eq!(c, n + 1, "complete keeps exactly one per orbit (n+1 weight classes)");
        assert!(c <= ch && ch <= g, "complete ≤ stabilizer-chain ≤ generators: {c} ≤ {ch} ≤ {g}");
        assert!(g <= (1usize << n), "all sound (≤ total assignments)");
    }

    /// **Value (phase) symmetry is broken — what the variable-only scheme cannot see.** `F = (x₀∨x₁) ∧
    /// (¬x₀∨x₁)` is invariant under *flipping* `x₀` (`x₀ ↦ ¬x₀`); its two models `(0,1),(1,1)` form one
    /// orbit under the flip, and the literal lex-leader keeps exactly one. The phase-free variable scheme
    /// is blind to it (its only generator flips a phase).
    #[test]
    fn value_phase_symmetry_is_broken() {
        let f = vec![vec![Lit::pos(0), Lit::pos(1)], vec![Lit::neg(0), Lit::pos(1)]];
        assert!(
            variable_automorphism_generators(2, &f).is_none_or(|g| g.is_empty()),
            "the phase-free variable scheme is blind to the value symmetry"
        );
        let gens = literal_automorphism_generators(2, &f);
        assert!(!gens.is_empty(), "the value symmetry x₀ ↦ ¬x₀ is detected as a literal symmetry");
        // It must satisfy the negation-respecting round-trip through the literal points.
        for s in &gens {
            assert_eq!(litsym_from_points(&litsym_to_points(s, 2), 2), *s, "litsym ↔ points round-trips");
        }
        let (sbp, total) = lex_leader_sbp_lit(2, &gens);
        let mut broken = f.clone();
        broken.extend(sbp);
        assert_eq!(count_projected_models(total, 2, &broken), 1, "value symmetry broken to one model");
        // Sanity: F itself has two models, so the symmetry genuinely halved them.
        assert_eq!(models(2, &f).len(), 2, "F has two models, one orbit under the flip");
    }

    /// **Complete symmetry breaking on a real formula: one model per orbit, satisfiability preserved.**
    /// `clique_coloring(3,3)` (proper 3-colourings of K₃) has 6 models in a single orbit under
    /// `S₃(vertices) × S₃(colours)`; the complete lex-leader SBP leaves exactly one, and the formula stays
    /// SAT. PHP(3) is UNSAT and stays UNSAT under the SBP (soundness on the unsatisfiable side).
    #[test]
    fn complete_lex_leader_keeps_one_model_per_orbit_end_to_end() {
        // SAT, symmetric.
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let group = variable_automorphism_group(cnf.num_vars, &cnf.clauses, 100_000)
            .expect("clique colouring has a phase-free, small automorphism group");
        let ms = models(cnf.num_vars, &cnf.clauses);
        let orbits = orbit_count(&group, &ms);
        let (sbp, total) = lex_leader_sbp(cnf.num_vars, &group);
        let mut broken = cnf.clauses.clone();
        broken.extend(sbp);
        let surviving = count_projected_models(total, cnf.num_vars, &broken);
        assert_eq!(surviving, orbits, "complete SBP leaves exactly one model per orbit");
        assert!(orbits >= 1 && surviving < ms.len(), "and it strictly breaks the symmetry");

        // UNSAT, symmetric: the SBP must preserve unsatisfiability.
        let (php, _) = crate::families::php(3);
        let pg = variable_automorphism_group(php.num_vars, &php.clauses, 100_000).expect("PHP group");
        let (psbp, ptotal) = lex_leader_sbp(php.num_vars, &pg);
        let mut pbroken = php.clauses.clone();
        pbroken.extend(psbp);
        assert_eq!(count_projected_models(ptotal, php.num_vars, &pbroken), 0, "UNSAT stays UNSAT");
    }

    /// **Conditional (local) symmetry emerges under a partial assignment.** `F` is globally asymmetric —
    /// the clause `x₀∨x₁` singles out `x₁` — so there is no global `x₁↔x₂` symmetry. But under `x₀ = true`
    /// the residual is `(x₁∨x₂) ∧ (¬x₁∨¬x₂)`, symmetric under swapping `x₁ ↔ x₂`: a symmetry that exists
    /// only down that branch. A different symmetry source — the residual's, not the formula's.
    #[test]
    fn conditional_symmetry_emerges_under_a_partial_assignment() {
        let f = vec![
            vec![Lit::neg(0), Lit::pos(1), Lit::pos(2)], // ¬x0 ∨ x1 ∨ x2
            vec![Lit::neg(0), Lit::neg(1), Lit::neg(2)], // ¬x0 ∨ ¬x1 ∨ ¬x2
            vec![Lit::pos(0), Lit::pos(1)],              // x0 ∨ x1   (breaks the global x1↔x2 symmetry)
        ];
        let swaps_12 = |gens: &[Vec<Lit>]| {
            gens.iter().any(|img| img[1] == Lit::pos(2) && img[2] == Lit::pos(1))
        };
        // No global x1↔x2 symmetry.
        assert!(!swaps_12(&literal_automorphism_generators(3, &f)), "F has no global x1↔x2 symmetry");
        // Conditionally on x0 = true, the residual is symmetric under x1↔x2.
        let local = conditional_symmetry_generators(3, &f, &[Lit::pos(0)]);
        assert!(swaps_12(&local), "the residual under x0=true has the x1↔x2 symmetry: {local:?}");
    }

    /// **Hierarchical (block-wise) breaking: a polynomial break of an exponential grid symmetry.** On
    /// `clique_coloring(3,3)` (`S₃×S₃`, `|G| = 36`), the block-wise break uses only the adjacent
    /// vertex-row swaps and uniform colour swaps — a handful of structured generators — yet is sound
    /// (keeps ≥ one model per orbit) and genuinely breaks the symmetry (fewer survivors than models).
    #[test]
    fn hierarchical_block_wise_breaking_is_sound_and_polynomial() {
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let nv = cnf.num_vars;
        let (sbp, total) = hierarchical_break(nv, &cnf.clauses).expect("clique colouring is a grid symmetry");
        let ms = models(nv, &cnf.clauses);
        let var_group = variable_automorphism_group(nv, &cnf.clauses, 100_000).unwrap();
        let orbits = orbit_count(&var_group, &ms);
        let mut broken = cnf.clauses.clone();
        broken.extend(sbp);
        let surviving = count_projected_models(total, nv, &broken);
        assert!(surviving >= orbits, "hierarchical break is sound: ≥ one model per orbit ({surviving} ≥ {orbits})");
        assert!(surviving < ms.len(), "and it breaks the symmetry: fewer survivors than the {} models", ms.len());
    }

    /// **PHP's symmetry is an imprimitive grid — symmetry within the symmetry.** PHP(n)'s variable group
    /// `S_n × S_{n-1}` acts on the `n × (n-1)` grid of variables `x_{p,h}`; it is **imprimitive**, and the
    /// pigeon rows are the minimal block system (`n` blocks of size `n-1`). The block structure lays the
    /// grid bare — the internal structure of the formula's symmetry, recovered group-theoretically.
    #[test]
    fn php_symmetry_is_an_imprimitive_grid() {
        let n = 4;
        let (cnf, _) = crate::families::php(n);
        let gens = variable_automorphism_generators(cnf.num_vars, &cnf.clauses).expect("phase-free");
        assert!(
            !crate::permgroup::is_primitive(cnf.num_vars, &gens),
            "PHP's symmetry is imprimitive — it is a grid, not an atom"
        );
        let blocks =
            crate::permgroup::minimal_block_system(cnf.num_vars, &gens).expect("PHP has a block system");
        assert!(blocks.iter().all(|b| b.len() == n - 1), "blocks are pigeon-rows of size n-1: {blocks:?}");
        assert_eq!(blocks.len(), n, "there are n pigeon-rows");
        for b in &blocks {
            let pigeon = b[0] / (n - 1);
            assert!(b.iter().all(|&v| v / (n - 1) == pigeon), "each block is exactly one pigeon's row: {b:?}");
        }
    }

    /// **Counting up to symmetry = the orbit count, three ways.** The number of essentially-distinct
    /// solutions equals the complete-SBP model count, the brute-force orbit count, AND Burnside's lemma
    /// `(1/|G|)·Σ_σ #{models fixed by σ}` — the canonical orbit-counting theorem. On `clique_coloring(3,3)`
    /// all six proper colourings lie in a single orbit, so there is essentially one solution.
    #[test]
    fn count_modulo_symmetry_equals_burnside_and_brute_orbit_count() {
        let (cnf, _) = crate::families::clique_coloring(3, 3);
        let nv = cnf.num_vars;
        let group = literal_automorphism_group(nv, &cnf.clauses, 100_000).unwrap();
        let ms = models(nv, &cnf.clauses);
        let image = |img: &[Lit], a: &[bool]| -> Vec<bool> {
            (0..a.len())
                .map(|j| if img[j].is_positive() { a[img[j].var() as usize] } else { !a[img[j].var() as usize] })
                .collect()
        };
        // Brute-force orbit count under the literal action.
        let brute = {
            let mut seen: BTreeSet<Vec<bool>> = BTreeSet::new();
            let mut c = 0;
            for m in &ms {
                if seen.contains(m) {
                    continue;
                }
                c += 1;
                for s in &group {
                    seen.insert(image(s, m));
                }
            }
            c
        };
        // Burnside: the average number of fixed models over the group.
        let fixed: usize = group.iter().map(|s| ms.iter().filter(|&m| image(s, m) == *m).count()).sum();
        let burnside = fixed / group.len();
        let counted = count_models_modulo_symmetry(nv, &cnf.clauses).unwrap();
        assert_eq!(counted, brute, "complete-SBP count == brute orbit count");
        assert_eq!(counted, burnside, "== Burnside count");
        assert_eq!(counted, 1, "clique_coloring(3,3): all six proper colourings are one orbit");
        assert_eq!(ms.len(), 6, "and there are six of them");
    }
}
