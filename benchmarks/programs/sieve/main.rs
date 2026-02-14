fn main() {
    let limit: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut sieve = vec![false; limit + 1];
    let mut count = 0u64;
    for i in 2..=limit {
        if !sieve[i] {
            count += 1;
            let mut j = i * i;
            while j <= limit {
                sieve[j] = true;
                j += i;
            }
        }
    }
    println!("{}", count);
}
