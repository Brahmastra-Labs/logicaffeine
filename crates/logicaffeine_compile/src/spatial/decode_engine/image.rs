//! The weight image: a checkpoint's tensors laid out over the card's memory channels in the order a pass reads
//! them, so each channel streams its share of every pass as one run of lines.
//!
//! A tensor is its rows, a row being its innermost dimension: a matrix's rows are its output rows, a vector is one
//! row. The image cuts each tensor into tiles of whole rows packed end to end, as many as fill [`TILE_LINES`] lines
//! (one row when a row alone is larger), and starts every tile on a line. A kernel stored tap-major has a tap's every
//! channel for a row; a tap larger than a tile is cut into the fewest equal runs of channels that fit one, each run a
//! row of the image, so the machine's own layout never makes a tile past the tile's lines. A tensor's tiles go to the channels in turn,
//! least loaded first: the channels sorted by the lines they hold before it, ties to the lower channel, and the tiles
//! dealt round that order. Every tile but a tensor's last holds the same rows, so a row's place is arithmetic
//! ([`Image::locate`]): the tensor's tile, the channel its turn falls on, and that channel's first line of the tensor
//! plus the tensor's earlier tiles there.
//!
//! The deal keeps the channels within one tile of each other after every tile, so when all four stream at one rate
//! each tile arrives within one tile's time of its turn, and the order on every channel is the read order. With a
//! check word ([`ImageLayout::check`]) every tile ends in the CRC-32C of its lines before it, little-endian, in the
//! last four bytes of its last line, the bytes between its rows and the check word zero: the integrity a memory
//! controller built without ECC leaves to the image when all 72 bits of a beat carry data.
//!
//! A layout with a stripe ([`ImageLayout::stripe`]) of `n` pairs deals every tensor a matvec job streams through the
//! array in tiles of `2n` rows instead, each tile block-major: every row's block 0, then every row's block 1, and so
//! on, so a channel's tile is the row pairs its columns hold, a block of each at a time, in the order the columns
//! take them. The tiles 4s..4s+3 of a striped tensor are the four channels' shares of its stripe `s`, which the array
//! runs a block step at a time; the default layout has no stripe.
//!
//! The order is the caller's. [`image_of`] lays out a machine layout's tensors (`decode_engine::machine::Layout`, in
//! the order the program first reads them) as the machine stores them, a convolution kernel tap-major, so the image
//! and the machine hold one tensor list; the card's images and the ledger are made so ([`machine_of`] reads the
//! layout off a header ledger). [`TensorTable::read_order`] orders a bare tensor table as the engine's step reads a
//! `qwen35` checkpoint, then every tensor the step does not read, in the table's order.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::codegen_sva::ddr4_jedec::Ddr4Grade;
use crate::model::gguf::Gguf;
use crate::model::quant::GgmlType;
use crate::model::reference::WeightSource;
use crate::spatial::d5005::{Beat, DDR_CHANNELS};
use crate::spatial::decode::GgufHeader;

use super::machine::Layout as MachineLayout;
use super::model::EngineNumerics;
use super::program::lower;

/// Lines a tile fills before the next channel's turn. Modeled: the skew one channel's tile buffer must cover.
pub const TILE_LINES: u64 = 512;
/// Bytes of a tile's check word.
pub const CHECK_BYTES: u64 = 4;

/// One tensor as the image stores it: its name, format and stored dims (innermost first); for a convolution kernel
/// stored tap-major, its taps: then each stored row is one tap's every channel, the transpose of the checkpoint's rows
/// of each channel's taps; and whether a matvec job streams it through the array, which a layout with a stripe lays in
/// stripes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TensorEntry {
    pub name: String,
    pub ty: GgmlType,
    pub dims: Vec<u64>,
    pub tap_major: Option<u64>,
    pub streamed: bool,
}

impl TensorEntry {
    pub fn rows(&self) -> u64 {
        self.dims.iter().skip(1).product()
    }

    pub fn row_bytes(&self) -> u64 {
        self.ty.bytes_for(self.dims[0] as usize) as u64
    }
}

/// Why a checkpoint has no image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    MissingMetadata(String),
    UnknownType { tensor: String, id: u32 },
    /// A row that is not a whole number of its format's blocks.
    RaggedRow { tensor: String, elems: u64, block: u64 },
    /// An order that does not name every tensor exactly once.
    Order(String),
    /// A channel the image overflows.
    TooBig { channel: u32, lines: u64, capacity: u64 },
    /// A tap-major kernel's tap that no cut into equal runs of channels fits a tile.
    RowPastTile { tensor: String, row_bytes: u64, room: u64 },
    /// A stripe of no row pairs.
    EmptyStripe,
    /// A tensor both streamed through the array and stored tap-major.
    StreamedKernel(String),
}

/// A checkpoint's tensor table and the blocks its step runs (the rest are drafting blocks).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TensorTable {
    pub blocks: u64,
    pub tensors: Vec<TensorEntry>,
}

/// A block's tensors in the order the engine's step reads them: the mixer's norm; a Gated-DeltaNet mixer's
/// projections, its convolution, decay and norm vectors, its output; an attention mixer's projections, its query and
/// key norms, its output; the FFN's norm and its gate, up and down projections.
const BLOCK_READS: &[&str] = &[
    "attn_norm.weight",
    "attn_qkv.weight",
    "attn_gate.weight",
    "ssm_beta.weight",
    "ssm_alpha.weight",
    "ssm_conv1d.weight",
    "ssm_dt.bias",
    "ssm_a",
    "ssm_norm.weight",
    "ssm_out.weight",
    "attn_q.weight",
    "attn_k.weight",
    "attn_v.weight",
    "attn_q_norm.weight",
    "attn_k_norm.weight",
    "attn_output.weight",
    "post_attention_norm.weight",
    "ffn_norm.weight",
    "ffn_gate.weight",
    "ffn_up.weight",
    "ffn_down.weight",
];

impl TensorTable {
    /// The table of `rows` (name, dims, GGUF type id), the first `blocks` blocks being the step's.
    pub fn from_rows(blocks: u64, rows: impl IntoIterator<Item = (String, Vec<u64>, u32)>) -> Result<TensorTable, ImageError> {
        let mut tensors = Vec::new();
        for (name, dims, id) in rows {
            let ty = GgmlType::from_id(id).map_err(|id| ImageError::UnknownType { tensor: name.clone(), id })?;
            let elems = dims.first().copied().unwrap_or(0);
            let block = ty.block_len() as u64;
            if elems % block != 0 || dims.contains(&0) || dims.is_empty() {
                return Err(ImageError::RaggedRow { tensor: name, elems, block });
            }
            tensors.push(TensorEntry { name, ty, dims, tap_major: None, streamed: false });
        }
        Ok(TensorTable { blocks, tensors })
    }

    /// The table of a checkpoint on disk.
    pub fn of_gguf(g: &Gguf) -> Result<TensorTable, ImageError> {
        let meta = |key: &str| g.arch_meta(key).and_then(|v| v.as_u64());
        let blocks = meta("block_count").ok_or_else(|| ImageError::MissingMetadata("block_count".into()))?;
        TensorTable::from_rows(blocks.saturating_sub(meta("nextn_predict_layers").unwrap_or(0)), g.tensor_names())
    }

    /// The table of a checkpoint's header ledger.
    pub fn of_header(h: &GgufHeader) -> Result<TensorTable, ImageError> {
        let blocks = h.meta_u64("block_count").ok_or_else(|| ImageError::MissingMetadata("block_count".into()))?;
        let rows = h.tensors.iter().map(|t| (t.name.clone(), t.dims.clone(), t.type_id));
        TensorTable::from_rows(blocks.saturating_sub(h.meta_u64("nextn_predict_layers").unwrap_or(0)), rows)
    }

    /// The order a pass reads the tensors: the embedding table, each of the step's blocks as `BLOCK_READS` lists
    /// it, the output norm and the head (a head tied to the embedding table reads the table already placed), then
    /// every tensor the step does not read, in the table's order.
    pub fn read_order(&self) -> Vec<usize> {
        let at: HashMap<&str, usize> = self.tensors.iter().enumerate().map(|(i, t)| (t.name.as_str(), i)).collect();
        let mut order = Vec::with_capacity(self.tensors.len());
        let push = |name: &str, order: &mut Vec<usize>| {
            if let Some(&i) = at.get(name) {
                order.push(i);
            }
        };
        push("token_embd.weight", &mut order);
        for b in 0..self.blocks {
            for key in BLOCK_READS {
                push(&format!("blk.{b}.{key}"), &mut order);
            }
        }
        push("output_norm.weight", &mut order);
        push("output.weight", &mut order);
        let mut placed = vec![false; self.tensors.len()];
        for &i in &order {
            placed[i] = true;
        }
        order.extend((0..self.tensors.len()).filter(|&i| !placed[i]));
        order
    }
}

/// How an image lays its tiles: over how many channels, in lines of how many bytes, how many lines a tile fills, how
/// many lines a channel holds, whether every tile carries a check word, the row pairs a channel's tile of a streamed
/// tensor holds when the image is laid in stripes for the streaming engine's array, and whether a streamed Q8_0
/// tensor's blocks lie eight to one of the array's blocks (its octets: a row's eight blocks one after another, 256
/// elements in 272 bytes), and the bytes a block of a tensor laid in stripes takes rounded up to (its slot: the
/// block's bytes, then zeros; 1 for blocks packed as the checkpoint packs them).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct ImageLayout {
    pub channels: u32,
    pub line_bytes: u64,
    pub tile_lines: u64,
    pub capacity_lines: u64,
    pub check: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stripe: Option<u64>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub octets: bool,
    #[serde(skip_serializing_if = "packed")]
    pub slot: u64,
}

fn packed(slot: &u64) -> bool {
    *slot == 1
}

impl ImageLayout {
    /// The D5005's four channels at `beat`: 64 data bytes a line, or all 72 with a check word on every tile.
    pub fn d5005(beat: Beat) -> ImageLayout {
        let ddr = Ddr4Grade::D5005_EMIF.at();
        let (line_bytes, check) = match beat {
            Beat::Data64 => (64, false),
            Beat::Full72 => (ddr.burst_bits() / 8, true),
        };
        ImageLayout { channels: DDR_CHANNELS, line_bytes, tile_lines: TILE_LINES, capacity_lines: ddr.lines(), check, stripe: None, octets: false, slot: 1 }
    }

    fn check_bytes(&self) -> u64 {
        if self.check {
            CHECK_BYTES
        } else {
            0
        }
    }

    /// Lines a tile of `bytes` row bytes takes, its check word included.
    pub fn lines_for(&self, bytes: u64) -> u64 {
        (bytes + self.check_bytes()).div_ceil(self.line_bytes)
    }
}

/// Where one tensor lies: its rows in the image and their size — each stored row cut into `pieces` equal runs, more
/// than one only for a tap-major kernel's tap past a tile — the rows every tile but its last holds and the lines such a
/// tile takes, the channels in the order its tiles are dealt, the line each of them holds its first tile of the tensor
/// at, its tiles' run in [`Image::tiles`], its taps when it is stored tap-major, the bytes of its format's block,
/// whether its tiles hold their rows block-major (laid in stripes), and the zeros each of a striped tensor's blocks is
/// padded with to the image's slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Placement {
    pub name: String,
    pub ty: &'static str,
    pub dims: Vec<u64>,
    pub rows: u64,
    pub row_bytes: u64,
    pub pieces: u64,
    pub rows_per_tile: u64,
    pub tile_lines: u64,
    pub channels: Vec<u32>,
    pub first_line: Vec<u64>,
    pub first_tile: usize,
    pub tile_count: usize,
    pub tap_major: Option<u64>,
    pub block_bytes: u64,
    pub block_major: bool,
    /// The zeros after each block of a tensor laid in stripes, to the image's slot.
    #[serde(skip_serializing_if = "unpadded")]
    pub pad: u64,
}

fn unpadded(pad: &u64) -> bool {
    *pad == 0
}

impl Placement {
    /// The bytes one of the tensor's blocks takes in the image: its block's, and its pad.
    pub fn slot_bytes(&self) -> u64 {
        self.block_bytes + self.pad
    }

    /// The bytes one of the tensor's rows takes in the image: its stored bytes, each block's pad beside it.
    pub fn image_row_bytes(&self) -> u64 {
        if self.pad == 0 {
            self.row_bytes
        } else {
            self.row_bytes / self.block_bytes * self.slot_bytes()
        }
    }
}

/// A run of one tensor's rows `[first_row, end_row)` on one channel from line `line`, over `lines` lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Tile {
    pub tensor: usize,
    pub first_row: u64,
    pub end_row: u64,
    pub channel: u32,
    pub line: u64,
    pub lines: u64,
}

/// A checkpoint laid out on the channels: its tensors in the order given, its tiles in that order, and the lines each
/// channel holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Image {
    pub layout: ImageLayout,
    pub tensors: Vec<Placement>,
    pub tiles: Vec<Tile>,
    pub channel_lines: Vec<u64>,
}

/// Lay `table`'s tensors out in `order` (each index once) under `layout`.
pub fn weight_image(table: &TensorTable, order: &[usize], layout: &ImageLayout) -> Result<Image, ImageError> {
    let mut seen = vec![false; table.tensors.len()];
    for &i in order {
        match seen.get_mut(i) {
            None => return Err(ImageError::Order(format!("index {i} of {} tensors", table.tensors.len()))),
            Some(true) => return Err(ImageError::Order(format!("`{}` twice", table.tensors[i].name))),
            Some(s) => *s = true,
        }
    }
    if let Some(i) = seen.iter().position(|s| !s) {
        return Err(ImageError::Order(format!("`{}` left out", table.tensors[i].name)));
    }
    if layout.stripe == Some(0) {
        return Err(ImageError::EmptyStripe);
    }
    let c = layout.channels as usize;
    let room = (layout.tile_lines * layout.line_bytes).saturating_sub(layout.check_bytes());
    let mut loads = vec![0u64; c];
    let mut tensors = Vec::with_capacity(order.len());
    let mut tiles = Vec::new();
    for &i in order {
        let e = &table.tensors[i];
        let stripe = layout.stripe.filter(|_| e.streamed);
        if stripe.is_some() && e.tap_major.is_some() {
            return Err(ImageError::StreamedKernel(e.name.clone()));
        }
        let pieces = pieces_of(e, room)?;
        let (rows, row_bytes) = (e.rows() * pieces, e.row_bytes() / pieces);
        if stripe.is_some() && layout.octets && e.ty == GgmlType::Q8_0 && e.dims[0] % 256 != 0 {
            return Err(ImageError::RaggedRow { tensor: e.name.clone(), elems: e.dims[0], block: 256 });
        }
        let block_bytes = e.ty.block_bytes() as u64 * if stripe.is_some() && layout.octets && e.ty == GgmlType::Q8_0 { 8 } else { 1 };
        let pad = if stripe.is_some() { block_bytes.next_multiple_of(layout.slot) - block_bytes } else { 0 };
        let image_row = row_bytes / block_bytes * (block_bytes + pad);
        let rows_per_tile = match stripe {
            Some(pairs) => 2 * pairs,
            None => room / row_bytes,
        }
        .max(1)
        .min(rows.max(1));
        let tile_lines = layout.lines_for(rows_per_tile * image_row);
        let mut channels: Vec<u32> = (0..layout.channels).collect();
        channels.sort_by_key(|&ch| (loads[ch as usize], ch));
        let first_line: Vec<u64> = channels.iter().map(|&ch| loads[ch as usize]).collect();
        let first_tile = tiles.len();
        let mut first_row = 0;
        let mut turn = 0;
        while first_row < rows {
            let end_row = (first_row + rows_per_tile).min(rows);
            let channel = channels[turn % c];
            let lines = layout.lines_for((end_row - first_row) * image_row);
            tiles.push(Tile { tensor: tensors.len(), first_row, end_row, channel, line: loads[channel as usize], lines });
            loads[channel as usize] += lines;
            first_row = end_row;
            turn += 1;
        }
        tensors.push(Placement {
            name: e.name.clone(),
            ty: e.ty.name(),
            dims: e.dims.clone(),
            rows,
            row_bytes,
            pieces,
            rows_per_tile,
            tile_lines,
            channels,
            first_line,
            first_tile,
            tile_count: tiles.len() - first_tile,
            tap_major: e.tap_major,
            block_bytes,
            block_major: stripe.is_some(),
            pad,
        });
    }
    for (ch, &lines) in loads.iter().enumerate() {
        if lines > layout.capacity_lines {
            return Err(ImageError::TooBig { channel: ch as u32, lines, capacity: layout.capacity_lines });
        }
    }
    Ok(Image { layout: *layout, tensors, tiles, channel_lines: loads })
}

/// The fewest equal runs of whole blocks a tap-major kernel's taps are cut into so that one fits `room` bytes; 1 for
/// a tap that fits, and for every row of any other tensor.
fn pieces_of(e: &TensorEntry, room: u64) -> Result<u64, ImageError> {
    let row_bytes = e.row_bytes();
    if e.tap_major.is_none() || row_bytes <= room {
        return Ok(1);
    }
    let blocks = e.dims[0] / e.ty.block_len() as u64;
    let block_bytes = row_bytes / blocks;
    (row_bytes.div_ceil(room)..=blocks)
        .find(|&p| blocks % p == 0 && blocks / p * block_bytes <= room)
        .ok_or(ImageError::RowPastTile { tensor: e.name.clone(), row_bytes, room })
}

/// The weight image the machine runs over: `machine`'s tensors in its order, each stored as the machine reads it — a
/// convolution kernel tap-major, its taps rows of every channel — laid out under `layout`.
pub fn image_of(machine: &MachineLayout, layout: &ImageLayout) -> Result<Image, ImageError> {
    let mut tensors = Vec::with_capacity(machine.tensors.len());
    for p in &machine.tensors {
        let entry = match p.taps {
            None => TensorEntry { name: p.name.clone(), ty: p.ty, dims: p.dims.clone(), tap_major: None, streamed: p.streamed },
            Some(taps) => {
                if p.dims.len() != 2 || p.dims[0] != taps as u64 {
                    return Err(ImageError::RaggedRow { tensor: p.name.clone(), elems: p.dims[0], block: taps as u64 });
                }
                TensorEntry { name: p.name.clone(), ty: p.ty, dims: vec![p.dims[1], taps as u64], tap_major: Some(taps as u64), streamed: p.streamed }
            }
        };
        tensors.push(entry);
    }
    let order: Vec<usize> = (0..tensors.len()).collect();
    weight_image(&TensorTable { blocks: 0, tensors }, &order, layout)
}

/// The weight image of the matrices `machine`'s matvec jobs stream, in its order and no other tensor, laid out under
/// `layout` (in stripes when it has one), with each image tensor's index in the machine's: what the hybrid engine's
/// streaming matvec reads beside the flat space the reference engine reads the rest of the memory from.
pub fn streamed_image_of(machine: &MachineLayout, layout: &ImageLayout) -> Result<(Image, Vec<usize>), ImageError> {
    let indices: Vec<usize> = (0..machine.tensors.len()).filter(|&i| machine.tensors[i].streamed).collect();
    let tensors = indices
        .iter()
        .map(|&i| {
            let p = &machine.tensors[i];
            TensorEntry { name: p.name.clone(), ty: p.ty, dims: p.dims.clone(), tap_major: None, streamed: true }
        })
        .collect();
    let order: Vec<usize> = (0..indices.len()).collect();
    Ok((weight_image(&TensorTable { blocks: 0, tensors }, &order, layout)?, indices))
}

/// A tensor table as a weight source that describes its tensors and holds none of their bytes: what a header ledger
/// gives a machine layout to place.
struct Described<'a>(&'a TensorTable);

impl WeightSource for Described<'_> {
    fn describe(&self, name: &str) -> std::io::Result<(GgmlType, Vec<u64>)> {
        self.0
            .tensors
            .iter()
            .find(|t| t.name == name)
            .map(|t| (t.ty, t.dims.clone()))
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("no tensor {name}")))
    }
    fn row_bytes(&mut self, name: &str, _: usize, _: usize) -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, format!("a header holds no bytes of {name}")))
    }
    fn has(&self, name: &str) -> bool {
        self.0.tensors.iter().any(|t| t.name == name)
    }
}

/// The machine layout of the checkpoint `h` describes, at the engine's fastest knob point, one sequence of one
/// position: what its weight image is laid out from.
pub fn machine_of(h: &GgufHeader) -> Result<MachineLayout, String> {
    let table = TensorTable::of_header(h).map_err(|e| format!("{e:?}"))?;
    let spec = h.spec().map_err(|e| format!("{e:?}"))?;
    let numerics = EngineNumerics::fastest();
    let program = lower(&spec, numerics, 1).map_err(|e| format!("{}: {e}", spec.arch))?.program;
    MachineLayout::of(&spec, numerics, &program, &Described(&table), 1, 1).map_err(|e| format!("{e:?}"))
}

impl Image {
    /// Bytes each channel holds.
    pub fn channel_bytes(&self) -> Vec<u64> {
        self.channel_lines.iter().map(|l| l * self.layout.line_bytes).collect()
    }

    /// The channel and the byte within it where row `row` of tensor `tensor` (an index into [`Image::tensors`])
    /// starts: for a tensor laid block-major, where its block 0 is.
    pub fn locate(&self, tensor: usize, row: u64) -> (u32, u64) {
        self.locate_block(tensor, row, 0)
    }

    /// The channel and the byte within it where block `block` of row `row` of tensor `tensor` starts, for a tensor of
    /// weight blocks (its row's blocks one after another, or, laid block-major, the tile's every row's block before
    /// any row's next).
    pub fn locate_block(&self, tensor: usize, row: u64, block: u64) -> (u32, u64) {
        let p = &self.tensors[tensor];
        let tile = row / p.rows_per_tile;
        let turn = (tile % p.channels.len() as u64) as usize;
        let line = p.first_line[turn] + tile / p.channels.len() as u64 * p.tile_lines;
        let (r, held) = (row % p.rows_per_tile, p.rows_per_tile.min(p.rows - tile * p.rows_per_tile));
        let within = if p.block_major { (block * held + r) * p.slot_bytes() } else { r * p.row_bytes + block * p.block_bytes };
        (p.channels[turn], line * self.layout.line_bytes + within)
    }

    /// The image's rows `[first, first + count)` of tensor `tensor`, read from `source`: the pieces of the stored rows
    /// they lie in.
    pub fn stored_rows(&self, tensor: usize, source: &mut dyn WeightSource, first: u64, count: u64) -> std::io::Result<Vec<u8>> {
        let p = &self.tensors[tensor];
        let (lo, hi) = (first / p.pieces, (first + count).div_ceil(p.pieces));
        let whole = self.whole_rows(tensor, source, lo, hi - lo)?;
        let skip = ((first - lo * p.pieces) * p.row_bytes) as usize;
        Ok(whole[skip..skip + (count * p.row_bytes) as usize].to_vec())
    }

    /// Stored rows `[first, first + count)` of tensor `tensor`, read from `source`: the checkpoint's rows, or for a
    /// tensor stored tap-major, each tap's every channel gathered from the checkpoint's rows of each channel's taps.
    fn whole_rows(&self, tensor: usize, source: &mut dyn WeightSource, first: u64, count: u64) -> std::io::Result<Vec<u8>> {
        let p = &self.tensors[tensor];
        let Some(taps) = p.tap_major else {
            return source.row_bytes(&p.name, first as usize, count as usize);
        };
        let channels = p.dims[0];
        let element = p.row_bytes * p.pieces / channels;
        let checkpoint = source.row_bytes(&p.name, 0, channels as usize)?;
        let mut out = Vec::with_capacity((count * p.row_bytes * p.pieces) as usize);
        for tap in first..first + count {
            for c in 0..channels {
                let at = ((c * taps + tap) * element) as usize;
                out.extend_from_slice(&checkpoint[at..at + element as usize]);
            }
        }
        Ok(out)
    }

    /// Tile `t`'s lines: its stored rows read from `source` (block-major in a stripe), zeros to its check word, the
    /// check word.
    fn tile_bytes(&self, t: &Tile, source: &mut dyn WeightSource) -> std::io::Result<Vec<u8>> {
        let held = t.end_row - t.first_row;
        let mut rows = self.stored_rows(t.tensor, source, t.first_row, held)?;
        let p = &self.tensors[t.tensor];
        if p.block_major {
            let (bb, pad, row_bytes) = (p.block_bytes as usize, p.pad as usize, p.row_bytes as usize);
            let mut out = Vec::with_capacity(rows.len() / bb * (bb + pad));
            for b in 0..row_bytes / bb {
                for r in 0..held as usize {
                    out.extend_from_slice(&rows[r * row_bytes + b * bb..r * row_bytes + (b + 1) * bb]);
                    out.resize(out.len() + pad, 0);
                }
            }
            rows = out;
        }
        let mut span = vec![0u8; (t.lines * self.layout.line_bytes) as usize];
        span[..rows.len()].copy_from_slice(&rows);
        if self.layout.check {
            let body = span.len() - CHECK_BYTES as usize;
            let word = crc32c(&span[..body]);
            span[body..].copy_from_slice(&word.to_le_bytes());
        }
        Ok(span)
    }

    /// The channels' bytes, every tile's rows read from `source` and its check word written.
    pub fn fill(&self, source: &mut dyn WeightSource) -> std::io::Result<Vec<Vec<u8>>> {
        let lb = self.layout.line_bytes;
        let mut channels: Vec<Vec<u8>> = self.channel_lines.iter().map(|&l| vec![0u8; (l * lb) as usize]).collect();
        for t in &self.tiles {
            let at = (t.line * lb) as usize;
            let span = self.tile_bytes(t, source)?;
            channels[t.channel as usize][at..at + span.len()].copy_from_slice(&span);
        }
        Ok(channels)
    }

    /// The bytes [`Image::fill`] gives each channel written to that channel's writer in `out`, a tile at a time, so no
    /// more than a tile is ever held: each channel takes its tiles in line order from line 0.
    pub fn write_channels(&self, source: &mut dyn WeightSource, out: &mut [&mut dyn std::io::Write]) -> std::io::Result<()> {
        if out.len() != self.channel_lines.len() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{} writers for {} channels", out.len(), self.channel_lines.len())));
        }
        let mut next = vec![0u64; out.len()];
        for t in &self.tiles {
            let ch = t.channel as usize;
            if t.line != next[ch] {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("channel {ch}'s tile at line {} after line {}", t.line, next[ch])));
            }
            out[ch].write_all(&self.tile_bytes(t, source)?)?;
            next[ch] += t.lines;
        }
        Ok(())
    }

    /// The tile table a host loads and scrubs the image by, little-endian 64-bit words after the eight bytes
    /// `LOGOSIMG`: the format (1), the line's bytes, whether tiles end in check words, the channels, each channel's
    /// lines, the tiles, and each tile's channel, first line and lines.
    pub fn tile_table(&self) -> Vec<u8> {
        let mut words = vec![1, self.layout.line_bytes, u64::from(self.layout.check), self.channel_lines.len() as u64];
        words.extend(&self.channel_lines);
        words.push(self.tiles.len() as u64);
        for t in &self.tiles {
            words.extend([u64::from(t.channel), t.line, t.lines]);
        }
        let mut out = b"LOGOSIMG".to_vec();
        for w in words {
            out.extend(w.to_le_bytes());
        }
        out
    }

    /// The first tile whose check word does not match its lines in `channels`, or `None` when every one does (or the
    /// layout carries none).
    pub fn first_bad_tile(&self, channels: &[Vec<u8>]) -> Option<usize> {
        if !self.layout.check {
            return None;
        }
        let lb = self.layout.line_bytes as usize;
        self.tiles.iter().position(|t| {
            let at = t.line as usize * lb;
            let span = &channels[t.channel as usize][at..at + t.lines as usize * lb];
            let body = span.len() - CHECK_BYTES as usize;
            crc32c(&span[..body]).to_le_bytes() != span[body..]
        })
    }
}

/// The Qwen3.8-27B checkpoints whose images the ledger lays out.
pub const HEADERS: &[&str] = &[
    "qwen3.8-27b-iq4_xs",
    "qwen3.8-27b-q3_k_m",
    "qwen3.8-27b-q4_0",
    "qwen3.8-27b-q4_k_m",
    "qwen3.8-27b-q5_k_m",
    "qwen3.8-27b-q6_k",
    "qwen3.8-27b-q8_0",
];

/// One checkpoint's image at one beat, summed: its tensors and tiles, the bytes of rows, of padding and of check
/// words, each channel's lines, the most two channels differ by and the largest tile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImageRow {
    pub header: &'static str,
    pub beat: Beat,
    pub layout: ImageLayout,
    pub tensors: usize,
    pub tiles: usize,
    pub data_bytes: u64,
    pub padding_bytes: u64,
    pub check_bytes: u64,
    pub channel_lines: Vec<u64>,
    pub spread_lines: u64,
    pub largest_tile_lines: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImageLedger {
    pub rows: Vec<ImageRow>,
}

impl Image {
    /// The image summed as a ledger row.
    pub fn row(&self, header: &'static str, beat: Beat) -> ImageRow {
        let data_bytes: u64 = self.tiles.iter().map(|t| (t.end_row - t.first_row) * self.tensors[t.tensor].row_bytes).sum();
        let check_bytes = if self.layout.check { CHECK_BYTES * self.tiles.len() as u64 } else { 0 };
        let total: u64 = self.channel_bytes().iter().sum();
        let (most, least) = (self.channel_lines.iter().max(), self.channel_lines.iter().min());
        ImageRow {
            header,
            beat,
            layout: self.layout,
            tensors: self.tensors.len(),
            tiles: self.tiles.len(),
            data_bytes,
            padding_bytes: total - data_bytes - check_bytes,
            check_bytes,
            channel_lines: self.channel_lines.clone(),
            spread_lines: most.zip(least).map_or(0, |(m, l)| m - l),
            largest_tile_lines: self.tiles.iter().map(|t| t.lines).max().unwrap_or(0),
        }
    }
}

/// Every [`HEADERS`] checkpoint's machine image ([`image_of`] over [`machine_of`]) on the D5005 at both beats.
pub fn image_ledger(decode_dir: &Path) -> Result<ImageLedger, String> {
    let mut rows = Vec::new();
    for &header in HEADERS {
        let h = GgufHeader::read_ledger(&decode_dir.join(format!("headers/{header}.json")))?;
        let machine = machine_of(&h).map_err(|e| format!("{header}: {e}"))?;
        for beat in [Beat::Data64, Beat::Full72] {
            let image = image_of(&machine, &ImageLayout::d5005(beat)).map_err(|e| format!("{header} {beat:?}: {e:?}"))?;
            rows.push(image.row(header, beat));
        }
    }
    Ok(ImageLedger { rows })
}

/// CRC-32C (Castagnoli: the reflected polynomial 0x82F63B78, initial value and final xor all ones), a byte at a time.
pub fn crc32c(bytes: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut t = [0u32; 256];
        let mut i = 0;
        while i < 256 {
            let mut c = i as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 != 0 { 0x82F6_3B78 ^ (c >> 1) } else { c >> 1 };
                k += 1;
            }
            t[i] = c;
            i += 1;
        }
        t
    };
    !bytes.iter().fold(!0u32, |c, &b| TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8))
}
