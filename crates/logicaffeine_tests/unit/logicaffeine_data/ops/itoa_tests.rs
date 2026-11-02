use super::LogosStrPushInt;

/// `push_i64` must be byte-identical to `Display` for every value, including
/// the `i64::MIN` edge that LOGOS itself can't express as a literal (the
/// `unsigned_abs` there is the whole reason the hand itoa is subtle).
#[test]
fn push_i64_matches_display_across_edges() {
    for &v in &[
        0i64,
        1,
        -1,
        9,
        -9,
        10,
        -10,
        99,
        -99,
        100,
        42,
        -42,
        12_345,
        -12_345,
        1_000_000_000,
        -1_000_000_000,
        i64::MAX,
        i64::MIN,
        i64::MIN + 1,
    ] {
        let mut s = String::from("[");
        s.push_i64(v);
        s.push(']');
        assert_eq!(s, format!("[{v}]"), "push_i64({v}) diverged from Display");
    }
}

/// `push_i64` appends exactly what `Display` writes, for values of every digit count and both signs, after text
/// already in the string, whether or not the string has room for the digits (the digits are written straight into
/// its spare capacity, so a string grown mid-number would lose them).
#[test]
fn push_i64_appends_display_at_every_length_with_and_without_room() {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut values = vec![i64::MIN, i64::MAX, 0, -1, 1];
    for digits in 1..=19u32 {
        let low = if digits == 1 { 0 } else { 10i64.pow(digits - 1) };
        let high = if digits == 19 { i64::MAX } else { 10i64.pow(digits) - 1 };
        values.extend([low, high, -low, -high, low + 1, high - 1]);
    }
    for _ in 0..20_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let shift = x % 64;
        values.push((x as i64) >> shift);
    }
    for v in values {
        for room in [0usize, 1, 64] {
            let mut s = String::with_capacity(0);
            s.push_str("ab");
            s.shrink_to_fit();
            s.reserve_exact(room);
            s.push_i64(v);
            s.push('|');
            assert_eq!(s, format!("ab{v}|"), "push_i64({v}) with {room} spare bytes");
        }
    }
}
