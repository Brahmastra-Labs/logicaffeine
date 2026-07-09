//! The stdlib ML-KEM serialization native functions (`byteEncode`/`byteDecode`/`compress`/
//! `decompress`) lower a LOGOS call to the FIPS-203 §4.2.1 kernels in `logicaffeine_system`.
//! A compiled program must (a) round-trip a polynomial through ByteEncode/ByteDecode exactly and
//! (b) produce the same Compress output as the verified runtime kernel — proving ML-KEM's
//! public-key/ciphertext encoding is reachable from compiled LOGOS.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::run_logos_with_args;

fn coeffs_out(prog: &str) -> Vec<i64> {
    let aot = run_logos_with_args(prog, &[]);
    assert!(
        aot.success,
        "AOT compile+run failed:\n--- stderr ---\n{}\n--- generated rust ---\n{}",
        aot.stderr, aot.rust_code
    );
    aot.stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse().expect("integer output line"))
        .collect()
}

fn lit(input: &[i64]) -> String {
    input.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ")
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT ByteEncode/ByteDecode round-trip gate"]
fn byte_encode_decode_round_trips_in_compiled_logos() {
    // d = 12 (the public-key encoding): coefficients in [0, q).
    let input: Vec<i64> = (0..256).map(|i| (i * 13 + 7) % 3329).collect();
    let prog = format!(
        "## Main\nLet coeffs be [{}].\nLet bytes be byteEncode(coeffs, 12).\nLet back be byteDecode(bytes, 12).\nRepeat for i from 1 to 256:\n    Show item i of back.\n",
        lit(&input)
    );
    let got = coeffs_out(&prog);
    assert_eq!(got.len(), 256, "256 coefficients survive the round-trip");
    assert_eq!(got, input, "compiled byteDecode∘byteEncode (d=12) must be the identity");
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT Compress native-kernel gate"]
fn compress_in_compiled_logos_matches_runtime() {
    // d = 10 (ML-KEM-768 ciphertext u). Compress maps [0, q) → [0, 1024).
    let input: Vec<i64> = (0..256).map(|i| (i * 101 + 5) % 3329).collect();
    let want: Vec<i64> =
        logicaffeine_system::mlkem_compress(&input, 10)
            .to_vec();
    assert!(want.iter().all(|&c| (0..1024).contains(&c)), "Compress_10 ∈ [0, 1024)");

    let prog = format!(
        "## Main\nLet coeffs be [{}].\nLet c be compress(coeffs, 10).\nRepeat for i from 1 to 256:\n    Show item i of c.\n",
        lit(&input)
    );
    let got = coeffs_out(&prog);
    assert_eq!(got, want, "compiled compress(coeffs, 10) must equal the verified runtime kernel");
}
