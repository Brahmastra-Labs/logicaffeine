use super::*;

/// SplitMix64 — a deterministic, nonlinear source for test data.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A probability in `1..4096` (12-bit), never a degenerate 0 or 4096.
    fn p12(&mut self) -> u16 {
        1 + (self.next() % 4095) as u16
    }

    /// A Bernoulli draw that is 1 with probability `p12 / 4096`.
    fn bit(&mut self, p12: u16) -> bool {
        (self.next() % 4096) < p12 as u64
    }
}

fn h(p: f64) -> f64 {
    if p <= 0.0 || p >= 1.0 {
        0.0
    } else {
        -(p * p.log2() + (1.0 - p) * (1.0 - p).log2())
    }
}

#[test]
fn arithmetic_coder_round_trips_random_bits_under_random_probabilities() {
    for seed in 0..8u64 {
        let mut rng = Rng(seed);
        let n = 50_000;
        let mut ps = Vec::with_capacity(n);
        let mut bits = Vec::with_capacity(n);
        for _ in 0..n {
            let p = rng.p12();
            ps.push(p);
            bits.push(rng.bit(p));
        }
        let mut enc = Encoder::new();
        for (&b, &p) in bits.iter().zip(&ps) {
            enc.encode(b, p);
        }
        let bytes = enc.finish();
        let mut dec = Decoder::new(&bytes);
        for (i, (&b, &p)) in bits.iter().zip(&ps).enumerate() {
            assert_eq!(dec.decode(p), b, "seed {seed}: bit {i} decoded wrong");
        }
    }
}

#[test]
fn arithmetic_coder_round_trips_bits_that_contradict_the_model() {
    // The worst case for a range coder: every bit is the one the model called least likely.
    for &p in &[1u16, 2, 7, 2048, 4088, 4095] {
        let n = 20_000;
        let bits: Vec<bool> = (0..n).map(|i| if p >= 2048 { i % 97 == 0 } else { i % 97 != 0 }).collect();
        let mut enc = Encoder::new();
        for &b in &bits {
            enc.encode(b, p);
        }
        let bytes = enc.finish();
        let mut dec = Decoder::new(&bytes);
        for (i, &b) in bits.iter().enumerate() {
            assert_eq!(dec.decode(p), b, "p={p}: bit {i}");
        }
    }
}

#[test]
fn an_empty_stream_round_trips_and_stays_tiny() {
    let bytes = Encoder::new().finish();
    assert!(bytes.len() <= 4, "an empty stream flushes at most the coder state, got {}", bytes.len());
    let _ = Decoder::new(&bytes);
}

#[test]
fn coding_cost_tracks_the_entropy_of_the_source() {
    // Bits drawn at a known probability and coded AT that probability must cost n·H(p) bits, up to the
    // 12-bit quantization of the model and a few bytes of flush.
    for &p12 in &[41u16, 410, 1024, 2048, 3500, 4050] {
        let mut rng = Rng(u64::from(p12));
        let n = 200_000usize;
        let mut enc = Encoder::new();
        for _ in 0..n {
            enc.encode(rng.bit(p12), p12);
        }
        let got_bits = enc.finish().len() as f64 * 8.0;
        let ideal = n as f64 * h(f64::from(p12) / 4096.0);
        assert!(
            got_bits <= ideal * 1.01 + 64.0,
            "p12={p12}: {got_bits} bits vs entropy bound {ideal:.0}"
        );
    }
}

#[test]
fn the_decoder_never_panics_on_truncated_or_arbitrary_bytes() {
    let mut rng = Rng(7);
    for len in 0..64usize {
        let junk: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let mut dec = Decoder::new(&junk);
        for _ in 0..4096 {
            let p = rng.p12();
            let _ = dec.decode(p);
        }
    }
}

#[test]
fn squash_is_monotone_and_stretch_inverts_it() {
    let mut prev = 0u16;
    for d in -2047..=2047 {
        let p = squash(d);
        assert!(p >= prev, "squash not monotone at {d}");
        assert!((1..4096).contains(&p), "squash({d}) = {p} leaves (0, 4096)");
        prev = p;
    }
    assert!(squash(0).abs_diff(2048) <= 1, "squash(0) is one half");
    assert!(squash(-4000) < 8 && squash(4000) > 4088, "squash saturates at the ends");
    for p in 1..4096u16 {
        let back = squash(stretch(p));
        // The table's resolution is coarsest in the tails; in the middle the inverse is near-exact.
        let tol = if (64..4032).contains(&p) { p / 32 + 2 } else { p.min(4096 - p) + 2 };
        assert!(back.abs_diff(p) <= tol, "squash(stretch({p})) = {back}");
    }
    for d in -2047..=2047 {
        assert!(stretch(squash(d)) <= 2047 && stretch(squash(d)) >= -2047);
    }
}

#[test]
fn an_adaptive_probability_learns_a_biased_source_quickly() {
    let mut cell = AdaptiveProb::new();
    assert_eq!(cell.p(), 2048, "a fresh cell predicts one half");
    for _ in 0..40 {
        cell.update(true);
    }
    assert!(cell.p() > 3900, "after 40 ones the cell is confident: {}", cell.p());
    for _ in 0..40 {
        cell.update(false);
    }
    assert!(cell.p() < 1500, "it keeps adapting when the source flips: {}", cell.p());
}

#[test]
fn the_mixer_learns_to_trust_the_informative_input() {
    // Input 0 is a confident, correct predictor; input 1 is a confident liar; input 2 is noise. The mixer
    // must code the stream far cheaper than the 1 bit/symbol a fixed one-half model pays.
    let mut rng = Rng(11);
    let mut mixer = Mixer::new(3, 1);
    let n = 30_000usize;
    let mut enc = Encoder::new();
    for _ in 0..n {
        let bit = rng.next() % 2 == 0;
        let truth = if bit { 3800 } else { 296 };
        let liar = 4096 - truth;
        let noise = rng.p12();
        mixer.set(0, stretch(truth));
        mixer.set(1, stretch(liar));
        mixer.set(2, stretch(noise));
        let p = mixer.mix(0);
        enc.encode(bit, p);
        mixer.update(bit);
    }
    let bits = enc.finish().len() as f64 * 8.0;
    let ideal = n as f64 * h(296.0 / 4096.0);
    assert!(bits < n as f64 * 0.5, "mixer cost {bits} bits for {n} symbols");
    assert!(bits < ideal * 1.6, "mixer cost {bits} vs the truthful input's own {ideal:.0}");
}

#[test]
fn the_apm_recalibrates_a_systematically_overconfident_input() {
    // The input always claims p(1) = 0.95 but the source is 1 only 60% of the time; the APM must learn
    // the real rate and code below what trusting the input costs.
    let mut rng = Rng(5);
    let mut apm = Apm::new(1);
    let claimed = 3891u16;
    let n = 40_000usize;
    let (mut raw, mut refined) = (Encoder::new(), Encoder::new());
    for _ in 0..n {
        let bit = rng.bit(2458);
        raw.encode(bit, claimed);
        let p = apm.refine(claimed, 0);
        refined.encode(bit, p);
        apm.update(bit);
    }
    let (raw, refined) = (raw.finish().len(), refined.finish().len());
    let ideal = n as f64 * h(0.6) / 8.0;
    assert!(refined < raw * 2 / 3, "APM {refined} B vs raw {raw} B");
    assert!((refined as f64) < ideal * 1.05 + 64.0, "APM {refined} B vs entropy {ideal:.0} B");
}

#[test]
fn hashing_is_deterministic_and_spreads_nearby_keys() {
    assert_eq!(hash2(1, 2), hash2(1, 2));
    assert_ne!(hash2(1, 2), hash2(2, 1));
    let buckets: std::collections::HashSet<u64> = (0..4096u64).map(|i| hash2(i, 0) >> 52).collect();
    assert!(buckets.len() > 2500, "consecutive keys land in {} of 4096 top-12-bit buckets", buckets.len());
}

#[test]
fn the_integer_coding_cost_matches_the_logarithm() {
    for p in [1u16, 2, 17, 410, 1024, 2048, 3000, 4095] {
        for bit in [false, true] {
            let q = if bit { f64::from(p) } else { 4096.0 - f64::from(p) } / 4096.0;
            let want = -q.log2() * 256.0;
            let got = f64::from(cost(bit, p));
            assert!((got - want).abs() <= 2.0, "cost({bit}, {p}) = {got}, want {want:.1}");
        }
    }
    assert_eq!(cost(true, 2048), 256, "a fair bit costs exactly one bit");
}

#[test]
fn the_two_layer_mixer_learns_what_two_selector_contexts_each_explain() {
    // Input 0 tells the truth exactly when context `a` is 0, input 1 exactly when context `b` is 0. A mixer
    // whose weights are chosen by `a` alone cannot learn when to trust input 1; two first-layer mixers,
    // one chosen by `a` and one by `b`, with a final mixer over them, can.
    let mut rng = Rng(21);
    let n = 60_000usize;
    let mut single = Mixer::new(3, 2);
    let mut two = Mixer2::new(3, &[2, 2], 1);
    let (mut enc1, mut enc2) = (Encoder::new(), Encoder::new());
    for _ in 0..n {
        let bit = rng.next() % 2 == 0;
        let (a, b) = ((rng.next() % 2) as usize, (rng.next() % 2) as usize);
        let truth = |good: bool| if good == bit { 3900u16 } else { 196 };
        let x0 = if a == 0 { truth(true) } else { rng.p12() };
        let x1 = if b == 0 { truth(true) } else { rng.p12() };
        for (i, x) in [stretch(x0), stretch(x1), 256].into_iter().enumerate() {
            single.set(i, x);
            two.set(i, x);
        }
        let p1 = single.mix(a);
        enc1.encode(bit, p1);
        single.update(bit);
        let p2 = two.mix(&[a, b], 0);
        enc2.encode(bit, p2);
        two.update(bit);
    }
    let (c1, c2) = (enc1.finish().len(), enc2.finish().len());
    assert!(c2 * 10 < c1 * 9, "two-layer mixer {c2} B vs single mixer {c1} B");
}

#[test]
fn the_mixer_kernels_compute_their_scalar_definitions_exactly() {
    let mut rng = Rng(0x5EED);
    for round in 0..2_000 {
        let n = LANES * (1 + round % 24);
        let x: Vec<i16> = (0..n).map(|_| (rng.next() % 4095) as i16 - 2047).collect();
        let w: Vec<i16> = (0..n).map(|_| rng.next() as u16 as i16).collect();
        let pairs: i32 = x
            .chunks_exact(2)
            .zip(w.chunks_exact(2))
            .map(|(a, b)| (i32::from(a[0]) * i32::from(b[0]) + i32::from(a[1]) * i32::from(b[1])) >> 8)
            .sum();
        assert_eq!(dot16(&x, &w), pairs >> (W_SHIFT - 8), "dot product, round {round}");
        let err = (rng.next() % 65_536) as i32 - 32_768;
        let mut trained = w.clone();
        train16(&mut trained, &x, err);
        for i in 0..n {
            let step = (i32::from(x[i]) * err + (1 << 15)) >> 16;
            let expected = (i32::from(w[i]) + step).clamp(-32_768, 32_767) as i16;
            assert_eq!(trained[i], expected, "update {i}, round {round}, err {err}");
        }
    }
}

// ---- Fine probabilities and the final calibration ------------------------------------------

#[test]
fn the_twelve_bit_coder_is_the_fine_coder_at_p12_shifted_into_place() {
    // Every format built on 12-bit probabilities must code exactly what it did before the coder took 32-bit ones.
    for seed in 0..6u64 {
        let mut rng = Rng(100 + seed);
        let (mut coarse, mut fine) = (Encoder::new(), Encoder::new());
        let mut coded = Vec::new();
        for _ in 0..40_000 {
            let p = match rng.next() % 4 {
                0 => 1 + (rng.next() % 3) as u16,
                1 => 4095 - (rng.next() % 3) as u16,
                _ => rng.p12(),
            };
            let bit = rng.bit(p);
            coarse.encode(bit, p);
            fine.encode_fine(bit, u32::from(p) << 20);
            coded.push((bit, p));
        }
        let (coarse, fine) = (coarse.finish(), fine.finish());
        assert_eq!(coarse, fine, "seed {seed}");
        let mut dec = Decoder::new(&fine);
        for (i, &(bit, p)) in coded.iter().enumerate() {
            assert_eq!(dec.decode_fine(u32::from(p) << 20), bit, "seed {seed}: bit {i}");
        }
    }
}

#[test]
fn fine_probabilities_round_trip_at_every_extreme() {
    for seed in 0..6u64 {
        let mut rng = Rng(200 + seed);
        let mut enc = Encoder::new();
        let mut coded = Vec::new();
        for _ in 0..60_000 {
            let p = match rng.next() % 5 {
                0 => 1 + (rng.next() % 16) as u32,
                1 => u32::MAX - (rng.next() % 16) as u32,
                2 => (rng.next() >> 32) as u32 | 1,
                3 => 1 << (rng.next() % 32),
                _ => u32::MAX >> (rng.next() % 32),
            };
            // Bits drawn at the probability, and sometimes against it — the coder must survive both.
            let bit = if rng.next() % 50 == 0 { rng.next() & 1 == 1 } else { (rng.next() >> 32) < u64::from(p) };
            enc.encode_fine(bit, p);
            coded.push((bit, p));
        }
        let bytes = enc.finish();
        let mut dec = Decoder::new(&bytes);
        for (i, &(bit, p)) in coded.iter().enumerate() {
            assert_eq!(dec.decode_fine(p), bit, "seed {seed}: bit {i} at p {p:#010x}");
        }
    }
}

#[test]
fn a_fine_probability_codes_a_near_certain_bit_below_what_twelve_bits_can() {
    // A million ones: at 4095/4096 they cost 1e6 · 0.00035 bits ≈ 44 B; at 1 − 2⁻²⁴ about 0.1 B.
    let n = 1_000_000;
    let (mut coarse, mut fine) = (Encoder::new(), Encoder::new());
    for _ in 0..n {
        coarse.encode(true, 4095);
        fine.encode_fine(true, u32::MAX - (1 << 8));
    }
    let (coarse, fine) = (coarse.finish().len(), fine.finish().len());
    assert!((40..=50).contains(&coarse), "the 12-bit coder wrote {coarse} B");
    assert!(fine <= 4, "the fine coder wrote {fine} B");
}

#[test]
fn a_calibration_starts_as_the_identity() {
    let mut c = Calibration::new(3);
    for ctx in 0..3 {
        for p12 in 1..4096u16 {
            let p = c.p(p12, ctx);
            let identity = u32::from(p12) << 20;
            assert!(p.abs_diff(identity) <= 1 << 21, "context {ctx}, p12 {p12}: {p:#010x} against {identity:#010x}");
        }
    }
}

#[test]
fn a_calibration_learns_a_bucket_surer_than_twelve_bits_can_say() {
    let mut c = Calibration::new(1);
    for _ in 0..1_000_000 {
        c.p(4095, 0);
        c.update(true);
    }
    let p = c.p(4095, 0);
    assert!(u32::MAX - p < 1 << 12, "after a million ones the top bucket says {p:#010x}");
    // One miss is remembered, but it does not undo a million hits.
    c.update(false);
    let after = c.p(4095, 0);
    assert!(after < p && u32::MAX - after < 1 << 13, "{after:#010x} after a miss");
}

#[test]
fn a_calibration_corrects_a_biased_predictor_and_keeps_contexts_apart() {
    // The predictor says 1/2 but context 0's bits are 1 nine times in ten and context 1's one time in ten.
    let mut rng = Rng(7);
    let mut c = Calibration::new(2);
    for _ in 0..200_000 {
        for (ctx, rate) in [(0, 3686u16), (1, 410)] {
            c.p(2048, ctx);
            c.update(rng.bit(rate));
        }
    }
    let (p0, p1) = (f64::from(c.p(2048, 0)) / 4_294_967_296.0, f64::from(c.p(2048, 1)) / 4_294_967_296.0);
    assert!((p0 - 0.9).abs() < 0.01 && (p1 - 0.1).abs() < 0.01, "context 0 says {p0:.4}, context 1 {p1:.4}");
    assert!((c.p(1000, 0) as i64 - (1000i64 << 20)).abs() <= 1 << 21, "an untouched bucket keeps its prior");
}

#[test]
fn a_calibrations_counts_halve_before_they_can_overflow() {
    let mut c = Calibration::new(1);
    let i = 4095;
    c.counts[i] = (3, COUNT_CAP);
    c.p(4095, 0);
    c.update(true);
    assert_eq!(c.counts[i], (1, (COUNT_CAP + 1) / 2));
    let p = c.p(4095, 0);
    assert!(p > u32::MAX - (1 << 4), "{p:#010x}");
    c.counts[i] = (COUNT_CAP, COUNT_CAP);
    c.update(false);
    assert!(c.counts[i].0 <= COUNT_CAP && c.counts[i].1 <= COUNT_CAP);
    let p = c.p(4095, 0);
    assert!(p.abs_diff(1 << 31) < 1 << 12, "{p:#010x}");
}

/// Codes `n` bits drawn at `rate(i)` (of 4096) through one APM context whose input always claims `claimed`.
fn apm_coded(apm: &mut Apm, seed: u64, n: usize, claimed: u16, rate: impl Fn(usize) -> u16) -> usize {
    let mut rng = Rng(seed);
    let mut enc = Encoder::new();
    for i in 0..n {
        let bit = rng.bit(rate(i));
        enc.encode(bit, apm.refine(claimed, 0));
        apm.update(bit);
    }
    enc.finish().len()
}

#[test]
fn an_apm_that_counts_its_updates_codes_a_steady_source_below_a_fixed_rate_one() {
    // The input says 0.5 and the bit is 1 with p = 4055/4096: the fixed-rate knot keeps moving 1/128 of the way toward
    // every bit and pays for that jitter; the counting one averages over its last few hundred.
    let n = 200_000;
    let fixed = apm_coded(&mut Apm::new(1), 7, n, 2048, |_| 4055);
    let counted = apm_coded(&mut Apm::new(1).with_counts(4, 255), 7, n, 2048, |_| 4055);
    assert!(counted * 100 < fixed * 99, "counting {counted} B vs fixed-rate {fixed} B");
}

#[test]
fn a_counting_apm_still_follows_a_source_that_changes() {
    // 0.95 for 100,000 bits, then 0.6: a cap of 255 updates keeps the knot able to move; an uncapped count would sit
    // on its old average.
    let n = 200_000;
    let rate = |i: usize| if i < 100_000 { 3891 } else { 2458 };
    let ideal = 100_000.0 * (h(3891.0 / 4096.0) + h(2458.0 / 4096.0)) / 8.0;
    let capped = apm_coded(&mut Apm::new(1).with_counts(4, 255), 9, n, 2048, rate);
    assert!((capped as f64) < ideal * 1.01, "capped at 255: {capped} B vs entropy {ideal:.0} B");
    let uncapped = apm_coded(&mut Apm::new(1).with_counts(4, u16::MAX), 9, n, 2048, rate);
    assert!((uncapped as f64) > ideal * 1.1, "uncapped: {uncapped} B vs entropy {ideal:.0} B");
}

/// One bit of the settling mixers' source: the truth's probability and seven noise probabilities.
fn truth_and_noise(rng: &mut Rng) -> (u16, [u16; 7]) {
    let truth = rng.p12();
    let mut noise = [0u16; 7];
    for v in noise.iter_mut() {
        *v = rng.p12();
    }
    (truth, noise)
}

/// The bytes `n` bits of the source cost a nine-input mixer (one weight set): input 0 is the truth's logit while
/// `trusty(i)` holds and input 1 is otherwise, the six after carry noise, the last is a bias. And what the bits cost
/// at the truth's own probabilities.
fn settling_run(seed: u64, n: usize, trusty: impl Fn(usize) -> bool, settle: Option<(u32, u32)>) -> (usize, f64) {
    let mut rng = Rng(seed);
    let mut mixer = Mixer2::new(9, &[1], 1).with_learning_rates(3, 2);
    if let Some((half, floor)) = settle {
        mixer = mixer.with_settling(half, floor);
    }
    let mut enc = Encoder::new();
    let mut ideal = 0.0f64;
    for i in 0..n {
        let (truth, noise) = truth_and_noise(&mut rng);
        let (a, b) = if trusty(i) { (truth, noise[0]) } else { (noise[0], truth) };
        mixer.set(0, stretch(a));
        mixer.set(1, stretch(b));
        for k in 1..7 {
            mixer.set(1 + k, stretch(noise[k]));
        }
        mixer.set(8, 256);
        let p = mixer.mix(&[0], 0);
        let bit = rng.bit(truth);
        let q = f64::from(truth) / 4096.0;
        ideal -= if bit { q } else { 1.0 - q }.log2();
        enc.encode(bit, p);
        mixer.update(bit);
    }
    (enc.finish().len(), ideal / 8.0)
}

#[test]
fn weight_sets_that_settle_waste_less_on_a_steady_source() {
    // The truth's input never changes: a set that keeps stepping at the full rate keeps its weights jittering around
    // where they belong; one that settles pays less over the entropy.
    let (plain, ideal) = settling_run(31, 400_000, |_| true, None);
    let (settled, _) = settling_run(31, 400_000, |_| true, Some((4096, 10)));
    let (excess_plain, excess_settled) = (plain as f64 - ideal, settled as f64 - ideal);
    assert!(excess_settled < excess_plain * 0.8, "settled {settled} B, full rate {plain} B, entropy {ideal:.0} B");
}

#[test]
fn a_settled_weight_set_still_learns_when_its_inputs_trade_places() {
    // Half way the truth moves from input 0 to input 1: a floor above zero lets the busy set relearn; with none it
    // would stay fixed on its first answer.
    let switch = |i: usize| i < 200_000;
    let (settled, ideal) = settling_run(33, 400_000, switch, Some((4096, 10)));
    assert!((settled as f64) < ideal * 1.01, "settled {settled} B vs entropy {ideal:.0} B");
    let (frozen, _) = settling_run(33, 400_000, switch, Some((4096, 0)));
    assert!((frozen as f64) > ideal * 1.1, "a zero floor {frozen} B vs entropy {ideal:.0} B");
}
