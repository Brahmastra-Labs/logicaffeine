//! The machine-sympathy corpus: the kernel shapes an optimizing compiler is judged on, each one run bit for bit
//! on every CPU tier, with the optimizations that fire on it recorded and held.
//!
//! Every kernel reproduces the shape behind one claim of the CPU performance-engineering microbenchmarks
//! (`usamahz/cpu-performance-engineering`, `misc/benchmarks/NN-*`, MIT): [`CATALOGUE`] names the claim. The
//! programs are in `fixtures/machine_sympathy/` and run at every size of [`tier_parity::machine_sympathy::SIZES`].
//! Each prints what it computed, so no tier can delete the work, and the float kernels use operands whose
//! partial sums stay exact wherever the claim is about speed rather than rounding.
//!
//! [`the_census_of_fired_optimizations_only_grows`] reads which optimizations fire on each kernel on the three
//! optimizing paths (the AOT pipeline the Rust and C backends consume, the run path, the VM compiler) and holds
//! them to `machine_sympathy_census.tsv`: an optimization that stops firing on a kernel is RED until the change
//! names the unsoundness it closed and what fires instead. `UPDATE_SNAPSHOTS=1` rewrites the census.

#![cfg(not(target_arch = "wasm32"))]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

use std::collections::{BTreeMap, BTreeSet};

use tier_parity::{machine_sympathy, Backend, Program};

/// Every kernel and the claim it reproduces.
const CATALOGUE: &[(&str, &str)] = &[
    (
        "branch_sum",
        "02 a data-dependent branch costs its mispredictions; sorted data or a select removes them",
    ),
    (
        "chain_vs_lanes_int",
        "03 one dependency chain is bound by latency, eight independent lanes by throughput",
    ),
    (
        "chain_vs_lanes_real",
        "03 the same for Real, where each written order is its own exact rounding",
    ),
    (
        "pointer_chase",
        "04 a dependent load per step pays the latency of wherever the line lives",
    ),
    (
        "unused_result",
        "05 a result nobody observes is never computed",
    ),
    (
        "saxpy",
        "06 a streaming map below one flop per byte is bound by memory bandwidth",
    ),
    (
        "stencil7",
        "06 seven taps over one stream raise the arithmetic intensity",
    ),
    (
        "horner_map",
        "06 a degree-16 polynomial per element is bound by arithmetic",
    ),
    (
        "stream_triad",
        "06/15 the triad's two reads and one write are the bandwidth roof",
    ),
    (
        "particles_field_sum",
        "07 one field of an array of records moves every record's bytes",
    ),
    (
        "saxpy_in_place",
        "08 a call whose two sequences are one sequence must not change the answer",
    ),
    (
        "counters_simultaneously",
        "09 independent parallel counters must not share a cache line",
    ),
    (
        "first_touch_fill",
        "10 allocation is not placement: the first pass pays every page fault",
    ),
    (
        "show_many_lines",
        "11 every kernel crossing has a fixed cost that one line per write pays",
    ),
    (
        "gemm_real",
        "13 a dense product, whose blocked forms all add each element's terms in ascending order",
    ),
];

/// The CPU tiers every kernel must agree on byte for byte with the VM.
const TIERS: &[Backend] = &[
    Backend::TreeWalker,
    Backend::Vm,
    Backend::VmOptimized,
    Backend::VmForge,
    Backend::Interpret,
    Backend::RustDebug,
    Backend::RustRelease,
    Backend::C,
    Backend::CBytecode,
    Backend::Wasm,
    Backend::RiscV,
];

fn jobs() -> usize {
    std::env::var("LOGOS_PARITY_JOBS")
        .ok()
        .and_then(|j| j.parse().ok())
        .unwrap_or(4)
}

#[test]
#[ignore = "the worker the parity sweep re-executes this binary into"]
fn tier_parity_worker() {
    tier_parity::worker_main();
}

#[test]
fn every_kernel_is_catalogued_once_with_its_claim() {
    let on_disk: BTreeSet<String> = machine_sympathy::kernels()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let catalogued: BTreeSet<String> = CATALOGUE.iter().map(|(name, _)| name.to_string()).collect();
    assert_eq!(
        catalogued.len(),
        CATALOGUE.len(),
        "a kernel is catalogued twice"
    );
    let uncatalogued: Vec<&String> = on_disk.difference(&catalogued).collect();
    let missing: Vec<&String> = catalogued.difference(&on_disk).collect();
    assert!(
        uncatalogued.is_empty(),
        "kernels with no claim in CATALOGUE: {uncatalogued:?}"
    );
    assert!(
        missing.is_empty(),
        "catalogued kernels with no program in fixtures/machine_sympathy: {missing:?}"
    );
}

#[test]
fn every_kernel_runs_bit_for_bit_on_every_cpu_tier() {
    let programs: Vec<Program> = machine_sympathy::programs();
    let rows = tier_parity::sweep(&programs, TIERS, jobs())
        .unwrap_or_else(|why| panic!("the Rust batch build could not start: {why}"));
    let bad: Vec<String> = rows
        .iter()
        .filter(|r| !r.matches())
        .map(|r| {
            format!(
                "{} on {}: {} {}",
                r.program,
                r.backend.name(),
                r.status(),
                r.detail()
            )
        })
        .collect();
    assert!(
        bad.is_empty(),
        "{} (kernel, tier) runs differ from the VM:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// The three optimizing paths, each with the optimizations that fire on `source` there.
fn fired(source: &str) -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    use logicaffeine_compile::compile;
    BTreeMap::from([
        (
            "aot",
            compile::optimizations_fired(source).into_iter().collect(),
        ),
        (
            "run",
            compile::optimizations_fired_run(source)
                .into_iter()
                .collect(),
        ),
        (
            "vm",
            compile::optimizations_fired_vm(source)
                .into_iter()
                .collect(),
        ),
    ])
}

const CENSUS: &str = "tests/machine_sympathy_census.tsv";

fn census_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(CENSUS)
}

fn render(census: &BTreeMap<(String, String), BTreeSet<String>>) -> String {
    let mut out = String::from("kernel\tpath\tfired\n");
    for ((kernel, path), opts) in census {
        out.push_str(&format!(
            "{kernel}\t{path}\t{}\n",
            opts.iter().cloned().collect::<Vec<_>>().join(",")
        ));
    }
    out
}

fn parse(text: &str) -> BTreeMap<(String, String), BTreeSet<String>> {
    text.lines()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            assert_eq!(
                cols.len(),
                3,
                "{CENSUS}: a row is `kernel<TAB>path<TAB>fired`, got {l:?}"
            );
            let opts = cols[2]
                .split(',')
                .filter(|o| !o.is_empty())
                .map(str::to_string)
                .collect();
            ((cols[0].to_string(), cols[1].to_string()), opts)
        })
        .collect()
}

#[test]
fn the_census_of_fired_optimizations_only_grows() {
    let mut now: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for (kernel, source) in machine_sympathy::kernels() {
        for (path, opts) in fired(&source) {
            now.insert(
                (kernel.clone(), path.to_string()),
                opts.into_iter().map(str::to_string).collect(),
            );
        }
    }
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(census_path(), render(&now)).expect("write the census");
        return;
    }
    let recorded =
        parse(&std::fs::read_to_string(census_path()).unwrap_or_else(|e| panic!("{CENSUS}: {e}")));
    let rows_now: BTreeSet<&(String, String)> = now.keys().collect();
    let rows_recorded: BTreeSet<&(String, String)> = recorded.keys().collect();
    assert_eq!(rows_now, rows_recorded, "the census rows are the kernels × paths; UPDATE_SNAPSHOTS=1 rewrites it after a kernel is added or removed");
    let mut lost = Vec::new();
    let mut gained = Vec::new();
    for (row, opts) in &recorded {
        let current = &now[row];
        for o in opts.difference(current) {
            lost.push(format!(
                "{} on the {} path: {o} no longer fires",
                row.0, row.1
            ));
        }
        for o in current.difference(opts) {
            gained.push(format!("{} on the {} path: {o}", row.0, row.1));
        }
    }
    if !gained.is_empty() {
        eprintln!(
            "newly firing (UPDATE_SNAPSHOTS=1 records them):\n{}",
            gained.join("\n")
        );
    }
    assert!(
        lost.is_empty(),
        "{} optimizations stopped firing on a machine-sympathy kernel. Name the unsoundness each closes and what fires \
         in its place, then record the census with UPDATE_SNAPSHOTS=1:\n{}",
        lost.len(),
        lost.join("\n")
    );
}
