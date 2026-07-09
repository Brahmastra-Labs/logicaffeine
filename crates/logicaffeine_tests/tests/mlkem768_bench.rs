//! Whole-primitive speed benchmark: our shipped ML-KEM-768 native kernels — the ones the Logos
//! `mlkem768Keygen`/`Encaps`/`Decaps` orchestration in `assets/std/crypto.lg` lowers to, whose
//! NTT/Keccak are the cfg-inline SIMD lane kernels measured at 1.08× of raw AVX2 — vs. TWO oracles in
//! ONE process (so shared-box contention hits all three equally, the only apples-to-apples on a loaded
//! machine):
//!   • RustCrypto `ml-kem` 0.2  — the PORTABLE reference (no hand AVX2; autovec only).
//!   • `libcrux-ml-kem` 0.0.9 (simd256) — Cryspen's FORMALLY-VERIFIED AVX2. **This is the hard bar.**
//!
//! Correctness is asserted THREE-WAY bit-exact FIRST (ours == ml-kem == libcrux for the same seed —
//! libcrux's 64-byte keygen randomness is exactly our `d‖z`, and all three are FIPS 203), so the bench
//! doubles as a differential gate; then keygen/encaps/decaps are timed and reported ns/op + ratios.
//! Both oracle crates are dev-only — never in the shipped graph. Run with the target the shipped AOT
//! build uses so every implementation gets its best codegen (and libcrux's AVX2 path is selected):
//!   RUSTFLAGS="-C target-cpu=native" cargo test --release -p logicaffeine-tests \
//!     --test mlkem768_bench -- --ignored --nocapture
#![cfg(not(target_arch = "wasm32"))]

use std::hint::black_box;
use std::time::Instant;

use logicaffeine_system::mlkem;

use libcrux_ml_kem::mlkem768 as lc;
use ml_kem::kem::Decapsulate;
use ml_kem::{B32, EncapsulateDeterministic, EncodedSizeUser, KemCore, MlKem768};

/// Time `f` over `iters` iterations (with a warmup tenth) and return ns/op.
fn bench<F: FnMut()>(iters: usize, mut f: F) -> f64 {
    for _ in 0..(iters / 10 + 1) {
        f();
    }
    let t = Instant::now();
    for _ in 0..iters {
        f();
    }
    t.elapsed().as_nanos() as f64 / iters as f64
}

#[test]
#[ignore = "speed benchmark — run with --ignored --nocapture (best under -C target-cpu=native)"]
fn bench_mlkem768_ours_vs_rustcrypto_and_libcrux() {
    let d = [0x11u8; 32];
    let z = [0x22u8; 32];
    let m = [0x33u8; 32];
    let mut seed64 = [0u8; 64]; // libcrux keygen randomness = d ‖ z
    seed64[..32].copy_from_slice(&d);
    seed64[32..].copy_from_slice(&z);

    // ── Correctness gate: THREE-WAY bit-exact (ours == RustCrypto == libcrux), FIPS 203 ──
    let (oracle_dk, oracle_ek) = MlKem768::generate_deterministic(&B32::from(d), &B32::from(z));
    let (our_ek, our_dk) = mlkem::keygen(&d, &z);
    let lc_kp = lc::generate_key_pair(seed64);
    assert_eq!(our_ek, oracle_ek.as_bytes().to_vec(), "keygen ek: ours == ml-kem");
    assert_eq!(&our_ek[..], &lc_kp.pk()[..], "keygen ek: ours == libcrux");
    assert_eq!(our_dk, oracle_dk.as_bytes().to_vec(), "keygen dk: ours == ml-kem");
    assert_eq!(&our_dk[..], &lc_kp.sk()[..], "keygen dk: ours == libcrux");

    let (oracle_ct, oracle_k) =
        oracle_ek.encapsulate_deterministic(&B32::from(m)).expect("oracle encaps");
    let (our_ct, our_ss) = mlkem::encaps(&our_ek, &m);
    let (lc_ct, lc_ss) = lc::encapsulate(lc_kp.public_key(), m);
    assert_eq!(our_ct.as_slice(), oracle_ct.as_slice(), "encaps ct: ours == ml-kem");
    assert_eq!(our_ct.as_slice(), &lc_ct.as_slice()[..], "encaps ct: ours == libcrux");
    assert_eq!(&our_ss[..], oracle_k.as_slice(), "encaps ss: ours == ml-kem");
    assert_eq!(&our_ss[..], &lc_ss[..], "encaps ss: ours == libcrux");

    let our_recovered = mlkem::decaps(&our_dk, &our_ct);
    let oracle_recovered = oracle_dk.decapsulate(&oracle_ct).expect("oracle decaps");
    let lc_recovered = lc::decapsulate(lc_kp.private_key(), &lc_ct);
    assert_eq!(&our_recovered[..], &our_ss[..], "decaps: ours recovers the secret");
    assert_eq!(&our_recovered[..], oracle_recovered.as_slice(), "decaps: ours == ml-kem");
    assert_eq!(&our_recovered[..], &lc_recovered[..], "decaps: ours == libcrux");

    // ── Timing: same-run, N iterations each ──
    const N: usize = 3000;
    let od = B32::from(d);
    let oz = B32::from(z);
    let om = B32::from(m);

    let ours_kg = bench(N, || {
        black_box(mlkem::keygen(black_box(&d), black_box(&z)));
    });
    let rc_kg = bench(N, || {
        black_box(MlKem768::generate_deterministic(black_box(&od), black_box(&oz)));
    });
    let lc_kg = bench(N, || {
        black_box(lc::generate_key_pair(black_box(seed64)));
    });

    let ours_en = bench(N, || {
        black_box(mlkem::encaps(black_box(&our_ek), black_box(&m)));
    });
    let rc_en = bench(N, || {
        black_box(oracle_ek.encapsulate_deterministic(black_box(&om)).expect("encaps"));
    });
    let lc_en = bench(N, || {
        black_box(lc::encapsulate(black_box(lc_kp.public_key()), black_box(m)));
    });

    let ours_de = bench(N, || {
        black_box(mlkem::decaps(black_box(&our_dk), black_box(&our_ct)));
    });
    let rc_de = bench(N, || {
        black_box(oracle_dk.decapsulate(black_box(&oracle_ct)).expect("decaps"));
    });
    let lc_de = bench(N, || {
        black_box(lc::decapsulate(black_box(lc_kp.private_key()), black_box(&lc_ct)));
    });

    let row = |name: &str, ours: f64, rc: f64, lc: f64| {
        println!(
            "  {name:<7} ours {ours:>8.0}   ml-kem {rc:>8.0} ({:>4.2}×)   libcrux {lc:>8.0} ({:>4.2}× {})",
            rc / ours.max(1.0),
            lc / ours.max(1.0),
            if ours <= lc { "ours faster" } else { "libcrux faster" },
        );
    };
    println!("\n=== ML-KEM-768 whole-primitive (ns/op, same process, N={N}) — 3-way bit-exact ✓ ===");
    println!("    ours = native cfg-inline SIMD (what crypto.lg lowers to);  ml-kem = portable ref;  libcrux = VERIFIED AVX2 (the bar)");
    row("keygen", ours_kg, rc_kg, lc_kg);
    row("encaps", ours_en, rc_en, lc_en);
    row("decaps", ours_de, rc_de, lc_de);
    println!("    (ratios are competitor_ns / ours_ns; >1.00× means we are faster)");
}
