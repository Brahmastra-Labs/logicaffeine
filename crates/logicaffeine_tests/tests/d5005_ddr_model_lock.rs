//! The D5005's DDR4-2400 channels: the timing the card's controllers are configured with, the JESD79-4 rules a
//! command trace is held to, and the in-order controller model the engine's streams are priced on.
//!
//! `codegen_sva::ddr4_jedec` clocks the controller's configuration, checks traces against the rules and schedules
//! line streams; `decode_engine::ddr` prices a decode pass's streams with it into
//! `benchmarks/silicon/d5005/ddr4_efficiency.json`. These tests hold the grade to the shell's own configuration file,
//! the clocked rules to JESD79-4's arithmetic and to Ramulator 2's DDR4-2400 table, the checker to every rule at its
//! boundary (one clock inside it passes, one clock past it is refused by that rule at that command), the controller
//! to legal traces whose turnarounds and refresh stalls cost exactly what the rules charge, and the ledger to the
//! model.

#![cfg(feature = "codegen")]

mod common;
#[path = "common/ddr4_log.rs"]
mod ddr4_log;

use std::path::{Path, PathBuf};

use logicaffeine_compile::codegen_sva::ddr4_jedec::{
    check_bus, locate, measure, schedule, stream_lines, verilog_model, Ddr4Cmd, Ddr4Facts, Ddr4Grade, Ddr4Timed, Ddr4Timing,
    Location, Scheduler, BL, LOOKAHEAD, REFRESH_POSTPONE_MAX, REFRESH_PULL_IN_MAX,
};
use logicaffeine_compile::codegen_sva::dram::StreamDesc;
use logicaffeine_compile::spatial::d5005::{Beat, DDR_CHANNELS, DDR_CHANNEL_BYTES};
use logicaffeine_compile::spatial::decode::GgufHeader;
use logicaffeine_compile::spatial::decode_engine::ddr::{ddr_ledger, stream_bytes, CACHE_POSITIONS, HEADER, SEQUENCES, STATE_CHUNKS};
use ddr4_log::{read_log, replay, ChannelLog};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn t() -> Ddr4Timing {
    Ddr4Grade::D5005_EMIF.at()
}

fn act(at: u64, group: u32, bank: u32, row: u64) -> Ddr4Timed {
    Ddr4Timed { at, cmd: Ddr4Cmd::Act { group, bank, row } }
}
fn rd(at: u64, group: u32, bank: u32, col: u64) -> Ddr4Timed {
    Ddr4Timed { at, cmd: Ddr4Cmd::Rd { group, bank, col } }
}
fn wr(at: u64, group: u32, bank: u32, col: u64) -> Ddr4Timed {
    Ddr4Timed { at, cmd: Ddr4Cmd::Wr { group, bank, col } }
}
fn pre(at: u64, group: u32, bank: u32) -> Ddr4Timed {
    Ddr4Timed { at, cmd: Ddr4Cmd::Pre { group, bank } }
}
fn prea(at: u64) -> Ddr4Timed {
    Ddr4Timed { at, cmd: Ddr4Cmd::PreAll }
}
fn refresh(at: u64) -> Ddr4Timed {
    Ddr4Timed { at, cmd: Ddr4Cmd::Ref }
}

/// `legal` passes, and `broken` is refused by `rule` at command `index`.
fn boundary(t: &Ddr4Timing, legal: &[Ddr4Timed], broken: &[Ddr4Timed], rule: &str, index: usize) {
    if let Err(v) = check_bus(t, legal) {
        panic!("the legal trace {legal:?} is refused: {v}");
    }
    match check_bus(t, broken) {
        Ok(_) => panic!("{rule}: the broken trace {broken:?} passes"),
        Err(v) => {
            assert_eq!(v.rule, rule, "{broken:?}: {v}");
            assert_eq!(v.index, index, "{broken:?}: {v}");
        }
    }
}

// ── The grade ───────────────────────────────────────────────────────────────────────────────────────

/// A parameter's value in the shell's IP-XACT configuration.
fn ip_value<'a>(ip: &'a str, name: &str) -> &'a str {
    let key = format!("parameterId=\"{name}\"");
    let at = ip.find(&key).unwrap_or_else(|| panic!("the configuration names no {name}"));
    let rest = &ip[at..];
    let open = rest.find("<ipxact:value>").expect("a value") + "<ipxact:value>".len();
    let close = rest[open..].find("</ipxact:value>").expect("the value closes");
    &rest[open..open + close]
}

fn ps(ns: &str) -> u64 {
    (ns.parse::<f64>().unwrap() * 1000.0).round() as u64
}

/// Every field of the grade is the shell's configuration of its controller, and the configuration is the one the
/// rules are written for: burst length eight, no additive latency, a one-clock write preamble, fixed 1x refresh, one
/// rank. The controller's Avalon side carries one burst a word over the 27 address bits the channel's lines take.
#[test]
fn the_d5005_grade_is_its_controllers_configuration() {
    if !common::require_tool("d5005-pim") {
        return;
    }
    let build = repo().join("tools/d5005/pr-template/hw/lib/build");
    let ip = std::fs::read_to_string(build.join("ipss/mem/ip/emif_ddr4_no_ecc.ip")).expect("the EMIF configuration");
    let g = Ddr4Grade::D5005_EMIF;
    let int = |name: &str| ip_value(&ip, name).parse::<u64>().unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(1u32 << int("MEM_DDR4_BANK_GROUP_WIDTH"), g.bank_groups);
    assert_eq!(1u32 << int("MEM_DDR4_BANK_ADDR_WIDTH"), g.banks_per_group);
    assert_eq!(int("MEM_DDR4_ROW_ADDR_WIDTH") as u32, g.row_bits);
    assert_eq!(int("MEM_DDR4_COL_ADDR_WIDTH") as u32, g.col_bits);
    assert_eq!(int("MEM_DDR4_DQ_WIDTH") as u32, g.dq_bits);
    assert_eq!(ip_value(&ip, "PHY_DDR4_RATE_ENUM"), "RATE_QUARTER");
    assert_eq!(g.rate, 4);
    assert_eq!(ip_value(&ip, "PHY_DDR4_MEM_CLK_FREQ_MHZ").parse::<f64>().unwrap(), g.mem_mhz as f64);
    assert_eq!(ip_value(&ip, "MEM_DDR4_SPEEDBIN_ENUM"), "DDR4_SPEEDBIN_2400");
    assert_eq!(int("MEM_DDR4_TCL"), g.cl);
    assert_eq!(int("MEM_DDR4_WTCL"), g.cwl);
    assert_eq!(ip_value(&ip, "MEM_DDR4_AC_PARITY_LATENCY"), format!("DDR4_AC_PARITY_LATENCY_{}", g.pl));
    for (name, value) in [
        ("MEM_DDR4_TRCD_NS", g.trcd_ps),
        ("MEM_DDR4_TRP_NS", g.trp_ps),
        ("MEM_DDR4_TRAS_NS", g.tras_ps),
        ("MEM_DDR4_TFAW_NS", g.tfaw_ps),
        ("MEM_DDR4_TWR_NS", g.twr_ps),
        ("MEM_DDR4_TRFC_NS", g.trfc_ps),
    ] {
        assert_eq!(ps(ip_value(&ip, name)), value, "{name}");
    }
    assert_eq!(ps(ip_value(&ip, "MEM_DDR4_TREFI_US")) * 1000, g.trefi_ps);
    for (name, value) in [
        ("MEM_DDR4_TRRD_S_CYC", g.trrd_s_ck),
        ("MEM_DDR4_TRRD_L_CYC", g.trrd_l_ck),
        ("MEM_DDR4_TCCD_S_CYC", g.tccd_s_ck),
        ("MEM_DDR4_TCCD_L_CYC", g.tccd_l_ck),
        ("MEM_DDR4_TWTR_S_CYC", g.twtr_s_ck),
        ("MEM_DDR4_TWTR_L_CYC", g.twtr_l_ck),
    ] {
        assert_eq!(int(name), value, "{name}");
    }
    assert_eq!(g.trc_ps, None, "the configuration names no tRC");
    assert_eq!(ip_value(&ip, "CTRL_DDR4_ADDR_ORDER_ENUM"), "DDR4_CTRL_ADDR_ORDER_CS_R_B_C_BG");
    assert_eq!(ip_value(&ip, "MEM_DDR4_BL_ENUM"), "DDR4_BL_BL8");
    assert_eq!(ip_value(&ip, "MEM_DDR4_ATCL_ENUM"), "DDR4_ATCL_DISABLED");
    assert_eq!(int("MEM_DDR4_WRITE_PREAMBLE"), 1);
    assert_eq!(ip_value(&ip, "MEM_DDR4_FINE_GRANULARITY_REFRESH"), "DDR4_FINE_REFRESH_FIXED_1X");
    assert_eq!(int("MEM_DDR4_RANKS_PER_DIMM"), 1);

    let pkg = std::fs::read_to_string(build.join("ipss/mem/includes/ofs_fim_emif_cfg_pkg.sv")).expect("the EMIF package");
    let local = |name: &str| {
        let at = pkg.find(&format!("localparam {name} = ")).unwrap_or_else(|| panic!("the package names no {name}"));
        let rest = &pkg[at + format!("localparam {name} = ").len()..];
        rest[..rest.find(';').unwrap()].trim().to_string()
    };
    let t = g.at();
    assert_eq!(1u64 << local("AVMM_ADDR_WIDTH").parse::<u32>().unwrap(), t.lines(), "one Avalon word a line");
    let data: u64 = local("AVMM_DATA_BASE_WIDTH").parse::<u64>().unwrap() + local("AVMM_ECC_WIDTH").parse::<u64>().unwrap();
    assert_eq!(local("AVMM_DATA_WIDTH"), "AVMM_DATA_BASE_WIDTH + AVMM_ECC_WIDTH");
    assert_eq!(data, t.burst_bits(), "one Avalon word a burst");
}

/// The grade clocked at 1200 MHz: every nanosecond minimum rounded up to whole clocks, the latencies with the parity
/// latency added, the command-to-command delays JESD79-4 composes from them.
#[test]
fn the_grade_clocks_to_whole_cycles() {
    let t = t();
    assert_eq!((t.rl, t.wl), (25, 20));
    assert_eq!((t.trcd, t.trp, t.tras, t.trc), (17, 17, 39, 56));
    assert_eq!(t.trc, t.tras + t.trp, "tRC is tRAS + tRP where the configuration names none");
    assert_eq!((t.trrd_s, t.trrd_l, t.tfaw), (4, 6, 26));
    assert_eq!((t.tccd_s, t.tccd_l), (4, 6));
    assert_eq!((t.twr, t.twtr_s, t.twtr_l, t.trtp), (18, 3, 9, 9));
    assert_eq!((t.trfc, t.trefi), (420, 9360));
    assert_eq!(t.read_to_write(), 25 + 4 + 2 - 20);
    assert_eq!((t.write_to_read(true), t.write_to_read(false)), (20 + 4 + 9, 20 + 4 + 3));
    assert_eq!(t.write_to_precharge(), 20 + 4 + 18);
    assert_eq!(t.burst_ck(), BL / 2);
    assert_eq!(t.lines(), 1 << 27);
    assert_eq!(t.burst_bits(), 576);
    assert_eq!(t.lines() * 64, DDR_CHANNEL_BYTES as u64, "the pricer's channel holds the lines' 64 data bytes each");
    assert_eq!((t.controller_ck(0), t.controller_ck(1), t.controller_ck(4), t.controller_ck(5)), (0, 1, 1, 2));
    assert_eq!(t.peak_bytes_per_s(64), 19.2e9);
    assert_eq!(t.peak_bytes_per_s(72), 21.6e9);
    assert_eq!(t.peak_bytes_per_s(64) * DDR_CHANNELS as f64, 76.8e9, "the card's four channels");
}

/// The clocked minimums are Ramulator 2's DDR4-2400U row (`src/dram/impl/DDR4.cpp`: nRCD 17, nRP 17, nRAS 39,
/// nRC 56, nWR 18, nRTP 9, nCCDS 4, nCCDL 6, nWTRS 3, nWTRL 9), an independent reading of the same JESD79-4 bin; and
/// its READ to WRITE delay (nCL + nBL + 2 − nCWL) and WRITE to READ delays (nCWL + nBL + nWTRS/L) are this model's.
#[test]
fn the_clocked_minimums_agree_with_ramulator() {
    let t = t();
    assert_eq!([t.trcd, t.trp, t.tras, t.trc, t.twr, t.trtp, t.tccd_s, t.tccd_l, t.twtr_s, t.twtr_l], [17, 17, 39, 56, 18, 9, 4, 6, 3, 9]);
    let (n_cl, n_cwl, n_bl) = (t.rl, t.wl, BL / 2);
    assert_eq!(t.read_to_write(), n_cl + n_bl + 2 - n_cwl);
    assert_eq!(t.write_to_read(false), n_cwl + n_bl + t.twtr_s);
    assert_eq!(t.write_to_read(true), n_cwl + n_bl + t.twtr_l);
    assert_eq!(t.write_to_precharge(), n_cwl + n_bl + t.twr);
}

/// A column spacing under a burst's four clocks would put two bursts on the bus at once: the clocking never yields
/// one, whatever the configuration states.
#[test]
fn the_clocking_never_spaces_columns_under_a_burst() {
    let g = Ddr4Grade { tccd_s_ck: 2, tccd_l_ck: 3, ..Ddr4Grade::D5005_EMIF };
    assert_eq!((g.at().tccd_s, g.at().tccd_l), (4, 4));
    let g = Ddr4Grade { trtp_ps: 1_000, ..Ddr4Grade::D5005_EMIF };
    assert_eq!(g.at().trtp, 4, "tRTP is at least four clocks");
}

// ── The address ─────────────────────────────────────────────────────────────────────────────────────

fn line_of(t: &Ddr4Timing, l: Location) -> u64 {
    let g = t.grade;
    let (gb, cb, bb) = (g.bank_groups.trailing_zeros(), g.col_bits - BL.trailing_zeros(), g.banks_per_group.trailing_zeros());
    (l.row << (gb + cb + bb)) | (u64::from(l.bank) << (gb + cb)) | ((l.col / BL) << gb) | u64::from(l.group)
}

/// Consecutive lines take the four bank groups in turn, then the 128 bursts of a row, then the banks, then the rows;
/// past the channel's last line the address wraps.
#[test]
fn consecutive_lines_take_the_bank_groups_in_turn() {
    let t = t();
    let at = |line: u64| locate(&t, line);
    let loc = |group, bank, row, col| Location { group, bank, row, col };
    for g in 0..4 {
        assert_eq!(at(g), loc(g as u32, 0, 0, 0));
    }
    assert_eq!(at(4), loc(0, 0, 0, 8));
    assert_eq!(at(511), loc(3, 0, 0, 1016));
    assert_eq!(at(512), loc(0, 1, 0, 0));
    assert_eq!(at(2047), loc(3, 3, 0, 1016));
    assert_eq!(at(2048), loc(0, 0, 1, 0));
    assert_eq!(at((1 << 27) - 1), loc(3, 3, 65535, 1016));
    assert_eq!(at(1 << 27), at(0));
}

/// Every line has its own place: the place's fields put back in the controller's order are the line, over every low
/// line and the first lines of every row.
#[test]
fn locate_is_one_to_one() {
    let t = t();
    for line in (0..1u64 << 20).chain((0..1u64 << 16).flat_map(|row| [row << 11, (row << 11) | 2047])) {
        let l = locate(&t, line);
        assert!(l.group < 4 && l.bank < 4 && l.row < 1 << 16 && l.col < 1024 && l.col % BL == 0, "{line}: {l:?}");
        assert_eq!(line_of(&t, l), line, "{l:?}");
    }
}

/// A stream's lines: every line an element covers, one entry where consecutive elements share a line.
#[test]
fn a_stream_touches_every_line_its_elements_cover_once() {
    let lines = |s: StreamDesc, lb: u64| stream_lines(&s, lb).collect::<Vec<_>>();
    assert_eq!(lines(StreamDesc { base: 60, elem_bytes: 8, stride_bytes: 8, count: 3, write: false }, 64), [(0, false), (1, false)]);
    assert_eq!(lines(StreamDesc { base: 0, elem_bytes: 4, stride_bytes: 4096, count: 3, write: true }, 64), [(0, true), (64, true), (128, true)]);
    assert_eq!(lines(StreamDesc { base: 70, elem_bytes: 150, stride_bytes: 150, count: 1, write: false }, 72), [(0, false), (1, false), (2, false), (3, false)]);
    assert_eq!(lines(StreamDesc { base: 128, elem_bytes: 64, stride_bytes: 64, count: 2, write: false }, 64), [(2, false), (3, false)]);
}

// ── Every rule at its boundary ───────────────────────────────────────────────────────────────────────

#[test]
fn commands_come_one_a_clock() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), act(4, 1, 0, 0)], &[act(0, 0, 0, 0), act(0, 1, 0, 0)], "one command per clock", 1);
    boundary(&t, &[act(4, 0, 0, 0), act(8, 1, 0, 0)], &[act(4, 0, 0, 0), act(3, 1, 0, 0)], "one command per clock", 1);
}

#[test]
fn every_address_is_in_the_channel() {
    let t = t();
    boundary(&t, &[act(0, 3, 0, 0)], &[act(0, 4, 0, 0)], "address range", 0);
    boundary(&t, &[act(0, 0, 3, 0)], &[act(0, 0, 4, 0)], "address range", 0);
    boundary(&t, &[act(0, 0, 0, 65535)], &[act(0, 0, 0, 65536)], "address range", 0);
    boundary(&t, &[act(0, 0, 0, 0), rd(17, 0, 0, 1016)], &[act(0, 0, 0, 0), rd(17, 0, 0, 1024)], "address range", 1);
    boundary(&t, &[act(0, 0, 0, 0), wr(17, 0, 0, 8)], &[act(0, 0, 0, 0), wr(17, 0, 0, 12)], "address range", 1);
    boundary(&t, &[pre(0, 0, 3)], &[pre(0, 0, 4)], "address range", 0);
}

#[test]
fn an_act_opens_a_closed_bank() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), pre(39, 0, 0), act(56, 0, 0, 1)], &[act(0, 0, 0, 0), act(56, 0, 0, 1)], "ACT to an open bank", 1);
}

#[test]
fn a_column_command_needs_an_open_row() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), rd(17, 0, 0, 0)], &[rd(17, 0, 0, 0)], "column command to an idle bank", 0);
    boundary(&t, &[act(0, 0, 0, 0), pre(39, 0, 0)], &[act(0, 0, 0, 0), pre(39, 0, 0), wr(56, 0, 0, 0)], "column command to an idle bank", 2);
}

#[test]
fn a_refresh_needs_every_bank_closed() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), pre(39, 0, 0), refresh(56)], &[act(0, 0, 0, 0), refresh(56)], "REFRESH with an open bank", 1);
    boundary(
        &t,
        &[act(0, 0, 0, 0), act(4, 1, 0, 0), prea(43), refresh(60)],
        &[act(0, 0, 0, 0), act(4, 1, 0, 0), pre(43, 0, 0), refresh(60)],
        "REFRESH with an open bank",
        3,
    );
}

#[test]
fn trp_holds_from_precharge_to_activate() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), pre(50, 0, 0), act(67, 0, 0, 1)], &[act(0, 0, 0, 0), pre(50, 0, 0), act(66, 0, 0, 1)], "tRP", 2);
    boundary(&t, &[act(0, 0, 0, 0), prea(50), act(67, 0, 0, 1)], &[act(0, 0, 0, 0), prea(50), act(66, 0, 0, 1)], "tRP", 2);
}

#[test]
fn trp_holds_from_precharge_to_refresh() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), pre(39, 0, 0), refresh(56)], &[act(0, 0, 0, 0), pre(39, 0, 0), refresh(55)], "tRP", 2);
    boundary(&t, &[act(0, 0, 0, 0), prea(39), refresh(56)], &[act(0, 0, 0, 0), prea(39), refresh(55)], "tRP", 2);
}

/// A PRECHARGE to a bank with no open row is legal, and the bank's tRP still runs from it.
#[test]
fn a_precharge_to_an_idle_bank_restarts_its_trp() {
    let t = t();
    boundary(&t, &[pre(0, 0, 0), act(17, 0, 0, 0)], &[pre(0, 0, 0), act(16, 0, 0, 0)], "tRP", 1);
}

/// tRC binds where a configuration states one longer than tRAS + tRP.
#[test]
fn trc_holds_from_activate_to_activate_in_a_bank() {
    let t = Ddr4Grade { trc_ps: Some(60_000), ..Ddr4Grade::D5005_EMIF }.at();
    assert_eq!(t.trc, 72);
    boundary(&t, &[act(0, 0, 0, 0), pre(39, 0, 0), act(72, 0, 0, 1)], &[act(0, 0, 0, 0), pre(39, 0, 0), act(71, 0, 0, 1)], "tRC", 2);
}

#[test]
fn trrd_s_spaces_acts_across_bank_groups() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), act(4, 1, 0, 0)], &[act(0, 0, 0, 0), act(3, 1, 0, 0)], "tRRD_S", 1);
}

#[test]
fn trrd_l_spaces_acts_within_a_bank_group() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), act(6, 0, 1, 0)], &[act(0, 0, 0, 0), act(5, 0, 1, 0)], "tRRD_L", 1);
    boundary(
        &t,
        &[act(0, 0, 0, 0), act(4, 1, 0, 0), act(8, 0, 1, 0)],
        &[act(0, 0, 0, 0), act(4, 1, 0, 0), act(5, 0, 1, 0)],
        "tRRD_L",
        2,
    );
}

/// No fifth ACT within tFAW of the fourth before it, the window sliding with every ACT.
#[test]
fn tfaw_admits_four_acts_in_its_window() {
    let t = t();
    let four = [act(0, 0, 0, 0), act(10, 1, 0, 0), act(14, 2, 0, 0), act(18, 3, 0, 0)];
    let with = |extra: &[Ddr4Timed]| [&four[..], extra].concat();
    boundary(&t, &with(&[act(26, 0, 1, 0)]), &with(&[act(25, 0, 1, 0)]), "tFAW", 4);
    boundary(&t, &with(&[act(26, 0, 1, 0), act(36, 1, 1, 0)]), &with(&[act(26, 0, 1, 0), act(35, 1, 1, 0)]), "tFAW", 5);
}

#[test]
fn trcd_holds_from_activate_to_column() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), rd(17, 0, 0, 0)], &[act(0, 0, 0, 0), rd(16, 0, 0, 0)], "tRCD", 1);
    boundary(&t, &[act(0, 0, 0, 0), wr(17, 0, 0, 0)], &[act(0, 0, 0, 0), wr(16, 0, 0, 0)], "tRCD", 1);
}

#[test]
fn tccd_s_spaces_columns_across_bank_groups() {
    let t = t();
    let two = [act(0, 0, 0, 0), act(4, 1, 0, 0)];
    let with = |extra: &[Ddr4Timed]| [&two[..], extra].concat();
    boundary(&t, &with(&[rd(21, 0, 0, 0), rd(25, 1, 0, 0)]), &with(&[rd(21, 0, 0, 0), rd(24, 1, 0, 0)]), "tCCD_S", 3);
    boundary(&t, &with(&[wr(21, 0, 0, 0), wr(25, 1, 0, 0)]), &with(&[wr(21, 0, 0, 0), wr(24, 1, 0, 0)]), "tCCD_S", 3);
}

#[test]
fn tccd_l_spaces_columns_within_a_bank_group() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), rd(17, 0, 0, 0), rd(23, 0, 0, 8)], &[act(0, 0, 0, 0), rd(17, 0, 0, 0), rd(22, 0, 0, 8)], "tCCD_L", 2);
    boundary(
        &t,
        &[act(0, 0, 0, 0), act(6, 0, 1, 0), wr(23, 0, 0, 0), wr(29, 0, 1, 0)],
        &[act(0, 0, 0, 0), act(6, 0, 1, 0), wr(23, 0, 0, 0), wr(28, 0, 1, 0)],
        "tCCD_L",
        3,
    );
}

#[test]
fn twtr_s_turns_a_write_to_a_read_across_bank_groups() {
    let t = t();
    let two = [act(0, 0, 0, 0), act(4, 1, 0, 0), wr(21, 0, 0, 0)];
    let with = |extra: &[Ddr4Timed]| [&two[..], extra].concat();
    boundary(&t, &with(&[rd(48, 1, 0, 0)]), &with(&[rd(47, 1, 0, 0)]), "tWTR_S", 3);
}

#[test]
fn twtr_l_turns_a_write_to_a_read_within_a_bank_group() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), wr(17, 0, 0, 0), rd(50, 0, 0, 8)], &[act(0, 0, 0, 0), wr(17, 0, 0, 0), rd(49, 0, 0, 8)], "tWTR_L", 2);
}

#[test]
fn a_read_turns_to_a_write_after_rl_plus_bl2_minus_wl_plus_2() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), rd(17, 0, 0, 0), wr(28, 0, 0, 8)], &[act(0, 0, 0, 0), rd(17, 0, 0, 0), wr(27, 0, 0, 8)], "READ to WRITE", 2);
    boundary(
        &t,
        &[act(0, 0, 0, 0), act(4, 1, 0, 0), rd(21, 0, 0, 0), wr(32, 1, 0, 0)],
        &[act(0, 0, 0, 0), act(4, 1, 0, 0), rd(21, 0, 0, 0), wr(31, 1, 0, 0)],
        "READ to WRITE",
        3,
    );
}

/// Two bursts never share the data bus: under a hand-made timing whose column spacing is shorter than a burst, the
/// data bus is the rule that refuses them.
#[test]
fn the_data_bus_carries_one_burst_at_a_time() {
    let t = Ddr4Timing { tccd_s: 2, tccd_l: 2, ..t() };
    let two = [act(0, 0, 0, 0), act(4, 1, 0, 0), rd(21, 0, 0, 0)];
    let with = |extra: &[Ddr4Timed]| [&two[..], extra].concat();
    boundary(&t, &with(&[rd(25, 1, 0, 0)]), &with(&[rd(24, 1, 0, 0)]), "data bus", 3);
}

#[test]
fn tras_holds_a_row_open() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), pre(39, 0, 0)], &[act(0, 0, 0, 0), pre(38, 0, 0)], "tRAS", 1);
    boundary(&t, &[act(0, 0, 0, 0), prea(39)], &[act(0, 0, 0, 0), prea(38)], "tRAS", 1);
}

#[test]
fn trtp_holds_from_read_to_precharge() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), rd(40, 0, 0, 0), pre(49, 0, 0)], &[act(0, 0, 0, 0), rd(40, 0, 0, 0), pre(48, 0, 0)], "tRTP", 2);
}

#[test]
fn twr_holds_from_write_to_precharge() {
    let t = t();
    boundary(&t, &[act(0, 0, 0, 0), wr(40, 0, 0, 0), pre(82, 0, 0)], &[act(0, 0, 0, 0), wr(40, 0, 0, 0), pre(81, 0, 0)], "tWR", 2);
    boundary(&t, &[act(0, 0, 0, 0), wr(40, 0, 0, 0), prea(82)], &[act(0, 0, 0, 0), wr(40, 0, 0, 0), prea(81)], "tWR", 2);
}

#[test]
fn trfc_holds_the_channel_after_a_refresh() {
    let t = t();
    boundary(&t, &[refresh(0), act(420, 0, 0, 0)], &[refresh(0), act(419, 0, 0, 0)], "tRFC", 1);
    boundary(&t, &[refresh(0), pre(420, 0, 0)], &[refresh(0), pre(419, 0, 0)], "tRFC", 1);
    boundary(&t, &[refresh(0), refresh(420)], &[refresh(0), refresh(419)], "tRFC", 1);
}

/// At most eight REFRESH may be owed: nine intervals passed with none paid is refused at the command that sees it, a
/// REFRESH on that clock pays the ninth.
#[test]
fn at_most_eight_refreshes_are_owed() {
    let t = t();
    let ninth = 9 * t.trefi;
    boundary(&t, &[pre(ninth - 2, 0, 0)], &[pre(ninth, 0, 0)], "refresh postponed", 0);
    check_bus(&t, &[refresh(ninth)]).expect("a REFRESH on the ninth interval's clock pays it");
    assert_eq!(REFRESH_POSTPONE_MAX, 8);
}

/// The account holds at the trace's end too: a trace whose last clock passes the ninth interval unpaid is refused
/// there.
#[test]
fn the_trace_end_owes_no_more_than_eight() {
    let t = t();
    let ninth = 9 * t.trefi;
    boundary(&t, &[pre(ninth - 2, 0, 0)], &[pre(ninth - 1, 0, 0)], "refresh postponed", 1);
}

/// REFRESH paid ahead count at most eight: ten paid at the start buy no more time than eight, so seventeen intervals
/// later the account owes nine. Counted whole, the ten would hold it at seven owed.
#[test]
fn at_most_eight_refreshes_are_paid_ahead() {
    let t = t();
    let early: Vec<Ddr4Timed> = (0..10).map(|k| refresh(k * t.trfc)).collect();
    let with = |at: u64| [&early[..], &[pre(at, 0, 0)]].concat();
    let limit = (REFRESH_PULL_IN_MAX + REFRESH_POSTPONE_MAX + 1) as u64 * t.trefi;
    assert_eq!((REFRESH_PULL_IN_MAX, limit), (8, 17 * t.trefi));
    boundary(&t, &with(limit - 2), &with(limit), "refresh postponed", 10);
}

// ── The controller ───────────────────────────────────────────────────────────────────────────────────

fn run(base: u64, bytes: u64, write: bool) -> StreamDesc {
    StreamDesc { base, elem_bytes: bytes, stride_bytes: bytes, count: 1, write }
}

/// Read and write back `chunk` lines at a time over `lines` lines from line 0.
fn read_modify_write(lines: u64, chunk: u64, lb: u64) -> Vec<StreamDesc> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < lines {
        let n = chunk.min(lines - at);
        out.push(run(at * lb, n * lb, false));
        out.push(run(at * lb, n * lb, true));
        at += n;
    }
    out
}

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// The streams the controller is held legal on: long reads and writes across refreshes, read-modify-writes of every
/// chunk shape, one line a row in one bank, alternating directions, and seeded random lines anywhere in the channel.
fn patterns() -> Vec<(String, Vec<StreamDesc>)> {
    let mut out = vec![
        ("a read of 20000 lines".to_string(), vec![run(0, 20_000 * 64, false)]),
        ("a write of 20000 lines".to_string(), vec![run(0, 20_000 * 64, true)]),
        ("one line a row in one bank".to_string(), vec![StreamDesc { base: 0, elem_bytes: 64, stride_bytes: 1 << 20, count: 2_000, write: false }]),
        ("alternating directions".to_string(), (0..4_000).map(|i| run(i * 64, 64, i % 2 == 1)).collect()),
        ("72-byte lines across row ends".to_string(), vec![run(500 * 72, 9_000 * 72, false), run(500 * 72, 9_000 * 72, true)]),
    ];
    for chunk in [1, 3, 8, 32, 512] {
        out.push((format!("read-modify-write {chunk} at a time"), read_modify_write(8_192, chunk, 64)));
    }
    let mut s = 0x9E37_79B9_7F4A_7C15;
    let random: Vec<StreamDesc> = (0..5_000).map(|_| {
        let v = xorshift(&mut s);
        run((v >> 8) % (1 << 27) * 64, 64, v & 1 == 1)
    }).collect();
    out.push(("random lines".to_string(), random));
    out
}

fn lines_of(streams: &[StreamDesc], lb: u64) -> (u64, u64) {
    streams.iter().flat_map(|s| stream_lines(s, lb)).fold((0, 0), |(r, w), (_, write)| if write { (r, w + 1) } else { (r + 1, w) })
}

/// Every trace the controller issues is legal, carries each line once, and the streamed check sees what the whole
/// trace's check sees.
#[test]
fn every_trace_the_controller_issues_is_legal() {
    let t = t();
    for (name, streams) in patterns() {
        let lb = if name.starts_with("72") { 72 } else { 64 };
        let trace = schedule(&t, lb, &streams);
        let facts = check_bus(&t, &trace).unwrap_or_else(|v| panic!("{name}: {v}"));
        let (reads, writes) = lines_of(&streams, lb);
        assert_eq!((facts.reads, facts.writes), (reads, writes), "{name}");
        assert_eq!(facts.data_ck, (reads + writes) * t.burst_ck(), "{name}");
        assert_eq!(measure(&t, lb, &streams), Ok(facts), "{name}: the streamed check");
        assert!(facts.efficiency() > 0.0 && facts.efficiency() <= 1.0, "{name}");
    }
}

/// The column commands in clock order, with whether a REFRESH falls between each and the one before it.
fn columns(trace: &[Ddr4Timed]) -> Vec<(u64, bool, bool)> {
    let mut out = Vec::new();
    let mut refreshed = false;
    for c in trace {
        match c.cmd {
            Ddr4Cmd::Ref => refreshed = true,
            Ddr4Cmd::Rd { .. } | Ddr4Cmd::Wr { .. } => {
                out.push((c.at, matches!(c.cmd, Ddr4Cmd::Wr { .. }), refreshed));
                refreshed = false;
            }
            _ => {}
        }
    }
    out
}

/// A long read pauses for nothing but refresh: its rows change behind the data, every gap between bursts that holds
/// no REFRESH is tCCD_S, and a refresh costs the row's close (tRTP after the last read, or the rest of tRAS), tRP,
/// tRFC and tRCD.
#[test]
fn a_long_read_pauses_only_to_refresh() {
    let t = t();
    let trace = schedule(&t, 64, &[run(0, 65_536 * 64, false)]);
    let facts = check_bus(&t, &trace).unwrap();
    let cols = columns(&trace);
    let mut stalls = 0;
    for w in cols.windows(2) {
        let gap = w[1].0 - w[0].0;
        if w[1].2 {
            stalls += 1;
            assert!(
                (t.trtp + t.trp + t.trfc + t.trcd..=t.tras + t.trp + t.trfc + t.trcd).contains(&gap),
                "a refresh stall of {gap} clocks at {}",
                w[1].0
            );
        } else {
            assert_eq!(gap, t.tccd_s, "a gap at {}", w[0].0);
        }
    }
    assert_eq!(stalls, facts.refreshes);
    assert!(facts.refreshes >= facts.end / t.trefi - 1, "{} refreshes in {} clocks", facts.refreshes, facts.end);
    let worst = (t.tras + t.trp + t.trfc + t.trcd) as f64 / t.trefi as f64;
    assert!(facts.efficiency() > 1.0 - worst - 0.002, "{}", facts.efficiency());
}

/// A read-modify-write turns the bus exactly as the rules charge: tCCD_S within a direction, READ to WRITE when the
/// chunk turns to its writes, WRITE to READ across bank groups (tWTR_S) when the next chunk begins, the next chunk's
/// rows opened behind the writes.
#[test]
fn a_read_modify_write_turns_the_bus_at_its_rules() {
    let t = t();
    let (lines, chunk) = (8_192, 32);
    let trace = schedule(&t, 64, &read_modify_write(lines, chunk, 64));
    let facts = check_bus(&t, &trace).unwrap();
    assert_eq!(facts.turnarounds, 2 * lines / chunk - 1);
    for w in columns(&trace).windows(2) {
        let ((a, a_write, _), (b, b_write, refreshed)) = (w[0], w[1]);
        if refreshed {
            continue;
        }
        let want = match (a_write, b_write) {
            (false, true) => t.read_to_write(),
            (true, false) => t.write_to_read(false),
            _ => t.tccd_s,
        };
        assert_eq!(b - a, want, "{} to {} at {a}", if a_write { "WRITE" } else { "READ" }, if b_write { "WRITE" } else { "READ" });
    }
}

/// The lookahead is what hides a row change: with room for one line the controller cannot open the next bank's row
/// before the data needs it, and the same read runs slower.
#[test]
fn row_commands_hide_behind_the_data() {
    let t = t();
    let stream = run(0, 16_384 * 64, false);
    let with = |lookahead: usize| -> Ddr4Facts {
        let mut trace = Vec::new();
        let mut sched = Scheduler::new(&t, lookahead);
        let mut sink = |c: Ddr4Timed| trace.push(c);
        for (line, write) in stream_lines(&stream, 64) {
            sched.line(line, write, &mut sink);
        }
        sched.finish(&mut sink);
        check_bus(&t, &trace).unwrap_or_else(|v| panic!("lookahead {lookahead}: {v}"))
    };
    let (one, full) = (with(1), with(LOOKAHEAD));
    assert!(one.efficiency() < full.efficiency(), "lookahead 1: {}, {LOOKAHEAD}: {}", one.efficiency(), full.efficiency());
    assert_eq!(full, measure(&t, 64, &[stream]).unwrap());
}

// ── The ledger ───────────────────────────────────────────────────────────────────────────────────────

fn first_difference(a: &serde_json::Value, b: &serde_json::Value, at: String) -> Option<String> {
    use serde_json::Value::*;
    match (a, b) {
        (Number(x), Number(y)) => {
            let (x, y) = (x.as_f64()?, y.as_f64()?);
            ((x - y).abs() > 1e-12 * x.abs().max(y.abs())).then_some(format!("{at}: {x} vs {y}"))
        }
        (Array(x), Array(y)) if x.len() == y.len() => x.iter().zip(y).enumerate().find_map(|(i, (x, y))| first_difference(x, y, format!("{at}/{i}"))),
        (Object(x), Object(y)) if x.len() == y.len() && x.keys().all(|k| y.contains_key(k)) => {
            x.iter().find_map(|(k, v)| first_difference(v, &y[k], format!("{at}/{k}")))
        }
        _ => (a != b).then_some(format!("{at}: {a} vs {b}")),
    }
}

fn decode_dir() -> PathBuf {
    repo().join("benchmarks/silicon/decode")
}

#[test]
fn the_committed_ddr_ledger_is_what_the_model_computes() {
    let path = repo().join("benchmarks/silicon/d5005/ddr4_efficiency.json");
    let committed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).expect("the ledger parses");
    let fresh = serde_json::to_value(ddr_ledger(&decode_dir()).expect("the ledger prices")).expect("serializes");
    if let Some(at) = first_difference(&committed, &fresh, String::new()) {
        panic!("benchmarks/silicon/d5005/ddr4_efficiency.json is stale at `{at}`: regenerate it with `cargo run -p logicaffeine-compile --example d5005_ddr`");
    }
}

/// The streams are sized from the checkpoint's own header: a Gated-DeltaNet block's memory is its value heads' key ×
/// value states and its conv ring the kernel's last taps over the convolved channels, two bytes each; a cache
/// position is the key heads' keys and values, a byte each.
#[test]
fn the_ledger_sizes_its_streams_from_the_header() {
    let h = GgufHeader::read_ledger(&decode_dir().join(format!("headers/{HEADER}.json"))).unwrap();
    let m = |k: &str| h.meta_u64(k).unwrap_or_else(|| panic!("{k}"));
    let (v_heads, k_dim, inner) = (m("ssm.time_step_rank"), m("ssm.state_size"), m("ssm.inner_size"));
    let conv = (m("ssm.conv_kernel") - 1) * (2 * m("ssm.group_count") * k_dim + inner);
    let state = (v_heads * k_dim * (inner / v_heads) + conv) * 2;
    let position = m("attention.head_count_kv") * (m("attention.key_length") + m("attention.value_length"));
    assert_eq!(stream_bytes(&h), Ok((state, position)));
    assert_eq!((state, position), (1_634_304, 2_048));
    let l = ddr_ledger(&decode_dir()).unwrap();
    assert_eq!((l.state_bytes, l.position_bytes), (state, position));
}

/// Every row is a legal trace's facts and its rates follow from them; a state chunk turns the bus twice and runs below
/// the long read, the more so the smaller it is; a cache turns it once to append and once to the next sequence.
#[test]
fn every_ledger_row_follows_from_its_facts() {
    let l = ddr_ledger(&decode_dir()).unwrap();
    let t = t();
    assert_eq!(l.rows.len(), 2 * (1 + STATE_CHUNKS.len() + CACHE_POSITIONS.len()));
    for beat in [Beat::Data64, Beat::Full72] {
        let rows: Vec<_> = l.rows.iter().filter(|r| r.beat == beat).collect();
        for r in &rows {
            let f = r.facts;
            assert_eq!(f.data_ck, (f.reads + f.writes) * t.burst_ck(), "{}", r.pattern);
            assert_eq!(r.efficiency, f.efficiency(), "{}", r.pattern);
            assert_eq!(r.controller_ck, f.end.div_ceil(4), "{}", r.pattern);
            assert_eq!(r.channel_bytes_per_s, f.efficiency() * t.peak_bytes_per_s(r.line_bytes), "{}", r.pattern);
            assert_eq!(r.card_bytes_per_s, 4.0 * r.channel_bytes_per_s, "{}", r.pattern);
            assert!(r.efficiency > 0.5 && r.efficiency <= 1.0, "{}: {}", r.pattern, r.efficiency);
        }
        assert!(rows[0].pattern.starts_with("weights"));
        let state = &rows[1..1 + STATE_CHUNKS.len()];
        let cache = &rows[1 + STATE_CHUNKS.len()..];
        let lines = l.state_bytes.div_ceil(rows[0].line_bytes);
        for (r, chunk) in state.iter().zip(STATE_CHUNKS) {
            assert_eq!((r.facts.reads, r.facts.writes), (SEQUENCES * lines, SEQUENCES * lines), "{}", r.pattern);
            assert_eq!(r.facts.turnarounds, 2 * SEQUENCES * lines.div_ceil(chunk) - 1, "{}", r.pattern);
            assert!(r.efficiency < rows[0].efficiency, "{}", r.pattern);
        }
        assert!(state.windows(2).all(|w| w[0].efficiency < w[1].efficiency), "{beat:?}: state chunks");
        for r in cache {
            assert_eq!(r.facts.turnarounds, 2 * SEQUENCES - 1, "{}", r.pattern);
        }
    }
}

// ── The channel model in Verilog ─────────────────────────────────────────────────────────────────────

/// One Avalon-MM request of a script: a read burst, a write burst (its lines written with their pattern), a write
/// burst enabling every other byte, or idle controller clocks.
#[derive(Clone, Copy)]
enum Request {
    Read(u64, u64),
    Write(u64, u64),
    Masked(u64, u64),
    Idle(u64),
}

/// What a run of the channel model said: its log, the controller clock each read returned on, and the reads whose
/// data was not the last data written to their line.
struct ModelRun {
    log: ChannelLog,
    returns: Vec<u64>,
    bad_reads: u64,
}

const DW: usize = 576;

/// The model driven by a scripted Avalon-MM master that issues each request as soon as waitrequest lets it.
fn run_model(t: &Ddr4Timing, script: &[Request]) -> ModelRun {
    let dir = tempfile::tempdir().unwrap();
    let words: String = script
        .iter()
        .flat_map(|r| match *r {
            Request::Read(a, n) => [0, a, n],
            Request::Write(a, n) => [1, a, n],
            Request::Idle(n) => [2, 0, n],
            Request::Masked(a, n) => [3, a, n],
        })
        .map(|v| format!("{v:016x}\n"))
        .collect();
    std::fs::write(dir.path().join("script.hex"), words).unwrap();
    std::fs::write(dir.path().join("model.v"), verilog_model(t, LOOKAHEAD)).unwrap();
    let tb = format!(
        r#"`timescale 1ps/1ps
module tb;
  localparam AW = 27, DW = {DW}, N = {n};
  reg clk = 1'b0, rst_n = 1'b0;
  always #1667 clk = ~clk;
  reg [63:0] script [0:3*N-1];
  initial $readmemh("script.hex", script);
  reg [AW-1:0] address = 0;
  reg read = 1'b0, write = 1'b0;
  reg [6:0] burstcount = 0;
  reg [DW-1:0] writedata = 0;
  reg [DW/8-1:0] byteenable = 0;
  wire waitrequest, readdatavalid;
  wire [DW-1:0] readdata;
  logos_ddr4_channel_model #(.AW(AW), .DW(DW), .BW(7), .LINES_LOG2(17), .LOG(1), .LOGFILE("channel"), .ID(0)) model(
    .clk(clk), .rst_n(rst_n), .address(address), .read(read), .write(write), .burstcount(burstcount),
    .writedata(writedata), .byteenable(byteenable), .waitrequest(waitrequest), .readdata(readdata),
    .readdatavalid(readdatavalid));
  function [DW-1:0] pattern(input [63:0] line, input [63:0] stamp);
    integer k;
    begin
      for (k = 0; k < DW / 64; k = k + 1)
        pattern[64*k +: 64] = line * 64'h9e3779b97f4a7c15 ^ stamp * 64'hc2b2ae3d27d4eb4f ^ (k + 1) * 64'h165667b19e3779f9;
    end
  endfunction
  // What each line holds after the writes so far, byte for byte as their byte enables merged them; expected reads
  // in order.
  reg [DW-1:0] expect_mem [0:(1<<17)-1];
  reg          written [0:(1<<17)-1];
  reg [DW-1:0] want_data [0:65535];
  reg          want_known [0:65535];
  integer wh = 0, wt = 0, bad = 0, returned = 0, f, i;
  initial for (i = 0; i < (1<<17); i = i + 1) written[i] = 0;
  reg [63:0] cycle = 0, pc = 0, left = 0, at = 0, stamp = 1;
  reg busy = 1'b0, is_write = 1'b0;
  always @(posedge clk) begin
    cycle = cycle + 1;
    if (rst_n) begin
      if (readdatavalid) begin
        $fwrite(f, "R %0d\n", cycle - 1);
        if (want_known[wh % 65536] && readdata !== want_data[wh % 65536]) bad = bad + 1;
        wh = wh + 1;
        returned = returned + 1;
      end
      if ((read || write) && !waitrequest) begin
        if (read) begin
          for (i = 0; i < burstcount; i = i + 1) begin
            want_data[wt % 65536] = expect_mem[(address + i) % (1<<17)];
            want_known[wt % 65536] = written[(address + i) % (1<<17)];
            wt = wt + 1;
          end
          read <= 1'b0;
          busy = 1'b0;
          left = 0;
        end else begin
          written[at % (1<<17)] = 1;
          for (i = 0; i < DW / 8; i = i + 1)
            if (byteenable[i]) expect_mem[at % (1<<17)][8*i +: 8] = writedata[8*i +: 8];
          at = at + 1;
          left = left - 1;
          if (left == 0) begin write <= 1'b0; busy = 1'b0; stamp = stamp + 1; end
          else writedata <= pattern(at, stamp);
        end
      end
      if (!busy && !read && !write && pc < N) begin
        if (script[3*pc] == 2) begin
          if (left == 0) left = script[3*pc+2];
          left = left - 1;
          if (left == 0) pc = pc + 1;
        end else begin
          busy = 1'b1;
          address <= script[3*pc+1];
          burstcount <= script[3*pc+2];
          at = script[3*pc+1];
          left = script[3*pc+2];
          if (script[3*pc] == 0) read <= 1'b1;
          else begin
            write <= 1'b1;
            writedata <= pattern(script[3*pc+1], stamp);
            byteenable <= script[3*pc] == 3 ? {{(DW/16){{2'b01}}}} : {{(DW/8){{1'b1}}}};
          end
          pc = pc + 1;
        end
      end
      if (pc == N && !busy && !read && !write && wh == wt) begin
        $fwrite(f, "BAD %0d\n", bad);
        $fclose(f);
        $display("DONE");
        $finish;
      end
    end
  end
  initial begin
    f = $fopen("returns.log", "w");
    #20000 rst_n = 1'b1;
  end
  initial begin #(64'd20000000000); $display("TIMEOUT"); $finish; end
endmodule
"#,
        n = script.len()
    );
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let sim = dir.path().join("sim");
    let build = std::process::Command::new("iverilog")
        .args(["-g2012", "-o"])
        .arg(&sim)
        .arg(dir.path().join("tb.v"))
        .arg(dir.path().join("model.v"))
        .output()
        .expect("iverilog");
    assert!(build.status.success(), "iverilog:\n{}{}", String::from_utf8_lossy(&build.stdout), String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new("vvp").arg(&sim).current_dir(dir.path()).output().expect("vvp");
    let said = String::from_utf8_lossy(&run.stdout).to_string();
    assert!(run.status.success() && said.contains("DONE"), "vvp:\n{said}{}", String::from_utf8_lossy(&run.stderr));
    let mut out = ModelRun { log: read_log(&dir.path().join("channel_0.log")), returns: vec![], bad_reads: 0 };
    for l in std::fs::read_to_string(dir.path().join("returns.log")).unwrap().lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        match f[0] {
            "R" => out.returns.push(f[1].parse().unwrap()),
            "BAD" => out.bad_reads = f[1].parse().unwrap(),
            _ => panic!("{l}"),
        }
    }
    out
}

/// The model's run replayed through the controller model, every read returned on the controller clock its burst left
/// the bus in and holding what was last written to its line.
fn replay_run(t: &Ddr4Timing, run: &ModelRun) -> Ddr4Facts {
    let (due, facts) = replay(t, &run.log, "the channel model");
    assert_eq!(run.returns, due, "every read returns on the clock its burst leaves the bus");
    assert_eq!(run.bad_reads, 0, "every read holds the last data written to its line");
    facts
}

/// A long read in bursts the master issues as fast as the slave takes them: the model schedules every line as the
/// controller model does, and since every line arrives before the channel needs it, as the controller model does
/// with every line there from the first one's clock — the cycles `measure` prices.
#[test]
fn the_channel_model_streams_as_the_controller_model() {
    if !common::require_tool("iverilog") {
        return;
    }
    let script: Vec<Request> = (0..256).map(|k| Request::Read(k * 64, 64)).collect();
    let run = run_model(&t(), &script);
    let facts = replay_run(&t(), &run);
    assert_eq!(facts.reads, 256 * 64);
    let first = run.log.lines[0].2;
    let t = t();
    let mut sched = Scheduler::new(&t, LOOKAHEAD);
    for (k, &(line, write, _, cas)) in run.log.lines.iter().enumerate() {
        assert_eq!(sched.line_at(line, write, first, &mut |_| {}), cas, "line {k} waited on its arrival");
    }
}

/// Writes then reads of the same lines, in chunks, some writes enabling every other byte, and reads of an open row
/// after the channel waited a while: the bus turns at the rules', no line's command comes before the line, the data
/// read back is the data written, merged by its byte enables.
#[test]
fn the_channel_model_reads_back_what_it_wrote() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut script = Vec::new();
    for k in 0..16u64 {
        script.push(Request::Read(k * 8, 4));
        script.push(Request::Idle(40 + k * 7));
        script.push(Request::Read(k * 8 + 4, 4));
    }
    for k in 0..64u64 {
        script.push(Request::Write(k * 32, 32));
        script.push(Request::Read(k * 32, 32));
    }
    for k in 0..16u64 {
        script.push(Request::Write(k * 32 + 7, 5));
        script.push(Request::Read(k * 32, 32));
        script.push(Request::Masked(k * 32 + 3, 9));
        script.push(Request::Read(k * 32, 16));
    }
    let facts = replay_run(&t(), &run_model(&t(), &script));
    assert!(facts.turnarounds >= 2 * 64 + 2 * 16 - 1);
}

/// Scattered lines and directions with the channel left idle, longer than a refresh interval at a time: refreshes that
/// came due while it waited are issued on their time, the trace stays legal and the model stays the controller model.
#[test]
fn the_channel_model_refreshes_while_it_waits() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut s = 0x0123_4567_89AB_CDEFu64;
    let mut script = Vec::new();
    for k in 0..400u64 {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let at = (s >> 20) % (1 << 17);
        let n = 1 + (s >> 8) % 8;
        let at = at.min((1 << 17) - n);
        script.push(if s & 1 == 1 { Request::Write(at, n) } else { Request::Read(at, n) });
        if k % 40 == 39 {
            script.push(Request::Idle(3_000 + (s >> 40) % 2_000));
        }
    }
    let facts = replay_run(&t(), &run_model(&t(), &script));
    let t = t();
    assert!(facts.refreshes >= facts.end / t.trefi - 1, "{} refreshes over {} clocks", facts.refreshes, facts.end);
}

/// The model is printed for any grade: on one whose tRC outlasts tRAS + tRP, a line a row in one bank waits on tRC,
/// and the model still schedules as the controller model does.
#[test]
fn the_channel_model_holds_a_longer_trc() {
    if !common::require_tool("iverilog") {
        return;
    }
    let slow = Ddr4Grade { trc_ps: Some(60_000), ..Ddr4Grade::D5005_EMIF }.at();
    assert!(slow.trc > slow.tras + slow.trp);
    let script: Vec<Request> = (0..48u64).map(|k| if k % 3 == 2 { Request::Write(k << 11, 1) } else { Request::Read(k << 11, 1) }).collect();
    let run = run_model(&slow, &script);
    replay_run(&slow, &run);
    let acts: Vec<u64> = run.log.commands.iter().filter(|c| c.1 == 0).map(|c| c.0).collect();
    assert!(acts.windows(2).any(|w| w[1] - w[0] == slow.trc), "an ACT waits exactly tRC: {acts:?}");
}

/// Under a hand-made timing whose column spacing is shorter than a burst, the data bus is what spaces the bursts: the
/// controller keeps every burst off the last, and the channel model with it.
#[test]
fn the_controller_keeps_bursts_apart_at_any_spacing() {
    let tight = Ddr4Timing { tccd_s: 2, tccd_l: 2, ..t() };
    for (name, streams) in patterns() {
        let lb = if name.starts_with("72") { 72 } else { 64 };
        check_bus(&tight, &schedule(&tight, lb, &streams)).unwrap_or_else(|v| panic!("{name}: {v}"));
    }
    if !common::require_tool("iverilog") {
        return;
    }
    let script: Vec<Request> = (0..64u64).map(|k| if k % 4 == 3 { Request::Write(k * 16, 16) } else { Request::Read(k * 16, 16) }).collect();
    let facts = replay_run(&tight, &run_model(&tight, &script));
    assert_eq!(facts.reads + facts.writes, 64 * 16);
}
