//! ML-KEM-768 oracle probe + (eventually) the bit-exactness gate for the Logos-native scheme.
//! `ml-kem` (RustCrypto) is a dev-only oracle — never in the shipped graph.

#![cfg(not(target_arch = "wasm32"))]

use ml_kem::kem::Decapsulate;
use ml_kem::{B32, EncapsulateDeterministic, EncodedSizeUser, KemCore, MlKem768};

#[test]
fn oracle_deterministic_round_trip_and_sizes() {
    let d = B32::from([0x11u8; 32]);
    let z = B32::from([0x22u8; 32]);
    let (dk, ek) = MlKem768::generate_deterministic(&d, &z);

    let ek_bytes = ek.as_bytes();
    assert_eq!(ek_bytes.len(), 1184, "ML-KEM-768 encapsulation key is 1184 bytes");

    let m = B32::from([0x33u8; 32]);
    let (ct, k_send) = ek.encapsulate_deterministic(&m).expect("encaps");
    assert_eq!(ct.len(), 1088, "ML-KEM-768 ciphertext is 1088 bytes");
    assert_eq!(k_send.len(), 32, "shared secret is 32 bytes");

    let k_recv = dk.decapsulate(&ct).expect("decaps");
    assert_eq!(k_send, k_recv, "decaps recovers the encaps shared secret");
}
