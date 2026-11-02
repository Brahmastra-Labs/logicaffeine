use std::env;

fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let mut text = Vec::with_capacity(n);
    let mut seed: i64 = 42;
    for _ in 0..n {
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        text.push(b'a' + ((seed >> 16) % 5) as u8);
    }
    let needle = b"cabed";
    let needle_len = needle.len();
    let mut count: i64 = 0;
    let mut positions: Vec<i64> = Vec::new();
    for i in 0..(n + 1).saturating_sub(needle_len) {
        let mut matched = true;
        for j in 0..needle_len {
            if text[i + j] != needle[j] {
                matched = false;
                break;
            }
        }
        if matched { count += 1; positions.push(i as i64); }
    }
    let mut checksum: i64 = 0;
    for &p in &positions { checksum = (checksum * 31 + p) & 268435455; }
    println!("{} {}", count, checksum);
}
