use super::*;
use crate::aig::Init;

/// What the tests read of a network beyond what the passes do.
trait Probe {
    /// How many nodes and ports read node `n`.
    fn refs(&self, n: u32) -> usize;
    /// The ports' literals: outputs, bad states, constraints, next states.
    fn ports(&self) -> &[AigLit];
}

impl Probe for Network {
    fn refs(&self, n: u32) -> usize {
        self.readers[n as usize].len()
    }

    fn ports(&self) -> &[AigLit] {
        &self.ports
    }
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

/// A random circuit: inputs, latches, And/Or/Xor gates over earlier nodes, several outputs, a bad state, a constraint
/// now and then, the same logic built twice over in places so that equal nodes abound.
fn random_circuit(seed: u64, inputs: usize, latches: usize, gates: usize) -> Aig {
    let mut next = xorshift(seed);
    let mut g = Aig::new();
    let mut pool: Vec<AigLit> = (0..inputs).map(|_| g.add_input(None)).collect();
    let ls: Vec<usize> = (0..latches).map(|_| g.add_latch(None, Init::Zero)).collect();
    pool.extend(ls.iter().map(|&k| g.latch_lit(k)));
    for _ in 0..gates {
        let a = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
        let b = pool[(next() % pool.len() as u64) as usize].with_phase(next() & 1 == 1);
        let c = match next() % 4 {
            0 => g.and(a, b),
            1 => g.or(a, b),
            2 => g.xor(a, b),
            _ => {
                let c = pool[(next() % pool.len() as u64) as usize];
                let left = g.and(a, b);
                let twice = g.and(left, c);
                pool.push(twice);
                let right = g.and(b, c);
                g.and(a, right)
            }
        };
        pool.push(c);
    }
    for &k in &ls {
        g.set_next(k, pool[(next() % pool.len() as u64) as usize]);
    }
    for k in 0..4 {
        g.add_output(pool[pool.len() - 1 - k], None);
    }
    g.add_bad(pool[pool.len() - 5], None);
    if seed % 2 == 0 {
        g.add_constraint(pool[pool.len() - 6], None);
    }
    g
}

/// The And nodes the ports of `g` read.
fn read_ands(g: &Aig) -> usize {
    let ports = g.outputs().iter().chain(g.bad()).chain(g.constraints()).map(|p| p.lit);
    let mut stack: Vec<u32> = ports.chain(g.latches().iter().map(|l| l.next)).map(|l| l.node()).collect();
    let mut seen = std::collections::HashSet::new();
    while let Some(n) = stack.pop() {
        if let crate::aig::Node::And(a, b) = g.node(n) {
            if seen.insert(n) {
                stack.extend([a.node(), b.node()]);
            }
        }
    }
    seen.len()
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

/// The value of each node of `net` under every assignment of `g`'s inputs and latches, a bit an assignment.
fn values(net: &Network, g: &Aig) -> Vec<u64> {
    let leaves: Vec<u32> = g.inputs().iter().map(|p| p.lit.node()).chain(g.latches().iter().map(|l| l.node)).collect();
    assert!(leaves.len() <= 6, "an assignment a bit of one word");
    let mut v = vec![0u64; net.len()];
    for (i, &n) in leaves.iter().enumerate() {
        v[n as usize] = (0..1u64 << leaves.len()).fold(0, |w, x| w | (x >> i & 1) << x);
    }
    let read = |v: &[u64], l: AigLit| if l.is_complemented() { !v[l.node() as usize] } else { v[l.node() as usize] };
    for n in net.topological() {
        let Kind::And(a, b) = net.kind(n) else { unreachable!() };
        v[n as usize] = read(&v, a) & read(&v, b);
    }
    v
}

#[test]
fn a_network_made_from_a_graph_reads_back_as_that_graph() {
    for seed in 0..40u64 {
        let g = random_circuit(seed, 3 + (seed % 4) as usize, (seed % 3) as usize, 20 + (seed % 50) as usize);
        let net = Network::new(&g);
        net.check().unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        assert_eq!(net.num_ands(), read_ands(&g), "seed {seed}: what the ports read, and no more");
        let back = net.to_aig(&g);
        assert_eq!(behaviour(&back), behaviour(&g), "seed {seed}");
        assert_eq!(back.num_ands(), net.num_ands(), "seed {seed}");
        assert_eq!(back.inputs().len(), g.inputs().len());
        assert_eq!(back.latches().iter().map(|l| l.init).collect::<Vec<_>>(), g.latches().iter().map(|l| l.init).collect::<Vec<_>>());
    }
}

#[test]
fn a_replaced_node_hands_every_reader_to_its_equal_and_its_cone_is_freed() {
    // x = (a ∧ b) ∧ c and y = a ∧ (b ∧ c), each an output, z = x ∧ d another: x replaced by y, both outputs and z read y,
    // and x and the a ∧ b only it read are freed.
    let mut g = Aig::new();
    let (a, b, c, d) = (g.add_input(None), g.add_input(None), g.add_input(None), g.add_input(None));
    let ab = g.and(a, b);
    let x = g.and(ab, c);
    let bc = g.and(b, c);
    let y = g.and(a, bc);
    let z = g.and(x, d);
    g.add_output(x, None);
    g.add_output(!y, None);
    g.add_output(!z, None);
    let mut net = Network::new(&g);
    assert_eq!(net.num_ands(), 5);
    net.replace(x.node(), y);
    net.check().unwrap();
    assert_eq!(net.num_ands(), 3, "x and a ∧ b freed");
    assert_eq!(net.kind(x.node()), Kind::Freed);
    assert_eq!(net.kind(ab.node()), Kind::Freed);
    assert_eq!(net.ports()[0], y);
    assert_eq!(net.ports()[1], !y);
    let z2 = net.ports()[2];
    assert!(z2.is_complemented(), "a port keeps its complement");
    assert_eq!(net.kind(z2.node()), Kind::And(y.max(d), y.min(d)), "z reads y now, the same node");
    assert_eq!(z2.node(), z.node(), "a reader whose pair is new is patched in place");
    assert_eq!(net.version(z.node()), 1);
    assert_eq!(behaviour(&net.to_aig(&g)), behaviour(&g));
}

#[test]
fn readers_brought_to_one_pair_are_merged_and_a_reader_that_folds_becomes_its_fold() {
    // u = (c ∧ d) ∧ e and v = c ∧ (d ∧ e) are equal; r = a ∧ u and s = a ∧ v read one pair once u is v, and t = u ∧ ¬v
    // folds to false, and w = t ∨ b, reading it, to b.
    let mut g = Aig::new();
    let (a, b, c, d, e) = (g.add_input(None), g.add_input(None), g.add_input(None), g.add_input(None), g.add_input(None));
    let cd = g.and(c, d);
    let u = g.and(cd, e);
    let de = g.and(d, e);
    let v = g.and(c, de);
    let r = g.and(a, u);
    let s = g.and(a, v);
    let t = g.and(u, !v);
    let w = g.or(t, b);
    for p in [r, s, w] {
        g.add_output(p, None);
    }
    let mut net = Network::new(&g);
    assert_eq!(net.num_ands(), 8);
    net.replace(u.node(), v);
    net.check().unwrap();
    assert_eq!(net.ports()[0], s, "r merged into s");
    assert_eq!(net.ports()[1], s);
    assert_eq!(net.ports()[2], b, "t folded to false, so ¬(¬t ∧ ¬b) is b");
    assert_eq!(net.num_ands(), 3, "d ∧ e, v and s are left");
    for n in [cd, u, r, t, w] {
        assert_eq!(net.kind(n.node()), Kind::Freed);
    }
    assert_eq!(behaviour(&net.to_aig(&g)), behaviour(&g));
}

#[test]
fn a_replacement_merges_a_whole_copy_of_the_logic_above_it_level_by_level() {
    // u = (c ∧ d) ∧ e and v = c ∧ (d ∧ e) are equal, and the same thirty-nine steps are built over each, a step the
    // conjunction of the one before and an input and every other step that and the step three before, each read under
    // a polarity: replacing u by v merges the copy over u into the copy over v gate by gate, and every port of the
    // first copy then reads its twin in the second.
    let mut g = Aig::new();
    let inputs: Vec<AigLit> = (0..6).map(|_| g.add_input(None)).collect();
    let (c, d, e) = (inputs[0], inputs[1], inputs[2]);
    let cd = g.and(c, d);
    let u = g.and(cd, e);
    let de = g.and(d, e);
    let v = g.and(c, de);
    let mut next = xorshift(17);
    let plan: Vec<(bool, bool, usize, bool)> =
        (1..40).map(|_| (next() & 1 == 1, next() & 1 == 1, (next() % 6) as usize, next() & 1 == 1)).collect();
    let copy = |g: &mut Aig, base: AigLit| -> Vec<AigLit> {
        let mut chain = vec![base];
        for (i, &(p, q, x, r)) in plan.iter().enumerate() {
            let back = chain[i.saturating_sub(2)].with_phase(q);
            let step = g.and(chain[i].with_phase(p), inputs[x].with_phase(r));
            let gate = if i % 2 == 0 { g.and(step, back) } else { step };
            chain.push(gate);
        }
        chain
    };
    let (over_u, over_v) = (copy(&mut g, u), copy(&mut g, v));
    for (i, (&a, &b)) in over_u.iter().zip(&over_v).enumerate() {
        if i % 5 == 4 {
            g.add_output(a, None);
            g.add_output(!b, None);
        }
    }
    g.add_output(*over_u.last().unwrap(), None);
    let mut net = Network::new(&g);
    let before = net.num_ands();
    net.replace(u.node(), v);
    net.check().unwrap();
    let ports = net.ports().to_vec();
    for pair in ports[..ports.len() - 1].chunks(2) {
        assert_eq!(pair[0], !pair[1], "a port of the first copy reads its twin");
    }
    assert_eq!(*ports.last().unwrap(), *over_v.last().unwrap());
    assert_eq!(net.num_ands(), before / 2, "the copy over u and u's own cone are freed");
    for l in over_u.iter().chain([&cd]) {
        assert_eq!(net.kind(l.node()), Kind::Freed, "{l:?}");
    }
    assert_eq!(behaviour(&net.to_aig(&g)), behaviour(&g));
}

#[test]
fn a_merge_still_under_way_keeps_the_node_it_will_hand_readers_to() {
    // x = (a ∧ c) ∧ d and y = a ∧ (c ∧ d) are equal. m = x ∧ b merges into e = y ∧ b once x is y, and w = x ∧ ¬y folds
    // to false, so z = ¬e ∧ ¬w folds to ¬e and f = e ∧ z to false: freeing f and then z leaves e read by nothing
    // while m is yet to hand it its readers, and e must stay. Either order of m and w among x's readers.
    for m_first in [true, false] {
        let mut g = Aig::new();
        let (a, b, c, d) = (g.add_input(None), g.add_input(None), g.add_input(None), g.add_input(None));
        let ac = g.and(a, c);
        let x = g.and(ac, d);
        let cd = g.and(c, d);
        let y = g.and(a, cd);
        let e = g.and(y, b);
        let (m, w) = if m_first {
            let m = g.and(x, b);
            (m, g.and(x, !y))
        } else {
            let w = g.and(x, !y);
            (g.and(x, b), w)
        };
        let z = g.and(!e, !w);
        let f = g.and(e, z);
        g.add_output(m, None);
        g.add_output(f, None);
        let mut net = Network::new(&g);
        net.replace(x.node(), y);
        net.check().unwrap_or_else(|err| panic!("m first {m_first}: {err}"));
        assert_eq!(net.ports()[0], e, "m first {m_first}: m merged into e");
        assert_eq!(net.ports()[1], AigLit::FALSE, "m first {m_first}: f folded to false");
        assert_ne!(net.kind(e.node()), Kind::Freed, "m first {m_first}");
        assert_eq!(behaviour(&net.to_aig(&g)), behaviour(&g), "m first {m_first}");
    }
}

#[test]
fn the_cone_a_replacement_frees_is_what_only_it_reads_above_the_leaves() {
    // root = (a ∧ b) ∧ (b ∧ c) with b ∧ c an output too: above {a, b, c} the root frees itself and a ∧ b; above
    // {a ∧ b, b, c} only itself.
    let mut g = Aig::new();
    let (a, b, c) = (g.add_input(None), g.add_input(None), g.add_input(None));
    let ab = g.and(a, b);
    let bc = g.and(b, c);
    let root = g.and(ab, bc);
    g.add_output(root, None);
    g.add_output(bc, None);
    let mut net = Network::new(&g);
    let mut cone = net.freed(root.node(), &[a.node(), b.node(), c.node()]);
    cone.sort_unstable();
    assert_eq!(cone, vec![ab.node(), root.node()]);
    assert!(net.frees(ab.node()) && net.frees(root.node()) && !net.frees(bc.node()) && !net.frees(a.node()));
    assert_eq!(net.freed(root.node(), &[ab.node(), b.node(), c.node()]), vec![root.node()]);
    assert!(!net.frees(ab.node()), "a later call forgets the earlier cone");
    assert_eq!(net.refs(bc.node()), 2, "counting a cone changes no reader");
}

#[test]
fn random_replacements_by_equal_literals_keep_every_port_and_the_network_whole() {
    // Nodes of random circuits replaced, one after another, by a literal equal to each built anew by Shannon expansion
    // over the inputs and latches (a literal whose cone holds the node is passed over): after every replacement the
    // network holds together and every node live before and after computes what it did, and the graph read back
    // behaves as the input.
    let mut replaced = 0;
    for seed in 0..60u64 {
        let g = random_circuit(seed, 3 + (seed % 3) as usize, (seed % 2) as usize, 25 + (seed % 30) as usize);
        let mut net = Network::new(&g);
        let leaves: Vec<AigLit> =
            g.inputs().iter().map(|p| p.lit).chain(g.latches().iter().map(|l| AigLit::new(l.node, false))).collect();
        let mut next = xorshift(seed ^ 0xA5A5);
        for _ in 0..12 {
            let live: Vec<u32> = net.topological();
            if live.is_empty() {
                break;
            }
            let n = live[(next() % live.len() as u64) as usize];
            let before = values(&net, &g);
            let want = before[n as usize];
            let rows = 1u64 << leaves.len();
            let mut built: HashMap<(usize, u64), AigLit> = HashMap::new();
            fn shannon(
                net: &mut Network,
                leaves: &[AigLit],
                built: &mut HashMap<(usize, u64), AigLit>,
                var: usize,
                f: u64,
                rows: u64,
            ) -> AigLit {
                let mask = if rows == 64 { u64::MAX } else { (1 << rows) - 1 };
                if f & mask == 0 {
                    return AigLit::FALSE;
                }
                if f & mask == mask {
                    return AigLit::TRUE;
                }
                if let Some(&l) = built.get(&(var, f & mask)) {
                    return l;
                }
                let x = leaves[var];
                let half = 1u64 << var;
                let (mut lo, mut hi) = (0u64, 0u64);
                for row in 0..rows {
                    let bit = f >> (row & !half) & 1;
                    lo |= bit << row;
                    let bit = f >> (row | half) & 1;
                    hi |= bit << row;
                }
                let e = shannon(net, leaves, built, var + 1, lo, rows);
                let t = shannon(net, leaves, built, var + 1, hi, rows);
                let l = net.and(x, t);
                let r = net.and(!x, e);
                let out = !net.and(!l, !r);
                built.insert((var, f & mask), out);
                out
            }
            let lit = shannon(&mut net, &leaves, &mut built, 0, want, rows);
            let holds_n = {
                let mut stack = vec![lit.node()];
                let mut seen = std::collections::HashSet::new();
                let mut found = false;
                while let Some(m) = stack.pop() {
                    if m == n {
                        found = true;
                        break;
                    }
                    if !seen.insert(m) {
                        continue;
                    }
                    if let Kind::And(a, b) = net.kind(m) {
                        stack.push(a.node());
                        stack.push(b.node());
                    }
                }
                found
            };
            if holds_n {
                for m in built.values().map(|l| l.node()).collect::<Vec<_>>() {
                    net.free_unread(m);
                }
                net.check().unwrap_or_else(|e| panic!("seed {seed}: freeing an unused build: {e}"));
                continue;
            }
            net.replace(n, lit);
            for m in built.values().map(|l| l.node()).collect::<Vec<_>>() {
                net.free_unread(m);
            }
            replaced += 1;
            net.check().unwrap_or_else(|e| panic!("seed {seed}: after replacing {n}: {e}"));
            let now = values(&net, &g);
            let mask = if rows == 64 { u64::MAX } else { (1 << rows) - 1 };
            for m in net.topological() {
                if live.contains(&m) {
                    assert_eq!(now[m as usize] & mask, before[m as usize] & mask, "seed {seed}: node {m} changed");
                }
            }
        }
        let back = net.to_aig(&g);
        assert_eq!(behaviour(&back), behaviour(&g), "seed {seed}");
        assert_eq!(back.num_ands(), net.num_ands(), "seed {seed}: every live node read back, none merged");
    }
    assert!(replaced > 400, "{replaced} replacements");
}
