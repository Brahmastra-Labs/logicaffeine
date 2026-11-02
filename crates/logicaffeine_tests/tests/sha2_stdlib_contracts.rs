//! `crypto.lg`'s SHA-256, HMAC-SHA256 and PBKDF2-HMAC-SHA256 take byte strings and nothing else. SHA-256
//! packs four bytes into a word as `b0·2^24 + b1·2^16 + b2·2^8 + b3`, so an item past 255 carries into its
//! neighbour: `[0, 256]` would hash as `[1, 0]` and two different messages share one digest. PBKDF2 runs a
//! positive count of iterations (RFC 8018 §5.2), and the tree-walker and the VM run it on a native
//! accelerator that counts in 32 bits: a count of 2^32 would have become no iterations at all. Each
//! function refuses what it cannot take with its name and the reason, identically on every tier, the
//! accelerated ones included.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::run_all_tiers;

use std::process::Command;

fn fails_with(main: &str, reason: &str) {
    let src = format!("## Main\n{main}");
    let r = run_all_tiers(&src);
    assert_eq!(
        r.error.as_deref(),
        Some(reason),
        "every tier stops with the function's reason\n--- program ---\n{src}\n--- output ---\n{}",
        r.output
    );
}

fn answers(main: &str, expected: &str) {
    let src = format!("## Main\n{main}");
    let r = run_all_tiers(&src);
    assert_eq!(r.error, None, "the program runs\n{src}");
    assert_eq!(r.output.trim(), expected, "\n{src}");
}

/// The C tier, which compiles `crypto.lg` itself rather than accelerating it, stops with `reason`.
fn c_fails_with(main: &str, reason: &str) {
    if !common::require_tool(&common::c_compiler()) {
        return;
    }
    let src = format!("## Main\n{main}");
    let c = logicaffeine_compile::compile::compile_to_c(&src).expect("the program compiles to C");
    let dir = tempfile::Builder::new()
        .prefix("logos_sha2_contract_")
        .tempdir()
        .expect("a run directory");
    let (file, bin) = (dir.path().join("prog.c"), dir.path().join("prog"));
    std::fs::write(&file, &c).expect("write the C");
    let built = common::build_c_release(&file, &bin).expect("the C compiler starts");
    assert!(
        built.status.success(),
        "the C builds\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let run = Command::new(&bin).output().expect("the program starts");
    assert_eq!(run.status.code(), Some(1), "the C program fails\n{src}");
    assert_eq!(
        String::from_utf8_lossy(&run.stderr).trim_end(),
        reason,
        "the C program stops with the function's reason\n{src}"
    );
}

#[test]
fn sha256_takes_bytes_and_names_the_first_item_that_is_not_one() {
    fails_with(
        "Show sha256([0, 256]).\n",
        "sha256(): the message must be bytes (0 to 255); item 2 is 256",
    );
    fails_with(
        "Show sha256([97, 98, -1]).\n",
        "sha256(): the message must be bytes (0 to 255); item 3 is -1",
    );
    c_fails_with(
        "Show sha256([0, 256]).\n",
        "sha256(): the message must be bytes (0 to 255); item 2 is 256",
    );
}

#[test]
fn hmac_sha256_takes_a_byte_key_and_a_byte_message() {
    fails_with(
        "Show hmacSha256([1, 2, 300], [97]).\n",
        "hmacSha256(): the key must be bytes (0 to 255); item 3 is 300",
    );
    fails_with(
        "Let mutable key be a new Seq of Int.\nRepeat for i from 1 to 70:\n    Push i to key.\nPush 256 to key.\nShow hmacSha256(key, [97]).\n",
        "hmacSha256(): the key must be bytes (0 to 255); item 71 is 256",
    );
    fails_with(
        "Show hmacSha256([1, 2, 3], [97, -2]).\n",
        "hmacSha256(): the message must be bytes (0 to 255); item 2 is -2",
    );
    c_fails_with(
        "Show hmacSha256([1, 2, 300], [97]).\n",
        "hmacSha256(): the key must be bytes (0 to 255); item 3 is 300",
    );
}

#[test]
fn pbkdf2_sha256_takes_a_byte_password_and_a_byte_salt() {
    fails_with(
        "Show pbkdf2Sha256([256], [1, 2, 3, 4], 1).\n",
        "pbkdf2Sha256(): the password must be bytes (0 to 255); item 1 is 256",
    );
    fails_with(
        "Show pbkdf2Sha256(text_bytes(\"pencil\"), [1, 2, 3, -4], 1).\n",
        "pbkdf2Sha256(): the salt must be bytes (0 to 255); item 4 is -4",
    );
    c_fails_with(
        "Show pbkdf2Sha256(text_bytes(\"pencil\"), [1, 2, 3, -4], 1).\n",
        "pbkdf2Sha256(): the salt must be bytes (0 to 255); item 4 is -4",
    );
}

#[test]
fn pbkdf2_sha256_runs_a_positive_count_its_accelerator_can_count() {
    for iters in [0i64, -1, 4_294_967_296, 4_294_967_297, i64::MAX] {
        fails_with(
            &format!("Show pbkdf2Sha256(text_bytes(\"pencil\"), [1, 2, 3, 4], {iters}).\n"),
            &format!(
                "pbkdf2Sha256(): the iteration count must be from 1 to 4294967295; it is {iters}"
            ),
        );
    }
    c_fails_with(
        "Show pbkdf2Sha256(text_bytes(\"pencil\"), [1, 2, 3, 4], 0).\n",
        "pbkdf2Sha256(): the iteration count must be from 1 to 4294967295; it is 0",
    );
}

#[test]
fn in_contract_arguments_still_hash_to_the_official_vectors() {
    answers(
        "Show bytesToHex(sha256(text_bytes(\"abc\"))).\n",
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    );
    answers(
        "Show bytesToHex(hmacSha256(text_bytes(\"Jefe\"), text_bytes(\"what do ya want for nothing?\"))).\n",
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843",
    );
    answers(
        "Show bytesToHex(pbkdf2Sha256(text_bytes(\"password\"), text_bytes(\"salt\"), 2)).\n",
        "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43",
    );
}
