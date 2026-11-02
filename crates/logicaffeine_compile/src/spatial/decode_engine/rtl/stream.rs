//! The image stream engine: one memory channel's tiles of the weight image in, the tiles' rows out as 64-byte beats.
//!
//! A tile descriptor names a tile of [`super::super::image::Image`]: its first line on the channel, its lines, the bytes
//! of rows it holds, its tensor and its first row. The engine sits in the compute clock on the shell's channel
//! interface: it asks the shell for the tile's lines with one read descriptor, takes the lines as they cross back, and
//! sends the tile's row bytes on as beats of 64 bytes, each tagged with the tile's tensor, its first row and the beat's
//! byte offset in the tile, the last beat carrying how many of its bytes are rows (the rest zero). At 72-byte lines the
//! bytes are regathered: eight lines make nine beats, so the engine's output runs at the line rate when its clock is
//! 9/8 of the channel's. With check words every line passes through a CRC-32C as it is taken, and the tile's last beat
//! says whether the word in its last line's last four bytes was the CRC of the lines before it.
//!
//! The beats wait in a 128-byte gather register: a beat leaves once 64 bytes are gathered, a line comes in when its
//! bytes fit or the register already holds every row byte the tile has left (then the line only feeds the CRC), and
//! the tile's last beat waits for the tile's last line, so the verdict is known when it leaves. Up to four tiles'
//! reads are outstanding at once, so a tile's lines follow the last tile's without a gap.
//!
//! [`beats`] is the unit's software twin: from a channel's bytes and a descriptor, the beats the RTL sends.

use std::fmt::Write;

use super::super::image::{crc32c, Image, ImageLayout, CHECK_BYTES};

/// Bytes a beat carries.
pub const BEAT_BYTES: u64 = 64;
/// Bits of the descriptor fields and tags: the line address, the line count (the shell's descriptor widths), the byte
/// count, the tensor and the row.
pub const AW: u32 = 27;
pub const LW: u32 = 32;
pub const BYW: u32 = 24;
pub const TW: u32 = 16;
pub const ROWW: u32 = 32;
const GATHER_BYTES: u64 = 128;

/// One tile as the engine takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileDescriptor {
    pub line: u64,
    pub lines: u64,
    pub bytes: u64,
    pub tensor: u32,
    pub first_row: u64,
}

/// One beat as the engine sends it: `keep` of its bytes are the tile's rows (the rest zero), `offset` is its first
/// byte's place in the tile, `crc_error` is set on a tile's last beat when its check word was wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Beat {
    pub data: [u8; BEAT_BYTES as usize],
    pub keep: u64,
    pub last: bool,
    pub tensor: u32,
    pub row: u64,
    pub offset: u64,
    pub crc_error: bool,
}

/// The descriptors of every tile `image` holds on `channel`, in the channel's order.
pub fn descriptors(image: &Image, channel: u32) -> Vec<TileDescriptor> {
    image
        .tiles
        .iter()
        .filter(|t| t.channel == channel)
        .map(|t| TileDescriptor {
            line: t.line,
            lines: t.lines,
            bytes: (t.end_row - t.first_row) * image.tensors[t.tensor].image_row_bytes(),
            tensor: t.tensor as u32,
            first_row: t.first_row,
        })
        .collect()
}

/// The beats the engine sends for tile `d` read from `channel` (the channel's bytes) under `layout`.
pub fn beats(channel: &[u8], d: &TileDescriptor, layout: &ImageLayout) -> Vec<Beat> {
    let lb = layout.line_bytes as usize;
    let span = &channel[d.line as usize * lb..(d.line + d.lines) as usize * lb];
    let crc_error = layout.check && {
        let body = span.len() - CHECK_BYTES as usize;
        crc32c(&span[..body]).to_le_bytes() != span[body..]
    };
    let rows = &span[..d.bytes as usize];
    let count = rows.len().div_ceil(BEAT_BYTES as usize);
    rows.chunks(BEAT_BYTES as usize)
        .enumerate()
        .map(|(k, chunk)| {
            let mut data = [0u8; BEAT_BYTES as usize];
            data[..chunk.len()].copy_from_slice(chunk);
            let last = k + 1 == count;
            Beat {
                data,
                keep: chunk.len() as u64,
                last,
                tensor: d.tensor,
                row: d.first_row,
                offset: k as u64 * BEAT_BYTES,
                crc_error: last && crc_error,
            }
        })
        .collect()
}

/// The CRC-32C register after `bytes` from `state`, with no initial or final inversion: the map the hardware applies
/// to its register a line at a time, linear in the state and the bytes.
pub fn crc32c_update(mut state: u32, bytes: &[u8]) -> u32 {
    for &b in bytes {
        state ^= b as u32;
        for _ in 0..8 {
            state = (state >> 1) ^ (0x82F6_3B78 & (state & 1).wrapping_neg());
        }
    }
    state
}

/// For each bit of the register after `len` bytes, the register bits and the data bits (byte `k`'s bit `j` at bit
/// `8k + j`) whose exclusive-or it is.
pub fn crc_masks(len: usize) -> Vec<(u32, Vec<bool>)> {
    let zeros = vec![0u8; len];
    let from_state: Vec<u32> = (0..32).map(|j| crc32c_update(1 << j, &zeros)).collect();
    let from_data: Vec<u32> = (0..8 * len)
        .map(|k| {
            let mut d = zeros.clone();
            d[k / 8] = 1 << (k % 8);
            crc32c_update(0, &d)
        })
        .collect();
    (0..32)
        .map(|i| {
            let state = (0..32).filter(|&j| from_state[j] >> i & 1 == 1).fold(0u32, |m, j| m | 1 << j);
            (state, from_data.iter().map(|&c| c >> i & 1 == 1).collect())
        })
        .collect()
}

fn hex_mask(bits: &[bool]) -> String {
    let digits = bits.len().div_ceil(4);
    (0..digits)
        .rev()
        .map(|d| {
            let nibble = (0..4).fold(0u8, |v, b| v | (u8::from(bits.get(4 * d + b).copied().unwrap_or(false)) << b));
            char::from_digit(nibble as u32, 16).unwrap()
        })
        .collect()
}

/// The engine for lines of `layout.line_bytes` bytes, checking every tile's CRC-32C when `layout.check`.
pub fn verilog(layout: &ImageLayout) -> String {
    let lb = layout.line_bytes;
    assert!(lb % 8 == 0 && lb >= BEAT_BYTES && lb + 56 <= GATHER_BYTES, "a line of {lb} bytes");
    let dw = 8 * lb;
    let mut v = String::new();
    let _ = writeln!(
        v,
        "// The image stream engine for {lb}-byte lines{}: tile descriptors in, the tile's lines asked of the channel as one\n\
         // read descriptor, its rows out as 64-byte beats tagged with tensor, first row and byte offset.",
        if layout.check { " with a CRC-32C check word ending every tile" } else { "" }
    );
    let _ = writeln!(
        v,
        "module logos_image_stream (
  input              clk,
  input              rst_n,
  input              tile_valid,
  output             tile_ready,
  input  [{a}:0]      tile_line,
  input  [{l}:0]      tile_lines,
  input  [{b}:0]      tile_bytes,
  input  [{t}:0]      tile_tensor,
  input  [{r}:0]      tile_row,
  output             md_valid,
  input              md_ready,
  output [{m}:0]      md_data,
  input              mr_valid,
  output             mr_ready,
  input  [{d}:0]     mr_data,
  output             out_valid,
  input              out_ready,
  output [511:0]     out_data,
  output [6:0]       out_keep,
  output             out_last,
  output [{t}:0]      out_tensor,
  output [{r}:0]      out_row,
  output [{b}:0]      out_offset,
  output             out_crc_error
);",
        a = AW - 1,
        l = LW - 1,
        b = BYW - 1,
        t = TW - 1,
        r = ROWW - 1,
        m = AW + LW,
        d = dw - 1
    );
    let _ = writeln!(
        v,
        "  // The tiles whose lines are asked for, oldest first, in registers.
  (* ramstyle = \"logic\" *) reg [{l}:0] q_lines [0:3];
  (* ramstyle = \"logic\" *) reg [{b}:0] q_bytes [0:3];
  (* ramstyle = \"logic\" *) reg [{t}:0] q_tensor [0:3];
  (* ramstyle = \"logic\" *) reg [{r}:0] q_row [0:3];
  reg [2:0]  qh, qt;
  wire       q_full = (qt - qh) == 3'd4;
  wire       q_some = qt != qh;
  assign md_valid = tile_valid && !q_full;
  assign tile_ready = md_ready && !q_full;
  assign md_data = {{1'b0, tile_line, tile_lines}};
  wire       tile_take = tile_valid && tile_ready;

  // The head tile: its lines still to take and row bytes still to send, each with its flag (none left, the last beat)
  // in a register beside it; row bytes sent; the gather register and its bytes; the CRC register; the verdict. The head's
  // counts are loaded from its queue entry, or from the descriptor taken the clock it becomes the head.
  reg [{l}:0] lines_left;
  reg [{b}:0] pay_left;
  reg         no_lines, final_beat;
  reg [{b}:0] sent;
  reg [7:0]   n;
  reg [1023:0] acc;
  reg [31:0]  crc;
  reg         bad;
  wire        can_emit = q_some && (final_beat ? no_lines : n[7] | n[6]);
  wire        emit = can_emit && out_ready;
  wire        shift = emit && !final_beat;
  wire        retire = emit && final_beat;
  wire [7:0]  n1 = shift ? n - 8'd64 : n;
  // Shifting takes 64 from both the gathered bytes and the bytes left, so whether the register holds the rest is told
  // before the shift; room for a line is told on the bytes after it.
  wire        holds_rest = {{{pad}'d0, n}} >= pay_left;
  wire        room = (shift ? n <= 8'd{room_shift} : n <= 8'd{room_hold}) || holds_rest;
  wire        take = q_some && !no_lines && mr_valid && room;
  wire        place = take && !holds_rest;
  wire        last_line = lines_left == {lw}'d1;
  // The next head's counts: the entry after the head's when it is queued, else the descriptor taken now.
  wire [2:0]  qn = qh + 3'd1;
  wire        next_queued = qt != qn;
  wire [{l}:0] next_lines = next_queued ? q_lines[qn[1:0]] : tile_lines;
  wire [{b}:0] next_bytes = next_queued ? q_bytes[qn[1:0]] : tile_bytes;
  wire        load_next = retire && (next_queued || tile_take);
  wire        load_new = !q_some && tile_take;
  assign mr_ready = take;
  wire [1023:0] shifted = shift ? {{512'd0, acc[1023:512]}} : acc;
  wire [1023:0] line_at = {{{rest}'d0, mr_data}} << {{n1[6:3], 6'd0}};",
        l = LW - 1,
        b = BYW - 1,
        t = TW - 1,
        r = ROWW - 1,
        lw = LW,
        pad = BYW - 8,
        room_shift = 192 - lb,
        room_hold = 128 - lb,
        rest = 1024 - dw
    );
    if layout.check {
        let full = crc_masks(lb as usize);
        let last = crc_masks((lb - CHECK_BYTES) as usize);
        let body = 8 * (lb - CHECK_BYTES);
        let _ = writeln!(v, "  // The CRC register after a whole line, and after a last line's bytes before its check word.");
        let _ = writeln!(v, "  wire [31:0] crc_full, crc_last;");
        for (i, (state, data)) in full.iter().enumerate() {
            let _ = writeln!(v, "  assign crc_full[{i}] = ^(crc & 32'h{state:08x}) ^ ^(mr_data & {dw}'h{});", hex_mask(data));
        }
        for (i, (state, data)) in last.iter().enumerate() {
            let _ = writeln!(
                v,
                "  assign crc_last[{i}] = ^(crc & 32'h{state:08x}) ^ ^(mr_data[{}:0] & {body}'h{});",
                body - 1,
                hex_mask(data)
            );
        }
        let _ = writeln!(v, "  wire [31:0] check_word = mr_data[{}:{}];", dw - 1, body);
    }
    let _ = writeln!(
        v,
        "  wire [511:0] keep_mask;
  genvar i;
  generate for (i = 0; i < 64; i = i + 1) begin : keep_bytes
    assign keep_mask[8*i +: 8] = {{8{{i < out_keep}}}};
  end endgenerate
  assign out_valid = can_emit;
  assign out_keep = final_beat ? pay_left[6:0] : 7'd64;
  assign out_last = final_beat;
  assign out_data = acc[511:0] & keep_mask;
  assign out_tensor = q_tensor[qh[1:0]];
  assign out_row = q_row[qh[1:0]];
  assign out_offset = sent;
  assign out_crc_error = {crc_error};

  always @(posedge clk) begin
    if (!rst_n) begin
      qh <= 3'd0;
      qt <= 3'd0;
      lines_left <= {lw}'d0;
      pay_left <= {byw}'d0;
      no_lines <= 1'b1;
      final_beat <= 1'b1;
      sent <= {byw}'d0;
      n <= 8'd0;
      acc <= 1024'd0;
      crc <= 32'hffffffff;
      bad <= 1'b0;
    end else begin
      if (tile_take) begin
        q_lines[qt[1:0]] <= tile_lines;
        q_bytes[qt[1:0]] <= tile_bytes;
        q_tensor[qt[1:0]] <= tile_tensor;
        q_row[qt[1:0]] <= tile_row;
        qt <= qt + 3'd1;
      end
      if (load_next || load_new) begin
        lines_left <= load_next ? next_lines : tile_lines;
        pay_left <= load_next ? next_bytes : tile_bytes;
        no_lines <= (load_next ? next_lines : tile_lines) == {lw}'d0;
        final_beat <= (load_next ? next_bytes : tile_bytes) <= {byw}'d64;
      end else begin
        if (take) begin lines_left <= lines_left - {lw}'d1; no_lines <= last_line; end
        if (shift) begin pay_left <= pay_left - {byw}'d64; final_beat <= pay_left <= {byw}'d128; end
      end
      if (retire) begin
        qh <= qh + 3'd1;
        sent <= {byw}'d0;
        n <= 8'd0;
        acc <= 1024'd0;
        crc <= 32'hffffffff;
        bad <= 1'b0;
      end else begin
        if (shift) sent <= sent + {byw}'d64;
        n <= n1 + (place ? 8'd{lb} : 8'd0);
        acc <= place ? (shifted | line_at) : shifted;{update}
      end
    end
  end
endmodule",
        lw = LW,
        byw = BYW,
        lb = lb,
        crc_error = if layout.check { "final_beat && bad" } else { "1'b0" },
        update = if layout.check {
            "\n        if (take) crc <= crc_full;\n        if (take && last_line) bad <= check_word != ~crc_last;"
        } else {
            ""
        }
    );
    v
}
