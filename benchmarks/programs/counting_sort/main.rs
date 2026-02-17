use std::env;

fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let mut arr = Vec::with_capacity(n);
    let mut seed: i64 = 42;
    for _ in 0..n {
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        arr.push((seed >> 16) % 1000);
    }
    let mut counts = [0i64; 1000];
    for &v in &arr { counts[v as usize] += 1; }
    let mut sorted = Vec::with_capacity(n);
    for v in 0..1000 {
        for _ in 0..counts[v] { sorted.push(v as i64); }
    }
    let mut checksum: i64 = 0;
    for &v in &sorted { checksum = (checksum + v) % 1000000007; }
    println!("{} {} {}", sorted[0], sorted[n - 1], checksum);
}
