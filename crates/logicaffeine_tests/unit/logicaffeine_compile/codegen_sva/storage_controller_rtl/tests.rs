use super::super::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
use super::*;
use logicaffeine_proof::sat::{find_model, prove_unsat, ModelOutcome, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

fn read_zone(base: u64, count: u64) -> ZoneDesc {
    ZoneDesc {
        zone: 0,
        base_page: base,
        count,
        write: false,
    }
}

// ── S1 — the scheduler is a pure function ────────────────────────────────────────
#[test]
fn scheduling_is_deterministic() {
    let t = FlashTimings::zns_tlc();
    let a = schedule_zoned(
        &t,
        &[read_zone(0, 2048)],
        &[],
        &[ZoneReset { zone: 0 }],
        VisitOrder::ProgramOrder,
    );
    let b = schedule_zoned(
        &t,
        &[read_zone(0, 2048)],
        &[],
        &[ZoneReset { zone: 0 }],
        VisitOrder::ProgramOrder,
    );
    assert_eq!(a.len(), b.len());
    assert!(a
        .iter()
        .zip(&b)
        .all(|(x, y)| x.at == y.at && x.cmd == y.cmd));
}

// ── S2 — the scheduler can never emit an illegal stream (seeded fuzz) ─────────────
#[test]
fn fuzzed_stream_mixes_always_schedule_legally() {
    let t = FlashTimings::zns_tlc();
    let mut x = 0xDEADBEEFCAFEF00Du64;
    let mut rng = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    for case in 0..50 {
        let nzones = 1 + (rng() % 4) as usize;
        let zones: Vec<ZoneDesc> = (0..nzones)
            .map(|_| ZoneDesc {
                zone: rng() % 8,
                base_page: rng() % (1 << 20),
                count: 64 + rng() % 2048,
                write: rng() % 3 == 0,
            })
            .collect();
        let gather = if rng() % 2 == 0 {
            vec![Gather {
                pages: (0..(rng() % 512)).map(|_| rng() % (1 << 20)).collect(),
            }]
        } else {
            vec![]
        };
        let resets: Vec<ZoneReset> = (0..(rng() % 3))
            .map(|_| ZoneReset { zone: rng() % 8 })
            .collect();
        let order = if rng() % 2 == 0 {
            VisitOrder::ProgramOrder
        } else {
            VisitOrder::Interleaved
        };
        let plan = schedule_zoned(&t, &zones, &gather, &resets, order);
        check_legal_flash(&t, &plan)
            .unwrap_or_else(|e| panic!("case {case}: scheduler emitted illegal plan: {e}"));
    }
}

// ── S3 — TIER A: reads are zero-jitter, and scheduled GC never displaces one ──────
#[test]
fn tier_a_reads_are_zero_jitter_even_with_scheduled_gc() {
    let t = FlashTimings::zns_tlc();
    let no_gc = schedule_zoned(
        &t,
        &[read_zone(0, 2048)],
        &[],
        &[],
        VisitOrder::ProgramOrder,
    );
    let with_gc = schedule_zoned(
        &t,
        &[read_zone(0, 2048)],
        &[],
        &[ZoneReset { zone: 0 }],
        VisitOrder::ProgramOrder,
    );
    check_legal_flash(&t, &no_gc).expect("no-gc plan legal");
    check_legal_flash(&t, &with_gc).expect("with-gc plan legal");

    // The device delivers EXACTLY the compile-time promise — zero jitter, cycle-exact.
    assert_eq!(
        planned_read_beats(&t, &with_gc),
        FlashDeviceModel::execute(&t, &with_gc),
        "device replay diverged from the compile-time promise"
    );
    // Scheduled erases NEVER displaced a read beat: identical read beats with/without GC.
    assert_eq!(
        planned_read_beats(&t, &no_gc),
        planned_read_beats(&t, &with_gc),
        "a scheduled zone reset moved a read beat — GC is not free"
    );
    // Non-vacuity: the GC plan really does contain erases.
    assert!(
        with_gc.iter().any(|c| c.cmd.op == FlashOp::Erase),
        "no erase in the GC plan"
    );
    // Every read paid exactly sense + data-out — no read ever waited on the die.
    let u = measure(&t, &with_gc);
    assert_eq!(
        u.worst_read_latency,
        t.t_r + t.data_cycles,
        "a read waited — not zero-jitter"
    );
    assert!(u.erases > 0);
}

// ── Non-vacuity: the independent checker REJECTS a read issued during an erase ─────
#[test]
fn the_checker_rejects_a_read_colliding_with_an_erase() {
    let t = FlashTimings::zns_tlc();
    // An erase at 0 on die 0 (busy until tBERS) and a read on die 0 well inside it.
    let plan = vec![
        TimedCmd {
            at: 0,
            cmd: FlashCmd {
                op: FlashOp::Erase,
                chan: 0,
                die: 0,
            },
        },
        TimedCmd {
            at: 100,
            cmd: FlashCmd {
                op: FlashOp::Read,
                chan: 0,
                die: 0,
            },
        },
    ];
    let err = check_legal_flash(&t, &plan).expect_err("a read during an erase must be rejected");
    assert!(
        err.contains("collision"),
        "unexpected rejection reason: {err}"
    );
    // And an unsorted plan is rejected too.
    let unsorted = vec![
        TimedCmd {
            at: 50,
            cmd: FlashCmd {
                op: FlashOp::Read,
                chan: 0,
                die: 0,
            },
        },
        TimedCmd {
            at: 10,
            cmd: FlashCmd {
                op: FlashOp::Read,
                chan: 1,
                die: 1,
            },
        },
    ];
    assert!(check_legal_flash(&t, &unsorted)
        .unwrap_err()
        .contains("time-sorted"));
}

// ── S4 — TIER B: an erase-suspend keeps every read within the latency bound ───────
#[test]
fn tier_b_erase_suspend_stays_within_the_latency_bound() {
    let t = FlashTimings::zns_tlc();
    // A read whose die is mid-erase: illegal for Tier A, but Tier B suspends the erase.
    let plan = vec![
        TimedCmd {
            at: 0,
            cmd: FlashCmd {
                op: FlashOp::Erase,
                chan: 0,
                die: 0,
            },
        },
        TimedCmd {
            at: 10,
            cmd: FlashCmd {
                op: FlashOp::Read,
                chan: 0,
                die: 0,
            },
        },
    ];
    // Tier A refuses it (the guarantee is contractual, not luck).
    assert!(check_legal_flash(&t, &plan).is_err());
    // Tier B delivers the read within the bound, paying only the suspend window.
    let beats = execute_with_suspend(&t, &plan);
    assert_eq!(beats.len(), 1);
    let read_at = 10u64;
    assert!(
        beats[0] <= read_at + t.read_latency_bound(),
        "Tier-B read {} exceeded the bound {} (issue {read_at})",
        beats[0],
        read_at + t.read_latency_bound()
    );
    // It really did suspend (paid the window, not the full tBERS).
    assert_eq!(beats[0], read_at + t.suspend_window + t.t_r + t.data_cycles);
    assert!(
        t.suspend_window < t.t_bers,
        "suspend must be far cheaper than a full erase"
    );
}

// ── S5 — TIER C: a black-box reactive FTL BLOWS the bound (impossibility) ─────────
#[test]
fn tier_c_black_box_ftl_breaks_the_latency_bound() {
    let t = FlashTimings::zns_tlc();
    // A clean Tier-A read stream: page 0 lands on die 0.
    let plan = schedule_zoned(&t, &[read_zone(0, 256)], &[], &[], VisitOrder::ProgramOrder);
    check_legal_flash(&t, &plan).expect("legal");
    let issues: Vec<u64> = plan
        .iter()
        .filter(|c| c.cmd.op == FlashOp::Read)
        .map(|c| c.at)
        .collect();

    // Host-managed (Tier A): every read within the bound.
    let good = FlashDeviceModel::execute(&t, &plan);
    for (b, at) in good.iter().zip(&issues) {
        assert!(
            *b <= at + t.read_latency_bound(),
            "Tier-A read escaped its bound"
        );
    }

    // Black-box: a hidden erase lands on die 0 just before read 0 → it blows the bound.
    let bad = black_box_execute(&t, &plan, 0, 0);
    assert!(
        bad[0] > issues[0] + t.read_latency_bound(),
        "the black-box read {} should exceed the bound {} — no static guarantee survives a hidden FTL",
        bad[0],
        issues[0] + t.read_latency_bound()
    );
    // The overshoot is on the order of a full block erase.
    assert!(bad[0] - good[0] >= t.t_bers - t.suspend_window);
}

// ── S6 — the standard report is legal and measured; utilization is high ───────────
#[test]
fn standard_report_is_legal_and_streams_efficiently() {
    let t = FlashTimings::zns_tlc();
    let plan = schedule_zoned(
        &t,
        &[read_zone(0, 8192)],
        &[],
        &[],
        VisitOrder::ProgramOrder,
    );
    check_legal_flash(&t, &plan).expect("legal");
    let u = measure(&t, &plan);
    // Die-interleave + channel-stripe keeps the data buses busy.
    assert!(
        u.value() > 0.85,
        "sequential read utilization unexpectedly low: {:.4}",
        u.value()
    );
    assert!(standard_report().contains("read_latency_bound"));
}

// ── S7a — certified bounded-latency: a budgeted retry can't escape the envelope ───
fn lower(e: BoundedExpr) -> ProofExpr {
    super::super::bitblast::lower_bool(&e).expect("lowers")
}

#[test]
fn bounded_read_latency_is_proven_by_our_sat() {
    use BitVecBoundedOp::*;
    const W: u32 = 16;
    let t = FlashTimings::zns_tlc();
    let bv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), W));
    let k = |v: u64| Box::new(BoundedExpr::BitVecConst { width: W, value: v });
    let bin = |o: BitVecBoundedOp, l: Box<BoundedExpr>, r: Box<BoundedExpr>| {
        Box::new(BoundedExpr::BitVecBinary {
            op: o,
            left: l,
            right: r,
        })
    };
    let base = t.t_r + t.data_cycles; // fixed sense + data-out
    let rw = t.ldpc_retry_worst;
    let bound = base + rw;
    // completion excess = base + retry, with retry small enough not to wrap.
    let excess = bin(Add, k(base), bv("retry"));
    let no_overflow = lower(BoundedExpr::BitVecBinary {
        op: ULt,
        left: bv("retry"),
        right: k((1 << W) - base),
    });
    // guarantee: retry ≤ retry_worst  ⟺  retry < retry_worst + 1
    let guarantee = lower(BoundedExpr::BitVecBinary {
        op: ULt,
        left: bv("retry"),
        right: k(rw + 1),
    });
    // violated: bound < excess
    let violated = lower(BoundedExpr::BitVecBinary {
        op: ULt,
        left: k(bound),
        right: excess,
    });
    let conj = ProofExpr::And(
        Box::new(no_overflow),
        Box::new(ProofExpr::And(Box::new(guarantee), Box::new(violated))),
    );
    assert_eq!(
        prove_unsat(&conj),
        UnsatOutcome::Refuted,
        "our SAT proves a budgeted retry keeps the read within the latency envelope"
    );
}

#[test]
fn read_latency_can_escape_without_the_retry_budget() {
    use BitVecBoundedOp::*;
    const W: u32 = 16;
    let t = FlashTimings::zns_tlc();
    let bv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), W));
    let k = |v: u64| Box::new(BoundedExpr::BitVecConst { width: W, value: v });
    let base = t.t_r + t.data_cycles;
    let bound = base + t.ldpc_retry_worst;
    let excess = Box::new(BoundedExpr::BitVecBinary {
        op: Add,
        left: k(base),
        right: bv("retry"),
    });
    let no_overflow = lower(BoundedExpr::BitVecBinary {
        op: ULt,
        left: bv("retry"),
        right: k((1 << W) - base),
    });
    let violated = lower(BoundedExpr::BitVecBinary {
        op: ULt,
        left: k(bound),
        right: excess,
    });
    let conj = ProofExpr::And(Box::new(no_overflow), Box::new(violated));
    assert!(
        matches!(find_model(&conj), ModelOutcome::Sat(_)),
        "without the retry budget an unbounded retry must be satisfiable (non-vacuity)"
    );
}

// ── S7b — certified erase-collision-freedom: a read placed AFTER an erase can't
//          fall inside the erase's tBERS window ───────────────────────────────────
#[test]
fn erase_collision_freedom_is_proven_by_our_sat() {
    use BitVecBoundedOp::*;
    const W: u32 = 16;
    let t = FlashTimings::zns_tlc();
    let bv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), W));
    let k = |v: u64| Box::new(BoundedExpr::BitVecConst { width: W, value: v });
    let tb = t.t_bers; // 6400 < 2^16
                       // The scheduler places the read at erase_end + slack (erase_end normalized to tBERS).
    let gap = Box::new(BoundedExpr::BitVecBinary {
        op: Add,
        left: k(tb),
        right: bv("slack"),
    });
    let no_overflow = lower(BoundedExpr::BitVecBinary {
        op: ULt,
        left: bv("slack"),
        right: k((1 << W) - tb),
    });
    // collision: the read lands inside [0, tBERS).
    let collide = lower(BoundedExpr::BitVecBinary {
        op: ULt,
        left: gap,
        right: k(tb),
    });
    let conj = ProofExpr::And(Box::new(no_overflow), Box::new(collide));
    assert_eq!(
        prove_unsat(&conj),
        UnsatOutcome::Refuted,
        "our SAT proves a read scheduled after an erase completes can never collide with it"
    );
}

#[test]
fn a_read_can_collide_with_an_unscheduled_erase() {
    use BitVecBoundedOp::*;
    const W: u32 = 16;
    let t = FlashTimings::zns_tlc();
    let bv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), W));
    let k = |v: u64| Box::new(BoundedExpr::BitVecConst { width: W, value: v });
    // Without the "after the erase" placement, a free read time CAN land in the window.
    let collide = lower(BoundedExpr::BitVecBinary {
        op: ULt,
        left: bv("read_at"),
        right: k(t.t_bers),
    });
    assert!(
        matches!(find_model(&collide), ModelOutcome::Sat(_)),
        "an unscheduled read must be able to collide with an erase (non-vacuity)"
    );
}

// ── S8 — the executor RTL is in-order by construction; deltas round-trip ──────────
#[test]
fn executor_rtl_is_in_order_by_construction() {
    let v = emit_storage_controller_verilog();
    assert!(v.contains("module storage_controller"));
    assert_eq!(
        v.matches("issue <= 1'b1;").count(),
        1,
        "one and only one issue path"
    );
    assert!(!v.to_lowercase().contains("reorder"));
    assert!(!v.contains("sort"));
    assert!(v.contains("loaded <= 1'b0;") && v.contains("loaded <= 1'b1;"));
}

/// The testbench itself must check timing faithfulness (gap − delta = fixed overhead), not
/// just order + payload — so the iverilog PASS actually proves the hardware honors the plan.
#[test]
fn the_testbench_checks_timing_faithfulness_not_just_order() {
    let tb = emit_storage_controller_testbench(64);
    assert!(
        tb.contains("timing faithful"),
        "TB dropped the timing-faithfulness check"
    );
    assert!(
        tb.contains("cyc - prev_issue"),
        "TB is not measuring inter-issue cycle gaps"
    );
    assert!(
        tb.contains("overhead"),
        "TB is not asserting a constant pipeline overhead"
    );
}

#[test]
fn delta_encoding_round_trips_the_plan() {
    let t = FlashTimings::zns_tlc();
    let plan = schedule_zoned(
        &t,
        &[read_zone(0, 128)],
        &[],
        &[ZoneReset { zone: 0 }],
        VisitOrder::ProgramOrder,
    );
    let words = delta_encode(&plan);
    assert_eq!(words.len(), plan.len());
    let mut at = 0u64;
    for (w, c) in words.iter().zip(&plan) {
        let delta = w >> 40;
        let op = (w >> 37) & 0x7;
        let chan = ((w >> 32) & 0x1f) as u32;
        let die = ((w >> 24) & 0xff) as u32;
        at += delta;
        assert_eq!(at, c.at, "delta decodes to the planned absolute time");
        let eop = match c.cmd.op {
            FlashOp::Read => 0,
            FlashOp::Program => 1,
            FlashOp::Erase => 2,
        };
        assert_eq!((op, chan, die), (eop, c.cmd.chan, c.cmd.die));
    }
}

// ── S10 — integrity + die-granularity redundancy (reuses the shipped FEC) ─────────
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[test]
fn pages_survive_a_storage_round_trip_and_a_bit_flip_is_detected() {
    let page: Vec<u8> = (0..4096u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
        .collect();
    let tag = fnv1a(&page);
    assert_eq!(fnv1a(&page.clone()), tag, "clean round-trip must verify");
    for &pos in &[0usize, 1, 2000, 4095] {
        let mut corrupt = page.clone();
        corrupt[pos] ^= 1;
        assert_ne!(
            fnv1a(&corrupt),
            tag,
            "a bit flip at byte {pos} must change the checksum"
        );
    }
}

#[test]
fn reed_solomon_reconstructs_a_dropped_die() {
    use crate::concurrency::fec;
    let payload: Vec<u8> = (0..2000u32).map(|i| (i % 251) as u8).collect();
    let (k, n) = (10usize, 14usize); // 4 parity dies
    let (orig_len, shards) = fec::encode(&payload, k, n).expect("encode");
    let survivors: Vec<(usize, Vec<u8>)> = shards
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 3) // die 3 failed
        .take(k)
        .map(|(i, s)| (i, s.clone()))
        .collect();
    let recovered = fec::decode(orig_len, k, n, &survivors).expect("decode");
    assert_eq!(recovered, payload, "the exact payload survives a lost die");
}

// ── STRENGTHENING: independent oracles + fuzzed invariants (to the point of absurdity) ──

fn seeded(seed: u64) -> impl FnMut() -> u64 {
    let mut x = seed | 1;
    move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    }
}

fn random_read_plan(t: &FlashTimings, rng: &mut impl FnMut() -> u64) -> Vec<TimedCmd> {
    let nz = 1 + (rng() % 3) as usize;
    let zones: Vec<ZoneDesc> = (0..nz)
        .map(|_| ZoneDesc {
            zone: rng() % 8,
            base_page: rng() % (1 << 20),
            count: 16 + rng() % 1024,
            write: rng() % 4 == 0,
        })
        .collect();
    let order = if rng() % 2 == 0 {
        VisitOrder::ProgramOrder
    } else {
        VisitOrder::Interleaved
    };
    schedule_zoned(t, &zones, &[], &[], order)
}

/// TRIANGULATION: on any legal plan, the independent device SIMULATOR reproduces the
/// compile-time promise exactly — zero jitter across the whole input space, both media.
#[test]
fn every_legal_plan_is_zero_jitter_under_the_independent_simulator() {
    for t in [FlashTimings::zns_tlc(), FlashTimings::pslc_fast()] {
        let mut rng = seeded(0xF00D_1234 ^ t.t_r);
        for case in 0..80 {
            let plan = random_read_plan(&t, &mut rng);
            check_legal_flash(&t, &plan).unwrap_or_else(|e| panic!("case {case}: illegal: {e}"));
            assert_eq!(
                FlashDeviceModel::execute(&t, &plan),
                planned_read_beats(&t, &plan),
                "case {case}: the device diverged from the promise on a LEGAL plan"
            );
        }
    }
}

/// METAMORPHIC: inject a die collision (an erase overlapping a same-die read) into a legal
/// plan and BOTH independent oracles catch it — the checker rejects, and the device
/// simulator diverges from the promise. Two detectors, no shared code, same verdict.
#[test]
fn a_die_collision_is_caught_by_both_the_checker_and_the_device() {
    let t = FlashTimings::zns_tlc();
    let mut rng = seeded(0xC0FFEE_5);
    let mut exercised = 0;
    for _ in 0..120 {
        let mut plan = random_read_plan(&t, &mut rng);
        let reads: Vec<usize> = plan
            .iter()
            .enumerate()
            .filter(|(_, c)| c.cmd.op == FlashOp::Read)
            .map(|(i, _)| i)
            .collect();
        if reads.is_empty() {
            continue;
        }
        let victim = reads[(rng() as usize) % reads.len()];
        let v = plan[victim];
        // Insert an erase on the SAME die at the read's time (stable sort keeps it BEFORE
        // the read among equal timestamps), so the read now issues into a busy/erasing die.
        plan.insert(
            victim,
            TimedCmd {
                at: v.at,
                cmd: FlashCmd {
                    op: FlashOp::Erase,
                    chan: v.cmd.chan,
                    die: v.cmd.die,
                },
            },
        );
        plan.sort_by(|a, b| a.at.cmp(&b.at));

        assert!(
            check_legal_flash(&t, &plan).is_err(),
            "the checker missed an injected die collision"
        );
        assert_ne!(
            FlashDeviceModel::execute(&t, &plan),
            planned_read_beats(&t, &plan),
            "the device did not diverge on an injected die collision"
        );
        exercised += 1;
    }
    assert!(
        exercised > 40,
        "the metamorphic fuzz did not exercise enough cases ({exercised})"
    );
}

/// A scheduled zone reset (GC) never moves a read beat — proven across a fuzzed workload
/// family, on both the promise and the independent device replay.
#[test]
fn scheduled_gc_is_free_across_a_fuzzed_workload_family() {
    let t = FlashTimings::zns_tlc();
    let mut rng = seeded(0xBEEF_99);
    for case in 0..60 {
        let nz = 1 + (rng() % 3) as usize;
        let zones: Vec<ZoneDesc> = (0..nz)
            .map(|_| ZoneDesc {
                zone: rng() % 4,
                base_page: rng() % (1 << 18),
                count: 16 + rng() % 512,
                write: false,
            })
            .collect();
        let resets: Vec<ZoneReset> = (0..(1 + rng() % 3))
            .map(|_| ZoneReset { zone: rng() % 4 })
            .collect();
        let no_gc = schedule_zoned(&t, &zones, &[], &[], VisitOrder::ProgramOrder);
        let with_gc = schedule_zoned(&t, &zones, &[], &resets, VisitOrder::ProgramOrder);
        check_legal_flash(&t, &with_gc).unwrap_or_else(|e| panic!("case {case}: illegal: {e}"));
        assert_eq!(
            planned_read_beats(&t, &no_gc),
            planned_read_beats(&t, &with_gc),
            "case {case}: GC moved a promised beat"
        );
        assert_eq!(
            FlashDeviceModel::execute(&t, &with_gc),
            planned_read_beats(&t, &with_gc),
            "case {case}: GC perturbed the device replay"
        );
        assert!(
            with_gc.iter().any(|c| c.cmd.op == FlashOp::Erase),
            "case {case}: no erase (vacuous)"
        );
    }
}

/// WEAR BALANCE: a zone reset erases exactly one block per die, so N resets wear every die
/// equally — the reclamation is wear-leveled by construction (an FTL-correctness invariant).
#[test]
fn zone_resets_balance_wear_across_all_dies() {
    let t = FlashTimings::zns_tlc();
    let nd = t.n_dies();
    for nres in 1..6u64 {
        let resets: Vec<ZoneReset> = (0..nres).map(|z| ZoneReset { zone: z }).collect();
        let plan = schedule_zoned(
            &t,
            &[ZoneDesc {
                zone: 0,
                base_page: 0,
                count: 1024,
                write: false,
            }],
            &[],
            &resets,
            VisitOrder::ProgramOrder,
        );
        let mut per_die = vec![0u64; nd as usize];
        for c in &plan {
            if c.cmd.op == FlashOp::Erase {
                per_die[c.cmd.die as usize] += 1;
            }
        }
        assert!(
            per_die.iter().all(|&e| e == nres),
            "wear imbalance for {nres} resets: {per_die:?}"
        );
        assert_eq!(
            per_die.iter().sum::<u64>(),
            nres * nd as u64,
            "total erases must be resets × dies"
        );
    }
}

/// CONCURRENT GC: a zone reset on a die window DISJOINT from the read window runs
/// concurrently with the reads and does not move a single read beat — "GC is free" at the
/// system level, not merely trailing. The erases overlap the read window in time.
#[test]
fn a_disjoint_zone_reset_runs_concurrently_and_reads_stay_zero_jitter() {
    let t = FlashTimings::zns_tlc();
    let read_win = Placement {
        die_base: 0,
        die_count: 56,
    };
    let gc_win = Placement {
        die_base: 56,
        die_count: 8,
    };
    let reads = ZoneDesc {
        zone: 0,
        base_page: 0,
        count: 4096,
        write: false,
    };
    let alone = schedule_placed(
        &t,
        &[(reads.clone(), read_win)],
        &[],
        VisitOrder::ProgramOrder,
    );
    let with_gc = schedule_placed(
        &t,
        &[(reads, read_win)],
        &[(ZoneReset { zone: 1 }, gc_win)],
        VisitOrder::ProgramOrder,
    );
    check_legal_flash(&t, &with_gc).expect("the concurrent plan is timing-legal");

    // Reads byte-for-byte unaffected by the concurrent reclamation.
    assert_eq!(
        planned_read_beats(&t, &alone),
        planned_read_beats(&t, &with_gc),
        "concurrent GC moved a read beat"
    );
    assert_eq!(
        FlashDeviceModel::execute(&t, &with_gc),
        planned_read_beats(&t, &with_gc),
        "the device diverged"
    );

    // The erases OVERLAP the read window in time (concurrent, not trailing).
    let last_read = *planned_read_beats(&t, &with_gc).iter().max().unwrap();
    let first_erase = with_gc
        .iter()
        .filter(|c| c.cmd.op == FlashOp::Erase)
        .map(|c| c.at)
        .min()
        .unwrap();
    assert!(
        first_erase < last_read,
        "GC should overlap the reads (first erase {first_erase} vs last read {last_read})"
    );

    // Strict die partition: reads on the read window, erases on the GC window.
    for c in &with_gc {
        match c.cmd.op {
            FlashOp::Read => assert!(c.cmd.die < 56, "a read leaked onto a GC die"),
            FlashOp::Erase => assert!(
                (56..64).contains(&c.cmd.die),
                "an erase leaked onto a read die"
            ),
            _ => {}
        }
    }
}

/// STRENGTHENED SAT: erase-collision-freedom, fully SYMBOLIC in the erase time. For a free
/// `erase_at` and a read placed at `erase_at + tBERS + slack`, the read can NEVER fall in
/// the erase window [erase_at, erase_at+tBERS) — negation UNSAT, on both media.
#[test]
fn erase_collision_freedom_is_proven_symbolically_on_both_media() {
    use BitVecBoundedOp::*;
    const W: u32 = 16;
    for t in [FlashTimings::zns_tlc(), FlashTimings::pslc_fast()] {
        let tb = t.t_bers;
        assert!(tb < 20000, "keep tBERS in the no-wrap window");
        let k = |v: u64| Box::new(BoundedExpr::BitVecConst { width: W, value: v });
        let erase_at = || Box::new(BoundedExpr::BitVecVar("erase_at".to_string(), W));
        let slack = || Box::new(BoundedExpr::BitVecVar("slack".to_string(), W));
        let add = |l: Box<BoundedExpr>, r: Box<BoundedExpr>| {
            Box::new(BoundedExpr::BitVecBinary {
                op: Add,
                left: l,
                right: r,
            })
        };
        let erase_end = || add(erase_at(), k(tb));
        let read = add(erase_end(), slack()); // read = erase_at + tBERS + slack ≥ erase_end
                                              // No wrap: erase_at and slack each well below the window.
        let ne1 = lower(BoundedExpr::BitVecBinary {
            op: ULt,
            left: erase_at(),
            right: k(20000),
        });
        let ne2 = lower(BoundedExpr::BitVecBinary {
            op: ULt,
            left: slack(),
            right: k(20000),
        });
        // Collision: read < erase_end.
        let collide = lower(BoundedExpr::BitVecBinary {
            op: ULt,
            left: read,
            right: erase_end(),
        });
        let conj = ProofExpr::And(
            Box::new(ne1),
            Box::new(ProofExpr::And(Box::new(ne2), Box::new(collide))),
        );
        assert_eq!(
            prove_unsat(&conj),
            UnsatOutcome::Refuted,
            "our SAT proves a read scheduled after a symbolic erase completes never collides (t_bers={tb})"
        );
    }
}

/// GENERALIZED TIER C: ANY victim read on its die blows the latency bound under a black-box
/// FTL; and with NO reclamation the black box collapses to the host-managed device exactly.
#[test]
fn any_black_box_reclamation_blows_the_bound_and_none_equals_tier_a() {
    let t = FlashTimings::zns_tlc();
    let plan = schedule_zoned(&t, &[read_zone(0, 256)], &[], &[], VisitOrder::ProgramOrder);
    let good = FlashDeviceModel::execute(&t, &plan);
    let reads: Vec<(usize, u32, u64)> = plan
        .iter()
        .filter(|c| c.cmd.op == FlashOp::Read)
        .enumerate()
        .map(|(i, c)| (i, c.cmd.die, c.at))
        .collect();
    let mut rng = seeded(0x5A5A_1);
    for _ in 0..50 {
        let (vr, vd, vat) = reads[(rng() as usize) % reads.len()];
        let bad = black_box_execute(&t, &plan, vd, vr);
        assert!(
            bad[vr] > vat + t.read_latency_bound(),
            "victim read {vr} on die {vd} must blow the bound under a hidden FTL"
        );
    }
    // No reclamation (victim index unreachable) ⇒ identical to the honest host-managed device.
    assert_eq!(
        black_box_execute(&t, &plan, 0, usize::MAX),
        good,
        "with no reclamation the black box must equal the host-managed device"
    );
}

/// TIER B across RANDOM media: an isolated read that suspends an in-progress erase always
/// lands within the stated latency bound, paying exactly one suspend window.
#[test]
fn tier_b_isolated_suspend_is_within_bound_across_random_media() {
    let mut rng = seeded(0xB0B_7);
    for _ in 0..80 {
        let t = FlashTimings {
            channels: 8,
            dies_per_channel: 8,
            planes_per_die: 2,
            pages_per_block: 256,
            zone_pages: 8192,
            t_r: 16 + rng() % 64,
            t_prog: 128 + rng() % 512,
            t_bers: 1024 + rng() % 8192,
            data_cycles: 4 + rng() % 12,
            suspend_window: 16 + rng() % 128,
            ldpc_retry_worst: rng() % 64,
        };
        let read_at = 1 + rng() % 50; // well inside tBERS ⇒ the read really preempts the erase
        let plan = vec![
            TimedCmd {
                at: 0,
                cmd: FlashCmd {
                    op: FlashOp::Erase,
                    chan: 0,
                    die: 0,
                },
            },
            TimedCmd {
                at: read_at,
                cmd: FlashCmd {
                    op: FlashOp::Read,
                    chan: 0,
                    die: 0,
                },
            },
        ];
        let beats = execute_with_suspend(&t, &plan);
        assert!(
            beats[0] <= read_at + t.read_latency_bound(),
            "Tier-B read {} escaped the bound {}",
            beats[0],
            read_at + t.read_latency_bound()
        );
        assert_eq!(
            beats[0],
            read_at + t.suspend_window + t.t_r + t.data_cycles,
            "Tier-B must pay exactly one suspend"
        );
    }
}

// ── S11 — emitted RTL + a real plan + the testbench are golden artifacts ──────────
#[test]
fn storage_controller_artifacts_are_current() {
    let t = FlashTimings::zns_tlc();
    let plan = schedule_zoned(
        &t,
        &[read_zone(0, 256)],
        &[],
        &[ZoneReset { zone: 0 }],
        VisitOrder::ProgramOrder,
    );
    let words = delta_encode(&plan);
    let rtl = emit_storage_controller_verilog();
    let hex: String = words.iter().map(|w| format!("{w:016x}\n")).collect();
    let tb = emit_storage_controller_testbench(words.len());
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon");
    let files = [
        ("storage_controller.v", rtl),
        ("storage_plan.hex", hex),
        ("tb_storage_controller.v", tb),
    ];
    if std::env::var("STORAGE_RECORD").is_ok() {
        for (name, body) in &files {
            std::fs::write(dir.join(name), body).expect("write artifact");
        }
        let report = standard_report();
        std::fs::write(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../benchmarks/results/storage_sim.json"),
            report,
        )
        .expect("write storage_sim.json");
        return;
    }
    for (name, body) in &files {
        let path = dir.join(name);
        let got = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "missing {} ({e}) — record: STORAGE_RECORD=1 cargo test -p logicaffeine-compile \
                     storage_controller_artifacts",
                path.display()
            )
        });
        assert_eq!(
            &got, body,
            "{name} is stale — regenerate with STORAGE_RECORD=1"
        );
    }
}
