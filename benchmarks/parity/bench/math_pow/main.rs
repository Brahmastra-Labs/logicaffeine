use std::env;

fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut s: i64 = 0;
    for i in 1..=n { s += (((i%1000+1) as f64/100.0).powf(1.5) * 1000000.0) as i64; }
    println!("{}", s);
}
