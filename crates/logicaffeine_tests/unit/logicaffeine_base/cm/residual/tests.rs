use super::*;
use crate::entropy::squash;

/// Code `byte` bit by bit under predictor 0, returning its cost in bits, then count it.
fn code(r: &mut Residuals, byte: u8) -> f64 {
    let mut c0 = 1u32;
    let mut bits = 0.0;
    for depth in 0..8 {
        let bit = (byte >> (7 - depth)) & 1;
        let p = f64::from(squash(r.input(0, c0, depth))) / 4096.0;
        bits -= if bit == 1 { p.log2() } else { (1.0 - p).log2() };
        c0 = c0 << 1 | u32::from(bit);
    }
    r.update(byte);
    bits
}

#[test]
fn an_unseen_histogram_says_one_half_and_counts_add_up() {
    let mut r = Residuals::new(2, 4);
    r.set(0, 100, 0);
    r.set(1, 7, 3);
    for depth in 0..8 {
        assert!(r.input(0, 1 << depth, depth).abs() <= 1, "depth {depth}: one half, stretched");
    }
    r.update(103);
    assert_eq!(r.count(0, (CENTER + 3) % BINS, 1), 1, "residual +3 for predictor 0");
    assert_eq!(r.count(1, (CENTER + 96) % BINS, 1), 1, "residual +96 for predictor 1");
    assert_eq!(r.count(0, 0, BINS), 1);
    assert_eq!(r.count(1, 200, 100), 1, "a range that wraps past the last bin counts its first bins too");
    assert_eq!(r.count(1, 200, 50), 0);
}

#[test]
fn one_residual_distribution_serves_every_predicted_value() {
    // The truth is always the prediction plus 2 or 3, whatever the prediction is: once learnt at a few values the
    // residual codes cheaply at values never seen before.
    let mut r = Residuals::new(1, 1);
    let mut s = 1u64;
    for _ in 0..2000 {
        s = s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let predicted = 40 + (s >> 60) as u8;
        r.set(0, predicted, 0);
        code(&mut r, predicted + 2 + (s >> 59 & 1) as u8);
    }
    let mut total = 0.0;
    for predicted in [200u8, 3, 250, 128] {
        r.set(0, predicted, 0);
        total += code(&mut r, predicted.wrapping_add(2));
    }
    assert!(total / 4.0 < 1.5, "{:.2} bits a byte at predicted values never seen", total / 4.0);
}

#[test]
fn buckets_keep_their_distributions_apart_and_counts_age_without_overflow() {
    let mut r = Residuals::new(1, 2);
    for _ in 0..500 {
        r.set(0, 50, 0);
        r.update(50);
        r.set(0, 50, 1);
        r.update(90);
    }
    r.set(0, 50, 0);
    let in_quiet = code(&mut r, 50);
    r.set(0, 50, 1);
    let in_busy = code(&mut r, 90);
    assert!(in_quiet < 0.1 && in_busy < 0.1, "{in_quiet:.3} / {in_busy:.3} bits");
    for _ in 0..200_000 {
        r.set(0, 10, 0);
        r.update(10);
    }
    let s = &r.sums[..BINS];
    assert!(s.windows(2).all(|w| w[0] <= w[1]), "prefix sums stay sums after halving");
    assert!(s[BINS - 1] < u16::MAX);
}
