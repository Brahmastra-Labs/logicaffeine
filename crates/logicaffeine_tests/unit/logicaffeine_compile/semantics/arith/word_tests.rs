use super::*;
use logicaffeine_base::{Word32, Word64, WordVal};

fn w32(n: u32) -> RuntimeValue {
    RuntimeValue::Word(WordVal::W32(Word32(n)))
}
fn w64(n: u64) -> RuntimeValue {
    RuntimeValue::Word(WordVal::W64(Word64(n)))
}

#[test]
fn word_arithmetic_wraps_through_binary_op() {
    // The ring ℤ/2³²: MAX + 1 wraps to 0 and never promotes to BigInt.
    assert_eq!(
        binary_op(BinaryOpKind::Add, w32(0xFFFF_FFFF), w32(1)).unwrap(),
        w32(0)
    );
    assert_eq!(
        binary_op(BinaryOpKind::Subtract, w32(0), w32(1)).unwrap(),
        w32(0xFFFF_FFFF)
    );
    assert_eq!(
        binary_op(BinaryOpKind::Multiply, w32(0x1000_0000), w32(0x10)).unwrap(),
        w32(0)
    );
    assert_eq!(
        binary_op(BinaryOpKind::BitXor, w32(0xFF00), w32(0x0FF0)).unwrap(),
        w32(0xF0F0)
    );
    assert_eq!(
        binary_op(BinaryOpKind::And, w32(0xFF00), w32(0x0FF0)).unwrap(),
        w32(0x0F00)
    );
    assert_eq!(
        binary_op(BinaryOpKind::Or, w32(0xFF00), w32(0x0FF0)).unwrap(),
        w32(0xFFF0)
    );
}

#[test]
fn word_shift_takes_an_integer_count() {
    assert_eq!(
        binary_op(BinaryOpKind::Shl, w32(1), RuntimeValue::Int(8)).unwrap(),
        w32(0x100)
    );
    assert_eq!(
        binary_op(BinaryOpKind::Shr, w32(0x100), RuntimeValue::Int(8)).unwrap(),
        w32(1)
    );
}

#[test]
fn word_equality_and_width_mismatch_is_typed() {
    assert!(values_equal(&w32(5), &w32(5)));
    assert!(
        !values_equal(&w32(5), &w64(5)),
        "different widths are never equal"
    );
    assert!(
        binary_op(BinaryOpKind::Add, w32(1), w64(1)).is_err(),
        "Word32 + Word64 must error, not silently coerce"
    );
}

#[test]
fn word_division_and_modulo_are_unsigned_truncating() {
    // `/` and `%` are the raw UNSIGNED integer ops (`x % 2^k` is a mask, `x / 2^k`
    // a shift) — the same native `/`/`%` the C/Zig ports use on machine ints.
    assert_eq!(
        binary_op(BinaryOpKind::Divide, w64(100), w64(7)).unwrap(),
        w64(14)
    );
    assert_eq!(
        binary_op(BinaryOpKind::Modulo, w64(100), w64(7)).unwrap(),
        w64(2)
    );
    // Unsigned: the top-bit-set value is huge, not negative — `% 2` masks its low bit.
    assert_eq!(
        binary_op(BinaryOpKind::Modulo, w64(0xFFFF_FFFF_FFFF_FFFF), w64(2)).unwrap(),
        w64(1)
    );
    assert_eq!(
        binary_op(BinaryOpKind::Divide, w32(255), w32(16)).unwrap(),
        w32(15)
    );
    // Divide/modulo by zero is a graceful typed error — never a process panic.
    assert!(binary_op(BinaryOpKind::Divide, w64(1), w64(0)).is_err());
    assert!(binary_op(BinaryOpKind::Modulo, w64(1), w64(0)).is_err());
    // Width mismatch is a typed error, mirroring `+`/`*` — never a silent coercion.
    assert!(binary_op(BinaryOpKind::Divide, w32(1), w64(1)).is_err());
    assert!(binary_op(BinaryOpKind::Modulo, w32(1), w64(1)).is_err());
}

#[test]
fn word_ordering_is_unsigned() {
    use crate::semantics::compare::compare;
    // The whole point of the Word64 same-semantics track: `0xFFFF…` is the LARGEST
    // u64, not `-1`. Ordering matches C/Zig's unsigned compare, not signed `Int`.
    let big = w64(0xFFFF_FFFF_FFFF_FFFF);
    assert_eq!(
        compare(BinaryOpKind::Gt, &big, &w64(1)).unwrap(),
        RuntimeValue::Bool(true)
    );
    assert_eq!(
        compare(BinaryOpKind::Lt, &w64(1), &big).unwrap(),
        RuntimeValue::Bool(true)
    );
    assert_eq!(
        compare(BinaryOpKind::LtEq, &w64(5), &w64(5)).unwrap(),
        RuntimeValue::Bool(true)
    );
    assert_eq!(
        compare(BinaryOpKind::GtEq, &w64(5), &w64(4)).unwrap(),
        RuntimeValue::Bool(true)
    );
    assert_eq!(
        compare(BinaryOpKind::Lt, &w32(7), &w32(9)).unwrap(),
        RuntimeValue::Bool(true)
    );
    // Cross-width ordering is a typed error (mirrors arithmetic), never a coercion.
    assert!(compare(BinaryOpKind::Lt, &w32(1), &w64(2)).is_err());
}
