//! The **coalgebraic / `Poly` view**, made into checked code rather than narration.
//!
//! A certified refutation is a *discrete dynamical system* — a coalgebra `S → p(S)` for a polynomial
//! functor `p` (positions = states, directions = the certified moves available there), exactly
//! Spivak's model of dynamical systems in `Poly`. A Lyapunov measure is then a **coalgebra morphism
//! to the countdown coalgebra** `(ℕ, n ↦ n-1)` — the canonical well-founded object: it intertwines
//! the system's dynamics with the countdown's, i.e. every transition strictly decreases it.
//!
//! The load-bearing theorem (Taylor; Adámek–Milius–Moss on *well-founded coalgebras*): **a finite
//! system is well-founded (terminating — no infinite forward trajectory) iff it admits a morphism to
//! a well-founded object.** That equivalence *is* the synthesis–impossibility duality at the
//! categorical level: a collapsing measure is a countdown morphism, and its non-existence is the
//! presence of a cycle. We make both directions constructive and machine-checked here, and connect
//! it back: every Lyapunov trajectory this crate produces is a countdown morphism on its path system.

/// A finite transition system: a coalgebra `S → 𝒫(S)` for the finite-powerset polynomial functor.
/// `successors[s]` lists the states reachable from `s` in one step.
#[derive(Clone, Debug)]
pub struct TransitionSystem {
    pub n_states: usize,
    pub successors: Vec<Vec<usize>>,
}

impl TransitionSystem {
    /// A linear "path" system `0 → 1 → … → n-1` — the shape of a (deterministic) proof trajectory.
    pub fn path(n: usize) -> TransitionSystem {
        TransitionSystem {
            n_states: n,
            successors: (0..n)
                .map(|s| if s + 1 < n { vec![s + 1] } else { vec![] })
                .collect(),
        }
    }
}

/// Is `measure` a **coalgebra morphism to the countdown coalgebra**? Every transition `s → t` must
/// strictly decrease it (`measure[t] < measure[s]`) — the morphism square commuting with `n ↦ n-1`.
/// Because the countdown is well-founded, a `true` here *witnesses* that the system is well-founded.
pub fn is_countdown_morphism(system: &TransitionSystem, measure: &[u64]) -> bool {
    if measure.len() != system.n_states {
        return false;
    }
    system.successors.iter().enumerate().all(|(s, succ)| {
        succ.iter()
            .all(|&t| t < measure.len() && measure[t] < measure[s])
    })
}

/// Is the system **well-founded** (acyclic — no infinite forward trajectory)? Equivalent, for a
/// finite system, to "is a DAG". Computed by Kahn's algorithm: a full topological order exists iff
/// there is no cycle.
pub fn is_well_founded(system: &TransitionSystem) -> bool {
    let n = system.n_states;
    let mut indeg = vec![0usize; n];
    for succ in &system.successors {
        for &t in succ {
            if t < n {
                indeg[t] += 1;
            }
        }
    }
    let mut queue: Vec<usize> = (0..n).filter(|&s| indeg[s] == 0).collect();
    let mut seen = 0;
    let mut qi = 0;
    while qi < queue.len() {
        let s = queue[qi];
        qi += 1;
        seen += 1;
        for &t in &system.successors[s] {
            if t < n {
                indeg[t] -= 1;
                if indeg[t] == 0 {
                    queue.push(t);
                }
            }
        }
    }
    seen == n // all states ordered ⇒ no cycle ⇒ well-founded
}

/// The **canonical countdown morphism** of a well-founded system: the longest forward path from each
/// state (sinks = 0). This is the *terminal* such morphism — the existence half of the theorem made
/// constructive. Returns `None` if the system is not well-founded (no morphism exists).
pub fn canonical_countdown_morphism(system: &TransitionSystem) -> Option<Vec<u64>> {
    if !is_well_founded(system) {
        return None;
    }
    let n = system.n_states;
    let mut rank = vec![0u64; n];
    let mut done = vec![false; n];
    // Memoised longest-path via an explicit stack (the system is a DAG, so this terminates).
    for start in 0..n {
        if done[start] {
            continue;
        }
        let mut stack = vec![start];
        while let Some(&s) = stack.last() {
            let pending: Option<usize> = system.successors[s]
                .iter()
                .copied()
                .find(|&t| t < n && !done[t]);
            match pending {
                Some(t) => stack.push(t),
                None => {
                    let r = system.successors[s]
                        .iter()
                        .filter(|&&t| t < n)
                        .map(|&t| rank[t] + 1)
                        .max()
                        .unwrap_or(0);
                    rank[s] = r;
                    done[s] = true;
                    stack.pop();
                }
            }
        }
    }
    Some(rank)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/coalgebra/tests.rs"]
mod tests;
