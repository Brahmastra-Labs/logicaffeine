use std::env;

fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let capacity = n * 5;
    let mut prev = vec![0i64; capacity + 1];
    let mut curr = vec![0i64; capacity + 1];
    for i in 0..n {
        let w = (i * 17 + 3) % 50 + 1;
        let v = (i as i64 * 31 + 7) % 100 + 1;
        for j in 0..=capacity {
            curr[j] = prev[j];
            if j >= w && prev[j - w] + v > curr[j] { curr[j] = prev[j - w] + v; }
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    println!("{}", prev[capacity]);
}
