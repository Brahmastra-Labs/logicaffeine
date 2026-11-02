//! A search, for each number of a counted range, for a divisor no greater than its square root, run as one sieve of
//! the range (`Opt::DivisorSieve`).
//!
//! Trial division asks of every `i` whether some `d` from `D0` while `d * d <= i` divides it:
//!
//! ```text
//! While i is at most n:
//!     Let isPrime be 1.
//!     Let d be 2.
//!     While d * d is at most i:
//!         If i % d equals 0:
//!             Set isPrime to 0.
//!             Break.
//!         Set d to d + 1.
//!     If isPrime equals 1:
//!         Set count to count + 1.
//!     Set i to i + 1.
//! ```
//!
//! The search finds a divisor exactly when `i = k * d` for some `d >= D0` and `k >= d`, since `d` divides `i` and
//! `d * d <= i` together say so. One pass before the loop marks every such number up to the range's end: for each
//! `d` from `D0` while `d * d <= n`, every `m` from `d * d` by `d`. A `d` already marked needs no pass of its own: some
//! `e >= D0` with `e * e <= d` divides it, so `e` marked every multiple of `d` the pass would mark. The search then
//! reads one mark: `If i >= D0 * D0 and item (i + 1) of the marks: <what the search did on a find>`, with no loop.
//! The marks are a `Seq of Bool`, one bit per number once `Opt::BitPack` holds them.
//!
//! The rewrite holds when:
//! - the loop counts `i` by one over Ints (`While i <= E` or `< E` after its start, or `Repeat for i from A to E`);
//! - its body holds, at its top level, `Let d be D0` (an integer literal at least 2) directly followed by
//!   `While d * d <= i` (or `d <= i / d`) whose body is `If i % d equals 0` (either way round) holding `Set`s and a
//!   `Break`, then `Set d to d + 1`; each `Set` writes neither `i` nor `d`, and gives a literal, `i`, or a name the
//!   loop never writes;
//! - nothing after the search in the body names `d`.
//!
//! The search runs no step of its own that can fail: `d >= 2` divides, and `d * d` stays below `i` plus `2d`. So
//! running the finds out of a table changes no output and no failure. One hoisted check, built by
//! [`super::version::total_guard`], bounds the marks' size: the range's end at most 2^28. Otherwise the loop runs as
//! written.

use crate::analysis::types::TypeEnv;
use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::loop_shape::{extract_counted_repeat, extract_counted_while, CountedLoop};
use crate::optimization::Opt;

use super::ast_build::Build;
use super::kinds::Kinds;
use super::writes::WriteSet;

/// The largest range end the marks cover: 2^28 numbers, 32 MiB as bits.
const MARKS_CAP: i64 = 1 << 28;

/// Rewrite every divisor search in `stmts`; true when one was rewritten.
pub(crate) fn divisor_sieve_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    let env = TypeEnv::infer_program(&stmts, interner, &crate::analysis::TypeRegistry::new());
    let kinds = super::kinds::of_program(&stmts, &env, interner);
    let mut cx = Siever {
        b: Build {
            ea: expr_arena,
            sa: stmt_arena,
        },
        kinds: &kinds,
        env: &env,
        fired: false,
        sieves: 0,
    };
    let out = super::ast_build::rewrite_blocks(
        &stmts,
        stmt_arena,
        Opt::DivisorSieve,
        &mut |block, at| {
            let pair: Vec<&Stmt<'a>> = block[at..block.len().min(at + 2)].iter().collect();
            if let Some((outer, span)) = extract_counted_while(&pair, 0) {
                if let Some(sieved) = cx.rewrite(Some(&block[at]), &block[at + 1], &outer, interner)
                {
                    return Some((sieved, span));
                }
            }
            let outer = extract_counted_repeat(&block[at])?;
            cx.rewrite(None, &block[at], &outer, interner)
                .map(|sieved| (sieved, 1))
        },
    );
    (out, cx.fired)
}

/// A divisor search at `body[at]` (its `Let d`) and `body[at + 1]` (its loop): the smallest divisor it tries and the
/// statements it runs on a find.
struct Search<'a> {
    at: usize,
    first: i64,
    on_find: Vec<Stmt<'a>>,
}

struct Siever<'a, 'k> {
    b: Build<'a>,
    kinds: &'k Kinds,
    env: &'k TypeEnv,
    fired: bool,
    sieves: usize,
}

impl<'a, 'k> Siever<'a, 'k> {
    /// The loop `lp` (after `init` for a `While`) as `If P: <sieve; the loop reading it> Otherwise: <the loop>`, or
    /// `None` when its body holds no divisor search.
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
        let search =
            (0..body.len().saturating_sub(1)).find_map(|at| search_at(body, at, i, &writes))?;

        // The marks cover every number up to the range's last: `E`, or `E - 1` below an exclusive end.
        let b = self.b;
        let last = |b: Build<'a>| -> Option<&'a Expr<'a>> {
            let limit = b.copy(outer.limit)?;
            Some(if outer.inclusive {
                limit
            } else {
                b.bin(BinaryOpKind::Subtract, limit, b.num(1))
            })
        };
        let guard = super::version::total_guard(
            b.ea,
            &[b.bin(BinaryOpKind::LtEq, last(b)?, b.num(MARKS_CAP))],
        )?;

        let k = self.sieves;
        self.sieves += 1;
        let mut fresh = |name: &str| it.intern(&format!("__divsieve{k}_{name}"));
        let (n, marks, fill, d, m) = (
            fresh("n"),
            fresh("marks"),
            fresh("fill"),
            fresh("d"),
            fresh("m"),
        );
        let (seq, boolean) = (it.intern("Seq"), it.intern("Bool"));

        let one = || b.num(1);
        let mark_of = |x: Symbol| b.item(marks, b.bin(BinaryOpKind::Add, b.id(x), one()));
        let mut fast = vec![
            b.let_(n, last(b)?, false),
            b.let_(marks, b.new_seq(seq, TypeExpr::Primitive(boolean)), true),
            b.let_(fill, b.num(0), true),
            b.while_(
                b.bin(BinaryOpKind::LtEq, b.id(fill), b.id(n)),
                vec![
                    b.push(b.boolean(false), marks),
                    b.step(fill, BinaryOpKind::Add),
                ],
            ),
            b.let_(d, b.num(search.first), true),
            b.while_(
                b.bin(
                    BinaryOpKind::LtEq,
                    b.bin(BinaryOpKind::Multiply, b.id(d), b.id(d)),
                    b.id(n),
                ),
                vec![
                    b.if_(
                        b.bin(BinaryOpKind::Eq, mark_of(d), b.boolean(false)),
                        vec![
                            b.let_(m, b.bin(BinaryOpKind::Multiply, b.id(d), b.id(d)), true),
                            b.while_(
                                b.bin(BinaryOpKind::LtEq, b.id(m), b.id(n)),
                                vec![
                                    b.store(
                                        marks,
                                        b.bin(BinaryOpKind::Add, b.id(m), one()),
                                        b.boolean(true),
                                    ),
                                    b.set(m, b.bin(BinaryOpKind::Add, b.id(m), b.id(d))),
                                ],
                            ),
                        ],
                        None,
                    ),
                    b.step(d, BinaryOpKind::Add),
                ],
            ),
        ];

        // The loop, its search replaced by one read of the marks.
        let read = b.if_(
            b.bin(
                BinaryOpKind::GtEq,
                b.id(i),
                b.num(search.first * search.first),
            ),
            vec![b.if_(
                b.bin(BinaryOpKind::Eq, mark_of(i), b.boolean(true)),
                search.on_find.clone(),
                None,
            )],
            None,
        );
        let sieved_body = |whole: &[Stmt<'a>]| -> Vec<Stmt<'a>> {
            let mut out: Vec<Stmt<'a>> = whole[..search.at].to_vec();
            out.push(read.clone());
            out.extend(whole[search.at + 2..].iter().cloned());
            out
        };
        let fast_loop = match lp {
            Stmt::While {
                cond,
                body: whole,
                decreasing,
            } => Stmt::While {
                cond: *cond,
                body: b.block(sieved_body(whole)),
                decreasing: *decreasing,
            },
            Stmt::Repeat {
                pattern,
                iterable,
                body: whole,
            } => Stmt::Repeat {
                pattern: pattern.clone(),
                iterable: *iterable,
                body: b.block(sieved_body(whole)),
            },
            _ => return None,
        };
        fast.push(fast_loop);
        let branches = b.if_(guard, fast, Some(vec![lp.clone()]));
        self.fired = true;
        Some(match init {
            Some(init) => vec![init.clone(), branches],
            None => vec![branches],
        })
    }
}

/// The divisor search starting at `body[at]` for the loop's counter `i`, by the shape the module documents; `writes`
/// is what the loop writes.
fn search_at<'a>(
    body: &'a [Stmt<'a>],
    at: usize,
    i: Symbol,
    writes: &WriteSet,
) -> Option<Search<'a>> {
    let Stmt::Let {
        var: d,
        value: Expr::Literal(Literal::Number(first)),
        ..
    } = &body[at]
    else {
        return None;
    };
    let (d, first) = (*d, *first);
    if first < 2 || d == i || first > 1 << 15 {
        return None;
    }
    let Stmt::While {
        cond, body: inner, ..
    } = &body[at + 1]
    else {
        return None;
    };
    if !bounded_by_root(cond, d, i) {
        return None;
    }
    let [Stmt::If {
        cond: test,
        then_block,
        else_block: None,
    }, step] = &inner[..]
    else {
        return None;
    };
    if !divides(test, d, i) || !steps_by_one(step, d) {
        return None;
    }
    let (Some(Stmt::Break), sets) = (
        then_block.last(),
        &then_block[..then_block.len().saturating_sub(1)],
    ) else {
        return None;
    };
    let fixed = |e: &Expr| match e {
        Expr::Literal(Literal::Number(_) | Literal::Boolean(_)) => true,
        Expr::Identifier(s) => *s == i || (*s != d && !writes.may_write(*s)),
        _ => false,
    };
    if !sets
        .iter()
        .all(|s| matches!(s, Stmt::Set { target, value } if *target != i && *target != d && fixed(value)))
    {
        return None;
    }
    let after = &body[at + 2..];
    if after.iter().any(|s| super::mentions::stmt_touches(s, d)) {
        return None;
    }
    Some(Search {
        at,
        first,
        on_find: sets.to_vec(),
    })
}

/// `d * d <= i` or `d <= i / d`.
fn bounded_by_root(cond: &Expr, d: Symbol, i: Symbol) -> bool {
    let is = |e: &Expr, s: Symbol| matches!(e, Expr::Identifier(x) if *x == s);
    match cond {
        Expr::BinaryOp {
            op: BinaryOpKind::LtEq,
            left,
            right,
        } => match (left, right) {
            (
                Expr::BinaryOp {
                    op: BinaryOpKind::Multiply,
                    left: a,
                    right: c,
                },
                r,
            ) => is(a, d) && is(c, d) && is(r, i),
            (
                l,
                Expr::BinaryOp {
                    op: BinaryOpKind::Divide,
                    left: a,
                    right: c,
                },
            ) => is(l, d) && is(a, i) && is(c, d),
            _ => false,
        },
        _ => false,
    }
}

/// `i % d equals 0`, either way round.
fn divides(test: &Expr, d: Symbol, i: Symbol) -> bool {
    let rem = |e: &Expr| {
        matches!(e, Expr::BinaryOp { op: BinaryOpKind::Modulo, left: Expr::Identifier(a), right: Expr::Identifier(c) }
            if *a == i && *c == d)
    };
    let zero = |e: &Expr| matches!(e, Expr::Literal(Literal::Number(0)));
    matches!(test, Expr::BinaryOp { op: BinaryOpKind::Eq, left, right } if (rem(left) && zero(right)) || (zero(left) && rem(right)))
}

/// `Set d to d + 1`.
fn steps_by_one(s: &Stmt, d: Symbol) -> bool {
    matches!(s, Stmt::Set { target, value: Expr::BinaryOp { op: BinaryOpKind::Add, left: Expr::Identifier(a), right: Expr::Literal(Literal::Number(1)) } }
        if *target == d && *a == d)
}
