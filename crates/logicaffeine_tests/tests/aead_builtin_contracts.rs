//! `sealBytes`, `openBytes`, `sealFrame` and `openFrame` take their keys, nonces and byte strings at
//! their word: an item that is not a byte (0 to 255) is refused with the builtin's name, the argument
//! and the item, never folded into the byte its low eight bits spell. A key `[256, …]` is not the key
//! `[0, …]`, and a plaintext `[300]` is not `[44]`.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};

fn bytes(n: usize, k: usize) -> String {
    (0..n)
        .map(|i| ((i * k + 3) % 256).to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn with_item(n: usize, k: usize, at: usize, x: i64) -> String {
    (0..n)
        .map(|i| {
            if i + 1 == at {
                x.to_string()
            } else {
                ((i * k + 3) % 256).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `main` stops with exactly `reason` on the tree-walker and the VM.
fn fails_with(main: &str, reason: &str) {
    let src = format!("## Main\n{main}");
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
fn a_key_or_nonce_of_items_that_are_not_bytes_is_refused() {
    fails_with(
        &format!(
            "Show sealBytes([{}], [{}], [1]).\n",
            with_item(32, 7, 1, 256),
            bytes(12, 5)
        ),
        "sealBytes(): the key must be bytes (0 to 255); item 1 is 256",
    );
    fails_with(
        &format!(
            "Show sealFrame([{}], [{}], 5).\n",
            bytes(32, 7),
            with_item(12, 5, 12, -1)
        ),
        "sealFrame(): the nonce must be bytes (0 to 255); item 12 is -1",
    );
    fails_with(
        &format!(
            "Show openBytes([{}], [{}], [{}]).\n",
            with_item(32, 7, 9, 1000),
            bytes(12, 5),
            bytes(20, 3)
        ),
        "openBytes(): the key must be bytes (0 to 255); item 9 is 1000",
    );
    fails_with(
        &format!(
            "Show openFrame([{}], [{}], [{}]).\n",
            bytes(32, 7),
            with_item(12, 5, 3, 300),
            bytes(20, 3)
        ),
        "openFrame(): the nonce must be bytes (0 to 255); item 3 is 300",
    );
}

#[test]
fn a_plaintext_or_sealed_frame_of_items_that_are_not_bytes_is_refused() {
    fails_with(
        &format!(
            "Show sealBytes([{}], [{}], [1, 2, 300]).\n",
            bytes(32, 7),
            bytes(12, 5)
        ),
        "sealBytes(): the plaintext must be bytes (0 to 255); item 3 is 300",
    );
    fails_with(
        &format!(
            "Show openBytes([{}], [{}], [{}]).\n",
            bytes(32, 7),
            bytes(12, 5),
            with_item(20, 3, 17, -9)
        ),
        "openBytes(): the sealed frame must be bytes (0 to 255); item 17 is -9",
    );
    fails_with(
        &format!(
            "Show openFrame([{}], [{}], [{}]).\n",
            bytes(32, 7),
            bytes(12, 5),
            with_item(20, 3, 1, 256)
        ),
        "openFrame(): the sealed frame must be bytes (0 to 255); item 1 is 256",
    );
}

#[test]
fn a_key_or_nonce_of_the_wrong_length_is_still_refused_by_its_length() {
    fails_with(
        &format!(
            "Show sealBytes([{}], [{}], [1]).\n",
            bytes(31, 7),
            bytes(12, 5)
        ),
        "sealBytes(): key must be exactly 32 bytes, got 31",
    );
    fails_with(
        &format!(
            "Show sealFrame([{}], [{}], 1).\n",
            bytes(32, 7),
            bytes(13, 5)
        ),
        "sealFrame(): nonce must be exactly 12 bytes, got 13",
    );
}
