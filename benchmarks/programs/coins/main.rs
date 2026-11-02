use std::env;
fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let coins = [1usize, 5, 10, 25, 50, 100];
    let mut dp = vec![0i64; n + 1];
    dp[0] = 1;
    for &c in &coins { for j in c..=n { dp[j] = (dp[j] + dp[j - c]) % 1000000007; } }
    let mut checksum: i64 = 0;
    for &v in &dp { checksum = (checksum * 31 + v) & 268435455; }
    println!("{} {}", dp[n], checksum);
}
