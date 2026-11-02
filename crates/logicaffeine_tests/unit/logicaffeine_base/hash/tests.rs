use super::*;

#[test]
fn md5_known_rfc_vectors() {
    assert_eq!(to_hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(to_hex(&md5(b"a")), "0cc175b9c0f1b6a831c399e269772661");
    assert_eq!(to_hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(
        to_hex(&md5(b"message digest")),
        "f96b697d7cb7938d525a2f31aaf161d0"
    );
    assert_eq!(
        to_hex(&md5(b"abcdefghijklmnopqrstuvwxyz")),
        "c3fcd3d76192e4007dfb496cca67e13b"
    );
    assert_eq!(
        to_hex(&md5(b"The quick brown fox jumps over the lazy dog")),
        "9e107d9d372bb6826bd81d3542a419d6"
    );
}

#[test]
fn sha1_known_rfc_vectors() {
    assert_eq!(
        to_hex(&sha1(b"")),
        "da39a3ee5e6b4b0d3255bfef95601890afd80709"
    );
    assert_eq!(
        to_hex(&sha1(b"abc")),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
    assert_eq!(
        to_hex(&sha1(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
    );
    assert_eq!(
        to_hex(&sha1(b"The quick brown fox jumps over the lazy dog")),
        "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"
    );
}

#[test]
fn md5_block_boundaries_are_exact() {
    // Lengths straddling the 55/56/64/119/120-byte padding edges — the cases padding bugs hide in.
    for n in [
        0usize, 1, 54, 55, 56, 57, 63, 64, 65, 119, 120, 127, 128, 256, 1000,
    ] {
        let data = vec![0xABu8; n];
        let mut oracle = <md5::Md5 as md5::Digest>::new();
        md5::Digest::update(&mut oracle, &data);
        let want: [u8; 16] = md5::Digest::finalize(oracle).into();
        assert_eq!(md5(&data), want, "md5 mismatch at len {n}");
    }
}

#[test]
#[cfg(target_arch = "x86_64")]
fn md5_x4_matches_scalar_over_lengths() {
    // The 4-way SIMD multi-buffer MD5 must equal the scalar MD5 on every lane, across lengths that
    // straddle the block and one/two-block padding edges (55/56/57, 63/64/65, 119/120).
    for len in [
        0usize, 1, 3, 16, 32, 55, 56, 57, 63, 64, 65, 119, 120, 128, 200,
    ] {
        let mut msgs = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        let mut st = 0x1234_5678u64.wrapping_add(len as u64);
        for m in msgs.iter_mut() {
            for _ in 0..len {
                st = st.wrapping_mul(6364136223846793005).wrapping_add(1);
                m.push((st >> 56) as u8);
            }
        }
        let refs: [&[u8]; 4] = [&msgs[0], &msgs[1], &msgs[2], &msgs[3]];
        let x4 = md5_x4(refs);
        for (j, m) in msgs.iter().enumerate() {
            assert_eq!(x4[j], md5(m), "md5_x4 lane {j} differs at len {len}");
        }
    }
}

#[test]
#[cfg(target_arch = "x86_64")]
fn md5_x8_matches_scalar_over_lengths() {
    // The 8-way AVX2 multi-buffer MD5 must equal the scalar MD5 on every lane, across the same
    // block/padding edges the 4-way is checked at.
    for len in [
        0usize, 1, 3, 16, 32, 55, 56, 57, 63, 64, 65, 119, 120, 128, 200,
    ] {
        let mut msgs: Vec<Vec<u8>> = (0..8).map(|_| Vec::new()).collect();
        let mut st = 0x9e37_79b9u64.wrapping_add(len as u64);
        for m in msgs.iter_mut() {
            for _ in 0..len {
                st = st.wrapping_mul(6364136223846793005).wrapping_add(1);
                m.push((st >> 56) as u8);
            }
        }
        let refs: [&[u8]; 8] = [
            &msgs[0], &msgs[1], &msgs[2], &msgs[3], &msgs[4], &msgs[5], &msgs[6], &msgs[7],
        ];
        let x8 = md5_x8(refs);
        for (j, m) in msgs.iter().enumerate() {
            assert_eq!(x8[j], md5(m), "md5_x8 lane {j} differs at len {len}");
        }
    }
}

#[test]
fn sha1_block_boundaries_are_exact() {
    for n in [
        0usize, 1, 54, 55, 56, 57, 63, 64, 65, 119, 120, 127, 128, 256, 1000,
    ] {
        let data = vec![0x5Au8; n];
        let mut oracle = <sha1::Sha1 as sha1::Digest>::new();
        sha1::Digest::update(&mut oracle, &data);
        let want: [u8; 20] = sha1::Digest::finalize(oracle).into();
        assert_eq!(sha1(&data), want, "sha1 mismatch at len {n}");
    }
}

#[test]
fn differential_fuzz_against_the_reference_crates() {
    // A cheap deterministic LCG drives 5000 random-length, random-content inputs; both digests
    // must agree with the reference crates bit-for-bit. (No external RNG — reproducible.)
    let mut state: u64 = 0x1234_5678_9abc_def0;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as u32
    };
    for _ in 0..5000 {
        let len = (next() % 600) as usize;
        let data: Vec<u8> = (0..len).map(|_| (next() & 0xff) as u8).collect();

        let mut m = <md5::Md5 as md5::Digest>::new();
        md5::Digest::update(&mut m, &data);
        let m_want: [u8; 16] = md5::Digest::finalize(m).into();
        assert_eq!(md5(&data), m_want, "md5 differs at len {len}");

        let mut s = <sha1::Sha1 as sha1::Digest>::new();
        sha1::Digest::update(&mut s, &data);
        let s_want: [u8; 20] = sha1::Digest::finalize(s).into();
        assert_eq!(sha1(&data), s_want, "sha1 differs at len {len}");
    }
}

/// The PORTABLE SCALAR paths (`sha1_scalar`, `md5_x*_scalar`) ship to every target without
/// SHA-NI / SSE2 / AVX2, but on this hardware `sha1`/`md5_x4`/`md5_x8` dispatch to the
/// accelerated routes — so without a direct test these fallbacks are DEAD CODE here and their
/// every mutation survives. These tests exercise them straight, against the reference crate and
/// the scalar `md5` oracle, over RFC vectors + block boundaries + a fuzz sweep.
#[test]
fn sha1_scalar_matches_reference_over_vectors_boundaries_and_fuzz() {
    // RFC 3174 known vectors, straight through the scalar path.
    assert_eq!(
        to_hex(&sha1_scalar(b"")),
        "da39a3ee5e6b4b0d3255bfef95601890afd80709"
    );
    assert_eq!(
        to_hex(&sha1_scalar(b"abc")),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
    assert_eq!(
        to_hex(&sha1_scalar(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
    );
    // Block boundaries (55/56/63/64/65/119/120 bytes exercise both padding-spill branches) and a
    // fuzz sweep — each cross-checked against the independent `sha1` reference crate.
    let mut state: u64 = 0xdead_beef_cafe_babe;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as u32
    };
    let lens = (0usize..130).chain([255, 256, 257, 511, 512, 513, 1000]);
    for len in lens {
        let data: Vec<u8> = (0..len).map(|_| (next() & 0xff) as u8).collect();
        let mut s = <sha1::Sha1 as sha1::Digest>::new();
        sha1::Digest::update(&mut s, &data);
        let want: [u8; 20] = sha1::Digest::finalize(s).into();
        assert_eq!(sha1_scalar(&data), want, "sha1_scalar differs at len {len}");
    }
}

#[test]
fn md5_scalar_lane_fallbacks_match_the_scalar_oracle() {
    let mut state: u64 = 0x0123_4567_89ab_cdef;
    let mut next = || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as u32
    };
    for len in [0usize, 1, 55, 56, 63, 64, 65, 120, 200] {
        let msgs: Vec<Vec<u8>> = (0..8)
            .map(|_| (0..len).map(|_| (next() & 0xff) as u8).collect())
            .collect();
        let r: Vec<&[u8]> = msgs.iter().map(|m| m.as_slice()).collect();

        let x4 = md5_x4_scalar([r[0], r[1], r[2], r[3]]);
        for j in 0..4 {
            assert_eq!(
                x4[j],
                md5(r[j]),
                "md5_x4_scalar lane {j} differs at len {len}"
            );
        }
        let x8 = md5_x8_scalar([r[0], r[1], r[2], r[3], r[4], r[5], r[6], r[7]]);
        for j in 0..8 {
            assert_eq!(
                x8[j],
                md5(r[j]),
                "md5_x8_scalar lane {j} differs at len {len}"
            );
        }
    }
}
