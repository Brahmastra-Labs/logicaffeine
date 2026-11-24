//! Rewrite rules (EXODIA Groups 1–3) with KERNEL-CHECKED soundness
//! certificates (D11b): the compiler's e-graph is a sibling of the proof
//! engine, not a stranger.
//!
//! # Soundness discipline
//!
//! Three obligations gate every rule, and all three fail closed:
//!
//! 1. **Type**: a rule fires only when the operand kinds are PROVEN
//!    (literal nodes or Oracle facts). Float operands never rewrite —
//!    `-0.0 + 0.0 == +0.0` already breaks `x + 0 → x` bit-for-bit.
//! 2. **Error/effect preservation**: a rule that DELETES a subterm (stops
//!    it being evaluated) requires [`CompilerEGraph::provably_total`] —
//!    `(a / b) * 0 → 0` would erase the divide-by-zero. Short-circuit
//!    forms (`false ∧ x → false`) are exempt for the RIGHT operand only,
//!    because the runtime never evaluates it either.
//! 3. **Value**: the identity itself carries a certificate checked through
//!    `logicaffeine_kernel`:
//!    - [`Certificate::Ring`] — polynomial identity via the kernel `ring`
//!      tactic. Ring identities with integer coefficients hold in EVERY
//!      commutative ring, in particular ℤ/2⁶⁴ = wrapping `i64`.
//!    - [`Certificate::BoolCases`] — exhaustive Bool case analysis: both
//!      sides are built as CIC terms over Match-defined `and/or/not` and
//!      normalized BY THE KERNEL for every valuation.
//!    - [`Certificate::Bitvector`] — wrapping/shift identities outside the
//!      ring fragment, exhaustively checked on the adversarial i64
//!      boundary grid. The full Z3 bitvector theorem lands with the Forge
//!      SMT layer (M15); this grid is the standing mechanical check.
//!
//!    A `Bitvector` or `LogosProperty` certificate can also carry its
//!    identity as a kernel theorem over [`super::kernel_semantics`], where
//!    the LOGOS operations the rule rewrites are defined in the kernel.
//!    The constant folder evaluates a Rust function, so its check is an
//!    oracle instead, [`fold_agrees_with_kernel`]: the kernel's verdict on
//!    every folded value over a grid of literals.

use logicaffeine_kernel::int_inductive::canonical_int;
use logicaffeine_kernel::{normalize, recheck, ring, Literal as KLiteral, Term};

use super::kernel_semantics;
use super::{CompilerEGraph, CompilerENode, NodeId};
use crate::optimize::ScalarKind;

pub struct Rewrite {
    pub name: &'static str,
    pub apply: fn(&mut CompilerEGraph, NodeId) -> Option<NodeId>,
    pub certificate: Certificate,
}

// =====================================================================
// Certificates
// =====================================================================

/// A tiny polynomial expression language reified into the kernel's
/// `ring` tactic syntax.
#[derive(Debug, Clone)]
pub enum RingExpr {
    Var(u8),
    Const(i64),
    Add(Box<RingExpr>, Box<RingExpr>),
    Sub(Box<RingExpr>, Box<RingExpr>),
    Mul(Box<RingExpr>, Box<RingExpr>),
}

/// A tiny boolean expression language evaluated through kernel
/// normalization (Match-defined connectives over the prelude's Bool).
#[derive(Debug, Clone)]
pub enum BoolExpr {
    Var(u8),
    Const(bool),
    And(Box<BoolExpr>, Box<BoolExpr>),
    Or(Box<BoolExpr>, Box<BoolExpr>),
    Not(Box<BoolExpr>),
}

pub enum Certificate {
    /// `lhs = rhs` as polynomials — kernel `ring` tactic.
    Ring { lhs: RingExpr, rhs: RingExpr },
    /// `lhs = rhs` for all Bool valuations of `vars` variables — each side
    /// normalized by the kernel.
    BoolCases {
        vars: u8,
        lhs: BoolExpr,
        rhs: BoolExpr,
    },
    /// Wrapping-i64 identity checked exhaustively on the boundary grid by
    /// the named check function (upgraded to a Z3 proof in M15).
    Bitvector {
        check: fn() -> Result<(), String>,
        /// The identity as a kernel theorem, once one is proved.
        theorem: Option<KernelTheorem>,
    },
    /// An executable LOGOS property program (D11b — the language is its
    /// own test substrate): the program generates a deterministic corpus,
    /// compares the rule's two sides over it, and Shows its failure
    /// count. Certification = the tree-walker printing `expected`.
    LogosProperty {
        program: &'static str,
        expected: &'static str,
        /// The identity as a kernel theorem, once one is proved.
        theorem: Option<KernelTheorem>,
    },
}

/// Builds a rule's identity as a theorem over the LOGOS operations it rewrites, with the kernel's proof of it.
pub type KernelTheorem = fn() -> Result<KernelCertificate, String>;

// ----- Ring reification ------------------------------------------------

fn s_lit(n: i64) -> Term {
    Term::App(
        Box::new(Term::Global("SLit".to_string())),
        Box::new(Term::Lit(KLiteral::Int(n))),
    )
}

fn s_var(i: i64) -> Term {
    Term::App(
        Box::new(Term::Global("SVar".to_string())),
        Box::new(Term::Lit(KLiteral::Int(i))),
    )
}

fn s_name(name: &str) -> Term {
    Term::App(
        Box::new(Term::Global("SName".to_string())),
        Box::new(Term::Lit(KLiteral::Text(name.to_string()))),
    )
}

fn s_app(f: Term, x: Term) -> Term {
    Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("SApp".to_string())),
            Box::new(f),
        )),
        Box::new(x),
    )
}

fn s_binop(op: &str, a: Term, b: Term) -> Term {
    s_app(s_app(s_name(op), a), b)
}

fn ring_to_syntax(e: &RingExpr) -> Term {
    match e {
        RingExpr::Var(i) => s_var(*i as i64),
        RingExpr::Const(k) => s_lit(*k),
        RingExpr::Add(a, b) => s_binop("add", ring_to_syntax(a), ring_to_syntax(b)),
        RingExpr::Sub(a, b) => s_binop("sub", ring_to_syntax(a), ring_to_syntax(b)),
        RingExpr::Mul(a, b) => s_binop("mul", ring_to_syntax(a), ring_to_syntax(b)),
    }
}

fn check_ring(lhs: &RingExpr, rhs: &RingExpr) -> Result<(), String> {
    let mut vars = logicaffeine_kernel::VarInterner::new();
    let pl = ring::reify(&ring_to_syntax(lhs), &mut vars)
        .map_err(|e| format!("ring reify (lhs): {e:?}"))?;
    let pr = ring::reify(&ring_to_syntax(rhs), &mut vars)
        .map_err(|e| format!("ring reify (rhs): {e:?}"))?;
    if pl.canonical_eq(&pr) {
        Ok(())
    } else {
        Err("polynomials differ".to_string())
    }
}

// ----- Kernel proofs ----------------------------------------------------

/// A rewrite's identity as a theorem with a proof the kernel checks: the identity's variables, each an arbitrary value
/// of its type declared over the standard library with nothing assumed about it, the definitions of the LOGOS
/// operations it mentions, the identity over them, and a term of that type. A checker rebuilds the context from
/// `variables` and `definitions` alone. A definition is declared with its body, so it is never an assumption, and it
/// may only add a name, so a certificate cannot assume anything else.
pub struct KernelCertificate {
    /// Each variable and its type.
    pub variables: Vec<(String, Term)>,
    /// Each LOGOS operation the statement mentions, as `(name, type, body)`, each after the ones it uses: the
    /// definitions of [`kernel_semantics::definitions`], which a checker holds to the runtime.
    pub definitions: Vec<(String, Term, Term)>,
    /// The identity: `Eq T lhs rhs` over the variables, or a `Π` over values and hypotheses ending in one.
    pub statement: Term,
    /// A proof of `statement`.
    pub proof: Term,
}

/// The kernel proof of `rule`'s identity, or why it has none yet.
pub fn kernel_certificate(rule: &Rewrite) -> Result<KernelCertificate, String> {
    match &rule.certificate {
        Certificate::Ring { lhs, rhs } => ring_certificate(lhs, rhs),
        Certificate::BoolCases { vars, lhs, rhs } => bool_certificate(*vars, lhs, rhs),
        Certificate::Bitvector { theorem: Some(theorem), .. }
        | Certificate::LogosProperty { theorem: Some(theorem), .. } => {
            theorem().map_err(|e| format!("{}: {e}", rule.name))
        }
        Certificate::Bitvector { theorem: None, .. } => {
            Err(format!("{}: a check over a grid of i64 values", rule.name))
        }
        Certificate::LogosProperty { theorem: None, .. } => {
            Err(format!("{}: a property program run on the tree-walker", rule.name))
        }
    }
}

/// The standard library with each of `definitions` declared with its body, in order, then each of `variables`
/// declared at its type. A definition the kernel refuses, one whose body is left waiting on a name nothing binds, and a
/// definition or variable whose name is already bound are each an error: a certificate adds names and never rebinds
/// one.
pub fn certificate_context(
    variables: &[(String, Term)],
    definitions: &[(String, Term, Term)],
) -> Result<logicaffeine_kernel::Context, String> {
    let mut ctx = logicaffeine_kernel::prelude::StandardLibrary::context().clone();
    for (name, ty, body) in definitions {
        if ctx.get_global(name).is_some() {
            return Err(format!("`{name}` is already bound, and a certificate's definition may not rebind it"));
        }
        ctx.declare(name, ty.clone(), body.clone()).map_err(|e| format!("the kernel refuses `{name}`: {e}"))?;
    }
    if let Some((name, missing)) =
        ctx.pending_obligations().into_iter().find(|(name, _)| definitions.iter().any(|(d, _, _)| d == name))
    {
        return Err(format!("`{name}` waits on `{missing}`, which nothing binds"));
    }
    for (name, ty) in variables {
        if ctx.get_global(name).is_some() {
            return Err(format!("`{name}` is already bound, and a certificate's variable may not rebind it"));
        }
        ctx.add_declaration(name, ty.clone());
    }
    Ok(ctx)
}

fn kernel_app(f: &str, args: &[Term]) -> Term {
    args.iter().fold(Term::Global(f.to_string()), |acc, a| Term::App(Box::new(acc), Box::new(a.clone())))
}

fn ring_term(e: &RingExpr) -> Term {
    match e {
        RingExpr::Var(i) => Term::Global(format!("v{i}")),
        RingExpr::Const(k) => Term::Lit(KLiteral::Int(*k)),
        RingExpr::Add(a, b) => kernel_app("add", &[ring_term(a), ring_term(b)]),
        RingExpr::Sub(a, b) => kernel_app("sub", &[ring_term(a), ring_term(b)]),
        RingExpr::Mul(a, b) => kernel_app("mul", &[ring_term(a), ring_term(b)]),
    }
}

fn ring_vars(e: &RingExpr, out: &mut std::collections::BTreeSet<u8>) {
    match e {
        RingExpr::Var(i) => {
            out.insert(*i);
        }
        RingExpr::Const(_) => {}
        RingExpr::Add(a, b) | RingExpr::Sub(a, b) | RingExpr::Mul(a, b) => {
            ring_vars(a, out);
            ring_vars(b, out);
        }
    }
}

/// `lhs = rhs` over `Int`, the runtime's exact integers (an `Int` overflow promotes, it never wraps), proved by the
/// proof crate's ring procedure from axiom-free integer laws.
fn ring_certificate(lhs: &RingExpr, rhs: &RingExpr) -> Result<KernelCertificate, String> {
    let mut vars = std::collections::BTreeSet::new();
    ring_vars(lhs, &mut vars);
    ring_vars(rhs, &mut vars);
    let variables: Vec<(String, Term)> =
        vars.into_iter().map(|i| (format!("v{i}"), Term::Global("Int".to_string()))).collect();
    let ctx = certificate_context(&variables, &[])?;
    let (l, r) = (ring_term(lhs), ring_term(rhs));
    let proof = logicaffeine_proof::arith::prove_int_eq(&ctx, &l, &r)
        .ok_or_else(|| format!("no ring proof of {lhs:?} = {rhs:?}"))?;
    Ok(KernelCertificate {
        variables,
        definitions: Vec::new(),
        statement: kernel_app("Eq", &[Term::Global("Int".to_string()), l, r]),
        proof,
    })
}

/// `e` over the variables `b0, b1, …`, with the connectives of [`bool_to_term`].
fn bool_open(e: &BoolExpr) -> Term {
    match e {
        BoolExpr::Var(i) => Term::Var(format!("b{i}")),
        BoolExpr::Const(b) => k_bool(*b),
        BoolExpr::And(a, b) => match_bool(bool_open(a), bool_open(b), k_bool(false)),
        BoolExpr::Or(a, b) => match_bool(bool_open(a), k_bool(true), bool_open(b)),
        BoolExpr::Not(a) => match_bool(bool_open(a), k_bool(false), k_bool(true)),
    }
}

/// `∀ b0 … : Bool, lhs = rhs`, proved by the kernel's tactics: split a variable until each case closes by `rfl`.
fn bool_certificate(vars: u8, lhs: &BoolExpr, rhs: &BoolExpr) -> Result<KernelCertificate, String> {
    use logicaffeine_kernel::tactics::GoalState;
    let boolean = Term::Global("Bool".to_string());
    let identity = kernel_app("Eq", &[boolean.clone(), bool_open(lhs), bool_open(rhs)]);
    let statement = (0..vars).rev().fold(identity, |body, i| Term::Pi {
        param: format!("b{i}"),
        param_type: Box::new(boolean.clone()),
        body_type: Box::new(body),
    });
    let ctx = logicaffeine_kernel::prelude::StandardLibrary::context();
    let mut state = GoalState::new(ctx, &statement).map_err(|e| format!("{e:?}"))?;
    let names: Vec<String> = (0..vars).map(|i| format!("b{i}")).collect();
    for name in &names {
        state.intro(name).map_err(|e| format!("{e:?}"))?;
    }
    // Each split doubles a branch and each branch closes once: never more steps than twice the valuations.
    let mut steps = 2usize << vars;
    while let Some(goal) = state.goals().first().cloned() {
        steps = steps.checked_sub(1).ok_or_else(|| "the case split did not terminate".to_string())?;
        if state.rfl().is_ok() {
            continue;
        }
        let split = names
            .iter()
            .find(|n| goal.hyps.iter().any(|(h, _)| h == *n))
            .ok_or_else(|| format!("a case where the sides differ: {:?}", goal.target))?;
        state.cases(split, &[&[], &[]]).map_err(|e| format!("{e:?}"))?;
    }
    let proof = state.qed().map_err(|e| format!("{e:?}"))?;
    Ok(KernelCertificate { variables: Vec::new(), definitions: Vec::new(), statement, proof })
}

// ----- Bool case analysis through kernel normalization ------------------

fn k_bool(b: bool) -> Term {
    Term::Global(if b { "true" } else { "false" }.to_string())
}

fn match_bool(disc: Term, on_true: Term, on_false: Term) -> Term {
    Term::Match {
        discriminant: Box::new(disc),
        motive: Box::new(Term::Lambda {
            param: "_b".to_string(),
            param_type: Box::new(Term::Global("Bool".to_string())),
            body: Box::new(Term::Global("Bool".to_string())),
        }),
        cases: vec![on_true, on_false],
    }
}

/// Build the kernel term for `e` under a concrete valuation. The
/// connectives are the textbook Match definitions:
/// `and a b = match a with true ⇒ b | false ⇒ false`, etc. — evaluation
/// is entirely the kernel's iota reduction.
fn bool_to_term(e: &BoolExpr, valuation: &[bool]) -> Term {
    match e {
        BoolExpr::Var(i) => k_bool(valuation[*i as usize]),
        BoolExpr::Const(b) => k_bool(*b),
        BoolExpr::And(a, b) => match_bool(
            bool_to_term(a, valuation),
            bool_to_term(b, valuation),
            k_bool(false),
        ),
        BoolExpr::Or(a, b) => match_bool(
            bool_to_term(a, valuation),
            k_bool(true),
            bool_to_term(b, valuation),
        ),
        BoolExpr::Not(a) => match_bool(bool_to_term(a, valuation), k_bool(false), k_bool(true)),
    }
}

fn check_bool_cases(vars: u8, lhs: &BoolExpr, rhs: &BoolExpr) -> Result<(), String> {
    let ctx = logicaffeine_kernel::prelude::StandardLibrary::context();
    for bits in 0..(1u32 << vars) {
        let valuation: Vec<bool> = (0..vars).map(|i| bits & (1 << i) != 0).collect();
        let nl = normalize(ctx, &bool_to_term(lhs, &valuation));
        let nr = normalize(ctx, &bool_to_term(rhs, &valuation));
        if nl != nr {
            return Err(format!("valuation {valuation:?}: {nl:?} ≠ {nr:?}"));
        }
    }
    Ok(())
}

// ----- Bitvector boundary grid ------------------------------------------

/// Adversarial i64 boundary values — overflow rims, sign boundaries,
/// shift-width rims, small primes.
const GRID: &[i64] = &[
    i64::MIN,
    i64::MIN + 1,
    i64::MIN / 2,
    -4_294_967_296,
    -65_537,
    -255,
    -65,
    -64,
    -63,
    -9,
    -8,
    -7,
    -3,
    -2,
    -1,
    0,
    1,
    2,
    3,
    7,
    8,
    9,
    63,
    64,
    65,
    255,
    65_537,
    4_294_967_296,
    i64::MAX / 2,
    i64::MAX - 1,
    i64::MAX,
];

const POWERS: &[u32] = &[0, 1, 2, 3, 5, 16, 31, 32, 62];

/// `mul-pow2-shl` replaces the EXACT product `x * 2^n` by the wrapping `x << n`, so what it rests on is that the two
/// agree wherever the product fits an i64, and that [`mul_pow2_fits_i64`] admits exactly those products: every
/// point interval on the grid for every `n` a rewrite shifts by, and every interval between two grid points, whose
/// admission must cover each grid value inside it.
fn bv_mul_pow2_is_shl() -> Result<(), String> {
    let mut refused = 0usize;
    for n in 0..=62u32 {
        for &x in GRID {
            let exact = i128::from(x) << n;
            let fits = i64::try_from(exact).is_ok();
            if fits && i128::from(x.wrapping_shl(n)) != exact {
                return Err(format!("{x} << {n} is {}, not {x} * 2^{n} = {exact}", x.wrapping_shl(n)));
            }
            if mul_pow2_fits_i64(Some((x, x)), n) != fits {
                return Err(format!(
                    "the gate {} {x} * 2^{n} = {exact}, which {} an i64",
                    if fits { "refuses" } else { "admits" },
                    if fits { "fits" } else { "does not fit" },
                ));
            }
            refused += usize::from(!fits);
        }
        for &lo in GRID {
            for &hi in GRID.iter().filter(|&&hi| lo <= hi) {
                if !mul_pow2_fits_i64(Some((lo, hi)), n) {
                    continue;
                }
                let overflows = |&&x: &&i64| lo <= x && x <= hi && i64::try_from(i128::from(x) << n).is_err();
                if let Some(&x) = GRID.iter().find(overflows) {
                    return Err(format!("the gate admits [{lo}, {hi}] * 2^{n}, and {x} * 2^{n} does not fit an i64"));
                }
            }
        }
    }
    if refused == 0 {
        return Err("no grid product overflows, so the gate's refusal is never exercised".to_string());
    }
    Ok(())
}

fn bv_div_pow2_is_shr_nonneg() -> Result<(), String> {
    for &x in GRID.iter().filter(|&&x| x >= 0) {
        for &n in POWERS {
            let p = 1i64.wrapping_shl(n);
            if p <= 0 {
                continue;
            }
            let div = x.wrapping_div(p);
            let shr = x.wrapping_shr(n);
            if div != shr {
                return Err(format!("{x} / 2^{n}: div {div} ≠ shr {shr}"));
            }
        }
    }
    // The guard is REQUIRED: witness that negatives disagree.
    if (-7i64).wrapping_div(4) == (-7i64).wrapping_shr(2) {
        return Err("guard witness failed: -7/4 should differ from -7>>2".to_string());
    }
    Ok(())
}

fn bv_mod_pow2_is_and_nonneg() -> Result<(), String> {
    for &x in GRID.iter().filter(|&&x| x >= 0) {
        for &n in POWERS {
            let p = 1i64.wrapping_shl(n);
            if p <= 0 {
                continue;
            }
            let md = x.wrapping_rem(p);
            let masked = x & (p - 1);
            if md != masked {
                return Err(format!("{x} % 2^{n}: rem {md} ≠ mask {masked}"));
            }
        }
    }
    if (-7i64).wrapping_rem(4) == (-7i64 & 3) {
        return Err("guard witness failed: -7%4 should differ from -7&3".to_string());
    }
    Ok(())
}

fn bv_div_one_is_identity() -> Result<(), String> {
    for &x in GRID {
        if x.wrapping_div(1) != x {
            return Err(format!("{x} / 1 ≠ {x}"));
        }
    }
    Ok(())
}

/// The literal operands [`fold_agrees_with_kernel`] folds every operation on, both ways round: both rims of `i64`
/// and a neighbour of each, the signs around zero, a zero divisor, and shift amounts that are negative, past 63, and
/// at the rims of the six bits the runtime keeps.
pub const FOLD_GRID: &[i64] = &[i64::MIN, i64::MIN + 1, -65, -8, -7, -1, 0, 1, 2, 7, 64, i64::MAX];

/// `fold` held to the kernel on every pair of [`FOLD_GRID`], for every operation. Where `fold` gives a value, the
/// kernel's definition of the operation computes it: the re-checker accepts `refl v` as a proof of `a op b = v`.
/// Where `fold` gives none, the runtime raises, on a zero divisor, or the kernel's exact value lies outside `i64`,
/// which the re-checker decides the same way. The re-checker computes the standard library's arithmetic natively,
/// `div` and `mod` among it, where the first kernel's conversion unfolds a division one step at a time.
pub fn fold_agrees_with_kernel(fold: impl Fn(FoldOp, i64, i64) -> Option<i64>) -> Result<(), String> {
    let ctx = certificate_context(&[], &kernel_semantics::definitions())?;
    for &a in FOLD_GRID {
        for &b in FOLD_GRID {
            for op in FoldOp::ALL {
                fold_verdict(&ctx, op, a, b, fold(op, a, b)).map_err(|e| format!("{op:?} on ({a}, {b}) {e}"))?;
            }
        }
    }
    Ok(())
}

/// Whether `folded`, a folder's value for `a op b`, is the kernel's.
fn fold_verdict(
    ctx: &logicaffeine_kernel::Context,
    op: FoldOp,
    a: i64,
    b: i64,
    folded: Option<i64>,
) -> Result<(), String> {
    let exact = op.kernel(canonical_int(a), canonical_int(b));
    match folded {
        Some(value) if op.raises_on(b) => Err(format!("folds to {value}, where the runtime raises")),
        Some(value) if computes_to(ctx, Term::Global("Int".to_string()), &exact, canonical_int(value)) => Ok(()),
        Some(value) => Err(format!("folds to {value}, which the kernel does not compute")),
        None if op.raises_on(b) || outside_i64(ctx, &exact) => Ok(()),
        None => Err("does not fold, and the kernel computes an i64".to_string()),
    }
}

/// The kernel computes `exact` to an integer outside `i64`: `exact ≤ i64::MAX` or `i64::MIN ≤ exact` is false.
fn outside_i64(ctx: &logicaffeine_kernel::Context, exact: &Term) -> bool {
    let global = |name: &str| Term::Global(name.to_string());
    let false_of = |bound: Term| computes_to(ctx, global("Bool"), &bound, global("false"));
    false_of(kernel_app("le", &[exact.clone(), canonical_int(i64::MAX)]))
        || false_of(kernel_app("le", &[canonical_int(i64::MIN), exact.clone()]))
}

/// The re-checker accepts `refl value` as a proof of `term = value`: the kernel computes `term` to `value`.
fn computes_to(ctx: &logicaffeine_kernel::Context, ty: Term, term: &Term, value: Term) -> bool {
    let claim = kernel_app("Eq", &[ty.clone(), term.clone(), value.clone()]);
    let cited = Term::Lambda {
        param: "h".to_string(),
        param_type: Box::new(claim),
        body: Box::new(Term::Var("h".to_string())),
    };
    recheck(ctx, &Term::App(Box::new(cited), Box::new(kernel_app("refl", &[ty, value])))).is_ok()
}

/// The constant folder's check: [`fold_agrees_with_kernel`] on [`fold_binop`].
fn fold_is_the_kernels() -> Result<(), String> {
    fold_agrees_with_kernel(fold_binop)
}

// =====================================================================
// The fold evaluator (shared by the const-fold rule and its certificate)
// =====================================================================

/// An integer operation the constant folder evaluates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    Xor,
    And,
    Or,
}

impl FoldOp {
    /// Every operation the constant folder evaluates.
    pub const ALL: [FoldOp; 10] = [
        FoldOp::Add,
        FoldOp::Sub,
        FoldOp::Mul,
        FoldOp::Div,
        FoldOp::Mod,
        FoldOp::Shl,
        FoldOp::Shr,
        FoldOp::Xor,
        FoldOp::And,
        FoldOp::Or,
    ];

    /// `a op b` in the kernel: the standard library's exact `add`, `sub` and `mul`, and the definitions of
    /// [`kernel_semantics`] for the rest, a shift by the low six bits of `b`, as the runtime's shifts take it.
    pub fn kernel(self, a: Term, b: Term) -> Term {
        match self {
            FoldOp::Add => kernel_app("add", &[a, b]),
            FoldOp::Sub => kernel_app("sub", &[a, b]),
            FoldOp::Mul => kernel_app("mul", &[a, b]),
            FoldOp::Div => kernel_semantics::tdiv(a, b),
            FoldOp::Mod => kernel_semantics::tmod(a, b),
            FoldOp::Shl => kernel_semantics::shl(a, kernel_semantics::shift_amount(b)),
            FoldOp::Shr => kernel_semantics::shr(a, kernel_semantics::shift_amount(b)),
            FoldOp::Xor => kernel_semantics::bitxor(a, b),
            FoldOp::And => kernel_semantics::bitand(a, b),
            FoldOp::Or => kernel_semantics::bitor(a, b),
        }
    }

    /// The runtime raises on `a op divisor` for every `a`: `/` and `%` by zero.
    pub fn raises_on(self, divisor: i64) -> bool {
        matches!(self, FoldOp::Div | FoldOp::Mod) && divisor == 0
    }
}

/// Kernel-exact arithmetic for the constant-folder. `None` = refuses to fold —
/// either because the operation has no value (a zero divisor, whose runtime error
/// is the program's meaning) OR because the result OVERFLOWS i64, in which case the
/// exact (promoting) runtime must compute the BigInt rather than have us bake a
/// wrapped constant. `i64::MIN % -1` is 0, which fits. Bitwise/shift ops are total
/// and always fold.
pub fn fold_binop(op: FoldOp, a: i64, b: i64) -> Option<i64> {
    match op {
        FoldOp::Add => a.checked_add(b),
        FoldOp::Sub => a.checked_sub(b),
        FoldOp::Mul => a.checked_mul(b),
        FoldOp::Div => {
            if b == 0 {
                return None;
            }
            a.checked_div(b)
        }
        FoldOp::Mod => {
            if b == 0 {
                return None;
            }
            Some(a.wrapping_rem(b))
        }
        FoldOp::Shl => Some(a.wrapping_shl(b as u32)),
        FoldOp::Shr => Some(a.wrapping_shr(b as u32)),
        FoldOp::Xor => Some(a ^ b),
        FoldOp::And => Some(a & b),
        FoldOp::Or => Some(a | b),
    }
}

// =====================================================================
// Rule guards
// =====================================================================

fn is_int(eg: &mut CompilerEGraph, id: NodeId) -> bool {
    eg.scalar_of(id) == Some(ScalarKind::Int)
}

fn is_bool(eg: &mut CompilerEGraph, id: NodeId) -> bool {
    eg.scalar_of(id) == Some(ScalarKind::Bool)
}

/// May this operand be DELETED from the residual?
fn removable(eg: &mut CompilerEGraph, id: NodeId) -> bool {
    eg.provably_total(id)
}

/// The class is a proven zero / one (point interval — literals seed these,
/// and the Oracle can prove them for variables too).
fn proven(eg: &mut CompilerEGraph, id: NodeId, k: i64) -> bool {
    eg.int_value(id) == Some(k)
}

// =====================================================================
// Group 1 — algebraic identities (int-guarded)
// =====================================================================

fn r_add_zero(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Add(l, r) = eg.canonical_node(id) {
        if proven(eg, r, 0) && is_int(eg, l) && removable(eg, r) {
            return Some(l);
        }
        if proven(eg, l, 0) && is_int(eg, r) && removable(eg, l) {
            return Some(r);
        }
    }
    None
}

fn r_mul_one(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mul(l, r) = eg.canonical_node(id) {
        if proven(eg, r, 1) && is_int(eg, l) && removable(eg, r) {
            return Some(l);
        }
        if proven(eg, l, 1) && is_int(eg, r) && removable(eg, l) {
            return Some(r);
        }
    }
    None
}

fn r_mul_zero(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mul(l, r) = eg.canonical_node(id) {
        let zero_side = if proven(eg, r, 0) {
            Some((r, l))
        } else if proven(eg, l, 0) {
            Some((l, r))
        } else {
            None
        };
        if let Some((zero, other)) = zero_side {
            // BOTH operands stop being evaluated.
            if is_int(eg, other) && removable(eg, other) && removable(eg, zero) {
                let z = eg.add(CompilerENode::Int(0));
                return Some(z);
            }
        }
    }
    None
}

fn r_sub_zero(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Sub(l, r) = eg.canonical_node(id) {
        if proven(eg, r, 0) && is_int(eg, l) && removable(eg, r) {
            return Some(l);
        }
    }
    None
}

fn r_sub_self(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Sub(l, r) = eg.canonical_node(id) {
        if eg.find(l) == eg.find(r) && is_int(eg, l) && removable(eg, l) {
            let z = eg.add(CompilerENode::Int(0));
            return Some(z);
        }
    }
    None
}

fn r_div_one(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Div(l, r) = eg.canonical_node(id) {
        if proven(eg, r, 1) && is_int(eg, l) && removable(eg, r) {
            return Some(l);
        }
    }
    None
}

fn r_not_not_bool(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Not(inner) = eg.canonical_node(id) {
        for m in eg.class_members(inner) {
            if let CompilerENode::Not(x) = eg.canonical_node(m) {
                if is_bool(eg, x) {
                    return Some(x);
                }
            }
        }
    }
    None
}

// (`not` is logical over truthiness: `Not(Not(x))` on an Int is `x != 0` as a
// Bool, NOT `x` — so the only sound double-negation elimination is the
// Bool-guarded rule above.)

// =====================================================================
// Group 2 — boolean simplification (bool-guarded; `And`/`Or` are
// short-circuit in the language, so left-constant forms delete only the
// never-evaluated RIGHT operand)
// =====================================================================

fn r_true_and(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::And(l, r) = eg.canonical_node(id) {
        if eg.class_has_bool(l, true) && is_bool(eg, r) && removable(eg, l) {
            return Some(r);
        }
    }
    None
}

fn r_false_and(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::And(l, r) = eg.canonical_node(id) {
        // Short-circuit: the runtime never evaluates r either, so only
        // the deleted LEFT operand needs the totality proof.
        if eg.class_has_bool(l, false) && is_bool(eg, r) && removable(eg, l) {
            let f = eg.add(CompilerENode::Bool(false));
            return Some(f);
        }
    }
    None
}

fn r_true_or(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Or(l, r) = eg.canonical_node(id) {
        if eg.class_has_bool(l, true) && is_bool(eg, r) && removable(eg, l) {
            let t = eg.add(CompilerENode::Bool(true));
            return Some(t);
        }
    }
    None
}

fn r_false_or(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Or(l, r) = eg.canonical_node(id) {
        if eg.class_has_bool(l, false) && is_bool(eg, r) && removable(eg, l) {
            return Some(r);
        }
    }
    None
}

fn r_and_self(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::And(l, r) = eg.canonical_node(id) {
        // The kept copy evaluates identically (same class, same first
        // error) — no totality requirement.
        if eg.find(l) == eg.find(r) && is_bool(eg, l) {
            return Some(l);
        }
    }
    None
}

fn r_or_self(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Or(l, r) = eg.canonical_node(id) {
        if eg.find(l) == eg.find(r) && is_bool(eg, l) {
            return Some(l);
        }
    }
    None
}

/// Is some member of `maybe_not`'s class `Not(x)` with x ≡ `base`?
fn class_negates(eg: &mut CompilerEGraph, maybe_not: NodeId, base: NodeId) -> bool {
    let broot = eg.find(base);
    for m in eg.class_members(maybe_not) {
        if let CompilerENode::Not(x) = eg.canonical_node(m) {
            if eg.find(x) == broot {
                return true;
            }
        }
    }
    false
}

fn r_and_not_self(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::And(l, r) = eg.canonical_node(id) {
        if is_bool(eg, l)
            && (class_negates(eg, r, l) || class_negates(eg, l, r))
            && removable(eg, l)
            && removable(eg, r)
        {
            let f = eg.add(CompilerENode::Bool(false));
            return Some(f);
        }
    }
    None
}

fn r_or_not_self(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Or(l, r) = eg.canonical_node(id) {
        if is_bool(eg, l)
            && (class_negates(eg, r, l) || class_negates(eg, l, r))
            && removable(eg, l)
            && removable(eg, r)
        {
            let t = eg.add(CompilerENode::Bool(true));
            return Some(t);
        }
    }
    None
}

// =====================================================================
// Group 3 — strength reduction (Oracle-conditional)
// =====================================================================

fn pow2_log(k: i64) -> Option<u32> {
    if k > 0 && (k as u64).is_power_of_two() {
        Some(k.trailing_zeros())
    } else {
        None
    }
}

fn r_mul_two_add(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mul(l, r) = eg.canonical_node(id) {
        let x = if proven(eg, r, 2) && removable(eg, r) {
            Some(l)
        } else if proven(eg, l, 2) && removable(eg, l) {
            Some(r)
        } else {
            None
        };
        if let Some(x) = x {
            if is_int(eg, x) {
                let sum = eg.add(CompilerENode::Add(x, x));
                return Some(sum);
            }
        }
    }
    None
}

/// `x * 2^n` provably stays within i64 for every `x` in its proven interval —
/// the soundness precondition for replacing the (EXACT, BigInt-promoting)
/// multiply with a (wrapping) left shift. `2^n` fits a positive i64 for every
/// `n` `pow2_log` returns (`n ≤ 62`); the product's extremes sit at the
/// interval ends, so checking both is sufficient.
fn mul_pow2_fits_i64(range: Option<(i64, i64)>, n: u32) -> bool {
    let Some((lo, hi)) = range else { return false };
    if n >= 63 {
        return false;
    }
    let m = 1i64 << n;
    lo.checked_mul(m).is_some() && hi.checked_mul(m).is_some()
}

fn r_mul_pow2_shl(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mul(l, r) = eg.canonical_node(id) {
        let hit = if let Some(n) = eg.int_value(r).and_then(pow2_log) {
            if removable(eg, r) {
                Some((l, n))
            } else {
                None
            }
        } else if let Some(n) = eg.int_value(l).and_then(pow2_log) {
            if removable(eg, l) {
                Some((r, n))
            } else {
                None
            }
        } else {
            None
        };
        if let Some((x, n)) = hit {
            // ORACLE GATE: integer `*` is EXACT (promotes to BigInt on overflow)
            // but `<<` WRAPS, so the rewrite is valid only where the Oracle's
            // interval for `x` proves `x * 2^n` cannot escape i64. Mirrors the
            // gate on `r_div_pow2_shr`; unproven cases keep the exact multiply
            // (the backend still lowers it to a checked native `imul`).
            if is_int(eg, x) && mul_pow2_fits_i64(eg.int_range(x), n) {
                let shift = eg.add(CompilerENode::Int(n as i64));
                let shl = eg.add(CompilerENode::Shl(x, shift));
                return Some(shl);
            }
        }
    }
    None
}

fn r_div_pow2_shr(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Div(l, r) = eg.canonical_node(id) {
        if let Some(n) = eg.int_value(r).and_then(pow2_log) {
            // ORACLE GATE: truncating ÷ and arithmetic shift agree only
            // for proven non-negative dividends.
            if is_int(eg, l) && eg.proven_nonneg(l) && removable(eg, r) {
                let shift = eg.add(CompilerENode::Int(n as i64));
                let shr = eg.add(CompilerENode::Shr(l, shift));
                return Some(shr);
            }
        }
    }
    None
}

fn r_mod_pow2_and(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mod(l, r) = eg.canonical_node(id) {
        if let Some(k) = eg.int_value(r) {
            if pow2_log(k).is_some() && is_int(eg, l) && eg.proven_nonneg(l) && removable(eg, r) {
                let mask = eg.add(CompilerENode::Int(k - 1));
                let masked = eg.add(CompilerENode::BitAnd(l, mask));
                return Some(masked);
            }
        }
    }
    None
}

// =====================================================================
// Group 5: deforestation / fusion — the Len/Slice/Copy algebra
// (Wadler 1988 at expression grain: intermediate collections that exist
// only to be measured or read once never materialize)
// =====================================================================

/// A member of `class` matching `pred`, canonicalized.
fn member_matching(
    eg: &mut CompilerEGraph,
    class: NodeId,
    pred: fn(&CompilerENode) -> bool,
) -> Option<CompilerENode> {
    let members = eg.class_members(class);
    members.into_iter().map(|m| eg.canonical_node(m)).find(pred)
}

/// `len(copy(xs))` → `len(xs)`: over a PROVEN collection the copy cannot
/// raise and preserves length — the O(n) materialization disappears.
fn r_len_copy(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Len(c) = eg.canonical_node(id) {
        if let Some(CompilerENode::Copy(inner)) =
            member_matching(eg, c, |n| matches!(n, CompilerENode::Copy(_)))
        {
            if eg.proven_collection(inner) {
                return Some(eg.add(CompilerENode::Len(inner)));
            }
        }
    }
    None
}

/// `index(copy(xs), i)` → `index(xs, i)`: same contents, same bounds,
/// same error — one read through a fresh copy is unobservable.
fn r_index_copy(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Index(c, i) = eg.canonical_node(id) {
        if let Some(CompilerENode::Copy(inner)) =
            member_matching(eg, c, |n| matches!(n, CompilerENode::Copy(_)))
        {
            if eg.proven_collection(inner) {
                return Some(eg.add(CompilerENode::Index(inner, i)));
            }
        }
    }
    None
}

/// `copy(copy(x))` ≡ `copy(x)` extensionally: both are fresh values with
/// identical contents, and an erroring operand raises the SAME first
/// error on both sides — unconditional.
fn r_copy_copy(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Copy(c) = eg.canonical_node(id) {
        if member_matching(eg, c, |n| matches!(n, CompilerENode::Copy(_))).is_some() {
            return Some(eg.find(c));
        }
    }
    None
}

/// `slice(xs, 1, len(xs))` ≡ `copy(xs)` for a PROVEN LIST: the runtime's
/// slice is every item from offset `start - 1` up to `end` when `end` is at
/// most the length, and EMPTY otherwise, so items 1 through the length are
/// the whole list, and both sides are fresh. Gated on listness because
/// slicing raises on anything but a list while copy accepts any collection.
fn r_slice_full_copy(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Slice(xs, a, b) = eg.canonical_node(id) {
        if eg.int_value(a) == Some(1) && eg.proven_list(xs) {
            let len_match = member_matching(eg, b, |n| matches!(n, CompilerENode::Len(_)));
            if let Some(CompilerENode::Len(c2)) = len_match {
                if eg.find(c2) == eg.find(xs) {
                    return Some(eg.add(CompilerENode::Copy(xs)));
                }
            }
        }
    }
    None
}

/// `len(slice(xs, a, b))` → `b − a + 1` under PROOFS: constant bounds
/// with 1 ≤ a, a ≤ b + 1 and b ≤ len(xs) guaranteed — the slice stays inside
/// the list, where the runtime keeps items a through b (none when a = b + 1)
/// instead of answering EMPTY, so the length is exact and the whole slice
/// (xs included) disappears. Deleting xs requires its tree total.
fn r_len_slice_bounds(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Len(s) = eg.canonical_node(id) {
        let slice = member_matching(eg, s, |n| matches!(n, CompilerENode::Slice(..)));
        if let Some(CompilerENode::Slice(xs, a, b)) = slice {
            let (Some(av), Some(bv)) = (eg.int_value(a), eg.int_value(b)) else {
                return None;
            };
            if av < 1 || av > bv.saturating_add(1) || !eg.proven_list(xs) {
                return None;
            }
            let len_class = eg.add(CompilerENode::Len(xs));
            let Some((len_lo, _)) = eg.int_range(len_class) else {
                return None;
            };
            if bv <= len_lo && removable(eg, xs) {
                return Some(eg.add(CompilerENode::Int(bv - av + 1)));
            }
        }
    }
    None
}

// =====================================================================
// Commutativity / associativity (int-only — float reassociation is
// unsound, and `And`/`Or` short-circuit so they are NOT commutative
// with respect to errors)
// =====================================================================

fn r_add_comm(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Add(l, r) = eg.canonical_node(id) {
        // Swapping evaluation order is observable through error precedence
        // unless both operands are total.
        if is_int(eg, l) && is_int(eg, r) && removable(eg, l) && removable(eg, r) {
            let flipped = eg.add(CompilerENode::Add(r, l));
            return Some(flipped);
        }
    }
    None
}

fn r_mul_comm(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mul(l, r) = eg.canonical_node(id) {
        if is_int(eg, l) && is_int(eg, r) && removable(eg, l) && removable(eg, r) {
            let flipped = eg.add(CompilerENode::Mul(r, l));
            return Some(flipped);
        }
    }
    None
}

fn r_add_assoc(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Add(l, c) = eg.canonical_node(id) {
        if !is_int(eg, c) {
            return None;
        }
        for m in eg.class_members(l) {
            if let CompilerENode::Add(a, b) = eg.canonical_node(m) {
                if is_int(eg, a) && is_int(eg, b) {
                    let bc = eg.add(CompilerENode::Add(b, c));
                    let abc = eg.add(CompilerENode::Add(a, bc));
                    return Some(abc);
                }
            }
        }
    }
    None
}

fn r_mul_assoc(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Mul(l, c) = eg.canonical_node(id) {
        if !is_int(eg, c) {
            return None;
        }
        for m in eg.class_members(l) {
            if let CompilerENode::Mul(a, b) = eg.canonical_node(m) {
                if is_int(eg, a) && is_int(eg, b) {
                    let bc = eg.add(CompilerENode::Mul(b, c));
                    let abc = eg.add(CompilerENode::Mul(a, bc));
                    return Some(abc);
                }
            }
        }
    }
    None
}

/// Gauss / Karatsuba butterfly: `la·lb + ra·rb → (la+ra)(lb+rb) − la·rb − ra·lb`. The naive
/// sum is two multiplies; the rewritten form spends one NEW multiply `(la+ra)(lb+rb)` plus the
/// cross products `la·rb`, `ra·lb`. It is a *conditional* speedup — it pays only when those
/// cross products are SHARED with a sibling (the real part `la·rb − ra·lb` of a complex / NTT
/// butterfly), where the pair then costs 3 multiplies instead of 4. Adding the alternative
/// here lets the cost extractor pick it exactly when the sharing makes it cheaper; the value is
/// unchanged either way, because the rule's `Certificate::Ring` is kernel-proven (the NTT can
/// never be miscompiled by it).
fn r_gauss_butterfly(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    if let CompilerENode::Add(l, r) = eg.canonical_node(id) {
        for lm in eg.class_members(l) {
            if let CompilerENode::Mul(la, lb) = eg.canonical_node(lm) {
                for rm in eg.class_members(r) {
                    if let CompilerENode::Mul(ra, rb) = eg.canonical_node(rm) {
                        if [la, lb, ra, rb]
                            .iter()
                            .all(|&x| is_int(eg, x) && removable(eg, x))
                        {
                            let sum_a = eg.add(CompilerENode::Add(la, ra));
                            let sum_b = eg.add(CompilerENode::Add(lb, rb));
                            let prod = eg.add(CompilerENode::Mul(sum_a, sum_b));
                            let cross1 = eg.add(CompilerENode::Mul(la, rb));
                            let cross2 = eg.add(CompilerENode::Mul(ra, lb));
                            let s1 = eg.add(CompilerENode::Sub(prod, cross1));
                            let res = eg.add(CompilerENode::Sub(s1, cross2));
                            return Some(res);
                        }
                    }
                }
            }
        }
    }
    None
}

// =====================================================================
// Constant folding (point intervals — subsumes literal folding and
// extends to Oracle-proven single values)
// =====================================================================

fn r_const_fold(eg: &mut CompilerEGraph, id: NodeId) -> Option<NodeId> {
    let node = eg.canonical_node(id);
    let op = match node {
        CompilerENode::Add(..) => FoldOp::Add,
        CompilerENode::Sub(..) => FoldOp::Sub,
        CompilerENode::Mul(..) => FoldOp::Mul,
        CompilerENode::Div(..) => FoldOp::Div,
        CompilerENode::Mod(..) => FoldOp::Mod,
        CompilerENode::Shl(..) => FoldOp::Shl,
        CompilerENode::Shr(..) => FoldOp::Shr,
        CompilerENode::BitXor(..) => FoldOp::Xor,
        CompilerENode::BitAnd(..) => FoldOp::And,
        CompilerENode::BitOr(..) => FoldOp::Or,
        _ => return None,
    };
    let (l, r) = match node {
        CompilerENode::Add(l, r)
        | CompilerENode::Sub(l, r)
        | CompilerENode::Mul(l, r)
        | CompilerENode::Div(l, r)
        | CompilerENode::Mod(l, r)
        | CompilerENode::Shl(l, r)
        | CompilerENode::Shr(l, r)
        | CompilerENode::BitXor(l, r)
        | CompilerENode::BitAnd(l, r)
        | CompilerENode::BitOr(l, r) => (l, r),
        _ => unreachable!(),
    };
    if eg.find(id) == eg.find(l) || eg.find(id) == eg.find(r) {
        // Cyclic class (id ≡ one of its own children) — folding would
        // self-justify; leave it to extraction.
        return None;
    }
    let a = eg.int_value(l)?;
    let b = eg.int_value(r)?;
    if !(removable(eg, l) && removable(eg, r)) {
        return None;
    }
    let v = fold_binop(op, a, b)?;
    let lit = eg.add(CompilerENode::Int(v));
    Some(lit)
}

// =====================================================================
// The registry
// =====================================================================

fn rx(e: RingExpr) -> Box<RingExpr> {
    Box::new(e)
}

fn bx(e: BoolExpr) -> Box<BoolExpr> {
    Box::new(e)
}

pub fn all() -> Vec<Rewrite> {
    use BoolExpr as B;
    use RingExpr as R;
    vec![
        Rewrite {
            name: "add-zero",
            apply: r_add_zero,
            certificate: Certificate::Ring {
                lhs: R::Add(rx(R::Var(0)), rx(R::Const(0))),
                rhs: R::Var(0),
            },
        },
        Rewrite {
            name: "mul-one",
            apply: r_mul_one,
            certificate: Certificate::Ring {
                lhs: R::Mul(rx(R::Var(0)), rx(R::Const(1))),
                rhs: R::Var(0),
            },
        },
        Rewrite {
            name: "mul-zero",
            apply: r_mul_zero,
            certificate: Certificate::Ring {
                lhs: R::Mul(rx(R::Var(0)), rx(R::Const(0))),
                rhs: R::Const(0),
            },
        },
        Rewrite {
            name: "sub-zero",
            apply: r_sub_zero,
            certificate: Certificate::Ring {
                lhs: R::Sub(rx(R::Var(0)), rx(R::Const(0))),
                rhs: R::Var(0),
            },
        },
        Rewrite {
            name: "sub-self",
            apply: r_sub_self,
            certificate: Certificate::Ring {
                lhs: R::Sub(rx(R::Var(0)), rx(R::Var(0))),
                rhs: R::Const(0),
            },
        },
        Rewrite {
            name: "gauss-butterfly",
            apply: r_gauss_butterfly,
            certificate: Certificate::Ring {
                // la·lb + ra·rb  =  (la+ra)(lb+rb) − la·rb − ra·lb   (la=V0, lb=V1, ra=V2, rb=V3)
                lhs: R::Add(
                    rx(R::Mul(rx(R::Var(0)), rx(R::Var(1)))),
                    rx(R::Mul(rx(R::Var(2)), rx(R::Var(3)))),
                ),
                rhs: R::Sub(
                    rx(R::Sub(
                        rx(R::Mul(
                            rx(R::Add(rx(R::Var(0)), rx(R::Var(2)))),
                            rx(R::Add(rx(R::Var(1)), rx(R::Var(3)))),
                        )),
                        rx(R::Mul(rx(R::Var(0)), rx(R::Var(3)))),
                    )),
                    rx(R::Mul(rx(R::Var(2)), rx(R::Var(1)))),
                ),
            },
        },
        Rewrite {
            name: "div-one",
            apply: r_div_one,
            certificate: Certificate::Bitvector {
                check: bv_div_one_is_identity,
                theorem: Some(kernel_semantics::div_one),
            },
        },
        Rewrite {
            name: "not-not-bool",
            apply: r_not_not_bool,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::Not(bx(B::Not(bx(B::Var(0))))),
                rhs: B::Var(0),
            },
        },
        Rewrite {
            name: "true-and",
            apply: r_true_and,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::And(bx(B::Const(true)), bx(B::Var(0))),
                rhs: B::Var(0),
            },
        },
        Rewrite {
            name: "false-and",
            apply: r_false_and,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::And(bx(B::Const(false)), bx(B::Var(0))),
                rhs: B::Const(false),
            },
        },
        Rewrite {
            name: "true-or",
            apply: r_true_or,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::Or(bx(B::Const(true)), bx(B::Var(0))),
                rhs: B::Const(true),
            },
        },
        Rewrite {
            name: "false-or",
            apply: r_false_or,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::Or(bx(B::Const(false)), bx(B::Var(0))),
                rhs: B::Var(0),
            },
        },
        Rewrite {
            name: "and-self",
            apply: r_and_self,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::And(bx(B::Var(0)), bx(B::Var(0))),
                rhs: B::Var(0),
            },
        },
        Rewrite {
            name: "or-self",
            apply: r_or_self,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::Or(bx(B::Var(0)), bx(B::Var(0))),
                rhs: B::Var(0),
            },
        },
        Rewrite {
            name: "and-not-self",
            apply: r_and_not_self,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::And(bx(B::Var(0)), bx(B::Not(bx(B::Var(0))))),
                rhs: B::Const(false),
            },
        },
        Rewrite {
            name: "or-not-self",
            apply: r_or_not_self,
            certificate: Certificate::BoolCases {
                vars: 1,
                lhs: B::Or(bx(B::Var(0)), bx(B::Not(bx(B::Var(0))))),
                rhs: B::Const(true),
            },
        },
        Rewrite {
            name: "mul-two-add",
            apply: r_mul_two_add,
            certificate: Certificate::Ring {
                lhs: R::Mul(rx(R::Var(0)), rx(R::Const(2))),
                rhs: R::Add(rx(R::Var(0)), rx(R::Var(0))),
            },
        },
        Rewrite {
            name: "mul-pow2-shl",
            apply: r_mul_pow2_shl,
            certificate: Certificate::Bitvector {
                check: bv_mul_pow2_is_shl,
                theorem: Some(kernel_semantics::mul_pow2_shl),
            },
        },
        Rewrite {
            name: "div-pow2-shr",
            apply: r_div_pow2_shr,
            certificate: Certificate::Bitvector {
                check: bv_div_pow2_is_shr_nonneg,
                theorem: Some(kernel_semantics::div_pow2_shr),
            },
        },
        Rewrite {
            name: "mod-pow2-and",
            apply: r_mod_pow2_and,
            certificate: Certificate::Bitvector {
                check: bv_mod_pow2_is_and_nonneg,
                theorem: Some(kernel_semantics::mod_pow2_and),
            },
        },
        Rewrite {
            name: "add-comm",
            apply: r_add_comm,
            certificate: Certificate::Ring {
                lhs: R::Add(rx(R::Var(0)), rx(R::Var(1))),
                rhs: R::Add(rx(R::Var(1)), rx(R::Var(0))),
            },
        },
        Rewrite {
            name: "add-assoc",
            apply: r_add_assoc,
            certificate: Certificate::Ring {
                lhs: R::Add(rx(R::Add(rx(R::Var(0)), rx(R::Var(1)))), rx(R::Var(2))),
                rhs: R::Add(rx(R::Var(0)), rx(R::Add(rx(R::Var(1)), rx(R::Var(2))))),
            },
        },
        Rewrite {
            name: "mul-comm",
            apply: r_mul_comm,
            certificate: Certificate::Ring {
                lhs: R::Mul(rx(R::Var(0)), rx(R::Var(1))),
                rhs: R::Mul(rx(R::Var(1)), rx(R::Var(0))),
            },
        },
        Rewrite {
            name: "mul-assoc",
            apply: r_mul_assoc,
            certificate: Certificate::Ring {
                lhs: R::Mul(rx(R::Mul(rx(R::Var(0)), rx(R::Var(1)))), rx(R::Var(2))),
                rhs: R::Mul(rx(R::Var(0)), rx(R::Mul(rx(R::Var(1)), rx(R::Var(2))))),
            },
        },
        Rewrite {
            name: "const-fold",
            apply: r_const_fold,
            certificate: Certificate::Bitvector {
                check: fold_is_the_kernels,
                theorem: None,
            },
        },
        Rewrite {
            name: "len-copy",
            apply: r_len_copy,
            certificate: Certificate::LogosProperty {
                program: PROP_LEN_COPY,
                expected: "0",
                theorem: Some(kernel_semantics::len_copy),
            },
        },
        Rewrite {
            name: "index-copy",
            apply: r_index_copy,
            certificate: Certificate::LogosProperty {
                program: PROP_INDEX_COPY,
                expected: "0",
                theorem: Some(kernel_semantics::index_copy),
            },
        },
        Rewrite {
            name: "copy-copy",
            apply: r_copy_copy,
            certificate: Certificate::LogosProperty {
                program: PROP_COPY_COPY,
                expected: "0",
                theorem: Some(kernel_semantics::copy_copy),
            },
        },
        Rewrite {
            name: "slice-full-copy",
            apply: r_slice_full_copy,
            certificate: Certificate::LogosProperty {
                program: PROP_SLICE_FULL_COPY,
                expected: "0",
                theorem: Some(kernel_semantics::slice_full_copy),
            },
        },
        Rewrite {
            name: "len-slice-bounds",
            apply: r_len_slice_bounds,
            certificate: Certificate::LogosProperty {
                program: PROP_LEN_SLICE_BOUNDS,
                expected: "0",
                theorem: Some(kernel_semantics::len_slice_bounds),
            },
        },
    ]
}

// =====================================================================
// Group 5 property certificates: deterministic LCG corpora (lists of
// length 0..12 — the empty list and the a = b + 1 empty slice are in
// range by construction), each Showing its failure count.
// =====================================================================

const PROP_LEN_COPY: &str = "## Main\n\
Let mutable seed be 42.\n\
Let mutable failures be 0.\n\
Let mutable t be 0.\n\
While t is less than 40:\n\
\x20   Let mutable xs be a new Seq of Int.\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Let n be seed % 13.\n\
\x20   Let mutable i be 0.\n\
\x20   While i is less than n:\n\
\x20       Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20       Push seed % 100 to xs.\n\
\x20       Set i to i + 1.\n\
\x20   If length of (copy of xs) is not length of xs:\n\
\x20       Set failures to failures + 1.\n\
\x20   Set t to t + 1.\n\
Show failures.\n";

const PROP_INDEX_COPY: &str = "## Main\n\
Let mutable seed be 99.\n\
Let mutable failures be 0.\n\
Let mutable t be 0.\n\
While t is less than 40:\n\
\x20   Let mutable xs be a new Seq of Int.\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Let n be seed % 13.\n\
\x20   Let mutable i be 0.\n\
\x20   While i is less than n:\n\
\x20       Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20       Push seed % 100 to xs.\n\
\x20       Set i to i + 1.\n\
\x20   If n is at least 1:\n\
\x20       Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20       Let k be seed % n + 1.\n\
\x20       If item k of (copy of xs) is not item k of xs:\n\
\x20           Set failures to failures + 1.\n\
\x20   Set t to t + 1.\n\
Show failures.\n";

const PROP_COPY_COPY: &str = "## Main\n\
Let mutable seed be 7.\n\
Let mutable failures be 0.\n\
Let mutable t be 0.\n\
While t is less than 40:\n\
\x20   Let mutable xs be a new Seq of Int.\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Let n be seed % 13.\n\
\x20   Let mutable i be 0.\n\
\x20   While i is less than n:\n\
\x20       Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20       Push seed % 100 to xs.\n\
\x20       Set i to i + 1.\n\
\x20   Let a be copy of (copy of xs).\n\
\x20   Let b be copy of xs.\n\
\x20   If length of a is not length of b:\n\
\x20       Set failures to failures + 1.\n\
\x20   Let mutable j be 1.\n\
\x20   While j is at most length of a:\n\
\x20       If item j of a is not item j of b:\n\
\x20           Set failures to failures + 1.\n\
\x20       Set j to j + 1.\n\
\x20   Set t to t + 1.\n\
Show failures.\n";

const PROP_SLICE_FULL_COPY: &str = "## Main\n\
Let mutable seed be 1234.\n\
Let mutable failures be 0.\n\
Let mutable t be 0.\n\
While t is less than 40:\n\
\x20   Let mutable xs be a new Seq of Int.\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Let n be seed % 13.\n\
\x20   Let mutable i be 0.\n\
\x20   While i is less than n:\n\
\x20       Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20       Push seed % 100 to xs.\n\
\x20       Set i to i + 1.\n\
\x20   Let s be items 1 through length of xs of xs.\n\
\x20   Let c be copy of xs.\n\
\x20   If length of s is not length of c:\n\
\x20       Set failures to failures + 1.\n\
\x20   Let mutable j be 1.\n\
\x20   While j is at most length of s:\n\
\x20       If item j of s is not item j of c:\n\
\x20           Set failures to failures + 1.\n\
\x20       Set j to j + 1.\n\
\x20   Set t to t + 1.\n\
Show failures.\n";

const PROP_LEN_SLICE_BOUNDS: &str = "## Main\n\
Let mutable seed be 5150.\n\
Let mutable failures be 0.\n\
Let mutable t be 0.\n\
While t is less than 60:\n\
\x20   Let mutable xs be a new Seq of Int.\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Let n be seed % 12 + 1.\n\
\x20   Let mutable i be 0.\n\
\x20   While i is less than n:\n\
\x20       Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20       Push seed % 100 to xs.\n\
\x20       Set i to i + 1.\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Let lo be seed % n + 1.\n\
\x20   Set seed to (seed * 1103515245 + 12345) % 2147483648.\n\
\x20   Let hi be lo - 1 + (seed % (n - lo + 2)).\n\
\x20   Let s be items (lo) through (hi) of xs.\n\
\x20   If length of s is not hi - lo + 1:\n\
\x20       Set failures to failures + 1.\n\
\x20   Set t to t + 1.\n\
Show failures.\n";

/// Check every rule's certificate through the kernel. Returns the number
/// of verified rules; the first failure aborts with the rule's name.
pub fn verify_all_with_kernel() -> Result<usize, String> {
    let rules = all();
    for rule in &rules {
        let outcome = match &rule.certificate {
            Certificate::Ring { lhs, rhs } => check_ring(lhs, rhs),
            Certificate::BoolCases { vars, lhs, rhs } => check_bool_cases(*vars, lhs, rhs),
            Certificate::Bitvector { check, .. } => check(),
            Certificate::LogosProperty { program, expected, .. } => {
                match crate::compile::interpret_program(program) {
                    Ok(out) if out.trim() == *expected => Ok(()),
                    Ok(out) => Err(format!(
                        "property program printed {:?}, expected {:?}",
                        out.trim(),
                        expected
                    )),
                    Err(e) => Err(format!(
                        "property program failed: {}",
                        crate::compile::describe_parse_error(&e)
                    )),
                }
            }
        };
        outcome.map_err(|e| format!("rule '{}': {e}", rule.name))?;
    }
    Ok(rules.len())
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/optimize/egraph/rules/tests.rs"]
mod tests;
