use super::*;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[test]
#[cfg(target_arch = "x86_64")]
fn shake256_x4_128_matches_four_scalar() {
    if !std::is_x86_feature_detected!("avx2") {
        return;
    }
    // Four independent `seed‖nonce` inputs (the ML-KEM CBD PRF shape) + a couple odd lengths.
    let inputs: [Vec<u8>; 4] = [
        (0..33u8).collect(),
        b"logos ml-kem noise stream one".to_vec(),
        vec![0xABu8; 100],
        (0..135u8).map(|i| i.wrapping_mul(7)).collect(),
    ];
    let refs: [&[u8]; 4] = [&inputs[0], &inputs[1], &inputs[2], &inputs[3]];
    let got = unsafe { shake256_x4_128(&refs) };
    for (lane, inp) in refs.iter().enumerate() {
        let want = shake256_bytes(inp, 128);
        assert_eq!(
            &got[lane][..],
            &want[..],
            "shake256_x4 lane {lane} must equal scalar SHAKE256"
        );
    }
}

#[test]
fn sha3_matches_nist_kats() {
    // FIPS-202 / NIST CAVP vectors.
    assert_eq!(
        hex(&sha3_256_bytes(b"")),
        "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a"
    );
    assert_eq!(
        hex(&sha3_256_bytes(b"abc")),
        "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532"
    );
    assert_eq!(
        hex(&sha3_512_bytes(b"")),
        "a69f73cca23a9ac5c8b567dc185a756e97c982164fe25859e0d1dcc1475c80a615b2123af1f5f94c11e3e9402c3ac558f500199d95b6d3e301758586281dcd26"
    );
    assert_eq!(
        hex(&sha3_512_bytes(b"abc")),
        "b751850b1a57168a5693cd924b6b096e08f621827444f70d884f5d0240d2712e10e116e9192af3c91a7ec57647e3934057340b4cf408d5a56592f8274eec53f0"
    );
}

#[test]
fn shake_matches_nist_kats() {
    assert_eq!(
        hex(&shake128_bytes(b"", 32)),
        "7f9c2ba4e88f827d616045507605853ed73b8093f6efbc88eb1a6eacfa66ef26"
    );
    assert_eq!(
        hex(&shake256_bytes(b"", 32)),
        "46b9dd2b0ba88d13233b3feb743eeb243fcd52ea62b81b82b50c27646ed5762fb5cb7df11ff5f88f60c5c7e1eb2e83d7ea4f81b7"[..64].to_string()
    );
    // SHAKE long output crosses a rate boundary (>168 bytes): exercises multi-block squeeze.
    let long = shake128_bytes(b"abc", 200);
    assert_eq!(long.len(), 200);
    assert_eq!(&hex(&long)[..32], "5881092dd818bf5cf8a3ddb793fbcba7");
}

#[test]
#[cfg(target_arch = "x86_64")]
fn keccak_f1600_x4_matches_four_scalar_permutations() {
    use std::arch::x86_64::*;
    if !std::is_x86_feature_detected!("avx2") {
        return;
    }
    let mut s = 0x9E3779B97F4A7C15u64;
    let mut rng = || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        s
    };
    for _ in 0..200 {
        // Four independent random states.
        let mut states: [[u64; 25]; 4] = [[0u64; 25]; 4];
        for st in states.iter_mut() {
            for lane in st.iter_mut() {
                *lane = rng();
            }
        }
        // Scalar reference.
        let mut want = states;
        for st in want.iter_mut() {
            keccak_f1600(st);
        }
        // Pack lane-parallel: vec[lane] = {state0[lane], .., state3[lane]}.
        let mut packed = [unsafe { _mm256_setzero_si256() }; 25];
        for (lane, slot) in packed.iter_mut().enumerate() {
            *slot = unsafe {
                _mm256_set_epi64x(
                    states[3][lane] as i64,
                    states[2][lane] as i64,
                    states[1][lane] as i64,
                    states[0][lane] as i64,
                )
            };
        }
        unsafe { keccak_f1600_x4(&mut packed) };
        // Unpack and compare per state.
        for (lane, slot) in packed.iter().enumerate() {
            let mut out = [0u64; 4];
            unsafe { _mm256_storeu_si256(out.as_mut_ptr() as *mut __m256i, *slot) };
            for state in 0..4 {
                assert_eq!(
                    out[state], want[state][lane],
                    "4-way Keccak lane {lane} of state {state} must equal the scalar permutation"
                );
            }
        }
    }
}

#[test]
fn logos_wrappers_round_trip_bytes() {
    let input: Vec<i64> = b"abc".iter().map(|&x| x as i64).collect();
    let out = sha3_256(&input).expect("in-contract arguments");
    let bytes: Vec<u8> = out.borrow().iter().map(|&x| x as u8).collect();
    assert_eq!(
        hex(&bytes),
        "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532"
    );
}
