use std::env;
fn main() {
    let reps: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let m: usize = 4096;
    let mut xs: Vec<i64> = (0..m).map(|k| ((k as i64) * 2654435761) % 1048576).collect();
    xs.sort();
    let mut checksum: i64 = 0;
    for r in 0..reps {
        let t = r % 1048576; match xs.binary_search(&t) { Ok(i) => checksum += i as i64 + 1, Err(_) => {} }
    }
    println!("{}", checksum);
}
