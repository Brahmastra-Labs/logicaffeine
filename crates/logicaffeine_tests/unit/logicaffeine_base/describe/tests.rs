use super::*;

fn roundtrips(v: &[i64]) {
    let enc = describe_int_seq(v);
    let dec = decode_int_seq(&enc);
    assert_eq!(
        dec.as_deref(),
        Some(v),
        "round-trip failed for {v:?} (enc {enc:?})"
    );
}

#[test]
fn affine_round_trips_and_beats_varint() {
    let v: Vec<i64> = (0..1000).map(|i| 10 + 7 * i).collect();
    roundtrips(&v);
    // The generator (3 numbers) must be far smaller than 1000 varints.
    assert!(
        describe_int_seq(&v).len() < 20,
        "affine must ship as a generator, not data"
    );
}

#[test]
fn geometric_round_trips() {
    let v: Vec<i64> = (0..40)
        .map(|i| 3i64.wrapping_mul(2i64.wrapping_pow(i)))
        .collect();
    roundtrips(&v);
}

#[test]
fn polynomial_round_trips() {
    let v: Vec<i64> = (0..500).map(|i| i * i - 3 * i + 5).collect();
    roundtrips(&v);
    assert!(
        describe_int_seq(&v).len() < 30,
        "poly must ship as finite-difference seeds"
    );
}

#[test]
fn linear_recurrence_ships_the_generator() {
    // Fibonacci — order-2 recurrence, NOT polynomial (its finite differences never settle), so
    // ONLY the recurrence detector catches it: 60 terms collapse to a handful of numbers.
    let mut fib = vec![0i64, 1];
    while fib.len() < 60 {
        let n = fib.len();
        fib.push(fib[n - 1].wrapping_add(fib[n - 2]));
    }
    roundtrips(&fib);
    assert!(
        describe_int_seq(&fib).len() < 15,
        "Fibonacci ships as (order, coeffs, seeds), not data"
    );

    // Lucas (same recurrence, different seeds).
    let mut lucas = vec![2i64, 1];
    while lucas.len() < 60 {
        let n = lucas.len();
        lucas.push(lucas[n - 1].wrapping_add(lucas[n - 2]));
    }
    roundtrips(&lucas);
    assert!(describe_int_seq(&lucas).len() < 15);

    // Pell: c = [2, 1].
    let mut pell = vec![0i64, 1];
    while pell.len() < 50 {
        let n = pell.len();
        pell.push(2i64.wrapping_mul(pell[n - 1]).wrapping_add(pell[n - 2]));
    }
    roundtrips(&pell);
    assert!(describe_int_seq(&pell).len() < 15);

    // A general order-3 recurrence v[i] = v[i-1] + v[i-2] − v[i-3].
    let mut r3 = vec![1i64, 2, 3];
    while r3.len() < 40 {
        let n = r3.len();
        r3.push(r3[n - 1].wrapping_add(r3[n - 2]).wrapping_sub(r3[n - 3]));
    }
    roundtrips(&r3);
    assert!(describe_int_seq(&r3).len() < 18);
}

#[test]
fn berlekamp_massey_recovers_the_shortest_lfsr() {
    // Connection polynomial 1 + x + x³ → taps [1,0,1] (s[i] = s[i-1] ⊕ s[i-3]); linear complexity 3.
    let taps = vec![true, false, true];
    let seed = vec![true, false, false];
    let seq = lfsr_generate(&taps, &seed, 40);
    let (l, recovered) = berlekamp_massey_gf2(&seq);
    assert_eq!(l, 3, "a length-3 LFSR has linear complexity 3");
    // The recovered LFSR regenerates the whole sequence from its first L bits (the attack).
    assert_eq!(lfsr_generate(&recovered, &seq[..l], seq.len()), seq);
    // A well-mixed random bit sequence has linear complexity ≈ n/2 — incompressible as an LFSR.
    // (splitmix64's nonlinear finalizer; a plain xorshift low bit is deliberately NOT used — BM
    // correctly reports its ~64 linear complexity, exposing that generator's linear weakness.)
    let mut st = 0x1234_5678u64;
    let rnd: Vec<bool> = (0..200)
        .map(|_| {
            st = st.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = st;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            z & 1 == 1
        })
        .collect();
    let (lr, _) = berlekamp_massey_gf2(&rnd);
    assert!(
        (80..=120).contains(&lr),
        "high-quality random complexity ≈ n/2, got {lr}"
    );
    // Bit ↔ byte conversions round-trip.
    let bytes = vec![0x41i64, 0x00, 0xFF, 0x7E];
    assert_eq!(bits_to_bytes(&bytes_to_bits(&bytes)), bytes);
}

#[test]
fn lfsr_keystream_bytes_compress_to_the_register() {
    // A maximal length-7 LFSR (x⁷+x³+1, bit-period 127) — its 200-byte keystream has byte-period
    // 127, beyond the periodic detector's n/2 reach, and looks random to every arithmetic generator.
    // Berlekamp–Massey recovers the 7-bit register and the whole column collapses to a handful of
    // bytes — the classic stream-cipher attack, here as a compressor.
    let taps = vec![false, false, true, false, false, false, true]; // c₃ = c₇ = 1
    let seed = vec![true, false, true, true, false, false, true];
    let bits = lfsr_generate(&taps, &seed, 200 * 8);
    let bytes = bits_to_bytes(&bits);
    roundtrips(&bytes);
    let enc = describe_int_seq(&bytes);
    assert_eq!(
        enc[0], T_INTS_LFSR,
        "the keystream ships as its LFSR register (tag), got {}",
        enc[0]
    );
    assert!(
        enc.len() < 20,
        "200 bytes collapse to the 7-bit register, got {} bytes",
        enc.len()
    );
}

#[test]
fn berlekamp_massey_over_gf256_recovers_a_word_lfsr() {
    // GF(2⁸) field sanity: 3⁵¹ ≠ 1 but 3²⁵⁵ = 1 (generator), and inv is a true inverse.
    assert_eq!(Gf256(0x53).mul(Gf256(0xCA)), Gf256(0x53).mul(Gf256(0xCA)));
    assert_eq!(Gf256(0x53).mul(Gf256(0x53).inv()), Gf256::one());
    assert_eq!(Gf256(0xFF).mul(Gf256(0xFF).inv()), Gf256::one());
    // An order-3 word-LFSR over GF(256): s[i] = c₁·s[i-1] + c₂·s[i-2] + c₃·s[i-3] with field
    // coefficients. Berlekamp–Massey over GF(256) recovers order 3 directly — exercising the
    // b_disc bookkeeping the GF(2) path never touches.
    let taps = vec![Gf256(0x02), Gf256(0x8d), Gf256(0x1f)];
    let seed = vec![Gf256(0x41), Gf256(0x9c), Gf256(0x07)];
    let seq = lfsr_generate_field(&taps, &seed, 60);
    let (l, recovered) = berlekamp_massey_field(&seq);
    assert_eq!(
        l, 3,
        "order-3 word-LFSR has GF(256) linear complexity 3, got {l}"
    );
    assert_eq!(
        lfsr_generate_field(&recovered, &seq[..l], seq.len()),
        seq,
        "recovered register regenerates it"
    );
}

#[test]
fn two_adic_complexity_detects_fcsr_keystreams() {
    use crate::numeric::BigInt;
    // FCSR: connection integer q = 19 (odd), numerator p = 3. Its 2-adic expansion is an FCSR
    // keystream — the carry makes it nonlinear over GF(2), so no field-BM sees its structure.
    let bits = fcsr_generate(&BigInt::from_i64(3), &BigInt::from_i64(19), 120);
    let (rp, rq) = two_adic_reconstruct(&bits).expect("a clean 2-adic rational");
    assert_eq!(
        fcsr_generate(&rp, &rq, bits.len()),
        bits,
        "the recovered FCSR regenerates the keystream"
    );
    let tac = two_adic_complexity(&bits);
    assert!(
        tac < 12,
        "a small FCSR has low 2-adic complexity, got {tac}"
    );
    // The carry makes it MORE complex to a LINEAR view than to a 2-adic one — BM sees more.
    assert!(
        berlekamp_massey_gf2(&bits).0 > tac,
        "the FCSR is simpler 2-adically than linearly"
    );
    // A well-mixed random sequence has 2-adic complexity ≈ n/2.
    let mut st = 0xABCD_1234u64;
    let rnd: Vec<bool> = (0..200)
        .map(|_| {
            st = st.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = st;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            z & 1 == 1
        })
        .collect();
    assert!(
        two_adic_complexity(&rnd) > 60,
        "random ≈ n/2 2-adic complexity, got {}",
        two_adic_complexity(&rnd)
    );
}

#[test]
fn fcsr_keystream_bytes_compress_to_the_rational() {
    use crate::numeric::BigInt;
    // A large connection integer q = 1000003: long period (not periodic), HIGH linear complexity
    // (fools the LFSR compressor), tiny 2-adic complexity. Only the FCSR generator collapses it.
    let bits = fcsr_generate(&BigInt::from_i64(7), &BigInt::from_i64(1_000_003), 100 * 8);
    let bytes = bits_to_bytes(&bits);
    roundtrips(&bytes);
    let enc = describe_int_seq(&bytes);
    assert_eq!(
        enc[0], T_INTS_FCSR,
        "the FCSR keystream ships as its rational p/q (tag), got {}",
        enc[0]
    );
    assert!(
        enc.len() < 20,
        "100 bytes collapse to a small rational, got {} bytes",
        enc.len()
    );
}

// ── Small-descriptor / huge-work DoS gate ──────────────────────────────────────────────────
// A generator column is a tiny header that expands to `n` (or `8·n`) elements, and FCSR/LFSR do
// bignum / `O(L)` work PER element — so the element-count budget cannot bound wall-time. A
// crafted large `n` must be refused in O(1) (clean `None`), never replayed. Regression for the
// never-panic fuzz: a random `WireValue` frame decoded a `T_INTS_FCSR` column with a
// multi-million byte count and spun the decoder for MINUTES (a full-suite 30-min timeout).
fn enc_uvarint(n: u64) -> Vec<u8> {
    let mut v = Vec::new();
    write_uvarint(n, &mut v);
    v
}

#[test]
fn a_giant_fcsr_keystream_is_refused_not_replayed() {
    // n = 10_000_000 bytes → 80_000_000 2-adic bignum steps. Under the 16M work budget this is
    // rejected in O(1); without the charge it spins for minutes (fcsr_generate loops 8·n times).
    let mut body = enc_uvarint(10_000_000);
    body.extend_from_slice(&[0, 1, 3]); // p = 3   (sign, len, LE byte)
    body.extend_from_slice(&[0, 1, 5]); // q = 5   (odd — a valid FCSR connection integer)
    let mut pos = 0;
    assert_eq!(
        decode_int_column_body(T_INTS_FCSR, &body, &mut pos, 1 << 24, 0),
        None,
        "a 10M-byte FCSR keystream must be refused fast, not replayed"
    );
    // A modest FCSR descriptor is far within budget and MUST still replay — the gate rejects
    // only the bomb, never a real keystream.
    let mut small = enc_uvarint(4);
    small.extend_from_slice(&[0, 1, 3]);
    small.extend_from_slice(&[0, 1, 5]);
    let mut sp = 0;
    assert!(
        decode_int_column_body(T_INTS_FCSR, &small, &mut sp, 1 << 24, 0).is_some(),
        "a 4-byte FCSR keystream is well within budget and must still decode"
    );
}

#[test]
fn a_giant_lfsr_keystream_is_refused_not_replayed() {
    // n = 10_000_000 bytes, L = 8 → 8·n·L register clocks. Refused in O(1) under the work budget;
    // without the charge lfsr_generate replays O(total_bits·L) and spins.
    let mut body = enc_uvarint(10_000_000); // byte count
    body.extend_from_slice(&enc_uvarint(8)); // L = 8
    body.push(0b1000_1001); // 8 tap bits
    body.push(0b0000_0001); // 8 seed bits
    let mut pos = 0;
    assert_eq!(
        decode_int_column_body(T_INTS_LFSR, &body, &mut pos, 1 << 24, 0),
        None,
        "a 10M-byte LFSR keystream must be refused fast, not replayed"
    );
}

#[test]
fn maximal_order_complexity_catches_nonlinear_feedback() {
    // A De Bruijn sequence of order L=6: generated by a NONLINEAR order-6 feedback (prefer-one),
    // full period 2⁶=64, every 6-window appearing exactly once. Its linear complexity is ~2⁵ and its
    // 2-adic complexity is high (both tools see it as complex), but its maximal order complexity is
    // exactly the register order 6 — the nonlinear rung sees the short register the linear rungs cannot.
    let order = 6;
    let period = 1usize << order;
    let mut db: Vec<bool> = vec![false; order];
    let mut seen: std::collections::HashSet<Vec<bool>> = std::collections::HashSet::new();
    seen.insert(db[..order].to_vec());
    while db.len() < period {
        let mut w: Vec<bool> = db[db.len() - (order - 1)..].to_vec();
        w.push(true);
        if seen.contains(&w) {
            w.pop();
            w.push(false);
        }
        seen.insert(w.clone());
        db.push(*w.last().unwrap());
    }
    let bits: Vec<bool> = [db.as_slice(), db.as_slice(), db.as_slice()].concat(); // 3 cyclic copies
    let moc = maximal_order_complexity(&bits);
    let lin = berlekamp_massey_gf2(&bits).0;
    assert!(
        (5..=7).contains(&moc),
        "the order-6 De Bruijn register has MOC ≈ 6, got {moc}"
    );
    // The SAME sequence fools the LINEAR tools — they see far higher complexity than its true order.
    assert!(
        lin > moc,
        "nonlinear feedback fools linear complexity (BM {lin} > MOC {moc})"
    );
    assert!(
        two_adic_complexity(&bits) > moc,
        "nonlinear feedback fools 2-adic complexity too"
    );
    // MOC ≤ linear complexity for EVERYTHING (it is the shorter, more general register).
    let mut st = 0x2468_ace0_1357_9bdfu64;
    let rnd: Vec<bool> = (0..300)
        .map(|_| {
            st = st.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = st;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            z & 1 == 1
        })
        .collect();
    assert!(
        maximal_order_complexity(&rnd) <= berlekamp_massey_gf2(&rnd).0,
        "MOC ≤ linear complexity"
    );
}

#[test]
fn algebraic_recurrence_recovers_low_degree_nonlinear_feedback() {
    // A QUADRATIC (degree-2) order-8 nonlinear feedback register:
    //   s[i] = s[i-1] ⊕ s[i-5] ⊕ s[i-6] ⊕ s[i-8] ⊕ (s[i-1] AND s[i-6]).
    // Its linear complexity is 246 (the linear rung is fooled — Berlekamp–Massey needs a 246-stage
    // LFSR), its 2-adic complexity is high, and maximal order complexity would diagnose "order 8"
    // only as a 2⁸=256-entry truth table. The ALGEBRAIC recovery returns the sparse degree-2 ANF
    // that regenerates it — 37 coefficients — compressing a nonlinear generator every earlier rung
    // can only measure.
    let seed: Vec<bool> = (0..8).map(|k| (0x9E37u64 >> k) & 1 == 1).collect();
    let mut bits = seed.clone();
    while bits.len() < 600 {
        let i = bits.len();
        let s = |k: usize| bits[i - k];
        bits.push(s(1) ^ s(5) ^ s(6) ^ s(8) ^ (s(1) & s(6)));
    }

    let coeffs = detect_algebraic_recurrence(&bits, 8, 2).expect("recovers the degree-2 feedback");
    assert_eq!(
        algebraic_generate(8, 2, &coeffs, &bits[..8], bits.len()),
        bits,
        "the recovered ANF regenerates the whole nonlinear sequence"
    );

    // The description is the sparse ANF: M = 1 + C(8,1) + C(8,2) = 37 monomials, versus the 2⁸ = 256
    // truth-table entries maximal order complexity would need for the same register.
    assert_eq!(
        coeffs.len(),
        37,
        "degree-2 ANF over 8 vars has 37 monomials"
    );
    assert!(
        coeffs.len() < (1usize << 8),
        "the ANF is far sparser than the full truth table"
    );

    // The quadratic term genuinely fools the LINEAR rung — Berlekamp–Massey reports complexity far
    // above the true register order — so nothing below this rung could have compressed it.
    let lin = berlekamp_massey_gf2(&bits).0;
    assert!(
        lin > 200,
        "the nonlinear term drives linear complexity to 246, got {lin}"
    );
    assert!(
        two_adic_complexity(&bits) > 8,
        "and 2-adic complexity is high too — carry tools miss it"
    );

    // A degree-1 (purely affine) fit CANNOT reproduce it — the compression is genuinely nonlinear.
    assert!(
        detect_algebraic_recurrence(&bits, 8, 1).is_none(),
        "no affine order-8 recurrence fits"
    );

    // `algebraic_complexity` finds the same short register at degree 2 with its true coefficient count.
    let (l, m) = algebraic_complexity(&bits, 2).expect("degree-2 algebraic complexity exists");
    assert!(
        l <= 8,
        "the algebraic recovery finds an order-≤8 register, got {l}"
    );
    assert!(
        m <= 37,
        "and describes it in at most its 37 ANF coefficients, got {m}"
    );
}

#[test]
fn algebraic_recovery_is_a_strict_generalization_of_berlekamp_massey() {
    // At degree 1 the algebraic recovery IS Berlekamp–Massey: a pure GF(2) LFSR keystream must be
    // recovered by the degree-1 solve and regenerate identically. (The affine ANF is not unique when
    // the constant column is dependent on the taps over the sample, so we assert regeneration — the
    // real correctness witness — not a specific coefficient vector.)
    let taps = vec![true, false, false, true, false, true]; // an order-6 LFSR
    let seed = vec![true, false, true, true, false, false];
    let bits = lfsr_generate(&taps, &seed, 200);
    let coeffs =
        detect_algebraic_recurrence(&bits, 6, 1).expect("degree-1 recovery = Berlekamp–Massey");
    assert_eq!(
        algebraic_generate(6, 1, &coeffs, &bits[..6], bits.len()),
        bits,
        "the degree-1 ANF regenerates the linear keystream"
    );
    // The linear rung already handles this one — Berlekamp–Massey finds the same short register.
    assert!(
        berlekamp_massey_gf2(&bits).0 <= 6,
        "an LFSR keystream has linear complexity ≤ its order"
    );
}

#[test]
fn correlation_attack_breaks_the_geffe_combiner_register_by_register() {
    // The Geffe generator: three LFSRs feed a multiplexer z[i] = x2[i] ? x1[i] : x3[i]. Its output
    // is a function of the HIDDEN register outputs, not of its own past — so every feedback-recovery
    // rung (linear, word, carry, maximal-order, algebraic) is blind to it. But Geffe leaks: z agrees
    // with x1 and with x3 three-quarters of the time (Pr[z=x1]=Pr[z=x3]=¾), while x2 is invisible
    // (Pr[z=x2]=½). The correlation attack recovers x1 and x3 ALONE — divide-and-conquer.
    let n = 2000;
    let taps1 = [false, false, true, false, false, false, true]; // L=7, period 127
    let taps2 = [false, false, true, false, true]; // L=5, period 31 (the protected middle)
    let taps3 = [false, false, false, false, true, false, false, false, true]; // L=9, period 511
    let seed1 = [true, false, true, true, false, false, true];
    let seed2 = [true, true, false, false, true];
    let seed3 = [true, false, false, true, false, true, true, false, true];
    let x1 = lfsr_generate(&taps1, &seed1, n);
    let x2 = lfsr_generate(&taps2, &seed2, n);
    let x3 = lfsr_generate(&taps3, &seed3, n);
    let z: Vec<bool> = (0..n).map(|i| if x2[i] { x1[i] } else { x3[i] }).collect();

    // Attack x1's register alone (2⁷ = 128 states): the correct initial state pops out at ¾ agreement.
    let a1 = correlation_attack(&z, &taps1).expect("x1 register attackable");
    assert_eq!(
        a1.init_state,
        seed1.to_vec(),
        "recovers x1's exact initial state"
    );
    assert!(
        (a1.agreement - 0.75).abs() < 0.05,
        "x1 leaks at ~¾ agreement, got {}",
        a1.agreement
    );
    assert!(
        a1.bias > 3.0 * spurious_bias_floor(7, n),
        "x1's correlation clears the spurious floor"
    );
    // The recovered state is a re-checkable witness — regenerate it and it IS x1.
    assert_eq!(
        lfsr_generate(&taps1, &a1.init_state, n),
        x1,
        "the witness regenerates the x1 register"
    );

    // Attack x3's register alone (2⁹ = 512 states) — independently of x1 and x2.
    let a3 = correlation_attack(&z, &taps3).expect("x3 register attackable");
    assert_eq!(
        a3.init_state,
        seed3.to_vec(),
        "recovers x3's exact initial state"
    );
    assert!(
        (a3.agreement - 0.75).abs() < 0.05,
        "x3 leaks at ~¾ agreement, got {}",
        a3.agreement
    );
    assert!(
        a3.bias > 3.0 * spurious_bias_floor(9, n),
        "x3's correlation clears the spurious floor"
    );
    assert_eq!(
        lfsr_generate(&taps3, &a3.init_state, n),
        x3,
        "the witness regenerates the x3 register"
    );

    // x2 is the CORRELATION-IMMUNE middle: its best state sits at the spurious floor, no real leak.
    let a2 = correlation_attack(&z, &taps2).expect("x2 register scanned");
    assert!(
        a2.bias < 3.0 * spurious_bias_floor(5, n),
        "x2 does not measurably leak, bias {}",
        a2.bias
    );
    assert!(
        a1.bias > 3.0 * a2.bias,
        "the leaking registers are starkly separated from the protected one"
    );
}

#[test]
fn walsh_spectrum_sees_the_linear_approximation_correlation_is_blind_to() {
    // f(x1,x2,x3,x4) = x1 ⊕ x2 ⊕ (x3 ∧ x4). It is FIRST-ORDER correlation-immune: every weight-1
    // Walsh coefficient vanishes, so Rung E's single-input attack sees Pr[z=xⱼ]=½ and finds nothing.
    // But the full Walsh spectrum reveals a WEIGHT-2 approximation z ≈ x1⊕x2 with bias ¼ — the
    // multi-register leak E structurally cannot express (it only reads weight-1 masks).
    let f: Vec<bool> = (0..16)
        .map(|x| {
            let b = |i: usize| (x >> i) & 1 == 1;
            b(0) ^ b(1) ^ (b(2) & b(3))
        })
        .collect();
    let spec = walsh_spectrum(&f).expect("well-formed truth table");
    for m in [1usize, 2, 4, 8] {
        assert_eq!(
            spec[m], 0,
            "weight-1 mask {m} vanishes — Rung E is blind here"
        );
    }
    assert_eq!(
        correlation_immunity_order(&f),
        Some(1),
        "f is first-order correlation-immune"
    );
    let (mask, bias) = best_linear_approximation(&f, true).expect("a best approximation exists");
    assert_eq!(bias, 0.25, "the exploitable approximation has bias ¼");
    assert!(
        mask.count_ones() >= 2,
        "and it lives at weight ≥ 2 — beyond E's single-register view"
    );
    assert_eq!(nonlinearity(&f), Some(4), "nonlinearity 2³ − 8/2 = 4");

    // THE CEILING — a bent function g = x1x2 ⊕ x3x4 has a FLAT spectrum: every |Ŵ| = 2^{n/2} = 4, so
    // no linear approximation beats bias 1/8. Maximal nonlinearity, the linear-incompressible residue.
    let g: Vec<bool> = (0..16)
        .map(|x| {
            let b = |i: usize| (x >> i) & 1 == 1;
            (b(0) & b(1)) ^ (b(2) & b(3))
        })
        .collect();
    let spec_g = walsh_spectrum(&g).expect("well-formed");
    assert!(
        spec_g.iter().all(|&c| c.unsigned_abs() == 4),
        "bent ⇒ perfectly flat spectrum"
    );
    assert_eq!(
        nonlinearity(&g),
        Some(6),
        "bent nonlinearity 2³ − 2¹ = 6 (maximal for n=4)"
    );
    assert_eq!(
        best_linear_approximation(&g, true).unwrap().1,
        0.125,
        "no approximation beats 1/8"
    );
}

#[test]
fn anf_is_its_own_inverse_and_reads_the_degree() {
    // The Möbius transform is an involution over GF(2): anf(anf(f)) == f, for every function.
    for n in 1..=5usize {
        for seed in 0..8u64 {
            let f: Vec<bool> = (0..1u64 << n)
                .map(|i| {
                    let mut z =
                        0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i.wrapping_add(seed * 131 + 1));
                    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                    (z ^ (z >> 27)) & 1 == 1
                })
                .collect();
            let a = anf(&f).expect("well-formed");
            assert_eq!(
                anf(&a).as_deref(),
                Some(&f[..]),
                "anf∘anf = identity (n={n}, seed={seed})"
            );
        }
    }
    // Known ANFs: constant (degree 0), parity (degree 1), the inner-product bent (degree 2).
    let n = 4;
    let parity: Vec<bool> = (0..1usize << n)
        .map(|x| (x as u32).count_ones() % 2 == 1)
        .collect();
    assert_eq!(algebraic_degree(&parity), Some(1), "parity is affine");
    let bent: Vec<bool> = (0..1usize << n)
        .map(|x| {
            let b = |i: usize| x & (1 << i) != 0;
            (b(0) && b(1)) ^ (b(2) && b(3))
        })
        .collect();
    assert_eq!(algebraic_degree(&bent), Some(2), "x0x1 ⊕ x2x3 is quadratic");
    assert_eq!(
        algebraic_degree(&vec![true; 8]),
        Some(0),
        "a constant has degree 0"
    );
}

#[test]
fn autocorrelation_reads_the_derivative_symmetry() {
    let n = 4;
    // A linear function has EVERY direction as a linear structure: |r(a)| = 2ⁿ for all a.
    let parity: Vec<bool> = (0..1usize << n)
        .map(|x| (x as u32).count_ones() % 2 == 1)
        .collect();
    let rp = autocorrelation(&parity).expect("well-formed");
    assert_eq!(rp[0], 16, "r(0) = 2ⁿ always");
    assert!(
        rp.iter().all(|&c| c.unsigned_abs() == 16),
        "a linear function is flat: |r(a)| = 2ⁿ ∀a"
    );

    // A bent function is PERFECTLY nonlinear: r(a) = 0 for every a ≠ 0 — no linear structure at all.
    let bent: Vec<bool> = (0..1usize << n)
        .map(|x| {
            let b = |i: usize| x & (1 << i) != 0;
            (b(0) && b(1)) ^ (b(2) && b(3))
        })
        .collect();
    let rb = autocorrelation(&bent).expect("well-formed");
    assert_eq!(rb[0], 16);
    assert!(
        rb[1..].iter().all(|&c| c == 0),
        "bent ⇒ no nonzero linear structure"
    );

    // f(x) = g(x0,x1,x2) ⊕ x3 has the complement structure a = e3: flipping x3 always flips f.
    let f: Vec<bool> = (0..1usize << n)
        .map(|x| {
            let b = |i: usize| x & (1 << i) != 0;
            ((b(0) && b(1)) ^ b(2)) ^ b(3)
        })
        .collect();
    let rf = autocorrelation(&f).expect("well-formed");
    assert_eq!(
        rf[1 << 3],
        -16,
        "flipping x3 always flips f ⇒ r(e3) = −2ⁿ (a linear structure)"
    );
}

#[test]
fn walsh_found_approximation_breaks_a_correlation_immune_combiner() {
    // Four LFSRs feed the CI(1) combiner z = a1 ⊕ a2 ⊕ (a3 ∧ a4). Because it is first-order
    // correlation-immune, Rung E sees each single register at ½ — invisible. The Walsh-found weight-2
    // approximation z ≈ a1⊕a2 is the break: the COMBINED register leaks at ¾.
    let n = 4000;
    let taps1 = [false, false, true, false, false, false, true]; // L=7
    let taps2 = [false, false, true, false, true]; // L=5
    let taps3 = [false, false, false, false, true, false, false, false, true]; // L=9
    let taps4 = [
        false, false, false, false, false, false, false, false, true, false, true,
    ]; // L=11
    let a1 = lfsr_generate(&taps1, &[true, false, true, true, false, false, true], n);
    let a2 = lfsr_generate(&taps2, &[true, true, false, false, true], n);
    let a3 = lfsr_generate(
        &taps3,
        &[true, false, false, true, false, true, true, false, true],
        n,
    );
    let a4 = lfsr_generate(
        &taps4,
        &[
            true, false, true, false, false, true, true, false, false, true, false,
        ],
        n,
    );
    let z: Vec<bool> = (0..n).map(|i| a1[i] ^ a2[i] ^ (a3[i] & a4[i])).collect();

    // E's view — the keystream against each single register: flat, no leak (correlation-immune).
    let agree1 = z.iter().zip(&a1).filter(|(x, y)| x == y).count() as f64 / n as f64;
    assert!(
        (agree1 - 0.5).abs() < 0.04,
        "single register a1 is invisible to first-order correlation, {agree1}"
    );

    // F's view — the keystream against the COMBINED register a1⊕a2 (the weight-2 mask Walsh found).
    let combined: Vec<bool> = (0..n).map(|i| a1[i] ^ a2[i]).collect();
    let agree_r = z.iter().zip(&combined).filter(|(x, y)| x == y).count() as f64 / n as f64;
    assert!(
        (agree_r - 0.75).abs() < 0.04,
        "the weight-2 approximation leaks at ¾ — E could not see it, {agree_r}"
    );
}

#[test]
fn fast_correlation_attack_decodes_a_noisy_lfsr_without_exhaustive_search() {
    // Primitive trinomial x¹⁷+x³+1: a[i] = a[i-14] ⊕ a[i-17], weight-3 parity checks — ideal for fast
    // correlation. Rung E would search 2¹⁷ states; the decoder recovers the state in polynomial time.
    let mut taps = vec![false; 17];
    taps[13] = true; // a[i-14]
    taps[16] = true; // a[i-17]
    let seed: Vec<bool> = (0..17).map(|k| (0xACE1u64 >> k) & 1 == 1).collect();
    let n = 4000;
    let a = lfsr_generate(&taps, &seed, n);

    // Transmit through a binary symmetric channel: flip ~12% of the bits.
    let mut st = 0x1234_5678_9abc_def0u64;
    let z: Vec<bool> = a
        .iter()
        .map(|&bit| {
            st = st
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            bit ^ ((st >> 40) % 100 < 12)
        })
        .collect();
    let agree = a.iter().zip(&z).filter(|(x, y)| x == y).count() as f64 / n as f64;
    assert!(
        (agree - 0.88).abs() < 0.04,
        "the channel is genuinely ~12% noisy, got {agree}"
    );

    let state = fast_correlation_attack(&z, &taps, 400).expect("the decoder recovers the register");
    assert_eq!(
        state, seed,
        "fast correlation recovers the exact initial state — no 2¹⁷ search"
    );

    // Ceiling: pure noise (no correlation) decodes to nothing significant.
    let mut st2 = 0xdead_beef_0bad_c0deu64;
    let pure: Vec<bool> = (0..n)
        .map(|_| {
            st2 = st2.wrapping_mul(6364136223846793005).wrapping_add(1);
            (st2 >> 40) & 1 == 1
        })
        .collect();
    assert!(
        fast_correlation_attack(&pure, &taps, 400).is_none(),
        "pure noise leaks no register — the ceiling"
    );
}

#[test]
fn shrinking_generator_falls_to_a_clock_guess_the_linear_rungs_cannot_touch() {
    // Clock register A (L=7) gates data register S (L=9). The output is a data-dependent decimation
    // of S — not a fixed function of any register's own past — so its linear complexity is enormous
    // and every feedback/correlation/algebraic/linear rung is blind. The attack guesses A's 2⁷ states
    // (divide-and-conquer on the CLOCK alone) and linear-solves S.
    let a_taps = [false, false, true, false, false, false, true]; // L=7
    let s_taps = [false, false, false, false, true, false, false, false, true]; // L=9
    let a_seed = [true, false, true, true, false, false, true];
    let s_seed = [true, true, false, true, false, true, true, false, true];
    let m = 300;
    let output = shrinking_generator(&a_taps, &a_seed, &s_taps, &s_seed, m);

    // The linear rung is blind: the output's linear complexity is far above the data register order.
    assert!(
        berlekamp_massey_gf2(&output).0 > 40,
        "the shrinking generator's output has high linear complexity, got {}",
        berlekamp_massey_gf2(&output).0
    );

    let (a_rec, s_rec) =
        attack_shrinking_generator(&output, &a_taps, &s_taps).expect("the generator falls");
    // The shrinking generator has key-equivalences (distinct register states yielding the same
    // keystream), so the break is keystream-equivalence, not literal state equality: the recovered
    // pair reproduces the output — and keeps reproducing it far past the 300 bits the attack used
    // (1000 bits), proving a full recovery of the generator, not a short coincidence.
    assert_eq!(
        shrinking_generator(&a_taps, &a_rec, &s_taps, &s_rec, m),
        output,
        "the recovered pair regenerates the keystream — the certified break"
    );
    assert_eq!(
        shrinking_generator(&a_taps, &a_rec, &s_taps, &s_rec, 1000),
        shrinking_generator(&a_taps, &a_seed, &s_taps, &s_seed, 1000),
        "and stays identical to the true generator well past the attacked length — a full break"
    );
}

#[test]
fn xl_solves_a_quadratic_system_by_degree_escalation() {
    // A quadratic system over GF(2) in x0..x3 with the unique solution [1,0,1,0]. Each equation is an
    // XOR of monomial bitmasks that must be 0 (bit i = variable i; mask 0 = the constant 1).
    let eqs: Vec<Vec<u64>> = vec![
        vec![0b0011],         // x0·x1 = 0
        vec![0b0101, 0b0000], // x0·x2 = 1
        vec![0b0110],         // x1·x2 = 0
        vec![0b1100],         // x2·x3 = 0
        vec![0b0001, 0b0100], // x0 ⊕ x2 = 0
        vec![0b0010, 0b1000], // x1 ⊕ x3 = 0
    ];
    // At operating degree 1 the quadratic constraints cannot even be represented — only the two
    // linear relations survive, and the system is underdetermined.
    assert_eq!(
        solve_polynomial_system_gf2(&eqs, 4, 1),
        PolySolveResult::Undetermined,
        "degree 1 cannot represent the quadratic constraints"
    );
    // XL at degree 2 multiplies the equations, linking the products back to the variables, and the
    // unique root falls out — verified against the original nonlinear system.
    assert_eq!(
        solve_polynomial_system_gf2(&eqs, 4, 2),
        PolySolveResult::Solved(vec![true, false, true, false]),
        "XL recovers the unique root"
    );
}

#[test]
fn xl_refutes_an_unsatisfiable_system_linearization_misses() {
    // x0·x1 = 1 and x0 = 0 is contradictory (x0=0 forces x0·x1=0≠1), but a degree-1 linearization
    // cannot even represent x0·x1. XL multiplies x0=0 by x1 to derive x0·x1=0, colliding with
    // x0·x1=1 → 0=1, a certified refutation.
    // Over 2 variables (x0 = bit 0, x1 = bit 1), x0·x1 has mask 0b11.
    let eqs: Vec<Vec<u64>> = vec![
        vec![0b11, 0b00], // x0·x1 ⊕ 1 = 0
        vec![0b01],       // x0 = 0
    ];
    assert_eq!(
        solve_polynomial_system_gf2(&eqs, 2, 1),
        PolySolveResult::Undetermined,
        "at degree 1 the quadratic contradiction is invisible"
    );
    assert_eq!(
        solve_polynomial_system_gf2(&eqs, 2, 2),
        PolySolveResult::Refuted,
        "XL manufactures x0·x1 = 0, refuting the system"
    );
}

#[test]
fn algebraic_immunity_computes_and_verifies_known_values() {
    let b = |x: usize, i: usize| (x >> i) & 1 == 1;

    // An affine function has algebraic immunity 1: g = C ⊕ 1 annihilates it (degree 1).
    let affine: Vec<bool> = (0..8).map(|x| b(x, 0) ^ b(x, 1) ^ b(x, 2)).collect();
    let (ai, w) = algebraic_immunity(&affine).expect("well-formed");
    assert_eq!(ai, 1, "affine functions have AI 1");
    assert!(
        verify_annihilator(&affine, &w),
        "the affine annihilator re-checks"
    );

    // Majority-of-3 has algebraic immunity 2 — no affine annihilator exists on either side.
    let maj3: Vec<bool> = (0..8).map(|x| (x as u32).count_ones() >= 2).collect();
    let (ai, w) = algebraic_immunity(&maj3).expect("well-formed");
    assert_eq!(ai, 2, "majority-3 has AI 2 (the maximum for n=3)");
    assert!(
        verify_annihilator(&maj3, &w),
        "the degree-2 annihilator re-checks"
    );

    // A product term x1∧x2∧x3 has AI 1 (x1 ⊕ 1 vanishes on its lone support point 111).
    let and3: Vec<bool> = (0..8).map(|x| b(x, 0) & b(x, 1) & b(x, 2)).collect();
    assert_eq!(
        algebraic_immunity(&and3).unwrap().0,
        1,
        "a single product term has AI 1"
    );

    // Tamper cases: an all-zero witness is not an annihilator, and flipping the side breaks vanishing.
    let (_, w) = algebraic_immunity(&maj3).unwrap();
    let mut zeroed = w.clone();
    zeroed.coeffs = vec![false; zeroed.coeffs.len()];
    assert!(
        !verify_annihilator(&maj3, &zeroed),
        "the zero function is not a valid annihilator"
    );
    let mut flipped = w.clone();
    flipped.annihilates_complement = !flipped.annihilates_complement;
    assert!(
        !verify_annihilator(&maj3, &flipped),
        "a nonzero g cannot vanish on both sides"
    );
}

#[test]
fn algebraic_immunity_never_exceeds_half_n() {
    // AI(C) ≤ ⌈n/2⌉ for every Boolean function — a theorem; re-checked here on pseudo-random tables.
    let mut st = 0x00C0_FFEE_1234_5678u64;
    for _ in 0..24 {
        let truth: Vec<bool> = (0..16)
            .map(|_| {
                st = st
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (st >> 33) & 1 == 1
            })
            .collect();
        let (ai, w) = algebraic_immunity(&truth).expect("well-formed");
        assert!(ai <= 2, "AI ≤ ⌈4/2⌉ = 2, got {ai}");
        assert!(verify_annihilator(&truth, &w), "every witness re-checks");
    }
}

#[test]
fn algebraic_filter_attack_recovers_the_filter_generator_state() {
    // A filter generator: a length-10 maximal LFSR (x¹⁰+x³+1, period 1023) filtered by majority-of-3
    // over three consecutive state bits. Correlation/Walsh see only statistical leaks; the algebraic
    // attack is exact — maj3 has AI 2, so each keystream bit is a degree-2 equation in the initial
    // state, and linearizing over the 56 monomials recovers all 10 secret bits at once.
    let taps = [
        false, false, false, false, false, false, true, false, false, true,
    ]; // s[i]=s[i-7]⊕s[i-10]
    let s0 = [
        true, false, true, true, false, false, true, false, true, true,
    ];
    let filter_truth: Vec<bool> = (0..8).map(|x| (x as u32).count_ones() >= 2).collect(); // maj3
    let m = 3;
    let n = 400;
    let seq = lfsr_generate(&taps, &s0, n + m);
    let keystream: Vec<bool> = (0..n)
        .map(|t| {
            let idx = (0..m).fold(0usize, |a, i| a | (usize::from(seq[t + i]) << i));
            filter_truth[idx]
        })
        .collect();

    let recovered = algebraic_filter_attack(&keystream, &taps, &filter_truth)
        .expect("the algebraic attack recovers the state");
    assert_eq!(
        recovered,
        s0.to_vec(),
        "the recovered initial state IS the secret key"
    );
}

#[test]
fn periodic_round_trips() {
    let block = [4i64, 1, 1, 5, 9, 2, 6];
    let v: Vec<i64> = (0..300).map(|i| block[i % block.len()]).collect();
    roundtrips(&v);
}

#[test]
fn modular_affine_round_trips() {
    let v: Vec<i64> = (0..200).map(|i| 100 + 3 * ((i % 8) as i64)).collect();
    roundtrips(&v);
}

#[test]
fn sparse_round_trips() {
    let mut v = vec![42i64; 500];
    v[17] = -1;
    v[300] = 999;
    v[499] = 7;
    roundtrips(&v);
}

#[test]
fn delta_and_dod_shapes_round_trip() {
    let monotone: Vec<i64> = (0..200).map(|i| i * i / 7 + i).collect();
    roundtrips(&monotone);
    let jittered: Vec<i64> = (0..200).map(|i| 1_000_000 + 60 * i + (i % 3)).collect();
    roundtrips(&jittered);
}

#[test]
fn runs_and_low_cardinality_round_trip() {
    let mut runs = vec![5i64; 100];
    runs.extend(std::iter::repeat(9).take(80));
    runs.extend(std::iter::repeat(-2).take(120));
    roundtrips(&runs);
    let categorical: Vec<i64> = (0..600).map(|i| [10, 20, 30][i % 3]).collect();
    roundtrips(&categorical);
}

#[test]
fn byte_column_round_trips() {
    let v: Vec<i64> = (0..500).map(|i| ((i * 37) % 256) as i64).collect();
    roundtrips(&v);
}

#[test]
fn edge_cases_round_trip() {
    roundtrips(&[]);
    roundtrips(&[42]);
    roundtrips(&[-1]);
    roundtrips(&[i64::MIN, i64::MAX, 0, -1, 1]);
    roundtrips(&[7, 7, 7, 7]);
}

#[test]
fn pseudorandom_round_trips_and_is_never_larger_than_varint() {
    // A pseeded pseudo-random column has no generator structure; the varint baseline must win
    // and the round-trip must still be exact.
    let mut state = 0x1234_5678_9abc_def0u64;
    let v: Vec<i64> = (0..400)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 1) as i64
        })
        .collect();
    roundtrips(&v);
    let mut baseline = vec![T_INTS];
    leb128_encode(&mut baseline, v.iter().copied(), v.len());
    assert!(
        describe_int_seq(&v).len() <= baseline.len(),
        "never worse than the varint baseline"
    );
}
