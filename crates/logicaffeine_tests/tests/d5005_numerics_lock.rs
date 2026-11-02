//! The decode engine's arithmetic, held to independent oracles.
//!
//! `spatial::decode_engine::numerics` defines every number the engine produces. These tests hold each primitive to
//! a definition computed another way: the f16 codec over every binary16 value and a million f32 values against
//! exact f64 rounding; the wide-integer rounding against f64; every block format's coded form against ggml's own
//! dequantizer to the bit; the exact group dot and the per-block dot against element-by-element exact sums; the
//! per-block scales against their least-significand definition; the block-scale lane over the array's excess partial
//! sums against the per-block dot; the vector unit's micro-programs against the functions they are written from; the
//! rotary turn, the int8 cache row and its codes, the largest magnitude, the convolution and the arg-max against their
//! definitions, the arg-max folded in any lane order; the array's excess codes and digit passes against the plain
//! products; the transcendentals within their ulp bounds of f64.

use logicaffeine_compile::model::quant::{f16_to_f32, GgmlType};
use logicaffeine_compile::spatial::decode_engine::lane::{block_sums, deliver, lane};
use logicaffeine_compile::spatial::decode_engine::numerics::*;
use logicaffeine_compile::spatial::decode_engine::rtl::weights::{units, DECODERS};
use logicaffeine_compile::spatial::decode_engine::vector::{self, Regs};

/// splitmix64: a seeded stream every run reproduces.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
    /// A float of every sign, magnitude and fraction: random bits of a finite f32.
    fn any_f32(&mut self) -> f32 {
        loop {
            let v = f32::from_bits(self.next() as u32);
            if v.is_finite() {
                return v;
            }
        }
    }
    /// A float with the magnitudes activations have: `±m·2^e`, `e` in `-20..8`.
    fn activation(&mut self) -> f32 {
        let m = (self.next() >> 40) as f32 / (1u64 << 24) as f32;
        let e = (self.next() % 28) as i32 - 20;
        let s = if self.next() & 1 == 0 { 1.0 } else { -1.0 };
        s * m * 2f32.powi(e)
    }
}

/// The ulps between two f32s of the same sign.
fn ulps(a: f32, b: f32) -> u32 {
    let key = |x: f32| {
        let b = x.to_bits() as i32;
        if b < 0 {
            i32::MIN - b
        } else {
            b
        }
    };
    (key(a) as i64 - key(b) as i64).unsigned_abs() as u32
}

// ── f32 and f16 ────────────────────────────────────────────────────────────────────────────────────

#[test]
fn denormals_flush_to_zero_keeping_their_sign() {
    let tiny = f32::from_bits(1);
    assert_eq!(ftz(tiny).to_bits(), 0);
    assert_eq!(ftz(-tiny).to_bits(), 0x8000_0000);
    assert_eq!(ftz(f32::MIN_POSITIVE), f32::MIN_POSITIVE, "the least normal stays");
    // A product of normals that lands below the normals is flushed; an operand that is denormal is zero.
    assert_eq!(fmul(f32::MIN_POSITIVE, 0.5).to_bits(), 0);
    assert_eq!(fadd(tiny, 1.0), 1.0);
    assert_eq!(fmul(tiny, 1e30), 0.0, "a denormal operand reads as zero");
    let mut r = Rng(1);
    for _ in 0..100_000 {
        let (a, b) = (r.any_f32(), r.any_f32());
        let want = |v: f32| if v.is_subnormal() { 0f32.copysign(v) } else { v };
        assert_eq!(fmul(a, b).to_bits(), want(want(a) * want(b)).to_bits(), "{a:e} × {b:e}");
        assert_eq!(fadd(a, b).to_bits(), want(want(a) + want(b)).to_bits(), "{a:e} + {b:e}");
    }
}

#[test]
fn every_binary16_value_survives_the_round_trip() {
    for h in 0..=u16::MAX {
        let f = f16_to_f32(h);
        if f.is_nan() {
            assert!(f16_to_f32(f32_to_f16(f)).is_nan(), "{h:04x}");
            continue;
        }
        assert_eq!(f32_to_f16(f), h, "{h:04x} = {f:e}");
    }
}

/// Binary16 rounding by an independent route: the value in units of its binary16 ulp, rounded in f64 (exact for
/// every f32), clamped to infinity past the top.
fn f16_by_units(x: f32) -> u16 {
    let sign = if x.is_sign_negative() { 0x8000u16 } else { 0 };
    let a = x.abs() as f64;
    if a == 0.0 {
        return sign;
    }
    let e = a.log2().floor() as i32;
    let unit = 2f64.powi(e.max(-14) - 10);
    let n = (a / unit).round_ties_even();
    let v = n * unit;
    if v >= 65520.0 {
        return sign | 0x7c00;
    }
    // Encode `v`, which binary16 now holds exactly.
    let e2 = if v < 2f64.powi(-14) { -15 } else { v.log2().floor() as i32 };
    if e2 < -14 {
        return sign | (v / 2f64.powi(-24)) as u16;
    }
    let m = ((v / 2f64.powi(e2) - 1.0) * 1024.0).round() as u16;
    sign | (((e2 + 15) as u16) << 10) | m
}

#[test]
fn binary16_rounding_is_to_nearest_even_with_its_denormals() {
    let mut r = Rng(2);
    let mut checked = 0;
    for i in 0..1_000_000u32 {
        let x = if i % 2 == 0 { r.any_f32() } else { r.activation() * 2f32.powi((r.next() % 40) as i32 - 20) };
        assert_eq!(f32_to_f16(x), f16_by_units(x), "{x:e} ({:08x})", x.to_bits());
        checked += 1;
    }
    // Ties at the denormal and normal boundaries, and the overflow edge.
    for x in [2f32.powi(-25), 3.0 * 2f32.powi(-25), 2f32.powi(-14) - 2f32.powi(-25), 65504.0, 65519.99, 65520.0, 1.0 + 2f32.powi(-11), 1.0 + 3.0 * 2f32.powi(-11)] {
        assert_eq!(f32_to_f16(x), f16_by_units(x), "{x:e}");
        checked += 1;
    }
    assert!(checked > 1_000_000);
}

#[test]
fn a_wide_integer_rounds_to_f32_once() {
    let mut r = Rng(3);
    for _ in 0..1_000_000 {
        let bits = r.next() % 53;
        let v = (r.next() as i64 >> (63 - bits)) as i128;
        let exp = (r.next() % 120) as i32 - 60;
        let want = {
            let f = (v as f64 * 2f64.powi(exp)) as f32;
            if f.is_subnormal() {
                0f32.copysign(f)
            } else {
                f
            }
        };
        assert_eq!(scaled_to_f32(v, exp).to_bits(), want.to_bits(), "{v} · 2^{exp}");
    }
    // Past f64's 53 bits: ties to even at the 24th bit with the sticky bits below it.
    let tie_even = (1i128 << 80) + (1i128 << 56);
    assert_eq!(scaled_to_f32(tie_even, 0), 2f32.powi(80), "a tie to an even significand rounds down");
    let tie_odd = (1i128 << 80) + (3i128 << 56);
    assert_eq!(scaled_to_f32(tie_odd, 0), 2f32.powi(80) + 2f32.powi(58), "a tie to an odd significand rounds up");
    assert_eq!(scaled_to_f32(tie_even + 1, 0), 2f32.powi(80) + 2f32.powi(57), "a sticky bit past the tie rounds up");
    assert_eq!(scaled_to_f32(-tie_even, 0), -(2f32.powi(80)));
    assert_eq!(scaled_to_f32(1, 128), f32::INFINITY);
    assert_eq!(scaled_to_f32(-1, -127).to_bits(), 0x8000_0000, "below the normals is a zero of its sign");
}

#[test]
fn dots_run_in_sixteen_lanes_then_a_pairwise_tree() {
    let mut r = Rng(4);
    for n in [16usize, 128, 256, 37] {
        let a: Vec<f32> = (0..n).map(|_| r.activation()).collect();
        let b: Vec<f32> = (0..n).map(|_| r.activation()).collect();
        let mut lanes = vec![0f32; 16];
        for i in 0..n {
            lanes[i % 16] = fadd(lanes[i % 16], fmul(a[i], b[i]));
        }
        let level1: Vec<f32> = lanes.chunks(2).map(|p| fadd(p[0], p[1])).collect();
        let level2: Vec<f32> = level1.chunks(2).map(|p| fadd(p[0], p[1])).collect();
        let level3: Vec<f32> = level2.chunks(2).map(|p| fadd(p[0], p[1])).collect();
        assert_eq!(fdot(FpUnit::Soft, &a, &b).to_bits(), fadd(level3[0], level3[1]).to_bits(), "n = {n}");
    }
    // The order is a fact a hardware fold can get wrong: here a sequential sum differs from the lanes.
    let x: Vec<f32> = (0..32).map(|i| if i == 0 { 1e8 } else if i == 16 { -1e8 } else { 1.0 }).collect();
    let sequential = x.iter().fold(0f32, |a, &v| fadd(a, v));
    assert_ne!(fsum(&x).to_bits(), sequential.to_bits(), "{} vs {}", fsum(&x), sequential);
}

// ── Activations ─────────────────────────────────────────────────────────────────────────────────────

/// Is `code` the nearest integer to `x / (m·2^e)`, ties to even? Decided exactly: `code·m·2^e` and `x` are exact in
/// f64 and so is their difference.
fn nearest(x: f32, code: i64, m: i64, e: i32) -> bool {
    let scale = m as f64 * 2f64.powi(e);
    let twice_off = 2.0 * (x as f64 - code as f64 * scale).abs();
    twice_off < scale || (twice_off == scale && code % 2 == 0)
}

#[test]
fn a_block_scale_is_the_least_four_bit_significand_that_holds_its_block() {
    let mut r = Rng(5);
    for _ in 0..200 {
        let x: Vec<f32> = (0..512).map(|_| r.activation()).collect();
        for words in [Words::One, Words::Two] {
            let q = quantize(&x, words);
            assert_eq!(q.scales.len(), 512 / ACT_BLOCK);
            for (b, block) in x.chunks(ACT_BLOCK).enumerate() {
                let amax = block.iter().fold(0f32, |m, v| m.max(v.abs())) as f64;
                let sc = q.scales[b];
                let e = sc.e + if words == Words::Two { 8 } else { 0 };
                assert!((8..=15).contains(&sc.m), "a 4-bit significand");
                let holds = |m: i64, e: i32| 127.0 * m as f64 * 2f64.powi(e) >= amax;
                assert!(holds(sc.m, e), "block {b}: {}·2^{e} does not hold {amax}", sc.m);
                let smaller = if sc.m > 8 { (sc.m - 1, e) } else { (15, e - 1) };
                assert!(!holds(smaller.0, smaller.1), "block {b}: {}·2^{} also holds {amax}", smaller.0, smaller.1);
                for (i, &v) in block.iter().enumerate() {
                    let code = q.codes[b * ACT_BLOCK + i] as i64;
                    assert!(nearest(v, code, sc.m, sc.e), "{v} at {}·2^{}: {code}", sc.m, sc.e);
                    if words == Words::One {
                        assert!(code.abs() <= 127);
                    } else {
                        assert!(code.div_euclid(256).abs() <= 127, "two bytes");
                    }
                }
            }
        }
    }
    let zero = quantize(&[0.0; 256], Words::One);
    assert!(zero.codes.iter().all(|&c| c == 0));
}

/// Per weight block, every block's scale is the least 8-bit significand that holds it, and every code its value's
/// nearest integer under that scale, over the two block lengths the formats use.
#[test]
fn a_per_block_scale_is_the_least_eight_bit_significand_that_holds_its_block() {
    let mut r = Rng(15);
    for block in [32usize, 256] {
        for _ in 0..100 {
            let x: Vec<f32> = (0..1024).map(|_| r.activation()).collect();
            for words in [Words::One, Words::Two] {
                let q = quantize_blocks(&x, words, block);
                assert_eq!((q.scaling, q.block, q.scales.len()), (ActScaling::PerBlock, block, 1024 / block));
                for (b, chunk) in x.chunks(block).enumerate() {
                    let amax = chunk.iter().fold(0f32, |m, v| m.max(v.abs())) as f64;
                    let sc = q.scales[b];
                    let e = sc.e + if words == Words::Two { 8 } else { 0 };
                    assert!((128..=255).contains(&sc.m), "an 8-bit significand");
                    let holds = |m: i64, e: i32| 127.0 * m as f64 * 2f64.powi(e) >= amax;
                    assert!(holds(sc.m, e), "block {b}: {}·2^{e} does not hold {amax}", sc.m);
                    let smaller = if sc.m > 128 { (sc.m - 1, e) } else { (255, e - 1) };
                    assert!(!holds(smaller.0, smaller.1), "block {b}: {}·2^{} also holds {amax}", smaller.0, smaller.1);
                    for (i, &v) in chunk.iter().enumerate() {
                        let code = q.codes[b * block + i] as i64;
                        assert!(nearest(v, code, sc.m, sc.e), "{v} at {}·2^{}: {code}", sc.m, sc.e);
                        if words == Words::One {
                            assert!(code.abs() <= 127);
                        } else {
                            assert!(code.abs() <= 127 * 256, "a 16-bit code within its scale");
                        }
                    }
                }
            }
        }
    }
    let zero = quantize_blocks(&[0.0; 256], Words::One, 256);
    assert!(zero.codes.iter().all(|&c| c == 0));
    assert_eq!(zero.values(), vec![0.0; 256]);
}

#[test]
fn an_exact_division_rounds_to_the_nearest_with_ties_to_even() {
    let mut r = Rng(12);
    for _ in 0..500_000 {
        let x = r.activation();
        let m = 8 + (r.next() % 8) as i64;
        let e = (r.next() % 40) as i32 - 30;
        let code = rne_div(x, m, e);
        if code.abs() < (1 << 40) {
            assert!(nearest(x, code, m, e), "{x} / ({m}·2^{e}) = {code}");
        }
    }
    // Exact ties: 2.5 and 3.5 units to even, below and above.
    assert_eq!(rne_div(20.0, 8, 0), 2);
    assert_eq!(rne_div(28.0, 8, 0), 4);
    assert_eq!(rne_div(-20.0, 8, 0), -2);
    assert_eq!(rne_div(22.5, 9, 0), 2);
}

#[test]
fn an_exact_sum_keeps_its_window_and_floors_past_it() {
    assert_eq!(exact_sum(&[]), 0.0);
    assert_eq!(exact_sum(&[(3, 0), (5, 2)]), 23.0);
    assert_eq!(exact_sum(&[(1, 80), (-1, 80)]).to_bits(), 0, "an exact zero is +0");
    // A 25-bit odd significand at 2^64 is a tie between two f32s. A term 64 places below the top breaks the tie
    // upward; one 65 places below is past the window, floored away, and the tie goes to even.
    let tie = (1i128 << 24) + 1;
    assert_eq!(exact_sum(&[(tie, 64), (1, 0)]), 2f32.powi(88) + 2f32.powi(65), "inside the window");
    assert_eq!(exact_sum(&[(tie, 64), (1, -1)]), 2f32.powi(88), "past the window");
    assert_eq!(exact_sum(&[(tie, 64)]), 2f32.powi(88), "the bare tie goes to even");
}

/// A term any distance past the window floors at its least place: to −1 when negative, to 0 when not, however many
/// places below it lies — past an i128's width too, where a group's activation scales lie that many binades apart.
#[test]
fn an_exact_sum_floors_a_term_any_distance_past_its_window() {
    // The top terms cancel and fix the least place at 0; what survives is 3 plus the floored term.
    let top = [(1i128, 64), (-1i128, 64), (3, 0)];
    for (term, want) in [
        ((-(1i128 << 40), -100), 2.0),
        ((-(1i128 << 40), -127), 2.0),
        ((-(1i128 << 40), -128), 2.0),
        ((-(1i128 << 40), -140), 2.0),
        ((-1, -300), 2.0),
        ((1i128 << 100, -140), 3.0),
        ((1i128 << 100, -400), 3.0),
        ((i128::MIN / 4, -1000), 2.0),
        ((i128::MAX / 4, -1000), 3.0),
    ] {
        let mut terms = top.to_vec();
        terms.push(term);
        assert_eq!(exact_sum(&terms), want, "{term:?}");
    }
}

// ── Matrix rows ─────────────────────────────────────────────────────────────────────────────────────

const FORMATS: [GgmlType; 12] = [
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
];

/// Random blocks of `ty` with finite scales of the size real checkpoints carry.
fn blocks(r: &mut Rng, ty: GgmlType, n: usize) -> Vec<u8> {
    let mut b: Vec<u8> = (0..ty.block_bytes() * n).map(|_| r.byte()).collect();
    let scale_at: &[usize] = match ty {
        GgmlType::Q4_0 | GgmlType::Q5_0 | GgmlType::Q8_0 | GgmlType::Iq4Nl | GgmlType::Iq4Xs => &[0],
        GgmlType::Q4_1 | GgmlType::Q5_1 => &[0, 2],
        GgmlType::Q2K => &[80, 82],
        GgmlType::Q3K => &[108],
        GgmlType::Q4K | GgmlType::Q5K => &[0, 2],
        GgmlType::Q6K => &[208],
        _ => unreachable!(),
    };
    for blk in b.chunks_exact_mut(ty.block_bytes()) {
        for &at in scale_at {
            let s = f32_to_f16(((r.next() % 2000) as f32 + 1.0) * 1e-5 * if r.next() & 1 == 0 { 1.0 } else { -1.0 });
            blk[at..at + 2].copy_from_slice(&s.to_le_bytes());
        }
    }
    b
}

/// ggml writes each weight as a scale times an integer, and every such product is exact in f32: so its dequantized
/// float is the exact weight rounded once, and the engine's coded form, which never rounds, must round to it.
#[test]
fn every_coded_weight_rounds_to_ggmls_dequantized_weight() {
    let mut r = Rng(6);
    for ty in FORMATS {
        let raw = blocks(&mut r, ty, 64);
        let coded = CodedRow::decode(ty, &raw).expect("the array reads it").expect("finite scales");
        let mut ggml = Vec::new();
        ty.dequantize(&raw, &mut ggml);
        assert_eq!(coded.coef.len(), ggml.len(), "{}", ty.name());
        for (i, &g) in ggml.iter().enumerate() {
            // The same weight: a zero under a negative scale is ggml's -0, the engine's integer 0.
            assert!(coded.exact(i) as f32 == g, "{} element {i}: {} vs {g}", ty.name(), coded.exact(i));
        }
    }
}

#[test]
fn a_format_the_array_does_not_read_is_none() {
    for ty in [GgmlType::F32, GgmlType::F16, GgmlType::Bf16, GgmlType::Mxfp4] {
        assert!(CodedRow::decode(ty, &vec![0u8; ty.block_bytes()]).is_none(), "{}", ty.name());
    }
    let mut b = vec![0u8; GgmlType::Q8_0.block_bytes()];
    b[0..2].copy_from_slice(&0x7c00u16.to_le_bytes());
    assert_eq!(CodedRow::decode(GgmlType::Q8_0, &b), Some(Err(Unreadable { ty: GgmlType::Q8_0, block: 0 })), "an infinite scale");
}

/// The dot by an independent route: each element's exact weight times its exact activation, summed per group in
/// an i128 at one common fixed point, rounded once.
fn dot_by_elements(row: &CodedRow, act: &ActRow) -> f32 {
    let mut acc = 0f32;
    for g in 0..row.coef.len() / 256 {
        // Every scale exponent here is above -80, and every product fits 2^-80 · 2^120.
        let unit = -80;
        let mut sum: i128 = 0;
        for i in g * 256..(g + 1) * 256 {
            let s = &row.segments[i / row.segments[0].len];
            let a = act.scales[i / ACT_BLOCK];
            let code = act.codes[i] as i128;
            for (sc, c) in [(s.a, row.coef[i] as i128), (s.b, row.offs[i] as i128)] {
                sum += (sc.sig as i128 * a.m as i128 * c * code) << (sc.exp + a.e - unit);
            }
        }
        acc = fadd(acc, scaled_to_f32(sum, unit));
    }
    acc
}

#[test]
fn a_row_dot_is_exact_within_each_group_and_rounds_once_a_group() {
    let mut r = Rng(7);
    for ty in FORMATS {
        for words in [Words::One, Words::Two] {
            for _ in 0..40 {
                let n = 1024 / ty.block_len();
                let raw = blocks(&mut r, ty, n);
                let row = CodedRow::decode(ty, &raw).unwrap().unwrap();
                let x: Vec<f32> = (0..1024).map(|_| r.activation()).collect();
                let act = quantize(&x, words);
                assert_eq!(dot(&row, &act).to_bits(), dot_by_elements(&row, &act).to_bits(), "{} {words:?}", ty.name());
            }
        }
    }
}

/// The per-block dot by an independent route: each element's exact weight times its exact activation, summed per
/// weight block in an i128 at one common fixed point, rounded once.
fn dot_by_blocks(row: &CodedRow, act: &ActRow) -> f32 {
    let mut acc = 0f32;
    for (b, s) in row.segments.iter().enumerate() {
        let unit = -80;
        let a = act.scales[b];
        let mut sum: i128 = 0;
        for i in s.start..s.start + s.len {
            let code = act.codes[i] as i128;
            for (sc, c) in [(s.a, row.coef[i] as i128), (s.b, row.offs[i] as i128)] {
                sum += (sc.sig as i128 * a.m as i128 * c * code) << (sc.exp + a.e - unit);
            }
        }
        acc = fadd(acc, scaled_to_f32(sum, unit));
    }
    acc
}

#[test]
fn a_per_block_dot_is_exact_within_each_weight_block_and_rounds_once_a_block() {
    let mut r = Rng(17);
    for ty in FORMATS {
        for words in [Words::One, Words::Two] {
            for _ in 0..40 {
                let n = 1024 / ty.block_len();
                let raw = blocks(&mut r, ty, n);
                let row = CodedRow::decode(ty, &raw).unwrap().unwrap();
                let x: Vec<f32> = (0..1024).map(|_| r.activation()).collect();
                let act = quantize_blocks(&x, words, ty.block_len());
                assert_eq!(dot(&row, &act).to_bits(), dot_by_blocks(&row, &act).to_bits(), "{} {words:?}", ty.name());
            }
        }
    }
}

/// A row read against activations scaled per blocks of another length is refused, not mismatched.
#[test]
#[should_panic(expected = "an activation scaled per 32 elements against a row of 256-element blocks")]
fn a_per_block_dot_refuses_an_activation_of_another_blocking() {
    let mut r = Rng(18);
    let row = CodedRow::decode(GgmlType::Q4K, &blocks(&mut r, GgmlType::Q4K, 1)).unwrap().unwrap();
    let x: Vec<f32> = (0..256).map(|_| r.activation()).collect();
    dot(&row, &quantize_blocks(&x, Words::One, 32));
}

/// The decoders' software twin gives the coded row: every weight `c_u·(code − z)` and offset term `n_u`, and the
/// block's scales.
#[test]
fn the_decoders_twin_gives_the_coded_row() {
    let mut r = Rng(20);
    for d in DECODERS {
        let raw = blocks(&mut r, d.ty, 64);
        let row = CodedRow::decode(d.ty, &raw).unwrap().unwrap();
        for (b, blk) in raw.chunks_exact(d.ty.block_bytes()).enumerate() {
            let u = units(d.ty, blk);
            let seg = &row.segments[b];
            assert_eq!(Scale::f16(u.a), Some(seg.a), "{} block {b}: the first scale", d.ty.name());
            assert!(Scale::f16(u.b).map_or(false, |s| s == seg.b || (s.sig == 0 && seg.b.sig == 0)), "{} block {b}: the second scale", d.ty.name());
            for i in 0..d.ty.block_len() {
                let at = seg.start + i;
                assert!(u.code[i] < 1 << d.code_bits, "{} element {i}: a {}-bit code", d.ty.name(), d.code_bits);
                assert_eq!(u.c[i / 16] as i32 * (u.code[i] as i32 - d.offset as i32), row.coef[at] as i32, "{} element {i}", d.ty.name());
                assert_eq!(u.n[i / 16], row.offs[at], "{} element {i}: the offset term", d.ty.name());
            }
        }
    }
}

/// The lane's arithmetic over what the array actually delivers — excess codes, recombined digits, the weight image's
/// code sums and the activation side's code sums — gives the per-block dot bit for bit, for every format and both
/// word counts.
#[test]
fn the_lane_over_the_arrays_excess_sums_is_the_per_block_dot() {
    let mut r = Rng(21);
    for d in DECODERS {
        for words in [Words::One, Words::Two] {
            for _ in 0..30 {
                let n = 1024 / d.ty.block_len();
                let raw = blocks(&mut r, d.ty, n);
                let row = CodedRow::decode(d.ty, &raw).unwrap().unwrap();
                let x: Vec<f32> = (0..1024).map(|_| r.activation()).collect();
                let act = quantize_blocks(&x, words, d.ty.block_len());
                let mut acc = 0f32;
                for (b, blk) in raw.chunks_exact(d.ty.block_bytes()).enumerate() {
                    let seg = &row.segments[b];
                    let delivered = deliver(&units(d.ty, blk), (seg.a, seg.b), d.offset, &act, words, b);
                    // The signed sums the lane recovers are the coded row's own.
                    let range = seg.start..seg.start + seg.len;
                    let ia: i128 = range.clone().map(|i| row.coef[i] as i128 * act.codes[i] as i128).sum();
                    let ib: i128 = range.map(|i| row.offs[i] as i128 * act.codes[i] as i128).sum();
                    assert_eq!(block_sums(&delivered), (ia, ib), "{} {words:?} block {b}", d.ty.name());
                    acc = lane(&delivered, acc);
                }
                assert_eq!(acc.to_bits(), dot(&row, &act).to_bits(), "{} {words:?}", d.ty.name());
            }
        }
    }
}

/// The array reads excess codes `a + 128` and 4-bit digits of wider codes; the sums it recombines are the plain
/// products, exactly.
#[test]
fn the_arrays_excess_codes_and_digits_recombine_to_the_plain_sum() {
    let mut r = Rng(8);
    for _ in 0..20_000 {
        let n = 32;
        let q: Vec<i64> = (0..n).map(|_| (r.next() % 256) as i64).collect();
        let a: Vec<i64> = (0..n).map(|_| (r.next() % 255) as i64 - 127).collect();
        let plain: i64 = q.iter().zip(&a).map(|(q, a)| q * a).sum();
        let excess: i64 = q.iter().zip(&a).map(|(q, a)| q * (a + 128)).sum::<i64>() - 128 * q.iter().sum::<i64>();
        let digits: i64 = q.iter().zip(&a).map(|(q, a)| (q & 15) * (a + 128)).sum::<i64>()
            + 16 * q.iter().zip(&a).map(|(q, a)| (q >> 4) * (a + 128)).sum::<i64>()
            - 128 * q.iter().sum::<i64>();
        assert_eq!((plain, excess), (digits, digits));
    }
}

// ── Transcendentals ─────────────────────────────────────────────────────────────────────────────────

fn sweep(lo: f32, hi: f32, n: usize) -> impl Iterator<Item = f32> {
    (0..n).map(move |i| lo + (hi - lo) * i as f32 / (n - 1) as f32)
}

#[test]
fn exp_is_within_two_ulps_where_its_result_is_normal() {
    let mut worst = 0;
    for x in sweep(-87.0, 88.5, 2_000_003) {
        let want = (x as f64).exp() as f32;
        worst = worst.max(ulps(exp(FpUnit::Soft, x), want));
    }
    assert!(worst <= 2, "exp: {worst} ulps");
    assert_eq!(exp(FpUnit::Soft, 0.0), 1.0);
    assert_eq!(exp(FpUnit::Soft, -200.0), 0.0);
    assert_eq!(exp(FpUnit::Soft, 200.0), f32::INFINITY);
}

#[test]
fn the_reciprocal_and_its_square_root_are_within_two_ulps() {
    let mut r = Rng(9);
    let (mut worst_r, mut worst_s) = (0, 0);
    for _ in 0..1_000_000 {
        // Normal positives below 2^125, whose reciprocals are normal too.
        let x = f32::from_bits((r.next() as u32) % (0x7e00_0000 - 0x0080_0000) + 0x0080_0000);
        worst_r = worst_r.max(ulps(recip(FpUnit::Soft, x), (1.0 / x as f64) as f32));
        if x.is_normal() {
            worst_s = worst_s.max(ulps(rsqrt(FpUnit::Soft, x), (1.0 / (x as f64).sqrt()) as f32));
        }
    }
    // Two Newton steps without a fused multiply-add leave the reciprocal within two ulps.
    assert!(worst_r <= 2, "recip: {worst_r} ulps");
    assert!(worst_s <= 2, "rsqrt: {worst_s} ulps");
    assert_eq!(recip(FpUnit::Soft, -4.0), -0.25);
    assert_eq!(rsqrt(FpUnit::Soft, 4.0), 0.5);
    assert_eq!(recip(FpUnit::Soft, 0.0), f32::INFINITY);
}

#[test]
fn ln_softplus_sigmoid_and_silu_are_within_their_bounds() {
    let mut worst = (0, 0, 0);
    for x in sweep(1e-30, 1e30, 1_000_003).chain(sweep(0.01, 100.0, 1_000_003)) {
        worst.0 = worst.0.max(ulps(ln(FpUnit::Soft, x), (x as f64).ln() as f32));
    }
    for u in sweep(-30.0, 30.0, 600_001) {
        let sp = (u as f64).exp().ln_1p() as f32;
        worst.1 = worst.1.max(ulps(softplus(FpUnit::Soft, u), sp));
        let sg = (1.0 / (1.0 + (-(u as f64)).exp())) as f32;
        worst.2 = worst.2.max(ulps(sigmoid(FpUnit::Soft, u), sg));
    }
    assert!(worst.0 <= 2, "ln: {} ulps", worst.0);
    assert!(worst.1 <= 4, "softplus: {} ulps", worst.1);
    assert!(worst.2 <= 4, "sigmoid: {} ulps", worst.2);
    assert_eq!(silu(FpUnit::Soft, 0.0), 0.0);
    // Near 1 a logarithm keeps its relative precision; ln1p keeps it for small arguments.
    for x in [0.9999f32, 1.0001, 0.999_999, 1.000_001] {
        assert!(ulps(ln(FpUnit::Soft, x), (x as f64).ln() as f32) <= 2, "ln(FpUnit::Soft, {x})");
    }
    for t in [1e-8f32, 1e-5, 0.01, 0.3, 1.0, 100.0] {
        assert!(ulps(ln1p(FpUnit::Soft, t), (t as f64).ln_1p() as f32) <= 2, "ln1p(FpUnit::Soft, {t})");
    }
}

#[test]
fn norms_are_their_formulas_within_a_few_ulps() {
    let mut r = Rng(10);
    for _ in 0..2000 {
        let x: Vec<f32> = (0..256).map(|_| r.activation()).collect();
        let w: Vec<f32> = (0..256).map(|_| 0.5 + (r.next() % 1000) as f32 / 1000.0).collect();
        let mean = x.iter().map(|&v| v as f64 * v as f64).sum::<f64>() / 256.0;
        let scale = 1.0 / (mean + 1e-6).sqrt();
        for (i, y) in rmsnorm(FpUnit::Soft, &x, &w, 1e-6).iter().enumerate() {
            let want = (x[i] as f64 * scale * w[i] as f64) as f32;
            assert!(ulps(*y, want) <= 6, "rmsnorm element {i}: {y} vs {want}");
        }
    }
}

/// The rotation is the Walsh–Hadamard matrix `H_ij = (−1)^popcount(i∧j)`: exact on whole numbers, where every
/// partial sum is exact in f32; applied twice it is the length times the identity; on any floats it stays within
/// a rounding a stage of the exact transform.
#[test]
fn the_hadamard_rotation_is_the_walsh_matrix_and_its_own_inverse() {
    let mut r = Rng(19);
    for n in [1usize, 2, 4, 8, 64, 128, 256] {
        for _ in 0..50 {
            let x: Vec<f32> = (0..n).map(|_| ((r.next() % 2001) as i64 - 1000) as f32).collect();
            let mut y = x.clone();
            hadamard(&mut y);
            for (i, &yi) in y.iter().enumerate() {
                let want: i64 = x.iter().enumerate().map(|(j, &v)| if (i & j).count_ones() % 2 == 0 { v as i64 } else { -(v as i64) }).sum();
                assert_eq!(yi as i64, want, "n={n} element {i}");
            }
            hadamard(&mut y);
            for (a, b) in y.iter().zip(&x) {
                assert_eq!(*a, b * n as f32, "n={n}: twice is {n} times the identity");
            }
            let f: Vec<f32> = (0..n).map(|_| r.activation()).collect();
            let mut g = f.clone();
            hadamard(&mut g);
            let norm = f.iter().map(|&v| (v as f64).powi(2)).sum::<f64>().sqrt();
            for (i, &gi) in g.iter().enumerate() {
                let exact: f64 = f.iter().enumerate().map(|(j, &v)| if (i & j).count_ones() % 2 == 0 { v as f64 } else { -(v as f64) }).sum();
                let stages = n.trailing_zeros() as f64;
                assert!((gi as f64 - exact).abs() <= stages * 2f64.powi(-23) * norm * (n as f64).sqrt(), "n={n} element {i}");
            }
        }
    }
}

#[test]
#[should_panic(expected = "a Hadamard transform of 96 elements")]
fn a_hadamard_rotation_of_a_length_not_a_power_of_two_is_refused() {
    hadamard(&mut [0.0; 96]);
}

#[test]
fn a_cache_row_is_int8_under_one_f16_scale() {
    let mut r = Rng(11);
    for _ in 0..5000 {
        // Rows of the magnitudes keys and values have: one row-wide power of two, so the scale is a normal f16.
        let k = (r.next() % 14) as i32 - 6;
        let x: Vec<f32> = (0..256).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 2f32.powi(k)).collect();
        let (s, codes) = cache_row(FpUnit::Soft, &x);
        let sf = f16_to_f32(s) as f64;
        let amax = x.iter().fold(0f32, |m, v| m.max(v.abs())) as f64;
        assert!((sf - amax / 127.0).abs() <= amax / 127.0 * 2f64.powi(-10), "the scale is amax/127 in f16");
        for (&v, &c) in x.iter().zip(&codes) {
            assert!((v as f64 - c as f64 * sf).abs() <= 0.5 * sf * 1.01 + 1e-30 || c.unsigned_abs() == 127, "{v} vs {c}·{sf}");
        }
    }
}

// ── The vector unit ────────────────────────────────────────────────────────────────────────────────

fn vector_of(r: &mut Rng, n: usize) -> Vec<f32> {
    (0..n).map(|_| r.activation()).collect()
}

/// Every micro-program computes, bit for bit, the numerics function it is written from, over seeded vectors of the
/// lengths the engine runs them at.
#[test]
fn the_vector_units_micro_programs_are_their_numerics() {
    let mut r = Rng(22);
    for n in [16usize, 128, 256, 1024, 5120] {
        for _ in 0..8 {
            let (x, w) = (vector_of(&mut r, n), vector_of(&mut r, n));
            let mut regs = Regs::new(8, 8);
            regs.v[0] = x.clone();
            regs.v[1] = w.clone();
            vector::run(FpUnit::Soft, &vector::rmsnorm(FpUnit::Soft, 0, 1, 2, 3, n, 1e-6), &mut regs);
            assert_eq!(bits(&regs.v[2]), bits(&rmsnorm(FpUnit::Soft, &x, &w, 1e-6)), "rmsnorm over {n}");
            vector::run(FpUnit::Soft, &vector::l2norm(0, 4, 1e-6), &mut regs);
            assert_eq!(bits(&regs.v[4]), bits(&l2norm(FpUnit::Soft, &x, 1e-6)), "l2norm over {n}");
            vector::run(FpUnit::Soft, &vector::swiglu(0, 1, 5), &mut regs);
            let want: Vec<f32> = x.iter().zip(&w).map(|(&g, &u)| fmul(silu(FpUnit::Soft, g), u)).collect();
            assert_eq!(bits(&regs.v[5]), bits(&want), "swiglu over {n}");
            vector::run(FpUnit::Soft, &vector::sigmoid_gate(0, 1, 6, 7), &mut regs);
            let want: Vec<f32> = x.iter().zip(&w).map(|(&o, &g)| fmul(o, sigmoid(FpUnit::Soft, g))).collect();
            assert_eq!(bits(&regs.v[6]), bits(&want), "the sigmoid gate over {n}");
        }
    }
    for n in [1usize, 7, 33, 512, 4096] {
        let scores: Vec<f32> = (0..n).map(|_| r.activation() * 40.0).collect();
        let mut regs = Regs::new(2, 2);
        regs.v[0] = scores.clone();
        vector::run(FpUnit::Soft, &vector::softmax(0, 1), &mut regs);
        let m = fmax(&scores);
        let e: Vec<f32> = scores.iter().map(|&s| exp(FpUnit::Soft, fsub(s, m))).collect();
        let inv = recip(FpUnit::Soft, fsum(&e));
        let want: Vec<f32> = e.iter().map(|&v| fmul(v, inv)).collect();
        assert_eq!(bits(&regs.v[1]), bits(&want), "softmax over {n}");
    }
    for _ in 0..2000 {
        let (alpha, dt, a, beta) = (r.activation() * 30.0, r.activation(), -r.activation().abs() * 8.0, r.activation() * 20.0);
        let mut regs = Regs::new(0, 8);
        regs.s[0..4].copy_from_slice(&[alpha, dt, a, beta]);
        vector::run(FpUnit::Soft, &vector::gdn_gates(0, 1, 2, 3, 4, 5, 6), &mut regs);
        let g = fmul(softplus(FpUnit::Soft, fadd(alpha, dt)), a);
        assert_eq!((regs.s[4].to_bits(), regs.s[5].to_bits(), regs.s[6].to_bits()), (g.to_bits(), exp(FpUnit::Soft, g).to_bits(), sigmoid(FpUnit::Soft, beta).to_bits()));
    }
}

/// A multiply-add is the multiply then the add, bit for bit, at either multiplier: a vector addend and a scalar one,
/// over every pairing of a corpus of special values — signed zeros, the least normal and the values about it, denormals,
/// the largest finite, infinities, NaNs of both signs — and random ones, products past the top and below the normals
/// among them.
#[test]
fn a_multiply_add_is_the_multiply_then_the_add_bit_for_bit() {
    use vector::{Bin, Op};
    let special: Vec<f32> = [
        0.0f32,
        -0.0,
        f32::MIN_POSITIVE,
        -f32::MIN_POSITIVE,
        f32::from_bits(0x0080_0001),
        f32::from_bits(0x007f_ffff),
        f32::from_bits(1),
        f32::MAX,
        -f32::MAX,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
        f32::from_bits(0xffc0_0001),
        1.0,
        -1.0,
        0.5,
        1.0000001,
        3.0e-20,
        -7.5e19,
        2.0f32.powi(-63),
    ]
    .to_vec();
    let mut state = 0x6d75_6c61_6464u64;
    let mut next = || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let r = state >> 33;
        let m = ((r & 0x7f_ffff) as f32 / (1u32 << 23) as f32 + 1.0) * if r & (1 << 23) != 0 { -1.0 } else { 1.0 };
        m * 2.0f32.powi(((r >> 24) % 160) as i32 - 80)
    };
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut c = Vec::new();
    for &x in &special {
        for &y in &special {
            for &z in &special {
                a.push(x);
                b.push(y);
                c.push(z);
            }
        }
    }
    for _ in 0..20_000 {
        a.push(next());
        b.push(next());
        c.push(next());
    }
    for fp in [FpUnit::Soft, FpUnit::Hard] {
        let mut fused = Regs::new(4, 1);
        fused.v[0] = a.clone();
        fused.v[1] = b.clone();
        fused.v[2] = c.clone();
        let mut apart = fused.clone();
        vector::run(fp, &[Op::MulAdd { a: 0, b: 1, c: 2, d: 3 }], &mut fused);
        vector::run(fp, &[Op::Vec { op: Bin::Mul, a: 0, b: 1, d: 3 }, Op::Vec { op: Bin::Add, a: 2, b: 3, d: 3 }], &mut apart);
        assert_eq!(bits(&fused.v[3]), bits(&apart.v[3]), "{fp:?}: a vector addend");
        for &k in &special {
            let (mut fused, mut apart) = (fused.clone(), apart.clone());
            fused.s[0] = k;
            apart.s[0] = k;
            vector::run(fp, &[Op::MulAddScalar { a: 0, b: 1, s: 0, d: 3 }], &mut fused);
            vector::run(fp, &[Op::Vec { op: Bin::Mul, a: 0, b: 1, d: 3 }, Op::VecScalar { op: Bin::Add, a: 3, s: 0, d: 3 }], &mut apart);
            assert_eq!(bits(&fused.v[3]), bits(&apart.v[3]), "{fp:?}: the scalar addend {k:e}");
        }
    }
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

/// The maximum is the largest value whatever order it is folded in: a NaN is passed over, and a largest zero is `+0`.
#[test]
fn the_maximum_is_the_largest_value_in_any_order() {
    let mut r = Rng(23);
    for _ in 0..2000 {
        let mut x: Vec<f32> = (0..(1 + r.next() % 40) as usize).map(|_| r.activation()).collect();
        if r.next() % 3 == 0 {
            x.push(0.0);
            x.push(-0.0);
        }
        if r.next() % 5 == 0 {
            x.push(f32::NAN);
        }
        let m = fmax(&x);
        let want = x.iter().filter(|v| !v.is_nan()).fold(f32::NEG_INFINITY, |a, &b| if b > a { b } else { a });
        let want = if want == 0.0 { 0.0 } else { want };
        assert_eq!(m.to_bits(), want.to_bits());
        let mut y = x.clone();
        y.reverse();
        assert_eq!(fmax(&y).to_bits(), m.to_bits(), "the reverse order");
    }
    assert_eq!(fmax(&[-0.0, -0.0]).to_bits(), 0, "a largest zero is +0");
    assert_eq!(fmax(&[]), f32::NEG_INFINITY);
}

// ── The rotary turn, the int8 cache row, the convolution, the greedy token ─────────────────────────

/// The rotary turn is its pairs: NEOX pairs `(i, i + half)` and interleaved pairs `(2i, 2i + 1)`, each
/// `(x₀·c − x₁·s, x₀·s + x₁·c)`, the elements past the table's pairs passing through bit for bit; the turn keeps each
/// pair's length to f32 rounding; the rotary program is the turn, in place or into another register.
#[test]
fn the_rotary_turn_is_its_pairs_and_its_program() {
    let mut r = Rng(31);
    for &(n, half) in &[(2usize, 1usize), (16, 8), (40, 12), (64, 16), (96, 40), (128, 32), (256, 32), (256, 128), (16, 0)] {
        for neox in [true, false] {
            for _ in 0..40 {
                let x = vector_of(&mut r, n);
                let (cos, sin): (Vec<f32>, Vec<f32>) = (0..half)
                    .map(|_| {
                        let a = (r.next() % 2_000_000) as f64 * 1e-3;
                        (a.cos() as f32, a.sin() as f32)
                    })
                    .unzip();
                let mut want = x.clone();
                for i in 0..half {
                    let (a, b) = if neox { (i, i + half) } else { (2 * i, 2 * i + 1) };
                    want[a] = fsub(fmul(x[a], cos[i]), fmul(x[b], sin[i]));
                    want[b] = fadd(fmul(x[a], sin[i]), fmul(x[b], cos[i]));
                    let before = (x[a] as f64).hypot(x[b] as f64);
                    let after = (want[a] as f64).hypot(want[b] as f64);
                    assert!((after - before).abs() <= before * 1e-5 + 1e-30, "pair {i} keeps its length: {before} → {after}");
                }
                assert_eq!(bits(&want[2 * half..]), bits(&x[2 * half..]), "the elements past the pairs pass through");
                let mut got = x.clone();
                rope(FpUnit::Soft, &mut got, &cos, &sin, neox);
                assert_eq!(bits(&got), bits(&want), "{n} elements, {half} pairs, neox {neox}");
                let mut regs = Regs::new(4, 1);
                regs.v[0] = x.clone();
                regs.v[1] = cos.clone();
                regs.v[2] = sin.clone();
                vector::run(FpUnit::Soft, &vector::rotary(0, 1, 2, 3, neox), &mut regs);
                assert_eq!(bits(&regs.v[3]), bits(&want), "the rotary program");
                vector::run(FpUnit::Soft, &vector::rotary(0, 1, 2, 0, neox), &mut regs);
                assert_eq!(bits(&regs.v[0]), bits(&want), "the rotary program in place");
            }
        }
    }
}

/// A cache row is its definition, the zero scale included: the largest magnitude with denormals read as zero, its
/// `1/127` rounded once to the f16 scale, every element times the scale's reciprocal to the nearest integer, ties to
/// even, within ±127 — every code zero when the scale rounds to zero. The cache-row program leaves the scale, as the
/// f32 the f16 holds, and the codes.
#[test]
fn a_cache_row_is_its_definition_and_its_program() {
    let mut r = Rng(32);
    let mut rows: Vec<Vec<f32>> = Vec::new();
    // Magnitudes whose scale underflows binary16, lands among its denormals, is a normal, and overflows it.
    for k in [-30i32, -22, -18, -16, -12, -8, -6, 0, 6, 14, 122] {
        for len in [4usize, 37, 256] {
            for _ in 0..12 {
                rows.push((0..len).map(|_| ((r.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0) * 2f32.powi(k)).collect());
            }
        }
    }
    rows.push(vec![0.0; 64]);
    rows.push(vec![-0.0; 64]);
    rows.push((0..64).map(|i| f32::from_bits(1 + i)).collect());
    rows.push(vec![2.5e-6, -1e-6, 0.0, 3.7e-6]);
    rows.push((0..32).map(|i| (i as f32 - 15.5) * 0.25).collect());
    for x in &rows {
        let amax = x.iter().fold(0f32, |m, &v| m.max(ftz(v).abs()));
        let s = f32_to_f16(fmul(amax, INV_127));
        let sf = f16_to_f32(s);
        let want: Vec<i8> = if sf == 0.0 {
            vec![0; x.len()]
        } else {
            let inv = recip(FpUnit::Soft, sf);
            x.iter().map(|&v| (fmul(v, inv) as f64).round_ties_even().clamp(-127.0, 127.0) as i8).collect()
        };
        assert_eq!(cache_row(FpUnit::Soft, x), (s, want.clone()), "the row {:?}…", &x[..x.len().min(4)]);
        let mut regs = Regs::new(2, 2);
        regs.v[0] = x.clone();
        vector::run(FpUnit::Soft, &vector::cache_row(0, 1, 0), &mut regs);
        assert_eq!(regs.s[0].to_bits(), sf.to_bits(), "the program's scale");
        assert_eq!(bits(&regs.v[1]), bits(&want.iter().map(|&c| c as f32).collect::<Vec<_>>()), "the program's codes");
    }
}

/// The int8 codes round each product to the nearest integer, ties to even, saturate at ±127, read a NaN product as
/// zero, and are all zero under an infinite inverse scale.
#[test]
fn the_int8_codes_round_to_even_and_saturate() {
    assert_eq!(
        int8_codes(FpUnit::Soft, &[0.5, 1.5, 2.5, -0.5, -1.5, -2.5, 126.5, 127.4, 127.5, 128.0, -127.6, -1e30, 3e38, f32::NAN, -0.0], 1.0),
        vec![0, 2, 2, 0, -2, -2, 126, 127, 127, 127, -127, -127, 127, 0, 0]
    );
    assert_eq!(int8_codes(FpUnit::Soft, &[1.0, -5.0, 0.0, f32::NAN], f32::INFINITY), vec![0; 4]);
    assert_eq!(int8_codes(FpUnit::Soft, &[1.0, -5.0], f32::NEG_INFINITY), vec![0; 2]);
    assert_eq!(int8_codes(FpUnit::Soft, &[1.0, -5.0], f32::NAN), vec![0; 2]);
    assert_eq!(int8_codes(FpUnit::Soft, &[f32::from_bits(1), -1.0], 2f32.powi(100)), vec![0, -127], "a denormal reads as zero");
    let mut r = Rng(35);
    for _ in 0..100_000 {
        let (x, inv) = (r.activation() * 64.0, r.activation().abs() * 2f32.powi((r.next() % 40) as i32));
        let want = (fmul(x, inv) as f64).round_ties_even().clamp(-127.0, 127.0) as i8;
        assert_eq!(int8_codes(FpUnit::Soft, &[x], inv), vec![want], "{x}·{inv}");
    }
}

/// The largest magnitude reads a denormal as zero and passes a NaN over, folded in any order; with nothing larger it
/// is `+0`.
#[test]
fn the_largest_magnitude_reads_denormals_as_zero() {
    let mut r = Rng(36);
    for _ in 0..3000 {
        let mut x: Vec<f32> = (0..(r.next() % 50) as usize)
            .map(|_| match r.next() % 6 {
                0 => f32::NAN,
                1 => f32::from_bits(r.next() as u32 & 0x807f_ffff),
                2 => -0.0,
                _ => r.activation(),
            })
            .collect();
        let want = x.iter().filter(|v| !v.is_nan()).map(|&v| if v.is_subnormal() { 0.0 } else { v.abs() }).fold(0f32, f32::max);
        assert_eq!(absmax(&x).to_bits(), want.to_bits());
        x.reverse();
        assert_eq!(absmax(&x).to_bits(), want.to_bits(), "the reverse order");
    }
    assert_eq!(absmax(&[]).to_bits(), 0);
    assert_eq!(absmax(&[-0.0, f32::from_bits(0x8000_0001)]).to_bits(), 0, "a zero magnitude is +0");
    assert_eq!(absmax(&[-3.0, 2.0, f32::NAN]), 3.0);
}

/// The causal convolution is each channel's products in tap order from `+0` — the ring's rows, oldest first, then the
/// token's own row against the last tap — through SiLU; the conv program, the kernel held tap by tap, is the
/// convolution.
#[test]
fn the_convolution_is_its_taps_and_its_program() {
    let mut r = Rng(33);
    for taps in 1..=4usize {
        for &ch in &[1usize, 16, 37, 160] {
            for _ in 0..10 {
                let ring: Vec<Vec<f32>> = (0..taps - 1).map(|_| vector_of(&mut r, ch)).collect();
                let x = vector_of(&mut r, ch);
                let kernel = vector_of(&mut r, ch * taps);
                let want: Vec<f32> = (0..ch)
                    .map(|c| {
                        let mut acc = 0f32;
                        for j in 0..taps {
                            let row = if j + 1 < taps { &ring[j] } else { &x };
                            acc = fadd(acc, fmul(row[c], kernel[c * taps + j]));
                        }
                        silu(FpUnit::Soft, acc)
                    })
                    .collect();
                assert_eq!(bits(&conv(FpUnit::Soft, &ring, &x, &kernel, taps)), bits(&want), "{taps} taps over {ch} channels");
                let mut regs = Regs::new(16, 1);
                for (j, row) in ring.iter().enumerate() {
                    regs.v[j] = row.clone();
                }
                regs.v[3] = x.clone();
                for j in 0..taps {
                    regs.v[4 + j] = (0..ch).map(|c| kernel[c * taps + j]).collect();
                }
                let ring_regs: Vec<u8> = (0..taps as u8 - 1).collect();
                let tap_regs: Vec<u8> = (4..4 + taps as u8).collect();
                vector::run(FpUnit::Soft, &vector::conv(&ring_regs, 3, &tap_regs, 8, 9), &mut regs);
                assert_eq!(bits(&regs.v[9]), bits(&want), "the conv program, {taps} taps over {ch} channels");
            }
        }
    }
}

/// A Gated-DeltaNet head's read out of the program: RMS-normed by its weights, then times `silu(z)`.
#[test]
fn the_delta_read_out_program_is_its_norm_and_gate() {
    let mut r = Rng(37);
    for n in [16usize, 128, 256] {
        for _ in 0..20 {
            let (o, w, z) = (vector_of(&mut r, n), vector_of(&mut r, n), vector_of(&mut r, n));
            let want: Vec<f32> = rmsnorm(FpUnit::Soft, &o, &w, 1e-6).iter().zip(&z).map(|(&v, &g)| fmul(v, silu(FpUnit::Soft, g))).collect();
            let mut regs = Regs::new(5, 3);
            regs.v[0] = o;
            regs.v[1] = w;
            regs.v[2] = z;
            vector::run(FpUnit::Soft, &vector::gdn_out(FpUnit::Soft, 0, 1, 2, 3, 4, n, 1e-6), &mut regs);
            assert_eq!(bits(&regs.v[3]), bits(&want), "over {n}");
        }
    }
}

/// A trailing flush's one walk of a head's log gives the two folds its rows read: the fold over the tokens after the
/// `older` ones being replayed, then the fold over the whole log, each `gdn_fold` over its own tokens bit for bit, at
/// every split of logs from empty to two windows long, at both multipliers.
#[test]
fn a_trailing_walk_of_a_log_gives_the_fold_of_its_newer_tokens_and_of_the_whole_log() {
    let mut r = Rng(53);
    for n in [16usize, 128] {
        for len in 0..=2 * 8 + 1 {
            let tokens: Vec<(Vec<f32>, Vec<f32>, f32, f32)> = (0..len)
                .map(|_| (vector_of(&mut r, n), vector_of(&mut r, n), -r.activation().abs(), (r.next() >> 40) as f32 / (1u64 << 24) as f32))
                .collect();
            let log: Vec<Logged> = tokens.iter().map(|(k, v, log_decay, beta)| Logged { k, v, log_decay: *log_decay, beta: *beta }).collect();
            let q = vector_of(&mut r, n);
            for fp in [FpUnit::Soft, FpUnit::Hard] {
                for older in 0..=len {
                    let [newer, all] = gdn_fold_trailing(fp, &q, &log, n, older);
                    let (want_newer, want_all) = (gdn_fold(fp, &q, &log[older..], n), gdn_fold(fp, &q, &log, n));
                    let what = format!("{fp:?}, {len} tokens over {n}, the oldest {older} replaying");
                    assert_eq!((bits(&newer.0), bits(&newer.1)), (bits(&want_newer.0), bits(&want_newer.1)), "{what}: the newer tokens' fold");
                    assert_eq!((bits(&all.0), bits(&all.1)), (bits(&want_all.0), bits(&want_all.1)), "{what}: the whole log's fold");
                }
            }
        }
    }
}

/// The arg-max is the lowest index no element exceeds, NaNs passed over and the two zeros equal: so sixteen lanes,
/// each keeping its own first largest, folded in any order to the larger, the lower index of equals, give it; the
/// greedy program leaves it as an f32.
#[test]
fn the_arg_max_is_the_first_largest_in_any_fold() {
    let mut r = Rng(34);
    for _ in 0..3000 {
        let n = 1 + (r.next() % 300) as usize;
        let x: Vec<f32> = (0..n)
            .map(|_| match r.next() % 8 {
                0 => f32::NAN,
                1 => 0.0,
                2 => -0.0,
                3 => f32::NEG_INFINITY,
                4 => (r.next() % 5) as f32 - 2.0,
                _ => r.activation(),
            })
            .collect();
        let want = (0..n).find(|&i| !x[i].is_nan() && x.iter().all(|&v| v.is_nan() || v <= x[i])).unwrap_or(0);
        let got = argmax(&x);
        assert_eq!(got, want, "{x:?}");
        let mut lanes: Vec<Option<(f32, usize)>> = (0..16)
            .map(|l| {
                let mut best: Option<(f32, usize)> = None;
                for i in (l..n).step_by(16) {
                    if !x[i].is_nan() && best.is_none_or(|(v, _)| x[i] > v) {
                        best = Some((x[i], i));
                    }
                }
                best
            })
            .collect();
        let fold = |a: Option<(f32, usize)>, b: Option<(f32, usize)>| match (a, b) {
            (None, o) | (o, None) => o,
            (Some(a), Some(b)) => Some(if b.0 > a.0 || (b.0 == a.0 && b.1 < a.1) { b } else { a }),
        };
        while lanes.len() > 1 {
            let a = lanes.swap_remove((r.next() % lanes.len() as u64) as usize);
            let b = lanes.swap_remove((r.next() % lanes.len() as u64) as usize);
            lanes.push(fold(a, b));
        }
        assert_eq!(lanes[0].map_or(0, |(_, i)| i), got, "the lanes folded in a seeded order");
        let mut regs = Regs::new(1, 1);
        regs.v[0] = x.clone();
        vector::run(FpUnit::Soft, &vector::greedy(0, 0), &mut regs);
        assert_eq!(regs.s[0].to_bits(), (got as f32).to_bits(), "the greedy program");
    }
    assert_eq!(argmax(&[]), 0);
    assert_eq!(argmax(&[f32::NAN, f32::NAN]), 0, "nothing to take");
    assert_eq!(argmax(&[f32::NAN, 1.0]), 1, "a NaN first is passed over");
    assert_eq!(argmax(&[-0.0, 0.0]), 0, "the zeros are equal");
    assert_eq!(argmax(&[f32::NEG_INFINITY, f32::NEG_INFINITY]), 0);
    assert_eq!(argmax(&[f32::NEG_INFINITY, -1.0, -1.0]), 1);
}

/// Every coefficient and offset a coded row of any format the array reads takes lies within `COEF_BOUND`, over
/// seeded blocks and blocks of every scale and code at its extremes: the bound the exact block sums are taken in 32
/// bits under.
#[test]
fn every_coded_weight_lies_within_the_coefficient_bound() {
    let mut r = Rng(38);
    let bound = COEF_BOUND as i32;
    for d in DECODERS {
        let ty = d.ty;
        let bb = ty.block_bytes();
        let mut blocks: Vec<Vec<u8>> = (0..2000).map(|_| (0..bb).map(|_| r.byte()).collect()).collect();
        blocks.push(vec![0xff; bb]);
        blocks.push(vec![0x80; bb]);
        blocks.push(vec![0x00; bb]);
        blocks.push((0..bb).map(|i| if i % 2 == 0 { 0x80 } else { 0x7f }).collect());
        for raw in &blocks {
            let Some(Ok(row)) = CodedRow::decode(ty, raw) else { continue };
            let worst = row.coef.iter().chain(&row.offs).map(|&v| (v as i32).abs()).max().unwrap_or(0);
            assert!(worst <= bound, "{}: a coded weight of {worst} past the bound {bound}", ty.name());
        }
    }
}

/// Scaling by a power of two is not always one f32 product: `ldexp(x, n)` and `fmul(x, 2^n)` agree on every value an f32
/// operation leaves and every denormal it flushes, at every `n` from −24 to 0, except where the exact product lies
/// within half a unit below the normals — the product rounds up to the least normal, the exact scale flushes it to a
/// zero — which is why a rotation back scales on the vector unit by `vector::Op::Scale` and not by a product; and
/// `vector::Op::Scale` is `ldexp`, element for element.
#[test]
fn a_power_of_two_scale_parts_from_a_product_only_at_the_least_normal() {
    let mut xs = vec![0.0f32, -0.0, f32::INFINITY, f32::NEG_INFINITY, NAN];
    for e in 0u32..255 {
        for m in [0u32, 1, 2, 0x40_0000, 0x7f_ffff, 0x7f_fffe, 0x12_3456, 0x55_5555, 0x2a_aaab] {
            for s in [0u32, 1] {
                xs.push(f32::from_bits(s << 31 | e << 23 | m));
            }
        }
    }
    let least = f32::MIN_POSITIVE;
    let mut parted = 0;
    for n in -24..=0 {
        let k = ldexp(1.0, n);
        assert_eq!(k, 2f32.powi(n), "2^{n}");
        for &x in &xs {
            let (exact, product) = (ldexp(x, n), fmul(x, k));
            if exact.to_bits() != product.to_bits() {
                parted += 1;
                assert_eq!(exact.abs(), 0.0, "{x:e} (bits {:#010x}) times 2^{n}: the exact scale {exact:e}", x.to_bits());
                assert_eq!(product.abs(), least, "{x:e} (bits {:#010x}) times 2^{n}: the product {product:e}", x.to_bits());
                assert_eq!(exact.is_sign_negative(), x.is_sign_negative());
                assert_eq!(product.is_sign_negative(), x.is_sign_negative());
                // Within half a unit of the least normal: the significand's top bits all set, one exponent above it.
                assert_eq!(x.to_bits() & 0x7f_ffff, 0x7f_ffff, "{x:e} times 2^{n}");
            }
        }
    }
    assert!(parted > 0, "the band below the normals where the two part");
    let mut regs = vector::Regs::new(2, 1);
    for n in [-24i8, -7, -1, 0, 5] {
        regs.v[0] = xs.clone();
        vector::run(FpUnit::Soft, &[vector::Op::Scale { a: 0, n, d: 1 }], &mut regs);
        for (x, y) in xs.iter().zip(&regs.v[1]) {
            assert_eq!(y.to_bits(), ldexp(*x, n as i32).to_bits(), "{x:e} · 2^{n}");
        }
    }
}

/// The per-32 lane over what the array delivers — each 32-element sub-block's excess sums against its own activation
/// scale, a group's terms summed exactly and rounded once — gives the per-32 dot bit for bit, for every format and
/// both word counts.
#[test]
fn the_per_32_lane_over_the_arrays_excess_sums_is_the_per_32_dot() {
    use logicaffeine_compile::spatial::decode_engine::lane::{deliver32, group, SUB_BLOCK};
    let mut r = Rng(32);
    for d in DECODERS {
        for words in [Words::One, Words::Two] {
            for _ in 0..30 {
                let blocks_n = 1024 / d.ty.block_len();
                let raw = blocks(&mut r, d.ty, blocks_n);
                let row = CodedRow::decode(d.ty, &raw).unwrap().unwrap();
                let x: Vec<f32> = (0..1024).map(|_| r.activation()).collect();
                let act = quantize(&x, words);
                let subs: Vec<_> = raw
                    .chunks_exact(d.ty.block_bytes())
                    .enumerate()
                    .flat_map(|(b, blk)| {
                        let seg = &row.segments[b];
                        let u = units(d.ty, blk);
                        let act = &act;
                        (0..d.ty.block_len() / SUB_BLOCK).map(move |s| deliver32(&u, (seg.a, seg.b), d.offset, act, words, b, s)).collect::<Vec<_>>()
                    })
                    .collect();
                let mut acc = 0f32;
                for g in subs.chunks(GROUP / SUB_BLOCK) {
                    acc = group(g, acc);
                }
                assert_eq!(acc.to_bits(), dot(&row, &act).to_bits(), "{} {words:?}", d.ty.name());
            }
        }
    }
}

/// The repository root, from this crate's manifest.
fn repo() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The card's floating-point DSP block multiplies as fp-oracle6's receipt names it (its one agreeing class in
/// `benchmarks/silicon/d5005/fp_hard_receipts.json`), and [`FpUnit::CARD`] is that member; the block's add is among
/// the classes its receipt names, the engine's own among them.
#[test]
fn the_cards_multiply_is_the_member_its_receipt_names() {
    use logicaffeine_compile::codegen_sva::fp_hypotheses::Semantics;
    let ledger: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo().join("benchmarks/silicon/d5005/fp_hard_receipts.json")).expect("the receipts ledger")).expect("its JSON");
    let receipt = ledger["receipts"].as_array().expect("its receipts").iter().rev().find(|r| r["build"] == "fp-oracle6").expect("fp-oracle6's receipt");
    assert_eq!(receipt["multiply"]["class"], serde_json::json!([FpUnit::CARD.name()]), "the card's multiply");
    let adds: Vec<&str> = receipt["add"]["class"].as_array().expect("its add's classes").iter().map(|c| c.as_str().expect("a class name")).collect();
    assert!(adds.contains(&Semantics::ENGINE.name().as_str()), "the card's add is the engine's: {adds:?}");
}

/// The card's product is the engine's but where the card decides it tiny first: an exact product below the least normal
/// that the engine rounds up into it, `(1 − 2^-24)·2^-126` among them, is the engine's `±2^-126` and the card's zero of
/// its sign; and every NaN takes the operands' signs' exclusive or. Every other product — of the specials, of random
/// operands, and of operands whose product lies about the least normal — is the engine's, bit for bit, the band and the
/// NaNs both reached.
#[test]
fn the_cards_multiply_parts_from_the_engines_only_in_tininess_and_the_nans_sign() {
    use logicaffeine_compile::codegen_sva::fp_hypotheses::SPECIALS;
    let at = |u: FpUnit, a: u32, b: u32| u.mul(f32::from_bits(a), f32::from_bits(b)).to_bits();
    assert_eq!((at(FpUnit::Soft, 0x3f7f_ffff, 0x0080_0000), at(FpUnit::Hard, 0x3f7f_ffff, 0x0080_0000)), (0x0080_0000, 0x0000_0000));
    assert_eq!((at(FpUnit::Soft, 0xbf7f_ffff, 0x0080_0000), at(FpUnit::Hard, 0xbf7f_ffff, 0x0080_0000)), (0x8080_0000, 0x8000_0000));
    let mut r = Rng(0xfa7d_0c16);
    let mut pairs: Vec<(u32, u32)> = SPECIALS.iter().flat_map(|&a| SPECIALS.iter().map(move |&b| (a, b))).collect();
    pairs.extend((0..200_000).map(|_| (r.next() as u32, r.next() as u32)));
    // Operands whose exponents sum to about the least normal's, so their products lie about it.
    pairs.extend((0..400_000).map(|_| {
        let ea = 1 + (r.next() % 125) as u32;
        let eb = (127 + (r.next() % 3) as u32 - 1 - ea).max(1);
        ((r.next() as u32 & 0x8000_0000) | ea << 23 | (r.next() as u32 & 0x7f_ffff), (r.next() as u32 & 0x8000_0000) | eb << 23 | (r.next() as u32 & 0x7f_ffff))
    }));
    // The band, exactly: (1 − k·2^-24)·(1 + j·2^-23)·2^-126 lies (k − 2j + jk·2^-23)·2^-150 below the least normal, in
    // the band for k = 2j and j from 1449 to 2048; beside it k = 2j ± 1, either side of the band.
    for j in 1300u32..2200 {
        for k in [2 * j - 1, 2 * j, 2 * j + 1] {
            let a = (1.0 - k as f64 * 2f64.powi(-24)) as f32;
            assert_eq!(a as f64, 1.0 - k as f64 * 2f64.powi(-24), "an exact operand");
            for sign in [0u32, 0x8000_0000] {
                pairs.push((sign | a.to_bits(), 0x0080_0000 | j));
            }
        }
    }
    let (mut band, mut nans) = (0usize, 0usize);
    for (a, b) in pairs {
        let (s, h) = (at(FpUnit::Soft, a, b), at(FpUnit::Hard, a, b));
        if f32::from_bits(s).is_nan() {
            assert_eq!((s, h), (0x7fc0_0000, 0x7fc0_0000 | ((a ^ b) & 0x8000_0000)), "{a:08x} × {b:08x}: a NaN");
            nans += 1;
        } else if s != h {
            let exact = ftz(f32::from_bits(a)) as f64 * ftz(f32::from_bits(b)) as f64;
            assert!(
                s & 0x7fff_ffff == 0x0080_0000 && h == s & 0x8000_0000 && exact.abs() < f32::MIN_POSITIVE as f64,
                "{a:08x} × {b:08x}: the engine's {s:08x}, the card's {h:08x}, exactly {exact:e}"
            );
            band += 1;
        }
    }
    assert!(band >= 100 && nans >= 100, "the band reached {band} times, the NaNs {nans}");
}

/// A dot product at the card's unit takes the card's products: a product in the band is the card's zero and the
/// engine's least normal, and a dot of ordinary products is the same at both.
#[test]
fn a_dot_at_the_cards_unit_takes_the_cards_products() {
    let (a, b) = ([f32::from_bits(0x3f7f_ffff)], [f32::from_bits(0x0080_0000)]);
    assert_eq!((fdot(FpUnit::Soft, &a, &b).to_bits(), fdot(FpUnit::Hard, &a, &b).to_bits()), (0x0080_0000, 0));
    let mut r = Rng(0xd07);
    let (x, y): (Vec<f32>, Vec<f32>) = (0..300).map(|_| (r.activation(), r.activation())).unzip();
    assert_eq!(fdot(FpUnit::Soft, &x, &y).to_bits(), fdot(FpUnit::Hard, &x, &y).to_bits());
}

/// Every product the engine's arithmetic makes goes through its unit's multiply ([`FpUnit::mul`]): in the files that
/// define what the engine computes — the numerics, the vector unit's interpreter, the model, the machine and the
/// program — the engine's own `fmul` is called by the soft unit's multiply alone, so no function keeps a product the
/// card's unit would compute otherwise.
#[test]
fn every_product_of_the_engines_arithmetic_is_its_units() {
    let dir = repo().join("crates/logicaffeine_compile/src/spatial/decode_engine");
    for (file, allowed) in [("numerics.rs", 1usize), ("vector.rs", 0), ("model.rs", 0), ("machine.rs", 0), ("program.rs", 0)] {
        let text = std::fs::read_to_string(dir.join(file)).expect("the source");
        let calls = text.lines().filter(|l| !l.trim_start().starts_with("//")).map(|l| l.matches("fmul(").count() - l.matches("fn fmul(").count()).sum::<usize>();
        assert_eq!(calls, allowed, "{file}: the engine's own multiply called {calls} times past the soft unit's");
    }
    let numerics = std::fs::read_to_string(dir.join("numerics.rs")).unwrap();
    let unit = &numerics[numerics.find("impl FpUnit {").expect("the unit's impl")..];
    assert!(unit[..unit.find("\n}\n").unwrap()].contains("FpUnit::Soft => fmul("), "the soft unit's multiply is the engine's own");
}
