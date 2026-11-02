//! Which pops are proven to find an item: the `Pop` statements whose list holds at least one item on
//! every run that reaches them, so they need no check before them.
//!
//! The proof follows each block in order, keeping a lower bound on the length of every list it can name
//! (a variable, or a chain of fields from one: `xs`, `b's items`). A literal list or a new one starts it, a
//! copy carries it, `Push` raises it, `Pop` lowers it, and a condition on `length of` adds what holds while
//! it does (`While length of xs > 0`, the rest of a block after `If length of xs is 0: Return.`). A loop
//! keeps the bound of a list it changes only by pushes and pops at its own top level: the pass's net change
//! carries the bound from pass to pass, over a trip count a literal range or `1 to length of xs` fixes.
//!
//! Sound by construction:
//! - a bound is only ever a lower one, and a list nothing here names may be empty;
//! - whatever a statement may write ([`crate::optimize::WriteSet`], with what its calls lend and the
//!   globals a callee writes) loses its bounds, unless the statement is one modelled here exactly, and
//!   foreign code loses every bound;
//! - a name a block declares loses its bound when the block ends, so a shadowed list never lends its bound
//!   to the one it shadows;
//! - a branch that cannot go on past its end (`Return`, `Break`, `Fail`) adds nothing where the branches
//!   meet, and a zone or an `Inspect` ends with only what none of its paths writes;
//! - a statement this proof does not enter keeps every pop inside it checked, and a program that runs tasks
//!   side by side is not proven at all, since another task may pop between a check and its pop.

use std::collections::{BTreeMap, HashSet};

use logicaffeine_base::Symbol;

use crate::ast::stmt::{BinaryOpKind, Expr, Pattern, Stmt};
use crate::optimize::writes::CalleeGlobals;
use crate::optimize::WriteSet;

/// The `Pop` statements of `program` (by address) whose list holds an item whenever the pop runs.
pub(super) fn pops_that_find_an_item(program: &[Stmt]) -> HashSet<usize> {
    if crate::concurrency::uses_scheduler(program) {
        return HashSet::new();
    }
    crate::analysis::callgraph::install_lending_parameters(program);
    let globals = CalleeGlobals::of_program(program);
    let mut proof = Proof {
        globals: &globals,
        found: HashSet::new(),
    };
    proof.block(program, Some(Lengths::default()));
    proof.found
}

/// A list named by a variable and the fields followed from it.
type Place = Vec<Symbol>;

/// The list `e` names, when it names one.
fn place(e: &Expr) -> Option<Place> {
    match e {
        Expr::Identifier(s) => Some(vec![*s]),
        Expr::FieldAccess { object, field } => {
            let mut p = place(object)?;
            p.push(*field);
            Some(p)
        }
        _ => None,
    }
}

/// The least number of items each named list holds; a list not named here may hold none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Lengths(BTreeMap<Place, u64>);

impl Lengths {
    fn at_least(&self, p: &[Symbol]) -> u64 {
        self.0.get(p).copied().unwrap_or(0)
    }

    fn set(&mut self, p: Place, n: u64) {
        if n == 0 {
            self.0.remove(&p);
        } else {
            self.0.insert(p, n);
        }
    }

    /// Forget every list reached through `root`.
    fn forget_root(&mut self, root: Symbol) {
        self.0.retain(|p, _| p[0] != root);
    }

    /// Forget `p` and every list reached through it.
    fn forget_under(&mut self, p: &[Symbol]) {
        self.0.retain(|q, _| !q.starts_with(p));
    }

    /// Raise each bound to what `other` knows.
    fn raise(&mut self, other: Lengths) {
        for (p, n) in other.0 {
            let cur = self.at_least(&p);
            self.set(p, cur.max(n));
        }
    }

    /// What holds on both paths: the smaller bound of each list both name.
    fn meet(a: Lengths, b: &Lengths) -> Lengths {
        Lengths(
            a.0.into_iter()
                .filter_map(|(p, n)| b.0.get(&p).map(|m| (p, n.min(*m))))
                .collect(),
        )
    }
}

/// Where control goes after a block: on, with the bounds that hold there, or never.
fn join(a: Option<Lengths>, b: Option<Lengths>) -> Option<Lengths> {
    match (a, b) {
        (Some(a), Some(b)) => Some(Lengths::meet(a, &b)),
        (a, None) => a,
        (None, b) => b,
    }
}

/// How many passes a loop makes, as far as its header fixes them.
enum Trips {
    Unknown,
    /// `Repeat for i from a to b` over integer constants.
    Exact(u64),
    /// `Repeat for i from a to length of p`, `a` at least 1: the length `p` has when the loop starts, less
    /// `a - 1`.
    ToLengthOf { place: Place, from: u64 },
}

fn trips(iterable: &Expr) -> Trips {
    let Expr::Range { start, end } = iterable else {
        return Trips::Unknown;
    };
    let Some(a) = crate::loop_shape::const_eval_i64(start) else {
        return Trips::Unknown;
    };
    if let Some(b) = crate::loop_shape::const_eval_i64(end) {
        let n = (b as i128 - a as i128 + 1).max(0);
        return u64::try_from(n).map_or(Trips::Unknown, Trips::Exact);
    }
    match (end, u64::try_from(a)) {
        (Expr::Length { collection }, Ok(from)) if from >= 1 => match place(collection) {
            Some(place) => Trips::ToLengthOf { place, from },
            None => Trips::Unknown,
        },
        _ => Trips::Unknown,
    }
}

/// A list the loop body changes only by pushes and pops at its own top level, and how much one pass
/// changes its length.
struct Counted {
    place: Place,
    net: i64,
}

impl Counted {
    /// The least length at the top of every pass the loop makes, from `start` before it. A pass that does
    /// not shrink the list leaves at least what it found; one that does can run at most the passes `trips`
    /// allows, and `1 to length of` the list itself allows one pass per item, so a pass that pops one more
    /// than it pushes always finds as many as the range starts from.
    fn at_every_pass(&self, trips: &Trips, start: u64) -> u64 {
        match trips {
            Trips::ToLengthOf { place, from } if *place == self.place && self.net >= -1 => {
                if self.net >= 0 {
                    start.max(*from)
                } else {
                    *from
                }
            }
            _ if self.net >= 0 => start,
            Trips::Exact(t) => {
                let shrink = u128::from(t.saturating_sub(1)) * u128::from(self.net.unsigned_abs());
                u64::try_from(u128::from(start).saturating_sub(shrink)).unwrap_or(0)
            }
            _ => 0,
        }
    }

    /// The least length once the loop has made every pass, when no `Break` can end it early.
    fn after_every_pass(&self, trips: &Trips, start: u64) -> u64 {
        match trips {
            Trips::Exact(t) => {
                let end = i128::from(start) + i128::from(*t) * i128::from(self.net);
                u64::try_from(end.max(0)).unwrap_or(u64::MAX)
            }
            Trips::ToLengthOf { place, from } if *place == self.place && self.net == -1 => from - 1,
            _ if self.net >= 0 => start,
            _ => 0,
        }
    }
}

struct Proof<'g> {
    globals: &'g CalleeGlobals,
    found: HashSet<usize>,
}

impl Proof<'_> {
    fn writes_of_stmt(&self, stmt: &Stmt) -> WriteSet {
        WriteSet::of_stmt(stmt).with_callee_globals(self.globals)
    }

    fn writes_of_expr(&self, expr: &Expr) -> WriteSet {
        WriteSet::of_expr(expr).with_callee_globals(self.globals)
    }

    /// The bounds after something that writes `w` ran.
    fn after(&self, w: &WriteSet, mut at: Lengths) -> Lengths {
        if w.is_opaque() {
            return Lengths::default();
        }
        for s in w.symbols() {
            at.forget_root(s);
        }
        at
    }

    /// The least length of the list `value` evaluates to, read before evaluating it changes anything.
    fn length_of(&self, value: &Expr, at: &Lengths) -> u64 {
        if self.writes_of_expr(value).has_calls() {
            return 0;
        }
        match value {
            Expr::List(items) => items.len() as u64,
            Expr::Copy { expr } | Expr::Give { value: expr } => self.length_of(expr, at),
            Expr::WithCapacity { value, .. } => self.length_of(value, at),
            Expr::BinaryOp {
                op: BinaryOpKind::SeqConcat,
                left,
                right,
            } => self
                .length_of(left, at)
                .saturating_add(self.length_of(right, at)),
            other => place(other).map_or(0, |p| at.at_least(&p)),
        }
    }

    /// Bind `target` to `value`: its own length, and the lengths of the lists inside it, which a copy of
    /// a named value or a construction with fields gives.
    fn bind(&self, target: Place, value: &Expr, at: Lengths) -> Lengths {
        let n = self.length_of(value, &at);
        let calls = self.writes_of_expr(value).has_calls();
        let mut inner: Vec<(Place, u64)> = Vec::new();
        if !calls {
            if let Some(src) = place(value) {
                for (q, m) in &at.0 {
                    if q.len() > src.len() && q.starts_with(&src) {
                        let mut p = target.clone();
                        p.extend_from_slice(&q[src.len()..]);
                        inner.push((p, *m));
                    }
                }
            }
            if let Expr::New { init_fields, .. } = value {
                for (field, e) in init_fields {
                    let mut p = target.clone();
                    p.push(*field);
                    inner.push((p, self.length_of(e, &at)));
                }
            }
        }
        let mut at = self.after(&self.writes_of_expr(value), at);
        at.forget_under(&target);
        at.set(target, n);
        for (p, m) in inner {
            at.set(p, m);
        }
        at
    }

    /// A block's statements in order, from `at` (`None`: no run reaches it). A block nested in a statement
    /// ends with the names it declares forgotten.
    fn block(&mut self, block: &[Stmt], mut at: Option<Lengths>) -> Option<Lengths> {
        for stmt in block {
            at = match at {
                Some(facts) => self.stmt(stmt, facts),
                None => {
                    self.stmt(stmt, Lengths::default());
                    None
                }
            };
        }
        at
    }

    fn scoped(&mut self, block: &[Stmt], at: Option<Lengths>) -> Option<Lengths> {
        let mut out = self.block(block, at)?;
        for stmt in block {
            match stmt {
                Stmt::Let { var, .. } => out.forget_root(*var),
                _ => {
                    for s in WriteSet::of_stmt_shallow(stmt).declared() {
                        out.forget_root(*s);
                    }
                }
            }
        }
        Some(out)
    }

    fn stmt(&mut self, stmt: &Stmt, at: Lengths) -> Option<Lengths> {
        match stmt {
            Stmt::Let { var, value, .. } | Stmt::Set { target: var, value } => {
                Some(self.bind(vec![*var], value, at))
            }
            Stmt::SetField {
                object,
                field,
                value,
            } => match place(object) {
                Some(mut p) => {
                    let at = self.after(&self.writes_of_expr(object), at);
                    p.push(*field);
                    Some(self.bind(p, value, at))
                }
                None => Some(self.after(&self.writes_of_stmt(stmt), at)),
            },
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => match place(collection) {
                Some(p) => {
                    let at = self.after(&self.writes_of_expr(index), at);
                    let mut at = self.after(&self.writes_of_expr(value), at);
                    let kept = at.at_least(&p);
                    at.forget_under(&p);
                    at.set(p, kept);
                    Some(at)
                }
                None => Some(self.after(&self.writes_of_stmt(stmt), at)),
            },
            Stmt::Push { value, collection } => {
                let mut at = self.after(&self.writes_of_expr(value), at);
                match place(collection) {
                    Some(p) => {
                        let n = at.at_least(&p).saturating_add(1);
                        at.set(p, n);
                        Some(at)
                    }
                    None => Some(self.after(&self.writes_of_stmt(stmt), at)),
                }
            }
            Stmt::Pop { collection, into } => {
                let mut at = match place(collection) {
                    Some(p) => {
                        let n = at.at_least(&p);
                        if n >= 1 {
                            self.found.insert(stmt as *const Stmt as usize);
                        }
                        let mut at = at;
                        at.set(p, n.saturating_sub(1));
                        at
                    }
                    None => self.after(&self.writes_of_stmt(stmt), at),
                };
                if let Some(v) = into {
                    at.forget_root(*v);
                }
                Some(at)
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                let cond_writes = self.writes_of_expr(cond);
                let at = self.after(&cond_writes, at);
                let mut then_at = at.clone();
                then_at.raise(self.when(cond, true, &cond_writes));
                let mut else_at = at;
                else_at.raise(self.when(cond, false, &cond_writes));
                let then_out = self.scoped(then_block, Some(then_at));
                let else_out = match else_block {
                    Some(b) => self.scoped(b, Some(else_at)),
                    None => Some(else_at),
                };
                join(then_out, else_out)
            }
            Stmt::While { cond, body, .. } => Some(self.repeat(stmt, Some(cond), body, Trips::Unknown, &[], at)),
            Stmt::Repeat {
                pattern,
                iterable,
                body,
            } => {
                let at = self.after(&self.writes_of_expr(iterable), at);
                let binders = match pattern {
                    Pattern::Identifier(s) => vec![*s],
                    Pattern::Tuple(ss) => ss.clone(),
                };
                Some(self.repeat(stmt, None, body, trips(iterable), &binders, at))
            }
            Stmt::Zone { body, .. } => {
                self.scoped(body, Some(at.clone()));
                Some(self.after(&self.writes_of_stmt(stmt), at))
            }
            Stmt::Inspect { target, arms, .. } => {
                let at = self.after(&self.writes_of_expr(target), at);
                for arm in arms {
                    let mut arm_at = at.clone();
                    for (_, binding) in &arm.bindings {
                        arm_at.forget_root(*binding);
                    }
                    self.scoped(arm.body, Some(arm_at));
                }
                Some(self.after(&self.writes_of_stmt(stmt), at))
            }
            Stmt::FunctionDef { body, .. } => {
                self.scoped(body, Some(Lengths::default()));
                Some(at)
            }
            Stmt::Return { .. } | Stmt::Break | Stmt::Fail { .. } => None,
            _ => Some(self.after(&self.writes_of_stmt(stmt), at)),
        }
    }

    /// A loop: its body from the bounds that hold at the top of every pass, and the bounds after it.
    fn repeat(
        &mut self,
        stmt: &Stmt,
        cond: Option<&Expr>,
        body: &[Stmt],
        trips: Trips,
        binders: &[Symbol],
        at: Lengths,
    ) -> Lengths {
        let whole = self.writes_of_stmt(stmt);
        if whole.is_opaque() {
            self.scoped(body, Some(Lengths::default()));
            return Lengths::default();
        }
        let counted = self.counted(cond, body, binders);
        let mut invariant = self.after(&whole, at.clone());
        for b in binders {
            invariant.forget_root(*b);
        }
        let cond_writes = cond.map(|c| self.writes_of_expr(c));

        let mut top = invariant.clone();
        for c in &counted {
            top.set(c.place.clone(), c.at_every_pass(&trips, at.at_least(&c.place)));
        }
        if let (Some(c), Some(w)) = (cond, &cond_writes) {
            top.raise(self.when(c, true, w));
        }
        self.scoped(body, Some(top));

        let breaks = crate::loop_shape::leaves_by_break(body);
        let mut out = invariant;
        if !breaks {
            for c in &counted {
                out.set(c.place.clone(), c.after_every_pass(&trips, at.at_least(&c.place)));
            }
            if let (Some(c), Some(w)) = (cond, &cond_writes) {
                out.raise(self.when(c, false, w));
            }
        }
        out
    }

    /// The lists the body changes only by pushes and pops of the list itself at its top level, with nothing
    /// else in the loop (its condition, the rest of the body, what a pushed value's evaluation or a call
    /// writes, a name the loop binds) touching the variable they hang from.
    fn counted(&self, cond: Option<&Expr>, body: &[Stmt], binders: &[Symbol]) -> Vec<Counted> {
        let mut candidates: Vec<Place> = Vec::new();
        for s in body {
            if let Stmt::Push { collection, .. } | Stmt::Pop { collection, .. } = s {
                if let Some(p) = place(collection) {
                    if !candidates.contains(&p) {
                        candidates.push(p);
                    }
                }
            }
        }
        let cond_writes = cond.map(|c| self.writes_of_expr(c));
        candidates
            .into_iter()
            .filter_map(|p| {
                let root = p[0];
                if binders.contains(&root)
                    || cond_writes
                        .as_ref()
                        .is_some_and(|w| w.is_opaque() || w.may_write(root))
                {
                    return None;
                }
                let mut net = 0i64;
                for s in body {
                    let others = match s {
                        Stmt::Push { value, collection } if place(collection).as_ref() == Some(&p) => {
                            net += 1;
                            self.writes_of_expr(value)
                        }
                        Stmt::Pop { collection, into } if place(collection).as_ref() == Some(&p) => {
                            net -= 1;
                            if *into == Some(root) {
                                return None;
                            }
                            WriteSet::default()
                        }
                        other => self.writes_of_stmt(other),
                    };
                    if others.is_opaque() || others.may_write(root) {
                        return None;
                    }
                }
                Some(Counted { place: p, net })
            })
            .collect()
    }

    /// What `cond` evaluating to `holds` says of lengths, leaving out every list its own evaluation writes.
    fn when(&self, cond: &Expr, holds: bool, cond_writes: &WriteSet) -> Lengths {
        let mut out = Lengths::default();
        if cond_writes.is_opaque() {
            return out;
        }
        facts(cond, holds, &mut out);
        let written: Vec<Symbol> = cond_writes.symbols().collect();
        for s in written {
            out.forget_root(s);
        }
        out
    }
}

/// What `cond` evaluating to `holds` says of lengths: `length of p` compared with an integer constant, under
/// `and` (when it holds), `or` (when it fails) and `not`.
fn facts(cond: &Expr, holds: bool, out: &mut Lengths) {
    match cond {
        Expr::Not { operand } => facts(operand, !holds, out),
        Expr::BinaryOp {
            op: BinaryOpKind::And,
            left,
            right,
        } if holds => {
            facts(left, true, out);
            facts(right, true, out);
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Or,
            left,
            right,
        } if !holds => {
            facts(left, false, out);
            facts(right, false, out);
        }
        Expr::BinaryOp { op, left, right }
            if matches!(
                op,
                BinaryOpKind::Eq
                    | BinaryOpKind::NotEq
                    | BinaryOpKind::Lt
                    | BinaryOpKind::LtEq
                    | BinaryOpKind::Gt
                    | BinaryOpKind::GtEq
            ) =>
        {
            let (op, p, k) = match (left, right) {
                (Expr::Length { collection }, other) => match (place(collection), crate::loop_shape::const_eval_i64(other)) {
                    (Some(p), Some(k)) => (*op, p, k),
                    _ => return,
                },
                (other, Expr::Length { collection }) => {
                    match (place(collection), crate::loop_shape::const_eval_i64(other)) {
                        (Some(p), Some(k)) => (mirrored(*op), p, k),
                        _ => return,
                    }
                }
                _ => return,
            };
            let least = match (op, holds) {
                (BinaryOpKind::Gt, true) | (BinaryOpKind::LtEq, false) => k.checked_add(1),
                (BinaryOpKind::GtEq, true) | (BinaryOpKind::Lt, false) => Some(k),
                (BinaryOpKind::Eq, true) | (BinaryOpKind::NotEq, false) => Some(k),
                (BinaryOpKind::NotEq, true) | (BinaryOpKind::Eq, false) if k == 0 => Some(1),
                _ => None,
            };
            if let Some(n) = least.and_then(|n| u64::try_from(n).ok()) {
                let cur = out.at_least(&p);
                out.set(p, cur.max(n));
            }
        }
        _ => {}
    }
}

/// `k op length` read as `length op' k`.
fn mirrored(op: BinaryOpKind) -> BinaryOpKind {
    match op {
        BinaryOpKind::Lt => BinaryOpKind::Gt,
        BinaryOpKind::Gt => BinaryOpKind::Lt,
        BinaryOpKind::LtEq => BinaryOpKind::GtEq,
        BinaryOpKind::GtEq => BinaryOpKind::LtEq,
        other => other,
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/resolve_pop/nonempty/tests.rs"]
mod tests;
