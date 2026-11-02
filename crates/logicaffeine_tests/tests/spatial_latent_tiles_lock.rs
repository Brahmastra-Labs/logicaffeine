//! spatial_latent_tiles_lock — the latent-attention family lowers onto the SN50 as tiles at the widths published latent
//! attentions have, and computes what the VM computes.
//!
//! At whole-tile widths the three pieces DeepSeek's absorbed decode adds to an attention are tile work: `the first K of
//! every N of X` and `A joined with B across H heads` move whole tiles a block at a time, and `X matmul W across H heads`
//! is a GEMV per head, an item per output tile reading only its head's rows of one tile column of `W`. The attention
//! over the latent cache reads keys wider than its values (`[c ‖ pe]` against `c`), each wider than an accumulator page
//! at DeepSeek-V2's widths (576 and 512), in the flash form, its values folded a page at a time.
//!
//! Each case is held to the program-path ledger's rule: on an fp32 twin of the chip the program is the VM's within a
//! relative 2⁻¹⁶ (its wiring), and on the SN50 within the bf16 datapath's 2% of the largest value. Each case also names
//! the form it must take, so a fallback to a flat section that happens to fit a toy width cannot pass for the tile form.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_latent_tiles_lock`

#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::{Phase, SpatialProgram};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

/// A value in `[-1, 1]` in steps of 1/64 for element `i` of tensor `name`.
fn seeded(name: &str, i: usize) -> f64 {
    let salt = name.bytes().fold(7u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64));
    let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29);
    ((h >> 40) % 129) as f64 / 64.0 - 1.0
}

/// Seeded `Real` tensors `(name, rows, cols)`, then `body`, then every entry of `show` (`n` of them) as `round(v·1e6)`.
fn program(tensors: &[(&str, usize, usize)], body: &str, show: &str, n: usize) -> String {
    let mut s = String::from("## Main\n");
    for &(name, r, c) in tensors {
        s.push_str(&format!("Let {name} be a {r} by {c} tensor of Real.\n"));
        for i in 0..r * c {
            s.push_str(&format!("Set item {} of {name} to {:?}.\n", i + 1, seeded(name, i)));
        }
    }
    s.push_str(body);
    for i in 1..=n {
        s.push_str(&format!("Show round(item {i} of {show} * 1000000).\n"));
    }
    s
}

/// The SN50 with fp32 wherever it offers a float.
fn fp32_twin() -> SpatialTarget {
    let mut t = SpatialTarget::sn50_rdu();
    for c in &mut t.compute {
        if let ComputeCap::MatmulTile { precisions, .. } | ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } = c {
            if precisions.contains(&Precision::Bf16) {
                precisions.insert(Precision::Fp32);
            }
        }
    }
    t.precisions.insert(Precision::Fp32);
    t
}

/// The program lowered whole for `target` and certified.
fn composed(src: &str, target: &SpatialTarget, what: &str) -> SpatialProgram {
    let pl = compile_program_to_spatial(src, target).unwrap_or_else(|e| panic!("{what}: the walk: {e:?}"));
    let sp = pl.whole().unwrap_or_else(|| panic!("{what}: not lowered whole: {}", pl.refusal())).clone();
    assert_eq!(sp.phase(), Phase::Verified, "{what}: the program certifies");
    sp
}

/// The case on the fp32 twin and on the SN50 against the VM; every section name in `forms` is a section of the SN50's
/// program (by prefix).
fn verdict(src: &str, what: &str, forms: &[&str]) {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "{what}: the VM: {:?}", vm.error);
    let want = oracle_ints(&vm.output).expect("the VM shows integers");
    let twin = composed(src, &fp32_twin(), what);
    let out = eval_choreography(&twin, &inputs_from_fills(&twin).expect("fills")).unwrap_or_else(|e| panic!("{what}: fp32: {e:?}"));
    let got = shown_from_outputs(src, &twin, &out).expect("shown");
    assert_eq!(got.len(), want.len(), "{what}: one Show a value");
    for (i, (g, v)) in got.iter().zip(&want).enumerate() {
        assert!(((g - v).abs() as f64) <= (v.abs() as f64 / 65536.0).max(1.0), "{what}: WIRING Show #{}: fp32 twin {g} vs VM {v}", i + 1);
    }
    let chip = composed(src, &SpatialTarget::sn50_rdu(), what);
    let names: Vec<&str> = chip.sections.iter().map(|s| s.name.as_str()).collect();
    for form in forms {
        assert!(names.iter().any(|n| n.starts_with(form)), "{what}: the SN50 program takes the form `{form}…`: {names:?}");
    }
    let out = eval_choreography(&chip, &inputs_from_fills(&chip).expect("fills")).unwrap_or_else(|e| panic!("{what}: bf16: {e:?}"));
    let got = shown_from_outputs(src, &chip, &out).expect("shown");
    let scale = want.iter().fold(1i64, |m, v| m.max(v.abs())) as f64;
    for (i, (g, v)) in got.iter().zip(&want).enumerate() {
        assert!(((g - v).abs() as f64) <= 0.02 * scale, "{what}: CHIP Show #{}: bf16 {g} vs VM {v} (scale {scale})", i + 1);
    }
}

#[test]
fn a_head_slice_of_whole_tiles_moves_tiles() {
    // Three groups of 96: the first 64 of each, then the last 32, then a middle 32 written as the last 64's first half.
    for (what, body, n) in [
        ("the first 64 of every 96", "Let P be the first 64 of every 96 of X.\n", 192),
        ("the last 32 of every 96", "Let P be the last 32 of every 96 of X.\n", 96),
    ] {
        verdict(&program(&[("X", 1, 288)], body, "P", n), what, &["head_slice_3x"]);
    }
    // Wider than an accumulator page: 10 tiles a part, moved a block at a time.
    verdict(&program(&[("X", 1, 960)], "Let P be the first 320 of every 480 of X.\n", "P", 640), "the first 320 of every 480", &["head_slice_2x320_of_480_tiled"]);
}

#[test]
fn a_join_of_whole_tiles_moves_tiles() {
    // Every head's 64 of `A` then its 32 of `B`, three heads; and parts of unequal block counts (5 tiles and 2).
    verdict(
        &program(&[("A", 1, 192), ("B", 1, 96)], "Let J be A joined with B across 3 heads.\n", "J", 288),
        "64 + 32 across 3 heads",
        &["join_1x3_64+32_tiled"],
    );
    verdict(
        &program(&[("A", 1, 320), ("B", 1, 128)], "Let J be A joined with B across 2 heads.\n", "J", 448),
        "160 + 64 across 2 heads",
        &["join_1x2_160+64_tiled"],
    );
    // One head: a row joined with a row.
    verdict(&program(&[("A", 1, 64), ("B", 1, 32)], "Let J be A joined with B.\n", "J", 96), "a row joined with a row", &["join_1x1_64+32_tiled"]);
}

#[test]
fn a_per_head_matmul_of_whole_tiles_is_a_gemv_per_head() {
    verdict(
        &program(&[("X", 1, 192), ("W", 192, 32)], "Let Y be X matmul W across 3 heads.\n", "Y", 96),
        "3 heads of 64 by 32",
        &["head_matmul_1x3x64x32_tiled"],
    );
    verdict(
        &program(&[("X", 1, 256), ("W", 256, 96)], "Let Y be X matmul W across 2 heads.\n", "Y", 192),
        "2 heads of 128 by 96",
        &["head_matmul_1x2x128x96_tiled"],
    );
}

/// The absorbed latent-attention decode step at whole-tile widths: `h` heads, `nope` unrotated and `rope` rotated query
/// dims a head, a `lora`-wide latent, values `vd` wide a head, a cache of `len` positions attended up to `pos`.
fn absorbed(h: usize, nope: usize, rope: usize, lora: usize, vd: usize, len: usize, pos: usize) -> String {
    let body = format!(
        "Let qn be the first {nope} of every {qk} of Qf.\n\
         Let qr be the last {rope} of every {qk} of Qf.\n\
         Let ql be qn matmul Wkb across {h} heads.\n\
         Let qa be ql joined with qr across {h} heads.\n\
         Let Vc be the first {lora} of every {lk} of Kc.\n\
         Let pos be {pos}.\n\
         Let ol be attention of qa over Kc and Vc with {h} heads 1 groups up to pos scaled by 0.0625.\n\
         Let O be ol matmul Wvb across {h} heads.\n",
        qk = nope + rope,
        lk = lora + rope,
    );
    program(&[("Qf", 1, h * (nope + rope)), ("Kc", len, lora + rope), ("Wkb", h * nope, lora), ("Wvb", h * lora, vd)], &body, "O", h * vd)
}

#[test]
fn an_absorbed_latent_step_of_whole_tiles_is_the_vms() {
    // Keys of 64 + 32 against values of 64: keys wider than values, both within an accumulator page.
    verdict(&absorbed(2, 32, 32, 64, 32, 40, 37), "2 heads, latent 64 + rope 32", &["head_matmul_1x2x32x64_tiled", "join_1x2_64+32_tiled"]);
}

#[test]
fn an_absorbed_latent_step_wider_than_an_accumulator_page_is_the_vms() {
    // DeepSeek-V2's shape at a quarter of its latent: keys of 256 + 64 (10 tiles) against values of 256 (8 tiles), and
    // a latent of 320 (10 tiles, past an accumulator page) against values of 320.
    verdict(&absorbed(2, 64, 64, 256, 64, 40, 33), "latent 256 + rope 64", &["head_matmul_1x2x64x256_tiled", "join_1x2_256+64_tiled"]);
    verdict(&absorbed(2, 64, 32, 320, 64, 40, 33), "latent 320 + rope 32", &["head_matmul_1x2x320x64_tiled", "join_1x2_320+32_tiled"]);
}

#[test]
fn deepseek_v2_lites_latent_step_lowers_whole_and_certifies() {
    // 16 heads, 128 + 64 query dims, a latent of 512, values of 128: the step the SN50 runs, lowered and certified at
    // its own widths over a cache of 64 positions.
    let src = absorbed(16, 128, 64, 512, 128, 64, 61);
    let sp = composed(&src, &SpatialTarget::sn50_rdu(), "DeepSeek-V2-Lite");
    let names: Vec<&str> = sp.sections.iter().map(|s| s.name.as_str()).collect();
    for form in ["head_matmul_1x16x128x512_tiled", "join_1x16_512+64_tiled", "head_matmul_1x16x512x128_tiled"] {
        assert!(names.iter().any(|n| n.starts_with(form)), "the step takes `{form}`: {names:?}");
    }
}

#[test]
fn deepseek_v2_lites_latent_step_lowers_whole_with_fp32_beside_its_bf16_products() {
    // The same step with fp32 beside bf16 in the pointwise and row units: the flash fold's key and value pages at fp32
    // leave no room for a second page of each beside its query and running outputs, so they take one, and the step
    // still lowers whole and certifies in the same forms.
    let src = absorbed(16, 128, 64, 512, 128, 64, 61);
    let sp = composed(&src, &fp32_twin(), "DeepSeek-V2-Lite with fp32 beside its bf16 products");
    let names: Vec<&str> = sp.sections.iter().map(|s| s.name.as_str()).collect();
    for form in ["head_matmul_1x16x128x512_tiled", "join_1x16_512+64_tiled", "head_matmul_1x16x512x128_tiled"] {
        assert!(names.iter().any(|n| n.starts_with(form)), "the step takes `{form}`: {names:?}");
    }
}

