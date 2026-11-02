//! ★ LOCK — a compiled program's bit operations on exact integers answer what the reference answers. ★
//!
//! The generated Rust hands an operand of `&`, `|`, `^`, `<<` or `>>` that may be a big integer to
//! `logos_band_exact` / `logos_word_operand`. Over machine words, big integers of both signs, low masks and
//! values that are not masks, each answers exactly what `semantics::arith::binary_op` answers: the same value,
//! or a refusal in the same words.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test exact_bitwise_lock`

use logicaffeine_base::BigInt;
use logicaffeine_compile::ast::stmt::BinaryOpKind;
use logicaffeine_compile::interpreter::RuntimeValue;
use logicaffeine_compile::semantics::arith::binary_op;
use logicaffeine_data::failure;
use logicaffeine_data::{logos_band_exact, logos_word_operand, LogosInt};

fn big(decimal: &str) -> BigInt {
    let (negative, digits) = match decimal.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, decimal),
    };
    let ten = BigInt::from_i64(10);
    let magnitude = digits.bytes().fold(BigInt::from_i64(0), |acc, d| {
        acc.mul(&ten).add(&BigInt::from_i64((d - b'0') as i64))
    });
    if negative {
        magnitude.negated()
    } else {
        magnitude
    }
}

/// Every operand: words at the edges, low masks of every interesting width, values that are not masks, and big
/// integers of both signs.
fn operands() -> Vec<String> {
    let mut v: Vec<String> = [
        0i64,
        1,
        -1,
        5,
        6,
        255,
        256,
        65535,
        (1 << 31) - 1,
        (1 << 32) - 1,
        1 << 40,
        i64::MAX,
        i64::MIN,
        i64::MIN + 1,
        -256,
    ]
    .iter()
    .map(|n| n.to_string())
    .collect();
    v.extend(
        [
            "9223372036854775808",
            "18446744073709551621",
            "1267650600228229401496703205395",
            "-9223372036854775809",
            "-18446744073709551621",
            "-1267650600228229401496703205395",
        ]
        .map(String::from),
    );
    v
}

fn reference(op: BinaryOpKind, a: &str, b: &str) -> Result<i64, String> {
    match binary_op(op, RuntimeValue::from_bigint(big(a)), RuntimeValue::from_bigint(big(b))) {
        Ok(RuntimeValue::Int(v)) => Ok(v),
        Ok(other) => panic!("{a} {op:?} {b} gave {other:?}, which no word operation gives"),
        Err(why) => Err(why),
    }
}

fn compiled(run: impl FnOnce() -> i64 + std::panic::UnwindSafe) -> Result<i64, String> {
    std::panic::catch_unwind(run).map_err(|panic| {
        panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default()
    })
}

fn quiet<T>(f: impl FnOnce() -> T) -> T {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let out = f();
    std::panic::set_hook(hook);
    out
}

#[test]
fn band_exact_is_the_reference_on_every_pairing() {
    let ops = operands();
    let mut checked = (0usize, 0usize);
    quiet(|| {
        for a in &ops {
            for b in &ops {
                let want = reference(BinaryOpKind::BitAnd, a, b);
                let (x, y) = (LogosInt::from_big(big(a)), LogosInt::from_big(big(b)));
                let got = compiled(move || logos_band_exact(x, y));
                assert_eq!(got, want, "{a} & {b}");
                if want.is_ok() {
                    checked.0 += 1;
                } else {
                    checked.1 += 1;
                }
            }
        }
    });
    assert!(checked.0 > 200 && checked.1 > 50, "values {} refusals {}", checked.0, checked.1);
}

#[test]
fn a_big_integer_takes_a_low_mask_only_when_it_is_not_negative() {
    let mask = LogosInt::from_i64(255);
    assert_eq!(logos_band_exact(LogosInt::from_big(big("18446744073709551621")), mask.clone()), 5);
    assert_eq!(
        logos_band_exact(
            LogosInt::from_i64(i64::MAX),
            LogosInt::from_big(big("18446744073709551621"))
        ),
        5,
        "a mask of 63 bits"
    );
    let refused = quiet(|| {
        compiled(|| logos_band_exact(LogosInt::from_big(big("-18446744073709551621")), mask))
    });
    assert_eq!(refused, Err(failure::BITAND_OPERANDS.to_string()));
}

#[test]
fn a_word_operand_passes_a_word_and_refuses_a_big_integer_in_each_operators_words() {
    let ops = operands();
    quiet(|| {
        for (op, why) in [
            (BinaryOpKind::BitOr, failure::BITOR_OPERANDS),
            (BinaryOpKind::BitXor, failure::BITXOR_OPERANDS),
            (BinaryOpKind::Shl, failure::SHL_OPERANDS),
            (BinaryOpKind::Shr, failure::SHR_OPERANDS),
        ] {
            for a in &ops {
                for b in &ops {
                    let want = reference(op, a, b);
                    let (x, y) = (LogosInt::from_big(big(a)), LogosInt::from_big(big(b)));
                    let got = compiled(move || {
                        let (l, r) = (logos_word_operand(x, why), logos_word_operand(y, why));
                        match op {
                            BinaryOpKind::BitOr => l | r,
                            BinaryOpKind::BitXor => l ^ r,
                            BinaryOpKind::Shl => l.wrapping_shl(r as u32),
                            _ => l.wrapping_shr(r as u32),
                        }
                    });
                    assert_eq!(got, want, "{a} {op:?} {b}");
                }
            }
        }
    });
}
