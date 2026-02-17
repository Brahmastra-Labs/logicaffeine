use std::env;

fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut a: i64 = 0;
    let mut b: i64 = 1;
    for _ in 0..n {
        let temp = b;
        b = (a + b) % 1000000007;
        a = temp;
    }
    println!("{}", a);
}
