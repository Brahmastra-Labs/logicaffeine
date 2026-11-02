//! Congruence Closure Tactic
//!
//! Proves equalities over uninterpreted functions using Union-Find with
//! congruence propagation. This implements a simplified Downey-Sethi-Tarjan
//! algorithm.
//!
//! # Algorithm
//!
//! The congruence closure tactic works in four steps:
//! 1. **Build E-graph**: Add all subterms from goal and hypotheses
//! 2. **Merge hypotheses**: For each hypothesis `x = y`, merge x and y
//! 3. **Propagate**: When `x = y` and `f(x)`, `f(y)` exist, merge `f(x)` and `f(y)`
//! 4. **Check**: Goal `a = b` holds iff a and b are in the same equivalence class
//!
//! # Supported Goals
//!
//! - Direct equalities: `Eq a a` (reflexivity)
//! - Implications: `(Eq x y) -> (Eq (f x) (f y))` (congruence)
//! - Nested implications with multiple hypotheses
//!
//! # E-Graph Structure
//!
//! The E-graph maintains:
//! - A Union-Find for equivalence classes
//! - Hash-consing for structural sharing
//! - Use lists for efficient congruence propagation

use std::collections::HashMap;

use crate::term::{Literal, Term};

type TermId = usize;

/// The shared Union-Find — one equivalence engine underneath the kernel's
/// congruence closure and the compiler's e-graph.
pub use logicaffeine_base::union_find::UnionFind;

// =============================================================================
// E-GRAPH DATA STRUCTURE
// =============================================================================

/// Node in the E-graph representing a term.
///
/// Terms are represented in a curried style where function application
/// is a binary node. For example, `f(x, y)` is `App(App(f, x), y)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ENode {
    /// Integer literal from `SLit`.
    Lit(i64),
    /// De Bruijn variable from `SVar`.
    Var(i64),
    /// Named constant or function symbol from `SName`.
    Name(String),
    /// Function application (curried): `func` applied to `arg`.
    App {
        /// The function being applied.
        func: TermId,
        /// The argument.
        arg: TermId,
    },
}

/// E-graph with congruence closure.
///
/// Combines a Union-Find for equivalence classes with hash-consing for
/// structural sharing and use lists for efficient congruence propagation.
pub struct EGraph {
    /// The nodes stored in the graph.
    nodes: Vec<ENode>,
    /// Union-Find tracking equivalence classes.
    uf: UnionFind,
    /// Hash-consing map: node content to its canonical ID.
    node_map: HashMap<ENode, TermId>,
    /// Pending merges to propagate (worklist algorithm).
    pending: Vec<(TermId, TermId)>,
    /// Use lists: for each term, the App nodes that use it as func or arg.
    /// Used to find potential congruences when terms are merged.
    use_list: Vec<Vec<TermId>>,
}

impl EGraph {
    pub fn new() -> Self {
        EGraph {
            nodes: Vec::new(),
            uf: UnionFind::new(),
            node_map: HashMap::new(),
            pending: Vec::new(),
            use_list: Vec::new(),
        }
    }

    /// Add a node, return its ID (hash-consed)
    pub fn add(&mut self, node: ENode) -> TermId {
        // Hash-consing: return existing ID if node already exists
        if let Some(&id) = self.node_map.get(&node) {
            return id;
        }

        let id = self.nodes.len();
        self.nodes.push(node.clone());
        self.node_map.insert(node.clone(), id);
        self.uf.make_set();
        self.use_list.push(Vec::new());

        // Register in use lists for congruence detection
        if let ENode::App { func, arg } = &node {
            self.use_list[*func].push(id);
            self.use_list[*arg].push(id);
        }

        id
    }

    /// Merge two terms and propagate congruences
    pub fn merge(&mut self, a: TermId, b: TermId) {
        self.pending.push((a, b));
        self.propagate();
    }

    /// Propagate congruences until fixed point
    fn propagate(&mut self) {
        while let Some((a, b)) = self.pending.pop() {
            let ra = self.uf.find(a);
            let rb = self.uf.find(b);
            if ra == rb {
                continue;
            }

            // Before merging, collect uses for congruence checking
            let uses_a: Vec<TermId> = self.use_list[ra].clone();
            let uses_b: Vec<TermId> = self.use_list[rb].clone();

            // Merge equivalence classes
            self.uf.union(ra, rb);
            let new_root = self.uf.find(ra);

            // Check for new congruences first (before modifying use lists)
            // If f(a) and f(b) exist, and a=b now, then f(a)=f(b)
            for &ua in &uses_a {
                for &ub in &uses_b {
                    if self.congruent(ua, ub) {
                        self.pending.push((ua, ub));
                    }
                }
            }

            // Merge use lists (now safe to consume uses_a/uses_b)
            if new_root == ra {
                for u in uses_b {
                    self.use_list[ra].push(u);
                }
            } else {
                for u in uses_a {
                    self.use_list[rb].push(u);
                }
            }
        }
    }

    /// Check if two application nodes are congruent
    fn congruent(&mut self, a: TermId, b: TermId) -> bool {
        match (&self.nodes[a].clone(), &self.nodes[b].clone()) {
            (ENode::App { func: f1, arg: a1 }, ENode::App { func: f2, arg: a2 }) => {
                self.uf.find(*f1) == self.uf.find(*f2) && self.uf.find(*a1) == self.uf.find(*a2)
            }
            _ => false,
        }
    }

    /// Check if two terms are in the same equivalence class
    pub fn equivalent(&mut self, a: TermId, b: TermId) -> bool {
        self.uf.find(a) == self.uf.find(b)
    }
}

// =============================================================================
// SYNTAX TERM REIFICATION
// =============================================================================

/// Reify a Syntax term into an E-graph node.
///
/// Converts the deep embedding (Syntax) into E-graph nodes, returning
/// the ID of the root node. Subterms are recursively reified and
/// hash-consed (duplicate structures share the same ID).
///
/// # Returns
///
/// `Some(id)` on successful reification, `None` if the term cannot be reified.
pub fn reify(egraph: &mut EGraph, term: &Term) -> Option<TermId> {
    // SLit n -> Lit(n)
    if let Some(n) = extract_slit(term) {
        return Some(egraph.add(ENode::Lit(n)));
    }

    // SVar i -> Var(i)
    if let Some(i) = extract_svar(term) {
        return Some(egraph.add(ENode::Var(i)));
    }

    // SName s -> Name(s)
    if let Some(name) = extract_sname(term) {
        return Some(egraph.add(ENode::Name(name)));
    }

    // SApp f a -> App { func, arg }
    if let Some((func_term, arg_term)) = extract_sapp(term) {
        let func = reify(egraph, &func_term)?;
        let arg = reify(egraph, &arg_term)?;
        return Some(egraph.add(ENode::App { func, arg }));
    }

    None
}

// =============================================================================
// GOAL DECOMPOSITION
// =============================================================================

/// Decompose a goal into hypotheses and conclusion.
///
/// Peels off nested implications to extract equality hypotheses.
/// For example, `(h1 -> h2 -> conclusion)` becomes `([h1, h2], conclusion)`.
///
/// # Returns
///
/// A tuple of:
/// - Vector of equality hypothesis pairs (LHS, RHS)
/// - The final conclusion term
///
/// Only equalities in hypothesis position are extracted; other hypotheses
/// are ignored.
pub fn decompose_goal(goal: &Term) -> (Vec<(Term, Term)>, Term) {
    let mut hypotheses = Vec::new();
    let mut current = goal.clone();

    // Peel off nested implications
    while let Some((hyp, rest)) = extract_implication(&current) {
        if let Some((lhs, rhs)) = extract_equality(&hyp) {
            hypotheses.push((lhs, rhs));
        }
        current = rest;
    }

    (hypotheses, current)
}

/// Check if a goal is provable by congruence closure.
///
/// This is the main entry point for the CC tactic. It builds an E-graph,
/// adds hypothesis equalities, propagates congruences, and checks if the
/// conclusion follows.
///
/// # Supported Goals
///
/// - Direct equalities: `(Eq a a)` succeeds by reflexivity
/// - Implications: `(implies (Eq x y) (Eq (f x) (f y)))` succeeds by congruence
/// - Multiple hypotheses: `(implies (Eq a b) (implies (Eq b c) (Eq a c)))`
///
/// # Returns
///
/// `true` if the goal is provable by congruence closure, `false` otherwise.
pub fn check_goal(goal: &Term) -> bool {
    let (hypotheses, conclusion) = decompose_goal(goal);

    // Conclusion must be an equality
    let (lhs, rhs) = match extract_equality(&conclusion) {
        Some(eq) => eq,
        None => return false,
    };

    let mut egraph = EGraph::new();

    // IMPORTANT: Reify conclusion FIRST so that fx and fy exist in the graph
    // with their use lists populated. This way when we merge x=y, congruence
    // will propagate to fx=fy.
    let lhs_id = match reify(&mut egraph, &lhs) {
        Some(id) => id,
        None => return false,
    };

    let rhs_id = match reify(&mut egraph, &rhs) {
        Some(id) => id,
        None => return false,
    };

    // Now reify and merge hypothesis equalities
    // The subterms (x, y) will be hash-consed with the ones in fx, fy
    for (h_lhs, h_rhs) in &hypotheses {
        let h_lhs_id = match reify(&mut egraph, h_lhs) {
            Some(id) => id,
            None => return false,
        };
        let h_rhs_id = match reify(&mut egraph, h_rhs) {
            Some(id) => id,
            None => return false,
        };
        egraph.merge(h_lhs_id, h_rhs_id);
    }

    // Check if conclusion follows by congruence
    egraph.equivalent(lhs_id, rhs_id)
}

// =============================================================================
// HELPER EXTRACTORS
// =============================================================================

/// Extract integer from SLit n
fn extract_slit(term: &Term) -> Option<i64> {
    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SLit" {
                if let Term::Lit(Literal::Int(n)) = arg.as_ref() {
                    return Some(*n);
                }
            }
        }
    }
    None
}

/// Extract variable index from SVar i
fn extract_svar(term: &Term) -> Option<i64> {
    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SVar" {
                if let Term::Lit(Literal::Int(i)) = arg.as_ref() {
                    return Some(*i);
                }
            }
        }
    }
    None
}

/// Extract name from SName "x"
fn extract_sname(term: &Term) -> Option<String> {
    if let Term::App(ctor, arg) = term {
        if let Term::Global(name) = ctor.as_ref() {
            if name == "SName" {
                if let Term::Lit(Literal::Text(s)) = arg.as_ref() {
                    return Some(s.clone());
                }
            }
        }
    }
    None
}

/// Extract unary application: SApp f a
fn extract_sapp(term: &Term) -> Option<(Term, Term)> {
    if let Term::App(outer, arg) = term {
        if let Term::App(sapp, func) = outer.as_ref() {
            if let Term::Global(ctor) = sapp.as_ref() {
                if ctor == "SApp" {
                    return Some((func.as_ref().clone(), arg.as_ref().clone()));
                }
            }
        }
    }
    None
}

/// Extract implication: SApp (SApp (SName "implies") hyp) concl
fn extract_implication(term: &Term) -> Option<(Term, Term)> {
    if let Some((op, hyp, concl)) = extract_binary_app(term) {
        if op == "implies" {
            return Some((hyp, concl));
        }
    }
    None
}

/// Extract equality: SApp (SApp (SName "Eq") lhs) rhs
fn extract_equality(term: &Term) -> Option<(Term, Term)> {
    if let Some((op, lhs, rhs)) = extract_binary_app(term) {
        if op == "Eq" || op == "eq" {
            return Some((lhs, rhs));
        }
    }
    None
}

/// Extract binary application: SApp (SApp (SName "op") a) b
fn extract_binary_app(term: &Term) -> Option<(String, Term, Term)> {
    if let Term::App(outer, b) = term {
        if let Term::App(sapp_outer, inner) = outer.as_ref() {
            if let Term::Global(ctor) = sapp_outer.as_ref() {
                if ctor == "SApp" {
                    if let Term::App(partial, a) = inner.as_ref() {
                        if let Term::App(sapp_inner, op_term) = partial.as_ref() {
                            if let Term::Global(ctor2) = sapp_inner.as_ref() {
                                if ctor2 == "SApp" {
                                    if let Some(op) = extract_sname(op_term) {
                                        return Some((op, a.as_ref().clone(), b.as_ref().clone()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

// =============================================================================
// UNIT TESTS
// =============================================================================

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/cc/tests.rs"]
mod tests;
