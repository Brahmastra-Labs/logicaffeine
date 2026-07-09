//! Phase 9c — large buffers stage through OPFS, not just linear memory.
//!
//! A browser tab's linear memory is precious; large payloads (mounted files,
//! synced CRDT snapshots) belong in the Origin Private File System. This headless
//! test writes a multi-megabyte buffer through the OPFS VFS and reads it back
//! byte-exact, proving the staging path works end to end in a real browser.
//!
//! Runs only under `wasm-pack test --headless` with the `persistence` feature;
//! inert on a normal `cargo test`.

#![cfg(all(target_arch = "wasm32", feature = "persistence"))]

use logicaffeine_system::fs::{OpfsVfs, Vfs};
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
async fn opfs_stages_large_buffer_roundtrip() {
    let vfs = OpfsVfs::new().await.expect("OPFS available in a secure context");

    // 2 MiB — large enough that "stage it, don't keep a copy in memory" matters.
    let big: Vec<u8> = (0..(2usize << 20)).map(|i| (i % 251) as u8).collect();
    vfs.write("/staged.bin", &big).await.expect("write to OPFS");

    let back = vfs.read("/staged.bin").await.expect("read from OPFS");
    assert_eq!(back.len(), big.len(), "the full 2 MiB round-trips through OPFS");
    assert_eq!(back, big, "OPFS preserves the staged bytes exactly");

    // And it persists addressably (a second handle sees it).
    let vfs2 = OpfsVfs::new().await.expect("re-open OPFS");
    assert!(vfs2.exists("/staged.bin").await.expect("exists check"), "staged buffer persists");
    vfs2.remove("/staged.bin").await.expect("cleanup");
}
