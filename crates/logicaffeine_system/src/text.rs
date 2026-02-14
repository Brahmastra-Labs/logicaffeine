#[inline]
pub fn parseInt(s: String) -> i64 {
    s.trim()
        .parse::<i64>()
        .unwrap_or_else(|_| panic!("Cannot parse '{}' as Int", s))
}

#[inline]
pub fn parseFloat(s: String) -> f64 {
    s.trim()
        .parse::<f64>()
        .unwrap_or_else(|_| panic!("Cannot parse '{}' as Float", s))
}
