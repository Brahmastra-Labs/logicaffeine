use std::env;

fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b > 0 { let t = b; b = a % b; a = t; }
    a
}

fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut sum: i64 = 0;
    for i in 1..=n {
        for j in i..=n {
            sum += gcd(i, j);
        }
    }
    println!("{}", sum);
}
