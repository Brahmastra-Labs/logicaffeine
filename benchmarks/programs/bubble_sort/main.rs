fn main() {
    let n: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut arr = Vec::with_capacity(n);
    let mut seed: u32 = 42;
    for _ in 0..n {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        arr.push(((seed >> 16) & 0x7fff) as i64);
    }
    for i in 0..n.saturating_sub(1) {
        for j in 0..n - 1 - i {
            if arr[j] > arr[j + 1] {
                arr.swap(j, j + 1);
            }
        }
    }
    let mut checksum: i64 = 0;
    for &v in &arr { checksum = (checksum * 31 + v) & 268435455; }
    println!("{} {} {}", arr[0], arr[n - 1], checksum);
}
