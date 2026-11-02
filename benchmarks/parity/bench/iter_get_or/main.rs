use std::env;
fn main() {
    let reps: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let m: usize = 4096;
    let mut xs: Vec<i64> = (0..m).map(|k| ((k as i64) * 2654435761) % 1048576).collect();
    let mut checksum: i64 = 0;
    for r in 0..reps {
        xs[(r as usize) % m] = r % 1048576;
        let i = (r % 6000) + 1; checksum += if i >= 1 && i <= m as i64 { xs[(i-1) as usize] } else { -1 };
    }
    println!("{}", checksum);
}
