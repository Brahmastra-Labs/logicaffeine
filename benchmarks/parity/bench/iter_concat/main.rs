use std::env;
fn main() {
    let reps: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let m: usize = 4096;
    let mut xs: Vec<i64> = (0..m).map(|k| ((k as i64) * 2654435761) % 1048576).collect();
    let mut ys: Vec<i64> = (0..m).map(|k| ((k as i64) * 1013904223 + 12345) % 1048576).collect();
    let mut checksum: i64 = 0;
    for r in 0..reps {
        xs[(r as usize) % m] = r % 1048576;
        let mut res = xs.clone(); res.extend_from_slice(&ys); checksum += res[(r as usize) % m];
    }
    println!("{}", checksum);
}
