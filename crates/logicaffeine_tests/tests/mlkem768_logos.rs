//! The end goal: ML-KEM-768 written in LOGOS (the `mlkem768Keygen`/`mlkem768Encaps`/`mlkem768Decaps`
//! orchestration in `assets/std/crypto.lg`, calling the verified native primitives) must, compiled
//! the full AOT way, reproduce the FIPS-203 result byte-for-byte. The RustCrypto `ml-kem` oracle is
//! dev-only; the shipped bytes are the Logos program's. This is the Logos counterpart of the
//! `logicaffeine_system` reference (`mlkem768_kpke.rs`) that pinned the recipe.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::run_logos_with_args;
use ml_kem::kem::{Decapsulate, Encapsulate};
use ml_kem::{B32, EncapsulateDeterministic, EncodedSizeUser, KemCore, MlKem768};

fn lit(v: &[u8]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ")
}

fn run_bytes(prog: &str, n: usize) -> Vec<u8> {
    let aot = run_logos_with_args(prog, &[]);
    assert!(aot.success, "AOT failed:\n--- stderr ---\n{}\n--- rust ---\n{}", aot.stderr, aot.rust_code);
    let got: Vec<u8> = aot
        .stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.trim().parse::<i64>().expect("byte line") as u8)
        .collect();
    assert_eq!(got.len(), n, "expected {n} output bytes");
    got
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT ML-KEM-768 keygen gate"]
fn logos_mlkem768_keygen_is_bit_exact_vs_oracle() {
    let d = [0x11u8; 32];
    let z = [0x22u8; 32];
    let (_dk, ek) = MlKem768::generate_deterministic(&B32::from(d), &B32::from(z));
    let want: Vec<u8> = ek.as_bytes().to_vec();

    let prog = format!(
        "## Main\nLet d be [{}].\nLet ek be mlkem768Keygen(d).\nRepeat for i from 1 to 1184:\n    Show item i of ek.\n",
        lit(&d)
    );
    assert_eq!(run_bytes(&prog, 1184), want, "compiled Logos ML-KEM-768 keygen must match the oracle");
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT ML-KEM-768 encaps gate"]
fn logos_mlkem768_encaps_is_bit_exact_vs_oracle() {
    let d = [0x11u8; 32];
    let z = [0x22u8; 32];
    let m = [0x33u8; 32];
    let (_dk, ek) = MlKem768::generate_deterministic(&B32::from(d), &B32::from(z));
    let ek_bytes: Vec<u8> = ek.as_bytes().to_vec();
    let (ct, k) = ek.encapsulate_deterministic(&B32::from(m)).expect("encaps");

    // mlkem768Encaps returns c ‖ K (1088 + 32 = 1120 bytes).
    let prog = format!(
        "## Main\nLet ek be [{}].\nLet m be [{}].\nLet out be mlkem768Encaps(ek, m).\nRepeat for i from 1 to 1120:\n    Show item i of out.\n",
        lit(&ek_bytes),
        lit(&m)
    );
    let out = run_bytes(&prog, 1120);
    assert_eq!(&out[..1088], ct.as_slice(), "compiled Logos encaps ciphertext must match the oracle");
    assert_eq!(&out[1088..], k.as_slice(), "compiled Logos encaps shared secret must match the oracle");
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the AOT ML-KEM-768 decaps gate"]
fn logos_mlkem768_decaps_recovers_oracle_secret() {
    let d = [0x11u8; 32];
    let z = [0x22u8; 32];
    let m = [0x33u8; 32];
    let (dk, ek) = MlKem768::generate_deterministic(&B32::from(d), &B32::from(z));
    let dk_bytes: Vec<u8> = dk.as_bytes().to_vec();
    let (ct, k) = ek.encapsulate_deterministic(&B32::from(m)).expect("encaps");
    let want: Vec<u8> = k.as_slice().to_vec();
    // Sanity: the oracle decapsulates its own ciphertext to the same secret.
    assert_eq!(dk.decapsulate(&ct).expect("decaps").as_slice(), &want[..]);

    let prog = format!(
        "## Main\nLet dk be [{}].\nLet c be [{}].\nLet k be mlkem768Decaps(dk, c).\nRepeat for i from 1 to 32:\n    Show item i of k.\n",
        lit(&dk_bytes),
        lit(ct.as_slice())
    );
    assert_eq!(run_bytes(&prog, 32), want, "compiled Logos decaps must recover the oracle shared secret");
}
