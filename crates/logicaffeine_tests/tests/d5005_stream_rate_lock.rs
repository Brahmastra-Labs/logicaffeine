//! The weight stream's rate, proven clock for clock. One channel's path as the streaming matvec wires it — the tile run
//! walked into descriptors, the image stream reading the tiles' lines off the channel, the column feed dealing their
//! blocks to the channel's cores — is unrolled under a channel that has a line whenever one is asked for and cores that
//! take a beat every clock, and every handshake is proven the constant it is whatever the lines hold
//! (`logicaffeine_proof::streams::schedule`, each derivation re-checked). The clocks are then held to the law. A set
//! whose blocks are a beat or more (Q4_K's 144 bytes, Q8_0 taken in octets of 272) gives the feed a beat every clock
//! and loses one clock between tiles, the image stream retiring a tile before it reads the next one's lines; each
//! tile's lines are taken within its beats' clocks, at 64-byte lines each a clock ahead of its beat. A set of smaller
//! blocks (plain Q8_0's 34 bytes) is dealt a block a clock, its beats spanning one clock fewer than it has blocks: about
//! half the channel.
//!
//! Downstream, the streaming column (`logos_column_<format>`) offered a beat whenever it is ready takes exactly
//! `column_clocks` from the edge that takes `start` to the edge that raises its last sum, whatever the beats and the
//! activations hold, over a sweep of groups, pairs, blocks, token-rows and word counts: its datapath (the processing
//! elements, the lanes, the adds, the decoders, the activation memory) cut out as units nothing is known of, so the
//! proof is of the control alone.

mod common;

use std::collections::HashMap;

use logicaffeine_compile::codegen_sva::netlist::{synthesize, synthesize_cut, Undefined};
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::spatial::d5005::Beat;
use logicaffeine_compile::spatial::decode_engine::image::{weight_image, ImageLayout, TensorTable};
use logicaffeine_compile::spatial::decode_engine::numerics::Words;
use logicaffeine_compile::spatial::decode_engine::rtl::column::{column_clocks, column_passes, column_verilog, ColumnSet};
use logicaffeine_compile::spatial::decode_engine::rtl::stream::{AW, BEAT_BYTES, BYW, LW, ROWW, TW};
use logicaffeine_compile::spatial::decode_engine::rtl::streaming::{streaming_verilog, TileRun};
use logicaffeine_proof::fraig;
use logicaffeine_proof::streams::{schedule, Environment, Ports, Schedule, StreamSide};
use logicaffeine_proof::units::Binding;

/// One channel of `logos_streaming_matvec_<set>`: its tile run, image stream and feed wired as the matvec wires them,
/// `beat` high on each clock a beat crosses from the stream into the feed.
fn channel_verilog(set: &str, line_bytes: u64) -> String {
    format!(
        r#"
module logos_channel_path #(parameter N = 2)
  (input clk, input rst, input start,
   input [31:0] count, input [{aw}:0] line, input [{lw}:0] lines, input [{byw}:0] bytes, input [{lw}:0] last_lines,
   input [{byw}:0] last_bytes, input [{roww}:0] row, input [{roww}:0] row_step, input [{tw}:0] tensor,
   input [15:0] blocks, input [31:0] rows,
   output md_valid, input md_ready,
   input mr_valid, output mr_ready, input [{dw}:0] mr_data,
   output beat, output [N-1:0] w_valid, input [N-1:0] w_ready, output busy);
  wire tile_valid, tile_ready, in_valid, in_ready;
  wire [{aw}:0] tile_line;
  wire [{lw}:0] tile_lines;
  wire [{byw}:0] tile_bytes, in_offset;
  wire [{tw}:0] tile_tensor;
  wire [{roww}:0] tile_row, in_row;
  wire [511:0] in_data;
  wire [6:0] in_keep;
  assign beat = in_valid && in_ready;
  logos_tile_run walk(.clk(clk), .rst(rst), .start(start), .count(count), .line(line), .lines(lines), .bytes(bytes),
    .last_lines(last_lines), .last_bytes(last_bytes), .row(row), .row_step(row_step), .tensor(tensor),
    .tile_valid(tile_valid), .tile_ready(tile_ready), .tile_line(tile_line), .tile_lines(tile_lines),
    .tile_bytes(tile_bytes), .tile_tensor(tile_tensor), .tile_row(tile_row), .busy(busy));
  logos_image_stream stream(.clk(clk), .rst_n(!rst), .tile_valid(tile_valid), .tile_ready(tile_ready),
    .tile_line(tile_line), .tile_lines(tile_lines), .tile_bytes(tile_bytes), .tile_tensor(tile_tensor),
    .tile_row(tile_row), .md_valid(md_valid), .md_ready(md_ready), .md_data(),
    .mr_valid(mr_valid), .mr_ready(mr_ready), .mr_data(mr_data), .out_valid(in_valid),
    .out_ready(in_ready), .out_data(in_data), .out_keep(in_keep), .out_last(), .out_tensor(),
    .out_row(in_row), .out_offset(in_offset), .out_crc_error());
  logos_column_feed_{set} #(.N(N)) feed(.clk(clk), .rst(rst), .blocks(blocks), .rows(rows),
    .in_valid(in_valid), .in_ready(in_ready), .in_data(in_data), .in_keep(in_keep), .in_row(in_row),
    .in_offset(in_offset), .w_valid(w_valid), .w_ready(w_ready), .w_beat());
endmodule
"#,
        aw = AW - 1,
        lw = LW - 1,
        byw = BYW - 1,
        roww = ROWW - 1,
        tw = TW - 1,
        dw = 8 * line_bytes - 1
    )
}

/// A matrix streamed through one channel: its format, taken in octets or not, `elems` elements a row, `rows` rows,
/// laid in stripes of `pairs` row pairs a core's.
#[derive(Clone, Copy, Debug)]
struct Matrix {
    ty: GgmlType,
    octets: bool,
    elems: u64,
    rows: u64,
    pairs: u64,
}

const Q4K: Matrix = Matrix { ty: GgmlType::Q4K, octets: false, elems: 512, rows: 64, pairs: 2 };
/// Tiles of 27 beats but the last, of 9: a channel's last tile short.
const Q4K_SHORT: Matrix = Matrix { ty: GgmlType::Q4K, octets: false, elems: 512, rows: 50, pairs: 3 };
const Q8_0_OCTETS: Matrix = Matrix { ty: GgmlType::Q8_0, octets: true, elems: 512, rows: 64, pairs: 2 };
const Q8_0_PLAIN: Matrix = Matrix { ty: GgmlType::Q8_0, octets: false, elems: 512, rows: 64, pairs: 2 };

/// The clocks the reset is held; `start` rises on the next.
const RESET: usize = 2;

/// Channel 0's tile run of `m` under `beat` lines, and its schedule: the clocks the channel's lines are taken at and
/// the clocks a beat crosses into the feed, `verilog` the library the channel is built from (the streaming matvec's
/// unless a mutant's).
fn channel(m: Matrix, beat: Beat, verilog: Option<&dyn Fn(&str) -> String>) -> (TileRun, ImageLayout, Schedule) {
    let layout = ImageLayout { stripe: Some(m.pairs), octets: m.octets, ..ImageLayout::d5005(beat) };
    let mut table = TensorTable::from_rows(0, vec![("w".to_string(), vec![m.elems, m.rows], m.ty.id())]).unwrap();
    table.tensors[0].streamed = true;
    let image = weight_image(&table, &[0], &layout).unwrap();
    let run = TileRun::of(&image, 0, 0);
    let name = if m.octets { "q8o" } else if m.ty == GgmlType::Q8_0 { "q80" } else { "q4k" };
    let set = if m.octets { ColumnSet::new(name, &[m.ty], Words::One).with_octets() } else { ColumnSet::new(name, &[m.ty], Words::One) };
    let library = streaming_verilog(&[set], &layout);
    let library = verilog.map_or(library.clone(), |f| f(&library));
    let v = format!("{library}\n{}", channel_verilog(name, layout.line_bytes));
    let net = synthesize(&v, "logos_channel_path", &[("N", &m.pairs.to_string())]).unwrap_or_else(|e| panic!("{m:?}: {e}"));
    let block = if m.octets { 256 } else { m.ty.block_len() as u64 };
    let values: HashMap<&str, u64> = HashMap::from([
        ("count", run.count),
        ("line", run.line),
        ("lines", run.lines),
        ("bytes", run.bytes),
        ("last_lines", run.last_lines),
        ("last_bytes", run.last_bytes),
        ("row", run.row),
        ("row_step", run.row_step),
        ("tensor", 0),
        ("blocks", m.elems / block),
        ("rows", m.rows),
    ]);
    let other = move |name: &str, k: usize| -> Binding {
        let (base, bit) = match name.find('[') {
            Some(p) => (&name[..p], name[p + 1..name.len() - 1].parse::<u32>().unwrap()),
            None => (name, 0),
        };
        match base {
            "rst" => Binding::Const(k < RESET),
            "start" => Binding::Const(k == RESET),
            "md_ready" | "w_ready" => Binding::Const(true),
            // The clock is no signal of the unrolled circuit: free, whatever reads it.
            "clk" => Binding::Shared("clk".into()),
            b => match values.get(b) {
                Some(&v) => Binding::Const(v >> bit & 1 == 1),
                None => panic!("{name}: an input the channel's environment does not hold"),
            },
        }
    };
    let after_start = |k: usize| k > RESET;
    let token = |_: usize, _: &str| Binding::Fresh;
    let blocks: u64 = run.descriptors().iter().map(|d| d.bytes.div_ceil(m.ty.block_bytes() as u64 * if m.octets { 8 } else { 1 })).sum();
    let beats: u64 = run.descriptors().iter().map(|d| d.bytes.div_ceil(BEAT_BYTES) + 1).sum();
    let frames = RESET + 8 + beats.max(blocks) as usize;
    let env = Environment { frames, offer: &after_start, accept: &after_start, other: &other };
    let data: Vec<String> = (0..8 * layout.line_bytes).map(|i| format!("mr_data[{i}]")).collect();
    let ports = Ports { in_valid: "mr_valid".into(), in_ready: "mr_ready".into(), in_data: data, out_valid: "beat".into(), out_ready: None, out_data: vec![], token: &token };
    let side = StreamSide { aig: &net.aig, units: &[], ports };
    let s = schedule(&side, &env, &fraig::Options::default()).unwrap_or_else(|e| panic!("{m:?} at {beat:?}: {e}"));
    assert!(s.check(&side, &env), "{m:?} at {beat:?}: the schedule's derivation checks");
    let lines: u64 = run.descriptors().iter().map(|d| d.lines).sum();
    assert_eq!(s.took.len() as u64, lines, "{m:?} at {beat:?}: every line of the run is taken within {frames} clocks");
    (run, layout, s)
}

/// Where a channel of beat-sized blocks departs from the law: each tile's beats one a clock from the second clock after
/// its first line, a clock between tiles, each tile's lines within its beats' clocks — at 64-byte lines each the clock
/// before its beat.
fn off_the_beat_law(run: &TileRun, layout: &ImageLayout, s: &Schedule) -> Option<String> {
    if s.took.first() != Some(&(RESET + 2)) || s.gave.first() != Some(&(RESET + 3)) {
        return Some(format!("the first line at {:?} and its beat at {:?}: not the second and third clocks after start", s.took.first(), s.gave.first()));
    }
    let (mut beats, mut lines) = (Vec::new(), Vec::new());
    let mut at = RESET + 3;
    for (k, d) in run.descriptors().iter().enumerate() {
        let n = d.bytes.div_ceil(BEAT_BYTES) as usize;
        beats.extend(at..at + n);
        let window = at - 1..at - 1 + n;
        let taken: Vec<usize> = s.took.iter().copied().filter(|t| window.contains(t)).collect();
        if taken.len() as u64 != d.lines {
            return Some(format!("tile {k}: {} of its {} lines within its beats' clocks {window:?}", taken.len(), d.lines));
        }
        if layout.line_bytes == BEAT_BYTES && taken != window.clone().take(d.lines as usize).collect::<Vec<_>>() {
            return Some(format!("tile {k}: its lines at {taken:?}, not each the clock before its beat"));
        }
        lines.extend(taken);
        at += n + 1;
    }
    if s.gave != beats {
        return Some(format!("the beats at {:?}, not {beats:?}", s.gave));
    }
    if s.took != lines {
        return Some("a line taken outside every tile's beats' clocks".into());
    }
    None
}

#[test]
fn a_set_of_beat_sized_blocks_takes_a_beat_every_clock_and_loses_one_clock_between_tiles() {
    if !common::require_tool("yosys") {
        return;
    }
    for m in [Q4K, Q4K_SHORT, Q8_0_OCTETS] {
        for beat in [Beat::Data64, Beat::Full72] {
            let (run, layout, s) = channel(m, beat, None);
            if let Some(why) = off_the_beat_law(&run, &layout, &s) {
                panic!("{m:?} at {beat:?} ({run:?}): {why}");
            }
        }
    }
}

#[test]
fn plain_q8_0_is_dealt_a_block_a_clock_about_half_the_channel() {
    if !common::require_tool("yosys") {
        return;
    }
    for beat in [Beat::Data64, Beat::Full72] {
        let (run, layout, s) = channel(Q8_0_PLAIN, beat, None);
        assert!(off_the_beat_law(&run, &layout, &s).is_some(), "a 34-byte block cannot keep a beat a clock");
        let bytes: u64 = run.descriptors().iter().map(|d| d.bytes).sum();
        let blocks = bytes / GgmlType::Q8_0.block_bytes() as u64;
        let span = (s.gave.last().unwrap() - s.gave[0] + 1) as u64;
        assert_eq!(s.gave.len() as u64, bytes.div_ceil(BEAT_BYTES), "{beat:?}: every beat crosses");
        assert_eq!(span, blocks - 1, "{beat:?}: the beats span one clock fewer than the {blocks} blocks the feed deals a clock each");
    }
}

#[test]
fn an_image_stream_holding_one_tile_at_a_time_breaks_the_law() {
    if !common::require_tool("yosys") {
        return;
    }
    let honest = "wire       q_full = (qt - qh) == 3'd4;";
    let one = |library: &str| -> String {
        assert_eq!(library.matches(honest).count(), 1, "the mutation point is the image stream's four outstanding tiles");
        library.replace(honest, "wire       q_full = (qt - qh) == 3'd1;")
    };
    let (run, layout, s) = channel(Q4K, Beat::Data64, Some(&one));
    assert!(off_the_beat_law(&run, &layout, &s).is_some(), "a tile's descriptor taken only once the last has retired: {:?}", s.took);
}

/// The streaming column of `ty` with its datapath cut out: every module the library defines but the column's own
/// (`logos_column_*`, its control) read as a unit whose results are free.
fn column_netlist(ty: GgmlType) -> (logicaffeine_compile::codegen_sva::netlist::CutNetlist, Vec<logicaffeine_proof::units::Unit>) {
    let v = column_verilog();
    let top = format!("logos_column_{}", ty.name().to_lowercase());
    let datapath: Vec<String> = v
        .lines()
        .filter_map(|l| l.strip_prefix("module "))
        .map(|rest| rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>())
        .filter(|m| !m.starts_with("logos_column"))
        .collect();
    let cut: Vec<&str> = datapath.iter().map(String::as_str).collect();
    let net = synthesize_cut(&v, &top, &[], Undefined::Free, &cut).unwrap_or_else(|e| panic!("{top}: {e}"));
    let units = net.units(&|_| 0, &["clk"]);
    (net, units)
}

/// The column of `ty` offered a beat whenever it is ready, over every shape of the sweep: each beat taken once, a sum a
/// pair a token-row a group, the last `column_clocks` after the edge that takes `start`, whatever the data.
fn column_clocks_hold(ty: GgmlType) {
    {
        let (net, units) = column_netlist(ty);
        let bb = ty.block_bytes();
        let data: Vec<String> = (0..8 * bb).map(|i| format!("w_blk1[{i}]")).chain((0..8 * bb).map(|i| format!("w_blk2[{i}]"))).collect();
        let token = |_: usize, _: &str| Binding::Fresh;
        let mut shapes = 0;
        for two in [false, true] {
            for pairs in [1usize, 2, 4] {
                for tokens in [1usize, 2, 4] {
                    for groups in [1usize, 3] {
                        for blocks in [2usize, 3] {
                            let passes = column_passes(ty, two);
                            let want = column_clocks(groups, pairs, blocks, tokens, passes);
                            let values: HashMap<&str, u64> =
                                HashMap::from([("groups", groups as u64), ("pairs", pairs as u64), ("blocks", blocks as u64), ("tokens", tokens as u64), ("two", two as u64)]);
                            let other = move |name: &str, k: usize| -> Binding {
                                let (base, bit) = match name.find('[') {
                                    Some(p) => (&name[..p], name[p + 1..name.len() - 1].parse::<u32>().unwrap()),
                                    None => (name, 0),
                                };
                                match base {
                                    "rst" => Binding::Const(k < RESET),
                                    "start" => Binding::Const(k == RESET),
                                    "act_wr" => Binding::Const(false),
                                    "clk" | "act_t" | "act_b" | "act_codes" | "act_m" | "act_e" => Binding::Shared(name.to_string()),
                                    b => match values.get(b) {
                                        Some(&v) => Binding::Const(v >> bit & 1 == 1),
                                        None => panic!("{name}: an input the column's environment does not hold"),
                                    },
                                }
                            };
                            let after_start = |k: usize| k > RESET;
                            let env = Environment { frames: RESET + want + 8, offer: &after_start, accept: &after_start, other: &other };
                            let ports = Ports { in_valid: "w_valid".into(), in_ready: "w_ready".into(), in_data: data.clone(), out_valid: "o_valid".into(), out_ready: None, out_data: vec![], token: &token };
                            let side = StreamSide { aig: &net.netlist.aig, units: &units, ports };
                            let what = format!("{} {}: {groups} groups of {pairs} pairs, {blocks} blocks, {tokens} token-rows", ty.name(), if two { "two words" } else { "one word" });
                            let s = schedule(&side, &env, &fraig::Options::default()).unwrap_or_else(|e| panic!("{what}: {e}"));
                            assert!(s.check(&side, &env), "{what}: the schedule's derivation checks");
                            assert_eq!(s.took.len(), groups * blocks * pairs, "{what}: every pair's beat of every block of every group taken once");
                            assert_eq!(s.gave.len(), groups * pairs * tokens, "{what}: a sum a pair a token-row a group");
                            assert_eq!(s.gave.last().map(|&k| k - (RESET + 1)), Some(want), "{what}: the last sum column_clocks after the edge that takes start");
                            shapes += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(shapes, 72);
    }
}

#[test]
fn the_q4_k_column_fed_whenever_it_is_ready_takes_exactly_column_clocks_whatever_its_data() {
    if common::require_tool("yosys") {
        column_clocks_hold(GgmlType::Q4K);
    }
}

#[test]
fn the_q8_0_column_fed_whenever_it_is_ready_takes_exactly_column_clocks_whatever_its_data() {
    if common::require_tool("yosys") {
        column_clocks_hold(GgmlType::Q8_0);
    }
}
