//! Interpolated-string format specifiers (`{x$}`, `{x.2}`, `{x>8}`, …).

use crate::interpreter::RuntimeValue;

/// Apply a format spec to a value.
pub fn apply_format_spec(val: &RuntimeValue, spec: &str) -> String {
    // Currency: $
    if spec == "$" {
        // A BigInt is an exact integer number of units — print it exactly with cents,
        // rather than routing through a lossy f64.
        if let RuntimeValue::BigInt(b) = val {
            return format!("${}.00", b);
        }
        let f = match val {
            RuntimeValue::Float(f) => *f,
            RuntimeValue::Int(n) => *n as f64,
            _ => return format!("${}", val.to_display_string()),
        };
        return format!("${:.2}", f);
    }
    // Precision: .N
    if spec.starts_with('.') {
        if let Ok(precision) = spec[1..].parse::<usize>() {
            match val {
                RuntimeValue::Float(f) => return format!("{:.prec$}", f, prec = precision),
                RuntimeValue::Int(n) => return format!("{:.prec$}", *n as f64, prec = precision),
                RuntimeValue::BigInt(b) => return format!("{:.prec$}", b.to_f64(), prec = precision),
                _ => return val.to_display_string(),
            }
        }
    }
    // Alignment: >N, <N, ^N
    if spec.len() >= 2 {
        let first = spec.as_bytes()[0];
        if first == b'>' || first == b'<' || first == b'^' {
            if let Ok(width) = spec[1..].parse::<usize>() {
                let s = val.to_display_string();
                return match first {
                    b'>' => format!("{:>w$}", s, w = width),
                    b'<' => format!("{:<w$}", s, w = width),
                    b'^' => format!("{:^w$}", s, w = width),
                    _ => unreachable!(),
                };
            }
        }
    }
    // Bare width: N (right-align by default, matching Rust's behavior)
    if let Ok(width) = spec.parse::<usize>() {
        let s = val.to_display_string();
        return format!("{:>w$}", s, w = width);
    }
    val.to_display_string()
}
