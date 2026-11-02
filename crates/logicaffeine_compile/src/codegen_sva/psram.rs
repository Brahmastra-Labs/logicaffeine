//! The off-chip memory edge for a QSPI PSRAM — the block behind the memory-edge descriptor contract
//! when the far side is a pseudo-SRAM instead of a DRAM.
//!
//! The part is data ([`PsramPart`], its numbers quoted from the datasheet). A transfer descriptor is
//! split at compile time into bursts the part can legally take ([`plan_stream`]): a burst never
//! crosses a page, because the part wraps inside a page, and never holds CE# low past tCEM, because
//! the part refreshes itself only while deselected. [`check_legal`] re-checks a plan independently.
//! The controller ([`emit_psram_edge_verilog`]) runs the power-up sequence the part requires (tPU,
//! Reset-Enable + Reset, tRST, enter QPI) and then one QPI burst per request; the pin-level model
//! ([`emit_psram_model_verilog`]) implements the part's semantics and counts every timing and pin rule
//! a host breaks, so a co-simulation reports violations rather than assuming there are none.

use super::dram::StreamDesc;
use std::fmt::Write as _;

/// A QSPI PSRAM, as its datasheet specifies it. Times in picoseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PsramPart {
    pub name: &'static str,
    pub provenance: &'static str,
    pub capacity_bytes: u32,
    /// Bursts wrap to the start of the same page.
    pub page_bytes: u32,
    pub t_clk_min_ps: u32,
    pub t_cph_min_ps: u32,
    pub t_cem_max_ps: u32,
    pub t_pu_min_ps: u32,
    pub t_rst_min_ps: u32,
    pub t_aclk_max_ps: u32,
    /// QPI Fast Quad Read and its wait cycles.
    pub read_cmd: u8,
    pub read_wait: u32,
    /// QPI Quad Write (no wait cycles).
    pub write_cmd: u8,
    /// SPI-mode command that switches the part into QPI.
    pub enter_qpi: u8,
    pub reset_enable: u8,
    pub reset: u8,
}

/// AP Memory APS6404L-3SQN: 64 Mb (8M x 8) QSPI PSRAM, 3.3 V package, standard temperature grade.
pub const APS6404L_3SQN: PsramPart = PsramPart {
    name: "APS6404L-3SQN",
    provenance: "AP Memory APS6404L-3SQN QSPI PSRAM datasheet, Rev. 2.7 (2024-01-05): §8 power-up \
                 initialization, §9.1-9.5 address space, page size and command table, §16.7 Table 5 \
                 READ/WRITE timing (3.3 V package, standard grade)",
    capacity_bytes: 8 << 20,
    page_bytes: 1024,
    t_clk_min_ps: 9_170,
    t_cph_min_ps: 18_000,
    t_cem_max_ps: 8_000_000,
    t_pu_min_ps: 150_000_000,
    t_rst_min_ps: 50_000,
    t_aclk_max_ps: 5_500,
    read_cmd: 0xEB,
    read_wait: 6,
    write_cmd: 0x38,
    enter_qpi: 0x35,
    reset_enable: 0x66,
    reset: 0x99,
};

/// Why an edge or a plan cannot be built for a part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PsramUnplaceable {
    /// The SCLK the system clock divides down to is faster than the part's tCLK.
    SclkTooFast { sclk_ps: u64, min_ps: u32 },
    /// Read data is taken at the rising edge half a period after the part drives it at the falling
    /// edge, so half a period must cover tACLK.
    ReadWindowTooShort { half_ps: u64, t_aclk_ps: u32 },
    /// A zero half-period divider.
    ZeroDivider,
    /// Only 8-byte words are planned; a stream of any other shape is refused.
    NotWords {
        elem_bytes: u64,
        stride_bytes: u64,
        base: u64,
    },
    /// A cache needs at least two lines to have an index and two words a line to have an offset.
    CacheShape { index_bits: u32, offset_bits: u32 },
    /// Index and offset must leave at least one tag bit of the edge's word address.
    CacheWiderThanAddress {
        index_bits: u32,
        offset_bits: u32,
        addr_bits: u32,
    },
    /// A line is filled and written back as one burst, so it must be a burst the part can take.
    LineBurst { line_words: u32, max_words: u32 },
    /// The slowest clock is faster than the clock the edge is timed for.
    SlowestFasterThanFastest { slowest_ps: u32, fastest_ps: u32 },
    /// At the slowest clock not even a one-word burst ends inside tCEM.
    NoBurstAtSlowestClock { slowest_ps: u32 },
}

/// One burst: `words` consecutive 64-bit words from word address `word_addr`, one CE# low.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PsramBurst {
    pub write: bool,
    pub word_addr: u32,
    pub words: u32,
}

/// The edge: a part, the system clock that drives the controller, and the number of system cycles
/// per SCLK half-period. The controller counts cycles, so the rules that set a minimum time — tPU, tRST,
/// tCPH, tCLK, tACLK — are met for every clock no faster than `sys_period_ps`, while tCEM, which caps how
/// long a burst may hold CE# low, must hold at the slowest clock the edge is run at, `slowest_period_ps`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PsramEdge {
    pub part: PsramPart,
    pub sys_period_ps: u32,
    pub half: u32,
    pub slowest_period_ps: u32,
}

/// Command (2 nibbles) and 24-bit address (6 nibbles) edges of a QPI burst.
const QPI_HEADER_EDGES: u32 = 8;
/// SCLK edges per 64-bit word: 8 bytes, a nibble per edge.
const EDGES_PER_WORD: u32 = 16;
/// Width of the edge's word address (`req_addr`): the 24-bit QPI byte address of an 8 MiB part, less the
/// three bits inside a word.
pub const EDGE_WORD_ADDR_BITS: u32 = 20;

impl PsramEdge {
    pub fn new(part: PsramPart, sys_period_ps: u32, half: u32) -> Result<Self, PsramUnplaceable> {
        if half == 0 {
            return Err(PsramUnplaceable::ZeroDivider);
        }
        let e = PsramEdge {
            part,
            sys_period_ps,
            half,
            slowest_period_ps: sys_period_ps,
        };
        if e.sclk_period_ps() < part.t_clk_min_ps as u64 {
            return Err(PsramUnplaceable::SclkTooFast {
                sclk_ps: e.sclk_period_ps(),
                min_ps: part.t_clk_min_ps,
            });
        }
        let half_ps = half as u64 * sys_period_ps as u64;
        if half_ps <= part.t_aclk_max_ps as u64 {
            return Err(PsramUnplaceable::ReadWindowTooShort {
                half_ps,
                t_aclk_ps: part.t_aclk_max_ps,
            });
        }
        Ok(e)
    }

    /// The same edge, also run at every clock down to `slowest_period_ps`: its bursts shrink to what
    /// tCEM allows there.
    pub fn clocked_down_to(self, slowest_period_ps: u32) -> Result<Self, PsramUnplaceable> {
        if slowest_period_ps < self.sys_period_ps {
            return Err(PsramUnplaceable::SlowestFasterThanFastest {
                slowest_ps: slowest_period_ps,
                fastest_ps: self.sys_period_ps,
            });
        }
        let e = PsramEdge {
            slowest_period_ps,
            ..self
        };
        if e.max_words(false) == 0 || e.max_words(true) == 0 {
            return Err(PsramUnplaceable::NoBurstAtSlowestClock {
                slowest_ps: slowest_period_ps,
            });
        }
        Ok(e)
    }

    pub fn sclk_period_ps(&self) -> u64 {
        2 * self.half as u64 * self.sys_period_ps as u64
    }

    fn cycles_for(&self, ps: u32) -> u32 {
        (ps as u64).div_ceil(self.sys_period_ps as u64) as u32
    }

    /// System cycles the controller waits after reset release before its first command (tPU).
    pub fn powerup_cycles(&self) -> u32 {
        self.cycles_for(self.part.t_pu_min_ps)
    }

    /// System cycles CE# stays high after a burst (tCPH).
    pub fn cph_cycles(&self) -> u32 {
        self.cycles_for(self.part.t_cph_min_ps)
    }

    /// System cycles CE# stays high after the Reset command (tRST).
    pub fn rst_cycles(&self) -> u32 {
        self.cycles_for(self.part.t_rst_min_ps)
    }

    /// Rising edges from reset release until `req_ready` first rises: tPU, then Reset-Enable, Reset and
    /// Enter-QPI as eight-edge SPI bytes, with CE# high for tCPH, tRST and tCPH after them.
    pub fn init_cycles(&self) -> u64 {
        let gap = |c: u32| c.max(1) as u64;
        self.powerup_cycles() as u64
            + 3 * 8 * 2 * self.half as u64
            + gap(self.cph_cycles())
            + gap(self.rst_cycles())
            + gap(self.cph_cycles())
    }

    /// SCLK edges of a burst of `words` words.
    pub fn edges(&self, write: bool, words: u32) -> u32 {
        QPI_HEADER_EDGES + if write { 0 } else { self.part.read_wait } + EDGES_PER_WORD * words
    }

    /// How long CE# is low for a burst at the slowest clock: every SCLK edge, from CE# falling with the
    /// first output to CE# rising with the last falling edge.
    pub fn ce_low_ps(&self, write: bool, words: u32) -> u64 {
        self.edges(write, words) as u64 * 2 * self.half as u64 * self.slowest_period_ps as u64
    }

    /// The longest burst the part can take: inside tCEM, and no longer than a page.
    pub fn max_words(&self, write: bool) -> u32 {
        let page_words = self.part.page_bytes / 8;
        (1..=page_words)
            .take_while(|&n| self.ce_low_ps(write, n) <= self.part.t_cem_max_ps as u64)
            .last()
            .unwrap_or(0)
    }

    /// System cycles from the request being accepted to the controller accepting the next one: the
    /// burst's SCLK edges, then CE# high for tCPH (at least one cycle).
    pub fn burst_sys_cycles(&self, write: bool, words: u32) -> u64 {
        self.edges(write, words) as u64 * 2 * self.half as u64 + self.cph_cycles().max(1) as u64
    }
}

/// Split a stream of 8-byte words into bursts the part can take: at every page boundary and at the
/// longest burst tCEM allows. A strided stream is one burst per word.
pub fn plan_stream(edge: &PsramEdge, s: &StreamDesc) -> Result<Vec<PsramBurst>, PsramUnplaceable> {
    if s.elem_bytes != 8 || s.base % 8 != 0 || s.stride_bytes % 8 != 0 || s.stride_bytes == 0 {
        return Err(PsramUnplaceable::NotWords {
            elem_bytes: s.elem_bytes,
            stride_bytes: s.stride_bytes,
            base: s.base,
        });
    }
    let page_words = edge.part.page_bytes / 8;
    let max = edge.max_words(s.write);
    let mut out = Vec::new();
    if s.stride_bytes != 8 {
        for k in 0..s.count {
            out.push(PsramBurst {
                write: s.write,
                word_addr: ((s.base + k * s.stride_bytes) / 8) as u32,
                words: 1,
            });
        }
        return Ok(out);
    }
    let mut w = (s.base / 8) as u32;
    let mut left = s.count as u32;
    while left > 0 {
        let n = left.min(max).min(page_words - w % page_words);
        out.push(PsramBurst {
            write: s.write,
            word_addr: w,
            words: n,
        });
        w += n;
        left -= n;
    }
    Ok(out)
}

/// Every rule a burst plan must satisfy, checked independently of the planner.
pub fn check_legal(edge: &PsramEdge, plan: &[PsramBurst]) -> Result<(), String> {
    let page_words = edge.part.page_bytes / 8;
    let words_total = edge.part.capacity_bytes / 8;
    for (i, b) in plan.iter().enumerate() {
        if b.words == 0 {
            return Err(format!("burst {i}: empty"));
        }
        if b.word_addr as u64 + b.words as u64 > words_total as u64 {
            return Err(format!(
                "burst {i}: words {}..{} past the part's {words_total}",
                b.word_addr,
                b.word_addr + b.words
            ));
        }
        if b.word_addr % page_words + b.words > page_words {
            return Err(format!(
                "burst {i}: word {} + {} crosses a {}-byte page",
                b.word_addr, b.words, edge.part.page_bytes
            ));
        }
        let low = edge.ce_low_ps(b.write, b.words);
        if low > edge.part.t_cem_max_ps as u64 {
            return Err(format!(
                "burst {i}: CE# low {low} ps exceeds tCEM {} ps",
                edge.part.t_cem_max_ps
            ));
        }
    }
    Ok(())
}

/// The controller. One request is one burst: `req_len` words at word address `req_addr`. A write
/// takes `wdata` at the start of each word (pulsing `wdata_take`); a read returns each word on
/// `rdata` with a one-cycle `rdata_valid`. `req_ready` is high whenever a request can be taken.
pub fn emit_psram_edge_verilog(edge: &PsramEdge) -> String {
    let p = &edge.part;
    let [c_hi, c_lo] = [p.read_cmd >> 4, p.read_cmd & 15];
    let [w_hi, w_lo] = [p.write_cmd >> 4, p.write_cmd & 15];
    let data_read = QPI_HEADER_EDGES + p.read_wait;
    format!(
        "// psram_edge — the off-chip memory edge for the {name} QSPI PSRAM (codegen_sva::psram).\n\
         // Power-up: tPU with CLK low and CE# high, Reset-Enable 0x{rsten:02x} + Reset 0x{rst:02x} in SPI, tRST,\n\
         // then 0x{qpi:02x} into QPI. A request is one QPI burst: 0x{rd:02x} (wait {wait}) to read, 0x{wr:02x} to write,\n\
         // a 24-bit byte address, 16 nibbles per 64-bit word (byte 0 first, high nibble first). Outputs\n\
         // change at SCLK's falling edge; the part samples at its rising edge, and read data is taken at\n\
         // the rising edge. Emitted by `cargo run -p logicaffeine-compile --example emit_psram_edge`.\n\
         module psram_edge #(\n\
         \x20 parameter integer H = {half},\n\
         \x20 parameter integer POWERUP = {pu},\n\
         \x20 parameter integer CPH = {cph},\n\
         \x20 parameter integer RSTGAP = {rstgap}\n\
         ) (\n\
         \x20 input clk, input rst,\n\
         \x20 input req_valid, output req_ready, input req_we, input [{am}:0] req_addr, input [7:0] req_len,\n\
         \x20 input [63:0] wdata, output reg wdata_take,\n\
         \x20 output reg [63:0] rdata, output reg rdata_valid,\n\
         \x20 output reg ce_n, output reg sclk, output reg [3:0] sio_out, output reg [3:0] sio_oe, input [3:0] sio_in\n\
         );\n\
         \x20 localparam S_PU = 2'd0, S_OP = 2'd1, S_GAP = 2'd2, S_IDLE = 2'd3;\n\
         \x20 localparam O_SPI = 2'd0, O_READ = 2'd1, O_WRITE = 2'd2;\n\
         \x20 reg [1:0] state, op, init;\n\
         \x20 reg [7:0] spi_byte;\n\
         \x20 reg [23:0] baddr;\n\
         \x20 reg [11:0] ei, nei;\n\
         \x20 reg [7:0] phase;\n\
         \x20 reg [31:0] count;\n\
         \x20 reg [63:0] wbuf, rbuf;\n\
         \x20 assign req_ready = (state == S_IDLE);\n\
         \n\
         \x20 // The nibble to present on edge `e` of the current operation, and whether to drive it.\n\
         \x20 function [4:0] nibble(input [1:0] o, input [11:0] e, input [7:0] sb, input [23:0] a, input [63:0] w);\n\
         \x20   reg [11:0] d; reg [3:0] j;\n\
         \x20   begin\n\
         \x20     if (o == O_SPI) nibble = {{1'b1, 3'b000, sb[3'd7 - e[2:0]]}};\n\
         \x20     else if (e == 12'd0) nibble = {{1'b1, (o == O_READ) ? 4'h{c_hi:x} : 4'h{w_hi:x}}};\n\
         \x20     else if (e == 12'd1) nibble = {{1'b1, (o == O_READ) ? 4'h{c_lo:x} : 4'h{w_lo:x}}};\n\
         \x20     else if (e < 12'd{hdr}) nibble = {{1'b1, a[5'd23 - 5'd4 * (e[4:0] - 5'd2) -: 4]}};\n\
         \x20     else if (o == O_READ) nibble = 5'b0_0000;\n\
         \x20     else begin\n\
         \x20       d = e - 12'd{hdr}; j = d[3:0];\n\
         \x20       nibble = {{1'b1, w[8 * j[3:1] + 7 - 4 * j[0] -: 4]}};\n\
         \x20     end\n\
         \x20   end\n\
         \x20 endfunction\n\
         \n\
         \x20 // Begin operation `o`: CE# falls with the first edge's output.\n\
         \x20 task start(input [1:0] o, input [7:0] sb);\n\
         \x20   reg [4:0] n;\n\
         \x20   begin\n\
         \x20     op <= o; spi_byte <= sb; ei <= 12'd0; phase <= 8'd0; state <= S_OP;\n\
         \x20     ce_n <= 1'b0; sclk <= 1'b0;\n\
         \x20     nei <= (o == O_SPI) ? 12'd8 : (o == O_READ) ? 12'd{data_read} + {{req_len, 4'b0000}} : 12'd{hdr} + {{req_len, 4'b0000}};\n\
         \x20     n = nibble(o, 12'd0, sb, (o == O_SPI) ? 24'd0 : {{1'b0, req_addr, 3'b000}}, wdata);\n\
         \x20     sio_oe <= (o == O_SPI) ? 4'b0001 : {{4{{n[4]}}}}; sio_out <= n[3:0];\n\
         \x20   end\n\
         \x20 endtask\n\
         \n\
         \x20 reg [4:0] nx;\n\
         \x20 reg [11:0] dn;\n\
         \x20 reg [11:0] ds;\n\
         \x20 always @(posedge clk) begin\n\
         \x20   wdata_take <= 1'b0; rdata_valid <= 1'b0;\n\
         \x20   if (rst) begin\n\
         \x20     state <= S_PU; count <= 32'd0; init <= 2'd0;\n\
         \x20     ce_n <= 1'b1; sclk <= 1'b0; sio_oe <= 4'b1111; sio_out <= 4'b0000;\n\
         \x20   end else case (state)\n\
         \x20     S_PU: if (count >= POWERUP - 1) start(O_SPI, 8'h{rsten:02x}); else count <= count + 32'd1;\n\
         \x20     S_IDLE: if (req_valid) begin\n\
         \x20       baddr <= {{1'b0, req_addr, 3'b000}};\n\
         \x20       start(req_we ? O_WRITE : O_READ, 8'h00);\n\
         \x20     end\n\
         \x20     S_GAP: if (count <= 32'd1) begin\n\
         \x20       case (init)\n\
         \x20         2'd1: start(O_SPI, 8'h{rst:02x});\n\
         \x20         2'd2: start(O_SPI, 8'h{qpi:02x});\n\
         \x20         default: state <= S_IDLE;\n\
         \x20       endcase\n\
         \x20     end else count <= count - 32'd1;\n\
         \x20     S_OP: begin\n\
         \x20       if (phase == H - 1) begin\n\
         \x20         sclk <= 1'b1;\n\
         \x20         if (op == O_READ && ei >= 12'd{data_read}) begin\n\
         \x20           ds = ei - 12'd{data_read};\n\
         \x20           if (ds[3:0] == 4'd15) begin\n\
         \x20             rdata <= {{rbuf[63:60], sio_in, rbuf[55:0]}}; rdata_valid <= 1'b1;\n\
         \x20           end else rbuf[8 * ds[3:1] + 7 - 4 * ds[0] -: 4] <= sio_in;\n\
         \x20         end\n\
         \x20       end\n\
         \x20       if (phase == 2 * H - 1) begin\n\
         \x20         sclk <= 1'b0; phase <= 8'd0;\n\
         \x20         if (ei == nei - 12'd1) begin\n\
         \x20           ce_n <= 1'b1; sio_oe <= 4'b0000; state <= S_GAP;\n\
         \x20           if (op == O_SPI) begin\n\
         \x20             init <= init + 2'd1;\n\
         \x20             count <= (init == 2'd1) ? RSTGAP : CPH;\n\
         \x20           end else count <= CPH;\n\
         \x20         end else begin\n\
         \x20           ei <= ei + 12'd1;\n\
         \x20           dn = ei + 12'd1 - 12'd{hdr};\n\
         \x20           if (op == O_WRITE && ei + 12'd1 >= 12'd{hdr} && dn[3:0] == 4'd0) begin\n\
         \x20             wbuf <= wdata; wdata_take <= 1'b1;\n\
         \x20             nx = nibble(op, ei + 12'd1, spi_byte, baddr, wdata);\n\
         \x20           end else nx = nibble(op, ei + 12'd1, spi_byte, baddr, wbuf);\n\
         \x20           sio_oe <= (op == O_SPI) ? 4'b0001 : {{4{{nx[4]}}}}; sio_out <= nx[3:0];\n\
         \x20         end\n\
         \x20       end else phase <= phase + 8'd1;\n\
         \x20     end\n\
         \x20   endcase\n\
         \x20 end\n\
         endmodule\n",
        name = p.name,
        rsten = p.reset_enable,
        rst = p.reset,
        qpi = p.enter_qpi,
        rd = p.read_cmd,
        wr = p.write_cmd,
        wait = p.read_wait,
        half = edge.half,
        pu = edge.powerup_cycles(),
        cph = edge.cph_cycles(),
        rstgap = edge.rst_cycles(),
        hdr = QPI_HEADER_EDGES,
        am = EDGE_WORD_ADDR_BITS - 1,
    )
}

/// The heap cache in front of the edge: direct-mapped, write-back and write-allocate, `2^index_bits` lines
/// of `2^offset_bits` words, each line filled and written back as one burst.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PsramCache {
    pub index_bits: u32,
    pub offset_bits: u32,
}

impl PsramCache {
    pub fn new(
        edge: &PsramEdge,
        index_bits: u32,
        offset_bits: u32,
    ) -> Result<Self, PsramUnplaceable> {
        if index_bits == 0 || offset_bits == 0 {
            return Err(PsramUnplaceable::CacheShape {
                index_bits,
                offset_bits,
            });
        }
        if index_bits + offset_bits >= EDGE_WORD_ADDR_BITS {
            return Err(PsramUnplaceable::CacheWiderThanAddress {
                index_bits,
                offset_bits,
                addr_bits: EDGE_WORD_ADDR_BITS,
            });
        }
        let line_words = 1u32 << offset_bits;
        let max_words = edge
            .max_words(true)
            .min(edge.max_words(false))
            .min(u8::MAX as u32);
        if line_words > max_words {
            return Err(PsramUnplaceable::LineBurst {
                line_words,
                max_words,
            });
        }
        Ok(PsramCache {
            index_bits,
            offset_bits,
        })
    }

    pub fn lines(&self) -> u32 {
        1 << self.index_bits
    }

    pub fn line_words(&self) -> u32 {
        1 << self.offset_bits
    }

    pub fn words(&self) -> u32 {
        self.lines() * self.line_words()
    }

    /// Cycles `busy` holds for a miss whose victim is dirty: the request's own cycle, then for the
    /// write-back and for the fill the cycle the edge accepts it, its burst, and the cycle that sees
    /// `req_ready` again.
    pub fn miss_stall_cycles(&self, edge: &PsramEdge) -> u64 {
        1 + 2 * 2
            + edge.burst_sys_cycles(true, self.line_words())
            + edge.burst_sys_cycles(false, self.line_words())
    }

    /// The longest stall: a write and a read issued together, each missing with a dirty victim.
    pub fn worst_stall_cycles(&self, edge: &PsramEdge) -> u64 {
        2 * self.miss_stall_cycles(edge)
    }
}

/// The cache. Its engine side is a registered-read memory port: `ra` holds the read address and `rd` is
/// `data[ra]` one cycle later, refreshed every cycle, so a resident line reads exactly as a block RAM does;
/// `req_r`/`req_w` pulse the cycle after the engine issues a read or a write, whose address, data and
/// half-word enables `wm` hold until its next one. `busy` is high while a miss is served — a dirty victim
/// written back, then the line filled, with a write miss merging its enabled halves into the fill — and
/// while a read issued together with a write waits for it.
///
/// `busy` freezes the whole engine, so it takes no tag lookup of `ra` or `wa`: `rhit`/`whit` are their hits, which
/// the engine registers beside each address as it sets it, looking the new address up in the lines as they will
/// hold after that edge ([`emit_psram_cache_hit_function`]) — the same value, a cycle earlier, so the engine stalls in
/// exactly the cycles it did. The cache exports what that lookup reads (`tags`, `vals`, and the line a fill installs at
/// this edge: `filled`, `fidx`, `ftag`); in simulation it counts every cycle a registered hit disagrees with its lines
/// (`hit_mismatches`, a `PSRAM-CACHE-HIT-MISMATCH` line each).
pub fn emit_psram_cache_verilog(cache: &PsramCache) -> String {
    let tw = EDGE_WORD_ADDR_BITS - cache.index_bits - cache.offset_bits;
    format!(
        "// psram_cache — {lines} lines of {words} 64-bit words, direct-mapped, write-back, in front of psram_edge\n\
         // (codegen_sva::psram).\n\
         module psram_cache #(parameter integer IW = {iw}, parameter integer OW = {ow}) (\n\
         \x20 input clk, input rst,\n\
         \x20 input req_r, input [{am}:0] ra, input rhit,\n\
         \x20 input req_w, input [{am}:0] wa, input whit, input [63:0] wd, input [1:0] wm,\n\
         \x20 output [63:0] rd,\n\
         \x20 output busy,\n\
         \x20 output [{tags_top}:0] tags, output [{lines_top}:0] vals, output filled, output [{iw_top}:0] fidx,\n\
         \x20 output [{tw_top}:0] ftag,\n\
         \x20 output e_valid, input e_ready, output e_we, output [{am}:0] e_addr, output [7:0] e_len,\n\
         \x20 output [63:0] e_wdata, input e_wdata_take, input [63:0] e_rdata, input e_rdata_valid\n\
         );\n\
         \x20 localparam integer LINES = 1 << IW;\n\
         \x20 localparam integer TW = {ab} - IW - OW;\n\
         \x20 localparam [2:0] C_IDLE = 3'd0, C_PEND = 3'd1, C_WBQ = 3'd2, C_WB = 3'd3, C_FQ = 3'd4, C_FL = 3'd5;\n\
         \x20 reg [2:0] cs;\n\
         \x20 reg [TW-1:0] ctag [0:LINES-1];\n\
         \x20 reg [LINES-1:0] cval, cdirty;\n\
         \x20 reg [31:0] cd_lo [0:(LINES << OW)-1];\n\
         \x20 reg [31:0] cd_hi [0:(LINES << OW)-1];\n\
         \x20 reg [63:0] cr_q;\n\
         \x20 reg mis_we, pr;\n\
         \x20 reg [OW-1:0] wcnt, rcnt;\n\
         \n\
         \x20 wire [IW-1:0] ri = ra[OW +: IW];\n\
         \x20 wire [IW-1:0] wi = wa[OW +: IW];\n\
         \x20 wire hit_r = rhit;\n\
         \x20 wire hit_w = whit;\n\
         \x20 wire [{am}:0] ma = mis_we ? wa : ra;\n\
         \x20 wire [IW-1:0] mi = ma[OW +: IW];\n\
         \n\
         \x20 genvar g;\n\
         \x20 generate for (g = 0; g < LINES; g = g + 1) begin : tag_out\n\
         \x20   assign tags[g * TW +: TW] = ctag[g];\n\
         \x20 end endgenerate\n\
         \x20 assign vals = cval;\n\
         \x20 assign filled = (cs == C_FL) && e_ready;\n\
         \x20 assign fidx = mi;\n\
         \x20 assign ftag = ma[{am} -: TW];\n\
         \n\
         `ifndef SYNTHESIS\n\
         \x20 integer hit_mismatches = 0;\n\
         \x20 always @(posedge clk) if (!rst) begin\n\
         \x20   if ((^ra) !== 1'bx && rhit !== (cval[ri] && ctag[ri] == ra[{am} -: TW])) begin\n\
         \x20     hit_mismatches = hit_mismatches + 1; $display(\"PSRAM-CACHE-HIT-MISMATCH read %m %0t\", $time);\n\
         \x20   end\n\
         \x20   if ((^wa) !== 1'bx && whit !== (cval[wi] && ctag[wi] == wa[{am} -: TW])) begin\n\
         \x20     hit_mismatches = hit_mismatches + 1; $display(\"PSRAM-CACHE-HIT-MISMATCH write %m %0t\", $time);\n\
         \x20   end\n\
         \x20 end\n\
         `endif\n\
         \n\
         \x20 assign busy = (cs != C_IDLE) | (req_w & ~hit_w) | (req_r & ~hit_r) | (req_w & req_r);\n\
         \x20 assign rd = cr_q;\n\
         \x20 assign e_wdata = cr_q;\n\
         \x20 assign e_valid = (cs == C_WBQ) | (cs == C_FQ);\n\
         \x20 assign e_we = (cs == C_WBQ);\n\
         \x20 assign e_addr = (cs == C_WBQ) ? {{ctag[mi], mi, {{OW{{1'b0}}}}}} : {{ma[{am}:OW], {{OW{{1'b0}}}}}};\n\
         \x20 assign e_len = 8'd1 << OW;\n\
         \n\
         \x20 // One read port and one write port on the line store. The read address is the engine's, except\n\
         \x20 // while a victim streams out; the write port takes an engine write hit or a filled word.\n\
         \x20 wire [IW+OW-1:0] cr_a = (cs == C_WBQ || cs == C_WB) ? {{mi, wcnt}} : {{ri, ra[OW-1:0]}};\n\
         \x20 wire hit_wr = cs == C_IDLE && req_w && hit_w;\n\
         \x20 wire fill = cs == C_FL && e_rdata_valid;\n\
         \x20 wire merge = fill && mis_we && rcnt == wa[OW-1:0];\n\
         \x20 wire [IW+OW-1:0] dwa = (cs == C_FL) ? {{mi, rcnt}} : {{wi, wa[OW-1:0]}};\n\
         \x20 wire [31:0] dwd_lo = (fill && !(merge && wm[0])) ? e_rdata[31:0] : wd[31:0];\n\
         \x20 wire [31:0] dwd_hi = (fill && !(merge && wm[1])) ? e_rdata[63:32] : wd[63:32];\n\
         \n\
         \x20 always @(posedge clk) begin\n\
         \x20   cr_q <= {{cd_hi[cr_a], cd_lo[cr_a]}};\n\
         \x20   if (fill || (hit_wr && wm[0])) cd_lo[dwa] <= dwd_lo;\n\
         \x20   if (fill || (hit_wr && wm[1])) cd_hi[dwa] <= dwd_hi;\n\
         \x20 end\n\
         \n\
         \x20 task miss(input we, input [IW-1:0] idx);\n\
         \x20   begin\n\
         \x20     mis_we <= we; rcnt <= {{OW{{1'b0}}}}; wcnt <= {{OW{{1'b0}}}};\n\
         \x20     cs <= (cval[idx] && cdirty[idx]) ? C_WBQ : C_FQ;\n\
         \x20   end\n\
         \x20 endtask\n\
         \n\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin\n\
         \x20     cs <= C_IDLE; cval <= {{LINES{{1'b0}}}}; cdirty <= {{LINES{{1'b0}}}}; mis_we <= 1'b0; pr <= 1'b0;\n\
         \x20   end else case (cs)\n\
         \x20     C_IDLE: if (req_w) begin\n\
         \x20               if (hit_w) begin cdirty[wi] <= 1'b1; if (req_r) cs <= C_PEND; end\n\
         \x20               else begin pr <= req_r; miss(1'b1, wi); end\n\
         \x20             end else if (req_r && !hit_r) miss(1'b0, ri);\n\
         \x20     C_PEND: begin pr <= 1'b0; if (hit_r) cs <= C_IDLE; else miss(1'b0, ri); end\n\
         \x20     C_WBQ:  if (e_ready) cs <= C_WB;\n\
         \x20     C_WB:   begin if (e_wdata_take) wcnt <= wcnt + 1'b1; if (e_ready) cs <= C_FQ; end\n\
         \x20     C_FQ:   if (e_ready) cs <= C_FL;\n\
         \x20     C_FL:   begin\n\
         \x20               if (e_rdata_valid) rcnt <= rcnt + 1'b1;\n\
         \x20               if (e_ready) begin\n\
         \x20                 ctag[mi] <= ma[{am} -: TW]; cval[mi] <= 1'b1; cdirty[mi] <= mis_we;\n\
         \x20                 cs <= pr ? C_PEND : C_IDLE;\n\
         \x20               end\n\
         \x20             end\n\
         \x20     default: cs <= C_IDLE;\n\
         \x20   endcase\n\
         \x20 end\n\
         endmodule\n",
        lines = cache.lines(),
        words = cache.line_words(),
        iw = cache.index_bits,
        ow = cache.offset_bits,
        ab = EDGE_WORD_ADDR_BITS,
        am = EDGE_WORD_ADDR_BITS - 1,
        tags_top = cache.lines() * tw - 1,
        lines_top = cache.lines() - 1,
        iw_top = cache.index_bits - 1,
        tw_top = tw - 1,
    )
}

/// The engine-side lookup of a cache's registered hits: `{inst}_hit(a)`, the hit of address `a` in the cache
/// instance `inst`'s lines as they will hold after this edge — none after a reset, and a fill completing now holding the
/// line it fills — read from the instance's exported `{inst}_tags`, `{inst}_vals`, `{inst}_filled`, `{inst}_fidx` and
/// `{inst}_ftag`. Registering `{inst}_hit(a)` beside an address `a` the engine sets, and `{inst}_hit` of the held
/// address every other cycle, gives the cache exactly the hit it would have looked up itself, a cycle earlier.
pub fn emit_psram_cache_hit_function(inst: &str, cache: &PsramCache) -> String {
    let am = EDGE_WORD_ADDR_BITS - 1;
    let state = |s: &str| format!("{inst}_{s}");
    psram_cache_hit_lookup(
        &format!("{inst}_hit"),
        &format!("input [{am}:0] a"),
        cache,
        [state("tags"), state("vals"), state("filled"), state("fidx"), state("ftag")],
        "rst",
    )
}

/// The lookup [`emit_psram_cache_hit_function`] names, as a function `name(args)` whose last argument is the address
/// `a`, reading a cache's exported `[tags, vals, filled, fidx, ftag]` through the expressions given for them and its
/// reset through `reset`.
fn psram_cache_hit_lookup(name: &str, args: &str, cache: &PsramCache, state: [String; 5], reset: &str) -> String {
    let (iw, ow) = (cache.index_bits, cache.offset_bits);
    let tw = EDGE_WORD_ADDR_BITS - iw - ow;
    let am = EDGE_WORD_ADDR_BITS - 1;
    let [tags, vals, filled, fidx, ftag] = state;
    format!(
        "  function {name}({args});\n\
         \x20   begin\n\
         \x20     if ({reset}) {name} = 1'b0;\n\
         \x20     else if ({filled} && {fidx} == a[{ow} +: {iw}]) {name} = {ftag} == a[{am} -: {tw}];\n\
         \x20     else {name} = {vals}[a[{ow} +: {iw}]] && {tags}[a[{ow} +: {iw}] * {tw} +: {tw}] == a[{am} -: {tw}];\n\
         \x20   end\n\
         \x20 endfunction\n"
    )
}

/// The wires a cache instance `inst` exports to its engine-side lookup ([`emit_psram_cache_hit_function`]), declared.
pub fn emit_psram_cache_hit_wires(inst: &str, cache: &PsramCache) -> String {
    let tw = EDGE_WORD_ADDR_BITS - cache.index_bits - cache.offset_bits;
    format!(
        "  wire [{}:0] {inst}_tags; wire [{}:0] {inst}_vals; wire {inst}_filled; wire [{}:0] {inst}_fidx; wire [{}:0] {inst}_ftag;\n",
        cache.lines() * tw - 1,
        cache.lines() - 1,
        cache.index_bits - 1,
        tw - 1
    )
}

/// The port connections of a cache instance `inst` whose engine registers its hits in `rhit` and `whit`.
pub fn psram_cache_hit_ports(inst: &str, rhit: &str, whit: &str) -> String {
    format!(
        ".rhit({rhit}), .whit({whit}), .tags({inst}_tags), .vals({inst}_vals), .filled({inst}_filled), \
         .fidx({inst}_fidx), .ftag({inst}_ftag)"
    )
}

/// A pin-level model of the part: its semantics (SPI until Enter-QPI, Reset-Enable + Reset before
/// use, bursts wrapping inside a page) and every rule it checks — the timing rules, an SIO bit the part
/// samples that nobody drives (`float`), and a host driving against the part's read data (`contend`) —
/// each counted by name on a `PSRAM-VIOLATION <rule>` line. `violations` holds the total.
pub fn emit_psram_model_verilog(p: &PsramPart) -> String {
    let ns = |ps: u32| format!("{:.3}", ps as f64 / 1000.0);
    let addr_bits = p.capacity_bytes.trailing_zeros();
    let page_bits = p.page_bytes.trailing_zeros();
    let data_read = QPI_HEADER_EDGES + p.read_wait;
    format!(
        "// psram_model — a pin-level model of the {name} ({prov}).\n\
         // Emitted by `cargo run -p logicaffeine-compile --example emit_psram_edge` (codegen_sva::psram).\n\
         `timescale 1ns/1ps\n\
         module psram_model (input ce_n, input sclk, inout [3:0] sio);\n\
         \x20 localparam real T_PU = {t_pu}, T_CEM = {t_cem}, T_CPH = {t_cph}, T_RST = {t_rst}, T_CLK = {t_clk}, T_ACLK = {t_aclk};\n\
         \x20 reg [7:0] mem [0:{top}];\n\
         \x20 reg qpi = 1'b0, armed = 1'b0, ready = 1'b0, wrapped = 1'b0, drive = 1'b0;\n\
         \x20 reg [3:0] dout = 4'b0000;\n\
         \x20 assign sio = drive ? dout : 4'bzzzz;\n\
         \x20 integer violations = 0, nedge = 0;\n\
         \x20 reg [7:0] cmd, wbyte;\n\
         \x20 reg [23:0] addr;\n\
         \x20 realtime t_fall = 0.0, t_rise = -1.0e9, t_rst_end = -1.0e9, t_last = 0.0;\n\
         \n\
         \x20 // An SIO bit nobody drives resolves to z; a driven unknown value is x, and only the first is a fault.\n\
         \x20 function floating(input [3:0] v);\n\
         \x20   floating = (v[0] === 1'bz) || (v[1] === 1'bz) || (v[2] === 1'bz) || (v[3] === 1'bz);\n\
         \x20 endfunction\n\
         \x20 task violation(input [8*8-1:0] rule);\n\
         \x20   begin $display(\"PSRAM-VIOLATION %0s %0t\", rule, $realtime); violations = violations + 1; end\n\
         \x20 endtask\n\
         \n\
         \x20 // The byte `k` bytes into the burst: a burst wraps to the start of its page.\n\
         \x20 function [{abits_m1}:0] at(input [23:0] a, input integer k);\n\
         \x20   reg [{pbits}:0] off;\n\
         \x20   begin off = a[{pbits_m1}:0] + k; at = {{a[{abits_m1}:{pbits}], off[{pbits_m1}:0]}}; end\n\
         \x20 endfunction\n\
         \x20 task note_wrap(input [23:0] a, input integer k);\n\
         \x20   begin if (!wrapped && a[{pbits_m1}:0] + k >= {page}) begin wrapped = 1'b1; violation(\"wrap\"); end end\n\
         \x20 endtask\n\
         \n\
         \x20 always @(negedge ce_n) begin\n\
         \x20   if ($realtime < T_PU) violation(\"tPU\");\n\
         \x20   if ($realtime - t_rise < T_CPH) violation(\"tCPH\");\n\
         \x20   if ($realtime - t_rst_end < T_RST) violation(\"tRST\");\n\
         \x20   t_rst_end = -1.0e9;\n\
         \x20   t_fall = $realtime; nedge = 0; cmd = 8'h00; addr = 24'h0; wrapped = 1'b0;\n\
         \x20 end\n\
         \n\
         \x20 always @(posedge ce_n) begin\n\
         \x20   if ($realtime - t_fall > T_CEM) violation(\"tCEM\");\n\
         \x20   t_rise = $realtime; drive = 1'b0;\n\
         \x20   if (!qpi && nedge == 8) begin\n\
         \x20     if (cmd == 8'h{rst:02x} && armed) begin ready = 1'b1; qpi = 1'b0; t_rst_end = $realtime; end\n\
         \x20     else if (cmd == 8'h{qpi:02x} && ready) qpi = 1'b1;\n\
         \x20     armed = (cmd == 8'h{rsten:02x});\n\
         \x20   end else if (qpi && nedge == 2) begin\n\
         \x20     if (cmd == 8'h{rst:02x} && armed) begin ready = 1'b1; qpi = 1'b0; t_rst_end = $realtime; end\n\
         \x20     armed = (cmd == 8'h{rsten:02x});\n\
         \x20   end else armed = 1'b0;\n\
         \x20 end\n\
         \n\
         \x20 always @(posedge sclk) begin\n\
         \x20   if ($realtime < T_PU) violation(\"tPU\");\n\
         \x20   if (!ce_n) begin\n\
         \x20     if (nedge > 0 && $realtime - t_last < T_CLK) violation(\"tCLK\");\n\
         \x20     t_last = $realtime;\n\
         \x20     if (!qpi && nedge < 8) begin\n\
         \x20       if (sio[0] === 1'bz) violation(\"float\");\n\
         \x20       cmd = {{cmd[6:0], sio[0]}};\n\
         \x20       if (nedge == 7 && !(cmd == 8'h{rsten:02x} || cmd == 8'h{rst:02x} || (cmd == 8'h{qpi:02x} && ready)))\n\
         \x20         violation(ready ? \"cmd\" : \"init\");\n\
         \x20     end else if (qpi && nedge < 2) begin\n\
         \x20       if (floating(sio)) violation(\"float\");\n\
         \x20       cmd = {{cmd[3:0], sio}};\n\
         \x20       if (nedge == 1 && cmd != 8'h{rd:02x} && cmd != 8'h{wr:02x} && cmd != 8'h02 && cmd != 8'h{rsten:02x} && cmd != 8'h{rst:02x})\n\
         \x20         violation(\"cmd\");\n\
         \x20     end else if (qpi) begin\n\
         \x20       if (nedge < {hdr}) begin\n\
         \x20         if (floating(sio)) violation(\"float\");\n\
         \x20         addr = {{addr[19:0], sio}};\n\
         \x20       end else if (cmd == 8'h{wr:02x} || cmd == 8'h02) begin\n\
         \x20         if (floating(sio)) violation(\"float\");\n\
         \x20         if ((nedge - {hdr}) % 2 == 0) wbyte[7:4] = sio;\n\
         \x20         else begin\n\
         \x20           wbyte[3:0] = sio;\n\
         \x20           note_wrap(addr, (nedge - {hdr}) / 2);\n\
         \x20           mem[at(addr, (nedge - {hdr}) / 2)] = wbyte;\n\
         \x20         end\n\
         \x20       end else if (drive && sio !== dout) violation(\"contend\");\n\
         \x20     end\n\
         \x20     nedge = nedge + 1;\n\
         \x20   end\n\
         \x20 end\n\
         \n\
         \x20 // Read data: nibble n follows the falling edge after rising edge {data_read} + n, valid tACLK later.\n\
         \x20 always @(negedge sclk) begin\n\
         \x20   if (!ce_n && qpi && cmd == 8'h{rd:02x} && nedge >= {data_read}) begin : drive_nibble\n\
         \x20     reg [7:0] b; integer n;\n\
         \x20     n = nedge - {data_read};\n\
         \x20     note_wrap(addr, n / 2);\n\
         \x20     b = mem[at(addr, n / 2)];\n\
         \x20     #(T_ACLK);\n\
         \x20     if (!ce_n) begin dout = (n % 2 == 0) ? b[7:4] : b[3:0]; drive = 1'b1; end\n\
         \x20   end\n\
         \x20 end\n\
         endmodule\n",
        name = p.name,
        prov = p.provenance,
        t_pu = ns(p.t_pu_min_ps),
        t_cem = ns(p.t_cem_max_ps),
        t_cph = ns(p.t_cph_min_ps),
        t_rst = ns(p.t_rst_min_ps),
        t_clk = ns(p.t_clk_min_ps),
        t_aclk = ns(p.t_aclk_max_ps),
        top = p.capacity_bytes - 1,
        abits_m1 = addr_bits - 1,
        pbits = page_bits,
        pbits_m1 = page_bits - 1,
        page = p.page_bytes,
        rsten = p.reset_enable,
        rst = p.reset,
        qpi = p.enter_qpi,
        rd = p.read_cmd,
        wr = p.write_cmd,
        hdr = QPI_HEADER_EDGES,
    )
}

/// Several caches share one edge. When the edge is idle the lowest-numbered requester is granted and its
/// request goes to the edge; that cache owns the edge until the burst completes and sees `c_ready` exactly
/// once more when it does, in a cycle in which no other cache is granted. The edge's write-data takes and
/// read strobes go to every cache: only the owner is ever in a burst state, the only states that listen.
pub fn emit_psram_arbiter_verilog() -> String {
    format!(
        "// psram_arbiter — N psram_cache instances share one psram_edge (codegen_sva::psram).\n\
         module psram_arbiter #(parameter integer N = 1, parameter integer AW = {ab}) (\n\
         \x20 input clk, input rst,\n\
         \x20 input  [N-1:0] c_valid, output [N-1:0] c_ready, input [N-1:0] c_we,\n\
         \x20 input  [N*AW-1:0] c_addr, input [N*8-1:0] c_len, input [N*64-1:0] c_wdata,\n\
         \x20 output e_valid, input e_ready, output e_we, output [AW-1:0] e_addr, output [7:0] e_len, output [63:0] e_wdata\n\
         );\n\
         \x20 reg pend;\n\
         \x20 reg [7:0] owner;\n\
         \x20 reg [7:0] pick;\n\
         \x20 reg any;\n\
         \x20 integer i;\n\
         \x20 always @* begin\n\
         \x20   any = 1'b0; pick = 8'd0;\n\
         \x20   for (i = N - 1; i >= 0; i = i - 1) if (c_valid[i]) begin any = 1'b1; pick = i; end\n\
         \x20 end\n\
         \x20 wire [7:0] sel = pend ? owner : pick;\n\
         \x20 assign e_valid = !pend && any;\n\
         \x20 assign e_we = c_we[sel];\n\
         \x20 assign e_addr = c_addr[sel*AW +: AW];\n\
         \x20 assign e_len = c_len[sel*8 +: 8];\n\
         \x20 assign e_wdata = c_wdata[sel*64 +: 64];\n\
         \x20 genvar g;\n\
         \x20 generate for (g = 0; g < N; g = g + 1) begin : grant\n\
         \x20   assign c_ready[g] = e_ready && (pend ? owner == g : (any && pick == g));\n\
         \x20 end endgenerate\n\
         \x20 always @(posedge clk) begin\n\
         \x20   if (rst) begin pend <= 1'b0; owner <= 8'd0; end\n\
         \x20   else if (pend) begin if (e_ready) pend <= 1'b0; end\n\
         \x20   else if (e_valid && e_ready) begin pend <= 1'b1; owner <= pick; end\n\
         \x20 end\n\
         endmodule\n",
        ab = EDGE_WORD_ADDR_BITS,
    )
}

/// A randomized testbench for the cache: an engine-shaped driver that issues nothing while `busy` and
/// otherwise a read (then waits a cycle and checks `rd` against a reference memory), a write of one or
/// both halves of a word, or a write and a read together. Once the edge has initialized the part it writes every word it will use — `words`
/// consecutive from 0 and `words` scattered over the whole part — then runs `ops` random operations.
/// Prints `CACHE-ERRORS`, `PSRAM-VIOLATIONS`, the longest `busy` run as `CACHE-MAX-STALL`, the rising edge
/// after reset at which `req_ready` was first seen as `EDGE-READY-AT`, then `CACHE-DONE`. The clock runs at
/// `run_period_ps`, anywhere in the edge's range.
pub fn emit_psram_cache_testbench(
    edge: &PsramEdge,
    cache: &PsramCache,
    run_period_ps: u32,
    seed: u32,
    words: u32,
    ops: u32,
) -> String {
    let period_ns = run_period_ps as f64 / 1000.0;
    let budget_ns = (edge.init_cycles() as f64
        + (2 * words + ops) as f64 * (cache.worst_stall_cycles(edge) as f64 + 4.0))
        * period_ns
        * 2.0;
    format!(
        "`timescale 1ns/1ps\n\
         module tb;\n\
         \x20 localparam integer NADDR = {words};\n\
         \x20 localparam integer M = {ops};\n\
         \x20 reg clk = 1'b0; always #{half_ns} clk = ~clk;\n\
         \x20 reg rst = 1'b1;\n\
         \x20 reg req_r = 1'b0, req_w = 1'b0; reg [{am}:0] ra = 0, wa = 0; reg [63:0] wd = 64'd0; reg [1:0] wm = 2'b11;\n\
         \x20 // The engine's half of the hits: each registered beside the address it is the hit of.\n\
         \x20 reg rhit = 1'b0, whit = 1'b0;\n\
         \x20 // The cache's own reset: the bench's, and one cycle once the traffic is done, with lines valid.\n\
         \x20 reg rst_pulse = 1'b0;\n\
         \x20 wire crst = rst | rst_pulse;\n\
         {hit_wires}\
         \x20 wire [63:0] rd; wire busy;\n\
         \x20 wire e_valid, e_ready, e_we, e_take, e_rv; wire [{am}:0] e_addr; wire [7:0] e_len; wire [63:0] e_wdata, e_rdata;\n\
         \x20 wire ce_n, sclk; wire [3:0] sio_out, sio_oe; wire [3:0] sio;\n\
         \x20 genvar gi;\n\
         \x20 generate for (gi = 0; gi < 4; gi = gi + 1) begin : pads\n\
         \x20   assign sio[gi] = sio_oe[gi] ? sio_out[gi] : 1'bz;\n\
         \x20 end endgenerate\n\
         \x20 psram_cache cache(.clk(clk), .rst(crst), .req_r(req_r), .ra(ra), .req_w(req_w), .wa(wa), .wd(wd), .wm(wm),\n\
         \x20   {hit_ports},\n\
         \x20   .rd(rd), .busy(busy), .e_valid(e_valid), .e_ready(e_ready), .e_we(e_we), .e_addr(e_addr), .e_len(e_len),\n\
         \x20   .e_wdata(e_wdata), .e_wdata_take(e_take), .e_rdata(e_rdata), .e_rdata_valid(e_rv));\n\
         \x20 psram_edge edge_u(.clk(clk), .rst(rst), .req_valid(e_valid), .req_ready(e_ready), .req_we(e_we),\n\
         \x20   .req_addr(e_addr), .req_len(e_len), .wdata(e_wdata), .wdata_take(e_take), .rdata(e_rdata),\n\
         \x20   .rdata_valid(e_rv), .ce_n(ce_n), .sclk(sclk), .sio_out(sio_out), .sio_oe(sio_oe), .sio_in(sio));\n\
         \x20 psram_model part(.ce_n(ce_n), .sclk(sclk), .sio(sio));\n\
         {hit_fn}\
         \n\
         \x20 // Word k < NADDR is address k; k >= NADDR is scattered over the whole part.\n\
         \x20 function [{am}:0] addr_of(input integer k);\n\
         \x20   reg [31:0] s;\n\
         \x20   begin\n\
         \x20     if (k < NADDR) addr_of = k;\n\
         \x20     else begin s = (k - NADDR) * 40503 + (32'd1 << {am}); addr_of = s[{am}:0]; end\n\
         \x20   end\n\
         \x20 endfunction\n\
         \x20 // Both halves of a written word vary with the draw, so a half that should have kept its old value\n\
         \x20 // cannot pass by coinciding with the new one.\n\
         \x20 function [63:0] mix(input [31:0] a, input [31:0] b);\n\
         \x20   mix = {{a ^ b ^ 32'h9E3779B9, b * 32'd2654435761}};\n\
         \x20 endfunction\n\
         \n\
         \x20 reg [63:0] refm [0:2*NADDR-1];\n\
         \x20 reg [1:0] m;\n\
         \x20 // A write of the enabled halves of `v`, to the part through the cache and to the reference memory.\n\
         \x20 task put(input integer kk, input [63:0] v, input [1:0] mask);\n\
         \x20   begin\n\
         \x20     if (mask[0]) refm[kk][31:0] = v[31:0];\n\
         \x20     if (mask[1]) refm[kk][63:32] = v[63:32];\n\
         \x20     wa <= addr_of(kk); whit <= cache_hit(addr_of(kk)); wd <= v; wm <= mask; req_w <= 1'b1;\n\
         \x20   end\n\
         \x20 endtask\n\
         \x20 reg [31:0] lcg;\n\
         \x20 reg [1:0] tst;\n\
         \x20 reg [63:0] expv;\n\
         \x20 integer k, k2, op, init, done_ops, errors, run, max_run, edges, ready_at, fin;\n\
         \x20 always @(posedge clk) if (!rst) begin\n\
         \x20   edges = edges + 1;\n\
         \x20   if (e_ready && ready_at < 0) ready_at = edges;\n\
         \x20   if (busy) begin run = run + 1; if (run > max_run) max_run = run; end else run = 0;\n\
         \x20 end\n\
         \x20 always @(posedge clk) begin\n\
         \x20   req_r <= 1'b0; req_w <= 1'b0;\n\
         \x20   rhit <= cache_hit(ra); whit <= cache_hit(wa);\n\
         \x20   if (!rst && !busy && ready_at >= 0) begin\n\
         \x20     case (tst)\n\
         \x20       2'd0: begin\n\
         \x20         lcg = lcg * 32'd1664525 + 32'd1013904223;\n\
         \x20         if (init < 2 * NADDR) begin\n\
         \x20           put(init, mix(init, lcg), 2'b11); init = init + 1;\n\
         \x20         end else if (done_ops < M) begin\n\
         \x20           done_ops = done_ops + 1;\n\
         \x20           k = (lcg >> 8) % (2 * NADDR); k2 = (lcg >> 17) % (2 * NADDR); op = lcg[31:28];\n\
         \x20           m = lcg[27:26] == 2'b00 ? 2'b11 : lcg[27:26];\n\
         \x20           if (op < 7) begin\n\
         \x20             put(k, mix(k, lcg), m);\n\
         \x20           end else if (op < 14) begin\n\
         \x20             ra <= addr_of(k); rhit <= cache_hit(addr_of(k)); expv <= refm[k]; req_r <= 1'b1; tst <= 2'd1;\n\
         \x20           end else begin\n\
         \x20             put(k, mix(k, lcg ^ 32'hA5A5), m);\n\
         \x20             if (op == 15) k2 = k;\n\
         \x20             ra <= addr_of(k2); rhit <= cache_hit(addr_of(k2)); expv <= refm[k2]; req_r <= 1'b1; tst <= 2'd1;\n\
         \x20           end\n\
         \x20         end else if (fin < 3) begin\n\
         \x20           rst_pulse <= (fin == 0); fin = fin + 1;\n\
         \x20         end else begin\n\
         \x20           $display(\"CACHE-ERRORS %0d\", errors);\n\
         \x20           $display(\"PSRAM-VIOLATIONS %0d\", part.violations);\n\
         \x20           $display(\"CACHE-MAX-STALL %0d\", max_run);\n\
         \x20           $display(\"EDGE-READY-AT %0d\", ready_at);\n\
         \x20           $display(\"CACHE-HIT-MISMATCHES %0d\", cache.hit_mismatches);\n\
         \x20           $display(\"CACHE-DONE\");\n\
         \x20           $finish;\n\
         \x20         end\n\
         \x20       end\n\
         \x20       2'd1: tst <= 2'd2;\n\
         \x20       2'd2: begin\n\
         \x20         if (rd !== expv) begin\n\
         \x20           errors = errors + 1;\n\
         \x20           if (errors < 10) $display(\"MISMATCH addr=%h got=%016h exp=%016h\", ra, rd, expv);\n\
         \x20         end\n\
         \x20         tst <= 2'd0;\n\
         \x20       end\n\
         \x20       default: tst <= 2'd0;\n\
         \x20     endcase\n\
         \x20   end\n\
         \x20 end\n\
         \x20 initial begin\n\
         \x20   lcg = 32'd{seed}; tst = 2'd0; fin = 0; init = 0; done_ops = 0; errors = 0; run = 0; max_run = 0; edges = 0; ready_at = -1;\n\
         \x20   repeat (4) @(posedge clk); rst = 1'b0;\n\
         \x20 end\n\
         \x20 initial begin #{budget:.0}; $display(\"CACHE-TIMEOUT\"); $display(\"CACHE-ERRORS %0d\", errors); $finish; end\n\
         endmodule\n",
        half_ns = period_ns / 2.0,
        am = EDGE_WORD_ADDR_BITS - 1,
        budget = budget_ns + 10_000.0,
        hit_wires = emit_psram_cache_hit_wires("cache", cache),
        hit_ports = psram_cache_hit_ports("cache", "rhit", "whit"),
        hit_fn = psram_cache_hit_lookup(
            "cache_hit",
            &format!("input [{}:0] a", EDGE_WORD_ADDR_BITS - 1),
            cache,
            ["cache_tags", "cache_vals", "cache_filled", "cache_fidx", "cache_ftag"].map(String::from),
            "crst",
        ),
    )
}

/// A randomized testbench for caches sharing the edge through the arbiter: `n` caches, each over its own
/// region of the part, driven by one engine-shaped driver that issues to any subset of them in the same
/// cycle — a read, a write, or a write and a read each — and nothing while any is `busy`. Prints
/// `MULTI-ERRORS`, `PSRAM-VIOLATIONS`, the longest stall as `MULTI-MAX-STALL`, then `MULTI-DONE`.
pub fn emit_psram_arbiter_testbench(
    edge: &PsramEdge,
    cache: &PsramCache,
    n: u32,
    seed: u32,
    words: u32,
    ops: u32,
) -> String {
    let period_ns = edge.sys_period_ps as f64 / 1000.0;
    let budget_ns = (edge.init_cycles() as f64
        + (words + ops) as f64 * n as f64 * (cache.worst_stall_cycles(edge) as f64 + 8.0))
        * period_ns
        * 2.0;
    let region_bits = EDGE_WORD_ADDR_BITS - (32 - (n.max(2) - 1).leading_zeros());
    let tw = EDGE_WORD_ADDR_BITS - cache.index_bits - cache.offset_bits;
    format!(
        "`timescale 1ns/1ps\n\
         module tb;\n\
         \x20 localparam integer N = {n};\n\
         \x20 localparam integer NADDR = {words};\n\
         \x20 localparam integer M = {ops};\n\
         \x20 reg clk = 1'b0; always #{half_ns} clk = ~clk;\n\
         \x20 reg rst = 1'b1;\n\
         \x20 reg [N-1:0] req_r = 0, req_w = 0;\n\
         \x20 reg [{am}:0] ra [0:N-1]; reg [{am}:0] wa [0:N-1]; reg [63:0] wd [0:N-1];\n\
         \x20 wire [63:0] rd [0:N-1];\n\
         \x20 wire [N-1:0] busy;\n\
         \x20 // The engine's half of each cache's hits, registered beside the address each is the hit of.\n\
         \x20 reg [N-1:0] rhit = 0, whit = 0;\n\
         \x20 wire [{tags_top}:0] c_tags [0:N-1]; wire [{lines_top}:0] c_vals [0:N-1]; wire [N-1:0] c_filled;\n\
         \x20 wire [{iw_top}:0] c_fidx [0:N-1]; wire [{tw_top}:0] c_ftag [0:N-1];\n\
         \x20 wire [N-1:0] c_valid, c_ready, c_we;\n\
         \x20 wire [N*{aw}-1:0] c_addr; wire [N*8-1:0] c_len; wire [N*64-1:0] c_wdata;\n\
         \x20 wire e_valid, e_ready, e_we, e_take, e_rv; wire [{am}:0] e_addr; wire [7:0] e_len; wire [63:0] e_wdata, e_rdata;\n\
         \x20 wire ce_n, sclk; wire [3:0] sio_out, sio_oe; wire [3:0] sio;\n\
         \x20 genvar gi;\n\
         \x20 generate for (gi = 0; gi < 4; gi = gi + 1) begin : pads\n\
         \x20   assign sio[gi] = sio_oe[gi] ? sio_out[gi] : 1'bz;\n\
         \x20 end endgenerate\n\
         \x20 // Cache g owns the region whose high bits are g.\n\
         \x20 generate for (gi = 0; gi < N; gi = gi + 1) begin : caches\n\
         \x20   wire [{am}:0] a_r = ra[gi] | (gi << {region_bits});\n\
         \x20   wire [{am}:0] a_w = wa[gi] | (gi << {region_bits});\n\
         \x20   psram_cache c(.clk(clk), .rst(rst), .req_r(req_r[gi]), .ra(a_r), .req_w(req_w[gi]), .wa(a_w), .wd(wd[gi]),\n\
         \x20     .rhit(rhit[gi]), .whit(whit[gi]), .tags(c_tags[gi]), .vals(c_vals[gi]), .filled(c_filled[gi]),\n\
         \x20     .fidx(c_fidx[gi]), .ftag(c_ftag[gi]),\n\
         \x20     .wm(2'b11), .rd(rd[gi]), .busy(busy[gi]), .e_valid(c_valid[gi]), .e_ready(c_ready[gi]), .e_we(c_we[gi]),\n\
         \x20     .e_addr(c_addr[gi*{aw} +: {aw}]), .e_len(c_len[gi*8 +: 8]), .e_wdata(c_wdata[gi*64 +: 64]),\n\
         \x20     .e_wdata_take(e_take), .e_rdata(e_rdata), .e_rdata_valid(e_rv));\n\
         \x20 end endgenerate\n\
         \x20 psram_arbiter #(.N(N)) arb(.clk(clk), .rst(rst), .c_valid(c_valid), .c_ready(c_ready), .c_we(c_we),\n\
         \x20   .c_addr(c_addr), .c_len(c_len), .c_wdata(c_wdata),\n\
         \x20   .e_valid(e_valid), .e_ready(e_ready), .e_we(e_we), .e_addr(e_addr), .e_len(e_len), .e_wdata(e_wdata));\n\
         \x20 psram_edge edge_u(.clk(clk), .rst(rst), .req_valid(e_valid), .req_ready(e_ready), .req_we(e_we),\n\
         \x20   .req_addr(e_addr), .req_len(e_len), .wdata(e_wdata), .wdata_take(e_take), .rdata(e_rdata),\n\
         \x20   .rdata_valid(e_rv), .ce_n(ce_n), .sclk(sclk), .sio_out(sio_out), .sio_oe(sio_oe), .sio_in(sio));\n\
         \x20 psram_model part(.ce_n(ce_n), .sclk(sclk), .sio(sio));\n\
         {hit_fn}\
         \n\
         \x20 reg [63:0] refm [0:N*NADDR-1];\n\
         \x20 reg [63:0] expv [0:N-1];\n\
         \x20 reg [N-1:0] chk;\n\
         \x20 reg [31:0] lcg;\n\
         \x20 reg [1:0] tst;\n\
         \x20 integer c, k, op, init, done_ops, errors, run, max_run, ready_at, edges;\n\
         \x20 wire any_busy = |busy;\n\
         \x20 always @(posedge clk) if (!rst) begin\n\
         \x20   edges = edges + 1;\n\
         \x20   if (e_ready && ready_at < 0) ready_at = edges;\n\
         \x20   if (any_busy) begin run = run + 1; if (run > max_run) max_run = run; end else run = 0;\n\
         \x20 end\n\
         \x20 always @(posedge clk) begin\n\
         \x20   req_r <= 0; req_w <= 0;\n\
         \x20   for (c = 0; c < N; c = c + 1) begin\n\
         \x20     rhit[c] <= hit_of(c, ra[c] | (c << {region_bits})); whit[c] <= hit_of(c, wa[c] | (c << {region_bits}));\n\
         \x20   end\n\
         \x20   if (!rst && !any_busy && ready_at >= 0) begin\n\
         \x20     case (tst)\n\
         \x20       2'd0: begin\n\
         \x20         if (init < NADDR) begin\n\
         \x20           for (c = 0; c < N; c = c + 1) begin\n\
         \x20             lcg = lcg * 32'd1664525 + 32'd1013904223;\n\
         \x20             refm[c*NADDR + init] = {{lcg, lcg ^ 32'h5555AAAA}};\n\
         \x20             wa[c] <= init; whit[c] <= hit_of(c, init | (c << {region_bits})); wd[c] <= refm[c*NADDR + init]; req_w[c] <= 1'b1;\n\
         \x20           end\n\
         \x20           init = init + 1;\n\
         \x20         end else if (done_ops < M) begin\n\
         \x20           done_ops = done_ops + 1; chk = 0;\n\
         \x20           for (c = 0; c < N; c = c + 1) begin\n\
         \x20             lcg = lcg * 32'd1664525 + 32'd1013904223;\n\
         \x20             op = lcg[31:29]; k = (lcg >> 8) % NADDR;\n\
         \x20             if (op < 3) begin\n\
         \x20               refm[c*NADDR + k] = {{lcg ^ c[31:0], lcg * 32'd3}}; wa[c] <= k; whit[c] <= hit_of(c, k | (c << {region_bits})); wd[c] <= refm[c*NADDR + k]; req_w[c] <= 1'b1;\n\
         \x20             end else if (op < 6) begin\n\
         \x20               ra[c] <= k; rhit[c] <= hit_of(c, k | (c << {region_bits})); expv[c] <= refm[c*NADDR + k]; req_r[c] <= 1'b1; chk[c] = 1'b1;\n\
         \x20             end else if (op == 6) begin\n\
         \x20               refm[c*NADDR + k] = {{lcg, ~lcg}}; wa[c] <= k; whit[c] <= hit_of(c, k | (c << {region_bits})); wd[c] <= refm[c*NADDR + k]; req_w[c] <= 1'b1;\n\
         \x20               k = (lcg >> 20) % NADDR;\n\
         \x20               ra[c] <= k; rhit[c] <= hit_of(c, k | (c << {region_bits})); expv[c] <= refm[c*NADDR + k]; req_r[c] <= 1'b1; chk[c] = 1'b1;\n\
         \x20             end\n\
         \x20           end\n\
         \x20           if (chk != 0) tst <= 2'd1;\n\
         \x20         end else begin\n\
         \x20           $display(\"MULTI-ERRORS %0d\", errors);\n\
         \x20           $display(\"PSRAM-VIOLATIONS %0d\", part.violations);\n\
         \x20           $display(\"MULTI-MAX-STALL %0d\", max_run);\n\
         \x20           $display(\"MULTI-DONE\");\n\
         \x20           $finish;\n\
         \x20         end\n\
         \x20       end\n\
         \x20       2'd1: tst <= 2'd2;\n\
         \x20       2'd2: begin\n\
         \x20         for (c = 0; c < N; c = c + 1) if (chk[c] && rd[c] !== expv[c]) begin\n\
         \x20           errors = errors + 1;\n\
         \x20           if (errors < 10) $display(\"MISMATCH cache=%0d addr=%h got=%016h exp=%016h\", c, ra[c], rd[c], expv[c]);\n\
         \x20         end\n\
         \x20         tst <= 2'd0;\n\
         \x20       end\n\
         \x20       default: tst <= 2'd0;\n\
         \x20     endcase\n\
         \x20   end\n\
         \x20 end\n\
         \x20 initial begin\n\
         \x20   lcg = 32'd{seed}; tst = 2'd0; init = 0; done_ops = 0; errors = 0; run = 0; max_run = 0; ready_at = -1; edges = 0;\n\
         \x20   for (c = 0; c < N; c = c + 1) begin ra[c] = 0; wa[c] = 0; wd[c] = 0; end\n\
         \x20   repeat (4) @(posedge clk); rst = 1'b0;\n\
         \x20 end\n\
         \x20 initial begin #{budget:.0}; $display(\"MULTI-TIMEOUT\"); $display(\"MULTI-ERRORS %0d\", errors); $finish; end\n\
         endmodule\n",
        half_ns = period_ns / 2.0,
        am = EDGE_WORD_ADDR_BITS - 1,
        aw = EDGE_WORD_ADDR_BITS,
        budget = budget_ns + 10_000.0,
        tags_top = cache.lines() * tw - 1,
        lines_top = cache.lines() - 1,
        iw_top = cache.index_bits - 1,
        tw_top = tw - 1,
        hit_fn = psram_cache_hit_lookup(
            "hit_of",
            &format!("input integer g, input [{}:0] a", EDGE_WORD_ADDR_BITS - 1),
            cache,
            ["c_tags[g]", "c_vals[g]", "c_filled[g]", "c_fidx[g]", "c_ftag[g]"].map(String::from),
            "rst",
        ),
    )
}

/// Controller parameters a test may override, to drive the part out of its rules on purpose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EdgeOverrides {
    pub powerup_cycles: Option<u32>,
    pub cph_cycles: Option<u32>,
    pub rst_cycles: Option<u32>,
}

/// A testbench: the controller and the model, the bursts issued in order (write data taken from
/// `wdata` in order), every word read printed as `RD <hex>`, every burst's cycles from acceptance to
/// the next `req_ready` as `BURST <n>`, then the model's total and `PSRAM-DONE`.
pub fn emit_psram_edge_testbench(
    edge: &PsramEdge,
    over: &EdgeOverrides,
    bursts: &[PsramBurst],
    wdata: &[u64],
) -> String {
    let period_ns = edge.sys_period_ps as f64 / 1000.0;
    let mut words = String::new();
    for (i, w) in wdata.iter().enumerate() {
        let _ = writeln!(words, "    wmem[{i}] = 64'h{w:016x};");
    }
    let mut calls = String::new();
    for b in bursts {
        let _ = writeln!(
            calls,
            "    burst({}, 20'h{:05x}, 8'd{});",
            u8::from(b.write),
            b.word_addr,
            b.words
        );
    }
    let sim_budget_ns: f64 = (edge.powerup_cycles() as f64 + 100.0) * period_ns
        + bursts
            .iter()
            .map(|b| edge.burst_sys_cycles(b.write, b.words) as f64 + 8.0)
            .sum::<f64>()
            * period_ns
            * 2.0;
    format!(
        "`timescale 1ns/1ps\n\
         module tb;\n\
         \x20 reg clk = 1'b0; always #{half_ns} clk = ~clk;\n\
         \x20 reg rst = 1'b1;\n\
         \x20 reg req_valid = 1'b0, req_we = 1'b0; reg [19:0] req_addr = 20'h0; reg [7:0] req_len = 8'd0;\n\
         \x20 reg [63:0] wdata = 64'h0; wire wdata_take, req_ready, rdata_valid; wire [63:0] rdata;\n\
         \x20 wire ce_n, sclk; wire [3:0] sio_out, sio_oe; wire [3:0] sio;\n\
         \x20 genvar gi;\n\
         \x20 generate for (gi = 0; gi < 4; gi = gi + 1) begin : pads\n\
         \x20   assign sio[gi] = sio_oe[gi] ? sio_out[gi] : 1'bz;\n\
         \x20 end endgenerate\n\
         \x20 psram_edge #(.H({half}), .POWERUP({pu}), .CPH({cph}), .RSTGAP({rstgap})) dut(\n\
         \x20   .clk(clk), .rst(rst), .req_valid(req_valid), .req_ready(req_ready), .req_we(req_we), .req_addr(req_addr),\n\
         \x20   .req_len(req_len), .wdata(wdata), .wdata_take(wdata_take), .rdata(rdata), .rdata_valid(rdata_valid),\n\
         \x20   .ce_n(ce_n), .sclk(sclk), .sio_out(sio_out), .sio_oe(sio_oe), .sio_in(sio));\n\
         \x20 psram_model part(.ce_n(ce_n), .sclk(sclk), .sio(sio));\n\
         \x20 reg [63:0] wmem [0:{wlast}];\n\
         \x20 integer wi = 0, n;\n\
         \x20 always @(negedge clk) begin\n\
         \x20   if (rdata_valid) $display(\"RD %016h\", rdata);\n\
         \x20   if (wdata_take) begin wi = wi + 1; wdata = wmem[wi]; end\n\
         \x20 end\n\
         \x20 task burst(input we, input [19:0] a, input [7:0] len);\n\
         \x20   begin\n\
         \x20     @(negedge clk); while (!req_ready) @(negedge clk);\n\
         \x20     req_valid = 1'b1; req_we = we; req_addr = a; req_len = len;\n\
         \x20     @(posedge clk); n = 0;\n\
         \x20     @(negedge clk); req_valid = 1'b0;\n\
         \x20     while (!req_ready) begin @(posedge clk); n = n + 1; @(negedge clk); end\n\
         \x20     $display(\"BURST %0d\", n);\n\
         \x20   end\n\
         \x20 endtask\n\
         \x20 initial begin\n\
         {words}\
         \x20   wdata = wmem[0];\n\
         \x20   repeat (4) @(posedge clk); rst = 1'b0;\n\
         {calls}\
         \x20   repeat (4) @(posedge clk);\n\
         \x20   $display(\"PSRAM-VIOLATIONS %0d\", part.violations);\n\
         \x20   $display(\"PSRAM-DONE\");\n\
         \x20   $finish;\n\
         \x20 end\n\
         \x20 initial begin #{budget:.0}; $display(\"PSRAM-TIMEOUT\"); $display(\"PSRAM-VIOLATIONS %0d\", part.violations); $finish; end\n\
         endmodule\n",
        half_ns = period_ns / 2.0,
        half = edge.half,
        pu = over.powerup_cycles.unwrap_or(edge.powerup_cycles()),
        cph = over.cph_cycles.unwrap_or(edge.cph_cycles()),
        rstgap = over.rst_cycles.unwrap_or(edge.rst_cycles()),
        wlast = wdata.len().max(1) - 1,
        budget = sim_budget_ns + 10_000.0,
    )
}
