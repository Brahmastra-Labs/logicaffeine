//! The decode engine as an accelerator function of the Open FPGA Stack on the Intel PAC D5005.
//!
//! [`afu_sources`] writes the files an OFS partial-reconfiguration build of the card's FIM takes
//! (`afu_synth_setup -s hw/rtl/sources.txt <build dir>`, then `afu_synth`):
//!   • `ofs_plat_afu`, the boundary with the Platform Interface Manager: host channel 0 as Avalon-MM MMIO and host
//!     memory in pClk, every local-memory bank as Avalon-MM in its own EMIF clock (its ECC lanes carried as data at
//!     [`LocalMemoryLine::DataAndEcc576`], holding each line's SECDED(72,64) check bytes at
//!     [`LocalMemoryLine::Data512Secded`]), the compute clock the user clock the AFU JSON asks for;
//!   • `logos_d5005_shell`, every platform port flat in its own clock, the only paths between clocks the dual-clock
//!     FIFOs `logos_dcfifo_mlab` and `logos_dcfifo_m20k` (Gray pointers through two-register synchronizers);
//!   • the stream engines (a reader and a writer on each memory channel's one port, and on the host memory port's
//!     two; on SECDED lines the line coder at each bank's port and the decoder in one registered stage behind it,
//!     `logos_secded.v` from [`secded_verilog`], the code [`secded_line`] and [`secded_read`] define), the registers
//!     ([`REGISTERS`]: the OFS feature header at word 0, the AFU ID, a command doorbell, the last status record), and
//!     the bring-up core ([`Op`]): fills and checks of every channel at full rate, copies between host memory and a
//!     channel, and on SECDED lines flips injected into the lines written and the words corrected and lost, so every
//!     path is proven on the card before the decode engine takes the core's ports;
//!   • the AFU JSON, the timing constraints of every FIFO crossing (the recipe the FIM's own `syn/setup/dcfifo.sdc`
//!     applies), the clock-domain map [`CLOCK_DOMAINS_FILE`] the CDC gate reads, and the C header a driver includes;
//!   • the host side: `sw/logos_hal.h`, the device a host program drives (its MMIO words, buffers the accelerator
//!     reaches by I/O address, its user clock), its OPAE 2.12 backend for the card and its physical backend
//!     (`sw/logos_hal_pci.c`: the VF's BAR and command word from sysfs, buffers in huge pages at their physical
//!     addresses, for a host with no IOMMU), `sw/logos_core.h`, the core's commands as a program rings them,
//!     `sw/logos_receipt.c`, the bring-up receipt (identity, every channel filled and checked alone and all at once
//!     with its bandwidth, a wrong seed seen, a host buffer's round trip through a channel, on SECDED lines every
//!     channel's code proven by injected flips) as one JSON object, and `sw/logos_load.c`, the image loader (a weight
//!     image the image writer made copied onto the channels at their full line width, read back against its files
//!     with every tile's check word checked, and with `--self-test` a damaged line found in its tile and the tile
//!     loaded again);
//!   • the simulation of all of it: `sim/logos_d5005_cosim.v`, the shell with four memory channels on four unrelated
//!     clocks behind each the [`ChannelModel`] the configuration names (`sim/logos_avmm_memory_model.v`, a memory that
//!     waits at random, or `sim/logos_ddr4_channel_model.v`, a DDR4-2400 channel timed by the controller model of
//!     `codegen_sva::ddr4_jedec`), and the same backend over a simulator (`sim/logos_hal_sim.h`), Icarus Verilog's
//!     VPI (`sim/logos_hal_vpi.c`) or Verilator's DPI-C (`sim/logos_hal_dpi.c`, the top built with `LOGOS_DPI`), so a
//!     program that runs on the card runs unchanged against the RTL, the simulation stepping only while the program
//!     waits on an MMIO access;
//!   • on the engine build, `sw/logos_step.c`, the program that runs a model on the engine one token a step from what
//!     [`write_engine_program`] wrote, and `sim/logos_engine_gated.v`, the engine a simulation runs in place of
//!     `hw/rtl/logos_engine.v`: the same engine with its core's clock gated to the user cycles the core acts in
//!     (`rtl::engine::simulation_twin`).
//!
//! The platform's widths are the D5005 FIM's (`src/top/ofs_d5005.ini`, `ipss/mem/includes/ofs_fim_emif_cfg_pkg.sv`):
//! four banks of 2^27 lines, 512 data bits and 64 ECC bits a line on a 72-bit DDR4-2400 bus whose controller has ECC
//! off, bursts of up to 64 lines; host channel lines of 512 bits at 51 address bits, MMIO words of 64 bits at 15.

use crate::codegen_sva::ddr4_jedec::{verilog_model, Ddr4Grade, LOOKAHEAD};
use crate::codegen_sva::scratchpad_rtl::{decode, encode, Decode};

use super::physical::card_port_verilog;
use super::rtl::engine::{engine_verilog, hard_float_model, simulation_twin, EngineParams, ENGINE_MIN_DIVIDE};

/// Memory banks the D5005 FIM gives the AFU.
pub const BANKS: u32 = 4;
/// Bits of a bank's line address.
pub const BANK_ADDRESS_BITS: u32 = 27;
/// Bits of a bank's burst count.
pub const BANK_BURST_BITS: u32 = 7;
/// Bits of a host channel line address and of its line.
pub const HOST_ADDRESS_BITS: u32 = 51;
pub const HOST_LINE_BITS: u32 = 512;
/// Bits of an MMIO word address (64-bit words of an 18-bit byte space).
pub const MMIO_WORD_ADDRESS_BITS: u32 = 15;

/// What a local-memory line carries to the AFU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalMemoryLine {
    /// The 512 data bits; the PIM's `LOCAL_MEM_AVALON_MEM_PARAMS_DEFAULT`.
    Data512,
    /// The data bits and the 64 ECC lanes the controller leaves unused, 576 bits as data; the PIM's
    /// `LOCAL_MEM_AVALON_MEM_PARAMS_FULL_BUS_DEFAULT`.
    DataAndEcc576,
    /// The 512 data bits, each of a line's eight 64-bit words guarded by SECDED(72,64) whose check byte the shell keeps
    /// in an ECC lane ([`secded_line`]): the PIM's full bus at the bank, every line coded as it is written and
    /// corrected as it is read.
    Data512Secded,
}

impl LocalMemoryLine {
    /// Bits of a line as the core moves it.
    pub fn bits(self) -> u32 {
        match self {
            LocalMemoryLine::Data512 | LocalMemoryLine::Data512Secded => 512,
            LocalMemoryLine::DataAndEcc576 => 576,
        }
    }

    /// Bits of a line on the bank's bus.
    pub fn bus_bits(self) -> u32 {
        match self {
            LocalMemoryLine::Data512 => 512,
            LocalMemoryLine::DataAndEcc576 | LocalMemoryLine::Data512Secded => 576,
        }
    }

    /// Whether the shell codes every line SECDED(72,64).
    pub fn secded(self) -> bool {
        self == LocalMemoryLine::Data512Secded
    }

    fn pim_params(self) -> &'static str {
        match self {
            LocalMemoryLine::Data512 => "LOCAL_MEM_AVALON_MEM_PARAMS_DEFAULT",
            LocalMemoryLine::DataAndEcc576 | LocalMemoryLine::Data512Secded => "LOCAL_MEM_AVALON_MEM_PARAMS_FULL_BUS_DEFAULT",
        }
    }

    fn pim_width(self) -> &'static str {
        match self {
            LocalMemoryLine::Data512 => "LOCAL_MEM_DATA_WIDTH",
            LocalMemoryLine::DataAndEcc576 | LocalMemoryLine::Data512Secded => "LOCAL_MEM_FULL_BUS_WIDTH",
        }
    }
}

// ── SECDED(72,64) on a channel line ─────────────────────────────────────────────────────────────────

/// Words of a channel line, each its own SECDED(72,64) codeword.
pub const SECDED_WORDS: usize = 8;

/// The codeword position of bit `j` of a word's check byte: the overall parity, then the Hamming checks at the powers
/// of two, as `codegen_sva::scratchpad_rtl` places them.
pub const SECDED_CHECKS: [u32; 8] = [0, 1, 2, 4, 8, 16, 32, 64];

/// The ECC lane holding word `w`'s check byte: bus byte `64 + 7 − w`. The controller's 576-bit word is either the
/// memory burst's eight beats of 72 DQ pins each (bus bit `i` on pin `i mod 72`), or its eight beats of the 64 data
/// pins beneath the eight beats of the eight ECC pins (data bit `i` on pin `i mod 64`, ECC bit `e` on pin
/// `64 + e mod 8`). Under either, word `w`'s data bits and ECC lane `7 − w` lie on 72 different pins, so a failing pin
/// flips one bit of each codeword, which the code corrects; pairing word `w` with lane `w` holds only under the
/// second. Were the bus word each group of eight pins' beats in turn, no grouping of whole bytes would hold under all
/// three.
pub fn secded_check_byte(word: usize) -> usize {
    SECDED_WORDS - 1 - word
}

/// The codeword position of each of a word's data bits.
fn secded_positions() -> &'static [u32; 64] {
    static AT: std::sync::OnceLock<[u32; 64]> = std::sync::OnceLock::new();
    AT.get_or_init(|| {
        let mut at = [0u32; 64];
        for (k, p) in at.iter_mut().enumerate() {
            let cw = encode(1u64 << k);
            *p = (0..72u32).find(|&q| cw >> q & 1 == 1 && !SECDED_CHECKS.contains(&q)).expect("a data bit has a place");
        }
        at
    })
}

/// A word's check byte: bit `j` the codeword's bit at [`SECDED_CHECKS`]`[j]`.
fn secded_check(word: u64) -> u8 {
    let cw = encode(word);
    SECDED_CHECKS.iter().enumerate().fold(0u8, |c, (j, &p)| c | ((cw >> p & 1) as u8) << j)
}

/// A channel line as the bus holds it: its 64 data bytes where they are, then the eight ECC lanes, word `w`'s check
/// byte at lane [`secded_check_byte`]`(w)`.
pub fn secded_line(data: &[u8; 64]) -> [u8; 72] {
    let mut bus = [0u8; 72];
    bus[..64].copy_from_slice(data);
    for w in 0..SECDED_WORDS {
        let word = u64::from_le_bytes(data[8 * w..8 * w + 8].try_into().expect("eight bytes"));
        bus[64 + secded_check_byte(w)] = secded_check(word);
    }
    bus
}

/// A bus line read back: its data, each word corrected where its codeword shows one flipped bit, and bit `w` of
/// `corrected` and of `uncorrectable` the verdict on word `w`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SecdedRead {
    pub data: [u8; 64],
    pub corrected: u8,
    pub uncorrectable: u8,
}

/// What the shell makes of a bus line it reads: each word's codeword reassembled from its data bits and its check
/// byte and decoded as `scratchpad_rtl::decode` decodes it.
pub fn secded_read(bus: &[u8; 72]) -> SecdedRead {
    let positions = secded_positions();
    let mut out = SecdedRead { data: [0; 64], corrected: 0, uncorrectable: 0 };
    for w in 0..SECDED_WORDS {
        let word = u64::from_le_bytes(bus[8 * w..8 * w + 8].try_into().expect("eight bytes"));
        let check = bus[64 + secded_check_byte(w)];
        let mut cw = 0u128;
        for (k, &p) in positions.iter().enumerate() {
            cw |= ((word >> k & 1) as u128) << p;
        }
        for (j, &p) in SECDED_CHECKS.iter().enumerate() {
            cw |= ((check >> j & 1) as u128) << p;
        }
        let (data, verdict) = decode(cw);
        out.data[8 * w..8 * w + 8].copy_from_slice(&data.to_le_bytes());
        match verdict {
            Decode::NoError => {}
            Decode::Corrected(_) | Decode::CorrectedParityBit => out.corrected |= 1 << w,
            Decode::DoubleError => out.uncorrectable |= 1 << w,
        }
    }
    out
}

/// `logos_secded.v`: the line coder and decoder, every parity a mask of a word's data bits read off
/// [`encode`]'s codewords, every correction the data bit whose codeword position the syndrome names.
pub fn secded_verilog() -> String {
    let positions = secded_positions();
    let mut emask = [0u64; 8];
    for k in 0..64 {
        let c = secded_check(1u64 << k);
        for (j, m) in emask.iter_mut().enumerate() {
            *m |= ((c >> j & 1) as u64) << k;
        }
    }
    let smask = |i: u32| (0..64).filter(|&k| positions[k] >> i & 1 == 1).fold(0u64, |m, k| m | 1 << k);
    let mut v = String::from(SECDED_HEAD);
    v += "module logos_secded_word_encode(input [63:0] d, output [7:0] c);\n";
    for (j, m) in emask.iter().enumerate() {
        v += &format!("  assign c[{j}] = ^(d & 64'h{m:016x});\n");
    }
    v += "endmodule\n\n";
    v += SECDED_WORD_DECODE_HEAD;
    v += "module logos_secded_word_decode(input [63:0] d, input [7:0] c, output [63:0] q, output corrected,\n  output uncorrectable);\n";
    let syndrome: Vec<String> = (0..7u32).rev().map(|i| format!("^(d & 64'h{:016x}) ^ c[{}]", smask(i), i + 1)).collect();
    v += &format!("  wire [6:0] s = {{{}}};\n", syndrome.join(",\n                   "));
    v += "  wire       odd = ^d ^ ^c;\n";
    v += "  assign corrected = odd && s <= 7'd71;\n";
    v += "  assign uncorrectable = odd ? s > 7'd71 : s != 7'd0;\n";
    for (k, p) in positions.iter().enumerate() {
        v += &format!("  assign q[{k}] = d[{k}] ^ (odd && s == 7'd{p});\n");
    }
    v += "endmodule\n\n";
    v += SECDED_LINE;
    v
}

const SECDED_HEAD: &str = r##"// SECDED(72,64) on a channel line, generated from codegen_sva::scratchpad_rtl's code: each of the line's eight
// 64-bit words is one codeword, its data bits where they are on the bus (bits 64w..64w+63) and its check byte in ECC
// lane 7 - w (bus bits 512 + 8(7 - w) ..), check bit j the codeword's bit at position C_j, C = 0, 1, 2, 4, 8, 16, 32, 64
// (C_0 the overall parity). Lane 7 - w puts each codeword on 72 different DQ pins whether the controller's bus word
// runs beat by beat over all 72 pins or holds the data pins' beats beneath the ECC pins'.

// A word's check byte: each bit the parity of the data bits that feed it.
"##;

const SECDED_WORD_DECODE_HEAD: &str = r##"// A word read back: syndrome bit i the parity of the codeword positions with bit i set, so a single flipped bit's
// position; the overall parity odd for an odd count of flips. Odd with a syndrome naming a position: one flip,
// corrected (a check bit's flip leaves the data as it is). Even with a syndrome, or odd with one naming no position:
// uncorrectable, the data as read.
"##;

const SECDED_LINE: &str = r##"// A line coded: the data bits as they are, word w's check byte in ECC lane 7 - w.
module logos_secded_encode(input [511:0] d, output [575:0] bus);
  assign bus[511:0] = d;
  genvar w;
  generate for (w = 0; w < 8; w = w + 1) begin : word
    logos_secded_word_encode coder(.d(d[64*w +: 64]), .c(bus[512 + 8*(7-w) +: 8]));
  end endgenerate
endmodule

// A line read back: the data, each word corrected where it can be, and each word's verdict, bit w for word w.
module logos_secded_decode(input [575:0] bus, output [511:0] d, output [7:0] corrected, output [7:0] uncorrectable);
  genvar w;
  generate for (w = 0; w < 8; w = w + 1) begin : word
    logos_secded_word_decode decoder(.d(bus[64*w +: 64]), .c(bus[512 + 8*(7-w) +: 8]), .q(d[64*w +: 64]),
      .corrected(corrected[w]), .uncorrectable(uncorrectable[w]));
  end endgenerate
endmodule
"##;

/// What stands behind each memory channel in the co-simulation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelModel {
    /// A memory that waits and gaps at random, the stress the shell's FIFOs and the bring-up are proved under.
    Random,
    /// A DDR4-2400 channel timed by the controller model of `codegen_sva::ddr4_jedec` (`logos_ddr4_channel_model`):
    /// the cycles a run takes are the controller model's for the lines the shell asks for.
    Ddr4,
}

/// The accelerator a build makes.
#[derive(Clone, Debug, PartialEq)]
pub struct AfuConfig {
    /// The accelerator's name in the AFU JSON.
    pub name: &'static str,
    /// The `accelerator-type-uuid` a driver opens it by, and the AFU ID its feature header carries.
    pub uuid: u128,
    /// A build identity the BUILD register reads back.
    pub build: u64,
    pub line: LocalMemoryLine,
    /// The user clock the AFU JSON asks for, MHz: the fitter's target, the clock set from the timing it closes. A divided
    /// build asks for exactly this clock.
    pub user_clock_mhz: u32,
    /// The compute clock the shell runs on: the user clock divided by this, an even number, or the user clock itself at
    /// 1. A divided build closes timing at the divided period.
    pub compute_divide: u32,
    /// Lines a burst, on every port.
    pub max_burst: u32,
    /// log2 of the lines a read-data FIFO holds, and a write-data FIFO.
    pub read_fifo_log2: u32,
    pub write_fifo_log2: u32,
    /// The co-simulation's memory channels.
    pub channel_model: ChannelModel,
    /// The decode engine the core runs steps on (`rtl::engine::engine_verilog`, its memory on the channels through the
    /// card port, `physical::card_port_verilog`), or none for the bring-up build.
    pub engine: Option<EngineParams>,
    /// Placement assignments for Quartus, as Tcl: the tree's `hw/rtl/logos_floorplan.tcl`, named in sources.txt as
    /// `QI:` so the build sources it; none when no floorplan is given.
    pub floorplan: Option<String>,
    /// Whether the accelerator is compiled into the FIM itself (a flat image, `scripts/d5005/build-afu.sh --flat`)
    /// rather than as a persona. The FIM's static region instantiates its exercisers (`he_lb_top`, PF0 VF0) whatever
    /// the port holds, but the Stratix 10 release's `afu_main.tcl` loads their sources only when no PIM-wrapped AFU is
    /// configured, so a flat build's sources.txt names them: [`FIM_STATIC_EXERCISERS`], from the FIM that builds it.
    pub flat: bool,
}

/// The FIM's static exercisers' file list, as sources.txt names it for a flat build (`QI:`, so Quartus sources it):
/// under `OFS_ROOTDIR`, the FIM source tree the FIM's build runs from, which OPAE's rtl_src_config expands.
pub const FIM_STATIC_EXERCISERS: &str = "${OFS_ROOTDIR}/ofs-common/src/common/afu_main_std_exerciser_design_files.tcl";

/// Every accelerator the engine builds for the card is found by this UUID: the name-based UUID of
/// `logicaffeine:d5005:decode-engine` (the first 16 bytes of its SHA-256, version 5).
pub const AFU_UUID: u128 = 0x1adf73eb_54c5_5b4c_8a2e_3a459f9b2a6e;

/// The user clock an engine build asks for, MHz, and the divide its compute clock is: the reference engine's
/// single-cycle units close at that period on the 1SX280.
pub const ENGINE_USER_CLOCK_MHZ: u32 = 100;
pub const ENGINE_COMPUTE_DIVIDE: u32 = 64;

impl AfuConfig {
    /// The engine build for `engine` at `line` (data64 or SECDED lines): the bring-up build's core and channels with
    /// the engine and its card port behind them, its compute clock [`ENGINE_USER_CLOCK_MHZ`] divided by
    /// [`ENGINE_COMPUTE_DIVIDE`].
    pub fn engine(engine: EngineParams, line: LocalMemoryLine) -> AfuConfig {
        AfuConfig {
            name: "logos_d5005_engine",
            line,
            user_clock_mhz: ENGINE_USER_CLOCK_MHZ,
            compute_divide: ENGINE_COMPUTE_DIVIDE,
            engine: Some(engine),
            ..AfuConfig::d5005()
        }
    }

    /// The bring-up build: the ECC lanes as data, the user clock at the engine plan's 450 MHz, 64-line bursts, 512
    /// lines of read FIFO a channel (a run of eight bursts in flight) and 64 of write FIFO.
    pub fn d5005() -> AfuConfig {
        AfuConfig {
            name: "logos_d5005_bringup",
            uuid: AFU_UUID,
            build: 0,
            line: LocalMemoryLine::DataAndEcc576,
            user_clock_mhz: 450,
            compute_divide: 1,
            max_burst: 64,
            read_fifo_log2: 9,
            write_fifo_log2: 6,
            channel_model: ChannelModel::Random,
            engine: None,
            floorplan: None,
            flat: false,
        }
    }

    /// Whether the build's engine is a hybrid one: its matvec jobs on a streaming unit in the user clock.
    pub fn hybrid(&self) -> bool {
        self.engine.as_ref().is_some_and(|e| e.streaming.is_some())
    }

    /// Whether the engine's streaming unit pairs its cores on processing elements in a clock of twice theirs
    /// ([`super::rtl::column::ColumnSet::pe_pairs`]): the user side in the PIM's `uClk_usrDiv2`, the pairs in `uClk_usr`.
    pub fn pe_pairs(&self) -> bool {
        self.engine.as_ref().and_then(|e| e.streaming.as_ref()).is_some_and(|s| s.set.pe_pairs)
    }

    /// Whether the engine's array issues its held beat against two token-rows a clock
    /// ([`super::rtl::column::ColumnSet::with_token_pairs`]).
    pub fn token_pairs(&self) -> bool {
        self.engine.as_ref().and_then(|e| e.streaming.as_ref()).is_some_and(|s| s.set.token_pairs)
    }

    /// Whether the engine's f32 products are the card's floating-point DSP blocks' ([`super::numerics::FpUnit::Hard`]):
    /// its programs are written at the card's products.
    pub fn fp_hard(&self) -> bool {
        self.engine
            .as_ref()
            .and_then(|e| e.streaming.as_ref())
            .and_then(|s| s.hard_float)
            .is_some_and(|h| h.fp_unit == super::numerics::FpUnit::Hard)
    }

    /// The engine's sequencer contexts ([`super::rtl::engine::Streaming::contexts`]): 1 for one sequencer or none.
    pub fn contexts(&self) -> usize {
        self.engine.as_ref().and_then(|e| e.streaming.as_ref()).map_or(1, |s| s.contexts)
    }

    /// The UUID as the AFU JSON writes it.
    pub fn uuid_text(&self) -> String {
        let h = format!("{:032x}", self.uuid);
        format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
    }
}

/// How the host reaches a register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    ReadOnly,
    ReadWrite,
    /// A write pushes a command; a read returns zero.
    Doorbell,
    /// A written one clears that bit.
    WriteOneToClear,
}

/// One 64-bit register of the MMIO space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Register {
    pub name: &'static str,
    /// The 64-bit word it is at; its byte offset is eight times that.
    pub word: u32,
    pub access: Access,
}

/// The register map: the OFS AFU feature header (type 1, end of the list, no next feature) and the AFU ID at words 0
/// to 4, as every OFS AFU carries them, then the engine's.
pub const REGISTERS: [Register; 18] = [
    Register { name: "DFH", word: 0, access: Access::ReadOnly },
    Register { name: "ID_L", word: 1, access: Access::ReadOnly },
    Register { name: "ID_H", word: 2, access: Access::ReadOnly },
    Register { name: "RSVD0", word: 3, access: Access::ReadOnly },
    Register { name: "RSVD1", word: 4, access: Access::ReadOnly },
    Register { name: "SCRATCH", word: 5, access: Access::ReadWrite },
    Register { name: "BUILD_ID", word: 6, access: Access::ReadOnly },
    Register { name: "STATUS", word: 8, access: Access::ReadOnly },
    Register { name: "RESULT0", word: 9, access: Access::ReadOnly },
    Register { name: "RESULT1", word: 10, access: Access::ReadOnly },
    Register { name: "STATUS_SEQ", word: 11, access: Access::ReadOnly },
    Register { name: "CMD_SPACE", word: 12, access: Access::ReadOnly },
    Register { name: "ERRORS", word: 13, access: Access::WriteOneToClear },
    Register { name: "DROPPED", word: 14, access: Access::ReadOnly },
    Register { name: "ARG0", word: 16, access: Access::ReadWrite },
    Register { name: "ARG1", word: 17, access: Access::ReadWrite },
    Register { name: "ARG2", word: 18, access: Access::ReadWrite },
    Register { name: "DOORBELL", word: 19, access: Access::Doorbell },
];

/// The bring-up core's commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Nop,
    /// Every channel in mask ARG0 writes ARG2\[31:0\] lines from line ARG1, line L the pattern seeded ARG2\[63:32\].
    Fill,
    /// Every channel in the mask reads those lines back; RESULT0 the lines that differ.
    Check,
    /// ARG2\[31:0\] host lines from host line ARG0 into channel ARG1\[63:56\] at line ARG1\[26:0\], zero-extended.
    HostToMem,
    /// The reverse, a channel line truncated to a host line; complete when every host write burst is acknowledged.
    MemToHost,
    /// ARG2\[31:0\] channel lines of the channel's full width into channel ARG1\[63:56\] at line ARG1\[26:0\], read packed end
    /// to end from host line ARG0 on: ceil(lines × line bits / 512) host lines, the bytes past the last channel line
    /// unused.
    HostToMemPacked,
    /// The reverse: the channel lines packed end to end into host lines from ARG0, the last host line zero past them;
    /// complete when every host write burst is acknowledged.
    MemToHostPacked,
    /// On SECDED lines: the next ARG2\[31:0\] lines written to each channel in mask ARG0 have bus bits ARG1\[9:0\] and
    /// ARG1\[25:16\] flipped after they are coded, one bit when the two are equal; complete at once.
    Inject,
    /// On SECDED lines: RESULT0 = {words found uncorrectable, words corrected} of channel ARG0\[7:0\] since reset, each
    /// counted modulo 2^32, so a host's difference of two reads is exact; complete at once.
    EccCounts,
    /// With the engine: its configuration a program keeps — ARG0\[15:0\] the token-rows of a step, ARG0\[19:16\] {per-block
    /// scaling, two words, an int8 cache, an f16 state} — and the card port's map: ARG1\[31:0\] the granule space's first
    /// logical line, ARG2\[26:0\] the channel line it starts at, ARG2\[60:56\] log2 of a granule's lines; complete at once.
    EngineConfig,
    /// With the engine: ARG2\[31:0\] host lines from host line ARG0 into the engine's table ARG1\[34:32\] from entry
    /// ARG1\[31:0\] on, one entry a line ([`crate::spatial::decode_engine::rtl::engine::Tables`]; table 6 the store).
    EngineTable,
    /// With the engine: a step, the jobs from word ARG0\[31:0\] to word ARG1\[31:0\], the card port holding the channels;
    /// complete when the engine is idle again, RESULT0 = {the word the sequencer reached \[63:32\], the card port's fault
    /// \[1\], the engine's fault \[0\]}.
    EngineRun,
    /// With the engine: ARG2\[31:0\] beats of its store from beat ARG1\[31:0\] into host lines from ARG0; complete when every
    /// host write burst is acknowledged.
    StoreRead,
}

impl Op {
    pub const ALL: [Op; 13] = [
        Op::Nop,
        Op::Fill,
        Op::Check,
        Op::HostToMem,
        Op::MemToHost,
        Op::HostToMemPacked,
        Op::MemToHostPacked,
        Op::Inject,
        Op::EccCounts,
        Op::EngineConfig,
        Op::EngineTable,
        Op::EngineRun,
        Op::StoreRead,
    ];

    pub fn code(self) -> u8 {
        match self {
            Op::Nop => 0,
            Op::Fill => 1,
            Op::Check => 2,
            Op::HostToMem => 3,
            Op::MemToHost => 4,
            Op::HostToMemPacked => 5,
            Op::MemToHostPacked => 6,
            Op::Inject => 7,
            Op::EccCounts => 8,
            Op::EngineConfig => 9,
            Op::EngineTable => 10,
            Op::EngineRun => 11,
            Op::StoreRead => 12,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Op::Nop => "NOP",
            Op::Fill => "FILL",
            Op::Check => "CHECK",
            Op::HostToMem => "HOST_TO_MEM",
            Op::MemToHost => "MEM_TO_HOST",
            Op::HostToMemPacked => "HOST_TO_MEM_PACKED",
            Op::MemToHostPacked => "MEM_TO_HOST_PACKED",
            Op::Inject => "INJECT",
            Op::EccCounts => "ECC_COUNTS",
            Op::EngineConfig => "ENGINE_CONFIG",
            Op::EngineTable => "ENGINE_TABLE",
            Op::EngineRun => "ENGINE_RUN",
            Op::StoreRead => "STORE_READ",
        }
    }
}

// ── The engine's program on the card ───────────────────────────────────────────────────────────────

/// What a host program runs a model on the engine build with, one token-row a step: written by
/// [`write_engine_program`] as `engine.txt`, beside the engine's tables, the memory image and the embedding table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineProgram {
    /// ENGINE_CONFIG's knobs, {per_block, two_words, cache_int8, state_f16}.
    pub knobs: u8,
    /// The card port's map: the space's first logical line, the channel line it starts at, log2 of a granule's lines.
    pub space: u64,
    pub base: u64,
    pub granule_log2: u32,
    /// Words of the step's jobs after the host's embedding, table 0's entries.
    pub job_words: usize,
    /// The store's first beat and beats a row of a row's embedding and of its logits.
    pub residual: (u32, u32),
    pub logits: (u32, u32),
    /// Whether a row's logits slot holds their arg-max and that logit ([`super::machine::StoreLayout::greedy`]) in place
    /// of the logits.
    pub greedy: bool,
    /// The store's beats over the program's rows: a build whose store is smaller cannot run it.
    pub store_beats: u32,
    /// Tokens the embedding table holds, its row's elements, and the logits a step reads back.
    pub tokens: usize,
    pub hidden: usize,
    pub vocab: usize,
    /// A row's ring index is its position modulo this.
    pub ring_rows: usize,
    /// The first logical line of sequence 0's region, a region's lines, the regions, the positions a sequence reaches.
    pub regions: u64,
    pub region: u64,
    pub sequences: usize,
    pub ctx: usize,
    /// Lines of each channel's image.
    pub channel_lines: Vec<u64>,
    /// The streaming unit the program's stream entries are written for, as its build names it
    /// (`LOGOS_D5005_STREAMING`): empty for the reference engine.
    pub streaming: String,
    /// The engine's sequencer contexts the program is written for: sequence s runs in context s mod the contexts, as its
    /// row s / the contexts, the store laid for a context's rows.
    pub contexts: usize,
    /// The multiplier the program's numerics take ([`super::numerics::FpUnit`]): its build's products must be the same
    /// (`LOGOS_D5005_FP_HARD`).
    pub fp_unit: super::numerics::FpUnit,
}

/// One 64-byte host line holding `bits` from its lowest byte.
fn host_line(bits: &[u8]) -> [u8; 64] {
    let mut line = [0u8; 64];
    line[..bits.len()].copy_from_slice(bits);
    line
}

/// The job listing of a step whose jobs past the host's embedding are `jobs` (the engine's job table, as
/// [`super::machine::encode`] lays them in `table0.bin`): a "job <j> word <w> <kind>" line a job, job 0 the host's
/// embedding at word 0 and job j the table's `jobs[j - 1]` at the word the table opens it at, its kind its `Debug` text
/// cut at 240 characters. `sw/logos_step.c` profiles a step by it (LOGOS_PROFILE), a job at a time.
pub fn job_listing(jobs: &[super::machine::Job]) -> String {
    use std::fmt::Write;
    let mut listing = String::from("job 0 word 0 Embed\n");
    let mut word = 0;
    for (j, job) in jobs.iter().enumerate() {
        writeln!(listing, "job {} word {word} {}", j + 1, format!("{job:?}").chars().take(240).collect::<String>()).unwrap();
        word += super::machine::encode(std::slice::from_ref(job)).len();
    }
    listing
}

/// [`job_listing`] of the program in `dir` read back from its job table (`table0.bin`, a word a 64-byte line) alone: for
/// a program written before programs carried their listing.
pub fn job_listing_of(dir: &std::path::Path) -> std::io::Result<String> {
    let table = std::fs::read(dir.join("table0.bin"))?;
    let words: Vec<u64> = table.chunks_exact(64).map(|line| u64::from_le_bytes(line[..8].try_into().unwrap())).collect();
    let jobs = super::machine::decode(&words).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {e}", dir.display())))?;
    Ok(job_listing(&jobs))
}

/// Write the program `machine` — as [`super::machine::Machine::new`] left it, its regions empty — runs on the engine
/// build over `map`, a flat map ([`super::physical::PhysicalMap::flat`]: the reference engine reads its weights a
/// line at a time through the card port), into `dir`: `engine.txt`, `table<k>.bin` for the engine's static tables
/// (0 the step's jobs after the host's embedding, 2 the slots, 3 the tensors, 4 the rotary tables, 5 the blocks), a
/// 64-byte line an entry, `jobs.txt` the step's job listing ([`job_listing`]), `channel<k>.bin` the memory image, and
/// `embed.bin`, every token's embedding as `f32`.
pub fn write_engine_program(machine: &super::machine::Machine, map: &super::physical::PhysicalMap, dir: &std::path::Path) -> std::io::Result<EngineProgram> {
    assert_eq!(map.tensors_end, 0, "the reference engine reads its weights through the card port, over a flat map");
    assert!(map.image.tensors.is_empty(), "a flat map has no weight image");
    let channels = map.channel_bytes(machine.memory(), &vec![Vec::new(); map.channel_lines.len()]);
    write_program(machine, map, &|at| Some(at), &channels, String::new(), false, 1, dir)
}

/// Write the program `machine` runs on the hybrid engine build over `map`, a hybrid map
/// ([`super::physical::PhysicalMap::hybrid`]: the matrices the machine streams in their image at each channel's start,
/// the memory compacted by `compaction` with the streamed tensors' stream entries after it, the space the engine's card
/// port reads), for a streaming unit built for `set`, the image's tiles read from `weights`: the files
/// [`write_engine_program`] writes, every address the engine reads in the flat space where `compaction` finds it.
pub fn write_hybrid_program(
    machine: &super::machine::Machine,
    map: &super::physical::PhysicalMap,
    compaction: &super::physical::Compaction,
    set: &super::rtl::column::ColumnSet,
    weights: &mut dyn crate::model::reference::WeightSource,
    dir: &std::path::Path,
) -> std::io::Result<EngineProgram> {
    hybrid_program(machine, map, compaction, set, weights, false, 1, dir)
}

/// [`write_hybrid_program`] with a greedy head: the head's stream entry asks the streaming unit for each row's arg-max of
/// its logits rather than the logits, and the store is laid greedy ([`super::machine::StoreLayout::greedy`]), so a row's
/// read-back is one beat however wide the vocabulary and the store holds many rows. An error when the head's matrix is
/// not one the unit streams.
pub fn write_greedy_program(
    machine: &super::machine::Machine,
    map: &super::physical::PhysicalMap,
    compaction: &super::physical::Compaction,
    set: &super::rtl::column::ColumnSet,
    weights: &mut dyn crate::model::reference::WeightSource,
    dir: &std::path::Path,
) -> std::io::Result<EngineProgram> {
    hybrid_program(machine, map, compaction, set, weights, true, 1, dir)
}

/// [`write_hybrid_program`], greedy when `greedy` ([`write_greedy_program`]), for an engine of `contexts` sequencer
/// contexts ([`super::rtl::engine::Streaming::contexts`]): the machine's sequences dealt over them, sequence s in
/// context s mod `contexts` as its row s / `contexts`, the store laid for a context's rows — an error unless the
/// sequences fill every context alike.
#[allow(clippy::too_many_arguments)]
pub fn write_contexts_program(
    machine: &super::machine::Machine,
    map: &super::physical::PhysicalMap,
    compaction: &super::physical::Compaction,
    set: &super::rtl::column::ColumnSet,
    weights: &mut dyn crate::model::reference::WeightSource,
    greedy: bool,
    contexts: usize,
    dir: &std::path::Path,
) -> std::io::Result<EngineProgram> {
    let sequences = machine.layout.sequences;
    if contexts < 1 || sequences % contexts != 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{sequences} sequences over {contexts} contexts")));
    }
    hybrid_program(machine, map, compaction, set, weights, greedy, contexts, dir)
}

#[allow(clippy::too_many_arguments)]
fn hybrid_program(
    machine: &super::machine::Machine,
    map: &super::physical::PhysicalMap,
    compaction: &super::physical::Compaction,
    set: &super::rtl::column::ColumnSet,
    weights: &mut dyn crate::model::reference::WeightSource,
    greedy: bool,
    contexts: usize,
    dir: &std::path::Path,
) -> std::io::Result<EngineProgram> {
    use super::rtl::streaming::StreamEntry;
    assert_eq!(map.tensors_end, 0, "the hybrid engine's card port reads the flat space from logical line 0");
    let invalid = |e: String| std::io::Error::new(std::io::ErrorKind::InvalidInput, e);
    let head = &machine.layout.tensors[head_tensor(&machine.jobs)].name;
    let streamed = map.image.tensors.iter().position(|t| &t.name == head);
    if greedy && streamed.is_none() {
        return Err(invalid(format!("a greedy head streams its matrix, and `{head}` is not streamed")));
    }
    let folds = folds_of(machine).map_err(invalid)?;
    let entries = (0..map.image.tensors.len())
        .map(|k| {
            let fold = folds.get(map.image.tensors[k].name.as_str()).copied().unwrap_or(super::machine::Fold::Put);
            StreamEntry::of(&map.image, k, set).map(|e| StreamEntry { greedy: greedy && Some(k) == streamed, fold, ..e })
        })
        .collect::<Result<Vec<_>, String>>()
        .map_err(invalid)?;
    let folding = folds.values().any(|&f| f != super::machine::Fold::Put);
    let normed = machine.jobs.iter().any(|j| matches!(j, super::machine::Job::Delta { norm: Some(_), .. }));
    let image = map.image.fill(weights)?;
    let channels = map.channel_bytes(&compaction.space(machine.memory(), &entries), &image);
    let pairs = map.image.layout.stripe.expect("a hybrid map's image laid in stripes");
    let streaming = format!(
        "{}/{pairs}{}{}{}",
        set.signature(),
        if folding { "+folds" } else { "" },
        if normed { "+norm" } else { "" },
        super::rtl::engine::behind(Some(machine.layout.numerics.flush))
    );
    write_program(machine, map, &|at| compaction.find(at), &channels, streaming, greedy, contexts, dir)
}

/// Each tensor a matrix job of `machine`'s step reads and how that job folds its sums: an error naming a tensor two jobs
/// fold two ways, which no stream entry could say.
fn folds_of(machine: &super::machine::Machine) -> Result<std::collections::HashMap<&str, super::machine::Fold>, String> {
    use super::machine::Job;
    let mut folds = std::collections::HashMap::new();
    for job in &machine.jobs {
        if let Job::Matvec { tensor, fold, .. } = job {
            let name = machine.layout.tensors[*tensor].name.as_str();
            if folds.insert(name, *fold).is_some_and(|f| f != *fold) {
                return Err(format!("`{name}` folded two ways by two jobs"));
            }
        }
    }
    Ok(folds)
}

/// The tensor of the head, the last matvec job writing the logits.
fn head_tensor(jobs: &[super::machine::Job]) -> usize {
    use super::machine::Job;
    use super::program::Slot;
    jobs.iter()
        .rev()
        .find_map(|j| match j {
            Job::Matvec { tensor, to: Slot::Logits, .. } => Some(*tensor),
            _ => None,
        })
        .expect("a head writes the logits")
}

/// The program's files: the engine's tables with every address placed by `place`, the step's job listing (`jobs.txt`,
/// [`job_listing`]), the channels' bytes `channels`, the embeddings and `engine.txt`, which names the streaming unit
/// `streaming` when the program is a hybrid engine's and says when its head is greedy, then the model's program the
/// step was read off: `program.lg`, the LOGOS text the author wrote, and `program.txt`, its identity line. The store holds a row for each of the machine's sequences, a
/// step a token each — a bank of the store a context, each the rows of its sequences, for `contexts` contexts.
#[allow(clippy::too_many_arguments)]
fn write_program(
    machine: &super::machine::Machine,
    map: &super::physical::PhysicalMap,
    place: &dyn Fn(u64) -> Option<u64>,
    channels: &[Vec<u8>],
    streaming: String,
    greedy: bool,
    contexts: usize,
    dir: &std::path::Path,
) -> std::io::Result<EngineProgram> {
    use super::machine::{BlockState, Job, Row, StoreLayout, LINE};
    use super::program::Slot;
    let layout = &machine.layout;
    let jobs = &machine.jobs;
    let Some(Job::Embed { tensor: embed }) = jobs.first() else { panic!("a step opens with the host's embedding") };
    assert!(map.granule.is_power_of_two(), "the card port's granule is a power of two");
    let rows = layout.sequences / contexts;
    let store = if greedy { StoreLayout::greedy(jobs, layout, rows) } else { StoreLayout::of(jobs, layout, rows) };
    let t = super::rtl::engine::tables_at(layout, &store, &jobs[1..], &[Row { sequence: 0, position: 0, token: 0, ring: 0 }], place);
    std::fs::create_dir_all(dir)?;
    let write = |name: &str, lines: Vec<[u8; 64]>| std::fs::write(dir.join(name), lines.concat());
    write("table0.bin", t.jobs.iter().map(|w| host_line(&w.to_le_bytes())).collect())?;
    write("table2.bin", t.slots.iter().map(|w| host_line(&w.to_le_bytes())).collect())?;
    write("table3.bin", t.tensors.iter().map(|w| host_line(&w.to_le_bytes())).collect())?;
    write("table4.bin", t.ropes.iter().map(|w| host_line(&w.to_le_bytes())).collect())?;
    write("table5.bin", t.blocks.iter().map(|f| host_line(&f.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>())).collect())?;
    std::fs::write(dir.join("jobs.txt"), job_listing(&jobs[1..]))?;
    for (k, channel) in channels.iter().enumerate() {
        std::fs::write(dir.join(format!("channel{k}.bin")), channel)?;
    }
    let e = &layout.tensors[*embed];
    let (hidden, tokens) = (e.dims[0] as usize, e.dims[1] as usize);
    let rb = e.row_bytes() as usize;
    let mut table = Vec::with_capacity(tokens * hidden * 4);
    for token in 0..tokens {
        let at = e.at as usize + token * rb;
        let mut v = Vec::new();
        e.ty.dequantize(&machine.memory()[at..at + rb], &mut v);
        table.extend(v.iter().flat_map(|x| x.to_le_bytes()));
    }
    std::fs::write(dir.join("embed.bin"), table)?;
    let vocab = layout.tensors[head_tensor(jobs)].dims[1] as usize;
    let ring_rows = layout
        .blocks
        .iter()
        .find_map(|b| match b {
            BlockState::Delta { taps, .. } => Some(taps.saturating_sub(1).max(1)),
            _ => None,
        })
        .unwrap_or(1);
    assert!(layout.regions % LINE == 0 && layout.region % LINE == 0, "regions on lines");
    let regions = place(layout.regions).expect("a place for the regions");
    let p = EngineProgram {
        knobs: t.state_f16 as u8 | (t.cache_int8 as u8) << 1 | (t.two_words as u8) << 2 | (t.per_block as u8) << 3,
        space: map.tensors_end / LINE,
        base: map.base,
        granule_log2: map.granule.trailing_zeros(),
        job_words: t.jobs.len(),
        residual: store.slots[Slot::Residual.code() as usize],
        logits: store.slots[Slot::Logits.code() as usize],
        greedy,
        store_beats: store.beats,
        tokens,
        hidden,
        vocab,
        ring_rows,
        regions: regions / LINE,
        region: layout.region / LINE,
        sequences: layout.sequences,
        ctx: layout.ctx,
        channel_lines: map.channel_lines.clone(),
        streaming,
        contexts,
        fp_unit: layout.numerics.fp_unit,
    };
    let lines: Vec<String> = p.channel_lines.iter().map(u64::to_string).collect();
    let text = format!(
        "knobs {}\nspace {}\nbase {}\ngranule_log2 {}\njob_words {}\nresidual {} {}\nlogits {} {}\ntokens {}\nhidden {}\n\
         vocab {}\nring_rows {}\nregions {}\nregion {}\nsequences {}\nctx {}\nchannels {}\nstore {}\n",
        p.knobs,
        p.space,
        p.base,
        p.granule_log2,
        p.job_words,
        p.residual.0,
        p.residual.1,
        p.logits.0,
        p.logits.1,
        p.tokens,
        p.hidden,
        p.vocab,
        p.ring_rows,
        p.regions,
        p.region,
        p.sequences,
        p.ctx,
        lines.join(" "),
        p.store_beats
    );
    let text = if p.streaming.is_empty() { text } else { format!("{text}streaming {}\n", p.streaming) };
    let text = if p.greedy { format!("{text}greedy 1\n") } else { text };
    let text = if p.contexts > 1 { format!("{text}contexts {}\n", p.contexts) } else { text };
    let text = if p.fp_unit == super::numerics::FpUnit::Hard { format!("{text}fp hard\n") } else { text };
    std::fs::write(dir.join("engine.txt"), text)?;
    let (source, _, _) = machine.identity.cut.write(&machine.spec, "");
    assert_eq!(
        crate::model::program::Identity::of(machine.identity.cut, &source),
        machine.identity,
        "the machine's step is read off the program its identity names"
    );
    std::fs::write(dir.join("program.lg"), source)?;
    std::fs::write(dir.join("program.txt"), format!("{}\n", machine.identity.line()))?;
    Ok(p)
}

/// The status code of a completed command, and of one whose op the core does not know.
pub const STATUS_OK: u8 = 0x00;
/// The code of a running command's progress record: the core cuts one every `2^HB` of its cycles in which the
/// command's lines, its host lines or the engine's sequencer moved, so a host waits while a long command moves.
pub const STATUS_RUNNING: u8 = 0x01;
pub const STATUS_UNKNOWN_OP: u8 = 0x80;
/// ERRORS bit 0: a doorbell rang with the command FIFO full, and was dropped.
pub const ERROR_COMMAND_DROPPED: u64 = 1;

/// The clock-domain map's path among [`afu_sources`].
pub const CLOCK_DOMAINS_FILE: &str = "hw/logos_d5005_domains.json";

/// One emitted file: its path under the AFU's root and its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AfuFile {
    pub path: &'static str,
    pub text: String,
}

/// Every file of the accelerator under `cfg`.
pub fn afu_sources(cfg: &AfuConfig) -> Vec<AfuFile> {
    assert!(cfg.compute_divide == 1 || (cfg.compute_divide >= 2 && cfg.compute_divide % 2 == 0), "a compute clock divided by {}", cfg.compute_divide);
    assert!(cfg.max_burst.is_power_of_two() && cfg.max_burst <= 1 << (BANK_BURST_BITS - 1), "{} lines a burst", cfg.max_burst);
    assert!(1u32 << cfg.read_fifo_log2 >= 2 * cfg.max_burst, "a read-data FIFO holds two bursts");
    if cfg.engine.is_some() {
        assert!(cfg.compute_divide >= ENGINE_MIN_DIVIDE, "an engine's clock divided by {}, under {ENGINE_MIN_DIVIDE}", cfg.compute_divide);
    }
    if cfg.hybrid() {
        assert!(!cfg.line.secded(), "a hybrid engine's streaming unit reads lines without SECDED verdicts");
    }
    let file = |path, text| AfuFile { path, text };
    let mut files = vec![
        file("hw/rtl/sources.txt", sources_txt(cfg)),
        file("hw/rtl/logos_d5005.json", afu_json(cfg)),
        file("hw/rtl/ofs_plat_afu.sv", ofs_plat_afu(cfg)),
        file("hw/rtl/logos_d5005_shell.v", shell_text(cfg.engine.as_ref(), cfg.compute_divide, cfg.contexts())),
        file("hw/rtl/logos_dcfifo.v", dcfifo_verilog()),
        file("hw/rtl/logos_secded.v", secded_verilog()),
        file("hw/rtl/logos_stream.v", STREAM.to_string()),
        file("hw/rtl/logos_csr.v", csr_verilog()),
        file("hw/rtl/logos_bringup_core.v", core_contexts(&core_verilog(), cfg.contexts())),
        file("hw/rtl/logos_d5005.sdc", sdc_text(cfg, &shell_text(cfg.engine.as_ref(), cfg.compute_divide, cfg.contexts()))),
        file(CLOCK_DOMAINS_FILE, clock_domains_json(cfg)),
        file("sw/logos_d5005.h", c_header(cfg)),
        file("sw/logos_hal.h", HAL_H.to_string()),
        file("sw/logos_hal_opae.c", HAL_OPAE.to_string()),
        file("sw/logos_hal_pci.c", HAL_PCI.to_string()),
        file("sw/logos_core.h", CORE_H.to_string()),
        file("sw/logos_receipt.c", RECEIPT.to_string()),
        file("sw/logos_load.c", LOAD.to_string()),
        file("sim/logos_hal_sim.h", HAL_SIM.to_string()),
        file("sim/logos_hal_vpi.c", HAL_VPI.to_string()),
        file("sim/logos_hal_dpi.c", HAL_DPI.to_string()),
        file("sim/logos_d5005_cosim.v", cosim_verilog(cfg)),
        file("sim/logos_avmm_memory_model.v", MEMORY_MODEL.to_string()),
        file("sim/logos_ddr4_channel_model.v", verilog_model(&Ddr4Grade::D5005_EMIF.at(), LOOKAHEAD)),
    ];
    if cfg.compute_divide > 1 {
        files.push(file("hw/rtl/logos_compute_clock.v", COMPUTE_CLOCK_VERILOG.to_string()));
    }
    if let Some(plan) = &cfg.floorplan {
        files.push(file("hw/rtl/logos_floorplan.tcl", plan.clone()));
    }
    if let Some(p) = &cfg.engine {
        assert_eq!(cfg.line.bits(), 512, "the engine's card port moves lines of 64 data bytes");
        files.push(file("hw/rtl/logos_card_port.v", card_port_verilog()));
        files.push(file("hw/rtl/logos_settings.tcl", ENGINE_SETTINGS.to_string()));
        let engine = engine_verilog(p);
        files.push(file("sim/logos_engine_gated.v", simulation_twin(&engine)));
        files.push(file("hw/rtl/logos_engine.v", engine));
        if let Some(hard) = p.streaming.as_ref().and_then(|s| s.hard_float) {
            files.push(file("sim/logos_fp_mac_model.v", hard_float_model(&hard)));
        }
        files.push(file("sw/logos_step.c", STEP.to_string()));
    }
    files
}

/// The shell, its compute side stepping at the user clock divided by `divide` ([`compute_step`]), with the engine and
/// its card port behind the core when `engine` ([`engine_shell`]); a hybrid engine's with every channel's fast port
/// ([`fast_channels`]).
fn shell_text(engine: Option<&EngineParams>, divide: u32, contexts: usize) -> String {
    let text = SHELL
        .replace("/*ENGINE_PARAM*/", &format!("  localparam ENGINE = {};", engine.is_some() as u32))
        .replace("/*COMPUTE*/", &compute_step(divide))
        .replace("/*ENGINE*/", if engine.is_some() { SHELL_ENGINE } else { SHELL_NO_ENGINE });
    let text = if contexts > 1 {
        assert_eq!(text.matches("  wire [15:0]   eng_rows;\n").count(), 1, "the shell's engine rows");
        text.replace("  wire [15:0]   eng_rows;\n", &format!("  wire [{}:0]   eng_rows;\n", 16 * contexts - 1))
    } else {
        text
    };
    let shell = match engine {
        Some(p) if p.streaming.is_some() => fast_channels(&engine_shell(&text), p.streaming.as_ref().is_some_and(|s| s.recurrence)),
        Some(_) => engine_shell(&text),
        None => text,
    };
    if engine.and_then(|p| p.streaming.as_ref()).is_some_and(|s| s.set.pe_pairs) {
        pe_pairs_shell(&shell)
    } else {
        shell
    }
}

/// The shell `text` of an engine whose streaming unit pairs its cores ([`AfuConfig::pe_pairs`]): `uclk2`, twice `uclk`,
/// their rising edges together, given to the engine for its pairs' processing elements.
fn pe_pairs_shell(text: &str) -> String {
    edited(
        text,
        &[
            (
                "  input                  uclk,\n  input                  uclk_rst_n,\n",
                "  input                  uclk,\n  input                  uclk_rst_n,\n  input                  uclk2,            // twice uclk, their rising edges together\n",
            ),
            (".fclk(uclk), .frst(frst),", ".fclk(uclk), .clk2(uclk2), .frst(frst),"),
        ],
    )
}

/// The shell of a hybrid engine build at the engine's divide, with the fast ports' writes when `recurrence`: the shell
/// [`afu_sources`] writes for an engine whose streaming unit is built so.
pub fn hybrid_shell_verilog(recurrence: bool) -> String {
    let text = SHELL
        .replace("/*ENGINE_PARAM*/", "  localparam ENGINE = 1;")
        .replace("/*COMPUTE*/", &compute_step(ENGINE_COMPUTE_DIVIDE))
        .replace("/*ENGINE*/", SHELL_ENGINE);
    fast_channels(&engine_shell(&text), recurrence)
}

/// The shell's compute step: `cce` high in the user cycles that end with the rising edges of the user clock divided by
/// `divide`, `fmid` in those that end with its falling edges, and `crst_n` the compute side's reset, released at the
/// second rising edge ([`COMPUTE_CLOCK_VERILOG`], its divided clock's level read by nothing), the marks and the release
/// [`MARK_LEAD`] user clocks on through registers placed beside their loads; every user cycle, from the user clock's
/// reset, undivided.
fn compute_step(divide: u32) -> String {
    if divide == 1 {
        return "  // The compute side steps every user cycle, from the user clock's reset.\n  wire cce = 1'b1;\n  wire crst_n = uclk_rst_n;".to_string();
    }
    format!(
        "  // The compute side's step: the user cycles that end with the rising edges of the user clock divided by {divide},\n  \
         // those that end with its falling edges, and the compute side's reset, released at the second rising edge; the\n  \
         // divided clock's level (cphase) clocks and feeds nothing.\n  wire cce, fmid, crst_n, cphase;\n  \
         logos_compute_clock #(.DIVIDE({divide}), .LEAD({MARK_LEAD})) compute_clock(.clk(uclk), .rst_n(uclk_rst_n), .cclk(cphase),\n    \
         .cclk_rst_n(crst_n), .fedge(cce), .fmid(fmid));"
    )
}

/// The user clocks a divided build's compute-clock marks and compute-side reset release come out of the divider late:
/// registers of no reset between the divider and the marks' tens of thousands of loads, which the retimer may move
/// along the wire and the last of which Quartus copies beside them.
pub const MARK_LEAD: u32 = 2;

/// `text` with every `(from, to)` of `edits` made, each `from` there exactly once.
fn edited(text: &str, edits: &[(&str, &str)]) -> String {
    let mut out = text.to_string();
    for (from, to) in edits {
        assert_eq!(out.matches(from).count(), 1, "the shell's `{from}`");
        out = out.replace(from, to);
    }
    out
}

/// The shell `text` of an engine build: the engine given the user clock it steps in, from the user clock's reset two
/// registers on, the last copied by Quartus a copy each 256 of its loads, so no reset wire crosses the die, and the
/// compute clock's marks of the user cycles that end with its rising and falling edges.
fn engine_shell(text: &str) -> String {
    edited(
        text,
        &[
            ("  logos_engine engine(", FAST_RESET),
            ("    .mem_wr_ready(e_wr_ready));\n", "    .mem_wr_ready(e_wr_ready), .fclk(uclk), .frst(frst), .fedge(cce), .fmid(fmid));\n"),
        ],
    )
}

/// The engine's user-clock side's reset, ahead of the engine's instance: the user clock's reset two registers on, the
/// second to be copied a copy each 256 loads.
const FAST_RESET: &str = "  // The engine's user-clock side's reset, the user clock's two clocks on; Quartus copies the last register a copy each\n  \
                          // 256 of its loads, each placed beside them.\n  reg frst_a;\n  (* maxfan = 256 *) reg frst;\n  \
                          always @(posedge uclk) begin frst_a <= !uclk_rst_n; frst <= frst_a; end\n  logos_engine engine(";

/// The shell `text` for a hybrid engine: each channel a fast port — a descriptor FIFO into the channel's clock and a
/// read-data FIFO out of it, both the user clock's on the engine's side — its stream engine serving the core's FIFOs
/// and the fast port's one side at a time: idle, with nothing offered from the side it serves and a descriptor offered
/// from the other, it turns to the other, and a line read goes back through the side that asked. With `recurrence`
/// the fast port writes as well ([`fast_writes`]).
fn fast_channels(text: &str, recurrence: bool) -> String {
    let edits: [(&str, &str); 8] = [
        (
            "// a whole run of bursts the channel has in flight. The FIFOs' sides toward the core step at its step (uce).\n",
            "// a whole run of bursts the channel has in flight. The FIFOs' sides toward the core step at its step (uce). A hybrid\n\
             // engine's streaming unit reads the channel through a fast port: FIFOs of its own in the fast clock, every cycle\n\
             // of it, the stream engine serving one side at a time, turning only idle.\n",
        ),
        (
            "  input              mdone_ready,\n",
            "  input              mdone_ready,\n  input              fclk,\n  input              fclk_rst_n,\n  input              fd_valid,\n  \
             output             fd_ready,\n  input  [AW+LW:0]   fd_data,\n  output             fr_valid,\n  input              fr_ready,\n  \
             output [DWL-1:0]   fr_data,\n",
        ),
        (
            "  logos_dcfifo_mlab #(.W(DLW), .AW(2), .HOLDS(1)) desc_fifo(",
            "  // The core's side of the stream engine's descriptors and lines, and the fast port's.\n  \
             wire           sd_valid, sd_ready, fq_valid, fq_ready, sr_push, fr_push, fr_wready;\n  wire [DLW-1:0] sd_data, fq_data;\n  \
             wire [RAW:0]   sr_room, fr_room, fr_used;\n  wire [2:0]     fd_wfree, fd_rused;\n  \
             logos_dcfifo_mlab #(.W(DLW), .AW(2), .HOLDS(1)) desc_fifo(",
        ),
        (".rvalid(d_valid),\n    .rready(d_ready), .rdata(d_data), .rused(d_rused));", ".rvalid(sd_valid),\n    .rready(sd_ready), .rdata(sd_data), .rused(d_rused));"),
        (
            "read_fifo(.wclk(mem_clk), .wce(1'b1), .wrst_n(mem_rst_n), .wvalid(r_push),\n    .wready(r_wready), .wdata(r_word), .wfree(r_room),",
            "read_fifo(.wclk(mem_clk), .wce(1'b1), .wrst_n(mem_rst_n), .wvalid(sr_push),\n    .wready(r_wready), .wdata(r_word), .wfree(sr_room),",
        ),
        (
            "  wire [RAW:0] owed;\n  wire         busy;\n",
            r#"  wire [RAW:0] owed;
  wire         busy;
  // The fast port: its descriptors and lines through FIFOs of their own; the side the stream engine serves. The
  // descriptors' writer is the channel multiplexer, which may offer another port's descriptor at the step after one is
  // refused, the refused one still held at its port: a writer that holds no one word, so its FIFO says neither.
  logos_dcfifo_mlab #(.W(DLW), .AW(2)) fast_desc_fifo(.wclk(fclk), .wce(1'b1), .wrst_n(fclk_rst_n), .wvalid(fd_valid),
    .wready(fd_ready), .wdata(fd_data), .wfree(fd_wfree), .rclk(mem_clk), .rce(1'b1), .rrst_n(mem_rst_n), .rvalid(fq_valid),
    .rready(fq_ready), .rdata(fq_data), .rused(fd_rused));
  logos_dcfifo_m20k #(.W(DWL), .AW(RAW), .HOLDS(1)) fast_read_fifo(.wclk(mem_clk), .wce(1'b1), .wrst_n(mem_rst_n), .wvalid(fr_push),
    .wready(fr_wready), .wdata(r_data), .wfree(fr_room), .rclk(fclk), .rce(1'b1), .rrst_n(fclk_rst_n), .rvalid(fr_valid),
    .rready(fr_ready), .rdata(fr_data), .rused(fr_used));
  reg            fast;
  always @(posedge mem_clk or negedge mem_rst_n)
    if (!mem_rst_n) fast <= 1'b0;
    else if (!busy && !(fast ? fq_valid : sd_valid) && (fast ? sd_valid : fq_valid)) fast <= !fast;
  assign d_valid = fast ? fq_valid : sd_valid;
  assign d_data = fast ? fq_data : sd_data;
  assign sd_ready = d_ready && !fast;
  assign fq_ready = d_ready && fast;
  assign sr_push = r_push && !fast;
  assign fr_push = r_push && fast;
  assign r_room = fast ? fr_room : sr_room;
"#,
        ),
        (
            "  wire [NB*21-1:0]  mw_flip;\n\n  genvar b;\n",
            "  wire [NB*21-1:0]  mw_flip;\n  // Every channel's fast port, the engine's streaming unit's.\n  wire [NB-1:0]     f_md_valid, f_md_ready, f_mr_valid, f_mr_ready;\n  \
             wire [NB*DLW-1:0] f_md_data;\n  wire [NB*DWL-1:0] f_mr_data;\n\n  genvar b;\n",
        ),
        (
            "      .mdone_valid(mdone_valid[b]), .mdone_ready(mdone_ready[b]),\n",
            "      .mdone_valid(mdone_valid[b]), .mdone_ready(mdone_ready[b]),\n      .fclk(uclk), .fclk_rst_n(uclk_rst_n), .fd_valid(f_md_valid[b]), \
             .fd_ready(f_md_ready[b]),\n      .fd_data(f_md_data[b*DLW +: DLW]), .fr_valid(f_mr_valid[b]), .fr_ready(f_mr_ready[b]),\n      \
             .fr_data(f_mr_data[b*DWL +: DWL]),\n",
        ),
    ];
    let out = edited(text, &edits);
    let out = edited(
        &out,
        &[(
            "    .mem_wr_ready(e_wr_ready), .fclk(uclk), .frst(frst), .fedge(cce), .fmid(fmid));\n",
            "    .mem_wr_ready(e_wr_ready), .fclk(uclk), .frst(frst), .fedge(cce), .fmid(fmid), .sm_md_valid(f_md_valid),\n    \
             .sm_md_ready(f_md_ready), .sm_md_data(f_md_data), .sm_mr_valid(f_mr_valid), .sm_mr_ready(f_mr_ready), .sm_mr_data(f_mr_data));\n",
        )],
    );
    if recurrence {
        fast_writes(&out)
    } else {
        out
    }
}

/// The hybrid shell `text` ([`fast_channels`]) with each channel's fast port writing too: its lines through a FIFO of
/// their own into the channel's clock and its writes' done tokens back through another, the stream engine's writer
/// taking the lines of the side it serves and a write done going back to that side; and the engine given the fast
/// ports' writes and the card port's map, which its fast side's port places its lines by.
fn fast_writes(text: &str) -> String {
    edited(
        text,
        &[
            (
                "// of it, the stream engine serving one side at a time, turning only idle.\n",
                "// of it, the stream engine serving one side at a time, turning only idle. Its recurrence unit writes the channel\n\
                 // through the fast port too, a write's done token back to the side that wrote.\n",
            ),
            (
                "  output [DWL-1:0]   fr_data,\n",
                "  output [DWL-1:0]   fr_data,\n  input              fw_valid,\n  output             fw_ready,\n  input  [DWL-1:0]   fw_data,\n  \
                 output             fdone_valid,\n  input              fdone_ready,\n",
            ),
            (".wvalid(done),\n    .wready(done_wready)", ".wvalid(done_core),\n    .wready(done_wready)"),
            ("  wire           r_wready, done_wready, done_token;\n", "  wire           r_wready, done_wready, done_token, done_core;\n"),
            (
                ".w_valid(w_valid), .w_ready(w_ready),\n    .w_data(w_data), .w_flip(w_flip),",
                ".w_valid(e_w_valid), .w_ready(e_w_ready),\n    .w_data(e_w_data), .w_flip(e_w_flip),",
            ),
            ("  assign r_room = fast ? fr_room : sr_room;\n", FAST_WRITES),
            (
                "  wire [NB*DLW-1:0] f_md_data;\n  wire [NB*DWL-1:0] f_mr_data;\n",
                "  wire [NB*DLW-1:0] f_md_data;\n  wire [NB*DWL-1:0] f_mr_data;\n  wire [NB-1:0]     f_mw_valid, f_mw_ready, f_mdone_valid, f_mdone_ready;\n  \
                 wire [NB*DWL-1:0] f_mw_data;\n",
            ),
            (
                "      .fr_data(f_mr_data[b*DWL +: DWL]),\n",
                "      .fr_data(f_mr_data[b*DWL +: DWL]), .fw_valid(f_mw_valid[b]), .fw_ready(f_mw_ready[b]),\n      \
                 .fw_data(f_mw_data[b*DWL +: DWL]), .fdone_valid(f_mdone_valid[b]), .fdone_ready(f_mdone_ready[b]),\n",
            ),
            (
                ".sm_mr_valid(f_mr_valid), .sm_mr_ready(f_mr_ready), .sm_mr_data(f_mr_data));\n",
                ".sm_mr_valid(f_mr_valid), .sm_mr_ready(f_mr_ready), .sm_mr_data(f_mr_data),\n    \
                 .sm_mw_valid(f_mw_valid), .sm_mw_ready(f_mw_ready), .sm_mw_data(f_mw_data), .sm_mdone_valid(f_mdone_valid),\n    \
                 .sm_mdone_ready(f_mdone_ready), .map_space(port_space), .map_base(port_base), .map_granule_log2(port_granule_log2));\n",
            ),
        ],
    )
}

/// A channel's fast writes ([`fast_writes`]): the FIFOs and the writer's side.
const FAST_WRITES: &str = r#"  assign r_room = fast ? fr_room : sr_room;
  // The fast port's lines to write through a FIFO of their own and its writes' done tokens back through another: the
  // stream engine's writer takes the lines of the side it serves, and a write done goes back to that side.
  wire           fw_q_valid, fw_q_ready, fdone_wready, fdone_token, e_w_valid, e_w_ready;
  wire [DWL-1:0] fw_q_data, e_w_data;
  wire [20:0]    e_w_flip;
  wire [WAW:0]   fw_wfree, fw_rused;
  wire [2:0]     fdone_wfree, fdone_rused;
  logos_dcfifo_m20k #(.W(DWL), .AW(WAW), .HOLDS(1)) fast_write_fifo(.wclk(fclk), .wce(1'b1), .wrst_n(fclk_rst_n), .wvalid(fw_valid),
    .wready(fw_ready), .wdata(fw_data), .wfree(fw_wfree), .rclk(mem_clk), .rce(1'b1), .rrst_n(mem_rst_n),
    .rvalid(fw_q_valid), .rready(fw_q_ready), .rdata(fw_q_data), .rused(fw_rused));
  logos_dcfifo_mlab #(.W(1), .AW(2), .TOKENS(1)) fast_done_fifo(.wclk(mem_clk), .wce(1'b1), .wrst_n(mem_rst_n), .wvalid(done && fast),
    .wready(fdone_wready), .wdata(1'b1), .wfree(fdone_wfree), .rclk(fclk), .rce(1'b1), .rrst_n(fclk_rst_n),
    .rvalid(fdone_valid), .rready(fdone_ready), .rdata(fdone_token), .rused(fdone_rused));
  assign e_w_valid = fast ? fw_q_valid : w_valid;
  assign e_w_data = fast ? fw_q_data : w_data;
  assign e_w_flip = fast ? 21'h0 : w_flip;
  assign w_ready = e_w_ready && !fast;
  assign fw_q_ready = e_w_ready && fast;
  assign done_core = done && !fast;
"#;

/// The user clock's period in the co-simulation, picoseconds: the clock the AFU JSON asks for, the shell dividing it as
/// the build divides it, so a divided build's compute side meets its memory channels at the ratio it does on the card.
pub fn cosim_user_clock_ps(cfg: &AfuConfig) -> u64 {
    (1e6 / cfg.user_clock_mhz as f64).round() as u64 / 2 * 2
}

/// The module that stands behind channel `b` of the co-simulation, and its parameters.
fn channel_model(model: ChannelModel) -> &'static str {
    match model {
        ChannelModel::Random => "logos_avmm_memory_model #(.AW(AW), .DW(DWM), .BW(MBW), .LINES_LOG2(20), .SEED(100 + b))",
        ChannelModel::Ddr4 => "logos_ddr4_channel_model #(.AW(AW), .DW(DWM), .BW(MBW), .LINES_LOG2(20), .LOG(DDR4_LOG), .ID(b))",
    }
}

fn cosim_verilog(cfg: &AfuConfig) -> String {
    [
        ("{mem_bits}", cfg.line.bus_bits().to_string()),
        ("{ecc}", (cfg.line.secded() as u32).to_string()),
        ("{id_l}", format!("{:016x}", cfg.uuid as u64)),
        ("{id_h}", format!("{:016x}", (cfg.uuid >> 64) as u64)),
        ("{build}", format!("{:016x}", cfg.build)),
        ("{uclk_ps}", cosim_user_clock_ps(cfg).to_string()),
        ("{divide}", cfg.compute_divide.to_string()),
        ("{channel_model}", channel_model(cfg.channel_model).to_string()),
        (
            "{uclk2_clock}",
            if cfg.pe_pairs() {
                assert_eq!(cosim_user_clock_ps(cfg) % 4, 0, "a paired engine's twice clock takes a quarter of the user period");
                "  // The paired processing elements' clock, twice uclk, its rising edges on uclk's and between them.\n  \
                 reg uclk2 = 1;\n  always #(UCLK_PS / 4) uclk2 = ~uclk2;\n"
                    .to_string()
            } else {
                String::new()
            },
        ),
        ("{uclk2_cosim}", if cfg.pe_pairs() { " .uclk2(uclk2),".to_string() } else { String::new() }),
    ]
    .iter()
    .fold(COSIM_TEMPLATE.to_string(), |text, (marker, value)| text.replace(marker, value))
}

/// The Verilog every simulation of the shell reads: the shell and all it instantiates, in order.
pub fn shell_verilog() -> Vec<(&'static str, String)> {
    vec![
        ("logos_d5005_shell.v", shell_text(None, 1, 1)),
        ("logos_dcfifo.v", dcfifo_verilog()),
        ("logos_secded.v", secded_verilog()),
        ("logos_stream.v", STREAM.to_string()),
        ("logos_csr.v", csr_verilog()),
        ("logos_bringup_core.v", core_verilog()),
    ]
}

/// The files `afu_synth_setup` reads, in the order Quartus compiles them.
fn sources_txt(cfg: &AfuConfig) -> String {
    let engine: &[&str] = if cfg.engine.is_some() { &["logos_engine.v", "logos_card_port.v"] } else { &[] };
    let divider: &[&str] = if cfg.compute_divide > 1 { &["logos_compute_clock.v"] } else { &[] };
    let floorplan: &[&str] = if cfg.floorplan.is_some() { &["QI:logos_floorplan.tcl"] } else { &[] };
    let settings: &[&str] = if cfg.engine.is_some() { &["QI:logos_settings.tcl"] } else { &[] };
    let exercisers = format!("QI:{FIM_STATIC_EXERCISERS}");
    let flat: &[&str] = if cfg.flat { &[exercisers.as_str()] } else { &[] };
    ["logos_d5005.json", "logos_dcfifo.v", "logos_secded.v", "logos_stream.v", "logos_csr.v", "logos_bringup_core.v"]
        .iter()
        .chain(engine)
        .chain(divider)
        .chain(&["logos_d5005_shell.v", "ofs_plat_afu.sv", "logos_d5005.sdc"])
        .chain(settings)
        .chain(floorplan)
        .chain(flat)
        .map(|f| format!("{f}\n"))
        .collect()
}

/// An engine build's project settings, as Tcl its build sources (`hw/rtl/logos_settings.tcl`, named in sources.txt as
/// `QI:`). The engine's memories of thirty-two words or fewer read without a register after them are in MLABs checking
/// no read-during-write (`rtl::ram::RAMSTYLE`, `LOGOS_RAMSTYLE_ASYNC`), which Quartus Prime Pro otherwise leaves untimed
/// between an MLAB's write and a read of the same word: timed, a word read in the clock after its write is the one
/// written, as the RTL reads it.
pub const ENGINE_SETTINGS: &str = "# The engine's MLABs read without a register after them: an MLAB's write and a read of the same word timed, so a word
# read in the clock after its write is the one written, as the RTL reads it.
set_global_assignment -name MLAB_ADD_TIMING_CONSTRAINTS_FOR_MIXED_PORT_FEED_THROUGH_MODE_SETTING_DONT_CARE ON
";

/// The SDC: the FIFO crossings, and an engine build's engine stepping in the user clock at the compute clock's edges
/// ([`ENGINE_SDC`]). No clock is generated: every clock is the platform's.
fn sdc_text(cfg: &AfuConfig, shell: &str) -> String {
    let step = if cfg.compute_divide > 1 { compute_step_sdc(cfg.compute_divide, shell) } else { String::new() };
    if cfg.engine.is_none() {
        return format!("{SDC}{step}");
    }
    let d = cfg.compute_divide;
    let engine = [("{half}", d / 2 - 1), ("{half_hold}", d / 2 - 2), ("{held}", d / 2 - 2), ("{held_hold}", d / 2 - 3)]
        .iter()
        .fold(ENGINE_SDC.to_string(), |t, (m, v)| t.replace(m, &v.to_string()));
    format!("{SDC}{step}{engine}")
}

/// The sides of the shell's dual-clock FIFOs that step at the compute clock's step, from the shell's text: each FIFO
/// instance's name and its side, `'w'` its writes' and `'r'` its reads', clocked by the user clock under the compute
/// side's enable (`cce`, a channel's `uce`). Every other side runs on its own clock or every user cycle.
pub fn compute_fifo_sides(shell: &str) -> Vec<(String, char)> {
    let mut sides = Vec::new();
    for (at, _) in shell.match_indices("logos_dcfifo_") {
        let text = &shell[at..];
        let text = &text[..text.find(");").unwrap_or(text.len())];
        let Some(open) = text.find(") ") else { continue };
        let name: String = text[open + 2..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        let port = |p: &str| text.find(&format!(".{p}(")).map(|i| text[i + p.len() + 2..].split(')').next().unwrap_or("").trim().to_string());
        let compute = |clk: &str, ce: &str| port(clk).as_deref() == Some("uclk") && matches!(port(ce).as_deref(), Some("cce") | Some("uce"));
        if name.is_empty() {
            continue;
        }
        if compute("wclk", "wce") {
            sides.push((name.clone(), 'w'));
        }
        if compute("rclk", "rce") {
            sides.push((name, 'r'));
        }
    }
    sides.sort();
    sides.dedup();
    sides
}

/// The registers of a FIFO's side: its pointer, the other side's pointer crossing in and its binary, its RAM, and on the
/// read side the word and valid it holds.
pub const FIFO_SIDE_REGISTERS: [(char, &[&str]); 2] =
    [('w', &["wbin", "wgray", "rgray_s1", "rgray_s2", "rbin_w"]), ('r', &["rbin", "rgray", "wgray_s1", "wgray_s2", "wbin_r", "rdata_q", "rvalid_q"])];

/// The shell's compute side in a divided build: the core, the card port and the FIFO sides [`compute_fifo_sides`] names,
/// every register of which steps only at the compute clock's step, `divide` user clocks apart, so a path between two of
/// them has that many user clocks; a RAM's ports are its side's (its write port written only at the step, its read
/// port read only then). A path from or to anything else — the compute clock's marks, the engine and its crossing, a
/// FIFO's other side, the fast side — keeps its own constraint.
fn compute_step_sdc(divide: u32, shell: &str) -> String {
    let mut sides = String::new();
    for (name, side) in compute_fifo_sides(shell) {
        sides.push_str(&format!(" {name} {side}"));
    }
    let regs = |side: char| FIFO_SIDE_REGISTERS.iter().find(|(s, _)| *s == side).map_or(String::new(), |(_, r)| r.join(" "));
    COMPUTE_STEP_SDC
        .replace("{sides}", sides.trim_start())
        .replace("{w_regs}", &regs('w'))
        .replace("{r_regs}", &regs('r'))
        .replace("{setup}", &divide.to_string())
        .replace("{hold}", &(divide - 1).to_string())
}

/// The compute side's paths, each the compute clock's period.
const COMPUTE_STEP_SDC: &str = r##"
# The shell's compute side steps only at the compute clock's step: a path between two of its registers has the step's
# user clocks.
set logos_step [get_keepers -nowarn {*|shell|core|* *|shell|port|*}]
foreach {name side} {{sides}} {
  foreach entity {logos_dcfifo_mlab logos_dcfifo_m20k} {
    foreach inst [get_entity_instances $entity] {
      if {![string match "*|$name" $inst]} { continue }
      set regs [expr {$side eq "w" ? {{w_regs}} : {{r_regs}}}]
      foreach reg $regs {
        set logos_step [add_to_collection $logos_step [get_keepers -nowarn $inst|$reg]]
        set logos_step [add_to_collection $logos_step [get_keepers -nowarn $inst|$reg[*]]]
      }
      set logos_step [add_to_collection $logos_step [get_keepers -nowarn $inst|mem*]]
    }
  }
}
if {[get_collection_size $logos_step] > 0} {
  set_multicycle_path -from $logos_step -to $logos_step -setup -end {setup}
  set_multicycle_path -from $logos_step -to $logos_step -hold -end {hold}
}
"##;

/// The user clock request: the fitter's best up to the clock asked for, or exactly it for a divided build.
/// The user clocks the AFU JSON asks for, {high, low}: auto up to the bring-up build's target, or a divided build's
/// user clock and its half, the pair the PIM's `user_clock_config.tcl` takes (a fixed high clock with an auto low one
/// fails its build).
fn clock_request(cfg: &AfuConfig) -> (String, String) {
    if cfg.pe_pairs() {
        // The user side in the low clock, the paired processing elements in the high one, twice it.
        return ((2 * cfg.user_clock_mhz).to_string(), cfg.user_clock_mhz.to_string());
    }
    if cfg.compute_divide == 1 {
        (format!("\"auto-{}\"", cfg.user_clock_mhz), "\"auto\"".to_string())
    } else {
        let low = if cfg.user_clock_mhz % 2 == 0 { (cfg.user_clock_mhz / 2).to_string() } else { format!("{}.5", cfg.user_clock_mhz / 2) };
        (cfg.user_clock_mhz.to_string(), low)
    }
}

fn afu_json(cfg: &AfuConfig) -> String {
    format!(
        "{{\n  \"version\": 1,\n  \"afu-image\": {{\n    \"power\": 0,\n    \"clock-frequency-high\": {high},\n    \
         \"clock-frequency-low\": {low},\n    \"afu-top-interface\": {{\n      \"class\": \"ofs_plat_afu\"\n    }},\n    \
         \"accelerator-clusters\": [\n      {{\n        \"name\": \"{name}\",\n        \"total-contexts\": 1,\n        \
         \"accelerator-type-uuid\": \"{uuid}\"\n      }}\n    ]\n  }}\n}}\n",
        high = clock_request(cfg).0,
        low = clock_request(cfg).1,
        name = cfg.name,
        uuid = cfg.uuid_text(),
    )
}

fn ofs_plat_afu(cfg: &AfuConfig) -> String {
    let lanes = match cfg.line {
        LocalMemoryLine::Data512 => "its data bits",
        LocalMemoryLine::DataAndEcc576 => "its ECC lanes carried as data",
        LocalMemoryLine::Data512Secded => "its ECC lanes holding each line's SECDED check bytes",
    };
    [
        ("{lanes}", lanes.to_string()),
        ("{mem_params}", cfg.line.pim_params().to_string()),
        ("{mem_width}", cfg.line.pim_width().to_string()),
        ("{ecc}", (cfg.line.secded() as u32).to_string()),
        ("{host_burst_bits}", BANK_BURST_BITS.to_string()),
        ("{max_burst}", cfg.max_burst.to_string()),
        ("{read_fifo_log2}", cfg.read_fifo_log2.to_string()),
        ("{write_fifo_log2}", cfg.write_fifo_log2.to_string()),
        ("{id_l}", format!("{:016x}", cfg.uuid as u64)),
        ("{id_h}", format!("{:016x}", (cfg.uuid >> 64) as u64)),
        ("{build}", format!("{:016x}", cfg.build)),
        ("{user_clocks}", user_clocks(cfg).to_string()),
        ("{uclk2}", if cfg.pe_pairs() { " .uclk2(uclk2),".to_string() } else { String::new() }),
    ]
    .iter()
    .fold(AFU_TEMPLATE.to_string(), |text, (marker, value)| text.replace(marker, value))
}

/// The PIM's user clocks the accelerator runs on: `uclk` its user side's, and for an engine whose cores are paired
/// ([`AfuConfig::pe_pairs`]) `uclk2`, twice it, one PLL's two outputs, their rising edges together: `uClk_usrDiv2` and
/// `uClk_usr`; otherwise `uClk_usr` alone.
fn user_clocks(cfg: &AfuConfig) -> &'static str {
    if cfg.pe_pairs() {
        "  // The user side in uClk_usrDiv2 and the engine's paired processing elements in uClk_usr, twice it: one PLL's two\n  \
         // outputs, their rising edges together.\n  wire uclk = plat_ifc.clocks.ports[0].uClk_usrDiv2.clk;\n  \
         wire uclk_rst_n = plat_ifc.clocks.ports[0].uClk_usrDiv2.reset_n;\n  wire uclk2 = plat_ifc.clocks.ports[0].uClk_usr.clk;"
    } else {
        "  wire uclk = plat_ifc.clocks.ports[0].uClk_usr.clk;\n  wire uclk_rst_n = plat_ifc.clocks.ports[0].uClk_usr.reset_n;"
    }
}

/// The user clock divided by `DIVIDE` (even), as marks in the user clock: a register toggled every `DIVIDE / 2` user
/// clock edges (the divided clock, a level that clocks nothing); `fedge` and `fmid` high in the user cycles that end
/// with its rising and falling edges, where the shell's compute side and an engine's core step and an engine's step
/// boundary takes the engine's values; and a reset asserted with the user clock's and released in the user clock at the
/// second divided rising edge after it.
pub const COMPUTE_CLOCK_VERILOG: &str = r#"module logos_compute_clock #(parameter DIVIDE = 2, parameter LEAD = 0)
  (input clk, input rst_n, output reg cclk, output cclk_rst_n, output fedge, output fmid);
  localparam W = (DIVIDE > 2) ? $clog2(DIVIDE / 2) : 1;
  localparam [W-1:0] LAST = DIVIDE / 2 - 1;
  reg [W-1:0] count;
  always @(posedge clk or negedge rst_n)
    if (!rst_n) begin count <= {W{1'b0}}; cclk <= 1'b0; end
    else if (count == LAST) begin count <= {W{1'b0}}; cclk <= ~cclk; end
    else count <= count + 1'b1;
  // The marks registered, each the next cycle's, so an enable of the whole engine leaves a register.
  wire [W-1:0] count_next = (count == LAST) ? {W{1'b0}} : count + 1'b1;
  wire cclk_next = (count == LAST) ? ~cclk : cclk;
  (* maxfan = 2000 *) reg mark_edge, mark_mid;
  always @(posedge clk or negedge rst_n)
    if (!rst_n) begin mark_edge <= LAST == {W{1'b0}}; mark_mid <= 1'b0; end
    else begin mark_edge <= count_next == LAST && !cclk_next; mark_mid <= count_next == LAST && cclk_next; end
  reg [1:0] release_q;
  always @(posedge clk or negedge rst_n)
    if (!rst_n) release_q <= 2'b00; else if (mark_edge) release_q <= {release_q[0], 1'b1};
  // LEAD > 0: the marks and the reset's release LEAD user clocks on, through registers of no reset the retimer may move
  // along the wire and the last of which Quartus copies beside its loads, so no mark crosses the die in one clock; the
  // reset still asserted the moment the user clock's is. Every consumer sees the divider LEAD clocks late, alike.
  generate
    if (LEAD == 0) begin : now
      assign fedge = mark_edge;
      assign fmid = mark_mid;
      assign cclk_rst_n = release_q[1];
    end else begin : late
      (* maxfan = 64 *) reg [LEAD-1:0] edge_d, mid_d;
      reg [LEAD-1:0] release_d;
      always @(posedge clk) begin
        edge_d <= (edge_d << 1) | {{(LEAD-1){1'b0}}, mark_edge};
        mid_d <= (mid_d << 1) | {{(LEAD-1){1'b0}}, mark_mid};
        release_d <= (release_d << 1) | {{(LEAD-1){1'b0}}, release_q[1]};
      end
      assign fedge = edge_d[LEAD-1];
      assign fmid = mid_d[LEAD-1];
      assign cclk_rst_n = release_q[1] && release_d[LEAD-1];
    end
  endgenerate
endmodule
"#;

/// An engine build's engine, its core stepping in the user clock at the compute clock's rising edges and reading its
/// memories the user cycle after the falling edges: the paths out of its step boundary's held values (`held_*`), which
/// end in the shell's compute side, taking them only at its step, have half a compute cycle less two; those within the
/// core, between it and the user-clock side, and from the boundary's captures (`cap_*`) into either, half a compute
/// cycle less one, the least of them: from a step to the next read, from a read to the next step. The shell's paths
/// into the captures keep their one user cycle.
const ENGINE_SDC: &str = r##"
# The engine stepping in the user clock at the compute clock's edges, and its step boundary with the shell.
set logos_cap [get_keepers -nowarn {*|engine|bridge|cap_*}]
set logos_held [get_keepers -nowarn {*|engine|bridge|held_*}]
set logos_core [get_keepers -nowarn {*|engine|core|*}]
set logos_fast [get_keepers -nowarn {*|engine|fast|*}]
if {[get_collection_size $logos_held] > 0} {
  set_multicycle_path -from $logos_held -setup -end {held}
  set_multicycle_path -from $logos_held -hold -end {held_hold}
}
foreach {from to} [list $logos_cap $logos_core $logos_core $logos_core $logos_core $logos_fast $logos_fast $logos_core $logos_cap $logos_fast] {
  if {[get_collection_size $from] > 0 && [get_collection_size $to] > 0} {
    set_multicycle_path -from $from -to $to -setup -end {half}
    set_multicycle_path -from $from -to $to -hold -end {half_hold}
  }
}
"##;

fn dcfifo_verilog() -> String {
    [("logos_dcfifo_mlab", "MLAB"), ("logos_dcfifo_m20k", "M20K")]
        .iter()
        .map(|(name, style)| DCFIFO.replace("MODULE_NAME", name).replace("RAMSTYLE", style))
        .collect::<Vec<_>>()
        .join("\n")
}

fn csr_verilog() -> String {
    let map: Vec<String> = REGISTERS.iter().map(|r| format!("{} = {}", r.name, r.word)).collect();
    CSR.replace("/*REGISTER_MAP*/", &format!("localparam [MAW-1:0] {};", map.join(", ")))
}

/// The bring-up core `text` ([`core_verilog`]'s) for an engine of `contexts` sequencer contexts: ENGINE_CONFIG takes
/// context c's token-rows at arg0[32 + 16(c − 1) +: 16] beside context 0's, and ENGINE_TABLE and STORE_READ the context
/// at arg1[63:56], given the engine in its host address's top four bits; `text` itself for one.
fn core_contexts(text: &str, contexts: usize) -> String {
    if contexts == 1 {
        return text.to_string();
    }
    let rows: Vec<String> = (1..contexts).rev().map(|c| format!("cmd_data[{} +: 16]", 96 + 16 * (c - 1))).chain(["cmd_data[64 +: 16]".to_string()]).collect();
    let mut out = text.to_string();
    for (from, to) in [
        ("  output reg [15:0]         eng_rows,", format!("  output reg [{}:0]         eng_rows,", 16 * contexts - 1)),
        ("                  eng_rows <= cmd_data[64 +: 16];\n", format!("                  eng_rows <= {{{}}};\n", rows.join(", "))),
        ("                  tbase <= cmd_data[128 +: 32];\n", "                  tbase <= {cmd_data[184 +: 4], cmd_data[128 +: 28]};\n".to_string()),
        ("                default: sbase <= cmd_data[128 +: 32];\n", "                default: sbase <= {cmd_data[184 +: 4], cmd_data[128 +: 28]};\n".to_string()),
        (
            "//   Any other op completes at once with code 0x80,",
            format!(
                "//   With an engine of {contexts} contexts, ENGINE_CONFIG's context c's token-rows at arg0[32 + 16(c - 1) +: 16] for c >= 1,\n\
                 //   ENGINE_TABLE's and STORE_READ's context at arg1[63:56] (the engine's host address's top four bits), ENGINE_RUN\n\
                 //   starting every context with rows, its word each context's 16 bits, context 0's lowest.\n\
                 //   Any other op completes at once with code 0x80,"
            ),
        ),
    ] {
        assert_eq!(out.matches(from).count(), 1, "`{from}`");
        out = out.replace(from, &to);
    }
    out
}

fn core_verilog() -> String {
    let ops: Vec<String> = Op::ALL.iter().map(|o| format!("{} = {}", o.name(), o.code())).collect();
    CORE.replace(
        "/*OPS*/",
        &format!(
            "localparam [7:0] {};\n  localparam [7:0] UNKNOWN_OP = 8'h{STATUS_UNKNOWN_OP:02x}, RUNNING = 8'h{STATUS_RUNNING:02x};",
            ops.join(", ")
        ),
    )
}

/// Every shell port's clock, and the shell's parameters as the build sets them. A vector port of the memory channels is
/// one channel's per `width` bits.
fn clock_domains_json(cfg: &AfuConfig) -> String {
    let p = |name: &str, clock: &str| format!("    {{\"port\": \"{name}\", \"clock\": \"{clock}\"}}");
    let per_bank = |name: &str, width: &str| format!("    {{\"port\": \"{name}\", \"clock\": \"mem_clk\", \"per_bank\": \"{width}\"}}");
    let mut rows = vec![p("pclk_rst_n", "pclk"), p("uclk_rst_n", "uclk")];
    for port in [
        "mmio_address", "mmio_read", "mmio_write", "mmio_writedata", "mmio_byteenable", "mmio_waitrequest",
        "mmio_readdata", "mmio_readdatavalid", "host_rd_address", "host_rd_read", "host_rd_burstcount",
        "host_rd_waitrequest", "host_rd_readdata", "host_rd_readdatavalid", "host_wr_address", "host_wr_write",
        "host_wr_burstcount", "host_wr_writedata", "host_wr_byteenable", "host_wr_waitrequest",
        "host_wr_writeresponsevalid",
    ] {
        rows.push(p(port, "pclk"));
    }
    for (port, width) in [
        ("mem_rst_n", "1"), ("mem_address", "AW"), ("mem_read", "1"), ("mem_write", "1"), ("mem_burstcount", "MBW"),
        ("mem_writedata", "DWM"), ("mem_byteenable", "DWM/8"), ("mem_waitrequest", "1"), ("mem_readdata", "DWM"),
        ("mem_readdatavalid", "1"),
    ] {
        rows.push(per_bank(port, width));
    }
    if cfg.engine.is_some() {
        // An engine build's engine in the user clock: its core, and a hybrid engine's user-clock side, each proven
        // single-clock and boxed, the step boundary between the shell's compute side and the engine declared.
        let boxes = if cfg.hybrid() { "\"logos_engine_core_ce\", \"logos_engine_fast\"" } else { "\"logos_engine_core_ce\"" };
        // A paired engine's processing elements in uclk2 within the user side's box, proven apart (`cdc_check.py pair`):
        // two cores' (`logos_matvec_pass_x2`), or with token pairs a core's two token-rows' (`logos_matvec_pass_x2t`).
        let pair = if cfg.token_pairs() { "logos_matvec_pass_x2t" } else { "logos_matvec_pass_x2" };
        let (uclk2, pairs) = if cfg.pe_pairs() { (" \"uclk2\",".to_string(), format!("\n  \"pairs\": [\"{pair}\"],")) } else { (String::new(), String::new()) };
        return format!(
            "{{\n  \"top\": \"logos_d5005_shell\",\n  \"parameters\": {{\"DWM\": {}, \"ECC\": {}}},\n  \"clocks\": [\"pclk\", \"uclk\",{uclk2} \
             \"mem_clk\"],\n  \"crossings\": [\"logos_dcfifo_mlab\", \"logos_dcfifo_m20k\", \"logos_engine_bridge\"],\n  \
             \"single_clock\": [{boxes}],{pairs}\n  \"ports\": [\n{}\n  ]\n}}\n",
            cfg.line.bus_bits(),
            cfg.line.secded() as u32,
            rows.join(",\n")
        );
    }
    format!(
        "{{\n  \"top\": \"logos_d5005_shell\",\n  \"parameters\": {{\"DWM\": {}, \"ECC\": {}}},\n  \"clocks\": [\"pclk\", \"uclk\", \
         \"mem_clk\"],\n  \"crossings\": [\"logos_dcfifo_mlab\", \"logos_dcfifo_m20k\"],\n  \"ports\": [\n{}\n  ]\n}}\n",
        cfg.line.bus_bits(),
        cfg.line.secded() as u32,
        rows.join(",\n")
    )
}

fn c_header(cfg: &AfuConfig) -> String {
    let mut h = String::from(
        "/* The LOGOS decode engine's bring-up accelerator on the Intel PAC D5005: the ID a driver opens it by, the byte\n   \
         offsets of its MMIO registers, its commands and their status codes. */\n#ifndef LOGOS_D5005_H\n#define LOGOS_D5005_H\n\n",
    );
    h += &format!("#define LOGOS_D5005_AFU_ID \"{}\"\n\n", cfg.uuid_text());
    h += &format!(
        "#define LOGOS_D5005_BANKS {BANKS}\n#define LOGOS_D5005_MEM_LINE_BYTES {}\n#define LOGOS_D5005_HOST_LINE_BYTES {}\n\n",
        cfg.line.bits() / 8,
        HOST_LINE_BITS / 8
    );
    h += &format!(
        "/* Whether each channel line's words are SECDED(72,64)-coded on the bus, and the bus bit of check bit `j` of word\n   \
         `w` (INJECT's bit numbers: data bit `i` of a line is bus bit `i`). */\n#define LOGOS_D5005_SECDED {}\n\
         #define LOGOS_D5005_SECDED_WORDS {SECDED_WORDS}u\n\
         #define LOGOS_D5005_SECDED_CHECK_BIT(w, j) ({} + 8u * ({}u - (w)) + (j))\n\n",
        cfg.line.secded() as u32,
        LocalMemoryLine::Data512Secded.bits(),
        SECDED_WORDS - 1
    );
    h += &format!("/* Whether the core runs the decode engine's steps (ENGINE_CONFIG, ENGINE_TABLE, ENGINE_RUN, STORE_READ). */\n#define LOGOS_D5005_ENGINE {}\n\n", cfg.engine.is_some() as u32);
    h += &format!("/* The divide from the user clock to the clock the core runs at. */\n#define LOGOS_D5005_COMPUTE_DIVIDE {}u\n\n", cfg.compute_divide);
    let (rows, store) = cfg.engine.as_ref().map_or((0, 0), |e| (e.rows, e.store_beats));
    h += &format!(
        "/* The most token-rows a step of the engine holds and its token-row store's beats: a program for more rows, or\n   \
         whose store is larger, cannot run on it. */\n#define LOGOS_D5005_ROWS {rows}u\n#define LOGOS_D5005_STORE_BEATS {store}u\n\n"
    );
    if cfg.contexts() > 1 {
        h += &format!(
            "/* The engine's sequencer contexts, each its own token-rows (LOGOS_D5005_ROWS of them) and bank of the store: a\n   \
             program is written for as many. */\n#define LOGOS_D5005_CONTEXTS {}u\n\n",
            cfg.contexts()
        );
    }
    if cfg.fp_hard() {
        h += "/* The engine's f32 products are the card's floating-point DSP blocks' (FpUnit::Hard): a program is written at\n   \
              them, and one written at the engine's own multiply is refused. */\n#define LOGOS_D5005_FP_HARD 1\n\n";
    }
    let streaming = cfg.engine.as_ref().map_or(String::new(), |e| e.streaming_signature());
    h += &format!(
        "/* A hybrid engine's streaming unit, its column set and the row pairs a channel's stripe holds, as the programs\n   \
         it runs name it in engine.txt; empty for an engine that streams nothing. */\n#define LOGOS_D5005_STREAMING \"{streaming}\"\n\n"
    );
    for r in REGISTERS {
        h += &format!("#define LOGOS_D5005_REG_{} 0x{:04x}u\n", r.name, 8 * r.word);
    }
    h += "\n";
    for o in Op::ALL {
        h += &format!("#define LOGOS_D5005_OP_{} {}u\n", o.name(), o.code());
    }
    h += &format!(
        "\n#define LOGOS_D5005_STATUS_OK 0x{STATUS_OK:02x}u\n#define LOGOS_D5005_STATUS_UNKNOWN_OP 0x{STATUS_UNKNOWN_OP:02x}u\n\
         #define LOGOS_D5005_STATUS_RUNNING 0x{STATUS_RUNNING:02x}u\n\
         #define LOGOS_D5005_ERROR_COMMAND_DROPPED 0x{ERROR_COMMAND_DROPPED:x}u\n\n#endif\n"
    );
    h
}

/// The PIM boundary, its configured values at the `{…}` markers.
const AFU_TEMPLATE: &str = r##"// The engine's boundary with the OFS platform on the D5005: the PIM's host channel as Avalon-MM MMIO and host memory
// in pClk, each local-memory bank as Avalon-MM in its own EMIF clock, {lanes}, and the shell behind them on the user
// clock, which it divides into its compute side's steps itself: every clock the platform's.
`include "ofs_plat_if.vh"

module ofs_plat_afu (ofs_plat_if plat_ifc);
  localparam NB = local_mem_cfg_pkg::LOCAL_MEM_NUM_BANKS;
  localparam AW = local_mem_cfg_pkg::LOCAL_MEM_ADDR_WIDTH;
  localparam DWM = local_mem_cfg_pkg::{mem_width};
  localparam MBW = local_mem_cfg_pkg::LOCAL_MEM_BURST_CNT_WIDTH;
  localparam HAW = ofs_plat_host_chan_pkg::ADDR_WIDTH_LINES;
  localparam DWH = ofs_plat_host_chan_pkg::DATA_WIDTH;
  localparam HBW = {host_burst_bits};

  wire pclk = plat_ifc.clocks.ports[0].pClk.clk;
  wire pclk_rst_n = plat_ifc.clocks.ports[0].pClk.reset_n;
{user_clocks}

  ofs_plat_avalon_mem_rdwr_if #(`HOST_CHAN_AVALON_MEM_RDWR_PARAMS, .BURST_CNT_WIDTH(HBW)) host_mem();
  ofs_plat_avalon_mem_if #(`HOST_CHAN_AVALON_MMIO_PARAMS(64)) mmio64();
  ofs_plat_host_chan_as_avalon_mem_rdwr_with_mmio #(.ADD_CLOCK_CROSSING(0)) host_shim(
    .to_fiu(plat_ifc.host_chan.ports[0]),
    .host_mem_to_afu(host_mem),
    .mmio_to_afu(mmio64),
    .afu_clk(pclk),
    .afu_reset_n(pclk_rst_n));

  ofs_plat_avalon_mem_if #(`{mem_params}) mem[NB]();

  wire [NB-1:0]       mem_clk, mem_rst_n, mem_read, mem_write, mem_waitrequest, mem_readdatavalid;
  wire [NB*AW-1:0]    mem_address;
  wire [NB*MBW-1:0]   mem_burstcount;
  wire [NB*DWM-1:0]   mem_writedata, mem_readdata;
  wire [NB*DWM/8-1:0] mem_byteenable;

  genvar b;
  for (b = 0; b < NB; b = b + 1) begin : bank
    ofs_plat_local_mem_as_avalon_mem #(.ADD_CLOCK_CROSSING(0)) mem_shim(
      .to_fiu(plat_ifc.local_mem.banks[b]),
      .to_afu(mem[b]),
      .afu_clk(uclk),
      .afu_reset_n(uclk_rst_n));
    assign mem_clk[b] = mem[b].clk;
    assign mem_rst_n[b] = mem[b].reset_n;
    assign mem[b].address = mem_address[b*AW +: AW];
    assign mem[b].read = mem_read[b];
    assign mem[b].write = mem_write[b];
    assign mem[b].burstcount = mem_burstcount[b*MBW +: MBW];
    assign mem[b].writedata = mem_writedata[b*DWM +: DWM];
    assign mem[b].byteenable = mem_byteenable[b*DWM/8 +: DWM/8];
    assign mem[b].user = '0;
    assign mem_waitrequest[b] = mem[b].waitrequest;
    assign mem_readdata[b*DWM +: DWM] = mem[b].readdata;
    assign mem_readdatavalid[b] = mem[b].readdatavalid;
  end

  ofs_plat_if_tie_off_unused #(.HOST_CHAN_IN_USE_MASK(1), .LOCAL_MEM_IN_USE_MASK((1 << NB) - 1)) tie_off(plat_ifc);

  assign host_mem.rd_byteenable = '1;
  assign host_mem.rd_user = '0;
  assign host_mem.wr_user = '0;
  assign mmio64.response = '0;
  assign mmio64.readresponseuser = '0;
  assign mmio64.writeresponsevalid = 1'b0;
  assign mmio64.writeresponse = '0;
  assign mmio64.writeresponseuser = '0;

  logos_d5005_shell #(
    .NB(NB), .AW(AW), .DWM(DWM), .ECC({ecc}), .MBW(MBW), .HAW(HAW), .DWH(DWH), .HBW(HBW),
    .MAW(ofs_plat_host_chan_pkg::MMIO_ADDR_WIDTH_BYTES - 3),
    .MAXB({max_burst}), .RAW({read_fifo_log2}), .WAW({write_fifo_log2}),
    .AFU_ID_L(64'h{id_l}), .AFU_ID_H(64'h{id_h}), .BUILD(64'h{build})
  ) shell (
    .pclk(pclk), .pclk_rst_n(pclk_rst_n), .uclk(uclk), .uclk_rst_n(uclk_rst_n),{uclk2}
    .mmio_address(mmio64.address), .mmio_read(mmio64.read), .mmio_write(mmio64.write),
    .mmio_writedata(mmio64.writedata), .mmio_byteenable(mmio64.byteenable), .mmio_waitrequest(mmio64.waitrequest),
    .mmio_readdata(mmio64.readdata), .mmio_readdatavalid(mmio64.readdatavalid),
    .host_rd_address(host_mem.rd_address), .host_rd_read(host_mem.rd_read), .host_rd_burstcount(host_mem.rd_burstcount),
    .host_rd_waitrequest(host_mem.rd_waitrequest), .host_rd_readdata(host_mem.rd_readdata),
    .host_rd_readdatavalid(host_mem.rd_readdatavalid),
    .host_wr_address(host_mem.wr_address), .host_wr_write(host_mem.wr_write), .host_wr_burstcount(host_mem.wr_burstcount),
    .host_wr_writedata(host_mem.wr_writedata), .host_wr_byteenable(host_mem.wr_byteenable),
    .host_wr_waitrequest(host_mem.wr_waitrequest), .host_wr_writeresponsevalid(host_mem.wr_writeresponsevalid),
    .mem_clk(mem_clk), .mem_rst_n(mem_rst_n), .mem_address(mem_address), .mem_read(mem_read), .mem_write(mem_write),
    .mem_burstcount(mem_burstcount), .mem_writedata(mem_writedata), .mem_byteenable(mem_byteenable),
    .mem_waitrequest(mem_waitrequest), .mem_readdata(mem_readdata), .mem_readdatavalid(mem_readdatavalid));
endmodule
"##;

const DCFIFO: &str = r##"// A dual-clock FIFO of 2^AW words: the only way a value crosses between two of the engine's clocks.
//
// Each side keeps a binary and a Gray pointer in its own clock. A Gray pointer changes one bit a step, so the other
// side samples it through two registers and sees either the old or the new count, never a mix. The RAM is written in
// wclk and read in rclk; a word is read only after its write pointer has crossed, two rclk edges after the write.
// The read side shows ahead: rvalid means rdata holds the oldest word. wfree and rused are each side's conservative
// view: the other side's last pointer to cross. Each side steps at its enable (wce, rce), its synchronizers too, as a
// side clocked that much slower does: its outputs move only at the edges it steps at.
//
// A FIFO says what its writer is: HOLDS, one that holds a refused word until it is taken (checked in simulation);
// TOKENS, one that gives tokens (W is 1) as pulses, counted on the write side until taken; or neither, one that may
// let a refused word fall.
module MODULE_NAME #(parameter W = 64, parameter AW = 4, parameter HOLDS = 0, parameter TOKENS = 0) (
  input               wclk,
  input               wce,
  input               wrst_n,
  input               wvalid,
  output              wready,
  input  [W-1:0]      wdata,
  output [AW:0]       wfree,
  input               rclk,
  input               rce,
  input               rrst_n,
  output              rvalid,
  input               rready,
  output [W-1:0]      rdata,
  output [AW:0]       rused
);
  localparam [AW:0] DEPTH = 1 << AW;

  function [AW:0] gray(input [AW:0] b);
    gray = b ^ (b >> 1);
  endfunction

  function [AW:0] binary(input [AW:0] g);
    integer i;
    begin
      binary[AW] = g[AW];
      for (i = AW - 1; i >= 0; i = i - 1) binary[i] = binary[i + 1] ^ g[i];
    end
  endfunction

  (* ramstyle = "RAMSTYLE, no_rw_check" *) reg [W-1:0] mem [0:DEPTH-1];

  // Write side. Each Gray pointer stays a register of its own, the one its crossing is constrained from (its top bit
  // is the binary pointer's, which synthesis would otherwise merge it with); the other side's pointer, synchronized,
  // is converted to binary in a register, a cycle older than the synchronizer, so the free count only under-reports.
  reg  [AW:0] wbin;
  (* dont_merge *) reg [AW:0] wgray;
  (* altera_attribute = "-name SYNCHRONIZER_IDENTIFICATION \"FORCED IF ASYNCHRONOUS\"" *) reg [AW:0] rgray_s1;
  (* altera_attribute = "-name SYNCHRONIZER_IDENTIFICATION \"FORCED IF ASYNCHRONOUS\"" *) reg [AW:0] rgray_s2;
  (* dont_merge *) reg [AW:0] rgray;
  reg  [AW:0] rbin_w;
  wire        full = (wbin - rbin_w) == DEPTH;
  // With TOKENS a write is a token given as a pulse whatever wready says: the write side counts the tokens it has not
  // taken and takes one a step as it has room, so none is let fall while the FIFO looks full. Otherwise a write is the
  // writer's word, offered until the FIFO takes it.
  wire        offer;
  generate if (TOKENS) begin : tokens
    reg [15:0] owed;
    always @(posedge wclk or negedge wrst_n)
      if (!wrst_n) owed <= 16'd0;
      else if (wce) owed <= owed + {15'd0, wvalid} - {15'd0, owed != 16'd0 && !full};
    assign offer = owed != 16'd0;
  end else begin : words
    assign offer = wvalid;
  end endgenerate
  wire        push = wce && offer && !full;
  assign wready = !full;
  assign wfree = DEPTH - (wbin - rbin_w);
  // With HOLDS the writer holds a word the FIFO refused until it is taken: in simulation, a refused word not offered
  // again at the next step, unchanged, stops the run.
  // synthesis translate_off
`ifndef SYNTHESIS
  reg          sim_refused = 1'b0;
  reg  [W-1:0] sim_word;
  always @(posedge wclk)
    if (!wrst_n) sim_refused <= 1'b0;
    else if (wce) begin
      if (HOLDS != 0 && sim_refused && !(wvalid && wdata === sim_word))
        $fatal(1, "%m: a word the FIFO refused was not offered again");
      sim_refused <= wvalid && full;
      sim_word <= wdata;
    end
`endif
  // synthesis translate_on

  always @(posedge wclk) if (push) mem[wbin[AW-1:0]] <= wdata;

  always @(posedge wclk or negedge wrst_n) begin
    if (!wrst_n) begin
      wbin <= 0;
      wgray <= 0;
      rgray_s1 <= 0;
      rgray_s2 <= 0;
      rbin_w <= 0;
    end else if (wce) begin
      rgray_s1 <= rgray;
      rgray_s2 <= rgray_s1;
      rbin_w <= binary(rgray_s2);
      if (push) begin
        wbin <= wbin + 1'b1;
        wgray <= gray(wbin + 1'b1);
      end
    end
  end

  // Read side.
  reg  [AW:0] rbin;
  (* altera_attribute = "-name SYNCHRONIZER_IDENTIFICATION \"FORCED IF ASYNCHRONOUS\"" *) reg [AW:0] wgray_s1;
  (* altera_attribute = "-name SYNCHRONIZER_IDENTIFICATION \"FORCED IF ASYNCHRONOUS\"" *) reg [AW:0] wgray_s2;
  reg  [W-1:0] rdata_q;
  reg          rvalid_q;
  reg  [AW:0] wbin_r;
  wire        empty = wbin_r == rbin;
  wire        pop = rce && !empty && (!rvalid_q || rready);
  assign rvalid = rvalid_q;
  assign rdata = rdata_q;
  assign rused = (wbin_r - rbin) + {{AW{1'b0}}, rvalid_q};

  always @(posedge rclk) if (pop) rdata_q <= mem[rbin[AW-1:0]];

  always @(posedge rclk or negedge rrst_n) begin
    if (!rrst_n) begin
      rbin <= 0;
      rgray <= 0;
      wgray_s1 <= 0;
      wgray_s2 <= 0;
      wbin_r <= 0;
      rvalid_q <= 1'b0;
    end else if (rce) begin
      wgray_s1 <= wgray;
      wgray_s2 <= wgray_s1;
      wbin_r <= binary(wgray_s2);
      if (pop) begin
        rbin <= rbin + 1'b1;
        rgray <= gray(rbin + 1'b1);
      end
      if (pop) rvalid_q <= 1'b1;
      else if (rready) rvalid_q <= 1'b0;
    end
  end
endmodule
"##;

const STREAM: &str = r##"// The stream engines, each in its channel's own clock: descriptors in, Avalon-MM bursts out, lines through FIFOs.
//
// A descriptor {line address, lines} moves `lines` lines in bursts of at most MAXB. A reader issues a burst only
// when the read-data FIFO has room for it beside every line still owed: readdatavalid cannot be held off, so the
// room is reserved at issue (`r_room` is the FIFO's write-side free count, which only under-reports), one request on
// the bus at a time so each is sized against what the last reserved. A writer takes a burst's beats from the
// write-data FIFO as they come; an Avalon burst may pause between beats.
module logos_stream_reader #(
  parameter AW = 27,           // line address bits
  parameter DW = 512,          // data bits a line
  parameter BW = 7,            // burstcount bits
  parameter LW = 32,           // line-count bits of a descriptor
  parameter MAXB = 64,         // lines a burst, at most 2^(BW-1)
  parameter RW = 10            // read-FIFO count bits
) (
  input               clk,
  input               rst_n,
  input               d_valid,
  output              d_ready,
  input  [AW+LW-1:0]  d_data,
  output              r_valid,
  output [DW-1:0]     r_data,
  input  [RW-1:0]     r_room,
  output reg [AW-1:0] avm_address,
  output reg          avm_read,
  output reg [BW-1:0] avm_burstcount,
  input               avm_waitrequest,
  input  [DW-1:0]     avm_readdata,
  input               avm_readdatavalid,
  output reg [RW-1:0] owed,
  output              active
);
  localparam [LW-1:0] MAXL = MAXB;
  reg          busy_q;
  reg [AW-1:0] next_addr;
  reg [LW-1:0] left;
  // The next burst and the room beside every line owed, each a register a cycle behind left and owed; a burst issues
  // only while both are current, a cycle after anything changed them. The room only grows meanwhile (the FIFO's other
  // side frees it; a line returning leaves the room and the lines owed together), so it never over-reports.
  reg [LW-1:0] chunk;
  reg [RW:0]   spare;
  reg          current;
  wire [AW+LW-1:0] chunk_wide = {{AW{1'b0}}, chunk};
  wire [AW-1:0] step = chunk_wide[AW-1:0];
  // A burst is at most MAXB lines, fewer than the read FIFO holds, so the room and the burst compare at its width.
  wire          fits = spare >= {1'b0, chunk[RW-1:0]};
  wire          accepted = avm_read && !avm_waitrequest;
  wire [RW-1:0] burst_lines = avm_burstcount;
  wire [RW-1:0] returned = avm_readdatavalid;
  assign d_ready = !busy_q;
  assign active = busy_q;
  assign r_valid = avm_readdatavalid;
  assign r_data = avm_readdata;
  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      busy_q <= 1'b0;
      next_addr <= 0;
      left <= 0;
      avm_address <= 0;
      avm_read <= 1'b0;
      avm_burstcount <= 0;
      owed <= 0;
      chunk <= 0;
      spare <= 0;
      current <= 1'b0;
    end else begin
      owed <= owed + (accepted ? burst_lines : {RW{1'b0}}) - returned;
      chunk <= (left > MAXL) ? MAXL : left;
      spare <= {1'b0, r_room} - {1'b0, owed};
      current <= 1'b1;
      if (!busy_q) begin
        if (d_valid) begin
          busy_q <= 1'b1;
          next_addr <= d_data[AW+LW-1:LW];
          left <= d_data[LW-1:0];
          current <= 1'b0;
        end
      end else if (avm_read) begin
        if (!avm_waitrequest) begin avm_read <= 1'b0; current <= 1'b0; end
      end else if (left == 0) begin
        busy_q <= 1'b0;
      end else if (current && fits) begin
        current <= 1'b0;
        avm_read <= 1'b1;
        avm_address <= next_addr;
        avm_burstcount <= chunk[BW-1:0];
        next_addr <= next_addr + step;
        left <= left - chunk;
      end
    end
  end
endmodule

module logos_stream_writer #(
  parameter AW = 27,
  parameter DW = 512,
  parameter BW = 7,
  parameter LW = 32,
  parameter MAXB = 64
) (
  input               clk,
  input               rst_n,
  input               d_valid,
  output              d_ready,
  input  [AW+LW-1:0]  d_data,
  input               w_valid,
  output              w_ready,
  input  [DW-1:0]     w_data,
  output reg [AW-1:0] avm_address,
  output              avm_write,
  output reg [BW-1:0] avm_burstcount,
  output [DW-1:0]     avm_writedata,
  output [DW/8-1:0]   avm_byteenable,
  input               avm_waitrequest,
  output              active,
  output reg          done
);
  localparam [LW-1:0] MAXL = MAXB;
  reg          busy_q, in_burst;
  reg [AW-1:0] next_addr;
  reg [LW-1:0] left;
  reg [BW-1:0] beats;
  wire [LW-1:0] chunk = (left > MAXL) ? MAXL : left;
  wire [AW+LW-1:0] chunk_wide = {{AW{1'b0}}, chunk};
  wire [AW-1:0] step = chunk_wide[AW-1:0];
  wire          accepted = avm_write && !avm_waitrequest;
  assign d_ready = !busy_q;
  assign active = busy_q;
  assign avm_write = in_burst && w_valid;
  assign w_ready = accepted;
  assign avm_writedata = w_data;
  assign avm_byteenable = {(DW/8){1'b1}};
  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      busy_q <= 1'b0;
      in_burst <= 1'b0;
      next_addr <= 0;
      left <= 0;
      beats <= 0;
      avm_address <= 0;
      avm_burstcount <= 0;
      done <= 1'b0;
    end else begin
      done <= 1'b0;
      if (!busy_q) begin
        if (d_valid) begin
          busy_q <= 1'b1;
          next_addr <= d_data[AW+LW-1:LW];
          left <= d_data[LW-1:0];
        end
      end else if (in_burst) begin
        if (accepted) begin
          beats <= beats - 1'b1;
          if (beats == 1) in_burst <= 1'b0;
        end
      end else if (left == 0) begin
        busy_q <= 1'b0;
        done <= 1'b1;
      end else begin
        in_burst <= 1'b1;
        avm_address <= next_addr;
        avm_burstcount <= chunk[BW-1:0];
        beats <= chunk[BW-1:0];
        next_addr <= next_addr + step;
        left <= left - chunk;
      end
    end
  end
endmodule

// One local-memory channel: a reader and a writer on the bank's single Avalon-MM port. A descriptor {write, address,
// lines} goes to the engine its op names once neither engine is issuing, so one engine drives the bus at a time;
// lines a read still owes keep returning while a write issues.
//
// With ECC set every line is SECDED(72,64)-coded on the bus (logos_secded.v): a line written is coded, then has the
// bus bits `w_flip` = {enable, b, a} names flipped (one bit when a = b); a line read is decoded in one registered
// stage, its words' verdicts `r_ecc` = {uncorrectable, corrected} beside it, and the line in the stage counted
// against the read FIFO's room as the lines the reader still owes are.
module logos_stream_avmm #(
  parameter AW = 27,
  parameter DW = 512,          // bus bits a line
  parameter BW = 7,
  parameter LW = 32,
  parameter MAXB = 64,
  parameter RW = 10,
  parameter ECC = 0,
  parameter DL = ECC ? DW / 9 * 8 : DW   // data bits a line
) (
  input               clk,
  input               rst_n,
  input               d_valid,
  output              d_ready,
  input  [AW+LW:0]    d_data,
  output              r_valid,
  output [DL-1:0]     r_data,
  output [15:0]       r_ecc,
  input  [RW-1:0]     r_room,
  input               w_valid,
  output              w_ready,
  input  [DL-1:0]     w_data,
  input  [20:0]       w_flip,
  output [AW-1:0]     avm_address,
  output              avm_read,
  output              avm_write,
  output [BW-1:0]     avm_burstcount,
  output [DW-1:0]     avm_writedata,
  output [DW/8-1:0]   avm_byteenable,
  input               avm_waitrequest,
  input  [DW-1:0]     avm_readdata,
  input               avm_readdatavalid,
  output [RW-1:0]     owed,
  output              busy,
  output              wdone
);
  wire          is_write = d_data[AW+LW];
  wire          rd_active, wr_active, rd_ready, wr_ready;
  wire          idle = !rd_active && !wr_active;
  wire [AW-1:0] rd_address, wr_address;
  wire [BW-1:0] rd_burstcount, wr_burstcount;
  wire          bus_valid;
  wire [DW-1:0] bus_rdata, bus_wdata;
  wire [RW-1:0] room;
  wire          staged;
  // The bus's read data in a register on its way in, the platform's bridge register to this one. The reader counts a
  // line returned when it leaves this register, so a line is owed until it reaches the read FIFO, or the decoding
  // stage on SECDED lines, which the room counts.
  reg           in_valid;
  reg  [DW-1:0] in_rdata;
  always @(posedge clk) in_rdata <= avm_readdata;
  always @(posedge clk or negedge rst_n)
    if (!rst_n) in_valid <= 1'b0;
    else in_valid <= avm_readdatavalid;
  assign d_ready = idle;
  // The engines' requests in a two-deep queue on their way out to the platform's bridge: the engines see room while the
  // queue holds at most one, its count a register, and the bus takes from its head, so a request reaches the bus in
  // the order the engines made it, a read after a write behind it, and nothing in the engines waits on the bus. The
  // head's read and write are registers of their own, cleared as the head leaves with nothing behind it, so what the
  // bridge registers is a register.
  localparam QW = AW + BW + DW + DW / 8;
  wire          q_read, q_write;
  wire [AW-1:0] q_address = wr_active ? wr_address : rd_address;
  wire [BW-1:0] q_burstcount = wr_active ? wr_burstcount : rd_burstcount;
  wire [DW-1:0] q_writedata;
  wire [DW/8-1:0] q_byteenable;
  reg  [1:0]    q_count;
  reg           head_read, head_write, next_read, next_write;
  reg  [QW-1:0] q_head, q_next;
  wire          q_room = q_count != 2'd2;
  wire          q_in = (q_read || q_write) && q_room;
  wire          q_out = (head_read || head_write) && !avm_waitrequest;
  wire [QW-1:0] q_entry = {q_address, q_burstcount, q_writedata, q_byteenable};
  assign busy = !idle || owed != 0 || q_count != 2'd0 || staged;
  always @(posedge clk or negedge rst_n)
    if (!rst_n) begin
      q_count <= 2'd0;
      {head_read, head_write, next_read, next_write} <= 4'b0000;
    end else begin
      q_count <= q_count + {1'b0, q_in} - {1'b0, q_out};
      if (q_out) begin
        if (q_count == 2'd2) {head_read, head_write} <= {next_read, next_write};
        else if (q_in) {head_read, head_write} <= {q_read, q_write};
        else {head_read, head_write} <= 2'b00;
      end else if (q_in) begin
        if (q_count == 2'd0) {head_read, head_write} <= {q_read, q_write};
        else {next_read, next_write} <= {q_read, q_write};
      end
    end
  always @(posedge clk)
    if (q_out) begin
      if (q_count == 2'd2) q_head <= q_next;
      else if (q_in) q_head <= q_entry;
    end else if (q_in) begin
      if (q_count == 2'd0) q_head <= q_entry;
      else q_next <= q_entry;
    end
  assign avm_read = head_read;
  assign avm_write = head_write;
  assign {avm_address, avm_burstcount, avm_writedata, avm_byteenable} = q_head;
  logos_stream_reader #(.AW(AW), .DW(DW), .BW(BW), .LW(LW), .MAXB(MAXB), .RW(RW)) reader(
    .clk(clk), .rst_n(rst_n), .d_valid(d_valid && idle && !is_write), .d_ready(rd_ready), .d_data(d_data[AW+LW-1:0]),
    .r_valid(bus_valid), .r_data(bus_rdata), .r_room(room), .avm_address(rd_address), .avm_read(q_read),
    .avm_burstcount(rd_burstcount), .avm_waitrequest(!q_room), .avm_readdata(in_rdata),
    .avm_readdatavalid(in_valid), .owed(owed), .active(rd_active));
  logos_stream_writer #(.AW(AW), .DW(DW), .BW(BW), .LW(LW), .MAXB(MAXB)) writer(
    .clk(clk), .rst_n(rst_n), .d_valid(d_valid && idle && is_write), .d_ready(wr_ready), .d_data(d_data[AW+LW-1:0]),
    .w_valid(w_valid), .w_ready(w_ready), .w_data(bus_wdata), .avm_address(wr_address), .avm_write(q_write),
    .avm_burstcount(wr_burstcount), .avm_writedata(q_writedata), .avm_byteenable(q_byteenable),
    .avm_waitrequest(!q_room), .active(wr_active), .done(wdone));
  generate if (ECC) begin : secded
    wire [DW-1:0] one = {{(DW-1){1'b0}}, 1'b1};
    wire [DW-1:0] coded;
    wire [DL-1:0] fixed;
    wire [7:0]    corrected, uncorrectable;
    reg           q_valid;
    reg  [DL-1:0] q_data;
    reg  [15:0]   q_ecc;
    logos_secded_encode coder(.d(w_data), .bus(coded));
    assign bus_wdata = coded ^ (w_flip[20] ? (one << w_flip[9:0]) | (one << w_flip[19:10]) : {DW{1'b0}});
    logos_secded_decode decoder(.bus(bus_rdata), .d(fixed), .corrected(corrected), .uncorrectable(uncorrectable));
    always @(posedge clk or negedge rst_n) begin
      if (!rst_n) begin
        q_valid <= 1'b0;
        q_data <= 0;
        q_ecc <= 0;
      end else begin
        q_valid <= bus_valid;
        q_data <= fixed;
        q_ecc <= {uncorrectable, corrected};
      end
    end
    assign r_valid = q_valid;
    assign r_data = q_data;
    assign r_ecc = q_ecc;
    assign room = r_room - {{(RW-1){1'b0}}, q_valid};
    assign staged = q_valid;
  end else begin : plain
    assign bus_wdata = w_data;
    assign r_valid = bus_valid;
    assign r_data = bus_rdata;
    assign r_ecc = 16'h0;
    assign room = r_room;
    assign staged = 1'b0;
  end endgenerate
endmodule
"##;

const CSR: &str = r##"// The accelerator's registers, in the host channel's clock: the OFS feature header and ID the driver finds it by,
// a command doorbell into the engine, and the engine's last status record.
//
// MMIO addresses are 64-bit word indices. A write to DOORBELL pushes {arg2, arg1, arg0, doorbell} into the command
// FIFO in that cycle; with the FIFO full it is dropped, counted in DROPPED and ERRORS bit 0 set, so a driver reads
// CMD_SPACE first. Every
// status record the engine pushes is taken at once and mirrored. Reads answer the next cycle, in order.
module logos_csr #(
  parameter MAW = 15,                     // MMIO word-address bits
  parameter [63:0] AFU_ID_L = 64'h0,
  parameter [63:0] AFU_ID_H = 64'h0,
  parameter [3:0]  AFU_MAJOR = 4'd1,
  parameter [3:0]  AFU_MINOR = 4'd0,
  parameter [63:0] BUILD = 64'h0,
  parameter CW = 8                        // command-FIFO count bits
) (
  input             clk,
  input             rst_n,
  input  [MAW-1:0]  mmio_address,
  input             mmio_read,
  input             mmio_write,
  input  [63:0]     mmio_writedata,
  input  [7:0]      mmio_byteenable,
  output            mmio_waitrequest,
  output reg [63:0] mmio_readdata,
  output reg        mmio_readdatavalid,
  // Commands, into the engine's clock.
  output            cmd_valid,
  input             cmd_ready,
  output [255:0]    cmd_data,
  input  [CW-1:0]   cmd_space,
  // Status records {seq, status, result0, result1}, from the engine's clock.
  input             st_valid,
  output            st_ready,
  input  [255:0]    st_data
);
  /*REGISTER_MAP*/

  reg [63:0] scratch, arg0, arg1, arg2, status, result0, result1, status_seq, errors, dropped;
  wire       ring = mmio_write && mmio_address == DOORBELL;

  assign mmio_waitrequest = 1'b0;
  assign cmd_valid = ring;
  assign cmd_data = {arg2, arg1, arg0, mmio_writedata};
  assign st_ready = 1'b1;

  wire [63:0] dfh = {4'h1, 8'h0, AFU_MINOR, 7'h0, 1'b1, 24'h0, AFU_MAJOR, 12'h0};

  function [63:0] merge(input [63:0] old, input [63:0] new_, input [7:0] be);
    integer i;
    for (i = 0; i < 8; i = i + 1) merge[8*i +: 8] = be[i] ? new_[8*i +: 8] : old[8*i +: 8];
  endfunction

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      scratch <= 0; arg0 <= 0; arg1 <= 0; arg2 <= 0; dropped <= 0;
      status <= 0; result0 <= 0; result1 <= 0; status_seq <= 0; errors <= 0;
      mmio_readdata <= 0; mmio_readdatavalid <= 1'b0;
    end else begin
      if (ring && !cmd_ready) begin
        errors[0] <= 1'b1;
        dropped <= dropped + 1'b1;
      end
      if (st_valid) begin
        status_seq <= st_data[63:0];
        status <= st_data[127:64];
        result0 <= st_data[191:128];
        result1 <= st_data[255:192];
      end
      if (mmio_write) begin
        case (mmio_address)
          SCRATCH: scratch <= merge(scratch, mmio_writedata, mmio_byteenable);
          ARG0: arg0 <= merge(arg0, mmio_writedata, mmio_byteenable);
          ARG1: arg1 <= merge(arg1, mmio_writedata, mmio_byteenable);
          ARG2: arg2 <= merge(arg2, mmio_writedata, mmio_byteenable);
          ERRORS: errors <= errors & ~mmio_writedata;
          default: ;
        endcase
      end
      mmio_readdatavalid <= mmio_read;
      if (mmio_read) begin
        case (mmio_address)
          DFH: mmio_readdata <= dfh;
          ID_L: mmio_readdata <= AFU_ID_L;
          ID_H: mmio_readdata <= AFU_ID_H;
          SCRATCH: mmio_readdata <= scratch;
          BUILD_ID: mmio_readdata <= BUILD;
          STATUS: mmio_readdata <= status;
          RESULT0: mmio_readdata <= result0;
          RESULT1: mmio_readdata <= result1;
          STATUS_SEQ: mmio_readdata <= status_seq;
          CMD_SPACE: mmio_readdata <= {{(64-CW){1'b0}}, cmd_space};
          ERRORS: mmio_readdata <= errors;
          DROPPED: mmio_readdata <= dropped;
          ARG0: mmio_readdata <= arg0;
          ARG1: mmio_readdata <= arg1;
          ARG2: mmio_readdata <= arg2;
          default: mmio_readdata <= 64'h0;
        endcase
      end
    end
  end
endmodule
"##;

const CORE: &str = r##"// The bring-up engine, in the compute clock — every register stepping in its clock at ce: fills and checks the memory
// channels at full rate and copies lines between host memory and a channel, so the shell's every path is exercised on
// the card before the decode engine takes its place behind the same ports.
//
// A command is {arg2, arg1, arg0, {tag, op}}; each completes with a status record {seq, {tag, code}, result0,
// result1}: seq the count of commands completed with this one, result1 the compute cycles it took. While a command
// runs, every 2^HB cycles in which it moved since the last, the core cuts a progress record {seq, {tag, RUNNING},
// {pc, progress}, cycles so far}: seq the commands completed before it, so a host waiting for the command's own record
// still waits, pc the engine's word (each context's in its 16 bits), and progress the low 32 bits of the count of
// cycles since reset in which a line crossed a memory channel (on whichever side holds the channels) or the host
// channel or the engine's sequencer took its next word.
//   FILL  (1): every channel in mask arg0 writes `lines` = arg2[31:0] lines from line arg1, line L = pattern(seed, L)
//              with seed = arg2[63:32]; done when each channel reports its last beat accepted.
//   CHECK (2): every channel in the mask reads those lines back; result0 = lines that differ.
//   HOST_TO_MEM (3): `lines` host lines from host line arg0 into channel arg1[63:56] at line arg1[AW-1:0]; a line is
//              the host line, zero-extended.
//   MEM_TO_HOST (4): the reverse, a channel line truncated to a host line; done when every host write burst is
//              acknowledged.
//   HOST_TO_MEM_PACKED (5): `lines` channel lines at the channel's full width into channel arg1[63:56] at line
//              arg1[AW-1:0], read packed end to end from host line arg0 on, the fewest host lines that hold them; done
//              when the channel's writes are done.
//   MEM_TO_HOST_PACKED (6): the reverse, the channel lines packed end to end into host lines, the last zero past
//              them; done when every host write burst is acknowledged.
//   A packed copy moves its bytes through one gather register of 64-bit words: a line leaves it once it holds a
//   line's words, a line comes into it when its words fit.
//   INJECT (7): on SECDED channels, the next arg2[31:0] lines written to each channel in mask arg0 have bus bits
//              arg1[9:0] and arg1[25:16] flipped after they are coded, one bit when the two are equal.
//   ECC_COUNTS (8): on SECDED channels, result0 = {words found uncorrectable, words corrected} of channel arg0[7:0]
//              since reset, each modulo 2^32, every line read counted, whichever side took it.
//   ENGINE_CONFIG (9): with the engine, its token-rows arg0[15:0] and knobs arg0[19:16] {per_block, two_words,
//              cache_int8, state_f16}, the card port's space arg1[31:0], base arg2[AW-1:0] and log2 granule
//              arg2[60:56], held until the next.
//   ENGINE_TABLE (10): with the engine, `lines` host lines from host line arg0 into its table arg1[34:32] from entry
//              arg1[31:0] on, a line an entry; done when the last is written.
//   ENGINE_RUN (11): with the engine, the step from job word arg0[31:0] to arg1[31:0]: the card port's fault cleared,
//              the engine started, the channels the card port's until the engine is idle again; result0 = {the word
//              the sequencer reached, 30'b0, the card port's fault, the engine's fault}.
//   STORE_READ (12): with the engine, `lines` beats of its store from beat arg1[31:0] into host lines from host line
//              arg0, each beat read a cycle after its address is set; done when every host write burst is acknowledged.
//   Any other op completes at once with code 0x80, as INJECT and ECC_COUNTS do on channels without SECDED and the
//   engine's four without the engine; NOP (0) with code 0.
module logos_bringup_core #(
  parameter NB = 4,            // memory channels
  parameter AW = 27,           // channel line-address bits
  parameter DWM = 512,         // channel line bits
  parameter ECC = 0,           // the channels' lines SECDED-coded
  parameter ENGINE = 0,        // an engine and its card port behind the core
  parameter HAW = 42,          // host line-address bits
  parameter DWH = 512,         // host line bits
  parameter LW = 32,
  parameter HMAXB = 64,        // lines a host write burst
  parameter HB = 16            // log2 of the cycles between a running command's progress records
) (
  input                     clk,
  input                     ce,
  input                     rst_n,
  input                     cmd_valid,
  output                    cmd_ready,
  input  [255:0]            cmd_data,
  output                    st_valid,
  input                     st_ready,
  output [255:0]            st_data,
  // Channel descriptors {write, address, lines}, lines read, lines to write, write descriptors finished.
  output [NB-1:0]           md_valid,
  input  [NB-1:0]           md_ready,
  output [NB*(AW+LW+1)-1:0] md_data,
  input  [NB-1:0]           mr_valid,
  output [NB-1:0]           mr_ready,
  input  [NB*DWM-1:0]       mr_data,
  input  [NB*16-1:0]        mr_ecc,      // each line read's words' verdicts {uncorrectable, corrected}
  output [NB-1:0]           mw_valid,
  input  [NB-1:0]           mw_ready,
  output [NB*DWM-1:0]       mw_data,
  output [NB*21-1:0]        mw_flip,     // the bus bits each line written flips {enable, b, a}
  input  [NB-1:0]           mdone_valid,
  output [NB-1:0]           mdone_ready,
  // Host read and write descriptors {address, lines}, host lines read, host lines to write, host write acknowledgements.
  output                    hrd_valid,
  input                     hrd_ready,
  output [HAW+LW-1:0]       hrd_data,
  input                     hr_valid,
  output                    hr_ready,
  input  [DWH-1:0]          hr_data,
  output                    hwd_valid,
  input                     hwd_ready,
  output [HAW+LW-1:0]       hwd_data,
  output                    hw_valid,
  input                     hw_ready,
  output [DWH-1:0]          hw_data,
  input                     hack_valid,
  output                    hack_ready,
  // The lines each read FIFO gives up, to whichever side takes them; a line crossing any channel, either way.
  input  [NB-1:0]           mr_taken,
  input                     moved,
  // The engine: its run, its configuration, its tables and store; the card port's map, its fault, and the channels.
  output reg                card_mode,
  output reg                eng_start,
  output reg [31:0]         eng_job_at,
  output reg [31:0]         eng_job_end,
  output reg [15:0]         eng_rows,
  output reg [3:0]          eng_knobs,
  input                     eng_busy,
  input                     eng_fault,
  input  [31:0]             eng_pc,
  output                    eng_host_wr,
  output reg [2:0]          eng_host_table,
  output [31:0]             eng_host_addr,
  output [DWH-1:0]          eng_host_wdata,
  output [31:0]             eng_host_raddr,
  input  [DWH-1:0]          eng_host_rdata,
  output reg [31:0]         port_space,
  output reg [AW-1:0]       port_base,
  output reg [4:0]          port_granule_log2,
  output reg                port_clear,
  input                     port_fault
);
  /*OPS*/
  localparam LANES = DWM / 64;
  localparam BB = NB > 1 ? $clog2(NB) : 1;
  localparam [LW-1:0] HMAXL = HMAXB;
  localparam HMAXB_LOG2 = $clog2(HMAXB);

  // Line L of a fill seeded `seed`: each 64-bit lane distinct, so a swapped lane or line reads back wrong.
  function [DWM-1:0] pattern(input [31:0] seed, input [31:0] line);
    integer k;
    reg [31:0] lane;
    begin
      for (k = 0; k < LANES; k = k + 1) begin
        lane = k;
        pattern[64*k +: 64] = {seed ^ line ^ (lane * 32'h9e3779b9), ~line + lane * 32'h85ebca6b};
      end
    end
  endfunction

  // The 32 low bits of channel line `base + i`, the line a pattern is seeded by.
  function [31:0] line_of(input [AW-1:0] base_line, input [LW-1:0] i);
    reg [AW-1:0] at;
    begin
      at = base_line + i[AW-1:0];
      line_of = at;
    end
  endfunction

  reg              busy, reported;
  reg [7:0]        op;
  reg [55:0]       tag;
  reg [7:0]        code;
  reg [63:0]       seq, cycles, started, result0;
  reg [NB-1:0]     mask, need_desc, wrote, done;
  reg [AW-1:0]     base;
  reg [LW-1:0]     lines;
  reg [31:0]       seed;
  reg [HAW-1:0]    hbase;
  reg [7:0]        bank;
  reg [LW-1:0]     sent [0:NB-1];
  reg [LW-1:0]     seen [0:NB-1];
  reg [63:0]       wrong [0:NB-1];
  reg              need_hdesc;
  reg [LW-1:0]     acks, bursts;
  // SECDED: lines each channel still flips and the two bus bits, the words corrected and lost since reset.
  reg [31:0]       flips_left [0:NB-1];
  reg [19:0]       flip_bits [0:NB-1];
  reg [31:0]       fixed [0:NB-1];
  reg [31:0]       lost [0:NB-1];

  function [3:0] ones(input [7:0] v);
    integer k;
    begin
      ones = 0;
      for (k = 0; k < 8; k = k + 1) ones = ones + v[k];
    end
  endfunction

  wire packed_in = op == HOST_TO_MEM_PACKED;
  wire packed_out = op == MEM_TO_HOST_PACKED;
  wire eng_table = op == ENGINE_TABLE;
  wire store_read = op == STORE_READ;
  // The engine's tables and store: the entry a table write starts at and the host lines it has taken; the beat a store
  // read starts at, the beats it has sent, and whether the current beat's address has stood a cycle.
  reg [31:0]       tbase, taken, sbase, sent_beats;
  reg              beat_ready;
  reg [1:0]        run_hold;
  // A table write's end, from registers: every line taken, or every line but the one taken now.
  wire taken_all = taken == lines, taken_last = taken + 32'd1 == lines;
  wire to_mem = op == FILL || op == HOST_TO_MEM || packed_in;
  wire from_host = op == HOST_TO_MEM || packed_in;
  wire to_host = op == MEM_TO_HOST || packed_out;

  // The packed copies' gather register: `gn` 64-bit words of the bytes in flight, the rest zero.
  localparam LWD = DWM / 64, HWD = DWH / 64, GW = LWD + HWD;
  localparam [7:0] LWD8 = LWD, HWD8 = HWD, GW8 = GW;
  reg [64*GW-1:0] g;
  reg [7:0]       gn;
  reg [LW-1:0]    hlines;
  // The host lines a command moves: a packed copy's channel lines end to end in host lines, any other one each.
  wire [LW-1:0]   cmd_lines = cmd_data[192+LW-1:192];
  wire [LW+7:0]   cmd_words = cmd_lines * LWD8;
  wire [LW+7:0]   cmd_hwords = (cmd_words + HWD8 - 8'd1) / HWD8;
  wire [LW-1:0]   cmd_hlines = (cmd_data[7:0] == HOST_TO_MEM_PACKED || cmd_data[7:0] == MEM_TO_HOST_PACKED)
                               ? cmd_hwords[LW-1:0] : cmd_lines;

  // Progress: cycles in which a line or the sequencer moved, each counted the step after (`moved_q`), so the count's
  // carry chain never waits on the channels' handshakes; `told` the count the last progress record carried.
  reg [63:0]       progress, told;
  reg              moved_q;
  reg [31:0]       last_pc;
  reg [HB-1:0]     beat_timer;
  reg              beat;
  wire host_moved = (hr_valid && hr_ready) || (hw_valid && hw_ready);
  assign cmd_ready = !busy;
  assign st_valid = busy && (reported || beat);
  assign st_data = reported ? {cycles - started, result0, {tag, code}, seq + 1'b1}
                            : {cycles - started, eng_pc, progress[31:0], {tag, RUNNING}, seq};

  wire [BB-1:0]  ch_sel = bank[BB-1:0];
  wire [DWM-1:0] host_src = mr_data[ch_sel*DWM +: DWM];
  // Into a channel: a line leaves the register once it holds one, a host line comes in when its words fit beside what
  // stays. The host lines asked for are the fewest that hold the channel lines, so the last channel line takes the last
  // of them.
  wire       pin_emit = busy && packed_in && mw_valid[ch_sel] && mw_ready[ch_sel];
  wire [7:0] pin_gn1 = pin_emit ? gn - LWD8 : gn;
  wire       pin_take = busy && packed_in && hr_valid && pin_gn1 + HWD8 <= GW8;
  // Out of a channel: a host line leaves once the register holds one, or holds the last channel line's last words;
  // a channel line comes in when its words fit beside what stays.
  wire       pout_valid = busy && packed_out && (gn >= HWD8 || (seen[ch_sel] == lines && gn != 0));
  wire       pout_emit = pout_valid && hw_ready;
  wire [7:0] pout_gn1 = pout_emit ? (gn > HWD8 ? gn - HWD8 : 8'd0) : gn;
  wire       pout_take = busy && packed_out && mr_valid[ch_sel] && seen[ch_sel] < lines && pout_gn1 + LWD8 <= GW8;
  genvar b;
  generate for (b = 0; b < NB; b = b + 1) begin : ch
    wire use_this = busy && mask[b];
    wire mem_side = (op == HOST_TO_MEM || op == MEM_TO_HOST || packed_in || packed_out) ? bank == b : 1'b1;
    assign md_valid[b] = use_this && need_desc[b];
    assign md_data[b*(AW+LW+1) +: AW+LW+1] = {to_mem, base, lines};
    assign mw_valid[b] = use_this && to_mem && sent[b] < lines &&
                         (packed_in ? (mem_side && gn >= LWD8) : from_host ? (mem_side && hr_valid) : 1'b1);
    assign mw_data[b*DWM +: DWM] = packed_in ? g[DWM-1:0] : from_host ? {{(DWM-DWH){1'b0}}, hr_data}
                                                                     : pattern(seed, line_of(base, sent[b]));
    assign mw_flip[b*21 +: 21] = {ECC != 0 && flips_left[b] != 0, flip_bits[b]};
    assign mr_ready[b] = use_this && !to_mem && (packed_out ? (mem_side && pout_take) : to_host ? (mem_side && hw_ready) : 1'b1);
    assign mdone_ready[b] = 1'b1;
  end endgenerate

  assign hr_ready = packed_in ? pin_take : eng_table ? busy && taken < lines
                                                     : busy && from_host && mw_ready[ch_sel] && sent[ch_sel] < lines;
  assign hw_valid = packed_out ? pout_valid : store_read ? busy && sent_beats < lines && beat_ready
                                                         : busy && to_host && mr_valid[ch_sel];
  assign hw_data = packed_out ? g[DWH-1:0] : store_read ? eng_host_rdata : host_src[DWH-1:0];
  assign hrd_valid = busy && (from_host || eng_table) && need_hdesc;
  assign hrd_data = {hbase, hlines};
  assign hwd_valid = busy && (to_host || store_read) && need_hdesc;
  assign hwd_data = {hbase, hlines};
  assign hack_ready = 1'b1;
  assign eng_host_wr = busy && eng_table && hr_valid && hr_ready;
  assign eng_host_addr = tbase + taken;
  assign eng_host_wdata = hr_data;
  assign eng_host_raddr = sbase + sent_beats;

  integer i;
  reg [63:0] total;
  always @(*) begin
    total = 0;
    for (i = 0; i < NB; i = i + 1) total = total + wrong[i];
  end

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      busy <= 1'b0; reported <= 1'b0; op <= 0; tag <= 0; code <= 0; seq <= 0; cycles <= 0; started <= 0;
      result0 <= 0; mask <= 0; need_desc <= 0; wrote <= 0; done <= 0; base <= 0; lines <= 0; seed <= 0;
      hbase <= 0; bank <= 0; need_hdesc <= 1'b0; acks <= 0; bursts <= 0; g <= 0; gn <= 0; hlines <= 0;
      for (i = 0; i < NB; i = i + 1) begin
        sent[i] <= 0; seen[i] <= 0; wrong[i] <= 0; flips_left[i] <= 0; flip_bits[i] <= 0; fixed[i] <= 0; lost[i] <= 0;
      end
      card_mode <= 1'b0; eng_start <= 1'b0; eng_job_at <= 0; eng_job_end <= 0; eng_rows <= 0; eng_knobs <= 0;
      eng_host_table <= 0; port_space <= 0; port_base <= 0; port_granule_log2 <= 0; port_clear <= 1'b0;
      tbase <= 0; taken <= 0; sbase <= 0; sent_beats <= 0; beat_ready <= 1'b0; run_hold <= 0;
      progress <= 0; told <= 0; last_pc <= 0; beat_timer <= 0; beat <= 1'b0; moved_q <= 1'b0;
    end else if (ce) begin
      cycles <= cycles + 1'b1;
      moved_q <= moved || host_moved || eng_pc != last_pc;
      progress <= progress + {63'd0, moved_q};
      last_pc <= eng_pc;
      eng_start <= 1'b0;
      port_clear <= 1'b0;
      if (!busy) begin
        if (cmd_valid) begin
          busy <= 1'b1;
          reported <= 1'b0;
          beat <= 1'b0;
          beat_timer <= 0;
          told <= progress;
          op <= cmd_data[7:0];
          tag <= cmd_data[63:8];
          started <= cycles;
          code <= 8'd0;
          result0 <= 0;
          seed <= cmd_data[255:224];
          lines <= cmd_data[192+LW-1:192];
          need_hdesc <= cmd_data[7:0] == HOST_TO_MEM || cmd_data[7:0] == MEM_TO_HOST ||
                        cmd_data[7:0] == HOST_TO_MEM_PACKED || cmd_data[7:0] == MEM_TO_HOST_PACKED ||
                        (ENGINE != 0 && (cmd_data[7:0] == ENGINE_TABLE || cmd_data[7:0] == STORE_READ));
          hbase <= cmd_data[64+HAW-1:64];
          taken <= 0;
          sent_beats <= 0;
          beat_ready <= 1'b0;
          acks <= 0;
          hlines <= cmd_hlines;
          bursts <= (cmd_hlines + HMAXL - 1'b1) >> HMAXB_LOG2;
          g <= 0;
          gn <= 0;
          for (i = 0; i < NB; i = i + 1) begin sent[i] <= 0; seen[i] <= 0; wrong[i] <= 0; end
          done <= 0;
          case (cmd_data[7:0])
            FILL, CHECK: begin
              mask <= cmd_data[64+NB-1:64];
              need_desc <= cmd_data[64+NB-1:64];
              base <= cmd_data[128+AW-1:128];
            end
            HOST_TO_MEM, MEM_TO_HOST, HOST_TO_MEM_PACKED, MEM_TO_HOST_PACKED: begin
              hbase <= cmd_data[64+HAW-1:64];
              bank <= cmd_data[191:184];
              mask <= {{(NB-1){1'b0}}, 1'b1} << cmd_data[184+BB-1:184];
              need_desc <= {{(NB-1){1'b0}}, 1'b1} << cmd_data[184+BB-1:184];
              base <= cmd_data[128+AW-1:128];
            end
            NOP: begin mask <= 0; need_desc <= 0; reported <= 1'b1; end
            INJECT: begin
              mask <= 0; need_desc <= 0; reported <= 1'b1;
              if (ECC == 0) code <= UNKNOWN_OP;
              else for (i = 0; i < NB; i = i + 1) if (cmd_data[64 + i]) begin
                flips_left[i] <= cmd_data[192 +: 32];
                flip_bits[i] <= {cmd_data[144 +: 10], cmd_data[128 +: 10]};
              end
            end
            ECC_COUNTS: begin
              mask <= 0; need_desc <= 0; reported <= 1'b1;
              if (ECC == 0) code <= UNKNOWN_OP;
              else result0 <= {lost[cmd_data[64 +: BB]], fixed[cmd_data[64 +: BB]]};
            end
            ENGINE_CONFIG, ENGINE_TABLE, ENGINE_RUN, STORE_READ: begin
              mask <= 0; need_desc <= 0;
              if (ENGINE == 0) begin
                code <= UNKNOWN_OP; reported <= 1'b1;
              end else case (cmd_data[7:0])
                ENGINE_CONFIG: begin
                  eng_rows <= cmd_data[64 +: 16];
                  eng_knobs <= cmd_data[80 +: 4];
                  port_space <= cmd_data[128 +: 32];
                  port_base <= cmd_data[192 +: AW];
                  port_granule_log2 <= cmd_data[248 +: 5];
                  reported <= 1'b1;
                end
                ENGINE_TABLE: begin
                  tbase <= cmd_data[128 +: 32];
                  eng_host_table <= cmd_data[160 +: 3];
                end
                ENGINE_RUN: begin
                  eng_job_at <= cmd_data[64 +: 32];
                  eng_job_end <= cmd_data[128 +: 32];
                  card_mode <= 1'b1;
                  port_clear <= 1'b1;
                  eng_start <= 1'b1;
                  run_hold <= 2'd2;
                end
                default: sbase <= cmd_data[128 +: 32];
              endcase
            end
            default: begin mask <= 0; need_desc <= 0; code <= UNKNOWN_OP; reported <= 1'b1; end
          endcase
        end
      end else if (reported) begin
        beat <= 1'b0;
        if (st_ready) begin
          busy <= 1'b0;
          reported <= 1'b0;
          seq <= seq + 1'b1;
        end
      end else begin
        beat_timer <= beat_timer + 1'b1;
        if (beat) begin
          if (st_ready) begin beat <= 1'b0; told <= progress; end
        end else if (&beat_timer && progress != told) beat <= 1'b1;
        for (i = 0; i < NB; i = i + 1) begin
          if (md_valid[i] && md_ready[i]) need_desc[i] <= 1'b0;
          if (mw_valid[i] && mw_ready[i]) begin
            sent[i] <= sent[i] + 1'b1;
            if (flips_left[i] != 0) flips_left[i] <= flips_left[i] - 1'b1;
          end
          if (mr_valid[i] && mr_ready[i]) begin
            seen[i] <= seen[i] + 1'b1;
            if (op == CHECK && mr_data[i*DWM +: DWM] !== pattern(seed, line_of(base, seen[i]))) wrong[i] <= wrong[i] + 1'b1;
          end
          if (mr_valid[i] && mr_taken[i]) begin
            fixed[i] <= fixed[i] + ones(mr_ecc[i*16 +: 8]);
            lost[i] <= lost[i] + ones(mr_ecc[i*16 + 8 +: 8]);
          end
          if (mdone_valid[i] && mask[i]) done[i] <= 1'b1;
        end
        if ((hrd_valid && hrd_ready) || (hwd_valid && hwd_ready)) need_hdesc <= 1'b0;
        if (hack_valid) acks <= acks + 1'b1;
        if (eng_host_wr) taken <= taken + 1'b1;
        if (store_read) begin
          if (hw_valid && hw_ready) begin
            sent_beats <= sent_beats + 1'b1;
            beat_ready <= 1'b0;
          end else beat_ready <= sent_beats < lines;
        end
        if (run_hold != 0) run_hold <= run_hold - 1'b1;
        if (packed_in) begin
          g <= (pin_emit ? g >> DWM : g) | (pin_take ? {{(64*GW-DWH){1'b0}}, hr_data} << (64 * pin_gn1) : {(64*GW){1'b0}});
          gn <= pin_gn1 + (pin_take ? HWD8 : 8'd0);
        end
        if (packed_out) begin
          g <= (pout_emit ? g >> DWH : g) | (pout_take ? {{(64*GW-DWM){1'b0}}, host_src} << (64 * pout_gn1) : {(64*GW){1'b0}});
          gn <= pout_gn1 + (pout_take ? LWD8 : 8'd0);
        end
        case (op)
          FILL, HOST_TO_MEM, HOST_TO_MEM_PACKED: if ((done | (mdone_valid & mask)) == mask && need_desc == 0) begin
            reported <= 1'b1;
          end
          // Every line compared, in the registers result0 sums, before the record is cut.
          CHECK: if (all_seen(mask) && need_desc == 0) begin
            reported <= 1'b1;
            result0 <= total;
          end
          MEM_TO_HOST, MEM_TO_HOST_PACKED, STORE_READ:
            if (acks + {{(LW-1){1'b0}}, hack_valid} == bursts && !need_hdesc) reported <= 1'b1;
          ENGINE_TABLE: if ((eng_host_wr ? taken_last : taken_all) && !need_hdesc) reported <= 1'b1;
          ENGINE_RUN: if (run_hold == 0 && !eng_busy) begin
            reported <= 1'b1;
            card_mode <= 1'b0;
            result0 <= {eng_pc, 30'd0, port_fault, eng_fault};
          end
          default: reported <= 1'b1;
        endcase
      end
    end
  end

  function all_seen(input [NB-1:0] m);
    integer j;
    begin
      all_seen = 1'b1;
      for (j = 0; j < NB; j = j + 1) if (m[j] && seen[j] < lines) all_seen = 1'b0;
    end
  endfunction
endmodule
"##;

const SHELL: &str = r##"// One memory channel as the core sees it: descriptors and lines to write (with ECC the bus bits each flips) crossing
// into the channel's clock through FIFOs, lines read (with ECC their words' verdicts) and write-done tokens crossing
// out of it, the channel's stream engine on the bank's Avalon-MM port. A read-data FIFO is deep enough to hold
// a whole run of bursts the channel has in flight. The FIFOs' sides toward the core step at its step (uce).
module logos_d5005_channel #(
  parameter AW = 27,           // channel line-address bits
  parameter DWM = 512,         // channel bus bits a line
  parameter ECC = 0,           // 1: each line's eight words SECDED(72,64)-coded in the bus's ECC lanes
  parameter MBW = 7,           // channel burstcount bits
  parameter LW = 32,
  parameter MAXB = 64,         // lines a burst
  parameter RAW = 9,           // log2 of the read-data FIFO's lines
  parameter WAW = 6,           // log2 of the write-data FIFO's lines
  parameter DWL = ECC ? DWM / 9 * 8 : DWM   // line bits the core moves
) (
  input              uclk,
  input              uce,
  input              uclk_rst_n,
  input              mem_clk,
  input              mem_rst_n,
  input              md_valid,
  output             md_ready,
  input  [AW+LW:0]   md_data,
  output             mr_valid,
  input              mr_ready,
  output [DWL-1:0]   mr_data,
  output [15:0]      mr_ecc,
  input              mw_valid,
  output             mw_ready,
  input  [DWL-1:0]   mw_data,
  input  [20:0]      mw_flip,
  output             mdone_valid,
  input              mdone_ready,
  output [AW-1:0]    mem_address,
  output             mem_read,
  output             mem_write,
  output [MBW-1:0]   mem_burstcount,
  output [DWM-1:0]   mem_writedata,
  output [DWM/8-1:0] mem_byteenable,
  input              mem_waitrequest,
  input  [DWM-1:0]   mem_readdata,
  input              mem_readdatavalid
);
  localparam DLW = AW + LW + 1;
  localparam RFW = ECC ? DWL + 16 : DWL;     // a read-FIFO word: the line, with ECC its words' verdicts
  localparam WFW = ECC ? DWL + 21 : DWL;     // a write-FIFO word: the line, with ECC the bus bits it flips
  wire           d_valid, d_ready, r_push, w_valid, w_ready, done;
  wire [DLW-1:0] d_data;
  wire [DWL-1:0] r_data, w_data;
  wire [15:0]    r_ecc;
  wire [20:0]    w_flip;
  wire [RFW-1:0] r_word, mr_word;
  wire [WFW-1:0] w_word, mw_word;
  wire [RAW:0]   r_room, r_used;
  wire [2:0]     d_wfree, d_rused, done_wfree, done_rused;
  wire [WAW:0]   w_wfree, w_rused;
  wire           r_wready, done_wready, done_token;
  generate if (ECC) begin : coded
    assign mw_word = {mw_flip, mw_data};
    assign {w_flip, w_data} = w_word;
    assign r_word = {r_ecc, r_data};
    assign {mr_ecc, mr_data} = mr_word;
  end else begin : plain
    assign mw_word = mw_data;
    assign w_data = w_word;
    assign w_flip = 21'h0;
    assign r_word = r_data;
    assign mr_data = mr_word;
    assign mr_ecc = 16'h0;
  end endgenerate
  logos_dcfifo_mlab #(.W(DLW), .AW(2), .HOLDS(1)) desc_fifo(.wclk(uclk), .wce(uce), .wrst_n(uclk_rst_n), .wvalid(md_valid),
    .wready(md_ready), .wdata(md_data), .wfree(d_wfree), .rclk(mem_clk), .rce(1'b1), .rrst_n(mem_rst_n), .rvalid(d_valid),
    .rready(d_ready), .rdata(d_data), .rused(d_rused));
  logos_dcfifo_m20k #(.W(WFW), .AW(WAW), .HOLDS(1)) write_fifo(.wclk(uclk), .wce(uce), .wrst_n(uclk_rst_n), .wvalid(mw_valid),
    .wready(mw_ready), .wdata(mw_word), .wfree(w_wfree), .rclk(mem_clk), .rce(1'b1), .rrst_n(mem_rst_n), .rvalid(w_valid),
    .rready(w_ready), .rdata(w_word), .rused(w_rused));
  logos_dcfifo_m20k #(.W(RFW), .AW(RAW), .HOLDS(1)) read_fifo(.wclk(mem_clk), .wce(1'b1), .wrst_n(mem_rst_n), .wvalid(r_push),
    .wready(r_wready), .wdata(r_word), .wfree(r_room), .rclk(uclk), .rce(uce), .rrst_n(uclk_rst_n), .rvalid(mr_valid),
    .rready(mr_ready), .rdata(mr_word), .rused(r_used));
  logos_dcfifo_mlab #(.W(1), .AW(2), .TOKENS(1)) done_fifo(.wclk(mem_clk), .wce(1'b1), .wrst_n(mem_rst_n), .wvalid(done),
    .wready(done_wready), .wdata(1'b1), .wfree(done_wfree), .rclk(uclk), .rce(uce), .rrst_n(uclk_rst_n),
    .rvalid(mdone_valid), .rready(mdone_ready), .rdata(done_token), .rused(done_rused));
  wire [RAW:0] owed;
  wire         busy;
  logos_stream_avmm #(.AW(AW), .DW(DWM), .BW(MBW), .LW(LW), .MAXB(MAXB), .RW(RAW+1), .ECC(ECC)) stream(
    .clk(mem_clk), .rst_n(mem_rst_n), .d_valid(d_valid), .d_ready(d_ready), .d_data(d_data),
    .r_valid(r_push), .r_data(r_data), .r_ecc(r_ecc), .r_room(r_room), .w_valid(w_valid), .w_ready(w_ready),
    .w_data(w_data), .w_flip(w_flip), .avm_address(mem_address), .avm_read(mem_read), .avm_write(mem_write),
    .avm_burstcount(mem_burstcount), .avm_writedata(mem_writedata), .avm_byteenable(mem_byteenable),
    .avm_waitrequest(mem_waitrequest), .avm_readdata(mem_readdata), .avm_readdatavalid(mem_readdatavalid),
    .owed(owed), .busy(busy), .wdone(done));
endmodule

// The engine's shell on the D5005: every port the OFS platform gives the accelerator, flat, each in its own clock,
// and the only paths between those clocks the dual-clock FIFOs.
//
//   pclk     the host channel: MMIO into the registers, host memory through a reader and a writer;
//   mem_clk  one clock a memory channel: its stream engine on the channel's Avalon-MM port;
//   uclk     the user clock: the core, the engine and every FIFO's side toward them.
//
// The shell makes no clock: its compute side — the core, an engine build's card port and every FIFO's side toward
// them — steps in the user clock at cce, high in the user cycles that end with the rising edges of the user clock
// divided by the build's divide (every cycle in an undivided build), from crst_n, a reset released at the second.
// The core sends descriptors and the lines it writes through FIFOs into each channel's clock and takes the lines
// read back through FIFOs out of it (logos_d5005_channel); commands and status records cross between the registers and
// the core the same way. With ECC set the channels' lines are SECDED-coded at each bank's port.
module logos_d5005_shell #(
  parameter NB = 4,            // memory channels
  parameter AW = 27,           // channel line-address bits
  parameter DWM = 512,         // channel bus bits a line
  parameter ECC = 0,           // 1: each line's eight words SECDED(72,64)-coded in the bus's ECC lanes
  parameter MBW = 7,           // channel burstcount bits
  parameter HAW = 51,          // host line-address bits
  parameter DWH = 512,         // host line bits
  parameter HBW = 7,           // host burstcount bits
  parameter MAW = 15,          // MMIO word-address bits
  parameter MAXB = 64,         // lines a burst
  parameter LW = 32,
  parameter RAW = 9,           // log2 of a read-data FIFO's lines
  parameter WAW = 6,           // log2 of a write-data FIFO's lines
  parameter [63:0] AFU_ID_L = 64'h0,
  parameter [63:0] AFU_ID_H = 64'h0,
  parameter [63:0] BUILD = 64'h0,
  parameter HB = 16            // log2 of the core cycles between a running command's progress records, well inside a
                               // host's wait: 42 ms of the engine build's 1.5625 MHz core against the card's minute of
                               // reads, some 2.6M reads against 2^24 in a simulation of the divided clock
) (
  input                  pclk,
  input                  pclk_rst_n,
  input                  uclk,
  input                  uclk_rst_n,
  // MMIO, a 64-bit Avalon-MM slave.
  input  [MAW-1:0]       mmio_address,
  input                  mmio_read,
  input                  mmio_write,
  input  [63:0]          mmio_writedata,
  input  [7:0]           mmio_byteenable,
  output                 mmio_waitrequest,
  output [63:0]          mmio_readdata,
  output                 mmio_readdatavalid,
  // Host memory, an Avalon-MM read/write master.
  output [HAW-1:0]       host_rd_address,
  output                 host_rd_read,
  output [HBW-1:0]       host_rd_burstcount,
  input                  host_rd_waitrequest,
  input  [DWH-1:0]       host_rd_readdata,
  input                  host_rd_readdatavalid,
  output [HAW-1:0]       host_wr_address,
  output                 host_wr_write,
  output [HBW-1:0]       host_wr_burstcount,
  output [DWH-1:0]       host_wr_writedata,
  output [DWH/8-1:0]     host_wr_byteenable,
  input                  host_wr_waitrequest,
  input                  host_wr_writeresponsevalid,
  // The memory channels, each an Avalon-MM master in its own clock.
  input  [NB-1:0]        mem_clk,
  input  [NB-1:0]        mem_rst_n,
  output [NB*AW-1:0]     mem_address,
  output [NB-1:0]        mem_read,
  output [NB-1:0]        mem_write,
  output [NB*MBW-1:0]    mem_burstcount,
  output [NB*DWM-1:0]    mem_writedata,
  output [NB*DWM/8-1:0]  mem_byteenable,
  input  [NB-1:0]        mem_waitrequest,
  input  [NB*DWM-1:0]    mem_readdata,
  input  [NB-1:0]        mem_readdatavalid
);
  localparam DLW = AW + LW + 1;
  localparam HDW = HAW + LW;
  localparam DWL = ECC ? DWM / 9 * 8 : DWM;  // line bits the core moves
/*ENGINE_PARAM*/
/*COMPUTE*/

  // Registers <-> core.
  wire         cmd_wvalid, cmd_wready, cmd_rvalid, cmd_rready;
  wire [255:0] cmd_wdata, cmd_rdata;
  wire [4:0]   cmd_wfree, cmd_rused;
  wire         st_wvalid, st_wready, st_rvalid, st_rready;
  wire [255:0] st_wdata, st_rdata;
  logos_csr #(.MAW(MAW), .AFU_ID_L(AFU_ID_L), .AFU_ID_H(AFU_ID_H), .BUILD(BUILD), .CW(5)) csr(
    .clk(pclk), .rst_n(pclk_rst_n), .mmio_address(mmio_address), .mmio_read(mmio_read), .mmio_write(mmio_write),
    .mmio_writedata(mmio_writedata), .mmio_byteenable(mmio_byteenable), .mmio_waitrequest(mmio_waitrequest),
    .mmio_readdata(mmio_readdata), .mmio_readdatavalid(mmio_readdatavalid), .cmd_valid(cmd_wvalid),
    .cmd_ready(cmd_wready), .cmd_data(cmd_wdata), .cmd_space(cmd_wfree), .st_valid(st_rvalid), .st_ready(st_rready),
    .st_data(st_rdata));
  logos_dcfifo_mlab #(.W(256), .AW(4)) cmd_fifo(.wclk(pclk), .wce(1'b1), .wrst_n(pclk_rst_n), .wvalid(cmd_wvalid),
    .wready(cmd_wready), .wdata(cmd_wdata), .wfree(cmd_wfree), .rclk(uclk), .rce(cce), .rrst_n(crst_n), .rvalid(cmd_rvalid),
    .rready(cmd_rready), .rdata(cmd_rdata), .rused(cmd_rused));
  wire [4:0] st_wfree, st_rused;
  logos_dcfifo_mlab #(.W(256), .AW(4)) st_fifo(.wclk(uclk), .wce(cce), .wrst_n(crst_n), .wvalid(st_wvalid), .wready(st_wready),
    .wdata(st_wdata), .wfree(st_wfree), .rclk(pclk), .rce(1'b1), .rrst_n(pclk_rst_n), .rvalid(st_rvalid),
    .rready(st_rready), .rdata(st_rdata), .rused(st_rused));

  // The core's side of every channel FIFO.
  wire [NB-1:0]     md_valid, md_ready, mr_valid, mr_ready, mw_valid, mw_ready, mdone_valid, mdone_ready;
  wire [NB*DLW-1:0] md_data;
  wire [NB*DWL-1:0] mr_data, mw_data;
  wire [NB*16-1:0]  mr_ecc;
  wire [NB*21-1:0]  mw_flip;

  genvar b;
  generate for (b = 0; b < NB; b = b + 1) begin : ch
    logos_d5005_channel #(.AW(AW), .DWM(DWM), .ECC(ECC), .MBW(MBW), .LW(LW), .MAXB(MAXB), .RAW(RAW), .WAW(WAW)) channel(
      .uclk(uclk), .uce(cce), .uclk_rst_n(crst_n), .mem_clk(mem_clk[b]), .mem_rst_n(mem_rst_n[b]),
      .md_valid(md_valid[b]), .md_ready(md_ready[b]), .md_data(md_data[b*DLW +: DLW]),
      .mr_valid(mr_valid[b]), .mr_ready(mr_ready[b]), .mr_data(mr_data[b*DWL +: DWL]), .mr_ecc(mr_ecc[b*16 +: 16]),
      .mw_valid(mw_valid[b]), .mw_ready(mw_ready[b]), .mw_data(mw_data[b*DWL +: DWL]), .mw_flip(mw_flip[b*21 +: 21]),
      .mdone_valid(mdone_valid[b]), .mdone_ready(mdone_ready[b]),
      .mem_address(mem_address[b*AW +: AW]), .mem_read(mem_read[b]), .mem_write(mem_write[b]),
      .mem_burstcount(mem_burstcount[b*MBW +: MBW]), .mem_writedata(mem_writedata[b*DWM +: DWM]),
      .mem_byteenable(mem_byteenable[b*DWM/8 +: DWM/8]), .mem_waitrequest(mem_waitrequest[b]),
      .mem_readdata(mem_readdata[b*DWM +: DWM]), .mem_readdatavalid(mem_readdatavalid[b]));
  end endgenerate

  // Host memory.
  wire           hrd_valid, hrd_ready, hwd_valid, hwd_ready, hr_valid, hr_ready, hw_valid, hw_ready, hack_valid, hack_ready;
  wire [HDW-1:0] hrd_data, hwd_data;
  wire [DWH-1:0] hr_data, hw_data;
  wire           phrd_valid, phrd_ready, phwd_valid, phwd_ready, phr_push, phr_wready, phw_valid, phw_ready, pack_wready;
  wire [HDW-1:0] phrd_data, phwd_data;
  wire [DWH-1:0] phr_data, phw_data;
  wire [2:0]     hrd_wfree, hrd_rused, hwd_wfree, hwd_rused;
  wire [RAW:0]   hr_room, hr_used;
  wire [WAW:0]   hw_wfree, hw_rused;
  wire [4:0]     hack_wfree, hack_rused;
  wire           hack_token;
  logos_dcfifo_mlab #(.W(HDW), .AW(2), .HOLDS(1)) hrd_fifo(.wclk(uclk), .wce(cce), .wrst_n(crst_n), .wvalid(hrd_valid), .wready(hrd_ready),
    .wdata(hrd_data), .wfree(hrd_wfree), .rclk(pclk), .rce(1'b1), .rrst_n(pclk_rst_n), .rvalid(phrd_valid),
    .rready(phrd_ready), .rdata(phrd_data), .rused(hrd_rused));
  logos_dcfifo_mlab #(.W(HDW), .AW(2), .HOLDS(1)) hwd_fifo(.wclk(uclk), .wce(cce), .wrst_n(crst_n), .wvalid(hwd_valid), .wready(hwd_ready),
    .wdata(hwd_data), .wfree(hwd_wfree), .rclk(pclk), .rce(1'b1), .rrst_n(pclk_rst_n), .rvalid(phwd_valid),
    .rready(phwd_ready), .rdata(phwd_data), .rused(hwd_rused));
  logos_dcfifo_m20k #(.W(DWH), .AW(RAW), .HOLDS(1)) hr_fifo(.wclk(pclk), .wce(1'b1), .wrst_n(pclk_rst_n), .wvalid(phr_push),
    .wready(phr_wready), .wdata(phr_data), .wfree(hr_room), .rclk(uclk), .rce(cce), .rrst_n(crst_n), .rvalid(hr_valid),
    .rready(hr_ready), .rdata(hr_data), .rused(hr_used));
  logos_dcfifo_m20k #(.W(DWH), .AW(WAW), .HOLDS(1)) hw_fifo(.wclk(uclk), .wce(cce), .wrst_n(crst_n), .wvalid(hw_valid), .wready(hw_ready),
    .wdata(hw_data), .wfree(hw_wfree), .rclk(pclk), .rce(1'b1), .rrst_n(pclk_rst_n), .rvalid(phw_valid),
    .rready(phw_ready), .rdata(phw_data), .rused(hw_rused));
  // A host write response cannot wait: a token, counted on the FIFO's write side until it is taken.
  logos_dcfifo_mlab #(.W(1), .AW(4), .TOKENS(1)) hack_fifo(.wclk(pclk), .wce(1'b1), .wrst_n(pclk_rst_n), .wvalid(host_wr_writeresponsevalid),
    .wready(pack_wready), .wdata(1'b1), .wfree(hack_wfree), .rclk(uclk), .rce(cce), .rrst_n(crst_n), .rvalid(hack_valid),
    .rready(hack_ready), .rdata(hack_token), .rused(hack_rused));
  wire [RAW:0] howed;
  wire         hrd_active, hwd_active, hw_done;
  logos_stream_reader #(.AW(HAW), .DW(DWH), .BW(HBW), .LW(LW), .MAXB(MAXB), .RW(RAW+1)) host_reader(
    .clk(pclk), .rst_n(pclk_rst_n), .d_valid(phrd_valid), .d_ready(phrd_ready), .d_data(phrd_data), .r_valid(phr_push),
    .r_data(phr_data), .r_room(hr_room), .avm_address(host_rd_address), .avm_read(host_rd_read),
    .avm_burstcount(host_rd_burstcount), .avm_waitrequest(host_rd_waitrequest), .avm_readdata(host_rd_readdata),
    .avm_readdatavalid(host_rd_readdatavalid), .owed(howed), .active(hrd_active));
  logos_stream_writer #(.AW(HAW), .DW(DWH), .BW(HBW), .LW(LW), .MAXB(MAXB)) host_writer(
    .clk(pclk), .rst_n(pclk_rst_n), .d_valid(phwd_valid), .d_ready(phwd_ready), .d_data(phwd_data), .w_valid(phw_valid),
    .w_ready(phw_ready), .w_data(phw_data), .avm_address(host_wr_address), .avm_write(host_wr_write),
    .avm_burstcount(host_wr_burstcount), .avm_writedata(host_wr_writedata), .avm_byteenable(host_wr_byteenable),
    .avm_waitrequest(host_wr_waitrequest), .active(hwd_active), .done(hw_done));

  // The channels are the core's, or the card port's while the core runs the engine's step.
  wire              card_mode;
  wire [NB-1:0]     c_md_valid, c_mr_ready, c_mw_valid, c_mdone_ready, p_md_valid, p_mr_ready, p_mw_valid, p_mdone_ready;
  wire [NB*DLW-1:0] c_md_data, p_md_data;
  wire [NB*DWL-1:0] c_mw_data, p_mw_data;
  wire [NB*21-1:0]  c_mw_flip;
  assign md_valid = card_mode ? p_md_valid : c_md_valid;
  assign md_data = card_mode ? p_md_data : c_md_data;
  assign mr_ready = card_mode ? p_mr_ready : c_mr_ready;
  assign mw_valid = card_mode ? p_mw_valid : c_mw_valid;
  assign mw_data = card_mode ? p_mw_data : c_mw_data;
  assign mw_flip = card_mode ? {(NB*21){1'b0}} : c_mw_flip;
  assign mdone_ready = card_mode ? p_mdone_ready : c_mdone_ready;

  wire          eng_start, eng_busy, eng_fault, eng_host_wr, port_clear, port_fault;
  wire [31:0]   eng_job_at, eng_job_end, eng_pc, eng_host_addr, eng_host_raddr, port_space;
  wire [15:0]   eng_rows;
  wire [3:0]    eng_knobs;
  wire [2:0]    eng_host_table;
  wire [DWH-1:0] eng_host_wdata, eng_host_rdata;
  wire [AW-1:0] port_base;
  wire [4:0]    port_granule_log2;
  wire moved = |(mr_valid & mr_ready) || |(mw_valid & mw_ready);
  logos_bringup_core #(.NB(NB), .AW(AW), .DWM(DWL), .ECC(ECC), .ENGINE(ENGINE), .HAW(HAW), .DWH(DWH), .LW(LW),
                       .HMAXB(MAXB), .HB(HB)) core(
    .clk(uclk), .ce(cce), .rst_n(crst_n), .cmd_valid(cmd_rvalid), .cmd_ready(cmd_rready), .cmd_data(cmd_rdata),
    .st_valid(st_wvalid), .st_ready(st_wready), .st_data(st_wdata), .md_valid(c_md_valid), .md_ready(md_ready),
    .md_data(c_md_data), .mr_valid(mr_valid), .mr_ready(c_mr_ready), .mr_data(mr_data), .mr_ecc(mr_ecc),
    .mw_valid(c_mw_valid), .mw_ready(mw_ready), .mw_data(c_mw_data), .mw_flip(c_mw_flip), .mdone_valid(mdone_valid),
    .mdone_ready(c_mdone_ready),
    .hrd_valid(hrd_valid), .hrd_ready(hrd_ready), .hrd_data(hrd_data), .hr_valid(hr_valid), .hr_ready(hr_ready),
    .hr_data(hr_data), .hwd_valid(hwd_valid), .hwd_ready(hwd_ready), .hwd_data(hwd_data), .hw_valid(hw_valid),
    .hw_ready(hw_ready), .hw_data(hw_data), .hack_valid(hack_valid), .hack_ready(hack_ready), .mr_taken(mr_ready),
    .moved(moved),
    .card_mode(card_mode), .eng_start(eng_start), .eng_job_at(eng_job_at), .eng_job_end(eng_job_end),
    .eng_rows(eng_rows), .eng_knobs(eng_knobs), .eng_busy(eng_busy), .eng_fault(eng_fault), .eng_pc(eng_pc),
    .eng_host_wr(eng_host_wr), .eng_host_table(eng_host_table), .eng_host_addr(eng_host_addr),
    .eng_host_wdata(eng_host_wdata), .eng_host_raddr(eng_host_raddr), .eng_host_rdata(eng_host_rdata),
    .port_space(port_space), .port_base(port_base), .port_granule_log2(port_granule_log2), .port_clear(port_clear),
    .port_fault(port_fault));
/*ENGINE*/
endmodule
"##;

/// The shell's engine with its card port, between the core's engine ports and the channels' card side.
const SHELL_ENGINE: &str = r##"
  // The engine, its memory port the card port on the channels.
  wire          e_rd, e_rd_valid, e_wr, e_wr_ready;
  wire [31:0]   e_rd_line, e_wr_line;
  wire [511:0]  e_rd_data, e_wr_data;
  wire [63:0]   e_wr_mask;
  logos_engine engine(.clk(uclk), .rst(!crst_n), .start(eng_start), .job_at(eng_job_at), .job_end(eng_job_end),
    .rows(eng_rows), .state_f16(eng_knobs[0]), .cache_int8(eng_knobs[1]), .two_words(eng_knobs[2]),
    .per_block(eng_knobs[3]), .busy(eng_busy), .fault(eng_fault), .pc(eng_pc), .host_wr(eng_host_wr),
    .host_table(eng_host_table), .host_addr(eng_host_addr), .host_wdata(eng_host_wdata), .host_raddr(eng_host_raddr),
    .host_rdata(eng_host_rdata), .mem_rd(e_rd), .mem_rd_line(e_rd_line), .mem_rd_valid(e_rd_valid),
    .mem_rd_data(e_rd_data), .mem_wr(e_wr), .mem_wr_line(e_wr_line), .mem_wr_data(e_wr_data), .mem_wr_mask(e_wr_mask),
    .mem_wr_ready(e_wr_ready));
  logos_card_port #(.NB(NB), .AW(AW), .LW(LW), .DL(DWL)) port(.clk(uclk), .ce(cce), .rst_n(crst_n), .space(port_space),
    .base(port_base), .granule_log2(port_granule_log2), .clear(port_clear), .fault(port_fault), .mem_rd(e_rd),
    .mem_rd_line(e_rd_line), .mem_rd_valid(e_rd_valid), .mem_rd_data(e_rd_data), .mem_wr(e_wr),
    .mem_wr_line(e_wr_line), .mem_wr_data(e_wr_data), .mem_wr_mask(e_wr_mask), .mem_wr_ready(e_wr_ready),
    .md_valid(p_md_valid), .md_ready(md_ready), .md_data(p_md_data), .mr_valid(mr_valid), .mr_ready(p_mr_ready),
    .mr_data(mr_data), .mr_ecc(mr_ecc), .mw_valid(p_mw_valid), .mw_ready(mw_ready), .mw_data(p_mw_data),
    .mdone_valid(mdone_valid), .mdone_ready(p_mdone_ready));
"##;

/// A shell without an engine: the engine's ports idle, the card side of the channels never asking.
const SHELL_NO_ENGINE: &str = r##"
  assign eng_busy = 1'b0;
  assign eng_fault = 1'b0;
  assign eng_pc = 32'd0;
  assign eng_host_rdata = {DWH{1'b0}};
  assign port_fault = 1'b0;
  assign p_md_valid = {NB{1'b0}};
  assign p_md_data = {(NB*DLW){1'b0}};
  assign p_mr_ready = {NB{1'b0}};
  assign p_mw_valid = {NB{1'b0}};
  assign p_mw_data = {(NB*DWL){1'b0}};
  assign p_mdone_ready = {NB{1'b0}};
"##;

const SDC: &str = r##"# The shell's dual-clock FIFOs, constrained as the D5005 FIM constrains its own (syn/setup/dcfifo.sdc): each Gray
# pointer's bits cross with their skew under 0.8 of the source clock's period and their net delay under 0.8 of the
# destination's, setup and hold relaxed on those paths; the net between a synchronizer's two registers held to 0.8 of
# its clock; and a RAM word read on the other side, written two destination edges before its pointer arrived, held to
# the pointer's net delay.

proc logos_crossing {from to} {
  if {[get_collection_size $from] == 0 || [get_collection_size $to] == 0} { return }
  set_max_skew -from $from -to $to -get_skew_value_from_clock_period src_clock_period -skew_value_multiplier 0.8
  if { ![string equal "quartus_syn" $::TimeQuestInfo(nameofexecutable)] } {
    set_net_delay -from $from -to $to -max -get_value_from_clock_period dst_clock_period -value_multiplier 0.8
  }
  set_max_delay -from $from -to $to 100
  set_min_delay -from $from -to $to -100
}

proc logos_synchronizer {from to} {
  if {[get_collection_size $from] == 0 || [get_collection_size $to] == 0} { return }
  if { ![string equal "quartus_syn" $::TimeQuestInfo(nameofexecutable)] } {
    set_net_delay -from $from -to $to -max -get_value_from_clock_period dst_clock_period -value_multiplier 0.8
  }
}

foreach entity {logos_dcfifo_mlab logos_dcfifo_m20k} {
  foreach inst [get_entity_instances $entity] {
    logos_crossing [get_keepers $inst|wgray[*]] [get_keepers $inst|wgray_s1[*]]
    logos_crossing [get_keepers $inst|rgray[*]] [get_keepers $inst|rgray_s1[*]]
    logos_synchronizer [get_keepers $inst|wgray_s1[*]] [get_keepers $inst|wgray_s2[*]]
    logos_synchronizer [get_keepers $inst|rgray_s1[*]] [get_keepers $inst|rgray_s2[*]]
    logos_crossing [get_keepers $inst|mem*] [get_keepers $inst|rdata_q[*]]
  }
}
"##;

const HAL_H: &str = r##"/* The device a host program drives the D5005 accelerator through: its MMIO words, buffers the accelerator reads and
   writes by I/O address, and its user clock. OPAE implements it on the card (logos_hal_opae.c), the card's PCI function
   driven directly implements it on a host whose devices reach memory at physical addresses (logos_hal_pci.c), and a
   simulation of the accelerator's RTL implements it through its simulator's bridge: Icarus Verilog's VPI
   (logos_hal_vpi.c) or Verilator's DPI-C (logos_hal_dpi.c). */
#ifndef LOGOS_HAL_H
#define LOGOS_HAL_H

#include <stddef.h>
#include <stdint.h>

typedef struct logos_dev logos_dev;

/* Open the accelerator whose AFU ID is `afu_id` (the UUID text); NULL with a message on stderr if there is none. */
logos_dev *logos_open(const char *afu_id);
void logos_close(logos_dev *dev);

uint64_t logos_mmio_read(logos_dev *dev, uint32_t byte_offset);
void logos_mmio_write(logos_dev *dev, uint32_t byte_offset, uint64_t value);

/* A buffer of `bytes` the accelerator can read and write, 64-byte lines at the I/O address `*io_address`; NULL with a
   message on stderr if it cannot be had. */
void *logos_buffer(logos_dev *dev, size_t bytes, uint64_t *io_address);
void logos_buffer_free(logos_dev *dev, void *buffer);

/* The clock the accelerator's core runs at, in Hz: the user clock, divided by the build's compute divide. */
double logos_user_clock_hz(logos_dev *dev);

#endif
"##;

const HAL_OPAE: &str = r##"/* The D5005 accelerator through OPAE 2.12 (libopae-c): the accelerator found by its AFU ID through whichever plugin
   serves it — the PF's port under the DFL driver, or on the OFS FIM PF0's VF1 bound to vfio-pci
   (scripts/d5005/afu-vf.sh) — MMIO region 0, buffers prepared and pinned by the driver or the IOMMU, the user clock as
   the FIM's user-clock PLL reports it, or for a port with no user-clock interface (a VF under VFIO) the one it was set
   to before it was handed over, LOGOS_USER_CLOCK_MHZ. */
#include "logos_d5005.h"
#include "logos_hal.h"

#include <opae/fpga.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define LOGOS_MAX_BUFFERS 64

struct logos_dev {
    fpga_token token;
    fpga_handle handle;
    struct { void *ptr; uint64_t wsid; } buffers[LOGOS_MAX_BUFFERS];
};

static int parse_guid(const char *text, fpga_guid guid) {
    int n = 0;
    for (const char *p = text; *p && n < 32; ++p) {
        int v;
        if (*p == '-') continue;
        if (*p >= '0' && *p <= '9') v = *p - '0';
        else if (*p >= 'a' && *p <= 'f') v = *p - 'a' + 10;
        else if (*p >= 'A' && *p <= 'F') v = *p - 'A' + 10;
        else return -1;
        if (n % 2 == 0) guid[n / 2] = (uint8_t)(v << 4);
        else guid[n / 2] |= (uint8_t)v;
        ++n;
    }
    return n == 32 ? 0 : -1;
}

logos_dev *logos_open(const char *afu_id) {
    fpga_properties filter = NULL;
    fpga_guid guid;
    uint32_t found = 0;
    fpga_result r;
    logos_dev *dev = calloc(1, sizeof *dev);
    if (!dev) return NULL;
    if (parse_guid(afu_id, guid) != 0) {
        fprintf(stderr, "logos_open: %s is not a UUID\n", afu_id);
        free(dev);
        return NULL;
    }
    if ((r = fpgaGetProperties(NULL, &filter)) != FPGA_OK ||
        (r = fpgaPropertiesSetObjectType(filter, FPGA_ACCELERATOR)) != FPGA_OK ||
        (r = fpgaPropertiesSetGUID(filter, guid)) != FPGA_OK ||
        (r = fpgaEnumerate(&filter, 1, &dev->token, 1, &found)) != FPGA_OK) {
        fprintf(stderr, "logos_open: enumerating accelerators: %s\n", fpgaErrStr(r));
        fpgaDestroyProperties(&filter);
        free(dev);
        return NULL;
    }
    fpgaDestroyProperties(&filter);
    if (found == 0) {
        fprintf(stderr, "logos_open: no accelerator %s: is its image loaded (fpgaconf, or the FIM over JTAG) and its port on a "
                        "function this user opens (the PF's port, or a VF bound to vfio-pci)?\n", afu_id);
        free(dev);
        return NULL;
    }
    if ((r = fpgaOpen(dev->token, &dev->handle, 0)) != FPGA_OK || (r = fpgaMapMMIO(dev->handle, 0, NULL)) != FPGA_OK) {
        fprintf(stderr, "logos_open: opening %s: %s\n", afu_id, fpgaErrStr(r));
        fpgaDestroyToken(&dev->token);
        free(dev);
        return NULL;
    }
    return dev;
}

void logos_close(logos_dev *dev) {
    for (int i = 0; i < LOGOS_MAX_BUFFERS; ++i)
        if (dev->buffers[i].ptr) fpgaReleaseBuffer(dev->handle, dev->buffers[i].wsid);
    fpgaUnmapMMIO(dev->handle, 0);
    fpgaClose(dev->handle);
    fpgaDestroyToken(&dev->token);
    free(dev);
}

uint64_t logos_mmio_read(logos_dev *dev, uint32_t byte_offset) {
    uint64_t v = 0;
    fpga_result r = fpgaReadMMIO64(dev->handle, 0, byte_offset, &v);
    if (r != FPGA_OK) {
        fprintf(stderr, "logos_mmio_read 0x%x: %s\n", byte_offset, fpgaErrStr(r));
        exit(3);
    }
    return v;
}

void logos_mmio_write(logos_dev *dev, uint32_t byte_offset, uint64_t value) {
    fpga_result r = fpgaWriteMMIO64(dev->handle, 0, byte_offset, value);
    if (r != FPGA_OK) {
        fprintf(stderr, "logos_mmio_write 0x%x: %s\n", byte_offset, fpgaErrStr(r));
        exit(3);
    }
}

void *logos_buffer(logos_dev *dev, size_t bytes, uint64_t *io_address) {
    for (int i = 0; i < LOGOS_MAX_BUFFERS; ++i) {
        if (dev->buffers[i].ptr) continue;
        void *ptr = NULL;
        uint64_t wsid = 0;
        fpga_result r = fpgaPrepareBuffer(dev->handle, bytes, &ptr, &wsid, 0);
        if (r == FPGA_OK) r = fpgaGetIOAddress(dev->handle, wsid, io_address);
        if (r != FPGA_OK) {
            fprintf(stderr, "logos_buffer %zu bytes: %s\n", bytes, fpgaErrStr(r));
            if (ptr) fpgaReleaseBuffer(dev->handle, wsid);
            return NULL;
        }
        dev->buffers[i].ptr = ptr;
        dev->buffers[i].wsid = wsid;
        return ptr;
    }
    fprintf(stderr, "logos_buffer: more than %d buffers\n", LOGOS_MAX_BUFFERS);
    return NULL;
}

void logos_buffer_free(logos_dev *dev, void *buffer) {
    for (int i = 0; i < LOGOS_MAX_BUFFERS; ++i) {
        if (dev->buffers[i].ptr == buffer) {
            fpgaReleaseBuffer(dev->handle, dev->buffers[i].wsid);
            dev->buffers[i].ptr = NULL;
        }
    }
}

double logos_user_clock_hz(logos_dev *dev) {
    uint64_t high = 0, low = 0;
    fpga_result r = fpgaGetUserClock(dev->handle, &high, &low, 0);
    if (r == FPGA_OK) return (double)high * 1e6 / LOGOS_D5005_COMPUTE_DIVIDE;
    const char *mhz = getenv("LOGOS_USER_CLOCK_MHZ");
    if (mhz && *mhz) return strtod(mhz, NULL) * 1e6 / LOGOS_D5005_COMPUTE_DIVIDE;
    fprintf(stderr, "logos_user_clock_hz: %s, and LOGOS_USER_CLOCK_MHZ is not set\n", fpgaErrStr(r));
    exit(3);
}
"##;

const HAL_PCI: &str = r##"/* The D5005 accelerator driven through its PCI function directly, on a host whose devices reach its memory at
   physical addresses (no IOMMU, or the function in an identity domain), with no driver and no OPAE. The PR region's
   AFU is a VF of the FIM's PF (8086:bccf, PF0's VF1; scripts/d5005/afu-vf.sh attach enables the VFs and leaves them to
   no driver), found among the functions of LOGOS_PCI_DEVICES (default /sys/bus/pci/devices), or only LOGOS_D5005_VF
   when that is set, by the AFU ID its MMIO words 1 and 2 carry. BAR 0 is mapped from the function's resource0. Its command word decodes memory
   and masters the bus only while it is open, and gets back the value it had on close, at exit, and on SIGINT, SIGTERM
   and SIGHUP, so the accelerator never writes into memory the program no longer holds. Each buffer is one huge page
   (2 MiB, or 1 GiB past that, from the pages reserved in /sys/kernel/mm/hugepages); its I/O address is the page's
   physical address, read from /proc/self/pagemap and checked contiguous, and a buffer whose frames cannot be read is
   refused. The user clock is LOGOS_USER_CLOCK_MHZ, the one the port ran at before it was released. The BAR, the command
   word and the physical frames are root's. */
#define _GNU_SOURCE
#include "logos_d5005.h"
#include "logos_hal.h"

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#ifndef MAP_HUGE_SHIFT
#define MAP_HUGE_SHIFT 26
#endif

#define LOGOS_MAX_BUFFERS 64
#define LOGOS_PCI_COMMAND 4
#define LOGOS_PCI_MEMORY 0x0002u
#define LOGOS_PCI_BUS_MASTER 0x0004u
#define LOGOS_FRAME 4096ull
#define LOGOS_FRAME_PRESENT (1ull << 63)
#define LOGOS_FRAME_NUMBER ((1ull << 55) - 1)

struct logos_dev {
    int config, resource;
    uint16_t command;
    volatile uint64_t *bar;
    size_t bar_bytes;
    struct { void *ptr; size_t bytes; } buffers[LOGOS_MAX_BUFFERS];
};

/* The open function's configuration space and the command word it had, given back at exit or on a signal. */
static volatile sig_atomic_t held_config = -1;
static volatile sig_atomic_t held_command;

static int command_read(int config, uint16_t *word) {
    uint8_t b[2];
    if (pread(config, b, 2, LOGOS_PCI_COMMAND) != 2) return -1;
    *word = (uint16_t)(b[0] | b[1] << 8);
    return 0;
}

static int command_write(int config, uint16_t word) {
    uint8_t b[2] = {(uint8_t)word, (uint8_t)(word >> 8)};
    return pwrite(config, b, 2, LOGOS_PCI_COMMAND) == 2 ? 0 : -1;
}

static void give_back(void) {
    int config = held_config;
    if (config < 0) return;
    held_config = -1;
    uint8_t b[2] = {(uint8_t)held_command, (uint8_t)(held_command >> 8)};
    ssize_t n = pwrite(config, b, 2, LOGOS_PCI_COMMAND);
    (void)n;
}

static void on_signal(int sig) {
    give_back();
    raise(sig);
}

static void hold(int config, uint16_t command) {
    static int armed = 0;
    held_command = command;
    held_config = config;
    if (armed) return;
    armed = 1;
    atexit(give_back);
    struct sigaction sa;
    memset(&sa, 0, sizeof sa);
    sa.sa_handler = on_signal;
    sa.sa_flags = SA_RESETHAND;
    sigemptyset(&sa.sa_mask);
    sigaction(SIGINT, &sa, NULL);
    sigaction(SIGTERM, &sa, NULL);
    sigaction(SIGHUP, &sa, NULL);
}

static int parse_id(const char *text, uint64_t *high, uint64_t *low) {
    int n = 0;
    *high = *low = 0;
    for (const char *p = text; *p; ++p) {
        uint64_t v;
        if (*p == '-') continue;
        if (*p >= '0' && *p <= '9') v = (uint64_t)(*p - '0');
        else if (*p >= 'a' && *p <= 'f') v = (uint64_t)(*p - 'a' + 10);
        else if (*p >= 'A' && *p <= 'F') v = (uint64_t)(*p - 'A' + 10);
        else return -1;
        if (n == 32) return -1;
        if (n < 16) *high = *high << 4 | v;
        else *low = *low << 4 | v;
        ++n;
    }
    return n == 32 ? 0 : -1;
}

/* `<dir>/<name>` into `out`; -1 if it does not fit. */
static int join(char *out, size_t size, const char *dir, const char *name) {
    int n = snprintf(out, size, "%s/%s", dir, name);
    return n >= 0 && (size_t)n < size ? 0 : -1;
}

/* The text of `<dir>/<name>`, trailing whitespace dropped; -1 if there is no such file. */
static int read_text(const char *dir, const char *name, char *out, size_t size) {
    char path[4096];
    if (join(path, sizeof path, dir, name) != 0) return -1;
    FILE *f = fopen(path, "r");
    if (!f) return -1;
    size_t n = fread(out, 1, size - 1, f);
    fclose(f);
    while (n > 0 && (out[n - 1] == '\n' || out[n - 1] == ' ')) --n;
    out[n] = 0;
    return 0;
}

/* Open `function` under `devices` as the accelerator if it is one this backend may drive and its MMIO words 1 and 2
   are the AFU ID: 1 open, 0 not the accelerator (its command word given back), -1 refused with a message. */
static int try_function(logos_dev *dev, const char *devices, const char *function, int pinned, uint64_t high, uint64_t low) {
    char dir[4096], text[256], path[4096];
    if (join(dir, sizeof dir, devices, function) != 0) return 0;
    if (!pinned) {
        char vendor[32], device[32];
        if (read_text(dir, "vendor", vendor, sizeof vendor) || read_text(dir, "device", device, sizeof device)) return 0;
        if (strcmp(vendor, "0x8086") != 0 || strcmp(device, "0xbccf") != 0) return 0;
    }
    char driver[4096];
    if (join(path, sizeof path, dir, "driver") != 0) return 0;
    ssize_t k = readlink(path, driver, sizeof driver - 1);
    if (k >= 0) {
        driver[k] = 0;
        const char *name = strrchr(driver, '/');
        fprintf(stderr, "logos_open: %s has the driver %s bound; this backend drives a function with none\n", function, name ? name + 1 : driver);
        return -1;
    }
    if (join(path, sizeof path, dir, "iommu_group") != 0) return 0;
    if (access(path, F_OK) == 0) {
        if (read_text(dir, "iommu_group/type", text, sizeof text) != 0) strcpy(text, "unreadable");
        if (strcmp(text, "identity") != 0) {
            fprintf(stderr, "logos_open: %s is behind an IOMMU that translates its addresses (its group's domain is %s): "
                            "open it through VFIO (sw/logos_hal_opae.c)\n", function, text);
            return -1;
        }
    }
    if (join(path, sizeof path, dir, "config") != 0) return 0;
    int config = open(path, O_RDWR | O_CLOEXEC);
    uint16_t command = 0;
    if (config < 0 || command_read(config, &command) != 0) {
        fprintf(stderr, "logos_open: %s: %s (the command word is root's)\n", path, strerror(errno));
        if (config >= 0) close(config);
        return -1;
    }
    if (join(path, sizeof path, dir, "resource0") != 0) {
        close(config);
        return 0;
    }
    int resource = open(path, O_RDWR | O_SYNC | O_CLOEXEC);
    struct stat st;
    void *bar = MAP_FAILED;
    if (resource >= 0 && fstat(resource, &st) == 0 && st.st_size >= 24)
        bar = mmap(NULL, (size_t)st.st_size, PROT_READ | PROT_WRITE, MAP_SHARED, resource, 0);
    if (bar == MAP_FAILED) {
        fprintf(stderr, "logos_open: %s: %s (BAR 0 is root's)\n", path, strerror(errno));
        if (resource >= 0) close(resource);
        close(config);
        return -1;
    }
    if (command_write(config, (uint16_t)(command | LOGOS_PCI_MEMORY)) != 0) {
        fprintf(stderr, "logos_open: %s: writing the command word: %s\n", function, strerror(errno));
        munmap(bar, (size_t)st.st_size);
        close(resource);
        close(config);
        return -1;
    }
    volatile uint64_t *words = bar;
    if (words[2] != high || words[1] != low) {
        command_write(config, command);
        munmap(bar, (size_t)st.st_size);
        close(resource);
        close(config);
        return 0;
    }
    hold(config, command);
    if (command_write(config, (uint16_t)(command | LOGOS_PCI_MEMORY | LOGOS_PCI_BUS_MASTER)) != 0) {
        fprintf(stderr, "logos_open: %s: writing the command word: %s\n", function, strerror(errno));
        give_back();
        munmap(bar, (size_t)st.st_size);
        close(resource);
        close(config);
        return -1;
    }
    dev->config = config;
    dev->resource = resource;
    dev->command = command;
    dev->bar = words;
    dev->bar_bytes = (size_t)st.st_size;
    return 1;
}

logos_dev *logos_open(const char *afu_id) {
    uint64_t high, low;
    if (parse_id(afu_id, &high, &low) != 0) {
        fprintf(stderr, "logos_open: %s is not a UUID\n", afu_id);
        return NULL;
    }
    logos_dev *dev = calloc(1, sizeof *dev);
    if (!dev) return NULL;
    const char *devices = getenv("LOGOS_PCI_DEVICES");
    if (!devices || !*devices) devices = "/sys/bus/pci/devices";
    const char *pinned = getenv("LOGOS_D5005_VF");
    if (pinned && *pinned) {
        if (try_function(dev, devices, pinned, 1, high, low) == 1) return dev;
        fprintf(stderr, "logos_open: no accelerator %s at %s\n", afu_id, pinned);
        free(dev);
        return NULL;
    }
    DIR *d = opendir(devices);
    if (!d) {
        fprintf(stderr, "logos_open: %s: %s\n", devices, strerror(errno));
        free(dev);
        return NULL;
    }
    for (struct dirent *e; (e = readdir(d)) != NULL;) {
        if (e->d_name[0] == '.') continue;
        if (try_function(dev, devices, e->d_name, 0, high, low) == 1) {
            closedir(d);
            return dev;
        }
    }
    closedir(d);
    fprintf(stderr, "logos_open: no accelerator %s on a VF of the FIM's PF (8086:bccf) with no driver bound: is its image "
                    "configured, and the PF's VFs enabled (scripts/d5005/afu-vf.sh attach)?\n", afu_id);
    free(dev);
    return NULL;
}

void logos_close(logos_dev *dev) {
    give_back();
    for (int i = 0; i < LOGOS_MAX_BUFFERS; ++i)
        if (dev->buffers[i].ptr) munmap(dev->buffers[i].ptr, dev->buffers[i].bytes);
    munmap((void *)dev->bar, dev->bar_bytes);
    close(dev->resource);
    close(dev->config);
    free(dev);
}

static size_t word_of(logos_dev *dev, uint32_t byte_offset, const char *what) {
    if (byte_offset % 8 != 0 || (size_t)byte_offset + 8 > dev->bar_bytes) {
        fprintf(stderr, "%s 0x%x: not a word of the %zu-byte BAR\n", what, byte_offset, dev->bar_bytes);
        exit(3);
    }
    return byte_offset / 8;
}

uint64_t logos_mmio_read(logos_dev *dev, uint32_t byte_offset) {
    return dev->bar[word_of(dev, byte_offset, "logos_mmio_read")];
}

void logos_mmio_write(logos_dev *dev, uint32_t byte_offset, uint64_t value) {
    dev->bar[word_of(dev, byte_offset, "logos_mmio_write")] = value;
}

/* The physical address of the `bytes` at `ptr`, frame by frame from the page map `pagemap` (an entry of 64 bits a
   4 KiB page of the address space: bit 63 present, bits 54..0 the frame): 0 if every frame is present, read and the
   one after the frame before it. */
static int frames(int pagemap, const void *ptr, size_t bytes, uint64_t *address) {
    uint64_t first = 0;
    for (size_t at = 0; at < bytes; at += LOGOS_FRAME) {
        uint64_t entry = 0;
        off_t where = (off_t)(((uintptr_t)ptr + at) / LOGOS_FRAME * 8);
        if (pread(pagemap, &entry, 8, where) != 8 || !(entry & LOGOS_FRAME_PRESENT) || (entry & LOGOS_FRAME_NUMBER) == 0) {
            fprintf(stderr, "logos_buffer: the page map gives no physical frame for this user (root reads them)\n");
            return -1;
        }
        if (at == 0) {
            first = entry & LOGOS_FRAME_NUMBER;
        } else if ((entry & LOGOS_FRAME_NUMBER) != first + at / LOGOS_FRAME) {
            fprintf(stderr, "logos_buffer: the huge page's frames are not contiguous\n");
            return -1;
        }
    }
    *address = first * LOGOS_FRAME;
    return 0;
}

static int physical(const void *ptr, size_t bytes, uint64_t *address) {
    int fd = open("/proc/self/pagemap", O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        fprintf(stderr, "logos_buffer: /proc/self/pagemap: %s\n", strerror(errno));
        return -1;
    }
    int r = frames(fd, ptr, bytes, address);
    close(fd);
    return r;
}

void *logos_buffer(logos_dev *dev, size_t bytes, uint64_t *io_address) {
    const size_t small = (size_t)2 << 20, large = (size_t)1 << 30;
    *io_address = 0;
    if (bytes == 0 || bytes > large) {
        fprintf(stderr, "logos_buffer %zu bytes: a buffer is one huge page, at most 1 GiB\n", bytes);
        return NULL;
    }
    int slot = -1;
    for (int i = 0; i < LOGOS_MAX_BUFFERS && slot < 0; ++i)
        if (!dev->buffers[i].ptr) slot = i;
    if (slot < 0) {
        fprintf(stderr, "logos_buffer: more than %d buffers\n", LOGOS_MAX_BUFFERS);
        return NULL;
    }
    size_t page = bytes <= small ? small : large;
    int shift = page == small ? 21 : 30;
    void *ptr = mmap(NULL, page, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_HUGETLB | MAP_POPULATE | shift << MAP_HUGE_SHIFT,
                     -1, 0);
    if (ptr == MAP_FAILED) {
        fprintf(stderr, "logos_buffer %zu bytes: no free %zu kB huge page (%s): reserve them in "
                        "/sys/kernel/mm/hugepages/hugepages-%zukB/nr_hugepages\n", bytes, page >> 10, strerror(errno), page >> 10);
        return NULL;
    }
    uint64_t address = 0;
    if (physical(ptr, page, &address) != 0) {
        fprintf(stderr, "logos_buffer %zu bytes: refused, its I/O address unknown\n", bytes);
        munmap(ptr, page);
        return NULL;
    }
    dev->buffers[slot].ptr = ptr;
    dev->buffers[slot].bytes = page;
    *io_address = address;
    return ptr;
}

void logos_buffer_free(logos_dev *dev, void *buffer) {
    for (int i = 0; i < LOGOS_MAX_BUFFERS; ++i) {
        if (dev->buffers[i].ptr == buffer) {
            munmap(buffer, dev->buffers[i].bytes);
            dev->buffers[i].ptr = NULL;
        }
    }
}

double logos_user_clock_hz(logos_dev *dev) {
    (void)dev;
    const char *mhz = getenv("LOGOS_USER_CLOCK_MHZ");
    if (mhz && *mhz) return strtod(mhz, NULL) * 1e6 / LOGOS_D5005_COMPUTE_DIVIDE;
    fprintf(stderr, "logos_user_clock_hz: a VF has no user-clock interface, and LOGOS_USER_CLOCK_MHZ is not set\n");
    exit(3);
}
"##;

const CORE_H: &str = r##"/* The bring-up core's commands as a host program rings them: the arguments, the doorbell, and the status record the core
   cuts when the command completes. A wait is bounded by the command's progress, not its length: the core reports a
   running command's progress while it moves (STATUS {tag, LOGOS_D5005_STATUS_RUNNING}, RESULT0 the count), and once
   LOGOS_CORE_MAX_POLLS register reads pass with no newer progress of this command than the wait last saw, the command
   is given up, its record {status 0xff, no tag} with the results of the last progress record it cut (none if it cut
   none: an engine's carries each context's job word), so a core that stops moving fails the program rather than
   hanging it, however long a moving command takes, and says where it stood. */
#ifndef LOGOS_CORE_H
#define LOGOS_CORE_H

#include "logos_d5005.h"
#include "logos_hal.h"

#ifndef LOGOS_CORE_MAX_POLLS
#define LOGOS_CORE_MAX_POLLS (1ull << 26)
#endif

typedef struct {
    uint64_t status, result0, result1;
} logos_record;

/* Ring `op` tagged `tag` with its three arguments once the command FIFO has room, and wait for its status record. */
static inline logos_record logos_core_run(logos_dev *dev, uint8_t op, uint64_t tag, uint64_t a0, uint64_t a1, uint64_t a2) {
    logos_record r = {0xff, 0, 0};
    uint64_t seq = logos_mmio_read(dev, LOGOS_D5005_REG_STATUS_SEQ), polls = 0, seen = 0;
    while (logos_mmio_read(dev, LOGOS_D5005_REG_CMD_SPACE) == 0) {
        if (++polls > LOGOS_CORE_MAX_POLLS) return r;
    }
    logos_mmio_write(dev, LOGOS_D5005_REG_ARG0, a0);
    logos_mmio_write(dev, LOGOS_D5005_REG_ARG1, a1);
    logos_mmio_write(dev, LOGOS_D5005_REG_ARG2, a2);
    logos_mmio_write(dev, LOGOS_D5005_REG_DOORBELL, (tag << 8) | op);
    polls = 0;
    while (logos_mmio_read(dev, LOGOS_D5005_REG_STATUS_SEQ) == seq) {
        if (++polls <= LOGOS_CORE_MAX_POLLS) continue;
        uint64_t status = logos_mmio_read(dev, LOGOS_D5005_REG_STATUS), at = logos_mmio_read(dev, LOGOS_D5005_REG_RESULT0);
        if (logos_mmio_read(dev, LOGOS_D5005_REG_STATUS_SEQ) != seq) break;
        if ((status & 0xff) != LOGOS_D5005_STATUS_RUNNING || status >> 8 != (tag & ((1ull << 56) - 1))) return r;
        if (at == seen) {
            r.result0 = at;
            r.result1 = logos_mmio_read(dev, LOGOS_D5005_REG_RESULT1);
            return r;
        }
        seen = at;
        polls = 0;
    }
    r.status = logos_mmio_read(dev, LOGOS_D5005_REG_STATUS);
    r.result0 = logos_mmio_read(dev, LOGOS_D5005_REG_RESULT0);
    r.result1 = logos_mmio_read(dev, LOGOS_D5005_REG_RESULT1);
    return r;
}

#endif
"##;

const LOAD: &str = r##"/* Load a weight image into the D5005's memory channels and prove it is there: the directory the image writer filled
   (`channel<k>.bin`, a channel's lines; `tiles.bin`, the tile table of decode_engine::image::Image::tile_table). Each
   channel is copied in through a host buffer at the channel's full line width, then read back and compared with its
   file line for line, and where the layout carries check words each tile's CRC-32C is checked as its lines come back.
   With --self-test one line of a tile is then damaged on the card, the verify must name that tile and no other, the
   tile is loaded again from its file, and the verify must pass again. One JSON object on stdout; the exit status is 0
   only when every check held, 2 when the image or the card cannot be had.

     logos_load <image directory> [--self-test]

   Built against the OPAE backend it runs on the card; against the VPI backend, inside a simulation of the RTL. */
#include "logos_core.h"

#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifndef LOGOS_PROGRAM_MAIN
#define LOGOS_PROGRAM_MAIN main
#endif

/* Channel lines one copy moves. */
#define LOGOS_LOAD_CHUNK 4096u
#define LOGOS_MAX_BAD 64

typedef struct {
    uint64_t channel, line, lines;
} tile;

typedef struct {
    uint64_t line_bytes, check, channels, tiles;
    uint64_t *channel_lines;
    tile *tile;
} table;

typedef struct {
    uint64_t bad[LOGOS_MAX_BAD], crc[LOGOS_MAX_BAD];
    int bad_n, crc_n;
    uint64_t cycles[LOGOS_D5005_BANKS];
} verdict;

static int failures;
static uint64_t tag = 1;

static void expect(int ok, const char *what) {
    if (!ok) {
        ++failures;
        fprintf(stderr, "logos_load: %s\n", what);
    }
}

static logos_record run(logos_dev *dev, uint8_t op, uint64_t a0, uint64_t a1, uint64_t a2) {
    uint64_t t = tag++;
    logos_record r = logos_core_run(dev, op, t, a0, a1, a2);
    expect((r.status >> 8) == t && (r.status & 0xff) == LOGOS_D5005_STATUS_OK, "a copy does not complete");
    return r;
}

/* CRC-32C (Castagnoli), the register carried across calls without its inversions. */
static uint32_t crc32c_update(uint32_t c, const uint8_t *p, size_t n) {
    for (size_t i = 0; i < n; ++i) {
        c ^= p[i];
        for (int k = 0; k < 8; ++k) c = (c >> 1) ^ (0x82F63B78u & (0u - (c & 1u)));
    }
    return c;
}

static int read_words(FILE *f, uint64_t *w, size_t n) {
    uint8_t b[8];
    for (size_t i = 0; i < n; ++i) {
        if (fread(b, 1, 8, f) != 8) return -1;
        w[i] = 0;
        for (int k = 7; k >= 0; --k) w[i] = (w[i] << 8) | b[k];
    }
    return 0;
}

static int read_table(const char *dir, table *t) {
    char path[4096], magic[8];
    uint64_t head[4];
    snprintf(path, sizeof path, "%s/tiles.bin", dir);
    FILE *f = fopen(path, "rb");
    if (!f) {
        fprintf(stderr, "logos_load: no %s\n", path);
        return -1;
    }
    int ok = fread(magic, 1, 8, f) == 8 && memcmp(magic, "LOGOSIMG", 8) == 0 && read_words(f, head, 4) == 0 && head[0] == 1;
    if (ok) {
        t->line_bytes = head[1];
        t->check = head[2];
        t->channels = head[3];
        t->channel_lines = calloc(t->channels, sizeof *t->channel_lines);
        ok = t->channel_lines && read_words(f, t->channel_lines, t->channels) == 0 && read_words(f, &t->tiles, 1) == 0;
    }
    if (ok) {
        t->tile = calloc(t->tiles ? t->tiles : 1, sizeof *t->tile);
        ok = t->tile != NULL;
        for (uint64_t i = 0; ok && i < t->tiles; ++i) ok = read_words(f, &t->tile[i].channel, 3) == 0;
    }
    fclose(f);
    if (!ok) fprintf(stderr, "logos_load: %s is not a tile table\n", path);
    return ok ? 0 : -1;
}

static FILE *open_channel(const char *dir, uint64_t k) {
    char path[4096];
    snprintf(path, sizeof path, "%s/channel%" PRIu64 ".bin", dir, k);
    FILE *f = fopen(path, "rb");
    if (!f) fprintf(stderr, "logos_load: no %s\n", path);
    return f;
}

/* Host lines `lines` channel lines take packed end to end. */
static uint64_t host_lines(uint64_t lines) {
    return (lines * LOGOS_D5005_MEM_LINE_BYTES + LOGOS_D5005_HOST_LINE_BYTES - 1) / LOGOS_D5005_HOST_LINE_BYTES;
}

/* Copy channel `k`'s lines [first, first + lines) from its file onto the card; the cycles the copies took. */
static uint64_t load_lines(logos_dev *dev, uint8_t *buf, uint64_t io, FILE *f, uint64_t k, uint64_t first, uint64_t lines) {
    uint64_t cycles = 0;
    for (uint64_t at = first; at < first + lines; at += LOGOS_LOAD_CHUNK) {
        uint64_t n = first + lines - at < LOGOS_LOAD_CHUNK ? first + lines - at : LOGOS_LOAD_CHUNK;
        size_t bytes = (size_t)(n * LOGOS_D5005_MEM_LINE_BYTES);
        memset(buf, 0, (size_t)host_lines(n) * LOGOS_D5005_HOST_LINE_BYTES);
        expect(fseek(f, (long)(at * LOGOS_D5005_MEM_LINE_BYTES), SEEK_SET) == 0 && fread(buf, 1, bytes, f) == bytes,
               "a channel's file is short of its lines");
        cycles += run(dev, LOGOS_D5005_OP_HOST_TO_MEM_PACKED, io >> 6, k << 56 | at, n).result1;
    }
    return cycles;
}

static void add(uint64_t *list, int *n, uint64_t v) {
    if (*n > 0 && list[*n - 1] == v) return;
    if (*n < LOGOS_MAX_BAD) list[(*n)++] = v;
}

/* Read every channel back and hold it to its file and its tiles to their check words. */
static void verify(logos_dev *dev, const table *t, const char *dir, uint8_t *buf, uint64_t io, uint8_t *file, verdict *v) {
    memset(v, 0, sizeof *v);
    const uint64_t lb = LOGOS_D5005_MEM_LINE_BYTES;
    for (uint64_t k = 0; k < t->channels; ++k) {
        FILE *f = open_channel(dir, k);
        if (!f) {
            ++failures;
            continue;
        }
        uint64_t ti = 0, tile_end = 0;
        uint32_t crc = 0;
        int tile_bad = 0;
        while (ti < t->tiles && t->tile[ti].channel != k) ++ti;
        for (uint64_t at = 0; at < t->channel_lines[k]; at += LOGOS_LOAD_CHUNK) {
            uint64_t n = t->channel_lines[k] - at < LOGOS_LOAD_CHUNK ? t->channel_lines[k] - at : LOGOS_LOAD_CHUNK;
            v->cycles[k] += run(dev, LOGOS_D5005_OP_MEM_TO_HOST_PACKED, io >> 6, k << 56 | at, n).result1;
            expect(fseek(f, (long)(at * lb), SEEK_SET) == 0 && fread(file, 1, (size_t)(n * lb), f) == n * lb,
                   "a channel's file is short of its lines");
            for (uint64_t i = 0; i < n; ++i) {
                uint64_t line = at + i;
                const uint8_t *got = buf + i * lb;
                if (ti < t->tiles && line == t->tile[ti].line) {
                    tile_end = line + t->tile[ti].lines;
                    crc = 0xFFFFFFFFu;
                    tile_bad = 0;
                }
                if (memcmp(got, file + i * lb, (size_t)lb) != 0 && !tile_bad) {
                    tile_bad = 1;
                    add(v->bad, &v->bad_n, ti);
                }
                if (line + 1 < tile_end) {
                    crc = crc32c_update(crc, got, (size_t)lb);
                } else if (ti < t->tiles && line + 1 == tile_end) {
                    if (t->check) {
                        crc = ~crc32c_update(crc, got, (size_t)(lb - 4));
                        uint32_t word = (uint32_t)got[lb - 4] | (uint32_t)got[lb - 3] << 8 | (uint32_t)got[lb - 2] << 16 |
                                        (uint32_t)got[lb - 1] << 24;
                        if (word != crc) add(v->crc, &v->crc_n, ti);
                    }
                    do ++ti;
                    while (ti < t->tiles && t->tile[ti].channel != k);
                }
            }
        }
        fclose(f);
    }
}

static void print_list(const char *name, const uint64_t *list, int n) {
    printf("\"%s\": [", name);
    for (int i = 0; i < n; ++i) printf("%s%" PRIu64, i ? ", " : "", list[i]);
    printf("]");
}

int LOGOS_PROGRAM_MAIN(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: logos_load <image directory> [--self-test]\n");
        return 2;
    }
    const char *dir = argv[1];
    int self_test = argc > 2 && strcmp(argv[2], "--self-test") == 0;
    table t;
    if (read_table(dir, &t) != 0) return 2;
    if (t.line_bytes != LOGOS_D5005_MEM_LINE_BYTES || t.channels != LOGOS_D5005_BANKS) {
        fprintf(stderr, "logos_load: the image is %" PRIu64 " channels of %" PRIu64 "-byte lines, the accelerator %d of %d\n",
                t.channels, t.line_bytes, LOGOS_D5005_BANKS, LOGOS_D5005_MEM_LINE_BYTES);
        return 2;
    }
    logos_dev *dev = logos_open(LOGOS_D5005_AFU_ID);
    if (!dev) return 2;
    uint64_t io = 0;
    size_t bytes = (size_t)host_lines(LOGOS_LOAD_CHUNK) * LOGOS_D5005_HOST_LINE_BYTES;
    uint8_t *buf = logos_buffer(dev, bytes, &io), *file = malloc(bytes);
    if (!buf || !file) return 2;

    printf("{\"image\": \"%s\", \"line_bytes\": %" PRIu64 ", \"check\": %s, \"channels\": [", dir, t.line_bytes,
           t.check ? "true" : "false");
    uint64_t load_cycles[LOGOS_D5005_BANKS] = {0};
    for (uint64_t k = 0; k < t.channels; ++k) {
        FILE *f = open_channel(dir, k);
        if (!f) return 2;
        load_cycles[k] = load_lines(dev, buf, io, f, k, 0, t.channel_lines[k]);
        fclose(f);
    }
    verdict v;
    verify(dev, &t, dir, buf, io, file, &v);
    for (uint64_t k = 0; k < t.channels; ++k)
        printf("%s{\"lines\": %" PRIu64 ", \"load_cycles\": %" PRIu64 ", \"verify_cycles\": %" PRIu64 "}", k ? ", " : "",
               t.channel_lines[k], load_cycles[k], v.cycles[k]);
    printf("], ");
    print_list("bad_tiles", v.bad, v.bad_n);
    printf(", ");
    print_list("crc_errors", v.crc, v.crc_n);
    expect(v.bad_n == 0 && v.crc_n == 0, "the image read back is not the image");

    if (self_test && t.tiles > 0) {
        uint64_t victim = t.tiles / 2;
        const tile *x = &t.tile[victim];
        uint64_t line = x->line + x->lines / 2;
        run(dev, LOGOS_D5005_OP_MEM_TO_HOST_PACKED, io >> 6, x->channel << 56 | line, 1);
        buf[(line * 7) % LOGOS_D5005_MEM_LINE_BYTES] ^= 0x10;
        run(dev, LOGOS_D5005_OP_HOST_TO_MEM_PACKED, io >> 6, x->channel << 56 | line, 1);
        verdict damaged, reloaded;
        verify(dev, &t, dir, buf, io, file, &damaged);
        FILE *f = open_channel(dir, x->channel);
        if (!f) return 2;
        load_lines(dev, buf, io, f, x->channel, x->line, x->lines);
        fclose(f);
        verify(dev, &t, dir, buf, io, file, &reloaded);
        printf(", \"self_test\": {\"tile\": %" PRIu64 ", ", victim);
        print_list("bad_tiles", damaged.bad, damaged.bad_n);
        printf(", ");
        print_list("crc_errors", damaged.crc, damaged.crc_n);
        printf(", ");
        print_list("after_reload", reloaded.bad, reloaded.bad_n);
        printf("}");
        expect(damaged.bad_n == 1 && damaged.bad[0] == victim, "the damaged tile is not the one the verify names");
        expect(!t.check || (damaged.crc_n == 1 && damaged.crc[0] == victim), "the damaged tile's check word does not catch it");
        expect(reloaded.bad_n == 0 && reloaded.crc_n == 0, "the tile loaded again does not verify");
    }
    printf(", \"checks_failed\": %d}\n", failures);
    fflush(stdout);
    logos_buffer_free(dev, buf);
    free(file);
    logos_close(dev);
    return failures ? 1 : 0;
}
"##;

const STEP: &str = r##"/* Run a model on the decode engine: the program `ofs::write_engine_program` wrote — the memory image, the engine's
   configuration and tables, the embedding table — loaded onto the card, then greedy decoding one token a step from a
   prompt, for each of up to the program's sequences at once, their prompts apart by `--`: each step every sequence's
   row of the row table (its region, its position, its ring row) and its token's embedding written into the engine,
   the step run over every row, and each sequence once past its prompt reading its own row's logits back, their
   arg-max its next token; a greedy program's engine takes the arg-max itself, and the row's read-back is that one beat.
   The sequences step together, so each is at the same position; one whose prompt and tokens are done runs on with the
   rest. Each step's position, first sequence's token and cycles on stderr as it completes; one JSON object on stdout
   once decoding ends: the first sequence's tokens, every sequence's, each step's cycles and the engine's and card
   port's faults. The exit status is 0 only when every step ran without one, 2 when the program or the card cannot be
   had, a program for more rows or a larger store than the build's among them.

     logos_step [--inject] <program directory> <tokens to generate> <prompt token>... [-- <prompt token>...]...

   With --inject, on SECDED lines, every line the image load writes carries one flipped bit, which every read of it
   corrects; the record then carries each channel's words corrected and lost. With LOGOS_PROFILE naming a job list
   (`d5005_afu --trace-store`'s jobs.txt, a "job <j> word <w> <kind>" line a job), the prompt's last position runs a job
   at a time, each job from its word to the next job's alone, its cycles on stdout as "<j> <kind> <cycles>" before the
   JSON; the step's record is its last job's. Built against the OPAE backend it runs on the card; against a simulation
   backend, inside the RTL. */
#include "logos_core.h"

#include <inttypes.h>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#ifndef LOGOS_PROGRAM_MAIN
#define LOGOS_PROGRAM_MAIN main
#endif

/* Host lines one copy moves. */
#define LOGOS_STEP_CHUNK 4096u
/* The most sequences one run decodes. */
#define LOGOS_STEP_SEQUENCES 64
/* The sequencer contexts of the build: sequence s runs in context s mod the contexts, as its row s / the contexts. */
#ifndef LOGOS_D5005_CONTEXTS
#define LOGOS_D5005_CONTEXTS 1
#endif
/* Whether the build's f32 products are the card's floating-point DSP blocks'. */
#ifndef LOGOS_D5005_FP_HARD
#define LOGOS_D5005_FP_HARD 0
#endif

typedef struct {
    uint64_t knobs, space, base, granule_log2, job_words, residual_beat, residual_beats, logits_beat, logits_beats;
    uint64_t tokens, hidden, vocab, ring_rows, regions, region, sequences, ctx, store_beats, greedy, contexts, fp_hard;
    uint64_t channel_lines[LOGOS_D5005_BANKS];
    char streaming[256];
} program;

static int failures;
static uint64_t tag = 1;

static void expect(int ok, const char *what) {
    if (!ok) {
        ++failures;
        fprintf(stderr, "logos_step: %s\n", what);
    }
}

static logos_record run(logos_dev *dev, uint8_t op, uint64_t a0, uint64_t a1, uint64_t a2) {
    uint64_t t = tag++;
    logos_record r = logos_core_run(dev, op, t, a0, a1, a2);
    expect((r.status >> 8) == t && (r.status & 0xff) == LOGOS_D5005_STATUS_OK, "a command does not complete");
    return r;
}

static int read_program(const char *dir, program *p) {
    char path[4096], key[64];
    snprintf(path, sizeof path, "%s/engine.txt", dir);
    FILE *f = fopen(path, "r");
    if (!f) return -1;
    memset(p, 0, sizeof *p);
    int fields = 0;
    while (fscanf(f, "%63s", key) == 1) {
        uint64_t *one = NULL, *two = NULL;
        if (!strcmp(key, "knobs")) one = &p->knobs;
        else if (!strcmp(key, "space")) one = &p->space;
        else if (!strcmp(key, "base")) one = &p->base;
        else if (!strcmp(key, "granule_log2")) one = &p->granule_log2;
        else if (!strcmp(key, "job_words")) one = &p->job_words;
        else if (!strcmp(key, "residual")) { one = &p->residual_beat; two = &p->residual_beats; }
        else if (!strcmp(key, "logits")) { one = &p->logits_beat; two = &p->logits_beats; }
        else if (!strcmp(key, "tokens")) one = &p->tokens;
        else if (!strcmp(key, "hidden")) one = &p->hidden;
        else if (!strcmp(key, "vocab")) one = &p->vocab;
        else if (!strcmp(key, "ring_rows")) one = &p->ring_rows;
        else if (!strcmp(key, "regions")) one = &p->regions;
        else if (!strcmp(key, "region")) one = &p->region;
        else if (!strcmp(key, "sequences")) one = &p->sequences;
        else if (!strcmp(key, "ctx")) one = &p->ctx;
        else if (!strcmp(key, "store")) one = &p->store_beats;
        else if (!strcmp(key, "greedy")) {
            if (fscanf(f, "%" SCNu64, &p->greedy) != 1 || p->greedy != 1) { fclose(f); return -1; }
            continue;
        } else if (!strcmp(key, "fp")) {
            char unit[8];
            if (fscanf(f, "%7s", unit) != 1 || strcmp(unit, "hard")) { fclose(f); return -1; }
            p->fp_hard = 1;
            continue;
        } else if (!strcmp(key, "contexts")) {
            if (fscanf(f, "%" SCNu64, &p->contexts) != 1 || p->contexts < 2) { fclose(f); return -1; }
            continue;
        } else if (!strcmp(key, "channels")) {
            for (int k = 0; k < LOGOS_D5005_BANKS; ++k)
                if (fscanf(f, "%" SCNu64, &p->channel_lines[k]) != 1) { fclose(f); return -1; }
            ++fields;
            continue;
        } else if (!strcmp(key, "streaming")) {
            if (fscanf(f, "%255s", p->streaming) != 1) { fclose(f); return -1; }
            continue;
        } else { fclose(f); return -1; }
        if (fscanf(f, "%" SCNu64, one) != 1 || (two && fscanf(f, "%" SCNu64, two) != 1)) { fclose(f); return -1; }
        ++fields;
    }
    fclose(f);
    if (!p->contexts) p->contexts = 1;
    return fields == 17 && p->ring_rows > 0 && p->hidden <= p->residual_beats * 16 && p->logits_beats >= 1 &&
                   (p->greedy || p->vocab <= p->logits_beats * 16)
               ? 0
               : -1;
}

/* A file's bytes, its length in `n`. */
static uint8_t *slurp(const char *dir, const char *name, size_t *n) {
    char path[4096];
    snprintf(path, sizeof path, "%s/%s", dir, name);
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long len = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *b = malloc(len > 0 ? (size_t)len : 1);
    *n = len > 0 && b && fread(b, 1, (size_t)len, f) == (size_t)len ? (size_t)len : 0;
    fclose(f);
    if (len > 0 && *n == 0) { free(b); return NULL; }
    return b;
}

/* A file mapped read-only, its length in `n`: the memory image and the embeddings are larger than a host's memory may
   be, so they are read as the card takes them. */
static const uint8_t *map_file(const char *dir, const char *name, size_t *n) {
    char path[4096];
    snprintf(path, sizeof path, "%s/%s", dir, name);
    *n = 0;
    int fd = open(path, O_RDONLY);
    if (fd < 0) return NULL;
    struct stat st;
    if (fstat(fd, &st) || st.st_size <= 0) {
        close(fd);
        return NULL;
    }
    void *m = mmap(NULL, (size_t)st.st_size, PROT_READ, MAP_PRIVATE, fd, 0);
    close(fd);
    if (m == MAP_FAILED) return NULL;
    *n = (size_t)st.st_size;
    return m;
}

/* `lines` host lines of `bytes` into the engine's table `table` from entry `first`, through the host buffer. */
static void table_write(logos_dev *dev, uint8_t *buf, uint64_t io, int table, uint64_t first, const uint8_t *bytes, uint64_t lines) {
    for (uint64_t at = 0; at < lines; at += LOGOS_STEP_CHUNK) {
        uint64_t n = lines - at < LOGOS_STEP_CHUNK ? lines - at : LOGOS_STEP_CHUNK;
        memcpy(buf, bytes + at * 64, (size_t)n * 64);
        run(dev, LOGOS_D5005_OP_ENGINE_TABLE, io >> 6, (uint64_t)table << 32 | (first + at), n);
    }
}

/* The prompt's last position run a job at a time from the job list at `path`: each job from its word to the next's
   runs alone and its cycles go to stdout. */
static logos_record profiled(logos_dev *dev, const char *path, uint64_t job_words) {
    FILE *f = fopen(path, "r");
    if (!f) {
        fprintf(stderr, "logos_step: no job list %s\n", path);
        exit(2);
    }
    static uint64_t word[4096];
    static char kind[4096][24];
    uint64_t n = 0, j, w;
    char line[65536], k[24];
    while (fgets(line, sizeof line, f) && n < 4096)
        if (sscanf(line, "job %" SCNu64 " word %" SCNu64 " %23[A-Za-z]", &j, &w, k) == 3) {
            word[n] = w;
            strcpy(kind[n], k);
            ++n;
        }
    fclose(f);
    logos_record r = {0, 0, 0};
    for (uint64_t i = 1; i < n; ++i) {
        uint64_t to = i + 1 < n ? word[i + 1] : job_words;
        if (to <= word[i]) continue;
        r = run(dev, LOGOS_D5005_OP_ENGINE_RUN, word[i], to, 0);
        if (r.result0 & 3) fprintf(stderr, "logos_step: a fault at job %" PRIu64 "\n", i);
        printf("%" PRIu64 " %s %" PRIu64 "\n", i, kind[i], r.result1);
    }
    fflush(stdout);
    return r;
}

/* The first strictly largest logit that is a number, as numerics::argmax picks it. */
static uint64_t argmax(const float *x, uint64_t n) {
    uint64_t best = 0;
    int found = 0;
    for (uint64_t i = 0; i < n; ++i)
        if (!isnan(x[i]) && (!found || x[i] > x[best])) { best = i; found = 1; }
    return best;
}

int LOGOS_PROGRAM_MAIN(int argc, char **argv) {
    int inject = argc > 1 && !strcmp(argv[1], "--inject");
    if (inject) {
        --argc;
        ++argv;
    }
    if (argc < 4 || (inject && !LOGOS_D5005_SECDED)) {
        fprintf(stderr, "usage: logos_step [--inject] <program directory> <tokens to generate> <prompt token>... [-- <prompt token>...]..., --inject on SECDED lines\n");
        return 2;
    }
    const char *dir = argv[1];
    uint64_t generate = strtoull(argv[2], NULL, 0);
    /* The sequences' prompts: each from argv[first[s]], prompt_n[s] tokens. */
    uint64_t seqs = 0, first[LOGOS_STEP_SEQUENCES], prompt_n[LOGOS_STEP_SEQUENCES];
    for (int a = 3; a <= argc; ++a) {
        if (a == argc || !strcmp(argv[a], "--")) {
            uint64_t start = seqs ? first[seqs - 1] + prompt_n[seqs - 1] + 1 : 3;
            if (seqs == LOGOS_STEP_SEQUENCES || a == (int)start) {
                fprintf(stderr, "logos_step: every sequence needs a prompt, at most %d sequences\n", LOGOS_STEP_SEQUENCES);
                return 2;
            }
            first[seqs] = start;
            prompt_n[seqs] = (uint64_t)a - start;
            ++seqs;
        }
    }
    program p;
    if (read_program(dir, &p)) {
        fprintf(stderr, "logos_step: %s/engine.txt is no engine program\n", dir);
        return 2;
    }
    if (strcmp(p.streaming, LOGOS_D5005_STREAMING)) {
        fprintf(stderr, "logos_step: %s is a program for the streaming unit \"%s\", this build's is \"%s\"\n", dir, p.streaming,
                LOGOS_D5005_STREAMING);
        return 2;
    }
    uint64_t longest = 0;
    for (uint64_t s = 0; s < seqs; ++s) longest = prompt_n[s] > longest ? prompt_n[s] : longest;
    if (longest + generate > p.ctx) {
        fprintf(stderr, "logos_step: %" PRIu64 " tokens past the %" PRIu64 " positions a sequence reaches\n", longest + generate, p.ctx);
        return 2;
    }
    if (seqs > p.sequences) {
        fprintf(stderr, "logos_step: %" PRIu64 " sequences past the program's %" PRIu64 "\n", seqs, p.sequences);
        return 2;
    }
    if (p.contexts != LOGOS_D5005_CONTEXTS) {
        fprintf(stderr, "logos_step: %s is a program of %" PRIu64 " contexts, this build runs %u\n", dir, p.contexts,
                (unsigned)LOGOS_D5005_CONTEXTS);
        return 2;
    }
    if (p.fp_hard != LOGOS_D5005_FP_HARD) {
        fprintf(stderr, "logos_step: %s is a program for %s products, this build's are %s\n", dir,
                p.fp_hard ? "the card's floating-point blocks'" : "the engine's own", LOGOS_D5005_FP_HARD ? "the card's floating-point blocks'" : "the engine's own");
        return 2;
    }
    const uint64_t C = LOGOS_D5005_CONTEXTS;
    if ((p.sequences + C - 1) / C > LOGOS_D5005_ROWS || p.store_beats > LOGOS_D5005_STORE_BEATS) {
        fprintf(stderr, "logos_step: %s steps %" PRIu64 " rows over a store of %" PRIu64 " beats, past this build's %u rows and %u beats\n",
                dir, (p.sequences + C - 1) / C, p.store_beats, LOGOS_D5005_ROWS, LOGOS_D5005_STORE_BEATS);
        return 2;
    }
    size_t embed_n = 0;
    const uint8_t *embed = map_file(dir, "embed.bin", &embed_n);
    if (!embed || embed_n != p.tokens * p.hidden * 4) {
        fprintf(stderr, "logos_step: embed.bin is not %" PRIu64 " embeddings of %" PRIu64 " elements\n", p.tokens, p.hidden);
        return 2;
    }
    logos_dev *dev = logos_open(LOGOS_D5005_AFU_ID);
    if (!dev) return 2;
    uint64_t io = 0, buf_lines = LOGOS_STEP_CHUNK > p.logits_beats ? LOGOS_STEP_CHUNK : p.logits_beats;
    uint8_t *buf = logos_buffer(dev, (size_t)buf_lines * 64, &io);
    if (!buf) return 2;

    /* Context c's rows, the sequences s with s mod C = c: context 0's at arg0[15:0], context c's at arg0[32 + 16(c - 1) +: 16]. */
    uint64_t config = p.knobs << 16;
    for (uint64_t c = 0; c < C; ++c) config |= ((seqs + C - 1 - c) / C) << (c ? 32 + 16 * (c - 1) : 0);
    run(dev, LOGOS_D5005_OP_ENGINE_CONFIG, config, p.space, p.base | p.granule_log2 << 56);
    static const struct { int table; const char *file; } tables[] = {
        {0, "table0.bin"}, {2, "table2.bin"}, {3, "table3.bin"}, {4, "table4.bin"}, {5, "table5.bin"}};
    for (size_t k = 0; k < sizeof tables / sizeof tables[0]; ++k) {
        size_t n = 0;
        uint8_t *bytes = slurp(dir, tables[k].file, &n);
        if (!bytes || n % 64) {
            fprintf(stderr, "logos_step: %s is not whole 64-byte entries\n", tables[k].file);
            return 2;
        }
        table_write(dev, buf, io, tables[k].table, 0, bytes, n / 64);
        free(bytes);
    }
    if (inject)
        for (int k = 0; k < LOGOS_D5005_BANKS; ++k)
            run(dev, LOGOS_D5005_OP_INJECT, (uint64_t)1 << k, 5u << 16 | 5u, p.channel_lines[k]);
    for (int k = 0; k < LOGOS_D5005_BANKS; ++k) {
        char name[32];
        size_t n = 0;
        snprintf(name, sizeof name, "channel%d.bin", k);
        const uint8_t *bytes = p.channel_lines[k] ? map_file(dir, name, &n) : NULL;
        if (n != p.channel_lines[k] * LOGOS_D5005_MEM_LINE_BYTES || (n && !bytes)) {
            fprintf(stderr, "logos_step: %s is not %" PRIu64 " lines\n", name, p.channel_lines[k]);
            return 2;
        }
        for (uint64_t at = 0; at < p.channel_lines[k]; at += LOGOS_STEP_CHUNK) {
            uint64_t lines = p.channel_lines[k] - at < LOGOS_STEP_CHUNK ? p.channel_lines[k] - at : LOGOS_STEP_CHUNK;
            memcpy(buf, bytes + at * LOGOS_D5005_MEM_LINE_BYTES, (size_t)lines * LOGOS_D5005_MEM_LINE_BYTES);
            run(dev, LOGOS_D5005_OP_HOST_TO_MEM, io >> 6, (uint64_t)k << 56 | at, lines);
        }
        if (bytes) munmap((void *)bytes, n);
    }

    uint64_t total = longest + generate, ran = 0;
    uint64_t *cycles = calloc((size_t)total, sizeof *cycles), *tokens = calloc((size_t)(seqs * total), sizeof *tokens);
    uint64_t token[LOGOS_STEP_SEQUENCES];
    for (uint64_t s = 0; s < seqs; ++s) token[s] = strtoull(argv[first[s]], NULL, 0);
    uint8_t *rows = calloc((size_t)seqs, 64), *e = calloc((size_t)p.residual_beats, 64);
    int faults = 0;
    for (uint64_t pos = 0; pos < total; ++pos) {
        for (uint64_t s = 0; s < seqs; ++s) {
            if (token[s] >= p.tokens) {
                fprintf(stderr, "logos_step: token %" PRIu64 " past the embedding table\n", token[s]);
                return 2;
            }
            tokens[s * total + pos] = token[s];
            /* Each context's rows in its own run of the buffer: sequence s at row s / C of context s mod C. */
            uint64_t region = p.regions + s * p.region, ring = pos % p.ring_rows, at = (s % C) * ((seqs + C - 1) / C) + s / C;
            memcpy(rows + 64 * at, &region, 4);
            memcpy(rows + 64 * at + 4, &pos, 4);
            rows[64 * at + 8] = (uint8_t)ring;
        }
        ran = pos + 1;
        for (uint64_t c = 0; c < C; ++c)
            if ((seqs + C - 1 - c) / C)
                table_write(dev, buf, io, 1, c << 56, rows + 64 * c * ((seqs + C - 1) / C), (seqs + C - 1 - c) / C);
        for (uint64_t s = 0; s < seqs; ++s) {
            memset(e, 0, (size_t)p.residual_beats * 64);
            memcpy(e, embed + token[s] * p.hidden * 4, (size_t)p.hidden * 4);
            table_write(dev, buf, io, 6, (s % C) << 56 | (p.residual_beat + s / C * p.residual_beats), e, p.residual_beats);
        }
        const char *profile = getenv("LOGOS_PROFILE");
        logos_record step = profile && pos + 1 == longest ? profiled(dev, profile, p.job_words)
                                                          : run(dev, LOGOS_D5005_OP_ENGINE_RUN, 0, p.job_words, 0);
        cycles[pos] = step.result1;
        fprintf(stderr, "logos_step: position %" PRIu64 " of %" PRIu64 ", token %" PRIu64 ": %" PRIu64 " cycles\n", pos, total,
                token[0], step.result1);
        if ((step.status & 0xff) == 0xff) {
            ++faults;
            if (C == 1)
                fprintf(stderr, "logos_step: position %" PRIu64 ": the engine stopped moving at word %" PRIu64 "\n", pos, step.result0 >> 32);
            else
                for (uint64_t c = 0; c < C; ++c)
                    fprintf(stderr, "logos_step: position %" PRIu64 ": the engine stopped moving; context %" PRIu64 " at word %" PRIu64 "\n", pos, c,
                            step.result0 >> (32 + 16 * c) & 0xffff);
            break;
        }
        if (step.result0 & 3) {
            ++faults;
            if (C == 1)
                fprintf(stderr, "logos_step: position %" PRIu64 ": the %s faulted at word %" PRIu64 "\n", pos,
                        step.result0 & 1 ? "engine" : "card port", step.result0 >> 32);
            else
                for (uint64_t c = 0; c < C; ++c)
                    fprintf(stderr, "logos_step: position %" PRIu64 ": the %s faulted; context %" PRIu64 " reached word %" PRIu64 "\n", pos,
                            step.result0 & 1 ? "engine" : "card port", c, step.result0 >> (32 + 16 * c) & 0xffff);
            break;
        }
        for (uint64_t s = 0; s < seqs; ++s) {
            if (pos + 1 < prompt_n[s]) {
                token[s] = strtoull(argv[first[s] + pos + 1], NULL, 0);
            } else if (pos + 1 < total) {
                run(dev, LOGOS_D5005_OP_STORE_READ, io >> 6, (s % C) << 56 | (p.logits_beat + s / C * p.logits_beats), p.greedy ? 1 : p.logits_beats);
                token[s] = p.greedy ? ((const uint32_t *)buf)[0] : argmax((const float *)buf, p.vocab);
            }
        }
    }
    printf("{\"tokens\": [");
    for (uint64_t pos = 0; pos < ran && pos < prompt_n[0] + generate; ++pos) printf("%s%" PRIu64, pos ? ", " : "", tokens[pos]);
    printf("], \"sequences\": [");
    for (uint64_t s = 0; s < seqs; ++s) {
        printf("%s[", s ? ", " : "");
        for (uint64_t pos = 0; pos < ran && pos < prompt_n[s] + generate; ++pos) printf("%s%" PRIu64, pos ? ", " : "", tokens[s * total + pos]);
        printf("]");
    }
    printf("], \"prompt\": %" PRIu64 ", \"cycles\": [", prompt_n[0]);
    for (uint64_t pos = 0; pos < total; ++pos) printf("%s%" PRIu64, pos ? ", " : "", cycles[pos]);
    printf("], \"faults\": %d", faults);
    if (inject) {
        printf(", \"secded\": [");
        for (int k = 0; k < LOGOS_D5005_BANKS; ++k) {
            logos_record c = run(dev, LOGOS_D5005_OP_ECC_COUNTS, (uint64_t)k, 0, 0);
            printf("%s{\"corrected\": %" PRIu64 ", \"uncorrectable\": %" PRIu64 "}", k ? ", " : "", c.result0 & 0xffffffffu, c.result0 >> 32);
        }
        printf("]");
    }
    printf(", \"checks_failed\": %d}\n", failures);
    fflush(stdout);
    free(cycles);
    free(tokens);
    free(rows);
    free(e);
    munmap((void *)embed, embed_n);
    logos_buffer_free(dev, buf);
    logos_close(dev);
    return failures || faults ? 1 : 0;
}
"##;

const RECEIPT: &str = r##"/* The bring-up receipt of the D5005 accelerator: the accelerator identifies itself, every memory channel is filled and
   checked alone and all four at once (each at the core's cycle count, so its bandwidth), a flipped seed is seen, and a
   buffer of host memory makes the round trip through a channel unchanged. On SECDED lines every channel then proves its
   code: no word corrected or lost before a flip is injected, a flipped data bit and a flipped check bit corrected on
   every line, two flips in one word found uncorrectable on every line, a flip in each of two words both corrected, and
   a clean fill clean again; without SECDED the core refuses the code's two ops, and without the engine the engine's
   four. The receipt is one JSON object on stdout; the exit status is 0 only when every check held.

     logos_receipt [lines per channel] [host lines]

   Built against the OPAE backend it runs on the card; against the VPI backend, inside a simulation of the RTL. */
#include "logos_core.h"

#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifndef LOGOS_PROGRAM_MAIN
#define LOGOS_PROGRAM_MAIN main
#endif

typedef logos_record record;

static int failures;

static void expect(int ok, const char *what) {
    if (!ok) {
        ++failures;
        fprintf(stderr, "logos_receipt: %s\n", what);
    }
}

/* Ring `op` with its arguments and wait for its status record. */
static record run(logos_dev *dev, uint8_t op, uint64_t tag, uint64_t a0, uint64_t a1, uint64_t a2) {
    record r = logos_core_run(dev, op, tag, a0, a1, a2);
    expect((r.status >> 8) == tag, "a status record of another command");
    return r;
}

static double gbps(uint64_t lines, uint64_t cycles, double hz) {
    return cycles ? (double)lines * LOGOS_D5005_MEM_LINE_BYTES * hz / (double)cycles / 1e9 : 0.0;
}

#if LOGOS_D5005_SECDED
/* Channel `b`'s words corrected and words found uncorrectable since reset. */
static void secded_counts(logos_dev *dev, uint64_t *tag, int b, uint64_t *fixed, uint64_t *lost) {
    record r = run(dev, LOGOS_D5005_OP_ECC_COUNTS, (*tag)++, (uint64_t)b, 0, 0);
    expect((r.status & 0xff) == LOGOS_D5005_STATUS_OK, "the SECDED counts cannot be read");
    *fixed = r.result0 & UINT64_C(0xffffffff);
    *lost = r.result0 >> 32;
}

/* The code on channel `b`: `n` lines filled with bus bits `a` and `c` flipped (none when `a` is negative), checked,
   and the lines that differ and the words corrected and lost since the last case held to what the case expects of
   every line. */
static void secded_case(logos_dev *dev, uint64_t *tag, int b, uint64_t n, const char *name, int a, int c, uint64_t wrong,
                        uint64_t fixed, uint64_t lost, uint64_t *fixed_at, uint64_t *lost_at) {
    uint64_t size = ((uint64_t)0x5ec << 32) | n, f, l;
    if (a >= 0) {
        record inject = run(dev, LOGOS_D5005_OP_INJECT, (*tag)++, (uint64_t)1 << b, (uint64_t)c << 16 | (uint64_t)a, n);
        expect((inject.status & 0xff) == LOGOS_D5005_STATUS_OK, "a flip cannot be injected");
    }
    record fill = run(dev, LOGOS_D5005_OP_FILL, (*tag)++, (uint64_t)1 << b, 0, size);
    record check = run(dev, LOGOS_D5005_OP_CHECK, (*tag)++, (uint64_t)1 << b, 0, size);
    expect((fill.status & 0xff) == LOGOS_D5005_STATUS_OK && (check.status & 0xff) == LOGOS_D5005_STATUS_OK, "a SECDED fill or check fails");
    secded_counts(dev, tag, b, &f, &l);
    char what[96];
    snprintf(what, sizeof what, "channel %d, %s: the lines that differ", b, name);
    expect(check.result0 == wrong * n, what);
    snprintf(what, sizeof what, "channel %d, %s: the words corrected and lost", b, name);
    expect(f - *fixed_at == fixed * n && l - *lost_at == lost * n, what);
    printf(", \"%s\": {\"mismatches\": %" PRIu64 ", \"corrected\": %" PRIu64 ", \"uncorrectable\": %" PRIu64 "}", name,
           check.result0, f - *fixed_at, l - *lost_at);
    *fixed_at = f;
    *lost_at = l;
}
#endif

int LOGOS_PROGRAM_MAIN(int argc, char **argv) {
    uint64_t lines = argc > 1 ? strtoull(argv[1], NULL, 0) : 1u << 20;
    uint64_t host_lines = argc > 2 ? strtoull(argv[2], NULL, 0) : 1u << 12;
    uint64_t tag = 1;
    logos_dev *dev = logos_open(LOGOS_D5005_AFU_ID);
    if (!dev) return 2;
    double hz = logos_user_clock_hz(dev);
    uint64_t dfh = logos_mmio_read(dev, LOGOS_D5005_REG_DFH);
    uint64_t id_l = logos_mmio_read(dev, LOGOS_D5005_REG_ID_L), id_h = logos_mmio_read(dev, LOGOS_D5005_REG_ID_H);
    char id[40];
    snprintf(id, sizeof id, "%08" PRIx64 "-%04" PRIx64 "-%04" PRIx64 "-%04" PRIx64 "-%012" PRIx64, id_h >> 32,
             (id_h >> 16) & 0xffff, id_h & 0xffff, id_l >> 48, id_l & UINT64_C(0xffffffffffff));
    expect((dfh >> 60) == 1 && ((dfh >> 40) & 1), "the feature header is no AFU's at the end of the list");
    expect(strcmp(id, LOGOS_D5005_AFU_ID) == 0, "the AFU ID is not the one opened");
    logos_mmio_write(dev, LOGOS_D5005_REG_SCRATCH, 0x5a5aa5a5f00dcafeull);
    expect(logos_mmio_read(dev, LOGOS_D5005_REG_SCRATCH) == 0x5a5aa5a5f00dcafeull, "the scratch register does not hold");
    record nop = run(dev, LOGOS_D5005_OP_NOP, tag++, 0, 0, 0);
    expect((nop.status & 0xff) == LOGOS_D5005_STATUS_OK, "a NOP does not complete");

    printf("{\"afu_id\": \"%s\", \"build\": %" PRIu64 ", \"user_clock_hz\": %.0f, \"line_bytes\": %d, \"channels\": [",
           id, logos_mmio_read(dev, LOGOS_D5005_REG_BUILD_ID), hz, LOGOS_D5005_MEM_LINE_BYTES);
    for (int b = 0; b <= LOGOS_D5005_BANKS; ++b) {
        uint64_t mask = b < LOGOS_D5005_BANKS ? 1u << b : (1u << LOGOS_D5005_BANKS) - 1;
        uint64_t seed = 0x1000 + (uint64_t)b, size = (seed << 32) | lines;
        record fill = run(dev, LOGOS_D5005_OP_FILL, tag++, mask, 0, size);
        record check = run(dev, LOGOS_D5005_OP_CHECK, tag++, mask, 0, size);
        record wrong = run(dev, LOGOS_D5005_OP_CHECK, tag++, mask, 0, ((seed + 1) << 32) | lines);
        int channels = b < LOGOS_D5005_BANKS ? 1 : LOGOS_D5005_BANKS;
        expect((fill.status & 0xff) == LOGOS_D5005_STATUS_OK && (check.status & 0xff) == LOGOS_D5005_STATUS_OK, "a fill or check fails");
        expect(check.result0 == 0, "lines read back differ from those written");
        expect(wrong.result0 == lines * (uint64_t)channels, "a check under another seed does not differ on every line");
        printf("%s{\"mask\": %" PRIu64 ", \"lines\": %" PRIu64 ", \"fill_cycles\": %" PRIu64 ", \"check_cycles\": %" PRIu64
               ", \"mismatches\": %" PRIu64 ", \"write_gb_per_s\": %.3f, \"read_gb_per_s\": %.3f}",
               b ? ", " : "", mask, lines, fill.result1, check.result1, check.result0,
               gbps(lines * (uint64_t)channels, fill.result1, hz), gbps(lines * (uint64_t)channels, check.result1, hz));
    }
    printf("]");

#if LOGOS_D5005_SECDED
    printf(", \"secded\": [");
    for (int b = 0; b < LOGOS_D5005_BANKS; ++b) {
        uint64_t n = lines < 64 ? lines : 64, fixed, lost;
        int data = 64 * 3 + 5, other = 64 * 3 + 58, check_bit = (int)LOGOS_D5005_SECDED_CHECK_BIT(3, 3), far = 64 * 6 + 1;
        secded_counts(dev, &tag, b, &fixed, &lost);
        expect(fixed == 0 && lost == 0, "a channel corrected or lost words before a flip was injected");
        printf("%s{\"channel\": %d, \"lines\": %" PRIu64 ", \"before\": {\"corrected\": %" PRIu64 ", \"uncorrectable\": %" PRIu64 "}",
               b ? ", " : "", b, n, fixed, lost);
        secded_case(dev, &tag, b, n, "data_bit", data, data, 0, 1, 0, &fixed, &lost);
        secded_case(dev, &tag, b, n, "check_bit", check_bit, check_bit, 0, 1, 0, &fixed, &lost);
        secded_case(dev, &tag, b, n, "two_bits_one_word", data, other, 1, 0, 1, &fixed, &lost);
        secded_case(dev, &tag, b, n, "two_words", data, far, 0, 2, 0, &fixed, &lost);
        secded_case(dev, &tag, b, n, "clean", -1, -1, 0, 0, 0, &fixed, &lost);
        printf("}");
    }
    printf("]");
#else
    record inject = run(dev, LOGOS_D5005_OP_INJECT, tag++, 0, 0, 0);
    record counts = run(dev, LOGOS_D5005_OP_ECC_COUNTS, tag++, 0, 0, 0);
    expect((inject.status & 0xff) == LOGOS_D5005_STATUS_UNKNOWN_OP && (counts.status & 0xff) == LOGOS_D5005_STATUS_UNKNOWN_OP,
           "a core without SECDED lines runs the code's ops");
#endif
#if !LOGOS_D5005_ENGINE
    {
        static const uint8_t engine_ops[] = {LOGOS_D5005_OP_ENGINE_CONFIG, LOGOS_D5005_OP_ENGINE_TABLE, LOGOS_D5005_OP_ENGINE_RUN,
                                             LOGOS_D5005_OP_STORE_READ};
        for (size_t k = 0; k < sizeof engine_ops; ++k)
            expect((run(dev, engine_ops[k], tag++, 0, 0, 0).status & 0xff) == LOGOS_D5005_STATUS_UNKNOWN_OP,
                   "a core without the engine runs the engine's ops");
    }
#endif

    uint64_t io_in = 0, io_out = 0;
    size_t bytes = (size_t)host_lines * LOGOS_D5005_HOST_LINE_BYTES;
    uint8_t *in = logos_buffer(dev, bytes, &io_in), *out = logos_buffer(dev, bytes, &io_out);
    if (!in || !out) return 2;
    for (size_t i = 0; i < bytes; ++i) in[i] = (uint8_t)(i * 2654435761u >> 13);
    memset(out, 0, bytes);
    uint64_t at = (uint64_t)1 << 56 | (lines & ((1u << 27) - 1));
    record to_mem = run(dev, LOGOS_D5005_OP_HOST_TO_MEM, tag++, io_in >> 6, at, host_lines);
    record to_host = run(dev, LOGOS_D5005_OP_MEM_TO_HOST, tag++, io_out >> 6, at, host_lines);
    int same = memcmp(in, out, bytes) == 0;
    expect((to_mem.status & 0xff) == LOGOS_D5005_STATUS_OK && (to_host.status & 0xff) == LOGOS_D5005_STATUS_OK, "a host copy fails");
    expect(same, "host lines do not come back unchanged through a channel");
    printf(", \"host\": {\"lines\": %" PRIu64 ", \"to_mem_cycles\": %" PRIu64 ", \"to_host_cycles\": %" PRIu64
           ", \"round_trip\": %s}", host_lines, to_mem.result1, to_host.result1, same ? "true" : "false");
    record unknown = run(dev, 0x7f, tag++, 0, 0, 0);
    expect((unknown.status & 0xff) == LOGOS_D5005_STATUS_UNKNOWN_OP, "an unknown op is not refused");
    expect(logos_mmio_read(dev, LOGOS_D5005_REG_ERRORS) == 0, "the accelerator reports an error");
    printf(", \"checks_failed\": %d}\n", failures);
    fflush(stdout);
    logos_buffer_free(dev, in);
    logos_buffer_free(dev, out);
    logos_close(dev);
    return failures ? 1 : 0;
}
"##;

const HAL_SIM: &str = r##"/* The D5005 accelerator's RTL as a logos_dev, the part every simulator shares: the host program runs in a thread of
   its own, each of its MMIO accesses carried out on the shell's MMIO port at the falling edges of pClk (`sim_mmio`),
   and the simulation's host memory the program's own buffers at the I/O addresses `logos_buffer` gave them
   (`host_bytes`). The simulation advances only while the program waits on an MMIO access, so a run's every cycle
   count is the same on every machine. A backend includes this once and calls `sim_start`, `sim_mmio` and
   `host_bytes` from its simulator's hooks: logos_hal_vpi.c for Icarus Verilog, logos_hal_dpi.c for Verilator. */
#ifndef LOGOS_HAL_SIM_H
#define LOGOS_HAL_SIM_H

#include "logos_hal.h"

#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define LOGOS_MAX_BUFFERS 64
#define LOGOS_IO_BASE (1ull << 32)
#define LOGOS_IO_ALIGN (1ull << 21)

int logos_program_main(int argc, char **argv);

enum { IDLE, WANT_READ, WANT_WRITE, READ_ISSUED, AWAIT_DATA, WRITE_ISSUED, DONE };

static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t wake = PTHREAD_COND_INITIALIZER;
static int state = IDLE, finished, exit_status, started;
static uint32_t req_word;
static uint64_t req_value;
static double user_clock_hz = 450e6;

static int program_argc;
static char **program_argv;

static struct { uint64_t io; size_t bytes; uint8_t *ptr; } buffers[LOGOS_MAX_BUFFERS];
static uint64_t next_io = LOGOS_IO_BASE;

struct logos_dev { int open; };
static struct logos_dev the_device;

/* What the MMIO port carries until the next falling edge. */
static struct { uint32_t address; int read, write; uint64_t wdata; } bus;

logos_dev *logos_open(const char *afu_id) {
    (void)afu_id;
    the_device.open = 1;
    return &the_device;
}

void logos_close(logos_dev *dev) { dev->open = 0; }

static uint64_t bus_access(int want, uint32_t byte_offset, uint64_t value) {
    pthread_mutex_lock(&lock);
    req_word = byte_offset / 8;
    req_value = value;
    state = want;
    pthread_cond_broadcast(&wake);
    while (state != DONE) pthread_cond_wait(&wake, &lock);
    value = req_value;
    state = IDLE;
    pthread_cond_broadcast(&wake);
    pthread_mutex_unlock(&lock);
    return value;
}

uint64_t logos_mmio_read(logos_dev *dev, uint32_t byte_offset) {
    (void)dev;
    return bus_access(WANT_READ, byte_offset, 0);
}

void logos_mmio_write(logos_dev *dev, uint32_t byte_offset, uint64_t value) {
    (void)dev;
    bus_access(WANT_WRITE, byte_offset, value);
}

void *logos_buffer(logos_dev *dev, size_t bytes, uint64_t *io_address) {
    (void)dev;
    for (int i = 0; i < LOGOS_MAX_BUFFERS; ++i) {
        if (buffers[i].ptr) continue;
        size_t rounded = (bytes + 4095) & ~(size_t)4095;
        buffers[i].ptr = aligned_alloc(4096, rounded);
        if (!buffers[i].ptr) return NULL;
        buffers[i].io = next_io;
        buffers[i].bytes = bytes;
        next_io += (rounded + LOGOS_IO_ALIGN - 1) & ~(LOGOS_IO_ALIGN - 1);
        *io_address = buffers[i].io;
        return buffers[i].ptr;
    }
    fprintf(stderr, "logos_buffer: more than %d buffers\n", LOGOS_MAX_BUFFERS);
    return NULL;
}

void logos_buffer_free(logos_dev *dev, void *buffer) {
    (void)dev;
    for (int i = 0; i < LOGOS_MAX_BUFFERS; ++i) {
        if (buffers[i].ptr == buffer) {
            free(buffers[i].ptr);
            buffers[i].ptr = NULL;
        }
    }
}

double logos_user_clock_hz(logos_dev *dev) {
    (void)dev;
    return user_clock_hz;
}

/* The byte of host memory at I/O address `io`, `len` bytes of it inside one buffer; NULL for an address no buffer
   holds, which the simulation stops on, as an IOMMU fault would stop the card. */
static uint8_t *host_bytes(uint64_t io, size_t len) {
    for (int i = 0; i < LOGOS_MAX_BUFFERS; ++i)
        if (buffers[i].ptr && io >= buffers[i].io && io + len <= buffers[i].io + buffers[i].bytes) return buffers[i].ptr + (io - buffers[i].io);
    return NULL;
}

static void *program(void *unused) {
    (void)unused;
    int status = logos_program_main(program_argc, program_argv);
    fflush(stdout);
    pthread_mutex_lock(&lock);
    finished = 1;
    exit_status = status;
    pthread_cond_broadcast(&wake);
    pthread_mutex_unlock(&lock);
    return NULL;
}

/* Start the host program, the core at `core_period_ps`, its arguments those of `argv` that are +arg=<argument>, every
   one of them in order. */
static void sim_start(double core_period_ps, int argc, char **argv) {
    user_clock_hz = 1e12 / core_period_ps;
    int given = 0;
    for (int i = 0; i < argc; ++i) given += strncmp(argv[i], "+arg=", 5) == 0;
    program_argv = calloc((size_t)given + 2, sizeof *program_argv);
    if (!program_argv) {
        fprintf(stderr, "logos_hal_sim: no memory for the program's %d arguments\n", given);
        exit(2);
    }
    program_argc = 0;
    program_argv[program_argc++] = "logos_program";
    for (int i = 0; i < argc; ++i)
        if (strncmp(argv[i], "+arg=", 5) == 0) program_argv[program_argc++] = argv[i] + 5;
    program_argv[program_argc] = NULL;
    pthread_t t;
    started = 1;
    pthread_create(&t, NULL, program, NULL);
    pthread_detach(t);
}

/* This process's command line, the whole of /proc/self/cmdline however long, split into its arguments; `*argc` their
   count. A simulator that keeps no argument list of its own hands the program its arguments from it. */
char **sim_cmdline(int *argc) {
    FILE *f = fopen("/proc/self/cmdline", "rb");
    size_t cap = 1 << 12, n = 0, got;
    char *text = malloc(cap + 1);
    if (!f || !text) {
        perror("logos_hal_sim: /proc/self/cmdline");
        exit(2);
    }
    while ((got = fread(text + n, 1, cap - n, f)) > 0) {
        n += got;
        if (n == cap && !(text = realloc(text, (cap *= 2) + 1))) {
            perror("logos_hal_sim: /proc/self/cmdline");
            exit(2);
        }
    }
    fclose(f);
    text[n] = 0;
    int count = 0;
    for (size_t i = 0; i < n; i += strlen(text + i) + 1) ++count;
    char **all = calloc((size_t)count + 1, sizeof *all);
    if (!all) {
        perror("logos_hal_sim: /proc/self/cmdline");
        exit(2);
    }
    count = 0;
    for (size_t i = 0; i < n; i += strlen(text + i) + 1) all[count++] = text + i;
    *argc = count;
    return all;
}

/* At a falling edge of pClk, the read data the port shows `valid`/`data`: finish the access on the bus, then wait for
   the program's next one (or its end) and set `bus` to put it on the bus. 1 once the program has exited with no
   access outstanding. */
static int sim_mmio(int valid, uint64_t data) {
    int over = 0;
    pthread_mutex_lock(&lock);
    if (state == READ_ISSUED) {
        bus.read = 0;
        state = AWAIT_DATA;
    }
    if (state == WRITE_ISSUED) {
        bus.write = 0;
        state = DONE;
        pthread_cond_broadcast(&wake);
    }
    if (state == AWAIT_DATA && valid) {
        req_value = data;
        state = DONE;
        pthread_cond_broadcast(&wake);
    }
    while ((state == IDLE || state == DONE) && !finished) pthread_cond_wait(&wake, &lock);
    if (state == WANT_READ) {
        bus.address = req_word;
        bus.read = 1;
        state = READ_ISSUED;
    } else if (state == WANT_WRITE) {
        bus.address = req_word;
        bus.wdata = req_value;
        bus.write = 1;
        state = WRITE_ISSUED;
    } else if (finished && state == IDLE) {
        over = 1;
    }
    pthread_mutex_unlock(&lock);
    return over;
}

#endif
"##;

const HAL_VPI: &str = r##"/* The D5005 accelerator's RTL as a logos_dev, for Icarus Verilog's VPI (logos_hal_sim.h the rest): cosim's
   `$logos_program` starts the program, `$logos_mmio` carries its accesses on the MMIO port at every falling edge of
   pClk, and `$logos_host_read` and `$logos_host_write` are host memory.

   Build: iverilog-vpi --name=logos_cosim -DLOGOS_PROGRAM_MAIN=logos_program_main logos_hal_vpi.c <program>.c */
#include "logos_hal_sim.h"

#include <vpi_user.h>

static vpiHandle arg(vpiHandle *it) { return vpi_scan(*it); }

static uint64_t get_u64(vpiHandle h) {
    s_vpi_value v = {vpiVectorVal, {0}};
    vpi_get_value(h, &v);
    int words = (vpi_get(vpiSize, h) + 31) / 32;
    uint64_t lo = (uint32_t)v.value.vector[0].aval;
    uint64_t hi = words > 1 ? (uint32_t)v.value.vector[1].aval : 0;
    return lo | hi << 32;
}

static void put_u64(vpiHandle h, uint64_t x) {
    s_vpi_vecval vec[2] = {{(PLI_INT32)(uint32_t)x, 0}, {(PLI_INT32)(uint32_t)(x >> 32), 0}};
    s_vpi_value v;
    v.format = vpiVectorVal;
    v.value.vector = vec;
    vpi_put_value(h, &v, NULL, vpiNoDelay);
}

/* $logos_program(core_period_ps): start the host program, its arguments the plusargs +arg=<argument> in order. */
static PLI_INT32 program_calltf(PLI_BYTE8 *unused) {
    (void)unused;
    vpiHandle call = vpi_handle(vpiSysTfCall, NULL), it = vpi_iterate(vpiArgument, call);
    s_vpi_vlog_info info;
    double period = (double)get_u64(arg(&it));
    vpi_free_object(it);
    vpi_get_vlog_info(&info);
    sim_start(period, info.argc, info.argv);
    return 0;
}

/* $logos_mmio(readdatavalid, readdata, address, read, write, writedata), at every falling edge of pClk. */
static PLI_INT32 mmio_calltf(PLI_BYTE8 *unused) {
    (void)unused;
    vpiHandle call = vpi_handle(vpiSysTfCall, NULL), it = vpi_iterate(vpiArgument, call);
    vpiHandle valid = arg(&it), data = arg(&it), address = arg(&it), read = arg(&it), write = arg(&it), wdata = arg(&it);
    vpi_free_object(it);
    int over = sim_mmio((int)get_u64(valid), get_u64(data));
    put_u64(address, bus.address);
    put_u64(read, (uint64_t)bus.read);
    put_u64(write, (uint64_t)bus.write);
    put_u64(wdata, bus.wdata);
    if (over) {
        vpi_printf("logos_cosim: the program exited %d\n", exit_status);
        vpi_control(vpiFinish, 1);
    }
    return 0;
}

/* $logos_host_read(line, data) and $logos_host_write(line, data): one 64-byte line of host memory. */
static PLI_INT32 host_calltf(PLI_BYTE8 *write) {
    vpiHandle call = vpi_handle(vpiSysTfCall, NULL), it = vpi_iterate(vpiArgument, call);
    vpiHandle line = arg(&it), data = arg(&it);
    vpi_free_object(it);
    uint64_t io = get_u64(line) << 6;
    uint8_t *bytes = host_bytes(io, 64);
    if (!bytes) {
        vpi_printf("logos_cosim: host %s of I/O address 0x%llx, which no buffer holds\n", write ? "write" : "read", (unsigned long long)io);
        vpi_control(vpiFinish, 1);
        return 0;
    }
    s_vpi_vecval vec[16];
    s_vpi_value v;
    v.format = vpiVectorVal;
    if (write) {
        v.value.vector = NULL;
        vpi_get_value(data, &v);
        for (int w = 0; w < 16; ++w) {
            uint32_t x = (uint32_t)v.value.vector[w].aval;
            memcpy(bytes + 4 * w, &x, 4);
        }
    } else {
        for (int w = 0; w < 16; ++w) {
            uint32_t x;
            memcpy(&x, bytes + 4 * w, 4);
            vec[w].aval = (PLI_INT32)x;
            vec[w].bval = 0;
        }
        v.value.vector = vec;
        vpi_put_value(data, &v, NULL, vpiNoDelay);
    }
    return 0;
}

static void register_task(const char *name, PLI_INT32 (*calltf)(PLI_BYTE8 *), PLI_BYTE8 *data) {
    s_vpi_systf_data tf = {vpiSysTask, 0, (PLI_BYTE8 *)name, calltf, NULL, NULL, data};
    vpi_register_systf(&tf);
}

static void register_all(void) {
    register_task("$logos_program", program_calltf, NULL);
    register_task("$logos_mmio", mmio_calltf, NULL);
    register_task("$logos_host_read", host_calltf, NULL);
    register_task("$logos_host_write", host_calltf, (PLI_BYTE8 *)"w");
}

void (*vlog_startup_routines[])(void) = {register_all, NULL};
"##;

const HAL_DPI: &str = r##"/* The D5005 accelerator's RTL as a logos_dev, for Verilator's DPI-C (logos_hal_sim.h the rest): cosim built with
   LOGOS_DPI calls `logos_dpi_program` to start the program, `logos_dpi_mmio` at every falling edge of pClk to carry
   its accesses on the MMIO port, and `logos_dpi_host_read` and `logos_dpi_host_write` for host memory. The program's
   arguments are the simulation's own +arg=<argument> plusargs, read from the process's command line.

   Build: cc -c -I<verilator root>/include/vltstd -DLOGOS_PROGRAM_MAIN=logos_program_main logos_hal_dpi.c <program>.c,
   the objects linked into the Verilated cosim. */
#include "logos_hal_sim.h"

#include <svdpi.h>

void logos_dpi_program(long long core_period_ps) {
    int argc;
    char **argv = sim_cmdline(&argc);
    sim_start((double)core_period_ps, argc, argv);
}

int logos_dpi_mmio(svBit valid, long long data, int *address, svBit *read, svBit *write, long long *wdata) {
    int over = sim_mmio(valid, (uint64_t)data);
    *address = (int)bus.address;
    *read = (svBit)bus.read;
    *write = (svBit)bus.write;
    *wdata = (long long)bus.wdata;
    return over;
}

int logos_dpi_exit_status(void) { return exit_status; }

int logos_dpi_host_read(long long line, svBitVecVal *data) {
    uint8_t *bytes = host_bytes((uint64_t)line << 6, 64);
    if (!bytes) return 1;
    memcpy(data, bytes, 64);
    return 0;
}

int logos_dpi_host_write(long long line, const svBitVecVal *data) {
    uint8_t *bytes = host_bytes((uint64_t)line << 6, 64);
    if (!bytes) return 1;
    memcpy(bytes, data, 64);
    return 0;
}
"##;

/// The co-simulation top, its configured values at the `{…}` markers.
const COSIM_TEMPLATE: &str = r##"// The accelerator's shell with a host program attached: the program (through logos_hal_vpi under Icarus Verilog, or
// logos_hal_dpi under Verilator with LOGOS_DPI defined) drives the MMIO port and owns host memory, four memory
// channels run on four unrelated clocks near 300 MHz, the host channel at 250 MHz, the shell at the user clock, its
// compute side stepping at every DIVIDE-th edge and the host program told that step's period; behind
// each channel the AFU configuration's channel model (with DDR4_LOG set, a DDR4-timed channel logs what it schedules).
// Run with the VPI module, vvp -M <dir> -m logos_cosim cosim, or the Verilated binary, either with
// [+arg=<the program's argument>]... (scripts/d5005/run-cosim.sh).
`timescale 1ps/1ps
module logos_d5005_cosim;
  parameter NB = 4, AW = 27, DWM = {mem_bits}, ECC = {ecc}, MBW = 7, HAW = 51, DWH = 512, HBW = 7, MAW = 15;
  parameter [63:0] AFU_ID_L = 64'h{id_l}, AFU_ID_H = 64'h{id_h}, BUILD = 64'h{build};
  parameter UCLK_PS = {uclk_ps};
  parameter DIVIDE = {divide};
  parameter DDR4_LOG = 0;
  reg pclk = 0, uclk = 0, rst_n = 0;
  // A channel clock a register, each its own process: Verilator takes no edge of one bit of a vector.
  reg mem_clk0 = 0, mem_clk1 = 0, mem_clk2 = 0, mem_clk3 = 0;
  wire [NB-1:0] mem_clk = {mem_clk3, mem_clk2, mem_clk1, mem_clk0};
  always #2000 pclk = ~pclk;
  always #(UCLK_PS / 2) uclk = ~uclk;
{uclk2_clock}  always #1667 mem_clk0 = ~mem_clk0;
  initial begin #431 forever #1671 mem_clk1 = ~mem_clk1; end
  initial begin #977 forever #1663 mem_clk2 = ~mem_clk2; end
  initial begin #1301 forever #1669 mem_clk3 = ~mem_clk3; end

  reg  [MAW-1:0] mmio_address = 0;
  reg            mmio_read = 0, mmio_write = 0;
  reg  [63:0]    mmio_writedata = 0;
  wire           mmio_waitrequest, mmio_readdatavalid;
  wire [63:0]    mmio_readdata;
  wire [HAW-1:0] host_rd_address, host_wr_address;
  wire           host_rd_read, host_wr_write, host_rd_waitrequest, host_rd_readdatavalid, host_wr_waitrequest, host_wr_writeresponsevalid;
  wire [HBW-1:0] host_rd_burstcount, host_wr_burstcount;
  wire [DWH-1:0] host_rd_readdata, host_wr_writedata;
  wire [DWH/8-1:0] host_wr_byteenable;
  wire [NB*AW-1:0] mem_address;
  wire [NB-1:0]    mem_read, mem_write, mem_waitrequest, mem_readdatavalid;
  wire [NB*MBW-1:0] mem_burstcount;
  wire [NB*DWM-1:0] mem_writedata, mem_readdata;
  wire [NB*DWM/8-1:0] mem_byteenable;

  logos_d5005_shell #(.NB(NB), .AW(AW), .DWM(DWM), .ECC(ECC), .MBW(MBW), .HAW(HAW), .DWH(DWH), .HBW(HBW), .MAW(MAW),
                      .AFU_ID_L(AFU_ID_L), .AFU_ID_H(AFU_ID_H), .BUILD(BUILD)) shell(
    .pclk(pclk), .pclk_rst_n(rst_n), .uclk(uclk), .uclk_rst_n(rst_n),{uclk2_cosim}
    .mmio_address(mmio_address), .mmio_read(mmio_read), .mmio_write(mmio_write), .mmio_writedata(mmio_writedata),
    .mmio_byteenable(8'hff), .mmio_waitrequest(mmio_waitrequest), .mmio_readdata(mmio_readdata),
    .mmio_readdatavalid(mmio_readdatavalid),
    .host_rd_address(host_rd_address), .host_rd_read(host_rd_read), .host_rd_burstcount(host_rd_burstcount),
    .host_rd_waitrequest(host_rd_waitrequest), .host_rd_readdata(host_rd_readdata), .host_rd_readdatavalid(host_rd_readdatavalid),
    .host_wr_address(host_wr_address), .host_wr_write(host_wr_write), .host_wr_burstcount(host_wr_burstcount),
    .host_wr_writedata(host_wr_writedata), .host_wr_byteenable(host_wr_byteenable), .host_wr_waitrequest(host_wr_waitrequest),
    .host_wr_writeresponsevalid(host_wr_writeresponsevalid),
    .mem_clk(mem_clk), .mem_rst_n({NB{rst_n}}), .mem_address(mem_address), .mem_read(mem_read), .mem_write(mem_write),
    .mem_burstcount(mem_burstcount), .mem_writedata(mem_writedata), .mem_byteenable(mem_byteenable),
    .mem_waitrequest(mem_waitrequest), .mem_readdata(mem_readdata), .mem_readdatavalid(mem_readdatavalid));

  // Each channel's lines and the host's, a toggle flipped in its own clock on each one that moves: a run whose program
  // waits on a command while no line of memory or of the host moves for STALL compute cycles has stalled, far past any
  // job's own quiet stretch, and ends saying so rather than waiting out the program's polls.
  parameter STALL = 1000000;
  wire [NB-1:0] moved;
  genvar b;
  generate for (b = 0; b < NB; b = b + 1) begin : channel
    {channel_model} memory(.clk(mem_clk[b]),
      .rst_n(rst_n), .address(mem_address[b*AW +: AW]), .read(mem_read[b]), .write(mem_write[b]),
      .burstcount(mem_burstcount[b*MBW +: MBW]), .writedata(mem_writedata[b*DWM +: DWM]),
      .byteenable(mem_byteenable[b*DWM/8 +: DWM/8]), .waitrequest(mem_waitrequest[b]), .readdata(mem_readdata[b*DWM +: DWM]),
      .readdatavalid(mem_readdatavalid[b]));
    reg flip = 1'b0;
    always @(posedge mem_clk[b]) if (mem_readdatavalid[b] || ((mem_read[b] || mem_write[b]) && !mem_waitrequest[b])) flip <= ~flip;
    assign moved[b] = flip;
  end endgenerate
  reg host_flip = 1'b0;
  always @(posedge pclk)
    if (host_rd_readdatavalid || (host_rd_read && !host_rd_waitrequest) || (host_wr_write && !host_wr_waitrequest)) host_flip <= ~host_flip;
  reg [NB:0] seen = 0;
  longint quiet = 0;
  always @(posedge uclk) if (rst_n) begin
    if ({host_flip, moved} != seen) begin
      seen <= {host_flip, moved};
      quiet <= 0;
    end else if (quiet == STALL * DIVIDE) begin
      $display("logos_cosim: stalled: no line of memory or of the host moved for %0d compute cycles", STALL);
      $finish;
    end else quiet <= quiet + 1;
  end

  logos_host_memory_vpi #(.AW(HAW), .DW(DWH), .BW(HBW)) host(.clk(pclk), .rst_n(rst_n),
    .rd_address(host_rd_address), .rd_read(host_rd_read), .rd_burstcount(host_rd_burstcount), .rd_waitrequest(host_rd_waitrequest),
    .rd_readdata(host_rd_readdata), .rd_readdatavalid(host_rd_readdatavalid), .wr_address(host_wr_address), .wr_write(host_wr_write),
    .wr_burstcount(host_wr_burstcount), .wr_writedata(host_wr_writedata), .wr_waitrequest(host_wr_waitrequest),
    .wr_writeresponsevalid(host_wr_writeresponsevalid));

`ifdef LOGOS_DPI
  import "DPI-C" function void logos_dpi_program(input longint core_period_ps);
  import "DPI-C" function int logos_dpi_mmio(input bit valid, input longint data, output int address, output bit read,
                                             output bit write, output longint wdata);
  import "DPI-C" function int logos_dpi_exit_status();
  int     dpi_address;
  bit     dpi_read, dpi_write;
  longint dpi_wdata;
  initial begin
    #50000 rst_n = 1;
    logos_dpi_program(UCLK_PS * DIVIDE);
  end
  always @(negedge pclk) if (rst_n) begin
    if (logos_dpi_mmio(mmio_readdatavalid, mmio_readdata, dpi_address, dpi_read, dpi_write, dpi_wdata) != 0) begin
      $display("logos_cosim: the program exited %0d", logos_dpi_exit_status());
      $finish;
    end
    mmio_address = dpi_address[MAW-1:0];
    mmio_read = dpi_read;
    mmio_write = dpi_write;
    mmio_writedata = dpi_wdata;
  end
`else
  initial begin
    #50000 rst_n = 1;
    $logos_program(UCLK_PS * DIVIDE);
  end
  always @(negedge pclk) if (rst_n) $logos_mmio(mmio_readdatavalid, mmio_readdata, mmio_address, mmio_read, mmio_write, mmio_writedata);
`endif
endmodule

// Host memory behind the PIM's Avalon-MM read/write port, its lines the host program's buffers: waitrequest at random
// on each channel, read data in order after a random latency, one write response a burst.
module logos_host_memory_vpi #(parameter AW = 51, parameter DW = 512, parameter BW = 7, parameter SEED = 11,
                               parameter WAIT_PCT = 25) (
  input               clk,
  input               rst_n,
  input  [AW-1:0]     rd_address,
  input               rd_read,
  input  [BW-1:0]     rd_burstcount,
  output reg          rd_waitrequest,
  output reg [DW-1:0] rd_readdata,
  output reg          rd_readdatavalid,
  input  [AW-1:0]     wr_address,
  input               wr_write,
  input  [BW-1:0]     wr_burstcount,
  input  [DW-1:0]     wr_writedata,
  output reg          wr_waitrequest,
  output reg          wr_writeresponsevalid
);
`ifdef LOGOS_DPI
  import "DPI-C" function int logos_dpi_host_read(input longint line, output bit [511:0] data);
  import "DPI-C" function int logos_dpi_host_write(input longint line, input bit [511:0] data);
`endif
  integer seed = SEED, cycle = 0, qh = 0, qt = 0, beat = 0, rh = 0, rt = 0;
  reg [AW-1:0] q_addr [0:255];
  reg [BW-1:0] q_len [0:255];
  integer      q_due [0:255];
  integer      r_due [0:255];
  reg [AW-1:0] w_addr;
  reg [BW-1:0] w_left = 0;
  reg [63:0]   line;
  reg [DW-1:0] word;
  always @(posedge clk) begin
    cycle = cycle + 1;
    rd_readdatavalid <= 1'b0;
    wr_writeresponsevalid <= 1'b0;
    if (rst_n) begin
      if (rd_read && !rd_waitrequest) begin
        q_addr[qt % 256] = rd_address; q_len[qt % 256] = rd_burstcount;
        q_due[qt % 256] = cycle + 20 + (($random(seed) % 40 + 40) % 40); qt = qt + 1;
      end
      if (wr_write && !wr_waitrequest) begin
        if (w_left == 0) begin
          line = wr_address; w_addr = wr_address + 1; w_left = wr_burstcount - 1;
        end else begin
          line = w_addr; w_addr = w_addr + 1; w_left = w_left - 1;
        end
        word = wr_writedata;
`ifdef LOGOS_DPI
        if (logos_dpi_host_write(line, word) != 0) begin
          $display("logos_cosim: host write of I/O address 0x%0h, which no buffer holds", line << 6);
          $finish;
        end
`else
        $logos_host_write(line, word);
`endif
        if (w_left == 0) begin r_due[rt % 256] = cycle + 5 + (($random(seed) % 30 + 30) % 30); rt = rt + 1; end
      end
      if (qh != qt && q_due[qh % 256] <= cycle) begin
        line = q_addr[qh % 256] + beat;
`ifdef LOGOS_DPI
        if (logos_dpi_host_read(line, word) != 0) begin
          $display("logos_cosim: host read of I/O address 0x%0h, which no buffer holds", line << 6);
          $finish;
        end
`else
        $logos_host_read(line, word);
`endif
        rd_readdata <= word;
        rd_readdatavalid <= 1'b1;
        beat = beat + 1;
        if (beat == q_len[qh % 256]) begin beat = 0; qh = qh + 1; end
      end
      if (rh != rt && r_due[rh % 256] <= cycle) begin wr_writeresponsevalid <= 1'b1; rh = rh + 1; end
      rd_waitrequest <= (($random(seed) % 100 + 100) % 100) < WAIT_PCT;
      wr_waitrequest <= (($random(seed) % 100 + 100) % 100) < WAIT_PCT;
    end else begin
      rd_waitrequest <= 1'b1;
      wr_waitrequest <= 1'b1;
    end
  end
endmodule
"##;

const MEMORY_MODEL: &str = r##"// A memory behind an Avalon-MM slave port, for simulation: waitrequest at random, a read burst's lines taken as the
// memory holds them when the burst is accepted and returned in order after a random latency with random gaps, so a
// write accepted after a read never reaches it, bursts on reads and writes, lines addressed by index.
module logos_avmm_memory_model #(parameter AW = 27, parameter DW = 64, parameter BW = 7, parameter LINES_LOG2 = 12,
                                 parameter SEED = 7, parameter WAIT_PCT = 30, parameter GAP_PCT = 20) (
  input               clk,
  input               rst_n,
  input  [AW-1:0]     address,
  input               read,
  input               write,
  input  [BW-1:0]     burstcount,
  input  [DW-1:0]     writedata,
  input  [DW/8-1:0]   byteenable,
  output reg          waitrequest,
  output reg [DW-1:0] readdata,
  output reg          readdatavalid
);
  reg [DW-1:0] mem [0:(1<<LINES_LOG2)-1];
  integer seed = SEED;
  // Accepted read bursts, in order: {beats, ready time}; and their lines, in order.
  reg [BW-1:0] q_len [0:255];
  integer      q_due [0:255];
  reg [DW-1:0] q_data [0:4095];
  integer qh = 0, qt = 0, lh = 0, lt = 0, cycle = 0, beat = 0;
  reg [AW-1:0] w_addr;
  reg [BW-1:0] w_left = 0;
  integer i;
  initial for (i = 0; i < (1 << LINES_LOG2); i = i + 1) mem[i] = {DW{1'bx}};
  always @(posedge clk) begin
    cycle = cycle + 1;
    readdatavalid <= 1'b0;
    if (rst_n) begin
      if (read && !waitrequest) begin
        q_len[qt % 256] = burstcount;
        q_due[qt % 256] = cycle + 10 + (($random(seed) % 30 + 30) % 30);
        qt = qt + 1;
        for (i = 0; i < burstcount; i = i + 1) begin
          q_data[lt % 4096] = mem[(address + i) % (1 << LINES_LOG2)];
          lt = lt + 1;
        end
      end
      if (write && !waitrequest) begin
        if (w_left == 0) begin
          mem[address[LINES_LOG2-1:0]] = writedata;
          w_addr = address + 1;
          w_left = burstcount - 1;
        end else begin
          mem[w_addr[LINES_LOG2-1:0]] = writedata;
          w_addr = w_addr + 1;
          w_left = w_left - 1;
        end
      end
      if (qh != qt && q_due[qh % 256] <= cycle && (($random(seed) % 100 + 100) % 100) >= GAP_PCT) begin
        readdata <= q_data[lh % 4096];
        readdatavalid <= 1'b1;
        lh = lh + 1;
        beat = beat + 1;
        if (beat == q_len[qh % 256]) begin
          beat = 0;
          qh = qh + 1;
        end
      end
      waitrequest <= (($random(seed) % 100 + 100) % 100) < WAIT_PCT;
    end else begin
      waitrequest <= 1'b1;
    end
  end
endmodule
"##;
