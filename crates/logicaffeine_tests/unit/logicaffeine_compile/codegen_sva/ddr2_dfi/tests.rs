use super::*;
use crate::codegen_sva::ddr2_jedec::Ddr2Grade;

fn at(tck_ps: u64) -> Ddr2Timing {
    Ddr2Grade::MT47H64M16_25.at(tck_ps, 4).unwrap()
}

/// The value of localparam `name` in the emitted controller.
fn param(v: &str, name: &str) -> String {
    let key = format!(" {name} = ");
    let at = v.find(&key).unwrap_or_else(|| panic!("no localparam {name}"));
    v[at + key.len()..].split([',', ';']).next().unwrap().trim().to_string()
}

#[test]
fn the_controller_bakes_every_rule_of_the_clocked_part() {
    for tck in [5000, 3750] {
        let t = at(tck);
        let v = emit_ddr2_dfi_controller_verilog(&t);
        for (name, want) in [
            ("T_RCD", t.trcd),
            ("T_RP", t.trp),
            ("T_RPA", t.trpa),
            ("T_RAS", t.tras),
            ("T_RC", t.trc),
            ("T_RFC", t.trfc),
            ("RD2PRE", t.read_to_precharge()),
            ("WR2PRE", t.write_to_precharge()),
            ("WR2RD", t.write_to_read()),
            ("RD2WR", t.read_to_write()),
            ("CAS2CAS", t.cas_to_cas()),
            ("POSTPONE", REFRESH_POSTPONE_MAX),
        ] {
            assert_eq!(param(&v, name), want.to_string(), "tCK {tck}: {name}");
        }
        let cw: u32 = param(&v, "CW").parse().unwrap();
        assert!(1u64 << cw > 1 + t.trfc && 1u64 << cw > t.write_to_precharge().max(1 + t.tras) + t.trp, "tCK {tck}: counters hold every rule");
        assert_eq!(param(&v, "QB"), "2", "a queue of {DFI_CTRL_QUEUE}");
    }
}

/// tRRD and tFAW bound ACTs, which go out on phase 1 at most once a system clock: an ACT must be at least
/// ceil(tRRD/2) system clocks after the one before and ceil(tFAW/2) after the fourth before, so the controller looks
/// at the ACTs of the last ceil(tRRD/2) − 1 and ceil(tFAW/2) − 1 clocks.
#[test]
fn the_act_window_is_trrd_and_tfaw_in_system_clocks() {
    let ddr2_400 = emit_ddr2_dfi_controller_verilog(&at(5000));
    assert_eq!((at(5000).trrd, at(5000).tfaw), (2, 9));
    assert!(ddr2_400.contains("&& ({2'd0, acts[0]} + {2'd0, acts[1]} + {2'd0, acts[2]} + {2'd0, acts[3]}) < 3'd4 && 1'b1;"), "{ddr2_400}");
    assert!(ddr2_400.contains("reg [4-1:0] acts;"));
    let ddr2_533 = emit_ddr2_dfi_controller_verilog(&at(3750));
    assert_eq!((at(3750).trrd, at(3750).tfaw), (3, 12));
    assert!(
        ddr2_533.contains("&& ({2'd0, acts[0]} + {2'd0, acts[1]} + {2'd0, acts[2]} + {2'd0, acts[3]} + {2'd0, acts[4]}) < 3'd4 && acts[0:0] == 0;"),
        "{ddr2_533}"
    );
    assert!(ddr2_533.contains("reg [5-1:0] acts;"));
}

#[test]
fn the_commands_are_the_jedec_truth_table() {
    let v = emit_ddr2_dfi_controller_verilog(&at(5000));
    for (name, cmd) in [
        ("NOP", Ddr2Cmd::Nop),
        ("ACT", Ddr2Cmd::Active),
        ("PRE", Ddr2Cmd::Precharge),
        ("REF", Ddr2Cmd::Refresh),
        ("RD", Ddr2Cmd::Read),
        ("WR", Ddr2Cmd::Write),
    ] {
        let (cs, ras, cas, we) = ddr2_pins(cmd);
        assert_eq!(param(&v, name), format!("4'b{cs}{ras}{cas}{we}"), "{name}");
    }
}

/// The controller counts a refresh owed every REFI system clocks, two DRAM clocks each: never later than tREFI, so
/// it never owes fewer than the JEDEC checker counts.
#[test]
fn a_refresh_is_owed_no_later_than_trefi() {
    for tck in [5000, 3750, 3000] {
        let t = at(tck);
        let refi: u64 = param(&emit_ddr2_dfi_controller_verilog(&t), "REFI").parse().unwrap();
        assert!(2 * refi <= t.trefi && 2 * (refi + 1) > t.trefi, "tCK {tck}: REFI {refi} for tREFI {}", t.trefi);
    }
}

#[test]
#[should_panic(expected = "one BL4 burst")]
fn a_burst_other_than_four_is_refused() {
    emit_ddr2_dfi_controller_verilog(&Ddr2Grade::MT47H64M16_25.at(5000, 8).unwrap());
}
