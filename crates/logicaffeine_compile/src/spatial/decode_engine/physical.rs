//! The card's memory as its four channels hold it: where every byte of a machine layout (`super::machine::Layout`)
//! lives, and the streams a job's reads and writes of one sequence's state are on each channel.
//!
//! The tensors are the weight image's ([`super::image::image_of`]): each tensor's tiles dealt to the channels from line
//! 0, streamed by tensor and row. Everything the layout places after them — the rotary tables, then every sequence's
//! region — is one logical space of 64-byte lines, cut into granules of [`PhysicalMap::granule`] lines (the engine
//! plan's state chunk, one value head's memory) dealt round the channels: granule `g` goes to channel `g mod 4`, at
//! that channel's local line `(g / 4) · G` plus the line's place in the granule, from line `B`, the first line past the
//! weight image on every channel. At 64 data bytes a beat a local line is a channel line, `B` plus the local line; at
//! all 72 each channel's local lines are packed end to end, local line `i` at byte `72 · B + 64 · i`, nine logical
//! lines to eight of the channel's, so a granule's first and last bytes may share a channel line with the channel's
//! granule before or after it.
//!
//! A run of logical bytes is one run of channel bytes on each channel it touches ([`PhysicalMap::runs`]): the granules
//! of one channel within the run are consecutive local lines. The four patterns a job streams of a sequence — a conv
//! ring's row, one value head's memory, a lazily flushed memory's log entries, an attention block's cache rows of
//! positions `0..=p` with their scales — are runs of the layout's own addresses ([`PhysicalMap::ring_row`],
//! [`PhysicalMap::memory`], [`PhysicalMap::log`], [`PhysicalMap::cache`]), and each run a stream the DDR4 model prices
//! ([`Run::stream`]).
//!
//! The engine reaches the space through two ports that place every logical line as [`PhysicalMap::locate`] does: the
//! card port ([`card_port_verilog`]), a line a request for the engine's core, and the fast port
//! ([`fast_port_verilog`]), whole lines in runs a descriptor for a hybrid engine's fast side.

use std::ops::Range;

use crate::codegen_sva::dram::StreamDesc;
use crate::spatial::d5005::{Beat, EnginePlan};

use super::image::{image_of, streamed_image_of, weight_image, Image, ImageError, ImageLayout, TensorTable};
use super::machine::{BlockState, Layout as MachineLayout, LINE};
use super::model::{CacheFormat, StateFormat};
use super::rtl::streaming::{StreamEntry, STREAM_ENTRY_LINES};

/// Bytes of one cached row's int8 scale.
const SCALE_BYTES: u64 = 2;

/// Logical bytes `[first, first + bytes)` of the space after the weights as one channel holds them: from channel byte
/// `at` on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub channel: u32,
    pub at: u64,
    pub bytes: u64,
}

impl Run {
    /// The channel lines the run touches, `line_bytes` a line.
    pub fn lines(&self, line_bytes: u64) -> Range<u64> {
        self.at / line_bytes..(self.at + self.bytes).div_ceil(line_bytes)
    }

    /// The run as one stream of its channel: a read, or a write when `write`.
    pub fn stream(&self, write: bool) -> StreamDesc {
        StreamDesc { base: self.at, elem_bytes: self.bytes, stride_bytes: self.bytes, count: 1, write }
    }
}

/// The machine's memory without the matrices its matvec jobs stream: each streamed tensor's bytes, to the next line,
/// taken out, and every other byte moved down by the bytes taken out below it. The hybrid engine's flat space holds the
/// memory so, and after it each streamed tensor's stream entry ([`StreamEntry`], [`STREAM_ENTRY_LINES`] lines) in the
/// runs' order; the tables give the reference engine every address placed so, a streamed tensor's its entry's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compaction {
    /// The runs taken out, `[start, end)` in order.
    pub runs: Vec<(u64, u64)>,
    /// The compacted memory's bytes to the next line: where the stream entries start.
    pub entries: u64,
}

impl Compaction {
    /// The runs of `machine`'s streamed tensors.
    pub fn of(machine: &MachineLayout) -> Compaction {
        let runs: Vec<(u64, u64)> = machine.tensors.iter().filter(|t| t.streamed).map(|t| (t.at, (t.at + t.bytes).div_ceil(LINE) * LINE)).collect();
        let removed: u64 = runs.iter().map(|(s, e)| e - s).sum();
        Compaction { entries: (machine.bytes - removed).div_ceil(LINE) * LINE, runs }
    }

    /// The flat space's bytes: the compacted memory and the stream entries.
    pub fn space_bytes(&self) -> u64 {
        self.entries + self.runs.len() as u64 * STREAM_ENTRY_LINES * LINE
    }

    /// Where the hybrid engine finds what logical byte `at` holds: its compacted place, or for a streamed tensor's
    /// first byte the tensor's stream entry; none for the rest of a streamed tensor's bytes.
    pub fn find(&self, at: u64) -> Option<u64> {
        self.place(at).or_else(|| self.runs.iter().position(|&(s, _)| s == at).map(|k| self.entries + k as u64 * STREAM_ENTRY_LINES * LINE))
    }

    /// The flat space over `memory`: the memory compacted, zeros to the next line, then the streamed tensors'
    /// `entries`, one a run in the runs' order.
    pub fn space(&self, memory: &[u8], entries: &[StreamEntry]) -> Vec<u8> {
        assert_eq!(entries.len(), self.runs.len(), "an entry a streamed tensor");
        let mut out = self.bytes(memory);
        out.resize(self.entries as usize, 0);
        for e in entries {
            out.extend(e.lines().concat());
        }
        out
    }

    /// The bytes taken out.
    pub fn removed(&self) -> u64 {
        self.runs.iter().map(|(s, e)| e - s).sum()
    }

    /// Where logical byte `at` lies in the compacted memory: none for a byte of a run taken out.
    pub fn place(&self, at: u64) -> Option<u64> {
        if self.runs.iter().any(|&(s, e)| (s..e).contains(&at)) {
            return None;
        }
        Some(at - self.runs.iter().filter(|&&(_, e)| e <= at).map(|(s, e)| e - s).sum::<u64>())
    }

    /// `memory` with the runs taken out.
    pub fn bytes(&self, memory: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(memory.len() - self.removed() as usize);
        let mut from = 0usize;
        for &(s, e) in &self.runs {
            out.extend_from_slice(&memory[from..s as usize]);
            from = e as usize;
        }
        out.extend_from_slice(&memory[from..]);
        out
    }
}

/// A machine layout on the card's channels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PhysicalMap {
    pub image: Image,
    /// The layout's first byte past its tensors: where the space after the weights starts.
    pub tensors_end: u64,
    /// Logical lines of the space after the weights.
    pub lines: u64,
    pub granule: u64,
    /// The first channel line past the weight image, the same on every channel.
    pub base: u64,
    /// Channel lines each channel holds in all.
    pub channel_lines: Vec<u64>,
}

impl PhysicalMap {
    /// `machine` on the D5005's channels at `beat`, its granule the standard engine plan's state chunk.
    pub fn d5005(machine: &MachineLayout, beat: Beat) -> Result<PhysicalMap, ImageError> {
        PhysicalMap::of(machine, &ImageLayout::d5005(beat), EnginePlan::standard().state_chunk_lines)
    }

    /// `machine` on the channels of `layout`, the space after the weights dealt in granules of `granule` lines.
    pub fn of(machine: &MachineLayout, layout: &ImageLayout, granule: u64) -> Result<PhysicalMap, ImageError> {
        let image = image_of(machine, layout)?;
        let tensors_end = machine.rotary.first().map_or(machine.regions, |&(_, at)| at);
        PhysicalMap::over(image, tensors_end, machine.bytes, layout, granule)
    }

    /// `machine` on the channels of `layout` with no weight image: every byte, the tensors' with the rest, dealt in
    /// granules of `granule` lines from the channels' first line — the memory as an engine that reads its weights a
    /// line at a time by address finds it ([`card_port_verilog`]).
    pub fn flat(machine: &MachineLayout, layout: &ImageLayout, granule: u64) -> Result<PhysicalMap, ImageError> {
        let image = weight_image(&TensorTable { blocks: 0, tensors: Vec::new() }, &[], layout)?;
        PhysicalMap::over(image, 0, machine.bytes, layout, granule)
    }

    /// The hybrid engine's memory: the matrices `machine`'s matvec jobs stream in their image under `layout` (in stripes
    /// when it has one), and the rest of the memory compacted ([`Compaction`]) with the streamed tensors' stream entries
    /// after it, as one logical space from logical byte 0 dealt in granules of `granule` lines past the image, the space
    /// the reference engine's card port reads; with the image's tensors' indices in the machine's and the compaction.
    pub fn hybrid(machine: &MachineLayout, layout: &ImageLayout, granule: u64) -> Result<(PhysicalMap, Vec<usize>, Compaction), ImageError> {
        let (image, streamed) = streamed_image_of(machine, layout)?;
        let compaction = Compaction::of(machine);
        let map = PhysicalMap::over(image, 0, compaction.space_bytes(), layout, granule)?;
        Ok((map, streamed, compaction))
    }

    /// The space `[tensors_end, bytes)` dealt in granules past `image`.
    fn over(image: Image, tensors_end: u64, bytes: u64, layout: &ImageLayout, granule: u64) -> Result<PhysicalMap, ImageError> {
        let lines = (bytes - tensors_end).div_ceil(LINE);
        let base = image.channel_lines.iter().copied().max().unwrap_or(0);
        let mut map = PhysicalMap { image, tensors_end, lines, granule: granule.max(1), base, channel_lines: Vec::new() };
        let channels = layout.channels as u64;
        let granules = lines.div_ceil(map.granule);
        map.channel_lines = (0..channels)
            .map(|c| {
                let local = (granules + channels - 1 - c) / channels * map.granule;
                base + (local * LINE).div_ceil(layout.line_bytes)
            })
            .collect();
        for (c, &l) in map.channel_lines.iter().enumerate() {
            if l > layout.capacity_lines {
                return Err(ImageError::TooBig { channel: c as u32, lines: l, capacity: layout.capacity_lines });
            }
        }
        Ok(map)
    }

    fn channels(&self) -> u64 {
        self.image.layout.channels as u64
    }

    /// The channel and the channel byte logical byte `at` of the space after the weights lives at.
    pub fn locate(&self, at: u64) -> (u32, u64) {
        let rel = at - self.tensors_end;
        let (x, within) = (rel / LINE, rel % LINE);
        let g = x / self.granule;
        let local = g / self.channels() * self.granule + x % self.granule;
        let lb = self.image.layout.line_bytes;
        ((g % self.channels()) as u32, self.base * lb + local * LINE + within)
    }

    /// The channel runs that hold logical bytes `[at, at + bytes)` of the space after the weights, one on each channel
    /// the bytes touch, in channel order.
    pub fn runs(&self, at: u64, bytes: u64) -> Vec<Run> {
        let mut runs: Vec<Run> = Vec::new();
        let (end, granule_bytes) = (at + bytes, self.granule * LINE);
        let mut from = at;
        while from < end {
            let to = end.min(self.tensors_end + ((from - self.tensors_end) / granule_bytes + 1) * granule_bytes);
            let (channel, byte) = self.locate(from);
            match runs.iter_mut().find(|r| r.channel == channel) {
                Some(r) if r.at + r.bytes == byte => r.bytes += to - from,
                Some(_) => unreachable!("a channel's granules within one run are consecutive local lines"),
                None => runs.push(Run { channel, at: byte, bytes: to - from }),
            }
            from = to;
        }
        runs.sort_by_key(|r| r.channel);
        runs
    }

    fn block(machine: &MachineLayout, layer: usize) -> &BlockState {
        &machine.blocks[layer]
    }

    /// Sequence `s`'s conv-ring row for position `p` in Gated-DeltaNet layer `layer`.
    pub fn ring_row(&self, machine: &MachineLayout, s: usize, layer: usize, p: usize) -> Vec<Run> {
        let BlockState::Delta { ring, channels, taps, .. } = *Self::block(machine, layer) else {
            panic!("layer {layer} is not a Gated-DeltaNet block")
        };
        let row = channels as u64 * state_element(machine);
        let rows = taps.saturating_sub(1).max(1) as u64;
        self.runs(machine.region_of(s) + ring + (p as u64 % rows) * row, row)
    }

    /// Value head `head`'s memory of sequence `s` in Gated-DeltaNet layer `layer`.
    pub fn memory(&self, machine: &MachineLayout, s: usize, layer: usize, head: usize) -> Vec<Run> {
        let BlockState::Delta { memory, width, .. } = *Self::block(machine, layer) else {
            panic!("layer {layer} is not a Gated-DeltaNet block")
        };
        let bytes = (width * width) as u64 * state_element(machine);
        self.runs(machine.region_of(s) + memory + head as u64 * bytes, bytes)
    }

    /// Sequence `s`'s log in Gated-DeltaNet layer `layer`: its count line and entries `entries`.
    pub fn log(&self, machine: &MachineLayout, s: usize, layer: usize, entries: Range<usize>) -> Vec<Run> {
        let BlockState::Delta { log: Some(log), entry, .. } = *Self::block(machine, layer) else {
            panic!("layer {layer} keeps no log")
        };
        let at = machine.region_of(s) + log;
        let mut runs = self.runs(at, LINE);
        if !entries.is_empty() {
            let more = self.runs(at + LINE + entries.start as u64 * entry, entries.len() as u64 * entry);
            runs.extend(more);
        }
        runs
    }

    /// Sequence `s`'s cache rows of positions `0..=p` in attention layer `layer`: every cached head's key rows, value
    /// rows and, for an int8 cache, their scales.
    pub fn cache(&self, machine: &MachineLayout, s: usize, layer: usize, p: usize) -> Vec<Run> {
        let BlockState::Attention { keys, values, key_scales, value_scales, kv_heads, width, value_width } = *Self::block(machine, layer) else {
            panic!("layer {layer} is not an attention block")
        };
        let element = match machine.numerics.cache {
            CacheFormat::F16 => 2,
            CacheFormat::Int8 | CacheFormat::Int8Hadamard => 1,
        };
        let (key_row, value_row) = (width as u64 * element, value_width as u64 * element);
        let region = machine.region_of(s);
        let positions = (p + 1) as u64 * kv_heads as u64;
        let mut runs = self.runs(region + keys, positions * key_row);
        runs.extend(self.runs(region + values, positions * value_row));
        for scales in [key_scales, value_scales].into_iter().flatten() {
            runs.extend(self.runs(region + scales, positions * SCALE_BYTES));
        }
        runs
    }

    /// The channels' bytes of `memory`, the machine's whole memory: the weight image's from `weights` (its channels'
    /// bytes, [`Image::fill`]) and every byte after the weights where [`PhysicalMap::locate`] puts it.
    pub fn channel_bytes(&self, memory: &[u8], weights: &[Vec<u8>]) -> Vec<Vec<u8>> {
        let lb = self.image.layout.line_bytes;
        let mut out: Vec<Vec<u8>> = self.channel_lines.iter().map(|&l| vec![0u8; (l * lb) as usize]).collect();
        for (c, w) in weights.iter().enumerate() {
            out[c][..w.len()].copy_from_slice(w);
        }
        let mut from = self.tensors_end;
        let granule_bytes = self.granule * LINE;
        while from < memory.len() as u64 {
            let to = (memory.len() as u64).min(self.tensors_end + ((from - self.tensors_end) / granule_bytes + 1) * granule_bytes);
            let (c, byte) = self.locate(from);
            out[c as usize][byte as usize..(byte + to - from) as usize].copy_from_slice(&memory[from as usize..to as usize]);
            from = to;
        }
        out
    }
}

/// Bytes of one element of the layout's recurrent state.
fn state_element(machine: &MachineLayout) -> u64 {
    match machine.numerics.state {
        StateFormat::F32 => 4,
        StateFormat::F16 => 2,
    }
}

/// `logos_card_port.v`: the engine's memory port on the card's channels, every logical line where the map puts it.
pub fn card_port_verilog() -> String {
    CARD_PORT.to_string()
}

/// `logos_fast_port`, a hybrid engine's fast side's memory port on the channels' fast ports: whole logical lines read
/// and written where the map puts them, consecutive lines of a channel one descriptor, the reads returned in order.
pub fn fast_port_verilog() -> String {
    FAST_PORT.to_string()
}

/// `logos_channel_mux`, several fast ports on the channels' fast ports at once: each port's descriptors and write lines
/// through registers of its own in the order it gives them, each channel's descriptors taken from its ports in turn, each
/// read line and write's done token given back to the port it is of, one port's write lines on a channel at a time.
pub fn channel_mux_verilog() -> String {
    format!("{}{CHANNEL_MUX}", super::rtl::ram::RAMSTYLE)
}

/// `logos_read_credit`, a reader's lines of one channel through the channel multiplexer asked only as a buffer of its own
/// can hold them: its descriptors asked in runs of at most 32 lines, every line back taken at once, the reader's next line
/// in a register. A reader whose channels step together — the streaming unit — reaches the multiplexer through one a
/// channel, so the multiplexer never waits on it for a line.
pub fn read_credit_verilog() -> String {
    READ_CREDIT.to_string()
}

/// `logos_pipe`, a stream through one register for a path a floorplan stretches across the device: each word taken leaves
/// a clock or more later, in order, a word a clock while the far side takes them, the near side's ready a register.
pub fn pipe_verilog() -> &'static str {
    PIPE
}

const PIPE: &str = r##"// A stream through one register, for a path a floorplan stretches across the device: each word taken from `in` leaves
// `out` a clock or more later, in order, a word a clock while `out` takes them. `in_ready` is a register: a second
// place holds the word taken the clock `out` stops taking, and `in` waits until it has left. The second place takes
// `in`'s word every clock it is empty, so its load waits on no valid from either side.
module logos_pipe #(parameter W = 32) (input clk, input rst_n, input in_valid, output reg in_ready, input [W-1:0] in_data,
  output reg out_valid, input out_ready, output reg [W-1:0] out_data);
  reg skid_v;
  reg [W-1:0] skid;
  wire take = in_valid && in_ready;
  wire moves = !out_valid || out_ready;
  always @(posedge clk)
    if (!rst_n) begin
      out_valid <= 1'b0;
      skid_v <= 1'b0;
      in_ready <= 1'b0;
    end else begin
      if (moves) out_valid <= skid_v || take;
      skid_v <= !moves && (skid_v || take);
      in_ready <= moves || !(skid_v || take);
    end
  always @(posedge clk) begin
    if (moves) out_data <= skid_v ? skid : in_data;
    if (!skid_v) skid <= in_data;
  end
endmodule

// K pipes in a row, for a path across the die a pipe each part of the way: each word leaves K clocks or more after it is
// taken, in order, a word a clock while `out` takes them; every ready a register.
module logos_pipe_chain #(parameter W = 32, parameter K = 1) (input clk, input rst_n, input in_valid, output in_ready,
  input [W-1:0] in_data, output out_valid, input out_ready, output [W-1:0] out_data);
  wire [K:0] v, r;
  wire [(K+1)*W-1:0] d;
  assign v[0] = in_valid;
  assign in_ready = r[0];
  assign d[W-1:0] = in_data;
  assign out_valid = v[K];
  assign r[K] = out_ready;
  assign out_data = d[K*W +: W];
  genvar k;
  generate for (k = 0; k < K; k = k + 1) begin : stage
    logos_pipe #(.W(W)) p(.clk(clk), .rst_n(rst_n), .in_valid(v[k]), .in_ready(r[k]), .in_data(d[k*W +: W]), .out_valid(v[k+1]),
      .out_ready(r[k+1]), .out_data(d[(k+1)*W +: W]));
  end endgenerate
endmodule
"##;

const CARD_PORT: &str = r##"// The engine's memory port on the card's channels: each request one logical line of the machine's memory, its place
// the physical map's — logical line x of the granule space, which starts at logical line `space`, lies in granule
// g = (x - space) / 2^granule_log2, on channel g mod NB, at channel line base + (g / NB) * 2^granule_log2 + (x - space)
// mod 2^granule_log2. Requests are served one at a time, in the order they come: a read sends its line's descriptor and
// returns the line; a write under a mask of every byte writes its line, under any other mask reads the line first and
// writes it merged, and is ready to the engine only once its channel has written it, so a read after a write reads
// what it wrote. A read pulse taken while a write is served waits its turn. A line below the space, or one past a
// channel's 2^AW lines, faults: its read returns zeros, its write is dropped. A line read with a word its channel's
// SECDED code finds uncorrectable faults too, its read returning the line as read and a merged write dropped, so a bad
// word is never coded afresh. `fault` holds until `clear`. Every register steps at `ce`, the shell's compute step.
module logos_card_port #(
  parameter NB = 4,            // channels, a power of two
  parameter AW = 27,           // channel line-address bits
  parameter LW = 32,           // a descriptor's line-count bits
  parameter DL = 512           // a line's bits
) (
  input                     clk,
  input                     ce,
  input                     rst_n,
  input  [31:0]             space,
  input  [AW-1:0]           base,
  input  [4:0]              granule_log2,
  input                     clear,
  output reg                fault,
  input                     mem_rd,
  input  [31:0]             mem_rd_line,
  output reg                mem_rd_valid,
  output reg [DL-1:0]       mem_rd_data,
  input                     mem_wr,
  input  [31:0]             mem_wr_line,
  input  [DL-1:0]           mem_wr_data,
  input  [DL/8-1:0]         mem_wr_mask,
  output reg                mem_wr_ready,
  output [NB-1:0]           md_valid,
  input  [NB-1:0]           md_ready,
  output [NB*(AW+LW+1)-1:0] md_data,
  input  [NB-1:0]           mr_valid,
  output [NB-1:0]           mr_ready,
  input  [NB*DL-1:0]        mr_data,
  input  [NB*16-1:0]        mr_ecc,      // each line read's words' verdicts {uncorrectable, corrected}
  output [NB-1:0]           mw_valid,
  input  [NB-1:0]           mw_ready,
  output [NB*DL-1:0]        mw_data,
  input  [NB-1:0]           mdone_valid,
  output [NB-1:0]           mdone_ready
);
  localparam CB = NB > 1 ? $clog2(NB) : 1;
  localparam [2:0] IDLE = 3'd0, RD_DESC = 3'd1, RD_WAIT = 3'd2, WR_SEND = 3'd3, WR_DONE = 3'd4, ACK = 3'd5;
  reg  [2:0]    state;
  reg           rd_pending, merge, desc_sent, data_sent;
  reg  [31:0]   rd_line;
  reg  [CB-1:0] ch;
  reg  [AW-1:0] at;
  reg  [DL-1:0] line;
  reg  [DL/8-1:0] mask;

  // Where logical line `l` lives: its channel, its channel line, and whether it lies outside the space.
  reg  [31:0]   x, g, in_granule, local_line;
  reg  [32:0]   to;
  reg  [CB-1:0] l_ch;
  reg  [AW-1:0] l_at;
  reg           l_bad;
  wire [31:0]   l = mem_wr ? mem_wr_line : rd_line;
  always @(*) begin
    x = l - space;
    g = x >> granule_log2;
    in_granule = x & ((32'd1 << granule_log2) - 32'd1);
    local_line = ((g >> CB) << granule_log2) | in_granule;
    to = {1'b0, {(32-AW){1'b0}}, base} + {1'b0, local_line};
    l_ch = g[CB-1:0];
    l_at = to[AW-1:0];
    l_bad = l < space || to[32:AW] != 0;
  end

  function [DL-1:0] merged(input [DL-1:0] old, input [DL-1:0] new_, input [DL/8-1:0] m);
    integer i;
    for (i = 0; i < DL/8; i = i + 1) merged[8*i +: 8] = m[i] ? new_[8*i +: 8] : old[8*i +: 8];
  endfunction

  wire writing = state == WR_SEND;
  genvar b;
  generate for (b = 0; b < NB; b = b + 1) begin : channel
    wire mine = ch == b;
    assign md_valid[b] = mine && (state == RD_DESC || (writing && !desc_sent));
    assign md_data[b*(AW+LW+1) +: AW+LW+1] = {writing, at, {{(LW-1){1'b0}}, 1'b1}};
    assign mr_ready[b] = mine && state == RD_WAIT;
    assign mw_valid[b] = mine && writing && !data_sent;
    assign mw_data[b*DL +: DL] = line;
    assign mdone_ready[b] = mine && state == WR_DONE;
  end endgenerate

  always @(posedge clk or negedge rst_n) begin
    if (!rst_n) begin
      state <= IDLE; rd_pending <= 1'b0; merge <= 1'b0; desc_sent <= 1'b0; data_sent <= 1'b0; rd_line <= 0; ch <= 0;
      at <= 0; line <= 0; mask <= 0; fault <= 1'b0; mem_rd_valid <= 1'b0; mem_rd_data <= 0; mem_wr_ready <= 1'b0;
    end else if (ce) begin
      mem_rd_valid <= 1'b0;
      mem_wr_ready <= 1'b0;
      if (clear) fault <= 1'b0;
      if (mem_rd) begin
        rd_pending <= 1'b1;
        rd_line <= mem_rd_line;
      end
      case (state)
        IDLE: if (mem_wr) begin
          ch <= l_ch; at <= l_at; line <= mem_wr_data; mask <= mem_wr_mask; desc_sent <= 1'b0; data_sent <= 1'b0;
          if (l_bad) begin
            fault <= 1'b1; mem_wr_ready <= 1'b1; state <= ACK;
          end else if (&mem_wr_mask) begin
            merge <= 1'b0; state <= WR_SEND;
          end else begin
            merge <= 1'b1; state <= RD_DESC;
          end
        end else if (rd_pending) begin
          rd_pending <= 1'b0;
          ch <= l_ch; at <= l_at; merge <= 1'b0;
          if (l_bad) begin
            fault <= 1'b1; mem_rd_data <= {DL{1'b0}}; mem_rd_valid <= 1'b1;
          end else state <= RD_DESC;
        end
        RD_DESC: if (md_ready[ch]) state <= RD_WAIT;
        RD_WAIT: if (mr_valid[ch]) begin
          if (|mr_ecc[ch*16 + 8 +: 8]) fault <= 1'b1;
          if (merge && |mr_ecc[ch*16 + 8 +: 8]) begin
            mem_wr_ready <= 1'b1; state <= ACK;
          end else if (merge) begin
            line <= merged(mr_data[ch*DL +: DL], line, mask); state <= WR_SEND;
          end else begin
            mem_rd_data <= mr_data[ch*DL +: DL]; mem_rd_valid <= 1'b1; state <= IDLE;
          end
        end
        WR_SEND: begin
          if (md_valid[ch] && md_ready[ch]) desc_sent <= 1'b1;
          if (mw_valid[ch] && mw_ready[ch]) data_sent <= 1'b1;
          if ((desc_sent || md_ready[ch]) && (data_sent || mw_ready[ch])) state <= WR_DONE;
        end
        WR_DONE: if (mdone_valid[ch]) begin mem_wr_ready <= 1'b1; state <= ACK; end
        default: state <= IDLE;
      endcase
    end
  end
endmodule
"##;

const FAST_PORT: &str = r##"// The fast side's memory port on the channels' fast ports: whole logical lines of the granule space read and written,
// each where the physical map puts it — logical line x of the space that starts at logical line `space` lies in granule
// g = (x - space) / 2^granule_log2, on channel g mod NB, at channel line base + (g / NB) * 2^granule_log2 + (x - space)
// mod 2^granule_log2, as logos_card_port places it. Reads of consecutive lines of one channel go out as one descriptor
// of up to MAXL lines, and so do writes, a run closed when the next line asked is not its next or none is asked; reads
// return in the order they were asked, whichever channels hold them, each taken the cycle its channel has it and given
// from a register the cycle after (rs_valid takes no wait: the side asks for no more lines than it has room for), so the
// line crosses to the side that asked from a register beside it; a write's line goes to its channel as it is taken,
// ahead of its run's descriptor, a run closed early when its channel holds all the lines it has room for, and the
// writes are counted out until their channels report them written (`writing`). A read asked before a write of its line
// reads the line before the write, a read's descriptor never leaving behind a later write's; a line read while a write
// of it is still out may read it before the write. A line outside the space, or past a channel's 2^AW lines, faults:
// its read returns zeros, its write is dropped. `fault` holds until `clear`. A request is taken into two registers of
// its kind, a clock before the port serves it, and its line's place comes from a register: the line after the one last
// served is placed a clock ahead, and any other waits a clock to be placed, its run held open.
module logos_fast_port #(parameter NB = 4, AW = 27, LW = 32, DL = 512, MAXL = 32, ORDER = 64)
  (input clk, input rst,
   input [31:0] space, input [AW-1:0] base, input [4:0] granule_log2, input clear, output reg fault,
   input rq_valid, output rq_ready, input [31:0] rq_line, output rs_valid, output [DL-1:0] rs_data,
   input wq_valid, output wq_ready, input [31:0] wq_line, input [DL-1:0] wq_data, output writing,
   output [NB-1:0] md_valid, input [NB-1:0] md_ready, output [NB*(AW+LW+1)-1:0] md_data,
   input [NB-1:0] mr_valid, output [NB-1:0] mr_ready, input [NB*DL-1:0] mr_data,
   output [NB-1:0] mw_valid, input [NB-1:0] mw_ready, output [NB*DL-1:0] mw_data,
   input [NB-1:0] mdone_valid, output [NB-1:0] mdone_ready);
  localparam CB = NB > 1 ? $clog2(NB) : 1, OB = $clog2(ORDER), NW = $clog2(MAXL + 1);
  localparam [NW-1:0] FULL = MAXL;

  // Each kind's requests through two registers, taken while the second is empty, so neither the asking unit's address
  // arithmetic nor the port's placement waits on the other within a clock: the port serves the oldest it holds (rqh_*,
  // wqh_*), a request from the clock after it is taken. A write's word goes into one of two places as it is taken in
  // (wq_d0, wq_d1, the next taken in going to wq_put's), and the head's word is the place wq_get names: the words move
  // only on their own taking in, never on the port's serving a write, which only turns wq_get.
  reg rqh_valid, rqt_valid, wqh_valid, wqt_valid;
  reg [31:0] rqh_line, rqt_line, wqh_line, wqt_line;
  reg [DL-1:0] wq_d0, wq_d1;
  (* maxfan = 64 *) reg wq_get;
  reg wq_put;
  wire [DL-1:0] wqh_data = wq_get ? wq_d1 : wq_d0;
  assign rq_ready = !rqt_valid;
  assign wq_ready = !wqt_valid;
  wire r_in = rq_valid && !rqt_valid, w_in = wq_valid && !wqt_valid;

  // Where line `l` lives: whether it lies outside the space, its channel, its channel line. Its granule g = x >> gl
  // (x its place in the space) lies on channel g mod NB as the channel's granule g >> CB, so its channel line's bits
  // below gl are x's and those above are x's CB places higher: no shift by the granule but for the channel's bits.
  function [CB+AW:0] placed(input [31:0] l, input [31:0] sp, input [AW-1:0] b, input [4:0] gl, input [31:0] mask);
    reg [31:0] x, g, local_line;
    reg [32:0] to;
    begin
      x = l - sp;
      g = x >> gl;
      local_line = (x & mask) | ((x >> CB) & ~mask);
      to = {1'b0, {(32-AW){1'b0}}, b} + {1'b0, local_line};
      placed = {l < sp || to[32:AW] != 0, g[CB-1:0], to[AW-1:0]};
    end
  endfunction
  // The map, registered where it enters: it holds for the step, and every place is reckoned from the port's own copy,
  // a granule's offset mask beside it.
  reg [31:0] m_space, m_mask;
  reg [AW-1:0] m_base;
  reg [4:0] m_gl;
  always @(posedge clk) begin
    m_space <= space; m_base <= base; m_gl <= granule_log2; m_mask <= (32'd1 << granule_log2) - 32'd1;
  end
  // The place of each kind's next line: the line after the one last taken, or the line waiting; a line asked is placed
  // once it is the one held (`r_hit`, `w_hit`). A map about to change drops both.
  reg [31:0] rg_line, wg_line;
  reg [CB+AW:0] rp, wp;
  reg rg_ok, wg_ok;
  wire r_hit = rg_ok && rqh_line == rg_line, w_hit = wg_ok && wqh_line == wg_line;
  wire map_moves = space != m_space || base != m_base || granule_log2 != m_gl;
  wire [CB+AW:0] r_here = placed(rqh_line, m_space, m_base, m_gl, m_mask), r_after = placed(rqh_line + 32'd1, m_space, m_base, m_gl, m_mask);
  wire [CB+AW:0] w_here = placed(wqh_line, m_space, m_base, m_gl, m_mask), w_after = placed(wqh_line + 32'd1, m_space, m_base, m_gl, m_mask);
  wire r_bad = rp[CB+AW], w_bad = wp[CB+AW];
  wire [CB-1:0] r_ch = rp[CB+AW-1:AW], w_ch = wp[CB+AW-1:AW];
  wire [AW-1:0] r_at = rp[AW-1:0], w_at = wp[AW-1:0];

  // The descriptor on its way to its channel: {write, first line, lines}.
  reg d_valid;
  reg [CB-1:0] d_ch;
  reg [AW+LW:0] d_data;
  wire d_free = !d_valid || md_ready[d_ch];

  // The reads in the order they were asked: each run's channel and lines, or a faulting line's zeros.
  reg [CB+NW:0] order [0:ORDER-1];
  reg [OB-1:0] o_wr, o_rd;
  reg [OB:0] o_count;
  reg [NW-1:0] o_given;
  wire [CB+NW:0] o_head = order[o_rd];
  wire o_zero = o_head[CB+NW], o_have = o_count != 0;
  wire [CB-1:0] o_ch = o_head[CB+NW-1:NW];
  wire [NW-1:0] o_n = o_head[NW-1:0];
  localparam [OB:0] O_LAST = ORDER - 1;
  wire o_room = o_count < O_LAST;
  wire o_take = o_have && (o_zero || mr_valid[o_ch]);
  wire o_pop = o_take && o_given + 1'b1 == o_n;
  (* maxfan = 64 *) reg rs_q;
  reg [DL-1:0] rs_d;
  always @(posedge clk) begin
    rs_q <= !rst && o_take;
    rs_d <= o_zero ? {DL{1'b0}} : mr_data[o_ch*DL +: DL];
  end
  assign rs_valid = rs_q;
  assign rs_data = rs_d;

  // The run of reads being gathered, and of writes, each with the channel line it would take next.
  reg r_open, w_open;
  reg [CB-1:0] rr_ch, wr_ch;
  reg [AW-1:0] rr_at, wr_at, rr_next, wr_next;
  reg [NW-1:0] rr_n, wr_n;
  wire r_joins = r_open && !r_bad && r_ch == rr_ch && r_at == rr_next && rr_n != FULL;
  wire w_joins = w_open && !w_bad && w_ch == wr_ch && w_at == wr_next && wr_n != FULL;
  // A run closes into the descriptor when the next line asked is not its next, or none is, or, for writes, when its
  // channel has no room for the next line (the lines it holds leave only behind their descriptor): the reads' first.
  wire r_close = r_open && !(rqh_valid && (!r_hit || r_joins)) && d_free && o_room;
  wire w_close = w_open && !(wqh_valid && (!w_hit || (w_joins && mw_ready[wr_ch]))) && d_free && !r_close;
  // A read is taken joining its run, opening one where none is open or the open one closes, or, faulting, with none
  // open; a write joining its run, or opening one where none is open or the open one closes, its channel taking its
  // line, or faulting.
  wire rqh_ready = r_hit && (r_bad ? !r_open && o_room : r_joins || !r_open || r_close);
  wire wqh_ready = w_hit && (w_bad || ((w_joins || !w_open || w_close) && mw_ready[w_ch]));
  wire r_take = rqh_valid && rqh_ready, w_take = wqh_valid && wqh_ready;
  wire r_zero = r_take && r_bad, w_line = w_take && !w_bad;

  genvar b;
  generate for (b = 0; b < NB; b = b + 1) begin : channel
    assign md_valid[b] = d_valid && d_ch == b;
    assign md_data[b*(AW+LW+1) +: AW+LW+1] = d_data;
    assign mr_ready[b] = o_have && !o_zero && o_ch == b;
    assign mw_valid[b] = wqh_valid && w_hit && !w_bad && w_ch == b && (w_joins || !w_open || w_close);
    assign mw_data[b*DL +: DL] = wqh_data;
    assign mdone_ready[b] = 1'b1;
  end endgenerate

  // Writes out: descriptors sent less writes reported written. The done tokens are counted by a continuous assignment,
  // which a simulator evaluates from time 0 whether or not a token ever comes.
  reg [OB:0] w_out;
  function [CB:0] ones(input [NB-1:0] v);
    integer i;
    begin
      ones = 0;
      for (i = 0; i < NB; i = i + 1) ones = ones + {{CB{1'b0}}, v[i]};
    end
  endfunction
  wire [CB:0] dones = ones(mdone_valid);
  wire w_sent = d_valid && md_ready[d_ch] && d_data[AW+LW];
  assign writing = wqh_valid || wqt_valid || w_open || (d_valid && d_data[AW+LW]) || w_out != 0;

  always @(posedge clk) begin
    if (rst) begin
      rqh_valid <= 1'b0; rqt_valid <= 1'b0; wqh_valid <= 1'b0; wqt_valid <= 1'b0; wq_put <= 1'b0; wq_get <= 1'b0;
    end else begin
      case ({r_take, r_in})
        2'b01: if (rqh_valid) begin rqt_valid <= 1'b1; rqt_line <= rq_line; end
               else begin rqh_valid <= 1'b1; rqh_line <= rq_line; end
        2'b10: begin rqh_valid <= rqt_valid; rqh_line <= rqt_line; rqt_valid <= 1'b0; end
        2'b11: rqh_line <= rq_line;
        default: ;
      endcase
      case ({w_take, w_in})
        2'b01: if (wqh_valid) begin wqt_valid <= 1'b1; wqt_line <= wq_line; end
               else begin wqh_valid <= 1'b1; wqh_line <= wq_line; end
        2'b10: begin wqh_valid <= wqt_valid; wqh_line <= wqt_line; wqt_valid <= 1'b0; end
        2'b11: wqh_line <= wq_line;
        default: ;
      endcase
      if (w_in) wq_put <= !wq_put;
      if (w_take) wq_get <= !wq_get;
    end
    if (r_take) begin rg_line <= rqh_line + 32'd1; rp <= r_after; end
    else if (rqh_valid && !r_hit) begin rg_line <= rqh_line; rp <= r_here; end
    if (w_in && !wq_put) wq_d0 <= wq_data;
    if (w_in && wq_put) wq_d1 <= wq_data;
    if (w_take) begin wg_line <= wqh_line + 32'd1; wp <= w_after; end
    else if (wqh_valid && !w_hit) begin wg_line <= wqh_line; wp <= w_here; end
    rg_ok <= !rst && !map_moves && (rg_ok || r_take || rqh_valid);
    wg_ok <= !rst && !map_moves && (wg_ok || w_take || wqh_valid);
    if (rst) begin
      d_valid <= 1'b0; r_open <= 1'b0; w_open <= 1'b0; o_wr <= 0; o_rd <= 0; o_count <= 0; o_given <= 0; w_out <= 0;
      fault <= 1'b0;
    end else begin
      if (clear) fault <= 1'b0;
      if (r_zero || (w_take && w_bad)) fault <= 1'b1;
      if (d_valid && md_ready[d_ch]) d_valid <= 1'b0;
      if (r_close) begin
        d_valid <= 1'b1; d_ch <= rr_ch; d_data <= {1'b0, rr_at, {(LW-NW){1'b0}}, rr_n};
        order[o_wr] <= {1'b0, rr_ch, rr_n};
      end else if (w_close) begin
        d_valid <= 1'b1; d_ch <= wr_ch; d_data <= {1'b1, wr_at, {(LW-NW){1'b0}}, wr_n};
      end
      if (r_zero) order[o_wr] <= {1'b1, {CB{1'b0}}, {{(NW-1){1'b0}}, 1'b1}};
      o_wr <= o_wr + (r_close || r_zero);
      o_count <= o_count + (r_close || r_zero) - o_pop;
      if (o_take) o_given <= o_pop ? 0 : o_given + 1'b1;
      if (o_pop) o_rd <= o_rd + 1'b1;
      if (r_take && !r_bad) begin
        if (r_joins) begin rr_n <= rr_n + 1'b1; rr_next <= rr_next + 1'b1; end
        else begin r_open <= 1'b1; rr_ch <= r_ch; rr_at <= r_at; rr_next <= r_at + 1'b1; rr_n <= 1; end
      end else if (r_close) r_open <= 1'b0;
      if (w_line) begin
        if (w_joins) begin wr_n <= wr_n + 1'b1; wr_next <= wr_next + 1'b1; end
        else begin w_open <= 1'b1; wr_ch <= w_ch; wr_at <= w_at; wr_next <= w_at + 1'b1; wr_n <= 1; end
      end else if (w_close) w_open <= 1'b0;
      w_out <= w_out + w_sent - dones;
    end
  end
endmodule
"##;

const CHANNEL_MUX: &str = r##"// Several fast ports on the channels' fast ports at once: P ports' channel sides (logos_fast_port's md_*, mr_*, mw_*,
// mdone_*; port p's of channel c at p·NB + c) onto the NB channels'. Each port's descriptors come in through two places
// of the port's own, one descriptor at a time whatever channel each is for: the next waits behind the last until its
// channel takes it, so the channels take a port's runs in the order the port gives them, whichever channels they are
// of. (A port that offers several channels at once gives them one at a time.) Each port's write lines come in
// through two places of the port's own likewise, a line leaving for its channel only after every line the port gave
// before it has left for its own. Every ready a port sees is a register's. Each channel takes its descriptors from its
// ports in turn, the choice a register a clock ahead: the port it names, if its descriptor is still held and the
// channel takes one; a port just taken is not chosen the clock after, its next descriptor not yet seen. Each read's
// lines go back to the port whose descriptor it was and each write's done token
// likewise, in the order the channel took them (every port reads the channels' lines from mr_data itself). Reads need
// no other rule so long as every port takes its runs' lines in the order it asked for them and waits on nothing else to
// take one: each channel serves its descriptors in the order it took them, and descriptors are taken one at a time a
// channel, so the earliest run taken that has lines still to give is first in both its port's order and its channel's,
// and always moves. A reader that takes a channel's lines only as its other channels let it (the streaming unit, whose
// channels' feeds step together) would deadlock a port waiting on a line of one channel behind the reader's run while
// the reader waits on that port's run in another; such a reader reaches the multiplexer through logos_read_credit a
// channel, which takes every line as it comes. A write's lines reach its channel ahead of its descriptor, so a channel
// takes write lines from one port at a time: the port that holds it — the first whose next line is the channel's, from
// the channel's turn, the clock after it is free — until every line it gave is in a descriptor the channel has taken and
// its next line is not the channel's. A write's descriptor is taken only from that port. A port holds a channel only once
// every line it gave before is in its own channel, so each of its earlier writes' descriptors is taken from a channel the
// port still holds: no port's descriptor waits on a channel whose holder waits on that port's. Each channel's reads and
// writes owed are at most DEPTH apiece: no descriptor is taken beyond them.
module logos_channel_mux #(parameter P = 2, NB = 4, AW = 27, LW = 32, DL = 512, DEPTH = 32)
  (input clk, input rst,
   input [P*NB-1:0] pd_valid, output [P*NB-1:0] pd_ready, input [P*NB*(AW+LW+1)-1:0] pd_data,
   output [P*NB-1:0] pr_valid, input [P*NB-1:0] pr_ready,
   input [P*NB-1:0] pw_valid, output [P*NB-1:0] pw_ready, input [P*NB*DL-1:0] pw_data,
   output [P*NB-1:0] pdone_valid, input [P*NB-1:0] pdone_ready,
   output [NB-1:0] md_valid, input [NB-1:0] md_ready, output [NB*(AW+LW+1)-1:0] md_data,
   input [NB-1:0] mr_valid, output [NB-1:0] mr_ready,
   output [NB-1:0] mw_valid, input [NB-1:0] mw_ready, output [NB*DL-1:0] mw_data,
   input [NB-1:0] mdone_valid, output [NB-1:0] mdone_ready);
  localparam DLW = AW + LW + 1, PB = P > 1 ? $clog2(P) : 1, DB = $clog2(DEPTH);
  localparam [PB-1:0] LAST = P - 1;
  localparam [DB:0] ROOM = DEPTH - 2;
  localparam CB = NB > 1 ? $clog2(NB) : 1;
  // Inside: each port's held descriptor offered to its channel (h_dv, h_dd) and taken (h_dr), and each port's held
  // write line offered to its channel (h_wv, h_wd) and taken (h_wr).
  wire [P*NB-1:0] h_dv, h_dr, h_wv, h_wr;
  wire [P*NB*DLW-1:0] h_dd;
  wire [P*DL-1:0] h_wd;
  // The line of the channel `on` names (one-hot) among each channel's `lines`.
  function [DL-1:0] line_of(input [NB-1:0] on, input [NB*DL-1:0] lines);
    integer b;
    begin
      line_of = {DL{1'b0}};
      for (b = 0; b < NB; b = b + 1) if (on[b]) line_of = line_of | lines[b*DL +: DL];
    end
  endfunction
  genvar c, q;
  generate for (q = 0; q < P; q = q + 1) begin : held
    // The port's descriptor: the lowest channel it offers one to and that channel's descriptor, taken while the port's
    // ready is up; the place it leaves from, and a second place for the one taken the clock the first does not leave.
    reg [CB-1:0] pick;
    reg [DLW-1:0] offered;
    integer pc;
    always @* begin
      pick = {CB{1'b0}};
      offered = pd_data[q*NB*DLW +: DLW];
      for (pc = NB - 1; pc >= 0; pc = pc - 1)
        if (pd_valid[q*NB + pc]) begin pick = pc[CB-1:0]; offered = pd_data[(q*NB + pc)*DLW +: DLW]; end
    end
    reg d_rdy, d_v, d_sv;
    reg [CB-1:0] d_ch, d_sch;
    reg [DLW-1:0] d_d, d_sd;
    wire d_take = |pd_valid[q*NB +: NB] && d_rdy;
    wire d_moves = !d_v || h_dr[q*NB + d_ch];
    always @(posedge clk) begin
      if (d_moves) begin
        d_v <= d_sv || d_take;
        d_ch <= d_sv ? d_sch : pick;
        d_d <= d_sv ? d_sd : offered;
      end else if (d_take) begin
        d_sch <= pick;
        d_sd <= offered;
      end
      d_sv <= !d_moves && (d_sv || d_take);
      d_rdy <= d_moves || !(d_sv || d_take);
      if (rst) begin d_v <= 1'b0; d_sv <= 1'b0; d_rdy <= 1'b0; end
    end
    // The port's write lines, alike, through two places of their own in the order the port gives them, each line with
    // its channel one-hot (none while the place is empty): the lowest channel the port offers a line to — a port offers
    // a line to one channel at a time, so its ready is the places' whatever the channel. A line leaves for its channel
    // only once every line the port gave before it has left for its own, so a port holds a channel's write lines only
    // after its earlier runs' lines are in theirs, and never waits for a channel's lines behind a later run's. The spare
    // place takes the port's line every clock it is empty, so a clock that fills it has given it the line taken.
    wire [NB-1:0] w_offers = pw_valid[q*NB +: NB];
    wire [NB-1:0] w_first = w_offers & ~(w_offers - {{(NB-1){1'b0}}, 1'b1});
    wire [DL-1:0] w_line = line_of(w_first, pw_data[q*NB*DL +: NB*DL]);
    reg w_rdy;
    (* maxfan = 256 *) reg w_sv;
    reg [NB-1:0] w_on, w_son;
    reg [DL-1:0] w_d, w_sd;
    wire w_take = |w_offers && w_rdy;
    wire w_moves = (w_on & ~h_wr[q*NB +: NB]) == {NB{1'b0}};
    always @(posedge clk) begin
      if (w_moves) begin
        w_on <= w_sv ? w_son : w_take ? w_first : {NB{1'b0}};
        w_d <= w_sv ? w_sd : w_line;
      end
      if (!w_sv) begin w_son <= w_first; w_sd <= w_line; end
      w_sv <= !w_moves && (w_sv || w_take);
      w_rdy <= w_moves || !(w_sv || w_take);
      if (rst) begin w_on <= {NB{1'b0}}; w_sv <= 1'b0; w_rdy <= 1'b0; end
    end
    assign h_wv[q*NB +: NB] = w_on;
    assign h_wd[q*DL +: DL] = w_d;
    for (c = 0; c < NB; c = c + 1) begin : lane
      assign pd_ready[q*NB + c] = d_rdy && pick == c;
      assign h_dv[q*NB + c] = d_v && d_ch == c;
      assign h_dd[(q*NB + c)*DLW +: DLW] = d_d;
      assign pw_ready[q*NB + c] = w_rdy;
    end
  end endgenerate
  generate for (c = 0; c < NB; c = c + 1) begin : channel
    // Every port's descriptor of this channel, and whether it is a write.
    wire [P-1:0] offers, writes, lines_offered;
    wire [DLW*P-1:0] descs;
    for (q = 0; q < P; q = q + 1) begin : port
      assign offers[q] = h_dv[q*NB + c];
      assign descs[DLW*q +: DLW] = h_dd[(q*NB + c)*DLW +: DLW];
      assign writes[q] = descs[DLW*q + DLW - 1];
      assign lines_offered[q] = h_wv[q*NB + c];
    end
    // The reads owed (each its port and lines) and the writes (each its port), in the order taken.
    `LOGOS_RAMSTYLE_ASYNC(DEPTH) reg [PB-1:0] r_port [0:DEPTH-1];
    `LOGOS_RAMSTYLE_ASYNC(DEPTH) reg [LW-1:0] r_lines [0:DEPTH-1];
    `LOGOS_RAMSTYLE_ASYNC(DEPTH) reg [PB-1:0] w_port [0:DEPTH-1];
    reg [DB:0] r_n, w_n;
    reg [DB-1:0] r_wr, r_rd, w_wr, w_rd;
    reg [LW-1:0] r_given;
    wire r_have = r_n != 0, w_have = w_n != 0;
    wire [PB-1:0] r_head = r_port[r_rd], w_head = w_port[w_rd];
    // The write lines' holder, and its lines given and not yet in a descriptor taken.
    reg lock;
    (* maxfan = 256 *) reg [PB-1:0] holder;
    reg [LW:0] owed_lines;
    // The port chosen for the next clock's descriptor.
    reg chosen;
    reg [PB-1:0] choice, turn;
    wire [DLW-1:0] desc = descs[DLW*choice +: DLW];
    wire takes = chosen && offers[choice];
    assign md_valid[c] = takes;
    assign md_data[c*DLW +: DLW] = desc;
    wire taken = takes && md_ready[c];
    wire w_taken = taken && desc[DLW-1];
    for (q = 0; q < P; q = q + 1) begin : grant
      assign h_dr[q*NB + c] = chosen && choice == q && md_ready[c];
      assign pr_valid[q*NB + c] = mr_valid[c] && r_have && r_head == q;
      assign pdone_valid[q*NB + c] = mdone_valid[c] && w_have && w_head == q;
      assign h_wr[q*NB + c] = mw_ready[c] && lock && holder == q;
    end
    assign mr_ready[c] = r_have && pr_ready[r_head*NB + c];
    assign mdone_ready[c] = w_have && pdone_ready[w_head*NB + c];
    assign mw_valid[c] = lock && h_wv[holder*NB + c];
    assign mw_data[c*DL +: DL] = h_wd[holder*DL +: DL];
    wire given = mw_valid[c] && mw_ready[c];
    wire r_back = mr_valid[c] && mr_ready[c];
    wire r_done = r_back && r_given + 1'b1 == r_lines[r_rd];
    wire w_back = mdone_valid[c] && mdone_ready[c];
    wire [LW:0] owed_next = owed_lines + {{LW{1'b0}}, given} - (w_taken ? {1'b0, desc[LW-1:0]} : {(LW+1){1'b0}});
    // The next clock's choice: from the turn, the first port offering a descriptor the channel may take — a read with a
    // read owed's room after this clock's, a write from the holder with a write's room — other than the port taken now.
    reg next_chosen;
    reg [PB-1:0] next_choice, lock_pick;
    reg lock_any;
    integer k, j;
    always @* begin
      next_chosen = 1'b0;
      next_choice = {PB{1'b0}};
      lock_any = 1'b0;
      lock_pick = {PB{1'b0}};
      for (k = 0; k < P; k = k + 1) begin
        j = k + turn;
        if (j >= P) j = j - P;
        if (!next_chosen && offers[j] && !(taken && choice == j[PB-1:0])
            && (writes[j] ? lock && holder == j[PB-1:0] && w_n <= ROOM : r_n <= ROOM)) begin
          next_chosen = 1'b1;
          next_choice = j[PB-1:0];
        end
        if (!lock_any && lines_offered[j]) begin lock_any = 1'b1; lock_pick = j[PB-1:0]; end
      end
    end
    always @(posedge clk) begin
      chosen <= !rst && next_chosen;
      choice <= next_choice;
      if (taken) turn <= (choice == LAST) ? {PB{1'b0}} : choice + 1'b1;
      if (taken && !desc[DLW-1]) begin r_port[r_wr] <= choice; r_lines[r_wr] <= desc[LW-1:0]; r_wr <= r_wr + 1'b1; end
      if (w_taken) begin w_port[w_wr] <= choice; w_wr <= w_wr + 1'b1; end
      if (r_back) r_given <= r_done ? {LW{1'b0}} : r_given + 1'b1;
      if (r_done) r_rd <= r_rd + 1'b1;
      if (w_back) w_rd <= w_rd + 1'b1;
      r_n <= r_n + {{DB{1'b0}}, taken && !desc[DLW-1]} - {{DB{1'b0}}, r_done};
      w_n <= w_n + {{DB{1'b0}}, w_taken} - {{DB{1'b0}}, w_back};
      owed_lines <= owed_next;
      if (!lock) begin lock <= lock_any; holder <= lock_pick; end
      else if (owed_next == 0 && !h_wv[holder*NB + c]) lock <= 1'b0;
      if (rst) begin
        turn <= {PB{1'b0}}; r_wr <= 0; r_rd <= 0; w_wr <= 0; w_rd <= 0; r_n <= 0; w_n <= 0; r_given <= 0; owed_lines <= 0;
        lock <= 1'b0;
      end
    end
  end endgenerate
endmodule
"##;

const READ_CREDIT: &str = r##"// One reader's lines of one channel through the channel multiplexer, asked only as they can be held. The reader's read
// descriptors (a first line and a count of any size, its write bit clear) are asked of the channel in runs of at most
// MAXL lines, a run only while the lines asked and not yet taken by the reader, the run's own included, are at most
// 2^LA; every line back is held in a buffer of 2^LA lines the reader takes from in order, its next line in a register.
// So the multiplexer never waits on the reader for a line (mr_ready is high), however the reader takes them: a reader
// that takes a channel's lines only as its other channels let it — the streaming unit, whose channels' feeds step
// together — needs this to share the channels with ports that take their lines in the order they asked for them.
module logos_read_credit #(parameter AW = 27, LW = 32, DL = 512, LA = 9, MAXL = 32)
  (input clk, input rst,
   input in_valid, output in_ready, input [AW+LW:0] in_data,
   output out_valid, input out_ready, output [DL-1:0] out_data,
   output reg md_valid, input md_ready, output [AW+LW:0] md_data,
   input mr_valid, output mr_ready, input [DL-1:0] mr_data);
  localparam NW = $clog2(MAXL + 1);
  localparam [LW-1:0] RUN = MAXL, TWO_RUNS = 2 * MAXL;
  localparam [NW-1:0] RUN_N = MAXL;
  localparam [LA:0] ALL = 1 << LA;
  // The descriptor being asked: its next line, whether it has lines left, and whether more than a run's.
  reg [AW-1:0] at;
  reg [LW-1:0] left;
  reg none, more;
  wire [NW-1:0] n = more ? RUN_N : left[NW-1:0];
  // The lines the buffer can still promise: all of it less the lines asked and not yet taken by the reader.
  reg [LA:0] credit;
  wire pop = out_valid && out_ready;
  wire ask = !none && (!md_valid || md_ready) && credit >= {{(LA+1-NW){1'b0}}, n};
  assign in_ready = none;
  reg [AW-1:0] md_line;
  reg [NW-1:0] md_n;
  assign md_data = {1'b0, md_line, {(LW-NW){1'b0}}, md_n};
  always @(posedge clk) begin
    if (md_valid && md_ready) md_valid <= 1'b0;
    if (ask) begin
      md_valid <= 1'b1; md_line <= at; md_n <= n;
      at <= at + {{(AW-NW){1'b0}}, n}; left <= left - {{(LW-NW){1'b0}}, n};
      none <= !more; more <= more && left > TWO_RUNS;
    end else if (in_valid && none) begin
      at <= in_data[LW +: AW]; left <= in_data[LW-1:0];
      none <= in_data[LW-1:0] == {LW{1'b0}}; more <= in_data[LW-1:0] > RUN;
    end
    credit <= credit - (ask ? {{(LA+1-NW){1'b0}}, n} : {(LA+1){1'b0}}) + {{LA{1'b0}}, pop};
    if (rst) begin md_valid <= 1'b0; none <= 1'b1; more <= 1'b0; credit <= ALL; end
  end
  // The lines back, in the order they come: written into the buffer, read a clock ahead of the reader into one of two
  // places beside it (o0 its next), so the reader's line and its valid are registers.
  assign mr_ready = 1'b1;
  (* ramstyle = "M20K" *) reg [DL-1:0] held [0:(1 << LA) - 1];
  reg [LA-1:0] wp, rp;
  reg [LA:0] stored;
  reg [DL-1:0] got, o0, o1;
  reg fetched;
  reg [1:0] on;
  wire [1:0] kept = on - {1'b0, pop};
  wire fetch = stored != {(LA+1){1'b0}} && kept + {1'b0, fetched} <= 2'd1;
  assign out_valid = on != 2'd0;
  assign out_data = o0;
  always @(posedge clk) begin
    if (mr_valid) held[wp] <= mr_data;
    if (fetch) got <= held[rp];
    if (pop) o0 <= o1;
    if (fetched) begin
      if (kept == 2'd0) o0 <= got;
      else o1 <= got;
    end
    wp <= wp + {{(LA-1){1'b0}}, mr_valid};
    rp <= rp + {{(LA-1){1'b0}}, fetch};
    stored <= stored + {{LA{1'b0}}, mr_valid} - {{LA{1'b0}}, fetch};
    fetched <= fetch;
    on <= kept + {1'b0, fetched};
    if (rst) begin wp <= {LA{1'b0}}; rp <= {LA{1'b0}}; stored <= {(LA+1){1'b0}}; fetched <= 1'b0; on <= 2'd0; end
  end
endmodule
"##;
