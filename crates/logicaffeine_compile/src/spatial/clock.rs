//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE SPATIAL CLOCK — a period computed from the graph, not fitted to a counter.
//!
//! A spatial program has no meaningful global clock. Sixty-four sites run concurrently, each engine
//! advances when its inputs are present, and "how long does it take" is not a property any one of them
//! holds. Asking a simulator for a cycle count answers a different question — ttsim's counter advances
//! once per simulated clock while it steps the RV32 cores, so a 4x deeper reduction moves it by one
//! cycle (`benchmarks/silicon/ttsim_instrument_audit.json`).
//!
//! But the choreography is not an arbitrary concurrent program. [`certificate`](super::certificate)
//! already proves it is a **Petri MARKED GRAPH** — every place has exactly one producer and one
//! consumer — and certifies its liveness by Commoner's theorem. Marked graphs have a classical and
//! exact steady-state result: the system settles into a period equal to the **maximum cycle mean**
//!
//! ```text
//!     Π  =  max over directed cycles C of   ( Σ delay(t) for t ∈ C ) / ( Σ tokens(p) for p ∈ C )
//! ```
//!
//! That is the spatial clock. It is a THEOREM about the graph the compiler already builds and already
//! proves live, evaluated at per-transition delays that are architectural facts — a DRAM tile's
//! bandwidth occupancy, an engine's issue rate — rather than regression coefficients. Nothing here is
//! fitted, so nothing here can be corrupted by a counter that cannot see work.
//!
//! It also EXPLAINS itself: the cycle that attains the maximum is the critical cycle, and it names the
//! exact chain of events that sets the period. "Bound on DRAM bandwidth" is a summary; the critical
//! cycle is the evidence.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use super::certificate::{MarkedGraph, Transition};

/// The steady-state period of a marked graph, with the cycle that attains it.
#[derive(Clone, Debug, PartialEq)]
pub struct Period {
    /// Cycles per steady-state iteration — the spatial clock.
    pub period: f64,
    /// The transitions of a cycle attaining the maximum, in order. Empty when the graph is acyclic,
    /// which means the program has no steady state to speak of: it runs once and drains.
    pub critical_cycle: Vec<usize>,
    /// Whether any directed cycle exists at all.
    pub cyclic: bool,
    /// False when some directed cycle carries NO token. Such a cycle can never fire — it is a deadlock,
    /// and it is exactly what Commoner's condition forbids — so the period is infinite rather than
    /// large. Reporting a big finite number there would hand a caller a schedule that cannot run.
    pub live: bool,
}

/// One edge of the transition graph: place `p` runs producer → consumer, carrying `tokens` initially.
struct Edge {
    from: usize,
    to: usize,
    delay: f64,
    tokens: f64,
}

fn edges(g: &MarkedGraph) -> Vec<Edge> {
    let mut producer = vec![usize::MAX; g.places.len()];
    for (ti, t) in g.transitions.iter().enumerate() {
        for &p in &t.produces {
            producer[p] = ti;
        }
    }
    let mut out = Vec::new();
    for (ti, t) in g.transitions.iter().enumerate() {
        for &p in &t.consumes {
            let from = producer[p];
            if from == usize::MAX {
                continue; // a source place: nobody produces it, so it closes no cycle
            }
            out.push(Edge {
                from,
                to: ti,
                delay: 0.0,
                tokens: g.places[p].marking as f64,
            });
        }
    }
    out
}

/// **The spatial clock.** The maximum cycle mean of `g` under `delay`, by parametric search: for a
/// candidate period λ, give each edge the weight `delay(producer) − λ·tokens`. A positive-weight cycle
/// exists exactly when some cycle's mean exceeds λ, so Bellman-Ford's longest-path relaxation decides
/// the comparison and bisection converges on Π.
///
/// Deterministic: the same graph and the same delays give the same period, bit for bit.
pub fn cycle_period(g: &MarkedGraph, delay: impl Fn(&Transition) -> f64) -> Period {
    let n = g.transitions.len();
    let mut e = edges(g);
    for edge in e.iter_mut() {
        edge.delay = delay(&g.transitions[edge.from]);
    }
    // A cycle needs at least one edge whose head reaches its own tail; bisect only if one exists.
    let hi_seed = e.iter().map(|x| x.delay).fold(0.0f64, f64::max).max(1.0) * n.max(1) as f64;
    if n == 0 || e.is_empty() || positive_cycle(n, &e, hi_seed * 2.0).is_none() {
        // No cycle attains even an absurd mean ⇒ acyclic.
        if n == 0 || e.is_empty() || !has_cycle(n, &e) {
            return Period {
                period: 0.0,
                critical_cycle: Vec::new(),
                cyclic: false,
                live: true,
            };
        }
    }
    // A token-free cycle never fires. Detect it before bisecting, because parametric search cannot
    // distinguish "infinite mean" from "very large mean" and would return a plausible finite lie.
    let starved: Vec<&Edge> = e.iter().filter(|x| x.tokens == 0.0).collect();
    let mut sn = vec![0usize; n];
    let mut sadj = vec![Vec::new(); n];
    for x in &starved {
        sadj[x.from].push(x.to);
        sn[x.to] += 1;
    }
    {
        let mut q: Vec<usize> = (0..n).filter(|&i| sn[i] == 0).collect();
        let mut seen = 0usize;
        while let Some(v) = q.pop() {
            seen += 1;
            for &w in &sadj[v] {
                sn[w] -= 1;
                if sn[w] == 0 {
                    q.push(w);
                }
            }
        }
        if seen != n {
            return Period {
                period: f64::INFINITY,
                critical_cycle: Vec::new(),
                cyclic: true,
                live: false,
            };
        }
    }
    let (mut lo, mut hi) = (0.0f64, hi_seed.max(1.0));
    while positive_cycle(n, &e, hi).is_some() {
        hi *= 2.0;
        if hi > 1e18 {
            break;
        }
    }
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if positive_cycle(n, &e, mid).is_some() {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let period = 0.5 * (lo + hi);
    // Report the cycle attaining it: at a period just under Π one still exists.
    let critical_cycle = positive_cycle(n, &e, period * (1.0 - 1e-9) - 1e-12).unwrap_or_default();
    Period {
        period,
        critical_cycle,
        cyclic: true,
        live: true,
    }
}

/// Any directed cycle at all, ignoring weights.
fn has_cycle(n: usize, e: &[Edge]) -> bool {
    let mut indeg = vec![0usize; n];
    let mut adj = vec![Vec::new(); n];
    for x in e {
        adj[x.from].push(x.to);
        indeg[x.to] += 1;
    }
    let mut q: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut seen = 0usize;
    while let Some(v) = q.pop() {
        seen += 1;
        for &w in &adj[v] {
            indeg[w] -= 1;
            if indeg[w] == 0 {
                q.push(w);
            }
        }
    }
    seen != n
}

/// A cycle of strictly positive weight under `w(edge) = delay − λ·tokens`, as a transition list.
fn positive_cycle(n: usize, e: &[Edge], lambda: f64) -> Option<Vec<usize>> {
    // Longest path relaxation from every node at once; a node still improving after n rounds lies on
    // (or downstream of) a positive cycle.
    let mut dist = vec![0.0f64; n];
    let mut pred = vec![usize::MAX; n];
    let mut last = usize::MAX;
    for round in 0..=n {
        last = usize::MAX;
        for x in e {
            let cand = dist[x.from] + x.delay - lambda * x.tokens;
            if cand > dist[x.to] + 1e-12 {
                dist[x.to] = cand;
                pred[x.to] = x.from;
                last = x.to;
            }
        }
        if last == usize::MAX {
            return None;
        }
        let _ = round;
    }
    // Walk predecessors n times to land inside the cycle, then read it off.
    let mut v = last;
    for _ in 0..n {
        v = pred[v];
        if v == usize::MAX {
            return None;
        }
    }
    let start = v;
    let mut cyc = vec![start];
    let mut u = pred[start];
    while u != start {
        if u == usize::MAX || cyc.len() > n {
            return None;
        }
        cyc.push(u);
        u = pred[u];
    }
    cyc.reverse();
    Some(cyc)
}
