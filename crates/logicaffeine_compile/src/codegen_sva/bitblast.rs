//! Bit-blasting: `BoundedExpr` datapath (bitvector) operations → boolean `ProofExpr`.
//!
//! A bitvector is represented LSB-first as a `Vec<Bit>`, each `Bit` either a known constant
//! or a boolean `ProofExpr` (a signal bit `name#i`, or a gate). Every combinator
//! constant-folds, so a constant operand collapses gates instead of emitting them — the CNF
//! stays small. Each operation is the textbook circuit: ripple-carry add/sub, array
//! multiplier, barrel shifters, magnitude/signed comparators, slice and concat.
//!
//! This is the data-path half of the certified-prover seam: combined with the Boolean
//! lowering in [`super::sva_to_proof`], multi-bit hardware obligations reduce to a
//! propositional formula our CDCL→RUP tiers discharge — in the browser, no Z3.

use super::sva_to_verify::{BitVecBoundedOp, BoundedExpr, CmpBoundedOp};
use logicaffeine_proof::ProofExpr;

/// One blasted bit: a known constant, or a boolean `ProofExpr`.
#[derive(Clone)]
enum Bit {
    Const(bool),
    Dyn(ProofExpr),
}

fn b_not(x: Bit) -> Bit {
    match x {
        Bit::Const(c) => Bit::Const(!c),
        Bit::Dyn(e) => Bit::Dyn(ProofExpr::Not(Box::new(e))),
    }
}

fn b_and(a: Bit, b: Bit) -> Bit {
    match (a, b) {
        (Bit::Const(false), _) | (_, Bit::Const(false)) => Bit::Const(false),
        (Bit::Const(true), y) | (y, Bit::Const(true)) => y,
        (Bit::Dyn(x), Bit::Dyn(y)) => Bit::Dyn(ProofExpr::And(Box::new(x), Box::new(y))),
    }
}

fn b_or(a: Bit, b: Bit) -> Bit {
    match (a, b) {
        (Bit::Const(true), _) | (_, Bit::Const(true)) => Bit::Const(true),
        (Bit::Const(false), y) | (y, Bit::Const(false)) => y,
        (Bit::Dyn(x), Bit::Dyn(y)) => Bit::Dyn(ProofExpr::Or(Box::new(x), Box::new(y))),
    }
}

/// XNOR / equality of two bits.
fn b_iff(a: Bit, b: Bit) -> Bit {
    match (a, b) {
        (Bit::Const(c), Bit::Const(d)) => Bit::Const(c == d),
        (Bit::Const(true), y) | (y, Bit::Const(true)) => y,
        (Bit::Const(false), y) | (y, Bit::Const(false)) => b_not(y),
        (Bit::Dyn(x), Bit::Dyn(y)) => Bit::Dyn(ProofExpr::Iff(Box::new(x), Box::new(y))),
    }
}

fn b_xor(a: Bit, b: Bit) -> Bit {
    b_not(b_iff(a, b))
}

/// If-then-else (a 1-bit mux): `s ? t : e`.
fn b_ite(s: Bit, t: Bit, e: Bit) -> Bit {
    match s {
        Bit::Const(true) => t,
        Bit::Const(false) => e,
        s => b_or(b_and(s.clone(), t), b_and(b_not(s), e)),
    }
}

fn bit_to_proof(b: Bit) -> ProofExpr {
    match b {
        // `ProofExpr` has no Boolean literal; encode constants as a tautology / contradiction
        // over a reserved atom (`Or(c,¬c)` is always true, `And(c,¬c)` always false).
        Bit::Const(true) => {
            let c = ProofExpr::Atom("__bit_const".to_string());
            ProofExpr::Or(Box::new(c.clone()), Box::new(ProofExpr::Not(Box::new(c))))
        }
        Bit::Const(false) => {
            let c = ProofExpr::Atom("__bit_const".to_string());
            ProofExpr::And(Box::new(c.clone()), Box::new(ProofExpr::Not(Box::new(c))))
        }
        Bit::Dyn(e) => e,
    }
}

// ── Bitvector circuits (operands LSB-first, equal width) ────────────────────────────────

/// Ripple-carry add: returns `(sum bits, carry-out)`.
fn ripple_add(a: &[Bit], b: &[Bit], carry_in: Bit) -> (Vec<Bit>, Bit) {
    let mut carry = carry_in;
    let mut out = Vec::with_capacity(a.len());
    for i in 0..a.len() {
        let axb = b_xor(a[i].clone(), b[i].clone());
        let sum = b_xor(axb.clone(), carry.clone());
        let carry_next = b_or(
            b_and(a[i].clone(), b[i].clone()),
            b_and(carry.clone(), axb),
        );
        out.push(sum);
        carry = carry_next;
    }
    (out, carry)
}

fn bv_add(a: &[Bit], b: &[Bit]) -> Vec<Bit> {
    ripple_add(a, b, Bit::Const(false)).0
}

/// Two's-complement subtract: `a - b = a + ¬b + 1`.
fn bv_sub(a: &[Bit], b: &[Bit]) -> Vec<Bit> {
    let nb: Vec<Bit> = b.iter().cloned().map(b_not).collect();
    ripple_add(a, &nb, Bit::Const(true)).0
}

/// Shift-and-add array multiplier, truncated to the operand width.
fn bv_mul(a: &[Bit], b: &[Bit]) -> Vec<Bit> {
    let w = a.len();
    let mut acc: Vec<Bit> = vec![Bit::Const(false); w];
    for i in 0..w {
        // Partial product: (a << i) gated by b[i], truncated to width w.
        let mut pp: Vec<Bit> = vec![Bit::Const(false); w];
        for j in i..w {
            pp[j] = b_and(a[j - i].clone(), b[i].clone());
        }
        acc = bv_add(&acc, &pp);
    }
    acc
}

/// Shift left by a constant `k` (zero-fill), width-preserving.
fn shl_const(a: &[Bit], k: usize) -> Vec<Bit> {
    let w = a.len();
    (0..w)
        .map(|j| if j >= k { a[j - k].clone() } else { Bit::Const(false) })
        .collect()
}

/// Shift right by a constant `k`, filling the vacated top bits with `fill`.
fn shr_const(a: &[Bit], k: usize, fill: Bit) -> Vec<Bit> {
    let w = a.len();
    (0..w)
        .map(|j| if j + k < w { a[j + k].clone() } else { fill.clone() })
        .collect()
}

/// Barrel shifter for a variable amount: compose constant shifts of `1<<s` gated by the
/// amount's bit `s`. `fill` is the vacated-bit value (0 for logical, sign for arithmetic);
/// `left` selects direction.
fn barrel_shift(a: &[Bit], amount: &[Bit], left: bool, fill: Bit) -> Vec<Bit> {
    let w = a.len();
    let mut cur = a.to_vec();
    for (s, amt_bit) in amount.iter().enumerate() {
        let dist = 1usize << s;
        if dist >= w {
            // Any set bit at or beyond this position shifts everything out.
            let shifted = vec![fill.clone(); w];
            cur = (0..w)
                .map(|j| b_ite(amt_bit.clone(), shifted[j].clone(), cur[j].clone()))
                .collect();
            continue;
        }
        let shifted = if left {
            shl_const(&cur, dist)
        } else {
            shr_const(&cur, dist, fill.clone())
        };
        cur = (0..w)
            .map(|j| b_ite(amt_bit.clone(), shifted[j].clone(), cur[j].clone()))
            .collect();
    }
    cur
}

/// `a == b` over equal-width bitvectors → a single bit.
fn bv_eq(a: &[Bit], b: &[Bit]) -> Bit {
    let mut acc = Bit::Const(true);
    for i in 0..a.len() {
        acc = b_and(acc, b_iff(a[i].clone(), b[i].clone()));
    }
    acc
}

/// Unsigned `a < b` → a single bit (LSB→MSB magnitude comparator).
fn bv_ult(a: &[Bit], b: &[Bit]) -> Bit {
    let mut lt = Bit::Const(false);
    for i in 0..a.len() {
        // a<b on bits 0..=i  ≡  (a[i]<b[i]) ∨ (a[i]=b[i] ∧ a<b on bits below i)
        let bit_lt = b_and(b_not(a[i].clone()), b[i].clone());
        let bit_eq = b_iff(a[i].clone(), b[i].clone());
        lt = b_or(bit_lt, b_and(bit_eq, lt));
    }
    lt
}

/// Signed (two's-complement) `a < b` → a single bit.
fn bv_slt(a: &[Bit], b: &[Bit]) -> Bit {
    let w = a.len();
    let sa = a[w - 1].clone();
    let sb = b[w - 1].clone();
    // Differing signs: the negative one (sign bit 1) is smaller, so result = a's sign.
    // Same sign: the unsigned ordering coincides with the signed ordering.
    b_ite(b_xor(sa.clone(), sb), sa, bv_ult(a, b))
}

// ── BoundedExpr → bits / bool ───────────────────────────────────────────────────────────

/// Blast a bitvector-VALUED `BoundedExpr` into its bits (LSB-first), or `None` if it is not a
/// supported bitvector expression (e.g. an Int, a comparison, or mismatched widths).
fn blast_bits(e: &BoundedExpr) -> Option<Vec<Bit>> {
    match e {
        BoundedExpr::BitVecConst { width, value } => {
            Some((0..*width).map(|i| Bit::Const((value >> i) & 1 == 1)).collect())
        }
        BoundedExpr::BitVecVar(name, width) => Some(
            (0..*width)
                .map(|i| Bit::Dyn(ProofExpr::Atom(format!("{name}#{i}"))))
                .collect(),
        ),
        BoundedExpr::BitVecExtract { high, low, operand } => {
            let bits = blast_bits(operand)?;
            let (lo, hi) = (*low as usize, *high as usize);
            if hi >= bits.len() || lo > hi {
                return None;
            }
            Some(bits[lo..=hi].to_vec())
        }
        BoundedExpr::BitVecConcat(a, b) => {
            // SVA `{a, b}`: `a` is the high half. LSB-first ⇒ b's bits then a's bits.
            let mut bits = blast_bits(b)?;
            bits.extend(blast_bits(a)?);
            Some(bits)
        }
        BoundedExpr::BitVecBinary { op, left, right } => {
            let a = blast_bits(left)?;
            let b = blast_bits(right)?;
            match op {
                BitVecBoundedOp::Not => Some(a.into_iter().map(b_not).collect()),
                // Width-preserving binary circuits require matching widths.
                _ if a.len() != b.len() => None,
                BitVecBoundedOp::And => Some(zip_map(a, b, b_and)),
                BitVecBoundedOp::Or => Some(zip_map(a, b, b_or)),
                BitVecBoundedOp::Xor => Some(zip_map(a, b, b_xor)),
                BitVecBoundedOp::Add => Some(bv_add(&a, &b)),
                BitVecBoundedOp::Sub => Some(bv_sub(&a, &b)),
                BitVecBoundedOp::Mul => Some(bv_mul(&a, &b)),
                BitVecBoundedOp::Shl => Some(barrel_shift(&a, &b, true, Bit::Const(false))),
                BitVecBoundedOp::Shr => Some(barrel_shift(&a, &b, false, Bit::Const(false))),
                BitVecBoundedOp::AShr => {
                    let sign = a[a.len() - 1].clone();
                    Some(barrel_shift(&a, &b, false, sign))
                }
                // Comparison ops are boolean-valued — not a bit vector.
                BitVecBoundedOp::Eq | BitVecBoundedOp::ULt | BitVecBoundedOp::SLt => None,
            }
        }
        _ => None,
    }
}

fn zip_map(a: Vec<Bit>, b: Vec<Bit>, f: fn(Bit, Bit) -> Bit) -> Vec<Bit> {
    a.into_iter().zip(b).map(|(x, y)| f(x, y)).collect()
}

/// Lower a boolean-VALUED datapath comparison to a `ProofExpr`, or `None` if unsupported.
/// Comparisons over bitvectors are unsigned unless the op is explicitly signed (`SLt`).
pub fn lower_bool(e: &BoundedExpr) -> Option<ProofExpr> {
    let bit = match e {
        BoundedExpr::BitVecBinary { op, left, right } => {
            let a = blast_bits(left)?;
            let b = blast_bits(right)?;
            if a.len() != b.len() {
                return None;
            }
            match op {
                BitVecBoundedOp::Eq => bv_eq(&a, &b),
                BitVecBoundedOp::ULt => bv_ult(&a, &b),
                BitVecBoundedOp::SLt => bv_slt(&a, &b),
                _ => return None,
            }
        }
        BoundedExpr::Eq(l, r) => {
            let a = blast_bits(l)?;
            let b = blast_bits(r)?;
            if a.len() != b.len() {
                return None;
            }
            bv_eq(&a, &b)
        }
        BoundedExpr::Lt(l, r) | BoundedExpr::Gt(l, r) | BoundedExpr::Lte(l, r)
        | BoundedExpr::Gte(l, r) => bv_compare(e, l, r)?,
        BoundedExpr::Comparison { op, left, right } => {
            let a = blast_bits(left)?;
            let b = blast_bits(right)?;
            if a.len() != b.len() {
                return None;
            }
            match op {
                CmpBoundedOp::Lt => bv_ult(&a, &b),
                CmpBoundedOp::Gt => bv_ult(&b, &a),
                CmpBoundedOp::Lte => b_not(bv_ult(&b, &a)),
                CmpBoundedOp::Gte => b_not(bv_ult(&a, &b)),
            }
        }
        _ => return None,
    };
    Some(bit_to_proof(bit))
}

/// Unsigned magnitude comparison for the top-level `Lt/Gt/Lte/Gte` BoundedExpr variants.
fn bv_compare(e: &BoundedExpr, l: &BoundedExpr, r: &BoundedExpr) -> Option<Bit> {
    let a = blast_bits(l)?;
    let b = blast_bits(r)?;
    if a.len() != b.len() {
        return None;
    }
    Some(match e {
        BoundedExpr::Lt(..) => bv_ult(&a, &b),
        BoundedExpr::Gt(..) => bv_ult(&b, &a),
        BoundedExpr::Lte(..) => b_not(bv_ult(&b, &a)),
        BoundedExpr::Gte(..) => b_not(bv_ult(&a, &b)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};

    const W: u32 = 4;
    const MASK: u64 = (1 << W) - 1;

    fn konst(v: u64) -> BoundedExpr {
        BoundedExpr::BitVecConst { width: W, value: v & MASK }
    }
    fn bv_bin(op: BitVecBoundedOp, a: u64, b: u64) -> BoundedExpr {
        BoundedExpr::BitVecBinary { op, left: Box::new(konst(a)), right: Box::new(konst(b)) }
    }
    /// A constant boolean expression is a tautology — assert it via certified UNSAT of its
    /// negation. `expect` says whether it should be a tautology or a contradiction.
    fn assert_bool(e: &BoundedExpr, expect_true: bool) {
        let p = lower_bool(e).expect("lowered to a boolean");
        if expect_true {
            assert_eq!(prove_unsat(&ProofExpr::Not(Box::new(p))), UnsatOutcome::Refuted);
        } else {
            assert_eq!(prove_unsat(&p), UnsatOutcome::Refuted);
        }
    }
    /// Assert a bitvector op equals an integer-oracle result, exactly (and that off-by-one is
    /// rejected), by lowering `op(a,b) == expected` and `== expected+1`.
    fn assert_bv(op: BitVecBoundedOp, a: u64, b: u64, expected: u64) {
        let eq = BoundedExpr::Eq(Box::new(bv_bin(op.clone(), a, b)), Box::new(konst(expected)));
        assert_bool(&eq, true);
        let neq =
            BoundedExpr::Eq(Box::new(bv_bin(op, a, b)), Box::new(konst(expected.wrapping_add(1))));
        assert_bool(&neq, false);
    }

    #[test]
    fn add_sub_mul_match_integer_oracle_exhaustively() {
        for a in 0..=MASK {
            for b in 0..=MASK {
                assert_bv(BitVecBoundedOp::Add, a, b, (a + b) & MASK);
                assert_bv(BitVecBoundedOp::Sub, a, b, a.wrapping_sub(b) & MASK);
                assert_bv(BitVecBoundedOp::Mul, a, b, (a * b) & MASK);
            }
        }
    }

    #[test]
    fn bitwise_ops_match_oracle_exhaustively() {
        for a in 0..=MASK {
            for b in 0..=MASK {
                assert_bv(BitVecBoundedOp::And, a, b, a & b);
                assert_bv(BitVecBoundedOp::Or, a, b, a | b);
                assert_bv(BitVecBoundedOp::Xor, a, b, a ^ b);
            }
        }
        // NOT is unary: ¬a over the width.
        for a in 0..=MASK {
            let e = BoundedExpr::Eq(
                Box::new(BoundedExpr::BitVecBinary {
                    op: BitVecBoundedOp::Not,
                    left: Box::new(konst(a)),
                    right: Box::new(konst(0)),
                }),
                Box::new(konst(!a & MASK)),
            );
            assert_bool(&e, true);
        }
    }

    #[test]
    fn shifts_match_oracle_exhaustively() {
        for a in 0..=MASK {
            for s in 0..=MASK {
                let sh = (s & MASK) as u32;
                let shl = if sh < W { (a << sh) & MASK } else { 0 };
                assert_bv(BitVecBoundedOp::Shl, a, s, shl);
                let shr = if sh < W { (a & MASK) >> sh } else { 0 };
                assert_bv(BitVecBoundedOp::Shr, a, s, shr);
                // Arithmetic shift right: sign-extend the top bit.
                let sign = (a >> (W - 1)) & 1 == 1;
                let ashr = if sh >= W {
                    if sign { MASK } else { 0 }
                } else {
                    let base = (a & MASK) >> sh;
                    if sign {
                        base | (MASK & !((1 << (W - sh)) - 1))
                    } else {
                        base
                    }
                };
                assert_bv(BitVecBoundedOp::AShr, a, s, ashr & MASK);
            }
        }
    }

    #[test]
    fn comparisons_match_oracle_exhaustively() {
        for a in 0..=MASK {
            for b in 0..=MASK {
                assert_bool(&bv_bin(BitVecBoundedOp::Eq, a, b), a == b);
                assert_bool(&bv_bin(BitVecBoundedOp::ULt, a, b), a < b);
                // Signed interpretation over W bits (two's complement).
                let sa = sign_extend(a);
                let sb = sign_extend(b);
                assert_bool(&bv_bin(BitVecBoundedOp::SLt, a, b), sa < sb);
                // Top-level unsigned comparisons.
                assert_bool(&BoundedExpr::Lt(Box::new(konst(a)), Box::new(konst(b))), a < b);
                assert_bool(&BoundedExpr::Gte(Box::new(konst(a)), Box::new(konst(b))), a >= b);
            }
        }
    }

    fn sign_extend(v: u64) -> i64 {
        let v = (v & MASK) as i64;
        if (v >> (W - 1)) & 1 == 1 {
            v - (1 << W)
        } else {
            v
        }
    }

    #[test]
    fn extract_and_concat_match_oracle() {
        // extract [2:1] of a 4-bit constant.
        for a in 0..=MASK {
            let ex = BoundedExpr::BitVecExtract {
                high: 2,
                low: 1,
                operand: Box::new(konst(a)),
            };
            let expected = (a >> 1) & 0b11;
            let e = BoundedExpr::Eq(
                Box::new(ex),
                Box::new(BoundedExpr::BitVecConst { width: 2, value: expected }),
            );
            assert_bool(&e, true);
        }
        // concat {a(2b), b(2b)} == a<<2 | b, width 4.
        for a in 0..4 {
            for b in 0..4 {
                let cc = BoundedExpr::BitVecConcat(
                    Box::new(BoundedExpr::BitVecConst { width: 2, value: a }),
                    Box::new(BoundedExpr::BitVecConst { width: 2, value: b }),
                );
                let e = BoundedExpr::Eq(Box::new(cc), Box::new(konst((a << 2) | b)));
                assert_bool(&e, true);
            }
        }
    }

    #[test]
    fn datapath_equivalence_over_variables() {
        // x + y ≡ y + x (commutativity), proven over all 4-bit x,y by our certified prover.
        let x = || Box::new(BoundedExpr::BitVecVar("x".to_string(), W));
        let y = || Box::new(BoundedExpr::BitVecVar("y".to_string(), W));
        let xy = BoundedExpr::BitVecBinary { op: BitVecBoundedOp::Add, left: x(), right: y() };
        let yx = BoundedExpr::BitVecBinary { op: BitVecBoundedOp::Add, left: y(), right: x() };
        let eq = BoundedExpr::Eq(Box::new(xy), Box::new(yx));
        let p = lower_bool(&eq).unwrap();
        assert_eq!(prove_unsat(&ProofExpr::Not(Box::new(p))), UnsatOutcome::Refuted);
    }

    #[test]
    fn rotl_is_a_bit_permutation_at_full_32_bits() {
        // The word-width proof wall closes: rotl over the FULL 32-bit width is proven a
        // bit-permutation (injective) by our OWN certified CDCL — no Z3, and not a ≤16-bit
        // exhaustive sample. rotl is pure wiring, so plain `prove_unsat` discharges it; the
        // lex-leader symmetry-breaking layer (`logicaffeine_proof::symmetry`) is the lever
        // reserved for the carry-heavy datapaths (wide add/mul) whose CNF would blow up.
        const WW: u32 = 32;
        // rotl(name, k) = (v << k) | (v >> (32 - k)) over a fresh 32-bit variable `name`.
        fn rotl(name: &str, k: u32) -> BoundedExpr {
            let v = || Box::new(BoundedExpr::BitVecVar(name.to_string(), WW));
            let kc = |s: u32| Box::new(BoundedExpr::BitVecConst { width: WW, value: s as u64 });
            let hi = Box::new(BoundedExpr::BitVecBinary { op: BitVecBoundedOp::Shl, left: v(), right: kc(k) });
            let lo =
                Box::new(BoundedExpr::BitVecBinary { op: BitVecBoundedOp::Shr, left: v(), right: kc(WW - k) });
            BoundedExpr::BitVecBinary { op: BitVecBoundedOp::Or, left: hi, right: lo }
        }
        let var = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), WW));

        // The ChaCha20 rotation amounts — the ones the quarter-round actually uses.
        for k in [7u32, 8, 12, 16] {
            // Injectivity: rotl(x,k) == rotl(y,k)  ⟹  x == y. The counterexample (equal
            // rotations, unequal inputs) must be REFUTED over all 32-bit inputs.
            let eq_rot = lower_bool(&BoundedExpr::Eq(Box::new(rotl("x", k)), Box::new(rotl("y", k))))
                .expect("rotation equality lowered");
            let eq_in = lower_bool(&BoundedExpr::Eq(var("x"), var("y"))).expect("input equality lowered");
            let counter = ProofExpr::And(Box::new(eq_rot), Box::new(ProofExpr::Not(Box::new(eq_in))));
            assert_eq!(
                prove_unsat(&counter),
                UnsatOutcome::Refuted,
                "rotl by {k} must be injective (a bit-permutation) over all 32-bit inputs"
            );
        }

        // Non-vacuity: a NON-permutation (`x & 0`, which collapses every input to 0) must NOT
        // be refuted — distinct inputs with equal images genuinely exist, so a sound prover
        // finds the counterexample rather than vacuously "proving" injectivity.
        let zero_and = |n: &str| BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::And,
            left: Box::new(BoundedExpr::BitVecVar(n.to_string(), WW)),
            right: Box::new(BoundedExpr::BitVecConst { width: WW, value: 0 }),
        };
        let eq_img = lower_bool(&BoundedExpr::Eq(Box::new(zero_and("x")), Box::new(zero_and("y"))))
            .expect("image equality lowered");
        let eq_in = lower_bool(&BoundedExpr::Eq(var("x"), var("y"))).expect("input equality lowered");
        let counter = ProofExpr::And(Box::new(eq_img), Box::new(ProofExpr::Not(Box::new(eq_in))));
        assert_ne!(
            prove_unsat(&counter),
            UnsatOutcome::Refuted,
            "x & 0 is NOT injective — the prover must not vacuously refute its counterexample"
        );
    }
}
