//! Minimum-cost flow, exact and certified: the dual of every scheduling and retiming problem whose constraints are
//! differences of two variables, which is how a register cut of least area at a target clock is found.
//!
//! [`min_cost_flow`] runs the primal network simplex on a strongly feasible spanning tree (Cunningham, *A network
//! simplex method*, 1976): an artificial root joined to every node, block-search pricing, and the tree kept as a
//! thread list so a pivot touches only the subtree it moves (the layout of LEMON's `NetworkSimplex`, Kovács,
//! *Minimum-cost flow algorithms: an experimental evaluation*, 2015). Integer data give an integer optimum.
//!
//! Every answer carries what proves it, and [`check`] replays it trusting nothing the search did:
//! - an optimum carries its flow and a potential `π` under which every arc below capacity has reduced cost
//!   `cost + π(from) − π(to) ≥ 0` and every arc with flow has it `≤ 0`, which is optimality by complementary
//!   slackness;
//! - an infeasible problem carries a node set whose supply exceeds the capacity leaving it;
//! - one without an optimum carries a cycle of uncapacitated arcs of negative cost.

/// The bound on an arc's cost.
pub const COST_LIMIT: i64 = 1 << 32;

/// The bound on an arc's capacity and on the supplies' total size.
pub const FLOW_LIMIT: i64 = 1 << 62;

/// An arc: `capacity` units at most (`None` for no bound), at `cost` a unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arc {
    pub from: usize,
    pub to: usize,
    pub capacity: Option<i64>,
    pub cost: i64,
}

/// A problem's answer, with its certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Solution {
    Optimal { flow: Vec<i64>, potential: Vec<i128>, cost: i128 },
    /// Nodes whose supply is more than the arcs leaving them can carry.
    Infeasible { cut: Vec<usize> },
    /// No optimum: uncapacitated arcs, in order, around a cycle of negative cost, around which any flow meeting the
    /// supplies gets cheaper without end.
    Unbounded { cycle: Vec<usize> },
}

/// Why a problem was not solved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowError {
    /// An arc names a node past the count.
    OutOfRange { arc: usize },
    /// An arc's capacity is negative or past [`FLOW_LIMIT`], or its cost past [`COST_LIMIT`].
    OutOfBounds { arc: usize },
    /// The supplies do not sum to zero, or their sizes sum past [`FLOW_LIMIT`].
    Unbalanced,
    /// More than 2³¹ nodes or arcs.
    TooLarge,
}

fn validate(nodes: usize, supply: &[i64], arcs: &[Arc]) -> Result<(), FlowError> {
    if nodes >= 1 << 31 || arcs.len() >= 1 << 31 || supply.len() != nodes {
        return Err(if supply.len() != nodes { FlowError::Unbalanced } else { FlowError::TooLarge });
    }
    for (k, a) in arcs.iter().enumerate() {
        if a.from >= nodes || a.to >= nodes {
            return Err(FlowError::OutOfRange { arc: k });
        }
        if a.capacity.is_some_and(|c| !(0..FLOW_LIMIT).contains(&c)) || a.cost.abs() >= COST_LIMIT {
            return Err(FlowError::OutOfBounds { arc: k });
        }
    }
    let size: i128 = supply.iter().map(|&s| (s as i128).abs()).sum();
    if supply.iter().map(|&s| s as i128).sum::<i128>() != 0 || size >= FLOW_LIMIT as i128 {
        return Err(FlowError::Unbalanced);
    }
    Ok(())
}

const INF: i64 = i64::MAX;
const UP: i8 = 1;
const DOWN: i8 = -1;
const TREE: i8 = 0;
const LOWER: i8 = 1;
const UPPER: i8 = -1;

/// The simplex's state: arcs `0..m` the problem's, `m..m + n` the artificial ones, node `n` the root.
struct Simplex {
    m: usize,
    source: Vec<usize>,
    target: Vec<usize>,
    cap: Vec<i64>,
    cost: Vec<i128>,
    flow: Vec<i64>,
    state: Vec<i8>,
    pi: Vec<i128>,
    parent: Vec<usize>,
    pred: Vec<usize>,
    pred_dir: Vec<i8>,
    thread: Vec<usize>,
    rev_thread: Vec<usize>,
    succ_num: Vec<usize>,
    last_succ: Vec<usize>,
    dirty: Vec<usize>,
    next_arc: usize,
    block: usize,
    in_arc: usize,
    join: usize,
    u_in: usize,
    v_in: usize,
    u_out: usize,
    delta: i64,
}

const NONE: usize = usize::MAX;

impl Simplex {
    fn new(nodes: usize, supply: &[i64], arcs: &[Arc]) -> Simplex {
        let (n, m) = (nodes, arcs.len());
        let all = m + n;
        let max_cost = arcs.iter().map(|a| (a.cost as i128).abs()).max().unwrap_or(0);
        let art = (max_cost + 1) * (n as i128 + 1);
        let mut s = Simplex {
            m,
            source: Vec::with_capacity(all),
            target: Vec::with_capacity(all),
            cap: Vec::with_capacity(all),
            cost: Vec::with_capacity(all),
            flow: vec![0; all],
            state: vec![LOWER; all],
            pi: vec![0; n + 1],
            parent: vec![NONE; n + 1],
            pred: vec![NONE; n + 1],
            pred_dir: vec![0; n + 1],
            thread: vec![0; n + 1],
            rev_thread: vec![0; n + 1],
            succ_num: vec![1; n + 1],
            last_succ: vec![0; n + 1],
            dirty: Vec::new(),
            next_arc: 0,
            block: ((m as f64).sqrt() as usize).max(10),
            in_arc: 0,
            join: 0,
            u_in: 0,
            v_in: 0,
            u_out: 0,
            delta: 0,
        };
        for a in arcs {
            s.source.push(a.from);
            s.target.push(a.to);
            s.cap.push(a.capacity.unwrap_or(INF));
            s.cost.push(a.cost as i128);
        }
        let root = n;
        s.thread[root] = 0;
        s.rev_thread[0] = root;
        s.succ_num[root] = n + 1;
        s.last_succ[root] = if n == 0 { root } else { root - 1 };
        if n == 0 {
            s.thread[root] = root;
            s.rev_thread[root] = root;
        }
        for u in 0..n {
            let e = m + u;
            s.parent[u] = root;
            s.pred[u] = e;
            s.thread[u] = u + 1;
            s.rev_thread[u + 1] = u;
            s.last_succ[u] = u;
            s.cap.push(INF);
            s.state[e] = TREE;
            if supply[u] >= 0 {
                s.pred_dir[u] = UP;
                s.source.push(u);
                s.target.push(root);
                s.flow[e] = supply[u];
                s.cost.push(0);
                s.pi[u] = 0;
            } else {
                s.pred_dir[u] = DOWN;
                s.source.push(root);
                s.target.push(u);
                s.flow[e] = -supply[u];
                s.cost.push(art);
                s.pi[u] = art;
            }
        }
        s
    }

    fn reduced(&self, e: usize) -> i128 {
        self.cost[e] + self.pi[self.source[e]] - self.pi[self.target[e]]
    }

    /// The arc of the most negative violation in the next block that has one.
    fn entering(&mut self) -> bool {
        let m = self.m;
        if m == 0 {
            return false;
        }
        let mut best: i128 = 0;
        let mut count = self.block;
        let mut e = self.next_arc;
        for _ in 0..m {
            let c = self.state[e] as i128 * self.reduced(e);
            if c < best {
                best = c;
                self.in_arc = e;
            }
            e = if e + 1 == m { 0 } else { e + 1 };
            count -= 1;
            if count == 0 {
                if best < 0 {
                    break;
                }
                count = self.block;
            }
        }
        self.next_arc = e;
        best < 0
    }

    fn join_node(&mut self) {
        let (mut u, mut v) = (self.source[self.in_arc], self.target[self.in_arc]);
        while u != v {
            if self.succ_num[u] < self.succ_num[v] {
                u = self.parent[u];
            } else {
                v = self.parent[v];
            }
        }
        self.join = u;
    }

    /// The arc leaving the tree: the first bottleneck on the cycle's way back to the join from the side the
    /// entering arc starts, else the last on the other side, so the tree stays strongly feasible. Whether one does.
    fn leaving(&mut self) -> bool {
        let e = self.in_arc;
        let (first, second) = if self.state[e] == LOWER { (self.source[e], self.target[e]) } else { (self.target[e], self.source[e]) };
        self.delta = self.cap[e];
        let mut result = 0;
        let mut u = first;
        while u != self.join {
            let a = self.pred[u];
            let d = if self.pred_dir[u] == DOWN { if self.cap[a] == INF { INF } else { self.cap[a] - self.flow[a] } } else { self.flow[a] };
            if d < self.delta {
                self.delta = d;
                self.u_out = u;
                result = 1;
            }
            u = self.parent[u];
        }
        let mut u = second;
        while u != self.join {
            let a = self.pred[u];
            let d = if self.pred_dir[u] == UP { if self.cap[a] == INF { INF } else { self.cap[a] - self.flow[a] } } else { self.flow[a] };
            if d <= self.delta {
                self.delta = d;
                self.u_out = u;
                result = 2;
            }
            u = self.parent[u];
        }
        if result == 1 {
            self.u_in = first;
            self.v_in = second;
        } else {
            self.u_in = second;
            self.v_in = first;
        }
        result != 0
    }

    fn change_flow(&mut self, change: bool) {
        let e = self.in_arc;
        if self.delta > 0 {
            let val = self.state[e] as i64 * self.delta;
            self.flow[e] += val;
            let mut u = self.source[e];
            while u != self.join {
                let a = self.pred[u];
                self.flow[a] -= self.pred_dir[u] as i64 * val;
                u = self.parent[u];
            }
            let mut u = self.target[e];
            while u != self.join {
                let a = self.pred[u];
                self.flow[a] += self.pred_dir[u] as i64 * val;
                u = self.parent[u];
            }
        }
        if change {
            self.state[e] = TREE;
            let out = self.pred[self.u_out];
            self.state[out] = if self.flow[out] == 0 { LOWER } else { UPPER };
        } else {
            self.state[e] = -self.state[e];
        }
    }

    /// Hang the subtree under `u_out` from `v_in` by the entering arc, re-rooted at `u_in`.
    fn update_tree(&mut self) {
        let (u_in, v_in, u_out, join, in_arc) = (self.u_in, self.v_in, self.u_out, self.join, self.in_arc);
        let old_rev_thread = self.rev_thread[u_out];
        let old_succ_num = self.succ_num[u_out];
        let old_last_succ = self.last_succ[u_out];
        let v_out = self.parent[u_out];
        if u_in == u_out {
            self.parent[u_in] = v_in;
            self.pred[u_in] = in_arc;
            self.pred_dir[u_in] = if u_in == self.source[in_arc] { UP } else { DOWN };
            if self.thread[v_in] != u_out {
                let mut after = self.thread[old_last_succ];
                self.thread[old_rev_thread] = after;
                self.rev_thread[after] = old_rev_thread;
                after = self.thread[v_in];
                self.thread[v_in] = u_out;
                self.rev_thread[u_out] = v_in;
                self.thread[old_last_succ] = after;
                self.rev_thread[after] = old_last_succ;
            }
        } else {
            let thread_continue = if old_rev_thread == v_in { self.thread[old_last_succ] } else { self.thread[v_in] };
            let mut stem = u_in;
            let mut par_stem = v_in;
            let mut last = self.last_succ[u_in];
            let mut after = self.thread[last];
            self.thread[v_in] = u_in;
            self.dirty.clear();
            self.dirty.push(v_in);
            while stem != u_out {
                let next_stem = self.parent[stem];
                self.thread[last] = next_stem;
                self.dirty.push(last);
                let before = self.rev_thread[stem];
                self.thread[before] = after;
                self.rev_thread[after] = before;
                self.parent[stem] = par_stem;
                par_stem = stem;
                stem = next_stem;
                last = if self.last_succ[stem] == self.last_succ[par_stem] { self.rev_thread[par_stem] } else { self.last_succ[stem] };
                after = self.thread[last];
            }
            self.parent[u_out] = par_stem;
            self.thread[last] = thread_continue;
            self.rev_thread[thread_continue] = last;
            self.last_succ[u_out] = last;
            if old_rev_thread != v_in {
                self.thread[old_rev_thread] = after;
                self.rev_thread[after] = old_rev_thread;
            }
            for i in 0..self.dirty.len() {
                let u = self.dirty[i];
                let t = self.thread[u];
                self.rev_thread[t] = u;
            }
            let mut tmp_sc = 0usize;
            let tmp_ls = self.last_succ[u_out];
            let mut u = u_out;
            while u != u_in {
                let p = self.parent[u];
                self.pred[u] = self.pred[p];
                self.pred_dir[u] = -self.pred_dir[p];
                tmp_sc = tmp_sc + self.succ_num[u] - self.succ_num[p];
                self.succ_num[u] = tmp_sc;
                self.last_succ[p] = tmp_ls;
                u = p;
            }
            self.pred[u_in] = in_arc;
            self.pred_dir[u_in] = if u_in == self.source[in_arc] { UP } else { DOWN };
            self.succ_num[u_in] = old_succ_num;
        }
        let up_limit_out = if self.last_succ[join] == v_in { join } else { NONE };
        let last_succ_out = self.last_succ[u_out];
        let mut u = v_in;
        while u != NONE && self.last_succ[u] == v_in {
            self.last_succ[u] = last_succ_out;
            u = self.parent[u];
        }
        if join != old_rev_thread && v_in != old_rev_thread {
            let mut u = v_out;
            while u != up_limit_out && u != NONE && self.last_succ[u] == old_last_succ {
                self.last_succ[u] = old_rev_thread;
                u = self.parent[u];
            }
        } else if last_succ_out != old_last_succ {
            let mut u = v_out;
            while u != up_limit_out && u != NONE && self.last_succ[u] == old_last_succ {
                self.last_succ[u] = last_succ_out;
                u = self.parent[u];
            }
        }
        let mut u = v_in;
        while u != join {
            self.succ_num[u] += old_succ_num;
            u = self.parent[u];
        }
        let mut u = v_out;
        while u != join {
            self.succ_num[u] -= old_succ_num;
            u = self.parent[u];
        }
    }

    fn update_potential(&mut self) {
        let u_in = self.u_in;
        let sigma = self.pi[self.v_in] - self.pi[u_in] - self.pred_dir[u_in] as i128 * self.cost[self.in_arc];
        let end = self.thread[self.last_succ[u_in]];
        let mut u = u_in;
        while u != end {
            self.pi[u] += sigma;
            u = self.thread[u];
        }
    }

    /// The cycle the entering arc closes through the tree, its arcs in the direction flow would go around it.
    fn cycle(&self) -> Vec<usize> {
        let e = self.in_arc;
        let (first, second) = if self.state[e] == LOWER { (self.source[e], self.target[e]) } else { (self.target[e], self.source[e]) };
        let mut back = Vec::new();
        let mut u = second;
        while u != self.join {
            back.push(self.pred[u]);
            u = self.parent[u];
        }
        let mut down = Vec::new();
        let mut u = first;
        while u != self.join {
            down.push(self.pred[u]);
            u = self.parent[u];
        }
        down.reverse();
        let mut cycle = vec![e];
        cycle.extend(back);
        cycle.extend(down);
        cycle
    }
}

/// The least-cost flow meeting `supply` (positive where flow starts, negative where it ends) over `arcs`.
pub fn min_cost_flow(nodes: usize, supply: &[i64], arcs: &[Arc]) -> Result<Solution, FlowError> {
    validate(nodes, supply, arcs)?;
    let mut s = Simplex::new(nodes, supply, arcs);
    while s.entering() {
        s.join_node();
        let change = s.leaving();
        if s.delta == INF {
            return Ok(Solution::Unbounded { cycle: s.cycle() });
        }
        s.change_flow(change);
        if change {
            s.update_tree();
            s.update_potential();
        }
    }
    let m = arcs.len();
    if (m..m + nodes).any(|e| s.flow[e] != 0) {
        return Ok(Solution::Infeasible { cut: infeasible_cut(nodes, arcs, &s) });
    }
    let flow: Vec<i64> = s.flow[..m].to_vec();
    let cost = arcs.iter().zip(&flow).map(|(a, &f)| a.cost as i128 * f as i128).sum();
    Ok(Solution::Optimal { flow, potential: s.pi[..nodes].to_vec(), cost })
}

/// The nodes the residual network reaches from those whose supply did not leave: no residual path reaches a node
/// whose demand went unmet, or the optimum would have used it, so their supply exceeds what can leave them.
fn infeasible_cut(nodes: usize, arcs: &[Arc], s: &Simplex) -> Vec<usize> {
    let m = arcs.len();
    let mut reached = vec![false; nodes];
    let mut stack: Vec<usize> = (0..nodes).filter(|&u| s.source[m + u] == u && s.flow[m + u] > 0).collect();
    for &u in &stack {
        reached[u] = true;
    }
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); nodes];
    let mut inn: Vec<Vec<usize>> = vec![Vec::new(); nodes];
    for (k, a) in arcs.iter().enumerate() {
        out[a.from].push(k);
        inn[a.to].push(k);
    }
    while let Some(u) = stack.pop() {
        for &k in &out[u] {
            let v = arcs[k].to;
            if !reached[v] && arcs[k].capacity.map_or(true, |c| s.flow[k] < c) {
                reached[v] = true;
                stack.push(v);
            }
        }
        for &k in &inn[u] {
            let v = arcs[k].from;
            if !reached[v] && s.flow[k] > 0 {
                reached[v] = true;
                stack.push(v);
            }
        }
    }
    (0..nodes).filter(|&u| reached[u]).collect()
}

/// Whether `solution` is the problem's answer, by its certificate alone.
pub fn check(nodes: usize, supply: &[i64], arcs: &[Arc], solution: &Solution) -> bool {
    if validate(nodes, supply, arcs).is_err() {
        return false;
    }
    match solution {
        Solution::Optimal { flow, potential, cost } => {
            if flow.len() != arcs.len() || potential.len() != nodes {
                return false;
            }
            let mut net: Vec<i128> = supply.iter().map(|&s| s as i128).collect();
            for (a, &f) in arcs.iter().zip(flow) {
                if f < 0 || a.capacity.is_some_and(|c| f > c) {
                    return false;
                }
                net[a.from] -= f as i128;
                net[a.to] += f as i128;
            }
            if net.iter().any(|&x| x != 0) {
                return false;
            }
            let slack = arcs.iter().zip(flow).all(|(a, &f)| {
                let Some(rc) = (a.cost as i128).checked_add(potential[a.from]).and_then(|x| x.checked_sub(potential[a.to])) else { return false };
                (a.capacity.is_some_and(|c| f == c) || rc >= 0) && (f == 0 || rc <= 0)
            });
            slack && *cost == arcs.iter().zip(flow).map(|(a, &f)| a.cost as i128 * f as i128).sum::<i128>()
        }
        Solution::Infeasible { cut } => {
            let mut inside = vec![false; nodes];
            for &u in cut {
                if u >= nodes || std::mem::replace(&mut inside[u], true) {
                    return false;
                }
            }
            let mut leaving: i128 = 0;
            for a in arcs {
                if inside[a.from] && !inside[a.to] {
                    match a.capacity {
                        Some(c) => leaving += c as i128,
                        None => return false,
                    }
                }
            }
            cut.iter().map(|&u| supply[u] as i128).sum::<i128>() > leaving
        }
        Solution::Unbounded { cycle } => {
            !cycle.is_empty()
                && cycle.iter().all(|&k| k < arcs.len() && arcs[k].capacity.is_none())
                && cycle.iter().enumerate().all(|(i, &k)| arcs[k].to == arcs[cycle[(i + 1) % cycle.len()]].from)
                && cycle.iter().map(|&k| arcs[k].cost as i128).sum::<i128>() < 0
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/min_cost_flow/tests.rs"]
mod tests;
