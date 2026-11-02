fn main() {
    let limit: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut sieve = vec![false; limit + 1];
    let mut count = 0u64;
    let mut h: i64 = 0;
    for i in 2..=limit {
        h = (h * 31 + i64::from(!sieve[i])) & 268435455;
        if !sieve[i] {
            count += 1;
            let mut j = i * i;
            while j <= limit {
                sieve[j] = true;
                j += i;
            }
        }
    }
    println!("{} {}", count, h);
}
