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

/// `div` side-exits on BOTH its edge cases: a zero divisor and the one
/// overflowing quotient `i64::MIN / -1` (exact arithmetic — the deopt hands
/// it to the promoting tiers rather than wrapping).
fn pre_div_defined<'c>(ctx: &'c Context, a: &BV<'c>, b: &BV<'c>) -> Bool<'c> {
    let min = BV::from_i64(ctx, i64::MIN, 64);
    let neg1 = BV::from_i64(ctx, -1, 64);
    let overflow = Bool::and(ctx, &[&a._eq(&min), &b._eq(&neg1)]);
    Bool::and(ctx, &[&pre_divisor_nonzero(ctx, a, b), &overflow.not()])
}

/// Integer arithmetic is EXACT: an op whose true value escapes i64 side-exits
/// (deopt → BigInt promotion) instead of wrapping. The precondition for the
/// total binops is therefore "the exact result fits in signed 64-bit" — proved
/// by comparing the 128-bit exact value against the sign-extension of the
/// wrapped 64-bit result (equal iff no overflow). The witness harness then
/// requires both machine code and reference to side-exit when this fails.
fn no_overflow<'c>(exact128: &BV<'c>, narrow64: &BV<'c>) -> Bool<'c> {
    exact128._eq(&narrow64.sign_ext(64))
}

fn pre_add_no_overflow<'c>(_ctx: &'c Context, a: &BV<'c>, b: &BV<'c>) -> Bool<'c> {
    no_overflow(&a.sign_ext(64).bvadd(&b.sign_ext(64)), &a.bvadd(b))
}

fn pre_sub_no_overflow<'c>(_ctx: &'c Context, a: &BV<'c>, b: &BV<'c>) -> Bool<'c> {
    no_overflow(&a.sign_ext(64).bvsub(&b.sign_ext(64)), &a.bvsub(b))
}

fn pre_mul_no_overflow<'c>(_ctx: &'c Context, a: &BV<'c>, b: &BV<'c>) -> Bool<'c> {
    no_overflow(&a.sign_ext(64).bvmul(&b.sign_ext(64)), &a.bvmul(b))
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
            kind: SpecKind::Checked,
            build: || MicroOp::Add { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_add_no_overflow,
            result: |_c, a, b| a.bvadd(b),
        },
        OpSpec {
            name: "sub",
            kind: SpecKind::Checked,
            build: || MicroOp::Sub { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_sub_no_overflow,
            result: |_c, a, b| a.bvsub(b),
        },
        OpSpec {
            name: "mul",
            kind: SpecKind::Checked,
            build: || MicroOp::Mul { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_mul_no_overflow,
            result: |_c, a, b| a.bvmul(b),
        },
        OpSpec {
            name: "div",
            kind: SpecKind::Checked,
            build: || MicroOp::Div { dst: 2, lhs: 0, rhs: 1 },
            pre: pre_div_defined,
            // On the defined domain (no zero divisor, no MIN/-1 overflow)
            // bvsdiv truncates toward zero — exactly the kernel's `/`.
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

/// Prove a single spec inhabited: some (a, b, r) satisfies `pre ∧ (r = post)`,
/// in its own fresh Z3 context. [`prove_all_satisfiable`] is exactly this over
/// every spec in [`all_specs`]; exposing the per-spec gate lets the test layer
/// shard the (multi-minute) satisfiability sweep across specs without re-deriving
/// the encoding.
pub fn prove_spec_satisfiable(spec: &OpSpec) -> Result<(), String> {
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
    Ok(())
}

/// Every spec is inhabited: some (a, b, r) satisfies pre ∧ post. Returns
/// the number of specs checked.
pub fn prove_all_satisfiable() -> Result<usize, String> {
    let specs = all_specs();
    for spec in &specs {
        prove_spec_satisfiable(spec)?;
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

/// Prove the div spec EXCLUDES `i64::MIN / -1`: its precondition is
/// unsatisfiable at that input, so the spec never claims a value where the
/// machine side-exits (exact arithmetic — the deopt hands the overflowing
/// quotient to the promoting tiers instead of wrapping).
pub fn prove_min_div_excluded() -> Result<(), String> {
    let cfg = z3::Config::new();
    let ctx = Context::new(&cfg);
    let solver = z3::Solver::new(&ctx);
    let min = BV::from_i64(&ctx, i64::MIN, 64);
    let neg1 = BV::from_i64(&ctx, -1, 64);
    solver.assert(&pre_div_defined(&ctx, &min, &neg1));
    match solver.check() {
        z3::SatResult::Unsat => Ok(()),
        _ => Err("div precondition admits MIN / -1 — the side-exit case leaked into the spec".to_string()),
    }
}
