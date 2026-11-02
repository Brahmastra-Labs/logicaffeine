use std::env;
const MOD: i64 = 1000000007;

fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let mut a = vec![0i64; n * n];
    let mut b = vec![0i64; n * n];
    let mut c = vec![0i64; n * n];
    let mut seed: i64 = 42;
    for i in 0..n * n {
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        a[i] = (seed >> 16) % 100;
    }
    for i in 0..n * n {
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        b[i] = (seed >> 16) % 100;
    }
    for i in 0..n {
        for k in 0..n {
            for j in 0..n {
                c[i * n + j] = (c[i * n + j] + a[i * n + k] * b[k * n + j]) % MOD;
            }
        }
    }
    let mut checksum: i64 = 0;
    for &v in &c { checksum = (checksum * 31 + v) & 268435455; }
    println!("{}", checksum);
}
