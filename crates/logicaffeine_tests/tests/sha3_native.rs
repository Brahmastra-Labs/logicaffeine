//! The SHA-3 / SHAKE stdlib native functions lower a LOGOS `sha3_256(a)` / `shake256(a, n)` call
//! to the KAT-verified Keccak kernel in `logicaffeine_system`. A compiled program must produce
//! the NIST FIPS-202 output — proving the symmetric/hash layer is reachable from compiled LOGOS.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::run_logos_with_args;

fn run_bytes(prog: &str, n: usize) -> Vec<u8> {
    let aot = run_logos_with_args(prog, &[]);
    assert!(aot.success, "AOT failed:\n{}\n{}", aot.stderr, aot.rust_code);
    let got: Vec<u8> = aot
        .stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse::<i64>().expect("byte") as u8)
        .collect();
    assert_eq!(got.len(), n, "expected {n} output bytes");
    got
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT SHA3-256 KAT gate"]
fn sha3_256_in_compiled_logos_matches_nist_kat() {
    // "abc" = bytes 97, 98, 99.
    let prog = "## Main\nLet a be [97, 98, 99].\nLet h be sha3_256(a).\nRepeat for i from 1 to 32:\n    Show item i of h.\n";
    let got = run_bytes(prog, 32);
    assert_eq!(
        hex(&got),
        "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532",
        "compiled Logos SHA3-256(\"abc\") must equal the NIST KAT"
    );
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT SHAKE256 gate"]
fn shake256_in_compiled_logos_matches_runtime() {
    let input = vec![97i64, 98, 99]; // "abc"
    let want: Vec<u8> = logicaffeine_system::shake256(&input, 64)
        .borrow()
        .iter()
        .map(|&x| x as u8)
        .collect();
    let prog = "## Main\nLet a be [97, 98, 99].\nLet h be shake256(a, 64).\nRepeat for i from 1 to 64:\n    Show item i of h.\n";
    let got = run_bytes(prog, 64);
    assert_eq!(hex(&got), hex(&want), "compiled Logos SHAKE256(\"abc\", 64) must equal the KAT-verified kernel");
}
