//! ═══════════════════════════════════════════════════════════════════════
//! PLASTICINE REPORT LOCK — P4 of the Plasticine lane: the artifact an RDU engineer reads, generated from
//! the model and pinned here so no number in it can be typed by hand.
//!
//! A. `benchmarks/silicon/plasticine_report.json` regenerates BYTE-EQUAL from the model — pinned where the
//!    negotiated router that routed every row lives, the private place-and-route engine's
//!    `plasticine_negotiated_lock`.
//! B. Every counted-loop row is anchored to the VM: the VM's answer == the fabric's == the cycle-timed
//!    pipelined run at the modeled II with zero hazards, and one cycle below the II the pipeline hazards
//!    (the II is tight, not vacuous). The anchor is recomputed HERE, independently of the report's own, on
//!    the calibrated PCU model (`UnitModel::plasticine_pcu`: a carried register costs the pipeline depth + 1).
//! C. Every row and the report are labeled MODELED; the word "measured" never appears — nothing here ran
//!    on a Plasticine simulator or silicon (that is P5's `simulated_tungsten` provenance).
//! D. The GEMM rows obey the roofline arithmetic they print: cycles = max(compute, DRAM), the bound named,
//!    GFLOPS and % of the published fp32 peak recomputed, the A+B panel double-buffered in ONE PMU, and the
//!    automatic unroll MAXIMAL (one more instance exceeds capacity, the work, or the static link budget).
//! E. Utilization / routing metrics are recomputed from the placement (PCU/PMU/FU %, hop counts, static
//!    routability against the Spade link budget); `par` unrolling multiplies the per-instance allocation.
//! F. `compile_to_plasticine` yields a placed artifact with a SAT certificate (re-checked; a tampered
//!    certificate fails) and Spatial source naming the loop, its register and its recurrence; a program
//!    that is not a counted loop is refused TYPED (`plasticine backend:`), never modeled wrong.
//! G. The report names, with a reason, every canonical workload the model does NOT cover. (That the two
//!    targets differ where the papers differ depends on the negotiated router's unroll, so it is pinned in
//!    the private place-and-route engine's `plasticine_negotiated_lock`.)
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(plasticine_report_lock)'`
//! ═══════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

use logicaffeine_compile::codegen_sva::fabric_rtl::{eval_loop_fabric, loop_fabric_from_source};
use logicaffeine_compile::codegen_sva::metapipeline::{eval_loop_fabric_at_ii, UnitModel};
use logicaffeine_compile::codegen_sva::plasticine::PlasticineTarget;
use logicaffeine_compile::codegen_sva::plasticine_place::{place_unrolled, LinkBudget};
use logicaffeine_compile::codegen_sva::plasticine_report::{
    canonical_targets, canonical_workloads, gemm_metapipeline, logos_gemm_program,
    lower_counted_loop, lower_gemm_source, WorkloadKind, WorkloadRow, PROVENANCE_MODELED,
    REPORT_PATH,
};
use logicaffeine_compile::compile::{compile_to_plasticine, vm_outcome};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn report_text() -> String {
    std::fs::read_to_string(repo_root().join(REPORT_PATH))
        .unwrap_or_else(|e| panic!("{REPORT_PATH} must exist ({e}) — regenerate it with the private place-and-route engine's report emitter, emit_plasticine_report"))
}

fn report() -> serde_json::Value {
    serde_json::from_str(&report_text()).expect("the report parses")
}

fn rows() -> Vec<WorkloadRow> {
    serde_json::from_value(report()["rows"].clone()).expect("rows deserialize into WorkloadRow")
}

fn r3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn vm_f(source: &str, fn_name: &str, params: &[i64]) -> i64 {
    let args = params
        .iter()
        .map(|p| {
            if *p < 0 {
                format!("(0 - {})", -p)
            } else {
                p.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let prog = format!("{source}## Main\n    Show {fn_name}({args}).\n");
    let out = vm_outcome(&prog);
    assert!(
        out.error.is_none(),
        "VM error for {fn_name}({args}): {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("an integer")
        .parse()
        .expect("i64")
}

const SUM: &str = "## To f (n: Int) -> Int:\n    Let mutable s be 1.\n    Repeat for i from 1 to n:\n        Set s to s * 3 + i.\n    Return s.\n";

// ───────────────────────────── B ─────────────────────────────

#[test]
fn every_counted_loop_row_is_anchored_to_the_vm_and_its_ii_is_tight() {
    let rows = rows();
    let mut anchored = 0usize;
    let mut tight = 0usize;
    for w in canonical_workloads() {
        let WorkloadKind::CountedLoop {
            source,
            fn_name,
            params,
        } = &w.kind
        else {
            continue;
        };
        let vm = vm_f(source, fn_name, params);
        let lf = loop_fabric_from_source(source, fn_name);
        let fabric = eval_loop_fabric(&lf, params);
        assert_eq!(vm, fabric, "{}: fabric ≠ VM", w.name);
        for t in canonical_targets() {
            let row = rows
                .iter()
                .find(|r| r.name == w.name && r.target == t.name)
                .unwrap_or_else(|| panic!("row {} on {}", w.name, t.name));
            let a = row
                .vm_anchor
                .as_ref()
                .unwrap_or_else(|| panic!("{}: a counted-loop row carries its VM anchor", w.name));
            assert_eq!(a.params, *params);
            assert_eq!(a.vm, vm, "{}: the report's VM value is the VM's", w.name);
            assert_eq!(a.fabric, vm);
            assert_eq!(
                a.pipelined_at_ii, vm,
                "{}: the pipelined run at II {} reproduces the VM",
                w.name, row.pipeline_ii
            );
            assert_eq!(
                a.hazards_at_ii, 0,
                "{}: no hazards at the modeled II",
                w.name
            );
            // The report models counted loops on the calibrated PCU (a carried register costs the pipeline
            // depth + 1 per iteration — what Tungsten simulates; plasticine_sim_lock holds it there).
            let model = UnitModel::plasticine_pcu(t.pcu_stages);
            let run = eval_loop_fabric_at_ii(&lf, params, row.pipeline_ii, &model);
            assert_eq!(
                (run.value, run.hazards),
                (vm, 0),
                "{}: recomputed pipelined run",
                w.name
            );
            if row.pipeline_ii > 1 {
                let below = a.hazards_below_ii.unwrap_or_else(|| {
                    panic!(
                        "{}: an II above 1 records the hazards one cycle below it",
                        w.name
                    )
                });
                assert!(
                    below > 0,
                    "{}: II {} is tight — one cycle less must hazard",
                    w.name,
                    row.pipeline_ii
                );
                let again = eval_loop_fabric_at_ii(&lf, params, row.pipeline_ii - 1, &model);
                assert_eq!(again.hazards, below);
                tight += 1;
            } else {
                assert_eq!(a.hazards_below_ii, None);
            }
            assert_eq!(row.kind, "counted_loop");
            assert_eq!(
                row.stages.len(),
                1,
                "a counted loop is one metapipeline stage"
            );
            assert_eq!(row.buffers.len(), 0, "a scalar counted loop streams its induction variable from the counter chain — no PMU");
            assert_eq!(row.pmu_used, 0);
            assert!(row.pcu_used >= 1);
            assert_eq!(
                row.iterations,
                (lf.hi.eval(params, &[]) - lf.lo.eval(params, &[]) + 1) as usize
            );
            assert_eq!(row.operand_stall_cycles, 0, "a counted loop streams no operands from memory");
            assert_eq!(row.modeled_cycles, row.compute_cycles);
            assert_eq!(row.bound, "compute");
            assert_eq!(row.dram_bytes, 0);
            anchored += 1;
        }
    }
    assert!(
        anchored >= 14,
        "seven counted loops on two targets, got {anchored}"
    );
    assert!(tight >= 12, "every loop with a carried register has an II above 1 on the calibrated PCU (tight on both targets), got {tight}");
    let carried: Vec<&WorkloadRow> = rows
        .iter()
        .filter(|r| r.kind == "counted_loop" && r.name != "loop-no-carry")
        .collect();
    for r in &carried {
        assert_eq!(r.pipeline_ii, 7, "{}: a carried register on the six-stage PCU costs 6 + 1 per iteration (Tungsten-calibrated)", r.name);
    }
    let free = rows.iter().find(|r| r.name == "loop-no-carry").unwrap();
    assert_eq!(
        free.pipeline_ii, 1,
        "no carried register: the loop issues every cycle"
    );
}

// ───────────────────────────── C ─────────────────────────────

#[test]
fn every_row_is_labeled_modeled_and_the_report_never_says_measured() {
    let text = report_text();
    assert!(
        !text.to_lowercase().contains("measured"),
        "a MODELED report never says 'measured'"
    );
    for row in rows() {
        assert_eq!(row.provenance, PROVENANCE_MODELED, "{}", row.name);
    }
    let doc = report();
    assert!(
        doc["honesty"].as_str().unwrap().contains("simulator"),
        "the honesty line says what did not run"
    );
    assert!(
        doc["model"]["ii"].as_str().unwrap().contains("PLDI"),
        "the II definition is cited to Spatial"
    );
}

// ───────────────────────────── D ─────────────────────────────

#[test]
fn the_gemm_rows_obey_the_roofline_arithmetic() {
    let mut seen = 0usize;
    for row in rows().into_iter().filter(|r| r.kind == "gemm") {
        let t = canonical_targets()
            .into_iter()
            .find(|t| t.name == row.target)
            .unwrap();
        let g = row
            .gemm
            .as_ref()
            .unwrap_or_else(|| panic!("{}: a gemm row carries its shape", row.name));
        assert_eq!(
            row.modeled_cycles,
            (row.compute_cycles + row.operand_stall_cycles).max(row.dram_cycles),
            "{}",
            row.name
        );
        assert_eq!(
            row.bound,
            if row.dram_cycles > row.compute_cycles + row.operand_stall_cycles {
                "dram"
            } else {
                "compute"
            },
            "{}",
            row.name
        );
        let dram_cycles = (row.dram_bytes as f64 * t.clock_ghz / t.dram.gbps).ceil() as usize;
        assert_eq!(
            row.dram_cycles, dram_cycles,
            "{}: DRAM cycles = bytes × GHz / (GB/s)",
            row.name
        );
        let col_tiles = (g.n + g.tile_n - 1) / g.tile_n;
        let row_tiles = (g.m + g.tile_m - 1) / g.tile_m;
        assert_eq!((g.col_tiles, g.row_tiles), (col_tiles, row_tiles));
        assert_eq!(
            g.iters_per_tile,
            g.tile_m * g.tile_n * ((g.k + g.lanes - 1) / g.lanes)
        );
        assert_eq!(
            row.iterations,
            ((col_tiles + row.par - 1) / row.par) * row_tiles * g.iters_per_tile,
            "{}: iterations per unrolled instance",
            row.name
        );
        // The operands stream chunk by chunk at the reduce's lane width: the first chunk loads before any compute,
        // and a chunk whose load outlasts its compute stalls the MAC for the difference.
        let ceil = |a: usize, b: usize| (a + b - 1) / b;
        assert_eq!(g.chunk_load_cycles, ceil(g.k_tile * g.tile_n, g.lanes), "{}: one chunk's load", row.name);
        let per_tile = ceil(ceil(g.k, g.k_tile), g.kk);
        assert_eq!(g.chunks_in_sequence, ceil(col_tiles, row.par) * row_tiles * per_tile, "{}", row.name);
        assert_eq!(g.chunk_compute_cycles, ceil(g.iters_per_tile, per_tile), "{}", row.name);
        assert_eq!(
            row.operand_stall_cycles,
            g.chunk_load_cycles + g.chunks_in_sequence * g.chunk_load_cycles.saturating_sub(g.chunk_compute_cycles),
            "{}: the first chunk's load and every stall a chunk's compute does not hide",
            row.name
        );
        // DRAM traffic is B once, A as many times as the operand-sharing decision says, and C once. This
        // read `col_tiles * m * k` for A, from when A was unconditionally re-streamed per column panel.
        // Sharing turned that into a COST DECISION — broadcast fetches A once and pays fabric hops,
        // replication fetches it per instance and pays DRAM — so a fixed multiplier is no longer the
        // specification. What must hold is the structure, that the decision is one of its two legal
        // outcomes, and that the copies charged match the decision recorded.
        let ops = row
            .operands
            .as_ref()
            .unwrap_or_else(|| panic!("{}: a gemm row records its operand decisions", row.name));
        let a = ops
            .iter()
            .find(|o| o.name == "A")
            .unwrap_or_else(|| panic!("{}: the A operand", row.name));
        let b = ops
            .iter()
            .find(|o| o.name == "B")
            .unwrap_or_else(|| panic!("{}: the B operand", row.name));
        assert_eq!(
            b.copies, 1,
            "{}: instances own disjoint column panels, so every B byte is fetched once",
            row.name
        );
        assert_eq!(
            b.bytes_moved,
            4 * g.k * g.n,
            "{}: B is the whole matrix, once",
            row.name
        );
        assert!(
            a.copies == 1 || a.copies == row.par,
            "{}: A is either broadcast (one fetch, shared over the fabric) or replicated (one fetch per \
             instance) — {} copies at par {} is neither",
            row.name, a.copies, row.par
        );
        // At par 1 the two decisions coincide — a single instance fetches A once either way — so the copy
        // count identifies the decision only when there is more than one consumer to share between.
        let expected_copies = if a.sharing == "broadcast" { 1 } else { row.par };
        assert_eq!(
            a.copies, expected_copies,
            "{}: the copies charged must match the decision recorded ({:?} at par {})",
            row.name, a.sharing, row.par
        );
        assert_eq!(
            a.bytes_moved,
            a.copies * 4 * g.m * g.k,
            "{}: A's traffic is its copies",
            row.name
        );
        assert_eq!(
            row.dram_bytes,
            4 * (g.k * g.n + a.copies * g.m * g.k + g.m * g.n),
            "{}: B once + A per the sharing decision + C once",
            row.name
        );
        let flops = 2.0 * g.m as f64 * g.n as f64 * g.k as f64;
        let gflops = r3(flops * t.clock_ghz / row.modeled_cycles as f64);
        assert_eq!(row.gflops, Some(gflops), "{}", row.name);
        let pct = r3(gflops / (t.peak_fp32_tflops * 1000.0) * 100.0);
        assert_eq!(row.pct_of_peak_fp32, Some(pct), "{}", row.name);
        assert!(pct > 0.0 && pct <= 100.0, "{}: {pct}% of peak", row.name);
        assert_eq!(row.stages.len(), 3, "load → mac-reduce → store");
        assert_eq!(
            row.stages
                .iter()
                .map(|s| s.host.as_str())
                .collect::<Vec<_>>(),
            vec!["ag", "pcu", "ag"]
        );
        assert_eq!(row.buffers.len(), 2);
        assert_eq!(
            row.buffers[0].words,
            g.tile_m * g.k + g.k * g.tile_n,
            "{}: the A+B panel",
            row.name
        );
        assert_eq!(row.buffers[0].depth, 2, "adjacent stages double-buffer");
        assert_eq!(
            row.buffers[0].pmus, 1,
            "{}: the tile is chosen so the double-buffered A+B panel fits ONE PMU",
            row.name
        );
        assert_eq!(row.buffers[1].words, g.tile_m * g.tile_n);
        assert!(
            row.pipeline_ii == 1,
            "{}: a conflict-free 16-lane MAC-reduce issues every cycle (memory II {})",
            row.name,
            row.stages[1].memory_ii
        );
        assert!(
            row.par <= row.par_capacity,
            "{}: the unroll never exceeds capacity",
            row.name
        );
        let per_instance_pcus: usize = row.stages.iter().map(|s| s.pcus).sum();
        let per_instance_pmus: usize = row.buffers.iter().map(|b| b.pmus).sum();
        // Capacity = the tightest of PCUs, PMUs, and the perimeter DRAM ports (rows × 2 sides × vlink per
        // direction; one inbound stream per DRAM-written PMU, one outbound per DRAM-read PMU).
        let ports = t.pcu_rows * 2 * row.vector_links;
        let inbound: usize = row
            .buffers
            .iter()
            .filter(|b| row.stages[b.writer].host == "ag")
            .map(|b| b.pmus)
            .sum();
        let outbound: usize = row
            .buffers
            .iter()
            .filter(|b| row.stages[b.reader].host == "ag")
            .map(|b| b.pmus)
            .sum();
        let expected = (row.pcu_total / per_instance_pcus)
            .min(row.pmu_total / per_instance_pmus)
            .min(ports / inbound.max(1))
            .min(ports / outbound.max(1));
        assert_eq!(
            row.par_capacity, expected,
            "{}: capacity = the tightest of the PCU, PMU and perimeter-port budgets",
            row.name
        );
        if row.name.starts_with("gemm-") {
            assert!(row.par > 1, "{}: a GEMM unrolls across the mesh", row.name);
            // The automatic unroll is MAXIMAL: one more instance would exceed capacity, exceed the column
            // panels there are to hand out, or fail to route within the static link budget (recomputed).
            let work_bound = row.par_capacity.min(g.col_tiles);
            if row.par < work_bound {
                let (mp, words, ags) = gemm_metapipeline(g.k, g.tile_n, &t);
                let wider = place_unrolled(
                    &mp,
                    &words,
                    &ags,
                    row.par + 1,
                    &t,
                    LinkBudget::spade_default(),
                )
                .expect("one more instance still fits by capacity");
                assert!(
                    !wider.routed_statically,
                    "{}: par {} is not maximal — {} instances still route within vlink {}",
                    row.name,
                    row.par,
                    row.par + 1,
                    row.vector_links
                );
            } else {
                assert_eq!(
                    row.par, work_bound,
                    "{}: the unroll fills the tighter of capacity and work",
                    row.name
                );
            }
            assert!(
                row.router == "pnr-negotiated" || row.router == "pnr-exact",
                "{}: routed by our pnr engine, got {}",
                row.name,
                row.router
            );
        }
        assert_eq!(row.ag_stages, 2 * row.par);
        seen += 1;
    }
    assert!(
        seen >= 6,
        "three GEMM-class workloads on two targets, got {seen}"
    );
}

// ───────────────────────────── E ─────────────────────────────

#[test]
fn utilization_and_routing_metrics_are_recomputed_from_the_placement() {
    for row in rows() {
        assert!(row.par >= 1, "{}", row.name);
        let per_instance_pcus: usize = row.stages.iter().map(|s| s.pcus).sum();
        assert_eq!(
            row.pcu_used,
            per_instance_pcus * row.par,
            "{}: par unrolling replicates the PCU allocation",
            row.name
        );
        let per_instance_pmus: usize = row.buffers.iter().map(|b| b.pmus).sum();
        assert_eq!(row.pmu_used, per_instance_pmus * row.par, "{}", row.name);
        assert!(
            row.pcu_used <= row.pcu_total && row.pmu_used <= row.pmu_total,
            "{}",
            row.name
        );
        assert_eq!(
            row.pcu_utilization_pct,
            r3(row.pcu_used as f64 / row.pcu_total as f64 * 100.0),
            "{}",
            row.name
        );
        assert_eq!(
            row.pmu_utilization_pct,
            r3(row.pmu_used as f64 / row.pmu_total as f64 * 100.0),
            "{}",
            row.name
        );
        assert!(
            row.fu_utilization_pct > 0.0 && row.fu_utilization_pct <= 100.0,
            "{}: FU utilization {}",
            row.name,
            row.fu_utilization_pct
        );
        assert_eq!(
            row.routed_statically,
            row.max_link_occupancy <= row.vector_links,
            "{}",
            row.name
        );
        assert!(
            row.routed_statically,
            "{}: every canonical workload routes statically within Spade's vlink budget",
            row.name
        );
        assert!(row.max_hops <= row.total_hops, "{}", row.name);
        let chain_links: usize = row
            .stages
            .iter()
            .map(|s| s.pcus.saturating_sub(1))
            .sum::<usize>()
            * row.par;
        if row.buffers.is_empty() && chain_links == 0 {
            assert_eq!(
                row.total_hops, 0,
                "{}: no buffers, no chains, no routes",
                row.name
            );
        } else {
            assert!(
                row.total_hops >= chain_links + row.buffers.len() * row.par,
                "{}: every chain link and buffer edge is at least one hop",
                row.name
            );
        }
        assert!(
            row.router.starts_with("pnr-"),
            "{}: routed through the private place-and-route engine, got {}",
            row.name,
            row.router
        );
        let ii = row.stages.iter().map(|s| s.ii).max().unwrap();
        assert_eq!(
            row.pipeline_ii, ii,
            "{}: the pipeline II is the slowest stage",
            row.name
        );
        assert!(
            row.stages.iter().any(|s| s.critical && s.ii == ii),
            "{}: the critical stage is flagged",
            row.name
        );
        assert_eq!(
            row.critical_stage,
            row.stages.iter().find(|s| s.ii == ii).unwrap().name
        );
        for s in &row.stages {
            assert_eq!(
                s.ii,
                s.resource_ii.max(s.recurrence_ii).max(s.memory_ii).max(1),
                "{}/{}",
                row.name,
                s.name
            );
            if s.host == "ag" {
                assert_eq!(s.pcus, 0, "an address-generator stage takes no PCU");
            } else {
                assert!(s.pcus >= 1);
            }
        }
        assert_eq!(row.throughput_iters_per_cycle, r3(1.0 / ii as f64));
        assert_eq!(
            row.fill_cycles,
            row.stages.iter().map(|s| s.latency).sum::<usize>()
        );
        let expected_compute = if row.iterations == 0 {
            0
        } else {
            row.fill_cycles + (row.iterations - 1) * ii
        };
        assert_eq!(
            row.compute_cycles, expected_compute,
            "{}: fill + (iters − 1) × II",
            row.name
        );
    }
}

// ───────────────────────────── F ─────────────────────────────

#[test]
fn compile_to_plasticine_yields_a_placed_certified_artifact_with_spatial_source() {
    let art = compile_to_plasticine(SUM).expect("a counted loop lowers");
    assert_eq!(art.fn_name, "f");
    assert_eq!(art.target, "plasticine-isca17");
    assert!(art.placement.routed_statically);
    assert_eq!(
        art.row.pipeline_ii, 7,
        "s*3+i carried through the six-stage PCU: pipeline depth + 1 (Tungsten-calibrated)"
    );
    let cert = art
        .certificate
        .as_ref()
        .expect("an ALU-only body carries a SAT placement certificate");
    assert!(cert.recheck(), "the certificate re-checks independently");
    assert!(
        !cert.with_last_step_removed().recheck(),
        "a tampered certificate fails"
    );
    let c = art.row.certificate.as_ref().unwrap();
    assert_eq!(c.steps, cert.len());
    assert!(c.rechecked);
    assert!(
        c.proves.contains("4-bit"),
        "the certificate names its width: {}",
        c.proves
    );
    // The LOGOS init is peeled into the first iteration (`mux(iv === lo, init, s.value)`): PIR refuses a
    // register constructed with a non-zero init and a bare init write before the loop.
    for needle in [
        "@spatial object Logos_f extends SpatialApp",
        "Foreach(",
        "Reg[Long]",
        "s0 := ",
        "ArgIn[Long]",
        "ArgOut[Long]",
        "Accel {",
        "val c0 = mux(iv === 1.to[Long], 1.to[Long], s0.value)",
        "c0 * 3.to[Long]",
    ] {
        assert!(
            art.spatial.contains(needle),
            "Spatial source must contain {needle:?}:\n{}",
            art.spatial
        );
    }
    assert!(
        art.spatial
            .contains("Foreach(1 until (in0.value).to[Int] + 1)"),
        "the Foreach counts the LOGOS range 1..=n:\n{}",
        art.spatial
    );
    let anchor = art.row.vm_anchor.as_ref().unwrap();
    assert_eq!(anchor.vm, anchor.pipelined_at_ii);
    for needle in [
        "@spatial class Logos_f extends SpatialTest",
        "override def runtimeArgs: Args = \"8\"",
        &format!("val gold = {}.to[Long]", anchor.vm),
        "PASS: ",
        "assert(cksum)",
    ] {
        assert!(
            art.spatial_test.contains(needle),
            "the regression-harness app must contain {needle:?}:\n{}",
            art.spatial_test
        );
    }
    assert_eq!(art.placement_json().matches("routed_statically").count(), 1);
    assert!(art.row_json().contains("\"provenance\": \"modeled\""));

    let selecty = "## To f (n: Int) -> Int:\n    Let mutable s be 1.\n    Repeat for i from 1 to n:\n        If s is greater than 10:\n            Set s to s - i.\n        Else:\n            Set s to s + i * 2.\n    Return s.\n";
    let art2 = compile_to_plasticine(selecty).expect("a select body lowers");
    assert!(
        art2.certificate.is_none(),
        "a select body is outside the bounded ALU fragment"
    );
    assert!(
        art2.row
            .certificate_omitted
            .as_ref()
            .unwrap()
            .contains("select"),
        "the omission names its reason"
    );
    assert!(
        art2.spatial.contains("mux("),
        "a select is a Spatial mux:\n{}",
        art2.spatial
    );
}

#[test]
fn compile_to_plasticine_refuses_non_counted_loop_programs_typed() {
    let whiley = "## To f (x: Int) -> Int:\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is at most x:\n        Set s to s + i.\n        Set i to i + 1.\n    Return s.\n";
    let err = format!(
        "{:?}",
        compile_to_plasticine(whiley)
            .err()
            .expect("a while loop is refused")
    );
    assert!(err.contains("plasticine backend:"), "typed refusal: {err}");
    assert!(
        err.contains("counted range loop"),
        "the refusal says what the model takes: {err}"
    );
    let no_fn = "## Main\n    Show 1.\n";
    let err = format!(
        "{:?}",
        compile_to_plasticine(no_fn)
            .err()
            .expect("no function is refused")
    );
    assert!(err.contains("no region function"), "{err}");
    let t = PlasticineTarget::isca17();
    let err = lower_counted_loop(SUM, "f", &[1, 2], &t)
        .err()
        .expect("arity mismatch refused");
    assert!(err.contains("parameters"), "{err}");
}

/// A LOGOS matmul (the triple loop the tensor recognizer reads) is emitted as Spatial's own GEMM structure with
/// our shape and tiles — the regression app the public chain runs — and tiles that do not divide the shape
/// are refused typed.
#[test]
fn a_logos_matmul_emits_spatials_gemm_structure_with_our_shape() {
    let src = logos_gemm_program(640, 640, 640);
    let (m, n, k, class, spatial) = lower_gemm_source(&src, 64, 64, 16).expect("a matmul lowers");
    assert_eq!((m, n, k), (640, 640, 640));
    assert_eq!(class, "Logos_gemm_640x640x640");
    for needle in [
        "@spatial class Logos_gemm_640x640x640 extends SpatialTest",
        "val M = 640",
        "val N = 640",
        "val K = 640",
        "val ts = 64",
        "val its = 64",
        "val ip = 16",
        "DRAM[T](M, K)",
        "DRAM[T](K, N)",
        "DRAM[T](M, N)",
        "Foreach(M by its, N by ts)",
        "MemReduce(c_col par ip)(K by ts)",
        "Reduce(Reg[T])(ts by 1 par ip)",
        "c_dram(ii::ii+its, jj::jj+ts par ip) store c_col",
        "PASS: ",
        "assert(cksum)",
    ] {
        assert!(
            spatial.contains(needle),
            "the Spatial GEMM app must contain {needle:?}:\n{spatial}"
        );
    }
    let (m2, n2, k2, class2, _) =
        lower_gemm_source(&logos_gemm_program(2048, 1024, 512), 32, 32, 16)
            .expect("the 2048-class shape lowers");
    assert_eq!(
        (m2, n2, k2, class2.as_str()),
        (2048, 1024, 512, "Logos_gemm_2048x1024x512")
    );
    let err = lower_gemm_source(&src, 48, 64, 16)
        .err()
        .expect("a tile that does not divide the shape is refused");
    assert!(err.contains("whole tiles"), "{err}");
    let err = lower_gemm_source(&src, 64, 64, 12)
        .err()
        .expect("lanes that do not divide the k-chunk are refused");
    assert!(err.contains("lane"), "{err}");
    let err = lower_gemm_source("## Main\n    Show 1.\n", 64, 64, 16)
        .err()
        .expect("not a matmul");
    assert!(err.contains("not a matmul"), "{err}");
}

// ───────────────────────────── G ─────────────────────────────

#[test]
fn the_not_modeled_list_names_the_data_dependent_corpus_honestly() {
    let doc = report();
    let nm = doc["not_modeled"]
        .as_array()
        .expect("not_modeled is a list");
    assert!(nm.len() >= 3);
    for e in nm {
        assert!(e["name"].as_str().map_or(false, |s| !s.is_empty()));
        assert!(
            e["reason"].as_str().map_or(false, |s| s.len() > 20),
            "a reason, not a shrug: {e}"
        );
    }
    let names: Vec<&str> = nm.iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert!(
        names.iter().any(|n| n.contains("while")),
        "the data-dependent while loops are named: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.contains("route128")),
        "the MoE routing is named: {names:?}"
    );
    let reference = &doc["published_reference"]["spatial_pldi18_table4_plasticine"];
    assert_eq!(
        reference["GEMM"]["pcu_pct"], 96.8,
        "Spatial Table 4's GEMM row is the credibility target"
    );
    assert!(
        doc["published_reference"]["note"]
            .as_str()
            .unwrap()
            .contains("not our"),
        "labeled as the paper's numbers"
    );
    let targets = doc["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0]["name"], "plasticine-isca17");
    assert_eq!(targets[0]["pcus"], 64);
    assert_eq!(targets[1]["pcus"], 200);
}
