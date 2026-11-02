//! The RISC-V architecture tests (vendored under `fixtures/riscv-tests`) and the hart's own contract suite
//! (`fixtures/hart-contract`), assembled for the hart's ISA exactly as every hart suite runs them.
#![allow(dead_code)]

use std::path::PathBuf;

pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/riscv-tests")
}

/// The hart's own contract suite.
pub const CONTRACT: &str = "hart-contract";

pub fn suite_dir(suite: &str) -> PathBuf {
    if suite == CONTRACT {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(CONTRACT)
    } else {
        fixtures().join("isa").join(suite)
    }
}

/// Every test source of `suite`, sorted.
pub fn tests_of(suite: &str) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(suite_dir(suite))
        .unwrap_or_else(|e| panic!("{suite}: {e}"))
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".S").map(str::to_string))
        .collect();
    names.sort();
    names
}

/// Assemble and link one test for the hart's ISA; the ELF's bytes.
pub fn assemble(suite: &str, test: &str) -> Vec<u8> {
    let f = fixtures();
    let dir = std::env::temp_dir().join(format!("hart_isa_lock_{}_{suite}_{test}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let elf = dir.join("test.elf");
    let out = std::process::Command::new("riscv64-linux-gnu-gcc")
        .args([
            "-march=rv64imac_zicsr_zifencei",
            "-mabi=lp64",
            "-static",
            "-fno-pie",
            "-no-pie",
            "-mcmodel=medany",
            "-fvisibility=hidden",
            "-nostdlib",
            "-nostartfiles",
        ])
        .arg("-I")
        .arg(f.join("env/p"))
        .arg("-I")
        .arg(f.join("env"))
        .arg("-I")
        .arg(f.join("isa/macros/scalar"))
        .arg("-I")
        .arg(suite_dir(CONTRACT))
        .arg("-T")
        .arg(f.join("env/p/link.ld"))
        .arg(suite_dir(suite).join(format!("{test}.S")))
        .arg("-o")
        .arg(&elf)
        .output()
        .expect("run riscv64-linux-gnu-gcc");
    assert!(
        out.status.success(),
        "{suite}/{test} did not assemble:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(&elf).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    bytes
}

