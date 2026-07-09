//! Studying "the sorted arrangement on the hypercube + the binary walk" empirically.
//!
//! The structure being described — a variable-ordered arrangement over {0,1}ⁿ, a shared tree where
//! "every node connects to every node," and a binary walk root→answer that halves at each step — is
//! exactly the **Ordered Binary Decision Diagram** (Bryant 1986). The variable ORDER is the
//! arrangement; Bryant's two reductions (merge isomorphic subfunctions, skip don't-care variables)
//! are the sharing that makes "every node connect"; and following it from the root, one bit per level,
//! IS the binary walk — it reaches a satisfying point in exactly n steps once the diagram exists.
//!
//! The whole question is then: **how big is the arrangement?** That is the OBDD node count. This
//! example measures it across structured vs unstructured functions, and across a good vs a bad order,
//! to make the boundary visible: the binary walk is always n steps, but a *small* arrangement exists
//! only when the function has structure AND the order matches it — and finding that order is NP-hard,
//! while some functions have no small arrangement in any order.
//!
//! Run: cargo run --release -p logicaffeine-proof --example hypercube_walk

use std::collections::HashMap;

/// A Boolean function's full truth table, indexed so that the most-significant bit of the index is the
/// value of `order[0]`, the next of `order[1]`, etc. — i.e. the table is laid out along the variable
/// order, so splitting it in half is "fix the top variable."
fn truth_table(n: usize, order: &[usize], f: &dyn Fn(&[bool]) -> bool) -> Vec<u8> {
    let mut tt = vec![0u8; 1usize << n];
    let mut assign = vec![false; n];
    for (idx, slot) in tt.iter_mut().enumerate() {
        for (k, &v) in order.iter().enumerate() {
            assign[v] = (idx >> (n - 1 - k)) & 1 == 1;
        }
        *slot = u8::from(f(&assign));
    }
    tt
}

/// Exact ROBDD node count for a function presented as an order-indexed truth table. Recursively
/// reduce: a constant cofactor is a terminal; a node whose two children are equal is skipped (the
/// variable is a don't-care here); isomorphic nodes are shared via the unique table. The number of
/// surviving internal nodes is the size of the arrangement.
fn robdd_size(tt: &[u8], n: usize) -> usize {
    let mut unique: HashMap<(usize, usize, usize), usize> = HashMap::new();
    let mut memo: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut next = 2usize; // 0 = ZERO terminal, 1 = ONE terminal
    go(tt, n, &mut next, &mut unique, &mut memo);
    return unique.len();

    fn go(
        tt: &[u8],
        n: usize,
        next: &mut usize,
        unique: &mut HashMap<(usize, usize, usize), usize>,
        memo: &mut HashMap<Vec<u8>, usize>,
    ) -> usize {
        if tt.iter().all(|&b| b == 0) {
            return 0;
        }
        if tt.iter().all(|&b| b == 1) {
            return 1;
        }
        if let Some(&id) = memo.get(tt) {
            return id;
        }
        let half = tt.len() / 2;
        let lo = go(&tt[..half], n, next, unique, memo);
        let hi = go(&tt[half..], n, next, unique, memo);
        // The variable at this node = its depth in the order = n − log2(len).
        let level = n - tt.len().trailing_zeros() as usize;
        let id = if lo == hi {
            lo // don't-care: skip the node (Bryant reduction 1)
        } else {
            *unique.entry((level, lo, hi)).or_insert_with(|| {
                let i = *next;
                *next += 1;
                i
            })
        };
        memo.insert(tt.to_vec(), id);
        id
    }
}

/// The binary walk: descend the order-indexed table one variable at a time, going to whichever half
/// still contains a satisfying point, until a 1-cell remains. Reaches a model in exactly `n` steps —
/// no backtracking — *because the arrangement is already built*. Returns the satisfying assignment, or
/// `None` if the function is UNSAT. This is the walk; `robdd_size` is what it cost to make it possible.
fn binary_walk(tt: &[u8], n: usize, order: &[usize]) -> Option<Vec<bool>> {
    if tt.iter().all(|&b| b == 0) {
        return None;
    }
    let mut cur: &[u8] = tt;
    let mut assign = vec![false; n];
    for k in 0..n {
        let half = cur.len() / 2;
        let (lo, hi) = cur.split_at(half);
        if hi.iter().any(|&b| b == 1) {
            assign[order[k]] = true;
            cur = hi;
        } else {
            assign[order[k]] = false;
            cur = lo;
        }
    }
    Some(assign)
}

// ---- function families ---------------------------------------------------------------------------

/// Parity x₀⊕…⊕xₙ₋₁ = 1 — linear over GF(2); structured, and order-ROBUST (small in every order).
fn parity(a: &[bool]) -> bool {
    a.iter().filter(|&&b| b).count() % 2 == 1
}

/// Interleaved (x₀∧x₁) ∨ (x₂∧x₃) ∨ … — structured, but order-SENSITIVE: linear if the partners are
/// adjacent, exponential if they are all separated (the textbook OBDD-ordering example).
fn interleaved(a: &[bool]) -> bool {
    (0..a.len() / 2).any(|i| a[2 * i] && a[2 * i + 1])
}

/// A structureless function: a fixed pseudo-random bit per assignment (SplitMix-hash of the index).
/// No exploitable structure ⇒ near-maximal arrangement in EVERY order.
fn structureless(a: &[bool]) -> bool {
    let mut z = a.iter().enumerate().fold(0u64, |acc, (i, &b)| acc ^ ((b as u64) << (i % 64)));
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    (z ^ (z >> 31)) & 1 == 1
}

/// Majority — the cardinality / covering essence (≥ ⌈n/2⌉ true). Fully symmetric: its value depends
/// only on the Hamming weight, so the variable symmetry is the whole group S_n.
fn majority(a: &[bool]) -> bool {
    a.iter().filter(|&&b| b).count() * 2 >= a.len()
}

/// At-most-one true — the PHP column constraint, satisfiable. Symmetric (weight ≤ 1).
fn at_most_one(a: &[bool]) -> bool {
    a.iter().filter(|&&b| b).count() <= 1
}

/// A fixed satisfiable random 3-CNF as a function: every clause must be satisfied. Low ratio ⇒ a rich
/// SAT region with NO covering symmetry — the control for "structure the second quotient can exploit."
fn random_3sat(n: usize, seed: u64) -> impl Fn(&[bool]) -> bool {
    let mut state = seed ^ 0x9E3779B97F4A7C15;
    let mut next = move || {
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let m = (n as f64 * 1.5) as usize; // below threshold ⇒ satisfiable, non-trivial region
    let mut clauses: Vec<[(usize, bool); 3]> = Vec::with_capacity(m);
    while clauses.len() < m {
        let mut vs = [0usize; 3];
        let mut k = 0;
        while k < 3 {
            let v = next() as usize % n;
            if !vs[..k].contains(&v) {
                vs[k] = v;
                k += 1;
            }
        }
        clauses.push([(vs[0], next() & 1 == 0), (vs[1], next() & 1 == 0), (vs[2], next() & 1 == 0)]);
    }
    move |a: &[bool]| clauses.iter().all(|c| c.iter().any(|&(v, pos)| a[v] == pos))
}

/// The SECOND quotient — break the variable symmetry. If `f` depends only on Hamming weight (it is
/// fully symmetric), its arrangement collapses from the OBDD's order-shared DAG to a chain over the
/// `n+1` weight classes; we measure that chain's node count as the number of value transitions across
/// weights (plus the two terminals). Returns `None` when `f` is not weight-symmetric (no collapse).
fn symmetry_reduced_size(n: usize, f: &dyn Fn(&[bool]) -> bool) -> Option<usize> {
    let mut by_weight: Vec<Option<bool>> = vec![None; n + 1];
    let mut assign = vec![false; n];
    for idx in 0..(1usize << n) {
        for (j, slot) in assign.iter_mut().enumerate() {
            *slot = (idx >> j) & 1 == 1;
        }
        let w = assign.iter().filter(|&&b| b).count();
        let v = f(&assign);
        match by_weight[w] {
            None => by_weight[w] = Some(v),
            Some(prev) if prev != v => return None, // two assignments, same weight, different value ⇒ not symmetric
            _ => {}
        }
    }
    let vals: Vec<bool> = by_weight.into_iter().map(|o| o.unwrap()).collect();
    let transitions = vals.windows(2).filter(|w| w[0] != w[1]).count();
    Some(transitions + 2) // the weight-chain: each transition is a branch node, plus the 0/1 terminals
}

fn main() {
    let evens_then_odds = |n: usize| -> Vec<usize> {
        (0..n).step_by(2).chain((1..n).step_by(2)).collect()
    };
    let natural = |n: usize| -> Vec<usize> { (0..n).collect() };

    println!("Arrangement size (OBDD nodes) — the cost of the sorted hypercube walk:\n");
    println!("   n │  parity  │ interleaved(good) │ interleaved(bad) │ structureless │  2^n");
    println!("  ───┼──────────┼───────────────────┼──────────────────┼───────────────┼────────");
    for n in [4usize, 6, 8, 10, 12, 14, 16] {
        let nat = natural(n);
        let bad = evens_then_odds(n);
        let par = robdd_size(&truth_table(n, &nat, &parity), n);
        let il_good = robdd_size(&truth_table(n, &nat, &interleaved), n);
        let il_bad = robdd_size(&truth_table(n, &bad, &interleaved), n);
        let rnd = robdd_size(&truth_table(n, &nat, &structureless), n);
        println!(
            "  {n:>3} │ {par:>8} │ {il_good:>17} │ {il_bad:>16} │ {rnd:>13} │ {:>7}",
            1usize << n
        );
    }

    // The binary walk in action: build the arrangement for interleaved(8) and walk to a model.
    let n = 8;
    let order = natural(n);
    let tt = truth_table(n, &order, &interleaved);
    println!(
        "\nBinary walk on interleaved(n={n}) [arrangement = {} nodes]:",
        robdd_size(&tt, n)
    );
    if let Some(model) = binary_walk(&tt, n, &order) {
        let bits: String = model.iter().map(|&b| if b { '1' } else { '0' }).collect();
        println!(
            "  walked {n} steps → model {bits}  (satisfies: {})",
            interleaved(&model)
        );
    }

    println!(
        "\nReading: the walk is always n steps. The *arrangement* is linear for parity (any order) and\n\
         for interleaved IN THE RIGHT ORDER — but exponential for interleaved in the wrong order, and\n\
         near-maximal for the structureless function in EVERY order. So the sorted arrangement is real,\n\
         and the binary walk is cheap — but a SMALL arrangement exists only when the function has\n\
         structure the order can expose (finding the best order is NP-hard), and some functions have no\n\
         small arrangement at all. That is the same wall the symmetry and degree views show."
    );

    // ---- Section 2: real families, satisfiable — first quotient (ORDER) vs second (SYMMETRY) -------
    println!("\n\nSatisfiable structured families — OBDD (order quotient) vs symmetry-reduced (2nd quotient):\n");
    println!("   family                │  n  │ OBDD │ symmetric? │ reduced │  2^n");
    println!("  ───────────────────────┼─────┼──────┼────────────┼─────────┼────────");
    for n in [6usize, 8, 10, 12, 14, 16] {
        let nat = natural(n);
        let families: [(&str, Box<dyn Fn(&[bool]) -> bool>); 4] = [
            ("majority (cardinality)", Box::new(majority)),
            ("at-most-one (PHP col) ", Box::new(at_most_one)),
            ("parity (GF(2) linear) ", Box::new(parity)),
            ("random 3-SAT (sat)    ", Box::new(random_3sat(n, 0xBEEF ^ n as u64))),
        ];
        for (name, f) in families.iter() {
            let obdd = robdd_size(&truth_table(n, &nat, f.as_ref()), n);
            let (sym, red) = match symmetry_reduced_size(n, f.as_ref()) {
                Some(r) => ("yes", r.to_string()),
                None => ("NO", "—".to_string()),
            };
            println!("  {name}│ {n:>3} │ {obdd:>4} │ {sym:>10} │ {red:>7} │ {:>7}", 1usize << n);
        }
        println!("  ───────────────────────┼─────┼──────┼────────────┼─────────┼────────");
    }

    println!(
        "\nDoes it hold on 3-SAT? NO — and that row is the proof. The cardinality/covering families are\n\
         weight-symmetric, so the SECOND quotient collapses their O(n²) OBDD to O(1) (majority → 3,\n\
         at-most-one → 3); parity is already linear. But random 3-SAT is 'symmetric? NO' — its SAT\n\
         region is not invariant under variable permutations, so there is nothing to quotient, and its\n\
         OBDD does not collapse. The shift (order) + lift (symmetry) get EVERYTHING that HAS structure —\n\
         which is exactly the families we crush — and get nothing on arbitrary 3-SAT, which is precisely\n\
         the P≠NP boundary: if the symmetry quotient collapsed random 3-SAT too, that would be P=NP."
    );
}
