use std::env;

fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut count: i64 = 0;
    for i in 2..=n {
        let mut is_prime = true;
        let mut d: i64 = 2;
        while d * d <= i {
            if i % d == 0 { is_prime = false; break; }
            d += 1;
        }
        if is_prime { count += 1; }
    }
    println!("{}", count);
}
