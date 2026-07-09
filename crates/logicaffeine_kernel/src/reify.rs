//! Shared reification substrate for the arithmetic decision procedures.
//!
//! `ring`, `lia`, and `omega` all read the same deep embedding of terms
//! (`SLit`/`SVar`/`SName`/`SApp` applied via kernel `App` nodes). The pattern
//! extractors live here once, and so does [`VarInterner`], which maps named
//! globals to variable indices.
//!
//! # Why an interner
//!
//! Named globals must reify to variable indices that are (a) distinct for
//! distinct names and (b) stable across every term reified for one goal —
//! the left- and right-hand sides of an equation, or a hypothesis set and its
//! conclusion. A hash of the name satisfies (b) but not (a): a collision
//! identifies two different variables and lets the procedure prove `Aa = BB`.
//! The interner satisfies both by construction; callers create one per goal
//! and thread it through every reification belonging to that goal.

use std::collections::HashMap;

use crate::term::{Literal, Term};

/// Per-goal map from global names to variable indices.
///
/// Indices are negative, starting far from zero, so they can never collide
/// with `SVar` de Bruijn indices (which are non-negative).
#[derive(Debug, Default)]
pub struct VarInterner {
    map: HashMap<String, i64>,
    next: i64,
}

impl VarInterner {
    pub fn new() -> Self {
        VarInterner {
            map: HashMap::new(),
            next: -1_000_000,
        }
    }

    /// The index for `name`, allocating a fresh one on first sight.
    pub fn intern(&mut self, name: &str) -> i64 {
        if let Some(&idx) = self.map.get(name) {
            return idx;
        }
        let idx = self.next;
        self.next -= 1;
        self.map.insert(name.to_string(), idx);
        idx
    }
}

/// Extract integer from `SLit n`.
pub(crate) fn extract_slit(term: &Term) -> Option<i64> {
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

/// Extract variable index from `SVar i`.
pub(crate) fn extract_svar(term: &Term) -> Option<i64> {
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

/// Extract name from `SName "x"`.
pub(crate) fn extract_sname(term: &Term) -> Option<String> {
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

/// Extract binary application: `SApp (SApp (SName "op") a) b`.
pub(crate) fn extract_binary_app(term: &Term) -> Option<(String, Term, Term)> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interner_distinct_names_distinct_indices() {
        let mut it = VarInterner::new();
        let a = it.intern("Aa");
        let b = it.intern("BB");
        assert_ne!(a, b);
    }

    #[test]
    fn interner_same_name_same_index() {
        let mut it = VarInterner::new();
        assert_eq!(it.intern("x"), it.intern("x"));
    }

    #[test]
    fn interner_indices_negative_below_svar_range() {
        let mut it = VarInterner::new();
        assert!(it.intern("x") <= -1_000_000);
    }
}
