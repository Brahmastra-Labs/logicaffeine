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
    let eval = |l: &Lit| {
        if l.is_positive() {
            a[l.var() as usize]
        } else {
            !a[l.var() as usize]
        }
    };
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
/// as a fresh variable, then the standard prefix-equality chain `encode_lex_le` enforces `x ≤_lex α(x)`.
/// Satisfiability-preserving for any model-set affine symmetry (the lex-least model of each orbit survives).
/// Returns the extra clauses and the new total variable count (aux appended above `num_vars`).
pub fn affine_lex_leader_sbp(
    num_vars: usize,
    maps: &[Vec<(Vec<usize>, bool)>],
) -> (Vec<Vec<Lit>>, usize) {
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
    let positions: Vec<usize> = (0..num_vars)
        .filter(|&j| img[j] != Lit::pos(j as u32))
        .collect();
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
pub fn variable_automorphism_generators(
    num_vars: usize,
    clauses: &[Vec<Lit>],
) -> Option<Vec<Perm>> {
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
pub fn variable_automorphism_group(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    cap: usize,
) -> Option<Vec<Perm>> {
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
pub fn conditional_symmetry_generators(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    fixed: &[Lit],
) -> Vec<Vec<Lit>> {
    literal_automorphism_generators(num_vars, &simplify_under(clauses, fixed))
}

/// The full literal-automorphism group (variable + value symmetry) enumerated via the Schreier–Sims
/// backend on the `2·num_vars` literal points, for the *complete* break. `None` if `|G| > cap` (use the
/// generators for the polynomial partial break instead).
pub fn literal_automorphism_group(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    cap: usize,
) -> Option<Vec<Vec<Lit>>> {
    let gens = literal_automorphism_generators(num_vars, clauses);
    let point_gens: Vec<Perm> = gens.iter().map(|s| litsym_to_points(s, num_vars)).collect();
    let elems = crate::permgroup::schreier_sims(2 * num_vars, &point_gens).elements(cap)?;
    Some(
        elems
            .iter()
            .map(|p| litsym_from_points(p, num_vars))
            .collect(),
    )
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
            s.add_clause(
                (0..num_orig)
                    .map(|v| Lit::new(v as u32, !proj[v]))
                    .collect(),
            );
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
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/sym_break/tests.rs"]
mod tests;
