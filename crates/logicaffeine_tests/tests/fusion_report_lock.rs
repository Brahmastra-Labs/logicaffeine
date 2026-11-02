//! Every compile reports its fusions (benchmarks/FUSIONS_WE_NEED.md, OPT-3): each place a rule could fire, and what
//! became of it — fired, refused by the chip with its reason, or held — in the order the table decided, with a digest an
//! artifact records beside its program's identity.
//!
//!   cargo nextest run -p logicaffeine-tests --test fusion_report_lock

#![cfg(feature = "codegen")]

#[path = "compile_common/tensor_probes.rs"]
mod tensor_probes;
#[path = "common/lane_cuts.rs"]
mod lane_cuts;
#[path = "common/scratch.rs"]
mod scratch;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::codegen_cuda::forward_shim::ForwardShim;
use logicaffeine_compile::codegen_cuda::knobs::CudaKnobs;
use logicaffeine_compile::codegen_cuda::lane::{compile_batch_holding, FormKind, LaneOptions};
use logicaffeine_compile::codegen_cuda::lane_shim::LaneShim;
use logicaffeine_compile::codegen_cuda::qwen_emit::Q8MatKind;
use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
use logicaffeine_compile::tensor_graph::fuse::{optimize, Outcome, RULES};
use logicaffeine_compile::tensor_graph::graph::{Storage, TensorGraph};
use logicaffeine_compile::tensor_graph::shim::{Linking, ReferenceShim, Shim, Support};
use logicaffeine_compile::tensor_graph::symbols::ClassId;

/// Every tiny architecture's cuts under each lane numerics, with the shim each compiles under, and every tensor probe
/// under the reference shim and the older forward's.
fn corpus() -> Vec<(String, TensorGraph, Box<dyn Shim>)> {
    let mut out: Vec<(String, TensorGraph, Box<dyn Shim>)> = Vec::new();
    for &t in tiny_models::ALL {
        let spec = t.spec(24);
        let ckpt = scratch::dir("logos_fusion_report_lock").join(format!("{}.gguf", t.name()));
        tiny_models::write_checkpoint(&spec, &ckpt);
        for activations in [Q8MatKind::Dequant, Q8MatKind::TwoPassInt8] {
            let knobs = CudaKnobs { activations, ..CudaKnobs::default() };
            let lane = compile_batch_holding(&spec, &ckpt, &LaneOptions { ctx: 24, knobs, prefill: 4 }, 2, &["sequences"])
                .unwrap_or_else(|e| panic!("{}: {e}", t.name()));
            for form in &lane.forms {
                let what = format!("{} {activations:?} {}", t.name(), form.tag());
                let g = lane_cuts::lifted(form, &what);
                out.push((format!("{what} reference"), g.clone(), Box::new(ReferenceShim)));
                out.push((what, g, Box::new(LaneShim { knobs, batch: form.kind == FormKind::Batch, held: Vec::new() })));
            }
        }
    }
    for p in tensor_probes::OP_PROBES {
        let Ok(rp) = recognize_stages(p.program) else { continue };
        let Ok(g) = TensorGraph::lift(&rp, None, None, &|_| Storage::Values) else { continue };
        out.push((format!("probe {} reference", p.op), g.clone(), Box::new(ReferenceShim)));
        out.push((format!("probe {} forward", p.op), g, Box::new(ForwardShim { kind: Q8MatKind::Int8Act })));
    }
    out
}

/// Each rule's place in the table: the rules a compile runs before it are decided the same whoever links it.
fn earlier(rule: &str) -> Vec<&'static str> {
    RULES.iter().map(|r| r.name).take_while(|n| *n != rule).collect()
}

/// The report lists every firing, in order; the reference shim refuses nothing; and every place a rule the chip does not
/// link would fire — had the chip linked its classes whole — is reported refused there, with the chip's reason.
#[test]
fn every_rule_that_could_fire_is_reported() {
    let (mut unlinked_reported, mut shape_refusals) = (0usize, 0usize);
    for (what, g, shim) in corpus() {
        let o = optimize(&g, shim.as_ref(), &[]);
        let fired: Vec<(&str, ClassId, Vec<usize>)> = o.fired.iter().map(|f| (f.rule, f.class, f.covers.clone())).collect();
        let reported: Vec<(&str, ClassId, Vec<usize>)> =
            o.report.decisions.iter().filter(|d| d.outcome == Outcome::Fired).map(|d| (d.rule, d.class, d.covers.clone())).collect();
        assert_eq!(reported, fired, "{what}: the report lists every firing, in order");
        if shim.name() == "reference" {
            assert!(o.report.decisions.iter().all(|d| d.outcome == Outcome::Fired), "{what}: the reference shim refuses nothing");
            continue;
        }
        // A rule the chip links, refused where the chip's link does not build the candidate's shape: reported, with why.
        for d in &o.report.decisions {
            let Outcome::Refused(why) = &d.outcome else { continue };
            if shim.supports(d.class) == Support::Native {
                assert!(why.split_whitespace().count() >= 4, "{what}: `{}` is refused without a reason: `{why}`", d.rule);
                shape_refusals += 1;
            }
        }
        for rule in RULES {
            let linked = rule.builds.iter().all(|c| shim.supports(*c) == Support::Native);
            let shared_class = RULES.iter().take_while(|r| r.name != rule.name).any(|r| r.builds.iter().any(|c| rule.builds.contains(c)));
            if linked || shared_class {
                continue;
            }
            let twin = optimize(&g, &Linking { base: shim.as_ref(), classes: rule.builds }, &[]);
            let would: Vec<(ClassId, Vec<usize>)> = twin.fired.iter().filter(|f| f.rule == rule.name).map(|f| (f.class, f.covers.clone())).collect();
            let declined: Vec<(ClassId, Vec<usize>)> = o.report.of(rule.name).iter().map(|d| (d.class, d.covers.clone())).collect();
            assert_eq!(declined, would, "{what}: `{}`, which the chip does not link, is reported wherever it would fire", rule.name);
            for d in o.report.of(rule.name) {
                let Outcome::Refused(why) = &d.outcome else { panic!("{what}: an unlinked `{}` is reported refused", rule.name) };
                assert!(why.split_whitespace().count() >= 4, "{what}: `{}` is refused without a reason: `{why}`", rule.name);
                unlinked_reported += 1;
            }
        }
    }
    assert!(unlinked_reported > 0, "the corpus holds rules a chip declines");
    assert!(shape_refusals > 0, "the corpus holds candidates a chip's link refuses by their shape");
}

/// Holding a rule reports it held wherever it would have fired, and leaves every decision of the rules before it as it was.
#[test]
fn holding_a_rule_moves_it_to_declined_and_changes_nothing_else() {
    let mut held_reported = 0usize;
    for (what, g, shim) in corpus() {
        let all = optimize(&g, shim.as_ref(), &[]);
        for rule in RULES {
            let fired: Vec<(ClassId, Vec<usize>)> = all.fired.iter().filter(|f| f.rule == rule.name).map(|f| (f.class, f.covers.clone())).collect();
            if fired.is_empty() {
                continue;
            }
            let held = optimize(&g, shim.as_ref(), &[rule.name]);
            assert_eq!(held.fired(rule.name), 0, "{what}: `{}` held fires", rule.name);
            let declined: Vec<(ClassId, Vec<usize>)> =
                held.report.of(rule.name).iter().filter(|d| d.outcome == Outcome::Held).map(|d| (d.class, d.covers.clone())).collect();
            assert_eq!(declined, fired, "{what}: `{}` held is reported held wherever it fired", rule.name);
            let before = earlier(rule.name);
            let decided = |o: &logicaffeine_compile::tensor_graph::fuse::Optimized| {
                o.report.decisions.iter().filter(|d| before.contains(&d.rule)).cloned().collect::<Vec<_>>()
            };
            assert_eq!(decided(&held), decided(&all), "{what}: holding `{}` changes no decision of the rules before it", rule.name);
            held_reported += declined.len();
        }
    }
    assert!(held_reported > 0, "the corpus holds firings to hold");
}

/// The report is a function of the graph, the table and the chip: compiled twice it is the same, decision for decision and
/// digest for digest, and a compile that decides anything differently has another digest.
#[test]
fn the_report_is_deterministic() {
    let mut differing = 0usize;
    for (what, g, shim) in corpus() {
        let (a, b) = (optimize(&g, shim.as_ref(), &[]), optimize(&g, shim.as_ref(), &[]));
        assert_eq!(a.report, b.report, "{what}: the report is the same compiled twice");
        assert_eq!(a.report.digest(), b.report.digest(), "{what}: so is its digest");
        assert_eq!(a.report.digest().len(), 64, "{what}: a sha256 in hex");
        if let Some(f) = a.fired.first() {
            let held = optimize(&g, shim.as_ref(), &[f.rule]);
            assert_ne!(held.report.digest(), a.report.digest(), "{what}: holding `{}` changes the digest", f.rule);
            differing += 1;
        }
    }
    assert!(differing > 0, "the corpus holds firings");
}
