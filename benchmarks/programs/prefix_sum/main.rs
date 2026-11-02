use std::env;

fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let mut arr = Vec::with_capacity(n);
    let mut seed: i64 = 42;
    for _ in 0..n {
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        arr.push(((seed >> 16) & 0x7fff) % 1000);
    }
    for i in 1..n {
        arr[i] = (arr[i] + arr[i - 1]) % 1000000007;
    }
    let mut h: i64 = 0;
    for &v in &arr {
        h = (h * 31 + v) & 268435455;
    }
    println!("{} {}", arr[n - 1], h);
}
