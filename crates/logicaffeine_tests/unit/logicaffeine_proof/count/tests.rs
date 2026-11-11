use super::*;
use crate::families::php;

fn rng(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed;
    move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn brute(num_vars: usize, clauses: &[Vec<Lit>]) -> u64 {
    (0..1u64 << num_vars)
        .filter(|bits| clauses.iter().all(|c| c.iter().any(|l| (bits >> l.var() & 1 == 1) == l.is_positive())))
        .count() as u64
}

fn big(n: u64) -> BigInt {
    BigInt::from_u64(n)
}

fn two_to(k: u64) -> BigInt {
    BigInt::from_u64(1).shl_bits(k)
}

#[test]
fn every_small_formula_counts_as_brute_force_does() {
    let mut next = rng(0xC0E7);
    for round in 0..1500u64 {
        let n = (next() % 15) as usize;
        let m = if n == 0 { 0 } else { (next() % (4 * n as u64 + 2)) as usize };
        let clauses: Vec<Vec<Lit>> = (0..m)
            .map(|_| {
                let width = 1 + (next() % 4) as usize;
                (0..width).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect()
            })
            .collect();
        let got = count_models(n, &clauses);
        assert_eq!(got.models, big(brute(n, &clauses)), "round {round}: n={n} m={m}");
    }
}

#[test]
fn the_empty_formula_has_every_assignment_and_the_empty_clause_none() {
    for n in [0usize, 1, 7, 64, 200] {
        assert_eq!(count_models(n, &[]).models, two_to(n as u64), "{n} free variables");
        assert_eq!(count_models(n, &[vec![]]).models, big(0), "the empty clause with {n} variables");
    }
}

#[test]
fn one_wide_clause_excludes_exactly_one_assignment_of_its_variables() {
    // (x₀ ∨ … ∨ x_{k−1}) over n variables: 2ⁿ − 2^{n−k}.
    for (n, k) in [(5usize, 3usize), (40, 10), (130, 64)] {
        let clause: Vec<Lit> = (0..k as Var).map(Lit::pos).collect();
        let expect = two_to(n as u64).sub(&two_to((n - k) as u64));
        assert_eq!(count_models(n, &[clause]).models, expect, "n={n} k={k}");
    }
}

#[test]
fn a_parity_chain_counts_by_its_rank() {
    // x₀ ⊕ x₁ = 1, x₁ ⊕ x₂ = 1, …: a chain of n − 1 independent equations over n variables leaves 2 models.
    for n in [2usize, 5, 30, 120] {
        let mut clauses = Vec::new();
        for v in 0..n as Var - 1 {
            clauses.push(vec![Lit::pos(v), Lit::pos(v + 1)]);
            clauses.push(vec![Lit::neg(v), Lit::neg(v + 1)]);
        }
        assert_eq!(count_models(n, &clauses).models, big(2), "chain of {n}");
    }
}

#[test]
fn an_unsatisfiable_formula_counts_zero() {
    for holes in [3u32, 4, 5, 6] {
        let (cnf, _) = php(holes as usize + 1);
        assert_eq!(count_models(cnf.num_vars, &cnf.clauses).models, big(0), "PHP({})", holes + 1);
    }
}

#[test]
fn disjoint_parts_multiply_and_are_counted_as_components() {
    // k disjoint copies of a small formula over fresh variables: the count is the copy's count to the k-th power,
    // and the counter splits it into components rather than branching across the copies.
    let base = vec![vec![Lit::pos(0), Lit::pos(1), Lit::neg(2)], vec![Lit::neg(0), Lit::pos(2)], vec![Lit::neg(1), Lit::neg(2), Lit::pos(3)]];
    let one = brute(4, &base);
    for k in [1u32, 2, 5, 20] {
        let mut clauses = Vec::new();
        for copy in 0..k {
            for c in &base {
                clauses.push(c.iter().map(|l| Lit::new(l.var() + 4 * copy, l.is_positive())).collect());
            }
        }
        let got = count_models(4 * k as usize, &clauses);
        assert_eq!(got.models, big(one).pow(k), "{k} copies");
        assert!(got.components >= k as u64, "{k} copies are {k} components ({})", got.components);
    }
}

#[test]
fn counts_past_sixty_four_bits_are_exact() {
    // (x₀ ∨ x₁) with 200 variables: 3·2¹⁹⁸.
    let got = count_models(200, &[vec![Lit::pos(0), Lit::pos(1)]]);
    assert_eq!(got.models, big(3).mul(&two_to(198)));
    assert_eq!(got.models.to_u64(), None);
}

#[test]
fn a_repeated_subformula_is_answered_from_the_cache() {
    // The same component reappears under different assignments of a variable it does not touch.
    let mut clauses = vec![vec![Lit::pos(0), Lit::pos(1)], vec![Lit::neg(0), Lit::pos(1)]];
    for v in 2..14u32 {
        clauses.push(vec![Lit::pos(v), Lit::pos(v + 1), Lit::neg(v + 2)]);
    }
    let got = count_models(18, &clauses);
    assert_eq!(got.models, big(brute(18, &clauses)));
    let again = count_models(18, &clauses);
    assert_eq!(got.models, again.models, "counting is deterministic");
}

#[test]
fn a_random_3sat_formula_at_moderate_size_matches_a_split_count() {
    // Counting F and splitting on x₀ by hand must agree: #F = #(F ∧ x₀) + #(F ∧ ¬x₀).
    let mut next = rng(77);
    for round in 0..20u64 {
        let n = 40usize;
        let clauses: Vec<Vec<Lit>> = (0..120)
            .map(|_| (0..3).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect())
            .collect();
        let whole = count_models(n, &clauses).models;
        let mut with = clauses.clone();
        with.push(vec![Lit::pos(0)]);
        let mut without = clauses.clone();
        without.push(vec![Lit::neg(0)]);
        let split = count_models(n, &with).models.add(&count_models(n, &without).models);
        assert_eq!(whole, split, "round {round}");
    }
}

#[test]
fn one_set_of_variables_left_open_with_different_clauses_is_two_components() {
    // Branching on x leaves y and z open either way: x true leaves y ∨ z (three models), x false leaves y ∨ z and
    // ¬y ∨ ¬z (two). A cache that knew a component by its variables alone would answer the second from the first.
    // Repeated over disjoint copies on fresh variables, so the cache meets each residual many times, under every
    // polarity of the three.
    for phases in 0..8u32 {
        let lit = |v: u32, i: u32| Lit::new(v, phases >> i & 1 == 0);
        let copy = |base: u32| {
            let (x, y, z) = (base, base + 1, base + 2);
            vec![
                vec![lit(x, 0), lit(y, 1), lit(z, 2)],
                vec![lit(x, 0), lit(y, 1).negated(), lit(z, 2).negated()],
                vec![lit(x, 0).negated(), lit(y, 1), lit(z, 2)],
            ]
        };
        let one = copy(0);
        assert_eq!(brute(3, &one), 5);
        assert_eq!(count_models(3, &one).models, big(5), "phases {phases:03b}");
        let clauses: Vec<Vec<Lit>> = (0..6).flat_map(|k| copy(3 * k)).collect();
        assert_eq!(count_models(18, &clauses).models, big(5u64.pow(6)), "phases {phases:03b}");
        let mut chained = clauses.clone();
        chained.push(vec![Lit::pos(0), Lit::pos(3), Lit::pos(6)]);
        assert_eq!(count_models(18, &chained).models, big(brute(18, &chained)), "phases {phases:03b}");
    }
}

/// A complete binary tree of blocks `depth` deep, the leaves' variables numbered first: each block three variables
/// bound by four clauses of random signs, and tied to its parent by one clause over a variable of each, the links that
/// separate the tree.
fn block_tree(depth: u32, seed: u64) -> (usize, Vec<Vec<Lit>>) {
    let mut next = rng(seed);
    let blocks = (1usize << depth) - 1;
    let var = |node: usize, k: usize| ((blocks - 1 - node) * 3 + k) as Var;
    let mut clauses = Vec::new();
    for node in 0..blocks {
        for _ in 0..4 {
            clauses.push((0..3).map(|k| Lit::new(var(node, k), next() & 1 == 0)).collect());
        }
        if node > 0 {
            let parent = (node - 1) / 2;
            let (child, up) = (var(node, 0), var(parent, 1 + node % 2));
            clauses.push(vec![Lit::new(child, next() & 1 == 0), Lit::new(up, next() & 1 == 0)]);
        }
    }
    (3 * blocks, clauses)
}

#[test]
fn a_tree_of_blocks_is_counted_through_its_separators_in_decisions_linear_in_its_blocks() {
    // Branching first on what a min-degree elimination eliminates last cuts the tree at its links, so the decisions
    // grow with the blocks; branching on the variable in the most open clauses takes the tree leaf by leaf and meets
    // every combination of the links it leaves open. Each count checked by brute force where it reaches, and by
    // splitting on a variable past that.
    for depth in 2..=7 {
        let (n, clauses) = block_tree(depth, u64::from(depth));
        let blocks = (1u64 << depth) - 1;
        let counted = count_models(n, &clauses);
        assert!(counted.decisions <= 8 * blocks, "depth {depth}: {} decisions for {blocks} blocks", counted.decisions);
        if n <= 21 {
            assert_eq!(counted.models, big(brute(n, &clauses)), "depth {depth}");
        } else {
            let pick = |l: Lit| {
                let mut with = clauses.clone();
                with.push(vec![l]);
                count_models(n, &with).models
            };
            assert_eq!(counted.models, pick(Lit::pos(0)).add(&pick(Lit::neg(0))), "depth {depth}");
        }
    }
}

#[test]
fn the_elimination_takes_a_star_from_its_leaves_and_gives_up_past_its_fill() {
    // A star of binary clauses: its leaves before the centre but for the last, the centre and that leaf an edge left
    // in either order. A ring of twelve fills in nine edges, twenty-one with its own: eliminated within that budget,
    // given up one below it.
    let star: Vec<Vec<Lit>> = (1..9).map(|v| vec![Lit::pos(0), Lit::neg(v)]).collect();
    let place = elimination(9, &star, FILL).expect("a star fills in nothing");
    assert!(place[0] >= 7, "the centre among the last two: {place:?}");
    let mut sorted = place.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..9).collect::<Vec<u32>>(), "every variable placed once");
    let ring: Vec<Vec<Lit>> = (0..12).map(|v| vec![Lit::pos(v), Lit::pos((v + 1) % 12)]).collect();
    assert!(elimination(12, &ring, 21).is_some(), "twelve edges and nine filled in");
    assert!(elimination(12, &ring, 20).is_none(), "one short");
}

/// Every assignment of `n` variables, as bits, that satisfies `clauses`.
fn models_of(n: usize, clauses: &[Vec<Lit>]) -> Vec<u64> {
    (0..1u64 << n).filter(|&bits| clauses.iter().all(|c| satisfied_by(bits, c))).collect()
}

fn satisfied_by(bits: u64, clause: &[Lit]) -> bool {
    clause.iter().any(|l| (bits >> l.var() & 1 == 1) == l.is_positive())
}

/// `m` clauses of three distinct variables of `n`, of random signs.
fn random_3cnf(n: usize, m: usize, next: &mut impl FnMut() -> u64) -> Vec<Vec<Lit>> {
    (0..m)
        .map(|_| {
            let mut vars: Vec<Var> = Vec::new();
            while vars.len() < 3 {
                let v = (next() % n as u64) as Var;
                if !vars.contains(&v) {
                    vars.push(v);
                }
            }
            vars.into_iter().map(|v| Lit::new(v, next() & 1 == 0)).collect()
        })
        .collect()
}

/// A small formula of one of two shapes: clauses of one to three literals over a few blocks of variables and a few
/// across them; or blocks bound to a few hub variables, so that the components a split leaves under the hubs are
/// often without a model.
fn small_formula(next: &mut impl FnMut() -> u64) -> (usize, Vec<Vec<Lit>>) {
    let mut clauses: Vec<Vec<Lit>> = Vec::new();
    if next() % 2 == 0 {
        let n = 4 + (next() % 9) as usize;
        let blocks = 1 + (next() % 4) as usize;
        for b in 0..blocks {
            let lo = b * n / blocks;
            let size = ((b + 1) * n / blocks).max(lo + 1) - lo;
            for _ in 0..(1 + next() % 7) as usize * size / 2 + 1 {
                let width = 1 + (next() % 3) as usize;
                clauses.push((0..width).map(|_| Lit::new((lo + (next() % size as u64) as usize) as Var, next() & 1 == 0)).collect());
            }
        }
        for _ in 0..next() % 4 {
            let width = 2 + (next() % 3) as usize;
            clauses.push((0..width).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect());
        }
        (n, clauses)
    } else {
        let hubs = 1 + (next() % 3) as usize;
        let block = 3 + (next() % 3) as usize;
        let blocks = (12 - hubs) / block;
        for b in 0..blocks {
            let lo = hubs + b * block;
            for _ in 0..block * (3 + (next() % 3) as usize) {
                let mut c: Vec<Lit> = (0..2 + (next() % 2) as usize)
                    .map(|_| Lit::new((lo + (next() % block as u64) as usize) as Var, next() & 1 == 0))
                    .collect();
                if next() % 2 == 0 {
                    c.push(Lit::new((next() % hubs as u64) as Var, next() & 1 == 0));
                }
                if next() % 6 == 0 {
                    c.push(Lit::new((hubs + (next() % (blocks * block) as u64) as usize) as Var, next() & 1 == 0));
                }
                clauses.push(c);
            }
        }
        (hubs + blocks * block, clauses)
    }
}

/// A counter over `clauses`, normalized as [`count_models`] normalizes them, run on this thread.
fn counted(n: usize, clauses: &[Vec<Lit>], taught: &[Vec<Lit>]) -> (BigInt, Counter) {
    let mut counter = Counter::new(n, clauses.iter().filter_map(|c| normalized(c)).collect());
    for t in taught.iter().filter_map(|c| normalized(c)) {
        counter.teach(t);
    }
    let models = counter.run();
    (models, counter)
}

#[test]
fn every_clause_a_conflict_teaches_holds_in_every_model() {
    // A taught clause is a resolvent of the formula's clauses and of clauses taught before it, so no model falsifies
    // it; one that did would prune models from every count it reached. Half the formulas are small formulas of both
    // shapes, half random 3-CNF of 10 to 14 variables at 4 to 6 clauses each, where conflicts abound.
    let mut next = rng(0x1E4C);
    let mut taught = 0;
    for round in 0..1500u64 {
        let (n, clauses) = if round % 2 == 0 {
            small_formula(&mut next)
        } else {
            let n = 10 + (next() % 5) as usize;
            let m = n * (4 + (next() % 3) as usize);
            (n, random_3cnf(n, m, &mut next))
        };
        let models = models_of(n, &clauses);
        let (got, counter) = counted(n, &clauses, &[]);
        assert_eq!(got, big(models.len() as u64), "round {round}");
        for c in &counter.clauses[counter.original..] {
            assert!(models.iter().all(|&bits| satisfied_by(bits, c)), "round {round}: taught {c:?} drops a model");
        }
        taught += counter.clauses.len() - counter.original;
    }
    assert!(taught >= 2000, "the rounds teach clauses ({taught})");
}

#[test]
fn no_component_key_names_a_taught_clause() {
    // A component is the formula's clauses still open among its variables, the residual formula its key names; a
    // taught clause in a key would make two names for one residual and miss the cache between them.
    let mut next = rng(0xC1A5);
    let mut keys = 0;
    for round in 0..600u64 {
        let (n, clauses) = small_formula(&mut next);
        let (_, counter) = counted(n, &clauses, &[]);
        let taught = counter.clauses.len() - counter.original;
        for key in counter.cache.keys() {
            let at = key.iter().position(|&x| x == u32::MAX).expect("the separator");
            assert!(
                key[at + 1..].iter().all(|&c| (c as usize) < counter.original),
                "round {round}: a key names a taught clause ({taught} taught)"
            );
        }
        if taught > 0 {
            keys += counter.cache.len();
        }
    }
    assert!(keys >= 1000, "keys of searches that taught clauses ({keys})");
}

#[test]
fn a_count_is_exact_whatever_implied_clauses_it_was_taught() {
    // The formula implies every clause the counter may be taught, but a taught clause can cross two components and
    // carry a literal from one into the other: beside a sibling with no model it prunes models of the component being
    // counted, and that count, cached, must not answer for the component where its siblings have models. Each formula
    // is counted with random clauses it implies taught first, against brute force.
    let mut next = rng(0x7A06);
    let mut acted = 0;
    for round in 0..3000u64 {
        let (n, clauses) = small_formula(&mut next);
        let models = models_of(n, &clauses);
        let wanted = 1 + next() % 8;
        let mut taught: Vec<Vec<Lit>> = Vec::new();
        for _ in 0..400 {
            if taught.len() as u64 == wanted {
                break;
            }
            let width = 1 + (next() % 4) as usize;
            let c: Vec<Lit> = (0..width).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect();
            if models.iter().all(|&bits| satisfied_by(bits, &c)) {
                taught.push(c);
            }
        }
        let (got, counter) = counted(n, &clauses, &taught);
        assert_eq!(got, big(models.len() as u64), "round {round}: n={n} taught {taught:?}");
        if counter.taught_acts > 0 {
            acted += 1;
        }
    }
    assert!(acted >= 1000, "rounds in which a taught clause acted ({acted})");
}

/// The number of models of `clauses`, enumerated by the CDCL solver with each model found blocked, when there are at
/// most `limit`.
fn enumerated(n: usize, clauses: &[Vec<Lit>], limit: u64) -> Option<u64> {
    let mut blocked: Vec<Vec<Lit>> = Vec::new();
    loop {
        let mut solver = crate::cdcl::Solver::new(n);
        for c in clauses.iter().chain(&blocked) {
            solver.add_clause(c.clone());
        }
        match solver.solve() {
            crate::cdcl::SolveResult::Unsat => return Some(blocked.len() as u64),
            crate::cdcl::SolveResult::Sat(model) => {
                if blocked.len() as u64 == limit {
                    return None;
                }
                blocked.push((0..n as Var).map(|v| Lit::new(v, !model[v as usize])).collect());
            }
        }
    }
}

#[test]
fn the_clauses_its_conflicts_teach_cut_the_search_of_a_dense_formula() {
    // Random 3-CNF past the threshold: of 100 variables at 4.8 clauses each, six formulas with no model; of 80 at 4.2,
    // eight with none or few. Searched without learning they take 56,987 and 24,515 decisions; each conflict's clause
    // cuts the branches that would meet it again, to under a third. A count of at most a thousand is checked against
    // the CDCL solver's enumeration of the models, a larger one against the count split on a variable.
    for (n, m, seeds, most) in [(100usize, 480usize, 6u64, 56_987u64 / 3), (80, 336, 8, 24_515 / 3)] {
        let mut decisions = 0;
        for seed in 0..seeds {
            let mut next = rng(seed * 1000 + n as u64);
            let clauses = random_3cnf(n, m, &mut next);
            let counted = count_models(n, &clauses);
            decisions += counted.decisions;
            match counted.models.to_u64().filter(|&k| k <= 1000) {
                Some(k) => assert_eq!(enumerated(n, &clauses, k), Some(k), "n={n} seed {seed}: {k} models"),
                None => {
                    let pick = |l: Lit| {
                        let mut with = clauses.clone();
                        with.push(vec![l]);
                        count_models(n, &with).models
                    };
                    assert_eq!(counted.models, pick(Lit::pos(0)).add(&pick(Lit::neg(0))), "n={n} seed {seed}");
                }
            }
        }
        assert!(decisions <= most, "n={n} m={m}: {decisions} decisions, at most {most}");
    }
}

/// The Tseitin clauses of `gates` random AND gates over `inputs` inputs, each gate over two earlier signals of random
/// polarity, numbered after the inputs; and each gate's two input literals.
fn and_circuit(next: &mut impl FnMut() -> u64, inputs: usize, gates: usize) -> (Vec<Vec<Lit>>, Vec<(Lit, Lit)>) {
    let (mut clauses, mut wiring) = (Vec::new(), Vec::new());
    for g in inputs..inputs + gates {
        let a = Lit::new((next() % g as u64) as Var, next() & 1 == 0);
        let b = Lit::new((next() % g as u64) as Var, next() & 1 == 0);
        let o = Lit::pos(g as Var);
        clauses.push(vec![o.negated(), a]);
        clauses.push(vec![o.negated(), b]);
        clauses.push(vec![o, a.negated(), b.negated()]);
        wiring.push((a, b));
    }
    (clauses, wiring)
}

/// Every signal's value when the circuit's inputs read `bits`.
fn simulate(inputs: usize, wiring: &[(Lit, Lit)], bits: u64) -> Vec<bool> {
    let mut value: Vec<bool> = (0..inputs).map(|v| bits >> v & 1 == 1).collect();
    for &(a, b) in wiring {
        let read = |l: Lit, value: &[bool]| value[l.var() as usize] == l.is_positive();
        let out = read(a, &value) && read(b, &value);
        value.push(out);
    }
    value
}

#[test]
fn a_circuit_counts_its_inputs_once_its_gates_are_eliminated() {
    // An AND circuit with no constraint has a model for each assignment of its inputs, the gates following them. Its
    // gates eliminated from the outputs in, nothing is left to search but the free inputs; branching on the gates
    // instead takes tens of thousands of decisions on these.
    for seed in 0..3u64 {
        let mut next = rng(seed);
        let (inputs, gates) = (24usize, 300usize);
        let (clauses, _) = and_circuit(&mut next, inputs, gates);
        let counted = count_models(inputs + gates, &clauses);
        assert_eq!(counted.models, two_to(inputs as u64), "seed {seed}");
        assert_eq!((counted.settled, counted.decisions), (gates as u64, 0), "seed {seed}");
    }
}

#[test]
fn a_constrained_circuit_counts_as_its_simulation_does() {
    // With some outputs fixed and random clauses over its signals, a circuit's count is the number of input assignments
    // whose simulation satisfies every clause: each assignment of the inputs extends to the gates in one way.
    let mut next = rng(0xC1C7);
    let mut settled = 0;
    for round in 0..200u64 {
        let inputs = 4 + (next() % 9) as usize;
        let gates = 20 + (next() % 200) as usize;
        let n = inputs + gates;
        let (mut clauses, wiring) = and_circuit(&mut next, inputs, gates);
        for k in 0..(next() % 4) as usize {
            clauses.push(vec![Lit::new((n - 1 - k) as Var, next() & 1 == 0)]);
        }
        for _ in 0..next() % 3 {
            let width = 2 + (next() % 2) as usize;
            clauses.push((0..width).map(|_| Lit::new((next() % n as u64) as Var, next() & 1 == 0)).collect());
        }
        let want = (0..1u64 << inputs)
            .filter(|&bits| {
                let value = simulate(inputs, &wiring, bits);
                clauses.iter().all(|c| c.iter().any(|l| value[l.var() as usize] == l.is_positive()))
            })
            .count() as u64;
        let counted = count_models(n, &clauses);
        assert_eq!(counted.models, big(want), "round {round}: {inputs} inputs, {gates} gates");
        settled += counted.settled;
    }
    assert!(settled >= 10_000, "the rounds settle gates ({settled})");
}
