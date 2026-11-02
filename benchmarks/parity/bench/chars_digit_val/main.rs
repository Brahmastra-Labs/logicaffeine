use std::env;

fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut s: i64 = 0;
    for i in 1..=n { let c = (i % 128) as u32; s += char::from_u32(c).unwrap().to_digit(10).map(|d| d as i64).unwrap_or(-1); }
    println!("{}", s);
}
