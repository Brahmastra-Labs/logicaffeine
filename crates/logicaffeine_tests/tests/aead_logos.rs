//! ChaCha20-Poly1305 AEAD, ORCHESTRATED IN LOGOS (`assets/std/crypto.lg` `aeadSeal`/`aeadOpen`): the
//! RFC 8439 §2.8 construction — Poly1305 one-time key from ChaCha20 counter 0, payload encrypted at
//! counter 1, the MAC over `aad ‖ pad ‖ ct ‖ pad ‖ len(aad) ‖ len(ct)`, the tag appended and
//! constant-time-compared on open — written as Logos flow (`followed by`, `length of`, mod/div byte
//! splits). The cipher (`chacha20Encrypt`) and MAC (`poly1305Mac`) are native kernels. Run through
//! the compiled-Logos (AOT) path, like the ML-KEM stdlib.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::run_logos_with_args;

fn fill(name: &str, len: usize, expr: &str) -> String {
    format!("Let mutable {name} be a new Seq of Int.\nRepeat for i from 1 to {len}:\n    Push {expr} to {name}.\n")
}

fn program() -> String {
    let mut p = String::from("## Main\n");
    p.push_str(&fill("key", 32, "(i + 6)"));
    p.push_str(&fill("nonce", 12, "(i + 64)"));
    p.push_str(&fill("aad", 11, "(i % 7)"));
    p.push_str(&fill("pt", 40, "((i * 5) % 256)"));
    // Seal → ciphertext ‖ tag (40 + 16 = 56).
    p.push_str("Let sealed be aeadSeal(key, nonce, aad, pt).\n");
    p.push_str("Show length of sealed.\n");
    // Open → recovers the 40-byte plaintext.
    p.push_str("Let opened be aeadOpen(key, nonce, aad, sealed).\n");
    p.push_str("Show length of opened.\n");
    p.push_str("Let mutable ok be 1.\n");
    p.push_str("Repeat for i from 1 to 40:\n");
    p.push_str("    Let d be (item i of opened) - (item i of pt).\n");
    p.push_str("    If d * d is at least 1:\n");
    p.push_str("        Set ok to 0.\n");
    p.push_str("Show ok.\n");
    // Tamper the first ciphertext byte ⇒ the MAC fails ⇒ open returns empty (length 0).
    p.push_str("Let mutable bad be a new Seq of Int.\n");
    p.push_str("Repeat for x in sealed:\n    Push x to bad.\n");
    p.push_str("Set item 1 of bad to (((item 1 of bad) + 1) % 256).\n");
    p.push_str("Let badOpen be aeadOpen(key, nonce, aad, bad).\n");
    p.push_str("Show length of badOpen.\n");
    p
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the COMPILED-LOGOS AEAD gate"]
fn logos_aead_seal_open_round_trips_and_rejects_tamper() {
    // Expected: 56 (ct‖tag) · 40 (recovered plaintext) · 1 (matches) · 0 (tamper rejected).
    let aot = run_logos_with_args(&program(), &[]);
    assert!(
        aot.success,
        "AOT compile+run of the Logos AEAD failed:\n--- stderr ---\n{}\n--- rust ---\n{}",
        aot.stderr, aot.rust_code
    );
    let out: Vec<&str> = aot.stdout.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    assert_eq!(
        out,
        vec!["56", "40", "1", "0"],
        "Logos aeadSeal/aeadOpen: 56-byte sealed, 40-byte round-trip, matches, tamper rejected"
    );
}
