fn ackermann(m: i64, n: i64) -> i64 {
    if m == 0 { return n + 1; }
    if n == 0 { return ackermann(m - 1, 1); }
    ackermann(m - 1, ackermann(m, n - 1))
}

fn main() {
    let m: i64 = std::env::args().nth(1).unwrap().parse().unwrap();
    println!("{}", ackermann(3, m));
}
