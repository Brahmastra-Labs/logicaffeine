use std::env;
fn main() {
    let reps: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let base = "abcABC012 ".repeat(400);
    let mut checksum: i64 = 0;
    for r in 0..reps {
        let s = "a".repeat((r as usize) % 100) + &base;
        checksum += s.split('0').count() as i64;
    }
    println!("{}", checksum);
}
