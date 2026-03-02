use std::env;

fn sift_down(arr: &mut [i64], start: usize, end: usize) {
    let mut root = start;
    while 2 * root + 1 <= end {
        let child = 2 * root + 1;
        let mut swap = root;
        if arr[swap] < arr[child] { swap = child; }
        if child + 1 <= end && arr[swap] < arr[child + 1] { swap = child + 1; }
        if swap == root { return; }
        arr.swap(root, swap);
        root = swap;
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
    let mut start = (n as isize - 2) / 2;
    while start >= 0 { sift_down(&mut arr, start as usize, n - 1); start -= 1; }
    for end in (1..n).rev() {
        arr.swap(0, end);
        sift_down(&mut arr, 0, end - 1);
    }
    let mut checksum: i64 = 0;
    for &v in &arr { checksum = (checksum + v) % 1000000007; }
    println!("{} {} {}", arr[0], arr[n - 1], checksum);
}
