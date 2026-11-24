//! ★ LOCK — an e-graph rewrite's identity is a theorem the kernel checks. ★
//!
//! Every rewrite the e-graph optimizer applies carries a certificate. `egraph::rules::kernel_certificate` turns a
//! certificate into a theorem with a proof term: the identity's variables (each an arbitrary value of its type, nothing
//! assumed about it), the definitions of the LOGOS operations it mentions, the identity over them, and a term of that
//! type. This lock rebuilds the context from the variables and definitions alone, over the standard library, so a
//! certificate cannot assume anything else, and holds each proof to two kernels: `infer_type` and the independent
//! `recheck` must both give the statement.
//!
//! The ring rules are identities over `Int`, the runtime's exact integers (an overflow promotes, it never wraps), each
//! proved from axiom-free integer laws. The Bool rules are `∀ b0 … : Bool, lhs = rhs` over the `match`-defined
//! connectives, proved by splitting a variable until each case closes by computation. A false identity gets no proof.
//!
//! The division, remainder, shift, mask, copy and slice rules rewrite operations the standard library does not have:
//! `/` and `%` rounding toward zero, `>>`, `<<`, `&`, `|`, `^`, `copy of`, `length of`, `item i of`,
//! `items a through b of`. `egraph::kernel_semantics` defines each in the kernel with a body, and a certificate may
//! carry those definitions and no others. The kernel checks that a definition is well typed; this lock checks that it
//! computes what the runtime computes: on literal operands, the kernel's term converts with the runtime's answer, and
//! an answer off by one does not. A guarded rule's guard is load-bearing: without it the identity is false at a point
//! the lock names, and the guard is false there.
//!
//! The constant folder evaluates a Rust function, which no theorem states. Its certificate is an oracle instead: on a
//! grid of literal pairs, every value it folds is one the kernel's definition of the operation computes, and every
//! value it refuses is a zero divisor or lies outside `i64`. The lock runs the oracle on the folder and on folders
//! seeded with an error, each of which it rejects.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use logicaffeine_compile::ast::stmt::BinaryOpKind;
use logicaffeine_compile::interpreter::{ListRepr, RuntimeValue};
use logicaffeine_compile::optimize::egraph::kernel_semantics::{
    self, bitand, bitor, bitwise, bitxor, copy, index, len, list_drop, list_take, nth, power_of_two,
    prove_by_computation, prove_by_int_cases, shl, shr, slice, tdiv, tmod, BitOp,
};
use logicaffeine_compile::optimize::egraph::rules::{
    all, certificate_context, fold_agrees_with_kernel, fold_binop, kernel_certificate, BoolExpr, Certificate,
    FoldOp, KernelCertificate, RingExpr, Rewrite,
};
use logicaffeine_compile::semantics::arith::binary_op;
use logicaffeine_compile::semantics::collections::{index_get, length_of, slice as runtime_slice};
use logicaffeine_kernel::int_inductive::canonical_int;
use logicaffeine_kernel::tactics::{GoalState, TacticStep};
use logicaffeine_kernel::{
    axiom_dependencies, defeq_for_test, infer_type, normalize, recheck, BigInt, Context, Literal, Term, Universe,
};

/// `certificate` is a proof both kernels accept, in a context built from its variables and definitions alone, and every
/// definition it carries is one this lock holds to the runtime.
fn holds(rule: &str, certificate: &KernelCertificate) -> Result<(), String> {
    let KernelCertificate { variables, definitions, statement, proof } = certificate;
    let pinned = kernel_semantics::definitions();
    for (name, ty, body) in definitions {
        if !pinned.iter().any(|(n, t, b)| n == name && t == ty && b == body) {
            return Err(format!("{rule}: `{name}` is not a definition this lock holds to the runtime"));
        }
    }
    let ctx = certificate_context(variables, definitions).map_err(|e| format!("{rule}: {e}"))?;
    let inferred = infer_type(&ctx, proof).map_err(|e| format!("{rule}: the kernel rejects the proof: {e:?}"))?;
    if !defeq_for_test(&ctx, &inferred, statement) {
        return Err(format!("{rule}: the proof proves {inferred:?}, not {statement:?}"));
    }
    let rechecked = recheck(&ctx, proof).map_err(|e| format!("{rule}: the re-checker rejects the proof: {e:?}"))?;
    if !defeq_for_test(&ctx, &rechecked, statement) {
        return Err(format!("{rule}: the re-checker gives {rechecked:?}"));
    }
    Ok(())
}

/// The rule's certificate, which [`holds`].
fn accepted(rule: &Rewrite) -> Result<KernelCertificate, String> {
    let certificate = kernel_certificate(rule)?;
    holds(rule.name, &certificate)?;
    Ok(certificate)
}

/// [`accepted`], with every value the statement speaks of bound in it rather than declared, and neither the proof, the
/// statement nor a definition resting on an axiom.
fn accepted_from_nothing(rule: &Rewrite) -> Result<KernelCertificate, String> {
    let certificate = accepted(rule)?;
    if !certificate.variables.is_empty() {
        return Err(format!("{}: declares {:?} instead of binding them", rule.name, certificate.variables));
    }
    let ctx = certificate_context(&[], &certificate.definitions)?;
    let mut usage = axiom_dependencies(&ctx, &certificate.proof);
    usage.absorb(axiom_dependencies(&ctx, &certificate.statement));
    for (_, ty, body) in &certificate.definitions {
        usage.absorb(axiom_dependencies(&ctx, ty));
        usage.absorb(axiom_dependencies(&ctx, body));
    }
    if !usage.axioms.is_empty() || !usage.unresolved.is_empty() {
        return Err(format!("{}: rests on {usage:?}", rule.name));
    }
    Ok(certificate)
}

fn rules_where(kind: fn(&Certificate) -> bool) -> Vec<Rewrite> {
    all().into_iter().filter(|r| kind(&r.certificate)).collect()
}

fn rule(name: &str) -> Rewrite {
    all().into_iter().find(|r| r.name == name).unwrap_or_else(|| panic!("`{name}` is a registered rule"))
}

#[test]
fn every_ring_rule_carries_a_kernel_proof() {
    let ring = rules_where(|c| matches!(c, Certificate::Ring { .. }));
    assert!(ring.len() >= 10, "the ring rules are there to check: {}", ring.len());
    let refused: Vec<String> = ring.iter().filter_map(|r| accepted(r).err()).collect();
    assert!(refused.is_empty(), "every ring rule is a kernel theorem:\n{}", refused.join("\n"));
}

#[test]
fn a_false_ring_identity_gets_no_proof() {
    use RingExpr as R;
    let wrong = Rewrite {
        name: "wrong-distribution",
        apply: |_, _| None,
        certificate: Certificate::Ring {
            lhs: R::Mul(Box::new(R::Var(0)), Box::new(R::Add(Box::new(R::Var(1)), Box::new(R::Const(1))))),
            rhs: R::Add(Box::new(R::Mul(Box::new(R::Var(0)), Box::new(R::Var(1)))), Box::new(R::Const(1))),
        },
    };
    assert!(kernel_certificate(&wrong).is_err(), "x(y + 1) is not xy + 1, so no proof exists");
}

#[test]
fn a_certificate_states_its_own_rule() {
    // The statement is the rule's identity, not a tautology: swapping in another rule's sides changes it.
    let ring = rules_where(|c| matches!(c, Certificate::Ring { .. }));
    let statements: Vec<String> =
        ring.iter().map(|r| format!("{:?}", kernel_certificate(r).map(|c| c.statement).ok())).collect();
    let distinct: std::collections::BTreeSet<&String> = statements.iter().collect();
    assert_eq!(distinct.len(), statements.len(), "no two ring rules state the same theorem");
}

#[test]
fn every_bool_rule_carries_a_kernel_proof() {
    let bool_rules = rules_where(|c| matches!(c, Certificate::BoolCases { .. }));
    assert!(bool_rules.len() >= 8, "the Bool rules are there to check: {}", bool_rules.len());
    let refused: Vec<String> = bool_rules.iter().filter_map(|r| accepted(r).err()).collect();
    assert!(refused.is_empty(), "every Bool rule is a kernel theorem:\n{}", refused.join("\n"));
}

#[test]
fn a_false_bool_identity_gets_no_proof() {
    use BoolExpr as B;
    let wrong = Rewrite {
        name: "and-is-or",
        apply: |_, _| None,
        certificate: Certificate::BoolCases {
            vars: 2,
            lhs: B::And(Box::new(B::Var(0)), Box::new(B::Var(1))),
            rhs: B::Or(Box::new(B::Var(0)), Box::new(B::Var(1))),
        },
    };
    assert!(kernel_certificate(&wrong).is_err(), "a ∧ b is not a ∨ b, so no case closes for a ≠ b");
}

// ----- Terms -------------------------------------------------------------

fn g(name: &str) -> Term {
    Term::Global(name.to_string())
}

fn v(name: &str) -> Term {
    Term::Var(name.to_string())
}

fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}

fn apps(f: Term, args: &[Term]) -> Term {
    args.iter().cloned().fold(f, app)
}

fn pi(param: &str, ty: Term, body: Term) -> Term {
    Term::Pi { param: param.to_string(), param_type: Box::new(ty), body_type: Box::new(body) }
}

fn lam(param: &str, ty: Term, body: Term) -> Term {
    Term::Lambda { param: param.to_string(), param_type: Box::new(ty), body: Box::new(body) }
}

fn eq(carrier: Term, lhs: Term, rhs: Term) -> Term {
    apps(g("Eq"), &[carrier, lhs, rhs])
}

fn type0() -> Term {
    Term::Sort(Universe::Type(0))
}

fn int() -> Term {
    g("Int")
}

fn nat() -> Term {
    g("Nat")
}

fn succ(n: Term) -> Term {
    app(g("Succ"), n)
}

fn of_nat(n: Term) -> Term {
    app(g("Int_ofNat"), n)
}

fn list_of(elem: Term) -> Term {
    app(g("List"), elem)
}

fn option_of(elem: Term) -> Term {
    app(g("Option"), elem)
}

fn nat_literal(n: i64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_i64(n)))
}

fn int_add(a: Term, b: Term) -> Term {
    apps(g("Int_add"), &[a, b])
}

fn int_sub(a: Term, b: Term) -> Term {
    apps(g("Int_sub"), &[a, b])
}

fn int_le(a: Term, b: Term) -> Term {
    apps(g("Int_le"), &[a, b])
}

/// `Eq Bool b true`.
fn is_true(b: Term) -> Term {
    eq(g("Bool"), b, g("true"))
}

/// `x · 2^n` with the runtime's exact product.
fn times_power_of_two(x: Term, n: Term) -> Term {
    apps(g("mul"), &[x, of_nat(power_of_two(n))])
}

/// `item` as a kernel `Option Int`: `none Int` where the runtime names no item.
fn kernel_item(item: Option<i64>) -> Term {
    match item {
        Some(n) => apps(g("some"), &[int(), canonical_int(n)]),
        None => app(g("none"), int()),
    }
}

/// The list `items` as a kernel `List Int`.
fn kernel_list(items: &[i64]) -> Term {
    items.iter().rev().fold(app(g("nil"), int()), |tail, &n| apps(g("cons"), &[int(), canonical_int(n), tail]))
}

/// `Π (x : Int) (n : Nat), guard → identity` with its guard dropped.
fn without_guard(statement: &Term) -> Term {
    let Term::Pi { param: x, param_type: x_ty, body_type } = statement else {
        panic!("the statement binds x first: {statement}")
    };
    let Term::Pi { param: n, param_type: n_ty, body_type } = body_type.as_ref() else {
        panic!("the statement binds n second: {statement}")
    };
    let Term::Pi { body_type: identity, .. } = body_type.as_ref() else {
        panic!("the statement's guard is its third binder: {statement}")
    };
    pi(x, (**x_ty).clone(), pi(n, (**n_ty).clone(), (**identity).clone()))
}

/// `statement` without its binder `name`, which the rest of the statement does not mention.
fn without_binder(statement: &Term, name: &str) -> Term {
    match statement {
        Term::Pi { param, body_type, .. } if param == name => (**body_type).clone(),
        Term::Pi { param, param_type, body_type } => pi(param, (**param_type).clone(), without_binder(body_type, name)),
        other => panic!("the statement binds `{name}`: {other}"),
    }
}

/// The standard library with every trusted definition declared.
fn semantics_context() -> Context {
    certificate_context(&[], &kernel_semantics::definitions())
        .unwrap_or_else(|e| panic!("the kernel accepts every trusted definition: {e}"))
}

/// The kernel computes `term` to `value`: the two convert. On closed data the kernel decides this by evaluation, so a
/// definition that computes anything else fails it.
fn computes(ctx: &Context, term: &Term, value: &Term) -> bool {
    defeq_for_test(ctx, term, value)
}

/// The kernel computes `term` to the integer `value` and not to `value + 1`, so the comparison can fail.
fn computes_int(ctx: &Context, term: &Term, value: i64) -> bool {
    computes(ctx, term, &canonical_int(value)) && !computes(ctx, term, &canonical_int(value + 1))
}

/// The kernel's normal form of `term` is the integer `value` and not `value + 1`. A value the kernel reaches through
/// the standard library's `mod` on a 64-bit literal is normalized before it is compared: `normalize` computes `mod`
/// natively, where a conversion first tries the remainder by unfolding the division one step at a time.
fn evaluates_int(ctx: &Context, term: &Term, value: i64) -> bool {
    let normal = normalize(ctx, term);
    computes(ctx, &normal, &canonical_int(value)) && !computes(ctx, &normal, &canonical_int(value.wrapping_add(1)))
}

/// The natural `n`, which may lie past `i64`.
fn natural(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}

/// [`evaluates_int`] for a `Nat` term, read through `Int_ofNat`: a `Nat` value normalizes to successors of a literal
/// where `ofNat` of it normalizes to one literal.
fn evaluates_natural(ctx: &Context, term: &Term, value: u64) -> bool {
    let normal = normalize(ctx, &of_nat(term.clone()));
    computes(ctx, &normal, &of_nat(natural(value))) && !computes(ctx, &normal, &of_nat(natural(value.wrapping_add(1))))
}

/// The independent re-checker accepts `refl value` as a proof that `term` is the integer `value`, and does not accept
/// `value + 1`. The re-checker computes the standard library's `div` on a 64-bit literal natively, as `normalize` does,
/// and its `Nat_pow` natively, as the first kernel's conversion does.
fn rechecks_int(ctx: &Context, term: &Term, value: i64) -> bool {
    let accepts = |value: i64| {
        let claim = eq(int(), term.clone(), canonical_int(value));
        let cited = app(lam("h", claim, v("h")), apps(g("refl"), &[int(), canonical_int(value)]));
        recheck(ctx, &cited).is_ok()
    };
    accepts(value) && !accepts(value.wrapping_add(1))
}

// ----- The runtime ---------------------------------------------------------

fn runtime_int(result: Result<RuntimeValue, String>, what: &str) -> i64 {
    match result {
        Ok(RuntimeValue::Int(n)) => n,
        other => panic!("the runtime's {what} is an Int: {other:?}"),
    }
}

fn runtime_list(items: &[i64]) -> RuntimeValue {
    let values = items.iter().map(|&n| RuntimeValue::Int(n)).collect();
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(values))))
}

/// `item i of list` at the runtime: the item, or `None` where the runtime raises because the index names none.
fn runtime_item(list: &RuntimeValue, i: i64) -> Option<i64> {
    match index_get(list, &RuntimeValue::Int(i)) {
        Ok(RuntimeValue::Int(n)) => Some(n),
        Ok(other) => panic!("item {i} of a list of Ints is {other:?}"),
        Err(_) => None,
    }
}

/// `items start through end of list` at the runtime.
fn runtime_items(list: &RuntimeValue, start: i64, end: i64) -> Vec<i64> {
    let sliced = runtime_slice(list, &RuntimeValue::Int(start), &RuntimeValue::Int(end));
    let Ok(RuntimeValue::List(items)) = sliced else {
        panic!("items {start} through {end} of a list of Ints is a list: {sliced:?}")
    };
    let values = items.borrow().to_values();
    values
        .into_iter()
        .map(|value| match value {
            RuntimeValue::Int(n) => n,
            other => panic!("an item of a list of Ints is an Int: {other:?}"),
        })
        .collect()
}

/// Lists of every length the index arithmetic distinguishes: empty, one item, several, negative and zero items.
fn sample_lists() -> Vec<Vec<i64>> {
    vec![vec![], vec![7], vec![10, 20, 30], vec![5, -3, 0, 42, -17], (1..=12).map(|k| k * k - 40).collect()]
}

// ----- Each trusted definition, held to the runtime ------------------------

/// `/` on integers: for every divisor the runtime accepts, the runtime's quotient, which rounds toward zero.
fn division_is_the_runtimes(ctx: &Context) {
    let divisors: Vec<i64> = (-9..=9).filter(|&b| b != 0).chain([-128, -100, -64, -16, 16, 64, 100, 128]).collect();
    for a in (-30..=30).chain([-256, -255, -128, -127, -64, -63, 63, 64, 127, 128, 255, 256]) {
        for &b in &divisors {
            let runtime = runtime_int(
                binary_op(BinaryOpKind::Divide, RuntimeValue::Int(a), RuntimeValue::Int(b)),
                &format!("{a} / {b}"),
            );
            assert_eq!(Some(runtime), a.checked_div(b), "the runtime's {a} / {b} truncates, as `checked_div` does");
            assert!(
                computes(ctx, &tdiv(canonical_int(a), canonical_int(b)), &canonical_int(runtime)),
                "the kernel's {a} / {b} is not the runtime's {runtime}"
            );
        }
    }
    for (a, b, quotient) in [(-7, 2, -3), (7, -2, -3), (-7, -2, 3), (7, 2, 3), (-1, 2, 0), (1, -2, 0)] {
        let term = tdiv(canonical_int(a), canonical_int(b));
        assert!(computes_int(ctx, &term, quotient), "{a} / {b} rounds toward zero, to {quotient}");
        assert!(!computes(ctx, &term, &canonical_int(quotient - 1)), "{a} / {b} is not {}", quotient - 1);
    }
    for a in [-5, 0, 5] {
        assert!(
            binary_op(BinaryOpKind::Divide, RuntimeValue::Int(a), RuntimeValue::Int(0)).is_err(),
            "the runtime raises on {a} / 0, so no rule's statement may divide by zero"
        );
        assert!(computes_int(ctx, &tdiv(canonical_int(a), canonical_int(0)), 0), "{a} / 0 is 0, as in Lean");
    }
    let rims = [i64::MIN, i64::MIN + 1, -(1 << 40), -7, -1, 1, 7, 1 << 40, i64::MAX - 1, i64::MAX];
    for a in rims {
        for b in rims {
            let quotient = binary_op(BinaryOpKind::Divide, RuntimeValue::Int(a), RuntimeValue::Int(b));
            let term = tdiv(canonical_int(a), canonical_int(b));
            match quotient {
                Ok(RuntimeValue::Int(q)) => {
                    assert!(evaluates_int(ctx, &term, q), "the kernel's {a} / {b} is not the runtime's {q}")
                }
                Ok(RuntimeValue::BigInt(q)) => {
                    let two_to_the_63 = BigInt::from_u64(1 << 63);
                    assert_eq!((a, b, &*q), (i64::MIN, -1, &two_to_the_63), "only i64::MIN / -1 leaves i64");
                    let exact = of_nat(natural(1 << 63));
                    assert!(computes(ctx, &normalize(ctx, &term), &exact), "the kernel's i64::MIN / -1 is 2^63");
                }
                other => panic!("the runtime's {a} / {b} is an integer: {other:?}"),
            }
        }
    }
}

/// `%` on integers: for every divisor the runtime accepts, from both rims of `i64` to around zero, the runtime's
/// remainder, which takes the dividend's sign; `i64::MIN % -1` is 0. It is the remainder of `/`: `a` is
/// `b · (a / b) + a % b`. A zero divisor raises at the runtime, and the kernel's remainder by 0 is the dividend, as in
/// Lean.
fn remainder_is_the_runtimes(ctx: &Context) {
    let operands = [
        i64::MIN,
        i64::MIN + 1,
        -(1 << 40),
        -65,
        -64,
        -9,
        -8,
        -7,
        -2,
        -1,
        0,
        1,
        2,
        7,
        8,
        9,
        64,
        65,
        1 << 40,
        i64::MAX - 1,
        i64::MAX,
    ];
    for a in operands {
        for b in operands.into_iter().filter(|&b| b != 0) {
            let runtime = runtime_int(
                binary_op(BinaryOpKind::Modulo, RuntimeValue::Int(a), RuntimeValue::Int(b)),
                &format!("{a} % {b}"),
            );
            assert_eq!(runtime, a.checked_rem(b).unwrap_or(0), "the runtime's {a} % {b} truncates, 0 at i64::MIN % -1");
            let remainder = tmod(canonical_int(a), canonical_int(b));
            assert!(evaluates_int(ctx, &remainder, runtime), "the kernel's {a} % {b} is not the runtime's {runtime}");
            let multiple = apps(g("mul"), &[canonical_int(b), tdiv(canonical_int(a), canonical_int(b))]);
            let rebuilt = apps(g("add"), &[multiple, remainder]);
            assert!(evaluates_int(ctx, &rebuilt, a), "{b} · ({a} / {b}) + {a} % {b} is {a}");
        }
    }
    for (a, b, remainder) in [(-7, 2, -1), (7, -2, 1), (-7, -2, -1), (7, 2, 1), (-8, 2, 0), (i64::MIN, -1, 0)] {
        let term = tmod(canonical_int(a), canonical_int(b));
        assert!(evaluates_int(ctx, &term, remainder), "{a} % {b} takes the dividend's sign: {remainder}");
        assert!(!evaluates_int(ctx, &term, remainder - 1), "{a} % {b} is not {}", remainder - 1);
    }
    for a in [-5, 0, 5] {
        assert!(
            binary_op(BinaryOpKind::Modulo, RuntimeValue::Int(a), RuntimeValue::Int(0)).is_err(),
            "the runtime raises on {a} % 0, so no rule's statement may take a remainder by zero"
        );
        assert!(evaluates_int(ctx, &tmod(canonical_int(a), canonical_int(0)), a), "{a} % 0 is {a}, as in Lean");
    }
}

/// Lean's `Nat.bitwise`: each combiner applied to the binary expansions of two naturals up to `2^64 - 1`, past `i64`,
/// and with fewer halvings than the bits, only the bits it reached.
fn bits_are_combined_pairwise(ctx: &Context) {
    let words: [u64; 10] = [0, 1, 2, 5, 6, 255, 1 << 32, (1 << 63) - 1, 1 << 63, u64::MAX];
    for combine in BitOp::ALL {
        for m in words {
            for n in words {
                let expected = match combine {
                    BitOp::And => m & n,
                    BitOp::Or => m | n,
                    BitOp::Xor => m ^ n,
                    BitOp::AndNot => m & !n,
                };
                let term = bitwise(combine.combiner(), nat_literal(64), natural(m), natural(n));
                assert!(evaluates_natural(ctx, &term, expected), "{combine:?} of {m} and {n} is {expected}");
            }
        }
    }
    let halted = bitwise(BitOp::And.combiner(), nat_literal(2), natural(0b1111), natural(0b1111));
    assert!(evaluates_natural(ctx, &halted, 0b11), "two halvings of 15 and 15 keep their two low bits");
}

/// `op`, one of the runtime's `&`, `|` and `^`, on two `i64`s from both rims to around zero: the kernel's `kernel`.
fn bitwise_op_is_the_runtimes(ctx: &Context, op: BinaryOpKind, kernel: fn(Term, Term) -> Term) {
    let operands = [i64::MIN, i64::MIN + 1, -(1 << 40), -65, -8, -1, 0, 1, 7, 64, 1 << 40, i64::MAX - 1, i64::MAX];
    for a in operands {
        for b in operands {
            let runtime =
                runtime_int(binary_op(op, RuntimeValue::Int(a), RuntimeValue::Int(b)), &format!("{a} {op:?} {b}"));
            assert!(
                evaluates_int(ctx, &kernel(canonical_int(a), canonical_int(b)), runtime),
                "the kernel's {a} {op:?} {b} is not the runtime's {runtime}"
            );
        }
    }
}

/// `&`: the runtime's two's-complement and of two `i64`s, and of a nonnegative bignum with a low-bit mask, which the
/// runtime reads as the remainder by the mask's power of two.
fn and_is_the_runtimes(ctx: &Context) {
    bitwise_op_is_the_runtimes(ctx, BinaryOpKind::BitAnd, bitand);
    let big = BigInt::from_u64(1).shl_bits(70).add(&BigInt::from_i64(5));
    let runtime = runtime_int(
        binary_op(BinaryOpKind::BitAnd, RuntimeValue::from_bigint(big.clone()), RuntimeValue::Int(7)),
        "(2^70 + 5) & 7",
    );
    assert_eq!(runtime, 5, "the runtime's (2^70 + 5) & 7 is 5");
    let term = bitand(of_nat(Term::Lit(Literal::Nat(big))), canonical_int(7));
    assert!(evaluates_int(ctx, &term, runtime), "the kernel's (2^70 + 5) & 7 is 5");
}

/// `|`: the runtime's two's-complement or of two `i64`s.
fn or_is_the_runtimes(ctx: &Context) {
    bitwise_op_is_the_runtimes(ctx, BinaryOpKind::BitOr, bitor);
}

/// `^`: the runtime's two's-complement exclusive or of two `i64`s.
fn xor_is_the_runtimes(ctx: &Context) {
    bitwise_op_is_the_runtimes(ctx, BinaryOpKind::BitXor, bitxor);
}

/// `>>`: the runtime's arithmetic shift for every amount below 64, where its reduction of the amount mod 64 changes
/// nothing.
fn shift_is_the_runtimes(ctx: &Context) {
    let check = |x: i64, n: i64| {
        let runtime = runtime_int(
            binary_op(BinaryOpKind::Shr, RuntimeValue::Int(x), RuntimeValue::Int(n)),
            &format!("{x} >> {n}"),
        );
        assert_eq!(runtime, x >> n, "the runtime's {x} >> {n} is the arithmetic shift");
        assert!(
            computes(ctx, &shr(canonical_int(x), nat_literal(n)), &canonical_int(runtime)),
            "the kernel's {x} >> {n} is not the runtime's {runtime}"
        );
    };
    for x in -9..=9 {
        for n in 0..64 {
            check(x, n);
        }
    }
    for x in [-256, -255, -65, -64, -63, -20, -17, -16, 16, 17, 20, 63, 64, 65, 255, 256] {
        for n in [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 15, 31, 32, 62, 63] {
            check(x, n);
        }
    }
    for (x, n, shifted) in [(-7, 1, -4), (-1, 1, -1), (-1, 62, -1), (-8, 3, -1), (-9, 3, -2), (7, 1, 3)] {
        let term = shr(canonical_int(x), nat_literal(n));
        assert!(computes_int(ctx, &term, shifted), "{x} >> {n} rounds toward negative infinity, to {shifted}");
        assert!(!computes(ctx, &term, &canonical_int(shifted - 1)), "{x} >> {n} is not {}", shifted - 1);
    }
    for x in [i64::MIN, i64::MIN + 1, -(1 << 40), 1 << 40, i64::MAX - 1, i64::MAX] {
        for n in [0, 1, 2, 31, 32, 62, 63] {
            let runtime = runtime_int(
                binary_op(BinaryOpKind::Shr, RuntimeValue::Int(x), RuntimeValue::Int(n)),
                &format!("{x} >> {n}"),
            );
            assert!(
                rechecks_int(ctx, &shr(canonical_int(x), nat_literal(n)), runtime),
                "the kernel's {x} >> {n} is not the runtime's {runtime}"
            );
        }
    }
}

/// `length of xs`: the runtime's count of the items.
fn length_is_the_runtimes(ctx: &Context) {
    for items in sample_lists() {
        let runtime = runtime_int(length_of(&runtime_list(&items)), &format!("length of {items:?}"));
        assert!(computes_int(ctx, &len(int(), kernel_list(&items)), runtime), "length of {items:?} is {runtime}");
    }
}

/// The item at a 0-based offset: the runtime's `item (offset + 1)`, and none past the end.
fn offset_is_the_runtimes(ctx: &Context) {
    for items in sample_lists() {
        let list = runtime_list(&items);
        for offset in 0..=items.len() as i64 + 2 {
            let runtime = runtime_item(&list, offset + 1);
            assert!(
                computes(ctx, &nth(int(), kernel_list(&items), nat_literal(offset)), &kernel_item(runtime)),
                "offset {offset} of {items:?} is {runtime:?}"
            );
        }
    }
}

/// `item i of xs`: the runtime's item for every index from past the back to past the front, and none exactly where
/// the runtime raises.
fn index_is_the_runtimes(ctx: &Context) {
    for items in sample_lists() {
        let list = runtime_list(&items);
        let count = items.len() as i64;
        for i in -(count + 3)..=count + 3 {
            let runtime = runtime_item(&list, i);
            assert!(
                computes(ctx, &index(int(), kernel_list(&items), canonical_int(i)), &kernel_item(runtime)),
                "item {i} of {items:?} is {runtime:?}"
            );
        }
    }
    let ten_twenty_thirty = kernel_list(&[10, 20, 30]);
    for (i, item, not) in [
        (1, Some(10), Some(20)),
        (3, Some(30), None),
        (-1, Some(30), Some(10)),
        (-3, Some(10), Some(30)),
        (0, None, Some(10)),
        (4, None, Some(30)),
        (-4, None, Some(10)),
    ] {
        let term = index(int(), ten_twenty_thirty.clone(), canonical_int(i));
        assert!(computes(ctx, &term, &kernel_item(item)), "item {i} of [10, 20, 30] is {item:?}");
        assert!(!computes(ctx, &term, &kernel_item(not)), "item {i} of [10, 20, 30] is not {not:?}");
    }
}

/// `copy of v`: the runtime's deep clone has the copied value's length and the same item at every index, once and
/// twice over, and the definition gives back the value it copies.
fn copy_is_the_runtimes(ctx: &Context) {
    let list_int = || list_of(int());
    for items in sample_lists() {
        let list = runtime_list(&items);
        let kernel = kernel_list(&items);
        let once = copy(list_int(), kernel.clone());
        let twice = copy(list_int(), once.clone());
        assert!(computes(ctx, &once, &kernel), "the copy of {items:?} is {items:?}");
        let count = items.len() as i64;
        for (copied, runtime_copy) in [(once, list.deep_clone()), (twice, list.deep_clone().deep_clone())] {
            let runtime_length = runtime_int(length_of(&runtime_copy), &format!("length of a copy of {items:?}"));
            assert_eq!(runtime_length, count, "the runtime's copy of {items:?} keeps its length");
            assert!(
                computes_int(ctx, &len(int(), copied.clone()), runtime_length),
                "length of a copy of {items:?} is {runtime_length}"
            );
            for i in -(count + 2)..=count + 2 {
                let runtime = runtime_item(&runtime_copy, i);
                assert_eq!(runtime, runtime_item(&list, i), "item {i} of the runtime's copy of {items:?}");
                assert!(
                    computes(ctx, &index(int(), copied.clone(), canonical_int(i)), &kernel_item(runtime)),
                    "item {i} of a copy of {items:?} is {runtime:?}"
                );
            }
        }
    }
    for n in [-5, 0, 7] {
        assert!(matches!(RuntimeValue::Int(n).deep_clone(), RuntimeValue::Int(m) if m == n), "a copy of {n} is {n}");
        assert!(computes_int(ctx, &copy(int(), canonical_int(n)), n), "the kernel's copy of {n} is {n}");
    }
}

/// `<<`: the runtime's wrapping shift for every amount below 64, where its reduction of the amount mod 64 changes
/// nothing: the exact product by `2^n`, wrapped to 64 bits.
fn shift_left_is_the_runtimes(ctx: &Context) {
    let check = |x: i64, n: i64| {
        let runtime = runtime_int(
            binary_op(BinaryOpKind::Shl, RuntimeValue::Int(x), RuntimeValue::Int(n)),
            &format!("{x} << {n}"),
        );
        assert_eq!(runtime, x.wrapping_shl(n as u32), "the runtime's {x} << {n} is the wrapping shift");
        assert!(
            evaluates_int(ctx, &shl(canonical_int(x), nat_literal(n)), runtime),
            "the kernel's {x} << {n} is not the runtime's {runtime}"
        );
    };
    for x in -9..=9 {
        for n in [0, 1, 2, 3, 31, 32, 33, 61, 62, 63] {
            check(x, n);
        }
    }
    for x in [i64::MIN, i64::MIN + 1, -(1 << 40), -65, -64, 63, 64, 65, 1 << 40, i64::MAX - 1, i64::MAX] {
        for n in [0, 1, 32, 62, 63] {
            check(x, n);
        }
    }
    for (x, n, shifted) in [
        (1, 63, i64::MIN),
        (2, 62, i64::MIN),
        (3, 62, -(1 << 62)),
        (-3, 62, 1 << 62),
        (-1, 63, i64::MIN),
        (i64::MAX, 1, -2),
        (i64::MIN, 1, 0),
    ] {
        let term = shl(canonical_int(x), nat_literal(n));
        assert!(evaluates_int(ctx, &term, shifted), "{x} << {n} wraps to {shifted}");
        assert!(!evaluates_int(ctx, &term, shifted.wrapping_sub(1)), "{x} << {n} is not {}", shifted.wrapping_sub(1));
    }
}

/// The first `count` items: the runtime's items 1 through `count` while the list has that many, and the whole list
/// past its end, as Lean's `List.take` is.
fn take_is_the_runtimes_prefix(ctx: &Context) {
    for items in sample_lists() {
        let list = runtime_list(&items);
        for count in 0..=items.len() + 2 {
            let prefix = &items[..count.min(items.len())];
            if count <= items.len() {
                assert_eq!(runtime_items(&list, 1, count as i64), prefix, "items 1 through {count} of {items:?}");
            }
            assert!(
                computes(ctx, &list_take(int(), nat_literal(count as i64), kernel_list(&items)), &kernel_list(prefix)),
                "the first {count} items of {items:?} are {prefix:?}"
            );
        }
    }
}

/// The items after the first `count`: the runtime's items `count + 1` through the length while the list has `count`
/// items, and none past its end, as Lean's `List.drop` is.
fn drop_is_the_runtimes_suffix(ctx: &Context) {
    for items in sample_lists() {
        let list = runtime_list(&items);
        let length = items.len() as i64;
        for count in 0..=items.len() + 2 {
            let suffix = &items[count.min(items.len())..];
            if count <= items.len() {
                let first = count as i64 + 1;
                assert_eq!(runtime_items(&list, first, length), suffix, "items {first} through {length} of {items:?}");
            }
            assert!(
                computes(ctx, &list_drop(int(), nat_literal(count as i64), kernel_list(&items)), &kernel_list(suffix)),
                "{items:?} after its first {count} items is {suffix:?}"
            );
        }
    }
}

/// `items start through end of xs`: the runtime's items, for every sample list of up to five items and every pair of
/// bounds from before the front to past the back and at the ends of `i64`; they are none wherever the slice runs
/// backwards or past the end.
fn slice_is_the_runtimes(ctx: &Context) {
    for items in sample_lists().into_iter().filter(|items| items.len() <= 5) {
        let list = runtime_list(&items);
        let length = items.len() as i64;
        let bounds: Vec<i64> = (-2..=length + 2).chain([i64::MIN, -64, 64, i64::MAX]).collect();
        for &start in &bounds {
            for &end in &bounds {
                let runtime = runtime_items(&list, start, end);
                assert!(
                    computes(
                        ctx,
                        &slice(int(), kernel_list(&items), canonical_int(start), canonical_int(end)),
                        &kernel_list(&runtime)
                    ),
                    "items {start} through {end} of {items:?} are {runtime:?}"
                );
            }
        }
    }
    let ten_twenty_thirty = kernel_list(&[10, 20, 30]);
    for (start, end, sliced, not) in [
        (1, 3, vec![10, 20, 30], vec![10, 20]),
        (2, 3, vec![20, 30], vec![10, 20]),
        (0, 2, vec![10, 20], vec![]),
        (3, 3, vec![30], vec![]),
        (3, 2, vec![], vec![30]),
        (2, 4, vec![], vec![20, 30]),
        (-1, 2, vec![], vec![10, 20]),
    ] {
        let term = slice(int(), ten_twenty_thirty.clone(), canonical_int(start), canonical_int(end));
        let of_ten_twenty_thirty = format!("items {start} through {end} of [10, 20, 30]");
        assert!(computes(ctx, &term, &kernel_list(&sliced)), "{of_ten_twenty_thirty} are {sliced:?}");
        assert!(!computes(ctx, &term, &kernel_list(&not)), "{of_ten_twenty_thirty} are not {not:?}");
    }
}

#[test]
fn every_trusted_definition_computes_as_the_runtime_does() {
    let ctx = semantics_context();
    let pins: [(&str, fn(&Context)); 15] = [
        (kernel_semantics::TDIV, division_is_the_runtimes),
        (kernel_semantics::TMOD, remainder_is_the_runtimes),
        (kernel_semantics::BITWISE, bits_are_combined_pairwise),
        (kernel_semantics::AND, and_is_the_runtimes),
        (kernel_semantics::OR, or_is_the_runtimes),
        (kernel_semantics::XOR, xor_is_the_runtimes),
        (kernel_semantics::SHR, shift_is_the_runtimes),
        (kernel_semantics::SHL, shift_left_is_the_runtimes),
        (kernel_semantics::LEN, length_is_the_runtimes),
        (kernel_semantics::NTH, offset_is_the_runtimes),
        (kernel_semantics::INDEX, index_is_the_runtimes),
        (kernel_semantics::COPY, copy_is_the_runtimes),
        (kernel_semantics::TAKE, take_is_the_runtimes_prefix),
        (kernel_semantics::DROP, drop_is_the_runtimes_suffix),
        (kernel_semantics::SLICE, slice_is_the_runtimes),
    ];
    for (_, pin) in &pins {
        pin(&ctx);
    }
    let pinned: BTreeSet<String> = pins.iter().map(|(name, _)| name.to_string()).collect();
    let defined: BTreeSet<String> = kernel_semantics::definitions().into_iter().map(|(name, _, _)| name).collect();
    assert_eq!(pinned, defined, "every trusted definition is held to the runtime");
}

#[test]
fn every_trusted_definition_has_a_body_and_rests_on_no_axiom() {
    let ctx = semantics_context();
    for (name, ty, body) in kernel_semantics::definitions() {
        assert_eq!(ctx.get_definition_body(&name), Some(&body), "`{name}` is declared with its body");
        let mut usage = axiom_dependencies(&ctx, &ty);
        usage.absorb(axiom_dependencies(&ctx, &body));
        assert!(usage.axioms.is_empty() && usage.unresolved.is_empty(), "`{name}` rests on {usage:?}");
    }
}

#[test]
fn a_certificate_cannot_rebind_a_library_name() {
    let (_, ty, body) = kernel_semantics::definitions().into_iter().next().expect("there is a trusted definition");
    let rebinding = [("Int_add".to_string(), ty, body)];
    assert!(certificate_context(&[], &rebinding).is_err(), "`Int_add` is the library's, and stays the library's");
    let assumed = [("False".to_string(), Term::Sort(Universe::Prop))];
    assert!(certificate_context(&assumed, &[]).is_err(), "a variable cannot stand in for the library's `False`");
    let twice = [kernel_semantics::definitions().remove(0), kernel_semantics::definitions().remove(0)];
    assert!(certificate_context(&[], &twice).is_err(), "a definition cannot rebind one declared before it");
    assert!(certificate_context(&[], &kernel_semantics::definitions()).is_ok(), "the trusted definitions add names");
}

#[test]
fn the_division_and_shift_rules_carry_kernel_proofs() {
    let guard = eq(g("Bool"), apps(g("Int_le"), &[of_nat(g("Zero")), v("x")]), g("true"));
    let shift = eq(int(), tdiv(v("x"), of_nat(power_of_two(v("n")))), shr(v("x"), v("n")));
    let by_one = eq(int(), tdiv(v("x"), of_nat(succ(g("Zero")))), v("x"));
    for (name, statement) in [
        ("div-pow2-shr", pi("x", int(), pi("n", nat(), pi("h", guard, shift)))),
        ("div-one", pi("x", int(), by_one)),
    ] {
        let certificate = accepted_from_nothing(&rule(name)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(certificate.statement, statement, "`{name}` states its rewrite over the runtime's `/` and `>>`");
    }
}

#[test]
fn the_copy_rules_carry_kernel_proofs() {
    let element = || v("A");
    let list = || list_of(element());
    let copied = || copy(list(), v("xs"));
    let len_copy = eq(int(), len(element(), copied()), len(element(), v("xs")));
    let index_copy = eq(option_of(element()), index(element(), copied(), v("i")), index(element(), v("xs"), v("i")));
    let copy_copy = eq(v("T"), copy(v("T"), copy(v("T"), v("x"))), copy(v("T"), v("x")));
    for (name, statement) in [
        ("len-copy", pi("A", type0(), pi("xs", list(), len_copy))),
        ("index-copy", pi("A", type0(), pi("xs", list(), pi("i", int(), index_copy)))),
        ("copy-copy", pi("T", type0(), pi("x", v("T"), copy_copy))),
    ] {
        let certificate = accepted_from_nothing(&rule(name)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(certificate.statement, statement, "`{name}` states its rewrite over `copy of`");
    }
}

#[test]
fn the_slice_rules_carry_kernel_proofs() {
    let element = || v("A");
    let list = || list_of(element());
    let one = || of_nat(succ(g("Zero")));
    let full = slice(element(), v("xs"), one(), len(element(), v("xs")));
    let full_copy = eq(list(), full, copy(list(), v("xs")));
    let counted = eq(
        int(),
        len(element(), slice(element(), v("xs"), v("a"), v("b"))),
        int_add(int_sub(v("b"), v("a")), one()),
    );
    let bounded = pi(
        "h1",
        is_true(int_le(one(), v("a"))),
        pi(
            "h2",
            is_true(int_le(v("a"), int_add(v("b"), one()))),
            pi("h3", is_true(int_le(v("b"), len(element(), v("xs")))), counted),
        ),
    );
    for (name, statement) in [
        ("slice-full-copy", pi("A", type0(), pi("xs", list(), full_copy))),
        ("len-slice-bounds", pi("A", type0(), pi("xs", list(), pi("a", int(), pi("b", int(), bounded))))),
    ] {
        let certificate = accepted_from_nothing(&rule(name)).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(certificate.statement, statement, "`{name}` states its rewrite over the runtime's slice");
    }
}

#[test]
fn the_shift_range_rule_carries_a_kernel_proof() {
    let product = || times_power_of_two(v("x"), v("n"));
    let guarded = pi(
        "within",
        is_true(apps(g("natLe"), &[v("n"), nat_literal(62)])),
        pi(
            "above",
            is_true(int_le(canonical_int(i64::MIN), product())),
            pi("below", is_true(int_le(product(), canonical_int(i64::MAX))), eq(int(), shl(v("x"), v("n")), product())),
        ),
    );
    let certificate = accepted_from_nothing(&rule("mul-pow2-shl")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        certificate.statement,
        pi("x", int(), pi("n", nat(), guarded)),
        "`mul-pow2-shl` states that the wrapping `<<` keeps an exact product inside i64"
    );
}

#[test]
fn every_kernel_certificate_a_rule_carries_is_accepted() {
    let mut statements = BTreeSet::new();
    let mut proved = Vec::new();
    for rule in all() {
        let Ok(certificate) = kernel_certificate(&rule) else { continue };
        holds(rule.name, &certificate).unwrap_or_else(|e| panic!("{e}"));
        assert!(statements.insert(format!("{:?}", certificate.statement)), "`{}` states another rule's theorem", rule.name);
        proved.push(rule.name);
    }
    for name in [
        "div-pow2-shr",
        "div-one",
        "len-copy",
        "index-copy",
        "copy-copy",
        "slice-full-copy",
        "len-slice-bounds",
        "mul-pow2-shl",
        "mod-pow2-and",
    ] {
        assert!(proved.contains(&name), "`{name}` carries a kernel proof: {proved:?}");
    }
}

#[test]
fn every_rule_has_a_kernel_certificate() {
    // The constant folder evaluates a Rust function, which no theorem states; its certificate is the kernel oracle
    // `the_constant_folder_agrees_with_the_kernel` runs. Every other rule is a kernel theorem.
    let unproved: Vec<(&str, String)> =
        all().iter().filter_map(|r| kernel_certificate(r).err().map(|why| (r.name, why))).collect();
    let names: Vec<&str> = unproved.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["const-fold"], "every rule but the constant folder carries a kernel proof: {unproved:?}");
    assert!(
        matches!(rule("const-fold").certificate, Certificate::Bitvector { theorem: None, .. }),
        "the constant folder's certificate is its check, the kernel oracle"
    );
}

#[test]
fn the_mask_rule_carries_a_kernel_proof() {
    let power = || of_nat(power_of_two(v("n")));
    let mask = int_sub(power(), of_nat(succ(g("Zero"))));
    let identity = eq(int(), tmod(v("x"), power()), bitand(v("x"), mask));
    let guarded = pi(
        "h",
        is_true(int_le(of_nat(g("Zero")), v("x"))),
        pi("within", is_true(apps(g("natLe"), &[v("n"), nat_literal(62)])), identity),
    );
    let certificate = accepted_from_nothing(&rule("mod-pow2-and")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        certificate.statement,
        pi("x", int(), pi("n", nat(), guarded)),
        "`mod-pow2-and` states that the runtime's `%` by 2^n of a nonnegative integer is its `&` with 2^n - 1"
    );
}

#[test]
fn a_mask_identity_without_its_sign_guard_gets_no_proof() {
    let ctx = semantics_context();
    let remainder =
        runtime_int(binary_op(BinaryOpKind::Modulo, RuntimeValue::Int(-1), RuntimeValue::Int(2)), "-1 % 2");
    let masked = runtime_int(binary_op(BinaryOpKind::BitAnd, RuntimeValue::Int(-1), RuntimeValue::Int(1)), "-1 & 1");
    assert_eq!((remainder, masked), (-1, 1), "the runtime's -1 % 2 is -1 and its -1 & 1 is 1");
    let two = || of_nat(power_of_two(nat_literal(1)));
    let lhs = tmod(canonical_int(-1), two());
    let rhs = bitand(canonical_int(-1), int_sub(two(), of_nat(succ(g("Zero")))));
    assert!(computes_int(&ctx, &lhs, -1), "the kernel's -1 % 2 is -1");
    assert!(computes_int(&ctx, &rhs, 1), "the kernel's -1 & (2 - 1) is 1");
    assert!(
        computes(&ctx, &int_le(of_nat(g("Zero")), canonical_int(-1)), &g("false")),
        "the guard `0 ≤ -1` is false"
    );
    let KernelCertificate { statement, definitions, .. } =
        kernel_certificate(&rule("mod-pow2-and")).unwrap_or_else(|e| panic!("{e}"));
    let unguarded = without_binder(&statement, "h");
    let truth = apps(g("refl"), &[g("Bool"), g("true")]);
    let args = [canonical_int(-1), nat_literal(1), truth];
    assert_eq!(
        conclusion_at(&unguarded, &args[..2]),
        eq(int(), lhs.clone(), rhs.clone()),
        "without its guard the rule claims -1 % 2 = -1 & (2 - 1)"
    );
    let certificate_ctx = certificate_context(&[], &definitions).unwrap_or_else(|e| panic!("{e}"));
    let proof = refutation(&certificate_ctx, &unguarded, &args, lhs, rhs)
        .unwrap_or_else(|e| panic!("the kernel refutes `mod-pow2-and` without `0 ≤ x`: {e}"));
    let refuted = pi("claim", unguarded, g("False"));
    let inferred =
        infer_type(&certificate_ctx, &proof).unwrap_or_else(|e| panic!("the kernel rejects the refutation: {e:?}"));
    assert!(defeq_for_test(&certificate_ctx, &inferred, &refuted), "the refutation proves {inferred}");
    let rechecked =
        recheck(&certificate_ctx, &proof).unwrap_or_else(|e| panic!("the re-checker rejects the refutation: {e:?}"));
    assert!(defeq_for_test(&certificate_ctx, &rechecked, &refuted), "the re-checker gives {rechecked}");
}

#[test]
fn the_constant_folder_agrees_with_the_kernel() {
    fold_agrees_with_kernel(fold_binop)
        .unwrap_or_else(|e| panic!("the constant folder disagrees with the kernel: {e}"));
}

/// [`fold_binop`] with `op`'s value changed by `wrong`.
fn folding_wrongly(op: FoldOp, wrong: fn(i64, i64) -> Option<i64>) -> impl Fn(FoldOp, i64, i64) -> Option<i64> {
    move |at, a, b| if at == op { wrong(a, b) } else { fold_binop(at, a, b) }
}

#[test]
fn a_constant_folder_seeded_with_an_error_disagrees_with_the_kernel() {
    let floored = folding_wrongly(FoldOp::Div, |a, b| {
        let quotient = if b == 0 { None } else { a.checked_div(b) }?;
        Some(if a % b != 0 && (a < 0) != (b < 0) { quotient - 1 } else { quotient })
    });
    let refusal = fold_agrees_with_kernel(floored).expect_err("a folder whose `/` floors disagrees with the kernel");
    assert!(refusal.starts_with("Div ") && refusal.contains("which the kernel does not compute"), "{refusal}");
    for op in FoldOp::ALL {
        let off_by_one = move |at: FoldOp, a: i64, b: i64| {
            let folded = fold_binop(at, a, b);
            if at == op { folded.map(|value| value.wrapping_add(1)) } else { folded }
        };
        let refusal = fold_agrees_with_kernel(off_by_one).expect_err("a folder off by one disagrees with the kernel");
        assert!(refusal.starts_with(&format!("{op:?} ")), "{op:?} off by one: {refusal}");
    }
    let raising = folding_wrongly(FoldOp::Mod, |a, b| a.checked_rem(b).or(Some(0)));
    let refusal = fold_agrees_with_kernel(raising).expect_err("a folder that folds `% 0` disagrees with the kernel");
    assert!(refusal.starts_with("Mod ") && refusal.contains("where the runtime raises"), "{refusal}");
    let refusing = folding_wrongly(FoldOp::Mod, |a, b| a.checked_rem(b));
    let refusal = fold_agrees_with_kernel(refusing).expect_err("a folder that leaves i64::MIN % -1 disagrees");
    assert_eq!(
        refusal,
        format!("Mod on ({}, -1) does not fold, and the kernel computes an i64", i64::MIN),
        "`checked_rem` refuses i64::MIN % -1, whose remainder is 0"
    );
    let wrapping = folding_wrongly(FoldOp::Mul, |a, b| Some(a.wrapping_mul(b)));
    let refusal = fold_agrees_with_kernel(wrapping).expect_err("a folder that wraps `*` disagrees with the kernel");
    assert!(refusal.starts_with("Mul ") && refusal.contains("which the kernel does not compute"), "{refusal}");
}

#[test]
fn a_division_identity_without_its_guard_gets_no_proof() {
    let ctx = semantics_context();
    let (halved, shifted) = (tdiv(canonical_int(-1), canonical_int(2)), shr(canonical_int(-1), nat_literal(1)));
    assert!(computes_int(&ctx, &halved, 0), "-1 / 2 rounds up to 0");
    assert!(computes_int(&ctx, &shifted, -1), "-1 >> 1 rounds down to -1");
    assert!(!computes(&ctx, &halved, &shifted), "so -1 / 2 and -1 >> 1 differ");
    let two = of_nat(succ(succ(g("Zero"))));
    let halving = pi("x", int(), eq(int(), tdiv(v("x"), two.clone()), shr(v("x"), succ(g("Zero")))));
    assert!(
        prove_by_int_cases(&halving, kernel_semantics::definitions()).is_err(),
        "x / 2 = x >> 1 fails at -1, so no proof exists"
    );
    let halving_is_identity = pi("x", int(), eq(int(), tdiv(v("x"), two), v("x")));
    assert!(
        prove_by_int_cases(&halving_is_identity, kernel_semantics::definitions()).is_err(),
        "x / 2 = x fails at 1"
    );
    let KernelCertificate { statement, definitions, .. } =
        kernel_certificate(&rule("div-pow2-shr")).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        prove_by_int_cases(&statement, definitions.clone()).is_ok(),
        "with its guard, the rule's statement is proved by cases"
    );
    assert!(
        prove_by_int_cases(&without_guard(&statement), definitions).is_err(),
        "without its guard, the negative case does not close"
    );
}

#[test]
fn a_false_copy_identity_gets_no_proof() {
    let element = || v("A");
    let consed = apps(g("cons"), &[element(), v("y"), v("xs")]);
    let longer = eq(int(), len(element(), copy(list_of(element()), consed.clone())), len(element(), v("xs")));
    let statement = pi("A", type0(), pi("xs", list_of(element()), pi("y", element(), longer)));
    assert!(
        prove_by_computation(&statement, kernel_semantics::definitions()).is_err(),
        "a copy of y :: xs is one item longer than xs"
    );
    let moved = eq(
        option_of(element()),
        index(element(), copy(list_of(element()), consed.clone()), canonical_int(1)),
        index(element(), consed, canonical_int(2)),
    );
    let statement = pi("A", type0(), pi("xs", list_of(element()), pi("y", element(), moved)));
    assert!(
        prove_by_computation(&statement, kernel_semantics::definitions()).is_err(),
        "item 1 of a copy is not item 2 of the original"
    );
}

/// A proof of `False` from `claim`, a statement whose instance at `args` is `lhs = rhs` for integers the kernel
/// computes on opposite sides of 0: `lhs` at most 0 and `rhs` above it.
fn refutation(ctx: &Context, claim: &Term, args: &[Term], lhs: Term, rhs: Term) -> Result<Term, String> {
    let at_most_zero = lam("z", int(), int_le(v("z"), canonical_int(0)));
    let sides = apps(g("IntLaw_congrArg"), &[int(), g("Bool"), at_most_zero, lhs.clone(), rhs.clone(), v("wrong")]);
    let script = [
        TacticStep::Intro(vec!["claim".to_string()]),
        TacticStep::Have("wrong".to_string(), eq(int(), lhs, rhs)),
        TacticStep::Exact(apps(v("claim"), args)),
        TacticStep::Exact(app(
            g("Eq_false_true_absurd"),
            apps(g("Eq_sym"), &[g("Bool"), g("true"), g("false"), sides]),
        )),
    ];
    let mut state = GoalState::new(ctx, &pi("claim", claim.clone(), g("False"))).map_err(|e| e.to_string())?;
    state.run(&script).map_err(|e| e.to_string())?;
    state.qed().map_err(|e| e.to_string())
}

/// The equation `statement` concludes with, at `args` for its leading binders, past the hypotheses that follow them.
fn conclusion_at(statement: &Term, args: &[Term]) -> Term {
    let mut rest = statement.clone();
    for arg in args {
        let Term::Pi { param, body_type, .. } = rest else { panic!("the statement binds {} values", args.len()) };
        rest = logicaffeine_kernel::substitute(&body_type, &param, arg);
    }
    while let Term::Pi { body_type, .. } = rest {
        rest = *body_type;
    }
    rest
}

#[test]
fn a_slice_length_without_its_bound_gets_no_proof() {
    let ctx = semantics_context();
    let items = [10, 20, 30];
    let one = || of_nat(succ(g("Zero")));
    assert!(
        runtime_items(&runtime_list(&items), 1, 4).is_empty(),
        "the runtime's items 1 through 4 of a list of 3 are none"
    );
    let sliced = len(int(), slice(int(), kernel_list(&items), canonical_int(1), canonical_int(4)));
    let counted = int_add(int_sub(canonical_int(4), canonical_int(1)), one());
    assert!(computes_int(&ctx, &sliced, 0), "the kernel's items 1 through 4 of [10, 20, 30] are none");
    assert!(computes_int(&ctx, &counted, 4), "4 - 1 + 1 is 4");
    assert!(
        computes(&ctx, &int_le(canonical_int(4), len(int(), kernel_list(&items))), &g("false")),
        "the guard `4 ≤ length of [10, 20, 30]` is false"
    );
    let KernelCertificate { statement, definitions, .. } =
        kernel_certificate(&rule("len-slice-bounds")).unwrap_or_else(|e| panic!("{e}"));
    let unbounded = without_binder(&statement, "h3");
    let truth = || apps(g("refl"), &[g("Bool"), g("true")]);
    let args = [int(), kernel_list(&items), canonical_int(1), canonical_int(4), truth(), truth()];
    assert_eq!(
        conclusion_at(&unbounded, &args[..4]),
        eq(int(), sliced.clone(), counted.clone()),
        "without its bound the rule claims items 1 through 4 of [10, 20, 30] number 4 - 1 + 1"
    );
    let certificate_ctx = certificate_context(&[], &definitions).unwrap_or_else(|e| panic!("{e}"));
    let proof = refutation(&certificate_ctx, &unbounded, &args, sliced, counted)
        .unwrap_or_else(|e| panic!("the kernel refutes `len-slice-bounds` without `b ≤ length of xs`: {e}"));
    let refuted = pi("claim", unbounded, g("False"));
    let inferred =
        infer_type(&certificate_ctx, &proof).unwrap_or_else(|e| panic!("the kernel rejects the refutation: {e:?}"));
    assert!(defeq_for_test(&certificate_ctx, &inferred, &refuted), "the refutation proves {inferred}");
    let rechecked =
        recheck(&certificate_ctx, &proof).unwrap_or_else(|e| panic!("the re-checker rejects the refutation: {e:?}"));
    assert!(defeq_for_test(&certificate_ctx, &rechecked, &refuted), "the re-checker gives {rechecked}");
}

#[test]
fn a_shift_identity_without_its_range_guard_gets_no_proof() {
    let ctx = semantics_context();
    let shifted = runtime_int(binary_op(BinaryOpKind::Shl, RuntimeValue::Int(2), RuntimeValue::Int(62)), "2 << 62");
    assert_eq!(shifted, i64::MIN, "the runtime's 2 << 62 wraps to i64::MIN");
    let exact = binary_op(BinaryOpKind::Multiply, RuntimeValue::Int(2), RuntimeValue::Int(1 << 62));
    assert!(
        matches!(&exact, Ok(RuntimeValue::BigInt(n)) if **n == BigInt::from_u64(1 << 63)),
        "the runtime's 2 * 2^62 is exact, 2^63, past i64: {exact:?}"
    );
    let KernelCertificate { statement, .. } =
        kernel_certificate(&rule("mul-pow2-shl")).unwrap_or_else(|e| panic!("{e}"));
    let unguarded = without_binder(&without_binder(&statement, "above"), "below");
    let identity = conclusion_at(&unguarded, &[canonical_int(2), nat_literal(62)]);
    let product = times_power_of_two(canonical_int(2), nat_literal(62));
    let shift = shl(canonical_int(2), nat_literal(62));
    assert_eq!(
        identity,
        eq(int(), shift.clone(), product.clone()),
        "without its range guards the rule claims 2 << 62 = 2 · 2^62"
    );
    assert!(
        computes(&ctx, &apps(g("natLe"), &[nat_literal(62), nat_literal(62)]), &g("true")),
        "62 is a shift the rule makes"
    );
    assert!(evaluates_int(&ctx, &shift, i64::MIN), "the kernel's 2 << 62 is i64::MIN");
    let two_to_the_63 = of_nat(Term::Lit(Literal::Nat(BigInt::from_u64(1 << 63))));
    assert!(computes(&ctx, &product, &two_to_the_63), "the kernel's 2 · 2^62 is 2^63");
    assert!(!computes(&ctx, &product, &canonical_int(i64::MIN)), "so the two sides differ");
    assert!(
        computes(&ctx, &int_le(canonical_int(i64::MIN), product.clone()), &g("true")),
        "2 · 2^62 is above i64::MIN"
    );
    assert!(
        computes(&ctx, &int_le(product, canonical_int(i64::MAX)), &g("false")),
        "and the guard `2 · 2^62 ≤ i64::MAX` is false there"
    );
}
