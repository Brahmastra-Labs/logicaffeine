//! Independent verification of reflected ring, linear-arithmetic, and congruence goals.
//!
//! The second kernel uses these checks for tactic computations and their derivation
//! conclusions. Linear arithmetic and congruence emit certificates and replay them;
//! ring uses exact Kronecker substitution rather than polynomial normalization.
//! None calls the first kernel's arithmetic engines or reflected-syntax reader.
//!
//! A successful check corroborates the primitive's success constructor. Failure means
//! the independent checker has no verdict; it never licenses an error reduction.
//! The proof crate re-exports this module for certificate producers and clients.

use std::collections::BTreeMap;

use crate::{BigInt, Literal, Term};

// =================================================================================================
// THE REFLECTED-SYNTAX READER
// =================================================================================================

/// The object language's four shapes, as this module reads them.
///
/// The reflection grammar the tactic primitives are handed is `SLit n`, `SVar i`, `SName s` and
/// `SApp f x` — four global constructors applied to literals and to each other. That is the
/// whole of it. Reading it here rather than through the kernel's `reify` extractors is the
/// point: a shared reader is a shared assumption, and an assumption both kernels make is one
/// neither checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Syn {
    /// `SLit n` — an integer literal of the object language.
    Lit(i64),
    /// `SVar i` — a de Bruijn occurrence.
    Var(i64),
    /// `SName s` — a named global: a free variable to arithmetic, a function symbol to
    /// congruence.
    Name(String),
    /// `SApp f x` — object-language application, curried.
    App(Box<Syn>, Box<Syn>),
}

impl Syn {
    /// `SApp (SApp (SName op) a) b`, destructured — the two-argument spine every relation and
    /// every arithmetic operator arrives in.
    fn binary(&self) -> Option<(&str, &Syn, &Syn)> {
        let Syn::App(fx, b) = self else { return None };
        let Syn::App(f, a) = fx.as_ref() else {
            return None;
        };
        let Syn::Name(op) = f.as_ref() else {
            return None;
        };
        Some((op.as_str(), a, b.as_ref()))
    }
}

/// `SApp (SApp (SApp (SName "Eq") ty) a) b`, destructured.
///
/// A free function over the spine rather than a method, because the three-argument form is NOT
/// the two-argument form with a layer added: its head is `SApp (SName "Eq") ty`, not
/// `SName "Eq"`, so a matcher written as "binary, then peel" would read `Eq ty a` out of a
/// four-argument application and hand `ring` the wrong sides.
fn typed_equation(s: &Syn) -> Option<(&Syn, &Syn, &Syn)> {
    let Syn::App(l3, b) = s else { return None };
    let Syn::App(l2, a) = l3.as_ref() else {
        return None;
    };
    let Syn::App(l1, ty) = l2.as_ref() else {
        return None;
    };
    let Syn::Name(head) = l1.as_ref() else {
        return None;
    };
    if head != "Eq" {
        return None;
    }
    Some((ty, a, b))
}

/// Read a kernel `Term` as reflected object syntax, or `None` if it is not one.
///
/// The literal window is EXACTLY `Literal::Int`. An `SLit` carrying a `Literal::BigInt` is
/// refused here because the kernel's `reify::extract_slit` refuses it too, and a re-checker
/// that read a numeral the first kernel cannot would decide goals the first kernel leaves
/// stuck. Widening that window is a change to BOTH kernels or to neither.
pub fn syn_of_term(t: &Term) -> Option<Syn> {
    let Term::App(ctor, arg) = t else { return None };
    if let Term::Global(name) = ctor.as_ref() {
        match name.as_str() {
            "SLit" => {
                return match arg.as_ref() {
                    Term::Lit(Literal::Int(n)) => Some(Syn::Lit(*n)),
                    _ => None,
                }
            }
            "SVar" => {
                return match arg.as_ref() {
                    Term::Lit(Literal::Int(i)) => Some(Syn::Var(*i)),
                    _ => None,
                }
            }
            "SName" => {
                return match arg.as_ref() {
                    Term::Lit(Literal::Text(s)) => Some(Syn::Name(s.clone())),
                    _ => None,
                }
            }
            _ => {}
        }
    }
    // `SApp f x` is `App(App(Global("SApp"), f), x)`.
    if let Term::App(sapp, f) = ctor.as_ref() {
        if matches!(sapp.as_ref(), Term::Global(c) if c == "SApp") {
            let fs = syn_of_term(f)?;
            let xs = syn_of_term(arg)?;
            return Some(Syn::App(Box::new(fs), Box::new(xs)));
        }
    }
    None
}

// =================================================================================================
// THE VARIABLE KEY — no interner, so no interner collision
// =================================================================================================

/// What an arithmetic procedure treats as an unknown.
///
/// `SVar i` and `SName s` are different kinds of thing, and this key keeps them different
/// STRUCTURALLY, where the kernel's `reify::VarInterner` keeps them apart by allocating named
/// globals from `-1_000_000` downwards — a separation that holds for every de Bruijn index the
/// quoter produces (all non-negative) and fails on a hand-built `SVar (-1000000)`, which the
/// interner would identify with the first named global it meets. This module is therefore never
/// more complete than the first kernel, and on that one malformed shape strictly less: it keeps
/// two unknowns apart where the interner merges them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum VarKey {
    /// A de Bruijn occurrence `SVar i`.
    Bound(i64),
    /// A named global `SName s`.
    Named(String),
}

// =================================================================================================
// LINEAR FORMS — integer coefficients, exactly
// =================================================================================================

/// `constant + Σ coeff·var`, with coefficients in ℤ.
///
/// Integer, not rational, and that is a fact about the FRAGMENT rather than a restriction:
/// `SLit` carries an integer and the only multiplication the reader admits is by a literal, so
/// nothing a reified goal can say has a non-integer coefficient. Rationals enter the first
/// kernel only INSIDE Fourier–Motzkin, where a bound is divided by its coefficient; the Farkas
/// check below never divides, so this module stays in ℤ and never rounds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LinForm {
    constant: BigInt,
    coeffs: BTreeMap<VarKey, BigInt>,
}

impl LinForm {
    fn constant(c: BigInt) -> Self {
        LinForm {
            constant: c,
            coeffs: BTreeMap::new(),
        }
    }

    fn var(k: VarKey) -> Self {
        let mut coeffs = BTreeMap::new();
        coeffs.insert(k, BigInt::from_i64(1));
        LinForm {
            constant: BigInt::zero(),
            coeffs,
        }
    }

    fn is_constant(&self) -> bool {
        self.coeffs.is_empty()
    }

    fn add(&self, other: &LinForm) -> LinForm {
        let mut out = self.clone();
        out.constant = out.constant.add(&other.constant);
        for (k, c) in &other.coeffs {
            let slot = out.coeffs.entry(k.clone()).or_insert_with(BigInt::zero);
            *slot = slot.add(c);
            if slot.is_zero() {
                out.coeffs.remove(k);
            }
        }
        out
    }

    fn scale(&self, f: &BigInt) -> LinForm {
        if f.is_zero() {
            return LinForm::constant(BigInt::zero());
        }
        LinForm {
            constant: self.constant.mul(f),
            coeffs: self
                .coeffs
                .iter()
                .map(|(k, c)| (k.clone(), c.mul(f)))
                .collect(),
        }
    }

    fn neg(&self) -> LinForm {
        self.scale(&BigInt::from_i64(-1))
    }

    fn sub(&self, other: &LinForm) -> LinForm {
        self.add(&other.neg())
    }

    fn coeff(&self, k: &VarKey) -> BigInt {
        self.coeffs.get(k).cloned().unwrap_or_else(BigInt::zero)
    }

    /// The largest magnitude anywhere in the form, in bytes — the meter the elimination budget
    /// reads. Fourier–Motzkin multiplies coefficients pairwise, so an unbudgeted run on an
    /// adversarial system grows them geometrically, and a checker that dies of memory is
    /// exactly as useless as one that answers wrongly.
    fn magnitude_bytes(&self) -> usize {
        let mut worst = self.constant.to_le_bytes().1.len();
        for c in self.coeffs.values() {
            worst = worst.max(c.to_le_bytes().1.len());
        }
        worst
    }
}

/// `form ≤ 0`, or `form < 0` when `strict`.
///
/// The same normal form the kernel's `lia::Constraint` uses, reached independently: every
/// comparison becomes an upper bound on a linear form, so that a non-negative combination of
/// constraints is again a constraint of the same shape — which is the entire mechanism Farkas
/// duality runs on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ineq {
    form: LinForm,
    strict: bool,
}

impl Ineq {
    /// Whether a CONSTANT inequality is false — `c ≤ 0` with `c > 0`, or `c < 0` with `c ≥ 0`.
    fn is_constant_contradiction(&self) -> bool {
        if !self.form.is_constant() {
            return false;
        }
        if self.strict {
            !self.form.constant.is_negative()
        } else {
            self.form.constant.is_positive()
        }
    }
}

// =================================================================================================
// LIA — THE FARKAS CERTIFICATE
// =================================================================================================

/// A refutation of a constraint system, as the multipliers that refute it.
///
/// Farkas' lemma in the form the checker uses: if every `λᵢ ≥ 0` and `Σ λᵢ·formᵢ` is a CONSTANT
/// `c` with `c > 0` — or with `c ≥ 0` when some `λᵢ > 0` weights a STRICT constraint — then no
/// assignment satisfies every `formᵢ ≤ 0` at once, because that assignment would have to make a
/// non-negative combination of non-positive numbers positive.
///
/// This is Lean's `Justification` flattened. Lean keeps the derivation as a TREE of
/// `combo a j b k` nodes (`Lean/Elab/Tactic/Omega/Core.lean:49`) because it must assemble a
/// proof TERM out of `combo_sat'` applications; a checker that only has to agree with a VERDICT
/// can multiply the tree out into one multiplier per assumption and check a single dot product.
/// Same certificate, one level flatter, and the flattening is what makes the checker fit on a
/// page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiaCert {
    multipliers: Vec<BigInt>,
}

impl LiaCert {
    /// The multipliers, in constraint order.
    pub fn multipliers(&self) -> &[BigInt] {
        &self.multipliers
    }

    /// Build a certificate from raw multipliers — the door a test uses to hand the checker a
    /// LIE and require it to be refused. A certificate no mutation can spoil is a certificate
    /// nothing is checking.
    pub fn from_multipliers(multipliers: Vec<BigInt>) -> Self {
        LiaCert { multipliers }
    }
}

/// How large a coefficient the elimination will carry before giving up, in bytes.
///
/// The emitter is untrusted, so this bound costs nothing but completeness: an abandoned search
/// produces no certificate, the checker verifies nothing, and the caller falls back to the
/// honest `Unchecked`.
const MAX_COEFF_BYTES: usize = 64;

/// How many constraints the elimination will hold at once. Fourier–Motzkin is doubly
/// exponential in the worst case; this is the door before that.
const MAX_CONSTRAINTS: usize = 512;

/// Check `cert` against `system`: are the multipliers a Farkas refutation?
///
/// Nothing here searches. It multiplies, adds, and looks at the sign of one constant — which is
/// why it can be believed about a system refuted by a search it does not contain.
fn check_farkas(system: &[Ineq], cert: &LiaCert) -> bool {
    if system.is_empty() || cert.multipliers.len() != system.len() {
        return false;
    }
    let mut combination = LinForm::constant(BigInt::zero());
    let mut any_positive = false;
    let mut strict = false;
    for (c, lambda) in system.iter().zip(cert.multipliers.iter()) {
        if lambda.is_negative() {
            return false;
        }
        if lambda.is_zero() {
            continue;
        }
        any_positive = true;
        if c.strict {
            strict = true;
        }
        combination = combination.add(&c.form.scale(lambda));
    }
    if !any_positive {
        return false;
    }
    Ineq {
        form: combination,
        strict,
    }
    .is_constant_contradiction()
}

/// Search for a Farkas refutation of `system` — UNTRUSTED, and deliberately so.
///
/// Fourier–Motzkin elimination carrying provenance: every constraint holds the vector of
/// multipliers over the ORIGINAL system that produced it, and eliminating `v` from a lower
/// bound `L` (coefficient `a < 0` on `v`) against an upper bound `U` (coefficient `b > 0`)
/// forms `(-a)·U + b·L`, whose `v` coefficient is `(-a)b + ba = 0` and whose provenance is the
/// same combination of theirs. Both weights are positive, so provenance stays a NON-NEGATIVE
/// combination at every step — exactly the invariant [`check_farkas`] re-establishes from
/// scratch, without being told any of this.
///
/// Cross-multiplying instead of dividing through by the coefficient (which is what the kernel's
/// `lia.rs` does, and why it needs rationals) is the reason this module stays in ℤ.
fn find_farkas(system: &[Ineq]) -> Option<LiaCert> {
    if system.is_empty() || system.len() > MAX_CONSTRAINTS {
        return None;
    }
    let n = system.len();
    let unit = |i: usize| -> Vec<BigInt> {
        (0..n)
            .map(|j| BigInt::from_i64(i64::from(j == i)))
            .collect()
    };
    let mut current: Vec<(Ineq, Vec<BigInt>)> = system
        .iter()
        .enumerate()
        .map(|(i, c)| (c.clone(), unit(i)))
        .collect();

    // A contradiction already present in the input needs no elimination at all — which is the
    // only shape `try_lia` reaches today, since it hands Fourier–Motzkin a single constraint.
    for (c, mult) in &current {
        if c.is_constant_contradiction() {
            return Some(LiaCert {
                multipliers: mult.clone(),
            });
        }
    }

    let mut vars: Vec<VarKey> = current
        .iter()
        .flat_map(|(c, _)| c.form.coeffs.keys().cloned())
        .collect();
    vars.sort();
    vars.dedup();

    for v in vars {
        let mut lower: Vec<(Ineq, Vec<BigInt>, BigInt)> = Vec::new();
        let mut upper: Vec<(Ineq, Vec<BigInt>, BigInt)> = Vec::new();
        let mut next: Vec<(Ineq, Vec<BigInt>)> = Vec::new();
        for (c, mult) in &current {
            let a = c.form.coeff(&v);
            if a.is_zero() {
                next.push((c.clone(), mult.clone()));
            } else if a.is_positive() {
                upper.push((c.clone(), mult.clone(), a));
            } else {
                lower.push((c.clone(), mult.clone(), a));
            }
        }
        for (lo, lo_mult, a) in &lower {
            for (up, up_mult, b) in &upper {
                let wa = a.negated();
                let form = up.form.scale(&wa).add(&lo.form.scale(b));
                if form.magnitude_bytes() > MAX_COEFF_BYTES {
                    return None;
                }
                let combined = Ineq {
                    form,
                    strict: lo.strict || up.strict,
                };
                let mult: Vec<BigInt> = (0..n)
                    .map(|i| up_mult[i].mul(&wa).add(&lo_mult[i].mul(b)))
                    .collect();
                if combined.is_constant_contradiction() {
                    return Some(LiaCert { multipliers: mult });
                }
                next.push((combined, mult));
                if next.len() > MAX_CONSTRAINTS {
                    return None;
                }
            }
        }
        current = next;
    }
    None
}

/// Reify a reflected arithmetic term to a linear form, or `None` outside the fragment.
///
/// The fragment is the kernel's `lia::reify_linear`, name for name: `SLit`, `SVar`, `SName`,
/// `add`, `sub`, and `mul` ONLY when one side is constant. `div` and `mod` are not linear and
/// are refused here as they are there — this module may not decide a goal the first kernel
/// leaves stuck.
fn lin_of_syn(s: &Syn) -> Option<LinForm> {
    match s {
        Syn::Lit(n) => Some(LinForm::constant(BigInt::from_i64(*n))),
        Syn::Var(i) => Some(LinForm::var(VarKey::Bound(*i))),
        Syn::Name(n) => Some(LinForm::var(VarKey::Named(n.clone()))),
        Syn::App(..) => {
            let (op, a, b) = s.binary()?;
            let la = lin_of_syn(a)?;
            let lb = lin_of_syn(b)?;
            match op {
                "add" => Some(la.add(&lb)),
                "sub" => Some(la.sub(&lb)),
                "mul" => {
                    if la.is_constant() {
                        Some(lb.scale(&la.constant))
                    } else if lb.is_constant() {
                        Some(la.scale(&lb.constant))
                    } else {
                        None
                    }
                }
                _ => None,
            }
        }
    }
}

/// The system whose unsatisfiability makes `goal` valid: the goal's NEGATION, as one
/// constraint.
///
/// The kernel's `lia::goal_to_negated_constraint` table, re-derived rather than copied: `a ≤ b`
/// is valid exactly when `b - a < 0` is unsatisfiable, `a < b` exactly when `b - a ≤ 0` is, and
/// the `≥`/`>` rows are those two with the sides swapped.
fn lia_negated_system(goal: &Syn) -> Option<Vec<Ineq>> {
    let (rel, lhs, rhs) = goal.binary()?;
    let a = lin_of_syn(lhs)?;
    let b = lin_of_syn(rhs)?;
    let c = match rel {
        "Lt" | "lt" => Ineq {
            form: b.sub(&a),
            strict: false,
        },
        "Le" | "le" => Ineq {
            form: b.sub(&a),
            strict: true,
        },
        "Gt" | "gt" => Ineq {
            form: a.sub(&b),
            strict: false,
        },
        "Ge" | "ge" => Ineq {
            form: a.sub(&b),
            strict: true,
        },
        _ => return None,
    };
    Some(vec![c])
}

/// Emit a Farkas certificate for a `try_lia` goal, or `None` when the search finds none.
///
/// The producer is untrusted — this is the half a re-checker is allowed to get wrong, because
/// [`check_lia`] re-establishes the claim from the certificate alone.
pub fn emit_lia(goal: &Syn) -> Option<LiaCert> {
    find_farkas(&lia_negated_system(goal)?)
}

/// Check a Farkas certificate against a `try_lia` goal.
pub fn check_lia(goal: &Syn, cert: &LiaCert) -> bool {
    match lia_negated_system(goal) {
        Some(system) => check_farkas(&system, cert),
        None => false,
    }
}

/// `try_lia goal` reduces to `DLiaSolve goal`, verified from a certificate this module both
/// produced and then checked without reference to how it was produced.
///
/// COMPLETENESS, stated so the one-sided contract can be audited: a verified certificate means
/// the negated system is genuinely infeasible over ℚ, and Fourier–Motzkin is complete for
/// exactly that, so the first kernel succeeds wherever this returns `true`. The converse is not
/// claimed — a `false` is `Unchecked`, never a disagreement.
pub fn verify_lia(goal: &Syn) -> bool {
    match emit_lia(goal) {
        Some(cert) => check_lia(goal, &cert),
        None => false,
    }
}

// =================================================================================================
// RING — A SECOND DECISION PROCEDURE, BY KRONECKER SUBSTITUTION
// =================================================================================================

/// The per-variable degree bound and the sum of coefficient magnitudes of a reflected
/// polynomial expression, computed structurally.
///
/// Both are UPPER bounds on the polynomial the expression denotes, and neither expands it:
/// `deg` is pointwise `max` at a sum and pointwise `+` at a product, `weight` is `+` at a sum
/// and `×` at a product. `weight` bounds the sum of the absolute values of the coefficients,
/// hence bounds every individual coefficient — which is what the substitution below needs.
struct PolyBounds {
    deg: BTreeMap<VarKey, u32>,
    weight: BigInt,
}

fn poly_bounds(s: &Syn) -> Option<PolyBounds> {
    match s {
        Syn::Lit(n) => Some(PolyBounds {
            deg: BTreeMap::new(),
            weight: BigInt::from_i64(*n).abs(),
        }),
        Syn::Var(i) => {
            let mut deg = BTreeMap::new();
            deg.insert(VarKey::Bound(*i), 1u32);
            Some(PolyBounds {
                deg,
                weight: BigInt::from_i64(1),
            })
        }
        Syn::Name(n) => {
            let mut deg = BTreeMap::new();
            deg.insert(VarKey::Named(n.clone()), 1u32);
            Some(PolyBounds {
                deg,
                weight: BigInt::from_i64(1),
            })
        }
        Syn::App(..) => {
            let (op, a, b) = s.binary()?;
            let pa = poly_bounds(a)?;
            let pb = poly_bounds(b)?;
            match op {
                "add" | "sub" => {
                    let mut deg = pa.deg;
                    for (k, d) in pb.deg {
                        let slot = deg.entry(k).or_insert(0);
                        *slot = (*slot).max(d);
                    }
                    Some(PolyBounds {
                        deg,
                        weight: pa.weight.add(&pb.weight),
                    })
                }
                "mul" => {
                    let mut deg = pa.deg;
                    for (k, d) in pb.deg {
                        let slot = deg.entry(k).or_insert(0);
                        *slot = slot.checked_add(d)?;
                    }
                    Some(PolyBounds {
                        deg,
                        weight: pa.weight.mul(&pb.weight),
                    })
                }
                _ => None,
            }
        }
    }
}

/// Evaluate a reflected polynomial expression at an integer point, exactly.
fn eval_at(s: &Syn, point: &BTreeMap<VarKey, BigInt>) -> Option<BigInt> {
    match s {
        Syn::Lit(n) => Some(BigInt::from_i64(*n)),
        Syn::Var(i) => point.get(&VarKey::Bound(*i)).cloned(),
        Syn::Name(n) => point.get(&VarKey::Named(n.clone())).cloned(),
        Syn::App(..) => {
            let (op, a, b) = s.binary()?;
            let va = eval_at(a, point)?;
            let vb = eval_at(b, point)?;
            match op {
                "add" => Some(va.add(&vb)),
                "sub" => Some(va.sub(&vb)),
                "mul" => Some(va.mul(&vb)),
                _ => None,
            }
        }
    }
}

/// How many monomial slots the Kronecker point may have to keep apart — the product of
/// `(degᵢ + 1)` over the variables.
const MAX_KRONECKER_SLOTS: u32 = 1 << 14;

/// How many bits the substitution point's largest power may occupy before the procedure gives
/// up and lets the caller fall back to `Unchecked`.
const MAX_KRONECKER_BITS: u64 = 1 << 21;

/// Decide whether two reflected polynomial expressions denote the SAME polynomial over ℤ —
/// without ever forming a polynomial.
///
/// **Kronecker substitution.** Order the variables `x₁ … xₙ` and let `dᵢ` bound the degree of
/// either side in `xᵢ`. Substituting `xᵢ ↦ B^(Eᵢ)` with `E₁ = 1` and `Eᵢ₊₁ = Eᵢ·(dᵢ + 1)` sends
/// distinct monomials `∏ xᵢ^(eᵢ)` with `eᵢ ≤ dᵢ` to distinct powers `B^(Σ eᵢEᵢ)`, because the
/// exponent vector is precisely the mixed-radix digit string of that exponent. Taking
/// `B = 2·C + 1` with `C` bounding every coefficient makes the value's balanced base-`B`
/// expansion unique, so the two sides evaluate to the same integer if and only if they are the
/// same polynomial.
///
/// This decides the relation `ring`'s canonical normal form decides, by an argument that shares
/// no step with it: no monomial map, no like-term collection, no comparison of normal forms.
/// Exact `BigInt` arithmetic throughout — an evaluation that wrapped would equate unequal
/// polynomials, which is the failure the first kernel's own doc comment records as its reason
/// for arbitrary precision.
fn polynomials_identical(a: &Syn, b: &Syn) -> Option<bool> {
    let ba = poly_bounds(a)?;
    let bb = poly_bounds(b)?;
    let mut deg = ba.deg;
    for (k, d) in bb.deg {
        let slot = deg.entry(k).or_insert(0);
        *slot = (*slot).max(d);
    }
    let coeff_bound = ba.weight.add(&bb.weight);
    let base = coeff_bound
        .mul(&BigInt::from_i64(2))
        .add(&BigInt::from_i64(1));

    let mut slots: u32 = 1;
    let mut point: BTreeMap<VarKey, BigInt> = BTreeMap::new();
    for (k, d) in &deg {
        let base_bits = (base.to_le_bytes().1.len() as u64).saturating_mul(8);
        if base_bits.saturating_mul(u64::from(slots)) > MAX_KRONECKER_BITS {
            return None;
        }
        point.insert(k.clone(), base.pow(slots));
        slots = slots.checked_mul(d.checked_add(1)?)?;
        if slots > MAX_KRONECKER_SLOTS {
            return None;
        }
    }

    Some(eval_at(a, &point)? == eval_at(b, &point)?)
}

/// `try_ring goal` reduces to `DRingSolve goal`, verified here.
///
/// The gate is the first kernel's: a three-argument `Eq ty lhs rhs` whose `ty` is `Int` or
/// `Nat`, both sides inside the `add`/`sub`/`mul` fragment. `div` and `mod` fall out of
/// `poly_bounds` exactly as `ring::reify` refuses them.
pub fn verify_ring(goal: &Syn) -> bool {
    let Some((ty, lhs, rhs)) = typed_equation(goal) else {
        return false;
    };
    let Syn::Name(ty_name) = ty else { return false };
    if ty_name != "Int" && ty_name != "Nat" {
        return false;
    }
    polynomials_identical(lhs, rhs).unwrap_or(false)
}

// =================================================================================================
// CC — THE CONGRUENCE PROOF CERTIFICATE
// =================================================================================================

/// One step of an equality proof.
///
/// Five rules, and between them they are the whole of equational logic on a first-order term
/// language. A congruence-closure ENGINE is a fixpoint over an e-graph with a union-find and
/// use lists; a PROOF that one particular equation is in that closure is this list, and
/// checking it needs neither. That asymmetry is the entire reason a second kernel can afford to
/// look at `try_cc` at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CcStep {
    /// The goal's `i`-th equality hypothesis, as it stands.
    Hyp(usize),
    /// `t = t`, for the term at table index `t`.
    Refl(usize),
    /// From step `i` proving `a = b`, conclude `b = a`.
    Sym(usize),
    /// From step `i` proving `a = b` and step `j` proving `b = c`, conclude `a = c`.
    Trans(usize, usize),
    /// From step `i` proving `f = g` and step `j` proving `x = y`, conclude `f x = g y`.
    Cong(usize, usize),
}

/// A checkable proof that a goal's conclusion follows from its hypotheses by congruence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CcCert {
    steps: Vec<CcStep>,
}

impl CcCert {
    /// The steps, in dependency order.
    pub fn steps(&self) -> &[CcStep] {
        &self.steps
    }

    /// Build a certificate from raw steps — the door a test uses to hand the checker a forged
    /// proof and require it to be refused.
    pub fn from_steps(steps: Vec<CcStep>) -> Self {
        CcCert { steps }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CcNode {
    Lit(i64),
    Var(i64),
    Name(String),
    App(usize, usize),
}

/// A hash-consed first-order term table. `App(f, x)` names earlier entries by index, so a term
/// IS an index and equality of indices is syntactic identity.
#[derive(Default)]
struct TermTable {
    nodes: Vec<CcNode>,
}

impl TermTable {
    fn add(&mut self, n: CcNode) -> usize {
        if let Some(i) = self.nodes.iter().position(|m| *m == n) {
            return i;
        }
        self.nodes.push(n);
        self.nodes.len() - 1
    }

    fn intern(&mut self, s: &Syn) -> Option<usize> {
        let node = match s {
            Syn::Lit(n) => CcNode::Lit(*n),
            Syn::Var(i) => CcNode::Var(*i),
            Syn::Name(n) => CcNode::Name(n.clone()),
            Syn::App(f, x) => {
                let fi = self.intern(f)?;
                let xi = self.intern(x)?;
                CcNode::App(fi, xi)
            }
        };
        Some(self.add(node))
    }
}

/// `SApp (SApp (SName "Eq") a) b` — the TWO-argument equation congruence closure reads.
///
/// Deliberately not [`typed_equation`]: the kernel's `cc::extract_equality` matches the untyped
/// spine and accepts the lowercase spelling `eq` as well, and a re-checker that demanded the
/// typed form would decline goals the first kernel decides — which costs completeness, not
/// soundness, but costs it for no reason.
fn untyped_equation(s: &Syn) -> Option<(&Syn, &Syn)> {
    let (op, a, b) = s.binary()?;
    if op == "Eq" || op == "eq" {
        Some((a, b))
    } else {
        None
    }
}

/// Peel `implies h₁ (implies h₂ … concl)`, keeping the hypotheses that are equations.
///
/// The kernel's `cc::decompose_goal` shape: a non-equational hypothesis is DROPPED rather than
/// refused, because congruence closure has nothing to do with it and refusing would lose goals
/// for nothing.
fn cc_decompose(goal: &Syn) -> (Vec<(&Syn, &Syn)>, &Syn) {
    let mut hyps = Vec::new();
    let mut current = goal;
    while let Some(("implies", h, rest)) = current.binary() {
        if let Some(eq) = untyped_equation(h) {
            hyps.push(eq);
        }
        current = rest;
    }
    (hyps, current)
}

/// How many steps a `try_cc` certificate may hold. This is the CHECKER's door, and it bounds
/// the work a hostile certificate can ask for.
const MAX_CC_STEPS: usize = 4096;

/// How many pairs the emitter's saturation may derive, and how many rounds it may take, before
/// giving up. Separate from — and tighter than — the checker's door, because the search is
/// quadratic in the pairs it holds while the check is linear in the steps it is handed. A
/// budget spent here costs completeness only: no certificate, no verification, `Unchecked`.
const MAX_CC_SEARCH_STEPS: usize = 512;

/// How many saturation rounds the emitter will run. Congruence closure over a goal's own
/// subterms converges in a handful; this is the door before an adversarial goal turns the
/// emitter into the hang a re-checker must never have.
const MAX_CC_ROUNDS: usize = 16;

/// Check a congruence proof against a goal.
///
/// A LINEAR REPLAY: each step's conclusion is computed from steps that come strictly BEFORE it,
/// each premise is checked to say what its rule needs it to say, and the last step must be the
/// goal's conclusion, orientation included. There is no union-find here and no e-graph; a
/// checker that rebuilt one would be the copy this module exists to avoid.
pub fn check_cc(goal: &Syn, cert: &CcCert) -> bool {
    if cert.steps.is_empty() || cert.steps.len() > MAX_CC_STEPS {
        return false;
    }
    let (hyps, concl) = cc_decompose(goal);
    let Some((clhs, crhs)) = untyped_equation(concl) else {
        return false;
    };

    let mut table = TermTable::default();
    let (Some(goal_l), Some(goal_r)) = (table.intern(clhs), table.intern(crhs)) else {
        return false;
    };
    let mut hyp_ids = Vec::with_capacity(hyps.len());
    for (hl, hr) in &hyps {
        let (Some(a), Some(b)) = (table.intern(hl), table.intern(hr)) else {
            return false;
        };
        hyp_ids.push((a, b));
    }

    let mut proved: Vec<(usize, usize)> = Vec::with_capacity(cert.steps.len());
    for (at, step) in cert.steps.iter().enumerate() {
        let claim = match step {
            CcStep::Hyp(i) => match hyp_ids.get(*i) {
                Some(p) => *p,
                None => return false,
            },
            CcStep::Refl(t) => {
                if *t >= table.nodes.len() {
                    return false;
                }
                (*t, *t)
            }
            CcStep::Sym(i) => {
                if *i >= at {
                    return false;
                }
                let (a, b) = proved[*i];
                (b, a)
            }
            CcStep::Trans(i, j) => {
                if *i >= at || *j >= at {
                    return false;
                }
                let (a, b) = proved[*i];
                let (b2, c) = proved[*j];
                if b != b2 {
                    return false;
                }
                (a, c)
            }
            CcStep::Cong(i, j) => {
                if *i >= at || *j >= at {
                    return false;
                }
                let (f, g) = proved[*i];
                let (x, y) = proved[*j];
                let left = table.add(CcNode::App(f, x));
                let right = table.add(CcNode::App(g, y));
                (left, right)
            }
        };
        proved.push(claim);
    }
    proved.last().copied() == Some((goal_l, goal_r))
}

/// Search for a congruence proof of `goal` — UNTRUSTED.
///
/// Saturation over PAIRS rather than a union-find: seed with reflexivity on every subterm of
/// the goal and with each hypothesis, then close under symmetry, transitivity and congruence,
/// recording the rule that derived each new pair, until the conclusion appears. The trace is
/// already in dependency order, because a pair is only ever derived from pairs already in the
/// list — which is exactly what [`check_cc`] demands of the indices.
///
/// Congruence is applied only where the LEFT side already occurs in the goal's term table.
/// Without that restriction the saturation is unbounded — `f x` can always be applied again —
/// and an emitter that does not terminate is a re-checker that hangs.
pub fn emit_cc(goal: &Syn) -> Option<CcCert> {
    let (hyps, concl) = cc_decompose(goal);
    let (clhs, crhs) = untyped_equation(concl)?;

    let mut table = TermTable::default();
    let goal_l = table.intern(clhs)?;
    let goal_r = table.intern(crhs)?;
    let mut hyp_ids = Vec::with_capacity(hyps.len());
    for (hl, hr) in &hyps {
        hyp_ids.push((table.intern(hl)?, table.intern(hr)?));
    }

    let mut steps: Vec<CcStep> = Vec::new();
    let mut proved: Vec<(usize, usize)> = Vec::new();
    let mut seen: BTreeMap<(usize, usize), usize> = BTreeMap::new();

    fn push(
        steps: &mut Vec<CcStep>,
        proved: &mut Vec<(usize, usize)>,
        seen: &mut BTreeMap<(usize, usize), usize>,
        claim: (usize, usize),
        step: CcStep,
    ) -> Option<()> {
        if seen.contains_key(&claim) {
            return Some(());
        }
        if steps.len() >= MAX_CC_SEARCH_STEPS {
            return None;
        }
        steps.push(step);
        proved.push(claim);
        seen.insert(claim, steps.len() - 1);
        Some(())
    }

    for t in 0..table.nodes.len() {
        push(&mut steps, &mut proved, &mut seen, (t, t), CcStep::Refl(t))?;
    }
    for (i, pair) in hyp_ids.iter().enumerate() {
        push(&mut steps, &mut proved, &mut seen, *pair, CcStep::Hyp(i))?;
    }

    for _round in 0..MAX_CC_ROUNDS {
        if let Some(i) = seen.get(&(goal_l, goal_r)).copied() {
            return Some(finalize_cc(&steps, i));
        }
        let before = steps.len();

        let snapshot = proved.clone();
        for (i, (a, b)) in snapshot.iter().enumerate() {
            push(&mut steps, &mut proved, &mut seen, (*b, *a), CcStep::Sym(i))?;
        }

        let snapshot = proved.clone();
        for (i, (a, b)) in snapshot.iter().enumerate() {
            for (j, (b2, c)) in snapshot.iter().enumerate() {
                if b == b2 {
                    push(
                        &mut steps,
                        &mut proved,
                        &mut seen,
                        (*a, *c),
                        CcStep::Trans(i, j),
                    )?;
                }
            }
        }

        let snapshot = proved.clone();
        let live = table.nodes.clone();
        for (i, (f, g)) in snapshot.iter().enumerate() {
            for (j, (x, y)) in snapshot.iter().enumerate() {
                if !live.iter().any(|n| *n == CcNode::App(*f, *x)) {
                    continue;
                }
                let left = table.add(CcNode::App(*f, *x));
                let right = table.add(CcNode::App(*g, *y));
                push(
                    &mut steps,
                    &mut proved,
                    &mut seen,
                    (left, right),
                    CcStep::Cong(i, j),
                )?;
            }
        }

        if steps.len() == before {
            return None;
        }
    }
    seen.get(&(goal_l, goal_r))
        .copied()
        .map(|i| finalize_cc(&steps, i))
}

/// Trim a saturation trace to the steps the conclusion actually rests on, and re-index.
///
/// A certificate carrying a thousand irrelevant pairs would still CHECK, so this is not about
/// soundness — it is about a certificate a person can read, and about the checker's cost being
/// the proof's size rather than the search's.
fn finalize_cc(steps: &[CcStep], goal_step: usize) -> CcCert {
    let mut needed = vec![false; steps.len()];
    let mut stack = vec![goal_step];
    while let Some(i) = stack.pop() {
        if needed[i] {
            continue;
        }
        needed[i] = true;
        match &steps[i] {
            CcStep::Hyp(_) | CcStep::Refl(_) => {}
            CcStep::Sym(j) => stack.push(*j),
            CcStep::Trans(j, k) | CcStep::Cong(j, k) => {
                stack.push(*j);
                stack.push(*k);
            }
        }
    }
    let mut remap = vec![usize::MAX; steps.len()];
    let mut out = Vec::new();
    for (i, keep) in needed.iter().enumerate() {
        if !keep {
            continue;
        }
        let step = match &steps[i] {
            CcStep::Hyp(h) => CcStep::Hyp(*h),
            CcStep::Refl(t) => CcStep::Refl(*t),
            CcStep::Sym(j) => CcStep::Sym(remap[*j]),
            CcStep::Trans(j, k) => CcStep::Trans(remap[*j], remap[*k]),
            CcStep::Cong(j, k) => CcStep::Cong(remap[*j], remap[*k]),
        };
        remap[i] = out.len();
        out.push(step);
    }
    CcCert { steps: out }
}

/// `try_cc goal` reduces to `DccSolve goal`, verified from a certificate this module both
/// produced and then checked without reference to how it was produced.
pub fn verify_cc(goal: &Syn) -> bool {
    match emit_cc(goal) {
        Some(cert) => check_cc(goal, &cert),
        None => false,
    }
}

// =================================================================================================
// THE THREE ENTRY POINTS A RE-CHECKER CALLS
// =================================================================================================

/// A tactic reduction primitive a second kernel can now corroborate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertifiablePrimitive {
    /// `try_ring goal ⟶ DRingSolve goal`.
    Ring,
    /// `try_lia goal ⟶ DLiaSolve goal`.
    Lia,
    /// `try_cc goal ⟶ DccSolve goal`.
    Cc,
}

impl CertifiablePrimitive {
    /// The reduction-rule name this primitive fires on.
    pub fn rule_name(self) -> &'static str {
        match self {
            CertifiablePrimitive::Ring => "try_ring",
            CertifiablePrimitive::Lia => "try_lia",
            CertifiablePrimitive::Cc => "try_cc",
        }
    }

    /// The derivation constructor the rule reduces to on success.
    pub fn success_constructor(self) -> &'static str {
        match self {
            CertifiablePrimitive::Ring => "DRingSolve",
            CertifiablePrimitive::Lia => "DLiaSolve",
            CertifiablePrimitive::Cc => "DccSolve",
        }
    }

    /// Whether `goal` is one this primitive genuinely solves, decided independently.
    ///
    /// `true` licenses the caller to reduce `<rule> goal` to `<ctor> goal`. `false` licenses
    /// NOTHING — in particular it never licenses reducing to the error derivation, because this
    /// module is incomplete on purpose and its silence is not a verdict.
    pub fn verify(self, goal: &Syn) -> bool {
        match self {
            CertifiablePrimitive::Ring => verify_ring(goal),
            CertifiablePrimitive::Lia => verify_lia(goal),
            CertifiablePrimitive::Cc => verify_cc(goal),
        }
    }

    /// The primitive a reduction-rule name denotes, if it is one of the three.
    pub fn of_rule(name: &str) -> Option<Self> {
        match name {
            "try_ring" => Some(CertifiablePrimitive::Ring),
            "try_lia" => Some(CertifiablePrimitive::Lia),
            "try_cc" => Some(CertifiablePrimitive::Cc),
            _ => None,
        }
    }

    /// All three, so a census can iterate them without a hand-written list going stale.
    pub const ALL: [CertifiablePrimitive; 3] = [
        CertifiablePrimitive::Ring,
        CertifiablePrimitive::Lia,
        CertifiablePrimitive::Cc,
    ];
}

// =================================================================================================
// BUILDING REFLECTED SYNTAX — so a caller can state a goal without spelling out four `SApp`s
// =================================================================================================

/// `SApp (SApp (SName op) a) b`.
pub fn syn_binary(op: &str, a: Syn, b: Syn) -> Syn {
    Syn::App(
        Box::new(Syn::App(Box::new(Syn::Name(op.to_string())), Box::new(a))),
        Box::new(b),
    )
}

/// `SApp f x`.
pub fn syn_apply(f: Syn, x: Syn) -> Syn {
    Syn::App(Box::new(f), Box::new(x))
}

/// `SApp (SApp (SApp (SName "Eq") (SName ty)) a) b` — the typed equation `try_ring` reads.
pub fn syn_typed_eq(ty: &str, a: Syn, b: Syn) -> Syn {
    syn_apply(
        syn_apply(
            syn_apply(Syn::Name("Eq".to_string()), Syn::Name(ty.to_string())),
            a,
        ),
        b,
    )
}

/// The same reflected syntax as a kernel `Term`, so a caller can hand the FIRST kernel exactly
/// the goal it handed this module — which is what makes a differential between the two a
/// differential and not two different questions.
pub fn term_of_syn(s: &Syn) -> Term {
    fn ctor(name: &str, arg: Term) -> Term {
        Term::App(Box::new(Term::Global(name.to_string())), Box::new(arg))
    }
    match s {
        Syn::Lit(n) => ctor("SLit", Term::Lit(Literal::Int(*n))),
        Syn::Var(i) => ctor("SVar", Term::Lit(Literal::Int(*i))),
        Syn::Name(n) => ctor("SName", Term::Lit(Literal::Text(n.clone()))),
        Syn::App(f, x) => Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("SApp".to_string())),
                Box::new(term_of_syn(f)),
            )),
            Box::new(term_of_syn(x)),
        ),
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_kernel/tactic_cert/tests.rs"]
mod tests;
