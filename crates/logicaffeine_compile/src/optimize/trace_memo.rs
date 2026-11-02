//! A pure iteration run from every start of a counted range, its trace cut off where it reaches a start already
//! finished (`Opt::TraceMemo`).
//!
//! collatz counts, for every start `i` up to `n`, the steps of the iteration from `i` to 1:
//!
//! ```text
//! While i is at most n:
//!     Let k be i.
//!     While k is not 1:
//!         If k % 2 equals 0:
//!             Set k to k / 2.
//!         Otherwise:
//!             Set k to 3 * k + 1.
//!         Set total to total + 1.
//!     Set i to i + 1.
//! ```
//!
//! The inner loop's state is `k` alone: its condition and its step read only `k` and values fixed for the whole range,
//! and its only other effect adds a constant to an accumulator. So what a trace adds from a given `k` is a function of
//! `k`, `F(k)`, and each start that finished left `F(start)` behind. The loop keeps those in a table, and a trace that
//! reaches a finished start `v` (one in `[lo, i)`) adds `F(v)` and stops:
//!
//! ```text
//!     Let k be i.
//!     Let base be total.
//!     While k is not 1:
//!         <the step>
//!         Set total to total + 1.
//!         If k >= lo and k < i:
//!             Set total to total + item (k - lo + 1) of table.
//!             Break.
//!     Push total - base to table.
//! ```
//!
//! The rewrite holds when the loop counts `i` up by one over Ints; its body holds `Let k be i` directly followed by
//! `While C` and nothing after reads `k`; `C` and every step read only `k`, the inner loop's own `Let`s and names the
//! range never writes, through arithmetic, comparisons and `not`; the inner body is `Set k`s, its own `Let`s and `Set`s
//! of them, `If`s of the same, and `Set acc to acc + H` with `H` a non-negative integer literal, `acc` an Int read
//! nowhere else in the inner loop; and some step can shrink `k` (a division, a remainder, a subtraction, a right
//! shift), so a trace can come back.
//!
//! A step of `k` that could fail already ran without failing on the trace the table entry came from, so cutting it
//! off loses no failure, and a trace that never comes back never meets the table. The accumulator only grows, so
//! adding a finished trace at once reaches no value the steps it stands for did not reach. One hoisted check, built
//! by [`super::version::total_guard`], bounds the table: at most 2^24 starts. Otherwise the loop runs as written. A
//! start whose sum reaches 65535 is entered as unknown, so a trace that reaches it keeps stepping; every entry then fits
//! the 32 bits the sequence narrowing gives the table.

use crate::analysis::types::TypeEnv;
use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::loop_shape::{extract_counted_repeat, extract_counted_while, CountedLoop};
use crate::optimization::Opt;

use super::ast_build::Build;
use super::kinds::Kinds;
use super::writes::WriteSet;

/// The most starts the table holds: 2^24.
const TABLE_CAP: i64 = 1 << 24;

/// An entry for a start whose sum is this or more, or below zero: unknown, so a trace that reaches it keeps stepping.
/// Every entry is then below 2^16, which the sequence narrowing holds in 32 bits.
const UNKNOWN: i64 = 65535;

/// Rewrite every memoizable iteration in `stmts`; true when one was rewritten.
pub(crate) fn trace_memo_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    let env = TypeEnv::infer_program(&stmts, interner, &crate::analysis::TypeRegistry::new());
    let kinds = super::kinds::of_program(&stmts, &env, interner);
    let mut cx = Memoizer {
        b: Build {
            ea: expr_arena,
            sa: stmt_arena,
        },
        kinds: &kinds,
        env: &env,
        fired: false,
        tables: 0,
    };
    let out =
        super::ast_build::rewrite_blocks(&stmts, stmt_arena, Opt::TraceMemo, &mut |block, at| {
            let pair: Vec<&Stmt<'a>> = block[at..block.len().min(at + 2)].iter().collect();
            if let Some((outer, span)) = extract_counted_while(&pair, 0) {
                if let Some(memo) = cx.rewrite(Some(&block[at]), &block[at + 1], &outer, interner) {
                    return Some((memo, span));
                }
            }
            let outer = extract_counted_repeat(&block[at])?;
            cx.rewrite(None, &block[at], &outer, interner)
                .map(|memo| (memo, 1))
        });
    (out, cx.fired)
}

/// An iteration at `body[at]` (`Let k be i`) and `body[at + 1]` (its loop), and the accumulator it adds to.
struct Trace {
    at: usize,
    acc: Symbol,
}

struct Memoizer<'a, 'k> {
    b: Build<'a>,
    kinds: &'k Kinds,
    env: &'k TypeEnv,
    fired: bool,
    tables: usize,
}

impl<'a, 'k> Memoizer<'a, 'k> {
    /// The loop `lp` (after `init` for a `While`) as `If P: <the loop with its traces cut off> Otherwise: <the loop>`,
    /// or `None` when its body holds no memoizable iteration.
    fn rewrite(
        &mut self,
        init: Option<&Stmt<'a>>,
        lp: &Stmt<'a>,
        outer: &CountedLoop<'a>,
        it: &mut Interner,
    ) -> Option<Vec<Stmt<'a>>> {
        let i = outer.var;
        let ints = |e: &Expr| self.kinds.int_valued(e, self.env);
        if (init.is_some() && !self.kinds.int(i)) || !ints(outer.start) || !ints(outer.limit) {
            return None;
        }
        let writes = WriteSet::of_stmt(lp);
        let body = outer.body_without_increment;
        let trace =
            (0..body.len().saturating_sub(1)).find_map(|at| trace_at(body, at, i, &writes))?;
        if !self.kinds.int(trace.acc) {
            return None;
        }
        let Stmt::While {
            cond: inner_cond,
            body: inner_body,
            decreasing,
        } = &body[trace.at + 1]
        else {
            return None;
        };
        let Stmt::Let { var: k, .. } = &body[trace.at] else {
            return None;
        };
        let k = *k;

        // At most TABLE_CAP starts: the range's last less its first, below the cap. The first is `i` where the loop
        // starts, read once on entry; both are fresh copies of the loop's own expressions.
        let b = self.b;
        let last = |b: Build<'a>| -> Option<&'a Expr<'a>> {
            let limit = b.copy(outer.limit)?;
            Some(if outer.inclusive {
                limit
            } else {
                b.bin(BinaryOpKind::Subtract, limit, b.num(1))
            })
        };
        let span = b.bin(BinaryOpKind::Subtract, last(b)?, b.id(i));
        let guard = super::version::total_guard(
            b.ea,
            &[b.bin(BinaryOpKind::LtEq, span, b.num(TABLE_CAP))],
        )?;

        let n = self.tables;
        self.tables += 1;
        let mut fresh = |name: &str| it.intern(&format!("__tracememo{n}_{name}"));
        let (table, lo, base, entry, sum) = (
            fresh("table"),
            fresh("lo"),
            fresh("base"),
            fresh("entry"),
            fresh("sum"),
        );
        let (seq, int) = (it.intern("Seq"), it.intern("Int"));

        // The step, then: a trace standing on a finished start whose sum is known adds it and stops.
        let finished = b.if_(
            b.bin(BinaryOpKind::GtEq, b.id(k), b.id(lo)),
            vec![b.if_(
                b.bin(BinaryOpKind::Lt, b.id(k), b.id(i)),
                vec![
                    b.let_(
                        entry,
                        b.item(
                            table,
                            b.bin(
                                BinaryOpKind::Add,
                                b.bin(BinaryOpKind::Subtract, b.id(k), b.id(lo)),
                                b.num(1),
                            ),
                        ),
                        false,
                    ),
                    b.if_(
                        b.bin(BinaryOpKind::Lt, b.id(entry), b.num(UNKNOWN)),
                        vec![
                            b.set(
                                trace.acc,
                                b.bin(BinaryOpKind::Add, b.id(trace.acc), b.id(entry)),
                            ),
                            Stmt::Break,
                        ],
                        None,
                    ),
                ],
                None,
            )],
            None,
        );
        let mut cut_body: Vec<Stmt<'a>> = inner_body.to_vec();
        cut_body.push(finished);
        let cut_loop = Stmt::While {
            cond: *inner_cond,
            body: b.block(cut_body),
            decreasing: *decreasing,
        };
        let memo_body = |whole: &[Stmt<'a>]| -> Vec<Stmt<'a>> {
            let mut out: Vec<Stmt<'a>> = whole[..=trace.at].to_vec();
            out.push(b.let_(base, b.id(trace.acc), false));
            out.push(cut_loop.clone());
            // What this start added, or unknown past the entries' range.
            out.push(b.let_(sum, b.num(UNKNOWN), true));
            let added = || b.bin(BinaryOpKind::Subtract, b.id(trace.acc), b.id(base));
            out.push(b.if_(
                b.bin(BinaryOpKind::GtEq, added(), b.num(0)),
                vec![b.if_(
                    b.bin(BinaryOpKind::Lt, added(), b.num(UNKNOWN)),
                    vec![b.set(sum, added())],
                    None,
                )],
                None,
            ));
            out.push(b.push(b.id(sum), table));
            out.extend(whole[trace.at + 2..].iter().cloned());
            out
        };
        let memo_loop = match lp {
            Stmt::While {
                cond,
                body: whole,
                decreasing,
            } => Stmt::While {
                cond: *cond,
                body: b.block(memo_body(whole)),
                decreasing: *decreasing,
            },
            Stmt::Repeat {
                pattern,
                iterable,
                body: whole,
            } => Stmt::Repeat {
                pattern: pattern.clone(),
                iterable: *iterable,
                body: b.block(memo_body(whole)),
            },
            _ => return None,
        };
        // A `Repeat` counter is bound by the loop; the first start is its range's start.
        let first = match init {
            Some(_) => b.id(i),
            None => b.copy(outer.start)?,
        };
        let fast = vec![
            b.let_(lo, first, false),
            b.let_(table, b.new_seq(seq, TypeExpr::Primitive(int)), true),
            memo_loop,
        ];
        let guard = match init {
            Some(_) => guard,
            None => {
                // Before a `Repeat`, the range's first start stands where `i` will.
                let span = b.bin(BinaryOpKind::Subtract, last(b)?, b.copy(outer.start)?);
                super::version::total_guard(
                    b.ea,
                    &[b.bin(BinaryOpKind::LtEq, span, b.num(TABLE_CAP))],
                )?
            }
        };
        let branches = b.if_(guard, fast, Some(vec![lp.clone()]));
        self.fired = true;
        Some(match init {
            Some(init) => vec![init.clone(), branches],
            None => vec![branches],
        })
    }
}

/// The iteration starting at `body[at]` for the range's counter `i`, by the shape the module documents; `writes` is
/// what the range writes.
fn trace_at(body: &[Stmt], at: usize, i: Symbol, writes: &WriteSet) -> Option<Trace> {
    let Stmt::Let {
        var: k,
        value: Expr::Identifier(start),
        ..
    } = &body[at]
    else {
        return None;
    };
    let k = *k;
    if *start != i || k == i {
        return None;
    }
    let Stmt::While {
        cond, body: inner, ..
    } = &body[at + 1]
    else {
        return None;
    };
    if body[at + 2..]
        .iter()
        .any(|s| super::mentions::stmt_touches(s, k))
    {
        return None;
    }
    let mut cx = Inner {
        k,
        i,
        writes,
        acc: None,
        locals: Vec::new(),
        shrinks: false,
    };
    if !cx.pure(cond) || !cx.block(inner) {
        return None;
    }
    let acc = cx.acc?;
    if !cx.shrinks
        || [k, i].contains(&acc)
        || cx.locals.contains(&acc)
        || super::mentions::expr_touches(cond, acc)
    {
        return None;
    }
    Some(Trace { at, acc })
}

/// What the inner loop's statements may read and write.
struct Inner<'w> {
    k: Symbol,
    i: Symbol,
    writes: &'w WriteSet,
    acc: Option<Symbol>,
    locals: Vec<Symbol>,
    shrinks: bool,
}

impl Inner<'_> {
    fn block(&mut self, block: &[Stmt]) -> bool {
        block.iter().all(|s| self.stmt(s))
    }

    fn stmt(&mut self, s: &Stmt) -> bool {
        match s {
            Stmt::Set { target, value } if *target == self.k => {
                if !self.pure(value) {
                    return false;
                }
                self.shrinks |= can_shrink(value);
                true
            }
            Stmt::Set {
                target,
                value:
                    Expr::BinaryOp {
                        op: BinaryOpKind::Add,
                        left: Expr::Identifier(a),
                        right: Expr::Literal(Literal::Number(h)),
                    },
            } if target == a && *h >= 0 && *target != self.i && !self.locals.contains(target) => {
                *self.acc.get_or_insert(*target) == *target
            }
            Stmt::Let { var, value, .. } => {
                if [self.k, self.i].contains(var) || self.acc == Some(*var) || !self.pure(value) {
                    return false;
                }
                self.shrinks |= can_shrink(value);
                self.locals.push(*var);
                true
            }
            Stmt::Set { target, value } if self.locals.contains(target) => {
                self.shrinks |= can_shrink(value);
                self.pure(value)
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                self.pure(cond)
                    && self.block(then_block)
                    && else_block.map_or(true, |b| self.block(b))
            }
            _ => false,
        }
    }

    /// `e` reads only `k`, the inner loop's own locals and names the range never writes, through arithmetic,
    /// comparisons and `not`, and never the accumulator.
    fn pure(&self, e: &Expr) -> bool {
        use BinaryOpKind::*;
        match e {
            Expr::Literal(Literal::Number(_) | Literal::Boolean(_)) => true,
            Expr::Identifier(s) => {
                *s == self.k
                    || self.locals.contains(s)
                    || (self.acc != Some(*s) && !self.writes.may_write(*s))
            }
            Expr::BinaryOp { op, left, right } => {
                matches!(
                    op,
                    Add | Subtract
                        | Multiply
                        | Divide
                        | FloorDivide
                        | Modulo
                        | Eq
                        | NotEq
                        | Lt
                        | Gt
                        | LtEq
                        | GtEq
                        | And
                        | Or
                        | BitAnd
                        | BitOr
                        | BitXor
                        | Shl
                        | Shr
                ) && self.pure(left)
                    && self.pure(right)
            }
            Expr::Not { operand } => self.pure(operand),
            _ => false,
        }
    }
}

/// A step that can make `k` smaller: a division, a remainder, a subtraction or a right shift somewhere in it.
fn can_shrink(e: &Expr) -> bool {
    match e {
        Expr::BinaryOp { op, left, right } => {
            matches!(
                op,
                BinaryOpKind::Divide
                    | BinaryOpKind::FloorDivide
                    | BinaryOpKind::Modulo
                    | BinaryOpKind::Subtract
                    | BinaryOpKind::Shr
            ) || can_shrink(left)
                || can_shrink(right)
        }
        _ => false,
    }
}
