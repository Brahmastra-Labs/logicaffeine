use std::env;

fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let mut text = Vec::with_capacity(n);
    let mut pos: usize = 0;
    while pos < n {
        if pos > 0 && pos % 1000 == 0 && pos + 5 <= n {
            text.extend_from_slice(b"XXXXX");
            pos += 5;
        } else {
            text.push(b'a' + (pos % 5) as u8);
            pos += 1;
        }
    }
    let needle = b"XXXXX";
    let mut count: i64 = 0;
    for i in 0..=(n.saturating_sub(needle.len())) {
        if text[i..].starts_with(needle) { count += 1; }
    }
    println!("{}", count);
}
