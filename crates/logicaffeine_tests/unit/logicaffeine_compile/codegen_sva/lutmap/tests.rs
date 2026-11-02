use super::*;

fn inputs(x: &mut Xag, n: usize) -> Vec<Lit> {
    (0..n).map(|_| x.input()).collect()
}

/// `kind` of `leaves` as a balanced binary tree, the way a netlist hands a wide gate over.
fn binary(x: &mut Xag, xor: bool, leaves: &[Lit]) -> Lit {
    let mut v = leaves.to_vec();
    while v.len() > 1 {
        v = v.chunks(2).map(|p| if p.len() == 2 { if xor { x.xor(p[0], p[1]) } else { x.and(p[0], p[1]) } } else { p[0] }).collect();
    }
    v[0]
}

fn one_group(x: &Xag) -> Vec<u32> {
    vec![0; x.len()]
}

/// The level of the deepest LUT the cover uses for `outputs`.
fn depth(m: &Mapping, outputs: &[Lit]) -> u32 {
    outputs.iter().filter_map(|&l| m.image[node(l)]).map(|l| m.level[node(l)]).max().unwrap_or(0)
}

/// Every LUT's leaves cut its root from the inputs, and the levels are the cover's own: a LUT one above its latest
/// leaf.
fn check_cover(m: &Mapping, k: usize) {
    for lut in &m.luts {
        assert!(!lut.leaves.is_empty() && lut.leaves.len() <= k, "{lut:?}");
        let mut stack = vec![lut.root as usize];
        let mut seen = std::collections::HashSet::new();
        while let Some(n) = stack.pop() {
            if !seen.insert(n) || (n != lut.root as usize && lut.leaves.contains(&(n as u32))) {
                continue;
            }
            match m.xag.gate(n) {
                Gate::And(a, b) | Gate::Xor(a, b) => {
                    stack.push(node(a));
                    stack.push(node(b));
                }
                Gate::Input | Gate::Constant => panic!("LUT {lut:?} reaches node {n}, past its leaves"),
            }
        }
        let latest = lut.leaves.iter().map(|&l| m.level[l as usize]).max().unwrap();
        assert_eq!(m.level[lut.root as usize], latest + 1, "{lut:?}");
    }
}

#[test]
fn the_graph_folds_constants_and_hashes_structure() {
    let mut x = Xag::new();
    let [a, b] = [x.input(), x.input()];
    assert_eq!(x.and(a, FALSE), FALSE);
    assert_eq!(x.and(a, TRUE), a);
    assert_eq!(x.and(a, a), a);
    assert_eq!(x.and(a, a ^ 1), FALSE);
    assert_eq!(x.and(a, b), x.and(b, a));
    assert_eq!(x.xor(a, a), FALSE);
    assert_eq!(x.xor(a, a ^ 1), TRUE);
    assert_eq!(x.xor(a ^ 1, b), x.xor(a, b) ^ 1, "a complement comes out of an XOR");
    assert_eq!(x.xor(FALSE, b), b);
    assert_eq!(x.xor(TRUE, b), b ^ 1);
    let before = x.len();
    x.and(b, a);
    x.xor(b, a ^ 1);
    assert_eq!(x.len(), before, "nothing new for a gate already there");
    let m = x.mux(a, b, a ^ 1);
    let v = x.simulate(&[0b1100, 0b1010]);
    let val = |l: Lit| if complemented(l) { !v[node(l)] } else { v[node(l)] } & 0b1111;
    assert_eq!(val(m), 0b1011, "a ? b : !a");
}

#[test]
fn a_wide_and_or_parity_is_log_k_of_its_width_deep() {
    for (xor, width, levels) in [(false, 6, 1), (false, 7, 2), (false, 36, 2), (false, 216, 3), (false, 217, 4), (true, 36, 2), (true, 216, 3)] {
        let mut x = Xag::new();
        let ins = inputs(&mut x, width);
        let out = binary(&mut x, xor, &ins);
        let out = if xor { out } else { out ^ 1 };
        let m = map(&x, &one_group(&x), &[out], 6);
        assert_eq!(depth(&m, &[out]), levels, "{} of {width}", if xor { "parity" } else { "NAND" });
        check_cover(&m, 6);
    }
}

#[test]
fn the_balanced_graph_computes_what_the_original_did() {
    let mut s: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = |m: u64| {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s % m
    };
    for round in 0..200 {
        let mut x = Xag::new();
        let n_in = 3 + next(10) as usize;
        let mut lits = inputs(&mut x, n_in);
        for _ in 0..(5 + next(60)) {
            let a = lits[next(lits.len() as u64) as usize] ^ next(2) as u32;
            let b = lits[next(lits.len() as u64) as usize] ^ next(2) as u32;
            let l = if next(3) == 0 { x.xor(a, b) } else { x.and(a, b) };
            lits.push(l);
        }
        let outputs: Vec<Lit> = (0..1 + next(4)).map(|_| lits[n_in + next((lits.len() - n_in) as u64) as usize]).collect();
        let groups: Vec<u32> = (0..x.len()).map(|_| next(2) as u32).collect();
        let k = 3 + next(4) as usize;
        let m = map(&x, &groups, &outputs, k);
        check_cover(&m, k);
        let words: Vec<u64> = (0..n_in).map(|_| next(u64::MAX)).collect();
        let (old, new) = (x.simulate(&words), m.xag.simulate(&words));
        let val = |v: &[u64], l: Lit| if complemented(l) { !v[node(l)] } else { v[node(l)] };
        for o in 1..x.len() {
            if let Some(img) = m.image[o] {
                assert_eq!(val(&old, (o as u32) << 1), val(&new, img), "round {round}: node {o}");
            }
        }
        for &out in &outputs {
            assert!(m.image[node(out)].is_some(), "round {round}: an output has an image");
            let root = node(m.image[node(out)].unwrap());
            assert!(root == 0 || matches!(m.xag.gate(root), Gate::Input) || m.luts.iter().any(|l| l.root as usize == root), "round {round}: an output's node is a LUT");
        }
    }
}

#[test]
fn a_late_leaf_waits_while_the_early_ones_are_gathered() {
    // A 36-input AND whose last leaf is three levels deep: the 35 early leaves take two levels, and the late one joins
    // at the last, four levels in all, not five.
    let mut x = Xag::new();
    let ins = inputs(&mut x, 35 + 216);
    let late = binary(&mut x, true, &ins[35..]);
    let mut leaves = ins[..35].to_vec();
    leaves.push(late);
    let out = binary(&mut x, false, &leaves);
    let m = map(&x, &one_group(&x), &[out], 6);
    assert_eq!(m.level[node(m.image[node(late)].unwrap())], 3);
    assert_eq!(depth(&m, &[out]), 4);
    check_cover(&m, 6);
}

#[test]
fn a_supergate_stops_at_its_groups_edge_and_at_a_node_read_twice() {
    let mut x = Xag::new();
    let ins = inputs(&mut x, 12);
    let left = binary(&mut x, false, &ins[..6]);
    let right = binary(&mut x, false, &ins[6..]);
    let out = x.and(left, right);
    let mut groups = one_group(&x);
    for n in 0..x.len() {
        if n > node(left) && n <= node(right) {
            groups[n] = 1;
        }
    }
    groups[node(out)] = 0;
    let m = map(&x, &groups, &[out], 6);
    assert!(m.image[node(right)].is_some(), "the other group's tree is not absorbed");
    assert!(m.image[node(left)].is_none(), "its own is");
    let mut x2 = Xag::new();
    let ins = inputs(&mut x2, 12);
    let shared = binary(&mut x2, false, &ins[..6]);
    let rest = binary(&mut x2, false, &ins[6..]);
    let out = x2.and(shared, rest);
    let m = map(&x2, &one_group(&x2), &[out, shared], 6);
    assert!(m.image[node(shared)].is_some(), "a node an output reads keeps its own image");
    assert_eq!(depth(&m, &[out]), 2);
    check_cover(&m, 6);
}

#[test]
fn every_original_node_is_traced_to_the_balanced_node_that_computes_it() {
    let mut x = Xag::new();
    let ins = inputs(&mut x, 8);
    let out = binary(&mut x, false, &ins);
    let m = map(&x, &one_group(&x), &[out], 6);
    let img = node(m.image[node(out)].unwrap());
    assert_eq!((m.origin[img], m.exact[img]), (node(out) as u32, true));
    let made: Vec<usize> = (1..m.xag.len()).filter(|&n| !m.exact[n]).collect();
    assert!(!made.is_empty(), "balancing made inner nodes");
    assert!(made.iter().all(|&n| m.origin[n] == node(out) as u32), "each stands for the supergate it was made for");
    for (o, &input) in ins.iter().enumerate() {
        let i = node(m.image[node(input)].unwrap());
        assert_eq!((m.origin[i], m.exact[i]), (node(input) as u32, true), "input {o}");
    }
}

#[test]
fn a_luts_function_is_its_cone_over_its_leaves() {
    let mut x = Xag::new();
    let [a, b, c] = [x.input(), x.input(), x.input()];
    let ab = x.and(a, b ^ 1);
    let f = x.xor(ab, c);
    let leaves = [node(a) as u32, node(b) as u32, node(c) as u32];
    let node_table = function(&x, node(f) as u32, &leaves);
    let table = if complemented(f) { !node_table & 0xFF } else { node_table };
    for i in 0..8u64 {
        let (va, vb, vc) = (i & 1 == 1, i >> 1 & 1 == 1, i >> 2 & 1 == 1);
        assert_eq!(table >> i & 1 == 1, (va && !vb) ^ vc, "row {i}");
    }
    let over = function(&x, node(f) as u32, &[node(ab) as u32, node(c) as u32]);
    assert_eq!(if complemented(f) { !over & 0xF } else { over }, 0b0110, "over the AND and c: an XOR");
}
