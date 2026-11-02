//! The Verilog `logicaffeine-compile` embeds is sourced from the crate; the copies the FPGA flows
//! read in `benchmarks/silicon/` are goldens of it.
//!
//! Production code may embed only files its own crate ships (`packaging_lock`), so hand-written RTL
//! the compiler emits verbatim lives beside its emitter in `src/codegen_sva/`. The board flows,
//! testbenches and `scripts/verify-*.sh` expect the same files in `benchmarks/silicon/`, and there
//! they stay, as goldens byte-identical to what the crate emits. Regenerate with
//! `SILICON_RTL_RECORD=1 cargo nextest run -p logicaffeine-tests --test embedded_rtl_goldens_lock`.

use std::path::PathBuf;

use logicaffeine_compile::codegen_sva::{bignum_rtl, rtl, wprog_engine_rtl};
use logicaffeine_compile::spatial::lower::fabric_board;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn silicon_dir() -> PathBuf {
    repo_root().join("benchmarks/silicon")
}

fn crate_rtl_dir() -> PathBuf {
    repo_root().join("crates/logicaffeine_compile/src/codegen_sva")
}

/// Each golden and the emitter that is its source.
fn goldens() -> Vec<(&'static str, String)> {
    vec![
        ("fpu.v", wprog_engine_rtl::FPU_V.to_string()),
        ("ft245_uart_bridge.v", fabric_board::FT245_UART_BRIDGE_V.to_string()),
        ("shared_divider.v", rtl::emit_shared_divider_verilog()),
        ("tb_shared_divider.v", rtl::emit_shared_divider_testbench()),
        ("bigint_div.v", bignum_rtl::emit_bigint_div_verilog()),
        ("tb_bigint_div.v", bignum_rtl::emit_bigint_div_testbench()),
    ]
}

#[test]
fn every_benchmarks_copy_is_what_the_crate_emits() {
    let dir = silicon_dir();
    if std::env::var_os("SILICON_RTL_RECORD").is_some() {
        for (name, text) in goldens() {
            std::fs::write(dir.join(name), text).unwrap_or_else(|e| panic!("write {name}: {e}"));
        }
        return;
    }
    for (name, text) in goldens() {
        let have = std::fs::read_to_string(dir.join(name)).unwrap_or_else(|e| {
            panic!(
                "benchmarks/silicon/{name} is missing ({e}) — regenerate with SILICON_RTL_RECORD=1"
            )
        });
        assert!(
            have == text,
            "benchmarks/silicon/{name} differs from what logicaffeine-compile emits (the crate's copy in \
             src/codegen_sva/ is the source) — edit the crate's copy, then regenerate with SILICON_RTL_RECORD=1"
        );
    }
}

/// Any `.v` living beside the emitters that ALSO has a `benchmarks/silicon/` copy is the same file,
/// so a future move cannot leave two diverging sources.
#[test]
fn a_crate_rtl_file_and_its_benchmarks_copy_never_diverge() {
    let mut twins = 0;
    for entry in std::fs::read_dir(crate_rtl_dir())
        .expect("codegen_sva dir")
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_none_or(|x| x != "v") {
            continue;
        }
        let name = entry.file_name();
        let copy = silicon_dir().join(&name);
        if !copy.is_file() {
            continue;
        }
        twins += 1;
        let a = std::fs::read(&path).unwrap();
        let b = std::fs::read(&copy).unwrap();
        assert!(
            a == b,
            "src/codegen_sva/{0} and benchmarks/silicon/{0} have diverged — the crate's copy is the source; \
             regenerate the golden with SILICON_RTL_RECORD=1",
            name.to_string_lossy()
        );
    }
    assert!(
        twins >= goldens().len(),
        "found {twins} crate/benchmarks RTL twins, expected at least {}",
        goldens().len()
    );
}
