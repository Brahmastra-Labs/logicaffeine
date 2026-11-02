use std::env;

fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut s: i64 = 0;
    for i in 1..=n { s += (i - 10000000).signum(); }
    println!("{}", s);
}
