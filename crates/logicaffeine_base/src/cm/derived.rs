//! Container fields the bytes they describe determine, blanked for the model and computed again by the decoder: a
//! tar header's checksum; a ZIP entry's CRC-32 and sizes — in its local header or its data descriptor — and the
//! central directory's copies of them with the offset of the entry's local header; every PNG chunk's CRC-32 except
//! the `IDAT` chunks' (which opening the image data checks); the Adler-32 after a zlib stream and the CRC-32 and length
//! after a gzip member.
//!
//! The model would pay for every such field in full: a CRC-32 is 32 bits of noise, and a size or an offset nearly as
//! much. The encoder finds each container by its signature, computes the fields from the container's other bytes as
//! the decoder will, and blanks those that hold exactly that (zero bytes; a checksum's octal digits become `0`s, so
//! the field keeps its shape). It goes through the containers from the last to the first and the decoder from the
//! first to the last, so a container inside another is blanked after it and restored before it: each side computes
//! a container's fields from the same bytes. The encoder then restores what it blanked as the decoder will and keeps
//! the blanks only if that gives back the input exactly. A record of which containers were blanked travels in the
//! header: how many were found (the decoder finds the same ones — blanking never touches what finding reads — and
//! checks the count) and which were left as they were. A stream's trailer is named by one code per opened stream.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/derived/tests.rs"]
mod tests;

use super::deflate::inflate;
use super::streams::{adler32, crc32, expansion_bound, Codec, Stream};
use crate::describe::{read_uvarint, write_uvarint};

/// The containers of one kind found in some bytes, and which of them were left as they were.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    pub found: usize,
    /// Indices into the containers found, ascending.
    pub kept: Vec<usize>,
}

impl Marks {
    /// Whether any container was blanked.
    pub fn any(&self) -> bool {
        self.kept.len() < self.found
    }

    fn none(found: usize) -> Self {
        Marks { found, kept: (0..found).collect() }
    }
}

/// Every level's blanks: the ZIP archives and the PNG images of each level's bytes (the original first, then each
/// opened level's), the trailers of each level's opened streams, and the tar headers of the bytes the model codes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub archives: Vec<Marks>,
    pub images: Vec<Marks>,
    pub trailers: Vec<Vec<u8>>,
    pub tar: Marks,
}

impl Record {
    /// Whether anything was blanked.
    pub fn any(&self) -> bool {
        self.archives.iter().chain(&self.images).any(Marks::any) || self.trailers.iter().flatten().any(|&c| c != TRAILER_NONE) || self.tar.any()
    }
}

/// A field a container's other bytes determine: where it sits, what it holds, and what the model is shown instead.
struct Field {
    at: usize,
    value: Vec<u8>,
    blank: Vec<u8>,
}

impl Field {
    fn zeroed(at: usize, value: u32) -> Self {
        Field { at, value: value.to_le_bytes().to_vec(), blank: vec![0; 4] }
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Archive,
    Png,
    Tar,
}

impl Kind {
    fn candidates(self, data: &[u8]) -> Vec<usize> {
        match self {
            Kind::Archive => positions(data, b"PK\x05\x06").filter(|&e| e + 22 <= data.len()).collect(),
            Kind::Png => positions(data, PNG_SIGNATURE).collect(),
            Kind::Tar => positions(data, b"ustar").filter_map(|m| m.checked_sub(TAR_MAGIC)).filter(|&h| h + TAR_BLOCK <= data.len()).collect(),
        }
    }

    /// The fields of the container at `at` as its other bytes determine them, `None` when it does not parse or is of
    /// a shape this does not cover. Inflating a ZIP entry draws on `budget`.
    fn fields(self, data: &[u8], at: usize, budget: &mut usize) -> Option<Vec<Field>> {
        match self {
            Kind::Archive => archive_fields(data, at, budget),
            Kind::Png => png_fields(data, at),
            Kind::Tar => tar_field(data, at).map(|f| vec![f]),
        }
    }
}

fn positions<'a>(data: &'a [u8], signature: &'static [u8]) -> impl Iterator<Item = usize> + 'a {
    data.windows(signature.len()).enumerate().filter(move |(_, w)| *w == signature).map(|(i, _)| i)
}

/// Blank the fields of every container of `kind` in `data` that holds exactly what its other bytes determine.
fn blank(data: &mut [u8], kind: Kind) -> Marks {
    let found = kind.candidates(data);
    let mut budget = expansion_bound(data.len());
    let mut chosen = vec![false; found.len()];
    let mut blanked: Vec<Field> = Vec::new();
    for (i, &at) in found.iter().enumerate().rev() {
        let Some(fields) = kind.fields(data, at, &mut budget) else {
            continue;
        };
        if fields.is_empty() || !fields.iter().all(|f| data.get(f.at..f.at + f.value.len()) == Some(&f.value[..])) {
            continue;
        }
        for f in &fields {
            data[f.at..f.at + f.blank.len()].copy_from_slice(&f.blank);
        }
        blanked.extend(fields);
        chosen[i] = true;
    }
    let marks = Marks { found: found.len(), kept: (0..found.len()).filter(|&i| !chosen[i]).collect() };
    // Blanks are zeros and `0` digits, which spell no signature: blanking can only erase a container, which the
    // decoder then does not find — the trial restoring refuses that with the rest.
    if blanked.is_empty() || !restores_exactly(data, kind, &marks, &blanked) {
        for f in blanked.iter().rev() {
            data[f.at..f.at + f.value.len()].copy_from_slice(&f.value);
        }
        return Marks::none(found.len());
    }
    marks
}

/// Whether restoring `data` as the decoder will writes exactly the `blanked` fields, each back to its value; `data`
/// is left as it was.
fn restores_exactly(data: &mut [u8], kind: Kind, marks: &Marks, blanked: &[Field]) -> bool {
    let mut log = Vec::new();
    let restored = restore_logged(data, kind, marks, &mut log).is_some();
    let mut want: Vec<(usize, usize)> = blanked.iter().map(|f| (f.at, f.value.len())).collect();
    let mut wrote: Vec<(usize, usize)> = log.iter().map(|(at, before): &(usize, Vec<u8>)| (*at, before.len())).collect();
    want.sort_unstable();
    wrote.sort_unstable();
    let exact = restored && want == wrote && blanked.iter().all(|f| data[f.at..f.at + f.value.len()] == f.value[..]);
    for (at, before) in log.iter().rev() {
        data[*at..*at + before.len()].copy_from_slice(before);
    }
    exact
}

/// Restore the fields [`blank`] blanked: `None` when the containers found are not the ones recorded or a blanked one
/// no longer parses.
fn restore(data: &mut [u8], kind: Kind, marks: &Marks) -> Option<()> {
    restore_logged(data, kind, marks, &mut Vec::new())
}

/// [`restore`], noting where each field went and what it overwrote.
fn restore_logged(data: &mut [u8], kind: Kind, marks: &Marks, log: &mut Vec<(usize, Vec<u8>)>) -> Option<()> {
    if !marks.any() {
        return Some(());
    }
    let found = kind.candidates(data);
    if found.len() != marks.found {
        return None;
    }
    let mut budget = expansion_bound(data.len());
    let mut kept = marks.kept.iter().peekable();
    for (i, &at) in found.iter().enumerate() {
        if kept.peek() == Some(&&i) {
            kept.next();
            continue;
        }
        for f in kind.fields(data, at, &mut budget)? {
            let place = data.get_mut(f.at..f.at + f.value.len())?;
            log.push((f.at, place.to_vec()));
            place.copy_from_slice(&f.value);
        }
    }
    Some(())
}

/// Blank the ZIP archives' fields in one level's bytes.
pub fn blank_archives(data: &mut [u8]) -> Marks {
    blank(data, Kind::Archive)
}

pub fn restore_archives(data: &mut [u8], marks: &Marks) -> Option<()> {
    restore(data, Kind::Archive, marks)
}

/// Blank the PNG images' chunk CRCs in one level's bytes. Run after [`blank_archives`] and undone before
/// [`restore_archives`], so an image stored in an archive comes back before the archive's CRC is computed over it.
pub fn blank_images(data: &mut [u8]) -> Marks {
    blank(data, Kind::Png)
}

pub fn restore_images(data: &mut [u8], marks: &Marks) -> Option<()> {
    restore(data, Kind::Png, marks)
}

/// Blank the tar headers' checksums in the bytes the model codes.
pub fn blank_tar(data: &mut [u8]) -> Marks {
    blank(data, Kind::Tar)
}

pub fn restore_tar(data: &mut [u8], marks: &Marks) -> Option<()> {
    restore(data, Kind::Tar, marks)
}

const TAR_BLOCK: usize = 512;
/// Where a tar header's `ustar` magic sits.
const TAR_MAGIC: usize = 257;
/// Where its checksum field sits, and how long it is.
const TAR_CHECKSUM: usize = 148;
const TAR_CHECKSUM_LEN: usize = 8;

/// A tar header's checksum: the sum of its 512 bytes with the field itself counted as spaces, in octal, in the field's
/// own shape — leading spaces, the digits (padded with `0`s), then NULs or spaces — whatever the digits are (the
/// blank's are all `0`s, which no header can hold: eight spaces alone sum to 256).
fn tar_field(data: &[u8], h: usize) -> Option<Field> {
    let header = data.get(h..h + TAR_BLOCK)?;
    let field = &header[TAR_CHECKSUM..TAR_CHECKSUM + TAR_CHECKSUM_LEN];
    let lead = field.iter().take_while(|&&b| b == b' ').count();
    let digits = field[lead..].iter().take_while(|b| (b'0'..=b'7').contains(b)).count();
    if digits == 0 || !field[lead + digits..].iter().all(|&b| b == 0 || b == b' ') {
        return None;
    }
    let sum: u32 = header.iter().enumerate().map(|(i, &b)| if (TAR_CHECKSUM..TAR_CHECKSUM + TAR_CHECKSUM_LEN).contains(&i) { 32 } else { u32::from(b) }).sum();
    let octal = format!("{sum:o}");
    if octal.len() > digits {
        return None;
    }
    let mut value = field.to_vec();
    let mut blank = field.to_vec();
    let run = &mut value[lead..lead + digits];
    run.fill(b'0');
    run[digits - octal.len()..].copy_from_slice(octal.as_bytes());
    blank[lead..lead + digits].fill(b'0');
    Some(Field { at: h + TAR_CHECKSUM, value, blank })
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The CRC-32 (big-endian) of every chunk of the PNG whose signature is at `s`, up to its `IEND`, except the `IDAT`
/// chunks' (which [`super::streams::find`] checks before it opens the image data). `None` unless every chunk is there.
fn png_fields(data: &[u8], s: usize) -> Option<Vec<Field>> {
    let mut fields = Vec::new();
    let mut p = s + PNG_SIGNATURE.len();
    loop {
        let len = usize::try_from(u32::from_be_bytes(data.get(p..p + 4)?.try_into().ok()?)).ok()?;
        let end = p.checked_add(8 + len)?;
        let kind = data.get(p + 4..p + 8)?;
        let crc = crc32(data.get(p + 4..end)?);
        data.get(end..end + 4)?;
        if kind != b"IDAT" {
            fields.push(Field { at: end, value: crc.to_be_bytes().to_vec(), blank: vec![0; 4] });
        }
        if kind == b"IEND" {
            return Some(fields);
        }
        p = end + 4;
    }
}

fn u16_at(data: &[u8], at: usize) -> Option<usize> {
    Some(usize::from(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?)))
}

fn u32_at(data: &[u8], at: usize) -> Option<usize> {
    usize::try_from(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?)).ok()
}

/// The fields of the ZIP archive whose end record is at `e`, as its entries determine them: each entry's CRC-32 and
/// sizes — in its local header (a stored entry's CRC-32 alone: its sizes say where it ends), or in the data descriptor
/// after it — and the central directory's copies of them with the offset of each local header. `None` unless the
/// archive is one disk, its local entries follow each other from its start, each is deflated or stored and not
/// encrypted, and its central directory runs up to the end record.
fn archive_fields(data: &[u8], e: usize, budget: &mut usize) -> Option<Vec<Field>> {
    let (disk, cd_disk) = (u16_at(data, e + 4)?, u16_at(data, e + 6)?);
    let (here, entries) = (u16_at(data, e + 8)?, u16_at(data, e + 10)?);
    let (cd_size, cd_offset) = (u32_at(data, e + 12)?, u32_at(data, e + 16)?);
    if disk != 0 || cd_disk != 0 || here != entries || entries == 0 {
        return None;
    }
    let cd_start = e.checked_sub(cd_size)?;
    let start = cd_start.checked_sub(cd_offset)?;
    let mut fields = Vec::new();
    let mut described = Vec::with_capacity(entries);
    let mut pos = start;
    for _ in 0..entries {
        if data.get(pos..pos + 4)? != b"PK\x03\x04" {
            return None;
        }
        let (flags, method) = (u16_at(data, pos + 6)?, u16_at(data, pos + 8)?);
        let body = pos + 30 + u16_at(data, pos + 26)? + u16_at(data, pos + 28)?;
        if flags & 1 != 0 {
            return None;
        }
        let (crc, compressed, size) = match method {
            8 => {
                let inflated = inflate(data.get(body..)?, *budget)?;
                *budget -= inflated.data.len();
                (crc32(&inflated.data), inflated.consumed, inflated.data.len())
            }
            0 if flags & 8 == 0 => {
                let n = u32_at(data, pos + 18)?;
                (crc32(data.get(body..body + n)?), n, n)
            }
            _ => return None,
        };
        let (compressed32, size32) = (u32::try_from(compressed).ok()?, u32::try_from(size).ok()?);
        let mut next = body + compressed;
        if flags & 8 != 0 {
            let at = if data.get(next..next + 4)? == b"PK\x07\x08" { next + 4 } else { next };
            fields.extend([Field::zeroed(at, crc), Field::zeroed(at + 4, compressed32), Field::zeroed(at + 8, size32)]);
            next = at + 12;
        } else if method == 8 {
            fields.extend([Field::zeroed(pos + 14, crc), Field::zeroed(pos + 18, compressed32), Field::zeroed(pos + 22, size32)]);
        } else {
            fields.push(Field::zeroed(pos + 14, crc));
        }
        described.push((crc, compressed32, size32, u32::try_from(pos - start).ok()?));
        pos = next;
    }
    let mut c = cd_start;
    for &(crc, compressed, size, offset) in &described {
        if data.get(c..c + 4)? != b"PK\x01\x02" {
            return None;
        }
        fields.extend([Field::zeroed(c + 16, crc), Field::zeroed(c + 20, compressed), Field::zeroed(c + 24, size), Field::zeroed(c + 42, offset)]);
        c += 46 + u16_at(data, c + 28)? + u16_at(data, c + 30)? + u16_at(data, c + 32)?;
    }
    (c == e).then_some(fields)
}

/// No trailer blanked; the zlib stream's Adler-32 (big-endian); the gzip member's CRC-32 and length.
pub const TRAILER_NONE: u8 = 0;
pub const TRAILER_ZLIB: u8 = 1;
pub const TRAILER_GZIP: u8 = 2;

/// Where each stream of a level sits in the level's opened bytes: its decoded bytes' start and end.
fn opened_spans(streams: &[Stream]) -> Vec<(usize, usize)> {
    let mut shift = 0isize;
    streams
        .iter()
        .map(|s| {
            let start = (s.offset as isize + shift) as usize;
            shift += s.raw as isize - s.compressed as isize;
            (start, start + s.raw)
        })
        .collect()
}

fn trailer(raw: &[u8], code: u8) -> Vec<u8> {
    match code {
        TRAILER_ZLIB => adler32(raw).to_be_bytes().to_vec(),
        _ => [crc32(raw).to_le_bytes(), (raw.len() as u32).to_le_bytes()].concat(),
    }
}

/// Blank the trailer after each DEFLATE stream of a level that holds its decoded bytes' checksum, in the level's
/// opened bytes `opened`: one code per stream.
pub fn blank_trailers(opened: &mut [u8], streams: &[Stream]) -> Vec<u8> {
    let spans = opened_spans(streams);
    (0..streams.len())
        .map(|i| {
            let (start, end) = spans[i];
            let room = spans.get(i + 1).map_or(opened.len(), |next| next.0);
            if !matches!(streams[i].codec, Codec::Deflate(_) | Codec::Gzip { .. }) {
                return TRAILER_NONE;
            }
            for code in [TRAILER_ZLIB, TRAILER_GZIP] {
                let value = trailer(&opened[start..end], code);
                if end + value.len() <= room && opened[end..end + value.len()] == value[..] {
                    opened[end..end + value.len()].fill(0);
                    return code;
                }
            }
            TRAILER_NONE
        })
        .collect()
}

pub fn restore_trailers(opened: &mut [u8], streams: &[Stream], codes: &[u8]) -> Option<()> {
    if codes.len() != streams.len() {
        return None;
    }
    let spans = opened_spans(streams);
    for (i, &code) in codes.iter().enumerate() {
        if code == TRAILER_NONE {
            continue;
        }
        let (start, end) = spans[i];
        let value = trailer(opened.get(start..end)?, code);
        if end + value.len() > spans.get(i + 1).map_or(opened.len(), |next| next.0) {
            return None;
        }
        opened.get_mut(end..end + value.len())?.copy_from_slice(&value);
    }
    Some(())
}

fn write_marks(m: &Marks, out: &mut Vec<u8>) {
    write_uvarint(m.found as u64, out);
    write_uvarint(m.kept.len() as u64, out);
    let mut last = 0;
    for &k in &m.kept {
        write_uvarint((k - last) as u64, out);
        last = k + 1;
    }
}

fn read_marks(buf: &[u8], pos: &mut usize, len: usize) -> Option<Marks> {
    let found = usize::try_from(read_uvarint(buf, pos)?).ok()?;
    let n = usize::try_from(read_uvarint(buf, pos)?).ok()?;
    if found > len || n > found {
        return None;
    }
    let mut kept = Vec::with_capacity(n);
    let mut next = 0usize;
    for _ in 0..n {
        let k = next.checked_add(usize::try_from(read_uvarint(buf, pos)?).ok()?)?;
        if k >= found {
            return None;
        }
        kept.push(k);
        next = k + 1;
    }
    Some(Marks { found, kept })
}

/// Header: each level's archive and image marks (the levels' streams plus one of each), each level's trailer codes
/// (one byte per stream), then the tar marks; marks are the count found, the count kept, and each kept index's
/// distance from the last.
pub fn write_record(r: &Record, out: &mut Vec<u8>) {
    for (a, i) in r.archives.iter().zip(&r.images) {
        write_marks(a, out);
        write_marks(i, out);
    }
    for codes in &r.trailers {
        out.extend_from_slice(codes);
    }
    write_marks(&r.tar, out);
}

/// Read a [`write_record`] record for levels of `streams` streams each, over bytes of at most `len`: `None` when it
/// is malformed or a code is not one of the trailers.
pub fn read_record(buf: &[u8], pos: &mut usize, streams: &[usize], len: usize) -> Option<Record> {
    let (mut archives, mut images) = (Vec::new(), Vec::new());
    for _ in 0..=streams.len() {
        archives.push(read_marks(buf, pos, len)?);
        images.push(read_marks(buf, pos, len)?);
    }
    let mut trailers = Vec::with_capacity(streams.len());
    for &n in streams {
        let codes = buf.get(*pos..pos.checked_add(n)?)?.to_vec();
        if codes.iter().any(|&c| c > TRAILER_GZIP) {
            return None;
        }
        *pos += n;
        trailers.push(codes);
    }
    let tar = read_marks(buf, pos, len)?;
    Some(Record { archives, images, trailers, tar })
}
