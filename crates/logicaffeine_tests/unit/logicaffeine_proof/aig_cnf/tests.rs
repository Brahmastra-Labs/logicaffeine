use super::*;
use crate::aig::Init;
use crate::cdcl::BudgetedResult;

/// A deterministic xorshift stream.
fn stream(seed: u64) -> impl FnMut() -> u64 {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    }
}

/// Function `f` of `n` inputs repeated over the inputs it ignores, as every function here is kept.
fn widened(f: u64, n: usize) -> Truth {
    let width = 1usize << n;
    let mask = if width == 64 { !0 } else { (1u64 << width) - 1 };
    let mut out = 0;
    for rep in (0..64).step_by(width) {
        out |= (f & mask) << rep;
    }
    out
}

/// Bit `x` of `f`.
fn at(f: Truth, x: usize) -> bool {
    f >> x & 1 == 1
}

/// Whether cube `c` holds at input `x`.
fn holds(c: &Cube, x: usize) -> bool {
    (0..MAX_LEAVES).all(|i| (c.pos >> i & 1 == 0 || x >> i & 1 == 1) && (c.neg >> i & 1 == 0 || x >> i & 1 == 0))
}

/// The function a sum of products is, over the whole word.
fn sum(cubes: &[Cube]) -> Truth {
    (0..64).filter(|&x| cubes.iter().any(|c| holds(c, x))).fold(0, |f, x| f | 1 << x)
}

/// That `cubes` is an exact, prime and irredundant cover of `f` over `n` inputs.
fn assert_isop(f: Truth, n: usize, cubes: &[Cube]) {
    assert_eq!(sum(cubes), f, "the cover of {f:#x} over {n} inputs is the function");
    for (k, c) in cubes.iter().enumerate() {
        assert_eq!((c.pos | c.neg) >> n, 0, "a cube of {f:#x} reads only the first {n} inputs");
        assert_eq!(c.pos & c.neg, 0, "a cube of {f:#x} holds no input both ways");
        for i in 0..n {
            for (pos, neg) in [(c.pos & !(1 << i), c.neg), (c.pos, c.neg & !(1 << i))] {
                if (pos, neg) == (c.pos, c.neg) {
                    continue;
                }
                let wider = Cube { pos, neg };
                let leaves = (0..64).any(|x| holds(&wider, x) && !at(f, x));
                assert!(leaves, "cube {k} of {f:#x} is prime: dropping input {i} leaves f");
            }
        }
        let rest: Vec<Cube> = cubes.iter().enumerate().filter(|&(j, _)| j != k).map(|(_, c)| *c).collect();
        assert_ne!(sum(&rest), f, "cube {k} of {f:#x} is needed");
    }
}

/// Every one of the 65,536 functions of four inputs: its sum of products is exact, every cube prime, none redundant,
/// and the cover of its complement is too.
#[test]
fn the_sum_of_products_of_every_four_input_function_is_exact_prime_and_irredundant() {
    for f in 0..=0xFFFFu64 {
        let f = widened(f, 4);
        assert_isop(f, 4, &isop(f, 4));
        assert_isop(!f, 4, &isop(!f, 4));
    }
}

/// Random functions of five and six inputs, and the constants and single inputs of every width.
#[test]
fn the_sum_of_products_of_wide_functions_is_exact_prime_and_irredundant() {
    let mut next = stream(7);
    for _ in 0..4000 {
        let f = next();
        assert_isop(f, 6, &isop(f, 6));
        let g = widened(next(), 5);
        assert_isop(g, 5, &isop(g, 5));
    }
    for n in 0..=MAX_LEAVES {
        assert!(isop(0, n).is_empty(), "false has no cube");
        assert_eq!(isop(!0, n), vec![Cube { pos: 0, neg: 0 }], "true is the empty cube");
        for i in 0..n {
            assert_eq!(isop(INPUT[i], n), vec![Cube { pos: 1 << i, neg: 0 }], "input {i} of {n}");
            assert_eq!(isop(!INPUT[i], n), vec![Cube { pos: 0, neg: 1 << i }], "not input {i} of {n}");
        }
    }
}

/// The truth-table operations against their definitions, bit by bit, on random functions.
#[test]
fn cofactors_flips_swaps_and_stretches_are_their_definitions_bit_by_bit() {
    let mut next = stream(11);
    for _ in 0..2000 {
        let f = next();
        for i in 0..MAX_LEAVES {
            for x in 0..64usize {
                let with = |v: bool| if v { x | 1 << i } else { x & !(1 << i) };
                assert_eq!(at(cofactor(f, i, false), x), at(f, with(false)), "cofactor of input {i} at 0, bit {x}");
                assert_eq!(at(cofactor(f, i, true), x), at(f, with(true)), "cofactor of input {i} at 1, bit {x}");
                assert_eq!(at(flip(f, i), x), at(f, x ^ 1 << i), "flip of input {i}, bit {x}");
            }
            let reads = (0..64usize).any(|x| at(f, x) != at(f, x ^ 1 << i));
            assert_eq!(depends(f, i), reads, "whether {f:#x} reads input {i}");
        }
        for i in 0..MAX_LEAVES - 1 {
            for x in 0..64usize {
                let (a, b) = (x >> i & 1, x >> (i + 1) & 1);
                let swapped = (x & !(3 << i)) | b << i | a << (i + 1);
                assert_eq!(at(swap_adjacent(f, i), x), at(f, swapped), "swap of inputs {i} and {}, bit {x}", i + 1);
            }
        }
        let n = 1 + (next() % 3) as usize;
        let mut to: Vec<u32> = Vec::new();
        while to.len() < n + 2 {
            let leaf = (next() % 40) as u32;
            if !to.contains(&leaf) {
                to.push(leaf);
            }
        }
        to.sort_unstable();
        let mut from: Vec<u32> = to.iter().copied().filter(|_| next() % 2 == 0).take(n).collect();
        if from.is_empty() {
            from.push(to[0]);
        }
        let g = widened(f, from.len());
        let s = stretch(g, &from, &to);
        for x in 0..(1usize << to.len()) {
            let y = from.iter().enumerate().fold(0, |y, (i, leaf)| {
                let p = to.iter().position(|t| t == leaf).unwrap();
                y | (x >> p & 1) << i
            });
            assert_eq!(at(s, x), at(g, y), "stretch of {from:?} to {to:?}, bit {x}");
        }
    }
}

/// A deterministic random combinational graph: `ni` inputs and `gates` And nodes over random earlier literals, some
/// fan-ins reconverging; its literals that read anything.
fn random_graph(seed: u64, ni: usize, gates: usize) -> (Aig, Vec<AigLit>) {
    let mut next = stream(seed);
    let mut g = Aig::new();
    let mut pool: Vec<AigLit> = (0..ni).map(|_| g.add_input(None)).collect();
    for _ in 0..gates {
        let pick = |next: &mut dyn FnMut() -> u64, pool: &[AigLit]| {
            let near = pool.len().saturating_sub(8);
            let from = if next() % 3 == 0 { 0 } else { near };
            pool[from + (next() % (pool.len() - from) as u64) as usize].with_phase(next() & 1 == 1)
        };
        let (a, b) = (pick(&mut next, &pool), pick(&mut next, &pool));
        let c = g.and(a, b);
        if !pool.contains(&c) && !pool.contains(&!c) && c.node() != 0 {
            pool.push(c);
        }
    }
    (g, pool)
}

/// On hundreds of random graphs, eight roots each, and every width: each cut the cover keeps is its root's function over
/// its leaves at every one of 256 random assignments, and the cover holds every And node its cuts read and every And
/// root.
#[test]
fn every_cut_of_the_cover_is_its_roots_function_over_its_leaves() {
    let mut cuts = 0;
    for seed in 0..400u64 {
        let ni = 3 + (seed % 6) as usize;
        let (g, pool) = random_graph(seed, ni, 30 + (seed % 60) as usize);
        let mut next = stream(seed ^ 0xC0FFEE);
        let roots: Vec<AigLit> = (0..8).map(|_| pool[(next() % pool.len() as u64) as usize]).collect();
        for k in 2..=MAX_LEAVES {
            let mapping = map(&g, &roots, k);
            let words = 4;
            let inputs: Vec<u64> = (0..ni * words).map(|_| next()).collect();
            let sim = g.simulate(words, &inputs, &[]);
            for &r in &roots {
                if matches!(g.node(r.node()), Node::And(..)) {
                    assert!(mapping.cut(r.node()).is_some(), "seed {seed} k {k}: the root {r:?} is covered");
                }
            }
            for n in 0..g.num_nodes() as u32 {
                let Some(cut) = mapping.cut(n) else { continue };
                assert!(cut.leaves().len() <= k, "seed {seed} k {k}: node {n}'s cut has at most {k} leaves");
                assert!(cut.leaves().windows(2).all(|w| w[0] < w[1]), "seed {seed} k {k}: leaves ascend");
                for &leaf in cut.leaves() {
                    if matches!(g.node(leaf), Node::And(..)) {
                        assert!(mapping.cut(leaf).is_some(), "seed {seed} k {k}: node {n}'s leaf {leaf} is covered");
                    }
                }
                for w in 0..words {
                    let root = Aig::word(&sim, words, AigLit::new(n, false), w);
                    let leaves: Vec<u64> =
                        cut.leaves().iter().map(|&l| Aig::word(&sim, words, AigLit::new(l, false), w)).collect();
                    for bit in 0..64 {
                        let x = leaves.iter().enumerate().fold(0usize, |x, (i, v)| x | ((v >> bit & 1) as usize) << i);
                        assert_eq!(at(cut.truth, x), root >> bit & 1 == 1, "seed {seed} k {k}: node {n}'s cut");
                    }
                }
                cuts += 1;
            }
        }
    }
    assert!(cuts >= 20_000, "{cuts} cuts checked");
}

/// The cover uses its width: on nearly every random graph the cover by cuts of six leaves keeps fewer roots than the
/// one by cuts of two, which is an And node each.
#[test]
fn a_wider_cut_covers_with_fewer_roots() {
    let mut fewer = 0;
    for seed in 0..100u64 {
        let (g, pool) = random_graph(seed, 6, 80);
        let chosen = &pool[pool.len() - 4..];
        fewer += (kept(&map(&g, chosen, MAX_LEAVES), &g) < kept(&map(&g, chosen, 2), &g)) as usize;
    }
    assert!(fewer >= 90, "fewer roots on {fewer} of 100 graphs");
}

/// The nodes a cover keeps.
fn kept(mapping: &Mapping, g: &Aig) -> usize {
    (0..g.num_nodes() as u32).filter(|&n| mapping.cut(n).is_some()).count()
}

/// An instance's signal over solver variables, read under an assignment of them.
fn read(s: Signal, model: &[bool]) -> bool {
    match s.as_lit() {
        Some(l) => model[l.var() as usize] == l.is_positive(),
        None => s == Signal::TRUE,
    }
}

/// On thousands of random functions over random signals (constants, and literals of four variables, repeated and
/// complemented): under every assignment of the four variables the clauses made hold and force the instance's
/// signal to the function's value at the leaves' values.
#[test]
fn an_instance_is_its_function_over_its_signals_under_every_assignment() {
    let mut next = stream(23);
    let mut made = 0;
    for _ in 0..3000 {
        let n = 1 + (next() % MAX_LEAVES as u64) as usize;
        let f = widened(next(), n);
        let mut solver = Solver::new(4);
        let base: Vec<Lit> = (0..4).map(Lit::pos).collect();
        let leaves: Vec<Signal> = (0..n)
            .map(|_| match next() % 8 {
                0 => Signal::FALSE,
                1 => Signal::TRUE,
                _ => Signal::lit(base[(next() % 4) as usize]).with_phase(next() & 1 == 1),
            })
            .collect();
        let mut instances = Instances::default();
        let s = instances.instance(f, &leaves, &mut solver);
        made += s.as_lit().is_some_and(|l| l.var() >= 4) as usize;
        for x in 0..16u32 {
            let assumptions: Vec<Lit> = (0..4).map(|v| Lit::new(v, x >> v & 1 == 1)).collect();
            let values: Vec<bool> = (0..4).map(|v| x >> v & 1 == 1).collect();
            let at_leaves = leaves.iter().enumerate().fold(0usize, |y, (i, &l)| y | (read(l, &values) as usize) << i);
            let want = at(f, at_leaves);
            match solver.solve_under_assumptions_budgeted(&assumptions, 0) {
                BudgetedResult::Sat(model) => assert_eq!(read(s, &model), want, "{f:#x} over {leaves:?} at {x:04b}"),
                other => panic!("{f:#x} over {leaves:?} at {x:04b}: the clauses hold, got {other:?}"),
            }
            if let Some(l) = s.as_lit() {
                let wrong = if want { l.negated() } else { l };
                let mut against = assumptions.clone();
                against.push(wrong);
                assert!(
                    matches!(solver.solve_under_assumptions_budgeted(&against, 0), BudgetedResult::Unsat),
                    "{f:#x} over {leaves:?} at {x:04b}: the signal is forced"
                );
            }
        }
    }
    assert!(made >= 1500, "{made} instances took a variable of their own");
}

/// One function over one set of variables is one variable however its leaves come: in another order, complemented
/// with the function flipped to match, or the whole function complemented — and a different function is another.
#[test]
fn one_function_over_the_same_variables_is_one_variable() {
    let mut next = stream(31);
    let mut checked = 0;
    for _ in 0..2000 {
        let mut solver = Solver::new(6);
        let mut instances = Instances::default();
        let n = 2 + (next() % 5) as usize;
        let f = widened(next(), n);
        let leaves: Vec<Signal> = (0..n as u32).map(|v| Signal::lit(Lit::pos(v))).collect();
        let s = instances.instance(f, &leaves, &mut solver);
        if s.as_lit().is_none() {
            continue;
        }
        let i = (next() % (n as u64 - 1)) as usize;
        let mut swapped = leaves.clone();
        swapped.swap(i, i + 1);
        let by_swap = instances.instance(swap_adjacent(f, i), &swapped, &mut solver);
        assert_eq!(by_swap, s, "{f:#x}: leaves {i} and {} swapped", i + 1);
        let mut negated = leaves.clone();
        negated[i] = negated[i].with_phase(true);
        assert_eq!(instances.instance(flip(f, i), &negated, &mut solver), s, "{f:#x}: leaf {i} complemented");
        assert_eq!(instances.instance(!f, &leaves, &mut solver), s.with_phase(true), "{f:#x} complemented");
        let other = f ^ (1 << (next() % (1 << n)));
        let t = instances.instance(widened(other, n), &leaves, &mut solver);
        assert_ne!(t, s, "{f:#x} and {other:#x} differ");
        assert_ne!(t, s.with_phase(true), "{f:#x} and {other:#x} are not complements");
        checked += 1;
    }
    assert!(checked >= 1500, "{checked} functions");
}

/// A constant leaf, a leaf equal to another and one complementing another leave a function of fewer variables: what
/// is left of the conjunction of x, true and x is x itself; of x and not x, false; of x, y and x, the one gate x ∧ y.
#[test]
fn constant_and_repeated_leaves_fold_away() {
    let mut solver = Solver::new(2);
    let mut instances = Instances::default();
    let (x, y) = (Signal::lit(Lit::pos(0)), Signal::lit(Lit::pos(1)));
    let and3 = INPUT[0] & INPUT[1] & INPUT[2];
    assert_eq!(instances.instance(and3, &[x, Signal::TRUE, x], &mut solver), x);
    assert_eq!(instances.instance(and3, &[x, Signal::TRUE, Signal::FALSE], &mut solver), Signal::FALSE);
    assert_eq!(instances.instance(INPUT[0] & INPUT[1], &[x, x.with_phase(true)], &mut solver), Signal::FALSE);
    assert_eq!(instances.instance(INPUT[0] | INPUT[1], &[x.with_phase(true), x], &mut solver), Signal::TRUE);
    let g = instances.instance(and3, &[x, y, x], &mut solver);
    assert_eq!(instances.instance(INPUT[0] & INPUT[1], &[y, x], &mut solver), g);
    assert_eq!(solver.add_vars(0), 3, "one variable for x ∧ y");
    assert_eq!(instances.instance(INPUT[2], &[y, Signal::FALSE, x], &mut solver), x, "a projection is its leaf");
}

/// A latch, its next state and an input are leaves: the cover of a sequential circuit's next states and bad state
/// stops at them, and a latch's value at the next clock is its next-state root's.
#[test]
fn a_cover_stops_at_latches_and_inputs() {
    let mut g = Aig::new();
    let i = g.add_input(None);
    let k = g.add_latch(None, Init::Zero);
    let x = g.latch_lit(k);
    let a = g.and(x, i);
    let b = g.and(!x, !i);
    let n = g.or(a, b);
    g.set_next(k, n);
    let bad = g.and(x, !i);
    g.add_bad(bad, None);
    let roots = [g.latches()[0].next, g.bad()[0].lit];
    let mapping = map(&g, &roots, 4);
    for node in 0..g.num_nodes() as u32 {
        if let Some(cut) = mapping.cut(node) {
            assert!(cut.leaves().iter().all(|&l| !matches!(g.node(l), Node::And(..)) || mapping.cut(l).is_some()));
        }
    }
    let next = mapping.cut(n.node()).expect("the next state is covered");
    assert_eq!(next.leaves(), &[i.node(), x.node()], "x xnor i over its two leaves");
    assert_eq!(kept(&mapping, &g), 2, "the next state and the bad state");
}
