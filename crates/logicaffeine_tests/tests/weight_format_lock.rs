//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! EVERY WEIGHT FORMAT THE RIVALS RUN, READ AT ITS OWN WIDTH (PARITY-WEIGHT-FORMATS).
//!
//! A matrix a checkpoint keeps in a 16-bit float format — the BF16 and F16 checkpoints vLLM runs — streams two bytes a
//! weight from the checkpoint into the lane's products, never widened to `f32` on the host: it is read as rows in place
//! (`Placement::Rows`) in its own format, every product by it reads its activation `f32`, and its rows take the
//! products' fusions as the quantized formats' do. The tiny checkpoints written in these formats hold the `f32`
//! checkpoint's values exactly. `cuda_lane_lock::every_weight_format_computes_what_its_dequantized_matrix_computes` runs
//! them on the GPU.
//!
//! A matrix a checkpoint keeps in MXFP4 is regrouped at upload, each row's sub-blocks 16 to a group, the group's
//! exponents before its nibbles: the regrouped rows are the stored rows under the layout map the kernels read, and the
//! tensor-core GEMM fetches each sub-block's nibbles as one aligned 16-byte word. A matrix it keeps in Q6_K is grouped at
//! upload, eight blocks to a group, the group's block scales before its blocks, so every block's levels start on a
//! 16-byte boundary; every product by a Q4_K or a Q6_K matrix reads it, and every GEMV by one its activation rows, in
//! 16-byte words.
//!
//!   cargo nextest run -p logicaffeine-tests --test weight_format_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::collections::BTreeSet;
use std::path::PathBuf;

use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane::plan::{Fill, Launch, Schedule, GEMV_ROWS};
use logicaffeine_compile::codegen_cuda::lane::residence::{matrix_bytes, pooled};
use logicaffeine_compile::codegen_cuda::lane::weights::Placement;
use logicaffeine_compile::codegen_cuda::lane::{compile, Lane, LaneOptions};
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::program::{self, Cut};
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::spec::ModelSpec;
use tiny_models::{Tiny, ALL};

const CTX: usize = 24;
const NUMERICS: [Q8MatKind; 3] = [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8];
/// The architectures whose BF16 checkpoints the SOTA contract's 3060 cells run: Llama, Gemma-3, Qwen3.5 — and Qwen3,
/// their attention with QK-norm.
const DENSE: [Tiny; 4] = [Tiny::Llama, Tiny::Qwen3, Tiny::Gemma3, Tiny::Qwen35];
const SIXTEEN: [GgmlType; 2] = [GgmlType::Bf16, GgmlType::F16];

fn dir() -> PathBuf {
    scratch::dir("logos_weight_format")
}

/// The tiny model and its checkpoint as llama.cpp's converter writes one in `ty` (`tiny_models::write_checkpoint_in`).
fn sixteen_bit(t: Tiny, ty: GgmlType) -> (ModelSpec, PathBuf) {
    let spec = t.spec(CTX);
    let path = dir().join(format!("{}-{}.gguf", t.name(), ty.name()));
    tiny_models::write_checkpoint_in(&spec, &path, ty);
    (spec, path)
}

fn options(numerics: Q8MatKind) -> LaneOptions {
    LaneOptions { ctx: CTX, knobs: CudaKnobs { activations: numerics, ..CudaKnobs::default() }, prefill: 4 }
}

/// The constants of a step the author declares quantized matrices.
fn declared_matrices(spec: &ModelSpec) -> BTreeSet<String> {
    program::written(spec, Cut::Decode { ctx: CTX }).0 .2.keys().cloned().collect()
}

/// Every product launch of a schedule — a fold's parts included — with the matrices it reads and their formats.
fn products(s: &Schedule) -> Vec<(&Launch, Vec<(&str, GgmlType)>)> {
    s.launches
        .iter()
        .flat_map(|l| l.parts())
        .filter_map(|l| {
            let read: Vec<(&str, GgmlType)> = match l {
                Launch::Gemv { w, format, .. } => vec![(w.as_str(), *format)],
                Launch::GemmParts { parts, format, .. } => parts.iter().map(|p| (p.w.as_str(), *format)).collect(),
                Launch::GemvGlu { wg, wu, formats, .. } => vec![(wg.as_str(), formats[0]), (wu.as_str(), formats[1])],
                Launch::HeadGemv { w, format, .. } => vec![(w.as_str(), *format)],
                Launch::Gather { table, format: Some(f), .. } => vec![(table.as_str(), *f)],
                _ => return None,
            };
            Some((l, read))
        })
        .collect()
}

/// A tiny checkpoint written in a 16-bit float format holds exactly the values the `f32` checkpoint holds, every matrix
/// of every architecture: the reference and the lane read the same numbers from either file.
#[test]
fn a_sixteen_bit_checkpoint_holds_the_f32_checkpoints_values() {
    for &t in ALL {
        let spec = t.spec(CTX);
        let f32_path = dir().join(format!("{}-f32-all.gguf", t.name()));
        tiny_models::write_checkpoint_as(&spec, &f32_path, &|_| GgmlType::F32);
        let mut f32s = Gguf::open(&f32_path).unwrap();
        for ty in SIXTEEN {
            let (_, path) = sixteen_bit(t, ty);
            let mut g = Gguf::open(&path).unwrap();
            let names: Vec<(String, GgmlType)> = g.tensors().iter().map(|t| (t.name.clone(), g.tensor_type(&t.name).unwrap())).collect();
            assert!(names.iter().any(|(_, f)| *f == ty), "{} {}: some matrix in the format", t.name(), ty.name());
            for (name, f) in names {
                let (a, b) = (g.dequant_tensor(&name).unwrap(), f32s.dequant_tensor(&name).unwrap());
                assert_eq!(a, b, "{} {}: `{name}` ({}) holds the f32 checkpoint's values", t.name(), ty.name(), f.name());
            }
        }
    }
}

/// Every matrix a BF16 checkpoint keeps in BF16 streams two bytes a weight (`streams_two_bytes_a_weight`).
#[test]
fn a_bf16_matrix_streams_two_bytes_a_weight() {
    streams_two_bytes_a_weight(GgmlType::Bf16);
}

/// Every matrix an F16 checkpoint keeps in F16 streams two bytes a weight (`streams_two_bytes_a_weight`).
#[test]
fn a_f16_matrix_streams_two_bytes_a_weight() {
    streams_two_bytes_a_weight(GgmlType::F16);
}

/// On Llama, Qwen3, Gemma-3 and Qwen3.5 written in `ty`, under every numerics, every matrix the author declares quantized
/// is read as rows in place in the checkpoint's format — none widened to `f32` values on the host — and takes the bytes
/// of its rows at two a weight; every product and gather reading one reads it in that format.
fn streams_two_bytes_a_weight(ty: GgmlType) {
    for t in DENSE {
        {
            let (spec, path) = sixteen_bit(t, ty);
            let declared = declared_matrices(&spec);
            for numerics in NUMERICS {
                let what = format!("{} {} {numerics:?}", t.name(), ty.name());
                let lane: Lane = compile(&spec, &path, &options(numerics)).unwrap_or_else(|e| panic!("{what}: {e}"));
                for f in &lane.forms {
                    let s = &f.schedule;
                    let mut read = 0;
                    for (name, b) in &s.buffers {
                        let Fill::Constant(p) = &b.fill else { continue };
                        if !declared.contains(name) {
                            continue;
                        }
                        let Placement::Rows(r) = p else { panic!("{what} {}: `{name}` is widened to f32 values, not read in place", f.tag()) };
                        assert_eq!(r.ty, ty, "{what} {}: `{name}` read in the checkpoint's format", f.tag());
                        assert!(
                            matrix_bytes(r.rows, r.cols, r.ty) <= pooled(2 * r.rows * r.cols + 32),
                            "{what} {}: `{name}` takes two bytes a weight: {} for {} × {}",
                            f.tag(),
                            matrix_bytes(r.rows, r.cols, r.ty),
                            r.rows,
                            r.cols
                        );
                        read += 1;
                    }
                    assert!(read > 0, "{what} {}: the step reads its matrices in place", f.tag());
                    for (l, ws) in products(s) {
                        for (w, format) in ws {
                            if declared.contains(w) {
                                assert_eq!(format, ty, "{what} {}: {l:?} reads `{w}` in the checkpoint's format", f.tag());
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A product by a 16-bit matrix reads its activation `f32`: under the quantized numerics the activation of every such
/// product keeps its `f32` buffer, and the decode step quantizes no activation that only 16-bit products read.
#[test]
fn a_sixteen_bit_product_reads_its_activation_f32() {
    for t in DENSE {
        for ty in SIXTEEN {
            let (spec, path) = sixteen_bit(t, ty);
            for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
                let what = format!("{} {} {numerics:?}", t.name(), ty.name());
                let lane = compile(&spec, &path, &options(numerics)).unwrap_or_else(|e| panic!("{what}: {e}"));
                let s = &lane.decode().schedule;
                assert!(products(s).iter().any(|(_, ws)| ws.iter().any(|(_, f)| *f == ty)), "{what}: the step's products read the format: {:?}", s.census());
                for (l, ws) in products(s) {
                    if ws.iter().any(|(_, f)| *f == ty) {
                        if let Some(x) = l.projected_input() {
                            assert!(s.buffers[x].plain, "{what}: {l:?} reads `{x}` f32");
                        }
                    }
                }
                assert!(!s.launches.iter().any(|l| matches!(l, Launch::Quant { .. })), "{what}: no activation quantized for 16-bit products: {:?}", s.census());
            }
        }
    }
}

/// Every program a 16-bit float checkpoint compiles to builds with `nvcc` and keeps its widening in registers: Llama,
/// Qwen3, Gemma-3 and Qwen3.5 in BF16 and F16, under the precise and the quantized numerics, their decode steps and
/// chunks of 4 — no kernel keeps an array in local memory (`__local_depot` in the PTX), the eight weights a lane widens
/// (`lk_h16_dot`) and the rows a gather widens among them.
#[test]
fn every_sixteen_bit_program_builds_and_widens_in_registers() {
    if !common::require_tool("nvcc") {
        return;
    }
    let mut depots = Vec::new();
    let mut widened = 0;
    for t in DENSE {
        for ty in SIXTEEN {
            let (spec, path) = sixteen_bit(t, ty);
            for numerics in [Q8MatKind::Dequant, Q8MatKind::Int8Act] {
                let what = format!("{} {} {numerics:?}", t.name(), ty.name());
                let lane = compile(&spec, &path, &options(numerics)).unwrap_or_else(|e| panic!("{what}: {e}"));
                let src = dir().join(format!("{}-{}-{numerics:?}.cu", t.name(), ty.name()));
                std::fs::write(&src, &lane.program.cu).unwrap();
                let ptx = src.with_extension("ptx");
                let out = std::process::Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-ptx", "-o"]).arg(&ptx).arg(&src).output().expect("nvcc runs");
                assert!(out.status.success(), "{what} compiles:\n{}", String::from_utf8_lossy(&out.stderr));
                let text = std::fs::read_to_string(&ptx).unwrap();
                for entry in text.split(".entry ").skip(1) {
                    let name = entry.split('(').next().unwrap_or("").trim().to_string();
                    if entry.contains("__local_depot") {
                        depots.push(format!("{what}: {name}"));
                    }
                }
                let kernels = ["lk_gemv_blk", "lk_gemv_parts", "lk_gemv_glu", "lk_gemv_blk_norm", "lk_gemv_parts_rope", "lk_gemv_gdn", "lk_gather_blk", "lk_gather_norm"];
                widened += kernels.iter().filter(|k| lane.program.cu.contains(&format!("{k}<{},", ty.id()))).count();
            }
        }
    }
    assert!(widened > 0, "the programs launch GEMVs and gathers in the 16-bit formats");
    assert!(depots.is_empty(), "a kernel keeps an array in local memory: {depots:?}");
}

/// A 16-bit matrix interleaved at upload (`lk_h16_interleave`) is the stored matrix under the lane's layout map: for
/// matrices of 1, 2, 3, 4, 6, 8, 12, 16, 24, 40 and 48 rows of 32, 96 and 288 values, the group the upload takes
/// (`lk_h16_group`) is the rows a warp the plan gives a product of that many outputs (`kernels::sixteen_rows_a_warp`),
/// every row's every block of 32 values (`lk_h16_addr`, the address every kernel reads it at) is one 64-byte run of the
/// matrix's own bytes, the runs tile the matrix exactly once, and each holds the stored row's block — the map and the
/// interleave evaluated on the host from the kernels' own source.
#[test]
fn a_sixteen_bit_matrix_interleaved_at_upload_is_the_stored_matrix_under_its_layout_map() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{sixteen_rows_a_warp, Kernel, PRELUDE};
    use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
    if !common::require_tool("nvcc") {
        return;
    }
    let rows = [1usize, 2, 3, 4, 6, 8, 12, 16, 24, 40, 48];
    let cols = [32usize, 96, 288];
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 24) as u8
    };
    let work = dir().join("h16-layout");
    std::fs::create_dir_all(&work).unwrap();
    let mut stored = std::collections::BTreeMap::new();
    for &r in &rows {
        for &c in &cols {
            let bytes: Vec<u8> = (0..r * c * 2).map(|_| next()).collect();
            std::fs::write(work.join(format!("stored-{r}-{c}.bin")), &bytes).unwrap();
            stored.insert((r, c), bytes);
        }
    }
    let probe = format!(
        "#include <cstdio>\n#include <vector>\n#include <cuda_runtime.h>\n{PRELUDE}{}\n\
         int main(int argc, char** argv){{\n\
         \x20 const long rows[] = {{{}}}, cols[] = {{{}}};\n\
         \x20 for(long r : rows) for(long c : cols){{\n\
         \x20   char path[4096];\n\
         \x20   snprintf(path, sizeof path, \"%s/stored-%ld-%ld.bin\", argv[1], r, c);\n\
         \x20   std::vector<unsigned char> src(r * c * 2), dst(r * c * 2, 0);\n\
         \x20   FILE* f = fopen(path, \"rb\");\n\
         \x20   if(!f || fread(src.data(), 1, src.size(), f) != src.size()) return 1;\n\
         \x20   fclose(f);\n\
         \x20   const int g = lk_h16_group(r);\n\
         \x20   lk_h16_interleave(src.data(), r, c, g, dst.data());\n\
         \x20   snprintf(path, sizeof path, \"%s/lane-%ld-%ld.bin\", argv[1], r, c);\n\
         \x20   f = fopen(path, \"wb\");\n\
         \x20   fwrite(dst.data(), 1, dst.size(), f);\n\
         \x20   fclose(f);\n\
         \x20   printf(\"%ld %ld %d\", r, c, g);\n\
         \x20   for(long i = 0; i < r; ++i) for(long b = 0; b < c / 32; ++b) printf(\" %ld\", (long)(lk_h16_addr(dst.data(), c * 2, g, i, (int)b) - dst.data()));\n\
         \x20   printf(\"\\n\");\n\
         \x20 }}\n\
         }}\n",
        Kernel::Blocks.source(),
        rows.map(|r| r.to_string()).join(", "),
        cols.map(|c| c.to_string()).join(", ")
    );
    let src = work.join("layout.cu");
    let bin = work.join("layout");
    std::fs::write(&src, probe).unwrap();
    let built = std::process::Command::new("nvcc").args(["-O0", "-std=c++17", "-arch=sm_86", "-o"]).arg(&bin).arg(&src).output().expect("nvcc runs");
    assert!(built.status.success(), "the layout probe builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let out = std::process::Command::new(&bin).arg(&work).output().expect("the probe runs");
    assert!(out.status.success(), "the probe reads and writes its matrices");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut cases = 0;
    for line in stdout.lines() {
        let v: Vec<usize> = line.split_whitespace().map(|t| t.parse().unwrap()).collect();
        let (r, c, g) = (v[0], v[1], v[2]);
        assert_eq!(g, sixteen_rows_a_warp(Q8MatKind::Int8Act, r), "{r} rows: the upload's group is the rows a warp the plan gives them");
        let lane = std::fs::read(work.join(format!("lane-{r}-{c}.bin"))).unwrap();
        let kept = &stored[&(r, c)];
        let mut covered = vec![false; r * c * 2];
        for (j, &at) in v[3..].iter().enumerate() {
            let (i, b) = (j / (c / 32), j % (c / 32));
            assert!(at % 64 == 0 && at + 64 <= r * c * 2, "{r}×{c}: row {i} block {b} at {at} is a 64-byte run inside the matrix");
            for (k, seen) in covered[at..at + 64].iter_mut().enumerate() {
                assert!(!*seen, "{r}×{c}: row {i} block {b} overlaps another at byte {}", at + k);
                *seen = true;
            }
            assert_eq!(&lane[at..at + 64], &kept[i * c * 2 + b * 64..i * c * 2 + b * 64 + 64], "{r}×{c}: row {i} block {b} holds the stored block");
        }
        assert!(covered.iter().all(|&s| s), "{r}×{c}: the blocks tile the matrix");
        cases += 1;
    }
    assert_eq!(cases, rows.len() * cols.len(), "every matrix is probed");
}

/// An MXFP4 matrix regrouped at upload (`lk_mx_regroup`) is the stored matrix under the lane's layout map: for rows of
/// 1, 2, 15, 16, 17, 31, 32, 33 and 90 sub-blocks — a row shorter than a group, whole groups, a short last group —
/// each sub-block's exponent byte (`lk_mx_exp`) and nibble word (`lk_mx_nib`, 16-byte aligned) lie inside its row
/// (`lk_mx_row_bytes`: the bytes `kernels::row_bytes` gives the device, a whole number of 16-byte words, at most 15 past
/// the stored blocks), no two overlap, and the regrouped row holds the stored block's bytes there and zero everywhere
/// else, so it dequantizes to what the stored row does — the map and the regroup evaluated on the host from the
/// kernels' own source.
#[test]
fn a_repacked_matrix_is_the_stored_matrix_under_its_layout_map() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{row_bytes, Kernel, PRELUDE};
    if !common::require_tool("nvcc") {
        return;
    }
    const ROWS: usize = 3;
    let widths = [1usize, 2, 15, 16, 17, 31, 32, 33, 90];
    // The stored rows: every byte from one xorshift stream, each block's exponent inside E8M0's normal range.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 24) as u8
    };
    let stored: Vec<Vec<u8>> = widths
        .iter()
        .map(|&nb| (0..ROWS * nb * 17).map(|i| if i % 17 == 0 { 100 + next() % 50 } else { next() }).collect())
        .collect();
    let work = dir().join("mx-layout");
    std::fs::create_dir_all(&work).unwrap();
    for (nb, bytes) in widths.iter().zip(&stored) {
        std::fs::write(work.join(format!("stored-{nb}.bin")), bytes).unwrap();
    }
    let probe = format!(
        "#include <cstdio>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n{PRELUDE}{}\n\
         int main(int argc, char** argv){{\n\
         \x20 const long widths[] = {{{}}};\n\
         \x20 for(long nb : widths){{\n\
         \x20   char path[4096];\n\
         \x20   snprintf(path, sizeof path, \"%s/stored-%ld.bin\", argv[1], nb);\n\
         \x20   std::vector<unsigned char> src({ROWS} * nb * 17), dst({ROWS} * lk_mx_row_bytes(nb), 0);\n\
         \x20   FILE* f = fopen(path, \"rb\");\n\
         \x20   if(!f || fread(src.data(), 1, src.size(), f) != src.size()) return 1;\n\
         \x20   fclose(f);\n\
         \x20   lk_mx_regroup(src.data(), {ROWS}, nb, dst.data());\n\
         \x20   snprintf(path, sizeof path, \"%s/lane-%ld.bin\", argv[1], nb);\n\
         \x20   f = fopen(path, \"wb\");\n\
         \x20   fwrite(dst.data(), 1, dst.size(), f);\n\
         \x20   fclose(f);\n\
         \x20   printf(\"%ld %ld\", nb, lk_mx_row_bytes(nb));\n\
         \x20   for(long s = 0; s < nb; ++s) printf(\" %ld %ld\", lk_mx_exp(s), lk_mx_nib(s));\n\
         \x20   printf(\"\\n\");\n\
         \x20 }}\n\
         }}\n",
        Kernel::Blocks.source(),
        widths.map(|w| w.to_string()).join(", ")
    );
    let src = work.join("layout.cu");
    let bin = work.join("layout");
    std::fs::write(&src, probe).unwrap();
    let built = std::process::Command::new("nvcc")
        .args(["-O0", "-std=c++17", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("nvcc runs");
    assert!(built.status.success(), "the layout probe builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let out = std::process::Command::new(&bin).arg(&work).output().expect("the probe runs");
    assert!(out.status.success(), "the probe reads and writes its rows");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<Vec<usize>> = stdout.lines().map(|l| l.split_whitespace().map(|v| v.parse().unwrap()).collect()).collect();
    assert_eq!(lines.len(), widths.len(), "a line a width:\n{stdout}");
    for ((&nb, stored), line) in widths.iter().zip(&stored).zip(&lines) {
        let rb = line[1];
        assert_eq!(line[0], nb);
        assert_eq!(rb, row_bytes(GgmlType::Mxfp4, 32 * nb), "{nb} sub-blocks: the device's row bytes are the map's");
        assert_eq!(rb % 16, 0, "{nb} sub-blocks: a row is whole 16-byte words");
        assert!(rb >= 17 * nb && rb - 17 * nb <= 15, "{nb} sub-blocks: {rb} bytes a row");
        assert_eq!(line.len(), 2 + 2 * nb, "{nb} sub-blocks: an exponent and a word each");
        // Which stored byte of which sub-block each byte of a row holds: the exponent byte 0, the nibbles 1 to 16.
        let mut holds: Vec<Option<(usize, usize)>> = vec![None; rb];
        for s in 0..nb {
            let (e, n) = (line[2 + 2 * s], line[3 + 2 * s]);
            assert_eq!(n % 16, 0, "{nb} sub-blocks: sub-block {s}'s nibbles are an aligned word");
            assert!(e < rb && n + 16 <= rb, "{nb} sub-blocks: sub-block {s} lies inside its row");
            for (at, k) in std::iter::once((e, 0)).chain((0..16).map(|j| (n + j, 1 + j))) {
                assert!(holds[at].is_none(), "{nb} sub-blocks: byte {at} of a row is held twice");
                holds[at] = Some((s, k));
            }
        }
        let lane = std::fs::read(work.join(format!("lane-{nb}.bin"))).unwrap();
        assert_eq!(lane.len(), ROWS * rb);
        for r in 0..ROWS {
            let row = &lane[r * rb..(r + 1) * rb];
            let blocks = &stored[r * nb * 17..(r + 1) * nb * 17];
            for (at, held) in holds.iter().enumerate() {
                let want = held.map_or(0, |(s, k)| blocks[s * 17 + k]);
                assert_eq!(row[at], want, "{nb} sub-blocks, row {r}: byte {at} holds {held:?}");
            }
            let regrouped: Vec<u8> = (0..nb)
                .flat_map(|s| {
                    let (e, n) = (line[2 + 2 * s], line[3 + 2 * s]);
                    std::iter::once(row[e]).chain(row[n..n + 16].iter().copied())
                })
                .collect();
            let (mut a, mut b) = (Vec::new(), Vec::new());
            GgmlType::Mxfp4.dequantize(&regrouped, &mut a);
            GgmlType::Mxfp4.dequantize(blocks, &mut b);
            assert_eq!(a, b, "{nb} sub-blocks, row {r}: the regrouped row dequantizes as the stored row does");
        }
    }
}

/// A Q6_K matrix grouped at upload (`lk_q6_regroup`) is the stored matrix under the lane's layout map: for rows of 1, 2,
/// 7, 8, 9, 15, 16, 17 and 48 blocks — a row shorter than a group, whole groups, a short last group — each block's 208
/// bytes of levels and sub-scales (`lk_q6_blk`, 16-byte aligned) and its f16 scale (`lk_q6_d`) lie inside its row
/// (`lk_q6_row_bytes`: the bytes `kernels::row_bytes` gives the device, a whole number of 16-byte words, at most 14 past
/// the stored blocks), no two overlap, and the grouped row holds the stored block's bytes there and zero everywhere
/// else, so it dequantizes to what the stored row does — the map and the regroup evaluated on the host from the
/// kernels' own source.
#[test]
fn a_grouped_q6_k_matrix_is_the_stored_matrix_under_its_layout_map() {
    use logicaffeine_compile::codegen_cuda::lane::kernels::{row_bytes, Kernel, PRELUDE};
    if !common::require_tool("nvcc") {
        return;
    }
    const ROWS: usize = 3;
    let widths = [1usize, 2, 7, 8, 9, 15, 16, 17, 48];
    // The stored rows: every byte from one xorshift stream, each block's scale a finite f16.
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 24) as u8
    };
    let stored: Vec<Vec<u8>> = widths
        .iter()
        .map(|&nb| (0..ROWS * nb * 210).map(|i| if i % 210 == 209 { 0x20 | (next() & 0x0f) } else { next() }).collect())
        .collect();
    let work = dir().join("q6-layout");
    std::fs::create_dir_all(&work).unwrap();
    for (nb, bytes) in widths.iter().zip(&stored) {
        std::fs::write(work.join(format!("stored-{nb}.bin")), bytes).unwrap();
    }
    let probe = format!(
        "#include <cstdio>\n#include <vector>\n#include <cuda_runtime.h>\n#include <cuda_fp16.h>\n{PRELUDE}{}\n\
         int main(int argc, char** argv){{\n\
         \x20 const long widths[] = {{{}}};\n\
         \x20 for(long nb : widths){{\n\
         \x20   char path[4096];\n\
         \x20   snprintf(path, sizeof path, \"%s/stored-%ld.bin\", argv[1], nb);\n\
         \x20   std::vector<unsigned char> src({ROWS} * nb * 210), dst({ROWS} * lk_q6_row_bytes(nb), 0);\n\
         \x20   FILE* f = fopen(path, \"rb\");\n\
         \x20   if(!f || fread(src.data(), 1, src.size(), f) != src.size()) return 1;\n\
         \x20   fclose(f);\n\
         \x20   lk_q6_regroup(src.data(), {ROWS}, nb, dst.data());\n\
         \x20   snprintf(path, sizeof path, \"%s/lane-%ld.bin\", argv[1], nb);\n\
         \x20   f = fopen(path, \"wb\");\n\
         \x20   fwrite(dst.data(), 1, dst.size(), f);\n\
         \x20   fclose(f);\n\
         \x20   printf(\"%ld %ld\", nb, lk_q6_row_bytes(nb));\n\
         \x20   for(long b = 0; b < nb; ++b) printf(\" %ld %ld\", lk_q6_blk(b), lk_q6_d(b));\n\
         \x20   printf(\"\\n\");\n\
         \x20 }}\n\
         }}\n",
        Kernel::Blocks.source(),
        widths.map(|w| w.to_string()).join(", ")
    );
    let src = work.join("layout.cu");
    let bin = work.join("layout");
    std::fs::write(&src, probe).unwrap();
    let built = std::process::Command::new("nvcc")
        .args(["-O0", "-std=c++17", "-arch=sm_86", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("nvcc runs");
    assert!(built.status.success(), "the layout probe builds:\n{}", String::from_utf8_lossy(&built.stderr));
    let out = std::process::Command::new(&bin).arg(&work).output().expect("the probe runs");
    assert!(out.status.success(), "the probe reads and writes its rows");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<Vec<usize>> = stdout.lines().map(|l| l.split_whitespace().map(|v| v.parse().unwrap()).collect()).collect();
    assert_eq!(lines.len(), widths.len(), "a line a width:\n{stdout}");
    for ((&nb, stored), line) in widths.iter().zip(&stored).zip(&lines) {
        let rb = line[1];
        assert_eq!(line[0], nb);
        assert_eq!(rb, row_bytes(GgmlType::Q6K, 256 * nb), "{nb} blocks: the device's row bytes are the map's");
        assert_eq!(rb % 16, 0, "{nb} blocks: a row is whole 16-byte words");
        assert!(rb >= 210 * nb && rb - 210 * nb <= 14, "{nb} blocks: {rb} bytes a row");
        assert_eq!(line.len(), 2 + 2 * nb, "{nb} blocks: a block's bytes and its scale each");
        // Which stored byte of which block each byte of a row holds: the levels and sub-scales 0 to 207, the scale 208
        // and 209.
        let mut holds: Vec<Option<(usize, usize)>> = vec![None; rb];
        for b in 0..nb {
            let (at_b, at_d) = (line[2 + 2 * b], line[3 + 2 * b]);
            assert_eq!(at_b % 16, 0, "{nb} blocks: block {b}'s bytes start on a 16-byte boundary");
            assert!(at_b + 208 <= rb && at_d + 2 <= rb, "{nb} blocks: block {b} lies inside its row");
            for (at, k) in (0..208).map(|k| (at_b + k, k)).chain((0..2).map(|k| (at_d + k, 208 + k))) {
                assert!(holds[at].is_none(), "{nb} blocks: byte {at} of a row is held twice");
                holds[at] = Some((b, k));
            }
        }
        let lane = std::fs::read(work.join(format!("lane-{nb}.bin"))).unwrap();
        assert_eq!(lane.len(), ROWS * rb);
        for r in 0..ROWS {
            let row = &lane[r * rb..(r + 1) * rb];
            let blocks = &stored[r * nb * 210..(r + 1) * nb * 210];
            for (at, held) in holds.iter().enumerate() {
                let want = held.map_or(0, |(b, k)| blocks[b * 210 + k]);
                assert_eq!(row[at], want, "{nb} blocks, row {r}: byte {at} holds {held:?}");
            }
            let regrouped: Vec<u8> = (0..nb)
                .flat_map(|b| {
                    let (at_b, at_d) = (line[2 + 2 * b], line[3 + 2 * b]);
                    row[at_b..at_b + 208].iter().chain(&row[at_d..at_d + 2]).copied().collect::<Vec<u8>>()
                })
                .collect();
            let (mut a, mut b) = (Vec::new(), Vec::new());
            GgmlType::Q6K.dequantize(&regrouped, &mut a);
            GgmlType::Q6K.dequantize(blocks, &mut b);
            assert_eq!(a, b, "{nb} blocks, row {r}: the grouped row dequantizes as the stored row does");
        }
    }
}

/// Every product by a Q6_K matrix under the quantized numerics reads its levels in 16-byte words: Llama at its wide
/// shape with every matrix of whole 256-value rows in Q6_K, compiled with chunks of 64 rows under one activation word
/// and two — every `lk_gemv_blk<14, …>` (the decode step's rows a warp, a chunk's rows in the GEMV's lanes) and every
/// `lk_gemm_blk<14, …>` (the chunks on the tensor cores) in the PTX loads 16-byte words of the matrix from global
/// memory, and no more narrower words than them (a block's scale, its sub-blocks' scales). A 210-byte block is only
/// 2-byte aligned, so its levels read as the checkpoint keeps them take a half-word at a time: 32 loads a sub-block.
#[test]
fn every_q6_k_product_reads_its_levels_in_16_byte_words() {
    if !common::require_tool("nvcc") {
        return;
    }
    let spec = Tiny::Llama.wide(CTX);
    let path = dir().join("llama-wide-q6k.gguf");
    let matrices: BTreeSet<String> = tiny_models::tensors(&spec)
        .into_iter()
        .filter(|(n, dims)| n.ends_with(".weight") && dims.len() == 2 && dims[0] % 256 == 0 && !n.contains("norm"))
        .map(|(n, _)| n)
        .collect();
    assert!(matrices.len() >= 4, "the wide Llama has Q6_K matrices: {matrices:?}");
    tiny_models::write_checkpoint_as(&spec, &path, &|n| if matrices.contains(n) { GgmlType::Q6K } else { GgmlType::F32 });
    let (mut gemvs, mut gemms) = (0, 0);
    for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        let what = format!("llama Q6_K {numerics:?}");
        let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"));
        let src = dir().join(format!("llama-q6k-{numerics:?}.cu"));
        std::fs::write(&src, &lane.program.cu).unwrap();
        let ptx = src.with_extension("ptx");
        let out = std::process::Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-ptx", "-o"]).arg(&ptx).arg(&src).output().expect("nvcc runs");
        assert!(out.status.success(), "{what} compiles:\n{}", String::from_utf8_lossy(&out.stderr));
        let text = std::fs::read_to_string(&ptx).unwrap();
        for entry in text.split(".entry ").skip(1) {
            let name = entry.split('(').next().unwrap_or("").trim();
            let gemv = name.starts_with("_Z11lk_gemv_blkILi14E");
            if !gemv && !name.starts_with("_Z11lk_gemm_blkILi14E") {
                continue;
            }
            let body = entry.split("\n}\n").next().unwrap_or(entry);
            let global = |l: &&str| l.trim_start().starts_with("ld.global");
            let words = body.lines().filter(global).filter(|l| l.contains(".v4.u32") || l.contains(".v4.b32")).count();
            let narrow = body.lines().filter(global).filter(|l| [".u8", ".s8", ".u16", ".s16", ".b8", ".b16"].iter().any(|w| l.contains(w))).count();
            assert!(words > 0 && narrow <= words, "{what}: `{name}` loads {words} 16-byte words and {narrow} narrower words");
            if gemv {
                gemvs += 1;
            } else {
                gemms += 1;
            }
        }
    }
    assert!(gemvs >= 2 && gemms >= 2, "the GEMVs and GEMMs of both numerics are checked: {gemvs} GEMVs, {gemms} GEMMs");
}

/// Every product by a Q4_K matrix under the quantized numerics reads its blocks in 16-byte words and never a byte or a
/// half-word: Llama at its wide shape with every matrix of whole 256-value rows in Q4_K, compiled with chunks of 64 rows
/// under one activation word and two — every `lk_gemv_blk<12, …>`, every gated pair's `lk_gemv_glu<12, 12, …>` (the
/// decode step's rows a warp, a chunk's rows in the GEMV's lanes) and every `lk_gemm_blk<12, …>` (the chunks on the
/// tensor cores) in the PTX loads 16-byte words of the matrix from global memory, and no narrower word than four bytes.
/// A 144-byte block's scale, its minimum's scale and its twelve bytes of six-bit sub-block scales are one 16-byte word
/// (`lk_pair4_q4k`), two sub-blocks' nibbles two more; read as the checkpoint keeps them, each sub-block's scale and
/// minimum take a byte each and its nibbles four bytes at a time — the decode GEMV 36 byte loads and 32 four-byte loads
/// a pass of its sub-blocks.
#[test]
fn every_q4_k_product_reads_its_blocks_in_16_byte_words() {
    if !common::require_tool("nvcc") {
        return;
    }
    let spec = Tiny::Llama.wide(CTX);
    let path = dir().join("llama-wide-q4k.gguf");
    let matrices: BTreeSet<String> = tiny_models::tensors(&spec)
        .into_iter()
        .filter(|(n, dims)| n.ends_with(".weight") && dims.len() == 2 && dims[0] % 256 == 0 && !n.contains("norm"))
        .map(|(n, _)| n)
        .collect();
    assert!(matrices.len() >= 4, "the wide Llama has Q4_K matrices: {matrices:?}");
    tiny_models::write_checkpoint_as(&spec, &path, &|n| if matrices.contains(n) { GgmlType::Q4K } else { GgmlType::F32 });
    let (mut gemvs, mut pairs, mut gemms) = (0, 0, 0);
    for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        let what = format!("llama Q4_K {numerics:?}");
        let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"));
        let src = dir().join(format!("llama-q4k-{numerics:?}.cu"));
        std::fs::write(&src, &lane.program.cu).unwrap();
        let ptx = src.with_extension("ptx");
        let out = std::process::Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-ptx", "-o"]).arg(&ptx).arg(&src).output().expect("nvcc runs");
        assert!(out.status.success(), "{what} compiles:\n{}", String::from_utf8_lossy(&out.stderr));
        let text = std::fs::read_to_string(&ptx).unwrap();
        for entry in text.split(".entry ").skip(1) {
            let name = entry.split('(').next().unwrap_or("").trim();
            let counter = if name.starts_with("_Z11lk_gemv_blkILi12E") {
                &mut gemvs
            } else if name.starts_with("_Z11lk_gemv_gluILi12ELi12E") {
                &mut pairs
            } else if name.starts_with("_Z11lk_gemm_blkILi12E") {
                &mut gemms
            } else {
                continue;
            };
            let body = entry.split("\n}\n").next().unwrap_or(entry);
            let global = |l: &&str| l.trim_start().starts_with("ld.global");
            let words = body.lines().filter(global).filter(|l| l.contains(".v4.u32") || l.contains(".v4.b32")).count();
            let narrow = body.lines().filter(global).filter(|l| [".u8", ".s8", ".u16", ".s16", ".b8", ".b16"].iter().any(|w| l.contains(w))).count();
            assert!(words > 0 && narrow == 0, "{what}: `{name}` loads {words} 16-byte words and {narrow} narrower words than four bytes");
            *counter += 1;
        }
    }
    assert!(
        gemvs >= 2 && pairs >= 2 && gemms >= 2,
        "the GEMVs, gated pairs and GEMMs of both numerics are checked: {gemvs} GEMVs, {pairs} gated pairs, {gemms} GEMMs"
    );
}

/// Every GEMV by a Q4_K or a Q6_K matrix reads its activation rows in 16-byte words: Llama at its wide shape with every
/// matrix of whole 256-value rows in Q4_K, and again in Q6_K, compiled with chunks of 16 rows under one activation word
/// and two — every `lk_gemv_blk<12, …>` and `<14, …>`, every gated pair's `lk_gemv_glu<12, 12, …>` and every
/// `lk_gemv_parts<12, …>`, the decode step's rows a warp and a chunk's 2 to 16 rows in the GEMV's lanes, loads its
/// activation words as 16-byte words through the read-only path, and of its 4-byte integer reads there only the step's
/// position remains. A lane's sub-block pair (a Q6_K quarter) spans 64 bytes (128) of each activation row; read four bytes
/// at a time, a chunk's rows took 1.5 to 3.6 times as long as the same rows a lane a sub-block (Qwen3-8B Q4_K_M on the
/// 3060: the gated pair over 4 rows 993.7 µs where 303.9 in 16-byte words).
#[test]
fn every_k_quant_gemv_reads_its_activation_rows_in_16_byte_words() {
    if !common::require_tool("nvcc") {
        return;
    }
    let spec = Tiny::Llama.wide(CTX);
    let (mut checked, mut chunks) = (0, 0);
    for ty in [GgmlType::Q4K, GgmlType::Q6K] {
        let path = dir().join(format!("llama-wide-acts-{}.gguf", ty.name()));
        let matrices: BTreeSet<String> = tiny_models::tensors(&spec)
            .into_iter()
            .filter(|(n, dims)| n.ends_with(".weight") && dims.len() == 2 && dims[0] % 256 == 0 && !n.contains("norm"))
            .map(|(n, _)| n)
            .collect();
        tiny_models::write_checkpoint_as(&spec, &path, &|n| if matrices.contains(n) { ty } else { GgmlType::F32 });
        let f = ty.id();
        for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
            let what = format!("llama {} {numerics:?}", ty.name());
            let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 16, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"));
            let src = dir().join(format!("llama-acts-{}-{numerics:?}.cu", ty.name()));
            std::fs::write(&src, &lane.program.cu).unwrap();
            let ptx = src.with_extension("ptx");
            let out = std::process::Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-ptx", "-o"]).arg(&ptx).arg(&src).output().expect("nvcc runs");
            assert!(out.status.success(), "{what} compiles:\n{}", String::from_utf8_lossy(&out.stderr));
            let text = std::fs::read_to_string(&ptx).unwrap();
            let gemvs = [format!("_Z11lk_gemv_blkILi{f}E"), format!("_Z11lk_gemv_gluILi{f}ELi{f}E"), format!("_Z13lk_gemv_partsILi{f}E")];
            for entry in text.split(".entry ").skip(1) {
                let name = entry.split('(').next().unwrap_or("").trim();
                if !gemvs.iter().any(|g| name.starts_with(g.as_str())) {
                    continue;
                }
                let body = entry.split("\n}\n").next().unwrap_or(entry);
                let read = |width: &str| body.lines().filter(|l| l.trim_start().starts_with(&format!("ld.global.nc.{width}"))).count();
                let (words, ints) = (read("v4.u32"), read("u32"));
                assert!(words > 0 && ints <= 1, "{what}: `{name}` reads {words} 16-byte words and {ints} 4-byte integers through the read-only path");
                checked += 1;
                // A chunk's GEMV: its third template argument, the activation rows, past one.
                if name.split('E').nth(2).and_then(|a| a.strip_prefix("Li")).is_some_and(|m| m.parse::<usize>().is_ok_and(|m| m > 1)) {
                    chunks += 1;
                }
            }
        }
    }
    assert!(checked >= 8 && chunks >= 4, "the decode step's GEMVs and the chunks' are checked: {checked} GEMVs, {chunks} of chunks");
}

/// An MXFP4 matrix's tensor-core GEMM fetches each weight sub-block as one aligned 16-byte word of nibbles and its
/// exponent byte: gpt-oss at its wide shape, its experts and every other matrix in MXFP4, compiled with chunks of 64
/// rows under both quantized numerics — every `lk_gemm_blk<39, …>` in the PTX loads as many 16-byte words from global
/// memory as bytes, and at least one. A 17-byte block read a byte at a time takes 17 loads a sub-block, each spanning a
/// line of the L1 for each of the warp's rows.
#[test]
fn an_mxfp4_gemm_fetches_each_sub_blocks_nibbles_in_one_load() {
    if !common::require_tool("nvcc") {
        return;
    }
    let spec = Tiny::GptOss.wide(CTX);
    let path = dir().join("gpt-oss-wide-mxfp4.gguf");
    let matrices: BTreeSet<String> = tiny_models::tensors(&spec)
        .into_iter()
        .map(|(n, _)| n)
        .filter(|n| n.ends_with(".weight") && !n.contains("norm") && !n.contains("attn_sinks") && !n.contains("ffn_gate_inp"))
        .collect();
    tiny_models::write_checkpoint_as(&spec, &path, &|n| if matrices.contains(n) { GgmlType::Mxfp4 } else { GgmlType::F32 });
    let mut gemms = 0;
    for numerics in [Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        let what = format!("gpt-oss MXFP4 {numerics:?}");
        let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill: 64, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"));
        let src = dir().join(format!("gpt-oss-mxfp4-{numerics:?}.cu"));
        std::fs::write(&src, &lane.program.cu).unwrap();
        let ptx = src.with_extension("ptx");
        let out = std::process::Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-ptx", "-o"]).arg(&ptx).arg(&src).output().expect("nvcc runs");
        assert!(out.status.success(), "{what} compiles:\n{}", String::from_utf8_lossy(&out.stderr));
        let text = std::fs::read_to_string(&ptx).unwrap();
        for entry in text.split(".entry ").skip(1) {
            let name = entry.split('(').next().unwrap_or("").trim();
            if !name.starts_with("_Z11lk_gemm_blkILi39E") {
                continue;
            }
            let body = entry.split("\n}\n").next().unwrap_or(entry);
            let loads = |width: &str| body.lines().filter(|l| l.trim_start().starts_with("ld.global") && l.contains(width)).count();
            let (words, bytes) = (loads(".v4.u32"), loads(".u8"));
            assert!(words > 0 && words == bytes, "{what}: `{name}` loads {words} 16-byte words and {bytes} bytes");
            gemms += 1;
        }
    }
    assert!(gemms >= 4, "the paired and down GEMMs of both numerics are checked: {gemms}");
}

/// The activation rows a product multiplies, and its outputs (a GEMM in parts' each part's, the fewest).
fn product_rows(l: &Launch) -> usize {
    match l {
        Launch::Gemv { m, .. } | Launch::GemmParts { m, .. } | Launch::GemvGlu { m, .. } => *m,
        _ => 0,
    }
}
fn product_outputs(l: &Launch) -> usize {
    match l {
        Launch::Gemv { n, .. } | Launch::GemvGlu { n, .. } => *n,
        Launch::GemmParts { parts, .. } => parts.iter().map(|p| p.n).fold(usize::MAX, usize::min),
        _ => 0,
    }
}

/// The output rows a warp of a GEMV-shaped product takes, or none for a launch that is not one.
fn rows_a_warp(l: &Launch) -> Option<usize> {
    match l {
        Launch::Gemv { per_warp, .. } | Launch::GemmParts { per_warp, .. } | Launch::GemvGlu { per_warp, .. } => Some(*per_warp),
        _ => None,
    }
}

/// A product by a 16-bit matrix runs on the 16-bit tensor cores under the quantized numerics at every row count, so a
/// prompt's chunks sum as its decode steps do (PARITY-WEIGHT-FORMATS, phase 2: an activation read from its `f32` buffer
/// and rounded in registers to one bf16 or f16 word under one word, to two under two words): Llama, Qwen3, Gemma-3 and
/// Qwen3.5 in BF16 and F16, with chunks of 4 rows and of 64 — a product of more than `GEMV_ROWS` rows on the GEMM
/// (`lk_gemm_h16`), a product of at most `GEMV_ROWS` rows in the GEMV's lanes (`lk_h16_mma_rows`, a decode step's one
/// row too), each at the most of sixteen output rows a warp its outputs allow, so a short prompt's chunks read each
/// matrix once as a step does and launch no GEMM; every program's PTX carries the format's `mma`. Under the precise
/// numerics every such product stays on the CUDA cores.
#[test]
fn a_sixteen_bit_projection_runs_on_the_tensor_cores_without_widening() {
    if !common::require_tool("nvcc") {
        return;
    }
    let mut checked = [0usize; 2];
    for t in DENSE {
        for ty in SIXTEEN {
            let (spec, path) = sixteen_bit(t, ty);
            for numerics in NUMERICS {
                for prefill in [4, 64] {
                    let what = format!("{} {} {numerics:?}, chunks of {prefill}", t.name(), ty.name());
                    let lane = compile(&spec, &path, &LaneOptions { ctx: 128, prefill, ..options(numerics) }).unwrap_or_else(|e| panic!("{what}: {e}"));
                    for f in &lane.forms {
                        for (l, ws) in products(&f.schedule) {
                            if matches!(l, Launch::Gather { .. }) || !ws.iter().any(|(_, x)| *x == ty) {
                                continue;
                            }
                            let rows = product_rows(l);
                            if numerics == Q8MatKind::Dequant {
                                assert!(!l.on_tensor_cores(numerics), "{what}, {} rows: {l:?} sums on the CUDA cores", f.rows);
                            } else if rows > GEMV_ROWS {
                                checked[1] += 1;
                                assert!(l.on_tensor_cores(numerics), "{what}, {} rows: {l:?} runs on the 16-bit GEMM", f.rows);
                            } else {
                                checked[0] += 1;
                                assert!(!l.on_tensor_cores(numerics), "{what}, {} rows: {l:?} runs in the GEMV's lanes", f.rows);
                                let n = product_outputs(l);
                                let want = [16, 8, 4, 2, 1].into_iter().find(|w| n % w == 0).unwrap();
                                assert_eq!(rows_a_warp(l), Some(want), "{what}: {l:?} takes the most of sixteen rows a warp its {n} outputs allow");
                            }
                        }
                    }
                    if numerics != Q8MatKind::Dequant {
                        let gemm = lane.program.cu.contains(&format!("lk_gemm_h16<{}", ty.id()));
                        assert_eq!(gemm, prefill > GEMV_ROWS, "{what}: the 16-bit GEMM launches only for chunks past {GEMV_ROWS} rows");
                    }
                    if numerics == Q8MatKind::Int8Act {
                        let src = dir().join(format!("{}-{}-{prefill}-mma.cu", t.name(), ty.name()));
                        std::fs::write(&src, &lane.program.cu).unwrap();
                        let ptx = src.with_extension("ptx");
                        let out = std::process::Command::new("nvcc").args(["-O3", "-std=c++17", "-arch=sm_86", "-ptx", "-o"]).arg(&ptx).arg(&src).output().expect("nvcc runs");
                        assert!(out.status.success(), "{what} compiles:\n{}", String::from_utf8_lossy(&out.stderr));
                        let word = if ty == GgmlType::Bf16 { "bf16" } else { "f16" };
                        let mma = format!("mma.sync.aligned.m16n8k16.row.col.f32.{word}.{word}.f32");
                        assert!(std::fs::read_to_string(&ptx).unwrap().contains(&mma), "{what}: the program multiplies on `{mma}`");
                    }
                }
            }
        }
    }
    assert!(checked.iter().all(|&c| c > 0), "the programs' 16-bit products of both kinds are checked: {checked:?}");
}
