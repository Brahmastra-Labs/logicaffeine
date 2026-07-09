// Intentionally camelCase to match the LOGOS surface builtins `parseInt`/`parseFloat`
// (codegen emits these exact names), so the non-snake-case lint is suppressed rather
// than renamed — and so compiled programs that link this crate keep a clean stderr.
#[inline]
#[allow(non_snake_case)]
pub fn parseInt(s: String) -> i64 {
    s.trim()
        .parse::<i64>()
        .unwrap_or_else(|_| panic!("Cannot parse '{}' as Int", s))
}

#[inline]
#[allow(non_snake_case)]
pub fn parseFloat(s: String) -> f64 {
    s.trim()
        .parse::<f64>()
        .unwrap_or_else(|_| panic!("Cannot parse '{}' as Float", s))
}

#[inline]
pub fn chr(code: i64) -> String {
    match char::from_u32(code as u32) {
        Some(c) => c.to_string(),
        None => panic!("Invalid character code: {}", code),
    }
}
