//! The stdlib `cbd2`/`cbd3` native functions lower a LOGOS `cbd2(buf)` call to the verified
//! centered-binomial-distribution sampler in `logicaffeine_system` (the Kyber bit-trick, proven
//! equal to the bit-by-bit CBD definition). A demand-imported program that calls it must, compiled
//! the full AOT way, produce exactly what the runtime kernel produces — proving ML-KEM noise
//! sampling is reachable from compiled LOGOS. CBD output is signed (in [−η, η]), so we compare to
//! the runtime kernel as integers, not bytes.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::run_logos_with_args;

fn program(call: &str, input: &[i64]) -> String {
    let lit = input.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    format!("## Main\nLet buf be [{lit}].\nLet result be {call}.\nRepeat for i from 1 to 256:\n    Show item i of result.\n")
}

fn run_coeffs(prog: &str) -> Vec<i64> {
    let aot = run_logos_with_args(prog, &[]);
    assert!(
        aot.success,
        "AOT compile+run of CBD failed:\n--- stderr ---\n{}\n--- generated rust ---\n{}",
        aot.stderr, aot.rust_code
    );
    let got: Vec<i64> = aot
        .stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse().expect("integer output line"))
        .collect();
    assert_eq!(got.len(), 256, "256 sampled coefficients");
    got
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT CBD_2 native-kernel gate"]
fn cbd2_stdlib_compiles_to_the_runtime_kernel() {
    // η=2 consumes 128 bytes.
    let input: Vec<i64> = (0..128).map(|i| (i * 181 + 17) % 256).collect();
    let want: Vec<i64> =
        logicaffeine_system::mlkem_cbd2(&input).to_vec();
    assert!(want.iter().all(|&c| (-2..=2).contains(&c)), "CBD_2 coefficients in [−2, 2]");

    let got = run_coeffs(&program("cbd2(buf)", &input));
    assert_eq!(got, want, "compiled cbd2 must equal the verified runtime CBD sampler");
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT CBD_3 native-kernel gate"]
fn cbd3_stdlib_compiles_to_the_runtime_kernel() {
    // η=3 consumes 192 bytes.
    let input: Vec<i64> = (0..192).map(|i| (i * 149 + 31) % 256).collect();
    let want: Vec<i64> =
        logicaffeine_system::mlkem_cbd3(&input).to_vec();
    assert!(want.iter().all(|&c| (-3..=3).contains(&c)), "CBD_3 coefficients in [−3, 3]");

    let got = run_coeffs(&program("cbd3(buf)", &input));
    assert_eq!(got, want, "compiled cbd3 must equal the verified runtime CBD sampler");
}
