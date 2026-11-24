//! ★ LOCK — reduced ordered binary decision diagrams, held to their truth tables. ★
//!
//! `logicaffeine_proof::bdd` is the representation bddbddb gives Datalog relations: a Boolean function over ordered
//! variables, every node unique, so two equal functions are one node. Each operation is checked against the function's
//! truth table, enumerated by brute force over every assignment:
//!
//! - equal functions are the same node, whatever expression built them;
//! - `and`, `or`, `not`, `xor`, `ite` evaluate as the connectives do;
//! - `exists` is the disjunction of the two cofactors, the relational product is `exists` of the conjunction, and a
//!   rename moves a function onto other variables;
//! - `sat_count` counts the satisfying assignments exactly, and parity over n variables has 2n − 1 nodes;
//! - `sat_count_over` counts the assignments of the variables a function is encoded over, wherever they sit in the
//!   order: the size of the set it is the characteristic function of.

use logicaffeine_proof::bdd::{Bdd, NodeId};

struct SplitMix(u64);
impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A random Boolean expression over `vars` variables, as a node and as a truth table.
fn random_function(rng: &mut SplitMix, bdd: &mut Bdd, vars: u32, depth: usize) -> (NodeId, Vec<bool>) {
    let rows = 1usize << vars;
    if depth == 0 || rng.below(4) == 0 {
        let v = rng.below(vars as usize) as u32;
        let node = bdd.var(v);
        return (node, (0..rows).map(|a| a >> v & 1 == 1).collect());
    }
    let (f, tf) = random_function(rng, bdd, vars, depth - 1);
    let (g, tg) = random_function(rng, bdd, vars, depth - 1);
    match rng.below(4) {
        0 => (bdd.and(f, g), tf.iter().zip(&tg).map(|(a, b)| *a && *b).collect()),
        1 => (bdd.or(f, g), tf.iter().zip(&tg).map(|(a, b)| *a || *b).collect()),
        2 => (bdd.xor(f, g), tf.iter().zip(&tg).map(|(a, b)| *a != *b).collect()),
        _ => (bdd.not(f), tf.iter().map(|a| !a).collect()),
    }
}

fn table(bdd: &Bdd, f: NodeId, vars: u32) -> Vec<bool> {
    (0..1usize << vars).map(|a| bdd.eval(f, |v| a >> v & 1 == 1)).collect()
}

#[test]
fn every_operation_evaluates_as_its_truth_table() {
    let mut rng = SplitMix(0xB0D0_0001);
    for _ in 0..400 {
        let mut bdd = Bdd::new(8);
        let (f, expected) = random_function(&mut rng, &mut bdd, 8, 6);
        assert_eq!(table(&bdd, f, 8), expected);
        assert_eq!(bdd.sat_count(f), expected.iter().filter(|b| **b).count() as u128);
    }
}

#[test]
fn equal_functions_are_one_node() {
    let mut bdd = Bdd::new(3);
    let (a, b, c) = (bdd.var(0), bdd.var(1), bdd.var(2));
    let ab = bdd.and(a, b);
    let ac = bdd.and(a, c);
    let left = bdd.or(ab, ac);
    let bc = bdd.or(b, c);
    let right = bdd.and(a, bc);
    assert_eq!(left, right, "(a ∧ b) ∨ (a ∧ c) and a ∧ (b ∨ c) are one function, so one node");
    let mut rng = SplitMix(0xB0D0_0002);
    for _ in 0..300 {
        let mut bdd = Bdd::new(6);
        let (f, tf) = random_function(&mut rng, &mut bdd, 6, 5);
        let (g, tg) = random_function(&mut rng, &mut bdd, 6, 5);
        assert_eq!(f == g, tf == tg, "two nodes are equal exactly when their functions are");
    }
}

#[test]
fn ite_chooses_by_its_condition() {
    let mut rng = SplitMix(0xB0D0_0003);
    for _ in 0..200 {
        let mut bdd = Bdd::new(6);
        let (c, tc) = random_function(&mut rng, &mut bdd, 6, 4);
        let (t, tt) = random_function(&mut rng, &mut bdd, 6, 4);
        let (e, te) = random_function(&mut rng, &mut bdd, 6, 4);
        let ite = bdd.ite(c, t, e);
        let expected: Vec<bool> = (0..64).map(|a| if tc[a] { tt[a] } else { te[a] }).collect();
        assert_eq!(table(&bdd, ite, 6), expected);
    }
}

#[test]
fn exists_is_the_disjunction_of_the_cofactors() {
    let mut rng = SplitMix(0xB0D0_0004);
    for _ in 0..300 {
        let mut bdd = Bdd::new(7);
        let (f, tf) = random_function(&mut rng, &mut bdd, 7, 6);
        let quantified: Vec<u32> = (0..7).filter(|_| rng.below(3) == 0).collect();
        let ex = bdd.exists(f, &quantified);
        let expected: Vec<bool> = (0..128usize)
            .map(|a| {
                let base = quantified.iter().fold(a, |acc, v| acc & !(1 << v));
                (0..1usize << quantified.len()).any(|bits| {
                    let mut assignment = base;
                    for (i, v) in quantified.iter().enumerate() {
                        if bits >> i & 1 == 1 {
                            assignment |= 1 << v;
                        }
                    }
                    tf[assignment]
                })
            })
            .collect();
        assert_eq!(table(&bdd, ex, 7), expected, "∃{quantified:?}");
    }
}

#[test]
fn the_relational_product_is_exists_of_the_conjunction() {
    let mut rng = SplitMix(0xB0D0_0005);
    for _ in 0..300 {
        let mut bdd = Bdd::new(7);
        let (f, _) = random_function(&mut rng, &mut bdd, 7, 5);
        let (g, _) = random_function(&mut rng, &mut bdd, 7, 5);
        let quantified: Vec<u32> = (0..7).filter(|_| rng.below(2) == 0).collect();
        let conj = bdd.and(f, g);
        let two_steps = bdd.exists(conj, &quantified);
        assert_eq!(bdd.rel_prod(f, g, &quantified), two_steps);
    }
}

#[test]
fn a_rename_moves_a_function_onto_other_variables() {
    let mut rng = SplitMix(0xB0D0_0006);
    for _ in 0..200 {
        let mut bdd = Bdd::new(8);
        // A function of variables 0..4 moved onto 4..8, and onto the same variables reversed.
        let (f, tf) = random_function(&mut rng, &mut bdd, 4, 5);
        let up = bdd.rename(f, &[(0, 4), (1, 5), (2, 6), (3, 7)]);
        let reversed = bdd.rename(f, &[(0, 3), (1, 2), (2, 1), (3, 0)]);
        for a in 0..256usize {
            assert_eq!(bdd.eval(up, |v| a >> v & 1 == 1), tf[a >> 4 & 15]);
        }
        for a in 0..16usize {
            let mirrored = (0..4).fold(0, |acc, v| acc | ((a >> v & 1) << (3 - v)));
            assert_eq!(bdd.eval(reversed, |v| a >> v & 1 == 1), tf[mirrored]);
        }
    }
}

#[test]
fn parity_over_n_variables_has_2n_minus_1_nodes() {
    for n in 1..=40u32 {
        let mut bdd = Bdd::new(n);
        let mut parity = bdd.constant(false);
        for v in 0..n {
            let x = bdd.var(v);
            parity = bdd.xor(parity, x);
        }
        assert_eq!(bdd.node_count(parity), (2 * n - 1) as usize, "parity of {n} variables");
        assert_eq!(bdd.sat_count(parity), 1u128 << (n - 1));
    }
}

#[test]
fn every_satisfying_assignment_is_enumerated_once() {
    let mut rng = SplitMix(0xB0D0_0007);
    for _ in 0..200 {
        let mut bdd = Bdd::new(6);
        let (f, tf) = random_function(&mut rng, &mut bdd, 6, 5);
        let mut seen: Vec<usize> = bdd
            .assignments(f, &[0, 1, 2, 3, 4, 5])
            .into_iter()
            .map(|bits| bits.iter().enumerate().fold(0, |acc, (i, b)| acc | ((*b as usize) << i)))
            .collect();
        seen.sort_unstable();
        let expected: Vec<usize> = (0..64).filter(|a| tf[*a]).collect();
        assert_eq!(seen, expected);
    }
}

#[test]
fn a_count_over_its_own_variables_is_the_size_of_the_set() {
    let mut rng = SplitMix(0xB0D0_0008);
    for _ in 0..300 {
        let mut bdd = Bdd::new(12);
        let chosen: Vec<u32> = (0..12).filter(|_| rng.below(2) == 0).collect();
        if chosen.is_empty() {
            continue;
        }
        let k = chosen.len() as u32;
        let (f, tf) = random_function(&mut rng, &mut bdd, k.min(8), 5);
        let k = k.min(8);
        let over: Vec<u32> = chosen[..k as usize].to_vec();
        let moved = bdd.rename(f, &(0..k).map(|i| (i, over[i as usize])).collect::<Vec<_>>());
        let members = tf.iter().filter(|b| **b).count() as u128;
        assert_eq!(bdd.sat_count_over(moved, &over), members, "over {over:?}");
        assert_eq!(bdd.sat_count(moved), members << (12 - k), "the whole manager's count is the set's times the rest");
        let mut wider = over.clone();
        wider.extend((0..12).filter(|v| !over.contains(v)).take(2));
        assert_eq!(
            bdd.sat_count_over(moved, &wider),
            members << (wider.len() - over.len()),
            "a variable the function does not test doubles the count"
        );
    }
}
