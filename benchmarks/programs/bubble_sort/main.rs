fn main() {
    let n: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut arr = Vec::with_capacity(n);
    let mut seed: u32 = 42;
    for _ in 0..n {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        arr.push(((seed >> 16) & 0x7fff) as i32);
    }
    for i in 0..n.saturating_sub(1) {
        for j in 0..n - 1 - i {
            if arr[j] > arr[j + 1] {
                arr.swap(j, j + 1);
            }
        }
    }
    println!("{}", arr[0]);
}
