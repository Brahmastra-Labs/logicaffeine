use std::env;

fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut sum: i64 = 0;
    for i in 1..=n {
        sum = (sum + i) % 1000000007;
    }
    println!("{}", sum);
}
