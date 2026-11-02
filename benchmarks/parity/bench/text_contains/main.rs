use std::env;
fn main() {
    let reps: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let base = "abcABC012 ".repeat(400);
    let sub = "012";
    let mut checksum: i64 = 0;
    for r in 0..reps {
        let s = "a".repeat((r as usize) % 100) + &base;
        if s.contains(sub) { checksum += 1; }
    }
    println!("{}", checksum);
}
