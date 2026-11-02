mod common;

use logicaffeine_compile::compile::interpret_program;

fn read_benchmark(name: &str) -> String {
    let path = format!("../../benchmarks/programs/{}/interp.lg", name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("Failed to read {}: {}", path, e))
}

#[test]
fn benchmark_interp_fib() {
    let source = read_benchmark("fib");
    let result = interpret_program(&source);
    assert!(result.is_ok(), "fib interp failed: {:?}", result);
    assert_eq!(result.unwrap().trim(), "9227465");
}

#[test]
fn benchmark_interp_sieve() {
    let source = read_benchmark("sieve");
    let result = interpret_program(&source);
    assert!(result.is_ok(), "sieve interp failed: {:?}", result);
    assert_eq!(result.unwrap().trim(), sieve_answer(10_000));
}

/// The primes up to `limit` and the fold of each number's primality flag, computed here in plain Rust.
fn sieve_answer(limit: usize) -> String {
    let mut composite = vec![false; limit + 1];
    let (mut count, mut h) = (0u64, 0u64);
    for i in 2..=limit {
        let prime = !composite[i];
        if prime {
            count += 1;
            let mut j = i * i;
            while j <= limit {
                composite[j] = true;
                j += i;
            }
        }
        h = (h * 31 + prime as u64) & 268_435_455;
    }
    format!("{count} {h}")
}

#[test]
fn benchmark_interp_collect() {
    let source = read_benchmark("collect");
    let result = interpret_program(&source);
    assert!(result.is_ok(), "collect interp failed: {:?}", result);
    assert_eq!(result.unwrap().trim(), collect_answer(1000));
}

/// `n` keys from the benchmark's LCG stored as `key → 2·key`, then `n` lookups that hit (the same sequence) and `n`
/// that mostly miss (the sequence continued), folding every value found — computed here in plain Rust.
fn collect_answer(n: usize) -> String {
    let next = |s: i64| (s * 1_103_515_245 + 12_345) % 2_147_483_648;
    let mut m = std::collections::HashMap::new();
    let mut seed = 42i64;
    for _ in 0..n {
        seed = next(seed);
        m.insert(seed, seed * 2);
    }
    let (mut hit, mut miss, mut found, mut h) = (42i64, seed, 0u64, 0i64);
    for _ in 0..n {
        hit = next(hit);
        if let Some(v) = m.get(&hit) {
            found += 1;
            h = (h * 31 + v) & 268_435_455;
        }
        miss = next(miss);
        if let Some(v) = m.get(&miss) {
            found += 1;
            h = (h * 31 + v) & 268_435_455;
        }
    }
    format!("{found} {h}")
}

#[test]
fn benchmark_interp_strings() {
    let source = read_benchmark("strings");
    let result = interpret_program(&source);
    assert!(result.is_ok(), "strings interp failed: {:?}", result);
    assert_eq!(result.unwrap().trim(), strings_answer(1000));
}

/// `"0 1 2 … (n-1) "`: its spaces, and the fold of its bytes — computed here in plain Rust.
fn strings_answer(n: usize) -> String {
    let text: String = (0..n).map(|i| format!("{i} ")).collect();
    let spaces = text.bytes().filter(|&b| b == b' ').count();
    let h = text
        .bytes()
        .fold(0u64, |h, b| (h * 31 + b as u64) & 268_435_455);
    format!("{spaces} {h}")
}

#[test]
fn benchmark_interp_bubble_sort() {
    let source = read_benchmark("bubble_sort");
    let result = interpret_program(&source);
    assert!(result.is_ok(), "bubble_sort interp failed: {:?}", result);
    // bubble_sort outputs the first (smallest) element — deterministic PRNG
    let output = result.unwrap();
    assert!(
        !output.trim().is_empty(),
        "bubble_sort should produce output"
    );
}

#[test]
fn benchmark_interp_ackermann() {
    let source = read_benchmark("ackermann");
    // ackermann(3,3) requires ~2500 recursive interpreter calls;
    // spawn on a larger stack to avoid overflow on the default 8 MB test thread.
    let result = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(move || interpret_program(&source))
        .unwrap()
        .join()
        .unwrap();
    assert!(result.is_ok(), "ackermann interp failed: {:?}", result);
    assert_eq!(result.unwrap().trim(), "61");
}
