//! A read-only GGUF reader: the whole metadata table, typed, the tensor directory, and every tensor's weights in
//! the formats [`super::quant::GgmlType`] reads.
//!
//! GGUF layout: `"GGUF"` magic, u32 version, u64 tensor count, u64 metadata count, the metadata key/value pairs,
//! the tensor infos (name, dims, ggml type, offset), then — aligned to `general.alignment` — the tensor data.
//! Metadata keys of the model's architecture are prefixed by `general.architecture` (`qwen3.block_count`); the
//! tokenizer's live under `tokenizer.ggml.*`. Nothing is skipped: a tokenizer, a chat template and every
//! architecture key are readable by name.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};

use super::quant::{f16_to_f32, GgmlType};

/// One metadata value.
#[derive(Clone, Debug, PartialEq)]
pub enum MetaValue {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    F32(f32),
    F64(f64),
    Bool(bool),
    /// A GGUF string: bytes, which a vocabulary's tokens need not keep UTF-8.
    Str(Vec<u8>),
    /// An array, with the GGUF type of its elements (which an empty array still has).
    Array { ty: u32, items: Vec<MetaValue> },
}

impl MetaValue {
    /// The value as an unsigned integer, when it is a non-negative integer.
    pub fn as_u64(&self) -> Option<u64> {
        match *self {
            MetaValue::U8(v) => Some(v as u64),
            MetaValue::U16(v) => Some(v as u64),
            MetaValue::U32(v) => Some(v as u64),
            MetaValue::U64(v) => Some(v),
            MetaValue::I8(v) if v >= 0 => Some(v as u64),
            MetaValue::I16(v) if v >= 0 => Some(v as u64),
            MetaValue::I32(v) if v >= 0 => Some(v as u64),
            MetaValue::I64(v) if v >= 0 => Some(v as u64),
            _ => None,
        }
    }

    /// The value as a signed integer, when it is an integer that fits.
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            MetaValue::I8(v) => Some(v as i64),
            MetaValue::I16(v) => Some(v as i64),
            MetaValue::I32(v) => Some(v as i64),
            MetaValue::I64(v) => Some(v),
            MetaValue::U8(v) => Some(v as i64),
            MetaValue::U16(v) => Some(v as i64),
            MetaValue::U32(v) => Some(v as i64),
            MetaValue::U64(v) => i64::try_from(v).ok(),
            _ => None,
        }
    }

    /// The value as a float, when it is a float.
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            MetaValue::F32(v) => Some(v as f64),
            MetaValue::F64(v) => Some(v),
            _ => None,
        }
    }

    /// The value as text, when it is a UTF-8 string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            MetaValue::Str(s) => std::str::from_utf8(s).ok(),
            _ => None,
        }
    }

    /// The value's bytes, when it is a string.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            MetaValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            MetaValue::Bool(b) => Some(b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[MetaValue]> {
        match self {
            MetaValue::Array { items, .. } => Some(items),
            _ => None,
        }
    }
}

/// The transformer architecture fields the hand-written model lanes read from GGUF metadata. Every field is a
/// key under the architecture prefix and reads 0 (or the documented default) when absent.
#[derive(Debug, Clone)]
pub struct Arch {
    pub arch: String,
    pub block_count: u64,
    /// `context_length` — the trained context window (Qwen3.8: 262144).
    pub context_length: u64,
    pub embedding_length: u64,
    pub feed_forward_length: u64,
    pub head_count: u64,
    pub head_count_kv: u64,
    pub head_dim: u64,
    pub rms_eps: f32,
    pub rope_freq_base: f32,
    /// `attention.value_length` — V head dim, which need not equal the K head dim.
    pub value_length: u64,
    /// `rope.dimension_count` — how many of each head's dims actually rotate. Qwen3.8 rotates **64 of 256**;
    /// treating this as the full head_dim silently corrupts three quarters of every query and key.
    pub rope_dimension_count: u64,
    /// `full_attention_interval` — every Nth block is full attention, the rest are linear (Gated DeltaNet).
    /// Qwen3.8: 4, so blocks 3, 7, 11 … are attention and the other 48 are GDN.
    pub full_attention_interval: u64,
    /// `nextn_predict_layers` — MTP draft blocks appended to `block_count`. Qwen3.8 ships 1, inside the same
    /// file, so `block_count` (65) is one MORE than the model's real depth (64).
    pub nextn_predict_layers: u64,
    /// `ssm.conv_kernel` — taps in GDN's depthwise causal conv (4).
    pub ssm_conv_kernel: u64,
    /// `ssm.state_size` — the D of each head's D×D associative memory (128).
    pub ssm_state_size: u64,
    /// `ssm.group_count` — GDN **key** heads (16).
    pub ssm_group_count: u64,
    /// `ssm.time_step_rank` — GDN **value** heads (48). Three value heads share each key head.
    pub ssm_time_step_rank: u64,
    /// `ssm.inner_size` — the GDN value width, `time_step_rank · state_size` (6144).
    pub ssm_inner_size: u64,
}

impl Arch {
    /// Blocks that are actually part of the model — `block_count` minus the MTP draft blocks appended to it.
    pub fn model_layers(&self) -> u64 {
        self.block_count.saturating_sub(self.nextn_predict_layers)
    }

    /// Is block `l` a full-attention block? With `full_attention_interval = N`, every Nth block is, counting
    /// from 1, i.e. indices `N-1, 2N-1, …`. A checkpoint without the key is uniformly full attention.
    pub fn is_full_attention_layer(&self, l: u64) -> bool {
        self.full_attention_interval == 0 || (l + 1) % self.full_attention_interval == 0
    }

    fn from_meta(meta: &BTreeMap<String, MetaValue>) -> Arch {
        let arch = meta
            .get("general.architecture")
            .and_then(MetaValue::as_str)
            .unwrap_or("")
            .to_string();
        let g = |suffix: &str| -> u64 {
            meta.get(&format!("{arch}.{suffix}"))
                .and_then(MetaValue::as_u64)
                .unwrap_or(0)
        };
        let gf = |suffix: &str, d: f32| -> f32 {
            meta.get(&format!("{arch}.{suffix}"))
                .and_then(MetaValue::as_f64)
                .map_or(d, |v| v as f32)
        };
        Arch {
            block_count: g("block_count"),
            context_length: g("context_length"),
            embedding_length: g("embedding_length"),
            feed_forward_length: g("feed_forward_length"),
            head_count: g("attention.head_count"),
            head_count_kv: g("attention.head_count_kv"),
            head_dim: g("attention.key_length"),
            rms_eps: gf("attention.layer_norm_rms_epsilon", 1e-6),
            rope_freq_base: gf("rope.freq_base", 10000.0),
            value_length: g("attention.value_length"),
            rope_dimension_count: g("rope.dimension_count"),
            full_attention_interval: g("full_attention_interval"),
            nextn_predict_layers: g("nextn_predict_layers"),
            ssm_conv_kernel: g("ssm.conv_kernel"),
            ssm_state_size: g("ssm.state_size"),
            ssm_group_count: g("ssm.group_count"),
            ssm_time_step_rank: g("ssm.time_step_rank"),
            ssm_inner_size: g("ssm.inner_size"),
            arch,
        }
    }
}

/// One tensor's entry in the directory.
#[derive(Debug, Clone)]
pub struct TensorInfo {
    pub name: String,
    /// ggml order: `dims[0]` is the fastest-varying (a weight's in-features).
    pub dims: Vec<u64>,
    /// The GGUF type id (read with [`GgmlType::from_id`]).
    pub ggml_type: u32,
    /// Offset of the tensor's bytes from the start of the data section.
    pub offset: u64,
}

impl TensorInfo {
    pub fn elements(&self) -> usize {
        self.dims.iter().map(|&d| d as usize).product()
    }
}

/// A parsed GGUF file: the metadata, the tensor directory, and where the data section starts.
pub struct Gguf {
    file: std::fs::File,
    pub version: u32,
    pub meta: BTreeMap<String, MetaValue>,
    /// The metadata keys in the order the file lists them.
    pub order: Vec<String>,
    pub arch: Arch,
    tensors: Vec<TensorInfo>,
    data_start: u64,
}

/// FNV-1a over 64 bits.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

fn invalid(msg: String) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, msg)
}

fn not_found(name: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::NotFound, format!("no tensor {name}"))
}

fn ru8(f: &mut impl Read) -> std::io::Result<u8> {
    let mut b = [0u8; 1];
    f.read_exact(&mut b)?;
    Ok(b[0])
}
fn ru16(f: &mut impl Read) -> std::io::Result<u16> {
    let mut b = [0u8; 2];
    f.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}
fn ru32(f: &mut impl Read) -> std::io::Result<u32> {
    let mut b = [0u8; 4];
    f.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn ru64(f: &mut impl Read) -> std::io::Result<u64> {
    let mut b = [0u8; 8];
    f.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}
fn rbytes(f: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let n = ru64(f)? as usize;
    let mut b = vec![0u8; n];
    f.read_exact(&mut b)?;
    Ok(b)
}

/// A key or tensor name, which must be UTF-8.
fn rstr(f: &mut impl Read) -> std::io::Result<String> {
    String::from_utf8(rbytes(f)?).map_err(|e| invalid(format!("a metadata key or tensor name is not UTF-8: {e}")))
}

/// Read one metadata value of GGUF type `t`.
fn read_value(f: &mut impl Read, t: u32) -> std::io::Result<MetaValue> {
    Ok(match t {
        0 => MetaValue::U8(ru8(f)?),
        1 => MetaValue::I8(ru8(f)? as i8),
        2 => MetaValue::U16(ru16(f)?),
        3 => MetaValue::I16(ru16(f)? as i16),
        4 => MetaValue::U32(ru32(f)?),
        5 => MetaValue::I32(ru32(f)? as i32),
        6 => MetaValue::F32(f32::from_bits(ru32(f)?)),
        7 => MetaValue::Bool(ru8(f)? != 0),
        8 => MetaValue::Str(rbytes(f)?),
        9 => {
            let et = ru32(f)?;
            let n = ru64(f)? as usize;
            let mut items = Vec::with_capacity(n.min(1 << 24));
            for _ in 0..n {
                items.push(read_value(f, et)?);
            }
            MetaValue::Array { ty: et, items }
        }
        10 => MetaValue::U64(ru64(f)?),
        11 => MetaValue::I64(ru64(f)? as i64),
        12 => MetaValue::F64(f64::from_bits(ru64(f)?)),
        other => return Err(invalid(format!("unknown GGUF metadata type {other}"))),
    })
}

impl Gguf {
    /// Open and parse a GGUF file's header, metadata and tensor directory. The weights stay on disk until read.
    pub fn open(path: &std::path::Path) -> std::io::Result<Gguf> {
        let file = std::fs::File::open(path)?;
        let mut r = std::io::BufReader::new(file);
        let mut magic = [0u8; 4];
        r.read_exact(&mut magic)?;
        if &magic != b"GGUF" {
            return Err(invalid(format!("{} is not a GGUF file", path.display())));
        }
        let version = ru32(&mut r)?;
        if version < 2 {
            return Err(invalid(format!(
                "GGUF version {version} (32-bit counts) is not read; versions 2 and 3 are"
            )));
        }
        let n_tensors = ru64(&mut r)?;
        let n_kv = ru64(&mut r)?;
        let mut meta = BTreeMap::new();
        let mut order = Vec::with_capacity(n_kv as usize);
        for _ in 0..n_kv {
            let key = rstr(&mut r)?;
            let t = ru32(&mut r)?;
            order.push(key.clone());
            meta.insert(key, read_value(&mut r, t)?);
        }
        let mut tensors = Vec::with_capacity(n_tensors as usize);
        for _ in 0..n_tensors {
            let name = rstr(&mut r)?;
            let ndims = ru32(&mut r)? as usize;
            let dims = (0..ndims)
                .map(|_| ru64(&mut r))
                .collect::<std::io::Result<Vec<u64>>>()?;
            let ggml_type = ru32(&mut r)?;
            let offset = ru64(&mut r)?;
            tensors.push(TensorInfo {
                name,
                dims,
                ggml_type,
                offset,
            });
        }
        let align = meta
            .get("general.alignment")
            .and_then(MetaValue::as_u64)
            .unwrap_or(32);
        let pos = r.stream_position()?;
        let data_start = pos.div_ceil(align) * align;
        let arch = Arch::from_meta(&meta);
        Ok(Gguf {
            file: r.into_inner(),
            version,
            meta,
            order,
            arch,
            tensors,
            data_start,
        })
    }

    /// Where the data section starts: a tensor's bytes sit at `data_start() + offset`.
    pub fn data_start(&self) -> u64 {
        self.data_start
    }

    /// The FNV-1a 64 digest of everything before the data section — the header, the metadata and the tensor
    /// directory: a program compiled against this checkpoint checks it reads the same file.
    pub fn header_digest(&mut self) -> std::io::Result<u64> {
        let mut head = vec![0u8; self.data_start as usize];
        self.file.seek(SeekFrom::Start(0))?;
        self.file.read_exact(&mut head)?;
        Ok(fnv1a64(&head))
    }

    /// A metadata value by its full key.
    pub fn meta(&self, key: &str) -> Option<&MetaValue> {
        self.meta.get(key)
    }

    /// A metadata value under the architecture prefix (`{arch}.{suffix}`).
    pub fn arch_meta(&self, suffix: &str) -> Option<&MetaValue> {
        self.meta.get(&format!("{}.{suffix}", self.arch.arch))
    }

    /// The bytes of weights one decode step reads for a token. That is every tensor of the model's blocks and around
    /// them, with three exceptions:
    /// - the token embedding, where an output matrix of its own stands beside it (the step reads one of its rows);
    /// - each expert matrix, counted in the share of its experts a token is routed to (`expert_used_count` of
    ///   `expert_count`);
    /// - the draft blocks appended for multi-token prediction, which count nothing.
    /// A tensor in a format this crate does not read is an error.
    pub fn decode_read_bytes(&self) -> std::io::Result<u64> {
        let untied = self.info("output.weight").is_some();
        let experts = self.arch_meta("expert_count").and_then(MetaValue::as_u64).unwrap_or(0);
        let routed = self.arch_meta("expert_used_count").and_then(MetaValue::as_u64).unwrap_or(0);
        let layers = self.arch.model_layers();
        let mut total = 0u128;
        for t in &self.tensors {
            if untied && t.name == "token_embd.weight" {
                continue;
            }
            let block = t.name.strip_prefix("blk.").and_then(|r| r.split('.').next()).and_then(|b| b.parse::<u64>().ok());
            if block.is_some_and(|b| b >= layers) {
                continue;
            }
            let ty = self.tensor_type(&t.name)?;
            let bytes = ty.bytes_for(t.elements()) as u128;
            total += if t.name.contains("_exps.") && experts > 0 {
                bytes * routed as u128 / experts as u128
            } else {
                bytes
            };
        }
        Ok(total as u64)
    }

    /// The tensor directory, in file order.
    pub fn tensors(&self) -> &[TensorInfo] {
        &self.tensors
    }

    /// One tensor's directory entry.
    pub fn info(&self, name: &str) -> Option<&TensorInfo> {
        self.tensors.iter().find(|t| t.name == name)
    }

    /// The format of a tensor, or why it is not one this crate reads.
    pub fn tensor_type(&self, name: &str) -> std::io::Result<GgmlType> {
        let t = self.info(name).ok_or_else(|| not_found(name))?;
        GgmlType::from_id(t.ggml_type).map_err(|id| {
            invalid(format!(
                "{name} is ggml type {id}, a format this crate does not read"
            ))
        })
    }

    /// The raw bytes of rows `[first, first + count)` of a tensor, a row being its `dims[0]` fastest elements.
    pub fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>> {
        let ty = self.tensor_type(name)?;
        let t = self.info(name).ok_or_else(|| not_found(name))?.clone();
        let row = t.dims.first().copied().unwrap_or(1) as usize;
        let rows = t.elements() / row.max(1);
        if first + count > rows {
            return Err(invalid(format!(
                "{name} has {rows} rows; rows {first}..{} were asked for",
                first + count
            )));
        }
        let row_bytes = ty.bytes_for(row);
        self.file.seek(SeekFrom::Start(
            self.data_start + t.offset + (first * row_bytes) as u64,
        ))?;
        let mut raw = vec![0u8; count * row_bytes];
        self.file.read_exact(&mut raw)?;
        Ok(raw)
    }

    /// Rows `[first, first + count)` of a tensor dequantized to `f32`, whatever its format.
    pub fn dequant_rows(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<f32>> {
        let ty = self.tensor_type(name)?;
        let raw = self.row_bytes(name, first, count)?;
        let mut out = Vec::new();
        ty.dequantize(&raw, &mut out);
        Ok(out)
    }

    /// A whole tensor dequantized to `f32`, whatever its format.
    pub fn dequant_tensor(&mut self, name: &str) -> std::io::Result<Vec<f32>> {
        let t = self.info(name).ok_or_else(|| not_found(name))?.clone();
        let row = t.dims.first().copied().unwrap_or(1) as usize;
        self.dequant_rows(name, 0, t.elements() / row.max(1))
    }

    /// `(in = ne0, out = ne1)` for a 2-D weight tensor (`y = W·x` uses `out×in`).
    pub fn dims2(&self, name: &str) -> Option<(usize, usize)> {
        self.info(name).and_then(|t| match t.dims.as_slice() {
            [ne0, ne1] => Some((*ne0 as usize, *ne1 as usize)),
            _ => None,
        })
    }

    /// The `[ne0, ne1, …]` dims of a tensor (ggml order: `ne0` is the fastest / in-features).
    pub fn dims(&self, name: &str) -> Option<&[u64]> {
        self.info(name).map(|t| t.dims.as_slice())
    }

    /// Every tensor's `(name, dims, ggml_type)` — for inspecting an unfamiliar checkpoint's layout.
    pub fn tensor_names(&self) -> Vec<(String, Vec<u64>, u32)> {
        self.tensors
            .iter()
            .map(|t| (t.name.clone(), t.dims.clone(), t.ggml_type))
            .collect()
    }

    /// `true` if the named tensor exists.
    pub fn has(&self, name: &str) -> bool {
        self.info(name).is_some()
    }

    fn exactly(&self, name: &str, want: GgmlType) -> std::io::Result<TensorInfo> {
        let ty = self.tensor_type(name)?;
        if ty != want {
            return Err(invalid(format!("{name} is {}, not {}", ty.name(), want.name())));
        }
        Ok(self.info(name).ok_or_else(|| not_found(name))?.clone())
    }

    /// Read an **F32** tensor's data in full (the RMSNorm weights).
    pub fn f32_tensor(&mut self, name: &str) -> std::io::Result<Vec<f32>> {
        self.exactly(name, GgmlType::F32)?;
        self.dequant_tensor(name)
    }

    /// Read an **F16** tensor, widened to `f32`. The Qwen3.8 vision tower (`mmproj-*.gguf`) stores every weight
    /// as F16.
    pub fn f16_tensor(&mut self, name: &str) -> std::io::Result<Vec<f32>> {
        self.exactly(name, GgmlType::F16)?;
        self.dequant_tensor(name)
    }

    /// Read an F32 **or** F16 tensor as `f32`, whichever the checkpoint stored.
    pub fn float_tensor(&mut self, name: &str) -> std::io::Result<Vec<f32>> {
        match self.tensor_type(name)? {
            GgmlType::F32 | GgmlType::F16 => self.dequant_tensor(name),
            other => Err(invalid(format!("{name} is {}, not F32/F16", other.name()))),
        }
    }

    /// Read a whole **Q8_0** tensor as `(quants i8 [out*in] row-major, scales f32 [out*in/32])`.
    /// `dequant W[o][t] = quants[o*in+t] * scales[o*nb + t/32]`. Works for 2-D `[in, out]` AND 3-D expert stacks
    /// `[in, out_per_expert, n_expert]` — for the latter `out` folds to `out_per_expert · n_expert`, so
    /// `ffn_gate_exps`/`ffn_up_exps` come out as the stacked `[E·dff, d]` (and `ffn_down_exps` as `[E·d, dff]`)
    /// layout `emit_moe_forward` consumes, expert `e`'s rows at `e·dff`.
    pub fn q8_0_matrix(&mut self, name: &str) -> std::io::Result<(Vec<i8>, Vec<f32>)> {
        let t = self.exactly(name, GgmlType::Q8_0)?;
        let in_f = t.dims[0] as usize;
        let out_f = t.elements() / in_f;
        let raw = self.row_bytes(name, 0, out_f)?;
        let nb = in_f / 32;
        let mut quants = Vec::with_capacity(out_f * in_f);
        let mut scales = Vec::with_capacity(out_f * nb);
        for block in raw.chunks_exact(34) {
            scales.push(f16_to_f32(u16::from_le_bytes([block[0], block[1]])));
            quants.extend(block[2..].iter().map(|&q| q as i8));
        }
        Ok((quants, scales))
    }

    /// Dequantize a single **Q8_0** row to f32 (a token embedding row).
    pub fn q8_0_dequant_row(&mut self, name: &str, row: usize) -> std::io::Result<Vec<f32>> {
        self.exactly(name, GgmlType::Q8_0)?;
        self.dequant_rows(name, row, 1)
    }

    /// Read a `rows` × `cols` **Q8_0** sub-tile as `(quants, scales)`: the int8 values (`rows*cols`, row-major)
    /// AND the per-32-block f32 scales (`rows * cols/32`). The full Q8_0 weight (`dequant = quant * scale`), the
    /// form a real block-scaled GEMM consumes.
    pub fn q8_0_tile_blocks(
        &mut self,
        name: &str,
        rows: usize,
        cols: usize,
    ) -> std::io::Result<(Vec<i32>, Vec<f32>)> {
        let t = self.exactly(name, GgmlType::Q8_0)?;
        let ne0 = t.dims[0] as usize;
        assert!(cols % 32 == 0, "cols must be a multiple of the Q8_0 block (32)");
        assert!(
            cols <= ne0 && rows <= t.dims.get(1).copied().unwrap_or(1) as usize,
            "tile exceeds tensor"
        );
        let raw = self.row_bytes(name, 0, rows)?;
        let row_bytes = ne0 / 32 * 34;
        let mut quants = Vec::with_capacity(rows * cols);
        let mut scales = Vec::with_capacity(rows * cols / 32);
        for r in 0..rows {
            for block in raw[r * row_bytes..].chunks_exact(34).take(cols / 32) {
                scales.push(f16_to_f32(u16::from_le_bytes([block[0], block[1]])));
                quants.extend(block[2..].iter().map(|&q| q as i8 as i32));
            }
        }
        Ok((quants, scales))
    }

    /// The raw int8 weights of the first `rows` × `cols` sub-tile of a **Q8_0** 2-D tensor, row-major over the
    /// tensor's ggml layout. `cols` must be ≤ `ne0` and a multiple of 32, `rows` ≤ `ne1`.
    pub fn q8_0_tile(&mut self, name: &str, rows: usize, cols: usize) -> std::io::Result<Vec<i32>> {
        Ok(self.q8_0_tile_blocks(name, rows, cols)?.0)
    }
}

impl MetaValue {
    /// The GGUF type code of the value.
    pub fn gguf_type(&self) -> u32 {
        match self {
            MetaValue::U8(_) => 0,
            MetaValue::I8(_) => 1,
            MetaValue::U16(_) => 2,
            MetaValue::I16(_) => 3,
            MetaValue::U32(_) => 4,
            MetaValue::I32(_) => 5,
            MetaValue::F32(_) => 6,
            MetaValue::Bool(_) => 7,
            MetaValue::Str(_) => 8,
            MetaValue::Array { .. } => 9,
            MetaValue::U64(_) => 10,
            MetaValue::I64(_) => 11,
            MetaValue::F64(_) => 12,
        }
    }

    fn write_to(&self, out: &mut impl std::io::Write) -> std::io::Result<()> {
        match self {
            MetaValue::U8(v) => out.write_all(&[*v]),
            MetaValue::I8(v) => out.write_all(&[*v as u8]),
            MetaValue::U16(v) => out.write_all(&v.to_le_bytes()),
            MetaValue::I16(v) => out.write_all(&v.to_le_bytes()),
            MetaValue::U32(v) => out.write_all(&v.to_le_bytes()),
            MetaValue::I32(v) => out.write_all(&v.to_le_bytes()),
            MetaValue::F32(v) => out.write_all(&v.to_bits().to_le_bytes()),
            MetaValue::Bool(v) => out.write_all(&[*v as u8]),
            MetaValue::Str(b) => {
                out.write_all(&(b.len() as u64).to_le_bytes())?;
                out.write_all(b)
            }
            MetaValue::Array { ty, items } => {
                out.write_all(&ty.to_le_bytes())?;
                out.write_all(&(items.len() as u64).to_le_bytes())?;
                items.iter().try_for_each(|v| v.write_to(out))
            }
            MetaValue::U64(v) => out.write_all(&v.to_le_bytes()),
            MetaValue::I64(v) => out.write_all(&v.to_le_bytes()),
            MetaValue::F64(v) => out.write_all(&v.to_bits().to_le_bytes()),
        }
    }
}

/// One tensor a written GGUF holds: its name, dims (`dims[0]` the fastest) and format.
#[derive(Clone, Debug)]
pub struct TensorOut {
    pub name: String,
    pub dims: Vec<u64>,
    pub ty: GgmlType,
}

/// Write a GGUF v3 file: the metadata in the order given, the tensor directory, then each tensor's bytes — asked
/// of `data` one tensor at a time, so a checkpoint is never held whole — each aligned to `general.alignment`
/// (32 when absent).
pub fn write(
    path: &std::path::Path,
    meta: &[(String, MetaValue)],
    tensors: &[TensorOut],
    mut data: impl FnMut(usize) -> std::io::Result<Vec<u8>>,
) -> std::io::Result<()> {
    use std::io::Write;
    let align = meta
        .iter()
        .find(|(k, _)| k == "general.alignment")
        .and_then(|(_, v)| v.as_u64())
        .unwrap_or(32);
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    let put_str = |out: &mut std::io::BufWriter<std::fs::File>, s: &[u8]| -> std::io::Result<()> {
        out.write_all(&(s.len() as u64).to_le_bytes())?;
        out.write_all(s)
    };
    out.write_all(b"GGUF")?;
    out.write_all(&3u32.to_le_bytes())?;
    out.write_all(&(tensors.len() as u64).to_le_bytes())?;
    out.write_all(&(meta.len() as u64).to_le_bytes())?;
    for (k, v) in meta {
        put_str(&mut out, k.as_bytes())?;
        out.write_all(&v.gguf_type().to_le_bytes())?;
        v.write_to(&mut out)?;
    }
    let mut offset = 0u64;
    let mut offsets = Vec::with_capacity(tensors.len());
    for t in tensors {
        put_str(&mut out, t.name.as_bytes())?;
        out.write_all(&(t.dims.len() as u32).to_le_bytes())?;
        for d in &t.dims {
            out.write_all(&d.to_le_bytes())?;
        }
        out.write_all(&t.ty.id().to_le_bytes())?;
        out.write_all(&offset.to_le_bytes())?;
        offsets.push(offset);
        let n = t.dims.iter().product::<u64>() as usize;
        offset = (offset + t.ty.bytes_for(n) as u64).div_ceil(align) * align;
    }
    let mut written = 4 + 4 + 8 + 8
        + meta.iter().map(|(k, v)| 8 + k.len() as u64 + 4 + encoded_len(v)).sum::<u64>()
        + tensors.iter().map(|t| 8 + t.name.len() as u64 + 4 + 8 * t.dims.len() as u64 + 4 + 8).sum::<u64>();
    let pad = |out: &mut std::io::BufWriter<std::fs::File>, written: &mut u64| -> std::io::Result<()> {
        let to = written.div_ceil(align) * align;
        out.write_all(&vec![0u8; (to - *written) as usize])?;
        *written = to;
        Ok(())
    };
    pad(&mut out, &mut written)?;
    let data_start = written;
    for (i, t) in tensors.iter().enumerate() {
        debug_assert_eq!(written - data_start, offsets[i], "{} lands where its directory entry says", t.name);
        let bytes = data(i)?;
        let n = t.dims.iter().product::<u64>() as usize;
        if bytes.len() != t.ty.bytes_for(n) {
            return Err(invalid(format!("{}: {} bytes for {n} {} weights", t.name, bytes.len(), t.ty.name())));
        }
        out.write_all(&bytes)?;
        written += bytes.len() as u64;
        if i + 1 < tensors.len() {
            pad(&mut out, &mut written)?;
        }
    }
    out.flush()
}

/// The bytes `v` takes in a GGUF file, its type code excluded.
fn encoded_len(v: &MetaValue) -> u64 {
    match v {
        MetaValue::U8(_) | MetaValue::I8(_) | MetaValue::Bool(_) => 1,
        MetaValue::U16(_) | MetaValue::I16(_) => 2,
        MetaValue::U32(_) | MetaValue::I32(_) | MetaValue::F32(_) => 4,
        MetaValue::U64(_) | MetaValue::I64(_) | MetaValue::F64(_) => 8,
        MetaValue::Str(b) => 8 + b.len() as u64,
        MetaValue::Array { items, .. } => 4 + 8 + items.iter().map(encoded_len).sum::<u64>(),
    }
}
