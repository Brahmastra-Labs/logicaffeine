//! The authenticated post-quantum handshake, ORCHESTRATED IN LOGOS (`assets/std/crypto.lg`
//! `pqAuthHandshake`): an ephemeral ML-KEM-768 key exchange bound to two long-term ML-DSA-65
//! identities by signing the transcript. The heavy crypto (ML-KEM/ML-DSA keygen, sign, verify,
//! encaps, decaps) are demand-imported native kernels; the protocol FLOW — transcript construction
//! (`followed by`), the sign/verify gating, the shared-secret derivation — is Logos. The handshake
//! must agree on the shared secret AND reject a man-in-the-middle who can't sign as the responder.
//! Exercised through the COMPILED-LOGOS (AOT) path, the same way the ML-KEM stdlib is gated.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::run_logos_with_args;

/// A Logos fragment that fills a 32-byte seed `name` with `val`.
fn seed(name: &str, val: u8) -> String {
    format!(
        "Let mutable {name} be a new Seq of Int.\nRepeat for i from 1 to 32:\n    Push {val} to {name}.\n"
    )
}

fn program() -> String {
    let mut p = String::from("## Main\n");
    p.push_str(&seed("seedI", 1));
    p.push_str(&seed("seedR", 2));
    p.push_str(&seed("kemD", 16));
    p.push_str(&seed("kemZ", 32));
    p.push_str(&seed("kemM", 48));
    p.push_str("Let ctx be a new Seq of Int.\n");
    // Two long-term identities (ML-DSA pk‖sk = 1952‖4032).
    p.push_str("Let idi be mldsaKeypairLogos(seedI).\n");
    p.push_str("Let idiPk be mlkemSubseq(idi, 0, 1952).\n");
    p.push_str("Let idiSk be mlkemSubseq(idi, 1952, 4032).\n");
    p.push_str("Let idr be mldsaKeypairLogos(seedR).\n");
    p.push_str("Let idrPk be mlkemSubseq(idr, 0, 1952).\n");
    p.push_str("Let idrSk be mlkemSubseq(idr, 1952, 4032).\n");
    // Honest handshake → ssI ‖ ssR (64 bytes); the two halves must agree.
    p.push_str("Let ss be pqAuthHandshake(idiPk, idiSk, idrPk, idrSk, kemD, kemZ, kemM, ctx).\n");
    p.push_str("Show length of ss.\n");
    p.push_str("Let mutable agree be 1.\n");
    p.push_str("Repeat for i from 1 to 32:\n");
    p.push_str("    Let d be (item i of ss) - (item (32 + i) of ss).\n");
    p.push_str("    If d * d is at least 1:\n");
    p.push_str("        Set agree to 0.\n");
    p.push_str("Show agree.\n");
    // MITM: the responder signs with the WRONG key (idiSk), but the initiator verifies against the
    // real idrPk ⇒ the handshake returns empty (length 0).
    p.push_str("Let mitm be pqAuthHandshake(idiPk, idiSk, idrPk, idiSk, kemD, kemZ, kemM, ctx).\n");
    p.push_str("Show length of mitm.\n");
    p
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the COMPILED-LOGOS PQ handshake gate"]
fn logos_authenticated_pq_handshake_agrees_and_defeats_mitm() {
    // Expected: 64 (agreed shared secret, ssI‖ssR) · 1 (the halves match) · 0 (MITM rejected).
    let aot = run_logos_with_args(&program(), &[]);
    assert!(
        aot.success,
        "AOT compile+run of the Logos PQ handshake failed:\n--- stderr ---\n{}\n--- rust ---\n{}",
        aot.stderr, aot.rust_code
    );
    let out: Vec<&str> = aot.stdout.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
    assert_eq!(
        out,
        vec!["64", "1", "0"],
        "Logos pqAuthHandshake: 64-byte agreed secret, halves match, MITM rejected"
    );
}
