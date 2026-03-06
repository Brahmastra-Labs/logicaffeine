use std::env;

fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let mut arr = Vec::with_capacity(n);
    for i in 0..n {
        arr.push((i as i64 * 7 + 3) % 1000000);
    }
    let mut sum: i64 = 0;
    for &v in &arr {
        sum = (sum + v) % 1000000007;
    }
    println!("{}", sum);
}
