mod common;

use logicaffeine_compile::compile::interpret_program;

fn read_benchmark(name: &str) -> String {
    let path = format!("../../benchmarks/programs/{}/interp.lg", name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", path, e))
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
    assert_eq!(result.unwrap().trim(), "1229");
}

#[test]
fn benchmark_interp_collect() {
    let source = read_benchmark("collect");
    let result = interpret_program(&source);
    assert!(result.is_ok(), "collect interp failed: {:?}", result);
    assert_eq!(result.unwrap().trim(), "1000");
}

#[test]
fn benchmark_interp_strings() {
    let source = read_benchmark("strings");
    let result = interpret_program(&source);
    assert!(result.is_ok(), "strings interp failed: {:?}", result);
    assert_eq!(result.unwrap().trim(), "1000");
}

#[test]
fn benchmark_interp_bubble_sort() {
    let source = read_benchmark("bubble_sort");
    let result = interpret_program(&source);
    assert!(result.is_ok(), "bubble_sort interp failed: {:?}", result);
    // bubble_sort outputs the first (smallest) element — deterministic PRNG
    let output = result.unwrap();
    assert!(!output.trim().is_empty(), "bubble_sort should produce output");
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
