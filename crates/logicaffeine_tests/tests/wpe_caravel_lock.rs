//! wpe_caravel_lock — the whole-program engine as a Caravel user project runs programs through the Caravel
//! pins: its UART on two user GPIOs, the QSPI PSRAM on six more, clocked by `user_clock2` and reset by the
//! management SoC.
//!
//! WHY. The engine's off-chip memories need a part on the board, reached through Caravel's user GPIOs, and
//! the pin plan appears in three places — the macro's `io_out`/`io_oeb` wiring, the pads' power-on modes in
//! `user_defines.v`, and the host that talks to it. One plan drives all three here, and a program run through
//! the pins against the part's pin model is the proof they agree.
//!
//! What this file locks:
//!   1. the shipped pin plan uses user GPIOs only, one signal each, clear of the management UART, and the
//!      GPIO modes follow it: the UART's receive pin an input, its transmit, CE# and SCLK outputs, SIO
//!      bidirectional, every other user GPIO an input;
//!   2. refusals: a housekeeping pin, a shared pin, a baud rate the clock cannot make within a UART's
//!      tolerance, a UART the loading stalls could lose bytes on, and an engine clock outside the range
//!      the PSRAM edge is timed for; and the engine runs on `user_clock2` alone, its reset crossing from the
//!      management SoC through a synchronizer, its Wishbone acknowledge on `wb_clk_i`;
//!   3. programs run through the Caravel pins stream the on-chip engine's records and frames and the
//!      model's output, with no PSRAM violation, no lost byte and no pin driven the wrong way;
//!   4. the bench can fail: a transmit pin left an input, and an SIO pad whose enable is inverted;
//!   5. the chip's configuration fits Caravel's harness: half an SCLK period at the engine's clock covers a
//!      read's round trip through the harness's characterized pad paths, the board and the part's tACLK, and
//!      a clock whose window does not is refused by name;
//!   6. the fabricated top cell, `user_project_wrapper`, runs the bring-up programs through its pins at the
//!      chip's clock with every PSRAM pin at the harness's slowest and at its fastest delay, and a round trip
//!      longer than the window is caught;
//!   7. the checked-in macro, wrapper and submission overlay are the emitters' output, byte for byte.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(wpe_caravel_lock)'`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::caravel_project::WPE_PROJECT;
use logicaffeine_compile::codegen_sva::pdk_target::{HarnessClock, SKY130A};
use logicaffeine_compile::codegen_sva::signoff_blocks::honest_period_ns;
use logicaffeine_compile::codegen_sva::fabric_rtl::program_cfg_fabric;
use logicaffeine_compile::codegen_sva::psram::{PsramCache, PsramEdge, APS6404L_3SQN};
use logicaffeine_compile::codegen_sva::wpe_caravel::{
    emit_wpe_user_defines, emit_wpe_user_project_testbench, emit_wpe_user_project_verilog,
    emit_wpe_user_project_wrapper_testbench, wpe_caravel_bringup_vectors, wpe_caravel_memory,
    wpe_caravel_overlay_files, wpe_caravel_wire, wpe_psram_read_sta_tcl, GpioMode, HarnessDelay,
    WpeCaravel, WpeCaravelUnplaceable, WpePins, BOARD_FLIGHT_MAX_PS, BRINGUP_TEXT,
    CARAVEL_FIRST_USER_IO, CARAVEL_IO, CARAVEL_IO_MAX, CARAVEL_IO_MIN, WPE_CARAVEL,
    WPE_CARAVEL_BACKEND, WPE_CARAVEL_CAPS, WPE_CARAVEL_NAME, WPE_CLOCK, WPE_PINS,
};
use logicaffeine_compile::codegen_sva::wpe_offchip::{OffchipMemory, OffchipUnplaceable};
use logicaffeine_compile::codegen_sva::wprog_engine_rtl::{
    emit_wprog_engine_soc_testbench, emit_wprog_engine_soc_verilog_backend,
    eval_wprog_config_counted_heap_preloaded, eval_wprog_config_run, program_main_fabric,
    render_show, serialize_program, wprog_config_image, MemBackend, WShow,
};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The engine's clock: 20 ns (50 MHz), the period the edge block is hardened at.
const SYS_PS: u32 = 20_000;

/// The template's default profile: room for text and a streamed preload.
const CAPS: [usize; 10] = [48, 16, 64, 64, 16, 48, 64, 128, 24, 64];

fn memory() -> OffchipMemory {
    let edge =
        PsramEdge::new(APS6404L_3SQN, SYS_PS, 1).expect("the part runs at a 20 ns system clock");
    let c = PsramCache::new(&edge, 2, 3).expect("four lines of eight words");
    OffchipMemory {
        edge,
        heap: Some(c),
        frames: Some(c),
        cells: Some(c),
    }
}

/// Pins with no harness between the macro and the part: the wiring's logic on its own.
const IDEAL: HarnessDelay = HarnessDelay {
    out_ps: 0,
    oeb_ps: 0,
    in_ps: 0,
};

/// 460800 baud: a fast host that still leaves every loading stall inside a byte.
fn caravel() -> WpeCaravel {
    WpeCaravel {
        clk_period_ps: SYS_PS,
        baud: 460_800,
        pins: WPE_PINS,
    }
}

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join("logos_wpe_caravel").join(tag);
    std::fs::create_dir_all(&d).expect("create the simulation directory");
    d
}

fn simulate(dir: &Path, files: &[(&str, String)]) -> String {
    for (name, text) in files {
        std::fs::write(dir.join(name), text).expect("write a simulation source");
    }
    let build = Command::new("iverilog")
        .args(["-g2012", "-o", "sim.vvp"])
        .args(files.iter().map(|(n, _)| *n))
        .current_dir(dir)
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed in {}:\n{}",
        dir.display(),
        String::from_utf8_lossy(&build.stderr)
    );
    let run = Command::new("vvp")
        .args(["-n", "sim.vvp"])
        .current_dir(dir)
        .output()
        .expect("run vvp");
    String::from_utf8_lossy(&run.stdout).into_owned()
}

// ── 1-2: the plan and its refusals ─────────────────────────────────────────────────────────────────────────

#[test]
fn the_pin_plan_and_the_gpio_modes_agree() {
    WPE_PINS
        .check()
        .expect("the shipped plan uses user GPIOs, one signal each");
    for (pin, _) in WPE_PINS.used() {
        assert!(
            !(5..=6).contains(&pin),
            "GPIO 5 and 6 carry the management UART"
        );
    }
    let modes = WPE_PINS.gpio_modes();
    assert_eq!(
        modes.len(),
        (CARAVEL_IO - CARAVEL_FIRST_USER_IO) as usize,
        "a mode for every user GPIO"
    );
    for (pin, mode) in &modes {
        let expect = if *pin == WPE_PINS.uart_tx
            || *pin == WPE_PINS.psram_ce_n
            || *pin == WPE_PINS.psram_sclk
        {
            GpioMode::Output
        } else if WPE_PINS.psram_sio.contains(pin) {
            GpioMode::Bidirectional
        } else {
            GpioMode::Input
        };
        assert_eq!(*mode, expect, "GPIO {pin}");
    }
    let defines = emit_wpe_user_defines(&WPE_PINS);
    assert!(
        !defines.contains("USER_CONFIG_GPIO_4_INIT"),
        "the housekeeping pins are not ours to configure"
    );
    for (pin, mode) in modes {
        let line = format!("`define USER_CONFIG_GPIO_{pin}_INIT `{}", mode.define());
        assert_eq!(
            defines.matches(&line).count(),
            1,
            "user_defines.v must say `{line}`"
        );
    }
    assert!(
        !defines
            .lines()
            .any(|l| l.starts_with("`define USER_CONFIG") && l.contains("INVALID")),
        "no GPIO left INVALID"
    );
}

#[test]
fn every_unwirable_caravel_engine_is_refused_by_name() {
    let m = memory();
    let housekeeping = WpePins {
        uart_rx: 3,
        ..WPE_PINS
    };
    assert_eq!(
        housekeeping.check(),
        Err(WpeCaravelUnplaceable::PinNotUser { pin: 3 })
    );
    assert_eq!(
        WpePins {
            psram_sclk: 38,
            ..WPE_PINS
        }
        .check(),
        Err(WpeCaravelUnplaceable::PinNotUser { pin: 38 })
    );
    assert_eq!(
        WpePins {
            psram_ce_n: 9,
            ..WPE_PINS
        }
        .check(),
        Err(WpeCaravelUnplaceable::PinShared { pin: 9 })
    );

    let at = |baud| WpeCaravel {
        clk_period_ps: SYS_PS,
        baud,
        pins: WPE_PINS,
    };
    assert_eq!(at(115_200).clks_per_bit(), 434);
    assert!(
        at(115_200).baud_error_ppm() < 100,
        "50 MHz makes 115200 baud to within 0.01%"
    );
    at(115_200)
        .check(&m)
        .expect("115200 baud is a host the engine loses nothing to");
    assert!(
        matches!(
            at(3_000_000).check(&m),
            Err(WpeCaravelUnplaceable::BaudUnreachable { .. })
        ),
        "17 cycles is 2% off 3 Mbaud"
    );
    assert!(
        matches!(
            at(1_000_000).check(&m),
            Err(WpeCaravelUnplaceable::Memory(
                OffchipUnplaceable::StallOutlastsByte { .. }
            ))
        ),
        "50 cycles a bit is exact, but a loading stall outlasts its byte"
    );
    let slower = WpeCaravel {
        clk_period_ps: 25_000,
        ..at(115_200)
    };
    assert_eq!(
        slower.check(&m),
        Err(WpeCaravelUnplaceable::ClockOutsideEdgeRange {
            engine_ps: 25_000,
            fastest_ps: SYS_PS,
            slowest_ps: SYS_PS
        }),
        "an edge timed for exactly 20 ns may not be run slower"
    );
    let ranged = OffchipMemory {
        edge: m.edge.clocked_down_to(40_000).unwrap(),
        ..m
    };
    let ranged = OffchipMemory {
        heap: Some(PsramCache::new(&ranged.edge, 2, 2).unwrap()),
        frames: None,
        cells: None,
        ..ranged
    };
    slower
        .check(&ranged)
        .expect("an edge timed for 20 to 40 ns runs at 25 ns");
    assert!(emit_wpe_user_project_verilog(
        "x",
        CAPS,
        MemBackend::ConfigBram,
        &m,
        &WpeCaravel {
            pins: housekeeping,
            ..caravel()
        }
    )
    .is_err());
}

/// The engine's clock is the period its slow corner meets, by the rule every signed block's is
/// (`signoff_blocks::honest_period_ns`). MEASURED at 18.5 ns on the macro harden RUN_2026.10.25_06.23.41, its every
/// net routed (detailed routing replayed to its 36th pass, 85 local shorts left), extracted with the max-RC rules and
/// timed at ss_100C_1v60: worst setup -1.13 ns, total -979 ns, through ~18.7 ns of compare-to-control logic and its
/// fanout tree with 0.94 ns of clock skew, where the routing stage's repairs had closed to +0.03 ns on global-route
/// parasitics. At 16 ns (RUN_2026.10.20_17.57.25) the same corner had missed by 1.482 ns on global-route parasitics.
#[test]
fn the_engine_runs_at_the_period_its_slow_corner_meets() {
    assert_eq!(
        WPE_CARAVEL.clk_period_ps as f64 / 1000.0,
        honest_period_ns(18.5, -1.13),
        "the engine's period is the measured slow corner's honest period"
    );
    assert_eq!(WPE_CARAVEL.clk_period_ps, 21_000);
    WPE_CARAVEL
        .check_harness(&wpe_caravel_memory())
        .expect("the harness and the board take the engine at that period");
}

#[test]
fn the_shipped_caravel_engine_wires_and_its_numbers_hold() {
    let m = wpe_caravel_memory();
    WPE_CARAVEL
        .check(&m)
        .expect("the shipped engine wires to Caravel");
    assert_eq!(WPE_CARAVEL.clks_per_bit(), 413, "47.62 MHz over 115200 baud");
    assert_eq!(
        m.edge.max_words(false),
        2,
        "tCEM leaves two-word bursts at 40 ns, two cycles per SCLK half"
    );
    assert!(m.check_uart(WPE_CARAVEL.clks_per_bit()).is_ok());
    let rtl = emit_wpe_user_project_verilog(
        WPE_CARAVEL_NAME,
        WPE_CARAVEL_CAPS,
        WPE_CARAVEL_BACKEND,
        &m,
        &WPE_CARAVEL,
    )
    .expect("the shipped engine emits");
    assert!(
        rtl.contains("module wpe_user_project")
            && rtl.contains("wpe_soc_caravel #(.CLKS_PER_BIT(413)) engine")
    );
    let regions = m.regions(WPE_CARAVEL_CAPS);
    let words: u64 = regions.iter().map(|r| 1u64 << r.bits).sum();
    assert!(
        words <= m.part_words(),
        "heap, frame stack and config image fit the part with room to spare: {words} words"
    );
}

/// The engine runs on `user_clock2` (`WPE_CLOCK`): the management SoC's core clock, `wb_clk_i`, is signed off at
/// 25 ns and the engine at 16. Only two things cross between them, and the macro's own top holds both: the reset,
/// which reaches the engine through two flops in its domain (asserted at once, with no clock running, and released
/// on the second `user_clock2` edge after its source falls), and the Wishbone acknowledge, which stays on
/// `wb_clk_i`. The reset's source is `wb_rst_i` or logic-analyzer probe 0 while the management SoC drives it
/// (`la_oenb[0]` low, efabless's own override in `user_proj_example.v`): at power-on `user_clock2` is the pad clock,
/// and the firmware that moves it to the PLL resets the engine after the switch, with the PLL left running. The top
/// is simulated with the engine replaced by a stub that counts its clock and shows its reset; nothing else of the bus
/// or the logic analyzer reaches the engine.
#[test]
fn the_engine_runs_on_user_clock2_and_its_reset_crosses_through_a_synchronizer() {
    assert_eq!(WPE_CLOCK, HarnessClock::User2);
    let rtl = emit_wpe_user_project_verilog(
        WPE_CARAVEL_NAME,
        WPE_CARAVEL_CAPS,
        WPE_CARAVEL_BACKEND,
        &wpe_caravel_memory(),
        &WPE_CARAVEL,
    )
    .expect("the shipped engine emits");
    let top = &rtl[rtl.find("module wpe_user_project (").expect("the macro's top")..];
    let inst = &top[top.find(&format!("wpe_soc_{WPE_CARAVEL_NAME} #(")).expect("the engine instance")..];
    let inst = &inst[..inst.find(");").unwrap()];
    assert!(inst.contains(".clk(user_clock2)"), "the engine's clock is user_clock2:\n{inst}");
    for word in ["wb_clk_i", "wb_rst_i", "wbs_", "la_"] {
        assert!(!inst.contains(word), "{word} reaches the engine only through the synchronizer:\n{inst}");
    }
    assert!(
        top.contains("always @(posedge wb_clk_i) ack <="),
        "the Wishbone acknowledge stays on wb_clk_i"
    );
    let stub = format!(
        "module wpe_soc_{WPE_CARAVEL_NAME} #(parameter CLKS_PER_BIT = 1) (input clk, input rst, input uart_rx,\n\
         \x20 output uart_tx, output psram_ce_n, output psram_sclk, output [3:0] psram_sio_out, output [3:0] psram_sio_oe,\n\
         \x20 input [3:0] psram_sio_in);\n\
         \x20 integer edges = 0;\n\
         \x20 always @(posedge clk) edges = edges + 1;\n\
         \x20 assign uart_tx = 1'b1; assign psram_ce_n = 1'b1; assign psram_sclk = 1'b0;\n\
         \x20 assign psram_sio_out = 4'd0; assign psram_sio_oe = 4'd0;\n\
         endmodule\n"
    );
    let tb = "`timescale 1ns/1ps\n\
        module tb;\n\
        \x20 reg wb_clk = 0, uclk = 0, wb_rst = 1, cyc = 0, stb = 0, probe = 0, probe_oenb = 1;\n\
        \x20 integer e0;\n\
        \x20 wire ack; wire [31:0] dat; wire [127:0] la_out; wire [37:0] io_out, io_oeb; wire [2:0] irq;\n\
        \x20 wpe_user_project dut (.wb_clk_i(wb_clk), .wb_rst_i(wb_rst), .wbs_stb_i(stb), .wbs_cyc_i(cyc),\n\
        \x20   .wbs_we_i(1'b0), .wbs_sel_i(4'd0), .wbs_dat_i(32'd0), .wbs_adr_i(32'd0), .wbs_ack_o(ack), .wbs_dat_o(dat),\n\
        \x20   .la_data_in({127'd0, probe}), .la_data_out(la_out), .la_oenb({{127{1'b1}}, probe_oenb}), .io_in(38'd0),\n\
        \x20   .io_out(io_out),\n\
        \x20   .io_oeb(io_oeb), .user_clock2(uclk), .user_irq(irq));\n\
        \x20 task utick; begin #8 uclk = 1; #8 uclk = 0; end endtask\n\
        \x20 task wtick; begin #12.5 wb_clk = 1; #12.5 wb_clk = 0; end endtask\n\
        \x20 initial begin\n\
        \x20   repeat (3) utick;\n\
        \x20   $display(\"HELD %0d\", dut.engine.rst);\n\
        \x20   #3 wb_rst = 0;\n\
        \x20   #1 $display(\"RELEASED %0d\", dut.engine.rst);\n\
        \x20   #4 uclk = 1; #1 $display(\"EDGE1 %0d\", dut.engine.rst);\n\
        \x20   #7 uclk = 0; #8 uclk = 1; #1 $display(\"EDGE2 %0d\", dut.engine.rst);\n\
        \x20   #7 uclk = 0;\n\
        \x20   e0 = dut.engine.edges;\n\
        \x20   repeat (4) wtick;\n\
        \x20   $display(\"WB_EDGES %0d\", dut.engine.edges - e0);\n\
        \x20   repeat (5) utick;\n\
        \x20   $display(\"USER_EDGES %0d\", dut.engine.edges - e0);\n\
        \x20   cyc = 1; stb = 1;\n\
        \x20   #1 $display(\"ACK_BEFORE %0d\", ack);\n\
        \x20   #12.5 wb_clk = 1; #1 $display(\"ACK_AFTER %0d\", ack);\n\
        \x20   #11.5 wb_clk = 0; cyc = 0; stb = 0;\n\
        \x20   wb_rst = 1; #1 $display(\"ASSERTED %0d\", dut.engine.rst);\n\
        \x20   #2 wb_rst = 0; repeat (2) utick;\n\
        \x20   $display(\"RUNNING %0d\", dut.engine.rst);\n\
        \x20   probe = 1; #1 $display(\"PROBE_UNDRIVEN %0d\", dut.engine.rst);\n\
        \x20   probe_oenb = 0; #1 $display(\"PROBE_ASSERTED %0d\", dut.engine.rst);\n\
        \x20   repeat (3) utick;\n\
        \x20   #2 probe = 0; #1 $display(\"PROBE_RELEASED %0d\", dut.engine.rst);\n\
        \x20   #5 uclk = 1; #1 $display(\"PROBE_EDGE1 %0d\", dut.engine.rst);\n\
        \x20   #7 uclk = 0; #8 uclk = 1; #1 $display(\"PROBE_EDGE2 %0d\", dut.engine.rst);\n\
        \x20   $finish;\n\
        \x20 end\n\
        endmodule\n";
    let dir = workdir("user_clock2_top");
    let out = simulate(&dir, &[("stub.v", stub), ("top.v", top.to_string()), ("tb.v", tb.to_string())]);
    let got = |key: &str| -> i64 {
        out.lines()
            .find_map(|l| l.strip_prefix(&format!("{key} "))?.trim().parse().ok())
            .unwrap_or_else(|| panic!("no {key} in:\n{out}"))
    };
    assert_eq!(got("HELD"), 1, "wb_rst_i holds the engine in reset while user_clock2 runs");
    assert_eq!(got("RELEASED"), 1, "releasing wb_rst_i does not release the engine between its edges");
    assert_eq!(got("EDGE1"), 1, "nor on the first user_clock2 edge after");
    assert_eq!(got("EDGE2"), 0, "the second user_clock2 edge releases it");
    assert_eq!(got("WB_EDGES"), 0, "wb_clk_i edges with user_clock2 stopped do not clock the engine");
    assert_eq!(got("USER_EDGES"), 5, "every user_clock2 edge does");
    assert_eq!((got("ACK_BEFORE"), got("ACK_AFTER")), (0, 1), "the acknowledge comes on a wb_clk_i edge");
    assert_eq!(got("ASSERTED"), 1, "wb_rst_i resets the engine at once, with no clock running");
    assert_eq!(got("RUNNING"), 0, "and releases it two user_clock2 edges after it falls");
    assert_eq!(got("PROBE_UNDRIVEN"), 0, "probe 0 is no reset while the management SoC does not drive it");
    assert_eq!(got("PROBE_ASSERTED"), 1, "driven, it resets the engine at once");
    assert_eq!(got("PROBE_RELEASED"), 1, "and released, the engine stays in reset between its edges");
    assert_eq!(got("PROBE_EDGE1"), 1, "and on the first user_clock2 edge after");
    assert_eq!(got("PROBE_EDGE2"), 0, "and leaves it on the second");
}

/// The bench drives the macro the way the chip does: the engine's clock on `user_clock2` at the engine's period,
/// and `wb_clk_i` beside it at the period Caravel's chip signoff times the core clock at.
#[test]
fn the_bench_drives_both_clocks_at_their_periods() {
    let h = SKY130A.user_area.unwrap().harness.unwrap();
    let m = wpe_caravel_memory();
    let tb = emit_wpe_user_project_testbench(WPE_CARAVEL_NAME, 3, 64, &m, &WPE_CARAVEL, CARAVEL_IO_MAX);
    for conn in [".user_clock2(clk)", ".wb_clk_i(wb_clk)", ".wb_rst_i(rst)"] {
        assert_eq!(tb.matches(conn).count(), 1, "the bench connects {conn}");
    }
    assert_eq!(
        tb.matches(&format!("always #{} wb_clk = ~wb_clk;", h.core_clock_period_ns / 2.0)).count(),
        1,
        "wb_clk_i runs at the core clock's signoff period"
    );
    assert!(
        tb.contains(&format!("always #{} clk = ~clk;", WPE_CARAVEL.clk_period_ps as f64 / 2000.0)),
        "the engine's clock at its own period"
    );
}

/// The PSRAM read window is closed on the engine's clock: the STA script creates its clock on `user_clock2`, where
/// the edge's SCLK and sample flops are clocked.
#[test]
fn the_read_window_is_timed_on_the_engines_clock() {
    let tcl = wpe_psram_read_sta_tcl(&WPE_CARAVEL, &wpe_caravel_memory(), "ss.lib", "macro.v", "macro.max.spef");
    assert!(
        tcl.contains(&format!(
            "create_clock -name clk -period {} [get_ports {}]\n",
            WPE_CARAVEL.clk_period_ps as f64 / 1000.0,
            WPE_CLOCK.port()
        )),
        "{tcl}"
    );
    assert!(!tcl.contains("wb_clk_i"), "{tcl}");
}

// ── 3-4: programs through the pins ─────────────────────────────────────────────────────────────────────────

struct Wire {
    bytes: Vec<Vec<u8>>,
    results: Vec<i64>,
    shows: Vec<String>,
}

fn stage(dir: &Path, name: &str, w: &Wire) -> usize {
    let all: Vec<u8> = w.bytes.concat();
    std::fs::write(
        dir.join(format!("wire_{name}.hex")),
        all.iter().map(|b| format!("{b:02x}\n")).collect::<String>(),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("lens_{name}.hex")),
        w.bytes
            .iter()
            .map(|b| format!("{:08x}\n", b.len()))
            .collect::<String>(),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("oracle_{name}.hex")),
        w.results
            .iter()
            .map(|r| format!("{:016x}\n", *r as u64))
            .collect::<String>(),
    )
    .unwrap();
    all.len()
}

fn text_program() -> Wire {
    let src =
        "## Main\n    Let s be \"abc\".\n    Show item 2 of s.\n    Show s equals \"abc\".\n    Show \"n=\" + 42.\n";
    let cfg = program_main_fabric(src);
    let run = eval_wprog_config_run(&cfg, &[], &[]);
    Wire {
        bytes: vec![wprog_config_image(&cfg).wire_bytes(&[])],
        results: vec![run.result],
        shows: run
            .shows
            .iter()
            .map(|s| render_show(*s, &run.heap))
            .collect(),
    }
}

const LOADED_SUM: &str = "## To native loaded () -> Seq of Int\n## To sumw (n: Int) -> Int:\n    Let s be 0.\n    Let i be 0.\n    While i is less than n:\n        Set s to s + item (i + 1) of loaded().\n        Set i to i + 1.\n    Return s.\n";

fn preload_program() -> Wire {
    let cfg = serialize_program(&program_cfg_fabric(LOADED_SUM, "sumw"));
    let blob: Vec<i64> = std::iter::once(40)
        .chain((1..=40).map(|k| 7 * k - 100))
        .collect();
    let mut w = Wire {
        bytes: Vec::new(),
        results: Vec::new(),
        shows: Vec::new(),
    };
    for n in [40i64, 17] {
        w.bytes
            .push(wprog_config_image(&cfg).wire_bytes_preloaded(&[n], &blob));
        w.results
            .push(eval_wprog_config_counted_heap_preloaded(&cfg, &[n], &blob).0);
    }
    w
}

fn streamed(out: &str) -> Vec<String> {
    out.lines()
        .filter(|l| l.starts_with("SHOW") || l.starts_with("FRAME"))
        .map(str::to_string)
        .collect()
}

/// Run `w` on the Caravel user project (its RTL possibly rewritten by `mutate`) and on the on-chip engine.
fn through_the_pins(tag: &str, w: &Wire, mutate: impl Fn(String) -> String) -> (String, String) {
    let (m, c) = (memory(), caravel());
    let on_dir = workdir(&format!("{tag}_on"));
    let n = stage(&on_dir, tag, w);
    let on = simulate(
        &on_dir,
        &[
            (
                "engine.v",
                emit_wprog_engine_soc_verilog_backend(tag, CAPS, MemBackend::ConfigBram),
            ),
            (
                "tb.v",
                emit_wprog_engine_soc_testbench(tag, w.bytes.len(), c.clks_per_bit() as usize, n),
            ),
        ],
    );
    let dir = workdir(tag);
    stage(&dir, tag, w);
    let up = mutate(
        emit_wpe_user_project_verilog(tag, CAPS, MemBackend::ConfigBram, &m, &c)
            .expect("a wirable engine"),
    );
    let out = simulate(
        &dir,
        &[
            ("user_project.v", up),
            (
                "tb.v",
                emit_wpe_user_project_testbench(tag, w.bytes.len(), n, &m, &c, IDEAL),
            ),
        ],
    );
    (on, out)
}

#[test]
fn programs_run_through_the_caravel_pins_as_on_the_engine() {
    if !common::require_tool("iverilog") {
        return;
    }
    for (tag, w) in [
        ("pins_text", text_program()),
        ("pins_preload", preload_program()),
    ] {
        let (on, out) = through_the_pins(tag, &w, |s| s);
        assert!(
            on.contains("PASS:"),
            "{tag}: the on-chip engine diverged:\n{on}"
        );
        for fault in ["PIN-DIRECTION", "UART-LOST", "FAIL"] {
            assert!(
                !out.contains(fault),
                "{tag}: `{fault}` through the Caravel pins:\n{out}"
            );
        }
        assert!(
            out.contains("PASS:"),
            "{tag}: the engine through the pins diverged from the model:\n{out}"
        );
        assert!(
            out.contains("PSRAM-VIOLATIONS 0"),
            "{tag}: the part saw a rule broken:\n{out}"
        );
        assert_eq!(
            streamed(&out),
            streamed(&on),
            "{tag}: the pins streamed something other than the on-chip engine"
        );
        let shows: Vec<&str> = out.lines().filter(|l| l.starts_with("SHOW")).collect();
        assert_eq!(
            shows.is_empty(),
            w.shows.is_empty(),
            "{tag}: records streamed iff the model shows"
        );
    }
}

#[test]
fn the_pin_bench_catches_a_mis_wired_pin() {
    if !common::require_tool("iverilog") {
        return;
    }
    let w = text_program();
    let tx = 37 - WPE_PINS.uart_tx as usize;
    let (_, out) = through_the_pins("pins_tx_as_input", &w, |s| {
        let at = s.find("  assign io_oeb = {").expect("the io_oeb wiring");
        let end = at + s[at..].find('\n').unwrap();
        let mut bits: Vec<String> = s[at + "  assign io_oeb = {".len()..end - 2]
            .split(", ")
            .map(str::to_string)
            .collect();
        assert_eq!(bits[tx], "1'b0", "the transmit pin drives");
        bits[tx] = "1'b1".into();
        format!(
            "{}  assign io_oeb = {{{}}};{}",
            &s[..at],
            bits.join(", "),
            &s[end..]
        )
    });
    assert!(
        out.contains("PIN-DIRECTION"),
        "a transmit pin left an input must be caught:\n{out}"
    );

    // The preload's 41 weights overflow a heap cache of 32 words, so they must come back from the part: a bus
    // that never reaches it returns them wrong.
    let (on, out) = through_the_pins("pins_sio_enable_inverted", &preload_program(), |s| {
        assert_eq!(
            s.matches("~psram_sio_oe[").count(),
            4,
            "each SIO pad's enable is the edge's, inverted to io_oeb"
        );
        s.replace("~psram_sio_oe[", "psram_sio_oe[")
    });
    assert!(
        out.contains("PSRAM-VIOLATION float"),
        "an SIO pad driven only when the edge lets go leaves the part's inputs floating:\n{out}"
    );
    assert_ne!(
        streamed(&out),
        streamed(&on),
        "and the weights come back wrong"
    );
}

/// A heap access at or past HEAP stops the run and the frame ends `0xE2` instead of aliasing another word: a
/// preload longer than the 128-word heap is refused before the run, and a run that indexes past the heap stops
/// there — on the on-chip engine and through the Caravel pins alike.
#[test]
fn a_heap_access_past_the_heap_is_refused_on_die_and_off_it() {
    if !common::require_tool("iverilog") {
        return;
    }
    let cfg = serialize_program(&program_cfg_fabric(LOADED_SUM, "sumw"));
    let heap = CAPS[7];
    let vector = |n: i64, weights: usize| {
        let blob: Vec<i64> = std::iter::once(weights as i64)
            .chain((1..=weights as i64).map(|k| 3 * k - 7))
            .collect();
        (
            wprog_config_image(&cfg).wire_bytes_preloaded(&[n], &blob),
            eval_wprog_config_counted_heap_preloaded(&cfg, &[n], &blob).0,
        )
    };
    for (tag, n, weights) in [
        ("heap_trap_preload", 10, heap + 72),
        ("heap_trap_index", heap as i64 + 22, heap - 28),
    ] {
        let (bytes, result) = vector(n, weights);
        let w = Wire {
            bytes: vec![bytes],
            results: vec![result],
            shows: Vec::new(),
        };
        let (on, out) = through_the_pins(tag, &w, |s| s);
        for (side, text) in [("on die", &on), ("through the pins", &out)] {
            assert!(
                text.contains("end=e2"),
                "{tag} {side}: a heap access past HEAP={heap} must end the frame 0xE2:\n{text}"
            );
            assert!(
                !text.contains("PASS:"),
                "{tag} {side}: a refused run cannot pass"
            );
        }
    }
    let (bytes, result) = vector(40, 40);
    let (on, out) = through_the_pins(
        "heap_trap_inside",
        &Wire {
            bytes: vec![bytes],
            results: vec![result],
            shows: Vec::new(),
        },
        |s| s,
    );
    assert!(
        on.contains("PASS:") && out.contains("PASS:"),
        "a preload inside the heap still runs:\n{on}\n{out}"
    );
}

/// A program whose tables overflow the shipped profile is refused by the host before a byte is sent — here 70
/// distinct constants against 64 constant slots — and the refusal names the table; a program that fits is sent
/// as exactly the image the engine loads.
#[test]
fn the_host_refuses_a_program_the_shipped_profile_cannot_hold() {
    let mut src = String::from("## To f (x: Int) -> Int:\n    Let mutable y be x.\n");
    for k in 0..70 {
        src.push_str(&format!("    Set y to y * 3 + {}.\n", 1_000 + 7 * k));
    }
    src.push_str("    Return y.\n");
    let cfg = serialize_program(&program_cfg_fabric(&src, "f"));
    let refused =
        wpe_caravel_wire(&cfg, &[5], &[]).expect_err("70 constants overflow 64 constant slots");
    assert!(
        refused
            .iter()
            .any(|&(dim, need, cap)| dim == "MAXK" && need > cap && cap == WPE_CARAVEL_CAPS[6]),
        "{refused:?}"
    );
    let text = program_main_fabric(BRINGUP_TEXT);
    assert_eq!(
        wpe_caravel_wire(&text, &[], &[]).expect("the text program fits"),
        wprog_config_image(&text).wire_bytes(&[])
    );
}

// ── 5: the harness ─────────────────────────────────────────────────────────────────────────────────────────

/// Half an SCLK period at 21 ns, two engine cycles per half, is 42 ns; a read's round trip — 9.12 ns out through the
/// harness, 1 ns of board, tACLK 5.5 ns, 1 ns of board, 4.55 ns back in — is 21.17 ns. The same 21 ns clock with one
/// cycle per half leaves a 21 ns window the round trip overruns, although the wiring on its own is sound there: the
/// divider, not the clock, is what makes the reads fit.
#[test]
fn the_shipped_read_window_covers_the_harness_round_trip() {
    let m = wpe_caravel_memory();
    assert_eq!(WPE_CARAVEL.read_round_trip_ps(&m), 21_170);
    assert_eq!(WPE_CARAVEL.half_sclk_ps(&m), 42_000);
    WPE_CARAVEL
        .check_harness(&m)
        .expect("the shipped engine's reads fit the harness");
    let one_cycle_edge = PsramEdge::new(APS6404L_3SQN, 16_000, 1)
        .and_then(|e| e.clocked_down_to(40_000))
        .expect("one cycle per half is a valid edge at 16 ns");
    let one_cycle_cache = PsramCache::new(&one_cycle_edge, 3, 1).expect("eight two-word lines");
    let one_cycle = OffchipMemory {
        edge: one_cycle_edge,
        heap: Some(one_cycle_cache),
        frames: Some(one_cycle_cache),
        cells: Some(one_cycle_cache),
    };
    WPE_CARAVEL
        .check(&one_cycle)
        .expect("one cycle per half: the wiring alone is sound");
    assert_eq!(
        WPE_CARAVEL.check_harness(&one_cycle),
        Err(WpeCaravelUnplaceable::ReadWindowShortOfHarness {
            half_sclk_ps: 21_000,
            round_trip_ps: 21_170
        }),
        "a 21 ns half period is shorter than the round trip"
    );
    assert_eq!(
        CARAVEL_IO_MAX.with_board(BOARD_FLIGHT_MAX_PS),
        HarnessDelay {
            out_ps: 10_120,
            oeb_ps: 10_320,
            in_ps: 5_550
        }
    );
    for (lo, hi) in [
        (CARAVEL_IO_MIN.out_ps, CARAVEL_IO_MAX.out_ps),
        (CARAVEL_IO_MIN.oeb_ps, CARAVEL_IO_MAX.oeb_ps),
        (CARAVEL_IO_MIN.in_ps, CARAVEL_IO_MAX.in_ps),
    ] {
        assert!(
            0 < lo && lo < hi,
            "each harness path has a fastest and a slower slowest delay"
        );
    }
}

/// The read window's STA reads SCLK's arrival from the last point of its path: in this OpenSTA a path END has no
/// `arrival` property, and the script that asked it for one died at its first query against a routed macro
/// (RUN_2026.09.27_06.33.43). The capture slack is the path end's own. A failed STA run ends the check with its
/// output and a FATAL, never silently under `set -e`.
#[test]
fn the_read_window_sta_reads_the_arrival_its_version_has() {
    let m = wpe_caravel_memory();
    let tcl = wpe_psram_read_sta_tcl(&WPE_CARAVEL, &m, "ss.lib", "macro.v", "macro.max.spef");
    let sclk = format!("io_out[{}]", WPE_CARAVEL.pins.psram_sclk);
    assert!(
        tcl.contains(&format!(
            "set sclk_arrival [get_property [lindex [get_property [find_timing_paths -to [get_ports {{{sclk}}}] -path_delay max] points] end] arrival]"
        )),
        "SCLK's arrival comes from its path's last point:\n{tcl}"
    );
    assert!(
        !tcl.contains("-path_delay max] arrival]"),
        "a path end has no arrival property:\n{tcl}"
    );
    assert!(tcl
        .contains("set slack [get_property [find_timing_paths -from $sio -path_delay max] slack]"));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sh = std::fs::read_to_string(root.join("scripts/check-wpe-psram-timing.sh")).unwrap();
    let run = sh.find("STA_OUT=$(").expect("the STA run");
    let line = &sh[run..sh[run..].find('\n').map(|e| run + e).unwrap()];
    assert!(
        line.contains("|| {"),
        "a failed STA is caught where it runs, not by set -e:\n{line}"
    );
    assert!(sh[run..].contains("FATAL: OpenSTA failed"), "and reported");
}

/// Once the WPE macro is signed off, its read window is closed through the macro itself: a
/// `scripts/check-wpe-psram-timing.sh` record exists for the very GDS the Caravel macro ledger signs off, measured
/// at the slowest corner over max RC, with non-negative slack. The harness check above budgets everything outside
/// the macro; this is the part only a routed macro has.
#[test]
fn the_read_window_closes_through_the_signed_off_macro() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ledger: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join(WPE_PROJECT.macro_ledger))
            .expect("the Caravel macro ledger"),
    )
    .expect("JSON");
    let Some(signed) = ledger["blocks"].get(WPE_PROJECT.macro_module) else {
        return;
    };
    let timing: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("benchmarks/silicon/wpe_psram_timing.json")).expect(
            "the WPE macro is signed off, so its read window must be checked: scripts/check-wpe-psram-timing.sh",
        ),
    )
    .expect("JSON");
    assert_eq!(
        timing["gds_bytes"], signed["gds_bytes"],
        "the read window was checked on a different macro than the one signed off"
    );
    assert_eq!(
        timing["corner_lib"], "sky130_fd_sc_hd__ss_100C_1v60.lib",
        "the read window is closed at the slowest corner"
    );
    assert!(
        timing["spef"]
            .as_str()
            .is_some_and(|s| s.ends_with(".max.spef")),
        "over the max-RC parasitics"
    );
    let slack = timing["sio_capture_slack_ns"].as_f64().expect("a slack");
    assert!(
        slack >= 0.0 && timing["read_window_closed"] == true,
        "the PSRAM read window is violated by {slack} ns"
    );
}

// ── 6: the fabricated top cell ─────────────────────────────────────────────────────────────────────────────

/// The shipped macro and wrapper and the bring-up programs, staged with a bench driving the top cell through
/// `pads`; returns the simulation's output.
fn top_cell(tag: &str, n_vectors: Option<usize>, pads: HarnessDelay) -> String {
    let m = wpe_caravel_memory();
    let vectors = wpe_caravel_bringup_vectors();
    let vectors = &vectors[..n_vectors.unwrap_or(vectors.len())];
    let bytes: Vec<u8> = vectors
        .iter()
        .flat_map(|v| v.bytes.iter().copied())
        .collect();
    let dir = workdir(tag);
    let name = WPE_CARAVEL_NAME;
    std::fs::write(
        dir.join(format!("wire_{name}.hex")),
        bytes
            .iter()
            .map(|b| format!("{b:02x}\n"))
            .collect::<String>(),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("lens_{name}.hex")),
        vectors
            .iter()
            .map(|v| format!("{:08x}\n", v.bytes.len()))
            .collect::<String>(),
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("oracle_{name}.hex")),
        vectors
            .iter()
            .map(|v| format!("{:016x}\n", v.result as u64))
            .collect::<String>(),
    )
    .unwrap();
    let macro_rtl = emit_wpe_user_project_verilog(
        name,
        WPE_CARAVEL_CAPS,
        WPE_CARAVEL_BACKEND,
        &m,
        &WPE_CARAVEL,
    )
    .expect("the shipped engine wires");
    simulate(
        &dir,
        &[
            ("wpe_user_project.v", macro_rtl),
            ("user_project_wrapper.v", WPE_PROJECT.wrapper_verilog()),
            (
                "tb.v",
                emit_wpe_user_project_wrapper_testbench(
                    name,
                    vectors.len(),
                    bytes.len(),
                    &m,
                    &WPE_CARAVEL,
                    pads,
                ),
            ),
        ],
    )
}

fn assert_clean_run(tag: &str, out: &str, frames: usize) {
    for fault in ["PIN-DIRECTION", "UART-LOST", "FAIL", "PSRAM-VIOLATION "] {
        assert!(
            !out.contains(fault),
            "{tag}: `{fault}` on the fabricated top cell:\n{out}"
        );
    }
    assert!(
        out.contains("PASS:"),
        "{tag}: the top cell diverged from the model:\n{out}"
    );
    assert!(
        out.contains("PSRAM-VIOLATIONS 0"),
        "{tag}: the part saw a rule broken:\n{out}"
    );
    assert_eq!(
        out.lines().filter(|l| l.starts_with("FRAME")).count(),
        frames,
        "{tag}: a frame per program:\n{out}"
    );
}

/// The bench the submission ships: every program at the chip's clock, each PSRAM pin at the harness's slowest
/// delay plus the board's longest flight, and the text program's records exactly the model's.
#[test]
fn the_fabricated_top_cell_runs_the_bringup_programs_through_the_slowest_harness() {
    if !common::require_tool("iverilog") {
        return;
    }
    let out = top_cell(
        "top_slowest",
        None,
        CARAVEL_IO_MAX.with_board(BOARD_FLIGHT_MAX_PS),
    );
    assert_clean_run("top_slowest", &out, wpe_caravel_bringup_vectors().len());
    let text = program_main_fabric(BRINGUP_TEXT);
    let run = eval_wprog_config_run(&text, &[], &[]);
    let model: Vec<String> = run
        .shows
        .iter()
        .map(|s| render_show(*s, &run.heap))
        .collect();
    assert!(!model.is_empty(), "the text program shows");
    assert_eq!(
        rendered(&out),
        model,
        "the top cell's records are the model's, rendered as the host renders them:\n{out}"
    );
}

/// The records a run streamed, rendered as the host renders them: a `SHOW` line is one value, a `SHOWT` line
/// opens a text record whose `SHOWB` bytes follow.
fn rendered(out: &str) -> Vec<String> {
    let mut shows = Vec::new();
    let mut text: Option<(usize, Vec<u8>)> = None;
    for l in out.lines() {
        if let Some(rest) = l.strip_prefix("SHOW t=") {
            let (t, v) = rest.split_once(" v=").expect("a SHOW line");
            let value = u64::from_str_radix(v.trim(), 16).expect("a hex value") as i64;
            shows.push(render_show(
                WShow {
                    tag: t.parse().expect("a tag"),
                    value,
                },
                &[],
            ));
        } else if let Some(n) = l.strip_prefix("SHOWT len=") {
            match n.trim().parse::<usize>().expect("a text length") {
                0 => shows.push(String::new()),
                n => text = Some((n, Vec::new())),
            }
        } else if let Some(b) = l.strip_prefix("SHOWB ") {
            let (n, bytes) = text.as_mut().expect("SHOWB outside a text record");
            bytes.push(u8::from_str_radix(b.trim(), 16).expect("a hex byte"));
            if bytes.len() == *n {
                shows.push(String::from_utf8_lossy(bytes).into_owned());
                text = None;
            }
        }
    }
    shows
}

/// The same programs with every pin at the harness's fastest delay and no board: an early arrival breaks
/// nothing either.
#[test]
fn the_fabricated_top_cell_runs_the_bringup_programs_through_the_fastest_harness() {
    if !common::require_tool("iverilog") {
        return;
    }
    let out = top_cell("top_fastest", None, CARAVEL_IO_MIN);
    assert_clean_run("top_fastest", &out, wpe_caravel_bringup_vectors().len());
}

/// A round trip longer than the 32 ns window — 24 ns out, 20 ns back, with tACLK 49.5 ns — samples the part's
/// data before it arrives, and the weights the preload streams back from the part come back wrong.
#[test]
fn a_round_trip_longer_than_the_read_window_is_caught() {
    if !common::require_tool("iverilog") {
        return;
    }
    let out = top_cell(
        "top_too_slow",
        Some(2),
        HarnessDelay {
            out_ps: 24_000,
            oeb_ps: 24_200,
            in_ps: 20_000,
        },
    );
    assert!(
        !out.contains("PASS:") || out.contains("FAIL"),
        "a read sampled before its data arrives must not pass:\n{out}"
    );
}

// ── 7: the checked-in artifacts ────────────────────────────────────────────────────────────────────────────

/// Regenerate with `cargo run -p logicaffeine-compile --example emit_wpe_caravel`.
#[test]
fn the_checked_in_caravel_artifacts_are_the_emitters_output() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let silicon = root.join("benchmarks/silicon");
    let m = wpe_caravel_memory();
    let macro_rtl = emit_wpe_user_project_verilog(
        WPE_CARAVEL_NAME,
        WPE_CARAVEL_CAPS,
        WPE_CARAVEL_BACKEND,
        &m,
        &WPE_CARAVEL,
    )
    .unwrap();
    let mut expected = vec![
        (
            silicon.join(format!("{}.v", WPE_PROJECT.macro_module)),
            macro_rtl,
        ),
        (
            silicon.join(WPE_PROJECT.wrapper_rtl),
            WPE_PROJECT.wrapper_verilog(),
        ),
    ];
    for (path, body) in wpe_caravel_overlay_files() {
        expected.push((root.join(WPE_PROJECT.overlay).join(path), body));
    }
    let mut stale = Vec::new();
    for (path, body) in &expected {
        match std::fs::read_to_string(path) {
            Ok(on_disk) if &on_disk == body => {}
            Ok(_) => stale.push(format!("  {} differs", path.display())),
            Err(e) => stale.push(format!("  {} unreadable: {e}", path.display())),
        }
    }
    assert!(
        stale.is_empty(),
        "the WPE Caravel artifacts are stale — run the emit_wpe_caravel example:\n{}",
        stale.join("\n")
    );
    assert!(
        !silicon.join("wpe_user_defines.v").exists(),
        "the pad configuration lives in the overlay, verilog/rtl/user_defines.v"
    );
}
