//! An integer chain under a low-bit mask, computed at the mask's width (`Opt::MaskNarrow`).
//!
//! The low `k` bits of `a + b`, `a - b`, `a * b`, `a & b`, `a | b`, `a ^ b` and `a << s` depend only on the low
//! `k` bits of `a` and `b`: reduction mod `2^k` is a ring homomorphism, the bitwise operators act bit by bit, and a
//! left shift moves bits upward only. So `x & (2^k - 1)` with `k <= 32` — or `x % 2^k` for an `x >= 0`, the same
//! value — of such a chain `x` is the chain computed in 32-bit wrapping arithmetic and masked, provided the chain's
//! value IS its i64 value: every `+`, `-` and `*` in it proven to stay within i64. Past i64 the language computes
//! the exact `BigInt`, whose `&` refuses a negative value, so an unproven node leaves the chain unnarrowed.
//!
//! A C or Rust compiler cannot narrow the 64-bit form itself when the chain is loop-carried: a masked LCG keeps a
//! serial 64-bit multiply chain, where the 32-bit form lets LLVM run the unrolled steps as independent multiplies by
//! `a`, `a²`, `a³`, `a⁴`. This module recognizes the shape; each emitter supplies its proofs ([`WordProof`]) and its
//! own spelling of the result.

use crate::ast::stmt::{BinaryOpKind, Expr, Literal};

/// What an emitter knows about the integers it emits.
pub(crate) trait WordProof {
    /// `node`, a `+`, `-` or `*`, computes its exact value without leaving i64.
    fn stays_in_word(&self, node: &Expr) -> bool;
    /// `operand` is emitted as a machine integer holding its exact value.
    fn is_word(&self, operand: &Expr) -> bool;
    /// `value >= 0` wherever it is evaluated.
    fn is_nonnegative(&self, value: &Expr) -> bool;
}

/// An operator of a low-bit chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LowOp {
    Add,
    Sub,
    Mul,
    And,
    Or,
    Xor,
}

/// A chain's value mod `2^32`.
#[derive(Debug)]
pub(crate) enum LowBits<'e, 'a> {
    /// An operand the chain does not see into, read as its low 32 bits.
    Word(&'e Expr<'a>),
    /// A literal's low 32 bits.
    Const(u32),
    /// Two low-bit chains combined.
    Op(LowOp, Box<LowBits<'e, 'a>>, Box<LowBits<'e, 'a>>),
    /// A chain shifted left by a constant below 32.
    Shl(Box<LowBits<'e, 'a>>, u32),
}

/// `expr` as a 32-bit chain under its mask: `chain & mask` is `expr`'s value.
#[derive(Debug)]
pub(crate) struct MaskedChain<'e, 'a> {
    pub chain: LowBits<'e, 'a>,
    pub mask: u32,
}

/// `k` for a literal `2^k - 1` with `1 <= k <= 32`.
fn low_mask_width(e: &Expr) -> Option<u32> {
    match e {
        Expr::Literal(Literal::Number(m)) if *m >= 1 && *m <= u32::MAX as i64 => {
            let k = (*m as u64 + 1).trailing_zeros();
            ((*m as u64 + 1) == 1u64 << k).then_some(k)
        }
        _ => None,
    }
}

/// `k` for a literal `2^k` with `1 <= k <= 63`.
fn power_of_two(e: &Expr) -> Option<u32> {
    match e {
        Expr::Literal(Literal::Number(d)) if *d >= 2 && (*d as u64).is_power_of_two() => {
            Some(d.trailing_zeros())
        }
        _ => None,
    }
}

/// `expr` as a chain under a low mask of at most 32 bits that does 64-bit arithmetic worth narrowing (a `+`, `-`,
/// `*` or `<<`), with every node and operand vouched for by `proof`. `None` for any other expression.
pub(crate) fn masked_chain<'e, 'a>(
    expr: &'e Expr<'a>,
    proof: &dyn WordProof,
) -> Option<MaskedChain<'e, 'a>> {
    let Expr::BinaryOp { op, left, right } = expr else {
        return None;
    };
    let (value, k) = match op {
        BinaryOpKind::BitAnd => match (low_mask_width(right), low_mask_width(left)) {
            (Some(k), _) => (*left, k),
            (None, Some(k)) => (*right, k),
            (None, None) => return None,
        },
        BinaryOpKind::Modulo => {
            let k = power_of_two(right).filter(|k| *k <= 32)?;
            if !proof.is_nonnegative(left) {
                return None;
            }
            (*left, k)
        }
        _ => return None,
    };
    let chain = low_bits(value, proof)?;
    if !does_word_arithmetic(&chain) {
        return None;
    }
    Some(MaskedChain {
        chain,
        mask: (((1u64 << k) - 1) as u32),
    })
}

/// `e`'s value mod `2^32` as a chain, or `None` when some node or operand is not vouched for.
fn low_bits<'e, 'a>(e: &'e Expr<'a>, proof: &dyn WordProof) -> Option<LowBits<'e, 'a>> {
    let pair = |op, l: &'e Expr<'a>, r: &'e Expr<'a>| -> Option<LowBits<'e, 'a>> {
        Some(LowBits::Op(
            op,
            Box::new(low_bits(l, proof)?),
            Box::new(low_bits(r, proof)?),
        ))
    };
    match e {
        Expr::Literal(Literal::Number(n)) => Some(LowBits::Const(*n as u32)),
        Expr::BinaryOp { op, left, right } => match op {
            BinaryOpKind::Add | BinaryOpKind::Subtract | BinaryOpKind::Multiply => {
                if !proof.stays_in_word(e) {
                    return None;
                }
                let op = match op {
                    BinaryOpKind::Add => LowOp::Add,
                    BinaryOpKind::Subtract => LowOp::Sub,
                    _ => LowOp::Mul,
                };
                pair(op, left, right)
            }
            BinaryOpKind::BitAnd => pair(LowOp::And, left, right),
            BinaryOpKind::BitOr => pair(LowOp::Or, left, right),
            BinaryOpKind::BitXor => pair(LowOp::Xor, left, right),
            // `a << s` shifts by `s mod 64` (`i64::wrapping_shl`); a shift of 32 or more leaves no low bit, and
            // its operand is still evaluated, so it is read whole as an operand.
            BinaryOpKind::Shl => match right {
                Expr::Literal(Literal::Number(s)) if (*s as u32) & 63 < 32 => Some(LowBits::Shl(
                    Box::new(low_bits(left, proof)?),
                    (*s as u32) & 63,
                )),
                _ => word(e, proof),
            },
            // `x % 2^j` of an `x >= 0` is `x & (2^j - 1)`, and with `j >= 32` keeps every low 32 bits.
            BinaryOpKind::Modulo => match power_of_two(right) {
                Some(j) if proof.is_nonnegative(left) => {
                    let inner = low_bits(left, proof)?;
                    Some(if j >= 32 {
                        inner
                    } else {
                        LowBits::Op(
                            LowOp::And,
                            Box::new(inner),
                            Box::new(LowBits::Const(((1u64 << j) - 1) as u32)),
                        )
                    })
                }
                _ => word(e, proof),
            },
            _ => word(e, proof),
        },
        _ => word(e, proof),
    }
}

fn word<'e, 'a>(e: &'e Expr<'a>, proof: &dyn WordProof) -> Option<LowBits<'e, 'a>> {
    proof.is_word(e).then_some(LowBits::Word(e))
}

/// Whether `chain` has a `+`, `-`, `*` or `<<` — arithmetic the 64-bit form does in 64 bits.
fn does_word_arithmetic(chain: &LowBits) -> bool {
    match chain {
        LowBits::Word(_) | LowBits::Const(_) => false,
        LowBits::Shl(..) => true,
        LowBits::Op(op, l, r) => {
            matches!(op, LowOp::Add | LowOp::Sub | LowOp::Mul)
                || does_word_arithmetic(l)
                || does_word_arithmetic(r)
        }
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen/lowbits/tests.rs"]
mod tests;
