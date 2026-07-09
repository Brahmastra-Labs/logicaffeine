//! **Schreier–Sims: a base and strong generating set (BSGS) for a permutation group** — the non-abelian
//! generalization of Gaussian elimination, and the last rung of the algebra ladder
//! (GF(2)→GF(p)→ℤ/m linear; this is the *group* level).
//!
//! Gaussian elimination decides membership/dimension of a vector subspace by a basis under a stabilizer
//! chain of coordinate projections. Schreier–Sims does the same for a permutation group: a **base**
//! `B = (β₁,…,β_k)` is a sequence of points whose pointwise stabilizer is trivial, giving the
//! **stabilizer chain** `G = G⁽¹⁾ ≥ G⁽²⁾ ≥ … ≥ G⁽ᵏ⁺¹⁾ = {id}` with `G⁽ⁱ⁾` fixing `β₁…β_{i−1}`. A
//! **strong generating set** generates every stage. The *stabilizer chain is the symmetry break* — fix a
//! base point, descend to its stabilizer, repeat — and from it, in polynomial time:
//! - **order** `|G| = Π |Δᵢ|` (the product of the basic orbit sizes), and
//! - **membership / coset decision** by *sifting* (stripping `g` through the chain; it lies in `G` iff it
//!   sifts to the identity), the decision procedure for **coset problems over non-abelian groups** —
//!   exactly the rung the linear engines (abelian only) could not reach.
//!
//! Permutations act on the right: `xᵍ = g[x]`, so `(g·h)[x] = h[g[x]]`.

use std::collections::{BTreeMap, BTreeSet, HashMap};

/// A permutation of `{0,…,n−1}`: `p[x]` is the image of point `x`.
pub type Perm = Vec<usize>;

fn identity(n: usize) -> Perm {
    (0..n).collect()
}
fn is_identity(p: &[usize]) -> bool {
    p.iter().enumerate().all(|(i, &v)| i == v)
}
/// Right-action composition: apply `g` then `h`, so `(g·h)[x] = h[g[x]]`.
fn compose(g: &[usize], h: &[usize]) -> Perm {
    g.iter().map(|&x| h[x]).collect()
}
fn invert(g: &[usize]) -> Perm {
    let mut inv = vec![0usize; g.len()];
    for (x, &gx) in g.iter().enumerate() {
        inv[gx] = x;
    }
    inv
}

/// The basic orbit of `base[level]` under `Gⁱ` (the strong generators fixing `base[0..level]`), with a
/// transversal: `trans[δ]` is a permutation in `Gⁱ` carrying `base[level]` to `δ`.
fn orbit_transversal(base: &[usize], strong: &[Perm], level: usize) -> HashMap<usize, Perm> {
    let degree = strong.first().map(|p| p.len()).unwrap_or(base.len());
    let stab: Vec<&Perm> =
        strong.iter().filter(|g| (0..level).all(|j| g[base[j]] == base[j])).collect();
    let mut trans: HashMap<usize, Perm> = HashMap::new();
    trans.insert(base[level], identity(degree));
    let mut queue = vec![base[level]];
    while let Some(p) = queue.pop() {
        let up = trans[&p].clone();
        for s in &stab {
            let q = s[p];
            if !trans.contains_key(&q) {
                trans.insert(q, compose(&up, s)); // base[level] → p → q
                queue.push(q);
            }
        }
    }
    trans
}

/// Sift `g` through the chain: at each level send `g` into the stabilizer of `base[i]` by right-dividing
/// the transversal element. Returns the residue and the level reached. `g ∈ ⟨strong⟩` iff the residue is
/// the identity (and the full depth was reached).
fn sift(base: &[usize], strong: &[Perm], mut g: Perm) -> (Perm, usize) {
    for (i, &beta) in base.iter().enumerate() {
        let trans = orbit_transversal(base, strong, i);
        let img = g[beta];
        match trans.get(&img) {
            None => return (g, i),
            Some(t) => g = compose(&g, &invert(t)), // now fixes base[i]
        }
    }
    (g, base.len())
}

/// Add `g ∈ G` to the (base, strong) data: sift it, and if the residue is non-trivial it is a new strong
/// generator — extend the base if the residue fixes the whole current base. Returns whether it grew.
fn extend_with(base: &mut Vec<usize>, strong: &mut Vec<Perm>, g: Perm) -> bool {
    let (res, lvl) = sift(base, strong, g);
    if is_identity(&res) {
        return false;
    }
    if lvl == base.len() {
        let moved = (0..res.len()).find(|&x| res[x] != x).expect("a non-identity moves a point");
        base.push(moved);
    }
    strong.push(res);
    true
}

/// The orbits of `{0,…,degree−1}` under `⟨generators⟩`, as a partition (each orbit sorted ascending,
/// orbits ordered by least element). Needs only the generators — a BFS, independent of the BSGS.
pub fn orbits(degree: usize, generators: &[Perm]) -> Vec<Vec<usize>> {
    let mut seen = vec![false; degree];
    let mut out = Vec::new();
    for start in 0..degree {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut orbit = vec![start];
        let mut i = 0;
        while i < orbit.len() {
            let p = orbit[i];
            i += 1;
            for g in generators {
                let q = g[p];
                if !seen[q] {
                    seen[q] = true;
                    orbit.push(q);
                }
            }
        }
        orbit.sort_unstable();
        out.push(orbit);
    }
    out
}

fn uf_find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

/// The minimal block (G-congruence class) containing both `alpha` and `beta`, returned as a block-id per
/// point (Atkinson's algorithm): merge `α,β`, then whenever two points share a block so must their images
/// under every generator — close under the generators with union–find. The class of `alpha` is the
/// minimal block containing the pair.
fn block_containing(degree: usize, gens: &[Perm], alpha: usize, beta: usize) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..degree).collect();
    let mut queue: Vec<(usize, usize)> = Vec::new();
    let (ra, rb) = (uf_find(&mut parent, alpha), uf_find(&mut parent, beta));
    if ra != rb {
        parent[ra] = rb;
        queue.push((alpha, beta));
    }
    while let Some((x, y)) = queue.pop() {
        for g in gens {
            let (gx, gy) = (g[x], g[y]);
            let (rx, ry) = (uf_find(&mut parent, gx), uf_find(&mut parent, gy));
            if rx != ry {
                parent[rx] = ry;
                queue.push((gx, gy));
            }
        }
    }
    (0..degree).map(|x| uf_find(&mut parent, x)).collect()
}

/// The minimal non-trivial **block system** of a TRANSITIVE permutation group — the finest `G`-invariant
/// partition into equal-size blocks bigger than a point and smaller than the whole set — or `None` if the
/// group is **primitive** (only the trivial partitions are invariant) or not transitive. Imprimitivity is
/// the symmetry's internal structure: a grid symmetry decomposes into its rows, a cyclic group of
/// composite order into cosets. (Atkinson's algorithm over each pair `{0, β}`.)
pub fn minimal_block_system(degree: usize, gens: &[Perm]) -> Option<Vec<Vec<usize>>> {
    if degree < 2 || orbits(degree, gens).len() != 1 {
        return None; // primitivity is a property of transitive groups only
    }
    let mut best: Option<Vec<usize>> = None;
    let mut best_size = degree;
    for beta in 1..degree {
        let ids = block_containing(degree, gens, 0, beta);
        let size = ids.iter().filter(|&&b| b == ids[0]).count();
        if 1 < size && size < degree && size < best_size {
            best_size = size;
            best = Some(ids);
        }
    }
    best.map(|ids| {
        let mut by_block: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (x, &b) in ids.iter().enumerate() {
            by_block.entry(b).or_default().push(x);
        }
        by_block.into_values().collect()
    })
}

/// Is the group **primitive** — transitive with no non-trivial block system? Primitive groups are the
/// indecomposable "atoms" of permutation-group structure; imprimitive ones split into blocks
/// ([`minimal_block_system`]).
pub fn is_primitive(degree: usize, gens: &[Perm]) -> bool {
    degree >= 2 && orbits(degree, gens).len() == 1 && minimal_block_system(degree, gens).is_none()
}

/// The **orbitals** — the orbits of the group on ORDERED PAIRS `(i, j)` under the action
/// `g·(i,j) = (g[i], g[j])`. The diagonal `{(i,i)}` is always one orbital; for a transitive group the
/// non-diagonal orbitals are its "relation classes" (its association scheme). The count is the group's
/// [`rank`]. One level finer than the point-orbits ([`orbits`]).
pub fn orbitals(degree: usize, gens: &[Perm]) -> Vec<Vec<(usize, usize)>> {
    let mut seen = vec![false; degree * degree];
    let idx = |i: usize, j: usize| i * degree + j;
    let mut out = Vec::new();
    for i in 0..degree {
        for j in 0..degree {
            if seen[idx(i, j)] {
                continue;
            }
            seen[idx(i, j)] = true;
            let mut orbit = vec![(i, j)];
            let mut k = 0;
            while k < orbit.len() {
                let (a, b) = orbit[k];
                k += 1;
                for g in gens {
                    let (ga, gb) = (g[a], g[b]);
                    if !seen[idx(ga, gb)] {
                        seen[idx(ga, gb)] = true;
                        orbit.push((ga, gb));
                    }
                }
            }
            out.push(orbit);
        }
    }
    out
}

/// The **rank** of the group: the number of orbitals (orbits on ordered pairs). A transitive group has
/// rank `2` iff it is 2-transitive; a regular group has rank equal to its degree.
pub fn rank(degree: usize, gens: &[Perm]) -> usize {
    orbitals(degree, gens).len()
}

/// Are all `degree` points connected by the undirected graph whose edges are `orbital`'s pairs?
fn orbital_graph_connected(degree: usize, orbital: &[(usize, usize)]) -> bool {
    let mut parent: Vec<usize> = (0..degree).collect();
    for &(i, j) in orbital {
        let (ri, rj) = (uf_find(&mut parent, i), uf_find(&mut parent, j));
        parent[ri] = rj;
    }
    let r0 = uf_find(&mut parent, 0);
    (0..degree).all(|v| uf_find(&mut parent, v) == r0)
}

/// Primitivity via **Higman's theorem**: a transitive group is primitive iff every non-diagonal orbital
/// graph is connected. An independent route to [`is_primitive`]; when a non-diagonal orbital graph is
/// disconnected, its connected components are a block system. (Returns `false` for an intransitive group.)
pub fn is_primitive_via_orbitals(degree: usize, gens: &[Perm]) -> bool {
    if degree < 2 || orbits(degree, gens).len() != 1 {
        return false;
    }
    orbitals(degree, gens)
        .iter()
        .filter(|orb| orb.iter().any(|&(i, j)| i != j)) // skip the diagonal
        .all(|orb| orbital_graph_connected(degree, orb))
}

/// Every ordered `k`-tuple of DISTINCT points from `0..degree`.
fn distinct_tuples(degree: usize, k: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut cur = Vec::with_capacity(k);
    let mut used = vec![false; degree];
    fn rec(degree: usize, k: usize, cur: &mut Vec<usize>, used: &mut [bool], out: &mut Vec<Vec<usize>>) {
        if cur.len() == k {
            out.push(cur.clone());
            return;
        }
        for x in 0..degree {
            if !used[x] {
                used[x] = true;
                cur.push(x);
                rec(degree, k, cur, used, out);
                cur.pop();
                used[x] = false;
            }
        }
    }
    rec(degree, k, &mut cur, &mut used, &mut out);
    out
}

/// The orbits of the group on ordered `k`-tuples of distinct points (`g·(t₁,…,t_k) = (g[t₁],…,g[t_k])`).
/// `k = 1` is the point-orbits ([`orbits`]); `k = 2` (on distinct pairs) refines [`orbitals`]; in general
/// the group is `k`-transitive iff this is a single orbit — the rungs of the transitivity ladder.
pub fn orbits_on_tuples(degree: usize, gens: &[Perm], k: usize) -> Vec<Vec<Vec<usize>>> {
    if k == 0 || k > degree {
        return Vec::new();
    }
    let tuples = distinct_tuples(degree, k);
    let index: HashMap<Vec<usize>, usize> =
        tuples.iter().enumerate().map(|(i, t)| (t.clone(), i)).collect();
    let mut seen = vec![false; tuples.len()];
    let mut out = Vec::new();
    for start in 0..tuples.len() {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut orbit = vec![tuples[start].clone()];
        let mut i = 0;
        while i < orbit.len() {
            let cur = orbit[i].clone();
            i += 1;
            for g in gens {
                let img: Vec<usize> = cur.iter().map(|&x| g[x]).collect();
                let idx = index[&img];
                if !seen[idx] {
                    seen[idx] = true;
                    orbit.push(img);
                }
            }
        }
        out.push(orbit);
    }
    out
}

/// The **transitivity degree**: the largest `t ≤ max_t` for which the group is transitive on ordered
/// `t`-tuples of distinct points (`1` = transitive, `2` = 2-transitive, …). `0` if intransitive. Capped at
/// `max_t` because the `t`-tuple space grows as `degree^t`. `Sₙ` is `n`-transitive; a regular group is only
/// `1`-transitive.
pub fn transitivity_degree(degree: usize, gens: &[Perm], max_t: usize) -> usize {
    let mut t = 0;
    for k in 1..=max_t.min(degree) {
        if orbits_on_tuples(degree, gens, k).len() == 1 {
            t = k;
        } else {
            break; // k-transitive ⟹ (k-1)-transitive, so the first failure is the ceiling
        }
    }
    t
}

/// The commutator `[g, h] = g⁻¹ h⁻¹ g h`.
fn commutator(g: &[usize], h: &[usize]) -> Perm {
    compose(&compose(&invert(g), &invert(h)), &compose(g, h))
}

/// Generators of the **normal closure** of `⟨sub⟩` inside `⟨gens⟩`: close `sub` under conjugation by the
/// generators and their inverses (which generates conjugation by the whole group) until the generated
/// group stops growing.
fn normal_closure(degree: usize, sub: &[Perm], gens: &[Perm]) -> Vec<Perm> {
    let mut closure: Vec<Perm> = sub.iter().filter(|p| !is_identity(p)).cloned().collect();
    if closure.is_empty() {
        return closure;
    }
    let mut bsgs = schreier_sims(degree, &closure);
    let mut i = 0;
    while i < closure.len() {
        let s = closure[i].clone();
        i += 1;
        for g in gens {
            for conj in [compose(&compose(&invert(g), &s), g), compose(&compose(g, &s), &invert(g))] {
                if !is_identity(&conj) && !bsgs.contains(&conj) {
                    closure.push(conj);
                    bsgs = schreier_sims(degree, &closure);
                }
            }
        }
    }
    closure
}

/// Generators of the **commutator subgroup** `[A, B]` — the normal closure (in `⟨gens_a⟩`) of the
/// commutators `[a, b]` for generators `a` of `A`, `b` of `B`. `[G, G]` is the derived subgroup; `[G, γ]`
/// is a step of the lower central series.
fn commutator_subgroup(degree: usize, gens_a: &[Perm], gens_b: &[Perm]) -> Vec<Perm> {
    let mut comms = Vec::new();
    for a in gens_a {
        for b in gens_b {
            let c = commutator(a, b);
            if !is_identity(&c) {
                comms.push(c);
            }
        }
    }
    normal_closure(degree, &comms, gens_a)
}

/// Generators of the **derived (commutator) subgroup** `[G, G]` — the normal closure of the commutators of
/// the generators. Always normal; `G / [G, G]` is the abelianisation, and `[G, G]` is trivial iff `G` is
/// abelian.
pub fn derived_subgroup(degree: usize, gens: &[Perm]) -> Vec<Perm> {
    commutator_subgroup(degree, gens, gens)
}

/// Is the group **nilpotent**? (Its lower central series reaches the trivial group.) Strictly stronger than
/// solvability — every `p`-group is nilpotent, but `S₃` (solvable) is not.
pub fn is_nilpotent(degree: usize, gens: &[Perm]) -> bool {
    nilpotency_class(degree, gens).is_some()
}

/// Is the group **abelian**? (Its generators pairwise commute.)
pub fn is_abelian(_degree: usize, gens: &[Perm]) -> bool {
    gens.iter().all(|g| gens.iter().all(|h| compose(g, h) == compose(h, g)))
}

/// The **derived length** (solvability class): the number of steps the derived series `G ⊵ G' ⊵ G'' ⊵ …`
/// takes to reach the trivial group, or `None` if it never does (`G` is unsolvable). `0` is the trivial
/// group, `1` a non-trivial abelian group, `2` for `S₃`, `3` for `S₄`.
pub fn derived_length(degree: usize, gens: &[Perm]) -> Option<usize> {
    let mut cur: Vec<Perm> = gens.to_vec();
    let mut len = 0;
    loop {
        let order = schreier_sims(degree, &cur).order();
        if order == 1 {
            return Some(len);
        }
        let d = derived_subgroup(degree, &cur);
        if schreier_sims(degree, &d).order() == order {
            return None; // G' = G ⇒ the series never descends to 1
        }
        cur = d;
        len += 1;
    }
}

/// Is the group **solvable**? (Its derived series reaches the trivial group.)
pub fn is_solvable(degree: usize, gens: &[Perm]) -> bool {
    derived_length(degree, gens).is_some()
}

/// The **nilpotency class**: the number of steps the lower central series `γ₁ = G`, `γ_{k+1} = [G, γ_k]`
/// takes to reach the trivial group, or `None` if it never does (`G` is not nilpotent). `0` is trivial,
/// `1` abelian, `2` for `D₄`.
pub fn nilpotency_class(degree: usize, gens: &[Perm]) -> Option<usize> {
    let mut gamma: Vec<Perm> = gens.to_vec();
    let mut class = 0;
    loop {
        let order = schreier_sims(degree, &gamma).order();
        if order == 1 {
            return Some(class);
        }
        let next = commutator_subgroup(degree, gens, &gamma);
        if schreier_sims(degree, &next).order() == order {
            return None; // the series stalls above the identity
        }
        gamma = next;
        class += 1;
    }
}

/// The **conjugacy classes** of the group — the partition of its elements by `g ~ x⁻¹gx`. The number of
/// classes equals the number of irreducible representations (the bridge to character theory); the
/// singleton classes are exactly the centre `Z(G)`. Requires enumerating the group, so it returns `None`
/// when `|G| > cap`.
pub fn conjugacy_classes(degree: usize, gens: &[Perm], cap: usize) -> Option<Vec<Vec<Perm>>> {
    let elements = schreier_sims(degree, gens).elements(cap)?;
    let mut remaining: BTreeSet<Perm> = elements.iter().cloned().collect();
    let mut classes = Vec::new();
    while let Some(g) = remaining.iter().next().cloned() {
        let mut class: BTreeSet<Perm> = BTreeSet::new();
        for x in &elements {
            class.insert(compose(&compose(&invert(x), &g), x)); // x⁻¹ g x
        }
        for c in &class {
            remaining.remove(c);
        }
        classes.push(class.into_iter().collect::<Vec<_>>());
    }
    Some(classes)
}

/// The order of the **centre** `Z(G)` — the elements commuting with all of `G`, which are exactly those in
/// singleton conjugacy classes. `None` when `|G| > cap`.
pub fn center_order(degree: usize, gens: &[Perm], cap: usize) -> Option<u128> {
    conjugacy_classes(degree, gens, cap)
        .map(|classes| classes.iter().filter(|c| c.len() == 1).count() as u128)
}

/// The **order of a single permutation**: the least `k ≥ 1` with `gᵏ = id`.
fn element_order(g: &[usize]) -> usize {
    if is_identity(g) {
        return 1;
    }
    let mut p = compose(g, g);
    let mut k = 2;
    while !is_identity(&p) {
        p = compose(&p, g);
        k += 1;
    }
    k
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

fn lcm(a: u128, b: u128) -> u128 {
    if a == 0 || b == 0 {
        0
    } else {
        a / gcd(a, b) * b
    }
}

/// The **order spectrum** — the set of distinct orders of the group's elements. `None` when `|G| > cap`.
pub fn element_orders(degree: usize, gens: &[Perm], cap: usize) -> Option<BTreeSet<usize>> {
    let elements = schreier_sims(degree, gens).elements(cap)?;
    Some(elements.iter().map(|g| element_order(g)).collect())
}

/// The **exponent** of the group — the least common multiple of all element orders, i.e. the smallest `e`
/// with `gᵉ = id` for every `g`. `None` when `|G| > cap`.
pub fn exponent(degree: usize, gens: &[Perm], cap: usize) -> Option<u128> {
    let elements = schreier_sims(degree, gens).elements(cap)?;
    Some(elements.iter().fold(1u128, |e, g| lcm(e, element_order(g) as u128)))
}

/// The orders `[|Z₀|, |Z₁|, …]` of the **upper central series**, up to the hypercentre. `Z₀ = {id}` and
/// `Z_{i+1} = { g : [g, x] ∈ Z_i for all x }` (the preimage of the centre of `G/Z_i`). The series ascends
/// to `|G|` iff `G` is nilpotent. `None` when `|G| > cap`.
pub fn upper_central_series(degree: usize, gens: &[Perm], cap: usize) -> Option<Vec<u128>> {
    let elements = schreier_sims(degree, gens).elements(cap)?;
    let mut z: BTreeSet<Perm> = BTreeSet::from([identity(degree)]);
    let mut orders = vec![1u128];
    loop {
        let next: BTreeSet<Perm> = elements
            .iter()
            .filter(|g| elements.iter().all(|x| z.contains(&commutator(g, x))))
            .cloned()
            .collect();
        if next.len() == z.len() {
            break; // stabilised at the hypercentre
        }
        orders.push(next.len() as u128);
        z = next;
    }
    Some(orders)
}

/// The length of the upper central series when it reaches `G` (the nilpotency class) — `None` if it stalls
/// below `G` (the group is not nilpotent) or `|G| > cap`. Equals [`nilpotency_class`] for nilpotent groups,
/// an independent route to the same number.
pub fn upper_central_length(degree: usize, gens: &[Perm], cap: usize) -> Option<usize> {
    let orders = upper_central_series(degree, gens, cap)?;
    (orders.last() == Some(&schreier_sims(degree, gens).order())).then_some(orders.len() - 1)
}

/// The **cycle type** of a permutation — the sorted multiset of its cycle lengths (fixed points are
/// 1-cycles). Its length is the number of cycles.
fn cycle_type(g: &[usize]) -> Vec<usize> {
    let mut seen = vec![false; g.len()];
    let mut lengths = Vec::new();
    for start in 0..g.len() {
        if seen[start] {
            continue;
        }
        let mut len = 0;
        let mut x = start;
        while !seen[x] {
            seen[x] = true;
            x = g[x];
            len += 1;
        }
        lengths.push(len);
    }
    lengths.sort_unstable();
    lengths
}

/// The **cycle index** data — the distribution of cycle types over the group, mapping each cycle type to
/// the number of elements with it. Dividing by `|G|` gives the cycle index polynomial, the engine of Pólya
/// enumeration. `None` when `|G| > cap`.
pub fn cycle_index(degree: usize, gens: &[Perm], cap: usize) -> Option<BTreeMap<Vec<usize>, u128>> {
    let elements = schreier_sims(degree, gens).elements(cap)?;
    let mut dist: BTreeMap<Vec<usize>, u128> = BTreeMap::new();
    for g in &elements {
        *dist.entry(cycle_type(g)).or_insert(0) += 1;
    }
    Some(dist)
}

/// **Pólya / Burnside count** — the number of ways to colour the `degree` points with `m` colours up to the
/// group action: `(1/|G|) Σ_g m^{#cycles(g)}`. With `m = 2` this is the number of distinct `{0,1}`
/// assignments to the points modulo symmetry — the symmetry-reduced size of the assignment space. `None`
/// when `|G| > cap`.
pub fn polya_count(degree: usize, gens: &[Perm], m: usize, cap: usize) -> Option<u128> {
    let elements = schreier_sims(degree, gens).elements(cap)?;
    let order = elements.len() as u128;
    let total: u128 = elements.iter().map(|g| (m as u128).pow(cycle_type(g).len() as u32)).sum();
    Some(total / order)
}

/// The **pattern inventory** (weighted Pólya, two colours) — `coeff[w]` is the number of distinct `{0,1}`
/// assignments to the points with exactly `w` ones, up to the group. Obtained by substituting
/// `aₖ → (1 + zᵏ)` into the cycle index: a `k`-cycle is either all-0 or all-1, contributing `1 + zᵏ`. The
/// coefficients sum to [`polya_count`]`(…, 2, …)`. `None` when `|G| > cap`.
pub fn pattern_inventory(degree: usize, gens: &[Perm], cap: usize) -> Option<Vec<u128>> {
    let elements = schreier_sims(degree, gens).elements(cap)?;
    let order = elements.len() as u128;
    let mut total = vec![0u128; degree + 1];
    for g in &elements {
        // Π over cycles of (1 + z^len): a polynomial of total degree = Σ len = degree.
        let mut poly = vec![0u128; degree + 1];
        poly[0] = 1;
        for &len in &cycle_type(g) {
            let prev = poly.clone();
            for i in 0..=degree {
                poly[i] = prev[i] + if i >= len { prev[i - len] } else { 0 };
            }
        }
        for i in 0..=degree {
            total[i] += poly[i];
        }
    }
    Some(total.iter().map(|&c| c / order).collect())
}

/// The **abelianisation** `G / [G, G]` — the largest abelian quotient — as `(order, exponent)`. The order
/// is `|G| / |[G, G]|`; the exponent (lcm of coset orders) and whether it equals the order — i.e. whether
/// `Gᵃᵇ` is cyclic — are the new structural content. `None` when `|G| > cap`.
pub fn abelianization(degree: usize, gens: &[Perm], cap: usize) -> Option<(u128, u128)> {
    let elements = schreier_sims(degree, gens).elements(cap)?;
    let derived: BTreeSet<Perm> =
        schreier_sims(degree, &derived_subgroup(degree, gens)).elements(cap)?.into_iter().collect();
    // Canonical coset representative g·[G,G] = the lexicographically least element of the coset.
    let coset_rep = |g: &Perm| -> Perm { derived.iter().map(|x| compose(g, x)).min().unwrap() };
    let cosets: BTreeSet<Perm> = elements.iter().map(|g| coset_rep(g)).collect();
    let order = cosets.len() as u128;
    // Order of a coset in the quotient: least k with rᵏ ∈ [G,G].
    let coset_order = |r: &Perm| -> usize {
        let mut p = r.clone();
        let mut k = 1;
        while !derived.contains(&p) {
            p = compose(&p, r);
            k += 1;
        }
        k
    };
    let exponent = cosets.iter().fold(1u128, |e, r| lcm(e, coset_order(r) as u128));
    Some((order, exponent))
}

/// The subgroup generated by `seed` — its closure under composition (with the identity).
fn subgroup_closure(degree: usize, seed: &BTreeSet<Perm>) -> BTreeSet<Perm> {
    let mut set = seed.clone();
    set.insert(identity(degree));
    loop {
        let snapshot: Vec<Perm> = set.iter().cloned().collect();
        let before = set.len();
        for a in &snapshot {
            for b in &snapshot {
                set.insert(compose(a, b));
            }
        }
        if set.len() == before {
            break;
        }
    }
    set
}

/// The **number of subgroups** of the group — the size of its subgroup lattice. Found by breadth-first
/// search: extend each known subgroup by one outside element and close, deduplicating. `None` when
/// `|G| > cap` (the enumeration, and the lattice walk, are exponential in the worst case). Classic counts:
/// `C₄ → 3`, `S₃ → 6`, `V₄ → 5`, `S₄ → 30`.
fn all_subgroups(degree: usize, gens: &[Perm], cap: usize) -> Option<BTreeSet<BTreeSet<Perm>>> {
    let elements = schreier_sims(degree, gens).elements(cap)?;
    let trivial: BTreeSet<Perm> = BTreeSet::from([identity(degree)]);
    let mut subgroups: BTreeSet<BTreeSet<Perm>> = BTreeSet::from([trivial.clone()]);
    let mut queue = vec![trivial];
    while let Some(h) = queue.pop() {
        for g in &elements {
            if h.contains(g) {
                continue;
            }
            let mut seed = h.clone();
            seed.insert(g.clone());
            let sub = subgroup_closure(degree, &seed);
            if subgroups.insert(sub.clone()) {
                queue.push(sub);
            }
        }
    }
    Some(subgroups)
}

pub fn subgroup_count(degree: usize, gens: &[Perm], cap: usize) -> Option<usize> {
    all_subgroups(degree, gens, cap).map(|s| s.len())
}

/// Is the subgroup (element set) `h` normal — closed under conjugation by the generators?
fn is_normal_set(h: &BTreeSet<Perm>, gens: &[Perm]) -> bool {
    gens.iter().all(|g| h.iter().all(|x| h.contains(&compose(&compose(&invert(g), x), g))))
}

/// A **maximal proper normal subgroup** (the largest-order one) as an element list — so the quotient
/// `G/N` is simple. `None` if there is none in range.
fn maximal_normal_subgroup(degree: usize, gens: &[Perm], cap: usize) -> Option<Vec<Perm>> {
    let order = schreier_sims(degree, gens).order();
    let subgroups = all_subgroups(degree, gens, cap)?;
    subgroups
        .iter()
        .filter(|h| (h.len() as u128) < order && is_normal_set(h, gens))
        .max_by_key(|h| h.len())
        .map(|h| h.iter().cloned().collect())
}

/// The **composition factors** of the group as the sorted multiset of their orders — the Jordan–Hölder
/// decomposition into simple groups (the "prime factorisation" of the group). Their product is `|G|`; for a
/// solvable group every factor is a prime (cyclic `Cₚ`), and a non-abelian simple factor (e.g. `A₅`, order
/// 60) marks unsolvability. `None` when the group is out of range.
pub fn composition_factor_orders(degree: usize, gens: &[Perm], cap: usize) -> Option<Vec<u128>> {
    let order = schreier_sims(degree, gens).order();
    if order == 1 {
        return Some(Vec::new());
    }
    if is_simple(degree, gens, cap)? {
        return Some(vec![order]);
    }
    let n = maximal_normal_subgroup(degree, gens, cap)?;
    let n_order = schreier_sims(degree, &n).order();
    let mut factors = composition_factor_orders(degree, &n, cap)?;
    factors.push(order / n_order); // |G/N| is simple
    factors.sort_unstable();
    Some(factors)
}

/// The distinct prime divisors of `n`.
fn distinct_primes(mut n: u128) -> Vec<u128> {
    let mut primes = Vec::new();
    let mut p = 2u128;
    while p * p <= n {
        if n % p == 0 {
            primes.push(p);
            while n % p == 0 {
                n /= p;
            }
        }
        p += 1;
    }
    if n > 1 {
        primes.push(n);
    }
    primes
}

/// The full `p`-power `pᵃ` with `pᵃ ∥ n` (i.e. `pᵃ | n` but `pᵃ⁺¹ ∤ n`).
fn prime_power_part(n: u128, p: u128) -> u128 {
    let mut pa = 1;
    let mut m = n;
    while m % p == 0 {
        pa *= p;
        m /= p;
    }
    pa
}

/// The **Sylow structure** — for each prime `p ∣ |G|`, the number `n_p` of Sylow `p`-subgroups (the
/// subgroups of maximal `p`-power order `pᵃ ∥ |G|`; all such subgroups are conjugate, so counting them
/// counts the Sylow subgroups). Returned as `(p, n_p)` pairs sorted by `p`. Sylow's theorems guarantee
/// `n_p ≡ 1 (mod p)` and `n_p ∣ |G|/pᵃ`. `None` when the subgroup lattice is out of range.
pub fn sylow_counts(degree: usize, gens: &[Perm], cap: usize) -> Option<Vec<(u128, usize)>> {
    let order = schreier_sims(degree, gens).order();
    let subgroups = all_subgroups(degree, gens, cap)?;
    Some(
        distinct_primes(order)
            .into_iter()
            .map(|p| {
                let pa = prime_power_part(order, p);
                let n_p = subgroups.iter().filter(|h| h.len() as u128 == pa).count();
                (p, n_p)
            })
            .collect(),
    )
}

/// Map every group element to the index of its conjugacy class.
fn class_index_map(classes: &[Vec<Perm>]) -> BTreeMap<Perm, usize> {
    let mut idx = BTreeMap::new();
    for (i, class) in classes.iter().enumerate() {
        for g in class {
            idx.insert(g.clone(), i);
        }
    }
    idx
}

/// The **class-algebra structure constants** `a[i][j][k] = #{ x ∈ Cᵢ : x⁻¹·z ∈ Cⱼ }` for `z` a fixed
/// representative of class `Cₖ` (independent of the choice of `z`). These are the multiplication
/// coefficients of the centre of the group algebra — `Cᵢ·Cⱼ = Σₖ a[i][j][k]·Cₖ` — and the foundation of
/// the Burnside–Dixon character-table algorithm. They satisfy `Σₖ a[i][j][k]·|Cₖ| = |Cᵢ|·|Cⱼ|`. `None`
/// when `|G| > cap`.
pub fn class_multiplication_coefficients(
    degree: usize,
    gens: &[Perm],
    cap: usize,
) -> Option<Vec<Vec<Vec<u128>>>> {
    let classes = conjugacy_classes(degree, gens, cap)?;
    let idx = class_index_map(&classes);
    let k = classes.len();
    let mut a = vec![vec![vec![0u128; k]; k]; k];
    for (kk, class_k) in classes.iter().enumerate() {
        let z = &class_k[0];
        for (i, class_i) in classes.iter().enumerate() {
            for x in class_i {
                let j = idx[&compose(&invert(x), z)]; // class of x⁻¹·z
                a[i][j][kk] += 1;
            }
        }
    }
    Some(a)
}

/// The number of **real conjugacy classes** — those closed under inversion (`C = C⁻¹`). By Burnside's
/// theorem this equals the number of real-valued irreducible characters. `None` when `|G| > cap`.
pub fn real_class_count(degree: usize, gens: &[Perm], cap: usize) -> Option<usize> {
    let classes = conjugacy_classes(degree, gens, cap)?;
    let idx = class_index_map(&classes);
    Some(classes.iter().enumerate().filter(|(i, c)| idx[&invert(&c[0])] == *i).count())
}

/// `g^t` under the right-action composition (`(g·h)[x] = h[g[x]]`), by repeated squaring.
fn perm_pow(g: &[usize], mut t: usize) -> Perm {
    let mut result = identity(g.len());
    let mut base = g.to_vec();
    while t > 0 {
        if t & 1 == 1 {
            result = compose(&result, &base);
        }
        base = compose(&base, &base);
        t >>= 1;
    }
    result
}

/// The **Galois orbits on conjugacy classes**. The Galois group `Gal(ℚ(ζ_e)/ℚ) ≅ (ℤ/e)*` (`e` = the group
/// exponent) acts on classes by `C ↦ C^t` (the class of `g^t`), for every `t` coprime to `e` — this is the
/// action dual to the Galois action `σ_t(χ)(g) = χ(g^t)` on irreducible characters. Two classes share an
/// orbit iff they are *algebraically conjugate* (`g ~ g^t` for some coprime `t`). `None` when `|G| > cap`.
pub fn galois_class_orbits(degree: usize, gens: &[Perm], cap: usize) -> Option<Vec<Vec<usize>>> {
    let classes = conjugacy_classes(degree, gens, cap)?;
    let idx = class_index_map(&classes);
    let e = exponent(degree, gens, cap)? as usize;
    let k = classes.len();
    let mut parent: Vec<usize> = (0..k).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for t in 1..e.max(2) {
        if gcd(t as u128, e as u128) != 1 {
            continue;
        }
        for r in 0..k {
            let img = idx[&perm_pow(&classes[r][0], t)];
            let (a, b) = (find(&mut parent, r), find(&mut parent, img));
            parent[a] = b;
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for r in 0..k {
        let root = find(&mut parent, r);
        groups.entry(root).or_default().push(r);
    }
    Some(groups.into_values().collect())
}

/// The number of **rational conjugacy classes** — classes `C` fixed by the *whole* Galois group
/// (`g ~ g^t` for every `t` coprime to `ord(g)`), i.e. the singleton [`galois_class_orbits`]. By Burnside's
/// rationality theorem this equals the number of rational-valued irreducible characters. Strictly refines
/// [`real_class_count`] (real = closed under the single element `t = −1`): rational ⟹ real, and the counts
/// differ exactly when a character is real but irrational (e.g. `A₅`'s golden-ratio degree-3 pair).
/// `None` when `|G| > cap`.
pub fn rational_class_count(degree: usize, gens: &[Perm], cap: usize) -> Option<usize> {
    Some(galois_class_orbits(degree, gens, cap)?.iter().filter(|o| o.len() == 1).count())
}

/// The order of the **automorphism group** `Aut(G)` of `G = ⟨gens⟩` — the symmetries of the group itself
/// (bijections `G → G` preserving multiplication, `φ(xy) = φ(x)φ(y)`). An automorphism is determined by the
/// images of a generating set, so the search ranges over candidate images (each generator must map to an
/// element of the same order, a necessary condition) and accepts those that extend to a consistent,
/// bijective homomorphism. `None` when `|G| > cap` or the candidate search would exceed its budget. Classic:
/// `|Aut(Cₙ)| = φ(n)`, `|Aut(Sₙ)| = n!` (n≠6), `|Aut(V₄)| = 6`, `|Aut(D₄)| = 8`, `|Aut(Q₈)| = 24`.
pub fn automorphism_group_order(degree: usize, gens: &[Perm], cap: usize) -> Option<u128> {
    let seed: BTreeSet<Perm> = gens.iter().cloned().collect();
    let elements: Vec<Perm> = subgroup_closure(degree, &seed).into_iter().collect();
    let n = elements.len();
    if n > cap {
        return None;
    }
    let idx: BTreeMap<Perm, usize> = elements.iter().enumerate().map(|(i, e)| (e.clone(), i)).collect();
    let id_idx = idx[&identity(degree)];
    let mul: Vec<Vec<usize>> =
        (0..n).map(|i| (0..n).map(|j| idx[&compose(&elements[i], &elements[j])]).collect()).collect();
    let ord: Vec<usize> = elements.iter().map(|e| element_order(e)).collect();
    // A generating set: the distinct non-identity input generators (the closure is unchanged).
    let mut gen_idx: Vec<usize> = Vec::new();
    for g in gens {
        let gi = idx[g];
        if gi != id_idx && !gen_idx.contains(&gi) {
            gen_idx.push(gi);
        }
    }
    if gen_idx.is_empty() {
        return Some(1); // the trivial group has only the identity automorphism
    }
    // Candidate images per generator: same-order elements. Budget the search-space product.
    let candidates: Vec<Vec<usize>> =
        gen_idx.iter().map(|&gi| (0..n).filter(|&e| ord[e] == ord[gi]).collect::<Vec<_>>()).collect();
    if candidates.iter().map(|c| c.len() as u128).product::<u128>() > 2_000_000 {
        return None;
    }
    let m = gen_idx.len();
    let mut count = 0u128;
    let mut choice = vec![0usize; m];
    loop {
        let img: Vec<usize> = (0..m).map(|t| candidates[t][choice[t]]).collect();
        // Extend φ (sending gen_idx[t] ↦ img[t]) by BFS over the Cayley graph from the identity.
        let mut phi = vec![usize::MAX; n];
        phi[id_idx] = id_idx;
        let mut queue = vec![id_idx];
        let mut head = 0;
        let mut ok = true;
        'bfs: while head < queue.len() {
            let u = queue[head];
            head += 1;
            for t in 0..m {
                let ug = mul[u][gen_idx[t]];
                let target = mul[phi[u]][img[t]];
                if phi[ug] == usize::MAX {
                    phi[ug] = target;
                    queue.push(ug);
                } else if phi[ug] != target {
                    ok = false;
                    break 'bfs;
                }
            }
        }
        // A valid automorphism is total and bijective.
        if ok && phi.iter().all(|&x| x != usize::MAX) {
            let mut seen = vec![false; n];
            if phi.iter().all(|&x| !std::mem::replace(&mut seen[x], true)) {
                count += 1;
            }
        }
        let mut t = 0;
        while t < m {
            choice[t] += 1;
            if choice[t] < candidates[t].len() {
                break;
            }
            choice[t] = 0;
            t += 1;
        }
        if t == m {
            break;
        }
    }
    Some(count)
}

/// The order of the **outer automorphism group** `Out(G) = Aut(G)/Inn(G)`, where the inner automorphisms
/// `Inn(G) ≅ G/Z(G)` are those realised by conjugation. `Out(G)` counts the "exotic" symmetries of the
/// group not coming from within it. `None` when out of range.
pub fn outer_automorphism_order(degree: usize, gens: &[Perm], cap: usize) -> Option<u128> {
    let aut = automorphism_group_order(degree, gens, cap)?;
    let order = schreier_sims(degree, gens).order();
    let center = center_order(degree, gens, cap)?;
    Some(aut / (order / center)) // |Inn(G)| = |G| / |Z(G)|
}

/// The **table of marks** of `G = ⟨gens⟩` — the Burnside-ring analogue of the character table. Rows and
/// columns are the conjugacy classes of subgroups (ordered by increasing order); the `(i,j)` entry is the
/// **mark** `m(H_i, H_j)` = the number of `H_i`-fixed points in the transitive action of `G` on the cosets
/// `G/H_j`, computed as `(1/|H_j|)·|{g ∈ G : g⁻¹ H_i g ⊆ H_j}|`. Returns `(subgroup_class_orders, marks)`.
///
/// The complete invariant of the category of `G`-sets: every finite `G`-set decomposes uniquely into the
/// transitive ones `G/H_j`, and the marks record how each subgroup sees each. With this ordering the matrix
/// is triangular with diagonal `[N_G(H_i):H_i]`, hence invertible. Where [`character_table`] classifies the
/// LINEAR representations of `G`, the table of marks classifies its PERMUTATION representations. Exact
/// integer arithmetic. `None` when `|G| > cap`.
pub fn table_of_marks(degree: usize, gens: &[Perm], cap: usize) -> Option<(Vec<u128>, Vec<Vec<u128>>)> {
    let elements: Vec<Perm> =
        subgroup_closure(degree, &gens.iter().cloned().collect()).into_iter().collect();
    let subs = all_subgroups(degree, gens, cap)?;
    // Conjugation x ↦ g⁻¹ x g, applied to a whole subgroup.
    let conjugate = |h: &BTreeSet<Perm>, g: &Perm| -> BTreeSet<Perm> {
        let gi = invert(g);
        h.iter().map(|x| compose(&compose(&gi, x), g)).collect()
    };
    // One representative per conjugacy class of subgroups.
    let mut reps: Vec<BTreeSet<Perm>> = Vec::new();
    let mut seen: BTreeSet<BTreeSet<Perm>> = BTreeSet::new();
    for h in &subs {
        if seen.contains(h) {
            continue;
        }
        for g in &elements {
            seen.insert(conjugate(h, g));
        }
        reps.push(h.clone());
    }
    // Order classes by subgroup order, then canonically — makes the table triangular and deterministic.
    reps.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    let k = reps.len();
    let orders: Vec<u128> = reps.iter().map(|h| h.len() as u128).collect();
    let mut marks = vec![vec![0u128; k]; k];
    for i in 0..k {
        for j in 0..k {
            if reps[i].len() > reps[j].len() {
                continue; // a larger subgroup cannot be conjugated inside a smaller one
            }
            let count = elements
                .iter()
                .filter(|g| conjugate(&reps[i], g).iter().all(|x| reps[j].contains(x)))
                .count() as u128;
            marks[i][j] = count / reps[j].len() as u128;
        }
    }
    Some((orders, marks))
}

/// The **Burnside ring** multiplication of `G = ⟨gens⟩` — the structure constants `N[a][b][l]` giving the
/// decomposition of the product G-set `(G/H_a) × (G/H_b) = ⊔_l N[a][b][l]·(G/H_l)` into transitive G-sets,
/// indexed by the conjugacy classes of subgroups (same order as [`table_of_marks`]).
///
/// Marks are multiplicative — a subgroup fixes a pair iff it fixes each coordinate — so the mark vector of a
/// product is the componentwise product of the factors' mark vectors; the (triangular, invertible) table of
/// marks is then back-substituted to recover the multiplicities. This is the multiplication of the Burnside
/// ring, the G-set analogue of the tensor/fusion ring of the character table ([`tensor_decomposition`]).
/// FAIL-CLOSED: `None` if any back-substitution is inexact (it never is for genuine G-sets, so this
/// certifies the result). Coefficients are non-negative integers.
pub fn burnside_ring_product(degree: usize, gens: &[Perm], cap: usize) -> Option<Vec<Vec<Vec<i128>>>> {
    let (_orders, marks) = table_of_marks(degree, gens, cap)?;
    let k = marks.len();
    // Solve marks · c = p by back-substitution (indices ordered by increasing subgroup order, so the system
    // is upper-triangular with a nonzero diagonal).
    let solve = |p: &[u128]| -> Option<Vec<i128>> {
        let mut c = vec![0i128; k];
        for i in (0..k).rev() {
            let mut acc = p[i] as i128;
            for l in (i + 1)..k {
                acc -= marks[i][l] as i128 * c[l];
            }
            let diag = marks[i][i] as i128;
            if diag == 0 || acc % diag != 0 {
                return None; // inexact ⇒ not a genuine G-set decomposition
            }
            c[i] = acc / diag;
        }
        Some(c)
    };
    let mut n = vec![vec![vec![0i128; k]; k]; k];
    for a in 0..k {
        for b in 0..k {
            let p: Vec<u128> = (0..k).map(|i| marks[i][a] * marks[i][b]).collect();
            let c = solve(&p)?;
            for l in 0..k {
                n[a][b][l] = c[l];
            }
        }
    }
    Some(n)
}

/// The **Möbius function to the top** of the subgroup lattice: `μ(H, G)` for every subgroup `H`, returned as
/// `(subgroup_orders, mu)` aligned by index. Defined by `μ(G,G)=1` and `μ(H,G) = -Σ_{H ⊊ L} μ(L,G)`. `None`
/// when `|G| > cap`.
fn lattice_mobius_to_top(degree: usize, gens: &[Perm], cap: usize) -> Option<(Vec<u128>, Vec<i128>)> {
    let subs: Vec<BTreeSet<Perm>> = all_subgroups(degree, gens, cap)?.into_iter().collect();
    let n = subs.len();
    let top_size = subs.iter().map(|h| h.len()).max().unwrap_or(0); // |G|, unique maximum
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(subs[i].len())); // descending: compute supergroups first
    let mut mu = vec![0i128; n];
    for &i in &order {
        if subs[i].len() == top_size {
            mu[i] = 1; // μ(G, G) = 1
        } else {
            let s: i128 = (0..n)
                .filter(|&j| subs[i].len() < subs[j].len() && subs[i].is_subset(&subs[j]))
                .map(|j| mu[j])
                .sum();
            mu[i] = -s;
        }
    }
    Some((subs.iter().map(|h| h.len() as u128).collect(), mu))
}

/// The **Möbius number** `μ(1, G)` of the subgroup lattice of `G = ⟨gens⟩` — the value of the lattice's
/// Möbius function from the trivial subgroup to the whole group. A classical invariant: for a cyclic group
/// `μ(1, Cₙ)` is the number-theoretic Möbius function `μ(n)`, and in general it drives the group's Eulerian
/// (probabilistic-zeta) function. `None` when `|G| > cap`.
pub fn mobius_number(degree: usize, gens: &[Perm], cap: usize) -> Option<i128> {
    let (orders, mu) = lattice_mobius_to_top(degree, gens, cap)?;
    (0..orders.len()).find(|&i| orders[i] == 1).map(|i| mu[i])
}

/// The number of ordered `k`-tuples of group elements that **generate** `G` — the Eulerian function
/// `e_k(G) = Σ_{H ≤ G} μ(H, G)·|H|ᵏ` (Hall's formula by Möbius inversion over the subgroup lattice, since
/// `|H|ᵏ` counts the `k`-tuples landing in `H`). Dividing by `|G|ᵏ` gives the probability that `k` random
/// elements generate `G`. `None` when `|G| > cap`.
pub fn generating_tuple_count(degree: usize, gens: &[Perm], cap: usize, k: u32) -> Option<i128> {
    let (orders, mu) = lattice_mobius_to_top(degree, gens, cap)?;
    Some((0..orders.len()).map(|i| mu[i] * (orders[i] as i128).pow(k)).sum())
}

/// One representative per conjugacy class of subgroups, ordered by increasing order (the row/column index
/// shared by [`table_of_marks`] and the permutation-character decomposition).
fn subgroup_class_reps(degree: usize, gens: &[Perm], cap: usize) -> Option<Vec<BTreeSet<Perm>>> {
    let elements: Vec<Perm> =
        subgroup_closure(degree, &gens.iter().cloned().collect()).into_iter().collect();
    let subs = all_subgroups(degree, gens, cap)?;
    let conjugate = |h: &BTreeSet<Perm>, g: &Perm| -> BTreeSet<Perm> {
        let gi = invert(g);
        h.iter().map(|x| compose(&compose(&gi, x), g)).collect()
    };
    let mut reps: Vec<BTreeSet<Perm>> = Vec::new();
    let mut seen: BTreeSet<BTreeSet<Perm>> = BTreeSet::new();
    for h in &subs {
        if seen.contains(h) {
            continue;
        }
        for g in &elements {
            seen.insert(conjugate(h, g));
        }
        reps.push(h.clone());
    }
    reps.sort_by(|a, b| a.len().cmp(&b.len()).then_with(|| a.cmp(b)));
    Some(reps)
}

/// The **permutation-character decomposition** — the bridge between the table of marks and the character
/// table. `M[i][s]` is the multiplicity of the irreducible `χ_s` in the permutation representation of `G`
/// on the cosets `G/H_i`, i.e. `M[i][s] = ⟨Ind_{H_i}^G 1, χ_s⟩ = (1/|H_i|)·Σ_{h ∈ H_i} χ_s(h)` (Frobenius
/// reciprocity = the dimension of the `H_i`-fixed subspace of `χ_s`). Rows are subgroup conjugacy classes
/// (as in [`table_of_marks`]), columns are irreducibles (as in [`character_table`]). Returns
/// `(subgroup_orders, irreducible_degrees, M)`.
///
/// This is the "linearization" map from the Burnside ring to the representation ring made explicit. The
/// character values are read off the `GF(p)` character table; each multiplicity is a small non-negative
/// integer (`≤ d_s`), so it decodes uniquely. FAIL-CLOSED: `None` unless `M[1] = ` the degrees (the regular
/// representation contains each `χ_s` with multiplicity `d_s`), `M[i][trivial] = 1` (every transitive action
/// contains the trivial character once), `M[G] = e_trivial`, and `Σ_s M[i][s]·d_s = [G:H_i]` for every row.
pub fn permutation_character_decomposition(
    degree: usize,
    gens: &[Perm],
    cap: usize,
) -> Option<(Vec<u128>, Vec<u128>, Vec<Vec<u128>>)> {
    let ct = character_table(degree, gens, cap)?;
    let p = ct.prime;
    let k = ct.degrees.len();
    let classes = conjugacy_classes(degree, gens, cap)?;
    let idx = class_index_map(&classes);
    let reps = subgroup_class_reps(degree, gens, cap)?;
    let order: u128 = ct.degrees.iter().map(|d| d * d).sum();

    let mut m = vec![vec![0u128; k]; reps.len()];
    for (i, h) in reps.iter().enumerate() {
        // |H_i ∩ C_r| for each conjugacy class C_r.
        let mut inter = vec![0u128; k];
        for x in h {
            inter[idx[x]] += 1;
        }
        let inv_h = mod_inv((h.len() as u128 % p as u128) as u64, p);
        for s in 0..k {
            let mut acc = 0u64;
            for r in 0..k {
                let term = (inter[r] % p as u128) as u64;
                acc = ((acc as u128 + term as u128 * ct.values[s][r] as u128) % p as u128) as u64;
            }
            m[i][s] = (acc as u128 * inv_h as u128 % p as u128) as u128;
        }
    }

    // FAIL-CLOSED verification against the classical identities.
    let trivial_irr = ct.values.iter().position(|row| row.iter().all(|&x| x == 1))?;
    for (i, h) in reps.iter().enumerate() {
        if m[i][trivial_irr] != 1 {
            return None; // a transitive action contains the trivial character exactly once
        }
        let dim: u128 = (0..k).map(|s| m[i][s] * ct.degrees[s]).sum();
        if dim != order / h.len() as u128 {
            return None; // Σ_s M·d_s = [G : H_i]
        }
        for s in 0..k {
            if m[i][s] > ct.degrees[s] {
                return None; // multiplicity cannot exceed the degree
            }
        }
    }
    if m[0] != ct.degrees {
        return None; // the regular representation G/1 = Σ_s d_s·χ_s
    }
    let mut e_triv = vec![0u128; k];
    e_triv[trivial_irr] = 1;
    if *m.last()? != e_triv {
        return None; // G/G is the trivial representation
    }
    let orders: Vec<u128> = reps.iter().map(|h| h.len() as u128).collect();
    Some((orders, ct.degrees, m))
}

// ---- Character table via the Burnside–Dixon method --------------------------------------------
//
// The class sums `Ĉ_r = Σ_{g∈C_r} g` span the centre of the group algebra; left-multiplication by `Ĉ_i`
// is the matrix `M_i` with `M_i[k][j] = a[i][j][k]` (the structure constants). The `M_i` pairwise commute
// (the centre is commutative) and are simultaneously diagonalisable, with exactly `k = #classes` common
// eigenvectors — one per irreducible character `χ_s`. The eigenvalue of `M_i` on `χ_s`'s eigenvector is
// `ω_i(χ_s) = |C_i|·χ_s(C_i)/d_s` (`d_s = χ_s(1)`). Dixon's insight: work over `GF(p)` for a prime
// `p ≡ 1 (mod exponent)` with `p > |G|`, where every `ω` lives (characters are sums of `e`-th roots of
// unity, which `GF(p)` contains), so the whole computation is EXACT — no floats, no SDP.

/// `aᵇ mod p`. `p` need not be prime here.
fn mod_pow(mut base: u64, mut exp: u64, p: u64) -> u64 {
    let mut r = 1u128;
    let mut b = (base % p) as u128;
    base = 0; // silence unused-assignment style; b carries the value
    let _ = base;
    while exp > 0 {
        if exp & 1 == 1 {
            r = (r * b) % p as u128;
        }
        b = (b * b) % p as u128;
        exp >>= 1;
    }
    r as u64
}

/// Multiplicative inverse in `GF(p)` (`p` prime) via Fermat: `a^(p-2)`. `a` must be `≢ 0`.
pub(crate) fn mod_inv(a: u64, p: u64) -> u64 {
    mod_pow(a % p, p - 2, p)
}

/// Trial-division primality (the Dixon primes here are small — `O(√|G|)`-ish).
pub(crate) fn is_prime(n: u64) -> bool {
    if n < 2 {
        return false;
    }
    if n % 2 == 0 {
        return n == 2;
    }
    let mut d = 3u64;
    while d * d <= n {
        if n % d == 0 {
            return false;
        }
        d += 2;
    }
    true
}

/// Integer floor square root.
fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    let mut x = (n as f64).sqrt() as u128;
    while x * x > n {
        x -= 1;
    }
    while (x + 1) * (x + 1) <= n {
        x += 1;
    }
    x
}

/// `M·v` over `GF(p)` (`m` is `k×k` as rows, `v` length `k`).
pub(crate) fn gf_mat_vec(m: &[Vec<u64>], v: &[u64], p: u64) -> Vec<u64> {
    m.iter()
        .map(|row| {
            let mut acc = 0u128;
            for (a, b) in row.iter().zip(v) {
                acc += (*a as u128) * (*b as u128);
            }
            (acc % p as u128) as u64
        })
        .collect()
}

/// A basis for the right null space `{x ∈ GF(p)^ncols : A·x = 0}` of `a` (given as rows), via RREF.
pub(crate) fn gf_nullspace(mut a: Vec<Vec<u64>>, ncols: usize, p: u64) -> Vec<Vec<u64>> {
    let nrows = a.len();
    let mut where_pivot = vec![usize::MAX; ncols]; // pivot row of each column, or MAX
    let mut row = 0usize;
    for col in 0..ncols {
        if row >= nrows {
            break;
        }
        let Some(sel) = (row..nrows).find(|&r| a[r][col] % p != 0) else { continue };
        a.swap(row, sel);
        let inv = mod_inv(a[row][col], p);
        for c in 0..ncols {
            a[row][c] = ((a[row][c] as u128 * inv as u128) % p as u128) as u64;
        }
        for r in 0..nrows {
            if r != row && a[r][col] != 0 {
                let f = a[r][col] as u128;
                for c in 0..ncols {
                    let sub = (f * a[row][c] as u128) % p as u128;
                    a[r][c] = ((a[r][c] as u128 + p as u128 - sub) % p as u128) as u64;
                }
            }
        }
        where_pivot[col] = row;
        row += 1;
    }
    let mut basis = Vec::new();
    for fc in 0..ncols {
        if where_pivot[fc] != usize::MAX {
            continue; // pivot column, not free
        }
        let mut x = vec![0u64; ncols];
        x[fc] = 1;
        for (col, &pr) in where_pivot.iter().enumerate() {
            if pr != usize::MAX {
                x[col] = (p - a[pr][fc] % p) % p; // x[col] = -a[pr][fc]
            }
        }
        basis.push(x);
    }
    basis
}

/// The character table of `⟨gens⟩`, computed exactly by Dixon's method.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterTable {
    /// The Dixon prime `p` (`p ≡ 1 mod exponent`, `p > |G|`) the values live in.
    pub prime: u64,
    /// Conjugacy class sizes `|C_r|` (column `r`).
    pub class_sizes: Vec<u128>,
    /// Inverse-class index `r̄` (`C_{r̄} = C_r⁻¹`); `χ(C_{r̄})` is the `GF(p)` image of `conj χ(C_r)`.
    pub inverse_class: Vec<usize>,
    /// The squaring power map: `power_map2[r]` is the class index of `g_r²` (`g_r ∈ C_r`) — a class
    /// invariant (`(xgx⁻¹)² = xg²x⁻¹`), the input to the Frobenius–Schur indicators.
    pub power_map2: Vec<usize>,
    /// The index of the identity class (`{e}`).
    pub identity_class: usize,
    /// A representative permutation of each class (`C_r`'s first element) — lets character-derived
    /// quantities that depend on the underlying action (e.g. the permutation character) read off the table.
    pub class_reps: Vec<Perm>,
    /// Irreducible degrees `χ_s(1)` (row `s`), as exact integers; `Σ d_s² = |G|`.
    pub degrees: Vec<u128>,
    /// `χ_s(C_r)` as a `GF(p)` element (the image of the algebraic character value). `values[s][r]`.
    pub values: Vec<Vec<u64>>,
}

/// Compute the full character table via the Burnside–Dixon algorithm: build the commuting class-algebra
/// matrices `M_i` from the structure constants, choose a Dixon prime `p`, simultaneously diagonalise the
/// `M_i` over `GF(p)` by iterated eigenspace refinement (one common eigenvector per irreducible), then
/// read off each degree `d_s` (exact integer, via `d² = |G|/Σ_r ω_r ω_{r̄}/|C_r|`) and the character
/// values `χ_s(C_r) = d_s ω_r/|C_r|`. Returns `None` when `|G| > cap`, the group is too large for the
/// finite-field arithmetic, or — FAIL-CLOSED — the recovered table does not satisfy the row-orthogonality
/// and degree relations (so a returned table is always a verified one). Rows are sorted by `(degree,
/// values)` for determinism.
pub fn character_table(degree: usize, gens: &[Perm], cap: usize) -> Option<CharacterTable> {
    let classes = conjugacy_classes(degree, gens, cap)?;
    let k = classes.len();
    // Bound the finite-field work: the structure tensor is k³ and the diagonalisation is O(p·k⁴).
    if k == 0 || k > 64 {
        return None;
    }
    let order = schreier_sims(degree, gens).order();
    if order == 0 || order > 100_000 {
        return None;
    }
    let idx = class_index_map(&classes);
    let class_sizes: Vec<u128> = classes.iter().map(|c| c.len() as u128).collect();
    let inverse_class: Vec<usize> = classes.iter().map(|c| idx[&invert(&c[0])]).collect();
    let power_map2: Vec<usize> = classes.iter().map(|c| idx[&compose(&c[0], &c[0])]).collect();
    let class_reps: Vec<Perm> = classes.iter().map(|c| c[0].clone()).collect();
    let id_perm = identity(degree);
    let identity_class = classes.iter().position(|c| c.iter().any(|g| *g == id_perm))?;

    // Dixon prime: p ≡ 1 (mod exponent), prime, and p > |G| (so |G| and every |C_r| are invertible and
    // degree recovery is unique — p > |G| ⟹ p > 2√|G|).
    let e = exponent(degree, gens, cap)? as u64;
    let order_u = order as u64;
    let p = {
        let mut m = order_u / e + 1;
        let mut found = None;
        for _ in 0..1_000_000 {
            let cand = e.checked_mul(m)?.checked_add(1)?;
            if cand > order_u && is_prime(cand) {
                found = Some(cand);
                break;
            }
            m += 1;
        }
        found?
    };

    // Structure constants → the commuting matrices M_i with M_i[k][j] = a[i][j][k] (mod p).
    let a = class_multiplication_coefficients(degree, gens, cap)?;
    let mmats: Vec<Vec<Vec<u64>>> = (0..k)
        .map(|i| {
            let mut m = vec![vec![0u64; k]; k];
            for j in 0..k {
                for kk in 0..k {
                    m[kk][j] = (a[i][j][kk] % p as u128) as u64;
                }
            }
            m
        })
        .collect();

    // Simultaneous diagonalisation: refine the whole space into the k common 1-dim eigenspaces.
    let mut subspaces: Vec<Vec<Vec<u64>>> = vec![(0..k)
        .map(|i| {
            let mut ei = vec![0u64; k];
            ei[i] = 1;
            ei
        })
        .collect()];
    for mi in &mmats {
        if subspaces.iter().all(|s| s.len() == 1) {
            break;
        }
        let mut next: Vec<Vec<Vec<u64>>> = Vec::new();
        for s in &subspaces {
            if s.len() == 1 {
                next.push(s.clone());
                continue;
            }
            let bn = s.len();
            let mb: Vec<Vec<u64>> = s.iter().map(|b| gf_mat_vec(mi, b, p)).collect();
            let mut pieces: Vec<Vec<Vec<u64>>> = Vec::new();
            let mut covered = 0usize;
            for lam in 0..p {
                // Null space of (M_i - λI)|_s in the basis `s`: columns are (M_i - λI)·b_j.
                let mut rows = vec![vec![0u64; bn]; k];
                for r in 0..k {
                    for (j, bj) in s.iter().enumerate() {
                        let shifted = (lam as u128 * bj[r] as u128) % p as u128;
                        rows[r][j] = ((mb[j][r] as u128 + p as u128 - shifted) % p as u128) as u64;
                    }
                }
                let ns = gf_nullspace(rows, bn, p);
                if ns.is_empty() {
                    continue;
                }
                let eig: Vec<Vec<u64>> = ns
                    .iter()
                    .map(|c| {
                        let mut x = vec![0u64; k];
                        for (j, &cj) in c.iter().enumerate() {
                            if cj != 0 {
                                for r in 0..k {
                                    x[r] = ((x[r] as u128 + cj as u128 * s[j][r] as u128) % p as u128) as u64;
                                }
                            }
                        }
                        x
                    })
                    .collect();
                covered += eig.len();
                pieces.push(eig);
                if covered == bn {
                    break;
                }
            }
            if covered == bn {
                next.extend(pieces);
            } else {
                next.push(s.clone()); // a later M_i may split it; final check rejects if none do
            }
        }
        subspaces = next;
    }
    if subspaces.iter().any(|s| s.len() != 1) {
        return None; // did not fully diagonalise — fail closed
    }

    // Read off each irreducible from its common eigenvector.
    let order_p = (order % p as u128) as u64;
    let max_deg = isqrt(order);
    let mut rows: Vec<(u128, Vec<u64>)> = Vec::with_capacity(k);
    for s in &subspaces {
        let v = &s[0];
        let t = v.iter().position(|&x| x != 0)?;
        let inv_vt = mod_inv(v[t], p);
        let omega: Vec<u64> = mmats
            .iter()
            .map(|mi| {
                let mv = gf_mat_vec(mi, v, p);
                ((mv[t] as u128 * inv_vt as u128) % p as u128) as u64
            })
            .collect();
        // d² = |G| / Σ_r ω_r·ω_{r̄}/|C_r|  (all over GF(p)).
        let mut denom = 0u64;
        for r in 0..k {
            let hr = (class_sizes[r] % p as u128) as u64;
            let term = (omega[r] as u128 * omega[inverse_class[r]] as u128 % p as u128) as u64;
            let contrib = (term as u128 * mod_inv(hr, p) as u128 % p as u128) as u64;
            denom = (denom + contrib) % p;
        }
        if denom == 0 {
            return None;
        }
        let d2 = (order_p as u128 * mod_inv(denom, p) as u128 % p as u128) as u64;
        // Recover the integer degree: the unique d ≤ √|G| dividing |G| with d² ≡ d2 (mod p).
        let mut deg = None;
        let mut d = 1u128;
        while d <= max_deg {
            if order % d == 0 && ((d * d) % p as u128) as u64 == d2 {
                deg = Some(d);
                break;
            }
            d += 1;
        }
        let deg = deg?;
        let vals: Vec<u64> = (0..k)
            .map(|r| {
                let hr = (class_sizes[r] % p as u128) as u64;
                let num = (deg % p as u128) as u64 as u128 * omega[r] as u128 % p as u128;
                (num * mod_inv(hr, p) as u128 % p as u128) as u64
            })
            .collect();
        rows.push((deg, vals));
    }
    rows.sort();
    let degrees: Vec<u128> = rows.iter().map(|(d, _)| *d).collect();
    let values: Vec<Vec<u64>> = rows.into_iter().map(|(_, v)| v).collect();

    // FAIL-CLOSED verification: degrees and row-orthogonality must hold, else we discovered nothing real.
    if degrees.iter().map(|d| d * d).sum::<u128>() != order {
        return None;
    }
    if !values.iter().any(|row| row.iter().all(|&x| x == 1)) {
        return None; // the trivial character must be present
    }
    for s in 0..k {
        for t in 0..k {
            let mut acc = 0u64;
            for r in 0..k {
                let hr = (class_sizes[r] % p as u128) as u64;
                let prod = values[s][r] as u128 * values[t][inverse_class[r]] as u128 % p as u128;
                acc = ((acc as u128 + hr as u128 * prod) % p as u128) as u64;
            }
            let want = if s == t { order_p } else { 0 };
            if acc != want {
                return None;
            }
        }
    }

    Some(CharacterTable {
        prime: p,
        class_sizes,
        inverse_class,
        power_map2,
        identity_class,
        class_reps,
        degrees,
        values,
    })
}

/// The **Frobenius–Schur indicators** `ν(χ_s) = (1/|G|) Σ_{g∈G} χ_s(g²) ∈ {+1, 0, −1}` read off a
/// character table: `+1` if the irreducible is **real** (orthogonal), `0` if **complex** (`χ ≠ χ̄`), `−1`
/// if **quaternionic** (symplectic). Computed exactly over the table's `GF(p)`:
/// `Σ_r |C_r|·χ_s(C_{r²})`, scaled by `1/|G|`, decoded `{0, 1, p−1} → {0, 1, −1}`. FAIL-CLOSED: returns
/// `None` if any value decodes outside `{−1,0,1}` or the Frobenius–Schur sum rule
/// `Σ_s ν(χ_s)·d_s = #{g : g²=1}` fails. The indicators distinguish groups with identical character
/// tables (e.g. `D₄` all `+1` vs `Q₈` with a `−1`).
pub fn frobenius_schur_from_table(t: &CharacterTable) -> Option<Vec<i8>> {
    let p = t.prime;
    let order: u128 = t.degrees.iter().map(|d| d * d).sum();
    let inv_order = mod_inv((order % p as u128) as u64, p);
    let k = t.degrees.len();
    // #{g : g² = e} = Σ over classes whose square is the identity class.
    let involutions_plus_id: u128 = (0..k)
        .filter(|&r| t.power_map2[r] == t.identity_class)
        .map(|r| t.class_sizes[r])
        .sum();
    let mut nu = Vec::with_capacity(k);
    for s in 0..k {
        let mut acc = 0u64;
        for r in 0..k {
            let hr = (t.class_sizes[r] % p as u128) as u64;
            let chi_sq = t.values[s][t.power_map2[r]];
            acc = ((acc as u128 + hr as u128 * chi_sq as u128) % p as u128) as u64;
        }
        let val = ((acc as u128 * inv_order as u128) % p as u128) as u64;
        let ind: i8 = if val == 0 {
            0
        } else if val == 1 {
            1
        } else if val == p - 1 {
            -1
        } else {
            return None; // not an indicator — fail closed
        };
        nu.push(ind);
    }
    // The Frobenius–Schur counting theorem: Σ_s ν_s·d_s = #{g : g²=1}.
    let sum: i128 = nu.iter().zip(&t.degrees).map(|(&v, &d)| v as i128 * d as i128).sum();
    if sum != involutions_plus_id as i128 {
        return None;
    }
    Some(nu)
}

/// The Frobenius–Schur indicators of `⟨gens⟩`, one per irreducible character (aligned with
/// [`character_table`]'s rows). See [`frobenius_schur_from_table`]. `None` when the character table is out
/// of range.
pub fn frobenius_schur_indicators(degree: usize, gens: &[Perm], cap: usize) -> Option<Vec<i8>> {
    frobenius_schur_from_table(&character_table(degree, gens, cap)?)
}

/// The **permutation character** `π(g) = #{points fixed by g}` of the natural action of `⟨gens⟩` on its
/// `degree` points, valued per conjugacy class (a class invariant, since conjugate permutations have the
/// same cycle type). The character of the permutation representation `ℂ^degree`. Aligned with the conjugacy
/// classes (so with [`CharacterTable`]'s columns). `None` when `|G| > cap`.
pub fn permutation_character(degree: usize, gens: &[Perm], cap: usize) -> Option<Vec<u128>> {
    let classes = conjugacy_classes(degree, gens, cap)?;
    Some(
        classes
            .iter()
            .map(|c| c[0].iter().enumerate().filter(|(i, &x)| *i == x).count() as u128)
            .collect(),
    )
}

/// The **isotypic decomposition** of the permutation representation: the multiplicity `m_s = ⟨π, χ_s⟩` of
/// each irreducible `χ_s` in the natural action's character `π`, i.e. `π = Σ_s m_s χ_s`. Computed from a
/// character table: `m_s = (1/|G|) Σ_r |C_r|·π(C_r)·χ_s(C_{r̄})` over the table's `GF(p)`. The
/// representation-theoretic spectrum of the symmetry — it ties the linear theory back to the action:
/// `m_trivial = #orbits` (Burnside), `Σ_s m_s² = rank` (#orbitals), `Σ_s m_s·d_s = degree`. FAIL-CLOSED:
/// `None` if `p ≤ degree` (then the small non-negative `m_s ≤ degree` would not decode uniquely) or if any
/// of those three identities fails. Aligned with the table's rows.
pub fn isotypic_from_table(degree: usize, gens: &[Perm], t: &CharacterTable) -> Option<Vec<u128>> {
    let p = t.prime;
    if p as u128 <= degree as u128 {
        return None; // multiplicities (≤ degree) must fit below p to decode uniquely
    }
    let order: u128 = t.degrees.iter().map(|d| d * d).sum();
    let inv_order = mod_inv((order % p as u128) as u64, p);
    let k = t.degrees.len();
    // The permutation character per class, from the table's class representatives.
    let pi: Vec<u64> = t
        .class_reps
        .iter()
        .map(|g| (g.iter().enumerate().filter(|(i, &x)| *i == x).count() as u64) % p)
        .collect();
    let mut mult = Vec::with_capacity(k);
    for s in 0..k {
        let mut acc = 0u64;
        for r in 0..k {
            let hr = (t.class_sizes[r] % p as u128) as u64;
            let term = (hr as u128 * pi[r] as u128 % p as u128) as u64;
            let contrib = (term as u128 * t.values[s][t.inverse_class[r]] as u128) % p as u128;
            acc = ((acc as u128 + contrib) % p as u128) as u64;
        }
        let m = ((acc as u128 * inv_order as u128) % p as u128) as u128;
        mult.push(m);
    }
    // FAIL-CLOSED cross-checks against independent computations of the action's invariants.
    if mult.iter().zip(&t.degrees).map(|(m, d)| m * d).sum::<u128>() != degree as u128 {
        return None; // Σ m_s·d_s = dim of the permutation rep
    }
    if mult.iter().map(|m| m * m).sum::<u128>() != rank(degree, gens) as u128 {
        return None; // ⟨π,π⟩ = #orbitals (rank)
    }
    let num_orbits = orbits(degree, gens).len() as u128;
    let trivial_row = t.values.iter().position(|row| row.iter().all(|&x| x == 1))?;
    if mult[trivial_row] != num_orbits {
        return None; // ⟨π,1⟩ = #orbits (Burnside)
    }
    Some(mult)
}

/// The isotypic multiplicities of `⟨gens⟩`'s permutation representation, one per irreducible (aligned with
/// [`character_table`]'s rows). See [`isotypic_from_table`]. `None` when the character table is out of range
/// or the multiplicities cannot be decoded/verified.
pub fn isotypic_multiplicities(degree: usize, gens: &[Perm], cap: usize) -> Option<Vec<u128>> {
    isotypic_from_table(degree, gens, &character_table(degree, gens, cap)?)
}

/// The **tensor (Clebsch–Gordan) decomposition** of the irreducibles: `N[i][j][k] = ⟨χ_i·χ_j, χ_k⟩`, the
/// multiplicity of `χ_k` in the tensor product `χ_i ⊗ χ_j`. These are the *fusion coefficients* — the
/// structure constants of the representation ring `R(G)` (the multiplication dual to the character table's
/// addition). Computed from a character table:
/// `N[i][j][k] = (1/|G|) Σ_r |C_r|·χ_i(C_r)·χ_j(C_r)·χ_k(C_{r̄})` over the table's `GF(p)`; each is a small
/// non-negative integer `≤ d_i·d_j ≤ |G| < p`, so it decodes uniquely. FAIL-CLOSED: returns `None` unless
/// every fusion product has the right dimension (`Σ_k N[i][j][k]·d_k = d_i·d_j`), the trivial character is a
/// unit (`χ_triv ⊗ χ_j = χ_j`), and the coefficients are symmetric (`N[i][j][k] = N[j][i][k]`). Indices
/// align with [`character_table`]'s rows.
pub fn tensor_from_table(t: &CharacterTable) -> Option<Vec<Vec<Vec<u128>>>> {
    let p = t.prime;
    let order: u128 = t.degrees.iter().map(|d| d * d).sum();
    let inv_order = mod_inv((order % p as u128) as u64, p);
    let k = t.degrees.len();
    let mut n = vec![vec![vec![0u128; k]; k]; k];
    for i in 0..k {
        for j in 0..k {
            for kk in 0..k {
                let mut acc = 0u64;
                for r in 0..k {
                    let hr = (t.class_sizes[r] % p as u128) as u64;
                    let mut prod = hr as u128;
                    prod = prod * t.values[i][r] as u128 % p as u128;
                    prod = prod * t.values[j][r] as u128 % p as u128;
                    prod = prod * t.values[kk][t.inverse_class[r]] as u128 % p as u128;
                    acc = ((acc as u128 + prod) % p as u128) as u64;
                }
                n[i][j][kk] = (acc as u128 * inv_order as u128 % p as u128) as u128;
            }
        }
    }
    // FAIL-CLOSED structural checks of the representation ring.
    let trivial = t.values.iter().position(|row| row.iter().all(|&x| x == 1))?;
    for i in 0..k {
        for j in 0..k {
            // Dimension: dim(χ_i ⊗ χ_j) = d_i·d_j.
            if (0..k).map(|kk| n[i][j][kk] * t.degrees[kk]).sum::<u128>() != t.degrees[i] * t.degrees[j] {
                return None;
            }
            // Symmetry of the tensor product.
            for kk in 0..k {
                if n[i][j][kk] != n[j][i][kk] {
                    return None;
                }
            }
        }
        // The trivial character is the multiplicative unit: χ_triv ⊗ χ_i = χ_i.
        for kk in 0..k {
            let expect = u128::from(kk == i);
            if n[trivial][i][kk] != expect {
                return None;
            }
        }
    }
    Some(n)
}

/// The tensor (fusion) decomposition of `⟨gens⟩`'s irreducibles. See [`tensor_from_table`]. `None` when the
/// character table is out of range or the fusion coefficients fail their structural checks.
pub fn tensor_decomposition(degree: usize, gens: &[Perm], cap: usize) -> Option<Vec<Vec<Vec<u128>>>> {
    tensor_from_table(&character_table(degree, gens, cap)?)
}

/// The **irreducible-representation degrees** `χ_s(1)` of `⟨gens⟩`, sorted ascending (`Σ dᵢ² = |G|`) —
/// the cheap summary of the [`character_table`]. `None` when the table cannot be computed (`|G| > cap`
/// or too large for the finite-field diagonalisation).
pub fn irreducible_degrees(degree: usize, gens: &[Perm], cap: usize) -> Option<Vec<u128>> {
    character_table(degree, gens, cap).map(|t| t.degrees)
}

/// Is the group **simple** — non-trivial with no normal subgroup but `{id}` and itself? Tested via
/// conjugacy: every non-trivial normal subgroup contains the whole conjugacy class of any of its elements,
/// hence the normal closure of that element. So `G` is simple iff the normal closure of *every*
/// non-identity element is all of `G`. Simple non-abelian groups (e.g. `A₅`) are exactly the unsolvable
/// building blocks. `None` when `|G| > cap`.
pub fn is_simple(degree: usize, gens: &[Perm], cap: usize) -> Option<bool> {
    let order = schreier_sims(degree, gens).order();
    if order <= 1 {
        return Some(false); // the trivial group is not simple
    }
    let classes = conjugacy_classes(degree, gens, cap)?;
    for class in &classes {
        let rep = &class[0];
        if is_identity(rep) {
            continue;
        }
        let ncl = normal_closure(degree, std::slice::from_ref(rep), gens);
        if schreier_sims(degree, &ncl).order() < order {
            return Some(false); // a proper non-trivial normal subgroup exists
        }
    }
    Some(true)
}

/// A base and strong generating set, with the per-level basic transversals.
#[derive(Clone, Debug)]
pub struct Bsgs {
    pub degree: usize,
    pub base: Vec<usize>,
    transversals: Vec<HashMap<usize, Perm>>,
}

impl Bsgs {
    /// `|G| = Π |Δᵢ|` — exact for `|G|` up to the `u128` range (any degree below ~33 factorial).
    pub fn order(&self) -> u128 {
        self.transversals.iter().map(|t| t.len() as u128).product()
    }

    /// Enumerate **all** `|G|` group elements, as the unique products `u_k·…·u₂·u₁` of one transversal
    /// element per level. (`sift` divides `g` by the level-i transversal element on the right, so
    /// `g = h·u₁` with `h ∈ G⁽²⁾`, recursing to `g = u_k·…·u₁` — deepest level innermost; we accumulate
    /// from the deepest level outward.) Returns `None` if `|G| > cap` — the BSGS knows the order without
    /// enumerating, so the caller gates on it. The basis of *complete* symmetry breaking.
    pub fn elements(&self, cap: usize) -> Option<Vec<Perm>> {
        if self.order() > cap as u128 {
            return None;
        }
        let mut elems = vec![identity(self.degree)];
        for trans in self.transversals.iter().rev() {
            let reps: Vec<&Perm> = trans.values().collect();
            let mut next = Vec::with_capacity(elems.len() * reps.len());
            for e in &elems {
                for r in &reps {
                    next.push(compose(e, r));
                }
            }
            elems = next;
        }
        Some(elems)
    }

    /// All coset representatives across the stabilizer chain — the transversal elements at every level.
    /// There are `Σ |Δᵢ|` of them (polynomial: at most `degree²`), spread across the group, so a lex-leader
    /// symmetry break over them (together with the generators) is stronger than the bare generators yet
    /// still polynomial, unlike the complete enumeration (`|G|`). This is the stabilizer chain breaking the
    /// symmetry level by level — "symmetry break again" at each base point.
    pub fn transversal_elements(&self) -> Vec<Perm> {
        self.transversals.iter().flat_map(|t| t.values().cloned()).collect()
    }

    /// Is `g` a member of the group (i.e., does it sift to the identity through the chain)? This is the
    /// coset decision: `g ∈ rep·G` iff `rep⁻¹·g` is a member.
    pub fn contains(&self, g: &[usize]) -> bool {
        if g.len() != self.degree {
            return false;
        }
        let mut g = g.to_vec();
        for (i, &beta) in self.base.iter().enumerate() {
            let img = g[beta];
            match self.transversals[i].get(&img) {
                None => return false,
                Some(t) => g = compose(&g, &invert(t)),
            }
        }
        is_identity(&g)
    }
}

/// **Schreier–Sims.** Build a BSGS for the permutation group on `degree` points generated by `generators`
/// (each a permutation of `{0,…,degree−1}`). Deterministic incremental construction: seed the chain with
/// the generators, then repeatedly sift every Schreier generator (`u·s` divided by its transversal
/// element, Schreier's lemma) into the chain, adding any non-trivial residue as a new strong generator
/// (extending the base as needed), until every Schreier generator sifts to the identity — the completeness
/// condition.
pub fn schreier_sims(degree: usize, generators: &[Perm]) -> Bsgs {
    let mut base: Vec<usize> = Vec::new();
    let mut strong: Vec<Perm> = Vec::new();
    for g in generators {
        if !is_identity(g) {
            extend_with(&mut base, &mut strong, g.clone());
        }
    }
    loop {
        let mut changed = false;
        'scan: for i in 0..base.len() {
            let trans = orbit_transversal(&base, &strong, i);
            let stab: Vec<Perm> =
                strong.iter().filter(|g| (0..i).all(|j| g[base[j]] == base[j])).cloned().collect();
            for u in trans.values() {
                for s in &stab {
                    let us = compose(u, s);
                    let img = us[base[i]];
                    let schreier = compose(&us, &invert(&trans[&img])); // fixes base[i] ∈ G⁽ⁱ⁺¹⁾
                    if !is_identity(&schreier) && extend_with(&mut base, &mut strong, schreier) {
                        changed = true;
                        break 'scan; // orbits changed — recompute from the top
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
    let transversals = (0..base.len()).map(|i| orbit_transversal(&base, &strong, i)).collect();
    Bsgs { degree, base, transversals }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Brute-force the whole group by closing the generators under right multiplication.
    fn closure(degree: usize, gens: &[Perm]) -> BTreeSet<Perm> {
        let mut set: BTreeSet<Perm> = BTreeSet::new();
        set.insert(identity(degree));
        for g in gens {
            set.insert(g.clone());
        }
        loop {
            let before = set.len();
            for a in set.iter().cloned().collect::<Vec<_>>() {
                for g in gens {
                    set.insert(compose(&a, g));
                }
            }
            if set.len() == before {
                break;
            }
        }
        set
    }

    /// Every permutation of `n` points (lexicographic), for an exhaustive membership oracle.
    fn all_perms(n: usize) -> Vec<Perm> {
        let mut out = Vec::new();
        let mut p: Perm = (0..n).collect();
        loop {
            out.push(p.clone());
            // next_permutation
            let Some(i) = (0..n.saturating_sub(1)).rev().find(|&i| p[i] < p[i + 1]) else { break };
            let j = (i + 1..n).rev().find(|&j| p[j] > p[i]).unwrap();
            p.swap(i, j);
            p[i + 1..].reverse();
        }
        out
    }

    fn splitmix(s: &mut u64) -> u64 {
        *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *s;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z ^ (z >> 31)
    }

    fn random_perm(n: usize, state: &mut u64) -> Perm {
        let mut p: Perm = (0..n).collect();
        for i in (1..n).rev() {
            let j = (splitmix(state) % (i as u64 + 1)) as usize;
            p.swap(i, j);
        }
        p
    }

    /// **Known orders, exactly.** Symmetric `S_n = n!`, alternating `A_n = n!/2`, cyclic `C_n = n`,
    /// dihedral `D_n = 2n` — the textbook anchors the BSGS must reproduce.
    #[test]
    fn schreier_sims_reproduces_textbook_group_orders() {
        let fact = |n: u128| (1..=n).product::<u128>();
        // S_n = ⟨(0 1), (0 1 … n−1)⟩
        for n in 2..=7usize {
            let transposition: Perm = {
                let mut p = identity(n);
                p.swap(0, 1);
                p
            };
            let cycle: Perm = (0..n).map(|i| (i + 1) % n).collect();
            assert_eq!(schreier_sims(n, &[transposition, cycle]).order(), fact(n as u128), "|S_{n}| = n!");
        }
        // A_n = ⟨3-cycles⟩, generated by (0 1 2) and (0 1 … n−1) for the right parity, but use 3-cycles.
        for n in 3..=7usize {
            let mut gens = Vec::new();
            for k in 2..n {
                // 3-cycle (0 1 k)
                let mut p = identity(n);
                p[0] = 1;
                p[1] = k;
                p[k] = 0;
                gens.push(p);
            }
            assert_eq!(schreier_sims(n, &gens).order(), fact(n as u128) / 2, "|A_{n}| = n!/2");
        }
        // C_n: one n-cycle.
        for n in 1..=12usize {
            let cycle: Perm = (0..n).map(|i| (i + 1) % n).collect();
            assert_eq!(schreier_sims(n, &[cycle]).order(), n as u128, "|C_{n}| = n");
        }
        // D_n on n points: rotation + reflection x ↦ (n−x) mod n.
        for n in 3..=10usize {
            let rot: Perm = (0..n).map(|i| (i + 1) % n).collect();
            let refl: Perm = (0..n).map(|i| (n - i) % n).collect();
            assert_eq!(schreier_sims(n, &[rot, refl]).order(), 2 * n as u128, "|D_{n}| = 2n");
        }
        // The trivial group.
        assert_eq!(schreier_sims(5, &[]).order(), 1, "the empty generating set gives the trivial group");
    }

    /// **Order and membership match brute force, to the point of absurdity.** On a fuzz of random
    /// generating sets over small degrees, the BSGS order equals the brute-force closure size, and
    /// `contains` agrees with closure membership on *every* permutation of the degree — an exhaustive
    /// oracle. Non-members are rejected; members are accepted.
    #[test]
    fn order_and_membership_match_brute_force_exhaustively() {
        let mut state = 0xC0FF_EE42u64;
        for _ in 0..120 {
            let degree = 3 + (splitmix(&mut state) % 4) as usize; // 3..6
            let ngens = 1 + (splitmix(&mut state) % 3) as usize; // 1..3
            let gens: Vec<Perm> = (0..ngens).map(|_| random_perm(degree, &mut state)).collect();
            let group = closure(degree, &gens);
            let bsgs = schreier_sims(degree, &gens);
            assert_eq!(
                bsgs.order(),
                group.len() as u128,
                "|G| must equal the brute-force closure size; gens = {gens:?}"
            );
            for p in all_perms(degree) {
                assert_eq!(
                    bsgs.contains(&p),
                    group.contains(&p),
                    "membership must match brute force for {p:?}; gens = {gens:?}"
                );
            }
        }
    }

    /// **The coset decision** — the rung the abelian linear engines could not reach. `g` lies in the coset
    /// `rep·G` iff `rep⁻¹·g` is a member; checked against brute force for random reps and targets.
    #[test]
    fn coset_membership_decides_non_abelian_cosets() {
        let mut state = 0x5EED_0A5Eu64;
        let degree = 5;
        // A non-abelian group: ⟨(0 1 2 3 4), (0 1)⟩ = S_5.
        let cycle: Perm = (0..degree).map(|i| (i + 1) % degree).collect();
        let transposition: Perm = {
            let mut p = identity(degree);
            p.swap(0, 1);
            p
        };
        // Use a proper subgroup so cosets are non-trivial: the stabilizer-ish ⟨(1 2 3 4), (1 2)⟩ = S_4 on {1,2,3,4}.
        let sub_cycle: Perm = vec![0, 2, 3, 4, 1];
        let sub_swap: Perm = vec![0, 2, 1, 3, 4];
        let _ = (&cycle, &transposition);
        let group = closure(degree, &[sub_cycle.clone(), sub_swap.clone()]);
        let bsgs = schreier_sims(degree, &[sub_cycle, sub_swap]);
        assert_eq!(bsgs.order(), group.len() as u128, "the S_4 subgroup order");
        for _ in 0..200 {
            let rep = random_perm(degree, &mut state);
            let g = random_perm(degree, &mut state);
            // g ∈ rep·G ⟺ rep⁻¹·g ∈ G
            let in_coset = bsgs.contains(&compose(&invert(&rep), &g));
            let brute = group.contains(&compose(&invert(&rep), &g));
            assert_eq!(in_coset, brute, "coset decision must match brute force: rep={rep:?} g={g:?}");
        }
    }

    /// **Enumeration equals the brute-force closure.** The transversal-product enumeration reproduces the
    /// whole group exactly — same size, same elements — and every enumerated element is a member.
    #[test]
    fn elements_enumerates_the_whole_group() {
        let mut state = 0xE1E_0F00Du64;
        for _ in 0..40 {
            let degree = 3 + (splitmix(&mut state) % 3) as usize; // 3..5
            let ngens = 1 + (splitmix(&mut state) % 3) as usize;
            let gens: Vec<Perm> = (0..ngens).map(|_| random_perm(degree, &mut state)).collect();
            let group = closure(degree, &gens);
            let bsgs = schreier_sims(degree, &gens);
            let elems = bsgs.elements(100_000).expect("small group enumerates");
            let as_set: BTreeSet<Perm> = elems.iter().cloned().collect();
            assert_eq!(elems.len(), as_set.len(), "enumeration has no duplicates");
            assert_eq!(as_set, group, "enumeration equals the brute-force closure; gens={gens:?}");
            assert!(elems.iter().all(|g| bsgs.contains(g)), "every enumerated element is a member");
        }
        // The order gate declines an oversized group rather than enumerate it.
        let n = 8;
        let cycle: Perm = (0..n).map(|i| (i + 1) % n).collect();
        let swap: Perm = {
            let mut p = identity(n);
            p.swap(0, 1);
            p
        };
        assert!(schreier_sims(n, &[cycle, swap]).elements(1000).is_none(), "|S_8|=40320 > cap ⟹ None");
    }

    /// **The transversal elements are genuine group members, polynomial in count.** `Σ |Δᵢ|` of them
    /// (the additive analogue of `|G| = Π |Δᵢ|`), every one a member — the stabilizer-chain coset reps the
    /// polynomial symmetry break draws on.
    #[test]
    fn transversal_elements_are_polynomial_members() {
        let mut state = 0x7AB_5E70u64;
        for _ in 0..30 {
            let degree = 3 + (splitmix(&mut state) % 4) as usize;
            let ngens = 1 + (splitmix(&mut state) % 3) as usize;
            let gens: Vec<Perm> = (0..ngens).map(|_| random_perm(degree, &mut state)).collect();
            let bsgs = schreier_sims(degree, &gens);
            let reps = bsgs.transversal_elements();
            assert!(reps.iter().all(|g| bsgs.contains(g)), "every transversal element is a member");
            assert!(reps.len() <= degree * degree, "polynomial count (≤ degree²): {}", reps.len());
            // Σ|Δᵢ| ≥ k (one per level) and the product of orbit sizes is |G| — the additive vs multiplicative.
            assert!(reps.len() as u128 >= bsgs.base.len() as u128, "at least one rep per base level");
        }
    }

    /// Orbits reflect the group action: a transitive group is one orbit; a point-stabilizer leaves that
    /// point a singleton; the trivial group is all singletons.
    #[test]
    fn orbits_match_the_group_action() {
        let cycle: Perm = vec![1, 2, 3, 0]; // (0 1 2 3)
        let swap: Perm = vec![1, 0, 2, 3]; // (0 1)
        assert_eq!(orbits(4, &[cycle, swap]), vec![vec![0, 1, 2, 3]], "S_4 is transitive");
        let three: Perm = vec![0, 2, 3, 1]; // (1 2 3), fixes 0
        assert_eq!(orbits(4, &[three]), vec![vec![0], vec![1, 2, 3]], "stabilizer of 0");
        assert_eq!(orbits(3, &[]), vec![vec![0], vec![1], vec![2]], "trivial group: all singletons");
    }

    /// **Primitivity vs imprimitivity.** `S_4`'s natural action and `C_5` (prime) are primitive — no
    /// non-trivial block system. `C_6` (composite) is imprimitive: it decomposes into size-2 blocks (the
    /// internal structure of the symmetry). Block systems are balanced and partition the points.
    #[test]
    fn block_systems_detect_primitivity_and_imprimitivity() {
        let s4: Vec<Perm> = vec![vec![1, 0, 2, 3], vec![1, 2, 3, 0]]; // (0 1), (0 1 2 3)
        assert!(is_primitive(4, &s4), "S_4 natural action is primitive");
        assert!(minimal_block_system(4, &s4).is_none());

        let c5: Vec<Perm> = vec![vec![1, 2, 3, 4, 0]];
        assert!(is_primitive(5, &c5), "C_5 is primitive (5 is prime)");

        let c6: Vec<Perm> = vec![vec![1, 2, 3, 4, 5, 0]];
        assert!(!is_primitive(6, &c6), "C_6 is imprimitive");
        let bs = minimal_block_system(6, &c6).expect("C_6 has a non-trivial block system");
        assert!(bs.iter().all(|b| b.len() == 2), "C_6 minimal blocks have size 2: {bs:?}");
        assert_eq!(bs.len(), 3, "three blocks");
        assert_eq!(bs.iter().map(|b| b.len()).sum::<usize>(), 6, "the blocks partition all points");
    }

    #[test]
    fn orbitals_give_the_rank_and_higmans_primitivity() {
        // Adjacent transpositions generate Sₙ; an n-cycle generates Cₙ.
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };

        // (degree, generators, expected rank, expected primitivity).
        let cases: Vec<(usize, Vec<Perm>, usize, bool)> = vec![
            (3, s_n(3), 2, true),  // S₃ is 2-transitive ⇒ rank 2, primitive
            (4, s_n(4), 2, true),  // S₄ is 2-transitive ⇒ rank 2, primitive
            (4, c_n(4), 4, false), // C₄ is regular ⇒ rank 4; 4 composite ⇒ imprimitive (blocks {0,2},{1,3})
            (5, c_n(5), 5, true),  // C₅ is regular ⇒ rank 5; 5 prime ⇒ primitive
            (6, c_n(6), 6, false), // C₆ is regular ⇒ rank 6; 6 composite ⇒ imprimitive
        ];

        for (deg, gens, want_rank, want_prim) in cases {
            assert_eq!(rank(deg, &gens), want_rank, "rank of the group on {deg} points");
            // The diagonal is always exactly one orbital.
            let diag = orbitals(deg, &gens).into_iter().filter(|o| o.iter().all(|&(i, j)| i == j)).count();
            assert_eq!(diag, 1, "the diagonal is a single orbital");
            // Higman's criterion agrees with the block-system primitivity test — two independent routes.
            assert_eq!(is_primitive_via_orbitals(deg, &gens), want_prim, "Higman primitivity");
            assert_eq!(
                is_primitive_via_orbitals(deg, &gens),
                is_primitive(deg, &gens),
                "orbital (Higman) and block-system primitivity must agree"
            );
        }
    }

    #[test]
    fn transitivity_ladder_climbs_the_tuple_orbits() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };

        // Sₙ is sharply n-transitive: transitive on every distinct tuple. Capped at max_t.
        assert_eq!(transitivity_degree(4, &s_n(4), 3), 3, "S₄ is ≥3-transitive");
        assert_eq!(transitivity_degree(3, &s_n(3), 5), 3, "S₃ is exactly 3-transitive on 3 points");
        // A regular cyclic group is transitive but never 2-transitive.
        assert_eq!(transitivity_degree(4, &c_n(4), 3), 1, "C₄ is 1-transitive only");
        assert_eq!(transitivity_degree(5, &c_n(5), 3), 1, "C₅ is 1-transitive only");

        // k=1 tuple-orbits coincide with the point-orbits; for transitive groups a single orbit.
        assert_eq!(orbits_on_tuples(4, &s_n(4), 1).len(), orbits(4, &s_n(4)).len());
        // 2-transitive ⟺ a single orbit on distinct pairs; Sₙ qualifies, Cₙ (n>2) does not.
        assert_eq!(orbits_on_tuples(4, &s_n(4), 2).len(), 1, "S₄ is 2-transitive");
        assert!(orbits_on_tuples(4, &c_n(4), 2).len() > 1, "C₄ is not 2-transitive");
    }

    #[test]
    fn derived_series_decides_solvability() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let order = |deg: usize, g: &[Perm]| schreier_sims(deg, g).order();

        // Cyclic groups are abelian ⇒ trivial derived subgroup, solvable.
        assert!(is_abelian(4, &c_n(4)), "C₄ is abelian");
        assert!(is_solvable(4, &c_n(4)));
        assert_eq!(order(4, &derived_subgroup(4, &c_n(4))), 1, "[C₄,C₄] is trivial");

        // [Sₙ, Sₙ] = Aₙ (order n!/2). S₃, S₄ are solvable; S₅ is NOT (it contains the simple A₅).
        assert!(!is_abelian(3, &s_n(3)));
        assert_eq!(order(3, &derived_subgroup(3, &s_n(3))), 3, "[S₃,S₃] = A₃ (order 3)");
        assert!(is_solvable(3, &s_n(3)), "S₃ is solvable");

        assert_eq!(order(4, &derived_subgroup(4, &s_n(4))), 12, "[S₄,S₄] = A₄ (order 12)");
        assert!(is_solvable(4, &s_n(4)), "S₄ is solvable");

        assert_eq!(order(5, &derived_subgroup(5, &s_n(5))), 60, "[S₅,S₅] = A₅ (order 60)");
        assert!(!is_solvable(5, &s_n(5)), "S₅ is NOT solvable — A₅ is perfect");
    }

    #[test]
    fn conjugacy_classes_partition_the_group_and_find_the_centre() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };

        // S₃ has 3 conjugacy classes (e · transpositions · 3-cycles), sizes 1+3+2 = 6 = |S₃|; trivial centre.
        let s3 = conjugacy_classes(3, &s_n(3), 1000).expect("S₃ is enumerable");
        assert_eq!(s3.len(), 3, "S₃ has 3 conjugacy classes (= 3 irreps)");
        assert_eq!(s3.iter().map(|c| c.len()).sum::<usize>(), 6, "the classes partition S₃");
        assert_eq!(center_order(3, &s_n(3), 1000), Some(1), "S₃ has a trivial centre");

        // S₄ has 5 conjugacy classes (1+6+3+8+6 = 24); trivial centre.
        let s4 = conjugacy_classes(4, &s_n(4), 1000).expect("S₄ is enumerable");
        assert_eq!(s4.len(), 5, "S₄ has 5 conjugacy classes (= 5 irreps)");
        assert_eq!(s4.iter().map(|c| c.len()).sum::<usize>(), 24, "the classes partition S₄");
        assert_eq!(center_order(4, &s_n(4), 1000), Some(1), "S₄ has a trivial centre");

        // An abelian group: every element is its own class, and the whole group is its centre.
        assert_eq!(conjugacy_classes(6, &c_n(6), 1000).map(|c| c.len()), Some(6), "C₆ has |C₆| classes");
        assert_eq!(center_order(6, &c_n(6), 1000), Some(6), "an abelian group is its own centre");
    }

    #[test]
    fn exponent_and_order_spectrum() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let spec = |s: BTreeSet<usize>| -> Vec<usize> { s.into_iter().collect() };

        // C₄: element orders {1,2,4}; exponent 4 (it has an order-4 element).
        assert_eq!(spec(element_orders(4, &c_n(4), 1000).unwrap()), vec![1, 2, 4]);
        assert_eq!(exponent(4, &c_n(4), 1000), Some(4), "C₄ has exponent 4");
        // C₆: orders {1,2,3,6}, exponent 6.
        assert_eq!(spec(element_orders(6, &c_n(6), 1000).unwrap()), vec![1, 2, 3, 6]);
        assert_eq!(exponent(6, &c_n(6), 1000), Some(6));

        // S₃: element orders {1,2,3}, exponent lcm = 6.
        assert_eq!(spec(element_orders(3, &s_n(3), 1000).unwrap()), vec![1, 2, 3]);
        assert_eq!(exponent(3, &s_n(3), 1000), Some(6), "S₃ has exponent 6");
        // S₄: element orders {1,2,3,4}, exponent lcm(1,2,3,4) = 12.
        assert_eq!(spec(element_orders(4, &s_n(4), 1000).unwrap()), vec![1, 2, 3, 4]);
        assert_eq!(exponent(4, &s_n(4), 1000), Some(12), "S₄ has exponent 12");
    }

    #[test]
    fn cycle_index_drives_polya_counting() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };

        // Cycle-type distribution of S₃: id [1,1,1]×1, transpositions [1,2]×3, 3-cycles [3]×2.
        let ci = cycle_index(3, &s_n(3), 1000).unwrap();
        assert_eq!(ci.get(&vec![1, 1, 1]), Some(&1));
        assert_eq!(ci.get(&vec![1, 2]), Some(&3));
        assert_eq!(ci.get(&vec![3]), Some(&2));

        // Pólya with m = 2 = the number of distinct {0,1}-assignments up to the group. Classic values:
        // C₄ → 6 binary necklaces of length 4; S₃ → 4 (assignments of 3 points by weight).
        assert_eq!(polya_count(4, &c_n(4), 2, 1000), Some(6), "6 binary necklaces of length 4");
        assert_eq!(polya_count(3, &s_n(3), 2, 1000), Some(4), "4 binary 3-point assignments up to S₃");
        assert_eq!(polya_count(5, &c_n(5), 2, 1000), Some(8), "8 binary necklaces of length 5");

        // Cross-check Pólya(m=2) against the brute orbit count of the 2^degree assignment space.
        let brute_assignment_orbits = |deg: usize, gens: &[Perm]| -> u128 {
            let mut seen = std::collections::HashSet::new();
            let mut orbits = 0u128;
            for x in 0u64..(1u64 << deg) {
                let a: Vec<bool> = (0..deg).map(|i| (x >> i) & 1 == 1).collect();
                if seen.contains(&a) {
                    continue;
                }
                orbits += 1;
                let mut stack = vec![a];
                while let Some(cur) = stack.pop() {
                    if !seen.insert(cur.clone()) {
                        continue;
                    }
                    for g in gens {
                        let mut pm = vec![false; deg];
                        for v in 0..deg {
                            pm[g[v]] = cur[v];
                        }
                        if !seen.contains(&pm) {
                            stack.push(pm);
                        }
                    }
                }
            }
            orbits
        };
        for (deg, gens) in [(4, c_n(4)), (3, s_n(3)), (4, s_n(4))] {
            assert_eq!(
                polya_count(deg, &gens, 2, 1000),
                Some(brute_assignment_orbits(deg, &gens)),
                "Pólya(2) equals the brute assignment-orbit count"
            );
        }

        // Pattern inventory — assignment-orbits split by weight.
        // C₄: binary necklaces by #black beads → 0:1, 1:1, 2:2, 3:1, 4:1 (sum 6).
        assert_eq!(pattern_inventory(4, &c_n(4), 1000), Some(vec![1, 1, 2, 1, 1]));
        // S₃: one orbit per weight class on 3 points → [1,1,1,1] (sum 4).
        assert_eq!(pattern_inventory(3, &s_n(3), 1000), Some(vec![1, 1, 1, 1]));
        // The inventory always sums to Pólya(2).
        for (deg, gens) in [(4, c_n(4)), (3, s_n(3)), (4, s_n(4)), (5, c_n(5))] {
            let inv = pattern_inventory(deg, &gens, 1000).unwrap();
            assert_eq!(inv.iter().sum::<u128>(), polya_count(deg, &gens, 2, 1000).unwrap());
        }
    }

    #[test]
    fn abelianisation_is_the_largest_abelian_quotient() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let v4 = vec![vec![1, 0, 3, 2], vec![2, 3, 0, 1]]; // Klein four-group
        let d4 = vec![vec![1, 2, 3, 0], vec![0, 3, 2, 1]];

        // Sₙ abelianises to C₂ (Sₙ/Aₙ): order 2, exponent 2, cyclic.
        assert_eq!(abelianization(3, &s_n(3), 1000), Some((2, 2)), "S₃ᵃᵇ = C₂");
        assert_eq!(abelianization(4, &s_n(4), 1000), Some((2, 2)), "S₄ᵃᵇ = C₂");
        // An abelian group is its own abelianisation.
        assert_eq!(abelianization(6, &c_n(6), 1000), Some((6, 6)), "C₆ᵃᵇ = C₆ (cyclic)");
        assert_eq!(abelianization(4, &v4, 1000), Some((4, 2)), "V₄ᵃᵇ = V₄ (order 4, exponent 2, NOT cyclic)");
        // D₄ abelianises to C₂ × C₂.
        assert_eq!(abelianization(4, &d4, 1000), Some((4, 2)), "D₄ᵃᵇ = C₂ × C₂");

        // Consistency: the abelianisation order is |G| / |[G,G]|.
        for (deg, gens) in [(3, s_n(3)), (4, s_n(4)), (6, c_n(6)), (4, v4.clone()), (4, d4.clone())] {
            let (ab_order, _) = abelianization(deg, &gens, 1000).unwrap();
            let g = schreier_sims(deg, &gens).order();
            let d = schreier_sims(deg, &derived_subgroup(deg, &gens)).order();
            assert_eq!(ab_order, g / d, "|Gᵃᵇ| = |G| / |[G,G]|");
        }
    }

    #[test]
    fn subgroup_lattice_is_counted() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let v4 = vec![vec![1, 0, 3, 2], vec![2, 3, 0, 1]];

        // Classic subgroup-lattice sizes.
        assert_eq!(subgroup_count(4, &c_n(4), 1000), Some(3), "C₄: 1, C₂, C₄");
        assert_eq!(subgroup_count(6, &c_n(6), 1000), Some(4), "C₆: 1, C₂, C₃, C₆");
        assert_eq!(subgroup_count(3, &s_n(3), 1000), Some(6), "S₃: 1, three C₂, C₃, S₃");
        assert_eq!(subgroup_count(4, &v4, 1000), Some(5), "V₄: 1, three C₂, V₄");
        assert_eq!(subgroup_count(4, &s_n(4), 1000), Some(30), "S₄ has 30 subgroups");
    }

    #[test]
    fn simplicity_detects_the_building_block_groups() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        // A₅ = ⟨(0 1 2), (2 3 4)⟩ on 5 points — the smallest non-abelian simple group, order 60.
        let a5 = vec![vec![1, 2, 0, 3, 4], vec![0, 1, 3, 4, 2]];

        // Cyclic groups of prime order are simple; composite order is not.
        assert_eq!(is_simple(5, &c_n(5), 1000), Some(true), "C₅ is simple (prime order)");
        assert_eq!(is_simple(4, &c_n(4), 1000), Some(false), "C₄ is not simple (has C₂)");
        assert_eq!(is_simple(6, &c_n(6), 1000), Some(false), "C₆ is not simple");
        // S₃ is not simple (A₃ is normal).
        assert_eq!(is_simple(3, &s_n(3), 1000), Some(false), "S₃ is not simple");

        // A₅: simple, non-abelian, order 60.
        assert_eq!(schreier_sims(5, &a5).order(), 60, "A₅ has order 60");
        assert_eq!(is_simple(5, &a5, 1000), Some(true), "A₅ is simple");
        // The structural link: a simple NON-abelian group is exactly an unsolvable building block.
        assert!(!is_abelian(5, &a5) && !is_solvable(5, &a5), "A₅ is non-abelian and unsolvable");
    }

    #[test]
    fn composition_factors_are_the_jordan_holder_decomposition() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let a5 = vec![vec![1, 2, 0, 3, 4], vec![0, 1, 3, 4, 2]];

        // Solvable groups decompose into primes (cyclic Cₚ factors).
        assert_eq!(composition_factor_orders(4, &c_n(4), 1000), Some(vec![2, 2]), "C₄: C₂, C₂");
        assert_eq!(composition_factor_orders(6, &c_n(6), 1000), Some(vec![2, 3]), "C₆: C₂, C₃");
        assert_eq!(composition_factor_orders(3, &s_n(3), 1000), Some(vec![2, 3]), "S₃: C₂, C₃");
        assert_eq!(composition_factor_orders(4, &s_n(4), 1000), Some(vec![2, 2, 2, 3]), "S₄: C₂³, C₃");
        // A₅ is its own composition factor — a non-abelian simple group.
        assert_eq!(composition_factor_orders(5, &a5, 1000), Some(vec![60]), "A₅ is simple");

        // Jordan–Hölder: the factor orders always multiply back to |G|.
        for (deg, gens) in [(4, c_n(4)), (6, c_n(6)), (3, s_n(3)), (4, s_n(4)), (5, a5.clone())] {
            let factors = composition_factor_orders(deg, &gens, 1000).unwrap();
            assert_eq!(factors.iter().product::<u128>(), schreier_sims(deg, &gens).order(), "Π factors = |G|");
        }
    }

    #[test]
    fn sylow_counts_satisfy_sylows_theorem() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let a4 = vec![vec![1, 2, 0, 3], vec![0, 2, 3, 1]]; // ⟨(0 1 2),(1 2 3)⟩, order 12
        let a5 = vec![vec![1, 2, 0, 3, 4], vec![0, 1, 3, 4, 2]];

        // Classic Sylow counts.
        assert_eq!(sylow_counts(3, &s_n(3), 1000), Some(vec![(2, 3), (3, 1)]), "S₃: 3 Sylow-2, 1 Sylow-3");
        assert_eq!(sylow_counts(4, &s_n(4), 1000), Some(vec![(2, 3), (3, 4)]), "S₄: 3 Sylow-2 (D₄), 4 Sylow-3");
        assert_eq!(schreier_sims(4, &a4).order(), 12, "A₄ has order 12");
        assert_eq!(sylow_counts(4, &a4, 1000), Some(vec![(2, 1), (3, 4)]), "A₄: V₄ normal, 4 Sylow-3");
        assert_eq!(sylow_counts(5, &a5, 1000), Some(vec![(2, 5), (3, 10), (5, 6)]), "A₅: 5/10/6 Sylow subgroups");
        // Cyclic ⇒ a unique (normal) Sylow subgroup for each prime.
        assert_eq!(sylow_counts(6, &c_n(6), 1000), Some(vec![(2, 1), (3, 1)]), "C₆: unique Sylow subgroups");

        // Sylow's third theorem: n_p ≡ 1 (mod p) for every group.
        for (deg, gens) in [(3, s_n(3)), (4, s_n(4)), (4, a4.clone()), (5, a5.clone()), (6, c_n(6))] {
            for (p, n_p) in sylow_counts(deg, &gens, 1000).unwrap() {
                assert_eq!(n_p as u128 % p, 1, "n_{p} ≡ 1 (mod {p})");
            }
        }
    }

    #[test]
    fn class_algebra_constants_and_real_classes() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };

        // The class-algebra constants satisfy Cᵢ·Cⱼ = Σ a[i][j][k]·Cₖ, i.e. Σₖ a[i][j][k]·|Cₖ| = |Cᵢ|·|Cⱼ|.
        for (deg, gens) in [(3, s_n(3)), (4, s_n(4)), (4, c_n(4)), (6, c_n(6))] {
            let classes = conjugacy_classes(deg, &gens, 1000).unwrap();
            let a = class_multiplication_coefficients(deg, &gens, 1000).unwrap();
            let k = classes.len();
            for i in 0..k {
                for j in 0..k {
                    let lhs: u128 = (0..k).map(|kk| a[i][j][kk] * classes[kk].len() as u128).sum();
                    let rhs = classes[i].len() as u128 * classes[j].len() as u128;
                    assert_eq!(lhs, rhs, "Σ a[{i}][{j}][k]·|Cₖ| = |Cᵢ|·|Cⱼ|");
                }
            }
        }

        // Real conjugacy classes (C = C⁻¹) = number of real irreducible characters.
        assert_eq!(real_class_count(3, &s_n(3), 1000), Some(3), "S₃: all 3 classes real");
        assert_eq!(real_class_count(4, &s_n(4), 1000), Some(5), "Sₙ: every class is real");
        assert_eq!(real_class_count(4, &c_n(4), 1000), Some(2), "C₄: only e and the order-2 class are real");
        assert_eq!(real_class_count(6, &c_n(6), 1000), Some(2), "C₆: e and the order-2 class");
    }

    #[test]
    fn character_table_matches_the_classical_tables() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let v4 = vec![vec![1, 0, 3, 2], vec![2, 3, 0, 1]]; // Klein four-group
        let d4 = vec![vec![1, 2, 3, 0], vec![0, 3, 2, 1]]; // ⟨(0123),(13)⟩, order 8
        let a5 = vec![vec![1, 2, 0, 3, 4], vec![0, 1, 3, 4, 2]]; // smallest non-abelian simple

        // (degree, gens, |G|, expected sorted irreducible degrees) — the textbook degree sequences.
        let cases: Vec<(usize, Vec<Perm>, u128, Vec<u128>)> = vec![
            (4, c_n(4), 4, vec![1, 1, 1, 1]),       // C₄: four linear characters
            (6, c_n(6), 6, vec![1, 1, 1, 1, 1, 1]), // C₆: six linear characters
            (4, v4.clone(), 4, vec![1, 1, 1, 1]),   // V₄: the sign table
            (3, s_n(3), 6, vec![1, 1, 2]),          // S₃: trivial, sign, standard
            (4, s_n(4), 24, vec![1, 1, 2, 3, 3]),   // S₄
            (4, d4.clone(), 8, vec![1, 1, 1, 1, 2]),// D₄: four linear + one 2-dim
            (5, a5.clone(), 60, vec![1, 3, 3, 4, 5]), // A₅
        ];

        for (deg, gens, order, want_degrees) in cases {
            let table = character_table(deg, &gens, 2000)
                .unwrap_or_else(|| panic!("character_table failed for |G|={order}"));
            let classes = conjugacy_classes(deg, &gens, 2000).unwrap();
            let k = classes.len();

            // One irreducible per conjugacy class, with the classical degrees, and Σ dᵢ² = |G|.
            assert_eq!(table.degrees.len(), k, "#irreducibles = #conjugacy classes (|G|={order})");
            assert_eq!(table.degrees, want_degrees, "degree sequence (|G|={order})");
            assert_eq!(table.degrees.iter().map(|d| d * d).sum::<u128>(), order, "Σ dᵢ² = |G|");

            // The trivial character (all ones) is present.
            assert!(
                table.values.iter().any(|row| row.iter().all(|&x| x == 1)),
                "trivial character present (|G|={order})"
            );

            // The identity-class column equals the degrees (χ_s(1) = d_s); abelian ⇒ every degree 1.
            let id: Perm = (0..deg).collect();
            let id_class = classes.iter().position(|c| c.contains(&id)).unwrap();
            for s in 0..k {
                assert_eq!(
                    table.values[s][id_class] as u128, table.degrees[s],
                    "χ_{s}(1) must equal its degree (|G|={order})"
                );
            }
            if is_abelian(deg, &gens) {
                assert!(table.degrees.iter().all(|&d| d == 1), "abelian ⇒ all degrees 1");
                assert_eq!(table.degrees.len() as u128, order, "abelian ⇒ |G| linear characters");
            }

            let p = table.prime as u128;
            // ROW orthogonality: Σ_r |C_r|·χ_s(C_r)·χ_t(C_r⁻¹) = |G|·δ_{st}  (independent re-check, mod p).
            for s in 0..k {
                for t in 0..k {
                    let mut acc = 0u128;
                    for r in 0..k {
                        let prod = table.values[s][r] as u128
                            * table.values[t][table.inverse_class[r]] as u128
                            % p;
                        acc = (acc + table.class_sizes[r] % p * prod) % p;
                    }
                    let want = if s == t { order % p } else { 0 };
                    assert_eq!(acc, want, "row orthogonality s={s} t={t} (|G|={order})");
                }
            }
            // COLUMN orthogonality: Σ_s χ_s(C_r)·χ_s(C_t⁻¹) = (|G|/|C_r|)·δ_{rt}  (mod p).
            for r in 0..k {
                for t in 0..k {
                    let mut acc = 0u128;
                    for s in 0..k {
                        acc = (acc
                            + table.values[s][r] as u128 * table.values[s][table.inverse_class[t]] as u128)
                            % p;
                    }
                    let want = if r == t { order / table.class_sizes[r] % p } else { 0 };
                    assert_eq!(acc, want, "column orthogonality r={r} t={t} (|G|={order})");
                }
            }
        }

        // Degenerate: the trivial group has the 1×1 table [[1]].
        let triv = character_table(1, &[], 10).unwrap();
        assert_eq!(triv.degrees, vec![1]);
        assert_eq!(triv.values, vec![vec![1]]);

        // The convenience wrapper returns exactly the sorted degrees.
        assert_eq!(irreducible_degrees(5, &a5, 2000), Some(vec![1, 3, 3, 4, 5]));
    }

    #[test]
    fn frobenius_schur_indicators_distinguish_d4_from_q8() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let d4 = vec![vec![1, 2, 3, 0], vec![0, 3, 2, 1]]; // ⟨(0123),(13)⟩, order 8
        // Q₈ in its left-regular representation on its 8 elements 0=1,1=-1,2=i,3=-i,4=j,5=-j,6=k,7=-k:
        // left-multiply by i and by j (computed from i²=j²=k²=-1, ij=k, ji=-k).
        let q8 = vec![vec![2, 3, 1, 0, 6, 7, 5, 4], vec![4, 5, 7, 6, 1, 0, 2, 3]];
        assert_eq!(schreier_sims(8, &q8).order(), 8, "Q₈ has order 8");

        let sorted = |mut v: Vec<i8>| {
            v.sort();
            v
        };

        // Sₙ: every irreducible is real ⇒ all indicators +1.
        assert_eq!(frobenius_schur_indicators(3, &s_n(3), 1000), Some(vec![1, 1, 1]), "S₃ is totally real");
        assert_eq!(
            frobenius_schur_indicators(4, &s_n(4), 1000),
            Some(vec![1, 1, 1, 1, 1]),
            "S₄ is totally real"
        );

        // C₄: trivial + order-2 character are real (+1); the order-4 pair are complex conjugates (0).
        let c4 = frobenius_schur_indicators(4, &c_n(4), 1000).unwrap();
        assert_eq!(sorted(c4.clone()), vec![0, 0, 1, 1], "C₄: two real, one complex-conjugate pair");
        assert_eq!(
            c4.iter().filter(|&&v| v != 0).count(),
            real_class_count(4, &c_n(4), 1000).unwrap(),
            "#real-valued characters = #real classes"
        );

        // THE HEADLINE: D₄ and Q₈ have the SAME character table (degrees [1,1,1,1,2]) and the SAME number
        // of real classes — yet the Frobenius–Schur indicators tell them apart.
        assert_eq!(
            irreducible_degrees(4, &d4, 1000),
            irreducible_degrees(8, &q8, 1000),
            "D₄ and Q₈ share a character table"
        );
        assert_eq!(real_class_count(4, &d4, 1000), real_class_count(8, &q8, 1000), "…and # real classes");

        let fs_d4 = frobenius_schur_indicators(4, &d4, 1000).unwrap();
        let fs_q8 = frobenius_schur_indicators(8, &q8, 1000).unwrap();
        assert_eq!(sorted(fs_d4.clone()), vec![1, 1, 1, 1, 1], "D₄: the 2-dim rep is REAL (+1)");
        assert_eq!(sorted(fs_q8.clone()), vec![-1, 1, 1, 1, 1], "Q₈: the 2-dim rep is QUATERNIONIC (−1)");
        assert_ne!(sorted(fs_d4.clone()), sorted(fs_q8.clone()), "Frobenius–Schur SEPARATES D₄ from Q₈");

        // The counting theorem Σ_s ν_s·d_s = #{g : g²=1}: D₄ has 6 such elements (id + 5 involutions),
        // Q₈ has only 2 (id and −1).
        let degs_d4 = irreducible_degrees(4, &d4, 1000).unwrap();
        let degs_q8 = irreducible_degrees(8, &q8, 1000).unwrap();
        let dot = |nu: &[i8], d: &[u128]| -> i128 { nu.iter().zip(d).map(|(&v, &x)| v as i128 * x as i128).sum() };
        assert_eq!(dot(&fs_d4, &degs_d4), 6, "D₄: 6 square roots of identity");
        assert_eq!(dot(&fs_q8, &degs_q8), 2, "Q₈: only id and −1 square to identity");
    }

    #[test]
    fn isotypic_decomposition_of_the_permutation_character() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let a5 = vec![vec![1, 2, 0, 3, 4], vec![0, 1, 3, 4, 2]];

        for (deg, gens) in [(3, s_n(3)), (4, s_n(4)), (5, a5.clone()), (4, c_n(4)), (6, c_n(6))] {
            let table = character_table(deg, &gens, 2000).unwrap();
            let mult = isotypic_multiplicities(deg, &gens, 2000)
                .unwrap_or_else(|| panic!("isotypic decomposition failed for degree {deg}"));
            // π = Σ_s m_s χ_s, so the three bridge identities to the ACTION must hold exactly.
            assert_eq!(
                mult.iter().zip(&table.degrees).map(|(m, d)| m * d).sum::<u128>(),
                deg as u128,
                "Σ m_s·d_s = dim of the permutation representation (degree {deg})"
            );
            assert_eq!(
                mult.iter().map(|m| m * m).sum::<u128>(),
                rank(deg, &gens) as u128,
                "⟨π,π⟩ = #orbitals = rank (degree {deg})"
            );
            let trivial = table.values.iter().position(|row| row.iter().all(|&x| x == 1)).unwrap();
            assert_eq!(
                mult[trivial],
                orbits(deg, &gens).len() as u128,
                "⟨π,1⟩ = #orbits (Burnside) (degree {deg})"
            );
            // The permutation character itself: identity fixes everything, and the average #fixed points
            // over the group equals the orbit count (Cauchy–Frobenius).
            let pi = permutation_character(deg, &gens, 2000).unwrap();
            assert_eq!(pi[table.identity_class], deg as u128, "the identity fixes all {deg} points");
            let order: u128 = table.degrees.iter().map(|d| d * d).sum();
            let avg_fixed: u128 = table.class_sizes.iter().zip(&pi).map(|(h, f)| h * f).sum();
            assert_eq!(avg_fixed, order * orbits(deg, &gens).len() as u128, "Σ|C_r|·π(C_r) = |G|·#orbits");
        }

        // S₄ on 4 points and A₅ on 5 points are 2-transitive: the permutation rep is trivial ⊕ standard,
        // so exactly two irreducibles appear, each once (rank 2).
        let m_s4 = isotypic_multiplicities(4, &s_n(4), 2000).unwrap();
        assert_eq!(m_s4.iter().filter(|&&m| m > 0).count(), 2, "S₄ on 4 points: trivial ⊕ standard");
        assert!(m_s4.iter().all(|&m| m <= 1), "each at most once (2-transitive ⇒ multiplicity-free)");

        // C₄ acting regularly on 4 points: the regular representation contains every irreducible d_s times;
        // C₄ is abelian (all d_s = 1) so every multiplicity is 1.
        assert_eq!(isotypic_multiplicities(4, &c_n(4), 2000).unwrap(), vec![1, 1, 1, 1], "C₄ regular rep");
    }

    #[test]
    fn table_of_marks_classifies_the_g_sets() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let v4 = vec![vec![1, 0, 3, 2], vec![2, 3, 0, 1]];
        let d4 = vec![vec![1, 2, 3, 0], vec![0, 3, 2, 1]];

        // C₄: subgroup classes 1 ⊂ C₂ ⊂ C₄. The table of marks is the textbook upper-triangular matrix.
        let (orders, m) = table_of_marks(4, &c_n(4), 200).unwrap();
        assert_eq!(orders, vec![1, 2, 4], "subgroup orders 1, 2, 4");
        assert_eq!(m, vec![vec![4, 2, 1], vec![0, 2, 1], vec![0, 0, 1]], "C₄ table of marks");

        // S₃: classes 1 ⊂ C₂ ⊂ C₃ ⊂ S₃ — the classic 4×4 table of marks.
        let (so, sm) = table_of_marks(3, &s_n(3), 200).unwrap();
        assert_eq!(so, vec![1, 2, 3, 6]);
        assert_eq!(
            sm,
            vec![vec![6, 3, 2, 1], vec![0, 1, 0, 1], vec![0, 0, 2, 1], vec![0, 0, 0, 1]],
            "S₃ table of marks"
        );

        // Structural laws hold for every group: the trivial-subgroup row is the index sequence [G:H_j], the
        // full-group column is all ones, the full-group row is e_last, the matrix is triangular, and the
        // diagonal [N(H_i):H_i] is nonzero (so the table is invertible — it determines every G-set).
        for (deg, gens, order) in [(3, s_n(3), 6u128), (4, c_n(4), 4), (4, v4.clone(), 4), (4, d4.clone(), 8), (4, s_n(4), 24)] {
            let (ord, mk) = table_of_marks(deg, &gens, 300).unwrap();
            let k = ord.len();
            assert_eq!(*ord.last().unwrap(), order, "the largest subgroup is G itself");
            for j in 0..k {
                assert_eq!(mk[0][j], order / ord[j], "m(1, H_j) = [G : H_j]");
                assert_eq!(mk[j][k - 1], 1, "every subgroup fixes the single coset of G");
                assert_eq!(mk[k - 1][j], u128::from(j == k - 1), "G fixes a coset of H only when H = G");
                assert!(mk[j][j] >= 1, "diagonal [N(H_j):H_j] is nonzero ⇒ invertible");
                for i in 0..k {
                    if ord[i] > ord[j] {
                        assert_eq!(mk[i][j], 0, "triangular: no mark when |H_i| > |H_j|");
                    }
                }
            }
        }
    }

    #[test]
    fn burnside_ring_multiplies_g_sets() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let v4 = vec![vec![1, 0, 3, 2], vec![2, 3, 0, 1]];

        // S₃ (classes 1, C₂, C₃, S₃ at indices 0..3): the natural 3-point set is G/C₂. Its square — the
        // action on ordered pairs of points — splits into the diagonal (≅ G/C₂) and the off-diagonal
        // (the 6-point regular action ≅ G/1). So (G/C₂) × (G/C₂) = G/1 ⊔ G/C₂.
        let n = burnside_ring_product(3, &s_n(3), 200).unwrap();
        assert_eq!(n[1][1], vec![1, 1, 0, 0], "G/C₂ × G/C₂ = G/1 ⊔ G/C₂ in S₃");

        // The Burnside ring laws, on several groups.
        for (deg, gens, order) in [(3, s_n(3), 6u128), (4, c_n(4), 4), (4, v4.clone(), 4), (4, s_n(4), 24)] {
            let (_o, marks) = table_of_marks(deg, &gens, 300).unwrap();
            let nn = burnside_ring_product(deg, &gens, 300).unwrap();
            let k = marks.len();
            let idx = marks[0].clone(); // marks[0][l] = [G : H_l] = #points of G/H_l
            for a in 0..k {
                for b in 0..k {
                    // Non-negative integer coefficients (a genuine G-set).
                    assert!(nn[a][b].iter().all(|&c| c >= 0), "G-set multiplicities are non-negative");
                    // Commutativity of the product.
                    assert_eq!(nn[a][b], nn[b][a], "Burnside product is commutative");
                    // Point-count: |G/H_a × G/H_b| = Σ_l N·|G/H_l|.
                    let lhs: i128 = (0..k).map(|l| nn[a][b][l] * idx[l] as i128).sum();
                    assert_eq!(lhs, (idx[a] * idx[b]) as i128, "point counts multiply");
                }
                // G/G (the one-point set, last class) is the multiplicative identity.
                let mut id = vec![0i128; k];
                id[a] = 1;
                assert_eq!(nn[k - 1][a], id, "G/G is the identity of the Burnside ring");
            }
            // (G/1)² = |G|·(G/1): the regular set squared is |G| copies of itself.
            let mut want0 = vec![0i128; k];
            want0[0] = order as i128;
            assert_eq!(nn[0][0], want0, "(G/1)² = |G|·(G/1)");
        }
    }

    #[test]
    fn permutation_character_decomposition_bridges_marks_and_characters() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let v4 = vec![vec![1, 0, 3, 2], vec![2, 3, 0, 1]];
        let a5 = vec![vec![1, 2, 0, 3, 4], vec![0, 1, 3, 4, 2]];

        // S₃: subgroup classes 1, C₂, C₃, S₃; irreducibles trivial, sign, standard (degrees 1,1,2). The
        // permutation reps decompose as: G/1 = regular = 1+sign+2·std; G/C₂ (3 points) = triv ⊕ std;
        // G/C₃ (2 points) = triv ⊕ sign; G/S₃ (point) = triv. Each row's Σ M·d equals the coset count.
        let (so, sd, sm) = permutation_character_decomposition(3, &s_n(3), 200).unwrap();
        assert_eq!(so, vec![1, 2, 3, 6]);
        assert_eq!(sd, vec![1, 1, 2], "S₃ irreducible degrees");
        // Rows as multisets (column order is the character table's, sorted by (degree, values)).
        assert_eq!(sm[0], vec![1, 1, 2], "G/1 is the regular representation");
        assert_eq!(sm[3], {
            let mut e = vec![0, 0, 0];
            // the trivial irreducible is the all-ones character; locate it and set 1.
            e[sd.iter().position(|&d| d == 1).unwrap()] = 1; // first degree-1 col is trivial after sort
            e
        }, "G/G is the trivial representation");
        // G/C₂ (3-point natural action) is 2-transitive ⇒ trivial ⊕ standard, so Σ M² = 2 (rank 2).
        assert_eq!(sm[1].iter().map(|&x| x * x).sum::<u128>(), 2, "G/C₂ has rank 2 (2-transitive)");

        // The classical laws, on several groups: the regular rep is the degree vector, every action contains
        // the trivial character once, G/G is trivial, and dimensions add to the coset count.
        for (deg, gens, order) in [(3, s_n(3), 6u128), (4, c_n(4), 4), (4, v4.clone(), 4), (4, s_n(4), 24), (5, a5.clone(), 60)] {
            let (orders, degrees, m) = permutation_character_decomposition(deg, &gens, 2000).unwrap();
            let triv = degrees.iter().position(|&d| {
                // the trivial irreducible has degree 1 and appears once in every row at multiplicity 1
                d == 1
            });
            assert!(triv.is_some());
            assert_eq!(m[0], degrees, "G/1 = regular representation = Σ d_s·χ_s");
            for (i, row) in m.iter().enumerate() {
                let dim: u128 = (0..degrees.len()).map(|s| row[s] * degrees[s]).sum();
                assert_eq!(dim, order / orders[i], "Σ_s M[i][s]·d_s = [G : H_i]");
            }
            // A₅: G/A₄ is the natural 5-point action = trivial ⊕ (the 4-dim irreducible) — 2-transitive.
            // Its row has Σ M² = 2; check SOME row realizes the 2-transitive 5-point action for A₅.
            if order == 60 {
                assert!(m.iter().any(|row| row.iter().map(|&x| x * x).sum::<u128>() == 2 && row.iter().any(|&x| x == 1)),
                    "A₅ has a 2-transitive action (the natural 5-point one)");
            }
        }
    }

    #[test]
    fn subgroup_lattice_mobius_and_generating_tuples() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let v4 = vec![vec![1, 0, 3, 2], vec![2, 3, 0, 1]];

        // For a cyclic group the subgroup lattice is the divisor lattice, so μ(1, Cₙ) is exactly the
        // number-theoretic Möbius function μ(n) — an independent cross-check.
        let nt_mobius = |mut m: usize| -> i128 {
            let mut sign = 1i128;
            let mut d = 2;
            while d * d <= m {
                if m % d == 0 {
                    m /= d;
                    if m % d == 0 {
                        return 0; // a squared prime factor
                    }
                    sign = -sign;
                }
                d += 1;
            }
            if m > 1 {
                sign = -sign; // a remaining prime factor
            }
            sign
        };
        for n in [2usize, 3, 4, 5, 6, 7, 8, 9, 12] {
            assert_eq!(
                mobius_number(n, &c_n(n), 400),
                Some(nt_mobius(n)),
                "μ(1, C_{n}) = number-theoretic μ({n})"
            );
        }
        // Classical Möbius numbers.
        assert_eq!(mobius_number(3, &s_n(3), 400), Some(3), "μ(1, S₃) = 3");
        assert_eq!(mobius_number(4, &v4, 400), Some(2), "μ(1, V₄) = 2");

        // The Eulerian function e_k(G) = #ordered k-tuples generating G — cross-checked against a direct
        // brute-force generation count over G^k.
        let brute_generating = |deg: usize, gens: &[Perm], k: u32| -> i128 {
            let elements: Vec<Perm> =
                subgroup_closure(deg, &gens.iter().cloned().collect()).into_iter().collect();
            let total = elements.len();
            let mut count = 0i128;
            // iterate all k-tuples by mixed-radix index
            let mut tuple = vec![0usize; k as usize];
            'outer: loop {
                let seed: Vec<Perm> = tuple.iter().map(|&t| elements[t].clone()).collect();
                if subgroup_closure(deg, &seed.into_iter().collect()).len() == total {
                    count += 1;
                }
                // increment the mixed-radix tuple
                let mut pos = 0;
                loop {
                    if pos == k as usize {
                        break 'outer;
                    }
                    tuple[pos] += 1;
                    if tuple[pos] < total {
                        break;
                    }
                    tuple[pos] = 0;
                    pos += 1;
                }
            }
            count
        };
        for (deg, gens, k) in [(3, s_n(3), 2u32), (3, s_n(3), 3), (4, c_n(4), 2), (4, v4.clone(), 2), (4, v4.clone(), 3)] {
            assert_eq!(
                generating_tuple_count(deg, &gens, 400, k),
                Some(brute_generating(deg, &gens, k)),
                "Hall's e_{k}(G) must equal the brute-force generating-tuple count"
            );
        }
        // A cyclic group is generated by a single element iff that element is a generator; e_1(Cₙ) = φ(n).
        assert_eq!(generating_tuple_count(6, &c_n(6), 400, 1), Some(2), "e₁(C₆) = φ(6) = 2");
        assert_eq!(generating_tuple_count(5, &c_n(5), 400, 1), Some(4), "e₁(C₅) = φ(5) = 4");
    }

    #[test]
    fn automorphism_group_order_matches_classical_values() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let v4 = vec![vec![1, 0, 3, 2], vec![2, 3, 0, 1]];
        let d4 = vec![vec![1, 2, 3, 0], vec![0, 3, 2, 1]];
        let q8 = vec![vec![2, 3, 1, 0, 6, 7, 5, 4], vec![4, 5, 7, 6, 1, 0, 2, 3]];

        // |Aut(Cₙ)| = Euler totient φ(n); cyclic groups are abelian so Inn is trivial and Out = Aut.
        assert_eq!(automorphism_group_order(4, &c_n(4), 500), Some(2), "|Aut(C₄)| = φ(4) = 2");
        assert_eq!(automorphism_group_order(5, &c_n(5), 500), Some(4), "|Aut(C₅)| = φ(5) = 4");
        assert_eq!(automorphism_group_order(6, &c_n(6), 500), Some(2), "|Aut(C₆)| = φ(6) = 2");
        assert_eq!(outer_automorphism_order(5, &c_n(5), 500), Some(4), "C₅ abelian ⇒ Out = Aut");

        // Sₙ (n ≠ 6) is complete: Aut = Inn = Sₙ, Out trivial.
        assert_eq!(automorphism_group_order(3, &s_n(3), 500), Some(6), "|Aut(S₃)| = 6");
        assert_eq!(outer_automorphism_order(3, &s_n(3), 500), Some(1), "S₃ complete ⇒ Out = 1");
        assert_eq!(automorphism_group_order(4, &s_n(4), 500), Some(24), "|Aut(S₄)| = 24");
        assert_eq!(outer_automorphism_order(4, &s_n(4), 500), Some(1), "S₄ complete ⇒ Out = 1");

        // V₄: Aut = GL(2,𝔽₂) = S₃ (order 6); abelian so Inn trivial and Out = Aut.
        assert_eq!(automorphism_group_order(4, &v4, 500), Some(6), "|Aut(V₄)| = |GL(2,2)| = 6");
        assert_eq!(outer_automorphism_order(4, &v4, 500), Some(6), "V₄ abelian ⇒ Out = Aut = S₃");

        // THE HEADLINE: D₄ and Q₈ share an order (8), a character table, AND |Inn| = 4 — yet the
        // automorphism group separates them, a THIRD invariant beyond Frobenius–Schur and rationality.
        assert_eq!(automorphism_group_order(4, &d4, 500), Some(8), "|Aut(D₄)| = 8");
        assert_eq!(automorphism_group_order(8, &q8, 500), Some(24), "|Aut(Q₈)| = 24 = |S₄|");
        assert_eq!(outer_automorphism_order(4, &d4, 500), Some(2), "Out(D₄) = C₂");
        assert_eq!(outer_automorphism_order(8, &q8, 500), Some(6), "Out(Q₈) = S₃");
        assert_ne!(
            automorphism_group_order(4, &d4, 500),
            automorphism_group_order(8, &q8, 500),
            "Aut SEPARATES D₄ from Q₈"
        );
    }

    #[test]
    fn galois_action_distinguishes_real_from_rational_classes() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let a5 = vec![vec![1, 2, 0, 3, 4], vec![0, 1, 3, 4, 2]];

        // Sₙ is rational (integer character table): every class is rational.
        assert_eq!(rational_class_count(3, &s_n(3), 2000), Some(3), "S₃ is rational");
        assert_eq!(rational_class_count(4, &s_n(4), 2000), Some(5), "S₄ is rational");
        // Cyclic groups: the generator's coprime powers fuse — only e and the involution stay rational.
        assert_eq!(rational_class_count(4, &c_n(4), 2000), Some(2), "C₄: only e, g² rational");
        assert_eq!(rational_class_count(6, &c_n(6), 2000), Some(2), "C₆: only e, g³ rational");
        assert_eq!(rational_class_count(5, &c_n(5), 2000), Some(1), "C₅: the Galois group fuses g..g⁴");

        // THE HEADLINE: A₅ is ambivalent (ALL classes real) yet only 3 are RATIONAL — Galois swaps the two
        // 5-cycle classes, exactly the irrationality of the golden-ratio degree-3 character pair.
        assert_eq!(real_class_count(5, &a5, 2000), Some(5), "A₅: all 5 classes are real");
        assert_eq!(rational_class_count(5, &a5, 2000), Some(3), "A₅: only 3 classes are rational");

        // Cross-check Burnside's rationality theorem on every group: #rational classes = #rational-valued
        // irreducible characters (= rows of the character table constant on each Galois orbit of classes).
        for (deg, gens) in [(3, s_n(3)), (4, s_n(4)), (4, c_n(4)), (6, c_n(6)), (5, c_n(5)), (5, a5.clone())] {
            let orbits = galois_class_orbits(deg, &gens, 2000).unwrap();
            let t = character_table(deg, &gens, 2000).unwrap();
            // Orbits partition the classes.
            assert_eq!(orbits.iter().map(|o| o.len()).sum::<usize>(), t.degrees.len(), "orbits partition");
            let rational_chars = (0..t.degrees.len())
                .filter(|&s| orbits.iter().all(|o| o.iter().all(|&r| t.values[s][r] == t.values[s][o[0]])))
                .count();
            assert_eq!(
                rational_chars,
                rational_class_count(deg, &gens, 2000).unwrap(),
                "Burnside: #rational characters = #rational classes (degree {deg})"
            );
            // Rational ⟹ real, always.
            assert!(
                rational_class_count(deg, &gens, 2000).unwrap() <= real_class_count(deg, &gens, 2000).unwrap(),
                "rational classes ⊆ real classes (degree {deg})"
            );
        }
    }

    #[test]
    fn tensor_decomposition_is_the_representation_ring() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let a5 = vec![vec![1, 2, 0, 3, 4], vec![0, 1, 3, 4, 2]];

        for (deg, gens) in [(3, s_n(3)), (4, s_n(4)), (5, a5.clone()), (4, c_n(4)), (6, c_n(6))] {
            let t = character_table(deg, &gens, 2000).unwrap();
            let n = tensor_decomposition(deg, &gens, 2000)
                .unwrap_or_else(|| panic!("tensor decomposition failed for degree {deg}"));
            let k = t.degrees.len();
            // χ_i ⊗ χ_i ⊇ trivial  ⟺  χ_i is self-dual (real) — the FROBENIUS–SCHUR link to #35.
            let fs = frobenius_schur_indicators(deg, &gens, 2000).unwrap();
            let trivial = t.values.iter().position(|row| row.iter().all(|&x| x == 1)).unwrap();
            for i in 0..k {
                let self_dual = n[i][i][trivial] == 1;
                assert_eq!(self_dual, fs[i] != 0, "χ_i⊗χ_i ⊇ 1 iff χ_i is real (degree {deg}, irrep {i})");
                // The trivial appears in χ_i ⊗ χ_j for EXACTLY ONE j (the dual χ_i*), with multiplicity 1.
                assert_eq!(
                    (0..k).filter(|&j| n[i][j][trivial] == 1).count(),
                    1,
                    "χ_i has a unique dual (degree {deg}, irrep {i})"
                );
            }
        }

        // C₄ is the cyclic group ℤ/4: its characters form the dual group ℤ/4, so fusion is ADDITION mod 4,
        // χ_a ⊗ χ_b = χ_{a+b mod 4}. Recover the +mod-4 Cayley table from the fusion coefficients.
        let t = character_table(4, &c_n(4), 2000).unwrap();
        let n = tensor_decomposition(4, &c_n(4), 2000).unwrap();
        // Index each linear character by the value it assigns the generator (its "frequency" in GF(p)).
        let gen_class = (0..4).find(|&r| t.class_reps[r] == vec![1, 2, 3, 0]).unwrap();
        let freq: Vec<u64> = (0..4).map(|s| t.values[s][gen_class]).collect();
        for a in 0..4 {
            for b in 0..4 {
                // χ_a ⊗ χ_b is the unique linear character whose frequency is freq[a]·freq[b].
                let prod_freq = (freq[a] as u128 * freq[b] as u128 % t.prime as u128) as u64;
                let want = (0..4).find(|&c| freq[c] == prod_freq).unwrap();
                for c in 0..4 {
                    assert_eq!(
                        n[a][b][c],
                        u128::from(c == want),
                        "C₄ fusion = character-group multiplication"
                    );
                }
            }
        }
    }

    #[test]
    fn upper_central_series_agrees_with_the_lower_one() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };
        let d4 = vec![vec![1, 2, 3, 0], vec![0, 3, 2, 1]];

        // Abelian: the upper series jumps straight to G (Z₀=1 ⊂ Z₁=G).
        assert_eq!(upper_central_series(4, &c_n(4), 1000), Some(vec![1, 4]));
        // D₄ (class 2): {id} ⊂ centre (order 2) ⊂ G (order 8).
        assert_eq!(upper_central_series(4, &d4, 1000), Some(vec![1, 2, 8]));
        // A non-nilpotent group's upper series stalls at a trivial hypercentre.
        assert_eq!(upper_central_series(3, &s_n(3), 1000), Some(vec![1]), "S₃ has a trivial hypercentre");

        // The deep cross-check: upper- and lower-central series have the SAME length (the nilpotency class)
        // for every group, and report non-nilpotency identically.
        for (deg, gens) in [(4, c_n(4)), (4, d4.clone()), (3, s_n(3)), (4, s_n(4)), (5, s_n(5))] {
            assert_eq!(
                upper_central_length(deg, &gens, 1000),
                nilpotency_class(deg, &gens),
                "upper- and lower-central series agree on the nilpotency class"
            );
        }
    }

    #[test]
    fn lower_central_series_decides_nilpotency() {
        let s_n = |n: usize| -> Vec<Perm> {
            (0..n - 1)
                .map(|i| {
                    let mut p: Perm = (0..n).collect();
                    p.swap(i, i + 1);
                    p
                })
                .collect()
        };
        let c_n = |n: usize| -> Vec<Perm> { vec![(1..n).chain(std::iter::once(0)).collect()] };

        // Abelian groups are nilpotent (class ≤ 1).
        assert!(is_nilpotent(4, &c_n(4)), "C₄ is abelian ⇒ nilpotent");

        // D₄ = ⟨(0 1 2 3), (1 3)⟩, order 8 — a 2-group, hence nilpotent (class 2), and non-abelian.
        let d4 = vec![vec![1, 2, 3, 0], vec![0, 3, 2, 1]];
        assert_eq!(schreier_sims(4, &d4).order(), 8, "D₄ has order 8");
        assert!(!is_abelian(4, &d4), "D₄ is non-abelian");
        assert!(is_nilpotent(4, &d4), "D₄ is a 2-group ⇒ nilpotent");

        // S₃ is solvable but NOT nilpotent — its lower central series stalls at A₃ — the smallest such group.
        assert!(is_solvable(3, &s_n(3)) && !is_nilpotent(3, &s_n(3)), "S₃: solvable but not nilpotent");
        // S₄ is solvable but not nilpotent either.
        assert!(is_solvable(4, &s_n(4)) && !is_nilpotent(4, &s_n(4)), "S₄: solvable but not nilpotent");

        // Series DEPTHS: derived length (solvability class) and nilpotency class.
        assert_eq!(derived_length(4, &c_n(4)), Some(1), "C₄ abelian ⇒ derived length 1");
        assert_eq!(nilpotency_class(4, &c_n(4)), Some(1), "C₄ abelian ⇒ nilpotency class 1");
        assert_eq!(nilpotency_class(4, &d4), Some(2), "D₄ has nilpotency class 2");
        assert_eq!(derived_length(3, &s_n(3)), Some(2), "S₃ has derived length 2");
        assert_eq!(nilpotency_class(3, &s_n(3)), None, "S₃ is not nilpotent");
        assert_eq!(derived_length(4, &s_n(4)), Some(3), "S₄ has derived length 3 (S₄ ⊵ A₄ ⊵ V₄ ⊵ 1)");
        assert_eq!(derived_length(5, &s_n(5)), None, "S₅ is not solvable");
    }
}
