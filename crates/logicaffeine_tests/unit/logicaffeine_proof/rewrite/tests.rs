use super::*;
use crate::aig::Init;

/// What the tests ask of a library beyond what a pass does.
trait Ask {
    /// The gates of the least circuit of `f`, if it has at most `limit`.
    fn within(&mut self, f: Truth, limit: usize) -> Option<usize>;
    /// `f`'s least circuit built into `g` over `leaves` (up to four; input `i` the leaf `i`).
    fn build<G: Gates>(&mut self, f: Truth, leaves: &[AigLit], g: &mut G) -> AigLit;
}

impl Ask for Library {
    fn within(&mut self, f: Truth, limit: usize) -> Option<usize> {
        let (class, _) = self.class(f);
        class.circuits.first().map(|b| b.gates.len()).filter(|&n| n <= limit)
    }

    fn build<G: Gates>(&mut self, f: Truth, leaves: &[AigLit], g: &mut G) -> AigLit {
        let (class, t) = self.class(f);
        implement(class.circuits.first().expect("a class has a circuit"), t, leaves, g)
    }
}
use crate::cdcl::{BudgetedResult, Solver, Var};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// A circuit of exactly `r` gates computing `f`, within `budget` conflicts (0: no limit).
fn circuit_of_size(f: Truth, r: usize, budget: u64) -> Search {
    Synthesis::new(f, r).next(budget)
}

/// The circuit of no gate for a constant or an input.
fn trivial(f: Truth) -> Option<Circuit> {
    if f == 0 || f == 0xFFFF {
        return Some(Circuit { gates: Vec::new(), out: None, negated: f == 0xFFFF });
    }
    let i = INPUT.iter().position(|&x| f == x || f == !x)?;
    Some(Circuit { gates: Vec::new(), out: Some(i as u8), negated: f != INPUT[i] })
}

/// What a search for a circuit of one size found.
enum Search {
    Found(Circuit),
    Refuted,
    GaveUp,
}

/// The circuits of exactly `r` gates computing `f`, one after another: each gate one pair of earlier nodes with a
/// complement on each, every gate but the last read by a later one, the last the output, and two gates in a row the
/// later of which does not read the earlier in the order of their pairs (the circuits that differ only in that order
/// are one).
struct Synthesis {
    f: Truth,
    r: usize,
    solver: Solver,
    sel: Vec<Vec<(u8, u8, Var)>>,
    pa: Vec<Var>,
    pb: Vec<Var>,
    po: Var,
}

impl Synthesis {
    fn new(f: Truth, r: usize) -> Synthesis {
        let mut next = 0u32;
        let mut fresh = || {
            next += 1;
            next - 1
        };
        let mut sel: Vec<Vec<(u8, u8, Var)>> = Vec::new();
        let (mut pa, mut pb, mut val) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..r {
            let nodes = 4 + i as u8;
            let mut pairs = Vec::new();
            for j in 0..nodes {
                for k in j + 1..nodes {
                    pairs.push((j, k, fresh()));
                }
            }
            sel.push(pairs);
            pa.push(fresh());
            pb.push(fresh());
            val.push((0..16).map(|_| fresh()).collect::<Vec<Var>>());
        }
        let po = fresh();
        let mut clauses: Vec<Vec<Lit>> = Vec::new();
        // A node's value at row `t`: a constant for an input, a variable for a gate.
        let value = |n: u8, t: usize| -> Result<bool, Var> {
            if n < 4 {
                Ok(INPUT[n as usize] >> t & 1 == 1)
            } else {
                Err(val[n as usize - 4][t])
            }
        };
        for i in 0..r {
            clauses.push(sel[i].iter().map(|&(_, _, s)| Lit::pos(s)).collect());
            for x in 0..sel[i].len() {
                for y in x + 1..sel[i].len() {
                    clauses.push(vec![Lit::neg(sel[i][x].2), Lit::neg(sel[i][y].2)]);
                }
            }
            for &(j, k, s) in &sel[i] {
                for p in [false, true] {
                    for q in [false, true] {
                        let cond = [Lit::neg(s), Lit::new(pa[i], !p), Lit::new(pb[i], !q)];
                        for t in 0..16 {
                            let v = Lit::pos(val[i][t]);
                            let side = |n: u8, flip: bool| match value(n, t) {
                                Ok(c) => Ok(c ^ flip),
                                Err(var) => Err(Lit::new(var, !flip)),
                            };
                            let (a, b) = (side(j, p), side(k, q));
                            let mut clause = |rest: &[Option<Lit>], satisfied: bool| {
                                if !satisfied {
                                    clauses.push(cond.iter().copied().chain(rest.iter().flatten().copied()).collect());
                                }
                            };
                            for operand in [a, b] {
                                match operand {
                                    Ok(true) => {}
                                    Ok(false) => clause(&[Some(v.negated())], false),
                                    Err(l) => clause(&[Some(v.negated()), Some(l)], false),
                                }
                            }
                            let negated = |o: Result<bool, Lit>| match o {
                                Ok(c) => (None, !c),
                                Err(l) => (Some(l.negated()), false),
                            };
                            let ((na, sa), (nb, sb)) = (negated(a), negated(b));
                            clause(&[Some(v), na, nb], sa || sb);
                        }
                    }
                }
            }
            if i > 0 {
                for &(j, k, s) in &sel[i - 1] {
                    for &(j2, k2, s2) in &sel[i] {
                        let reads = j2 == 4 + i as u8 - 1 || k2 == 4 + i as u8 - 1;
                        if !reads && (j2, k2) < (j, k) {
                            clauses.push(vec![Lit::neg(s), Lit::neg(s2)]);
                        }
                    }
                }
            }
            if i + 1 < r {
                let readers: Vec<Lit> = (i + 1..r)
                    .flat_map(|later| sel[later].iter().filter(|&&(j, k, _)| j == 4 + i as u8 || k == 4 + i as u8))
                    .map(|&(_, _, s)| Lit::pos(s))
                    .collect();
                clauses.push(readers);
            }
        }
        for t in 0..16 {
            let v = val[r - 1][t];
            if f >> t & 1 == 1 {
                clauses.push(vec![Lit::pos(v), Lit::pos(po)]);
                clauses.push(vec![Lit::neg(v), Lit::neg(po)]);
            } else {
                clauses.push(vec![Lit::neg(v), Lit::pos(po)]);
                clauses.push(vec![Lit::pos(v), Lit::neg(po)]);
            }
        }
        let mut solver = Solver::new(next as usize);
        for c in clauses {
            solver.add_clause(c);
        }
        Synthesis { f, r, solver, sel, pa, pb, po }
    }

    /// The next circuit, one not found before, within `budget` conflicts (0: no limit); `Refuted` once there is none.
    fn next(&mut self, budget: u64) -> Search {
        let m = match self.solver.solve_under_assumptions_budgeted(&[], budget) {
            BudgetedResult::Sat(m) => m,
            BudgetedResult::Unsat => return Search::Refuted,
            BudgetedResult::Budget => return Search::GaveUp,
        };
        let mut block = Vec::with_capacity(3 * self.r + 1);
        let gates = (0..self.r)
            .map(|i| {
                let &(j, k, s) = self.sel[i].iter().find(|&&(_, _, s)| m[s as usize]).expect("a selected pair");
                let (a, b) = (self.pa[i], self.pb[i]);
                block.extend([Lit::neg(s), Lit::new(a, !m[a as usize]), Lit::new(b, !m[b as usize])]);
                [(j, m[self.pa[i] as usize]), (k, m[self.pb[i] as usize])]
            })
            .collect();
        block.push(Lit::new(self.po, !m[self.po as usize]));
        self.solver.add_clause(block);
        let c = Circuit { gates, out: Some(4 + self.r as u8 - 1), negated: m[self.po as usize] };
        debug_assert_eq!(c.truth(), self.f, "the circuit found computes the function");
        Search::Found(c)
    }
}

/// The conflicts the generator's search for one size may spend before it gives up.
const GENERATE_BUDGET: u64 = 200_000;

/// The most gates the generator's search goes to.
const MOST_GATES: usize = 12;

/// The circuits the generator keeps of a class's least size found.
const LEAST: usize = 16;

/// The circuits the generator keeps of one gate more.
const NEXT: usize = 8;

/// Class `c` as the generator finds it: each size from one up refuted, given up on, or satisfied; at the first size
/// satisfied every circuit of it up to [`LEAST`], then up to [`NEXT`] of one gate more.
fn synthesize_class(c: Truth) -> Class {
    if let Some(t) = trivial(c) {
        return Class { circuits: vec![t], gave_up: None };
    }
    let mut gave_up = None;
    for r in 1..=MOST_GATES {
        let mut search = Synthesis::new(c, r);
        match search.next(GENERATE_BUDGET) {
            Search::Refuted => {}
            Search::GaveUp => {
                gave_up.get_or_insert(r);
            }
            Search::Found(first) => {
                let mut circuits = vec![first];
                while circuits.len() < LEAST {
                    let Search::Found(another) = search.next(GENERATE_BUDGET) else { break };
                    circuits.push(another);
                }
                let mut more = Synthesis::new(c, r + 1);
                for _ in 0..NEXT {
                    let Search::Found(another) = more.next(GENERATE_BUDGET) else { break };
                    circuits.push(another);
                }
                return Class { circuits, gave_up };
            }
        }
    }
    Class { circuits: Vec::new(), gave_up }
}

/// A circuit in the library's text, the text [`Circuit::parse`] reads.
fn text(k: &Circuit) -> String {
    let node = |n: u8, complemented: bool| format!("{}{}", (b'a' + n) as char, if complemented { "'" } else { "" });
    let mut s: String = k.gates.iter().flat_map(|g| g.iter().map(|&(n, c)| node(n, c))).collect();
    s.push('=');
    match k.out {
        None => s.push(if k.negated { '1' } else { '0' }),
        Some(o) => s.push_str(&node(o, k.negated)),
    }
    s
}

/// The circuit of `g`'s cone of `out` over its four inputs, each gate after its fan-ins and `out` the last.
fn circuit_of_cone(g: &Aig, out: AigLit) -> Circuit {
    let mut index: HashMap<u32, u8> = g.inputs().iter().enumerate().map(|(i, p)| (p.lit.node(), i as u8)).collect();
    let mut gates = Vec::new();
    let mut stack = vec![(out.node(), false)];
    while let Some((n, expanded)) = stack.pop() {
        if index.contains_key(&n) {
            continue;
        }
        let crate::aig::Node::And(a, b) = g.node(n) else { unreachable!("a cone of And nodes over the inputs") };
        if expanded {
            gates.push([(index[&a.node()], a.is_complemented()), (index[&b.node()], b.is_complemented())]);
            index.insert(n, 4 + gates.len() as u8 - 1);
        } else {
            stack.extend([(n, true), (b.node(), false), (a.node(), false)]);
        }
    }
    Circuit { out: Some(index[&out.node()]), negated: out.is_complemented(), gates }
}

/// A circuit for `c` by Shannon expansion on the input it costs least to expand on, `x ? c₁ : c₀`: each cofactor (a
/// function of the other three inputs) built from its class's least circuit in `classes`, both in one graph so that
/// what they share is built once.
fn by_expansion(c: Truth, classes: &HashMap<Truth, Class>) -> Circuit {
    let mut library = Library::new();
    let mut best: Option<Circuit> = None;
    for x in 0..4 {
        let cofactor = |v: bool| -> Truth {
            (0..16).fold(0, |t, row: u16| t | (c >> if v { row | 1 << x } else { row & !(1 << x) } & 1) << row)
        };
        let mut g = Aig::new();
        let inputs: Vec<AigLit> = (0..4).map(|_| g.add_input(None)).collect();
        let mut part = |f: Truth, g: &mut Aig| {
            let (canon, t) = library.canonical(f);
            implement(classes[&canon].circuits.first().expect("a cofactor's class has a circuit"), t, &inputs, g)
        };
        let (hi, lo) = (part(cofactor(true), &mut g), part(cofactor(false), &mut g));
        let out = g.mux(inputs[x], hi, lo);
        let circuit = circuit_of_cone(&g, out);
        assert_eq!(circuit.truth(), c, "{c:04x} expanded on input {x}");
        if best.as_ref().is_none_or(|b| circuit.gates.len() < b.gates.len()) {
            best = Some(circuit);
        }
    }
    best.expect("an input to expand on")
}

/// Every NPN class of four inputs, by canonical truth table, ascending.
fn every_class() -> Vec<Truth> {
    let mut library = Library::new();
    let mut classes: Vec<Truth> = (0..=u16::MAX).map(|f| library.canonical(f).0).collect();
    classes.sort_unstable();
    classes.dedup();
    classes
}

/// The library exact synthesis finds: every class synthesized, on threads, and a class the search found no circuit
/// for given the one [`by_expansion`] builds.
fn generate_library() -> Vec<(Truth, Class)> {
    let classes = every_class();
    let slots: Vec<Mutex<Option<Class>>> = classes.iter().map(|_| Mutex::new(None)).collect();
    let taken = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(8);
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| loop {
                let i = taken.fetch_add(1, Ordering::Relaxed);
                let Some(&c) = classes.get(i) else { break };
                *slots[i].lock().expect("a slot") = Some(synthesize_class(c));
            });
        }
    });
    let mut classes: HashMap<Truth, Class> = classes
        .iter()
        .zip(slots)
        .map(|(&c, slot)| (c, slot.into_inner().expect("a slot").expect("every class synthesized")))
        .collect();
    let bare: Vec<Truth> = classes.iter().filter(|(_, k)| k.circuits.is_empty()).map(|(&c, _)| c).collect();
    for c in bare {
        let circuit = by_expansion(c, &classes);
        classes.get_mut(&c).expect("a class").circuits.push(circuit);
    }
    let mut classes: Vec<(Truth, Class)> = classes.into_iter().collect();
    classes.sort_unstable_by_key(|&(c, _)| c);
    classes
}

/// A library in the text [`parse_library`] reads.
fn library_text(classes: &[(Truth, Class)]) -> String {
    let mut out = String::from(
        "# The circuits of every NPN class of four inputs, for logicaffeine_proof::rewrite, generated by exact\n\
         # synthesis and not edited by hand: the rewrite test\n\
         # exact_synthesis_finds_no_class_a_smaller_circuit_and_proves_every_least_claimed regenerates it. A line a\n\
         # class: its canonical truth table in hex, the first size the search gave up at (- for none), then its\n\
         # circuits.\n",
    );
    for (c, class) in classes {
        let gave_up = class.gave_up.map_or("-".to_string(), |r| r.to_string());
        let circuits: Vec<String> = class.circuits.iter().map(text).collect();
        out.push_str(&format!("{c:04x} {gave_up} {}\n", circuits.join(" ")));
    }
    out
}

/// The least circuit of `f` of at most `limit` gates, every size searched without a budget, the smaller ones refuted.
fn synthesize(f: Truth, limit: usize) -> Option<Circuit> {
    trivial(f).or_else(|| {
        (1..=limit).find_map(|r| match circuit_of_size(f, r, 0) {
            Search::Found(c) => Some(c),
            _ => None,
        })
    })
}

fn xorshift(seed: u64) -> impl FnMut() -> u64 {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    }
}

#[test]
fn the_least_circuit_of_every_function_of_two_inputs_has_its_known_size() {
    // Constants and inputs none, a conjunction or disjunction under complements one, parity three.
    for f in 0..16u16 {
        let truth = (0..16).fold(0u16, |t, x| t | ((f >> (x & 3) & 1) << x));
        let c = synthesize(truth, 4).expect("a circuit of at most four gates");
        assert_eq!(c.truth(), truth, "{f:04b}");
        let size = match f {
            0b0000 | 0b1111 | 0b1010 | 0b0101 | 0b1100 | 0b0011 => 0,
            0b0110 | 0b1001 => 3,
            _ => 1,
        };
        assert_eq!(c.gates.len(), size, "{f:04b}");
    }
}

#[test]
fn majority_and_parity_of_three_take_four_and_six_gates_and_no_fewer() {
    let (a, b, c) = (INPUT[0], INPUT[1], INPUT[2]);
    for (f, size) in [((a & b) | (b & c) | (a & c), 4), (a ^ b ^ c, 6)] {
        let circuit = synthesize(f, 8).expect("a circuit of at most eight gates");
        assert_eq!(circuit.truth(), f);
        assert_eq!(circuit.gates.len(), size, "{f:016b}");
        assert!(matches!(circuit_of_size(f, size - 1, 0), Search::Refuted), "{f:016b}: {} gates refuted", size - 1);
    }
}

#[test]
fn a_functions_class_is_the_same_under_every_transform() {
    let mut next = xorshift(5);
    let mut library = Library::new();
    let all = transforms();
    assert_eq!(all.len(), 768);
    for _ in 0..40 {
        let f = next() as u16;
        let (c, t) = library.canonical(f);
        assert_eq!(apply(f, t), c);
        for &u in all.iter().step_by(37) {
            assert_eq!(library.canonical(apply(f, u)).0, c, "{f:016b} under {u:?}");
        }
    }
}

#[test]
fn every_function_of_three_inputs_is_built_by_its_class_circuit_over_any_leaves() {
    // Each of the 256, its class's least circuit found within the budget and built over four fresh inputs (the fourth
    // read by no function of three) through the transform to its class.
    let mut library = Library::new();
    for f3 in 0..256u16 {
        let f = (0..16).fold(0u16, |t, x| t | ((f3 >> (x & 7) & 1) << x));
        assert!(library.within(f, 12).is_some(), "{f3:08b}: a circuit of at most twelve gates");
        let mut g = Aig::new();
        let leaves: Vec<AigLit> = (0..4).map(|_| g.add_input(None)).collect();
        let out = library.build(f, &leaves, &mut g);
        for x in 0..16usize {
            let inputs: Vec<bool> = (0..4).map(|i| x >> i & 1 == 1).collect();
            let v = g.eval(&inputs, &[]);
            assert_eq!(Aig::value(&v, out), f >> x & 1 == 1, "{f3:08b} at {x}");
        }
    }
}

/// A random circuit: inputs, latches, And/Or/Xor gates over earlier nodes, outputs, a bad state and now and then a
/// constraint, nothing named.
fn random_circuit(seed: u64, inputs: usize, latches: usize, gates: usize) -> Aig {
    let mut next = xorshift(seed);
    let mut g = Aig::new();
    let mut pool: Vec<AigLit> = (0..inputs).map(|_| g.add_input(None)).collect();
    let ls: Vec<usize> = (0..latches).map(|_| g.add_latch(None, Init::Zero)).collect();
    pool.extend(ls.iter().map(|&k| g.latch_lit(k)));
    for _ in 0..gates {
        let a = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
        let b = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
        let c = match next() % 3 {
            0 => g.and(a, b),
            1 => g.or(a, b),
            _ => g.xor(a, b),
        };
        pool.push(c);
    }
    for &k in &ls {
        g.set_next(k, pool[(next() % pool.len() as u64) as usize]);
    }
    for k in 0..3 {
        g.add_output(pool[pool.len() - 1 - k], None);
    }
    g.add_bad(pool[pool.len() - 4], None);
    if seed % 2 == 0 {
        g.add_constraint(pool[pool.len() - 5], None);
    }
    g
}

/// Every port's value under every assignment of the inputs and latches.
fn behaviour(g: &Aig) -> Vec<Vec<bool>> {
    let (ni, nl) = (g.inputs().len(), g.latches().len());
    (0..1u64 << (ni + nl))
        .map(|bits| {
            let x: Vec<bool> = (0..ni).map(|i| bits >> i & 1 == 1).collect();
            let s: Vec<bool> = (0..nl).map(|j| bits >> (ni + j) & 1 == 1).collect();
            let v = g.eval(&x, &s);
            let ports = g.outputs().iter().chain(g.bad()).chain(g.constraints()).map(|p| p.lit);
            ports.chain(g.latches().iter().map(|l| l.next)).map(|l| Aig::value(&v, l)).collect()
        })
        .collect()
}

#[test]
fn a_rewritten_circuit_behaves_as_its_input_on_every_port_and_its_proof_checks() {
    let (mut smaller, mut rewritten) = (0, 0);
    for seed in 0..30u64 {
        let g = random_circuit(seed, 4 + (seed % 4) as usize, (seed % 3) as usize, 30 + (seed % 40) as usize);
        let report = rewrite(&g, &Options::default()).expect("the rewrite is proven");
        assert!(report.check(&g), "seed {seed}");
        assert_eq!(behaviour(&report.aig), behaviour(&g), "seed {seed}: every port as it was");
        assert!(report.after <= report.before, "seed {seed}: {} → {}", report.before, report.after);
        assert_eq!(report.after, report.aig.num_ands());
        smaller += usize::from(report.after < report.before);
        rewritten += report.rewritten;
    }
    assert!(smaller >= 10 && rewritten >= 30, "{smaller} circuits made smaller, {rewritten} nodes rewritten");
}

#[test]
fn parity_built_the_long_way_is_rewritten_to_its_least_circuit() {
    // x ⊕ y as (x ∨ y) ∧ ¬(x ∧ y) and then again by hand from its minterms: the rewrite finds three gates.
    let mut g = Aig::new();
    let (x, y) = (g.add_input(None), g.add_input(None));
    let a = g.and(x, !y);
    let b = g.and(!x, y);
    let minterms = g.or(a, b);
    let both = g.and(x, y);
    let either = g.or(x, y);
    let other = g.and(either, !both);
    let out = g.and(minterms, other);
    g.add_output(out, None);
    let report = rewrite(&g, &Options::default()).expect("proven");
    assert!(report.after <= 3 && report.before > 3, "{} → {}", report.before, report.after);
    assert_eq!(behaviour(&report.aig), behaviour(&g));
}

#[test]
fn a_proof_that_is_tampered_with_or_for_another_circuit_does_not_check() {
    let g = random_circuit(3, 6, 1, 60);
    let report = rewrite(&g, &Options::default()).expect("proven");
    let mut tampered = report.clone();
    if !tampered.proof.steps.is_empty() {
        tampered.proof.steps.remove(0);
        assert!(!tampered.check(&g), "a proof missing a step does not check");
    }
    let other = random_circuit(4, 6, 1, 60);
    assert!(!report.check(&other), "nor does it prove the result equal to another circuit");
}

#[test]
fn a_node_the_graph_already_holds_pays_for_the_rewrite_that_reads_it() {
    // x ∧ y ∧ z built as x ∧ (y ∧ z) beside an output reading x ∧ y: rewriting the conjunction frees two nodes and its
    // least circuit has two gates, so the rewrite pays only by reading the x ∧ y the graph holds, a node it does not
    // add. Every pair of the three inputs shared, under every polarity of the inputs.
    for shared in 0..3 {
        for phases in 0..8u8 {
            let mut g = Aig::new();
            let inputs: Vec<AigLit> = (0..3).map(|i| g.add_input(None).with_phase(phases >> i & 1 == 1)).collect();
            let (x, y, z) = (inputs[shared], inputs[(shared + 1) % 3], inputs[(shared + 2) % 3]);
            let held = g.and(x, y);
            let inner = g.and(y, z);
            let all = g.and(x, inner);
            g.add_output(held, None);
            g.add_output(all, None);
            let report = rewrite(&g, &Options::default()).expect("proven");
            assert_eq!((report.before, report.after), (3, 2), "shared pair {shared}, phases {phases:03b}");
            assert_eq!(behaviour(&report.aig), behaviour(&g), "shared pair {shared}, phases {phases:03b}");
            assert!(report.check(&g));
        }
    }
}

#[test]
fn a_class_keeps_every_least_circuit_up_to_its_cap_and_a_few_of_one_gate_more() {
    // x ∧ y ∧ z has three least circuits of two gates, one for each pair conjoined first, and x ∧ y ∧ z ∧ w fifteen of
    // three: three balanced, twelve chains. Every circuit computes the class, none comes twice, the fewest gates first.
    let mut library = Library::new();
    let (and3, and4) = (INPUT[0] & INPUT[1] & INPUT[2], INPUT[0] & INPUT[1] & INPUT[2] & INPUT[3]);
    for (f, least, size) in [(and3, 3, 2), (and4, 15, 3)] {
        let (class, _) = library.class(f);
        let (c, _) = library.canonical(f);
        assert!(class.least(), "{f:016b}: the least size proven");
        assert_eq!(class.circuits.iter().filter(|k| k.gates.len() == size).count(), least, "{f:016b}");
        assert!(class.circuits.iter().all(|k| k.gates.len() == size || k.gates.len() == size + 1), "{f:016b}");
        assert!(class.circuits.iter().any(|k| k.gates.len() == size + 1), "{f:016b}: some of one gate more");
        assert!(class.circuits.iter().all(|k| k.truth() == c), "{f:016b}");
        assert!(class.circuits.windows(2).all(|w| w[0].gates.len() <= w[1].gates.len()), "{f:016b}");
        for (i, k) in class.circuits.iter().enumerate() {
            assert!(!class.circuits[..i].contains(k), "{f:016b}: circuit {i} twice");
        }
    }
}

#[test]
fn a_class_is_read_once_a_process_and_every_library_takes_the_same_circuits() {
    let f = (INPUT[0] & !INPUT[1]) | (INPUT[2] ^ INPUT[3]);
    let (first, t) = Library::new().class(f);
    let (second, u) = Library::new().class(f);
    assert!(std::ptr::eq(first, second), "one class read once");
    assert_eq!(t, u);
    let (c, _) = Library::new().canonical(f);
    assert!(std::ptr::eq(first, &shipped()[&c]));
}

#[test]
fn a_pass_saves_at_least_the_gain_it_counted_and_leaves_the_network_whole() {
    // Passes over random circuits until one rewrites nothing, within twenty: each saves no fewer nodes than the gains
    // its rewrites counted, rewrites something exactly when it counts a gain, and leaves a network that holds together
    // and behaves as the circuit did.
    let (mut counted, mut passes) = (0, 0);
    for seed in 0..40u64 {
        let g = random_circuit(seed, 4 + (seed % 4) as usize, (seed % 3) as usize, 30 + (seed % 40) as usize);
        let mut net = Network::new(&g);
        let mut library = Library::new();
        for round in 0.. {
            assert!(round < 20, "seed {seed}: the passes settle");
            let before = net.num_ands();
            let (rewritten, gained) = pass(&mut net, &Options::default(), &mut library);
            net.check().unwrap_or_else(|e| panic!("seed {seed}: {e}"));
            assert!(net.num_ands() + gained <= before, "seed {seed}: {before} → {} counting {gained}", net.num_ands());
            assert_eq!(rewritten == 0, gained == 0, "seed {seed}");
            assert_eq!(behaviour(&net.to_aig(&g)), behaviour(&g), "seed {seed}");
            counted += gained;
            passes += 1;
            if rewritten == 0 {
                break;
            }
        }
    }
    assert!(counted >= 60 && passes >= 70, "{counted} nodes counted over {passes} passes");
}

#[test]
fn the_library_holds_every_class_of_four_inputs_once_in_order_each_with_a_circuit() {
    let listed: Vec<Truth> = LIBRARY
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| Truth::from_str_radix(l.split_whitespace().next().expect("a class"), 16).expect("hex"))
        .collect();
    assert_eq!(listed.len(), 222, "the NPN classes of four inputs");
    assert_eq!(listed, every_class(), "each once, ascending");
    assert_eq!(shipped().len(), 222);
    for c in &listed {
        assert!(!shipped()[c].circuits.is_empty(), "{c:04x}: a circuit");
    }
}

#[test]
fn every_circuit_of_the_library_computes_its_class_reads_every_gate_and_reads_back_as_written() {
    for line in LIBRARY.lines().filter(|l| !l.starts_with('#')) {
        let mut words = line.split_whitespace();
        let c = Truth::from_str_radix(words.next().expect("a class"), 16).expect("hex");
        let gave_up = words.next().expect("the size given up at");
        let class = &shipped()[&c];
        assert_eq!(gave_up, class.gave_up.map_or("-".to_string(), |r| r.to_string()), "{c:04x}");
        let texts: Vec<&str> = words.collect();
        assert_eq!(texts.len(), class.circuits.len(), "{c:04x}");
        let least = class.circuits[0].gates.len();
        if least == 0 {
            assert_eq!(class.circuits.len(), 1, "{c:04x}: a constant or an input, one circuit of no gate");
        }
        for (i, (w, k)) in texts.iter().zip(&class.circuits).enumerate() {
            assert_eq!(k.truth(), c, "{c:04x}: {w}");
            assert_eq!(text(k), *w, "{c:04x}: reads back as written");
            assert!(!class.circuits[..i].contains(k), "{c:04x}: {w} twice");
            assert!(k.gates.len() == least || k.gates.len() == least + 1, "{c:04x}: {w}");
            if let Some(last) = k.gates.len().checked_sub(1) {
                assert_eq!(k.out, Some(4 + last as u8), "{c:04x}: {w}: the last gate the output");
                for g in 0..last {
                    let read = k.gates[g + 1..].iter().any(|p| p.iter().any(|&(n, _)| n as usize == 4 + g));
                    assert!(read, "{c:04x}: {w}: gate {g} read by a later one");
                }
            }
        }
        assert!(class.circuits.windows(2).all(|w| w[0].gates.len() <= w[1].gates.len()), "{c:04x}: fewest gates first");
        assert!(class.circuits.iter().filter(|k| k.gates.len() == least).count() <= LEAST, "{c:04x}");
        assert!(class.circuits.iter().filter(|k| k.gates.len() == least + 1).count() <= NEXT, "{c:04x}");
    }
}

#[test]
fn a_least_size_the_library_claims_proven_is_refuted_one_gate_down() {
    // Every class claimed least at five gates or fewer: no circuit of one gate fewer, the search unbudgeted.
    let mut checked = 0;
    for c in every_class() {
        let class = &shipped()[&c];
        let least = class.circuits[0].gates.len();
        if !class.least() || least == 0 || least > 5 {
            continue;
        }
        if least == 1 {
            assert!(trivial(c).is_none(), "{c:04x}: no circuit of no gate");
        } else {
            let below = circuit_of_size(c, least - 1, 0);
            assert!(matches!(below, Search::Refuted), "{c:04x}: {} gates refuted", least - 1);
        }
        checked += 1;
    }
    assert!(checked >= 40, "{checked} classes");
}

#[test]
fn a_library_whose_circuit_does_not_compute_its_class_is_refused() {
    let line = LIBRARY.lines().find(|l| !l.starts_with('#') && l.contains('\'')).expect("a line with a complement");
    let at = line.find('\'').expect("a complement");
    let tampered = format!("{}{}", &line[..at], &line[at + 1..]);
    assert!(std::panic::catch_unwind(|| parse_library(&tampered)).is_err(), "{tampered}");
    assert_eq!(parse_library(line).len(), 1);
    for bad in ["0001 - ab", "0001 - ab=z", "0001 - af=e", "0001 - a=e", "0001 - ab=e''"] {
        assert!(std::panic::catch_unwind(|| parse_library(bad)).is_err(), "{bad}");
    }
}

#[test]
#[ignore = "exact synthesis of every class of four inputs, minutes on up to eight cores"]
fn exact_synthesis_finds_no_class_a_smaller_circuit_and_proves_every_least_claimed() {
    // The library regenerated: no class's least circuit smaller than the shipped one's, and each class the library
    // claims least refuted one gate down without a budget. On a failure the regenerated library is written out to be
    // reviewed and copied over src/rewrite_npn4.txt.
    let generated = generate_library();
    let mut wrong: Vec<String> = Vec::new();
    for (c, class) in &generated {
        let found = class.circuits[0].gates.len();
        let ships = shipped().get(c).map_or(usize::MAX, |k| k.circuits[0].gates.len());
        if found < ships {
            wrong.push(format!("{c:04x}: synthesis finds {found} gates, the library ships {ships}"));
        }
    }
    let claimed: Vec<(Truth, usize)> = every_class()
        .into_iter()
        .filter_map(|c| shipped().get(&c).filter(|k| k.least()).map(|k| (c, k.circuits[0].gates.len())))
        .filter(|&(_, least)| least >= 2)
        .collect();
    let taken = AtomicUsize::new(0);
    let refused = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..std::thread::available_parallelism().map_or(4, |n| n.get()).min(8) {
            scope.spawn(|| loop {
                let i = taken.fetch_add(1, Ordering::Relaxed);
                let Some(&(c, least)) = claimed.get(i) else { break };
                if !matches!(circuit_of_size(c, least - 1, 0), Search::Refuted) {
                    refused.lock().expect("refusals").push(format!("{c:04x}: {} gates not refuted", least - 1));
                }
            });
        }
    });
    wrong.extend(refused.into_inner().expect("refusals"));
    if !wrong.is_empty() {
        let path = std::env::temp_dir().join("rewrite_npn4.txt");
        std::fs::write(&path, library_text(&generated)).expect("the regenerated library written");
        panic!(
            "{}\nthe regenerated library is at {}: review it and copy it over src/rewrite_npn4.txt",
            wrong.join("\n"),
            path.display()
        );
    }
}

#[test]
fn a_circuit_that_would_read_the_node_it_replaces_is_refused() {
    // root = (a ∧ b) ∧ c over the cut {a, b, c}: the circuit conjoining a ∧ b first holds both its gates, the second
    // the root itself, so it is refused; the one conjoining b ∧ c first costs its two gates.
    let mut g = Aig::new();
    let (a, b, c) = (g.add_input(None), g.add_input(None), g.add_input(None));
    let ab = g.and(a, b);
    let root = g.and(ab, c);
    g.add_output(root, None);
    let mut net = Network::new(&g);
    let mut cone = net.freed(root.node(), &[a.node(), b.node(), c.node()]);
    cone.sort_unstable();
    assert_eq!(cone, vec![ab.node(), root.node()]);
    let identity = Transform { perm: [0, 1, 2, 3], neg: 0, out: false };
    let circuit = |gates: Vec<[(u8, bool); 2]>| Circuit { gates, out: Some(5), negated: false };
    let same = circuit(vec![[(0, false), (1, false)], [(4, false), (2, false)]]);
    assert_eq!(cost(&net, root.node(), &same, identity, &[a, b, c], 10), None);
    let other = circuit(vec![[(1, false), (2, false)], [(0, false), (4, false)]]);
    assert_eq!(cost(&net, root.node(), &other, identity, &[a, b, c], 10), Some(2));
    assert_eq!(cost(&net, root.node(), &other, identity, &[a, b, c], 1), None, "past its bound");
}

#[test]
fn a_cut_one_of_whose_leaves_is_freed_has_no_function_even_where_the_cone_ends_at_the_others() {
    // r = x ∧ d with x = (a ∧ b) ∧ c; x replaced by y = a ∧ (b ∧ c) frees a ∧ b. Over {a ∧ b, c, d, y} the cone of r
    // now ends at d and y, but a ∧ b is gone, so the cut is refused; over {c, d, y} it reads y ∧ d.
    let mut g = Aig::new();
    let (a, b, c, d) = (g.add_input(None), g.add_input(None), g.add_input(None), g.add_input(None));
    let ab = g.and(a, b);
    let x = g.and(ab, c);
    let bc = g.and(b, c);
    let y = g.and(a, bc);
    let r = g.and(x, d);
    g.add_output(r, None);
    g.add_output(y, None);
    let mut net = Network::new(&g);
    assert!(truth(&net, r.node(), &[ab.node(), c.node(), d.node()]).is_some(), "a cut before the replacement");
    net.replace(x.node(), y);
    assert_eq!(net.kind(ab.node()), Kind::Freed);
    assert_eq!(truth(&net, r.node(), &[ab.node(), c.node(), d.node(), y.node()]), None);
    assert_eq!(truth(&net, r.node(), &[c.node(), d.node(), y.node()]), Some(INPUT[1] & INPUT[2]));
}

#[test]
fn a_circuit_text_whose_gate_reads_a_node_not_before_it_is_not_a_circuit() {
    let nand = Circuit { gates: vec![[(0, false), (1, true)]], out: Some(4), negated: true };
    assert_eq!(Circuit::parse("ab'=e'"), Some(nand));
    assert_eq!(Circuit::parse("=1"), Some(Circuit { gates: Vec::new(), out: None, negated: true }));
    for bad in ["af=e", "ae=e", "abef=f", "ab=f", "ab", "ab=e''", "aB=e", "a=e", "ab=e'x", "=a''"] {
        assert_eq!(Circuit::parse(bad), None, "{bad}");
    }
}
