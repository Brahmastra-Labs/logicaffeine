//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE PIECES OF A LATENT ATTENTION, AND THE ATTENTION THEY MAKE.
//!
//! DeepSeek's multi-head latent attention caches one compressed row per position and attends over it with each head's
//! query absorbed into the latent space. Written in the language it takes five general pieces:
//!
//!   • `the first K of every N of X` / `the last K of every N of X` / `the K after the first F of every N of X` — one
//!     part of every head;
//!   • `A joined with B` / `A joined with B across H heads` — rows, or every head's parts, side by side;
//!   • `X matmul W across H heads` — each head's slice of `X` times its own block of `W`'s rows;
//!   • an attention whose values are narrower than its keys (`V` of its own width per head);
//!   • `… scaled by S` — an attention's score scale given, not `1/√HD`.
//!
//! Each piece computes its `f64` formula on the VM, the tree-walker, C and AOT-Rust, is lifted into its spec by the
//! front end, and is refused by name where a shape does not fit; the pieces composed into an absorbed latent attention
//! step compute exactly the expanded attention the reference runs — keys and values decompressed per head — and the
//! spatial IR lowers the narrower values, the given scale and the whole absorbed step and computes what the VM shows.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::passes::recognize_stages::{recognize_stages, TensorOpSpec};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use tensor_probes::{inputs_from_fills, oracle_ints, shown_from_outputs};

/// A seeded value in about ±1 for element `i` of tensor `name`.
fn seeded(name: &str, i: usize) -> f64 {
    let salt = name.bytes().fold(7u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64));
    let h = (i as u64 ^ salt).wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29);
    ((h >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
}

/// Declarations of seeded `Real` tensors `(name, rows, cols)`, then `body`, then every entry of `show` (of `n` values)
/// shown as `round(v·1e9)`.
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
        s.push_str(&format!("Show round(item {i} of {show} * 1000000000).\n"));
    }
    s
}

fn shown(src: &str) -> Vec<i64> {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "VM error on:\n{src}\n{:?}", vm.error);
    let tw = tw_outcome(src);
    assert!(tw.error.is_none(), "tree-walker error: {:?}", tw.error);
    assert_eq!(tw.output.trim_end(), vm.output.trim_end(), "the tree-walker agrees with the VM");
    vm.output.split_whitespace().map(|t| t.parse().expect("an integer")).collect()
}

fn round9(v: &[f64]) -> Vec<i64> {
    v.iter().map(|x| (x * 1e9).round() as i64).collect()
}

fn close(got: &[i64], want: &[i64], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: {got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() <= 2, "{what}: {got:?} vs the formula's {want:?}");
    }
}

fn x(name: &str, n: usize) -> Vec<f64> {
    (0..n).map(|i| seeded(name, i)).collect()
}

#[test]
fn a_head_slice_takes_one_part_of_every_head() {
    let q = &x("Q", 2 * 12);
    let first: Vec<f64> = (0..2).flat_map(|r| (0..3).flat_map(move |h| (0..3).map(move |i| (r, h, i)))).map(|(r, h, i)| q[r * 12 + h * 4 + i]).collect();
    let last: Vec<f64> = (0..2).flat_map(|r| (0..3).map(move |h| q[r * 12 + h * 4 + 3])).collect();
    close(&shown(&program(&[("Q", 2, 12)], "Let P be the first 3 of every 4 of Q.\n", "P", 18)), &round9(&first), "the first 3 of every 4");
    close(&shown(&program(&[("Q", 2, 12)], "Let P be the last 1 of every 4 of Q.\n", "P", 6)), &round9(&last), "the last 1 of every 4");
}

#[test]
fn a_part_after_the_first_f_of_every_head_is_the_middle_of_each() {
    // Three heads of six a row: each head's two values after its first three, on every tier and lifted with its offset.
    let q = &x("Q", 2 * 18);
    let mid: Vec<f64> = (0..2).flat_map(|r| (0..3).flat_map(move |h| (0..2).map(move |i| q[r * 18 + h * 6 + 3 + i]))).collect();
    let src = program(&[("Q", 2, 18)], "Let P be the 2 after the first 3 of every 6 of Q.\n", "P", 12);
    close(&shown(&src), &round9(&mid), "the 2 after the first 3 of every 6");
    assert_eq!(
        shown(&program(&[("Q", 2, 18)], "Let P be the 2 after the first 4 of every 6 of Q.\n", "P", 12)),
        shown(&program(&[("Q", 2, 18)], "Let P be the last 2 of every 6 of Q.\n", "P", 12)),
        "the part that ends each head is its last part"
    );
    let want = vm_outcome(&src).output;
    common::assert_c_output(&src, want.trim());
    common::assert_exact_output(&src, want.trim());
    let rp = recognize_stages(&src).expect("parses");
    assert!(rp.unrecognized.is_empty(), "{:?}", rp.unrecognized);
    let slice = rp.stages.into_iter().find_map(|s| if let TensorOpSpec::HeadSlice(s) = s.op { Some(s) } else { None }).expect("a slice");
    assert_eq!((slice.every, slice.from, slice.width, slice.n), (6, 3, 2, 36));
    for (body, why) in [
        ("Let P be the 2 after the first 0 of every 6 of Q.", "the first 2 of every 6"),
        ("Let P be the 3 after the first 4 of every 6 of Q.", "a part of 3 after the first 4 must fit in a group of 6"),
        ("Let P be the 2 after the first 1 of every 4 of Q.", "its 18 columns are not whole groups of every 4"),
    ] {
        let src = format!("## Main\nLet Q be a 2 by 18 tensor of Real.\n{body}\nShow 1.\n");
        let err = vm_outcome(&src).error.unwrap_or_else(|| panic!("`{body}` is refused"));
        assert!(err.contains(why), "`{body}` is refused naming {why}: {err}");
    }
}

#[test]
fn a_join_puts_rows_or_every_heads_parts_side_by_side() {
    let (a, b) = (&x("A", 2 * 6), &x("B", 2 * 4));
    let plain: Vec<f64> = (0..2).flat_map(|r| a[r * 6..r * 6 + 6].iter().chain(&b[r * 4..r * 4 + 4]).copied().collect::<Vec<_>>()).collect();
    let heads: Vec<f64> = (0..2)
        .flat_map(|r| (0..2).flat_map(move |h| (0..3).map(move |i| a[r * 6 + h * 3 + i]).chain((0..2).map(move |i| b[r * 4 + h * 2 + i]))))
        .collect();
    close(&shown(&program(&[("A", 2, 6), ("B", 2, 4)], "Let J be A joined with B.\n", "J", 20)), &round9(&plain), "a join of rows");
    close(
        &shown(&program(&[("A", 2, 6), ("B", 2, 4)], "Let J be A joined with B across 2 heads.\n", "J", 20)),
        &round9(&heads),
        "a join of every head's parts",
    );
}

#[test]
fn a_per_head_matmul_multiplies_each_head_by_its_own_block() {
    let (xs, w) = (x("X", 2 * 6), x("W", 6 * 4));
    let want: Vec<f64> = (0..2)
        .flat_map(|t| {
            let (xs, w) = (&xs, &w);
            (0..3).flat_map(move |h| (0..4).map(move |r| (0..2).map(|e| xs[t * 6 + h * 2 + e] * w[(h * 2 + e) * 4 + r]).sum::<f64>()))
        })
        .collect();
    close(&shown(&program(&[("X", 2, 6), ("W", 6, 4)], "Let Y be X matmul W across 3 heads.\n", "Y", 24)), &round9(&want), "three heads");
}

#[test]
fn an_attention_reads_values_narrower_than_its_keys_at_the_scale_it_is_given() {
    // Two query heads over one key/value head: keys 4 wide, values 2 wide, six cached positions, a given scale.
    let (q, k, v) = (x("Q", 8), x("K", 6 * 4), x("V", 6 * 2));
    let pos = 5;
    let want: Vec<f64> = (0..2)
        .flat_map(|h| {
            let s: Vec<f64> = (0..pos).map(|j| (0..4).map(|e| q[h * 4 + e] * k[j * 4 + e]).sum::<f64>() * 0.3).collect();
            let m = s.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let z: f64 = s.iter().map(|v| (v - m).exp()).sum();
            let v = &v;
            (0..2).map(move |e| (0..pos).map(|j| (s[j] - m).exp() / z * v[j * 2 + e]).sum::<f64>()).collect::<Vec<_>>()
        })
        .collect();
    let src = program(
        &[("Q", 1, 8), ("K", 6, 4), ("V", 6, 2)],
        "Let pos be 5.\nLet C be attention of Q over K and V with 2 heads 1 groups up to pos scaled by 0.3.\n",
        "C",
        4,
    );
    close(&shown(&src), &round9(&want), "values two wide under keys four wide");
}

/// One absorbed latent-attention decode step and the expanded attention it computes. Two heads, a 4-wide latent `c` with
/// a 2-wide shared rotated key `pe`, 3 unrotated and 2 rotated query dims per head, values 3 wide per head; `Wkb`/`Wvb`
/// the two halves of each head's decompression. The cache holds `[c ‖ pe]` per position; the expanded form decompresses
/// every cached latent per head and attends as ordinary attention. `(program, the expanded attention's values)`.
fn absorbed() -> (String, Vec<f64>) {
    let (h, nope, rope, lora, vd, len) = (2usize, 3usize, 2usize, 4usize, 3usize, 5usize);
    let (q, cache, wkb, wvb) = (x("Qf", h * (nope + rope)), x("Kc", len * (lora + rope)), x("Wkb", h * nope * lora), x("Wvb", h * lora * vd));
    let scale = 0.37;
    // Expanded: key_h,j = [Wkb_h · c_j ‖ pe_j], value_h,j = Wvb_hᵀ-wise c_j (out[e] = Σ_r c_r · Wvb[(h·lora + r)·vd + e]).
    let want: Vec<f64> = (0..h)
        .flat_map(|hh| {
            let (q, cache, wkb, wvb) = (&q, &cache, &wkb, &wvb);
            let c = |j: usize| &cache[j * (lora + rope)..j * (lora + rope) + lora];
            let pe = |j: usize| &cache[j * (lora + rope) + lora..(j + 1) * (lora + rope)];
            let key = |j: usize| -> Vec<f64> {
                (0..nope).map(|e| (0..lora).map(|r| wkb[(hh * nope + e) * lora + r] * c(j)[r]).sum::<f64>()).chain(pe(j).iter().copied()).collect()
            };
            let qh = &q[hh * (nope + rope)..(hh + 1) * (nope + rope)];
            let s: Vec<f64> = (0..len).map(|j| qh.iter().zip(key(j)).map(|(a, b)| a * b).sum::<f64>() * scale).collect();
            let m = s.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let z: f64 = s.iter().map(|v| (v - m).exp()).sum();
            (0..vd)
                .map(move |e| {
                    (0..len)
                        .map(|j| (s[j] - m).exp() / z * (0..lora).map(|r| c(j)[r] * wvb[(hh * lora + r) * vd + e]).sum::<f64>())
                        .sum::<f64>()
                })
                .collect::<Vec<_>>()
        })
        .collect();
    let body = format!(
        "Let qn be the first {nope} of every {qk} of Qf.\n\
         Let qr be the last {rope} of every {qk} of Qf.\n\
         Let ql be qn matmul Wkb across {h} heads.\n\
         Let qa be ql joined with qr across {h} heads.\n\
         Let Vc be the first {lora} of every {lk} of Kc.\n\
         Let pos be {len}.\n\
         Let ol be attention of qa over Kc and Vc with {h} heads 1 groups up to pos scaled by {scale:?}.\n\
         Let O be ol matmul Wvb across {h} heads.\n",
        qk = nope + rope,
        lk = lora + rope,
    );
    let src = program(&[("Qf", 1, h * (nope + rope)), ("Kc", len, lora + rope), ("Wkb", h * nope, lora), ("Wvb", h * lora, vd)], &body, "O", h * vd);
    (src, want)
}

#[test]
fn absorbed_latent_attention_is_the_expanded_attention() {
    let (src, want) = absorbed();
    close(&shown(&src), &round9(&want), "the absorbed step");
}

/// The SN50 descriptor with Fp32 wherever it offers a float: the compiler carries every activation at Fp32, so a
/// lowering's arithmetic is its wiring, not its rounding.
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

/// The spatial IR lowers an attention whose values are narrower than its keys at the scale it is given, and the whole
/// absorbed step, on the program path a model takes; their `f32` twin shows what the VM shows to a relative 2⁻¹⁶.
#[test]
fn the_spatial_ir_computes_the_narrower_values_and_the_absorbed_step() {
    let narrow = program(
        &[("Q", 1, 8), ("K", 6, 4), ("V", 6, 2)],
        "Let pos be 5.\nLet C be attention of Q over K and V with 2 heads 1 groups up to pos scaled by 0.3.\n",
        "C",
        4,
    );
    for (what, src) in [("narrower values at a given scale", narrow), ("the absorbed step", absorbed().0)] {
        let pl = compile_program_to_spatial(&src, &fp32_twin()).unwrap_or_else(|e| panic!("{what}: {e:?}"));
        let sp = pl.whole().unwrap_or_else(|| panic!("{what}: {}", pl.refusal()));
        let out = eval_choreography(sp, &inputs_from_fills(sp).unwrap()).unwrap_or_else(|e| panic!("{what}: {e:?}"));
        let got = shown_from_outputs(&src, sp, &out).unwrap();
        let vm = oracle_ints(&vm_outcome(&src).output).unwrap();
        assert_eq!(got.len(), vm.len(), "{what}");
        for (g, v) in got.iter().zip(&vm) {
            assert!(((g - v).abs() as f64) <= (v.abs() as f64 / 65536.0).max(1.0), "{what}: {got:?} vs the VM's {vm:?}");
        }
    }
}

#[test]
fn the_compiled_tiers_compute_every_piece() {
    let src = program(
        &[("Q", 2, 12), ("A", 2, 6), ("B", 2, 4), ("X", 2, 6), ("W", 6, 4)],
        "Let P be the first 3 of every 4 of Q.\nLet J be A joined with B across 2 heads.\nLet Y be X matmul W across 3 heads.\n\
         Let Z be P joined with Y.\n",
        "Z",
        2 * (9 + 12),
    );
    let want = vm_outcome(&src).output;
    common::assert_c_output(&src, want.trim());
    common::assert_exact_output(&src, want.trim());
}

#[test]
fn the_front_end_lifts_every_piece() {
    let src = program(
        &[("Q", 2, 12), ("A", 2, 6), ("B", 2, 4), ("X", 2, 6), ("W", 6, 4), ("K", 6, 4), ("V", 6, 2), ("Qa", 1, 8)],
        "Let P be the last 1 of every 4 of Q.\nLet J be A joined with B across 2 heads.\nLet Y be X matmul W across 3 heads.\n\
         Let pos be 5.\nLet C be attention of Qa over K and V with 2 heads 1 groups up to pos scaled by 0.3.\n",
        "C",
        4,
    );
    let rp = recognize_stages(&src).expect("parses");
    assert!(rp.unrecognized.is_empty(), "{:?}", rp.unrecognized);
    let ops: Vec<TensorOpSpec> = rp.stages.into_iter().map(|s| s.op).collect();
    let slice = ops.iter().find_map(|o| if let TensorOpSpec::HeadSlice(s) = o { Some(s.clone()) } else { None }).expect("a slice");
    assert_eq!((slice.every, slice.from, slice.width, slice.n), (4, 3, 1, 24));
    let join = ops.iter().find_map(|o| if let TensorOpSpec::Join(j) = o { Some(j.clone()) } else { None }).expect("a join");
    assert_eq!((join.rows, join.heads, join.wa, join.wb), (2, 2, 6, 4));
    let mm = ops.iter().find_map(|o| if let TensorOpSpec::HeadMatmul(m) = o { Some(m.clone()) } else { None }).expect("a per-head matmul");
    assert_eq!((mm.m, mm.heads, mm.a, mm.b), (2, 3, 2, 4));
    let at = ops.iter().find_map(|o| if let TensorOpSpec::Attention(a) = o { Some(a.clone()) } else { None }).expect("an attention");
    assert_eq!((at.hd, at.vd), (4, 2), "keys four wide, values two");
    assert_eq!(at.scale.map(|s| s.get()), Some(0.3));
}

#[test]
fn a_piece_whose_shape_does_not_fit_is_refused() {
    for (decl, body, why) in [
        ("Let Q be a 2 by 12 tensor of Real.", "Let P be the first 5 of every 4 of Q.", "first 5 of every 4"),
        ("Let Q be a 2 by 10 tensor of Real.", "Let P be the first 2 of every 4 of Q.", "every 4"),
        ("Let A be a 2 by 6 tensor of Real.\nLet B be a 3 by 4 tensor of Real.", "Let J be A joined with B.", "rows"),
        ("Let A be a 2 by 6 tensor of Real.\nLet B be a 2 by 5 tensor of Real.", "Let J be A joined with B across 2 heads.", "2 heads"),
        ("Let X be a 2 by 6 tensor of Real.\nLet W be a 5 by 4 tensor of Real.", "Let Y be X matmul W across 3 heads.", "3 heads"),
    ] {
        let src = format!("## Main\n{decl}\n{body}\nShow 1.\n");
        let err = vm_outcome(&src).error.unwrap_or_else(|| panic!("`{body}` is refused"));
        assert!(err.contains(why), "`{body}` is refused naming {why}: {err}");
    }
}
