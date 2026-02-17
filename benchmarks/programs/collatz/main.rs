use std::env;

fn main() {
    let n: i64 = env::args().nth(1).unwrap().parse().unwrap();
    let mut total: i64 = 0;
    for i in 1..=n {
        let mut k = i;
        while k != 1 {
            if k % 2 == 0 { k /= 2; } else { k = 3 * k + 1; }
            total += 1;
        }
    }
    println!("{}", total);
}
