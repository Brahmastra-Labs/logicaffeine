use std::env;

fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut counts = [0i64; 1000];
    let mut seed: i64 = 42;
    for _ in 0..n {
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        counts[(((seed >> 16) & 0x7fff) % 1000) as usize] += 1;
    }
    let mut max_freq: i64 = 0;
    let mut max_idx: i64 = 0;
    let mut distinct: i64 = 0;
    for i in 0..1000 {
        if counts[i] > 0 { distinct += 1; }
        if counts[i] > max_freq { max_freq = counts[i]; max_idx = i as i64; }
    }
    println!("{} {} {}", max_freq, max_idx, distinct);
}
