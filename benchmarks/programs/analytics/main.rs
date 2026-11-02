use std::env;

const MOD: i64 = 1000000007;
const NGROUP: usize = 32768;

fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut seed: i64 = 42;
    let mut groups = vec![0i64; NGROUP];
    let mut kept: i64 = 0;
    for _ in 0..n {
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        let key = (seed >> 16) & 0x7fff;
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        let value = (seed >> 16) & 0x7fff;
        if value % 3 == 0 {
            continue;
        }
        let mapped = (value * key + 7) % MOD;
        let g = (key % NGROUP as i64) as usize;
        groups[g] = (groups[g] + mapped) % MOD;
        kept += 1;
    }
    let mut checksum: i64 = 0;
    for g in 0..NGROUP {
        checksum = (checksum * 31 + groups[g]) & 268435455;
    }
    println!("{}\n{}", kept, checksum);
}
