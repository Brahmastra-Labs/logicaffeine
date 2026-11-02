//! `sealBox(key, bytes)` / `openBox(key, box)` and `sealValueBox(key, value)` / `openValueBox(key, box)`
//! seal under a nonce nobody chooses. `sealBytes(key, nonce, …)` takes its caller's 12-byte nonce, and
//! one repeated under a key gives away the XOR of two plaintexts and the Poly1305 key that forges. A
//! box draws a 24-byte XChaCha20-Poly1305 nonce from the system's entropy for every seal and carries
//! it in front: `nonce(24) ‖ ciphertext ‖ tag(16)`. 192 random bits never repeat, so one key seals any
//! number of boxes. A byte box and a value box are sealed under different associated data, so neither
//! opens as the other.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::run_all_tiers;

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

const KEY: &str = "[7, 14, 21, 28, 35, 42, 49, 56, 63, 70, 77, 84, 91, 98, 105, 112, 119, 126, 133, 140, 147, 154, 161, 168, 175, 182, 189, 196, 203, 210, 217, 224]";
const OTHER_KEY: &str = "[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32]";

/// `main` runs cleanly on every tier and prints exactly `expected`.
fn answers(main: &str, expected: &str) {
    let src = format!("## Main\nLet key be {KEY}.\nLet otherKey be {OTHER_KEY}.\n{main}");
    let r = run_all_tiers(&src);
    assert_eq!(r.error, None, "the program runs\n{src}");
    assert_eq!(r.output.trim(), expected.trim(), "\n{src}");
}

/// `main` stops with exactly `reason` on the tree-walker and the VM.
fn fails_with(main: &str, reason: &str) {
    let src = format!("## Main\nLet key be {KEY}.\nLet otherKey be {OTHER_KEY}.\n{main}");
    for (tier, r) in [("tw", tw_outcome(&src)), ("vm", vm_outcome(&src))] {
        assert_eq!(
            r.error.as_deref(),
            Some(reason),
            "[{tier}]\n{src}\n--- output ---\n{}",
            r.output
        );
    }
}

#[test]
fn a_box_opens_to_its_bytes() {
    answers(
        "Let empty be a new Seq of Int.\n\
         Show openBox(key, sealBox(key, [104, 105])).\n\
         Show length of openBox(key, sealBox(key, empty)).\n\
         Show openBox(key, sealBox(key, [0, 255, 128, 1])).\n",
        "[104, 105]\n0\n[0, 255, 128, 1]",
    );
}

#[test]
fn a_box_is_its_nonce_its_ciphertext_and_its_tag() {
    answers(
        "Let empty be a new Seq of Int.\n\
         Show length of sealBox(key, empty).\n\
         Show length of sealBox(key, [1, 2, 3, 4, 5]).\n",
        "40\n45",
    );
}

#[test]
fn every_seal_draws_a_fresh_nonce_even_when_the_optimizer_sees_two_alike() {
    answers(
        "Let a be sealBox(key, [1, 2, 3]).\n\
         Let b be sealBox(key, [1, 2, 3]).\n\
         Show a is equal to b.\n\
         Show bytesToHex(a) is equal to bytesToHex(b).\n\
         Show openBox(key, a) is equal to openBox(key, b).\n",
        "false\nfalse\ntrue",
    );
}

#[test]
fn a_thousand_boxes_under_one_key_draw_a_thousand_nonces() {
    let src = format!(
        "## Main\nLet key be {KEY}.\nLet mutable seen be a new Set of Text.\n\
         Repeat for i from 1 to 1000:\n    Let box be sealBox(key, [i % 256]).\n    \
         Let mutable nonce be a new Seq of Int.\n    Repeat for j from 1 to 24:\n        Push item j of box to nonce.\n    \
         Add bytesToHex(nonce) to seen.\nShow length of seen.\n"
    );
    for (tier, r) in [("tw", tw_outcome(&src)), ("vm", vm_outcome(&src))] {
        assert_eq!(r.error, None, "[{tier}] the program runs");
        assert_eq!(r.output.trim(), "1000", "[{tier}] no nonce repeats");
    }
}

#[test]
fn a_box_that_does_not_authenticate_is_refused() {
    let refused =
        "openBox(): authentication failed — the box was tampered, truncated, or the key is wrong";
    fails_with(
        "Show openBox(otherKey, sealBox(key, [1, 2, 3])).\n",
        refused,
    );
    fails_with(
        "Let mutable box be sealBox(key, [1, 2, 3]).\nSet item 30 of box to (item 30 of box + 1) % 256.\nShow openBox(key, box).\n",
        refused,
    );
    fails_with(
        "Let mutable box be sealBox(key, [1, 2, 3]).\nSet item 1 of box to (item 1 of box + 1) % 256.\nShow openBox(key, box).\n",
        refused,
    );
    fails_with(
        "Show openBox(key, [1, 2, 3]).\n",
        "openBox(): the box is 3 bytes; a box is at least 40",
    );
}

#[test]
fn a_value_box_opens_to_its_value() {
    let src = format!(
        "## Main\nLet key be {KEY}.\nLet roster be [\"alice\", \"bob\", \"carol\"].\n\
         Show roster.\nShow openValueBox(key, sealValueBox(key, roster)).\n\
         Show 42.\nShow openValueBox(key, sealValueBox(key, 42)).\n"
    );
    let r = run_all_tiers(&src);
    assert_eq!(r.error, None, "the program runs\n{src}");
    let lines: Vec<&str> = r.output.lines().collect();
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert_eq!(lines[0], lines[1], "the roster comes back out of its box");
    assert_eq!(lines[2], lines[3], "the number comes back out of its box");
}

#[test]
fn a_byte_box_does_not_open_as_a_value_box_nor_the_reverse() {
    fails_with(
        "Show openValueBox(key, sealBox(key, wireBytes(42))).\n",
        "openValueBox(): authentication failed — the box was tampered, truncated, or the key is wrong",
    );
    fails_with(
        "Show openBox(key, sealValueBox(key, 42)).\n",
        "openBox(): authentication failed — the box was tampered, truncated, or the key is wrong",
    );
}

#[test]
fn a_box_takes_its_key_and_bytes_at_their_word() {
    fails_with(
        "Show sealBox([1, 2, 3], [1]).\n",
        "sealBox(): key must be exactly 32 bytes, got 3",
    );
    fails_with(
        &format!("Show sealBox({}, [1]).\n", KEY.replacen("[7,", "[256,", 1)),
        "sealBox(): the key must be bytes (0 to 255); item 1 is 256",
    );
    fails_with(
        "Show sealBox(key, [1, 300]).\n",
        "sealBox(): the plaintext must be bytes (0 to 255); item 2 is 300",
    );
    fails_with(
        "Show openBox(key, [1, -2, 3]).\n",
        "openBox(): the box must be bytes (0 to 255); item 2 is -2",
    );
    fails_with(
        "Show sealValueBox([9], 1).\n",
        "sealValueBox(): key must be exactly 32 bytes, got 1",
    );
}

#[test]
fn a_box_is_the_xchacha20_poly1305_seal_under_the_nonce_it_carries() {
    let key: [u8; 32] = std::array::from_fn(|i| ((i + 1) * 7) as u8);
    let src =
        format!("## Main\nLet key be {KEY}.\nShow sealBox(key, [104, 101, 108, 108, 111]).\n");
    let r = vm_outcome(&src);
    assert_eq!(r.error, None);
    let shown = r
        .output
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let bytes: Vec<u8> = shown.split(", ").map(|b| b.parse().unwrap()).collect();
    let nonce: [u8; 24] = bytes[..24].try_into().unwrap();
    assert_eq!(
        bytes[24..],
        logicaffeine_system::aead::xchacha20poly1305_seal(
            &key,
            &nonce,
            b"logos box: bytes",
            b"hello"
        )[..],
        "the box seals under the label for bytes and the nonce in front of it"
    );
}
