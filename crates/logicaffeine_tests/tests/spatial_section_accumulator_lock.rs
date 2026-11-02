//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! EVERY SECTION ACCUMULATES UNDER ITS OWN PLAN.
//!
//! A composed program runs sections lowered under different plans: a float matmul accumulates in F32, an
//! integer one in I32 or I64, and a section whose result is an index — an arg-max — carries an integer plan.
//! The composed program's own `work.plan` is only its LAST section's. An accumulator acquired by an op of
//! section k takes section k's accumulator policy, and a packed tile is narrowed by it; reading the program's
//! plan instead gives every section the last one's, and a forward that ends in `argmax of logits` then hands
//! its float matmuls integer accumulators.
//!
//! Both directions are pinned against the VM, exactly: a float matmul before an integer-result section, and an
//! integer matmul before a float section.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test spatial_section_accumulator_lock`
//!
//! ★ MUTATION PROBE (run when this landed): `AcquireAcc` reading `self.p.work.plan` — the program's plan — reds
//! both: `operand/accumulator mix for F32` on the float matmul and `… for I32` on the integer one. Drop
//! `require_one_plan` from the Tensix printer — the printer test reds, Tensix refusing instead for "a semaphore
//! increment outside a multicast group": the right answer for the wrong reason, which names nothing true.
//!
//! And the printers: Tensix, a printer of one plan, refuses a composition of several by name; the fabric and the
//! general GPU program, printers of a plan per section, run it to the evaluator's answer.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::eval::{eval_choreography, Value};
use logicaffeine_compile::spatial::ir::{AccPolicy, SpatialProgram};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::rdu_chip::RduChip;

mod common;
#[path = "common/cuda_run.rs"]
mod cuda_run;
#[path = "common/fabric_rtl.rs"]
mod fabric_rtl;
#[path = "common/spatial.rs"]
mod spatial_support;
use spatial_support::inputs_from_fills;

fn composed(src: &str) -> SpatialProgram {
    let target = RduChip::Sn50.facts().spatial_target();
    let l = compile_program_to_spatial(src, &target).expect("a tensor program");
    l.program
        .unwrap_or_else(|| panic!("lowers whole and certifies: {:?}", l.missing))
}

fn tensor(
    p: &SpatialProgram,
    out: &logicaffeine_compile::spatial::eval::Outputs,
    name: &str,
) -> Vec<f64> {
    let id = p
        .tensors
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("`{name}`"))
        .id;
    match out
        .tensors
        .get(&id)
        .unwrap_or_else(|| panic!("`{name}` is an output"))
    {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

fn plans(p: &SpatialProgram) -> Vec<Option<AccPolicy>> {
    p.sections.iter().map(|s| s.plan.map(|pl| pl.acc)).collect()
}

/// ── A FLOAT MATMUL BEFORE AN INTEGER RESULT. The decode step's `logits` then `argmax of logits`.
#[test]
fn a_float_matmul_before_an_integer_result_accumulates_in_float() {
    let mut src = String::from(
        "## Main\nLet X be a 1 by 64 tensor of Real.\nLet W be a 64 by 32 tensor of Real.\n",
    );
    for i in 1..=64 {
        src += &format!(
            "Set item {i} of X to {:?}.\n",
            ((i * 7) % 13) as f64 / 16.0 - 0.375
        );
    }
    for i in 1..=64 * 32 {
        src += &format!(
            "Set item {i} of W to {:?}.\n",
            ((i * 5) % 11) as f64 / 16.0 - 0.3125
        );
    }
    src += "Let Z be X matmul W.\nLet t be argmax of Z.\n";
    let p = composed(&src);
    let ps = plans(&p);
    assert_eq!(
        ps.first().copied().flatten(),
        Some(AccPolicy::F32),
        "the matmul section accumulates in F32: {ps:?}"
    );
    assert_ne!(
        ps.last().copied().flatten(),
        Some(AccPolicy::F32),
        "the precondition: the LAST section's plan is not float: {ps:?}"
    );
    let out =
        eval_choreography(&p, &inputs_from_fills(&p)).expect("the composed program evaluates");
    let mut shown = src.clone();
    shown += "Show t.\n";
    let vm = vm_outcome(&shown);
    assert!(vm.error.is_none(), "{:?}", vm.error);
    assert_eq!(
        tensor(&p, &out, "t"),
        vec![vm.output.trim().parse::<f64>().expect("an index")],
        "the arg-max is the VM's"
    );
}

/// ── AN INTEGER MATMUL BEFORE A FLOAT SECTION. Exact, because integer arithmetic is.
#[test]
fn an_integer_matmul_before_a_float_section_accumulates_in_integers() {
    let mut src = String::from("## Main\nLet A be a 1 by 64 tensor of Int.\nLet B be a 64 by 32 tensor of Int.\nLet X be a 1 by 64 tensor of Real.\n");
    for i in 1..=64 {
        src += &format!("Set item {i} of A to {}.\n", (i * 7) % 13 - 6);
    }
    for i in 1..=64 * 32 {
        src += &format!("Set item {i} of B to {}.\n", (i * 5) % 11 - 5);
    }
    for i in 1..=64 {
        src += &format!(
            "Set item {i} of X to {:?}.\n",
            ((i * 3) % 7) as f64 / 8.0 - 0.375
        );
    }
    src += "Let C be A matmul B.\nLet S be sigmoid of X.\n";
    let p = composed(&src);
    let ps = plans(&p);
    assert!(
        matches!(
            ps.first().copied().flatten(),
            Some(AccPolicy::I32 | AccPolicy::I64)
        ),
        "the matmul section accumulates in integers: {ps:?}"
    );
    let out =
        eval_choreography(&p, &inputs_from_fills(&p)).expect("the composed program evaluates");
    let mut shown = src.clone();
    for i in 1..=32 {
        shown += &format!("Show item {i} of C.\n");
    }
    let vm = vm_outcome(&shown);
    assert!(vm.error.is_none(), "{:?}", vm.error);
    let want: Vec<f64> = vm
        .output
        .split_whitespace()
        .map(|t| t.parse().expect("an integer"))
        .collect();
    assert_eq!(
        tensor(&p, &out, "C"),
        want,
        "the integer matmul is the VM's, exactly"
    );
}

/// ── A PRINTER OF ONE PLAN REFUSES SECTIONS UNDER SEVERAL; A PRINTER OF A PLAN PER SECTION RUNS EACH UNDER ITS OWN.
/// Tensix emits one kernel shape from one `BlockPlan`: handed a composition whose sections were lowered under different
/// plans, the only plan it could read is the last section's, the wrong kernel for every other section, so it refuses
/// by name. The fabric and the general GPU program print every section under its own plan, and each runs the
/// composition to exactly the evaluator's answer: the fabric's RTL under iverilog in its twin's cycles, the GPU program
/// on the RTX 3060.
#[test]
fn a_composition_of_several_plans_is_refused_by_a_printer_of_one_and_run_by_a_printer_of_each() {
    use logicaffeine_compile::spatial::lower::cuda::CudaPersistentBackend;
    use logicaffeine_compile::spatial::lower::fabric::FabricBackend;
    use logicaffeine_compile::spatial::lower::tensix::TensixBackend;
    use logicaffeine_compile::spatial::lower::{SpatialArtifact, SpatialBackend, SpatialLowerError};
    use logicaffeine_compile::spatial::target::SpatialTarget;
    // Shapes whose two products run under different plans on every printer and fit one GPU block's shared memory, and
    // units, so each product's range stays inside int8 and the GPU's integer engine computes both exactly.
    let mut src = String::from("## Main\nLet X be a 8 by 16 tensor of Int.\nLet W be a 16 by 8 tensor of Int.\nLet U be a 8 by 64 tensor of Int.\n");
    for (t, items) in [("X", [(1, 1), (20, -1), (128, 1)]), ("W", [(1, 1), (50, -1), (128, 1)]), ("U", [(1, 1), (100, -1), (512, 1)])] {
        for (i, v) in items {
            src += &format!("Set item {i} of {t} to {v}.\n");
        }
    }
    src += "Let Z be X matmul W.\nLet V be Z matmul U.\n";
    let backends: Vec<(&str, Box<dyn SpatialBackend>)> = vec![
        ("Tensix", Box::new(TensixBackend::new(SpatialTarget::wormhole()))),
        ("fabric", Box::new(FabricBackend::new(SpatialTarget::sky_fabric(64, 64)))),
        ("CUDA", Box::new(CudaPersistentBackend::new(SpatialTarget::rtx3060()))),
    ];
    for (name, backend) in backends {
        let l = compile_program_to_spatial(&src, backend.target()).expect("a tensor program");
        let p = l
            .program
            .unwrap_or_else(|| panic!("{name}: the chain composes and certifies: {:?}", l.missing));
        let section_plans: Vec<_> = p.sections.iter().map(|s| s.plan).collect();
        assert!(
            section_plans.iter().any(|pl| *pl != p.work.plan),
            "{name}: the precondition — the sections run under different plans: {section_plans:?}"
        );
        let inputs = inputs_from_fills(&p);
        match (name, backend.lower(&p)) {
            ("Tensix", Err(SpatialLowerError::Unsupported(why))) => assert!(why.contains("plan"), "Tensix: the refusal names the plans: {why}"),
            ("fabric", Ok(SpatialArtifact::Verilog(v))) => {
                if common::require_tool("iverilog") {
                    fabric_rtl::runs_as_the_evaluator("fabric × several plans", &p, backend.target(), &v, &inputs);
                }
            }
            ("CUDA", Ok(SpatialArtifact::Cuda(art))) => {
                if common::require_tool("nvcc") && common::require_gpu() {
                    cuda_run::runs_as_the_evaluator("CUDA × several plans", &p, &art.program_cu, &inputs);
                    if let Some(general) = &art.general {
                        cuda_run::runs_as_the_evaluator("CUDA × several plans (one block per site)", &p, &general.program_cu, &inputs);
                    }
                }
            }
            (name, other) => panic!("{name}: a composition of several plans, neither refused for its plans nor run section by section: {other:?}"),
        }
    }
}
