//! Z3 specifications for the JIT's integer micro-operations.
//!
//! Each spec relates two BV64 inputs `a`, `b` and one output `r` under a
//! precondition. The postcondition must determine `r` uniquely — the
//! satisfiability gate checks the spec is inhabited, the algebra gates
//! prove the laws the optimizer leans on, and the witness harness
//! (see [`crate::witness`]) grounds the spec against the real machine
//! code and the reference interpreter.

use logicaffeine_forge::jit::MicroOp;
use z3::ast::{Ast, Bool, BV};
use z3::Context;

/// Which micro-op family a spec describes, with the frame shape the
/// witness harness needs to build it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecKind {
    /// Plain 3-address op: `frame[2] = frame[0] op frame[1]`, total.
    Binop,
    /// Checked op (Div/Mod): side-exits when the precondition fails.
    Checked,
}

pub struct OpSpec {
    pub name: &'static str,
    pub kind: SpecKind,
    /// Build the micro-op computing `frame[2] = frame[0] op frame[1]`.
    pub build: fn() -> MicroOp,
    /// Precondition over (a, b).
    pub pre: for<'c> fn(&'c Context, &BV<'c>, &BV<'c>) -> Bool<'c>,
    /// Postcondition: the exact result value.
    pub result: for<'c> fn(&'c Context, &BV<'c>, &BV<'c>) -> BV<'c>,
}

fn pre_true<'c>(ctx: &'c Context, _a: &BV<'c>, _b: &BV<'c>) -> Bool<'c> {
    Bool::from_bool(ctx, true)
}

fn pre_divisor_nonzero<'c>(ctx: &'c Context, _a: &BV<'c>, b: &BV<'c>) -> Bool<'c> {
    b._eq(&BV::from_i64(ctx, 0, 64)).not()
}

/// The kernel's shift-amount rule: `wrapping_shl(b as u32)` masks the
/// amount to the low six bits.
fn shift_amount<'c>(ctx: &'c Context, b: &BV<'c>) -> BV<'c> {
    b.bvand(&BV::from_i64(ctx, 63, 64))
}

fn bool_to_bv<'c>(ctx: &'c Context, cond: Bool<'c>) -> BV<'c> {
    cond.ite(&BV::from_i64(ctx, 1, 64), &BV::from_i64(ctx, 0, 64))
}

pub fn all_specs() -> Vec<OpSpec> {
    vec![
        OpSpec {
            name: "add",
            kind: SpecKind::Binop,
            build: || MicroOp::Add { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_true,
            result: |_c, a, b| a.bvadd(b),
        },
        OpSpec {
            name: "sub",
            kind: SpecKind::Binop,
            build: || MicroOp::Sub { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_true,
            result: |_c, a, b| a.bvsub(b),
        },
        OpSpec {
            name: "mul",
            kind: SpecKind::Binop,
            build: || MicroOp::Mul { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_true,
            result: |_c, a, b| a.bvmul(b),
        },
        OpSpec {
            name: "div",
            kind: SpecKind::Checked,
            build: || MicroOp::Div { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_divisor_nonzero,
            // SMT bvsdiv wraps i64::MIN / -1 to i64::MIN and truncates
            // toward zero — exactly `wrapping_div`.
            result: |_c, a, b| a.bvsdiv(b),
        },
        OpSpec {
            name: "mod",
            kind: SpecKind::Checked,
            build: || MicroOp::Mod { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_divisor_nonzero,
            // bvsrem takes the DIVIDEND's sign — exactly `wrapping_rem`.
            result: |_c, a, b| a.bvsrem(b),
        },
        OpSpec {
            name: "and",
            kind: SpecKind::Binop,
            build: || MicroOp::BitAnd { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_true,
            result: |_c, a, b| a.bvand(b),
        },
        OpSpec {
            name: "or",
            kind: SpecKind::Binop,
            build: || MicroOp::BitOr { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_true,
            result: |_c, a, b| a.bvor(b),
        },
        OpSpec {
            name: "xor",
            kind: SpecKind::Binop,
            build: || MicroOp::BitXor { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_true,
            result: |_c, a, b| a.bvxor(b),
        },
        OpSpec {
            name: "shl",
            kind: SpecKind::Binop,
            build: || MicroOp::Shl { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_true,
            result: |c, a, b| a.bvshl(&shift_amount(c, b)),
        },
        OpSpec {
            name: "shr",
            kind: SpecKind::Binop,
            build: || MicroOp::Shr { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_true,
            // Arithmetic shift (the kernel's Int is signed).
            result: |c, a, b| a.bvashr(&shift_amount(c, b)),
        },
        OpSpec {
            name: "lt",
            kind: SpecKind::Binop,
            build: || MicroOp::Lt { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_true,
            result: |c, a, b| bool_to_bv(c, a.bvslt(b)),
        },
        OpSpec {
            name: "lteq",
            kind: SpecKind::Binop,
            build: || MicroOp::LtEq { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_true,
            result: |c, a, b| bool_to_bv(c, a.bvsle(b)),
        },
        OpSpec {
            name: "eq",
            kind: SpecKind::Binop,
            build: || MicroOp::Eq { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_true,
            result: |c, a, b| bool_to_bv(c, a._eq(b)),
        },
    ]
}

/// Every spec is inhabited: some (a, b, r) satisfies pre ∧ post. Returns
/// the number of specs checked.
pub fn prove_all_satisfiable() -> Result<usize, String> {
    let specs = all_specs();
    for spec in &specs {
        let cfg = z3::Config::new();
        let ctx = Context::new(&cfg);
        let solver = z3::Solver::new(&ctx);
        let a = BV::new_const(&ctx, "a", 64);
        let b = BV::new_const(&ctx, "b", 64);
        let r = BV::new_const(&ctx, "r", 64);
        solver.assert(&(spec.pre)(&ctx, &a, &b));
        solver.assert(&r._eq(&(spec.result)(&ctx, &a, &b)));
        if solver.check() != z3::SatResult::Sat {
            return Err(format!("spec '{}' is uninhabited", spec.name));
        }
    }
    Ok(specs.len())
}

/// Prove a binary algebraic law `f(a,b) == f(b,a)` for the named spec.
/// Returns Err with a counterexample description when the law FAILS.
pub fn prove_commutative(name: &'static str) -> Result<(), String> {
    let spec = all_specs()
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("no spec named {name}"))?;
    let cfg = z3::Config::new();
    let ctx = Context::new(&cfg);
    let solver = z3::Solver::new(&ctx);
    let a = BV::new_const(&ctx, "a", 64);
    let b = BV::new_const(&ctx, "b", 64);
    let lhs = (spec.result)(&ctx, &a, &b);
    let rhs = (spec.result)(&ctx, &b, &a);
    solver.assert(&lhs._eq(&rhs).not());
    match solver.check() {
        z3::SatResult::Unsat => Ok(()),
        z3::SatResult::Sat => Err(format!("{name} is not commutative: {:?}", solver.get_model())),
        z3::SatResult::Unknown => Err(format!("{name}: solver unknown")),
    }
}

/// A spec that CONTRADICTS the machine code (add claiming subtraction) —
/// the witness harness's canary: accepting it would mean the three-way
/// comparison cannot fail.
pub fn deliberately_wrong_spec_for_canary() -> OpSpec {
    OpSpec {
        name: "add-claiming-sub",
        kind: SpecKind::Binop,
        build: || MicroOp::Add { dst: 2, lhs: 0, rhs: 1 },
        pre: pre_true,
        result: |_c, a, b| a.bvsub(b),
    }
}

/// Prove `i64::MIN / -1 == i64::MIN` under the div spec (the locked
/// wrapping rim).
pub fn prove_min_div_wraps() -> Result<(), String> {
    let cfg = z3::Config::new();
    let ctx = Context::new(&cfg);
    let solver = z3::Solver::new(&ctx);
    let min = BV::from_i64(&ctx, i64::MIN, 64);
    let neg1 = BV::from_i64(&ctx, -1, 64);
    let r = min.bvsdiv(&neg1);
    solver.assert(&r._eq(&min).not());
    match solver.check() {
        z3::SatResult::Unsat => Ok(()),
        _ => Err("bvsdiv(MIN, -1) != MIN — spec model broken".to_string()),
    }
}
