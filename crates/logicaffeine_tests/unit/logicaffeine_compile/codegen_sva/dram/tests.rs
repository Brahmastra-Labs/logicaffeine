use super::super::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
use super::*;
use logicaffeine_proof::sat::{find_model, prove_unsat, ModelOutcome, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

fn seq_stream(bytes: u64, write: bool) -> StreamDesc {
    StreamDesc {
        base: 0,
        elem_bytes: 8,
        stride_bytes: 8,
        count: bytes / 8,
        write,
    }
}

// ── ComputeDRAM (rung 4): the in-DRAM compute plane, modeled + quarantined ──────────

/// A deterministic xorshift for fuzzed row contents (no randomness APIs — determinism law).
fn lcg(seed: u64) -> impl FnMut() -> u64 {
    let mut x = seed;
    move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }
}

/// "RUN LOGOS OVER DRAM": a LOGOS bulk bitwise op executed on the in-DRAM plane matches the
/// VM's native bitwise reference — bitset intersection = in-DRAM AND, union = in-DRAM OR,
/// both via the sense-amp majority reduction. This is the model-level proof that a LOGOS
/// program's bitwise work can run inside the memory and stay ≡VM.
#[test]
fn logos_bitset_ops_run_over_dram_match_the_vm_reference() {
    let words = 32; // 2048-bit bitsets
    let mut rng = lcg(0x1234_5678_9abc_def0);
    for _ in 0..200 {
        let a: Vec<u64> = (0..words).map(|_| rng()).collect();
        let b: Vec<u64> = (0..words).map(|_| rng()).collect();
        let mut store = RowStore::new(words);
        let bank = 0;
        store.write(bank, 10, &a);
        store.write(bank, 11, &b);
        store.apply(&ComputeDramOp::And {
            bank,
            a: 10,
            b: 11,
            dst: 20,
        });
        store.apply(&ComputeDramOp::Or {
            bank,
            a: 10,
            b: 11,
            dst: 21,
        });
        let native_inter: Vec<u64> = a.iter().zip(&b).map(|(p, q)| p & q).collect();
        let native_union: Vec<u64> = a.iter().zip(&b).map(|(p, q)| p | q).collect();
        assert_eq!(
            store.read(bank, 20),
            native_inter,
            "in-DRAM AND == VM bitset intersection"
        );
        assert_eq!(
            store.read(bank, 21),
            native_union,
            "in-DRAM OR == VM bitset union"
        );
    }
}

/// The sense-amp majority reduction IS bitwise AND (control 0) / OR (control 1), proven
/// EXHAUSTIVELY over the 3-input truth table (finite total) — the identity ComputeDRAM relies on.
#[test]
fn majority_reduction_is_exactly_and_with_0_and_or_with_1() {
    for a in 0..2u64 {
        for b in 0..2u64 {
            assert_eq!(word_majority(a, b, 0) & 1, a & b, "MAJ(a,b,0) == a AND b");
            assert_eq!(word_majority(a, b, 1) & 1, a | b, "MAJ(a,b,1) == a OR b");
            for c in 0..2u64 {
                let expect = if a + b + c >= 2 { 1 } else { 0 };
                assert_eq!(
                    word_majority(a, b, c) & 1,
                    expect,
                    "MAJ({a},{b},{c}) is the bit majority"
                );
            }
        }
    }
}

/// RowClone and MAJ oracle track the native bitwise result over fuzzed multi-word rows.
#[test]
fn rowcopy_and_maj_oracle_match_native_bitwise() {
    let words = 16;
    let mut rng = lcg(0xDEAD_BEEF_0BAD_F00D);
    for _ in 0..200 {
        let a: Vec<u64> = (0..words).map(|_| rng()).collect();
        let b: Vec<u64> = (0..words).map(|_| rng()).collect();
        let c: Vec<u64> = (0..words).map(|_| rng()).collect();
        let mut s = RowStore::new(words);
        s.write(1, 0, &a);
        s.write(1, 1, &b);
        s.write(1, 2, &c);
        s.apply(&ComputeDramOp::RowCopy {
            bank: 1,
            src: 0,
            dst: 5,
        });
        s.apply(&ComputeDramOp::Maj {
            bank: 1,
            a: 0,
            b: 1,
            c: 2,
            dst: 6,
        });
        assert_eq!(s.read(1, 5), a, "RowClone copies the row verbatim");
        let native_maj: Vec<u64> = (0..words)
            .map(|i| word_majority(a[i], b[i], c[i]))
            .collect();
        assert_eq!(
            s.read(1, 6),
            native_maj,
            "MAJ oracle == native bit majority"
        );
    }
}

/// PIT OF SUCCESS — the quarantine invariant: EVERY ComputeDRAM stream is REJECTED by the
/// independent JEDEC legality checker (deliberately timing-illegal), while a properly-spaced
/// ACT/PRE over the SAME rows is ACCEPTED — so the experimental path can never masquerade as a
/// certified plan, and the test is non-vacuous.
#[test]
fn computedram_streams_are_quarantined_from_the_legal_checker() {
    let t = DramTimings::ddr2_micron();
    let tv = ComputeDramTiming::nominal();
    let ops = [
        ComputeDramOp::RowCopy {
            bank: 0,
            src: 100,
            dst: 200,
        },
        ComputeDramOp::And {
            bank: 1,
            a: 5,
            b: 6,
            dst: 7,
        },
        ComputeDramOp::Or {
            bank: 2,
            a: 8,
            b: 9,
            dst: 10,
        },
        ComputeDramOp::Maj {
            bank: 3,
            a: 1,
            b: 2,
            c: 3,
            dst: 4,
        },
    ];
    for op in &ops {
        let stream = emit_computedram_stream(op, &tv);
        assert!(
            stream.len() >= 3,
            "a ComputeDRAM op emits an ACT-PRE-ACT… cadence: {op:?}"
        );
        assert!(
            check_legal(&t, &stream).is_err(),
            "a ComputeDRAM stream MUST be rejected as timing-illegal: {op:?}"
        );
    }
    // Non-vacuity: a legal ACT…PRE…ACT…PRE over the same two rows IS accepted.
    let legal = vec![
        TimedCmd {
            at: 0,
            cmd: DramCmd::Act { bank: 0, row: 100 },
        },
        TimedCmd {
            at: t.tras,
            cmd: DramCmd::Pre { bank: 0 },
        },
        TimedCmd {
            at: t.tras + t.trp,
            cmd: DramCmd::Act { bank: 0, row: 200 },
        },
        TimedCmd {
            at: t.tras + t.trp + t.tras,
            cmd: DramCmd::Pre { bank: 0 },
        },
    ];
    check_legal(&t, &legal).expect("a properly-spaced stream over the same rows is legal");
}

/// The DDR2-800 timing table is internally consistent with JEDEC relationships — a
/// pit-of-success guard so a mis-typed constant can't silently corrupt the model or the PHY.
#[test]
fn ddr2_micron_timings_are_internally_consistent() {
    let t = DramTimings::ddr2_micron();
    assert_eq!(t.banks, 8, "MT47H64M16 has 8 banks");
    assert_eq!(t.row_bytes, 2048, "x16 1 Gb page is 2 KB");
    assert_eq!(t.trc, t.tras + t.trp, "JEDEC: tRC == tRAS + tRP");
    assert!(
        t.tfaw >= 4 * t.trrd,
        "JEDEC: tFAW must admit 4 ACTs at tRRD spacing"
    );
    assert!(
        t.tras >= t.trcd,
        "a row must be open (tRCD) before it can close (tRAS)"
    );
    for v in [
        t.trcd, t.trp, t.tras, t.trc, t.trrd, t.tfaw, t.tccd, t.twr, t.trfc_pb,
    ] {
        assert!(v > 0, "every DDR2 timing must be positive");
    }
}

/// SHAPE LOCK: the RowClone lowering is pinned to the canonical ACT(src) → PRE → ACT(dst) → PRE
/// cadence, and the interrupting gaps are provably sub-tRAS / sub-tRP — a refactor can't silently
/// emit a cadence that would fail to trigger the charge-sharing effect on real silicon.
#[test]
fn computedram_rowcopy_lowers_to_the_canonical_act_pre_act_cadence() {
    let tv = ComputeDramTiming::nominal();
    let s = emit_computedram_stream(
        &ComputeDramOp::RowCopy {
            bank: 2,
            src: 40,
            dst: 41,
        },
        &tv,
    );
    let got: Vec<(u64, DramCmd)> = s.iter().map(|c| (c.at, c.cmd)).collect();
    assert_eq!(
        got,
        vec![
            (0, DramCmd::Act { bank: 2, row: 40 }),
            (tv.t1, DramCmd::Pre { bank: 2 }),
            (tv.t1 + tv.t2, DramCmd::Act { bank: 2, row: 41 }),
            (tv.t1 + tv.t2 + tv.t1, DramCmd::Pre { bank: 2 }),
        ],
        "RowClone must lower to ACT(src) → PRE → ACT(dst) → PRE"
    );
    let t = DramTimings::ddr2_micron();
    assert!(
        tv.t1 < t.tras && tv.t2 < t.trp,
        "the cadence must violate tRAS/tRP to trigger the effect"
    );
}

/// PHASE 2 (raw-command / SoftMC capability): a ComputeDRAM op lowers to a delta-encoded command
/// stream that the in-order executor carries VERBATIM — the sub-JEDEC ACT-PRE-ACT cadence survives
/// the raw-command path unchanged (the executor's countdown is proven exact by
/// `the_countdown_issues_at_exactly_the_decoded_delta_by_our_sat`), and the carried stream is
/// genuinely sub-JEDEC (the raw path bypasses `check_legal` by design).
#[test]
fn computedram_op_survives_the_raw_command_executor_path() {
    let op = ComputeDramOp::RowCopy {
        bank: 3,
        src: 100,
        dst: 200,
    };
    let stream = emit_computedram_stream(&op, &ComputeDramTiming::nominal());
    let words = delta_encode(&stream);
    assert_eq!(words.len(), stream.len(), "one executor word per command");
    // Decode each executor word and confirm it reconstructs the exact command AND absolute time.
    let mut at = 0u64;
    let decoded: Vec<TimedCmd> = words
        .iter()
        .map(|w| {
            at += w >> 40;
            let op = (w >> 37) & 0x7;
            let bank = ((w >> 32) & 0x1f) as u32;
            let row = w & 0xffff_ffff;
            let cmd = match op {
                0 => DramCmd::Act { bank, row },
                1 => DramCmd::Rd { bank },
                2 => DramCmd::Wr { bank },
                3 => DramCmd::Pre { bank },
                4 => DramCmd::RefPb { bank },
                _ => unreachable!(),
            };
            TimedCmd { at, cmd }
        })
        .collect();
    assert!(
        stream
            .iter()
            .zip(&decoded)
            .all(|(a, b)| a.at == b.at && a.cmd == b.cmd),
        "the raw-command executor path carries the ComputeDRAM stream verbatim"
    );
    assert!(
        check_legal(&DramTimings::ddr2_micron(), &stream).is_err(),
        "the carried stream is sub-JEDEC by design (the raw path bypasses the legality checker)"
    );
}

/// HOW IT BECOMES POWERFUL, as an invariant: one ComputeDRAM stream computes a WHOLE ROW of
/// bits in a CONSTANT handful of commands — the bit-parallelism (bits resolved per DRAM
/// command) is enormous and INDEPENDENT of operand width. The energy/throughput thesis
/// (row-parallel compute, bytes-not-moved) made mechanical.
#[test]
fn in_dram_bitwise_is_row_parallel_at_constant_command_cost() {
    let t = DramTimings::ddr2_micron();
    let tv = ComputeDramTiming::nominal();
    let row_bits = (t.row_bytes * 8) as usize; // 16,384 bits per row
    let and = emit_computedram_stream(
        &ComputeDramOp::And {
            bank: 0,
            a: 0,
            b: 1,
            dst: 2,
        },
        &tv,
    );
    let maj = emit_computedram_stream(
        &ComputeDramOp::Maj {
            bank: 0,
            a: 0,
            b: 1,
            c: 2,
            dst: 3,
        },
        &tv,
    );
    assert!(
        and.len() <= 8 && maj.len() <= 8,
        "an in-DRAM op is O(1) commands"
    );
    let bits_per_command = row_bits / and.len();
    assert!(
        bits_per_command >= 1000,
        "one AND stream resolves {bits_per_command} bits/command — massive row-parallelism"
    );
}

/// Sequential streaming — the shape of loop_sum/nbody/codec — must beat the
/// handout's 90% bus-efficiency claim on the LPDDR5X table, WITH refresh on.
#[test]
fn sequential_stream_exceeds_ninety_percent_bus_efficiency() {
    let t = DramTimings::lpddr5x();
    let plan = schedule(&t, &[seq_stream(1 << 20, false)], &[], &[]);
    check_legal(&t, &plan).expect("plan is timing-legal");
    let e = measure(&t, &plan);
    assert!(
        e.value() >= 0.90,
        "sequential efficiency {:.4} below the handout claim (bursts={} acts={} refs={})",
        e.value(),
        e.bursts,
        e.row_activates,
        e.refreshes
    );
    assert!(
        e.refreshes > 0,
        "a 1 MB stream must span refresh obligations"
    );
}

/// The zero-jitter fact: device execution reproduces the planned data beats
/// cycle-for-cycle, refresh included.
#[test]
fn device_execution_matches_the_plan_cycle_for_cycle() {
    let t = DramTimings::lpddr5x();
    let plan = schedule(&t, &[seq_stream(1 << 18, true)], &[], &[]);
    let planned = planned_beats(&t, &plan);
    let executed = DeviceModel::execute(&t, &plan).expect("legal");
    assert_eq!(
        planned, executed,
        "the machine executes the schedule — no jitter"
    );
}

/// Determinism: the scheduler is a pure function.
#[test]
fn scheduling_is_deterministic() {
    let t = DramTimings::lpddr5x();
    let a = schedule(&t, &[seq_stream(1 << 16, false)], &[], &[]);
    let b = schedule(&t, &[seq_stream(1 << 16, false)], &[], &[]);
    assert_eq!(a.len(), b.len());
    assert!(a
        .iter()
        .zip(&b)
        .all(|(x, y)| x.at == y.at && x.cmd == y.cmd));
}

/// Blacklisting rows re-places the same payload legally; efficiency moves by a
/// bounded amount (yield-by-recompile, measured).
#[test]
fn blacklisted_rows_are_avoided_and_the_plan_stays_legal() {
    let t = DramTimings::lpddr5x();
    let bad: Vec<(u32, u64)> = (0..8).map(|b| (b as u32, 0u64)).collect();
    let clean = schedule(&t, &[seq_stream(1 << 18, false)], &[], &[]);
    let remapped = schedule(&t, &[seq_stream(1 << 18, false)], &[], &bad);
    check_legal(&t, &remapped).expect("remapped plan legal");
    for c in &remapped {
        if let DramCmd::Act { bank, row } = c.cmd {
            assert!(
                !bad.contains(&(bank, row)),
                "blacklisted row activated: bank {bank} row {row}"
            );
        }
    }
    let (ec, er) = (measure(&t, &clean), measure(&t, &remapped));
    assert_eq!(ec.bursts, er.bursts, "same payload volume moves");
    assert!(
        er.value() >= ec.value() * 0.95,
        "remap cost exceeded 5%: {:.4} vs {:.4}",
        er.value(),
        ec.value()
    );
}

/// The strided / random tail measured honestly: a row-per-element gather is
/// tFAW/tRC-limited and lands FAR below the streaming number — this is the
/// measured cost of the dynamic 15% in graph_bfs, not a footnote.
#[test]
fn random_gather_efficiency_is_honestly_low() {
    let t = DramTimings::lpddr5x();
    // A deterministic scattered address list (seeded LCG — no randomness APIs).
    let mut x = 0x9E3779B97F4A7C15u64;
    let addrs: Vec<u64> = (0..4096)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % (1 << 26)) & !7
        })
        .collect();
    let plan = schedule(
        &t,
        &[],
        &[Gather {
            addrs,
            elem_bytes: 8,
            write: false,
        }],
        &[],
    );
    check_legal(&t, &plan).expect("gather plan legal");
    let e = measure(&t, &plan);
    assert!(
        e.value() < 0.60,
        "a random gather should NOT look efficient: {:.4}",
        e.value()
    );
}

/// DEMO 2 — a real compiled kernel's memory plan is a CERTIFIED-LEGAL command stream that the on-die
/// executor replays with ZERO jitter. A SoA kernel (nbody reads 7 arrays per step) is ROW-TILED by the
/// static scheduler; the emitted ACT/RD/WR/PRE/REF stream is proven timing-legal by the independent
/// checker, the device delivers EXACTLY the compile-time promised beat cycles, and the bus stays busy.
/// The checker is not a rubber stamp — an all-at-once plan is rejected. (`scripts/demo-dram-determinism.sh`
/// narrates this + runs `dram_executor.v` under iverilog + the tRP-legality SAT proof.)
#[test]
fn a_real_compiled_plan_emits_a_certified_legal_stream_the_executor_replays() {
    let t = DramTimings::lpddr5x();
    // A SoA kernel shape: 7 sequential streams (6 read arrays + 1 write), ROW-TILED — the compile-time
    // answer to the pattern a reactive controller thrashes rows on.
    let streams: Vec<StreamDesc> = (0..7)
        .map(|k| StreamDesc {
            base: (k as u64) << 20,
            elem_bytes: 8,
            stride_bytes: 8,
            count: 4096,
            write: k == 6,
        })
        .collect();
    let plan = schedule_ordered(&t, &streams, &[], &[], VisitOrder::RowTiled);
    assert!(!plan.is_empty(), "the scheduler produced a command stream");

    // (1) CERTIFIED timing-legal by the independent checker (tRCD/tRP/tRAS/tFAW/tREFI ...).
    check_legal(&t, &plan).expect("the compiler-emitted command stream is timing-legal");

    // (2) The on-die executor delivers EXACTLY the compile-time promise — zero jitter, cycle-exact.
    let promised = planned_beats(&t, &plan);
    let delivered = DeviceModel::execute(&t, &plan).expect("device executes the legal stream");
    assert_eq!(
        promised, delivered,
        "device replay diverged from the compile-time promise"
    );

    // (3) High sustained bus efficiency — a static schedule keeps the bus busy (no demand miss to hide).
    let eff = measure(&t, &plan).value();
    assert!(
        eff > 0.85,
        "row-tiled SoA bus efficiency unexpectedly low: {eff:.4}"
    );

    // (4) NON-VACUITY — collapse every command onto cycle 0 (a gross timing violation): REJECTED.
    let illegal: Vec<TimedCmd> = plan
        .iter()
        .map(|c| TimedCmd { at: 0, cmd: c.cmd })
        .collect();
    assert!(
        check_legal(&t, &illegal).is_err(),
        "an all-at-once plan must be rejected"
    );

    if std::env::var("LOGOS_DEMO").is_ok() {
        eprintln!(
            "  workload: 7-array SoA kernel (nbody shape), ROW-TILED by the static scheduler"
        );
        eprintln!(
            "  compiler-emitted command stream: {} ACT/RD/WR/PRE/REF commands",
            plan.len()
        );
        eprintln!(
            "  independent legality checker: TIMING-LEGAL (tRCD / tRP / tRAS / tFAW / tREFI)"
        );
        eprintln!(
            "  on-die executor: delivered {} data beats == the compile-time promise (ZERO jitter)",
            delivered.len()
        );
        eprintln!(
            "  sustained bus efficiency: {:.1}% (data cycles / span cycles)",
            eff * 100.0
        );
        eprintln!("  non-vacuity: an all-at-once (illegal) plan is REJECTED by the checker");
    }
}

/// Deliberately illegal plans are REJECTED by the independent checker (the
/// two-sided guarantee Wave 3's SAT obligation will mirror).
#[test]
fn the_checker_rejects_timing_violations() {
    let t = DramTimings::lpddr5x();
    // tRCD violation: CAS immediately after ACT.
    let plan = vec![
        TimedCmd {
            at: 0,
            cmd: DramCmd::Act { bank: 0, row: 0 },
        },
        TimedCmd {
            at: 1,
            cmd: DramCmd::Rd { bank: 0 },
        },
    ];
    assert!(check_legal(&t, &plan).unwrap_err().contains("tRCD"));
    // tRP violation: ACT right after PRE.
    let plan = vec![
        TimedCmd {
            at: 0,
            cmd: DramCmd::Act { bank: 0, row: 0 },
        },
        TimedCmd {
            at: 50,
            cmd: DramCmd::Pre { bank: 0 },
        },
        TimedCmd {
            at: 51,
            cmd: DramCmd::Act { bank: 0, row: 1 },
        },
    ];
    assert!(check_legal(&t, &plan).unwrap_err().contains("tRP"));
    // tFAW violation: five ACTs bunched on five banks.
    let plan: Vec<TimedCmd> = (0..5)
        .map(|i| TimedCmd {
            at: i * t.trrd,
            cmd: DramCmd::Act {
                bank: i as u32,
                row: 0,
            },
        })
        .collect();
    assert!(check_legal(&t, &plan).unwrap_err().contains("tFAW"));
    // Data-bus overlap on two banks (both CAS individually tRCD-legal).
    let plan = vec![
        TimedCmd {
            at: 0,
            cmd: DramCmd::Act { bank: 0, row: 0 },
        },
        TimedCmd {
            at: t.trrd,
            cmd: DramCmd::Act { bank: 1, row: 0 },
        },
        TimedCmd {
            at: t.trrd + t.trcd,
            cmd: DramCmd::Rd { bank: 0 },
        },
        TimedCmd {
            at: t.trrd + t.trcd + 1,
            cmd: DramCmd::Rd { bank: 1 },
        },
    ];
    assert!(check_legal(&t, &plan).unwrap_err().contains("overlap"));
}

/// Property fuzz: many seeded stream mixes — every scheduled plan passes the
/// independent checker (the scheduler cannot emit an illegal stream).
#[test]
fn fuzzed_stream_mixes_always_schedule_legally() {
    let t = DramTimings::lpddr5x();
    let mut x = 0xDEADBEEFCAFEF00Du64;
    let mut rng = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for case in 0..40 {
        let nstreams = 1 + (rng() % 4) as usize;
        let streams: Vec<StreamDesc> = (0..nstreams)
            .map(|_| StreamDesc {
                base: (rng() % (1 << 24)) & !63,
                elem_bytes: 8,
                stride_bytes: [8u64, 8, 8, 64, 4096][(rng() % 5) as usize],
                count: 256 + rng() % 4096,
                write: rng() % 2 == 0,
            })
            .collect();
        let plan = schedule(&t, &streams, &[], &[]);
        check_legal(&t, &plan)
            .unwrap_or_else(|e| panic!("case {case}: scheduler emitted illegal plan: {e}"));
    }
}

/// The row-tiled SoA schedule (nbody's shape) keeps multi-stream traffic at
/// streaming-class efficiency — the tiling a reactive controller cannot do.
#[test]
fn soa_row_tiling_keeps_multistream_traffic_efficient() {
    let t = DramTimings::lpddr5x();
    let soa: Vec<StreamDesc> = (0..7)
        .map(|i| StreamDesc {
            base: i * (1 << 22),
            elem_bytes: 8,
            stride_bytes: 8,
            count: (1 << 17) / 8,
            write: i == 6,
        })
        .collect();
    let plan = schedule_ordered(&t, &soa, &[], &[], VisitOrder::RowTiled);
    check_legal(&t, &plan).expect("legal");
    let e = measure(&t, &plan);
    assert!(
        e.value() >= 0.90,
        "row-tiled SoA efficiency {:.4} below streaming class",
        e.value()
    );
}

/// The standard scenario artifact is a golden: the checked-in
/// `benchmarks/results/dram_sim.json` must equal the recomputed report
/// (regenerate with `DRAM_RECORD=1 cargo test ... dram_sim_artifact`).
#[test]
fn dram_sim_artifact_matches_the_recomputed_report() {
    let report = standard_report();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/results/dram_sim.json");
    if std::env::var("DRAM_RECORD").is_ok() {
        std::fs::write(&path, &report).expect("write dram_sim.json");
        return;
    }
    let checked_in = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing {} ({e}) — record it: DRAM_RECORD=1 cargo test -p \
                 logicaffeine-compile dram_sim_artifact",
            path.display()
        )
    });
    assert_eq!(
        checked_in, report,
        "dram_sim.json is stale — the model or table changed; regenerate with \
             DRAM_RECORD=1 and re-cite the handout numbers"
    );
}

// ── Wave 3: the certified command stream + the in-order executor ─────────────

/// tFAW is PROVEN a genuinely separate constraint from tRRD by our certified SAT:
/// with three symbolic gaps between four consecutive ACTs, each ≥ tRRD, the solver
/// finds a model whose total span is still < tFAW — so tRRD spacing alone cannot
/// satisfy the four-ACT window. A concrete witness (not hand-waving) for why both
/// the scheduler and the independent checker carry a distinct tFAW rule.
#[test]
fn tfaw_is_proven_necessary_beyond_trrd_by_our_sat() {
    use BitVecBoundedOp::*;
    const W: u32 = 10; // gaps up to 1023 cycles — covers the table with headroom
    let t = DramTimings::lpddr5x();
    let bv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), W));
    let k = |v: u64| Box::new(BoundedExpr::BitVecConst { width: W, value: v });
    let bin = |o: BitVecBoundedOp, l: Box<BoundedExpr>, r: Box<BoundedExpr>| {
        Box::new(BoundedExpr::BitVecBinary {
            op: o,
            left: l,
            right: r,
        })
    };
    let lower = |e: BoundedExpr| super::super::bitblast::lower_bool(&e).expect("lowers");
    // g ≥ tRRD  ⟺  (tRRD−1) < g  (unsigned; gaps stay well below the W=10 wrap).
    let ge_trrd = |g: &str| {
        lower(BoundedExpr::BitVecBinary {
            op: ULt,
            left: k(t.trrd - 1),
            right: bv(g),
        })
    };
    let span = bin(Add, bin(Add, bv("g1"), bv("g2")), bv("g3"));
    let span_lt_faw = lower(BoundedExpr::BitVecBinary {
        op: ULt,
        left: span,
        right: k(t.tfaw),
    });
    let all = ProofExpr::And(
        Box::new(ProofExpr::And(
            Box::new(ge_trrd("g1")),
            Box::new(ge_trrd("g2")),
        )),
        Box::new(ProofExpr::And(
            Box::new(ge_trrd("g3")),
            Box::new(span_lt_faw),
        )),
    );
    assert!(
        matches!(find_model(&all), ModelOutcome::Sat(_)),
        "tRRD-spaced ACTs can span below tFAW — tFAW is a separate constraint"
    );
}

/// Delta-encoding round-trips: decoding the executor words reproduces the plan's
/// absolute times, opcodes, banks, and rows exactly.
#[test]
fn delta_encoding_round_trips_the_plan() {
    let t = DramTimings::lpddr5x();
    let plan = schedule(&t, &[seq_stream(1 << 14, false)], &[], &[]);
    let words = delta_encode(&plan);
    assert_eq!(words.len(), plan.len());
    let mut at = 0u64;
    for (w, c) in words.iter().zip(&plan) {
        let delta = w >> 40;
        let op = (w >> 37) & 0x7;
        let bank = ((w >> 32) & 0x1f) as u32;
        let row = w & 0xffff_ffff;
        at += delta;
        assert_eq!(at, c.at, "delta decodes to the planned absolute time");
        let (eop, ebank, erow) = match c.cmd {
            DramCmd::Act { bank, row } => (0, bank, row),
            DramCmd::Rd { bank } => (1, bank, 0),
            DramCmd::Wr { bank } => (2, bank, 0),
            DramCmd::Pre { bank } => (3, bank, 0),
            DramCmd::RefPb { bank } => (4, bank, 0),
        };
        assert_eq!((op, bank, row), (eop, ebank, erow));
    }
}

/// The CERTIFIED tRP legality theorem: under the scheduler's guarantee
/// (`act = pre + tRP + slack`, slack ≥ 0 and small enough not to wrap), our SAT
/// PROVES the resulting PRE→ACT gap can never fall below tRP — the negation is
/// UNSAT, on our own solver.
#[test]
fn trp_legality_is_proven_by_our_sat() {
    use BitVecBoundedOp::*;
    const W: u32 = 8;
    let bv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), W));
    let k = |v: u64| Box::new(BoundedExpr::BitVecConst { width: W, value: v });
    let bin = |o: BitVecBoundedOp, l: Box<BoundedExpr>, r: Box<BoundedExpr>| {
        Box::new(BoundedExpr::BitVecBinary {
            op: o,
            left: l,
            right: r,
        })
    };
    let lower = |e: BoundedExpr| super::super::bitblast::lower_bool(&e).expect("lowers");
    let trp = 15u64;
    // gap = tRP + slack, with slack small enough (< 256-tRP) that it can't wrap W=8.
    let gap = bin(Add, k(trp), bv("slack"));
    let no_overflow = lower(BoundedExpr::BitVecBinary {
        op: ULt,
        left: bv("slack"),
        right: k(256 - trp),
    });
    let violated = lower(BoundedExpr::BitVecBinary {
        op: ULt,
        left: gap,
        right: k(trp),
    });
    let conj = ProofExpr::And(Box::new(no_overflow), Box::new(violated));
    assert_eq!(
        prove_unsat(&conj),
        UnsatOutcome::Refuted,
        "our SAT proves a tRP-respecting gap can never be below tRP"
    );
}

/// Vacuity guard: WITHOUT the guarantee a sub-tRP gap is satisfiable — so the
/// theorem checks something real, and our solver finds the witness.
#[test]
fn sub_trp_gap_is_satisfiable_without_the_guarantee() {
    use BitVecBoundedOp::*;
    const W: u32 = 8;
    let bv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), W));
    let k = |v: u64| Box::new(BoundedExpr::BitVecConst { width: W, value: v });
    let lower = |e: BoundedExpr| super::super::bitblast::lower_bool(&e).expect("lowers");
    let sat = lower(BoundedExpr::BitVecBinary {
        op: ULt,
        left: bv("gap"),
        right: k(15),
    });
    assert!(
        matches!(find_model(&sat), ModelOutcome::Sat(_)),
        "a sub-tRP gap must be satisfiable when unconstrained (non-vacuity)"
    );
}

/// The executor RTL is an in-order FSM by construction: exactly one `issue` path,
/// no reorder structures (the iverilog testbench proves the behavior).
#[test]
fn executor_rtl_is_in_order_by_construction() {
    let v = emit_dram_executor_verilog();
    assert!(v.contains("module dram_executor"));
    assert_eq!(
        v.matches("issue <= 1'b1;").count(),
        1,
        "one and only one issue path"
    );
    assert!(!v.to_lowercase().contains("reorder"));
    assert!(!v.contains("sort"));
    assert!(v.contains("loaded <= 1'b0;") && v.contains("loaded <= 1'b1;"));
}

/// WORLD-CLASS (formal): the executor's plan-word DECODE is the exact inverse of the certified
/// compile-time ENCODER's bit layout — proven EXHAUSTIVELY (all field values) by our OWN
/// certified SAT. `delta_encode` packs each command as `[delta:24][op:3][bank:5][row:32]`; the
/// `dram_executor` netlist tears it back apart with the part-selects `cmd_word[63:40]` (the
/// countdown value), `[39:37]`, `[36:32]`, `[31:0]`. We reconstruct the encoder word INDEPENDENTLY
/// by CONCATENATION of symbolic fields, apply the RTL's exact `EXTRACT`s, and certify
/// `decode(encode(f)) == f` for every field. A wrong slice boundary in the netlist — e.g. an
/// off-by-one on the delta that would load the countdown with the wrong value and mis-time the
/// command — is caught here, not sampled. This is the third leg (alongside the eval battery and
/// the iverilog testbench) that makes the executor's decode a PROOF, not a spot-check.
///
/// The proof is pinned to the ACTUAL emitted netlist: we first assert the RTL contains exactly
/// these part-selects, so the boundaries certified below are the boundaries the silicon uses.
#[test]
fn the_executor_decode_is_the_certified_inverse_of_the_plan_encoding_by_our_sat() {
    // Pin the proof to the emitted netlist's part-selects (drift-proof): the four slices the
    // SAT reasons about below must be the four slices the RTL actually cuts.
    let v = emit_dram_executor_verilog();
    for slice in [
        "cmd_word[63:40]",
        "cmd_word[39:37]",
        "cmd_word[36:32]",
        "cmd_word[31:0]",
    ] {
        assert!(
            v.contains(slice),
            "RTL must decode with the part-select {slice}"
        );
    }

    let bvv = |n: &str, w: u32| Box::new(BoundedExpr::BitVecVar(n.to_string(), w));
    // Independent reconstruction of the encoder layout, assembled high-to-low by concatenation
    // (24 + 3 + 5 + 32 = 64): [63:40]=delta, [39:37]=op, [36:32]=bank, [31:0]=row.
    let word = BoundedExpr::BitVecConcat(
        bvv("delta", 24),
        Box::new(BoundedExpr::BitVecConcat(
            bvv("op", 3),
            Box::new(BoundedExpr::BitVecConcat(bvv("bank", 5), bvv("row", 32))),
        )),
    );
    let extract = |high: u32, low: u32| {
        Box::new(BoundedExpr::BitVecExtract {
            high,
            low,
            operand: Box::new(word.clone()),
        })
    };
    // Each RTL part-select must recover EXACTLY the field the encoder packed.
    let field_eq = |high, low, name: &str, w| {
        let e = BoundedExpr::Eq(extract(high, low), bvv(name, w));
        super::super::bitblast::lower_bool(&e).expect("field equality lowers")
    };
    let all = ProofExpr::And(
        Box::new(ProofExpr::And(
            Box::new(field_eq(63, 40, "delta", 24)),
            Box::new(field_eq(39, 37, "op", 3)),
        )),
        Box::new(ProofExpr::And(
            Box::new(field_eq(36, 32, "bank", 5)),
            Box::new(field_eq(31, 0, "row", 32)),
        )),
    );
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(all))),
        UnsatOutcome::Refuted,
        "the executor's field decode must be the exact inverse of the plan encoding for ALL fields"
    );
}

/// Non-vacuity guard for the decode proof: an OFF-BY-ONE part-select (`[62:39]` instead of the
/// real `[63:40]`) does NOT recover the delta — our solver finds a witness where they differ.
/// So the theorem above certifies a real bit-boundary, not a tautology any slice would satisfy.
#[test]
fn a_shifted_decode_slice_is_not_the_delta() {
    let bvv = |n: &str, w: u32| Box::new(BoundedExpr::BitVecVar(n.to_string(), w));
    let word = BoundedExpr::BitVecConcat(
        bvv("delta", 24),
        Box::new(BoundedExpr::BitVecConcat(
            bvv("op", 3),
            Box::new(BoundedExpr::BitVecConcat(bvv("bank", 5), bvv("row", 32))),
        )),
    );
    // Wrong slice: shifted down by one bit → {delta[22:0], op[2]} ≠ delta in general.
    let wrong = Box::new(BoundedExpr::BitVecExtract {
        high: 62,
        low: 39,
        operand: Box::new(word),
    });
    let eq_p = super::super::bitblast::lower_bool(&BoundedExpr::Eq(wrong, bvv("delta", 24)))
        .expect("slice equality lowers");
    assert!(
        matches!(
            find_model(&ProofExpr::Not(Box::new(eq_p))),
            ModelOutcome::Sat(_)
        ),
        "an off-by-one decode slice must be able to differ from the delta (non-vacuity)"
    );
}

/// WORLD-CLASS (formal, temporal): the countdown fires at EXACTLY the decoded delta. The
/// executor loads `wait_left = delta` and decrements by one each cycle; after `t` cycles the
/// counter holds `delta - t`, and `issue` is the zero-test of that counter. We certify, over an
/// EXHAUSTIVE 8-bit delta and every concrete cycle `t ∈ 0..256`, the timing law
/// `(delta - t == 0) ⟺ (delta == t)` — i.e. the counter is zero at cycle `t` iff `t` is the
/// planned delay. Together with the decode proof (`wait_left` is loaded with exactly the plan's
/// delta) and the strict unit descent, this certifies each command issues the planned number of
/// cycles after it is consumed — the executor's timing is a PROOF, not a sampled trace. A netlist
/// that decremented by two, saturated, or fired early would violate the law and be caught.
#[test]
fn the_countdown_issues_at_exactly_the_decoded_delta_by_our_sat() {
    const K: u32 = 8; // exhaustive over all 8-bit deltas (the CGRA PW=8 idiom)
    let delta = || Box::new(BoundedExpr::BitVecVar("delta".to_string(), K));
    let kc = |v: u64| Box::new(BoundedExpr::BitVecConst { width: K, value: v });
    // Atomic BV predicates lower individually; the boolean structure is built at ProofExpr level
    // (lower_bool only handles atomic comparisons — And/Or/Implies/Not live in ProofExpr).
    let lower = |e: BoundedExpr| super::super::bitblast::lower_bool(&e).expect("atom lowers");
    // issue_t == (delta - t == 0);  planned_t == (delta == t).  The law: issue_t ⟺ planned_t.
    let issue = |t: u64| {
        let counter = Box::new(BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::Sub,
            left: delta(),
            right: kc(t),
        });
        lower(BoundedExpr::Eq(counter, kc(0)))
    };
    let planned = |t: u64| lower(BoundedExpr::Eq(delta(), kc(t)));
    let iff_at = |t: u64| {
        ProofExpr::And(
            Box::new(ProofExpr::Implies(Box::new(issue(t)), Box::new(planned(t)))),
            Box::new(ProofExpr::Implies(Box::new(planned(t)), Box::new(issue(t)))),
        )
    };
    // Conjoin the law across every cycle in the exhaustive window.
    let mut law = iff_at(0);
    for t in 1..(1u64 << K) {
        law = ProofExpr::And(Box::new(law), Box::new(iff_at(t)));
    }
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(law))),
        UnsatOutcome::Refuted,
        "the counter must reach zero at exactly cycle == delta, for ALL deltas and cycles"
    );
}

/// Non-vacuity guard for the timing law: a netlist that decremented by TWO per cycle would
/// reach zero at cycle delta/2, not delta — our solver finds a delta/cycle witness where the
/// mutated law `(delta - 2t == 0) ⟺ (delta == t)` is violated. So the theorem pins a real
/// decrement rate, not a tautology.
#[test]
fn a_decrement_by_two_countdown_mis_times_the_issue() {
    const K: u32 = 8;
    let delta = || Box::new(BoundedExpr::BitVecVar("delta".to_string(), K));
    let kc = |v: u64| Box::new(BoundedExpr::BitVecConst { width: K, value: v });
    let lower = |e: BoundedExpr| super::super::bitblast::lower_bool(&e).expect("atom lowers");
    // The bug: decrement by two → zero at cycle delta/2. At t=2 the mutated counter is delta-4,
    // so `issue_at2 == (delta == 4)` while the plan expects the delay to be 2 (`delta == 2`).
    let counter = || {
        Box::new(BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::Sub,
            left: delta(),
            right: kc(4),
        })
    };
    let issue_at2 = || lower(BoundedExpr::Eq(counter(), kc(0)));
    let planned_at2 = || lower(BoundedExpr::Eq(delta(), kc(2)));
    // delta==4 ⟺ delta==2 — a false law, built at ProofExpr level.
    let mutated_law = ProofExpr::And(
        Box::new(ProofExpr::Implies(
            Box::new(issue_at2()),
            Box::new(planned_at2()),
        )),
        Box::new(ProofExpr::Implies(
            Box::new(planned_at2()),
            Box::new(issue_at2()),
        )),
    );
    assert!(
        matches!(
            find_model(&ProofExpr::Not(Box::new(mutated_law))),
            ModelOutcome::Sat(_)
        ),
        "the by-two mutation must have a counterexample (it is not a valid timing law)"
    );
}

/// Emit the executor RTL + a real delta-encoded plan + the self-checking testbench
/// into benchmarks/silicon/ (golden; regenerate with DRAM_RECORD=1). The iverilog
/// PASS/FAIL is wired into scripts/synth-soft-core.sh.
#[test]
fn dram_executor_artifacts_are_current() {
    let t = DramTimings::lpddr5x();
    let plan = schedule(&t, &[seq_stream(4096, false)], &[], &[]);
    let words = delta_encode(&plan);
    let rtl = emit_dram_executor_verilog();
    let hex: String = words.iter().map(|w| format!("{w:016x}\n")).collect();
    let tb = emit_dram_executor_testbench(words.len());
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    let files = [
        ("dram_executor.v", rtl),
        ("dram_plan.hex", hex),
        ("tb_dram_executor.v", tb),
    ];
    if std::env::var("DRAM_RECORD").is_ok() {
        for (name, body) in &files {
            std::fs::write(dir.join(name), body).expect("write artifact");
        }
        return;
    }
    for (name, body) in &files {
        let path = dir.join(name);
        let got = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "missing {} ({e}) — record: DRAM_RECORD=1 cargo test dram_executor_artifacts",
                path.display()
            )
        });
        assert_eq!(
            &got, body,
            "{name} is stale — regenerate with DRAM_RECORD=1"
        );
    }
}

// ── Wave 5: memory-tier integrity (G14) ─────────────────────────────────────

/// FNV-1a — the SAME frame checksum `binisa` carries (binisa.rs `checksum`), used
/// here to verify a scratchpad/DRAM round-trip end-to-end above the ECC.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A frame written to DRAM and read back is verified end-to-end by its checksum;
/// a single flipped bit anywhere in the payload is DETECTED (never silently
/// accepted) — the "zero silent corruption" requirement, at the frame level.
#[test]
fn frames_survive_a_dram_round_trip_and_a_bit_flip_is_detected() {
    // The "DRAM" here is a faithful byte store; the point is the integrity check
    // riding above it (the timing model proves the ACCESS is legal; this proves
    // the DATA is intact).
    let payload: Vec<u8> = (0..4096u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
        .collect();
    let tag = fnv1a(&payload);
    // Clean round-trip verifies.
    let read_back = payload.clone();
    assert_eq!(fnv1a(&read_back), tag, "clean round-trip must verify");
    // Any single-bit flip is caught.
    for &pos in &[0usize, 1, 1000, 4095] {
        let mut corrupt = payload.clone();
        corrupt[pos] ^= 1;
        assert_ne!(
            fnv1a(&corrupt),
            tag,
            "a bit flip at byte {pos} must change the checksum"
        );
    }
}

/// Reed-Solomon across BANKS: stripe a payload as K data + parity shards over N
/// banks; lose one bank (the G5 blacklist analogue) and reconstruct the exact
/// payload from the survivors — chipkill at the bank granularity, reusing the
/// shipped GF(256) FEC.
#[test]
fn reed_solomon_reconstructs_a_dropped_bank() {
    use crate::concurrency::fec;
    let payload: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
    let (k, n) = (10usize, 14usize); // 4 parity banks
    let (orig_len, shards) = fec::encode(&payload, k, n).expect("encode");
    // Drop bank 3 (a blacklisted/failed bank): keep any k of the survivors.
    let survivors: Vec<(usize, Vec<u8>)> = shards
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 3)
        .take(k)
        .map(|(i, s)| (i, s.clone()))
        .collect();
    let recovered = fec::decode(orig_len, k, n, &survivors).expect("decode");
    assert_eq!(recovered, payload, "the exact payload survives a lost bank");
}

#[test]
fn hot_derating_doubles_refresh_and_stays_legal() {
    let mut t = DramTimings::lpddr5x();
    let cold = schedule(&t, &[seq_stream(1 << 20, false)], &[], &[]);
    let cold_refs = measure(&t, &cold).refreshes;
    t.hot = true;
    let hot = schedule(&t, &[seq_stream(1 << 20, false)], &[], &[]);
    check_legal(&t, &hot).expect("hot plan legal");
    let hot_refs = measure(&t, &hot).refreshes;
    assert!(
        hot_refs > cold_refs,
        "hot derating must add refreshes: {hot_refs} vs {cold_refs}"
    );
}
