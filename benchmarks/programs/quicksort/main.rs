use std::env;

fn partition(arr: &mut [i64], lo: usize, hi: usize) -> usize {
    let pivot = arr[hi];
    let mut i = lo;
    for j in lo..hi {
        if arr[j] <= pivot { arr.swap(i, j); i += 1; }
    }
    arr.swap(i, hi);
    i
}

fn qs(arr: &mut [i64], lo: isize, hi: isize) {
    if lo < hi {
        let p = partition(arr, lo as usize, hi as usize);
        qs(arr, lo, p as isize - 1);
        qs(arr, p as isize + 1, hi);
    }
}

fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let mut arr = Vec::with_capacity(n);
    let mut seed: i64 = 42;
    for _ in 0..n {
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        arr.push((seed >> 16) & 0x7fff);
    }
    qs(&mut arr, 0, n as isize - 1);
    let mut checksum: i64 = 0;
    for &v in &arr { checksum = (checksum + v) % 1000000007; }
    println!("{} {} {}", arr[0], arr[n - 1], checksum);
}
