//! Built-in functions over already-evaluated values.
//!
//! `show` is NOT here — output is an engine concern. Arity is checked by the
//! caller BEFORE evaluating arguments (via [`check_arity`]) to preserve the
//! tree-walker's error ordering: a wrong-arity call reports the arity error
//! even when an argument expression would itself error.

use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::interpreter::RuntimeValue;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuiltinId {
    Length,
    Format,
    ParseInt,
    ParseFloat,
    Chr,
    Abs,
    Sqrt,
    Min,
    Max,
    Floor,
    Ceil,
    Round,
    Pow,
    Copy,
    CountOnes,
    RunAccepted,
}

/// Resolve a function name to a builtin, if it is one.
pub fn builtin_from_name(name: &str) -> Option<BuiltinId> {
    Some(match name {
        "length" => BuiltinId::Length,
        "format" => BuiltinId::Format,
        "parseInt" => BuiltinId::ParseInt,
        "parseFloat" => BuiltinId::ParseFloat,
        "chr" => BuiltinId::Chr,
        "abs" => BuiltinId::Abs,
        "sqrt" => BuiltinId::Sqrt,
        "min" => BuiltinId::Min,
        "max" => BuiltinId::Max,
        "floor" => BuiltinId::Floor,
        "ceil" => BuiltinId::Ceil,
        "round" => BuiltinId::Round,
        "pow" => BuiltinId::Pow,
        "copy" => BuiltinId::Copy,
        "count_ones" => BuiltinId::CountOnes,
        "run_accepted" => BuiltinId::RunAccepted,
        _ => return None,
    })
}

/// Check the call's arity BEFORE evaluating arguments. `format` accepts any
/// arity (it reads only its first argument, or none).
pub fn check_arity(id: BuiltinId, n: usize) -> Result<(), String> {
    let expected: usize = match id {
        BuiltinId::Format => return Ok(()),
        BuiltinId::Min | BuiltinId::Max | BuiltinId::Pow => 2,
        // run_accepted(fn, arg, lo, hi): the shipped computation + the argument + the
        // inclusive bounds of the acceptance contract.
        BuiltinId::RunAccepted => 4,
        _ => 1,
    };
    if n != expected {
        let name = match id {
            BuiltinId::Length => "length",
            BuiltinId::Format => unreachable!(),
            BuiltinId::ParseInt => "parseInt",
            BuiltinId::ParseFloat => "parseFloat",
            BuiltinId::Chr => "chr",
            BuiltinId::Abs => "abs",
            BuiltinId::Sqrt => "sqrt",
            BuiltinId::Min => "min",
            BuiltinId::Max => "max",
            BuiltinId::Floor => "floor",
            BuiltinId::Ceil => "ceil",
            BuiltinId::Round => "round",
            BuiltinId::Pow => "pow",
            BuiltinId::Copy => "copy",
            BuiltinId::CountOnes => "count_ones",
            BuiltinId::RunAccepted => "run_accepted",
        };
        return Err(format!(
            "{}() takes exactly {} argument{}",
            name,
            expected,
            if expected == 1 { "" } else { "s" }
        ));
    }
    Ok(())
}

/// Apply a builtin to already-evaluated arguments. The caller has already
/// validated arity with [`check_arity`].
pub fn call_builtin(id: BuiltinId, args: Vec<RuntimeValue>) -> Result<RuntimeValue, String> {
    let mut args = args;
    match id {
        BuiltinId::Length => {
            let val = args.remove(0);
            match &val {
                RuntimeValue::List(items) => Ok(RuntimeValue::Int(items.borrow().len() as i64)),
                RuntimeValue::Text(s) => Ok(RuntimeValue::Int(s.len() as i64)),
                RuntimeValue::Map(map) => Ok(RuntimeValue::Int(map.borrow().len() as i64)),
                _ => Err(format!("Cannot get length of {}", val.type_name())),
            }
        }
        BuiltinId::Format => {
            if args.is_empty() {
                return Ok(RuntimeValue::Text(Rc::new(String::new())));
            }
            let val = args.remove(0);
            Ok(RuntimeValue::Text(Rc::new(val.to_display_string())))
        }
        BuiltinId::ParseInt => {
            let val = args.remove(0);
            if let RuntimeValue::Text(s) = &val {
                Ok(RuntimeValue::Int(
                    s.trim()
                        .parse::<i64>()
                        .map_err(|_| format!("Cannot parse '{}' as Int", s))?,
                ))
            } else {
                Err("parseInt requires a Text argument".to_string())
            }
        }
        BuiltinId::ParseFloat => {
            let val = args.remove(0);
            if let RuntimeValue::Text(s) = &val {
                Ok(RuntimeValue::Float(
                    s.trim()
                        .parse::<f64>()
                        .map_err(|_| format!("Cannot parse '{}' as Float", s))?,
                ))
            } else {
                Err("parseFloat requires a Text argument".to_string())
            }
        }
        BuiltinId::Chr => {
            let val = args.remove(0);
            if let RuntimeValue::Int(code) = val {
                match char::from_u32(code as u32) {
                    Some(c) => Ok(RuntimeValue::Text(Rc::new(c.to_string()))),
                    None => Err(format!("Invalid character code: {}", code)),
                }
            } else {
                Err("chr() requires an Int argument".to_string())
            }
        }
        BuiltinId::Abs => {
            let val = args.remove(0);
            match val {
                // |i64::MIN| = 2^63 does not fit i64, so abs is EXACT via promotion
                // (wrapping_abs would have returned i64::MIN — a sign bug).
                RuntimeValue::Int(n) => Ok(match n.checked_abs() {
                    Some(a) => RuntimeValue::Int(a),
                    None => RuntimeValue::from_bigint(logicaffeine_base::BigInt::from_i64(n).abs()),
                }),
                RuntimeValue::BigInt(b) => Ok(RuntimeValue::from_bigint(b.abs())),
                // |·| of a rational stays a rational (`|-7/2| = 7/2`), downsized if whole.
                RuntimeValue::Rational(r) => Ok(RuntimeValue::from_rational(r.abs())),
                RuntimeValue::Float(f) => Ok(RuntimeValue::Float(f.abs())),
                _ => Err(format!("abs() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Sqrt => {
            let val = args.remove(0);
            match val {
                RuntimeValue::Float(f) => Ok(RuntimeValue::Float(f.sqrt())),
                RuntimeValue::Int(n) => Ok(RuntimeValue::Float((n as f64).sqrt())),
                RuntimeValue::BigInt(b) => Ok(RuntimeValue::Float(b.to_f64().sqrt())),
                _ => Err(format!("sqrt() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Min => {
            let b = args.remove(1);
            let a = args.remove(0);
            match (&a, &b) {
                (RuntimeValue::Int(x), RuntimeValue::Int(y)) => Ok(RuntimeValue::Int(*x.min(y))),
                (RuntimeValue::Float(x), RuntimeValue::Float(y)) => {
                    Ok(RuntimeValue::Float(x.min(*y)))
                }
                (RuntimeValue::Int(x), RuntimeValue::Float(y)) => {
                    Ok(RuntimeValue::Float((*x as f64).min(*y)))
                }
                (RuntimeValue::Float(x), RuntimeValue::Int(y)) => {
                    Ok(RuntimeValue::Float(x.min(*y as f64)))
                }
                _ => Err("min() requires numbers".to_string()),
            }
        }
        BuiltinId::Max => {
            let b = args.remove(1);
            let a = args.remove(0);
            match (&a, &b) {
                (RuntimeValue::Int(x), RuntimeValue::Int(y)) => Ok(RuntimeValue::Int(*x.max(y))),
                (RuntimeValue::Float(x), RuntimeValue::Float(y)) => {
                    Ok(RuntimeValue::Float(x.max(*y)))
                }
                (RuntimeValue::Int(x), RuntimeValue::Float(y)) => {
                    Ok(RuntimeValue::Float((*x as f64).max(*y)))
                }
                (RuntimeValue::Float(x), RuntimeValue::Int(y)) => {
                    Ok(RuntimeValue::Float(x.max(*y as f64)))
                }
                _ => Err("max() requires numbers".to_string()),
            }
        }
        BuiltinId::Floor => {
            let val = args.remove(0);
            // An exact integer (Int/BigInt) is already whole; a Rational rounds toward
            // −∞ to an exact integer — the explicit floor of `floor(7 / 2) → 3`.
            match &val {
                RuntimeValue::Float(f) => Ok(RuntimeValue::Int(f.floor() as i64)),
                RuntimeValue::Int(_) | RuntimeValue::BigInt(_) => Ok(val.clone()),
                RuntimeValue::Rational(r) => Ok(RuntimeValue::from_bigint(r.floor())),
                _ => Err(format!("floor() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Ceil => {
            let val = args.remove(0);
            match &val {
                RuntimeValue::Float(f) => Ok(RuntimeValue::Int(f.ceil() as i64)),
                RuntimeValue::Int(_) | RuntimeValue::BigInt(_) => Ok(val.clone()),
                RuntimeValue::Rational(r) => Ok(RuntimeValue::from_bigint(r.ceil())),
                _ => Err(format!("ceil() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Round => {
            let val = args.remove(0);
            match &val {
                RuntimeValue::Float(f) => Ok(RuntimeValue::Int(f.round() as i64)),
                RuntimeValue::Int(_) | RuntimeValue::BigInt(_) => Ok(val.clone()),
                RuntimeValue::Rational(r) => Ok(RuntimeValue::from_bigint(r.round())),
                _ => Err(format!("round() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Pow => {
            let exp = args.remove(1);
            let base = args.remove(0);
            match (&base, &exp) {
                // EXACT integer power: on i64 overflow, promote to BigInt rather than
                // wrapping (e.g. 2^63 is the value, not i64::MIN). A negative exponent
                // is a fractional power, so it falls to f64 as before.
                (RuntimeValue::Int(b), RuntimeValue::Int(e)) => {
                    if *e >= 0 {
                        Ok(match b.checked_pow(*e as u32) {
                            Some(p) => RuntimeValue::Int(p),
                            None => RuntimeValue::from_bigint(logicaffeine_base::BigInt::from_i64(*b).pow(*e as u32)),
                        })
                    } else {
                        Ok(RuntimeValue::Float((*b as f64).powi(*e as i32)))
                    }
                }
                (RuntimeValue::BigInt(b), RuntimeValue::Int(e)) => {
                    if *e >= 0 {
                        Ok(RuntimeValue::from_bigint(b.pow(*e as u32)))
                    } else {
                        Ok(RuntimeValue::Float(b.to_f64().powi(*e as i32)))
                    }
                }
                (RuntimeValue::Float(b), RuntimeValue::Int(e)) => {
                    Ok(RuntimeValue::Float(b.powi(*e as i32)))
                }
                (RuntimeValue::Float(b), RuntimeValue::Float(e)) => {
                    Ok(RuntimeValue::Float(b.powf(*e)))
                }
                (RuntimeValue::Int(b), RuntimeValue::Float(e)) => {
                    Ok(RuntimeValue::Float((*b as f64).powf(*e)))
                }
                _ => Err("pow() requires numbers".to_string()),
            }
        }
        BuiltinId::Copy => {
            let val = args.remove(0);
            Ok(val.deep_clone())
        }
        BuiltinId::CountOnes => {
            let val = args.remove(0);
            match val {
                // Two's-complement faithful: count set bits over the full 64-bit
                // pattern (matches codegen's `(x as u64).count_ones()`).
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int((n as u64).count_ones() as i64)),
                _ => Err(format!(
                    "count_ones() requires an Int, got {}",
                    val.type_name()
                )),
            }
        }
        BuiltinId::RunAccepted => {
            // run_accepted(fn, arg, lo, hi): the receiver's typed, bounded acceptance
            // contract for a SHIPPED computation — validate the function's shape and the
            // argument's range, then evaluate in the sandbox. Out-of-range is REFUSED, never
            // clamped; an ordinary (non-shipped) closure is refused at the signature check.
            let int_arg = |v: &RuntimeValue, what: &str| -> Result<i64, String> {
                match v {
                    RuntimeValue::Int(n) => Ok(*n),
                    other => Err(format!("run_accepted: {what} must be an Int, got {}", other.type_name())),
                }
            };
            let arg = int_arg(&args[1], "the argument")?;
            let lo = int_arg(&args[2], "the lower bound")?;
            let hi = int_arg(&args[3], "the upper bound")?;
            let contract = crate::semantics::acceptance::AcceptanceContract::new(lo, hi);
            contract.apply(&args[0], arg).map(RuntimeValue::Int)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_accepted_validates_then_runs_a_shipped_computation() {
        use crate::concurrency::marshal::GenExpr;
        use crate::interpreter::ClosureValue;
        use std::collections::HashMap;
        use std::rc::Rc;
        // A shipped `3·x + 1` computation (what a peer ships via `Send computed`).
        let mk = || {
            let gen = GenExpr::Add(
                Box::new(GenExpr::Mul(Box::new(GenExpr::Index), Box::new(GenExpr::Const(3)))),
                Box::new(GenExpr::Const(1)),
            );
            RuntimeValue::Function(Box::new(ClosureValue {
                body_index: usize::MAX,
                captured_env: HashMap::default(),
                param_names: vec![logicaffeine_base::Symbol::from_index(0)],
                generated: Some(Rc::new(gen)),
            }))
        };
        // In-bounds (5 ∈ [0,1000]) → 3·5 + 1 = 16, run in the sandbox.
        match call_builtin(
            BuiltinId::RunAccepted,
            vec![mk(), RuntimeValue::Int(5), RuntimeValue::Int(0), RuntimeValue::Int(1000)],
        ) {
            Ok(RuntimeValue::Int(n)) => assert_eq!(n, 16),
            other => panic!("in-bounds run_accepted should return Int(16), got {other:?}"),
        }
        // Out-of-bounds (5000 ∉ [0,1000]) → refused, not clamped.
        assert!(
            call_builtin(
                BuiltinId::RunAccepted,
                vec![mk(), RuntimeValue::Int(5000), RuntimeValue::Int(0), RuntimeValue::Int(1000)],
            )
            .is_err(),
            "an out-of-range argument must be refused at the contract"
        );
    }

    #[test]
    fn arity_messages_match_treewalker() {
        assert_eq!(
            check_arity(BuiltinId::Length, 2).unwrap_err(),
            "length() takes exactly 1 argument"
        );
        assert_eq!(
            check_arity(BuiltinId::Min, 1).unwrap_err(),
            "min() takes exactly 2 arguments"
        );
        assert!(check_arity(BuiltinId::Format, 0).is_ok());
        assert!(check_arity(BuiltinId::Format, 5).is_ok());
    }

    #[test]
    fn parse_and_chr_messages() {
        let e = call_builtin(
            BuiltinId::ParseInt,
            vec![RuntimeValue::Text(Rc::new("zz".to_string()))],
        )
        .unwrap_err();
        assert_eq!(e, "Cannot parse 'zz' as Int");
        let e = call_builtin(BuiltinId::Chr, vec![RuntimeValue::Int(-1)]).unwrap_err();
        assert_eq!(e, "Invalid character code: -1");
        let r = call_builtin(BuiltinId::Chr, vec![RuntimeValue::Int(65)]).unwrap();
        assert!(matches!(&r, RuntimeValue::Text(s) if **s == "A"));
    }

    #[test]
    fn numeric_builtins_coerce_like_treewalker() {
        assert!(matches!(
            call_builtin(BuiltinId::Sqrt, vec![RuntimeValue::Int(9)]).unwrap(),
            RuntimeValue::Float(f) if f == 3.0
        ));
        assert!(matches!(
            call_builtin(BuiltinId::Min, vec![RuntimeValue::Int(3), RuntimeValue::Float(2.5)]).unwrap(),
            RuntimeValue::Float(f) if f == 2.5
        ));
        assert!(matches!(
            call_builtin(BuiltinId::Floor, vec![RuntimeValue::Float(2.9)]).unwrap(),
            RuntimeValue::Int(2)
        ));
        assert!(matches!(
            call_builtin(BuiltinId::Pow, vec![RuntimeValue::Int(2), RuntimeValue::Int(10)]).unwrap(),
            RuntimeValue::Int(1024)
        ));
        // Negative Int exponent goes to float.
        assert!(matches!(
            call_builtin(BuiltinId::Pow, vec![RuntimeValue::Int(2), RuntimeValue::Int(-1)]).unwrap(),
            RuntimeValue::Float(f) if f == 0.5
        ));
    }

    #[test]
    fn abs_and_pow_are_exact_promoting_past_i64() {
        // Arrange / Act / Assert: |i64::MIN| = 2^63 has no i64 representation, so abs
        // promotes to the EXACT BigInt rather than wrapping back to i64::MIN.
        let abs_min = call_builtin(BuiltinId::Abs, vec![RuntimeValue::Int(i64::MIN)]).unwrap();
        assert_eq!(abs_min.to_display_string(), "9223372036854775808");
        assert_eq!(abs_min.type_name(), "Int", "a BigInt is still an integer type");

        // 2^63 overflows i64 → exact BigInt (not the wrapped i64::MIN).
        let two_pow_63 = call_builtin(BuiltinId::Pow, vec![RuntimeValue::Int(2), RuntimeValue::Int(63)]).unwrap();
        assert_eq!(two_pow_63.to_display_string(), "9223372036854775808");

        // A far-larger power stays exact (2^100 = 31 digits).
        let two_pow_100 = call_builtin(BuiltinId::Pow, vec![RuntimeValue::Int(2), RuntimeValue::Int(100)]).unwrap();
        assert_eq!(two_pow_100.to_display_string(), "1267650600228229401496703205376");

        // In-range results stay narrow `Int` (downsizing is automatic).
        assert!(matches!(
            call_builtin(BuiltinId::Pow, vec![RuntimeValue::Int(3), RuntimeValue::Int(4)]).unwrap(),
            RuntimeValue::Int(81)
        ));
        // abs of an ordinary negative is unchanged.
        assert!(matches!(
            call_builtin(BuiltinId::Abs, vec![RuntimeValue::Int(-5)]).unwrap(),
            RuntimeValue::Int(5)
        ));
    }

    #[test]
    fn copy_is_deep() {
        use std::cell::RefCell;
        let inner = std::rc::Rc::new(RefCell::new(
            crate::interpreter::ListRepr::from_values(vec![RuntimeValue::Int(1)]),
        ));
        let original = RuntimeValue::List(inner.clone());
        let copied = call_builtin(BuiltinId::Copy, vec![original]).unwrap();
        if let RuntimeValue::List(copied_items) = &copied {
            inner.borrow_mut().push(RuntimeValue::Int(2));
            assert_eq!(copied_items.borrow().len(), 1, "copy must not share the allocation");
        } else {
            panic!("copy changed the type");
        }
    }
}
