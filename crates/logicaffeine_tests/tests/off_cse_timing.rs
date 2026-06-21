//! Wave 26 RELATIVE timing harness for the scaled-index CSE. NOT a correctness
//! gate (the corpus differential is that) — an `#[ignore]`d study that times a
//! real benchmark through the EXODIA VM+JIT tier (`ForgeTier`) so the lever's
//! effect can be measured by toggling `LOGOS_NO_OFF_CSE` between two processes.
//!
//! Run interleaved (the trustworthy signal on a shared box):
//!   for i in 1 2 3; do
//!     CARGO_TARGET_DIR=/tmp/wf26_target cargo test -p logicaffeine-tests \
//!       --test off_cse_timing -- --ignored --nocapture off_cse_time_nbody
//!     LOGOS_NO_OFF_CSE=1 CARGO_TARGET_DIR=/tmp/wf26_target cargo test \
//!       -p logicaffeine-tests --test off_cse_timing -- --ignored --nocapture \
//!       off_cse_time_nbody
//!   done

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::vm_outcome_with_args;
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;
use std::time::Instant;

fn program_source(name: &str) -> String {
    let path = format!(
        "{}/../../benchmarks/programs/{}/main.lg",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read benchmark program {path}: {e}"))
}

fn time_one(name: &str, size: &str) {
    let source = program_source(name);
    let argv = vec!["bench".to_string(), size.to_string()];
    let cse = std::env::var("LOGOS_NO_OFF_CSE").map_or("ON", |v| if v == "1" { "OFF" } else { "ON" });

    // Best-of-N to dampen scheduler noise on a shared box.
    let mut best = f64::MAX;
    let mut last_out_len = 0usize;
    for _ in 0..5 {
        let tier = ForgeTier::new();
        let t0 = Instant::now();
        let vm = vm_outcome_with_args(&source, &argv, Some(&tier as &dyn NativeTier));
        let dt = t0.elapsed().as_secs_f64() * 1e3;
        assert_eq!(vm.error, None, "{name} errored: {:?}", vm.error);
        last_out_len = vm.output.len();
        best = best.min(dt);
    }
    eprintln!(
        "[OFF_CSE {cse}] {name} size={size}: best {best:.2} ms over 5 runs (out {last_out_len} bytes)"
    );
}

#[test]
#[ignore = "timing study — run explicitly, interleaved with LOGOS_NO_OFF_CSE=1"]
fn off_cse_time_nbody() {
    time_one("nbody", "50000");
}

#[test]
#[ignore = "timing study — run explicitly, interleaved with LOGOS_NO_OFF_CSE=1"]
fn off_cse_time_matrix_mult() {
    time_one("matrix_mult", "300");
}

#[test]
#[ignore = "timing study"]
fn off_cse_time_prefix_sum() {
    time_one("prefix_sum", "10000000");
}

#[test]
#[ignore = "timing study"]
fn off_cse_time_spectral_norm() {
    time_one("spectral_norm", "2000");
}

#[test]
#[ignore = "timing study"]
fn off_cse_time_graph_bfs() {
    time_one("graph_bfs", "100000");
}

#[test]
#[ignore = "timing study"]
fn off_cse_time_counting_sort() {
    time_one("counting_sort", "1000000");
}

#[test]
#[ignore = "timing study"]
fn off_cse_time_two_sum() {
    time_one("two_sum", "1000000");
}
