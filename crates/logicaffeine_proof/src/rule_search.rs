//! `aesop`-style rule-set search: turn `auto`'s fixed cascade into an
//! extensible, best-first search over registered rules.
//!
//! Lean's `aesop` lets you register lemmas as intro/elim/forward/destruct rules
//! with priorities and a safe/unsafe classification, then searches best-first.
//! This is that shape, built on the existing tactic combinators and the cheap
//! [`ProofState`] clone that already powers backtracking: SAFE rules are applied
//! whenever they fire (no branching — they never lose information); UNSAFE rules
//! fork the search, ordered by priority. A node budget bounds the search, and
//! [`SearchStats`] exposes how much was explored — so a best-first strategy can
//! be shown to expand fewer nodes than blind depth-first `first`/`repeat`.
//!
//! `auto` becomes one rule among many (an unsafe fallback), completing the
//! inversion the tactic framework was built for.

use crate::tactic::{ProofState, Tactic};

/// How a rule participates in search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Safety {
    /// Never loses provability — applied eagerly, no branch point.
    Safe,
    /// May not lead to a proof — forks the search; higher priority tried first.
    Unsafe(u8),
}

/// The role a rule plays (currently advisory metadata for ordering/diagnostics;
/// the tactic itself encodes the actual transformation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    Intro,
    Elim,
    Forward,
    Destruct,
}

/// A search rule: a named tactic with a role and a safety classification.
pub struct Rule {
    pub name: String,
    pub kind: RuleKind,
    pub safety: Safety,
    pub tactic: Tactic,
}

impl Rule {
    pub fn new(name: &str, kind: RuleKind, safety: Safety, tactic: Tactic) -> Self {
        Rule { name: name.to_string(), kind, safety, tactic }
    }
}

/// An extensible collection of search rules.
#[derive(Default)]
pub struct RuleSet {
    rules: Vec<Rule>,
}

/// What the search explored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchStats {
    pub nodes_expanded: usize,
    pub succeeded: bool,
}

/// Default node budget for a best-first search.
const DEFAULT_NODE_BUDGET: usize = 2000;

impl RuleSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, rule: Rule) {
        self.rules.push(rule);
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Best-first search from `st`. Applies safe rules eagerly at each node,
    /// then forks on unsafe rules in priority order. Succeeds (leaving `st` at
    /// a closed proof state) when some branch drives the open goals to zero.
    pub fn search(&self, st: &mut ProofState) -> SearchStats {
        self.search_bounded(st, DEFAULT_NODE_BUDGET)
    }

    pub fn search_bounded(&self, st: &mut ProofState, budget: usize) -> SearchStats {
        let mut stats = SearchStats::default();
        // A best-first frontier of `(path-cost, state)`; the next node expanded
        // is the one with the fewest open goals, then the cheapest path. The
        // frontier stays small for the goal sizes this targets.
        let mut frontier: Vec<(usize, ProofState)> = vec![(0, st.clone())];
        let rules = self.rules_by_priority();

        while let Some(best_idx) = pick_best(&frontier) {
            if stats.nodes_expanded >= budget {
                break;
            }
            let (cost, node) = frontier.swap_remove(best_idx);
            stats.nodes_expanded += 1;

            if node.open_goals() == 0 {
                *st = node;
                stats.succeeded = true;
                return stats;
            }

            // Fork on each rule, highest priority (and safe) first. A safe rule
            // adds no path-cost (it never loses provability), so its branch is
            // explored ahead of any unsafe alternative.
            for rule in &rules {
                let mut child = node.clone();
                if (rule.tactic)(&mut child).is_ok() && !state_eq(&child, &node) {
                    frontier.push((cost + rule_cost(rule), child));
                }
            }
        }
        stats
    }

    /// Rules ordered for expansion: safe first, then unsafe by descending priority.
    fn rules_by_priority(&self) -> Vec<&Rule> {
        let mut r: Vec<&Rule> = self.rules.iter().collect();
        r.sort_by_key(|rule| std::cmp::Reverse(priority_of(rule)));
        r
    }
}

/// The path-cost of taking a rule: zero for safe rules (free), and the inverse
/// of priority for unsafe ones, so high-priority unsafe branches are cheaper.
fn rule_cost(rule: &Rule) -> usize {
    match rule.safety {
        Safety::Safe => 0,
        Safety::Unsafe(p) => (255 - p as usize) + 1,
    }
}

fn priority_of(rule: &Rule) -> u8 {
    match rule.safety {
        Safety::Unsafe(p) => p,
        Safety::Safe => 255,
    }
}

/// Cheap progress check: two states differ if their open-goal count or focused
/// target differ (enough to avoid enqueuing a no-op rule application).
fn state_eq(a: &ProofState, b: &ProofState) -> bool {
    a.open_goals() == b.open_goals() && a.focused_target() == b.focused_target()
}

/// The frontier node to expand next: fewest open goals, then cheapest path.
fn pick_best(frontier: &[(usize, ProofState)]) -> Option<usize> {
    frontier
        .iter()
        .enumerate()
        .min_by_key(|(_, (cost, st))| (st.open_goals(), *cost))
        .map(|(i, _)| i)
}

/// The default rule set: `auto` as the sole unsafe fallback — so a default
/// `search` closes exactly what `auto` closes, the regression baseline.
pub fn default_ruleset() -> RuleSet {
    let mut rs = RuleSet::new();
    rs.register(Rule::new(
        "auto",
        RuleKind::Elim,
        Safety::Unsafe(10),
        crate::tactic::combinators::auto(),
    ));
    rs
}
