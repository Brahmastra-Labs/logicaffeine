use super::super::ddr2_phy::{ddr2_pins, Ddr2Cmd};
use super::super::dram::DramTimings;
use super::*;

const G25: Ddr2Grade = Ddr2Grade::MT47H64M16_25;
const G25E: Ddr2Grade = Ddr2Grade::MT47H64M16_25E;

fn ddr800() -> Ddr2Timing {
    G25.at(2500, 8).expect("-25 runs DDR2-800")
}

fn ddr400() -> Ddr2Timing {
    G25.at(5000, 8).expect("-25 runs DDR2-400")
}

fn cmd(cycle: u64, cmd: Ddr2Cmd, ba: u8, a: u16) -> BusCmd {
    BusCmd { cycle, cmd, ba, a }
}
fn act(cycle: u64, ba: u8, row: u16) -> BusCmd {
    cmd(cycle, Ddr2Cmd::Active, ba, row)
}
fn rd(cycle: u64, ba: u8, col: u16) -> BusCmd {
    cmd(cycle, Ddr2Cmd::Read, ba, col)
}
fn wr(cycle: u64, ba: u8, col: u16) -> BusCmd {
    cmd(cycle, Ddr2Cmd::Write, ba, col)
}
fn pre(cycle: u64, ba: u8) -> BusCmd {
    cmd(cycle, Ddr2Cmd::Precharge, ba, 0)
}
fn pre_all(cycle: u64) -> BusCmd {
    cmd(cycle, Ddr2Cmd::Precharge, 0, 1 << 10)
}
fn refresh(cycle: u64) -> BusCmd {
    cmd(cycle, Ddr2Cmd::Refresh, 0, 0)
}

/// The rule a trace breaks, and at which command; panics on a legal trace.
fn broken(t: &Ddr2Timing, trace: &[BusCmd]) -> (&'static str, usize) {
    match check_bus(t, trace) {
        Ok(_) => panic!("expected a violation in {trace:?}"),
        Err(v) => (v.rule, v.index),
    }
}

fn legal(t: &Ddr2Timing, trace: &[BusCmd]) -> BusFacts {
    check_bus(t, trace).unwrap_or_else(|v| panic!("expected a legal trace, got {v}"))
}

// ── the part ─────────────────────────────────────────────────────────────────────────────

/// The two MT47H64M16 grades, field by field, as Micron's 1Gb DDR2 datasheet (Rev. V 6/10) prints them: Table 1
/// for the CAS latency each clock allows, Table 12's x16 rows for every minimum. Typed here a second time so a slip
/// in either copy is a failure.
#[test]
fn the_grades_are_the_datasheet_rows() {
    for g in [G25, G25E] {
        assert_eq!((g.banks, g.row_bits, g.col_bits, g.dq_bits), (8, 13, 10, 16), "{}: 1 Gb x16 geometry", g.part);
        assert_eq!(g.tras_ps, 40_000, "{}: tRAS", g.part);
        assert_eq!(g.tras_max_ps, 70_000_000, "{}: tRAS max (70K ns)", g.part);
        assert_eq!(g.trc_ps, 55_000, "{}: tRC", g.part);
        assert_eq!(g.trrd_ps, 10_000, "{}: tRRD x16", g.part);
        assert_eq!(g.tfaw_ps, 45_000, "{}: tFAW x16", g.part);
        assert_eq!(g.twr_ps, 15_000, "{}: tWR", g.part);
        assert_eq!(g.twtr_ps, 7_500, "{}: tWTR", g.part);
        assert_eq!(g.trtp_ps, 7_500, "{}: tRTP", g.part);
        assert_eq!(g.trfc_ps, 127_500, "{}: tRFC 1Gb", g.part);
        assert_eq!(g.trefi_ps, 7_800_000, "{}: tREFI commercial", g.part);
        assert_eq!(g.tccd_ck, 2, "{}: tCCD", g.part);
        assert_eq!(g.tck_max_ps, 8_000, "{}: tCK (avg) max", g.part);
    }
    assert_eq!((G25.trcd_ps, G25.trp_ps), (15_000, 15_000), "-25 is 6-6-6 at DDR2-800");
    assert_eq!((G25E.trcd_ps, G25E.trp_ps), (12_500, 12_500), "-25E is 5-5-5 at DDR2-800");
    assert_eq!(G25.cl_floors, &[(5000, 3), (3750, 4), (3000, 5), (2500, 6)], "-25: CL3 400, CL4 533, CL5 667, CL6 800 MT/s");
    assert_eq!(G25E.cl_floors, &[(5000, 3), (3750, 4), (2500, 5)], "-25E: CL3 400, CL4 533, CL5 800 MT/s");
    assert_eq!(G25.part, "MT47H64M16-25");
    assert_eq!(G25E.part, "MT47H64M16-25E");
}

#[test]
fn ddr2_800_clocks_the_board_grade() {
    let t = ddr800();
    assert_eq!((t.tck_ps, t.bl, t.cl, t.wl), (2500, 8, 6, 5));
    assert_eq!((t.trcd, t.trp, t.trpa), (6, 6, 7), "tRPA = tRP + tCK = 17.5 ns");
    assert_eq!((t.tras, t.trc, t.trrd, t.tfaw), (16, 22, 4, 18));
    assert_eq!((t.twr, t.twtr, t.trtp, t.tccd), (6, 3, 3, 2));
    assert_eq!((t.trfc, t.trefi, t.tras_max), (51, 3120, 28_000));
}

#[test]
fn ddr2_400_clocks_the_board_grade() {
    let t = ddr400();
    assert_eq!((t.tck_ps, t.bl, t.cl, t.wl), (5000, 8, 3, 2));
    assert_eq!((t.trcd, t.trp, t.trpa), (3, 3, 4), "tRPA = tRP + tCK = 20 ns at this clock, not the 17.5 ns of DDR2-800");
    assert_eq!((t.tras, t.trc, t.trrd, t.tfaw), (8, 11, 2, 9));
    assert_eq!((t.twr, t.twtr, t.trtp, t.tccd), (3, 2, 2, 2), "7.5 ns rounds up to 2 clocks");
    assert_eq!((t.trfc, t.trefi, t.tras_max), (26, 1560, 14_000), "a minimum rounds up, a maximum down");
}

/// A maximum rounds down where the clock does not divide it: 7.8 µs at 2.8 ns is 2785.7 clocks, 70 µs at 3.7 ns is
/// 18918.9; a minimum rounds up there (15 ns at 2.8 ns is 5.4 clocks).
#[test]
fn maximums_round_down_where_the_clock_does_not_divide_them() {
    let t = G25.at(2800, 8).unwrap();
    assert_eq!((t.trefi, t.trcd), (2785, 6));
    assert_eq!(G25.at(3700, 8).unwrap().tras_max, 18918);
}

/// tRRD, tWR, tWTR and tRTP keep a two-clock minimum at any clock (Table 12 note 37); nothing else does.
#[test]
fn four_minimums_never_fall_below_two_clocks() {
    let t = G25.at(8000, 8).unwrap();
    assert_eq!((t.trrd, t.twr, t.twtr, t.trtp), (2, 2, 2, 2));
    assert_eq!((t.trcd, t.trp, t.trpa), (2, 2, 3), "15 ns, 15 ns, 23 ns at 8 ns carry no floor");
    let t = G25.at(7600, 4).unwrap();
    assert_eq!((t.twtr, t.trtp), (2, 2), "7.5 ns is one clock at 7.6 ns; the floor makes it two");
    assert_eq!(t.trrd, 2, "10 ns is two clocks at 7.6 ns on its own");
}

#[test]
fn the_cas_latency_is_the_smallest_the_clock_allows() {
    for (tck, cl) in [(2500, 6), (2800, 6), (2999, 6), (3000, 5), (3500, 5), (3749, 5), (3750, 4), (4999, 4), (5000, 3), (8000, 3)] {
        assert_eq!(G25.at(tck, 8).unwrap().cl, cl, "-25 at {tck} ps");
        assert_eq!(G25.at(tck, 8).unwrap().wl, cl - 1, "write latency is CL - 1");
    }
    for (tck, cl) in [(2500, 5), (3000, 5), (3749, 5), (3750, 4), (5000, 3)] {
        assert_eq!(G25E.at(tck, 8).unwrap().cl, cl, "-25E at {tck} ps");
    }
}

#[test]
fn a_clock_or_burst_the_part_cannot_run_is_refused() {
    assert!(G25.at(2499, 8).is_err(), "faster than DDR2-800");
    assert!(G25.at(8001, 8).is_err(), "slower than tCK max: the DLL does not lock");
    assert!(G25.at(0, 8).is_err());
    for bl in [0, 1, 2, 3, 5, 6, 7, 16] {
        assert!(G25.at(5000, bl).is_err(), "BL {bl}: DDR2 bursts are 4 or 8");
    }
    assert!(G25.at(5000, 4).is_ok());
}

/// The JESD79-2 spacings with AL = 0, against the datasheet's own figures: Figure 61 (WRITE-to-READ, CL 3, BL 4:
/// READ six clocks after WRITE) and Figure 49 (READ-to-WRITE, BL 4: four clocks).
#[test]
fn the_command_spacings_are_the_datasheet_formulas() {
    let t = ddr800();
    assert_eq!(t.read_to_precharge(), 4 - 2 + 3, "AL + BL/2 - 2 + max(tRTP, 2)");
    assert_eq!(t.write_to_precharge(), 5 + 4 + 6, "WL + BL/2 + tWR");
    assert_eq!(t.write_to_read(), 5 + 4 + 3, "WL + BL/2 + tWTR");
    assert_eq!(t.read_to_write(), 4 + 2, "BL/2 + 2");
    assert_eq!(t.cas_to_cas(), 4, "a burst is never cut short: max(tCCD, BL/2)");
    let t = G25.at(2500, 4).unwrap();
    assert_eq!((t.read_to_precharge(), t.write_to_precharge(), t.write_to_read(), t.read_to_write(), t.cas_to_cas()), (3, 13, 10, 4, 2));
    let t = ddr400();
    assert_eq!((t.read_to_precharge(), t.write_to_precharge(), t.write_to_read(), t.read_to_write(), t.cas_to_cas()), (4, 9, 8, 6, 4));
    let fig = G25.at(5000, 4).unwrap();
    assert_eq!(fig.cl, 3);
    assert_eq!(fig.write_to_read(), 6, "Figure 61: WRITE at T0, READ at T6");
    assert_eq!(fig.read_to_write(), 4, "Figure 49: BL 4, READ-to-WRITE of four clocks");
}

/// The table `ddr2_ctrl` and the planner read: DDR2's write latency rides in write recovery and the write→read
/// turnaround in `tturn`, so the abstract model's spacings are the JEDEC ones at DDR2-400.
#[test]
fn the_dram_table_carries_the_jedec_spacings() {
    let t = ddr400();
    let d = t.dram_timings();
    assert_eq!((d.banks, d.row_bytes, d.burst_bytes, d.data_cycles), (8, 2048, 16, 4));
    assert_eq!((d.trcd, d.trp, d.tras, d.trc, d.trrd, d.tfaw, d.tccd), (3, 3, 8, 11, 2, 9, 2));
    assert_eq!((d.trfc_pb, d.trefi_pb, d.hot), (26, 1560, false));
    assert_eq!(d.twr, t.wl + t.twr);
    assert_eq!(d.tturn, (t.wl + t.twtr).max(2));
    assert_eq!(d.data_cycles + d.twr, t.write_to_precharge(), "WRITE→PRECHARGE is exact");
    assert_eq!(d.tccd.max(d.data_cycles) + d.tturn, t.write_to_read(), "WRITE→READ is exact");
    assert!(d.tccd.max(d.data_cycles) + d.tturn >= t.read_to_write(), "READ→WRITE is never shorter");
    assert_eq!(d.data_cycles, t.read_to_precharge(), "READ→PRECHARGE is the burst at tRTP ≤ 2 clocks");
}

/// `DramTimings::ddr2_micron()` is the -25E grade at DDR2-800 and no faster on any datasheet minimum.
#[test]
fn the_existing_ddr2_table_is_never_faster_than_its_grade() {
    let m = DramTimings::ddr2_micron();
    let t = G25E.at(2500, 8).unwrap();
    assert_eq!((m.banks, m.row_bytes, m.burst_bytes, m.data_cycles), (8, 2048, 16, t.bl / 2));
    for (name, have, floor) in [
        ("tRCD", m.trcd, t.trcd),
        ("tRP", m.trp, t.trp),
        ("tRAS", m.tras, t.tras),
        ("tRC", m.trc, t.trc),
        ("tRRD", m.trrd, t.trrd),
        ("tFAW", m.tfaw, t.tfaw),
        ("tCCD", m.tccd, t.tccd),
        ("tRFC", m.trfc_pb, t.trfc),
        ("tWR", m.twr, t.twr),
    ] {
        assert!(have >= floor, "{name}: {have} < the grade's {floor}");
    }
    assert!(m.trefi_pb <= t.trefi, "the refresh interval is a maximum");
    assert_eq!(t.cl, m.trcd, "the bank model's CL is tRCD: 5 at -25E");
}

// ── the pins ─────────────────────────────────────────────────────────────────────────────

#[test]
fn the_pins_decode_by_the_truth_table() {
    use Ddr2Cmd::*;
    for c in [Nop, Active, Read, Write, Precharge, Refresh, LoadMode] {
        let (cs, ras, cas, we) = ddr2_pins(c);
        assert_eq!(command_of_pins(cs, ras, cas, we), Some(c), "{c:?}");
    }
    for p in 0..8u8 {
        assert_eq!(command_of_pins(1, p >> 2 & 1, p >> 1 & 1, p & 1), Some(Deselect), "CS# high is a deselect whatever else");
    }
    assert_eq!(command_of_pins(0, 1, 1, 0), None, "RAS# CAS# high, WE# low is reserved in DDR2");
    let decoded: Vec<_> = (0..8u8).filter_map(|p| command_of_pins(0, p >> 2 & 1, p >> 1 & 1, p & 1)).collect();
    assert_eq!(decoded.len(), 7, "seven of the eight CS#-low patterns are commands");
    assert_eq!(command_of_pins(2, 0, 0, 0), None, "a pin is 0 or 1");
}

// ── the rules, each one cycle early and exactly on time ───────────────────────────────────

#[test]
fn a_closed_page_transaction_at_its_minimums_is_legal() {
    let t = ddr800();
    let f = legal(&t, &[act(0, 0, 5), rd(6, 0, 8), pre(16, 0), act(22, 0, 6), wr(28, 0, 16), pre(43, 0)]);
    assert_eq!((f.activates, f.reads, f.writes, f.precharges, f.refreshes), (2, 1, 1, 2, 0));
    assert_eq!(f.first_cycle, 0);
    assert_eq!(f.last_cycle, 43);
    assert_eq!(f.cas.len(), 2);
    assert_eq!(f.cas[0], CasEvent { cycle: 6, write: false, bank: 0, row: 5, col: 8, data_at: 12 });
    assert_eq!(f.cas[1], CasEvent { cycle: 28, write: true, bank: 0, row: 6, col: 16, data_at: 33 });
    assert_eq!(f.data_cycles, 8, "two bursts of BL/2 clocks");
    assert_eq!(f.end, 44, "the last command, past the write data's end at 37");
}

#[test]
fn trcd() {
    let t = ddr800();
    assert_eq!(broken(&t, &[act(0, 0, 0), rd(5, 0, 0)]), ("tRCD", 1));
    assert_eq!(broken(&t, &[act(0, 0, 0), wr(5, 0, 0)]), ("tRCD", 1));
    legal(&t, &[act(0, 0, 0), rd(6, 0, 0)]);
    legal(&t, &[act(0, 0, 0), wr(6, 0, 0)]);
}

#[test]
fn tras() {
    let t = ddr800();
    assert_eq!(broken(&t, &[act(0, 0, 0), pre(15, 0)]), ("tRAS", 1));
    legal(&t, &[act(0, 0, 0), pre(16, 0)]);
    assert_eq!(broken(&t, &[act(0, 3, 0), pre_all(15)]), ("tRAS", 1), "a precharge-all closes every open bank");
    legal(&t, &[act(0, 3, 0), pre_all(16)]);
}

#[test]
fn read_to_precharge() {
    let t = ddr800();
    assert_eq!(broken(&t, &[act(0, 0, 0), rd(20, 0, 0), pre(24, 0)]), ("READ to PRECHARGE", 2));
    legal(&t, &[act(0, 0, 0), rd(20, 0, 0), pre(25, 0)]);
    assert_eq!(broken(&t, &[act(0, 0, 0), rd(20, 0, 0), pre_all(24)]), ("READ to PRECHARGE", 2));
}

#[test]
fn write_to_precharge() {
    let t = ddr800();
    assert_eq!(broken(&t, &[act(0, 0, 0), wr(20, 0, 0), pre(34, 0)]), ("WRITE to PRECHARGE", 2));
    legal(&t, &[act(0, 0, 0), wr(20, 0, 0), pre(35, 0)]);
    assert_eq!(broken(&t, &[act(0, 0, 0), wr(20, 0, 0), pre_all(34)]), ("WRITE to PRECHARGE", 2));
}

#[test]
fn trp_and_trpa() {
    let t = ddr800();
    assert_eq!(broken(&t, &[act(0, 0, 0), pre(20, 0), act(25, 0, 1)]), ("tRP", 2));
    legal(&t, &[act(0, 0, 0), pre(20, 0), act(26, 0, 1)]);
    assert_eq!(broken(&t, &[act(0, 0, 0), pre_all(20), act(26, 5, 1)]), ("tRPA", 2), "after a precharge-all every bank waits tRPA");
    legal(&t, &[act(0, 0, 0), pre_all(20), act(27, 5, 1)]);
    assert_eq!(broken(&t, &[act(0, 0, 0), pre(20, 0), refresh(25)]), ("tRP", 2));
    legal(&t, &[act(0, 0, 0), pre(20, 0), refresh(26)]);
    assert_eq!(broken(&t, &[act(0, 0, 0), pre_all(20), refresh(26)]), ("tRPA", 2));
    legal(&t, &[act(0, 0, 0), pre_all(20), refresh(27)]);
}

/// A PRECHARGE to a bank with no open row is a NOP (JESD79-2): legal, and it restarts no timer.
#[test]
fn a_precharge_to_an_idle_bank_is_a_nop() {
    let t = ddr800();
    legal(&t, &[pre(0, 2)]);
    legal(&t, &[act(0, 0, 0), pre(16, 0), pre(20, 0), act(22, 0, 1)]);
}

#[test]
fn trc() {
    let t = Ddr2Timing { trc: 30, ..ddr800() };
    assert_eq!(broken(&t, &[act(0, 0, 0), pre(16, 0), act(29, 0, 1)]), ("tRC", 2));
    legal(&t, &[act(0, 0, 0), pre(16, 0), act(30, 0, 1)]);
}

#[test]
fn trrd() {
    let t = ddr800();
    assert_eq!(broken(&t, &[act(0, 0, 0), act(3, 1, 0)]), ("tRRD", 1));
    legal(&t, &[act(0, 0, 0), act(4, 1, 0)]);
}

#[test]
fn tfaw() {
    let t = ddr800();
    let four = [act(0, 0, 0), act(4, 1, 0), act(8, 2, 0), act(12, 3, 0)];
    let mut early = four.to_vec();
    early.push(act(17, 4, 0));
    assert_eq!(broken(&t, &early), ("tFAW", 4));
    let mut on_time = four.to_vec();
    on_time.push(act(18, 4, 0));
    legal(&t, &on_time);
    // The window slides: a sixth ACT counts back four, to the second. Spaced so tRRD holds and tFAW alone binds.
    let slide = [act(0, 0, 0), act(10, 1, 0), act(14, 2, 0), act(18, 3, 0), act(22, 4, 0)];
    legal(&t, &slide);
    let mut early = slide.to_vec();
    early.push(act(27, 5, 0));
    assert_eq!(broken(&t, &early), ("tFAW", 5));
    let mut on_time = slide.to_vec();
    on_time.push(act(28, 5, 0));
    legal(&t, &on_time);
}

#[test]
fn cas_to_cas() {
    let t = ddr800();
    let open = [act(0, 0, 0), act(4, 1, 0)];
    assert_eq!(broken(&t, &[open[0], open[1], rd(10, 0, 0), rd(13, 1, 0)]), ("CAS to CAS", 3));
    legal(&t, &[open[0], open[1], rd(10, 0, 0), rd(14, 1, 0)]);
    assert_eq!(broken(&t, &[open[0], open[1], wr(10, 0, 0), wr(13, 1, 0)]), ("CAS to CAS", 3));
    legal(&t, &[open[0], open[1], wr(10, 0, 0), wr(14, 1, 0)]);
    assert_eq!(broken(&t, &[open[0], rd(10, 0, 0), rd(13, 0, 8)]), ("CAS to CAS", 2), "the same bank too");
}

#[test]
fn write_to_read() {
    let t = ddr800();
    for bank in [0, 1] {
        assert_eq!(broken(&t, &[act(0, 0, 0), act(4, 1, 0), wr(10, 0, 0), rd(21, bank, 0)]), ("WRITE to READ", 3), "bank {bank}");
        legal(&t, &[act(0, 0, 0), act(4, 1, 0), wr(10, 0, 0), rd(22, bank, 0)]);
    }
}

#[test]
fn read_to_write() {
    let t = ddr800();
    for bank in [0, 1] {
        assert_eq!(broken(&t, &[act(0, 0, 0), act(4, 1, 0), rd(10, 0, 0), wr(15, bank, 0)]), ("READ to WRITE", 3), "bank {bank}");
        legal(&t, &[act(0, 0, 0), act(4, 1, 0), rd(10, 0, 0), wr(16, bank, 0)]);
    }
}

#[test]
fn trfc() {
    let t = ddr800();
    assert_eq!(broken(&t, &[refresh(0), act(50, 0, 0)]), ("tRFC", 1));
    legal(&t, &[refresh(0), act(51, 0, 0)]);
    assert_eq!(broken(&t, &[refresh(0), refresh(50)]), ("tRFC", 1));
    legal(&t, &[refresh(0), refresh(51)]);
}

#[test]
fn bank_state() {
    let t = ddr800();
    assert_eq!(broken(&t, &[act(0, 0, 0), act(30, 0, 1)]), ("ACT to an open bank", 1));
    assert_eq!(broken(&t, &[rd(0, 0, 0)]), ("CAS to an idle bank", 0));
    assert_eq!(broken(&t, &[act(0, 0, 0), pre(16, 0), wr(30, 0, 0)]), ("CAS to an idle bank", 2));
    assert_eq!(broken(&t, &[act(0, 6, 0), refresh(40)]), ("REFRESH with an open bank", 1));
}

/// JESD79-2 lets a controller postpone up to eight REFRESH commands: by any time t the device has had at least
/// floor(t / tREFI) − 8 of them, counted from the trace's cycle 0, when it was last refreshed.
#[test]
fn the_refresh_interval() {
    let t = ddr800();
    let nine = 9 * t.trefi;
    assert_eq!(broken(&t, &[act(nine, 0, 0)]), ("refresh interval", 0));
    legal(&t, &[act(nine - 1, 0, 0)]);
    // A REF on the deadline's own clock pays the debt that clock is owed.
    legal(&t, &[refresh(nine)]);
    legal(&t, &[refresh(100), act(nine, 0, 0)]);
    assert_eq!(broken(&t, &[refresh(100), act(nine + t.trefi, 0, 0)]), ("refresh interval", 1));
    let mut on_schedule: Vec<BusCmd> = (1..40).map(|k| refresh(k * t.trefi)).collect();
    on_schedule.push(act(40 * t.trefi + t.trfc, 0, 0));
    legal(&t, &on_schedule);
    // The debt counts at the end of the trace too: a burst's data past the deadline still owes it.
    let last = nine - 1;
    assert!(check_bus(&t, &[act(last - t.trcd, 0, 0), rd(last, 0, 0)]).is_err(), "the read's data lands past nine intervals");
}

#[test]
fn tras_max() {
    let t = ddr800();
    assert_eq!(broken(&t, &[refresh(0), act(t.trfc, 0, 0), pre(t.trfc + t.tras_max + 1, 0)]), ("tRAS max", 2));
    legal(&t, &[refresh(0), act(t.trfc, 0, 0), pre(t.trfc + t.tras_max, 0)]);
}

#[test]
fn one_command_per_cycle() {
    let t = ddr800();
    assert_eq!(broken(&t, &[act(0, 0, 0), act(0, 1, 0)]), ("one command per cycle", 1));
    assert_eq!(broken(&t, &[act(10, 0, 0), act(5, 1, 0)]), ("one command per cycle", 1));
    legal(&t, &[cmd(0, Ddr2Cmd::Nop, 0, 0), act(0, 0, 0), cmd(0, Ddr2Cmd::Deselect, 0, 0), act(4, 1, 0)]);
}

fn rda(cycle: u64, ba: u8, col: u16) -> BusCmd {
    cmd(cycle, Ddr2Cmd::Read, ba, col | 1 << 10)
}
fn wra(cycle: u64, ba: u8, col: u16) -> BusCmd {
    cmd(cycle, Ddr2Cmd::Write, ba, col | 1 << 10)
}

/// A READ or WRITE with A10 high precharges its bank itself: from the later of the clock an explicit PRECHARGE could
/// have come and tRAS after the ACT, the bank closed at once and ready tRP after that precharge starts.
#[test]
fn auto_precharge() {
    let t = ddr800();
    // READ: precharge at max(20 + 5, 0 + 16) = 25, the bank ready at 31.
    assert_eq!(broken(&t, &[act(0, 0, 0), rda(20, 0, 0), act(30, 0, 1)]), ("tRP", 2));
    legal(&t, &[act(0, 0, 0), rda(20, 0, 0), act(31, 0, 1)]);
    // WRITE: precharge at max(20 + 15, 16) = 35 (WL + BL/2 + tWR), the bank ready at 41.
    assert_eq!(broken(&t, &[act(0, 0, 0), wra(20, 0, 0), act(40, 0, 1)]), ("tRP", 2));
    legal(&t, &[act(0, 0, 0), wra(20, 0, 0), act(41, 0, 1)]);
    // A read soon after the ACT waits for tRAS: precharge at max(6 + 5, 16) = 16, ready at 22.
    assert_eq!(broken(&t, &[act(0, 0, 0), rda(6, 0, 0), act(21, 0, 1)]), ("tRP", 2));
    legal(&t, &[act(0, 0, 0), rda(6, 0, 0), act(22, 0, 1)]);
    // The row is closed from the command on, and a REFRESH waits for its precharge like an ACT.
    assert_eq!(broken(&t, &[act(0, 0, 0), rda(6, 0, 0), rd(10, 0, 4)]), ("CAS to an idle bank", 2));
    assert_eq!(broken(&t, &[act(0, 0, 0), rda(20, 0, 0), refresh(30)]), ("tRP", 2));
    legal(&t, &[act(0, 0, 0), rda(20, 0, 0), refresh(31)]);
    // Another bank is untouched.
    legal(&t, &[act(0, 0, 0), act(4, 1, 0), rda(10, 0, 0), rd(14, 1, 0)]);
    let f = legal(&t, &[act(0, 0, 0), wra(6, 0, 8)]);
    assert_eq!((f.writes, f.precharges, f.cas[0].col), (1, 0, 8), "an auto-precharge is no PRECHARGE command; its column is A0-A9");
}

#[test]
fn what_the_checker_refuses_to_judge() {
    let t = ddr800();
    assert_eq!(broken(&t, &[cmd(0, Ddr2Cmd::LoadMode, 0, 0)]), ("mode register", 0));
    assert_eq!(broken(&t, &[act(0, 8, 0)]), ("address range", 0), "eight banks");
    assert_eq!(broken(&t, &[act(0, 0, 8192)]), ("address range", 0), "13 row bits");
    assert_eq!(broken(&t, &[act(0, 0, 0), rd(6, 0, 1 << 11)]), ("address range", 1), "10 column bits and A10");
    assert_eq!(broken(&t, &[pre(0, 9)]), ("address range", 0));
}

#[test]
fn a_violation_names_its_rule_and_place() {
    let t = ddr800();
    let v = check_bus(&t, &[act(0, 0, 0), rd(5, 0, 0)]).unwrap_err();
    assert_eq!((v.index, v.cycle, v.rule), (1, 5, "tRCD"));
    let shown = v.to_string();
    assert!(shown.contains("tRCD") && shown.contains("cycle 5") && shown.contains("command 1"), "{shown}");
}

#[test]
fn facts_count_what_reached_the_bus() {
    let t = ddr400();
    let trace = [
        refresh(0),
        act(26, 0, 7),
        act(28, 1, 9),
        wr(29, 0, 0),
        wr(33, 0, 8),
        wr(37, 1, 0),
        pre(46, 0),
        rd(49, 1, 0),
        pre(53, 1),
    ];
    let f = legal(&t, &trace);
    assert_eq!((f.activates, f.reads, f.writes, f.precharges, f.refreshes), (2, 1, 3, 2, 1));
    assert_eq!((f.first_cycle, f.last_cycle), (0, 53));
    assert_eq!(f.data_cycles, 4 * 4);
    assert_eq!(f.cas.iter().map(|c| (c.bank, c.row, c.col, c.write)).collect::<Vec<_>>(), vec![(0, 7, 0, true), (0, 7, 8, true), (1, 9, 0, true), (1, 9, 0, false)]);
    assert_eq!(f.cas.iter().map(|c| c.data_at).collect::<Vec<_>>(), vec![31, 35, 39, 52], "WL 2 for writes, CL 3 for reads");
    assert_eq!(f.end, 56, "the read's data: 52 + BL/2");
}

// ── an independent oracle ─────────────────────────────────────────────────────────────────

/// Every rule restated over PAIRS of commands — for each command, against every earlier one — instead of the
/// checker's running state. A command's bank context (which ACT opened the row it reads, which PRECHARGE closed
/// it) is found by scanning back, not tracked.
fn pairwise_first_violation(t: &Ddr2Timing, trace: &[BusCmd]) -> Option<usize> {
    use Ddr2Cmd::*;
    let is_all = |c: &BusCmd| c.cmd == Precharge && c.a & (1 << 10) != 0;
    let is_auto = |c: &BusCmd| matches!(c.cmd, Read | Write) && c.a & (1 << 10) != 0;
    let hits = |c: &BusCmd, bank: u8| match c.cmd {
        Refresh => true,
        Precharge => is_all(c) || c.ba == bank,
        _ => c.ba == bank,
    };
    // The open row of `bank` just before command j: the latest ACT to it with no closing command after (a
    // PRECHARGE, a REFRESH, or a column command with auto-precharge).
    let open_act = |j: usize, bank: u8| -> Option<usize> {
        for i in (0..j).rev() {
            let c = &trace[i];
            if !hits(c, bank) {
                continue;
            }
            match c.cmd {
                Active => return Some(i),
                Precharge | Refresh => return None,
                _ if is_auto(c) => return None,
                _ => {}
            }
        }
        None
    };
    // When the auto-precharge of column command i starts.
    let auto_start = |i: usize| {
        let c = &trace[i];
        let opened = trace[open_act(i, c.ba).expect("a legal column command has its row open")].cycle;
        let spacing = if c.cmd == Write { t.write_to_precharge() } else { t.read_to_precharge() };
        (c.cycle + spacing).max(opened + t.tras)
    };
    let owed = |at: u64, before: usize| {
        let refs = trace[..before].iter().filter(|c| c.cmd == Refresh).count() as u64;
        (at / t.trefi).saturating_sub(8) > refs
    };
    for j in 0..trace.len() {
        let c = trace[j];
        let gap = |i: usize| c.cycle - trace[i].cycle;
        if j > 0 && c.cycle <= trace[j - 1].cycle {
            return Some(j);
        }
        if owed(c.cycle, j + usize::from(c.cmd == Refresh)) {
            return Some(j);
        }
        for b in 0..8u8 {
            if let Some(i) = open_act(j, b) {
                if gap(i) > t.tras_max {
                    return Some(j);
                }
            }
        }
        let ok = match c.cmd {
            Active => {
                let fresh = open_act(j, c.ba).is_none();
                let after_close = (0..j).all(|i| {
                    let p = &trace[i];
                    // Every close of this bank since its last ACT: a PRECHARGE that closed an open row, or a REFRESH.
                    if is_auto(p) && p.ba == c.ba {
                        return c.cycle >= auto_start(i) + t.trp;
                    }
                    let closed = match p.cmd {
                        Precharge if is_all(p) => true,
                        Precharge => p.ba == c.ba && open_act(i, c.ba).is_some(),
                        Refresh => true,
                        _ => false,
                    };
                    let need = match p.cmd {
                        Refresh => t.trfc,
                        _ if is_all(p) => t.trpa,
                        _ => t.trp,
                    };
                    !closed || gap(i) >= need
                });
                let rc = (0..j).all(|i| trace[i].cmd != Active || trace[i].ba != c.ba || gap(i) >= t.trc);
                let rrd = (0..j).all(|i| trace[i].cmd != Active || gap(i) >= t.trrd);
                let acts: Vec<usize> = (0..j).filter(|&i| trace[i].cmd == Active).collect();
                let faw = acts.len() < 4 || gap(acts[acts.len() - 4]) >= t.tfaw;
                fresh && after_close && rc && rrd && faw
            }
            Read | Write => {
                let write = c.cmd == Write;
                let opened = open_act(j, c.ba);
                let rcd = opened.is_some_and(|i| gap(i) >= t.trcd);
                let spacing = (0..j).all(|i| match (trace[i].cmd, write) {
                    (Read, false) | (Write, true) => gap(i) >= t.cas_to_cas(),
                    (Write, false) => gap(i) >= t.write_to_read(),
                    (Read, true) => gap(i) >= t.read_to_write(),
                    _ => true,
                });
                rcd && spacing
            }
            Precharge => (0..8u8).filter(|&b| hits(&c, b)).all(|b| match open_act(j, b) {
                None => true,
                Some(i) => {
                    gap(i) >= t.tras
                        && (i..j).all(|k| match trace[k].cmd {
                            Read if trace[k].ba == b => gap(k) >= t.read_to_precharge(),
                            Write if trace[k].ba == b => gap(k) >= t.write_to_precharge(),
                            _ => true,
                        })
                }
            }),
            Refresh => (0..8u8).all(|b| open_act(j, b).is_none())
                && (0..j).all(|i| {
                    let p = &trace[i];
                    if is_auto(p) {
                        return c.cycle >= auto_start(i) + t.trp;
                    }
                    let need = match p.cmd {
                        Refresh => t.trfc,
                        Precharge if is_all(p) => t.trpa,
                        Precharge if open_act(i, p.ba).is_some() => t.trp,
                        _ => 0,
                    };
                    gap(i) >= need
                }),
            _ => true,
        };
        if !ok {
            return Some(j);
        }
    }
    // The trace's last busy cycle: its last command, or the last beat of its last burst.
    let Some(last) = trace.last() else { return None };
    let busy = trace
        .iter()
        .filter(|c| matches!(c.cmd, Read | Write))
        .map(|c| c.cycle + if c.cmd == Read { t.cl } else { t.wl } + t.bl / 2 - 1)
        .max()
        .unwrap_or(0)
        .max(last.cycle);
    if owed(busy, trace.len()) {
        return Some(trace.len());
    }
    for b in 0..8u8 {
        if let Some(i) = open_act(trace.len(), b) {
            if busy - trace[i].cycle > t.tras_max {
                return Some(trace.len());
            }
        }
    }
    None
}

/// Seeded random traces — each command chosen to suit its bank's state most of the time, its spacing drawn from
/// zero to past the longest rule — judged by the checker and by the pairwise oracle. They agree on every trace,
/// on legality and on the first offending command, and both verdicts occur in bulk.
#[test]
fn the_checker_agrees_with_a_pairwise_oracle_on_random_traces() {
    let mut s = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move |n: u64| {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s % n
    };
    for (tck, bl) in [(5000, 8), (2500, 8), (2500, 4), (3750, 4)] {
        let base = G25.at(tck, bl).unwrap();
        // A short refresh interval and row lifetime, so both limits bind inside traces of a few hundred cycles.
        let t = Ddr2Timing { trefi: base.trfc + 10, tras_max: 6 * base.trfc, ..base };
        let longest = [t.trfc, t.trc, t.tfaw, t.trpa, t.write_to_precharge(), t.write_to_read()].into_iter().max().unwrap();
        let (mut legal_n, mut illegal_n) = (0u32, 0u32);
        for _ in 0..6000 {
            let mut trace = Vec::new();
            let mut open = [false; 4];
            let mut cycle = next(3);
            for _ in 0..(3 + next(10)) {
                let bank = next(4) as u8;
                let (c, a) = if next(100) < 3 {
                    // A command that ignores its bank's state.
                    match next(5) {
                        0 => (Ddr2Cmd::Active, next(8) as u16),
                        1 => (Ddr2Cmd::Read, (next(4) * bl) as u16),
                        2 => (Ddr2Cmd::Write, (next(4) * bl) as u16),
                        3 => (Ddr2Cmd::Precharge, 0),
                        _ => (Ddr2Cmd::Refresh, 0),
                    }
                } else if open.iter().all(|o| !o) && next(5) == 0 {
                    (Ddr2Cmd::Refresh, 0)
                } else if !open[bank as usize] {
                    (Ddr2Cmd::Active, next(8) as u16)
                } else {
                    let auto = if next(8) == 0 { 1 << 10 } else { 0 };
                    match next(100) {
                        0..=4 => (Ddr2Cmd::Precharge, 1 << 10),
                        5..=29 => (Ddr2Cmd::Precharge, 0),
                        30..=69 => (Ddr2Cmd::Read, (next(4) * bl) as u16 | auto),
                        _ => (Ddr2Cmd::Write, (next(4) * bl) as u16 | auto),
                    }
                };
                match c {
                    Ddr2Cmd::Active => open[bank as usize] = true,
                    Ddr2Cmd::Precharge if a != 0 => open = [false; 4],
                    Ddr2Cmd::Precharge => open[bank as usize] = false,
                    Ddr2Cmd::Refresh => open = [false; 4],
                    Ddr2Cmd::Read | Ddr2Cmd::Write if a & (1 << 10) != 0 => open[bank as usize] = false,
                    _ => {}
                }
                trace.push(BusCmd { cycle, cmd: c, ba: bank, a });
                cycle += match next(60) {
                    0 => 0,
                    1..=18 => 1 + next(longest),
                    _ => longest + next(8),
                };
            }
            let oracle = pairwise_first_violation(&t, &trace);
            let checker = check_bus(&t, &trace);
            match (&checker, oracle) {
                (Ok(_), None) => legal_n += 1,
                (Err(v), Some(j)) => {
                    assert_eq!(v.index, j, "first violation differs ({v}) on {trace:?}");
                    illegal_n += 1;
                }
                _ => panic!("checker {checker:?} vs oracle {oracle:?} on {trace:?}"),
            }
        }
        assert!(legal_n > 500 && illegal_n > 500, "tCK {tck} BL {bl}: {legal_n} legal, {illegal_n} illegal — both verdicts in bulk");
    }
}
