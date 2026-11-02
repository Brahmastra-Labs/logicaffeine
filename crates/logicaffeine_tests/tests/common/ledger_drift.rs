//! A committed JSON ledger against the one its generator writes now.

/// The first path where two JSON values part: structure and text exactly, numbers to 1e-12 relative — serde_json
/// parses a float to within an ulp of what it printed, not always to the same bits.
pub fn first_difference(a: &serde_json::Value, b: &serde_json::Value, at: String) -> Option<String> {
    use serde_json::Value::*;
    match (a, b) {
        (Number(x), Number(y)) => {
            let (x, y) = (x.as_f64()?, y.as_f64()?);
            ((x - y).abs() > 1e-12 * x.abs().max(y.abs())).then_some(at)
        }
        (Array(x), Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{at} (length {} vs {})", x.len(), y.len()));
            }
            x.iter().zip(y).enumerate().find_map(|(i, (x, y))| first_difference(x, y, format!("{at}/{i}")))
        }
        (Object(x), Object(y)) => {
            if x.len() != y.len() || x.keys().any(|k| !y.contains_key(k)) {
                return Some(format!("{at} (keys)"));
            }
            x.iter().find_map(|(k, v)| first_difference(v, &y[k], format!("{at}/{k}")))
        }
        _ => (a != b).then_some(at),
    }
}
