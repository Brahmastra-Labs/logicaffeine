//! The multi-word integer core against native 64- and 128-bit arithmetic, at every lane shape.
//!
//! Each core operation is built into a routine at a shape (words of 2 to 32 bits, multipliers of 1 to 16) and
//! run on exact lanes; its words, joined, must be the native result: ripple sums and differences with their
//! carries, the full 128-bit product from multiplier-width digits, restoring division, constant and variable
//! shifts (logical, arithmetic, and with the sticky bit of what fell off), the leading-zero count, the
//! population count, and the unsigned, signed and equality comparisons.

use logicaffeine_compile::accel::lane::{
    join_u128, run, split_u64, ExactLanes, LaneShape, Routine,
};
use logicaffeine_compile::accel::multiword::{Builder, Mw};

fn shapes() -> Vec<LaneShape> {
    vec![
        LaneShape::new(2, 1),
        LaneShape::new(4, 2),
        LaneShape::new(8, 4),
        LaneShape::new(16, 8),
        LaneShape::new(32, 16),
        LaneShape::new(32, 8),
    ]
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// 64-bit operands: the edges, and random values of every magnitude.
fn values() -> Vec<u64> {
    let mut v = vec![
        0,
        1,
        2,
        3,
        u64::MAX,
        u64::MAX - 1,
        1 << 63,
        (1 << 63) - 1,
        (1 << 63) + 1,
        1 << 32,
        (1 << 32) - 1,
        0xFFFF,
        0x1_0000,
        0xDEAD_BEEF_CAFE_F00D,
    ];
    let mut s = 0x0DDB_A11_u64;
    for _ in 0..14 {
        v.push(splitmix(&mut s));
        let k = splitmix(&mut s) % 64;
        v.push(splitmix(&mut s) >> k);
    }
    v
}

/// Build a routine of `arity` 64-bit inputs whose outputs are `f`'s values.
fn routine(
    shape: LaneShape,
    arity: usize,
    f: impl FnOnce(&mut Builder, &[Mw]) -> Vec<Mw>,
) -> (Routine, Vec<usize>) {
    let mut b = Builder::new(shape);
    let n = shape.words_for(64);
    let ins: Vec<Mw> = (0..arity).map(|_| b.mw_input(n)).collect();
    let outs = f(&mut b, &ins);
    let lens: Vec<usize> = outs.iter().map(Mw::len).collect();
    let inputs = ins.iter().flat_map(|m| m.0.iter().copied()).collect();
    let outputs = outs.iter().flat_map(|m| m.0.iter().copied()).collect();
    (b.finish("probe", inputs, outputs), lens)
}

/// Run a routine on 64-bit arguments; each output value joined back into a `u128`.
fn exec(shape: LaneShape, r: &(Routine, Vec<usize>), args: &[u64]) -> Vec<u128> {
    let n = shape.words_for(64);
    let words: Vec<u32> = args.iter().flat_map(|a| split_u64(shape, *a, n)).collect();
    let mut lanes = ExactLanes { shape };
    let (out, _) =
        run(&r.0, &mut lanes, &words).unwrap_or_else(|e| panic!("{shape:?} {}: {e}", r.0.name));
    let mut at = 0;
    r.1.iter()
        .map(|len| {
            let v = join_u128(shape, &out[at..at + len]);
            at += len;
            v
        })
        .collect()
}

/// A word output (a 0/1 flag or a carry) as a one-word value.
fn flag(bit: u32) -> Mw {
    Mw(vec![bit])
}

#[test]
fn sums_and_differences_ripple_with_their_carries() {
    let mut bad = Vec::new();
    for shape in shapes() {
        let add = routine(shape, 2, |b, x| {
            let z = b.k(0);
            let (s, c) = b.mw_add(&x[0], &x[1], z);
            vec![s, flag(c)]
        });
        let add1 = routine(shape, 2, |b, x| {
            let one = b.k(1);
            let (s, c) = b.mw_add(&x[0], &x[1], one);
            vec![s, flag(c)]
        });
        let sub = routine(shape, 2, |b, x| {
            let z = b.k(0);
            let (d, bo) = b.mw_sub(&x[0], &x[1], z);
            vec![d, flag(bo)]
        });
        let neg = routine(shape, 1, |b, x| vec![b.mw_neg(&x[0])]);
        for &a in &values() {
            for &c in &values() {
                let (s, o) = a.overflowing_add(c);
                if exec(shape, &add, &[a, c]) != vec![s as u128, o as u128] {
                    bad.push(format!("{shape:?} {a:#x} + {c:#x}"));
                }
                let full = a as u128 + c as u128 + 1;
                if exec(shape, &add1, &[a, c]) != vec![full as u64 as u128, full >> 64] {
                    bad.push(format!("{shape:?} {a:#x} + {c:#x} + 1"));
                }
                let (d, o) = a.overflowing_sub(c);
                if exec(shape, &sub, &[a, c]) != vec![d as u128, o as u128] {
                    bad.push(format!("{shape:?} {a:#x} - {c:#x}"));
                }
            }
            if exec(shape, &neg, &[a]) != vec![a.wrapping_neg() as u128] {
                bad.push(format!("{shape:?} -{a:#x}"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} sums/differences wrong:\n{}",
        bad.len(),
        bad.iter().take(30).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn the_full_product_is_the_128_bit_product() {
    let mut bad = Vec::new();
    for shape in shapes() {
        let mul = routine(shape, 2, |b, x| vec![b.mw_mul(&x[0], &x[1])]);
        for &a in &values() {
            for &c in &values() {
                let want = a as u128 * c as u128;
                let got = exec(shape, &mul, &[a, c]);
                if got != vec![want] {
                    bad.push(format!(
                        "{shape:?} {a:#x} * {c:#x}: got {:#x}, want {want:#x}",
                        got[0]
                    ));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} products wrong:\n{}",
        bad.len(),
        bad.iter().take(30).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn restoring_division_gives_quotient_and_remainder() {
    let mut bad = Vec::new();
    for shape in shapes() {
        let div = routine(shape, 2, |b, x| {
            let (q, r) = b.mw_divrem(&x[0], &x[1]);
            vec![q, r]
        });
        for &a in &values() {
            for &c in values().iter().filter(|c| **c != 0) {
                let want = vec![(a / c) as u128, (a % c) as u128];
                let got = exec(shape, &div, &[a, c]);
                if got != want {
                    bad.push(format!(
                        "{shape:?} {a:#x} / {c:#x}: got {got:x?}, want {want:x?}"
                    ));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} divisions wrong:\n{}",
        bad.len(),
        bad.iter().take(30).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn constant_shifts_move_bits_across_words() {
    let mut bad = Vec::new();
    for shape in shapes() {
        for s in [
            0u32,
            1,
            2,
            3,
            shape.word - 1,
            shape.word,
            shape.word + 1,
            31,
            32,
            33,
            62,
            63,
        ] {
            let shl = routine(shape, 1, |b, x| vec![b.mw_shl(&x[0], s)]);
            let shr = routine(shape, 1, |b, x| vec![b.mw_shr(&x[0], s)]);
            let sar = routine(shape, 1, |b, x| vec![b.mw_sar(&x[0], s)]);
            let sticky = routine(shape, 1, |b, x| {
                let (v, st) = b.mw_shr_sticky(&x[0], s);
                vec![v, flag(st)]
            });
            for &a in &values() {
                let checks = [
                    ("shl", &shl, vec![(a << s) as u128]),
                    ("shr", &shr, vec![(a >> s) as u128]),
                    ("sar", &sar, vec![((a as i64) >> s) as u64 as u128]),
                    (
                        "shr_sticky",
                        &sticky,
                        vec![
                            (a >> s) as u128,
                            u128::from(s > 0 && a & ((1u64 << s) - 1) != 0),
                        ],
                    ),
                ];
                for (name, r, want) in checks {
                    let got = exec(shape, r, &[a]);
                    if got != want {
                        bad.push(format!(
                            "{shape:?} {name}({a:#x}, {s}): got {got:x?}, want {want:x?}"
                        ));
                    }
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} constant shifts wrong:\n{}",
        bad.len(),
        bad.iter().take(30).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn variable_shifts_are_barrel_shifts_over_the_amount_bits() {
    let mut bad = Vec::new();
    for shape in shapes() {
        let shl = routine(shape, 2, |b, x| vec![b.mw_shl_var(&x[0], &x[1], 7)]);
        let sar = routine(shape, 2, |b, x| vec![b.mw_sar_var(&x[0], &x[1], 6)]);
        let shr = routine(shape, 2, |b, x| {
            let (v, st) = b.mw_shr_var_sticky(&x[0], &x[1], 7);
            vec![v, flag(st)]
        });
        for &a in &values() {
            for amt in [0u64, 1, 5, 31, 32, 33, 63, 64, 65, 100, 127] {
                let want_shl = if amt >= 64 { 0 } else { a << amt };
                if exec(shape, &shl, &[a, amt]) != vec![want_shl as u128] {
                    bad.push(format!("{shape:?} shl_var({a:#x}, {amt})"));
                }
                let want_shr = if amt >= 64 { 0 } else { a >> amt };
                let lost = if amt >= 64 {
                    a != 0
                } else {
                    amt > 0 && a & ((1u64 << amt) - 1) != 0
                };
                if exec(shape, &shr, &[a, amt]) != vec![want_shr as u128, u128::from(lost)] {
                    bad.push(format!("{shape:?} shr_var_sticky({a:#x}, {amt})"));
                }
                if amt < 64
                    && exec(shape, &sar, &[a, amt]) != vec![((a as i64) >> amt) as u64 as u128]
                {
                    bad.push(format!("{shape:?} sar_var({a:#x}, {amt})"));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} variable shifts wrong:\n{}",
        bad.len(),
        bad.iter().take(30).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn counts_leading_zeros_and_set_bits() {
    let mut bad = Vec::new();
    for shape in shapes() {
        let nc = shape.words_for(16);
        let clz = routine(shape, 1, |b, x| vec![b.mw_clz(&x[0], nc)]);
        let clz128 = routine(shape, 2, |b, x| {
            let wide = Mw(x[0].0.iter().chain(&x[1].0).copied().collect());
            vec![b.mw_clz(&wide, nc)]
        });
        let pop = routine(shape, 1, |b, x| vec![b.mw_popcount(&x[0], nc)]);
        for &a in &values() {
            if exec(shape, &clz, &[a]) != vec![a.leading_zeros() as u128] {
                bad.push(format!("{shape:?} clz({a:#x})"));
            }
            if exec(shape, &pop, &[a]) != vec![a.count_ones() as u128] {
                bad.push(format!("{shape:?} popcount({a:#x})"));
            }
            for &hi in &[0u64, 1, a] {
                let v = (hi as u128) << 64 | a as u128;
                if exec(shape, &clz128, &[a, hi]) != vec![v.leading_zeros() as u128] {
                    bad.push(format!("{shape:?} clz128({v:#x})"));
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} counts wrong:\n{}",
        bad.len(),
        bad.iter().take(30).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn comparisons_extensions_and_selects() {
    let mut bad = Vec::new();
    for shape in shapes() {
        let n = shape.words_for(64);
        let cmp = routine(shape, 2, |b, x| {
            let ltu = b.mw_ltu(&x[0], &x[1]);
            let lts = b.mw_lts(&x[0], &x[1]);
            let eq = b.mw_eq(&x[0], &x[1]);
            let z = b.mw_is_zero(&x[0]);
            let top = b.mw_top_bit(&x[0]);
            let bit5 = b.mw_bit(&x[0], 5);
            vec![
                flag(ltu),
                flag(lts),
                flag(eq),
                flag(z),
                flag(top),
                flag(bit5),
            ]
        });
        let ext = routine(shape, 1, |b, x| {
            let low = x[0].slice(0, n / 2);
            vec![b.mw_sext(&low, n), b.mw_resize(&low, n)]
        });
        let sel = routine(shape, 2, |b, x| {
            let c = b.mw_bit(&x[0], 0);
            vec![b.mw_sel(c, &x[0], &x[1])]
        });
        for &a in &values() {
            for &c in &values() {
                let want = vec![
                    u128::from(a < c),
                    u128::from((a as i64) < (c as i64)),
                    u128::from(a == c),
                    u128::from(a == 0),
                    (a >> 63) as u128,
                    ((a >> 5) & 1) as u128,
                ];
                if exec(shape, &cmp, &[a, c]) != want {
                    bad.push(format!(
                        "{shape:?} compare({a:#x}, {c:#x}): got {:x?}",
                        exec(shape, &cmp, &[a, c])
                    ));
                }
                if exec(shape, &sel, &[a, c]) != vec![if a & 1 == 1 { a } else { c } as u128] {
                    bad.push(format!("{shape:?} sel({a:#x}, {c:#x})"));
                }
            }
            let low = a as u32;
            let want = vec![low as i32 as i64 as u64 as u128, low as u128];
            if exec(shape, &ext, &[a]) != want {
                bad.push(format!("{shape:?} extend({a:#x})"));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} comparisons/extensions wrong:\n{}",
        bad.len(),
        bad.iter().take(30).cloned().collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn constants_and_repeated_state_carry_across_trips() {
    for shape in shapes() {
        let n = shape.words_for(64);
        let (r, lens) = {
            let mut b = Builder::new(shape);
            let x = b.mw_input(n);
            let acc = b.mw_state(&x);
            b.repeat(10, |s| {
                let three = s.mw_const(3, n);
                let next = s.mw_plus(&acc, &three);
                s.mw_assign(&acc, &next);
            });
            let lens = vec![acc.len()];
            (b.finish("tenfold", x.0.clone(), acc.0.clone()), lens)
        };
        for &a in &values() {
            assert_eq!(
                exec(shape, &(r.clone(), lens.clone()), &[a]),
                vec![a.wrapping_add(30) as u128],
                "{shape:?}: ten trips of +3 from {a:#x}"
            );
        }
        let (k, lens) = routine(shape, 0, |b, _| {
            vec![b.mw_const(0x0123_4567_89AB_CDEF_0011_2233_4455_6677, 2 * n)]
        });
        let mut lanes = ExactLanes { shape };
        let (out, _) = run(&k, &mut lanes, &[]).expect("a constant routine runs");
        assert_eq!(lens, vec![2 * n]);
        assert_eq!(
            join_u128(shape, &out),
            0x0123_4567_89AB_CDEF_0011_2233_4455_6677,
            "{shape:?}: a 128-bit constant"
        );
    }
}

/// A state update is a parallel assignment: a new value built from the state's own words (a whole-word
/// shift or a rotation, which re-use the state registers themselves) lands as if every word were read before
/// any was written.
#[test]
fn a_state_update_is_a_parallel_assignment() {
    for shape in shapes() {
        let n = shape.words_for(64);
        let w = shape.word;
        let (shifted, rotated) = {
            let mut b = Builder::new(shape);
            let x = b.mw_input(n);
            let state = b.mw_state(&x);
            b.repeat(3, |s| {
                let next = s.mw_shl(&state, w);
                s.mw_assign(&state, &next);
            });
            let shifted = (b.finish("shift", x.0.clone(), state.0.clone()), vec![n]);
            let mut b = Builder::new(shape);
            let x = b.mw_input(n);
            let state = b.mw_state(&x);
            b.repeat(5, |s| {
                let mut words = state.0.clone();
                words.rotate_right(1);
                s.mw_assign(&state, &Mw(words));
            });
            let rotated = (b.finish("rotate", x.0.clone(), state.0.clone()), vec![n]);
            (shifted, rotated)
        };
        for &a in &values() {
            let want_shift = if 3 * w >= 64 { 0 } else { a << (3 * w) };
            assert_eq!(
                exec(shape, &shifted, &[a]),
                vec![want_shift as u128],
                "{shape:?}: three one-word shifts of {a:#x}"
            );
            let want_rot = a.rotate_left(5 * w);
            assert_eq!(
                exec(shape, &rotated, &[a]),
                vec![want_rot as u128],
                "{shape:?}: five one-word rotations of {a:#x}"
            );
        }
    }
}
