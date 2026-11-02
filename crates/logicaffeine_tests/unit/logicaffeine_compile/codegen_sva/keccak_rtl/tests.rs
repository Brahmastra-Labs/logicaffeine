use super::super::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
use super::*;
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

fn silicon_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// The flagship: the RTL core, its testbench, and a real oracle vector are emitted
/// (golden; regenerate with KECCAK_RECORD=1). The iverilog PASS is wired into
/// scripts/synth-soft-core.sh and proven bit-exact against the native permutation.
#[test]
fn keccak_artifacts_are_current_and_vectors_match_the_oracle() {
    // A deterministic input state (seeded LCG — no clocks/rand).
    let mut x = 0xA5A5_5A5A_1234_9E37u64;
    let mut input = [0u64; 25];
    for lane in input.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *lane = x;
    }
    // Oracle: the real native permutation.
    let mut st = input;
    logicaffeine_system::keccak::keccak_f1600(&mut st);

    let mut hex = String::new();
    for lane in input.iter() {
        hex.push_str(&format!("{lane:016x}\n"));
    }
    for lane in st.iter() {
        hex.push_str(&format!("{lane:016x}\n"));
    }
    let files = [
        ("keccak_round.v", emit_keccak_round_verilog()),
        ("keccak_core.v", emit_keccak_core_verilog()),
        ("keccak_top.v", emit_keccak_top_verilog()),
        ("tb_keccak_core.v", emit_keccak_testbench()),
        ("keccak_vec.hex", hex),
    ];
    let dir = silicon_dir();
    if std::env::var("KECCAK_RECORD").is_ok() {
        for (n, b) in &files {
            std::fs::write(dir.join(n), b).expect("write");
        }
        return;
    }
    for (n, b) in &files {
        let got = std::fs::read_to_string(dir.join(n)).unwrap_or_else(|e| {
            panic!("missing {n} ({e}) — record: KECCAK_RECORD=1 cargo test keccak_artifacts")
        });
        assert_eq!(&got, b, "{n} is stale — regenerate with KECCAK_RECORD=1");
    }
}

/// θ is GF(2)-linear — proven by our certified SAT on reduced 2-bit lanes (the
/// rotate-by-1 in θ's `d` stays non-trivial): θ(x⊕y) == θ(x) ⊕ θ(y) for all x,y.
/// θ is XOR-only, so this is exact at any width; 2-bit lanes keep the SAT small.
#[test]
fn keccak_theta_is_gf2_linear_proven_by_our_sat() {
    use BitVecBoundedOp::*;
    const LW: u32 = 2; // lane width
    let bv = |n: String| Box::new(BoundedExpr::BitVecVar(n, LW));
    let xor = |l: Box<BoundedExpr>, r: Box<BoundedExpr>| {
        Box::new(BoundedExpr::BitVecBinary {
            op: Xor,
            left: l,
            right: r,
        })
    };
    // rotl by 1 within LW bits: (v<<1)|(v>>(LW-1)).
    let rotl1 = |v: Box<BoundedExpr>| {
        Box::new(BoundedExpr::BitVecBinary {
            op: Or,
            left: Box::new(BoundedExpr::BitVecBinary {
                op: Shl,
                left: v.clone(),
                right: Box::new(BoundedExpr::BitVecConst {
                    width: LW,
                    value: 1,
                }),
            }),
            right: Box::new(BoundedExpr::BitVecBinary {
                op: Shr,
                left: v.clone(),
                right: Box::new(BoundedExpr::BitVecConst {
                    width: LW,
                    value: (LW - 1) as u64,
                }),
            }),
        })
    };
    // θ on lane i (var name prefix p): out_i = a_i ⊕ d_(i%5), d_col = c_(col+4)%5 ⊕ rotl1(c_(col+1)%5),
    // c_col = ⊕ of the 5 lanes in that column. Build symbolically for one prefix.
    let theta = |prefix: &str, i: usize| -> Box<BoundedExpr> {
        let a = |j: usize| bv(format!("{prefix}a{j}"));
        let col = |cc: usize| {
            xor(
                xor(xor(a(cc), a(cc + 5)), xor(a(cc + 10), a(cc + 15))),
                a(cc + 20),
            )
        };
        let c = |cc: usize| col(cc);
        let colu = i % 5;
        let d = xor(c((colu + 4) % 5), rotl1(c((colu + 1) % 5)));
        xor(a(i), d)
    };
    let lower = |e: BoundedExpr| super::super::bitblast::lower_bool(&e).expect("lowers");
    // Prove for a representative lane (lane 7 — a non-zero column, exercised by d).
    let i = 7;
    // θ(x)_i ⊕ θ(y)_i  ==  θ(x⊕y)_i   ⟺  their XOR is 0 for all inputs.
    // Encode x⊕y lanes as fresh combination: (x⊕y)a_j = xa_j ⊕ ya_j.
    let tx = theta("x", i);
    let ty = theta("y", i);
    let mut txy_prefix_sum = |j: usize| xor(bv(format!("xa{j}")), bv(format!("ya{j}")));
    // Build θ over the summed lanes inline.
    let a_sum = |j: usize| xor(bv(format!("xa{j}")), bv(format!("ya{j}")));
    let col_sum = |cc: usize| {
        xor(
            xor(
                xor(a_sum(cc), a_sum(cc + 5)),
                xor(a_sum(cc + 10), a_sum(cc + 15)),
            ),
            a_sum(cc + 20),
        )
    };
    let colu = i % 5;
    let d_sum = xor(col_sum((colu + 4) % 5), rotl1(col_sum((colu + 1) % 5)));
    let txy = xor(a_sum(i), d_sum);
    let _ = &mut txy_prefix_sum;
    let lhs = xor(tx, ty);
    let claim = BoundedExpr::Eq(lhs, txy);
    let p = lower(claim);
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "theta must be GF(2)-linear (theta(x)^theta(y) == theta(x^y))"
    );
}

/// χ is a bijection on a 5-lane row — proven by our certified SAT: on 1-bit lanes
/// (the algebraic core), `x != y ⟹ chi(x) != chi(y)`. χ is the ONLY nonlinear
/// Keccak step; its invertibility is what makes the permutation invertible.
#[test]
fn keccak_chi_is_a_bijection_proven_by_our_sat() {
    use BitVecBoundedOp::*;
    const LW: u32 = 1;
    let bv = |n: String| Box::new(BoundedExpr::BitVecVar(n, LW));
    let xor = |l: Box<BoundedExpr>, r: Box<BoundedExpr>| {
        Box::new(BoundedExpr::BitVecBinary {
            op: Xor,
            left: l,
            right: r,
        })
    };
    let and = |l: Box<BoundedExpr>, r: Box<BoundedExpr>| {
        Box::new(BoundedExpr::BitVecBinary {
            op: And,
            left: l,
            right: r,
        })
    };
    let notb = |v: Box<BoundedExpr>| {
        Box::new(BoundedExpr::BitVecBinary {
            op: Not,
            left: v.clone(),
            right: v,
        })
    };
    // chi_c(row) = b_c ^ (~b_(c+1) & b_(c+2)) on a 5-lane row.
    let chi = |prefix: &str, c: usize| {
        let b = |j: usize| bv(format!("{prefix}b{}", j % 5));
        xor(b(c), and(notb(b(c + 1)), b(c + 2)))
    };
    let lower = |e: BoundedExpr| super::super::bitblast::lower_bool(&e).expect("lowers");
    // Injectivity: (x != y) ∧ (chi(x) == chi(y)) is UNSAT.
    // x != y  ⟺  OR over c of (xb_c != yb_c).
    let neq = |c: usize| {
        ProofExpr::Not(Box::new(lower(BoundedExpr::Eq(
            bv(format!("xb{c}")),
            bv(format!("yb{c}")),
        ))))
    };
    let x_ne_y = ProofExpr::Or(
        Box::new(ProofExpr::Or(
            Box::new(ProofExpr::Or(Box::new(neq(0)), Box::new(neq(1)))),
            Box::new(ProofExpr::Or(Box::new(neq(2)), Box::new(neq(3)))),
        )),
        Box::new(neq(4)),
    );
    // chi(x) == chi(y): AND over c of equality.
    let eqc = |c: usize| lower(BoundedExpr::Eq(chi("x", c), chi("y", c)));
    let chi_eq = ProofExpr::And(
        Box::new(ProofExpr::And(
            Box::new(ProofExpr::And(Box::new(eqc(0)), Box::new(eqc(1)))),
            Box::new(ProofExpr::And(Box::new(eqc(2)), Box::new(eqc(3)))),
        )),
        Box::new(eqc(4)),
    );
    let conj = ProofExpr::And(Box::new(x_ne_y), Box::new(chi_eq));
    assert_eq!(
        prove_unsat(&conj),
        UnsatOutcome::Refuted,
        "chi must be injective on a row (x != y but chi(x)==chi(y) is impossible)"
    );
}

/// The NTT butterfly RTL semantics match the native `fqmul` oracle over an
/// exhaustive sweep of one operand × representative zetas — a Rust model of the
/// emitted 16-bit arithmetic vs `ntt::fqmul`.
#[test]
fn ntt_butterfly_matches_the_native_fqmul_oracle() {
    // Rust model of the emitted RTL (bit-for-bit the same 16-bit ops).
    fn rtl_t(zeta: i16, b: i16) -> i16 {
        let prod: i32 = zeta as i32 * b as i32;
        let m: i16 = (prod as i16).wrapping_mul(KYBER_QINV_I16 as i16);
        let sub: i32 = prod - (m as i32) * KYBER_Q;
        (sub >> 16) as i16
    }
    for &zeta in &[1i16, 17, -1044, 1477, -3329, 1600] {
        for b in (-3328i16..=3328).step_by(7) {
            assert_eq!(
                rtl_t(zeta, b),
                fqmul(zeta, b),
                "butterfly t mismatch at zeta={zeta} b={b}"
            );
        }
    }
}

/// The pipelined butterfly's arithmetic, a Rust model of its steps bit for bit, is `fqmul`'s over every 16-bit `b`
/// against zetas striding the whole 16-bit range and every extreme: `m`'s low byte is the product's and its high byte
/// `p[15:8] − (p[7:0] + p[5:0]·4 + p[4:0]·8)` mod 256 (q⁻¹ = 1 − 2⁸ − 2¹⁰ − 2¹¹ mod 2¹⁶), and the reduction subtracts
/// only high halves, `t = p[31:16] − (13·m + (m >>> 8)) >>> 8` (q = 13·2⁸ + 1, and the low halves of `p` and `m·q`
/// agree — the invariant proven below — so the full difference borrows nothing).
#[test]
fn the_pipelined_ntt_butterfly_matches_the_native_fqmul_oracle() {
    fn steps(a: i16, b: i16, zeta: i16) -> (i16, i16) {
        let p: i32 = zeta as i32 * b as i32;
        let u = (p as u8).wrapping_add(((p & 0x3f) << 2) as u8).wrapping_add(((p & 0x1f) << 3) as u8);
        let m = i16::from_le_bytes([p as u8, ((p >> 8) as u8).wrapping_sub(u)]) as i32;
        let s = 13 * m + (m >> 8);
        let t = ((p >> 16) as i16).wrapping_sub((s >> 8) as i16);
        (a.wrapping_add(t), a.wrapping_sub(t))
    }
    let zetas = (i16::MIN..=i16::MAX).step_by(251).chain([i16::MIN, -1, 0, 1, i16::MAX, 17, -1044, 1477, -3329, 3329]);
    for zeta in zetas {
        for b in i16::MIN..=i16::MAX {
            let t = fqmul(zeta, b);
            for a in [0i16, 1, -1, i16::MIN, i16::MAX] {
                assert_eq!(steps(a, b, zeta), (a.wrapping_add(t), a.wrapping_sub(t)), "a={a} b={b} zeta={zeta}");
            }
        }
    }
    assert_eq!(NttCuts::PER_MULTIPLY.latency(), 5, "operands, the product, m, t, the results");
    assert_eq!(NttCuts::EVERY_STEP.latency(), 2 + NTT_STEPS as u32, "operands, every step, the results");
    let rtl = emit_ntt_butterfly_pipelined_verilog("bfly", NttCuts::EVERY_STEP);
    assert!(rtl.contains("module bfly(input clk,"), "{rtl}");
    assert!(!rtl.contains('*'), "no multiply operator: the product is the adder tree's\n{rtl}");
}

/// The Montgomery INVARIANT proven by our certified SAT: for the low 16 bits `pl`
/// of any product, `pl − ((pl·qinv) · q) ≡ 0 (mod 2^16)` — i.e. the low 16 bits of
/// `prod − m·q` vanish, which is exactly what makes the `>> 16` an EXACT division
/// (the whole point of Montgomery form). Because the claim is entirely mod 2^16, it
/// is proven at W=16 with `pl` free over ALL 16-bit values — faithful and cheap.
#[test]
fn montgomery_reduction_low16_is_zero_proven_by_our_sat() {
    use BitVecBoundedOp::*;
    const W: u32 = 16;
    let bv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), W));
    let k = |v: u64| Box::new(BoundedExpr::BitVecConst { width: W, value: v });
    let bin = |o: BitVecBoundedOp, l: Box<BoundedExpr>, r: Box<BoundedExpr>| {
        Box::new(BoundedExpr::BitVecBinary {
            op: o,
            left: l,
            right: r,
        })
    };
    let lower = |e: BoundedExpr| super::super::bitblast::lower_bool(&e).expect("lowers");
    let qinv16 = (KYBER_QINV_I16 as i16 as u16) as u64; // q^-1 mod 2^16
    let q = (KYBER_Q as u16) as u64; // q mod 2^16
                                     // m = pl · qinv  (mod 2^16);  r = pl − m·q  (mod 2^16);  claim r == 0.
    let pl = bv("pl");
    let m = bin(Mul, pl.clone(), k(qinv16));
    let r = bin(Sub, pl, bin(Mul, m, k(q)));
    let claim = BoundedExpr::Eq(r, k(0));
    let p = lower(claim);
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "Montgomery reduction must zero the low 16 bits of prod - m*q (mod 2^16)"
    );
}

/// The NTT butterfly RTL + a self-checking testbench are emitted (golden).
#[test]
fn ntt_butterfly_artifacts_are_current() {
    let rtl = emit_ntt_butterfly_verilog();
    // Vectors: (a, b, zeta, hi, lo) rows from the oracle.
    let mut hex = String::new();
    let cases = [
        (10i16, 200, 17),
        (-500, 1477, -1044),
        (3000, -2000, 1600),
        (1, 1, 1),
    ];
    for &(a, b, zeta) in &cases {
        let t = fqmul(zeta, b);
        let hi = a.wrapping_add(t);
        let lo = a.wrapping_sub(t);
        hex.push_str(&format!(
            "{:04x} {:04x} {:04x} {:04x} {:04x}\n",
            a as u16, b as u16, zeta as u16, hi as u16, lo as u16
        ));
    }
    let dir = silicon_dir();
    let files = [("ntt_butterfly.v", rtl), ("ntt_vec.hex", hex)];
    if std::env::var("KECCAK_RECORD").is_ok() {
        for (n, b) in &files {
            std::fs::write(dir.join(n), b).expect("write");
        }
        return;
    }
    for (n, b) in &files {
        let got = std::fs::read_to_string(dir.join(n)).unwrap_or_else(|e| {
            panic!("missing {n} ({e}) — record: KECCAK_RECORD=1 cargo test ntt_butterfly_artifacts")
        });
        assert_eq!(&got, b, "{n} is stale — regenerate with KECCAK_RECORD=1");
    }
}
