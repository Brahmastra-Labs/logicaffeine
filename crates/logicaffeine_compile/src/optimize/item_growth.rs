//! Item growth: the arithmetic a counted loop does on sequence items that grow by a bounded amount per pass.
//!
//! A count kept in a sequence (`Set item v of counts to item v of counts + 1`) or a dynamic-programming row
//! rolled from pass to pass (`Push best to curr`, `best` an item of `prev` plus a bounded value, then
//! `Set prev to curr`) grows with the passes of the loop that drives it. Overflow-checking each addition keeps
//! the loop from vectorizing, and no range analysis can drop the check: the items have no bound until the number
//! of passes has one. This module finds that bound. It runs one pass of the loop abstractly, with every value an
//! interval whose ends move linearly with the passes made before it (`c + s·t`), every sequence the loop carries
//! from pass to pass starting the pass with its items in `[lo, hi + g·t]`, and solves for the least growth `g` per
//! pass that the pass preserves. Every sum, difference and product whose ends are lines then stays within i64 for
//! as many passes as the steepest of them allows, so one comparison on the loop's limit, made before it runs,
//! makes them all raw ([`super::version`]).
//!
//! Sound by construction:
//! - a sequence is followed only where every use of it is one this module models: an item read, its length, a
//!   `Push` or item store into it, its creation, and the roll `Set prev to curr` of a sequence the pass creates
//!   fresh. It is local to the body and never handed to a call, so nothing else reaches its items;
//! - a value nothing here follows is read from the range oracle ([`super::OracleFacts`]), whose interval at a node
//!   holds on every run, or is unknown — and an unknown value makes every operation over it unknown;
//! - a loop nested in the pass runs its body once abstractly, with every name it writes forgotten before and after,
//!   so a value it carries from one of its iterations to the next is the oracle's; a carried sequence it stores
//!   into is refused, since each of its iterations could grow an item again;
//! - a branch runs each arm abstractly and joins what they leave.

use std::collections::{HashMap, HashSet};

use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};

use super::writes::WriteSet;
use super::OracleFacts;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use crate::intern::Symbol;

/// What one comparison before a counted loop buys: with the loop's limit at most `limit_at_most`, every Int op
/// node whose address is in `raw` keeps its exact value within i64 on every pass.
#[derive(Debug)]
pub(crate) struct Plan {
    pub limit_at_most: i64,
    pub raw: HashSet<usize>,
}

/// The fewest passes worth a second copy of the loop.
const MIN_PASSES: i128 = 1 << 16;

/// Rounds of the growth fixpoint; a growth still climbing after them is not linear (an item doubled per pass).
const ROUNDS: usize = 8;

/// `c + s·t`, `t` the passes made before the current one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Line {
    c: i128,
    s: i128,
}

impl Line {
    fn level(c: i128) -> Line {
        Line { c, s: 0 }
    }
    fn add(self, o: Line) -> Option<Line> {
        Some(Line {
            c: self.c.checked_add(o.c)?,
            s: self.s.checked_add(o.s)?,
        })
    }
    fn sub(self, o: Line) -> Option<Line> {
        Some(Line {
            c: self.c.checked_sub(o.c)?,
            s: self.s.checked_sub(o.s)?,
        })
    }
}

/// The values an expression takes on pass `t`: from `lo(t)` through `hi(t)`, for every `t >= 0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Span {
    lo: Line,
    hi: Line,
}

impl Span {
    fn level(lo: i128, hi: i128) -> Span {
        Span {
            lo: Line::level(lo),
            hi: Line::level(hi),
        }
    }

    fn is_level(self) -> bool {
        self.lo.s == 0 && self.hi.s == 0
    }

    /// Both spans: below the larger line and above the smaller, on every `t >= 0`.
    fn join(self, o: Span) -> Span {
        Span {
            lo: Line {
                c: self.lo.c.min(o.lo.c),
                s: self.lo.s.min(o.lo.s),
            },
            hi: Line {
                c: self.hi.c.max(o.hi.c),
                s: self.hi.s.max(o.hi.s),
            },
        }
    }

    fn add(self, o: Span) -> Option<Span> {
        Some(Span {
            lo: self.lo.add(o.lo)?,
            hi: self.hi.add(o.hi)?,
        })
    }

    fn sub(self, o: Span) -> Option<Span> {
        Some(Span {
            lo: self.lo.sub(o.hi)?,
            hi: self.hi.sub(o.lo)?,
        })
    }

    /// A product of two levels (spans that do not move with the passes); a moving factor would make a curve.
    fn mul(self, o: Span) -> Option<Span> {
        if !self.is_level() || !o.is_level() {
            return None;
        }
        let corners = [
            self.lo.c.checked_mul(o.lo.c)?,
            self.lo.c.checked_mul(o.hi.c)?,
            self.hi.c.checked_mul(o.lo.c)?,
            self.hi.c.checked_mul(o.hi.c)?,
        ];
        Some(Span::level(
            *corners.iter().min()?,
            *corners.iter().max()?,
        ))
    }

    /// The most passes `T` for which every value on passes `0..T` lies within i64 (`i128::MAX` when the span does
    /// not move toward either end), or `None` when the first pass already leaves it.
    fn passes_within_word(self) -> Option<i128> {
        let (min, max) = (i64::MIN as i128, i64::MAX as i128);
        if self.hi.c > max || self.lo.c < min {
            return None;
        }
        let up = if self.hi.s > 0 {
            (max - self.hi.c) / self.hi.s + 1
        } else {
            i128::MAX
        };
        let down = if self.lo.s < 0 {
            (self.lo.c - min) / -self.lo.s + 1
        } else {
            i128::MAX
        };
        Some(up.min(down))
    }
}

/// The items of a sequence the pass follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Items {
    /// No item yet.
    Empty,
    /// Every item lies within the span.
    Within(Span),
    /// An item was given a value nothing here bounds.
    Lost,
}

impl Items {
    fn store(self, v: Option<Span>) -> Items {
        match (self, v) {
            (Items::Lost, _) | (_, None) => Items::Lost,
            (Items::Empty, Some(v)) => Items::Within(v),
            (Items::Within(a), Some(v)) => Items::Within(a.join(v)),
        }
    }

    fn join(self, o: Items) -> Items {
        match (self, o) {
            (Items::Lost, _) | (_, Items::Lost) => Items::Lost,
            (Items::Empty, x) | (x, Items::Empty) => x,
            (Items::Within(a), Items::Within(b)) => Items::Within(a.join(b)),
        }
    }
}

/// The plan for the counted loop `init; lp` (counter from a literal, limit fixed while it runs), preceded in its
/// block by `before`: the most passes under which the arithmetic its carried sequences feed stays within i64, as
/// a bound on the limit, and the nodes that arithmetic is. `None` when there is no such arithmetic, or no bound
/// worth a second copy of the loop.
pub(crate) fn plan<'a>(
    before: &'a [Stmt<'a>],
    init: &Stmt<'a>,
    lp: &'a Stmt<'a>,
    oracle: &OracleFacts,
) -> Option<Plan> {
    let (counter, start) = match init {
        Stmt::Let { var, value, .. } | Stmt::Set { target: var, value } => (*var, *value),
        _ => return None,
    };
    let Expr::Literal(Literal::Number(start)) = start else {
        return None;
    };
    let Stmt::While {
        cond:
            Expr::BinaryOp {
                op: op @ (BinaryOpKind::Lt | BinaryOpKind::LtEq),
                left: Expr::Identifier(c),
                right: limit,
            },
        body,
        ..
    } = lp
    else {
        return None;
    };
    if *c != counter {
        return None;
    }
    let inclusive = matches!(op, BinaryOpKind::LtEq);
    // The counter steps by one at the end of each pass and nowhere else, and the limit is fixed while the loop
    // runs, so a run makes at most `limit - start` passes (one more under `<=`).
    let (Some(Stmt::Set { target, value: step }), pass) = (body.last(), &body[..body.len().saturating_sub(1)])
    else {
        return None;
    };
    let steps_by_one = matches!(step, Expr::BinaryOp {
        op: BinaryOpKind::Add,
        left: Expr::Identifier(a),
        right: Expr::Literal(Literal::Number(1)),
    } if *a == counter);
    if *target != counter || !steps_by_one {
        return None;
    }
    let written = WriteSet::of_block(pass);
    if written.is_opaque()
        || written.may_write(counter)
        || names_in(limit).iter().any(|n| written.may_write(*n))
        || mentions_call(limit)
    {
        return None;
    }

    // The sequences the pass writes, and which of them it creates afresh each pass.
    let fresh: HashSet<Symbol> = pass
        .iter()
        .filter_map(|s| match s {
            Stmt::Let { var, value, .. } if matches!(without_capacity(value), Expr::New { .. }) => {
                Some(*var)
            }
            _ => None,
        })
        .collect();
    let mut followed: HashSet<Symbol> = written.mutated().iter().copied().collect();
    for s in pass {
        if let Stmt::Set {
            target,
            value: Expr::Identifier(from),
        } = s
        {
            if fresh.contains(from) {
                followed.insert(*target);
            }
        }
    }
    // A roll from a sequence that is not followed is not followed either: settle the set.
    loop {
        let candidates = followed.clone();
        followed.retain(|s| uses_are_modeled(*s, pass, &fresh, &candidates));
        if followed.len() == candidates.len() {
            break;
        }
    }
    // A carried sequence starts the loop with the items `before` gave it.
    let mut entry: HashMap<Symbol, (i128, i128)> = HashMap::new();
    for &s in &followed {
        if fresh.contains(&s) {
            continue;
        }
        match entry_items(s, before, oracle) {
            Some(Items::Within(span)) if span.is_level() => {
                entry.insert(s, (span.lo.c, span.hi.c));
            }
            _ => {}
        }
    }
    followed.retain(|s| fresh.contains(s) || entry.contains_key(s));
    if entry.is_empty() {
        return None;
    }

    // The least growth per pass that a pass preserves.
    let carried: HashSet<Symbol> = entry.keys().copied().collect();
    let mut growth: HashMap<Symbol, i128> = entry.keys().map(|s| (*s, 0)).collect();
    let mut settled = None;
    for _ in 0..ROUNDS {
        let mut run = Pass::new(oracle, &followed, &carried);
        for (&s, &(lo, hi)) in &entry {
            run.items.insert(
                s,
                Items::Within(Span {
                    lo: Line::level(lo),
                    hi: Line { c: hi, s: growth[&s] },
                }),
            );
        }
        for &q in &fresh {
            if followed.contains(&q) {
                run.items.insert(q, Items::Empty);
            }
        }
        run.block(pass);
        if !run.ok {
            return None;
        }
        let mut climbed = false;
        for (&s, &(lo, hi)) in &entry {
            match run.items.get(&s).copied().unwrap_or(Items::Lost) {
                Items::Lost => return None,
                Items::Empty => {}
                Items::Within(end) => {
                    if end.lo.c < lo || end.lo.s < 0 {
                        return None;
                    }
                    let need = end.hi.s.max(end.hi.c - hi).max(0);
                    if need > growth[&s] {
                        growth.insert(s, need);
                        climbed = true;
                    }
                }
            }
        }
        if !climbed {
            settled = Some(run);
            break;
        }
    }
    let run = settled?;

    // The ops that move with the passes set the bound; every op that holds for the whole bound goes raw.
    let mut passes = i128::MAX;
    let mut moving = false;
    for (addr, span) in &run.spans {
        if run.unknown.contains(addr) || span.is_level() {
            continue;
        }
        if let Some(n) = span.passes_within_word() {
            if n >= MIN_PASSES {
                passes = passes.min(n);
                moving = true;
            }
        }
    }
    if !moving {
        return None;
    }
    let raw: HashSet<usize> = run
        .spans
        .iter()
        .filter(|(addr, span)| {
            !run.unknown.contains(addr) && span.passes_within_word().is_some_and(|n| n >= passes)
        })
        .map(|(addr, _)| *addr)
        .collect();
    // `passes` passes run when the limit is at most `start + passes` (one fewer for an inclusive limit).
    let limit = *start as i128 + passes - i128::from(inclusive);
    Some(Plan {
        limit_at_most: limit.clamp(i64::MIN as i128, i64::MAX as i128) as i64,
        raw,
    })
}

/// Whether every use of `s` in the pass is one this module follows: an item read, its length, a store or push
/// into it, its creation as a fresh sequence, or the roll `Set s to q` / its source, both followed.
fn uses_are_modeled(
    s: Symbol,
    pass: &[Stmt],
    fresh: &HashSet<Symbol>,
    followed: &HashSet<Symbol>,
) -> bool {
    let mut allowed: HashSet<usize> = HashSet::new();
    let mut seen: Vec<usize> = Vec::new();
    let mut ok = true;
    walk_nodes(pass, |node| {
        match node {
            Visited::Stmt(stmt) => match stmt {
                Stmt::SetIndex {
                    collection: col @ Expr::Identifier(c),
                    ..
                }
                | Stmt::Push {
                    collection: col @ Expr::Identifier(c),
                    ..
                } if *c == s => {
                    allowed.insert(*col as *const Expr as usize);
                }
                Stmt::Let { var, value, .. } if *var == s => {
                    if !(fresh.contains(&s) && matches!(without_capacity(value), Expr::New { .. })) {
                        ok = false;
                    }
                }
                Stmt::Set { target, value } if *target == s => match value {
                    Expr::Identifier(from) if fresh.contains(from) && followed.contains(from) => {}
                    _ => ok = false,
                },
                Stmt::Set {
                    target,
                    value: v @ Expr::Identifier(from),
                } if *from == s => {
                    if fresh.contains(&s) && followed.contains(target) {
                        allowed.insert(*v as *const Expr as usize);
                    } else {
                        ok = false;
                    }
                }
                Stmt::Swap { a, b } if *a == s || *b == s => ok = false,
                Stmt::FunctionDef { .. } => return Walk::Past,
                _ => {}
            },
            Visited::Expr(expr) => match expr {
                Expr::Index {
                    collection: col @ Expr::Identifier(c),
                    ..
                }
                | Expr::Length {
                    collection: col @ Expr::Identifier(c),
                } if *c == s => {
                    allowed.insert(*col as *const Expr as usize);
                }
                Expr::Identifier(c) if *c == s => seen.push(expr as *const Expr as usize),
                // A closure runs its body when it is called, which this pass cannot place in order.
                Expr::Closure { .. } => {
                    ok = false;
                    return Walk::Past;
                }
                _ => {}
            },
        }
        Walk::Into
    });
    ok && seen.iter().all(|a| allowed.contains(a))
}

/// The value `e` makes, under any capacity hints: a hint says where a sequence's memory comes from, never what it holds.
fn without_capacity<'e, 'a>(e: &'e Expr<'a>) -> &'e Expr<'a> {
    match e {
        Expr::WithCapacity { value, .. } => without_capacity(value),
        other => other,
    }
}

/// The items `before` leaves in `s` when it creates `s` and only reads it and stores or pushes values the range
/// oracle bounds; `None` for any other history.
fn entry_items(s: Symbol, before: &[Stmt], oracle: &OracleFacts) -> Option<Items> {
    let at = before
        .iter()
        .rposition(|st| matches!(st, Stmt::Let { var, .. } if *var == s))?;
    let Stmt::Let { value, .. } = &before[at] else {
        return None;
    };
    let mut items = match without_capacity(value) {
        Expr::New { .. } => Items::Empty,
        Expr::List(elems) => elems.iter().fold(Items::Empty, |acc, e| {
            acc.store(oracle.expr_int_range(e).map(|(lo, hi)| Span::level(lo as i128, hi as i128)))
        }),
        _ => return None,
    };
    let rest = &before[at + 1..];
    let mut allowed: HashSet<usize> = HashSet::new();
    let mut seen: Vec<usize> = Vec::new();
    let mut ok = true;
    walk_nodes(rest, |node| {
        match node {
            Visited::Stmt(stmt) => match stmt {
                Stmt::SetIndex {
                    collection: col @ Expr::Identifier(c),
                    value,
                    ..
                }
                | Stmt::Push {
                    collection: col @ Expr::Identifier(c),
                    value,
                } if *c == s => {
                    allowed.insert(*col as *const Expr as usize);
                    items = items.store(
                        oracle
                            .expr_int_range(value)
                            .map(|(lo, hi)| Span::level(lo as i128, hi as i128)),
                    );
                }
                Stmt::Let { var, .. } if *var == s => ok = false,
                Stmt::Set { target, .. } if *target == s => ok = false,
                Stmt::Swap { a, b } if *a == s || *b == s => ok = false,
                Stmt::FunctionDef { .. } => return Walk::Past,
                _ => {}
            },
            Visited::Expr(expr) => match expr {
                Expr::Index {
                    collection: col @ Expr::Identifier(c),
                    ..
                }
                | Expr::Length {
                    collection: col @ Expr::Identifier(c),
                } if *c == s => {
                    allowed.insert(*col as *const Expr as usize);
                }
                Expr::Identifier(c) if *c == s => seen.push(expr as *const Expr as usize),
                Expr::Closure { .. } => {
                    ok = false;
                    return Walk::Past;
                }
                _ => {}
            },
        }
        Walk::Into
    });
    (ok && seen.iter().all(|a| allowed.contains(a)) && items != Items::Lost).then_some(items)
}

/// One pass, run abstractly.
struct Pass<'o> {
    oracle: &'o OracleFacts,
    followed: &'o HashSet<Symbol>,
    /// The followed sequences the loop carries from pass to pass (the others the pass creates afresh).
    carried: &'o HashSet<Symbol>,
    /// The followed sequences a loop enclosing the current statement stores into: a read of one sees what any of
    /// that loop's earlier iterations stored, so it is unknown.
    unstable: HashSet<Symbol>,
    env: HashMap<Symbol, Span>,
    items: HashMap<Symbol, Items>,
    /// Every Int op met, with the span of its value joined over every time it was met.
    spans: HashMap<usize, Span>,
    /// The Int ops whose value was unknown at least once.
    unknown: HashSet<usize>,
    ok: bool,
}

impl<'o> Pass<'o> {
    fn new(
        oracle: &'o OracleFacts,
        followed: &'o HashSet<Symbol>,
        carried: &'o HashSet<Symbol>,
    ) -> Pass<'o> {
        Pass {
            oracle,
            followed,
            carried,
            unstable: HashSet::new(),
            env: HashMap::new(),
            items: HashMap::new(),
            spans: HashMap::new(),
            unknown: HashSet::new(),
            ok: true,
        }
    }

    fn fork(&self) -> Pass<'o> {
        Pass {
            oracle: self.oracle,
            followed: self.followed,
            carried: self.carried,
            unstable: self.unstable.clone(),
            env: self.env.clone(),
            items: self.items.clone(),
            spans: HashMap::new(),
            unknown: HashSet::new(),
            ok: true,
        }
    }

    /// Take back what an arm met, and join the names and items it leaves with `others`'.
    fn merge(&mut self, arms: Vec<Pass<'o>>) {
        let mut env: Option<HashMap<Symbol, Span>> = None;
        let mut items: Option<HashMap<Symbol, Items>> = None;
        for arm in arms {
            self.ok &= arm.ok;
            for (a, s) in arm.spans {
                let joined = self.spans.get(&a).map_or(s, |o| o.join(s));
                self.spans.insert(a, joined);
            }
            self.unknown.extend(arm.unknown);
            env = Some(match env {
                None => arm.env,
                Some(e) => e
                    .into_iter()
                    .filter_map(|(k, v)| arm.env.get(&k).map(|w| (k, v.join(*w))))
                    .collect(),
            });
            items = Some(match items {
                None => arm.items,
                Some(i) => i
                    .into_iter()
                    .map(|(k, v)| (k, v.join(arm.items.get(&k).copied().unwrap_or(Items::Lost))))
                    .collect(),
            });
        }
        self.env = env.unwrap_or_default();
        if let Some(items) = items {
            self.items = items;
        }
    }

    fn record(&mut self, e: &Expr, v: Option<Span>) {
        let addr = e as *const Expr as usize;
        match v {
            Some(v) => {
                let joined = self.spans.get(&addr).map_or(v, |o| o.join(v));
                self.spans.insert(addr, joined);
            }
            None => {
                self.unknown.insert(addr);
            }
        }
    }

    /// The values `e` takes: the range oracle's interval when it has a finite one (it holds on every run), and
    /// otherwise what this pass follows.
    fn value(&mut self, e: &Expr) -> Option<Span> {
        let followed = match e {
            Expr::Literal(Literal::Number(n)) => Some(Span::level(*n as i128, *n as i128)),
            Expr::Identifier(x) => self.env.get(x).copied(),
            Expr::Index { collection, index } => {
                self.value(index);
                match collection {
                    Expr::Identifier(s) if self.followed.contains(s) && !self.unstable.contains(s) => {
                        match self.items.get(s).copied() {
                            Some(Items::Within(span)) => Some(span),
                            _ => None,
                        }
                    }
                    _ => None,
                }
            }
            Expr::BinaryOp { op, left, right } => {
                let (l, r) = (self.value(left), self.value(right));
                let v = match op {
                    BinaryOpKind::Add => l.zip(r).and_then(|(l, r)| l.add(r)),
                    BinaryOpKind::Subtract => l.zip(r).and_then(|(l, r)| l.sub(r)),
                    BinaryOpKind::Multiply => l.zip(r).and_then(|(l, r)| l.mul(r)),
                    BinaryOpKind::Modulo => match right {
                        Expr::Literal(Literal::Number(m)) if *m > 0 => {
                            let reach = (*m as i128) - 1;
                            let nonneg = l.is_some_and(|l| l.lo.c >= 0 && l.lo.s >= 0);
                            Some(Span::level(if nonneg { 0 } else { -reach }, reach))
                        }
                        _ => None,
                    },
                    _ => None,
                };
                let v = self.oracle_level(e).or(v);
                if matches!(
                    op,
                    BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply
                ) {
                    self.record(e, v);
                }
                return v;
            }
            other => {
                for child in logicaffeine_language::ast_depth::expr_children(other) {
                    self.value(child);
                }
                None
            }
        };
        self.oracle_level(e).or(followed)
    }

    fn oracle_level(&self, e: &Expr) -> Option<Span> {
        self.oracle
            .expr_int_range(e)
            .map(|(lo, hi)| Span::level(lo as i128, hi as i128))
    }

    fn store(&mut self, s: Symbol, v: Option<Span>) {
        if self.followed.contains(&s) {
            let now = self.items.get(&s).copied().unwrap_or(Items::Empty);
            self.items.insert(s, now.store(v));
        }
    }

    fn block(&mut self, stmts: &[Stmt]) {
        for s in stmts {
            if !self.ok {
                return;
            }
            self.stmt(s);
        }
    }

    fn forget(&mut self, w: &WriteSet) {
        for s in w.symbols() {
            self.env.remove(&s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { var, value, .. } | Stmt::Set { target: var, value }
                if self.followed.contains(var) =>
            {
                match without_capacity(value) {
                    Expr::New { .. } => {
                        self.items.insert(*var, Items::Empty);
                    }
                    Expr::Identifier(from) if self.followed.contains(from) => {
                        let rolled = self.items.get(from).copied().unwrap_or(Items::Lost);
                        self.items.insert(*var, rolled);
                    }
                    _ => self.ok = false,
                }
            }
            Stmt::Let { var, value, .. } | Stmt::Set { target: var, value } => {
                match self.value(value) {
                    Some(v) => {
                        self.env.insert(*var, v);
                    }
                    None => {
                        self.env.remove(var);
                    }
                }
            }
            Stmt::SetIndex {
                collection,
                index,
                value,
            } => {
                self.value(index);
                let v = self.value(value);
                match collection {
                    Expr::Identifier(c) => self.store(*c, v),
                    other => {
                        self.value(other);
                    }
                }
            }
            Stmt::Push { value, collection } => {
                let v = self.value(value);
                match collection {
                    Expr::Identifier(c) => self.store(*c, v),
                    other => {
                        self.value(other);
                    }
                }
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                self.value(cond);
                let mut then_arm = self.fork();
                then_arm.block(then_block);
                let mut else_arm = self.fork();
                if let Some(e) = else_block {
                    else_arm.block(e);
                }
                self.merge(vec![then_arm, else_arm]);
            }
            Stmt::While { cond, body, .. } => self.nested_loop(Some(cond), body, s),
            Stmt::Repeat { iterable, body, .. } => {
                self.value(iterable);
                self.nested_loop(None, body, s)
            }
            other => {
                let w = WriteSet::of_stmt(other);
                if w.is_opaque() || w.mutated().iter().any(|m| self.followed.contains(m)) {
                    self.ok = false;
                    return;
                }
                walk_nodes(std::slice::from_ref(other), |node| {
                    if let Visited::Expr(x) = node {
                        if matches!(
                            x,
                            Expr::BinaryOp {
                                op: BinaryOpKind::Add
                                    | BinaryOpKind::Subtract
                                    | BinaryOpKind::Multiply,
                                ..
                            }
                        ) {
                            let addr = x as *const Expr as usize;
                            self.unknown.insert(addr);
                        }
                    }
                    Walk::Into
                });
                self.forget(&w);
            }
        }
    }

    /// A loop inside the pass: its body once, with every name it writes forgotten before and after, so a value one
    /// of its iterations hands the next is the oracle's. A sequence it stores into must be one the pass creates
    /// afresh, and no store into it may read it, or each iteration could grow an item again.
    fn nested_loop(&mut self, cond: Option<&Expr>, body: &[Stmt], whole: &Stmt) {
        let w = WriteSet::of_stmt(whole);
        if w.is_opaque() {
            self.ok = false;
            return;
        }
        for m in w.mutated() {
            if self.followed.contains(m) && (self.carried.contains(m) || stores_read_back(*m, body)) {
                self.ok = false;
                return;
            }
        }
        self.forget(&w);
        let stored: Vec<Symbol> = w
            .mutated()
            .iter()
            .filter(|m| self.followed.contains(m) && self.unstable.insert(**m))
            .copied()
            .collect();
        if let Some(c) = cond {
            self.value(c);
        }
        self.block(body);
        for m in stored {
            self.unstable.remove(&m);
        }
        self.forget(&w);
    }
}

/// Whether some store or push into `s` within `body` reads `s`.
fn stores_read_back(s: Symbol, body: &[Stmt]) -> bool {
    let mut reads_back = false;
    walk_nodes(body, |node| {
        if let Visited::Stmt(
            Stmt::SetIndex {
                collection: Expr::Identifier(c),
                value,
                ..
            }
            | Stmt::Push {
                collection: Expr::Identifier(c),
                value,
            },
        ) = node
        {
            if *c == s && mentions(value, s) {
                reads_back = true;
            }
        }
        Walk::Into
    });
    reads_back
}

/// Every name `e` reads.
fn names_in(e: &Expr) -> Vec<Symbol> {
    let mut out = Vec::new();
    if let Expr::Identifier(x) = e {
        out.push(*x);
    }
    for c in logicaffeine_language::ast_depth::expr_children(e) {
        out.extend(names_in(c));
    }
    out
}

/// Whether evaluating `e` calls anything.
fn mentions_call(e: &Expr) -> bool {
    matches!(e, Expr::Call { .. } | Expr::CallExpr { .. })
        || logicaffeine_language::ast_depth::expr_children(e)
            .into_iter()
            .any(mentions_call)
}

/// Whether evaluating `e` reads the name `s`.
fn mentions(e: &Expr, s: Symbol) -> bool {
    matches!(e, Expr::Identifier(x) if *x == s)
        || logicaffeine_language::ast_depth::expr_children(e)
            .into_iter()
            .any(|c| mentions(c, s))
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/item_growth/tests.rs"]
mod tests;
