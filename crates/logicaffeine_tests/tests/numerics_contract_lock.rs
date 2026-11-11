//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! EVERY APPROXIMATION A CHIP MAKES IS DECLARED AND HELD (benchmarks/FUSIONS_WE_NEED.md, OPT-6).
//!
//! Each chip's shim declares the numerics contract of the knob point it answers for (`tensor_graph::numerics::Contract`):
//! the scheme a product by a block-format matrix reads its activation in, how the chip keeps a cache's rows and a
//! recurrent memory, how far a fused op may lie from its parts run apart, and the widest integer it accumulates in. Each
//! scheme's and each storage format's bound holds of the meaning the reference computes — the model reference's int8
//! activations, the D5005 engine's power-of-two codes, binary16 and bfloat16 — and is near tight. A product that reads
//! a quantized activation reads the form its producer, or a quantize of its own, wrote in the scheme its chip declares.
//! Every chip computes each fusion it makes as its parts compute it apart, within its declared bound: the meaning under
//! every shim, the D5005 machine with its vector jobs taken apart and its merged projections row by row, the spatial
//! chips' joined sections, and the GPU chips' programs apart, held on the device by the suites this lock names.
//!
//! ★ MUTATION PROBE: halve a scheme's step → the bound fails on a value the reference quantizes.
//! ★ MUTATION PROBE: let the machine's store keep a slot's elements in binary16 → the machine taken apart, whose parts
//!   pass their values through the store, parts from the machine.
//! ★ MUTATION PROBE: scale every row of a block-format product by the matrix's largest block scale → a projection of a
//!   merged matrix moves with the rows outside it.
//! ★ MUTATION PROBE: declare the lane's one-word activations as two words → the lane's contract no longer follows its
//!   knobs, and a product reads a form its chip does not declare.
//! ★ MUTATION PROBE: give two 16-bit words the bound of a word of `2p` significand bits → a value the reference reads in
//!   two words lies past it.
//!
//!   cargo nextest run -p logicaffeine-tests --test numerics_contract_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

#[path = "common/lane_corpus.rs"]
mod lane_corpus;
#[path = "common/lane_cuts.rs"]
mod lane_cuts;
#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use std::collections::BTreeSet;

use logicaffeine_compile::codegen_cuda::forward_shim::ForwardShim;
use logicaffeine_compile::codegen_cuda::knobs::{CachePrecision, CudaKnobs, StatePrecision};
use logicaffeine_compile::codegen_cuda::lane_shim::LaneShim;
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::model::reference::{quantize_activations, sixteen_bit_activations, Activations};
use logicaffeine_compile::spatial::decode_engine::model::EngineNumerics;
use logicaffeine_compile::spatial::decode_engine::numerics::{cache_row, hadamard, quantize, quantize_blocks, FpUnit, Words};
use logicaffeine_compile::spatial::decode_engine::shim::EngineShim;
use logicaffeine_compile::spatial::shim::SpatialShim;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_graph::fuse::{leaves, optimize};
use logicaffeine_compile::tensor_graph::graph::{QuantFormat, Sym, TensorGraph};
use logicaffeine_compile::tensor_graph::numerics::{AttentionNumerics, Block, Bound, Keep, Scale, Scheme, SixteenBit, Taken, BLOCK};
use logicaffeine_compile::tensor_graph::shim::{ReferenceShim, Shim};

use lane_corpus::every_lane_cut;

/// Every shim the compiler declares, under every configuration that changes its contract.
fn shims() -> Vec<Box<dyn Shim>> {
    let mut out: Vec<Box<dyn Shim>> = Vec::new();
    for activations in [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        for cache_precision in [CachePrecision::F32, CachePrecision::F16, CachePrecision::E4m3] {
            for recurrent_state in [StatePrecision::F32, StatePrecision::Bf16] {
                let knobs = CudaKnobs { activations, cache_precision, recurrent_state, ..CudaKnobs::default() };
                out.push(Box::new(LaneShim { knobs, batch: false, held: Vec::new() }));
            }
        }
        out.push(Box::new(ForwardShim { kind: activations }));
    }
    for target in [SpatialTarget::wormhole(), SpatialTarget::sky_fabric(64, 64), SpatialTarget::sn50_rdu(), SpatialTarget::rtx3060()] {
        out.push(Box::new(SpatialShim { target: Box::leak(Box::new(target)) }));
    }
    for numerics in EngineNumerics::real_points() {
        out.push(Box::new(EngineShim { spec: Box::leak(Box::new(tiny_models::Tiny::Qwen35.spec(24))), numerics }));
    }
    out.push(Box::new(ReferenceShim));
    out
}

/// The chips whose shim declares no contract yet, by the start of the shim's name, each with what declares it. The
/// list only shrinks.
const AWAITING_A_CONTRACT: &[(&str, &str)] = &[];

/// Values to quantize: blocks spanning magnitudes from a floor's worth to binary16's range, both signs, a block of
/// zeros, a block with one outlier, a block of near-ties — `len` values a block, `blocks` blocks.
fn blocks(len: usize, blocks: usize) -> Vec<Vec<f64>> {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let scales = [1e-30, 1e-12, 1e-6, 1e-3, 0.25, 1.0, 7.3, 1e3, 3e4];
    let mut out = vec![vec![0.0; len]];
    let mut outlier: Vec<f64> = (0..len).map(|i| ((i % 7) as f64 - 3.0) * 1e-3).collect();
    outlier[len / 2] = 5e2;
    out.push(outlier);
    out.push((0..len).map(|i| (i as f64 + 0.5) / 127.0 * if i % 2 == 0 { 1.0 } else { -1.0 }).collect());
    for b in 0..blocks {
        let s = scales[b % scales.len()];
        out.push((0..len).map(|_| (next() * 2.0 - 1.0) * s).collect());
    }
    out
}

fn block_max(block: &[f64]) -> f64 {
    block.iter().fold(0.0f64, |m, v| m.max(v.abs()))
}

/// The largest share of `bound`'s block term an error of `kept` from `exact` reaches, among blocks the block term bounds
/// (not ones so small the floor does).
fn worst(bound: &Bound, exact: &[f64], kept: &[f64], max: f64, what: &str) -> f64 {
    let mut worst = 0.0f64;
    for (i, (&x, &k)) in exact.iter().zip(kept).enumerate() {
        assert!(bound.holds(x, k, max), "{what}: value {i}, {x:e}, kept as {k:e}, lies {:e} away, past {bound:?} of a block whose largest is {max:e}", (x - k).abs());
        if bound.of_block * max > 16.0 * bound.floor {
            worst = worst.max((x - k).abs() / (bound.of_block * max));
        }
    }
    worst
}

/// Each scheme a chip declares, and each storage format, keeps its bound on every value of the reference's own
/// quantizing, and reaches at least two fifths of it somewhere: the bound is the format's, not a guess.
fn hold_scheme(scheme: Scheme) {
    let bound = scheme.bound();
    let mut reached = 0.0f64;
    match (scheme.block, scheme.scale) {
        (Block::Values(BLOCK), Scale::MaxOver127) => {
            let mode = if scheme.words == 1 { Activations::Int8 } else { Activations::Int8TwoPass };
            for b in blocks(BLOCK, 400) {
                let kept = quantize_activations(&b, mode);
                let max = block_max(&b.iter().map(|&v| f64::from(v as f32)).collect::<Vec<_>>());
                reached = reached.max(worst(&bound, &b, &kept, max, &format!("{scheme:?}")));
            }
        }
        (Block::Row, Scale::MaxOver127) => {
            // The lane's attention: a head's queries, a cached key or value row, a query's weights over a tile of keys.
            for len in [32usize, 64, 128, 256] {
                for b in blocks(len, 100) {
                    let row: Vec<f32> = b.iter().map(|&v| v as f32).collect();
                    let exact: Vec<f64> = row.iter().map(|&v| f64::from(v)).collect();
                    let max = block_max(&exact);
                    let forms = match scheme.words {
                        2 => vec![lane_two_words(&row, false), lane_two_words(&row, true)],
                        3 => vec![lane_three_words(&row)],
                        w => panic!("the lane takes no attention operand in {w} words"),
                    };
                    for kept in forms {
                        reached = reached.max(worst(&bound, &exact, &kept, max, &format!("{scheme:?}")));
                    }
                }
            }
        }
        (Block::Values(n), Scale::LeastPowerOfTwoOver255) => {
            // The engine's attention weights, each times its value row's scale: never negative.
            for b in blocks(n, 400) {
                let w: Vec<f32> = b.iter().map(|&v| (v as f32).abs()).collect();
                let exact: Vec<f64> = w.iter().map(|&v| f64::from(v)).collect();
                let max = block_max(&exact);
                reached = reached.max(worst(&bound, &exact, &engine_weights(&w), max, &format!("{scheme:?}")));
            }
        }
        (block, Scale::LeastPowerOfTwo { significand_bits }) => {
            let words = if scheme.words == 1 { Words::One } else { Words::Two };
            let len = match block {
                Block::Values(n) => n,
                Block::OfTheWeights => 256,
                Block::Row => 128,
            };
            for b in blocks(len, 400) {
                // The engine reads its activations in f32; a row is a whole number of its groups, a head the block of
                // the query its attention takes.
                let row: Vec<f32> = b.iter().cycle().take(if block == Block::Row { len } else { len.max(256) }).map(|&v| v as f32).collect();
                let act = match (block, significand_bits) {
                    (Block::Values(32), 4) => quantize(&row, words),
                    (Block::OfTheWeights, 8) => quantize_blocks(&row, words, 256),
                    (Block::Row, 8) => quantize_blocks(&row, words, row.len()),
                    _ => panic!("{scheme:?} is no scheme the engine quantizes in"),
                };
                let kept = act.values();
                for (k, chunk) in row.chunks(act.block).enumerate() {
                    let exact: Vec<f64> = chunk.iter().map(|&v| f64::from(v)).collect();
                    let max = block_max(&exact);
                    let got = &kept[k * act.block..(k + 1) * act.block];
                    reached = reached.max(worst(&bound, &exact, got, max, &format!("{scheme:?}")));
                }
            }
        }
        other => panic!("{other:?} is no scheme a chip quantizes in"),
    }
    assert!(reached >= 0.4, "{scheme:?}: the worst error reaches {reached:.3} of the bound's step — the bound is not the format's");
}

/// The lane's two-word quantizer in `f32`, bit for bit: a row's scale `max / 127` (`lk_two_scale`), each value's two
/// words by quotients (`lk_two_words`, the queries') or by products with the scale's reciprocal (`lk_two_words_r`, the
/// keys' and values') — the value the two words hold.
fn lane_two_words(row: &[f32], by_products: bool) -> Vec<f64> {
    let a = row.iter().fold(0f32, |m, v| m.max(v.abs()));
    let s = (if a > 1e-30 { a } else { 1e-30 }) / 127.0;
    let r = 1.0 / s;
    let word = |x: f32| x.round_ties_even().clamp(-128.0, 127.0);
    row.iter()
        .map(|&v| {
            let h = word(if by_products { v * r } else { v / s });
            let rest = (-h).mul_add(s, v);
            let l = word(if by_products { rest * r * 128.0 } else { rest / (s * 0.0078125) });
            f64::from(h) * f64::from(s) + f64::from(l) * f64::from(s * 0.0078125)
        })
        .collect()
}

/// The lane's three-word quantizer of an attention's weights (`lk_attention_i8`) in `f32`, bit for bit: the row's scale
/// `max / 127`, each weight's high, low and lowest words by products with its reciprocal, each rest one fused
/// multiply-add — the value the three words hold.
fn lane_three_words(row: &[f32]) -> Vec<f64> {
    let a = row.iter().fold(0f32, |m, v| m.max(v.abs()));
    let s = (if a > 1e-30 { a } else { 1e-30 }) / 127.0;
    let r = 1.0 / s;
    row.iter()
        .map(|&e| {
            let h = (e * r).round_ties_even();
            let r1 = (-h).mul_add(s, e);
            let l = (r1 * r * 128.0).round_ties_even();
            let q = ((-l).mul_add(s * 0.0078125, r1) * r * 16384.0).round_ties_even();
            f64::from(h) * f64::from(s) + f64::from(l) * f64::from(s * 0.0078125) + f64::from(q) * f64::from(s) / 16384.0
        })
        .collect()
}

/// The engine's attention weights as its array takes them (`numerics::weighted_int8`): the least `e` with the block's
/// largest at most `255·2^e`, each weight's unsigned code the nearest whole number of `2^e` — the value the code holds.
fn engine_weights(w: &[f32]) -> Vec<f64> {
    let wmax = w.iter().fold(0f32, |m, &v| m.max(v));
    if wmax == 0.0 {
        return vec![0.0; w.len()];
    }
    let mut e = (f64::from(wmax) / 255.0).log2().ceil() as i32;
    while 255.0 * 2f64.powi(e) < f64::from(wmax) {
        e += 1;
    }
    while 255.0 * 2f64.powi(e - 1) >= f64::from(wmax) {
        e -= 1;
    }
    w.iter().map(|&v| (f64::from(v) * 2f64.powi(-e)).round_ties_even() * 2f64.powi(e)).collect()
}

/// How a product by a 16-bit float matrix reads its activation in `words` words of the matrix's own format holds its
/// bound for both formats, bfloat16 and binary16, on every value of the reference's own reading below the format's
/// range, and reaches at least two fifths of it somewhere: the bound is the words', not a guess.
fn hold_sixteen(sixteen: SixteenBit) {
    let mode = match sixteen.words {
        1 => Activations::Int8,
        2 => Activations::Int8TwoPass,
        w => panic!("a 16-bit product reads its activation in no {w} words"),
    };
    for (ty, format) in [(GgmlType::Bf16, Keep::Bf16), (GgmlType::F16, Keep::F16)] {
        let bound = sixteen.bound(format);
        let mut reached = 0.0f64;
        for b in blocks(64, 400) {
            let b: Vec<f64> = b.into_iter().filter(|v| v.abs() <= format.range()).collect();
            let kept = sixteen_bit_activations(&b, ty, mode);
            for (&x, &k) in b.iter().zip(&kept) {
                assert!(bound.holds(x, k, 0.0), "{sixteen:?} of {ty:?}: {x:e} read as {k:e} lies {:e} away, past {bound:?}", (x - k).abs());
                if x.abs() * bound.of_value > 16.0 * bound.floor {
                    reached = reached.max((x - k).abs() / (bound.of_value * x.abs()));
                }
            }
        }
        assert!(reached >= 0.4, "{sixteen:?} of {ty:?}: the worst error reaches {reached:.3} of the bound — the bound is not the words'");
    }
}

/// How an attention takes an operand holds its bound: a kept float format's, or a scheme's.
fn hold_taken(t: Taken) {
    match t {
        Taken::Kept(k) => hold_keep(k),
        Taken::Quantized(s) => hold_scheme(s),
    }
}

fn hold_keep(keep: Keep) {
    let bound = keep.bound();
    if matches!(keep, Keep::Int8Rows | Keep::Int8RowsRotated) {
        // A row as the engine stores it: its codes and scale from the engine's own `cache_row`, a rotated row turned
        // first by the engine's `hadamard` and read back through the exact inverse turn.
        let mut reached = 0.0f64;
        let mut rows = blocks(64, 200);
        if keep == Keep::Int8RowsRotated {
            // The worst a rotated row can do: every code rounded down by 0.49 of a level along one Hadamard row, the
            // scale exactly one (the turned row's largest is 127), so the inverse turn gathers the errors on that row's
            // element instead of averaging them away.
            let (n, k) = (64usize, 5usize);
            let sign = |j: usize| if (k & j).count_ones() % 2 == 0 { 1.0 } else { -1.0 };
            let turned: Vec<f64> = (0..n).map(|j| if j == 0 { 127.0 } else { sign(j) * ((j % 120) as f64 + 0.49) }).collect();
            rows.push((0..n).map(|i| (0..n).map(|j| if (i & j).count_ones() % 2 == 0 { turned[j] } else { -turned[j] }).sum::<f64>() / n as f64).collect());
        }
        for b in rows {
            let row: Vec<f32> = b.iter().map(|&v| v as f32).collect();
            let mut stored = row.clone();
            if keep == Keep::Int8RowsRotated {
                hadamard(&mut stored);
            }
            let (s, codes) = cache_row(FpUnit::Soft, &stored);
            let scale = f64::from(logicaffeine_compile::model::quant::f16_to_f32(s));
            let mut kept: Vec<f64> = codes.iter().map(|&c| f64::from(c) * scale).collect();
            if keep == Keep::Int8RowsRotated {
                let n = kept.len();
                let mut h = 1;
                while h < n {
                    for start in (0..n).step_by(2 * h) {
                        for j in start..start + h {
                            let (a, c) = (kept[j], kept[j + h]);
                            kept[j] = a + c;
                            kept[j + h] = a - c;
                        }
                    }
                    h *= 2;
                }
                kept.iter_mut().for_each(|v| *v /= n as f64);
            }
            let exact: Vec<f64> = row.iter().map(|&v| f64::from(v)).collect();
            let max = block_max(&stored.iter().map(|&v| f64::from(v)).collect::<Vec<_>>());
            reached = reached.max(worst(&bound, &exact, &kept, max, &format!("{keep:?}")));
        }
        assert!(reached >= 0.4, "{keep:?}: the worst error reaches {reached:.3} of the bound's step — the bound is not the format's");
        return;
    }
    for b in blocks(64, 200) {
        let b: Vec<f64> = b.into_iter().filter(|v| v.abs() <= keep.range()).collect();
        let kept: Vec<f64> = b.iter().map(|&v| keep.keep(v).expect("a float keeps a value at a time")).collect();
        worst(&bound, &b, &kept, block_max(&b), &format!("{keep:?}"));
    }
    if keep == Keep::F32 {
        // A chip that flushes a denormal to zero, as the engine's f32 does, keeps it within the floor.
        for v in [1e-40f64, -3e-39, 1e-45, 1.1e-38] {
            let flushed = f64::from(logicaffeine_compile::spatial::decode_engine::numerics::ftz(v as f32));
            assert!(bound.holds(v, flushed, 0.0), "F32: {v:e} flushed to {flushed:e} lies past the floor {bound:?}");
        }
    }
    if keep == Keep::F16 {
        for v in [1.0f64, 1.0 + 2f64.powi(-12), 65504.0, 2f64.powi(-24), -3.25] {
            let half = logicaffeine_compile::model::quant::f16_to_f32(logicaffeine_compile::model::quant::f32_to_f16(v as f32));
            assert_eq!(keep.keep(v), Some(f64::from(half)), "binary16 keeps {v} as the model's own conversion does");
        }
    }
    if keep == Keep::E4m3 {
        use logicaffeine_compile::model::quant::{e4m3_to_f32, f32_to_e4m3};
        for v in [1.0f64, 1.0625, 1.1875, 448.0, 2f64.powi(-9), 3.0 * 2f64.powi(-10), -3.25, 0.0] {
            assert_eq!(keep.keep(v), Some(f64::from(e4m3_to_f32(f32_to_e4m3(v as f32)))), "e4m3 keeps {v} as the model's own conversion does");
        }
        // Past its range the format saturates: every value beyond ±448, infinities included, keeps ±448.
        for v in [448.5f64, 464.0, 1e3, 3e4, f64::INFINITY] {
            assert_eq!(keep.keep(v), Some(keep.range()), "e4m3 saturates {v} at its range");
            assert_eq!(keep.keep(-v), Some(-keep.range()), "e4m3 saturates {} at its range", -v);
        }
    }
}

#[test]
fn every_knob_point_declares_its_bound() {
    let shims = shims();
    let mut points = BTreeSet::new();
    for s in &shims {
        let name = s.name();
        let awaiting = AWAITING_A_CONTRACT.iter().find(|(prefix, _)| name.starts_with(prefix));
        let Some(c) = s.numerics() else {
            assert!(awaiting.is_some(), "`{name}` declares no numerics contract");
            continue;
        };
        assert!(awaiting.is_none(), "`{name}` declares its contract now: take it off AWAITING_A_CONTRACT");
        assert!(!c.point.is_empty(), "`{name}` names its knob point");
        assert!(points.insert(c.point.clone()), "`{name}`: two knob points named `{}`", c.point);
        assert!(matches!(c.accumulator_bits, 32 | 64), "`{name}` accumulates integers in {} bits", c.accumulator_bits);
        if let Some(scheme) = c.activations {
            hold_scheme(scheme);
        }
        if let Some(sixteen) = c.sixteen {
            hold_sixteen(sixteen);
        }
        hold_keep(c.cache);
        hold_keep(c.state);
        if let Some(a) = c.attention {
            for t in [a.queries, a.keys_values, a.weights] {
                hold_taken(t);
            }
        }
    }
    for (prefix, why) in AWAITING_A_CONTRACT {
        assert!(shims.iter().any(|s| s.name().starts_with(prefix)), "`{prefix}` awaits a contract ({why}) and no shim of it is listed");
    }
    // Every scheme a chip can quantize in, declared or not yet: the lane's two and its attention's two and three words,
    // the engine's four and its attention's whole-head queries and unsigned weights.
    for scheme in [
        Scheme::of(QuantFormat::OneWord),
        Scheme::of(QuantFormat::TwoWords),
        Scheme { block: Block::Values(32), scale: Scale::LeastPowerOfTwo { significand_bits: 4 }, words: 1 },
        Scheme { block: Block::Values(32), scale: Scale::LeastPowerOfTwo { significand_bits: 4 }, words: 2 },
        Scheme { block: Block::OfTheWeights, scale: Scale::LeastPowerOfTwo { significand_bits: 8 }, words: 1 },
        Scheme { block: Block::OfTheWeights, scale: Scale::LeastPowerOfTwo { significand_bits: 8 }, words: 2 },
        Scheme { block: Block::Row, scale: Scale::LeastPowerOfTwo { significand_bits: 8 }, words: 1 },
        Scheme { block: Block::Row, scale: Scale::LeastPowerOfTwo { significand_bits: 8 }, words: 2 },
        Scheme { block: Block::Row, scale: Scale::MaxOver127, words: 2 },
        Scheme { block: Block::Row, scale: Scale::MaxOver127, words: 3 },
        Scheme { block: Block::Values(32), scale: Scale::LeastPowerOfTwoOver255, words: 1 },
    ] {
        hold_scheme(scheme);
    }
    for keep in [Keep::Exact, Keep::F32, Keep::F16, Keep::Bf16, Keep::E4m3, Keep::Int8Rows, Keep::Int8RowsRotated] {
        hold_keep(keep);
    }
    // Every reading of a 16-bit product's activation a chip can take: one word of the matrix's format and two.
    for words in [1, 2] {
        hold_sixteen(SixteenBit { words });
    }
}

/// The lane's contract follows its knobs: the activations its products read, the cache and the memory it keeps.
#[test]
fn the_lanes_contract_follows_its_knobs() {
    for activations in [Q8MatKind::Dequant, Q8MatKind::Int8Act, Q8MatKind::TwoPassInt8] {
        for cache_precision in [CachePrecision::F32, CachePrecision::F16, CachePrecision::E4m3] {
            for recurrent_state in [StatePrecision::F32, StatePrecision::Bf16] {
                let knobs = CudaKnobs { activations, cache_precision, recurrent_state, ..CudaKnobs::default() };
                let c = LaneShim { knobs, batch: false, held: Vec::new() }.numerics().expect("the lane declares its contract");
                let scheme = match activations {
                    Q8MatKind::Dequant => None,
                    Q8MatKind::Int8Act => Some(Scheme::of(QuantFormat::OneWord)),
                    Q8MatKind::TwoPassInt8 => Some(Scheme::of(QuantFormat::TwoWords)),
                };
                assert_eq!(c.activations, scheme, "{knobs:?}");
                // A product by a 16-bit float matrix reads its activation `f32` under the precise numerics, and on the
                // tensor cores in one word of the matrix's format or two under the quantized ones.
                let sixteen = match activations {
                    Q8MatKind::Dequant => None,
                    Q8MatKind::Int8Act => Some(SixteenBit { words: 1 }),
                    Q8MatKind::TwoPassInt8 => Some(SixteenBit { words: 2 }),
                };
                assert_eq!(c.sixteen, sixteen, "{knobs:?}: a 16-bit product's activation");
                let row = |words| Taken::Quantized(Scheme { block: Block::Row, scale: Scale::MaxOver127, words });
                let attention = match activations {
                    Q8MatKind::Dequant => None,
                    Q8MatKind::Int8Act => Some(AttentionNumerics { queries: Taken::Kept(Keep::F16), keys_values: Taken::Kept(Keep::F16), weights: Taken::Kept(Keep::F16) }),
                    Q8MatKind::TwoPassInt8 => Some(AttentionNumerics { queries: row(2), keys_values: row(2), weights: row(3) }),
                };
                assert_eq!(c.attention, attention, "{knobs:?}");
                let cache = match cache_precision {
                    CachePrecision::F32 => Keep::F32,
                    CachePrecision::F16 => Keep::F16,
                    CachePrecision::E4m3 => Keep::E4m3,
                };
                assert_eq!(c.cache, cache, "{knobs:?}");
                assert_eq!(c.state, if recurrent_state == StatePrecision::Bf16 { Keep::Bf16 } else { Keep::F32 }, "{knobs:?}");
            }
        }
    }
}

/// Each attention the lane narrows is held on the device to the operands its contract declares: the f16 tensor-core
/// attention against the attention of its operands rounded to f16, the two-word one against the attention of its
/// operands in two words and its weights in three, every product of the words taken.
#[test]
fn the_lanes_narrowed_attentions_are_held_on_the_device() {
    let suite = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cuda_lane_lock.rs")).expect("the lane's suite");
    for (activations, test) in [
        (Q8MatKind::Int8Act, "the_tensor_core_attention_computes_what_the_attention_computes"),
        (Q8MatKind::TwoPassInt8, "the_two_word_attention_computes_what_the_attention_computes"),
    ] {
        assert!(LaneShim::attention(activations).is_some(), "{activations:?}: the lane declares the attention it narrows");
        let at = suite.find(&format!("#[test]\nfn {test}()")).unwrap_or_else(|| panic!("cuda_lane_lock::{test} holds the {activations:?} attention"));
        let body = &suite[at..suite[at + 1..].find("\n#[test]").map_or(suite.len(), |e| at + 1 + e)];
        assert!(body.contains("require_gpu()"), "cuda_lane_lock::{test} runs on the device");
    }
    assert_eq!(LaneShim::attention(Q8MatKind::Dequant), None, "the dequantized numerics attend in f32");
}

/// The 16-bit words the lane's tensor cores multiply are held on the device to the reference's reading of the
/// activation: `lk_h16_words` against `sixteen_bit_activations`, bit for bit, in both formats and both word counts.
#[test]
fn the_lanes_sixteen_bit_words_are_held_on_the_device() {
    let suite = std::fs::read_to_string(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cuda_lane_lock.rs")).expect("the lane's suite");
    let test = "the_tensor_cores_sixteen_bit_words_are_the_references_reading";
    let at = suite.find(&format!("#[test]\nfn {test}()")).unwrap_or_else(|| panic!("cuda_lane_lock::{test} holds the 16-bit words"));
    let body = &suite[at..suite[at + 1..].find("\n#[test]").map_or(suite.len(), |e| at + 1 + e)];
    assert!(body.contains("require_gpu()") && body.contains("sixteen_bit_activations"), "cuda_lane_lock::{test} runs on the device against the reference");
}

/// The engine's contract follows its build's numerics at every knob point: the activation scheme from its words and its
/// scaling, the state and the cache it keeps, its integer dots exact in 64 bits.
#[test]
fn the_engines_contract_follows_its_numerics() {
    use logicaffeine_compile::spatial::decode_engine::model::{CacheFormat, StateFormat};
    use logicaffeine_compile::spatial::decode_engine::numerics::ActScaling;
    let spec = tiny_models::Tiny::Qwen35.spec(24);
    for numerics in EngineNumerics::all() {
        let c = EngineShim { spec: &spec, numerics }.numerics().expect("the engine declares its contract");
        let (block, significand_bits) = match numerics.scaling {
            ActScaling::Sub32 => (Block::Values(32), 4),
            ActScaling::PerBlock => (Block::OfTheWeights, 8),
        };
        let words = if numerics.activations == Words::One { 1 } else { 2 };
        assert_eq!(c.activations, Some(Scheme { block, scale: Scale::LeastPowerOfTwo { significand_bits }, words }), "{numerics:?}");
        assert_eq!(c.state, if numerics.state == StateFormat::F16 { Keep::F16 } else { Keep::F32 }, "{numerics:?}");
        let cache = match numerics.cache {
            CacheFormat::F16 => Keep::F16,
            CacheFormat::Int8 => Keep::Int8Rows,
            CacheFormat::Int8Hadamard => Keep::Int8RowsRotated,
        };
        assert_eq!(c.cache, cache, "{numerics:?}");
        let attention = match numerics.attention {
            logicaffeine_compile::spatial::serving::AttentionMath::Vector => None,
            logicaffeine_compile::spatial::serving::AttentionMath::Array => Some(AttentionNumerics {
                queries: Taken::Quantized(Scheme { block: if block == Block::OfTheWeights { Block::Row } else { block }, scale: Scale::LeastPowerOfTwo { significand_bits }, words }),
                keys_values: Taken::Kept(cache),
                weights: Taken::Quantized(Scheme { block: Block::Values(32), scale: Scale::LeastPowerOfTwoOver255, words: 1 }),
            }),
        };
        assert_eq!(c.attention, attention, "{numerics:?}: the array's attention");
        assert_eq!(c.sixteen, None, "{numerics:?}: a 16-bit product reads its activation in f32");
        assert_eq!(c.accumulator_bits, 64, "{numerics:?}: integer dots exact in i64");
    }
}

/// The tensor a product reads a quantized form of: the root of the in-place views at whole blocks it reads it through.
fn root(g: &TensorGraph, mut t: String) -> String {
    loop {
        let view = g.ops.iter().flat_map(|op| leaves(op.clone())).find(|op| op.out == t && matches!(op.sym, Sym::View { offset } if offset % BLOCK == 0));
        match view {
            Some(v) => t = v.ins[0].clone(),
            None => return t,
        }
    }
}

/// On every lane cut, a quantized activation names its scheme, the one its chip declares, and every product that reads
/// an activation quantized reads one an op before it quantized — the producer's epilogue, or a quantize of its own; at
/// the precise numerics nothing is quantized.
#[test]
fn a_quantized_activation_names_its_scheme_and_its_consumer_reads_it() {
    for c in every_lane_cut() {
        let mut read = 0;
        let contract = c.shim.numerics().expect("the lane declares its contract");
        let o = optimize(&c.g, &c.shim, &[]).graph;
        let mut quantized: BTreeSet<String> = BTreeSet::new();
        for op in &o.ops {
            for (k, x) in op.ins.iter().enumerate() {
                if let Some(format) = c.shim.activation(op, k, &o) {
                    assert_eq!(Some(Scheme::of(format)), contract.activations, "{}: `{}` reads `{x}` in a form its chip does not declare", c.what, op.kind());
                    let r = root(&o, x.clone());
                    assert!(quantized.contains(&r), "{}: `{}` reads `{x}` quantized, and nothing before it quantizes `{r}`", c.what, op.kind());
                    read += 1;
                }
            }
            for part in leaves(op.clone()) {
                if let Sym::Quantize(format) = part.sym {
                    assert_eq!(Some(Scheme::of(format)), contract.activations, "{}: `{}` is quantized in a scheme its chip does not declare", c.what, part.out);
                    quantized.insert(part.out.clone());
                }
            }
        }
        if contract.activations.is_none() {
            assert!(quantized.is_empty(), "{}: the precise numerics quantize {quantized:?}", c.what);
        } else {
            assert!(read > 0, "{}: a cut at quantized numerics whose products read no activation quantized", c.what);
        }
    }
}

/// Integer programs: the tensor probes over Ints, a product of an operand no range bounds, products whose partial sums
/// need more than 32 bits (of operands past int8, and of int8 operands summed long), and the int8 decode model's step.
fn integer_programs() -> Vec<(String, TensorGraph)> {
    let lift = |src: &str| {
        let rp = logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages(src).unwrap_or_else(|e| panic!("{e:?}:\n{src}"));
        TensorGraph::lift(&rp, None, None, &|_| logicaffeine_compile::tensor_graph::graph::Storage::Values).unwrap_or_else(|e| panic!("{e}:\n{src}"))
    };
    let mut out = Vec::new();
    for (name, op) in [("matmul", "A matmul B"), ("plus", "A plus B"), ("hadamard", "A hadamard B")] {
        let src = format!(
            "## Main\nLet A be a 2 by 2 tensor of Int.\nSet item 1 of A to 1.\nSet item 2 of A to 2.\nSet item 3 of A to 3.\nSet item 4 of A to 4.\nLet B be a 2 by 2 tensor of Int.\nSet item 1 of B to 5.\nSet item 2 of B to 6.\nSet item 3 of B to 7.\nSet item 4 of B to 8.\nLet C be {op}.\nShow item 1 of C.\n"
        );
        out.push((format!("probe {name}"), lift(&src)));
    }
    let mut unranged = out[0].1.clone();
    unranged.tensors.get_mut("A").expect("the probe's left operand").range = None;
    out.push(("a product of an operand no range bounds".to_string(), unranged));
    // Each product fits 32 bits; the sum of 4096 of them does not.
    let wide = "## Main\nLet A be a 1 by 4096 tensor of Int.\nSet item 1 of A to 16384.\nSet item 2 of A to -16384.\nLet B be a 4096 by 1 tensor of Int.\nSet item 1 of B to 16384.\nSet item 2 of B to -16384.\nLet C be A matmul B.\nShow item 1 of C.\n";
    out.push(("a product past 32 bits".to_string(), lift(wide)));
    // Int8 operands, which a 32-bit integer datapath takes, summed 140,000 times: past 2^31.
    let long = "## Main\nLet A be a 1 by 140000 tensor of Int.\nSet item 1 of A to 127.\nSet item 2 of A to -127.\nLet B be a 140000 by 1 tensor of Int.\nSet item 1 of B to 127.\nSet item 2 of B to -127.\nLet C be A matmul B.\nShow item 1 of C.\n";
    out.push(("an int8 product past 32 bits".to_string(), lift(long)));
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).expect("the int8 model")).expect("its JSON");
    let m = logicaffeine_compile::spatial::int8_model::Int8Model::from_json(&j).expect("the int8 model");
    out.push(("the int8 model's step".to_string(), lift(&m.decode_step_lg(0, 0, &[], &[]))));
    out
}

/// Every integer sum a chip links is proven to fit its accumulator: for every integer product, sum and requant of the
/// integer programs, each chip's shim refuses it by name, or the ranges the program proves keep every partial result
/// within the width it declares. The product past 32 bits, and a product of an operand no range bounds, are refused by
/// every chip that accumulates in 32.
#[test]
fn every_integer_accumulator_is_proven_wide_enough() {
    use logicaffeine_compile::tensor_graph::facts::{accumulator, Accumulates};
    use logicaffeine_compile::tensor_graph::shim::{allows, Allow};
    let mut wide_refused = 0;
    for (what, g) in integer_programs() {
        for s in shims() {
            let Some(c) = s.numerics() else { continue };
            for op in &g.ops {
                let acc = accumulator(op, &g);
                if acc == Accumulates::Nothing {
                    continue;
                }
                let refused = matches!(allows(&*s, op, &g), Allow::Refuse(_));
                let proven = acc.bits().is_some_and(|b| b <= c.accumulator_bits) || (acc == Accumulates::Unproven && c.accumulator_bits >= 64);
                assert!(refused || proven, "{what}: `{}` on {}: its partial sums {acc:?} and the chip's {}-bit accumulator — the chip links it unproven", op.out, s.name(), c.accumulator_bits);
                if what.contains("past 32 bits") || what == "a product of an operand no range bounds" {
                    if c.accumulator_bits > 32 {
                        continue;
                    }
                    assert!(refused, "{what}: {} accumulates in 32 bits and links it ({acc:?})", s.name());
                    wide_refused += 1;
                }
            }
        }
    }
    assert!(wide_refused > 0, "no chip accumulating in 32 bits met the product past 32 bits");
}

/// The spatial pass claims an integer product exact exactly when the accumulator fact proves its partial sums fit the
/// accumulator the pass picks on the target: the pass's claim and every chip's refusal read one proof.
#[test]
fn the_spatial_matmul_claim_is_the_accumulator_proof() {
    use logicaffeine_compile::spatial::ir::{AccPolicy, NumericsClaim};
    use logicaffeine_compile::spatial::passes::from_tensor_op::choose_matmul_numerics_ranged;
    use logicaffeine_compile::tensor_graph::facts::accumulator;
    use logicaffeine_compile::tensor_graph::front::TensorOpSpec;
    let mut judged = 0;
    for (what, g) in integer_programs() {
        for op in &g.ops {
            let Some(TensorOpSpec::Matmul(m)) = op.prim() else { continue };
            let range = |k: usize| g.tensors.get(&op.ins[k]).and_then(|t| t.range);
            let (Some(a), Some(b)) = (range(0), range(1)) else { continue };
            let need = accumulator(op, &g).bits().expect("a product of ranged integers");
            for target in [SpatialTarget::wormhole(), SpatialTarget::sky_fabric(64, 64), SpatialTarget::sn50_rdu(), SpatialTarget::rtx3060()] {
                let Ok((_, acc, _, claim)) = choose_matmul_numerics_ranged(&target, Some(a), Some(b), m.k) else { continue };
                if acc == AccPolicy::F32 {
                    continue;
                }
                assert_eq!(claim == NumericsClaim::Exact, need <= acc.bits(), "{what}: `{}` on {}: sums need {need} bits, the pass accumulates in {} and claims {claim:?}", op.out, target.name, acc.bits());
                judged += 1;
            }
        }
    }
    assert!(judged > 0, "no integer product met a spatial target's integer path");
}

// ── Fused equals apart ─────────────────────────────────────────────────────────────────────────────

#[path = "common/fusion_corpus.rs"]
mod fusion_corpus;

use logicaffeine_compile::tensor_graph::eval::{eval, Data};

/// `fused` holds `apart` within `bound`: an integer exactly, a real bit for bit where the bound is exact and otherwise
/// within it, its block the whole tensor.
fn within(bound: &Bound, apart: &Data, fused: &Data, what: &str) {
    match (apart, fused) {
        (Data::Int(a), Data::Int(f)) => assert_eq!(a, f, "{what}: apart and fused"),
        (Data::Real(a), Data::Real(f)) => {
            assert_eq!(a.len(), f.len(), "{what}: apart and fused differ in length");
            let max = a.iter().fold(0.0f64, |m, v| m.max(v.abs()));
            for (i, (&x, &y)) in a.iter().zip(f).enumerate() {
                let held = if *bound == Bound::EXACT { x.to_bits() == y.to_bits() } else { bound.holds(x, y, max) };
                assert!(held, "{what}: element {i} is {x:e} apart and {y:e} fused, past {bound:?}");
            }
        }
        _ => panic!("{what}: an integer on one side and a real on the other"),
    }
}

/// Under every chip's shim, every fusion the shared table makes means what its parts mean, within the bound the chip
/// declares: every witness evaluated as lifted and as the shim leaves it, every tensor both keep. Every chip but the
/// spatial ones (which run every fused op as its parts) takes a fusion somewhere.
fn every_shims_fusions_mean_their_parts() {
    let corpus: Vec<_> = fusion_corpus::witnesses()
        .into_iter()
        .map(|(what, g)| {
            let ins = fusion_corpus::inputs(&g);
            let apart = eval(&g, &ins).unwrap_or_else(|e| panic!("{what} apart: {e}"));
            (what, g, ins, apart)
        })
        .collect();
    for s in shims() {
        let c = s.numerics().expect("every chip declares its contract");
        let mut fused = 0;
        for (what, g, ins, apart) in &corpus {
            let o = optimize(g, &*s, &[]);
            fused += o.graph.ops.iter().filter(|op| matches!(op.sym, Sym::Fused(_))).count();
            let together = eval(&o.graph, ins).unwrap_or_else(|e| panic!("{what} under {}: {e}", s.name()));
            for (name, v) in &together {
                if let Some(a) = apart.get(name) {
                    within(&c.fused, a, v, &format!("{} on {what}: `{name}`", s.name()));
                }
            }
        }
        assert!(fused > 0 || s.name().starts_with("spatial:"), "{} fuses nothing on any witness, so its claim is unchecked", s.name());
    }
}

// ── The D5005 engine, taken apart on its own machine ───────────────────────────────────────────────

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::reference::WeightSource;
use logicaffeine_compile::model::spec::{Mixer, ModelSpec};
use logicaffeine_compile::spatial::decode_engine::machine::{Job, Load, Machine, Put, Reg, Sink, Source, StoreLayout, VectorJob, VREGS};
use logicaffeine_compile::spatial::decode_engine::model::{argmax, CacheFormat};
use logicaffeine_compile::spatial::decode_engine::program::{Command, Slot};
use logicaffeine_compile::spatial::decode_engine::vector::Op as VOp;

/// A register of the vector unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VReg {
    V(u8),
    S(u8),
}

/// The ops and loads the value register `r` holds before op `at` of `v` is made from.
fn slice(v: &VectorJob, r: VReg, at: usize, ops: &mut BTreeSet<usize>, loads: &mut BTreeSet<usize>) {
    let def = (0..at).rev().find(|&j| match r {
        VReg::V(x) => v.program[j].writes() == Some(x),
        VReg::S(x) => v.program[j].writes_scalar() == Some(x),
    });
    match def {
        Some(j) => {
            if ops.insert(j) {
                let (rv, rs) = v.program[j].reads();
                rv.into_iter().for_each(|x| slice(v, VReg::V(x), j, ops, loads));
                rs.into_iter().for_each(|x| slice(v, VReg::S(x), j, ops, loads));
            }
        }
        None => {
            let VReg::V(x) = r else { panic!("scalar register {r:?} read before an op writes it") };
            loads.insert(v.loads.iter().rposition(|l| l.reg == x).unwrap_or_else(|| panic!("vector register {x} read and never loaded")));
        }
    }
}

fn loads_slot(v: &VectorJob, s: Slot) -> bool {
    v.loads.iter().any(|l| matches!(l.from, Source::Slot { slot, .. } if slot == s))
}

fn puts_slot(v: &VectorJob, s: Slot) -> bool {
    v.puts.iter().any(|p| matches!(p.to, Sink::Slot { slot, .. } if slot == s))
}

/// `v` cut into one job for each set of its stores that share no op and no load, each its own ops and loads in their
/// order; `None` where every store shares its work, or a part would load a slot another part writes.
fn components(v: &VectorJob) -> Option<Vec<VectorJob>> {
    let slices: Vec<(BTreeSet<usize>, BTreeSet<usize>)> = v
        .puts
        .iter()
        .map(|p| {
            let (mut ops, mut loads) = (BTreeSet::new(), BTreeSet::new());
            let r = match p.reg {
                Reg::V(x) => VReg::V(x),
                Reg::S(x) => VReg::S(x),
            };
            slice(v, r, v.program.len(), &mut ops, &mut loads);
            (ops, loads)
        })
        .collect();
    let n = slices.len();
    let mut root: Vec<usize> = (0..n).collect();
    fn find(root: &[usize], mut i: usize) -> usize {
        while root[i] != i {
            i = root[i];
        }
        i
    }
    for a in 0..n {
        for b in a + 1..n {
            if !slices[a].0.is_disjoint(&slices[b].0) || !slices[a].1.is_disjoint(&slices[b].1) {
                let (x, y) = (find(&root, a), find(&root, b));
                root[x.max(y)] = x.min(y);
            }
        }
    }
    let groups: BTreeSet<usize> = (0..n).map(|i| find(&root, i)).collect();
    if groups.len() < 2 {
        return None;
    }
    let parts: Vec<VectorJob> = groups
        .iter()
        .map(|&g| {
            let members: Vec<usize> = (0..n).filter(|&i| find(&root, i) == g).collect();
            let ops: BTreeSet<usize> = members.iter().flat_map(|&i| slices[i].0.iter().copied()).collect();
            let loads: BTreeSet<usize> = members.iter().flat_map(|&i| slices[i].1.iter().copied()).collect();
            let puts: Vec<Put> = members.iter().map(|&i| v.puts[i]).collect();
            let part = VectorJob { heads: v.heads, order: v.order, fresh: Vec::new(), loads: loads.iter().map(|&l| v.loads[l]).collect(), program: ops.iter().map(|&j| v.program[j]).collect(), puts };
            VectorJob { fresh: v.fresh.iter().copied().filter(|&(s, _)| puts_slot(&part, s)).collect(), ..part }
        })
        .collect();
    let fresh: usize = parts.iter().map(|p| p.fresh.len()).sum();
    assert_eq!(fresh, v.fresh.len(), "every slot the job makes afresh is one a part of it stores");
    for (i, a) in parts.iter().enumerate() {
        for (k, b) in parts.iter().enumerate() {
            if i != k && a.puts.iter().any(|p| matches!(p.to, Sink::Slot { slot, .. } if loads_slot(b, slot))) {
                return None;
            }
        }
    }
    Some(parts)
}

/// `op` reading vector register `to` wherever it read `from`.
fn renamed(op: VOp, from: u8, to: u8) -> VOp {
    let r = |x: u8| if x == from { to } else { x };
    match op {
        VOp::Dot { a, b, d } => VOp::Dot { a: r(a), b: r(b), d },
        VOp::Sum { a, d } => VOp::Sum { a: r(a), d },
        VOp::Max { a, d } => VOp::Max { a: r(a), d },
        VOp::Vec { op, a, b, d } => VOp::Vec { op, a: r(a), b: r(b), d },
        VOp::VecScalar { op, a, s, d } => VOp::VecScalar { op, a: r(a), s, d },
        VOp::Map { f, a, d } => VOp::Map { f, a: r(a), d },
        VOp::Hadamard { a, d } => VOp::Hadamard { a: r(a), d },
        VOp::Rotate { a, cos, sin, d, neox } => VOp::Rotate { a: r(a), cos: r(cos), sin: r(sin), d, neox },
        VOp::ArgMax { a, d } => VOp::ArgMax { a: r(a), d },
        VOp::AbsMax { a, d } => VOp::AbsMax { a: r(a), d },
        VOp::Codes { a, s, d } => VOp::Codes { a: r(a), s, d },
        VOp::Scale { a, n, d } => VOp::Scale { a: r(a), n, d },
        VOp::MulAdd { a, b, c, d } => VOp::MulAdd { a: r(a), b: r(b), c: r(c), d },
        VOp::MulAddScalar { a, b, s, d } => VOp::MulAddScalar { a: r(a), b: r(b), s, d },
        op @ (VOp::Scalar { .. } | VOp::ScalarMap { .. } | VOp::Const { .. }) => op,
    }
}

/// `v` with a lane function of a slot it loads run first as a job of its own, in place on the slot, and the rest reading
/// the slot as the function left it: the function's job and the rest's. Only where the function alone reads the load,
/// no other load reads the slot, the job stores none of it, nothing after the job reads it, and the function is not the
/// job's whole work.
fn hoisted(v: &VectorJob, live_out: u32) -> Option<(VectorJob, VectorJob)> {
    if v.program.len() < 2 {
        return None;
    }
    for (j, &op) in v.program.iter().enumerate() {
        let VOp::Map { f, a, d } = op else { continue };
        if v.program[..j].iter().any(|o| o.writes() == Some(a)) || v.program.iter().enumerate().any(|(i, o)| i != j && o.reads().0.contains(&a)) {
            continue;
        }
        if v.puts.iter().any(|p| p.reg == Reg::V(a)) {
            continue;
        }
        let Some(li) = v.loads.iter().rposition(|l| l.reg == a) else { continue };
        let load = v.loads[li];
        let Source::Slot { slot, at, stride } = load.from else { continue };
        if v.loads.iter().enumerate().any(|(k, l)| k != li && matches!(l.from, Source::Slot { slot: s, .. } if s == slot)) || puts_slot(v, slot) || live_out >> slot.code() & 1 == 1 {
            continue;
        }
        let first = VectorJob {
            heads: v.heads,
            order: v.order,
            fresh: Vec::new(),
            loads: vec![Load { reg: 0, from: load.from, len: load.len }],
            program: vec![VOp::Map { f, a: 0, d: 1 }],
            puts: vec![Put { reg: Reg::V(1), to: Sink::Slot { slot, at, stride }, len: load.len }],
        };
        let fresh_reg = v.loads.iter().map(|l| l.reg).chain(v.program.iter().filter_map(|o| o.writes())).chain(v.program.iter().flat_map(|o| o.reads().0)).max().unwrap_or(0) + 1;
        assert!((fresh_reg as usize) < VREGS, "a register past the vector unit's {VREGS}");
        let mut program = v.program[..j].to_vec();
        let mut renaming = true;
        for &o in &v.program[j + 1..] {
            let o2 = if renaming { renamed(o, d, fresh_reg) } else { o };
            renaming &= o.writes() != Some(d);
            program.push(o2);
        }
        let puts = v.puts.iter().map(|&p| if renaming && p.reg == Reg::V(d) { Put { reg: Reg::V(fresh_reg), ..p } } else { p }).collect();
        let mut loads = v.loads.clone();
        loads[li].reg = fresh_reg;
        return Some((first, VectorJob { loads, program, puts, ..v.clone() }));
    }
    None
}

/// `v` with a lane function whose result it only stores run after it as a job of its own, in place on the slots it
/// stores to: the job storing the function's argument there, and the function's job. Not where the function is the
/// job's whole work.
fn sunk(v: &VectorJob) -> Option<(VectorJob, VectorJob)> {
    if v.program.len() < 2 {
        return None;
    }
    for (j, &op) in v.program.iter().enumerate() {
        let VOp::Map { f, a, d } = op else { continue };
        if a == d || v.program[j + 1..].iter().any(|o| o.reads().0.contains(&d) || o.writes() == Some(d) || o.writes() == Some(a)) {
            continue;
        }
        let stored: Vec<Put> = v.puts.iter().copied().filter(|p| p.reg == Reg::V(d)).collect();
        if stored.is_empty() || stored.iter().any(|p| !matches!(p.to, Sink::Slot { .. })) {
            continue;
        }
        let mut program = v.program.clone();
        program.remove(j);
        let puts = v.puts.iter().map(|&p| if p.reg == Reg::V(d) { Put { reg: Reg::V(a), ..p } } else { p }).collect();
        let mut then = VectorJob { heads: v.heads, order: v.order, fresh: Vec::new(), loads: Vec::new(), program: Vec::new(), puts: Vec::new() };
        for (k, p) in stored.iter().enumerate() {
            let Sink::Slot { slot, at, stride } = p.to else { unreachable!("a slot") };
            let (x, y) = (2 * k as u8, 2 * k as u8 + 1);
            then.loads.push(Load { reg: x, from: Source::Slot { slot, at, stride }, len: p.len });
            then.program.push(VOp::Map { f, a: x, d: y });
            then.puts.push(Put { reg: Reg::V(y), to: p.to, len: p.len });
        }
        return Some((VectorJob { program, puts, ..v.clone() }, then));
    }
    None
}

/// The step's jobs with every vector job taken apart as far as the vector unit's instructions allow — its independent
/// stores into jobs of their own, each lane function of a slot or into a slot into a job of its own — and, for each job,
/// the job of `jobs` it was taken from.
fn taken_apart(jobs: &[Job]) -> (Vec<Job>, Vec<usize>) {
    let mut out: Vec<(Job, usize)> = jobs.iter().cloned().enumerate().map(|(i, j)| (j, i)).collect();
    'again: loop {
        let live = StoreLayout::live_out(&out.iter().map(|(j, _)| j.clone()).collect::<Vec<_>>());
        for i in 0..out.len() {
            let (Job::Vector(v), from) = &out[i] else { continue };
            let from = *from;
            let parts: Option<Vec<VectorJob>> = components(v).or_else(|| hoisted(v, live[i]).map(|(a, b)| vec![a, b])).or_else(|| sunk(v).map(|(a, b)| vec![a, b]));
            if let Some(parts) = parts {
                out.splice(i..=i, parts.into_iter().map(|p| (Job::Vector(p), from)));
                continue 'again;
            }
        }
        return out.into_iter().unzip();
    }
}

/// The tiny hybrid model the engine runs, its attention grouped as the 27B's: four query heads over two cached heads.
fn engine_spec() -> ModelSpec {
    let mut spec = tiny_models::Tiny::Qwen35.wide(64);
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            (a.heads, a.kv_heads) = (4, 2);
        }
    }
    spec
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

/// The D5005 machine computes its units' merges as it computes their parts apart: at every real knob point, with its
/// matrices in Q8_0 and in f32, the machine whose vector jobs are taken apart ([`taken_apart`]) generates the step's
/// logits and leaves its memory as the machine does, and as the folded machine does, bit for bit, through a prompt and
/// decode steps. Every convolution, gate, gated norm, output gate, gated product, cache append and split of the step is
/// taken apart.
fn the_engine_computes_its_merges_as_their_parts() {
    let spec = engine_spec();
    let dir = tempfile::tempdir().unwrap();
    let q8 = dir.path().join("q8.gguf");
    tiny_models::write_checkpoint(&spec, &q8);
    let f32 = dir.path().join("f32.gguf");
    tiny_models::write_checkpoint_as(&spec, &f32, &|_| GgmlType::F32);
    let prompt = [3u32, 1, 4, 1, 5];
    for numerics in EngineNumerics::real_points() {
        let contract = EngineShim { spec: &spec, numerics }.numerics().expect("the engine declares its contract");
        assert_eq!(contract.fused, Bound::EXACT, "{numerics:?}: the machine's merges are claimed exact, and held bit for bit here");
        let commands = logicaffeine_compile::spatial::decode_engine::program::lower(&spec, numerics, spec.ctx_len).unwrap().program.commands;
        for path in [&q8, &f32] {
            let what = format!("{numerics:?} on {}", path.file_name().unwrap().to_string_lossy());
            let mut fused = Machine::new(spec.clone(), Gguf::open(path).unwrap(), numerics, 1, spec.ctx_len).unwrap();
            let mut folded = Machine::new(spec.clone(), Gguf::open(path).unwrap(), numerics, 1, spec.ctx_len).unwrap().folded();
            let mut apart = Machine::new(spec.clone(), Gguf::open(path).unwrap(), numerics, 1, spec.ctx_len).unwrap();
            let (jobs, from) = taken_apart(&apart.jobs);
            // Each command's jobs: one, and for a rotated cache's attention a second, the rotation back.
            let mut of_job = Vec::new();
            for (c, command) in commands.iter().enumerate() {
                let n = if matches!(command, Command::Attend { .. }) && numerics.cache == CacheFormat::Int8Hadamard { 2 } else { 1 };
                of_job.extend(std::iter::repeat(c).take(n));
            }
            assert_eq!(of_job.len(), apart.jobs.len(), "{what}: a job a command, two for a rotated attention");
            for (c, command) in commands.iter().enumerate() {
                let merged = matches!(
                    command,
                    Command::Conv { .. } | Command::Gates { .. } | Command::GatedNorm { .. } | Command::SigmoidGate { .. } | Command::SwiGlu { .. } | Command::CacheAppend { .. } | Command::Split { .. }
                );
                if merged {
                    let parts = from.iter().filter(|&&j| of_job[j] == c).count();
                    assert!(parts >= 2, "{what}: command {c} ({command:?}) is not taken apart: {parts} job");
                }
            }
            for kind in ["Conv", "Gates", "GatedNorm", "SigmoidGate", "SwiGlu", "CacheAppend", "Split"] {
                assert!(commands.iter().any(|c| format!("{c:?}").starts_with(kind)), "{what}: the step has no {kind} to take apart");
            }
            apart.jobs = jobs;
            let (a, b, c) = (fused.sequence().unwrap(), folded.sequence().unwrap(), apart.sequence().unwrap());
            let mut tokens = prompt.to_vec();
            for step in 0..6 {
                let want = fused.step(&[(a, tokens.clone())]).unwrap().remove(0);
                let fold = folded.step(&[(b, tokens.clone())]).unwrap().remove(0);
                let got = apart.step(&[(c, tokens.clone())]).unwrap().remove(0);
                for (r, ((w, f), g)) in want.iter().zip(&fold).zip(&got).enumerate() {
                    assert_eq!(bits(g), bits(w), "{what}: step {step}, row {r}: apart and fused");
                    assert_eq!(bits(g), bits(f), "{what}: step {step}, row {r}: apart and folded");
                }
                assert!(apart.memory() == fused.memory(), "{what}: the memory after step {step}, apart and fused");
                tokens = vec![argmax(want.last().unwrap())];
            }
        }
    }
}

/// A checkpoint whose tensor `tensor` has every row outside `keep` replaced by another of its rows.
struct Scrambled<S> {
    inner: S,
    tensor: String,
    keep: Vec<std::ops::Range<usize>>,
}

impl<S: WeightSource> WeightSource for Scrambled<S> {
    fn describe(&self, name: &str) -> std::io::Result<(GgmlType, Vec<u64>)> {
        self.inner.describe(name)
    }
    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>> {
        if name != self.tensor {
            return self.inner.row_bytes(name, first, count);
        }
        let (_, dims) = self.inner.describe(name)?;
        let rows = dims[1] as usize;
        let mut out = Vec::new();
        for r in first..first + count {
            let from = if self.keep.iter().any(|k| k.contains(&r)) { r } else { (r + rows / 2 + 1) % rows };
            out.extend(self.inner.row_bytes(name, from, 1)?);
        }
        Ok(out)
    }
    fn has(&self, name: &str) -> bool {
        self.inner.has(name)
    }
}

/// The slot the job reading tensor `tensor` writes, at each row of the first step: the prompt's rows.
fn product_of(machine: &mut Machine, tensor: &str, prompt: &[u32]) -> (Slot, Vec<Vec<f32>>) {
    let t = machine.layout.tensors.iter().position(|p| p.name == tensor).unwrap_or_else(|| panic!("{tensor} in the image"));
    let (j, to) = machine
        .jobs
        .iter()
        .enumerate()
        .find_map(|(j, job)| match job {
            Job::Matvec { tensor, to, .. } if *tensor == t => Some((j, *to)),
            Job::Vector(v) if v.loads.iter().any(|l| matches!(l.from, Source::Tensor { tensor, .. } if tensor == t)) => {
                v.puts.iter().find_map(|p| if let Sink::Slot { slot, .. } = p.to { Some((j, slot)) } else { None })
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("a job reads {tensor}"));
    let s = machine.sequence().unwrap();
    let mut seen = None;
    machine
        .step_traced(&[(s, prompt.to_vec())], &mut |tr| {
            if tr.job == j + 1 {
                seen = Some(tr.store[&to].clone());
            }
        })
        .unwrap();
    (to, seen.expect("the job ran"))
}

/// One checkpoint matrix's projections run as one matvec compute each projection as it computes alone: every row of
/// the product is the machine's dot of that row of the matrix with the activation, never of another — with every row
/// of the matrix outside one projection's replaced by other rows, that projection's elements are what they were, bit for
/// bit, and the rest are not. A Gated-DeltaNet block's query, key and value of `attn_qkv`, and an attention block's
/// queries and output gate interleaved by head in `attn_q`, at every real knob point, in Q8_0 and in f32.
fn the_engines_merged_projections_are_each_projection_alone() {
    let spec = engine_spec();
    let dir = tempfile::tempdir().unwrap();
    let q8 = dir.path().join("q8.gguf");
    tiny_models::write_checkpoint(&spec, &q8);
    let f32 = dir.path().join("f32.gguf");
    tiny_models::write_checkpoint_as(&spec, &f32, &|_| GgmlType::F32);
    let prompt = [3u32, 1, 4, 1, 5];
    let mut merges = Vec::new();
    for (l, layer) in spec.layers.iter().enumerate() {
        match &layer.mixer {
            Mixer::GatedDelta(g) => {
                let (q, v) = (g.k_heads * g.head_dim, g.v_heads * g.head_dim);
                let parts = vec![vec![0..q], vec![q..2 * q], vec![2 * q..2 * q + v]];
                merges.push((format!("blk.{l}.attn_qkv.weight"), parts));
            }
            Mixer::Attention(a) if a.output_gate => {
                let hd = a.head_dim;
                let half = |o: usize| (0..a.heads).map(|h| 2 * h * hd + o..2 * h * hd + o + hd).collect::<Vec<_>>();
                merges.push((format!("blk.{l}.attn_q.weight"), vec![half(0), half(hd)]));
            }
            _ => {}
        }
    }
    assert!(merges.iter().any(|(t, _)| t.contains("attn_qkv")) && merges.iter().any(|(t, _)| t.contains("attn_q.")), "the tiny hybrid has both merges");
    for numerics in EngineNumerics::real_points() {
        for path in [&q8, &f32] {
            for (tensor, parts) in &merges {
                let what = format!("{numerics:?}, {tensor} in {}", path.file_name().unwrap().to_string_lossy());
                let mut whole = Machine::new(spec.clone(), Gguf::open(path).unwrap(), numerics, 1, spec.ctx_len).unwrap();
                let (slot, want) = product_of(&mut whole, tensor, &prompt);
                for keep in parts {
                    let source = Scrambled { inner: Gguf::open(path).unwrap(), tensor: tensor.clone(), keep: keep.clone() };
                    let mut m = Machine::new(spec.clone(), source, numerics, 1, spec.ctx_len).unwrap();
                    let (s, got) = product_of(&mut m, tensor, &prompt);
                    assert_eq!(s, slot, "{what}");
                    let mut others_moved = false;
                    for (r, (w, g)) in want.iter().zip(&got).enumerate() {
                        for (i, (x, y)) in w.iter().zip(g).enumerate() {
                            if keep.iter().any(|k| k.contains(&i)) {
                                assert_eq!(x.to_bits(), y.to_bits(), "{what}: row {r}, element {i} of {keep:?} moved with rows outside it");
                            } else {
                                others_moved |= x.to_bits() != y.to_bits();
                            }
                        }
                    }
                    assert!(others_moved, "{what}: scrambling every row outside {keep:?} moved nothing, so the check is vacuous");
                }
            }
        }
    }
}

// ── The spatial chips: a joined program computes what its sections compute stored ─────────────────

use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Inputs, Value};
use logicaffeine_compile::spatial::ir::{NumericsClaim, SpatialProgram, TensorRole};
use logicaffeine_compile::spatial::pipeline::{compile_program_to_spatial, compile_program_to_spatial_returning};

/// `source` with every tensor it declares given deterministic values: a real tensor's from [`fusion_corpus::value`], an
/// integer tensor's small integers of both signs.
fn filled(source: &str) -> String {
    let mut out = String::new();
    for line in source.lines() {
        out.push_str(line);
        out.push('\n');
        let words: Vec<&str> = line.trim_end_matches('.').split_whitespace().collect();
        if let ["Let", name, "be", "a", rows, "by", cols, "tensor", "of", elem] = words[..] {
            let (Ok(rows), Ok(cols)) = (rows.parse::<usize>(), cols.parse::<usize>()) else { continue };
            for i in 0..rows * cols {
                let v = match elem {
                    "Real" => format!("{:?}", fusion_corpus::value(name, i)),
                    _ => format!("{}", ((i * 7 + name.len()) % 11) as i64 - 5),
                };
                out.push_str(&format!("Set item {} of {name} to {v}.\n", i + 1));
            }
        }
    }
    out
}

/// Every tensor `p` computes, by name, from its fills.
fn every_tensor(p: &SpatialProgram, what: &str) -> std::collections::BTreeMap<String, Data> {
    let inputs = Inputs::from_fills(p).unwrap_or_else(|e| panic!("{what}: {e}"));
    let out = eval_choreography_every_tensor(p, &inputs).unwrap_or_else(|e| panic!("{what}: the evaluator: {e:?}"));
    p.tensors
        .iter()
        .filter(|t| t.role != TensorRole::Input && t.role != TensorRole::Param)
        .filter_map(|t| {
            out.tensors.get(&t.id).map(|v| {
                let d = match v {
                    Value::Int(v) => Data::Int(v.clone()),
                    Value::Float(v) => Data::Real(v.clone()),
                };
                (t.name.clone(), d)
            })
        })
        .collect()
}

/// On every spatial chip, a program whose sections the passes join computes what it computes with every statement's
/// result stored — bit for bit where the program claims its numerics exact, within the bound the chip declares where it
/// claims them approximate: every class's exemplar and every hold-back witness filled, and the int8 decode model's
/// step, lowered with its results declared (joined) and undeclared (every result stored), every tensor both keep
/// evaluated. Every chip joins sections somewhere.
fn every_spatial_chips_joins_compute_their_sections() {
    use logicaffeine_compile::tensor_graph::symbols::{Exemplar, CLASSES};
    let mut corpus: Vec<(String, String)> = Vec::new();
    for c in CLASSES {
        if let Exemplar::Program { source, .. } = c.exemplar {
            corpus.push((format!("{} exemplar", c.id), filled(source)));
        }
    }
    for (rule, why, forbidding, twin) in fusion_corpus::HOLD_BACKS {
        corpus.push((format!("{rule}: {why}"), filled(forbidding)));
        corpus.push((format!("{rule}: twin of {why}"), filled(twin)));
    }
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/tiny_transformer.json");
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).expect("the int8 model")).expect("its JSON");
    let m = logicaffeine_compile::spatial::int8_model::Int8Model::from_json(&j).expect("the int8 model");
    corpus.push(("the int8 model's step".to_string(), m.decode_step_lg(0, 0, &[], &[])));
    for target in [SpatialTarget::wormhole(), SpatialTarget::sky_fabric(64, 64), SpatialTarget::sn50_rdu(), SpatialTarget::rtx3060()] {
        let contract = SpatialShim { target: &target }.numerics().expect("a spatial chip declares its contract");
        let (mut joined, mut lowered) = (0, 0);
        for (what, source) in &corpus {
            let what = format!("{} on {what}", target.name);
            let (Ok(stored), Ok(together)) = (compile_program_to_spatial(source, &target), compile_program_to_spatial_returning(source, &target, &BTreeSet::new())) else { continue };
            let (Some(stored), Some(together)) = (stored.whole(), together.whole()) else { continue };
            lowered += 1;
            if together.sections.len() < stored.sections.len() {
                joined += 1;
            }
            // An exact program's joins are exact; an approximate one's within the chip's declared bound.
            let bound = if stored.claim == NumericsClaim::Exact { Bound::EXACT } else { contract.fused };
            let (apart, fused) = (every_tensor(stored, &format!("{what}, stored")), every_tensor(together, &format!("{what}, joined")));
            for (name, v) in &fused {
                if let Some(a) = apart.get(name) {
                    within(&bound, a, v, &format!("{what}: `{name}`"));
                }
            }
        }
        assert!(joined > 0, "{}: no program of {lowered} lowered joins a section, so its claim is unchecked", target.name);
    }
}

// ── The chips whose executor is a GPU ──────────────────────────────────────────────────────────────

/// Where each GPU chip's fused-equals-apart claim is held on the device — by the start of its shim's name, the suite,
/// and the test, each comparing its fused program's results with its program apart bit for bit. This binary runs no
/// device; it holds every GPU chip to a holder that exists.
const ON_THE_GPU: &[(&str, &str, &str)] = &[
    ("cuda-lane:", "cuda_lane_lock", "every_fusion_computes_what_its_launches_compute_apart"),
    ("cuda-lane:", "cuda_lane_lock", "every_fusion_of_the_hybrid_step_computes_what_its_launches_compute_apart_at_width"),
    ("cuda-forward:", "cuda_fusion_census_lock", "every_forward_fusion_computes_what_its_kernels_compute_apart"),
];

fn every_gpu_chip_names_where_its_claim_is_held() {
    let tests = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests");
    for s in shims() {
        let name = s.name();
        if !name.starts_with("cuda-") {
            continue;
        }
        let holders: Vec<_> = ON_THE_GPU.iter().filter(|(prefix, ..)| name.starts_with(prefix)).collect();
        assert!(!holders.is_empty(), "`{name}` runs on the GPU, and no test there holds its fused program to its program apart");
        for (_, suite, test) in holders {
            let src = std::fs::read_to_string(tests.join(format!("{suite}.rs"))).unwrap_or_else(|e| panic!("{suite}: {e}"));
            let at = src.find(&format!("#[test]\nfn {test}()")).unwrap_or_else(|| panic!("`{name}`'s holder {suite}::{test} is not a test"));
            let body = &src[at..src[at + 1..].find("\n#[test]").map_or(src.len(), |e| at + 1 + e)];
            assert!(body.contains("to_bits()"), "{suite}::{test} compares its fused and apart results bit for bit");
            assert!(body.contains("require_gpu()"), "{suite}::{test} runs on the device");
        }
    }
}

/// Every chip computes each fusion it makes as its parts compute it apart, within the bound its contract declares
/// (`Contract::fused`): the meaning under every chip's shim; the D5005 engine's merges on its own machine, taken apart
/// job by job, and its merged projections row by row; the spatial chips' joined sections on the evaluator; and the GPU
/// chips' fused programs against their launches apart, held on the device by the suites named in [`ON_THE_GPU`].
#[test]
fn every_chip_fused_equals_apart_within_its_declared_bound() {
    every_shims_fusions_mean_their_parts();
    the_engine_computes_its_merges_as_their_parts();
    the_engines_merged_projections_are_each_projection_alone();
    every_spatial_chips_joins_compute_their_sections();
    every_gpu_chip_names_where_its_claim_is_held();
}

/// Every cache scheme the lane stores declares its bound (PARITY-KV-CACHE-QUANT): each `cache_precision` the lane's
/// contract names keeps every value within its range inside its bound, and its range is the format's — f32's largest,
/// binary16's 65504, fp8 e4m3's 448, where it saturates.
#[test]
fn every_cache_scheme_declares_its_bound() {
    for (cache_precision, range) in [(CachePrecision::F32, f64::from(f32::MAX)), (CachePrecision::F16, 65504.0), (CachePrecision::E4m3, 448.0)] {
        let knobs = CudaKnobs { cache_precision, ..CudaKnobs::default() };
        let c = LaneShim { knobs, batch: false, held: Vec::new() }.numerics().expect("the lane declares its contract");
        assert_eq!(c.cache.range(), range, "{cache_precision:?}: the format's range");
        hold_keep(c.cache);
    }
}
