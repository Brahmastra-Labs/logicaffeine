//! Pipelining combinational logic to a target clock with the fewest register bits, the cut certified optimal.
//!
//! [`pipeline`] gives every node of a combinational [`TimingGraph`] a stage: its inputs at 0, its outputs at the
//! latency, registers only where [`Cuts`] allows them (on the nets the design names, the cuts a designer writes into
//! its source; or on any net, for a pipeline [`emit`] writes out), and no path between registers deeper than the
//! period. It is the minimum-area retiming of Leiserson & Saxe (*Retiming
//! synchronous circuitry*, 1991) of the logic with its outputs fed back to its inputs through the latency's registers,
//! written as a linear program over differences of stages:
//! - an edge out of a node no register may follow holds none, so its ends share a stage; any other edge holds
//!   `s(to) − s(from) ≥ 0`;
//! - a node where the depth from wherever a stage can begin (past a register on a net into it, or an input) first
//!   passes the period lies at least a stage later. These constraints number a node's reach over the period, every
//!   node over; only those a schedule breaks are added, found by walking each stage of the schedule from where it
//!   begins, and the schedule solved again until it breaks none. The rest it meets as they stand, so its optimum
//!   over the constraints added is the optimum over them all, its certificate the same flow;
//! - the registers on a net are shared by its readers, so a net costs the most any reader needs, through a mirror
//!   node no earlier than any reader (Leiserson & Saxe's fanout sharing).
//!
//! Its dual is a minimum-cost flow ([`logicaffeine_proof::min_cost_flow`]): the flow's potentials are the stages,
//! integral and optimal by complementary slackness, and a latency too short for the period is the dual's negative
//! cycle. [`check`] replays a plan against the graph: every register count it implies is whole and where a register
//! may go, every input is at stage 0 and every output reaches the latency, and the depth between registers is within
//! the period. [`emit`] writes a plan out as a Verilog module, every connection reading its source through the
//! registers the plan puts on it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use logicaffeine_proof::cycle_ratio::Edge;
use logicaffeine_proof::min_cost_flow::{min_cost_flow, Arc, FlowError, Solution};

use super::timing_graph::{ConeArg, Pin, Source, TimingGraph};

/// Where registers may go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cuts {
    /// On the bits of nets the design names, and on its ports: the cuts a designer writes into the source.
    Named,
    /// On any net: every LUT's output, every kept cell's ports, every port; never inside a kept cell.
    Nets,
}

/// A pipeline: each node's stage, and the registers each net bit carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub period: i64,
    pub latency: i64,
    pub policy: Cuts,
    /// Each node's stage: the registers between the inputs and it.
    pub stage: Vec<i64>,
    /// The net bits that carry registers, with how many, by node.
    pub registers: BTreeMap<usize, i64>,
    /// The register bits in all, the least any pipeline of this period and latency needs.
    pub bits: i64,
}

impl Plan {
    /// The same pipeline a clock longer, every output net carrying one more register past the last stage: its outputs
    /// read straight from registers. A pipeline whose outputs are registers is one a clock shorter with them added, so
    /// of those, this has the fewest bits.
    pub fn with_outputs_registered(&self, graph: &TimingGraph) -> Plan {
        let mut plan = self.clone();
        plan.latency += 1;
        let outputs: BTreeSet<usize> = graph.outputs.iter().copied().collect();
        for o in outputs {
            *plan.registers.entry(o).or_insert(0) += 1;
            plan.bits += 1;
        }
        plan
    }

    /// The registers by net, its bits gathered: each net's name, and for each register count the bits carrying it.
    pub fn cuts(&self, graph: &TimingGraph) -> Vec<(String, i64, Vec<usize>)> {
        let mut by: BTreeMap<(String, i64), Vec<usize>> = BTreeMap::new();
        for (&node, &count) in &self.registers {
            let name = &graph.nodes[node];
            let (base, bit) = match name.rfind('[') {
                Some(at) if name.ends_with(']') => (name[..at].to_string(), name[at + 1..name.len() - 1].parse().unwrap_or(0)),
                _ => (name.clone(), 0),
            };
            by.entry((base, count)).or_default().push(bit);
        }
        by.into_iter()
            .map(|((name, count), mut bits)| {
                bits.sort_unstable();
                (name, count, bits)
            })
            .collect()
    }
}

/// Why logic was not pipelined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipeError {
    /// The graph holds registers already: it is pipelined from combinational logic.
    Sequential,
    /// An edge is deeper than the period by itself, and no register splits an edge.
    EdgeTooDeep { edge: usize, delay: i64 },
    /// The latency is too short for the period: the least that is enough.
    LatencyTooShort { minimum: i64 },
    /// Logic deeper than the period with nowhere the policy allows a register, at any latency: the nets around the
    /// loop of constraints that says so.
    Uncuttable { through: Vec<String> },
    /// The latency is negative.
    NegativeLatency,
    /// The logic loops through no register.
    CombinationalLoop,
    /// The paths into a node hold different numbers of registers: not a pipeline, whose every path from its inputs
    /// to a node holds as many.
    Unbalanced { node: String },
    /// A register with more than its data in (an enable, a reset, a memory's port): state, not a pipeline's stage.
    Held { register: String },
    /// An instance of a module timed apart: its logic, and any loop through it, is not the graph's.
    Boxed { cell: String },
    Flow(FlowError),
}

impl std::fmt::Display for PipeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipeError::Sequential => write!(f, "the logic holds registers already"),
            PipeError::EdgeTooDeep { edge, delay } => write!(f, "edge {edge} is {delay} deep, past the period by itself"),
            PipeError::LatencyTooShort { minimum } => write!(f, "the period needs a latency of at least {minimum}"),
            PipeError::Uncuttable { through } => write!(f, "logic deeper than the period with nowhere to cut it: {}", through.join(", ")),
            PipeError::NegativeLatency => write!(f, "a latency is not negative"),
            PipeError::CombinationalLoop => write!(f, "the logic loops through no register"),
            PipeError::Unbalanced { node } => write!(f, "the paths into {node} hold different numbers of registers"),
            PipeError::Held { register } => write!(f, "{register} is a register with more than its data in"),
            PipeError::Boxed { cell } => write!(f, "{cell} is a box, whose logic is not here to cut"),
            PipeError::Flow(e) => write!(f, "the flow problem: {e:?}"),
        }
    }
}

impl std::error::Error for PipeError {}

/// Where a register may go under `policy`: always on the ports.
fn cuts(graph: &TimingGraph, policy: Cuts) -> Vec<bool> {
    let n = graph.nodes.len();
    let mut cut = match (policy, &graph.structure) {
        (Cuts::Named, _) => graph.public.clone(),
        (Cuts::Nets, Some(s)) => s.source.iter().map(|x| !matches!(x, Source::Inside | Source::Register { .. })).collect(),
        (Cuts::Nets, None) => vec![true; n],
    };
    cut.resize(n, false);
    for &i in graph.inputs.iter().chain(&graph.outputs) {
        cut[i] = true;
    }
    cut
}

/// The combinational edges in topological order of their sources, each node's position in it, and each node's
/// edges out.
fn order(graph: &TimingGraph) -> (Vec<usize>, Vec<Vec<usize>>) {
    let n = graph.nodes.len();
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut fanin = vec![0usize; n];
    for (k, e) in graph.edges.iter().enumerate() {
        out[e.from].push(k);
        fanin[e.to] += 1;
    }
    let mut ready: VecDeque<usize> = (0..n).filter(|&v| fanin[v] == 0).collect();
    let mut position = vec![usize::MAX; n];
    let mut at = 0;
    while let Some(v) = ready.pop_front() {
        position[v] = at;
        at += 1;
        for &k in &out[v] {
            let t = graph.edges[k].to;
            fanin[t] -= 1;
            if fanin[t] == 0 {
                ready.push_back(t);
            }
        }
    }
    (position, out)
}

/// A graph with no box in it: a box's logic, and every loop through it, is not the graph's to cut around.
fn unboxed(graph: &TimingGraph) -> Result<(), PipeError> {
    match graph.boxes.first() {
        Some(b) => Err(PipeError::Boxed { cell: b.cell.clone() }),
        None => Ok(()),
    }
}

/// The logic of a pipeline with its registers taken out, and the latency they made: a graph [`pipeline`] cuts afresh
/// at any period. A pipeline's registers are plain flip-flops, one registered edge into each, and every path from its
/// inputs to a node holds as many of them, so every output is the same number of clocks late; taken out, each is a wire
/// from its data, or its data's constant when the source hands it one. A register with more in than its data (an
/// enable, a reset, a memory's port) is refused by name, and so is a node whose paths hold different numbers of
/// registers; a register in a loop leaves logic looping through none.
pub fn unpipelined(graph: &TimingGraph) -> Result<(TimingGraph, i64), PipeError> {
    unboxed(graph)?;
    let n = graph.nodes.len();
    if let Some(s) = &graph.structure {
        if let Some(q) = (0..n).find(|&v| matches!(s.source[v], Source::Register { controls: true, .. })) {
            return Err(PipeError::Held { register: graph.nodes[q].clone() });
        }
    }
    let (mut held, mut wired) = (vec![0usize; n], vec![false; n]);
    for e in &graph.edges {
        if e.registers > 0 {
            held[e.to] += 1;
        } else {
            wired[e.to] = true;
        }
    }
    if let Some(q) = (0..n).find(|&v| held[v] > 1 || held[v] == 1 && wired[v]) {
        return Err(PipeError::Held { register: graph.nodes[q].clone() });
    }
    let mut logic = graph.clone();
    logic.flops = 0;
    if let Some(s) = &mut logic.structure {
        for source in &mut s.source {
            if let Source::Register { data, .. } = *source {
                *source = match data {
                    Pin::Node(d) => Source::Copy { from: d, inverted: false },
                    Pin::Zero => Source::Constant(false),
                    Pin::One => Source::Constant(true),
                };
            }
        }
    }
    for (k, e) in graph.edges.iter().enumerate() {
        if e.registers > 0 {
            logic.edges[k].registers = 0;
            if let Some(s) = &mut logic.structure {
                if matches!(s.source[e.to], Source::Inside) {
                    s.source[e.to] = Source::Copy { from: e.from, inverted: false };
                }
            }
        }
    }
    let (position, out) = order(&logic);
    if position.iter().any(|&p| p == usize::MAX) {
        return Err(PipeError::CombinationalLoop);
    }
    let mut topo = vec![0usize; n];
    for (v, &p) in position.iter().enumerate() {
        topo[p] = v;
    }
    let mut late: Vec<Option<i64>> = vec![None; n];
    for &i in &graph.inputs {
        late[i] = Some(0);
    }
    for &v in &topo {
        let Some(at) = late[v] else { continue };
        for &k in &out[v] {
            let e = &graph.edges[k];
            let reach = at + e.registers;
            match late[e.to] {
                Some(have) if have != reach => return Err(PipeError::Unbalanced { node: graph.nodes[e.to].clone() }),
                _ => late[e.to] = Some(reach),
            }
        }
    }
    let mut latency = None;
    for &o in &graph.outputs {
        let Some(l) = late[o] else { continue };
        match latency {
            Some(have) if have != l => return Err(PipeError::Unbalanced { node: graph.nodes[o].clone() }),
            _ => latency = Some(l),
        }
    }
    Ok((logic, latency.unwrap_or(0)))
}

/// The difference constraints `s(to) − s(from) ≥ least` of pipelining `graph` at `period`, over its nodes, a mirror
/// for each net a register may follow that is read, a sink for each output and a host (the last), the output sinks'
/// latency left out; each variable's coefficient in the register count; and what finding the period's constraints a
/// schedule breaks needs. The constraints hold the structure, then the period's constraints found so far.
struct Problem {
    vars: usize,
    host: usize,
    sink: Vec<usize>,
    mirror: Vec<Option<usize>>,
    constraints: Vec<(usize, usize, i64)>,
    weight: Vec<i64>,
    /// Each node's depth past a register on its deepest edge in from a net a register may follow; −1 with none.
    start: Vec<i64>,
    /// The nodes in topological order, and each node's edges out.
    topo: Vec<usize>,
    out: Vec<Vec<usize>>,
}

fn problem(graph: &TimingGraph, period: i64, policy: Cuts) -> Result<Problem, PipeError> {
    unboxed(graph)?;
    if graph.flops > 0 || graph.edges.iter().any(|e| e.registers != 0) {
        return Err(PipeError::Sequential);
    }
    if let Some((k, e)) = graph.edges.iter().enumerate().find(|(_, e)| e.delay > period) {
        return Err(PipeError::EdgeTooDeep { edge: k, delay: e.delay });
    }
    let n = graph.nodes.len();
    let cut = cuts(graph, policy);
    let (position, out) = order(graph);
    if position.iter().any(|&p| p == usize::MAX) {
        return Err(PipeError::CombinationalLoop);
    }
    let mut topo = vec![0usize; n];
    for (v, &p) in position.iter().enumerate() {
        topo[p] = v;
    }
    let mut constraints: Vec<(usize, usize, i64)> = Vec::new();
    let mut vars = n;
    let mut mirror: Vec<Option<usize>> = vec![None; n];
    for v in 0..n {
        if cut[v] && !out[v].is_empty() {
            mirror[v] = Some(vars);
            vars += 1;
        }
    }
    let sink: Vec<usize> = graph.outputs.iter().map(|_| { vars += 1; vars - 1 }).collect();
    for (i, &o) in graph.outputs.iter().enumerate() {
        if mirror[o].is_none() {
            mirror[o] = Some(vars);
            vars += 1;
        }
        constraints.push((o, sink[i], 0));
        constraints.push((sink[i], mirror[o].expect("an output has a mirror"), 0));
    }
    let host = vars;
    vars += 1;
    let mut weight = vec![0i64; vars];
    for v in 0..n {
        if let Some(m) = mirror[v] {
            weight[m] += 1;
            weight[v] -= 1;
        }
    }
    let mut start: Vec<i64> = vec![-1; n];
    for e in &graph.edges {
        constraints.push((e.from, e.to, 0));
        if cut[e.from] {
            if let Some(m) = mirror[e.from] {
                constraints.push((e.to, m, 0));
            }
            start[e.to] = start[e.to].max(e.delay);
        } else {
            constraints.push((e.to, e.from, 0));
        }
    }
    for &i in &graph.inputs {
        constraints.push((host, i, 0));
        constraints.push((i, host, 0));
    }
    Ok(Problem { vars, host, sink, mirror, constraints, weight, start, topo, out })
}

impl Problem {
    /// The period's constraints `stage` breaks: each stage walked from where it can begin, every node a path in it
    /// first passes the period at put a stage after the node the path began from, and the walk going on from that
    /// node as from the start of the stage it moves to. A path from where a stage can begin is as deep wherever the
    /// stage's register sits, before the node or further back, so each constraint holds in every pipeline; each
    /// joins two nodes of one stage of `stage`, so none the problem holds is among them.
    fn broken(&self, graph: &TimingGraph, stage: &[i64], period: i64) -> Vec<(usize, usize, i64)> {
        let n = graph.nodes.len();
        let mut depth = vec![i64::MIN; n];
        let mut began = vec![usize::MAX; n];
        let mut broken = Vec::new();
        for &v in &self.topo {
            if self.start[v] >= 0 && self.start[v] > depth[v] {
                depth[v] = self.start[v];
                began[v] = v;
            }
            if depth[v] == i64::MIN {
                continue;
            }
            if depth[v] > period {
                broken.push((began[v], v, 1));
                if self.start[v] < 0 {
                    continue;
                }
                depth[v] = self.start[v];
                began[v] = v;
            }
            for &k in &self.out[v] {
                let e = &graph.edges[k];
                let reach = depth[v] + e.delay;
                if stage[e.to] == stage[v] && reach > depth[e.to] {
                    depth[e.to] = reach;
                    began[e.to] = began[v];
                }
            }
        }
        broken
    }

    /// The earliest schedule the constraints allow, each variable's stage, every variable from stage 0 up. Every
    /// constraint is `≥ 0` or `≥ 1`, so a loop of them rises exactly when a `≥ 1` lies inside a strongly connected
    /// component, and without one the earliest schedule is one pass over the components in topological order: linear
    /// in the constraints either way.
    fn earliest(&self, graph: &TimingGraph) -> Result<Vec<i64>, PipeError> {
        let v = self.vars;
        let mut start = vec![0usize; v + 1];
        for &(a, _, _) in &self.constraints {
            start[a + 1] += 1;
        }
        for i in 0..v {
            start[i + 1] += start[i];
        }
        let mut fill = start.clone();
        let mut arcs = vec![(0usize, 0i64); self.constraints.len()];
        for &(a, b, l) in &self.constraints {
            arcs[fill[a]] = (b, l);
            fill[a] += 1;
        }
        let out = |u: usize| &arcs[start[u]..start[u + 1]];
        // Tarjan's components without recursion, emitted sinks first.
        let (mut index, mut low, mut on) = (vec![usize::MAX; v], vec![0usize; v], vec![false; v]);
        let mut comp = vec![usize::MAX; v];
        let (mut stack, mut order, mut next): (Vec<usize>, Vec<usize>, usize) = (Vec::new(), Vec::new(), 0);
        for root in 0..v {
            if index[root] != usize::MAX {
                continue;
            }
            let mut call: Vec<(usize, usize)> = vec![(root, 0)];
            index[root] = next;
            low[root] = next;
            next += 1;
            stack.push(root);
            on[root] = true;
            while let Some(top) = call.last_mut() {
                let (u, i) = *top;
                if let Some(&(w, _)) = out(u).get(i) {
                    top.1 += 1;
                    if index[w] == usize::MAX {
                        index[w] = next;
                        low[w] = next;
                        next += 1;
                        stack.push(w);
                        on[w] = true;
                        call.push((w, 0));
                    } else if on[w] {
                        low[u] = low[u].min(index[w]);
                    }
                    continue;
                }
                call.pop();
                if let Some(&(parent, _)) = call.last() {
                    low[parent] = low[parent].min(low[u]);
                }
                if low[u] == index[u] {
                    let c = order.len();
                    loop {
                        let w = stack.pop().expect("u is on the stack");
                        on[w] = false;
                        comp[w] = c;
                        if w == u {
                            break;
                        }
                    }
                    order.push(u);
                }
            }
        }
        if let Some(&(a, b, _)) = self.constraints.iter().find(|&&(a, b, l)| l > 0 && comp[a] == comp[b]) {
            // The rise from `a` to `b`, and the way back from `b` to `a` inside their component.
            let mut back = vec![usize::MAX; v];
            let mut queue: VecDeque<usize> = VecDeque::from([b]);
            back[b] = b;
            while let Some(u) = queue.pop_front() {
                if u == a {
                    break;
                }
                for &(w, _) in out(u) {
                    if comp[w] == comp[a] && back[w] == usize::MAX {
                        back[w] = u;
                        queue.push_back(w);
                    }
                }
            }
            let mut through = vec![a];
            let mut at = a;
            while at != b {
                at = back[at];
                through.push(at);
            }
            through.reverse();
            let n = graph.nodes.len();
            return Err(PipeError::Uncuttable { through: through.into_iter().filter(|&x| x < n).map(|x| graph.nodes[x].clone()).collect() });
        }
        // Components were numbered sinks first; walk them sources first.
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); order.len()];
        for u in 0..v {
            members[comp[u]].push(u);
        }
        let mut dist = vec![0i64; order.len()];
        for c in (0..order.len()).rev() {
            for &u in &members[c] {
                for &(w, l) in out(u) {
                    if comp[w] != c {
                        dist[comp[w]] = dist[comp[w]].max(dist[c] + l);
                    }
                }
            }
        }
        let base = dist[comp[self.host]];
        Ok((0..v).map(|u| dist[comp[u]] - base).collect())
    }

    /// A latest stage for each node in any pipeline at `latency`: each node, outputs first, in the latest stage its
    /// readers leave room for, a stage before them when the logic from it to their stage's end, with the deepest edge
    /// into it, passes the period. Every path through a node in its stage is at least its deepest edge in deep, past a
    /// register on that edge or through the node before it, and a register may follow any net here, so no pipeline
    /// under either policy puts a node later.
    fn latest(&self, graph: &TimingGraph, period: i64, latency: i64) -> Vec<i64> {
        let n = graph.nodes.len();
        let mut into = vec![0i64; n];
        for e in &graph.edges {
            into[e.to] = into[e.to].max(e.delay);
        }
        let (mut stage, mut depart) = (vec![latency; n], vec![0i64; n]);
        for &v in self.topo.iter().rev() {
            let Some(s) = self.out[v].iter().map(|&k| stage[graph.edges[k].to]).min() else { continue };
            let d = self.out[v].iter().map(|&k| &graph.edges[k]).filter(|e| stage[e.to] == s).map(|e| e.delay + depart[e.to]).max().unwrap_or(0);
            (stage[v], depart[v]) = if into[v] + d > period { (s - 1, 0) } else { (s, d) };
        }
        stage
    }

    /// The earliest schedule that breaks none of the period's constraints, those it would break added.
    fn settle(&mut self, graph: &TimingGraph, period: i64) -> Result<Vec<i64>, PipeError> {
        loop {
            let stage = self.earliest(graph)?;
            let broken = self.broken(graph, &stage, period);
            if broken.is_empty() {
                return Ok(stage);
            }
            self.constraints.extend(broken);
        }
    }
}

/// The least latency `graph` pipelines in at `period`: the latest any output's sink must be in the earliest schedule
/// the difference constraints allow.
pub fn min_latency(graph: &TimingGraph, period: i64, policy: Cuts) -> Result<i64, PipeError> {
    let mut p = problem(graph, period, policy)?;
    let stage = p.settle(graph, period)?;
    Ok(p.sink.iter().map(|&s| stage[s]).max().unwrap_or(0).max(0))
}

/// The pipeline of `graph` at `period` and `latency` with the fewest register bits, its registers where `policy`
/// allows.
pub fn pipeline(graph: &TimingGraph, period: i64, latency: i64, policy: Cuts) -> Result<Plan, PipeError> {
    plan(graph, period, latency, policy).map(|(plan, _)| plan)
}

/// [`pipeline`]'s plan, and the problem it solved: the period's constraints the earliest schedule breaks first, then
/// those each optimum breaks. Each node's stage is held between its earliest and its latest (Maheshwari &
/// Sapatnekar, *Efficient retiming of large circuits*, 1998), bounds every pipeline meets, so the optimum is the
/// same and logic without slack is fixed where it must be.
fn plan(graph: &TimingGraph, period: i64, latency: i64, policy: Cuts) -> Result<(Plan, Problem), PipeError> {
    if latency < 0 {
        return Err(PipeError::NegativeLatency);
    }
    let mut p = problem(graph, period, policy)?;
    let earliest = p.settle(graph, period)?;
    let minimum = p.sink.iter().map(|&s| earliest[s]).max().unwrap_or(0).max(0);
    if latency < minimum {
        return Err(PipeError::LatencyTooShort { minimum });
    }
    let latest = p.latest(graph, period, latency);
    let supply: Vec<i64> = p.weight.iter().map(|&w| -w).collect();
    let n = graph.nodes.len();
    loop {
        let mut arcs: Vec<Arc> = p.constraints.iter().map(|&(a, b, l)| Arc { from: a, to: b, capacity: None, cost: -l }).collect();
        for &s in &p.sink {
            arcs.push(Arc { from: p.host, to: s, capacity: None, cost: -latency });
            arcs.push(Arc { from: s, to: p.host, capacity: None, cost: latency });
        }
        for v in 0..n {
            arcs.push(Arc { from: p.host, to: v, capacity: None, cost: -earliest[v] });
            arcs.push(Arc { from: v, to: p.host, capacity: None, cost: latest[v] });
        }
        let potential = match min_cost_flow(p.vars, &supply, &arcs).map_err(PipeError::Flow)? {
            Solution::Optimal { potential, .. } => potential,
            Solution::Unbounded { .. } => unreachable!("the earliest schedule meets every constraint and bound at the latency or less"),
            Solution::Infeasible { .. } => unreachable!("every register count is a mirror's stage less its net's, never below 0"),
        };
        let host = -potential[p.host];
        let stage_of = |v: usize| -> i64 { (-potential[v] - host) as i64 };
        let stage: Vec<i64> = (0..n).map(stage_of).collect();
        let broken = p.broken(graph, &stage, period);
        if !broken.is_empty() {
            p.constraints.extend(broken);
            continue;
        }
        let mut registers = BTreeMap::new();
        for v in 0..n {
            let Some(m) = p.mirror[v] else { continue };
            let count = stage_of(m) - stage[v];
            if count > 0 {
                registers.insert(v, count);
            }
        }
        let bits = registers.values().sum();
        return Ok((Plan { period, latency, policy, stage, registers, bits }, p));
    }
}

/// Whether `plan` pipelines `graph`: every edge's register count is the difference of its ends' stages, never below
/// zero and zero out of a node its policy puts no register after; every input at stage 0 and every output with the registers to reach the
/// latency; every named net's registers the most its readers and the latency need; and no path between registers
/// deeper than the period.
pub fn check(graph: &TimingGraph, plan: &Plan) -> bool {
    let n = graph.nodes.len();
    if plan.stage.len() != n || graph.flops > 0 || graph.edges.iter().any(|e| e.registers != 0) {
        return false;
    }
    let cut = cuts(graph, plan.policy);
    if graph.inputs.iter().any(|&i| plan.stage[i] != 0) {
        return false;
    }
    let mut needs: Vec<i64> = vec![0; n];
    for e in &graph.edges {
        let r = plan.stage[e.to] - plan.stage[e.from];
        if r < 0 || (!cut[e.from] && r != 0) {
            return false;
        }
        needs[e.from] = needs[e.from].max(r);
    }
    for &o in &graph.outputs {
        let r = plan.latency - plan.stage[o];
        if r < 0 {
            return false;
        }
        needs[o] = needs[o].max(r);
    }
    for v in 0..n {
        if plan.registers.get(&v).copied().unwrap_or(0) != needs[v] {
            return false;
        }
    }
    if plan.registers.values().sum::<i64>() != plan.bits {
        return false;
    }
    // A register sits where its edge starts, so the edge's delay belongs to the stage after it.
    let mut piped = TimingGraph { edges: Vec::with_capacity(graph.edges.len()), costs: Vec::new(), owner: Vec::new(), fanout: None, ..graph.clone() };
    for (k, e) in graph.edges.iter().enumerate() {
        let r = plan.stage[e.to] - plan.stage[e.from];
        if r == 0 {
            piped.edges.push(*e);
        } else {
            let q = piped.nodes.len();
            piped.nodes.push(format!("{}~piped{k}", graph.nodes[e.from]));
            piped.public.push(false);
            piped.edges.push(Edge { from: e.from, to: q, delay: 0, registers: r });
            piped.edges.push(Edge { from: q, to: e.to, delay: e.delay, registers: 0 });
        }
    }
    piped.owner = vec![0; piped.edges.len()];
    piped.depth().is_ok_and(|d| d.depth <= plan.period)
}

/// Why a plan could not be written out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmitError {
    /// The graph was not read from a netlist, so nothing says how its nodes are made.
    NoStructure,
    /// A kept cell of a kind written out here has no form (floor division).
    Unsupported { cell: String, kind: String },
    /// A node a connection reads has no source to write.
    Undriven { node: String },
    /// A port the pipeline adds, its clock or its enable, is one the logic reads.
    Control { port: String },
}

impl std::fmt::Display for EmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmitError::NoStructure => write!(f, "the graph has no netlist structure to write"),
            EmitError::Unsupported { cell, kind } => write!(f, "cell {cell} is a {kind}, which is not written out"),
            EmitError::Undriven { node } => write!(f, "{node} is read but nothing drives it"),
            EmitError::Control { port } => write!(f, "{port} is read by the logic, so it cannot also be the pipeline's own"),
        }
    }
}

impl std::error::Error for EmitError {}

/// A Verilog identifier for a port name, escaped when it is not a plain one.
fn ident(name: &str) -> String {
    let plain = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$');
    if plain { name.to_string() } else { format!("\\{name} ") }
}

struct Writer<'a> {
    plan: &'a Plan,
    deepest: BTreeMap<usize, i64>,
    body: Vec<String>,
    /// Every node the text reads.
    read: BTreeSet<usize>,
}

impl Writer<'_> {
    /// `node`'s value `depth` registers late.
    fn tap(&mut self, node: usize, depth: i64) -> String {
        self.read.insert(node);
        if depth > 0 {
            let d = self.deepest.entry(node).or_insert(0);
            *d = (*d).max(depth);
            format!("w{node}_d{depth}")
        } else {
            format!("w{node}")
        }
    }

    /// A pin read at `stage`.
    fn pin(&mut self, pin: Pin, stage: i64) -> String {
        match pin {
            Pin::Node(n) => {
                let d = stage - self.plan.stage[n];
                self.tap(n, d)
            }
            Pin::Zero => "1'b0".into(),
            Pin::One => "1'b1".into(),
        }
    }

    /// An operand: its pins read at `stage`, most significant first, at its own width, signed when the cell's is;
    /// Verilog's sizing then extends it as the cell's semantics do.
    fn operand(&mut self, pins: &[Pin], stage: i64, signed: bool) -> String {
        let bits: Vec<String> = pins.iter().rev().map(|&p| self.pin(p, stage)).collect();
        let v = format!("{{{}}}", bits.join(", "));
        if signed { format!("$signed({v})") } else { v }
    }
}

/// `graph`'s logic pipelined by `plan` as a Verilog module named `module`: every connection reads its source through
/// as many registers as the plan puts on it, clocked by `clk` and, when `enable` names a port, stepping only when it
/// is high. An input of the source with one of those names that no logic reads is that port; one the logic reads is
/// refused.
pub fn emit(graph: &TimingGraph, plan: &Plan, module: &str, enable: Option<&str>) -> Result<String, EmitError> {
    let s = graph.structure.as_ref().ok_or(EmitError::NoStructure)?;
    // A source input named as a port the pipeline adds is that port when no logic reads it.
    let controls: Vec<&str> = std::iter::once("clk").chain(enable).collect();
    let mut read = vec![false; graph.nodes.len()];
    for e in &graph.edges {
        read[e.from] = true;
    }
    for &o in &graph.outputs {
        read[o] = true;
    }
    let mut control_nodes = BTreeSet::new();
    for (port, pins) in s.input_ports.iter().filter(|(port, _)| controls.contains(&port.as_str())) {
        for &p in pins {
            if let Pin::Node(n) = p {
                if read[n] {
                    return Err(EmitError::Control { port: port.clone() });
                }
                control_nodes.insert(n);
            }
        }
    }
    let mut w = Writer { plan, deepest: BTreeMap::new(), body: Vec::new(), read: BTreeSet::new() };
    for (i, cell) in s.cells.iter().enumerate() {
        let stage = plan.stage[cell.inside];
        let p = |k: &str| cell.ports.get(k).cloned().unwrap_or_default();
        let num = |k: &str| cell.number(k);
        let (a, b, y) = (p("A"), p("B"), p("Y"));
        let (sa, sb) = (num("A_SIGNED") == 1, num("B_SIGNED") == 1);
        let yw = y.len();
        let (x, z) = (w.operand(&a, stage, sa && sb), w.operand(&b, stage, sa && sb));
        let expr = match cell.kind.as_str() {
            "$add" => format!("{x} + {z}"),
            "$sub" => format!("{x} - {z}"),
            "$mul" => format!("{x} * {z}"),
            "$neg" => format!("-{}", w.operand(&a, stage, sa)),
            "$lt" => format!("{x} < {z}"),
            "$le" => format!("{x} <= {z}"),
            "$gt" => format!("{x} > {z}"),
            "$ge" => format!("{x} >= {z}"),
            "$div" => format!("{x} / {z}"),
            "$mod" => format!("{x} % {z}"),
            kind => return Err(EmitError::Unsupported { cell: cell.name.clone(), kind: kind.to_string() }),
        };
        w.body.push(format!("  wire [{}:0] c{i}_Y = {expr};", yw.max(1) - 1));
    }
    for n in 0..graph.nodes.len() {
        let stage = plan.stage[n];
        let line = match &s.source[n] {
            Source::Input { .. } if control_nodes.contains(&n) => continue,
            Source::Input { port, bit } => format!("  assign w{n} = {}[{bit}];", ident(port)),
            Source::Lut { pins, cone: Some(cone), .. } => {
                let read: Vec<String> = pins.iter().map(|&p| w.pin(p, stage)).collect();
                let arg = |a: ConeArg| {
                    let name = if a.index < read.len() { read[a.index].clone() } else { format!("w{n}_g{}", a.index - read.len()) };
                    if a.inverted { format!("~{name}") } else { name }
                };
                let mut lines: Vec<String> = cone
                    .gates
                    .iter()
                    .enumerate()
                    .map(|(i, g)| format!("  wire w{n}_g{i} = {} {} {};", arg(g.a), if g.xor { "^" } else { "&" }, arg(g.b)))
                    .collect();
                lines.push(format!("  assign w{n} = {};", arg(cone.out)));
                lines.join("\n")
            }
            Source::Lut { pins, table, .. } if pins.is_empty() => format!("  assign w{n} = 1'b{};", table & 1),
            Source::Lut { pins, table, .. } => {
                let read: Vec<String> = pins.iter().rev().map(|&p| w.pin(p, stage)).collect();
                let width = 1usize << pins.len();
                format!("  assign w{n} = {width}'h{table:x} >> {{{}}};", read.join(", "))
            }
            Source::Copy { from, inverted } => {
                let src = w.pin(Pin::Node(*from), stage);
                format!("  assign w{n} = {}{src};", if *inverted { "~" } else { "" })
            }
            Source::Cell { cell, bit, .. } => format!("  assign w{n} = c{cell}_Y[{bit}];"),
            Source::Constant(v) => format!("  assign w{n} = 1'b{};", *v as u8),
            Source::Register { .. } | Source::Inside => continue,
        };
        w.body.push(line);
    }
    let mut outs = Vec::new();
    for (port, pins) in &s.output_ports {
        for (bit, &p) in pins.iter().enumerate() {
            let src = match p {
                Pin::Node(n) => w.tap(n, plan.latency - plan.stage[n]),
                Pin::Zero => "1'b0".into(),
                Pin::One => "1'b1".into(),
            };
            outs.push(format!("  assign {}[{bit}] = {src};", ident(port)));
        }
    }
    // Every node the text reads is one it writes: never a register's output or a point nothing here drives.
    if let Some(&n) = w.read.iter().find(|&&n| matches!(s.source[n], Source::Register { .. } | Source::Inside)) {
        return Err(EmitError::Undriven { node: graph.nodes[n].clone() });
    }
    let mut v = String::new();
    let mut header = vec!["input clk".to_string()];
    if let Some(en) = enable {
        header.push(format!("input {}", ident(en)));
    }
    for (port, pins) in s.input_ports.iter().filter(|(port, _)| !controls.contains(&port.as_str())) {
        header.push(format!("input [{}:0] {}", pins.len().max(1) - 1, ident(port)));
    }
    for (port, pins) in &s.output_ports {
        header.push(format!("output [{}:0] {}", pins.len().max(1) - 1, ident(port)));
    }
    v.push_str(&format!("module {}(\n  {}\n);\n", ident(module), header.join(",\n  ")));
    for n in 0..graph.nodes.len() {
        if !matches!(s.source[n], Source::Register { .. } | Source::Inside) && !control_nodes.contains(&n) {
            v.push_str(&format!("  wire w{n};\n"));
        }
    }
    for (&n, &d) in &w.deepest {
        for k in 1..=d {
            v.push_str(&format!("  reg w{n}_d{k};\n"));
        }
    }
    for line in &w.body {
        v.push_str(line);
        v.push('\n');
    }
    for line in &outs {
        v.push_str(line);
        v.push('\n');
    }
    if !w.deepest.is_empty() {
        let step = match enable { Some(en) => format!("if ({}) begin", ident(en)), None => "begin".into() };
        v.push_str(&format!("  always @(posedge clk) {step}\n"));
        for (&n, &d) in &w.deepest {
            for k in 1..=d {
                let prev = if k == 1 { format!("w{n}") } else { format!("w{n}_d{}", k - 1) };
                v.push_str(&format!("    w{n}_d{k} <= {prev};\n"));
            }
        }
        v.push_str("  end\n");
    }
    v.push_str("endmodule\n");
    Ok(v)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/autopipe/tests.rs"]
mod tests;
