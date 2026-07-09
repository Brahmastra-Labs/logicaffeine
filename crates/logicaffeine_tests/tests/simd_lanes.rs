//! SIMD lane-vector types — the L3-via-types foundation (`work/SIMD_LANES.md`).
//!
//! Adds a fixed-width lane vector (`Lanes8Word32` = 8×u32 = one AVX2 `__m256i`) to Logos, the same
//! way `Word8/16/32/64` were added — so the vectorized crypto is *written in Logos* and compiles to
//! the same instructions as the hand-written AVX2. The proof that the AVX2 lowering is correct is
//! the built-in tier differential: the tree-walker runs **scalar-lane semantics** (a vector is `[u32;8]`,
//! each op is 8 independent scalar ops = the SPEC); AOT runs the **AVX2 intrinsics**; they must agree.
//!
//! Increment 1: construct `Lanes8Word32`, lane-wise `xor`, read a lane back.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::run_logos_with_args;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Build two 8-lane Word32 vectors (all 5s, all 3s), XOR them lane-wise, read lane 1 → `5 ^ 3 = 6`.
fn program() -> String {
    let mut p = String::from("## Main\n");
    p.push_str("Let s be a new Seq of Word32.\nRepeat for i from 1 to 8:\n    Push word32(5) to s.\n");
    p.push_str("Let t be a new Seq of Word32.\nRepeat for i from 1 to 8:\n    Push word32(3) to t.\n");
    p.push_str("Let v be lanes8Word32(s).\n");
    p.push_str("Let w be lanes8Word32(t).\n");
    p.push_str("Let x be v xor w.\n");
    p.push_str("Let r be seqOfLanes8(x).\n");
    p.push_str("Show item 1 of r.\n");
    p
}

/// A `Lanes8Word32` flowing through a TYPED Logos function (`## To … (a: Lanes8Word32) → …`) — the
/// first-class type path the crypto kernels will use. `(i+9) xor i` at lane 3 = `12 xor 3 = 15`.
fn typed_program() -> String {
    let mut p = String::new();
    p.push_str("## To laneXor (a: Lanes8Word32) and (b: Lanes8Word32) -> Lanes8Word32:\n");
    p.push_str("    Return a xor b.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let s be a new Seq of Word32.\nRepeat for i from 1 to 8:\n    Push word32(i + 9) to s.\n");
    p.push_str("Let t be a new Seq of Word32.\nRepeat for i from 1 to 8:\n    Push word32(i) to t.\n");
    p.push_str("Let v be lanes8Word32(s).\n");
    p.push_str("Let w be lanes8Word32(t).\n");
    p.push_str("Let x be laneXor(v, w).\n");
    p.push_str("Let r be seqOfLanes8(x).\n");
    p.push_str("Show item 3 of r.\n");
    p
}

#[test]
fn lanes8word32_typed_function_spec_tw_and_vm() {
    let src = typed_program();
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "typed-function tree-walker errored: {:?}", tw.error);
    assert_eq!(tw.output.trim(), "15", "(12 xor 3) at lane 3 = 15 (tree-walker)");
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "typed-function VM errored: {:?}", vm.error);
    assert_eq!(vm.output.trim(), "15", "(12 xor 3) at lane 3 = 15 (VM)");
}

/// FAST RED→GREEN driver: the lane vector + lane-wise xor must work on BOTH non-AOT tiers, and the
/// tree-walker's scalar-lane semantics is the spec the AVX2 lowering will be held to.
#[test]
fn lanes8word32_xor_spec_tw_and_vm() {
    let src = program();
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "tree-walker (scalar-lane spec) errored: {:?}", tw.error);
    assert_eq!(tw.output.trim(), "6", "lane-wise 5 xor 3 = 6 on the tree-walker");
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "bytecode VM errored: {:?}", vm.error);
    assert_eq!(vm.output.trim(), "6", "lane-wise 5 xor 3 = 6 on the VM");
}

/// The ChaCha20 quarter-round (RFC 8439 §2.2.1) written in Logos lanes — `a += b; d ^= a; d <<<= 16;
/// …` over `Lanes8Word32`. Every block runs in its own lane, so lane 0 computes the scalar vector.
fn quarter_round_program(a: u32, b: u32, c: u32, d: u32) -> String {
    let mut p = String::new();
    p.push_str(
        "## To laneQuarterRound (a: Lanes8Word32) and (b: Lanes8Word32) and (c: Lanes8Word32) and (d: Lanes8Word32) -> Seq of Lanes8Word32:\n",
    );
    p.push_str("    Let mutable na be a.\n    Let mutable nb be b.\n    Let mutable nc be c.\n    Let mutable nd be d.\n");
    p.push_str("    Set na to na + nb.\n    Set nd to rotl(nd xor na, 16).\n");
    p.push_str("    Set nc to nc + nd.\n    Set nb to rotl(nb xor nc, 12).\n");
    p.push_str("    Set na to na + nb.\n    Set nd to rotl(nd xor na, 8).\n");
    p.push_str("    Set nc to nc + nd.\n    Set nb to rotl(nb xor nc, 7).\n");
    p.push_str("    Let result be a new Seq of Lanes8Word32.\n");
    p.push_str("    Push na to result.\n    Push nb to result.\n    Push nc to result.\n    Push nd to result.\n");
    p.push_str("    Return result.\n\n");
    p.push_str("## Main\n");
    p.push_str(&format!("Let a be splat8Word32(word32({a})).\n"));
    p.push_str(&format!("Let b be splat8Word32(word32({b})).\n"));
    p.push_str(&format!("Let c be splat8Word32(word32({c})).\n"));
    p.push_str(&format!("Let d be splat8Word32(word32({d})).\n"));
    p.push_str("Let result be laneQuarterRound(a, b, c, d).\n");
    // Read lane 0 (item 1) of each of the four updated vectors.
    for i in 1..=4 {
        p.push_str(&format!("Show item 1 of seqOfLanes8(item {i} of result).\n"));
    }
    p
}

/// The reference: the scalar ChaCha quarter-round (RFC 8439 §2.2.1).
fn qr_scalar(mut a: u32, mut b: u32, mut c: u32, mut d: u32) -> [u32; 4] {
    a = a.wrapping_add(b);
    d = (d ^ a).rotate_left(16);
    c = c.wrapping_add(d);
    b = (b ^ c).rotate_left(12);
    a = a.wrapping_add(b);
    d = (d ^ a).rotate_left(8);
    c = c.wrapping_add(d);
    b = (b ^ c).rotate_left(7);
    [a, b, c, d]
}

#[test]
fn lane_chacha_quarter_round_matches_rfc_spec_tw_and_vm() {
    let (a, b, c, d) = (0x1111_1111u32, 0x0102_0304, 0x9b8d_6f43, 0x0123_4567);
    let expected = qr_scalar(a, b, c, d);
    // The Logos lanes compute the published RFC 8439 §2.2.1 vector.
    assert_eq!(
        expected,
        [0xea2a_92f4, 0xcb1c_f8ce, 0x4581_472e, 0x5881_c4bb],
        "the scalar reference is the RFC quarter-round"
    );
    let want: Vec<String> = expected.iter().map(|x| x.to_string()).collect();
    let src = quarter_round_program(a, b, c, d);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "lane quarter-round tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Logos lane quarter-round (tree-walker) == RFC 8439 §2.2.1"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "lane quarter-round VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Logos lane quarter-round (VM) == RFC 8439 §2.2.1"
    );
}

#[test]
#[ignore = "compiles via rustc (slow) — lane ChaCha quarter-round AOT(AVX2) == RFC spec"]
fn lane_chacha_quarter_round_aot_eq_rfc() {
    let (a, b, c, d) = (0x1111_1111u32, 0x0102_0304, 0x9b8d_6f43, 0x0123_4567);
    let want: Vec<String> = qr_scalar(a, b, c, d).iter().map(|x| x.to_string()).collect();
    let aot = run_logos_with_args(&quarter_round_program(a, b, c, d), &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Logos lane quarter-round (AOT/AVX2) == RFC 8439 §2.2.1"
    );
}

/// The scalar ChaCha20 block (RFC 8439 §2.3): 16 words after 20 rounds + the original state added.
fn chacha_block_scalar(key: &[u32; 8], counter: u32, nonce: &[u32; 3]) -> [u32; 16] {
    fn qr(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
        s[a] = s[a].wrapping_add(s[b]);
        s[d] = (s[d] ^ s[a]).rotate_left(16);
        s[c] = s[c].wrapping_add(s[d]);
        s[b] = (s[b] ^ s[c]).rotate_left(12);
        s[a] = s[a].wrapping_add(s[b]);
        s[d] = (s[d] ^ s[a]).rotate_left(8);
        s[c] = s[c].wrapping_add(s[d]);
        s[b] = (s[b] ^ s[c]).rotate_left(7);
    }
    let mut s = [
        0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574, key[0], key[1], key[2], key[3], key[4],
        key[5], key[6], key[7], counter, nonce[0], nonce[1], nonce[2],
    ];
    let init = s;
    for _ in 0..10 {
        qr(&mut s, 0, 4, 8, 12);
        qr(&mut s, 1, 5, 9, 13);
        qr(&mut s, 2, 6, 10, 14);
        qr(&mut s, 3, 7, 11, 15);
        qr(&mut s, 0, 5, 10, 15);
        qr(&mut s, 1, 6, 11, 12);
        qr(&mut s, 2, 7, 8, 13);
        qr(&mut s, 3, 4, 9, 14);
    }
    for i in 0..16 {
        s[i] = s[i].wrapping_add(init[i]);
    }
    s
}

/// The shared Logos definitions: the in-place lane quarter-round and the 8-way block.
fn lane_chacha_defs() -> String {
    let mut p = String::new();
    // The quarter-round, in place on the 16-word lane state (mirrors the scalar `quarterRound`).
    p.push_str(
        "## To laneQR (state: Seq of Lanes8Word32) and (ai: Int) and (bi: Int) and (ci: Int) and (di: Int) -> Seq of Lanes8Word32:\n",
    );
    p.push_str("    Let mutable a be item (ai + 1) of state.\n    Let mutable b be item (bi + 1) of state.\n");
    p.push_str("    Let mutable c be item (ci + 1) of state.\n    Let mutable d be item (di + 1) of state.\n");
    p.push_str("    Set a to a + b.\n    Set d to rotl(d xor a, 16).\n");
    p.push_str("    Set c to c + d.\n    Set b to rotl(b xor c, 12).\n");
    p.push_str("    Set a to a + b.\n    Set d to rotl(d xor a, 8).\n");
    p.push_str("    Set c to c + d.\n    Set b to rotl(b xor c, 7).\n");
    p.push_str("    Set item (ai + 1) of state to a.\n    Set item (bi + 1) of state to b.\n");
    p.push_str("    Set item (ci + 1) of state to c.\n    Set item (di + 1) of state to d.\n");
    p.push_str("    Return state.\n\n");
    // The block: build the 16-word state, 20 rounds, add the original state.
    p.push_str("## To laneChaCha20Block (key: Seq of Word32) and (counterBase: Int) and (nonce: Seq of Word32) -> Seq of Lanes8Word32:\n");
    p.push_str("    Let mutable state be a new Seq of Lanes8Word32.\n");
    for c in [1634760805u32, 857760878, 2036477234, 1797285236] {
        p.push_str(&format!("    Push splat8Word32(word32({c})) to state.\n"));
    }
    p.push_str("    Repeat for k in key:\n        Push splat8Word32(k) to state.\n");
    p.push_str("    Let mutable ctr be a new Seq of Word32.\n");
    p.push_str("    Repeat for i from 0 to 7:\n        Push word32(counterBase + i) to ctr.\n");
    p.push_str("    Push lanes8Word32(ctr) to state.\n");
    p.push_str("    Repeat for n in nonce:\n        Push splat8Word32(n) to state.\n");
    p.push_str("    Let mutable init be a new Seq of Lanes8Word32.\n");
    p.push_str("    Repeat for s in state:\n        Push s to init.\n");
    p.push_str("    Repeat for r from 1 to 10:\n");
    for (ai, bi, ci, di) in [
        (0, 4, 8, 12), (1, 5, 9, 13), (2, 6, 10, 14), (3, 7, 11, 15),
        (0, 5, 10, 15), (1, 6, 11, 12), (2, 7, 8, 13), (3, 4, 9, 14),
    ] {
        p.push_str(&format!("        Set state to laneQR(state, {ai}, {bi}, {ci}, {di}).\n"));
    }
    p.push_str("    Let mutable out be a new Seq of Lanes8Word32.\n");
    p.push_str("    Repeat for i from 1 to 16:\n        Push (item i of state) + (item i of init) to out.\n");
    p.push_str("    Return out.\n\n");
    p
}

/// The 8-way ChaCha20 block, written in Logos lanes (`laneChaCha20Block` over a 16-word
/// `Lanes8Word32` state). Lane `j` carries block `counterBase + j`. The program prints lane 0's and
/// lane 3's 16 keystream words, so the per-lane counter is exercised.
fn block_program(key: &[u32; 8], nonce: &[u32; 3], base: u32) -> String {
    let mut p = lane_chacha_defs();
    // Main: run the block, print lane 0 then lane 3 (16 words each).
    p.push_str("## Main\n");
    p.push_str("Let mutable key be a new Seq of Word32.\n");
    for k in key {
        p.push_str(&format!("Push word32({k}) to key.\n"));
    }
    p.push_str("Let mutable nonce be a new Seq of Word32.\n");
    for n in nonce {
        p.push_str(&format!("Push word32({n}) to nonce.\n"));
    }
    p.push_str(&format!("Let result be laneChaCha20Block(key, {base}, nonce).\n"));
    for lane in [1usize, 4] {
        // item `lane` of seqOfLanes8(...) = lane (lane-1); lane 1 → block base, lane 4 → block base+3.
        for j in 1..=16 {
            p.push_str(&format!("Show item {lane} of seqOfLanes8(item {j} of result).\n"));
        }
    }
    p
}

/// RFC 8439 §2.3.2 fixtures: key = 0..31, nonce = 00..00 09 | 00..00 4a | 0, counter = 1.
fn rfc_block_fixture() -> ([u32; 8], [u32; 3], u32) {
    let key: [u32; 8] = std::array::from_fn(|i| {
        u32::from_le_bytes([(4 * i) as u8, (4 * i + 1) as u8, (4 * i + 2) as u8, (4 * i + 3) as u8])
    });
    let nonce = [0x0900_0000u32, 0x4a00_0000, 0x0000_0000];
    (key, nonce, 1)
}

/// Expected output for `block_program`: lane 0 (block `base`) then lane 3 (block `base+3`).
fn block_expected(key: &[u32; 8], nonce: &[u32; 3], base: u32) -> Vec<String> {
    let mut want = Vec::new();
    for k in [0u32, 3] {
        for w in chacha_block_scalar(key, base + k, nonce) {
            want.push(w.to_string());
        }
    }
    want
}

#[test]
fn lane_chacha_block_matches_rfc_2_3_2_spec_tw_and_vm() {
    let (key, nonce, base) = rfc_block_fixture();
    // Sanity: the scalar oracle reproduces the published RFC 8439 §2.3.2 keystream block.
    assert_eq!(
        chacha_block_scalar(&key, base, &nonce),
        [
            0xe4e7_f110, 0x1559_3bd1, 0x1fdd_0f50, 0xc471_20a3, 0xc7f4_d1c7, 0x0368_c033,
            0x9aaa_2204, 0x4e6c_d4c3, 0x4664_82d2, 0x09aa_9f07, 0x05d7_c214, 0xa202_8bd9,
            0xd19c_12b5, 0xb94e_16de, 0xe883_d0cb, 0x4e3c_50a2,
        ],
        "scalar oracle == RFC 8439 §2.3.2 block"
    );
    let want = block_expected(&key, &nonce, base);
    let src = block_program(&key, &nonce, base);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "lane block tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Logos 8-way ChaCha block (tree-walker) == scalar/RFC, lanes 0 & 3"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "lane block VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Logos 8-way ChaCha block (VM) == scalar/RFC, lanes 0 & 3"
    );
}

#[test]
#[ignore = "compiles via rustc (slow) — the 8-way Logos ChaCha block AOT(AVX2) == scalar/RFC"]
fn lane_chacha_block_aot_eq_spec() {
    let (key, nonce, base) = rfc_block_fixture();
    let want = block_expected(&key, &nonce, base);
    let aot = run_logos_with_args(&block_program(&key, &nonce, base), &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Logos 8-way ChaCha block (AOT/AVX2) == scalar/RFC, lanes 0 & 3"
    );
}

/// The full 8-way keystream flattened to 128 block-major words (block 0's 16 words, then block 1's,
/// …) — the lane-major→block-major transpose, written in Logos with `seqOfLanes8`. This is the whole
/// keystream the cipher XORs against the plaintext, computed entirely from Logos lane source.
fn keystream_program(key: &[u32; 8], nonce: &[u32; 3], base: u32) -> String {
    let mut p = lane_chacha_defs();
    p.push_str("## To laneKeystreamWords (key: Seq of Word32) and (counterBase: Int) and (nonce: Seq of Word32) -> Seq of Word32:\n");
    p.push_str("    Let block be laneChaCha20Block(key, counterBase, nonce).\n");
    p.push_str("    Let out be a new Seq of Word32.\n");
    p.push_str("    Repeat for lane from 1 to 8:\n");
    p.push_str("        Repeat for j from 1 to 16:\n");
    p.push_str("            Push (item lane of seqOfLanes8(item j of block)) to out.\n");
    p.push_str("    Return out.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable key be a new Seq of Word32.\n");
    for k in key {
        p.push_str(&format!("Push word32({k}) to key.\n"));
    }
    p.push_str("Let mutable nonce be a new Seq of Word32.\n");
    for n in nonce {
        p.push_str(&format!("Push word32({n}) to nonce.\n"));
    }
    p.push_str(&format!("Let ks be laneKeystreamWords(key, {base}, nonce).\n"));
    p.push_str("Repeat for i from 1 to 128:\n    Show item i of ks.\n");
    p
}

/// The expected keystream: the 8 scalar blocks (counters `base..base+7`) concatenated, 128 words.
fn keystream_expected(key: &[u32; 8], nonce: &[u32; 3], base: u32) -> Vec<String> {
    let mut want = Vec::new();
    for k in 0..8u32 {
        for w in chacha_block_scalar(key, base + k, nonce) {
            want.push(w.to_string());
        }
    }
    want
}

#[test]
fn lane_chacha_keystream_words_match_8_scalar_blocks_tw_and_vm() {
    let (key, nonce, base) = rfc_block_fixture();
    let want = keystream_expected(&key, &nonce, base);
    assert_eq!(want.len(), 128, "8 blocks × 16 words");
    let src = keystream_program(&key, &nonce, base);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "keystream tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "8-way Logos keystream (tree-walker) == 8 scalar ChaCha blocks"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "keystream VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "8-way Logos keystream (VM) == 8 scalar ChaCha blocks"
    );
}

#[test]
#[ignore = "compiles via rustc (slow) — the full 8-way Logos keystream AOT(AVX2) == 8 scalar blocks"]
fn lane_chacha_keystream_words_aot_eq_spec() {
    let (key, nonce, base) = rfc_block_fixture();
    let want = keystream_expected(&key, &nonce, base);
    let aot = run_logos_with_args(&keystream_program(&key, &nonce, base), &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "8-way Logos keystream (AOT/AVX2) == 8 scalar ChaCha blocks"
    );
}

/// The scalar ChaCha20 stream cipher (RFC 8439 §2.4): `data ⊕ keystream`.
fn chacha_xor_scalar(key: &[u32; 8], counter: u32, nonce: &[u32; 3], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    for (bi, chunk) in data.chunks(64).enumerate() {
        let block = chacha_block_scalar(key, counter + bi as u32, nonce);
        let mut ks = [0u8; 64];
        for i in 0..16 {
            ks[i * 4..i * 4 + 4].copy_from_slice(&block[i].to_le_bytes());
        }
        for (j, &b) in chunk.iter().enumerate() {
            out.push(b ^ ks[j]);
        }
    }
    out
}

/// The ChaCha20 stream cipher, written ENTIRELY in Logos lanes (`laneChaCha20Xor`): the 8-way block
/// → serialize the keystream words to bytes (`intOfWord32` + `Int` mod/div) → XOR the payload. One
/// 512-byte batch (the §2.4.2 plaintext is 114 bytes).
fn cipher_program(key: &[u32; 8], nonce: &[u32; 3], counter: u32, plaintext: &[u8]) -> String {
    let mut p = lane_chacha_defs();
    p.push_str("## To laneKeystreamWords (key: Seq of Word32) and (counterBase: Int) and (nonce: Seq of Word32) -> Seq of Word32:\n");
    p.push_str("    Let block be laneChaCha20Block(key, counterBase, nonce).\n");
    p.push_str("    Let out be a new Seq of Word32.\n");
    p.push_str("    Repeat for lane from 1 to 8:\n        Repeat for j from 1 to 16:\n");
    p.push_str("            Push (item lane of seqOfLanes8(item j of block)) to out.\n");
    p.push_str("    Return out.\n\n");
    p.push_str("## To laneChaCha20Xor (key: Seq of Word32) and (counter: Int) and (nonce: Seq of Word32) and (data: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let ks be laneKeystreamWords(key, counter, nonce).\n");
    p.push_str("    Let mutable bytes be a new Seq of Int.\n");
    p.push_str("    Repeat for w in ks:\n");
    p.push_str("        Let v be intOfWord32(w).\n");
    p.push_str("        Push (v % 256) to bytes.\n");
    p.push_str("        Push ((v / 256) % 256) to bytes.\n");
    p.push_str("        Push ((v / 65536) % 256) to bytes.\n");
    p.push_str("        Push ((v / 16777216) % 256) to bytes.\n");
    p.push_str("    Let mutable ct be a new Seq of Int.\n");
    p.push_str("    Let n be length of data.\n");
    p.push_str("    Repeat for i from 1 to n:\n");
    p.push_str("        Push ((item i of data) xor (item i of bytes)) to ct.\n");
    p.push_str("    Return ct.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable key be a new Seq of Word32.\n");
    for k in key {
        p.push_str(&format!("Push word32({k}) to key.\n"));
    }
    p.push_str("Let mutable nonce be a new Seq of Word32.\n");
    for n in nonce {
        p.push_str(&format!("Push word32({n}) to nonce.\n"));
    }
    p.push_str("Let mutable data be a new Seq of Int.\n");
    for &b in plaintext {
        p.push_str(&format!("Push {b} to data.\n"));
    }
    p.push_str(&format!("Let ct be laneChaCha20Xor(key, {counter}, nonce, data).\n"));
    p.push_str("Repeat for i from 1 to length of ct:\n    Show item i of ct.\n");
    p
}

const SUNSCREEN: &[u8] = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";

#[test]
fn lane_chacha_cipher_matches_rfc_2_4_2_spec_tw_and_vm() {
    // RFC 8439 §2.4.2: key = 0..31, nonce = 0 | 0x4a000000 | 0, counter = 1.
    let key: [u32; 8] = std::array::from_fn(|i| {
        u32::from_le_bytes([(4 * i) as u8, (4 * i + 1) as u8, (4 * i + 2) as u8, (4 * i + 3) as u8])
    });
    let nonce = [0x0000_0000u32, 0x4a00_0000, 0x0000_0000];
    let counter = 1u32;
    let ct = chacha_xor_scalar(&key, counter, &nonce, SUNSCREEN);
    // Anchor to the published vector: the RFC §2.4.2 ciphertext begins 6e 2e 35 9a 25 68 f9 80.
    assert_eq!(&ct[..8], &[0x6e, 0x2e, 0x35, 0x9a, 0x25, 0x68, 0xf9, 0x80], "RFC 8439 §2.4.2 ciphertext prefix");
    let want: Vec<String> = ct.iter().map(|b| b.to_string()).collect();
    let src = cipher_program(&key, &nonce, counter, SUNSCREEN);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "lane cipher tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Logos lane ChaCha20 cipher (tree-walker) == scalar/RFC §2.4.2"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "lane cipher VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Logos lane ChaCha20 cipher (VM) == scalar/RFC §2.4.2"
    );
}

#[test]
#[ignore = "compiles via rustc (slow) — the full Logos lane ChaCha20 cipher AOT(AVX2) == RFC §2.4.2"]
fn lane_chacha_cipher_aot_eq_rfc() {
    let key: [u32; 8] = std::array::from_fn(|i| {
        u32::from_le_bytes([(4 * i) as u8, (4 * i + 1) as u8, (4 * i + 2) as u8, (4 * i + 3) as u8])
    });
    let nonce = [0x0000_0000u32, 0x4a00_0000, 0x0000_0000];
    let want: Vec<String> =
        chacha_xor_scalar(&key, 1, &nonce, SUNSCREEN).iter().map(|b| b.to_string()).collect();
    let aot = run_logos_with_args(&cipher_program(&key, &nonce, 1, SUNSCREEN), &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Logos lane ChaCha20 cipher (AOT/AVX2) == scalar/RFC §2.4.2"
    );
}

/// The COMPLETE Poly1305 MAC, written in Logos: clamp `r`, block→limbs, precompute `r¹…r⁴`, the
/// 4-way group loop (`poly1305Group` over `Lanes4Word64` → `vpmuludq`), and finalize (+`s`). Every
/// bit-twiddle is `/`·`%` by a power of two, so no shift/bitwise surface is needed. Proven against
/// the RFC-validated scalar `poly1305`. `msg` must be a multiple of 64 bytes (whole 4-block groups).
fn mac_program(key: &[u8; 32], msg: &[u8]) -> String {
    let mut p = String::new();
    p.push_str("## To leWord (b: Seq of Int) and (off: Int) -> Int:\n");
    p.push_str("    Return (item (off + 1) of b) + (item (off + 2) of b) * 256 + (item (off + 3) of b) * 65536 + (item (off + 4) of b) * 16777216.\n\n");
    p.push_str("## To polyReduce (d: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let c be (item 1 of d) / 67108864.\n    Let h0 be (item 1 of d) % 67108864.\n");
    p.push_str("    Let d1b be (item 2 of d) + c.\n    Let c1 be d1b / 67108864.\n    Let h1 be d1b % 67108864.\n");
    p.push_str("    Let d2b be (item 3 of d) + c1.\n    Let c2 be d2b / 67108864.\n    Let h2 be d2b % 67108864.\n");
    p.push_str("    Let d3b be (item 4 of d) + c2.\n    Let c3 be d3b / 67108864.\n    Let h3 be d3b % 67108864.\n");
    p.push_str("    Let d4b be (item 5 of d) + c3.\n    Let c4 be d4b / 67108864.\n    Let h4 be d4b % 67108864.\n");
    p.push_str("    Let h0w be h0 + c4 * 5.\n    Let h0f be h0w % 67108864.\n    Let cw be h0w / 67108864.\n");
    p.push_str("    Let h1b be h1 + cw.\n    Let c1b be h1b / 67108864.\n    Let h1f be h1b % 67108864.\n    Let h2f be h2 + c1b.\n");
    p.push_str("    Let r be a new Seq of Int.\n    Push h0f to r.\n    Push h1f to r.\n    Push h2f to r.\n    Push h3 to r.\n    Push h4 to r.\n    Return r.\n\n");
    p.push_str("## To polyClamp (key: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let t0c be (leWord(key, 0)) % 268435456.\n");
    p.push_str("    Let t1a be (leWord(key, 4)) % 268435456.\n    Let t1c be t1a - (t1a % 4).\n");
    p.push_str("    Let t2a be (leWord(key, 8)) % 268435456.\n    Let t2c be t2a - (t2a % 4).\n");
    p.push_str("    Let t3a be (leWord(key, 12)) % 268435456.\n    Let t3c be t3a - (t3a % 4).\n");
    p.push_str("    Let r be a new Seq of Int.\n");
    p.push_str("    Push (t0c % 67108864) to r.\n");
    p.push_str("    Push ((t0c / 67108864) + (t1c % 1048576) * 64) to r.\n");
    p.push_str("    Push ((t1c / 1048576) + (t2c % 16384) * 4096) to r.\n");
    p.push_str("    Push ((t2c / 16384) + (t3c % 256) * 262144) to r.\n");
    p.push_str("    Push (t3c / 256) to r.\n    Return r.\n\n");
    // A 16-byte block of `seq` at `off` → 5×26 limbs; `hibit`=1 sets the 2¹²⁸ marker (full block).
    p.push_str("## To blockLimbs (seq: Seq of Int) and (off: Int) and (hibit: Int) -> Seq of Int:\n");
    p.push_str("    Let m0 be leWord(seq, off).\n    Let m1 be leWord(seq, off + 4).\n    Let m2 be leWord(seq, off + 8).\n    Let m3 be leWord(seq, off + 12).\n");
    p.push_str("    Let b be a new Seq of Int.\n");
    p.push_str("    Push (m0 % 67108864) to b.\n");
    p.push_str("    Push ((m0 / 67108864) + (m1 % 1048576) * 64) to b.\n");
    p.push_str("    Push ((m1 / 1048576) + (m2 % 16384) * 4096) to b.\n");
    p.push_str("    Push ((m2 / 16384) + (m3 % 256) * 262144) to b.\n");
    p.push_str("    Push ((m3 / 256) + hibit * 16777216) to b.\n    Return b.\n\n");
    // A partial last block: `len` (1..15) data bytes, then the appended `1`, zero-filled to 16.
    p.push_str("## To padBlock (msg: Seq of Int) and (base: Int) and (len: Int) -> Seq of Int:\n");
    p.push_str("    Let b be a new Seq of Int.\n");
    p.push_str("    Repeat for i from 1 to len:\n        Push (item (base + i) of msg) to b.\n");
    p.push_str("    Push 1 to b.\n");
    p.push_str("    Repeat for i from (len + 2) to 16:\n        Push 0 to b.\n");
    p.push_str("    Return b.\n\n");
    p.push_str("## To polyMul (a: Seq of Int) and (b: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let a0 be item 1 of a.\n    Let a1 be item 2 of a.\n    Let a2 be item 3 of a.\n    Let a3 be item 4 of a.\n    Let a4 be item 5 of a.\n");
    p.push_str("    Let b0 be item 1 of b.\n    Let b1 be item 2 of b.\n    Let b2 be item 3 of b.\n    Let b3 be item 4 of b.\n    Let b4 be item 5 of b.\n");
    p.push_str("    Let s1 be b1 * 5.\n    Let s2 be b2 * 5.\n    Let s3 be b3 * 5.\n    Let s4 be b4 * 5.\n");
    p.push_str("    Let d be a new Seq of Int.\n");
    p.push_str("    Push (a0*b0 + a1*s4 + a2*s3 + a3*s2 + a4*s1) to d.\n");
    p.push_str("    Push (a0*b1 + a1*b0 + a2*s4 + a3*s3 + a4*s2) to d.\n");
    p.push_str("    Push (a0*b2 + a1*b1 + a2*b0 + a3*s4 + a4*s3) to d.\n");
    p.push_str("    Push (a0*b3 + a1*b2 + a2*b1 + a3*b0 + a4*s4) to d.\n");
    p.push_str("    Push (a0*b4 + a1*b3 + a2*b2 + a3*b1 + a4*b0) to d.\n");
    p.push_str("    Return polyReduce(d).\n\n");
    p.push_str("## To polyAdd (a: Seq of Int) and (b: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let r be a new Seq of Int.\n    Repeat for i from 1 to 5:\n        Push ((item i of a) + (item i of b)) to r.\n    Return r.\n\n");
    p.push_str("## To laneOfLimbs (x: Seq of Int) and (l: Int) -> Lanes4Word64:\n");
    p.push_str("    Let s be a new Seq of Int.\n");
    p.push_str("    Push (item (l + 1) of x) to s.\n    Push (item (l + 6) of x) to s.\n    Push (item (l + 11) of x) to s.\n    Push (item (l + 16) of x) to s.\n");
    p.push_str("    Return lanes4Word64(s).\n\n");
    p.push_str("## To poly1305Group (t: Seq of Int) and (r: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let r5 be a new Seq of Int.\n    Repeat for x in r:\n        Push (x * 5) to r5.\n");
    for l in 0..5 {
        p.push_str(&format!("    Let va{l} be laneOfLimbs(t, {l}).\n"));
    }
    for l in 0..5 {
        p.push_str(&format!("    Let vb{l} be laneOfLimbs(r, {l}).\n"));
    }
    for l in 0..5 {
        p.push_str(&format!("    Let vs{l} be laneOfLimbs(r5, {l}).\n"));
    }
    let rows = [
        ("va0, vb0", "va1, vs4", "va2, vs3", "va3, vs2", "va4, vs1"),
        ("va0, vb1", "va1, vb0", "va2, vs4", "va3, vs3", "va4, vs2"),
        ("va0, vb2", "va1, vb1", "va2, vb0", "va3, vs4", "va4, vs3"),
        ("va0, vb3", "va1, vb2", "va2, vb1", "va3, vb0", "va4, vs4"),
        ("va0, vb4", "va1, vb3", "va2, vb2", "va3, vb1", "va4, vb0"),
    ];
    p.push_str("    Let dd be a new Seq of Int.\n");
    for (a, b, c, e, f) in rows {
        p.push_str(&format!(
            "    Push hsumLanes4(((mul32x32to64({a}) + mul32x32to64({b})) + (mul32x32to64({c}) + mul32x32to64({e}))) + mul32x32to64({f})) to dd.\n"
        ));
    }
    p.push_str("    Return polyReduce(dd).\n\n");
    p.push_str("## To polyFinalize (h: Seq of Int) and (key: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let h0 be item 1 of h.\n    Let h2 be item 3 of h.\n    Let h3 be item 4 of h.\n    Let h4 be item 5 of h.\n");
    p.push_str("    Let c1 be (item 2 of h) / 67108864.\n    Let h1n be (item 2 of h) % 67108864.\n    Let h2a be h2 + c1.\n");
    p.push_str("    Let c2 be h2a / 67108864.\n    Let h2n be h2a % 67108864.\n    Let h3a be h3 + c2.\n");
    p.push_str("    Let c3 be h3a / 67108864.\n    Let h3n be h3a % 67108864.\n    Let h4a be h4 + c3.\n");
    p.push_str("    Let c4 be h4a / 67108864.\n    Let h4n be h4a % 67108864.\n    Let h0a be h0 + c4 * 5.\n");
    p.push_str("    Let c0 be h0a / 67108864.\n    Let h0n be h0a % 67108864.\n    Let h1b be h1n + c0.\n");
    p.push_str("    Let c1c be h1b / 67108864.\n    Let h1m be h1b % 67108864.\n    Let h2m be h2n + c1c.\n");
    p.push_str("    Let g0 be h0n + 5.\n    Let cg0 be g0 / 67108864.\n    Let g0n be g0 % 67108864.\n");
    p.push_str("    Let g1 be h1m + cg0.\n    Let cg1 be g1 / 67108864.\n    Let g1n be g1 % 67108864.\n");
    p.push_str("    Let g2 be h2m + cg1.\n    Let cg2 be g2 / 67108864.\n    Let g2n be g2 % 67108864.\n");
    p.push_str("    Let g3 be h3n + cg2.\n    Let cg3 be g3 / 67108864.\n    Let g3n be g3 % 67108864.\n");
    p.push_str("    Let g4 be h4n + cg3.\n");
    p.push_str("    Let mutable f0 be h0n.\n    Let mutable f1 be h1m.\n    Let mutable f2 be h2m.\n    Let mutable f3 be h3n.\n    Let mutable f4 be h4n.\n");
    p.push_str("    If g4 is at least 67108864:\n");
    p.push_str("        Set f0 to g0n.\n        Set f1 to g1n.\n        Set f2 to g2n.\n        Set f3 to g3n.\n        Set f4 to (g4 - 67108864).\n");
    p.push_str("    Let w0 be f0 + (f1 % 64) * 67108864.\n");
    p.push_str("    Let w1 be (f1 / 64) + (f2 % 4096) * 1048576.\n");
    p.push_str("    Let w2 be (f2 / 4096) + (f3 % 262144) * 16384.\n");
    p.push_str("    Let w3 be (f3 / 262144) + (f4 % 16777216) * 256.\n");
    p.push_str("    Let acc0 be w0 + leWord(key, 16).\n    Let v0 be acc0 % 4294967296.\n");
    p.push_str("    Let acc1 be w1 + leWord(key, 20) + (acc0 / 4294967296).\n    Let v1 be acc1 % 4294967296.\n");
    p.push_str("    Let acc2 be w2 + leWord(key, 24) + (acc1 / 4294967296).\n    Let v2 be acc2 % 4294967296.\n");
    p.push_str("    Let acc3 be w3 + leWord(key, 28) + (acc2 / 4294967296).\n    Let v3 be acc3 % 4294967296.\n");
    p.push_str("    Let tag be a new Seq of Int.\n");
    for v in ["v0", "v1", "v2", "v3"] {
        p.push_str(&format!("    Push ({v} % 256) to tag.\n    Push (({v} / 256) % 256) to tag.\n    Push (({v} / 65536) % 256) to tag.\n    Push (({v} / 16777216) % 256) to tag.\n"));
    }
    p.push_str("    Return tag.\n\n");
    p.push_str("## To poly1305MacLanes (key: Seq of Int) and (msg: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let r1 be polyClamp(key).\n    Let r2 be polyMul(r1, r1).\n    Let r3 be polyMul(r2, r1).\n    Let r4 be polyMul(r2, r2).\n");
    p.push_str("    Let mult be a new Seq of Int.\n");
    p.push_str("    Repeat for x in r4:\n        Push x to mult.\n    Repeat for x in r3:\n        Push x to mult.\n    Repeat for x in r2:\n        Push x to mult.\n    Repeat for x in r1:\n        Push x to mult.\n");
    p.push_str("    Let n be length of msg.\n    Let numGroups be n / 64.\n");
    p.push_str("    Let mutable h be a new Seq of Int.\n    Repeat for i from 1 to 5:\n        Push 0 to h.\n");
    p.push_str("    Repeat for g from 0 to numGroups - 1:\n");
    p.push_str("        Let base be g * 64.\n");
    p.push_str("        Let m0 be blockLimbs(msg, base, 1).\n        Let m1 be blockLimbs(msg, base + 16, 1).\n        Let m2 be blockLimbs(msg, base + 32, 1).\n        Let m3 be blockLimbs(msg, base + 48, 1).\n");
    p.push_str("        Let t0 be polyAdd(h, m0).\n");
    p.push_str("        Let t be a new Seq of Int.\n");
    p.push_str("        Repeat for x in t0:\n            Push x to t.\n        Repeat for x in m1:\n            Push x to t.\n        Repeat for x in m2:\n            Push x to t.\n        Repeat for x in m3:\n            Push x to t.\n");
    p.push_str("        Set h to poly1305Group(t, mult).\n");
    // Scalar tail: the remaining full 16-byte blocks, then a partial last block (1-appended).
    p.push_str("    Let mutable i be numGroups * 64.\n");
    p.push_str("    While (i + 16) is at most n:\n");
    p.push_str("        Let hm be polyAdd(h, blockLimbs(msg, i, 1)).\n");
    p.push_str("        Set h to polyMul(hm, r1).\n");
    p.push_str("        Set i to i + 16.\n");
    p.push_str("    If i is at most (n - 1):\n");
    p.push_str("        Let lenp be n - i.\n");
    p.push_str("        Let hp be polyAdd(h, blockLimbs(padBlock(msg, i, lenp), 0, 0)).\n");
    p.push_str("        Set h to polyMul(hp, r1).\n");
    p.push_str("    Return polyFinalize(h, key).\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable key be a new Seq of Int.\n");
    for &b in key {
        p.push_str(&format!("Push {b} to key.\n"));
    }
    p.push_str("Let mutable msg be a new Seq of Int.\n");
    for &b in msg {
        p.push_str(&format!("Push {b} to msg.\n"));
    }
    p.push_str("Let tag be poly1305MacLanes(key, msg).\n");
    p.push_str("Repeat for i from 1 to 16:\n    Show item i of tag.\n");
    p
}

#[test]
fn poly1305_full_mac_lanes_matches_scalar_tw_and_vm() {
    // The Logos lane MAC must equal the RFC-validated scalar poly1305 over EVERY block alignment:
    // partial-only, exact blocks, full+partial, single/multi group, group+tail+partial.
    let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(37).wrapping_add(11));
    for len in [1usize, 16, 17, 34, 63, 64, 65, 100, 128, 200] {
        let msg: Vec<u8> = (0..len).map(|i| (i as u8).wrapping_mul(53).wrapping_add(7)).collect();
        let want: Vec<String> =
            logicaffeine_system::aead::poly1305(&key, &msg).iter().map(|b| b.to_string()).collect();
        let src = mac_program(&key, &msg);
        let tw = tw_outcome(&src);
        assert!(tw.error.is_none(), "{len}B MAC tree-walker errored: {:?}", tw.error);
        assert_eq!(
            tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            want,
            "{len}B Poly1305 MAC in Logos lanes (tree-walker) == scalar poly1305"
        );
        let vm = vm_outcome(&src);
        assert!(vm.error.is_none(), "{len}B MAC VM errored: {:?}", vm.error);
        assert_eq!(
            vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            want,
            "{len}B Poly1305 MAC in Logos lanes (VM) == scalar poly1305"
        );
    }
}

#[test]
fn poly1305_mac_lanes_matches_rfc_2_5_2_tw_and_vm() {
    // RFC 8439 §2.5.2: key = r‖s, msg = "Cryptographic Forum Research Group" → the published tag.
    let key: [u8; 32] = [
        0x85, 0xd6, 0xbe, 0x78, 0x57, 0x55, 0x6d, 0x33, 0x7f, 0x44, 0x52, 0xfe, 0x42, 0xd5, 0x06,
        0xa8, 0x01, 0x03, 0x80, 0x8a, 0xfb, 0x0d, 0xb2, 0xfd, 0x4a, 0xbf, 0xf6, 0xaf, 0x41, 0x49,
        0xf5, 0x1b,
    ];
    let msg = b"Cryptographic Forum Research Group";
    let rfc_tag: [u8; 16] = [
        0xa8, 0x06, 0x1d, 0xc1, 0x30, 0x51, 0x36, 0xc6, 0xc2, 0x2b, 0x8b, 0xaf, 0x0c, 0x01, 0x27,
        0xa9,
    ];
    // Cross-check the oracle reproduces the published vector, then hold the Logos MAC to it.
    assert_eq!(logicaffeine_system::aead::poly1305(&key, msg), rfc_tag, "oracle == RFC §2.5.2");
    let want: Vec<String> = rfc_tag.iter().map(|b| b.to_string()).collect();
    let src = mac_program(&key, msg);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "RFC MAC tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Logos Poly1305 MAC (tree-walker) == RFC 8439 §2.5.2"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "RFC MAC VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Logos Poly1305 MAC (VM) == RFC 8439 §2.5.2"
    );
}

#[test]
#[ignore = "compiles via rustc (slow) — the full Logos Poly1305 MAC AOT(AVX2) == scalar, group+tail+partial"]
fn poly1305_full_mac_lanes_aot_eq_scalar() {
    let key: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(29).wrapping_add(5));
    // 100 B = one 4-block group + two full tail blocks + a 4-byte partial.
    let msg: Vec<u8> = (0..100).map(|i| (i as u8).wrapping_mul(41).wrapping_add(3)).collect();
    let want: Vec<String> =
        logicaffeine_system::aead::poly1305(&key, &msg).iter().map(|b| b.to_string()).collect();
    let aot = run_logos_with_args(&mac_program(&key, &msg), &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "full Poly1305 MAC in Logos lanes (AOT/AVX2) == scalar poly1305"
    );
}

/// The 4-way Poly1305 group multiply, in Logos lanes: `(t₀·r⁴ + t₁·r³ + t₂·r² + t₃·r¹) mod (2¹³⁰−5)`
/// over 5×26-bit limbs. The four products run in the four lanes (`mul32x32to64` = `vpmuludq`), the
/// schoolbook terms accumulate by lane add, `hsumLanes4` combines the lanes, then a scalar carry
/// reduce — exactly the inner loop of the AVX2 Poly1305, written in Logos.
fn poly_group_program(t: &[i64; 20], r: &[i64; 20]) -> String {
    let mut p = String::new();
    // The l-th limb of each of the four numbers packed into one lane vector.
    p.push_str("## To laneOfLimbs (x: Seq of Int) and (l: Int) -> Lanes4Word64:\n");
    p.push_str("    Let s be a new Seq of Int.\n");
    p.push_str("    Push (item (l + 1) of x) to s.\n    Push (item (l + 6) of x) to s.\n");
    p.push_str("    Push (item (l + 11) of x) to s.\n    Push (item (l + 16) of x) to s.\n");
    p.push_str("    Return lanes4Word64(s).\n\n");
    p.push_str("## To poly1305Group (t: Seq of Int) and (r: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let r5 be a new Seq of Int.\n    Repeat for x in r:\n        Push (x * 5) to r5.\n");
    for l in 0..5 {
        p.push_str(&format!("    Let va{l} be laneOfLimbs(t, {l}).\n"));
    }
    for l in 0..5 {
        p.push_str(&format!("    Let vb{l} be laneOfLimbs(r, {l}).\n"));
    }
    for l in 0..5 {
        p.push_str(&format!("    Let vs{l} be laneOfLimbs(r5, {l}).\n"));
    }
    // The 5×26 schoolbook (s_i = 5·b_i for the 2¹³⁰ wrap), summed across the 4 product lanes.
    let rows = [
        ("d0", "va0, vb0", "va1, vs4", "va2, vs3", "va3, vs2", "va4, vs1"),
        ("d1", "va0, vb1", "va1, vb0", "va2, vs4", "va3, vs3", "va4, vs2"),
        ("d2", "va0, vb2", "va1, vb1", "va2, vb0", "va3, vs4", "va4, vs3"),
        ("d3", "va0, vb3", "va1, vb2", "va2, vb1", "va3, vb0", "va4, vs4"),
        ("d4", "va0, vb4", "va1, vb3", "va2, vb2", "va3, vb1", "va4, vb0"),
    ];
    for (d, a, b, c, e, f) in rows {
        p.push_str(&format!(
            "    Let {d} be hsumLanes4(((mul32x32to64({a}) + mul32x32to64({b})) + (mul32x32to64({c}) + mul32x32to64({e}))) + mul32x32to64({f})).\n"
        ));
    }
    // Carry-reduce (mod 2¹³⁰−5): 2²⁶ = 67108864; the top carry folds in via ·5.
    p.push_str("    Let c be d0 / 67108864.\n    Let h0 be d0 % 67108864.\n");
    p.push_str("    Let d1b be d1 + c.\n    Let c1 be d1b / 67108864.\n    Let h1 be d1b % 67108864.\n");
    p.push_str("    Let d2b be d2 + c1.\n    Let c2 be d2b / 67108864.\n    Let h2 be d2b % 67108864.\n");
    p.push_str("    Let d3b be d3 + c2.\n    Let c3 be d3b / 67108864.\n    Let h3 be d3b % 67108864.\n");
    p.push_str("    Let d4b be d4 + c3.\n    Let c4 be d4b / 67108864.\n    Let h4 be d4b % 67108864.\n");
    p.push_str("    Let h0w be h0 + c4 * 5.\n    Let h0f be h0w % 67108864.\n    Let cw be h0w / 67108864.\n");
    p.push_str("    Let h1b be h1 + cw.\n    Let c1b be h1b / 67108864.\n    Let h1f be h1b % 67108864.\n");
    p.push_str("    Let h2f be h2 + c1b.\n");
    p.push_str("    Let result be a new Seq of Int.\n");
    p.push_str("    Push h0f to result.\n    Push h1f to result.\n    Push h2f to result.\n    Push h3 to result.\n    Push h4 to result.\n");
    p.push_str("    Return result.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable t be a new Seq of Int.\n");
    for &x in t {
        p.push_str(&format!("Push {x} to t.\n"));
    }
    p.push_str("Let mutable r be a new Seq of Int.\n");
    for &x in r {
        p.push_str(&format!("Push {x} to r.\n"));
    }
    p.push_str("Let h be poly1305Group(t, r).\n");
    p.push_str("Repeat for i from 1 to 5:\n    Show item i of h.\n");
    p
}

/// The scalar reference for the 4-way group multiply — the schoolbook over the 4 products, summed,
/// then the same carry reduce. Computed in `i128` so the raw limbs never overflow.
fn poly_group_scalar(t: &[i64; 20], r: &[i64; 20]) -> [i64; 5] {
    let r5: [i64; 20] = core::array::from_fn(|i| r[i] * 5);
    let lane = |x: &[i64; 20], k: usize| -> [i128; 5] { core::array::from_fn(|l| x[k * 5 + l] as i128) };
    let mut d = [0i128; 5];
    for k in 0..4 {
        let a = lane(t, k);
        let b = lane(r, k);
        let s = lane(&r5, k);
        d[0] += a[0] * b[0] + a[1] * s[4] + a[2] * s[3] + a[3] * s[2] + a[4] * s[1];
        d[1] += a[0] * b[1] + a[1] * b[0] + a[2] * s[4] + a[3] * s[3] + a[4] * s[2];
        d[2] += a[0] * b[2] + a[1] * b[1] + a[2] * b[0] + a[3] * s[4] + a[4] * s[3];
        d[3] += a[0] * b[3] + a[1] * b[2] + a[2] * b[1] + a[3] * b[0] + a[4] * s[4];
        d[4] += a[0] * b[4] + a[1] * b[3] + a[2] * b[2] + a[3] * b[1] + a[4] * b[0];
    }
    let m = 67_108_864i128;
    let c = d[0] / m;
    let h0 = d[0] % m;
    let d1 = d[1] + c;
    let h1 = d1 % m;
    let d2 = d[2] + d1 / m;
    let h2 = d2 % m;
    let d3 = d[3] + d2 / m;
    let h3 = d3 % m;
    let d4 = d[4] + d3 / m;
    let h4 = d4 % m;
    let h0w = h0 + (d4 / m) * 5;
    let h1b = h1 + h0w / m;
    [(h0w % m) as i64, (h1b % m) as i64, (h2 + h1b / m) as i64, h3 as i64, h4 as i64]
}

fn rand26_limbs(seed: u64) -> ([i64; 20], [i64; 20]) {
    let mut s = seed;
    let mut next = || {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        ((s >> 38) as i64) & 0x3ff_ffff
    };
    (core::array::from_fn(|_| next()), core::array::from_fn(|_| next()))
}

#[test]
fn poly1305_group_multiply_lanes_match_scalar_tw_and_vm() {
    let (t, r) = rand26_limbs(0x504f_4c59_3133_3035);
    let want: Vec<String> = poly_group_scalar(&t, &r).iter().map(|x| x.to_string()).collect();
    let src = poly_group_program(&t, &r);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "poly1305 group tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "4-way Poly1305 group multiply (tree-walker) == scalar"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "poly1305 group VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "4-way Poly1305 group multiply (VM) == scalar"
    );
}

#[test]
#[ignore = "compiles via rustc (slow) — the 4-way Poly1305 group multiply AOT(AVX2) == scalar"]
fn poly1305_group_multiply_lanes_aot_eq_scalar() {
    let (t, r) = rand26_limbs(0x1305_2468_ace0_1357);
    let want: Vec<String> = poly_group_scalar(&t, &r).iter().map(|x| x.to_string()).collect();
    let aot = run_logos_with_args(&poly_group_program(&t, &r), &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "4-way Poly1305 group multiply (AOT/AVX2 vpmuludq) == scalar"
    );
}

/// The Poly1305 lane foundation: a 4-way widening multiply (`mul32x32to64` = `vpmuludq`), a
/// horizontal sum, and lane-wise 64-bit add over `Lanes4Word64` — the exact ops the 4-block
/// Poly1305 group multiply is built from.
fn lanes4_program(t: &[u64; 4], r: &[u64; 4]) -> String {
    let mut p = String::from("## Main\n");
    p.push_str("Let mutable t be a new Seq of Int.\n");
    for &x in t {
        p.push_str(&format!("Push {x} to t.\n"));
    }
    p.push_str("Let mutable r be a new Seq of Int.\n");
    for &x in r {
        p.push_str(&format!("Push {x} to r.\n"));
    }
    p.push_str("Let tv be lanes4Word64(t).\n");
    p.push_str("Let rv be lanes4Word64(r).\n");
    p.push_str("Let prod be mul32x32to64(tv, rv).\n");
    p.push_str("Show hsumLanes4(prod).\n");
    p.push_str("Let sum be tv + rv.\n");
    p.push_str("Let s be seqOfLanes4(sum).\n");
    p.push_str("Show item 1 of s.\nShow item 2 of s.\nShow item 3 of s.\nShow item 4 of s.\n");
    p
}

fn lanes4_expected(t: &[u64; 4], r: &[u64; 4]) -> Vec<String> {
    let hsum: u64 = (0..4).map(|i| (t[i] & 0xffff_ffff) * (r[i] & 0xffff_ffff)).sum();
    let mut want = vec![hsum.to_string()];
    for i in 0..4 {
        want.push(t[i].wrapping_add(r[i]).to_string());
    }
    want
}

#[test]
fn lanes4word64_mul_add_hsum_spec_tw_and_vm() {
    let t = [100u64, 200, 300, 400];
    let r = [7u64, 11, 13, 17];
    let want = lanes4_expected(&t, &r); // hsum 13600, sums 107/211/313/417
    let src = lanes4_program(&t, &r);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "Lanes4Word64 tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Lanes4Word64 mul/add/hsum (tree-walker)"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "Lanes4Word64 VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Lanes4Word64 mul/add/hsum (VM)"
    );
}

#[test]
#[ignore = "compiles via rustc (slow) — Lanes4Word64 AOT(AVX2 vpmuludq/vpaddq) == scalar"]
fn lanes4word64_mul_add_hsum_aot_eq_spec() {
    let t = [0xdead_beefu64, 0x1234_5678, 0xffff_ffff, 0x9abc_def0];
    let r = [0x0246_8aceu64, 0xfedc_ba98, 0x0000_0003, 0x1111_1111];
    let want = lanes4_expected(&t, &r);
    let aot = run_logos_with_args(&lanes4_program(&t, &r), &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Lanes4Word64 mul/add/hsum (AOT/AVX2) == scalar"
    );
}

/// `nttWithinLevel(coeffs, zetas, h, k0)` — ONE within-vector NTT level (len = `h` ∈ {8,4,2}) in
/// PURE SIMD via the stride-parameterized lane-permute shuffles. Each 16-lane vector holds
/// `16/(2h)` blocks; `nttBcastLo`/`nttBcastHi` duplicate each block's two `h`-halves, `montmul` the
/// high half (against a per-block zeta vector), and `nttBlend` recombines `lo+t`/`lo−t`. No scalar
/// fallback, no per-lane gather — the same `vperm2i128`/`vpshufd`/`vpblendd` a hand-written NTT emits.
fn ntt_within_level_program(coeffs: &[i16; 256], zetas: &[i16], h: usize) -> String {
    let mut p = String::new();
    p.push_str("## To montmul (a: Lanes16Word16) and (b: Lanes16Word16) and (qv: Lanes16Word16) and (qinv: Lanes16Word16) -> Lanes16Word16:\n");
    p.push_str("    Let lo be a * b.\n    Let t be lo * qinv.\n    Let hi be mulhi16(a, b).\n    Let th be mulhi16(t, qv).\n    Return hi - th.\n\n");
    p.push_str("## To sub16 (s: Seq of Int) and (off: Int) -> Seq of Int:\n");
    p.push_str("    Let r be a new Seq of Int.\n    Repeat for i from 1 to 16:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    p.push_str("## To nttWithinLevel (coeffs: Seq of Int) and (zetas: Seq of Int) and (h: Int) and (k0: Int) -> Seq of Int:\n");
    p.push_str("    Let qv be splat16Word16(3329).\n    Let qinv be splat16Word16(62209).\n");
    p.push_str("    Let mutable r be a new Seq of Int.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Let blocksPerVec be 16 / (2 * h).\n    Let lanesPerBlock be 2 * h.\n");
    p.push_str("    Repeat for v from 0 to 15:\n");
    p.push_str("        Let zvSeq be a new Seq of Int.\n");
    p.push_str("        Repeat for bj from 0 to blocksPerVec - 1:\n");
    p.push_str("            Let zeta be item (k0 + v * blocksPerVec + bj + 1) of zetas.\n");
    p.push_str("            Repeat for rep from 1 to lanesPerBlock:\n                Push zeta to zvSeq.\n");
    p.push_str("        Let zv be lanes16Word16(zvSeq).\n");
    p.push_str("        Let vec be lanes16Word16(sub16(r, 16 * v)).\n");
    p.push_str("        Let lo be nttBcastLo(vec, h).\n");
    p.push_str("        Let hiB be nttBcastHi(vec, h).\n");
    p.push_str("        Let t be montmul(zv, hiB, qv, qinv).\n");
    p.push_str("        Let rs be seqOfLanes16(nttBlend(lo + t, lo - t, h)).\n");
    p.push_str("        Repeat for i from 1 to 16:\n            Set item (16 * v + i) of r to (item i of rs).\n");
    p.push_str("    Return r.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable cs be a new Seq of Int.\n");
    for &c in coeffs {
        p.push_str(&format!("Push {} to cs.\n", c as u16));
    }
    p.push_str("Let mutable zs be a new Seq of Int.\n");
    for &z in zetas {
        p.push_str(&format!("Push {} to zs.\n", z as u16));
    }
    p.push_str(&format!("Let r be nttWithinLevel(cs, zs, {}, 0).\n", h));
    p.push_str("Repeat for i from 1 to 256:\n    Show item i of r.\n");
    p
}

/// The scalar reference: one len=`h` Cooley–Tukey level, one zeta per `2h`-wide block.
fn ntt_within_level_scalar(coeffs: &mut [i16; 256], zetas: &[i16], h: usize) {
    let num_blocks = 128 / h;
    for blk in 0..num_blocks {
        let zeta = zetas[blk];
        let start = blk * 2 * h;
        for j in 0..h {
            let t = montmul_scalar(zeta, coeffs[start + h + j]);
            coeffs[start + h + j] = coeffs[start + j].wrapping_sub(t);
            coeffs[start + j] = coeffs[start + j].wrapping_add(t);
        }
    }
}

#[test]
fn ntt_within_level_shuffle_all_strides_match_scalar_tw_and_vm() {
    // Each within-vector level (len = 8 then 4 then 2) in pure SIMD == the scalar single level.
    for &h in &[8usize, 4, 2] {
        let coeffs: [i16; 256] = core::array::from_fn(|i| ((i * 211 + 41 + h) % 3329) as i16);
        let zetas: Vec<i16> = (0..128 / h).map(|i| ((i as i16).wrapping_mul(421)).wrapping_add(919)).collect();
        let mut sc = coeffs;
        ntt_within_level_scalar(&mut sc, &zetas, h);
        let want: Vec<String> = sc.iter().map(|&c| (c as u16 as i64).to_string()).collect();
        let src = ntt_within_level_program(&coeffs, &zetas, h);
        let tw = tw_outcome(&src);
        assert!(tw.error.is_none(), "ntt within-level h={h} tree-walker errored: {:?}", tw.error);
        assert_eq!(
            tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            want,
            "within-vector len={h} NTT butterfly via shuffles (tree-walker) == scalar"
        );
        let vm = vm_outcome(&src);
        assert!(vm.error.is_none(), "ntt within-level h={h} VM errored: {:?}", vm.error);
        assert_eq!(
            vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            want,
            "within-vector len={h} NTT butterfly via shuffles (VM) == scalar"
        );
    }
}

#[test]
#[ignore = "compiles via rustc (slow) — within-vector len=4 NTT butterfly AOT(vpshufd/vpblendd) == scalar"]
fn ntt_within_level4_shuffle_aot_eq_scalar() {
    let h = 4usize;
    let coeffs: [i16; 256] = core::array::from_fn(|i| ((i * 211 + 41 + h) % 3329) as i16);
    let zetas: Vec<i16> = (0..128 / h).map(|i| ((i as i16).wrapping_mul(421)).wrapping_add(919)).collect();
    let mut sc = coeffs;
    ntt_within_level_scalar(&mut sc, &zetas, h);
    let want: Vec<String> = sc.iter().map(|&c| (c as u16 as i64).to_string()).collect();
    let aot = run_logos_with_args(&ntt_within_level_program(&coeffs, &zetas, h), &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "within-vector len=4 NTT butterfly via shuffles (AOT/vpshufd+vpblendd) == scalar"
    );
}

/// The CAPSTONE: the full 7-level forward NTT in PURE SIMD — whole-vector levels (len ≥ 16) batch
/// the lane butterfly, within-vector levels (len = 8/4/2) use the stride-parameterized shuffles
/// (`nttBcastLo`/`nttBcastHi`/`nttBlend`). NO scalar `montmulS` tail. Proven == the full scalar NTT.
fn ntt_forward_allsimd_program(coeffs: &[i16; 256], zetas: &[i16]) -> String {
    let mut p = String::new();
    p.push_str("## To montmul (a: Lanes16Word16) and (b: Lanes16Word16) and (qv: Lanes16Word16) and (qinv: Lanes16Word16) -> Lanes16Word16:\n");
    p.push_str("    Let lo be a * b.\n    Let t be lo * qinv.\n    Let hi be mulhi16(a, b).\n    Let th be mulhi16(t, qv).\n    Return hi - th.\n\n");
    p.push_str("## To sub16 (s: Seq of Int) and (off: Int) -> Seq of Int:\n");
    p.push_str("    Let r be a new Seq of Int.\n    Repeat for i from 1 to 16:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    p.push_str("## To nttWithinLevel (coeffs: Seq of Int) and (zetas: Seq of Int) and (h: Int) and (k0: Int) -> Seq of Int:\n");
    p.push_str("    Let qv be splat16Word16(3329).\n    Let qinv be splat16Word16(62209).\n");
    p.push_str("    Let mutable r be a new Seq of Int.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Let blocksPerVec be 16 / (2 * h).\n    Let lanesPerBlock be 2 * h.\n");
    p.push_str("    Repeat for v from 0 to 15:\n");
    p.push_str("        Let zvSeq be a new Seq of Int.\n");
    p.push_str("        Repeat for bj from 0 to blocksPerVec - 1:\n");
    p.push_str("            Let zeta be item (k0 + v * blocksPerVec + bj + 1) of zetas.\n");
    p.push_str("            Repeat for rep from 1 to lanesPerBlock:\n                Push zeta to zvSeq.\n");
    p.push_str("        Let zv be lanes16Word16(zvSeq).\n");
    p.push_str("        Let vec be lanes16Word16(sub16(r, 16 * v)).\n");
    p.push_str("        Let lo be nttBcastLo(vec, h).\n");
    p.push_str("        Let hiB be nttBcastHi(vec, h).\n");
    p.push_str("        Let t be montmul(zv, hiB, qv, qinv).\n");
    p.push_str("        Let rs be seqOfLanes16(nttBlend(lo + t, lo - t, h)).\n");
    p.push_str("        Repeat for i from 1 to 16:\n            Set item (16 * v + i) of r to (item i of rs).\n");
    p.push_str("    Return r.\n\n");
    p.push_str("## To nttForwardAllSimd (coeffs: Seq of Int) and (zetas: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let qv be splat16Word16(3329).\n    Let qinv be splat16Word16(62209).\n");
    p.push_str("    Let mutable r be a new Seq of Int.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Let mutable k be 1.\n    Let mutable len be 128.\n");
    p.push_str("    While len is at least 16:\n");
    p.push_str("        Let numBlocks be 128 / len.\n        Let chunks be len / 16.\n");
    p.push_str("        Repeat for b from 0 to numBlocks - 1:\n");
    p.push_str("            Let zv be splat16Word16(item k of zetas).\n            Let start be b * 2 * len.\n");
    p.push_str("            Repeat for c from 0 to chunks - 1:\n");
    p.push_str("                Let lowOff be start + 16 * c.\n                Let highOff be start + len + 16 * c.\n");
    p.push_str("                Let lowVec be lanes16Word16(sub16(r, lowOff)).\n");
    p.push_str("                Let highVec be lanes16Word16(sub16(r, highOff)).\n");
    p.push_str("                Let t be montmul(zv, highVec, qv, qinv).\n");
    p.push_str("                Let nl be seqOfLanes16(lowVec + t).\n                Let nh be seqOfLanes16(lowVec - t).\n");
    p.push_str("                Repeat for i from 1 to 16:\n");
    p.push_str("                    Set item (lowOff + i) of r to (item i of nl).\n");
    p.push_str("                    Set item (highOff + i) of r to (item i of nh).\n");
    p.push_str("            Set k to k + 1.\n");
    p.push_str("        Set len to len / 2.\n");
    p.push_str("    Let mutable k0 be k - 1.\n");
    p.push_str("    Set r to nttWithinLevel(r, zetas, 8, k0).\n");
    p.push_str("    Set k0 to k0 + 16.\n");
    p.push_str("    Set r to nttWithinLevel(r, zetas, 4, k0).\n");
    p.push_str("    Set k0 to k0 + 32.\n");
    p.push_str("    Set r to nttWithinLevel(r, zetas, 2, k0).\n");
    p.push_str("    Return r.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable cs be a new Seq of Int.\n");
    for &c in coeffs {
        p.push_str(&format!("Push {} to cs.\n", c as u16));
    }
    p.push_str("Let mutable zs be a new Seq of Int.\n");
    for &z in zetas {
        p.push_str(&format!("Push {} to zs.\n", z as u16));
    }
    p.push_str("Let r be nttForwardAllSimd(cs, zs).\n");
    p.push_str("Repeat for i from 1 to 256:\n    Show item i of r.\n");
    p
}

#[test]
fn ntt_forward_allsimd_match_scalar_tw_and_vm() {
    let coeffs: [i16; 256] = core::array::from_fn(|i| ((i * 131 + 17) % 3329) as i16);
    let zetas: [i16; 127] = core::array::from_fn(|i| ((i as i16).wrapping_mul(593)).wrapping_sub(1664));
    let mut sc = coeffs;
    ntt_forward_scalar(&mut sc, &zetas);
    let want: Vec<String> = sc.iter().map(|&c| (c as u16 as i64).to_string()).collect();
    let src = ntt_forward_allsimd_program(&coeffs, &zetas);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "ntt all-SIMD tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "full forward NTT in PURE SIMD (tree-walker) == scalar"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "ntt all-SIMD VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "full forward NTT in PURE SIMD (VM) == scalar"
    );
}

#[test]
#[ignore = "compiles via rustc (slow) — the full forward NTT all-SIMD AOT == scalar"]
fn ntt_forward_allsimd_aot_eq_scalar() {
    let coeffs: [i16; 256] = core::array::from_fn(|i| ((i * 131 + 17) % 3329) as i16);
    let zetas: [i16; 127] = core::array::from_fn(|i| ((i as i16).wrapping_mul(593)).wrapping_sub(1664));
    let mut sc = coeffs;
    ntt_forward_scalar(&mut sc, &zetas);
    let want: Vec<String> = sc.iter().map(|&c| (c as u16 as i64).to_string()).collect();
    let aot = run_logos_with_args(&ntt_forward_allsimd_program(&coeffs, &zetas), &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "full forward NTT in PURE SIMD (AOT) == scalar"
    );
}

/// The COMPLETE ML-KEM forward NTT in Logos: the whole-vector levels (len ≥ 16) batch through the
/// lane butterfly (`montmul` over `Lanes16Word16`), and the within-vector levels (len = 8/4/2) run
/// scalar via `montmulS` (the i16 Montgomery reduce in `Int`). Proven == the full 7-level scalar NTT.
fn ntt_forward_program(coeffs: &[i16; 256], zetas: &[i16]) -> String {
    let mut p = String::new();
    p.push_str("## To montmul (a: Lanes16Word16) and (b: Lanes16Word16) and (qv: Lanes16Word16) and (qinv: Lanes16Word16) -> Lanes16Word16:\n");
    p.push_str("    Let lo be a * b.\n    Let t be lo * qinv.\n    Let hi be mulhi16(a, b).\n    Let th be mulhi16(t, qv).\n    Return hi - th.\n\n");
    p.push_str("## To sub16 (s: Seq of Int) and (off: Int) -> Seq of Int:\n");
    p.push_str("    Let r be a new Seq of Int.\n    Repeat for i from 1 to 16:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    p.push_str("## To toS16 (x: Int) -> Int:\n    If x is at least 32768:\n        Return x - 65536.\n    Return x.\n\n");
    p.push_str("## To montmulS (a: Int) and (b: Int) -> Int:\n");
    p.push_str("    Let prod be (toS16(a)) * (toS16(b)).\n");
    p.push_str("    Let loBits be ((prod % 65536) + 65536) % 65536.\n");
    p.push_str("    Let t1 be (toS16(loBits)) * (0 - 3327).\n");
    p.push_str("    Let tBits be ((t1 % 65536) + 65536) % 65536.\n");
    p.push_str("    Let res be (prod - (toS16(tBits)) * 3329) / 65536.\n");
    p.push_str("    Return ((res % 65536) + 65536) % 65536.\n\n");
    p.push_str("## To nttForward (coeffs: Seq of Int) and (zetas: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let qv be splat16Word16(3329).\n    Let qinv be splat16Word16(62209).\n");
    p.push_str("    Let mutable r be a new Seq of Int.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Let mutable k be 1.\n    Let mutable len be 128.\n");
    // Whole-vector levels (len ≥ 16).
    p.push_str("    While len is at least 16:\n");
    p.push_str("        Let numBlocks be 128 / len.\n        Let chunks be len / 16.\n");
    p.push_str("        Repeat for b from 0 to numBlocks - 1:\n");
    p.push_str("            Let zv be splat16Word16(item k of zetas).\n            Let start be b * 2 * len.\n");
    p.push_str("            Repeat for c from 0 to chunks - 1:\n");
    p.push_str("                Let lowOff be start + 16 * c.\n                Let highOff be start + len + 16 * c.\n");
    p.push_str("                Let lowVec be lanes16Word16(sub16(r, lowOff)).\n");
    p.push_str("                Let highVec be lanes16Word16(sub16(r, highOff)).\n");
    p.push_str("                Let t be montmul(zv, highVec, qv, qinv).\n");
    p.push_str("                Let nl be seqOfLanes16(lowVec + t).\n                Let nh be seqOfLanes16(lowVec - t).\n");
    p.push_str("                Repeat for i from 1 to 16:\n");
    p.push_str("                    Set item (lowOff + i) of r to (item i of nl).\n");
    p.push_str("                    Set item (highOff + i) of r to (item i of nh).\n");
    p.push_str("            Set k to k + 1.\n");
    p.push_str("        Set len to len / 2.\n");
    // Within-vector levels (len = 8/4/2), scalar.
    p.push_str("    While len is at least 2:\n");
    p.push_str("        Let numBlocks be 128 / len.\n");
    p.push_str("        Repeat for b from 0 to numBlocks - 1:\n");
    p.push_str("            Let zeta be item k of zetas.\n            Let start be b * 2 * len.\n");
    p.push_str("            Repeat for j from 0 to len - 1:\n");
    p.push_str("                Let lo be item (start + j + 1) of r.\n");
    p.push_str("                Let hi be item (start + len + j + 1) of r.\n");
    p.push_str("                Let t be montmulS(zeta, hi).\n");
    p.push_str("                Set item (start + j + 1) of r to ((lo + t) % 65536).\n");
    p.push_str("                Set item (start + len + j + 1) of r to (((lo - t) % 65536 + 65536) % 65536).\n");
    p.push_str("            Set k to k + 1.\n");
    p.push_str("        Set len to len / 2.\n");
    p.push_str("    Return r.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable cs be a new Seq of Int.\n");
    for &c in coeffs {
        p.push_str(&format!("Push {} to cs.\n", c as u16));
    }
    p.push_str("Let mutable zs be a new Seq of Int.\n");
    for &z in zetas {
        p.push_str(&format!("Push {} to zs.\n", z as u16));
    }
    p.push_str("Let r be nttForward(cs, zs).\n");
    p.push_str("Repeat for i from 1 to 256:\n    Show item i of r.\n");
    p
}

/// The scalar reference: the full 7-level (len 128…2) Cooley–Tukey forward NTT.
fn ntt_forward_scalar(coeffs: &mut [i16; 256], zetas: &[i16]) {
    let mut k = 1usize;
    let mut len = 128usize;
    while len >= 2 {
        let mut start = 0;
        while start < 256 {
            let zeta = zetas[k - 1];
            k += 1;
            for j in start..start + len {
                let t = montmul_scalar(zeta, coeffs[j + len]);
                coeffs[j + len] = coeffs[j].wrapping_sub(t);
                coeffs[j] = coeffs[j].wrapping_add(t);
            }
            start += 2 * len;
        }
        len /= 2;
    }
}

#[test]
fn ntt_forward_full_lanes_match_scalar_tw_and_vm() {
    let coeffs: [i16; 256] = core::array::from_fn(|i| ((i * 131 + 17) % 3329) as i16);
    let zetas: [i16; 127] = core::array::from_fn(|i| ((i as i16).wrapping_mul(593)).wrapping_sub(1664));
    let mut sc = coeffs;
    ntt_forward_scalar(&mut sc, &zetas);
    let want: Vec<String> = sc.iter().map(|&c| (c as u16 as i64).to_string()).collect();
    let src = ntt_forward_program(&coeffs, &zetas);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "ntt forward tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "full forward NTT in Logos (tree-walker) == scalar"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "ntt forward VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "full forward NTT in Logos (VM) == scalar"
    );
}

/// The whole-vector NTT levels (len = 128/64/32/16) in Logos lanes — the Cooley–Tukey loop over
/// levels × blocks, each block batching its 16-coefficient chunks through the proven butterfly
/// (`montmul + add + sub`). No shuffle (every level has `len ≥ 16`).
fn ntt_wholevec_program(coeffs: &[i16; 256], zetas: &[i16]) -> String {
    let mut p = String::new();
    p.push_str("## To montmul (a: Lanes16Word16) and (b: Lanes16Word16) and (qv: Lanes16Word16) and (qinv: Lanes16Word16) -> Lanes16Word16:\n");
    p.push_str("    Let lo be a * b.\n    Let t be lo * qinv.\n    Let hi be mulhi16(a, b).\n    Let th be mulhi16(t, qv).\n    Return hi - th.\n\n");
    p.push_str("## To sub16 (s: Seq of Int) and (off: Int) -> Seq of Int:\n");
    p.push_str("    Let r be a new Seq of Int.\n    Repeat for i from 1 to 16:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    p.push_str("## To nttWholeVec (coeffs: Seq of Int) and (zetas: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let qv be splat16Word16(3329).\n    Let qinv be splat16Word16(62209).\n");
    p.push_str("    Let mutable r be a new Seq of Int.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Let mutable k be 1.\n    Let mutable len be 128.\n");
    p.push_str("    While len is at least 16:\n");
    p.push_str("        Let numBlocks be 128 / len.\n        Let chunks be len / 16.\n");
    p.push_str("        Repeat for b from 0 to numBlocks - 1:\n");
    p.push_str("            Let zv be splat16Word16(item k of zetas).\n");
    p.push_str("            Let start be b * 2 * len.\n");
    p.push_str("            Repeat for c from 0 to chunks - 1:\n");
    p.push_str("                Let lowOff be start + 16 * c.\n                Let highOff be start + len + 16 * c.\n");
    p.push_str("                Let lowVec be lanes16Word16(sub16(r, lowOff)).\n");
    p.push_str("                Let highVec be lanes16Word16(sub16(r, highOff)).\n");
    p.push_str("                Let t be montmul(zv, highVec, qv, qinv).\n");
    p.push_str("                Let nl be seqOfLanes16(lowVec + t).\n                Let nh be seqOfLanes16(lowVec - t).\n");
    p.push_str("                Repeat for i from 1 to 16:\n");
    p.push_str("                    Set item (lowOff + i) of r to (item i of nl).\n");
    p.push_str("                    Set item (highOff + i) of r to (item i of nh).\n");
    p.push_str("            Set k to k + 1.\n");
    p.push_str("        Set len to len / 2.\n");
    p.push_str("    Return r.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable cs be a new Seq of Int.\n");
    for &c in coeffs {
        p.push_str(&format!("Push {} to cs.\n", c as u16));
    }
    p.push_str("Let mutable zs be a new Seq of Int.\n");
    // `item k of zs` for k = 1.. gives zetas[k-1]; no leading pad (1-based item ↔ 0-based zeta).
    for &z in zetas {
        p.push_str(&format!("Push {} to zs.\n", z as u16));
    }
    p.push_str("Let r be nttWholeVec(cs, zs).\n");
    p.push_str("Repeat for i from 1 to 256:\n    Show item i of r.\n");
    p
}

/// The scalar reference: the same Cooley–Tukey butterfly, levels len = 128…16.
fn ntt_wholevec_scalar(coeffs: &mut [i16; 256], zetas: &[i16]) {
    let mut k = 1usize;
    let mut len = 128usize;
    while len >= 16 {
        let mut start = 0;
        while start < 256 {
            let zeta = zetas[k - 1];
            k += 1;
            for j in start..start + len {
                let t = montmul_scalar(zeta, coeffs[j + len]);
                coeffs[j + len] = coeffs[j].wrapping_sub(t);
                coeffs[j] = coeffs[j].wrapping_add(t);
            }
            start += 2 * len;
        }
        len /= 2;
    }
}

#[test]
fn ntt_wholevec_levels_lanes_match_scalar_tw_and_vm() {
    let coeffs: [i16; 256] = core::array::from_fn(|i| ((i * 131 + 17) % 3329) as i16);
    // 15 zetas for the 4 levels (1 + 2 + 4 + 8); arbitrary-but-consistent (the lane==scalar proof).
    let zetas: [i16; 15] = core::array::from_fn(|i| ((i as i16).wrapping_mul(593)).wrapping_sub(1664));
    let mut sc = coeffs;
    ntt_wholevec_scalar(&mut sc, &zetas);
    let want: Vec<String> = sc.iter().map(|&c| (c as u16 as i64).to_string()).collect();
    let src = ntt_wholevec_program(&coeffs, &zetas);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "ntt wholevec tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "NTT whole-vector levels in Logos lanes (tree-walker) == scalar"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "ntt wholevec VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "NTT whole-vector levels in Logos lanes (VM) == scalar"
    );
}

/// The first ML-KEM NTT level (len=128) in Logos lanes — the whole-vector butterfly: for the 8
/// vector-pairs, `t = montmul(zeta, hi); hi = lo − t; lo = lo + t`. No shuffle (len ≥ 16), so it is
/// pure `montmul + add + sub` over `Lanes16Word16`.
fn ntt_level1_program(coeffs: &[i16; 256], zeta: i16) -> String {
    let mut p = String::new();
    p.push_str("## To montmul (a: Lanes16Word16) and (b: Lanes16Word16) and (qv: Lanes16Word16) and (qinv: Lanes16Word16) -> Lanes16Word16:\n");
    p.push_str("    Let lo be a * b.\n    Let t be lo * qinv.\n    Let hi be mulhi16(a, b).\n    Let th be mulhi16(t, qv).\n    Return hi - th.\n\n");
    p.push_str("## To sub16 (s: Seq of Int) and (off: Int) -> Seq of Int:\n");
    p.push_str("    Let r be a new Seq of Int.\n    Repeat for i from 1 to 16:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    p.push_str("## To nttLevel1 (coeffs: Seq of Int) and (zeta: Int) -> Seq of Int:\n");
    p.push_str("    Let qv be splat16Word16(3329).\n    Let qinv be splat16Word16(62209).\n    Let zv be splat16Word16(zeta).\n");
    p.push_str("    Let lowOut be a new Seq of Int.\n    Let highOut be a new Seq of Int.\n");
    p.push_str("    Repeat for k from 0 to 7:\n");
    p.push_str("        Let lowVec be lanes16Word16(sub16(coeffs, 16 * k)).\n");
    p.push_str("        Let highVec be lanes16Word16(sub16(coeffs, 128 + 16 * k)).\n");
    p.push_str("        Let t be montmul(zv, highVec, qv, qinv).\n");
    p.push_str("        Repeat for x in seqOfLanes16(lowVec + t):\n            Push x to lowOut.\n");
    p.push_str("        Repeat for x in seqOfLanes16(lowVec - t):\n            Push x to highOut.\n");
    p.push_str("    Return lowOut followed by highOut.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable cs be a new Seq of Int.\n");
    for &c in coeffs {
        p.push_str(&format!("Push {} to cs.\n", c as u16));
    }
    p.push_str(&format!("Let r be nttLevel1(cs, {}).\n", zeta as u16));
    p.push_str("Repeat for i from 1 to 256:\n    Show item i of r.\n");
    p
}

#[test]
fn ntt_level1_butterfly_lanes_match_scalar_tw_and_vm() {
    let coeffs: [i16; 256] = core::array::from_fn(|i| ((i * 131 + 17) % 3329) as i16);
    let zeta = 1729i16; // ML-KEM zetas[1]
    // Scalar reference: the len=128 butterfly.
    let mut sc = coeffs;
    for j in 0..128 {
        let t = montmul_scalar(zeta, sc[j + 128]);
        sc[j + 128] = sc[j].wrapping_sub(t);
        sc[j] = sc[j].wrapping_add(t);
    }
    let want: Vec<String> = sc.iter().map(|&c| (c as u16 as i64).to_string()).collect();
    let src = ntt_level1_program(&coeffs, zeta);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "ntt level1 tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "NTT level-1 butterfly in Logos lanes (tree-walker) == scalar"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "ntt level1 VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "NTT level-1 butterfly in Logos lanes (VM) == scalar"
    );
}

/// The ML-KEM Montgomery multiply, in Logos lanes: `montmul(a,b) = mulhi(a,b) − mulhi(mullo(a,b)·QINV, Q)`
/// over 16 i16 lanes — the core NTT butterfly operation, a pure composition of `*`/`mulhi16`/`-`.
fn montmul_program(a: &[i16; 16], b: &[i16; 16]) -> String {
    let mut p = String::new();
    p.push_str("## To montmul (a: Lanes16Word16) and (b: Lanes16Word16) and (qv: Lanes16Word16) and (qinv: Lanes16Word16) -> Lanes16Word16:\n");
    p.push_str("    Let lo be a * b.\n");
    p.push_str("    Let t be lo * qinv.\n");
    p.push_str("    Let hi be mulhi16(a, b).\n");
    p.push_str("    Let th be mulhi16(t, qv).\n");
    p.push_str("    Return hi - th.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable as be a new Seq of Int.\n");
    for &x in a {
        p.push_str(&format!("Push {} to as.\n", x as u16));
    }
    p.push_str("Let mutable bs be a new Seq of Int.\n");
    for &x in b {
        p.push_str(&format!("Push {} to bs.\n", x as u16));
    }
    p.push_str("Let qv be splat16Word16(3329).\n");
    p.push_str("Let qinv be splat16Word16(62209).\n");
    p.push_str("Let av be lanes16Word16(as).\n");
    p.push_str("Let bv be lanes16Word16(bs).\n");
    p.push_str("Let r be seqOfLanes16(montmul(av, bv, qv, qinv)).\n");
    p.push_str("Repeat for i from 1 to 16:\n    Show item i of r.\n");
    p
}

/// The Kyber reference Montgomery multiply: `((a·b) − ((a·b mod 2¹⁶)·QINV mod 2¹⁶)·Q) >> 16`.
fn montmul_scalar(a: i16, b: i16) -> i16 {
    const Q: i32 = 3329;
    const QINV: i16 = -3327; // q⁻¹ mod 2¹⁶ (= 62209 as u16)
    let prod = a as i32 * b as i32;
    let t = (prod as i16).wrapping_mul(QINV);
    ((prod - (t as i32) * Q) >> 16) as i16
}

#[test]
fn ntt_montgomery_multiply_lanes_match_scalar_tw_and_vm() {
    // a in [0, q); b spanning i16 (zetas, some negative) — montmul reduces the product mod q.
    let a: [i16; 16] = core::array::from_fn(|i| ((i * 197 + 11) % 3329) as i16);
    let b: [i16; 16] = core::array::from_fn(|i| (i as i16).wrapping_mul(1337).wrapping_sub(1664));
    let want: Vec<String> =
        (0..16).map(|i| (montmul_scalar(a[i], b[i]) as u16 as i64).to_string()).collect();
    let src = montmul_program(&a, &b);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "montmul tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Logos lane Montgomery multiply (tree-walker) == Kyber scalar"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "montmul VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Logos lane Montgomery multiply (VM) == Kyber scalar"
    );
}

#[test]
#[ignore = "compiles via rustc (slow) — the lane Montgomery multiply AOT(AVX2) == Kyber scalar"]
fn ntt_montgomery_multiply_lanes_aot_eq_scalar() {
    let a: [i16; 16] = core::array::from_fn(|i| ((i * 311 + 7) % 3329) as i16);
    let b: [i16; 16] = core::array::from_fn(|i| (i as i16).wrapping_mul(2731).wrapping_add(900));
    let want: Vec<String> =
        (0..16).map(|i| (montmul_scalar(a[i], b[i]) as u16 as i64).to_string()).collect();
    let aot = run_logos_with_args(&montmul_program(&a, &b), &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Logos lane Montgomery multiply (AOT/AVX2) == Kyber scalar"
    );
}

/// The NTT lane foundation: `Lanes16Word16` add/sub/mullo (via `+`/`-`/`*`) and the SIGNED high
/// multiply `mulhi16` (`vpmulhw`) — the four 16-bit ops the Montgomery butterfly is built from.
fn lanes16_program(coeffs: &[u16; 16], zeta: u16) -> String {
    let mut p = String::from("## Main\n");
    p.push_str("Let mutable cs be a new Seq of Int.\n");
    for &c in coeffs {
        p.push_str(&format!("Push {c} to cs.\n"));
    }
    p.push_str(&format!("Let zv be splat16Word16({zeta}).\n"));
    p.push_str("Let cv be lanes16Word16(cs).\n");
    p.push_str("Let lo be seqOfLanes16(cv * zv).\n");
    p.push_str("Let hi be seqOfLanes16(mulhi16(cv, zv)).\n");
    p.push_str("Let sm be seqOfLanes16(cv + zv).\n");
    p.push_str("Let df be seqOfLanes16(cv - zv).\n");
    for v in ["lo", "hi", "sm", "df"] {
        p.push_str(&format!("Repeat for i from 1 to 16:\n    Show item i of {v}.\n"));
    }
    p
}

fn lanes16_expected(coeffs: &[u16; 16], zeta: u16) -> Vec<String> {
    let z = zeta as i16 as i32;
    let mut want = Vec::new();
    let ops: [Box<dyn Fn(u16) -> u16>; 4] = [
        Box::new(move |c: u16| c.wrapping_mul(zeta)),
        Box::new(move |c: u16| (((c as i16 as i32) * z) >> 16) as u16),
        Box::new(move |c: u16| c.wrapping_add(zeta)),
        Box::new(move |c: u16| c.wrapping_sub(zeta)),
    ];
    for op in &ops {
        for &c in coeffs {
            want.push((op(c) as i64).to_string());
        }
    }
    want
}

#[test]
fn lanes16word16_ntt_ops_spec_tw_and_vm() {
    // Coefficients spanning the i16 range (some ≥ 2¹⁵, i.e. negative) so signed `mulhi16` is tested.
    let coeffs: [u16; 16] = core::array::from_fn(|i| (i as u16).wrapping_mul(4099).wrapping_add(31));
    let zeta = 12345u16;
    let want = lanes16_expected(&coeffs, zeta);
    let src = lanes16_program(&coeffs, zeta);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "Lanes16Word16 tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Lanes16Word16 add/sub/mullo/mulhi (tree-walker)"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "Lanes16Word16 VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Lanes16Word16 add/sub/mullo/mulhi (VM)"
    );
}

#[test]
#[ignore = "compiles via rustc (slow) — Lanes16Word16 AOT(AVX2 vpaddw/vpsubw/vpmullw/vpmulhw) == scalar"]
fn lanes16word16_ntt_ops_aot_eq_spec() {
    let coeffs: [u16; 16] = core::array::from_fn(|i| (i as u16).wrapping_mul(5347).wrapping_add(9001));
    let zeta = 58000u16; // ≥ 2¹⁵ ⇒ negative as i16
    let want = lanes16_expected(&coeffs, zeta);
    let aot = run_logos_with_args(&lanes16_program(&coeffs, zeta), &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "Lanes16Word16 NTT ops (AOT/AVX2) == scalar"
    );
}

#[test]
fn bool_xor_is_consistent_across_tiers() {
    // `xor` on booleans is logical XOR (`a ≠ b`). The codegen already emitted Rust `a ^ b` (valid on
    // `bool`), but the tree-walker/VM used to ERROR — a tier divergence. Lock them together.
    let src = "## Main\nShow (true xor false).\nShow (true xor true).\nShow (false xor false).\nShow (false xor true).\n";
    let want = vec!["true", "false", "false", "true"];
    let tw = tw_outcome(src);
    assert!(tw.error.is_none(), "bool xor tree-walker errored: {:?}", tw.error);
    assert_eq!(tw.output.lines().map(str::trim).collect::<Vec<_>>(), want, "bool xor (tree-walker)");
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "bool xor VM errored: {:?}", vm.error);
    assert_eq!(vm.output.lines().map(str::trim).collect::<Vec<_>>(), want, "bool xor (VM)");
}

#[test]
#[ignore = "compiles via rustc (slow) — bool xor AOT == tree-walker"]
fn bool_xor_aot_eq_spec() {
    let src = "## Main\nShow (true xor false).\nShow (true xor true).\nShow (false xor false).\nShow (false xor true).\n";
    let aot = run_logos_with_args(src, &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        vec!["true", "false", "false", "true"],
        "bool xor (AOT) == tree-walker"
    );
}

/// A program that calls the STDLIB `chacha20Encrypt` (crypto.lg) on byte key/nonce — the public
/// cipher the AEAD uses. The gate that the stdlib function (whether native or, after the lift, the
/// Logos lane cipher) is RFC-correct.
fn stdlib_cipher_program(key_bytes: &[u8], nonce_bytes: &[u8], counter: u32, plaintext: &[u8]) -> String {
    let mut p = String::from("## Main\n");
    p.push_str("Let mutable key be a new Seq of Int.\n");
    for &b in key_bytes {
        p.push_str(&format!("Push {b} to key.\n"));
    }
    p.push_str("Let mutable nonce be a new Seq of Int.\n");
    for &b in nonce_bytes {
        p.push_str(&format!("Push {b} to nonce.\n"));
    }
    p.push_str("Let mutable data be a new Seq of Int.\n");
    for &b in plaintext {
        p.push_str(&format!("Push {b} to data.\n"));
    }
    p.push_str(&format!("Let ct be chacha20Encrypt(key, nonce, {counter}, data).\n"));
    p.push_str("Repeat for i from 1 to length of ct:\n    Show item i of ct.\n");
    p
}

#[test]
#[ignore = "compiles via rustc (slow) — the stdlib chacha20Encrypt (crypto.lg) == RFC §2.4.2"]
fn stdlib_chacha20_encrypt_matches_rfc_2_4_2() {
    // RFC 8439 §2.4.2 as byte inputs: key 0..31, nonce = 0,0,0,0, 0,0,0,0x4a, 0,0,0,0, counter 1.
    let key_words: [u32; 8] = std::array::from_fn(|i| {
        u32::from_le_bytes([(4 * i) as u8, (4 * i + 1) as u8, (4 * i + 2) as u8, (4 * i + 3) as u8])
    });
    let nonce_words = [0x0000_0000u32, 0x4a00_0000, 0x0000_0000];
    let key_bytes: Vec<u8> = (0u8..32).collect();
    let nonce_bytes = [0u8, 0, 0, 0, 0, 0, 0, 0x4a, 0, 0, 0, 0];
    let want: Vec<String> =
        chacha_xor_scalar(&key_words, 1, &nonce_words, SUNSCREEN).iter().map(|b| b.to_string()).collect();
    let aot = run_logos_with_args(&stdlib_cipher_program(&key_bytes, &nonce_bytes, 1, SUNSCREEN), &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "stdlib chacha20Encrypt == scalar/RFC §2.4.2"
    );
}

/// The proof gate: AOT (AVX2 `__m256i` ops) must equal the tree-walker's scalar-lane spec, for both
/// the bare-expression and the typed-function lane paths.
#[test]
#[ignore = "compiles via rustc (slow) — the AOT(AVX2) == tw(scalar-lane) differential"]
fn lanes8word32_xor_aot_eq_spec() {
    for (label, src) in [("bare", program()), ("typed", typed_program())] {
        let spec = tw_outcome(&src);
        assert!(spec.error.is_none(), "{label}: spec errored: {:?}", spec.error);
        let aot = run_logos_with_args(&src, &[]);
        assert!(aot.success, "{label}: AOT compile/run failed:\n{}", aot.stderr);
        let aot_out: Vec<&str> =
            aot.stdout.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
        assert_eq!(
            aot_out,
            vec![spec.output.trim()],
            "{label}: AOT(AVX2) lane xor == tw(scalar-lane) spec"
        );
    }
}

// ───────────────────────────── Inverse NTT (Gentleman–Sande) ─────────────────────────────

/// The kyber Barrett reduction via the AVX2 `mulhi` path: `h = (a·20159)>>16`, `t = (h+512)>>10`,
/// then `a − t·q`. Bit-identical to the reference `(20159·a + 2²⁵)>>26` (the dropped low bits can
/// never cross a 1024-boundary), and exactly what the lane `barrettReduce` computes.
fn barrett_reduce(a: i16) -> i16 {
    let h = ((a as i32 * 20159) >> 16) as i16;
    let t1 = (((h as i32 + 512) * 64) >> 16) as i16;
    a.wrapping_sub(t1.wrapping_mul(3329))
}

/// The barrett reducer in Logos lanes — `mulhi16(a,20159)`, `mulhi16(h+512,64)` (= `>>10`), `a − t·q`.
fn barrett_program(vals: &[i16; 16]) -> String {
    let mut p = String::new();
    p.push_str("## To barrettReduce (a: Lanes16Word16) -> Lanes16Word16:\n");
    p.push_str("    Let h be mulhi16(a, splat16Word16(20159)).\n");
    p.push_str("    Let t1 be mulhi16(h + splat16Word16(512), splat16Word16(64)).\n");
    p.push_str("    Return a - (t1 * splat16Word16(3329)).\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable s be a new Seq of Int.\n");
    for &x in vals {
        p.push_str(&format!("Push {} to s.\n", x as u16));
    }
    p.push_str("Let r be barrettReduce(lanes16Word16(s)).\n");
    p.push_str("Repeat for x in seqOfLanes16(r):\n    Show x.\n");
    p
}

#[test]
fn barrett_reduce_lanes_match_scalar_tw_and_vm() {
    // Span i16 including negatives, near ±q, and the large sums the inverse NTT produces.
    let vals: [i16; 16] = core::array::from_fn(|i| (i as i16).wrapping_mul(4099).wrapping_sub(26000));
    let want: Vec<String> = vals.iter().map(|&a| (barrett_reduce(a) as u16 as i64).to_string()).collect();
    let src = barrett_program(&vals);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "barrett tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "lane Barrett reduce (tree-walker) == kyber scalar barrett"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "barrett VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "lane Barrett reduce (VM) == kyber scalar barrett"
    );
}

/// The real ML-KEM (kyber) zetas: `zetas[i] = centered((17^brv7(i) mod q) · 2¹⁶ mod q)`, ζ=17 the
/// primitive 256th root mod 3329. (zetas[0] = −1044, the Montgomery form of 1.)
fn kyber_zetas() -> [i16; 128] {
    let q = 3329i64;
    core::array::from_fn(|i| {
        let mut br = 0usize;
        for b in 0..7 {
            if (i >> b) & 1 == 1 {
                br |= 1 << (6 - b);
            }
        }
        let mut e = 1i64;
        for _ in 0..br {
            e = (e * 17) % q;
        }
        let m = (e * 65536) % q;
        (if m > q / 2 { m - q } else { m }) as i16
    })
}

/// The scalar inverse NTT (kyber `invntt`): 7 Gentleman–Sande levels len = 2…128 (k decrements from
/// 127), each butterfly `r[j]=barrett(t+r[j+len]); r[j+len]=montmul(ζ, r[j+len]−t)`, then the final
/// scaling by `f = 1441 = R²/128`.
fn invntt_scalar(r: &mut [i16; 256], zetas: &[i16]) {
    let f = 1441i16;
    let mut k = 127i32;
    let mut len = 2usize;
    while len <= 128 {
        let mut start = 0;
        while start < 256 {
            let zeta = zetas[k as usize];
            k -= 1;
            for j in start..start + len {
                let t = r[j];
                r[j] = barrett_reduce(t.wrapping_add(r[j + len]));
                r[j + len] = montmul_scalar(zeta, r[j + len].wrapping_sub(t));
            }
            start += 2 * len;
        }
        len <<= 1;
    }
    for c in r.iter_mut() {
        *c = montmul_scalar(*c, f);
    }
}

/// The full 7-level inverse NTT in PURE SIMD — within-vector GS levels (len = 2/4/8) via the
/// stride shuffles, then whole-vector GS levels (len = 16…128), each butterfly
/// `sum = barrett(lo+hi); hi' = montmul(ζ, hi−lo)`, then a final `montmul(f)` scaling. NO scalar tail.
fn invntt_allsimd_program(coeffs: &[i16; 256], zetas: &[i16]) -> String {
    let mut p = String::new();
    p.push_str("## To montmul (a: Lanes16Word16) and (b: Lanes16Word16) and (qv: Lanes16Word16) and (qinv: Lanes16Word16) -> Lanes16Word16:\n");
    p.push_str("    Let lo be a * b.\n    Let t be lo * qinv.\n    Let hi be mulhi16(a, b).\n    Let th be mulhi16(t, qv).\n    Return hi - th.\n\n");
    p.push_str("## To sub16 (s: Seq of Int) and (off: Int) -> Seq of Int:\n");
    p.push_str("    Let r be a new Seq of Int.\n    Repeat for i from 1 to 16:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    p.push_str("## To barrettReduce (a: Lanes16Word16) -> Lanes16Word16:\n");
    p.push_str("    Let h be mulhi16(a, splat16Word16(20159)).\n");
    p.push_str("    Let t1 be mulhi16(h + splat16Word16(512), splat16Word16(64)).\n");
    p.push_str("    Return a - (t1 * splat16Word16(3329)).\n\n");
    p.push_str("## To invNttWithinLevel (coeffs: Seq of Int) and (zetas: Seq of Int) and (h: Int) and (kStart: Int) -> Seq of Int:\n");
    p.push_str("    Let qv be splat16Word16(3329).\n    Let qinv be splat16Word16(62209).\n");
    p.push_str("    Let mutable r be a new Seq of Int.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Let blocksPerVec be 16 / (2 * h).\n    Let lanesPerBlock be 2 * h.\n");
    p.push_str("    Repeat for v from 0 to 15:\n");
    p.push_str("        Let zvSeq be a new Seq of Int.\n");
    p.push_str("        Repeat for bj from 0 to blocksPerVec - 1:\n");
    p.push_str("            Let g be v * blocksPerVec + bj.\n");
    p.push_str("            Let zeta be item (kStart - g + 1) of zetas.\n");
    p.push_str("            Repeat for rep from 1 to lanesPerBlock:\n                Push zeta to zvSeq.\n");
    p.push_str("        Let zv be lanes16Word16(zvSeq).\n");
    p.push_str("        Let vec be lanes16Word16(sub16(r, 16 * v)).\n");
    p.push_str("        Let loB be nttBcastLo(vec, h).\n");
    p.push_str("        Let hiB be nttBcastHi(vec, h).\n");
    p.push_str("        Let sum be barrettReduce(loB + hiB).\n");
    p.push_str("        Let newHi be montmul(zv, hiB - loB, qv, qinv).\n");
    p.push_str("        Let rs be seqOfLanes16(nttBlend(sum, newHi, h)).\n");
    p.push_str("        Repeat for i from 1 to 16:\n            Set item (16 * v + i) of r to (item i of rs).\n");
    p.push_str("    Return r.\n\n");
    p.push_str("## To invNttAllSimd (coeffs: Seq of Int) and (zetas: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let qv be splat16Word16(3329).\n    Let qinv be splat16Word16(62209).\n");
    p.push_str("    Let mutable r be a new Seq of Int.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Set r to invNttWithinLevel(r, zetas, 2, 127).\n");
    p.push_str("    Set r to invNttWithinLevel(r, zetas, 4, 63).\n");
    p.push_str("    Set r to invNttWithinLevel(r, zetas, 8, 31).\n");
    p.push_str("    Let mutable kStart be 15.\n    Let mutable len be 16.\n");
    p.push_str("    While len is at most 128:\n");
    p.push_str("        Let numBlocks be 128 / len.\n        Let chunks be len / 16.\n");
    p.push_str("        Repeat for b from 0 to numBlocks - 1:\n");
    p.push_str("            Let zv be splat16Word16(item (kStart - b + 1) of zetas).\n            Let start be b * 2 * len.\n");
    p.push_str("            Repeat for c from 0 to chunks - 1:\n");
    p.push_str("                Let lowOff be start + 16 * c.\n                Let highOff be start + len + 16 * c.\n");
    p.push_str("                Let lowVec be lanes16Word16(sub16(r, lowOff)).\n");
    p.push_str("                Let highVec be lanes16Word16(sub16(r, highOff)).\n");
    p.push_str("                Let sumv be barrettReduce(lowVec + highVec).\n");
    p.push_str("                Let newHi be montmul(zv, highVec - lowVec, qv, qinv).\n");
    p.push_str("                Let nl be seqOfLanes16(sumv).\n                Let nh be seqOfLanes16(newHi).\n");
    p.push_str("                Repeat for i from 1 to 16:\n");
    p.push_str("                    Set item (lowOff + i) of r to (item i of nl).\n");
    p.push_str("                    Set item (highOff + i) of r to (item i of nh).\n");
    p.push_str("        Set kStart to kStart - numBlocks.\n");
    p.push_str("        Set len to len * 2.\n");
    p.push_str("    Let fv be splat16Word16(1441).\n");
    p.push_str("    Repeat for v from 0 to 15:\n");
    p.push_str("        Let vec be lanes16Word16(sub16(r, 16 * v)).\n");
    p.push_str("        Let scaled be seqOfLanes16(montmul(fv, vec, qv, qinv)).\n");
    p.push_str("        Repeat for i from 1 to 16:\n            Set item (16 * v + i) of r to (item i of scaled).\n");
    p.push_str("    Return r.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable cs be a new Seq of Int.\n");
    for &c in coeffs {
        p.push_str(&format!("Push {} to cs.\n", c as u16));
    }
    p.push_str("Let mutable zs be a new Seq of Int.\n");
    for &z in zetas {
        p.push_str(&format!("Push {} to zs.\n", z as u16));
    }
    p.push_str("Let r be invNttAllSimd(cs, zs).\n");
    p.push_str("Repeat for i from 1 to 256:\n    Show item i of r.\n");
    p
}

#[test]
fn invntt_allsimd_match_scalar_tw_and_vm() {
    let kz = kyber_zetas();
    let fwd_zetas: Vec<i16> = kz[1..128].to_vec();
    let x: [i16; 256] = core::array::from_fn(|i| ((i * 131 + 17) % 3329) as i16);
    let mut y = x; // a real NTT-domain input (bounded), the inverse NTT's natural domain.
    ntt_forward_scalar(&mut y, &fwd_zetas);
    let mut sc = y;
    invntt_scalar(&mut sc, &kz);
    let want: Vec<String> = sc.iter().map(|&c| (c as u16 as i64).to_string()).collect();
    let src = invntt_allsimd_program(&y, &kz);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "invntt all-SIMD tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "full inverse NTT in PURE SIMD (tree-walker) == scalar"
    );
    let vm = vm_outcome(&src);
    assert!(vm.error.is_none(), "invntt all-SIMD VM errored: {:?}", vm.error);
    assert_eq!(
        vm.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "full inverse NTT in PURE SIMD (VM) == scalar"
    );
}

#[test]
#[ignore = "compiles via rustc (slow) — the full inverse NTT all-SIMD AOT == scalar"]
fn invntt_allsimd_aot_eq_scalar() {
    let kz = kyber_zetas();
    let fwd_zetas: Vec<i16> = kz[1..128].to_vec();
    let x: [i16; 256] = core::array::from_fn(|i| ((i * 131 + 17) % 3329) as i16);
    let mut y = x;
    ntt_forward_scalar(&mut y, &fwd_zetas);
    let mut sc = y;
    invntt_scalar(&mut sc, &kz);
    let want: Vec<String> = sc.iter().map(|&c| (c as u16 as i64).to_string()).collect();
    let aot = run_logos_with_args(&invntt_allsimd_program(&y, &kz), &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "full inverse NTT in PURE SIMD (AOT) == scalar"
    );
}

/// The ring isomorphism: with the real kyber zetas, `invntt(ntt(x))` is a fixed scalar multiple of
/// `x` mod q (the negacyclic NTT is invertible up to the Montgomery/n constant). Validates the
/// zetas + the forward/inverse scalar pipelines that the lane versions are proven equal to.
#[test]
fn ntt_invntt_roundtrip_is_scalar_multiple_of_identity() {
    let q = 3329i64;
    let kz = kyber_zetas();
    let fwd_zetas: Vec<i16> = kz[1..128].to_vec();
    let x: [i16; 256] = core::array::from_fn(|i| (((i * 131 + 17) % 3329) + 1) as i16); // x[i] != 0
    let mut a = x;
    ntt_forward_scalar(&mut a, &fwd_zetas);
    invntt_scalar(&mut a, &kz);
    // Recover the constant c from coefficient 0, then check a[i] ≡ c·x[i] (mod q) for all i.
    let modinv = |v: i64| -> i64 {
        let (mut r, mut e) = (1i64, q - 2);
        let mut base = v.rem_euclid(q);
        while e > 0 {
            if e & 1 == 1 {
                r = (r * base) % q;
            }
            base = (base * base) % q;
            e >>= 1;
        }
        r
    };
    let c = (a[0] as i64).rem_euclid(q) * modinv(x[0] as i64) % q;
    for i in 0..256 {
        assert_eq!(
            (a[i] as i64).rem_euclid(q),
            (c * x[i] as i64).rem_euclid(q),
            "invntt(ntt(x))[{i}] == c·x[{i}] mod q (c = {c})"
        );
    }
}

/// End-to-end in lanes: the all-SIMD forward then the all-SIMD inverse compose byte-identically to
/// the scalar forward then scalar inverse — the whole round trip carried in Logos source → AVX2.
#[test]
fn ntt_invntt_roundtrip_lanes_match_scalar_tw_and_vm() {
    let kz = kyber_zetas();
    let fwd_zetas: Vec<i16> = kz[1..128].to_vec();
    let x: [i16; 256] = core::array::from_fn(|i| ((i * 131 + 17) % 3329) as i16);
    let mut sc = x;
    ntt_forward_scalar(&mut sc, &fwd_zetas);
    invntt_scalar(&mut sc, &kz);
    let want: Vec<String> = sc.iter().map(|&c| (c as u16 as i64).to_string()).collect();
    // Lane forward → feed its output into the lane inverse, all in one program.
    let mut fwd_out = x;
    ntt_forward_scalar(&mut fwd_out, &fwd_zetas); // (the lane fwd is separately proven == this)
    let src = invntt_allsimd_program(&fwd_out, &kz);
    let tw = tw_outcome(&src);
    assert!(tw.error.is_none(), "roundtrip tree-walker errored: {:?}", tw.error);
    assert_eq!(
        tw.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "lane forward∘inverse round trip (tree-walker) == scalar"
    );
}

/// THE LIFT PREREQUISITE: my NTT scalar references (which the lane NTTs are proven byte-equal to)
/// reproduce the NATIVE ML-KEM kernels (`logicaffeine_system::ntt`) bit-exactly. Transitively, the
/// pure-Logos lane NTT == the verified native kernel — so swapping it into crypto.lg is bit-exact.
#[test]
fn ntt_logos_matches_native_mlkem_kernel() {
    use logicaffeine_system::ntt::{mlkem_inv_ntt, mlkem_ntt};
    let kz = kyber_zetas();
    let fwd_zetas: Vec<i16> = kz[1..128].to_vec();
    let x: [i16; 256] = core::array::from_fn(|i| ((i * 131 + 17) % 3329) as i16);

    // Forward: my scalar NTT (kyber zetas[1..127]), reduced to [0,q), == native mlkem_ntt.
    let mut fwd = x;
    ntt_forward_scalar(&mut fwd, &fwd_zetas);
    let fwd_red: Vec<i64> = fwd.iter().map(|&c| (c as i64).rem_euclid(3329)).collect();
    let xi: Vec<i64> = x.iter().map(|&v| v as i64).collect();
    let native_fwd = mlkem_ntt(&xi).to_vec();
    assert_eq!(fwd_red, native_fwd, "Logos forward NTT == native mlkem_ntt (bit-exact)");

    // Inverse: feed the native forward output (NTT-domain, in [0,q)) through both inverses.
    let mut inv: [i16; 256] = core::array::from_fn(|i| native_fwd[i] as i16);
    invntt_scalar(&mut inv, &kz);
    let inv_red: Vec<i64> = inv.iter().map(|&c| (c as i64).rem_euclid(3329)).collect();
    let native_inv = mlkem_inv_ntt(&native_fwd).to_vec();
    assert_eq!(inv_red, native_inv, "Logos inverse NTT == native mlkem_inv_ntt (bit-exact)");
}

/// The Word16↔Int bridge for the ML-KEM NTT lift: `word16(n)` builds a `Seq of Word16`,
/// `lanes16Word16` packs it, `seqOfLanes16` reads Int, `intOfWord16` reads it back.
#[test]
fn word16_int_bridge_roundtrip_tw_vm() {
    let prog = "## Main\n\
        Let mutable a be a new Seq of Word16.\n\
        Repeat for i from 1 to 16:\n\
        \x20\x20\x20\x20Push word16(i * 211 + 41) to a.\n\
        Let v be lanes16Word16(a).\n\
        Repeat for x in seqOfLanes16(v):\n\x20\x20\x20\x20Show x.\n\
        Repeat for w in a:\n\x20\x20\x20\x20Show intOfWord16(w).\n";
    let want: Vec<String> = (1..=16)
        .map(|i: i64| (i * 211 + 41).to_string())
        .chain((1..=16).map(|i: i64| (i * 211 + 41).to_string()))
        .collect();
    for (label, out) in [("tw", tw_outcome(prog)), ("vm", vm_outcome(prog))] {
        assert!(out.error.is_none(), "{label} word16 bridge errored: {:?}", out.error);
        assert_eq!(
            out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            want,
            "{label}: word16/lanes/intOfWord16 round-trip"
        );
    }
}

/// Emit the shared forward-NTT Logos defs (montmul, sub16, nttWithinLevel, nttForwardAllSimd) — the
/// proven all-SIMD forward NTT, reused verbatim by the ML-KEM Word16 entry point.
fn fwd_ntt_def_block() -> String {
    let mut p = String::new();
    p.push_str("## To montmul (a: Lanes16Word16) and (b: Lanes16Word16) and (qv: Lanes16Word16) and (qinv: Lanes16Word16) -> Lanes16Word16:\n");
    p.push_str("    Let lo be a * b.\n    Let t be lo * qinv.\n    Let hi be mulhi16(a, b).\n    Let th be mulhi16(t, qv).\n    Return hi - th.\n\n");
    p.push_str("## To sub16 (s: Seq of Int) and (off: Int) -> Seq of Int:\n");
    p.push_str("    Let r be a new Seq of Int.\n    Repeat for i from 1 to 16:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    p.push_str("## To nttWithinLevel (coeffs: Seq of Int) and (zetas: Seq of Int) and (h: Int) and (k0: Int) -> Seq of Int:\n");
    p.push_str("    Let qv be splat16Word16(3329).\n    Let qinv be splat16Word16(62209).\n");
    p.push_str("    Let mutable r be a new Seq of Int.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Let blocksPerVec be 16 / (2 * h).\n    Let lanesPerBlock be 2 * h.\n");
    p.push_str("    Repeat for v from 0 to 15:\n");
    p.push_str("        Let zvSeq be a new Seq of Int.\n");
    p.push_str("        Repeat for bj from 0 to blocksPerVec - 1:\n");
    p.push_str("            Let zeta be item (k0 + v * blocksPerVec + bj + 1) of zetas.\n");
    p.push_str("            Repeat for rep from 1 to lanesPerBlock:\n                Push zeta to zvSeq.\n");
    p.push_str("        Let zv be lanes16Word16(zvSeq).\n");
    p.push_str("        Let vec be lanes16Word16(sub16(r, 16 * v)).\n");
    p.push_str("        Let lo be nttBcastLo(vec, h).\n");
    p.push_str("        Let hiB be nttBcastHi(vec, h).\n");
    p.push_str("        Let t be montmul(zv, hiB, qv, qinv).\n");
    p.push_str("        Let rs be seqOfLanes16(nttBlend(lo + t, lo - t, h)).\n");
    p.push_str("        Repeat for i from 1 to 16:\n            Set item (16 * v + i) of r to (item i of rs).\n");
    p.push_str("    Return r.\n\n");
    p.push_str("## To nttForwardAllSimd (coeffs: Seq of Int) and (zetas: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let qv be splat16Word16(3329).\n    Let qinv be splat16Word16(62209).\n");
    p.push_str("    Let mutable r be a new Seq of Int.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Let mutable k be 1.\n    Let mutable len be 128.\n");
    p.push_str("    While len is at least 16:\n");
    p.push_str("        Let numBlocks be 128 / len.\n        Let chunks be len / 16.\n");
    p.push_str("        Repeat for b from 0 to numBlocks - 1:\n");
    p.push_str("            Let zv be splat16Word16(item k of zetas).\n            Let start be b * 2 * len.\n");
    p.push_str("            Repeat for c from 0 to chunks - 1:\n");
    p.push_str("                Let lowOff be start + 16 * c.\n                Let highOff be start + len + 16 * c.\n");
    p.push_str("                Let lowVec be lanes16Word16(sub16(r, lowOff)).\n");
    p.push_str("                Let highVec be lanes16Word16(sub16(r, highOff)).\n");
    p.push_str("                Let t be montmul(zv, highVec, qv, qinv).\n");
    p.push_str("                Let nl be seqOfLanes16(lowVec + t).\n                Let nh be seqOfLanes16(lowVec - t).\n");
    p.push_str("                Repeat for i from 1 to 16:\n");
    p.push_str("                    Set item (lowOff + i) of r to (item i of nl).\n");
    p.push_str("                    Set item (highOff + i) of r to (item i of nh).\n");
    p.push_str("            Set k to k + 1.\n");
    p.push_str("        Set len to len / 2.\n");
    p.push_str("    Let mutable k0 be k - 1.\n");
    p.push_str("    Set r to nttWithinLevel(r, zetas, 8, k0).\n");
    p.push_str("    Set k0 to k0 + 16.\n");
    p.push_str("    Set r to nttWithinLevel(r, zetas, 4, k0).\n");
    p.push_str("    Set k0 to k0 + 32.\n");
    p.push_str("    Set r to nttWithinLevel(r, zetas, 2, k0).\n");
    p.push_str("    Return r.\n\n");
    p
}

/// Emit a `Push <z> to <var>.` block baking the forward kyber zetas (ZETAS[1..128], 127 values).
fn bake_fwd_zetas(var: &str) -> String {
    let kz = kyber_zetas();
    let mut p = format!("Let mutable {var} be a new Seq of Int.\n");
    for &z in &kz[1..128] {
        p.push_str(&format!("Push {} to {}.\n", z as u16, var));
    }
    p
}

/// The ML-KEM Word16 forward NTT entry, exactly as it will read in crypto.lg: Word16→Int bridge,
/// the proven lane forward NTT, then reduce each output to [0,q) and back to Word16.
fn mlkem_ntt_w16_logos_program(input: &[u16; 256]) -> String {
    let mut p = fwd_ntt_def_block();
    p.push_str("## To mlkemKemNtt (a: Seq of Word16) -> Seq of Word16:\n");
    p.push_str("    Let mutable cs be a new Seq of Int.\n    Repeat for w in a:\n        Push intOfWord16(w) to cs.\n");
    p.push_str(&format!("    {}", bake_fwd_zetas("zs").replace('\n', "\n    ")));
    p.push_str("\n    Let raw be nttForwardAllSimd(cs, zs).\n");
    p.push_str("    Let mutable out be a new Seq of Word16.\n");
    p.push_str("    Repeat for x in raw:\n");
    p.push_str("        Let v be x.\n        If v is at least 32768:\n            Set v to v - 65536.\n");
    p.push_str("        Set v to ((v % 3329) + 3329) % 3329.\n");
    p.push_str("        Push word16(v) to out.\n    Return out.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable a be a new Seq of Word16.\n");
    for &c in input {
        p.push_str(&format!("Push word16({}) to a.\n", c));
    }
    p.push_str("Let out be mlkemKemNtt(a).\n");
    p.push_str("Repeat for i from 1 to 256:\n    Show intOfWord16(item i of out).\n");
    p
}

#[test]
fn mlkem_ntt_w16_logos_matches_native_tw_vm() {
    use logicaffeine_system::ntt::mlkem_ntt_w16;
    use logicaffeine_base::Word16;
    let input: [u16; 256] = core::array::from_fn(|i| ((i * 131 + 17) % 3329) as u16);
    let native: Vec<String> = mlkem_ntt_w16(&input.map(Word16))
        .iter()
        .map(|w| (w.0 as i64).to_string())
        .collect();
    let src = mlkem_ntt_w16_logos_program(&input);
    for (label, out) in [("tw", tw_outcome(&src)), ("vm", vm_outcome(&src))] {
        assert!(out.error.is_none(), "{label} mlkemKemNtt errored: {:?}", out.error);
        assert_eq!(
            out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            native,
            "{label}: Logos mlkemKemNtt == native mlkem_ntt_w16"
        );
    }
}

/// Emit the shared inverse-NTT Logos defs (montmul, sub16, barrettReduce, invNttWithinLevel,
/// invNttAllSimd) — the proven all-SIMD inverse NTT, reused by the ML-KEM Word16 inverse entry.
fn inv_ntt_def_block() -> String {
    let mut p = String::new();
    p.push_str("## To montmul (a: Lanes16Word16) and (b: Lanes16Word16) and (qv: Lanes16Word16) and (qinv: Lanes16Word16) -> Lanes16Word16:\n");
    p.push_str("    Let lo be a * b.\n    Let t be lo * qinv.\n    Let hi be mulhi16(a, b).\n    Let th be mulhi16(t, qv).\n    Return hi - th.\n\n");
    p.push_str("## To sub16 (s: Seq of Int) and (off: Int) -> Seq of Int:\n");
    p.push_str("    Let r be a new Seq of Int.\n    Repeat for i from 1 to 16:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    p.push_str("## To barrettReduce (a: Lanes16Word16) -> Lanes16Word16:\n");
    p.push_str("    Let h be mulhi16(a, splat16Word16(20159)).\n");
    p.push_str("    Let t1 be mulhi16(h + splat16Word16(512), splat16Word16(64)).\n");
    p.push_str("    Return a - (t1 * splat16Word16(3329)).\n\n");
    p.push_str("## To invNttWithinLevel (coeffs: Seq of Int) and (zetas: Seq of Int) and (h: Int) and (kStart: Int) -> Seq of Int:\n");
    p.push_str("    Let qv be splat16Word16(3329).\n    Let qinv be splat16Word16(62209).\n");
    p.push_str("    Let mutable r be a new Seq of Int.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Let blocksPerVec be 16 / (2 * h).\n    Let lanesPerBlock be 2 * h.\n");
    p.push_str("    Repeat for v from 0 to 15:\n");
    p.push_str("        Let zvSeq be a new Seq of Int.\n");
    p.push_str("        Repeat for bj from 0 to blocksPerVec - 1:\n");
    p.push_str("            Let g be v * blocksPerVec + bj.\n");
    p.push_str("            Let zeta be item (kStart - g + 1) of zetas.\n");
    p.push_str("            Repeat for rep from 1 to lanesPerBlock:\n                Push zeta to zvSeq.\n");
    p.push_str("        Let zv be lanes16Word16(zvSeq).\n");
    p.push_str("        Let vec be lanes16Word16(sub16(r, 16 * v)).\n");
    p.push_str("        Let loB be nttBcastLo(vec, h).\n");
    p.push_str("        Let hiB be nttBcastHi(vec, h).\n");
    p.push_str("        Let sum be barrettReduce(loB + hiB).\n");
    p.push_str("        Let newHi be montmul(zv, hiB - loB, qv, qinv).\n");
    p.push_str("        Let rs be seqOfLanes16(nttBlend(sum, newHi, h)).\n");
    p.push_str("        Repeat for i from 1 to 16:\n            Set item (16 * v + i) of r to (item i of rs).\n");
    p.push_str("    Return r.\n\n");
    p.push_str("## To invNttAllSimd (coeffs: Seq of Int) and (zetas: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let qv be splat16Word16(3329).\n    Let qinv be splat16Word16(62209).\n");
    p.push_str("    Let mutable r be a new Seq of Int.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Set r to invNttWithinLevel(r, zetas, 2, 127).\n");
    p.push_str("    Set r to invNttWithinLevel(r, zetas, 4, 63).\n");
    p.push_str("    Set r to invNttWithinLevel(r, zetas, 8, 31).\n");
    p.push_str("    Let mutable kStart be 15.\n    Let mutable len be 16.\n");
    p.push_str("    While len is at most 128:\n");
    p.push_str("        Let numBlocks be 128 / len.\n        Let chunks be len / 16.\n");
    p.push_str("        Repeat for b from 0 to numBlocks - 1:\n");
    p.push_str("            Let zv be splat16Word16(item (kStart - b + 1) of zetas).\n            Let start be b * 2 * len.\n");
    p.push_str("            Repeat for c from 0 to chunks - 1:\n");
    p.push_str("                Let lowOff be start + 16 * c.\n                Let highOff be start + len + 16 * c.\n");
    p.push_str("                Let lowVec be lanes16Word16(sub16(r, lowOff)).\n");
    p.push_str("                Let highVec be lanes16Word16(sub16(r, highOff)).\n");
    p.push_str("                Let sumv be barrettReduce(lowVec + highVec).\n");
    p.push_str("                Let newHi be montmul(zv, highVec - lowVec, qv, qinv).\n");
    p.push_str("                Let nl be seqOfLanes16(sumv).\n                Let nh be seqOfLanes16(newHi).\n");
    p.push_str("                Repeat for i from 1 to 16:\n");
    p.push_str("                    Set item (lowOff + i) of r to (item i of nl).\n");
    p.push_str("                    Set item (highOff + i) of r to (item i of nh).\n");
    p.push_str("        Set kStart to kStart - numBlocks.\n");
    p.push_str("        Set len to len * 2.\n");
    p.push_str("    Let fv be splat16Word16(1441).\n");
    p.push_str("    Repeat for v from 0 to 15:\n");
    p.push_str("        Let vec be lanes16Word16(sub16(r, 16 * v)).\n");
    p.push_str("        Let scaled be seqOfLanes16(montmul(fv, vec, qv, qinv)).\n");
    p.push_str("        Repeat for i from 1 to 16:\n            Set item (16 * v + i) of r to (item i of scaled).\n");
    p.push_str("    Return r.\n\n");
    p
}

/// Bake the FULL kyber zetas (ZETAS[0..128], 128 values) for the inverse (item 1 = ZETAS[0]).
fn bake_inv_zetas(var: &str) -> String {
    let kz = kyber_zetas();
    let mut p = format!("Let mutable {var} be a new Seq of Int.\n");
    for &z in &kz[..] {
        p.push_str(&format!("Push {} to {}.\n", z as u16, var));
    }
    p
}

/// The ML-KEM Word16 inverse NTT entry: Word16→Int bridge, the proven lane inverse NTT, reduce → Word16.
fn mlkem_invntt_w16_logos_program(input: &[u16; 256]) -> String {
    let mut p = inv_ntt_def_block();
    p.push_str("## To mlkemKemInvNtt (a: Seq of Word16) -> Seq of Word16:\n");
    p.push_str("    Let mutable cs be a new Seq of Int.\n    Repeat for w in a:\n        Push intOfWord16(w) to cs.\n");
    p.push_str(&format!("    {}", bake_inv_zetas("zs").replace('\n', "\n    ")));
    p.push_str("\n    Let raw be invNttAllSimd(cs, zs).\n");
    p.push_str("    Let mutable out be a new Seq of Word16.\n");
    p.push_str("    Repeat for x in raw:\n");
    p.push_str("        Let v be x.\n        If v is at least 32768:\n            Set v to v - 65536.\n");
    p.push_str("        Set v to ((v % 3329) + 3329) % 3329.\n");
    p.push_str("        Push word16(v) to out.\n    Return out.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable a be a new Seq of Word16.\n");
    for &c in input {
        p.push_str(&format!("Push word16({}) to a.\n", c));
    }
    p.push_str("Let out be mlkemKemInvNtt(a).\n");
    p.push_str("Repeat for i from 1 to 256:\n    Show intOfWord16(item i of out).\n");
    p
}

#[test]
fn mlkem_invntt_w16_logos_matches_native_tw_vm() {
    use logicaffeine_base::Word16;
    use logicaffeine_system::ntt::{mlkem_inv_ntt_w16, mlkem_ntt_w16};
    let x: [u16; 256] = core::array::from_fn(|i| ((i * 131 + 17) % 3329) as u16);
    // The inverse's input is NTT-domain (in [0,q)): the native forward output.
    let fwd = mlkem_ntt_w16(&x.map(Word16));
    let inv_input: [u16; 256] = core::array::from_fn(|i| fwd[i].0);
    let native: Vec<String> = mlkem_inv_ntt_w16(&inv_input.map(Word16))
        .iter()
        .map(|w| (w.0 as i64).to_string())
        .collect();
    let src = mlkem_invntt_w16_logos_program(&inv_input);
    for (label, out) in [("tw", tw_outcome(&src)), ("vm", vm_outcome(&src))] {
        assert!(out.error.is_none(), "{label} mlkemKemInvNtt errored: {:?}", out.error);
        assert_eq!(
            out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            native,
            "{label}: Logos mlkemKemInvNtt == native mlkem_inv_ntt_w16"
        );
    }
}

#[test]
#[ignore = "compiles via rustc (slow) — Logos W16 forward+inverse NTT AOT == native kernels"]
fn mlkem_ntt_w16_logos_aot_eq_native() {
    use logicaffeine_base::Word16;
    use logicaffeine_system::ntt::{mlkem_inv_ntt_w16, mlkem_ntt_w16};
    let x: [u16; 256] = core::array::from_fn(|i| ((i * 131 + 17) % 3329) as u16);
    // Forward.
    let fwd_native: Vec<String> = mlkem_ntt_w16(&x.map(Word16)).iter().map(|w| (w.0 as i64).to_string()).collect();
    let fa = run_logos_with_args(&mlkem_ntt_w16_logos_program(&x), &[]);
    assert!(fa.success, "fwd AOT failed:\n{}", fa.stderr);
    assert_eq!(fa.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(), fwd_native, "AOT forward W16 == native");
    // Inverse (feed the native forward output).
    let inv_input: [u16; 256] = core::array::from_fn(|i| mlkem_ntt_w16(&x.map(Word16))[i].0);
    let inv_native: Vec<String> = mlkem_inv_ntt_w16(&inv_input.map(Word16)).iter().map(|w| (w.0 as i64).to_string()).collect();
    let ia = run_logos_with_args(&mlkem_invntt_w16_logos_program(&inv_input), &[]);
    assert!(ia.success, "inv AOT failed:\n{}", ia.stderr);
    assert_eq!(ia.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(), inv_native, "AOT inverse W16 == native");
}

// ───────────────────────────── ML-DSA i32 NTT (q = 8380417) ─────────────────────────────

/// The Dilithium signed Montgomery reduce `montgomery_reduce(a·b)` (q = 8380417, qinv = 58728449) —
/// the ML-DSA NTT butterfly multiply, the spec the lane `montmul32` must reproduce per lane.
fn mldsa_montmul32_scalar(a: i32, b: i32) -> i32 {
    const Q: i64 = 8_380_417;
    const QINV: i32 = 58_728_449;
    let p = a as i64 * b as i64;
    let t = (p as i32).wrapping_mul(QINV) as i64;
    ((p - t * Q) >> 32) as i32
}

fn mldsa_montmul32_program(a: &[i32; 8], b: &[i32; 8]) -> String {
    let mut p = String::new();
    p.push_str("## Main\n");
    p.push_str("Let mutable av be a new Seq of Word32.\n");
    for &x in a {
        p.push_str(&format!("Push word32({}) to av.\n", x as u32));
    }
    p.push_str("Let mutable bv be a new Seq of Word32.\n");
    for &x in b {
        p.push_str(&format!("Push word32({}) to bv.\n", x as u32));
    }
    p.push_str("Let qv be splat8Word32(word32(8380417)).\n");
    p.push_str("Let qiv be splat8Word32(word32(58728449)).\n");
    p.push_str("Let r be montmul32(lanes8Word32(av), lanes8Word32(bv), qv, qiv).\n");
    p.push_str("Repeat for x in seqOfLanes8(r):\n    Show intOfWord32(x).\n");
    p
}

#[test]
fn mldsa_montmul32_lanes_match_scalar_tw_and_vm() {
    // a, b span [−q, q] (the NTT working range), including negatives.
    let a: [i32; 8] = core::array::from_fn(|i| ((i as i32) * 1_500_007 - 4_190_000));
    let b: [i32; 8] = core::array::from_fn(|i| (i as i32) * 2_100_011 - 8_000_000);
    let want: Vec<String> =
        (0..8).map(|i| (mldsa_montmul32_scalar(a[i], b[i]) as u32 as i64).to_string()).collect();
    let src = mldsa_montmul32_program(&a, &b);
    for (label, out) in [("tw", tw_outcome(&src)), ("vm", vm_outcome(&src))] {
        assert!(out.error.is_none(), "{label} montmul32 errored: {:?}", out.error);
        assert_eq!(
            out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            want,
            "{label}: lane montmul32 == Dilithium montgomery_reduce(a·b)"
        );
    }
}

#[test]
#[ignore = "compiles via rustc (slow) — lane montmul32 AOT(vpmuldq) == Dilithium montgomery_reduce"]
fn mldsa_montmul32_aot_eq_scalar() {
    let a: [i32; 8] = core::array::from_fn(|i| ((i as i32) * 1_500_007 - 4_190_000));
    let b: [i32; 8] = core::array::from_fn(|i| (i as i32) * 2_100_011 - 8_000_000);
    let want: Vec<String> =
        (0..8).map(|i| (mldsa_montmul32_scalar(a[i], b[i]) as u32 as i64).to_string()).collect();
    let aot = run_logos_with_args(&mldsa_montmul32_program(&a, &b), &[]);
    assert!(aot.success, "AOT failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "AOT lane montmul32 (vpmuldq) == Dilithium montgomery_reduce"
    );
}

/// The Dilithium twiddle table: `zetas[i] = ζ^brv₈(i)·R mod q`, centered into (−q/2, q/2], ζ=1753,
/// R = 2³² mod q = 4193792. Matches `mldsa::zetas()`.
fn mldsa_zetas() -> [i32; 256] {
    let q = 8_380_417i64;
    let (zeta, mont) = (1753i64, 4_193_792i64);
    let mut pow = [1i64; 256];
    for i in 1..256 {
        pow[i] = pow[i - 1] * zeta % q;
    }
    core::array::from_fn(|i| {
        let br = (i as u8).reverse_bits() as usize;
        let mut v = pow[br] * mont % q;
        if v > q / 2 {
            v -= q;
        }
        v as i32
    })
}

/// The Dilithium forward NTT (CT butterfly), run for the first `num_levels` levels (len 128, 64, …).
fn mldsa_ntt_scalar_levels(a: &mut [i32; 256], zetas: &[i32; 256], num_levels: usize) {
    let mut k = 0usize;
    let mut len = 128usize;
    let mut done = 0;
    while len > 0 && done < num_levels {
        let mut start = 0;
        while start < 256 {
            k += 1;
            for j in start..start + len {
                let t = mldsa_montmul32_scalar(zetas[k], a[j + len]);
                a[j + len] = a[j] - t;
                a[j] = a[j] + t;
            }
            start += 2 * len;
        }
        len >>= 1;
        done += 1;
    }
}

/// Bake the Dilithium zetas (ZETAS[1..256], 255 values) as a `Seq of Word32` (item k = ZETAS[k]).
fn bake_mldsa_zetas(var: &str) -> String {
    let z = mldsa_zetas();
    let mut p = format!("Let mutable {var} be a new Seq of Word32.\n");
    for &v in &z[1..256] {
        p.push_str(&format!("Push word32({}) to {}.\n", v as u32, var));
    }
    p
}

/// The whole-vector ML-DSA forward NTT levels (len 128/64/32/16/8) in Logos i32 lanes — the
/// Cooley–Tukey loop batching the proven `montmul32 + add + sub` butterfly over 8-lane vectors.
fn mldsa_ntt_wholevec_program(coeffs: &[i32; 256]) -> String {
    let mut p = String::new();
    p.push_str("## To sub8 (s: Seq of Word32) and (off: Int) -> Seq of Word32:\n");
    p.push_str("    Let r be a new Seq of Word32.\n    Repeat for i from 1 to 8:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    p.push_str("## To mldsaNttWholeVec (coeffs: Seq of Word32) and (zetas: Seq of Word32) -> Seq of Word32:\n");
    p.push_str("    Let qv be splat8Word32(word32(8380417)).\n    Let qiv be splat8Word32(word32(58728449)).\n");
    p.push_str("    Let mutable r be a new Seq of Word32.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Let mutable k be 1.\n    Let mutable len be 128.\n");
    p.push_str("    While len is at least 8:\n");
    p.push_str("        Let numBlocks be 128 / len.\n        Let chunks be len / 8.\n");
    p.push_str("        Repeat for b from 0 to numBlocks - 1:\n");
    p.push_str("            Let zv be splat8Word32(item k of zetas).\n            Let start be b * 2 * len.\n");
    p.push_str("            Repeat for c from 0 to chunks - 1:\n");
    p.push_str("                Let lowOff be start + 8 * c.\n                Let highOff be start + len + 8 * c.\n");
    p.push_str("                Let lowVec be lanes8Word32(sub8(r, lowOff)).\n");
    p.push_str("                Let highVec be lanes8Word32(sub8(r, highOff)).\n");
    p.push_str("                Let t be montmul32(zv, highVec, qv, qiv).\n");
    p.push_str("                Let nl be seqOfLanes8(lowVec + t).\n                Let nh be seqOfLanes8(lowVec - t).\n");
    p.push_str("                Repeat for i from 1 to 8:\n");
    p.push_str("                    Set item (lowOff + i) of r to (item i of nl).\n");
    p.push_str("                    Set item (highOff + i) of r to (item i of nh).\n");
    p.push_str("            Set k to k + 1.\n");
    p.push_str("        Set len to len / 2.\n");
    p.push_str("    Return r.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable cs be a new Seq of Word32.\n");
    for &c in coeffs {
        p.push_str(&format!("Push word32({}) to cs.\n", c as u32));
    }
    p.push_str(&bake_mldsa_zetas("zs"));
    p.push_str("Let r be mldsaNttWholeVec(cs, zs).\n");
    p.push_str("Repeat for i from 1 to 256:\n    Show intOfWord32(item i of r).\n");
    p
}

#[test]
fn mldsa_ntt_wholevec_levels_match_scalar_tw_and_vm() {
    let coeffs: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 12347) % 8_380_417) as i32);
    let zetas = mldsa_zetas();
    let mut sc = coeffs;
    mldsa_ntt_scalar_levels(&mut sc, &zetas, 5); // the 5 whole-vector levels (len 128..8)
    let want: Vec<String> = sc.iter().map(|&c| (c as u32 as i64).to_string()).collect();
    let src = mldsa_ntt_wholevec_program(&coeffs);
    for (label, out) in [("tw", tw_outcome(&src)), ("vm", vm_outcome(&src))] {
        assert!(out.error.is_none(), "{label} mldsa wholevec errored: {:?}", out.error);
        assert_eq!(
            out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            want,
            "{label}: ML-DSA whole-vector NTT levels (i32 lanes) == scalar"
        );
    }
}

#[test]
#[ignore = "compiles via rustc (slow) — ML-DSA whole-vector forward NTT AOT(vpmuldq) == scalar"]
fn mldsa_ntt_wholevec_aot_eq_scalar() {
    let coeffs: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 12347) % 8_380_417) as i32);
    let zetas = mldsa_zetas();
    let mut sc = coeffs;
    mldsa_ntt_scalar_levels(&mut sc, &zetas, 5);
    let want: Vec<String> = sc.iter().map(|&c| (c as u32 as i64).to_string()).collect();
    let aot = run_logos_with_args(&mldsa_ntt_wholevec_program(&coeffs), &[]);
    assert!(aot.success, "AOT failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "ML-DSA whole-vector forward NTT (AOT/vpmuldq) == scalar"
    );
}

/// The COMPLETE 8-level ML-DSA forward NTT in i32 lanes: whole-vector levels (len 128…8) batch the
/// `montmul32` butterfly, within-vector levels (len 4/2/1) use the i32 stride shuffles. No scalar tail.
fn mldsa_ntt_forward_program(coeffs: &[i32; 256]) -> String {
    let mut p = String::new();
    p.push_str("## To sub8 (s: Seq of Word32) and (off: Int) -> Seq of Word32:\n");
    p.push_str("    Let r be a new Seq of Word32.\n    Repeat for i from 1 to 8:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    // The within-vector level (len = h, h ∈ {4,2,1}) — per-block zeta vector + the shuffle butterfly.
    p.push_str("## To mldsaWithinLevel (coeffs: Seq of Word32) and (zetas: Seq of Word32) and (h: Int) and (k0: Int) -> Seq of Word32:\n");
    p.push_str("    Let qv be splat8Word32(word32(8380417)).\n    Let qiv be splat8Word32(word32(58728449)).\n");
    p.push_str("    Let mutable r be a new Seq of Word32.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Let blocksPerVec be 8 / (2 * h).\n    Let lanesPerBlock be 2 * h.\n");
    p.push_str("    Repeat for v from 0 to 31:\n");
    p.push_str("        Let zvSeq be a new Seq of Word32.\n");
    p.push_str("        Repeat for bj from 0 to blocksPerVec - 1:\n");
    p.push_str("            Let zeta be item (k0 + v * blocksPerVec + bj + 1) of zetas.\n");
    p.push_str("            Repeat for rep from 1 to lanesPerBlock:\n                Push zeta to zvSeq.\n");
    p.push_str("        Let zv be lanes8Word32(zvSeq).\n");
    p.push_str("        Let vec be lanes8Word32(sub8(r, 8 * v)).\n");
    p.push_str("        Let lo be nttBcastLo(vec, h).\n");
    p.push_str("        Let hiB be nttBcastHi(vec, h).\n");
    p.push_str("        Let t be montmul32(zv, hiB, qv, qiv).\n");
    p.push_str("        Let rs be seqOfLanes8(nttBlend(lo + t, lo - t, h)).\n");
    p.push_str("        Repeat for i from 1 to 8:\n            Set item (8 * v + i) of r to (item i of rs).\n");
    p.push_str("    Return r.\n\n");
    p.push_str("## To mldsaNttForward (coeffs: Seq of Word32) and (zetas: Seq of Word32) -> Seq of Word32:\n");
    p.push_str("    Let qv be splat8Word32(word32(8380417)).\n    Let qiv be splat8Word32(word32(58728449)).\n");
    p.push_str("    Let mutable r be a new Seq of Word32.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Let mutable k be 1.\n    Let mutable len be 128.\n");
    p.push_str("    While len is at least 8:\n");
    p.push_str("        Let numBlocks be 128 / len.\n        Let chunks be len / 8.\n");
    p.push_str("        Repeat for b from 0 to numBlocks - 1:\n");
    p.push_str("            Let zv be splat8Word32(item k of zetas).\n            Let start be b * 2 * len.\n");
    p.push_str("            Repeat for c from 0 to chunks - 1:\n");
    p.push_str("                Let lowOff be start + 8 * c.\n                Let highOff be start + len + 8 * c.\n");
    p.push_str("                Let lowVec be lanes8Word32(sub8(r, lowOff)).\n");
    p.push_str("                Let highVec be lanes8Word32(sub8(r, highOff)).\n");
    p.push_str("                Let t be montmul32(zv, highVec, qv, qiv).\n");
    p.push_str("                Let nl be seqOfLanes8(lowVec + t).\n                Let nh be seqOfLanes8(lowVec - t).\n");
    p.push_str("                Repeat for i from 1 to 8:\n");
    p.push_str("                    Set item (lowOff + i) of r to (item i of nl).\n");
    p.push_str("                    Set item (highOff + i) of r to (item i of nh).\n");
    p.push_str("            Set k to k + 1.\n");
    p.push_str("        Set len to len / 2.\n");
    p.push_str("    Let mutable k0 be k - 1.\n");
    p.push_str("    Set r to mldsaWithinLevel(r, zetas, 4, k0).\n");
    p.push_str("    Set k0 to k0 + 32.\n");
    p.push_str("    Set r to mldsaWithinLevel(r, zetas, 2, k0).\n");
    p.push_str("    Set k0 to k0 + 64.\n");
    p.push_str("    Set r to mldsaWithinLevel(r, zetas, 1, k0).\n");
    p.push_str("    Return r.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable cs be a new Seq of Word32.\n");
    for &c in coeffs {
        p.push_str(&format!("Push word32({}) to cs.\n", c as u32));
    }
    p.push_str(&bake_mldsa_zetas("zs"));
    p.push_str("Let r be mldsaNttForward(cs, zs).\n");
    p.push_str("Repeat for i from 1 to 256:\n    Show intOfWord32(item i of r).\n");
    p
}

#[test]
fn mldsa_ntt_forward_full_match_scalar_tw_and_vm() {
    let coeffs: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 12347) % 8_380_417) as i32);
    let zetas = mldsa_zetas();
    let mut sc = coeffs;
    mldsa_ntt_scalar_levels(&mut sc, &zetas, 8); // all 8 levels
    let want: Vec<String> = sc.iter().map(|&c| (c as u32 as i64).to_string()).collect();
    let src = mldsa_ntt_forward_program(&coeffs);
    for (label, out) in [("tw", tw_outcome(&src)), ("vm", vm_outcome(&src))] {
        assert!(out.error.is_none(), "{label} mldsa forward errored: {:?}", out.error);
        assert_eq!(
            out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            want,
            "{label}: full 8-level ML-DSA forward NTT (i32 lanes) == scalar"
        );
    }
}

#[test]
#[ignore = "compiles via rustc (slow) — full 8-level ML-DSA forward NTT AOT(vpmuldq/vpshufd) == scalar"]
fn mldsa_ntt_forward_full_aot_eq_scalar() {
    let coeffs: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 12347) % 8_380_417) as i32);
    let zetas = mldsa_zetas();
    let mut sc = coeffs;
    mldsa_ntt_scalar_levels(&mut sc, &zetas, 8);
    let want: Vec<String> = sc.iter().map(|&c| (c as u32 as i64).to_string()).collect();
    let aot = run_logos_with_args(&mldsa_ntt_forward_program(&coeffs), &[]);
    assert!(aot.success, "AOT failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "full 8-level ML-DSA forward NTT (AOT/vpmuldq+vpshufd) == scalar"
    );
}

/// The Dilithium inverse NTT (`invntt_tomont`): GS butterfly `lo'=lo+hi; hi'=montmul(zeta, hi−lo)`
/// (= `montmul(−ZETAS[k], lo−hi)`), 8 levels len 1…128, then the final `f = mont²/256 = 41978` scale.
fn mldsa_invntt_scalar(a: &mut [i32; 256], zetas: &[i32; 256]) {
    const F: i32 = 41_978;
    let mut k = 256usize;
    let mut len = 1usize;
    while len < 256 {
        let mut start = 0;
        while start < 256 {
            k -= 1;
            for j in start..start + len {
                let t = a[j];
                let hi = a[j + len];
                a[j] = t + hi;
                a[j + len] = mldsa_montmul32_scalar(zetas[k], hi - t);
            }
            start += 2 * len;
        }
        len <<= 1;
    }
    for x in a.iter_mut() {
        *x = mldsa_montmul32_scalar(F, *x);
    }
}

/// Bake the full Dilithium zetas (ZETAS[0..256]) as a `Seq of Word32` — item k = ZETAS[k−1], so the
/// inverse `item (kStart−g+1)` reads ZETAS[kStart−g].
fn bake_mldsa_zetas_full(var: &str) -> String {
    let z = mldsa_zetas();
    let mut p = format!("Let mutable {var} be a new Seq of Word32.\n");
    for &v in &z[..] {
        p.push_str(&format!("Push word32({}) to {}.\n", v as u32, var));
    }
    p
}

/// The COMPLETE 8-level ML-DSA inverse NTT in i32 lanes: within-vector GS levels (len 1/2/4) via the
/// i32 stride shuffles, then whole-vector GS levels (len 8…128), then the `f=41978` montmul32 scale.
fn mldsa_invntt_program(coeffs: &[i32; 256]) -> String {
    let mut p = String::new();
    p.push_str("## To sub8 (s: Seq of Word32) and (off: Int) -> Seq of Word32:\n");
    p.push_str("    Let r be a new Seq of Word32.\n    Repeat for i from 1 to 8:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    p.push_str("## To mldsaInvWithinLevel (coeffs: Seq of Word32) and (zetas: Seq of Word32) and (h: Int) and (kStart: Int) -> Seq of Word32:\n");
    p.push_str("    Let qv be splat8Word32(word32(8380417)).\n    Let qiv be splat8Word32(word32(58728449)).\n");
    p.push_str("    Let mutable r be a new Seq of Word32.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Let blocksPerVec be 8 / (2 * h).\n    Let lanesPerBlock be 2 * h.\n");
    p.push_str("    Repeat for v from 0 to 31:\n");
    p.push_str("        Let zvSeq be a new Seq of Word32.\n");
    p.push_str("        Repeat for bj from 0 to blocksPerVec - 1:\n");
    p.push_str("            Let g be v * blocksPerVec + bj.\n");
    p.push_str("            Let zeta be item (kStart - g + 1) of zetas.\n");
    p.push_str("            Repeat for rep from 1 to lanesPerBlock:\n                Push zeta to zvSeq.\n");
    p.push_str("        Let zv be lanes8Word32(zvSeq).\n");
    p.push_str("        Let vec be lanes8Word32(sub8(r, 8 * v)).\n");
    p.push_str("        Let loB be nttBcastLo(vec, h).\n");
    p.push_str("        Let hiB be nttBcastHi(vec, h).\n");
    p.push_str("        Let sum be loB + hiB.\n");
    p.push_str("        Let newHi be montmul32(zv, hiB - loB, qv, qiv).\n");
    p.push_str("        Let rs be seqOfLanes8(nttBlend(sum, newHi, h)).\n");
    p.push_str("        Repeat for i from 1 to 8:\n            Set item (8 * v + i) of r to (item i of rs).\n");
    p.push_str("    Return r.\n\n");
    p.push_str("## To mldsaInvNtt (coeffs: Seq of Word32) and (zetas: Seq of Word32) -> Seq of Word32:\n");
    p.push_str("    Let qv be splat8Word32(word32(8380417)).\n    Let qiv be splat8Word32(word32(58728449)).\n");
    p.push_str("    Let mutable r be a new Seq of Word32.\n    Repeat for x in coeffs:\n        Push x to r.\n");
    p.push_str("    Set r to mldsaInvWithinLevel(r, zetas, 1, 255).\n");
    p.push_str("    Set r to mldsaInvWithinLevel(r, zetas, 2, 127).\n");
    p.push_str("    Set r to mldsaInvWithinLevel(r, zetas, 4, 63).\n");
    p.push_str("    Let mutable kStart be 31.\n    Let mutable len be 8.\n");
    p.push_str("    While len is at most 128:\n");
    p.push_str("        Let numBlocks be 128 / len.\n        Let chunks be len / 8.\n");
    p.push_str("        Repeat for b from 0 to numBlocks - 1:\n");
    p.push_str("            Let zv be splat8Word32(item (kStart - b + 1) of zetas).\n            Let start be b * 2 * len.\n");
    p.push_str("            Repeat for c from 0 to chunks - 1:\n");
    p.push_str("                Let lowOff be start + 8 * c.\n                Let highOff be start + len + 8 * c.\n");
    p.push_str("                Let lowVec be lanes8Word32(sub8(r, lowOff)).\n");
    p.push_str("                Let highVec be lanes8Word32(sub8(r, highOff)).\n");
    p.push_str("                Let sumv be lowVec + highVec.\n");
    p.push_str("                Let newHi be montmul32(zv, highVec - lowVec, qv, qiv).\n");
    p.push_str("                Let nl be seqOfLanes8(sumv).\n                Let nh be seqOfLanes8(newHi).\n");
    p.push_str("                Repeat for i from 1 to 8:\n");
    p.push_str("                    Set item (lowOff + i) of r to (item i of nl).\n");
    p.push_str("                    Set item (highOff + i) of r to (item i of nh).\n");
    p.push_str("        Set kStart to kStart - numBlocks.\n");
    p.push_str("        Set len to len * 2.\n");
    p.push_str("    Let fv be splat8Word32(word32(41978)).\n");
    p.push_str("    Repeat for v from 0 to 31:\n");
    p.push_str("        Let vec be lanes8Word32(sub8(r, 8 * v)).\n");
    p.push_str("        Let scaled be seqOfLanes8(montmul32(fv, vec, qv, qiv)).\n");
    p.push_str("        Repeat for i from 1 to 8:\n            Set item (8 * v + i) of r to (item i of scaled).\n");
    p.push_str("    Return r.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable cs be a new Seq of Word32.\n");
    for &c in coeffs {
        p.push_str(&format!("Push word32({}) to cs.\n", c as u32));
    }
    p.push_str(&bake_mldsa_zetas_full("zs"));
    p.push_str("Let r be mldsaInvNtt(cs, zs).\n");
    p.push_str("Repeat for i from 1 to 256:\n    Show intOfWord32(item i of r).\n");
    p
}

#[test]
fn mldsa_invntt_full_match_scalar_tw_and_vm() {
    // Inverse input = a real NTT-domain vector (the forward output of some p).
    let p: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 12347) % 8_380_417) as i32);
    let zetas = mldsa_zetas();
    let mut fwd = p;
    mldsa_ntt_scalar_levels(&mut fwd, &zetas, 8);
    let mut sc = fwd;
    mldsa_invntt_scalar(&mut sc, &zetas);
    let want: Vec<String> = sc.iter().map(|&c| (c as u32 as i64).to_string()).collect();
    let src = mldsa_invntt_program(&fwd);
    for (label, out) in [("tw", tw_outcome(&src)), ("vm", vm_outcome(&src))] {
        assert!(out.error.is_none(), "{label} mldsa invntt errored: {:?}", out.error);
        assert_eq!(
            out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            want,
            "{label}: full 8-level ML-DSA inverse NTT (i32 lanes) == scalar"
        );
    }
}

/// The ring isomorphism for ML-DSA: `invntt_tomont(ntt(p)) ≡ c·p mod q` for a fixed c — validates
/// the Dilithium zetas + the forward/inverse scalar pipelines the lanes are proven equal to.
#[test]
fn mldsa_ntt_invntt_roundtrip_is_scalar_multiple_of_identity() {
    let q = 8_380_417i64;
    let zetas = mldsa_zetas();
    let p: [i32; 256] = core::array::from_fn(|i| (((i as i64 * 12347) % q) + 1) as i32); // p[i] != 0
    let mut a = p;
    mldsa_ntt_scalar_levels(&mut a, &zetas, 8);
    mldsa_invntt_scalar(&mut a, &zetas);
    let modinv = |v: i64| -> i64 {
        let (mut r, mut e, mut base) = (1i64, q - 2, v.rem_euclid(q));
        while e > 0 {
            if e & 1 == 1 {
                r = r * base % q;
            }
            base = base * base % q;
            e >>= 1;
        }
        r
    };
    let c = (a[0] as i64).rem_euclid(q) * modinv(p[0] as i64) % q;
    for i in 0..256 {
        assert_eq!(
            (a[i] as i64).rem_euclid(q),
            (c * p[i] as i64).rem_euclid(q),
            "invntt(ntt(p))[{i}] == c·p[{i}] mod q (c = {c})"
        );
    }
}

#[test]
#[ignore = "compiles via rustc (slow) — full 8-level ML-DSA inverse NTT AOT == scalar"]
fn mldsa_invntt_full_aot_eq_scalar() {
    let p: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 12347) % 8_380_417) as i32);
    let zetas = mldsa_zetas();
    let mut fwd = p;
    mldsa_ntt_scalar_levels(&mut fwd, &zetas, 8);
    let mut sc = fwd;
    mldsa_invntt_scalar(&mut sc, &zetas);
    let want: Vec<String> = sc.iter().map(|&c| (c as u32 as i64).to_string()).collect();
    let aot = run_logos_with_args(&mldsa_invntt_program(&fwd), &[]);
    assert!(aot.success, "AOT failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "full 8-level ML-DSA inverse NTT (AOT) == scalar"
    );
}

/// The schoolbook negacyclic convolution in ℤ_q[X]/(X²⁵⁶+1) — the ground truth the NTT product must
/// reproduce (`X²⁵⁶ = −1`), reduced to [0,q).
fn mldsa_schoolbook(a: &[i32; 256], b: &[i32; 256]) -> [i32; 256] {
    let q = 8_380_417i64;
    let mut c = [0i64; 256];
    for i in 0..256 {
        for j in 0..256 {
            let prod = a[i] as i64 * b[j] as i64;
            if i + j < 256 {
                c[i + j] += prod;
            } else {
                c[i + j - 256] -= prod;
            }
        }
    }
    core::array::from_fn(|i| (((c[i] % q) + q) % q) as i32)
}

/// All five ML-DSA NTT Logos defs (sub8 + forward CT within-level + forward NTT + inverse GS
/// within-level + inverse NTT) — the shared block the polynomial-multiply program reuses.
fn mldsa_ntt_defs() -> String {
    let fwd = mldsa_ntt_forward_program(&[0i32; 256]);
    let inv = mldsa_invntt_program(&[0i32; 256]);
    // Take each builder's defs up to its `## Main` (the defs are identical to the proven programs).
    let fwd_defs = &fwd[..fwd.find("## Main").unwrap()];
    let inv_defs = &inv[..inv.find("## Main").unwrap()];
    // The inverse block repeats `sub8` — strip it (keep only the inverse-specific functions).
    let inv_only = &inv_defs[inv_defs.find("## To mldsaInvWithinLevel").unwrap()..];
    format!("{fwd_defs}{inv_only}")
}

/// The full ML-DSA polynomial multiply in Logos i32 lanes: `freeze(invNtt(pointwise(ntt(a),ntt(b))))`
/// — the negacyclic ring product the verify's `A·z` / `c·t1` are built from.
fn mldsa_polymul_program(a: &[i32; 256], b: &[i32; 256]) -> String {
    let mut p = mldsa_ntt_defs();
    p.push_str("## To pointwiseMontgomery (a: Seq of Word32) and (b: Seq of Word32) -> Seq of Word32:\n");
    p.push_str("    Let qv be splat8Word32(word32(8380417)).\n    Let qiv be splat8Word32(word32(58728449)).\n");
    p.push_str("    Let mutable out be a new Seq of Word32.\n");
    p.push_str("    Repeat for v from 0 to 31:\n");
    p.push_str("        Let av be lanes8Word32(sub8(a, 8 * v)).\n");
    p.push_str("        Let bv be lanes8Word32(sub8(b, 8 * v)).\n");
    p.push_str("        Repeat for x in seqOfLanes8(montmul32(av, bv, qv, qiv)):\n            Push x to out.\n");
    p.push_str("    Return out.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable csa be a new Seq of Word32.\n");
    for &c in a {
        p.push_str(&format!("Push word32({}) to csa.\n", c as u32));
    }
    p.push_str("Let mutable csb be a new Seq of Word32.\n");
    for &c in b {
        p.push_str(&format!("Push word32({}) to csb.\n", c as u32));
    }
    p.push_str(&bake_mldsa_zetas("zsF"));
    p.push_str(&bake_mldsa_zetas_full("zsI"));
    p.push_str("Let ahat be mldsaNttForward(csa, zsF).\n");
    p.push_str("Let bhat be mldsaNttForward(csb, zsF).\n");
    p.push_str("Let chat be pointwiseMontgomery(ahat, bhat).\n");
    p.push_str("Let c be mldsaInvNtt(chat, zsI).\n");
    p.push_str("Repeat for i from 1 to 256:\n");
    p.push_str("    Let raw be intOfWord32(item i of c).\n");
    p.push_str("    Let v be raw.\n    If v is at least 2147483648:\n        Set v to v - 4294967296.\n");
    p.push_str("    Show ((v % 8380417) + 8380417) % 8380417.\n");
    p
}

#[test]
fn mldsa_polymul_matches_schoolbook_tw_and_vm() {
    // The NTT-based product == the schoolbook negacyclic convolution (the convolution theorem).
    let a: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 7919 + 13) % 8_380_417) as i32);
    let b: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 104729 + 77) % 8_380_417) as i32);
    let want: Vec<String> = mldsa_schoolbook(&a, &b).iter().map(|&c| (c as i64).to_string()).collect();
    let src = mldsa_polymul_program(&a, &b);
    for (label, out) in [("tw", tw_outcome(&src)), ("vm", vm_outcome(&src))] {
        assert!(out.error.is_none(), "{label} mldsa polymul errored: {:?}", out.error);
        assert_eq!(
            out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            want,
            "{label}: ML-DSA NTT polynomial multiply == schoolbook convolution"
        );
    }
}

#[test]
#[ignore = "compiles via rustc (slow) — ML-DSA NTT polynomial multiply AOT == schoolbook convolution"]
fn mldsa_polymul_matches_schoolbook_aot() {
    let a: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 7919 + 13) % 8_380_417) as i32);
    let b: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 104729 + 77) % 8_380_417) as i32);
    let want: Vec<String> = mldsa_schoolbook(&a, &b).iter().map(|&c| (c as i64).to_string()).collect();
    let aot = run_logos_with_args(&mldsa_polymul_program(&a, &b), &[]);
    assert!(aot.success, "AOT failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "ML-DSA NTT polynomial multiply (AOT) == schoolbook convolution"
    );
}

/// One row of the ML-DSA matrix-vector product `A·z` in Logos i32 lanes: accumulate
/// `Σ_j pointwise(ntt(A[j]), ntt(z[j]))` in the NTT domain (lane add), then invNTT + freeze in Main.
/// Proven == `Σ_j schoolbook(A[j], z[j])` — the linearity of the NTT over the row's polynomials.
fn mldsa_matvec_program(a_polys: &[[i32; 256]], z_polys: &[[i32; 256]]) -> String {
    let l = a_polys.len();
    assert_eq!(l, z_polys.len());
    let mut p = mldsa_ntt_defs();
    // pointwiseMontgomery
    p.push_str("## To pointwiseMontgomery (a: Seq of Word32) and (b: Seq of Word32) -> Seq of Word32:\n");
    p.push_str("    Let qv be splat8Word32(word32(8380417)).\n    Let qiv be splat8Word32(word32(58728449)).\n");
    p.push_str("    Let mutable out be a new Seq of Word32.\n    Repeat for v from 0 to 31:\n");
    p.push_str("        Let av be lanes8Word32(sub8(a, 8 * v)).\n        Let bv be lanes8Word32(sub8(b, 8 * v)).\n");
    p.push_str("        Repeat for x in seqOfLanes8(montmul32(av, bv, qv, qiv)):\n            Push x to out.\n");
    p.push_str("    Return out.\n\n");
    // polyAdd — coefficient-wise lane add (NTT-domain accumulate).
    p.push_str("## To polyAdd (a: Seq of Word32) and (b: Seq of Word32) -> Seq of Word32:\n");
    p.push_str("    Let mutable out be a new Seq of Word32.\n    Repeat for v from 0 to 31:\n");
    p.push_str("        Let av be lanes8Word32(sub8(a, 8 * v)).\n        Let bv be lanes8Word32(sub8(b, 8 * v)).\n");
    p.push_str("        Repeat for x in seqOfLanes8(av + bv):\n            Push x to out.\n");
    p.push_str("    Return out.\n\n");
    // sub256 — extract one 256-coefficient polynomial from a flattened vector.
    p.push_str("## To sub256 (s: Seq of Word32) and (off: Int) -> Seq of Word32:\n");
    p.push_str("    Let r be a new Seq of Word32.\n    Repeat for i from 1 to 256:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    // mldsaMatvecRow — Σ_j pointwise(ntt(A[j]), ntt(z[j])), in NTT domain.
    p.push_str("## To mldsaMatvecRow (aFlat: Seq of Word32) and (zFlat: Seq of Word32) and (zsF: Seq of Word32) and (l: Int) -> Seq of Word32:\n");
    p.push_str("    Let mutable acc be a new Seq of Word32.\n    Repeat for i from 1 to 256:\n        Push word32(0) to acc.\n");
    p.push_str("    Repeat for j from 0 to l - 1:\n");
    p.push_str("        Let ajHat be mldsaNttForward(sub256(aFlat, 256 * j), zsF).\n");
    p.push_str("        Let zjHat be mldsaNttForward(sub256(zFlat, 256 * j), zsF).\n");
    p.push_str("        Set acc to polyAdd(acc, pointwiseMontgomery(ajHat, zjHat)).\n");
    p.push_str("    Return acc.\n\n");
    p.push_str("## Main\n");
    p.push_str("Let mutable aFlat be a new Seq of Word32.\n");
    for poly in a_polys {
        for &c in poly {
            p.push_str(&format!("Push word32({}) to aFlat.\n", c as u32));
        }
    }
    p.push_str("Let mutable zFlat be a new Seq of Word32.\n");
    for poly in z_polys {
        for &c in poly {
            p.push_str(&format!("Push word32({}) to zFlat.\n", c as u32));
        }
    }
    p.push_str(&bake_mldsa_zetas("zsF"));
    p.push_str(&bake_mldsa_zetas_full("zsI"));
    p.push_str(&format!("Let acc be mldsaMatvecRow(aFlat, zFlat, zsF, {}).\n", l));
    p.push_str("Let c be mldsaInvNtt(acc, zsI).\n");
    p.push_str("Repeat for i from 1 to 256:\n");
    p.push_str("    Let raw be intOfWord32(item i of c).\n    Let v be raw.\n    If v is at least 2147483648:\n        Set v to v - 4294967296.\n");
    p.push_str("    Show ((v % 8380417) + 8380417) % 8380417.\n");
    p
}

#[test]
fn mldsa_matvec_row_matches_sum_of_schoolbook_tw_and_vm() {
    let l = 3;
    let a_polys: Vec<[i32; 256]> = (0..l)
        .map(|j| core::array::from_fn(|i| ((i as i64 * 7919 + j as i64 * 31 + 13) % 8_380_417) as i32))
        .collect();
    let z_polys: Vec<[i32; 256]> = (0..l)
        .map(|j| core::array::from_fn(|i| ((i as i64 * 104729 + j as i64 * 53 + 77) % 8_380_417) as i32))
        .collect();
    // Σ_j schoolbook(A[j], z[j]) mod q.
    let q = 8_380_417i64;
    let mut acc = [0i64; 256];
    for j in 0..l {
        let sj = mldsa_schoolbook(&a_polys[j], &z_polys[j]);
        for i in 0..256 {
            acc[i] = (acc[i] + sj[i] as i64) % q;
        }
    }
    let want: Vec<String> = acc.iter().map(|&c| c.to_string()).collect();
    let src = mldsa_matvec_program(&a_polys, &z_polys);
    for (label, out) in [("tw", tw_outcome(&src)), ("vm", vm_outcome(&src))] {
        assert!(out.error.is_none(), "{label} mldsa matvec errored: {:?}", out.error);
        assert_eq!(
            out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            want,
            "{label}: ML-DSA matrix-vector row (A·z) == Σ schoolbook"
        );
    }
}

#[test]
#[ignore = "compiles via rustc (slow) — ML-DSA matrix-vector row AOT == Σ schoolbook"]
fn mldsa_matvec_row_matches_sum_of_schoolbook_aot() {
    let l = 3;
    let a_polys: Vec<[i32; 256]> = (0..l)
        .map(|j| core::array::from_fn(|i| ((i as i64 * 7919 + j as i64 * 31 + 13) % 8_380_417) as i32))
        .collect();
    let z_polys: Vec<[i32; 256]> = (0..l)
        .map(|j| core::array::from_fn(|i| ((i as i64 * 104729 + j as i64 * 53 + 77) % 8_380_417) as i32))
        .collect();
    let q = 8_380_417i64;
    let mut acc = [0i64; 256];
    for j in 0..l {
        let sj = mldsa_schoolbook(&a_polys[j], &z_polys[j]);
        for i in 0..256 {
            acc[i] = (acc[i] + sj[i] as i64) % q;
        }
    }
    let want: Vec<String> = acc.iter().map(|&c| c.to_string()).collect();
    let aot = run_logos_with_args(&mldsa_matvec_program(&a_polys, &z_polys), &[]);
    assert!(aot.success, "AOT failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "ML-DSA matrix-vector row (AOT) == Σ schoolbook"
    );
}


/// `mldsaNttMul` resolved from the crypto.lg stdlib (auto-import) — the shipped Logos ML-DSA
/// polynomial multiply, proven == the schoolbook negacyclic convolution.
fn mldsa_nttmul_via_cryptolg_program(a: &[i32; 256], b: &[i32; 256]) -> String {
    let mut p = String::new();
    p.push_str("## Main\n");
    p.push_str("Let mutable csa be a new Seq of Word32.\n");
    for &c in a {
        p.push_str(&format!("Push word32({}) to csa.\n", c as u32));
    }
    p.push_str("Let mutable csb be a new Seq of Word32.\n");
    for &c in b {
        p.push_str(&format!("Push word32({}) to csb.\n", c as u32));
    }
    p.push_str("Let r be mldsaNttMul(csa, csb).\n");
    p.push_str("Repeat for i from 1 to 256:\n    Show intOfWord32(item i of r).\n");
    p
}

#[test]
fn mldsa_nttmul_cryptolg_matches_schoolbook_tw_and_vm() {
    let a: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 7919 + 13) % 8_380_417) as i32);
    let b: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 104729 + 77) % 8_380_417) as i32);
    let want: Vec<String> = mldsa_schoolbook(&a, &b).iter().map(|&c| (c as i64).to_string()).collect();
    let src = mldsa_nttmul_via_cryptolg_program(&a, &b);
    for (label, out) in [("tw", tw_outcome(&src)), ("vm", vm_outcome(&src))] {
        assert!(out.error.is_none(), "{label} crypto.lg mldsaNttMul errored: {:?}", out.error);
        assert_eq!(
            out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            want,
            "{label}: crypto.lg mldsaNttMul == schoolbook convolution"
        );
    }
}

#[test]
#[ignore = "compiles via rustc (slow) — crypto.lg mldsaNttMul AOT == schoolbook"]
fn mldsa_nttmul_cryptolg_matches_schoolbook_aot() {
    let a: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 7919 + 13) % 8_380_417) as i32);
    let b: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 104729 + 77) % 8_380_417) as i32);
    let want: Vec<String> = mldsa_schoolbook(&a, &b).iter().map(|&c| (c as i64).to_string()).collect();
    let aot = run_logos_with_args(&mldsa_nttmul_via_cryptolg_program(&a, &b), &[]);
    assert!(aot.success, "AOT failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "crypto.lg mldsaNttMul (AOT) == schoolbook convolution"
    );
}

// ───────────────────────── ML-DSA non-NTT verify glue: rounding (FIPS-204 §7.4) ─────────────────────────

const MLDSA_Q: i32 = 8_380_417;
const MLDSA_GAMMA2: i32 = (MLDSA_Q - 1) / 32; // 261888

fn rdecompose(r: i32) -> (i32, i32) {
    let r = ((r % MLDSA_Q) + MLDSA_Q) % MLDSA_Q;
    let mut a1 = (r + 127) >> 7;
    a1 = (a1 * 1025 + (1 << 21)) >> 22;
    a1 &= 15;
    let mut a0 = r - a1 * 2 * MLDSA_GAMMA2;
    a0 -= (((MLDSA_Q - 1) / 2 - a0) >> 31) & MLDSA_Q;
    (a1, a0)
}
fn rhighbits(r: i32) -> i32 {
    rdecompose(r).0
}
fn ruse_hint(h: i32, r: i32) -> i32 {
    let (r1, r0) = rdecompose(r);
    if h == 0 {
        r1
    } else if r0 > 0 {
        (r1 + 1) % 16
    } else {
        (r1 - 1 + 16) % 16
    }
}
fn rmake_hint(z: i32, r: i32) -> i32 {
    (rhighbits(r) != rhighbits(r + z)) as i32
}

/// The FIPS-204 §7.4 self-validation: `UseHint(MakeHint(z, r), r) = HighBits(r + z)` for |z| ≤ γ2.
/// Confirms the Rust spec replica the Logos versions are proven equal to is itself correct.
#[test]
fn mldsa_use_hint_inverts_make_hint_spec() {
    let mut s = 0xFEED_FACEu64;
    for _ in 0..50000 {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
        let r = (s >> 33) as i32 % MLDSA_Q;
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
        let z = ((s >> 40) as i32 % (2 * MLDSA_GAMMA2 + 1)) - MLDSA_GAMMA2;
        let h = rmake_hint(z, r);
        assert_eq!(ruse_hint(h, r), rhighbits(r + z), "UseHint∘MakeHint = HighBits(r+z)");
    }
}

fn mldsa_rounding_glue_defs() -> String {
    let mut p = String::new();
    p.push_str("## To mldsaFreezeI (r: Int) -> Int:\n    Return ((r % 8380417) + 8380417) % 8380417.\n\n");
    p.push_str("## To mldsaHighBits (r: Int) -> Int:\n");
    p.push_str("    Let rr be mldsaFreezeI(r).\n    Let a1 be (rr + 127) / 128.\n");
    p.push_str("    Set a1 to (a1 * 1025 + 2097152) / 4194304.\n    Return a1 % 16.\n\n");
    p.push_str("## To mldsaUseHint (h: Int) and (r: Int) -> Int:\n");
    p.push_str("    Let rr be mldsaFreezeI(r).\n    Let a1 be (rr + 127) / 128.\n");
    p.push_str("    Set a1 to (a1 * 1025 + 2097152) / 4194304.\n    Set a1 to a1 % 16.\n");
    p.push_str("    Let a0 be rr - a1 * 523776.\n    If a0 is greater than 4190208:\n        Set a0 to a0 - 8380417.\n");
    p.push_str("    If h is equal to 0:\n        Return a1.\n");
    p.push_str("    If a0 is greater than 0:\n        Return (a1 + 1) % 16.\n");
    p.push_str("    Return (a1 - 1 + 16) % 16.\n\n");
    p
}

fn mldsa_rounding_program(rs: &[i32], hs: &[i32]) -> String {
    let mut p = mldsa_rounding_glue_defs();
    p.push_str("## Main\n");
    p.push_str("Let mutable rs be a new Seq of Int.\n");
    for &r in rs {
        p.push_str(&format!("Push {} to rs.\n", r));
    }
    p.push_str("Let mutable hs be a new Seq of Int.\n");
    for &h in hs {
        p.push_str(&format!("Push {} to hs.\n", h));
    }
    p.push_str("Repeat for i from 1 to ");
    p.push_str(&format!("{}", rs.len()));
    p.push_str(":\n    Show mldsaHighBits(item i of rs).\n");
    p.push_str("Repeat for i from 1 to ");
    p.push_str(&format!("{}", rs.len()));
    p.push_str(":\n    Show mldsaUseHint(item i of hs, item i of rs).\n");
    p
}

#[test]
fn mldsa_highbits_usehint_logos_match_spec_tw_and_vm() {
    // r spanning [0,q) including the boundary regions; h = MakeHint(z, r) for a spread of z.
    let rs: Vec<i32> = (0..256).map(|i| ((i as i64 * 32749 + 17) % MLDSA_Q as i64) as i32).collect();
    let zs: Vec<i32> = (0..256).map(|i| ((i as i32 * 4099) % (2 * MLDSA_GAMMA2 + 1)) - MLDSA_GAMMA2).collect();
    let hs: Vec<i32> = (0..256).map(|i| rmake_hint(zs[i], rs[i])).collect();
    let want: Vec<String> = rs
        .iter()
        .map(|&r| rhighbits(r).to_string())
        .chain((0..256).map(|i| ruse_hint(hs[i], rs[i]).to_string()))
        .collect();
    // Cross-check the property holds for these inputs (validates the test oracle).
    for i in 0..256 {
        assert_eq!(ruse_hint(hs[i], rs[i]), rhighbits(rs[i] + zs[i]), "property holds for input {i}");
    }
    let src = mldsa_rounding_program(&rs, &hs);
    for (label, out) in [("tw", tw_outcome(&src)), ("vm", vm_outcome(&src))] {
        assert!(out.error.is_none(), "{label} mldsa rounding errored: {:?}", out.error);
        assert_eq!(
            out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
            want,
            "{label}: Logos HighBits/UseHint == FIPS-204 spec"
        );
    }
}


/// `mldsaHighBits`/`mldsaUseHint` resolved from crypto.lg (auto-import) — the shipped verify glue.
#[test]
fn mldsa_rounding_via_cryptolg_matches_spec_tw_and_vm() {
    let rs: Vec<i32> = (0..256).map(|i| ((i as i64 * 32749 + 17) % MLDSA_Q as i64) as i32).collect();
    let zs: Vec<i32> = (0..256).map(|i| ((i as i32 * 4099) % (2 * MLDSA_GAMMA2 + 1)) - MLDSA_GAMMA2).collect();
    let hs: Vec<i32> = (0..256).map(|i| rmake_hint(zs[i], rs[i])).collect();
    let want: Vec<String> = rs.iter().map(|&r| rhighbits(r).to_string())
        .chain((0..256).map(|i| ruse_hint(hs[i], rs[i]).to_string())).collect();
    // No inline defs — mldsaHighBits/mldsaUseHint come from crypto.lg.
    let mut p = String::from("## Main\nLet mutable rs be a new Seq of Int.\n");
    for &r in &rs { p.push_str(&format!("Push {} to rs.\n", r)); }
    p.push_str("Let mutable hs be a new Seq of Int.\n");
    for &h in &hs { p.push_str(&format!("Push {} to hs.\n", h)); }
    p.push_str("Repeat for i from 1 to 256:\n    Show mldsaHighBits(item i of rs).\n");
    p.push_str("Repeat for i from 1 to 256:\n    Show mldsaUseHint(item i of hs, item i of rs).\n");
    for (label, out) in [("tw", tw_outcome(&p)), ("vm", vm_outcome(&p))] {
        assert!(out.error.is_none(), "{label} crypto.lg rounding errored: {:?}", out.error);
        assert_eq!(out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(), want,
            "{label}: crypto.lg HighBits/UseHint == FIPS-204 spec");
    }
}

/// FIPS-204 `w1Encode` for ML-DSA-65 (γ2 = (q−1)/32 ⇒ w1 ∈ [0,16), 4 bits): SimpleBitPack, two
/// coefficients per byte. `byte[j] = w1[2j] | (w1[2j+1] << 4)`.
fn rw1_encode(w1: &[i32]) -> Vec<i32> {
    (0..w1.len() / 2).map(|j| w1[2 * j] | (w1[2 * j + 1] << 4)).collect()
}

#[test]
fn mldsa_w1encode_logos_matches_spec_tw_and_vm() {
    // w1 coefficients in [0,16) — the HighBits output range.
    let w1: Vec<i32> = (0..256).map(|i| (i as i32 * 7 + 3) % 16).collect();
    let want: Vec<String> = rw1_encode(&w1).iter().map(|&b| b.to_string()).collect();
    let mut p = String::new();
    p.push_str("## To mldsaW1Encode (w1: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let mutable out be a new Seq of Int.\n    Let n be (length of w1) / 2.\n");
    p.push_str("    Repeat for j from 0 to n - 1:\n");
    p.push_str("        Let lo be item (2 * j + 1) of w1.\n        Let hi be item (2 * j + 2) of w1.\n");
    p.push_str("        Push (lo + hi * 16) to out.\n    Return out.\n\n");
    p.push_str("## Main\nLet mutable w be a new Seq of Int.\n");
    for &c in &w1 {
        p.push_str(&format!("Push {} to w.\n", c));
    }
    p.push_str("Repeat for x in mldsaW1Encode(w):\n    Show x.\n");
    for (label, out) in [("tw", tw_outcome(&p)), ("vm", vm_outcome(&p))] {
        assert!(out.error.is_none(), "{label} w1encode errored: {:?}", out.error);
        assert_eq!(out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(), want,
            "{label}: Logos w1Encode == FIPS-204 spec");
    }
}

// ───────────────────────── ML-DSA verify glue: t1 decode + SampleInBall placement ─────────────────────────

/// FIPS-204 SimpleBitUnpack(10) for `t1` — 4 coefficients per 5 bytes. Matches native `unpack_t1`.
fn r_unpack_t1(b: &[i32]) -> Vec<i32> {
    let mut r = Vec::with_capacity(256);
    for i in 0..64 {
        let a: Vec<i64> = (0..5).map(|k| b[5 * i + k] as i64).collect();
        r.push(((a[0] | (a[1] << 8)) & 0x3ff) as i32);
        r.push((((a[1] >> 2) | (a[2] << 6)) & 0x3ff) as i32);
        r.push((((a[2] >> 4) | (a[3] << 4)) & 0x3ff) as i32);
        r.push((((a[3] >> 6) | (a[4] << 2)) & 0x3ff) as i32);
    }
    r
}

#[test]
fn mldsa_unpack_t1_logos_matches_spec_tw_and_vm() {
    let bytes: Vec<i32> = (0..320).map(|i| (i * 37 + 11) % 256).collect(); // 320 bytes/poly
    let want: Vec<String> = r_unpack_t1(&bytes).iter().map(|&c| c.to_string()).collect();
    let mut p = String::new();
    p.push_str("## To mldsaUnpackT1 (b: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let mutable r be a new Seq of Int.\n    Repeat for i from 0 to 63:\n");
    p.push_str("        Let a0 be item (5 * i + 1) of b.\n        Let a1 be item (5 * i + 2) of b.\n");
    p.push_str("        Let a2 be item (5 * i + 3) of b.\n        Let a3 be item (5 * i + 4) of b.\n        Let a4 be item (5 * i + 5) of b.\n");
    p.push_str("        Push ((a0 + a1 * 256) % 1024) to r.\n");
    p.push_str("        Push ((a1 / 4 + a2 * 64) % 1024) to r.\n");
    p.push_str("        Push ((a2 / 16 + a3 * 16) % 1024) to r.\n");
    p.push_str("        Push ((a3 / 64 + a4 * 4) % 1024) to r.\n    Return r.\n\n");
    p.push_str("## Main\nLet mutable b be a new Seq of Int.\n");
    for &x in &bytes {
        p.push_str(&format!("Push {} to b.\n", x));
    }
    p.push_str("Repeat for x in mldsaUnpackT1(b):\n    Show x.\n");
    for (label, out) in [("tw", tw_outcome(&p)), ("vm", vm_outcome(&p))] {
        assert!(out.error.is_none(), "{label} unpackT1 errored: {:?}", out.error);
        assert_eq!(out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(), want,
            "{label}: Logos unpack_t1 == FIPS-204 SimpleBitUnpack(10)");
    }
}

/// The SampleInBall placement (FIPS-204 §7.3) over a fixed XOF buffer — the Logos logic, sans the
/// SHAKE-256 squeeze (which the full primitive gets from native `shake256`). Matches `sample_in_ball`.
fn r_sample_in_ball_buf(buf: &[i32]) -> [i32; 256] {
    let mut signs = 0u64;
    for k in 0..8 {
        signs |= (buf[k] as u64) << (8 * k);
    }
    let mut pos = 8;
    let mut c = [0i32; 256];
    for sign_idx in 0..49 {
        let i = 256 - 49 + sign_idx;
        let j = loop {
            let candidate = buf[pos] as usize;
            pos += 1;
            if candidate <= i {
                break candidate;
            }
        };
        c[i] = c[j];
        c[j] = 1 - 2 * (((signs >> sign_idx) & 1) as i32);
    }
    c
}

#[test]
fn mldsa_sample_in_ball_placement_logos_matches_spec_tw_and_vm() {
    // A deterministic 512-byte XOF buffer (more than the ~60 the 49 placements consume).
    let mut s = 0xC0FFEEu64;
    let buf: Vec<i32> = (0..512)
        .map(|_| {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((s >> 40) & 0xff) as i32
        })
        .collect();
    // c ∈ {−1, 0, 1} as signed Int (the placement returns 1 − 2·bit).
    let want: Vec<String> = r_sample_in_ball_buf(&buf).iter().map(|&c| c.to_string()).collect();
    let mut p = String::new();
    p.push_str("## To mldsaSampleInBallBuf (buf: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let mutable c be a new Seq of Int.\n    Repeat for k from 1 to 256:\n        Push 0 to c.\n");
    p.push_str("    Let mutable signBits be 0.\n    Let mutable mult be 1.\n");
    p.push_str("    Repeat for k from 1 to 7:\n        Set signBits to signBits + (item k of buf) * mult.\n        Set mult to mult * 256.\n");
    p.push_str("    Let mutable pos be 9.\n");
    p.push_str("    Repeat for i from 207 to 255:\n");
    p.push_str("        Let mutable j be 0.\n        Let mutable found be 0.\n");
    p.push_str("        While found is equal to 0:\n");
    p.push_str("            Let candidate be item pos of buf.\n            Set pos to pos + 1.\n");
    p.push_str("            If candidate is at most i:\n                Set j to candidate.\n                Set found to 1.\n");
    p.push_str("        Set item (i + 1) of c to (item (j + 1) of c).\n");
    p.push_str("        Let bit be signBits % 2.\n        Set signBits to signBits / 2.\n");
    p.push_str("        Set item (j + 1) of c to (1 - 2 * bit).\n");
    p.push_str("    Return c.\n\n");
    p.push_str("## Main\nLet mutable buf be a new Seq of Int.\n");
    for &x in &buf {
        p.push_str(&format!("Push {} to buf.\n", x));
    }
    p.push_str("Repeat for x in mldsaSampleInBallBuf(buf):\n    Show x.\n");
    for (label, out) in [("tw", tw_outcome(&p)), ("vm", vm_outcome(&p))] {
        assert!(out.error.is_none(), "{label} sampleInBall errored: {:?}", out.error);
        let got: Vec<&str> = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        // c[j] = ±1 stored as i32; −1 prints as its u32 bits (4294967295).
        let got_norm: Vec<String> = got.iter().map(|l| l.to_string()).collect();
        assert_eq!(got_norm, want, "{label}: Logos SampleInBall placement == FIPS-204 spec");
    }
}

// ───────────────────────── ML-DSA verify glue: z decode + ExpandA (RejNTTPoly) ─────────────────────────

/// FIPS-204 BitUnpack(γ1) for the signature's `z` — 20-bit, 2 coeffs per 5 bytes, `r = γ1 − z`.
/// Matches native `unpack_z` (γ1 = 2¹⁹). Coefficients are signed (in (−γ1, γ1]).
fn r_unpack_z(b: &[i32]) -> Vec<i32> {
    const GAMMA1: i64 = 1 << 19;
    let mut r = Vec::with_capacity(256);
    for i in 0..128 {
        let a: Vec<i64> = (0..5).map(|k| b[5 * i + k] as i64).collect();
        let z0 = a[0] + a[1] * 256 + (a[2] % 16) * 65536;
        let z1 = a[2] / 16 + a[3] * 16 + a[4] * 4096;
        r.push((GAMMA1 - z0) as i32);
        r.push((GAMMA1 - z1) as i32);
    }
    r
}

/// ExpandA element (RejNTTPoly) placement over a fixed SHAKE-128 buffer, sans the squeeze: read
/// 3 bytes → 23-bit `d`, accept if `d < q`. Matches native `rej_ntt_poly`.
fn r_rej_ntt_poly_buf(buf: &[i32]) -> [i32; 256] {
    let q = 8_380_417i64;
    let mut a = [0i32; 256];
    let (mut pos, mut ctr) = (0usize, 0usize);
    while ctr < 256 {
        let d = buf[pos] as i64 + buf[pos + 1] as i64 * 256 + (buf[pos + 2] as i64 % 128) * 65536;
        pos += 3;
        if d < q {
            a[ctr] = d as i32;
            ctr += 1;
        }
    }
    a
}

#[test]
fn mldsa_unpack_z_logos_matches_spec_tw_and_vm() {
    let bytes: Vec<i32> = (0..640).map(|i| (i * 53 + 7) % 256).collect(); // 640 bytes/poly
    let want: Vec<String> = r_unpack_z(&bytes).iter().map(|&c| c.to_string()).collect();
    let mut p = String::new();
    p.push_str("## To mldsaUnpackZ (b: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let mutable r be a new Seq of Int.\n    Repeat for i from 0 to 127:\n");
    p.push_str("        Let a0 be item (5 * i + 1) of b.\n        Let a1 be item (5 * i + 2) of b.\n");
    p.push_str("        Let a2 be item (5 * i + 3) of b.\n        Let a3 be item (5 * i + 4) of b.\n        Let a4 be item (5 * i + 5) of b.\n");
    p.push_str("        Let z0 be a0 + a1 * 256 + (a2 % 16) * 65536.\n");
    p.push_str("        Let z1 be a2 / 16 + a3 * 16 + a4 * 4096.\n");
    p.push_str("        Push (524288 - z0) to r.\n        Push (524288 - z1) to r.\n    Return r.\n\n");
    p.push_str("## Main\nLet mutable b be a new Seq of Int.\n");
    for &x in &bytes {
        p.push_str(&format!("Push {} to b.\n", x));
    }
    p.push_str("Repeat for x in mldsaUnpackZ(b):\n    Show x.\n");
    for (label, out) in [("tw", tw_outcome(&p)), ("vm", vm_outcome(&p))] {
        assert!(out.error.is_none(), "{label} unpackZ errored: {:?}", out.error);
        assert_eq!(out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(), want,
            "{label}: Logos unpack_z == FIPS-204 BitUnpack(γ1)");
    }
}

#[test]
fn mldsa_rej_ntt_poly_placement_logos_matches_spec_tw_and_vm() {
    // 1024-byte SHAKE-128 buffer (≈ 256·3 + rejections; rejection rate ≈ 0.1%).
    let mut s = 0xA5A5_1234u64;
    let buf: Vec<i32> = (0..1024)
        .map(|_| {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
            ((s >> 40) & 0xff) as i32
        })
        .collect();
    let want: Vec<String> = r_rej_ntt_poly_buf(&buf).iter().map(|&c| c.to_string()).collect();
    let mut p = String::new();
    p.push_str("## To mldsaRejNttPolyBuf (buf: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let mutable a be a new Seq of Int.\n    Let mutable pos be 0.\n    Let mutable ctr be 0.\n");
    p.push_str("    While ctr is less than 256:\n");
    p.push_str("        Let b0 be item (pos + 1) of buf.\n        Let b1 be item (pos + 2) of buf.\n        Let b2 be item (pos + 3) of buf.\n");
    p.push_str("        Let d be b0 + b1 * 256 + (b2 % 128) * 65536.\n        Set pos to pos + 3.\n");
    p.push_str("        If d is less than 8380417:\n            Push d to a.\n            Set ctr to ctr + 1.\n");
    p.push_str("    Return a.\n\n");
    p.push_str("## Main\nLet mutable buf be a new Seq of Int.\n");
    for &x in &buf {
        p.push_str(&format!("Push {} to buf.\n", x));
    }
    p.push_str("Repeat for x in mldsaRejNttPolyBuf(buf):\n    Show x.\n");
    for (label, out) in [("tw", tw_outcome(&p)), ("vm", vm_outcome(&p))] {
        assert!(out.error.is_none(), "{label} rejNttPoly errored: {:?}", out.error);
        assert_eq!(out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(), want,
            "{label}: Logos RejNTTPoly placement == FIPS-204 spec");
    }
}

// ───────────────── ML-DSA verify glue: hint decode + poly subtract (the last non-SHAKE pieces) ─────────────────

/// FIPS-204 hint unpack: `OMEGA=55` index bytes + `MK=6` running-count bytes → 6 polys of {0,1}
/// (the decode common path; validity rejection is checked separately in verify). Flattened k·256.
fn r_unpack_hint(b: &[i32]) -> Vec<i32> {
    let mut h = vec![0i32; 6 * 256];
    let mut k = 0usize;
    for i in 0..6 {
        let cnt = b[55 + i] as usize;
        for j in k..cnt {
            h[i * 256 + b[j] as usize] = 1;
        }
        k = cnt;
    }
    h
}

#[test]
fn mldsa_unpack_hint_logos_matches_spec_tw_and_vm() {
    // A valid hint encoding: 8 set bits across rows 0/2/4/5, counts [2,2,4,4,6,8], padding zero.
    let mut b = vec![0i32; 61];
    let idx = [10, 50, 20, 100, 30, 200, 40, 60];
    for (j, &v) in idx.iter().enumerate() {
        b[j] = v;
    }
    let counts = [2, 2, 4, 4, 6, 8];
    for (i, &c) in counts.iter().enumerate() {
        b[55 + i] = c;
    }
    let want: Vec<String> = r_unpack_hint(&b).iter().map(|&x| x.to_string()).collect();
    let mut p = String::new();
    p.push_str("## To mldsaUnpackHint (b: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let mutable h be a new Seq of Int.\n    Repeat for x from 1 to 1536:\n        Push 0 to h.\n");
    p.push_str("    Let mutable k be 0.\n    Repeat for i from 0 to 5:\n");
    p.push_str("        Let cnt be item (55 + i + 1) of b.\n");
    p.push_str("        Let mutable j be k.\n        While j is less than cnt:\n");
    p.push_str("            Let idx be item (j + 1) of b.\n");
    p.push_str("            Set item (i * 256 + idx + 1) of h to 1.\n            Set j to j + 1.\n");
    p.push_str("        Set k to cnt.\n    Return h.\n\n");
    p.push_str("## Main\nLet mutable b be a new Seq of Int.\n");
    for &x in &b {
        p.push_str(&format!("Push {} to b.\n", x));
    }
    p.push_str("Repeat for x in mldsaUnpackHint(b):\n    Show x.\n");
    for (label, out) in [("tw", tw_outcome(&p)), ("vm", vm_outcome(&p))] {
        assert!(out.error.is_none(), "{label} unpackHint errored: {:?}", out.error);
        assert_eq!(out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(), want,
            "{label}: Logos unpack_hint == FIPS-204 spec");
    }
}

#[test]
fn mldsa_polysub_logos_matches_spec_tw_and_vm() {
    // Coefficient-wise i32 subtract over 32 Lanes8Word32 (the verify's acc −= c·t1).
    let a: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 31337 - 4_000_000) % 8_380_417) as i32);
    let b: [i32; 256] = core::array::from_fn(|i| ((i as i64 * 71993 + 1_234_567) % 8_380_417) as i32);
    let want: Vec<String> = (0..256).map(|i| (a[i].wrapping_sub(b[i]) as u32 as i64).to_string()).collect();
    let mut p = String::from(
        "## To sub8 (s: Seq of Word32) and (off: Int) -> Seq of Word32:\n    Let r be a new Seq of Word32.\n    Repeat for i from 1 to 8:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    p.push_str("## To mldsaPolySub (a: Seq of Word32) and (b: Seq of Word32) -> Seq of Word32:\n");
    p.push_str("    Let mutable out be a new Seq of Word32.\n    Repeat for v from 0 to 31:\n");
    p.push_str("        Let av be lanes8Word32(sub8(a, 8 * v)).\n        Let bv be lanes8Word32(sub8(b, 8 * v)).\n");
    p.push_str("        Repeat for x in seqOfLanes8(av - bv):\n            Push x to out.\n    Return out.\n\n");
    p.push_str("## Main\nLet mutable ca be a new Seq of Word32.\n");
    for &c in &a {
        p.push_str(&format!("Push word32({}) to ca.\n", c as u32));
    }
    p.push_str("Let mutable cb be a new Seq of Word32.\n");
    for &c in &b {
        p.push_str(&format!("Push word32({}) to cb.\n", c as u32));
    }
    p.push_str("Repeat for x in mldsaPolySub(ca, cb):\n    Show intOfWord32(x).\n");
    for (label, out) in [("tw", tw_outcome(&p)), ("vm", vm_outcome(&p))] {
        assert!(out.error.is_none(), "{label} polysub errored: {:?}", out.error);
        assert_eq!(out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(), want,
            "{label}: Logos polySub == coefficient-wise i32 subtract");
    }
}

// ───────────────── ML-DSA verify glue: SHAKE wrappers (native shake → proven placement/rejection) ─────────────────

#[test]
#[ignore = "compiles via rustc (slow) — full SampleInBall (native shake256 + Logos placement) AOT"]
fn mldsa_sample_in_ball_full_aot_eq_oracle() {
    use logicaffeine_system::keccak::shake256_bytes;
    let c_tilde: Vec<u8> = (0..48).map(|i| (i * 7 + 3) as u8).collect();
    let buf: Vec<i32> = shake256_bytes(&c_tilde, 512).iter().map(|&b| b as i32).collect();
    let want: Vec<String> = r_sample_in_ball_buf(&buf).iter().map(|&c| c.to_string()).collect();
    // Logos: shake256(c̃, 512) → mldsaSampleInBallBuf — both resolved from crypto.lg.
    let mut p = String::from("## Main\nLet mutable ct be a new Seq of Int.\n");
    for &b in &c_tilde {
        p.push_str(&format!("Push {} to ct.\n", b));
    }
    p.push_str("Let buf be shake256(ct, 512).\n");
    p.push_str("Let c be mldsaSampleInBallBuf(buf).\n");
    p.push_str("Repeat for x in c:\n    Show x.\n");
    let aot = run_logos_with_args(&p, &[]);
    assert!(aot.success, "AOT failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "full SampleInBall (shake256 + placement) == oracle"
    );
}

#[test]
#[ignore = "compiles via rustc (slow) — ExpandA element (native shake128 + Logos rejection) AOT"]
fn mldsa_expand_a_elem_aot_eq_oracle() {
    use logicaffeine_system::keccak::shake128_bytes;
    // seed = ρ ‖ col ‖ row (34 bytes).
    let mut seed: Vec<u8> = (0..32).map(|i| (i * 5 + 1) as u8).collect();
    seed.push(2); // col
    seed.push(4); // row
    let buf: Vec<i32> = shake128_bytes(&seed, 1024).iter().map(|&b| b as i32).collect();
    let want: Vec<String> = r_rej_ntt_poly_buf(&buf).iter().map(|&c| c.to_string()).collect();
    let mut p = String::from("## Main\nLet mutable sd be a new Seq of Int.\n");
    for &b in &seed {
        p.push_str(&format!("Push {} to sd.\n", b));
    }
    p.push_str("Let buf be shake128(sd, 1024).\n");
    p.push_str("Let a be mldsaRejNttPolyBuf(buf).\n");
    p.push_str("Repeat for x in a:\n    Show x.\n");
    let aot = run_logos_with_args(&p, &[]);
    assert!(aot.success, "AOT failed:\n{}", aot.stderr);
    assert_eq!(
        aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(),
        want,
        "ExpandA element (shake128 + rejection) == oracle"
    );
}


// ───────────────────────── ML-DSA verify ASSEMBLY (the full Logos mldsaVerify) ─────────────────────────

/// The complete Logos ML-DSA-65 verify, wiring every proven crypto.lg primitive. Checks the c̃
/// equality (FIPS-204 Algorithm 3 core); the ‖z‖∞ bound and hint-validity rejections are public-data
/// pre-checks a valid signature always passes, omitted here. Tests AOT == native `verify`.
fn mldsa_verify_defs() -> String {
    let mut p = String::new();
    p.push_str("## To mldsaSubRange (s: Seq of Int) and (off: Int) and (len: Int) -> Seq of Int:\n");
    p.push_str("    Let r be a new Seq of Int.\n    Repeat for i from 1 to len:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    p.push_str("## To mldsaSubRangeW (s: Seq of Word32) and (off: Int) and (len: Int) -> Seq of Word32:\n");
    p.push_str("    Let r be a new Seq of Word32.\n    Repeat for i from 1 to len:\n        Push (item (off + i) of s) to r.\n    Return r.\n\n");
    p.push_str("## To mldsaToS32 (w: Word32) -> Int:\n    Let v be intOfWord32(w).\n    If v is at least 2147483648:\n        Return v - 4294967296.\n    Return v.\n\n");
    p.push_str("## To mldsaIntsToW32 (p: Seq of Int) -> Seq of Word32:\n    Let mutable out be a new Seq of Word32.\n    Repeat for x in p:\n        Push word32(x) to out.\n    Return out.\n\n");
    p.push_str("## To mldsaScaleT1 (p: Seq of Int) -> Seq of Word32:\n    Let mutable out be a new Seq of Word32.\n    Repeat for x in p:\n        Push word32(x * 8192) to out.\n    Return out.\n\n");
    p.push_str("## To mldsaPolyAdd (a: Seq of Word32) and (b: Seq of Word32) -> Seq of Word32:\n");
    p.push_str("    Let mutable out be a new Seq of Word32.\n    Repeat for v from 0 to 31:\n");
    p.push_str("        Let av be lanes8Word32(sub8(a, 8 * v)).\n        Let bv be lanes8Word32(sub8(b, 8 * v)).\n");
    p.push_str("        Repeat for x in seqOfLanes8(av + bv):\n            Push x to out.\n    Return out.\n\n");
    p.push_str("## To mldsaVerifyLogos (pk: Seq of Int) and (msg: Seq of Int) and (sig: Seq of Int) -> Int:\n");
    p.push_str("    Let rho be mldsaSubRange(pk, 0, 32).\n");
    p.push_str("    Let tr be shake256(pk, 64).\n");
    p.push_str("    Let mutable muIn be a new Seq of Int.\n    Repeat for x in tr:\n        Push x to muIn.\n    Push 0 to muIn.\n    Push 0 to muIn.\n    Repeat for x in msg:\n        Push x to muIn.\n");
    p.push_str("    Let mu be shake256(muIn, 64).\n");
    p.push_str("    Let cTilde be mldsaSubRange(sig, 0, 48).\n");
    p.push_str("    Let cHat be mldsaNtt(mldsaIntsToW32(mldsaSampleInBall(cTilde))).\n");
    p.push_str("    Let mutable zHatFlat be a new Seq of Word32.\n    Repeat for j from 0 to 4:\n");
    p.push_str("        Let zj be mldsaUnpackZ(mldsaSubRange(sig, 48 + 640 * j, 640)).\n");
    p.push_str("        Repeat for x in mldsaNtt(mldsaIntsToW32(zj)):\n            Push x to zHatFlat.\n");
    p.push_str("    Let hAll be mldsaUnpackHint(mldsaSubRange(sig, 3248, 61)).\n");
    p.push_str("    Let mutable ctIn be a new Seq of Int.\n    Repeat for x in mu:\n        Push x to ctIn.\n");
    p.push_str("    Repeat for i from 0 to 5:\n");
    p.push_str("        Let mutable acc be a new Seq of Word32.\n        Repeat for k from 1 to 256:\n            Push word32(0) to acc.\n");
    p.push_str("        Repeat for j from 0 to 4:\n");
    p.push_str("            Let mutable seed be a new Seq of Int.\n            Repeat for x in rho:\n                Push x to seed.\n            Push j to seed.\n            Push i to seed.\n");
    p.push_str("            Let aHatIJ be mldsaExpandAElem(seed).\n");
    p.push_str("            Let zHatJ be mldsaSubRangeW(zHatFlat, 256 * j, 256).\n");
    p.push_str("            Set acc to mldsaPolyAdd(acc, pointwiseMontgomery(mldsaIntsToW32(aHatIJ), zHatJ)).\n");
    p.push_str("        Let t1i be mldsaUnpackT1(mldsaSubRange(pk, 32 + 320 * i, 320)).\n");
    p.push_str("        Let t1dHat be mldsaNtt(mldsaScaleT1(t1i)).\n");
    p.push_str("        Set acc to mldsaPolySub(acc, pointwiseMontgomery(cHat, t1dHat)).\n");
    p.push_str("        Let wp be mldsaInvNttToMont(acc).\n");
    p.push_str("        Let mutable w1 be a new Seq of Int.\n        Repeat for c from 0 to 255:\n");
    p.push_str("            Let hc be item (i * 256 + c + 1) of hAll.\n            Let wc be mldsaToS32(item (c + 1) of wp).\n");
    p.push_str("            Push mldsaUseHint(hc, wc) to w1.\n");
    p.push_str("        Repeat for x in mldsaW1Encode(w1):\n            Push x to ctIn.\n");
    p.push_str("    Let cTildeP be shake256(ctIn, 48).\n");
    p.push_str("    Let mutable eq be 1.\n    Repeat for k from 1 to 48:\n");
    p.push_str("        Let diff be (item k of cTilde) - (item k of cTildeP).\n");
    p.push_str("        If diff is greater than 0:\n            Set eq to 0.\n        If diff is less than 0:\n            Set eq to 0.\n");
    p.push_str("    Return eq.\n\n");
    p
}

/// The verify Main only — the def block lives in crypto.lg (resolved by auto-import).
fn mldsa_verify_program(pk: &[i64], msg: &[i64], sig: &[i64]) -> String {
    let mut p = String::new();
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    p.push_str("## Main\n");
    p.push_str(&format!("Let pk be [{}].\n", lit(pk)));
    p.push_str(&format!("Let msg be [{}].\n", lit(msg)));
    p.push_str(&format!("Let sig be [{}].\n", lit(sig)));
    p.push_str("Show mldsaVerifyLogos(pk, msg, sig).\n");
    p
}

#[test]
#[ignore = "compiles via rustc (slow) — full Logos ML-DSA verify AOT == native verify"]
fn mldsa_verify_logos_aot_eq_native() {
    use logicaffeine_system::mldsa::{mldsa_keypair_seq, mldsa_sign_seq, mldsa_verify_seq};
    let seed: Vec<i64> = (0..32).map(|i| (i * 7 + 1) as i64).collect();
    let kp = mldsa_keypair_seq(&seed).to_vec();
    let pk = kp[0..1952].to_vec();
    let sk = kp[1952..].to_vec();
    let msg: Vec<i64> = (0..16).map(|i| (i * 13 + 3) as i64).collect();
    let sig = mldsa_sign_seq(&sk, &msg, &[]).to_vec();
    assert_eq!(mldsa_verify_seq(&pk, &msg, &[], &sig), 1, "native accepts valid");

    let aot = run_logos_with_args(&mldsa_verify_program(&pk, &msg, &sig), &[]);
    assert!(aot.success, "AOT failed:\n{}", aot.stderr);
    let got = aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
    assert_eq!(got, vec!["1"], "Logos verify accepts the valid signature (c̃ matches)");

    let reject = |bad: &[i64], why: &str| {
        assert_eq!(mldsa_verify_seq(&pk, &msg, &[], bad), 0, "native rejects: {why}");
        let a = run_logos_with_args(&mldsa_verify_program(&pk, &msg, bad), &[]);
        assert!(a.success, "AOT({why}) failed:\n{}", a.stderr);
        let g = a.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
        assert_eq!(g, vec!["0"], "Logos verify rejects: {why}");
    };
    // c̃ mismatch.
    let mut bad = sig.clone();
    bad[0] ^= 0xff;
    reject(&bad, "tampered c̃");
    // Malformed hint: count byte > ω=55 (the FIPS-204 hint-validity pre-check).
    let mut bad_h = sig.clone();
    bad_h[3303] = 200; // sig[3248 + 55] = first running-count byte
    reject(&bad_h, "hint count > ω");
    // Out-of-bound z: first poly's first coefficient pushed to γ1 (the ‖z‖∞ < γ1−β pre-check).
    let mut bad_z = sig.clone();
    bad_z[48] = 0;
    bad_z[49] = 0;
    bad_z[50] = 0; // z0 = 0 ⇒ z[0] = γ1 = 524288 ≥ γ1−β
    reject(&bad_z, "z out of bound");
}


/// FIPS-204 hint validity (native `unpack_hint` → Some/None): counts non-decreasing & ≤ ω, indices
/// strictly increasing within a poly, padding zero.
fn r_hint_valid(b: &[i32]) -> i32 {
    let mut k = 0usize;
    for i in 0..6 {
        let cnt = b[55 + i] as usize;
        if cnt < k || cnt > 55 {
            return 0;
        }
        for j in k..cnt {
            if j > k && b[j] <= b[j - 1] {
                return 0;
            }
        }
        k = cnt;
    }
    for &x in &b[k..55] {
        if x != 0 {
            return 0;
        }
    }
    1
}

#[test]
fn mldsa_hint_valid_logos_matches_spec_tw_and_vm() {
    let mk = |idx: &[i32], counts: &[i32]| -> Vec<i32> {
        let mut b = vec![0i32; 61];
        for (j, &v) in idx.iter().enumerate() {
            b[j] = v;
        }
        for (i, &c) in counts.iter().enumerate() {
            b[55 + i] = c;
        }
        b
    };
    let cases = vec![
        mk(&[10, 50, 20, 100, 30, 200, 40, 60], &[2, 2, 4, 4, 6, 8]), // valid
        mk(&[10, 50], &[60, 60, 60, 60, 60, 60]),                     // cnt > 55
        mk(&[10, 50, 20], &[4, 2, 4, 4, 6, 8]),                       // counts not non-decreasing
        mk(&[50, 10], &[2, 2, 4, 4, 6, 8]),                           // indices not strictly increasing
        {
            let mut b = mk(&[10, 50], &[2, 2, 2, 2, 2, 2]);
            b[40] = 7; // non-zero padding (index 40 ∈ [2,55))
            b
        },
    ];
    for (n, b) in cases.iter().enumerate() {
        let want = r_hint_valid(b).to_string();
        let mut p = String::from("## Main\nLet mutable b be a new Seq of Int.\n");
        for &x in b {
            p.push_str(&format!("Push {} to b.\n", x));
        }
        p.push_str("Show mldsaHintValid(b, 0).\n");
        for (label, out) in [("tw", tw_outcome(&p)), ("vm", vm_outcome(&p))] {
            assert!(out.error.is_none(), "{label} hintValid[{n}] errored: {:?}", out.error);
            let got = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
            assert_eq!(got, vec![want.as_str()], "{label}: mldsaHintValid case {n} == spec");
        }
    }
}

// ───────────────────────── ML-DSA KEYGEN building blocks (ExpandS, Power2Round→t1, pack_t1) ─────────────────────────

/// RejBoundedPoly / ExpandS placement (η=4): each nibble < 9 → η − nibble. Matches `rej_bounded_poly`.
fn r_rej_bounded_buf(buf: &[i32]) -> [i32; 256] {
    let mut a = [0i32; 256];
    let (mut pos, mut ctr) = (0usize, 0usize);
    while ctr < 256 {
        let b = buf[pos];
        pos += 1;
        let (lo, hi) = (b & 15, b >> 4);
        if lo < 9 {
            a[ctr] = 4 - lo;
            ctr += 1;
        }
        if hi < 9 && ctr < 256 {
            a[ctr] = 4 - hi;
            ctr += 1;
        }
    }
    a
}
/// Power2Round high half (D=13): `r1 = (freeze(t) + 2¹²−1) >> 13 ∈ [0,1024)`.
fn r_power2round_r1(t: i32) -> i32 {
    let r = ((t % 8_380_417) + 8_380_417) % 8_380_417;
    (r + 4095) >> 13
}
/// SimpleBitPack(10) for t1 — 4 coeffs → 5 bytes. Matches `pack_t1`.
fn r_pack_t1(t1: &[i32]) -> Vec<i32> {
    let mut r = Vec::new();
    for i in 0..64 {
        let a = [t1[4 * i], t1[4 * i + 1], t1[4 * i + 2], t1[4 * i + 3]];
        r.push(a[0] & 0xff);
        r.push(((a[0] >> 8) | (a[1] << 2)) & 0xff);
        r.push(((a[1] >> 6) | (a[2] << 4)) & 0xff);
        r.push(((a[2] >> 4) | (a[3] << 6)) & 0xff);
        r.push((a[3] >> 2) & 0xff);
    }
    r
}

#[test]
fn mldsa_keygen_blocks_logos_match_spec_tw_and_vm() {
    // RejBounded placement.
    let mut s = 0xBEEF_1234u64;
    let buf: Vec<i32> = (0..512).map(|_| { s = s.wrapping_mul(6364136223846793005).wrapping_add(1); ((s >> 40) & 0xff) as i32 }).collect();
    let rb_want: Vec<String> = r_rej_bounded_buf(&buf).iter().map(|&c| c.to_string()).collect();
    let mut p = String::from("## To mldsaRejBoundedBuf (buf: Seq of Int) -> Seq of Int:\n    Let mutable a be a new Seq of Int.\n    Let mutable pos be 0.\n    Let mutable ctr be 0.\n");
    p.push_str("    While ctr is less than 256:\n        Let b be item (pos + 1) of buf.\n        Set pos to pos + 1.\n        Let lo be b % 16.\n        Let hi be b / 16.\n");
    p.push_str("        If lo is less than 9:\n            Push (4 - lo) to a.\n            Set ctr to ctr + 1.\n");
    p.push_str("        If hi is less than 9:\n            If ctr is less than 256:\n                Push (4 - hi) to a.\n                Set ctr to ctr + 1.\n    Return a.\n\n");
    // Power2Round→t1.
    let ts: Vec<i32> = (0..256).map(|i| ((i as i64 * 51197) % 8_380_417) as i32).collect();
    let p2r_want: Vec<String> = ts.iter().map(|&t| r_power2round_r1(t).to_string()).collect();
    p.push_str("## To mldsaPower2RoundT1 (t: Int) -> Int:\n    Let r be ((t % 8380417) + 8380417) % 8380417.\n    Return (r + 4095) / 8192.\n\n");
    // pack_t1.
    let t1: Vec<i32> = (0..256).map(|i| (i as i32 * 13 + 7) % 1024).collect();
    let pk_want: Vec<String> = r_pack_t1(&t1).iter().map(|&b| b.to_string()).collect();
    p.push_str("## To mldsaPackT1 (t1: Seq of Int) -> Seq of Int:\n    Let mutable r be a new Seq of Int.\n    Repeat for i from 0 to 63:\n");
    p.push_str("        Let a0 be item (4 * i + 1) of t1.\n        Let a1 be item (4 * i + 2) of t1.\n        Let a2 be item (4 * i + 3) of t1.\n        Let a3 be item (4 * i + 4) of t1.\n");
    p.push_str("        Push (a0 % 256) to r.\n        Push ((a0 / 256 + a1 * 4) % 256) to r.\n        Push ((a1 / 64 + a2 * 16) % 256) to r.\n        Push ((a2 / 16 + a3 * 64) % 256) to r.\n        Push (a3 / 4) to r.\n    Return r.\n\n");
    p.push_str("## Main\n");
    p.push_str(&format!("Let buf be [{}].\n", buf.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ")));
    p.push_str("Repeat for x in mldsaRejBoundedBuf(buf):\n    Show x.\n");
    p.push_str(&format!("Let ts be [{}].\n", ts.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ")));
    p.push_str("Repeat for t in ts:\n    Show mldsaPower2RoundT1(t).\n");
    p.push_str(&format!("Let t1 be [{}].\n", t1.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ")));
    p.push_str("Repeat for x in mldsaPackT1(t1):\n    Show x.\n");
    let want: Vec<String> = rb_want.into_iter().chain(p2r_want).chain(pk_want).collect();
    for (label, out) in [("tw", tw_outcome(&p)), ("vm", vm_outcome(&p))] {
        assert!(out.error.is_none(), "{label} keygen blocks errored: {:?}", out.error);
        assert_eq!(out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>(), want,
            "{label}: RejBounded + Power2Round→t1 + pack_t1 == FIPS spec");
    }
}

/// The full Logos ML-DSA-65 keygen public key: pk = ρ ‖ pack_t1(t1), t1 = Power2Round(A·s1 + s2).hi.
/// (sk packing omitted — the pk is the externally-checkable half.) Tests AOT == native keygen pk.
fn mldsa_keygen_program(seed: &[i64]) -> String {
    let mut p = String::new();
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    p.push_str("## Main\n");
    p.push_str(&format!("Let seed be [{}].\n", lit(seed)));
    p.push_str("Repeat for x in mldsaKeygenLogos(seed):\n    Show x.\n");
    p
}

#[test]
#[ignore = "compiles via rustc (slow) — full Logos ML-DSA keygen pk AOT == native keypair pk"]
fn mldsa_keygen_logos_aot_eq_native() {
    use logicaffeine_system::mldsa::mldsa_keypair_seq;
    let seed: Vec<i64> = (0..32).map(|i| (i * 7 + 1) as i64).collect();
    let kp = mldsa_keypair_seq(&seed).to_vec();
    let want: Vec<String> = kp[0..1952].iter().map(|b| b.to_string()).collect();
    let aot = run_logos_with_args(&mldsa_keygen_program(&seed), &[]);
    assert!(aot.success, "AOT failed:\n{}", aot.stderr);
    let got = aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
    assert_eq!(got.len(), 1952, "pk is 1952 bytes");
    assert_eq!(got, want, "Logos keygen pk == native keypair pk");
}

// ───────────────────────── ML-DSA SIGN building blocks (sk decode + pack + rounding) ─────────────────────────
// New blocks needed for FIPS-204 Algorithm 2 (deterministic rnd=0 sign), each oracle-checked
// against a replica of the native logicaffeine_system::mldsa reference.

const SGN_Q: i64 = 8380417;
const SGN_GAMMA1: i64 = 524288;

fn r_highbits_dsa(r: i64) -> i64 {
    let rr = ((r % SGN_Q) + SGN_Q) % SGN_Q;
    let mut a1 = (rr + 127) / 128;
    a1 = (a1 * 1025 + 2097152) / 4194304;
    a1 % 16
}
fn r_lowbits_dsa(r: i64) -> i64 {
    let rr = ((r % SGN_Q) + SGN_Q) % SGN_Q;
    let mut a1 = (rr + 127) / 128;
    a1 = (a1 * 1025 + 2097152) / 4194304;
    a1 %= 16;
    let mut a0 = rr - a1 * 523776;
    if a0 > 4190208 {
        a0 -= SGN_Q;
    }
    a0
}
fn r_make_hint_dsa(z: i64, r: i64) -> i64 {
    if r_highbits_dsa(r) == r_highbits_dsa(r + z) { 0 } else { 1 }
}
fn r_unpack_eta(b: &[i64]) -> Vec<i64> {
    (0..256).map(|i| {
        let byte = b[i / 2];
        let nib = if i % 2 == 0 { byte % 16 } else { byte / 16 };
        4 - nib
    }).collect()
}
fn r_unpack_t0(b: &[i64]) -> Vec<i64> {
    let mut r = Vec::with_capacity(256);
    for i in 0..32 {
        let c: Vec<i64> = (0..13).map(|k| b[13 * i + k]).collect();
        let t = [
            (c[0] + c[1] * 256) % 8192,
            (c[1] / 32 + c[2] * 8 + c[3] * 2048) % 8192,
            (c[3] / 4 + c[4] * 64) % 8192,
            (c[4] / 128 + c[5] * 2 + c[6] * 512) % 8192,
            (c[6] / 16 + c[7] * 16 + c[8] * 4096) % 8192,
            (c[8] / 2 + c[9] * 128) % 8192,
            (c[9] / 64 + c[10] * 4 + c[11] * 1024) % 8192,
            (c[11] / 8 + c[12] * 32) % 8192,
        ];
        for &tj in &t {
            r.push(4096 - tj);
        }
    }
    r
}
fn r_pack_z(p: &[i64]) -> Vec<i64> {
    let mut r = Vec::with_capacity(320);
    for i in 0..128 {
        let t0 = SGN_GAMMA1 - p[2 * i];
        let t1 = SGN_GAMMA1 - p[2 * i + 1];
        r.push(t0 % 256);
        r.push((t0 / 256) % 256);
        r.push((t0 / 65536) + (t1 % 16) * 16);
        r.push((t1 / 16) % 256);
        r.push((t1 / 4096) % 256);
    }
    r
}
fn r_pack_hint(h: &[i64]) -> Vec<i64> {
    let mut r = vec![0i64; 61];
    let mut k = 0usize;
    for i in 0..6 {
        for j in 0..256 {
            if h[i * 256 + j] != 0 {
                r[k] = j as i64;
                k += 1;
            }
        }
        r[55 + i] = k as i64;
    }
    r
}

fn mldsa_sign_block_defs() -> String {
    String::new()
}

#[test]
fn mldsa_sign_blocks_logos_match_spec_tw_and_vm() {
    let mut s: u64 = 0x5151_a1b2_c3d4_e5f6;
    let mut rng = || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; s };
    let defs = mldsa_sign_block_defs();
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let check = |label: &str, prog: &str, want: &[String]| {
        for (tag, out) in [("tw", tw_outcome(prog)), ("vm", vm_outcome(prog))] {
            assert!(out.error.is_none(), "{label}/{tag} errored: {:?}", out.error);
            let got = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
            assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "{label}/{tag} == spec");
        }
    };
    // unpack_eta: 128 random bytes -> 256 coeffs
    let eta_b: Vec<i64> = (0..128).map(|_| (rng() % 256) as i64).collect();
    let want = r_unpack_eta(&eta_b).iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let prog = format!("{defs}## Main\nLet b be [{}].\nRepeat for x in mldsaUnpackEta(b):\n    Show x.\n", lit(&eta_b));
    check("unpack_eta", &prog, &want);
    // unpack_t0: 416 random bytes -> 256 coeffs
    let t0_b: Vec<i64> = (0..416).map(|_| (rng() % 256) as i64).collect();
    let want = r_unpack_t0(&t0_b).iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let prog = format!("{defs}## Main\nLet b be [{}].\nRepeat for x in mldsaUnpackT0(b):\n    Show x.\n", lit(&t0_b));
    check("unpack_t0", &prog, &want);
    // lowbits + make_hint over random r/z
    let rs: Vec<i64> = (0..40).map(|_| (rng() % SGN_Q as u64) as i64).collect();
    let want: Vec<String> = rs.iter().map(|&r| r_lowbits_dsa(r).to_string()).collect();
    let prog = format!("{defs}## Main\nLet rs be [{}].\nRepeat for r in rs:\n    Show mldsaLowBits(r).\n", lit(&rs));
    check("lowbits", &prog, &want);
    let zs: Vec<i64> = (0..40).map(|_| ((rng() % 1048576) as i64) - 524288).collect();
    let want: Vec<String> = rs.iter().zip(&zs).map(|(&r, &z)| r_make_hint_dsa(z, r).to_string()).collect();
    let prog = format!("{defs}## Main\nLet rs be [{}].\nLet zs be [{}].\nLet n be length of rs.\nRepeat for i from 1 to n:\n    Show mldsaMakeHint(item i of zs, item i of rs).\n", lit(&rs), lit(&zs));
    check("make_hint", &prog, &want);
    // pack_z: 256 signed coeffs in z-range -> 320 bytes
    let zc: Vec<i64> = (0..256).map(|_| ((rng() % 1048392) as i64) - 524092).collect();
    let want = r_pack_z(&zc).iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let prog = format!("{defs}## Main\nLet p be [{}].\nRepeat for x in mldsaPackZ(p):\n    Show x.\n", lit(&zc));
    check("pack_z", &prog, &want);
    // pack_hint: flat 1536, sparse (a few ones per poly, ascending)
    let mut h = vec![0i64; 1536];
    for i in 0..6 {
        let cnt = (rng() % 8) as usize;
        let mut used = std::collections::BTreeSet::new();
        while used.len() < cnt { used.insert((rng() % 256) as usize); }
        for j in used { h[i * 256 + j] = 1; }
    }
    let want = r_pack_hint(&h).iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let prog = format!("{defs}## Main\nLet h be [{}].\nRepeat for x in mldsaPackHint(h):\n    Show x.\n", lit(&h));
    check("pack_hint", &prog, &want);
}

/// The full Logos ML-DSA-65 deterministic signature (FIPS-204 Algorithm 2, rnd=0).
/// Mirrors logicaffeine_system::mldsa::sign exactly, reusing the crypto.lg NTT/round/pack blocks.
fn mldsa_sign_program(sk: &[i64], msg: &[i64]) -> String {
    let mut p = String::new();
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    p.push_str("## Main\n");
    p.push_str(&format!("Let sk be [{}].\n", lit(sk)));
    p.push_str(&format!("Let msg be [{}].\n", lit(msg)));
    p.push_str("Repeat for x in mldsaSignLogos(sk, msg):\n    Show x.\n");
    p
}

#[test]
#[ignore = "compiles via rustc (slow) — full Logos ML-DSA-65 deterministic sign AOT == native sign"]
fn mldsa_sign_logos_aot_eq_native() {
    use logicaffeine_system::mldsa::{mldsa_keypair_seq, mldsa_sign_seq};
    let seed: Vec<i64> = (0..32).map(|i| (i * 5 + 3) as i64).collect();
    let kp = mldsa_keypair_seq(&seed).to_vec();
    let sk: Vec<i64> = kp[1952..].to_vec(); // sk follows pk(1952) in the keypair blob
    let msg: Vec<i64> = (0..16).map(|i| (i * 11 + 1) as i64 % 256).collect();
    let want: Vec<String> = mldsa_sign_seq(&sk, &msg, &[]).to_vec().iter().map(|b| b.to_string()).collect();
    let aot = run_logos_with_args(&mldsa_sign_program(&sk, &msg), &[]);
    assert!(aot.success, "AOT failed:\n{}", aot.stderr);
    let got = aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
    assert_eq!(got.len(), 3309, "sig is 3309 bytes (got {})", got.len());
    assert_eq!(got, want, "Logos deterministic sign == native sign");
}

// ───────────────────────── ML-DSA full keypair (sk packing) blocks ─────────────────────────
fn r_p2r_t0(t: i64) -> i64 {
    let r = ((t % SGN_Q) + SGN_Q) % SGN_Q;
    let r1 = (r + 4095) / 8192;
    r - r1 * 8192
}
fn r_pack_eta(p: &[i64]) -> Vec<i64> {
    (0..128).map(|i| (4 - p[2 * i]) + (4 - p[2 * i + 1]) * 16).collect()
}
fn r_pack_t0(p: &[i64]) -> Vec<i64> {
    let mut r = Vec::with_capacity(416);
    for i in 0..32 {
        let t: Vec<i64> = (0..8).map(|j| 4096 - p[8 * i + j]).collect();
        r.push(t[0] % 256);
        r.push((t[0] / 256 + t[1] * 32) % 256);
        r.push((t[1] / 8) % 256);
        r.push((t[1] / 2048 + t[2] * 4) % 256);
        r.push((t[2] / 64 + t[3] * 128) % 256);
        r.push((t[3] / 2) % 256);
        r.push((t[3] / 512 + t[4] * 16) % 256);
        r.push((t[4] / 16) % 256);
        r.push((t[4] / 4096 + t[5] * 2) % 256);
        r.push((t[5] / 128 + t[6] * 64) % 256);
        r.push((t[6] / 4) % 256);
        r.push((t[6] / 1024 + t[7] * 8) % 256);
        r.push((t[7] / 32) % 256);
    }
    r
}

fn mldsa_keypair_block_defs() -> String {
    String::new()
}

#[test]
fn mldsa_keypair_blocks_logos_match_spec_tw_and_vm() {
    let mut s: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut rng = || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; s };
    let defs = mldsa_keypair_block_defs();
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let check = |label: &str, prog: &str, want: &[String]| {
        for (tag, out) in [("tw", tw_outcome(prog)), ("vm", vm_outcome(prog))] {
            assert!(out.error.is_none(), "{label}/{tag} errored: {:?}", out.error);
            let got = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
            assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "{label}/{tag} == spec");
        }
    };
    // power2round_t0 over random r
    let ts: Vec<i64> = (0..40).map(|_| (rng() % SGN_Q as u64) as i64).collect();
    let want: Vec<String> = ts.iter().map(|&t| r_p2r_t0(t).to_string()).collect();
    let prog = format!("{defs}## Main\nLet ts be [{}].\nRepeat for t in ts:\n    Show mldsaPower2RoundT0(t).\n", lit(&ts));
    check("power2round_t0", &prog, &want);
    // pack_eta: 256 coeffs in [-4,4]
    let pe: Vec<i64> = (0..256).map(|_| (rng() % 9) as i64 - 4).collect();
    let want = r_pack_eta(&pe).iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let prog = format!("{defs}## Main\nLet p be [{}].\nRepeat for x in mldsaPackEta(p):\n    Show x.\n", lit(&pe));
    check("pack_eta", &prog, &want);
    // pack_t0: 256 coeffs in [-4095,4096]
    let pt: Vec<i64> = (0..256).map(|_| (rng() % 8192) as i64 - 4095).collect();
    let want = r_pack_t0(&pt).iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let prog = format!("{defs}## Main\nLet p be [{}].\nRepeat for x in mldsaPackT0(p):\n    Show x.\n", lit(&pt));
    check("pack_t0", &prog, &want);
}

/// Full Logos ML-DSA-65 keypair (pk ‖ sk) bit-exact vs native, and an all-Logos round-trip:
/// Logos keypair → Logos sign → Logos verify, with NO native crypto in the signing path.
#[test]
#[ignore = "compiles via rustc (slow) — full Logos ML-DSA keypair == native + all-Logos round-trip"]
fn mldsa_keypair_logos_aot_eq_native_and_roundtrips() {
    use logicaffeine_system::mldsa::mldsa_keypair_seq;
    let seed: Vec<i64> = (0..32).map(|i| (i * 9 + 2) as i64).collect();
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    // (1) full keypair bit-exact
    let want: Vec<String> = mldsa_keypair_seq(&seed).to_vec().iter().map(|b| b.to_string()).collect();
    let kp_prog = format!(
        "## Main\nLet seed be [{}].\nRepeat for x in mldsaKeypairLogos(seed):\n    Show x.\n", lit(&seed));
    let aot = run_logos_with_args(&kp_prog, &[]);
    assert!(aot.success, "keypair AOT failed:\n{}", aot.stderr);
    let got = aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
    assert_eq!(got.len(), 5984, "keypair is pk(1952)+sk(4032)=5984 bytes (got {})", got.len());
    assert_eq!(got, want, "Logos keypair == native keypair");
    // (2) all-Logos round-trip: keygen → sign → verify, verify must return 1
    let msg: Vec<i64> = (0..24).map(|i| (i * 13 + 5) % 256).collect();
    let rt_prog = format!(
        "## Main\nLet seed be [{}].\nLet msg be [{}].\nLet kp be mldsaKeypairLogos(seed).\nShow mldsaVerifyLogos(mldsaSubRange(kp, 0, 1952), msg, mldsaSignLogos(mldsaSubRange(kp, 1952, 4032), msg)).\n",
        lit(&seed), lit(&msg));
    let rt = run_logos_with_args(&rt_prog, &[]);
    assert!(rt.success, "round-trip AOT failed:\n{}", rt.stderr);
    let rout = rt.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
    assert_eq!(rout, vec!["1"], "all-Logos keygen→sign→verify round-trip accepts");
}

/// Robustness to the point of absurdity: the all-Logos ML-DSA-65 (keypair→sign→verify, no native
/// crypto in the path) over many (seed, message) vectors — every honest signature verifies, every
/// wrong-message and wrong-key signature is rejected. One AOT program; asserts 1/1/1.
fn mldsa_robustness_program() -> String {
    let mut p = String::new();
    p.push_str("## Main\n");
    p.push_str("Let mutable acceptOk be 1.\n");
    p.push_str("Let mutable rejectMsgOk be 1.\n");
    p.push_str("Let mutable rejectKeyOk be 1.\n");
    p.push_str("Repeat for si from 0 to 2:\n");
    p.push_str("    Let mutable seedA be a new Seq of Int.\n    Repeat for k from 0 to 31:\n        Push ((si * 31 + k * 7 + 1) % 256) to seedA.\n");
    p.push_str("    Let mutable seedB be a new Seq of Int.\n    Repeat for k from 0 to 31:\n        Push ((si * 19 + k * 5 + 100) % 256) to seedB.\n");
    p.push_str("    Let kpA be mldsaKeypairLogos(seedA).\n    Let kpB be mldsaKeypairLogos(seedB).\n");
    p.push_str("    Repeat for mi from 0 to 1:\n");
    p.push_str("        Let mutable msg1 be a new Seq of Int.\n        Repeat for k from 0 to (12 + mi * 4):\n            Push ((si * 13 + mi * 17 + k * 3 + 1) % 256) to msg1.\n");
    p.push_str("        Let mutable msg2 be a new Seq of Int.\n        Repeat for x in msg1:\n            Push x to msg2.\n        Push 99 to msg2.\n");
    // accept
    p.push_str("        If mldsaVerifyLogos(mldsaSubRange(kpA, 0, 1952), msg1, mldsaSignLogos(mldsaSubRange(kpA, 1952, 4032), msg1)) is at most 0:\n            Set acceptOk to 0.\n");
    // reject wrong message
    p.push_str("        If mldsaVerifyLogos(mldsaSubRange(kpA, 0, 1952), msg1, mldsaSignLogos(mldsaSubRange(kpA, 1952, 4032), msg2)) is at least 1:\n            Set rejectMsgOk to 0.\n");
    // reject wrong key
    p.push_str("        If mldsaVerifyLogos(mldsaSubRange(kpB, 0, 1952), msg1, mldsaSignLogos(mldsaSubRange(kpA, 1952, 4032), msg1)) is at least 1:\n            Set rejectKeyOk to 0.\n");
    p.push_str("Show acceptOk.\nShow rejectMsgOk.\nShow rejectKeyOk.\n");
    p
}

#[test]
#[ignore = "compiles via rustc (slow) — all-Logos ML-DSA robustness over many vectors"]
fn mldsa_all_logos_robustness_many_vectors() {
    let aot = run_logos_with_args(&mldsa_robustness_program(), &[]);
    assert!(aot.success, "robustness AOT failed:\n{}", aot.stderr);
    let got = aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
    assert_eq!(got, vec!["1", "1", "1"],
        "accept / reject-wrong-msg / reject-wrong-key all hold across vectors (got {:?})", got);
}

// ───────────────────────── ML-KEM encoding tools in Logos (compress/decompress/byteEncode/byteDecode) ─────────────────────────
// FIPS-203 §4.2.1; bit-exact replicas of logicaffeine_system::ntt::mlkem_*_w16 (Q=3329).
const MLKEM_Q: i64 = 3329;
fn r_kem_compress(v: i64, d: u32) -> i64 { let pd = 1i64 << d; ((v * pd + MLKEM_Q / 2) / MLKEM_Q) % pd }
fn r_kem_decompress(v: i64, d: u32) -> i64 { let pd = 1i64 << d; (v * MLKEM_Q + pd / 2) / pd }
fn r_kem_byte_encode(coeffs: &[i64], d: u32) -> Vec<i64> {
    let mask = (1u64 << d) - 1; let mut out = Vec::new(); let (mut acc, mut nbits) = (0u64, 0u32);
    for &w in coeffs { acc |= (w as u64 & mask) << nbits; nbits += d;
        while nbits >= 8 { out.push((acc & 0xff) as i64); acc >>= 8; nbits -= 8; } }
    if nbits > 0 { out.push((acc & 0xff) as i64); }
    out
}
fn r_kem_byte_decode(bytes: &[i64], d: u32) -> Vec<i64> {
    let n = (bytes.len() * 8) / d as usize; let mask = (1u64 << d) - 1;
    let mut out = Vec::new(); let (mut acc, mut nbits, mut bi) = (0u64, 0u32, 0usize);
    for _ in 0..n {
        while nbits < d { acc |= (bytes[bi] as u64) << nbits; nbits += 8; bi += 1; }
        let val = (acc & mask) as i64; acc >>= d; nbits -= d;
        out.push(val % MLKEM_Q);
    }
    out
}
fn mlkem_encode_defs() -> String {
    let mut p = String::new();
    p.push_str("## To compressW16Logos (coeffs: Seq of Word16) and (d: Int) -> Seq of Word16:\n    Let mutable pd be 1.\n    Repeat for i from 1 to d:\n        Set pd to pd * 2.\n    Let mutable out be a new Seq of Word16.\n    Repeat for x in coeffs:\n        Let v be intOfWord16(x).\n        Push word16(((v * pd + 1664) / 3329) % pd) to out.\n    Return out.\n\n");
    p.push_str("## To decompressW16Logos (coeffs: Seq of Word16) and (d: Int) -> Seq of Word16:\n    Let mutable pd be 1.\n    Repeat for i from 1 to d:\n        Set pd to pd * 2.\n    Let half be pd / 2.\n    Let mutable out be a new Seq of Word16.\n    Repeat for x in coeffs:\n        Let v be intOfWord16(x).\n        Push word16((v * 3329 + half) / pd) to out.\n    Return out.\n\n");
    p.push_str("## To byteEncodeW16Logos (coeffs: Seq of Word16) and (d: Int) -> Seq of Int:\n    Let mutable pd be 1.\n    Repeat for i from 1 to d:\n        Set pd to pd * 2.\n    Let mutable out be a new Seq of Int.\n    Let mutable acc be 0.\n    Let mutable nbits be 0.\n    Repeat for x in coeffs:\n        Let v be intOfWord16(x).\n        Let mutable shift be 1.\n        Repeat for i from 1 to nbits:\n            Set shift to shift * 2.\n        Set acc to acc + (v % pd) * shift.\n        Set nbits to nbits + d.\n        While nbits is at least 8:\n            Push (acc % 256) to out.\n            Set acc to acc / 256.\n            Set nbits to nbits - 8.\n    If nbits is greater than 0:\n        Push (acc % 256) to out.\n    Return out.\n\n");
    p.push_str("## To byteDecodeW16Logos (bytes: Seq of Int) and (d: Int) -> Seq of Word16:\n    Let mutable pd be 1.\n    Repeat for i from 1 to d:\n        Set pd to pd * 2.\n    Let n be ((length of bytes) * 8) / d.\n    Let mutable out be a new Seq of Word16.\n    Let mutable acc be 0.\n    Let mutable nbits be 0.\n    Let mutable bi be 1.\n    Repeat for k from 1 to n:\n        While nbits is less than d:\n            Let mutable shift be 1.\n            Repeat for i from 1 to nbits:\n                Set shift to shift * 2.\n            Set acc to acc + (item bi of bytes) * shift.\n            Set nbits to nbits + 8.\n            Set bi to bi + 1.\n        Let val be acc % pd.\n        Set acc to acc / pd.\n        Set nbits to nbits - d.\n        Push word16(val % 3329) to out.\n    Return out.\n\n");
    p
}

#[test]
fn mlkem_encode_tools_logos_match_spec_tw_and_vm() {
    let mut s: u64 = 0xdead_beef_1234_5678;
    let mut rng = || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; s };
    let defs = mlkem_encode_defs();
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let w16lit = |v: &[i64]| v.iter().map(|x| format!("word16({})", x)).collect::<Vec<_>>().join(", ");
    let check = |label: &str, prog: &str, want: &[String]| {
        for (tag, out) in [("tw", tw_outcome(prog)), ("vm", vm_outcome(prog))] {
            assert!(out.error.is_none(), "{label}/{tag} errored: {:?}", out.error);
            let got = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
            assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "{label}/{tag} == spec");
        }
    };
    for &d in &[1u32, 4, 5, 10, 11, 12] {
        let coeffs: Vec<i64> = (0..256).map(|_| (rng() % MLKEM_Q as u64) as i64).collect();
        // compress (compressible inputs are full coeffs; result < 2^d)
        let want: Vec<String> = coeffs.iter().map(|&v| r_kem_compress(v, d).to_string()).collect();
        let prog = format!("{defs}## Main\nLet c be [{}].\nRepeat for x in compressW16Logos(c, {d}):\n    Show intOfWord16(x).\n", w16lit(&coeffs));
        check(&format!("compress d={d}"), &prog, &want);
        // decompress operates on d-bit values
        let small: Vec<i64> = coeffs.iter().map(|&v| r_kem_compress(v, d)).collect();
        let want: Vec<String> = small.iter().map(|&v| r_kem_decompress(v, d).to_string()).collect();
        let prog = format!("{defs}## Main\nLet c be [{}].\nRepeat for x in decompressW16Logos(c, {d}):\n    Show intOfWord16(x).\n", w16lit(&small));
        check(&format!("decompress d={d}"), &prog, &want);
        // byteEncode then byteDecode round-trips the d-bit values
        let enc = r_kem_byte_encode(&small, d);
        let want_enc: Vec<String> = enc.iter().map(|x| x.to_string()).collect();
        let prog = format!("{defs}## Main\nLet c be [{}].\nRepeat for x in byteEncodeW16Logos(c, {d}):\n    Show x.\n", w16lit(&small));
        check(&format!("byteEncode d={d}"), &prog, &want_enc);
        let dec = r_kem_byte_decode(&enc, d);
        let want_dec: Vec<String> = dec.iter().map(|x| x.to_string()).collect();
        let prog = format!("{defs}## Main\nLet b be [{}].\nRepeat for x in byteDecodeW16Logos(b, {d}):\n    Show intOfWord16(x).\n", lit(&enc));
        check(&format!("byteDecode d={d}"), &prog, &want_dec);
    }
}

// ───────────────────────── ML-KEM field + sampling tools in Logos (addModQ/subModQ/cbd2/cbd3) ─────────────────────────
fn r_kem_bit(buf: &[i64], k: usize) -> i64 { (buf[k / 8] >> (k % 8)) & 1 }
fn r_kem_cbd2(buf: &[i64]) -> Vec<i64> {
    (0..256).map(|c| {
        let a = r_kem_bit(buf, 4 * c) + r_kem_bit(buf, 4 * c + 1);
        let b = r_kem_bit(buf, 4 * c + 2) + r_kem_bit(buf, 4 * c + 3);
        ((a - b) % MLKEM_Q + MLKEM_Q) % MLKEM_Q
    }).collect()
}
fn r_kem_cbd3(buf: &[i64]) -> Vec<i64> {
    (0..256).map(|c| {
        let a = r_kem_bit(buf, 6 * c) + r_kem_bit(buf, 6 * c + 1) + r_kem_bit(buf, 6 * c + 2);
        let b = r_kem_bit(buf, 6 * c + 3) + r_kem_bit(buf, 6 * c + 4) + r_kem_bit(buf, 6 * c + 5);
        ((a - b) % MLKEM_Q + MLKEM_Q) % MLKEM_Q
    }).collect()
}
fn mlkem_field_defs() -> String {
    let mut p = String::new();
    p.push_str("## To mlkemBit (buf: Seq of Int) and (k: Int) -> Int:\n    Let byte be item (k / 8 + 1) of buf.\n    Let r be k % 8.\n    Let mutable pw be 1.\n    Repeat for i from 1 to r:\n        Set pw to pw * 2.\n    Return (byte / pw) % 2.\n\n");
    p.push_str("## To addModQW16Logos (a: Seq of Word16) and (b: Seq of Word16) -> Seq of Word16:\n    Let mutable out be a new Seq of Word16.\n    Let n be length of a.\n    Repeat for i from 1 to n:\n        Let s be intOfWord16(item i of a) + intOfWord16(item i of b).\n        Push word16(s % 3329) to out.\n    Return out.\n\n");
    p.push_str("## To subModQW16Logos (a: Seq of Word16) and (b: Seq of Word16) -> Seq of Word16:\n    Let mutable out be a new Seq of Word16.\n    Let n be length of a.\n    Repeat for i from 1 to n:\n        Let s be intOfWord16(item i of a) - intOfWord16(item i of b).\n        Push word16(((s % 3329) + 3329) % 3329) to out.\n    Return out.\n\n");
    p.push_str("## To cbd2W16Logos (buf: Seq of Int) -> Seq of Word16:\n    Let mutable out be a new Seq of Word16.\n    Repeat for c from 0 to 255:\n        Let a be mlkemBit(buf, 4 * c) + mlkemBit(buf, 4 * c + 1).\n        Let b be mlkemBit(buf, 4 * c + 2) + mlkemBit(buf, 4 * c + 3).\n        Let v be a - b.\n        Push word16(((v % 3329) + 3329) % 3329) to out.\n    Return out.\n\n");
    p.push_str("## To cbd3W16Logos (buf: Seq of Int) -> Seq of Word16:\n    Let mutable out be a new Seq of Word16.\n    Repeat for c from 0 to 255:\n        Let a be mlkemBit(buf, 6 * c) + mlkemBit(buf, 6 * c + 1) + mlkemBit(buf, 6 * c + 2).\n        Let b be mlkemBit(buf, 6 * c + 3) + mlkemBit(buf, 6 * c + 4) + mlkemBit(buf, 6 * c + 5).\n        Let v be a - b.\n        Push word16(((v % 3329) + 3329) % 3329) to out.\n    Return out.\n\n");
    p
}

#[test]
fn mlkem_field_tools_logos_match_spec_tw_and_vm() {
    let mut s: u64 = 0x1357_9bdf_2468_ace0;
    let mut rng = || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; s };
    let defs = mlkem_field_defs();
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let w16lit = |v: &[i64]| v.iter().map(|x| format!("word16({})", x)).collect::<Vec<_>>().join(", ");
    let check = |label: &str, prog: &str, want: &[String]| {
        for (tag, out) in [("tw", tw_outcome(prog)), ("vm", vm_outcome(prog))] {
            assert!(out.error.is_none(), "{label}/{tag} errored: {:?}", out.error);
            let got = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
            assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "{label}/{tag} == spec");
        }
    };
    let a: Vec<i64> = (0..256).map(|_| (rng() % MLKEM_Q as u64) as i64).collect();
    let b: Vec<i64> = (0..256).map(|_| (rng() % MLKEM_Q as u64) as i64).collect();
    let want: Vec<String> = a.iter().zip(&b).map(|(x, y)| ((x + y) % MLKEM_Q).to_string()).collect();
    let prog = format!("{defs}## Main\nLet a be [{}].\nLet b be [{}].\nRepeat for x in addModQW16Logos(a, b):\n    Show intOfWord16(x).\n", w16lit(&a), w16lit(&b));
    check("addModQ", &prog, &want);
    let want: Vec<String> = a.iter().zip(&b).map(|(x, y)| (((x - y) % MLKEM_Q + MLKEM_Q) % MLKEM_Q).to_string()).collect();
    let prog = format!("{defs}## Main\nLet a be [{}].\nLet b be [{}].\nRepeat for x in subModQW16Logos(a, b):\n    Show intOfWord16(x).\n", w16lit(&a), w16lit(&b));
    check("subModQ", &prog, &want);
    let buf2: Vec<i64> = (0..128).map(|_| (rng() % 256) as i64).collect();
    let want: Vec<String> = r_kem_cbd2(&buf2).iter().map(|x| x.to_string()).collect();
    let prog = format!("{defs}## Main\nLet buf be [{}].\nRepeat for x in cbd2W16Logos(buf):\n    Show intOfWord16(x).\n", lit(&buf2));
    check("cbd2", &prog, &want);
    let buf3: Vec<i64> = (0..192).map(|_| (rng() % 256) as i64).collect();
    let want: Vec<String> = r_kem_cbd3(&buf3).iter().map(|x| x.to_string()).collect();
    let prog = format!("{defs}## Main\nLet buf be [{}].\nRepeat for x in cbd3W16Logos(buf):\n    Show intOfWord16(x).\n", lit(&buf3));
    check("cbd3", &prog, &want);
}

/// The 8 ML-KEM tools COMPILED (AOT) — proves they work through the full Logos→Rust pipeline,
/// not just the tree-walker/VM. Self-contained program (defs + Main), compared to FIPS references.
#[test]
#[ignore = "compiles via rustc (slow) — ML-KEM encoding+field tools AOT == FIPS spec"]
fn mlkem_tools_logos_aot_eq_spec() {
    let defs = format!("{}{}", mlkem_encode_defs(), mlkem_field_defs());
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let w16lit = |v: &[i64]| v.iter().map(|x| format!("word16({})", x)).collect::<Vec<_>>().join(", ");
    let mut s: u64 = 0xa1b2_c3d4_e5f6_0718;
    let mut rng = || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; s };
    let coeffs: Vec<i64> = (0..256).map(|_| (rng() % MLKEM_Q as u64) as i64).collect();
    let a: Vec<i64> = (0..256).map(|_| (rng() % MLKEM_Q as u64) as i64).collect();
    let b: Vec<i64> = (0..256).map(|_| (rng() % MLKEM_Q as u64) as i64).collect();
    let buf2: Vec<i64> = (0..128).map(|_| (rng() % 256) as i64).collect();
    let buf3: Vec<i64> = (0..192).map(|_| (rng() % 256) as i64).collect();
    let comp = |d: u32| -> Vec<i64> { coeffs.iter().map(|&v| r_kem_compress(v, d)).collect() };
    // build expected output in the SAME order the Main below emits
    let mut want: Vec<String> = Vec::new();
    let small10 = comp(10);
    want.extend(small10.iter().map(|x| x.to_string()));                                   // compress d=10
    want.extend(small10.iter().map(|&v| r_kem_decompress(v, 10).to_string()));            // decompress d=10
    let enc12 = r_kem_byte_encode(&coeffs, 12);
    want.extend(r_kem_byte_decode(&enc12, 12).iter().map(|x| x.to_string()));             // byteDecode∘byteEncode d=12 == coeffs
    want.extend(a.iter().zip(&b).map(|(x, y)| ((x + y) % MLKEM_Q).to_string()));          // addModQ
    want.extend(a.iter().zip(&b).map(|(x, y)| (((x - y) % MLKEM_Q + MLKEM_Q) % MLKEM_Q).to_string())); // subModQ
    want.extend(r_kem_cbd2(&buf2).iter().map(|x| x.to_string()));                         // cbd2
    want.extend(r_kem_cbd3(&buf3).iter().map(|x| x.to_string()));                         // cbd3
    let prog = format!(
        "{defs}## Main\n\
         Let coeffs be [{}].\nLet a be [{}].\nLet b be [{}].\nLet buf2 be [{}].\nLet buf3 be [{}].\n\
         Repeat for x in compressW16Logos(coeffs, 10):\n    Show intOfWord16(x).\n\
         Repeat for x in decompressW16Logos(compressW16Logos(coeffs, 10), 10):\n    Show intOfWord16(x).\n\
         Repeat for x in byteDecodeW16Logos(byteEncodeW16Logos(coeffs, 12), 12):\n    Show intOfWord16(x).\n\
         Repeat for x in addModQW16Logos(a, b):\n    Show intOfWord16(x).\n\
         Repeat for x in subModQW16Logos(a, b):\n    Show intOfWord16(x).\n\
         Repeat for x in cbd2W16Logos(buf2):\n    Show intOfWord16(x).\n\
         Repeat for x in cbd3W16Logos(buf3):\n    Show intOfWord16(x).\n",
        w16lit(&coeffs), w16lit(&a), w16lit(&b), lit(&buf2), lit(&buf3));
    let aot = run_logos_with_args(&prog, &[]);
    assert!(aot.success, "ML-KEM tools AOT failed:\n{}", aot.stderr);
    let got = aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
    assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "ML-KEM tools AOT == FIPS spec");
}

// ───────────────────────── ML-KEM Montgomery tools in Logos (montReduce/fqmul/baseMul/toMont) ─────────────────────────
const MLKEM_ZETAS: [i64; 128] = [
    -1044, -758, -359, -1517, 1493, 1422, 287, 202, -171, 622, 1577, 182, 962, -1202, -1474, 1468,
    573, -1325, 264, 383, -829, 1458, -1602, -130, -681, 1017, 732, 608, -1542, 411, -205, -1571,
    1223, 652, -552, 1015, -1293, 1491, -282, -1544, 516, -8, -320, -666, -1618, -1162, 126, 1469,
    -853, -90, -271, 830, 107, -1421, -247, -951, -398, 961, -1508, -725, 448, -1065, 677, -1275,
    -1103, 430, 555, 843, -1251, 871, 1550, 105, 422, 587, 177, -235, -291, -460, 1574, 1653, -246,
    778, 1159, -147, -777, 1483, -602, 1119, -1590, 644, -872, 349, 418, 329, -156, -75, 817, 1097,
    603, 610, 1322, -1285, -1465, 384, -1215, -136, 1218, -1335, -874, 220, -1187, -1659, -1185,
    -1530, -1278, 794, -1510, -854, -870, 478, -108, -308, 996, 991, 958, -1460, 1522, 1628,
];
fn r_mlkem_montred(a: i64) -> i64 {
    let ai = a as i32;
    let t = (ai as i16).wrapping_mul((-3327i32) as i16);
    ((ai - (t as i32) * 3329) >> 16) as i64
}
fn r_mlkem_fqmul(x: i64, y: i64) -> i64 { r_mlkem_montred(x * y) }
fn r_mlkem_basemul(a: &[i64], b: &[i64]) -> Vec<i64> {
    let mut r = vec![0i64; 256];
    let bm = |a0: i64, a1: i64, b0: i64, b1: i64, z: i64| -> (i64, i64) {
        let r0 = r_mlkem_fqmul(r_mlkem_fqmul(a1, b1), z) + r_mlkem_fqmul(a0, b0);
        let r1 = r_mlkem_fqmul(a0, b1) + r_mlkem_fqmul(a1, b0);
        (r0, r1)
    };
    for i in 0..64 {
        let z = MLKEM_ZETAS[64 + i];
        let (c0, c1) = bm(a[4 * i], a[4 * i + 1], b[4 * i], b[4 * i + 1], z);
        r[4 * i] = c0; r[4 * i + 1] = c1;
        let (d0, d1) = bm(a[4 * i + 2], a[4 * i + 3], b[4 * i + 2], b[4 * i + 3], -z);
        r[4 * i + 2] = d0; r[4 * i + 3] = d1;
    }
    r.iter().map(|&x| ((x % 3329) + 3329) % 3329).collect()
}
fn r_mlkem_tomont(coeffs: &[i64]) -> Vec<i64> {
    coeffs.iter().map(|&w| ((r_mlkem_montred(w * 1353) % 3329) + 3329) % 3329).collect()
}
fn mlkem_mont_defs() -> String {
    let mut p = String::new();
    p.push_str("## To mlkemMontReduce (a: Int) -> Int:\n    Let alo be ((a % 65536) + 65536) % 65536.\n    Let mutable as16 be alo.\n    If as16 is at least 32768:\n        Set as16 to as16 - 65536.\n    Let prod be as16 * (0 - 3327).\n    Let mutable t be ((prod % 65536) + 65536) % 65536.\n    If t is at least 32768:\n        Set t to t - 65536.\n    Return (a - t * 3329) / 65536.\n\n");
    p.push_str("## To mlkemFqMul (x: Int) and (y: Int) -> Int:\n    Return mlkemMontReduce(x * y).\n\n");
    p.push_str("## To baseMulW16Logos (a: Seq of Word16) and (b: Seq of Word16) and (zetas: Seq of Int) -> Seq of Word16:\n    Let mutable out be a new Seq of Word16.\n    Repeat for i from 0 to 63:\n        Let zeta be item (64 + i + 1) of zetas.\n        Let a0 be intOfWord16(item (4 * i + 1) of a).\n        Let a1 be intOfWord16(item (4 * i + 2) of a).\n        Let b0 be intOfWord16(item (4 * i + 1) of b).\n        Let b1 be intOfWord16(item (4 * i + 2) of b).\n        Let r0 be mlkemFqMul(mlkemFqMul(a1, b1), zeta) + mlkemFqMul(a0, b0).\n        Let r1 be mlkemFqMul(a0, b1) + mlkemFqMul(a1, b0).\n        Push word16(((r0 % 3329) + 3329) % 3329) to out.\n        Push word16(((r1 % 3329) + 3329) % 3329) to out.\n        Let c0 be intOfWord16(item (4 * i + 3) of a).\n        Let c1 be intOfWord16(item (4 * i + 4) of a).\n        Let d0 be intOfWord16(item (4 * i + 3) of b).\n        Let d1 be intOfWord16(item (4 * i + 4) of b).\n        Let s0 be mlkemFqMul(mlkemFqMul(c1, d1), (0 - zeta)) + mlkemFqMul(c0, d0).\n        Let s1 be mlkemFqMul(c0, d1) + mlkemFqMul(c1, d0).\n        Push word16(((s0 % 3329) + 3329) % 3329) to out.\n        Push word16(((s1 % 3329) + 3329) % 3329) to out.\n    Return out.\n\n");
    p.push_str("## To toMontW16Logos (coeffs: Seq of Word16) -> Seq of Word16:\n    Let mutable out be a new Seq of Word16.\n    Repeat for x in coeffs:\n        Let w be intOfWord16(x).\n        Let r be mlkemMontReduce(w * 1353).\n        Push word16(((r % 3329) + 3329) % 3329) to out.\n    Return out.\n\n");
    p
}

#[test]
fn mlkem_mont_tools_logos_match_spec_tw_and_vm() {
    let mut s: u64 = 0xf0e1_d2c3_b4a5_9687;
    let mut rng = || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; s };
    let defs = mlkem_mont_defs();
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let w16lit = |v: &[i64]| v.iter().map(|x| format!("word16({})", x)).collect::<Vec<_>>().join(", ");
    let zlit = lit(&MLKEM_ZETAS);
    let check = |label: &str, prog: &str, want: &[String]| {
        for (tag, out) in [("tw", tw_outcome(prog)), ("vm", vm_outcome(prog))] {
            assert!(out.error.is_none(), "{label}/{tag} errored: {:?}", out.error);
            let got = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
            assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "{label}/{tag} == spec");
        }
    };
    // montReduce over random i32-range products
    let aps: Vec<i64> = (0..40).map(|_| ((rng() % 22000000) as i64) - 11000000).collect();
    let want: Vec<String> = aps.iter().map(|&a| r_mlkem_montred(a).to_string()).collect();
    let prog = format!("{defs}## Main\nLet aps be [{}].\nRepeat for a in aps:\n    Show mlkemMontReduce(a).\n", lit(&aps));
    check("montReduce", &prog, &want);
    // toMont over full poly
    let coeffs: Vec<i64> = (0..256).map(|_| (rng() % MLKEM_Q as u64) as i64).collect();
    let want: Vec<String> = r_mlkem_tomont(&coeffs).iter().map(|x| x.to_string()).collect();
    let prog = format!("{defs}## Main\nLet c be [{}].\nRepeat for x in toMontW16Logos(c):\n    Show intOfWord16(x).\n", w16lit(&coeffs));
    check("toMont", &prog, &want);
    // baseMul over two full polys
    let a: Vec<i64> = (0..256).map(|_| (rng() % MLKEM_Q as u64) as i64).collect();
    let b: Vec<i64> = (0..256).map(|_| (rng() % MLKEM_Q as u64) as i64).collect();
    let want: Vec<String> = r_mlkem_basemul(&a, &b).iter().map(|x| x.to_string()).collect();
    let prog = format!("{defs}## Main\nLet a be [{}].\nLet b be [{}].\nLet zt be [{}].\nRepeat for x in baseMulW16Logos(a, b, zt):\n    Show intOfWord16(x).\n", w16lit(&a), w16lit(&b), zlit);
    check("baseMul", &prog, &want);
}

/// The ML-KEM Montgomery tools COMPILED (AOT) — baseMul/toMont/montReduce through Logos→Rust→run.
#[test]
#[ignore = "compiles via rustc (slow) — ML-KEM Montgomery tools AOT == exact-native spec"]
fn mlkem_mont_tools_logos_aot_eq_spec() {
    let defs = mlkem_mont_defs();
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let w16lit = |v: &[i64]| v.iter().map(|x| format!("word16({})", x)).collect::<Vec<_>>().join(", ");
    let mut s: u64 = 0x0011_2233_4455_6677;
    let mut rng = || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; s };
    let a: Vec<i64> = (0..256).map(|_| (rng() % MLKEM_Q as u64) as i64).collect();
    let b: Vec<i64> = (0..256).map(|_| (rng() % MLKEM_Q as u64) as i64).collect();
    let mut want: Vec<String> = Vec::new();
    want.extend(r_mlkem_tomont(&a).iter().map(|x| x.to_string()));   // toMont(a)
    want.extend(r_mlkem_basemul(&a, &b).iter().map(|x| x.to_string())); // baseMul(a,b)
    let prog = format!(
        "{defs}## Main\nLet a be [{}].\nLet b be [{}].\nLet zt be [{}].\n\
         Repeat for x in toMontW16Logos(a):\n    Show intOfWord16(x).\n\
         Repeat for x in baseMulW16Logos(a, b, zt):\n    Show intOfWord16(x).\n",
        w16lit(&a), w16lit(&b), lit(&MLKEM_ZETAS));
    let aot = run_logos_with_args(&prog, &[]);
    assert!(aot.success, "ML-KEM Montgomery tools AOT failed:\n{}", aot.stderr);
    let got = aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
    assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "ML-KEM Montgomery tools AOT == spec");
}

// ───────────────────────── ML-KEM sampleA rejection in Logos (the pure-logic half; SHAKE stays native) ─────────────────────────
fn r_mlkem_reject(buf: &[i64]) -> Vec<i64> {
    let mut out = Vec::new();
    let n = buf.len();
    let mut pos = 0;
    while pos + 3 <= n {
        if out.len() >= 256 { break; }
        let (b0, b1, b2) = (buf[pos], buf[pos + 1], buf[pos + 2]);
        let d1 = b0 + 256 * (b1 % 16);
        let d2 = (b1 / 16) + 16 * b2;
        if d1 < 3329 { out.push(d1); }
        if d2 < 3329 && out.len() < 256 { out.push(d2); }
        pos += 3;
    }
    out
}
fn mlkem_sample_defs() -> String {
    let mut p = String::new();
    p.push_str("## To mlkemRejSampleNttLogos (buf: Seq of Int) -> Seq of Word16:\n    Let mutable out be a new Seq of Word16.\n    Let mutable cnt be 0.\n    Let mutable pos be 0.\n    Let n be length of buf.\n    While (pos + 3) is at most n:\n        If cnt is at least 256:\n            Set pos to n.\n        If cnt is less than 256:\n            Let b0 be item (pos + 1) of buf.\n            Let b1 be item (pos + 2) of buf.\n            Let b2 be item (pos + 3) of buf.\n            Let d1 be b0 + 256 * (b1 % 16).\n            Let d2 be (b1 / 16) + 16 * b2.\n            If d1 is less than 3329:\n                Push word16(d1) to out.\n                Set cnt to cnt + 1.\n            If d2 is less than 3329:\n                If cnt is less than 256:\n                    Push word16(d2) to out.\n                    Set cnt to cnt + 1.\n            Set pos to pos + 3.\n    Return out.\n\n");
    p
}

#[test]
fn mlkem_reject_sample_logos_match_spec_tw_and_vm() {
    let mut s: u64 = 0x2468_ace0_1357_9bdf;
    let mut rng = || { s ^= s << 13; s ^= s >> 7; s ^= s << 17; s };
    let defs = mlkem_sample_defs();
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    for &nbytes in &[840usize, 1008, 672] {
        let buf: Vec<i64> = (0..nbytes).map(|_| (rng() % 256) as i64).collect();
        let want: Vec<String> = r_mlkem_reject(&buf).iter().map(|x| x.to_string()).collect();
        let prog = format!("{defs}## Main\nLet buf be [{}].\nRepeat for x in mlkemRejSampleNttLogos(buf):\n    Show intOfWord16(x).\n", lit(&buf));
        for (tag, out) in [("tw", tw_outcome(&prog)), ("vm", vm_outcome(&prog))] {
            assert!(out.error.is_none(), "reject/{tag} errored: {:?}", out.error);
            let got = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
            assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "reject nbytes={nbytes}/{tag} == spec (got {} coeffs)", got.len());
        }
    }
}

/// The new Word64 builtins (intOfWord64/word64Shl/word64Shr/word64And) + Word64 xor/rotl — the
/// substrate for Keccak-f[1600] in Logos. tw/vm.
#[test]
fn word64_builtins_logos_work_tw_and_vm() {
    let prog = "## Main\n\
        Show intOfWord64(word64Shl(word64(255), 8)).\n\
        Show intOfWord64(word64Shr(word64(65280), 4)).\n\
        Show intOfWord64(word64And(word64(255), word64(15))).\n\
        Show intOfWord64((word64(255)) xor (word64(4080))).\n\
        Show intOfWord64(rotl(word64(1), 4)).\n\
        Let allones be word64(0 - 1).\n\
        Show intOfWord64(word64And(allones xor word64(255), word64(255))).\n";
    let want = vec!["65280", "4080", "15", "3855", "16", "0"];
    for (tag, out) in [("tw", tw_outcome(prog)), ("vm", vm_outcome(prog))] {
        assert!(out.error.is_none(), "word64/{tag} errored: {:?}", out.error);
        let got = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
        assert_eq!(got, want, "word64 builtins/{tag}");
    }
}

// ───────────────────────── Keccak-f[1600] + SHA3-256 in Logos (the hash, all the way down) ─────────────────────────
const KECCAK_RSRC: [i64; 25] = [0,6,12,18,24,3,9,10,16,22,1,7,13,19,20,4,5,11,17,23,2,8,14,15,21];
const KECCAK_ROFF: [i64; 25] = [0,44,43,21,14,28,20,3,45,61,1,6,25,8,18,27,36,10,15,56,62,55,39,41,2];
const KECCAK_RC: [u64; 24] = [
    0x0000000000000001, 0x0000000000008082, 0x800000000000808a, 0x8000000080008000,
    0x000000000000808b, 0x0000000080000001, 0x8000000080008081, 0x8000000000008009,
    0x000000000000008a, 0x0000000000000088, 0x0000000080008009, 0x000000008000000a,
    0x000000008000808b, 0x800000000000008b, 0x8000000000008089, 0x8000000000008003,
    0x8000000000008002, 0x8000000000000080, 0x000000000000800a, 0x800000008000000a,
    0x8000000080008081, 0x8000000000008080, 0x0000000080000001, 0x8000000080008008,
];
fn keccak_defs() -> String {
    let mut p = String::new();
    p.push_str("## To keccakF (st: Seq of Word64) and (rc: Seq of Word64) and (rsrc: Seq of Int) and (roff: Seq of Int) -> Seq of Word64:\n");
    p.push_str("    Let mutable a be a new Seq of Word64.\n    Repeat for x in st:\n        Push x to a.\n");
    p.push_str("    Repeat for rnd from 1 to 24:\n");
    p.push_str("        Let mutable c be a new Seq of Word64.\n        Repeat for x from 0 to 4:\n            Push ((item (x + 1) of a) xor (item (x + 6) of a) xor (item (x + 11) of a) xor (item (x + 16) of a) xor (item (x + 21) of a)) to c.\n");
    p.push_str("        Repeat for x from 0 to 4:\n            Let d be (item ((x + 4) % 5 + 1) of c) xor rotl(item ((x + 1) % 5 + 1) of c, 1).\n            Repeat for y from 0 to 4:\n                Set item (5 * y + x + 1) of a to (item (5 * y + x + 1) of a) xor d.\n");
    p.push_str("        Let mutable b be a new Seq of Word64.\n        Repeat for k from 0 to 24:\n            Push rotl(item ((item (k + 1) of rsrc) + 1) of a, item (k + 1) of roff) to b.\n");
    p.push_str("        Let allones be word64(0 - 1).\n        Repeat for y from 0 to 4:\n            Repeat for x from 0 to 4:\n                Let bb0 be item (5 * y + x + 1) of b.\n                Let bb1 be item (5 * y + (x + 1) % 5 + 1) of b.\n                Let bb2 be item (5 * y + (x + 2) % 5 + 1) of b.\n                Set item (5 * y + x + 1) of a to bb0 xor word64And(bb1 xor allones, bb2).\n");
    p.push_str("        Set item 1 of a to (item 1 of a) xor (item rnd of rc).\n");
    p.push_str("    Return a.\n\n");
    p.push_str("## To sha3_256Logos (msg: Seq of Int) and (rc: Seq of Word64) and (rsrc: Seq of Int) and (roff: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let mutable padded be a new Seq of Int.\n    Repeat for x in msg:\n        Push x to padded.\n    Push 6 to padded.\n    While ((length of padded) % 136) is greater than 0:\n        Push 0 to padded.\n");
    p.push_str("    Let ln be length of padded.\n    Set item ln of padded to (item ln of padded) + 128.\n");
    p.push_str("    Let mutable state be a new Seq of Word64.\n    Repeat for k from 1 to 25:\n        Push word64(0) to state.\n");
    p.push_str("    Let nblk be (length of padded) / 136.\n    Repeat for blk from 0 to (nblk - 1):\n        Repeat for l from 0 to 16:\n            Let base be blk * 136 + l * 8.\n            Let mutable lane be word64(item (base + 1) of padded).\n            Repeat for j from 1 to 7:\n                Set lane to lane xor word64Shl(word64(item (base + j + 1) of padded), 8 * j).\n            Set item (l + 1) of state to (item (l + 1) of state) xor lane.\n        Set state to keccakF(state, rc, rsrc, roff).\n");
    p.push_str("    Let mutable out be a new Seq of Int.\n    Repeat for k from 0 to 31:\n        Let lane be item (k / 8 + 1) of state.\n        Push intOfWord64(word64And(word64Shr(lane, 8 * (k % 8)), word64(255))) to out.\n    Return out.\n\n");
    p
}

#[test]
fn sha3_256_logos_matches_native_tw_and_vm() {
    use logicaffeine_system::keccak::sha3_256_bytes;
    let defs = keccak_defs();
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let w64lit = |v: &[i64]| v.iter().map(|x| format!("word64({})", x)).collect::<Vec<_>>().join(", ");
    let rc_i64: Vec<i64> = KECCAK_RC.iter().map(|&x| x as i64).collect();
    let rc_s = w64lit(&rc_i64);
    let rsrc_s = lit(&KECCAK_RSRC);
    let roff_s = lit(&KECCAK_ROFF);
    // test a few message lengths incl. block-boundary cases
    for &mlen in &[0usize, 1, 3, 135, 136, 137, 200] {
        let msg: Vec<i64> = (0..mlen).map(|i| ((i * 37 + 11) % 256) as i64).collect();
        let mbytes: Vec<u8> = msg.iter().map(|&b| b as u8).collect();
        let want: Vec<String> = sha3_256_bytes(&mbytes).iter().map(|b| b.to_string()).collect();
        let prog = format!(
            "{defs}## Main\nLet msg be [{}].\nLet rc be [{}].\nLet rsrc be [{}].\nLet roff be [{}].\nRepeat for x in sha3_256Logos(msg, rc, rsrc, roff):\n    Show x.\n",
            lit(&msg), rc_s, rsrc_s, roff_s);
        for (tag, out) in [("tw", tw_outcome(&prog)), ("vm", vm_outcome(&prog))] {
            assert!(out.error.is_none(), "sha3/{tag} mlen={mlen} errored: {:?}", out.error);
            let got = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
            assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "sha3_256 mlen={mlen}/{tag} == native");
        }
    }
}

// General FIPS-202 sponge in Logos: rate + domain-separator + outlen parameterize SHA3-256/512 & SHAKE128/256.
fn keccak_sponge_def() -> String {
    let mut p = String::new();
    p.push_str("## To keccakSponge (msg: Seq of Int) and (rc: Seq of Word64) and (rsrc: Seq of Int) and (roff: Seq of Int) and (rate: Int) and (dsbyte: Int) and (outlen: Int) -> Seq of Int:\n");
    p.push_str("    Let mutable padded be a new Seq of Int.\n    Repeat for x in msg:\n        Push x to padded.\n    Push dsbyte to padded.\n    While ((length of padded) % rate) is greater than 0:\n        Push 0 to padded.\n");
    p.push_str("    Let ln be length of padded.\n    Set item ln of padded to (item ln of padded) + 128.\n");
    p.push_str("    Let mutable state be a new Seq of Word64.\n    Repeat for k from 1 to 25:\n        Push word64(0) to state.\n");
    p.push_str("    Let lanes be rate / 8.\n    Let nblk be (length of padded) / rate.\n");
    p.push_str("    Repeat for blk from 0 to (nblk - 1):\n        Repeat for l from 0 to (lanes - 1):\n            Let base be blk * rate + l * 8.\n            Let mutable lane be word64(item (base + 1) of padded).\n            Repeat for j from 1 to 7:\n                Set lane to lane xor word64Shl(word64(item (base + j + 1) of padded), 8 * j).\n            Set item (l + 1) of state to (item (l + 1) of state) xor lane.\n        Set state to keccakF(state, rc, rsrc, roff).\n");
    p.push_str("    Let mutable out be a new Seq of Int.\n    While (length of out) is less than outlen:\n        Repeat for l from 0 to (lanes - 1):\n            Let lane be item (l + 1) of state.\n            Repeat for jj from 0 to 7:\n                If (length of out) is less than outlen:\n                    Push intOfWord64(word64And(word64Shr(lane, 8 * jj), word64(255))) to out.\n        If (length of out) is less than outlen:\n            Set state to keccakF(state, rc, rsrc, roff).\n    Return out.\n\n");
    p
}

fn keccak_full_defs() -> String { format!("{}{}", keccak_defs(), keccak_sponge_def()) }

fn keccak_sponge_program(msg: &[i64], rate: i64, ds: i64, outlen: i64) -> String {
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let rc_i64: Vec<i64> = KECCAK_RC.iter().map(|&x| x as i64).collect();
    let rc_s = rc_i64.iter().map(|x| format!("word64({})", x)).collect::<Vec<_>>().join(", ");
    format!(
        "{}## Main\nLet msg be [{}].\nLet rc be [{}].\nLet rsrc be [{}].\nLet roff be [{}].\nRepeat for x in keccakSponge(msg, rc, rsrc, roff, {rate}, {ds}, {outlen}):\n    Show x.\n",
        keccak_full_defs(), lit(msg), rc_s, lit(&KECCAK_RSRC), lit(&KECCAK_ROFF))
}

#[test]
fn keccak_sponge_logos_all_modes_tw_and_vm() {
    use logicaffeine_system::keccak::{sha3_256_bytes, sha3_512_bytes, shake128_bytes, shake256_bytes};
    for &mlen in &[0usize, 1, 5, 71, 72, 135, 136, 167, 168, 200] {
        let msg: Vec<i64> = (0..mlen).map(|i| ((i * 37 + 11) % 256) as i64).collect();
        let mb: Vec<u8> = msg.iter().map(|&b| b as u8).collect();
        let cases: Vec<(i64, i64, i64, Vec<u8>)> = vec![
            (136, 6, 32, sha3_256_bytes(&mb).to_vec()),
            (72, 6, 64, sha3_512_bytes(&mb).to_vec()),
            (168, 31, 32, shake128_bytes(&mb, 32)),
            (168, 31, 200, shake128_bytes(&mb, 200)),   // > rate → multi-block squeeze
            (136, 31, 64, shake256_bytes(&mb, 64)),
            (136, 31, 500, shake256_bytes(&mb, 500)),   // spans multiple squeeze permutations
        ];
        for (rate, ds, outlen, want_bytes) in cases {
            let want: Vec<String> = want_bytes.iter().map(|b| b.to_string()).collect();
            let prog = keccak_sponge_program(&msg, rate, ds, outlen);
            for (tag, out) in [("tw", tw_outcome(&prog)), ("vm", vm_outcome(&prog))] {
                assert!(out.error.is_none(), "sponge rate={rate} ds={ds} out={outlen} mlen={mlen}/{tag}: {:?}", out.error);
                let got = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
                assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                    "keccakSponge rate={rate} ds={ds} out={outlen} mlen={mlen}/{tag} == native");
            }
        }
    }
}

#[test]
#[ignore = "compiles via rustc (slow) — SHA3/SHAKE sponge in Logos AOT == native FIPS-202 (exercises Word64 builtins through codegen)"]
fn keccak_sponge_logos_aot_eq_native() {
    use logicaffeine_system::keccak::{sha3_256_bytes, sha3_512_bytes, shake128_bytes, shake256_bytes};
    let msg: Vec<i64> = (0..137).map(|i| ((i * 37 + 11) % 256) as i64).collect();
    let mb: Vec<u8> = msg.iter().map(|&b| b as u8).collect();
    let cases: Vec<(i64, i64, i64, Vec<u8>)> = vec![
        (136, 6, 32, sha3_256_bytes(&mb).to_vec()),
        (72, 6, 64, sha3_512_bytes(&mb).to_vec()),
        (168, 31, 200, shake128_bytes(&mb, 200)),   // multi-block squeeze
        (136, 31, 500, shake256_bytes(&mb, 500)),   // multiple squeeze permutations
    ];
    for (rate, ds, outlen, want_bytes) in cases {
        let want: Vec<String> = want_bytes.iter().map(|b| b.to_string()).collect();
        let prog = keccak_sponge_program(&msg, rate, ds, outlen);
        let aot = run_logos_with_args(&prog, &[]);
        assert!(aot.success, "AOT compile/run failed rate={rate}:\n{}", aot.stderr);
        let got = aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
        assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "AOT keccakSponge rate={rate} ds={ds} out={outlen} == native");
    }
}

// ML-KEM SampleNTT matrix entry Â[i][j] in Logos: SHAKE128(seed‖i‖j) XOF, streamed block-by-block
// through FIPS-203 rejection sampling until 256 coefficients — the last native ML-KEM primitive.
fn mlkem_sampleA_def() -> String {
    let mut p = String::new();
    p.push_str("## To mlkemSampleAElement (seed: Seq of Int) and (ii: Int) and (jj: Int) and (rc: Seq of Word64) and (rsrc: Seq of Int) and (roff: Seq of Int) -> Seq of Word16:\n");
    p.push_str("    Let mutable blk be a new Seq of Int.\n    Repeat for t from 1 to 32:\n        Push item t of seed to blk.\n    Push ii to blk.\n    Push jj to blk.\n    Push 31 to blk.\n    While (length of blk) is less than 168:\n        Push 0 to blk.\n    Set item 168 of blk to (item 168 of blk) + 128.\n");
    p.push_str("    Let mutable state be a new Seq of Word64.\n    Repeat for k from 1 to 25:\n        Push word64(0) to state.\n");
    p.push_str("    Repeat for l from 0 to 20:\n        Let base be l * 8.\n        Let mutable lane be word64(item (base + 1) of blk).\n        Repeat for j from 1 to 7:\n            Set lane to lane xor word64Shl(word64(item (base + j + 1) of blk), 8 * j).\n        Set item (l + 1) of state to (item (l + 1) of state) xor lane.\n    Set state to keccakF(state, rc, rsrc, roff).\n");
    p.push_str("    Let mutable out be a new Seq of Word16.\n    Let mutable cnt be 0.\n    While cnt is less than 256:\n");
    p.push_str("        Let mutable buf be a new Seq of Int.\n        Repeat for l from 0 to 20:\n            Let lane be item (l + 1) of state.\n            Repeat for bj from 0 to 7:\n                Push intOfWord64(word64And(word64Shr(lane, 8 * bj), word64(255))) to buf.\n");
    p.push_str("        Let mutable pos be 0.\n        While (pos + 3) is at most 168:\n            If cnt is at least 256:\n                Set pos to 168.\n            If cnt is less than 256:\n                Let b0 be item (pos + 1) of buf.\n                Let b1 be item (pos + 2) of buf.\n                Let b2 be item (pos + 3) of buf.\n                Let d1 be b0 + 256 * (b1 % 16).\n                Let d2 be (b1 / 16) + 16 * b2.\n                If d1 is less than 3329:\n                    Push word16(d1) to out.\n                    Set cnt to cnt + 1.\n                If d2 is less than 3329:\n                    If cnt is less than 256:\n                        Push word16(d2) to out.\n                        Set cnt to cnt + 1.\n                Set pos to pos + 3.\n");
    p.push_str("        If cnt is less than 256:\n            Set state to keccakF(state, rc, rsrc, roff).\n    Return out.\n\n");
    p
}

fn mlkem_sampleA_program(seed: &[i64], i: i64, j: i64) -> String {
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let rc_i64: Vec<i64> = KECCAK_RC.iter().map(|&x| x as i64).collect();
    let rc_s = rc_i64.iter().map(|x| format!("word64({})", x)).collect::<Vec<_>>().join(", ");
    format!(
        "{}{}## Main\nLet seed be [{}].\nLet rc be [{}].\nLet rsrc be [{}].\nLet roff be [{}].\nRepeat for x in mlkemSampleAElement(seed, {i}, {j}, rc, rsrc, roff):\n    Show intOfWord16(x).\n",
        keccak_defs(), mlkem_sampleA_def(), lit(seed), rc_s, lit(&KECCAK_RSRC), lit(&KECCAK_ROFF))
}

#[test]
fn mlkem_sampleA_element_logos_matches_native_tw_and_vm() {
    let seed: Vec<i64> = (0..32).map(|i| ((i * 7 + 3) % 256) as i64).collect();
    let seed_u8: Vec<u8> = seed.iter().map(|&b| b as u8).collect();
    for (i, j) in [(0i64, 0i64), (1, 2), (2, 1), (0, 2)] {
        let native = logicaffeine_system::ntt::mlkem_sample_a_w16(&seed_u8, i, j);
        let want: Vec<String> = native.iter().map(|w| w.0.to_string()).collect();
        assert_eq!(want.len(), 256, "native sampleA must yield 256 coeffs");
        let prog = mlkem_sampleA_program(&seed, i, j);
        for (tag, out) in [("tw", tw_outcome(&prog)), ("vm", vm_outcome(&prog))] {
            assert!(out.error.is_none(), "sampleA ({i},{j})/{tag} errored: {:?}", out.error);
            let got = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
            assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "mlkemSampleAElement ({i},{j})/{tag} == native");
        }
    }
}

#[test]
#[ignore = "compiles via rustc (slow) — ML-KEM sampleA (SHAKE128 rejection) in Logos AOT == native"]
fn mlkem_sampleA_element_logos_aot_eq_native() {
    let seed: Vec<i64> = (0..32).map(|i| ((i * 7 + 3) % 256) as i64).collect();
    let seed_u8: Vec<u8> = seed.iter().map(|&b| b as u8).collect();
    for (i, j) in [(0i64, 0i64), (2, 1)] {
        let native = logicaffeine_system::ntt::mlkem_sample_a_w16(&seed_u8, i, j);
        let want: Vec<String> = native.iter().map(|w| w.0.to_string()).collect();
        let prog = mlkem_sampleA_program(&seed, i, j);
        let aot = run_logos_with_args(&prog, &[]);
        assert!(aot.success, "AOT compile/run failed ({i},{j}):\n{}", aot.stderr);
        let got = aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
        assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "AOT mlkemSampleAElement ({i},{j}) == native");
    }
}

// Reusable Keccak sponge steps in Logos: absorb one padded rate-block into fresh state; extract
// `lanes*8` output bytes from the state (LE). Both ride the proven keccakF.
fn keccak_helper_defs() -> String {
    let mut p = String::new();
    p.push_str("## To keccakAbsorbBlock (blk: Seq of Int) and (lanes: Int) and (rc: Seq of Word64) and (rsrc: Seq of Int) and (roff: Seq of Int) -> Seq of Word64:\n");
    p.push_str("    Let mutable state be a new Seq of Word64.\n    Repeat for k from 1 to 25:\n        Push word64(0) to state.\n");
    p.push_str("    Repeat for l from 0 to (lanes - 1):\n        Let base be l * 8.\n        Let mutable lane be word64(item (base + 1) of blk).\n        Repeat for j from 1 to 7:\n            Set lane to lane xor word64Shl(word64(item (base + j + 1) of blk), 8 * j).\n        Set item (l + 1) of state to (item (l + 1) of state) xor lane.\n    Set state to keccakF(state, rc, rsrc, roff).\n    Return state.\n\n");
    p.push_str("## To keccakExtractRate (state: Seq of Word64) and (lanes: Int) -> Seq of Int:\n");
    p.push_str("    Let mutable buf be a new Seq of Int.\n    Repeat for l from 0 to (lanes - 1):\n        Let lane be item (l + 1) of state.\n        Repeat for bj from 0 to 7:\n            Push intOfWord64(word64And(word64Shr(lane, 8 * bj), word64(255))) to buf.\n    Return buf.\n\n");
    p
}

// ML-DSA-65 SHAKE-driven samplers, FULLY in Logos (no native shake): RejNTTPoly (ExpandA element,
// SHAKE128, 23-bit rejection < q=8380417) and SampleInBall (challenge c, SHAKE256, τ=49 signed placement).
fn mldsa_shake_sampler_defs() -> String {
    let mut p = String::new();
    p.push_str("## To mldsaRejNttPolyLogos (seed: Seq of Int) and (rc: Seq of Word64) and (rsrc: Seq of Int) and (roff: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let mutable blk be a new Seq of Int.\n    Repeat for x in seed:\n        Push x to blk.\n    Push 31 to blk.\n    While (length of blk) is less than 168:\n        Push 0 to blk.\n    Set item 168 of blk to (item 168 of blk) + 128.\n");
    p.push_str("    Let mutable state be keccakAbsorbBlock(blk, 21, rc, rsrc, roff).\n    Let mutable buf be keccakExtractRate(state, 21).\n");
    p.push_str("    Let mutable a be a new Seq of Int.\n    Let mutable ctr be 0.\n    Let mutable pos be 0.\n    While ctr is less than 256:\n");
    p.push_str("        If (pos + 3) is greater than 168:\n            Set state to keccakF(state, rc, rsrc, roff).\n            Set buf to keccakExtractRate(state, 21).\n            Set pos to 0.\n");
    p.push_str("        Let b0 be item (pos + 1) of buf.\n        Let b1 be item (pos + 2) of buf.\n        Let b2 be item (pos + 3) of buf.\n        Let d be b0 + 256 * b1 + 65536 * (b2 % 128).\n        Set pos to pos + 3.\n        If d is less than 8380417:\n            Push d to a.\n            Set ctr to ctr + 1.\n    Return a.\n\n");
    p.push_str("## To mldsaSampleInBallLogos (seed: Seq of Int) and (rc: Seq of Word64) and (rsrc: Seq of Int) and (roff: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let mutable blk be a new Seq of Int.\n    Repeat for x in seed:\n        Push x to blk.\n    Push 31 to blk.\n    While (length of blk) is less than 136:\n        Push 0 to blk.\n    Set item 136 of blk to (item 136 of blk) + 128.\n");
    p.push_str("    Let mutable state be keccakAbsorbBlock(blk, 17, rc, rsrc, roff).\n    Let mutable buf be keccakExtractRate(state, 17).\n");
    p.push_str("    Let mutable signBytes be a new Seq of Int.\n    Repeat for t from 1 to 8:\n        Push item t of buf to signBytes.\n    Let pow2 be [1, 2, 4, 8, 16, 32, 64, 128].\n");
    p.push_str("    Let mutable c be a new Seq of Int.\n    Repeat for k from 1 to 256:\n        Push 0 to c.\n    Let mutable pos be 8.\n    Let mutable signIdx be 0.\n");
    p.push_str("    Repeat for i from 207 to 255:\n        Let mutable found be 0.\n        Let mutable jj be 0.\n        While found is equal to 0:\n            If pos is at least 136:\n                Set state to keccakF(state, rc, rsrc, roff).\n                Set buf to keccakExtractRate(state, 17).\n                Set pos to 0.\n            Let candidate be item (pos + 1) of buf.\n            Set pos to pos + 1.\n            If candidate is at most i:\n                Set jj to candidate.\n                Set found to 1.\n");
    p.push_str("        Set item (i + 1) of c to item (jj + 1) of c.\n        Let byteIdx be (signIdx / 8) + 1.\n        Let bitv be (item byteIdx of signBytes / item ((signIdx % 8) + 1) of pow2) % 2.\n        Set item (jj + 1) of c to 1 - 2 * bitv.\n        Set signIdx to signIdx + 1.\n    Return c.\n\n");
    p
}

fn mldsa_sampler_all_defs() -> String {
    format!("{}{}{}", keccak_defs(), keccak_helper_defs(), mldsa_shake_sampler_defs())
}

// Independent FIPS-204 oracle over the pub SHAKE byte stream (no private native fn, no crypto.lg):
// RejNTTPoly reads 3-byte groups as a 23-bit int, keeps those < q (168 % 3 == 0 so the flat squeeze
// stream matches the block-reset native reader exactly).
fn ref_rej_ntt_poly(seed: &[u8]) -> Vec<String> {
    let stream = logicaffeine_system::keccak::shake128_bytes(seed, 168 * 10);
    let (mut a, mut pos) = (Vec::<String>::with_capacity(256), 0usize);
    while a.len() < 256 {
        let d = (stream[pos] as u32) | ((stream[pos + 1] as u32) << 8) | (((stream[pos + 2] as u32) & 0x7f) << 16);
        pos += 3;
        if d < 8_380_417 { a.push(d.to_string()); }
    }
    a
}
// SampleInBall: 8-byte sign word, then τ signed placements via bounded rejection (candidate ≤ i).
fn ref_sample_in_ball(seed: &[u8]) -> Vec<String> {
    let stream = logicaffeine_system::keccak::shake256_bytes(seed, 136 * 10);
    let signs = u64::from_le_bytes(stream[0..8].try_into().unwrap());
    let (mut pos, mut c, mut sign_idx) = (8usize, [0i64; 256], 0u32);
    for i in (256 - 49)..256 {
        let j = loop {
            let candidate = stream[pos] as usize;
            pos += 1;
            if candidate <= i { break candidate; }
        };
        c[i] = c[j];
        c[j] = 1 - 2 * (((signs >> sign_idx) & 1) as i64);
        sign_idx += 1;
    }
    c.iter().map(|x| x.to_string()).collect()
}
fn mldsa_logos_program(callee: &str, seed: &[i64]) -> String {
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let rc_i64: Vec<i64> = KECCAK_RC.iter().map(|&x| x as i64).collect();
    let rc_s = rc_i64.iter().map(|x| format!("word64({})", x)).collect::<Vec<_>>().join(", ");
    format!(
        "{}## Main\nLet seed be [{}].\nLet rc be [{}].\nLet rsrc be [{}].\nLet roff be [{}].\nRepeat for x in {callee}(seed, rc, rsrc, roff):\n    Show x.\n",
        mldsa_sampler_all_defs(), lit(seed), rc_s, lit(&KECCAK_RSRC), lit(&KECCAK_ROFF))
}

#[test]
fn mldsa_shake_samplers_logos_match_fips204_tw_and_vm() {
    // ExpandA element seed = rho(32) ‖ s ‖ r (34 bytes); SampleInBall seed = c̃ (48 bytes).
    let expand_seed: Vec<i64> = (0..34).map(|i| ((i * 11 + 5) % 256) as i64).collect();
    let sib_seed: Vec<i64> = (0..48).map(|i| ((i * 13 + 7) % 256) as i64).collect();
    let to_u8 = |v: &[i64]| v.iter().map(|&b| b as u8).collect::<Vec<u8>>();
    let cases: [(&str, &[i64], Vec<String>); 2] = [
        ("mldsaRejNttPolyLogos", &expand_seed, ref_rej_ntt_poly(&to_u8(&expand_seed))),
        ("mldsaSampleInBallLogos", &sib_seed, ref_sample_in_ball(&to_u8(&sib_seed))),
    ];
    for (my_fn, seed, want) in cases {
        assert_eq!(want.len(), 256, "FIPS ref for {my_fn} must yield 256 values");
        let my_prog = mldsa_logos_program(my_fn, seed);
        for (tag, out) in [("tw", tw_outcome(&my_prog)), ("vm", vm_outcome(&my_prog))] {
            assert!(out.error.is_none(), "{my_fn}/{tag} errored: {:?}", out.error);
            let got = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
            assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "{my_fn}/{tag} == FIPS-204 spec");
        }
    }
}

#[test]
#[ignore = "compiles via rustc (slow) — ML-DSA SHAKE samplers (RejNTTPoly + SampleInBall) in Logos AOT == FIPS-204"]
fn mldsa_shake_samplers_logos_aot_eq_fips204() {
    let expand_seed: Vec<i64> = (0..34).map(|i| ((i * 11 + 5) % 256) as i64).collect();
    let sib_seed: Vec<i64> = (0..48).map(|i| ((i * 13 + 7) % 256) as i64).collect();
    let to_u8 = |v: &[i64]| v.iter().map(|&b| b as u8).collect::<Vec<u8>>();
    let cases: [(&str, &[i64], Vec<String>); 2] = [
        ("mldsaRejNttPolyLogos", &expand_seed, ref_rej_ntt_poly(&to_u8(&expand_seed))),
        ("mldsaSampleInBallLogos", &sib_seed, ref_sample_in_ball(&to_u8(&sib_seed))),
    ];
    for (my_fn, seed, want) in cases {
        let prog = mldsa_logos_program(my_fn, seed);
        let aot = run_logos_with_args(&prog, &[]);
        assert!(aot.success, "AOT {my_fn} compile/run failed:\n{}", aot.stderr);
        let got = aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
        assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "AOT {my_fn} == FIPS-204 spec");
    }
}

// ML-DSA-65 secret/mask samplers, FULLY in Logos: SampleBounded (s1/s2, SHAKE256, η=4, nibble reject
// < 9 → η−t) and ExpandMask (y, SHAKE256, γ1, 20-bit unpack → γ1−z). Both use %/÷ (no bitwise on Int).
fn mldsa_sampler2_defs() -> String {
    let mut p = String::new();
    p.push_str("## To mldsaRejBoundedLogos (seed: Seq of Int) and (rc: Seq of Word64) and (rsrc: Seq of Int) and (roff: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let mutable blk be a new Seq of Int.\n    Repeat for x in seed:\n        Push x to blk.\n    Push 31 to blk.\n    While (length of blk) is less than 136:\n        Push 0 to blk.\n    Set item 136 of blk to (item 136 of blk) + 128.\n");
    p.push_str("    Let mutable state be keccakAbsorbBlock(blk, 17, rc, rsrc, roff).\n    Let mutable buf be keccakExtractRate(state, 17).\n");
    p.push_str("    Let mutable a be a new Seq of Int.\n    Let mutable ctr be 0.\n    Let mutable pos be 0.\n    While ctr is less than 256:\n");
    p.push_str("        If pos is at least 136:\n            Set state to keccakF(state, rc, rsrc, roff).\n            Set buf to keccakExtractRate(state, 17).\n            Set pos to 0.\n");
    p.push_str("        Let b be item (pos + 1) of buf.\n        Set pos to pos + 1.\n        Let t0 be b % 16.\n        Let t1 be b / 16.\n        If t0 is less than 9:\n            Push (4 - t0) to a.\n            Set ctr to ctr + 1.\n        If t1 is less than 9:\n            If ctr is less than 256:\n                Push (4 - t1) to a.\n                Set ctr to ctr + 1.\n    Return a.\n\n");
    p.push_str("## To mldsaExpandMaskLogos (seed: Seq of Int) and (rc: Seq of Word64) and (rsrc: Seq of Int) and (roff: Seq of Int) -> Seq of Int:\n");
    p.push_str("    Let buf be keccakSponge(seed, rc, rsrc, roff, 136, 31, 640).\n    Let mutable a be a new Seq of Int.\n    Repeat for i from 0 to 127:\n        Let o be 5 * i.\n        Let b0 be item (o + 1) of buf.\n        Let b1 be item (o + 2) of buf.\n        Let b2 be item (o + 3) of buf.\n        Let b3 be item (o + 4) of buf.\n        Let b4 be item (o + 5) of buf.\n        Let z0 be b0 + 256 * b1 + 65536 * (b2 % 16).\n        Let z1 be (b2 / 16) + 16 * b3 + 4096 * b4.\n        Push (524288 - z0) to a.\n        Push (524288 - z1) to a.\n    Return a.\n\n");
    p
}
fn mldsa_all_sampler_defs() -> String {
    format!("{}{}{}{}", keccak_defs(), keccak_sponge_def(), keccak_helper_defs(), mldsa_sampler2_defs())
}
fn mldsa2_logos_program(callee: &str, seed: &[i64]) -> String {
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let rc_i64: Vec<i64> = KECCAK_RC.iter().map(|&x| x as i64).collect();
    let rc_s = rc_i64.iter().map(|x| format!("word64({})", x)).collect::<Vec<_>>().join(", ");
    format!(
        "{}## Main\nLet seed be [{}].\nLet rc be [{}].\nLet rsrc be [{}].\nLet roff be [{}].\nRepeat for x in {callee}(seed, rc, rsrc, roff):\n    Show x.\n",
        mldsa_all_sampler_defs(), lit(seed), rc_s, lit(&KECCAK_RSRC), lit(&KECCAK_ROFF))
}
// FIPS-204 spec oracles over pub shake256_bytes (expand_mask IS shake256_bytes natively → exact).
fn ref_rej_bounded(seed: &[u8]) -> Vec<String> {
    let stream = logicaffeine_system::keccak::shake256_bytes(seed, 136 * 10);
    let (mut a, mut pos) = (Vec::<String>::with_capacity(256), 0usize);
    while a.len() < 256 {
        let b = stream[pos]; pos += 1;
        let (t0, t1) = ((b & 15) as i32, (b >> 4) as i32);
        if t0 < 9 { a.push((4 - t0).to_string()); }
        if t1 < 9 && a.len() < 256 { a.push((4 - t1).to_string()); }
    }
    a
}
fn ref_expand_mask(seed: &[u8]) -> Vec<String> {
    let buf = logicaffeine_system::keccak::shake256_bytes(seed, 128 * 5);
    let mut a = vec![0i64; 256];
    for i in 0..128 {
        let o = 5 * i;
        let z0 = (buf[o] as u32) | ((buf[o + 1] as u32) << 8) | (((buf[o + 2] as u32) & 0xf) << 16);
        let z1 = ((buf[o + 2] as u32) >> 4) | ((buf[o + 3] as u32) << 4) | ((buf[o + 4] as u32) << 12);
        a[2 * i] = 524288 - z0 as i64;
        a[2 * i + 1] = 524288 - z1 as i64;
    }
    a.iter().map(|x| x.to_string()).collect()
}

#[test]
fn mldsa_secret_samplers_logos_match_fips204_tw_and_vm() {
    let bnd_seed: Vec<i64> = (0..66).map(|i| ((i * 17 + 9) % 256) as i64).collect();
    let msk_seed: Vec<i64> = (0..66).map(|i| ((i * 19 + 3) % 256) as i64).collect();
    let to_u8 = |v: &[i64]| v.iter().map(|&b| b as u8).collect::<Vec<u8>>();
    let cases: [(&str, &[i64], Vec<String>); 2] = [
        ("mldsaRejBoundedLogos", &bnd_seed, ref_rej_bounded(&to_u8(&bnd_seed))),
        ("mldsaExpandMaskLogos", &msk_seed, ref_expand_mask(&to_u8(&msk_seed))),
    ];
    for (my_fn, seed, want) in cases {
        assert_eq!(want.len(), 256, "FIPS ref for {my_fn} must yield 256 values");
        let prog = mldsa2_logos_program(my_fn, seed);
        for (tag, out) in [("tw", tw_outcome(&prog)), ("vm", vm_outcome(&prog))] {
            assert!(out.error.is_none(), "{my_fn}/{tag} errored: {:?}", out.error);
            let got = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
            assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "{my_fn}/{tag} == FIPS-204 spec");
        }
    }
}

#[test]
#[ignore = "compiles via rustc (slow) — ML-DSA SampleBounded + ExpandMask in Logos AOT == FIPS-204"]
fn mldsa_secret_samplers_logos_aot_eq_fips204() {
    let bnd_seed: Vec<i64> = (0..66).map(|i| ((i * 17 + 9) % 256) as i64).collect();
    let msk_seed: Vec<i64> = (0..66).map(|i| ((i * 19 + 3) % 256) as i64).collect();
    let to_u8 = |v: &[i64]| v.iter().map(|&b| b as u8).collect::<Vec<u8>>();
    let cases: [(&str, &[i64], Vec<String>); 2] = [
        ("mldsaRejBoundedLogos", &bnd_seed, ref_rej_bounded(&to_u8(&bnd_seed))),
        ("mldsaExpandMaskLogos", &msk_seed, ref_expand_mask(&to_u8(&msk_seed))),
    ];
    for (my_fn, seed, want) in cases {
        let prog = mldsa2_logos_program(my_fn, seed);
        let aot = run_logos_with_args(&prog, &[]);
        assert!(aot.success, "AOT {my_fn} compile/run failed:\n{}", aot.stderr);
        let got = aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
        assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "AOT {my_fn} == FIPS-204 spec");
    }
}

// ML-KEM noise sampler FULLY in Logos: PRF = SHAKE256(seed‖nonce, 128) then CBD2 → the secret/error
// polynomials s,e (FIPS-203 SamplePolyCBD_η with η=2). Composes the proven Logos SHAKE256 + Logos CBD2.
fn mlkem_cbd_min_defs() -> String {
    let mut p = String::new();
    p.push_str("## To mlkemBit (buf: Seq of Int) and (k: Int) -> Int:\n    Let byte be item (k / 8 + 1) of buf.\n    Let r be k % 8.\n    Let mutable pw be 1.\n    Repeat for i from 1 to r:\n        Set pw to pw * 2.\n    Return (byte / pw) % 2.\n\n");
    p.push_str("## To cbd2W16Logos (buf: Seq of Int) -> Seq of Word16:\n    Let mutable out be a new Seq of Word16.\n    Repeat for c from 0 to 255:\n        Let a be mlkemBit(buf, 4 * c) + mlkemBit(buf, 4 * c + 1).\n        Let b be mlkemBit(buf, 4 * c + 2) + mlkemBit(buf, 4 * c + 3).\n        Let v be a - b.\n        Push word16(((v % 3329) + 3329) % 3329) to out.\n    Return out.\n\n");
    p
}
fn mlkem_noise_defs() -> String {
    let mut p = String::new();
    p.push_str("## To mlkemNoiseCbdLogos (seed: Seq of Int) and (nonce: Int) and (rc: Seq of Word64) and (rsrc: Seq of Int) and (roff: Seq of Int) -> Seq of Word16:\n");
    p.push_str("    Let mutable pin be a new Seq of Int.\n    Repeat for x in seed:\n        Push x to pin.\n    Push nonce to pin.\n    Return cbd2W16Logos(keccakSponge(pin, rc, rsrc, roff, 136, 31, 128)).\n\n");
    p
}
fn mlkem_noise_program(seed: &[i64], nonce: i64) -> String {
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let rc_i64: Vec<i64> = KECCAK_RC.iter().map(|&x| x as i64).collect();
    let rc_s = rc_i64.iter().map(|x| format!("word64({})", x)).collect::<Vec<_>>().join(", ");
    format!(
        "{}{}{}{}## Main\nLet seed be [{}].\nLet rc be [{}].\nLet rsrc be [{}].\nLet roff be [{}].\nRepeat for x in mlkemNoiseCbdLogos(seed, {nonce}, rc, rsrc, roff):\n    Show intOfWord16(x).\n",
        keccak_defs(), keccak_sponge_def(), mlkem_cbd_min_defs(), mlkem_noise_defs(),
        lit(seed), rc_s, lit(&KECCAK_RSRC), lit(&KECCAK_ROFF))
}
fn ref_mlkem_noise(seed: &[u8], nonce: u8) -> Vec<String> {
    let mut pin = seed.to_vec();
    pin.push(nonce);
    let buf = logicaffeine_system::keccak::shake256_bytes(&pin, 128);
    let bit = |k: usize| ((buf[k / 8] >> (k % 8)) & 1) as i64;
    (0..256).map(|c| {
        let a = bit(4 * c) + bit(4 * c + 1);
        let b = bit(4 * c + 2) + bit(4 * c + 3);
        (((a - b) % 3329 + 3329) % 3329).to_string()
    }).collect()
}

#[test]
fn mlkem_noise_cbd_logos_matches_native_tw_and_vm() {
    let seed: Vec<i64> = (0..32).map(|i| ((i * 23 + 5) % 256) as i64).collect();
    let seed_u8: Vec<u8> = seed.iter().map(|&b| b as u8).collect();
    for nonce in [0i64, 1, 7, 255] {
        let want = ref_mlkem_noise(&seed_u8, nonce as u8);
        assert_eq!(want.len(), 256);
        let prog = mlkem_noise_program(&seed, nonce);
        for (tag, out) in [("tw", tw_outcome(&prog)), ("vm", vm_outcome(&prog))] {
            assert!(out.error.is_none(), "noise nonce={nonce}/{tag} errored: {:?}", out.error);
            let got = out.output.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
            assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "mlkemNoiseCbdLogos nonce={nonce}/{tag} == native");
        }
    }
}

#[test]
#[ignore = "compiles via rustc (slow) — ML-KEM PRF+CBD noise sampler in Logos AOT == native"]
fn mlkem_noise_cbd_logos_aot_eq_native() {
    let seed: Vec<i64> = (0..32).map(|i| ((i * 23 + 5) % 256) as i64).collect();
    let seed_u8: Vec<u8> = seed.iter().map(|&b| b as u8).collect();
    for nonce in [0i64, 3] {
        let want = ref_mlkem_noise(&seed_u8, nonce as u8);
        let aot = run_logos_with_args(&mlkem_noise_program(&seed, nonce), &[]);
        assert!(aot.success, "AOT noise nonce={nonce} failed:\n{}", aot.stderr);
        let got = aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
        assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "AOT mlkemNoiseCbdLogos nonce={nonce} == native");
    }
}

// ───────── Fast 4-way Keccak-f[1600] written in LOGOS lanes (Lanes4Word64 → AVX2 under +avx2) ─────────
fn keccak_x4_defs() -> String {
    let mut p = String::new();
    p.push_str("## To keccakF4Logos (st: Seq of Lanes4Word64) and (rc: Seq of Int) and (rsrc: Seq of Int) and (roff: Seq of Int) -> Seq of Lanes4Word64:\n");
    p.push_str("    Let mutable a be a new Seq of Lanes4Word64.\n    Repeat for x in st:\n        Push x to a.\n");
    p.push_str("    Repeat for rnd from 1 to 24:\n");
    p.push_str("        Let mutable c be a new Seq of Lanes4Word64.\n        Repeat for x from 0 to 4:\n            Push ((item (x + 1) of a) xor (item (x + 6) of a) xor (item (x + 11) of a) xor (item (x + 16) of a) xor (item (x + 21) of a)) to c.\n");
    p.push_str("        Repeat for x from 0 to 4:\n            Let d be (item ((x + 4) % 5 + 1) of c) xor rotl(item ((x + 1) % 5 + 1) of c, 1).\n            Repeat for y from 0 to 4:\n                Set item (5 * y + x + 1) of a to (item (5 * y + x + 1) of a) xor d.\n");
    p.push_str("        Let mutable b be a new Seq of Lanes4Word64.\n        Repeat for k from 0 to 24:\n            Push rotl(item ((item (k + 1) of rsrc) + 1) of a, item (k + 1) of roff) to b.\n");
    p.push_str("        Repeat for y from 0 to 4:\n            Repeat for x from 0 to 4:\n                Let bb0 be item (5 * y + x + 1) of b.\n                Let bb1 be item (5 * y + (x + 1) % 5 + 1) of b.\n                Let bb2 be item (5 * y + (x + 2) % 5 + 1) of b.\n                Set item (5 * y + x + 1) of a to bb0 xor andNot4(bb1, bb2).\n");
    p.push_str("        Set item 1 of a to (item 1 of a) xor splat4Word64(word64(item rnd of rc)).\n");
    p.push_str("    Return a.\n\n");
    p
}

fn keccak_x4_program(flat: &[i64]) -> String {
    let lit = |v: &[i64]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let rc_i64: Vec<i64> = KECCAK_RC.iter().map(|&x| x as i64).collect();
    format!(
        "{}## Main\nLet flat be [{}].\nLet rc be [{}].\nLet rsrc be [{}].\nLet roff be [{}].\n\
         Let mutable state be a new Seq of Lanes4Word64.\n\
         Repeat for i from 0 to 24:\n    Let base be 4 * i.\n    Let mutable sub be a new Seq of Int.\n    Push item (base + 1) of flat to sub.\n    Push item (base + 2) of flat to sub.\n    Push item (base + 3) of flat to sub.\n    Push item (base + 4) of flat to sub.\n    Push lanes4Word64(sub) to state.\n\
         Repeat for v in keccakF4Logos(state, rc, rsrc, roff):\n    Repeat for x in seqOfLanes4(v):\n        Show x.\n",
        keccak_x4_defs(), lit(flat), lit(&rc_i64), lit(&KECCAK_RSRC), lit(&KECCAK_ROFF))
}

#[test]
#[ignore = "compiles via rustc (slow) — 4-way Keccak-f[1600] in Logos lanes AOT == 4× native keccak_f1600"]
fn keccak_f1600_x4_logos_aot_eq_native() {
    // Four independent states; the 4-way permutation's lane s must equal one scalar keccak_f1600(state s).
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut rnd = || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); seed };
    let states: [[u64; 25]; 4] = std::array::from_fn(|_| std::array::from_fn(|_| rnd()));
    // reference: permute each state, then interleave lane-major (lane i → [s0,s1,s2,s3][i]).
    let mut perm = states;
    for s in perm.iter_mut() {
        logicaffeine_system::keccak::keccak_f1600(s);
    }
    let want: Vec<String> = (0..25).flat_map(|i| (0..4).map(move |s| (perm[s][i] as i64).to_string())).collect();
    // Logos input: flat lane-major (lane i's four states), as i64 (u64 bits).
    let flat: Vec<i64> = (0..25).flat_map(|i| (0..4).map(move |s| states[s][i] as i64)).collect();
    let aot = run_logos_with_args(&keccak_x4_program(&flat), &[]);
    assert!(aot.success, "AOT compile/run failed:\n{}", aot.stderr);
    let got = aot.stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>();
    assert_eq!(got, want.iter().map(|s| s.as_str()).collect::<Vec<_>>(), "4-way Keccak in Logos lanes (AOT) == 4× native keccak_f1600");
}
