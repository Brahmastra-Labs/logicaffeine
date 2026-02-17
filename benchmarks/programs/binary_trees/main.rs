use std::env;
fn make_check(d: i32) -> i64 { if d == 0 { 1 } else { 1 + make_check(d-1) + make_check(d-1) } }
fn main() {
    let n: i32 = env::args().nth(1).unwrap().parse().unwrap();
    let mn = 4; let mx = if mn + 2 > n { mn + 2 } else { n };
    println!("stretch tree of depth {}\t check: {}", mx+1, make_check(mx+1));
    let ll = make_check(mx);
    let mut d = mn;
    while d <= mx {
        let it = 1 << (mx - d + mn);
        let tc: i64 = (0..it).map(|_| make_check(d)).sum();
        println!("{}\t trees of depth {}\t check: {}", it, d, tc);
        d += 2;
    }
    println!("long lived tree of depth {}\t check: {}", mx, ll);
}
