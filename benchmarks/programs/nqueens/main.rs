use std::env;

fn solve(row: i32, cols: i32, diag1: i32, diag2: i32, n: i32) -> i32 {
    if row == n { return 1; }
    let mut count = 0;
    let mut available = ((1 << n) - 1) & !(cols | diag1 | diag2);
    while available != 0 {
        let bit = available & (-available);
        available ^= bit;
        count += solve(row + 1, cols | bit, (diag1 | bit) << 1, (diag2 | bit) >> 1, n);
    }
    count
}

fn main() {
    let n: i32 = env::args().nth(1).unwrap().parse().unwrap();
    println!("{}", solve(0, 0, 0, 0, n));
}
