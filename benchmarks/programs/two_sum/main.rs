use std::collections::HashSet;
use std::env;
fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut arr = Vec::with_capacity(n as usize);
    let mut seed: i64 = 42;
    for _ in 0..n { seed=(seed.wrapping_mul(1103515245)+12345)%2147483648; arr.push(((seed>>16)&0x7fff)%n); }
    let mut seen = HashSet::new();
    let mut count: i64 = 0;
    for &x in &arr {
        let c = n - x;
        if c >= 0 && seen.contains(&c) { count += 1; }
        seen.insert(x);
    }
    println!("{}", count);
}
