use super::*;

/// Every obligation shape the certifier builds: a multiply/accumulate miter (the MAC PE), a
/// predicated select (the mux tile), a shifted/masked bit test, a signed comparison, and a
/// constant-folded arm — so the equivalence below is exercised over Atom, Not, And, Or, Iff
/// and both `Bit::Const` spellings.
fn clausification_corpus() -> Vec<(&'static str, BoundedExpr)> {
    const W: u32 = 8;
    let v = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), W));
    let k = |c: u64| Box::new(BoundedExpr::BitVecConst { width: W, value: c });
    let bin = |op, l, r| {
        Box::new(BoundedExpr::BitVecBinary {
            op,
            left: l,
            right: r,
        })
    };
    let mac = bin(
        BitVecBoundedOp::Add,
        v("b"),
        bin(BitVecBoundedOp::Mul, v("a"), v("w")),
    );
    let mac2 = bin(
        BitVecBoundedOp::Add,
        bin(BitVecBoundedOp::Mul, v("w"), v("a")),
        v("b"),
    );
    let ite = Box::new(BoundedExpr::BitVecIte {
        cond: bin(BitVecBoundedOp::SLt, v("a"), k(0)),
        then: bin(BitVecBoundedOp::Sub, k(0), v("a")),
        else_: v("a"),
    });
    let masked = bin(
        BitVecBoundedOp::And,
        bin(BitVecBoundedOp::Shr, v("n"), k(1)),
        k(1),
    );
    vec![
        ("mac ≡ mac (commuted)", BoundedExpr::Eq(mac.clone(), mac2)),
        (
            "mac ≢ shifted",
            BoundedExpr::Eq(mac, bin(BitVecBoundedOp::Shl, v("a"), k(1))),
        ),
        ("abs ≡ abs", BoundedExpr::Eq(ite.clone(), ite)),
        ("masked bit ≡ const", BoundedExpr::Eq(masked, k(1))),
        ("signed less-than", BoundedExpr::Lt(v("a"), v("b"))),
        ("const ≡ const", BoundedExpr::Eq(k(3), k(3))),
        ("const ≢ const", BoundedExpr::Eq(k(3), k(4))),
    ]
}

/// The shared gate DAG is a PERFORMANCE property, never a semantic one. Clausifying the DAG in
/// place must emit the very clauses `Cnf::assert` emits from the materialised `ProofExpr`
/// tree — same count, same order, same variable numbering — because the certificate the
/// recipient rechecks IS that clause set. A cheaper encoding that produced a DIFFERENT (even
/// equisatisfiable) CNF would silently re-key every shipped certificate.
#[test]
fn the_gate_path_and_the_tree_path_clausify_identically() {
    for (label, e) in clausification_corpus() {
        let p = lower_bool(&e).unwrap_or_else(|| panic!("{label} blasts"));
        let mut tree = Cnf::new();
        tree.assert(&ProofExpr::Not(Box::new(p)))
            .unwrap_or_else(|| panic!("{label} asserts"));

        let pred = blast_pred(&e).unwrap_or_else(|| panic!("{label} blasts to a gate"));
        let mut dag = Cnf::new();
        assert_not(&pred, &mut dag);

        assert_eq!(dag.num_vars(), tree.num_vars(), "{label}: variable count");
        assert_eq!(
            dag.clauses().len(),
            tree.clauses().len(),
            "{label}: clause count"
        );
        assert_eq!(
            dag.clauses(),
            tree.clauses(),
            "{label}: the clauses themselves, in order"
        );
    }
}

/// And the DAG the producers build is the SAME circuit the `BoundedExpr` bridge blasts — so a
/// datapath built straight into gates and one routed through the tree are the same obligation.
#[test]
fn a_word_built_directly_is_the_word_the_bridge_blasts() {
    const W: u32 = 8;
    let direct = {
        let a = word_var("a", W);
        let w = word_var("w", W);
        let b = word_var("b", W);
        let m = word_binary(BitVecBoundedOp::Mul, &a, &w).expect("one width");
        word_binary(BitVecBoundedOp::Add, &b, &m).expect("one width")
    };
    let v = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), W));
    let bridged = blast_word(&BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Add,
        left: v("b"),
        right: Box::new(BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::Mul,
            left: v("a"),
            right: v("w"),
        }),
    })
    .expect("the bridge blasts");
    let miter = words_equal(&direct, &bridged).expect("one width");
    let mut cnf = Cnf::new();
    assert_not(&miter, &mut cnf);
    assert!(
        logicaffeine_proof::sat::prove_unsat_certified_cnf(cnf.num_vars(), cnf.clauses()).is_some(),
        "the directly-built word and the bridged one must be provably the same circuit"
    );
    // Fail-closed: a DIFFERENT circuit is not refutable.
    let other = word_binary(
        BitVecBoundedOp::Sub,
        &word_var("b", W),
        &word_binary(BitVecBoundedOp::Mul, &word_var("a", W), &word_var("w", W))
            .expect("one width"),
    )
    .expect("one width");
    let bad = words_equal(&direct, &other).expect("one width");
    let mut cnf = Cnf::new();
    assert_not(&bad, &mut cnf);
    assert!(
        logicaffeine_proof::sat::prove_unsat_certified_cnf(cnf.num_vars(), cnf.clauses()).is_none(),
        "`b − a·w` is not `b + a·w` — the miter must NOT be refutable"
    );
}

/// The gate DAG's depth is the datapath's, so every walk over it is iterative. A chain deep
/// enough to blow a recursive walk's stack must price and clausify without one.
#[test]
fn a_deep_chain_prices_and_clausifies_without_recursing() {
    const W: u32 = 8;
    let mut acc = word_var("x", W);
    let k = word_const(W, 3);
    for _ in 0..4_000 {
        acc = word_binary(BitVecBoundedOp::Add, &acc, &k).expect("one width");
    }
    let size = word_size(&acc);
    assert!(
        size.dag_nodes > 10_000,
        "a 4 000-deep chain is a real circuit: {size:?}"
    );
    let miter = words_equal(&acc, &acc).expect("one width");
    let mut cnf = Cnf::new();
    assert_not(&miter, &mut cnf);
    assert!(cnf.num_vars() > 0, "the deep chain clausified");
}
use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};

const W: u32 = 4;
const MASK: u64 = (1 << W) - 1;

fn konst(v: u64) -> BoundedExpr {
    BoundedExpr::BitVecConst {
        width: W,
        value: v & MASK,
    }
}
fn bv_bin(op: BitVecBoundedOp, a: u64, b: u64) -> BoundedExpr {
    BoundedExpr::BitVecBinary {
        op,
        left: Box::new(konst(a)),
        right: Box::new(konst(b)),
    }
}
/// A constant boolean expression is a tautology — assert it via certified UNSAT of its
/// negation. `expect` says whether it should be a tautology or a contradiction.
fn assert_bool(e: &BoundedExpr, expect_true: bool) {
    let p = lower_bool(e).expect("lowered to a boolean");
    if expect_true {
        assert_eq!(
            prove_unsat(&ProofExpr::Not(Box::new(p))),
            UnsatOutcome::Refuted
        );
    } else {
        assert_eq!(prove_unsat(&p), UnsatOutcome::Refuted);
    }
}
/// Assert a bitvector op equals an integer-oracle result, exactly (and that off-by-one is
/// rejected), by lowering `op(a,b) == expected` and `== expected+1`.
fn assert_bv(op: BitVecBoundedOp, a: u64, b: u64, expected: u64) {
    let eq = BoundedExpr::Eq(
        Box::new(bv_bin(op.clone(), a, b)),
        Box::new(konst(expected)),
    );
    assert_bool(&eq, true);
    let neq = BoundedExpr::Eq(
        Box::new(bv_bin(op, a, b)),
        Box::new(konst(expected.wrapping_add(1))),
    );
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
                if sign {
                    MASK
                } else {
                    0
                }
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
            assert_bool(
                &BoundedExpr::Lt(Box::new(konst(a)), Box::new(konst(b))),
                a < b,
            );
            assert_bool(
                &BoundedExpr::Gte(Box::new(konst(a)), Box::new(konst(b))),
                a >= b,
            );
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
            Box::new(BoundedExpr::BitVecConst {
                width: 2,
                value: expected,
            }),
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
    let xy = BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Add,
        left: x(),
        right: y(),
    };
    let yx = BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Add,
        left: y(),
        right: x(),
    };
    let eq = BoundedExpr::Eq(Box::new(xy), Box::new(yx));
    let p = lower_bool(&eq).unwrap();
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted
    );
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
        let kc = |s: u32| {
            Box::new(BoundedExpr::BitVecConst {
                width: WW,
                value: s as u64,
            })
        };
        let hi = Box::new(BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::Shl,
            left: v(),
            right: kc(k),
        });
        let lo = Box::new(BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::Shr,
            left: v(),
            right: kc(WW - k),
        });
        BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::Or,
            left: hi,
            right: lo,
        }
    }
    let var = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), WW));

    // The ChaCha20 rotation amounts — the ones the quarter-round actually uses.
    for k in [7u32, 8, 12, 16] {
        // Injectivity: rotl(x,k) == rotl(y,k)  ⟹  x == y. The counterexample (equal
        // rotations, unequal inputs) must be REFUTED over all 32-bit inputs.
        let eq_rot = lower_bool(&BoundedExpr::Eq(
            Box::new(rotl("x", k)),
            Box::new(rotl("y", k)),
        ))
        .expect("rotation equality lowered");
        let eq_in =
            lower_bool(&BoundedExpr::Eq(var("x"), var("y"))).expect("input equality lowered");
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
        right: Box::new(BoundedExpr::BitVecConst {
            width: WW,
            value: 0,
        }),
    };
    let eq_img = lower_bool(&BoundedExpr::Eq(
        Box::new(zero_and("x")),
        Box::new(zero_and("y")),
    ))
    .expect("image equality lowered");
    let eq_in = lower_bool(&BoundedExpr::Eq(var("x"), var("y"))).expect("input equality lowered");
    let counter = ProofExpr::And(Box::new(eq_img), Box::new(ProofExpr::Not(Box::new(eq_in))));
    assert_ne!(
        prove_unsat(&counter),
        UnsatOutcome::Refuted,
        "x & 0 is NOT injective — the prover must not vacuously refute its counterexample"
    );
}
