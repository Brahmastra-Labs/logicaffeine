//! The machine-sympathy corpus: one LOGOS program per runnable claim of the CPU performance-engineering
//! microbenchmarks (`usamahz/cpu-performance-engineering`, `misc/benchmarks/NN-*`), each a kernel shape an
//! optimizing compiler is judged on: a data-dependent branch, a dependency chain against independent lanes, a
//! pointer chase, streaming maps of rising arithmetic intensity, a field of an array of records, an in-place
//! call, independent parallel counters, a first-touch fill, a line-per-`Show` program and a dense product.
//!
//! The kernels live in `fixtures/machine_sympathy/<name>.lg` and read their size from their first argument.
//! Every one runs at every size in [`SIZES`], so a vectorized or unrolled body meets each tail it can have.

use std::path::PathBuf;

use super::Program;

/// The sizes every kernel runs at: an empty and a short tail, the ragged sizes 9, 17 and 30 (remainders 1, 1
/// and 6 by eight lanes), and 64. The two float sums of `chain_vs_lanes_real` differ at 9, 30 and 64, so a
/// backend that reorders either of them prints a different number there.
pub const SIZES: &[&str] = &["1", "3", "9", "17", "30", "64"];

pub fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/machine_sympathy")
}

/// Every kernel, `(name, source)`, sorted by name.
pub fn kernels() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(dir())
        .expect("tests/fixtures/machine_sympathy")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "lg"))
        .map(|p| {
            let name = p
                .file_stem()
                .expect("a kernel's name")
                .to_string_lossy()
                .to_string();
            let source = std::fs::read_to_string(&p).expect("a kernel's source");
            (name, source)
        })
        .collect();
    out.sort();
    out
}

/// Every kernel at every size, as `machine-sympathy/<name>/<size>`.
pub fn programs() -> Vec<Program> {
    let mut out = Vec::new();
    for (name, source) in kernels() {
        for size in SIZES {
            let mut p = Program::new(format!("machine-sympathy/{name}/{size}"), source.clone());
            p.args = vec![size.to_string()];
            out.push(p);
        }
    }
    out
}
