//! The streaming engine's units composed from the columns, the feeds and the activation quantizer.
//!
//! `logos_streaming_matvec_<set>` runs a machine matvec job ([`crate::spatial::decode_engine::machine::Job::Matvec`]) from
//! the token-row store to the token-row store, its matrix laid in stripes and streamed from the four memory channels'
//! image streams. It quantizes each token-row's input a weight block at a time, the store's beats through
//! `logos_act_stream` into the column array's activations, then streams the stripes through the channels' feeds into
//! the array, and writes each clock's sums — one stripe's rows for one token-row — into the output slot's row as they
//! leave, so the output row is `numerics::dot` of each matrix row with the quantized input, the machine's own values.
//! A clock's sums are queued in the stripe's row order and written a store beat a clock: the 8N rows of a stripe of N
//! pairs a channel span one beat or several, and a stripe of an odd N starts half a beat in when it is odd.
//!
//! A job's matrix comes off each channel as a [`TileRun`]: the channel's tiles of the tensor, every one but the last the
//! same lines and row bytes, a fixed number of lines and rows apart. `logos_tile_run` walks one into the channel's
//! `logos_image_stream`, which asks the channel for each tile's lines and sends its rows on as beats.

use std::fmt::Write as _;

use crate::model::quant::GgmlType;

use super::super::image::{Image, ImageLayout};
use super::super::machine::Fold;
use super::super::numerics::Words;
use super::column::ColumnSet;
use super::stream::{TileDescriptor, AW, BYW, LW, ROWW, TW};

/// One channel's tiles of one tensor: `count` tiles, the first at line `line`, each `lines` lines holding `bytes` row
/// bytes but the last, `last_lines` holding `last_bytes`; the first's first row `row`, each next `row_step` rows on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileRun {
    pub tensor: u32,
    pub count: u64,
    pub line: u64,
    pub lines: u64,
    pub bytes: u64,
    pub last_lines: u64,
    pub last_bytes: u64,
    pub row: u64,
    pub row_step: u64,
}

impl TileRun {
    /// Tensor `tensor`'s tiles on channel `channel` of `image`.
    pub fn of(image: &Image, tensor: usize, channel: u32) -> TileRun {
        let p = &image.tensors[tensor];
        let c = p.channels.len() as u64;
        let turn = p.channels.iter().position(|&ch| ch == channel).expect("a channel the tensor is dealt to") as u64;
        let tiles = p.tile_count as u64;
        let count = tiles.saturating_sub(turn).div_ceil(c);
        let last = turn + c * count.saturating_sub(1);
        let held = p.rows_per_tile.min(p.rows.saturating_sub(last * p.rows_per_tile));
        TileRun {
            tensor: tensor as u32,
            count,
            line: p.first_line[turn as usize],
            lines: p.tile_lines,
            bytes: p.rows_per_tile * p.image_row_bytes(),
            last_lines: image.layout.lines_for(held * p.image_row_bytes()),
            last_bytes: held * p.image_row_bytes(),
            row: turn * p.rows_per_tile,
            row_step: c * p.rows_per_tile,
        }
    }

    /// The tiles, as the image stream engine takes them.
    pub fn descriptors(&self) -> Vec<TileDescriptor> {
        (0..self.count)
            .map(|k| {
                let last = k + 1 == self.count;
                TileDescriptor {
                    line: self.line + k * self.lines,
                    lines: if last { self.last_lines } else { self.lines },
                    bytes: if last { self.last_bytes } else { self.bytes },
                    tensor: self.tensor,
                    first_row: self.row + k * self.row_step,
                }
            })
            .collect()
    }
}

/// Lines of a streamed tensor's [`StreamEntry`].
pub const STREAM_ENTRY_LINES: u64 = 3;

/// What a hybrid engine's streaming unit takes of one streamed tensor beside its job, as the engine reads it from three
/// lines of the card's memory at the tensor's address: line 0 the tensor's format's place in the unit's set, its blocks
/// a row, its rows, its stripes, each channel's turn (two bits a channel from the lowest), its tag in the image and
/// whether a job of it is greedy, a 32-bit word each from the lowest; lines 1 and 2 the four channels' tile runs,
/// channel `c`'s from bit `256 c` of the two lines' 1024, eight words: its count, line, lines, bytes, last tile's lines
/// and bytes, first row and row step. A greedy job writes each token-row's arg-max of the matrix's rows and that row's
/// value, a beat a token-row, in place of the rows: a head's. The greedy flag is bit 0 of line 0's seventh word, and from
/// bit 1 its two bits are how the job of the tensor folds each sum into its output slot ([`Fold::code`]): a step uses
/// each matrix in one job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamEntry {
    pub fmt: u32,
    pub blocks: u32,
    pub rows: u32,
    pub stripes: u32,
    pub turns: u32,
    pub tensor: u32,
    pub greedy: bool,
    pub fold: Fold,
    pub runs: [TileRun; 4],
}

impl StreamEntry {
    /// Image tensor `k`'s entry for a unit built for `set` over `image`, laid in stripes on four channels: an error
    /// naming what the unit cannot take — a format outside its set, a matrix not laid in stripes, a field past its
    /// port.
    pub fn of(image: &Image, k: usize, set: &ColumnSet) -> Result<StreamEntry, String> {
        let p = &image.tensors[k];
        let pairs = image.layout.stripe.ok_or("an image not laid in stripes")?;
        if !p.block_major || p.pieces != 1 {
            return Err(format!("`{}` is not laid in stripes", p.name));
        }
        if image.layout.channels != 4 || p.channels.len() != 4 {
            return Err(format!("`{}` over {} channels, the unit's four", p.name, p.channels.len()));
        }
        let fmt = set.formats.iter().position(|t| t.name() == p.ty).ok_or_else(|| format!("`{}` in {}, outside the set {:?}", p.name, p.ty, set.formats))?;
        let ty: GgmlType = set.formats[fmt];
        if p.block_bytes != set.block_bytes(ty) as u64 {
            return Err(format!("`{}` laid in blocks of {} bytes, the unit's of {}", p.name, p.block_bytes, set.block_bytes(ty)));
        }
        if p.slot_bytes() != set.slot_bytes(ty) as u64 {
            return Err(format!("`{}` laid in slots of {} bytes, the unit's of {}", p.name, p.slot_bytes(), set.slot_bytes(ty)));
        }
        let mut turns = 0u32;
        for (turn, &ch) in p.channels.iter().enumerate() {
            turns |= (turn as u32) << (2 * ch);
        }
        let runs = [0, 1, 2, 3].map(|ch| TileRun::of(image, k, ch));
        let fits = |what: &str, v: u64, bits: u32| if v >> bits == 0 { Ok(v as u32) } else { Err(format!("`{}`'s {what} {v} past {bits} bits", p.name)) };
        for r in &runs {
            fits("tile count", r.count, 32)?;
            fits("line", r.line + r.count * r.lines, AW)?;
            fits("tile lines", r.lines.max(r.last_lines), LW)?;
            fits("tile bytes", r.bytes.max(r.last_bytes), BYW)?;
            fits("rows", r.row + r.count * r.row_step, ROWW)?;
        }
        Ok(StreamEntry {
            fmt: fmt as u32,
            blocks: fits("blocks a row", p.row_bytes / set.block_bytes(ty) as u64, 16)?,
            rows: fits("rows", p.rows, 32)?,
            stripes: fits("stripes", p.rows.div_ceil(8 * pairs), 16)?,
            turns,
            tensor: fits("tag", k as u64, TW)?,
            greedy: false,
            fold: Fold::Put,
            runs,
        })
    }

    /// The entry's three lines, as the engine reads them.
    pub fn lines(&self) -> [[u8; 64]; 3] {
        let mut words = [0u32; 48];
        let flags = self.greedy as u32 | (self.fold.code() as u32) << 1;
        words[..7].copy_from_slice(&[self.fmt, self.blocks, self.rows, self.stripes, self.turns, self.tensor, flags]);
        for (c, r) in self.runs.iter().enumerate() {
            let at = 16 + 8 * c;
            words[at..at + 8].copy_from_slice(&[r.count, r.line, r.lines, r.bytes, r.last_lines, r.last_bytes, r.row, r.row_step].map(|v| v as u32));
        }
        let mut lines = [[0u8; 64]; 3];
        for (i, w) in words.iter().enumerate() {
            lines[i / 16][4 * (i % 16)..4 * (i % 16) + 4].copy_from_slice(&w.to_le_bytes());
        }
        lines
    }
}

/// `logos_tile_run`: a [`TileRun`] walked into tile descriptors, one a handshake, from `start` until its last is taken.
pub fn tile_run_verilog() -> String {
    format!(
        r#"// One channel's tiles of one tensor, walked into tile descriptors: `count` tiles from line `line`, `lines` lines and
// `bytes` row bytes each but the last's `last_lines` and `last_bytes`, the first row `row` and `row_step` more each.
module logos_tile_run
  (input clk, input rst, input start, input [31:0] count, input [{a}:0] line, input [{l}:0] lines, input [{b}:0] bytes,
   input [{l}:0] last_lines, input [{b}:0] last_bytes, input [{r}:0] row, input [{r}:0] row_step, input [{t}:0] tensor,
   output tile_valid, input tile_ready, output [{a}:0] tile_line, output [{l}:0] tile_lines, output [{b}:0] tile_bytes,
   output [{t}:0] tile_tensor, output [{r}:0] tile_row, output busy);
  reg on;
  reg [31:0] k;
  reg [{a}:0] at;
  reg [{r}:0] first;
  wire last = k + 32'd1 == count;
  assign tile_valid = on;
  assign tile_line = at;
  assign tile_lines = last ? last_lines : lines;
  assign tile_bytes = last ? last_bytes : bytes;
  assign tile_tensor = tensor;
  assign tile_row = first;
  assign busy = on;
  always @(posedge clk) begin
    if (rst) on <= 1'b0;
    else if (start) begin on <= count != 32'd0; k <= 32'd0; at <= line; first <= row; end
    else if (on && tile_ready) begin
      if (last) on <= 1'b0;
      k <= k + 32'd1; at <= at + lines[{a}:0]; first <= first + row_step;
    end
  end
endmodule
"#,
        a = AW - 1,
        l = LW - 1,
        b = BYW - 1,
        r = ROWW - 1,
        t = TW - 1
    )
}

/// Clocks from a value to its code in `logos_act_code_p`.
pub const ACT_CODE_LATENCY: usize = 20;

/// Clocks from a lone block's last beat into `logos_act_blocks_p` to its first beat's read out of the ring: the beat's
/// maximum, the block's, its scale, its place in the queue and the reader's start.
pub const ACT_BLOCKS_READ_LATENCY: usize = 4 + super::act_quant::WIDE_SCALE_P_LATENCY;

/// The clock of each block's `out_v` from `logos_act_blocks_p`, for blocks given in order as the clock of each one's
/// last beat and its beats: a block is read a beat a clock from [`ACT_BLOCKS_READ_LATENCY`] clocks after its last
/// beat or from the clock after the block before it is read, whichever is later, and its `out_v` is the clock after its
/// last beat's codes land, [`ACT_CODE_LATENCY`] and the read's clock after that beat's read.
pub fn act_blocks_out(blocks: &[(usize, usize)]) -> Vec<usize> {
    let mut free = 0;
    blocks
        .iter()
        .map(|&(last, beats)| {
            let start = (last + ACT_BLOCKS_READ_LATENCY).max(free);
            free = start + beats;
            start + beats + ACT_CODE_LATENCY + 1
        })
        .collect()
}

/// The streaming engine's activation quantizers with the code unit `logos_act_code_p`: `logos_act_stream_p`
/// (`logos_act_stream` with its code units pipelined and its beats in registered-read block RAM), a block at a time,
/// and `logos_act_blocks_p`, blocks back to back, each block's codes out a beat a clock behind its scale; the
/// reference quantizer's modules beside them.
pub fn act_stream_p_verilog() -> String {
    format!(
        "{}\n{}\n{}",
        super::act_quant::act_quant_verilog(),
        super::ram::bram_verilog(&[512]),
        ACT_STREAM_P
            .replace("@LAT@", &ACT_CODE_LATENCY.to_string())
            .replace("@WS@", &super::act_quant::WIDE_SCALE_P_LATENCY.to_string())
    )
}

/// Clocks from a beat into `logos_greedy` to its token-row's best: the four levels of its tree and the update.
pub const GREEDY_LATENCY: usize = 5;

/// `logos_greedy`, a greedy head's reduction of a matvec job's sums to each token-row's first largest.
pub fn greedy_verilog() -> String {
    format!("{}{GREEDY}", super::ram::RAMSTYLE)
}

/// Clocks from a fold lane's operands to its element (`logos_fold_lane_p`): the function unit's SiLU and the product
/// after it, the sum held to them.
pub const FOLD_LATENCY: usize = super::simd::LANE_FUNC_P_LATENCY + super::fp32::FMUL_LATENCY;

/// The streaming matvec of each named format set on channels of `layout`'s lines, with everything it is built from.
pub fn streaming_verilog(sets: &[ColumnSet], layout: &ImageLayout) -> String {
    streaming_verilog_folding(sets, layout, 0)
}

/// [`streaming_verilog`] with each unit folding a job's sums into its output slot over `folds` fold lanes, 1, 2, 4, 8
/// or 16 ([`fold_unit`]); 0 a unit that writes each sum as it is, [`streaming_verilog`]'s.
pub fn streaming_verilog_folding(sets: &[ColumnSet], layout: &ImageLayout, folds: usize) -> String {
    assert!(folds == 0 || [1, 2, 4, 8, 16].contains(&folds), "a fold over 1, 2, 4, 8 or 16 lanes, not {folds}");
    let mut v = format!(
        "{}\n{}\n{}\n{}\n{}",
        super::column::columns_verilog(sets),
        act_stream_p_verilog(),
        super::stream::verilog(layout),
        tile_run_verilog(),
        greedy_verilog()
    );
    let dw = 8 * layout.line_bytes;
    for set in sets {
        let ColumnSet { name, formats, words, .. } = set;
        let wide = *words == Words::Two;
        let elems = formats.iter().map(|t| t.block_len()).max().expect("a set of formats");
        let multi = formats.len() > 1;
        // An octet job quantizes each of its octets' eight blocks as its own and writes it into its octet's word.
        let (octet_job, act_block, quant_blocks) = match formats.iter().position(|&t| set.octet(t)) {
            Some(place) => (
                format!(
                    "  // An octet job's quantized blocks: eight to an octet, block b into its octet's word b / 8 at slot b mod 8, its\n  \
                     // codes in the slot's 32 lanes.\n  wire oct = {};\n  wire [15:0] qblocks = oct ? {{blocks[12:0], 3'd0}} : blocks;\n",
                    if multi { format!("fmt_q == 4'd{place}") } else { "1'b1".to_string() }
                ),
                ".act_b(oct ? {3'd0, ob[15:3]} : ob), .act_s(oct ? ob[2:0] : 3'd0), .act_codes(oct ? (aq_codes & keep) << (512 * ob[2:0]) : aq_codes & keep)".to_string(),
                "qblocks",
            ),
            None => (String::new(), ".act_b(ob), .act_codes(aq_codes & keep)".to_string(), "blocks"),
        };
        let mut lens = String::new();
        if multi {
            lens.push_str("  reg [15:0] blen;\n  always @*\n    case (fmt_q)\n");
            for (i, t) in formats.iter().enumerate() {
                writeln!(lens, "      4'd{i}: blen = 16'd{};", t.block_len()).unwrap();
            }
            writeln!(lens, "      default: blen = 16'd{};\n    endcase", formats[0].block_len()).unwrap();
        } else {
            writeln!(lens, "  wire [15:0] blen = 16'd{};", formats[0].block_len()).unwrap();
        }
        writeln!(
            v,
            r#"
// A {what} matvec job from the token-row store to the token-row store, its matrix's stripes from the four channels. The
// input slot's row t holds token-row t's input from beat from_base + t * from_beats, the output slot's from to_base + t *
// to_beats; the store's word for an address is out SRL clocks after it. Each weight block of each token-row's input is read a
// beat a clock into the quantizer, block after block, and each block's codes, zero past the block, written to the
// array's activations as the quantizer gives them; then each
// channel's tile run (packed a field a channel) walks into its image stream, which reads the tiles' lines off the
// channel's port, the array runs the stripes, and each clock's sums, one stripe's rows for one token-row, are written
// into that token-row's output row under a mask of the rows the matrix has. A greedy job (a head's) writes no row:
// each beat it would write goes to logos_greedy, which keeps every token-row's first largest number so far (a row's
// sums arrive in row order), and once the last is through, each token-row's beat is its row and its value (lanes 0
// and 1), numerics::argmax of the token-row's sums.
module logos_streaming_matvec_{name} #(parameter N = 1, T_MAX = 4, NB_MAX = 16, SA = 12, SRL = 1)
  (input clk,{clk2_in} input rst,
   input start, input greedy{fmt_in}, input [15:0] blocks, input [7:0] tokens{two_in}, input [31:0] rows, input [15:0] stripes,
   input [7:0] turns, input [SA-1:0] from_base, input [15:0] from_beats, input [SA-1:0] to_base, input [15:0] to_beats,
   output busy,
   output [SA-1:0] st_ra, input [511:0] st_rd,
   output reg st_we, output reg [SA-1:0] st_wa, output reg [511:0] st_wd, output reg [63:0] st_wm,
   input [127:0] tr_count, input [4*{aw}-1:0] tr_line, input [4*{lw}-1:0] tr_lines, input [4*{byw}-1:0] tr_bytes,
   input [4*{lw}-1:0] tr_last_lines, input [4*{byw}-1:0] tr_last_bytes, input [4*{roww}-1:0] tr_row,
   input [4*{roww}-1:0] tr_row_step, input [{tw}-1:0] tensor,
   output [3:0] md_valid, input [3:0] md_ready, output [4*{mdw}-1:0] md_data,
   input [3:0] mr_valid, output [3:0] mr_ready, input [4*{dw}-1:0] mr_data);
  // The queue holds a burst of a stripe's token-rows beside every sum the array has in flight: the array holds its
  // issues while the queue could not take them all.
  localparam C = 4 * N, E = 8 * N, TAGS = {tags}, QD = T_MAX + TAGS + 1, QA = $clog2(QD + 1);
{two_decl}  localparam [31:0] ROWS_A_STRIPE = E;
  localparam [QA-1:0] QLAST = QD - 1, HOLD_AT = T_MAX;
  reg [QA-1:0] qh, qt, qn;
  localparam IDLE = 3'd0, READ = 3'd1, QUANT = 3'd2, RUN = 3'd3, DRAIN = 3'd4, BEST = 3'd5;
  reg greedy_r;
{fmt_copy}{lens}  wire [15:0] bpb = blen >> 4;
  reg [2:0] state;
  reg [7:0] t;
  reg [15:0] b, j;
  // The beat at hand: token-row t's beat b·bpb + j from its row's first, kept as offsets from the input slot's.
  reg [31:0] in_row0, in_at;
  assign st_ra = from_base + in_at[SA-1:0];
  // A beat asked carries its block's marks and place (arriving, first, last, in_t, in_b, the clock after its address) to
  // the clock its word is out of the store, SRL clocks after its address (landed_*).
  reg arriving, first, last;
  reg [7:0] in_t;
  reg [15:0] in_b;
  wire landed, landed_first, landed_last, landing;
  wire [7:0] landed_t;
  wire [15:0] landed_b;
  logos_landing #(.W(26), .D(SRL - 1)) land(.clk(clk), .rst(rst), .in_v(arriving), .in_d({{first, last, in_t, in_b}}),
    .out_v(landed), .out_d({{landed_first, landed_last, landed_t, landed_b}}), .flight(landing));
  // The input's beats into the quantizer block after block, and each block's codes, scale and place out of it in order,
  // written into the array's activations as they come.
  wire aq_busy, aq_out;
  wire [7:0] aq_m;
  wire signed [11:0] aq_e;
  wire [23:0] aq_tag;
  wire [{codes_top}:0] aq_codes;
  wire [7:0] ot = aq_tag[23:16];
  wire [15:0] ob = aq_tag[15:0];
  logos_act_blocks_p #(.LANES({elems}), .TW(24)) aq(.clk(clk), .rst(rst), .in_v(landed), .in_beat(st_rd),
    .in_first(landed_first), .in_last(landed_last), .in_two(two), .in_tag({{landed_t, landed_b}}), .busy(aq_busy), .out_v(aq_out),
    .out_m(aq_m), .out_e(aq_e), .out_tag(aq_tag), .codes(aq_codes));
  // Which of the quantizer's codes are its block's, by the job's format, a register beside its loads: settled two clocks
  // after the job starts, long before the job's first codes are out.
  reg [{codes_top}:0] keep;
  integer code;
  always @(posedge clk) for (code = 0; code < {elems}; code = code + 1) keep[16*code +: 16] <= (code[15:0] < blen) ? 16'hffff : 16'h0000;
  wire act_wr = aq_out;
{octet_job}
  // The array and the channels' feeds.
  reg run;
  wire array_busy;
  wire [C-1:0] w_valid, w_ready, w_put, o_valid;
  wire [C*{beat}-1:0] w_beat;
  wire [C*32-1:0] o_val1, o_val2;
  wire [15:0] o_stripe;
  wire [7:0] o_t;
  logos_column_array_{name} #(.CH(4), .N(N), .T_MAX(T_MAX), .NB_MAX(NB_MAX)) u(.clk(clk), {clk2_pass}.rst(rst), .start(run),
    .stripes(stripes), .blocks(blocks), .tokens(tokens), {two_pass}{fmt_pass}.rows(rows), .turns(turns), .hold(qn > HOLD_AT),
    .busy(array_busy),
    .act_wr(act_wr), .act_t(ot), {act_block}, .act_m(aq_m), .act_e(aq_e), .w_valid(w_valid),
    .w_ready(w_ready), .w_put(w_put), .w_beat(w_beat), .o_valid(o_valid), .o_val1(o_val1), .o_val2(o_val2),
    .o_stripe(o_stripe), .o_t(o_t));
  // Each channel: its tile run, its image stream, its feed.
  wire [3:0] runs_busy;
  genvar ch;
  generate
    for (ch = 0; ch < 4; ch = ch + 1) begin : chan
      wire tile_valid, tile_ready, in_valid, in_ready, in_last, in_crc;
      wire [{aw}-1:0] tile_line;
      wire [{lw}-1:0] tile_lines;
      wire [{byw}-1:0] tile_bytes, in_offset;
      wire [{tw}-1:0] tile_tensor, in_tensor;
      wire [{roww}-1:0] tile_row, in_row;
      wire [511:0] in_data;
      wire [6:0] in_keep;
      logos_tile_run walk(.clk(clk), .rst(rst), .start(run), .count(tr_count[32*ch +: 32]), .line(tr_line[{aw}*ch +: {aw}]),
        .lines(tr_lines[{lw}*ch +: {lw}]), .bytes(tr_bytes[{byw}*ch +: {byw}]), .last_lines(tr_last_lines[{lw}*ch +: {lw}]),
        .last_bytes(tr_last_bytes[{byw}*ch +: {byw}]), .row(tr_row[{roww}*ch +: {roww}]), .row_step(tr_row_step[{roww}*ch +: {roww}]),
        .tensor(tensor), .tile_valid(tile_valid), .tile_ready(tile_ready), .tile_line(tile_line), .tile_lines(tile_lines),
        .tile_bytes(tile_bytes), .tile_tensor(tile_tensor), .tile_row(tile_row), .busy(runs_busy[ch]));
      logos_image_stream stream(.clk(clk), .rst_n(!rst), .tile_valid(tile_valid), .tile_ready(tile_ready),
        .tile_line(tile_line), .tile_lines(tile_lines), .tile_bytes(tile_bytes), .tile_tensor(tile_tensor),
        .tile_row(tile_row), .md_valid(md_valid[ch]), .md_ready(md_ready[ch]), .md_data(md_data[{mdw}*ch +: {mdw}]),
        .mr_valid(mr_valid[ch]), .mr_ready(mr_ready[ch]), .mr_data(mr_data[{dw}*ch +: {dw}]), .out_valid(in_valid),
        .out_ready(in_ready), .out_data(in_data), .out_keep(in_keep), .out_last(in_last), .out_tensor(in_tensor),
        .out_row(in_row), .out_offset(in_offset), .out_crc_error(in_crc));
      logos_column_feed_{name} #(.N(N)) feed(.clk(clk), .rst(rst), {fmt_pass}.blocks(blocks), .rows(rows),
        .in_valid(in_valid), .in_ready(in_ready), .in_data(in_data), .in_keep(in_keep), .in_row(in_row),
        .in_offset(in_offset), .w_valid(w_valid[ch*N +: N]), .w_ready(w_ready[ch*N +: N]), .w_put(w_put[ch*N +: N]),
        .w_beat(w_beat[ch*N*{beat} +: N*{beat}]));
    end
  endgenerate
  // A clock's sums in the stripe's row order: core ch * N + p holds rows turn * 2N + 2p and the next of the stripe's 8N,
  // a row past the matrix or of a core without a pair not one. The turns deal each block of 2N rows to one channel, so a
  // block's sums are its channel's, chosen among the four by the turn alone; a row is the matrix's when it is among the
  // rows the stripe leaves of it.
  reg [32*E-1:0] ordered;
  reg [E-1:0] held;
  wire [31:0] base = o_stripe * E;
  wire [31:0] rows_left = (rows > base) ? rows - base : 32'd0;
  integer ob_blk, ob_ch, ob_row;
  always @* begin
    ordered = {{(32*E){{1'b0}}}};
    held = {{E{{1'b0}}}};
    for (ob_blk = 0; ob_blk < 4; ob_blk = ob_blk + 1)
      for (ob_ch = 0; ob_ch < 4; ob_ch = ob_ch + 1)
        if (turns[2*ob_ch +: 2] == ob_blk[1:0])
          for (ob_row = 0; ob_row < 2 * N; ob_row = ob_row + 1) begin
            ordered[32*(2*N*ob_blk + ob_row) +: 32] = ordered[32*(2*N*ob_blk + ob_row) +: 32]
              | (ob_row[0] ? o_val2[32*(ob_ch*N + ob_row/2) +: 32] : o_val1[32*(ob_ch*N + ob_row/2) +: 32]);
            held[2*N*ob_blk + ob_row] = held[2*N*ob_blk + ob_row] | (o_valid[ob_ch*N + ob_row/2] && $unsigned(2*N*ob_blk + ob_row) < rows_left);
          end
  end
  // The queue of a token-row's stripe of sums, each written a store beat a clock: the beats from the one the stripe's
  // first row lies in to the one its last does.
  `LOGOS_RAMSTYLE_ASYNC(QD) reg [32*E-1:0] q_val [0:QD-1];
  `LOGOS_RAMSTYLE_ASYNC(QD) reg [E-1:0] q_held [0:QD-1];
  `LOGOS_RAMSTYLE_ASYNC(QD) reg [7:0] q_t [0:QD-1];
  `LOGOS_RAMSTYLE_ASYNC(QD) reg [15:0] q_s [0:QD-1];
  reg [15:0] beat;
  wire [31:0] e0 = q_s[qh] * ROWS_A_STRIPE;
  wire [31:0] skew = e0 % 32'd16;
  wire [31:0] beats = (skew + ROWS_A_STRIPE + 32'd15) / 32'd16;
  reg [511:0] lanes;
  reg [63:0] mask;
  reg [31:0] el, place;
  integer l;
  always @* begin
    lanes = 512'd0;
    mask = 64'd0;
    for (l = 0; l < 16; l = l + 1) begin
      place = {{16'd0, beat}} * 32'd16 + l[31:0];
      el = place - skew;
      if (place >= skew && el < ROWS_A_STRIPE && q_held[qh][el]) begin
        lanes[32*l +: 32] = q_val[qh][32*el +: 32];
        mask[4*l +: 4] = 4'hf;
      end
    end
  end
  wire writing = qn != {{QA{{1'b0}}}};
  wire done_entry = writing && {{16'd0, beat}} + 32'd1 == beats;
  wire [31:0] out_at = q_t[qh] * to_beats + e0 / 32'd16 + {{16'd0, beat}};
  // The greedy head: each beat the writer would write, into the reduction to every token-row's first largest so far.
  wire g_busy;
  wire [31:0] b_row, b_val;
  reg [15:0] present;
  integer pl;
  always @* for (pl = 0; pl < 16; pl = pl + 1) present[pl] = mask[4*pl];
  logos_greedy #(.T_MAX(T_MAX)) head(.clk(clk), .rst(rst), .clear(state == IDLE && start), .in_valid(writing && greedy_r),
    .in_t(q_t[qh]), .in_at(e0 / 32'd16 + {{16'd0, beat}}), .in_lanes(lanes), .in_present(present), .busy(g_busy), .rd_t(t),
    .rd_row(b_row), .rd_val(b_val));
  wire [31:0] best_at = t * to_beats;
  assign busy = state != IDLE || array_busy || writing || st_we;
  always @(posedge clk) begin
    st_we <= (writing && !greedy_r) || state == BEST;
    st_wa <= to_base + (state == BEST ? best_at[SA-1:0] : out_at[SA-1:0]);
    st_wd <= state == BEST ? {{448'd0, b_val, b_row}} : lanes;
    st_wm <= state == BEST ? 64'h0000_0000_0000_00ff : mask;
    if (|o_valid) begin
      q_val[qt] <= ordered; q_held[qt] <= held; q_t[qt] <= o_t; q_s[qt] <= o_stripe;
      qt <= (qt == QLAST) ? {{QA{{1'b0}}}} : qt + 1'b1;
    end
    if (done_entry) begin beat <= 16'd0; qh <= (qh == QLAST) ? {{QA{{1'b0}}}} : qh + 1'b1; end
    else if (writing) beat <= beat + 16'd1;
    qn <= qn + (|o_valid) - done_entry;
    if (rst) begin qh <= {{QA{{1'b0}}}}; qt <= {{QA{{1'b0}}}}; qn <= {{QA{{1'b0}}}}; beat <= 16'd0; end
    run <= 1'b0;
    arriving <= 1'b0;
    if (rst) state <= IDLE;
    else case (state)
      IDLE: if (start) begin
        t <= 8'd0; b <= 16'd0; j <= 16'd0; in_row0 <= 32'd0; in_at <= 32'd0; greedy_r <= greedy; state <= READ;
      end
      READ: begin
        arriving <= 1'b1; first <= j == 16'd0; last <= j + 16'd1 == bpb; in_t <= t; in_b <= b;
        in_at <= in_at + 32'd1;
        if (j + 16'd1 != bpb) j <= j + 16'd1;
        else begin
          j <= 16'd0;
          if (b + 16'd1 < {quant_blocks}) b <= b + 16'd1;
          else begin
            b <= 16'd0; in_row0 <= in_row0 + {{16'd0, from_beats}}; in_at <= in_row0 + {{16'd0, from_beats}};
            if (t + 8'd1 < tokens) t <= t + 8'd1; else state <= QUANT;
          end
        end
      end
      QUANT: if (!arriving && !landing && !aq_busy) begin run <= 1'b1; state <= RUN; end
      RUN: if (!run && !array_busy) state <= greedy_r ? DRAIN : IDLE;
      DRAIN: if (!writing && !g_busy) begin t <= 8'd0; state <= BEST; end
      BEST: begin
        t <= t + 8'd1;
        if (t + 8'd1 == tokens) state <= IDLE;
      end
      default: state <= IDLE;
    endcase
  end
endmodule"#,
            what = formats.iter().map(|t| t.name()).collect::<Vec<_>>().join("/"),
            fmt_in = if multi { ", input [3:0] fmt" } else { "" },
            fmt_pass = if multi { ".fmt(fmt), " } else { "" },
            fmt_copy = if multi { super::column::FMT_COPY } else { "" },
            clk2_in = if set.pe_pairs { " input clk2," } else { "" },
            clk2_pass = if set.pe_pairs { ".clk2(clk2), " } else { "" },
            two_in = if wide { ", input two" } else { "" },
            two_pass = if wide { ".two(two), " } else { "" },
            two_decl = if wide { "" } else { "  wire two = 1'b0;\n" },
            codes_top = 16 * elems - 1,
            octet_job = octet_job,
            act_block = act_block,
            quant_blocks = quant_blocks,
            beat = 2 * set.decoded_bits(),
            aw = AW,
            lw = LW,
            byw = BYW,
            roww = ROWW,
            tw = TW,
            mdw = AW + LW + 1,
            dw = dw,
            tags = super::column::COLUMN_VALUE_LATENCY + super::fp32::FADD_LATENCY,
        )
        .unwrap();
        if set.token_pairs {
            let head = format!("\nmodule logos_streaming_matvec_{name} ");
            let at = v.rfind(&head).expect("the set's unit");
            let end = at + v[at..].find("endmodule").expect("the unit's end") + "endmodule".len();
            let unit = token_pair_unit(&v[at..end]);
            v.replace_range(at..end, &unit);
        }
    }
    if folds == 0 {
        return v;
    }
    let units = v.matches("\nmodule logos_streaming_matvec_").count();
    assert_eq!(units, sets.len(), "a unit a set");
    let mut out = String::new();
    let mut rest = v.as_str();
    while let Some(at) = rest.find("\nmodule logos_streaming_matvec_") {
        let end = at + rest[at..].find("endmodule").expect("the unit's end") + "endmodule".len();
        out.push_str(&rest[..at]);
        out.push_str(&fold_unit(&rest[at..end], folds));
        rest = &rest[end..];
    }
    out.push_str(rest);
    format!("{out}\n{}\n{}", super::simd::simd_p_verilog(), fold_lane_verilog())
}

/// The unit `unit` (one `logos_streaming_matvec_<set>`) of a set of token pairs
/// ([`ColumnSet::token_pairs`](super::column::ColumnSet::token_pairs)): each clock's sums are a stripe's rows for the token
/// pair from `o_t`, its second token-row's ordered as its first's and queued beside them in one entry, which holds the
/// second only when the job has it (`q_two`); the writer then writes the entry's first token-row's beats and then its
/// second's (`half`, the beat within its token-row `tbeat`), so the fold and the greedy head take them as two entries'.
fn token_pair_unit(unit: &str) -> String {
    let ordered = {
        let at = unit.find("  always @* begin\n    ordered = ").expect("the stripe's order");
        &unit[at..at + unit[at..].find("\n  end\n").expect("its end") + "\n  end\n".len()]
    };
    let ordered_b: String = ordered
        .split_inclusive('\n')
        .filter(|l| !l.contains("held"))
        .collect::<String>()
        .replace("ordered", "ordered_b")
        .replace("o_val1[", "o_val1_b[")
        .replace("o_val2[", "o_val2_b[")
        .replace("ob_blk", "ob_blk_b")
        .replace("ob_ch", "ob_ch_b")
        .replace("ob_row", "ob_row_b");
    let mut u = unit.to_string();
    let mut swap = |a: &str, b: &str| {
        assert_eq!(u.matches(a).count(), 1, "the unit's `{a}`");
        u = u.replacen(a, b, 1);
    };
    swap("  wire [C*32-1:0] o_val1, o_val2;\n", "  wire [C*32-1:0] o_val1, o_val2, o_val1_b, o_val2_b;\n");
    swap(".o_val1(o_val1), .o_val2(o_val2),", ".o_val1(o_val1), .o_val2(o_val2), .o_val1_b(o_val1_b), .o_val2_b(o_val2_b),");
    swap(
        ordered,
        &format!(
            "{ordered}  // The pair's second token-row's sums in the same order, its rows the first's.\n  reg [32*E-1:0] ordered_b;\n  \
             integer ob_blk_b, ob_ch_b, ob_row_b;\n{ordered_b}"
        ),
    );
    swap(
        "  `LOGOS_RAMSTYLE_ASYNC(QD) reg [32*E-1:0] q_val [0:QD-1];\n",
        "  `LOGOS_RAMSTYLE_ASYNC(QD) reg [32*E-1:0] q_val [0:QD-1];\n  `LOGOS_RAMSTYLE_ASYNC(QD) reg [32*E-1:0] q_val_b [0:QD-1];\n  \
         `LOGOS_RAMSTYLE_ASYNC(QD) reg q_two [0:QD-1];\n",
    );
    swap(
        "q_val[qt] <= ordered; q_held[qt] <= held; q_t[qt] <= o_t; q_s[qt] <= o_stripe;",
        "q_val[qt] <= ordered; q_held[qt] <= held; q_t[qt] <= o_t; q_s[qt] <= o_stripe;\n      \
         q_val_b[qt] <= ordered_b; q_two[qt] <= o_t + 8'd1 < tokens;",
    );
    swap(
        "  wire [31:0] beats = (skew + ROWS_A_STRIPE + 32'd15) / 32'd16;\n",
        "  wire [31:0] tbeats = (skew + ROWS_A_STRIPE + 32'd15) / 32'd16;\n  wire [31:0] beats = q_two[qh] ? 2 * tbeats : tbeats;\n  \
         // The entry's beat within its token-row, and whether it is its second token-row's.\n  \
         wire half = q_two[qh] && {16'd0, beat} >= tbeats;\n  wire [15:0] tbeat = half ? beat - tbeats[15:0] : beat;\n",
    );
    swap("      place = {16'd0, beat} * 32'd16 + l[31:0];\n", "      place = {16'd0, tbeat} * 32'd16 + l[31:0];\n");
    swap("        lanes[32*l +: 32] = q_val[qh][32*el +: 32];\n", "        lanes[32*l +: 32] = half ? q_val_b[qh][32*el +: 32] : q_val[qh][32*el +: 32];\n");
    swap(
        "  wire [31:0] out_at = q_t[qh] * to_beats + e0 / 32'd16 + {16'd0, beat};\n",
        "  wire [31:0] out_at = (q_t[qh] + half) * to_beats + e0 / 32'd16 + {16'd0, tbeat};\n",
    );
    swap(".in_t(q_t[qh]), .in_at(e0 / 32'd16 + {16'd0, beat}),", ".in_t(q_t[qh] + half), .in_at(e0 / 32'd16 + {16'd0, tbeat}),");
    u
}

/// `logos_fold_lane_p`, a fold's element from the element its slot held and the sum ([`Fold::apply`]).
pub fn fold_lane_verilog() -> String {
    use super::fp32::FADD_LATENCY;
    use super::simd::LANE_FUNC_P_LATENCY;
    assert!(FADD_LATENCY <= FOLD_LATENCY, "the sum is held to the product");
    FOLD_LANE
        .replace("@LANE@", &LANE_FUNC_P_LATENCY.to_string())
        .replace("@ADD_HOLD@", &(FOLD_LATENCY - FADD_LATENCY).to_string())
        .replace("@FOLD@", &FOLD_LATENCY.to_string())
}

const FOLD_LANE: &str = r#"
// A fold's lane (machine::Fold): the element a sum leaves where its slot held old, @FOLD@ clocks on — fc 1 old + sum,
// fc 2 the SiLU of old times sum, the function unit's SiLU then the product, as the vector unit computes each; the
// other fold's value is held beside it to the same clock and the job's fold chooses.
module logos_fold_lane_p(input clk, input [1:0] fc, input [31:0] old, input [31:0] sum, output [31:0] y);
  wire [31:0] gate, held, product, total, total_held;
  wire [1:0] fc_held;
  logos_lane_func_p silu(.clk(clk), .f(4'd4), .x(old), .y(gate));
  logos_delay #(.W(32), .D(@LANE@)) hold_sum(.clk(clk), .d(sum), .q(held));
  logos_fmul_p times(.clk(clk), .a(gate), .b(held), .y(product));
  logos_fadd_p plus(.clk(clk), .a(old), .b(sum), .y(total));
  logos_delay #(.W(32), .D(@ADD_HOLD@)) hold_total(.clk(clk), .d(total), .q(total_held));
  logos_delay #(.W(2), .D(@FOLD@)) hold_fc(.clk(clk), .d(fc), .q(fc_held));
  assign y = fc_held == 2'd2 ? product : total_held;
endmodule
"#;

/// The unit `unit` (one `logos_streaming_matvec_<set>`) folding a job's sums into its output slot over `lanes` fold
/// lanes: the job's fold from its stream entry (`fold`, held from the job's start); a job that folds takes a beat off
/// its queue's head each `16 / lanes` clocks, its address registered, the store's beat there read, then `lanes`
/// elements a clock of the old elements and the sums through `logos_fold_lane_p`, the beat written under its lanes once
/// its last slice is out; the unit busy while a beat is on its way.
fn fold_unit(unit: &str, lanes: usize) -> String {
    let slices = 16 / lanes;
    let in_flight = (FOLD_LATENCY + slices).div_ceil(slices) + 3;
    let depth = in_flight.next_power_of_two();
    let qa = depth.trailing_zeros() as usize;
    let mut u = unit.to_string();
    let mut swap = |a: &str, b: &str| {
        assert_eq!(u.matches(a).count(), 1, "the unit's `{a}`");
        u = u.replacen(a, b, 1);
    };
    swap("input start, input greedy", "input start, input greedy, input [1:0] fold");
    swap("  assign st_ra = from_base + in_at[SA-1:0];", "  reg f0_v;\n  reg [SA-1:0] f0_addr;\n  assign st_ra = f0_v ? f0_addr : from_base + in_at[SA-1:0];");
    swap(
        "  wire done_entry = writing && {16'd0, beat} + 32'd1 == beats;",
        "  reg fold_on;\n  wire f_take;\n  wire adv = writing && (!fold_on || f_take);\n  wire done_entry = adv && {16'd0, beat} + 32'd1 == beats;",
    );
    let path = FOLD_PATH
        .replace("@FL@", &lanes.to_string())
        .replace("@S@", &slices.to_string())
        .replace("@SLAST@", &(slices - 1).to_string())
        .replace("@FOLD@", &FOLD_LATENCY.to_string())
        .replace("@QD@", &depth.to_string())
        .replace("@QA@", &qa.to_string())
        .replace("@ONEXT@", &if lanes == 16 { "fy".to_string() } else { format!("{{fy, o_beat[511:{}]}}", 32 * lanes) });
    swap("  // The greedy head: each beat the writer would write", &format!("{path}  // The greedy head: each beat the writer would write"));
    swap(
        "  assign busy = state != IDLE || array_busy || writing || st_we;",
        "  assign busy = state != IDLE || array_busy || writing || st_we || f_busy;",
    );
    swap("    st_we <= (writing && !greedy_r) || state == BEST;", "    st_we <= (writing && !greedy_r && !fold_on) || state == BEST || fo_end;");
    swap(
        "    st_wa <= to_base + (state == BEST ? best_at[SA-1:0] : out_at[SA-1:0]);",
        "    st_wa <= fo_end ? fq_head[SA-1:0] : to_base + (state == BEST ? best_at[SA-1:0] : out_at[SA-1:0]);",
    );
    swap("    st_wd <= state == BEST ? {448'd0, b_val, b_row} : lanes;", "    st_wd <= fo_end ? o_next : state == BEST ? {448'd0, b_val, b_row} : lanes;");
    swap(
        "    st_wm <= state == BEST ? 64'h0000_0000_0000_00ff : mask;",
        "    st_wm <= fo_end ? fq_head[SA+63:SA] : state == BEST ? 64'h0000_0000_0000_00ff : mask;",
    );
    swap("    else if (writing) beat <= beat + 16'd1;", "    else if (adv) beat <= beat + 16'd1;");
    swap("greedy_r <= greedy; state <= READ;", "greedy_r <= greedy; fold_on <= fold != 2'd0; fold_r <= fold; state <= READ;");
    u
}

const FOLD_PATH: &str = r#"  // The job's fold (machine::Fold, fold_r from its start): 0 writes each beat as it leaves the queue; 1 and 2 fold each
  // sum into the element of the output slot it lands on. A beat leaves the queue for the fold each @S@ clocks, its
  // address registered (f0), the store's beat there out SRL clocks on (f1); then @FL@ lanes of logos_fold_lane_p take it @FL@
  // elements a clock, the old elements and the sums shifted down a slice each clock; each beat's folded slices come
  // out in order @FOLD@ clocks on, and the beat is written under its lanes as its last comes out, its lanes and address
  // held in order in fq. Each slice's mark rides reset registers beside the lanes.
  reg [1:0] fold_r;
  reg [7:0] f_gap;
  assign f_take = fold_on && writing && f_gap == 8'd0 && state != READ;
  reg f1_v, sl_busy;
  reg [SA-1:0] f1_addr;
  reg [511:0] f0_new, f1_new, sl_old, sl_new, o_beat;
  reg [63:0] f0_mask, f1_mask;
  reg [7:0] sl_k;
  wire [32*@FL@-1:0] fy;
  genvar fk;
  generate
    for (fk = 0; fk < @FL@; fk = fk + 1) begin : fold_lane
      logos_fold_lane_p lane(.clk(clk), .fc(fold_r), .old(sl_old[32*fk +: 32]), .sum(sl_new[32*fk +: 32]), .y(fy[32*fk +: 32]));
    end
  endgenerate
  reg [@FOLD@-1:0] fo_v, fo_last;
  wire fo_out = fo_v[@FOLD@-1], fo_end = fo_last[@FOLD@-1];
  wire [511:0] o_next = @ONEXT@;
  `LOGOS_RAMSTYLE_ASYNC(@QD@) reg [SA+63:0] fq [0:@QD@-1];
  reg [@QA@:0] fq_w, fq_r;
  wire [SA+63:0] fq_head = fq[fq_r[@QA@-1:0]];
  // A beat's sums, lanes and address from its asking to the clock before its old beat is out.
  wire fl_v, fl_busy;
  wire [511:0] fl_new;
  wire [63:0] fl_mask;
  wire [SA-1:0] fl_addr;
  logos_landing #(.W(576 + SA), .D(SRL - 1)) fland(.clk(clk), .rst(rst), .in_v(f0_v), .in_d({f0_new, f0_mask, f0_addr}),
    .out_v(fl_v), .out_d({fl_new, fl_mask, fl_addr}), .flight(fl_busy));
  wire f_busy = f0_v || fl_busy || f1_v || sl_busy || fo_v != {@FOLD@{1'b0}} || fq_w != fq_r;
  always @(posedge clk) begin
    if (f_take) begin f0_new <= lanes; f0_mask <= mask; f0_addr <= to_base + out_at[SA-1:0]; end
    f1_new <= fl_new; f1_mask <= fl_mask; f1_addr <= fl_addr;
    if (f1_v) begin
      sl_old <= st_rd; sl_new <= f1_new; sl_k <= 8'd0;
      fq[fq_w[@QA@-1:0]] <= {f1_mask, f1_addr};
    end else if (sl_busy) begin
      sl_old <= sl_old >> 32*@FL@; sl_new <= sl_new >> 32*@FL@; sl_k <= sl_k + 8'd1;
    end
    if (fo_out) o_beat <= o_next;
    if (rst) begin
      f_gap <= 8'd0; f0_v <= 1'b0; f1_v <= 1'b0; sl_busy <= 1'b0;
      fo_v <= {@FOLD@{1'b0}}; fo_last <= {@FOLD@{1'b0}}; fq_w <= {(@QA@+1){1'b0}}; fq_r <= {(@QA@+1){1'b0}};
    end else begin
      f_gap <= f_take ? 8'd@SLAST@ : (f_gap != 8'd0 ? f_gap - 8'd1 : 8'd0);
      f0_v <= f_take;
      f1_v <= fl_v;
      sl_busy <= f1_v || (sl_busy && sl_k != 8'd@SLAST@);
      fo_v <= {fo_v[@FOLD@-2:0], sl_busy};
      fo_last <= {fo_last[@FOLD@-2:0], sl_busy && sl_k == 8'd@SLAST@};
      if (f1_v) fq_w <= fq_w + 1'b1;
      if (fo_end) fq_r <= fq_r + 1'b1;
    end
  end
"#;

const GREEDY: &str = r#"
// A greedy head's reduction of a matvec job's sums: beats of sixteen lanes, each beat token-row in_t's rows from
// 16 * in_at, a lane one of them when its present bit is set, every token-row's beats in row order. A beat passes
// through a tree of four registered levels to its lanes' first largest number — a node takes its higher child only when
// that is present and the lower is not, or is a number strictly larger than the lower or the lower is no number, so
// the lowest lane of the largest wins and a NaN is passed over — which then updates its token-row's best: the
// token-row's first candidate, then a candidate that is a number and strictly larger, or a number where the best is
// none. So a token-row's best is numerics::argmax of its rows and that row's value, its first row when none is a
// number. rd_row and rd_val are token-row rd_t's; busy while a beat is in the tree; clear forgets every token-row's.
module logos_greedy #(parameter T_MAX = 4)
  (input clk, input rst, input clear, input in_valid, input [7:0] in_t, input [31:0] in_at, input [511:0] in_lanes,
   input [15:0] in_present, output busy, input [7:0] rd_t, output [31:0] rd_row, output [31:0] rd_val);
  function [31:0] fkey(input [31:0] x);
    fkey = (x[30:0] == 31'd0) ? 32'h8000_0000 : (x[31] ? ~x : {1'b1, x[30:0]});
  endfunction
  function is_nan(input [31:0] x);
    is_nan = x[30:23] == 8'hff && x[22:0] != 23'd0;
  endfunction
  function higher(input lp, input ln, input [31:0] lv, input hp, input hn, input [31:0] hv);
    higher = hp && (!lp || (hn && (!ln || fkey(hv) > fkey(lv))));
  endfunction
  // The tree: per level every node's presence, whether it is a number, its value and its lane; beside it the beat's
  // token-row and place.
  reg [15:0] gp [0:4];
  reg [15:0] gn [0:4];
  reg [31:0] gv [0:4][0:15];
  reg [3:0] gl [0:4][0:15];
  reg g_live [0:4];
  reg [7:0] g_t [0:4];
  reg [31:0] g_at [0:4];
  integer gk, gi;
  always @(posedge clk) begin
    g_live[0] <= in_valid;
    g_t[0] <= in_t;
    g_at[0] <= in_at;
    for (gi = 0; gi < 16; gi = gi + 1) begin
      gp[0][gi] <= in_present[gi];
      gn[0][gi] <= in_present[gi] && !is_nan(in_lanes[32*gi +: 32]);
      gv[0][gi] <= in_lanes[32*gi +: 32];
      gl[0][gi] <= gi[3:0];
    end
    for (gk = 0; gk < 4; gk = gk + 1) begin
      g_live[gk+1] <= g_live[gk]; g_t[gk+1] <= g_t[gk]; g_at[gk+1] <= g_at[gk];
      for (gi = 0; gi < (8 >> gk); gi = gi + 1)
        if (higher(gp[gk][2*gi], gn[gk][2*gi], gv[gk][2*gi], gp[gk][2*gi+1], gn[gk][2*gi+1], gv[gk][2*gi+1])) begin
          gp[gk+1][gi] <= 1'b1; gn[gk+1][gi] <= gn[gk][2*gi+1]; gv[gk+1][gi] <= gv[gk][2*gi+1]; gl[gk+1][gi] <= gl[gk][2*gi+1];
        end else begin
          gp[gk+1][gi] <= gp[gk][2*gi]; gn[gk+1][gi] <= gn[gk][2*gi]; gv[gk+1][gi] <= gv[gk][2*gi]; gl[gk+1][gi] <= gl[gk][2*gi];
        end
    end
    if (rst) for (gk = 0; gk < 5; gk = gk + 1) g_live[gk] <= 1'b0;
  end
  assign busy = g_live[0] || g_live[1] || g_live[2] || g_live[3] || g_live[4];
  // Each token-row's best: whether it has one, whether that is a number, its value and its row.
  (* ramstyle = "logic" *) reg b_seen [0:T_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(T_MAX) reg b_num [0:T_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(T_MAX) reg [31:0] b_val [0:T_MAX-1];
  `LOGOS_RAMSTYLE_ASYNC(T_MAX) reg [31:0] b_row [0:T_MAX-1];
  wire [7:0] c_t = g_t[4];
  wire c_take = g_live[4] && gp[4][0] && (!b_seen[c_t] || higher(1'b1, b_num[c_t], b_val[c_t], 1'b1, gn[4][0], gv[4][0]));
  integer bk;
  always @(posedge clk) begin
    if (c_take) begin
      b_seen[c_t] <= 1'b1; b_num[c_t] <= gn[4][0]; b_val[c_t] <= gv[4][0]; b_row[c_t] <= g_at[4] * 32'd16 + {28'd0, gl[4][0]};
    end
    if (rst || clear) for (bk = 0; bk < T_MAX; bk = bk + 1) b_seen[bk] <= 1'b0;
  end
  assign rd_row = b_row[rd_t];
  assign rd_val = b_val[rd_t];
endmodule
"#;

const ACT_STREAM_P: &str = r#"
// logos_act_code pipelined, a value a clock: `code` is the code of the operands of @LAT@ clocks before. The first stage
// takes the value's significand and the shift that places it against the scale; the second places it (the dividend of
// logos_act_code, 32 places finer) and decides whether the quotient's whole part reaches 2^16 (the code then the word's
// limit); the next seventeen take its sixteen whole bits and its half a bit a stage by restoring division by m; the last
// rounds to nearest even, the dividend's bits below the half and the remainder its sticky bit, and clamps to the word.
module logos_act_code_p(input clk, input [31:0] x, input [7:0] m, input signed [11:0] e, input two,
                        output reg signed [15:0] code);
  wire [7:0]  ex = x[30:23];
  reg         neg, two_a;
  reg [7:0]   m_a;
  reg [23:0]  sx;
  reg signed [12:0] up32;
  always @(posedge clk) begin
    neg <= x[31]; two_a <= two; m_a <= m;
    sx <= (ex == 8'd0) ? 24'd0 : {1'b1, x[22:0]};
    up32 <= $signed({5'd0, ex}) - 13'sd150 - e + 13'sd32;
  end
  wire [63:0] num = (up32 < 13'sd0) ? 64'd0 : (up32 > 13'sd39) ? 64'hffff_ffff_ffff_ffff : ({40'd0, sx} << up32[5:0]);
  reg [7:0]  r [0:17];
  reg [16:0] q [0:17];
  reg [16:0] rest [0:17];
  reg [7:0]  mm [0:17];
  reg        over [0:17];
  reg        sign [0:17];
  reg        wide [0:17];
  reg        low [0:17];
  always @(posedge clk) begin
    r[0] <= num[55:48]; q[0] <= 17'd0; rest[0] <= num[47:31]; mm[0] <= m_a;
    over[0] <= num[63:48] >= {8'd0, m_a}; sign[0] <= neg; wide[0] <= two_a; low[0] <= |num[30:0];
  end
  genvar i;
  generate
    for (i = 0; i < 17; i = i + 1) begin : step
      wire [8:0] r2 = {r[i], rest[i][16]};
      wire       ge = r2 >= {1'b0, mm[i]};
      wire [8:0] d = r2 - {1'b0, mm[i]};
      always @(posedge clk) begin
        r[i+1] <= ge ? d[7:0] : r2[7:0];
        q[i+1] <= {q[i][15:0], ge};
        rest[i+1] <= {rest[i][15:0], 1'b0};
        mm[i+1] <= mm[i]; over[i+1] <= over[i]; sign[i+1] <= sign[i]; wide[i+1] <= wide[i]; low[i+1] <= low[i];
      end
    end
  endgenerate
  wire [15:0] whole = q[17][16:1];
  wire        up = q[17][0] && (r[17] != 8'd0 || low[17] || whole[0]);
  wire [16:0] n = {1'b0, whole} + {16'd0, up};
  wire [16:0] lim = wide[17] ? 17'd32767 : 17'd127;
  wire [16:0] c = (over[17] || n > lim) ? lim : n;
  always @(posedge clk) code <= sign[17] ? -$signed({1'b0, c[14:0]}) : $signed({1'b0, c[14:0]});
endmodule

// logos_act_stream with pipelined code units: the same ports and protocol, the block's beats in registered-read block
// RAM, the block's largest magnitude a clock behind its beats and its scale (logos_wide_scale_p) @WS@ clocks behind that,
// latched with the word count `go` brings LAG clocks after `go`; then a beat read a clock and its codes written @LAT@
// clocks after it arrives, `busy` from `go` until the last's are; past the block's length its codes are zero.
module logos_act_stream_p #(parameter LANES = 256)
  (input clk, input rst, input clear, input in_v, input [511:0] in_beat, input go, input two,
   output busy, output reg [7:0] m, output reg signed [11:0] e, output [16*LANES-1:0] codes);
  localparam BEATS = LANES / 16;
  localparam BA = (BEATS > 1) ? $clog2(BEATS) : 1;
  localparam L = @LAT@ + 1;
  // The beat's largest magnitude, a denormal read as zero, by a balanced tree: its first two levels here, its last two
  // a clock on.
  wire [30:0] mag [0:15];
  wire [30:0] t1 [0:7];
  wire [30:0] t2 [0:3];
  genvar g;
  generate
    for (g = 0; g < 16; g = g + 1) begin : magnitude
      assign mag[g] = (in_beat[32*g+30 -: 8] == 8'd0) ? 31'd0 : in_beat[32*g+30 -: 31];
    end
    for (g = 0; g < 8; g = g + 1) begin : max8
      assign t1[g] = (mag[2*g] > mag[2*g+1]) ? mag[2*g] : mag[2*g+1];
    end
    for (g = 0; g < 4; g = g + 1) begin : max4
      assign t2[g] = (t1[2*g] > t1[2*g+1]) ? t1[2*g] : t1[2*g+1];
    end
  endgenerate
  // The tree's last two levels and the block's maximum a clock after the beat, the scale WS clocks after the maximum
  // and the word count latched at `go`: the scale latched LAG clocks after `go` is its block's for a `go` any clock
  // after the last beat.
  localparam WS = @WS@;
  localparam [3:0] LAG = 1 + WS;
  reg [30:0] h0, h1, h2, h3;
  reg hv, hclear;
  always @(posedge clk) begin h0 <= t2[0]; h1 <= t2[1]; h2 <= t2[2]; h3 <= t2[3]; hv <= in_v && !rst; hclear <= clear; end
  wire [30:0] u0 = (h0 > h1) ? h0 : h1, u1 = (h2 > h3) ? h2 : h3;
  wire [30:0] beat_max = (u0 > u1) ? u0 : u1;
  reg [30:0] amax;
  reg [BA:0] n, c;
  reg reading, two_r, scaling;
  reg [3:0] lag;
  wire [7:0] m_s;
  wire signed [11:0] e_s;
  logos_wide_scale_p sc(.clk(clk), .amax(amax), .two(two_r), .m(m_s), .e(e_s));
  wire [511:0] xb;
  logos_bram_w512 #(.D(BEATS), .A(BA)) beats(.clk(clk), .ra(c[BA-1:0]), .rd(xb), .we(in_v),
    .wa(clear ? {BA{1'b0}} : n[BA-1:0]), .wd(in_beat), .wm({64{1'b1}}));
  wire [255:0] cb;
  generate
    for (g = 0; g < 16; g = g + 1) begin : lane
      wire signed [15:0] cg;
      logos_act_code_p u(.clk(clk), .x(xb[32*g +: 32]), .m(m), .e(e), .two(two_r), .code(cg));
      assign cb[16*g +: 16] = cg;
    end
  endgenerate
  // Each beat read, beside its codes on their way: whether it is one and its place.
  reg live [0:L-1];
  reg [BA:0] at [0:L-1];
  reg flight;
  integer h;
  always @* begin
    flight = 1'b0;
    for (h = 0; h < L; h = h + 1) if (live[h]) flight = 1'b1;
  end
  assign busy = scaling || reading || flight;
  reg [256*BEATS-1:0] cr;
  assign codes = cr[16*LANES-1:0];
  always @(posedge clk) begin
    live[0] <= reading; at[0] <= c;
    for (h = 1; h < L; h = h + 1) begin live[h] <= live[h-1]; at[h] <= at[h-1]; end
    // A block's first codes clear the beats past them: past its length a block's codes are zero.
    if (live[L-1]) begin
      if (at[L-1] == {(BA+1){1'b0}}) cr <= {{(256*BEATS-256){1'b0}}, cb};
      else cr[256*at[L-1] +: 256] <= cb;
    end
    if (rst) begin
      scaling <= 1'b0; reading <= 1'b0; n <= {(BA+1){1'b0}};
      for (h = 0; h < L; h = h + 1) live[h] <= 1'b0;
    end else begin
      if (in_v) n <= (clear ? {(BA+1){1'b0}} : n) + 1'b1;
      if (hv) amax <= (hclear || beat_max > amax) ? beat_max : amax;
      if (go) begin
        scaling <= 1'b1; lag <= LAG; two_r <= two;
      end else if (scaling) begin
        if (lag == 4'd1) begin
          scaling <= 1'b0; reading <= n != {(BA+1){1'b0}}; c <= {(BA+1){1'b0}}; m <= m_s; e <= e_s;
        end else lag <= lag - 4'd1;
      end else if (reading) begin
        if (c + 1'b1 == n) reading <= 1'b0; else c <= c + 1'b1;
      end
    end
  end
endmodule

// logos_act_stream_p's blocks back to back: a block's beats arrive a beat a clock (`in_v`), any clocks apart, its first
// marked `in_first`, its last `in_last` with its word count `in_two` and its tag `in_tag`, and the next block's first
// may come the clock after its last. A block's largest magnitude is final two clocks after its last beat, its scale @WS@
// clocks later, and the block then waits in a queue; the queue's blocks are read out in order, a beat a clock from a
// ring of block RAM, through the code units with their block's scale and word count beside each beat. The clock after
// a block's last codes land in `codes`, `out_v` holds with its scale and tag; the next block's first codes land at that
// clock's end at the soonest, so `codes` are the block's while `out_v` holds, zero past its length. `busy` holds from a
// block's first beat until the clock after its `out_v`.
module logos_act_blocks_p #(parameter LANES = 256, TW = 24)
  (input clk, input rst, input in_v, input [511:0] in_beat, input in_first, input in_last, input in_two,
   input [TW-1:0] in_tag, output busy, output reg out_v, output reg [7:0] out_m, output reg signed [11:0] out_e,
   output reg [TW-1:0] out_tag, output [16*LANES-1:0] codes);
  localparam BEATS = LANES / 16;
  localparam BA = (BEATS > 1) ? $clog2(BEATS) : 1;
  localparam L = @LAT@ + 1;
  localparam WS = @WS@;
  // The ring holds each beat from its arrival to its read: at most a block's beats and those arriving in the clocks
  // from its last to its first read, and one more.
  localparam RA = $clog2(BEATS + WS + 6);
  // The queue holds each block from its scale to its codes: at most a block a beat the ring holds and a block a beat
  // in the code units.
  localparam QA = $clog2((1 << RA) + L + 4);
  // The beat's largest magnitude, a denormal read as zero, by a balanced tree: its first two levels here, its last two
  // a clock on with the beat's marks.
  wire [30:0] mag [0:15];
  wire [30:0] t1 [0:7];
  wire [30:0] t2 [0:3];
  genvar g;
  generate
    for (g = 0; g < 16; g = g + 1) begin : magnitude
      assign mag[g] = (in_beat[32*g+30 -: 8] == 8'd0) ? 31'd0 : in_beat[32*g+30 -: 31];
    end
    for (g = 0; g < 8; g = g + 1) begin : max8
      assign t1[g] = (mag[2*g] > mag[2*g+1]) ? mag[2*g] : mag[2*g+1];
    end
    for (g = 0; g < 4; g = g + 1) begin : max4
      assign t2[g] = (t1[2*g] > t1[2*g+1]) ? t1[2*g] : t1[2*g+1];
    end
  endgenerate
  reg [30:0] h0, h1, h2, h3;
  reg hv, hfirst, hlast, htwo;
  reg [BA:0] hn, bn;
  reg [TW-1:0] htag;
  always @(posedge clk) begin
    h0 <= t2[0]; h1 <= t2[1]; h2 <= t2[2]; h3 <= t2[3];
    hv <= in_v && !rst; hfirst <= in_first; hlast <= in_last; htwo <= in_two; htag <= in_tag;
    hn <= in_first ? {{BA{1'b0}}, 1'b1} : bn + 1'b1;
    if (in_v) bn <= in_first ? {{BA{1'b0}}, 1'b1} : bn + 1'b1;
  end
  wire [30:0] u0 = (h0 > h1) ? h0 : h1, u1 = (h2 > h3) ? h2 : h3;
  wire [30:0] beat_max = (u0 > u1) ? u0 : u1;
  // The block's maximum the clock after its last beat's marks, `fin` beside it; its scale WS clocks on, its word count,
  // beats and tag carried beside.
  reg [30:0] amax;
  reg fin, ftwo;
  reg [BA:0] fn;
  reg [TW-1:0] ftag;
  always @(posedge clk) begin
    if (hv) amax <= (hfirst || beat_max > amax) ? beat_max : amax;
    fin <= hv && hlast && !rst; ftwo <= htwo; fn <= hn; ftag <= htag;
  end
  wire [7:0] m_s;
  wire signed [11:0] e_s;
  logos_wide_scale_p sc(.clk(clk), .amax(amax), .two(ftwo), .m(m_s), .e(e_s));
  reg sv [1:WS];
  reg sw [1:WS];
  reg [BA:0] sn [1:WS];
  reg [TW-1:0] st [1:WS];
  integer k;
  always @(posedge clk) begin
    sv[1] <= fin && !rst; sw[1] <= ftwo; sn[1] <= fn; st[1] <= ftag;
    for (k = 2; k <= WS; k = k + 1) begin sv[k] <= sv[k-1] && !rst; sw[k] <= sw[k-1]; sn[k] <= sn[k-1]; st[k] <= st[k-1]; end
  end
  // The queue of scaled blocks: written as each scale is out, read at the reader's start of the block and again for
  // its codes.
  reg [7:0] qm [0:(1<<QA)-1];
  reg signed [11:0] qe [0:(1<<QA)-1];
  reg qtwo [0:(1<<QA)-1];
  reg [BA:0] qn [0:(1<<QA)-1];
  reg [TW-1:0] qtag [0:(1<<QA)-1];
  reg [QA-1:0] q_wr, q_rd, q_out;
  always @(posedge clk)
    if (sv[WS]) begin qm[q_wr] <= m_s; qe[q_wr] <= e_s; qtwo[q_wr] <= sw[WS]; qn[q_wr] <= sn[WS]; qtag[q_wr] <= st[WS]; end
  // The ring: each beat written as it arrives; read a beat a clock, block after block, as the queue has them.
  reg [RA-1:0] wp, rp;
  wire [511:0] xb;
  logos_bram_w512 #(.D(1 << RA), .A(RA)) ring(.clk(clk), .ra(rp), .rd(xb), .we(in_v), .wa(wp), .wd(in_beat),
    .wm({64{1'b1}}));
  reg reading;
  reg [BA:0] rleft, rj;
  reg [7:0] rm;
  reg signed [11:0] re;
  reg rtwo;
  wire queued = q_rd != q_wr;
  wire take = queued && (!reading || rleft == {{BA{1'b0}}, 1'b1});
  // The code units take each beat the clock after its read, its block's scale and word count beside it.
  reg [7:0] cm;
  reg signed [11:0] ce;
  reg ctwo;
  wire [255:0] cb;
  generate
    for (g = 0; g < 16; g = g + 1) begin : lane
      wire signed [15:0] cg;
      logos_act_code_p u(.clk(clk), .x(xb[32*g +: 32]), .m(cm), .e(ce), .two(ctwo), .code(cg));
      assign cb[16*g +: 16] = cg;
    end
  endgenerate
  // Each beat read, beside its codes on their way: whether it is one, its place and whether its block's last.
  reg live [0:L-1];
  reg lst [0:L-1];
  reg [BA:0] at [0:L-1];
  reg [256*BEATS-1:0] cr;
  assign codes = cr[16*LANES-1:0];
  reg filling;
  reg [QA:0] held_blocks;
  assign busy = filling || held_blocks != {(QA+1){1'b0}};
  integer h;
  always @(posedge clk) begin
    cm <= rm; ce <= re; ctwo <= rtwo;
    live[0] <= reading && !rst; lst[0] <= rleft == {{BA{1'b0}}, 1'b1}; at[0] <= rj;
    for (h = 1; h < L; h = h + 1) begin live[h] <= live[h-1] && !rst; lst[h] <= lst[h-1]; at[h] <= at[h-1]; end
    // A block's first codes clear the beats past them: past its length a block's codes are zero.
    if (live[L-1]) begin
      if (at[L-1] == {(BA+1){1'b0}}) cr <= {{(256*BEATS-256){1'b0}}, cb};
      else cr[256*at[L-1] +: 256] <= cb;
    end
    out_v <= live[L-1] && lst[L-1] && !rst;
    if (live[L-1] && lst[L-1]) begin out_m <= qm[q_out]; out_e <= qe[q_out]; out_tag <= qtag[q_out]; end
    if (rst) begin
      wp <= {RA{1'b0}}; rp <= {RA{1'b0}}; q_wr <= {QA{1'b0}}; q_rd <= {QA{1'b0}}; q_out <= {QA{1'b0}};
      reading <= 1'b0; filling <= 1'b0; held_blocks <= {(QA+1){1'b0}};
    end else begin
      if (in_v) begin wp <= wp + 1'b1; filling <= !in_last; end
      held_blocks <= held_blocks + ((in_v && in_last) ? 1'b1 : 1'b0) - (out_v ? 1'b1 : 1'b0);
      if (sv[WS]) q_wr <= q_wr + 1'b1;
      if (live[L-1] && lst[L-1]) q_out <= q_out + 1'b1;
      if (reading) begin rp <= rp + 1'b1; rleft <= rleft - 1'b1; rj <= rj + 1'b1; end
      if (take) begin
        reading <= 1'b1; rleft <= qn[q_rd]; rj <= {(BA+1){1'b0}}; rm <= qm[q_rd]; re <= qe[q_rd]; rtwo <= qtwo[q_rd];
        q_rd <= q_rd + 1'b1;
      end else if (reading && rleft == {{BA{1'b0}}, 1'b1}) reading <= 1'b0;
    end
  end
endmodule
"#;
