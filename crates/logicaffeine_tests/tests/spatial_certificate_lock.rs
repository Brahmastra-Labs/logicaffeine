//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! CERTIFICATE LOCK — every verified spatial program PROVES its choreography, and the proof is
//! re-checkable by a checker that does not trust the passes.
//!
//! `verify` builds the unrolled event graph of the schedule (program order, FIFO push→wait pairing, space
//! return pop→reserve at the buffer's depth, semaphore increments→waits, valid flags), proves it acyclic and
//! fully sourced (Commoner's condition on the marked graph: no token-free cycle, no starved transition),
//! records the counting balances, and attaches a SAT placement certificate for every integer PE datapath.
//! This lock proves:
//!   A. every preset's programs carry a certificate whose `recheck` passes;
//!   B. the graph is a marked graph (each place has one producer and one consumer) and the liveness
//!      witness IS a topological order over the token-free arcs — verified here independently;
//!   C. `recheck` is fail-closed and independent: a tampered witness, a foreign program, a mutated balance
//!      or graph all fail — and so does a certificate that misstates what it certifies: another numerics claim than
//!      the program's, a compute stage dropped or one that is not a compute stage; one that claims less (a `Sat`
//!      stage stated `Uncertified`) still rechecks;
//!   D. the graph alone (no evaluation) catches a dropped pop, a dropped semaphore increment, a dropped
//!      valid flag — `verify` refuses and issues no certificate;
//!   E. integer matmul stages ship SAT PE certificates that recheck and are load-bearing; float stages are
//!      honestly `Uncertified` with the reason; the digest binds the certificate to one schedule;
//!   F. (fuzz) a certificate is NEVER issued to a program whose evaluation faults.
//!
//! ★ MUTATION PROBES (2026-09-25, private copy): `recheck` without its numerics clause → C; without its compute-stage
//! clause → C.
//!
//! Run: `cargo test -p logicaffeine-tests --test spatial_certificate_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::spatial::certificate::{
    certify_choreography, CertifyFault, ComputeStageCert,
};
use logicaffeine_compile::spatial::eval::eval_choreography;
use logicaffeine_compile::spatial::ir::{
    ChoreoOp, ComputeOp, NumericsClaim, NumericsPolicy, SpatialProgram, StageKind,
};
use logicaffeine_compile::spatial::passes::from_tensor_op::MatmulNames;
use logicaffeine_compile::spatial::passes::verify::verify;
use logicaffeine_compile::spatial::passes::SpatialReject;
use logicaffeine_compile::spatial::pipeline::{compile_to_spatial, run_passes, template_from_spec};
use logicaffeine_compile::spatial::target::SpatialTarget;
use logicaffeine_compile::tensor_recognize::{Fill, MatmulSpec};
use spatial_support::*;

fn unverified(p: &SpatialProgram) -> SpatialProgram {
    let mut q = p.clone();
    q.certificate = None;
    q
}

fn fused_small() -> SpatialProgram {
    compile_to_spatial(&gemm_program(128, 128, 64), &fusing_wormhole()).unwrap()
}

#[test]
fn a_verified_program_carries_a_certificate_that_rechecks() {
    for target in SpatialTarget::presets() {
        for (m, n, k) in [(64usize, 64usize, 64usize), (256, 256, 128)] {
            let p = compile_to_spatial(&gemm_program(m, n, k), &target).unwrap();
            let c = p
                .certificate
                .as_ref()
                .unwrap_or_else(|| panic!("{} {m}x{n}x{k}: no certificate", target.name));
            assert!(
                c.recheck(&p),
                "{} {m}x{n}x{k}: the certificate must re-verify",
                target.name
            );
        }
    }
    let big =
        compile_to_spatial(&gemm_program(2048, 1024, 512), &SpatialTarget::wormhole()).unwrap();
    assert!(big.certificate.as_ref().unwrap().recheck(&big));
    assert!(
        big.certificate.as_ref().unwrap().transitions() > 64 * 20,
        "a 64-site choreography has thousands of events"
    );
}

#[test]
fn the_event_graph_is_a_marked_graph_and_the_witness_is_a_topological_order() {
    let p = fused_small();
    let c = p.certificate.as_ref().unwrap();
    let g = c
        .proof
        .whole_graph()
        .expect("a single-section program is proved whole");
    let mut producers = vec![0usize; g.places.len()];
    let mut consumers = vec![0usize; g.places.len()];
    for t in &g.transitions {
        for pl in &t.produces {
            producers[*pl] += 1;
        }
        for pl in &t.consumes {
            consumers[*pl] += 1;
        }
    }
    for (i, place) in g.places.iter().enumerate() {
        assert!(
            producers[i] <= 1,
            "place {i} {:?} has {} producers",
            place.kind,
            producers[i]
        );
        assert!(
            consumers[i] <= 1,
            "place {i} {:?} has {} consumers",
            place.kind,
            consumers[i]
        );
        assert!(
            producers[i] == 1 || place.marking > 0,
            "place {i} {:?} is never produced and carries no initial token",
            place.kind
        );
    }
    // Independent check of the witness: for every token-free place, its producer precedes its consumer.
    let order = &c.proof.witnesses()[0].topo_order;
    assert_eq!(
        order.len(),
        g.transitions.len(),
        "the witness orders every transition"
    );
    let mut pos = vec![usize::MAX; g.transitions.len()];
    for (i, t) in order.iter().enumerate() {
        pos[*t] = i;
    }
    assert!(
        pos.iter().all(|p| *p != usize::MAX),
        "every transition appears once"
    );
    let mut producer_of = vec![None; g.places.len()];
    for (ti, t) in g.transitions.iter().enumerate() {
        for pl in &t.produces {
            producer_of[*pl] = Some(ti);
        }
    }
    let mut token_free_arcs = 0usize;
    for (ti, t) in g.transitions.iter().enumerate() {
        for pl in &t.consumes {
            if g.places[*pl].marking == 0 {
                let prod = producer_of[*pl].expect("a token-free place has a producer");
                assert!(
                    pos[prod] < pos[ti],
                    "token-free arc {prod}→{ti} violates the witness order"
                );
                token_free_arcs += 1;
            }
        }
    }
    assert!(
        token_free_arcs > 100,
        "the choreography has real dependencies: {token_free_arcs}"
    );
}

#[test]
fn recheck_is_independent_and_fail_closed() {
    let p = fused_small();
    let c = p.certificate.as_ref().unwrap();
    assert!(c.recheck(&p));
    assert!(
        !c.with_witness_tampered().recheck(&p),
        "a permuted witness must fail"
    );
    let other = compile_to_spatial(&gemm_program(96, 64, 64), &SpatialTarget::wormhole()).unwrap();
    assert!(
        !c.recheck(&other),
        "a certificate is bound to its own schedule (digest)"
    );
    let mut balances = c.clone();
    balances.balances[0].pushed += 1;
    assert!(!balances.recheck(&p), "a mutated balance must fail");
    let mut graph = c.clone();
    graph
        .proof
        .whole_graph_mut()
        .expect("a single-section program is proved whole")
        .places[0]
        .marking += 1;
    assert!(!graph.recheck(&p), "a mutated graph must fail");
    let mut sems = c.clone();
    if let Some(s) = sems.sem_counts.first_mut() {
        s.1 += 1;
        assert!(!sems.recheck(&p), "a mutated semaphore count must fail");
    }
    // A certificate states what it certifies — on the integer fabric, where every matmul stage carries a SAT certificate.
    let p = compile_to_spatial(
        &gemm_program(16, 16, 16),
        &SpatialTarget::sky_fabric(64, 64),
    )
    .unwrap();
    let c = p.certificate.as_ref().unwrap();
    assert!(c.recheck(&p));
    assert_eq!(
        p.claim,
        NumericsClaim::Exact,
        "an integer GEMM claims exact numerics"
    );
    let mut numerics = c.clone();
    numerics.numerics = NumericsClaim::Approx {
        reason: "not the program's claim".into(),
    };
    assert!(
        !numerics.recheck(&p),
        "a certificate stating another numerics claim than the program's must fail"
    );
    assert!(
        c.compute
            .iter()
            .any(|(_, k)| matches!(k, ComputeStageCert::Sat(_))),
        "the integer matmul stages carry SAT certificates"
    );
    let mut dropped = c.clone();
    dropped.compute.pop();
    assert!(
        !dropped.recheck(&p),
        "a certificate that drops a compute stage must fail"
    );
    let mut foreign = c.clone();
    let not_compute = p
        .stages
        .iter()
        .find(|s| {
            !matches!(
                s.kind,
                StageKind::Compute(ComputeOp::MatmulTiles { .. }) | StageKind::Scalar(_)
            )
        })
        .expect("a movement stage")
        .id;
    foreign.compute[0].0 = not_compute;
    assert!(
        !foreign.recheck(&p),
        "a certificate naming a stage that computes nothing must fail"
    );
    let mut less = c.clone();
    less.compute[0].1 = ComputeStageCert::Uncertified {
        reason: "claims less".into(),
    };
    assert!(
        less.recheck(&p),
        "a certificate claiming less than it could still rechecks"
    );
}

#[test]
fn a_dropped_pop_is_found_by_the_graph_alone_and_verify_issues_no_certificate() {
    let p = fused_small();
    let pop = enumerate_ops(p.schedule.as_ref().unwrap())
        .into_iter()
        .find(|(_, op)| matches!(op, ChoreoOp::Pop { .. }))
        .unwrap();
    let q = without_op(&p, &pop.0);
    match certify_choreography(&q) {
        Err(CertifyFault::Bounds { .. })
        | Err(CertifyFault::Starved { .. })
        | Err(CertifyFault::Deadlock { .. })
        | Err(CertifyFault::Invalid(_)) => {}
        other => panic!("a dropped pop must be refused by the certificate builder: {other:?}"),
    }
    match verify(&q, &fusing_wormhole()) {
        Err(SpatialReject::Unverified(_)) | Err(SpatialReject::Invalid(_)) => {}
        other => panic!("verify must refuse: {other:?}"),
    }
}

#[test]
fn a_dropped_semaphore_increment_or_flag_is_found_by_the_graph_alone() {
    let p = fused_small();
    let ops = enumerate_ops(p.schedule.as_ref().unwrap());
    let inc = ops
        .iter()
        .find(|(_, op)| matches!(op, ChoreoOp::SemInc { .. }))
        .unwrap();
    match certify_choreography(&without_op(&p, &inc.0)) {
        Err(CertifyFault::Starved { .. })
        | Err(CertifyFault::Deadlock { .. })
        | Err(CertifyFault::Invalid(_)) => {}
        other => panic!("a dropped ready increment starves the sender's wait: {other:?}"),
    }
    let flag = ops
        .iter()
        .find(|(_, op)| matches!(op, ChoreoOp::McastSetValid { .. }))
        .unwrap();
    match certify_choreography(&without_op(&p, &flag.0)) {
        Err(CertifyFault::Starved { .. })
        | Err(CertifyFault::Deadlock { .. })
        | Err(CertifyFault::Invalid(_)) => {}
        other => panic!("a dropped valid flag starves every receiver's wait: {other:?}"),
    }
    let wait = ops
        .iter()
        .find(|(_, op)| matches!(op, ChoreoOp::SemWait { .. }))
        .unwrap();
    match certify_choreography(&without_op(&p, &wait.0)) {
        Err(_) => {}
        Ok(_) => {
            panic!("a dropped semaphore wait breaks the counting balance and must not certify")
        }
    }
}

#[test]
fn every_int_matmul_stage_carries_a_sat_pe_certificate_that_rechecks() {
    let sky = SpatialTarget::sky_fabric(64, 64);
    let p = compile_to_spatial(&gemm_program(16, 16, 16), &sky).unwrap();
    let c = p.certificate.as_ref().unwrap();
    let mut seen = 0;
    for (stage, cert) in &c.compute {
        assert!(matches!(
            p.stages[stage.idx()].kind,
            StageKind::Compute(ComputeOp::MatmulTiles { .. })
        ));
        match cert {
            ComputeStageCert::Sat(pc) => {
                assert!(pc.recheck(), "the PE certificate must re-verify");
                assert!(
                    !pc.with_last_step_removed().recheck(),
                    "the PE certificate must be load-bearing"
                );
                seen += 1;
            }
            ComputeStageCert::Uncertified { reason } => {
                panic!("an integer PE must be SAT-certified, got: {reason}")
            }
        }
    }
    assert!(seen >= 1);
}

#[test]
fn float_stages_are_honestly_uncertified_with_a_reason() {
    let p = compile_to_spatial(&gemm_program(64, 64, 64), &SpatialTarget::wormhole()).unwrap();
    let c = p.certificate.as_ref().unwrap();
    assert!(!c.compute.is_empty());
    for (_, cert) in &c.compute {
        match cert {
            ComputeStageCert::Uncertified { reason } => assert!(reason.contains("float"), "{reason}"),
            ComputeStageCert::Sat(_) => panic!("a bf16/fp32 tensor datapath cannot be bit-blasted today; claiming SAT would be a lie"),
        }
    }
    assert_eq!(c.numerics, p.claim);
}

#[test]
fn the_digest_binds_the_certificate_to_one_schedule() {
    let p = fused_small();
    let c = p.certificate.as_ref().unwrap();
    let same = compile_to_spatial(&gemm_program(128, 128, 64), &fusing_wormhole()).unwrap();
    assert_eq!(
        c.schedule_digest,
        same.certificate.as_ref().unwrap().schedule_digest,
        "deterministic"
    );
    let flush = enumerate_ops(p.schedule.as_ref().unwrap())
        .into_iter()
        .find(|(_, op)| matches!(op, ChoreoOp::ReadBarrier))
        .unwrap();
    let q = without_op(&p, &flush.0);
    assert!(
        !c.recheck(&q),
        "even a harmless edit changes the schedule the certificate speaks about"
    );
    let fresh = certify_choreography(&q).expect("the edited program certifies on its own");
    assert!(fresh.recheck(&q));
    assert_ne!(fresh.schedule_digest, c.schedule_digest);
}

#[test]
fn verify_requires_a_scheduled_program_and_is_idempotent() {
    let t = SpatialTarget::wormhole();
    let template = template_from_spec(
        &MatmulSpec {
            m: 64,
            n: 64,
            k: 64,
            a: Default::default(),
            b: Default::default(),
            c: Default::default(),
        },
        &MatmulNames {
            a: "a".into(),
            b: "b".into(),
            c: "c".into(),
        },
        (&Fill::Unknown, &Fill::Unknown),
        (Some((-3, 3)), Some((-2, 2))),
        NumericsPolicy::Exact,
        &t,
    )
    .unwrap();
    assert!(matches!(
        verify(&template, &t),
        Err(SpatialReject::WrongPhase { pass: "verify", .. })
    ));
    let p = run_passes(template, &t).unwrap();
    let again = verify(&p, &t).unwrap();
    assert_eq!(again, p, "verifying a verified program is a no-op");
    let _ = unverified(&p);
}

#[test]
#[ignore = "300-program verify⇔eval soundness fuzz — run with --ignored"]
fn a_certificate_is_never_issued_to_a_program_whose_evaluation_faults() {
    let mut certified = 0usize;
    let mut refused = 0usize;
    for case in 0..300usize {
        let target = if case % 2 == 0 {
            fusing_wormhole()
        } else {
            SpatialTarget::wormhole()
        };
        let (m, n, k) = (
            32 * prng(81, case, 1, 4) as usize,
            32 * prng(82, case, 1, 4) as usize,
            32 * prng(83, case, 1, 3) as usize,
        );
        let p = compile_to_spatial(&gemm_program(m, n, k), &target).unwrap();
        let ops = enumerate_ops(p.schedule.as_ref().unwrap());
        let pick = prng(84, case, 0, ops.len() as i64 - 1) as usize;
        let q = if case % 3 == 0 {
            without_op(&p, &ops[pick].0)
        } else {
            with_op_duplicated(&p, &ops[pick].0)
        };
        let evaluated = eval_choreography(&q, &inputs_from_fills(&q));
        match certify_choreography(&q) {
            Ok(c) => {
                certified += 1;
                assert!(
                    evaluated.is_ok(),
                    "case {case}: a certificate was issued but the evaluation faults: {:?}",
                    evaluated.err()
                );
                assert!(q.validate().is_ok());
                assert!(c.recheck(&q));
            }
            Err(_) => refused += 1,
        }
    }
    assert!(
        certified > 0 && refused > 0,
        "certified {certified}, refused {refused}"
    );
}
