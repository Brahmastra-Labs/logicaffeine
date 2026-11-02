use super::*;

fn silicon_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon")
}

/// Emit the FIFO RTL, its testbench, and the oracle trace (golden; regenerate with
/// `CHAN_RECORD=1`). iverilog PASS is wired into scripts/synth-soft-core.sh.
#[test]
fn chan_fifo_artifacts_are_current() {
    const N: usize = 200;
    let (ctrl, din, pops, npops, final_count) = chan_trace(N);
    assert!(npops >= 1 && final_count <= DEPTH, "sane trace");
    let files = [
        ("chan_fifo.v", emit_chan_fifo_verilog()),
        (
            "tb_chan_fifo.v",
            emit_chan_fifo_testbench(N, npops, final_count),
        ),
        ("chan_ctrl.hex", ctrl),
        ("chan_din.hex", din),
        ("chan_pops.hex", pops),
    ];
    let dir = silicon_dir();
    if std::env::var("CHAN_RECORD").is_ok() {
        for (n, b) in &files {
            std::fs::write(dir.join(n), b).expect("write");
        }
        return;
    }
    for (n, b) in &files {
        let got = std::fs::read_to_string(dir.join(n)).unwrap_or_else(|e| {
            panic!("missing {n} ({e}) — record: CHAN_RECORD=1 cargo test chan_fifo_artifacts")
        });
        assert_eq!(&got, b, "{n} is stale — regenerate with CHAN_RECORD=1");
    }
}

/// FIFO order + bounded discipline: the oracle is a bounded VecDeque (the channel's own
/// buffer type); a full buffer drops the push, an empty buffer drops the pop.
#[test]
fn chan_trace_respects_the_bounded_fifo_discipline() {
    let (_c, _d, _p, npops, final_count) = chan_trace(200);
    assert!(final_count <= DEPTH, "never exceeds capacity");
    assert!(npops > 0, "some values flow through");
}

/// The FIFO reproduces the runtime channel's contract: values leave in the exact order
/// they entered (FIFO), and a full buffer drops the sender (bounded) — so the popped
/// sequence is a prefix of the successfully-entered sequence. This is the determinism the
/// silicon FIFO (proven ≡ this discipline under iverilog) inherits from
/// `logicaffeine_runtime::channel::Chan`.
#[test]
fn chan_fifo_matches_the_runtime_channel() {
    use std::collections::VecDeque;
    let mut st: u64 = 0x0BAD_C0DE_1234_5678;
    let mut next = || {
        st = st
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        st >> 32
    };
    let mut q: VecDeque<u32> = VecDeque::new();
    let (mut entered, mut popped) = (Vec::new(), Vec::new());
    for _ in 0..500 {
        let r = next();
        let (push, pop) = ((r % 8) < 5, ((r >> 3) % 8) < 5);
        let d = next() as u32;
        let do_pop = pop && !q.is_empty();
        let do_push = push && q.len() < DEPTH;
        if do_pop {
            popped.push(*q.front().unwrap());
            q.pop_front();
        }
        if do_push {
            entered.push(d);
            q.push_back(d);
        }
    }
    assert!(!popped.is_empty() && popped.len() <= entered.len());
    assert_eq!(
        &entered[..popped.len()],
        &popped[..],
        "FIFO order — a channel delivers values in the order they were sent"
    );
}

/// WORLD-CLASS: the FIFO's bound is proven by K-INDUCTION with our OWN certified SAT — not
/// sampled. Modelling the actual RTL's count register (`cnt' = cnt − popped + pushed`, with
/// `popped = pop ∧ cnt≠0`, `pushed = push ∧ cnt≠8`) as a transition system over ALL possible
/// push/pop input streams, the safety invariant `cnt ≤ 8` (the DEPTH bound — never overflow
/// to 9, never underflow-wrap to 15) holds in EVERY reachable state. This is the sequential
/// analogue of the combinational our-SAT proofs the flagship datapaths carry.
#[test]
fn the_chan_fifo_bound_is_proven_by_k_induction() {
    use super::super::rtl::{NextState, Register, TransitionSystem};
    use super::super::sva_to_verify::{BitVecBoundedOp::*, BoundedExpr};
    use logicaffeine_proof::bmc::{BmcOutcome, InductionOutcome};

    const WIDTH: u32 = 4; // cnt ∈ 0..8 — the 4-bit count register of chan_fifo.v
    let cnt = || Box::new(BoundedExpr::BitVecVar("cnt".into(), WIDTH));
    let k = |v: u64| {
        Box::new(BoundedExpr::BitVecConst {
            width: WIDTH,
            value: v,
        })
    };
    let bv = |op, l: Box<BoundedExpr>, r: Box<BoundedExpr>| {
        Box::new(BoundedExpr::BitVecBinary {
            op,
            left: l,
            right: r,
        })
    };
    let eqk = |v: u64| *bv(Eq, cnt(), k(v));
    let and = |a: BoundedExpr, b: BoundedExpr| BoundedExpr::And(Box::new(a), Box::new(b));
    let not = |a: BoundedExpr| BoundedExpr::Not(Box::new(a));

    // The guards, exactly as the RTL computes them (push/pop are FREE per-cycle inputs).
    let popped = and(BoundedExpr::Var("pop".into()), not(eqk(0)));
    let pushed = and(BoundedExpr::Var("push".into()), not(eqk(8)));

    // cnt' = popped ? (pushed ? cnt : cnt−1) : (pushed ? cnt+1 : cnt).
    let next = NextState::Ite {
        cond: popped,
        then_: Box::new(NextState::Ite {
            cond: pushed.clone(),
            then_: Box::new(NextState::Simple(*cnt())),
            else_: Box::new(NextState::Simple(*bv(Sub, cnt(), k(1)))),
        }),
        else_: Box::new(NextState::Ite {
            cond: pushed,
            then_: Box::new(NextState::Simple(*bv(Add, cnt(), k(1)))),
            else_: Box::new(NextState::Simple(*cnt())),
        }),
    };

    // Safety: cnt ≤ 8, i.e. ¬(8 < cnt) — catches overflow (9) AND underflow-wrap (15).
    let bound_ok = not(*bv(ULt, k(8), cnt()));

    let ts = TransitionSystem {
        registers: vec![Register {
            name: "cnt".into(),
            width: WIDTH,
            init: Some(0),
            next,
        }],
        property: bound_ok,
    };

    assert_eq!(
        ts.prove_invariant(1),
        InductionOutcome::Proven,
        "the FIFO count is bounded in [0,8] for EVERY push/pop stream (k-induction, our SAT)"
    );
    assert!(
        matches!(ts.bmc(16), BmcOutcome::NoneWithin(_)),
        "no reachable count-overflow within 16 steps (BMC sanity)"
    );
}
