use super::*;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn chacha20_avx2_matches_scalar_over_lengths() {
    // The 8-block AVX2 path must be byte-identical to the scalar block across chunk boundaries,
    // partial tails, and counter wrap-around.
    let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(31).wrapping_add(7));
    let nonce: [u8; 12] = std::array::from_fn(|i| (i as u8).wrapping_mul(17));
    let mut s = 0x1234_5678u64;
    let mut data = vec![0u8; 2000];
    for b in data.iter_mut() {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
        *b = (s >> 33) as u8;
    }
    for &len in &[0usize, 1, 63, 64, 65, 511, 512, 513, 1024, 1100, 2000] {
        for &ctr in &[0u32, 1, 7, u32::MAX - 3] {
            let want = chacha20_xor_scalar(&key, &nonce, ctr, &data[..len]);
            let got = chacha20_xor(&key, &nonce, ctr, &data[..len]);
            assert_eq!(got, want, "AVX2 == scalar, len {len} ctr {ctr}");
        }
    }
}

#[test]
fn poly1305_avx2_matches_scalar_over_lengths() {
    // The 4-block AVX2 path must be byte-identical to the scalar block across group boundaries
    // and tails (the AVX2 path engages at ≥ 256 bytes).
    let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(29).wrapping_add(3));
    let mut s = 0xdead_beefu64;
    let mut msg = vec![0u8; 4096];
    for b in msg.iter_mut() {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
        *b = (s >> 33) as u8;
    }
    for &len in &[
        0usize, 1, 16, 17, 63, 64, 255, 256, 257, 271, 512, 1000, 1024, 1025, 4000, 4096,
    ] {
        let want = poly1305_scalar(&key, &msg[..len]);
        let got = poly1305(&key, &msg[..len]);
        assert_eq!(got, want, "AVX2 == scalar Poly1305, len {len}");
    }
}

#[test]
fn poly1305_aead_streamed_matches_mac_data() {
    // The streamed AEAD MAC must equal the materialized `poly1305(mac_data(aad, ct))` across all
    // alignments of aad and ct (full groups, partial tails, AVX2 vs scalar ct).
    let mut s = 0x0123_4567_89ab_cdefu64;
    let mut next = |buf: &mut [u8]| {
        for b in buf.iter_mut() {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (s >> 33) as u8;
        }
    };
    let mut otk = [0u8; 32];
    next(&mut otk);
    let mut ct = vec![0u8; 4100];
    next(&mut ct);
    let mut aad = vec![0u8; 64];
    next(&mut aad);
    for &al in &[0usize, 1, 11, 16, 17, 32, 48] {
        for &cl in &[
            0usize, 1, 15, 16, 63, 64, 255, 256, 257, 271, 1024, 1025, 4100,
        ] {
            let streamed = poly1305_aead(&otk, &aad[..al], &ct[..cl]);
            let reference = poly1305(&otk, &mac_data(&aad[..al], &ct[..cl]));
            assert_eq!(streamed, reference, "streamed AEAD MAC, aad {al} ct {cl}");
        }
    }
}

#[test]
fn chacha20_xor_matches_rfc8439_2_4_2() {
    // RFC 8439 §2.4.2: the famous "Ladies and Gentlemen..." sunscreen vector.
    let key: [u8; 32] = std::array::from_fn(|i| i as u8);
    let nonce: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0x4a, 0, 0, 0, 0];
    let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
    let ct = chacha20_xor(&key, &nonce, 1, plaintext);
    assert_eq!(
        hex(&ct),
        "6e2e359a2568f98041ba0728dd0d6981e97e7aec1d4360c20a27afccfd9fae0bf91b65c5524733ab8f593dabcd62b3571639d624e65152ab8f530c359f0861d807ca0dbf500d6a6156a38e088a22b65e52bc514d16ccf806818ce91ab77937365af90bbf74a35be6b40b8eedf2785e42874d",
        "ChaCha20 keystream XOR must match RFC 8439 §2.4.2"
    );
}

#[test]
fn poly1305_matches_rfc8439_2_5_2() {
    let key: [u8; 32] = unhex("85d6be7857556d337f4452fe42d506a80103808afb0db2fd4abff6af4149f51b")
        [..32]
        .try_into()
        .unwrap();
    let msg = b"Cryptographic Forum Research Group";
    let tag = poly1305(&key, msg);
    assert_eq!(
        hex(&tag),
        "a8061dc1305136c6c22b8baf0c0127a9",
        "Poly1305 must match RFC 8439 §2.5.2"
    );
}

#[test]
fn aead_seals_and_opens_round_trip_and_rejects_tamper() {
    let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(3));
    let nonce: [u8; 12] = std::array::from_fn(|i| i as u8 + 0x40);
    let aad = b"channel-suite-id-v1";
    let msg = b"post-quantum sealed wire payload";
    let sealed = chacha20poly1305_seal(&key, &nonce, aad, msg);
    assert_eq!(sealed.len(), msg.len() + 16, "ciphertext + 16-byte tag");
    assert_eq!(
        chacha20poly1305_open(&key, &nonce, aad, &sealed).as_deref(),
        Some(&msg[..])
    );
    // Tamper the ciphertext, the tag, and the AAD — each must reject.
    let mut t = sealed.clone();
    t[0] ^= 1;
    assert_eq!(
        chacha20poly1305_open(&key, &nonce, aad, &t),
        None,
        "ciphertext tamper rejects"
    );
    let mut t = sealed.clone();
    let last = t.len() - 1;
    t[last] ^= 1;
    assert_eq!(
        chacha20poly1305_open(&key, &nonce, aad, &t),
        None,
        "tag tamper rejects"
    );
    assert_eq!(
        chacha20poly1305_open(&key, &nonce, b"wrong-aad", &sealed),
        None,
        "AAD mismatch rejects"
    );
}

#[test]
#[ignore = "benchmark — run with --ignored --nocapture"]
fn bench_aead_vs_rustcrypto() {
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
    use std::time::Instant;

    let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(1));
    let nonce: [u8; 12] = std::array::from_fn(|i| i as u8 + 0x10);
    let aad = b"channel-aad";
    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key));
    let n = Nonce::from_slice(&nonce);

    for &len in &[64usize, 1024, 16384] {
        let pt = vec![0x5au8; len];
        let sealed = chacha20poly1305_seal(&key, &nonce, aad, &pt);
        let sealed_o = cipher.encrypt(n, Payload { msg: &pt, aad }).unwrap();
        assert_eq!(
            sealed, sealed_o,
            "our AEAD must match RustCrypto for {len}B"
        );

        const ITERS: u32 = 20000;
        // Peak throughput = min time over repeats, so a transient scheduling/thermal stall on a
        // shared box can't masquerade as a slow primitive (both sides measured the same way).
        macro_rules! mbps {
            ($body:expr) => {{
                for _ in 0..50 {
                    std::hint::black_box($body);
                }
                let mut best = f64::INFINITY;
                for _ in 0..15 {
                    let t = Instant::now();
                    for _ in 0..ITERS {
                        std::hint::black_box($body);
                    }
                    best = best.min(t.elapsed().as_secs_f64());
                }
                (len as f64 * ITERS as f64) / best / 1e6
            }};
        }
        let ours = mbps!(chacha20poly1305_seal(&key, &nonce, aad, &pt));
        let theirs = mbps!(cipher.encrypt(n, Payload { msg: &pt, aad }).unwrap());
        eprintln!(
            "seal {:>6}B:  ours {:>8.0} MB/s   RustCrypto {:>8.0} MB/s   ({:.2}x)",
            len,
            ours,
            theirs,
            theirs / ours
        );
    }
}

#[test]
fn aead_matches_rfc8439_2_8_2() {
    let key: [u8; 32] = unhex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f")
        [..32]
        .try_into()
        .unwrap();
    let nonce: [u8; 12] = unhex("070000004041424344454647")[..12].try_into().unwrap();
    let aad = unhex("50515253c0c1c2c3c4c5c6c7");
    let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
    let sealed = chacha20poly1305_seal(&key, &nonce, &aad, plaintext);
    // RFC 8439 §2.8.2 ciphertext ‖ tag.
    assert_eq!(
        hex(&sealed),
        "d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc3ff4def08e4b7a9de576d26586cec64b61161ae10b594f09e26a7e902ecbd0600691",
        "AEAD seal must match RFC 8439 §2.8.2"
    );
    assert_eq!(
        chacha20poly1305_open(&key, &nonce, &aad, &sealed).as_deref(),
        Some(&plaintext[..]),
        "AEAD open round-trips the RFC vector"
    );
}
