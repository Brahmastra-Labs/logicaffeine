use std::env;
fn main() {
    let reps: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let base = "abcABC012 ".repeat(400);
    let base_b = base.as_bytes();
    let mut s: Vec<u8> = Vec::with_capacity(base_b.len() + 200);
    let mut checksum: i64 = 0;
    for r in 0..reps {
        let pre = (r % 100) as usize;
        s.clear();
        s.resize(pre, b'a');
        s.extend_from_slice(base_b);
        if let Some(idx) = s.iter().position(|&c| c == 48) { checksum += idx as i64 + 1; }
    }
    println!("{}", checksum);
}
