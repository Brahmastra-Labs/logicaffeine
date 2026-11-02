//! Building the statements the engine writes: ordinary nodes in the pass's arenas, and the fresh
//! names they bind.

use logicaffeine_base::{Interner, Symbol};

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Pattern, Stmt};
use crate::ast_map::Arenas;

pub(super) struct Build<'a, 'i> {
    pub arenas: Arenas<'a>,
    pub interner: &'i mut Interner,
    /// Which resolved supertask this is, so every name it binds is its own.
    pub serial: usize,
}

impl<'a, 'i> Build<'a, 'i> {
    /// A name no program spells, for one resolved supertask.
    pub fn fresh(&mut self, tag: &str) -> Symbol {
        self.interner.intern(&format!("__supertask{}_{tag}", self.serial))
    }

    pub fn expr(&self, e: Expr<'a>) -> &'a Expr<'a> {
        self.arenas.exprs.alloc(e)
    }

    pub fn ident(&self, s: Symbol) -> &'a Expr<'a> {
        self.expr(Expr::Identifier(s))
    }

    pub fn number(&self, n: i64) -> &'a Expr<'a> {
        self.expr(Expr::Literal(Literal::Number(n)))
    }

    pub fn boolean(&self, b: bool) -> &'a Expr<'a> {
        self.expr(Expr::Literal(Literal::Boolean(b)))
    }

    pub fn text(&mut self, t: &str) -> &'a Expr<'a> {
        let sym = self.interner.intern(t);
        self.expr(Expr::Literal(Literal::Text(sym)))
    }

    pub fn call(&mut self, name: &str, args: Vec<&'a Expr<'a>>) -> &'a Expr<'a> {
        let function = self.interner.intern(name);
        self.expr(Expr::Call { function, args })
    }

    pub fn binary(&self, op: BinaryOpKind, left: &'a Expr<'a>, right: &'a Expr<'a>) -> &'a Expr<'a> {
        self.expr(Expr::BinaryOp { op, left, right })
    }

    pub fn not(&self, operand: &'a Expr<'a>) -> &'a Expr<'a> {
        self.expr(Expr::Not { operand })
    }

    pub fn list(&self, items: Vec<&'a Expr<'a>>) -> &'a Expr<'a> {
        self.expr(Expr::List(items))
    }

    /// `item i of collection`, counted from 1.
    pub fn item(&self, collection: &'a Expr<'a>, i: usize) -> &'a Expr<'a> {
        let index = self.number(i as i64);
        self.expr(Expr::Index { collection, index })
    }

    pub fn length(&self, collection: &'a Expr<'a>) -> &'a Expr<'a> {
        self.expr(Expr::Length { collection })
    }

    pub fn contains(&self, collection: &'a Expr<'a>, value: &'a Expr<'a>) -> &'a Expr<'a> {
        self.expr(Expr::Contains { collection, value })
    }

    pub fn block(&self, stmts: Vec<Stmt<'a>>) -> &'a [Stmt<'a>] {
        self.arenas.stmts.alloc_slice(stmts)
    }

    pub fn let_(&self, var: Symbol, value: &'a Expr<'a>) -> Stmt<'a> {
        Stmt::Let {
            var,
            ty: None,
            value,
            mutable: true,
        }
    }

    pub fn set(&self, target: Symbol, value: &'a Expr<'a>) -> Stmt<'a> {
        Stmt::Set { target, value }
    }

    pub fn if_(&self, cond: &'a Expr<'a>, then: Vec<Stmt<'a>>, otherwise: Option<Vec<Stmt<'a>>>) -> Stmt<'a> {
        Stmt::If {
            cond,
            then_block: self.block(then),
            else_block: otherwise.map(|b| self.block(b)),
        }
    }

    pub fn while_(&self, cond: &'a Expr<'a>, body: Vec<Stmt<'a>>) -> Stmt<'a> {
        Stmt::While {
            cond,
            body: self.block(body),
            decreasing: None,
        }
    }

    pub fn repeat(&self, var: Symbol, iterable: &'a Expr<'a>, body: &'a [Stmt<'a>]) -> Stmt<'a> {
        Stmt::Repeat {
            pattern: Pattern::Identifier(var),
            iterable,
            body,
        }
    }

    /// `Repeat for var from 0 to last:`.
    pub fn repeat_upto(&self, var: Symbol, last: i64, body: &'a [Stmt<'a>]) -> Stmt<'a> {
        let range = self.expr(Expr::Range {
            start: self.number(0),
            end: self.number(last),
        });
        self.repeat(var, range, body)
    }

    pub fn push(&self, value: &'a Expr<'a>, collection: &'a Expr<'a>) -> Stmt<'a> {
        Stmt::Push { value, collection }
    }

    pub fn fail(&mut self, reason: &str) -> Stmt<'a> {
        let reason = self.text(reason);
        Stmt::Fail { reason }
    }

    pub fn splice(&self, stmts: Vec<Stmt<'a>>) -> Stmt<'a> {
        Stmt::Splice {
            body: self.block(stmts),
        }
    }
}

// ── invariant arithmetic ──────────────────────────────────────────────────────────────────────
//
// Coefficients the engine derives are expressions over the loop's invariants, `None` standing for
// zero; whole-number literals fold as they meet, so the statements it writes stay readable.

pub(super) fn literal(e: &Expr<'_>) -> Option<i64> {
    match e {
        Expr::Literal(Literal::Number(n)) => Some(*n),
        _ => None,
    }
}

pub(super) fn add<'a>(b: &Build<'a, '_>, x: Option<&'a Expr<'a>>, y: Option<&'a Expr<'a>>, negate_y: bool) -> Option<&'a Expr<'a>> {
    match (x, y) {
        (x, None) => x,
        (None, Some(y)) if negate_y => Some(match literal(y).and_then(i64::checked_neg) {
            Some(n) => b.number(n),
            None => b.binary(BinaryOpKind::Subtract, b.number(0), y),
        }),
        (None, Some(y)) => Some(y),
        (Some(x), Some(y)) => Some(match (literal(x), literal(y)) {
            (Some(m), Some(n)) if negate_y && m.checked_sub(n).is_some() => b.number(m - n),
            (Some(m), Some(n)) if !negate_y && m.checked_add(n).is_some() => b.number(m + n),
            _ => b.binary(
                if negate_y {
                    BinaryOpKind::Subtract
                } else {
                    BinaryOpKind::Add
                },
                x,
                y,
            ),
        }),
    }
}

pub(super) fn scale<'a>(b: &Build<'a, '_>, x: Option<&'a Expr<'a>>, by: &'a Expr<'a>, op: BinaryOpKind) -> Option<&'a Expr<'a>> {
    let x = x?;
    if op == BinaryOpKind::Multiply {
        match (literal(x), literal(by)) {
            (Some(0), _) | (_, Some(0)) => return None,
            (Some(1), _) => return Some(by),
            (_, Some(1)) => return Some(x),
            (Some(m), Some(n)) => {
                if let Some(p) = m.checked_mul(n) {
                    return Some(b.number(p));
                }
            }
            _ => {}
        }
    }
    Some(b.binary(op, x, by))
}

