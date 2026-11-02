//! residual_pipeline_lock — the compiled residual circuit, pipelined: one result per clock at a short period.
//!
//! `fabric_horner_seq` hardened the Horner residual as one combinational stage between two registers, so its clock
//! was the delay of two chained 64-bit multiplies and three carry chains. `fabric_pipe` carry-save lowers the same
//! residual (multiplies and sums become 3:2 compressor trees resolved by Kogge-Stone adds, in the same op vocabulary),
//! cuts it into register stages under a depth budget, and emits the pipeline. This lock holds every link:
//!
//! - A: the lowering computes the residual, at 64 bits for every input tried, and holds no adder or multiplier.
//! - B: the lowering is exact on every input at every narrow width, by exhaustion under the bounded semantics;
//!   our SAT certifies it at 5 bits and refutes a broken one.
//! - C: residuals of every shape, generated, lower exactly and stream exactly through their pipelines.
//! - D: the schedule respects dataflow and its budget, and each stage boundary wastes less than one op of depth.
//! - R: the placement holds the fewest live register bits: never more than the earliest or the latest placement,
//!   and exactly the minimum on every small residual enumerated in full.
//! - E: the cycle model streams one result per clock equal to the running VM, `latency` clocks later.
//! - F: the emitted RTL streams the VM's recorded results under iverilog.
//! - H: only computed values are registered; wiring is rebuilt at its readers, and the register count is exact.
//! - S: a decision fabric (arithmetic and compare-selects) lowers into the same op vocabulary — every compare to the
//!   mask of its outcome, exact on every pair of values at every narrow width — computes what the fabric computes,
//!   and streams one result per clock through its pipeline; a tile with no combinational form is refused.
//! - P: a pipeline reading its ports (`Inputs::Ports`, for an upstream that registers what it presents) is the same
//!   pipeline one clock sooner, without one register per input it reads, in the cycle model and under iverilog.
//! - T: the decision hoisted (`select_hoist`), its masks shared and its sums folded into the compares that read them
//!   (`SumSharing::Fold`) — the lowering for standard cells — computes the compiled decision: folding is exact on
//!   residuals of every shape at every input of a narrow width, our SAT certifies the whole chain against the compiled
//!   fabric at 5 bits and refutes a broken one, every op is emitted once, and no carry chain waits on another.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test residual_pipeline_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use logicaffeine_compile::codegen_sva::bitblast::{assert_not, lower_bool, word_binary, word_const, word_var, words_equal, Word};
use logicaffeine_compile::codegen_sva::fabric_pipe::{
    carry_save_lower, carry_save_lower_with, csd_digits, FinalAdd, decision_ops, earliest_schedule, emit_fabric_pipelined_verilog,
    emit_horner_seq_verilog, horner_seq_pipeline, is_wiring, known_bits, latest_schedule, op_delay, schedule_pipeline,
    simulate_pipeline, Inputs, PipelineSchedule, SumSharing, DECISION_CHAIN_BUDGET, HORNER_SEQ_BUDGET,
};
use logicaffeine_compile::codegen_sva::select_hoist::{hoist_selects, reads_a_select};
use logicaffeine_compile::codegen_sva::fabric_rtl::{
    decision_fabric_from_source, eval_decision_fabric, eval_fabric, fabric_to_bounded, poly_program,
    residual_from_source, AluOp, CmpKind, DecisionFabric, FabricOp, LoopCell, Src, POLY_BODY,
};
use logicaffeine_compile::codegen_sva::sva_to_verify::{BitVecBoundedOp, BoundedExpr};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_proof::cnf::Cnf;
use logicaffeine_proof::sat::{prove_unsat, prove_unsat_certified_cnf, UnsatOutcome};
use logicaffeine_proof::ProofExpr;

fn silicon(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/silicon")
        .join(name)
}

fn horner() -> (Vec<FabricOp>, usize) {
    let (ops, result, n_in) = residual_from_source(&poly_program("Show poly(4)."), "poly");
    assert_eq!(n_in, 1, "the Horner residual reads one input");
    (ops, result)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn word(&mut self) -> i64 {
        match self.below(4) {
            0 => EDGES[self.below(EDGES.len())],
            1 => (self.next() as i64) >> self.below(64),
            _ => self.next() as i64,
        }
    }
}

const EDGES: [i64; 12] = [
    0,
    1,
    -1,
    2,
    -2,
    i64::MAX,
    i64::MIN,
    i64::MIN + 1,
    1 << 32,
    (1 << 32) - 1,
    0x5555_5555_5555_5555,
    -0x5555_5555_5555_5556,
];

/// The VM's exact result of `poly(x)` reduced to the machine word, as the fixed-width circuit computes it.
fn vm_poly_word(x: i64) -> i64 {
    let src = format!("## To poly (x: Int) -> Int:\n    Return {POLY_BODY}.\n## Main\n    Show poly({x}).");
    let out = vm_outcome(&src);
    assert!(out.error.is_none(), "poly({x}) VM error: {:?}", out.error);
    let s = out.output.trim();
    let (neg, digits) = s.strip_prefix('-').map_or((false, s), |d| (true, d));
    let mut acc: u64 = 0;
    for ch in digits.bytes() {
        assert!(ch.is_ascii_digit(), "non-digit in VM output {s:?}");
        acc = acc.wrapping_mul(10).wrapping_add((ch - b'0') as u64);
    }
    (if neg { acc.wrapping_neg() } else { acc }) as i64
}

fn read_hex_words(name: &str) -> Vec<i64> {
    std::fs::read_to_string(silicon(name))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
        .lines()
        .map(|l| u64::from_str_radix(l.trim(), 16).expect("hex word") as i64)
        .collect()
}

fn has_arithmetic(ops: &[FabricOp]) -> bool {
    ops.iter()
        .any(|o| matches!(o.op, AluOp::Add | AluOp::Sub | AluOp::Mul))
}

// ── A. the lowering computes the residual ───────────────────────────────────────────────────────────

#[test]
fn the_lowered_residual_computes_the_residual_at_every_input_tried() {
    let (ops, result) = horner();
    let (lowered, lresult) = carry_save_lower(&ops, result, 64);
    assert!(
        !has_arithmetic(&lowered),
        "the lowered Horner residual still holds an adder or multiplier"
    );
    assert!(
        ops.iter().filter(|o| o.op == AluOp::Mul).count() >= 2,
        "the residual being lowered carries its multiplies"
    );
    let mut rng = Rng(0x5EED_0001);
    let mut inputs: Vec<i64> = EDGES.to_vec();
    inputs.extend((0..20_000).map(|_| rng.word()));
    for x in inputs {
        assert_eq!(
            eval_fabric(&lowered, &[x], lresult),
            eval_fabric(&ops, &[x], result),
            "the lowered Horner residual differs from the residual at x = {x}"
        );
    }
}

#[test]
fn canonical_signed_digits_sum_to_their_constant_with_no_two_adjacent() {
    let mut rng = Rng(0x5EED_0002);
    let mut constants: Vec<i64> = EDGES.to_vec();
    constants.extend([3, 5, 7, 100, -3, -100, 0x7FFF, 0x0F0F_0F0F]);
    constants.extend((0..2_000).map(|_| rng.word()));
    for width in [8u32, 13, 32, 63, 64] {
        let mask: u64 = if width == 64 { u64::MAX } else { (1 << width) - 1 };
        for &c in &constants {
            let digits = csd_digits(c, width);
            let sum = digits.iter().fold(0u64, |acc, &(k, d)| {
                let term = 1u64 << k;
                if d > 0 {
                    acc.wrapping_add(term)
                } else {
                    acc.wrapping_sub(term)
                }
            });
            assert_eq!(sum & mask, (c as u64) & mask, "the digits of {c} at width {width} do not sum to it");
            assert!(
                digits.windows(2).all(|p| p[1].0 > p[0].0 + 1),
                "the digits of {c} at width {width} are not canonical: {digits:?}"
            );
            assert!(digits.iter().all(|&(k, d)| k < width && (d == 1 || d == -1)));
        }
    }
}

// ── B. our SAT proves the lowering at a narrow width ────────────────────────────────────────────────

fn certified_equal(left: &[FabricOp], lres: usize, right: &[FabricOp], rres: usize, width: u32) -> bool {
    let claim = BoundedExpr::Eq(
        Box::new(fabric_to_bounded(left, lres, width)),
        Box::new(fabric_to_bounded(right, rres, width)),
    );
    let p = lower_bool(&claim).expect("the equivalence claim lowers to a bounded formula");
    prove_unsat(&ProofExpr::Not(Box::new(p))) == UnsatOutcome::Refuted
}

/// `ops` at `width` bits as a gate DAG: each value blasted once into one word that every reader shares, where
/// `fabric_to_bounded` copies a shared value into each reader's tree.
fn ops_word(ops: &[FabricOp], result: usize, width: u32) -> Word {
    let mut vals: Vec<Word> = Vec::with_capacity(ops.len());
    for op in ops {
        let word = |s: Src, vals: &[Word]| match s {
            Src::In(k) => word_var(&format!("in{k}"), width),
            Src::Val(k) => vals[k].clone(),
            Src::Const(c) => word_const(width, c as u64),
        };
        let bop = match op.op {
            AluOp::Add => BitVecBoundedOp::Add,
            AluOp::Sub => BitVecBoundedOp::Sub,
            AluOp::Mul => BitVecBoundedOp::Mul,
            AluOp::And => BitVecBoundedOp::And,
            AluOp::Or => BitVecBoundedOp::Or,
            AluOp::Xor => BitVecBoundedOp::Xor,
            AluOp::Shl => BitVecBoundedOp::Shl,
            AluOp::Shr => BitVecBoundedOp::AShr,
            AluOp::UShr => BitVecBoundedOp::Shr,
        };
        vals.push(word_binary(bop, &word(op.a, &vals), &word(op.b, &vals)).expect("every op blasts"));
    }
    vals[result].clone()
}

/// [`certified_equal`] for circuits whose shared values make the tree exponential: the miter is clausified straight
/// from the gate DAG and its refutation certified.
fn certified_equal_dag(left: &[FabricOp], lres: usize, right: &[FabricOp], rres: usize, width: u32) -> bool {
    let miter = words_equal(&ops_word(left, lres, width), &ops_word(right, rres, width)).expect("the words are one width");
    let mut cnf = Cnf::new();
    assert_not(&miter, &mut cnf);
    prove_unsat_certified_cnf(cnf.num_vars(), cnf.clauses()).is_some()
}

/// The residual's value at datapath width `width`, the semantics `fabric_to_bounded` gives it: every op on
/// `width`-bit words, wrapping, a shift by `width` or more emptying the word (or filling it with the sign).
fn eval_width(ops: &[FabricOp], inputs: &[u64], result: usize, width: u32) -> u64 {
    let mask = if width == 64 { u64::MAX } else { (1u64 << width) - 1 };
    let sign = |v: u64| v >> (width - 1) & 1 == 1;
    let mut vals: Vec<u64> = Vec::with_capacity(ops.len());
    for op in ops {
        let get = |s: Src, vals: &[u64]| match s {
            Src::In(k) => inputs[k] & mask,
            Src::Val(k) => vals[k],
            Src::Const(c) => (c as u64) & mask,
        };
        let (a, b) = (get(op.a, &vals), get(op.b, &vals));
        let v = match op.op {
            AluOp::Add => a.wrapping_add(b),
            AluOp::Sub => a.wrapping_sub(b),
            AluOp::Mul => a.wrapping_mul(b),
            AluOp::And => a & b,
            AluOp::Or => a | b,
            AluOp::Xor => a ^ b,
            AluOp::Shl => if b >= width as u64 { 0 } else { a << b },
            AluOp::UShr => if b >= width as u64 { 0 } else { a >> b },
            AluOp::Shr => {
                let fill = if sign(a) { mask } else { 0 };
                if b >= width as u64 { fill } else { (a >> b) | (fill & !(mask >> b)) }
            }
        } & mask;
        vals.push(v);
    }
    vals[result]
}

#[test]
fn the_lowering_is_exact_on_every_input_at_every_narrow_width() {
    let (ops, result) = horner();
    for width in 2..=14u32 {
        let (lowered, lresult) = carry_save_lower(&ops, result, width);
        for x in 0..1u64 << width {
            assert_eq!(
                eval_width(&lowered, &[x], lresult, width),
                eval_width(&ops, &[x], result, width),
                "the {width}-bit lowering of the Horner residual differs at x = {x}"
            );
        }
    }
    let mac = "## To mac (a: Int, b: Int, c: Int) -> Int:\n    Return a * b - c * 3 + 7.\n## Main\n    Show mac(1, 2, 3).";
    let (mops, mresult, n_in) = residual_from_source(mac, "mac");
    assert_eq!(n_in, 3);
    for width in 2..=6u32 {
        let (mlow, mlres) = carry_save_lower(&mops, mresult, width);
        for word in 0..1u64 << (3 * width) {
            let m = (1u64 << width) - 1;
            let v = [word & m, (word >> width) & m, (word >> (2 * width)) & m];
            assert_eq!(
                eval_width(&mlow, &v, mlres, width),
                eval_width(&mops, &v, mresult, width),
                "the {width}-bit lowering of a·b − 3c + 7 differs at {v:?}"
            );
        }
    }
}

#[test]
fn the_width_exact_evaluator_is_the_bounded_semantics_at_64_bits() {
    let (ops, result) = horner();
    let (lowered, lresult) = carry_save_lower(&ops, result, 64);
    let mut rng = Rng(0x5EED_000A);
    for _ in 0..2_000 {
        let x = rng.word();
        assert_eq!(eval_width(&ops, &[x as u64], result, 64) as i64, eval_fabric(&ops, &[x], result));
        assert_eq!(eval_width(&lowered, &[x as u64], lresult, 64) as i64, eval_fabric(&lowered, &[x], lresult));
    }
}

#[test]
fn our_sat_certifies_the_lowering_at_five_bits_and_refutes_a_broken_one() {
    let mac = "## To mac (a: Int, b: Int, c: Int) -> Int:\n    Return a * b - c * 3 + 7.\n## Main\n    Show mac(1, 2, 3).";
    let (mops, mresult, _) = residual_from_source(mac, "mac");
    let (mlow, mlres) = carry_save_lower(&mops, mresult, 5);
    assert!(
        certified_equal(&mlow, mlres, &mops, mresult, 5),
        "the 5-bit lowering of a·b − 3c + 7 is not certified ≡ the residual over all 2^15 inputs"
    );
    let mut broken = mlow.clone();
    assert_eq!(broken[mlres].op, AluOp::Xor, "the lowered result is the final sum of a prefix add");
    broken[mlres].op = AluOp::Or;
    assert!(
        !certified_equal(&broken, mlres, &mops, mresult, 5),
        "a lowering whose final sum is an OR certified ≡ the residual: the SAT check is vacuous"
    );
}

// ── C. residuals of every shape ─────────────────────────────────────────────────────────────────────

fn random_residual(rng: &mut Rng, n_in: usize, n_ops: usize) -> (Vec<FabricOp>, usize) {
    const OPS: [AluOp; 12] = [
        AluOp::Add,
        AluOp::Add,
        AluOp::Sub,
        AluOp::Sub,
        AluOp::Mul,
        AluOp::Mul,
        AluOp::And,
        AluOp::Or,
        AluOp::Xor,
        AluOp::Shl,
        AluOp::Shr,
        AluOp::UShr,
    ];
    let mut ops: Vec<FabricOp> = Vec::with_capacity(n_ops);
    for i in 0..n_ops {
        let pick = |rng: &mut Rng| -> Src {
            match rng.below(6) {
                0 => Src::Const(rng.word()),
                1 | 2 => Src::In(rng.below(n_in)),
                _ if i > 0 => Src::Val(i - 1 - rng.below(i.min(4))),
                _ => Src::In(rng.below(n_in)),
            }
        };
        let op = OPS[rng.below(OPS.len())];
        let a = pick(rng);
        let b = match op {
            AluOp::Shl | AluOp::Shr | AluOp::UShr if rng.below(4) != 0 => Src::Const(rng.below(64) as i64),
            _ => pick(rng),
        };
        ops.push(FabricOp { op, a, b });
    }
    (ops, n_ops - 1)
}

#[test]
fn residuals_of_every_shape_lower_and_stream_exactly() {
    let mut rng = Rng(0x5EED_0003);
    for case in 0..240 {
        let n_in = 1 + rng.below(4);
        let n_ops = 2 + rng.below(14);
        let (ops, result) = random_residual(&mut rng, n_in, n_ops);
        let (lowered, lresult) = carry_save_lower(&ops, result, 64);
        let vectors: Vec<Vec<i64>> = (0..40)
            .map(|_| (0..n_in).map(|_| rng.word()).collect())
            .collect();
        for v in &vectors {
            assert_eq!(
                eval_fabric(&lowered, v, lresult),
                eval_fabric(&ops, v, result),
                "case {case}: the lowering of {ops:?} differs at inputs {v:?}"
            );
        }
        let budget = [1.0, 2.5, 4.0, 6.0, 9.0, 1e9][rng.below(6)];
        let sched = schedule_pipeline(&lowered, lresult, budget);
        sched.check(&lowered).unwrap_or_else(|e| panic!("case {case}: {e}"));
        let latency = sched.latency();
        let mut stream = vectors.clone();
        stream.extend(std::iter::repeat(vec![0; n_in]).take(latency));
        let out = simulate_pipeline(&lowered, &sched, n_in, &stream);
        for (t, v) in vectors.iter().enumerate() {
            assert_eq!(
                out[t + latency - 1],
                eval_fabric(&ops, v, result),
                "case {case}: the pipeline at budget {budget} (latency {latency}) streamed a wrong result for input {t}"
            );
        }
    }
}

#[test]
fn every_shape_with_a_narrow_width_is_certified_by_our_sat_and_exact_on_every_input() {
    let mut rng = Rng(0x5EED_0004);
    for case in 0..24 {
        let n_in = 1 + rng.below(2);
        let n_ops = 2 + rng.below(5);
        let (ops, result) = random_residual(&mut rng, n_in, n_ops);
        let ops: Vec<FabricOp> = ops
            .into_iter()
            .map(|mut o| {
                if let (AluOp::Shl | AluOp::Shr | AluOp::UShr, Src::Const(c)) = (o.op, o.b) {
                    o.b = Src::Const(c & 3);
                }
                o
            })
            .collect();
        let (lowered, lresult) = carry_save_lower(&ops, result, 4);
        assert!(
            certified_equal(&lowered, lresult, &ops, result, 4),
            "case {case}: the 4-bit lowering of {ops:?} is not certified ≡ the residual"
        );
        let (wide, wresult) = carry_save_lower(&ops, result, 7);
        for word in 0..1u64 << (7 * n_in) {
            let v: Vec<u64> = (0..n_in).map(|k| (word >> (7 * k)) & 0x7F).collect();
            assert_eq!(
                eval_width(&wide, &v, wresult, 7),
                eval_width(&ops, &v, result, 7),
                "case {case}: the 7-bit lowering of {ops:?} differs at {v:?}"
            );
        }
    }
}

// ── D. the schedule ─────────────────────────────────────────────────────────────────────────────────

/// The deepest path from the inputs to the result, in the units of `op_delay`.
fn result_depth(ops: &[FabricOp], result: usize) -> f64 {
    let mut depth = vec![0.0f64; ops.len()];
    for (i, op) in ops.iter().enumerate() {
        let start = [op.a, op.b]
            .into_iter()
            .filter_map(|s| match s {
                Src::Val(k) => Some(depth[k]),
                _ => None,
            })
            .fold(0.0, f64::max);
        depth[i] = start + op_delay(op);
    }
    depth[result]
}

#[test]
fn the_schedule_respects_dataflow_and_its_budget_and_wastes_at_most_one_op_per_boundary() {
    let (ops, result) = horner();
    let (lowered, lresult) = carry_save_lower(&ops, result, 64);
    let depth = result_depth(&lowered, lresult);
    let widest = lowered.iter().map(op_delay).fold(0.0, f64::max);
    for budget in [2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 24.0] {
        let sched = schedule_pipeline(&lowered, lresult, budget);
        sched.check(&lowered).unwrap_or_else(|e| panic!("budget {budget}: {e}"));
        assert!(
            sched.stage_depth(&lowered) <= budget + 1e-9,
            "budget {budget}: a stage is {} deep",
            sched.stage_depth(&lowered)
        );
        let stages = sched.stages() as f64;
        assert!(
            stages >= (depth / budget).ceil(),
            "budget {budget}: {stages} stages cannot hold a path {depth} deep"
        );
        assert!(
            stages <= 1.0 + depth / (budget - widest),
            "budget {budget}: {stages} stages for a path {depth} deep wastes more than one op ({widest}) per boundary"
        );
    }
    let flat = schedule_pipeline(&lowered, lresult, 1e9);
    assert_eq!(flat.latency(), 2, "an unbounded budget is one combinational stage between two registers");
    let (_, horner_sched) = horner_seq_pipeline();
    assert_eq!(horner_sched.budget, HORNER_SEQ_BUDGET);
    assert!(horner_sched.latency() > 2, "fabric_horner_seq is pipelined, not one combinational stage");
}

// ── R. the placement holds the fewest register bits ────────────────────────────────────────────────

#[test]
fn the_placement_holds_no_more_register_bits_than_the_earliest_or_the_latest() {
    let (ops, result) = horner();
    let (lowered, lresult) = carry_save_lower(&ops, result, 64);
    for budget in [3.0, 4.0, 6.0, 8.0, 12.0] {
        let best = schedule_pipeline(&lowered, lresult, budget);
        let early = earliest_schedule(&lowered, lresult, budget);
        let late = latest_schedule(&lowered, lresult, budget);
        for (name, s) in [("placed", &best), ("earliest", &early), ("latest", &late)] {
            s.check(&lowered).unwrap_or_else(|e| panic!("budget {budget}, {name}: {e}"));
            assert!(s.stage_depth(&lowered) <= budget + 1e-9, "budget {budget}, {name}: a stage overflows");
        }
        assert_eq!(best.latency(), early.latency(), "budget {budget}: the placement lengthened the pipeline");
        assert_eq!(late.latency(), early.latency());
        let (b, e, l) = (best.register_bits(&lowered), early.register_bits(&lowered), late.register_bits(&lowered));
        assert!(
            b <= e && b <= l,
            "budget {budget}: the placement holds {b} bits, the earliest {e}, the latest {l}"
        );
        println!("budget {budget}: latency {}, register bits placed {b}, earliest {e}, latest {l}", best.latency());
    }
}

#[test]
fn known_bits_never_claim_a_bit_the_value_does_not_hold() {
    let (ops, result) = horner();
    let (lowered, _) = carry_save_lower(&ops, result, 64);
    let known = known_bits(&lowered);
    let mut rng = Rng(0x5EED_0008);
    for _ in 0..300 {
        let x = rng.word();
        let mut vals: Vec<i64> = Vec::with_capacity(lowered.len());
        for (i, op) in lowered.iter().enumerate() {
            let get = |s: Src, vals: &[i64]| match s {
                Src::In(_) => x,
                Src::Val(k) => vals[k],
                Src::Const(c) => c,
            };
            let v = op.op.eval(get(op.a, &vals), get(op.b, &vals));
            let (mask, value) = known[i];
            assert_eq!(
                (v as u64) & mask,
                value,
                "op {i} ({:?}) holds {v:#x}, outside its known bits {mask:#x} = {value:#x}",
                op.op
            );
            vals.push(v);
        }
    }
}

/// A random dataflow graph of bitwise ops between computed values only, so every op has depth and none is wiring.
fn random_logic(rng: &mut Rng, n_in: usize, n_ops: usize) -> (Vec<FabricOp>, usize) {
    const OPS: [AluOp; 3] = [AluOp::And, AluOp::Or, AluOp::Xor];
    let mut ops = Vec::with_capacity(n_ops);
    for i in 0..n_ops {
        let pick = |rng: &mut Rng| {
            if i == 0 || rng.below(4) == 0 {
                Src::In(rng.below(n_in))
            } else {
                Src::Val(i - 1 - rng.below(i.min(3)))
            }
        };
        let (a, b) = (pick(rng), pick(rng));
        ops.push(FabricOp { op: OPS[rng.below(3)], a, b });
    }
    (ops, n_ops - 1)
}

#[test]
fn the_placement_is_the_minimum_on_every_small_residual_enumerated_in_full() {
    let mut rng = Rng(0x5EED_0009);
    let mut compared = 0;
    for case in 0..400 {
        let (n_in, n_ops) = (1 + rng.below(3), 3 + rng.below(7));
        let (ops, result) = random_logic(&mut rng, n_in, n_ops);
        let budget = [1.0, 1.3, 1.7, 2.2][rng.below(4)];
        let best = schedule_pipeline(&ops, result, budget);
        let early = earliest_schedule(&ops, result, budget);
        let late = latest_schedule(&ops, result, budget);
        let ranges: Vec<(usize, usize)> = (0..ops.len()).map(|i| (early.stage[i], late.stage[i])).collect();
        let space: usize = ranges.iter().map(|(a, b)| b - a + 1).product();
        if space > 50_000 {
            continue;
        }
        let mut minimum = u64::MAX;
        let mut stage: Vec<usize> = ranges.iter().map(|r| r.0).collect();
        loop {
            let candidate = PipelineSchedule { stage: stage.clone(), result, budget, inputs: Inputs::Registered };
            if candidate.check(&ops).is_ok()
                && candidate.stage_depth(&ops) <= budget + 1e-9
                && candidate.latency() == early.latency()
            {
                minimum = minimum.min(candidate.register_bits(&ops));
            }
            let mut i = 0;
            while i < stage.len() && stage[i] == ranges[i].1 {
                stage[i] = ranges[i].0;
                i += 1;
            }
            if i == stage.len() {
                break;
            }
            stage[i] += 1;
        }
        best.check(&ops).unwrap_or_else(|e| panic!("case {case}: {e}"));
        assert!(best.stage_depth(&ops) <= budget + 1e-9, "case {case}: a stage overflows");
        assert_eq!(best.latency(), early.latency(), "case {case}: the placement lengthened the pipeline");
        assert_eq!(
            best.register_bits(&ops),
            minimum,
            "case {case}: the placement holds {} bits where an assignment holds {minimum}: {ops:?} at budget {budget}",
            best.register_bits(&ops)
        );
        compared += 1;
    }
    assert!(compared >= 300, "only {compared} residuals were small enough to enumerate");
}

// ── E. the cycle model against the VM ───────────────────────────────────────────────────────────────

#[test]
fn the_pipeline_streams_one_result_per_clock_equal_to_the_vm() {
    let (ops, result) = horner();
    let (lowered, sched) = horner_seq_pipeline();
    let latency = sched.latency();
    let mut rng = Rng(0x5EED_0005);
    let xs: Vec<i64> = EDGES
        .iter()
        .copied()
        .chain((0..500).map(|_| rng.word()))
        .collect();
    let mut stream: Vec<Vec<i64>> = xs.iter().map(|&x| vec![x]).collect();
    stream.extend(std::iter::repeat(vec![0]).take(latency));
    let out = simulate_pipeline(&lowered, &sched, 1, &stream);
    for (t, &x) in xs.iter().enumerate() {
        assert_eq!(
            out[t + latency - 1],
            eval_fabric(&ops, &[x], result),
            "the pipeline's result for input {t} (x = {x}) is wrong"
        );
    }
    for (t, &x) in xs.iter().enumerate().take(EDGES.len()) {
        assert_eq!(
            out[t + latency - 1],
            vm_poly_word(x),
            "the pipeline's result for x = {x} is not the VM's"
        );
    }
}

// ── F. the emitted RTL under iverilog ───────────────────────────────────────────────────────────────

/// The result register after each clock; `None` while it still holds the unknown power-on value.
fn stream_under_iverilog(verilog: &str, module: &str, n_in: usize, stream: &[Vec<i64>]) -> Vec<Option<i64>> {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut hex = String::new();
    for v in stream {
        for x in v {
            hex.push_str(&format!("{:016x}\n", *x as u64));
        }
    }
    std::fs::write(dir.path().join("in.hex"), hex).unwrap();
    let words = stream.len() * n_in;
    let regs = (0..n_in)
        .map(|k| format!("in{k}"))
        .collect::<Vec<_>>()
        .join(", ");
    let ports = (0..n_in)
        .map(|k| format!(".in{k}(in{k})"))
        .collect::<Vec<_>>()
        .join(", ");
    let drive = (0..n_in)
        .map(|k| format!("in{k} = mem[i * {n_in} + {k}];"))
        .collect::<Vec<_>>()
        .join(" ");
    let tb = format!(
        "module tb;\n  reg clk = 0;\n  reg [63:0] {regs};\n  wire [63:0] result;\n  reg [63:0] mem [0:{last}];\n  integer i;\n  \
         fabric_{module} dut(.clk(clk), {ports}, .result(result));\n  initial begin\n    $readmemh(\"{hexpath}\", mem);\n    \
         for (i = 0; i < {n}; i = i + 1) begin\n      {drive}\n      #1 clk = 1; #1 clk = 0;\n      $display(\"%016x\", result);\n    end\n    \
         $finish;\n  end\nendmodule\n",
        last = words - 1,
        n = stream.len(),
        hexpath = dir.path().join("in.hex").display(),
    );
    std::fs::write(dir.path().join("dut.v"), verilog).unwrap();
    std::fs::write(dir.path().join("tb.v"), tb).unwrap();
    let sim = dir.path().join("sim.vvp");
    let build = std::process::Command::new("iverilog")
        .arg("-g2012")
        .arg("-o")
        .arg(&sim)
        .arg(dir.path().join("dut.v"))
        .arg(dir.path().join("tb.v"))
        .output()
        .expect("run iverilog");
    assert!(
        build.status.success(),
        "iverilog failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = std::process::Command::new("vvp")
        .arg(&sim)
        .output()
        .expect("run vvp");
    let got: Vec<Option<i64>> = String::from_utf8_lossy(&run.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| l.len() == 16)
        .map(|l| u64::from_str_radix(l, 16).ok().map(|w| w as i64))
        .collect();
    assert_eq!(got.len(), stream.len(), "one result line per clock");
    got
}

#[test]
fn the_emitted_pipeline_streams_the_vms_recorded_results_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let (ops, result) = horner();
    let (_, sched) = horner_seq_pipeline();
    let latency = sched.latency();
    let vm_in = read_hex_words("fabric_in.hex");
    let vm_out = read_hex_words("fabric_out.hex");
    assert_eq!(vm_in.len(), vm_out.len());
    let mut rng = Rng(0x5EED_0006);
    let mut xs = vm_in.clone();
    xs.extend(EDGES);
    xs.extend((0..200).map(|_| rng.word()));
    let mut stream: Vec<Vec<i64>> = xs.iter().map(|&x| vec![x]).collect();
    stream.extend(std::iter::repeat(vec![0]).take(latency));
    let got = stream_under_iverilog(&emit_horner_seq_verilog(), "horner_seq", 1, &stream);
    for (t, &x) in xs.iter().enumerate() {
        let want = if t < vm_out.len() {
            vm_out[t]
        } else {
            eval_fabric(&ops, &[x], result)
        };
        assert_eq!(
            got[t + latency - 1],
            Some(want),
            "fabric_horner_seq streamed a wrong result for input {t} (x = {x})"
        );
    }
}

#[test]
fn generated_multi_input_pipelines_match_their_residuals_under_iverilog() {
    if !common::require_tool("iverilog") {
        return;
    }
    let mut rng = Rng(0x5EED_0007);
    for case in 0..6 {
        let n_in = 2 + rng.below(3);
        let n_ops = 6 + rng.below(8);
        let (ops, result) = random_residual(&mut rng, n_in, n_ops);
        let (lowered, lresult) = carry_save_lower(&ops, result, 64);
        let sched = schedule_pipeline(&lowered, lresult, [3.0, 6.0][case % 2]);
        let latency = sched.latency();
        let vectors: Vec<Vec<i64>> = (0..48)
            .map(|_| (0..n_in).map(|_| rng.word()).collect())
            .collect();
        let mut stream = vectors.clone();
        stream.extend(std::iter::repeat(vec![0; n_in]).take(latency));
        let verilog = emit_fabric_pipelined_verilog(&format!("gen{case}"), n_in, &lowered, &sched);
        let got = stream_under_iverilog(&verilog, &format!("gen{case}"), n_in, &stream);
        for (t, v) in vectors.iter().enumerate() {
            assert_eq!(
                got[t + latency - 1],
                Some(eval_fabric(&ops, v, result)),
                "case {case}: the emitted pipeline streamed a wrong result for input {t}"
            );
        }
    }
}

// ── H. registers ────────────────────────────────────────────────────────────────────────────────────

#[test]
fn only_computed_values_are_registered_and_the_count_is_exact() {
    let (ops, sched) = horner_seq_pipeline();
    let verilog = emit_horner_seq_verilog();
    let regs: Vec<&str> = verilog
        .lines()
        .filter_map(|l| l.trim().strip_prefix("reg [63:0] "))
        .map(|l| l.trim_end_matches(';'))
        .collect();
    for name in &regs {
        if let Some(rest) = name.strip_prefix('v') {
            let index: usize = rest
                .split("_d")
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or_else(|| panic!("unexpected register {name}"));
            assert!(
                !is_wiring(&ops[index]),
                "register {name} holds wiring, which its readers rebuild"
            );
        } else {
            assert!(name.starts_with("in0_r"), "unexpected register {name}");
        }
    }
    assert!(verilog.contains("output reg [63:0] result"));
    assert_eq!(
        regs.len() + 1,
        sched.register_words(&ops),
        "the emitted registers and the schedule's count disagree"
    );
    let updates = verilog.matches(" <= ").count();
    assert_eq!(updates, regs.len() + 1, "every register is updated exactly once per clock");
}

// ── S. decision fabrics: compares and selects in the op vocabulary ─────────────────────────────────────────────────

const CMPS: [CmpKind; 6] = [CmpKind::Lt, CmpKind::Gt, CmpKind::Le, CmpKind::Ge, CmpKind::Eq, CmpKind::Ne];

/// One compare-select over two inputs: `(in0 cmp in1) ? 1 : 0`.
fn compare_fabric(cmp: CmpKind) -> DecisionFabric {
    DecisionFabric {
        n_in: 2,
        cells: vec![LoopCell::Select { cmp, ca: Src::In(0), cb: Src::In(1), t: Src::Const(1), e: Src::Const(0) }],
        result: Src::Val(0),
        escaped_first_return: false,
    }
}

/// Every compare lowers to the mask of its outcome: at every width from 2 to 7 bits, on every pair of values, the
/// lowered select is 1 exactly when the compare holds on the values read signed — the overflow of `a − b` corrected,
/// the extremes included — and at 64 bits on every pair of edges and on random pairs.
#[test]
fn every_compare_lowers_to_the_mask_of_its_outcome_at_every_narrow_width() {
    for cmp in CMPS {
        let (ops, result) = decision_ops(&compare_fabric(cmp)).unwrap();
        assert!(!ops.iter().any(|o| o.op == AluOp::Mul), "{cmp:?}: a compare needs no multiply");
        for width in 2..=7u32 {
            let sext = |v: u64| ((v << (64 - width)) as i64) >> (64 - width);
            for a in 0..1u64 << width {
                for b in 0..1u64 << width {
                    let want = u64::from(cmp.eval(sext(a), sext(b)));
                    assert_eq!(eval_width(&ops, &[a, b], result, width), want, "{cmp:?} at {width} bits: {} vs {}", sext(a), sext(b));
                }
            }
        }
        let mut rng = Rng(0xC0FE_0000 + cmp as u64);
        let mut pairs: Vec<(i64, i64)> = EDGES.iter().flat_map(|&a| EDGES.iter().map(move |&b| (a, b))).collect();
        pairs.extend((0..20_000).map(|_| (rng.word(), rng.word())));
        for (a, b) in pairs {
            assert_eq!(eval_fabric(&ops, &[a, b], result), i64::from(cmp.eval(a, b)), "{cmp:?}: {a} vs {b}");
        }
    }
}

/// The trading decision K1 compiles from LOGOS, lowered: every input tried — the extremes, random words, and words
/// within a step of each threshold the decision draws, the book flag on and off — gives the decision fabric's value.
#[test]
fn the_decide_fabric_lowers_to_ops_that_compute_it() {
    use logicaffeine_compile::codegen_sva::decision_workload::{DECIDE_FN, DECIDE_NAME};
    let df = decision_fabric_from_source(DECIDE_FN, DECIDE_NAME);
    let (ops, result) = decision_ops(&df).unwrap();
    assert!(ops.iter().all(|o| o.op != AluOp::Mul), "no multiply in a decision");
    let mut rng = Rng(0xDEC1_DE00);
    for _ in 0..50_000 {
        let mut x: Vec<i64> = (0..df.n_in).map(|_| rng.word()).collect();
        if rng.below(2) == 0 {
            // near the thresholds: micro around fair ± edge, the projected position around ±cap
            let fair = (rng.next() % 1_000_000) as i64;
            let edge = (rng.next() % 1000) as i64;
            x[0] = fair + edge + (rng.below(5) as i64 - 2);
            x[1] = fair;
            x[2] = edge;
            x[3] = (rng.next() % 100) as i64;
            x[4] = (rng.next() % 100) as i64;
            x[6] = (rng.next() % 500) as i64;
            x[5] = x[6] - x[3] + (rng.below(5) as i64 - 2);
            x[7] = rng.below(2) as i64;
        }
        assert_eq!(eval_fabric(&ops, &x, result), eval_decision_fabric(&df, &x), "decide{x:?}");
    }
}

/// The lowered decision, scheduled at the carry-chain budget, streams one result per clock: the cycle model's result
/// register holds each input's decision `latency` clocks after it was presented, and no stage holds more than its
/// budget of depth.
#[test]
fn the_decide_pipeline_streams_one_result_a_clock() {
    use logicaffeine_compile::codegen_sva::decision_workload::{DECIDE_FN, DECIDE_NAME};
    let df = decision_fabric_from_source(DECIDE_FN, DECIDE_NAME);
    let (ops, result) = decision_ops(&df).unwrap();
    let sched = schedule_pipeline(&ops, result, DECISION_CHAIN_BUDGET);
    sched.check(&ops).unwrap();
    assert!(sched.stage_depth(&ops) <= DECISION_CHAIN_BUDGET, "a stage over budget");
    assert!(sched.stages() > 4, "the decision's chained compares fill several stages: {}", sched.stages());
    let mut rng = Rng(0x57EA_0001);
    let lat = sched.latency();
    let mut stream: Vec<Vec<i64>> = (0..400).map(|_| (0..df.n_in).map(|_| rng.word()).collect()).collect();
    stream.extend((0..lat).map(|_| vec![0; df.n_in]));
    let out = simulate_pipeline(&ops, &sched, df.n_in, &stream);
    for (t, x) in stream[..400].iter().enumerate() {
        assert_eq!(out[t + lat - 1], eval_decision_fabric(&df, x), "input {t} at latency {lat}");
    }
}

/// A tile with no combinational form here — a heap read — is refused, not lowered to something else.
#[test]
fn a_decision_fabric_with_a_tile_that_has_no_combinational_form_is_refused() {
    let df = DecisionFabric {
        n_in: 1,
        cells: vec![LoopCell::HeapRead { addr: Src::In(0) }],
        result: Src::Val(0),
        escaped_first_return: false,
    };
    let e = decision_ops(&df).unwrap_err();
    assert!(e.contains("cell 0"), "{e}");
}

// ── T. the decision for standard cells ──────────────────────────────────────────────────────────────

fn decide() -> DecisionFabric {
    use logicaffeine_compile::codegen_sva::decision_workload::{DECIDE_FN, DECIDE_NAME};
    decision_fabric_from_source(DECIDE_FN, DECIDE_NAME)
}

/// K1's decision for standard cells at width `width`: hoisted, masked, every sum folded into its readers.
fn decide_for_cells(width: u32) -> (Vec<FabricOp>, usize) {
    let hoisted = hoist_selects(&decide(), width).unwrap();
    let (ops, result) = decision_ops(&hoisted).unwrap();
    carry_save_lower_with(&ops, result, width, SumSharing::Fold, FinalAdd::PrefixTree)
}

/// Inputs to K1's decision: the extremes, random words, and words a step from each threshold it draws.
fn decide_inputs(rng: &mut Rng, n: usize) -> Vec<Vec<i64>> {
    (0..n)
        .map(|_| {
            let mut x: Vec<i64> = (0..8).map(|_| rng.word()).collect();
            if rng.below(2) == 0 {
                let fair = (rng.next() % 1_000_000) as i64;
                let edge = (rng.next() % 1000) as i64;
                x[0] = fair + [edge, -edge][rng.below(2)] + (rng.below(5) as i64 - 2);
                x[1] = fair;
                x[2] = edge;
                x[3] = (rng.next() % 100) as i64;
                x[4] = x[3] + (rng.below(5) as i64 - 2);
                x[6] = (rng.next() % 500) as i64;
                x[5] = [x[6], -x[6]][rng.below(2)] - [x[3], -x[3], x[4], -x[4]][rng.below(4)] + (rng.below(5) as i64 - 2);
                x[7] = rng.below(3) as i64;
            }
            x
        })
        .collect()
}

#[test]
fn folding_lowers_residuals_of_every_shape_exactly() {
    let mut rng = Rng(0x5EED_0005);
    for case in 0..240 {
        let n_in = 1 + rng.below(4);
        let n_ops = 2 + rng.below(14);
        let (ops, result) = random_residual(&mut rng, n_in, n_ops);
        let (lowered, lresult) = carry_save_lower_with(&ops, result, 64, SumSharing::Fold, FinalAdd::PrefixTree);
        assert!(!has_arithmetic(&lowered), "case {case}: a fold left an adder or a multiplier");
        for _ in 0..40 {
            let v: Vec<i64> = (0..n_in).map(|_| rng.word()).collect();
            assert_eq!(eval_fabric(&lowered, &v, lresult), eval_fabric(&ops, &v, result), "case {case}: {ops:?} at {v:?}");
        }
    }
    for case in 0..24 {
        let n_in = 1 + rng.below(2);
        let n_ops = 2 + rng.below(6);
        let (ops, result) = random_residual(&mut rng, n_in, n_ops);
        let ops: Vec<FabricOp> = ops
            .into_iter()
            .map(|mut o| {
                if let (AluOp::Shl | AluOp::Shr | AluOp::UShr, Src::Const(c)) = (o.op, o.b) {
                    o.b = Src::Const(c & 3);
                }
                o
            })
            .collect();
        let (narrow, nresult) = carry_save_lower_with(&ops, result, 4, SumSharing::Fold, FinalAdd::PrefixTree);
        assert!(certified_equal(&narrow, nresult, &ops, result, 4), "case {case}: the 4-bit fold of {ops:?} is not certified");
        let (wide, wresult) = carry_save_lower_with(&ops, result, 7, SumSharing::Fold, FinalAdd::PrefixTree);
        for word in 0..1u64 << (7 * n_in) {
            let v: Vec<u64> = (0..n_in).map(|k| (word >> (7 * k)) & 0x7F).collect();
            assert_eq!(eval_width(&wide, &v, wresult, 7), eval_width(&ops, &v, result, 7), "case {case}: {ops:?} at {v:?}");
        }
    }
}

/// A native final add — the compressor tree's last two words left to one `Add` that synthesis implements — lowers
/// exactly, on residuals of every shape at 64 bits, at every input of 7 bits, and certified by our SAT at 4 bits; the
/// lowering holds no subtract and no multiply.
#[test]
fn a_native_final_add_lowers_exactly() {
    let mac = "## To mac (a: Int, b: Int, c: Int) -> Int:\n    Return a * b - c * 3 + 7.\n## Main\n    Show mac(1, 2, 3).";
    let (mops, mresult, _) = residual_from_source(mac, "mac");
    let adds = |final_add| {
        let (lowered, _) = carry_save_lower_with(&mops, mresult, 64, SumSharing::Resolve, final_add);
        lowered.iter().filter(|o| o.op == AluOp::Add).count()
    };
    assert_eq!(adds(FinalAdd::Native), 1, "one sum, resolved by one add");
    assert_eq!(adds(FinalAdd::PrefixTree), 0, "a prefix tree is bitwise ops");
    let mut rng = Rng(0x5EED_0009);
    for case in 0..240 {
        let n_in = 1 + rng.below(4);
        let n_ops = 2 + rng.below(14);
        let (ops, result) = random_residual(&mut rng, n_in, n_ops);
        for sharing in [SumSharing::Resolve, SumSharing::Fold] {
            let (lowered, lresult) = carry_save_lower_with(&ops, result, 64, sharing, FinalAdd::Native);
            assert!(!lowered.iter().any(|o| matches!(o.op, AluOp::Sub | AluOp::Mul)), "case {case}: a subtract or multiply left");
            for _ in 0..40 {
                let v: Vec<i64> = (0..n_in).map(|_| rng.word()).collect();
                assert_eq!(eval_fabric(&lowered, &v, lresult), eval_fabric(&ops, &v, result), "case {case} {sharing:?}: {v:?}");
            }
        }
    }
    for case in 0..24 {
        let n_in = 1 + rng.below(2);
        let n_ops = 2 + rng.below(6);
        let (ops, result) = random_residual(&mut rng, n_in, n_ops);
        let ops: Vec<FabricOp> = ops
            .into_iter()
            .map(|mut o| {
                if let (AluOp::Shl | AluOp::Shr | AluOp::UShr, Src::Const(c)) = (o.op, o.b) {
                    o.b = Src::Const(c & 3);
                }
                o
            })
            .collect();
        let (narrow, nresult) = carry_save_lower_with(&ops, result, 4, SumSharing::Fold, FinalAdd::Native);
        assert!(certified_equal(&narrow, nresult, &ops, result, 4), "case {case}: {ops:?} is not certified");
        let (wide, wresult) = carry_save_lower_with(&ops, result, 7, SumSharing::Fold, FinalAdd::Native);
        for word in 0..1u64 << (7 * n_in) {
            let v: Vec<u64> = (0..n_in).map(|k| (word >> (7 * k)) & 0x7F).collect();
            assert_eq!(eval_width(&wide, &v, wresult, 7), eval_width(&ops, &v, result, 7), "case {case}: {ops:?} at {v:?}");
        }
    }
}

/// K1's decision for standard cells — each compare's sums folded into it, resolved by native adds, cut at the
/// standard-cell budget, its ports read — streams the compiled decision one result a clock, three clocks after its
/// operands.
#[test]
fn the_folded_decision_streams_the_compiled_decision_from_its_ports() {
    use logicaffeine_compile::codegen_sva::fabric_pipe::DECISION_CELLS_BUDGET;
    let df = decide();
    let (ops, result) = decision_ops(&df).unwrap();
    let (folded, fres) = carry_save_lower_with(&ops, result, 64, SumSharing::Fold, FinalAdd::Native);
    let sched = schedule_pipeline(&folded, fres, DECISION_CELLS_BUDGET).reading(Inputs::Ports);
    assert_eq!(sched.latency(), 3);
    let mut rng = Rng(0x5EED_000A);
    let inputs = decide_inputs(&mut rng, 600);
    let mut stream = inputs.clone();
    stream.extend((0..3).map(|_| vec![0; 8]));
    let out = simulate_pipeline(&folded, &sched, 8, &stream);
    for (t, x) in inputs.iter().enumerate() {
        assert_eq!(out[t + 2], eval_decision_fabric(&df, x), "input {t}");
    }
}

/// A sum read by several sums folds into each of them, and still resolves once for the bitwise op that reads it: no
/// carry chain waits on another, and the value is the residual's.
#[test]
fn a_shared_sum_folds_into_every_sum_that_reads_it() {
    let ops = vec![
        FabricOp { op: AluOp::Add, a: Src::In(0), b: Src::In(1) },
        FabricOp { op: AluOp::Sub, a: Src::Val(0), b: Src::In(2) },
        FabricOp { op: AluOp::Add, a: Src::Val(0), b: Src::In(3) },
        FabricOp { op: AluOp::Xor, a: Src::Val(1), b: Src::Val(2) },
        FabricOp { op: AluOp::And, a: Src::Val(3), b: Src::Val(0) },
    ];
    let (resolved, rres) = carry_save_lower(&ops, 4, 64);
    let (folded, fres) = carry_save_lower_with(&ops, 4, 64, SumSharing::Fold, FinalAdd::PrefixTree);
    assert!(
        result_depth(&folded, fres) < result_depth(&resolved, rres),
        "folding leaves a carry chain waiting on another: {} against {}",
        result_depth(&folded, fres),
        result_depth(&resolved, rres)
    );
    let mut rng = Rng(0x5EED_0006);
    for _ in 0..20_000 {
        let v: Vec<i64> = (0..4).map(|_| rng.word()).collect();
        assert_eq!(eval_fabric(&folded, &v, fres), eval_fabric(&ops, &v, 4));
    }
}

/// The lowering for standard cells computes K1's decision: our SAT certifies the hoisting, the masks and the folds
/// together against the compiled fabric's masks over every 5-bit input, and refutes the chain with one op broken.
#[test]
fn our_sat_certifies_the_decision_for_cells_against_the_compiled_decision_at_five_bits() {
    let (cells, cres) = decide_for_cells(5);
    let (compiled, res) = decision_ops(&decide()).unwrap();
    assert!(
        certified_equal_dag(&cells, cres, &compiled, res, 5),
        "the 5-bit decision for cells is not certified ≡ the compiled decision"
    );
    let mut broken = cells.clone();
    let k = broken.iter().position(|o| o.op == AluOp::Xor && !is_wiring(o)).expect("a computed XOR");
    broken[k].op = AluOp::Or;
    assert!(!certified_equal_dag(&broken, cres, &compiled, res, 5), "a broken chain certified: the check is vacuous");
}

/// At 64 bits the decision for cells streams the compiled decision one result a clock, cut at any budget.
#[test]
fn the_decision_for_cells_streams_the_compiled_decision() {
    let df = decide();
    let hoisted = hoist_selects(&df, 64).unwrap();
    assert!(!reads_a_select(&hoisted));
    let (ops, result) = decide_for_cells(64);
    assert!(!has_arithmetic(&ops), "every sum resolved by a prefix add");
    let mut rng = Rng(0x5EED_0007);
    let inputs = decide_inputs(&mut rng, 3000);
    for x in &inputs {
        assert_eq!(eval_fabric(&ops, x, result), eval_decision_fabric(&df, x), "decide{x:?}");
    }
    for budget in [6.0, 12.0, 20.0, 1e9] {
        let sched = schedule_pipeline(&ops, result, budget);
        sched.check(&ops).unwrap();
        let lat = sched.latency();
        let mut stream = inputs[..400].to_vec();
        stream.extend((0..lat).map(|_| vec![0; 8]));
        let out = simulate_pipeline(&ops, &sched, 8, &stream);
        for (t, x) in inputs[..400].iter().enumerate() {
            assert_eq!(out[t + lat - 1], eval_decision_fabric(&df, x), "budget {budget}, input {t}");
        }
    }
}

/// Hoisting and folding take the decision's depth from its compiled chain of compares to one carry chain and a tree of
/// selects: below half the compiled decision's, lowered the same way.
#[test]
fn the_decision_for_cells_waits_on_one_carry_chain() {
    let (cells, cres) = decide_for_cells(64);
    let (compiled, res) = decision_ops(&decide()).unwrap();
    let (resolved, rres) = carry_save_lower(&compiled, res, 64);
    let (fast, slow) = (result_depth(&cells, cres), result_depth(&resolved, rres));
    assert!(fast * 2.0 < slow, "the decision for cells is {fast} deep, the compiled decision {slow}");
}

/// Every op of a lowered decision is emitted once: no two ops read the same operands the same way, in either order when
/// the op commutes — `a < b` and `b < a` each read `a ^ b` for their overflow fix, once.
#[test]
fn a_lowered_decision_emits_every_op_once() {
    let both_orders = DecisionFabric {
        n_in: 2,
        cells: vec![
            LoopCell::Select { cmp: CmpKind::Lt, ca: Src::In(0), cb: Src::In(1), t: Src::Const(1), e: Src::Const(0) },
            LoopCell::Select { cmp: CmpKind::Lt, ca: Src::In(1), cb: Src::In(0), t: Src::Const(2), e: Src::Const(0) },
            LoopCell::Arith { op: AluOp::Or, a: Src::Val(0), b: Src::Val(1) },
        ],
        result: Src::Val(2),
        escaped_first_return: false,
    };
    for df in [both_orders, decide(), hoist_selects(&decide(), 64).unwrap()] {
        let (ops, _) = decision_ops(&df).unwrap();
        let mut seen = std::collections::HashSet::new();
        for o in &ops {
            let commutes = matches!(o.op, AluOp::Add | AluOp::Mul | AluOp::And | AluOp::Or | AluOp::Xor);
            assert!(
                seen.insert(*o) && !(commutes && seen.contains(&FabricOp { op: o.op, a: o.b, b: o.a }) && o.a != o.b),
                "{o:?} emitted twice"
            );
        }
    }
}

// ── P. a pipeline reading its ports ─────────────────────────────────────────────────────────────────

/// The inputs `result` reads, through any number of ops.
fn inputs_read(ops: &[FabricOp], result: usize) -> std::collections::BTreeSet<usize> {
    let mut seen = std::collections::BTreeSet::new();
    let mut stack = vec![Src::Val(result)];
    let mut visited = vec![false; ops.len()];
    while let Some(s) = stack.pop() {
        match s {
            Src::In(k) => {
                seen.insert(k);
            }
            Src::Val(k) if !visited[k] => {
                visited[k] = true;
                stack.extend([ops[k].a, ops[k].b]);
            }
            _ => {}
        }
    }
    seen
}

#[test]
fn a_pipeline_reading_its_ports_is_the_same_pipeline_one_clock_sooner() {
    let mut rng = Rng(0x5EED_0008);
    for case in 0..200 {
        let n_in = 1 + rng.below(4);
        let n_ops = 2 + rng.below(14);
        let (ops, result) = random_residual(&mut rng, n_in, n_ops);
        let (lowered, lresult) = carry_save_lower(&ops, result, 64);
        let budget = [1.0, 2.5, 4.0, 9.0, 1e9][rng.below(5)];
        let registered = schedule_pipeline(&lowered, lresult, budget);
        let ports = registered.clone().reading(Inputs::Ports);
        assert_eq!(ports.inputs, Inputs::Ports);
        assert_eq!(ports.latency() + 1, registered.latency(), "case {case}: one clock sooner");
        let read = inputs_read(&lowered, lresult).len();
        assert_eq!(registered.register_words(&lowered) - ports.register_words(&lowered), read, "case {case}");
        assert_eq!(registered.register_bits(&lowered) - ports.register_bits(&lowered), 64 * read as u64, "case {case}");
        let lat = ports.latency();
        let vectors: Vec<Vec<i64>> = (0..40).map(|_| (0..n_in).map(|_| rng.word()).collect()).collect();
        let mut stream = vectors.clone();
        stream.extend(std::iter::repeat(vec![0; n_in]).take(lat));
        let out = simulate_pipeline(&lowered, &ports, n_in, &stream);
        for (t, v) in vectors.iter().enumerate() {
            assert_eq!(out[t + lat - 1], eval_fabric(&ops, v, result), "case {case}: input {t} at latency {lat}");
        }
    }
    if !common::require_tool("iverilog") {
        return;
    }
    for case in 0..4 {
        let n_in = 2 + rng.below(3);
        let n_ops = 6 + rng.below(8);
        let (ops, result) = random_residual(&mut rng, n_in, n_ops);
        let (lowered, lresult) = carry_save_lower(&ops, result, 64);
        let sched = schedule_pipeline(&lowered, lresult, [1e9, 3.0][case % 2]).reading(Inputs::Ports);
        let lat = sched.latency();
        let vectors: Vec<Vec<i64>> = (0..48).map(|_| (0..n_in).map(|_| rng.word()).collect()).collect();
        let mut stream = vectors.clone();
        stream.extend(std::iter::repeat(vec![0; n_in]).take(lat));
        let verilog = emit_fabric_pipelined_verilog(&format!("ports{case}"), n_in, &lowered, &sched);
        assert!(!verilog.contains("in0_r"), "case {case}: an input registered twice");
        let got = stream_under_iverilog(&verilog, &format!("ports{case}"), n_in, &stream);
        for (t, v) in vectors.iter().enumerate() {
            assert_eq!(got[t + lat - 1], Some(eval_fabric(&ops, v, result)), "case {case}: input {t}");
        }
    }
}
