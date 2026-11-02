use std::env;

fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut s: i64 = 0;
    for i in 1..=n { let h: std::collections::BinaryHeap<i64> = std::collections::BinaryHeap::new(); s += h.len() as i64; }
    println!("{}", s);
}
