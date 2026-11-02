//! wprog_engine_rtl_fuzz — the WHOLE-PROGRAM ENGINE RTL DIFFERENTIAL FUZZ: random fabric-lowerable programs run
//! through the REAL Verilog engine (`emit_wprog_verilog`), not just the ~35 curated benchmarks.
//!
//! `wprog_all_benchmarks_lane` proves the engine model (`eval_wprog_config` ≡ VM) on 35 hand-written benchmark
//! programs, and `wprog_rtl_lock` proves the engine RTL (`emit_wprog_verilog` under iverilog) ≡ that model on a
//! HANDFUL of curated configs (division, recursion, one loop, one collection). The residual those leave open is
//! the exact silicon-bug class the soft-core fuzz found FOUR of on the OTHER processor: a randomly-generated
//! control-flow + arithmetic COMBINATION where the engine SILICON (`emit_wprog_verilog`) diverges from the model,
//! but which no curated benchmark happens to exercise. This harness closes it for the whole-program engine: a
//! deterministic PRNG synthesizes bounded function-form programs in the exact shape the engine lowers (the
//! benchmark shape — `## To f (n: Int) -> Int:` with while-loops, int arithmetic, if/else, nested loops, Break,
//! and self-recursion), each is lowered to a `WProgConfig`, and — for the configs that lower and FIT the fixed
//! engine caps — the config is baked into the engine RTL, run under iverilog for a set of random inputs, and the
//! hardware result is asserted BIT-EXACT against the VM. A divergence is a real engine-silicon defect and panics
//! with the reproducing seed + source + inputs + (rtl, vm) values.
//!
//! WHY IT IS SOUND (a green run is a real guarantee, not a vacuous one):
//!   * Every program is TERMINATING (a single counter `i` up to a small `n`, an inner loop bounded the same way,
//!     recursion depth ≤ the argument which is ≤ the engine's frame-stack DEPTH) and error-free by construction:
//!     divisors are `(… % k) + 1` so a `/`/`%` is never by zero, and every value is kept bounded by `% MOD`
//!     guards so nothing overflows i64 (the engine's registers and the VM are both i64 in this integer surface).
//!   * The VM is the oracle: for every input the program's VM output is computed (asserted no error), then the
//!     SAME value is fed to the model (`eval_wprog_config`, cross-checked == VM) and to the iverilog testbench as
//!     the expected result — so a hardware `PASS` transitively proves `engine RTL == VM`.
//!   * A config that doesn't lower (neither `program_cfg_fabric` nor `program_calls_fabric`) or that exceeds the
//!     fixed engine caps (`wprog_config_fits_engine`) is SKIPPED (counted as `boundary`) — never run half-way and
//!     never silently mis-run.
//!   * NON-VACUITY: a floor on the number of programs that actually reached the engine RTL under iverilog keeps
//!     the generator from silently drifting off the lowerable surface, and a distinct-shape count proves it is
//!     exercising a spread of control-flow structures, not the same trivial config over and over.
//!
//! Deterministic by seed (splitmix64): a failure reproduces exactly. iverilog-gated — if `iverilog` is not on
//! PATH the test SKIPS loudly. The engine RTL is heavy under iverilog, so the corpus is kept modest (a few dozen
//! programs) — it runs in a few minutes.
//!
//! Run: `cargo test -p logicaffeine-tests --test wprog_engine_rtl_fuzz -- --nocapture`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::panic::{catch_unwind, AssertUnwindSafe};

use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_testbench, emit_wprog_verilog, eval_wprog_config, program_calls_fabric,
    serialize_program, wprog_config_fits_engine, WProgConfig,
};
use logicaffeine_compile::compile::vm_outcome;

#[path = "common/mod.rs"]
mod common;

#[path = "support/fabric_program_gen.rs"]
mod fabric_program_gen;
use fabric_program_gen::{gen, gen_recursive_2arg, Rng, Shape};

/// Run the VM on `f(arg)` and return its first integer token, or None on error / non-integer output — the oracle.
fn vm_f(src: &str, arg: i64) -> Option<i64> {
    let a = if arg < 0 {
        format!("(0 - {})", -arg)
    } else {
        arg.to_string()
    };
    let prog = format!("{src}## Main\n    Show f({a}).\n");
    let out = vm_outcome(&prog);
    if out.error.is_some() {
        return None;
    }
    out.output.split_whitespace().next()?.parse::<i64>().ok()
}

/// Lower `src` to a `WProgConfig` that both LOWERS and evaluates ≡ VM for every `arg` — preferring the
/// single-function `program_cfg_fabric`, falling back to `program_calls_fabric` for recursion. `None` if neither
/// lowers or the model already disagrees with the VM (a model/lowering issue, distinct from an RTL divergence).
fn config_for(src: &str, args: &[i64], oracle: &[i64]) -> Option<WProgConfig> {
    let try_one = |lower: &dyn Fn() -> WProgConfig| -> Option<WProgConfig> {
        let cfg = catch_unwind(AssertUnwindSafe(lower)).ok()?;
        let ok = args.iter().zip(oracle).all(|(&a, &o)| {
            catch_unwind(AssertUnwindSafe(|| eval_wprog_config(&cfg, &[a]))).ok() == Some(o)
        });
        if ok {
            Some(cfg)
        } else {
            None
        }
    };
    try_one(&|| serialize_program(&program_cfg_fabric(src, "f")))
        .or_else(|| try_one(&|| program_calls_fabric(src, "f")))
}


/// The VM oracle for a 2-arg call `f(a, b)`.
fn vm_f2(src: &str, a: i64, b: i64) -> Option<i64> {
    let lit = |v: i64| {
        if v < 0 {
            format!("(0 - {})", -v)
        } else {
            v.to_string()
        }
    };
    let prog = format!("{src}## Main\n    Show f({}, {}).\n", lit(a), lit(b));
    let out = vm_outcome(&prog);
    if out.error.is_some() {
        return None;
    }
    out.output.split_whitespace().next()?.parse::<i64>().ok()
}

/// MULTI-ARG extension of the engine RTL fuzz: random 2-arg recursive programs run through the REAL engine
/// Verilog and asserted ≡ VM. Multi-arg calls exercise the arg-passing/frame-save machinery that yielded bugs
/// #4 (closure args) and #5 (single-arg Call frame-save) — this is the multi-arg analog on the fabric engine.
#[test]
fn multiarg_recursive_programs_run_on_the_engine_rtl_matching_the_vm() {
    if !common::require_tool("iverilog") {
        return;
    }
    let seed: u64 = std::env::var("WPROG_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0xF00D_BEEF);
    let mut rng = Rng::new(seed);
    let (mut ran, mut boundary) = (0u64, 0u64);
    for i in 0..24u64 {
        let src = gen_recursive_2arg(&mut rng);
        // `a` values match the single-arg fuzz's proven-fitting depths; `b` is random.
        let cases: Vec<(i64, i64)> = [0i64, 1, 3, 6, 10]
            .iter()
            .map(|&a| (a, rng.below(100_000) as i64))
            .collect();
        let mut oracle = Vec::new();
        let mut vm_ok = true;
        for &(a, b) in &cases {
            match vm_f2(&src, a, b) {
                Some(v) => oracle.push(v),
                None => {
                    vm_ok = false;
                    break;
                }
            }
        }
        if !vm_ok {
            boundary += 1;
            continue;
        }
        let cfg = match catch_unwind(AssertUnwindSafe(|| program_calls_fabric(&src, "f"))) {
            Ok(c) => c,
            Err(_) => {
                boundary += 1;
                continue;
            }
        };
        if wprog_config_fits_engine(&cfg).is_err() {
            boundary += 1;
            continue;
        }
        // Require the MODEL to already equal the VM — else it's a lowering/model issue, not an RTL divergence.
        let model_ok = cases.iter().zip(&oracle).all(|(&(a, b), &o)| {
            catch_unwind(AssertUnwindSafe(|| eval_wprog_config(&cfg, &[a, b]))).ok() == Some(o)
        });
        if !model_ok {
            boundary += 1;
            continue;
        }
        let inputs_flat: Vec<i64> = cases.iter().flat_map(|&(a, b)| [a, b]).collect();
        match engine_rtl_run(&format!("ma{i}"), &cfg, &inputs_flat, &oracle) {
            Ok(f) if f.is_empty() => ran += 1,
            Ok(fails) => panic!(
                "=== MULTI-ARG WHOLE-PROGRAM ENGINE RTL DIVERGED FROM THE VM ===\n\
                 seed=0x{seed:X}  program #{i}\n  src={src:?}\n  cases(a,b)={cases:?}\n  \
                 divergences (case, rtl, vm)={fails:?}"
            ),
            Err(e) => {
                eprintln!("skip #{i}: {e}");
                boundary += 1;
            }
        }
    }
    eprintln!("MULTI-ARG ENGINE RTL FUZZ (seed=0x{seed:X}): {ran} ran on the engine RTL ≡ VM, {boundary} boundary");
    assert!(
        ran >= 10,
        "only {ran} multi-arg programs ran on the engine RTL (floor 10) — near-vacuous"
    );
}

/// Build the engine RTL for `cfg`, run it under iverilog for each input, and return whether every hardware result
/// matches `oracle`. On a divergence returns the failing `(input, rtl_result, vm_result)` triples parsed from the
/// testbench's `FAIL case` lines (so the caller can panic with a precise repro). `Err` on a toolchain failure.
///
/// This mirrors `wprog_rtl_lock::iverilog_matches` but parses the exact mismatching values out of the transcript.
fn engine_rtl_run(
    name: &str,
    cfg: &WProgConfig,
    inputs: &[i64],
    oracle: &[i64],
) -> Result<Vec<(i64, i64, i64)>, String> {
    let dir = std::env::temp_dir().join("logos_wprog_engine_rtl_fuzz");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create temp dir: {e}"))?;
    let rtl = emit_wprog_verilog(name, cfg);
    // `inputs` is FLAT and interleaved (case0_p0, case0_p1, case1_p0, …); the testbench's case count is
    // inputs.len()/n_params (== inputs.len() for the single-param programs, so this is backward-compatible).
    let n_cases = inputs.len() / cfg.n_params.max(1);
    let tb = emit_wprog_testbench(name, cfg.n_params, n_cases);
    let srcv = dir.join(format!("wprog_{name}.v"));
    let tbv = dir.join(format!("tb_wprog_{name}.v"));
    let sim = dir.join(format!("sim_wprog_{name}.vvp"));
    std::fs::write(&srcv, &rtl).map_err(|e| format!("write rtl: {e}"))?;
    std::fs::write(&tbv, &tb).map_err(|e| format!("write tb: {e}"))?;
    let pin: String = inputs
        .iter()
        .map(|&n| format!("{:016x}\n", n as u64))
        .collect();
    let oout: String = oracle
        .iter()
        .map(|&v| format!("{:016x}\n", v as u64))
        .collect();
    std::fs::write(dir.join(format!("wprog_{name}_in.hex")), pin)
        .map_err(|e| format!("write in: {e}"))?;
    std::fs::write(dir.join(format!("wprog_{name}_out.hex")), oout)
        .map_err(|e| format!("write out: {e}"))?;

    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(&srcv)
        .arg(&tbv)
        .current_dir(&dir)
        .output()
        .map_err(|e| format!("spawn iverilog: {e}"))?;
    if !build.status.success() {
        return Err(format!(
            "iverilog compile failed on {name}:\n{}\n--- RTL ---\n{rtl}",
            String::from_utf8_lossy(&build.stderr)
        ));
    }
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .current_dir(&dir)
        .output()
        .map_err(|e| format!("spawn vvp: {e}"))?;
    let stdout = String::from_utf8_lossy(&run.stdout);
    if stdout.contains("PASS:") {
        return Ok(Vec::new());
    }
    // Parse `FAIL case %0d: got %0d exp %0d` into (input, rtl_got, vm_expected) triples for a precise repro.
    let mut fails = Vec::new();
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("FAIL case ") {
            // "N: got G exp E"
            let mut idx = None;
            let mut got = None;
            let mut exp = None;
            if let Some((n, tail)) = rest.split_once(':') {
                idx = n.trim().parse::<usize>().ok();
                let mut it = tail.split_whitespace();
                while let Some(tok) = it.next() {
                    match tok {
                        "got" => got = it.next().and_then(|v| v.parse::<i64>().ok()),
                        "exp" => exp = it.next().and_then(|v| v.parse::<i64>().ok()),
                        _ => {}
                    }
                }
            }
            if let (Some(i), Some(g), Some(e)) = (idx, got, exp) {
                let input = inputs.get(i).copied().unwrap_or(i64::MIN);
                fails.push((input, g, e));
            }
        }
    }
    if fails.is_empty() {
        // The testbench neither PASSed nor emitted a parseable FAIL — surface the raw transcript.
        return Err(format!(
            "engine RTL {name} produced no PASS and no parseable FAIL:\n{stdout}"
        ));
    }
    Ok(fails)
}

/// How many random programs to generate. Each that lowers+fits is compiled to engine RTL and run under iverilog
/// (heavy), so the corpus is kept modest — a few dozen programs, running in a few minutes.
const PROGRAMS: u64 = 48;
/// Non-vacuity floor: at least this many programs must actually reach the engine RTL under iverilog. Below this
/// the generator has drifted off the lowerable surface and the fuzz is vacuous — RED (never lower this to pass a
/// shrinking corpus; fix the generator).
const RAN_FLOOR: u64 = 20;
/// The random inputs fed to every program. Kept small: recursion depth ≤ arg ≤ engine frame-stack DEPTH, and
/// loop trip-counts stay small so iverilog finishes fast (and never hits the testbench's cycle guard).
const INPUTS: [i64; 5] = [0, 1, 3, 6, 10];

#[test]
fn random_programs_run_on_the_whole_program_engine_rtl_matching_the_vm() {
    if !common::require_tool("iverilog") {
        return;
    }

    let seed: u64 = std::env::var("WPROG_FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0xF00D_BEEF);
    let mut rng = Rng::new(seed);

    let mut ran = 0u64;
    let mut boundary = 0u64;
    let mut shapes_ran: BTreeSet<(Shape, usize, usize)> = BTreeSet::new(); // (shape, #blocks, #total-cells)

    for k in 0..PROGRAMS {
        let (src, shape) = gen(&mut rng);

        // The VM oracle must exist for every input (the program is error-free by construction) — else this is a
        // GENERATOR bug, not an RTL divergence, so skip it as a counted boundary rather than blaming the silicon.
        let oracle: Vec<i64> = match INPUTS
            .iter()
            .map(|&a| vm_f(&src, a))
            .collect::<Option<Vec<_>>>()
        {
            Some(o) => o,
            None => {
                boundary += 1;
                continue;
            }
        };

        // Lower to a config that also evaluates ≡ VM in the model (else it's a lowering/model issue, not RTL).
        let cfg = match config_for(&src, &INPUTS, &oracle) {
            Some(c) => c,
            None => {
                boundary += 1;
                continue;
            }
        };

        // Must fit the fixed engine's caps — an over-cap config can't run on the one-size netlist.
        if wprog_config_fits_engine(&cfg).is_err() {
            boundary += 1;
            continue;
        }

        let n_blocks = cfg.blocks.len();
        let n_cells: usize = cfg.blocks.iter().map(|b| b.cells.len()).sum();
        let name = format!("fz{k}");

        match engine_rtl_run(&name, &cfg, &INPUTS, &oracle) {
            Ok(fails) if fails.is_empty() => {
                ran += 1;
                shapes_ran.insert((shape, n_blocks, n_cells));
            }
            Ok(fails) => {
                // A DIVERGENCE: the engine RTL disagrees with the VM. This is a real engine-silicon bug — panic
                // LOUD with the exact reproducing seed + source + the failing (input, rtl, vm) triples.
                let detail: String = fails
                    .iter()
                    .map(|(inp, rtl, vm)| format!("    input n={inp}: RTL={rtl}  VM={vm}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                panic!(
                    "\n=== WHOLE-PROGRAM ENGINE RTL DIVERGED FROM THE VM ===\n\
                     seed=0x{seed:X}  program #{k}  shape={}  blocks={n_blocks} cells={n_cells}\n\
                     --- source ---\n{src}\n--- divergences (engine RTL vs VM) ---\n{detail}\n\
                     Reproduce: WPROG_FUZZ_SEED=0x{seed:X} then re-run; the divergent config is program #{k}.\n",
                    if shape == Shape::Recursive { "recursive" } else { "iterative" },
                );
            }
            Err(e) => {
                // A toolchain/emit failure (not a value divergence) — fail loudly; it must never be swallowed.
                panic!(
                    "\nengine RTL harness error on program #{k} (seed=0x{seed:X}, shape={}):\n{e}\n--- source ---\n{src}\n",
                    if shape == Shape::Recursive { "recursive" } else { "iterative" },
                );
            }
        }
    }

    let iter_shapes = shapes_ran
        .iter()
        .filter(|(s, ..)| *s == Shape::Iterative)
        .count();
    let rec_shapes = shapes_ran
        .iter()
        .filter(|(s, ..)| *s == Shape::Recursive)
        .count();
    println!(
        "\nWHOLE-PROGRAM ENGINE RTL FUZZ (seed=0x{seed:X}): {ran} programs ran on the engine RTL ≡ VM under iverilog, \
         {boundary} boundary (didn't lower/fit or a generator no-op); {} distinct control-flow shapes reached the RTL \
         ({iter_shapes} iterative, {rec_shapes} recursive).",
        shapes_ran.len(),
    );

    // NON-VACUITY: the generator must actually reach the engine RTL, not just skip everything to boundary.
    assert!(
        ran >= RAN_FLOOR,
        "only {ran} random programs reached the engine RTL (floor {RAN_FLOOR}) — the generator drifted off the \
         lowerable surface, making this fuzz vacuous. Fix the generator (do NOT lower the floor)."
    );
    // And it must exercise BOTH lanes (loops and recursion) — a spread of shapes, not one trivial config repeated.
    assert!(
        iter_shapes >= 3 && rec_shapes >= 1,
        "the fuzz reached only {iter_shapes} iterative + {rec_shapes} recursive shapes on the RTL — too narrow to \
         be a real differential (expected a spread of both lanes)."
    );
}
