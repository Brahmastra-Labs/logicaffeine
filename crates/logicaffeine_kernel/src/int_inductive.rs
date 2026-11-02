//! `Int` as a REAL INDUCTIVE — Lean's `ofNat | negSucc`, with its arithmetic DEFINED.
//!
//! The standard prelude represents integers with checked constructors and definitions.
//! Ring and order facts still need separate proof terms; installing a carrier does not
//! turn an assumed law into a theorem.
//! It is a block Lean does not pay: `Init/Data/Int/Basic.lean:40`–`45` declares
//!
//! ```text
//! inductive Int : Type where
//!   | ofNat   : Nat → Int
//!   | negSucc : Nat → Int
//! ```
//!
//! and then DEFINES `negOfNat`, `neg`, `subNatNat`, `add`, `mul` and `sub` by matching on those
//! two constructors (`Init/Data/Int/Basic.lean:78`–`166`). Every arithmetic identity Lean states
//! about `Int` is a theorem over that definition.
//!
//! # How Lean keeps it fast anyway
//!
//! The obvious objection to constructors is cost: `ofNat n` looks like a unary Peano spine, so
//! `2 + 3` looks linear and `2 * 3` quadratic. Lean's answer — read off its own source rather
//! than guessed — is that `Int`'s arithmetic BOTTOMS OUT IN `Nat`'s. Look at the `ofNat/ofNat`
//! arm of `Int.add`: it is `ofNat (m + n)`, ONE `Nat` addition, and `Nat` literals are
//! GMP-accelerated in Lean's kernel. The `@[extern "lean_int_add"]` attribute on the same
//! definition (`Init/Data/Int/Basic.lean:126`) is the COMPILER's story, not the kernel's:
//! `Int.ofNat` itself carries `@[extern "lean_nat_to_int"]` (line 47) so a runtime `Int` is
//! never a spine either.
//!
//! LOGOS keeps compact arbitrary-precision literals and canonical constructor bridges.
//! Public addition and multiplication have checked definition bodies. Their literal
//! accelerations attest the complete program dependency chain and preserve each kernel's
//! independent arithmetic implementation. Rebinding any dependency disables the shortcut.
//!
//! # What this module is for
//!
//! The standard prelude calls this module to install checked constructors and arithmetic.
//! Constructors pass positivity/universe checks; arithmetic remains ordinary definitions
//! visible to both kernels. The literal bridge below activates only for the canonical
//! declaration signatures. The standard prelude also installs the public add/mul aliases.
//! Other primitive operations and the ring/order axioms remain separately auditable until
//! their derived replacements are installed.

use crate::context::Context;
use crate::error::{KernelError, KernelResult};
use crate::term::Term;

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
}
fn lam(p: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn pi(p: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn fix(name: &str, body: Term) -> Term {
    Term::Fix {
        name: name.to_string(),
        body: Box::new(body),
    }
}
fn mat(d: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(motive),
        cases,
    }
}
fn nat() -> Term {
    g("Nat")
}
fn int() -> Term {
    g("Int")
}
fn succ(t: Term) -> Term {
    app(g("Succ"), t)
}
/// `λ_:Nat. Nat` — the motive every `Nat` recursion below returns at.
fn nat_motive() -> Term {
    lam("_", nat(), nat())
}
/// `λ_:Int. Int`.
fn int_motive() -> Term {
    lam("_", int(), int())
}
fn nat_binop() -> Term {
    arrow(nat(), arrow(nat(), nat()))
}
fn int_binop() -> Term {
    arrow(int(), arrow(int(), int()))
}
fn of_nat(n: Term) -> Term {
    app(g("Int_ofNat"), n)
}
fn neg_succ(n: Term) -> Term {
    app(g("Int_negSucc"), n)
}

/// Register `name` unless something is already bound to it — a caller that has its own `Nat_add`
/// keeps it, and this module never silently swaps one program for another.
fn define_if_absent(ctx: &mut Context, name: &str, ty: Term, body: Term) {
    if ctx.get_global(name).is_none() {
        ctx.add_definition(name.to_string(), ty, body);
    }
}

/// The `Nat` operations `Int`'s arithmetic bottoms out in, recursing on the SECOND argument
/// exactly as Lean's `Nat.add`/`Nat.sub`/`Nat.mul` do (`Init/Prelude.lean`'s `Nat.add` is
/// `| n, .succ m => .succ (Nat.add n m)`).
///
/// Recursing on the second argument is not a detail: it is what makes `n + 0 ≡ n` and
/// `n * 0 ≡ 0` hold by ι for an OPEN `n`, which is what every ring-law proof over `Int` starts
/// from.
type ArithmeticDefinition = (&'static str, Term, Term);

fn nat_arithmetic_definitions() -> &'static [ArithmeticDefinition] {
    static DEFINITIONS: std::sync::OnceLock<Vec<ArithmeticDefinition>> = std::sync::OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        let mut definitions = Vec::new();

        // Nat_add m 0 = m ; Nat_add m (k+1) = Succ (Nat_add m k)
        definitions.push((
            "Nat_add",
            nat_binop(),
            fix(
                "f✧",
                lam(
                    "m",
                    nat(),
                    lam(
                        "n",
                        nat(),
                        mat(
                            v("n"),
                            nat_motive(),
                            vec![
                                v("m"),
                                lam("k", nat(), succ(apps(v("f✧"), &[v("m"), v("k")]))),
                            ],
                        ),
                    ),
                ),
            ),
        ));
        // Nat_pred 0 = 0 ; Nat_pred (k+1) = k
        definitions.push((
            "Nat_pred",
            arrow(nat(), nat()),
            lam(
                "x",
                nat(),
                mat(
                    v("x"),
                    nat_motive(),
                    vec![g("Zero"), lam("p", nat(), v("p"))],
                ),
            ),
        ));
        // Nat_sub m 0 = m ; Nat_sub m (k+1) = Nat_pred (Nat_sub m k) — TRUNCATED, as Lean's is.
        definitions.push((
            "Nat_sub",
            nat_binop(),
            fix(
                "f✧",
                lam(
                    "m",
                    nat(),
                    lam(
                        "n",
                        nat(),
                        mat(
                            v("n"),
                            nat_motive(),
                            vec![
                                v("m"),
                                lam(
                                    "k",
                                    nat(),
                                    app(g("Nat_pred"), apps(v("f✧"), &[v("m"), v("k")])),
                                ),
                            ],
                        ),
                    ),
                ),
            ),
        ));
        // Nat_mul m 0 = 0 ; Nat_mul m (k+1) = Nat_add (Nat_mul m k) m
        definitions.push((
            "Nat_mul",
            nat_binop(),
            fix(
                "f✧",
                lam(
                    "m",
                    nat(),
                    lam(
                        "n",
                        nat(),
                        mat(
                            v("n"),
                            nat_motive(),
                            vec![
                                g("Zero"),
                                lam(
                                    "k",
                                    nat(),
                                    apps(g("Nat_add"), &[apps(v("f✧"), &[v("m"), v("k")]), v("m")]),
                                ),
                            ],
                        ),
                    ),
                ),
            ),
        ));

        definitions
    })
}

pub fn register_nat_arithmetic(ctx: &mut Context) {
    for (name, ty, body) in nat_arithmetic_definitions() {
        define_if_absent(ctx, name, ty.clone(), body.clone());
    }
}

/// `Int`'s two constructors, through the CHECKED door.
///
/// `ofNat : Nat → Int` and `negSucc : Nat → Int`, in Lean's declaration order
/// (`Init/Data/Int/Basic.lean:42`, `:45`) — order is the whole content of a declaration here,
/// because `Term::Match` is positional.
pub fn register_int_constructors(ctx: &mut Context) -> KernelResult<()> {
    if !ctx.is_inductive("Int") {
        return Err(KernelError::CertificationError(
            "`Int` must already be a registered inductive — this adds its constructors, it does \
             not declare the carrier"
                .to_string(),
        ));
    }
    if !ctx.get_constructors("Int").is_empty() {
        return Ok(());
    }
    ctx.add_constructor_checked("Int_ofNat", "Int", arrow(nat(), int()))?;
    ctx.add_constructor_checked("Int_negSucc", "Int", arrow(nat(), int()))?;
    Ok(())
}

/// `Int` negation, addition, subtraction and multiplication as DEFINITIONS, transcribed from
/// `Init/Data/Int/Basic.lean:78`–`166` arm for arm.
///
/// ```text
/// negOfNat 0        = ofNat 0          negOfNat (succ m) = negSucc m
/// neg (ofNat n)     = negOfNat n       neg (negSucc n)   = ofNat (succ n)
/// subNatNat m n     = match n - m with | 0 => ofNat (m - n) | succ k => negSucc k
/// add  (ofNat m)   (ofNat n)   = ofNat (m + n)
/// add  (ofNat m)   (negSucc n) = subNatNat m (succ n)
/// add  (negSucc m) (ofNat n)   = subNatNat n (succ m)
/// add  (negSucc m) (negSucc n) = negSucc (succ (m + n))
/// mul  (ofNat m)   (ofNat n)   = ofNat (m * n)
/// mul  (ofNat m)   (negSucc n) = negOfNat (m * succ n)
/// mul  (negSucc m) (ofNat n)   = negOfNat (succ m * n)
/// mul  (negSucc m) (negSucc n) = ofNat (succ m * succ n)
/// sub  m n                     = m + (- n)
/// ```
///
/// `negOfNat` is the ONE place `-0 = 0` is decided, which is why it is a named definition here
/// and not inlined into `neg` and `mul`: an off-by-one there is invisible on positive operands.
fn int_arithmetic_definitions() -> &'static [ArithmeticDefinition] {
    static DEFINITIONS: std::sync::OnceLock<Vec<ArithmeticDefinition>> = std::sync::OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        let mut definitions = Vec::new();
        let nadd = |a: Term, b: Term| apps(g("Nat_add"), &[a, b]);
        let nmul = |a: Term, b: Term| apps(g("Nat_mul"), &[a, b]);

        // Int_negOfNat : Nat → Int
        definitions.push((
            "Int_negOfNat",
            arrow(nat(), int()),
            lam(
                "n",
                nat(),
                mat(
                    v("n"),
                    lam("_", nat(), int()),
                    vec![of_nat(g("Zero")), lam("k", nat(), neg_succ(v("k")))],
                ),
            ),
        ));
        // Int_subNatNat : Nat → Nat → Int
        definitions.push((
            "Int_subNatNat",
            arrow(nat(), arrow(nat(), int())),
            lam(
                "m",
                nat(),
                lam(
                    "n",
                    nat(),
                    mat(
                        apps(g("Nat_sub"), &[v("n"), v("m")]),
                        lam("_", nat(), int()),
                        vec![
                            of_nat(apps(g("Nat_sub"), &[v("m"), v("n")])),
                            lam("k", nat(), neg_succ(v("k"))),
                        ],
                    ),
                ),
            ),
        ));
        // Int_neg : Int → Int
        definitions.push((
            "Int_neg",
            arrow(int(), int()),
            lam(
                "a",
                int(),
                mat(
                    v("a"),
                    int_motive(),
                    vec![
                        lam("m", nat(), app(g("Int_negOfNat"), v("m"))),
                        lam("m", nat(), of_nat(succ(v("m")))),
                    ],
                ),
            ),
        ));
        // Int_add : Int → Int → Int
        definitions.push((
            "Int_add",
            int_binop(),
            lam(
                "a",
                int(),
                lam(
                    "b",
                    int(),
                    mat(
                        v("a"),
                        int_motive(),
                        vec![
                            lam(
                                "m",
                                nat(),
                                mat(
                                    v("b"),
                                    int_motive(),
                                    vec![
                                        lam("n", nat(), of_nat(nadd(v("m"), v("n")))),
                                        lam(
                                            "n",
                                            nat(),
                                            apps(g("Int_subNatNat"), &[v("m"), succ(v("n"))]),
                                        ),
                                    ],
                                ),
                            ),
                            lam(
                                "m",
                                nat(),
                                mat(
                                    v("b"),
                                    int_motive(),
                                    vec![
                                        lam(
                                            "n",
                                            nat(),
                                            apps(g("Int_subNatNat"), &[v("n"), succ(v("m"))]),
                                        ),
                                        lam("n", nat(), neg_succ(succ(nadd(v("m"), v("n"))))),
                                    ],
                                ),
                            ),
                        ],
                    ),
                ),
            ),
        ));
        // Int_sub a b := Int_add a (Int_neg b). Public sub_elim additionally
        // rewrites Int_neg b to multiplication by -1 using a checked constructor proof.
        definitions.push((
            "Int_sub",
            int_binop(),
            lam(
                "a",
                int(),
                lam(
                    "b",
                    int(),
                    apps(g("Int_add"), &[v("a"), app(g("Int_neg"), v("b"))]),
                ),
            ),
        ));
        // Int_mul : Int → Int → Int
        definitions.push((
            "Int_mul",
            int_binop(),
            lam(
                "a",
                int(),
                lam(
                    "b",
                    int(),
                    mat(
                        v("a"),
                        int_motive(),
                        vec![
                            lam(
                                "m",
                                nat(),
                                mat(
                                    v("b"),
                                    int_motive(),
                                    vec![
                                        lam("n", nat(), of_nat(nmul(v("m"), v("n")))),
                                        lam(
                                            "n",
                                            nat(),
                                            app(g("Int_negOfNat"), nmul(v("m"), succ(v("n")))),
                                        ),
                                    ],
                                ),
                            ),
                            lam(
                                "m",
                                nat(),
                                mat(
                                    v("b"),
                                    int_motive(),
                                    vec![
                                        lam(
                                            "n",
                                            nat(),
                                            app(g("Int_negOfNat"), nmul(succ(v("m")), v("n"))),
                                        ),
                                        lam("n", nat(), of_nat(nmul(succ(v("m")), succ(v("n"))))),
                                    ],
                                ),
                            ),
                        ],
                    ),
                ),
            ),
        ));

        definitions
    })
}

pub fn register_int_arithmetic(ctx: &mut Context) -> KernelResult<()> {
    if ctx.get_constructors("Int").len() != 2 {
        return Err(KernelError::CertificationError(
            "`Int`'s arithmetic is defined BY CASES on `ofNat`/`negSucc` — register the \
             constructors first"
                .to_string(),
        ));
    }
    register_nat_arithmetic(ctx);
    for (name, ty, body) in int_arithmetic_definitions() {
        define_if_absent(ctx, name, ty.clone(), body.clone());
    }
    Ok(())
}

/// The whole adoption, as one call: `Int` gains Lean's two constructors and Lean's arithmetic.
///
/// `Nat` must already be registered (`Int_ofNat` stores a `Nat`); `StandardLibrary::register`
/// calls this once `Nat` and its arithmetic exist. It defines `Int_add`/`Int_sub`/`Int_mul`/
/// `Int_le` and leaves the public `add`/`sub`/`mul` names alone — binding those to these
/// definitions is `register_public_arithmetic_aliases`, a separate step with its own check.
///
/// ```
/// use logicaffeine_kernel::int_inductive::canonical_int;
/// use logicaffeine_kernel::prelude::StandardLibrary;
/// use logicaffeine_kernel::{normalize, Term};
///
/// let ctx = StandardLibrary::context();
/// let constructors: Vec<&str> = ctx.get_constructors("Int").into_iter().map(|(name, _)| name).collect();
/// assert_eq!(constructors, ["Int_ofNat", "Int_negSucc"]);
///
/// let app = |f: Term, x: Term| Term::App(Box::new(f), Box::new(x));
/// let sum = app(app(Term::Global("Int_add".into()), canonical_int(2)), canonical_int(-5));
/// assert_eq!(normalize(ctx, &sum), canonical_int(-3));
/// ```
pub fn register_inductive_int(ctx: &mut Context) -> KernelResult<()> {
    register_int_constructors(ctx)?;
    register_int_arithmetic(ctx)
}

/// The canonical `Int` term for a machine integer, in Lean's representation: `ofNat n` for
/// `n ≥ 0`, `negSucc (-n-1)` below. The `Nat` argument is a `Literal::Nat`, not a `Succ` spine,
/// so a large integer costs one node — which is the whole point of the `Nat` literal bridge.
pub fn canonical_int(n: i64) -> Term {
    canonical_bigint(&crate::BigInt::from_i64(n))
}

/// The literal bridge is valid only for the canonical monomorphic Nat/Int declarations.
/// A same-spelled user definition or a replaced constructor must never inherit this rule.
pub(crate) fn canonical_environment(ctx: &Context) -> bool {
    ctx.canonical_numerals().int
}

/// [`canonical_environment`] for `Nat` alone: the rules that read a `Nat` literal as `Zero` or
/// `Succ` of the literal below it.
pub(crate) fn canonical_nat_environment(ctx: &Context) -> bool {
    ctx.canonical_numerals().nat
}

/// Whether a `Nat` literal of value `n` reads as a constructor: `0` (and a malformed negative) as
/// `Zero` while `Zero : Nat` is `Nat`'s constructor, `n + 1` as `Succ n` while `Succ : Nat → Nat`
/// is too. A reading names its constructors, and holds exactly while they are `Nat`'s.
pub(crate) fn reads_nat_literal(ctx: &Context, n: &crate::BigInt) -> bool {
    let numerals = ctx.canonical_numerals();
    if *n <= crate::BigInt::from_i64(0) {
        numerals.zero
    } else {
        numerals.succ
    }
}

/// Decides [`crate::context::CanonicalNumerals`] from the registries
/// ([`Context::canonical_numerals`] keeps the answer for their generation).
pub(crate) fn canonical_numerals_of(ctx: &Context) -> crate::context::CanonicalNumerals {
    let nat = canonical_family_environment(ctx, "Nat", ["Zero", "Succ"]);
    let zero = nat || nat_constructor_environment(ctx, "Zero", &|ty| *ty == g("Nat"));
    let unary = |ty: &Term| {
        matches!(ty, Term::Pi { param_type, body_type, .. }
            if **param_type == g("Nat") && **body_type == g("Nat"))
    };
    crate::context::CanonicalNumerals {
        zero,
        succ: nat || (zero && nat_constructor_environment(ctx, "Succ", &unary)),
        nat,
        int: nat && canonical_family_environment(ctx, "Int", ["Int_ofNat", "Int_negSucc"]),
    }
}

/// `Nat` a monomorphic family in `Type 0` with a constructor `name` whose type `fits`, whatever
/// the others are: what reading a literal as `name` needs.
fn nat_constructor_environment(ctx: &Context, name: &str, fits: &dyn Fn(&Term) -> bool) -> bool {
    use crate::Universe;
    ctx.is_inductive("Nat")
        && ctx.get_global("Nat") == Some(&Term::Sort(Universe::Type(0)))
        && ctx.inductive_num_params("Nat") == 0
        && ctx.poly_declaration("Nat").is_none()
        && ctx.get_definition_body("Nat").is_none()
        && ctx.constructor_inductive(name) == Some("Nat")
        && ctx.get_global(name).is_some_and(fits)
        && ctx.poly_declaration(name).is_none()
        && ctx.get_definition_body(name).is_none()
}

fn canonical_family_environment(ctx: &Context, family: &str, names: [&str; 2]) -> bool {
    use crate::{Term, Universe};
    if !ctx.is_inductive(family)
        || ctx.get_global(family) != Some(&Term::Sort(Universe::Type(0)))
        || ctx.inductive_num_params(family) != 0
        || ctx.poly_declaration(family).is_some()
        || ctx.get_definition_body(family).is_some()
    {
        return false;
    }
    let constructors = ctx.get_constructors(family);
    if constructors.len() != 2 {
        return false;
    }
    for ((actual, ty), expected) in constructors.iter().zip(names) {
        if *actual != expected
            || ctx.constructor_inductive(expected) != Some(family)
            || ctx.get_global(expected) != Some(*ty)
            || ctx.poly_declaration(expected).is_some()
            || ctx.get_definition_body(expected).is_some()
        {
            return false;
        }
        let right_type = if expected == "Zero" {
            **ty == g("Nat")
        } else {
            matches!(ty, Term::Pi { param_type, body_type, .. }
                if **param_type == g("Nat") && **body_type == g(family))
        };
        if !right_type {
            return false;
        }
    }
    true
}

/// One constructor layer, preserving arbitrary precision and Int.negSucc's offset.
pub(crate) fn literal_constructor(ctx: &Context, t: &Term) -> Option<Term> {
    let Term::Lit(literal) = t else { return None };
    let n = crate::term::lit_bigint(literal)?;
    if !canonical_environment(ctx) {
        return None;
    }
    Some(canonical_bigint(&n))
}

fn canonical_bigint(n: &crate::BigInt) -> Term {
    use crate::{BigInt, Literal};
    if n.is_negative() {
        neg_succ(Term::Lit(Literal::Nat(n.abs().sub(&BigInt::from_i64(1)))))
    } else {
        of_nat(Term::Lit(Literal::Nat(n.clone())))
    }
}

/// Read a canonical constructor value for the existing primitive arithmetic evaluator.
/// The field is inspected iteratively: a compact literal and a finite Succ spine agree.
pub(crate) fn constructor_literal(ctx: &Context, t: &Term) -> Option<crate::Literal> {
    let Term::App(head, field) = t else {
        return None;
    };
    let negative = match head.as_ref() {
        Term::Global(n) if n == "Int_ofNat" => false,
        Term::Global(n) if n == "Int_negSucc" => true,
        _ => return None,
    };
    if !canonical_environment(ctx) {
        return None;
    }
    let mut field = field.as_ref();
    let mut count = crate::BigInt::from_i64(0);
    let one = crate::BigInt::from_i64(1);
    loop {
        match field {
            Term::Lit(crate::Literal::Nat(n)) if !n.is_negative() => {
                count = count.add(n);
                break;
            }
            Term::Global(n) if n == "Zero" => break,
            Term::App(f, a) if matches!(f.as_ref(), Term::Global(n) if n == "Succ") => {
                count = count.add(&one);
                field = a;
            }
            _ => return None,
        }
    }
    if negative {
        count = count.add(&one).negated();
    }
    Some(crate::term::int_lit(count))
}

/// Checked Boolean order on the existing signed constructors. Natural order
/// remains the prelude's canonical natLe; this module does not rebuild it.
fn integer_order_definition() -> &'static ArithmeticDefinition {
    static DEFINITION: std::sync::OnceLock<ArithmeticDefinition> = std::sync::OnceLock::new();
    DEFINITION.get_or_init(|| {
        let bool_motive = || lam("_", int(), g("Bool"));
        let body = lam(
            "a",
            int(),
            lam(
                "b",
                int(),
                mat(
                    v("a"),
                    bool_motive(),
                    vec![
                        lam(
                            "m",
                            nat(),
                            mat(
                                v("b"),
                                bool_motive(),
                                vec![
                                    lam("n", nat(), apps(g("natLe"), &[v("m"), v("n")])),
                                    lam("n", nat(), g("false")),
                                ],
                            ),
                        ),
                        lam(
                            "m",
                            nat(),
                            mat(
                                v("b"),
                                bool_motive(),
                                vec![
                                    lam("n", nat(), g("true")),
                                    lam("n", nat(), apps(g("natLe"), &[v("n"), v("m")])),
                                ],
                            ),
                        ),
                    ],
                ),
            ),
        );
        ("Int_le", arrow(int(), arrow(int(), g("Bool"))), body)
    })
}

/// Install order only after the standard prelude has supplied Bool and natLe.
/// Standalone structural integer arithmetic remains independent of those types.
pub(crate) fn register_public_order_alias(ctx: &mut Context) -> KernelResult<()> {
    let (name, ty, body) = integer_order_definition();
    ctx.declare_checked_auxiliary(name, ty.clone(), body.clone())?;
    let public_type = ctx.get_global("le").cloned().unwrap_or_else(|| ty.clone());
    ctx.declare_checked_auxiliary("le", public_type, g(name))?;
    Ok(())
}

// The public header is exactly Int -> Int -> Int. Parameter spelling has
// no semantic role because neither domain nor result contains a bound variable.
// Inspect only this shape: no unfolding, conversion, or inferred equivalence.
fn has_integer_binop_signature(ty: &Term) -> bool {
    has_integer_binary_result(ty, "Int")
}

fn has_integer_binary_result(ty: &Term, result_name: &str) -> bool {
    let Term::Pi {
        param_type: first,
        body_type: rest,
        ..
    } = ty
    else {
        return false;
    };
    let Term::Pi {
        param_type: second,
        body_type: result,
        ..
    } = rest.as_ref()
    else {
        return false;
    };
    [first.as_ref(), second.as_ref()]
        .into_iter()
        .all(|part| matches!(part, Term::Global(name) if name == "Int"))
        && matches!(result.as_ref(), Term::Global(name) if name == result_name)
}

fn integer_division_definitions() -> &'static [ArithmeticDefinition] {
    static DEFINITIONS: std::sync::OnceLock<Vec<ArithmeticDefinition>> = std::sync::OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        let pair_ty = || apps(g("Prod"), &[int(), int()]);
        let pair = |q: Term, r: Term| apps(g("Prod_mk"), &[int(), int(), q, r]);
        let natural_pair = || apps(g("Prod"), &[nat(), nat()]);
        let nonzero = |negative_divisor: bool| {
            let positive = mat(
                apps(g("Nat_divMod"), &[v("m"), succ(v("k"))]),
                lam("_", natural_pair(), pair_ty()),
                vec![lam(
                    "q",
                    nat(),
                    lam(
                        "r",
                        nat(),
                        pair(
                            if negative_divisor {
                                app(g("Int_negOfNat"), v("q"))
                            } else {
                                of_nat(v("q"))
                            },
                            of_nat(v("r")),
                        ),
                    ),
                )],
            );
            let negative = mat(
                apps(g("Nat_divMod"), &[succ(v("m")), succ(v("k"))]),
                lam("_", natural_pair(), pair_ty()),
                vec![lam(
                    "q",
                    nat(),
                    lam(
                        "r",
                        nat(),
                        mat(
                            v("r"),
                            lam("_", nat(), pair_ty()),
                            vec![
                                pair(
                                    if negative_divisor {
                                        of_nat(v("q"))
                                    } else {
                                        app(g("Int_negOfNat"), v("q"))
                                    },
                                    of_nat(g("Zero")),
                                ),
                                lam(
                                    "s",
                                    nat(),
                                    pair(
                                        if negative_divisor {
                                            of_nat(succ(v("q")))
                                        } else {
                                            neg_succ(v("q"))
                                        },
                                        of_nat(apps(g("Nat_sub"), &[succ(v("k")), succ(v("s"))])),
                                    ),
                                ),
                            ],
                        ),
                    ),
                )],
            );
            mat(
                v("a"),
                lam("_", int(), pair_ty()),
                vec![lam("m", nat(), positive), lam("m", nat(), negative)],
            )
        };
        let body = lam(
            "a",
            int(),
            lam(
                "b",
                int(),
                mat(
                    v("b"),
                    lam("_", int(), pair_ty()),
                    vec![
                        lam(
                            "n",
                            nat(),
                            mat(
                                v("n"),
                                lam("_", nat(), pair_ty()),
                                vec![
                                    pair(of_nat(g("Zero")), v("a")),
                                    lam("k", nat(), nonzero(false)),
                                ],
                            ),
                        ),
                        lam("k", nat(), nonzero(true)),
                    ],
                ),
            ),
        );
        let mut definitions = vec![("Int_divMod", arrow(int(), arrow(int(), pair_ty())), body)];
        for (name, projection) in [("Int_div", "Prod_fst"), ("Int_mod", "Prod_snd")] {
            definitions.push((
                name,
                int_binop(),
                lam(
                    "a",
                    int(),
                    lam(
                        "b",
                        int(),
                        apps(
                            g(projection),
                            &[int(), int(), apps(g("Int_divMod"), &[v("a"), v("b")])],
                        ),
                    ),
                ),
            ));
        }
        definitions
    })
}

pub(crate) fn register_public_division_aliases(ctx: &mut Context) -> KernelResult<()> {
    for (name, ty, body) in integer_division_definitions() {
        ctx.declare_checked_auxiliary(name, ty.clone(), body.clone())?;
    }
    for (name, target) in [("div", "Int_div"), ("mod", "Int_mod")] {
        let ty = ctx.get_global(name).cloned().unwrap_or_else(int_binop);
        ctx.declare_checked_auxiliary(name, ty, g(target))?;
    }
    Ok(())
}

/// The generated product projections and positional Bool inventory are part of
/// division's meaning, so compare with the same builders used by the prelude.
fn canonical_division_products(ctx: &Context) -> bool {
    let reference = crate::prelude::StandardLibrary::division_carrier_reference();
    for family in ["Bool", "Prod"] {
        if !ctx.is_inductive(family)
            || ctx.get_global(family) != reference.get_global(family)
            || ctx.inductive_num_params(family) != reference.inductive_num_params(family)
            || ctx.poly_declaration(family).is_some()
            || ctx.get_definition_body(family).is_some()
        {
            return false;
        }
        let actual = ctx.get_constructors(family);
        let expected = reference.get_constructors(family);
        if actual != expected {
            return false;
        }
        for (name, ty) in expected {
            if ctx.constructor_inductive(name) != Some(family)
                || ctx.get_global(name) != Some(ty)
                || ctx.poly_declaration(name).is_some()
                || ctx.get_definition_body(name).is_some()
            {
                return false;
            }
        }
    }
    ["Prod_fst", "Prod_snd"].into_iter().all(|name| {
        ctx.poly_declaration(name).is_none()
            && !ctx.is_constructor(name)
            && !ctx.is_inductive(name)
            && ctx.get_global(name) == reference.get_global(name)
            && ctx.get_definition_type(name) == reference.get_definition_type(name)
            && ctx.get_definition_body(name) == reference.get_definition_body(name)
    })
}

/// Definitions whose delta steps a closed public arithmetic acceleration skips.
pub(crate) fn integer_alias_dependencies(operation: &str) -> Option<&'static [&'static str]> {
    match operation {
        "add" => Some(&[
            "add",
            "Int_add",
            "Int_subNatNat",
            "Nat_sub",
            "Nat_pred",
            "Nat_add",
        ]),
        "mul" => Some(&["mul", "Int_mul", "Int_negOfNat", "Nat_mul", "Nat_add"]),
        "sub" => Some(&[
            "sub",
            "Int_sub",
            "Int_neg",
            "Int_negOfNat",
            "Int_add",
            "Int_subNatNat",
            "Nat_sub",
            "Nat_pred",
            "Nat_add",
        ]),
        "le" => Some(&["le", "Int_le", "natLe"]),
        "div" => Some(&[
            "div",
            "Int_div",
            "Int_divMod",
            "Prod_fst",
            "Prod_snd",
            "Nat_divMod",
            "Nat_divModSucc",
            "Nat_sub",
            "Nat_pred",
            "Int_negOfNat",
            "natLt",
            "natLe",
        ]),
        "mod" => Some(&[
            "mod",
            "Int_mod",
            "Int_divMod",
            "Prod_fst",
            "Prod_snd",
            "Nat_divMod",
            "Nat_divModSucc",
            "Nat_sub",
            "Nat_pred",
            "Int_negOfNat",
            "natLt",
            "natLe",
        ]),
        _ => None,
    }
}

/// A definition may use the existing exact arithmetic implementation only when
/// its complete program and canonical carriers match the checked constructors.
/// Rebinding any dependency changes the registry generation and invalidates the
/// context-owned attestation. Rewrite and transparency policy are not cached.
pub(crate) fn accelerates_integer_alias(ctx: &Context, operation: &str) -> bool {
    if !ctx.simp_rules().is_empty() {
        return false;
    }
    let Some(dependencies) = integer_alias_dependencies(operation) else {
        return false;
    };
    ctx.cached_integer_arithmetic(operation, || {
        if !canonical_environment(ctx)
            || !canonical_nat_environment(ctx)
            || ctx.inductive_num_params("Nat") != 0
            || ctx.inductive_num_params("Int") != 0
        {
            return false;
        }
        for name in ["Nat", "Zero", "Succ", "Int", "Int_ofNat", "Int_negSucc"] {
            if ctx.poly_declaration(name).is_some() {
                return false;
            }
        }
        let target = match operation {
            "add" => "Int_add",
            "mul" => "Int_mul",
            "sub" => "Int_sub",
            "le" => "Int_le",
            "div" => "Int_div",
            "mod" => "Int_mod",
            _ => return false,
        };
        let has_signature = |ty: &Term| {
            if operation == "le" {
                has_integer_binary_result(ty, "Bool")
            } else {
                has_integer_binop_signature(ty)
            }
        };
        if ctx.poly_declaration(operation).is_some()
            || ctx.is_constructor(operation)
            || ctx.is_inductive(operation)
            || !ctx.get_global(operation).is_some_and(has_signature)
            || ctx.get_definition_body(operation) != Some(&g(target))
            || !ctx
                .get_definition_type(operation)
                .is_some_and(has_signature)
        {
            return false;
        }
        if operation == "le" {
            let (_, ty, body) = integer_order_definition();
            return ctx.poly_declaration("Int_le").is_none()
                && !ctx.is_constructor("Int_le")
                && !ctx.is_inductive("Int_le")
                && ctx.get_global("Int_le") == Some(ty)
                && ctx.get_definition_type("Int_le") == Some(ty)
                && ctx.get_definition_body("Int_le") == Some(body)
                && ctx.poly_declaration("natLe").is_none()
                && !ctx.is_constructor("natLe")
                && !ctx.is_inductive("natLe")
                && ctx.get_global("natLe") == ctx.get_definition_type("natLe")
                && crate::reduction::accelerates_nat_order(ctx, "natLe");
        }
        let division = matches!(operation, "div" | "mod");
        if division && !canonical_division_products(ctx) {
            return false;
        }
        dependencies[1..].iter().all(|name| {
            if division && matches!(*name, "Prod_fst" | "Prod_snd") {
                return true; // compared against their canonical generated bodies above
            }
            if division && matches!(*name, "natLt" | "natLe") {
                return ctx.poly_declaration(name).is_none()
                    && !ctx.is_constructor(name)
                    && !ctx.is_inductive(name)
                    && ctx.get_global(name) == ctx.get_definition_type(name)
                    && crate::reduction::accelerates_nat_order(ctx, name);
            }
            let Some((_, ty, body)) = nat_arithmetic_definitions()
                .iter()
                .chain(int_arithmetic_definitions())
                .chain(integer_division_definitions())
                .chain(crate::prelude::natural_arithmetic::division_definitions())
                .find(|(candidate, _, _)| candidate == name)
            else {
                return false;
            };
            ctx.poly_declaration(name).is_none()
                && !ctx.is_constructor(name)
                && !ctx.is_inductive(name)
                && ctx.get_global(name) == Some(ty)
                && ctx.get_definition_type(name) == Some(ty)
                && ctx.get_definition_body(name) == Some(body)
        })
    })
}

/// Whether the public add/mul/sub vocabulary has the standard integer meaning.
///
/// Polynomial proof producers may preserve these operation names while exposing
/// surrounding definitions. Reuse the evaluator's complete checked-alias
/// attestation, including mutable dependencies and registered rewrite policy;
/// arbitrary same-spelled definitions must not receive a ring interpretation.
/// This does not certify a generated proof, which still requires kernel checking.
pub fn supports_integer_polynomial_operations(ctx: &Context) -> bool {
    accelerates_integer_alias(ctx, "add")
        && accelerates_integer_alias(ctx, "mul")
        && accelerates_integer_alias(ctx, "sub")
}

/// Public arithmetic has a checked meaning on symbolic operands as well as
/// literals. Exact literal evaluation is only an acceleration of these bodies.
pub(crate) fn register_public_arithmetic_aliases(ctx: &mut Context) -> KernelResult<()> {
    for (name, target) in [("add", "Int_add"), ("mul", "Int_mul"), ("sub", "Int_sub")] {
        ctx.declare(name, int_binop(), g(target))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/int_inductive/checked_alias_attestation_controls.rs"]
mod checked_alias_attestation_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/int_inductive/alias_visible_header_control.rs"]
mod alias_visible_header_control;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/int_inductive/subtraction_alias_adversaries.rs"]
mod subtraction_alias_adversaries;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/int_inductive/integer_alias_signature_controls.rs"]
mod integer_alias_signature_controls;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/int_inductive/checked_order_alias_adversaries.rs"]
mod checked_order_alias_adversaries;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/int_inductive/checked_division_attestation_controls.rs"]
mod checked_division_attestation_controls;
