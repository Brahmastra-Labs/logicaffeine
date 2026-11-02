//! What a compile has proven about its own nodes, held exactly as long as that compile runs.
//!
//! A pass that constructs a proof records it here against the node it holds for: an Int op its guard keeps in
//! i64, an access a versioning guard or a literal range puts inside its sequence, a nest that may run in tile
//! order, an expression to hide from LLVM's unroller. A backend lowering that node asks. Nodes live in the
//! arenas a compile makes, and their addresses are reused once those arenas are freed, so the facts live in a
//! [`scope`] the compile opens where it makes its arenas and drop with it.
//!
//! A mark goes to the innermost open scope; a question is answered from every open scope. Every open scope
//! belongs to a compile still running, whose arenas are still alive, so no two of their nodes share an address
//! and an outer compile's facts can never answer for an inner compile's node. Outside every scope a mark records
//! nothing and every question answers no: a lowering then keeps its checks, which costs speed and never
//! soundness. Membership only; nothing here is iterated.

use std::cell::RefCell;
use std::collections::HashSet;
use std::marker::PhantomData;
use std::rc::Rc;

use crate::ast::stmt::{Expr, Stmt};

/// A kind of fact a node can hold.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Fact {
    /// This Int op never leaves i64 on any run.
    RawIntOp,
    /// An exact affine advance synthesized by integer induction strength reduction.
    /// A backend must guard the whole progression before using native words.
    IntStrengthAdvance,
    /// Emit this expression inside `std::hint::black_box(...)`, a runtime identity.
    BlackBox,
    /// The access whose index is this node names an item wherever it is evaluated.
    InBounds,
    /// The loop nest headed by this statement may run its iterations in tile order.
    TileLegal,
    /// This `Repeat` counts through a range whose bounds are Ints ([`super::iteration`]).
    IntRange,
    /// This `Repeat` may walk its sequence in place: nothing the loop runs can change it, so the sequence is the
    /// snapshot it iterates ([`super::iteration`]).
    LentIteration,
    /// This `nothing` is what a place lent to a `mutable` parameter holds while the call holds its value
    /// ([`super::sequence`]): a store of it fits a place of any type.
    Vacated,
}

impl Fact {
    /// Which arena owns the node this fact describes.
    ///
    /// Keep this match exhaustive: adding a fact must classify it before the
    /// compiler can build, so AST rewrites cannot silently omit a new kind.
    const fn is_expression(self) -> bool {
        match self {
            Self::RawIntOp
            | Self::IntStrengthAdvance
            | Self::BlackBox
            | Self::InBounds
            | Self::TileLegal
            | Self::Vacated => true,
            Self::IntRange | Self::LentIteration => false,
        }
    }
}

thread_local! {
    static SCOPES: RefCell<Vec<HashSet<(Fact, usize)>>> = const { RefCell::new(Vec::new()) };
}

/// The facts of one compile, open until dropped ([`scope`]).
#[must_use = "the facts end when the scope is dropped"]
pub struct Scope {
    depth: usize,
    // Dropping the guard must close the same thread-local stack that created it.
    thread: PhantomData<Rc<()>>,
}

/// Open the facts scope of a compile. Opened where the compile makes its arenas, it drops with them.
pub fn scope() -> Scope {
    SCOPES.with(|s| {
        let mut s = s.borrow_mut();
        s.push(HashSet::new());
        Scope {
            depth: s.len(),
            thread: PhantomData,
        }
    })
}

impl Drop for Scope {
    fn drop(&mut self) {
        SCOPES.with(|s| s.borrow_mut().truncate(self.depth - 1));
    }
}

fn key<T>(node: &T) -> usize {
    node as *const T as usize
}

fn mark<T>(fact: Fact, node: &T) {
    SCOPES.with(|s| {
        if let Some(top) = s.borrow_mut().last_mut() {
            top.insert((fact, key(node)));
        }
    });
}

fn unmark<T>(fact: Fact, node: &T) -> bool {
    SCOPES.with(|s| {
        s.borrow_mut()
            .iter_mut()
            .rev()
            .any(|facts| facts.remove(&(fact, key(node))))
    })
}

fn holds<T>(fact: Fact, node: &T) -> bool {
    SCOPES.with(|s| {
        s.borrow()
            .iter()
            .any(|facts| facts.contains(&(fact, key(node))))
    })
}

fn copy_expression_facts(from: &Expr<'_>, to: &Expr<'_>) {
    let from = key(from);
    let to = key(to);
    if from == to {
        return;
    }
    SCOPES.with(|scopes| {
        for facts in scopes.borrow_mut().iter_mut() {
            let copied: Vec<_> = facts
                .iter()
                .filter_map(|(fact, node)| {
                    (*node == from && fact.is_expression()).then_some((*fact, to))
                })
                .collect();
            facts.extend(copied);
        }
    });
}

fn assert_expr_fact(fact: Fact) {
    debug_assert!(
        fact.is_expression(),
        "{fact:?} is a statement fact, not an expression fact"
    );
}

fn assert_stmt_fact(fact: Fact) {
    debug_assert!(
        !fact.is_expression(),
        "{fact:?} is an expression fact, not a statement fact"
    );
}

/// Record that expression `node` holds `fact` in the innermost open scope.
///
/// This deliberately takes `&Expr` instead of an unconstrained `&T`. A caller
/// holding `&&Expr` is therefore dereferenced to the arena node rather than
/// silently keying the address of a reference slot.
pub fn mark_expr(fact: Fact, node: &Expr<'_>) {
    assert_expr_fact(fact);
    mark(fact, node);
}

/// Take expression `fact` off `node`, answering whether a scope held it.
pub fn unmark_expr(fact: Fact, node: &Expr<'_>) -> bool {
    assert_expr_fact(fact);
    unmark(fact, node)
}

/// Whether expression `node` holds `fact` in some open scope.
pub fn holds_expr(fact: Fact, node: &Expr<'_>) -> bool {
    assert_expr_fact(fact);
    holds(fact, node)
}

/// Copy every expression fact on `from` to the equivalent node `to`.
///
/// Rebuilding AST passes use this when they change representation or type
/// names without changing value or control meaning. Facts remain in the scope
/// that proved them, and future expression facts participate automatically.
pub fn copy_expr_facts(from: &Expr<'_>, to: &Expr<'_>) {
    copy_expression_facts(from, to);
}

/// Record that statement `node` holds `fact` in the innermost open scope.
pub fn mark_stmt(fact: Fact, node: &Stmt<'_>) {
    assert_stmt_fact(fact);
    mark(fact, node);
}

/// Whether statement `node` holds `fact` in some open scope.
pub fn holds_stmt(fact: Fact, node: &Stmt<'_>) -> bool {
    assert_stmt_fact(fact);
    holds(fact, node)
}

/// How many scopes are open now.
pub fn open_scopes() -> usize {
    SCOPES.with(|s| s.borrow().len())
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/facts/tests.rs"]
mod tests;
