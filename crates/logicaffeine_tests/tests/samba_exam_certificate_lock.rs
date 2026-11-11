//! samba_exam_certificate_lock — the row of the Samba book's exam E8 (section 10.8 of `benchmarks/PREPARE_FOR_SAMBA.md`,
//! the compiler's own quality) on certificates: every program a bundle ships carries its placement certificate whole,
//! and the certificate passes the independent checker.
//!
//! The checker is [`ChoreographyCertificate::recheck`]: it rebuilds what the certificate claims from the program alone —
//! the schedule's digest, the marked graph the liveness witness orders, every buffer's balance, every semaphore's counts,
//! the numerics claim — and replays every compute stage's PR/RUP refutation through the proof checker. Nothing the
//! compiler derived is trusted, and nothing is derived again here: the row reads each program's `neutral.json` and
//! `certificate.whole.json` off disk, as a reader of the bundle has them, and hands the one to the checker against the
//! other; the certificate's summary in `certificate.json` is of the same schedule.
//! The bundles are the SN50 decode bundle of each shared architecture's tiny checkpoint, and two programs of their own:
//! an integer product, whose tile mapping carries a SAT refutation, and a product wide enough to spread over several
//! PCUs. The checker refuses what is not so: a certificate whose liveness witness is out of order, one checked against
//! another program, and an integer product's refutation with its last step dropped.
//!
//! A refutation proves only the formula it refutes. The integer product's certificate is also forged to carry a sound
//! refutation of another unsatisfiable formula in place of its stage's obligation — once a formula of one variable, once
//! one with the obligation's own variable and clause counts — and the checker refuses it, naming the stage: it rebuilds
//! every stage's obligation from the program and holds the carried formula to it.
//!
//! So does every reader of a placement certificate. A placement's certificate forged the same way, padded to the
//! placement obligation's own shape, is refused by the reader of `certify_external_placement`, by every target backend's
//! reader of `TargetBackend::certify`, and by the loop lane's reader of the proof its row ships — each naming what the
//! certificate was offered for.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_certificate_lock`

#![cfg(feature = "codegen")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_handoff.rs"]
#[allow(dead_code)]
mod samba_handoff;

use logicaffeine_compile::codegen_sva::backend::{RduBackend, SkyVerilogBackend, TargetBackend};
use logicaffeine_compile::codegen_sva::fabric_grid::route_loop_body;
use logicaffeine_compile::codegen_sva::fabric_rtl::{AluOp, FabricOp, LoopCell, Src};
use logicaffeine_compile::codegen_sva::plasticine::PlasticineBackend;
use logicaffeine_compile::codegen_sva::plasticine_report::{counted_loop_fabric, recheck_body, CertificateRow};
use logicaffeine_compile::codegen_sva::pnr_certificate::{
    certify_external_placement, recheck_external_placement, PlacementCertificate, PlacementFault,
};
use logicaffeine_compile::spatial::certificate::{ChoreographyCertificate, ComputeStageCert, RecheckFault};
use logicaffeine_compile::spatial::lower::rdu_lanes::lower_loop_lane;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::lower::rdu::{parse_certificate, parse_neutral};
use samba_handoff::{Model, CTX};
use serde_json::Value;
use std::path::Path;
use tiny_models::Tiny;

/// The architectures every row covers: attention alone, a hybrid with recurrent state, windowed attention, and an
/// expert mixture with sinks.
const SHARED: [Tiny; 4] = [Tiny::Qwen3, Tiny::Qwen35, Tiny::Gemma3, Tiny::GptOss];

/// An integer product: its tile mapping is bit-blasted and its certificate carries the refutation.
const INTEGER: &str = "## Main
Let A be a 4 by 64 tensor of Int.
Let B be a 64 by 32 tensor of Int.
Set item 1 of A to 3.
Set item 2 of B to -2.
Let C be A matmul B.
Show item 1 of C.
";

/// A product of a 128 × 512 matrix by a 512 × 512 one, and the SiLU of it: sections over several PCUs.
const WIDE: &str = "## Main
Let A be a 128 by 512 tensor of Real.
Let B be a 512 by 512 tensor of Real.
Let C be A matmul B.
Let D be silu of C.
Show item 1 of D.
";

/// The program in `dir` and the certificate it carries whole, read off disk as a bundle's reader reads them.
fn shipped(dir: &Path, what: &str) -> (SpatialProgram, ChoreographyCertificate) {
    let p = parse_neutral(&std::fs::read_to_string(dir.join("neutral.json")).unwrap_or_else(|e| panic!("{what}: neutral.json: {e}")))
        .unwrap_or_else(|e| panic!("{what}: neutral.json parses: {e}"));
    let whole = std::fs::read_to_string(dir.join("certificate.whole.json")).unwrap_or_else(|e| panic!("{what}: certificate.whole.json: {e}"));
    let c = parse_certificate(&whole).unwrap_or_else(|e| panic!("{what}: certificate.whole.json carries the certificate whole: {e}"));
    let summary: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("certificate.json")).unwrap_or_else(|e| panic!("{what}: certificate.json: {e}"))).unwrap();
    assert_eq!(summary["digest_hex"], format!("{:016x}", c.schedule_digest), "{what}: the certificate's summary and the certificate are of one schedule");
    (p, c)
}

/// `c` passes the checker against `p`, and a copy whose liveness witness is out of order does not.
fn rechecks(what: &str, p: &SpatialProgram, c: &ChoreographyCertificate) {
    assert!(c.recheck(p), "{what}: the certificate passes the independent checker against the program it travels with");
    assert!(!c.with_witness_tampered().recheck(p), "{what}: a certificate whose liveness witness is out of order is refused");
}

/// `largo emit rdu --chip sn50 <source> -o <out>` of a program written to `dir`.
fn emit_file(dir: &Path, name: &str, source: &str) -> std::path::PathBuf {
    let file = dir.join(format!("{name}.lg"));
    std::fs::write(&file, source).unwrap();
    let out = dir.join(name);
    let o = cli::largo_in(dir, &["emit", "rdu", "--chip", "sn50", file.to_str().unwrap(), "-o", out.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "{name}: largo emit rdu failed:\n{}", cli::stderr(&o));
    out
}

/// E8.8 — every program's placement certificate passes the independent checker: each program of each shared
/// architecture's SN50 bundle, an integer product with its refutation, and a product over several PCUs.
#[test]
fn every_placement_certificate_rechecks() {
    let dir = tempfile::tempdir().unwrap();
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let out = dir.path().join(format!("{t:?}"));
        let o = cli::largo_in(
            dir.path(),
            &["emit", "rdu", "--chip", "sn50", "--model", m.path.to_str().unwrap(), "--ctx", &CTX.to_string(), "-o", out.to_str().unwrap()],
        );
        assert_eq!(o.status.code(), Some(0), "{t:?}: largo emit rdu failed:\n{}", cli::stderr(&o));
        let manifest: Value = serde_json::from_str(&std::fs::read_to_string(out.join("manifest.json")).unwrap()).unwrap();
        let mut programs: Vec<(String, Value)> = vec![("embed".into(), manifest["embed"]["schedule_digest"].clone())];
        programs.extend(manifest["kinds"].as_array().expect("kinds").iter().map(|k| (k["dir"].as_str().unwrap().to_string(), k["schedule_digest"].clone())));
        programs.push(("head".into(), manifest["head"]["schedule_digest"].clone()));
        let read: Vec<(String, SpatialProgram, ChoreographyCertificate)> = programs
            .iter()
            .map(|(sub, digest)| {
                let what = format!("{t:?} {sub}");
                let (p, c) = shipped(&out.join(sub), &what);
                assert_eq!(serde_json::json!(format!("{:016x}", c.schedule_digest)), *digest, "{what}: the certificate is of the program the manifest records");
                rechecks(&what, &p, &c);
                (what, p, c)
            })
            .collect();
        // A certificate is of its own program: each refused against every other program of the bundle.
        for (what, _, c) in &read {
            for (other, p, _) in read.iter().filter(|(o, _, _)| o != what) {
                assert!(!c.recheck(p), "{what}'s certificate is refused against {other}'s program");
            }
        }
    }

    let wide = emit_file(dir.path(), "wide", WIDE);
    let (p, c) = shipped(&wide, "the wide product");
    assert!(p.sections.len() > 1, "the wide product is composed of sections");
    rechecks("the wide product", &p, &c);

    let integer = emit_file(dir.path(), "integer", INTEGER);
    let (p, c) = shipped(&integer, "the integer product");
    rechecks("the integer product", &p, &c);
    assert!(
        c.compute.iter().any(|(_, s)| matches!(s, ComputeStageCert::Sat(_))),
        "the integer product's tile mapping carries its refutation: {:?}",
        c.compute.iter().map(|(s, _)| s).collect::<Vec<_>>()
    );
    // The refutation as it travels, its last step dropped: the empty clause no longer follows, and the checker says so.
    let mut text: Value = serde_json::from_str(&std::fs::read_to_string(integer.join("certificate.whole.json")).unwrap()).unwrap();
    let steps = text["compute"]
        .as_array_mut()
        .expect("the compute certificates")
        .iter_mut()
        .find_map(|e| e[1]["Sat"]["steps"].as_array_mut())
        .expect("a SAT certificate travels with its steps");
    assert!(steps.pop().is_some(), "the refutation has steps");
    let cut = parse_certificate(&text.to_string()).expect("the cut certificate still reads back");
    assert!(!cut.recheck(&p), "a refutation with its last step dropped is refused");
}

/// A certificate whose SAT refutation proves some other unsatisfiable formula, not its stage's obligation: refused, the
/// stage named. The forged refutation is sound on its own — the PR/RUP checker accepts it against the formula it carries
/// — so only the obligation, rebuilt from the program, can tell it from the stage's own.
#[test]
fn a_certificate_refuting_another_formula_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let integer = emit_file(dir.path(), "integer", INTEGER);
    let (p, c) = shipped(&integer, "the integer product");
    assert!(c.checked(&p).is_ok(), "the integer product's own certificate rechecks: {:?}", c.checked(&p));
    let whole: Value = serde_json::from_str(&std::fs::read_to_string(integer.join("certificate.whole.json")).unwrap()).unwrap();
    let (k, stage) = c
        .compute
        .iter()
        .enumerate()
        .find_map(|(k, (s, cert))| matches!(cert, ComputeStageCert::Sat(_)).then_some((k, s.idx())))
        .expect("the integer product's tile mapping carries its refutation");
    let own = &whole["compute"][k][1]["Sat"];
    let (vars, clauses) = (own["num_vars"].as_u64().unwrap(), own["cnf"].as_array().unwrap().len());
    assert!(clauses >= 2, "the obligation has clauses to stand in for");
    // `¬x ∧ x`, refuted in one RUP step: over one variable, and padded to the obligation's own variable and clause counts.
    let small = serde_json::json!({ "num_vars": 1, "cnf": [[-1], [1]], "steps": [{ "rup": [] }] });
    let mut padded_cnf = vec![serde_json::json!([-1])];
    padded_cnf.extend((1..clauses).map(|_| serde_json::json!([1])));
    let padded = serde_json::json!({ "num_vars": vars, "cnf": padded_cnf, "steps": [{ "rup": [] }] });
    for (what, formula) in [("a formula of one variable", small), ("a formula of the obligation's own shape", padded)] {
        let mut forged = whole.clone();
        forged["compute"][k][1]["Sat"] = formula;
        let forged = parse_certificate(&forged.to_string()).unwrap_or_else(|e| panic!("{what}: the forged certificate reads back: {e}"));
        let ComputeStageCert::Sat(refutation) = &forged.compute[k].1 else { panic!("{what}: the forgery keeps its stage's SAT certificate") };
        assert!(refutation.recheck(), "{what}: the forged refutation is a sound refutation of the formula it carries");
        let fault = forged.checked(&p).expect_err(&format!("{what}: a certificate refuting another formula is refused"));
        assert!(!forged.recheck(&p), "{what}: the checker refuses it");
        let name = p.stages[stage].name.clone();
        assert!(
            matches!(&fault, RecheckFault::ForeignFormula { stage: s, name: n, .. } if *s == stage && *n == name),
            "{what}: the refusal names stage {stage} `{name}` and its foreign formula: {fault:?}"
        );
        let said = fault.to_string();
        assert!(said.contains(&name) && said.contains("not the stage's obligation"), "{what}: the refusal says so by name: {said}");
    }
}

/// A certificate of `¬x ∧ x`, refuted in one RUP step, padded to `own`'s variable and clause counts: sound on its own,
/// and of another formula.
fn padded_like(own: &PlacementCertificate) -> PlacementCertificate {
    let shipped = serde_json::to_value(own).expect("a placement certificate travels");
    let (vars, clauses) = (shipped["num_vars"].as_u64().unwrap(), shipped["cnf"].as_array().unwrap().len());
    assert!(clauses >= 2, "the obligation has clauses to stand in for");
    let mut cnf = vec![serde_json::json!([-1])];
    cnf.extend((1..clauses).map(|_| serde_json::json!([1])));
    let forged: PlacementCertificate = serde_json::from_value(serde_json::json!({ "num_vars": vars, "cnf": cnf, "steps": [{ "rup": [] }] }))
        .expect("the forged certificate reads back");
    assert!(forged.recheck(), "the forged refutation is a sound refutation of the formula it carries");
    assert_eq!(forged.num_vars(), own.num_vars(), "the forgery has the obligation's own variable count");
    forged
}

/// `fault` refuses a foreign formula for a subject naming every one of `names`.
fn foreign(what: &str, fault: Result<(), PlacementFault>, names: &[&str]) {
    let fault = fault.expect_err(&format!("{what}: a certificate refuting another formula is refused"));
    assert!(matches!(fault, PlacementFault::ForeignFormula { .. }), "{what}: refused for its foreign formula: {fault:?}");
    let said = fault.to_string();
    for name in names {
        assert!(fault.subject().contains(name), "{what}: the refusal names {name}: {said}");
    }
    assert!(said.contains("not the placement's obligation"), "{what}: the refusal says so: {said}");
}

/// A placement certificate whose refutation proves another formula, padded to the obligation's own shape: refused by
/// the reader of `certify_external_placement`, by every target backend's reader of `TargetBackend::certify`, and by the
/// loop lane's reader of the proof its row ships — each naming what it was offered for. The loop lane's reader also
/// refuses the obligation's own digest beside steps that do not refute it.
#[test]
fn a_placement_certificate_refuting_another_formula_is_refused() {
    // The processing element `psum + a·w`, placed by our own router as a stand-in for a vendor's placement.
    let source = vec![FabricOp { op: AluOp::Mul, a: Src::In(0), b: Src::In(2) }, FabricOp { op: AluOp::Add, a: Src::In(1), b: Src::Val(0) }];
    let cells: Vec<LoopCell> = source.iter().map(|o| LoopCell::Arith { op: o.op, a: o.a, b: o.b }).collect();
    let grid = route_loop_body(&cells, 3, &[Src::Val(1)]);
    let own = certify_external_placement(&grid, &source, 1, 8).expect("the PE placement earns its certificate");
    assert!(recheck_external_placement(&own, &grid, &source, 1, 8).is_ok(), "the placement's own certificate rechecks");
    let forged = padded_like(&own);
    foreign("import-and-certify", recheck_external_placement(&forged, &grid, &source, 1, 8), &["output 0", "cell 1"]);

    let backends: Vec<Box<dyn TargetBackend>> =
        vec![Box::new(SkyVerilogBackend::new()), Box::new(RduBackend::for_chip(RduChip::Sn50)), Box::new(PlasticineBackend::isca17())];
    for b in &backends {
        let what = format!("the {} backend", b.name());
        let cert = b.certify(&grid, &source, 1, 8).unwrap_or_else(|| panic!("{what}: the PE placement earns its certificate"));
        assert!(b.recheck(&cert, &grid, &source, 1, 8).is_ok(), "{what}: its own certificate rechecks");
        foreign(&what, b.recheck(&forged, &grid, &source, 1, 8), &[b.name(), "output 0"]);
    }

    // The loop lane: the row of `dot`'s bundle, as a reader of the bundle has it.
    let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/rdu_demo/dot.lg")).expect("the dot demo");
    let dir = tempfile::tempdir().unwrap();
    let bundle = emit_file(dir.path(), "dot", &src);
    let report: Value = serde_json::from_str(&std::fs::read_to_string(bundle.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["lane"], "loop", "dot is lowered by the loop lane");
    let row: CertificateRow = serde_json::from_value(report["plasticine_row"]["certificate"].clone()).expect("the loop lane's row ships its certificate");
    let lf = counted_loop_fabric(&src, "dot").expect("dot is a counted loop");
    assert!(row.proof.is_some() && row.rechecked, "the row ships its proof, rechecked");
    assert!(recheck_body(&lf, "dot", &row).is_ok(), "the row's own proof rechecks against the rebuilt loop body");
    let lane = lower_loop_lane(&src, RduChip::Sn50).expect("dot lowers by the loop lane");
    let forged = padded_like(lane.plasticine.certificate.as_ref().expect("the loop body's certificate"));
    let mut other = row.clone();
    other.proof = Some(forged.proof());
    foreign("the loop lane", recheck_body(&lf, "dot", &other), &["stage `dot`"]);
    let mut steps = serde_json::to_value(row.proof.as_ref().unwrap()).unwrap();
    steps["steps"] = serde_json::to_value(forged.proof()).unwrap()["steps"].clone();
    let mut wrong = row.clone();
    wrong.proof = Some(serde_json::from_value(steps).expect("the proof reads back"));
    let fault = recheck_body(&lf, "dot", &wrong).expect_err("the obligation's digest beside steps that do not refute it is refused");
    assert!(matches!(&fault, PlacementFault::Refutation { subject } if subject.contains("stage `dot`")), "the refusal names the stage and its refutation: {fault:?}");
}
