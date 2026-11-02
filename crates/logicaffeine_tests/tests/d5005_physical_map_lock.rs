//! The D5005's memory as its four channels hold it: a machine layout's weights in the weight image's tiles, everything
//! after them dealt round the channels in granules of the engine plan's state chunk.
//!
//! `spatial::decode_engine::physical` places every byte of a `decode_engine::machine::Layout` on the channels and names
//! the runs a job streams of one sequence's state. These tests hold the placement to one place a byte past the weights
//! and on the channels' lines, the granules to the channels in turn, a run of bytes to one stretch on each channel it
//! touches, and the patterns — a conv ring's row, a value head's memory, a log's entries, an attention block's cache
//! rows and scales — to the machine's own memory: after real steps, every byte the machine holds scattered to the
//! channels reads back where the map says, through `locate` and through every pattern's runs, at both beats and at the
//! knob points whose states and caches differ. The hybrid engine's memory — the streamed matrices in their striped image,
//! everything else compacted past it — holds the machine's the same way, its tables place every address by the
//! compaction, and the 27B's fits the card.

#![cfg(feature = "codegen")]

mod common;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::path::{Path, PathBuf};

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::spec::{Mixer, ModelSpec};
use logicaffeine_compile::spatial::d5005::{Beat, EnginePlan};
use logicaffeine_compile::spatial::decode_engine::image::{ImageError, ImageLayout};
use logicaffeine_compile::spatial::decode_engine::machine::{BlockState, Machine, LINE};
use logicaffeine_compile::spatial::decode_engine::model::{CacheFormat, EngineNumerics, StateFormat};
use logicaffeine_compile::spatial::decode_engine::physical::{PhysicalMap, Run};
use tiny_models::Tiny;

const SEQUENCES: usize = 3;
const CTX: usize = 16;

/// The tiny qwen35 with its attention grouped as the 27B's is, four query heads over two cached heads, so a position's
/// cache rows are several heads'.
fn checkpoint(dir: &Path) -> (ModelSpec, PathBuf) {
    let mut spec = Tiny::Qwen35.wide(CTX);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.heads = 4;
            a.kv_heads = 2;
        }
    }
    let path = dir.join("qwen35.gguf");
    tiny_models::write_checkpoint(&spec, &path);
    (spec, path)
}

/// The knob points whose memories differ in shape: an f16 state with a log and an int8 cache with its scales, and an
/// f32 state rewritten every pass with an f16 cache.
fn points() -> [EngineNumerics; 2] {
    [EngineNumerics::fastest(), EngineNumerics::precise()]
}

/// The machine after a few steps of every sequence, so every region holds what a real run leaves in it.
fn stepped(spec: &ModelSpec, path: &Path, numerics: EngineNumerics) -> Machine {
    let mut m = Machine::new(spec.clone(), Gguf::open(path).unwrap(), numerics, SEQUENCES, CTX).unwrap();
    let seqs: Vec<usize> = (0..SEQUENCES).map(|_| m.sequence().unwrap()).collect();
    m.step(&seqs.iter().map(|&s| (s, vec![3 + s as u32, 17, 41])).collect::<Vec<_>>()).unwrap();
    m.step(&seqs.iter().map(|&s| (s, vec![8 + s as u32])).collect::<Vec<_>>()).unwrap();
    m
}

/// The card's two layouts, at a granule small enough that the tiny machine's space spans many granules.
fn layouts() -> Vec<(Beat, u64)> {
    vec![(Beat::Data64, 8), (Beat::Full72, 8), (Beat::Data64, EnginePlan::standard().state_chunk_lines), (Beat::Full72, 3)]
}

fn map_of(m: &Machine, beat: Beat, granule: u64) -> PhysicalMap {
    PhysicalMap::of(&m.layout, &ImageLayout::d5005(beat), granule).unwrap()
}

/// Logical bytes `[at, at + bytes)` read back from `channels` a line at a time through `locate`.
fn read_back(map: &PhysicalMap, channels: &[Vec<u8>], at: u64, bytes: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes as usize);
    let mut from = at;
    while from < at + bytes {
        let to = (at + bytes).min((from / LINE + 1) * LINE);
        let (c, byte) = map.locate(from);
        out.extend_from_slice(&channels[c as usize][byte as usize..(byte + to - from) as usize]);
        from = to;
    }
    out
}

/// Every run is the channel's bytes of the logical pieces that fall to it, in logical order, and the runs together
/// hold exactly the range.
fn runs_hold(map: &PhysicalMap, channels: &[Vec<u8>], memory: &[u8], runs: &[Run], at: u64, bytes: u64, what: &str) {
    assert_eq!(runs.iter().map(|r| r.bytes).sum::<u64>(), bytes, "{what}: the runs hold the range");
    let mut per: Vec<Vec<u8>> = vec![Vec::new(); channels.len()];
    let mut from = at;
    while from < at + bytes {
        let to = (at + bytes).min((from / LINE + 1) * LINE);
        let (c, _) = map.locate(from);
        per[c as usize].extend_from_slice(&memory[from as usize..to as usize]);
        from = to;
    }
    let mut seen = vec![false; channels.len()];
    for r in runs {
        assert!(!seen[r.channel as usize], "{what}: one run a channel");
        seen[r.channel as usize] = true;
        assert_eq!(&channels[r.channel as usize][r.at as usize..(r.at + r.bytes) as usize], &per[r.channel as usize][..], "{what}: channel {}", r.channel);
    }
    assert!(per.iter().enumerate().all(|(c, p)| p.is_empty() == !seen[c]), "{what}: a run on every channel the range touches");
}

/// Every logical line past the weights has its own bytes on its channel, past the weight image and within the
/// channel's lines.
#[test]
fn every_byte_after_the_weights_has_one_place() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = checkpoint(dir.path());
    for numerics in points() {
        let m = Machine::new(spec.clone(), Gguf::open(&path).unwrap(), numerics, SEQUENCES, CTX).unwrap();
        for (beat, granule) in layouts() {
            let map = map_of(&m, beat, granule);
            let lb = map.image.layout.line_bytes;
            assert_eq!(map.base, *map.image.channel_lines.iter().max().unwrap());
            let mut taken: Vec<Vec<(u64, u64)>> = vec![Vec::new(); map.channel_lines.len()];
            for x in 0..map.lines {
                let (c, byte) = map.locate(map.tensors_end + x * LINE);
                assert!(byte >= map.base * lb, "{beat:?}: line {x} below the weight image's end");
                assert!(byte + LINE <= map.channel_lines[c as usize] * lb, "{beat:?}: line {x} past channel {c}'s lines");
                taken[c as usize].push((byte, byte + LINE));
            }
            for (c, t) in taken.iter_mut().enumerate() {
                t.sort();
                assert!(t.windows(2).all(|w| w[0].1 <= w[1].0), "{beat:?} granule {granule}: channel {c} holds two lines in one place");
            }
        }
    }
}

/// Granule `g` goes to channel `g mod 4`, a channel's granules at consecutive local lines; at 64 data bytes a beat a
/// local line is a channel line, at all 72 nine logical lines take eight of the channel's.
#[test]
fn granules_take_the_channels_in_turn() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = checkpoint(dir.path());
    let m = Machine::new(spec, Gguf::open(&path).unwrap(), EngineNumerics::fastest(), SEQUENCES, CTX).unwrap();
    for (beat, granule) in layouts() {
        let map = map_of(&m, beat, granule);
        let lb = map.image.layout.line_bytes;
        for g in 0..map.lines / granule {
            let (c, byte) = map.locate(map.tensors_end + g * granule * LINE);
            assert_eq!(c as u64, g % 4, "{beat:?}: granule {g}");
            assert_eq!(byte, map.base * lb + g / 4 * granule * LINE, "{beat:?}: granule {g}");
        }
        let last = map.channel_lines.iter().max().unwrap() - map.base;
        let granules = map.lines.div_ceil(granule);
        assert_eq!(last, (granules.div_ceil(4) * granule * LINE).div_ceil(lb), "{beat:?}: the channels hold no more than their granules");
    }
}

/// The machine's whole memory after real steps, scattered to the channels, reads back where the map says: every
/// tensor's rows where the weight image puts them, every byte after the weights through `locate`, every region and
/// every pattern — ring rows, value heads' memories, log entries, cache rows and scales — through its runs, each run
/// one stretch of its channel.
#[test]
fn the_channels_hold_the_machines_memory() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = checkpoint(dir.path());
    for numerics in points() {
        let m = stepped(&spec, &path, numerics);
        let memory = m.memory();
        for (beat, granule) in layouts() {
            let map = map_of(&m, beat, granule);
            let weights = map.image.fill(&mut Gguf::open(&path).unwrap()).unwrap();
            let channels = map.channel_bytes(memory, &weights);
            let last = m.layout.tensors.last().unwrap();
            assert_eq!(map.tensors_end, (last.at + last.bytes).div_ceil(LINE) * LINE, "{beat:?}: the map's space starts where the tensors end");
            for (i, t) in m.layout.tensors.iter().enumerate() {
                let p = &map.image.tensors[i];
                assert_eq!(p.rows * p.row_bytes, t.bytes, "{beat:?}: {} is stored whole", t.name);
                for row in 0..p.rows {
                    let (c, byte) = map.image.locate(i, row);
                    let at = (t.at + row * p.row_bytes) as usize;
                    assert_eq!(
                        &channels[c as usize][byte as usize..(byte + p.row_bytes) as usize],
                        &memory[at..at + p.row_bytes as usize],
                        "{beat:?}: {} row {row}",
                        t.name
                    );
                }
            }
            let after = memory.len() as u64 - map.tensors_end;
            assert_eq!(read_back(&map, &channels, map.tensors_end, after), &memory[map.tensors_end as usize..], "{beat:?}");
            let what = |s: &str| format!("{beat:?} granule {granule} {numerics:?}: {s}");
            for s in 0..SEQUENCES {
                let region = m.layout.region_of(s);
                runs_hold(&map, &channels, memory, &map.runs(region, m.layout.region), region, m.layout.region, &what("a region"));
                let element = if numerics.state == StateFormat::F16 { 2 } else { 4 };
                for (layer, block) in m.layout.blocks.iter().enumerate() {
                    match *block {
                        BlockState::Delta { ring, channels: width_c, taps, memory: mem, log, entry, v_heads, width, .. } => {
                            let rows = taps.saturating_sub(1).max(1);
                            let row = width_c as u64 * element;
                            for p in 0..CTX {
                                let at = region + ring + (p % rows) as u64 * row;
                                runs_hold(&map, &channels, memory, &map.ring_row(&m.layout, s, layer, p), at, row, &what("a ring row"));
                            }
                            let head = (width * width) as u64 * element;
                            for h in 0..v_heads {
                                let at = region + mem + h as u64 * head;
                                runs_hold(&map, &channels, memory, &map.memory(&m.layout, s, layer, h), at, head, &what("a head's memory"));
                            }
                            if let Some(log) = log {
                                let runs = map.log(&m.layout, s, layer, 0..2);
                                let count = map.runs(region + log, LINE).len();
                                runs_hold(&map, &channels, memory, &runs[..count], region + log, LINE, &what("a log's count"));
                                runs_hold(&map, &channels, memory, &runs[count..], region + log + LINE, 2 * entry, &what("a log's entries"));
                            }
                        }
                        BlockState::Attention { keys, values, key_scales, value_scales, kv_heads, width, value_width } => {
                            let element = if numerics.cache == CacheFormat::F16 { 2 } else { 1 };
                            for p in [0, 5, CTX - 1] {
                                let positions = (p + 1) as u64 * kv_heads as u64;
                                let mut spans = vec![(keys, positions * width as u64 * element), (values, positions * value_width as u64 * element)];
                                spans.extend([key_scales, value_scales].into_iter().flatten().map(|at| (at, positions * 2)));
                                let runs = map.cache(&m.layout, s, layer, p);
                                let mut used = 0;
                                for (at, bytes) in spans {
                                    let n = map.runs(region + at, bytes).len();
                                    runs_hold(&map, &channels, memory, &runs[used..used + n], region + at, bytes, &what("cache rows"));
                                    used += n;
                                }
                                assert_eq!(used, runs.len(), "{}", what("the cache's runs are its rows' and scales'"));
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A run of logical bytes, anywhere and of any length, is one stretch on each channel it touches.
#[test]
fn a_run_is_one_stretch_on_each_channel() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = checkpoint(dir.path());
    let m = stepped(&spec, &path, EngineNumerics::fastest());
    let memory = m.memory();
    let mut s = 0x9E37_79B9_7F4A_7C15u64;
    for (beat, granule) in layouts() {
        let map = map_of(&m, beat, granule);
        let channels = map.channel_bytes(memory, &map.image.fill(&mut Gguf::open(&path).unwrap()).unwrap());
        let span = memory.len() as u64 - map.tensors_end;
        for _ in 0..200 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let at = map.tensors_end + (s >> 20) % span;
            let bytes = 1 + (s >> 4) % (span - (at - map.tensors_end)).min(16 * granule * LINE);
            runs_hold(&map, &channels, memory, &map.runs(at, bytes), at, bytes, &format!("{beat:?} {at}+{bytes}"));
        }
    }
}

/// A machine whose memory the channels cannot hold is refused, naming the channel that overflows, though its weights
/// fit; at exactly the lines it needs it is placed.
#[test]
fn the_map_refuses_a_machine_past_the_channels() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = checkpoint(dir.path());
    let m = Machine::new(spec, Gguf::open(&path).unwrap(), EngineNumerics::fastest(), SEQUENCES, CTX).unwrap();
    for beat in [Beat::Data64, Beat::Full72] {
        let fits = map_of(&m, beat, 8);
        let need = *fits.channel_lines.iter().max().unwrap();
        let fullest = fits.channel_lines.iter().position(|&l| l == need).unwrap() as u32;
        assert!(fits.base < need, "{beat:?}: the state lies past the weights");
        let tight = ImageLayout { capacity_lines: need - 1, ..ImageLayout::d5005(beat) };
        assert_eq!(
            PhysicalMap::of(&m.layout, &tight, 8),
            Err(ImageError::TooBig { channel: fullest, lines: need, capacity: need - 1 }),
            "{beat:?}"
        );
        let exact = ImageLayout { capacity_lines: need, ..ImageLayout::d5005(beat) };
        assert_eq!(PhysicalMap::of(&m.layout, &exact, 8).unwrap().channel_lines, fits.channel_lines, "{beat:?}");
    }
}

// ── The hybrid engine's memory ───────────────────────────────────────────────────────────────────────

/// The hybrid engine's memory holds the machine's after real steps: every streamed matrix in its image, each block of
/// each row where `locate_block` finds it, in stripes of one and two pairs a channel; every other byte — the other
/// tensors, the rotary tables, every sequence's region — at its compacted place in the flat space past the image, read
/// back through `locate`. The compaction takes out exactly the streamed matrices, each to its next line, and the flat
/// space starts past the image's every line.
#[test]
fn the_hybrid_map_holds_the_machines_memory_with_its_matrices_streamed() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = checkpoint(dir.path());
    for numerics in points() {
        let m = stepped(&spec, &path, numerics);
        let memory = m.memory();
        let want: Vec<usize> = (0..m.layout.tensors.len()).filter(|&i| m.layout.tensors[i].streamed).collect();
        assert!(!want.is_empty(), "matrices to stream");
        for (beat, granule) in layouts() {
            for pairs in [1u64, 2] {
                let layout = ImageLayout { stripe: Some(pairs), ..ImageLayout::d5005(beat) };
                let (map, streamed, compaction) = PhysicalMap::hybrid(&m.layout, &layout, granule).unwrap();
                let what = format!("{numerics:?} {beat:?} granule {granule}, {pairs} pairs");
                assert_eq!(streamed, want, "{what}: the image's tensors are the streamed ones, in order");
                for (k, &i) in streamed.iter().enumerate() {
                    let t = &m.layout.tensors[i];
                    assert_eq!(compaction.runs[k], (t.at, (t.at + t.bytes).div_ceil(LINE) * LINE), "{what}: `{}`'s run", t.name);
                }
                let compacted = compaction.bytes(memory);
                assert_eq!(compacted.len() as u64, m.layout.bytes - compaction.removed(), "{what}");
                assert!(map.base >= map.image.channel_lines.iter().copied().max().unwrap_or(0), "{what}: the flat space past the image");
                let image_channels = map.image.fill(&mut Gguf::open(&path).unwrap()).unwrap();
                let channels = map.channel_bytes(&compacted, &image_channels);
                for (k, &i) in streamed.iter().enumerate() {
                    let t = &m.layout.tensors[i];
                    let (bb, rb) = (t.ty.block_bytes() as u64, t.row_bytes());
                    for r in 0..map.image.tensors[k].rows {
                        for b in 0..rb / bb {
                            let (c, at) = map.image.locate_block(k, r, b);
                            let from = (t.at + r * rb + b * bb) as usize;
                            assert!(channels[c as usize][at as usize..(at + bb) as usize] == memory[from..from + bb as usize], "{what}: `{}` row {r} block {b}", t.name);
                        }
                    }
                }
                let mut kept = 0u64;
                for x in (0..m.layout.bytes).step_by(LINE as usize) {
                    let Some(at) = compaction.place(x) else { continue };
                    let bytes = LINE.min(m.layout.bytes - x);
                    assert_eq!(at, kept, "{what}: byte {x} kept in order");
                    assert!(read_back(&map, &channels, at, bytes) == memory[x as usize..(x + bytes) as usize], "{what}: byte {x} at {at}");
                    kept += LINE;
                }
                assert_eq!(kept, compacted.len() as u64, "{what}: every kept line placed");
            }
        }
    }
}

/// The hybrid engine's tables place every address the reference engine reads by the compaction — each sequence's
/// region, each rotary table, each tensor it reads by address at its compacted place, a streamed matrix's address 0 —
/// and are otherwise the tables' own.
#[test]
fn the_hybrid_engines_tables_place_every_address_by_the_compaction() {
    use logicaffeine_compile::spatial::decode_engine::machine::{Row, StoreLayout};
    use logicaffeine_compile::spatial::decode_engine::rtl::engine::{tables, tables_at};
    let dir = tempfile::tempdir().unwrap();
    let (spec, path) = checkpoint(dir.path());
    let m = stepped(&spec, &path, EngineNumerics::fastest());
    let (_, _, compaction) = PhysicalMap::hybrid(&m.layout, &ImageLayout { stripe: Some(1), ..ImageLayout::d5005(Beat::Data64) }, 8).unwrap();
    let rows: Vec<Row> = (0..SEQUENCES).map(|s| Row { sequence: s, position: 4, token: 9, ring: 1 }).collect();
    let store = StoreLayout::of(&m.jobs, &m.layout, rows.len());
    let plain = tables(&m.layout, &store, &m.jobs, &rows);
    let placed = tables_at(&m.layout, &store, &m.jobs, &rows, &|at| compaction.place(at));
    assert_eq!((&placed.jobs, &placed.slots, &placed.blocks), (&plain.jobs, &plain.slots, &plain.blocks));
    for (i, t) in m.layout.tensors.iter().enumerate() {
        let want = if t.streamed { 0 } else { compaction.place(t.at).unwrap() / LINE };
        assert_eq!(placed.tensors[i] & 0xffff_ffff, want as u128, "`{}`'s address", t.name);
        assert_eq!(placed.tensors[i] >> 32, plain.tensors[i] >> 32, "`{}`'s format and dims", t.name);
    }
    for (k, (_, at)) in m.layout.rotary.iter().enumerate() {
        assert_eq!(placed.ropes[k] & 0xffff_ffff, compaction.place(*at).unwrap() / LINE, "rotary table {k}");
        assert_eq!(placed.ropes[k] >> 32, plain.ropes[k] >> 32);
    }
    for (r, row) in rows.iter().enumerate() {
        assert_eq!(placed.rows[r] & 0xffff_ffff, (compaction.place(m.layout.region_of(row.sequence)).unwrap() / LINE) as u128, "row {r}'s region");
        assert_eq!(placed.rows[r] >> 32, plain.rows[r] >> 32);
    }
    assert!(m.layout.tensors.iter().any(|t| t.streamed) && m.layout.tensors.iter().any(|t| !t.streamed));
}

/// Every matrix the 27B streams has a stream entry for the card's hybrid build — one pair a channel, the set of every
/// format it streams at the fastest point's words — its fields within the ports the engine gives them, its format's
/// place the set's, its stripes covering its rows, and its four tile runs the channels' tiles of it in order; the
/// entries' lines hold the fields where the engine reads them.
#[test]
fn the_27b_streams_every_matrix_through_a_stream_entry() {
    use logicaffeine_compile::spatial::decode::GgufHeader;
    use logicaffeine_compile::spatial::decode_engine::image::machine_of;
    use logicaffeine_compile::spatial::decode_engine::rtl::column::ColumnSet;
    use logicaffeine_compile::spatial::decode_engine::rtl::streaming::{StreamEntry, TileRun};
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/decode/headers/qwen3.8-27b-q4_k_m.json");
    let machine = machine_of(&GgufHeader::read_ledger(&path).unwrap()).unwrap();
    let layout = ImageLayout { stripe: Some(1), ..ImageLayout::d5005(Beat::Data64) };
    let (map, streamed, _) = PhysicalMap::hybrid(&machine, &layout, EnginePlan::standard().state_chunk_lines).unwrap();
    let formats: Vec<_> = streamed.iter().map(|&i| machine.tensors[i].ty).collect();
    let set = ColumnSet::hybrid(&formats, EngineNumerics::fastest().activations);
    for (k, &i) in streamed.iter().enumerate() {
        let t = &machine.tensors[i];
        let e = StreamEntry::of(&map.image, k, &set).unwrap_or_else(|why| panic!("`{}`: {why}", t.name));
        assert_eq!(set.formats[e.fmt as usize], t.ty, "`{}`'s format", t.name);
        assert_eq!(e.blocks as u64 * t.ty.block_len() as u64, t.dims[0], "`{}`'s blocks", t.name);
        assert_eq!(e.rows as u64, t.dims[1], "`{}`'s rows", t.name);
        assert!(e.stripes as u64 * 8 >= e.rows as u64 && (e.stripes as u64 - 1) * 8 < e.rows as u64, "`{}`'s stripes", t.name);
        assert_eq!(e.tensor as usize, k);
        for c in 0..4u32 {
            assert_eq!(e.runs[c as usize], TileRun::of(&map.image, k, c), "`{}` channel {c}", t.name);
            assert_eq!((e.turns >> (2 * c)) & 3, map.image.tensors[k].channels.iter().position(|&ch| ch == c).unwrap() as u32);
        }
        let lines = e.lines();
        let word = |i: usize| u32::from_le_bytes(lines[i / 16][4 * (i % 16)..4 * (i % 16) + 4].try_into().unwrap());
        assert_eq!([word(0), word(1), word(2), word(3), word(4), word(5)], [e.fmt, e.blocks, e.rows, e.stripes, e.turns, e.tensor]);
        for c in 0..4 {
            let r = &e.runs[c];
            let want = [r.count, r.line, r.lines, r.bytes, r.last_lines, r.last_bytes, r.row, r.row_step].map(|v| v as u32);
            assert_eq!((0..8).map(|w| word(16 + 8 * c + w)).collect::<Vec<_>>(), want.to_vec(), "`{}` channel {c}'s run", t.name);
        }
    }
    assert!(streamed.len() > 64 * 4 && set.formats.len() >= 4, "{} matrices over {:?}", streamed.len(), set.formats);
}

/// The 27B's hybrid memory — its streamed matrices in stripes of the card's ten pairs a channel, the rest compacted
/// past them — fits the card's four channels at both beats.
#[test]
fn the_27b_hybrid_memory_fits_the_card() {
    use logicaffeine_compile::spatial::decode::GgufHeader;
    use logicaffeine_compile::spatial::decode_engine::image::machine_of;
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/decode/headers/qwen3.8-27b-q4_k_m.json");
    let machine = machine_of(&GgufHeader::read_ledger(&path).unwrap()).unwrap();
    for beat in [Beat::Data64, Beat::Full72] {
        let layout = ImageLayout { stripe: Some(10), ..ImageLayout::d5005(beat) };
        let (map, streamed, compaction) = PhysicalMap::hybrid(&machine, &layout, EnginePlan::standard().state_chunk_lines).unwrap();
        assert!(streamed.len() > 64 * 4, "{beat:?}: {} streamed matrices", streamed.len());
        assert!(map.channel_lines.iter().all(|&l| l <= layout.capacity_lines), "{beat:?}");
        assert!(compaction.removed() > machine.bytes * 9 / 10, "{beat:?}: the matrices are most of the memory");
    }
}
