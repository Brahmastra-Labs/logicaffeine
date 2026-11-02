//! The D5005 image stream engine: one memory channel's tiles of the weight image in, their rows out as 64-byte beats.
//!
//! `spatial::decode_engine::rtl::stream` prints the engine and its software twin. These tests run the engine under
//! iverilog on the shell's own channel path (the descriptor and read-data dual-clock FIFOs, the Avalon-MM stream
//! engine, the randomly waiting memory model loaded with the channel's bytes), with the compute clock and the channel's
//! clock unrelated and the beats' consumer stalling at random, and hold every beat of every tile to the twin's: its
//! bytes, how many are rows, its tags, its place in the tile and the check-word verdict on the tile's last beat. A bit
//! flipped in a tile's rows, its padding or its check word is caught on that tile and no other. The twin is held to
//! the image: a tile's beats are its rows, where `Image::locate` puts them. The CRC equations are held to CRC-32C, and
//! the engine to the channel's line rate across tile boundaries.

#![cfg(feature = "codegen")]

mod common;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::reference::{Tensors, WeightSource};
use logicaffeine_compile::spatial::d5005::Beat as LineBeat;
use logicaffeine_compile::spatial::decode_engine::image::{crc32c, weight_image, Image, ImageLayout, TensorTable, CHECK_BYTES};
use logicaffeine_compile::spatial::decode_engine::ofs::{afu_sources, shell_verilog, AfuConfig};
use logicaffeine_compile::spatial::decode_engine::rtl::stream::{
    beats, crc32c_update, crc_masks, descriptors, verilog, Beat, TileDescriptor, BEAT_BYTES,
};
use tiny_models::Tiny;

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

/// The tiny Qwen3.5 checkpoint with K-quant matrices (rows of 144 and 210 bytes), a Q8_0 table and f32 vectors.
fn mixed_checkpoint(dir: &Path) -> PathBuf {
    let spec = Tiny::Qwen35.wide(64);
    let path = dir.join("qwen35-mixed.gguf");
    tiny_models::write_checkpoint_as(&spec, &path, &|name| {
        let matrix = ["output.weight", "attn_", "ssm_out", "ffn_"].iter().any(|p| name.contains(p)) && !name.contains("norm");
        if name.starts_with("token_embd") {
            GgmlType::Q8_0
        } else if name.ends_with("ffn_down.weight") && matrix {
            GgmlType::Q6K
        } else if matrix {
            GgmlType::Q4K
        } else {
            GgmlType::F32
        }
    });
    path
}

/// The tiny checkpoint at `path` laid out under `layout`, and its channels' bytes.
fn tiny_image(path: &Path, layout: &ImageLayout) -> (Image, Vec<Vec<u8>>) {
    let mut g = Gguf::open(path).unwrap();
    let blocks = Tiny::Qwen35.wide(64).layers.len() as u64;
    let table = TensorTable::from_rows(blocks, g.tensor_names()).unwrap();
    let image = weight_image(&table, &table.read_order(), layout).unwrap();
    let channels = image.fill(&mut g).unwrap();
    (image, channels)
}

/// How the testbench's channel behaves: the memory model's chance of waitrequest and of a gap between returned lines,
/// and the beats' consumer's chance of stalling, in percent.
#[derive(Clone, Copy)]
struct Weather {
    seed: u32,
    wait: u32,
    gap: u32,
    stall: u32,
}

const STORMY: Weather = Weather { seed: 11, wait: 30, gap: 20, stall: 25 };
const CALM: Weather = Weather { seed: 3, wait: 0, gap: 0, stall: 0 };

/// One run of the engine over `tiles` from `channel` (its bytes); the beats it sent, in order, and the picoseconds
/// from its first beat to its last.
fn run(layout: &ImageLayout, channel: &[u8], tiles: &[TileDescriptor], w: Weather) -> (Vec<Beat>, u64) {
    let dir = tempfile::tempdir().unwrap();
    let lb = layout.line_bytes as usize;
    let lines = channel.len() / lb;
    let lines_log2 = (usize::BITS - lines.max(2).leading_zeros()) as u32;
    let hex: String = channel.chunks(lb).map(|l| l.iter().rev().map(|b| format!("{b:02x}")).collect::<String>() + "\n").collect();
    std::fs::write(dir.path().join("channel.hex"), hex).unwrap();
    let words: String = tiles
        .iter()
        .flat_map(|t| [t.line, t.lines, t.bytes, t.tensor as u64, t.first_row])
        .map(|v| format!("{v:08x}\n"))
        .collect();
    std::fs::write(dir.path().join("tiles.hex"), words).unwrap();
    let mut files = vec![];
    let mut put = |name: &str, body: String| {
        let p = dir.path().join(name);
        std::fs::write(&p, body).unwrap();
        files.push(p);
    };
    for (name, body) in shell_verilog() {
        if name == "logos_dcfifo.v" || name == "logos_stream.v" {
            put(name, body);
        }
    }
    let model = afu_sources(&AfuConfig::d5005()).into_iter().find(|f| f.path.ends_with("logos_avmm_memory_model.v")).expect("the memory model");
    put("logos_avmm_memory_model.v", model.text);
    put("logos_image_stream.v", verilog(layout));
    put("tb.v", testbench(lb * 8, lines_log2, tiles.len(), w));
    let sim = dir.path().join("sim");
    let build = Command::new("iverilog").arg("-g2012").arg("-o").arg(&sim).args(&files).output().expect("iverilog");
    assert!(build.status.success(), "iverilog:\n{}", text(&build));
    let out = Command::new("vvp").arg(&sim).current_dir(dir.path()).output().expect("vvp");
    let said = text(&out);
    assert!(out.status.success() && said.contains("DONE"), "vvp:\n{said}");
    let span = said.lines().find_map(|l| l.strip_prefix("SPAN ")).and_then(|s| s.trim().parse().ok()).expect("the span");
    let log = std::fs::read_to_string(dir.path().join("beats.txt")).unwrap();
    let got = log
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            let digits = f[0];
            let mut data = [0u8; BEAT_BYTES as usize];
            for (k, b) in data.iter_mut().enumerate() {
                let at = digits.len() - 2 * (k + 1);
                *b = u8::from_str_radix(&digits[at..at + 2], 16).unwrap();
            }
            let n = |i: usize| f[i].parse::<u64>().unwrap();
            Beat { data, keep: n(1), last: n(2) == 1, tensor: n(3) as u32, row: n(4), offset: n(5), crc_error: n(6) == 1 }
        })
        .collect();
    (got, span)
}

/// The shell's channel path in two unrelated clocks around the engine, the memory model loaded with the channel's
/// lines, the tiles fed from a file with random gaps, the beats logged as they leave.
fn testbench(dw: usize, lines_log2: u32, tiles: usize, w: Weather) -> String {
    format!(
        r#"`timescale 1ps/1ps
module tb;
  localparam AW = 27, LW = 32, DW = {dw}, RAW = 9, NT = {tiles};
  reg uclk = 1'b0, mclk = 1'b0, urst_n = 1'b0, mrst_n = 1'b0;
  always #1111 uclk = ~uclk;
  always #1667 mclk = ~mclk;
  reg [31:0] desc [0:5*NT-1];
  initial $readmemh("tiles.hex", desc);
  integer next_tile = 0, seed = {seed}, lasts = 0, f;
  reg gate = 1'b0, out_ready = 1'b0;
  wire tile_ready;
  wire tile_valid = urst_n && gate && next_tile < NT;
  wire md_valid, md_ready, mr_valid, mr_ready, out_valid, out_last, out_crc_error;
  wire [AW+LW:0] md_data;
  wire [DW-1:0] mr_data;
  wire [511:0] out_data;
  wire [6:0] out_keep;
  wire [15:0] out_tensor;
  wire [31:0] out_row;
  wire [23:0] out_offset;
  reg md_gate = 1'b0;
  wire md_ready_fifo;
  assign md_ready = md_ready_fifo && md_gate;
  logos_image_stream engine(.clk(uclk), .rst_n(urst_n), .tile_valid(tile_valid), .tile_ready(tile_ready),
    .tile_line(desc[5*next_tile][AW-1:0]), .tile_lines(desc[5*next_tile+1]), .tile_bytes(desc[5*next_tile+2][23:0]),
    .tile_tensor(desc[5*next_tile+3][15:0]), .tile_row(desc[5*next_tile+4]), .md_valid(md_valid), .md_ready(md_ready),
    .md_data(md_data), .mr_valid(mr_valid), .mr_ready(mr_ready), .mr_data(mr_data), .out_valid(out_valid),
    .out_ready(out_ready), .out_data(out_data), .out_keep(out_keep), .out_last(out_last), .out_tensor(out_tensor),
    .out_row(out_row), .out_offset(out_offset), .out_crc_error(out_crc_error));
  wire d_valid, d_ready, r_push, r_wready, w_ready, busy, wdone;
  wire [AW+LW:0] d_data;
  wire [DW-1:0] r_data;
  wire [2:0] d_wfree, d_rused;
  wire [RAW:0] r_room, r_used, owed;
  logos_dcfifo_mlab #(.W(AW+LW+1), .AW(2)) desc_fifo(.wclk(uclk), .wce(1'b1), .wrst_n(urst_n), .wvalid(md_valid && md_gate), .wready(md_ready_fifo),
    .wdata(md_data), .wfree(d_wfree), .rclk(mclk), .rce(1'b1), .rrst_n(mrst_n), .rvalid(d_valid), .rready(d_ready), .rdata(d_data),
    .rused(d_rused));
  logos_dcfifo_m20k #(.W(DW), .AW(RAW)) read_fifo(.wclk(mclk), .wce(1'b1), .wrst_n(mrst_n), .wvalid(r_push), .wready(r_wready),
    .wdata(r_data), .wfree(r_room), .rclk(uclk), .rce(1'b1), .rrst_n(urst_n), .rvalid(mr_valid), .rready(mr_ready), .rdata(mr_data),
    .rused(r_used));
  wire [AW-1:0] address;
  wire read, write, waitrequest, readdatavalid;
  wire [6:0] burstcount;
  wire [DW-1:0] writedata, readdata;
  wire [DW/8-1:0] byteenable;
  logos_stream_avmm #(.AW(AW), .DW(DW), .BW(7), .LW(LW), .MAXB(64), .RW(RAW+1)) stream(.clk(mclk), .rst_n(mrst_n),
    .d_valid(d_valid), .d_ready(d_ready), .d_data(d_data), .r_valid(r_push), .r_data(r_data), .r_room(r_room),
    .w_valid(1'b0), .w_ready(w_ready), .w_data({{DW{{1'b0}}}}), .w_flip(21'h0), .avm_address(address), .avm_read(read),
    .avm_write(write), .avm_burstcount(burstcount), .avm_writedata(writedata), .avm_byteenable(byteenable),
    .avm_waitrequest(waitrequest), .avm_readdata(readdata), .avm_readdatavalid(readdatavalid), .owed(owed), .busy(busy),
    .wdone(wdone));
  logos_avmm_memory_model #(.AW(AW), .DW(DW), .BW(7), .LINES_LOG2({lines_log2}), .SEED({seed}), .WAIT_PCT({wait}),
    .GAP_PCT({gap})) memory(.clk(mclk), .rst_n(mrst_n), .address(address), .read(read), .write(write),
    .burstcount(burstcount), .writedata(writedata), .byteenable(byteenable), .waitrequest(waitrequest),
    .readdata(readdata), .readdatavalid(readdatavalid));
  initial begin #1; $readmemh("channel.hex", memory.mem); end
  always @(posedge uclk) begin
    gate <= (($random(seed) % 100 + 100) % 100) >= 20;
    md_gate <= (($random(seed) % 100 + 100) % 100) >= {md_stall};
    out_ready <= (($random(seed) % 100 + 100) % 100) >= {stall};
    if (tile_valid && tile_ready) next_tile <= next_tile + 1;
  end
  time first = 0, last_at = 0;
  always @(posedge uclk) if (urst_n && out_valid && out_ready) begin
    $fwrite(f, "%h %0d %0d %0d %0d %0d %0d\n", out_data, out_keep, out_last, out_tensor, out_row, out_offset, out_crc_error);
    if (first == 0) first = $time;
    last_at = $time;
    if (out_last) lasts = lasts + 1;
  end
  always @(posedge uclk) if (lasts == NT) begin
    $fclose(f);
    $display("SPAN %0d", last_at - first);
    $display("DONE");
    $finish;
  end
  initial begin
    f = $fopen("beats.txt", "w");
    #20000;
    mrst_n = 1'b1;
    urst_n = 1'b1;
  end
  initial begin #(64'd4000000000); $display("TIMEOUT after %0d of %0d tiles", lasts, NT); $finish; end
endmodule
"#,
        seed = w.seed,
        md_stall = w.stall,
        wait = w.wait,
        gap = w.gap,
        stall = w.stall
    )
}

fn expected(layout: &ImageLayout, channel: &[u8], tiles: &[TileDescriptor]) -> Vec<Beat> {
    tiles.iter().flat_map(|d| beats(channel, d, layout)).collect()
}

fn same_beats(got: &[Beat], want: &[Beat], what: &str) {
    for (k, (g, w)) in got.iter().zip(want).enumerate() {
        assert_eq!(g, w, "{what}: beat {k}");
    }
    assert_eq!(got.len(), want.len(), "{what}: beats sent");
}

// ── The RTL is its twin ──────────────────────────────────────────────────────────────────────────────

/// Every beat of every tile on every channel, at 72-byte lines with check words in tiles of four lines (most tiles
/// many beats across line boundaries, the smallest one beat), under random waits, gaps and stalls.
#[test]
fn every_tile_streams_its_rows_with_check_words() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let layout = ImageLayout { tile_lines: 4, ..ImageLayout::d5005(LineBeat::Full72) };
    let (image, channels) = tiny_image(&mixed_checkpoint(dir.path()), &layout);
    for ch in 0..layout.channels {
        let tiles = descriptors(&image, ch);
        let (got, _) = run(&layout, &channels[ch as usize], &tiles, Weather { seed: 11 + ch, ..STORMY });
        let want = expected(&layout, &channels[ch as usize], &tiles);
        same_beats(&got, &want, &format!("channel {ch}"));
        assert!(want.iter().all(|b| !b.crc_error));
        assert!(want.iter().any(|b| b.keep < BEAT_BYTES) && want.iter().filter(|b| b.last).count() == tiles.len());
    }
}

/// The card's own layouts: 512-line tiles at 72-byte lines with check words, and 64-byte lines without.
#[test]
fn every_tile_streams_its_rows_at_the_cards_layouts() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = mixed_checkpoint(dir.path());
    for beat in [LineBeat::Full72, LineBeat::Data64] {
        let layout = ImageLayout::d5005(beat);
        let (image, channels) = tiny_image(&path, &layout);
        for ch in [0, 3] {
            let tiles = descriptors(&image, ch);
            let (got, _) = run(&layout, &channels[ch as usize], &tiles, STORMY);
            same_beats(&got, &expected(&layout, &channels[ch as usize], &tiles), &format!("{beat:?} channel {ch}"));
        }
    }
}

/// A bit flipped in a tile — in a row, in its padding, in its check word, at its first and last bytes — is caught on
/// that tile's last beat, every other tile passing; the flipped bytes stream out as they are.
#[test]
fn a_flipped_bit_is_caught_in_its_tile() {
    if !common::require_tool("iverilog") {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let layout = ImageLayout { tile_lines: 4, ..ImageLayout::d5005(LineBeat::Full72) };
    let (image, mut channels) = tiny_image(&mixed_checkpoint(dir.path()), &layout);
    let ch = 1;
    let tiles = descriptors(&image, ch);
    let lb = layout.line_bytes as usize;
    let mut flipped = Vec::new();
    let mut kinds = [0usize; 4];
    for (k, d) in tiles.iter().enumerate().step_by(5) {
        let start = d.line as usize * lb;
        let end = (d.line + d.lines) as usize * lb;
        let body = end - CHECK_BYTES as usize;
        let rows_end = start + d.bytes as usize;
        let (kind, at) = match (k / 5) % 4 {
            0 => (0, start + (k * 7) % d.bytes as usize),
            1 if rows_end < body => (1, if k % 2 == 0 { rows_end } else { rows_end + k % (body - rows_end) }),
            1 | 2 => (2, end - 1 - k % 4),
            _ => (3, start),
        };
        kinds[kind] += 1;
        channels[ch as usize][at] ^= 1 << (k % 8);
        flipped.push(k);
    }
    let want = expected(&layout, &channels[ch as usize], &tiles);
    let caught: Vec<usize> = {
        let mut tile = 0;
        let mut out = Vec::new();
        for b in &want {
            if b.last {
                if b.crc_error {
                    out.push(tile);
                }
                tile += 1;
            }
        }
        out
    };
    assert_eq!(caught, flipped, "the twin catches every flipped tile and no other");
    assert!(kinds.iter().all(|&n| n > 0), "a flip in rows, padding, check words and first bytes: {kinds:?}");
    let (got, _) = run(&layout, &channels[ch as usize], &tiles, STORMY);
    same_beats(&got, &want, "flipped");
}

/// A tile whose check word follows its rows with no padding: the last beat keeps its rows' bytes and zeroes the check
/// word's beside them.
#[test]
fn the_last_beat_keeps_only_rows() {
    if !common::require_tool("iverilog") {
        return;
    }
    let rows = 96u64;
    let mut source = Tensors::default();
    let values: Vec<f32> = (0..17 * rows).map(|i| 1.0 + i as f32).collect();
    source.f32("v", &[17, rows], &values);
    let table = TensorTable::from_rows(0, vec![("v".to_string(), vec![17, rows], 0)]).unwrap();
    let layout = ImageLayout { tile_lines: 1, ..ImageLayout::d5005(LineBeat::Full72) };
    let image = weight_image(&table, &[0], &layout).unwrap();
    let channels = image.fill(&mut source).unwrap();
    for ch in 0..layout.channels {
        let tiles = descriptors(&image, ch);
        assert!(tiles.iter().all(|t| t.lines == 1 && t.bytes + CHECK_BYTES == layout.line_bytes), "{tiles:?}");
        let (got, _) = run(&layout, &channels[ch as usize], &tiles, Weather { seed: 5 + ch, ..STORMY });
        let want = expected(&layout, &channels[ch as usize], &tiles);
        assert!(want.iter().filter(|b| b.last).all(|b| b.keep == 4 && b.data[4..].iter().all(|&x| x == 0)));
        same_beats(&got, &want, &format!("channel {ch}"));
    }
}

/// With the channel and the consumer never waiting, the engine keeps up with the channel's line rate across tile
/// boundaries: a tensor of many 511-line tiles streams in its lines' time at the channel's clock, a small fill and
/// drain beside.
#[test]
fn the_engine_streams_at_the_channels_line_rate() {
    if !common::require_tool("iverilog") {
        return;
    }
    let rows = 4096u64;
    let mut source = Tensors::default();
    let mut s = 0x2545_F491_4F6C_DD1Du64;
    let bytes: Vec<u8> = (0..rows * 272)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s as u8
        })
        .collect();
    source.encoded("m", GgmlType::Q8_0, &[256, rows], bytes);
    let table = TensorTable::from_rows(0, vec![("m".to_string(), vec![256, rows], 8)]).unwrap();
    let layout = ImageLayout::d5005(LineBeat::Full72);
    let image = weight_image(&table, &[0], &layout).unwrap();
    let channels = image.fill(&mut source).unwrap();
    let tiles = descriptors(&image, 0);
    assert!(tiles.len() >= 7 && tiles.iter().all(|t| t.lines >= 500), "{tiles:?}");
    let (got, span) = run(&layout, &channels[0], &tiles, CALM);
    same_beats(&got, &expected(&layout, &channels[0], &tiles), "calm");
    let lines: u64 = tiles.iter().map(|t| t.lines).sum();
    let line_time = lines * 3334;
    assert!(span <= line_time + line_time / 50 + 200_000, "{span} ps for {lines} lines (their time {line_time} ps)");
}

// ── The twin and the equations ───────────────────────────────────────────────────────────────────────

/// A tile's beats are its rows: concatenated, the bytes they keep are the rows `Image::locate` finds, from the tile's
/// first row, each beat tagged with its tensor and first row and placed at its offset.
#[test]
fn a_tiles_beats_are_its_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = mixed_checkpoint(dir.path());
    let mut g = Gguf::open(&path).unwrap();
    let layouts = [
        ImageLayout::d5005(LineBeat::Full72),
        ImageLayout::d5005(LineBeat::Data64),
        ImageLayout { tile_lines: 3, ..ImageLayout::d5005(LineBeat::Full72) },
    ];
    for layout in layouts {
        let (image, channels) = tiny_image(&path, &layout);
        for ch in 0..layout.channels {
            for d in descriptors(&image, ch) {
                let bs = beats(&channels[ch as usize], &d, &layout);
                let p = &image.tensors[d.tensor as usize];
                let rows = d.bytes / p.row_bytes;
                assert_eq!(image.locate(d.tensor as usize, d.first_row), (ch, d.line * layout.line_bytes));
                let want = WeightSource::row_bytes(&mut g, &p.name, d.first_row as usize, rows as usize).unwrap();
                let got: Vec<u8> = bs.iter().flat_map(|b| b.data[..b.keep as usize].to_vec()).collect();
                assert_eq!(got, want, "`{}` rows from {}", p.name, d.first_row);
                for (k, b) in bs.iter().enumerate() {
                    assert_eq!((b.tensor, b.row, b.offset), (d.tensor, d.first_row, k as u64 * BEAT_BYTES));
                    assert!(b.data[b.keep as usize..].iter().all(|&x| x == 0));
                    assert_eq!(b.last, k + 1 == bs.len());
                }
            }
        }
    }
}

/// The engine's CRC equations are CRC-32C's: a register after a line is the masks' exclusive-ors of the register and
/// the line's bits, for whole lines and for a last line's bytes before its check word, and a tile's lines through them
/// end, inverted, in the tile's CRC-32C.
#[test]
fn the_crc_equations_are_crc32c() {
    let mut s = 0x9E37_79B9u32;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s
    };
    for len in [72usize, 68, 64, 60] {
        let masks = crc_masks(len);
        let apply = |state: u32, line: &[u8]| -> u32 {
            masks.iter().enumerate().fold(0u32, |out, (i, (sm, dm))| {
                let parity = (state & sm).count_ones()
                    + dm.iter().enumerate().filter(|&(k, &on)| on && line[k / 8] >> (k % 8) & 1 == 1).count() as u32;
                out | (parity & 1) << i
            })
        };
        for _ in 0..40 {
            let state = next();
            let line: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            assert_eq!(apply(state, &line), crc32c_update(state, &line), "{len} bytes");
        }
    }
    let tile: Vec<u8> = (0..72 * 5).map(|_| next() as u8).collect();
    let mut reg = !0u32;
    for line in tile[..72 * 4].chunks(72) {
        reg = crc32c_update(reg, line);
    }
    reg = crc32c_update(reg, &tile[72 * 4..72 * 5 - 4]);
    assert_eq!(!reg, crc32c(&tile[..72 * 5 - 4]));
}

/// The printed engine is well-formed for both of the card's layouts and carries its CRC equations only where the
/// layout carries check words.
#[test]
fn the_engine_prints_for_both_beats() {
    let checked = verilog(&ImageLayout::d5005(LineBeat::Full72));
    let plain = verilog(&ImageLayout::d5005(LineBeat::Data64));
    assert_eq!(checked.matches("assign crc_full[").count(), 32);
    assert_eq!(checked.matches("assign crc_last[").count(), 32);
    assert!(checked.contains("input  [575:0]     mr_data"));
    assert!(!plain.contains("crc_full") && plain.contains("input  [511:0]     mr_data"));
    assert!(plain.contains("assign out_crc_error = 1'b0;"));
}
