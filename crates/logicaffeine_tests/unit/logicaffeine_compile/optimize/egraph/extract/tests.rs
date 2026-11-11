use super::*;

/// The extraction as a fixpoint over the whole graph: every node in ascending id order, round after round, a class's
/// representative replaced only by a strictly cheaper node. What an extraction that reads only its root's reach
/// must agree with.
fn whole_graph_best(
    eg: &mut CompilerEGraph,
    root: NodeId,
    admissible: &dyn Fn(&CompilerENode) -> bool,
) -> Option<ExtractTree> {
    let mut best: HashMap<NodeId, (u64, NodeId)> = HashMap::new();
    loop {
        let mut changed = false;
        for id in 0..eg.node_count() {
            let node = eg.canonical_node(id);
            if !admissible(&node) {
                continue;
            }
            let mut total = op_cost(&node);
            let mut ready = true;
            for c in node.children() {
                match best.get(&c) {
                    Some(&(cc, _)) => total = total.saturating_add(cc),
                    None => {
                        ready = false;
                        break;
                    }
                }
            }
            if !ready {
                continue;
            }
            let class = eg.find(id);
            match best.get(&class) {
                Some(&(old, _)) if old <= total => {}
                _ => {
                    best.insert(class, (total, id));
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    fn build(eg: &mut CompilerEGraph, best: &HashMap<NodeId, (u64, NodeId)>, class: NodeId) -> Option<ExtractTree> {
        let root = eg.find(class);
        let (_, rep) = *best.get(&root)?;
        let node = eg.canonical_node(rep);
        let children = node.children().into_iter().map(|c| build(eg, best, c)).collect::<Option<Vec<_>>>()?;
        Some(ExtractTree { node, children })
    }
    build(eg, &best, root)
}

/// A deterministic stream of numbers.
struct Draws(u64);

impl Draws {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A graph of `size` nodes drawn from `draws`: literals, variable versions and opaques, operators over earlier
/// classes, and unions among classes that may hold one value, cycles included (`x ≡ x + 0`). Two classes whose ranges
/// are disjoint hold two values, and a union of them is an unsound rule's work, which the egraph refuses.
fn drawn_graph(draws: &mut Draws, size: usize) -> CompilerEGraph {
    let mut eg = CompilerEGraph::new();
    let mut ids: Vec<NodeId> = Vec::new();
    while eg.node_count() < size {
        let pick = |draws: &mut Draws, ids: &[NodeId]| ids[draws.below(ids.len())];
        let node = match (ids.is_empty(), draws.below(10)) {
            (true, _) | (_, 0) => CompilerENode::Int(draws.below(4) as i64),
            (_, 1) => CompilerENode::Var(draws.below(3) as u32, draws.below(3) as u32),
            (_, 2) => CompilerENode::Opaque(draws.below(4) as u32),
            (_, 3) => CompilerENode::Add(pick(draws, &ids), pick(draws, &ids)),
            (_, 4) => CompilerENode::Mul(pick(draws, &ids), pick(draws, &ids)),
            (_, 5) => CompilerENode::Sub(pick(draws, &ids), pick(draws, &ids)),
            (_, 6) => CompilerENode::Len(pick(draws, &ids)),
            (_, 7) => CompilerENode::Div(pick(draws, &ids), pick(draws, &ids)),
            _ => {
                let (a, b) = (pick(draws, &ids), pick(draws, &ids));
                let disjoint = match (eg.int_range(a), eg.int_range(b)) {
                    (Some((a_lo, a_hi)), Some((b_lo, b_hi))) => a_hi < b_lo || b_hi < a_lo,
                    _ => false,
                };
                if !disjoint {
                    eg.union(a, b);
                }
                continue;
            }
        };
        ids.push(eg.add(node));
    }
    eg
}

#[test]
fn an_extraction_agrees_with_the_whole_graph_fixpoint() {
    let mut draws = Draws(0x9e37_79b9_7f4a_7c15);
    let mut compared = 0;
    let mut found = 0;
    for _ in 0..300 {
        let size = 2 + draws.below(40);
        let mut eg = drawn_graph(&mut draws, size);
        let versions = [draws.below(3) as u32, draws.below(3) as u32, draws.below(3) as u32];
        let (lo, hi) = {
            let a = draws.below(5) as u32;
            (a, a + draws.below(3) as u32)
        };
        let filtered = move |n: &CompilerENode| match *n {
            CompilerENode::Var(ix, v) => versions[ix as usize] == v,
            CompilerENode::Opaque(j) => (lo..hi).contains(&j),
            _ => true,
        };
        let filters: [&dyn Fn(&CompilerENode) -> bool; 2] = [&|_| true, &filtered];
        let extraction = Extraction::of(&mut eg);
        for admissible in filters {
            for root in 0..eg.node_count() {
                let want = whole_graph_best(&mut eg, root, admissible);
                let got = extraction.best_tree(root, admissible);
                found += usize::from(want.is_some());
                compared += 1;
                assert_eq!(
                    format!("{got:?}"),
                    format!("{want:?}"),
                    "root {root} of a graph of {size} nodes: the extraction differs from the whole-graph fixpoint"
                );
            }
        }
    }
    assert!(compared > 3000 && found > compared / 3, "the drawn graphs exercise extraction: {found} trees of {compared}");
}

#[test]
fn an_extraction_reads_only_what_its_root_reaches() {
    let mut eg = CompilerEGraph::new();
    let mut sites = Vec::new();
    for k in 0..2000u32 {
        let total = eg.add(CompilerENode::Var(0, k));
        let call = eg.add(CompilerENode::Opaque(k));
        sites.push(eg.add(CompilerENode::Add(total, call)));
    }
    let extraction = Extraction::of(&mut eg);
    for (k, site) in sites.iter().enumerate() {
        let k = k as u32;
        let admissible = move |n: &CompilerENode| match *n {
            CompilerENode::Var(_, v) => v == k,
            CompilerENode::Opaque(j) => j == k,
            _ => true,
        };
        assert_eq!(
            extraction.reached(*site, &admissible).len(),
            3,
            "a statement's extraction reads its own three nodes, not the run's {}",
            eg.node_count()
        );
        assert!(extraction.best_tree(*site, &admissible).is_some());
    }
}
