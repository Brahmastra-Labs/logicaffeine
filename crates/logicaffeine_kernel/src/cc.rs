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

// =============================================================================
// UNION-FIND DATA STRUCTURE
// =============================================================================

/// Union-Find data structure with path compression and union by rank.
///
/// Maintains equivalence classes over term IDs. Supports near-constant time
/// operations for `find` (amortized) and `union`.
pub struct UnionFind {
    /// Parent pointer for each element (element is its own parent if root).
    parent: Vec<TermId>,
    /// Rank (approximate tree depth) for union by rank optimization.
    rank: Vec<usize>,
}

impl UnionFind {
    pub fn new() -> Self {
        UnionFind {
            parent: Vec::new(),
            rank: Vec::new(),
        }
    }

    /// Add a new element, returns its ID
    pub fn make_set(&mut self) -> TermId {
        let id = self.parent.len();
        self.parent.push(id);
        self.rank.push(0);
        id
    }

    /// Find representative with path compression
    pub fn find(&mut self, x: TermId) -> TermId {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }

    /// Union by rank, returns true if a merge occurred
    pub fn union(&mut self, x: TermId, y: TermId) -> bool {
        let rx = self.find(x);
        let ry = self.find(y);
        if rx == ry {
            return false;
        }

        if self.rank[rx] < self.rank[ry] {
            self.parent[rx] = ry;
        } else if self.rank[rx] > self.rank[ry] {
            self.parent[ry] = rx;
        } else {
            self.parent[ry] = rx;
            self.rank[rx] += 1;
        }
        true
    }
}

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
                                        return Some((
                                            op,
                                            a.as_ref().clone(),
                                            b.as_ref().clone(),
                                        ));
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
mod tests {
    use super::*;

    #[test]
    fn test_union_find_basic() {
        let mut uf = UnionFind::new();
        let a = uf.make_set();
        let b = uf.make_set();
        assert_ne!(uf.find(a), uf.find(b));
        uf.union(a, b);
        assert_eq!(uf.find(a), uf.find(b));
    }

    #[test]
    fn test_union_find_transitivity() {
        let mut uf = UnionFind::new();
        let a = uf.make_set();
        let b = uf.make_set();
        let c = uf.make_set();
        uf.union(a, b);
        uf.union(b, c);
        assert_eq!(uf.find(a), uf.find(c));
    }

    #[test]
    fn test_egraph_reflexive() {
        let mut eg = EGraph::new();
        let x = eg.add(ENode::Var(0));
        assert!(eg.equivalent(x, x));
    }

    #[test]
    fn test_egraph_congruence() {
        let mut eg = EGraph::new();
        let x = eg.add(ENode::Var(0));
        let y = eg.add(ENode::Var(1));
        let f = eg.add(ENode::Name("f".to_string()));
        let fx = eg.add(ENode::App { func: f, arg: x });
        let fy = eg.add(ENode::App { func: f, arg: y });

        // Before merging x=y, f(x) != f(y)
        assert!(!eg.equivalent(fx, fy));

        // After merging x=y, congruence gives f(x) = f(y)
        eg.merge(x, y);
        assert!(eg.equivalent(fx, fy));
    }

    #[test]
    fn test_egraph_nested_congruence() {
        let mut eg = EGraph::new();
        let a = eg.add(ENode::Var(0));
        let b = eg.add(ENode::Var(1));
        let c = eg.add(ENode::Var(2));
        let f = eg.add(ENode::Name("f".to_string()));

        let fa = eg.add(ENode::App { func: f, arg: a });
        let fc = eg.add(ENode::App { func: f, arg: c });
        let ffa = eg.add(ENode::App { func: f, arg: fa });
        let ffc = eg.add(ENode::App { func: f, arg: fc });

        // a = b, b = c should give f(f(a)) = f(f(c))
        eg.merge(a, b);
        eg.merge(b, c);
        assert!(eg.equivalent(ffa, ffc));
    }

    #[test]
    fn test_egraph_binary_congruence() {
        let mut eg = EGraph::new();
        let a = eg.add(ENode::Var(0));
        let b = eg.add(ENode::Var(1));
        let c = eg.add(ENode::Var(2));
        let add = eg.add(ENode::Name("add".to_string()));

        // add(a, c) and add(b, c) as curried: add a c = (add a) c
        let add_a = eg.add(ENode::App { func: add, arg: a });
        let add_b = eg.add(ENode::App { func: add, arg: b });
        let add_a_c = eg.add(ENode::App { func: add_a, arg: c });
        let add_b_c = eg.add(ENode::App { func: add_b, arg: c });

        assert!(!eg.equivalent(add_a_c, add_b_c));
        eg.merge(a, b);
        assert!(eg.equivalent(add_a_c, add_b_c));
    }

    // =========================================================================
    // EXTRACTION TESTS
    // =========================================================================

    /// Helper to build SName "s"
    fn make_sname(s: &str) -> Term {
        Term::App(
            Box::new(Term::Global("SName".to_string())),
            Box::new(Term::Lit(Literal::Text(s.to_string()))),
        )
    }

    /// Helper to build SVar i
    fn make_svar(i: i64) -> Term {
        Term::App(
            Box::new(Term::Global("SVar".to_string())),
            Box::new(Term::Lit(Literal::Int(i))),
        )
    }

    /// Helper to build SApp f a
    fn make_sapp(f: Term, a: Term) -> Term {
        Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(f),
            )),
            Box::new(a),
        )
    }

    #[test]
    fn test_extract_sname() {
        let term = make_sname("f");
        assert_eq!(extract_sname(&term), Some("f".to_string()));
    }

    #[test]
    fn test_extract_svar() {
        let term = make_svar(0);
        assert_eq!(extract_svar(&term), Some(0));
    }

    #[test]
    fn test_extract_sapp() {
        // SApp (SName "f") (SVar 0)
        let term = make_sapp(make_sname("f"), make_svar(0));
        let result = extract_sapp(&term);
        assert!(result.is_some());
        let (func, arg) = result.unwrap();
        assert_eq!(extract_sname(&func), Some("f".to_string()));
        assert_eq!(extract_svar(&arg), Some(0));
    }

    #[test]
    fn test_extract_binary_app() {
        // SApp (SApp (SName "Eq") (SVar 0)) (SVar 1)
        let term = make_sapp(make_sapp(make_sname("Eq"), make_svar(0)), make_svar(1));
        let result = extract_binary_app(&term);
        assert!(result.is_some(), "Should extract binary app");
        let (op, a, b) = result.unwrap();
        assert_eq!(op, "Eq");
        assert_eq!(extract_svar(&a), Some(0));
        assert_eq!(extract_svar(&b), Some(1));
    }

    #[test]
    fn test_extract_equality() {
        // SApp (SApp (SName "Eq") (SVar 0)) (SVar 1)
        let term = make_sapp(make_sapp(make_sname("Eq"), make_svar(0)), make_svar(1));
        let result = extract_equality(&term);
        assert!(result.is_some(), "Should extract equality");
        let (lhs, rhs) = result.unwrap();
        assert_eq!(extract_svar(&lhs), Some(0));
        assert_eq!(extract_svar(&rhs), Some(1));
    }

    #[test]
    fn test_extract_implication() {
        // Build: SApp (SApp (SName "implies") hyp) concl
        // hyp = SApp (SApp (SName "Eq") x) y
        // concl = SApp (SApp (SName "Eq") fx) fy
        let x = make_svar(0);
        let y = make_svar(1);
        let hyp = make_sapp(make_sapp(make_sname("Eq"), x.clone()), y.clone());

        let f = make_sname("f");
        let fx = make_sapp(f.clone(), x);
        let fy = make_sapp(f, y);
        let concl = make_sapp(make_sapp(make_sname("Eq"), fx), fy);

        let goal = make_sapp(make_sapp(make_sname("implies"), hyp.clone()), concl.clone());

        let result = extract_implication(&goal);
        assert!(result.is_some(), "Should extract implication");
        let (hyp_extracted, concl_extracted) = result.unwrap();

        // Verify hypothesis is the equality x = y
        let hyp_eq = extract_equality(&hyp_extracted);
        assert!(hyp_eq.is_some(), "Hypothesis should be equality");
        let (h_lhs, h_rhs) = hyp_eq.unwrap();
        assert_eq!(extract_svar(&h_lhs), Some(0));
        assert_eq!(extract_svar(&h_rhs), Some(1));

        // Verify conclusion is an equality
        let concl_eq = extract_equality(&concl_extracted);
        assert!(concl_eq.is_some(), "Conclusion should be equality");
    }

    #[test]
    fn test_decompose_goal_with_hypothesis() {
        // Build: (implies (Eq x y) (Eq (f x) (f y)))
        let x = make_svar(0);
        let y = make_svar(1);
        let hyp = make_sapp(make_sapp(make_sname("Eq"), x.clone()), y.clone());

        let f = make_sname("f");
        let fx = make_sapp(f.clone(), x);
        let fy = make_sapp(f, y);
        let concl = make_sapp(make_sapp(make_sname("Eq"), fx), fy);

        let goal = make_sapp(make_sapp(make_sname("implies"), hyp), concl);

        let (hypotheses, conclusion) = decompose_goal(&goal);
        assert_eq!(hypotheses.len(), 1, "Should have 1 hypothesis");

        // Verify conclusion is an equality
        let concl_eq = extract_equality(&conclusion);
        assert!(concl_eq.is_some(), "Conclusion should be equality");
    }

    #[test]
    fn test_check_goal_with_hypothesis() {
        // Build: (implies (Eq x y) (Eq (f x) (f y)))
        // This should be provable by CC
        let x = make_svar(0);
        let y = make_svar(1);
        let hyp = make_sapp(make_sapp(make_sname("Eq"), x.clone()), y.clone());

        let f = make_sname("f");
        let fx = make_sapp(f.clone(), x.clone());
        let fy = make_sapp(f.clone(), y.clone());
        let concl = make_sapp(make_sapp(make_sname("Eq"), fx), fy);

        let goal = make_sapp(make_sapp(make_sname("implies"), hyp), concl);

        assert!(check_goal(&goal), "CC should prove x=y → f(x)=f(y)");
    }
}
