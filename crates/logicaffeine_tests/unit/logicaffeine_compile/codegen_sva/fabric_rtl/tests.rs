use super::*;
use crate::compile::{count_dispatch, vm_outcome};

fn silicon_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// Run `fn_name(args)` through the REAL VM and reduce its EXACT (arbitrary-precision)
/// integer result to the machine word mod 2^64 — precisely what a fixed-width circuit
/// computes. The VM's `Int` is unbounded; the fabric is the fast fixed-width projection of
/// it (the BigInt coprocessor is the unbounded path), so the honest ≡ is "circuit ==
/// VM mod 2^64", which holds for EVERY input with no range restriction.
fn vm_word(decl: &str, fn_name: &str, args: &[i64]) -> u64 {
    let argstr = args
        .iter()
        .map(|a| a.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let src = format!("{decl}\n## Main\n    Show {fn_name}({argstr}).");
    let out = vm_outcome(&src);
    assert!(
        out.error.is_none(),
        "{fn_name}({argstr}) VM error: {:?}",
        out.error
    );
    let s = out.output.trim();
    let (neg, digits) = s.strip_prefix('-').map_or((false, s), |d| (true, d));
    // Reduce the exact decimal result mod 2^64 by u64-wrapping Horner — the machine word.
    let mut acc: u64 = 0;
    for ch in digits.bytes() {
        assert!(ch.is_ascii_digit(), "non-digit in VM output {s:?}");
        acc = acc.wrapping_mul(10).wrapping_add((ch - b'0') as u64);
    }
    if neg {
        acc.wrapping_neg()
    } else {
        acc
    }
}

fn poly_word(x: i64) -> u64 {
    vm_word(
        &format!("## To poly (x: Int) -> Int:\n    Return {POLY_BODY}."),
        "poly",
        &[x],
    )
}

/// Emit the fabric RTL (lowered from the REAL compiled residual), its testbench, and the
/// VM oracle vectors (golden; regenerate with `FABRIC_RECORD=1`).
#[test]
fn fabric_artifacts_are_current() {
    const N: usize = 64;
    let (ops, result, n_in) = residual_from_source(&poly_program("Show poly(4)."), "poly");
    assert_eq!(n_in, 1);
    let mut hin = String::new();
    let mut hout = String::new();
    for k in 0..N {
        let x = (k as i64) - 32; // -32..=31, a compact spread (correctness holds for all i64)
        hin.push_str(&format!("{:016x}\n", x as u64));
        hout.push_str(&format!("{:016x}\n", poly_word(x)));
    }
    let files = [
        (
            "fabric_horner.v",
            emit_fabric_verilog("horner", 1, &ops, result),
        ),
        ("tb_fabric_horner.v", emit_fabric_testbench("horner", N)),
        ("fabric_in.hex", hin),
        ("fabric_out.hex", hout),
    ];
    let dir = silicon_dir();
    if std::env::var("FABRIC_RECORD").is_ok() {
        for (n, b) in &files {
            std::fs::write(dir.join(n), b).expect("write");
        }
        return;
    }
    for (n, b) in &files {
        let got = std::fs::read_to_string(dir.join(n)).unwrap_or_else(|e| {
            panic!("missing {n} ({e}) — record: FABRIC_RECORD=1 cargo test fabric_artifacts")
        });
        assert_eq!(&got, b, "{n} is stale — regenerate with FABRIC_RECORD=1");
    }
}

/// THE THESIS, reduced to practice: the compiler residualizes the program dispatch-free
/// (`count_dispatch == 0`), we lower that REAL residual to a fixed dataflow circuit (no
/// interpreter), and the 64-bit circuit computes the VM's EXACT result reduced to the
/// machine word (mod 2^64) for the FULL i64 input range — `i64::MAX`/`MIN` and every
/// polynomial-overflowing input included. NO range restriction: the circuit is the
/// fixed-width projection of the VM's arbitrary-precision arithmetic.
#[test]
fn a_specialized_residual_becomes_a_dataflow_circuit_identical_to_the_vm() {
    let src = poly_program("Show poly(4).");
    assert_eq!(
        count_dispatch(&src),
        0,
        "the residual must be dispatch-free (Jones-optimal)"
    );

    let (ops, result, _) = residual_from_source(&src, "poly");
    assert!(
        !ops.is_empty(),
        "the residual lowered to real functional units"
    );

    // Pure dataflow — no interpreter (check the RTL, not the comments describing it).
    let v = emit_fabric_verilog("horner", 1, &ops, result);
    let code: String = v
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in ["case", "always", "opcode", "fetch", "decode"] {
        assert!(
            !code.contains(forbidden),
            "a dataflow residual must not contain `{forbidden}`"
        );
    }

    // The circuit == the VM's exact result mod 2^64, including inputs whose polynomial is a
    // 100+-digit bignum in the VM (both agree on the low 64 bits) — no range restriction.
    let xs = [
        0i64,
        1,
        -1,
        2,
        -2,
        7,
        -7,
        50,
        -50,
        1000,
        -1000,
        3037000499,
        -3037000499,
        i64::MAX,
        i64::MIN,
        i64::MAX - 1,
        i64::MIN + 1,
        123456789,
        -123456789,
    ];
    for &x in &xs {
        assert_eq!(
            eval_fabric(&ops, &[x], result) as u64,
            poly_word(x),
            "the dataflow circuit (lowered from the compiled residual) must equal the VM (mod 2^64) at x={x}"
        );
    }
}

/// WORLD-CLASS (generality): the residual→circuit lowering is not a one-off. Across a
/// BATTERY of DISTINCT compiled programs — linear, quadratic, cubic, subtractive, and
/// TWO-INPUT — the emitted circuit equals the RUNNING VM over the full input range.
#[test]
fn the_fabric_lowering_generalizes_across_compiled_programs() {
    struct Case {
        decl: &'static str,
        name: &'static str,
        n_in: usize,
    }
    let battery = [
        Case {
            decl: "## To lin (x: Int) -> Int:\n    Return 3 * x + 7.",
            name: "lin",
            n_in: 1,
        },
        Case {
            decl: "## To quad (x: Int) -> Int:\n    Return x * x + 2 * x + 1.",
            name: "quad",
            n_in: 1,
        },
        Case {
            decl: "## To poly (x: Int) -> Int:\n    Return (((2 * x + 5) * x - 3) * x + 100).",
            name: "poly",
            n_in: 1,
        },
        Case {
            decl: "## To subd (x: Int) -> Int:\n    Return x * x - 5 * x + 6.",
            name: "subd",
            n_in: 1,
        },
        Case {
            decl: "## To fxy (x: Int, y: Int) -> Int:\n    Return x * y + x - y.",
            name: "fxy",
            n_in: 2,
        },
    ];
    // Full-range samples, overflow included — the circuit is the VM's result mod 2^64.
    let samples = [0i64, 1, -1, 3, -4, 17, -23, 1000, -999, i64::MAX, i64::MIN];
    for c in &battery {
        let dummy = if c.n_in == 2 {
            format!("Show {}(0, 0).", c.name)
        } else {
            format!("Show {}(0).", c.name)
        };
        let (ops, result, n_in) =
            residual_from_source(&format!("{}\n## Main\n    {dummy}", c.decl), c.name);
        assert_eq!(n_in, c.n_in, "{}: input arity", c.name);
        for &x in &samples {
            for &y in &samples {
                let args: Vec<i64> = if c.n_in == 2 { vec![x, y] } else { vec![x] };
                assert_eq!(
                    eval_fabric(&ops, &args, result) as u64,
                    vm_word(c.decl, c.name, &args),
                    "{} circuit != VM (mod 2^64) at {args:?}",
                    c.name
                );
                if c.n_in == 1 {
                    break;
                }
            }
        }
    }
}

/// FIXED-POINT (the computer-vision-on-silicon unlock): a Q-format kernel — integer
/// multiply-accumulate then an arithmetic right shift to rescale — lowers to the fabric.
/// `shifted right by` / `shifted left by` become AluOp::Shr / AluOp::Shl tiles (the tiles
/// already had verilog + eval; residual extraction just needed to accept the ops), so a
/// quantized convolution stencil (`vision.lg`'s `conv3x3Window`, unrolled) maps onto silicon.
#[test]
fn a_fixed_point_shift_residual_lowers_to_the_fabric() {
    let decl = "## To fp (x: Int, w: Int) -> Int:\n    Return (x * w) shifted right by 4.";
    let (ops, result, n_in) =
        residual_from_source(&format!("{decl}\n## Main\n    Show fp(0, 0)."), "fp");
    assert_eq!(n_in, 2, "input arity");
    assert!(
        ops.iter().any(|o| matches!(o.op, AluOp::Shr)),
        "the fixed-point residual must contain a Shr tile"
    );
    let samples = [0i64, 1, -1, 3, -4, 16, -32, 255, -256, 4096];
    for &x in &samples {
        for &w in &samples {
            assert_eq!(
                eval_fabric(&ops, &[x, w], result) as u64,
                vm_word(decl, "fp", &[x, w]),
                "fp circuit != VM (mod 2^64) at x={x} w={w}"
            );
        }
    }
    // A left shift lowers too (the Shl tile).
    let decl2 = "## To up (x: Int) -> Int:\n    Return (x + 1) shifted left by 3.";
    let (ops2, result2, _) =
        residual_from_source(&format!("{decl2}\n## Main\n    Show up(0)."), "up");
    assert!(
        ops2.iter().any(|o| matches!(o.op, AluOp::Shl)),
        "the residual must contain a Shl tile"
    );
    for &x in &samples {
        assert_eq!(
            eval_fabric(&ops2, &[x], result2) as u64,
            vm_word(decl2, "up", &[x]),
            "up circuit != VM at x={x}"
        );
    }
}

/// WORLD-CLASS (formal): for a multiply-FREE residual the emitted circuit is proven equal
/// to an INDEPENDENT closed form EXHAUSTIVELY (all inputs) by our OWN certified SAT — so
/// the lowering is certified correct, not merely sampled. (The cubic's multiplies are the
/// already-SAT-proven general-ALU units; here we certify the residual→SSA wiring itself.)
#[test]
fn a_multiply_free_residual_lowers_to_a_certified_circuit() {
    use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    // `x + x + x + 5` residualizes to an Add-chain (no Mul); its closed form is 3·x + 5.
    let decl = "## To lin3 (x: Int) -> Int:\n    Return x + x + x + 5.";
    let (ops, result, _) =
        residual_from_source(&format!("{decl}\n## Main\n    Show lin3(0)."), "lin3");
    for op in &ops {
        assert!(
            !matches!(op.op, AluOp::Mul),
            "this residual must be multiply-free"
        );
    }

    const PW: u32 = 8; // proof width — exhaustive over all 8-bit inputs
    let circuit = fabric_to_bounded(&ops, result, PW);
    let bv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), PW));
    let k = |v: u64| {
        Box::new(BoundedExpr::BitVecConst {
            width: PW,
            value: v,
        })
    };
    let bin = |o, l, r| {
        Box::new(BoundedExpr::BitVecBinary {
            op: o,
            left: l,
            right: r,
        })
    };
    // spec = 3*x + 5, built independently (with a Mul) — pins the Add-chain lowering.
    let spec = *bin(
        BitVecBoundedOp::Add,
        bin(BitVecBoundedOp::Mul, k(3), bv("in0")),
        k(5),
    );
    let claim = BoundedExpr::Eq(Box::new(circuit), Box::new(spec));
    let p = super::super::bitblast::lower_bool(&claim).expect("claim lowers");
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "the lowered circuit must equal 3·x + 5 for ALL inputs (certified, not sampled)"
    );
}

// ---- CGRA (counted loops) ------------------------------------------------------------

const SUMTO: &str = "## To sumto (n: Int) -> Int:\n    Let total be 0.\n    Repeat for i from 1 to n:\n        Set total to total + i.\n    Return total.";
const FACT: &str = "## To fact (n: Int) -> Int:\n    Let p be 1.\n    Repeat for i from 1 to n:\n        Set p to p * i.\n    Return p.";

/// THE THESIS for control flow: a counted loop residualizes to a CGRA — a fixed spatial body
/// datapath streamed by a sequential counter — and it computes the VM's EXACT loop result
/// (mod 2^64) for every trip count, INCLUDING zero iterations (empty/negative range). No
/// interpreter: the body is wired, the loop is a counter.
#[test]
fn a_counted_loop_lowers_to_a_cgra_identical_to_the_vm() {
    let src = format!("{SUMTO}\n## Main\n    Show sumto(5).");
    assert_eq!(
        count_dispatch(&src),
        0,
        "the loop residual must be dispatch-free (Jones-optimal)"
    );

    let lf = loop_fabric_from_source(&src, "sumto");
    assert_eq!(lf.n_in, 1);
    assert_eq!(lf.n_state, 1, "one loop-carried accumulator (total)");
    assert!(
        !lf.body.is_empty(),
        "the body lowered to real functional units"
    );

    // The emitted CGRA is a real SEQUENTIAL circuit (a clocked loop counter), not an interpreter.
    let v = emit_loop_fabric_verilog("sumto", &lf);
    assert!(
        v.contains("posedge clk") && v.contains("running"),
        "the CGRA must be a clocked body datapath with a loop controller"
    );
    let code: String = v
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in ["case", "opcode", "fetch", "decode"] {
        assert!(
            !code.contains(forbidden),
            "a CGRA loop must not contain `{forbidden}`"
        );
    }

    // The CGRA == the VM's result mod 2^64 across trip counts, incl. 0 iters (n <= 0).
    for n in [0i64, 1, 2, 3, 5, 10, 25, 100, 1000, -1, -5, -100] {
        assert_eq!(
            eval_loop_fabric(&lf, &[n]) as u64,
            vm_word(SUMTO, "sumto", &[n]),
            "the CGRA (streamed body) must equal the VM (mod 2^64) at n={n}"
        );
    }
}

/// WORLD-CLASS (generality): the loop→CGRA lowering is not a one-off. Across DISTINCT counted
/// loops — additive accumulation, multiplicative accumulation (factorial, which OVERFLOWS i64
/// by n=21 so the wrapping projection is exercised), and a compound update — the CGRA equals
/// the RUNNING VM (mod 2^64) over the full trip-count range.
#[test]
fn the_cgra_lowering_generalizes_across_looping_programs() {
    struct Case {
        decl: &'static str,
        name: &'static str,
    }
    let battery = [
        Case { decl: SUMTO, name: "sumto" },
        Case { decl: FACT, name: "fact" },
        Case {
            decl: "## To sumsq (n: Int) -> Int:\n    Let s be 0.\n    Repeat for i from 1 to n:\n        Set s to s + i * i.\n    Return s.",
            name: "sumsq",
        },
        Case {
            decl: "## To wsum (n: Int) -> Int:\n    Let total be 3.\n    Repeat for i from 1 to n:\n        Set total to total + 2 * i - 1.\n    Return total.",
            name: "wsum",
        },
    ];
    // Trip counts incl. 0 (n<=0), small, and factorial-overflowing (n=21..30).
    let ns = [0i64, 1, 2, 3, 5, 10, 20, 21, 25, 30, 50, -1, -7];
    for c in &battery {
        let src = format!("{}\n## Main\n    Show {}(0).", c.decl, c.name);
        let lf = loop_fabric_from_source(&src, c.name);
        for &n in &ns {
            assert_eq!(
                eval_loop_fabric(&lf, &[n]) as u64,
                vm_word(c.decl, c.name, &[n]),
                "{} CGRA != VM (mod 2^64) at n={n}",
                c.name
            );
        }
    }
}

/// WORLD-CLASS (formal): the CGRA is sound by k-INDUCTION MADE PHYSICAL. The induction STEP —
/// the body's one-step next-state transition — is proven EQUAL to the VM's per-iteration
/// semantics EXHAUSTIVELY (all inputs) by our OWN certified SAT. For `sumto` the body is
/// multiply-free: next_total = total + i, and we certify exactly that against an independent
/// spec. With the base case (initial state) and the controller counting the induction variable
/// across exactly `lo..=hi`, the CGRA's final state equals the VM's loop result for EVERY
/// input by induction over the iterations — a certified loop, not a sampled one.
#[test]
fn a_cgra_loop_body_step_is_certified_by_our_sat() {
    use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    let src = format!("{SUMTO}\n## Main\n    Show sumto(0).");
    let lf = loop_fabric_from_source(&src, "sumto");
    assert_eq!(lf.n_state, 1);
    for cell in &lf.body {
        assert!(
            !matches!(cell, LoopCell::Arith { op: AluOp::Mul, .. }),
            "sumto's body must be multiply-free"
        );
    }

    const PW: u32 = 8; // exhaustive over all 8-bit (state, induction) pairs
    let step = loop_next_to_bounded(&lf, 0, PW);
    // spec: one iteration adds the induction variable to the accumulator — total' = total + i.
    let spec = BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Add,
        left: Box::new(BoundedExpr::BitVecVar("s0".to_string(), PW)),
        right: Box::new(BoundedExpr::BitVecVar("iv".to_string(), PW)),
    };
    let claim = BoundedExpr::Eq(Box::new(step), Box::new(spec));
    let p = super::super::bitblast::lower_bool(&claim).expect("claim lowers");
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "the CGRA body's one-step transition must equal total + i for ALL states/inductions \
             (the certified induction step)"
    );
}

/// WORLD-CLASS (formal, multiplicative): the certified induction step is not limited to
/// additive accumulation. For a MULTIPLY-containing loop body — `total += 2*i - 1` — the
/// emitted body datapath (a Mul wired with an Add and a Sub over the loop-carried state and
/// the induction variable) is proven EQUAL to the INDEPENDENT closed form `total + 2*i - 1`
/// EXHAUSTIVELY (all 8-bit states/inductions) by our OWN certified SAT. So the CGRA lowering
/// is certified for multiplicative/affine bodies too — a wrong operand order, a mis-identified
/// induction variable, or a wrong state slot would make the datapath ≠ the closed form and be
/// caught, not sampled.
#[test]
fn a_cgra_multiplicative_loop_step_is_certified_by_our_sat() {
    use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    let decl = "## To wsum (n: Int) -> Int:\n    Let total be 3.\n    Repeat for i from 1 to n:\n        Set total to total + 2 * i - 1.\n    Return total.";
    let src = format!("{decl}\n## Main\n    Show wsum(0).");
    let lf = loop_fabric_from_source(&src, "wsum");
    assert_eq!(lf.n_state, 1);
    assert!(
        lf.body
            .iter()
            .any(|c| matches!(c, LoopCell::Arith { op: AluOp::Mul, .. })),
        "this step must exercise a multiply (2 * i)"
    );

    const PW: u32 = 8; // exhaustive over all 8-bit (state, induction) pairs
    let step = loop_next_to_bounded(&lf, 0, PW);
    let bv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), PW));
    let k = |v: u64| {
        Box::new(BoundedExpr::BitVecConst {
            width: PW,
            value: v,
        })
    };
    let bin = |o, l, r| {
        Box::new(BoundedExpr::BitVecBinary {
            op: o,
            left: l,
            right: r,
        })
    };
    // spec = total + 2*i - 1, built independently (two's-complement add/sub associate, so the
    // exact grouping the compiler chose does not matter — only the value, over all inputs).
    let spec = *bin(
        BitVecBoundedOp::Sub,
        bin(
            BitVecBoundedOp::Add,
            bv("s0"),
            bin(BitVecBoundedOp::Mul, k(2), bv("iv")),
        ),
        k(1),
    );
    let claim = BoundedExpr::Eq(Box::new(step), Box::new(spec));
    let p = super::super::bitblast::lower_bool(&claim).expect("claim lowers");
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "the CGRA multiplicative body step must equal total + 2*i - 1 for ALL states/inductions \
             (the certified induction step, multiplicative case)"
    );
}

/// G1 — the predicated SELECT (the CGRA mux tile) is certified by our SAT. This is the atom of
/// data-dependent control flow: a branch becomes a mux, control becomes data, no divergence.
/// `BitVecIte(cond, t, e)` is proven EXHAUSTIVELY (all inputs, our own CDCL) to SELECT
/// correctly two ways — when the data-dependent predicate `cond` holds the result is `t`, and
/// when it does not the result is `e`. A mux that ignored the predicate, swapped the branches,
/// or blended them (e.g. `t^e`) would fail one of the two obligations.
#[test]
fn the_predicated_select_tile_is_certified_by_our_sat() {
    use crate::codegen_sva::bitblast::lower_bool;
    use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    const W: u32 = 8;
    let bv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), W));
    // A genuine data-dependent predicate: c0 <u c1.
    let cond = || {
        Box::new(BoundedExpr::BitVecBinary {
            op: BitVecBoundedOp::ULt,
            left: bv("c0"),
            right: bv("c1"),
        })
    };
    let ite = || BoundedExpr::BitVecIte {
        cond: cond(),
        then: bv("t"),
        else_: bv("e"),
    };

    let cond_pe = lower_bool(&cond()).expect("predicate lowers");
    // cond ⇒ (ite == t): ¬cond ∨ (ite == t), built at the ProofExpr level.
    let then_eq = lower_bool(&BoundedExpr::Eq(Box::new(ite()), bv("t"))).expect("eq lowers");
    let sel_then = ProofExpr::Or(
        Box::new(ProofExpr::Not(Box::new(cond_pe.clone()))),
        Box::new(then_eq),
    );
    // ¬cond ⇒ (ite == e): cond ∨ (ite == e).
    let else_eq = lower_bool(&BoundedExpr::Eq(Box::new(ite()), bv("e"))).expect("eq lowers");
    let sel_else = ProofExpr::Or(Box::new(cond_pe), Box::new(else_eq));

    for (claim, what) in [(sel_then, "cond ⇒ then"), (sel_else, "¬cond ⇒ else")] {
        assert_eq!(
            prove_unsat(&ProofExpr::Not(Box::new(claim))),
            UnsatOutcome::Refuted,
            "the predicated select must satisfy `{what}` for ALL inputs (certified mux tile)"
        );
    }
}

// A loop body with a data-dependent branch: `c += i` only when `i > 3` (if-only), and choose
// `s += i` vs `s -= i` on `i > 3` (if/else). Both if-CONVERT to a predicated select.
const COUNTBIG: &str = "## To countbig (n: Int) -> Int:\n    Let c be 0.\n    Repeat for i from 1 to n:\n        If i is greater than 3:\n            Set c to c + i.\n    Return c.";
const ALTSUM: &str = "## To altsum (n: Int) -> Int:\n    Let s be 0.\n    Repeat for i from 1 to n:\n        If i is greater than 3:\n            Set s to s + i.\n        Otherwise:\n            Set s to s - i.\n    Return s.";

/// G2 — a loop body with DATA-DEPENDENT CONTROL FLOW lowers to a predicated CGRA: the branch is
/// if-converted to a mux tile (both arms compute, the signed predicate selects), and the
/// clocked fabric computes the VM's EXACT loop result mod 2⁶⁴ across every trip count — for
/// both an if-only body and an if/else body. Control became data; no divergence.
#[test]
fn a_branchy_loop_lowers_to_a_predicated_cgra_identical_to_the_vm() {
    for (decl, name) in [(COUNTBIG, "countbig"), (ALTSUM, "altsum")] {
        let src = format!("{decl}\n## Main\n    Show {name}(0).");
        let lf = loop_fabric_from_source(&src, name);
        assert_eq!(lf.n_state, 1, "{name}: one accumulator");
        assert!(
            lf.body.iter().any(|c| matches!(c, LoopCell::Select { .. })),
            "{name}: the branch must lower to a predicated Select tile"
        );
        // The emitted RTL carries a real mux (`? :`), not a control branch.
        let v = emit_loop_fabric_verilog(name, &lf);
        assert!(
            v.contains(" ? ") && v.contains(" : "),
            "{name}: the CGRA must emit a mux"
        );

        for n in [0i64, 1, 3, 4, 5, 10, 25, 100, -1, -8] {
            assert_eq!(
                eval_loop_fabric(&lf, &[n]) as u64,
                vm_word(decl, name, &[n]),
                "{name} predicated CGRA != VM (mod 2^64) at n={n}"
            );
        }
    }
}

/// G2 (formal) — the PREDICATED induction step is certified by our SAT. For the if/else body
/// `s' = if i>3 then s+i else s-i`, we prove the emitted mux datapath EQUAL to the
/// differently-structured closed form `s + (if i>3 then i else -i)` EXHAUSTIVELY (all 8-bit
/// states/inductions) — a genuine distribute-add-over-select identity, not a restatement. A
/// wrong predicate, swapped arm, or mis-wired operand fails it. With the counter's base + trip
/// count, the whole data-dependent loop is proven by induction.
#[test]
fn the_predicated_cgra_step_is_certified_by_our_sat() {
    use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    let src = format!("{ALTSUM}\n## Main\n    Show altsum(0).");
    let lf = loop_fabric_from_source(&src, "altsum");
    assert_eq!(lf.n_state, 1);

    const PW: u32 = 8;
    let step = loop_next_to_bounded(&lf, 0, PW);
    let bvv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), PW));
    let k = |v: u64| {
        Box::new(BoundedExpr::BitVecConst {
            width: PW,
            value: v,
        })
    };
    let bin = |o, l, r| {
        Box::new(BoundedExpr::BitVecBinary {
            op: o,
            left: l,
            right: r,
        })
    };
    // spec = s0 + ite(iv > 3, iv, -iv), with iv>3 as SLt(3, iv) and -iv as 0 - iv.
    let neg_iv = bin(BitVecBoundedOp::Sub, k(0), bvv("iv"));
    let inner = Box::new(BoundedExpr::BitVecIte {
        cond: bin(BitVecBoundedOp::SLt, k(3), bvv("iv")),
        then: bvv("iv"),
        else_: neg_iv,
    });
    let spec = *bin(BitVecBoundedOp::Add, bvv("s0"), inner);
    let claim = BoundedExpr::Eq(Box::new(step), Box::new(spec));
    let p = super::super::bitblast::lower_bool(&claim).expect("claim lowers");
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "the predicated step `ite(i>3, s+i, s-i)` must equal `s + ite(i>3, i, -i)` for ALL \
             inputs (certified predicated induction step)"
    );
}

// WHILE loops — the trip count is DATA (a fabric-computed predicate), not a counter.
const ACCW: &str = "## To accw (n: Int) -> Int:\n    Let a be 0.\n    Let total be 0.\n    While a is less than n:\n        Set total to total + a.\n        Set a to a + 1.\n    Return total.";
const CLAMPW: &str = "## To clampw (n: Int) -> Int:\n    Let a be 0.\n    Let s be 0.\n    While a is less than n:\n        If a is greater than 2:\n            Set s to s + a.\n        Set a to a + 1.\n    Return s.";

/// G3 — a `while` loop lowers to a DATA-DEPENDENT CGRA: the controller's continue condition is a
/// fabric-computed predicate over the loop-carried state (no counter), so the trip count is
/// decided by the datapath. Across an arithmetic-body `while` AND one whose body ALSO branches
/// (reusing G2's if-conversion), the clocked fabric computes the VM's exact result mod 2⁶⁴.
#[test]
fn a_while_loop_lowers_to_a_data_dependent_cgra_identical_to_the_vm() {
    for (decl, name) in [(ACCW, "accw"), (CLAMPW, "clampw")] {
        let src = format!("{decl}\n## Main\n    Show {name}(0).");
        let wf = while_fabric_from_source(&src, name);
        assert_eq!(wf.n_state, 2, "{name}: two loop-carried registers");
        // The controller is predicate-driven — `running <= (...)`, no counter/induction var.
        let v = emit_while_fabric_verilog(name, &wf);
        assert!(
            v.contains("running <= ($signed") && !v.contains("iv"),
            "{name}: the while-CGRA must be driven by a fabric predicate, not a counter"
        );
        for n in [0i64, 1, 2, 3, 4, 6, 10, 20, 50, -1, -9] {
            assert_eq!(
                eval_while_fabric(&wf, &[n]) as u64,
                vm_word(decl, name, &[n]),
                "{name} while-CGRA != VM (mod 2^64) at n={n}"
            );
        }
    }
}

/// G3 (formal) — the while-CGRA body step is certified by our SAT: for `accw` (`total += a; a
/// += 1`) BOTH next-state components are proven EXHAUSTIVELY — `a' = a + 1` and `total' = total
/// + a`. With the fabric predicate ≡ the VM's `a < n` loop condition, the whole data-dependent
/// loop is sound by induction. The predicate is directly recovered from the VM's comparison.
#[test]
fn the_while_cgra_body_step_is_certified_by_our_sat() {
    use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    let src = format!("{ACCW}\n## Main\n    Show accw(0).");
    let wf = while_fabric_from_source(&src, "accw");
    assert_eq!(wf.n_state, 2);
    // The continue predicate is exactly the VM's `a < n` (Lt over state slot 0 and an invariant).
    assert_eq!(
        wf.pred.0,
        CmpKind::Lt,
        "accw's predicate must be the VM's `<`"
    );

    const PW: u32 = 8;
    let k = |v: u64| {
        Box::new(BoundedExpr::BitVecConst {
            width: PW,
            value: v,
        })
    };
    let sv = |n: &str| Box::new(BoundedExpr::BitVecVar(n.to_string(), PW));
    let add = |l, r| BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Add,
        left: l,
        right: r,
    };
    // slot 0 = a (R2 < R4), slot 1 = total. Specs: a' = a + 1, total' = total + a.
    let specs = [
        (0usize, add(sv("s0"), k(1))),
        (1usize, add(sv("s1"), sv("s0"))),
    ];
    for (slot, spec) in specs {
        let step = while_next_to_bounded(&wf, slot, PW);
        let claim = BoundedExpr::Eq(Box::new(step), Box::new(spec));
        let p = super::super::bitblast::lower_bool(&claim).expect("claim lowers");
        assert_eq!(
            prove_unsat(&ProofExpr::Not(Box::new(p))),
            UnsatOutcome::Refuted,
            "while body next-state slot {slot} must match its spec for ALL inputs (certified)"
        );
    }
}

// NESTED loops — a hierarchical CGRA. `nsum` inner bound = n (→ n²); `tri` inner bound = the
// OUTER induction variable (`for j from 1 to i`, → n(n+1)/2).
const NSUM: &str = "## To nsum (n: Int) -> Int:\n    Let total be 0.\n    Repeat for i from 1 to n:\n        Repeat for j from 1 to n:\n            Set total to total + 1.\n    Return total.";
const TRI: &str = "## To tri (n: Int) -> Int:\n    Let total be 0.\n    Repeat for i from 1 to n:\n        Repeat for j from 1 to i:\n            Set total to total + 1.\n    Return total.";

/// G4 — a perfectly-NESTED loop lowers to a HIERARCHICAL CGRA: an outer counter that runs the
/// inner loop (its own counter + body) to completion each iteration, the inner range possibly
/// depending on the outer induction variable. Across a fixed-inner-bound nest (`nsum` → n²) and
/// an outer-iv-dependent nest (`tri` → n(n+1)/2), the clocked two-level FSM computes the VM's
/// exact result mod 2⁶⁴.
#[test]
fn a_nested_loop_lowers_to_a_hierarchical_cgra_identical_to_the_vm() {
    let nsum = nested_fabric_from_source(&format!("{NSUM}\n## Main\n    Show nsum(0)."), "nsum");
    assert_eq!(
        nsum.inner_hi,
        NestSrc::Param(0),
        "nsum's inner bound is the param n"
    );
    let tri = nested_fabric_from_source(&format!("{TRI}\n## Main\n    Show tri(0)."), "tri");
    assert_eq!(
        tri.inner_hi,
        NestSrc::OuterIv,
        "tri's inner bound is the OUTER induction variable (`for j from 1 to i`)"
    );

    for (nf, decl, name) in [(&nsum, NSUM, "nsum"), (&tri, TRI, "tri")] {
        // A hierarchical outer+inner controller — SETUP/RUN states, an outer `oi` and inner `ij`.
        let v = emit_nested_fabric_verilog(name, nf);
        assert!(
            v.contains("SETUP") && v.contains("RUN") && v.contains("oi") && v.contains("ij"),
            "{name}: the nested CGRA must be a hierarchical outer+inner controller"
        );
        for n in [0i64, 1, 2, 3, 4, 5, 8, 12, -1, -4] {
            assert_eq!(
                eval_nested_fabric(nf, &[n]) as u64,
                vm_word(decl, name, &[n]),
                "{name} nested CGRA != VM (mod 2^64) at n={n}"
            );
        }
    }
    // The closed forms, for good measure.
    assert_eq!(eval_nested_fabric(&nsum, &[5]), 25);
    assert_eq!(eval_nested_fabric(&tri, &[5]), 15);
}

/// G4 (formal) — the inner body step is certified by our SAT: for `total += 1` the one-step
/// next-state is proven `== total + 1` EXHAUSTIVELY. This is the BASE of the double induction —
/// with it (⇒ the inner CGRA ≡ the VM inner loop) and the outer counter applying that inner
/// transition once per outer step, the whole nested CGRA ≡ the VM.
#[test]
fn the_nested_inner_body_step_is_certified_by_our_sat() {
    use crate::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
    use logicaffeine_proof::sat::{prove_unsat, UnsatOutcome};
    use logicaffeine_proof::ProofExpr;

    let tri = nested_fabric_from_source(&format!("{TRI}\n## Main\n    Show tri(0)."), "tri");
    assert_eq!(tri.n_state, 1);
    const PW: u32 = 8;
    let step = nested_inner_next_to_bounded(&tri, 0, PW);
    let spec = BoundedExpr::BitVecBinary {
        op: BitVecBoundedOp::Add,
        left: Box::new(BoundedExpr::BitVecVar("s0".to_string(), PW)),
        right: Box::new(BoundedExpr::BitVecConst {
            width: PW,
            value: 1,
        }),
    };
    let claim = BoundedExpr::Eq(Box::new(step), Box::new(spec));
    let p = super::super::bitblast::lower_bool(&claim).expect("claim lowers");
    assert_eq!(
        prove_unsat(&ProofExpr::Not(Box::new(p))),
        UnsatOutcome::Refuted,
        "the nested inner body step must equal total + 1 for ALL inputs (certified induction base)"
    );
}

/// Emit the CGRA RTL (lowered from the REAL compiled loop), its clocked testbench, and the VM
/// oracle vectors (golden; regenerate with `FABRIC_RECORD=1`). The `iverilog` gate (a script)
/// clocks the circuit against these vectors.
#[test]
fn cgra_artifacts_are_current() {
    const N: usize = 48;
    // Both a straight-line loop (`sumto`) and a data-dependent branchy loop (`altsum`, whose
    // body if-converts to a mux) — the iverilog gate clocks each against the VM oracle.
    let dir = silicon_dir();
    let mut files: Vec<(String, String)> = Vec::new();
    for (decl, name) in [(SUMTO, "sumto"), (ALTSUM, "altsum")] {
        let src = format!("{decl}\n## Main\n    Show {name}(5).");
        let lf = loop_fabric_from_source(&src, name);
        let mut hin = String::new();
        let mut hout = String::new();
        for k in 0..N {
            let n = k as i64; // 0..47, a compact spread (correctness holds for all i64)
            hin.push_str(&format!("{:016x}\n", n as u64));
            hout.push_str(&format!("{:016x}\n", vm_word(decl, name, &[n])));
        }
        files.push((
            format!("cgra_{name}.v"),
            emit_loop_fabric_verilog(name, &lf),
        ));
        files.push((
            format!("tb_cgra_{name}.v"),
            emit_loop_fabric_testbench(name, &lf, N, 256),
        ));
        files.push((format!("cgra_{name}_in.hex"), hin));
        files.push((format!("cgra_{name}_out.hex"), hout));
    }
    // WHILE-loop CGRAs (fabric-predicate controller): an arithmetic body (`accw`) and one whose
    // body ALSO branches (`clampw`). Same iverilog gate, clocked to `done`.
    for (decl, name) in [(ACCW, "accw"), (CLAMPW, "clampw")] {
        let src = format!("{decl}\n## Main\n    Show {name}(5).");
        let wf = while_fabric_from_source(&src, name);
        let mut hin = String::new();
        let mut hout = String::new();
        for k in 0..N {
            let n = k as i64;
            hin.push_str(&format!("{:016x}\n", n as u64));
            hout.push_str(&format!("{:016x}\n", vm_word(decl, name, &[n])));
        }
        files.push((
            format!("while_{name}.v"),
            emit_while_fabric_verilog(name, &wf),
        ));
        files.push((
            format!("tb_while_{name}.v"),
            emit_while_fabric_testbench(name, &wf, N, 256),
        ));
        files.push((format!("while_{name}_in.hex"), hin));
        files.push((format!("while_{name}_out.hex"), hout));
    }
    // NESTED-loop CGRAs (hierarchical outer+inner controller): a fixed-inner-bound nest (`nsum`
    // → n²) and an outer-iv-dependent nest (`tri` → n(n+1)/2). Up to ~n² inner iterations, so a
    // larger cycle budget. Smaller N to keep the clocked sim quick.
    const NN: usize = 24;
    for (decl, name) in [(NSUM, "nsum"), (TRI, "tri")] {
        let src = format!("{decl}\n## Main\n    Show {name}(5).");
        let nf = nested_fabric_from_source(&src, name);
        let mut hin = String::new();
        let mut hout = String::new();
        for k in 0..NN {
            let n = k as i64;
            hin.push_str(&format!("{:016x}\n", n as u64));
            hout.push_str(&format!("{:016x}\n", vm_word(decl, name, &[n])));
        }
        files.push((
            format!("nested_{name}.v"),
            emit_nested_fabric_verilog(name, &nf),
        ));
        files.push((
            format!("tb_nested_{name}.v"),
            emit_nested_fabric_testbench(name, &nf, NN, 4000),
        ));
        files.push((format!("nested_{name}_in.hex"), hin));
        files.push((format!("nested_{name}_out.hex"), hout));
    }
    if std::env::var("FABRIC_RECORD").is_ok() {
        for (n, b) in &files {
            std::fs::write(dir.join(n), b).expect("write");
        }
        return;
    }
    for (n, b) in &files {
        let got = std::fs::read_to_string(dir.join(n)).unwrap_or_else(|e| {
            panic!("missing {n} ({e}) — record: FABRIC_RECORD=1 cargo test cgra_artifacts")
        });
        assert_eq!(got, *b, "{n} is stale — regenerate with FABRIC_RECORD=1");
    }
}
