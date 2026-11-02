use super::*;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn sha256_fips_vectors() {
    assert_eq!(
        hex(&sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex(&sha256(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hex(&sha256(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}

#[test]
fn hmac_sha256_rfc4231() {
    // RFC 4231 test case 2: key="Jefe", data="what do ya want for nothing?".
    assert_eq!(
        hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
}

#[test]
fn pbkdf2_midstate_matches_naive() {
    // The midstate fast path must equal a naive four-compression HMAC chain.
    let naive = |password: &[u8], salt: &[u8], iters: u32| -> [u8; 32] {
        let mut blk = salt.to_vec();
        blk.extend_from_slice(&1u32.to_be_bytes());
        let mut u = hmac_sha256(password, &blk);
        let mut t = u;
        for _ in 1..iters {
            u = hmac_sha256(password, &u);
            for j in 0..32 {
                t[j] ^= u[j];
            }
        }
        t
    };
    for (pw, salt, iters) in [
        (&b"password"[..], &b"salt"[..], 1u32),
        (&b"password"[..], &b"salt"[..], 4096u32),
        (&b"secret"[..], &b"\x01\x02\x03\x04"[..], 4096u32),
        (&b"pass\0word"[..], &b"sa\0lt"[..], 4096u32),
    ] {
        assert_eq!(pbkdf2_sha256(pw, salt, iters), naive(pw, salt, iters));
    }
}
