//! The GGUF weight formats, and their dequantization to `f32`.
//!
//! Each format is a run of fixed-size blocks; a block holds a fixed count of weights and the scales that turn its
//! packed integers back into numbers. The dequantizers here follow ggml's own reference definitions
//! (`ggml/src/ggml-quants.c`, `dequantize_row_*`) operation for operation, in the order those definitions write
//! them, so their output is the reference's to the bit (`model_quant_lock` compiles the reference and compares).

/// A ggml tensor type this crate reads, by its GGUF type id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GgmlType {
    F32,
    F16,
    Bf16,
    Q4_0,
    Q4_1,
    Q5_0,
    Q5_1,
    Q8_0,
    Q2K,
    Q3K,
    Q4K,
    Q5K,
    Q6K,
    Iq4Nl,
    Iq4Xs,
    Mxfp4,
}

/// Weights per super-block of the K-quant formats.
const QK_K: usize = 256;

impl GgmlType {
    /// The type for a GGUF type id, or the id back when this crate does not read that format.
    pub fn from_id(id: u32) -> Result<GgmlType, u32> {
        Ok(match id {
            0 => GgmlType::F32,
            1 => GgmlType::F16,
            2 => GgmlType::Q4_0,
            3 => GgmlType::Q4_1,
            6 => GgmlType::Q5_0,
            7 => GgmlType::Q5_1,
            8 => GgmlType::Q8_0,
            10 => GgmlType::Q2K,
            11 => GgmlType::Q3K,
            12 => GgmlType::Q4K,
            13 => GgmlType::Q5K,
            14 => GgmlType::Q6K,
            20 => GgmlType::Iq4Nl,
            23 => GgmlType::Iq4Xs,
            30 => GgmlType::Bf16,
            39 => GgmlType::Mxfp4,
            other => return Err(other),
        })
    }

    /// The GGUF type id.
    pub fn id(self) -> u32 {
        match self {
            GgmlType::F32 => 0,
            GgmlType::F16 => 1,
            GgmlType::Q4_0 => 2,
            GgmlType::Q4_1 => 3,
            GgmlType::Q5_0 => 6,
            GgmlType::Q5_1 => 7,
            GgmlType::Q8_0 => 8,
            GgmlType::Q2K => 10,
            GgmlType::Q3K => 11,
            GgmlType::Q4K => 12,
            GgmlType::Q5K => 13,
            GgmlType::Q6K => 14,
            GgmlType::Iq4Nl => 20,
            GgmlType::Iq4Xs => 23,
            GgmlType::Bf16 => 30,
            GgmlType::Mxfp4 => 39,
        }
    }

    /// The name ggml gives the type.
    pub fn name(self) -> &'static str {
        match self {
            GgmlType::F32 => "f32",
            GgmlType::F16 => "f16",
            GgmlType::Bf16 => "bf16",
            GgmlType::Q4_0 => "q4_0",
            GgmlType::Q4_1 => "q4_1",
            GgmlType::Q5_0 => "q5_0",
            GgmlType::Q5_1 => "q5_1",
            GgmlType::Q8_0 => "q8_0",
            GgmlType::Q2K => "q2_K",
            GgmlType::Q3K => "q3_K",
            GgmlType::Q4K => "q4_K",
            GgmlType::Q5K => "q5_K",
            GgmlType::Q6K => "q6_K",
            GgmlType::Iq4Nl => "iq4_nl",
            GgmlType::Iq4Xs => "iq4_xs",
            GgmlType::Mxfp4 => "mxfp4",
        }
    }

    /// Weights per block.
    pub fn block_len(self) -> usize {
        match self {
            GgmlType::F32 | GgmlType::F16 | GgmlType::Bf16 => 1,
            GgmlType::Q4_0
            | GgmlType::Q4_1
            | GgmlType::Q5_0
            | GgmlType::Q5_1
            | GgmlType::Q8_0
            | GgmlType::Iq4Nl
            | GgmlType::Mxfp4 => 32,
            GgmlType::Q2K | GgmlType::Q3K | GgmlType::Q4K | GgmlType::Q5K | GgmlType::Q6K | GgmlType::Iq4Xs => QK_K,
        }
    }

    /// Bytes per block.
    pub fn block_bytes(self) -> usize {
        match self {
            GgmlType::F32 => 4,
            GgmlType::F16 | GgmlType::Bf16 => 2,
            GgmlType::Q4_0 => 2 + 16,
            GgmlType::Q4_1 => 2 + 2 + 16,
            GgmlType::Q5_0 => 2 + 4 + 16,
            GgmlType::Q5_1 => 2 + 2 + 4 + 16,
            GgmlType::Q8_0 => 2 + 32,
            GgmlType::Mxfp4 => 1 + 16,
            GgmlType::Q2K => QK_K / 16 + QK_K / 4 + 2 + 2,
            GgmlType::Q3K => QK_K / 8 + QK_K / 4 + 12 + 2,
            GgmlType::Q4K => 2 + 2 + 12 + QK_K / 2,
            GgmlType::Q5K => 2 + 2 + 12 + QK_K / 8 + QK_K / 2,
            GgmlType::Q6K => QK_K / 2 + QK_K / 4 + QK_K / 16 + 2,
            GgmlType::Iq4Nl => 2 + 16,
            GgmlType::Iq4Xs => 2 + 2 + QK_K / 64 + QK_K / 2,
        }
    }

    /// Bytes holding `n` weights (`n` a whole number of blocks).
    pub fn bytes_for(self, n: usize) -> usize {
        assert!(
            n % self.block_len() == 0,
            "{n} weights are not a whole number of {} blocks of {}",
            self.name(),
            self.block_len()
        );
        n / self.block_len() * self.block_bytes()
    }

    /// Dequantize `bytes` (whole blocks) to `f32`, appending to `out`.
    pub fn dequantize(self, bytes: &[u8], out: &mut Vec<f32>) {
        let bb = self.block_bytes();
        assert!(
            bytes.len() % bb == 0,
            "{} bytes are not a whole number of {} blocks",
            bytes.len(),
            self.name()
        );
        out.reserve(bytes.len() / bb * self.block_len());
        for b in bytes.chunks_exact(bb) {
            match self {
                GgmlType::F32 => out.push(f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                GgmlType::F16 => out.push(f16_to_f32(u16::from_le_bytes([b[0], b[1]]))),
                GgmlType::Bf16 => out.push(bf16_to_f32(u16::from_le_bytes([b[0], b[1]]))),
                GgmlType::Q4_0 => q4_0(b, out),
                GgmlType::Q4_1 => q4_1(b, out),
                GgmlType::Q5_0 => q5_0(b, out),
                GgmlType::Q5_1 => q5_1(b, out),
                GgmlType::Q8_0 => q8_0(b, out),
                GgmlType::Mxfp4 => mxfp4(b, out),
                GgmlType::Q2K => q2_k(b, out),
                GgmlType::Q3K => q3_k(b, out),
                GgmlType::Q4K => q4_k(b, out),
                GgmlType::Q5K => q5_k(b, out),
                GgmlType::Q6K => q6_k(b, out),
                GgmlType::Iq4Nl => iq4_nl(b, out),
                GgmlType::Iq4Xs => iq4_xs(b, out),
            }
        }
    }
}

/// `x` rounded to binary16, to nearest with ties to even: subnormals kept, past the largest finite value infinite, a
/// NaN quiet.
pub fn f32_to_f16(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let mant = b & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if mant != 0 { 0x0200 } else { 0 };
    }
    if exp == 0 {
        return sign;
    }
    let e = exp - 127;
    if e > 15 {
        return sign | 0x7c00;
    }
    if e >= -14 {
        let (m, rem) = (mant >> 13, mant & 0x1fff);
        let h = (((e + 15) as u32) << 10) | m;
        let up = rem > 0x1000 || (rem == 0x1000 && m & 1 == 1);
        return sign | (h + u32::from(up)) as u16;
    }
    // A subnormal counts units of 2^-24: the 24-bit significand shifted right by -(e + 1).
    let s = (-(e + 1)) as u32;
    if s > 24 {
        return sign;
    }
    let sig = 0x80_0000 | mant;
    let (n, rem, half) = (sig >> s, sig & ((1u32 << s) - 1), 1u32 << (s - 1));
    let up = rem > half || (rem == half && n & 1 == 1);
    sign | (n + u32::from(up)) as u16
}

/// IEEE-754 binary16 → binary32, exactly (subnormals included).
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1f) as u32;
    let mant = (h & 0x3ff) as u32;
    let bits = match (exp, mant) {
        (0, 0) => sign << 31,
        (0, _) => {
            let mut e: i32 = -1;
            let mut m = mant;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            (sign << 31) | (((e + 1 + 127 - 14) as u32) << 23) | ((m & 0x3ff) << 13)
        }
        (0x1f, 0) => (sign << 31) | 0x7f80_0000,
        (0x1f, _) => (sign << 31) | 0x7fc0_0000 | (mant << 13),
        _ => (sign << 31) | ((exp + 127 - 15) << 23) | (mant << 13),
    };
    f32::from_bits(bits)
}

/// bfloat16 → binary32: the high half of the word.
pub fn bf16_to_f32(h: u16) -> f32 {
    f32::from_bits((h as u32) << 16)
}

/// `x` rounded to bfloat16, to nearest with ties to even; a NaN stays a quiet NaN.
pub fn f32_to_bf16(x: f32) -> u16 {
    let b = x.to_bits();
    if x.is_nan() {
        return ((b >> 16) | 0x0040) as u16;
    }
    ((b + 0x7fff + ((b >> 16) & 1)) >> 16) as u16
}

/// An `e4m3fn` float's value as `torch` widens it: bias 7, no infinities, `S.1111.111` a NaN keeping its sign and
/// mantissa bits.
pub fn e4m3fn_to_f32(b: u8) -> f32 {
    let sign = u32::from(b >> 7) << 31;
    let (e, m) = (u32::from((b >> 3) & 0xf), u32::from(b & 7));
    if e == 0xf && m == 7 {
        return f32::from_bits(sign | 0x7f80_0000 | m << 20);
    }
    let magnitude = if e == 0 { m as f32 / 8.0 * 2f32.powi(-6) } else { (1.0 + m as f32 / 8.0) * 2f32.powi(e as i32 - 7) };
    f32::from_bits(sign | magnitude.to_bits())
}

/// An `e5m2` float's value: the high byte of a binary16.
pub fn e5m2_to_f32(b: u8) -> f32 {
    f16_to_f32(u16::from(b) << 8)
}

/// The scale of a scaled-float tensor (an FP8 checkpoint's `weight_scale_inv`) broadcast over the weight's elements, as
/// the Hugging Face converter's `dequant_simple` broadcasts it before `weight.float() * scale`: with a block size, each
/// of the scale's trailing axes repeated a block's length and cropped to the weight's; then axes of length 1 appended
/// until the scale has the weight's rank, every axis either 1 (broadcast) or the weight's length.
#[derive(Clone, Debug)]
pub struct ScaleGrid {
    weight: Vec<usize>,
    values: Vec<f32>,
    /// Per weight axis: the divisor of its index and the stored scale's stride on it, or `None` where it broadcasts.
    axes: Vec<Option<(usize, usize)>>,
}

impl ScaleGrid {
    /// The grid of a scale of axes `dims` and row-major `values` over a weight of axes `weight`, its trailing axes in
    /// blocks of `block` where one is given. A shape `dequant_simple` would not broadcast is refused.
    pub fn new(weight: &[usize], dims: &[usize], values: Vec<f32>, block: Option<&[usize]>) -> Result<ScaleGrid, String> {
        let count: usize = dims.iter().product();
        if values.len() != count {
            return Err(format!("a scale of axes {dims:?} holds {count} values, not {}", values.len()));
        }
        if dims.len() > weight.len() {
            return Err(format!("a scale of axes {dims:?} has more axes than its weight's {weight:?}"));
        }
        let repeat: Vec<usize> = match block {
            None => vec![1; dims.len()],
            Some(b) if b.len() <= dims.len() && b.iter().all(|&x| x > 0) => {
                let offset = dims.len() - b.len();
                (0..dims.len()).map(|d| if d < offset { 1 } else { b[d - offset] }).collect()
            }
            Some(b) => return Err(format!("a block of {b:?} over a scale of axes {dims:?}")),
        };
        let mut stride = vec![1usize; dims.len()];
        for d in (0..dims.len().saturating_sub(1)).rev() {
            stride[d] = stride[d + 1] * dims[d + 1];
        }
        let mut axes = Vec::with_capacity(weight.len());
        for (d, &w) in weight.iter().enumerate() {
            let expanded = if d < dims.len() { (dims[d] * repeat[d]).min(w) } else { 1 };
            axes.push(if expanded == w && w != 1 {
                Some((repeat[d], stride[d]))
            } else if expanded == 1 {
                None
            } else {
                return Err(format!(
                    "a scale of axes {dims:?} in blocks of {block:?} spans {expanded} on axis {d} of a weight of {weight:?}"
                ));
            });
        }
        Ok(ScaleGrid { weight: weight.to_vec(), values, axes })
    }

    /// `values`, the weight's elements from `first` on in row-major order widened to f32, each multiplied by its scale.
    pub fn apply(&self, first: usize, values: &mut [f32]) {
        let mut index = vec![0usize; self.weight.len()];
        let mut rest = first;
        for d in (0..self.weight.len()).rev() {
            index[d] = rest % self.weight[d].max(1);
            rest /= self.weight[d].max(1);
        }
        for v in values.iter_mut() {
            let at: usize = self.axes.iter().zip(&index).map(|(a, &i)| a.map_or(0, |(div, stride)| i / div * stride)).sum();
            *v *= self.values[at];
            for d in (0..index.len()).rev() {
                index[d] += 1;
                if index[d] < self.weight[d] {
                    break;
                }
                index[d] = 0;
            }
        }
    }
}

fn half(b: &[u8], at: usize) -> f32 {
    f16_to_f32(u16::from_le_bytes([b[at], b[at + 1]]))
}

fn q4_0(b: &[u8], y: &mut Vec<f32>) {
    let d = half(b, 0);
    let qs = &b[2..18];
    let (lo, hi): (Vec<f32>, Vec<f32>) = qs
        .iter()
        .map(|&q| (((q & 0x0f) as i32 - 8) as f32 * d, ((q >> 4) as i32 - 8) as f32 * d))
        .unzip();
    y.extend(lo);
    y.extend(hi);
}

fn q4_1(b: &[u8], y: &mut Vec<f32>) {
    let (d, m) = (half(b, 0), half(b, 2));
    let qs = &b[4..20];
    y.extend(qs.iter().map(|&q| (q & 0x0f) as f32 * d + m));
    y.extend(qs.iter().map(|&q| (q >> 4) as f32 * d + m));
}

fn q5_0(b: &[u8], y: &mut Vec<f32>) {
    let d = half(b, 0);
    let qh = u32::from_le_bytes([b[2], b[3], b[4], b[5]]);
    let qs = &b[6..22];
    let x = |j: usize, q: u8| -> (f32, f32) {
        let xh0 = (((qh >> j) << 4) & 0x10) as u8;
        let xh1 = ((qh >> (j + 12)) & 0x10) as u8;
        (
            (((q & 0x0f) | xh0) as i32 - 16) as f32 * d,
            (((q >> 4) | xh1) as i32 - 16) as f32 * d,
        )
    };
    let (lo, hi): (Vec<f32>, Vec<f32>) = qs.iter().enumerate().map(|(j, &q)| x(j, q)).unzip();
    y.extend(lo);
    y.extend(hi);
}

fn q5_1(b: &[u8], y: &mut Vec<f32>) {
    let (d, m) = (half(b, 0), half(b, 2));
    let qh = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    let qs = &b[8..24];
    let x = |j: usize, q: u8| -> (f32, f32) {
        let xh0 = (((qh >> j) << 4) & 0x10) as u8;
        let xh1 = ((qh >> (j + 12)) & 0x10) as u8;
        (
            ((q & 0x0f) | xh0) as f32 * d + m,
            ((q >> 4) | xh1) as f32 * d + m,
        )
    };
    let (lo, hi): (Vec<f32>, Vec<f32>) = qs.iter().enumerate().map(|(j, &q)| x(j, q)).unzip();
    y.extend(lo);
    y.extend(hi);
}

fn q8_0(b: &[u8], y: &mut Vec<f32>) {
    let d = half(b, 0);
    y.extend(b[2..34].iter().map(|&q| (q as i8) as f32 * d));
}

/// The non-linear levels an IQ4 nibble indexes (ggml's `kvalues_iq4nl`).
pub const KVALUES_IQ4NL: [i8; 16] = [-127, -104, -83, -65, -49, -35, -22, -10, 1, 13, 25, 38, 53, 69, 89, 113];

fn iq4_nl(b: &[u8], y: &mut Vec<f32>) {
    let d = half(b, 0);
    let qs = &b[2..18];
    y.extend(qs.iter().map(|&q| d * KVALUES_IQ4NL[(q & 0x0f) as usize] as f32));
    y.extend(qs.iter().map(|&q| d * KVALUES_IQ4NL[(q >> 4) as usize] as f32));
}

fn iq4_xs(b: &[u8], y: &mut Vec<f32>) {
    let d = half(b, 0);
    let scales_h = u16::from_le_bytes([b[2], b[3]]);
    let scales_l = &b[4..8];
    let qs = &b[8..136];
    for ib in 0..QK_K / 32 {
        let ls = ((scales_l[ib / 2] >> (4 * (ib % 2))) & 0xf) as i32 | ((((scales_h >> (2 * ib)) & 3) as i32) << 4);
        let dl = d * (ls - 32) as f32;
        let q = &qs[16 * ib..16 * ib + 16];
        y.extend(q.iter().map(|&v| dl * KVALUES_IQ4NL[(v & 0x0f) as usize] as f32));
        y.extend(q.iter().map(|&v| dl * KVALUES_IQ4NL[(v >> 4) as usize] as f32));
    }
}

/// The doubled E2M1 values of an MXFP4 nibble (ggml's `kvalues_mxfp4`).
const KVALUES_MXFP4: [i8; 16] = [0, 1, 2, 3, 4, 6, 8, 12, 0, -1, -2, -3, -4, -6, -8, -12];

/// Half of the E8M0 scale `2^(x-127)` — ggml's `ggml_e8m0_to_fp32_half`, halved to undo the doubled table.
fn e8m0_half(x: u8) -> f32 {
    let bits = if x < 2 {
        0x0020_0000u32 << x
    } else {
        ((x - 1) as u32) << 23
    };
    f32::from_bits(bits)
}

fn mxfp4(b: &[u8], y: &mut Vec<f32>) {
    let d = e8m0_half(b[0]);
    let qs = &b[1..17];
    y.extend(qs.iter().map(|&q| KVALUES_MXFP4[(q & 0x0f) as usize] as f32 * d));
    y.extend(qs.iter().map(|&q| KVALUES_MXFP4[(q >> 4) as usize] as f32 * d));
}

fn q2_k(b: &[u8], y: &mut Vec<f32>) {
    let scales = &b[0..16];
    let qs = &b[16..80];
    let (d, min) = (half(b, 80), half(b, 82));
    let mut is = 0;
    for n in (0..QK_K).step_by(128) {
        let q = &qs[n / 4..n / 4 + 32];
        let mut shift = 0;
        for _ in 0..4 {
            for half_block in [0usize, 16] {
                let sc = scales[is];
                is += 1;
                let (dl, ml) = (d * (sc & 0xf) as f32, min * (sc >> 4) as f32);
                for l in 0..16 {
                    y.push(dl * (((q[l + half_block] >> shift) & 3) as i8) as f32 - ml);
                }
            }
            shift += 2;
        }
    }
}

fn q3_k(b: &[u8], y: &mut Vec<f32>) {
    let hm = &b[0..32];
    let qs = &b[32..96];
    let packed = &b[96..108];
    let d_all = half(b, 108);
    let (kmask1, kmask2) = (0x0303_0303u32, 0x0f0f_0f0fu32);
    let mut aux = [0u32; 4];
    for (k, a) in aux.iter_mut().take(3).enumerate() {
        *a = u32::from_le_bytes([
            packed[4 * k],
            packed[4 * k + 1],
            packed[4 * k + 2],
            packed[4 * k + 3],
        ]);
    }
    let tmp = aux[2];
    aux[2] = ((aux[0] >> 4) & kmask2) | (((tmp >> 4) & kmask1) << 4);
    aux[3] = ((aux[1] >> 4) & kmask2) | (((tmp >> 6) & kmask1) << 4);
    aux[0] = (aux[0] & kmask2) | ((tmp & kmask1) << 4);
    aux[1] = (aux[1] & kmask2) | (((tmp >> 2) & kmask1) << 4);
    let scales: Vec<i8> = aux
        .iter()
        .flat_map(|a| a.to_le_bytes())
        .map(|v| v as i8)
        .collect();
    let mut is = 0;
    let mut m = 1u8;
    for n in (0..QK_K).step_by(128) {
        let q = &qs[n / 4..n / 4 + 32];
        let mut shift = 0;
        for _ in 0..4 {
            for half_block in [0usize, 16] {
                let dl = d_all * (scales[is] as i32 - 32) as f32;
                is += 1;
                for l in 0..16 {
                    let low = ((q[l + half_block] >> shift) & 3) as i8;
                    let high = if hm[l + half_block] & m != 0 { 0 } else { 4 };
                    y.push(dl * (low - high) as f32);
                }
            }
            shift += 2;
            m <<= 1;
        }
    }
}

/// ggml's `get_scale_min_k4`: the 6-bit scale and min of sub-block `j` from a K-quant's 12 packed bytes.
fn scale_min_k4(j: usize, q: &[u8]) -> (u8, u8) {
    if j < 4 {
        (q[j] & 63, q[j + 4] & 63)
    } else {
        (
            (q[j + 4] & 0xf) | ((q[j - 4] >> 6) << 4),
            (q[j + 4] >> 4) | ((q[j] >> 6) << 4),
        )
    }
}

fn q4_k(b: &[u8], y: &mut Vec<f32>) {
    let (d, min) = (half(b, 0), half(b, 2));
    let scales = &b[4..16];
    let qs = &b[16..144];
    let mut is = 0;
    for j in (0..QK_K).step_by(64) {
        let q = &qs[j / 2..j / 2 + 32];
        let (sc, m) = scale_min_k4(is, scales);
        let (d1, m1) = (d * sc as f32, min * m as f32);
        let (sc, m) = scale_min_k4(is + 1, scales);
        let (d2, m2) = (d * sc as f32, min * m as f32);
        y.extend(q.iter().map(|&v| d1 * (v & 0xf) as f32 - m1));
        y.extend(q.iter().map(|&v| d2 * (v >> 4) as f32 - m2));
        is += 2;
    }
}

fn q5_k(b: &[u8], y: &mut Vec<f32>) {
    let (d, min) = (half(b, 0), half(b, 2));
    let scales = &b[4..16];
    let qh = &b[16..48];
    let qs = &b[48..176];
    let mut is = 0;
    let (mut u1, mut u2) = (1u8, 2u8);
    for j in (0..QK_K).step_by(64) {
        let ql = &qs[j / 2..j / 2 + 32];
        let (sc, m) = scale_min_k4(is, scales);
        let (d1, m1) = (d * sc as f32, min * m as f32);
        let (sc, m) = scale_min_k4(is + 1, scales);
        let (d2, m2) = (d * sc as f32, min * m as f32);
        for l in 0..32 {
            let hi = if qh[l] & u1 != 0 { 16 } else { 0 };
            y.push(d1 * ((ql[l] & 0xf) as i32 + hi) as f32 - m1);
        }
        for l in 0..32 {
            let hi = if qh[l] & u2 != 0 { 16 } else { 0 };
            y.push(d2 * ((ql[l] >> 4) as i32 + hi) as f32 - m2);
        }
        is += 2;
        u1 <<= 2;
        u2 <<= 2;
    }
}

fn q6_k(b: &[u8], y: &mut Vec<f32>) {
    let ql_all = &b[0..128];
    let qh_all = &b[128..192];
    let sc_all: Vec<i8> = b[192..208].iter().map(|&v| v as i8).collect();
    let d = half(b, 208);
    let mut block = [0f32; QK_K];
    for (part, n) in (0..QK_K).step_by(128).enumerate() {
        let ql = &ql_all[part * 64..part * 64 + 64];
        let qh = &qh_all[part * 32..part * 32 + 32];
        let sc = &sc_all[part * 8..part * 8 + 8];
        for l in 0..32 {
            let is = l / 16;
            let q1 = (((ql[l] & 0xf) | ((qh[l] & 3) << 4)) as i8) - 32;
            let q2 = (((ql[l + 32] & 0xf) | (((qh[l] >> 2) & 3) << 4)) as i8) - 32;
            let q3 = (((ql[l] >> 4) | (((qh[l] >> 4) & 3) << 4)) as i8) - 32;
            let q4 = (((ql[l + 32] >> 4) | (((qh[l] >> 6) & 3) << 4)) as i8) - 32;
            block[n + l] = d * sc[is] as f32 * q1 as f32;
            block[n + l + 32] = d * sc[is + 2] as f32 * q2 as f32;
            block[n + l + 64] = d * sc[is + 4] as f32 * q3 as f32;
            block[n + l + 96] = d * sc[is + 6] as f32 * q4 as f32;
        }
    }
    y.extend_from_slice(&block);
}
