//! Which reorderings of a loop nest's iterations no run can observe — the one answer every backend asks.
//!
//! A nest is a band of counted loops (`Let c be lo` or `Set c to lo`, then `While c < hi` / `While c <= hi`
//! whose body ends `Set c to c + 1` and writes `c` nowhere else), perfectly nested down to one innermost body,
//! with every bound invariant over the whole band. Running the band's iterations in another order is invisible
//! exactly when:
//!
//! 1. no two iterations whose order the reorder swaps touch one item of a sequence, one of them storing it;
//! 2. the body does nothing else a later iteration or the outside world could see — it binds its own locals and
//!    stores into sequences. A variable carried from one iteration to the next is a dependence, an Int sum
//!    included: its terms commute only under exact arithmetic, and until every tier's Int is exact a reordered
//!    partial sum can leave i64 where the written order does not;
//! 3. no access can fail, so the order of failures cannot show: every index names an item and no divisor is
//!    zero, on every iteration of the band.
//!
//! [`tile`] decides 1 and 2 from the body alone and states 3 as [`Precondition`]s a versioning guard checks once
//! before the nest. Every bound is an Int and every collection a sequence (the callers' kind oracles), and every
//! name in an index is a counter or a value the band never writes, so the corners below are the indices' true
//! extremes. The guard need only test the band's extreme points: an index affine in the counters takes
//! its least and greatest value at corners of the iteration box, so the two corner values in range put every
//! iteration's index in range.
//!
//! The dependence test is the kernel's own decision procedure ([`super::affine`], Fourier–Motzkin). For each
//! pair of accesses to one sequence, at least one a store, it asks whether some iteration `I` and a later
//! iteration `I'` (in the program's order) can touch the same item while `I'` precedes `I` in some band counter
//! — the one pattern tiling swaps. Every such system unsatisfiable is the Wolf–Lam condition: the band is fully
//! permutable. A row-major index `R * S + C` with `C` a band counter running over `[0, S)` names one item per
//! `(R, C)`, so it is tested as two dimensions and a matrix product is proven, not assumed. Fourier–Motzkin
//! decides over the rationals, so a rational solution with no integer one reads as a dependence: the test can
//! refuse a legal nest, never accept an illegal one.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};
use crate::intern::Symbol;
use logicaffeine_language::ast_depth::expr_children;

use super::affine::{consistent, konst, le, lin_of, lt, var, Constraint, LinExpr};
use super::same_expr;
use super::writes::WriteSet;

/// One counted loop of a band: `counter` runs from `lo` while `counter < hi` (or `<= hi` when `inclusive`).
pub(crate) struct CountedLoop<'a> {
    pub counter: Symbol,
    pub lo: &'a Expr<'a>,
    pub hi: &'a Expr<'a>,
    pub inclusive: bool,
}

/// A perfect band of counted loops, outermost first, and the innermost body without its increment.
pub(crate) struct Nest<'a> {
    pub loops: Vec<CountedLoop<'a>>,
    pub body: &'a [Stmt<'a>],
}

/// Whether a reorder of a nest's iterations can be observed.
pub(crate) enum Legality<'a> {
    /// No run can tell.
    Always,
    /// No run can tell once every precondition holds; one hoisted check decides it.
    When(Vec<Precondition<'a>>),
    /// Some run can tell, and why.
    Never(&'static str),
}

/// A fact about values fixed before the nest runs that makes its reorder invisible.
pub(crate) enum Precondition<'a> {
    /// Every access `item index of seq` in the band names an item: the least index is at least 1 and the
    /// greatest is at most the sequence's length. The bounds are expressions over values fixed before the nest.
    InRange {
        seq: Symbol,
        least: Bound<'a>,
        greatest: Bound<'a>,
    },
    /// A divisor fixed before the nest is not zero.
    NonZero(&'a Expr<'a>),
}

/// An index bound as the versioning guard evaluates it: `base + scale × factor + Σ ±invariant`, each part an
/// expression over values fixed before the nest (a row-major index's `R × S` part is `scale × factor`; an
/// invariant term is a part of the index no counter appears in, kept as written).
pub(crate) struct Bound<'a> {
    pub base: LinExpr,
    pub scale: Option<(LinExpr, &'a Expr<'a>)>,
    pub invariant: Vec<(bool, &'a Expr<'a>)>,
}

/// The band starting at `stmts[at]` (its first counter's initialization; the loop is `stmts[at + 1]`), or `None`.
pub(crate) fn nest_at<'a>(stmts: &'a [Stmt<'a>], at: usize) -> Option<Nest<'a>> {
    let mut loops = Vec::new();
    let (first, mut body) = counted_loop(stmts.get(at)?, stmts.get(at + 1)?)?;
    loops.push(first);
    while let [init, inner] = body {
        let Some((next, inner_body)) = counted_loop(init, inner) else {
            break;
        };
        loops.push(next);
        body = inner_body;
    }
    let counters: Vec<Symbol> = loops.iter().map(|l| l.counter).collect();
    let writes = WriteSet::of_block(body);
    // Every bound is affine over values the band never writes, so it is the same on every iteration and its
    // corners are computable; the body writes no counter, so each counter steps only by its own increment.
    let bounds_fixed = loops.iter().all(|l| {
        [l.lo, l.hi].iter().all(|e| {
            lin_of(e).is_some()
                && identifiers(e)
                    .iter()
                    .all(|s| !counters.contains(s) && !writes.may_write(*s))
        })
    });
    let counters_untouched = counters.iter().all(|c| !writes.may_write(*c));
    (bounds_fixed && counters_untouched).then_some(Nest { loops, body })
}

/// `init` then `lp` as one counted loop, and the loop's body without its trailing increment.
fn counted_loop<'a>(
    init: &'a Stmt<'a>,
    lp: &'a Stmt<'a>,
) -> Option<(CountedLoop<'a>, &'a [Stmt<'a>])> {
    let (counter, lo) = match init {
        Stmt::Let { var, value, .. } => (*var, *value),
        Stmt::Set { target, value } => (*target, *value),
        _ => return None,
    };
    let Stmt::While {
        cond:
            Expr::BinaryOp {
                op,
                left: Expr::Identifier(c),
                right: hi,
            },
        body,
        ..
    } = lp
    else {
        return None;
    };
    let inclusive = match op {
        BinaryOpKind::Lt => false,
        BinaryOpKind::LtEq => true,
        _ => return None,
    };
    let (Some(Stmt::Set { target, value }), rest) =
        (body.last(), &body[..body.len().saturating_sub(1)])
    else {
        return None;
    };
    let steps_by_one = matches!(value, Expr::BinaryOp { op: BinaryOpKind::Add, left: Expr::Identifier(a), right: Expr::Literal(Literal::Number(1)) } if *a == counter);
    if *c != counter
        || *target != counter
        || !steps_by_one
        || WriteSet::of_block(rest).may_write(counter)
    {
        return None;
    }
    Some((
        CountedLoop {
            counter,
            lo,
            hi,
            inclusive,
        },
        rest,
    ))
}

/// Whether running `nest`'s band in tile order is invisible (see the module doc). `int_valued` says whether an
/// expression is an Int wherever it is evaluated, `seq` whether a name holds a sequence on every binding, and
/// `rows` whether a sequence of sequences has no two rows that are one object.
///
/// An access to an item of a row, `item k of (item E of c)` with `k` a literal, is placed by its row `E`: two such
/// accesses touch one item only when their rows' coordinates agree and their `k`s do. It needs `rows(c)`, and P
/// covers its row index only. All inner accesses must use the same literal item number, so whichever
/// iteration finds a row too short reports the same failure. Those accesses remain checked.
pub(crate) fn tile<'a>(
    nest: &Nest<'a>,
    int_valued: &dyn Fn(&Expr) -> bool,
    seq: &dyn Fn(Symbol) -> bool,
    rows: &dyn Fn(Symbol) -> bool,
) -> Legality<'a> {
    if !nest
        .loops
        .iter()
        .all(|l| int_valued(l.lo) && int_valued(l.hi))
    {
        return Legality::Never("a bound that is not an Int");
    }
    let mut body = Body::default();
    if let Err(why) = body.read(nest.body, &nest.loops) {
        return Legality::Never(why);
    }
    if !body.accesses.iter().all(|(s, _, _, _)| seq(*s))
        || !body.divisors.iter().all(|d| int_valued(d))
    {
        return Legality::Never("an access or a divisor of a kind this test does not place");
    }
    if body
        .accesses
        .iter()
        .any(|(s, _, _, inner)| inner.is_some() && !rows(*s))
    {
        return Legality::Never("rows of a sequence that may be one object");
    }
    // Inner row accesses stay checked. Two different item numbers can fail with different
    // messages, and tile order can expose the later failure first even with no data dependence.
    // A single possible row-index failure remains order-independent.
    let inner_indices: BTreeSet<i64> = body
        .accesses
        .iter()
        .filter_map(|(_, _, _, inner)| *inner)
        .collect();
    if inner_indices.len() > 1 {
        return Legality::Never("row accesses with different possible failures");
    }
    let writes = WriteSet::of_block(nest.body);
    let counters: BTreeSet<Symbol> = nest.loops.iter().map(|l| l.counter).collect();
    if !body.divisors.iter().all(|d| {
        identifiers(d)
            .iter()
            .all(|s| !counters.contains(s) && !writes.may_write(*s))
    }) {
        return Legality::Never("a divisor that changes inside the nest");
    }
    let dims = match body.dimensions(&nest.loops, &writes) {
        Ok(d) => d,
        Err(why) => return Legality::Never(why),
    };
    if let Some(why) = reorder_swaps_a_dependence(&dims, &nest.loops) {
        return Legality::Never(why);
    }
    let mut pre = Vec::new();
    for (seq, access) in dims
        .iter()
        .flat_map(|(seq, accesses)| accesses.iter().map(move |a| (*seq, a)))
    {
        pre.push(Precondition::InRange {
            seq,
            least: access.least(&nest.loops),
            greatest: access.greatest(&nest.loops),
        });
    }
    pre.extend(body.divisors.into_iter().map(Precondition::NonZero));
    if pre.is_empty() {
        Legality::Always
    } else {
        Legality::When(pre)
    }
}

/// The preconditions under which every access of `nest`'s body that `proven` does not already prove names an item
/// on every iteration, run in the order written, and whether any of those indices reads a name other than a
/// counter. Nothing is reordered, so every failure but an index's happens at the same iteration either way,
/// and P covers exactly the accesses. An access is placed affinely, row-major, or as a signed sum of terms no
/// counter appears in (kept as written, fixed over the band) plus an affine part in the counters. Refused: a
/// sequence the body resizes or rebinds, an index reading a value the body writes, a body whose effects are not
/// known, a closure.
pub(crate) fn bounds<'a>(
    nest: &Nest<'a>,
    int_valued: &dyn Fn(&Expr) -> bool,
    seq: &dyn Fn(Symbol) -> bool,
    proven: &dyn Fn(&Expr, &Expr) -> bool,
) -> Result<(Vec<Precondition<'a>>, bool), &'static str> {
    use logicaffeine_language::ast_depth::{for_each_node, Visited};
    if !nest
        .loops
        .iter()
        .all(|l| int_valued(l.lo) && int_valued(l.hi))
    {
        return Err("a bound that is not an Int");
    }
    let writes = WriteSet::of_block(nest.body);
    if writes.is_opaque() {
        return Err("a body whose effects are not known");
    }
    let counters: BTreeSet<Symbol> = nest.loops.iter().map(|l| l.counter).collect();
    let mut binds: BTreeMap<Symbol, usize> = BTreeMap::new();
    let mut accesses: Vec<(Symbol, &'a Expr<'a>, &'a Expr<'a>)> = Vec::new();
    let mut closure = false;
    for_each_node(nest.body, |node| match node {
        Visited::Stmt(Stmt::Let { var, .. }) | Visited::Stmt(Stmt::Set { target: var, .. }) => {
            *binds.entry(*var).or_default() += 1
        }
        Visited::Expr(Expr::Index {
            collection: c @ Expr::Identifier(s),
            index,
        }) => accesses.push((*s, c, index)),
        Visited::Stmt(Stmt::SetIndex {
            collection: c @ Expr::Identifier(s),
            index,
            ..
        }) => accesses.push((*s, c, index)),
        Visited::Expr(Expr::Closure { .. }) => closure = true,
        _ => {}
    });
    if closure {
        return Err("a closure in the body");
    }
    // A body-local index stands for its definition when the body binds it once, at its top level.
    let locals: BTreeMap<Symbol, &'a Expr<'a>> = nest
        .body
        .iter()
        .filter_map(|s| match s {
            Stmt::Let { var, value, .. } if binds.get(var) == Some(&1) => Some((*var, *value)),
            _ => None,
        })
        .collect();
    let mut pre = Vec::new();
    let mut symbolic = false;
    for (s, collection, index) in accesses {
        if proven(collection, index) {
            continue;
        }
        if !seq(s) {
            return Err("an access to a collection that is not a sequence");
        }
        if resized_or_rebound(nest.body, s) {
            return Err("a sequence the loop resizes or rebinds");
        }
        let index = match index {
            Expr::Identifier(v) => locals.get(v).copied().unwrap_or(index),
            _ => index,
        };
        let names = identifiers(index);
        if !names
            .iter()
            .all(|n| counters.contains(n) || !writes.may_write(*n))
        {
            return Err("an index that reads a value the band changes");
        }
        symbolic |= names.iter().any(|n| !counters.contains(n));
        let (least, greatest) = match Access::of(index, &nest.loops, false) {
            Some(a) => (a.least(&nest.loops), a.greatest(&nest.loops)),
            None => offset_corners(index, &nest.loops, &counters, int_valued)
                .ok_or("an index this test cannot place")?,
        };
        pre.push(Precondition::InRange {
            seq: s,
            least,
            greatest,
        });
    }
    Ok((pre, symbolic))
}

/// Whether some statement of `body` changes `s` other than by writing an item of it or of one of its rows: a
/// push, a pop, a rebinding, a swap, a call that may write it.
fn resized_or_rebound(body: &[Stmt], s: Symbol) -> bool {
    use logicaffeine_language::ast_depth::{for_each_node, Visited};
    let mut changed = false;
    for_each_node(body, |node| {
        if let Visited::Stmt(stmt) = node {
            let item_write = matches!(stmt,
                Stmt::SetIndex { collection: Expr::Identifier(c), .. } if *c == s)
                || matches!(stmt, Stmt::SetIndex { collection: Expr::Index { collection: Expr::Identifier(c), .. }, .. } if *c == s);
            if !item_write && WriteSet::of_stmt_shallow(stmt).may_write(s) {
                changed = true;
            }
        }
    });
    changed
}

/// The corners of an index that is a signed sum of terms no counter appears in (fixed over the band, any shape)
/// plus terms affine in the counters, each counter's coefficient a literal.
fn offset_corners<'a>(
    index: &'a Expr<'a>,
    loops: &[CountedLoop<'a>],
    counters: &BTreeSet<Symbol>,
    int_valued: &dyn Fn(&Expr) -> bool,
) -> Option<(Bound<'a>, Bound<'a>)> {
    let mut terms = Vec::new();
    signed_terms(index, true, &mut terms);
    let mut affine = konst(0);
    let mut invariant = Vec::new();
    for (positive, term) in terms {
        let names = identifiers(term);
        if names.iter().any(|n| counters.contains(n)) {
            let lin = lin_of(term)?;
            if !lin.coefficients.keys().all(|v| {
                loops
                    .iter()
                    .any(|l| var(l.counter).coefficients.contains_key(v))
            }) {
                return None;
            }
            affine = if positive {
                affine.add(&lin)
            } else {
                affine.sub(&lin)
            };
        } else {
            if !int_valued(term) {
                return None;
            }
            invariant.push((positive, term));
        }
    }
    let access = Access {
        coords: vec![affine],
        row_major: None,
        store: false,
        inner: None,
    };
    let (mut least, mut greatest) = (access.least(loops), access.greatest(loops));
    least.invariant = invariant.clone();
    greatest.invariant = invariant;
    Some((least, greatest))
}

/// `e` as a sum of signed terms (`a + b - c` is `+a, +b, -c`).
fn signed_terms<'a>(e: &'a Expr<'a>, positive: bool, out: &mut Vec<(bool, &'a Expr<'a>)>) {
    match e {
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => {
            signed_terms(left, positive, out);
            signed_terms(right, positive, out);
        }
        Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left,
            right,
        } => {
            signed_terms(left, positive, out);
            signed_terms(right, !positive, out);
        }
        other => out.push((positive, other)),
    }
}

/// What the innermost body does, read once.
#[derive(Default)]
struct Body<'a> {
    /// Each body-local's definition, while it is not reassigned.
    locals: BTreeMap<Symbol, &'a Expr<'a>>,
    /// Every access `(seq, index, store?, item-of-row)`, in body order: `index` is the row's index when the access
    /// reads or writes item `k` of a row, and the last field is that `k`.
    accesses: Vec<(Symbol, &'a Expr<'a>, bool, Option<i64>)>,
    /// Divisors fixed before the nest, which must not be zero.
    divisors: Vec<&'a Expr<'a>>,
}

impl<'a> Body<'a> {
    fn read(
        &mut self,
        stmts: &'a [Stmt<'a>],
        loops: &[CountedLoop<'a>],
    ) -> Result<(), &'static str> {
        let counters: BTreeSet<Symbol> = loops.iter().map(|l| l.counter).collect();
        for stmt in stmts {
            match stmt {
                Stmt::Let { var, value, .. } if !counters.contains(var) => {
                    self.expr(value, &counters)?;
                    self.locals.insert(*var, value);
                }
                Stmt::SetIndex {
                    collection: Expr::Identifier(seq),
                    index,
                    value,
                } => {
                    self.expr(index, &counters)?;
                    self.expr(value, &counters)?;
                    self.accesses.push((*seq, index, true, None));
                }
                Stmt::SetIndex {
                    collection:
                        Expr::Index {
                            collection: Expr::Identifier(seq),
                            index: row,
                        },
                    index: Expr::Literal(Literal::Number(k)),
                    value,
                } => {
                    self.expr(row, &counters)?;
                    self.expr(value, &counters)?;
                    self.accesses.push((*seq, row, true, Some(*k)));
                }
                Stmt::Set { target, value } if self.locals.contains_key(target) => {
                    self.expr(value, &counters)?;
                    self.locals.remove(target);
                }
                Stmt::Set { .. } => {
                    return Err("a variable carried from one iteration to the next")
                }
                Stmt::If {
                    cond,
                    then_block,
                    else_block,
                } => {
                    self.expr(cond, &counters)?;
                    self.read(then_block, loops)?;
                    if let Some(eb) = else_block {
                        self.read(eb, loops)?;
                    }
                }
                _ => return Err("a statement whose effect a reorder could show"),
            }
        }
        Ok(())
    }

    fn expr(&mut self, e: &'a Expr<'a>, counters: &BTreeSet<Symbol>) -> Result<(), &'static str> {
        match e {
            Expr::Literal(_) | Expr::Identifier(_) => Ok(()),
            Expr::Index {
                collection: Expr::Identifier(seq),
                index,
            } => {
                self.expr(index, counters)?;
                self.accesses.push((*seq, index, false, None));
                Ok(())
            }
            Expr::Index {
                collection:
                    Expr::Index {
                        collection: Expr::Identifier(seq),
                        index: row,
                    },
                index: Expr::Literal(Literal::Number(k)),
            } => {
                self.expr(row, counters)?;
                self.accesses.push((*seq, row, false, Some(*k)));
                Ok(())
            }
            Expr::Length {
                collection: Expr::Identifier(_),
            } => Ok(()),
            Expr::BinaryOp { op, left, right } => {
                self.expr(left, counters)?;
                self.expr(right, counters)?;
                match op {
                    BinaryOpKind::Divide | BinaryOpKind::Modulo => match right {
                        Expr::Literal(Literal::Number(n)) if *n != 0 => Ok(()),
                        d if lin_of(d).is_some()
                            && !counters
                                .iter()
                                .chain(self.locals.keys())
                                .any(|s| names(d, *s)) =>
                        {
                            self.divisors.push(d);
                            Ok(())
                        }
                        _ => Err("a divisor that changes inside the nest"),
                    },
                    _ => Ok(()),
                }
            }
            Expr::Not { operand } => self.expr(operand, counters),
            _ => Err("an expression whose effect a reorder could show"),
        }
    }

    /// Every access, normalized to its item's coordinates, grouped by sequence. Each name an index reads is a
    /// counter or a value `writes` never touches, so the coordinates mean the same on every iteration.
    fn dimensions(
        &self,
        loops: &[CountedLoop<'a>],
        writes: &WriteSet,
    ) -> Result<BTreeMap<Symbol, Vec<Access<'a>>>, &'static str> {
        let mut out: BTreeMap<Symbol, Vec<Access<'a>>> = BTreeMap::new();
        for &(seq, index, store, inner) in &self.accesses {
            let index = self.resolve(index);
            let fixed = |s: &Symbol| loops.iter().any(|l| l.counter == *s) || !writes.may_write(*s);
            if !identifiers(index).iter().all(fixed) {
                return Err("an index that reads a value the band changes");
            }
            let mut access =
                Access::of(index, loops, store).ok_or("an index this test cannot place")?;
            access.inner = inner;
            out.entry(seq).or_default().push(access);
        }
        Ok(out)
    }

    /// A body-local index stands for its definition.
    fn resolve(&self, e: &'a Expr<'a>) -> &'a Expr<'a> {
        match e {
            Expr::Identifier(s) => self.locals.get(s).copied().unwrap_or(e),
            _ => e,
        }
    }
}

/// One access's item: affine coordinates over the band's counters and values fixed before the nest. A plain
/// index is one coordinate; a row-major `R × S + C + k` is `(R, C)`, with `k` and `S` recorded so two accesses
/// compare only when they share them.
struct Access<'a> {
    coords: Vec<LinExpr>,
    row_major: Option<(&'a Expr<'a>, i64)>,
    store: bool,
    /// The item of the row this access reads or writes, when it reaches into a row.
    inner: Option<i64>,
}

impl<'a> Access<'a> {
    fn of(index: &'a Expr<'a>, loops: &[CountedLoop<'a>], store: bool) -> Option<Access<'a>> {
        if let Some(lin) = lin_of(index) {
            return Some(Access {
                coords: vec![lin],
                row_major: None,
                store,
                inner: None,
            });
        }
        let (r, s, c, k) = row_major(index)?;
        let spans_row = loops.iter().any(|l| {
            matches!(c, Expr::Identifier(x) if *x == l.counter)
                && matches!(l.lo, Expr::Literal(Literal::Number(0)))
                && !l.inclusive
                && same_expr(l.hi, s)
        });
        if !spans_row {
            return None;
        }
        Some(Access {
            coords: vec![lin_of(r)?, lin_of(c)?],
            row_major: Some((s, k)),
            store,
            inner: None,
        })
    }

    /// The least index this access takes over the band.
    fn least(&self, loops: &[CountedLoop<'a>]) -> Bound<'a> {
        self.corner(loops, false)
    }

    /// The greatest index this access takes over the band.
    fn greatest(&self, loops: &[CountedLoop<'a>]) -> Bound<'a> {
        self.corner(loops, true)
    }

    fn corner(&self, loops: &[CountedLoop<'a>], greatest: bool) -> Bound<'a> {
        let extreme = |e: &LinExpr| -> LinExpr {
            let mut out = LinExpr::constant(e.constant.clone());
            for (v, a) in &e.coefficients {
                let term = match loops
                    .iter()
                    .find(|l| var(l.counter).coefficients.contains_key(v))
                {
                    Some(l) if a.is_positive() == greatest => last_value(l),
                    Some(l) => lin_of(l.lo).expect("a band bound is affine"),
                    None => LinExpr::var(*v),
                };
                out = out.add(&term.scale(a));
            }
            out
        };
        match self.row_major {
            None => Bound {
                base: extreme(&self.coords[0]),
                scale: None,
                invariant: Vec::new(),
            },
            Some((s, k)) => {
                let col = if greatest {
                    lin_of(s).expect("a row stride is affine").sub(&konst(1))
                } else {
                    konst(0)
                };
                Bound {
                    base: col.add(&konst(k)),
                    scale: Some((extreme(&self.coords[0]), s)),
                    invariant: Vec::new(),
                }
            }
        }
    }
}

/// The last value a counted loop's counter takes.
fn last_value(l: &CountedLoop) -> LinExpr {
    let hi = lin_of(l.hi).expect("a band bound is affine");
    if l.inclusive {
        hi
    } else {
        hi.sub(&konst(1))
    }
}

/// `R × S + C + k` (any association of the sum), with `k` a literal.
fn row_major<'a>(e: &'a Expr<'a>) -> Option<(&'a Expr<'a>, &'a Expr<'a>, &'a Expr<'a>, i64)> {
    let mut terms = Vec::new();
    flatten_sum(e, &mut terms);
    let mut k = 0i64;
    let mut product = None;
    let mut column = None;
    for t in terms {
        match t {
            Expr::Literal(Literal::Number(n)) => k = k.checked_add(*n)?,
            Expr::BinaryOp {
                op: BinaryOpKind::Multiply,
                left,
                right,
            } if product.is_none() => product = Some((*left, *right)),
            other if column.is_none() => column = Some(other),
            _ => return None,
        }
    }
    let (r, s) = product?;
    Some((r, s, column?, k))
}

fn flatten_sum<'a>(e: &'a Expr<'a>, out: &mut Vec<&'a Expr<'a>>) {
    match e {
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } => {
            flatten_sum(left, out);
            flatten_sum(right, out);
        }
        other => out.push(other),
    }
}

/// The first dependence a tile reorder would swap, as a reason; `None` when the band is fully permutable.
fn reorder_swaps_a_dependence(
    dims: &BTreeMap<Symbol, Vec<Access>>,
    loops: &[CountedLoop],
) -> Option<&'static str> {
    for accesses in dims.values() {
        for (i, a) in accesses.iter().enumerate() {
            for b in &accesses[i..] {
                if !(a.store || b.store) {
                    continue;
                }
                match (a.inner, b.inner) {
                    (Some(x), Some(y)) if x != y => continue,
                    (Some(_), None) | (None, Some(_)) => {
                        return Some("an item of a row and a whole row of one sequence")
                    }
                    _ => {}
                }
                if !same_layout(a, b) {
                    return Some("two accesses to one sequence this test cannot compare");
                }
                if swapped_dependence(a, b, loops) || swapped_dependence(b, a, loops) {
                    return Some("iterations a tile reorder swaps touch one item");
                }
            }
        }
    }
    None
}

/// Two accesses name one item exactly when their coordinates agree: both plain, or both row-major with one
/// stride and one offset.
fn same_layout(a: &Access, b: &Access) -> bool {
    match (a.row_major, b.row_major) {
        (Some((sa, ka)), Some((sb, kb))) => ka == kb && same_expr(sa, sb),
        (None, None) => true,
        _ => false,
    }
}

/// Some iteration `I` running `source` and a later iteration `I'` running `sink` touch one item while `I'`
/// precedes `I` in some band counter.
fn swapped_dependence(source: &Access, sink: &Access, loops: &[CountedLoop]) -> bool {
    let d = loops.len();
    let mut base: Vec<Constraint> = Vec::new();
    for l in loops {
        for c in [var(l.counter), primed(&var(l.counter), loops)] {
            let lo = lin_of(l.lo).expect("a band bound is affine");
            base.push(le(&lo, &c));
            base.push(le(&c, &last_value(l)));
        }
    }
    for (x, y) in source.coords.iter().zip(&sink.coords) {
        let y = primed(y, loops);
        base.push(le(x, &y));
        base.push(le(&y, x));
    }
    for m in 0..d {
        for t in (m + 1)..d {
            let mut system = base.clone();
            for l in &loops[..m] {
                let (c, cp) = (var(l.counter), primed(&var(l.counter), loops));
                system.push(le(&c, &cp));
                system.push(le(&cp, &c));
            }
            system.push(lt(
                &var(loops[m].counter),
                &primed(&var(loops[m].counter), loops),
            ));
            system.push(lt(
                &primed(&var(loops[t].counter), loops),
                &var(loops[t].counter),
            ));
            if consistent(&system) {
                return true;
            }
        }
    }
    false
}

/// `e` with each band counter replaced by its copy in the later iteration.
fn primed(e: &LinExpr, loops: &[CountedLoop]) -> LinExpr {
    const PRIME: i64 = 1 << 40;
    let mut out = LinExpr::constant(e.constant.clone());
    for (v, a) in &e.coefficients {
        let is_counter = loops
            .iter()
            .any(|l| var(l.counter).coefficients.contains_key(v));
        let v = if is_counter { v + PRIME } else { *v };
        out = out.add(&LinExpr::var(v).scale(a));
    }
    out
}

/// `e` names `x` anywhere inside it (the exhaustive child walk).
fn names(e: &Expr, x: Symbol) -> bool {
    let mut stack = vec![e];
    while let Some(e) = stack.pop() {
        if matches!(e, Expr::Identifier(s) if *s == x) {
            return true;
        }
        stack.extend(expr_children(e));
    }
    false
}

/// Every name an expression reads (the exhaustive child walk).
fn identifiers(e: &Expr) -> BTreeSet<Symbol> {
    let mut out = BTreeSet::new();
    let mut stack = vec![e];
    while let Some(e) = stack.pop() {
        if let Expr::Identifier(s) = e {
            out.insert(*s);
        }
        stack.extend(expr_children(e));
    }
    out
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/dependence/tests.rs"]
mod tests;
