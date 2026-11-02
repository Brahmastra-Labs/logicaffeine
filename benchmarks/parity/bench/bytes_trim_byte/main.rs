use std::env;
fn main() {
    let reps: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let m = 4096usize;
    let mut xs = vec![0u8; m];
    for k in 0..m { xs[k] = ((k as u64).wrapping_mul(2654435761) % 256) as u8; }
    let mut checksum: i64 = 0;
    for r in 0..reps {
        xs[(r as usize) % m] = (r % 256) as u8;
        let mut lo = 0usize; while lo < xs.len() && xs[lo] == 0 { lo += 1; }
        let mut hi = xs.len(); while hi > lo && xs[hi-1] == 0 { hi -= 1; }
        checksum += (hi - lo) as i64;
    }
    println!("{}", checksum);
}
