//! The whole-program engine as a Caravel user project: the off-chip engine
//! ([`wpe_offchip`](super::wpe_offchip)) on the Caravel user interface, its UART and its PSRAM on user
//! GPIOs, clocked by `user_clock2` ([`WPE_CLOCK`]) and reset by the management SoC.
//!
//! One pin plan ([`WpePins`]) is the source of every place a pin appears: the macro's `io_out`/`io_oeb`
//! wiring, the power-on GPIO modes of `user_defines.v`, and the testbench that runs a program through the
//! Caravel pins against the part's pin model. The UART's bit period is derived from the clock and the baud
//! rate and checked against both the UART's tolerance and the off-chip memories' loading stalls.

use super::fabric_rtl::program_cfg_fabric;
use super::pdk_target::{HarnessClock, SKY130A};
use super::psram::{PsramCache, PsramEdge, APS6404L_3SQN};
use super::wpe_floorplan::{WpeDemands, WpeProfile};
use super::wpe_offchip::{
    emit_wprog_engine_soc_offchip_testbench_clocked, emit_wprog_engine_soc_verilog_offchip,
    OffchipMemory, OffchipUnplaceable,
};
use super::wprog_engine_rtl::{
    eval_wprog_config_counted_heap_preloaded, eval_wprog_config_run, program_main_fabric,
    serialize_program, wprog_config_image_unchecked, MemBackend, WProgConfig,
};

/// Caravel's user GPIOs: `io_in`/`io_out`/`io_oeb[37:0]`, of which 0..=4 carry the housekeeping SPI and JTAG.
pub const CARAVEL_IO: u8 = 38;
pub const CARAVEL_FIRST_USER_IO: u8 = 5;

/// Where the engine's pins land on Caravel's user GPIOs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WpePins {
    pub uart_rx: u8,
    pub uart_tx: u8,
    pub psram_ce_n: u8,
    pub psram_sclk: u8,
    pub psram_sio: [u8; 4],
}

/// The shipped plan: the UART on GPIO 8 and 9, the PSRAM's CE#, SCLK and `SIO[3:0]` on 10..=15 — clear of
/// the housekeeping pins and of the management UART on 5 and 6.
pub const WPE_PINS: WpePins = WpePins {
    uart_rx: 8,
    uart_tx: 9,
    psram_ce_n: 10,
    psram_sclk: 11,
    psram_sio: [12, 13, 14, 15],
};

/// How a GPIO comes up at power-on, as `user_defines.v` names the mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpioMode {
    Input,
    Output,
    Bidirectional,
}

impl GpioMode {
    pub fn define(&self) -> &'static str {
        match self {
            GpioMode::Input => "GPIO_MODE_USER_STD_INPUT_NOPULL",
            GpioMode::Output => "GPIO_MODE_USER_STD_OUTPUT",
            GpioMode::Bidirectional => "GPIO_MODE_USER_STD_BIDIRECTIONAL",
        }
    }
}

/// Why the engine cannot be wired to Caravel as asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WpeCaravelUnplaceable {
    /// A pin is a housekeeping pin or past the last GPIO.
    PinNotUser { pin: u8 },
    /// Two signals share a pin.
    PinShared { pin: u8 },
    /// The clock cannot make the baud rate within a UART's tolerance.
    BaudUnreachable {
        baud: u32,
        clks_per_bit: u64,
        error_ppm: u64,
    },
    /// The engine's clock lies outside the range the PSRAM edge is timed for.
    ClockOutsideEdgeRange {
        engine_ps: u32,
        fastest_ps: u32,
        slowest_ps: u32,
    },
    /// Half an SCLK period at the engine's clock does not cover a read's round trip through the harness and
    /// the board: SCLK out to the part, the part's tACLK, the data back in.
    ReadWindowShortOfHarness {
        half_sclk_ps: u64,
        round_trip_ps: u64,
    },
    /// The off-chip memories refuse the configuration or the UART.
    Memory(OffchipUnplaceable),
}

/// A UART tolerates a little over 2% of accumulated error across a frame, split between both ends.
pub const UART_MAX_ERROR_PPM: u64 = 10_000;

/// Caravel's harness between a user-area port and its pad, as efabless characterizes it for user projects: the
/// `IO_SYNC` budget of the scaffold's `base_user_project_wrapper.sdc`, generated from Caravel's own STA by
/// `get_cup_sdc.py`. `io_out` reaches its pad in 3.90..=9.12 ns, `io_oeb` in 2.34..=9.32 ns, and a pad
/// reaches `io_in` in 1.26..=4.55 ns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HarnessDelay {
    pub out_ps: u64,
    pub oeb_ps: u64,
    pub in_ps: u64,
}

pub const CARAVEL_IO_MAX: HarnessDelay = HarnessDelay {
    out_ps: 9_120,
    oeb_ps: 9_320,
    in_ps: 4_550,
};
pub const CARAVEL_IO_MIN: HarnessDelay = HarnessDelay {
    out_ps: 3_900,
    oeb_ps: 2_340,
    in_ps: 1_260,
};

impl HarnessDelay {
    /// The delay from a user-area port to the part's pin and back, with `board_ps` of trace each way.
    pub fn with_board(self, board_ps: u64) -> HarnessDelay {
        HarnessDelay {
            out_ps: self.out_ps + board_ps,
            oeb_ps: self.oeb_ps + board_ps,
            in_ps: self.in_ps + board_ps,
        }
    }
}

/// The carrier board's trace between a pad and the part, each way: at most 1 ns, about 15 cm of FR-4. A
/// requirement the board must meet, not a property of the die.
pub const BOARD_FLIGHT_MAX_PS: u64 = 1_000;

impl WpePins {
    /// Every used pin with its mode.
    pub fn used(&self) -> Vec<(u8, GpioMode)> {
        let mut v = vec![
            (self.uart_rx, GpioMode::Input),
            (self.uart_tx, GpioMode::Output),
            (self.psram_ce_n, GpioMode::Output),
            (self.psram_sclk, GpioMode::Output),
        ];
        v.extend(self.psram_sio.iter().map(|&p| (p, GpioMode::Bidirectional)));
        v
    }

    /// Refuse a housekeeping pin, a pin past the last GPIO, or a pin used twice.
    pub fn check(&self) -> Result<(), WpeCaravelUnplaceable> {
        let used = self.used();
        for (i, &(pin, _)) in used.iter().enumerate() {
            if !(CARAVEL_FIRST_USER_IO..CARAVEL_IO).contains(&pin) {
                return Err(WpeCaravelUnplaceable::PinNotUser { pin });
            }
            if used[..i].iter().any(|&(p, _)| p == pin) {
                return Err(WpeCaravelUnplaceable::PinShared { pin });
            }
        }
        Ok(())
    }

    /// The power-on mode of every user GPIO: the plan's pins as they are used, every other one an input.
    pub fn gpio_modes(&self) -> Vec<(u8, GpioMode)> {
        (CARAVEL_FIRST_USER_IO..CARAVEL_IO)
            .map(|pin| {
                (
                    pin,
                    self.used()
                        .iter()
                        .find(|(p, _)| *p == pin)
                        .map_or(GpioMode::Input, |&(_, m)| m),
                )
            })
            .collect()
    }
}

/// The engine on Caravel: its clock period, the host's baud rate, and the pin plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WpeCaravel {
    pub clk_period_ps: u32,
    pub baud: u32,
    pub pins: WpePins,
}

impl WpeCaravel {
    /// System cycles per UART bit, the nearest the clock can make the baud rate.
    pub fn clks_per_bit(&self) -> u64 {
        let cycles_per_second = 1_000_000_000_000u64 / self.clk_period_ps as u64;
        (cycles_per_second + self.baud as u64 / 2) / self.baud as u64
    }

    /// How far that bit period is from the baud rate's, in parts per million.
    pub fn baud_error_ppm(&self) -> u64 {
        let ideal_ps = 1_000_000_000_000u64 / self.baud as u64;
        let actual_ps = self.clks_per_bit() * self.clk_period_ps as u64;
        ideal_ps.abs_diff(actual_ps) * 1_000_000 / ideal_ps
    }

    /// A read's round trip from the edge's SCLK flop to its sample flop, outside the macro: the falling edge
    /// crosses the harness and the board, the part drives after tACLK, and the data crosses back. The edge
    /// samples half an SCLK period after that falling edge, so the half period must cover it.
    pub fn read_round_trip_ps(&self, memory: &OffchipMemory) -> u64 {
        CARAVEL_IO_MAX.out_ps
            + BOARD_FLIGHT_MAX_PS
            + memory.edge.part.t_aclk_max_ps as u64
            + BOARD_FLIGHT_MAX_PS
            + CARAVEL_IO_MAX.in_ps
    }

    /// Half an SCLK period at the engine's clock.
    pub fn half_sclk_ps(&self, memory: &OffchipMemory) -> u64 {
        memory.edge.half as u64 * self.clk_period_ps as u64
    }

    /// Refuse pins, a baud rate the clock cannot make, a clock the edge is not timed for, or a UART the
    /// off-chip memories could lose bytes on. These are the wiring's own rules, which hold in simulation with
    /// ideal pins; [`WpeCaravel::check_harness`] adds the rule the chip's harness imposes.
    pub fn check(&self, memory: &OffchipMemory) -> Result<(), WpeCaravelUnplaceable> {
        self.pins.check()?;
        if self.baud_error_ppm() > UART_MAX_ERROR_PPM {
            return Err(WpeCaravelUnplaceable::BaudUnreachable {
                baud: self.baud,
                clks_per_bit: self.clks_per_bit(),
                error_ppm: self.baud_error_ppm(),
            });
        }
        let (fastest_ps, slowest_ps) = (memory.edge.sys_period_ps, memory.edge.slowest_period_ps);
        if !(fastest_ps..=slowest_ps).contains(&self.clk_period_ps) {
            return Err(WpeCaravelUnplaceable::ClockOutsideEdgeRange {
                engine_ps: self.clk_period_ps,
                fastest_ps,
                slowest_ps,
            });
        }
        memory
            .check_uart(self.clks_per_bit())
            .map_err(WpeCaravelUnplaceable::Memory)
    }

    /// [`WpeCaravel::check`], and a read window the round trip through Caravel's harness and the board fits:
    /// the configuration a chip can be built with.
    pub fn check_harness(&self, memory: &OffchipMemory) -> Result<(), WpeCaravelUnplaceable> {
        self.check(memory)?;
        let (half_sclk_ps, round_trip_ps) =
            (self.half_sclk_ps(memory), self.read_round_trip_ps(memory));
        if half_sclk_ps <= round_trip_ps {
            return Err(WpeCaravelUnplaceable::ReadWindowShortOfHarness {
                half_sclk_ps,
                round_trip_ps,
            });
        }
        Ok(())
    }
}

/// The engine the Caravel user project carries: `wpe_soc_caravel`.
pub const WPE_CARAVEL_NAME: &str = "caravel";

/// The shipped cap profile, `[NREG, MAXBLK, MAXCELL, MAXWR, MAXARG, MAXLIV, MAXK, HEAP, DEPTH, MAXV]`.
///
/// The three memories that live in the part take the measured suite's ceilings: 1072 cells, 32 frames, and a
/// 32768-word heap — the heap region is zeroed at every program load, one line write-back at a time, so its
/// size costs load latency rather than die area. The on-die tables are flip-flops, so
/// they are sized to what the suite's programs need: every program whose per-block cells fit `MAXV` = 64 and
/// whose heap fits the region demands at most 29 blocks, 104 writes and 52 live-ins (`graph_bfs`), and the
/// tables take those with headroom. The per-block cell values stay at 64 — `nbody`, `strings` and `sha256`
/// need 223 to 450, the one array whose growth would not fit the user area.
pub const WPE_CARAVEL_CAPS: [usize; 10] = [48, 32, 1072, 112, 16, 56, 64, 32_768, 32, 64];

/// The config image reaches the walk through a registered port only under the config-in-BRAM backend.
pub const WPE_CARAVEL_BACKEND: MemBackend = MemBackend::ConfigBram;

/// The harness clock the engine runs on. Caravel's chip signoff times the core clock, `wb_clk_i`, at 25 ns, and the
/// management SoC and the bus run on it; `user_clock2` is the PLL's 90-degree phase through a divider of its own, so
/// the engine runs at its own period while the management SoC stays at the one the chip is signed for.
pub const WPE_CLOCK: HarnessClock = HarnessClock::User2;

/// The shipped wiring: the pin plan, the 21 ns clock (47.62 MHz on `user_clock2`) and a 115200-baud host.
///
/// The clock is the engine's own. Every float unit, the integer multiplier and the conversions are staged so no
/// stage holds more than one carry chain, leading-one search, barrel shift or compressor tree, and the macro is
/// hardened at this period (`CaravelProject::macro_period_ns`). The period is what the engine's slow corner meets
/// (`signoff_blocks::honest_period_ns`): hardened at 18.5 ns and routed, its paths through a compare chain into the
/// control and out through a fanout tree missed setup at ss_100C_1v60 by 1.13 ns on extracted parasitics, where the
/// routing stage's repairs had closed on global-route estimates; hardened at 16 ns, the paths from the frame cache's
/// hit into the stall that freezes every register, and from `blk_term_kind_r`, had missed by 1.48 ns. The off-chip
/// memory follows it: the edge runs two engine cycles per SCLK half, so the 42 ns half period still covers a read's
/// round trip through the harness and the board ([`WpeCaravel::read_round_trip_ps`]).
pub const WPE_CARAVEL: WpeCaravel = WpeCaravel {
    clk_period_ps: 21_000,
    baud: 115_200,
    pins: WPE_PINS,
};

/// The shipped off-chip memories: the edge's cycle counts are timed at 16 ns with two engine cycles per SCLK
/// half (a 64 ns SCLK), so every slower clock meets tPU, tRST, tCPH, tCLK and tACLK too, and its bursts are cut
/// for 40 ns, where tCEM leaves two words a line; heap, frame stack and config image each get eight such lines.
pub fn wpe_caravel_memory() -> OffchipMemory {
    let edge = PsramEdge::new(APS6404L_3SQN, 16_000, 2)
        .and_then(|e| e.clocked_down_to(40_000))
        .expect("the part runs from 16 to 40 ns");
    let cache = PsramCache::new(&edge, 3, 1).expect("eight two-word lines");
    OffchipMemory {
        edge,
        heap: Some(cache),
        frames: Some(cache),
        cells: Some(cache),
    }
}

/// The full-interface Caravel user macro `wpe_user_project`: the engine `wpe_soc_<name>` with `memory`'s
/// memories off-chip, clocked by [`WPE_CLOCK`], its UART and PSRAM on `caravel.pins` (a bidirectional SIO pad is
/// driven exactly while the edge drives it: `io_oeb` is its active-low enable), every other GPIO an input.
///
/// The engine's reset crosses into its clock domain through two flops, asserted at once and released on the second
/// edge after its source falls. The source is `wb_rst_i`, or logic-analyzer probe 0 while the management SoC drives
/// it (`la_oenb[0]` low, efabless's override in `user_proj_example.v`): at power-on the PLL is bypassed and
/// `user_clock2` is the pad clock, `caravel_clocking.v` moves it to the PLL with a plain select between the two, and
/// the firmware resets the engine after that switch, which a chip reset cannot do without bypassing the PLL again.
/// The Wishbone slave runs on `wb_clk_i`, acknowledges every access and reads zero, so the management SoC never
/// waits on the user area; the rest of the logic analyzer and the interrupts are unused.
pub fn emit_wpe_user_project_verilog(
    name: &str,
    caps: [usize; 10],
    backend: MemBackend,
    memory: &OffchipMemory,
    caravel: &WpeCaravel,
) -> Result<String, WpeCaravelUnplaceable> {
    caravel.check(memory)?;
    // The user macro is an ASIC build, so its multiplies take the compressor tree: the engine's MUL_TREE default
    // bakes to 1, the way a config-BRAM engine's CFG_BRAM does.
    let engine = emit_wprog_engine_soc_verilog_offchip(name, caps, backend, memory)
        .map_err(WpeCaravelUnplaceable::Memory)?
        .replacen("parameter MUL_TREE = 0", "parameter MUL_TREE = 1", 1);
    let p = &caravel.pins;
    let (mut outs, mut oebs) = (Vec::new(), Vec::new());
    for pin in (0..CARAVEL_IO).rev() {
        let (out, oeb) = if pin == p.uart_tx {
            ("uart_tx".to_string(), "1'b0".to_string())
        } else if pin == p.psram_ce_n {
            ("psram_ce_n".to_string(), "1'b0".to_string())
        } else if pin == p.psram_sclk {
            ("psram_sclk".to_string(), "1'b0".to_string())
        } else if let Some(k) = p.psram_sio.iter().position(|&s| s == pin) {
            (format!("psram_sio_out[{k}]"), format!("~psram_sio_oe[{k}]"))
        } else {
            ("1'b0".to_string(), "1'b1".to_string())
        };
        outs.push(out);
        oebs.push(oeb);
    }
    let sio_in = p
        .psram_sio
        .iter()
        .rev()
        .map(|s| format!("io_in[{s}]"))
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "{engine}\n\
         // wpe_user_project — the whole-program engine as a Caravel user macro (codegen_sva::wpe_caravel). UART rx/tx\n\
         // on GPIO {rx}/{tx}; PSRAM CE# {ce}, SCLK {sclk}, SIO[3:0] {s0}..{s3}; engine clock {clk} ({period} ps), {baud}\n\
         // baud ({cpb} cycles per bit); the Wishbone slave on wb_clk_i.\n\
         module wpe_user_project (\n\
         `ifdef USE_POWER_PINS\n\
         \x20 inout         vccd1,\n\
         \x20 inout         vssd1,\n\
         `endif\n\
         \x20 input         wb_clk_i,\n\
         \x20 input         wb_rst_i,\n\
         \x20 input         wbs_stb_i,\n\
         \x20 input         wbs_cyc_i,\n\
         \x20 input         wbs_we_i,\n\
         \x20 input  [3:0]  wbs_sel_i,\n\
         \x20 input  [31:0] wbs_dat_i,\n\
         \x20 input  [31:0] wbs_adr_i,\n\
         \x20 output        wbs_ack_o,\n\
         \x20 output [31:0] wbs_dat_o,\n\
         \x20 input  [127:0] la_data_in,\n\
         \x20 output [127:0] la_data_out,\n\
         \x20 input  [127:0] la_oenb,\n\
         \x20 input  [37:0]  io_in,\n\
         \x20 output [37:0]  io_out,\n\
         \x20 output [37:0]  io_oeb,\n\
         \x20 input         user_clock2,\n\
         \x20 output [2:0]  user_irq\n\
         );\n\
         \x20 wire uart_tx, psram_ce_n, psram_sclk;\n\
         \x20 wire [3:0] psram_sio_out, psram_sio_oe;\n\
         \x20 // The engine's reset: wb_rst_i, or probe 0 while the management SoC drives it, into the engine's clock\n\
         \x20 // domain through two flops, asserted at once and released on the second {clk} edge after it falls.\n\
         \x20 wire engine_rst_req = wb_rst_i | (la_data_in[0] & ~la_oenb[0]);\n\
         \x20 reg [1:0] engine_rst;\n\
         \x20 always @(posedge {clk} or posedge engine_rst_req)\n\
         \x20   if (engine_rst_req) engine_rst <= 2'b11;\n\
         \x20   else engine_rst <= {{engine_rst[0], 1'b0}};\n\
         \x20 wpe_soc_{name} #(.CLKS_PER_BIT({cpb})) engine (\n\
         \x20   .clk({clk}), .rst(engine_rst[1]), .uart_rx(io_in[{rx}]), .uart_tx(uart_tx),\n\
         \x20   .psram_ce_n(psram_ce_n), .psram_sclk(psram_sclk), .psram_sio_out(psram_sio_out), .psram_sio_oe(psram_sio_oe),\n\
         \x20   .psram_sio_in({{{sio_in}}}));\n\
         \x20 assign io_out = {{{outs}}};\n\
         \x20 assign io_oeb = {{{oebs}}};\n\
         \x20 // Every Wishbone access is acknowledged the cycle after it is presented and reads zero.\n\
         \x20 reg ack;\n\
         \x20 always @(posedge wb_clk_i) ack <= !wb_rst_i && wbs_cyc_i && wbs_stb_i && !ack;\n\
         \x20 assign wbs_ack_o = ack;\n\
         \x20 assign wbs_dat_o = 32'd0;\n\
         \x20 assign la_data_out = 128'd0;\n\
         \x20 assign user_irq = 3'd0;\n\
         \x20 // The rest of the fixed interface is kept and connected so the wrapper wires every pin.\n\
         \x20 wire _unused = &{{1'b0, la_data_in, la_oenb, wbs_we_i, wbs_sel_i, wbs_dat_i, wbs_adr_i, io_in}};\n\
         endmodule\n",
        rx = p.uart_rx,
        tx = p.uart_tx,
        ce = p.psram_ce_n,
        sclk = p.psram_sclk,
        s0 = p.psram_sio[0],
        s3 = p.psram_sio[3],
        period = caravel.clk_period_ps,
        baud = caravel.baud,
        cpb = caravel.clks_per_bit(),
        clk = WPE_CLOCK.port(),
        outs = outs.join(", "),
        oebs = oebs.join(", "),
    ))
}

/// The period Caravel's chip signoff times its core clock at, in ns: the clock the management SoC runs on.
fn core_clock_period_ns() -> f64 {
    SKY130A
        .user_area
        .and_then(|ua| ua.harness)
        .expect("Caravel's harness is characterized")
        .core_clock_period_ns
}

/// `user_defines.v`: every user GPIO's power-on mode, from the pin plan.
pub fn emit_wpe_user_defines(pins: &WpePins) -> String {
    let mut s = String::from(
        "// user_defines.v — user GPIO power-on configuration for the LOGOS whole-program engine\n\
         // (codegen_sva::wpe_caravel): the UART and PSRAM pins as the engine uses them, every other GPIO a\n\
         // no-pull input.\n\
         \n\
         `default_nettype none\n\
         \n\
         `ifndef __USER_DEFINES_H\n\
         `define __USER_DEFINES_H\n\
         \n\
         `define GPIO_MODE_INVALID                  13'hXXXX\n\
         \n\
         `define GPIO_MODE_MGMT_STD_INPUT_NOPULL    13'h0403\n\
         `define GPIO_MODE_MGMT_STD_INPUT_PULLDOWN  13'h0c01\n\
         `define GPIO_MODE_MGMT_STD_INPUT_PULLUP    13'h0801\n\
         `define GPIO_MODE_MGMT_STD_OUTPUT          13'h1809\n\
         `define GPIO_MODE_MGMT_STD_BIDIRECTIONAL   13'h1801\n\
         `define GPIO_MODE_MGMT_STD_ANALOG          13'h000b\n\
         \n\
         `define GPIO_MODE_USER_STD_INPUT_NOPULL    13'h0402\n\
         `define GPIO_MODE_USER_STD_INPUT_PULLDOWN  13'h0c00\n\
         `define GPIO_MODE_USER_STD_INPUT_PULLUP    13'h0800\n\
         `define GPIO_MODE_USER_STD_OUTPUT          13'h1808\n\
         `define GPIO_MODE_USER_STD_BIDIRECTIONAL   13'h1800\n\
         `define GPIO_MODE_USER_STD_OUT_MONITORED   13'h1802\n\
         `define GPIO_MODE_USER_STD_ANALOG          13'h000a\n\
         \n",
    );
    for (pin, mode) in pins.gpio_modes() {
        s.push_str(&format!(
            "`define USER_CONFIG_GPIO_{pin}_INIT `{}\n",
            mode.define()
        ));
    }
    s.push_str("\n`endif // __USER_DEFINES_H\n`default_nettype wire\n");
    s
}

/// The off-chip engine's host testbench, driving `wpe_user_project` through its Caravel pins at the engine's
/// clock: the UART on the plan's GPIOs, the part's pin model on the PSRAM GPIOs (a pad driven exactly while
/// its `io_oeb` is low), every PSRAM pin reaching the part `pads` after the macro drives it and the part's
/// data reaching `io_in` `pads.in_ps` after the part drives it, the management SoC's clock and reset, and an
/// idle Wishbone. A pin driven the wrong way at any point after reset fails the run with `PIN-DIRECTION`.
pub fn emit_wpe_user_project_testbench(
    name: &str,
    n_vectors: usize,
    max_bytes: usize,
    memory: &OffchipMemory,
    caravel: &WpeCaravel,
    pads: HarnessDelay,
) -> String {
    let p = &caravel.pins;
    let board = emit_wprog_engine_soc_offchip_testbench_clocked(
        name,
        n_vectors,
        caravel.clks_per_bit() as usize,
        max_bytes,
        memory,
        caravel.clk_period_ps,
    );
    let dut = format!(
        "  wpe_soc_{name} #(.CLKS_PER_BIT(CLKS)) dut(.clk(clk), .rst(rst), .uart_rx(uart_rx), .uart_tx(uart_tx),\n\
         \x20   .psram_ce_n(psram_ce_n), .psram_sclk(psram_sclk), .psram_sio_out(psram_sio_out), .psram_sio_oe(psram_sio_oe),\n\
         \x20   .psram_sio_in(psram_sio));\n"
    );
    assert_eq!(
        board.matches(dut.as_str()).count(),
        1,
        "the board testbench instantiates the engine once"
    );
    let ns = |ps: u64| format!("{}.{:03}", ps / 1000, ps % 1000);
    let (d_out, d_oeb, d_in) = (ns(pads.out_ps), ns(pads.oeb_ps), ns(pads.in_ps));
    let sio_in = p
        .psram_sio
        .iter()
        .enumerate()
        .map(|(k, s)| format!("  assign #({d_in}) io_in[{s}] = psram_sio[{k}];\n"))
        .collect::<String>();
    let sio_out = p
        .psram_sio
        .iter()
        .enumerate()
        .map(|(k, s)| {
            format!("  assign #({d_out}) psram_sio_out[{k}] = io_out[{s}]; assign #({d_oeb}) psram_sio_oe[{k}] = ~io_oeb[{s}];\n")
        })
        .collect::<String>();
    let outputs = [p.uart_tx, p.psram_ce_n, p.psram_sclk];
    let inputs: Vec<u8> = (0..CARAVEL_IO)
        .filter(|pin| !outputs.contains(pin) && !p.psram_sio.contains(pin))
        .collect();
    let directions = format!(
        "  // After reset every output pin drives and every other non-SIO pin is an input.\n\
         \x20 always @(posedge clk) if (!rst && ({outs_oeb} || ~&{{{ins_oeb}}})) begin\n\
         \x20   $display(\"PIN-DIRECTION io_oeb=%b\", io_oeb);\n\
         \x20   $display(\"FAIL: a Caravel pin is driven the wrong way\");\n\
         \x20   $finish;\n\
         \x20 end\n",
        outs_oeb = outputs
            .iter()
            .map(|o| format!("io_oeb[{o}]"))
            .collect::<Vec<_>>()
            .join(" || "),
        ins_oeb = inputs
            .iter()
            .map(|i| format!("io_oeb[{i}]"))
            .collect::<Vec<_>>()
            .join(", "),
    );
    let user = format!(
        "  wire [37:0] io_out, io_oeb; wire [37:0] io_in;\n\
         \x20 genvar hk;\n\
         \x20 generate for (hk = 0; hk < 38; hk = hk + 1) begin : idle_io\n\
         \x20   if (hk != {rx} && hk != {s0} && hk != {s1} && hk != {s2} && hk != {s3}) begin : quiet\n\
         \x20     assign io_in[hk] = 1'b0;\n\
         \x20   end\n\
         \x20 end endgenerate\n\
         \x20 assign io_in[{rx}] = uart_rx;\n\
         {sio_in}\
         \x20 assign uart_tx = io_out[{tx}];\n\
         \x20 assign #({d_out}) psram_ce_n = io_out[{ce}];\n\
         \x20 assign #({d_out}) psram_sclk = io_out[{sclk}];\n\
         {sio_out}\
         \x20 wire wbs_ack_o; wire [31:0] wbs_dat_o; wire [127:0] la_data_out; wire [2:0] user_irq;\n\
         \x20 // The management SoC's clock, at the period Caravel's chip signoff times it at; the engine's is clk.\n\
         \x20 reg wb_clk = 0;\n\
         \x20 always #{wb_half} wb_clk = ~wb_clk;\n\
         \x20 wpe_user_project dut (\n\
         \x20   .wb_clk_i(wb_clk), .wb_rst_i(rst), .wbs_stb_i(1'b0), .wbs_cyc_i(1'b0), .wbs_we_i(1'b0), .wbs_sel_i(4'd0),\n\
         \x20   .wbs_dat_i(32'd0), .wbs_adr_i(32'd0), .wbs_ack_o(wbs_ack_o), .wbs_dat_o(wbs_dat_o),\n\
         \x20   .la_data_in(128'd0), .la_data_out(la_data_out), .la_oenb({{128{{1'b1}}}}),\n\
         \x20   .io_in(io_in), .io_out(io_out), .io_oeb(io_oeb), .{clk}(clk), .user_irq(user_irq));\n\
         {directions}",
        rx = p.uart_rx,
        tx = p.uart_tx,
        ce = p.psram_ce_n,
        sclk = p.psram_sclk,
        s0 = p.psram_sio[0],
        s1 = p.psram_sio[1],
        s2 = p.psram_sio[2],
        s3 = p.psram_sio[3],
        wb_half = core_clock_period_ns() / 2.0,
        clk = WPE_CLOCK.port(),
    );
    board
        .replace(&dut, &user)
        .replace("dut.mstall", "dut.engine.mstall")
        .replace("dut.rx_valid", "dut.engine.rx_valid")
        .replace("dut.r_state", "dut.engine.r_state")
        .replace("dut.r_cnt", "dut.engine.r_cnt")
}

/// [`emit_wpe_user_project_testbench`] driving the cell that is fabricated: Caravel's `user_project_wrapper`,
/// the engine reached through its `mprj`.
pub fn emit_wpe_user_project_wrapper_testbench(
    name: &str,
    n_vectors: usize,
    max_bytes: usize,
    memory: &OffchipMemory,
    caravel: &WpeCaravel,
    pads: HarnessDelay,
) -> String {
    let tb = emit_wpe_user_project_testbench(name, n_vectors, max_bytes, memory, caravel, pads);
    assert_eq!(
        tb.matches("  wpe_user_project dut (").count(),
        1,
        "the macro testbench instantiates the macro once"
    );
    tb.replace("  wpe_user_project dut (", "  user_project_wrapper dut (")
        .replace("dut.engine.", "dut.mprj.engine.")
}

/// One program of the bring-up bench: the bytes the host sends and the result the model computes for them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BringupVector {
    pub bytes: Vec<u8>,
    pub result: i64,
}

/// Text built, compared and shown over the wire.
pub const BRINGUP_TEXT: &str = "## Main\n    Let s be \"abc\".\n    Show item 2 of s.\n    Show s equals \"abc\".\n    Show \"n=\" + 42.\n";

/// A weighted sum over a streamed preload of 40 weights: 41 heap words, more than a heap cache of eight two-word
/// lines holds, so the weights come back from the part.
pub const BRINGUP_PRELOAD: &str = "## To native loaded () -> Seq of Int\n## To sumw (n: Int) -> Int:\n    Let s be 0.\n    Let i be 0.\n    While i is less than n:\n        Set s to s + item (i + 1) of loaded().\n        Set i to i + 1.\n    Return s.\n";

/// The bytes a host sends the Caravel chip to run `cfg` on `params` with `heap_init` streamed ahead of the run,
/// or every table of the shipped profile the program overflows as (engine parameter, demand, cap). The chip
/// traps a call deeper than its frame stack and a heap access past its region on its own; every other table is
/// checked here, before anything is sent, because an overflowing table would be loaded into the wrong words.
pub fn wpe_caravel_wire(
    cfg: &WProgConfig,
    params: &[i64],
    heap_init: &[i64],
) -> Result<Vec<u8>, Vec<(&'static str, usize, usize)>> {
    let short = WpeProfile::from_caps(WPE_CARAVEL_CAPS).shortfalls(&WpeDemands::loaded(cfg));
    if !short.is_empty() {
        return Err(short);
    }
    Ok(wprog_config_image_unchecked(cfg).wire_bytes_preloaded(params, heap_init))
}

/// The bring-up bench's programs, in the order it sends them: the text program, then the preload summed over
/// all 40 weights and over the first 17.
pub fn wpe_caravel_bringup_vectors() -> Vec<BringupVector> {
    let wire = |cfg: &WProgConfig, params: &[i64], heap_init: &[i64]| {
        wpe_caravel_wire(cfg, params, heap_init)
            .unwrap_or_else(|short| panic!("a bring-up program overflows {short:?}"))
    };
    let text = program_main_fabric(BRINGUP_TEXT);
    let mut v = vec![BringupVector {
        bytes: wire(&text, &[], &[]),
        result: eval_wprog_config_run(&text, &[], &[]).result,
    }];
    let sumw = serialize_program(&program_cfg_fabric(BRINGUP_PRELOAD, "sumw"));
    let blob: Vec<i64> = std::iter::once(40)
        .chain((1..=40).map(|k| 7 * k - 100))
        .collect();
    for n in [40i64, 17] {
        v.push(BringupVector {
            bytes: wire(&sumw, &[n], &blob),
            result: eval_wprog_config_counted_heap_preloaded(&sumw, &[n], &blob).0,
        });
    }
    v
}

const SPDX_V: &str = "// SPDX-FileCopyrightText: 2026 DatacenterDotDev, Inc.\n// SPDX-License-Identifier: MIT\n";

/// The scaffold's copyright line: `user_defines.v` carries the scaffold's GPIO mode table.
const SCAFFOLD_COPYRIGHT_V: &str = "// SPDX-FileCopyrightText: 2022 Efabless Corporation\n";

/// The emitted files of the WPE submission's overlay, by path under it: the GPIOs' power-on modes, and the
/// bring-up directory — the bench driving the fabricated `user_project_wrapper` at the chip's clock with every
/// PSRAM pin at the harness's slowest delay and the board's longest flight, and the program bytes,
/// per-program lengths and model results it reads.
pub fn wpe_caravel_overlay_files() -> Vec<(String, String)> {
    let memory = wpe_caravel_memory();
    let vectors = wpe_caravel_bringup_vectors();
    let bytes: Vec<u8> = vectors
        .iter()
        .flat_map(|v| v.bytes.iter().copied())
        .collect();
    let tb = emit_wpe_user_project_wrapper_testbench(
        WPE_CARAVEL_NAME,
        vectors.len(),
        bytes.len(),
        &memory,
        &WPE_CARAVEL,
        CARAVEL_IO_MAX.with_board(BOARD_FLIGHT_MAX_PS),
    );
    let dv = "verilog/dv/logos_bringup";
    let spdx = |body: String| format!("{SPDX_V}{body}");
    vec![
        (
            "verilog/rtl/user_defines.v".to_string(),
            format!("{SCAFFOLD_COPYRIGHT_V}{}", spdx(emit_wpe_user_defines(&WPE_CARAVEL.pins))),
        ),
        (format!("{dv}/tb_user_project_wrapper.v"), spdx(tb)),
        (
            format!("{dv}/wire_{WPE_CARAVEL_NAME}.hex"),
            spdx(bytes.iter().map(|b| format!("{b:02x}\n")).collect()),
        ),
        (
            format!("{dv}/lens_{WPE_CARAVEL_NAME}.hex"),
            spdx(
                vectors
                    .iter()
                    .map(|v| format!("{:08x}\n", v.bytes.len()))
                    .collect(),
            ),
        ),
        (
            format!("{dv}/oracle_{WPE_CARAVEL_NAME}.hex"),
            spdx(
                vectors
                    .iter()
                    .map(|v| format!("{:016x}\n", v.result as u64))
                    .collect(),
            ),
        ),
    ]
}

/// The OpenSTA script that closes a PSRAM read through the hardened macro, at the corner `lib` characterizes and
/// over the parasitics in `spef`: the SCLK edge's latest arrival at its pad-side port, the SIO data constrained to
/// arrive the read's round trip outside the macro after it ([`WpeCaravel::read_round_trip_ps`]), and the worst
/// slack of that data into its capture flops at the edge the half SCLK period puts it on. The first two terms are
/// the macro's own share of the read window, which the harness check alone cannot see. Prints
/// `SCLK_ARRIVAL_NS <ns>` and `SIO_CAPTURE_SLACK_NS <ns>`.
pub fn wpe_psram_read_sta_tcl(
    caravel: &WpeCaravel,
    memory: &OffchipMemory,
    lib: &str,
    netlist: &str,
    spef: &str,
) -> String {
    let p = &caravel.pins;
    let sio = p
        .psram_sio
        .iter()
        .map(|s| format!("io_in[{s}]"))
        .collect::<Vec<_>>()
        .join(" ");
    let half = memory.edge.half;
    let multicycle = if half > 1 {
        format!("set_multicycle_path -setup {half} -from $sio\n")
    } else {
        String::new()
    };
    format!(
        "read_liberty {lib}\n\
         read_verilog {netlist}\n\
         link_design wpe_user_project\n\
         read_spef {spef}\n\
         create_clock -name clk -period {period} [get_ports {clk}]\n\
         set_propagated_clock [get_clocks clk]\n\
         set_output_delay 0 -clock clk [get_ports {{io_out[{sclk}]}}]\n\
         set sclk_arrival [get_property [lindex [get_property [find_timing_paths -to [get_ports {{io_out[{sclk}]}}] -path_delay max] points] end] arrival]\n\
         set sio [get_ports {{{sio}}}]\n\
         set_input_delay [expr {{$sclk_arrival + {outside}}}] -clock clk $sio\n\
         {multicycle}\
         set slack [get_property [find_timing_paths -from $sio -path_delay max] slack]\n\
         puts \"SCLK_ARRIVAL_NS [format %.3f $sclk_arrival]\"\n\
         puts \"SIO_CAPTURE_SLACK_NS [format %.3f $slack]\"\n",
        period = caravel.clk_period_ps as f64 / 1000.0,
        clk = WPE_CLOCK.port(),
        sclk = p.psram_sclk,
        outside = caravel.read_round_trip_ps(memory) as f64 / 1000.0,
    )
}
