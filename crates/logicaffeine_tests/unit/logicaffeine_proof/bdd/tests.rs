use super::*;

/// The canonical-form invariants every node of a manager keeps, whatever built it: no decision node has equal
/// children, every child sits strictly below its parent in the order, and the unique table maps each node to itself.
fn assert_reduced_and_ordered(bdd: &Bdd) {
    for (id, n) in bdd.nodes.iter().enumerate().skip(2) {
        assert_ne!(n.lo, n.hi, "node {id} has equal children");
        assert!(n.var < bdd.level(n.lo) && n.var < bdd.level(n.hi), "node {id} is not above its children");
        assert_eq!(bdd.unique.get(n), Some(&(id as NodeId)), "node {id} is not its own unique-table entry");
    }
    assert_eq!(bdd.unique.len(), bdd.nodes.len() - 2, "every decision node is in the unique table once");
}

#[test]
fn every_operation_leaves_the_manager_reduced_and_ordered() {
    let mut state: u64 = 0x0B0D_D001;
    let mut next = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (state >> 33) as u32
    };
    let mut bdd = Bdd::new(10);
    let mut pool: Vec<NodeId> = (0..10).map(|v| bdd.var(v)).collect();
    for _ in 0..2000 {
        let f = pool[next() as usize % pool.len()];
        let g = pool[next() as usize % pool.len()];
        let r = match next() % 6 {
            0 => bdd.and(f, g),
            1 => bdd.or(f, g),
            2 => bdd.xor(f, g),
            3 => bdd.exists(f, &[next() % 10, next() % 10]),
            4 => bdd.rel_prod(f, g, &[next() % 10]),
            _ => bdd.rename(f, &[(next() % 10, next() % 10)]),
        };
        pool.push(r);
    }
    assert_reduced_and_ordered(&bdd);
}
