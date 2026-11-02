use std::env;
fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut s: i64 = 0;
    for i in 1..=n { let c = (i % 128) as u8 as char; if c.is_ascii_punctuation() { s += 1; } }
    println!("{}", s);
}
