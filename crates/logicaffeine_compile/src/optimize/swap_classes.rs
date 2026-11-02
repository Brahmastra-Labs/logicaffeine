//! The names `Swap` ties to one representation.
//!
//! After `Swap a and b.` each name holds what the other held. A representation the compiler picks for one of them
//! (scalars in place of a list, a `[T; N]` stack array, a plain `Vec` in place of a shared one, `Vec<i32>`, a
//! specialized map, a byte for one character of text) has to fit the other's value too, or the exchange no longer
//! type-checks and the values no longer fit. Swaps chain: `Swap a and b.` then `Swap b and c.` put all three names in
//! one class.
//!
//! A detection that picks a representation closes its claims under these classes before it returns them.
//! [`SwapClasses::close`] keeps a claim only when every name of its class carries an equal claim: two fixed lists of
//! one length stay fixed when swapped, and a fixed list swapped with a growing one falls back with it.
//! [`SwapClasses::exclude`] drops every claim on a swapped name, for the representations a swap cannot exchange: an
//! array deleted in favour of its closed form, a constant table hoisted as an immutable, a worklist's register tail.
//! Claims closed at their source compose: a `retain` of one closed claim set against another removes whole classes,
//! so both stay closed. A property that must hold of every value a name may hold (a big integer stored as
//! `LogosInt`) runs the other way: it spreads across a swap, over the pairs [`swapped_pairs`] reports.
//!
//! The classes come from the optimizer's one exhaustive effect walk ([`super::writes`]), which reports each `Swap`
//! as an exchange, so one nested in any block or closure is seen. They are keyed by NAME: the parser may mint a
//! distinct symbol per occurrence. A nested function definition is its own scope and is not entered; its body's
//! classes are found when that body is analysed.

use std::collections::{HashMap, HashSet};

use crate::ast::stmt::{ClosureBody, Stmt};
use crate::intern::{Interner, Symbol};

use super::writes::{walk_block, walk_expr, Bind, EffectSink};

/// The swap classes of one scope: each name some `Swap` exchanges, and the names it shares a class with.
#[derive(Debug, Default)]
pub(crate) struct SwapClasses {
    class_of: HashMap<String, usize>,
    members: Vec<Vec<String>>,
}

/// Collects the symbol pairs a block's swaps exchange.
#[derive(Default)]
struct Exchanges {
    pairs: Vec<(Symbol, Symbol)>,
}

impl EffectSink for Exchanges {
    fn bind(&mut self, _sym: Symbol, _how: Bind) {}
    fn mutate(&mut self, _root: Symbol) {}
    fn call(&mut self) {}
    fn opaque(&mut self) {}
    fn exchange(&mut self, a: Symbol, b: Symbol) {
        self.pairs.push((a, b));
    }
    /// A closure swaps its own captured copies, which carry the representations of the names they copy.
    fn closure(&mut self, _params: &[Symbol], body: &ClosureBody) {
        match body {
            ClosureBody::Expression(e) => walk_expr(e, self),
            ClosureBody::Block(b) => walk_block(b, self),
        }
    }
}

impl SwapClasses {
    /// The swap classes of `block`, nested blocks and closures included.
    pub(crate) fn of(block: &[Stmt], interner: &Interner) -> Self {
        let pairs = swapped_pairs(block)
            .into_iter()
            .map(|(a, b)| {
                (
                    interner.resolve(a).to_string(),
                    interner.resolve(b).to_string(),
                )
            })
            .collect();
        Self::from_pairs(pairs)
    }

    fn from_pairs(pairs: Vec<(String, String)>) -> Self {
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut parent: Vec<usize> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        let mut id = |name: String, parent: &mut Vec<usize>, names: &mut Vec<String>| -> usize {
            *index.entry(name.clone()).or_insert_with(|| {
                parent.push(parent.len());
                names.push(name);
                parent.len() - 1
            })
        };
        for (a, b) in pairs {
            let (ia, ib) = (
                id(a, &mut parent, &mut names),
                id(b, &mut parent, &mut names),
            );
            let (ra, rb) = (root(&mut parent, ia), root(&mut parent, ib));
            if ra != rb {
                parent[ra] = rb;
            }
        }
        let mut class_ids: HashMap<usize, usize> = HashMap::new();
        let mut classes = SwapClasses::default();
        for (i, name) in names.into_iter().enumerate() {
            let r = root(&mut parent, i);
            let c = *class_ids.entry(r).or_insert_with(|| {
                classes.members.push(Vec::new());
                classes.members.len() - 1
            });
            classes.members[c].push(name.clone());
            classes.class_of.insert(name, c);
        }
        classes
    }

    /// Whether no `Swap` appears in the scope.
    pub(crate) fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Whether a `Swap` exchanges the name `name` (with another name, or with itself).
    pub(crate) fn is_swapped(&self, name: &str) -> bool {
        self.class_of.contains_key(name)
    }

    /// The names `name` shares a class with, itself included; empty for a name no `Swap` exchanges.
    pub(crate) fn class_of(&self, name: &str) -> &[String] {
        self.class_of.get(name).map_or(&[], |&c| &self.members[c])
    }

    /// Keep a claim only when every name of its class carries a claim `same` as it, dropping claims until that
    /// holds. A claim on a name no `Swap` exchanges is kept as it is.
    pub(crate) fn close_by<V>(
        &self,
        claims: &mut HashMap<Symbol, V>,
        interner: &Interner,
        same: impl Fn(&V, &V) -> bool,
    ) {
        if self.is_empty() {
            return;
        }
        loop {
            let mut by_name: HashMap<&str, Vec<&V>> = HashMap::new();
            for (s, v) in claims.iter() {
                by_name.entry(interner.resolve(*s)).or_default().push(v);
            }
            let dropped: Vec<Symbol> = claims
                .iter()
                .filter(|(s, v)| {
                    !self.class_of(interner.resolve(**s)).iter().all(|m| {
                        by_name
                            .get(m.as_str())
                            .is_some_and(|vs| vs.iter().any(|w| same(v, w)))
                    })
                })
                .map(|(s, _)| *s)
                .collect();
            if dropped.is_empty() {
                return;
            }
            for s in dropped {
                claims.remove(&s);
            }
        }
    }

    /// [`Self::close_by`] with equality as the test.
    pub(crate) fn close<V: PartialEq>(&self, claims: &mut HashMap<Symbol, V>, interner: &Interner) {
        self.close_by(claims, interner, |a, b| a == b);
    }

    /// [`Self::close`] for a claim that carries no information beyond the name.
    pub(crate) fn close_set(&self, claims: &mut HashSet<Symbol>, interner: &Interner) {
        if self.is_empty() {
            return;
        }
        let mut keyed: HashMap<Symbol, ()> = claims.iter().map(|s| (*s, ())).collect();
        self.close(&mut keyed, interner);
        claims.retain(|s| keyed.contains_key(s));
    }

    /// Drop every claim on a name some `Swap` exchanges.
    pub(crate) fn exclude<V>(&self, claims: &mut HashMap<Symbol, V>, interner: &Interner) {
        if self.is_empty() {
            return;
        }
        claims.retain(|s, _| !self.is_swapped(interner.resolve(*s)));
    }

    /// [`Self::exclude`] for a claim that carries no information beyond the name.
    pub(crate) fn exclude_set(&self, claims: &mut HashSet<Symbol>, interner: &Interner) {
        if self.is_empty() {
            return;
        }
        claims.retain(|s| !self.is_swapped(interner.resolve(*s)));
    }
}

/// The pairs every `Swap` in `block` exchanges (nested blocks and closures included), by the symbols the occurrence
/// carries: for an analysis that tracks bindings by symbol and must SPREAD a property across a swap rather than
/// require it of both sides (a name that may hold a big integer hands that value to its partner).
pub(crate) fn swapped_pairs(block: &[Stmt]) -> Vec<(Symbol, Symbol)> {
    let mut found = Exchanges::default();
    walk_block(block, &mut found);
    found.pairs
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/swap_classes/tests.rs"]
mod tests;
