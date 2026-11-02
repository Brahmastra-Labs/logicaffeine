//! A nest of adjacent compare-exchanges that sorts a window of a sequence, run as one in-place heap sort
//! (`Opt::SortNest`).
//!
//! Bubble sort is a loop of passes, each a counted loop exchanging two neighbours of a sequence when they are out of
//! order:
//!
//! ```text
//! Let i be 0.
//! While i is less than n - 1:
//!     Let j be 1.
//!     While j is at most n - 1 - i:
//!         Let a be item j of arr.
//!         Let b be item (j + 1) of arr.
//!         If a is greater than b:
//!             Set item j of arr to b.
//!             Set item (j + 1) of arr to a.
//!         Set j to j + 1.
//!     Set i to i + 1.
//! ```
//!
//! It leaves items `1` to `n` of `arr` in ascending order after `n(n-1)/2` comparisons. A heap sort leaves the same
//! window in the same order in `O(n log n)`, in place, and a sequence of Ints has one arrangement in ascending order,
//! so the two leave the same sequence. The rewrite holds when:
//! - the outer loop counts `i` from `S` by one for `K` passes (`While i < E` or `<= E`, or `Repeat for i from S to
//!   E`), and its body is the inner loop alone;
//! - the inner loop counts a `j` of its own from `L` by one through `T(i)`, `L` independent of `i` and `T(i)` either
//!   independent of `i` or one less on each pass; every bound is affine over Ints the nest does not write;
//! - the inner body exchanges `item (j + c)` and `item (j + c + 1)` of one sequence of Ints, through locals of its
//!   own, exactly when the lower item is greater than the upper (ascending; less for descending), and does nothing
//!   else. The body is read by evaluating it on the two items symbolically, so any spelling of the exchange qualifies.
//!
//! Pass `p` runs `j` over `[L, T(S) - p]`, or over all of `[L, T(S)]`, so after `p + 1` passes the last `p + 1` items
//! of the window `[L + c, T(S) + c + 1]` are its largest, in order; `m - 1` passes sort its `m` items, and a pass after
//! that exchanges nothing. One hoisted check, built by [`super::version::total_guard`] so that it cannot fail on any
//! backend, decides it: `K >= m - 1`, the window's first index at least 1 and its last at most the sequence's
//! length. Otherwise the nest runs as written and fails where it fails. A `While` counter leaves the sorted branch
//! where the loop would have left it.

use crate::analysis::types::TypeEnv;
use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Expr, Stmt};
use crate::intern::{Interner, Symbol};
use crate::loop_shape::{extract_counted_repeat, extract_counted_while, CountedLoop};
use crate::optimization::Opt;
use logicaffeine_kernel::lia::{LinearExpr, Rational};
use std::collections::HashMap;

use super::affine::{len_owner, lin_of_with_lengths, var};
use super::ast_build::Build;
use super::facts::{self, Fact};
use super::kinds::Kinds;
use super::writes::WriteSet;

/// Rewrite every sorting nest in `stmts`; true when one was rewritten.
pub(crate) fn sort_nest_stmts<'a>(
    stmts: Vec<Stmt<'a>>,
    expr_arena: &'a Arena<Expr<'a>>,
    stmt_arena: &'a Arena<Stmt<'a>>,
    interner: &mut Interner,
) -> (Vec<Stmt<'a>>, bool) {
    let env = TypeEnv::infer_program(&stmts, interner, &crate::analysis::TypeRegistry::new());
    let kinds = super::kinds::of_program(&stmts, &env, interner);
    let mut cx = Sorter {
        b: Build {
            ea: expr_arena,
            sa: stmt_arena,
        },
        kinds: &kinds,
        env: &env,
        fired: false,
        nests: 0,
    };
    let out =
        super::ast_build::rewrite_blocks(&stmts, stmt_arena, Opt::SortNest, &mut |block, at| {
            let pair: Vec<&Stmt<'a>> = block[at..block.len().min(at + 2)].iter().collect();
            if let Some((outer, span)) = extract_counted_while(&pair, 0) {
                if let Some(sorted) = cx.rewrite(Some(&block[at]), &block[at + 1], &outer, interner)
                {
                    return Some((sorted, span));
                }
            }
            let outer = extract_counted_repeat(&block[at])?;
            cx.rewrite(None, &block[at], &outer, interner)
                .map(|sorted| (sorted, 1))
        });
    (out, cx.fired)
}

/// The exchange an inner body performs: on `arr`, between the items at `j + lower` and `j + lower + 1`, when the
/// lower is greater than the upper (`ascending`) or less.
struct Exchange {
    arr: Symbol,
    lower: i64,
    ascending: bool,
}

struct Sorter<'a, 'k> {
    b: Build<'a>,
    kinds: &'k Kinds,
    env: &'k TypeEnv,
    fired: bool,
    nests: usize,
}

impl<'a, 'k> Sorter<'a, 'k> {
    /// The nest headed by `lp` (after `init` for a `While`) as `If P: <heap sort> Otherwise: <the nest>`, or `None`
    /// when it is not a sorting nest.
    fn rewrite(
        &mut self,
        init: Option<&Stmt<'a>>,
        lp: &Stmt<'a>,
        outer: &CountedLoop<'a>,
        it: &mut Interner,
    ) -> Option<Vec<Stmt<'a>>> {
        let inner = inner_loop(outer.body_without_increment)?;
        let (i, j) = (outer.var, inner.var);
        let exchange = exchange_of(inner.body_without_increment, j)?;
        let arr = exchange.arr;
        if i == j || i == arr || j == arr || !self.kinds.int_seq(arr) {
            return None;
        }
        let ints = |e: &Expr| self.kinds.int_valued(e, self.env);
        if init.is_some() && !self.kinds.int(i) {
            return None;
        }
        if ![outer.start, outer.limit, inner.start, inner.limit]
            .iter()
            .all(|e| ints(e))
        {
            return None;
        }

        // Every bound is affine over names the nest leaves alone; `arr` keeps its length, since the nest only
        // exchanges its items.
        let writes = WriteSet::of_stmt(lp);
        let stable = |s: Symbol| s == arr || !writes.may_write(s);
        let start = lin_of_with_lengths(outer.start, &stable)?;
        let limit = lin_of_with_lengths(outer.limit, &stable)?;
        let first = lin_of_with_lengths(inner.start, &stable)?;
        let last = lin_of_with_lengths(inner.limit, &stable)?;
        let fixed = |e: &LinearExpr, allow_i: bool| {
            e.coefficients.keys().all(|&v| match len_owner(v) {
                Some(c) => stable(c),
                None => {
                    let s = Symbol::from_index(v as usize);
                    (allow_i && s == i) || (s != i && s != j && !writes.may_write(s))
                }
            })
        };
        if !fixed(&start, false)
            || !fixed(&limit, false)
            || !fixed(&first, false)
            || !fixed(&last, true)
        {
            return None;
        }
        let slope = last
            .coefficients
            .get(&var_of(i))
            .cloned()
            .unwrap_or_else(|| Rational::from_i64(0));
        let falling = if slope == Rational::from_i64(0) {
            false
        } else if slope == Rational::from_i64(-1) {
            true
        } else {
            return None;
        };

        // `K` passes; the first pass's last `j`, `T(S)`; the window `[lo, hi]` it sorts.
        let one = LinearExpr::constant(Rational::from_i64(1));
        let mut passes = limit.sub(&start);
        if outer.inclusive {
            passes = passes.add(&one);
        }
        let mut top = last.clone();
        if falling {
            top = top.add(&var(i)).sub(&start);
        }
        if !inner.inclusive {
            top = top.sub(&one);
        }
        let shift = LinearExpr::constant(Rational::from_i64(exchange.lower));
        let lo = first.add(&shift);
        let hi = top.add(&shift).add(&one);
        let enough = passes.sub(&hi.sub(&lo));

        let ea = self.b.ea;
        let mut checks: Vec<&'a Expr<'a>> = Vec::new();
        for (e, at_least) in [(&enough, 0), (&lo, 1)] {
            match e.is_constant().then(|| e.constant.to_i64()).flatten() {
                Some(k) if k >= at_least => {}
                Some(_) => return None,
                None => checks.push(self.b.bin(
                    BinaryOpKind::GtEq,
                    self.b.linear(e)?,
                    self.b.num(at_least),
                )),
            }
        }
        let length = ea.alloc(Expr::Length {
            collection: self.b.id(arr),
        });
        checks.push(self.b.bin(BinaryOpKind::LtEq, self.b.linear(&hi)?, length));
        let guard = super::version::total_guard(ea, &checks)?;

        let mut fast = self.heap_sort(arr, &lo, &hi, exchange.ascending, it)?;
        if init.is_some() {
            // The loop leaves its counter at its limit (one past it for `<=`), or where it started when it ran no
            // pass.
            let (op, exit) = if outer.inclusive {
                (
                    BinaryOpKind::LtEq,
                    self.b.bin(BinaryOpKind::Add, outer.limit, self.b.num(1)),
                )
            } else {
                (BinaryOpKind::Lt, outer.limit)
            };
            fast.push(Stmt::If {
                cond: self.b.bin(op, self.b.id(i), outer.limit),
                then_block: self.b.block(vec![Stmt::Set {
                    target: i,
                    value: exit,
                }]),
                else_block: None,
            });
        }
        let branches = Stmt::If {
            cond: guard,
            then_block: self.b.block(fast),
            else_block: Some(self.b.block(vec![lp.clone()])),
        };
        self.fired = true;
        Some(match init {
            Some(init) => vec![init.clone(), branches],
            None => vec![branches],
        })
    }

    /// An in-place heap sort of items `lo` to `hi` of `arr`, ascending or descending. Every index it reads or
    /// writes lies in `[lo, hi]`, which the guard puts inside `arr`.
    fn heap_sort(
        &mut self,
        arr: Symbol,
        lo: &LinearExpr,
        hi: &LinearExpr,
        ascending: bool,
        it: &mut Interner,
    ) -> Option<Vec<Stmt<'a>>> {
        let k = self.nests;
        self.nests += 1;
        let mut fresh = |name: &str| it.intern(&format!("__sortnest{k}_{name}"));
        let (base, size, start, end, t) = (
            fresh("base"),
            fresh("size"),
            fresh("start"),
            fresh("end"),
            fresh("t"),
        );
        let build = (fresh("r1"), fresh("c1"), fresh("t1"));
        let settle = (fresh("r2"), fresh("c2"), fresh("t2"));
        let one = LinearExpr::constant(Rational::from_i64(1));
        let base_value = self.b.linear(&lo.sub(&one))?;
        let size_value = self.b.linear(&hi.sub(lo).add(&one))?;

        let mut out = vec![
            self.b.let_(base, base_value, false),
            self.b.let_(size, size_value, false),
            self.b.let_(
                start,
                self.b
                    .bin(BinaryOpKind::Divide, self.b.id(size), self.b.num(2)),
                true,
            ),
        ];
        let mut heapify = self.sift(arr, base, self.b.id(start), size, build, ascending);
        heapify.push(self.b.step(start, BinaryOpKind::Subtract));
        out.push(Stmt::While {
            cond: self
                .b
                .bin(BinaryOpKind::GtEq, self.b.id(start), self.b.num(1)),
            body: self.b.block(heapify),
            decreasing: None,
        });
        out.push(self.b.let_(end, self.b.id(size), true));
        let first = self.at(base, self.b.num(1));
        let last = self.at(base, self.b.id(end));
        let mut settle_body = vec![
            self.b.let_(t, self.b.item(arr, first), false),
            self.b
                .store(arr, self.at(base, self.b.num(1)), self.b.item(arr, last)),
            self.b
                .store(arr, self.at(base, self.b.id(end)), self.b.id(t)),
            self.b.step(end, BinaryOpKind::Subtract),
        ];
        settle_body.extend(self.sift(arr, base, self.b.num(1), end, settle, ascending));
        out.push(Stmt::While {
            cond: self
                .b
                .bin(BinaryOpKind::GtEq, self.b.id(end), self.b.num(2)),
            body: self.b.block(settle_body),
            decreasing: None,
        });
        Some(out)
    }

    /// Sift the item at heap position `root` down a heap of `size` items stored from `base + 1`: a max-heap for an
    /// ascending sort, a min-heap for a descending one.
    fn sift(
        &self,
        arr: Symbol,
        base: Symbol,
        root: &'a Expr<'a>,
        size: Symbol,
        (r, c, t): (Symbol, Symbol, Symbol),
        ascending: bool,
    ) -> Vec<Stmt<'a>> {
        let before = if ascending {
            BinaryOpKind::Lt
        } else {
            BinaryOpKind::Gt
        };
        let child = self
            .b
            .bin(BinaryOpKind::Multiply, self.b.num(2), self.b.id(r));
        let right = self.b.bin(BinaryOpKind::Add, self.b.id(c), self.b.num(1));
        let pick_right = Stmt::If {
            cond: self.b.bin(
                before,
                self.b.item(arr, self.at(base, self.b.id(c))),
                self.b.item(arr, self.at(base, right)),
            ),
            then_block: self.b.block(vec![self.b.step(c, BinaryOpKind::Add)]),
            else_block: None,
        };
        let has_right = Stmt::If {
            cond: self.b.bin(BinaryOpKind::Lt, self.b.id(c), self.b.id(size)),
            then_block: self.b.block(vec![pick_right]),
            else_block: None,
        };
        let exchange = vec![
            self.b
                .let_(t, self.b.item(arr, self.at(base, self.b.id(r))), false),
            self.b.store(
                arr,
                self.at(base, self.b.id(r)),
                self.b.item(arr, self.at(base, self.b.id(c))),
            ),
            self.b.store(arr, self.at(base, self.b.id(c)), self.b.id(t)),
            Stmt::Set {
                target: r,
                value: self.b.id(c),
            },
        ];
        let settle = Stmt::If {
            cond: self.b.bin(
                before,
                self.b.item(arr, self.at(base, self.b.id(r))),
                self.b.item(arr, self.at(base, self.b.id(c))),
            ),
            then_block: self.b.block(exchange),
            else_block: Some(self.b.block(vec![Stmt::Break])),
        };
        vec![
            self.b.let_(r, root, true),
            Stmt::While {
                cond: self.b.bin(
                    BinaryOpKind::LtEq,
                    self.b
                        .bin(BinaryOpKind::Multiply, self.b.num(2), self.b.id(r)),
                    self.b.id(size),
                ),
                body: self
                    .b
                    .block(vec![self.b.let_(c, child, true), has_right, settle]),
                decreasing: None,
            },
        ]
    }

    /// `base + offset`, an index the heap sort reads or writes: inside the window the guard proved.
    fn at(&self, base: Symbol, offset: &'a Expr<'a>) -> &'a Expr<'a> {
        let index = self.b.bin(BinaryOpKind::Add, self.b.id(base), offset);
        facts::mark_expr(Fact::InBounds, index);
        index
    }

}

/// The prover variable of a scalar name.
fn var_of(s: Symbol) -> i64 {
    super::affine::var_index(s)
}

/// The outer loop's body as one counted loop of its own counter: `Let j be L` and its `While`, or a `Repeat for j
/// from L to U`.
fn inner_loop<'a>(body: &'a [Stmt<'a>]) -> Option<CountedLoop<'a>> {
    match body {
        [init @ Stmt::Let { .. }, lp] => {
            let refs = [init, lp];
            let (inner, span) = extract_counted_while(&refs, 0)?;
            (span == 2).then_some(inner)
        }
        [lp] => extract_counted_repeat(lp),
        _ => None,
    }
}

/// The exchange `body` performs, read by running it on the two items it touches: leading `Let`s that read items
/// or other such locals, then one `If` comparing the two items whose branch leaves them exchanged and writes
/// nothing else. `None` for anything else.
fn exchange_of(body: &[Stmt], j: Symbol) -> Option<Exchange> {
    let (
        head,
        [Stmt::If {
            cond,
            then_block,
            else_block: None,
        }],
    ) = body.split_at(body.len().checked_sub(1)?)
    else {
        return None;
    };
    let mut run = Run {
        j,
        arr: None,
        slots: Vec::new(),
        locals: HashMap::new(),
        items: HashMap::new(),
    };
    for s in head {
        run.bind(s)?;
    }
    let Expr::BinaryOp { op, left, right } = cond else {
        return None;
    };
    let (a, b) = (run.value(left)?, run.value(right)?);
    let mut branch = run.clone();
    for s in *then_block {
        match s {
            Stmt::Let { .. } => branch.bind(s)?,
            _ => branch.write(s)?,
        }
    }
    let lower = *branch.slots.iter().min()?;
    let upper = lower + 1;
    if branch.slots.len() != 2 || !branch.slots.contains(&upper) {
        return None;
    }
    // The branch must leave the lower slot holding the upper item and the upper slot the lower one.
    if branch.item(lower) != upper || branch.item(upper) != lower {
        return None;
    }
    let op = match (a, b) {
        (x, y) if x == lower && y == upper => *op,
        (x, y) if x == upper && y == lower => flip(*op)?,
        _ => return None,
    };
    let ascending = match op {
        BinaryOpKind::Gt | BinaryOpKind::GtEq => true,
        BinaryOpKind::Lt | BinaryOpKind::LtEq => false,
        _ => return None,
    };
    Some(Exchange {
        arr: branch.arr?,
        lower,
        ascending,
    })
}

/// `a op b` as `b op' a`.
fn flip(op: BinaryOpKind) -> Option<BinaryOpKind> {
    Some(match op {
        BinaryOpKind::Lt => BinaryOpKind::Gt,
        BinaryOpKind::Gt => BinaryOpKind::Lt,
        BinaryOpKind::LtEq => BinaryOpKind::GtEq,
        BinaryOpKind::GtEq => BinaryOpKind::LtEq,
        _ => return None,
    })
}

/// A symbolic run of an exchange body: each slot `j + d` of the one sequence holds the item first found at some slot,
/// and each local the item it read.
#[derive(Clone)]
struct Run {
    j: Symbol,
    arr: Option<Symbol>,
    slots: Vec<i64>,
    locals: HashMap<Symbol, i64>,
    items: HashMap<i64, i64>,
}

impl Run {
    /// The slot `index` names, `j + d`, recorded.
    fn slot(&mut self, index: &Expr) -> Option<i64> {
        let form = super::affine::lin_of(index)?.sub(&var(self.j));
        let d = form
            .is_constant()
            .then(|| form.constant.to_i64())
            .flatten()?;
        if !self.slots.contains(&d) {
            self.slots.push(d);
        }
        Some(d)
    }

    /// The item slot `d` holds now.
    fn item(&self, d: i64) -> i64 {
        self.items.get(&d).copied().unwrap_or(d)
    }

    /// The item `e` evaluates to: an item of the sequence or a local holding one.
    fn value(&mut self, e: &Expr) -> Option<i64> {
        match e {
            Expr::Index {
                collection: Expr::Identifier(c),
                index,
            } => {
                if *self.arr.get_or_insert(*c) != *c {
                    return None;
                }
                let d = self.slot(index)?;
                Some(self.item(d))
            }
            Expr::Identifier(x) => self.locals.get(x).copied(),
            _ => None,
        }
    }

    /// A `Let` of a new local holding an item.
    fn bind(&mut self, s: &Stmt) -> Option<()> {
        let Stmt::Let { var, value, .. } = s else {
            return None;
        };
        if *var == self.j || self.arr == Some(*var) || self.locals.contains_key(var) {
            return None;
        }
        let v = self.value(value)?;
        self.locals.insert(*var, v);
        Some(())
    }

    /// A store of an item into a slot of the sequence.
    fn write(&mut self, s: &Stmt) -> Option<()> {
        let Stmt::SetIndex {
            collection: Expr::Identifier(c),
            index,
            value,
        } = s
        else {
            return None;
        };
        if *self.arr.get_or_insert(*c) != *c {
            return None;
        }
        let v = self.value(value)?;
        let d = self.slot(index)?;
        self.items.insert(d, v);
        Some(())
    }
}
