//! Lean's `Nat.ble`, `Nat.blt` and `Nat.beq` on literals — a CHECKED acceleration in both kernels.
//!
//! Lean's kernel computes `Nat.ble` and `Nat.beq` on literals natively (GMP), and `Nat.blt n m`
//! unfolds to `Nat.ble (n+1) m`. `decide` on a `Nat` order or equality reduces to them, which is
//! how `_private.Init.Prelude.0.isValidChar_UInt32` (blocker `B-FALSE-REJECT-2f01b2e5`) proves
//! `1114112 < 4294967296`. Reducing these operands in unary needs up to 2^32 steps, far past the
//! 2^20-step conversion budget, so every acceptance below is decided by the checked acceleration or
//! not at all.
//!
//! The acceleration trusts no name and no axiom. A binary `Nat → Nat → C` program over a
//! two-constructor carrier `C` is accelerated only after the kernel that uses it proves, by its own
//! conversion, the program's computation equations:
//! * an order (`ble`): `f 0 n = yes` (or Lean's two cases `f 0 0 = yes`, `f 0 (n+1) = yes`),
//!   `f (m+1) 0 = no`, `f (m+1) (n+1) = f m n`;
//! * an equality (`beq`): `f 0 0 = yes`, `f 0 (n+1) = no`, `f (m+1) 0 = no`, `f (m+1) (n+1) = f m n`.
//!
//! A program with one different equation, an assumed program (an axiom has no body to prove the
//! equations with), and a program whose recursion passes through an axiom are never accelerated.
//!
//! Fixture: `fixtures/lean_scale/LG_NatCompare.lean`, exported with the pinned Lean 4.14.0 and the
//! pinned lean4export (structural expression keys); Lean's own `#print axioms` for every root is
//! recorded in its provenance.
#![allow(dead_code)]

#[path = "harvest_support/checker.rs"]
mod checker;
#[path = "lean_scale_support/witness.rs"]
mod witness;

use checker::{check_in, Root};
use logicaffeine_kernel::interface::lean::reference::Reference;
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{defeq_for_test, infer_type, recheck, BigInt, Context, Literal, Term};
use witness::{
    against, both_accept, both_refuse, kernel_spelling, map_literal, with_last_argument,
};

const STEM: &str = "LG_NatCompare";

/// Every root of the witness, as Lean records it: a theorem with no axioms.
const ROOTS: [&str; 10] = [
    "LG_NC.bleBound",
    "LG_NC.bleRefuses",
    "LG_NC.bltBound",
    "LG_NC.bltRefusesEqual",
    "LG_NC.beqSame",
    "LG_NC.beqDiffer",
    "LG_NC.decEqSame",
    "LG_NC.decNe",
    "LG_NC.decLe",
    "LG_NC.decNotLt",
];

fn fixture() -> String {
    witness::witness(STEM, &ROOTS).0
}

fn imported(
    export: &str,
) -> (
    logicaffeine_kernel::interface::lean::ImportedContext,
    logicaffeine_kernel::interface::lean::ImportReport,
) {
    witness::imported(export, &ROOTS).unwrap_or_else(|e| {
        panic!("the production reader must accept Lean's original comparisons: {e}")
    })
}

/// The other constructor of a two-constructor carrier (`true` ↔ `false`).
fn other_answer(ctx: &Context, answer: &Term) -> Term {
    let Term::Global(name) = answer else {
        panic!("a comparison answer is a constructor: {answer}")
    };
    let carrier = ctx
        .constructor_inductive(name)
        .expect("the answer is a constructor");
    let constructors = ctx.get_constructors(carrier);
    assert_eq!(
        constructors.len(),
        2,
        "the answer's carrier has two constructors"
    );
    let other = constructors
        .iter()
        .find(|(c, _)| *c != name.as_str())
        .expect("the other constructor")
        .0;
    Term::Global(other.to_string())
}

/// The false comparison offered against each root's retained proof: the answer flipped for the
/// `rfl` theorems, one operand moved across the boundary for the `decide` theorems.
fn falsified(ctx: &Context, root: &str, statement: &Term) -> Term {
    const CHAR_BOUND: u64 = 1_114_112;
    const TWO_32: u64 = 4_294_967_296;
    match root {
        "LG_NC.bleBound"
        | "LG_NC.bleRefuses"
        | "LG_NC.bltBound"
        | "LG_NC.bltRefusesEqual"
        | "LG_NC.beqSame"
        | "LG_NC.beqDiffer" => with_last_argument(statement, |a| other_answer(ctx, a)),
        // `2^32 = 2^32` becomes `2^32 = 2^32 - 1`.
        "LG_NC.decEqSame" => with_last_argument(statement, |a| map_literal(a, TWO_32, TWO_32 - 1)),
        // `2^32 ≠ 2^32 - 1` becomes `2^32 ≠ 2^32`.
        "LG_NC.decNe" => with_last_argument(statement, |a| map_literal(a, TWO_32 - 1, TWO_32)),
        // `1114112 ≤ 2^32` becomes `1114112 ≤ 1114111`.
        "LG_NC.decLe" => with_last_argument(statement, |a| map_literal(a, TWO_32, CHAR_BOUND - 1)),
        // `¬ (2^32 < 1114112)` becomes `¬ (2^32 < 2^32 + 1)`.
        "LG_NC.decNotLt" => {
            with_last_argument(statement, |a| map_literal(a, CHAR_BOUND, TWO_32 + 1))
        }
        other => panic!("no false comparison for {other}"),
    }
}

#[test]
fn leans_original_comparisons_import_at_their_statements_in_both_kernels_with_no_axioms() {
    let export = fixture();
    let reference = Reference::read(&export).expect("the reference translator reads the export");
    let (ctx, report) = imported(&export);
    let mut failures = Vec::new();
    for lean in ROOTS {
        // Reference statement ≡ imported statement; the retained proof and the declared constant
        // are each checked against the reference statement by BOTH kernels; the LOGOS assumptions
        // equal Lean's `#print axioms` (none).
        if let Err(why) = check_in(
            &export,
            &ctx,
            &report,
            &reference,
            &Root {
                lean,
                kind: "theorem",
                axioms: &[],
            },
        ) {
            failures.push(format!("{lean}: {why}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn a_false_comparison_is_refused_by_both_kernels() {
    let export = fixture();
    let reference = Reference::read(&export).expect("the reference translator reads the export");
    let (ctx, _) = imported(&export);
    for lean in ROOTS {
        let name = kernel_spelling(lean);
        let statement = reference
            .declaration(&name)
            .expect("reference translation")
            .ty
            .clone();
        let (_, proof) = ctx.retained(&name).expect("the proof is retained");
        both_accept(&ctx, &against(proof, &statement))
            .unwrap_or_else(|e| panic!("{lean} at its statement: {e}"));
        let altered = falsified(&ctx, lean, &statement);
        assert!(
            !defeq_for_test(&ctx, &statement, &altered),
            "{lean}: the altered comparison must differ: {altered}"
        );
        both_accept(&ctx, &altered)
            .unwrap_or_else(|e| panic!("{lean}: the false comparison is still a proposition: {e}"));
        assert!(
            both_refuse(&ctx, &against(proof, &altered)),
            "{lean}: its proof must be refused at the false comparison {altered} by both kernels"
        );
        assert!(
            both_refuse(&ctx, &against(&Term::Global(name.clone()), &altered)),
            "{lean}: the constant must be refused at the false comparison by both kernels"
        );
    }
}

// ---- The recognizer, in isolation: Lean's shapes are accelerated; nothing else is. ----------------

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn nat() -> Term {
    g("Nat")
}
fn bool_() -> Term {
    g("Bool")
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn call(f: Term, args: &[Term]) -> Term {
    args.iter()
        .cloned()
        .fold(f, |f, a| Term::App(Box::new(f), Box::new(a)))
}
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn on_nat(d: Term, zero: Term, succ: Term) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(lam("_", nat(), bool_())),
        cases: vec![zero, succ],
    }
}
/// Lean's shape: `| 0, 0 => a | 0, _+1 => b | _+1, 0 => c | n+1, m+1 => step n m`.
fn two_sided(name: &str, a: Term, b: Term, c: Term, step: impl Fn(Term, Term) -> Term) -> Term {
    let rec = format!("{name}.rec");
    let recursive = step(v(&rec), call(v(&rec), &[v("n1"), v("m1")]));
    Term::Fix {
        name: rec.clone(),
        body: Box::new(lam(
            "n",
            nat(),
            lam(
                "m",
                nat(),
                on_nat(
                    v("n"),
                    on_nat(v("m"), a, lam("m1", nat(), b)),
                    lam("n1", nat(), on_nat(v("m"), c, lam("m1", nat(), recursive))),
                ),
            ),
        )),
    }
}
fn plain(name: &str, a: &str, b: &str, c: &str) -> Term {
    two_sided(name, g(a), g(b), g(c), |_, recursive| recursive)
}
fn comparison() -> Term {
    pi("n", nat(), pi("m", nat(), bool_()))
}
fn environment() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx.declare(
        "lean_ble",
        comparison(),
        plain("lean_ble", "true", "true", "false"),
    )
    .unwrap();
    // Lean: `def Nat.blt (n m : Nat) : Bool := Nat.ble (n+1) m`.
    ctx.declare(
        "lean_blt",
        comparison(),
        lam(
            "n",
            nat(),
            lam(
                "m",
                nat(),
                call(g("lean_ble"), &[call(g("Succ"), &[v("n")]), v("m")]),
            ),
        ),
    )
    .unwrap();
    ctx.declare(
        "lean_beq",
        comparison(),
        plain("lean_beq", "true", "false", "false"),
    )
    .unwrap();
    // Lean's `beq` with one equation changed (`n+1, 0 => true`): the program computes `n ≥ m`.
    ctx.declare(
        "impostor_beq",
        comparison(),
        plain("impostor_beq", "true", "false", "true"),
    )
    .unwrap();
    // An assumed comparison: a constant with `ble`'s type and no body.
    ctx.add_declaration("assumed_ble", comparison());
    // Lean's `beq` except that its recursion passes through an axiom.
    ctx.add_declaration("opaque_step", pi("b", bool_(), bool_()));
    ctx.declare(
        "axiom_beq",
        comparison(),
        two_sided(
            "axiom_beq",
            g("true"),
            g("false"),
            g("false"),
            |_, recursive| call(g("opaque_step"), &[recursive]),
        ),
    )
    .unwrap();
    ctx
}
/// `refl : Eq Bool (f a b) answer`, which checks only if `f a b` reduces to `answer`.
fn claim(f: &str, a: u64, b: u64, answer: &str) -> Term {
    let statement = call(
        g("Eq"),
        &[bool_(), call(g(f), &[lit(a), lit(b)]), g(answer)],
    );
    Term::Let {
        name: "claim".into(),
        ty: Box::new(statement),
        value: Box::new(call(g("refl"), &[bool_(), g(answer)])),
        body: Box::new(v("claim")),
    }
}
fn decided(ctx: &Context, f: &str, a: u64, b: u64, answer: &str) {
    let t = claim(f, a, b, answer);
    infer_type(ctx, &t).unwrap_or_else(|e| panic!("primary kernel: {f} {a} {b} = {answer}: {e:?}"));
    recheck(ctx, &t)
        .unwrap_or_else(|e| panic!("independent kernel: {f} {a} {b} = {answer}: {e:?}"));
}
fn refused(ctx: &Context, f: &str, a: u64, b: u64, answer: &str) {
    let t = claim(f, a, b, answer);
    assert!(
        infer_type(ctx, &t).is_err(),
        "the primary kernel must refuse {f} {a} {b} = {answer}"
    );
    assert!(
        recheck(ctx, &t).is_err(),
        "the independent kernel must refuse {f} {a} {b} = {answer}"
    );
}

const TWO_32: u64 = 4_294_967_296;

#[test]
fn leans_four_equation_beq_decides_large_equal_and_unequal_literals_in_both_kernels() {
    let ctx = environment();
    for (a, b, answer) in [
        (TWO_32, TWO_32, "true"),
        (TWO_32, TWO_32 - 1, "false"),
        (TWO_32 - 1, TWO_32, "false"),
        (0, 0, "true"),
        (0, TWO_32, "false"),
        (TWO_32, 0, "false"),
    ] {
        decided(&ctx, "lean_beq", a, b, answer);
    }
}

#[test]
fn the_accelerated_equality_is_never_the_wrong_constructor() {
    let ctx = environment();
    refused(&ctx, "lean_beq", TWO_32, TWO_32, "false");
    refused(&ctx, "lean_beq", TWO_32, TWO_32 - 1, "true");
    refused(&ctx, "lean_beq", TWO_32 - 1, TWO_32, "true");
}

#[test]
fn leans_blt_decides_through_its_ble_unfolding_in_both_kernels() {
    let ctx = environment();
    decided(&ctx, "lean_blt", 1_114_112, TWO_32, "true");
    decided(&ctx, "lean_blt", TWO_32, TWO_32, "false");
    decided(&ctx, "lean_blt", TWO_32 - 1, TWO_32, "true");
    refused(&ctx, "lean_blt", TWO_32, TWO_32, "true");
    refused(&ctx, "lean_blt", 1_114_112, TWO_32, "false");
}

#[test]
fn an_impostor_equality_with_one_different_equation_is_not_accelerated() {
    let ctx = environment();
    // Small inputs still compute by ordinary reduction: the program is well-formed and is `≥`.
    decided(&ctx, "impostor_beq", 3, 3, "true");
    decided(&ctx, "impostor_beq", 5, 3, "true");
    decided(&ctx, "impostor_beq", 3, 5, "false");
    // `2^32 ≥ 2^32 - 1` is true, so `= false` is a false statement — exactly the answer an
    // equality acceleration would give. It must be refused.
    refused(&ctx, "impostor_beq", TWO_32, TWO_32 - 1, "false");
    refused(&ctx, "impostor_beq", TWO_32 - 1, TWO_32, "true");
}

#[test]
fn an_assumed_comparison_is_never_accelerated() {
    let ctx = environment();
    for answer in ["true", "false"] {
        refused(&ctx, "assumed_ble", 0, 0, answer);
        refused(&ctx, "assumed_ble", 1_114_112, TWO_32, answer);
    }
}

#[test]
fn a_comparison_whose_recursion_passes_through_an_axiom_is_not_accelerated() {
    let ctx = environment();
    // Its base cases still compute: the program is well-formed.
    decided(&ctx, "axiom_beq", 0, 0, "true");
    decided(&ctx, "axiom_beq", 0, 5, "false");
    // Past the base cases its value is stuck on the axiom; no equality certificate exists.
    for answer in ["true", "false"] {
        refused(&ctx, "axiom_beq", 1, 1, answer);
        refused(&ctx, "axiom_beq", TWO_32, TWO_32, answer);
    }
}
