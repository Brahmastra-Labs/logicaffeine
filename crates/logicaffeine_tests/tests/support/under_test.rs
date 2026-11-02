//! The binaries a suite spawns, found where they are when the suite RUNS. nextest names each binary
//! target's path in `CARGO_BIN_EXE_<name>` at run time — the place an archive run extracted it, which
//! the build never saw — and a plain `cargo test` leaves only the path the build put it at, so the
//! run-time name comes first. `test_bin_shim_lock` holds every suite to these.
#![allow(dead_code)]

use std::path::PathBuf;

/// `largo-under-test`: the shipped `largo`'s one call (`test_bin_shim_lock`).
pub fn largo() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_largo-under-test")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_largo-under-test")))
}

/// `logos-sat-under-test`: the shipped `logos-sat`'s one call.
pub fn logos_sat() -> PathBuf {
    std::env::var_os("CARGO_BIN_EXE_logos-sat-under-test")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_logos-sat-under-test")))
}
