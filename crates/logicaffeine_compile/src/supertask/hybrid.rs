//! Loops whose state flows between jumps: `While t < T:` stepping its clock by an infinitesimal,
//! with Euler steps of a polynomial field and `If guard: …` statements that fire at isolated steps
//! — a bouncing ball, a thermostat, a switch thrown at a set time. The engine encodes the body for
//! `hybridFlow`, which runs the ideal hybrid system the loop follows to within an infinitesimal
//! (see `logicaffeine_base::hybrid`), and writes, in the loop's place:
//!
//! ```text
//! Let x be hybridFlow([body, jumps], [h, v, t, …], N).
//! Set h to item 1 of x.
//! …
//! Set t to t + N·dt.
//! ```
//!
//! The body is encoded in its order — each run of flow statements as one polynomial update, each
//! jump as the check of its guard — so the builtin knows which flow a guard sees before it is
//! checked. A guard is comparisons of polynomials in the state, combined with `and`, `or` and
//! `not`; a jump's statements set values to polynomials in the state, or to values that read none
//! of it, under conditions of the same kind or asking whether a value is infinitesimal or limited.

use logicaffeine_base::Symbol;

use super::build::Build;
use super::clock::{combine, polynomial, reads, update_forms, Clock, Form};
use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt};

/// What the engine writes for a hybrid loop: its statements, and the values they set.
pub(super) struct Resolved<'a> {
    pub(super) stmts: Vec<Stmt<'a>>,
    pub(super) written: Vec<Symbol>,
}

/// The statements that end a hybrid loop after `n` steps; `None` when the body is not one — it
/// has no jump, or nothing in it flows and nothing reads the clock, which the cycle strategy
/// follows.
pub(super) fn resolve<'a>(
    clock: &Clock<'a>,
    body: &'a [Stmt<'a>],
    n: Symbol,
    b: &mut Build<'a, '_>,
) -> Option<Result<Resolved<'a>, String>> {
    if !body.iter().any(|s| matches!(s, Stmt::If { .. })) {
        return None;
    }
    let flows = body
        .iter()
        .enumerate()
        .any(|(i, s)| i != clock.step_at && matches!(s, Stmt::Set { .. } | Stmt::Let { .. }));
    let mut read = Vec::new();
    let reads_clock = body
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != clock.step_at)
        .any(|(_, s)| statement_reads(s, &mut read) && read.contains(&clock.var));
    if !flows && !reads_clock {
        return None;
    }
    Some(encode(clock, body, n, b))
}

/// Every name a statement reads, at any depth; `false` when it holds an expression the engine
/// does not read through.
fn statement_reads(s: &Stmt<'_>, out: &mut Vec<Symbol>) -> bool {
    match s {
        Stmt::Set { value, .. } | Stmt::Let { value, .. } => expr_reads(value, out),
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => {
            let mut ok = expr_reads(cond, out);
            for s in then_block.iter().chain(else_block.iter().flat_map(|b| b.iter())) {
                ok &= statement_reads(s, out);
            }
            ok
        }
        _ => false,
    }
}

fn expr_reads(e: &Expr<'_>, out: &mut Vec<Symbol>) -> bool {
    match e {
        Expr::Call { args, .. } => args.iter().all(|a| expr_reads(a, out)),
        _ => reads(e, out).is_some(),
    }
}

/// Every name the body sets, in the order it first sets them, and every name a jump binds.
fn written(body: &[Stmt<'_>], sets: &mut Vec<Symbol>, bound: &mut Vec<Symbol>, in_jump: bool) {
    for s in body {
        match s {
            Stmt::Set { target, .. } => {
                if !sets.contains(target) {
                    sets.push(*target);
                }
            }
            Stmt::Let { var, .. } if in_jump => {
                if !bound.contains(var) {
                    bound.push(*var);
                }
            }
            Stmt::If {
                then_block,
                else_block,
                ..
            } => {
                written(then_block, sets, bound, true);
                if let Some(b) = else_block {
                    written(b, sets, bound, true);
                }
            }
            _ => {}
        }
    }
}

struct Encoder<'a, 's> {
    slots: &'s [Symbol],
    env: Vec<(Symbol, Form<'a>)>,
}

impl<'a, 's> Encoder<'a, 's> {
    fn slot(&self, s: Symbol) -> Option<usize> {
        self.slots.iter().position(|x| *x == s)
    }

    fn form(&self, e: &'a Expr<'a>, b: &Build<'a, '_>) -> Option<Form<'a>> {
        polynomial(e, self.slots, &self.env, b)
    }

    fn poly(&self, form: &Form<'a>, b: &Build<'a, '_>) -> &'a Expr<'a> {
        b.list(
            form.terms
                .iter()
                .map(|(exponents, coefficient)| {
                    let mut entry = vec![*coefficient];
                    entry.extend(exponents.iter().map(|e| b.number(i64::from(*e))));
                    b.list(entry)
                })
                .collect(),
        )
    }

    fn tagged(&self, tag: &str, mut parts: Vec<&'a Expr<'a>>, b: &mut Build<'a, '_>) -> &'a Expr<'a> {
        let mut items = vec![b.text(tag)];
        items.append(&mut parts);
        b.list(items)
    }

    fn cond(&self, e: &'a Expr<'a>, b: &mut Build<'a, '_>) -> Result<&'a Expr<'a>, String> {
        let unsupported = || {
            "a condition of a jump in the loop is not built from comparisons of polynomials in its state, `and`, `or`, `not` and asking whether a value is infinitesimal or limited"
                .to_string()
        };
        match e {
            Expr::BinaryOp { op, left, right } => {
                let rel = match op {
                    BinaryOpKind::And => {
                        let (l, r) = (self.cond(left, b)?, self.cond(right, b)?);
                        return Ok(self.tagged("all", vec![l, r], b));
                    }
                    BinaryOpKind::Or => {
                        let (l, r) = (self.cond(left, b)?, self.cond(right, b)?);
                        return Ok(self.tagged("any", vec![l, r], b));
                    }
                    BinaryOpKind::Lt => "<",
                    BinaryOpKind::LtEq => "<=",
                    BinaryOpKind::Gt => ">",
                    BinaryOpKind::GtEq => ">=",
                    BinaryOpKind::Eq => "=",
                    BinaryOpKind::NotEq => "!=",
                    _ => return Err(unsupported()),
                };
                let (l, r) = (self.form(left, b), self.form(right, b));
                let (Some(l), Some(r)) = (l, r) else {
                    return Err(unsupported());
                };
                let difference = combine(b, &l, &r, true);
                let rel = b.text(rel);
                let poly = self.poly(&difference, b);
                Ok(self.tagged("compare", vec![rel, poly], b))
            }
            Expr::Not { operand } => {
                let inner = self.cond(operand, b)?;
                Ok(self.tagged("not", vec![inner], b))
            }
            Expr::Call { function, args } => {
                let name = b.interner.resolve(*function).to_string();
                let (tag, operand) = match (name.as_str(), args.as_slice()) {
                    ("isInfinitesimal", [x]) => ("infinitesimal", self.form(x, b)),
                    ("isLimited", [x]) => ("limited", self.form(x, b)),
                    ("infinitelyClose", [x, y]) => (
                        "infinitesimal",
                        self.form(x, b)
                            .zip(self.form(y, b))
                            .map(|(x, y)| combine(b, &x, &y, true)),
                    ),
                    _ => return Err(unsupported()),
                };
                let operand = operand.ok_or_else(unsupported)?;
                let poly = self.poly(&operand, b);
                Ok(self.tagged(tag, vec![poly], b))
            }
            Expr::Identifier(s) => Ok(match self.slot(*s) {
                Some(i) => {
                    let i = b.number(i as i64);
                    self.tagged("truth", vec![i], b)
                }
                None => self.tagged("constant", vec![e], b),
            }),
            Expr::Literal(Literal::Boolean(_)) => Ok(self.tagged("constant", vec![e], b)),
            _ => Err(unsupported()),
        }
    }

    fn block(&self, stmts: &'a [Stmt<'a>], b: &mut Build<'a, '_>) -> Result<&'a Expr<'a>, String> {
        let mut out = Vec::with_capacity(stmts.len());
        for s in stmts {
            out.push(match s {
                Stmt::Set { target, value } | Stmt::Let { var: target, value, .. } => {
                    let i = self.slot(*target).expect("every name a jump sets or binds is a slot");
                    let at = b.number(i as i64);
                    match self.form(value, b) {
                        Some(form) => {
                            let poly = self.poly(&form, b);
                            self.tagged("set", vec![at, poly], b)
                        }
                        None => {
                            let mut read = Vec::new();
                            let standalone = expr_reads(value, &mut read) && read.iter().all(|r| self.slot(*r).is_none());
                            if !standalone {
                                return Err(
                                    "a jump of the loop sets a value to an expression of its state that is not a polynomial"
                                        .to_string(),
                                );
                            }
                            self.tagged("put", vec![at, value], b)
                        }
                    }
                }
                Stmt::If {
                    cond,
                    then_block,
                    else_block,
                } => {
                    let cond = self.cond(cond, b)?;
                    let then = self.block(then_block, b)?;
                    let otherwise = match else_block {
                        Some(e) => self.block(e, b)?,
                        None => b.list(Vec::new()),
                    };
                    self.tagged("if", vec![cond, then, otherwise], b)
                }
                _ => {
                    return Err(
                        "a jump of the loop does more than set values under conditions, which an end state infinitely many steps away cannot replay"
                            .to_string(),
                    )
                }
            });
        }
        Ok(b.list(out))
    }
}

fn encode<'a>(clock: &Clock<'a>, body: &'a [Stmt<'a>], n: Symbol, b: &mut Build<'a, '_>) -> Result<Resolved<'a>, String> {
    let mut sets = Vec::new();
    let mut bound = Vec::new();
    written(body, &mut sets, &mut bound, false);
    let mut flow_lets = Vec::new();
    for s in body {
        if let Stmt::Let { var, .. } = s {
            flow_lets.push(*var);
        }
    }
    if bound.iter().any(|v| sets.contains(v) || flow_lets.contains(v)) {
        return Err("a jump of the loop binds a name the loop already sets".to_string());
    }
    let mut slots = sets.clone();
    slots.extend(bound.iter().copied());
    let width = slots.len();
    for s in body {
        if let Stmt::If { .. } = s {
            let mut read = Vec::new();
            if statement_reads(s, &mut read) && read.iter().any(|r| flow_lets.contains(r)) {
                return Err("a jump of the loop reads a value its flow binds for one step".to_string());
            }
        }
    }
    let env: Vec<(Symbol, Form<'a>)> = slots
        .iter()
        .enumerate()
        .map(|(i, s)| (*s, Form::unit(width, i, b)))
        .collect();
    let encoder = Encoder { slots: &slots, env };
    let mut schedule = Vec::new();
    let mut jumps = Vec::new();
    let mut run_start: Option<usize> = None;
    let close = |from: usize, to: usize, schedule: &mut Vec<&'a Expr<'a>>, b: &mut Build<'a, '_>| -> Result<(), String> {
        let segment = &body[from..to];
        let forms = update_forms(segment, &slots, b).ok_or_else(|| {
            "the loop's flow is not a polynomial update of its state, so it is not followed as a flow".to_string()
        })?;
        let field = b.list(forms.iter().map(|f| encoder.poly(f, b)).collect());
        let flow = b.text("flow");
        schedule.push(b.list(vec![flow, field]));
        Ok(())
    };
    for (i, s) in body.iter().enumerate() {
        match s {
            Stmt::Set { .. } | Stmt::Let { .. } => {
                run_start.get_or_insert(i);
            }
            Stmt::If {
                cond,
                then_block,
                else_block,
            } => {
                if let Some(from) = run_start.take() {
                    close(from, i, &mut schedule, b)?;
                }
                if else_block.is_some_and(|e| !e.is_empty()) {
                    return Err(
                        "a jump of the loop has an `Otherwise:` that runs at every step it does not fire, which makes it part of the flow, and a flow that switches with a condition is not followed"
                            .to_string(),
                    );
                }
                let guard = encoder.cond(cond, b)?;
                let block = encoder.block(then_block, b)?;
                let check = b.text("check");
                let k = b.number(jumps.len() as i64);
                schedule.push(b.list(vec![check, k]));
                jumps.push(b.list(vec![guard, block]));
            }
            _ => {
                return Err(
                    "the loop does more than flow and jump at each step, so its end state infinitely many steps away is not followed"
                        .to_string(),
                )
            }
        }
    }
    if let Some(from) = run_start.take() {
        close(from, body.len(), &mut schedule, b)?;
    }
    let model = b.list(vec![b.list(schedule), b.list(jumps)]);
    let vector = b.list(
        slots
            .iter()
            .map(|s| if bound.contains(s) { b.number(0) } else { b.ident(*s) })
            .collect(),
    );
    let x = b.fresh("hybrid");
    let end = b.call("hybridFlow", vec![model, vector, b.ident(n)]);
    let mut stmts = vec![b.let_(x, end)];
    let mut out = Vec::new();
    for (i, s) in slots.iter().enumerate() {
        if *s != clock.var && !bound.contains(s) {
            stmts.push(b.set(*s, b.item(b.ident(x), i + 1)));
            out.push(*s);
        }
    }
    let travelled = b.binary(BinaryOpKind::Multiply, b.ident(n), clock.step);
    stmts.push(b.set(clock.var, b.binary(BinaryOpKind::Add, b.ident(clock.var), travelled)));
    out.push(clock.var);
    Ok(Resolved { stmts, written: out })
}
