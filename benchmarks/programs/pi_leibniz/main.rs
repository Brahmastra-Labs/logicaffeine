use std::env;
fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut sum = 0.0f64;
    let mut sign = 1.0f64;
    for k in 0..n { sum += sign / (2.0 * k as f64 + 1.0); sign = -sign; }
    println!("{:.15}", sum * 4.0);
}
