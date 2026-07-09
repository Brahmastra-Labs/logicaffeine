//! First-order discrimination tree: the shared pattern index under `simp`
//! rule lookup, `exact?`/`apply?` library search, and `crush`'s E-matching.
//!
//! # How it works
//!
//! A pattern is flattened to its preorder key sequence — head symbols carrying
//! their arity, with `*` at each pattern-variable position — and inserted into
//! a trie. Retrieval walks the trie in step with the query's own flattening:
//! a symbol edge must match the query symbol exactly; a `*` edge skips one
//! complete query subterm (the arity annotations make subterm extents
//! computable without re-walking the term).
//!
//! # The contract
//!
//! Retrieval NEVER MISSES: every inserted pattern that one-sided matches the
//! query (per [`crate::unify::match_term_pattern`]) is among the candidates.
//! It may over-approximate — variable positions are treated independently, so
//! the non-linear pattern `f(x, x)` is retrieved for `f(a, b)` — and the
//! matcher is the arbiter. The value of the tree is pruning: a query touches
//! only patterns sharing its head structure, not the whole rule set.
//!
//! Quantified subexpressions flatten to `*` on both sides (opaque), which
//! preserves never-miss at the cost of extra candidates — the conservative
//! choice for shapes the matcher itself treats conservatively.

use std::collections::BTreeMap;

use crate::{ProofExpr, ProofTerm};

/// One symbol in a flattened pattern/query.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    /// A head symbol with its arity. The `String` is namespaced by kind
    /// (predicate/function/constant/connective) so same-named symbols of
    /// different kinds never collide.
    Sym(String, usize),
    /// A variable position — spans one complete subterm.
    Star,
}

struct Node<T> {
    children: BTreeMap<Key, Node<T>>,
    /// Values whose pattern's key sequence ends exactly here.
    values: Vec<T>,
}

impl<T> Default for Node<T> {
    fn default() -> Self {
        Node { children: BTreeMap::new(), values: Vec::new() }
    }
}

/// A discrimination tree mapping term/expression patterns to payloads.
///
/// Term and expression patterns share one tree — their key alphabets are
/// disjoint by construction, so they never cross-match.
pub struct DiscTree<T> {
    root: Node<T>,
}

impl<T> Default for DiscTree<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> DiscTree<T> {
    pub fn new() -> Self {
        DiscTree { root: Node::default() }
    }

    /// Index a term pattern (its `Variable`s are the wildcard positions).
    pub fn insert_term(&mut self, pattern: &ProofTerm, value: T) {
        let mut keys = Vec::new();
        flatten_term(pattern, &mut keys);
        self.insert_keys(&keys, value);
    }

    /// Index an expression pattern (its `Variable`s are the wildcard positions).
    pub fn insert_expr(&mut self, pattern: &ProofExpr, value: T) {
        let mut keys = Vec::new();
        flatten_expr(pattern, &mut keys);
        self.insert_keys(&keys, value);
    }

    /// Every pattern that could one-sided match the query term (superset —
    /// confirm with [`crate::unify::match_term_pattern`]).
    pub fn candidates_term(&self, query: &ProofTerm) -> Vec<&T> {
        let mut keys = Vec::new();
        flatten_term(query, &mut keys);
        self.candidates_keys(&keys)
    }

    /// Every pattern that could one-sided match the query expression
    /// (superset — confirm with [`crate::unify::match_expr_pattern`]).
    pub fn candidates_expr(&self, query: &ProofExpr) -> Vec<&T> {
        let mut keys = Vec::new();
        flatten_expr(query, &mut keys);
        self.candidates_keys(&keys)
    }

    fn insert_keys(&mut self, keys: &[Key], value: T) {
        let mut node = &mut self.root;
        for key in keys {
            node = node.children.entry(key.clone()).or_default();
        }
        node.values.push(value);
    }

    fn candidates_keys(&self, keys: &[Key]) -> Vec<&T> {
        let jump = subterm_jumps(keys);
        let mut out = Vec::new();
        retrieve(&self.root, keys, &jump, 0, &mut out);
        out
    }
}

/// `jump[i]` = the index one past the subterm that starts at position `i`.
fn subterm_jumps(keys: &[Key]) -> Vec<usize> {
    let mut jump = vec![0usize; keys.len()];
    fill_jumps(keys, 0, &mut jump);
    jump
}

fn fill_jumps(keys: &[Key], i: usize, jump: &mut [usize]) -> usize {
    let end = match &keys[i] {
        Key::Star => i + 1,
        Key::Sym(_, arity) => {
            let mut j = i + 1;
            for _ in 0..*arity {
                j = fill_jumps(keys, j, jump);
            }
            j
        }
    };
    jump[i] = end;
    end
}

fn retrieve<'a, T>(
    node: &'a Node<T>,
    keys: &[Key],
    jump: &[usize],
    i: usize,
    out: &mut Vec<&'a T>,
) {
    if i == keys.len() {
        out.extend(node.values.iter());
        return;
    }
    match &keys[i] {
        sym @ Key::Sym(..) => {
            // A pattern with the same head continues in lockstep.
            if let Some(child) = node.children.get(sym) {
                retrieve(child, keys, jump, i + 1, out);
            }
            // A pattern variable here swallows this whole query subterm.
            if let Some(star) = node.children.get(&Key::Star) {
                retrieve(star, keys, jump, jump[i], out);
            }
        }
        Key::Star => {
            // The query is opaque here (a query variable or a shape flattened
            // conservatively): any complete pattern subterm may align with it.
            let mut landings = Vec::new();
            skip_pattern_subterm(node, 1, &mut landings);
            for landing in landings {
                retrieve(landing, keys, jump, i + 1, out);
            }
        }
    }
}

/// All trie nodes exactly one complete pattern subterm below `node`.
fn skip_pattern_subterm<'a, T>(node: &'a Node<T>, pending: usize, out: &mut Vec<&'a Node<T>>) {
    if pending == 0 {
        out.push(node);
        return;
    }
    for (key, child) in &node.children {
        let need = pending - 1
            + match key {
                Key::Sym(_, arity) => *arity,
                Key::Star => 0,
            };
        skip_pattern_subterm(child, need, out);
    }
}

fn flatten_term(t: &ProofTerm, out: &mut Vec<Key>) {
    match t {
        ProofTerm::Variable(_) => out.push(Key::Star),
        ProofTerm::Constant(s) => out.push(Key::Sym(format!("c:{s}"), 0)),
        ProofTerm::BoundVarRef(s) => out.push(Key::Sym(format!("b:{s}"), 0)),
        ProofTerm::Function(name, args) => {
            out.push(Key::Sym(format!("f:{name}"), args.len()));
            for a in args {
                flatten_term(a, out);
            }
        }
        ProofTerm::Group(args) => {
            out.push(Key::Sym("(,)".to_string(), args.len()));
            for a in args {
                flatten_term(a, out);
            }
        }
    }
}

fn flatten_expr(e: &ProofExpr, out: &mut Vec<Key>) {
    match e {
        ProofExpr::Predicate { name, args, .. } => {
            out.push(Key::Sym(format!("p:{name}"), args.len()));
            for a in args {
                flatten_term(a, out);
            }
        }
        ProofExpr::Identity(l, r) => {
            out.push(Key::Sym("=".to_string(), 2));
            flatten_term(l, out);
            flatten_term(r, out);
        }
        ProofExpr::Atom(s) => out.push(Key::Sym(format!("a:{s}"), 0)),
        ProofExpr::And(l, r) => {
            out.push(Key::Sym("∧".to_string(), 2));
            flatten_expr(l, out);
            flatten_expr(r, out);
        }
        ProofExpr::Or(l, r) => {
            out.push(Key::Sym("∨".to_string(), 2));
            flatten_expr(l, out);
            flatten_expr(r, out);
        }
        ProofExpr::Implies(l, r) => {
            out.push(Key::Sym("→".to_string(), 2));
            flatten_expr(l, out);
            flatten_expr(r, out);
        }
        ProofExpr::Iff(l, r) => {
            out.push(Key::Sym("↔".to_string(), 2));
            flatten_expr(l, out);
            flatten_expr(r, out);
        }
        ProofExpr::Not(p) => {
            out.push(Key::Sym("¬".to_string(), 1));
            flatten_expr(p, out);
        }
        // Binders and every other shape are opaque: `*` preserves never-miss
        // (the matcher treats them conservatively too).
        _ => out.push(Key::Star),
    }
}
