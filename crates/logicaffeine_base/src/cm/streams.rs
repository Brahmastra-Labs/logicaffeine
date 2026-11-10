//! Compressed streams inside the input, opened up for the byte model.
//!
//! A DEFLATE stream (a ZIP entry, a gzip member, a zlib stream in a PDF or a PNG) or a GIF's LZW image data is
//! noise to a context model. When a stream re-compresses to exactly its own bytes under settings the encoder can
//! name — zlib's level, memory level and window, or the level of gzip and InfoZIP's zip, which [`super::deflate`]
//! reproduces bit for bit, or the LZW table size at which a GIF encoder clears ([`super::lzw`]) — the input is
//! stored with the stream replaced by
//! its decoded bytes and those settings, and the decoder re-compresses them in place. A stream that does not
//! reproduce is left as it is.
//!
//! A PNG's image data is one zlib stream cut into `IDAT` chunks, each framed by its length and a CRC-32 of its
//! type and payload; a GIF image's is an LZW code stream cut into length-prefixed sub-blocks. When every chunk
//! but the last carries the same length (and every CRC is right), the pieces are stitched into their stream and
//! opened the same way, and the decoder frames the re-compressed stream again.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/streams/tests.rs"]
mod tests;

use super::deflate::{deflate, gzip, inflate, Params};
use super::lzw;
use crate::describe::{read_uvarint, write_uvarint};

/// A stream shorter than this (decoded) is not worth its header entry.
const MIN_RAW: usize = 64;
/// The opened-up input may be at most this many times the original (plus a little): the decoder refuses a
/// stream that declares more, so the encoder never opens one past it.
pub const EXPANSION_LIMIT: usize = 16;
const EXPANSION_SLACK: usize = 1 << 16;

/// One reproduced stream: where its bytes sit in the original, how long they are, how long the decoded bytes
/// are, and how to compress and frame them back.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Stream {
    pub offset: usize,
    pub compressed: usize,
    pub raw: usize,
    pub codec: Codec,
}

/// How a stream was compressed and how its bytes sit in the original.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Codec {
    /// DEFLATE bytes as they are.
    Deflate(Params),
    /// DEFLATE bytes as gzip and InfoZIP's zip write them, at `level`.
    Gzip { level: u8 },
    /// PNG `IDAT` chunks of `chunk` payload bytes each (the last one shorter or equal), holding a zlib stream with
    /// header `zlib`; its scanlines are `stride` bytes (a filter byte and the pixels, `0` when interlaced) of
    /// `bpp`-byte pixels.
    Png { params: Params, chunk: u32, zlib: [u8; 2], stride: u32, bpp: u8 },
    /// A GIF image `width` pixels wide: its LZW code stream of `min_code` bits per pixel, cleared whenever the
    /// table reaches `clear_at` entries and followed by `tail` zero bytes (encoders that flush one more byte after
    /// the end code write 1), in sub-blocks of `block` bytes (the last one shorter or equal) and a zero terminator.
    /// Its matches are as long as the table allows but at `cuts`, the pixels where an encoder whose dictionary lost
    /// a string ended one early ([`lzw::encode_cut`]). With `table` set its decoded indices are renumbered by the luma
    /// of the colour table `table` bytes before its image data, `colours` entries long: each index becomes its
    /// colour's place in luma order (`77 R + 150 G + 29 B`, ties by index).
    Gif { min_code: u8, clear_at: u16, block: u8, width: u16, tail: u8, cuts: Vec<u32>, table: u32, colours: u16 },
}

impl Codec {
    /// Whether the decoded bytes are palette indices (a GIF's): values that name colours rather than measure them.
    pub fn indexed(&self) -> bool {
        matches!(self, Codec::Gif { .. })
    }

    /// The decoded bytes' rows, as (bytes per row, bytes per pixel, bytes before the pixels in each row), when
    /// they are an image's: a PNG scanline starts with its filter byte, a GIF row is all pixels.
    pub fn geometry(&self) -> Option<(usize, usize, usize)> {
        match *self {
            Codec::Deflate(_) | Codec::Gzip { .. } => None,
            Codec::Png { stride, bpp, .. } => (stride > 1).then_some((stride as usize, usize::from(bpp), 1)),
            Codec::Gif { width, .. } => (width > 1).then_some((usize::from(width), 1, 0)),
        }
    }
}

/// A PNG's scanline length and bytes per pixel from its `IHDR` (the chunk right after the signature at `at`), or
/// `(0, 1)` for an interlaced or unreadable one.
fn png_rows(data: &[u8], at: usize) -> (u32, u8) {
    let Some(h) = data.get(at + 8..at + 33).filter(|h| &h[4..8] == b"IHDR") else {
        return (0, 1);
    };
    let width = u64::from(u32::from_be_bytes([h[8], h[9], h[10], h[11]]));
    let (depth, color, interlace) = (u64::from(h[16]), h[17], h[20]);
    let channels = match color {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return (0, 1),
    };
    let row = (width * channels * depth).div_ceil(8) + 1;
    if interlace != 0 || row > u64::from(u32::MAX) {
        return (0, 1);
    }
    (row as u32, ((channels * depth / 8).max(1)) as u8)
}

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

pub(super) fn crc32(bytes: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        std::array::from_fn(|n| (0..8).fold(n as u32, |c, _| if c & 1 == 1 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 }))
    });
    !bytes.iter().fold(!0u32, |c, &b| table[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8))
}

pub(super) fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in bytes.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= 65_521;
        b %= 65_521;
    }
    b << 16 | a
}

/// The `IDAT` run of the PNG whose signature is at `at`: where its first chunk starts, where its last ends, the
/// common payload length, and the stitched payloads. `None` unless the chunks parse, are consecutive, all but the
/// last share one length, and every CRC is right.
fn png_idat(data: &[u8], at: usize) -> Option<(usize, usize, u32, Vec<u8>)> {
    let mut p = at + 8;
    let (mut first, mut stitched) = (None, Vec::new());
    let mut lengths = Vec::new();
    loop {
        let len = u32::from_be_bytes(data.get(p..p + 4)?.try_into().ok()?);
        let kind = data.get(p + 4..p + 8)?;
        let body = data.get(p + 8..p + 8 + len as usize)?;
        let crc = u32::from_be_bytes(data.get(p + 8 + len as usize..p + 12 + len as usize)?.try_into().ok()?);
        if kind == b"IDAT" {
            if crc32(&data[p + 4..p + 8 + len as usize]) != crc {
                return None;
            }
            first.get_or_insert(p);
            lengths.push(len);
            stitched.extend_from_slice(body);
        } else if first.is_some() {
            let chunk = lengths[0];
            let uniform = lengths[..lengths.len() - 1].iter().all(|&l| l == chunk) && *lengths.last()? <= chunk && chunk > 0;
            return uniform.then_some((first?, p, chunk, stitched));
        } else if kind == b"IEND" {
            return None;
        }
        p += 12 + len as usize;
    }
}

/// The DEFLATE stream of a stitched zlib stream, if its header is a plain zlib header and the stream, its
/// Adler-32 and nothing else fill it exactly.
fn png_stream(stitched: &[u8], room: usize) -> Option<([u8; 2], super::deflate::Inflated)> {
    let zlib = [*stitched.first()?, *stitched.get(1)?];
    if zlib[0] & 0x0F != 8 || (u16::from(zlib[0]) << 8 | u16::from(zlib[1])) % 31 != 0 || zlib[1] & 0x20 != 0 {
        return None;
    }
    let inflated = inflate(&stitched[2..], room)?;
    let trailer = stitched.get(2 + inflated.consumed..)?;
    (trailer.len() == 4 && u32::from_be_bytes(trailer.try_into().ok()?) == adler32(&inflated.data)).then_some((zlib, inflated))
}

const GIF_SIGNATURES: [&[u8; 6]; 2] = [b"GIF87a", b"GIF89a"];

/// The images of the GIF whose signature is at `at`, as (where the image data's sub-blocks start, where its
/// terminator ends, the minimum code size, the joined code stream, the sub-block length, the width, where its colour
/// table starts and its entries — the image's own, or else the global one). An image whose
/// sub-blocks do not all but the last share one length is skipped; the walk stops at the trailer or at anything
/// that does not parse.
fn gif_images(data: &[u8], at: usize) -> Vec<(usize, usize, u8, Vec<u8>, u8, u16, Option<(usize, usize)>)> {
    let mut images = Vec::new();
    let Some(&flags) = data.get(at + 10) else {
        return images;
    };
    let mut p = at + 13;
    let global = (flags & 0x80 != 0).then(|| (p, 2usize << (flags & 7)));
    if flags & 0x80 != 0 {
        p += 3 << ((flags & 7) + 1);
    }
    loop {
        match data.get(p) {
            Some(0x21) => {
                p += 2;
                loop {
                    let Some(&n) = data.get(p) else {
                        return images;
                    };
                    p += 1 + usize::from(n);
                    if n == 0 {
                        break;
                    }
                }
            }
            Some(0x2C) => {
                let Some(&local) = data.get(p + 9) else {
                    return images;
                };
                let width = u16::from_le_bytes([data[p + 5], data[p + 6]]);
                p += 10;
                let palette = if local & 0x80 != 0 { Some((p, 2usize << (local & 7))) } else { global };
                if local & 0x80 != 0 {
                    p += 3 << ((local & 7) + 1);
                }
                let Some(&min_code) = data.get(p) else {
                    return images;
                };
                p += 1;
                let start = p;
                let (mut stitched, mut sizes) = (Vec::new(), Vec::new());
                loop {
                    let Some(&n) = data.get(p) else {
                        return images;
                    };
                    p += 1;
                    if n == 0 {
                        break;
                    }
                    let Some(piece) = data.get(p..p + usize::from(n)) else {
                        return images;
                    };
                    stitched.extend_from_slice(piece);
                    sizes.push(n);
                    p += usize::from(n);
                }
                if let Some((&last, body)) = sizes.split_last() {
                    let block = body.first().copied().unwrap_or(last);
                    if body.iter().all(|&s| s == block) && last <= block {
                        images.push((start, p, min_code, stitched, block, width, palette));
                    }
                }
            }
            _ => return images,
        }
    }
}

/// The table sizes at which GIF encoders clear, most common first.
const GIF_CLEARS: [u16; 4] = [4096, 4095, 4094, 4093];

/// A GIF image opens with cuts only while it has at most one for every this many of its bytes: each costs the
/// header two bytes or so.
const GIF_CUT_SPACING: usize = 16;

/// The most the opened-up form of `len` original bytes may hold.
pub fn expansion_bound(len: usize) -> usize {
    len.saturating_mul(EXPANSION_LIMIT).saturating_add(EXPANSION_SLACK)
}

/// Where a DEFLATE stream may start: after a ZIP local header (method 8), a gzip header, or a zlib header.
fn candidates(data: &[u8]) -> Vec<usize> {
    let mut at = Vec::new();
    let n = data.len();
    for i in 0..n.saturating_sub(2) {
        match data[i] {
            b'P' if i + 30 <= n && &data[i..i + 4] == b"PK\x03\x04" && u16::from_le_bytes([data[i + 8], data[i + 9]]) == 8 => {
                let name = usize::from(u16::from_le_bytes([data[i + 26], data[i + 27]]));
                let extra = usize::from(u16::from_le_bytes([data[i + 28], data[i + 29]]));
                at.push(i + 30 + name + extra);
            }
            0x1F if i + 10 <= n && data[i + 1] == 0x8B && data[i + 2] == 8 => {
                let flags = data[i + 3];
                let mut p = i + 10;
                if flags & 4 != 0 && p + 2 <= n {
                    p += 2 + usize::from(u16::from_le_bytes([data[p], data[p + 1]]));
                }
                for bit in [8u8, 16] {
                    if flags & bit != 0 {
                        while p < n && data[p] != 0 {
                            p += 1;
                        }
                        p += 1;
                    }
                }
                if flags & 2 != 0 {
                    p += 2;
                }
                at.push(p);
            }
            cmf if cmf & 0x0F == 8 && cmf >> 4 <= 7 => {
                let flg = data[i + 1];
                if (u16::from(cmf) << 8 | u16::from(flg)) % 31 == 0 && flg & 0x20 == 0 {
                    at.push(i + 2);
                }
            }
            _ => {}
        }
    }
    at.retain(|&p| p < n);
    at.sort_unstable();
    at.dedup();
    at
}

/// The zlib parameters to try, most common first: level 6 at the default memory level, then every level at
/// memory levels 8 and 9, then the rest; each at the full window, then smaller ones.
fn parameter_order() -> Vec<Params> {
    let mut order = Vec::new();
    for window_bits in [15u8, 14, 13, 12, 11, 10, 9] {
        for mem_level in [8u8, 9, 7, 6, 5, 4, 3, 2, 1] {
            for level in [6u8, 9, 5, 4, 7, 8, 1, 2, 3] {
                order.push(Params { level, mem_level, window_bits });
            }
        }
    }
    order
}

/// The parameters (out of the first `tries` of [`parameter_order`]) under which `raw` deflates to exactly
/// `compressed`.
fn reproduce(raw: &[u8], compressed: &[u8], tries: usize) -> Option<Params> {
    parameter_order().into_iter().take(tries).find(|&p| deflate(raw, p) == compressed)
}

/// gzip's levels, most common first.
const GZIP_LEVELS: [u8; 9] = [6, 9, 5, 4, 7, 8, 1, 2, 3];

/// How many parameter sets the encoder tries per stream: every level at memory levels 8 and 9 with the full
/// window — what zip, gzip, Java and most libraries write.
const TRIES: usize = 18;

/// Every stream in `data` that reproduces, in order, none overlapping, within the expansion bound.
pub fn find(data: &[u8]) -> Vec<Stream> {
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Start {
        Deflate,
        Png,
        Gif,
    }
    let bound = expansion_bound(data.len());
    let mut found = Vec::new();
    let mut size = data.len();
    let mut next_free = 0;
    let mut starts: Vec<(usize, Start)> = candidates(data).into_iter().map(|c| (c, Start::Deflate)).collect();
    for i in 0..data.len().saturating_sub(8) {
        if data[i] == 0x89 && &data[i..i + 8] == PNG_SIGNATURE {
            starts.push((i, Start::Png));
        } else if data[i] == b'G' && GIF_SIGNATURES.iter().any(|s| &data[i..i + 6] == *s) {
            starts.push((i, Start::Gif));
        }
    }
    starts.sort_unstable();
    for (start, kind) in starts {
        if start < next_free {
            continue;
        }
        let room = bound.saturating_sub(size).min(1 << 30);
        match kind {
            Start::Png => {
                let Some((first, end, chunk, stitched)) = png_idat(data, start) else {
                    continue;
                };
                let Some((zlib, inflated)) = png_stream(&stitched, room) else {
                    continue;
                };
                if inflated.data.len() < MIN_RAW {
                    continue;
                }
                // The zlib header names the window; PNG writers shrink it for small images.
                let window_bits = (zlib[0] >> 4) + 8;
                let tries = parameter_order().into_iter().filter(|p| p.window_bits == window_bits).take(TRIES);
                let (stride, bpp) = png_rows(data, start);
                let mut tries = tries.map(|params| Codec::Png { params, chunk, zlib, stride, bpp });
                if let Some(codec) = tries.find(|c| frame(&inflated.data, c) == data[first..end]) {
                    size = size - (end - first) + inflated.data.len();
                    found.push(Stream { offset: first, compressed: end - first, raw: inflated.data.len(), codec });
                    next_free = end;
                }
            }
            Start::Gif => {
                for (first, end, min_code, stitched, block, width, palette) in gif_images(data, start) {
                    let room = bound.saturating_sub(size).min(1 << 30);
                    let Some(pixels) = lzw::decode(&stitched, min_code, room) else {
                        continue;
                    };
                    if pixels.len() < MIN_RAW {
                        continue;
                    }
                    let original = &data[first..end];
                    let settings = || [0u8, 1].into_iter().flat_map(|tail| GIF_CLEARS.iter().map(move |&clear_at| (tail, clear_at)));
                    let plain = settings()
                        .map(|(tail, clear_at)| Codec::Gif { min_code, clear_at, block, width, tail, cuts: Vec::new(), table: 0, colours: 0 })
                        .find(|c| frame(&pixels, c) == original);
                    // An encoder whose dictionary lost strings ended some matches early: the image opens with the
                    // pixels it cut at, when they are few.
                    let codec = plain.or_else(|| {
                        settings().find_map(|(tail, clear_at)| {
                            let (_, cuts) = lzw::cuts(&stitched, min_code, clear_at, pixels.len())?;
                            if cuts.is_empty() || cuts.len() > original.len() / GIF_CUT_SPACING {
                                return None;
                            }
                            let codec = Codec::Gif { min_code, clear_at, block, width, tail, cuts, table: 0, colours: 0 };
                            (frame(&pixels, &codec) == original).then_some(codec)
                        })
                    });
                    if let Some(mut codec) = codec {
                        // Indices renumbered by luma where the table is out of that order.
                        if let (Codec::Gif { table, colours, .. }, Some((at, n))) = (&mut codec, palette) {
                            let rank = luma_rank(&data[at..at + 3 * n], n);
                            if rank.iter().enumerate().any(|(i, &r)| usize::from(r) != i) {
                                *table = (first - at) as u32;
                                *colours = n as u16;
                            }
                        }
                        size = size - (end - first) + pixels.len();
                        found.push(Stream { offset: first, compressed: end - first, raw: pixels.len(), codec });
                        next_free = end;
                    }
                }
            }
            Start::Deflate => {
                let Some(inflated) = inflate(&data[start..], room) else {
                    continue;
                };
                if inflated.data.len() < MIN_RAW || inflated.consumed < 2 {
                    continue;
                }
                let compressed = &data[start..start + inflated.consumed];
                let codec = reproduce(&inflated.data, compressed, TRIES)
                    .map(Codec::Deflate)
                    .or_else(|| GZIP_LEVELS.iter().find(|&&level| gzip(&inflated.data, level) == compressed).map(|&level| Codec::Gzip { level }));
                if let Some(codec) = codec {
                    size = size - inflated.consumed + inflated.data.len();
                    found.push(Stream { offset: start, compressed: inflated.consumed, raw: inflated.data.len(), codec });
                    next_free = start + inflated.consumed;
                }
            }
        }
    }
    found
}

/// Cut `stream` into pieces of `size` bytes (the last shorter or equal) with `frame` around each.
fn pieces(stream: &[u8], size: usize, mut frame: impl FnMut(&[u8], &mut Vec<u8>)) -> Vec<u8> {
    let mut out = Vec::with_capacity(stream.len() + stream.len() / size.max(1) * 12 + 12);
    for piece in stream.chunks(size.max(1)) {
        frame(piece, &mut out);
    }
    out
}

/// A stream's bytes as the original holds them: its DEFLATE bytes; for a PNG the zlib stream cut into `IDAT`
/// chunks with their lengths and CRCs; for a GIF image the LZW code stream in sub-blocks and a terminator.
fn frame(raw: &[u8], codec: &Codec) -> Vec<u8> {
    match *codec {
        Codec::Deflate(params) => deflate(raw, params),
        Codec::Gzip { level } => gzip(raw, level),
        Codec::Png { params, chunk, zlib, .. } => {
            let mut stream = zlib.to_vec();
            stream.extend_from_slice(&deflate(raw, params));
            stream.extend_from_slice(&adler32(raw).to_be_bytes());
            pieces(&stream, chunk as usize, |piece, out| {
                out.extend_from_slice(&(piece.len() as u32).to_be_bytes());
                let start = out.len();
                out.extend_from_slice(b"IDAT");
                out.extend_from_slice(piece);
                let crc = crc32(&out[start..]);
                out.extend_from_slice(&crc.to_be_bytes());
            })
        }
        Codec::Gif { min_code, clear_at, block, tail, ref cuts, .. } => {
            let mut code = lzw::encode_cut(raw, min_code, clear_at, cuts);
            code.resize(code.len() + usize::from(tail), 0);
            let mut out = pieces(&code, usize::from(block), |piece, out| {
                out.push(piece.len() as u8);
                out.extend_from_slice(piece);
            });
            out.push(0);
            out
        }
    }
}

/// Each palette index's place among a colour table's `colours` entries ordered by luma (`77 R + 150 G + 29 B`, ties by
/// index); an index past the table keeps its own, so the renumbering is a permutation of every byte.
fn luma_rank(table: &[u8], colours: usize) -> [u8; 256] {
    let luma = |i: usize| 77 * u32::from(table[3 * i]) + 150 * u32::from(table[3 * i + 1]) + 29 * u32::from(table[3 * i + 2]);
    let mut order: Vec<usize> = (0..colours).collect();
    order.sort_by_key(|&i| (luma(i), i));
    let mut rank: [u8; 256] = std::array::from_fn(|i| i as u8);
    for (r, &i) in order.iter().enumerate() {
        rank[i] = r as u8;
    }
    rank
}

/// The decoded bytes of a found stream.
fn decoded(data: &[u8], s: &Stream) -> Vec<u8> {
    let region = &data[s.offset..s.offset + s.compressed];
    match s.codec {
        Codec::Deflate(_) | Codec::Gzip { .. } => inflate(region, s.raw).expect("a found stream inflates").data,
        Codec::Png { .. } => {
            let stitched = stitch_png(region).expect("a found PNG run parses");
            png_stream(&stitched, s.raw).expect("a found PNG stream inflates").1.data
        }
        Codec::Gif { min_code, table, colours, .. } => {
            let stitched = stitch_gif(region).expect("a found GIF image parses");
            let mut pixels = lzw::decode(&stitched, min_code, s.raw).expect("a found GIF image decodes");
            if table > 0 {
                let at = s.offset - table as usize;
                let rank = luma_rank(&data[at..at + 3 * usize::from(colours)], usize::from(colours));
                for p in &mut pixels {
                    *p = rank[usize::from(*p)];
                }
            }
            pixels
        }
    }
}

/// The payloads of a run of `IDAT` chunks that fills `region`, joined.
fn stitch_png(region: &[u8]) -> Option<Vec<u8>> {
    let mut p = 0;
    let mut stitched = Vec::new();
    while p < region.len() {
        let len = u32::from_be_bytes(region.get(p..p + 4)?.try_into().ok()?) as usize;
        stitched.extend_from_slice(region.get(p + 8..p + 8 + len)?);
        p += 12 + len;
    }
    Some(stitched)
}

/// The sub-blocks of a GIF image's data that fills `region` (its terminator last), joined.
fn stitch_gif(region: &[u8]) -> Option<Vec<u8>> {
    let mut p = 0;
    let mut stitched = Vec::new();
    loop {
        let n = usize::from(*region.get(p)?);
        if n == 0 {
            return (p + 1 == region.len()).then_some(stitched);
        }
        stitched.extend_from_slice(region.get(p + 1..p + 1 + n)?);
        p += 1 + n;
    }
}

/// The input with each stream's bytes replaced by its decoded ones.
pub fn open(data: &[u8], streams: &[Stream]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut at = 0;
    for s in streams {
        out.extend_from_slice(&data[at..s.offset]);
        out.extend_from_slice(&decoded(data, s));
        at = s.offset + s.compressed;
    }
    out.extend_from_slice(&data[at..]);
    out
}

/// Undo [`open`]: compress and frame each stream's decoded bytes back in place. `None` when a stream's bytes do
/// not fit the opened-up input or do not come back to the declared length.
pub fn close(opened: &[u8], streams: &[Stream]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(opened.len());
    let mut at = 0;
    let mut original = 0;
    for s in streams {
        let gap = s.offset.checked_sub(original)?;
        out.extend_from_slice(opened.get(at..at + gap)?);
        at += gap;
        let raw = opened.get(at..at + s.raw)?;
        let compressed = match s.codec {
            // The indices back from their luma ranks, by the table already rebuilt in front of the image.
            Codec::Gif { table, colours, .. } if table > 0 => {
                let start = out.len().checked_sub(table as usize)?;
                let rank = luma_rank(out.get(start..start + 3 * usize::from(colours))?, usize::from(colours));
                let mut index = [0u8; 256];
                for (i, &r) in rank.iter().enumerate() {
                    index[usize::from(r)] = i as u8;
                }
                frame(&raw.iter().map(|&p| index[usize::from(p)]).collect::<Vec<u8>>(), &s.codec)
            }
            _ => frame(raw, &s.codec),
        };
        if compressed.len() != s.compressed {
            return None;
        }
        out.extend_from_slice(&compressed);
        at += s.raw;
        original = s.offset + s.compressed;
    }
    out.extend_from_slice(opened.get(at..)?);
    Some(out)
}

fn write_params(p: Params, out: &mut Vec<u8>) {
    out.push(p.level | p.mem_level << 4);
    out.push(p.window_bits);
}

fn read_params(buf: &[u8], pos: &mut usize) -> Option<Params> {
    let lm = *buf.get(*pos)?;
    let window_bits = *buf.get(*pos + 1)?;
    *pos += 2;
    let p = Params { level: lm & 15, mem_level: lm >> 4, window_bits };
    ((1..=9).contains(&p.level) && (1..=9).contains(&p.mem_level) && (9..=15).contains(&window_bits)).then_some(p)
}

/// Header: the count, then per stream the gap since the previous one ended, the compressed and decoded lengths, and the
/// codec: a tag (0 DEFLATE, 1 PNG, 2 GIF, 3 gzip's DEFLATE, 4 a GIF with cuts) and its settings — zlib's level and
/// memory level in one byte and the window bits in another; for a PNG also the chunk length, the zlib header, the
/// scanline length and the bytes per pixel; for gzip's DEFLATE the level; for a GIF the minimum code size, the table
/// size it clears at, the sub-block length, the width, the zero bytes after the end code and the pixels its encoder cut
/// a match at, with tag 4 (their count, then each one's distance from the last).
pub fn write_header(streams: &[Stream], out: &mut Vec<u8>) {
    write_uvarint(streams.len() as u64, out);
    let mut end = 0;
    for s in streams {
        write_uvarint((s.offset - end) as u64, out);
        write_uvarint(s.compressed as u64, out);
        write_uvarint(s.raw as u64, out);
        match s.codec {
            Codec::Deflate(params) => {
                out.push(0);
                write_params(params, out);
            }
            Codec::Png { params, chunk, zlib, stride, bpp } => {
                out.push(1);
                write_params(params, out);
                write_uvarint(u64::from(chunk), out);
                out.extend_from_slice(&zlib);
                write_uvarint(u64::from(stride), out);
                out.push(bpp);
            }
            Codec::Gzip { level } => {
                out.push(3);
                out.push(level);
            }
            Codec::Gif { min_code, clear_at, block, width, tail, ref cuts, table, colours } => {
                out.push(if cuts.is_empty() { 2 } else { 4 });
                out.push(min_code);
                write_uvarint(u64::from(clear_at), out);
                out.push(block);
                write_uvarint(u64::from(width), out);
                out.push(tail);
                write_uvarint(u64::from(table), out);
                if table > 0 {
                    out.push(colours.trailing_zeros() as u8 - 1);
                }
                if !cuts.is_empty() {
                    write_uvarint(cuts.len() as u64, out);
                    let mut last = 0;
                    for &cut in cuts {
                        write_uvarint(u64::from(cut - last), out);
                        last = cut;
                    }
                }
            }
        }
        end = s.offset + s.compressed;
    }
}

/// Read a [`write_header`] header for an original of `len` bytes: `None` when it is malformed, a stream runs
/// past the original, a setting is out of range, or the opened-up size passes [`expansion_bound`].
pub fn read_header(buf: &[u8], pos: &mut usize, len: usize) -> Option<(Vec<Stream>, usize)> {
    let count = usize::try_from(read_uvarint(buf, pos)?).ok()?;
    if count > len / 2 + 1 {
        return None;
    }
    let mut streams = Vec::with_capacity(count);
    let mut end = 0usize;
    let mut opened = len;
    for _ in 0..count {
        let gap = usize::try_from(read_uvarint(buf, pos)?).ok()?;
        let compressed = usize::try_from(read_uvarint(buf, pos)?).ok()?;
        let raw = usize::try_from(read_uvarint(buf, pos)?).ok()?;
        let tag = *buf.get(*pos)?;
        *pos += 1;
        let codec = match tag {
            0 => Codec::Deflate(read_params(buf, pos)?),
            1 => {
                let params = read_params(buf, pos)?;
                let chunk = u32::try_from(read_uvarint(buf, pos)?).ok().filter(|&c| c > 0)?;
                let zlib = [*buf.get(*pos)?, *buf.get(*pos + 1)?];
                *pos += 2;
                let stride = u32::try_from(read_uvarint(buf, pos)?).ok()?;
                let bpp = *buf.get(*pos)?;
                *pos += 1;
                if !(1..=8).contains(&bpp) {
                    return None;
                }
                Codec::Png { params, chunk, zlib, stride, bpp }
            }
            2 | 4 => {
                let min_code = *buf.get(*pos)?;
                *pos += 1;
                let clear_at = u16::try_from(read_uvarint(buf, pos)?).ok()?;
                let block = *buf.get(*pos)?;
                *pos += 1;
                let width = u16::try_from(read_uvarint(buf, pos)?).ok()?;
                let tail = *buf.get(*pos)?;
                *pos += 1;
                // The colour table the indices are renumbered by: how far back it starts, then its entries as a power of two.
                let table = u32::try_from(read_uvarint(buf, pos)?).ok()?;
                let colours = if table > 0 {
                    let k = *buf.get(*pos)?;
                    *pos += 1;
                    if k > 7 {
                        return None;
                    }
                    2u16 << k
                } else {
                    0
                };
                let valid = (2..=8).contains(&min_code)
                    && ((1u16 << min_code) + 2..=4096).contains(&clear_at)
                    && block > 0
                    && tail <= 1
                    && (table == 0 || table as usize >= 3 * usize::from(colours));
                if !valid {
                    return None;
                }
                // With tag 4 the cuts: a count, then each one's distance from the last, ascending inside the pixels.
                let n = if tag == 4 { usize::try_from(read_uvarint(buf, pos)?).ok().filter(|&n| n > 0)? } else { 0 };
                if n > raw {
                    return None;
                }
                let mut cuts = Vec::with_capacity(n);
                let mut last = 0u64;
                for _ in 0..n {
                    let cut = last.checked_add(read_uvarint(buf, pos)?)?;
                    if cut <= last || cut >= raw as u64 {
                        return None;
                    }
                    cuts.push(u32::try_from(cut).ok()?);
                    last = cut;
                }
                Codec::Gif { min_code, clear_at, block, width, tail, cuts, table, colours }
            }
            3 => {
                let level = *buf.get(*pos)?;
                *pos += 1;
                if !(1..=9).contains(&level) {
                    return None;
                }
                Codec::Gzip { level }
            }
            _ => return None,
        };
        let offset = end.checked_add(gap)?;
        if matches!(codec, Codec::Gif { table, .. } if table as usize > offset) {
            return None;
        }
        end = offset.checked_add(compressed)?;
        if end > len || compressed == 0 {
            return None;
        }
        opened = opened.checked_sub(compressed)?.checked_add(raw)?;
        if opened > expansion_bound(len) {
            return None;
        }
        streams.push(Stream { offset, compressed, raw, codec });
    }
    Some((streams, opened))
}
