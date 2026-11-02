//! The maximum cycle ratio of a weighted digraph, exact and certified: the bound a loop puts on a clock period.
//!
//! In a circuit's retiming graph (Leiserson & Saxe, *Retiming synchronous circuitry*, 1991) an edge carries the
//! combinational delay of its source and the registers on it. A cycle of total delay `D` through `R` registers
//! cannot run faster than `D / R` a clock, however the registers are moved, because retiming keeps every cycle's
//! register count; so the largest `D / R` over every cycle is the period floor no retiming or pipelining of the
//! loop's own logic can beat, and the cycle reaching it names the loop to re-architect. A cycle of no register is a
//! combinational loop and has no period at all.
//!
//! [`max_cycle_ratio`] splits the graph into strongly connected components, since every cycle lies in one, and runs
//! Howard's policy iteration on each (Cochet-Terrasson, Cohen, Gaubert, Mc Gettrick & Quadrat, *Numerical
//! computation of spectral elements in max-plus algebra*, 1998), the fastest method in practice (Dasdan,
//! *Experimental analysis of the fastest optimum cycle ratio and mean algorithms*, 2004). A policy picks one edge out
//! of every node; its cycles have exact ratios `p/q` in lowest terms, and each node takes the ratio of the cycle its
//! policy path reaches and a bias `x` with `x(u) = q·delay − p·registers + x(v)` along its edge. A node switches to
//! an edge reaching a larger ratio, or, when none does, to an edge of the same ratio and a larger
//! `q·delay − p·registers + x(v)`; when no node can switch, every node of the component has the component's largest
//! ratio. All of it is integer arithmetic.
//!
//! The largest ratio `D/R` over the components is the bound, and a longest-path pass over the integer weights
//! `R·delay − D·registers`, which no cycle makes positive, gives a potential `π` with
//! `π(to) ≥ π(from) + R·delay − D·registers` on every edge. Summed around any cycle that says no cycle's ratio
//! exceeds `D/R`, and the cycle found reaches it: the [`Bound::Ratio`] is that cycle and that potential, and [`check`]
//! replays both in exact integer arithmetic, trusting nothing the search did.
//!
//! Every delay and register count is below [`LIMIT`] and a graph has at most [`NODES`] nodes, so every sum and
//! product the search and the checker form fits `i128` exactly.

use std::cmp::Ordering;
use std::collections::VecDeque;

/// The bound on an edge's delay and register count.
pub const LIMIT: i64 = 1 << 24;

/// The bound on the node count.
pub const NODES: usize = 1 << 31;

/// An edge of the graph: `delay` of combinational logic and `registers` on it, both counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub delay: i64,
    pub registers: i64,
}

/// What bounds the period.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bound {
    /// No cycle: nothing bounds the period but the longest path, which retiming can cut.
    Acyclic,
    /// A cycle through no register, its edges in order.
    CombinationalLoop(Vec<usize>),
    /// The largest `delay / registers` over every cycle: the sums of the cycle reaching it, its edges in order, and a
    /// potential over the nodes under which no cycle exceeds it.
    Ratio { delay: i64, registers: i64, cycle: Vec<usize>, potential: Vec<i128> },
}

impl Bound {
    /// The bound as a fraction in lowest terms, `delay / registers`.
    pub fn period(&self) -> Option<(i64, i64)> {
        match self {
            Bound::Ratio { delay, registers, .. } => {
                let g = gcd(*delay, *registers).max(1);
                Some((delay / g, registers / g))
            }
            _ => None,
        }
    }
}

fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.abs()
}

/// Why a graph has no bound computed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphError {
    /// An edge names a node past the count.
    OutOfRange { edge: usize },
    /// An edge's delay or register count is negative.
    Negative { edge: usize },
    /// An edge's delay or register count is [`LIMIT`] or more.
    TooLarge { edge: usize },
    /// More than [`NODES`] nodes.
    TooManyNodes,
}

fn validate(nodes: usize, edges: &[Edge]) -> Result<(), GraphError> {
    if nodes > NODES {
        return Err(GraphError::TooManyNodes);
    }
    for (k, e) in edges.iter().enumerate() {
        if e.from >= nodes || e.to >= nodes {
            return Err(GraphError::OutOfRange { edge: k });
        }
        if e.delay < 0 || e.registers < 0 {
            return Err(GraphError::Negative { edge: k });
        }
        if e.delay >= LIMIT || e.registers >= LIMIT {
            return Err(GraphError::TooLarge { edge: k });
        }
    }
    Ok(())
}

/// The edges out of every node, in edge order.
struct Out {
    start: Vec<usize>,
    list: Vec<usize>,
}

impl Out {
    fn new(nodes: usize, edges: &[Edge]) -> Out {
        let mut start = vec![0usize; nodes + 1];
        for e in edges {
            start[e.from + 1] += 1;
        }
        for n in 0..nodes {
            start[n + 1] += start[n];
        }
        let mut fill = start.clone();
        let mut list = vec![0usize; edges.len()];
        for (k, e) in edges.iter().enumerate() {
            list[fill[e.from]] = k;
            fill[e.from] += 1;
        }
        Out { start, list }
    }

    fn of(&self, n: usize) -> &[usize] {
        &self.list[self.start[n]..self.start[n + 1]]
    }
}

/// A cycle of the subgraph `keep` admits, its edges in order, by depth-first search.
fn find_cycle(nodes: usize, edges: &[Edge], keep: impl Fn(&Edge) -> bool) -> Option<Vec<usize>> {
    let out = Out::new(nodes, edges);
    // 0 unvisited, 1 on the stack, 2 done.
    let mut state = vec![0u8; nodes];
    let mut via: Vec<usize> = vec![usize::MAX; nodes];
    for root in 0..nodes {
        if state[root] != 0 {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        state[root] = 1;
        while let Some(top) = stack.last_mut() {
            let (n, i) = *top;
            let Some(&k) = out.of(n).get(i) else {
                state[n] = 2;
                stack.pop();
                continue;
            };
            top.1 += 1;
            if !keep(&edges[k]) {
                continue;
            }
            let t = edges[k].to;
            match state[t] {
                0 => {
                    state[t] = 1;
                    via[t] = k;
                    stack.push((t, 0));
                }
                1 => {
                    let mut cycle = vec![k];
                    let mut at = n;
                    while at != t {
                        let e = via[at];
                        cycle.push(e);
                        at = edges[e].from;
                    }
                    cycle.reverse();
                    return Some(cycle);
                }
                _ => {}
            }
        }
    }
    None
}

/// The strongly connected components, sources first, by Tarjan's algorithm without recursion; and each node's
/// component.
fn components(nodes: usize, edges: &[Edge], out: &Out) -> (Vec<Vec<usize>>, Vec<usize>) {
    let mut index = vec![usize::MAX; nodes];
    let mut low = vec![0usize; nodes];
    let mut on = vec![false; nodes];
    let mut stack: Vec<usize> = Vec::new();
    let mut comps: Vec<Vec<usize>> = Vec::new();
    let mut next = 0;
    for root in 0..nodes {
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
            let (v, i) = *top;
            if let Some(&k) = out.of(v).get(i) {
                top.1 += 1;
                let w = edges[k].to;
                if index[w] == usize::MAX {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on[w] = true;
                    call.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            call.pop();
            if let Some(&(u, _)) = call.last() {
                low[u] = low[u].min(low[v]);
            }
            if low[v] == index[v] {
                let mut members = Vec::new();
                loop {
                    let w = stack.pop().expect("v is on the stack");
                    on[w] = false;
                    members.push(w);
                    if w == v {
                        break;
                    }
                }
                comps.push(members);
            }
        }
    }
    comps.reverse();
    let mut comp = vec![0usize; nodes];
    for (c, members) in comps.iter().enumerate() {
        for &v in members {
            comp[v] = c;
        }
    }
    (comps, comp)
}

/// A ratio `p/q` in lowest terms, `q > 0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ratio {
    p: i64,
    q: i64,
}

impl Ratio {
    fn new(p: i64, q: i64) -> Ratio {
        let g = gcd(p, q).max(1);
        Ratio { p: p / g, q: q / g }
    }

    fn weight(self, e: &Edge) -> i128 {
        self.q as i128 * e.delay as i128 - self.p as i128 * e.registers as i128
    }
}

impl Ord for Ratio {
    fn cmp(&self, other: &Ratio) -> Ordering {
        (self.p as i128 * other.q as i128).cmp(&(other.p as i128 * self.q as i128))
    }
}

impl PartialOrd for Ratio {
    fn partial_cmp(&self, other: &Ratio) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn sums(edges: &[Edge], cycle: &[usize]) -> (i64, i64) {
    cycle.iter().fold((0, 0), |(d, r), &k| (d + edges[k].delay, r + edges[k].registers))
}

/// Howard's policy iteration on component `c`, whose every cycle holds a register: its largest ratio and a cycle
/// reaching it.
fn howard(edges: &[Edge], out: &Out, members: &[usize], comp: &[usize], c: usize) -> (Ratio, Vec<usize>) {
    let inside = |k: &&usize| comp[edges[**k].to] == c;
    let mut policy: Vec<usize> = vec![usize::MAX; comp.len()];
    for &v in members {
        policy[v] = *out.of(v).iter().filter(inside).max_by_key(|&&k| edges[k].delay).expect("a component's node has an edge in it");
    }
    let mut ratio: Vec<Ratio> = vec![Ratio { p: 0, q: 1 }; comp.len()];
    let mut bias: Vec<i128> = vec![0; comp.len()];
    let mut state: Vec<u8> = vec![0; comp.len()];
    let mut path: Vec<usize> = Vec::new();
    loop {
        // Value determination: 0 unseen, 1 on the walk, 2 valued.
        for &v in members {
            state[v] = 0;
        }
        let mut found: Option<(Ratio, Vec<usize>)> = None;
        for &start in members {
            if state[start] != 0 {
                continue;
            }
            path.clear();
            let mut at = start;
            while state[at] == 0 {
                state[at] = 1;
                path.push(at);
                at = edges[policy[at]].to;
            }
            let mut tail = path.len();
            if state[at] == 1 {
                let pos = path.iter().position(|&n| n == at).expect("the walk met itself");
                let cycle = &path[pos..];
                let edges_of: Vec<usize> = cycle.iter().map(|&n| policy[n]).collect();
                let (d, r) = sums(edges, &edges_of);
                let lambda = Ratio::new(d, r);
                let root = (0..cycle.len()).min_by_key(|&i| cycle[i]).expect("a cycle has a node");
                bias[cycle[root]] = 0;
                ratio[cycle[root]] = lambda;
                state[cycle[root]] = 2;
                for k in 1..cycle.len() {
                    let i = (root + cycle.len() - k) % cycle.len();
                    let u = cycle[i];
                    let t = cycle[(i + 1) % cycle.len()];
                    bias[u] = lambda.weight(&edges[policy[u]]) + bias[t];
                    ratio[u] = lambda;
                    state[u] = 2;
                }
                if found.as_ref().map_or(true, |(best, _)| lambda > *best) {
                    found = Some((lambda, edges_of));
                }
                tail = pos;
            }
            for &u in path[..tail].iter().rev() {
                let t = edges[policy[u]].to;
                ratio[u] = ratio[t];
                bias[u] = ratio[u].weight(&edges[policy[u]]) + bias[t];
                state[u] = 2;
            }
        }
        // Improvement: a larger ratio first; else, at the same ratio, a larger bias.
        let mut changed = false;
        for &u in members {
            let mut best = ratio[u];
            for &k in out.of(u).iter().filter(inside) {
                let r = ratio[edges[k].to];
                if r > best {
                    best = r;
                    policy[u] = k;
                    changed = true;
                }
            }
        }
        if !changed {
            for &u in members {
                let mut best = bias[u];
                for &k in out.of(u).iter().filter(inside) {
                    let t = edges[k].to;
                    if ratio[t] != ratio[u] {
                        continue;
                    }
                    let value = ratio[u].weight(&edges[k]) + bias[t];
                    if value > best {
                        best = value;
                        policy[u] = k;
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            return found.expect("a component with an edge has a policy cycle");
        }
    }
}

/// Longest paths from a virtual source to every node under the weights `lambda.q·delay − lambda.p·registers`, which
/// no cycle makes positive, one component at a time in topological order.
fn potential(nodes: usize, edges: &[Edge], out: &Out, comps: &[Vec<usize>], comp: &[usize], lambda: Ratio) -> Vec<i128> {
    let mut dist = vec![0i128; nodes];
    let mut queued = vec![false; nodes];
    let mut queue: VecDeque<usize> = VecDeque::new();
    for (c, members) in comps.iter().enumerate() {
        queue.extend(members.iter().copied());
        for &v in members {
            queued[v] = true;
        }
        while let Some(u) = queue.pop_front() {
            queued[u] = false;
            for &k in out.of(u) {
                let e = &edges[k];
                if comp[e.to] != c {
                    continue;
                }
                let d = dist[u] + lambda.weight(e);
                if d > dist[e.to] {
                    dist[e.to] = d;
                    if !queued[e.to] {
                        queued[e.to] = true;
                        queue.push_back(e.to);
                    }
                }
            }
        }
        for &u in members {
            for &k in out.of(u) {
                let e = &edges[k];
                if comp[e.to] != c {
                    dist[e.to] = dist[e.to].max(dist[u] + lambda.weight(e));
                }
            }
        }
    }
    dist
}

/// The bound on the period the graph's cycles put.
pub fn max_cycle_ratio(nodes: usize, edges: &[Edge]) -> Result<Bound, GraphError> {
    validate(nodes, edges)?;
    if let Some(cycle) = find_cycle(nodes, edges, |e| e.registers == 0) {
        return Ok(Bound::CombinationalLoop(cycle));
    }
    let out = Out::new(nodes, edges);
    let (comps, comp) = components(nodes, edges, &out);
    let mut best: Option<(Ratio, Vec<usize>)> = None;
    for (c, members) in comps.iter().enumerate() {
        let cyclic = members.len() > 1 || out.of(members[0]).iter().any(|&k| edges[k].to == members[0]);
        if !cyclic {
            continue;
        }
        let (lambda, cycle) = howard(edges, &out, members, &comp, c);
        if best.as_ref().map_or(true, |(b, _)| lambda > *b) {
            best = Some((lambda, cycle));
        }
    }
    let Some((lambda, cycle)) = best else {
        return Ok(Bound::Acyclic);
    };
    let (delay, registers) = sums(edges, &cycle);
    let scale = (registers / lambda.q) as i128;
    let potential = potential(nodes, edges, &out, &comps, &comp, lambda).into_iter().map(|d| d * scale).collect();
    Ok(Bound::Ratio { delay, registers, cycle, potential })
}

/// Whether `bound` is the graph's: a combinational loop is a simple cycle of no register; a ratio's cycle is a simple
/// cycle with those sums and some register, and its potential satisfies every edge; acyclic means the graph has no
/// cycle. The graph itself must be well formed.
pub fn check(nodes: usize, edges: &[Edge], bound: &Bound) -> bool {
    if validate(nodes, edges).is_err() {
        return false;
    }
    let is_cycle = |cycle: &[usize]| {
        let mut seen = vec![false; nodes];
        !cycle.is_empty()
            && cycle.iter().all(|&k| k < edges.len() && !std::mem::replace(&mut seen[edges[k].from], true))
            && cycle.iter().enumerate().all(|(i, &k)| edges[k].to == edges[cycle[(i + 1) % cycle.len()]].from)
    };
    match bound {
        Bound::Acyclic => find_cycle(nodes, edges, |_| true).is_none(),
        Bound::CombinationalLoop(cycle) => is_cycle(cycle) && cycle.iter().all(|&k| edges[k].registers == 0),
        Bound::Ratio { delay, registers, cycle, potential } => {
            if !is_cycle(cycle) || sums(edges, cycle) != (*delay, *registers) || *registers <= 0 || potential.len() != nodes {
                return false;
            }
            edges.iter().all(|e| {
                let w = *registers as i128 * e.delay as i128 - *delay as i128 * e.registers as i128;
                match potential[e.from].checked_add(w) {
                    Some(reach) => potential[e.to] >= reach,
                    None => w < 0,
                }
            })
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/cycle_ratio/tests.rs"]
mod tests;
