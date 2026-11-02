//! Free-name capture — the one question an inliner asks before it substitutes a callee's body.
//!
//! A function's frame sees its parameters and its own locals, then the program's bindings: never its
//! caller's locals (`Interpreter::push_frame`). A callee's FREE names — the program bindings it reads or
//! sets, and names it reads that nothing binds — are therefore resolved where the callee is declared. An
//! inliner that moves the body to a call site resolves them there instead, and a local of the same name in
//! the caller's frame captures them: `## To peek -> Int: Return x.` inlined into a function with its own
//! `Let x be 7` answers 7 where the interpreter answers the program's `x`.
//!
//! So an inliner substitutes a callee only where [`captures`] says no: none of the callee's
//! [`free_names`] is bound in the frame of the call site ([`frame_names`] for a function, and
//! [`main_frame_names`] for `## Main`, whose own root bindings ARE the program's).
//!
//! A pass that SPLICES a block into its parent (a branch whose condition is known, a peeled loop
//! iteration) asks the converse question: the block's [`own_names`] would outlive it — shadowing the
//! parent's names for the statements after it, or, at Main's root, becoming program bindings every function
//! reads. A pass that carries what it knows of each name (constant propagation) asks a third: which names a
//! statement binds ([`own_names`]) and which a block nested in it binds on entry ([`EntryBinders`]) — a
//! binding of a name ends whatever was known of the name it shadows. Every question comes from
//! [`super::writes`]' exhaustive walk, so a binder a new statement kind introduces is counted the day it
//! compiles.

use std::collections::HashSet;

use logicaffeine_language::ast_depth::{for_each_node, Visited};

use crate::ast::stmt::{ClosureBody, Expr, Stmt};
use crate::intern::Symbol;

use super::writes::{walk_block, walk_expr, Bind, EffectSink};

/// The names a block declares: at the block's own level, in the blocks nested in it (closures included), or
/// both.
struct Declared {
    names: HashSet<Symbol>,
    /// Blocks entered below the one the walk started in.
    depth: usize,
    /// Whether a name declared at the starting block itself counts.
    own: bool,
    /// Whether a name declared in a nested block counts.
    nested: bool,
}

impl EffectSink for Declared {
    fn bind(&mut self, sym: Symbol, how: Bind) {
        let counts = if self.depth == 0 {
            self.own
        } else {
            self.nested
        };
        if how == Bind::Declare && counts {
            self.names.insert(sym);
        }
    }
    fn mutate(&mut self, _root: Symbol) {}
    fn call(&mut self) {}
    fn opaque(&mut self) {}
    fn scoped(&mut self, binders: &[Symbol], body: &[Stmt]) {
        if !self.nested {
            return;
        }
        self.depth += 1;
        for b in binders {
            self.bind(*b, Bind::Declare);
        }
        walk_block(body, self);
        self.depth -= 1;
    }
    fn closure(&mut self, params: &[Symbol], body: &ClosureBody) {
        if !self.nested {
            return;
        }
        self.depth += 1;
        self.names.extend(params.iter().copied());
        match body {
            ClosureBody::Expression(e) => walk_expr(e, self),
            ClosureBody::Block(b) => walk_block(b, self),
        }
        self.depth -= 1;
    }
}

fn declared(body: &[Stmt], own: bool, nested: bool) -> HashSet<Symbol> {
    let mut d = Declared {
        names: HashSet::new(),
        depth: 0,
        own,
        nested,
    };
    walk_block(body, &mut d);
    d.names
}

/// Every name `stmts` reads or sets.
pub(crate) fn mentioned(stmts: &[Stmt]) -> HashSet<Symbol> {
    let mut names = HashSet::new();
    for_each_node(stmts, |node| match node {
        Visited::Expr(Expr::Identifier(sym)) => {
            names.insert(*sym);
        }
        Visited::Stmt(Stmt::Set { target, .. }) => {
            names.insert(*target);
        }
        _ => {}
    });
    names
}

/// The names a callee resolves where it is declared: every name its body reads or sets somewhere neither one
/// of its parameters nor a name it has declared by then in a block enclosing that place. A block's names end
/// with the block, and a name read before the body declares it is the one around the callee, as the
/// tree-walker resolves both.
pub(crate) fn free_names(params: &[Symbol], body: &[Stmt]) -> HashSet<Symbol> {
    let mut walk = Unbound {
        scopes: vec![params.iter().copied().collect()],
        names: HashSet::new(),
    };
    walk_block(body, &mut walk);
    let mut free = mentioned(body);
    free.retain(|n| walk.names.contains(n));
    free
}

/// A walk of a body through its scopes, collecting each name met where no enclosing scope binds it yet.
struct Unbound {
    scopes: Vec<HashSet<Symbol>>,
    names: HashSet<Symbol>,
}

impl Unbound {
    fn met(&mut self, sym: Symbol) {
        if !self.scopes.iter().any(|scope| scope.contains(&sym)) {
            self.names.insert(sym);
        }
    }
}

impl EffectSink for Unbound {
    fn bind(&mut self, sym: Symbol, how: Bind) {
        match how {
            Bind::Declare => {
                self.scopes.last_mut().expect("a scope is open").insert(sym);
            }
            Bind::Assign => self.met(sym),
        }
    }
    fn mutate(&mut self, root: Symbol) {
        self.met(root);
    }
    fn read(&mut self, sym: Symbol) {
        self.met(sym);
    }
    fn call(&mut self) {}
    fn opaque(&mut self) {}
    fn scoped(&mut self, binders: &[Symbol], body: &[Stmt]) {
        self.scopes.push(binders.iter().copied().collect());
        walk_block(body, self);
        self.scopes.pop();
    }
    fn closure(&mut self, params: &[Symbol], body: &ClosureBody) {
        self.scopes.push(params.iter().copied().collect());
        match body {
            ClosureBody::Expression(e) => walk_expr(e, self),
            ClosureBody::Block(b) => walk_block(b, self),
        }
        self.scopes.pop();
    }
}

/// The names bound in a function's frame: its parameters and every name its body declares.
pub(crate) fn frame_names(params: &[Symbol], body: &[Stmt]) -> HashSet<Symbol> {
    let mut names = declared(body, true, true);
    names.extend(params.iter().copied());
    names
}

/// The names bound in `## Main`'s frame other than the program's own bindings: every name a nested block
/// of Main declares (a `Let` inside an `If`, a loop variable, a closure's parameter). Main's root bindings
/// are the program bindings a callee's free names mean, so an inlined read of one reads the same binding.
pub(crate) fn main_frame_names(main: &[Stmt]) -> HashSet<Symbol> {
    declared(main, false, true)
}

/// The names a block declares at its own level — the ones a splice into its parent would let outlive it.
pub(crate) fn own_names(block: &[Stmt]) -> HashSet<Symbol> {
    declared(block, true, false)
}

/// Whether `block` may be spliced in front of `rest` in their parent: every read of `rest` still reads
/// what it read, because none of the block's own names is one `rest` mentions.
pub(crate) fn splices_before(block: &[Stmt], rest: &[Stmt]) -> bool {
    own_names(block).is_disjoint(&mentioned(rest))
}

/// The names each block nested directly in a statement binds on entry — a loop's pattern, a match arm's
/// bindings, a zone's name — keyed by the block, so a pass descending into that block starts without them.
pub(crate) struct EntryBinders(Vec<(*const (), Vec<Symbol>)>);

impl EntryBinders {
    pub(crate) fn of(stmt: &Stmt) -> Self {
        struct Entries(Vec<(*const (), Vec<Symbol>)>);
        impl EffectSink for Entries {
            fn bind(&mut self, _sym: Symbol, _how: Bind) {}
            fn mutate(&mut self, _root: Symbol) {}
            fn call(&mut self) {}
            fn opaque(&mut self) {}
            fn scoped(&mut self, binders: &[Symbol], body: &[Stmt]) {
                if !binders.is_empty() {
                    self.0.push((body.as_ptr() as *const (), binders.to_vec()));
                }
            }
        }
        let mut e = Entries(Vec::new());
        super::writes::walk_stmt(stmt, &mut e);
        EntryBinders(e.0)
    }

    /// The names `block` binds on entry; empty for a block that binds none.
    pub(crate) fn of_block(&self, block: &[Stmt]) -> &[Symbol] {
        let at = block.as_ptr() as *const ();
        self.0
            .iter()
            .find(|(b, _)| *b == at)
            .map_or(&[], |(_, names)| names.as_slice())
    }
}

/// The free names a callee writes: every name its body assigns or mutates in place (an element, a field)
/// that is neither one of its parameters nor one it declares — the program bindings a call changes.
pub(crate) fn free_writes(params: &[Symbol], body: &[Stmt]) -> HashSet<Symbol> {
    struct Writes(HashSet<Symbol>);
    impl EffectSink for Writes {
        fn bind(&mut self, sym: Symbol, how: Bind) {
            if how == Bind::Assign {
                self.0.insert(sym);
            }
        }
        fn mutate(&mut self, root: Symbol) {
            self.0.insert(root);
        }
        fn call(&mut self) {}
        fn opaque(&mut self) {}
    }
    let mut w = Writes(HashSet::new());
    walk_block(body, &mut w);
    let free = free_names(params, body);
    w.0.retain(|n| free.contains(n));
    w.0
}

/// Would substituting a callee with these free names into this frame capture one of them?
pub(crate) fn captures(free: &HashSet<Symbol>, frame: &HashSet<Symbol>) -> bool {
    !free.is_disjoint(frame)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/capture/tests.rs"]
mod tests;
