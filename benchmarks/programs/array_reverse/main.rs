use std::env;

fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let mut arr = Vec::with_capacity(n);
    let mut seed: i64 = 42;
    for _ in 0..n {
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        arr.push((seed >> 16) & 0x7fff);
    }
    let mut lo = 0;
    let mut hi = n - 1;
    while lo < hi {
        arr.swap(lo, hi);
        lo += 1;
        hi -= 1;
    }
    println!("{} {} {}", arr[0], arr[n - 1], arr[n / 2]);
}
