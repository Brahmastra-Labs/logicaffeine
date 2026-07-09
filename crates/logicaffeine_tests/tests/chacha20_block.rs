//! The full ChaCha20 block function (`chacha20Block`) ships as a DEMAND-IMPORTED LOGOS function
//! in `assets/std/crypto.lg`, built purely on `Word32` wrapping arithmetic + the `quarterRound`.
//! Its 16-word output must equal the RFC 8439 §2.3.2 test vector, bit-for-bit, on the tree-walker
//! and the bytecode VM — proving the shipped stdlib ChaCha20 core is correct end to end.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

/// Reference ChaCha20 quarter-round over native `u32` (the validated oracle primitive).
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

/// Reference ChaCha20 block (RFC 8439 §2.3) — the oracle the Logos function must match.
fn chacha20_block(key: &[u32; 8], counter: u32, nonce: &[u32; 3]) -> [u32; 16] {
    let mut state = [0u32; 16];
    state[0] = 0x6170_7865;
    state[1] = 0x3320_646e;
    state[2] = 0x7962_2d32;
    state[3] = 0x6b20_6574;
    state[4..12].copy_from_slice(key);
    state[12] = counter;
    state[13..16].copy_from_slice(nonce);
    let mut w = state;
    for _ in 0..10 {
        qr(&mut w, 0, 4, 8, 12);
        qr(&mut w, 1, 5, 9, 13);
        qr(&mut w, 2, 6, 10, 14);
        qr(&mut w, 3, 7, 11, 15);
        qr(&mut w, 0, 5, 10, 15);
        qr(&mut w, 1, 6, 11, 12);
        qr(&mut w, 2, 7, 8, 13);
        qr(&mut w, 3, 4, 9, 14);
    }
    let mut out = [0u32; 16];
    for i in 0..16 {
        out[i] = w[i].wrapping_add(state[i]);
    }
    out
}

/// Pack 4 little-endian bytes into a u32.
fn le_words<const N: usize>(bytes: &[u8]) -> [u32; N] {
    let mut out = [0u32; N];
    for (i, w) in out.iter_mut().enumerate() {
        *w = u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap());
    }
    out
}

// RFC 8439 §2.3.2 fixed inputs.
fn rfc_inputs() -> ([u32; 8], u32, [u32; 3]) {
    let key_bytes: Vec<u8> = (0u8..32).collect();
    let nonce_bytes: [u8; 12] = [0, 0, 0, 0x09, 0, 0, 0, 0x4a, 0, 0, 0, 0];
    (le_words::<8>(&key_bytes), 1, le_words::<3>(&nonce_bytes))
}

fn program(key: &[u32; 8], counter: u32, nonce: &[u32; 3]) -> String {
    let kw: Vec<String> = key.iter().map(|w| format!("word32({w})")).collect();
    let nw: Vec<String> = nonce.iter().map(|w| format!("word32({w})")).collect();
    format!(
        "## Main\n\
         Let key be [{}].\n\
         Let nonce be [{}].\n\
         Let block be chacha20Block(key, word32({counter}), nonce).\n\
         Repeat for i from 1 to 16:\n    Show item i of block.\n",
        kw.join(", "),
        nw.join(", "),
    )
}

#[test]
fn chacha20_block_matches_rfc8439_2_3_2() {
    let (key, counter, nonce) = rfc_inputs();
    let oracle = chacha20_block(&key, counter, &nonce);

    // Anchor the oracle to the RFC's published §2.3.2 output words (no hand math).
    assert_eq!(
        oracle,
        [
            0xe4e7_f110, 0x1559_3bd1, 0x1fdd_0f50, 0xc471_20a3, 0xc7f4_d1c7, 0x0368_c033,
            0x9aaa_2204, 0x4e6c_d4c3, 0x4664_82d2, 0x09aa_9f07, 0x05d7_c214, 0xa202_8bd9,
            0xd19c_12b5, 0xb94e_16de, 0xe883_d0cb, 0x4e3c_50a2,
        ],
        "oracle must match RFC 8439 §2.3.2"
    );

    let expected: String =
        oracle.iter().map(|w| w.to_string()).collect::<Vec<_>>().join("\n");
    let r = tw_outcome(&program(&key, counter, &nonce));
    assert_eq!(r.error, None, "stdlib chacha20Block compiles + runs: {:?}", r.error);
    assert_eq!(
        r.output.trim(),
        expected,
        "the demand-imported stdlib ChaCha20 block must equal the RFC 8439 §2.3.2 vector"
    );
}

#[test]
fn chacha20_block_tw_vm_byte_identical() {
    fn norm(s: &str) -> Vec<String> {
        s.lines().map(|l| l.trim_end().to_string()).filter(|l| !l.is_empty()).collect()
    }
    let (key, counter, nonce) = rfc_inputs();
    let prog = program(&key, counter, &nonce);
    let tw = tw_outcome(&prog);
    let vm = vm_outcome(&prog);
    assert_eq!(tw.error, None, "tw clean: {:?}", tw.error);
    assert_eq!(vm.error, None, "vm clean: {:?}", vm.error);
    assert_eq!(norm(&tw.output), norm(&vm.output), "tw == vm for stdlib ChaCha20 block");
}

#[test]
#[ignore = "compiles a cargo project via rustc (slow) — the COMPILED stdlib ChaCha20-block gate"]
fn chacha20_block_aot_matches_treewalker() {
    fn norm(s: &str) -> String {
        s.lines().map(|l| l.trim_end()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
    }
    let (key, counter, nonce) = rfc_inputs();
    let prog = program(&key, counter, &nonce);
    let tw = tw_outcome(&prog);
    assert_eq!(tw.error, None, "tw clean: {:?}", tw.error);
    let aot = common::run_logos_with_args(&prog, &[]);
    assert!(aot.success, "AOT failed:\n{}\n{}", aot.stderr, aot.rust_code);
    assert_eq!(norm(&aot.stdout), norm(&tw.output), "COMPILED ChaCha20 block == tree-walker");
}
