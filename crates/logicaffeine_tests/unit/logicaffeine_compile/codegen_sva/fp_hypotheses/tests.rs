use super::*;

#[test]
fn the_family_holds_every_meaningful_choice_once_the_engine_first() {
    let f = family();
    assert_eq!(f[0], Semantics::ENGINE);
    assert_eq!(f.len(), 2 * 7 + 2 * 3 * 2 * 7, "kept results: inputs x NaNs; flushed: inputs x tininess x sign x NaNs");
    for (i, a) in f.iter().enumerate() {
        assert!(!f[i + 1..].contains(a), "{} twice", a.name());
    }
    assert!(f.contains(&Semantics::IEEE), "an x86 core's arithmetic is a member");
    assert!(f.contains(&Semantics { nan: Nan::Propagate { invalid: 0x7fc0_0000 }, ..Semantics::IEEE }), "and an ARM core's");
    let card = Semantics { tininess: Tininess::AfterRoundingUnbounded, nan: Nan::XorSigned(0x7fc0_0000), ..Semantics::ENGINE };
    assert!(f.contains(&card), "and the D5005's multiply, as its oracle measured it");
}

#[test]
fn an_xor_signed_nan_takes_the_operands_signs_and_the_default_quiet_nan() {
    let s = Semantics { nan: Nan::XorSigned(0x7fc0_0000), ..Semantics::ENGINE };
    assert_eq!(s.name(), "din-flush.dout-flush.tiny-after-denormals.sign-kept.nan-xor-7fc00000");
    assert_eq!(nan_result(&s), "{sa ^ sb, 31'h7fc00000}");
    assert!(fmul_verilog("m", &s).contains("{sa ^ sb, 31'h7fc00000}") && fadd_verilog("m", &s).contains("{sa ^ sb, 31'h7fc00000}"));
}

#[test]
fn every_member_has_its_own_name() {
    let names: Vec<String> = family().iter().map(|s| s.name()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len());
    assert_eq!(Semantics::ENGINE.name(), "din-flush.dout-flush.tiny-after-denormals.sign-kept.nan-7fc00000");
    assert_eq!(Semantics::IEEE.name(), "din-keep.dout-keep.nan-prop-ffc00000");
}

#[test]
fn kept_results_ignore_tininess_and_flushed_sign() {
    let s = Semantics {
        denormal_inputs: Denormals::Keep,
        denormal_results: Denormals::Keep,
        tininess: Tininess::BeforeRounding,
        flushed_sign: FlushSign::Positive,
        nan: Nan::Canonical(0xffff_ffff),
    };
    assert_eq!(fmul_verilog("m", &s), fmul_verilog("m", &s.canonical()));
    assert_eq!(fadd_verilog("m", &s), fadd_verilog("m", &s.canonical()));
}

#[test]
fn each_choice_changes_the_unit_it_prints() {
    let base = Semantics::ENGINE;
    for other in family().into_iter().skip(1) {
        assert!(
            fmul_verilog("m", &base) != fmul_verilog("m", &other) || fadd_verilog("m", &base) != fadd_verilog("m", &other),
            "{}",
            other.name()
        );
    }
}

#[test]
fn a_unit_is_one_module_with_its_ports() {
    for v in [fmul_verilog("unit_mul", &Semantics::IEEE), fadd_verilog("unit_add", &Semantics::ENGINE)] {
        assert_eq!(v.matches("module ").count(), 1);
        assert!(v.contains("(input [31:0] a, input [31:0] b, output [31:0] y);"));
        assert!(v.contains("  assign y = (") && !v.contains(" y = 32") && !v.contains("always @* begin\n    if"), "the result one chain of choices");
        assert!(v.trim_end().ends_with("endmodule"));
    }
}

/// Pairs: every pair of specials, then random operands crowding the denormals, the least normal, the overflow edge and
/// the cancellation band, with mantissas that make ties.
fn pairs() -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = SPECIALS.iter().flat_map(|&a| SPECIALS.iter().map(move |&b| (a, b))).collect();
    let mut s: u64 = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..40_000 {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let exp = |k: u64| -> u32 { [(k >> 3) as u32 % 4, 60 + (k >> 3) as u32 % 8, 124 + (k >> 3) as u32 % 6, 248 + (k >> 3) as u32 % 7, (k >> 3) as u32 % 256][k as usize % 5] };
        let mant = |k: u64| -> u32 { [0x7f_ffff, 0x40_0000 | ((k >> 8) as u32 & 1), 0, (k >> 9) as u32 & 0x7f_ffff][k as usize % 4] };
        let a = ((s >> 63) as u32) << 31 | exp(s >> 1) << 23 | mant(s >> 17);
        let b = ((s >> 62 & 1) as u32) << 31 | exp(s >> 21) << 23 | mant(s >> 34);
        out.push((a, b));
        out.push((a, a ^ ((s >> 41) as u32 & 0x3f) ^ 0x8000_0000));
    }
    out
}

#[test]
fn the_engine_member_multiplies_and_adds_as_the_engines_numerics() {
    use crate::spatial::decode_engine::numerics::{fadd, fmul};
    for (a, b) in pairs() {
        let (x, y) = (f32::from_bits(a), f32::from_bits(b));
        assert_eq!(Semantics::ENGINE.mul(a, b), fmul(x, y).to_bits(), "{a:08x} x {b:08x}");
        assert_eq!(Semantics::ENGINE.add(a, b), fadd(x, y).to_bits(), "{a:08x} + {b:08x}");
    }
}

/// The IEEE member is this machine's own `f32` arithmetic: an x86 core's, its NaNs the first NaN operand's payload
/// quieted and `0xffc00000` for an invalid operation (two NaN operands: either one's, as a compiler may commute them).
#[test]
fn the_ieee_member_multiplies_and_adds_as_this_machine() {
    if !cfg!(target_arch = "x86_64") {
        return;
    }
    let nan = |v: u32| v & 0x7fff_ffff > 0x7f80_0000;
    let agrees = |a: u32, b: u32, host: u32, unit: u32| if nan(a) && nan(b) { unit == a | 0x40_0000 || unit == b | 0x40_0000 } else { unit == host };
    for (a, b) in pairs() {
        let (x, y) = (f32::from_bits(a), f32::from_bits(b));
        assert!(agrees(a, b, (x * y).to_bits(), Semantics::IEEE.mul(a, b)), "{a:08x} x {b:08x}: {:08x}", Semantics::IEEE.mul(a, b));
        assert!(agrees(a, b, (x + y).to_bits(), Semantics::IEEE.add(a, b)), "{a:08x} + {b:08x}: {:08x}", Semantics::IEEE.add(a, b));
    }
}

/// The card's multiply, as its oracle named it, is the engine's but for two things: its NaN takes the operands' signs,
/// and it flushes a product the engine rounds up into the least normal. `(1 - 2^-24) x 2^-126 = 2^-126 - 2^-150` is a tie
/// at the denormals' place, which the engine rounds to even, up to `2^-126`; exact at 24 bits, the card decides it tiny.
#[test]
fn the_cards_multiply_is_the_engines_but_for_its_nan_sign_and_the_least_normal() {
    let card = Semantics { tininess: Tininess::AfterRoundingUnbounded, nan: Nan::XorSigned(0x7fc0_0000), ..Semantics::ENGINE };
    assert_eq!(Semantics::ENGINE.mul(0x3f7f_ffff, 0x0080_0000), 0x0080_0000);
    assert_eq!(card.mul(0x3f7f_ffff, 0x0080_0000), 0x0000_0000);
    assert_eq!(card.mul(0xbf7f_ffff, 0x0080_0000), 0x8000_0000, "flushed with its sign");
    assert_eq!(card.mul(0x7f80_0000, 0x8000_0000), 0xffc0_0000, "infinity times -0: the NaN the signs make");
    let nan = |v: u32| v & 0x7fff_ffff > 0x7f80_0000;
    for (a, b) in pairs() {
        let (c, e) = (card.mul(a, b), Semantics::ENGINE.mul(a, b));
        if c == e {
            continue;
        }
        if nan(c) && nan(e) {
            assert_eq!((c, e), ((a ^ b) & 0x8000_0000 | 0x7fc0_0000, 0x7fc0_0000), "{a:08x} x {b:08x}");
        } else {
            let s = (a ^ b) & 0x8000_0000;
            assert_eq!((c, e), (s, s | 0x0080_0000), "{a:08x} x {b:08x}: flushed where the engine reaches the least normal");
        }
    }
}
