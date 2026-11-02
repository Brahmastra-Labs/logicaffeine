//! One versioning pass: a region that runs faster under facts one hoisted check can establish runs as
//!
//! ```text
//! If P:
//!     <the region, cloned; what P proves about the clone recorded against the clone's own nodes>
//! Otherwise:
//!     <the region as written, every check in place>
//! ```
//!
//! Every backend compiles the `If` like any other, so every backend runs one program; a backend that reads the
//! recorded facts ([`super::facts`]) lowers the clone without the checks they discharge.
//!
//! Contract, each rule load-bearing:
//! - **P cannot fail, on any backend.** P opens with a magnitude guard on every name it reads (`-2^30 <= v <=
//!   2^30`, joined by short-circuit `and`), and the comparisons after it are accepted only when interval arithmetic
//!   over those guarded ranges keeps every node they evaluate inside i64. A value too large for the guard makes P
//!   false, and the region runs as written; no operation in P can overflow, divide or index.
//! - **P reads only what the region does not write.** Every name in P is fixed before the region and not
//!   written inside it, so P, evaluated once at entry, says what holds on every iteration.
//! - **The original is the program.** The `Otherwise` branch is the region's own statements: when P is false the
//!   program runs, and fails, exactly as written.
//! - **One version per outermost region.** A versioned region is not searched again, so code grows by at most one
//!   clone per outermost region.
//! - **Facts are keyed by the clone's own nodes.** The clone is a deep copy with fresh nodes, leaves included,
//!   so a fact recorded for the clone can never answer for the original.
//!
//! Clients propose regions; legality is not theirs. [`super::dependence::tile`] supplies a nest's verdict and the
//! preconditions under which tile order is invisible.

use std::collections::HashMap;

use crate::analysis::types::TypeEnv;
use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use logicaffeine_language::ast_map::{self, Arenas, AstMap};

use super::dependence::{bounds, nest_at, tile, Bound, Legality, Nest, Precondition};
use super::facts::{self, Fact};
use super::kinds::Kinds;

/// The magnitude every name P reads is guarded to: `|v| <= 2^30`.
const GUARD: i64 = 1 << 30;

/// Version every outermost tileable nest in `stmts` (see the module doc). `env` is the checked program's type
/// environment: the kinds of its names ([`super::kinds`]) come from it and from the statements.
pub(crate) fn version_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    arenas: Arenas<'a>,
    env: &TypeEnv,
    interner: &Interner,
    cfg: &crate::optimization::OptimizationConfig,
) -> Vec<Stmt<'a>> {
    // Tiling is `## No tile`'s; versioning a loop for its bounds alone is the bounds-check elision's, `## No
    // oraclehints`; for the arithmetic on items it grows, the bounded accumulators', `## No accumbound`.
    let (tiling, bounding, growing) = (
        cfg.is_on(crate::optimization::Opt::Tile),
        cfg.is_on(crate::optimization::Opt::OracleHints),
        cfg.is_on(crate::optimization::Opt::AccumBound),
    );
    if !tiling && !bounding && !growing {
        return stmts;
    }
    let kinds = super::kinds::of_program(&stmts, env, interner);
    // What the range oracle already proves needs no version; past codegen's own oracle limit nothing is versioned
    // for its bounds alone, so code never grows blind.
    const MAX_ORACLE_STMTS: usize = 5_000;
    let oracle = (stmts.len() <= MAX_ORACLE_STMTS)
        .then(|| super::oracle_analyze_with_entry_guards(&stmts, interner));
    let stmt_arena = arenas.stmts;
    let mut walk = Versioner {
        arenas,
        kinds: &kinds,
        env,
        oracle: oracle.as_ref(),
        tiling,
        bounding,
        growing,
    };
    walk.map_block(stmt_arena.alloc_slice(stmts)).to_vec()
}

/// The walk: every block of the program, every expression node kept as it is.
struct Versioner<'a, 'k> {
    arenas: Arenas<'a>,
    kinds: &'k Kinds,
    env: &'k TypeEnv,
    oracle: Option<&'k super::OracleFacts>,
    tiling: bool,
    bounding: bool,
    growing: bool,
}

impl<'a, 'k> AstMap<'a> for Versioner<'a, 'k> {
    fn arenas(&self) -> Arenas<'a> {
        self.arenas
    }

    fn map_expr(&mut self, e: &'a Expr<'a>) -> &'a Expr<'a> {
        e
    }

    fn map_type(&mut self, t: &'a TypeExpr<'a>) -> &'a TypeExpr<'a> {
        t
    }

    fn map_block(&mut self, block: &'a [Stmt<'a>]) -> &'a [Stmt<'a>] {
        let mut out = Vec::with_capacity(block.len());
        let mut i = 0;
        while i < block.len() {
            if let Some(nest) = nest_at(block, i) {
                let (kinds, env) = (self.kinds, self.env);
                let int_valued = |e: &Expr| kinds.int_valued(e, env);
                let seq = |s: Symbol| kinds.seq(s);
                let reference = !crate::semantics::collections::value_semantics_enabled();
                let rows = |s: Symbol| !reference || kinds.rows_unaliased(s);
                let order = if self.tiling && nest.loops.len() >= 2 {
                    tile(&nest, &int_valued, &seq, &rows)
                } else {
                    Legality::Never("not tiled")
                };
                if std::env::var_os("LOGOS_DEBUG_VERSION").is_some() && nest.loops.len() >= 2 {
                    match &order {
                        Legality::Always => {
                            eprintln!("version: {}-loop tile is always legal", nest.loops.len())
                        }
                        Legality::When(pre) => eprintln!(
                            "version: {}-loop tile is guarded by {} preconditions",
                            nest.loops.len(),
                            pre.len()
                        ),
                        Legality::Never(why) => {
                            eprintln!("version: {}-loop tile rejected: {why}", nest.loops.len())
                        }
                    }
                }
                let region = match order {
                    Legality::Always => {
                        facts::mark_expr(Fact::TileLegal, loop_cond(&block[i + 1]));
                        super::mark_fired(crate::optimization::Opt::Tile);
                        Some(vec![block[i].clone(), block[i + 1].clone()])
                    }
                    Legality::When(pre) => {
                        self.version(&block[i], &block[i + 1], &nest, &pre, true)
                    }
                    Legality::Never(_) => match self.oracle.filter(|_| self.bounding) {
                        Some(oracle) => match bounds(&nest, &int_valued, &seq, &|c, i| {
                            oracle.index_always_in_bounds(c, i)
                        }) {
                            Ok((pre, true)) if !pre.is_empty() => {
                                self.version(&block[i], &block[i + 1], &nest, &pre, false)
                            }
                            _ => None,
                        },
                        None => None,
                    },
                };
                let region = region.or_else(|| self.grow(block, i));
                if let Some(region) = region {
                    out.extend(region);
                    i += 2;
                    continue;
                }
            }
            out.push(self.map_stmt(&block[i]));
            i += 1;
        }
        self.arenas.stmts.alloc_slice(out)
    }
}

impl<'a, 'k> Versioner<'a, 'k> {
    /// `init; loop` as `If P: <clone> Otherwise: <as written>`, or `None` when no total P states `pre`. The clone's
    /// accesses hold [`Fact::InBounds`]; its band holds [`Fact::TileLegal`] too when `tile_legal`.
    fn version(
        &mut self,
        init: &Stmt<'a>,
        lp: &Stmt<'a>,
        nest: &Nest<'a>,
        pre: &[Precondition<'a>],
        tile_legal: bool,
    ) -> Option<Vec<Stmt<'a>>> {
        let ea = self.arenas.exprs;
        let mut checks = Vec::new();
        for p in pre {
            match p {
                Precondition::InRange {
                    seq,
                    least,
                    greatest,
                } => {
                    let length = ea.alloc(Expr::Length {
                        collection: ea.alloc(Expr::Identifier(*seq)),
                    });
                    // A least corner every guarded value keeps at 1 or more needs no check.
                    let least = bound_expr(ea, least)?;
                    if !interval(least, &|_| Some((-(GUARD as i128), GUARD as i128)))
                        .is_some_and(|r| r.0 >= 1)
                    {
                        checks.push(cmp(ea, BinaryOpKind::GtEq, least, num(ea, 1)));
                    }
                    checks.push(cmp(
                        ea,
                        BinaryOpKind::LtEq,
                        bound_expr(ea, greatest)?,
                        length,
                    ));
                }
                Precondition::NonZero(d) => {
                    checks.push(cmp(ea, BinaryOpKind::NotEq, d, num(ea, 0)))
                }
            }
        }
        let guard = total_guard(ea, &checks)?;
        let mut guarded = Vec::new();
        for c in &checks {
            collect_names(c, &mut guarded);
        }

        let mut clone = Cloner {
            arenas: self.arenas,
            raw: None,
        };
        let fast_loop = clone.map_stmt(lp);
        record_clone_facts(&fast_loop, nest, &guarded, tile_legal);
        let (counter, start) = match init {
            Stmt::Let { var, value, .. } | Stmt::Set { target: var, value } => (*var, *value),
            _ => return None,
        };
        let fast_init = Stmt::Set {
            target: counter,
            value: clone.map_expr(start),
        };
        if tile_legal {
            super::mark_fired(crate::optimization::Opt::Tile);
        }
        Some(self.branches(init, guard, fast_init, fast_loop, lp))
    }

    /// `init; loop` as `If <limit> <= C: <clone, the arithmetic on the items it grows raw> Otherwise: <as
    /// written>`, when that arithmetic stays within i64 for as many passes as `C` allows ([`super::item_growth`]).
    /// P is one comparison of the loop's own limit, which the loop evaluates before its first pass anyway.
    fn grow(&mut self, block: &'a [Stmt<'a>], at: usize) -> Option<Vec<Stmt<'a>>> {
        if !self.growing {
            return None;
        }
        let (init, lp) = (&block[at], &block[at + 1]);
        let plan = super::item_growth::plan(&block[..at], init, lp, self.oracle?)?;
        let Stmt::While {
            cond: Expr::BinaryOp { right: limit, .. },
            ..
        } = lp
        else {
            return None;
        };
        let (counter, start) = match init {
            Stmt::Let { var, value, .. } | Stmt::Set { target: var, value } => (*var, *value),
            _ => return None,
        };
        let ea = self.arenas.exprs;
        let mut clone = Cloner {
            arenas: self.arenas,
            raw: Some(&plan.raw),
        };
        let guard = cmp(
            ea,
            BinaryOpKind::LtEq,
            clone.map_expr(limit),
            num(ea, plan.limit_at_most),
        );
        let fast_loop = clone.map_stmt(lp);
        let fast_init = Stmt::Set {
            target: counter,
            value: clone.map_expr(start),
        };
        super::mark_fired(crate::optimization::Opt::AccumBound);
        Some(self.branches(init, guard, fast_init, fast_loop, lp))
    }

    /// `If guard: <fast_init; fast_loop> Otherwise: <init's value restated; lp>`. The counter is re-stated at the
    /// top of each branch, so each branch keeps the counted-loop shape the backends lower; a `Let` is hoisted above
    /// the guard so the counter stays in the enclosing scope.
    fn branches(
        &self,
        init: &Stmt<'a>,
        guard: &'a Expr<'a>,
        fast_init: Stmt<'a>,
        fast_loop: Stmt<'a>,
        lp: &Stmt<'a>,
    ) -> Vec<Stmt<'a>> {
        let (counter, start) = match init {
            Stmt::Let { var, value, .. } | Stmt::Set { target: var, value } => (*var, *value),
            _ => unreachable!("a nest's init is a Let or a Set"),
        };
        let slow_init = Stmt::Set {
            target: counter,
            value: start,
        };
        let sa = self.arenas.stmts;
        let branches = Stmt::If {
            cond: guard,
            then_block: sa.alloc_slice(vec![fast_init, fast_loop]),
            else_block: Some(sa.alloc_slice(vec![slow_init, lp.clone()])),
        };
        match init {
            Stmt::Let { .. } => vec![init.clone(), branches],
            _ => vec![branches],
        }
    }
}

/// The loop condition a `While` is keyed by: an arena node, stable however the statement moves.
fn loop_cond<'s, 'a>(lp: &'s Stmt<'a>) -> &'a Expr<'a> {
    match lp {
        Stmt::While { cond, .. } => cond,
        _ => unreachable!("a nest's loop is a While"),
    }
}

/// Record what P proves about the clone: its band may run in tile order; every access in it names an item; and
/// the arithmetic of an index stays in i64 when interval arithmetic says so, with each name P guards in
/// `[-2^30, 2^30]` and each counter between its bounds' extremes over those names.
fn record_clone_facts<'a>(
    fast_loop: &Stmt<'a>,
    nest: &Nest<'a>,
    guarded: &[Symbol],
    tile_legal: bool,
) {
    use logicaffeine_language::ast_depth::{for_each_node, Visited};
    if tile_legal {
        facts::mark_expr(Fact::TileLegal, loop_cond(fast_loop));
    }
    let mut body: &[Stmt<'a>] = std::slice::from_ref(fast_loop);
    for _ in 0..nest.loops.len() {
        let Some(Stmt::While { body: inner, .. }) =
            body.iter().find(|s| matches!(s, Stmt::While { .. }))
        else {
            return;
        };
        body = inner;
    }
    let locals: HashMap<Symbol, &'a Expr<'a>> = body
        .iter()
        .filter_map(|s| match s {
            Stmt::Let { var, value, .. } => Some((*var, *value)),
            _ => None,
        })
        .collect();
    let mut indices: Vec<&'a Expr<'a>> = Vec::new();
    for_each_node(body, |node| match node {
        Visited::Expr(Expr::Index {
            collection: Expr::Identifier(_),
            index,
        })
        | Visited::Stmt(Stmt::SetIndex {
            collection: Expr::Identifier(_),
            index,
            ..
        }) => indices.push(index),
        _ => {}
    });
    let name_range = |s: Symbol| {
        guarded
            .contains(&s)
            .then_some((-(GUARD as i128), GUARD as i128))
    };
    let counters: Vec<(Symbol, Option<(i128, i128)>)> = nest
        .loops
        .iter()
        .map(|l| {
            let (lo, hi) = (interval(l.lo, &name_range), interval(l.hi, &name_range));
            (l.counter, lo.zip(hi).map(|(lo, hi)| (lo.0, hi.1)))
        })
        .collect();
    let index_range = |s: Symbol| match counters.iter().find(|(c, _)| *c == s) {
        Some((_, r)) => *r,
        None => name_range(s),
    };
    for index in indices {
        facts::mark_expr(Fact::InBounds, index);
        let value = match index {
            Expr::Identifier(s) => locals.get(s).copied().unwrap_or(index),
            _ => index,
        };
        if interval(value, &index_range).is_some() {
            mark_ops_raw(value);
        }
    }
}

/// Mark every Int op node of `e` as one that stays in i64.
fn mark_ops_raw(e: &Expr) {
    if let Expr::BinaryOp {
        op: BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply,
        left,
        right,
    } = e
    {
        facts::mark_expr(Fact::RawIntOp, e);
        mark_ops_raw(left);
        mark_ops_raw(right);
    }
}

/// A deep copy with fresh nodes everywhere, leaves included. Each copy holds every fact the node it copies holds
/// (the clone runs the same code under a stronger guard, so what held of the original holds of it), and a node in
/// `raw` gets [`Fact::RawIntOp`] as well.
struct Cloner<'a, 'r> {
    arenas: Arenas<'a>,
    raw: Option<&'r std::collections::HashSet<usize>>,
}

impl<'a> AstMap<'a> for Cloner<'a, '_> {
    fn arenas(&self) -> Arenas<'a> {
        self.arenas
    }

    fn map_expr(&mut self, e: &'a Expr<'a>) -> &'a Expr<'a> {
        let ea = self.arenas.exprs;
        let copy = match e {
            Expr::Literal(l) => ea.alloc(Expr::Literal(l.clone())),
            Expr::Identifier(s) => ea.alloc(Expr::Identifier(*s)),
            Expr::OptionNone => ea.alloc(Expr::OptionNone),
            _ => ast_map::walk_expr(self, e),
        };
        facts::copy_expr_facts(e, copy);
        if self
            .raw
            .is_some_and(|raw| raw.contains(&(e as *const Expr as usize)))
        {
            facts::mark_expr(Fact::RawIntOp, copy);
        }
        copy
    }
}

fn num<'a>(ea: &'a Arena<Expr<'a>>, n: i64) -> &'a Expr<'a> {
    ea.alloc(Expr::Literal(Literal::Number(n)))
}

fn cmp<'a>(
    ea: &'a Arena<Expr<'a>>,
    op: BinaryOpKind,
    left: &'a Expr<'a>,
    right: &'a Expr<'a>,
) -> &'a Expr<'a> {
    ea.alloc(Expr::BinaryOp { op, left, right })
}

fn and<'a>(ea: &'a Arena<Expr<'a>>, left: &'a Expr<'a>, right: &'a Expr<'a>) -> &'a Expr<'a> {
    cmp(ea, BinaryOpKind::And, left, right)
}

/// A corner bound as an expression: `base + scale × factor + Σ ±invariant`.
fn bound_expr<'a>(ea: &'a Arena<Expr<'a>>, b: &Bound<'a>) -> Option<&'a Expr<'a>> {
    let base = super::affine::expr_of(&b.base, ea)?;
    let mut sum = match &b.scale {
        None => base,
        Some((scale, factor)) => {
            let product = cmp(
                ea,
                BinaryOpKind::Multiply,
                super::affine::expr_of(scale, ea)?,
                factor,
            );
            cmp(ea, BinaryOpKind::Add, product, base)
        }
    };
    for (positive, term) in &b.invariant {
        let op = if *positive {
            BinaryOpKind::Add
        } else {
            BinaryOpKind::Subtract
        };
        sum = cmp(ea, op, sum, term);
    }
    Some(sum)
}

/// `checks` behind a magnitude guard on every name they read, when interval arithmetic over the guarded ranges
/// keeps every node of every check inside i64; otherwise `None`.
pub(crate) fn total_guard<'a>(
    ea: &'a Arena<Expr<'a>>,
    checks: &[&'a Expr<'a>],
) -> Option<&'a Expr<'a>> {
    let mut names: Vec<Symbol> = Vec::new();
    for c in checks {
        collect_names(c, &mut names);
    }
    if checks.iter().any(|c| !fits(c)) {
        return None;
    }
    let mut parts: Vec<&'a Expr<'a>> = Vec::new();
    for v in names {
        parts.push(cmp(
            ea,
            BinaryOpKind::GtEq,
            ea.alloc(Expr::Identifier(v)),
            num(ea, -GUARD),
        ));
        parts.push(cmp(
            ea,
            BinaryOpKind::LtEq,
            ea.alloc(Expr::Identifier(v)),
            num(ea, GUARD),
        ));
    }
    for c in checks {
        if !parts.iter().any(|p| super::same_expr(p, c)) {
            parts.push(c);
        }
    }
    // Every check is evaluated only after every magnitude guard holds (the conjunction reads left to right and
    // short-circuits), and under the guards each of its nodes stays in i64: its Int ops lower raw.
    for c in checks {
        if let Expr::BinaryOp { left, right, .. } = c {
            mark_ops_raw(left);
            mark_ops_raw(right);
        }
    }
    (!parts.is_empty()).then(|| conjunction(ea, &parts))
}

/// `parts` joined by `and` as a balanced tree, evaluated left to right with short-circuit exactly as the list
/// reads, and only `log2(n)` deep however many parts a guard has.
fn conjunction<'a>(ea: &'a Arena<Expr<'a>>, parts: &[&'a Expr<'a>]) -> &'a Expr<'a> {
    match parts {
        [one] => one,
        _ => {
            let (left, right) = parts.split_at(parts.len() / 2);
            and(ea, conjunction(ea, left), conjunction(ea, right))
        }
    }
}

/// Every name `e` reads, once each, in first-read order; `length of s` reads `s` but its value is a length.
fn collect_names(e: &Expr, out: &mut Vec<Symbol>) {
    match e {
        Expr::Identifier(s) if !out.contains(s) => out.push(*s),
        Expr::BinaryOp { left, right, .. } => {
            collect_names(left, out);
            collect_names(right, out);
        }
        Expr::Not { operand } => collect_names(operand, out),
        _ => {}
    }
}

/// Whether every node of the comparison `e` stays inside i64 with each name in `[-2^30, 2^30]` and each length in
/// `[0, 2^62]`.
fn fits(e: &Expr) -> bool {
    interval(e, &|_| Some((-(GUARD as i128), GUARD as i128))).is_some()
}

/// The range of `e` by interval arithmetic, each name's range from `name_range` and each length in `[0, 2^62]`;
/// `None` when a name has no range, a node is not one this reads, or any node leaves i64.
fn interval(e: &Expr, name_range: &dyn Fn(Symbol) -> Option<(i128, i128)>) -> Option<(i128, i128)> {
    let r = match e {
        Expr::Literal(Literal::Number(n)) => (*n as i128, *n as i128),
        Expr::Identifier(s) => name_range(*s)?,
        Expr::Length { .. } => (0, 1i128 << 62),
        Expr::BinaryOp { op, left, right } => {
            let (a, b) = (interval(left, name_range)?, interval(right, name_range)?);
            match op {
                BinaryOpKind::Add => (a.0 + b.0, a.1 + b.1),
                BinaryOpKind::Subtract => (a.0 - b.1, a.1 - b.0),
                BinaryOpKind::Multiply => {
                    let ps = [a.0 * b.0, a.0 * b.1, a.1 * b.0, a.1 * b.1];
                    (*ps.iter().min().unwrap(), *ps.iter().max().unwrap())
                }
                BinaryOpKind::Lt
                | BinaryOpKind::LtEq
                | BinaryOpKind::Gt
                | BinaryOpKind::GtEq
                | BinaryOpKind::Eq
                | BinaryOpKind::NotEq => (0, 1),
                _ => return None,
            }
        }
        _ => return None,
    };
    (r.0 >= i64::MIN as i128 && r.1 <= i64::MAX as i128).then_some(r)
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/version/tests.rs"]
mod tests;
