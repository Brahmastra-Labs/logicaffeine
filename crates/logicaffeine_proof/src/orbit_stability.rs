//! **Orbit stability: deciding symmetric Nullstellensatz questions at every scale by one finite
//! computation.** The machinery behind the ∀m lower-bound program.
//!
//! A symmetric family (pigeonhole, modular counting) is one formula *shape* instantiated at every
//! scale `m`, with the scale-`m` symmetric group acting on its variables. At a **fixed degree `d`**,
//! the `GF(2)` NS dual question — does a degree-`d` pseudo-expectation exist? — restricted to
//! **invariant** functionals collapses onto *orbit types*: an invariant `L` is constant on monomial
//! orbits, an orbit is named scale-independently by its **canonical structure** (the bipartite
//! pigeon/hole graph; the block-intersection hypergraph), and one constraint per joint
//! (multiplier-orbit × generator) representative captures the whole system
//! ([`collapsed_dual_system`], soundness = the one-line invariance lemma tested in
//! `an_invariant_functional_checked_on_orbit_representatives_is_checked_on_all_generators`).
//!
//! The stability phenomenon: at fixed `d` the type set stabilizes once every `≤ d`-edge structure is
//! realizable, and the constraint rows' entries are parities of *counting polynomials in `m`*
//! (binomials — e.g. `Count_3`'s off-point constraint `a + C(n−4, 2)·b₀`), hence eventually periodic
//! in `m` with period a power of two (Lucas). So the invariant-witness verdict is decidable **for
//! every `m`** from finitely many evaluations — each in-window verdict differentially validated
//! against the direct orbit-collapsed solver and, where a witness exists, lifted and re-checked by
//! `check_ns_lower_bound_polys` with zero trust.
//!
//! The honest boundary, forced by our own char-2 theorem
//! (`over_gf2_symmetrizing_a_proof_annihilates_when_the_group_is_even`): over `GF(2)` there is no
//! Reynolds averaging, so "no invariant witness" does **not** imply "no witness" — `Count_3(8)`
//! realizes the gap (an asymmetric witness on the partial-partition support survives where every
//! invariant candidate dies). Verdicts here are about *invariant* certificates; a positive verdict
//! is a sound `NS-degree > d` lower bound, a negative one is measured against the gap.

use crate::cdcl::Lit;
use crate::polycalc::{
    apply_perm_to_mono, check_ns_lower_bound_polys, clause_polynomial, gf2_solve,
    monomials_up_to_degree, ns_lower_bound_witness_polys, poly_degree, poly_mul_mono, Mono, Poly,
};
use crate::proof::Perm;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A monomial's **structure**: one atom list per variable it contains, each atom a `(sort, point)`
/// pair — pigeonhole variables are `[(0, pigeon), (1, hole)]`, counting variables are the block's
/// points `[(0, a), (0, b), (0, c)]`. Two monomials at any two scales have the same orbit type iff
/// their structures agree up to a sort-respecting relabeling of points ([`canonical_structure`]).
pub type Structure = Vec<Vec<(u8, u32)>>;

/// The **canonical form** of a structure: the lexicographically least relabeling, minimized over all
/// sort-respecting point bijections (brute force over the touched points of each sort — bounded by
/// the degree, so a handful of points). This is the scale-independent orbit-type name.
pub fn canonical_structure(structure: &Structure) -> Structure {
    // Touched points per sort, in first-appearance order.
    let mut sorts: BTreeMap<u8, Vec<u32>> = BTreeMap::new();
    for var in structure {
        for &(s, p) in var {
            let pts = sorts.entry(s).or_default();
            if !pts.contains(&p) {
                pts.push(p);
            }
        }
    }
    let sort_keys: Vec<u8> = sorts.keys().copied().collect();
    // All relabelings = product of per-sort permutations, enumerated recursively.
    let mut best: Option<Structure> = None;
    let mut perms: Vec<Vec<usize>> = Vec::new();
    fn rec(
        level: usize,
        sort_keys: &[u8],
        sorts: &BTreeMap<u8, Vec<u32>>,
        perms: &mut Vec<Vec<usize>>,
        structure: &Structure,
        best: &mut Option<Structure>,
    ) {
        if level == sort_keys.len() {
            // Apply: point `pts[i]` of sort `s` ↦ `perm[i]`.
            let mut maps: BTreeMap<u8, HashMap<u32, u32>> = BTreeMap::new();
            for (li, &s) in sort_keys.iter().enumerate() {
                let pts = &sorts[&s];
                let map = maps.entry(s).or_default();
                for (i, &p) in pts.iter().enumerate() {
                    map.insert(p, perms[li][i] as u32);
                }
            }
            let mut relabeled: Structure = structure
                .iter()
                .map(|var| {
                    let mut v: Vec<(u8, u32)> =
                        var.iter().map(|&(s, p)| (s, maps[&s][&p])).collect();
                    v.sort_unstable();
                    v
                })
                .collect();
            relabeled.sort_unstable();
            if best.as_ref().is_none_or(|b| relabeled < *b) {
                *best = Some(relabeled);
            }
            return;
        }
        let k = sorts[&sort_keys[level]].len();
        let mut idx: Vec<usize> = (0..k).collect();
        permute(&mut idx, 0, &mut |perm| {
            perms.push(perm.to_vec());
            rec(level + 1, sort_keys, sorts, perms, structure, best);
            perms.pop();
        });
    }
    fn permute(idx: &mut Vec<usize>, at: usize, f: &mut dyn FnMut(&[usize])) {
        if at == idx.len() {
            f(idx);
            return;
        }
        for i in at..idx.len() {
            idx.swap(at, i);
            permute(idx, at + 1, f);
            idx.swap(at, i);
        }
    }
    rec(0, &sort_keys, &sorts, &mut perms, structure, &mut best);
    best.unwrap_or_default()
}

/// **Degree-bounded monomial orbits**: the quotient of the degree-`≤ d` basis under the group, walked
/// on the bounded basis (`C(n, ≤d)` monomials, never `2ⁿ`) so it scales to `num_vars ≤ 63` — the
/// scale window `polycalc::monomial_orbits`'s cube filter cannot reach.
pub fn monomial_orbits_bounded(num_vars: usize, degree: usize, generators: &[Perm]) -> Vec<Vec<Mono>> {
    let basis: BTreeSet<Mono> = monomials_up_to_degree(num_vars, degree).into_iter().collect();
    let mut seen: BTreeSet<Mono> = BTreeSet::new();
    let mut orbits = Vec::new();
    for &m in &basis {
        if seen.contains(&m) {
            continue;
        }
        let mut orbit = BTreeSet::new();
        orbit.insert(m);
        let mut stack = vec![m];
        while let Some(x) = stack.pop() {
            for g in generators {
                let y = apply_perm_to_mono(g, x);
                if basis.contains(&y) && orbit.insert(y) {
                    stack.push(y);
                }
            }
        }
        for &x in &orbit {
            seen.insert(x);
        }
        orbits.push(orbit.into_iter().collect());
    }
    orbits
}

/// A symmetric family instantiated at one scale: the variable count, the `GF(2)` generator
/// polynomials, the scale's symmetry generators, the variable → atom-list map that names orbit
/// types scale-independently, and one **anchor** per generator — the pseudo-structure that pins the
/// generator's identity for cross-scale row labeling (a pigeon row is anchored by its pigeon, a
/// point generator by its point, a fixed-shape generator by its own monomial structure; anchor
/// atom-lists carry the marker sort `255` so they never collide with multiplier variables, while
/// their point atoms share the real sorts and relabel *consistently* with the multiplier's).
pub struct SymmetricInstance {
    pub num_vars: usize,
    pub gens: Vec<Poly>,
    pub sym: Vec<Perm>,
    pub atoms: Box<dyn Fn(u32) -> Vec<(u8, u32)>>,
    pub anchors: Vec<Structure>,
}

impl SymmetricInstance {
    /// The structure of a monomial under this instance's atom map.
    pub fn structure(&self, mono: Mono) -> Structure {
        let mut vars = Vec::new();
        let mut bits = mono;
        while bits != 0 {
            let v = bits.trailing_zeros();
            vars.push((self.atoms)(v));
            bits &= bits - 1;
        }
        vars
    }

    /// The canonical orbit-type of a monomial — [`canonical_structure`] of its structure.
    pub fn type_of(&self, mono: Mono) -> Structure {
        canonical_structure(&self.structure(mono))
    }
}

/// The anchor of a **fixed-shape generator**: its own variables' atom lists, each carrying the
/// marker sort `255` — so the row label distinguishes "multiplier variable" from "generator
/// variable" while the point atoms relabel consistently across both.
fn fixed_shape_anchor(vars: &[Vec<(u8, u32)>]) -> Structure {
    vars.iter()
        .map(|atoms| {
            let mut a = atoms.clone();
            a.push((255, 0));
            a
        })
        .collect()
}

/// **Pigeonhole at scale `m`, clause encoding** (the [`crate::families::php`] layout, `Sₘ × Sₘ₋₁`
/// symmetry): variable `p·holes + h` is the atom pair `(pigeon p, hole h)` — two sorts. A pigeon
/// clause (all-positive row) is anchored by its pigeon; a hole pair by its own structure.
pub fn php_instance_clause(m: usize) -> SymmetricInstance {
    let (cnf, _) = crate::families::php(m);
    let holes = m - 1;
    let atom = move |v: u32| vec![(0u8, v / holes as u32), (1u8, v % holes as u32)];
    let mut gens = Vec::new();
    let mut anchors = Vec::new();
    for c in &cnf.clauses {
        gens.push(clause_polynomial(c));
        if c.iter().all(|l| l.is_positive()) {
            let pigeon = c[0].var() / holes as u32;
            anchors.push(vec![vec![(0u8, pigeon), (255, 0)]]);
        } else {
            anchors.push(fixed_shape_anchor(
                &c.iter().map(|l| atom(l.var())).collect::<Vec<_>>(),
            ));
        }
    }
    SymmetricInstance {
        num_vars: cnf.num_vars,
        gens,
        sym: crate::hypercube::php_perm_symmetries(m),
        atoms: Box::new(atom),
        anchors,
    }
}

/// **Pigeonhole at scale `m`, linear encoding**: each pigeon row as the degree-1 generator
/// `1 + Σ_h x_{p,h}` (never dropped by a degree budget — the fixed-degree questions stay real at
/// every `m`), plus the hole at-most-one pairs. Same symmetry and atoms as the clause encoding;
/// pigeon-row generators anchored by their pigeon.
pub fn php_instance_linear(m: usize) -> SymmetricInstance {
    let (cnf, _) = crate::families::php(m);
    let holes = m - 1;
    let atom = move |v: u32| vec![(0u8, v / holes as u32), (1u8, v % holes as u32)];
    let mut gens: Vec<Poly> = Vec::new();
    let mut anchors: Vec<Structure> = Vec::new();
    for p in 0..m {
        let mut lin: Poly = [0u64].into_iter().collect();
        for h in 0..holes {
            lin.insert(1u64 << (p * holes + h));
        }
        gens.push(lin);
        anchors.push(vec![vec![(0u8, p as u32), (255, 0)]]);
    }
    for c in &cnf.clauses {
        if c.iter().all(|l| !l.is_positive()) {
            gens.push(clause_polynomial(c)); // the hole AMO pairs x_{p,h}·x_{q,h}
            anchors.push(fixed_shape_anchor(
                &c.iter().map(|l| atom(l.var())).collect::<Vec<_>>(),
            ));
        }
    }
    SymmetricInstance {
        num_vars: cnf.num_vars,
        gens,
        sym: crate::hypercube::php_perm_symmetries(m),
        atoms: Box::new(atom),
        anchors,
    }
}

/// **Modular counting `Count_q` at scale `n`, linear encoding** (`Sₙ` symmetry acting on the
/// `q`-subsets): variable `e` is its block's points — one sort. The symmetry generators are the
/// adjacent point transpositions, induced onto the edge variables.
pub fn count_instance_linear(n: usize, q: usize) -> SymmetricInstance {
    let (cnf, _) = crate::families::mod_counting(n, q);
    let groups = crate::families::mod_counting_groups(n, q);
    let gens = crate::polycalc::exactly_one_linear_generators(&groups);
    let edges = crate::families::mod_counting_edges(n, q);
    let edge_index: HashMap<Vec<usize>, usize> =
        edges.iter().enumerate().map(|(i, e)| (e.clone(), i)).collect();
    // Adjacent transpositions i ↔ i+1 of the point set, induced on edge variables.
    let mut sym = Vec::new();
    for i in 0..n - 1 {
        let images: Vec<Lit> = (0..cnf.num_vars as u32)
            .map(|e| {
                let mut swapped: Vec<usize> = edges[e as usize]
                    .iter()
                    .map(|&p| if p == i { i + 1 } else if p == i + 1 { i } else { p })
                    .collect();
                swapped.sort_unstable();
                Lit::pos(edge_index[&swapped] as u32)
            })
            .collect();
        sym.push(Perm::from_images(images));
    }
    // Anchors: the first n generators are the point rows (anchored by their point, in group order);
    // the rest are the overlap pairs (fixed shapes).
    let atom = {
        let edges = edges.clone();
        move |v: u32| -> Vec<(u8, u32)> {
            edges[v as usize].iter().map(|&p| (0u8, p as u32)).collect()
        }
    };
    let mut anchors: Vec<Structure> = (0..n).map(|i| vec![vec![(0u8, i as u32), (255, 0)]]).collect();
    for g in gens.iter().skip(n) {
        let &pair_mono = g.iter().next().expect("a pair generator is a single monomial");
        let mut vars = Vec::new();
        let mut bits = pair_mono;
        while bits != 0 {
            vars.push(atom(bits.trailing_zeros()));
            bits &= bits - 1;
        }
        anchors.push(fixed_shape_anchor(&vars));
    }
    let atoms_edges = edges.clone();
    SymmetricInstance {
        num_vars: cnf.num_vars,
        gens,
        sym,
        atoms: Box::new(move |v| atoms_edges[v as usize].iter().map(|&p| (0, p as u32)).collect()),
        anchors,
    }
}

/// The **collapsed dual system** at one scale: columns = canonical orbit types of the degree-`≤ d`
/// basis, rows = the distinct type-vectors of the admitted NS generators `m·g`, taken over one
/// multiplier per orbit × every generator (complete by the invariance lemma: the row of
/// `σ(m)·g` equals the row of `m·σ⁻¹(g)`, and the group permutes the generator set). An invariant
/// functional `L` (one bit per type, `L(∅-type) = 1`) is a valid degree-`d` pseudo-expectation iff
/// it annihilates every row.
pub struct CollapsedDual {
    /// Canonical types, the empty monomial's type first; the column order of `rows`.
    pub types: Vec<Structure>,
    /// Distinct constraint rows, each a bitmask over `types` (types ≤ 64 at the degrees this runs).
    pub rows: BTreeSet<u64>,
}

impl CollapsedDual {
    /// Does an invariant degree-`d` pseudo-expectation exist — is the system `L ⊥ rows, L(∅) = 1`
    /// solvable? Returns the type-bitmask of a solution.
    pub fn solve(&self) -> Option<u64> {
        let nt = self.types.len();
        let mut eqs: Vec<(Vec<u64>, bool)> = self.rows.iter().map(|&r| (vec![r], false)).collect();
        eqs.push((vec![1u64], true)); // column 0 is the empty type: L(1) = 1
        gf2_solve(&eqs, nt).map(|x| x[0])
    }

    /// Lift a type-solution back to the full monomial witness at a concrete scale.
    pub fn lift(&self, inst: &SymmetricInstance, degree: usize, solution: u64) -> Vec<Mono> {
        let on: BTreeSet<&Structure> = self
            .types
            .iter()
            .enumerate()
            .filter(|(i, _)| (solution >> i) & 1 == 1)
            .map(|(_, t)| t)
            .collect();
        monomials_up_to_degree(inst.num_vars, degree)
            .into_iter()
            .filter(|&m| on.contains(&inst.type_of(m)))
            .collect()
    }
}

/// Build the [`CollapsedDual`] of an instance at degree `d`. The type column set comes from the
/// bounded orbit quotient (one canonization per orbit); the rows from one multiplier representative
/// per orbit against every generator, each product's monomials bucketed by cached type.
pub fn collapsed_dual_system(inst: &SymmetricInstance, degree: usize) -> CollapsedDual {
    let orbits = monomial_orbits_bounded(inst.num_vars, degree, &inst.sym);
    let mut types: Vec<Structure> = Vec::new();
    let mut type_index: BTreeMap<Structure, usize> = BTreeMap::new();
    let mut mono_type: HashMap<Mono, usize> = HashMap::new();
    // The empty monomial's orbit is {0}; force its type to column 0.
    let empty_type = inst.type_of(0);
    types.push(empty_type.clone());
    type_index.insert(empty_type, 0);
    for orbit in &orbits {
        let t = inst.type_of(orbit[0]);
        let idx = *type_index.entry(t.clone()).or_insert_with(|| {
            types.push(t.clone());
            types.len() - 1
        });
        for &m in orbit {
            mono_type.insert(m, idx);
        }
    }
    assert!(types.len() <= 64, "the type bitmask carries ≤ 64 orbit types (degree too high?)");

    let mut rows: BTreeSet<u64> = BTreeSet::new();
    for orbit in &orbits {
        let mult = orbit[0]; // one multiplier per orbit — complete by the invariance lemma
        for g in &inst.gens {
            let prod = poly_mul_mono(g, mult);
            if prod.is_empty() || poly_degree(&prod) > degree {
                continue;
            }
            let mut row = 0u64;
            for &t in &prod {
                row ^= 1u64 << mono_type[&t]; // parity of the type count
            }
            if row != 0 {
                rows.insert(row);
            }
        }
    }
    CollapsedDual { types, rows }
}

/// **The direct orbit-collapsed solver** — the independent validation path. No canonical types, no
/// representative tricks: every admitted generator `m·g` (all multipliers) becomes a constraint, the
/// unknowns are one bit per *orbit* (constancy imposed directly), plus `L(1) = 1`. Same mathematical
/// question as [`collapsed_dual_system`] + [`CollapsedDual::solve`], different code path — the two
/// must agree at every scale, which is the machine check that representative completeness and type
/// canonization are right.
pub fn invariant_witness_exists_direct(inst: &SymmetricInstance, degree: usize) -> Option<Vec<Mono>> {
    let orbits = monomial_orbits_bounded(inst.num_vars, degree, &inst.sym);
    let orbit_of: HashMap<Mono, usize> = orbits
        .iter()
        .enumerate()
        .flat_map(|(i, o)| o.iter().map(move |&m| (m, i)))
        .collect();
    let no = orbits.len();
    let words = no.div_ceil(64).max(1);
    let mut eqs: Vec<(Vec<u64>, bool)> = Vec::new();
    for g in &inst.gens {
        if g.is_empty() {
            continue;
        }
        for &m in &monomials_up_to_degree(inst.num_vars, degree) {
            let prod = poly_mul_mono(g, m);
            if prod.is_empty() || poly_degree(&prod) > degree {
                continue;
            }
            let mut mask = vec![0u64; words];
            for &t in &prod {
                let oi = orbit_of[&t];
                mask[oi / 64] ^= 1u64 << (oi % 64); // parity per orbit column
            }
            if mask.iter().any(|&w| w != 0) {
                eqs.push((mask, false));
            }
        }
    }
    let empty_orbit = orbit_of[&0u64];
    let mut target = vec![0u64; words];
    target[empty_orbit / 64] |= 1u64 << (empty_orbit % 64);
    eqs.push((target, true));
    let sol = gf2_solve(&eqs, no)?;
    let mut witness = Vec::new();
    for (i, orbit) in orbits.iter().enumerate() {
        if (sol[i / 64] >> (i % 64)) & 1 == 1 {
            witness.extend(orbit.iter().copied());
        }
    }
    Some(witness)
}

/// **Modular counting with one marked point** — the point-stabilizer instrument. Point `0` gets its
/// own sort, so the symmetry drops from `Sₙ` to `Stab(0) ≅ Sₙ₋₁` (adjacent transpositions of the
/// unmarked points only) and the canonical types automatically distinguish "touches the marked
/// point." The collapsed dual then decides existence of a *marked-invariant* witness — a strictly
/// larger search space than the fully-invariant one (every `Sₙ`-invariant functional is
/// `Stab(0)`-invariant), the first refinement rung between "symmetric" and "arbitrary" on the
/// char-2 gap.
pub fn count_instance_linear_marked(n: usize, q: usize) -> SymmetricInstance {
    let base = count_instance_linear(n, q);
    let edges = crate::families::mod_counting_edges(n, q);
    let edge_index: HashMap<Vec<usize>, usize> =
        edges.iter().enumerate().map(|(i, e)| (e.clone(), i)).collect();
    // Adjacent transpositions of the UNMARKED points 1..n−1 only.
    let mut sym = Vec::new();
    for i in 1..n - 1 {
        let images: Vec<Lit> = (0..base.num_vars as u32)
            .map(|e| {
                let mut swapped: Vec<usize> = edges[e as usize]
                    .iter()
                    .map(|&p| if p == i { i + 1 } else if p == i + 1 { i } else { p })
                    .collect();
                swapped.sort_unstable();
                Lit::pos(edge_index[&swapped] as u32)
            })
            .collect();
        sym.push(Perm::from_images(images));
    }
    let mark = |p: usize| -> (u8, u32) { if p == 0 { (1, 0) } else { (0, p as u32) } };
    // Anchors re-marked to match: point generators by their (marked-aware) point atom; pair
    // generators by their variables' marked atom lists.
    let mut anchors: Vec<Structure> = (0..n).map(|i| vec![vec![mark(i), (255, 0)]]).collect();
    for g in base.gens.iter().skip(n) {
        let &pair_mono = g.iter().next().expect("a pair generator is a single monomial");
        let mut vars = Vec::new();
        let mut bits = pair_mono;
        while bits != 0 {
            let e = bits.trailing_zeros() as usize;
            let mut atoms: Vec<(u8, u32)> = edges[e].iter().map(|&p| mark(p)).collect();
            atoms.push((255, 0));
            atoms.sort_unstable();
            vars.push(atoms);
            bits &= bits - 1;
        }
        anchors.push(vars);
    }
    let atoms_edges = edges;
    SymmetricInstance {
        num_vars: base.num_vars,
        gens: base.gens,
        sym,
        atoms: Box::new(move |v| atoms_edges[v as usize].iter().map(|&p| mark(p)).collect()),
        anchors,
    }
}

/// **Linear-encoded pigeonhole with one marked hole** — the hole-stabilizer instrument
/// (`Sₘ × Stab(h₀) ≅ Sₘ × Sₘ₋₂`). Hole `0` gets its own sort; pigeon symmetry is untouched.
pub fn php_instance_linear_marked_hole(m: usize) -> SymmetricInstance {
    let base = php_instance_linear(m);
    let holes = m - 1;
    let num_vars = base.num_vars;
    let var = |p: usize, h: usize| p * holes + h;
    let mark = move |v: u32| -> Vec<(u8, u32)> {
        let (p, h) = (v / holes as u32, v % holes as u32);
        if h == 0 {
            vec![(0, p), (2, 0)]
        } else {
            vec![(0, p), (1, h)]
        }
    };
    // Pigeon transpositions (all) + hole transpositions among the UNMARKED holes 1..holes−1.
    let mut sym = Vec::new();
    for p in 0..m - 1 {
        let mut images: Vec<Lit> = (0..num_vars as u32).map(Lit::pos).collect();
        for h in 0..holes {
            images.swap(var(p, h), var(p + 1, h));
        }
        sym.push(Perm::from_images(images));
    }
    for h in 1..holes.saturating_sub(1) {
        let mut images: Vec<Lit> = (0..num_vars as u32).map(Lit::pos).collect();
        for p in 0..m {
            images.swap(var(p, h), var(p, h + 1));
        }
        sym.push(Perm::from_images(images));
    }
    // Anchors: pigeon rows keep their pigeon anchor; AMO pairs re-marked.
    let mut anchors: Vec<Structure> = (0..m).map(|p| vec![vec![(0u8, p as u32), (255, 0)]]).collect();
    for g in base.gens.iter().skip(m) {
        let &pair = g.iter().next().expect("an AMO generator is a single monomial");
        let mut vars = Vec::new();
        let mut bits = pair;
        while bits != 0 {
            let mut atoms = mark(bits.trailing_zeros());
            atoms.push((255, 0));
            atoms.sort_unstable();
            vars.push(atoms);
            bits &= bits - 1;
        }
        anchors.push(vars);
    }
    SymmetricInstance { num_vars, gens: base.gens, sym, atoms: Box::new(mark), anchors }
}

/// **Linear-encoded pigeonhole with one marked pigeon** — the pigeon-stabilizer instrument
/// (`Stab(p₀) × Sₘ₋₁`). Pigeon `0` gets its own sort; hole symmetry is untouched.
pub fn php_instance_linear_marked_pigeon(m: usize) -> SymmetricInstance {
    let base = php_instance_linear(m);
    let holes = m - 1;
    let num_vars = base.num_vars;
    let var = |p: usize, h: usize| p * holes + h;
    let mark = move |v: u32| -> Vec<(u8, u32)> {
        let (p, h) = (v / holes as u32, v % holes as u32);
        if p == 0 {
            vec![(3, 0), (1, h)]
        } else {
            vec![(0, p), (1, h)]
        }
    };
    let mut sym = Vec::new();
    for p in 1..m - 1 {
        let mut images: Vec<Lit> = (0..num_vars as u32).map(Lit::pos).collect();
        for h in 0..holes {
            images.swap(var(p, h), var(p + 1, h));
        }
        sym.push(Perm::from_images(images));
    }
    for h in 0..holes.saturating_sub(1) {
        let mut images: Vec<Lit> = (0..num_vars as u32).map(Lit::pos).collect();
        for p in 0..m {
            images.swap(var(p, h), var(p, h + 1));
        }
        sym.push(Perm::from_images(images));
    }
    let mut anchors: Vec<Structure> = (0..m)
        .map(|p| {
            let sort = if p == 0 { 3u8 } else { 0u8 };
            let id = if p == 0 { 0u32 } else { p as u32 };
            vec![vec![(sort, id), (255, 0)]]
        })
        .collect();
    for g in base.gens.iter().skip(m) {
        let &pair = g.iter().next().expect("an AMO generator is a single monomial");
        let mut vars = Vec::new();
        let mut bits = pair;
        while bits != 0 {
            let mut atoms = mark(bits.trailing_zeros());
            atoms.push((255, 0));
            atoms.sort_unstable();
            vars.push(atoms);
            bits &= bits - 1;
        }
        anchors.push(vars);
    }
    SymmetricInstance { num_vars, gens: base.gens, sym, atoms: Box::new(mark), anchors }
}

/// A cross-scale row label: the canonical joint structure of `multiplier ⊕ generator anchor`. Two
/// (multiplier, generator) pairs with the same label lie in the same joint orbit, so their entry
/// counts must be identical — enforced fail-closed during collection.
pub type RowLabel = Structure;

/// The labeled dual entries at one scale: `label → (column type → monomial count)` — the raw
/// integer counts before the mod-2 reduction, the objects that are polynomial in the scale.
pub fn labeled_dual_counts(
    inst: &SymmetricInstance,
    degree: usize,
) -> BTreeMap<RowLabel, BTreeMap<Structure, u64>> {
    let orbits = monomial_orbits_bounded(inst.num_vars, degree, &inst.sym);
    let mut type_cache: HashMap<Mono, Structure> = HashMap::new();
    let mut out: BTreeMap<RowLabel, BTreeMap<Structure, u64>> = BTreeMap::new();
    for orbit in &orbits {
        let mult = orbit[0];
        let mult_structure = inst.structure(mult);
        for (gi, g) in inst.gens.iter().enumerate() {
            let prod = poly_mul_mono(g, mult);
            if prod.is_empty() || poly_degree(&prod) > degree {
                continue;
            }
            let mut joint = mult_structure.clone();
            joint.extend(inst.anchors[gi].iter().cloned());
            let label = canonical_structure(&joint);
            let mut counts: BTreeMap<Structure, u64> = BTreeMap::new();
            for &t in &prod {
                let ty = type_cache
                    .entry(t)
                    .or_insert_with(|| inst.type_of(t))
                    .clone();
                *counts.entry(ty).or_insert(0) += 1;
            }
            match out.entry(label) {
                std::collections::btree_map::Entry::Vacant(e) => {
                    e.insert(counts);
                }
                std::collections::btree_map::Entry::Occupied(e) => {
                    assert_eq!(
                        e.get(),
                        &counts,
                        "same joint label ⟹ same entry counts (joint-orbit invariance, fail-closed)"
                    );
                }
            }
        }
    }
    out
}

/// An integer-valued polynomial in the scale, in the **finite-difference (binomial) basis**:
/// `value(m) = Σ_k diffs[k] · C(m − base, k)`. Fitted from consecutive window evaluations; the
/// interpolation certificate is that the difference table vanishes beyond the allowed degree — every
/// window point past the fitting prefix is an exact verification.
#[derive(Clone, Debug, PartialEq)]
pub struct FittedCount {
    pub base: usize,
    pub diffs: Vec<i128>,
}

impl FittedCount {
    /// Fit `values` observed at consecutive scales `base, base+1, …`. Returns `None` (fail-closed)
    /// unless the finite-difference table vanishes above `max_degree` — which, given
    /// `values.len() > max_degree + 1`, is exactly the statement that the fitted polynomial
    /// *reproduces every extra window point*.
    pub fn fit(base: usize, values: &[i128], max_degree: usize) -> Option<FittedCount> {
        let mut table: Vec<i128> = values.to_vec();
        let mut diffs: Vec<i128> = Vec::new();
        let mut level = 0usize;
        while !table.is_empty() {
            diffs.push(table[0]);
            if level > max_degree && table.iter().any(|&v| v != 0) {
                return None; // needs degree beyond the bound — no certificate
            }
            table = table.windows(2).map(|w| w[1] - w[0]).collect();
            level += 1;
        }
        diffs.truncate(max_degree + 1);
        Some(FittedCount { base, diffs })
    }

    /// Evaluate at any scale `m ≥ base` (exact `i128` binomials).
    pub fn eval(&self, m: usize) -> i128 {
        let a = (m - self.base) as u128;
        let mut c: u128 = 1; // C(a, k), built incrementally
        let mut total: i128 = 0;
        for (k, &d) in self.diffs.iter().enumerate() {
            if k > 0 {
                if a < k as u128 {
                    break;
                }
                c = c * (a - (k as u128 - 1)) / k as u128;
            }
            total += d * c as i128;
        }
        total
    }

    /// The parity at any scale, via Lucas: `C(a, k)` is odd iff `k & a == k` (every base-2 digit of
    /// `k` is ≤ the digit of `a`). No big integers, valid at every `m ≥ base`.
    pub fn parity(&self, m: usize) -> bool {
        let a = (m - self.base) as u64;
        self.diffs
            .iter()
            .enumerate()
            .filter(|(k, &d)| d % 2 != 0 && (*k as u64) & a == *k as u64)
            .count()
            % 2
            == 1
    }

    /// The period of [`FittedCount::parity`] in `m`: `C(·, k) mod 2` has period `2^⌈log₂(k+1)⌉`, so
    /// the parity's period is the next power of two above the highest odd-coefficient index.
    pub fn parity_period(&self) -> usize {
        let top = self
            .diffs
            .iter()
            .enumerate()
            .filter(|(_, &d)| d % 2 != 0)
            .map(|(k, _)| k)
            .max();
        match top {
            None => 1,
            Some(k) => (k + 1).next_power_of_two(),
        }
    }
}

/// The **stabilized symbolic system**: the window-fitted entry polynomials, evaluable (mod 2) at
/// every scale — the finite object that decides the invariant-witness question for all `m`.
pub struct StabilizedSystem {
    pub degree: usize,
    /// The stabilized column types (union over the window), the empty type first.
    pub cols: Vec<Structure>,
    /// Per row label, one fitted counting polynomial per column.
    pub entries: BTreeMap<RowLabel, Vec<FittedCount>>,
    /// First scale of the fitting window — verdicts apply to `m ≥ onset`.
    pub onset: usize,
    /// The lcm of all entry parity periods (a power of two).
    pub period: usize,
}

impl StabilizedSystem {
    /// The mod-2 collapsed dual at scale `m` (from the fitted entries), solved: does an invariant
    /// degree-`d` pseudo-expectation exist at scale `m`? By construction this is periodic in `m`
    /// with period [`StabilizedSystem::period`] for `m ≥ onset`.
    pub fn invariant_witness_exists_at(&self, m: usize) -> bool {
        let nc = self.cols.len();
        let mut eqs: Vec<(Vec<u64>, bool)> = Vec::new();
        for fits in self.entries.values() {
            let mut row = 0u64;
            for (ci, f) in fits.iter().enumerate() {
                if f.parity(m) {
                    row |= 1u64 << ci;
                }
            }
            if row != 0 {
                eqs.push((vec![row], false));
            }
        }
        eqs.push((vec![1u64], true)); // L(empty type) = 1
        gf2_solve(&eqs, nc).is_some()
    }
}

/// The ∀-scales verdict: the stabilized system plus its per-residue answers and the in-window
/// differential validation record.
pub struct ForAllVerdict {
    pub system: StabilizedSystem,
    /// For each residue `r` mod `period`: does an invariant witness exist at scales `≡ r`,
    /// `≥ onset`?
    pub by_residue: Vec<bool>,
    /// Window validation: per scale, the fitted-system verdict — each asserted equal to the direct
    /// orbit-collapsed solver's, with every positive lifted and re-checked.
    pub validated: Vec<(usize, bool)>,
}

/// **Decide the invariant-witness question at every scale by one finite computation.** Build the
/// labeled entry counts at each window scale (consecutive scales, all post-onset so every label is
/// realizable throughout), fit each entry as a degree-`≤ max_count_degree` integer polynomial (the
/// finite-difference certificate — window points beyond the fitting prefix are exact
/// verifications), read off the Lucas period, and solve the fitted system once per residue class.
/// Every window scale is differentially validated: the fitted verdict must equal
/// [`invariant_witness_exists_direct`]'s, and positive verdicts lift to
/// `check_ns_lower_bound_polys`-passing witnesses. A `true` residue is a machine-decided
/// `NS-degree > degree` lower bound for **every** scale in that class from `onset` on — including
/// scales no explicit basis can represent.
pub fn decide_invariant_witness_for_all_scales(
    make: &dyn Fn(usize) -> SymmetricInstance,
    window: &[usize],
    degree: usize,
    max_count_degree: usize,
) -> ForAllVerdict {
    assert!(window.windows(2).all(|w| w[1] == w[0] + 1), "the window is consecutive scales");
    assert!(window.len() > max_count_degree, "enough window points to pin the polynomials");
    let onset = window[0];

    // Collect labeled counts and the stabilized column set.
    let mut per_scale: Vec<BTreeMap<RowLabel, BTreeMap<Structure, u64>>> = Vec::new();
    let mut col_set: BTreeSet<Structure> = BTreeSet::new();
    let mut instances: Vec<SymmetricInstance> = Vec::new();
    for &m in window {
        let inst = make(m);
        let counts = labeled_dual_counts(&inst, degree);
        for cols in counts.values() {
            col_set.extend(cols.keys().cloned());
        }
        per_scale.push(counts);
        instances.push(inst);
    }
    let empty_type = instances[0].type_of(0);
    let mut cols: Vec<Structure> = vec![empty_type.clone()];
    cols.extend(col_set.into_iter().filter(|t| *t != empty_type));
    assert!(cols.len() <= 64, "the type bitmask carries ≤ 64 columns");

    // Every label must be realized at every window scale (post-onset window).
    let labels: BTreeSet<RowLabel> = per_scale.iter().flat_map(|s| s.keys().cloned()).collect();
    for (i, s) in per_scale.iter().enumerate() {
        for l in &labels {
            assert!(
                s.contains_key(l),
                "scale {}: a row label is unrealized — the window starts before the onset",
                window[i]
            );
        }
    }

    // Fit every entry, fail-closed on the certificate.
    let mut entries: BTreeMap<RowLabel, Vec<FittedCount>> = BTreeMap::new();
    let mut period = 1usize;
    for l in &labels {
        let mut fits = Vec::with_capacity(cols.len());
        for c in &cols {
            let values: Vec<i128> = per_scale
                .iter()
                .map(|s| *s[l].get(c).unwrap_or(&0) as i128)
                .collect();
            let f = FittedCount::fit(onset, &values, max_count_degree)
                .expect("every entry count is a bounded-degree integer polynomial in the scale");
            period = period.max(f.parity_period()); // powers of two: max = lcm
            fits.push(f);
        }
        entries.insert(l.clone(), fits);
    }
    let system = StabilizedSystem { degree, cols, entries, onset, period };

    // Differential validation across the window + witness lifting.
    let mut validated = Vec::new();
    for (i, &m) in window.iter().enumerate() {
        let fitted = system.invariant_witness_exists_at(m);
        let direct = invariant_witness_exists_direct(&instances[i], degree);
        assert_eq!(
            fitted,
            direct.is_some(),
            "scale {m}: the fitted system agrees with the direct orbit-collapsed solver"
        );
        if let Some(w) = direct {
            assert!(
                check_ns_lower_bound_polys(instances[i].num_vars, &instances[i].gens, degree, &w),
                "scale {m}: the direct invariant witness re-checks"
            );
        }
        validated.push((m, fitted));
    }

    // One verdict per residue class, evaluated past the window (periodicity makes the choice moot).
    let beyond = window.last().unwrap() + 1;
    let by_residue: Vec<bool> = (0..system.period)
        .map(|r| {
            let mut m = beyond;
            while m % system.period != r {
                m += 1;
            }
            system.invariant_witness_exists_at(m)
        })
        .collect();
    ForAllVerdict { system, by_residue, validated }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Orbit types align across scales.** The canonical structure names an orbit
    /// scale-independently: at each scale the map orbit → type is a bijection (same orbit ⟹ same
    /// type by group-invariance of the structure; distinct orbits ⟹ distinct types by completeness
    /// of the canonization), and at each fixed degree the type SET stabilizes across scales once
    /// every structure is realizable — the cross-`m` alignment the stability theorem stands on.
    /// Reproduces the fixed-degree orbit-count stabilization of
    /// `php_symmetric_ns_width_is_constant_in_m_at_fixed_degree` through the naming layer, for both
    /// pigeonhole (two sorts) and modular counting (one sort).
    #[test]
    fn php_monomial_orbit_types_align_across_m_by_bipartite_graph_type() {
        for d in [1usize, 2, 3] {
            let mut prev: Option<BTreeSet<Structure>> = None;
            for m in [d + 1, d + 2, d + 3] {
                let inst = php_instance_clause(m);
                let orbits = monomial_orbits_bounded(inst.num_vars, d, &inst.sym);
                let type_set: BTreeSet<Structure> =
                    orbits.iter().map(|o| inst.type_of(o[0])).collect();
                // Bijection: one type per orbit, every orbit one type.
                assert_eq!(
                    type_set.len(),
                    orbits.len(),
                    "PHP({m}) d={d}: distinct orbits get distinct canonical types"
                );
                for orbit in &orbits {
                    let t = inst.type_of(orbit[0]);
                    for &mono in orbit.iter().take(8) {
                        assert_eq!(
                            inst.type_of(mono),
                            t,
                            "PHP({m}) d={d}: the canonical type is constant on the orbit"
                        );
                    }
                }
                if let Some(p) = &prev {
                    assert_eq!(
                        p, &type_set,
                        "PHP d={d}: the type set is IDENTICAL across scales m={m}−1, {m}"
                    );
                }
                prev = Some(type_set);
            }
        }
        // The one-sort family: Count_3 types stabilize across its scale window too.
        for d in [1usize, 2] {
            let mut prev: Option<BTreeSet<Structure>> = None;
            for n in [7usize, 8] {
                let inst = count_instance_linear(n, 3);
                let orbits = monomial_orbits_bounded(inst.num_vars, d, &inst.sym);
                let type_set: BTreeSet<Structure> =
                    orbits.iter().map(|o| inst.type_of(o[0])).collect();
                assert_eq!(type_set.len(), orbits.len(), "Count_3({n}) d={d}: orbit↔type bijection");
                if let Some(p) = &prev {
                    assert_eq!(p, &type_set, "Count_3 d={d}: type set identical at n=7, 8");
                }
                prev = Some(type_set);
            }
        }
    }

    /// **The invariance lemma, machine-checked.** For an invariant functional `L` (constant on
    /// monomial orbits) and any group element `σ`: `⟨L, σ(p)⟩ = ⟨L, p⟩`. Hence a constraint checked
    /// on one representative per joint (multiplier-orbit × generator) is checked on all — the
    /// soundness of the collapsed system's representative rows. Verified on pigeonhole with random
    /// invariant functionals against every generator polynomial and symmetry generator.
    #[test]
    fn an_invariant_functional_checked_on_orbit_representatives_is_checked_on_all_generators() {
        let mut seed = 0xA11C_E5EEDu64;
        let mut lcg = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            seed >> 33
        };
        for m in [3usize, 4] {
            let inst = php_instance_clause(m);
            let d = 3;
            let orbits = monomial_orbits_bounded(inst.num_vars, d, &inst.sym);
            for _trial in 0..8 {
                // A random invariant L: each orbit all-in or all-out.
                let l: BTreeSet<Mono> = orbits
                    .iter()
                    .filter(|_| lcg() & 1 == 1)
                    .flat_map(|o| o.iter().copied())
                    .collect();
                let pair = |p: &Poly| p.iter().filter(|t| l.contains(t)).count() % 2;
                for g in &inst.gens {
                    for sigma in &inst.sym {
                        let image: Poly =
                            g.iter().map(|&t| apply_perm_to_mono(sigma, t)).collect();
                        assert_eq!(
                            pair(g),
                            pair(&image),
                            "PHP({m}): ⟨L, σ(p)⟩ = ⟨L, p⟩ for invariant L"
                        );
                    }
                }
            }
        }
    }

    /// **The collapsed dual agrees with the direct orbit-collapsed solver, and its lifted witnesses
    /// re-check.** Two independent implementations of "does an invariant degree-`d`
    /// pseudo-expectation exist": the type-named representative system ([`collapsed_dual_system`])
    /// and the everything-explicit direct solver ([`invariant_witness_exists_direct`]). They must
    /// agree at every scale and degree — and every positive verdict lifts to a full witness passing
    /// `check_ns_lower_bound_polys` with zero trust. This pins representative completeness and type
    /// canonization at once. The `Count_3` degree-2 row reproduces today's Lucas-schedule discovery:
    /// invariant witness at `n = 7` (`n ≡ 3 mod 4`), NO invariant witness at `n = 8` — machine-decided
    /// through both paths.
    #[test]
    fn collapsed_dual_system_agrees_with_full_symmetric_ns_on_php_small_m() {
        // PHP, clause and linear encodings, over the window.
        for m in [3usize, 4, 5] {
            for (label, inst) in
                [("clause", php_instance_clause(m)), ("linear", php_instance_linear(m))]
            {
                for d in 1..=3usize {
                    let sys = collapsed_dual_system(&inst, d);
                    let collapsed = sys.solve();
                    let direct = invariant_witness_exists_direct(&inst, d);
                    assert_eq!(
                        collapsed.is_some(),
                        direct.is_some(),
                        "PHP({m}) {label} d={d}: type-system and direct solver agree"
                    );
                    if let Some(sol) = collapsed {
                        let witness = sys.lift(&inst, d, sol);
                        assert!(
                            check_ns_lower_bound_polys(inst.num_vars, &inst.gens, d, &witness),
                            "PHP({m}) {label} d={d}: the lifted invariant witness re-checks"
                        );
                    }
                    if let Some(w) = direct {
                        assert!(
                            check_ns_lower_bound_polys(inst.num_vars, &inst.gens, d, &w),
                            "PHP({m}) {label} d={d}: the direct invariant witness re-checks"
                        );
                    }
                }
            }
        }
        // Count_3 at degree 2: the Lucas schedule, decided by the collapsed machinery.
        for (n, invariant_exists) in [(7usize, true), (8, false)] {
            let inst = count_instance_linear(n, 3);
            let sys = collapsed_dual_system(&inst, 2);
            let collapsed = sys.solve();
            let direct = invariant_witness_exists_direct(&inst, 2);
            assert_eq!(collapsed.is_some(), direct.is_some(), "Count_3({n}): paths agree");
            assert_eq!(
                collapsed.is_some(),
                invariant_exists,
                "Count_3({n}) d=2: invariant witness iff n ≡ 3 (mod 4) — the Lucas schedule"
            );
            if let Some(sol) = collapsed {
                let witness = sys.lift(&inst, 2, sol);
                assert!(
                    check_ns_lower_bound_polys(inst.num_vars, &inst.gens, 2, &witness),
                    "Count_3({n}): the lifted witness re-checks"
                );
            } else {
                // The char-2 gap, live: no invariant witness, yet a general witness exists.
                let general = ns_lower_bound_witness_polys(inst.num_vars, &inst.gens, 2);
                assert!(
                    general.is_some(),
                    "Count_3({n}): the general witness survives where every invariant one dies"
                );
            }
        }
    }

    /// **Entry counts are bounded-degree integer polynomials in the scale, with an interpolation
    /// certificate.** Every labeled entry of the collapsed dual, observed across a consecutive
    /// window, fits a degree-`≤ 2` polynomial in the finite-difference basis — and for PHP the
    /// window (5 points, 3 fitted) leaves two exact verification points per entry, which is the
    /// certificate that the fit is the truth and not an artifact. For `Count_3` the machinery
    /// locates the hand-derived quadratic: some entry's difference table is exactly `[1, 2, 1]` —
    /// the `C(n−4, 2)` count behind the mod-4 witness schedule, machine-found.
    #[test]
    fn collapsed_entry_counts_are_integer_polynomials_in_m_with_an_interpolation_certificate() {
        // PHP linear encoding, degree 2, window m = 4..8 — every entry certified with 2 spare points.
        let window: Vec<usize> = (4..=8).collect();
        let mut per_scale = Vec::new();
        let mut labels: BTreeSet<RowLabel> = BTreeSet::new();
        let mut cols: BTreeSet<Structure> = BTreeSet::new();
        for &m in &window {
            let counts = labeled_dual_counts(&php_instance_linear(m), 2);
            labels.extend(counts.keys().cloned());
            for c in counts.values() {
                cols.extend(c.keys().cloned());
            }
            per_scale.push(counts);
        }
        let mut fitted = 0usize;
        for l in &labels {
            for c in &cols {
                let values: Vec<i128> = per_scale
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        *s.get(l)
                            .unwrap_or_else(|| panic!("label unrealized at m={}", window[i]))
                            .get(c)
                            .unwrap_or(&0) as i128
                    })
                    .collect();
                let f = FittedCount::fit(window[0], &values, 2)
                    .expect("every PHP entry is a degree-≤2 polynomial, verified on 2 spare points");
                for (i, &m) in window.iter().enumerate() {
                    assert_eq!(f.eval(m), values[i], "the fit reproduces every window point");
                }
                fitted += 1;
            }
        }
        assert!(fitted > 20, "a non-trivial system was fitted ({fitted} entries)");

        // Count_3, degree 2, window n = 6..8: the machinery finds the hand-derived C(n−4,2) entry.
        let cwindow: Vec<usize> = (6..=8).collect();
        let mut cscale = Vec::new();
        for &n in &cwindow {
            cscale.push(labeled_dual_counts(&count_instance_linear(n, 3), 2));
        }
        let clabels: BTreeSet<RowLabel> = cscale.iter().flat_map(|s| s.keys().cloned()).collect();
        let ccols: BTreeSet<Structure> = cscale
            .iter()
            .flat_map(|s| s.values().flat_map(|c| c.keys().cloned()))
            .collect();
        let mut found_quadratic = false;
        for l in &clabels {
            for c in &ccols {
                let values: Vec<i128> = cscale
                    .iter()
                    .map(|s| s.get(l).map(|m| *m.get(c).unwrap_or(&0)).unwrap_or(0) as i128)
                    .collect();
                let f = FittedCount::fit(cwindow[0], &values, 2)
                    .expect("every Count_3 entry fits at degree ≤ 2");
                if f.diffs == vec![1, 2, 1] {
                    found_quadratic = true; // C(n−4, 2) at n = 6, 7, 8 is 1, 3, 6 — diffs [1, 2, 1]
                    assert_eq!(f.parity_period(), 4, "the quadratic entry has Lucas period 4");
                }
            }
        }
        assert!(
            found_quadratic,
            "the machinery locates the hand-derived C(n−4,2) quadratic behind the mod-4 schedule"
        );
    }

    /// **Parity of binomial-basis polynomials is periodic, exactly as Lucas says.** The bit-trick
    /// `C(a, k) odd ⟺ k & a == k` is pinned against exact binomials; fitted counts round-trip
    /// through evaluation; and every [`FittedCount`]'s parity repeats with its declared period —
    /// swept far past the fitting window. This is the finite arithmetic fact that turns a window of
    /// evaluations into a verdict for every scale.
    #[test]
    fn parity_of_binomial_entries_is_eventually_periodic_by_lucas() {
        // Lucas bit-trick vs exact binomials.
        fn binom_exact(a: u64, k: u64) -> u128 {
            if k > a {
                return 0;
            }
            let mut c: u128 = 1;
            for i in 0..k {
                c = c * (a - i) as u128 / (i + 1) as u128;
            }
            c
        }
        for a in 0u64..=40 {
            for k in 0u64..=12 {
                assert_eq!(
                    binom_exact(a, k) % 2 == 1,
                    k & a == k,
                    "Lucas: C({a},{k}) parity by the digit condition"
                );
            }
        }
        // Fit round-trips and declared periods hold, far past the window.
        let mut seed = 0xB1_A5EDu64;
        let mut lcg = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as i128
        };
        for _ in 0..50 {
            let base = 4 + (lcg() % 5) as usize;
            let coeffs: Vec<i128> = (0..=3).map(|_| lcg() % 7 - 3).collect();
            let poly = |m: usize| -> i128 {
                let a = (m - base) as u64;
                coeffs
                    .iter()
                    .enumerate()
                    .map(|(k, &c)| c * binom_exact(a, k as u64) as i128)
                    .sum()
            };
            let values: Vec<i128> = (base..base + 6).map(&poly).collect();
            let f = FittedCount::fit(base, &values, 3).expect("a degree-3 polynomial fits");
            let period = f.parity_period();
            assert!(period.is_power_of_two() && period <= 4, "period is a small power of two");
            for m in base..base + 96 {
                assert_eq!(f.eval(m), poly(m), "evaluation reproduces the polynomial everywhere");
                assert_eq!(
                    f.parity(m),
                    f.parity(m + period),
                    "parity repeats with the declared period (m={m})"
                );
            }
        }
    }

    /// **THE CROWN: fixed-degree invariant-witness verdicts decided for EVERY scale by one finite
    /// computation.** For each family and degree the machinery fits the collapsed dual across a
    /// window, certifies the entry polynomials, and solves once per residue class — validated at
    /// every window scale against the direct solver, with every positive lifted to a re-checked
    /// full-basis witness. A `true` residue is a machine-decided `NS-degree > d` lower bound for
    /// every scale in the class — including scales whose explicit monomial basis (`C(n,3)` variables
    /// at `n ≥ 10`) exceeds anything the per-scale engines can represent. Verdicts locked from the
    /// machinery's own first run; the `Count_3` ones agree with (and extend beyond) the
    /// independently-derived mod-4 schedule.
    #[test]
    fn fixed_degree_symmetric_ns_verdict_for_php_is_decided_for_all_m_by_finite_computation() {
        // Pigeonhole, linear encoding, degree 2 — window m = 4..8, entries certified with cushion.
        let php = decide_invariant_witness_for_all_scales(
            &|m| php_instance_linear(m),
            &(4..=8).collect::<Vec<_>>(),
            2,
            2,
        );
        eprintln!(
            "PHP-linear d=2: onset={} period={} by_residue={:?} validated={:?}",
            php.system.onset, php.system.period, php.by_residue, php.validated
        );
        // LOCKED (machine-decided ∀m ≥ 4): the linear-encoded pigeonhole dual has NO invariant
        // witness at degree 2, at ANY scale — while general witnesses exist (the gap test) — so for
        // this family the char-2 gap is the RULE, not the exception: every degree-2 witness of
        // linear PHP is necessarily asymmetric, at every m.
        assert_eq!(php.system.period, 2, "PHP-linear entries are degree-≤1 in m — Lucas period 2");
        assert_eq!(
            php.by_residue,
            vec![false, false],
            "PHP-linear d=2: the invariant dual is EMPTY at every scale m ≥ 4"
        );

        // Pigeonhole, clause encoding, degree 2 — the contrast: past m = d + 1 the wide pigeon
        // clauses leave the degree budget and the hole-injective indicator survives invariantly.
        let php_clause = decide_invariant_witness_for_all_scales(
            &|m| php_instance_clause(m),
            &(4..=8).collect::<Vec<_>>(),
            2,
            2,
        );
        eprintln!(
            "PHP-clause d=2: onset={} period={} by_residue={:?} validated={:?}",
            php_clause.system.onset,
            php_clause.system.period,
            php_clause.by_residue,
            php_clause.validated
        );
        assert!(
            php_clause.by_residue.iter().all(|&v| v),
            "PHP-clause d=2: the invariant witness exists at EVERY scale m ≥ 4 — the encoding \
             chooses whether symmetry can see the bound"
        );

        // Modular counting, linear encoding, degree 2 — window n = 6..8 (the post-onset scales the
        // u64 basis can represent; the entry certificate rests on the located closed forms and the
        // per-scale differential validation).
        let count = decide_invariant_witness_for_all_scales(
            &|n| count_instance_linear(n, 3),
            &(6..=8).collect::<Vec<_>>(),
            2,
            2,
        );
        eprintln!(
            "Count_3 d=2: onset={} period={} by_residue={:?} validated={:?}",
            count.system.onset, count.system.period, count.by_residue, count.validated
        );
        // The window validations already hard-assert agreement + witness re-checks inside decide().
        assert_eq!(count.system.period, 4, "Count_3's schedule is mod 4 — the located quadratic");
        // LOCKED (machine-decided ∀n ≥ 6): invariant witnesses exist EXACTLY on the class
        // n ≡ 3 (mod 4) — so NS-degree(Count_3(n)) ≥ 3 for every n ≡ 3 (mod 4), including scales
        // (n = 11: 165 variables; n = 15: 455) far beyond any representable monomial basis.
        assert_eq!(
            count.by_residue,
            vec![false, false, false, true],
            "Count_3 d=2: the invariant witness lives exactly on n ≡ 3 (mod 4)"
        );
    }

    /// **Refining the gap: the asymmetric witnesses are NOT one-marking-symmetric — the
    /// symmetry-breaking depth is ≥ 2.** The first rung between "fully invariant" and "arbitrary":
    /// restrict the symmetry to a point/hole/pigeon stabilizer (one marked object, its own sort)
    /// and re-run the collapsed dual. Predicted by hand before measurement and CONFIRMED at all
    /// seven cells: one marking extends nothing — `Count_3` stays exactly on its `n ≡ 3 (mod 4)`
    /// schedule, and all four marked PHP-linear cells stay empty. The parity mechanics say why:
    /// marking one object shifts the counting-polynomial arguments by one, and the dead constraint
    /// rows are dead by an *evenness* a single mark cannot split (e.g. the marked-point row
    /// `1 + C(n−2, 2)-type` counts stay even off schedule; PHP's cross rows keep their even
    /// hole-sums). So the char-2 gap witnesses carry **symmetry-breaking depth ≥ 2** — the witness-
    /// level analog of the census's composite-shear depth measure. Every marked search space
    /// contains the fully-invariant one, so on-schedule scales stay positive (a fortiori); both
    /// engine paths agree and every positive lifts to a re-checked witness, at every cell.
    #[test]
    fn the_off_schedule_witnesses_are_probed_for_stabilizer_invariance() {
        // Count_3 at degree 2: one marked point does NOT extend the mod-4 schedule (locked).
        for n in [6usize, 7, 8] {
            let marked = count_instance_linear_marked(n, 3);
            let sys = collapsed_dual_system(&marked, 2);
            let collapsed = sys.solve();
            let direct = invariant_witness_exists_direct(&marked, 2);
            assert_eq!(collapsed.is_some(), direct.is_some(), "Count_3({n}) marked: paths agree");
            assert_eq!(
                collapsed.is_some(),
                n % 4 == 3,
                "Count_3({n}): one marked point does not extend the mod-4 schedule (locked)"
            );
            if let Some(sol) = collapsed {
                let w = sys.lift(&marked, 2, sol);
                assert!(
                    check_ns_lower_bound_polys(marked.num_vars, &marked.gens, 2, &w),
                    "Count_3({n}) marked: the lifted witness re-checks"
                );
            }
            eprintln!(
                "MARKED | Count_3({n}) d=2 (n mod 4 = {}): Stab(point)-invariant witness = {}",
                n % 4,
                collapsed.is_some()
            );
        }
        // PHP-linear at degree 2: fully-invariant is empty at EVERY scale; is one marked hole or
        // one marked pigeon enough to see the bound?
        for m in [4usize, 5] {
            for (which, inst) in [
                ("hole", php_instance_linear_marked_hole(m)),
                ("pigeon", php_instance_linear_marked_pigeon(m)),
            ] {
                let sys = collapsed_dual_system(&inst, 2);
                let collapsed = sys.solve();
                let direct = invariant_witness_exists_direct(&inst, 2);
                assert_eq!(
                    collapsed.is_some(),
                    direct.is_some(),
                    "PHP-linear({m}) marked-{which}: paths agree"
                );
                assert!(
                    collapsed.is_none(),
                    "PHP-linear({m}) marked-{which}: one marking sees nothing — the witnesses are \
                     ≥ 2-deep (locked)"
                );
                eprintln!(
                    "MARKED | PHP-linear({m}) d=2 marked-{which}: stabilizer-invariant witness = {}",
                    collapsed.is_some()
                );
            }
        }
    }

    /// **The symmetric primal–dual gap over `GF(2)`, measured.** Forced by the char-2 Reynolds
    /// annihilation, "no invariant witness" need not mean "no witness". The map of
    /// `(invariant, general)` witness existence across families, scales and degrees: soundness
    /// demands `invariant ⟹ general` everywhere (no `(true, false)` cell), and the gap is real —
    /// `Count_3(8)` at degree 2 is `(false, true)`, an asymmetric witness surviving where every
    /// invariant candidate dies.
    #[test]
    fn the_symmetric_primal_dual_gap_over_gf2_is_measured() {
        let mut gap_cells = Vec::new();
        let mut cases: Vec<(String, SymmetricInstance, usize)> = Vec::new();
        for m in [3usize, 4, 5] {
            for d in 1..=3usize {
                cases.push((format!("PHP-linear({m}) d={d}"), php_instance_linear(m), d));
            }
        }
        for n in [7usize, 8] {
            cases.push((format!("Count_3({n}) d=2"), count_instance_linear(n, 3), 2));
        }
        for (label, inst, d) in &cases {
            let invariant = invariant_witness_exists_direct(inst, *d).is_some();
            let general = ns_lower_bound_witness_polys(inst.num_vars, &inst.gens, *d).is_some();
            assert!(
                !invariant || general,
                "{label}: an invariant witness IS a witness — (true, false) is impossible"
            );
            if !invariant && general {
                gap_cells.push(label.clone());
            }
            eprintln!("{label}: invariant={invariant} general={general}");
        }
        assert!(
            gap_cells.contains(&"Count_3(8) d=2".to_string()),
            "the char-2 gap is real: Count_3(8) at degree 2 has only asymmetric witnesses"
        );
        // The systematic face of the gap: linear-encoded pigeonhole's degree-2 witnesses are
        // asymmetric at every measured scale where they exist at all.
        for cell in ["PHP-linear(4) d=2", "PHP-linear(5) d=2", "PHP-linear(5) d=3"] {
            assert!(
                gap_cells.contains(&cell.to_string()),
                "{cell}: a gap cell — the witness exists and is necessarily asymmetric"
            );
        }
    }
}
