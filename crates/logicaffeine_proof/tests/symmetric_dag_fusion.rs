//! **The symmetry-fused DAG: twist-edges, and the compound crush.**
//!
//! The memoized unfolding merged IDENTICAL cofactors. This fuses in the symmetry breaker: merge
//! cofactors that are EQUIVALENT under the family's automorphism group — the DAG's edges now
//! carry explicit **twists** (literal renamings π), and the certificate reads exactly as the
//! program demands: one node per structural class, every child reached by twisting. Soundness is
//! still structural induction — unsatisfiability is isomorphism-invariant, and each twist is
//! verified LOCALLY by the zero-trust checker (recompute the cofactor, apply the stored π,
//! demand exact equality with the child). A corrupted twist is rejected.
//!
//! The compound crush, measured on pigeonhole: plain memoization cannot merge cofactors that
//! differ by a pigeon/hole swap; the fused DAG can — node counts drop again on top of L12's
//! collapse. And the poetry underneath, asserted exactly: the super-family's OWN unfolding is a
//! CHAIN (`n + 1` nodes — every cofactor of the cube is the smaller cube, both branches
//! identical), so the entire picture is: one linear source, twisting into all the families.

use logicaffeine_proof::cdcl::Lit;
use logicaffeine_proof::hypercube::php_perm_symmetries;
use logicaffeine_proof::proof::Perm;
use std::collections::HashMap;

type CanonClauses = Vec<Vec<(u32, bool)>>;

fn canon(clauses: &[Vec<(u32, bool)>]) -> CanonClauses {
    let mut out: CanonClauses = clauses
        .iter()
        .map(|c| {
            let mut lits = c.clone();
            lits.sort_unstable();
            lits.dedup();
            lits
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

fn cofactor(clauses: &CanonClauses, x: u32, b: bool) -> CanonClauses {
    canon(
        &clauses
            .iter()
            .filter(|c| !c.iter().any(|&(v, pos)| v == x && pos == b))
            .map(|c| c.iter().copied().filter(|&(v, _)| v != x).collect())
            .collect::<Vec<_>>(),
    )
}

/// A twist: an explicit literal renaming `var → (var′, flip)`, stored on the edge and verified
/// by the checker.
type Twist = Vec<(u32, u32, bool)>;

fn apply_twist(clauses: &CanonClauses, twist: &Twist) -> Option<CanonClauses> {
    let map: HashMap<u32, (u32, bool)> =
        twist.iter().map(|&(a, b, f)| (a, (b, f))).collect();
    let mut out = Vec::new();
    for c in clauses {
        let mut nc = Vec::new();
        for &(v, pos) in c {
            let &(v2, f) = map.get(&v)?;
            nc.push((v2, pos ^ f));
        }
        out.push(nc);
    }
    Some(canon(&out))
}

/// Deterministic name-normalization: rename variables by first appearance over the sorted clause
/// list, iterated to a fixpoint of (rename, sort). Returns the normalized set and the renaming.
fn normalize(clauses: &CanonClauses) -> (CanonClauses, Vec<(u32, u32)>) {
    let mut cur = clauses.clone();
    let mut total: HashMap<u32, u32> = HashMap::new();
    for c in clauses.iter().flatten() {
        total.entry(c.0).or_insert(c.0); // identity start
    }
    for _ in 0..3 {
        let mut next_name: u32 = 0;
        let mut ren: HashMap<u32, u32> = HashMap::new();
        for c in &cur {
            for &(v, _) in c {
                ren.entry(v).or_insert_with(|| {
                    let x = next_name;
                    next_name += 1;
                    x
                });
            }
        }
        let renamed: Vec<Vec<(u32, bool)>> = cur
            .iter()
            .map(|c| c.iter().map(|&(v, p)| (ren[&v], p)).collect())
            .collect();
        let renamed = canon(&renamed);
        for (_, tgt) in total.iter_mut() {
            if let Some(&t2) = ren.get(tgt) {
                *tgt = t2;
            }
        }
        if renamed == cur {
            break;
        }
        cur = renamed;
    }
    (cur.clone(), total.into_iter().collect())
}

/// The group-canonical form of a clause set: the minimum, over every group element `g`, of the
/// name-normalized image — plus the twist that realizes it. Two cofactors in the same `G`-orbit
/// canonicalize identically, whatever corner of the orbit they sit in.
fn group_canon(clauses: &CanonClauses, group: &[Perm]) -> (CanonClauses, Twist) {
    let mut best: Option<(CanonClauses, Twist)> = None;
    for g in group {
        let mapped: Vec<Vec<(u32, bool)>> = clauses
            .iter()
            .map(|c| {
                c.iter()
                    .map(|&(v, pos)| {
                        let img = g.apply(Lit::new(v, pos));
                        (img.var(), img.is_positive())
                    })
                    .collect()
            })
            .collect();
        let mapped = canon(&mapped);
        let (normed, ren) = normalize(&mapped);
        let ren_map: HashMap<u32, u32> = ren.into_iter().collect();
        let twist: Twist = clauses
            .iter()
            .flatten()
            .map(|&(v, _)| {
                let img = g.apply(Lit::pos(v));
                (v, ren_map[&img.var()], !img.is_positive())
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        if best.as_ref().map_or(true, |(b, _)| normed < *b) {
            best = Some((normed, twist));
        }
    }
    best.unwrap()
}

#[derive(Clone, Debug)]
enum Node {
    Leaf(CanonClauses),
    Internal { clauses: CanonClauses, var: u32, lo: usize, hi: usize, lo_twist: Twist, hi_twist: Twist },
}

/// The fused unfolding: memoize on the GROUP-canonical form; edges carry the realizing twists.
fn fused_unfold(
    n: usize,
    clauses: &CanonClauses,
    group: &[Perm],
) -> Option<(usize, Vec<Node>, usize)> {
    let mut nodes: Vec<Node> = Vec::new();
    let mut memo: HashMap<(usize, CanonClauses), Option<usize>> = HashMap::new();
    fn go(
        depth: usize,
        n: usize,
        clauses: CanonClauses, // stored in group-canonical, name-normalized form
        nodes: &mut Vec<Node>,
        memo: &mut HashMap<(usize, CanonClauses), Option<usize>>,
        group: &[Perm],
    ) -> Option<usize> {
        if let Some(&hit) = memo.get(&(depth, clauses.clone())) {
            return hit;
        }
        let result = if clauses.iter().any(|c| c.is_empty()) {
            let id = nodes.len();
            nodes.push(Node::Leaf(clauses.clone()));
            Some(id)
        } else if clauses.is_empty() || depth > n {
            None // satisfiable branch
        } else {
            // Branch on the first live variable of the (normalized) subproblem.
            let x = clauses.iter().flatten().map(|&(v, _)| v).min().unwrap();
            let mut children: Vec<(usize, Twist)> = Vec::new();
            let mut ok = true;
            for b in [false, true] {
                let cof = cofactor(&clauses, x, b);
                let (cn, twist) = group_canon(&cof, group);
                match go(depth + 1, n, cn, nodes, memo, group) {
                    Some(id) => children.push((id, twist)),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                let id = nodes.len();
                let (lo, lo_twist) = children[0].clone();
                let (hi, hi_twist) = children[1].clone();
                nodes.push(Node::Internal { clauses: clauses.clone(), var: x, lo, hi, lo_twist, hi_twist });
                Some(id)
            } else {
                None
            }
        };
        memo.insert((depth, clauses), result);
        result
    }
    let (root_canon, _) = group_canon(clauses, group);
    let root = go(0, n, root_canon, &mut nodes, &mut memo, group)?;
    let visits = memo.len();
    Some((root, nodes, visits))
}

/// The plain (identity-group) unfolding, for the head-to-head.
fn plain_unfold(n: usize, clauses: &CanonClauses) -> Option<(usize, Vec<Node>, usize)> {
    fused_unfold(n, clauses, &[Perm::identity(n)])
}

/// **The zero-trust checker with twist verification**: leaves carry `⊥`; each internal node's
/// recomputed cofactor, pushed through the stored twist, must equal the child EXACTLY.
fn check_fused(nodes: &[Node]) -> bool {
    nodes.iter().all(|node| match node {
        Node::Leaf(c) => c.iter().any(|cl| cl.is_empty()),
        Node::Internal { clauses, var, lo, hi, lo_twist, hi_twist } => {
            let child = |id: usize| match &nodes[id] {
                Node::Leaf(c) => c,
                Node::Internal { clauses, .. } => clauses,
            };
            let ok = |b: bool, id: usize, tw: &Twist| {
                apply_twist(&cofactor(clauses, *var, b), tw)
                    .map_or(false, |t| t == *child(id))
            };
            ok(false, *lo, lo_twist) && ok(true, *hi, hi_twist)
        }
    })
}

fn php_clauses(m: usize) -> (usize, CanonClauses) {
    let (php, _) = logicaffeine_proof::families::php(m);
    let cc: Vec<Vec<(u32, bool)>> = php
        .clauses
        .iter()
        .map(|c| c.iter().map(|l| (l.var(), l.is_positive())).collect())
        .collect();
    (php.num_vars, canon(&cc))
}

/// BFS closure of the pigeonhole symmetry generators (the small full group).
fn php_group(m: usize) -> Vec<Perm> {
    let nv = m * (m - 1);
    let gens = php_perm_symmetries(m);
    let key = |p: &Perm| -> Vec<u32> {
        (0..nv).map(|v| p.apply(Lit::pos(v as u32)).var()).collect()
    };
    let id = Perm::identity(nv);
    let mut seen: std::collections::BTreeSet<Vec<u32>> = [key(&id)].into_iter().collect();
    let mut group = vec![id.clone()];
    let mut frontier = vec![id];
    while let Some(p) = frontier.pop() {
        for g in &gens {
            let q = p.compose(g);
            if seen.insert(key(&q)) {
                group.push(q.clone());
                frontier.push(q);
            }
        }
    }
    group
}

/// **The compound crush on pigeonhole, and the checker's teeth.** Plain memoization cannot merge
/// cofactors that differ by a pigeon or hole swap; the fused DAG can — node counts drop again on
/// top of the L12 collapse, measured head-to-head at `m = 3, 4`. Every fused DAG passes the
/// twist-verifying local checker; a corrupted twist is REJECTED.
#[test]
fn the_symmetry_fused_dag_compounds_the_crush_on_pigeonhole() {
    for m in [3usize, 4] {
        let (nv, clauses) = php_clauses(m);
        let group = php_group(m);
        let (_, plain, plain_visits) = plain_unfold(nv, &clauses).expect("PHP unfolds");
        let (_, fused, fused_visits) = fused_unfold(nv, &clauses, &group).expect("PHP unfolds under the group");
        assert!(check_fused(&plain), "m={m}: the plain DAG re-checks");
        assert!(check_fused(&fused), "m={m}: the fused DAG re-checks, twists verified");
        assert!(
            fused.len() < plain.len(),
            "m={m}: the fusion compounds the crush ({} < {})",
            fused.len(),
            plain.len()
        );
        // THE RATCHET (locked 2026-07-03): plain 25 → fused 18 at m = 3; plain 103 → fused 60 at
        // m = 4 — and the compound ratio GROWS with scale (×1.39 → ×1.72). Any regression that
        // loses a merge breaks here.
        let expected = [(3usize, 25usize, 18usize), (4, 103, 60)];
        let (_, ep, ef) = expected.iter().find(|&&(em, ..)| em == m).unwrap();
        assert_eq!(plain.len(), *ep, "m={m}: plain DAG size is locked");
        assert_eq!(fused.len(), *ef, "m={m}: fused DAG size is locked");
        // OUTPUT-SENSITIVITY (the guaranteed-discovery bound): the memoized recursion visits each
        // distinct class ONCE, so the work of FINDING the collapse is linear in the collapse
        // found (visits ≤ 2·nodes + failed/satisfiable probes, each visited once too). If a small
        // twisted DAG exists along the order, it is found in time proportional to its own size —
        // "how long to find one" is bounded by "how small the answer is."
        assert!(
            plain_visits <= 2 * plain.len() + 2 * nv + 2,
            "m={m}: plain finder work {plain_visits} is linear in its output {}",
            plain.len()
        );
        assert!(
            fused_visits <= 2 * fused.len() + 2 * nv + 2,
            "m={m}: fused finder work {fused_visits} is linear in its output {}",
            fused.len()
        );
        eprintln!(
            "output-sensitivity[PHP({m})]: plain visits {plain_visits} vs nodes {}, fused visits \
             {fused_visits} vs nodes {} — time-to-find ≈ size-of-answer",
            plain.len(),
            fused.len()
        );
        eprintln!(
            "fusion[PHP({m})]: plain DAG {} nodes → fused DAG {} nodes (group order {}, compound ×{:.1})",
            plain.len(),
            fused.len(),
            group.len(),
            plain.len() as f64 / fused.len() as f64
        );
    }
    // Teeth: corrupt one twist (flip a polarity) and the checker must refuse.
    let (nv, clauses) = php_clauses(3);
    let group = php_group(3);
    let (_, mut nodes, _) = fused_unfold(nv, &clauses, &group).unwrap();
    let victim = nodes
        .iter()
        .position(|n| matches!(n, Node::Internal { lo_twist, .. } if !lo_twist.is_empty()))
        .expect("an internal node with a nontrivial twist exists");
    if let Node::Internal { lo_twist, .. } = &mut nodes[victim] {
        lo_twist[0].2 = !lo_twist[0].2;
    }
    assert!(!check_fused(&nodes), "a corrupted twist is rejected — the checker has teeth");
}

/// **The super-family's own DAG is a CHAIN — one linear source, twisting into everything.** Every
/// cofactor of the all-corners cube is the smaller cube, on both branches alike, so the plain
/// unfolding collapses to exactly `n + 1` nodes at every scale. Asserted for `n = 3..8`. The full
/// picture, in one sentence: the source of all families is linear; the families are its twists;
/// the twists are verified edges; and the cost of everything is the width of the twisting.
#[test]
fn the_super_familys_own_dag_is_a_chain() {
    for n in 3usize..=8 {
        let cube: Vec<Vec<(u32, bool)>> = (0u64..(1u64 << n))
            .map(|a| (0..n as u32).map(|v| (v, (a >> v) & 1 == 0)).collect())
            .collect();
        let cube = canon(&cube);
        let (_, nodes, _) = plain_unfold(n, &cube).expect("the cube is UNSAT");
        assert!(check_fused(&nodes), "n={n}: the chain re-checks");
        assert_eq!(
            nodes.len(),
            n + 1,
            "n={n}: the super-family's DAG is a chain of exactly n+1 nodes"
        );
    }
    eprintln!(
        "the super-family's own unfolding is LINEAR (n+1 nodes at every scale) — one chain, \
         twisting into all the families; the twists are the certificate edges and the toll is \
         the width of the twisting"
    );
}
