//! How the self-interpreter applies each binary operator. For an operator it applies itself, the ways it can be
//! applied: each overload's operand types and its answer's type, as LOGOS writes them, tried in order, the cases an
//! overload stops on before computing, and the report an application no overload takes stops with, in the tree-walker's
//! words. The self-interpreter's operator dispatch is generated from this table (`optimize::core_operators`), so an
//! operator it applies is the language's own, written in LOGOS on the values its operands carry.
//!
//! An operand typed `Any` takes a value of any type, as the text `Show` prints it: `"a" + 2.5` is `"a2.5"`. A guard is a
//! LOGOS condition on the operands, `{left}` and `{right}`. A refusal is the first of its cases whose operand type names
//! match (`*` matches any), and names the type of operand `n` as `{n}`.

use crate::ast::stmt::BinaryOpKind;

/// One way of applying an operator: the types of its two operands, of its answer, and the cases it stops on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpOverload {
    pub params: [&'static str; 2],
    pub answer: &'static str,
    pub guards: &'static [Guard],
}

/// A case an overload stops on before computing: a condition on its operands, and the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guard {
    pub when: &'static str,
    pub report: &'static str,
}

/// One case of an operator's refusal: the type names of the operands it is for, `*` for any, and its report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub left: &'static str,
    pub right: &'static str,
    pub report: &'static str,
}

/// How the self-interpreter applies one operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelfApplied {
    /// The operator itself, written as the program writes it ([`surface_text`]), applied to its operands' values by the
    /// first overload that takes them; an application no overload takes, and that no runtime value family of the
    /// self-interpreter's own answers, stops with `refusal`.
    Overloads {
        overloads: &'static [OpOverload],
        refusal: &'static [Refusal],
    },
    /// A definition of the self-interpreter's own (`coreOp…` in `core_interp.logos`): equality over every value, the
    /// logical words over every value's truth.
    Defined,
    /// Not applied by the self-interpreter yet: the runtime value it needs, which its values do not carry.
    Awaits(&'static str),
}

const fn op(params: [&'static str; 2], answer: &'static str) -> OpOverload {
    OpOverload { params, answer, guards: &[] }
}

const ZERO_DIVISOR: &[Guard] = &[Guard { when: "{right} equals 0", report: "Division by zero" }];
const ZERO_REAL_DIVISOR: &[Guard] = &[Guard { when: "{right} equals 0.0", report: "Division by zero" }];
const ZERO_MODULUS: &[Guard] = &[Guard { when: "{right} equals 0", report: "Modulo by zero" }];
const NEGATIVE_EXPONENT: &[Guard] = &[Guard {
    when: "{right} is less than 0",
    report: "negative exponent on an integer (an Int can't hold a fraction — use a Float base)",
}];

/// The four ways two numbers meet: `Int` with `Int` answering `int`, every pair with a `Real` answering `Real`.
const fn numeric(int: &'static str, real: &'static str) -> [OpOverload; 4] {
    [op(["Int", "Int"], int), op(["Real", "Real"], real), op(["Int", "Real"], real), op(["Real", "Int"], real)]
}

/// A division's four ways, each stopping on a zero divisor with `int`'s and `real`'s report.
const fn dividing(int: &'static [Guard], real: &'static [Guard]) -> [OpOverload; 4] {
    [
        OpOverload { params: ["Int", "Int"], answer: "Int", guards: int },
        OpOverload { params: ["Real", "Real"], answer: "Real", guards: real },
        OpOverload { params: ["Int", "Real"], answer: "Real", guards: real },
        OpOverload { params: ["Real", "Int"], answer: "Real", guards: int },
    ]
}

const ADD: &[OpOverload] = &{
    let n = numeric("Int", "Real");
    [
        n[0],
        n[1],
        n[2],
        n[3],
        op(["Text", "Text"], "Text"),
        op(["Text", "Any"], "Text"),
        op(["Any", "Text"], "Text"),
        op(["Seq", "Seq"], "Seq"),
        op(["Duration", "Duration"], "Duration"),
        op(["Date", "Span"], "Date"),
        op(["Moment", "Span"], "Moment"),
        op(["Span", "Moment"], "Moment"),
        op(["Moment", "Duration"], "Moment"),
        op(["Duration", "Moment"], "Moment"),
    ]
};
/// A difference: of two numbers, of two durations, or a date or an instant moved back by a span or a duration.
const SUBTRACT: &[OpOverload] = &{
    let n = numeric("Int", "Real");
    [
        n[0],
        n[1],
        n[2],
        n[3],
        op(["Duration", "Duration"], "Duration"),
        op(["Date", "Span"], "Date"),
        op(["Moment", "Span"], "Moment"),
        op(["Moment", "Duration"], "Moment"),
    ]
};
/// A list times a count is the list repeated, either way round.
const MULTIPLY: &[OpOverload] = &{
    let n = numeric("Int", "Real");
    [n[0], n[1], n[2], n[3], op(["Int", "Seq"], "Seq"), op(["Seq", "Int"], "Seq")]
};
const POWER: &[OpOverload] = &[
    OpOverload { params: ["Int", "Int"], answer: "Int", guards: NEGATIVE_EXPONENT },
    op(["Real", "Real"], "Real"),
    op(["Int", "Real"], "Real"),
    op(["Real", "Int"], "Real"),
];
const DIVIDE: &[OpOverload] = &dividing(ZERO_DIVISOR, ZERO_REAL_DIVISOR);
const MODULO: &[OpOverload] = &[OpOverload { params: ["Int", "Int"], answer: "Int", guards: ZERO_MODULUS }];
const ORDER: &[OpOverload] = &numeric("Bool", "Bool");
/// Two values the language orders: two numbers, two values of one temporal kind, or an instant and a time of day,
/// which it orders by the instant's time of day.
const RELATION: &[OpOverload] = &{
    let n = numeric("Bool", "Bool");
    [
        n[0],
        n[1],
        n[2],
        n[3],
        op(["Duration", "Duration"], "Bool"),
        op(["Date", "Date"], "Bool"),
        op(["Moment", "Moment"], "Bool"),
        op(["Time", "Time"], "Bool"),
        op(["Moment", "Time"], "Bool"),
        op(["Time", "Moment"], "Bool"),
    ]
};
const BITWISE: &[OpOverload] = &[op(["Int", "Int"], "Int"), op(["Bool", "Bool"], "Bool")];
const SHIFT: &[OpOverload] = &[op(["Int", "Int"], "Int")];
const SHOWN: &[OpOverload] = &[op(["Any", "Any"], "Text")];
const SEQUENCES: &[OpOverload] = &[op(["Seq", "Seq"], "Seq")];

/// A refusal's case for any operands.
const fn any(report: &'static str) -> Refusal {
    Refusal { left: "*", right: "*", report }
}

const NO_ADD: &[Refusal] = &[any("Cannot add {1} and {2}")];
const NO_SUBTRACT: &[Refusal] = &[any("Cannot subtract {2} from {1}")];
const NO_MULTIPLY: &[Refusal] = &[any("Cannot multiply {1} and {2}")];
/// A power with a `Real` operand is taken in the reals, which ask each operand for a number in turn.
const NO_POWER: &[Refusal] = &[
    Refusal { left: "Float", right: "*", report: "cannot raise to a {2} power" },
    Refusal { left: "*", right: "Float", report: "cannot raise {1} to a power" },
    any("cannot raise {1} to the {2} power"),
];
const NO_DIVIDE: &[Refusal] = &[any("Cannot divide {1} by {2}")];
const NO_FLOOR_DIVIDE: &[Refusal] = &[any("Cannot floor-divide {1} by {2}")];
const NO_MODULO: &[Refusal] = &[any("Cannot compute modulo of {1} and {2}")];
const NO_ORDER: &[Refusal] = &[any("Cannot compare {1} and {2}")];
const NO_APPROXIMATION: &[Refusal] = &[any("`is approximately` compares numbers, got {1} and {2}")];
const NO_SEQUENCES: &[Refusal] = &[any("`followed by` requires two sequences (merge two sequences into one)")];
const NO_XOR: &[Refusal] = &[any(logicaffeine_data::failure::BITXOR_OPERANDS)];
const NO_AND: &[Refusal] = &[any(logicaffeine_data::failure::BITAND_OPERANDS)];
const NO_OR: &[Refusal] = &[any(logicaffeine_data::failure::BITOR_OPERANDS)];
const NO_SHL: &[Refusal] = &[any(logicaffeine_data::failure::SHL_OPERANDS)];
const NO_SHR: &[Refusal] = &[any(logicaffeine_data::failure::SHR_OPERANDS)];

/// The Core IR's spelling of each operator: what a `CBinOp`'s `op` holds. A division the program writes and one the
/// analysis made exact share `/`.
pub fn core_op_text(op: BinaryOpKind) -> &'static str {
    match op {
        BinaryOpKind::Add => "+",
        BinaryOpKind::Subtract => "-",
        BinaryOpKind::Multiply => "*",
        BinaryOpKind::Divide => "/",
        BinaryOpKind::ExactDivide => "exact /",
        BinaryOpKind::FloorDivide => "//",
        BinaryOpKind::Modulo => "%",
        BinaryOpKind::Eq => "==",
        BinaryOpKind::NotEq => "!=",
        BinaryOpKind::Lt => "<",
        BinaryOpKind::Gt => ">",
        BinaryOpKind::LtEq => "<=",
        BinaryOpKind::GtEq => ">=",
        BinaryOpKind::And => "&&",
        BinaryOpKind::Or => "||",
        BinaryOpKind::Concat => "combined with",
        BinaryOpKind::SeqConcat => "followed by",
        BinaryOpKind::ApproxEq => "is approximately",
        BinaryOpKind::Pow => "**",
        BinaryOpKind::BitXor => "^",
        BinaryOpKind::BitAnd => "&",
        BinaryOpKind::BitOr => "|",
        BinaryOpKind::Shl => "<<",
        BinaryOpKind::Shr => ">>",
    }
}

/// What a program writes between an operator's operands: the spelling the self-interpreter applies it by and every
/// decompiled residual prints it with.
pub fn surface_text(op: BinaryOpKind) -> &'static str {
    match op {
        BinaryOpKind::Add => "+",
        BinaryOpKind::Subtract => "-",
        BinaryOpKind::Multiply => "*",
        BinaryOpKind::Divide | BinaryOpKind::ExactDivide => "/",
        BinaryOpKind::FloorDivide => "//",
        BinaryOpKind::Modulo => "%",
        BinaryOpKind::Eq => "equals",
        BinaryOpKind::NotEq => "is not",
        BinaryOpKind::Lt => "is less than",
        BinaryOpKind::Gt => "is greater than",
        BinaryOpKind::LtEq => "is at most",
        BinaryOpKind::GtEq => "is at least",
        BinaryOpKind::And => "and",
        BinaryOpKind::Or => "or",
        BinaryOpKind::Concat => "combined with",
        BinaryOpKind::SeqConcat => "followed by",
        BinaryOpKind::ApproxEq => "is approximately",
        BinaryOpKind::Pow => "**",
        BinaryOpKind::BitXor => "xor",
        BinaryOpKind::BitAnd => "&",
        BinaryOpKind::BitOr => "|",
        BinaryOpKind::Shl => "shifted left by",
        BinaryOpKind::Shr => "shifted right by",
    }
}

/// Every operator, in the order the self-interpreter's dispatch tries them.
pub const ALL_OPERATORS: &[BinaryOpKind] = &[
    BinaryOpKind::Add,
    BinaryOpKind::Subtract,
    BinaryOpKind::Multiply,
    BinaryOpKind::Pow,
    BinaryOpKind::Divide,
    BinaryOpKind::ExactDivide,
    BinaryOpKind::FloorDivide,
    BinaryOpKind::Modulo,
    BinaryOpKind::Eq,
    BinaryOpKind::NotEq,
    BinaryOpKind::Lt,
    BinaryOpKind::Gt,
    BinaryOpKind::LtEq,
    BinaryOpKind::GtEq,
    BinaryOpKind::And,
    BinaryOpKind::Or,
    BinaryOpKind::Concat,
    BinaryOpKind::SeqConcat,
    BinaryOpKind::ApproxEq,
    BinaryOpKind::BitXor,
    BinaryOpKind::BitAnd,
    BinaryOpKind::BitOr,
    BinaryOpKind::Shl,
    BinaryOpKind::Shr,
];

/// How the self-interpreter applies `op`. Exhaustive: a new operator does not compile until it is classified.
pub fn self_applied(op: BinaryOpKind) -> SelfApplied {
    use SelfApplied::*;
    let overloads = |overloads, refusal| Overloads { overloads, refusal };
    match op {
        BinaryOpKind::Add => overloads(ADD, NO_ADD),
        BinaryOpKind::Subtract => overloads(SUBTRACT, NO_SUBTRACT),
        BinaryOpKind::Multiply => overloads(MULTIPLY, NO_MULTIPLY),
        BinaryOpKind::Pow => overloads(POWER, NO_POWER),
        BinaryOpKind::Divide => overloads(DIVIDE, NO_DIVIDE),
        BinaryOpKind::ExactDivide => Awaits("an exact Rational"),
        BinaryOpKind::FloorDivide => overloads(DIVIDE, NO_FLOOR_DIVIDE),
        BinaryOpKind::Modulo => overloads(MODULO, NO_MODULO),
        BinaryOpKind::Eq | BinaryOpKind::NotEq | BinaryOpKind::And | BinaryOpKind::Or => Defined,
        BinaryOpKind::Lt => overloads(RELATION, NO_ORDER),
        BinaryOpKind::Gt => overloads(RELATION, NO_ORDER),
        BinaryOpKind::LtEq => overloads(RELATION, NO_ORDER),
        BinaryOpKind::GtEq => overloads(RELATION, NO_ORDER),
        BinaryOpKind::Concat => overloads(SHOWN, &[]),
        BinaryOpKind::SeqConcat => overloads(SEQUENCES, NO_SEQUENCES),
        BinaryOpKind::ApproxEq => overloads(ORDER, NO_APPROXIMATION),
        BinaryOpKind::BitXor => overloads(BITWISE, NO_XOR),
        BinaryOpKind::BitAnd => overloads(BITWISE, NO_AND),
        BinaryOpKind::BitOr => overloads(BITWISE, NO_OR),
        BinaryOpKind::Shl => overloads(SHIFT, NO_SHL),
        BinaryOpKind::Shr => overloads(SHIFT, NO_SHR),
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/semantics/operator_overloads/tests.rs"]
mod tests;
