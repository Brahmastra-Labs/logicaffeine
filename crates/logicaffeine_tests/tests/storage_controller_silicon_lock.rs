//! storage_controller_silicon_lock — the deterministic NAND storage controller's
//! guarantees are pinned here, in the integration suite, independently of the module's own
//! inline battery. The memory controller proved DRAM timing can be guaranteed statically
//! (zero jitter, refresh co-scheduled); this proves the SAME for host-managed flash: a read
//! stream is zero-jitter and scheduled block erases NEVER displace a read (the tBERS analog
//! of REFpb), every read stays within a stated latency bound, and — the impossibility that
//! makes owning the controller the requirement — a black-box reactive FTL blows that bound.
//! The independent checker + device model + the certified-SAT obligations live in
//! `logicaffeine_compile::codegen_sva::storage_controller_rtl`; cited in STORAGE_EDGE_SPEC.md.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::storage_controller_rtl::{
    black_box_execute, check_legal_flash, measure, planned_read_beats, schedule_placed,
    schedule_zoned, FlashDeviceModel, FlashOp, FlashTimings, Gather, Placement, VisitOrder,
    ZoneDesc, ZoneReset,
};

fn read_zone(base: u64, count: u64) -> ZoneDesc {
    ZoneDesc {
        zone: 0,
        base_page: base,
        count,
        write: false,
    }
}

/// TIER A — the crown guarantee: the device delivers exactly the compile-time read beats
/// (zero jitter), and a host-scheduled zone reset (block erase) does not move a single read.
#[test]
fn host_managed_reads_are_zero_jitter_and_scheduled_gc_is_free() {
    for t in [FlashTimings::zns_tlc(), FlashTimings::pslc_fast()] {
        let no_gc = schedule_zoned(
            &t,
            &[read_zone(0, 4096)],
            &[],
            &[],
            VisitOrder::ProgramOrder,
        );
        let with_gc = schedule_zoned(
            &t,
            &[read_zone(0, 4096)],
            &[],
            &[ZoneReset { zone: 0 }],
            VisitOrder::ProgramOrder,
        );
        check_legal_flash(&t, &no_gc).expect("no-gc plan is timing-legal");
        check_legal_flash(&t, &with_gc).expect("with-gc plan is timing-legal");

        // The device replays the plan exactly — zero jitter.
        assert_eq!(
            planned_read_beats(&t, &with_gc),
            FlashDeviceModel::execute(&t, &with_gc),
            "device diverged from the compile-time promise"
        );
        // The scheduled erases never displaced a read beat.
        assert_eq!(
            planned_read_beats(&t, &no_gc),
            planned_read_beats(&t, &with_gc),
            "a scheduled zone reset moved a read — GC is not free"
        );
        assert!(
            with_gc.iter().any(|c| c.cmd.op == FlashOp::Erase),
            "no erase present (vacuous)"
        );

        // Every read paid exactly sense + data-out; none ever waited on its die.
        assert_eq!(
            measure(&t, &with_gc).worst_read_latency,
            t.t_r + t.data_cycles
        );
    }
}

/// The crown medium saturates the aggregate data bus on a sequential read stream (die depth
/// hides the sense latency behind the channel transfer).
#[test]
fn crown_sequential_reads_saturate_the_bus() {
    let t = FlashTimings::zns_tlc();
    let plan = schedule_zoned(
        &t,
        &[read_zone(0, 8192)],
        &[],
        &[],
        VisitOrder::ProgramOrder,
    );
    check_legal_flash(&t, &plan).expect("legal");
    let util = measure(&t, &plan).value();
    assert!(
        util >= 0.99,
        "crown sequential utilization regressed below 0.99: {util:.4}"
    );
}

/// CONCURRENT reclamation: a zone reset on a die window DISJOINT from the read window runs
/// alongside the reads without moving a single beat — "GC is free" at the system level, and the
/// erases genuinely overlap the read window in time. Reads sustain a high bus utilization.
#[test]
fn a_concurrent_zone_reset_on_disjoint_dies_is_free() {
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
        count: 6144,
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
    check_legal_flash(&t, &with_gc).expect("concurrent plan is legal");

    assert_eq!(
        planned_read_beats(&t, &alone),
        planned_read_beats(&t, &with_gc),
        "concurrent GC moved a read"
    );
    assert_eq!(
        FlashDeviceModel::execute(&t, &with_gc),
        planned_read_beats(&t, &with_gc),
        "device diverged"
    );

    let last_read = *planned_read_beats(&t, &with_gc).iter().max().unwrap();
    let first_erase = with_gc
        .iter()
        .filter(|c| c.cmd.op == FlashOp::Erase)
        .map(|c| c.at)
        .min()
        .unwrap();
    assert!(
        first_erase < last_read,
        "the erase must overlap the read window (concurrent, not trailing)"
    );
    assert!(
        measure(&t, &with_gc).value() >= 0.80,
        "reads should sustain a high bus while GC runs"
    );
}

/// Every host-managed read completes within the stated latency bound — the contract the
/// static schedule keeps.
#[test]
fn every_host_managed_read_is_within_the_latency_bound() {
    let t = FlashTimings::zns_tlc();
    let mut x = 0xA5A5_1234_DEAD_0001u64;
    let mut rng = move || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let pages: Vec<u64> = (0..2048).map(|_| rng() % (1 << 22)).collect();
    let plan = schedule_zoned(&t, &[], &[Gather { pages }], &[], VisitOrder::ProgramOrder);
    check_legal_flash(&t, &plan).expect("legal");
    let issues: Vec<u64> = plan
        .iter()
        .filter(|c| c.cmd.op == FlashOp::Read)
        .map(|c| c.at)
        .collect();
    let beats = FlashDeviceModel::execute(&t, &plan);
    for (b, at) in beats.iter().zip(&issues) {
        assert!(
            *b <= at + t.read_latency_bound(),
            "a host-managed read escaped its {}-cycle bound",
            t.read_latency_bound()
        );
    }
}

/// TIER C — the impossibility: the SAME plan run on a black-box SSD (a hidden FTL that
/// reclaims reactively) blows the latency bound. This is why the guarantee requires owning
/// the controller — you cannot schedule what you do not control.
#[test]
fn a_black_box_ftl_provably_breaks_the_bound() {
    let t = FlashTimings::zns_tlc();
    let plan = schedule_zoned(&t, &[read_zone(0, 512)], &[], &[], VisitOrder::ProgramOrder);
    check_legal_flash(&t, &plan).expect("legal");
    let issues: Vec<u64> = plan
        .iter()
        .filter(|c| c.cmd.op == FlashOp::Read)
        .map(|c| c.at)
        .collect();

    // Host-managed: within bound.
    let good = FlashDeviceModel::execute(&t, &plan);
    assert!(good[0] <= issues[0] + t.read_latency_bound());

    // Black-box: a hidden erase lands on die 0 just before read 0 → the bound is blown.
    let bad = black_box_execute(&t, &plan, 0, 0);
    assert!(
        bad[0] > issues[0] + t.read_latency_bound(),
        "the black-box read must exceed the bound — no static guarantee survives a hidden FTL"
    );
    assert!(
        bad[0] - good[0] >= t.t_bers - t.suspend_window,
        "the overshoot is erase-sized"
    );
}

/// The independent checker is not a rubber stamp: a read issued while its die is mid-erase
/// (the exact collision the schedule avoids) is REJECTED.
#[test]
fn the_independent_checker_rejects_an_erase_read_collision() {
    let t = FlashTimings::zns_tlc();
    use logicaffeine_compile::codegen_sva::storage_controller_rtl::{FlashCmd, TimedCmd};
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
    assert!(check_legal_flash(&t, &plan)
        .unwrap_err()
        .contains("collision"));
}

/// The synthesized executor's ECP5 area/timing floor — measured by yosys + nextpnr-ecp5 and
/// recorded in `benchmarks/silicon/reports.json` (re-measured by scripts/synth-soft-core.sh
/// → scripts/record-silicon-reports.sh). Ratchets: area may not balloon, Fmax may not sink,
/// the functional sim must PASS.
#[test]
fn the_synthesized_executor_meets_its_area_and_timing_floor() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/silicon/reports.json");
    let raw = std::fs::read_to_string(&path).expect("read reports.json");
    let doc: serde_json::Value = serde_json::from_str(&raw).expect("parse reports.json");
    let b = &doc["blocks"]["storage_controller"];
    assert!(
        !b.is_null(),
        "reports.json missing the storage_controller block"
    );
    assert_eq!(
        b["sim"], "PASS",
        "storage_controller functional sim must PASS"
    );
    let comb = b["trellis_comb"].as_u64().expect("trellis_comb");
    let ff = b["trellis_ff"].as_u64().expect("trellis_ff");
    let fmax = b["fmax_mhz"].as_f64().expect("fmax_mhz");
    // Same countdown-FSM class as dram_executor — a wide drift means the design changed shape.
    assert!(comb <= 150, "storage_controller LUTs ballooned: {comb}");
    assert!(ff <= 100, "storage_controller FFs ballooned: {ff}");
    assert!(
        fmax >= 120.0,
        "storage_controller Fmax sank below 120 MHz: {fmax}"
    );
}
