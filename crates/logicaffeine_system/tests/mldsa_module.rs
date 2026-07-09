//! FIPS-204 conformance for the `logicaffeine_system::mldsa` keygen: the public and secret keys
//! must be byte-identical to the RustCrypto `ml-dsa` reference. This validates ExpandA, ExpandS, the
//! NTT pipeline, Power2Round, and every bit-packing layout at once.

use logicaffeine_system::mldsa;
use ml_dsa::{B32, KeyGen, MlDsa65, Signature};

#[test]
#[ignore = "benchmark — run with --ignored --nocapture"]
fn bench_mldsa65_vs_rustcrypto() {
    use std::time::Instant;
    const ITERS: u32 = 1000;
    let seed = [0x11u8; 32];
    let m = b"benchmark message for ML-DSA-65 signing throughput on this box";
    let ctx: &[u8] = b"";

    let (pk, sk) = mldsa::keygen(&seed);
    let kp = MlDsa65::key_gen_internal(&B32::from(seed));
    let sig = mldsa::sign(&sk, m, ctx);
    let ours_parsed = Signature::<MlDsa65>::try_from(sig.as_slice()).unwrap();
    // Sanity: we're benchmarking byte-identical implementations.
    assert_eq!(sig.as_slice(), kp.signing_key().sign_deterministic(m, ctx).unwrap().encode().as_slice());

    macro_rules! time {
        ($label:expr, $body:expr) => {{
            for _ in 0..20 {
                std::hint::black_box($body);
            }
            let t = Instant::now();
            for _ in 0..ITERS {
                std::hint::black_box($body);
            }
            let ns = t.elapsed().as_nanos() as f64 / ITERS as f64;
            eprintln!("{:<26} {:>10.0} ns/op", $label, ns);
            ns
        }};
    }

    eprintln!("\n=== ML-DSA-65 timings ({} iters, this box) ===", ITERS);
    let lk = time!("ours   keygen", mldsa::keygen(&seed));
    let ok = time!("oracle keygen", MlDsa65::key_gen_internal(&B32::from(seed)));
    let ls = time!("ours   sign", mldsa::sign(&sk, m, ctx));
    let os = time!("oracle sign", kp.signing_key().sign_deterministic(m, ctx).unwrap());
    let lv = time!("ours   verify", mldsa::verify(&pk, m, ctx, &sig));
    let ov = time!("oracle verify", kp.verifying_key().verify_with_context(m, ctx, &ours_parsed));
    eprintln!("\n--- ratio ours/oracle (>1 = we are slower) ---");
    eprintln!("keygen {:.2}x   sign {:.2}x   verify {:.2}x", lk / ok, ls / os, lv / ov);
    eprintln!();
    mldsa::bench_verify_breakdown(&pk, &sig);
}

#[test]
fn keygen_is_bit_exact_vs_fips204_oracle() {
    for seed_byte in [0x11u8, 0x44, 0xc7] {
        let seed = [seed_byte; 32];
        let (pk, sk) = mldsa::keygen(&seed);
        let kp = MlDsa65::key_gen_internal(&B32::from(seed));
        assert_eq!(
            pk.as_slice(),
            kp.verifying_key().encode().as_slice(),
            "ML-DSA-65 pk must match the FIPS-204 oracle (seed {seed_byte:#x})"
        );
        assert_eq!(
            sk.as_slice(),
            kp.signing_key().encode().as_slice(),
            "ML-DSA-65 sk must match the FIPS-204 oracle (seed {seed_byte:#x})"
        );
    }
}

#[test]
fn sign_is_bit_exact_vs_oracle_and_interops_both_ways() {
    let seed = [0x33u8; 32];
    let (pk, sk) = mldsa::keygen(&seed);
    let kp = MlDsa65::key_gen_internal(&B32::from(seed));
    let m = b"FIPS-204 conformance message for ML-DSA-65 signatures";
    let ctx = b"logos-pq";

    // Our DETERMINISTIC signature is byte-identical to the FIPS-204 oracle — the rejection loop,
    // SampleInBall, ExpandMask, hint generation, and sig packing all match.
    let sig = mldsa::sign(&sk, m, ctx);
    let sig_o = kp.signing_key().sign_deterministic(m, ctx).unwrap();
    assert_eq!(
        sig.as_slice(),
        sig_o.encode().as_slice(),
        "ML-DSA-65 signature must be bit-exact vs the FIPS-204 oracle"
    );

    // Interop both ways: the oracle verifies our signature; we verify the oracle's.
    let ours_parsed = Signature::<MlDsa65>::try_from(sig.as_slice()).unwrap();
    assert!(
        kp.verifying_key().verify_with_context(m, ctx, &ours_parsed),
        "the FIPS-204 oracle must verify our signature"
    );
    assert!(
        mldsa::verify(&pk, m, ctx, sig_o.encode().as_slice()),
        "we must verify the FIPS-204 oracle's signature"
    );
    // And we reject a tampered version of the oracle's signature.
    let mut bad = sig_o.encode().as_slice().to_vec();
    bad[200] ^= 1;
    assert!(!mldsa::verify(&pk, m, ctx, &bad), "tampered oracle signature rejects");
}
