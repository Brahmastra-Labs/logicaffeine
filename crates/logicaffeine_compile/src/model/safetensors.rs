//! Safetensors files as a checkpoint directory publishes them: one file, or shards named by
//! `model.safetensors.index.json`. A file is an 8-byte little-endian header length, a JSON header naming every tensor's
//! dtype, shape and byte range within the data section, then the data. Tensors are read by position, never mapped or
//! read whole, so a checkpoint of any size opens in the time it takes to read its headers.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

/// An element type a safetensors header names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dtype {
    F64,
    F32,
    F16,
    Bf16,
    F8E4m3,
    F8E5m2,
    I64,
    I32,
    I16,
    I8,
    U8,
    Bool,
}

impl Dtype {
    fn parse(s: &str) -> Option<Dtype> {
        Some(match s {
            "F64" => Dtype::F64,
            "F32" => Dtype::F32,
            "F16" => Dtype::F16,
            "BF16" => Dtype::Bf16,
            "F8_E4M3" => Dtype::F8E4m3,
            "F8_E5M2" => Dtype::F8E5m2,
            "I64" => Dtype::I64,
            "I32" => Dtype::I32,
            "I16" => Dtype::I16,
            "I8" => Dtype::I8,
            "U8" => Dtype::U8,
            "BOOL" => Dtype::Bool,
            _ => return None,
        })
    }

    /// Bytes an element takes.
    pub fn bytes(self) -> usize {
        match self {
            Dtype::F64 | Dtype::I64 => 8,
            Dtype::F32 | Dtype::I32 => 4,
            Dtype::F16 | Dtype::Bf16 | Dtype::I16 => 2,
            Dtype::F8E4m3 | Dtype::F8E5m2 | Dtype::I8 | Dtype::U8 | Dtype::Bool => 1,
        }
    }
}

/// One tensor's header entry: its dtype, its shape (outermost first, as PyTorch states it) and its byte range within
/// its file's data section.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub dtype: Dtype,
    pub shape: Vec<u64>,
    pub begin: u64,
    pub end: u64,
}

impl Entry {
    pub fn elements(&self) -> u64 {
        self.shape.iter().product()
    }
}

/// One safetensors file.
pub struct Shard {
    file: File,
    path: PathBuf,
    data_start: u64,
    pub tensors: BTreeMap<String, Entry>,
}

fn invalid(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

impl Shard {
    pub fn open(path: &Path) -> io::Result<Shard> {
        let mut file = File::open(path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        let len = file.metadata()?.len();
        let mut head = [0u8; 8];
        file.read_exact(&mut head)?;
        let header_len = u64::from_le_bytes(head);
        if header_len > len.saturating_sub(8) {
            return Err(invalid(format!("{}: a header of {header_len} bytes in a file of {len}", path.display())));
        }
        let mut header = vec![0u8; header_len as usize];
        file.read_exact(&mut header)?;
        let json: serde_json::Value = serde_json::from_slice(&header)
            .map_err(|e| invalid(format!("{}: the header is not JSON: {e}", path.display())))?;
        let object = json.as_object().ok_or_else(|| invalid(format!("{}: the header is not an object", path.display())))?;
        let data_start = 8 + header_len;
        let data_len = len - data_start;
        let mut tensors = BTreeMap::new();
        for (name, v) in object {
            if name == "__metadata__" {
                continue;
            }
            let dtype_name = v["dtype"].as_str().unwrap_or_default();
            let dtype = Dtype::parse(dtype_name)
                .ok_or_else(|| invalid(format!("{}: {name} has dtype `{dtype_name}`, which is not read", path.display())))?;
            let shape: Vec<u64> = v["shape"]
                .as_array()
                .ok_or_else(|| invalid(format!("{}: {name} has no shape", path.display())))?
                .iter()
                .map(|d| d.as_u64().ok_or_else(|| invalid(format!("{}: {name}'s shape is not integers", path.display()))))
                .collect::<io::Result<_>>()?;
            let offsets = v["data_offsets"]
                .as_array()
                .filter(|a| a.len() == 2)
                .ok_or_else(|| invalid(format!("{}: {name} has no data offsets", path.display())))?;
            let (begin, end) = (offsets[0].as_u64().unwrap_or(u64::MAX), offsets[1].as_u64().unwrap_or(u64::MAX));
            let entry = Entry { dtype, shape, begin, end };
            if begin > end || end > data_len || end - begin != entry.elements() * dtype.bytes() as u64 {
                return Err(invalid(format!(
                    "{}: {name} claims bytes {begin}..{end} for {} elements of {dtype_name} in a data section of {data_len}",
                    path.display(),
                    entry.elements()
                )));
            }
            tensors.insert(name.clone(), entry);
        }
        Ok(Shard { file, path: path.to_path_buf(), data_start, tensors })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `len` bytes of tensor `name` starting `offset` bytes into it.
    pub fn read(&self, name: &str, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let e = self
            .tensors
            .get(name)
            .ok_or_else(|| invalid(format!("{}: no tensor {name}", self.path.display())))?;
        if offset + len as u64 > e.end - e.begin {
            return Err(invalid(format!("{}: {name} read past its end", self.path.display())));
        }
        let mut out = vec![0u8; len];
        self.read_at(&mut out, self.data_start + e.begin + offset)?;
        Ok(out)
    }

    /// `buf.len()` bytes of the file from `offset`, moving no cursor another reader of the shard shares: a positional
    /// read where the platform has one, and elsewhere a seek and a read on a handle of the read's own.
    #[cfg(unix)]
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        use std::os::unix::fs::FileExt;
        self.file.read_exact_at(buf, offset)
    }

    #[cfg(windows)]
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < buf.len() {
            match self.file.seek_read(&mut buf[done..], offset + done as u64)? {
                0 => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, format!("{}: read past its end", self.path.display()))),
                n => done += n,
            }
        }
        Ok(())
    }

    #[cfg(not(any(unix, windows)))]
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<()> {
        use std::io::{Seek, SeekFrom};
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(buf)
    }
}

/// The shard files a checkpoint directory holds: the files its index names, else its one `model.safetensors`, else
/// every `*.safetensors` file in it. A file the index names that is not there is refused by name.
pub fn shard_paths(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let index = dir.join("model.safetensors.index.json");
    if index.is_file() {
        let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&index)?)
            .map_err(|e| invalid(format!("{}: {e}", index.display())))?;
        let map = json["weight_map"]
            .as_object()
            .ok_or_else(|| invalid(format!("{}: no weight_map", index.display())))?;
        let mut files: Vec<String> = map.values().filter_map(|v| v.as_str().map(str::to_string)).collect();
        files.sort();
        files.dedup();
        let mut out = Vec::new();
        for f in files {
            let p = dir.join(&f);
            if !p.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{}: the index names shard {f}, which the directory does not hold", dir.display()),
                ));
            }
            out.push(p);
        }
        return Ok(out);
    }
    let single = dir.join("model.safetensors");
    if single.is_file() {
        return Ok(vec![single]);
    }
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "safetensors"))
        .collect();
    out.sort();
    if out.is_empty() {
        return Err(io::Error::new(io::ErrorKind::NotFound, format!("{}: no safetensors file", dir.display())));
    }
    Ok(out)
}
