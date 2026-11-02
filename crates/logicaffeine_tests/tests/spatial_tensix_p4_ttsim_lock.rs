//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! P4 ttsim LOCK — the compiler-printed Tensix kernels for the NON-GEMM workloads are CORRECT on
//! Tenstorrent's own simulator, checked against the host golden the same compiler printed.
//!
//! `spatial_tensix_p4_lock` (in `logicaffeine-compile`) proves the printer LOWERS a normalization, a
//! gated activation and an elementwise map. Lowering is not correctness. Every kernel this lock runs
//! lowered cleanly, built cleanly under tt-metal, and two of the three were WRONG the first time they
//! ran — one because `mul_tiles` overwrites the destination register rather than multiplying into it,
//! so a SwiGLU that computed `silu(g)` and then discarded it scored PCC 0.099; one because the host
//! golden sized its planes by a PAGE (32 wide) instead of by the DATA (2048 wide). Neither is visible
//! to any test that stops at "the backend returned Ok".
//!
//! ★ THIS IS THE ONLY THING THAT CATCHES A PLAUSIBLE KERNEL. The lowering lock proves a program was
//! printed; the census proves every kernel was generated; the evaluator proves the IR's semantics.
//! None of them execute the C++ that reaches silicon. This does.
//!
//! ★ NOT A SPEED CLAIM. Every number here is a PCC — a correctness ratio against the golden. The
//! device cycle counter is deliberately not asserted on: `benchmarks/silicon/ttsim_instrument_audit.json`
//! shows it does not move with work, and `tt_ledger::assert_cycles_are_not_quoted_as_throughput`
//! enforces that mechanically. Nothing is written to the GEMM ledger, whose rows are keyed on an
//! (m, n, k) a row op does not have.
//!
//! ★ THREE WITNESSES, AND THE THIRD IS NOT THE COMPILER. `device == golden` is a differential between
//! two things the SAME compiler printed (Tensix kernels through NOC/CB/tilize, versus a scalar C++
//! reference composed stage-by-stage from the same choreography). That catches a wrong kernel, and it
//! is blind by construction to a wrong GOLDEN — which is exactly the second bug this file's history
//! records. So `reference_checksum` re-derives each workload's checksum from its MATHEMATICAL
//! definition, in Rust, from the fills alone, touching no compiler code; the golden the host printed
//! must equal it. `the_reference_model_reproduces_the_goldens_three_real_runs_printed` pins that model
//! to the numbers three real simulator runs printed on 2026-09-09, so nobody can quietly retune the
//! model until it agrees with a broken golden.
//!
//! ★ THE BUNDLE IS PRINTED IN-PROCESS, NEVER BY A NESTED `cargo run`. This lock used to reach its
//! bundle through `scripts/tt-matmul-measure.sh p4`, which shelled out to
//! `cargo run --example emit_spatial_tensix` — a second cargo INSIDE a cargo test run. cargo takes a
//! global lock on the target directory, so those nested invocations blocked on the very build that
//! spawned them and on each other, and every workload died at exit status 2 after two minutes without
//! ever reaching the simulator. Nothing about the measurement needed a subprocess: this crate depends
//! on `logicaffeine-compile`, so the test prints the bundle itself (`compile_to_spatial` →
//! `TensixBackend::lower_tensix` → `bundle_dir_files`) into a staging directory and hands the script
//! `--bundle <dir>`. The script no longer compiles Rust at all — it INSTALLS the staged bundle into
//! the box's single tt-metal tree, builds it and runs it, all still under `logs/tt/.measure.lock`, so
//! moving the emission out of the script did not move it out of the lock: a peer session's GEMM
//! emission still cannot land between our copy and our build.
//!
//! ★ AND THE RUN MUST PROVE IT RAN *OUR* BUNDLE. One build tree is shared by every session, so an
//! install that silently copied nothing would leave a peer's GEMM bundle in place — and a GEMM host
//! prints a passing `LOGOS_MM_PASS` of its own. Writing `spatial.json` beside the bundle does not stop
//! that; nothing reads it. What stops it is `ran_our_bundle`: the host prints its `LOGOS_MM_CFG` line
//! (arrangement, grid, blocking, mesh, site count) at startup, and the run log's copy must contain the
//! `cfg_line` of the artifact THIS test just printed. A stale tree cannot forge it.
//!
//! Gated on the simulator (`require_tool("ttsim")`; `LOGOS_REQUIRE_TOOLS=ttsim` makes absence fail loud).
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(spatial_tensix_p4_ttsim_lock)'`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use logicaffeine_compile::spatial::lower::tensix::{bundle_dir_files, TensixBackend};
use logicaffeine_compile::spatial::lower::Census;
use logicaffeine_compile::spatial::pipeline::compile_to_spatial;
use logicaffeine_compile::spatial::target::SpatialTarget;

/// The P4 workloads and the shape each is measured at. 64 columns is two column tiles, so the
/// reduction genuinely folds more than one tile into the row's accumulator instead of degenerating
/// into a single-tile pass that could never expose a fold bug.
const WORKLOADS: &[(&str, usize, usize)] = &[
    ("rmsnorm", 32, 64),
    ("layernorm", 32, 64),
    ("swiglu", 32, 64),
    ("softmax", 32, 64),
    ("elementwise", 32, 64),
];

/// The tile edge every Tensix geometry is a multiple of. The reference model needs it because the
/// golden's row mean is taken over the PADDED column count; at a shape that is not a whole number of
/// tiles the two would part company silently, so the model refuses such a shape instead.
const TILE: usize = 32;

/// The measure script this lock drives, and the emitter a human drives by hand — the two files whose
/// agreement `the_measured_programs_are_the_ones_the_emitter_writes` and the script-contract tests keep.
const MEASURE_SH: &str = "scripts/tt-matmul-measure.sh";
const EMITTER_RS: &str = "crates/logicaffeine_compile/examples/emit_spatial_tensix.rs";
const THIS_FILE: &str = "crates/logicaffeine_tests/tests/spatial_tensix_p4_ttsim_lock.rs";

// ════════════════════════════════════════════════════════════════════════════════════════════════
// The programs. These are the same three the `emit_spatial_tensix` example writes for a hand-run
// measurement, and `the_measured_programs_are_the_ones_the_emitter_writes` below holds the two
// copies together — byte for byte, in BOTH directions.
//
// The fills are explicit element sets — the same shape the front-end locks use — so the recognizer
// sees a literal tensor and the emitted host golden is computed from exactly those values rather
// than from a pattern the two sides could read differently. `reference_checksum` reads the same
// fills straight out of these formulas, which is why it is allowed to be the independent witness:
// it shares the INPUT with the compiler and shares none of the arithmetic.
// ════════════════════════════════════════════════════════════════════════════════════════════════

fn rmsnorm_program(rows: usize, cols: usize) -> String {
    let mut s = format!("## Main\nLet X be a {rows} by {cols} tensor of Real.\n");
    for i in 1..=rows * cols {
        s += &format!("Set item {i} of X to {:.1}.\n", (i % 7) as f64 - 3.0);
    }
    s += &format!("Let W be a 1 by {cols} tensor of Real.\n");
    for i in 1..=cols {
        s += &format!("Set item {i} of W to {:.1}.\n", 0.5 + (i % 3) as f64 * 0.5);
    }
    s += "Let Y be rmsnorm of X with weight W.\nShow round(item 1 of Y * 1000000).\n";
    s
}

fn swiglu_program(rows: usize, cols: usize) -> String {
    let mut s = format!("## Main\nLet G be a {rows} by {cols} tensor of Real.\n");
    for i in 1..=rows * cols {
        s += &format!("Set item {i} of G to {:.1}.\n", (i % 5) as f64 - 2.0);
    }
    s += &format!("Let U be a {rows} by {cols} tensor of Real.\n");
    for i in 1..=rows * cols {
        s += &format!("Set item {i} of U to {:.1}.\n", (i % 3) as f64 + 1.0);
    }
    s += "Let Y be swiglu of G and U.\nShow round(item 1 of Y * 1000000).\n";
    s
}

fn elementwise_program(rows: usize, cols: usize) -> String {
    let mut s = format!("## Main\nLet A be a {rows} by {cols} tensor of Int.\n");
    for i in 1..=rows * cols {
        s += &format!("Set item {i} of A to {}.\n", (i % 7) as i64 - 3);
    }
    s += &format!("Let B be a {rows} by {cols} tensor of Int.\n");
    for i in 1..=rows * cols {
        s += &format!("Set item {i} of B to {}.\n", (i % 11) as i64 - 5);
    }
    s += "Let C be A plus B.\nShow C.\n";
    s
}

/// LayerNorm — RMSNorm's shape plus a mean subtraction and a per-column bias, which is exactly what
/// distinguishes the two for the recognizer. Weight and bias are per-COLUMN, so both are `1 by cols`.
fn layernorm_program(rows: usize, cols: usize) -> String {
    let mut s = format!("## Main\nLet X be a {rows} by {cols} tensor of Real.\n");
    for i in 1..=rows * cols {
        s += &format!("Set item {i} of X to {:.1}.\n", (i % 7) as f64 - 3.0);
    }
    s += &format!("Let W be a 1 by {cols} tensor of Real.\n");
    for i in 1..=cols {
        s += &format!("Set item {i} of W to {:.1}.\n", 0.5 + (i % 3) as f64 * 0.5);
    }
    s += &format!("Let B be a 1 by {cols} tensor of Real.\n");
    // ★ TWO DECIMALS, NOT ONE. The bias steps by 0.25, and `{:.1}` writes 0.25 as "0.2" — so at one
    // decimal this program does not contain the values this line appears to say, and the device and
    // the host golden agree with each other on the quantized ones while any reader (or independent
    // model) computes the unquantized ones. That cost a real investigation: the p4 reference model
    // made the checksum 26 and the device made it 24, and neither was wrong about its own inputs.
    for i in 1..=cols {
        s += &format!("Set item {i} of B to {:.2}.\n", (i % 5) as f64 * 0.25 - 0.5);
    }
    s += "Let Y be layernorm of X with weight W and bias B.\nShow round(item 1 of Y * 1000000).\n";
    s
}

/// Softmax — the row reduction with no weight at all, which is what makes it the shape that caught the
/// padded-extent bug: a max folded over padding zeros wins outright when every real element is negative.
fn softmax_program(rows: usize, cols: usize) -> String {
    let mut s = format!("## Main\nLet X be a {rows} by {cols} tensor of Real.\n");
    for i in 1..=rows * cols {
        s += &format!("Set item {i} of X to {:.1}.\n", (i % 7) as f64 - 3.0);
    }
    s += "Let Y be softmax of X.\nShow round(item 1 of Y * 1000000).\n";
    s
}

fn program_source(workload: &str, rows: usize, cols: usize) -> String {
    match workload {
        "rmsnorm" => rmsnorm_program(rows, cols),
        "layernorm" => layernorm_program(rows, cols),
        "softmax" => softmax_program(rows, cols),
        "swiglu" => swiglu_program(rows, cols),
        "elementwise" => elementwise_program(rows, cols),
        other => panic!("`{other}` is not a P4 workload — add it to WORKLOADS, to `reference_checksum`, and to `cmd_p4` in scripts/tt-matmul-measure.sh together"),
    }
}

// ════════════════════════════════════════════════════════════════════════════════════════════════
// The independent reference.
// ════════════════════════════════════════════════════════════════════════════════════════════════

/// The checksum a CORRECT implementation of this workload must produce, computed here from the
/// workload's mathematical definition and the fill formulas above — no compiler code on the path.
///
/// ★ THIS IS THE ONLY WITNESS THAT IS NOT THE COMPILER TALKING TO ITSELF. The device result and the
/// host golden are both printed by `logicaffeine-compile`; agreeing with each other says the kernels
/// implement the reference, not that the reference is right. The bug that motivated this file — a
/// golden whose planes were a PAGE wide instead of the DATA's width — made BOTH sides wrong in the
/// same way for the elementwise map, and no PCC could have seen it. This model is the third opinion.
///
/// It mirrors the host's arithmetic ORDER exactly, in the same widths (f64 stages, one f32 store at
/// the end, `llround` = round-half-away-from-zero), which is why it can demand exact equality rather
/// than a tolerance: rmsnorm and the elementwise map use only IEEE-exact operations (×, +, ÷, √) and
/// SwiGLU's single `exp` clears the nearest rounding boundary by 0.023 — a margin no libm difference
/// can cross. If this ever needs a tolerance, that is news about the arithmetic and must be
/// investigated, not papered over.
fn reference_checksum(workload: &str, rows: usize, cols: usize) -> Result<i64, String> {
    if rows % TILE != 0 || cols % TILE != 0 {
        return Err(format!(
            "the reference model refuses {workload} {rows}x{cols}: the host golden reduces over the PADDED \
             column count and copies out of a padded plane, so a shape that is not a whole number of {TILE}×{TILE} \
             tiles would make the model and the golden disagree for a reason that is not a bug. Measure a \
             tile-aligned shape, or teach this model the padding the same way `golden_lines` computes it."
        ));
    }
    let n = rows * cols;
    match workload {
        // square → sum×(1/cols) → rsqrt(x+eps) → rescale → per-column weight, each stage in f64,
        // exactly as `from_tensor_op::rms chain` composes it and `host.rs::golden_lines` prints it.
        "rmsnorm" => {
            let x = |r: usize, c: usize| ((r * cols + c + 1) % 7) as f64 - 3.0;
            let w = |c: usize| 0.5 + ((c + 1) % 3) as f64 * 0.5;
            let k = 1.0f64 / cols as f64;
            // `RowXformKind::Rsqrt { eps_nano: 1_000 }`, converted the way the printer converts it.
            let eps = 1_000f64 * 1e-9;
            let mut sum = 0i64;
            for r in 0..rows {
                let mut acc = 0.0f64;
                for c in 0..cols {
                    let v = x(r, c);
                    acc += v * v * k;
                }
                let scale = 1.0f64 / (acc + eps).sqrt();
                for c in 0..cols {
                    sum += ((x(r, c) * scale * w(c)) as f32).round() as i64;
                }
            }
            Ok(sum)
        }
        // (x − mean)·rsqrt(var/cols + eps) → per-column weight → per-column bias, each stage in f64,
        // exactly as `from_tensor_op` composes the LayerNorm chain. `eps_nano: 10_000` and the divide is
        // carried in `SfpuKind::Rsqrt { over: cols }`, which evaluates as `1/√(v/over + eps)` — so the
        // variance is divided INSIDE the root, once, not folded into the reduction's scaler.
        "layernorm" => {
            let x = |r: usize, c: usize| ((r * cols + c + 1) % 7) as f64 - 3.0;
            let w = |c: usize| 0.5 + ((c + 1) % 3) as f64 * 0.5;
            let b = |c: usize| ((c + 1) % 5) as f64 * 0.25 - 0.5;
            let eps = 10_000f64 * 1e-9;
            let mut sum = 0i64;
            for r in 0..rows {
                let mean = (0..cols).map(|c| x(r, c)).sum::<f64>() / cols as f64;
                let var_sum = (0..cols).map(|c| (x(r, c) - mean) * (x(r, c) - mean)).sum::<f64>();
                let scale = 1.0f64 / (var_sum / cols as f64 + eps).sqrt();
                for c in 0..cols {
                    sum += ((((x(r, c) - mean) * scale) * w(c) + b(c)) as f32).round() as i64;
                }
            }
            Ok(sum)
        }
        // exp(x − rowmax) / Σ exp(x − rowmax). The max subtraction is not an optimization here — it is
        // part of what the compiler emits, so the model composes it the same way or the two would differ
        // in the last bits for a reason that is not a bug.
        "softmax" => {
            let x = |r: usize, c: usize| ((r * cols + c + 1) % 7) as f64 - 3.0;
            let mut sum = 0i64;
            for r in 0..rows {
                let m = (0..cols).map(|c| x(r, c)).fold(f64::NEG_INFINITY, f64::max);
                let denom = (0..cols).map(|c| (x(r, c) - m).exp()).sum::<f64>();
                let inv = 1.0f64 / denom;
                for c in 0..cols {
                    sum += (((x(r, c) - m).exp() * inv) as f32).round() as i64;
                }
            }
            Ok(sum)
        }
        "swiglu" => {
            let mut sum = 0i64;
            for p in 0..n {
                let g = ((p + 1) % 5) as f64 - 2.0;
                let u = ((p + 1) % 3) as f64 + 1.0;
                sum += (((g / (1.0 + (-g).exp())) * u) as f32).round() as i64;
            }
            Ok(sum)
        }
        "elementwise" => {
            let mut sum = 0i64;
            for p in 0..n {
                sum += ((p + 1) % 7) as i64 - 3 + ((p + 1) % 11) as i64 - 5;
            }
            Ok(sum)
        }
        other => Err(format!(
            "`{other}` has no independent reference. A P4 workload without one is measured only against the \
             compiler's own golden, which is the blind spot this model exists to close — write the model \
             before adding the workload to WORKLOADS."
        )),
    }
}

// ════════════════════════════════════════════════════════════════════════════════════════════════
// Printing, running, reading a verdict.
// ════════════════════════════════════════════════════════════════════════════════════════════════

/// Print one workload's whole Tensix bundle into `dir`: host, every kernel, the CMake, the C oracle,
/// and a `spatial.json` describing the plan the bundle came from. Returns the `LOGOS_MM_CFG` line the
/// host in that bundle will print, which is the bundle's identity at run time.
///
/// This is the emitter path with the subprocess taken out, not a reimplementation of it — the same
/// `compile_to_spatial` → `lower_tensix` → `bundle_dir_files` chain, on the same target.
fn emit_bundle(workload: &str, rows: usize, cols: usize, dir: &Path) -> Result<String, String> {
    // `wormhole_ttsim()` is `wormhole()` on the 8×9 compute-with-storage mesh the slow-dispatch
    // simulator exposes — the grid `sim_grid` in the measure script hands the hand-run emitter.
    // `the_simulated_grid_is_the_grid_the_hand_run_emits_on` holds those two together, un-gated, so a
    // drift is RED on every box rather than only on the one with a simulator.
    let target = SpatialTarget::wormhole_ttsim();
    let src = program_source(workload, rows, cols);
    let program = compile_to_spatial(&src, &target)
        .map_err(|e| format!("the spatial pipeline refused {workload} {rows}x{cols} on the simulated wormhole: {e:?}"))?;
    let art = TensixBackend::new(target.clone())
        .lower_tensix(&program)
        .map_err(|e| format!("the Tensix lowering refused {workload} {rows}x{cols}: {e:?} — the fix is in `spatial/lower/tensix/`, and `spatial_tensix_p4_lock` is the matrix that enumerates the gaps"))?;
    std::fs::create_dir_all(dir.join("kernels")).map_err(|e| {
        format!(
            "cannot stage the {workload} bundle at {}: {e}",
            dir.display()
        )
    })?;
    for (rel, body) in bundle_dir_files(&art) {
        std::fs::write(dir.join(&rel), body)
            .map_err(|e| format!("cannot write {rel} into {}: {e}", dir.display()))?;
    }
    // The bundle carries its own plan for a human reading an installed directory. It is NOT what
    // proves the run used it — `ran_our_bundle` does that, from the line the host actually prints.
    let meta = serde_json::json!({
        "workload": workload,
        "shape": [rows, cols],
        "chip": "wormhole",
        "mesh": [target.sites.x, target.sites.y],
        "summary": program.summary(),
        "census": Census::of(&program),
        "cfg_line": art.cfg_line,
        "kernels": art.kernels.iter().map(|k| k.name.clone()).collect::<Vec<_>>(),
    });
    std::fs::write(
        dir.join("spatial.json"),
        serde_json::to_string_pretty(&meta).unwrap(),
    )
    .map_err(|e| format!("cannot write spatial.json into {}: {e}", dir.display()))?;
    Ok(art.cfg_line)
}

/// Install the staged bundle into the box's tt-metal tree, build it, run it under ttsim, and return
/// whether the script itself succeeded together with its whole output.
///
/// The script still owns everything that touches shared state: it holds `logs/tt/.measure.lock` for
/// the copy, the cmake build and the simulator run, so this can never interleave with a GEMM run or
/// with another session's simulator.
///
/// ★ THE EXIT CODE IS NOT THE VERDICT, AND IS NOT DISCARDED EITHER. The run log is the verdict — it is
/// what the host printed — so a non-zero exit is carried alongside it and reported after the log has
/// been read, rather than short-circuiting the rich "which witness disagreed" message. A run whose log
/// says PASS while the script exits non-zero is still a failure: see `measure`.
fn run_p4(
    workload: &str,
    rows: usize,
    cols: usize,
    bundle: &Path,
) -> Result<(bool, String), String> {
    let root = common::ttsim::repo_root();
    let args: Vec<String> = vec![
        "p4".into(),
        "--workload".into(),
        workload.into(),
        "--rows".into(),
        rows.to_string(),
        "--cols".into(),
        cols.to_string(),
        "--chip".into(),
        "wormhole".into(),
        "--bundle".into(),
        bundle.display().to_string(),
    ];
    let out = Command::new(root.join(MEASURE_SH))
        .current_dir(&root)
        .env("TT_METAL_HOME", common::ttsim::tt_metal_home())
        .env("LEDGER", common::ttsim::recording_ledger())
        .args(&args)
        .output()
        .map_err(|e| format!("cannot run {MEASURE_SH}: {e}"))?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok((out.status.success(), text))
}

/// The run log the script named. Everything asserted on is read from THIS file, not from the script's
/// summary, so the test reads what the simulator actually printed.
fn run_log_path(script_out: &str) -> Result<PathBuf, String> {
    script_out
        .lines()
        .find_map(|l| l.trim().strip_prefix("LOGOS_P4_LOG "))
        .map(|p| PathBuf::from(p.trim()))
        .ok_or_else(|| {
            format!(
                "the measure script printed no `LOGOS_P4_LOG <path>` line, so there is no run to read a verdict \
                 from — it died before the simulator (install / cmake / ttsim), or `cmd_p4` in {MEASURE_SH} \
                 stopped naming its log. Both are failures, never a skip.\n{script_out}"
            )
        })
}

/// The run log's own account of WHICH program ran, checked against the bundle this test just printed.
///
/// ★ WITHOUT THIS THE LOCK IS A LOCK ON WHATEVER IS IN THE BUILD TREE. `$SPATIAL_DIR` is one directory
/// shared by every session; an install that copied nothing, or a peer's GEMM bundle that outlived its
/// run, would build and pass and this test would call it a P4 result. The host prints its `cfg_line`
/// before it does anything else, and `cfg_line` is a function of the plan — arrangement, grid, per-core
/// block, mesh, site count. A different program cannot print ours.
fn ran_our_bundle(log: &str, cfg_line: &str) -> Result<(), String> {
    let ran = log.lines().find(|l| l.contains("LOGOS_MM_CFG")).ok_or_else(|| {
        format!(
            "the run printed no `LOGOS_MM_CFG` line, so nothing identifies the program the simulator ran and \
             a stale bundle in the shared tt-metal tree would be indistinguishable from ours. The host prints \
             it (`host.rs::print_host`); if it stopped, restore it rather than dropping this check.\n\
             ours: {cfg_line}"
        )
    })?;
    if ran.contains(cfg_line) {
        return Ok(());
    }
    Err(format!(
        "the simulator ran a DIFFERENT program than this test printed — the staged bundle did not reach \
         `$SPATIAL_DIR`, or a peer session's bundle was built instead.\n  ran:  {}\n  ours: {cfg_line}\n\
         Fix `install_bundle` in {MEASURE_SH} (it must replace host.cpp + kernels/ under the measure lock); \
         never relax this comparison — it is the only thing tying the verdict to the program under test.",
        ran.trim()
    ))
}

/// The PCC a run printed and the floor the program itself declared — both parsed from the log, never
/// typed here.
///
/// ★ A WRONG KERNEL PRINTS `LOGOS_MM_FAIL`, NOT A LOW `LOGOS_MM_PASS`. The host evaluates its own
/// `ok` and prints one line or the other (`host.rs`: `if (ok) … else …`), so "PASS with pcc below the
/// floor" is not the shape a failure arrives in — reading only the PASS line would have reported a
/// genuinely wrong kernel as "the run did not reach its own comparison", which is false and points the
/// reader at the harness instead of at the printer. The FAIL line is matched FIRST and named for what
/// it is. The floor comparison below is kept as a second reading of the PASS line: it re-derives the
/// host's verdict from the two numbers the host printed, so a host whose `ok` and whose printed floor
/// ever part company is caught rather than believed.
fn verdict(log: &str) -> Result<(f64, f64), String> {
    if let Some(failed) = log.lines().find(|l| l.contains("LOGOS_MM_FAIL")) {
        return Err(format!(
            "the compiler-printed Tensix kernel is WRONG on the simulator — the host refused its own \
             comparison: {}\n\
             The fix is in the PRINTER (`spatial/lower/tensix/kernel.rs` for the compute op, \
             `host.rs::golden_lines` for the reference) — never in the floor the program declares, and \
             never by teaching this test to accept a FAIL line.",
            failed.trim()
        ));
    }
    let line = log
        .lines()
        .find(|l| l.contains("LOGOS_MM_PASS"))
        .ok_or_else(|| "the run printed neither LOGOS_MM_PASS nor LOGOS_MM_FAIL — it never reached its own comparison (the program died, or the simulator did)".to_string())?;
    let field = |k: &str| -> Result<f64, String> {
        line.split_whitespace()
            .find_map(|f| f.strip_prefix(k))
            .ok_or_else(|| format!("no `{k}` in the verdict line: {line}"))?
            .parse()
            .map_err(|e| format!("unparsable `{k}` in {line}: {e}"))
    };
    Ok((field("pcc=")?, field("floor=")?))
}

/// The checksum the DEVICE produced and the one the GOLDEN produced, as the host printed them.
///
/// ★ A SECOND WITNESS, AND IT IS REQUIRED, NOT CONSULTED. PCC is a correlation: it is insensitive to a
/// uniform scale and forgiving of a few bad elements. The checksum is a plain sum, so a kernel that
/// gets the shape right and the arithmetic subtly wrong disagrees here even when the correlation still
/// looks good. Both were wrong together on the first SwiGLU run (1228 vs 23). A missing or unparsable
/// line is therefore a REFUSAL: reading it as `None` and moving on would delete the witness the moment
/// the host's print format drifted, and the test would keep passing while checking half of what its
/// name claims.
fn checksums(log: &str) -> Result<(i64, i64), String> {
    let line = log.lines().find(|l| l.contains("LOGOS_CHECKSUM")).ok_or_else(|| {
        "the run printed no LOGOS_CHECKSUM line, so the device/golden sums cannot be compared and PCC would be \
         the only witness left. The host prints it unconditionally (`host.rs`: `fmt::print(\"LOGOS_CHECKSUM {} \
         golden {}\\n\", …)`); if that changed, teach this parser the new shape — do not drop the witness."
            .to_string()
    })?;
    let f: Vec<&str> = line.split_whitespace().collect();
    let i = f
        .iter()
        .position(|x| *x == "LOGOS_CHECKSUM")
        .ok_or_else(|| {
            format!("`LOGOS_CHECKSUM` is not a whole field of the line that contains it: {line}")
        })?;
    let at = |j: usize, what: &str| -> Result<i64, String> {
        f.get(j)
            .ok_or_else(|| format!("the LOGOS_CHECKSUM line has no {what} field: {line}"))?
            .parse::<i64>()
            .map_err(|e| format!("unparsable {what} checksum in `{line}`: {e}"))
    };
    Ok((at(i + 1, "device")?, at(i + 3, "golden")?))
}

/// One workload measured end to end: print the bundle → install/build/run it under ttsim → check every
/// witness. `Ok(())` is a workload that passed all of them; an `Err` carries the whole story of the one
/// that did not, so a run reports EVERY broken workload rather than only the first.
fn measure(workload: &str, rows: usize, cols: usize) -> Result<(), String> {
    let want = reference_checksum(workload, rows, cols)?;
    let staged = tempfile::tempdir()
        .map_err(|e| format!("cannot make a staging directory for {workload}: {e}"))?;
    let cfg_line = emit_bundle(workload, rows, cols, staged.path())?;
    let (script_ok, script_out) = run_p4(workload, rows, cols, staged.path())?;
    let at = format!("{workload} {rows}x{cols}");
    let log_path = run_log_path(&script_out).map_err(|why| format!("{at}: {why}"))?;
    let log = std::fs::read_to_string(&log_path).map_err(|e| {
        format!(
            "{at}: the script named its run log {} but it cannot be read: {e}",
            log_path.display()
        )
    })?;
    let seen = |why: String| format!("{at}: {why}\n(run log: {})", log_path.display());

    ran_our_bundle(&log, &cfg_line).map_err(|why| seen(why))?;
    let (pcc, floor) = verdict(&log).map_err(|why| seen(why))?;
    if pcc < floor {
        return Err(seen(format!(
            "the host printed LOGOS_MM_PASS at PCC {pcc} while declaring a floor of {floor} — its own `ok` \
             expression and the numbers it printed disagree.\n\
             The fix is in the PRINTER (`host.rs`, where `ok` and the PASS line are emitted together) — never \
             in this floor."
        )));
    }
    let (device, golden) = checksums(&log).map_err(|why| seen(why))?;
    if device != golden {
        return Err(seen(format!(
            "PCC passed at {pcc} but the checksums disagree (device {device}, golden {golden}) — the kernel \
             correlates with the reference without computing it. This is the near-miss PCC alone would let \
             through; the fix is in `spatial/lower/tensix/kernel.rs`."
        )));
    }
    if golden != want {
        return Err(seen(format!(
            "the device AGREED with the host golden ({golden}) and the golden is WRONG: an independent Rust \
             model of {workload} over the same fills makes it {want}.\n\
             Two sides of the same compiler agreeing says the kernels implement the reference, not that the \
             reference is right — this is exactly the shape of the bug where `golden_lines` sized its planes by \
             a page instead of by the data. Fix `host.rs::golden_lines` (and `spatial/eval.rs`, which it \
             mirrors); change `reference_checksum` only after proving the new golden right from the workload's \
             definition, never to make this line agree."
        )));
    }
    if !script_ok {
        return Err(seen(format!(
            "every witness passed (pcc {pcc} ≥ {floor}, device = golden = {golden} = the independent model) yet \
             {MEASURE_SH} exited non-zero. Something after the verdict failed — the ledger step, a cleanup, the \
             script's own refusal. Read the script output rather than trusting the verdict alone.\n{script_out}"
        )));
    }
    Ok(())
}

/// Every P4 workload, measured one after another, IN ONE TEST.
///
/// ★ ONE SIMULATOR, ONE BUILD TREE — SO ONE TEST. The box has a single tt-metal `build_Release` and a
/// single ttsim, and `scripts/tt-matmul-measure.sh` serializes every session on `logs/tt/.measure.lock`.
/// Three `#[test]`s could not be serialized in-process: nextest runs each test in its OWN PROCESS, so a
/// `static Mutex` here would guard nothing and all three would race into the flock anyway, each carrying
/// the other two's build time inside its own per-test timeout. A single test that loops makes the
/// sequencing a property of the code rather than of whichever runner happens to launch it — and because
/// the loop collects failures instead of panicking on the first, one run still reports every broken
/// workload, which three separate tests only did by accident of running concurrently.
#[test]
fn every_printed_p4_kernel_is_correct_on_the_simulator() {
    if !common::require_tool("ttsim") {
        return;
    }
    let mut broken = Vec::new();
    for &(workload, rows, cols) in WORKLOADS {
        if let Err(why) = measure(workload, rows, cols) {
            broken.push(why);
        }
    }
    assert!(
        broken.is_empty(),
        "\n{} of {} compiler-printed Tensix workload(s) are wrong on Tenstorrent's simulator:\n\n{}\n",
        broken.len(),
        WORKLOADS.len(),
        broken.join("\n\n────────────────────────────────────────────────────────────────────────\n\n")
    );
}

// ════════════════════════════════════════════════════════════════════════════════════════════════
// The un-gated locks. None of these need a simulator, so the seams that make the gated lock mean
// something are checked on every box, on every run.
// ════════════════════════════════════════════════════════════════════════════════════════════════

/// The goldens three REAL simulator runs printed on 2026-09-09, kept as the fixed point the
/// independent model is pinned to:
///   `logs/tt/p4_rmsnorm_32x64__20260909T203832Z.full.log`     LOGOS_CHECKSUM -1 golden -1
///   `logs/tt/p4_swiglu_32x64__20260909T203706Z.full.log`      LOGOS_CHECKSUM 1637 golden 1637
///   `logs/tt/p4_elementwise_32x64__20260909T204001Z.full.log` LOGOS_CHECKSUM -9 golden -9
const RECORDED_GOLDENS: &[(&str, usize, usize, i64)] = &[
    ("rmsnorm", 32, 64, -1),
    ("swiglu", 32, 64, 1637),
    ("elementwise", 32, 64, -9),
];

/// ★ AN INDEPENDENT REFERENCE THAT NOBODY CAN CHECK IS NOT INDEPENDENT. `reference_checksum` is the one
/// witness in this file that does not come from the compiler, which makes it the one witness someone
/// under pressure could quietly edit until the compiler agrees with it. So it is pinned, here, without
/// a simulator, to numbers that were MEASURED — the goldens three real ttsim runs printed on a day the
/// logs still record. Editing the model now costs a second, deliberate edit of a recorded fact.
#[test]
fn the_reference_model_reproduces_the_goldens_three_real_runs_printed() {
    for &(workload, rows, cols, recorded) in RECORDED_GOLDENS {
        let got = reference_checksum(workload, rows, cols).unwrap_or_else(|why| panic!("{why}"));
        assert_eq!(
            got, recorded,
            "the independent model of {workload} {rows}x{cols} now makes {got}, but a real ttsim run recorded a \
             golden of {recorded} (see the log names beside RECORDED_GOLDENS).\n\
             Either the model drifted — fix the model — or the workload's definition genuinely changed, in which \
             case re-measure on the box and update this recorded fact from the new log. Never edit the recorded \
             number to match the model: it is the only thing that keeps the model from being retuned to whatever \
             the compiler currently prints."
        );
    }
}

/// ★ TWO COPIES OF A PROGRAM ARE TWO PROGRAMS UNLESS SOMETHING HOLDS THEM TOGETHER. The bundle this
/// lock measures is printed from the generators at the top of THIS file; the bundle a human emits by
/// hand comes from `emit_spatial_tensix.rs`. If the two ever build a different program, a hand-run
/// measurement stops being a reproduction of this lock and nothing says so.
///
/// ★ AND THE COMPARISON RUNS IN BOTH DIRECTIONS. A list of expected substrings, typed here and looked
/// for over there, is half a lock: it catches an edit to the emitter and is blind to an edit to this
/// file, because the substrings drift with the code they were copied from. So the generators' SOURCE
/// TEXT is extracted from both files and compared byte for byte — an edit to either side is RED.
///
/// The durable fix, when either side next needs to change, is to LIFT the three generators into
/// `logicaffeine-compile` so there is one definition and this test has nothing left to compare — not
/// to relax the comparison below.
#[test]
fn the_measured_programs_are_the_ones_the_emitter_writes() {
    let root = common::ttsim::repo_root();
    let mine = read_repo_file(&root, THIS_FILE);
    assert!(
        mine.contains("fn every_printed_p4_kernel_is_correct_on_the_simulator"),
        "{THIS_FILE} does not look like this test file — it was renamed or moved, and this lock has been \
         comparing the emitter against the wrong text. Point THIS_FILE at the new path."
    );
    let theirs = read_repo_file(&root, EMITTER_RS);
    for name in ["rmsnorm_program", "swiglu_program", "elementwise_program"] {
        let a = generator_text(&mine, name, THIS_FILE);
        let b = generator_text(&theirs, name, EMITTER_RS);
        assert_eq!(
            a, b,
            "`{name}` has drifted: {THIS_FILE} and {EMITTER_RS} no longer build the same program, so a hand-run \
             `emit_spatial_tensix --workload …` would measure something other than what \
             `spatial_tensix_p4_ttsim_lock` measures — and `reference_checksum` would be modelling whichever of \
             the two it was written against.\n\
             Fix by making both sides identical, best of all by lifting the three generators into \
             `logicaffeine-compile` so only one copy exists. Never by loosening this comparison."
        );
    }
}

/// ★ THE PLACEMENT THIS LOCK MEASURES MUST BE THE PLACEMENT A HAND RUN EMITS. This test prints its
/// bundle for `SpatialTarget::wormhole_ttsim()`; a human runs `emit_spatial_tensix … --grid $(sim_grid
/// wormhole)`, where `sim_grid` is a case statement in the shell script. Two constants, in two
/// languages, that must name one mesh — so the shell's answer is READ rather than restated, and a
/// lock that measured a different placement from the one anybody can reproduce is impossible.
#[test]
fn the_simulated_grid_is_the_grid_the_hand_run_emits_on() {
    let root = common::ttsim::repo_root();
    let script = read_repo_file(&root, MEASURE_SH);
    let line = script
        .lines()
        .find(|l| l.trim_start().starts_with("sim_grid()"))
        .unwrap_or_else(|| panic!("{MEASURE_SH} no longer defines `sim_grid()`, which is what hands the hand-run emitter its mesh"));
    let after = line
        .split_once("wormhole) echo ")
        .unwrap_or_else(|| {
            panic!("`sim_grid()` in {MEASURE_SH} no longer answers for `wormhole`: {line}")
        })
        .1;
    let shell_grid = after.split_whitespace().next().unwrap_or_default();
    let target = SpatialTarget::wormhole_ttsim();
    assert_eq!(
        shell_grid,
        format!("{}x{}", target.sites.x, target.sites.y),
        "`SpatialTarget::wormhole_ttsim()` places on {}×{} but `sim_grid wormhole` in {MEASURE_SH} hands the \
         hand-run emitter {shell_grid}. This lock would then measure a placement nobody can reproduce by hand. \
         Fix whichever of the two is wrong about the simulator's compute-with-storage grid — never this assertion.",
        target.sites.x,
        target.sites.y
    );
}

/// ★ THE WHOLE POINT OF THE REWRITE IS A CONTRACT WITH A SHELL SCRIPT, SO THE CONTRACT IS LOCKED. The
/// nested-`cargo run` deadlock comes back the instant `--bundle` stops being handled, and it comes back
/// as a two-minute exit-2 hang per workload on a box that has a simulator — invisible everywhere else.
/// The lock-ordering half matters just as much: the install must happen INSIDE `logs/tt/.measure.lock`,
/// or a peer session's GEMM emission can land between our copy and our build and we would measure it.
#[test]
fn the_measure_script_installs_a_printed_bundle_under_the_lock() {
    let root = common::ttsim::repo_root();
    let script = read_repo_file(&root, MEASURE_SH);
    for needle in [
        "install_bundle() {",
        "--bundle) bundle=\"$2\"",
        "if [ -n \"$bundle\" ]; then",
        "install_bundle \"$bundle\"",
    ] {
        assert!(
            script.contains(needle),
            "{MEASURE_SH} no longer carries `{needle}`, so `p4 --bundle <dir>` cannot install an already-printed \
             bundle. Without it this lock is back to a nested `cargo run --example emit_spatial_tensix` inside a \
             cargo test run, which deadlocks on cargo's target-directory lock and dies at exit 2 without ever \
             reaching the simulator. Restore the branch rather than reverting this test."
        );
    }
    let lock_at = script
        .find("exec 9>\"$LOG_DIR/.measure.lock\"")
        .unwrap_or_else(|| panic!("{MEASURE_SH} no longer takes `logs/tt/.measure.lock` — one box, one build tree, one simulator: every mode must serialize on it"));
    let dispatch_at = script
        .find("p4) cmd_p4 \"$@\"")
        .unwrap_or_else(|| panic!("{MEASURE_SH} no longer dispatches the `p4` mode"));
    assert!(
        lock_at < dispatch_at,
        "{MEASURE_SH} dispatches `p4` before taking logs/tt/.measure.lock, so `install_bundle` copies into the \
         shared tt-metal tree unserialized: a peer session's emission can land between our copy and our build \
         and this lock would measure THEIR program. Take the flock before the mode dispatch."
    );
    // The workloads the harness can run and the workloads this lock measures must be the same set —
    // otherwise a workload is added, is runnable by hand, and is silently never measured here.
    let case_line = script
        .lines()
        .find(|l| l.contains("case \"$workload\" in"))
        .unwrap_or_else(|| panic!("{MEASURE_SH} no longer validates `--workload` with a case statement, so this lock cannot read which workloads it accepts"));
    let accepted: Vec<&str> = case_line
        .split_once("case \"$workload\" in ")
        .unwrap_or_else(|| panic!("unreadable workload case in {MEASURE_SH}: {case_line}"))
        .1
        .split(')')
        .next()
        .unwrap_or_default()
        .split('|')
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .collect();
    let measured: Vec<&str> = WORKLOADS.iter().map(|(w, _, _)| *w).collect();
    assert_eq!(
        accepted, measured,
        "{MEASURE_SH} accepts `--workload` {accepted:?} but this lock measures {measured:?}. A workload the \
         harness can run and this test never runs is a kernel nobody proves correct on the simulator — add it \
         to WORKLOADS and give it a `reference_checksum`, or take it out of the script."
    );
}

// ════════════════════════════════════════════════════════════════════════════════════════════════
// Reading the repo's own text.
// ════════════════════════════════════════════════════════════════════════════════════════════════

fn read_repo_file(root: &Path, rel: &str) -> String {
    let path = root.join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// The source text of one `fn <name>(rows: usize, cols: usize) -> String { … }`, from its signature to
/// the closing brace in column 0. Both files declare these at the top level with the same signature, so
/// the same slice is taken from each and compared verbatim.
fn generator_text(text: &str, name: &str, whose: &str) -> String {
    let head = format!("\nfn {name}(rows: usize, cols: usize) -> String {{\n");
    let at = text
        .find(&head)
        .unwrap_or_else(|| panic!("{whose} no longer declares `fn {name}(rows: usize, cols: usize) -> String` at the top level — the two copies of the P4 programs cannot be compared, which is the drift this lock exists to refuse"));
    let body = &text[at + 1..];
    let end = body
        .find("\n}\n")
        .unwrap_or_else(|| panic!("`{name}` in {whose} has no closing brace in column 0 — reformat it back to a plain top-level function so both copies stay comparable"));
    body[..end + 2].to_string()
}

#[test]
fn every_fill_value_survives_the_formatter_that_writes_it() {
    // ★ THE TRAP THIS EXISTS TO CLOSE, because it caught us. A workload's fills are written into LOGOS
    // source with `{:.N}`, and a value that needs more decimals than N is SILENTLY QUANTIZED — the
    // program then does not contain the values its own line appears to say. The device and the host
    // golden both read the program, so they agree with each other perfectly on the quantized values,
    // and only an independent model computing the unquantized ones ever notices. That is exactly what
    // happened: LayerNorm's bias stepped by 0.25 through a `{:.1}`, ±0.25 became ±0.2, the reference
    // made the checksum 26 and the device made it 24, and NEITHER was wrong about its own input.
    //
    // A PCC check cannot see this and no amount of agreement between device and golden can either.
    // So: every value a fill writes must round-trip through the formatter that writes it.
    for &(workload, rows, cols) in WORKLOADS {
        let src = program_source(workload, rows, cols);
        for (n, line) in src.lines().enumerate() {
            let Some(rest) = line.strip_prefix("Set item ") else {
                continue;
            };
            let Some((_, value)) = rest.split_once(" to ") else {
                continue;
            };
            let text = value.trim_end_matches('.');
            let parsed: f64 = text
                .parse()
                .unwrap_or_else(|e| panic!("{workload}:{n}: `{text}` is not a number: {e}"));
            // Re-emitting the parsed value at the same precision must reproduce the text exactly. If it
            // does not, the source carries a value the author did not write.
            let decimals = text.split_once('.').map(|(_, d)| d.len()).unwrap_or(0);
            let round_tripped = format!("{parsed:.decimals$}");
            assert_eq!(
                round_tripped, text,
                "{workload} line {n}: `{text}` does not survive its own formatter at {decimals} decimal(s)"
            );
            // And the value must be exactly representable at that precision — i.e. writing it with MORE
            // decimals adds nothing. This is the half of the check that actually catches quantization:
            // `0.25` written as `{:.1}` round-trips as "0.2" perfectly well, and is still not 0.25.
            let wider = format!("{parsed:.*}", decimals + 4);
            let trimmed = wider.trim_end_matches('0').trim_end_matches('.');
            assert_eq!(
                trimmed.parse::<f64>().unwrap(),
                parsed,
                "{workload} line {n}: `{text}` at {decimals} decimal(s) is a QUANTIZED value — give the fill enough precision to mean what it says"
            );
        }
    }
}
