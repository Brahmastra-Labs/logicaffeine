//! Exact `Int` locals held as machine words through a loop that can be run again.
//!
//! A local the exactness decision makes a `LogosInt` (`exact_scalars`) pays an exact operation wherever it is touched.
//! Through a loop that cannot carry it past the word, the local is a machine word: every operation the loop makes on
//! it is proven to stay in the word (a counted loop's guard, a versioned clone's bound), so the loop runs on words
//! wherever the local holds one as the loop begins. Through a loop that can, but whose only effects are on locals,
//! the local is a machine word until an operation would leave the word; then the loop is abandoned there and run
//! again from where it began, exactly. That word form runs inside `logicaffeine_system::failure::speculate`, which
//! ends it at the canonical overflow (`logicaffeine_data::ops::raise_overflow`) wherever that is raised, inside a
//! closure or a call included, so the emission of either word form is the emitter's own for machine words. A program
//! with a loop that runs again unwinds on panic.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::stmt::{Expr, Stmt};
use crate::intern::Symbol;
use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};

/// A loop the emitter writes twice: on machine words, and exactly.
pub(crate) struct Region {
    pub(crate) id: u32,
    /// The exact locals the word form holds as machine words, in a fixed order.
    pub(crate) vars: Vec<Symbol>,
    /// Of `vars`, those bound before the loop: read into words as it starts, and written back as it ends.
    pub(crate) outer: Vec<Symbol>,
    /// Of `outer`, those the loop assigns.
    pub(crate) assigned: Vec<Symbol>,
    /// The machine-word locals bound before the loop that it assigns: restored before it runs again exactly.
    pub(crate) restores: Vec<Symbol>,
    /// Whether the word form gives up at a value past the word and the loop runs again exactly; otherwise every
    /// operation the word form makes on `vars` is proven to stay in the word, and it never gives up.
    pub(crate) restarts: bool,
}

/// What a region's planning asks of the compile: whether an expression stays in the word with the given locals still
/// exact (`exact_scalars::stays_word`).
pub(crate) type Proves<'p> = dyn Fn(&Expr, &HashSet<Symbol>) -> bool + 'p;

thread_local! {
    static REGIONS: RefCell<HashMap<usize, Rc<Region>>> = RefCell::new(HashMap::new());
    static EMITTING: RefCell<HashSet<u32>> = RefCell::new(HashSet::new());
    static REGION_ID: Cell<u32> = const { Cell::new(0) };
}

/// Number the regions of the program about to be emitted from the first: each names its `__done{id}` flag.
pub(crate) fn reset_region_ids() {
    REGION_ID.with(|n| n.set(0));
}

/// The program's regions, installed for one compile; dropping it removes them.
pub(crate) struct Plan;

impl Drop for Plan {
    fn drop(&mut self) {
        REGIONS.with(|r| r.borrow_mut().clear());
        EMITTING.with(|e| e.borrow_mut().clear());
    }
}

/// Plan every body of the program: `bodies` each with the locals the decision makes exact there and the locals bound
/// to an `Int`. Whether any region was planned, so the program unwinds on panic.
/// One body to plan: its statements, the locals the decision makes exact there, the locals bound to an `Int`, and
/// what its planning asks of the compile.
pub(crate) struct Body<'b, 'p> {
    pub(crate) stmts: &'b [Stmt<'b>],
    pub(crate) exact: HashSet<Symbol>,
    pub(crate) ints: HashSet<Symbol>,
    pub(crate) proves: Box<Proves<'p>>,
}

/// Plan every body of the program. Whether a region that runs again was planned, so the program unwinds on panic.
pub(crate) fn plan_program(bodies: &[Body], pure: &HashSet<Symbol>) -> (Plan, bool) {
    let mut restarts = false;
    for body in bodies {
        if !body.exact.is_empty() {
            plan_block(body.stmts, body, pure, &mut restarts);
        }
    }
    (Plan, restarts)
}

fn plan_block(block: &[Stmt], body: &Body, pure: &HashSet<Symbol>, restarts: &mut bool) {
    for s in block {
        match s {
            Stmt::While { body: inner, .. } | Stmt::Repeat { body: inner, .. } => {
                if let Some(region) = region_of(s, body, pure) {
                    *restarts |= region.restarts;
                    REGIONS.with(|r| r.borrow_mut().insert(s as *const Stmt as usize, Rc::new(region)));
                } else {
                    plan_block(inner, body, pure, restarts);
                }
            }
            Stmt::If { then_block, else_block, .. } => {
                plan_block(then_block, body, pure, restarts);
                if let Some(b) = else_block {
                    plan_block(b, body, pure, restarts);
                }
            }
            Stmt::Zone { body: inner, .. } => plan_block(inner, body, pure, restarts),
            Stmt::Inspect { arms, .. } => {
                for arm in arms.iter() {
                    plan_block(arm.body, body, pure, restarts);
                }
            }
            _ => {}
        }
    }
}

/// Of `touched`, the locals every operation of the loop `l` on which is proven to stay in the word, with the others
/// still exact: an operation that reads one of them, and a value one of them is given, stays in the word. A local
/// that fails leaves, and what it fails may depend on, so the rest are asked again until none leaves.
fn proven(l: &Stmt, touched: &[Symbol], body: &Body) -> Vec<Symbol> {
    let mut words: Vec<Symbol> = touched.to_vec();
    loop {
        let exact: HashSet<Symbol> = body.exact.iter().copied().filter(|s| !words.contains(s)).collect();
        let mut leaving: HashSet<Symbol> = HashSet::new();
        walk_nodes(std::slice::from_ref(l), |node| {
            match node {
                Visited::Stmt(Stmt::FunctionDef { .. }) => return Walk::Past,
                Visited::Expr(c @ Expr::Closure { .. }) => {
                    leaving.extend(words.iter().copied().filter(|w| crate::optimize::expr_touches(c, *w)));
                    return Walk::Past;
                }
                Visited::Stmt(Stmt::Let { var: v, value, .. } | Stmt::Set { target: v, value })
                    if words.contains(v) && !(body.proves)(value, &exact) =>
                {
                    leaving.insert(*v);
                }
                Visited::Expr(e @ Expr::BinaryOp { .. }) if !(body.proves)(e, &exact) => {
                    leaving.extend(words.iter().copied().filter(|w| crate::optimize::expr_touches(e, *w)));
                }
                _ => {}
            }
            Walk::Into
        });
        if leaving.is_empty() {
            return words;
        }
        words.retain(|w| !leaving.contains(w));
        if words.is_empty() {
            return words;
        }
    }
}

/// The region of the loop `l`, where it is one: it touches an exact local, and running it again from where it began
/// is invisible. Its effects are on locals alone: it assigns machine words, exact locals and what it binds itself;
/// it writes no collection it does not bind; it shows, sends and returns nothing; every call it makes is to a pure
/// function.
fn region_of(l: &Stmt, body: &Body, pure: &HashSet<Symbol>) -> Option<Region> {
    let (exact, ints) = (&body.exact, &body.ints);
    let mut bound: HashSet<Symbol> = HashSet::new();
    let mut assigned: Vec<Symbol> = Vec::new();
    let mut touched: Vec<Symbol> = Vec::new();
    let mut confined = true;
    let note = |list: &mut Vec<Symbol>, s: Symbol| {
        if !list.contains(&s) {
            list.push(s);
        }
    };
    walk_nodes(std::slice::from_ref(l), |node| {
        match node {
            Visited::Stmt(Stmt::FunctionDef { .. }) | Visited::Expr(Expr::Closure { .. }) => {
                confined = false;
                return Walk::Past;
            }
            Visited::Stmt(Stmt::Let { var, .. }) => {
                bound.insert(*var);
            }
            Visited::Stmt(Stmt::Set { target, .. }) => note(&mut assigned, *target),
            Visited::Stmt(Stmt::Repeat { pattern, .. }) => match pattern {
                crate::ast::stmt::Pattern::Identifier(x) => {
                    bound.insert(*x);
                }
                crate::ast::stmt::Pattern::Tuple(xs) => bound.extend(xs.iter().copied()),
            },
            Visited::Stmt(
                Stmt::Push { collection, .. }
                | Stmt::SetIndex { collection, .. }
                | Stmt::Pop { collection, .. }
                | Stmt::Add { collection, .. }
                | Stmt::Remove { collection, .. },
            ) => {
                if !matches!(collection, Expr::Identifier(c) if bound.contains(c)) {
                    confined = false;
                }
                if let Stmt::Pop { into: Some(x), .. } = node_stmt(&node) {
                    note(&mut assigned, *x);
                }
            }
            Visited::Stmt(
                Stmt::While { .. }
                | Stmt::If { .. }
                | Stmt::Inspect { .. }
                | Stmt::Break
                | Stmt::Zone { .. }
                | Stmt::Splice { .. },
            ) => {}
            Visited::Stmt(Stmt::Call { function, .. }) => confined &= pure.contains(function),
            Visited::Stmt(_) => confined = false,
            Visited::Expr(Expr::Call { function, .. }) => confined &= pure.contains(function),
            Visited::Expr(Expr::Identifier(s)) if exact.contains(s) => note(&mut touched, *s),
            _ => {}
        }
        Walk::Into
    });
    for s in &assigned {
        if exact.contains(s) {
            note(&mut touched, *s);
        }
    }
    if touched.is_empty() {
        return None;
    }
    let mut restores = Vec::new();
    let mut rerunnable = confined;
    for s in &assigned {
        if exact.contains(s) || bound.contains(s) {
            continue;
        }
        if !ints.contains(s) {
            rerunnable = false;
        }
        restores.push(*s);
    }
    let words = proven(l, &touched, body);
    let (vars, restarts) = if words.len() == touched.len() {
        (words, false)
    } else if rerunnable {
        (touched, true)
    } else if !words.is_empty() {
        (words, false)
    } else {
        return None;
    };
    let outer: Vec<Symbol> = vars.iter().copied().filter(|s| !bound.contains(s)).collect();
    let assigned_outer = outer.iter().copied().filter(|s| assigned.contains(s)).collect();
    let id = REGION_ID.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    Some(Region { id, vars, outer, assigned: assigned_outer, restores: if restarts { restores } else { Vec::new() }, restarts })
}

fn node_stmt<'n, 'a>(node: &Visited<'n, 'a>) -> &'n Stmt<'a> {
    match node {
        Visited::Stmt(s) => s,
        Visited::Expr(_) => unreachable!("a statement node"),
    }
}

/// The region the loop `stmt` is, unless its forms are being emitted now.
pub(crate) fn region_at(stmt: &Stmt) -> Option<Rc<Region>> {
    let region = REGIONS.with(|r| r.borrow().get(&(stmt as *const Stmt as usize)).cloned())?;
    (!EMITTING.with(|e| e.borrow().contains(&region.id))).then_some(region)
}

/// The emission of one of `region`'s forms: the region is not entered again while it runs.
pub(crate) struct Emitting(u32);

impl Drop for Emitting {
    fn drop(&mut self) {
        EMITTING.with(|e| e.borrow_mut().remove(&self.0));
    }
}

pub(crate) fn emitting(region: &Region) -> Emitting {
    EMITTING.with(|e| e.borrow_mut().insert(region.id));
    Emitting(region.id)
}
