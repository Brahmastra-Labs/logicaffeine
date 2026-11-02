use super::super::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
use super::*;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

fn silicon_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// SINGLE-ERROR CORRECTION, exhaustive: for a spread of data words, flipping ANY one
/// of the 72 codeword bits is corrected and the original 64 data bits recovered.
#[test]
fn every_single_bit_error_is_corrected() {
    let words = [
        0u64,
        u64::MAX,
        0xDEAD_BEEF_CAFE_F00D,
        0x5555_5555_5555_5555,
        0x0123_4567_89AB_CDEF,
        1,
        1 << 63,
    ];
    for &d in &words {
        let cw = encode(d);
        // No fault → clean.
        let (dd, v) = decode(cw);
        assert_eq!((dd, v), (d, Decode::NoError), "clean decode for {d:#x}");
        for bit in 0..CW_BITS {
            let (rec, verdict) = decode(cw ^ (1u128 << bit));
            assert!(
                matches!(verdict, Decode::Corrected(_) | Decode::CorrectedParityBit),
                "bit {bit} flip on {d:#x} must be a correctable single error, got {verdict:?}"
            );
            assert_eq!(rec, d, "single error at bit {bit} must recover data {d:#x}");
        }
    }
}

/// DOUBLE-ERROR DETECTION, swept: flipping any two distinct codeword bits is
/// flagged `DoubleError` (never silently mis-corrected).
#[test]
fn double_bit_errors_are_detected() {
    let d = 0xA5A5_5A5A_1234_9E37u64;
    let cw = encode(d);
    let mut checked = 0;
    for a in 0..CW_BITS {
        for b in (a + 1)..CW_BITS {
            let (_, verdict) = decode(cw ^ (1u128 << a) ^ (1u128 << b));
            assert_eq!(
                verdict,
                Decode::DoubleError,
                "double error at bits {a},{b} must be DETECTED, got {verdict:?}"
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked,
        CW_BITS * (CW_BITS - 1) / 2,
        "swept all C(72,2) pairs"
    );
}

/// The single-error-correction PROPERTY proven by our certified SAT on a tractable
/// Hamming(7,4) instance (4 data + 3 check): for every data word and every
/// single-bit flip position, the recomputed syndrome EQUALS the flipped position —
/// so the corrector (which flips the syndrome-indexed bit) always restores the
/// codeword. Proven by showing the negation is UNSAT over all 4 data bits.
#[test]
fn hamming_syndrome_points_at_the_error_proven_by_our_sat() {
    use BitVecBoundedOp::*;
    // Hamming(7,4): positions 1..7, check bits at 1,2,4; data at 3,5,6,7.
    // We prove: for a codeword with a single flip at a chosen position `e`, the
    // 3-bit syndrome (recomputed) == e. We encode the codeword symbolically from 4
    // data vars, flip position `e` (a constant per obligation), and assert the
    // syndrome equals e — negation UNSAT for every e in 1..=7.
    const W: u32 = 1;
    let bit = |n: String| Box::new(BoundedExpr::BitVecVar(n, W));
    let xor = |l: Box<BoundedExpr>, r: Box<BoundedExpr>| {
        Box::new(BoundedExpr::BitVecBinary {
            op: Xor,
            left: l,
            right: r,
        })
    };
    let k = |v: u64| Box::new(BoundedExpr::BitVecConst { width: W, value: v });
    let lower = |e: BoundedExpr| super::super::bitblast::lower_bool(&e).expect("lowers");

    // Codeword positions 1..7. Data at 3,5,6,7 = d0..d3; check at 1,2,4.
    // check[1] covers positions {1,3,5,7}; check[2] covers {2,3,6,7}; check[4] covers {4,5,6,7}.
    let d = |j: usize| bit(format!("d{j}"));
    let pos = |p: usize| -> Box<BoundedExpr> {
        match p {
            3 => d(0),
            5 => d(1),
            6 => d(2),
            7 => d(3),
            1 => xor(xor(d(0), d(1)), d(3)), // c1 = p3^p5^p7
            2 => xor(xor(d(0), d(2)), d(3)), // c2 = p3^p6^p7
            4 => xor(xor(d(1), d(2)), d(3)), // c4 = p5^p6^p7
            _ => unreachable!(),
        }
    };
    for e in 1..=7usize {
        // Flip position e: q_p = pos(p) ^ (p==e).
        let q = |p: usize| -> Box<BoundedExpr> {
            if p == e {
                xor(pos(p), k(1))
            } else {
                pos(p)
            }
        };
        // Recomputed syndrome bits over the flipped codeword.
        let s1 = xor(xor(xor(q(1), q(3)), q(5)), q(7));
        let s2 = xor(xor(xor(q(2), q(3)), q(6)), q(7));
        let s4 = xor(xor(xor(q(4), q(5)), q(6)), q(7));
        // Claim: (s4,s2,s1) as a 3-bit number == e. Assert each syndrome bit equals
        // the corresponding bit of e; the conjunction's negation must be UNSAT.
        let want = |b: usize| k(((e >> b) & 1) as u64);
        let eq = |lhs: Box<BoundedExpr>, rhs: Box<BoundedExpr>| lower(BoundedExpr::Eq(lhs, rhs));
        let all = ProofExpr::And(
            Box::new(ProofExpr::And(
                Box::new(eq(s1, want(0))),
                Box::new(eq(s2, want(1))),
            )),
            Box::new(eq(s4, want(2))),
        );
        assert_eq!(
            prove_unsat(&ProofExpr::Not(Box::new(all))),
            UnsatOutcome::Refuted,
            "syndrome must point at the flipped position e={e} (single-error correction)"
        );
    }
}

/// The RTL encoder/decoder + a self-checking testbench + oracle fault vectors are
/// emitted (golden; regenerate with SECDED_RECORD=1). iverilog PASS is wired into
/// scripts/synth-soft-core.sh.
#[test]
fn secded_artifacts_are_current() {
    // Build fault vectors: several data words × {no fault, each single flip (sample),
    // several double flips}. The oracle computes the expected verdict/data.
    let words = [0xDEAD_BEEF_CAFE_F00Du64, 0x0123_4567_89AB_CDEF, u64::MAX, 0];
    let mut rows = String::new();
    let mut count = 0usize;
    let emit = |rows: &mut String, count: &mut usize, d: u64, mask: u128| {
        let (dout, verdict) = decode(encode(d) ^ mask);
        let (corr, dbl) = match verdict {
            Decode::NoError => (0, 0),
            Decode::Corrected(_) | Decode::CorrectedParityBit => (1, 0),
            Decode::DoubleError => (0, 1),
        };
        rows.push_str(&format!(
            "{:016x} {:018x} {:016x} {} {}\n",
            d, mask, dout, corr, dbl
        ));
        *count += 1;
    };
    for &d in &words {
        emit(&mut rows, &mut count, d, 0); // clean
        for bit in [0usize, 1, 35, 63, 64, 71] {
            emit(&mut rows, &mut count, d, 1u128 << bit); // single
        }
        for (a, b) in [(0usize, 1usize), (10, 40), (63, 71), (5, 5 + 1)] {
            emit(&mut rows, &mut count, d, (1u128 << a) ^ (1u128 << b)); // double
        }
    }
    let files = [
        ("secded.v", {
            let mut s = emit_secded_verilog();
            s.push('\n');
            s.push_str(&emit_double_buffer_verilog());
            s
        }),
        ("tb_secded.v", emit_secded_testbench()),
        ("secded_vec.hex", rows),
    ];
    let dir = silicon_dir();
    if std::env::var("SECDED_RECORD").is_ok() {
        for (nm, b) in &files {
            std::fs::write(dir.join(nm), b).expect("write");
        }
        return;
    }
    for (nm, b) in &files {
        let got = std::fs::read_to_string(dir.join(nm)).unwrap_or_else(|e| {
            panic!("missing {nm} ({e}) — record: SECDED_RECORD=1 cargo test secded_artifacts")
        });
        assert_eq!(&got, b, "{nm} is stale — regenerate with SECDED_RECORD=1");
    }
}

/// The double-buffer safety invariant holds by construction: read and fill banks
/// are always complements (compute never reads the bank being DMA-filled).
#[test]
fn double_buffer_banks_are_always_disjoint() {
    let v = emit_double_buffer_verilog();
    assert!(
        v.contains("assign fill_bank = ~read_bank;"),
        "fill = ~read enforces disjointness"
    );
}
