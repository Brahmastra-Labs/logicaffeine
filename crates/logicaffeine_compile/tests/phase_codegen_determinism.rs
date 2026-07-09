//! Codegen must be DETERMINISTIC: compiling the same LOGOS source twice yields
//! byte-identical Rust. Non-determinism (e.g. emitting operands in `HashMap`
//! iteration order) breaks reproducible builds, makes the optimization-firing
//! trace's soundness check flaky, and gives the differential optimization
//! analyzer false positives. Each benchmark program is compiled several times;
//! all outputs must agree.

use logicaffeine_compile::compile::compile_to_rust;
use std::fs;

/// All benchmark programs (`benchmarks/programs/<name>/main.lg`), read from disk
/// so the test covers every committed benchmark without enumerating them.
fn benchmark_programs() -> Vec<(String, String)> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/programs");
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).expect("benchmarks/programs directory") {
        let path = entry.unwrap().path();
        let main_lg = path.join("main.lg");
        if main_lg.exists() {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            out.push((name, fs::read_to_string(&main_lg).unwrap()));
        }
    }
    out.sort();
    out
}

// ── Sharding ─────────────────────────────────────────────────────────────────
// Compiling every benchmark several times is multi-minute; nextest parallelizes
// ACROSS tests, never within one, so the determinism gate is split into
// independent `#[test]` shards over the SAME deterministic (sorted)
// `benchmark_programs()` list, selected by `index % DET_SHARDS`. Each program's
// determinism is independent of the others, so a per-program partition checks the
// identical set the original single test did. `det_partition_tiles_programs`
// proves the tiling — every program owned by exactly one shard, none dropped.

/// Number of parallel shards the determinism gate is split across.
const DET_SHARDS: usize = 4;

/// The indices into `benchmark_programs()` owned by `shard` — every `i` with
/// `i % DET_SHARDS == shard`. Derived from the one shared (sorted) list, so the
/// partition is identical in every shard process.
fn det_shard(shard: usize) -> impl Iterator<Item = usize> {
    (0..benchmark_programs().len()).filter(move |i| i % DET_SHARDS == shard)
}

/// One shard of the determinism gate: compiling each owned program repeatedly
/// must produce identical Rust every time. `HashMap`/`HashSet` iteration order
/// changes between compiles (the per-process `RandomState` seed advances), so
/// several rounds reliably surface any emission that depends on it.
fn run_determinism_shard(shard: usize) {
    const ROUNDS: usize = 6;
    let programs = benchmark_programs();
    let mut owned = 0usize;
    let mut nondeterministic: Vec<String> = Vec::new();

    for i in det_shard(shard) {
        owned += 1;
        let (name, src) = &programs[i];
        let first = match compile_to_rust(src) {
            Ok(r) => r,
            Err(_) => continue, // not all programs compile to Rust; skip those
        };
        let stable = (1..ROUNDS).all(|_| compile_to_rust(src).map(|r| r == *first).unwrap_or(false));
        if !stable {
            nondeterministic.push(name.clone());
        }
    }

    assert!(owned > 0, "determinism shard {shard}/{DET_SHARDS} owns no programs");
    assert!(
        nondeterministic.is_empty(),
        "codegen is non-deterministic for (shard {shard}/{DET_SHARDS}): {nondeterministic:?}"
    );
}

macro_rules! determinism_shards {
    ($($name:ident => $idx:expr;)+) => {
        $(
            /// One determinism shard — see `run_determinism_shard`.
            #[test] fn $name() { run_determinism_shard($idx); }
        )+
    };
}
determinism_shards! {
    codegen_is_deterministic_across_compiles_s0 => 0;
    codegen_is_deterministic_across_compiles_s1 => 1;
    codegen_is_deterministic_across_compiles_s2 => 2;
    codegen_is_deterministic_across_compiles_s3 => 3;
}

/// Coverage guard: the `DET_SHARDS` shards tile `benchmark_programs()` exactly —
/// every program owned by exactly one shard, every shard non-empty. Proves the
/// shard fns together check the identical program set the original single test
/// did (no program dropped or double-counted).
#[test]
fn det_partition_tiles_programs() {
    let len = benchmark_programs().len();
    let mut hits = vec![0u32; len];
    for shard in 0..DET_SHARDS {
        for i in det_shard(shard) {
            hits[i] += 1;
        }
    }
    assert!(
        hits.iter().all(|&h| h == 1),
        "programs not tiled exactly once by {DET_SHARDS} shards: {hits:?}"
    );
    for shard in 0..DET_SHARDS {
        assert!(det_shard(shard).count() > 0, "determinism shard {shard} owns no programs");
    }
}
