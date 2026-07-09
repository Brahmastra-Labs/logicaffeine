//! FIPS-203 conformance for the `logicaffeine_system::mlkem` Rust module (the post-quantum channel's
//! handshake): its keygen / encaps / decaps must be byte-identical to the RustCrypto `ml-kem`
//! reference. This proves the channel speaks standards-compliant ML-KEM-768, not merely a
//! self-consistent KEM.

use logicaffeine_system::mlkem;
use ml_kem::kem::Decapsulate;
use ml_kem::{B32, EncapsulateDeterministic, EncodedSizeUser, KemCore, MlKem768};

#[test]
fn keygen_encaps_decaps_are_bit_exact_vs_fips203_oracle() {
    let d = [0x11u8; 32];
    let z = [0x22u8; 32];
    let m = [0x33u8; 32];

    // Keygen: ek (and dk) byte-identical to the FIPS-203 oracle.
    let (ek, dk) = mlkem::keygen(&d, &z);
    let (dk_o, ek_o) = MlKem768::generate_deterministic(&B32::from(d), &B32::from(z));
    assert_eq!(ek, ek_o.as_bytes().to_vec(), "ek must match the FIPS-203 oracle");
    assert_eq!(dk, dk_o.as_bytes().to_vec(), "dk must match the FIPS-203 oracle");

    // Encaps: ciphertext + shared secret byte-identical to the oracle.
    let (ct, ss) = mlkem::encaps(&ek, &m);
    let (ct_o, ss_o) = ek_o.encapsulate_deterministic(&B32::from(m)).unwrap();
    assert_eq!(ct, ct_o.as_slice(), "ciphertext must match the FIPS-203 oracle");
    assert_eq!(&ss, ss_o.as_slice(), "encaps shared secret must match the FIPS-203 oracle");

    // Decaps: our dk recovers the oracle's secret; the oracle's dk recovers ours.
    assert_eq!(mlkem::decaps(&dk, &ct), ss, "our decaps recovers the shared secret");
    assert_eq!(
        dk_o.decapsulate(&ct_o).unwrap().as_slice(),
        &ss,
        "oracle decaps of our ciphertext recovers the same secret"
    );
}

#[test]
fn implicit_reject_matches_the_oracle() {
    let (ek, dk) = mlkem::keygen(&[0x44; 32], &[0x55; 32]);
    let (dk_o, _ek_o) = MlKem768::generate_deterministic(&B32::from([0x44u8; 32]), &B32::from([0x55u8; 32]));
    let (ct, _) = mlkem::encaps(&ek, &[0x66; 32]);
    let mut bad = ct.clone();
    bad[10] ^= 0xff;
    // FIPS-203: a corrupted ciphertext implicit-rejects to a z-derived secret — and OUR derived
    // reject secret matches the oracle's, byte for byte.
    let ours = mlkem::decaps(&dk, &bad);
    let ct_arr = ml_kem::Ciphertext::<MlKem768>::try_from(bad.as_slice()).unwrap();
    let oracle = dk_o.decapsulate(&ct_arr).unwrap();
    assert_eq!(&ours, oracle.as_slice(), "implicit-reject secret must match the oracle");
}
