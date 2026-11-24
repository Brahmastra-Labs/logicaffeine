//! A PyTorch export archive — the `.pt2` file `torch.export.save` writes: a zip whose root folder holds the exported
//! program's graph (`models/model.json`, the ATen graph as JSON), its weights (`data/weights/`, each tensor's bytes raw
//! and little-endian, `model_weights_config.json` naming each one's file, dtype, sizes and strides) and its constants.
//!
//! The archive is read as a zip is: its end-of-central-directory record found from the end of the file (its ZIP64
//! record where a count, size or offset outgrows 32 bits), every entry's name, method, sizes and local header offset
//! read from the central directory, and an entry's bytes read from where its local header ends — stored, or inflated
//! from DEFLATE. Nothing is read until it is asked for, so a model's weights are read a row at a time, never whole.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// One file of the archive: where its data begins, its sizes and how it is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Entry {
    /// The offset of its local header.
    header: u64,
    compressed: u64,
    size: u64,
    /// 0 stored, 8 DEFLATE.
    method: u16,
}

/// A `.pt2` archive opened: its entries by name, and the root folder they all sit under.
#[derive(Debug)]
pub struct Pt2 {
    path: PathBuf,
    entries: BTreeMap<String, Entry>,
    root: String,
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("four bytes"))
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("eight bytes"))
}

const EOCD: u32 = 0x0605_4b50;
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
const ZIP64_EOCD: u32 = 0x0606_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

impl Pt2 {
    /// The archive at `path`, its central directory read. Refused when it is not a zip, an entry is stored other than
    /// as stored or DEFLATE, or its files do not sit under one root folder.
    pub fn open(path: &Path) -> io::Result<Pt2> {
        let mut file = File::open(path)?;
        let len = file.metadata()?.len();
        // The end-of-central-directory record is 22 bytes and ends the file, but for a comment of at most 65,535 bytes.
        let tail_len = len.min(22 + 65_535);
        file.seek(SeekFrom::Start(len - tail_len))?;
        let mut tail = vec![0u8; tail_len as usize];
        file.read_exact(&mut tail)?;
        let at = (0..tail.len().saturating_sub(21))
            .rev()
            .find(|&i| u32_at(&tail, i) == EOCD && i + 22 + u16_at(&tail, i + 20) as usize == tail.len())
            .ok_or_else(|| invalid(format!("{}: not a zip archive (no end of central directory)", path.display())))?;
        let (mut count, mut cd_size, mut cd_offset) =
            (u16_at(&tail, at + 10) as u64, u32_at(&tail, at + 12) as u64, u32_at(&tail, at + 16) as u64);
        if count == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_offset == 0xFFFF_FFFF {
            // ZIP64: the locator sits just before the record and names where the ZIP64 record is.
            let locator = at.checked_sub(20).filter(|&l| u32_at(&tail, l) == ZIP64_LOCATOR).ok_or_else(|| {
                invalid(format!("{}: a ZIP64 archive without its locator", path.display()))
            })?;
            let record = u64_at(&tail, locator + 8);
            file.seek(SeekFrom::Start(record))?;
            let mut r = [0u8; 56];
            file.read_exact(&mut r)?;
            if u32_at(&r, 0) != ZIP64_EOCD {
                return Err(invalid(format!("{}: the ZIP64 record is not where its locator says", path.display())));
            }
            (count, cd_size, cd_offset) = (u64_at(&r, 32), u64_at(&r, 40), u64_at(&r, 48));
        }
        file.seek(SeekFrom::Start(cd_offset))?;
        let mut cd = vec![0u8; cd_size as usize];
        file.read_exact(&mut cd)?;
        let mut entries = BTreeMap::new();
        let mut p = 0usize;
        for _ in 0..count {
            if p + 46 > cd.len() || u32_at(&cd, p) != CENTRAL {
                return Err(invalid(format!("{}: a central directory entry is malformed", path.display())));
            }
            let method = u16_at(&cd, p + 10);
            let (mut compressed, mut size) = (u32_at(&cd, p + 20) as u64, u32_at(&cd, p + 24) as u64);
            let (name_len, extra_len, comment_len) =
                (u16_at(&cd, p + 28) as usize, u16_at(&cd, p + 30) as usize, u16_at(&cd, p + 32) as usize);
            let mut header = u32_at(&cd, p + 42) as u64;
            let name = String::from_utf8(cd[p + 46..p + 46 + name_len].to_vec())
                .map_err(|_| invalid(format!("{}: an entry's name is not UTF-8", path.display())))?;
            // The ZIP64 extra field holds, in order, each of the three values that did not fit 32 bits.
            let mut e = p + 46 + name_len;
            let extra_end = e + extra_len;
            while e + 4 <= extra_end {
                let (id, n) = (u16_at(&cd, e), u16_at(&cd, e + 2) as usize);
                if id == 0x0001 {
                    let mut q = e + 4;
                    for field in [&mut size, &mut compressed, &mut header] {
                        if *field == 0xFFFF_FFFF && q + 8 <= e + 4 + n {
                            *field = u64_at(&cd, q);
                            q += 8;
                        }
                    }
                }
                e += 4 + n;
            }
            if method != 0 && method != 8 {
                return Err(invalid(format!("{}: `{name}` is stored by method {method}, neither stored nor DEFLATE", path.display())));
            }
            entries.insert(name, Entry { header, compressed, size, method });
            p = extra_end + comment_len;
        }
        let root = entries
            .keys()
            .next()
            .and_then(|n| n.split_once('/'))
            .map(|(r, _)| format!("{r}/"))
            .ok_or_else(|| invalid(format!("{}: an archive with no folder", path.display())))?;
        if entries.keys().any(|n| !n.starts_with(&root)) {
            return Err(invalid(format!("{}: its files do not sit under one root folder", path.display())));
        }
        Ok(Pt2 { path: path.to_path_buf(), entries, root })
    }

    /// Whether the archive holds `rel` under its root folder.
    pub fn has(&self, rel: &str) -> bool {
        self.entries.contains_key(&format!("{}{rel}", self.root))
    }

    fn entry(&self, rel: &str) -> io::Result<(String, Entry)> {
        let name = format!("{}{rel}", self.root);
        let e = *self
            .entries
            .get(&name)
            .ok_or_else(|| invalid(format!("{}: no `{rel}` in the archive", self.path.display())))?;
        Ok((name, e))
    }

    /// Where `entry`'s data begins: past its local header, whose name and extra field may differ in length from the
    /// central directory's.
    fn data_start(&self, file: &mut File, name: &str, e: Entry) -> io::Result<u64> {
        file.seek(SeekFrom::Start(e.header))?;
        let mut h = [0u8; 30];
        file.read_exact(&mut h)?;
        if u32_at(&h, 0) != LOCAL {
            return Err(invalid(format!("{}: `{name}`'s local header is not where the directory says", self.path.display())));
        }
        Ok(e.header + 30 + u16_at(&h, 26) as u64 + u16_at(&h, 28) as u64)
    }

    /// The bytes of `rel`, whole: stored, or inflated.
    pub fn read(&self, rel: &str) -> io::Result<Vec<u8>> {
        let (name, e) = self.entry(rel)?;
        let mut file = File::open(&self.path)?;
        let start = self.data_start(&mut file, &name, e)?;
        file.seek(SeekFrom::Start(start))?;
        let mut raw = vec![0u8; e.compressed as usize];
        file.read_exact(&mut raw)?;
        let bytes = match e.method {
            0 => raw,
            _ => miniz_oxide::inflate::decompress_to_vec(&raw)
                .map_err(|why| invalid(format!("{}: `{rel}` does not inflate: {why:?}", self.path.display())))?,
        };
        if bytes.len() as u64 != e.size {
            return Err(invalid(format!("{}: `{rel}` holds {} bytes where the directory says {}", self.path.display(), bytes.len(), e.size)));
        }
        Ok(bytes)
    }

    /// `len` bytes of `rel` from `offset`: read in place where it is stored, from the inflated whole where it is not.
    pub fn read_range(&self, rel: &str, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let (name, e) = self.entry(rel)?;
        if offset + len as u64 > e.size {
            return Err(invalid(format!("{}: bytes {offset}..{} of `{rel}`, which holds {}", self.path.display(), offset + len as u64, e.size)));
        }
        if e.method != 0 {
            let whole = self.read(rel)?;
            return Ok(whole[offset as usize..offset as usize + len].to_vec());
        }
        let mut file = File::open(&self.path)?;
        let start = self.data_start(&mut file, &name, e)?;
        file.seek(SeekFrom::Start(start + offset))?;
        let mut bytes = vec![0u8; len];
        file.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    /// `rel` read as JSON.
    pub fn json(&self, rel: &str) -> io::Result<serde_json::Value> {
        serde_json::from_slice(&self.read(rel)?).map_err(|why| invalid(format!("{}: `{rel}` is not JSON: {why}", self.path.display())))
    }

    /// Every file under the root folder, by its path below it.
    pub fn files(&self) -> Vec<String> {
        self.entries.keys().map(|n| n[self.root.len()..].to_string()).collect()
    }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/model/pt2/tests.rs"]
mod tests;
