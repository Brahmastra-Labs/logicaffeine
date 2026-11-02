use super::*;

/// A graph of `nodes` (name, named by the design) and `edges` (from, to, delay), its ports given by index.
fn graph(nodes: &[(&str, bool)], edges: &[(usize, usize, i64)], inputs: &[usize], outputs: &[usize]) -> TimingGraph {
    TimingGraph {
        nodes: nodes.iter().map(|(n, _)| n.to_string()).collect(),
        public: nodes.iter().map(|&(_, p)| p).collect(),
        edges: edges.iter().map(|&(from, to, delay)| Edge { from, to, delay, registers: 0 }).collect(),
        costs: Vec::new(),
        owner: vec![0; edges.len()],
        instances: vec![String::new()],
        flops: 0,
        area: vec![crate::codegen_sva::timing_graph::Area::default()],
        inputs: inputs.to_vec(),
        outputs: outputs.to_vec(),
        structure: None,
        boxes: Vec::new(),
        fanout: None,
    }
}

/// `in → l1 → … → l{levels} = out`, every net named, a LUT a level.
fn chain(levels: usize) -> TimingGraph {
    let names: Vec<String> = std::iter::once("in".to_string()).chain((1..=levels).map(|i| format!("l{i}"))).collect();
    let nodes: Vec<(&str, bool)> = names.iter().map(|n| (n.as_str(), true)).collect();
    let edges: Vec<(usize, usize, i64)> = (0..levels).map(|i| (i, i + 1, 100)).collect();
    graph(&nodes, &edges, &[0], &[levels])
}

#[test]
fn a_chain_is_cut_every_period_and_its_latency_has_a_floor() {
    let g = chain(8);
    assert_eq!(min_latency(&g, 200, Cuts::Named), Ok(3), "eight levels two to a stage");
    assert_eq!(min_latency(&g, 800, Cuts::Named), Ok(0));
    let p = pipeline(&g, 200, 3, Cuts::Named).unwrap();
    assert!(check(&g, &p), "{p:?}");
    assert_eq!(p.bits, 3);
    let longer = pipeline(&g, 200, 5, Cuts::Named).unwrap();
    assert!(check(&g, &longer));
    assert_eq!(longer.bits, 5, "a register a stage on the one bit");
    assert_eq!(pipeline(&g, 200, 2, Cuts::Named), Err(PipeError::LatencyTooShort { minimum: 3 }));
    assert_eq!(pipeline(&g, 200, -1, Cuts::Named), Err(PipeError::NegativeLatency));
}

#[test]
fn the_cut_lands_where_the_logic_is_narrow() {
    // Twelve inputs into two LUTs, those into one: a stage a level cuts the two nets, not the twelve inputs.
    let mut nodes: Vec<(String, bool)> = (0..12).map(|i| (format!("x[{i}]"), true)).collect();
    nodes.push(("a".into(), true));
    nodes.push(("b".into(), true));
    nodes.push(("y".into(), true));
    let names: Vec<(&str, bool)> = nodes.iter().map(|(n, p)| (n.as_str(), *p)).collect();
    let mut edges: Vec<(usize, usize, i64)> = (0..6).map(|i| (i, 12, 100)).collect();
    edges.extend((6..12).map(|i| (i, 13, 100)));
    edges.push((12, 14, 100));
    edges.push((13, 14, 100));
    let g = graph(&names, &edges, &(0..12).collect::<Vec<_>>(), &[14]);
    let p = pipeline(&g, 100, 1, Cuts::Named).unwrap();
    assert!(check(&g, &p));
    assert_eq!(p.bits, 2);
    assert_eq!(p.cuts(&g), [("a".to_string(), 1, vec![0]), ("b".to_string(), 1, vec![0])]);
    let mut hidden = names.clone();
    hidden[12].1 = false;
    hidden[13].1 = false;
    let g = graph(&hidden, &edges, &(0..12).collect::<Vec<_>>(), &[14]);
    assert!(matches!(pipeline(&g, 100, 1, Cuts::Named), Err(PipeError::Uncuttable { .. })), "two levels with nothing named between them");
}

#[test]
fn a_net_read_three_times_carries_one_register_for_all_three() {
    let nodes = [("x", true), ("m", true), ("p", true), ("q", true), ("r", true), ("y", true)];
    let edges = [(0, 1, 100), (1, 2, 100), (1, 3, 100), (1, 4, 100), (2, 5, 100), (3, 5, 100), (4, 5, 100)];
    let g = graph(&nodes, &edges, &[0], &[5]);
    let p = pipeline(&g, 100, 2, Cuts::Named).unwrap();
    assert!(check(&g, &p), "{p:?}");
    assert_eq!(p.bits, 4, "m once for its three readers, then p, q and r once each for y, which reads all three");
    assert_eq!(p.cuts(&g), [("m".to_string(), 1, vec![0]), ("p".to_string(), 1, vec![0]), ("q".to_string(), 1, vec![0]), ("r".to_string(), 1, vec![0])]);
    assert_eq!(Some(4), fewest(&g, 100, 2));
}

#[test]
fn what_cannot_be_pipelined_is_refused_by_name() {
    let g = chain(3);
    assert_eq!(pipeline(&g, 50, 9, Cuts::Named), Err(PipeError::EdgeTooDeep { edge: 0, delay: 100 }));
    let mut seq = chain(3);
    seq.edges[1].registers = 1;
    assert_eq!(pipeline(&seq, 100, 3, Cuts::Named), Err(PipeError::Sequential));
    let mut looped = chain(3);
    looped.edges.push(Edge { from: 3, to: 1, delay: 100, registers: 0 });
    looped.owner.push(0);
    assert_eq!(pipeline(&looped, 100, 3, Cuts::Named), Err(PipeError::CombinationalLoop));
}

#[test]
fn a_tampered_plan_is_refused() {
    let g = chain(8);
    let p = pipeline(&g, 200, 3, Cuts::Named).unwrap();
    assert!(check(&g, &p));
    let mut early = p.clone();
    early.stage[8] -= 1;
    assert!(!check(&g, &early), "the stages are the registers");
    let mut cheap = p.clone();
    cheap.bits -= 1;
    assert!(!check(&g, &cheap));
    let mut deep = p.clone();
    deep.period = 100;
    assert!(!check(&g, &deep), "two levels a stage is past a period of one");
    let flat = Plan { period: 800, latency: 0, policy: Cuts::Named, stage: vec![0; 9], registers: BTreeMap::new(), bits: 0 };
    assert!(check(&g, &flat));
    assert!(!check(&g, &Plan { period: 700, ..flat.clone() }));
    let mut hidden = g.clone();
    hidden.public[4] = false;
    assert!(!check(&hidden, &p) || p.stage[5] == p.stage[4], "no register on an unnamed net");
}

/// The fewest register bits any valid assignment of stages `0..=latency` gives, by enumeration; `None` when none is
/// valid.
fn fewest(g: &TimingGraph, period: i64, latency: i64) -> Option<i64> {
    fewest_under(g, period, latency, Cuts::Named)
}

/// [`fewest`] with registers where `policy` allows them.
fn fewest_under(g: &TimingGraph, period: i64, latency: i64, policy: Cuts) -> Option<i64> {
    let n = g.nodes.len();
    let mut stage = vec![0i64; n];
    let mut best: Option<i64> = None;
    loop {
        let mut registers = BTreeMap::new();
        let mut ok = g.inputs.iter().all(|&i| stage[i] == 0);
        let mut needs = vec![0i64; n];
        for e in &g.edges {
            let r = stage[e.to] - stage[e.from];
            ok &= r >= 0;
            needs[e.from] = needs[e.from].max(r);
        }
        for &o in &g.outputs {
            ok &= stage[o] <= latency;
            needs[o] = needs[o].max(latency - stage[o]);
        }
        if ok {
            for (v, &r) in needs.iter().enumerate() {
                if r > 0 {
                    registers.insert(v, r);
                }
            }
            let bits = registers.values().sum();
            let plan = Plan { period, latency, policy, stage: stage.clone(), registers, bits };
            if check(g, &plan) && best.map_or(true, |b| bits < b) {
                best = Some(bits);
            }
        }
        let mut i = 0;
        loop {
            if i == n {
                return best;
            }
            stage[i] += 1;
            if stage[i] <= latency {
                break;
            }
            stage[i] = 0;
            i += 1;
        }
    }
}

#[test]
fn random_logic_is_pipelined_with_the_fewest_registers_enumeration_finds() {
    let mut s: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = |m: u64| {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s % m
    };
    let mut solved = 0;
    for round in 0..150 {
        let n = 3 + next(4) as usize;
        let names: Vec<String> = (0..n).map(|i| format!("v{i}")).collect();
        let nodes: Vec<(&str, bool)> = names.iter().map(|x| (x.as_str(), next(4) > 0)).collect();
        let mut edges = Vec::new();
        for to in 1..n {
            for from in 0..to {
                if next(3) == 0 || from + 1 == to && next(2) == 0 {
                    edges.push((from, to, 50 * (1 + next(3) as i64)));
                }
            }
        }
        let inputs: Vec<usize> = (0..n).filter(|&v| !edges.iter().any(|e| e.1 == v)).collect();
        let outputs: Vec<usize> = (0..n).filter(|&v| !edges.iter().any(|e| e.0 == v)).collect();
        let g = graph(&nodes, &edges, &inputs, &outputs);
        let period = 150 + 50 * next(3) as i64;
        let Ok(least) = min_latency(&g, period, Cuts::Named) else { continue };
        for latency in [least, least + 1] {
            let p = pipeline(&g, period, latency, Cuts::Named).unwrap_or_else(|e| panic!("round {round}: {e}"));
            assert!(check(&g, &p), "round {round}: {p:?}");
            assert_eq!(Some(p.bits), fewest(&g, period, latency), "round {round}: {edges:?} named {nodes:?}");
            solved += 1;
        }
        if least > 0 {
            assert_eq!(fewest(&g, period, least - 1), None, "round {round}: nothing shorter than the floor");
        }
    }
    assert!(solved > 100, "{solved}");
}

#[test]
fn a_net_whose_readers_all_need_it_a_stage_later_carries_one_register_for_them_all() {
    let nodes = [("x", true), ("m", true), ("p", true), ("q", true), ("r", true)];
    let edges = [(0, 1, 100), (1, 2, 100), (1, 3, 100), (1, 4, 100)];
    let g = graph(&nodes, &edges, &[0], &[2, 3, 4]);
    let p = pipeline(&g, 100, 1, Cuts::Named).unwrap();
    assert!(check(&g, &p), "{p:?}");
    assert_eq!(p.bits, 1, "one register on m serves p, q and r");
    assert_eq!(p.cuts(&g), [("m".to_string(), 1, vec![0])]);
    assert_eq!(Some(1), fewest(&g, 100, 1));
}

#[test]
fn a_net_the_source_does_not_name_takes_a_register_when_any_net_may() {
    let mut nodes: Vec<(String, bool)> = (0..12).map(|i| (format!("x[{i}]"), true)).collect();
    nodes.extend([("a".to_string(), false), ("b".to_string(), false), ("y".to_string(), true)]);
    let names: Vec<(&str, bool)> = nodes.iter().map(|(n, p)| (n.as_str(), *p)).collect();
    let mut edges: Vec<(usize, usize, i64)> = (0..6).map(|i| (i, 12, 100)).collect();
    edges.extend((6..12).map(|i| (i, 13, 100)));
    edges.extend([(12, 14, 100), (13, 14, 100)]);
    let g = graph(&names, &edges, &(0..12).collect::<Vec<_>>(), &[14]);
    assert!(matches!(pipeline(&g, 100, 1, Cuts::Named), Err(PipeError::Uncuttable { .. })));
    let p = pipeline(&g, 100, 1, Cuts::Nets).unwrap();
    assert!(check(&g, &p));
    assert_eq!((p.bits, p.policy), (2, Cuts::Nets), "the two unnamed nets, not the twelve inputs");
    assert!(!check(&g, &Plan { policy: Cuts::Named, ..p.clone() }), "a named-only plan cannot hold them");
}

/// `width` columns `depth` levels deep, every node of a level reading its own column and the next one's from the
/// level before, a LUT a level, every net named.
fn grid(width: usize, depth: usize) -> TimingGraph {
    let at = |level: usize, column: usize| level * width + column;
    let names: Vec<String> = (0..=depth).flat_map(|l| (0..width).map(move |c| format!("g{l}_{c}"))).collect();
    let nodes: Vec<(&str, bool)> = names.iter().map(|n| (n.as_str(), true)).collect();
    let mut edges = Vec::new();
    for l in 1..=depth {
        for c in 0..width {
            edges.push((at(l - 1, c), at(l, c), 100));
            edges.push((at(l - 1, (c + 1) % width), at(l, c), 100));
        }
    }
    let inputs: Vec<usize> = (0..width).map(|c| at(0, c)).collect();
    let outputs: Vec<usize> = (0..width).map(|c| at(depth, c)).collect();
    graph(&nodes, &edges, &inputs, &outputs)
}

#[test]
fn a_period_constraint_is_added_only_where_a_schedule_breaks_it() {
    // Thirty-two levels eight to a stage: three cuts of sixteen bits. Every node starts a walk that passes the
    // period nine nodes wide eight levels on, some 3,500 constraints; the schedules break a few rows of sixteen.
    let g = grid(16, 32);
    assert_eq!(min_latency(&g, 800, Cuts::Named), Ok(3));
    let (p, problem) = plan(&g, 800, 3, Cuts::Named).unwrap();
    assert!(check(&g, &p), "{p:?}");
    assert_eq!(p.bits, 48);
    let period = problem.constraints.iter().filter(|c| c.2 > 0).count();
    assert!(period <= 3 * 16 * 4, "{period} period constraints");
}

#[test]
fn random_logic_cut_on_any_net_is_pipelined_with_the_fewest_registers_enumeration_finds() {
    let mut s: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = |m: u64| {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s % m
    };
    let mut solved = 0;
    for round in 0..150 {
        let n = 3 + next(4) as usize;
        let names: Vec<String> = (0..n).map(|i| format!("v{i}")).collect();
        let nodes: Vec<(&str, bool)> = names.iter().map(|x| (x.as_str(), next(4) > 0)).collect();
        let mut edges = Vec::new();
        for to in 1..n {
            for from in 0..to {
                if next(3) == 0 || from + 1 == to && next(2) == 0 {
                    edges.push((from, to, 50 * (1 + next(3) as i64)));
                }
            }
        }
        let inputs: Vec<usize> = (0..n).filter(|&v| !edges.iter().any(|e| e.1 == v)).collect();
        let outputs: Vec<usize> = (0..n).filter(|&v| !edges.iter().any(|e| e.0 == v)).collect();
        let g = graph(&nodes, &edges, &inputs, &outputs);
        let period = 150 + 50 * next(3) as i64;
        let least = min_latency(&g, period, Cuts::Nets).unwrap_or_else(|e| panic!("round {round}: any net cuts any path: {e}"));
        for latency in [least, least + 1] {
            let p = pipeline(&g, period, latency, Cuts::Nets).unwrap_or_else(|e| panic!("round {round}: {e}"));
            assert!(check(&g, &p), "round {round}: {p:?}");
            assert_eq!(Some(p.bits), fewest_under(&g, period, latency, Cuts::Nets), "round {round}: {edges:?}");
            solved += 1;
        }
        if least > 0 {
            assert_eq!(fewest_under(&g, period, least - 1, Cuts::Nets), None, "round {round}: nothing shorter than the floor");
        }
    }
    assert_eq!(solved, 300);
}

#[test]
fn registering_the_outputs_adds_a_clock_and_one_register_a_bit_and_the_plan_still_checks() {
    let g = chain(8);
    let p = pipeline(&g, 200, 3, Cuts::Named).unwrap();
    let r = p.with_outputs_registered(&g);
    assert_eq!(r.latency, 4);
    assert_eq!(r.stage, p.stage, "no node moves");
    assert_eq!(r.registers[&8], p.registers.get(&8).copied().unwrap_or(0) + 1, "the output net carries one more");
    assert_eq!(r.bits, p.bits + 1);
    assert!(check(&g, &r), "{r:?}");
    let both = graph(&[("x", true), ("y", true), ("z", true)], &[(0, 1, 100), (1, 2, 100)], &[0], &[1, 2, 2]);
    let p = pipeline(&both, 100, 1, Cuts::Named).unwrap();
    let r = p.with_outputs_registered(&both);
    assert_eq!(r.bits, p.bits + 2, "a node that is two output bits is one net: one register");
    assert!(check(&both, &r), "{r:?}");
}

/// `graph` with `registers` on the edges named by index: a pipeline drawn by hand.
fn piped(nodes: &[(&str, bool)], edges: &[(usize, usize, i64, i64)], inputs: &[usize], outputs: &[usize]) -> TimingGraph {
    let flat: Vec<(usize, usize, i64)> = edges.iter().map(|&(f, t, d, _)| (f, t, d)).collect();
    let mut g = graph(nodes, &flat, inputs, outputs);
    for (e, &(_, _, _, r)) in g.edges.iter_mut().zip(edges) {
        e.registers = r;
    }
    g.flops = edges.iter().filter(|e| e.3 > 0).count();
    g
}

#[test]
fn a_pipeline_unpipelined_is_its_logic_and_the_latency_its_registers_made() {
    // in → a → [r] → b → c → [r] → out, the second stage three times the first.
    let g = piped(
        &[("in", true), ("a", true), ("a_q", true), ("b", true), ("c", true), ("out", true)],
        &[(0, 1, 100, 0), (1, 2, 0, 1), (2, 3, 100, 0), (3, 4, 100, 0), (4, 5, 0, 1)],
        &[0],
        &[5],
    );
    let (logic, latency) = unpipelined(&g).unwrap();
    assert_eq!(latency, 2);
    assert_eq!(logic.flops, 0);
    assert!(logic.edges.iter().all(|e| e.registers == 0));
    assert_eq!(logic.depth().unwrap().depth, 300);
    let p = pipeline(&logic, 200, latency, Cuts::Named).unwrap();
    assert!(check(&logic, &p), "{p:?}");
    assert_eq!(p.bits, 2, "a chain of one bit two clocks long: {p:?}");
    let stage_of = |name: &str| p.stage[logic.nodes.iter().position(|n| n == name).unwrap()];
    assert!(stage_of("b") - stage_of("a") + stage_of("c") - stage_of("b") <= 1, "the deep stage is split at most once: {p:?}");
}

#[test]
fn a_pipeline_whose_paths_hold_different_registers_or_whose_registers_have_controls_is_refused() {
    // in → a → out directly, and in → [r] → b → out: out is a clock late on one path and not on the other.
    let uneven = piped(
        &[("in", true), ("a", true), ("q", true), ("b", true), ("out", true)],
        &[(0, 1, 100, 0), (1, 4, 100, 0), (0, 2, 0, 1), (2, 3, 100, 0), (3, 4, 100, 0)],
        &[0],
        &[4],
    );
    assert!(matches!(unpipelined(&uneven), Err(PipeError::Unbalanced { .. })), "{:?}", unpipelined(&uneven));
    // A register with an enable: two registered edges into its output.
    let held = piped(&[("d", true), ("en", true), ("q", true), ("y", true)], &[(0, 2, 0, 1), (1, 2, 0, 1), (2, 3, 100, 0)], &[0, 1], &[3]);
    assert!(matches!(unpipelined(&held), Err(PipeError::Held { register }) if register == "q"));
    let looped = piped(&[("x", true), ("q", true), ("y", true)], &[(0, 2, 100, 0), (1, 2, 100, 0), (2, 1, 0, 1)], &[0], &[2]);
    assert!(matches!(unpipelined(&looped), Err(PipeError::CombinationalLoop)), "a register in a loop is state, not a stage");
}

#[test]
fn a_node_the_logic_reads_that_nothing_drives_is_refused_by_name_not_written_undriven() {
    // in → m → out, out a copy of m, m driven by nothing the netlist says: a register's output left unresolved.
    let mut g = graph(&[("in", true), ("m", true), ("out", true)], &[(0, 1, 0), (1, 2, 0)], &[0], &[2]);
    g.structure = Some(crate::codegen_sva::timing_graph::Structure {
        source: vec![Source::Input { port: "in".into(), bit: 0 }, Source::Inside, Source::Copy { from: 1, inverted: false }],
        cells: Vec::new(),
        input_ports: vec![("in".into(), vec![Pin::Node(0)])],
        output_ports: vec![("out".into(), vec![Pin::Node(2)])],
    });
    let plan = pipeline(&g, 1, 0, Cuts::Nets).unwrap();
    assert_eq!(emit(&g, &plan, "undriven", None), Err(EmitError::Undriven { node: "m".into() }));
}
