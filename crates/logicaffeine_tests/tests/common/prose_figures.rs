//! The figures a ledger's prose states, read back as numbers so a lock can hold each one to the
//! measurement it quotes. Included per-lock via
//! `#[path = "common/prose_figures.rs"] mod prose_figures;`.
#![allow(dead_code)]

/// Every `<number> <unit>` figure `s` states: (value, digits printed after the point). A unit
/// counts only as a whole token directly after its number (one space allowed), so `mW` is not
/// also read as `W`, and `v2 x` is not a figure. A range `a-b<unit>` states both ends.
pub fn figures(s: &str, unit: &str) -> Vec<(f64, usize)> {
    let b: Vec<char> = s.chars().collect();
    let u: Vec<char> = unit.chars().collect();
    let mut out = Vec::new();
    let number_before = |mut j: usize| -> Option<(usize, f64, usize)> {
        let stop = j;
        while j > 0 && (b[j - 1].is_ascii_digit() || b[j - 1] == '.' || b[j - 1] == ',') {
            j -= 1;
        }
        if j == stop || (j > 0 && (b[j - 1].is_alphanumeric() || b[j - 1] == '_')) {
            return None;
        }
        let tok: String = b[j..stop].iter().filter(|c| **c != ',').collect();
        let tok = tok.trim_end_matches('.');
        let v = tok.parse::<f64>().ok()?;
        Some((j, v, tok.split('.').nth(1).map_or(0, str::len)))
    };
    for i in 0..b.len() {
        let end = i + u.len();
        if end > b.len() || b[i..end] != u[..] || (end < b.len() && b[end].is_alphanumeric()) {
            continue;
        }
        let at = if i > 0 && b[i - 1] == ' ' { i - 1 } else { i };
        let Some((j, v, d)) = number_before(at) else {
            continue;
        };
        out.push((v, d));
        if j > 1 && (b[j - 1] == '-' || b[j - 1] == '–') {
            if let Some((_, lo, dl)) = number_before(j - 1) {
                out.push((lo, dl));
            }
        }
    }
    out
}

/// Does `v`, printed with `digits` after the point, round from one of `candidates`?
pub fn printed_from(v: f64, digits: usize, candidates: &[f64]) -> bool {
    let tol = 0.5 * 10f64.powi(-(digits as i32)) + 1e-9;
    candidates.iter().any(|u| (u - v).abs() <= tol)
}

/// Every string in `v`, with its JSON path.
pub fn prose(v: &serde_json::Value) -> Vec<(String, String)> {
    fn walk(v: &serde_json::Value, path: String, out: &mut Vec<(String, String)>) {
        match v {
            serde_json::Value::String(s) => out.push((path, s.clone())),
            serde_json::Value::Array(a) => a
                .iter()
                .enumerate()
                .for_each(|(i, x)| walk(x, format!("{path}[{i}]"), out)),
            serde_json::Value::Object(o) => o
                .iter()
                .for_each(|(k, x)| walk(x, format!("{path}.{k}"), out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(v, String::new(), &mut out);
    out
}

/// Every number `v` records.
pub fn numbers(v: &serde_json::Value) -> Vec<f64> {
    fn walk(v: &serde_json::Value, out: &mut Vec<f64>) {
        match v {
            serde_json::Value::Number(n) => out.extend(n.as_f64()),
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            serde_json::Value::Object(o) => o.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(v, &mut out);
    out
}
