//! The forms a compiled program holds numbers in, and how the forms of the values one variable takes join.
//!
//! The VM keeps each number the value it is: an integer, a big integer, a fraction, a real. A backend holds a
//! variable in one machine form, so where the values a program gives a variable are of different kinds it joins
//! them. The exact forms join upward: an integer is a big integer, and both are rationals, since a rational held by a
//! compiled program answers every operation on a whole value as the integer it equals (the VM never holds a whole
//! rational at all). An integer and a real join to the real only where the program cannot tell the integer's phase
//! from the real it widens to ([`integer_phase_hidden`]); an exact form beside a real has no machine form. The
//! bytecode backends join a variable's definitions ([`crate::vm::numbers`]); the AST backends join the values a name is
//! bound to in its scope.

use logicaffeine_language::ast_depth::{walk_nodes, Visited, Walk};

use crate::ast::stmt::{BinaryOpKind, Expr, Stmt};
use crate::intern::Symbol;

/// A number form a backend holds a value in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Number {
    Int,
    BigInt,
    Rational,
    Real,
}

/// The form of a value a number join reads: a number, or a list whose items are numbers of one form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Form {
    Number(Number),
    List(Number),
}

/// The bound below which a double holds every integer exactly and orders it exactly against any other.
pub(crate) const EXACT_IN_A_DOUBLE: u64 = 1 << 53;

/// The exact join of two number forms: the wider of two exact forms; `None` for an exact form beside a real.
pub(crate) fn exact_join(a: Number, b: Number) -> Option<Number> {
    match (a, b) {
        _ if a == b => Some(a),
        (Number::Real, _) | (_, Number::Real) => None,
        _ => Some(a.max(b)),
    }
}

/// Whether an integer a double holds exactly is what `value` is: an integer literal within 2^53.
pub(crate) fn exact_in_a_double(value: &Expr) -> bool {
    matches!(value, Expr::Literal(crate::ast::stmt::Literal::Number(n)) if n.unsigned_abs() < EXACT_IN_A_DOUBLE)
}

/// Whether every read of `var` in `body` answers an integer as it answers the real it widens to: `+ - *` beside a
/// real (`real` says which operands are), an order or equality test, a `Show` (`show` names the `show` recipient).
pub(crate) fn integer_phase_hidden<'a>(
    body: &[Stmt<'a>],
    var: Symbol,
    show: Option<Symbol>,
    real: impl Fn(&Expr<'a>) -> bool,
) -> bool {
    let is_var = |e: &Expr| matches!(e, Expr::Identifier(s) if *s == var);
    let (mut reads, mut hidden) = (0usize, 0usize);
    walk_nodes(body, |node| {
        match node {
            Visited::Expr(Expr::Identifier(s)) if *s == var => reads += 1,
            Visited::Expr(Expr::BinaryOp { op, left, right }) => match op {
                BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply => {
                    hidden += usize::from(is_var(left) && !is_var(right) && real(right));
                    hidden += usize::from(is_var(right) && !is_var(left) && real(left));
                }
                BinaryOpKind::Eq
                | BinaryOpKind::NotEq
                | BinaryOpKind::Lt
                | BinaryOpKind::LtEq
                | BinaryOpKind::Gt
                | BinaryOpKind::GtEq => {
                    hidden += usize::from(is_var(left)) + usize::from(is_var(right));
                }
                _ => {}
            },
            Visited::Stmt(Stmt::Show { object, recipient })
                if is_var(object) && matches!(recipient, Expr::Identifier(r) if Some(*r) == show) =>
            {
                hidden += 1
            }
            _ => {}
        }
        Walk::Into
    });
    reads == hidden
}
