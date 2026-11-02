//! fabric_perf_lock — Phase G (Lock 2): the spatial fabric vs the interpreter, honestly.
//!
//! `benchmarks/silicon/fabric_perf.json` records, for every fabric datapath, the fabric's on-chip settle
//! (`fabric_cycles`) and the soft-core interpreter's on-chip run-cycle count (`interp_cycles`) for the SAME
//! kernel on the SAME Artix-7. This lock pins the table so a perf number can never drift from its proof:
//!
//!   1. COMPLETENESS — the table covers EXACTLY the fabric corpus (`FABRIC_SOURCES`).
//!   2. ≡VM — each kernel's `vm_result` is recomputed on the VM and must match (a speedup for a WRONG answer
//!      is worthless; the number rides on a correct result).
//!   3. HONEST SPEEDUP — `fabric_cycles == 1` (a combinational decision settles in one clock), `interp_cycles
//!      > 1`, and `speedup == interp_cycles / fabric_cycles` — the cycle reduction is derived, not asserted by
//!      magnitude (magnitudes are machine-dependent; the RATIO and its derivation are what's locked).
//!   4. NOT THE DEMO CLOCK — `asic_clock_ghz != fpga_demo_clock`, and `fabric_asic_ns == fabric_cycles /
//!      asic_clock_ghz`, so the wall-clock column can never silently become the slow FPGA bring-up clock.

#[path = "common/mod.rs"]
mod common;
#[path = "common/board_settle.rs"]
mod board_settle;

use logicaffeine_compile::codegen_sva::fabric_soc::{
    emit_config_grid, emit_nexys_fabric_soc, vectors, FABRIC_SOURCES, STATE_SOC_NAME, STATE_SOC_SRC,
};
use logicaffeine_compile::compile::vm_outcome;
use std::collections::BTreeSet;
use std::path::Path;

/// Call `FABRIC_SOURCES[cfg]`'s function with `inputs` (only the function's arity) on the VM.
fn vm_call(cfg: usize, inputs: &[i64]) -> i64 {
    let (src, name) = FABRIC_SOURCES[cfg];
    let arity = src.matches(": Int").count().max(1);
    let arg = |n: i64| {
        if n < 0 {
            format!("(0 - {})", -n)
        } else {
            n.to_string()
        }
    };
    let arglist = inputs[..arity].iter().map(|a| arg(*a)).collect::<Vec<_>>().join(", ");
    let out = vm_outcome(&format!("{src}## Main\n    Show {name}({arglist}).\n"));
    assert!(out.error.is_none(), "VM error calling {name}: {:?}", out.error);
    out.output.split_whitespace().next().expect("a Show").parse::<i64>().expect("int")
}

/// One streamed record of the board top: the config word, the result and the cycle field.
#[derive(Debug)]
struct Record {
    cfg: u8,
    result: i64,
    cycles: u16,
}

/// Run the board top under iverilog with every fabric output arriving `settle_ns` after its inputs change (a
/// transport delay wrapped around the real `config_fabric`), on a 100 MHz clock, and decode one full lap of its
/// UART stream. The board top is instantiated with a 16-clock sweep window and two repetitions, so the
/// simulation stays short; every other parameter is the flashed image's.
fn run_board_top_with_a_slow_fabric(settle_ns: u32) -> Vec<Record> {
    let dir = std::env::temp_dir().join(format!("logos_nexys_fabric_settle_{settle_ns}_{}", std::process::id()));
    board_settle::stream(
        &dir,
        &emit_nexys_fabric_soc(),
        "nexys_fabric_soc",
        ".CLKS_PER_BIT(4), .LMAX(16), .REPS(2)",
        &emit_config_grid(),
        "config_fabric",
        settle_ns,
        vectors().len(),
    )
    .unwrap_or_else(|e| panic!("the board top must take the settle sweep's window and repetitions (LMAX, REPS): {e}"))
    .into_iter()
    .map(|r| Record { cfg: r.tag, result: r.result, cycles: r.cycles })
    .collect()
}

/// The cycle field the board streams is what the board measured, never a constant: with the fabric's outputs
/// arriving 0, 15, 35 or 95 ns after its inputs on a 10 ns clock, every datapath reports the first capture
/// latency from which its result holds (1, 2, 4 and 10 clocks) and the VM's answer; with them arriving after
/// the whole 16-clock window (200 ns), every record reports 0, the mark of a result that never settled.
#[test]
fn the_fabric_cycle_count_is_read_from_a_counter_the_rtl_drives() {
    if !common::require_tool("iverilog") {
        return;
    }
    for (settle_ns, want) in [(0u32, Some(1u16)), (15, Some(2)), (35, Some(4)), (95, Some(10)), (200, None)] {
        let records = run_board_top_with_a_slow_fabric(settle_ns);
        for ((cfg, inputs), rec) in vectors().iter().zip(&records) {
            assert_eq!(rec.cfg as u64, *cfg, "records stream in the vector order: {rec:?}");
            match want {
                Some(cycles) => {
                    assert_eq!(
                        rec.result,
                        vm_call(*cfg as usize, inputs),
                        "settle {settle_ns} ns: cfg {cfg} inputs {inputs:?} streamed {rec:?}, not the VM's answer"
                    );
                    assert_eq!(
                        rec.cycles, cycles,
                        "settle {settle_ns} ns on a 10 ns clock: cfg {cfg} inputs {inputs:?} must report {cycles} cycles, streamed {rec:?}"
                    );
                }
                None => assert_eq!(
                    rec.cycles, 0,
                    "settle {settle_ns} ns is past the 16-clock window: cfg {cfg} inputs {inputs:?} must report 0, streamed {rec:?}"
                ),
            }
        }
    }
}

/// Call `FABRIC_SOURCES[cfg]`'s function on its default demo arguments (the ones the .json's `vm_result` was
/// recorded for) — the corpus programs each `Show` a fixed call, so we run the same program the perf kernel
/// compiled. Returns the VM's printed integer.
fn vm_result_for(name: &str) -> i64 {
    // The perf kernels Show a fixed call; reuse the same demo inputs the on-chip measurement used.
    let call = match name {
        "affine" => "affine(3)",
        "clamp" => "clamp(5, 0, 10)",
        "sign" => "sign(9)",
        "min2" => "min2(3, 8)",
        "ratio" => "ratio(10, 3)",
        other => panic!("no demo call known for kernel {other}"),
    };
    let (src, _) = FABRIC_SOURCES
        .iter()
        .find(|(_, n)| *n == name)
        .expect("kernel in corpus");
    let prog = format!("{src}## Main\n    Show {call}.\n");
    let out = vm_outcome(&prog);
    assert!(out.error.is_none(), "VM error for {name}: {:?}", out.error);
    out.output
        .split_whitespace()
        .next()
        .expect("a Show")
        .parse::<i64>()
        .expect("int")
}

#[test]
fn the_fabric_perf_table_is_honest_and_internally_consistent() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let json = std::fs::read_to_string(root.join("benchmarks/silicon/fabric_perf.json"))
        .expect("benchmarks/silicon/fabric_perf.json must exist");
    let doc: serde_json::Value = serde_json::from_str(&json).expect("fabric_perf.json parses");

    let asic_ghz = doc["asic_clock_ghz"].as_f64().expect("asic_clock_ghz");
    let demo_mhz = doc["fpga_demo_clock_mhz"]
        .as_f64()
        .expect("fpga_demo_clock_mhz");
    // (4) the honest metric is an ASIC clock, NEVER the slow FPGA bring-up clock.
    assert!(
        (asic_ghz * 1000.0 - demo_mhz).abs() > 1.0,
        "asic_clock_ghz must differ from the FPGA demo clock (the cycles/op metric is not the demo wall clock)"
    );

    // The overhead baseline (a bare call + Show) — decision_cycles isolates the marginal decision logic.
    let baseline = doc["decomposition"]["baseline_overhead_cycles"]
        .as_u64()
        .expect("baseline_overhead_cycles");
    assert!(
        baseline > 0,
        "the overhead baseline must be a positive on-chip measurement"
    );

    let kernels = doc["kernels"].as_array().expect("kernels array");

    // (1) COMPLETENESS — the table's kernel set == the fabric corpus.
    let table: BTreeSet<String> = kernels
        .iter()
        .map(|k| k["name"].as_str().unwrap().to_string())
        .collect();
    let corpus: BTreeSet<String> = FABRIC_SOURCES.iter().map(|(_, n)| n.to_string()).collect();
    assert_eq!(
        table, corpus,
        "the perf table must cover EXACTLY the fabric corpus"
    );

    for k in kernels {
        let name = k["name"].as_str().unwrap();
        let fabric_cycles = k["fabric_cycles"].as_u64().unwrap();
        let interp_cycles = k["interp_cycles"].as_u64().unwrap();
        let speedup = k["speedup"].as_u64().unwrap();
        let vm_result = k["vm_result"].as_i64().unwrap();
        let asic_ns = k["fabric_asic_ns"].as_f64().unwrap();

        // (2) ≡VM — the recorded result must be what the VM computes.
        assert_eq!(
            vm_result,
            vm_result_for(name),
            "{name}: vm_result drifted from the VM"
        );

        // (3) HONEST SPEEDUP — combinational settle of 1, a real interpreter cost > 1, derived ratio.
        assert_eq!(
            fabric_cycles, 1,
            "{name}: a spatial decision settles in ONE clock (fabric_cycles == 1)"
        );
        assert!(
            interp_cycles > 1,
            "{name}: interp_cycles must be a positive on-chip measurement > 1"
        );
        assert!(
            interp_cycles > fabric_cycles,
            "{name}: the fabric must be faster than the interpreter"
        );
        assert_eq!(
            speedup,
            interp_cycles / fabric_cycles,
            "{name}: speedup must equal interp_cycles / fabric_cycles (derived, not hand-typed)"
        );

        // (4) derived wall-clock consistency: fabric_asic_ns == fabric_cycles / asic_ghz.
        let expect_ns = fabric_cycles as f64 / asic_ghz;
        assert!(
            (asic_ns - expect_ns).abs() < 1e-6,
            "{name}: fabric_asic_ns {asic_ns} != fabric_cycles/asic_ghz {expect_ns}"
        );

        // (5) DECOMPOSITION consistency — decision_cycles isolates the marginal decision logic (interp minus
        // the shared overhead baseline), is positive (a real decision does real work), and derives its own
        // speedup. This keeps BOTH the end-to-end and the decision-isolated speedups honest and pinned.
        let decision_cycles = k["decision_cycles"].as_u64().unwrap();
        let decision_speedup = k["decision_speedup"].as_u64().unwrap();
        assert_eq!(
            decision_cycles,
            interp_cycles - baseline,
            "{name}: decision_cycles must equal interp_cycles - baseline (the isolated decision cost)"
        );
        assert!(
            decision_cycles > 0,
            "{name}: the decision must cost > 0 cycles on the interpreter"
        );
        assert_eq!(
            decision_speedup,
            decision_cycles / fabric_cycles,
            "{name}: decision_speedup must equal decision_cycles / fabric_cycles"
        );
    }

    // STAGE X2c — the STATE PLANE on silicon. Its on-chip measurement (a SEQUENCED heap datapath, not a
    // 1-clock combinational decision) is pinned the same way: the recorded result must be what the VM computes,
    // the cycle count must be honestly MULTI-cycle (> 1), the utilization must derive from the LUT count, and
    // the timing must PASS (Fmax > the 12 MHz demo constraint — the state plane routes where the divider did not).
    let sp = &doc["state_plane"];
    let sp_result = sp["vm_result_first_vector"]
        .as_i64()
        .expect("state_plane result");
    let want_sp = {
        // grade(10, 20, 30) = clamp(60, 0, 100) = 60 — the VM oracle for the first flashed vector.
        let prog = format!("{STATE_SOC_SRC}## Main\n    Show {STATE_SOC_NAME}(10, 20, 30).\n");
        let out = vm_outcome(&prog);
        assert!(out.error.is_none(), "state_plane VM error: {:?}", out.error);
        out.output
            .split_whitespace()
            .next()
            .unwrap()
            .parse::<i64>()
            .unwrap()
    };
    assert_eq!(
        sp_result, want_sp,
        "state_plane vm_result drifted from the VM (grade(10,20,30))"
    );
    let sp_cycles = sp["fabric_cycles"]
        .as_u64()
        .expect("state_plane fabric_cycles");
    assert!(sp_cycles > 1, "the state plane is a SEQUENCED heap datapath — fabric_cycles must be > 1 (honest multi-cycle)");
    let sp_luts = sp["slice_luts"].as_u64().expect("state_plane slice_luts");
    let sp_avail = sp["of_available_luts"]
        .as_f64()
        .expect("state_plane of_available_luts");
    let sp_util = sp["utilization_pct"]
        .as_f64()
        .expect("state_plane utilization_pct");
    let expect_util = (sp_luts as f64 / sp_avail * 10000.0).round() / 100.0;
    assert!(
        (sp_util - expect_util).abs() < 0.01,
        "state_plane utilization_pct {sp_util} != slice_luts/of_available_luts {expect_util} (derived, not hand-typed)"
    );
    let sp_fmax = sp["fmax_mhz"].as_f64().expect("state_plane fmax_mhz");
    assert!(
        sp_fmax > 12.0,
        "the state plane must MEET the 12 MHz demo timing (Fmax {sp_fmax} — it routes where the combinational divider did not)"
    );

    // STAGE X3b — the FLOAT LANE on silicon. IEEE-754 f64 arithmetic (fp_mul/fp_addsub/fp_sqrt) is a MULTI-cycle
    // pipeline that must run on a DIVIDED core clock (its deep shifter stages fail at 100 MHz). Pin: the floored
    // result matches the f64 reference, the cycle count is honestly multi-cycle, the utilization derives from
    // the LUT count, the core clock derives from the divider, and the Fmax clears the 12 MHz constraint.
    let fl = &doc["float_lane"];
    let fl_floor = fl["vm_result_first_vector_floor"]
        .as_i64()
        .expect("float_lane result floor");
    // floor(sqrt(3*3 + 4*4)) = 5 — the f64 reference for the first flashed vector.
    let want_fl = ((3.0_f64 * 3.0 + 4.0 * 4.0).sqrt()).floor() as i64;
    assert_eq!(
        fl_floor, want_fl,
        "float_lane floored result drifted from the f64 reference (hypot(3,4))"
    );
    let fl_cycles = fl["fabric_cycles"]
        .as_u64()
        .expect("float_lane fabric_cycles");
    assert!(
        fl_cycles > 1,
        "the float lane is a MULTI-cycle FPU pipeline — fabric_cycles must be > 1"
    );
    let fl_luts = fl["slice_luts"].as_u64().expect("float_lane slice_luts");
    let fl_avail = fl["of_available_luts"]
        .as_f64()
        .expect("float_lane of_available_luts");
    let fl_util = fl["utilization_pct"]
        .as_f64()
        .expect("float_lane utilization_pct");
    let expect_fl_util = (fl_luts as f64 / fl_avail * 10000.0).round() / 100.0;
    assert!(
        (fl_util - expect_fl_util).abs() < 0.01,
        "float_lane utilization_pct {fl_util} != slice_luts/of_available_luts {expect_fl_util}"
    );
    let fl_div = fl["core_clock_divider"]
        .as_f64()
        .expect("float_lane core_clock_divider");
    let fl_core = fl["core_clock_mhz"]
        .as_f64()
        .expect("float_lane core_clock_mhz");
    assert!(
        (fl_core - 100.0 / fl_div).abs() < 1e-6,
        "float_lane core_clock_mhz {fl_core} != 100 / divider {}",
        100.0 / fl_div
    );
    let fl_fmax = fl["fmax_mhz"].as_f64().expect("float_lane fmax_mhz");
    assert!(
        fl_fmax > 12.0,
        "the float lane core clock must clear 12 MHz timing (Fmax {fl_fmax})"
    );
    assert!(
        fl_core < fl_fmax,
        "the divided core clock {fl_core} MHz must run UNDER the FPU Fmax {fl_fmax} MHz (why the divider exists)"
    );

    let total_speedup: u64 = kernels.iter().map(|k| k["speedup"].as_u64().unwrap()).sum();
    let avg = total_speedup / kernels.len() as u64;
    println!(
        "Phase G: {} datapaths, each 1 clock on the spatial fabric vs ~{} on the interpreter — mean {}x fewer cycles/decision, same Artix-7.",
        kernels.len(),
        kernels.iter().map(|k| k["interp_cycles"].as_u64().unwrap()).sum::<u64>() / kernels.len() as u64,
        avg
    );
}
