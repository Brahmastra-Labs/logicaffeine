//! Loops driven by a clock: `While <guard>:` whose body steps a counter `t` the guard reads by an
//! invariant amount once per pass. When the step is infinitesimal, or the guard reads a value of the
//! exact extension that can be infinite, the loop may run an infinite number of times, so the engine
//! writes, in its place:
//!
//! ```text
//! Let N be tripCount(t, step, bound, "<").
//! If isLimited(N):
//!     <the loop exactly as written>
//! Otherwise:
//!     <the end state after N steps>
//! ```
//!
//! A guard `t < bound` is counted by `tripCount`. Any other guard built from `and`, `or` and `not`
//! over comparisons of terms linear in `t` and of their remainders — `not (n <= x and x < n + 1)`,
//! the floor of Imamura's Lemma 1 — is handed to `firstExit` as a formula, which finds the first
//! pass at which it fails. A loop that runs finitely often keeps exactly the loop as written; only a
//! loop that could never finish takes the other branch. The end state comes from the first strategy
//! that applies:
//!
//! - **the affine closed form**: a body of assignments that is a polynomial map of the variables it
//!   sets, whose monomials `m` close under it, is an affine map of them, `m ↦ A·m + b`, and ends in
//!   `affinePower([[A, b], [0, 1]], [m, 1], N)` — an affine map closes on its own variables, and
//!   `x ← x + t·t·dt, t ← t + dt` on `x, t, t²`;
//! - **the flow**: a body of assignments that is a polynomial Euler step `x ↦ x + dt·f(x) + …`
//!   ends in `polynomialFlow(f, x, N)`, the solution of `x' = f(x)` at the time the steps reach;
//! - **the hybrid flow**: a body of Euler steps and jumps `If guard: …` that fire at isolated steps
//!   — a bouncing ball — ends in `hybridFlow(…)`, the ideal hybrid system it follows (see
//!   [`super::hybrid`]);
//! - **the cycle**: a body whose state is finite enough to repeat — a toggle, a phase, a remainder
//!   walk — is run until a state recurs, and the state at step `N` is read off the cycle;
//! - otherwise the program stops with the reason, rather than running forever.
//!
//! Before any of them, each supertask nested in the pass that reads what changes from pass to pass is
//! written as the map it is ([`compose_nested`]), and the part of a pass that reads nothing the passes
//! carry — a supertask nested in it, over locals the pass binds — runs once ahead of the end state
//! ([`super::stage`]).

use logicaffeine_base::Symbol;

use super::build::{add, literal, scale, Build};
use super::linear::{invariant_in, linear, remainder};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Pattern, Stmt};
use crate::optimize::mentions::Mentions;

/// What a clock loop that has no end state stops the program with, before the reason.
const NO_ANSWER: &str = "this loop's clock steps by an infinitesimal, so it runs infinitely many times, and the \
                         supertask engine has no exact answer for its body: ";

/// The steps a supertask's state is followed before its cycle is given up on.
const CYCLE_BUDGET: i64 = 100_000;

/// The bits a followed state may grow to before the cycle search gives up on it. A step's work grows
/// with the square of the state it multiplies, so the bound keeps the last step cheap as well as the
/// states the search keeps: a state that squares a number each step passes it within about sixteen
/// steps.
const STATE_BUDGET_BITS: i64 = 1 << 16;

/// A recognized clock: `While <guard>:` with `Set var to var + step` in the body.
#[derive(Clone)]
pub(super) struct Clock<'a> {
    pub(super) var: Symbol,
    guard: Guard<'a>,
    pub(super) step: &'a Expr<'a>,
    /// The position of the clock's own step among the body's statements.
    pub(super) step_at: usize,
}

/// The identifiers a simple expression reads — literals, names, arithmetic, comparisons and `not`
/// — or `None` for anything else (a call, an index, a field), whose purity the engine does not
/// vouch for.
pub(super) fn reads(e: &Expr<'_>, out: &mut Vec<Symbol>) -> Option<()> {
    match e {
        Expr::Literal(_) => Some(()),
        Expr::Identifier(s) => {
            out.push(*s);
            Some(())
        }
        Expr::BinaryOp { left, right, .. } => {
            reads(left, out)?;
            reads(right, out)
        }
        Expr::Not { operand } => reads(operand, out),
        _ => None,
    }
}

/// The variables a block of `Set`/`Let`/`If` statements assigns and binds locally, or `None` when
/// the block holds any other statement.
fn assignments(block: &[Stmt<'_>], sets: &mut Vec<Symbol>, locals: &mut Vec<Symbol>, read: &mut Vec<Symbol>) -> Option<()> {
    for s in block {
        match s {
            Stmt::Set { target, value } => {
                if !sets.contains(target) {
                    sets.push(*target);
                }
                reads(value, read)?;
            }
            Stmt::Let { var, value, .. } => {
                locals.push(*var);
                reads(value, read)?;
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                reads(cond, read)?;
                assignments(then_block, sets, locals, read)?;
                if let Some(b) = else_block {
                    assignments(b, sets, locals, read)?;
                }
            }
            _ => return None,
        }
    }
    Some(())
}

fn invariant(e: &Expr<'_>, assigned: &[Symbol]) -> bool {
    let mut names = Vec::new();
    reads(e, &mut names).is_some() && names.iter().all(|n| !assigned.contains(n))
}

/// Every variable any statement of `block` sets, at any depth.
fn all_sets(block: &[Stmt<'_>], out: &mut Vec<Symbol>) {
    for s in block {
        match s {
            Stmt::Set { target, .. } => out.push(*target),
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                all_sets(then_block, out);
                if let Some(b) = else_block {
                    all_sets(b, out);
                }
            }
            Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => all_sets(body, out),
            _ => {}
        }
    }
}

/// Every variable a statement of `block` sets, at any depth, that no block inside `block` binds
/// first: what running `block` changes outside it. A spliced block binds in the block around it.
fn escaping_sets(block: &[Stmt<'_>], out: &mut Vec<Symbol>) {
    fn walk(block: &[Stmt<'_>], bound: &mut Vec<Symbol>, out: &mut Vec<Symbol>) {
        for s in block {
            match s {
                Stmt::Let { var, .. } => bound.push(*var),
                Stmt::Set { target, .. } => {
                    if !bound.contains(target) && !out.contains(target) {
                        out.push(*target);
                    }
                }
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => {
                    scoped(then_block, bound, out);
                    if let Some(b) = else_block {
                        scoped(b, bound, out);
                    }
                }
                Stmt::While { body, .. } => scoped(body, bound, out),
                Stmt::Repeat { pattern, body, .. } => {
                    let mark = bound.len();
                    if let Pattern::Identifier(v) = pattern {
                        bound.push(*v);
                    }
                    scoped(body, bound, out);
                    bound.truncate(mark);
                }
                Stmt::Splice { body } => walk(body, bound, out),
                _ => {}
            }
        }
    }
    fn scoped(block: &[Stmt<'_>], bound: &mut Vec<Symbol>, out: &mut Vec<Symbol>) {
        let mark = bound.len();
        walk(block, bound, out);
        bound.truncate(mark);
    }
    scoped(block, &mut Vec::new(), out);
}

/// What a clock loop runs while.
#[derive(Clone)]
enum Guard<'a> {
    /// `t <relation> bound`, counted by `tripCount`.
    Bound {
        relation: &'static str,
        bound: &'a Expr<'a>,
    },
    /// Any other guard built from `and`, `or` and `not` over comparisons of terms linear in `t` and
    /// of their remainders, as the formula `firstExit` reads.
    Formula(&'a Expr<'a>),
}

/// The relation a comparison names, as `tripCount` and `firstExit` read it.
fn relation_of(op: BinaryOpKind) -> Option<&'static str> {
    Some(match op {
        BinaryOpKind::Lt => "<",
        BinaryOpKind::LtEq => "<=",
        BinaryOpKind::Gt => ">",
        BinaryOpKind::GtEq => ">=",
        BinaryOpKind::Eq => "=",
        BinaryOpKind::NotEq => "!=",
        _ => return None,
    })
}

/// The relation with its two sides swapped.
fn flipped(relation: &'static str) -> &'static str {
    match relation {
        "<" => ">",
        "<=" => ">=",
        ">" => "<",
        ">=" => "<=",
        other => other,
    }
}

/// `t <relation> bound`, either way round, for an ordering relation.
fn bound_of<'a>(cond: &'a Expr<'a>) -> Option<(Symbol, &'a Expr<'a>, &'static str)> {
    let Expr::BinaryOp { op, left, right } = cond else {
        return None;
    };
    let relation = relation_of(*op).filter(|r| !matches!(*r, "=" | "!="))?;
    match (left, right) {
        (Expr::Identifier(t), other) => Some((*t, *other, relation)),
        (other, Expr::Identifier(t)) => Some((*t, *other, flipped(relation))),
        _ => None,
    }
}

/// `cond` as the formula `firstExit` reads over the clock `t`: `["and", f, g]`, `["or", f, g]`,
/// `["not", f]`, `[relation, a, b]` for `a·t + b <relation> 0`, and `["%", relation, a, b, m, c]`
/// for `(a·t + b) % m <relation> c`; `None` for a guard that is anything else.
fn formula<'a>(cond: &'a Expr<'a>, t: Symbol, assigned: &[Symbol], b: &mut Build<'a, '_>) -> Option<&'a Expr<'a>> {
    match cond {
        Expr::Not { operand } => {
            let inner = formula(operand, t, assigned, b)?;
            let tag = b.text("not");
            Some(b.list(vec![tag, inner]))
        }
        Expr::BinaryOp {
            op: op @ (BinaryOpKind::And | BinaryOpKind::Or),
            left,
            right,
        } => {
            let (l, r) = (formula(left, t, assigned, b)?, formula(right, t, assigned, b)?);
            let tag = b.text(if *op == BinaryOpKind::And { "and" } else { "or" });
            Some(b.list(vec![tag, l, r]))
        }
        Expr::BinaryOp { op, left, right } => {
            let relation = relation_of(*op)?;
            for (side, other, relation) in [(*left, *right, relation), (*right, *left, flipped(relation))] {
                if let Some((term, m)) = remainder(side, t, assigned, b) {
                    if !invariant_in(other, t, assigned) {
                        return None;
                    }
                    let a = term.a.unwrap_or_else(|| b.number(0));
                    let c = term.b.unwrap_or_else(|| b.number(0));
                    let (tag, rel) = (b.text("%"), b.text(relation));
                    return Some(b.list(vec![tag, rel, a, c, m, other]));
                }
            }
            let (l, r) = (linear(left, t, assigned, b)?, linear(right, t, assigned, b)?);
            let a = add(b, l.a, r.a, true).unwrap_or_else(|| b.number(0));
            let c = add(b, l.b, r.b, true).unwrap_or_else(|| b.number(0));
            let rel = b.text(relation);
            Some(b.list(vec![rel, a, c]))
        }
        _ => None,
    }
}

/// The statement of `body` that steps `t`, and the amount: `Set t to t + step`, `step + t` or
/// `t - step`, as a statement of the body itself.
fn step_of<'a>(body: &'a [Stmt<'a>], t: Symbol, b: &Build<'a, '_>) -> Option<(usize, &'a Expr<'a>)> {
    let (at, value) = body.iter().enumerate().find_map(|(i, s)| match s {
        Stmt::Set { target, value } if *target == t => Some((i, *value)),
        _ => None,
    })?;
    let is_clock = |e: &Expr<'_>| matches!(e, Expr::Identifier(s) if *s == t);
    let step = match value {
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } if is_clock(left) => *right,
        Expr::BinaryOp {
            op: BinaryOpKind::Add,
            left,
            right,
        } if is_clock(right) => *left,
        Expr::BinaryOp {
            op: BinaryOpKind::Subtract,
            left,
            right,
        } if is_clock(left) => b.binary(BinaryOpKind::Subtract, b.number(0), right),
        _ => return None,
    };
    Some((at, step))
}

/// A loop whose body steps one counter its guard reads exactly once, by an invariant amount, while
/// the guard compares terms linear in the counter. A step that names a variable is a clock wherever
/// the program declares an infinitesimal; a step written as a constant is one only when the guard
/// reads a value of the exact extension (`reached`), which can put its bound beyond every natural —
/// any other such loop runs finitely often and is left exactly as it is.
pub(super) fn recognize<'a>(
    cond: &'a Expr<'a>,
    body: &'a [Stmt<'a>],
    reached: &dyn Fn(&Expr<'_>) -> bool,
    b: &mut Build<'a, '_>,
) -> Option<Clock<'a>> {
    let mut sets = Vec::new();
    all_sets(body, &mut sets);
    let once = |v: Symbol| sets.iter().filter(|s| **s == v).count() == 1;
    let (var, guard) = match bound_of(cond) {
        Some((t, bound, relation)) if once(t) && invariant(bound, &sets) => (t, Guard::Bound { relation, bound }),
        _ => {
            let mut read = Vec::new();
            reads(cond, &mut read)?;
            let mut counters: Vec<Symbol> = Vec::new();
            for s in read {
                if once(s) && !counters.contains(&s) && step_of(body, s, b).is_some() {
                    counters.push(s);
                }
            }
            let [t] = counters.as_slice() else {
                return None;
            };
            (*t, Guard::Formula(formula(cond, *t, &sets, b)?))
        }
    };
    let (step_at, step) = step_of(body, var, b)?;
    let mut step_reads = Vec::new();
    if !invariant(step, &sets) || reads(step, &mut step_reads).is_none() {
        return None;
    }
    if step_reads.is_empty() && !reached(cond) {
        return None;
    }
    Some(Clock {
        var,
        guard,
        step,
        step_at,
    })
}

// ── polynomial forms ──────────────────────────────────────────────────────────────────────────

/// The largest total degree a form may reach before the body is left to the next strategy.
const MAX_DEGREE: u32 = 8;

/// The most monomials a form may hold: a product multiplies the sizes of its factors.
const MAX_TERMS: usize = 64;

/// A polynomial over the state: each monomial by its exponents, with its coefficient, an invariant
/// expression. A monomial not listed has coefficient zero.
#[derive(Clone)]
pub(super) struct Form<'a> {
    pub(super) terms: Vec<(Vec<u32>, &'a Expr<'a>)>,
}

impl<'a> Form<'a> {
    fn of(terms: Vec<(Vec<u32>, &'a Expr<'a>)>) -> Form<'a> {
        Form {
            terms: terms.into_iter().filter(|(_, c)| literal(c) != Some(0)).collect(),
        }
    }

    fn constant(width: usize, c: &'a Expr<'a>) -> Form<'a> {
        Form::of(vec![(vec![0; width], c)])
    }

    pub(super) fn unit(width: usize, i: usize, b: &Build<'a, '_>) -> Form<'a> {
        let mut exponents = vec![0; width];
        exponents[i] = 1;
        Form::of(vec![(exponents, b.number(1))])
    }

    fn degree(&self) -> u32 {
        self.terms.iter().map(|(e, _)| e.iter().sum()).max().unwrap_or(0)
    }

    fn coefficient(&self, exponents: &[u32]) -> Option<&'a Expr<'a>> {
        self.terms.iter().find(|(e, _)| e.as_slice() == exponents).map(|(_, c)| *c)
    }
}

pub(super) fn combine<'a>(b: &Build<'a, '_>, x: &Form<'a>, y: &Form<'a>, negate_y: bool) -> Form<'a> {
    let mut exponents: Vec<Vec<u32>> = x.terms.iter().chain(&y.terms).map(|(e, _)| e.clone()).collect();
    exponents.sort();
    exponents.dedup();
    Form::of(
        exponents
            .into_iter()
            .filter_map(|e| add(b, x.coefficient(&e), y.coefficient(&e), negate_y).map(|c| (e, c)))
            .collect(),
    )
}

fn scaled<'a>(b: &Build<'a, '_>, x: &Form<'a>, by: &'a Expr<'a>, op: BinaryOpKind) -> Form<'a> {
    Form::of(
        x.terms
            .iter()
            .filter_map(|(e, c)| scale(b, Some(*c), by, op).map(|c| (e.clone(), c)))
            .collect(),
    )
}

fn product<'a>(b: &Build<'a, '_>, x: &Form<'a>, y: &Form<'a>) -> Option<Form<'a>> {
    let mut out = Form { terms: Vec::new() };
    for (e, c) in &x.terms {
        for (f, d) in &y.terms {
            let exponents: Vec<u32> = e.iter().zip(f).map(|(i, j)| i + j).collect();
            if let Some(coefficient) = scale(b, Some(*c), d, BinaryOpKind::Multiply) {
                out = combine(b, &out, &Form::of(vec![(exponents, coefficient)]), false);
            }
        }
    }
    (out.degree() <= MAX_DEGREE && out.terms.len() <= MAX_TERMS).then_some(out)
}

/// The polynomial form of `e` over the state, or `None` when `e` is not a polynomial in it (a
/// division by a state term or one that may floor, a comparison, a call) or grows past
/// [`MAX_DEGREE`] or [`MAX_TERMS`].
pub(super) fn polynomial<'a>(
    e: &'a Expr<'a>,
    state: &[Symbol],
    env: &[(Symbol, Form<'a>)],
    b: &Build<'a, '_>,
) -> Option<Form<'a>> {
    let width = state.len();
    match e {
        Expr::Literal(Literal::Number(_)) | Expr::Literal(Literal::Float(_)) => Some(Form::constant(width, e)),
        Expr::Identifier(s) => Some(match env.iter().rev().find(|(name, _)| name == s) {
            Some((_, form)) => form.clone(),
            None => Form::constant(width, e),
        }),
        Expr::BinaryOp { op, left, right } => {
            let l = polynomial(left, state, env, b)?;
            let r = polynomial(right, state, env, b)?;
            match op {
                BinaryOpKind::Add => Some(combine(b, &l, &r, false)),
                BinaryOpKind::Subtract => Some(combine(b, &l, &r, true)),
                BinaryOpKind::Multiply => product(b, &l, &r),
                BinaryOpKind::ExactDivide if r.degree() == 0 => {
                    Some(scaled(b, &l, r.coefficient(&vec![0; width])?, BinaryOpKind::ExactDivide))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Each state variable's form after one pass of a straight-line body, or `None` when the body is
/// not a polynomial map of the state.
pub(super) fn update_forms<'a>(body: &'a [Stmt<'a>], state: &[Symbol], b: &Build<'a, '_>) -> Option<Vec<Form<'a>>> {
    let width = state.len();
    let mut env: Vec<(Symbol, Form<'a>)> = state
        .iter()
        .enumerate()
        .map(|(i, s)| (*s, Form::unit(width, i, b)))
        .collect();
    for s in body {
        match s {
            Stmt::Set { target, value } | Stmt::Let { var: target, value, .. } => {
                let form = polynomial(value, state, &env, b)?;
                env.push((*target, form));
            }
            _ => return None,
        }
    }
    Some(
        state
            .iter()
            .map(|s| {
                env.iter()
                    .rev()
                    .find(|(name, _)| name == s)
                    .map(|(_, f)| f.clone())
                    .expect("every state variable has a form")
            })
            .collect(),
    )
}

/// The most monomials a polynomial map is written over as an affine map.
const MAX_BASIS: usize = 24;

/// A polynomial map of the state written as an affine map of monomials: `basis` the monomials the
/// map carries into combinations of one another, the state's own variables first, and `rows` its
/// update matrix over them, `[[A, b], [0, 1]]`.
pub(super) struct Linear<'a> {
    basis: Vec<Vec<u32>>,
    rows: Vec<Vec<&'a Expr<'a>>>,
}

impl<'a> Linear<'a> {
    /// The vector the map acts on: each monomial of the basis over `vars`, then `1`.
    fn vector(&self, vars: &[Symbol], b: &Build<'a, '_>) -> Vec<&'a Expr<'a>> {
        let mut v: Vec<&'a Expr<'a>> = self.basis.iter().map(|m| monomial(m, vars, b)).collect();
        v.push(b.number(1));
        v
    }
}

/// `forms` as an affine map of the monomials they reach. A monomial's image is the product of its
/// variables' forms, and every monomial an image holds joins the basis, until the basis closes: an
/// affine map closes on the state itself, and `x ← x + t·t·dt, t ← t + dt` on `x, t, t²`, whose image
/// `t² + 2·dt·t + dt²` holds nothing new. Iterating the affine map over the monomials is iterating the
/// polynomial map exactly. `None` when the basis grows past [`MAX_BASIS`] or an image past
/// [`MAX_DEGREE`] — `x ← x + x·x·dt` reaches every power of `x`.
fn linearized<'a>(forms: &[Form<'a>], b: &Build<'a, '_>) -> Option<Linear<'a>> {
    let width = forms.len();
    let mut basis: Vec<Vec<u32>> = (0..width).map(|i| Form::unit(width, i, b).terms[0].0.clone()).collect();
    let mut images: Vec<Form<'a>> = forms.to_vec();
    let mut at = 0;
    while at < images.len() {
        let reached: Vec<Vec<u32>> = images[at].terms.iter().map(|(e, _)| e.clone()).collect();
        for e in reached {
            if e.iter().all(|p| *p == 0) || basis.contains(&e) {
                continue;
            }
            if basis.len() == MAX_BASIS {
                return None;
            }
            let mut image = Form::constant(width, b.number(1));
            for (i, power) in e.iter().enumerate() {
                for _ in 0..*power {
                    image = product(b, &image, &forms[i])?;
                }
            }
            basis.push(e);
            images.push(image);
        }
        at += 1;
    }
    let constant = vec![0; width];
    let mut rows: Vec<Vec<&'a Expr<'a>>> = images
        .iter()
        .map(|image| {
            basis
                .iter()
                .chain(std::iter::once(&constant))
                .map(|m| image.coefficient(m).unwrap_or_else(|| b.number(0)))
                .collect()
        })
        .collect();
    let mut last = vec![b.number(0); basis.len()];
    last.push(b.number(1));
    rows.push(last);
    Some(Linear { basis, rows })
}

/// The monomial `vars^exponents`, `1` for the empty one.
fn monomial<'a>(exponents: &[u32], vars: &[Symbol], b: &Build<'a, '_>) -> &'a Expr<'a> {
    let mut product: Option<&'a Expr<'a>> = None;
    for (v, power) in vars.iter().zip(exponents) {
        for _ in 0..*power {
            let factor = b.ident(*v);
            product = Some(match product {
                None => factor,
                Some(p) => b.binary(BinaryOpKind::Multiply, p, factor),
            });
        }
    }
    product.unwrap_or_else(|| b.number(1))
}

/// Whether polynomial forms read as an Euler step `x ← x + dt·f(x) + …`: every monomial of a
/// component other than its own variable's carries a coefficient that reads the clock's step.
/// Whether the coefficients really are `1 + O(dt)` and `O(dt)` is checked when they are known.
fn euler_step(forms: &[Form<'_>], clock: &Clock<'_>) -> bool {
    let mut step_reads = Vec::new();
    if reads(clock.step, &mut step_reads).is_none() || forms.iter().all(|f| f.degree() <= 1) {
        return false;
    }
    forms.iter().enumerate().all(|(i, form)| {
        form.terms.iter().all(|(exponents, coefficient)| {
            let own = exponents.iter().enumerate().all(|(j, e)| *e == u32::from(j == i));
            let mut read = Vec::new();
            own || (reads(coefficient, &mut read).is_some() && read.iter().any(|r| step_reads.contains(r)))
        })
    })
}

// ── resolution ────────────────────────────────────────────────────────────────────────────────

/// The statements that replace a clock loop. `original` is the loop as written; `body` is the pass
/// the strategies read, or the reason none can; `rule` is the limit rule in force, if any.
pub(super) fn resolve<'a>(
    clock: Clock<'a>,
    original: Stmt<'a>,
    body: Result<&'a [Stmt<'a>], String>,
    rule: Option<&str>,
    b: &mut Build<'a, '_>,
) -> Stmt<'a> {
    let n = b.fresh("n");
    let t = b.ident(clock.var);
    let count = match clock.guard {
        Guard::Bound { relation, bound } => {
            let relation = b.text(relation);
            b.call("tripCount", vec![t, clock.step, bound, relation])
        }
        Guard::Formula(formula) => b.call("firstExit", vec![t, clock.step, formula]),
    };
    let limited = b.call("isLimited", vec![b.ident(n)]);
    let infinite = match body.and_then(|body| end_state(&clock, body, n, rule, b)) {
        Ok(stmts) => stmts,
        Err(reason) => vec![b.fail(&format!("{NO_ANSWER}{reason}"))],
    };
    let count = b.let_(n, count);
    let branch = b.if_(limited, vec![original], Some(infinite));
    b.splice(vec![count, branch])
}

fn end_state<'a>(
    clock: &Clock<'a>,
    body: &'a [Stmt<'a>],
    n: Symbol,
    rule: Option<&str>,
    b: &mut Build<'a, '_>,
) -> Result<Vec<Stmt<'a>>, String> {
    let composed = compose_nested(body, &[], clock.step_at, "", b);
    if let Some(pass) = composed.pass {
        let pass = b.block(pass);
        let clock = Clock {
            step_at: composed.kept_at,
            ..clock.clone()
        };
        return end_state(&clock, pass, n, rule, b);
    }
    if let Some(why) = composed.refused {
        return Err(why);
    }
    if let Some(split) = super::stage::split(body, clock.step_at, b.interner) {
        let rest = b.block(split.rest);
        let pass = Clock {
            step_at: split.kept_at,
            ..clock.clone()
        };
        let mut out = split.ahead;
        out.extend(end_state(&pass, rest, n, rule, b)?);
        return Ok(out);
    }
    if let Some(resolved) = super::hybrid::resolve(clock, body, n, b) {
        let resolved = resolved?;
        let mut out = resolved.stmts;
        if let Some(rule) = rule {
            let rule = b.text(rule);
            for s in &resolved.written {
                let limit = b.call("limitOf", vec![b.ident(*s), rule]);
                out.push(b.set(*s, limit));
            }
        }
        return Ok(out);
    }
    let (mut sets, mut locals, mut read) = (Vec::new(), Vec::new(), Vec::new());
    if assignments(body, &mut sets, &mut locals, &mut read).is_none() {
        return Err("it does more than assign variables at each step".to_string());
    }
    let others: Vec<Stmt<'a>> = body
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != clock.step_at)
        .map(|(_, s)| s.clone())
        .collect();
    let mut read = Vec::new();
    let (mut ignored_sets, mut ignored_locals) = (Vec::new(), Vec::new());
    assignments(&others, &mut ignored_sets, &mut ignored_locals, &mut read).expect("a part of an assignment block");
    let state: Vec<Symbol> = sets.into_iter().filter(|s| !locals.contains(s)).collect();
    let forms = update_forms(body, &state, b);
    let linear = forms.as_deref().and_then(|forms| linearized(forms, b));
    let mut out = match linear {
        Some(linear) => {
            let x = b.fresh("x");
            let vector = b.list(linear.vector(&state, b));
            let matrix = b.list(linear.rows.into_iter().map(|r| b.list(r)).collect());
            let power = b.call("affinePower", vec![matrix, vector, b.ident(n)]);
            let mut out = vec![b.let_(x, power)];
            for (i, s) in state.iter().enumerate() {
                out.push(b.set(*s, b.item(b.ident(x), i + 1)));
            }
            out
        }
        None => match forms.filter(|forms| euler_step(forms, clock)) {
            Some(forms) => polynomial_flow(clock, &forms, &state, n, b),
            None => cycle(clock, body, &state, &read, n, b)?,
        },
    };
    if let Some(rule) = rule {
        let rule = b.text(rule);
        for s in &state {
            let resolved = b.call("limitOf", vec![b.ident(*s), rule]);
            out.push(b.set(*s, resolved));
        }
    }
    Ok(out)
}

// ── a supertask nested in each pass ──────────────────────────────────────────────────────────

/// A pass with the supertasks resolved inside it written as the maps they are.
struct Composed<'a> {
    /// The pass, when a supertask in it was written as its map.
    pass: Option<Vec<Stmt<'a>>>,
    /// Where the statement at the position asked for stands in that pass.
    kept_at: usize,
    /// Why the first supertask that reads what changes from pass to pass has no map the pass can
    /// carry.
    refused: Option<String>,
}

/// The pass `body` with every supertask resolved inside it that touches what changes from one pass to
/// the next — what the pass carries, a local bound to a value of it, a name in `outer`, which the
/// loops around it change — written as its map.
///
/// A supertask whose pass, its own nested supertasks composed first, is a polynomial map of its
/// variables `x` and of the changing names `c` it reads, which stay fixed through it, is an affine map
/// `M` of the monomials `m(x, c)` that close under it ([`linearized`]), and ends at `x ← P·[m(x, c), 1]`
/// for `P = M^N`, `N` its count. When `M` and `N` read nothing that changes, `P` is the same at every
/// pass, so it is computed once ahead of the end state ([`super::stage`]) and the rest of the pass is
/// assignments the strategies read; the part of the inner pass that reads nothing that changes is
/// computed with `P`, and only when the supertask runs a step. A nested count reads the inner clock's
/// start through the pass's bindings of it.
///
/// `path` keeps the names written apart from those of every other supertask the resolution writes.
fn compose_nested<'a>(
    body: &'a [Stmt<'a>],
    outer: &[Symbol],
    keep: usize,
    path: &str,
    b: &mut Build<'a, '_>,
) -> Composed<'a> {
    let locals: Vec<Symbol> = body
        .iter()
        .filter_map(|s| match s {
            Stmt::Let { var, .. } => Some(*var),
            _ => None,
        })
        .collect();
    let mut written = Vec::new();
    escaping_sets(body, &mut written);
    let mut carried: Vec<Symbol> = outer.to_vec();
    for w in written {
        if !carried.contains(&w) {
            carried.push(w);
        }
    }
    let mut known: Vec<(Symbol, &'a Expr<'a>)> = Vec::new();
    let mut composed = Composed {
        pass: None,
        kept_at: keep,
        refused: None,
    };
    let mut out = Vec::with_capacity(body.len());
    let mut changed = false;
    for (at, s) in body.iter().enumerate() {
        if at == keep {
            composed.kept_at = out.len();
        }
        let varying: Vec<Symbol> = carried
            .iter()
            .copied()
            .chain(locals.iter().copied().filter(|l| !known.iter().any(|(k, _)| k == l)))
            .collect();
        let stmts = match nested(s, &varying, &known, &format!("{path}{at}_"), b) {
            Nested::Map(stmts) => {
                changed = true;
                stmts
            }
            Nested::Refused(why) => {
                composed.refused.get_or_insert(why);
                vec![s.clone()]
            }
            Nested::Not => vec![s.clone()],
        };
        let mut w = Vec::new();
        all_sets(&stmts, &mut w);
        if let Stmt::Let { var, .. } = s {
            w.push(*var);
        }
        known.retain(|(k, v)| !w.contains(k) && steady(v, &w, b.interner));
        if let Stmt::Let { var, value, .. } = s {
            if steady(value, &varying, b.interner) {
                known.push((*var, *value));
            }
        }
        out.extend(stmts);
    }
    if changed {
        composed.pass = Some(out);
    }
    composed
}

/// What [`nested`] makes of a statement of a pass.
enum Nested<'a> {
    /// No supertask that touches what changes from pass to pass.
    Not,
    /// The supertask written as its map.
    Map(Vec<Stmt<'a>>),
    /// Why the supertask, which touches what changes from pass to pass, has no map the pass can carry.
    Refused(String),
}

/// A supertask the engine resolved: `Let n be <count>. If isLimited(n): <the loop>. Otherwise: …`.
struct Resolved<'a> {
    n: Symbol,
    count: &'a Expr<'a>,
    cond: &'a Expr<'a>,
    body: &'a [Stmt<'a>],
    otherwise: &'a [Stmt<'a>],
}

fn resolved<'a>(s: &Stmt<'a>, interner: &logicaffeine_base::Interner) -> Option<Resolved<'a>> {
    let Stmt::Splice { body: parts } = s else {
        return None;
    };
    let [Stmt::Let { var: n, value: count, .. }, Stmt::If {
        cond: limited,
        then_block,
        else_block: Some(otherwise),
    }] = *parts
    else {
        return None;
    };
    let Expr::Call { function, args } = limited else {
        return None;
    };
    if interner.resolve(*function) != "isLimited" || !matches!(args.as_slice(), [Expr::Identifier(m)] if m == n) {
        return None;
    }
    let [Stmt::While { cond, body, .. }] = *then_block else {
        return None;
    };
    Some(Resolved {
        n: *n,
        count,
        cond,
        body,
        otherwise,
    })
}

/// Whether a resolved supertask stands anywhere in `block`.
fn holds_supertask(block: &[Stmt<'_>], interner: &logicaffeine_base::Interner) -> bool {
    block.iter().any(|s| {
        resolved(s, interner).is_some()
            || match s {
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => holds_supertask(then_block, interner) || else_block.is_some_and(|e| holds_supertask(e, interner)),
                Stmt::While { body, .. } | Stmt::Repeat { body, .. } | Stmt::Splice { body } => {
                    holds_supertask(body, interner)
                }
                _ => false,
            }
    })
}

/// A statement of a pass whose names in `varying` change from pass to pass, and `known` the pass's
/// bindings, so far, of values that do not: a resolved supertask that touches what changes, as its
/// map. See [`compose_nested`].
fn nested<'a>(
    s: &Stmt<'a>,
    varying: &[Symbol],
    known: &[(Symbol, &'a Expr<'a>)],
    tag: &str,
    b: &mut Build<'a, '_>,
) -> Nested<'a> {
    let touches = Mentions::of_stmt(s).symbols().any(|m| varying.contains(&m));
    let Some(found) = resolved(s, b.interner) else {
        let branched = match s {
            Stmt::If {
                then_block,
                else_block,
                ..
            } => holds_supertask(then_block, b.interner) || else_block.is_some_and(|e| holds_supertask(e, b.interner)),
            _ => false,
        };
        return if branched && touches {
            Nested::Refused("a supertask runs in a branch of its pass".to_string())
        } else {
            Nested::Not
        };
    };
    if !touches {
        return Nested::Not;
    }
    if let [Stmt::Fail { reason }] = found.otherwise {
        let why = match reason {
            Expr::Literal(Literal::Text(why)) => b.interner.resolve(*why),
            _ => "",
        };
        return Nested::Refused(format!(
            "the supertask in each of its passes has no exact answer: {}",
            why.strip_prefix(NO_ANSWER).unwrap_or(why)
        ));
    }
    if takes_rule(found.otherwise, b.interner) {
        return Nested::Refused("the supertask in each of its passes takes its limit rule in every pass".to_string());
    }
    let count_changes = || {
        Nested::Refused("the supertask in each of its passes counts its steps from a value its pass changes".to_string())
    };
    let Some(count) = through(found.count, known, b) else {
        return count_changes();
    };
    let Expr::Call { args, .. } = count else {
        return count_changes();
    };
    if !args.iter().all(|a| steady(a, varying, b.interner)) {
        return count_changes();
    }
    let inner = match super::scope::distinct(&[found.cond], found.body, b) {
        Ok(Some((renamed, _))) => renamed,
        Ok(None) => found.body,
        Err(why) => return Nested::Refused(format!("the supertask in each of its passes cannot be read: {why}")),
    };
    let mut sets = Vec::new();
    all_sets(inner, &mut sets);
    let mut reads_changing: Vec<Symbol> = Vec::new();
    for m in Mentions::of_stmt(&Stmt::While {
        cond: found.cond,
        body: inner,
        decreasing: None,
    })
    .symbols()
    {
        if varying.contains(&m) && !sets.contains(&m) && !reads_changing.contains(&m) {
            reads_changing.push(m);
        }
    }
    reads_changing.sort_by(|x, y| b.interner.resolve(*x).cmp(b.interner.resolve(*y)));
    let names: Vec<String> = reads_changing.iter().map(|m| spoken(b.interner, *m)).collect();
    let no_polynomial = || Nested::Refused(no_polynomial_map(&names));
    let deeper = compose_nested(inner, varying, usize::MAX, tag, b);
    let pass: Vec<Stmt<'a>> = deeper.pass.unwrap_or_else(|| inner.to_vec());
    let Some(ahead) = super::stage::ahead(&pass, varying, b.interner) else {
        return no_polynomial();
    };
    let (mut once, mut rest) = (Vec::new(), Vec::new());
    for (s, ahead) in pass.into_iter().zip(ahead) {
        if ahead {
            once.push(s);
        } else {
            rest.push(s);
        }
    }
    let (mut sets, mut locals, mut read) = (Vec::new(), Vec::new(), Vec::new());
    if assignments(&rest, &mut sets, &mut locals, &mut read).is_none() {
        return match deeper.refused {
            Some(why) => Nested::Refused(why),
            None => no_polynomial(),
        };
    }
    let mut state: Vec<Symbol> = Vec::new();
    for v in sets {
        if !locals.contains(&v) && !state.contains(&v) {
            state.push(v);
        }
    }
    let mut extended = state.clone();
    for r in read {
        if varying.contains(&r) && !extended.contains(&r) {
            extended.push(r);
        }
    }
    let rest = b.block(rest);
    let Some(linear) = update_forms(rest, &extended, b).and_then(|forms| linearized(&forms, b)) else {
        return no_polynomial();
    };
    let fixed: Vec<Symbol> = varying.iter().chain(&extended).copied().collect();
    if !linear.rows.iter().flatten().all(|e| invariant(e, &fixed)) {
        return no_polynomial();
    }
    let width = linear.basis.len() + 1;
    let v = linear.vector(&extended, b);
    let matrix = b.list(linear.rows.into_iter().map(|r| b.list(r)).collect());
    let mut out = vec![b.let_(found.n, count)];
    let mut columns = Vec::with_capacity(width);
    for k in 0..width {
        let column = b.fresh(&format!("column{tag}{k}"));
        out.push(b.let_(column, unit_vector(b, width, k)));
        let power = b.call("affinePower", vec![matrix, unit_vector(b, width, k), b.ident(found.n)]);
        once.push(b.set(column, power));
        columns.push(column);
    }
    let runs = b.binary(BinaryOpKind::Gt, b.ident(found.n), b.number(0));
    out.push(b.if_(runs, once, None));
    let mut next = Vec::with_capacity(state.len());
    for i in 0..state.len() {
        let mut sum: Option<&'a Expr<'a>> = None;
        for (k, column) in columns.iter().enumerate() {
            let c = b.fresh(&format!("coefficient{tag}{i}_{k}"));
            out.push(b.let_(c, b.item(b.ident(*column), i + 1)));
            let term = match literal(v[k]) {
                Some(1) => b.ident(c),
                _ => b.binary(BinaryOpKind::Multiply, b.ident(c), v[k]),
            };
            sum = Some(match sum {
                None => term,
                Some(acc) => b.binary(BinaryOpKind::Add, acc, term),
            });
        }
        let value = b.fresh(&format!("next{tag}{i}"));
        out.push(b.let_(value, sum.expect("an affine map has a constant column")));
        next.push(value);
    }
    for (var, value) in state.iter().zip(next) {
        out.push(b.set(*var, b.ident(value)));
    }
    Nested::Map(out)
}

/// Whether `e` computes the same value wherever the names `varying` change: it only computes, over
/// values, and reads none of them.
fn steady(e: &Expr<'_>, varying: &[Symbol], interner: &logicaffeine_base::Interner) -> bool {
    super::stage::pure(e, interner) && !Mentions::of_expr(e).symbols().any(|m| varying.contains(&m))
}

/// `[0, …, 1, …, 0]`, the `k`-th of `width` unit vectors.
fn unit_vector<'a>(b: &Build<'a, '_>, width: usize, k: usize) -> &'a Expr<'a> {
    b.list((0..width).map(|j| b.number(i64::from(j == k))).collect())
}

/// Whether a resolved supertask's end state takes a limit rule: `Set x to limitOf(x, rule)`.
fn takes_rule(block: &[Stmt<'_>], interner: &logicaffeine_base::Interner) -> bool {
    block.iter().any(|s| {
        matches!(s, Stmt::Set { value: Expr::Call { function, .. }, .. } if interner.resolve(*function) == "limitOf")
    })
}

/// Why a nested supertask that reads `names`, which the passes change, has no map the pass can carry.
fn no_polynomial_map(names: &[String]) -> String {
    match names {
        [] => "the supertask in each of its passes starts each pass from what its passes change, and its end \
               state is no polynomial in where it starts"
            .to_string(),
        [one] => format!(
            "the supertask in each of its passes reads `{one}`, which its passes change, and its end state is no \
             polynomial in it"
        ),
        [init @ .., last] => format!(
            "the supertask in each of its passes reads {} and `{last}`, which its passes change, and its end \
             state is no polynomial in them",
            init.iter().map(|n| format!("`{n}`")).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// A name as the program spells it: a binding the engine renamed apart reads as the name it renamed.
fn spoken(interner: &logicaffeine_base::Interner, s: Symbol) -> String {
    let mut name = interner.resolve(s);
    while let Some((serial, inner)) = name.strip_prefix("__supertask").and_then(|rest| rest.split_once("_pass_")) {
        if serial.is_empty() || !serial.bytes().all(|c| c.is_ascii_digit()) {
            break;
        }
        name = inner;
    }
    name.to_string()
}

/// `e` with each name the pass bound to a value it has not changed since read as that value.
fn through<'a>(e: &'a Expr<'a>, known: &[(Symbol, &'a Expr<'a>)], b: &Build<'a, '_>) -> Option<&'a Expr<'a>> {
    if !Mentions::of_expr(e).symbols().any(|m| known.iter().any(|(k, _)| *k == m)) {
        return Some(e);
    }
    Some(match e {
        Expr::Identifier(s) => known.iter().rev().find(|(k, _)| k == s).map_or(e, |(_, v)| *v),
        Expr::Literal(_) => e,
        Expr::BinaryOp { op, left, right } => b.binary(*op, through(left, known, b)?, through(right, known, b)?),
        Expr::Not { operand } => b.not(through(operand, known, b)?),
        Expr::Call { function, args } => {
            let args = args.iter().map(|a| through(a, known, b)).collect::<Option<Vec<_>>>()?;
            b.expr(Expr::Call {
                function: *function,
                args,
            })
        }
        _ => return None,
    })
}

/// The flow strategy: the state after `N` Euler steps of `x' = f(x)`, through `polynomialFlow`. A
/// clock no other update reads leaves the flow and advances by `N` steps exactly.
fn polynomial_flow<'a>(
    clock: &Clock<'a>,
    forms: &[Form<'a>],
    state: &[Symbol],
    n: Symbol,
    b: &mut Build<'a, '_>,
) -> Vec<Stmt<'a>> {
    let at = state.iter().position(|s| *s == clock.var);
    let clock_read = at.is_some_and(|c| {
        forms
            .iter()
            .enumerate()
            .any(|(i, f)| i != c && f.terms.iter().any(|(e, _)| e[c] > 0))
    });
    let kept: Vec<usize> = (0..state.len()).filter(|i| clock_read || Some(*i) != at).collect();
    let field = b.list(
        kept.iter()
            .map(|&i| {
                b.list(
                    forms[i]
                        .terms
                        .iter()
                        .map(|(exponents, coefficient)| {
                            let mut entry = vec![*coefficient];
                            entry.extend(kept.iter().map(|&j| b.number(i64::from(exponents[j]))));
                            b.list(entry)
                        })
                        .collect(),
                )
            })
            .collect(),
    );
    let vector = b.list(kept.iter().map(|&i| b.ident(state[i])).collect());
    let x = b.fresh("x");
    let end = b.call("polynomialFlow", vec![field, vector, b.ident(n)]);
    let mut out = vec![b.let_(x, end)];
    for (k, &i) in kept.iter().enumerate() {
        out.push(b.set(state[i], b.item(b.ident(x), k + 1)));
    }
    if !clock_read {
        let travelled = b.binary(BinaryOpKind::Multiply, b.ident(n), clock.step);
        out.push(b.set(clock.var, b.binary(BinaryOpKind::Add, b.ident(clock.var), travelled)));
    }
    out
}

/// The cycle strategy: follow the state, without the clock, until it repeats.
fn cycle<'a>(
    clock: &Clock<'a>,
    body: &'a [Stmt<'a>],
    state: &[Symbol],
    read: &[Symbol],
    n: Symbol,
    b: &mut Build<'a, '_>,
) -> Result<Vec<Stmt<'a>>, String> {
    if read.contains(&clock.var) {
        return Err("its steps read the clock, so its state never repeats".to_string());
    }
    let cycled: Vec<Symbol> = state.iter().copied().filter(|s| *s != clock.var).collect();
    if cycled.is_empty() {
        return Err("it changes nothing but its clock".to_string());
    }
    let (states, current, steps, x) = (b.fresh("states"), b.fresh("state"), b.fresh("steps"), b.fresh("x"));
    let snapshot = |b: &Build<'a, '_>| b.list(cycled.iter().map(|s| b.ident(*s)).collect());
    let mut walk: Vec<Stmt<'a>> = Vec::new();
    let at_budget = b.binary(BinaryOpKind::Eq, b.ident(steps), b.number(CYCLE_BUDGET));
    let give_up = b.fail(&format!(
        "the supertask's state did not repeat within {} steps, so its end state is not read off a cycle",
        thousands(CYCLE_BUDGET)
    ));
    walk.push(b.if_(at_budget, vec![give_up], None));
    let size = b.call("stateSize", vec![b.ident(current)]);
    let too_big = b.binary(BinaryOpKind::Gt, size, b.number(STATE_BUDGET_BITS));
    let grown = b.fail(
        "the supertask's state grows without repeating, so its end state is not read off a cycle",
    );
    walk.push(b.if_(too_big, vec![grown], None));
    walk.push(b.push(b.ident(current), b.ident(states)));
    for (i, s) in body.iter().enumerate() {
        if i != clock.step_at {
            walk.push(s.clone());
        }
    }
    walk.push(b.set(current, snapshot(b)));
    walk.push(b.set(steps, b.binary(BinaryOpKind::Add, b.ident(steps), b.number(1))));
    let seen = b.contains(b.ident(states), b.ident(current));
    let mut out = vec![
        b.let_(states, b.list(Vec::new())),
        b.let_(current, snapshot(b)),
        b.let_(steps, b.number(0)),
        b.while_(b.not(seen), walk),
    ];
    let at = b.call("cycleAt", vec![b.ident(states), b.ident(current), b.ident(n)]);
    out.push(b.let_(x, at));
    for (i, s) in cycled.iter().enumerate() {
        out.push(b.set(*s, b.item(b.ident(x), i + 1)));
    }
    let travelled = b.binary(BinaryOpKind::Multiply, b.ident(n), clock.step);
    out.push(b.set(clock.var, b.binary(BinaryOpKind::Add, b.ident(clock.var), travelled)));
    Ok(out)
}

fn thousands(n: i64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}
