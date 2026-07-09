//! Dimension-aware static analysis — rejects dimension-incoherent quantity arithmetic at COMPILE
//! time, before any code is generated. `2 meters + 1 gram` is a *type error*, not a runtime panic:
//! a length and a mass have no common dimension, so adding them cannot mean anything.
//!
//! This is a dedicated pass in the [`crate::analysis::escape::EscapeChecker`] /
//! [`crate::analysis::ownership`] mold, wired as a gate in `compile.rs`. It runs on the SAME AST the
//! interpreter and the AOT compiler share, so a dimension error is reported identically on every tier
//! before execution.
//!
//! **Soundness is conservative.** Every quantity *value* already carries its dimension at runtime;
//! this pass only adds a *static* rejection where it can PROVE incompatibility — i.e. when both
//! operands' dimensions are statically known and differ. A quantity of unknown dimension (a
//! `Quantity` function parameter, a collection element, a value from an opaque call) is treated as
//! dimension-polymorphic and deferred to the existing runtime check. So this pass never rejects a
//! correct program; it only promotes provable runtime failures to compile-time errors.

use std::collections::HashMap;

use logicaffeine_base::quantity::units;
use logicaffeine_base::Dimension;

use crate::ast::stmt::{BinaryOpKind, Expr, Literal, Stmt, TypeExpr};
use crate::intern::{Interner, Symbol};
use crate::token::Span;

/// The dimensional nature of an expression's value.
#[derive(Clone, Copy, PartialEq)]
enum QDim {
    /// A quantity whose dimension is statically known (e.g. a literal `2 meters`).
    Known(Dimension),
    /// A quantity whose dimension is not statically known (a `Quantity` parameter, a collection
    /// element, …) — dimension-polymorphic, deferred to the runtime check.
    Unknown,
    /// Not a quantity (a plain number, text, struct, …).
    NotQuantity,
}

impl QDim {
    fn is_quantity(self) -> bool {
        !matches!(self, QDim::NotQuantity)
    }
}

/// The currency nature of an expression — money's analogue of [`QDim`]. The same pass that proves
/// dimension coherence also proves currency coherence (`5 USD + 1 EUR` has no answer without a rate
/// context, exactly like `meter + gram`).
#[derive(Clone, PartialEq, Eq)]
enum CurInfo {
    /// Money whose currency is statically known (a `19.99 USD` / `money(_,"USD")` literal).
    Known(String),
    /// Money whose currency is not statically known (a `Money` parameter, a collection element).
    Unknown,
    /// Not money.
    NotMoney,
}

impl CurInfo {
    fn is_money(&self) -> bool {
        !matches!(self, CurInfo::NotMoney)
    }
}

/// A dimension coherence error, with the same shape the other analysis passes use so `compile.rs`
/// can convert it to a `ParseError` uniformly.
pub struct DimensionError {
    pub message: String,
    pub span: Span,
}

impl std::fmt::Display for DimensionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

pub struct DimensionChecker<'a> {
    /// Variables (and parameters) whose dimensional nature is known in the current scope.
    vars: HashMap<Symbol, QDim>,
    /// User function name → the dimensional nature of its declared return type, so a call to a
    /// function returning `Quantity of Area` is statically known to be an area.
    fn_returns: HashMap<Symbol, QDim>,
    /// Variables whose currency is statically known in the current scope (money's analogue of `vars`).
    cur_vars: HashMap<Symbol, CurInfo>,
    interner: &'a Interner,
}

impl<'a> DimensionChecker<'a> {
    pub fn new(interner: &'a Interner) -> Self {
        Self {
            vars: HashMap::new(),
            fn_returns: HashMap::new(),
            cur_vars: HashMap::new(),
            interner,
        }
    }

    pub fn check_program(&mut self, stmts: &[Stmt<'_>]) -> Result<(), DimensionError> {
        // Pre-pass: record every function's declared return dimension (forward references included).
        self.collect_fn_returns(stmts);
        self.check_block(stmts)
    }

    fn collect_fn_returns(&mut self, stmts: &[Stmt<'_>]) {
        for stmt in stmts {
            if let Stmt::FunctionDef { name, return_type, .. } = stmt {
                let qd = return_type.map(|t| self.dim_from_type(t)).unwrap_or(QDim::NotQuantity);
                self.fn_returns.insert(*name, qd);
            }
        }
    }

    /// The dimensional nature declared by a type annotation: `Quantity of Length` → `Known(L)`,
    /// bare `Quantity` → `Unknown` (dimension-polymorphic), anything else → `NotQuantity`.
    fn dim_from_type(&self, ty: &TypeExpr<'_>) -> QDim {
        match ty {
            TypeExpr::Generic { base, params }
                if self.interner.resolve(*base) == "Quantity" && params.len() == 1 =>
            {
                match &params[0] {
                    TypeExpr::Primitive(s) | TypeExpr::Named(s) => {
                        match Dimension::by_name(self.interner.resolve(*s)) {
                            Some(d) => QDim::Known(d),
                            None => QDim::Unknown,
                        }
                    }
                    _ => QDim::Unknown,
                }
            }
            TypeExpr::Primitive(s) | TypeExpr::Named(s)
                if self.interner.resolve(*s) == "Quantity" =>
            {
                QDim::Unknown
            }
            _ => QDim::NotQuantity,
        }
    }

    fn check_block(&mut self, stmts: &[Stmt<'_>]) -> Result<(), DimensionError> {
        for stmt in stmts {
            self.check_stmt(stmt)?;
        }
        Ok(())
    }

    fn check_stmt(&mut self, stmt: &Stmt<'_>) -> Result<(), DimensionError> {
        match stmt {
            Stmt::Let { var, value, ty, .. } => {
                let inferred = self.infer(value)?;
                // A `Let d: Quantity of Length be …` declares the dimension authoritatively.
                let declared = ty.map(|t| self.dim_from_type(t));
                let d = match declared {
                    Some(QDim::Known(k)) => QDim::Known(k),
                    Some(QDim::Unknown) if !inferred.is_quantity() => QDim::Unknown,
                    _ => inferred,
                };
                self.vars.insert(*var, d);
                // Track the binding's currency too, so `Let p be 5 USD. ... p + 1 EUR.` is caught.
                let c = self.currency_of(value);
                self.cur_vars.insert(*var, c);
            }
            Stmt::Set { value, .. } => {
                self.infer(value)?;
            }
            Stmt::SetField { object, value, .. } => {
                self.infer(object)?;
                self.infer(value)?;
            }
            Stmt::If { cond, then_block, else_block } => {
                self.infer(cond)?;
                self.check_block(then_block)?;
                if let Some(e) = else_block {
                    self.check_block(e)?;
                }
            }
            Stmt::While { cond, body, .. } => {
                self.infer(cond)?;
                self.check_block(body)?;
            }
            Stmt::Repeat { iterable, body, .. } => {
                self.infer(iterable)?;
                self.check_block(body)?;
            }
            Stmt::Return { value: Some(e) } => {
                self.infer(e)?;
            }
            Stmt::Show { object, .. } => {
                self.infer(object)?;
            }
            Stmt::RuntimeAssert { condition, .. } => {
                self.infer(condition)?;
            }
            Stmt::Call { args, .. } => {
                for a in args {
                    self.infer(a)?;
                }
            }
            Stmt::FunctionDef { params, body, .. } => {
                // A function body is a fresh scope: a `Quantity` parameter is dimension-polymorphic
                // (unknown), every other parameter is a non-quantity for our purposes.
                let saved = self.vars.clone();
                let saved_cur = self.cur_vars.clone();
                for (name, ty) in params {
                    let qd = self.dim_from_type(ty);
                    if qd.is_quantity() {
                        // `Quantity of Length` → Known(L); bare `Quantity` → Unknown (polymorphic).
                        self.vars.insert(*name, qd);
                    }
                }
                self.check_block(body)?;
                self.vars = saved;
                self.cur_vars = saved_cur;
            }
            _ => {}
        }
        Ok(())
    }

    /// The dimension named by a unit-string argument (`Literal::Text`), if it resolves; an
    /// unresolved unit is left `Unknown` (the construction itself errors at runtime).
    fn unit_dim(&self, expr: &Expr<'_>) -> QDim {
        if let Expr::Literal(Literal::Text(sym)) = expr {
            if let Some(unit) = units::by_name(self.interner.resolve(*sym)) {
                return QDim::Known(unit.dimension);
            }
        }
        QDim::Unknown
    }

    /// The currency of an expression, when statically known. Mirrors [`Self::infer`] but for money:
    /// a `money(_, "USD")` literal is `Known("USD")`, a Let-bound money variable carries its currency,
    /// `+ −` and scaling keep it, and a `Money ÷ Money` becomes `NotMoney` (a Rational ratio).
    fn currency_of(&self, expr: &Expr<'_>) -> CurInfo {
        match expr {
            Expr::Identifier(s) => self.cur_vars.get(s).cloned().unwrap_or(CurInfo::NotMoney),
            Expr::Call { function, args } => {
                if self.interner.resolve(*function) == "money" && args.len() == 2 {
                    if let Expr::Literal(Literal::Text(code)) = args[1] {
                        return CurInfo::Known(self.interner.resolve(*code).to_ascii_uppercase());
                    }
                    return CurInfo::Unknown;
                }
                CurInfo::NotMoney
            }
            Expr::BinaryOp { op, left, right } => match op {
                BinaryOpKind::Add | BinaryOpKind::Subtract => {
                    let (l, r) = (self.currency_of(left), self.currency_of(right));
                    if let CurInfo::Known(_) = l {
                        l
                    } else {
                        r
                    }
                }
                // Scaling money by a number keeps the currency; `Money ÷ Money` is a ratio (not money).
                BinaryOpKind::Multiply => {
                    let (l, r) = (self.currency_of(left), self.currency_of(right));
                    if matches!(l, CurInfo::Known(_) | CurInfo::Unknown) {
                        l
                    } else {
                        r
                    }
                }
                BinaryOpKind::Divide => {
                    let (l, r) = (self.currency_of(left), self.currency_of(right));
                    // money ÷ money → ratio (not money); money ÷ number → money.
                    if r.is_money() {
                        CurInfo::NotMoney
                    } else {
                        l
                    }
                }
                _ => CurInfo::NotMoney,
            },
            Expr::Copy { expr } | Expr::Give { value: expr } => self.currency_of(expr),
            _ => CurInfo::NotMoney,
        }
    }

    /// Error if both operands are money of *provably different* currencies (`5 USD + 1 EUR`).
    fn check_currency_match(
        &self,
        left: &Expr<'_>,
        right: &Expr<'_>,
        verb: &str,
    ) -> Result<(), DimensionError> {
        if let (CurInfo::Known(a), CurInfo::Known(b)) = (self.currency_of(left), self.currency_of(right))
        {
            if a != b {
                return Err(DimensionError {
                    message: format!("cannot {verb} money of different currencies ({a} vs {b})"),
                    span: Span::default(),
                });
            }
        }
        Ok(())
    }

    /// Infer the dimensional nature of an expression, erroring on a statically-incoherent operation.
    fn infer(&self, expr: &Expr<'_>) -> Result<QDim, DimensionError> {
        match expr {
            Expr::Literal(_) => Ok(QDim::NotQuantity),
            Expr::Identifier(s) => Ok(self.vars.get(s).copied().unwrap_or(QDim::NotQuantity)),

            Expr::Call { function, args } => {
                for a in args {
                    self.infer(a)?;
                }
                match self.interner.resolve(*function) {
                    "quantity" | "convert" if args.len() == 2 => Ok(self.unit_dim(args[1])),
                    // A user function with a declared `Quantity of <Dim>` return is statically known;
                    // any other call may return a quantity we can't see into — polymorphic (Unknown).
                    _ => Ok(self.fn_returns.get(function).copied().unwrap_or(QDim::Unknown)),
                }
            }

            Expr::BinaryOp { op, left, right } => {
                let l = self.infer(left)?;
                let r = self.infer(right)?;
                match op {
                    // `+` / `−` require equal dimensions; provably-different dimensions are rejected.
                    BinaryOpKind::Add | BinaryOpKind::Subtract => {
                        self.check_currency_match(
                            left,
                            right,
                            if matches!(op, BinaryOpKind::Add) { "add" } else { "subtract" },
                        )?;
                        if let (QDim::Known(a), QDim::Known(b)) = (l, r) {
                            if a != b {
                                return Err(self.mismatch_err(
                                    if matches!(op, BinaryOpKind::Add) { "add" } else { "subtract" },
                                    a,
                                    b,
                                ));
                            }
                            return Ok(QDim::Known(a));
                        }
                        Ok(self.propagate(l, r))
                    }
                    // Ordering two quantities/monies of provably-different kind is meaningless.
                    BinaryOpKind::Lt | BinaryOpKind::Gt | BinaryOpKind::LtEq | BinaryOpKind::GtEq => {
                        self.check_currency_match(left, right, "compare")?;
                        if let (QDim::Known(a), QDim::Known(b)) = (l, r) {
                            if a != b {
                                return Err(self.mismatch_err("compare", a, b));
                            }
                        }
                        Ok(QDim::NotQuantity)
                    }
                    // `×` combines dimensions (Length × Length = Area); scaling by a number keeps it.
                    BinaryOpKind::Multiply => Ok(match (l, r) {
                        (QDim::Known(a), QDim::Known(b)) => QDim::Known(a.mul(b)),
                        (QDim::Known(a), QDim::NotQuantity) | (QDim::NotQuantity, QDim::Known(a)) => {
                            QDim::Known(a)
                        }
                        _ if l.is_quantity() || r.is_quantity() => QDim::Unknown,
                        _ => QDim::NotQuantity,
                    }),
                    // `÷` subtracts dimensions (Volume ÷ Area = Length); dividing by a number keeps it.
                    BinaryOpKind::Divide => Ok(match (l, r) {
                        (QDim::Known(a), QDim::Known(b)) => QDim::Known(a.div(b)),
                        (QDim::Known(a), QDim::NotQuantity) => QDim::Known(a),
                        _ if l.is_quantity() || r.is_quantity() => QDim::Unknown,
                        _ => QDim::NotQuantity,
                    }),
                    _ => Ok(QDim::NotQuantity),
                }
            }

            // Recurse into expression shapes that nest sub-expressions, so a mismatch buried inside
            // one is still caught. A collection element may be a quantity of unknown dimension.
            Expr::Not { operand } => {
                self.infer(operand)?;
                Ok(QDim::NotQuantity)
            }
            Expr::Index { collection, index } => {
                self.infer(collection)?;
                self.infer(index)?;
                Ok(QDim::Unknown)
            }
            Expr::Slice { collection, start, end } => {
                self.infer(collection)?;
                self.infer(start)?;
                self.infer(end)?;
                Ok(QDim::Unknown)
            }
            Expr::Copy { expr } | Expr::Give { value: expr } => self.infer(expr),
            Expr::Length { collection } => {
                self.infer(collection)?;
                Ok(QDim::NotQuantity)
            }
            Expr::Contains { collection, value } => {
                self.infer(collection)?;
                self.infer(value)?;
                Ok(QDim::NotQuantity)
            }
            _ => Ok(QDim::NotQuantity),
        }
    }

    /// For `+`/`−` where at least one side is dimension-unknown: carry a known dimension forward if
    /// there is one, otherwise stay quantity-but-unknown when either side is a quantity.
    fn propagate(&self, l: QDim, r: QDim) -> QDim {
        match (l, r) {
            (QDim::Known(d), _) | (_, QDim::Known(d)) => QDim::Known(d),
            _ if l.is_quantity() || r.is_quantity() => QDim::Unknown,
            _ => QDim::NotQuantity,
        }
    }

    fn mismatch_err(&self, verb: &str, a: Dimension, b: Dimension) -> DimensionError {
        DimensionError {
            message: format!("cannot {verb} quantities of different dimensions ({a} vs {b})"),
            span: Span::default(),
        }
    }
}
