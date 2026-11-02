//! The statements and expressions a rewrite emits, built in the optimizer's arenas.
//!
//! A pass that replaces a region with code of its own (a heap sort, a sieve) spells that code through these, so each
//! node is one call and the emitted program reads like the LOGOS it stands for.

use crate::arena::Arena;
use crate::ast::stmt::{BinaryOpKind, Block, Expr, Literal, MatchArm, Stmt, TypeExpr};
use crate::intern::Symbol;
use crate::optimization::Opt;
use logicaffeine_kernel::lia::{LinearExpr, Rational};

use super::affine::len_owner;

/// The arenas a rewrite builds into.
#[derive(Clone, Copy)]
pub(crate) struct Build<'a> {
    pub ea: &'a Arena<Expr<'a>>,
    pub sa: &'a Arena<Stmt<'a>>,
}

impl<'a> Build<'a> {
    pub fn id(&self, s: Symbol) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Identifier(s))
    }

    pub fn num(&self, n: i64) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Literal(Literal::Number(n)))
    }

    pub fn boolean(&self, b: bool) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Literal(Literal::Boolean(b)))
    }

    pub fn bin(&self, op: BinaryOpKind, left: &'a Expr<'a>, right: &'a Expr<'a>) -> &'a Expr<'a> {
        self.ea.alloc(Expr::BinaryOp { op, left, right })
    }

    /// `item index of seq`.
    pub fn item(&self, seq: Symbol, index: &'a Expr<'a>) -> &'a Expr<'a> {
        self.ea.alloc(Expr::Index {
            collection: self.id(seq),
            index,
        })
    }

    /// `a new Seq of <item>`.
    pub fn new_seq(&self, seq: Symbol, item: TypeExpr<'a>) -> &'a Expr<'a> {
        self.ea.alloc(Expr::New {
            type_name: seq,
            type_args: vec![item],
            init_fields: Vec::new(),
        })
    }

    /// `Set item index of seq to value.`
    pub fn store(&self, seq: Symbol, index: &'a Expr<'a>, value: &'a Expr<'a>) -> Stmt<'a> {
        Stmt::SetIndex {
            collection: self.id(seq),
            index,
            value,
        }
    }

    /// `Push value to seq.`
    pub fn push(&self, value: &'a Expr<'a>, seq: Symbol) -> Stmt<'a> {
        Stmt::Push {
            value,
            collection: self.id(seq),
        }
    }

    /// `Let [mutable] var be value.`
    pub fn let_(&self, var: Symbol, value: &'a Expr<'a>, mutable: bool) -> Stmt<'a> {
        Stmt::Let {
            var,
            ty: None,
            value,
            mutable,
        }
    }

    /// `Set target to value.`
    pub fn set(&self, target: Symbol, value: &'a Expr<'a>) -> Stmt<'a> {
        Stmt::Set { target, value }
    }

    /// `Set x to x ± 1.`
    pub fn step(&self, x: Symbol, op: BinaryOpKind) -> Stmt<'a> {
        self.set(x, self.bin(op, self.id(x), self.num(1)))
    }

    pub fn block(&self, stmts: Vec<Stmt<'a>>) -> Block<'a> {
        self.sa.alloc_slice(stmts)
    }

    pub fn while_(&self, cond: &'a Expr<'a>, body: Vec<Stmt<'a>>) -> Stmt<'a> {
        Stmt::While {
            cond,
            body: self.block(body),
            decreasing: None,
        }
    }

    pub fn if_(
        &self,
        cond: &'a Expr<'a>,
        then: Vec<Stmt<'a>>,
        otherwise: Option<Vec<Stmt<'a>>>,
    ) -> Stmt<'a> {
        Stmt::If {
            cond,
            then_block: self.block(then),
            else_block: otherwise.map(|b| self.block(b)),
        }
    }

    /// A fresh copy of a simple expression (names, number literals, arithmetic, `length of` a name), so that what a
    /// pass records against the copy's nodes never answers for the original's; `None` for anything else.
    pub fn copy(&self, e: &Expr<'a>) -> Option<&'a Expr<'a>> {
        Some(match e {
            Expr::Identifier(s) => self.id(*s),
            Expr::Literal(Literal::Number(n)) => self.num(*n),
            Expr::BinaryOp { op, left, right } => {
                self.bin(*op, self.copy(left)?, self.copy(right)?)
            }
            Expr::Length {
                collection: Expr::Identifier(s),
            } => self.ea.alloc(Expr::Length {
                collection: self.id(*s),
            }),
            _ => return None,
        })
    }

    /// The expression for an integer linear form over names and lengths ([`super::affine`]'s variables), or `None`
    /// when a coefficient is fractional or leaves i64.
    pub fn linear(&self, e: &LinearExpr) -> Option<&'a Expr<'a>> {
        let int = |r: &Rational| if r.is_integer() { r.to_i64() } else { None };
        let mut sum: Option<&'a Expr<'a>> = None;
        for (&v, c) in &e.coefficients {
            let c = int(c)?;
            if c == 0 {
                continue;
            }
            let x = match len_owner(v) {
                Some(owner) => self.ea.alloc(Expr::Length {
                    collection: self.id(owner),
                }),
                None => self.id(Symbol::from_index(usize::try_from(v).ok()?)),
            };
            let (op, term) = match c {
                1 => (BinaryOpKind::Add, x),
                -1 => (BinaryOpKind::Subtract, x),
                c if c > 0 => (
                    BinaryOpKind::Add,
                    self.bin(BinaryOpKind::Multiply, self.num(c), x),
                ),
                c => (
                    BinaryOpKind::Subtract,
                    self.bin(BinaryOpKind::Multiply, self.num(c.checked_neg()?), x),
                ),
            };
            sum = Some(match (sum, op) {
                (None, BinaryOpKind::Add) => term,
                (None, _) => self.bin(BinaryOpKind::Subtract, self.num(0), term),
                (Some(acc), op) => self.bin(op, acc, term),
            });
        }
        let k = int(&e.constant)?;
        Some(match sum {
            None => self.num(k),
            Some(acc) if k == 0 => acc,
            Some(acc) if k > 0 => self.bin(BinaryOpKind::Add, acc, self.num(k)),
            Some(acc) => self.bin(BinaryOpKind::Subtract, acc, self.num(k.checked_neg()?)),
        })
    }
}

/// `stmts` with every block offered to `try_at`, position by position: the program's top level, each function body
/// whose flags allow `opt`, each branch, and each loop and arm body. `try_at(block, at)` returns the statements that
/// replace `block[at..at + span]` and `span`, or `None` to keep `block[at]` and look inside it.
pub(crate) fn rewrite_blocks<'a>(
    stmts: &[Stmt<'a>],
    sa: &'a Arena<Stmt<'a>>,
    opt: Opt,
    try_at: &mut dyn FnMut(&[Stmt<'a>], usize) -> Option<(Vec<Stmt<'a>>, usize)>,
) -> Vec<Stmt<'a>> {
    let mut out = Vec::with_capacity(stmts.len());
    let mut at = 0;
    while at < stmts.len() {
        if let Some((replaced, span)) = try_at(stmts, at) {
            out.extend(replaced);
            at += span.max(1);
            continue;
        }
        out.push(rewrite_inside(&stmts[at], sa, opt, try_at));
        at += 1;
    }
    out
}

fn rewrite_inside<'a>(
    s: &Stmt<'a>,
    sa: &'a Arena<Stmt<'a>>,
    opt: Opt,
    try_at: &mut dyn FnMut(&[Stmt<'a>], usize) -> Option<(Vec<Stmt<'a>>, usize)>,
) -> Stmt<'a> {
    let mut nested =
        |b: Block<'a>| -> Block<'a> { sa.alloc_slice(rewrite_blocks(b, sa, opt, &mut *try_at)) };
    match s {
        Stmt::FunctionDef {
            name,
            generics,
            params,
            body,
            return_type,
            is_native,
            native_path,
            is_exported,
            export_target,
            opt_flags,
        } if opt_flags.is_on(opt) => Stmt::FunctionDef {
            name: *name,
            generics: generics.clone(),
            params: params.clone(),
            body: nested(*body),
            return_type: *return_type,
            is_native: *is_native,
            native_path: *native_path,
            is_exported: *is_exported,
            export_target: *export_target,
            opt_flags: *opt_flags,
        },
        Stmt::If {
            cond,
            then_block,
            else_block,
        } => Stmt::If {
            cond: *cond,
            then_block: nested(*then_block),
            else_block: else_block.map(|b| nested(b)),
        },
        Stmt::While {
            cond,
            body,
            decreasing,
        } => Stmt::While {
            cond: *cond,
            body: nested(*body),
            decreasing: *decreasing,
        },
        Stmt::Repeat {
            pattern,
            iterable,
            body,
        } => Stmt::Repeat {
            pattern: pattern.clone(),
            iterable: *iterable,
            body: nested(*body),
        },
        Stmt::Inspect {
            target,
            arms,
            has_otherwise,
        } => Stmt::Inspect {
            target: *target,
            arms: arms
                .iter()
                .map(|arm| MatchArm {
                    enum_name: arm.enum_name,
                    variant: arm.variant,
                    bindings: arm.bindings.clone(),
                    body: nested(arm.body),
                })
                .collect(),
            has_otherwise: *has_otherwise,
        },
        other => other.clone(),
    }
}
