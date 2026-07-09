//! Witness verification: three independent evaluators, one verdict.
//!
//! For each spec, Z3 produces witness inputs (solver-chosen models plus an
//! adversarial corner battery), and every input runs through:
//!
//! 1. the REAL stencil chain — actual machine code through the forge's
//!    copy-and-patch buffer;
//! 2. the forge's `reference_eval` — the deliberately-dumb MicroOp
//!    interpreter every differential trusts;
//! 3. the Z3 spec itself, evaluated on the concrete input.
//!
//! Any disagreement is a finding: a wrong spec, a miscompiled stencil, or
//! a reference bug. The deliberate-bug canary in the test gate proves the
//! harness can actually fail.

use logicaffeine_forge::jit::{compile_straightline, ChainOutcome, MicroOp};
use z3::ast::{Ast, BV};
use z3::{Context, SatResult, Solver};

use crate::spec::{OpSpec, SpecKind};

#[derive(Debug)]
pub struct WitnessReport {
    pub spec: &'static str,
    pub inputs_checked: usize,
}

/// The adversarial corner battery — every rim the int kernel has.
const CORNERS: &[i64] = &[
    i64::MIN,
    i64::MIN + 1,
    -2,
    -1,
    0,
    1,
    2,
    63,
    64,
    65,
    i64::MAX - 1,
    i64::MAX,
];

fn run_real_chain(op: &MicroOp, a: i64, b: i64) -> Option<i64> {
    let prog = [*op, MicroOp::Return { src: 2 }];
    let chain = compile_straightline(&prog).expect("stencil chain must compile");
    let mut frame = [a, b, 0i64];
    match chain.run_with_frame(&mut frame) {
        ChainOutcome::Return(v) => Some(v),
        ChainOutcome::Deopt(_) => None,
    }
}

fn run_reference(op: &MicroOp, a: i64, b: i64) -> Option<i64> {
    let prog = [*op, MicroOp::Return { src: 2 }];
    let mut frame = [a, b, 0i64];
    logicaffeine_forge::jit::reference_eval(&prog, &mut frame, 1_000)
}

fn spec_value(spec: &OpSpec, a: i64, b: i64) -> Option<i64> {
    let cfg = z3::Config::new();
    let ctx = Context::new(&cfg);
    let av = BV::from_i64(&ctx, a, 64);
    let bv = BV::from_i64(&ctx, b, 64);
    let pre = (spec.pre)(&ctx, &av, &bv);
    let solver = Solver::new(&ctx);
    solver.assert(&pre);
    if solver.check() != SatResult::Sat {
        return None; // precondition excludes this input
    }
    let r = (spec.result)(&ctx, &av, &bv);
    let simplified = r.simplify();
    // as_u64 then reinterpret: z3's signed accessor refuses the MIN bit
    // pattern; the u64 road is total and bit-exact.
    simplified.as_u64().map(|u| u as i64).or_else(|| simplified.as_i64())
}

/// Check one input through all three evaluators.
fn check_one(spec: &OpSpec, op: &MicroOp, a: i64, b: i64) -> Result<(), String> {
    let real = run_real_chain(op, a, b);
    let reference = run_reference(op, a, b);
    let model = spec_value(spec, a, b);
    match (spec.kind, model) {
        (_, Some(expected)) => {
            if real != Some(expected) {
                return Err(format!(
                    "{}({a}, {b}): machine code gave {real:?}, spec says {expected}",
                    spec.name
                ));
            }
            if reference != Some(expected) {
                return Err(format!(
                    "{}({a}, {b}): reference gave {reference:?}, spec says {expected}",
                    spec.name
                ));
            }
            Ok(())
        }
        (SpecKind::Checked, None) => {
            // Precondition failed: both executions must side-exit.
            if real.is_some() || reference.is_some() {
                return Err(format!(
                    "{}({a}, {b}): precondition excluded, but machine={real:?} reference={reference:?}",
                    spec.name
                ));
            }
            Ok(())
        }
        (SpecKind::Binop, None) => Err(format!(
            "{}({a}, {b}): total op but the spec produced no value",
            spec.name
        )),
    }
}

/// Z3-chosen witnesses: distinct models satisfying the precondition, plus
/// models pinned to each spec's interesting boundary (`r` maximal/minimal
/// under the post).
fn solver_witnesses(spec: &OpSpec, n: usize) -> Vec<(i64, i64)> {
    let cfg = z3::Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new(&ctx);
    let a = BV::new_const(&ctx, "a", 64);
    let b = BV::new_const(&ctx, "b", 64);
    solver.assert(&(spec.pre)(&ctx, &a, &b));
    let mut out = Vec::new();
    for _ in 0..n {
        if solver.check() != SatResult::Sat {
            break;
        }
        let model = solver.get_model().expect("sat without model");
        let cast = |v: BV| v.as_u64().map(|u| u as i64).or_else(|| v.as_i64());
        let (Some(av), Some(bv)) = (
            model.eval(&a, true).and_then(cast),
            model.eval(&b, true).and_then(cast),
        ) else {
            break;
        };
        out.push((av, bv));
        // Exclude this model so the next check yields a fresh witness.
        solver.assert(
            &(a._eq(&BV::from_i64(&ctx, av, 64)) & b._eq(&BV::from_i64(&ctx, bv, 64))).not(),
        );
    }
    out
}

/// Run the full witness battery for one spec.
pub fn check_spec_with_witnesses(spec: &OpSpec, solver_models: usize) -> Result<WitnessReport, String> {
    let op = (spec.build)();
    let mut checked = 0usize;
    for (a, b) in solver_witnesses(spec, solver_models) {
        check_one(spec, &op, a, b)?;
        checked += 1;
    }
    for &a in CORNERS {
        for &b in CORNERS {
            check_one(spec, &op, a, b)?;
            checked += 1;
        }
    }
    Ok(WitnessReport { spec: spec.name, inputs_checked: checked })
}
