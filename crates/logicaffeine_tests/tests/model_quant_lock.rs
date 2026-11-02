//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! EVERY WEIGHT FORMAT A MODEL SHIPS IN, DEQUANTIZED EXACTLY AS GGML DEFINES IT.
//!
//! `model::quant` reads F32, F16, BF16, Q4_0, Q4_1, Q5_0, Q5_1, Q8_0, Q2_K, Q3_K, Q4_K, Q5_K, Q6_K, IQ4_NL, IQ4_XS and
//! MXFP4 — the formats Llama, Qwen, Gemma, gpt-oss and DeepSeek checkpoints are distributed in. A dequantizer that is
//! off by a scale, a nibble order or a sign still produces plausible numbers, so this lock holds them three ways:
//!
//!   • every format's block is the size ggml's `static_assert`s say it is;
//!   • hand-built blocks with known contents decode to the values worked out by hand;
//!   • random blocks decode to ggml's own `dequantize_row_*` — the reference compiled from the pinned llama.cpp
//!     source and compared bit for bit. The reference is compiled with `-ffp-contract=off`: GCC otherwise fuses
//!     `d*q - m` into one rounding, which the C source does not write and Rust never does.
//!
//! FP8 checkpoints (DeepSeek-V3 and R1, Kimi K2, MiniMax-M2, the FP8 releases of GLM-4.5 and Qwen3) store 8-bit floats
//! beside their scales; `quant::ScaleGrid` widens them as the Hugging Face converter's own `dequant_simple` does, bit
//! for bit (the oracle runs that function out of the pinned converter's source).
//!
//! And the reader: the local Qwen3-8B checkpoint's metadata reads in full (its tokenizer included), and its Q8_0
//! tensors decode through the generic path to exactly the values the Q8_0 accessor gives.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;

use std::io::Write;
use std::process::{Command, Stdio};

use logicaffeine_compile::model::gguf::{Gguf, MetaValue};
use logicaffeine_compile::model::quant::{bf16_to_f32, e4m3fn_to_f32, e5m2_to_f32, f32_to_bf16, GgmlType, ScaleGrid};

const FORMATS: &[GgmlType] = &[
    GgmlType::F32,
    GgmlType::F16,
    GgmlType::Bf16,
    GgmlType::Q4_0,
    GgmlType::Q4_1,
    GgmlType::Q5_0,
    GgmlType::Q5_1,
    GgmlType::Q8_0,
    GgmlType::Q2K,
    GgmlType::Q3K,
    GgmlType::Q4K,
    GgmlType::Q5K,
    GgmlType::Q6K,
    GgmlType::Iq4Nl,
    GgmlType::Iq4Xs,
    GgmlType::Mxfp4,
];

fn decode(ty: GgmlType, bytes: &[u8]) -> Vec<f32> {
    let mut out = Vec::new();
    ty.dequantize(bytes, &mut out);
    out
}

#[test]
fn every_format_has_ggmls_block_size() {
    let want = [
        (GgmlType::F32, 1, 4),
        (GgmlType::F16, 1, 2),
        (GgmlType::Bf16, 1, 2),
        (GgmlType::Q4_0, 32, 18),
        (GgmlType::Q4_1, 32, 20),
        (GgmlType::Q5_0, 32, 22),
        (GgmlType::Q5_1, 32, 24),
        (GgmlType::Q8_0, 32, 34),
        (GgmlType::Q2K, 256, 84),
        (GgmlType::Q3K, 256, 110),
        (GgmlType::Q4K, 256, 144),
        (GgmlType::Q5K, 256, 176),
        (GgmlType::Q6K, 256, 210),
        (GgmlType::Iq4Nl, 32, 18),
        (GgmlType::Iq4Xs, 256, 136),
        (GgmlType::Mxfp4, 32, 17),
    ];
    assert_eq!(want.len(), FORMATS.len(), "every format is sized");
    for (ty, len, bytes) in want {
        assert_eq!((ty.block_len(), ty.block_bytes()), (len, bytes), "{}", ty.name());
        assert_eq!(GgmlType::from_id(ty.id()), Ok(ty), "{} round-trips its id", ty.name());
    }
    assert_eq!(GgmlType::from_id(16), Err(16), "an unread format is refused by its id");
}

#[test]
fn known_blocks_decode_to_their_values() {
    // f16 1.0 = 0x3C00, 0.5 = 0x3800, 2.0 = 0x4000.
    let mut q8 = vec![0x00, 0x38];
    q8.extend((0..32).map(|i| (i as i8 - 16) as u8));
    assert_eq!(
        decode(GgmlType::Q8_0, &q8),
        (0..32).map(|i| (i - 16) as f32 * 0.5).collect::<Vec<_>>(),
        "q8_0: q · d"
    );

    let mut q4 = vec![0x00, 0x40];
    q4.extend((0..16u8).map(|j| j | ((15 - j) << 4)));
    let mut want: Vec<f32> = (0..16).map(|j| (j as f32 - 8.0) * 2.0).collect();
    want.extend((0..16).map(|j| ((15 - j) as f32 - 8.0) * 2.0));
    assert_eq!(decode(GgmlType::Q4_0, &q4), want, "q4_0: low nibbles first, then high, offset 8");

    let mut q41 = vec![0x00, 0x3c, 0x00, 0x40];
    q41.extend([0x21u8; 16]);
    let mut want: Vec<f32> = vec![1.0 + 2.0; 16];
    want.extend(vec![2.0 + 2.0; 16]);
    assert_eq!(decode(GgmlType::Q4_1, &q41), want, "q4_1: q · d + m");

    let mut q50 = vec![0x00, 0x3c];
    q50.extend(0xffff_ffffu32.to_le_bytes());
    q50.extend([0x00u8; 16]);
    assert_eq!(
        decode(GgmlType::Q5_0, &q50),
        vec![0.0f32; 32],
        "q5_0: the fifth bit lifts 0 to 16, and 16 - 16 is 0"
    );

    let mut mx = vec![127u8];
    mx.extend((0..16u8).map(|j| j | (j << 4)));
    let table = [0.0f32, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, 0.0, -0.5, -1.0, -1.5, -2.0, -3.0, -4.0, -6.0];
    let mut want = table.to_vec();
    want.extend(table);
    assert_eq!(
        decode(GgmlType::Mxfp4, &mx),
        want,
        "mxfp4: the E2M1 values at scale 2^0"
    );

    assert_eq!(decode(GgmlType::F16, &[0x00, 0x3c, 0x01, 0x00]), vec![1.0, 2f32.powi(-24)], "f16 incl. a subnormal");
    assert_eq!(decode(GgmlType::Bf16, &[0x80, 0x3f]), vec![1.0], "bf16 is the high half of an f32");

    // q4_K: d = 1, dmin = 1; sub-block scales 1 and mins 0 (scale bytes 0..4 = 1, 4..8 = 0, the rest 0);
    // every quant 3 → sub-blocks 0..3 (values 0..128) are 3, sub-blocks 4..7 have scale 0 → 0.
    let mut q4k = vec![0x00, 0x3c, 0x00, 0x3c, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0];
    q4k.extend([0x33u8; 128]);
    let got = decode(GgmlType::Q4K, &q4k);
    assert_eq!(&got[..128], &[3.0f32; 128][..], "q4_K: sub-blocks 0..3 scale 1");
    assert_eq!(&got[128..], &[0.0f32; 128][..], "q4_K: sub-blocks 4..7 scale 0");

    // iq4_nl: d = 1, each nibble indexing a level of the non-linear table, low nibbles first.
    let levels = [-127.0f32, -104.0, -83.0, -65.0, -49.0, -35.0, -22.0, -10.0, 1.0, 13.0, 25.0, 38.0, 53.0, 69.0, 89.0, 113.0];
    let mut nl = vec![0x00, 0x3c];
    nl.extend((0..16u8).map(|j| j | ((15 - j) << 4)));
    let mut want: Vec<f32> = levels.to_vec();
    want.extend(levels.iter().rev());
    assert_eq!(decode(GgmlType::Iq4Nl, &nl), want, "iq4_nl: d · level");

    // iq4_xs: d = 0.5; every sub-block's high scale bits 2 (scales_h 0xaaaa), sub-block 0's low nibble 1, the others
    // 0: scales 33, 32, 32, …; every nibble 8, the level 1. Sub-block 0 is 0.5 · (33 − 32) · 1, the rest 0.
    let mut xs = vec![0x00, 0x38, 0xaa, 0xaa, 0x01, 0x00, 0x00, 0x00];
    xs.extend([0x88u8; 128]);
    let got = decode(GgmlType::Iq4Xs, &xs);
    assert_eq!(&got[..32], &[0.5f32; 32][..], "iq4_xs: sub-block 0 at scale 33 − 32");
    assert_eq!(&got[32..], &[0.0f32; 224][..], "iq4_xs: the others at scale 32 − 32");
}

/// A deterministic byte stream (xorshift64*).
struct Bytes(u64);
impl Bytes {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn fill(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
    /// A finite f16, both signs, normals and subnormals.
    fn finite_f16(&mut self) -> [u8; 2] {
        let v = self.next() as u16;
        let exp = ((v >> 10) & 0x1f) % 31;
        ((v & 0x83ff) | (exp << 10)).to_le_bytes()
    }
}

/// Random blocks of `ty` whose f16 fields are finite (NaN payloads are not part of any format's meaning).
fn random_blocks(ty: GgmlType, blocks: usize, seed: u64) -> Vec<u8> {
    let mut r = Bytes(seed);
    let mut out = Vec::new();
    let half_fields: &[usize] = match ty {
        GgmlType::Q4_0 | GgmlType::Q5_0 | GgmlType::Q8_0 | GgmlType::Iq4Nl | GgmlType::Iq4Xs => &[0],
        GgmlType::Q4_1 | GgmlType::Q5_1 | GgmlType::Q4K | GgmlType::Q5K => &[0, 2],
        GgmlType::Q2K => &[80, 82],
        GgmlType::Q3K => &[108],
        GgmlType::Q6K => &[208],
        _ => &[],
    };
    for _ in 0..blocks {
        let mut b = r.fill(ty.block_bytes());
        match ty {
            GgmlType::F16 => b.copy_from_slice(&r.finite_f16()),
            GgmlType::F32 => {
                let exp = (r.next() % 254) as u32 + 1;
                let bits = (r.next() as u32 & 0x807f_ffff) | (exp << 23);
                b.copy_from_slice(&bits.to_le_bytes());
            }
            GgmlType::Bf16 => {
                let exp = (r.next() % 254) as u16 + 1;
                let bits = (r.next() as u16 & 0x807f) | (exp << 7);
                b.copy_from_slice(&bits.to_le_bytes());
            }
            _ => {
                for &at in half_fields {
                    let h = r.finite_f16();
                    b[at..at + 2].copy_from_slice(&h);
                }
            }
        }
        out.extend(b);
    }
    out
}

const HARNESS: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <stdarg.h>
#include "ggml.h"
#include "ggml-impl.h"
#include "ggml-quants.h"
void ggml_abort(const char * file, int line, const char * fmt, ...) { fprintf(stderr, "abort %s:%d\n", file, line); abort(); }
size_t ggml_row_size(enum ggml_type type, int64_t ne) { (void)type; (void)ne; abort(); }
const char * ggml_type_name(enum ggml_type type) { (void)type; abort(); }
size_t ggml_type_size(enum ggml_type type) { (void)type; abort(); }
int main(void) {
    uint32_t id; uint64_t n, nbytes;
    if (fread(&id, 4, 1, stdin) != 1 || fread(&n, 8, 1, stdin) != 1 || fread(&nbytes, 8, 1, stdin) != 1) return 2;
    uint8_t * x = malloc(nbytes); float * y = malloc(n * sizeof(float));
    if (fread(x, 1, nbytes, stdin) != nbytes) return 3;
    switch (id) {
        case 0: for (uint64_t i = 0; i < n; ++i) y[i] = ((const float *) x)[i]; break;
        case 1: for (uint64_t i = 0; i < n; ++i) y[i] = GGML_FP16_TO_FP32(((const ggml_fp16_t *) x)[i]); break;
        case 30: for (uint64_t i = 0; i < n; ++i) y[i] = GGML_BF16_TO_FP32(((const ggml_bf16_t *) x)[i]); break;
        case 2: dequantize_row_q4_0((const void *) x, y, n); break;
        case 3: dequantize_row_q4_1((const void *) x, y, n); break;
        case 6: dequantize_row_q5_0((const void *) x, y, n); break;
        case 7: dequantize_row_q5_1((const void *) x, y, n); break;
        case 8: dequantize_row_q8_0((const void *) x, y, n); break;
        case 10: dequantize_row_q2_K((const void *) x, y, n); break;
        case 11: dequantize_row_q3_K((const void *) x, y, n); break;
        case 12: dequantize_row_q4_K((const void *) x, y, n); break;
        case 13: dequantize_row_q5_K((const void *) x, y, n); break;
        case 14: dequantize_row_q6_K((const void *) x, y, n); break;
        case 20: dequantize_row_iq4_nl((const void *) x, y, n); break;
        case 23: dequantize_row_iq4_xs((const void *) x, y, n); break;
        case 39: dequantize_row_mxfp4((const void *) x, y, n); break;
        default: return 4;
    }
    fwrite(y, sizeof(float), n, stdout);
    return 0;
}
"#;

/// A scaled-float case: the weight's dtype, axes and stored bytes, the scale's, and the block size.
struct ScaledCase {
    weight_dtype: &'static str,
    weight_dims: Vec<usize>,
    weight: Vec<u8>,
    scale_dtype: &'static str,
    scale_dims: Vec<usize>,
    scale: Vec<u8>,
    block: Option<Vec<usize>>,
}

#[test]
fn fp8_block_scaled_dequantizes_as_the_reference() {
    if !(common::require_tool("torch") && common::require_tool("competitor-sources")) {
        return;
    }
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    // Every code first, then random ones; scales of both signs over sixty binades, f32 or bf16.
    let mut case = |weight_dtype: &'static str, weight_dims: &[usize], scale_dtype: &'static str, scale_dims: &[usize], block: Option<&[usize]>| {
        let n: usize = weight_dims.iter().product();
        let width = if weight_dtype == "bfloat16" { 2 } else { 1 };
        let weight: Vec<u8> = (0..n * width).map(|i| if i < 256 { i as u8 } else { next() as u8 }).collect();
        let scales: Vec<f32> = (0..scale_dims.iter().product::<usize>())
            .map(|_| {
                let unit = (next() >> 40) as f32 / (1u64 << 24) as f32;
                (unit - 0.5) * 2f32.powi((next() % 60) as i32 - 40)
            })
            .collect();
        let scale = match scale_dtype {
            "bfloat16" => scales.iter().flat_map(|&s| f32_to_bf16(s).to_le_bytes()).collect(),
            _ => scales.iter().flat_map(|s| s.to_le_bytes()).collect(),
        };
        ScaledCase {
            weight_dtype,
            weight_dims: weight_dims.to_vec(),
            weight,
            scale_dtype,
            scale_dims: scale_dims.to_vec(),
            scale,
            block: block.map(<[usize]>::to_vec),
        }
    };
    let cases = [
        // 128×128 blocks, as every FP8 release states them, over a matrix of one block and of several.
        case("float8_e4m3fn", &[96, 160], "float32", &[1, 2], Some(&[128, 128])),
        case("float8_e4m3fn", &[256, 384], "float32", &[2, 3], Some(&[128, 128])),
        // Blocks that divide neither axis: the repeated scale cropped to the weight.
        case("float8_e4m3fn", &[50, 70], "float32", &[4, 3], Some(&[16, 24])),
        case("float8_e5m2", &[64, 96], "float32", &[4, 4], Some(&[16, 24])),
        // A stack of experts, the block over its two trailing axes.
        case("float8_e4m3fn", &[3, 32, 48], "float32", &[3, 2, 2], Some(&[16, 24])),
        // Scales stored in bf16, and a bf16 weight scaled.
        case("float8_e4m3fn", &[64, 96], "bfloat16", &[4, 4], Some(&[16, 24])),
        case("bfloat16", &[32, 48], "float32", &[2, 2], Some(&[16, 24])),
        // No block size: a scale per row, and one for the whole tensor.
        case("float8_e4m3fn", &[32, 40], "float32", &[32], None),
        case("float8_e4m3fn", &[32, 40], "float32", &[], None),
    ];
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let json: Vec<serde_json::Value> = cases
        .iter()
        .map(|c| {
            serde_json::json!({
                "weight": hex(&c.weight), "weight_dtype": c.weight_dtype, "weight_dims": c.weight_dims,
                "scale": hex(&c.scale), "scale_dtype": c.scale_dtype, "scale_dims": c.scale_dims, "block": c.block,
            })
        })
        .collect();
    let dir = std::env::temp_dir().join(format!("logos_scaled_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("cases.json");
    std::fs::write(&input, serde_json::to_string(&json).unwrap()).unwrap();
    let out = Command::new(common::python_for_tools())
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/accel/hf-oracle.py"))
        .arg("scaled")
        .arg(&input)
        .output()
        .expect("run the oracle");
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(out.status.success(), "the oracle failed: {}", String::from_utf8_lossy(&out.stderr));
    let reference: Vec<Vec<u32>> = serde_json::from_slice(&out.stdout).expect("the oracle's JSON");
    for (c, want) in cases.iter().zip(&reference) {
        let what = format!("{} {:?} scaled by {} {:?} in blocks of {:?}", c.weight_dtype, c.weight_dims, c.scale_dtype, c.scale_dims, c.block);
        let widen = |bytes: &[u8], dtype: &str| -> Vec<f32> {
            match dtype {
                "float8_e4m3fn" => bytes.iter().map(|&b| e4m3fn_to_f32(b)).collect(),
                "float8_e5m2" => bytes.iter().map(|&b| e5m2_to_f32(b)).collect(),
                "bfloat16" => bytes.chunks_exact(2).map(|h| bf16_to_f32(u16::from_le_bytes([h[0], h[1]]))).collect(),
                _ => bytes.chunks_exact(4).map(|f| f32::from_le_bytes([f[0], f[1], f[2], f[3]])).collect(),
            }
        };
        let grid = ScaleGrid::new(&c.weight_dims, &c.scale_dims, widen(&c.scale, c.scale_dtype), c.block.as_deref())
            .unwrap_or_else(|e| panic!("{what}: {e}"));
        let values = widen(&c.weight, c.weight_dtype);
        // Whole, and in three runs from inside rows, as a reader asks for rows.
        let n = values.len();
        let cuts = [0, n / 3 + 1, 2 * n / 3 + 5, n];
        for runs in [vec![0, n], cuts.to_vec()] {
            let mut ours = Vec::with_capacity(n);
            for w in runs.windows(2) {
                let mut part = values[w[0]..w[1]].to_vec();
                grid.apply(w[0], &mut part);
                ours.extend(part);
            }
            let ours: Vec<u32> = ours.iter().map(|v| v.to_bits()).collect();
            assert_eq!(ours.len(), want.len(), "{what}: element count");
            if let Some(i) = ours.iter().zip(want).position(|(a, b)| a != b) {
                panic!("{what}: element {i} is {} here and {} in the converter", f32::from_bits(ours[i]), f32::from_bits(want[i]));
            }
        }
    }
}

#[test]
fn a_scale_the_converter_would_not_broadcast_is_refused() {
    // A block whose scale spans a length the weight's axis is not, a block over more axes than the scale has, and a
    // scale with more axes than its weight: `dequant_simple` would raise or write another shape; the grid refuses.
    assert!(ScaleGrid::new(&[64, 96], &[3, 4], vec![1.0; 12], Some(&[16, 24])).is_err(), "48 rows of scale over 64");
    assert!(ScaleGrid::new(&[64, 96], &[4], vec![1.0; 4], Some(&[16, 24])).is_err(), "a 2-axis block over a 1-axis scale");
    assert!(ScaleGrid::new(&[64], &[4, 4], vec![1.0; 16], None).is_err(), "a 2-axis scale over a vector");
    assert!(ScaleGrid::new(&[64, 96], &[4, 4], vec![1.0; 15], Some(&[16, 24])).is_err(), "too few values");
    assert!(ScaleGrid::new(&[64, 96], &[4, 4], vec![1.0; 16], Some(&[16, 24])).is_ok(), "a block that divides both axes");
}

#[test]
fn every_format_matches_ggmls_reference_bit_for_bit() {
    if !(common::require_tool("competitor-sources") && common::require_tool("gcc")) {
        return;
    }
    let ggml = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../external/competitors/llama.cpp/ggml");
    let dir = std::env::temp_dir().join(format!("logos_quantref_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("h.c"), HARNESS).unwrap();
    let bin = dir.join("h");
    let built = Command::new("gcc")
        .args(["-O2", "-ffp-contract=off", "-std=gnu11"])
        .arg(format!("-I{}", ggml.join("include").display()))
        .arg(format!("-I{}", ggml.join("src").display()))
        .arg(dir.join("h.c"))
        .arg(ggml.join("src/ggml-quants.c"))
        .args(["-lm", "-o"])
        .arg(&bin)
        .output()
        .expect("gcc");
    assert!(
        built.status.success(),
        "the ggml reference harness must build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    for (k, &ty) in FORMATS.iter().enumerate() {
        let blocks = if ty.block_len() == 1 { 4096 } else { 64 };
        let bytes = random_blocks(ty, blocks, 0x9e37_79b9_7f4a_7c15 ^ (k as u64 + 1));
        let n = blocks * ty.block_len();
        let mut child = Command::new(&bin)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("run the reference");
        {
            let stdin = child.stdin.as_mut().unwrap();
            stdin.write_all(&ty.id().to_le_bytes()).unwrap();
            stdin.write_all(&(n as u64).to_le_bytes()).unwrap();
            stdin.write_all(&(bytes.len() as u64).to_le_bytes()).unwrap();
            stdin.write_all(&bytes).unwrap();
        }
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{}: the reference exited {:?}", ty.name(), out.status);
        let reference: Vec<u32> = out
            .stdout
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let ours: Vec<u32> = decode(ty, &bytes).iter().map(|v| v.to_bits()).collect();
        assert_eq!(ours.len(), reference.len(), "{}: element count", ty.name());
        if let Some(i) = ours.iter().zip(&reference).position(|(a, b)| a != b) {
            panic!(
                "{}: element {i} (block {}) is {} here and {} in ggml",
                ty.name(),
                i / ty.block_len(),
                f32::from_bits(ours[i]),
                f32::from_bits(reference[i])
            );
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_reader_reads_a_whole_checkpoint() {
    if !common::require_tool("qwen3-8b-checkpoint") {
        return;
    }
    let mut g = Gguf::open(&common::qwen3_8b_checkpoint()).expect("open");
    assert_eq!(g.arch.arch, "qwen3");
    assert_eq!(g.meta("tokenizer.ggml.model").and_then(MetaValue::as_str), Some("gpt2"));
    let tokens = g
        .meta("tokenizer.ggml.tokens")
        .and_then(MetaValue::as_array)
        .expect("the vocabulary is in the metadata");
    let (_, vocab) = g.dims2("output.weight").expect("output.weight");
    assert_eq!(tokens.len(), vocab, "one token string per output row");
    assert!(
        g.meta("tokenizer.ggml.merges").and_then(MetaValue::as_array).is_some_and(|m| m.len() > 100_000),
        "the BPE merges are in the metadata"
    );
    assert!(g.meta("tokenizer.chat_template").and_then(MetaValue::as_str).is_some());

    let name = "blk.0.attn_q.weight";
    assert_eq!(g.tensor_type(name).unwrap(), GgmlType::Q8_0);
    let (in_f, _) = g.dims2(name).unwrap();
    let (quants, scales) = g.q8_0_matrix(name).unwrap();
    let rows = g.dequant_rows(name, 3, 2).unwrap();
    let want: Vec<f32> = (3 * in_f..5 * in_f)
        .map(|i| quants[i] as f32 * scales[i / 32])
        .collect();
    assert_eq!(rows, want, "rows 3..5 decode through the generic path to the Q8_0 accessor's values");
    let norm = g.dequant_tensor("blk.0.attn_norm.weight").unwrap();
    assert_eq!(norm, g.f32_tensor("blk.0.attn_norm.weight").unwrap());
}
