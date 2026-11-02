use std::env;

fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let capacity = n * 5;
    let mut weight: Vec<i64> = Vec::with_capacity(n);
    let mut value: Vec<i64> = Vec::with_capacity(n);
    let mut seed: i64 = 42;
    for _ in 0..n {
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        weight.push((seed >> 16) % 50 + 1);
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        value.push((seed >> 16) % 100 + 1);
    }
    let mut prev = vec![0i64; capacity + 1];
    let mut curr = vec![0i64; capacity + 1];
    for i in 0..n {
        let w = weight[i] as usize;
        let v = value[i];
        for j in 0..=capacity {
            curr[j] = prev[j];
            if j >= w && prev[j - w] + v > curr[j] { curr[j] = prev[j - w] + v; }
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    let mut h: i64 = 0;
    for j in 0..=capacity {
        h = (h * 31 + prev[j]) & 268435455;
    }
    println!("{} {}", prev[capacity], h);
}
