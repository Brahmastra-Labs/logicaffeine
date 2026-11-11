//! spatial_int64_lock — 64-bit integer arithmetic runs exact on a chip whose integer lanes are 32 bits wide.
//!
//! An RDU's widest integer is INT32 (`rdu_index_datatype_lock`), and Qwen3.8-Flash-Next's n-gram hash, large indices
//! and some samplers need 64 bits. `spatial::wide_int` lowers 64-bit add, subtract, multiply, and, or, exclusive-or,
//! shifts and the remainder by a divisor of 32 bits or fewer to programs over 32-bit lane operations, and proves each
//! equal to its 64-bit definition:
//! - every operation agrees with Rust's `u64` arithmetic on every input class (zero, one, the largest value, powers of
//!   two and their neighbours, alternating bits, products near 2^63) and on seeded random inputs;
//! - every lowering carries a proof that is checked afresh: the bitwise ones by a certified equivalence check (the
//!   bit-blasted miter decided by SAT sweeping, its LRAT derivation replayed), the arithmetic ones by their algebra (an
//!   identity over the integers, checked by both kernels' ring decision procedure) and the bounds that make each
//!   lane operation exact;
//! - a lane program only ever computes on 32-bit lanes, and the chip's integer width is what decides a lowering is
//!   needed;
//! - every program, as the decision fabric's cells, computes its operation on the fabric's 64-bit ALU, each lane kept to
//!   its 32 bits where the ALU would carry past them;
//! - the n-gram hash's row ids, computed on 32-bit lanes, equal `model::ngram_hash`'s.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_int64_lock`

#![cfg(feature = "codegen")]

use logicaffeine_compile::codegen_sva::fabric_rtl::{eval_decision_fabric_heap, DecisionFabric, Src};
use logicaffeine_compile::model::ngram_hash::NgramHash;
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::spatial::wide_int::{lower, needs_limbs, prove, prove_program, KernelVerdict, LaneOp, Op64, ProofKind};

/// The n-gram hash's sixteen head primes, the divisors QWEN4-3 lowers.
const HEAD_PRIMES: [u32; 16] = [
    20000003, 20000023, 20000033, 20000047, 20000059, 20000063, 20000069, 20000077, 20000081, 20000093, 20000107,
    20000147, 20000153, 20000159, 20000161, 20000171,
];

/// Every operation the lowering knows, at every constant the lock holds it to.
fn operations() -> Vec<Op64> {
    let mut ops = vec![Op64::Add, Op64::Sub, Op64::Mul, Op64::And, Op64::Or, Op64::Xor];
    for k in [0, 1, 15, 16, 17, 31, 32, 33, 47, 48, 63] {
        ops.push(Op64::Shl(k));
        ops.push(Op64::Shr(k));
    }
    for d in [1, 2, 3, 7, 10, 255, 65535, 65536, 65537, (1 << 31) - 1, 1 << 31, (1 << 31) + 1, u32::MAX] {
        ops.push(Op64::Rem(d));
    }
    ops.extend(HEAD_PRIMES.iter().map(|&p| Op64::Rem(p)));
    ops
}

/// The input classes: the edges of the range and of every limb, and products near 2^63.
fn edge_values() -> Vec<u64> {
    let mut v = vec![0, 1, 2, 3, u64::MAX, u64::MAX - 1, 0x5555_5555_5555_5555, 0xAAAA_AAAA_AAAA_AAAA, 0x0123_4567_89AB_CDEF];
    for k in [15, 16, 17, 31, 32, 33, 47, 48, 49, 62, 63] {
        let p = 1u64 << k;
        v.extend([p - 1, p, p + 1]);
    }
    // The n-gram hash's largest product: a multiplier below 2^45 times the largest token below 2^18.
    v.extend([(1u64 << 45) - 1, (1u64 << 18) - 1, ((1u64 << 45) - 1) * ((1u64 << 18) - 1)]);
    v.extend([23703573157769, 20109073645365, 8052911324071, 248319]);
    v
}

fn xorshift(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

#[test]
fn every_operation_equals_its_64_bit_definition() {
    let edges = edge_values();
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    for op in operations() {
        let program = lower(op);
        let mut cases: Vec<(u64, u64)> = Vec::new();
        for &a in &edges {
            for &b in &edges {
                cases.push((a, b));
            }
        }
        for _ in 0..4096 {
            let (a, b) = (xorshift(&mut seed), xorshift(&mut seed));
            // Short operands too, so carries out of every limb are exercised from small values.
            let shift = xorshift(&mut seed) % 64;
            cases.push((a, b));
            cases.push((a >> shift, b >> (63 - shift)));
        }
        for (a, b) in cases {
            let want = op.apply(a, b);
            let got = program.run(a, b);
            assert_eq!(got, want, "{op:?}({a:#x}, {b:#x}) on 32-bit lanes is {got:#x}, its definition {want:#x}");
        }
    }
}

/// The workers the proof sweep spreads its lowerings over; `.config/nextest.toml` gives the test as many threads.
const PROOF_WORKERS: usize = 4;

#[test]
fn every_lowering_carries_a_proof() {
    // Each worker keeps its own kernel context, built once; the lowerings are dealt round-robin so every worker gets
    // a share of the remainders, which cost the most.
    let ops = operations();
    let shares: Vec<Vec<Op64>> =
        (0..PROOF_WORKERS).map(|w| ops.iter().copied().skip(w).step_by(PROOF_WORKERS).collect()).collect();
    let outcomes: Vec<(Op64, Result<(bool, (usize, usize), ProofKind), String>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = shares
            .into_iter()
            .map(|share| {
                scope.spawn(move || {
                    share
                        .into_iter()
                        .map(|op| (op, prove(op).map(|p| (p.check(), p.second_kernel_decided(), p.kind().clone()))))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().expect("a proof worker")).collect()
    });
    assert_eq!(outcomes.len(), ops.len(), "every lowering is proved");
    let (mut decided, mut identities) = (0, 0);
    for (op, outcome) in outcomes {
        let (checks, (d, n), kind) = outcome.unwrap_or_else(|e| panic!("{op:?}: no proof: {e}"));
        assert!(checks, "{op:?}: the proof does not check");
        (decided, identities) = (decided + d, identities + n);
        match op {
            Op64::Add | Op64::Sub | Op64::Mul | Op64::Rem(_) => {
                assert!(matches!(&kind, ProofKind::Algebra(ids) if !ids.is_empty()), "{op:?}: proved by its algebra")
            }
            _ => assert_eq!(kind, ProofKind::Circuit, "{op:?}: proved by its circuit"),
        }
        // The add's and the subtract's identities are small enough for the second kernel's exact evaluation too.
        if matches!(op, Op64::Add | Op64::Sub) {
            assert!(matches!(&kind, ProofKind::Algebra(ids) if ids.iter().all(|(_, v)| *v == KernelVerdict::Both)), "{op:?}");
        }
    }
    eprintln!("the second kernel decided {decided} of {identities} identities; the first kernel all of them");
}

/// A program that is not the lowering has no proof: the add with its first carry dropped computes the wrong sum, and
/// the prover refuses it; the remainder with its correction bit taken one place too low likewise.
#[test]
fn a_wrong_program_has_no_proof() {
    let add = lower(Op64::Add);
    // The carries are the shifts by 16 of the column sums, after the four that cut the operands' lanes into limbs.
    let carries: Vec<usize> =
        add.ops().iter().enumerate().filter(|(_, o)| matches!(o, LaneOp::Shr(_, 16))).map(|(i, _)| i).skip(4).collect();
    assert_eq!(carries.len(), 4, "a carry out of each of the four columns");
    let dropped = add.with_op(carries[0], LaneOp::Const(0));
    assert_ne!(dropped.run(0xFFFF, 1), Op64::Add.apply(0xFFFF, 1), "the dropped carry shows in a sum");
    assert!(prove_program(dropped).is_err(), "a dropped carry is proved");

    let rem = lower(Op64::Rem(20000003));
    let over = rem.ops().iter().rposition(|o| matches!(o, LaneOp::Shr(_, 2))).expect("the correction bit");
    let LaneOp::Shr(probe, _) = rem.ops()[over] else { unreachable!() };
    let low = rem.with_op(over, LaneOp::Shr(probe, 1));
    assert!(prove_program(low).is_err(), "a correction bit read one place low is proved");
}

/// A proof that does not fit its program, or whose identity was altered, does not check.
#[test]
fn a_tampered_proof_does_not_check() {
    let proof = prove(Op64::Add).unwrap();
    let ProofKind::Algebra(ids) = proof.kind().clone() else { panic!("the add is proved by its algebra") };
    let mut wrong = ids.clone();
    wrong[0].0 .1 = logicaffeine_kernel::tactic_cert::Syn::Lit(1);
    assert!(!proof.with_kind(ProofKind::Algebra(wrong)).check(), "an identity changed after the fact");
    assert!(!prove(Op64::Sub).unwrap().with_kind(ProofKind::Algebra(ids)).check(), "the add's identities for the subtract");
}

#[test]
fn the_edge_cases_hold_exactly() {
    // A product near 2^63, the n-gram hash's largest.
    let (m, t) = ((1u64 << 45) - 1, (1u64 << 18) - 1);
    assert_eq!(lower(Op64::Mul).run(m, t), m * t);
    // A modulus by 1 is zero for every value; a modulus near 2^31 keeps every remainder below it.
    for x in edge_values() {
        assert_eq!(lower(Op64::Rem(1)).run(x, 0), 0);
        for d in [(1u32 << 31) - 1, 1 << 31, (1 << 31) + 1] {
            let r = lower(Op64::Rem(d)).run(x, 0);
            assert!(r < u64::from(d) && r == x % u64::from(d), "{x:#x} mod {d}: {r}");
        }
    }
}

#[test]
fn a_lane_program_computes_only_on_32_bit_lanes() {
    // A lane program's values are 32-bit lanes by construction; what the test holds is its shape, and that the
    // chip's integer width is what calls for the lowering at all.
    assert!(needs_limbs(&SpatialTarget::sn50_rdu()), "an RDU's integer is INT32: 64-bit values need limbs");
    for op in operations() {
        let program = lower(op);
        assert!(program.len() > 0, "{op:?}: an empty program");
        assert_eq!(program.outputs(), 2, "{op:?}: a 64-bit result is two lanes");
        assert_eq!(program.inputs(), 2 * op.arity(), "{op:?}: each 64-bit operand is two lanes");
    }
}

/// `op`'s program as decision-fabric cells, run by the fabric's 64-bit ALU on `a` and `b`: each lane the ALU computes,
/// held to 32 bits, and the 64-bit result they make.
fn on_the_fabric(op: Op64, a: u64, b: u64) -> u64 {
    let program = lower(op);
    let inputs: Vec<Src> = (0..program.inputs()).map(Src::In).collect();
    let mut cells = Vec::new();
    let [lo, hi] = program.cells(&inputs, &mut cells);
    let values: Vec<i64> = [a, a >> 32, b, b >> 32][..program.inputs()].iter().map(|v| i64::from(*v as u32)).collect();
    let lane = |result: Src| {
        let fabric = DecisionFabric { n_in: values.len(), cells: cells.clone(), result, escaped_first_return: false };
        let v = eval_decision_fabric_heap(&fabric, &values, &mut Vec::new());
        assert!((0..=i64::from(u32::MAX)).contains(&v), "{op:?}({a:#x}, {b:#x}): a lane of {v:#x} is wider than 32 bits");
        v as u64
    };
    lane(lo) | lane(hi) << 32
}

#[test]
fn every_program_computes_its_operation_as_fabric_cells_on_a_wider_alu() {
    // The fabric's ALU wraps at 64 bits, past a lane's 32: an add, a subtract, a product's low half and a left shift
    // carry past them there (a subtract below zero, a shift of a lane's top bits, the product of two wide lanes), and
    // each is masked back to its lane.
    let edges = edge_values();
    let mut seed = 0x0DDB_1A5E_5BAD_5EEDu64;
    for op in operations() {
        let mut cases: Vec<(u64, u64)> = Vec::new();
        for &a in &edges {
            for &b in &edges {
                cases.push((a, b));
            }
        }
        for _ in 0..256 {
            cases.push((xorshift(&mut seed), xorshift(&mut seed)));
        }
        for (a, b) in cases {
            let (got, want) = (on_the_fabric(op, a, b), op.apply(a, b));
            assert_eq!(got, want, "{op:?}({a:#x}, {b:#x}) as fabric cells is {got:#x}, its definition {want:#x}");
        }
    }
    // The ALU's wraps the masks undo, each made to happen.
    assert_eq!(on_the_fabric(Op64::Sub, 0, 1), u64::MAX, "a lane subtracted below zero");
    assert_eq!(on_the_fabric(Op64::Shl(1), 1 << 31, 0), 1 << 32, "a lane's top bit shifted out of it");
    assert_eq!(on_the_fabric(Op64::Shl(33), u64::MAX, 0), u64::MAX << 33, "a lane shifted across");
}

#[test]
fn the_n_gram_hash_runs_exact_on_32_bit_lanes() {
    let hash = NgramHash::new(248320, 3, 8, 20_000_000, 128, 0, 1234, 248044);
    assert_eq!(hash.head_primes(), HEAD_PRIMES.map(u64::from).as_slice());
    let (mul, xor) = (lower(Op64::Mul), lower(Op64::Xor));
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    for _ in 0..512 {
        // No end-of-sequence token among them: its segment reset is the hash's, not the arithmetic's.
        let tokens: Vec<u32> = (0..6).map(|_| (xorshift(&mut seed) % 248320) as u32).map(|t| if t == 248044 { t + 1 } else { t }).collect();
        let want = hash.ids(&[], &tokens);
        let m = hash.multipliers();
        for (p, row) in want.iter().enumerate() {
            // The token at p and its two predecessors, the history padded with end-of-sequence as the hash pads it.
            let at = |s: usize| if p >= s { u64::from(tokens[p - s]) } else { 248044 };
            let mut got = Vec::new();
            for n in 2..=3usize {
                let mut mixed = mul.run(at(0), m[0]);
                for place in 1..n {
                    let term = mul.run(at(place), m[place]);
                    mixed = xor.run(mixed, term);
                }
                for h in 0..8 {
                    let head = (n - 2) * 8 + h;
                    got.push(lower(Op64::Rem(HEAD_PRIMES[head])).run(mixed, 0) + hash.head_offsets()[head]);
                }
            }
            assert_eq!(&got, row, "position {p} of {tokens:?}");
        }
    }
}
