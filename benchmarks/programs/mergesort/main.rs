use std::env;

fn merge_sort(arr: &mut Vec<i64>) {
    let n = arr.len();
    if n < 2 { return; }
    let mid = n / 2;
    let mut left = arr[..mid].to_vec();
    let mut right = arr[mid..].to_vec();
    merge_sort(&mut left);
    merge_sort(&mut right);
    let (mut i, mut j, mut k) = (0, 0, 0);
    while i < left.len() && j < right.len() {
        if left[i] <= right[j] { arr[k] = left[i]; i += 1; }
        else { arr[k] = right[j]; j += 1; }
        k += 1;
    }
    while i < left.len() { arr[k] = left[i]; i += 1; k += 1; }
    while j < right.len() { arr[k] = right[j]; j += 1; k += 1; }
}

fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let mut arr = Vec::with_capacity(n);
    let mut seed: i64 = 42;
    for _ in 0..n {
        seed = (seed.wrapping_mul(1103515245) + 12345) % 2147483648;
        arr.push((seed >> 16) & 0x7fff);
    }
    merge_sort(&mut arr);
    let mut checksum: i64 = 0;
    for &v in &arr { checksum = (checksum + v) % 1000000007; }
    println!("{} {} {}", arr[0], arr[n - 1], checksum);
}
