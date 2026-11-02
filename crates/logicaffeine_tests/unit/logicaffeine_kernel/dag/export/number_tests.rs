use super::Cur;

/// What the general path accepts: 1 to 19 decimal digits, no leading zero unless the number
/// is `0`, ended by any non-digit or by the end of the input.
fn reference(b: &[u8], at: usize) -> Option<(u64, usize)> {
    let digits = b[at..].iter().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 || digits > 19 || (digits > 1 && b[at] == b'0') {
        return None;
    }
    let v = std::str::from_utf8(&b[at..at + digits])
        .unwrap()
        .parse::<u64>()
        .ok()?;
    Some((v, at + digits))
}

fn check(b: &[u8], at: usize) {
    let mut c = Cur { b, i: at };
    let got = c.num().map(|v| (v, c.i));
    assert_eq!(
        got,
        reference(b, at),
        "input {:?} at {at}",
        String::from_utf8_lossy(b)
    );
}

#[test]
fn every_length_and_terminator_agrees_with_the_reference() {
    let digits = b"98765432109876543210123";
    for len in 1..=22 {
        for lead in [b'1', b'0', b'9'] {
            for term in [
                &b","[..],
                b"}",
                b"]",
                b"\n",
                b"",
                b"},\"ie\":7}\n",
                b"\"",
                b"-",
            ] {
                let mut b = vec![lead];
                b.extend_from_slice(&digits[..len - 1]);
                b.extend_from_slice(term);
                check(&b, 0);
                let mut shifted = b"{\"x\":".to_vec();
                shifted.extend_from_slice(&b);
                check(&shifted, 5);
            }
        }
    }
}

#[test]
fn zero_and_non_numbers_agree_with_the_reference() {
    for s in [
        &b"0"[..],
        b"0,",
        b"00",
        b"01",
        b"0}",
        b"x",
        b"",
        b",1",
        b"-1",
        b" 1",
        b"12a",
    ] {
        check(s, 0);
    }
}

#[test]
fn pseudo_random_numbers_agree_with_the_reference() {
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for _ in 0..200_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let v = x >> (x % 64);
        let mut b = v.to_string().into_bytes();
        let tails: [&[u8]; 4] = [b",\"fn\":12}", b"}", b"", b"]},\"ie\":3}\n"];
        b.extend_from_slice(tails[(x % 4) as usize]);
        check(&b, 0);
    }
}
