//! Whole-primitive speed benchmark: our shipped ML-DSA-65 (Dilithium3) native kernels vs. TWO oracles
//! in ONE process (shared-box contention hits all three equally):
//!   • RustCrypto `ml-dsa` 0.1.1     — portable reference.
//!   • `libcrux-ml-dsa` 0.0.9 (simd256) — Cryspen's FORMALLY-VERIFIED AVX2. **The hard bar.**
//!
//! Correctness asserted FIRST (all deterministic, FIPS 204): keygen (pk+sk) THREE-WAY bit-exact, the
//! deterministic signature (rnd=0, empty ctx) ours == RustCrypto == libcrux bit-exact, and every
//! library verifies the (identical) signature — so the bench is a differential + interop gate. Then
//! keygen/sign/verify are timed and reported ns/op + ratios. Both oracle crates are dev-only.
//! Run with the shipped-AOT target so libcrux's AVX2 path is selected:
//!   RUSTFLAGS="-C target-cpu=native" cargo test --release -p logicaffeine-tests \
//!     --test mldsa65_bench -- --ignored --nocapture
#![cfg(not(target_arch = "wasm32"))]

use std::hint::black_box;
use std::time::Instant;

use logicaffeine_system::mldsa;

use libcrux_ml_dsa::ml_dsa_65 as lcd;
use ml_dsa::{B32, Keypair, MlDsa65, Signer, SigningKey};

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
fn bench_mldsa65_ours_vs_rustcrypto_and_libcrux() {
    let seed = [0x11u8; 32];
    let msg: &[u8] = b"ML-DSA-65 whole-primitive Logos benchmark message";
    let ctx: &[u8] = &[];
    let zero_rnd = [0u8; 32]; // FIPS 204 deterministic variant

    // ── Correctness: keygen THREE-WAY bit-exact ──
    let (our_pk, our_sk) = mldsa::keygen(&seed);
    let lc_kp = lcd::generate_key_pair(seed);
    let rc_sk = SigningKey::<MlDsa65>::from_seed(&B32::from(seed));
    let rc_vk = rc_sk.verifying_key();
    assert_eq!(&our_pk[..], &lc_kp.verification_key.as_ref()[..], "keygen pk: ours == libcrux");
    assert_eq!(&our_pk[..], rc_vk.encode().as_slice(), "keygen pk: ours == RustCrypto");
    // sk is compared bit-exact vs libcrux (clean `.as_ref()`); RustCrypto's SigningKey has no
    // ergonomic raw-byte encode (it's pkcs8/KeyExport-flavored), and pk agreement already pins the
    // derivation three-way, so the sk gate is ours == libcrux.
    assert_eq!(&our_sk[..], &lc_kp.signing_key.as_ref()[..], "keygen sk: ours == libcrux");

    // ── Correctness: deterministic signature THREE-WAY bit-exact + each verifies ──
    let our_sig = mldsa::sign(&our_sk, msg, ctx);
    let lc_sig = lcd::sign(&lc_kp.signing_key, msg, ctx, zero_rnd).expect("libcrux sign");
    // RustCrypto's `Signer::sign` IS the deterministic, empty-context ML-DSA variant.
    let rc_sig = rc_sk.sign(msg);
    assert_eq!(our_sig.as_slice(), lc_sig.as_ref().as_slice(), "sign: ours == libcrux (det, ctx=∅)");
    assert_eq!(our_sig.as_slice(), rc_sig.encode().as_slice(), "sign: ours == RustCrypto (det, ctx=∅)");
    assert!(mldsa::verify(&our_pk, msg, ctx, &our_sig), "ours must verify its own signature");
    assert!(lcd::verify(&lc_kp.verification_key, msg, ctx, &lc_sig).is_ok(), "libcrux verifies");
    assert!(rc_vk.verify_with_context(msg, ctx, &rc_sig), "RustCrypto verifies");
    assert!(!mldsa::verify(&our_pk, b"tampered", ctx, &our_sig), "ours rejects a wrong message");

    // ── Timing: same-run, N iterations each ──
    const N: usize = 1500;
    let rc_seed = B32::from(seed);

    let ours_kg = bench(N, || {
        black_box(mldsa::keygen(black_box(&seed)));
    });
    let rc_kg = bench(N, || {
        black_box(SigningKey::<MlDsa65>::from_seed(black_box(&rc_seed)));
    });
    let lc_kg = bench(N, || {
        black_box(lcd::generate_key_pair(black_box(seed)));
    });

    let ours_sg = bench(N, || {
        black_box(mldsa::sign(black_box(&our_sk), black_box(msg), black_box(ctx)));
    });
    let rc_sg = bench(N, || {
        black_box(rc_sk.sign(black_box(msg)));
    });
    let lc_sg = bench(N, || {
        black_box(lcd::sign(black_box(&lc_kp.signing_key), black_box(msg), black_box(ctx), zero_rnd).expect("sign"));
    });

    let ours_vf = bench(N, || {
        black_box(mldsa::verify(black_box(&our_pk), black_box(msg), black_box(ctx), black_box(&our_sig)));
    });
    let rc_vf = bench(N, || {
        black_box(rc_vk.verify_with_context(black_box(msg), black_box(ctx), black_box(&rc_sig)));
    });
    let lc_vf = bench(N, || {
        black_box(lcd::verify(black_box(&lc_kp.verification_key), black_box(msg), black_box(ctx), black_box(&lc_sig)).is_ok());
    });

    let row = |name: &str, ours: f64, rc: f64, lc: f64| {
        println!(
            "  {name:<7} ours {ours:>8.0}   ml-dsa {rc:>8.0} ({:>4.2}×)   libcrux {lc:>8.0} ({:>4.2}× {})",
            rc / ours.max(1.0),
            lc / ours.max(1.0),
            if ours <= lc { "ours faster" } else { "libcrux faster" },
        );
    };
    println!("\n=== ML-DSA-65 whole-primitive (ns/op, same process, N={N}) — 3-way bit-exact ✓ ===");
    println!("    ours = native cfg-inline SIMD (4-way Keccak ExpandA + i32 NTT);  ml-dsa = portable ref;  libcrux = VERIFIED AVX2 (the bar)");
    row("keygen", ours_kg, rc_kg, lc_kg);
    row("sign", ours_sg, rc_sg, lc_sg);
    row("verify", ours_vf, rc_vf, lc_vf);
    println!("    (ratios are competitor_ns / ours_ns; >1.00× means we are faster)");
    println!("    NOTE: RustCrypto pre-expands the signing key at keygen, so its `sign` skips per-call");
    println!("          ExpandA (ours + libcrux re-derive Â each sign) — its keygen is heavier, sign lighter;");
    println!("          the keygen+sign SUM is the fair sign-once total.");
}
