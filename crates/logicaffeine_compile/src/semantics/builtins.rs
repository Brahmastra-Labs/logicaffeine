//! Built-in functions over already-evaluated values.
//!
//! `show` is NOT here — output is an engine concern. Arity is checked by the
//! caller BEFORE evaluating arguments (via [`check_arity`]) to preserve the
//! tree-walker's error ordering: a wrong-arity call reports the arity error
//! even when an argument expression would itself error.

use std::rc::Rc;

use crate::interpreter::RuntimeValue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
        _ => return None,
    })
}

/// Check the call's arity BEFORE evaluating arguments. `format` accepts any
/// arity (it reads only its first argument, or none).
pub fn check_arity(id: BuiltinId, n: usize) -> Result<(), String> {
    let expected: usize = match id {
        BuiltinId::Format => return Ok(()),
        BuiltinId::Min | BuiltinId::Max | BuiltinId::Pow => 2,
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
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n.wrapping_abs())),
                RuntimeValue::Float(f) => Ok(RuntimeValue::Float(f.abs())),
                _ => Err(format!("abs() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Sqrt => {
            let val = args.remove(0);
            match val {
                RuntimeValue::Float(f) => Ok(RuntimeValue::Float(f.sqrt())),
                RuntimeValue::Int(n) => Ok(RuntimeValue::Float((n as f64).sqrt())),
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
            match val {
                RuntimeValue::Float(f) => Ok(RuntimeValue::Int(f.floor() as i64)),
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n)),
                _ => Err(format!("floor() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Ceil => {
            let val = args.remove(0);
            match val {
                RuntimeValue::Float(f) => Ok(RuntimeValue::Int(f.ceil() as i64)),
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n)),
                _ => Err(format!("ceil() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Round => {
            let val = args.remove(0);
            match val {
                RuntimeValue::Float(f) => Ok(RuntimeValue::Int(f.round() as i64)),
                RuntimeValue::Int(n) => Ok(RuntimeValue::Int(n)),
                _ => Err(format!("round() requires a number, got {}", val.type_name())),
            }
        }
        BuiltinId::Pow => {
            let exp = args.remove(1);
            let base = args.remove(0);
            match (&base, &exp) {
                (RuntimeValue::Int(b), RuntimeValue::Int(e)) => {
                    if *e >= 0 {
                        Ok(RuntimeValue::Int(b.wrapping_pow(*e as u32)))
                    } else {
                        Ok(RuntimeValue::Float((*b as f64).powi(*e as i32)))
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn abs_and_pow_wrap_like_the_int_spec() {
        // abs(i64::MIN) has no positive representation: it wraps to itself.
        assert!(matches!(
            call_builtin(BuiltinId::Abs, vec![RuntimeValue::Int(i64::MIN)]).unwrap(),
            RuntimeValue::Int(i64::MIN)
        ));
        // 2^63 overflows i64: wrapping_pow yields i64::MIN.
        assert!(matches!(
            call_builtin(BuiltinId::Pow, vec![RuntimeValue::Int(2), RuntimeValue::Int(63)]).unwrap(),
            RuntimeValue::Int(i64::MIN)
        ));
        // In-range pow is unchanged.
        assert!(matches!(
            call_builtin(BuiltinId::Pow, vec![RuntimeValue::Int(3), RuntimeValue::Int(4)]).unwrap(),
            RuntimeValue::Int(81)
        ));
    }

    #[test]
    fn copy_is_deep() {
        use std::cell::RefCell;
        let inner = std::rc::Rc::new(RefCell::new(vec![RuntimeValue::Int(1)]));
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
