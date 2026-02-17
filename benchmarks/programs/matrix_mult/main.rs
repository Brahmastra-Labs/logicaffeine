use std::env;
const MOD: i64 = 1000000007;

fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let mut a = vec![0i64; n * n];
    let mut b = vec![0i64; n * n];
    let mut c = vec![0i64; n * n];
    for i in 0..n {
        for j in 0..n {
            a[i * n + j] = ((i * n + j) % 100) as i64;
            b[i * n + j] = ((j * n + i) % 100) as i64;
        }
    }
    for i in 0..n {
        for k in 0..n {
            for j in 0..n {
                c[i * n + j] = (c[i * n + j] + a[i * n + k] * b[k * n + j]) % MOD;
            }
        }
    }
    let mut checksum: i64 = 0;
    for &v in &c { checksum = (checksum + v) % MOD; }
    println!("{}", checksum);
}
