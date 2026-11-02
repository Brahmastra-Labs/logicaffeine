//! The D5005 weight image: a checkpoint's tensors laid over the card's four memory channels in the order a pass reads
//! them.
//!
//! `spatial::decode_engine::image` cuts each tensor into tiles of whole rows and deals them to the channels least
//! loaded first; `benchmarks/silicon/d5005/weight_image.json` sums the layout of every Qwen3.8-27B checkpoint at both
//! beats. These tests hold the order to the reads the engine's own step makes, every tensor byte to exactly one place
//! that `locate` finds, the channels to within one tile of each other after every tile, each channel's tiles to the
//! read order end to end, the check words to every flipped bit, CRC-32C to its published check values, and the
//! ledger to the layout. Laid in stripes for the streaming engine's array, every tensor a matvec job streams is
//! dealt in tiles of its stripe's row pairs, each block-major, the four tiles of a stripe on the four channels.

#![cfg(feature = "codegen")]

mod common;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::reference::WeightSource;
use logicaffeine_compile::model::spec::ModelSpec;
use logicaffeine_compile::spatial::d5005::Beat;
use logicaffeine_compile::spatial::decode::GgufHeader;
use logicaffeine_compile::spatial::decode_engine::image::{
    crc32c, image_ledger, image_of, machine_of, weight_image, Image, ImageError, ImageLayout, TensorTable, CHECK_BYTES, HEADERS,
    TILE_LINES,
};
use logicaffeine_compile::spatial::decode_engine::machine::{Layout as MachineLayout, Machine};
use logicaffeine_compile::spatial::decode_engine::model::{EngineModel, EngineNumerics};
use logicaffeine_compile::spatial::decode_engine::program::lower;
use tiny_models::Tiny;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn decode_dir() -> PathBuf {
    repo().join("benchmarks/silicon/decode")
}

fn header(name: &str) -> GgufHeader {
    let path = decode_dir().join(format!("headers/{name}.json"));
    GgufHeader::read_ledger(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The tiny Qwen3.5 checkpoint with its matrices in K-quant formats (rows of 144 and 210 bytes, which no line
/// width divides), its table in Q8_0 and its vectors in f32.
fn mixed_checkpoint(dir: &Path) -> (ModelSpec, PathBuf) {
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
    (spec, path)
}

fn table_of(spec: &ModelSpec, g: &Gguf) -> TensorTable {
    TensorTable::from_rows(spec.layers.len() as u64, g.tensor_names()).expect("the tiny table")
}

/// The layouts the round trip runs: the card's two, and small odd ones that cut every tensor into many tiles.
fn layouts() -> Vec<ImageLayout> {
    let card = ImageLayout::d5005(Beat::Full72);
    vec![
        ImageLayout::d5005(Beat::Data64),
        card,
        ImageLayout { tile_lines: 2, ..card },
        ImageLayout { tile_lines: 3, channels: 3, line_bytes: 40, check: false, ..card },
        ImageLayout { tile_lines: 1, channels: 5, line_bytes: 24, ..card },
    ]
}

// ── The order ────────────────────────────────────────────────────────────────────────────────────────

/// A source that remembers the order its tensors are first read in.
struct Recording {
    inner: Gguf,
    reads: Arc<Mutex<Vec<String>>>,
}

impl WeightSource for Recording {
    fn describe(&self, name: &str) -> std::io::Result<(GgmlType, Vec<u64>)> {
        WeightSource::describe(&self.inner, name)
    }
    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>> {
        let mut reads = self.reads.lock().unwrap();
        if !reads.iter().any(|n| n == name) {
            reads.push(name.to_string());
        }
        drop(reads);
        WeightSource::row_bytes(&mut self.inner, name, first, count)
    }
    fn has(&self, name: &str) -> bool {
        WeightSource::has(&self.inner, name)
    }
}

/// The image's order is the order the engine's step first reads the checkpoint's tensors in, the step reads every
/// tensor the tiny checkpoint holds, and the card's image from the machine layout lays them in that order too.
#[test]
fn the_image_is_laid_in_the_order_the_engine_reads() {
    let dir = tempfile::tempdir().unwrap();
    for (name, (spec, path)) in [
        ("q8_0", {
            let spec = Tiny::Qwen35.wide(64);
            let path = dir.path().join("qwen35-q8.gguf");
            tiny_models::write_checkpoint(&spec, &path);
            (spec, path)
        }),
        ("mixed", mixed_checkpoint(dir.path())),
    ] {
        let g = Gguf::open(&path).unwrap();
        let table = table_of(&spec, &g);
        let order: Vec<String> = table.read_order().iter().map(|&i| table.tensors[i].name.clone()).collect();
        let reads = Arc::new(Mutex::new(Vec::new()));
        let source = Recording { inner: g, reads: reads.clone() };
        let mut e = EngineModel::new(spec.clone(), source, EngineNumerics::precise()).expect("the engine runs qwen35");
        let s = e.sequence();
        e.step(&[(s, vec![3, 17])]).expect("a step");
        let reads = reads.lock().unwrap().clone();
        assert_eq!(order, reads, "{name}: the image's order is the step's");
        let image = weight_image(&table, &table.read_order(), &ImageLayout::d5005(Beat::Data64)).unwrap();
        let laid: Vec<&str> = image.tensors.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(laid, reads.iter().map(String::as_str).collect::<Vec<_>>(), "{name}: the image lays them in that order");
        let numerics = EngineNumerics::precise();
        let machine = MachineLayout::of(&spec, numerics, &lower(&spec, numerics, 1).unwrap().program, &Gguf::open(&path).unwrap(), 1, 1).unwrap();
        let card = image_of(&machine, &ImageLayout::d5005(Beat::Full72)).unwrap();
        let carded: Vec<&str> = card.tensors.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(carded, reads.iter().map(String::as_str).collect::<Vec<_>>(), "{name}: the card's image lays them in that order");
    }
}

/// On every Qwen3.8-27B checkpoint the step's blocks hold no tensor the order does not place in the step: after the
/// head come only the drafting block's.
#[test]
fn the_27b_order_leaves_only_the_drafting_block_after_the_head() {
    for &name in HEADERS {
        let h = header(name);
        let table = TensorTable::of_header(&h).unwrap();
        let blocks = h.meta_u64("block_count").unwrap();
        assert_eq!(table.blocks, blocks - h.meta_u64("nextn_predict_layers").unwrap(), "{name}");
        let order = table.read_order();
        let head = order.iter().position(|&i| table.tensors[i].name == "output.weight").expect("a head");
        assert_eq!(table.tensors[order[0]].name, "token_embd.weight", "{name}");
        assert_eq!(table.tensors[order[head - 1]].name, "output_norm.weight", "{name}");
        for &i in &order[head + 1..] {
            let n = &table.tensors[i].name;
            assert!(n.starts_with(&format!("blk.{}.", table.blocks)), "{name}: `{n}` comes after the head");
        }
        for &i in &order[1..head - 1] {
            let n = &table.tensors[i].name;
            let b: u64 = n.strip_prefix("blk.").and_then(|r| r.split('.').next()).and_then(|b| b.parse().ok()).unwrap_or(u64::MAX);
            assert!(b < table.blocks, "{name}: `{n}` is read in the step");
        }
        let mut seen = HashSet::new();
        assert!(order.iter().all(|&i| seen.insert(i)) && seen.len() == table.tensors.len(), "{name}: every tensor once");
    }
}

/// A drafting block follows the head in the table's own order, and a head tied to the table reads the table already
/// placed first.
#[test]
fn a_drafting_block_and_a_tied_head_take_their_places() {
    let row = |name: &str, dims: &[u64], id: u32| (name.to_string(), dims.to_vec(), id);
    let rows = vec![
        row("blk.1.nextn.eh_proj.weight", &[64, 32], 0),
        row("output.weight", &[32, 96], 8),
        row("blk.0.ffn_up.weight", &[32, 64], 8),
        row("token_embd.weight", &[32, 96], 8),
        row("blk.1.attn_norm.weight", &[32], 0),
        row("blk.0.attn_norm.weight", &[32], 0),
        row("output_norm.weight", &[32], 0),
        row("blk.0.ffn_down.weight", &[64, 32], 8),
    ];
    let table = TensorTable::from_rows(1, rows.clone()).unwrap();
    let names = |t: &TensorTable| t.read_order().iter().map(|&i| t.tensors[i].name.clone()).collect::<Vec<_>>();
    assert_eq!(
        names(&table),
        [
            "token_embd.weight",
            "blk.0.attn_norm.weight",
            "blk.0.ffn_up.weight",
            "blk.0.ffn_down.weight",
            "output_norm.weight",
            "output.weight",
            "blk.1.nextn.eh_proj.weight",
            "blk.1.attn_norm.weight"
        ]
    );
    let tied = TensorTable::from_rows(1, rows.into_iter().filter(|r| r.0 != "output.weight")).unwrap();
    let order = names(&tied);
    assert_eq!(order[0], "token_embd.weight");
    assert_eq!(order[4], "output_norm.weight");
    assert_eq!(order.iter().filter(|n| *n == "token_embd.weight").count(), 1);
}

// ── Every byte once ──────────────────────────────────────────────────────────────────────────────────

/// Every row of every tensor is where `locate` says, byte for byte; each channel's tiles run end to end from line 0
/// to its last; the bytes between a tile's rows and its end (or its check word) are zero; every check word is its
/// tile's CRC-32C.
#[test]
fn every_tensor_byte_lands_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = mixed_checkpoint(dir.path());
    let mut g = Gguf::open(&path).unwrap();
    let table = table_of(&spec, &g);
    for layout in layouts() {
        let image = weight_image(&table, &table.read_order(), &layout).unwrap();
        let channels = image.fill(&mut g).unwrap();
        let lb = layout.line_bytes as usize;
        for (i, p) in image.tensors.iter().enumerate() {
            for r in 0..p.rows {
                let (ch, at) = image.locate(i, r);
                let want = WeightSource::row_bytes(&mut g, &p.name, r as usize, 1).unwrap();
                assert_eq!(&channels[ch as usize][at as usize..at as usize + want.len()], &want[..], "{layout:?}: {} row {r}", p.name);
            }
        }
        let mut owned: Vec<Vec<u8>> = channels.iter().map(|c| vec![0u8; c.len()]).collect();
        for (k, t) in image.tiles.iter().enumerate() {
            let p = &image.tensors[t.tensor];
            let span = t.line as usize * lb..(t.line + t.lines) as usize * lb;
            for b in span.clone() {
                owned[t.channel as usize][b] += 1;
            }
            let data = ((t.end_row - t.first_row) * p.row_bytes) as usize;
            let check = if layout.check { CHECK_BYTES as usize } else { 0 };
            assert_eq!(t.lines, (data + check).div_ceil(lb) as u64, "{layout:?}: tile {k} takes the lines its rows need");
            let bytes = &channels[t.channel as usize][span];
            assert!(bytes[data..bytes.len() - check].iter().all(|&b| b == 0), "{layout:?}: tile {k}'s padding is zero");
            if layout.check {
                let body = bytes.len() - check;
                assert_eq!(bytes[body..], crc32c(&bytes[..body]).to_le_bytes(), "{layout:?}: tile {k}'s check word");
            }
        }
        for (ch, o) in owned.iter().enumerate() {
            assert!(o.iter().all(|&n| n == 1), "{layout:?}: channel {ch} holds every byte in exactly one tile");
        }
        let held: u64 = image.tiles.iter().map(|t| (t.end_row - t.first_row) * image.tensors[t.tensor].row_bytes).sum();
        let stored: u64 = table.tensors.iter().map(|e| e.rows() * e.row_bytes()).sum();
        assert_eq!(held, stored, "{layout:?}");
        assert_eq!(image.first_bad_tile(&channels), None, "{layout:?}");
    }
}

/// A flipped bit anywhere in a tile — its rows, its padding, its check word — is found in that tile; without check
/// words nothing is checked.
#[test]
fn the_check_word_finds_a_flipped_bit() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = mixed_checkpoint(dir.path());
    let mut g = Gguf::open(&path).unwrap();
    let table = table_of(&spec, &g);
    let layout = ImageLayout { tile_lines: 4, ..ImageLayout::d5005(Beat::Full72) };
    let image = weight_image(&table, &table.read_order(), &layout).unwrap();
    let mut bytes = image.fill(&mut g).unwrap();
    let lb = layout.line_bytes as usize;
    let mut padded = 0;
    for (k, t) in image.tiles.iter().enumerate().step_by((image.tiles.len() / 40).max(1)) {
        let start = t.line as usize * lb;
        let end = (t.line + t.lines) as usize * lb;
        let data = ((t.end_row - t.first_row) * image.tensors[t.tensor].row_bytes) as usize;
        let mut sites = vec![start, start + data - 1, end - 1, end - CHECK_BYTES as usize];
        if start + data < end - CHECK_BYTES as usize {
            sites.push(start + data);
            padded += 1;
        }
        for at in sites {
            for bit in [0, 7] {
                bytes[t.channel as usize][at] ^= 1 << bit;
                assert_eq!(image.first_bad_tile(&bytes), Some(k), "a flip at byte {at} bit {bit} of tile {k}");
                bytes[t.channel as usize][at] ^= 1 << bit;
            }
        }
    }
    assert_eq!(image.first_bad_tile(&bytes), None);
    assert!(padded > 0, "some tile has padding to flip");
    let plain = weight_image(&table, &table.read_order(), &ImageLayout::d5005(Beat::Data64)).unwrap();
    let mut bytes = plain.fill(&mut g).unwrap();
    bytes[0][0] ^= 1;
    assert_eq!(plain.first_bad_tile(&bytes), None);
}

/// CRC-32C is Castagnoli's: the catalogue's check value, and RFC 3720's iSCSI vectors (32 zero bytes, 32 bytes of ones,
/// 0 to 31 rising and falling).
#[test]
fn crc32c_is_castagnoli() {
    assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    assert_eq!(crc32c(&[]), 0);
    assert_eq!(crc32c(&[0u8; 32]), 0x8A91_36AA);
    assert_eq!(crc32c(&[0xFFu8; 32]), 0x62A8_AB43);
    assert_eq!(crc32c(&(0..32).collect::<Vec<u8>>()), 0x46DD_794E);
    assert_eq!(crc32c(&(0..32).rev().collect::<Vec<u8>>()), 0x113F_DB5C);
}

// ── The card's images ────────────────────────────────────────────────────────────────────────────────

/// Every Qwen3.8-27B checkpoint's card image at both beats: its machine layout, laid out as the machine stores it.
fn images() -> Vec<(&'static str, Beat, MachineLayout, Image)> {
    let mut out = Vec::new();
    for &name in HEADERS {
        let machine = machine_of(&header(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        for beat in [Beat::Data64, Beat::Full72] {
            let image = image_of(&machine, &ImageLayout::d5005(beat)).unwrap_or_else(|e| panic!("{name} {beat:?}: {e:?}"));
            out.push((name, beat, machine.clone(), image));
        }
    }
    out
}

/// After every tile the channels differ by no more than the largest tile dealt so far, and each tile is dealt to a
/// channel no more loaded than any other was when its tensor began.
#[test]
fn the_channels_stay_within_one_tile_after_every_tile() {
    for (name, beat, _, image) in images() {
        let mut loads = vec![0u64; image.layout.channels as usize];
        let mut largest = 0;
        for (k, t) in image.tiles.iter().enumerate() {
            loads[t.channel as usize] += t.lines;
            largest = largest.max(t.lines);
            let spread = loads.iter().max().unwrap() - loads.iter().min().unwrap();
            assert!(spread <= largest, "{name} {beat:?}: after tile {k} the channels differ by {spread} lines, the largest tile is {largest}");
        }
        assert_eq!(loads, image.channel_lines, "{name} {beat:?}");
        for p in &image.tensors {
            assert!(p.first_line.windows(2).all(|w| w[0] <= w[1]), "{name} {beat:?}: `{}` is dealt least loaded first", p.name);
        }
    }
}

/// Each channel holds its tiles in the read order, end to end from line 0: a channel streams its share of a pass as
/// one run of lines.
#[test]
fn every_channel_streams_its_tiles_in_the_read_order() {
    for (name, beat, _, image) in images() {
        let mut next = vec![0u64; image.layout.channels as usize];
        let mut last_tensor = vec![0usize; image.layout.channels as usize];
        for (k, t) in image.tiles.iter().enumerate() {
            let ch = t.channel as usize;
            assert_eq!(t.line, next[ch], "{name} {beat:?}: tile {k} follows the channel's last");
            assert!(t.tensor >= last_tensor[ch], "{name} {beat:?}: tile {k}");
            next[ch] += t.lines;
            last_tensor[ch] = t.tensor;
        }
        assert_eq!(next, image.channel_lines, "{name} {beat:?}");
    }
}

/// Every tile of every checkpoint is where its tensor's arithmetic puts it: its first row at its first line, its last
/// row its rows' bytes on; each tensor's tiles cover its rows once, in order; no channel overflows.
#[test]
fn a_rows_place_is_arithmetic() {
    for (name, beat, machine, image) in images() {
        let lb = image.layout.line_bytes;
        for (i, p) in image.tensors.iter().enumerate() {
            let tiles = &image.tiles[p.first_tile..p.first_tile + p.tile_count];
            assert_eq!(tiles.first().map(|t| t.first_row), Some(0), "{name} {beat:?}: `{}`", p.name);
            assert_eq!(tiles.last().map(|t| t.end_row), Some(p.rows), "{name} {beat:?}: `{}`", p.name);
            assert!(tiles.windows(2).all(|w| w[0].end_row == w[1].first_row), "{name} {beat:?}: `{}`", p.name);
            assert!(tiles.iter().all(|t| t.tensor == i));
            for t in tiles {
                assert_eq!(image.locate(i, t.first_row), (t.channel, t.line * lb), "{name} {beat:?}: `{}` row {}", p.name, t.first_row);
                let last = t.end_row - 1;
                assert_eq!(image.locate(i, last), (t.channel, t.line * lb + (last - t.first_row) * p.row_bytes), "{name} {beat:?}");
            }
            let placed = &machine.tensors[i];
            assert_eq!(p.name, placed.name);
            assert_eq!(p.rows * p.row_bytes, placed.bytes, "`{}`'s stored rows are its bytes", p.name);
            assert_eq!(p.tap_major, placed.taps.map(|t| t as u64));
        }
        assert!(image.channel_lines.iter().all(|&l| l <= image.layout.capacity_lines), "{name} {beat:?}");
    }
}

/// The image holds every byte of every tensor the machine reads, in the machine's order and no other tensor: its rows
/// sum to the machine layout's bytes, and each of those tensors is the header's own byte count.
#[test]
fn the_27b_images_hold_every_tensor_byte() {
    for (name, beat, machine, image) in images() {
        let h = header(name);
        let laid: Vec<&str> = image.tensors.iter().map(|p| p.name.as_str()).collect();
        let read: Vec<&str> = machine.tensors.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(laid, read, "{name} {beat:?}");
        for p in &machine.tensors {
            let t = h.tensors.iter().find(|t| t.name == p.name).unwrap();
            assert_eq!(p.bytes, h.tensor_bytes(t).unwrap(), "`{}`", p.name);
        }
        let stored: u64 = machine.tensors.iter().map(|p| p.bytes).sum();
        let row = image.row(name, beat);
        assert_eq!(row.data_bytes, stored, "{name} {beat:?}");
        assert_eq!(row.check_bytes, if beat == Beat::Full72 { CHECK_BYTES * row.tiles as u64 } else { 0 });
        let total: u64 = image.channel_bytes().iter().sum();
        assert_eq!(row.padding_bytes + row.data_bytes + row.check_bytes, total);
        assert!(row.spread_lines <= row.largest_tile_lines && row.largest_tile_lines <= TILE_LINES, "{name} {beat:?}");
    }
}

// ── Stripes ──────────────────────────────────────────────────────────────────────────────────────────

/// The machine marks streamed exactly the tensors its matvec jobs stream through the array — every matrix of weight
/// blocks a matvec command reads — and runs a float matrix on the vector unit, on the tiny checkpoint in Q8_0 and in its
/// K-quant mix.
#[test]
fn the_machine_streams_exactly_its_matvec_jobs_matrices() {
    use logicaffeine_compile::spatial::decode_engine::machine::Job;
    let dir = tempfile::tempdir().unwrap();
    let q8 = dir.path().join("qwen35-q8.gguf");
    tiny_models::write_checkpoint(&Tiny::Qwen35.wide(64), &q8);
    for (spec, path) in [(Tiny::Qwen35.wide(64), q8), mixed_checkpoint(dir.path())] {
        let machine = Machine::new(spec, Gguf::open(&path).unwrap(), EngineNumerics::fastest(), 1, 8).unwrap();
        let jobs: HashSet<usize> = machine.jobs.iter().filter_map(|j| if let Job::Matvec { tensor, .. } = j { Some(*tensor) } else { None }).collect();
        assert!(!jobs.is_empty(), "matrices to stream");
        for (i, p) in machine.layout.tensors.iter().enumerate() {
            assert_eq!(p.streamed, jobs.contains(&i), "`{}`", p.name);
            assert!(!p.streamed || !matches!(p.ty, GgmlType::F32 | GgmlType::F16 | GgmlType::Bf16), "`{}` streamed in floats", p.name);
        }
    }
}

/// A layout with a stripe of `n` pairs deals each streamed tensor in tiles of `2n` rows (fewer only in its last),
/// tile `k` to the `k`-th channel of its deal in turn, each tile's bytes every row's block 0, then every row's block
/// 1, …, each block the machine's own bytes of that row's block, where `locate_block` finds it; every other tensor
/// is laid as the layout without a stripe lays it, a row's bytes together; every channel byte lies in exactly one
/// tile, the padding zero, the check words right — at one, two and three pairs, on the card's layouts and small odd
/// ones, for the tiny checkpoint in Q8_0 (blocks of 34 bytes) and in K-quants (144 and 210).
#[test]
fn a_striped_image_holds_each_streamed_tile_block_major() {
    let dir = tempfile::tempdir().unwrap();
    let q8 = dir.path().join("qwen35-q8.gguf");
    tiny_models::write_checkpoint(&Tiny::Qwen35.wide(64), &q8);
    for (spec, path) in [(Tiny::Qwen35.wide(64), q8), mixed_checkpoint(dir.path())] {
        let machine = Machine::new(spec, Gguf::open(&path).unwrap(), EngineNumerics::fastest(), 1, 8).unwrap();
        let memory = machine.memory();
        for pairs in [1u64, 2, 3] {
            for plain in layouts() {
                let layout = ImageLayout { stripe: Some(pairs), ..plain };
                let image = image_of(&machine.layout, &layout).unwrap();
                let flat = image_of(&machine.layout, &plain).unwrap();
                let channels = image.fill(&mut Gguf::open(&path).unwrap()).unwrap();
                let lb = layout.line_bytes;
                for (i, p) in image.tensors.iter().enumerate() {
                    let placed = &machine.layout.tensors[i];
                    let kept = &memory[placed.at as usize..(placed.at + placed.bytes) as usize];
                    let what = format!("{pairs} pairs, {layout:?}: `{}`", p.name);
                    assert_eq!(p.block_major, placed.streamed, "{what}");
                    if !placed.streamed {
                        let f = &flat.tensors[i];
                        assert_eq!((p.rows, p.row_bytes, p.rows_per_tile, p.tile_lines, p.pieces), (f.rows, f.row_bytes, f.rows_per_tile, f.tile_lines, f.pieces), "{what}");
                        for r in 0..p.rows {
                            let (ch, at) = image.locate(i, r);
                            let row = &kept[(r * p.row_bytes) as usize..((r + 1) * p.row_bytes) as usize];
                            assert!(&channels[ch as usize][at as usize..(at + p.row_bytes) as usize] == row, "{what} row {r}");
                        }
                        continue;
                    }
                    let bb = placed.ty.block_bytes() as u64;
                    let blocks = p.row_bytes / bb;
                    assert_eq!((p.block_bytes, p.rows_per_tile), (bb, (2 * pairs).min(p.rows)), "{what}");
                    let tiles = &image.tiles[p.first_tile..p.first_tile + p.tile_count];
                    for (k, t) in tiles.iter().enumerate() {
                        let held = t.end_row - t.first_row;
                        assert_eq!(t.first_row, k as u64 * 2 * pairs, "{what}: tile {k}");
                        assert!(held == (2 * pairs).min(p.rows - t.first_row), "{what}: tile {k} holds {held} rows");
                        assert_eq!(t.channel, p.channels[k % p.channels.len()], "{what}: tile {k}'s channel");
                        for j in 0..held {
                            let r = t.first_row + j;
                            for b in 0..blocks {
                                let at = t.line * lb + (b * held + j) * bb;
                                assert_eq!(image.locate_block(i, r, b), (t.channel, at), "{what}: row {r} block {b}");
                                let want = &kept[(r * p.row_bytes + b * bb) as usize..(r * p.row_bytes + (b + 1) * bb) as usize];
                                assert!(&channels[t.channel as usize][at as usize..(at + bb) as usize] == want, "{what}: row {r} block {b}");
                            }
                        }
                    }
                }
                let mut owned: Vec<Vec<u8>> = channels.iter().map(|c| vec![0u8; c.len()]).collect();
                for (k, t) in image.tiles.iter().enumerate() {
                    let span = (t.line * lb) as usize..((t.line + t.lines) * lb) as usize;
                    for b in span.clone() {
                        owned[t.channel as usize][b] += 1;
                    }
                    let data = ((t.end_row - t.first_row) * image.tensors[t.tensor].row_bytes) as usize;
                    let check = if layout.check { CHECK_BYTES as usize } else { 0 };
                    assert_eq!(t.lines, (data + check).div_ceil(lb as usize) as u64, "{pairs} pairs, {layout:?}: tile {k}");
                    let bytes = &channels[t.channel as usize][span];
                    assert!(bytes[data..bytes.len() - check].iter().all(|&b| b == 0), "{pairs} pairs, {layout:?}: tile {k}'s padding");
                }
                for (ch, o) in owned.iter().enumerate() {
                    assert!(o.iter().all(|&n| n == 1), "{pairs} pairs, {layout:?}: channel {ch} holds every byte in exactly one tile");
                }
                assert_eq!(image.first_bad_tile(&channels), None, "{pairs} pairs, {layout:?}");
            }
        }
    }
}

/// A striped image with a slot lays each streamed block in its slot: the block's bytes at `(b · held + j) · slot`
/// bytes into its tile, then its pad's zeros (the block's bytes rounded up to the slot), its tile's lines and the
/// stream's tile runs counting the slots; a tensor not streamed is laid as without a slot, and a slot of one byte is the
/// image without one — at slots of 16 and 64 bytes, one to three pairs, the card's layouts and small odd ones, for the
/// tiny checkpoint in Q8_0 (blocks of 34 bytes) and in K-quants (144 and 210).
#[test]
fn a_striped_image_lays_each_block_in_its_slot() {
    use logicaffeine_compile::spatial::decode_engine::rtl::stream::descriptors;
    use logicaffeine_compile::spatial::decode_engine::rtl::streaming::TileRun;
    let dir = tempfile::tempdir().unwrap();
    let q8 = dir.path().join("qwen35-q8.gguf");
    tiny_models::write_checkpoint(&Tiny::Qwen35.wide(64), &q8);
    for (spec, path) in [(Tiny::Qwen35.wide(64), q8), mixed_checkpoint(dir.path())] {
        let machine = Machine::new(spec, Gguf::open(&path).unwrap(), EngineNumerics::fastest(), 1, 8).unwrap();
        let memory = machine.memory();
        for pairs in [1u64, 2, 3] {
            for plain in layouts() {
                let striped = ImageLayout { stripe: Some(pairs), ..plain };
                assert_eq!(image_of(&machine.layout, &ImageLayout { slot: 1, ..striped }).unwrap(), image_of(&machine.layout, &striped).unwrap());
                for slot in [16u64, 64] {
                    let layout = ImageLayout { slot, ..striped };
                    let image = image_of(&machine.layout, &layout).unwrap();
                    let packed = image_of(&machine.layout, &striped).unwrap();
                    let channels = image.fill(&mut Gguf::open(&path).unwrap()).unwrap();
                    let lb = layout.line_bytes;
                    let check = if layout.check { CHECK_BYTES } else { 0 };
                    for (i, p) in image.tensors.iter().enumerate() {
                        let placed = &machine.layout.tensors[i];
                        let kept = &memory[placed.at as usize..(placed.at + placed.bytes) as usize];
                        let what = format!("{pairs} pairs, slot {slot}, {layout:?}: `{}`", p.name);
                        if !placed.streamed {
                            let f = &packed.tensors[i];
                            assert_eq!((p.pad, p.row_bytes, p.tile_lines, p.image_row_bytes()), (0, f.row_bytes, f.tile_lines, f.row_bytes), "{what}");
                            continue;
                        }
                        let bb = placed.ty.block_bytes() as u64;
                        let sb = bb.next_multiple_of(slot);
                        let blocks = p.row_bytes / bb;
                        assert_eq!((p.block_bytes, p.pad, p.slot_bytes(), p.image_row_bytes()), (bb, sb - bb, sb, blocks * sb), "{what}");
                        let tiles = &image.tiles[p.first_tile..p.first_tile + p.tile_count];
                        for (k, t) in tiles.iter().enumerate() {
                            let held = t.end_row - t.first_row;
                            assert_eq!(t.lines, (held * blocks * sb + check).div_ceil(lb), "{what}: tile {k}");
                            for j in 0..held {
                                let r = t.first_row + j;
                                for b in 0..blocks {
                                    let at = t.line * lb + (b * held + j) * sb;
                                    assert_eq!(image.locate_block(i, r, b), (t.channel, at), "{what}: row {r} block {b}");
                                    let want = &kept[(r * p.row_bytes + b * bb) as usize..(r * p.row_bytes + (b + 1) * bb) as usize];
                                    let lay = &channels[t.channel as usize][at as usize..(at + sb) as usize];
                                    assert!(&lay[..bb as usize] == want, "{what}: row {r} block {b}");
                                    assert!(lay[bb as usize..].iter().all(|&x| x == 0), "{what}: row {r} block {b}'s pad");
                                }
                            }
                            let data = (held * blocks * sb) as usize;
                            let bytes = &channels[t.channel as usize][(t.line * lb) as usize..((t.line + t.lines) * lb) as usize];
                            assert!(bytes[data..bytes.len() - check as usize].iter().all(|&x| x == 0), "{what}: tile {k}'s padding");
                        }
                        for &ch in &p.channels {
                            let run = TileRun::of(&image, i, ch);
                            let mine: Vec<_> = descriptors(&image, ch).into_iter().filter(|d| d.tensor == i as u32).collect();
                            assert_eq!(run.descriptors(), mine, "{what}: channel {ch}'s tile run");
                        }
                    }
                    assert_eq!(image.first_bad_tile(&channels), None, "{pairs} pairs, slot {slot}, {layout:?}");
                }
            }
        }
    }
}

/// A stream entry takes an image only laid in the slots its set takes: a matrix laid in slots of 16 bytes refused by a
/// packed set and by one of 64-byte slots, a packed matrix refused by a slotted set, each by the slots' sizes, and each
/// taken by its own.
#[test]
fn a_stream_entry_refuses_a_matrix_laid_in_other_slots() {
    use logicaffeine_compile::spatial::decode_engine::numerics::Words;
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    use logicaffeine_compile::spatial::decode_engine::rtl::streaming::StreamEntry;
    let mut table = TensorTable::from_rows(0, vec![("w".to_string(), vec![512, 40], GgmlType::Q6K.id())]).unwrap();
    table.tensors[0].streamed = true;
    let striped = ImageLayout { stripe: Some(2), ..ImageLayout::d5005(Beat::Data64) };
    let packed = weight_image(&table, &[0], &striped).unwrap();
    let slotted = weight_image(&table, &[0], &ImageLayout { slot: 16, ..striped }).unwrap();
    let set = ColumnSet::new("s", &[GgmlType::Q6K], Words::One);
    let refused = |image: &Image, set: &ColumnSet| StreamEntry::of(image, 0, set).expect_err("another slot");
    assert!(refused(&slotted, &set).contains("slots of 224 bytes, the unit's of 210"), "{}", refused(&slotted, &set));
    assert!(refused(&slotted, &set.clone().with_slots(64)).contains("slots of 224 bytes, the unit's of 256"));
    assert!(refused(&packed, &set.clone().with_slots(16)).contains("slots of 210 bytes, the unit's of 224"));
    assert!(StreamEntry::of(&slotted, 0, &set.clone().with_slots(16)).is_ok());
    assert!(StreamEntry::of(&packed, 0, &set).is_ok());
}

/// The 27B laid as the card's slotted builds lay it (stripes of four pairs, Q8_0 in octets, slots of 16 bytes): every
/// streamed block but Q6_K's already a whole number of slots (Q4_K 144, Q5_K 176, Q8_0's octets 272), Q6_K's 210 padded to
/// 224, so the image holds the packed image's bytes and Q6_K's pads, which are under 2% of its streamed bytes.
#[test]
fn the_27b_in_slots_pads_only_q6_k() {
    let machine = machine_of(&header("qwen3.8-27b-q4_k_m")).unwrap();
    let striped = ImageLayout { stripe: Some(4), octets: true, ..ImageLayout::d5005(Beat::Data64) };
    let packed = image_of(&machine, &striped).unwrap();
    let slotted = image_of(&machine, &ImageLayout { slot: 16, ..striped }).unwrap();
    let (mut streamed, mut pads) = (0u64, 0u64);
    for (p, q) in slotted.tensors.iter().zip(&packed.tensors) {
        if !p.block_major {
            assert_eq!(p.pad, 0, "`{}`", p.name);
            continue;
        }
        let want = if p.ty == GgmlType::Q6K.name() { 14 } else { 0 };
        assert_eq!(p.pad, want, "`{}` in {}", p.name, p.ty);
        streamed += q.rows * q.row_bytes;
        pads += p.rows * (p.image_row_bytes() - p.row_bytes);
    }
    assert!(pads > 0 && (pads as f64) < 0.02 * streamed as f64, "{pads} bytes of pads over {streamed} streamed");
}

/// The four tiles of a stripe of the 27B's every streamed matrix are its four channels' shares of the stripe's rows,
/// each channel holding the same rows of every stripe but the last, at the stripe the card's columns take (ten pairs
/// a channel): a block step's four channels run in lockstep.
#[test]
fn a_stripes_four_tiles_are_its_four_channels_shares() {
    let machine = machine_of(&header("qwen3.8-27b-q4_k_m")).unwrap();
    let image = image_of(&machine, &ImageLayout { stripe: Some(10), ..ImageLayout::d5005(Beat::Data64) }).unwrap();
    let mut streamed = 0;
    for (i, p) in image.tensors.iter().enumerate() {
        if !machine.tensors[i].streamed {
            assert!(!p.block_major, "`{}`", p.name);
            continue;
        }
        streamed += 1;
        let tiles = &image.tiles[p.first_tile..p.first_tile + p.tile_count];
        for (s, stripe) in tiles.chunks(4).enumerate() {
            let chans: HashSet<u32> = stripe.iter().map(|t| t.channel).collect();
            assert_eq!(chans.len(), stripe.len(), "`{}` stripe {s}: one tile a channel", p.name);
            for (c, t) in stripe.iter().enumerate() {
                assert_eq!(t.first_row, (4 * s as u64 + c as u64) * 20, "`{}` stripe {s}", p.name);
                assert_eq!(t.channel, p.channels[c], "`{}` stripe {s}: the deal's turn", p.name);
                if (s + 1) * 4 < tiles.len() {
                    assert_eq!(t.end_row - t.first_row, 20, "`{}` stripe {s}", p.name);
                }
            }
        }
    }
    assert_eq!(streamed, machine.tensors.iter().filter(|p| p.streamed).count());
    assert!(streamed > 64 * 4, "every block's projections stream: {streamed}");
}

/// A channel's tiles of a tensor are one tile run: `TileRun::of` walked into descriptors is exactly the image stream's
/// descriptors of the tensor on that channel, for every tensor on every channel — striped and not, of the tiny
/// checkpoint's mixture on the card's layouts and small odd ones, and of the 27B laid in the card's stripes.
#[test]
fn a_channels_tiles_of_a_tensor_are_one_tile_run() {
    use logicaffeine_compile::spatial::decode_engine::rtl::stream::descriptors;
    use logicaffeine_compile::spatial::decode_engine::rtl::streaming::TileRun;
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = mixed_checkpoint(dir.path());
    let machine = Machine::new(spec, Gguf::open(&path).unwrap(), EngineNumerics::fastest(), 1, 8).unwrap();
    let mut images = Vec::new();
    for plain in layouts() {
        for stripe in [None, Some(1), Some(3)] {
            images.push(image_of(&machine.layout, &ImageLayout { stripe, ..plain }).unwrap());
        }
    }
    let big = machine_of(&header("qwen3.8-27b-q4_k_m")).unwrap();
    images.push(image_of(&big, &ImageLayout { stripe: Some(10), ..ImageLayout::d5005(Beat::Data64) }).unwrap());
    for image in &images {
        for ch in 0..image.layout.channels {
            let all = descriptors(image, ch);
            for (i, p) in image.tensors.iter().enumerate() {
                let want: Vec<_> = all.iter().filter(|d| d.tensor as usize == i).copied().collect();
                let run = TileRun::of(image, i, ch);
                assert_eq!(run.descriptors(), want, "{:?}: `{}` on channel {ch}", image.layout, p.name);
                assert_eq!(run.count as usize, want.len());
            }
        }
    }
}

/// A stripe of no pairs, and a streamed tensor stored tap-major, are refused.
#[test]
fn a_stripe_refuses_what_the_array_cannot_take() {
    use logicaffeine_compile::spatial::decode_engine::image::TensorEntry;
    let row = |name: &str, dims: &[u64], id: u32| (name.to_string(), dims.to_vec(), id);
    let mut table = TensorTable::from_rows(0, vec![row("a", &[64], 0), row("b", &[32, 4], 8)]).unwrap();
    table.tensors[1].streamed = true;
    let layout = ImageLayout { stripe: Some(1), ..ImageLayout::d5005(Beat::Data64) };
    assert!(weight_image(&table, &[0, 1], &layout).is_ok());
    assert_eq!(weight_image(&table, &[0, 1], &ImageLayout { stripe: Some(0), ..layout }), Err(ImageError::EmptyStripe));
    table.tensors.push(TensorEntry { name: "k".into(), ty: GgmlType::F32, dims: vec![8, 4], tap_major: Some(4), streamed: true });
    assert_eq!(weight_image(&table, &[0, 1, 2], &layout), Err(ImageError::StreamedKernel("k".into())));
    assert!(weight_image(&table, &[0, 1, 2], &ImageLayout { stripe: None, ..layout }).is_ok(), "no stripe, nothing striped");
}

// ── What it refuses ──────────────────────────────────────────────────────────────────────────────────

#[test]
fn an_image_refuses_what_it_cannot_lay() {
    let row = |name: &str, dims: &[u64], id: u32| (name.to_string(), dims.to_vec(), id);
    let table = TensorTable::from_rows(0, vec![row("a", &[64], 0), row("b", &[32, 4], 8)]).unwrap();
    let layout = ImageLayout::d5005(Beat::Data64);
    assert!(weight_image(&table, &[0, 1], &layout).is_ok());
    assert!(matches!(weight_image(&table, &[0], &layout), Err(ImageError::Order(_))), "a tensor left out");
    assert!(matches!(weight_image(&table, &[0, 1, 1], &layout), Err(ImageError::Order(_))), "a tensor twice");
    assert!(matches!(weight_image(&table, &[0, 2], &layout), Err(ImageError::Order(_))), "an index past the table");
    assert_eq!(
        weight_image(&table, &[0, 1], &ImageLayout { capacity_lines: 3, ..layout }),
        Err(ImageError::TooBig { channel: 0, lines: 4, capacity: 3 })
    );
    assert!(matches!(TensorTable::from_rows(0, vec![row("q", &[48, 2], 8)]), Err(ImageError::RaggedRow { .. })), "48 is no whole Q8_0 block");
    assert!(matches!(TensorTable::from_rows(0, vec![row("z", &[32, 0], 0)]), Err(ImageError::RaggedRow { .. })), "no rows");
    assert_eq!(TensorTable::from_rows(0, vec![row("u", &[32], 99)]), Err(ImageError::UnknownType { tensor: "u".into(), id: 99 }));
}

// ── The ledger ───────────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_committed_image_ledger_is_what_the_layout_computes() {
    let path = repo().join("benchmarks/silicon/d5005/weight_image.json");
    let committed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).expect("the ledger parses");
    let fresh = serde_json::to_value(image_ledger(&decode_dir()).expect("the ledger lays out")).expect("serializes");
    assert!(
        committed == fresh,
        "benchmarks/silicon/d5005/weight_image.json is stale: regenerate it with `cargo run -p logicaffeine-compile --example d5005_weight_image`"
    );
}

// ── The image on disk ────────────────────────────────────────────────────────────────────────────────

/// Streamed a tile at a time, every channel's bytes are what `fill` holds; a writer short, or a tile that does not
/// follow its channel's last, is refused.
#[test]
fn write_channels_streams_what_fill_holds() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = mixed_checkpoint(dir.path());
    let mut g = Gguf::open(&path).unwrap();
    let table = table_of(&spec, &g);
    for layout in layouts() {
        let image = weight_image(&table, &table.read_order(), &layout).unwrap();
        let held = image.fill(&mut g).unwrap();
        let mut streamed: Vec<Vec<u8>> = vec![Vec::new(); layout.channels as usize];
        let mut out: Vec<&mut dyn std::io::Write> = streamed.iter_mut().map(|v| v as &mut dyn std::io::Write).collect();
        image.write_channels(&mut g, &mut out).unwrap();
        assert_eq!(streamed, held, "{layout:?}");
        let mut short: Vec<Vec<u8>> = vec![Vec::new(); layout.channels as usize - 1];
        let mut out: Vec<&mut dyn std::io::Write> = short.iter_mut().map(|v| v as &mut dyn std::io::Write).collect();
        assert_eq!(image.write_channels(&mut g, &mut out).unwrap_err().kind(), std::io::ErrorKind::InvalidInput);
        let mut gapped = image.clone();
        let last = gapped.tiles.len() - 1;
        gapped.tiles[last].line += 1;
        let mut sink: Vec<Vec<u8>> = vec![Vec::new(); layout.channels as usize];
        let mut out: Vec<&mut dyn std::io::Write> = sink.iter_mut().map(|v| v as &mut dyn std::io::Write).collect();
        assert_eq!(gapped.write_channels(&mut g, &mut out).unwrap_err().kind(), std::io::ErrorKind::InvalidData, "a tile past its channel's last");
    }
}

/// The tile table carries the layout, every channel's lines and every tile's place, in the image's order.
#[test]
fn the_tile_table_names_every_tile() {
    for (name, beat, _, image) in images().into_iter().filter(|(n, ..)| *n == "qwen3.8-27b-q4_k_m") {
        let table = image.tile_table();
        assert_eq!(&table[..8], b"LOGOSIMG", "{name} {beat:?}");
        let words: Vec<u64> = table[8..].chunks(8).map(|w| u64::from_le_bytes(w.try_into().unwrap())).collect();
        let channels = image.channel_lines.len();
        assert_eq!(&words[..4], &[1, image.layout.line_bytes, u64::from(image.layout.check), channels as u64]);
        assert_eq!(&words[4..4 + channels], &image.channel_lines[..]);
        assert_eq!(words[4 + channels], image.tiles.len() as u64);
        let tiles = &words[5 + channels..];
        assert_eq!(tiles.len(), 3 * image.tiles.len());
        for (t, w) in image.tiles.iter().zip(tiles.chunks(3)) {
            assert_eq!(w, [u64::from(t.channel), t.line, t.lines]);
        }
    }
}

/// Where Qwen3.8-27B's checkpoints are on this machine, the image laid out from the file is the one the header ledger
/// lays out: the same tensors in the same formats and dims, so the ledger's rows are the card's.
#[test]
fn the_27b_checkpoint_lays_out_as_its_header() {
    let name = "checkpoint:qwen3.8-27b/Qwen3.8-27B-Q4_K_M.gguf";
    if !common::require_tool(name) {
        return;
    }
    let path = common::checkpoint("qwen3.8-27b/Qwen3.8-27B-Q4_K_M.gguf").unwrap();
    let g = Gguf::open(&path).unwrap();
    let from_file = TensorTable::of_gguf(&g).unwrap();
    let from_header = TensorTable::of_header(&header("qwen3.8-27b-q4_k_m")).unwrap();
    assert_eq!(from_file, from_header, "{}", path.display());
    let spec = ModelSpec::from_gguf(&g).unwrap();
    let numerics = EngineNumerics::fastest();
    let file_machine = MachineLayout::of(&spec, numerics, &lower(&spec, numerics, 1).unwrap().program, &g, 1, 1).unwrap();
    let header_machine = machine_of(&header("qwen3.8-27b-q4_k_m")).unwrap();
    assert_eq!(file_machine, header_machine);
    for beat in [Beat::Data64, Beat::Full72] {
        let layout = ImageLayout::d5005(beat);
        assert_eq!(image_of(&file_machine, &layout).unwrap(), image_of(&header_machine, &layout).unwrap(), "{beat:?}");
    }
}

/// The image is the machine's memory: every tensor's stored rows, read where `locate` puts them, are the bytes the
/// machine keeps for it — the checkpoint's rows, and a convolution kernel's taps each with every channel together —
/// on the tiny checkpoints in the card's layouts and in layouts that cut every tensor into many tiles.
#[test]
fn the_image_holds_the_machines_memory() {
    let dir = tempfile::tempdir().unwrap();
    let q8 = dir.path().join("qwen35-q8.gguf");
    tiny_models::write_checkpoint(&Tiny::Qwen35.wide(64), &q8);
    for (spec, path) in [(Tiny::Qwen35.wide(64), q8), mixed_checkpoint(dir.path())] {
        for numerics in [EngineNumerics::fastest(), EngineNumerics::precise()] {
            let machine = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), numerics, 1, 8).unwrap();
            assert!(machine.layout.tensors.iter().any(|p| p.taps.is_some()), "a tap-major kernel to hold");
            let memory = machine.memory();
            for layout in layouts() {
                let image = image_of(&machine.layout, &layout).unwrap();
                let channels = image.fill(&mut Gguf::open(&path).unwrap()).unwrap();
                for (i, p) in image.tensors.iter().enumerate() {
                    let placed = &machine.layout.tensors[i];
                    let mut held = Vec::new();
                    for r in 0..p.rows {
                        let (ch, at) = image.locate(i, r);
                        held.extend_from_slice(&channels[ch as usize][at as usize..(at + p.row_bytes) as usize]);
                    }
                    let kept = &memory[placed.at as usize..(placed.at + placed.bytes) as usize];
                    assert!(held == kept, "{layout:?}: `{}` is not the machine's", p.name);
                }
            }
        }
    }
}
