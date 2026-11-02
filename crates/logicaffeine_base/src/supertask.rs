//! The run-time half of the supertask engine: the exact operations the statements it writes call,
//! generic over a value model ([`ValueAlgebra`]) so the interpreter and a compiled program run one
//! implementation.
//!
//! The engine resolves a supertask by rewriting the loop into ordinary statements; what those
//! statements cannot express in arithmetic alone lives here, each as a builtin every tier runs
//! the same way.
//!
//! - [`trip_count`]: how many times a loop stepped by a clock runs — an infinite natural when the
//!   step is infinitesimal.
//! - [`affine_power`]: `Aᴺ·x` for an exact matrix whose eigenvalues are roots of unity, at an
//!   infinite `N`. With `A^z·(A^p − I)^m = 0` (read off the characteristic polynomial, then
//!   checked by multiplying it out), `q ↦ A^(z + r + p·q)·x` is a polynomial of degree below `m` in
//!   `q` for each residue `r`, so `m` samples and their forward differences give
//!   `Aᴺ·x = Σ_j C(q, j)·Δʲ` exactly, read on each residue class of `N`.
//! - [`polynomial_flow`]: `N` Euler steps of a polynomial differential equation, ending on the
//!   validated solution's germ.
//! - [`hybrid_flow`]: `N` steps of a loop whose state flows by a polynomial Euler step between
//!   jumps: the values of the ideal hybrid system it follows (see [`crate::hybrid`]), Zeno tails and
//!   continuous firing included, each a germ known by its standard part or, where the loop's own
//!   value is exactly it, that value.
//! - [`cycle_at`]: the state at step `N` of a recorded run that has entered a cycle.
//! - [`first_exit`]: how many passes a loop makes whose guard compares terms linear in a counter
//!   it steps — `While not (n <= x and x < n + 1):`, the floor of Imamura's Lemma 1 — read where
//!   the guard's truth can change, and on each residue class of the generator where those points
//!   depend on it.
//! - [`search_candidates`]: the naturals at which a search over every natural must look. A
//!   predicate built from linear terms `a·n + b` and remainders modulo `m` is constant between the
//!   integers where some `a·n + b` changes sign and periodic with the period `lcm(m)` there, so its
//!   first witness, if any, lies at a sign change or within one period after one.

use crate::algebraic::Algebraic;
use crate::exact_real::{ExactReal, RealError};
use crate::hybrid;
use crate::germ::{Flow, Germ};
use crate::hypernatural::Hypernatural;
use crate::hyperreal::Hyperreal;
use crate::limit_value::{bounded_lcm, Depends, ResidueError, ResidueType, MAX_MODULUS};
use crate::number::Number;
use crate::numeric::{BigInt, Rational};
use crate::ode::{escape_time_bound, Field as OdeField, Monomial};
use crate::poly::Poly;
use crate::tower::{self, Branch, ExactRef, HyperForm, HyperValue, NaturalValue, Rel, Scale, Shape, TowerValue, ValueAlgebra};
use std::cmp::Ordering;
use std::sync::Arc;

/// A square matrix of values, row by row.
pub type Matrix<V> = Vec<Vec<V>>;

fn int<V: TowerValue>(n: i64) -> V {
    V::from_bigint(BigInt::from_i64(n))
}

fn items<V: ValueAlgebra>(v: &V, what: &str) -> Result<Vec<V>, String> {
    match v.shape() {
        Shape::List(values) => Ok(values),
        _ => Err(format!("{what} is a Seq, got {}", v.type_name())),
    }
}

// ── trip counts ───────────────────────────────────────────────────────────────────────────────

/// A number of the tower, standard or not.
fn is_number<V: TowerValue>(v: &V) -> bool {
    v.is_standard_number() || matches!(v.exact(), Some(e) if !matches!(e, ExactRef::Ordinal(_)))
}

/// A number with no infinitesimal or infinite part and no dependence on the residue.
fn is_standard<V: TowerValue>(v: &V) -> bool {
    v.is_standard_number() || matches!(v.exact(), Some(ExactRef::Algebraic(_)) | Some(ExactRef::Real(_)))
}

/// How many times `While t <relation> bound:` runs when each step adds `step` to `t`, starting
/// from `t0`: `⌈(bound − t0)/step⌉` for a strict bound, `⌊…⌋ + 1` for an inclusive one, `0` when
/// the guard is false at the start. `−1` — not counted, so the loop runs as written — when the
/// clock is not a number, or when a standard step walks away from its bound; a refusal when an
/// infinitesimal step does, since that loop could never end.
pub fn trip_count<V: ValueAlgebra>(t0: &V, step: &V, bound: &V, relation: &str) -> Result<V, String> {
    let (guard, rising) = match relation {
        "<" => (Rel::Lt, true),
        "<=" => (Rel::LtEq, true),
        ">" => (Rel::Gt, false),
        ">=" => (Rel::GtEq, false),
        other => {
            return Err(format!(
                "a clock's guard is a relation <, <=, > or >=, got `{other}`"
            ))
        }
    };
    if !(is_number(t0) && is_number(step) && is_number(bound)) {
        return Ok(int(-1));
    }
    if !V::holds(guard, t0, bound)? {
        return Ok(int(0));
    }
    let toward = if rising { Rel::Gt } else { Rel::Lt };
    if !V::holds(toward, step, &int(0))? {
        if is_standard(step) {
            return Ok(int(-1));
        }
        return Err(format!(
            "the clock never reaches its bound: it starts at {}, steps by {} and runs while it is {relation} {}",
            t0.display_string(),
            step.display_string(),
            bound.display_string()
        ));
    }
    let distance = V::subtract(bound.clone(), t0.clone())?;
    let steps = V::exact_divide(distance, step.clone())?;
    match guard {
        Rel::Lt | Rel::Gt => V::ceil(steps),
        _ => V::add(V::floor(steps)?, int(1)),
    }
}

// ── affine powers ─────────────────────────────────────────────────────────────────────────────

fn matrix_of<V: ValueAlgebra>(v: &V) -> Result<Matrix<V>, String> {
    let rows = items(v, "a supertask's update matrix")?;
    let rows: Matrix<V> = rows
        .iter()
        .map(|r| items(r, "each row of a supertask's update matrix"))
        .collect::<Result<_, _>>()?;
    if rows.is_empty() || rows.iter().any(|r| r.len() != rows.len()) {
        return Err("a supertask's update matrix is square and not empty".to_string());
    }
    Ok(rows)
}

fn identity<V: ValueAlgebra>(n: usize) -> Matrix<V> {
    (0..n)
        .map(|i| (0..n).map(|j| int(i64::from(i == j))).collect())
        .collect()
}

fn mat_mul<V: ValueAlgebra>(a: &Matrix<V>, b: &Matrix<V>) -> Result<Matrix<V>, String> {
    let n = a.len();
    let mut out = Vec::with_capacity(n);
    for row in a {
        let mut out_row = Vec::with_capacity(n);
        for j in 0..n {
            let mut acc = int(0);
            for (k, entry) in row.iter().enumerate() {
                acc = V::add(acc, V::multiply(entry.clone(), b[k][j].clone())?)?;
            }
            out_row.push(acc);
        }
        out.push(out_row);
    }
    Ok(out)
}

fn mat_vec<V: ValueAlgebra>(a: &Matrix<V>, x: &[V]) -> Result<Vec<V>, String> {
    a.iter()
        .map(|row| {
            row.iter().zip(x).try_fold(int(0), |acc, (e, v)| {
                V::add(acc, V::multiply(e.clone(), v.clone())?)
            })
        })
        .collect()
}

fn mat_sub<V: ValueAlgebra>(a: &Matrix<V>, b: &Matrix<V>) -> Result<Matrix<V>, String> {
    a.iter()
        .zip(b)
        .map(|(ra, rb)| {
            ra.iter()
                .zip(rb)
                .map(|(x, y)| V::subtract(x.clone(), y.clone()))
                .collect()
        })
        .collect()
}

fn mat_pow<V: ValueAlgebra>(a: &Matrix<V>, mut e: BigInt) -> Result<Matrix<V>, String> {
    let two = BigInt::from_u64(2);
    let mut acc = identity(a.len());
    let mut base = a.clone();
    while e.is_positive() {
        let (q, r) = e.div_rem(&two).expect("two is nonzero");
        if !r.is_zero() {
            acc = mat_mul(&acc, &base)?;
        }
        e = q;
        if e.is_positive() {
            base = mat_mul(&base, &base)?;
        }
    }
    Ok(acc)
}

fn is_zero<V: ValueAlgebra>(v: &V) -> Result<bool, String> {
    V::holds(Rel::Eq, v, &int(0))
}

/// A standard rational entry of a characteristic polynomial: an integer, a rational, a float as the
/// rational it denotes, or a rational algebraic number; `None` for anything else.
fn as_rational<V: ValueAlgebra>(v: &V) -> Option<Rational> {
    match v.shape() {
        Shape::Int(n) => Some(Rational::from_i64(n)),
        Shape::BigInt(b) => Some(Rational::from_bigint(b)),
        Shape::Rational(r) => Some(r),
        Shape::Float => v.standard_rational(),
        _ => match v.exact()? {
            ExactRef::Algebraic(a) => a.to_rational(),
            _ => None,
        },
    }
}

/// `det(xI − A)` by Faddeev–LeVerrier: `M₀ = 0`, `Mₖ = A·Mₖ₋₁ + c_{n−k+1}·I`,
/// `c_{n−k} = −tr(A·Mₖ)/k`.
fn characteristic_polynomial<V: ValueAlgebra>(a: &Matrix<V>) -> Result<Vec<V>, String> {
    let n = a.len();
    let mut c: Vec<V> = vec![int(0); n + 1];
    c[n] = int(1);
    let mut m: Matrix<V> = vec![vec![int(0); n]; n];
    for k in 1..=n {
        let am = mat_mul(a, &m)?;
        m = am;
        for (i, row) in m.iter_mut().enumerate() {
            row[i] = V::add(row[i].clone(), c[n - k + 1].clone())?;
        }
        let am = mat_mul(a, &m)?;
        let trace = (0..n).try_fold(int(0), |acc, i| V::add(acc, am[i][i].clone()))?;
        let neg = V::subtract(int(0), trace)?;
        c[n - k] = V::exact_divide(neg, int(k as i64))?;
    }
    Ok(c)
}

fn totient(mut n: u64) -> u64 {
    let mut result = n;
    let mut p = 2;
    while p * p <= n {
        if n.is_multiple_of(p) {
            while n.is_multiple_of(p) {
                n /= p;
            }
            result -= result / p;
        }
        p += 1;
    }
    if n > 1 {
        result -= result / n;
    }
    result
}

fn mobius(mut n: u64) -> i32 {
    let mut sign = 1;
    let mut p = 2;
    while p * p <= n {
        if n.is_multiple_of(p) {
            n /= p;
            if n.is_multiple_of(p) {
                return 0;
            }
            sign = -sign;
        }
        p += 1;
    }
    if n > 1 {
        sign = -sign;
    }
    sign
}

/// `Φ_d = ∏_{e | d} (xᵉ − 1)^μ(d/e)`.
fn cyclotomic(d: u64) -> Poly {
    let mut num = Poly::one();
    let mut den = Poly::one();
    for e in (1..=d).filter(|e| d.is_multiple_of(*e)) {
        let mut c = vec![Rational::zero(); e as usize + 1];
        c[0] = Rational::from_i64(-1);
        c[e as usize] = Rational::from_i64(1);
        let factor = Poly::from_coeffs(c);
        match mobius(d / e) {
            1 => num = num.mul(&factor),
            -1 => den = den.mul(&factor),
            _ => {}
        }
    }
    num.div_rem(&den).expect("a nonzero divisor").0
}

/// The order `d` with `Φ_d = f` for a monic irreducible `f`; `None` when `f` is not cyclotomic.
/// `φ(d) ≥ √(d/2)`, so every `d` with `φ(d) = deg f` is below `2·deg²`.
fn cyclotomic_order(f: &Poly) -> Option<u64> {
    let degree = f.degree()? as u64;
    (1..=2 * degree * degree + 2)
        .filter(|d| totient(*d) == degree)
        .find(|d| cyclotomic(*d) == *f)
}

/// `(z, p, m)` with `A^z·(A^p − I)^m = 0`, read off the characteristic polynomial: `z` the
/// multiplicity of the eigenvalue 0, `p` the lcm of the orders of the roots of unity, `m` the
/// largest multiplicity among them.
fn unit_root_structure<V: ValueAlgebra>(a: &Matrix<V>) -> Result<(u64, u64, u64), String> {
    let coefficients = characteristic_polynomial(a)?;
    let rational: Vec<Rational> = coefficients
        .iter()
        .map(|c| {
            as_rational(c).ok_or_else(|| {
                format!(
                    "the update's characteristic polynomial has the coefficient {}; a supertask's closed \
                     form is exact here only for an eigenvalue that is a standard root of unity",
                    c.display_string()
                )
            })
        })
        .collect::<Result<_, _>>()?;
    let chi = Poly::from_coeffs(rational);
    let (mut z, mut p, mut m) = (0u64, 1u64, 0u64);
    for (factor, multiplicity) in chi.factor_over_q() {
        let factor = factor.monic();
        if factor == Poly::x() {
            z = u64::from(multiplicity);
            continue;
        }
        let d = cyclotomic_order(&factor).ok_or_else(|| {
            format!(
                "an eigenvalue of the update is not a root of unity (a root of {factor}); the state \
                 grows, shrinks or turns by an irrational angle at every step, and a supertask over \
                 such an update has no exact closed form here"
            )
        })?;
        p = p / gcd(p, d) * d;
        m = m.max(u64::from(multiplicity));
    }
    Ok((z, p, m))
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// The infinite natural a step count is, or the refusal of anything but a natural number.
fn infinite_count<V: TowerValue>(n: &V) -> Result<NaturalValue, String> {
    match n.exact() {
        Some(ExactRef::Natural(nv)) => Ok(nv.clone()),
        _ if several_count(n) => Err(format!(
            "this supertask's step count, {}, is infinite in more than one infinitesimal, which only an affine \
             closed form or a cycle is read over; a flow is integrated over a count of one",
            n.display_string()
        )),
        _ => Err(format!(
            "a supertask's step count is a natural number, got {}",
            n.display_string()
        )),
    }
}

/// Whether a step count is an infinite integer of several infinitesimals ([`HyperForm::Integer`]).
fn several_count<V: TowerValue>(n: &V) -> bool {
    matches!(n.exact(), Some(ExactRef::Hyper(h)) if matches!(h.value, HyperForm::Integer(_)))
}

/// `f` of a residue `r` — a natural number, or one that depends on the residues of the generators — read
/// on each class it is a natural number on, the classes' answers joined as one value.
fn on_residue<V: ValueAlgebra>(r: &V, f: &dyn Fn(u64) -> Result<V, String>) -> Result<V, String> {
    if let Some(k) = r.integer() {
        return f(k.to_u64().ok_or_else(|| format!("a residue is a natural number, got {}", r.display_string()))?);
    }
    match r.exact() {
        Some(ExactRef::Depends(d)) => tower::map_depends(d, |b| on_residue(&b, f)),
        _ => Err(format!("a residue is a natural number, got {}", r.display_string())),
    }
}

/// `Aᴺ·x` for a step count infinite in several infinitesimals, one dependency closure at a time, as
/// [`affine_power`] reads one of one: an update whose characteristic polynomial is standard and whose
/// eigenvalues are roots of unity by the finite differences of its powers, read on the classes of
/// `N` modulo their period; any other by `exp(N·log A)` in the smallest infinitesimal the update, its
/// state and the count hold ([`crate::nested_flow::power`]).
fn several_count_power<V: ValueAlgebra>(a: &Matrix<V>, x: &[V], n: &V) -> Result<Vec<V>, String> {
    let width = a.len();
    let mut values: Vec<Option<V>> = vec![None; width];
    for i in 0..width {
        if values[i].is_some() {
            continue;
        }
        let closure = dependency_closure(a, i)?;
        let sub_a: Matrix<V> = closure
            .iter()
            .map(|&r| closure.iter().map(|&c| a[r][c].clone()).collect())
            .collect();
        let sub_x: Vec<V> = closure.iter().map(|&k| x[k].clone()).collect();
        let standard = characteristic_polynomial(&sub_a)?
            .iter()
            .all(|c| as_rational(c).is_some());
        let results = if standard {
            let (z, p, m) = unit_root_structure(&sub_a)?;
            check_annihilator(&sub_a, z, p, m)?;
            let differences = root_differences(&sub_a, &sub_x, z, p, m)?;
            evaluate_over_count(n, z, p, &differences, sub_x.len())?
        } else {
            let mut found = Vec::new();
            for v in sub_a.iter().flatten().chain(sub_x.iter()).chain(std::iter::once(n)) {
                infinitesimals_in(v, &mut found);
            }
            for (k, s) in found.iter().enumerate() {
                if let Some(t) = found[k + 1..].iter().find(|t| s.order(t).is_none()) {
                    return Err(s.unordered(t));
                }
            }
            let finest = found
                .iter()
                .find(|s| found.iter().all(|t| s.order(t) != Some(Ordering::Greater)))
                .expect("a count infinite in several infinitesimals holds one")
                .clone();
            let matrix: crate::nested_flow::NMatrix = sub_a.iter().map(|row| row.iter().map(tower::as_number).collect()).collect();
            let start: Vec<Number> = sub_x.iter().map(tower::as_number).collect();
            crate::nested_flow::power(&finest, &matrix, tower::as_number(n), &start)?
                .into_iter()
                .map(tower::from_number)
                .collect()
        };
        for (pos, &k) in closure.iter().enumerate() {
            values[k].get_or_insert_with(|| results[pos].clone());
        }
    }
    Ok(values.into_iter().map(|v| v.expect("every component is resolved")).collect())
}

/// `Aᴺ·x` read from the finite differences of `A^(z + r + p·j)·x` ([`root_differences`]) for a count
/// `N` of any kind: `N − z ≡ r (mod p)` on each class, and `Σ_k C((N − z − r)/p, k)·Δᵏ` there.
fn evaluate_over_count<V: ValueAlgebra>(
    n: &V,
    z: u64,
    p: u64,
    differences: &[Vec<Vec<V>>],
    width: usize,
) -> Result<Vec<V>, String> {
    let shifted = V::subtract(n.clone(), int(z as i64))?;
    let residue = V::modulo(shifted.clone(), int(p as i64))?;
    (0..width)
        .map(|i| {
            on_residue(&residue, &|r| {
                let q = V::exact_divide(V::subtract(shifted.clone(), int(r as i64))?, int(p as i64))?;
                let mut binomial: V = int(1);
                let mut total: V = int(0);
                for (j, delta) in differences[r as usize].iter().enumerate() {
                    if j > 0 {
                        let factor = V::subtract(q.clone(), int(j as i64 - 1))?;
                        binomial = V::exact_divide(V::multiply(binomial, factor)?, int(j as i64))?;
                    }
                    total = V::add(total, V::multiply(binomial.clone(), delta[i].clone())?)?;
                }
                Ok(total)
            })
        })
        .collect()
}

/// `Aᴺ·x`. See the module documentation.
pub fn affine_power<V: ValueAlgebra>(matrix: &V, vector: &V, n: &V) -> Result<V, String> {
    if let Some(on_classes) = on_each_class(&[matrix, vector, n], |args| affine_power(&args[0], &args[1], &args[2])) {
        return on_classes;
    }
    let a = matrix_of(matrix)?;
    let x = items(vector, "a supertask's state")?;
    if x.len() != a.len() {
        return Err(format!(
            "a supertask's state has {} values for a {}-by-{} update",
            x.len(),
            a.len(),
            a.len()
        ));
    }
    let natural = match n.shape() {
        Shape::Int(k) if k >= 0 => {
            return Ok(V::list(mat_vec(&mat_pow(&a, BigInt::from_i64(k))?, &x)?));
        }
        Shape::BigInt(k) if k.is_positive() => {
            return Ok(V::list(mat_vec(&mat_pow(&a, k)?, &x)?));
        }
        _ if several_count(n) => return Ok(V::list(several_count_power(&a, &x, n)?)),
        _ => infinite_count(n)?,
    };
    let width = a.len();
    let mut values: Vec<Option<V>> = vec![None; width];
    for i in 0..width {
        if values[i].is_some() {
            continue;
        }
        let closure = dependency_closure(&a, i)?;
        let sub_a: Matrix<V> = closure
            .iter()
            .map(|&r| closure.iter().map(|&c| a[r][c].clone()).collect())
            .collect();
        let sub_x: Vec<V> = closure.iter().map(|&k| x[k].clone()).collect();
        let standard = characteristic_polynomial(&sub_a)?
            .iter()
            .all(|c| as_rational(c).is_some());
        let results = if standard {
            root_of_unity_power(&sub_a, &sub_x, &natural)?
        } else {
            flow_power(&sub_a, &sub_x, &natural)?
        };
        for (pos, &k) in closure.iter().enumerate() {
            values[k].get_or_insert_with(|| results[pos].clone());
        }
    }
    Ok(V::list(
        values
            .into_iter()
            .map(|v| v.expect("every component is resolved"))
            .collect(),
    ))
}

/// Every component the state's component `i` reads, at any distance, `i` included, ascending.
fn dependency_closure<V: ValueAlgebra>(a: &Matrix<V>, i: usize) -> Result<Vec<usize>, String> {
    let mut seen = vec![false; a.len()];
    let mut stack = vec![i];
    seen[i] = true;
    while let Some(r) = stack.pop() {
        for (c, entry) in a[r].iter().enumerate() {
            if !seen[c] && !is_zero(entry)? {
                seen[c] = true;
                stack.push(c);
            }
        }
    }
    Ok((0..a.len()).filter(|&k| seen[k]).collect())
}

/// The certificate the closed form rests on, checked by multiplying it out: `A^z·(A^p − I)^m = 0`.
pub fn check_annihilator<V: ValueAlgebra>(a: &Matrix<V>, z: u64, p: u64, m: u64) -> Result<(), String> {
    let checked = mat_mul(
        &mat_pow(a, BigInt::from_u64(z))?,
        &mat_pow(
            &mat_sub(&mat_pow(a, BigInt::from_u64(p))?, &identity(a.len()))?,
            BigInt::from_u64(m),
        )?,
    )?;
    for entry in checked.iter().flatten() {
        if !is_zero(entry)? {
            return Err(format!(
                "the supertask engine's certificate A^{z}·(A^{p} − I)^{m} = 0 failed to check; no closed form is used"
            ));
        }
    }
    Ok(())
}

/// `Aᴺ·x` for an update whose characteristic polynomial is standard and whose eigenvalues are
/// roots of unity — its entries may still carry the infinitesimal, as a clock's step does.
fn root_of_unity_power<V: ValueAlgebra>(
    a: &Matrix<V>,
    x: &[V],
    natural: &NaturalValue,
) -> Result<Vec<V>, String> {
    let (z, p, m) = unit_root_structure(a)?;
    check_annihilator(a, z, p, m)?;
    let differences = root_differences(a, x, z, p, m)?;
    evaluate_by_class(natural, z, p, &differences, x.len())
}

/// The finite differences, through order `m`, of `A^(z + r + p·j)·x` in `j`, for each `r < p`: the
/// pieces `Aᴺ·x` is read from on each class of `N − z` modulo `p`.
fn root_differences<V: ValueAlgebra>(a: &Matrix<V>, x: &[V], z: u64, p: u64, m: u64) -> Result<Vec<Vec<Vec<V>>>, String> {
    let samples = m.max(1);
    let mut differences: Vec<Vec<Vec<V>>> = Vec::with_capacity(p as usize);
    for r in 0..p {
        let mut points = Vec::with_capacity(samples as usize);
        for j in 0..samples {
            let power = mat_pow(a, BigInt::from_u64(z + r + p * j))?;
            points.push(mat_vec(&power, x)?);
        }
        let mut deltas = Vec::with_capacity(samples as usize);
        for _ in 0..samples {
            deltas.push(points[0].clone());
            points = points
                .windows(2)
                .map(|w| {
                    w[1].iter()
                        .zip(&w[0])
                        .map(|(b, a)| V::subtract(b.clone(), a.clone()))
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<_, _>>()?;
        }
        differences.push(deltas);
    }
    Ok(differences)
}

/// A polynomial in a root of the infinitesimal `scale` with rational coefficients: the root's `q` and
/// the coefficients of `dt^(k/q)`, lowest power first. `1 + 2·dt + dt^(3/2)` is `(2, [1, 0, 2, 1])`.
fn dt_polynomial<V: ValueAlgebra>(v: &V, scale: &Scale) -> Option<(u32, Vec<Rational>)> {
    let (h, q) = match v.exact() {
        Some(ExactRef::Hyper(h)) if !h.scale.same_infinitesimal(scale) => return None,
        Some(ExactRef::Hyper(h)) => match &h.value {
            HyperForm::Rational(r) => (r.clone(), 1),
            HyperForm::Puiseux(p) => p.rational_function()?,
            HyperForm::Germ(_) | HyperForm::Multi(_) | HyperForm::Integer(_) | HyperForm::Nested(_) => return None,
        },
        _ => return Some((1, vec![as_rational(v)?])),
    };
    let coefficients = h.as_dt_polynomial()?.iter().map(|c| c.to_rational()).collect::<Option<_>>()?;
    Some((q, coefficients))
}

/// Whether a polynomial in `dt^(1/q)` has a term between `dt⁰` and `dt¹`, which no step of a flow has.
fn below_dt(q: u32, coefficients: &[Rational]) -> bool {
    coefficients.iter().take(q as usize).skip(1).any(|c| !c.is_zero())
}

/// The value on the residue class `c`, for a value that depends on the residue.
fn at_class<V: TowerValue>(v: &V, c: u64) -> V {
    match v.exact() {
        Some(ExactRef::Depends(d)) => d.value.at(c).map(|b| b.0.clone()).unwrap_or_else(|| v.clone()),
        Some(ExactRef::Natural(n)) => match n.value.depends().at(c) {
            Some(p) => tower::from_hyper(Hyperreal::from_reciprocal_powers(p.coeffs()), &n.scale),
            None => v.clone(),
        },
        _ => v.clone(),
    }
}

/// The moduli the residue-dependent values of a state are split by.
fn state_moduli<V: TowerValue>(x: &[V]) -> Vec<u64> {
    x.iter()
        .filter_map(|v| match v.exact() {
            Some(ExactRef::Depends(d)) => Some(d.value.modulus()),
            Some(ExactRef::Natural(h)) => Some(h.value.depends().modulus()),
            _ => None,
        })
        .collect()
}

fn classes(moduli: impl IntoIterator<Item = u64>) -> Option<u64> {
    moduli.into_iter().try_fold(1u64, |acc, m| {
        let l = u128::from(acc / gcd(acc, m)) * u128::from(m);
        (l <= u128::from(MAX_MODULUS)).then_some(l as u64)
    })
}

/// `N` steps of `x ↦ A·x` for `A = I + Σ_{k≥1} dt^(k/q)·A_k` with rational `A_k`, none below `dt`, and
/// `N = T/dt + δ` on each residue class: the state as germs (see `crate::germ`).
fn flow_power<V: ValueAlgebra>(a: &Matrix<V>, x: &[V], natural: &NaturalValue) -> Result<Vec<V>, String> {
    let n = a.len();
    let clock = &natural.scale;
    let mut found = Vec::new();
    for v in a.iter().flatten().chain(x.iter()) {
        infinitesimals_in(v, &mut found);
    }
    for s in &found {
        if s.order(clock).is_none() {
            return Err(clock.unordered(s));
        }
    }
    let finer = found
        .iter()
        .filter(|s| s.order(clock) == Some(Ordering::Less))
        .fold(None::<Arc<Scale>>, |least, s| match least {
            Some(t) if t.order(s) == Some(Ordering::Less) => Some(t),
            _ => Some(s.clone()),
        });
    if let Some(finer) = finer {
        return several_on_classes(a, x, natural, |matrix, start, count| {
            crate::nested_flow::power(&finer, matrix, count.clone(), start)
        });
    }
    if found.iter().any(|s| s.order(clock) == Some(Ordering::Greater)) {
        return several_on_classes(a, x, natural, |matrix, start, count| {
            let (t, delta) = linear_count(count)?;
            crate::nested_flow::flow(clock, matrix, t, delta, start)
        });
    }
    let mut entries = Vec::with_capacity(n * n);
    for (i, row) in a.iter().enumerate() {
        for (j, entry) in row.iter().enumerate() {
            let (q, coefficients) = dt_polynomial(entry, &natural.scale).ok_or_else(|| {
                format!(
                    "the update's entry {} is not a polynomial in the infinitesimal, or in a root of it, with rational coefficients, so this supertask is not a flow the engine integrates",
                    entry.display_string()
                )
            })?;
            if below_dt(q, &coefficients) {
                return Err(format!(
                    "the update's entry {} has a term of a fractional order of the infinitesimal below its first, so this supertask is not a flow",
                    entry.display_string()
                ));
            }
            entries.push((i, j, q, coefficients));
        }
    }
    let grid = entries.iter().fold(1u64, |l, (_, _, q, _)| l / gcd(l, u64::from(*q)) * u64::from(*q));
    let grid = u32::try_from(grid).map_err(|_| "the update's entries lie on too fine a root of the infinitesimal".to_string())?;
    let mut orders: Vec<Vec<Vec<Rational>>> = Vec::new();
    for (i, j, q, coefficients) in entries {
        let spread = (grid / q) as usize;
        for (k, c) in coefficients.into_iter().enumerate() {
            while orders.len() <= k * spread {
                orders.push(vec![vec![Rational::zero(); n]; n]);
            }
            orders[k * spread][i][j] = c;
        }
    }
    let identity: Vec<Vec<Rational>> = (0..n)
        .map(|i| (0..n).map(|j| if i == j { Rational::one() } else { Rational::zero() }).collect())
        .collect();
    if orders.first() != Some(&identity) {
        return Err(
            "the update changes its state by a standard amount at every step as well as an infinitesimal one, so this supertask is neither a cycle nor a flow"
                .to_string(),
        );
    }
    let terms: Vec<Vec<Vec<Rational>>> = orders[1..].to_vec();
    if terms.is_empty() {
        return Ok(x.to_vec());
    }
    let scale = &natural.scale;
    let ty = scale.residues.clone();
    let counts = natural.value.depends();
    let l = classes([counts.modulus(), ty.modulus()].into_iter().chain(state_moduli(x)))
        .ok_or_else(|| "a flow's step count and state split over more residue classes than one value holds".to_string())?;
    let mut columns: Vec<Vec<Option<Branch<V>>>> = vec![Vec::with_capacity(l as usize); n];
    for c in 0..l {
        let Some(count) = counts.at(c) else {
            columns.iter_mut().for_each(|col| col.push(None));
            continue;
        };
        if count.degree().unwrap_or(0) != 1 {
            return Err(
                "a flow's step count is a multiple of 1/dt; one that grows faster or slower has no standard end state here"
                    .to_string(),
            );
        }
        let (t, delta) = (count.coeff(1), count.coeff(0));
        if !t.is_positive() {
            return Err("a flow runs forward a positive time".to_string());
        }
        let start: Vec<Germ> = x
            .iter()
            .map(|v| {
                let at = at_class(v, c);
                tower::germ_of(&at).ok_or_else(|| {
                    format!("a flow's state is numbers, not {}", at.display_string())
                })
            })
            .collect::<Result<_, _>>()?;
        let flow = Flow::on_grid(grid, terms.clone(), t, delta, start)
            .ok_or_else(|| "a flow's update and state disagree in shape".to_string())?;
        for (i, column) in columns.iter_mut().enumerate() {
            column.push(Some(Branch(tower::from_germ(Germ::flow(&flow, i), scale))));
        }
    }
    Ok(columns
        .into_iter()
        .map(|raw| {
            let d = Depends::from_branches(ty.clone(), l, raw).expect("the classes are the type's own");
            tower::from_depends(d, scale)
        })
        .collect())
}

/// `N` steps of `x ↦ A·x` for an update that holds infinitesimals other than the clock's, on each
/// residue class of the step count: `end` gives the state from the update, the start and the count
/// on the class, each a [`Number`].
fn several_on_classes<V: ValueAlgebra>(
    a: &Matrix<V>,
    x: &[V],
    natural: &NaturalValue,
    end: impl Fn(&crate::nested_flow::NMatrix, &[Number], &Number) -> Result<Vec<Number>, String>,
) -> Result<Vec<V>, String> {
    let n = a.len();
    let scale = &natural.scale;
    let ty = scale.residues.clone();
    let counts = natural.value.depends();
    let l = classes([counts.modulus(), ty.modulus()].into_iter().chain(state_moduli(x)))
        .ok_or_else(|| "a flow's step count and state split over more residue classes than one value holds".to_string())?;
    let matrix: crate::nested_flow::NMatrix = a.iter().map(|row| row.iter().map(tower::as_number).collect()).collect();
    let mut columns: Vec<Vec<Option<Branch<V>>>> = vec![Vec::with_capacity(l as usize); n];
    for c in 0..l {
        let Some(count) = counts.at(c) else {
            columns.iter_mut().for_each(|col| col.push(None));
            continue;
        };
        let count = tower::as_number::<V>(&tower::from_hyper(Hyperreal::from_reciprocal_powers(count.coeffs()), scale));
        let start: Vec<Number> = x.iter().map(|v| tower::as_number(&at_class(v, c))).collect();
        let state = end(&matrix, &start, &count)?;
        for (column, value) in columns.iter_mut().zip(state) {
            column.push(Some(Branch(tower::from_number(value))));
        }
    }
    Ok(columns
        .into_iter()
        .map(|raw| {
            let d = Depends::from_branches(ty.clone(), l, raw).expect("the classes are the type's own");
            tower::from_depends(d, scale)
        })
        .collect())
}

/// `N = T/dt + δ`, a step count read as the time it covers and its offset; refused for a count that
/// grows faster or slower than `1/dt`.
fn linear_count(count: &Number) -> Result<(Rational, Rational), String> {
    let refuse = || {
        "a flow's step count is a multiple of 1/dt; one that grows faster or slower has no standard end state here".to_string()
    };
    let hyper = match count.exact() {
        Some(ExactRef::Hyper(h)) => h.value.rational().cloned().ok_or_else(refuse)?,
        Some(ExactRef::Natural(n)) => n.value.to_hyperreal().single().cloned().ok_or_else(refuse)?,
        _ => return Err(refuse()),
    };
    let powers = hyper.as_reciprocal_powers().ok_or_else(refuse)?;
    if powers.len() != 2 || !powers[1].is_positive() {
        return Err(if powers.len() == 2 { "a flow runs forward a positive time".to_string() } else { refuse() });
    }
    Ok((powers[1].clone(), powers[0].clone()))
}

/// One monomial of a supertask's polynomial update: its coefficient and its exponents.
struct Term<V> {
    coefficient: V,
    exponents: Vec<u32>,
}

/// A polynomial over a state of `width` values, encoded as a list of `[coefficient, e₁, …, eₖ]`
/// entries.
fn poly_terms<V: ValueAlgebra>(poly: &V, width: usize) -> Result<Vec<Term<V>>, String> {
    items(poly, "a polynomial update's component")?
        .iter()
        .map(|entry| {
            let entry = items(entry, "a polynomial update's term")?;
            if entry.len() != width + 1 {
                return Err("a polynomial update's term names an exponent for every state variable".to_string());
            }
            let exponents = entry[1..]
                .iter()
                .map(|e| match e.shape() {
                    Shape::Int(k) => u32::try_from(k).map_err(|_| "an exponent is a small natural number".to_string()),
                    _ => Err("an exponent is a small natural number".to_string()),
                })
                .collect::<Result<Vec<u32>, String>>()?;
            Ok(Term {
                coefficient: entry[0].clone(),
                exponents,
            })
        })
        .collect()
}

/// The update `field` encodes, one list of `[coefficient, e₁, …, eₖ]` entries per state variable.
fn update_terms<V: ValueAlgebra>(field: &V, width: usize) -> Result<Vec<Vec<Term<V>>>, String> {
    let components = items(field, "a polynomial update")?;
    if components.len() != width {
        return Err(format!(
            "a polynomial update has {} components for a state of {width}",
            components.len()
        ));
    }
    components.iter().map(|component| poly_terms(component, width)).collect()
}

/// A polynomial's value at `x`, in the value model's own arithmetic.
fn eval_terms<V: ValueAlgebra>(terms: &[Term<V>], x: &[V]) -> Result<V, String> {
    let mut total = int(0);
    for term in terms {
        let mut value = term.coefficient.clone();
        for (v, e) in x.iter().zip(&term.exponents) {
            for _ in 0..*e {
                value = V::multiply(value, v.clone())?;
            }
        }
        total = V::add(total, value)?;
    }
    Ok(total)
}

/// One pass of a polynomial update.
fn apply_update<V: ValueAlgebra>(terms: &[Vec<Term<V>>], x: &[V]) -> Result<Vec<V>, String> {
    terms.iter().map(|component| eval_terms(component, x)).collect()
}

/// `N` passes of `x ← x + dt·f(x) + O(dt²)`, `f` polynomial with rational coefficients. A finite
/// `N` runs the update itself. An infinite one, `N = T/dt + δ` on each residue class, ends
/// infinitely close to the solution of `x' = f(x)` from the start's standard part at time `T`, a
/// germ known by its standard part; a scalar solution certified to leave every bound before `T`
/// leaves an unlimited germ.
pub fn polynomial_flow<V: ValueAlgebra>(field: &V, vector: &V, n: &V) -> Result<V, String> {
    let x = items(vector, "a supertask's state")?;
    let terms = update_terms(field, x.len())?;
    let natural = match n.shape() {
        Shape::Int(k) if k >= 0 => {
            let mut state = x;
            for _ in 0..k {
                state = apply_update(&terms, &state)?;
            }
            return Ok(V::list(state));
        }
        _ => infinite_count(n)?,
    };
    let width = x.len();
    on_one_infinitesimal(terms.iter().flatten().map(|t| &t.coefficient), &natural.scale, "update", "a nonlinear flow is integrated over rates that are standard numbers")?;
    on_one_infinitesimal(x.iter(), &natural.scale, "state", "a nonlinear flow is integrated from a start whose standard part is a number")?;
    let mut components = Vec::with_capacity(width);
    for (i, component) in terms.iter().enumerate() {
        let mut monomials = Vec::new();
        let mut identity = false;
        for term in component {
            let (q, series) = dt_polynomial(&term.coefficient, &natural.scale).ok_or_else(|| {
                format!(
                    "the update's coefficient {} is not a polynomial in the infinitesimal, or in a root of it, with rational coefficients, so this supertask is not a flow the engine integrates",
                    term.coefficient.display_string()
                )
            })?;
            if below_dt(q, &series) {
                return Err(format!(
                    "the update's coefficient {} has a term of a fractional order of the infinitesimal below its first, so this supertask is not a flow",
                    term.coefficient.display_string()
                ));
            }
            let at_zero = series.first().cloned().unwrap_or_else(Rational::zero);
            let own = term.exponents.iter().enumerate().all(|(j, e)| *e == u32::from(j == i));
            if own && at_zero == Rational::one() {
                identity = true;
            } else if !at_zero.is_zero() {
                return Err(
                    "the update changes its state by a standard amount at every step as well as an infinitesimal one, so this supertask is neither a cycle nor a flow"
                        .to_string(),
                );
            }
            let rate = series.get(q as usize).cloned().unwrap_or_else(Rational::zero);
            if !rate.is_zero() {
                monomials.push(Monomial {
                    coefficient: rate,
                    powers: term.exponents.clone(),
                });
            }
        }
        if !identity {
            return Err(
                "the update does not keep its state when the infinitesimal is zero, so this supertask is not a flow"
                    .to_string(),
            );
        }
        components.push(monomials);
    }
    let ode = OdeField { components };
    let scale = &natural.scale;
    let ty = scale.residues.clone();
    let counts = natural.value.depends();
    let l = classes([counts.modulus(), ty.modulus()].into_iter().chain(state_moduli(&x)))
        .ok_or_else(|| "a flow's step count and state split over more residue classes than one value holds".to_string())?;
    let mut columns: Vec<Vec<Option<Branch<V>>>> = vec![Vec::with_capacity(l as usize); width];
    for c in 0..l {
        let Some(count) = counts.at(c) else {
            columns.iter_mut().for_each(|col| col.push(None));
            continue;
        };
        if count.degree().unwrap_or(0) != 1 {
            return Err(
                "a flow's step count is a multiple of 1/dt; one that grows faster or slower has no standard end state here"
                    .to_string(),
            );
        }
        let t = count.coeff(1);
        if !t.is_positive() {
            return Err("a flow runs forward a positive time".to_string());
        }
        let start = x
            .iter()
            .map(|v| {
                let at = at_class(v, c);
                tower::germ_of(&at)
                    .and_then(|g| g.standard_part().ok().flatten())
                    .ok_or_else(|| {
                        format!("a flow starts from limited numbers, not {}", at.display_string())
                    })
            })
            .collect::<Result<Vec<ExactReal>, String>>()?;
        let ends = flow_end(&ode, &start, &t)?;
        for (column, end) in columns.iter_mut().zip(ends) {
            column.push(Some(Branch(tower::from_germ(end, scale))));
        }
    }
    Ok(V::list(
        columns
            .into_iter()
            .map(|raw| {
                let d = Depends::from_branches(ty.clone(), l, raw).expect("the classes are the type's own");
                tower::from_depends(d, scale)
            })
            .collect(),
    ))
}

/// The germs a flow of `x' = f(x)` from `start` leaves at time `t`: each known by its standard
/// part, or, for a scalar solution certified to blow up before `t`, unlimited with its sign.
fn flow_end(ode: &OdeField, start: &[ExactReal], t: &Rational) -> Result<Vec<Germ>, String> {
    let ends = ExactReal::flow(ode, start, t)
        .ok_or_else(|| "a flow's update and state disagree in shape".to_string())?;
    match ends[0].enclosure(8) {
        Ok(_) => Ok(ends.iter().map(Germ::standard).collect()),
        Err(RealError::Domain(_)) => {
            let escape = |p: &Poly, x0: &Rational| {
                escape_time_bound(p, x0, 24).is_some_and(|bound| bound < *t)
            };
            let scalar = (ode.components.len() == 1).then(|| {
                let mut coefficients = Vec::new();
                for m in &ode.components[0] {
                    let k = m.powers[0] as usize;
                    if coefficients.len() <= k {
                        coefficients.resize(k + 1, Rational::zero());
                    }
                    coefficients[k] = coefficients[k].add(&m.coefficient);
                }
                Poly::from_coeffs(coefficients)
            });
            let x0 = start[0].exact_algebraic().and_then(|a| a.to_rational());
            match (scalar, x0) {
                (Some(p), Some(x0)) if escape(&p, &x0) => Ok(vec![Germ::unlimited(1)]),
                (Some(p), Some(x0)) if escape(&p.neg_var().neg(), &x0.negated()) => {
                    Ok(vec![Germ::unlimited(-1)])
                }
                _ => Err(format!(
                    "the solution of the flow leaves every bound before time {t}, and no certificate says it blows up there"
                )),
            }
        }
        Err(e) => Err(e.to_string()),
    }
}

/// `Σ_j C(q, j)·Δʲ_r` on every residue class of the step count `N`, with `r = (N − z) mod p` and
/// `q = (N − z − r)/p` there.
fn evaluate_by_class<V: ValueAlgebra>(
    natural: &NaturalValue,
    z: u64,
    p: u64,
    differences: &[Vec<Vec<V>>],
    width: usize,
) -> Result<Vec<V>, String> {
    let scale = &natural.scale;
    let refuse = |e: ResidueError| e.describe(&scale.generator_text());
    let ty = scale.residues.clone();
    let shifted = natural
        .value
        .sub(&Hypernatural::from_integer(BigInt::from_u64(z), &ty))
        .map_err(refuse)?;
    let residues = shifted.rem(&BigInt::from_u64(p)).map_err(refuse)?;
    let l = classes([residues.modulus(), shifted.depends().modulus(), ty.modulus()])
        .ok_or_else(|| "a supertask's step count splits over more residue classes than one value holds".to_string())?;
    let mut columns: Vec<Vec<Option<Branch<V>>>> = vec![Vec::with_capacity(l as usize); width];
    for c in 0..l {
        let (Some(r), Some(poly)) = (residues.at(c), shifted.depends().at(c)) else {
            for column in &mut columns {
                column.push(None);
            }
            continue;
        };
        let r_value = r.to_u64().expect("a residue below p");
        let q = poly
            .sub(&Poly::constant(Rational::from_bigint(r.clone())))
            .scale(&Rational::new(BigInt::from_u64(1), BigInt::from_u64(p)).expect("p > 0"));
        let q: V = tower::from_hyper(Hyperreal::from_reciprocal_powers(q.coeffs()), scale);
        let deltas = &differences[r_value as usize];
        let mut binomial: V = int(1);
        let mut totals: Vec<V> = vec![int(0); width];
        for (j, delta) in deltas.iter().enumerate() {
            if j > 0 {
                let factor = V::subtract(q.clone(), int(j as i64 - 1))?;
                binomial = V::exact_divide(V::multiply(binomial, factor)?, int(j as i64))?;
            }
            for (total, d) in totals.iter_mut().zip(delta) {
                *total = V::add(total.clone(), V::multiply(binomial.clone(), d.clone())?)?;
            }
        }
        for (column, total) in columns.iter_mut().zip(totals) {
            column.push(Some(Branch(total)));
        }
    }
    Ok(columns
        .into_iter()
        .map(|raw| {
            let d = Depends::from_branches(ty.clone(), l, raw).expect("the classes are the type's own");
            tower::from_depends(d, scale)
        })
        .collect())
}

// ── hybrid flows ─────────────────────────────────────────────────────────────────────────────

/// A condition of a hybrid loop as the engine encodes it, its coefficients still the program's
/// values.
enum Encoded<V> {
    All(Vec<Encoded<V>>),
    Any(Vec<Encoded<V>>),
    Not(Box<Encoded<V>>),
    Compare(hybrid::Rel, Vec<Term<V>>),
    Infinitesimal(Vec<Term<V>>),
    Limited(Vec<Term<V>>),
    Truth(usize),
    Constant(bool),
}

enum EncodedStmt<V> {
    Set(usize, Vec<Term<V>>),
    Put(usize, V),
    If(Encoded<V>, Vec<EncodedStmt<V>>, Vec<EncodedStmt<V>>),
}

struct EncodedJump<V> {
    guard: Encoded<V>,
    block: Vec<EncodedStmt<V>>,
}

/// A part of a hybrid loop's body: a run of flow statements as one polynomial update, or the
/// check of a jump.
enum EncodedItem<V> {
    Flow(Vec<Vec<Term<V>>>),
    Check(usize),
}

/// A list's kind, named by the Text it starts with, and the rest of its items.
fn tagged<V: ValueAlgebra>(v: &V, what: &str) -> Result<(String, Vec<V>), String> {
    let mut parts = items(v, what)?;
    if parts.is_empty() {
        return Err(format!("{what} is empty"));
    }
    let head = parts.remove(0);
    let tag = head
        .text()
        .ok_or_else(|| format!("{what} starts with its kind, named by Text"))?
        .to_string();
    Ok((tag, parts))
}

fn slot_of<V: ValueAlgebra>(v: &V, width: usize) -> Result<usize, String> {
    match v.shape() {
        Shape::Int(k) if k >= 0 && (k as usize) < width => Ok(k as usize),
        _ => Err(format!(
            "a hybrid loop names a value by its place among its {width}, got {}",
            v.display_string()
        )),
    }
}

fn encoded_cond<V: ValueAlgebra>(v: &V, width: usize) -> Result<Encoded<V>, String> {
    let (tag, parts) = tagged(v, "a hybrid loop's condition")?;
    let many = |parts: &[V]| -> Result<Vec<Encoded<V>>, String> { parts.iter().map(|p| encoded_cond(p, width)).collect() };
    fn one<'p, V>(parts: &'p [V], tag: &str) -> Result<&'p V, String> {
        match parts {
            [p] => Ok(p),
            _ => Err(format!("a hybrid loop's `{tag}` condition has one operand")),
        }
    }
    Ok(match tag.as_str() {
        "all" => Encoded::All(many(&parts)?),
        "any" => Encoded::Any(many(&parts)?),
        "not" => Encoded::Not(Box::new(encoded_cond(one(&parts, &tag)?, width)?)),
        "compare" => match parts.as_slice() {
            [rel, poly] => {
                let rel = rel
                    .text()
                    .and_then(hybrid::Rel::named)
                    .ok_or_else(|| "a hybrid loop's comparison names its relation".to_string())?;
                Encoded::Compare(rel, poly_terms(poly, width)?)
            }
            _ => return Err("a hybrid loop's comparison is a relation and a polynomial".to_string()),
        },
        "infinitesimal" => Encoded::Infinitesimal(poly_terms(one(&parts, &tag)?, width)?),
        "limited" => Encoded::Limited(poly_terms(one(&parts, &tag)?, width)?),
        "truth" => Encoded::Truth(slot_of(one(&parts, &tag)?, width)?),
        "constant" => Encoded::Constant(truth_of(one(&parts, &tag)?)?),
        other => return Err(format!("a hybrid loop has no condition of kind `{other}`")),
    })
}

fn encoded_block<V: ValueAlgebra>(v: &V, width: usize) -> Result<Vec<EncodedStmt<V>>, String> {
    items(v, "a hybrid loop's jump")?
        .iter()
        .map(|stmt| {
            let (tag, parts) = tagged(stmt, "a hybrid loop's statement")?;
            Ok(match (tag.as_str(), parts.as_slice()) {
                ("set", [slot, poly]) => EncodedStmt::Set(slot_of(slot, width)?, poly_terms(poly, width)?),
                ("put", [slot, value]) => EncodedStmt::Put(slot_of(slot, width)?, value.clone()),
                ("if", [cond, then, otherwise]) => EncodedStmt::If(
                    encoded_cond(cond, width)?,
                    encoded_block(then, width)?,
                    encoded_block(otherwise, width)?,
                ),
                _ => return Err(format!("a hybrid loop has no statement of kind `{tag}` with {} parts", parts.len())),
            })
        })
        .collect()
}

fn encoded_jumps<V: ValueAlgebra>(v: &V, width: usize) -> Result<Vec<EncodedJump<V>>, String> {
    items(v, "a hybrid loop's jumps")?
        .iter()
        .map(|jump| match items(jump, "a hybrid loop's jump")?.as_slice() {
            [guard, block] => Ok(EncodedJump {
                guard: encoded_cond(guard, width)?,
                block: encoded_block(block, width)?,
            }),
            _ => Err("a hybrid loop's jump is a guard and a block".to_string()),
        })
        .collect()
}

fn encoded_schedule<V: ValueAlgebra>(v: &V, width: usize, jumps: usize) -> Result<Vec<EncodedItem<V>>, String> {
    let schedule = items(v, "a hybrid loop's body")?
        .iter()
        .map(|item| {
            let (tag, parts) = tagged(item, "a part of a hybrid loop's body")?;
            match (tag.as_str(), parts.as_slice()) {
                ("flow", [field]) => Ok(EncodedItem::Flow(update_terms(field, width)?)),
                ("check", [k]) => match k.shape() {
                    Shape::Int(k) if k >= 0 && (k as usize) < jumps => Ok(EncodedItem::Check(k as usize)),
                    _ => Err(format!("a hybrid loop checks one of its {jumps} jumps, got {}", k.display_string())),
                },
                _ => Err(format!("a part of a hybrid loop's body is a `flow` or a `check`, not `{tag}`")),
            }
        })
        .collect::<Result<Vec<_>, String>>()?;
    for k in 0..jumps {
        if schedule.iter().filter(|item| matches!(item, EncodedItem::Check(j) if *j == k)).count() != 1 {
            return Err("a hybrid loop's body checks each of its jumps once".to_string());
        }
    }
    Ok(schedule)
}

fn truth_of<V: ValueAlgebra>(v: &V) -> Result<bool, String> {
    match v.shape() {
        Shape::Bool => Ok(v.values_equal(&V::boolean(true))),
        _ => Err(format!("a hybrid loop's condition reads a truth value, got {}", v.type_name())),
    }
}

fn tower_rel(rel: hybrid::Rel) -> Rel {
    match rel {
        hybrid::Rel::Lt => Rel::Lt,
        hybrid::Rel::Le => Rel::LtEq,
        hybrid::Rel::Gt => Rel::Gt,
        hybrid::Rel::Ge => Rel::GtEq,
        hybrid::Rel::Eq => Rel::Eq,
        hybrid::Rel::Ne => Rel::NotEq,
    }
}

/// One run of flow statements, a component that only keeps its own value keeping it as it is —
/// a truth value or text among the state is never multiplied by one.
fn literal_update<V: ValueAlgebra>(field: &[Vec<Term<V>>], x: &[V]) -> Result<Vec<V>, String> {
    field
        .iter()
        .enumerate()
        .map(|(i, component)| match component.as_slice() {
            [only]
                if only.exponents.iter().enumerate().all(|(j, e)| *e == u32::from(j == i))
                    && only.coefficient.integer().is_some_and(|n| n == BigInt::from_i64(1)) =>
            {
                Ok(x[i].clone())
            }
            _ => eval_terms(component, x),
        })
        .collect()
}

fn literal_cond<V: ValueAlgebra>(c: &Encoded<V>, x: &[V]) -> Result<bool, String> {
    Ok(match c {
        Encoded::All(cs) => {
            for c in cs {
                if !literal_cond(c, x)? {
                    return Ok(false);
                }
            }
            true
        }
        Encoded::Any(cs) => {
            for c in cs {
                if literal_cond(c, x)? {
                    return Ok(true);
                }
            }
            false
        }
        Encoded::Not(c) => !literal_cond(c, x)?,
        Encoded::Compare(rel, p) => V::holds(tower_rel(*rel), &eval_terms(p, x)?, &int(0))?,
        Encoded::Infinitesimal(p) => truth_of(&tower::is_infinitesimal(&eval_terms(p, x)?)?)?,
        Encoded::Limited(p) => truth_of(&tower::is_limited(&eval_terms(p, x)?)?)?,
        Encoded::Truth(slot) => truth_of(&x[*slot])?,
        Encoded::Constant(b) => *b,
    })
}

fn literal_exec<V: ValueAlgebra>(stmts: &[EncodedStmt<V>], x: &mut [V]) -> Result<(), String> {
    for s in stmts {
        match s {
            EncodedStmt::Set(slot, p) => x[*slot] = eval_terms(p, x)?,
            EncodedStmt::Put(slot, v) => x[*slot] = v.clone(),
            EncodedStmt::If(c, then, otherwise) => {
                if literal_cond(c, x)? {
                    literal_exec(then, x)?;
                } else {
                    literal_exec(otherwise, x)?;
                }
            }
        }
    }
    Ok(())
}

/// The standard part of a coefficient or a value as an algebraic number, and whether it is that
/// number exactly.
fn standard_value<V: ValueAlgebra>(v: &V) -> Result<(Algebraic, bool), String> {
    if let Some(r) = v.standard_rational() {
        return Ok((Algebraic::from_rational(r), true));
    }
    if let Some(ExactRef::Algebraic(a)) = v.exact() {
        return Ok((a.clone(), true));
    }
    let germ = tower::germ_of(v)
        .ok_or_else(|| format!("a hybrid loop computes with numbers, not {}", v.type_name()))?;
    let standard = germ
        .standard_part()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("a hybrid loop starts from limited numbers, not {}", v.display_string()))?;
    let a = standard.exact_algebraic().ok_or_else(|| {
        format!(
            "{} has no algebraic standard part, so the times a hybrid loop's jumps fire at are not found exactly",
            v.display_string()
        )
    })?;
    let exact = is_standard(v) || germ.as_hyperreal().is_some_and(|h| h.as_standard().is_some());
    Ok((a, exact))
}

/// The coefficients of `dt⁰` and `dt¹` in a coefficient of an Euler step, which has no term of a
/// fractional order between them.
fn step_coefficients<V: ValueAlgebra>(v: &V) -> Result<(Algebraic, Algebraic), String> {
    if let Some(r) = v.standard_rational() {
        return Ok((Algebraic::from_rational(r), Algebraic::from_i64(0)));
    }
    let germ = tower::germ_of(v)
        .ok_or_else(|| format!("a hybrid loop's flow computes with numbers, not {}", v.type_name()))?;
    let order = |k: i64| -> Result<Algebraic, String> {
        germ.coefficient(k)
            .map_err(|e| e.to_string())?
            .exact_algebraic()
            .ok_or_else(|| format!("the flow's coefficient {} is not algebraic", v.display_string()))
    };
    if germ.floor() < 0 && (germ.floor()..0).any(|k| order(k).map_or(true, |c| !c.is_zero())) {
        return Err(format!("the flow's coefficient {} is not limited", v.display_string()));
    }
    let dt = i64::from(germ.ramification());
    if (1..dt).any(|k| order(k).map_or(true, |c| !c.is_zero())) {
        return Err(format!(
            "the flow's coefficient {} has a term of a fractional order of the infinitesimal below its first, so the loop is not an Euler step",
            v.display_string()
        ));
    }
    Ok((order(0)?, order(dt)?))
}

fn ideal_poly<V: ValueAlgebra>(terms: &[Term<V>], width: usize, class: u64) -> Result<hybrid::Term, String> {
    let mut out = Vec::with_capacity(terms.len());
    let mut exact = true;
    for t in terms {
        let (a, is_exact) = standard_value(&at_class(&t.coefficient, class))?;
        exact &= is_exact;
        out.push((t.exponents.clone(), a));
    }
    Ok(hybrid::Term {
        poly: hybrid::MPoly::from_terms(width, out),
        exact,
    })
}

fn ideal_cond<V: ValueAlgebra>(c: &Encoded<V>, width: usize, class: u64) -> Result<hybrid::Cond, String> {
    Ok(match c {
        Encoded::All(cs) => hybrid::Cond::All(cs.iter().map(|c| ideal_cond(c, width, class)).collect::<Result<_, _>>()?),
        Encoded::Any(cs) => hybrid::Cond::Any(cs.iter().map(|c| ideal_cond(c, width, class)).collect::<Result<_, _>>()?),
        Encoded::Not(c) => hybrid::Cond::Not(Box::new(ideal_cond(c, width, class)?)),
        Encoded::Compare(rel, p) => hybrid::Cond::Compare(*rel, ideal_poly(p, width, class)?),
        Encoded::Infinitesimal(p) => hybrid::Cond::Infinitesimal(ideal_poly(p, width, class)?),
        Encoded::Limited(p) => hybrid::Cond::Limited(ideal_poly(p, width, class)?),
        Encoded::Truth(slot) => hybrid::Cond::Truth(*slot),
        Encoded::Constant(b) => hybrid::Cond::Constant(*b),
    })
}

/// A value a slot holds, as the ideal run holds it; a value no polynomial reads is kept in
/// `others` and named by its place there.
fn ideal_cell<V: ValueAlgebra>(v: &V, others: &mut Vec<V>) -> Result<hybrid::Cell, String> {
    if matches!(v.shape(), Shape::Bool) {
        return Ok(hybrid::Cell::Bool(truth_of(v)?));
    }
    if is_number(v) {
        let (value, exact) = standard_value(v)?;
        return Ok(hybrid::Cell::Num { value, exact });
    }
    others.push(v.clone());
    Ok(hybrid::Cell::Other(others.len() - 1))
}

fn ideal_block<V: ValueAlgebra>(
    stmts: &[EncodedStmt<V>],
    width: usize,
    class: u64,
    others: &mut Vec<V>,
) -> Result<Vec<hybrid::Stmt>, String> {
    stmts
        .iter()
        .map(|s| {
            Ok(match s {
                EncodedStmt::Set(slot, p) => hybrid::Stmt::Set {
                    slot: *slot,
                    value: hybrid::Value::Poly(ideal_poly(p, width, class)?),
                },
                EncodedStmt::Put(slot, v) => hybrid::Stmt::Set {
                    slot: *slot,
                    value: match ideal_cell(&at_class(v, class), others)? {
                        hybrid::Cell::Num { value, exact } => hybrid::Value::Poly(hybrid::Term {
                            poly: hybrid::MPoly::constant(width, value),
                            exact,
                        }),
                        cell => hybrid::Value::Fixed(cell),
                    },
                },
                EncodedStmt::If(c, then, otherwise) => hybrid::Stmt::If {
                    cond: ideal_cond(c, width, class)?,
                    then: ideal_block(then, width, class, others)?,
                    otherwise: ideal_block(otherwise, width, class, others)?,
                },
            })
        })
        .collect()
}

/// The rates and the exactly unchanged slots of a run of flow statements, read on residue class
/// `class`: each component keeps its own value at `dt = 0`, and moves by the coefficient of `dt`.
fn segment<V: ValueAlgebra>(field: &[Vec<Term<V>>], class: u64) -> Result<(Vec<hybrid::MPoly>, Vec<bool>), String> {
    let width = field.len();
    let mut rates = Vec::with_capacity(width);
    let mut preserved = Vec::with_capacity(width);
    for (i, component) in field.iter().enumerate() {
        let own = |t: &Term<V>| t.exponents.iter().enumerate().all(|(j, e)| *e == u32::from(j == i));
        let mut terms = Vec::new();
        let mut identity = false;
        for term in component {
            let (at_zero, rate) = step_coefficients(&at_class(&term.coefficient, class))?;
            if own(term) && at_zero == Algebraic::from_i64(1) {
                identity = true;
            } else if !at_zero.is_zero() {
                return Err(
                    "a hybrid loop's flow changes a value by a standard amount at every step, so it is not an Euler step"
                        .to_string(),
                );
            }
            if !rate.is_zero() {
                terms.push((term.exponents.clone(), rate));
            }
        }
        if !identity {
            return Err("a hybrid loop's flow does not keep its state when the infinitesimal is zero".to_string());
        }
        preserved.push(match component.as_slice() {
            [only] => {
                let c = at_class(&only.coefficient, class);
                own(only) && is_standard(&c) && as_rational(&c) == Some(Rational::one())
            }
            _ => false,
        });
        rates.push(hybrid::MPoly::from_terms(width, terms));
    }
    Ok((rates, preserved))
}

/// The slots a floating-point value reaches: those that start as one, and those a statement sets
/// from a floating-point coefficient or from a slot it reaches.
fn float_reach<V: ValueAlgebra>(jumps: &[EncodedJump<V>], start: &[V]) -> Vec<bool> {
    fn walk<V: ValueAlgebra>(stmts: &[EncodedStmt<V>], reach: &mut [bool]) -> bool {
        let mut grown = false;
        for s in stmts {
            match s {
                EncodedStmt::Set(slot, p) => {
                    let floats = p.iter().any(|t| {
                        matches!(t.coefficient.shape(), Shape::Float)
                            || t.exponents.iter().enumerate().any(|(i, e)| *e > 0 && reach[i])
                    });
                    if floats && !reach[*slot] {
                        reach[*slot] = true;
                        grown = true;
                    }
                }
                EncodedStmt::Put(slot, v) => {
                    if matches!(v.shape(), Shape::Float) && !reach[*slot] {
                        reach[*slot] = true;
                        grown = true;
                    }
                }
                EncodedStmt::If(_, then, otherwise) => {
                    grown |= walk(then, reach);
                    grown |= walk(otherwise, reach);
                }
            }
        }
        grown
    }
    let mut reach: Vec<bool> = start.iter().map(|v| matches!(v.shape(), Shape::Float)).collect();
    while jumps.iter().any(|j| walk(&j.block, &mut reach)) {}
    reach
}

/// `N` steps of a loop whose body is polynomial Euler steps and jumps, as `model` encodes them:
/// `[body, jumps]`, the body its parts in order — `["flow", field]` a run of flow statements as one
/// polynomial update per slot, as for [`polynomial_flow`], `["check", k]` the check of jump `k` —
/// and each jump `[guard, block]`. A finite `N` runs the steps themselves. An infinite one,
/// `N = T/dt + δ` on each residue class, ends on the values of the ideal hybrid system the loop
/// follows for time `T` (see [`crate::hybrid`]): a value the flow moves is a germ known by its
/// standard part, a value the loop holds exactly is that value.
pub fn hybrid_flow<V: ValueAlgebra>(model: &V, vector: &V, n: &V) -> Result<V, String> {
    let x = items(vector, "a hybrid loop's state")?;
    let width = x.len();
    let parts = items(model, "a hybrid loop's model")?;
    let [body, jumps] = parts.as_slice() else {
        return Err("a hybrid loop's model is its body and its jumps".to_string());
    };
    let jumps = encoded_jumps(jumps, width)?;
    let schedule = encoded_schedule(body, width, jumps.len())?;
    let natural = match n.shape() {
        Shape::Int(k) if k >= 0 => {
            let mut state = x;
            for _ in 0..k {
                for item in &schedule {
                    match item {
                        EncodedItem::Flow(field) => state = literal_update(field, &state)?,
                        EncodedItem::Check(j) => {
                            if literal_cond(&jumps[*j].guard, &state)? {
                                literal_exec(&jumps[*j].block, &mut state)?;
                            }
                        }
                    }
                }
            }
            return Ok(V::list(state));
        }
        _ => infinite_count(n)?,
    };
    on_one_infinitesimal([model, vector].into_iter(), &natural.scale, "loop", "a hybrid loop's rates and jumps are standard numbers")?;
    let floats = float_reach(&jumps, &x);
    let scale = &natural.scale;
    let ty = scale.residues.clone();
    let counts = natural.value.depends();
    let coefficient_moduli: Vec<V> = schedule
        .iter()
        .flat_map(|item| match item {
            EncodedItem::Flow(field) => field.iter().flatten().map(|t| t.coefficient.clone()).collect(),
            EncodedItem::Check(_) => Vec::new(),
        })
        .collect();
    let l = classes(
        [counts.modulus(), ty.modulus()]
            .into_iter()
            .chain(state_moduli(&x))
            .chain(state_moduli(&coefficient_moduli)),
    )
    .ok_or_else(|| "a hybrid loop's step count and state split over more residue classes than one value holds".to_string())?;
    let mut columns: Vec<Vec<Option<Branch<V>>>> = vec![Vec::with_capacity(l as usize); width];
    for c in 0..l {
        let Some(count) = counts.at(c) else {
            columns.iter_mut().for_each(|col| col.push(None));
            continue;
        };
        if count.degree().unwrap_or(0) != 1 {
            return Err(
                "a hybrid loop's step count is a multiple of 1/dt; one that grows faster or slower has no standard end state here"
                    .to_string(),
            );
        }
        let span = count.coeff(1);
        if !span.is_positive() {
            return Err("a hybrid loop runs forward a positive time".to_string());
        }
        let items = schedule
            .iter()
            .map(|item| {
                Ok(match item {
                    EncodedItem::Flow(field) => {
                        let (rates, preserved) = segment(field, c)?;
                        hybrid::Item::Flow { rates, preserved }
                    }
                    EncodedItem::Check(k) => hybrid::Item::Check(*k),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut others = Vec::new();
        let start = x
            .iter()
            .map(|v| ideal_cell(&at_class(v, c), &mut others))
            .collect::<Result<Vec<_>, String>>()?;
        let ideal_jumps = jumps
            .iter()
            .map(|j| {
                Ok(hybrid::Jump {
                    guard: ideal_cond(&j.guard, width, c)?,
                    block: ideal_block(&j.block, width, c, &mut others)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let model = hybrid::Model::new(width, items, ideal_jumps);
        let end = hybrid::run(&model, start.clone(), &Algebraic::from_rational(span))?;
        for (i, (column, cell)) in columns.iter_mut().zip(end).enumerate() {
            let value = match cell {
                hybrid::Cell::Num { value, exact: true } => {
                    if floats[i] {
                        match &start[i] {
                            hybrid::Cell::Num { value: was, .. } if *was == value => at_class(&x[i], c),
                            _ => {
                                return Err(
                                    "a hybrid loop computes exactly, and a value its jumps set is a floating-point number, whose arithmetic rounds; declare it Rational or Real"
                                        .to_string(),
                                )
                            }
                        }
                    } else {
                        tower::from_algebraic(value)
                    }
                }
                hybrid::Cell::Num { value, exact: false } => {
                    tower::from_germ(Germ::standard(&ExactReal::from_algebraic(value)), scale)
                }
                hybrid::Cell::Bool(b) => V::boolean(b),
                hybrid::Cell::Other(k) => others[k].clone(),
            };
            column.push(Some(Branch(value)));
        }
    }
    Ok(V::list(
        columns
            .into_iter()
            .map(|raw| {
                let d = Depends::from_branches(ty.clone(), l, raw).expect("the classes are the type's own");
                tower::from_depends(d, scale)
            })
            .collect(),
    ))
}

// ── cycles ────────────────────────────────────────────────────────────────────────────────────

/// The state at step `n` of a run recorded as `states` (the state before each step) up to the step
/// whose state, `repeated`, had been seen before: the run is eventually periodic from the first
/// occurrence of `repeated`.
pub fn cycle_at<V: ValueAlgebra>(states: &V, repeated: &V, n: &V) -> Result<V, String> {
    let states = items(states, "a supertask's recorded states")?;
    let mu = states
        .iter()
        .position(|s| s.values_equal(repeated))
        .ok_or_else(|| "the supertask engine's repeated state is not among the recorded ones".to_string())?;
    let lambda = (states.len() - mu) as u64;
    let state_at = |i: usize| items(&states[i], "a recorded state");
    if let Shape::Int(k) = n.shape() {
        if k >= 0 {
            let k = k as u64;
            let i = if k < states.len() as u64 {
                k
            } else {
                mu as u64 + (k - mu as u64) % lambda
            };
            return Ok(V::list(state_at(i as usize)?));
        }
    }
    if several_count(n) {
        let residue = V::modulo(V::subtract(n.clone(), int(mu as i64))?, int(lambda as i64))?;
        let width = state_at(mu)?.len();
        return Ok(V::list(
            (0..width)
                .map(|i| on_residue(&residue, &|r| Ok(state_at(mu + r as usize)?[i].clone())))
                .collect::<Result<_, _>>()?,
        ));
    }
    let nv = infinite_count(n)?;
    let scale = &nv.scale;
    let refuse = |e: ResidueError| e.describe(&scale.generator_text());
    let ty = scale.residues.clone();
    let shifted = nv
        .value
        .sub(&Hypernatural::from_integer(BigInt::from_u64(mu as u64), &ty))
        .map_err(refuse)?;
    let residues = shifted.rem(&BigInt::from_u64(lambda)).map_err(refuse)?;
    let width = state_at(mu)?.len();
    let mut columns: Vec<Vec<Option<Branch<V>>>> = vec![Vec::new(); width];
    for c in 0..residues.modulus() {
        match residues.at(c) {
            None => columns.iter_mut().for_each(|col| col.push(None)),
            Some(r) => {
                let state = state_at(mu + r.to_u64().expect("a residue below λ") as usize)?;
                for (column, value) in columns.iter_mut().zip(state) {
                    column.push(Some(Branch(value)));
                }
            }
        }
    }
    Ok(V::list(
        columns
            .into_iter()
            .map(|raw| {
                let d = Depends::from_branches(ty.clone(), residues.modulus(), raw)
                    .expect("the classes are the type's own");
                tower::from_depends(d, scale)
            })
            .collect(),
    ))
}

// ── state sizes ───────────────────────────────────────────────────────────────────────────────

/// The bits a state occupies — a number by its magnitude, text by its length, a list or tuple by
/// its items, an exact value by its canonical encoding — the measure the cycle search stops a
/// growing state by.
pub fn state_size<V: ValueAlgebra>(v: &V) -> u64 {
    match v.shape() {
        Shape::Bool => 1,
        Shape::Int(_) | Shape::Float => 64,
        Shape::BigInt(b) => b.bit_len().max(64),
        Shape::Rational(r) => r.numerator().bit_len() + r.denominator().bit_len(),
        Shape::Text(len) => 8 * len as u64,
        Shape::List(values) | Shape::Tuple(values) => values.iter().map(state_size).sum(),
        Shape::Decimal | Shape::Other => match tower::exact_bytes(v) {
            Some((_, bytes)) => 8 * bytes.len() as u64,
            None => 8 * v.display_string().len() as u64,
        },
    }
}

// ── first exits ───────────────────────────────────────────────────────────────────────────────

/// A loop's guard as the supertask engine writes it: `and`, `or` and `not` over atoms linear in the
/// loop's counter `t`.
enum Guard<V> {
    /// `a·t + b <rel> 0`.
    Linear { rel: Rel, a: V, b: V },
    /// `(a·t + b) % m <rel> c`.
    Remainder { rel: Rel, a: V, b: V, m: V, c: V },
    And(Box<Guard<V>>, Box<Guard<V>>),
    Or(Box<Guard<V>>, Box<Guard<V>>),
    Not(Box<Guard<V>>),
}

/// A guard's answer at one value of its counter: whether it holds, or the finer residue classes a
/// remainder needs before it can say.
enum Truth {
    Holds(bool),
    Finer(u64),
}

fn relation_named(name: &str) -> Option<Rel> {
    Some(match name {
        "<" => Rel::Lt,
        "<=" => Rel::LtEq,
        ">" => Rel::Gt,
        ">=" => Rel::GtEq,
        "=" => Rel::Eq,
        "!=" => Rel::NotEq,
        _ => return None,
    })
}

impl<V: ValueAlgebra> Guard<V> {
    /// The formula `[tag, …]` the engine writes: `["and", f, g]`, `["or", f, g]`, `["not", f]`,
    /// `[relation, a, b]` and `["%", relation, a, b, m, c]`.
    fn read(v: &V) -> Result<Guard<V>, String> {
        let malformed = || format!("a loop's guard is a formula the supertask engine writes, got {}", v.display_string());
        let parts = items(v, "a loop's guard")?;
        let tag = parts.first().and_then(|t| t.text().map(str::to_string));
        let sub = |i: usize| match parts.get(i) {
            Some(p) => Guard::read(p).map(Box::new),
            None => Err(malformed()),
        };
        let at = |i: usize| parts.get(i).cloned().ok_or_else(malformed);
        let relation = |v: V| v.text().and_then(relation_named).ok_or_else(malformed);
        match (tag.as_deref(), parts.len()) {
            (Some("and"), 3) => Ok(Guard::And(sub(1)?, sub(2)?)),
            (Some("or"), 3) => Ok(Guard::Or(sub(1)?, sub(2)?)),
            (Some("not"), 2) => Ok(Guard::Not(sub(1)?)),
            (Some("%"), 6) => Ok(Guard::Remainder {
                rel: relation(at(1)?)?,
                a: at(2)?,
                b: at(3)?,
                m: at(4)?,
                c: at(5)?,
            }),
            (Some(_), 3) => Ok(Guard::Linear {
                rel: relation(at(0)?)?,
                a: at(1)?,
                b: at(2)?,
            }),
            _ => Err(malformed()),
        }
    }

    /// Every number the guard reads.
    fn numbers<'g>(&'g self, out: &mut Vec<&'g V>) {
        match self {
            Guard::Linear { a, b, .. } => out.extend([a, b]),
            Guard::Remainder { a, b, m, c, .. } => out.extend([a, b, m, c]),
            Guard::And(f, g) | Guard::Or(f, g) => {
                f.numbers(out);
                g.numbers(out);
            }
            Guard::Not(f) => f.numbers(out),
        }
    }

    /// The linear terms and remainders the guard is built from.
    fn atoms<'g>(&'g self, out: &mut Vec<&'g Guard<V>>) {
        match self {
            Guard::Linear { .. } | Guard::Remainder { .. } => out.push(self),
            Guard::And(f, g) | Guard::Or(f, g) => {
                f.atoms(out);
                g.atoms(out);
            }
            Guard::Not(f) => f.atoms(out),
        }
    }

    /// The same guard with every number passed through `f`.
    fn map(&self, f: &dyn Fn(&V) -> V) -> Guard<V> {
        match self {
            Guard::Linear { rel, a, b } => Guard::Linear {
                rel: *rel,
                a: f(a),
                b: f(b),
            },
            Guard::Remainder { rel, a, b, m, c } => Guard::Remainder {
                rel: *rel,
                a: f(a),
                b: f(b),
                m: f(m),
                c: f(c),
            },
            Guard::And(x, y) => Guard::And(Box::new(x.map(f)), Box::new(y.map(f))),
            Guard::Or(x, y) => Guard::Or(Box::new(x.map(f)), Box::new(y.map(f))),
            Guard::Not(x) => Guard::Not(Box::new(x.map(f))),
        }
    }

    /// The guard at the counter `t`.
    fn at(&self, t: &V) -> Result<Truth, String> {
        let term = |a: &V, b: &V| V::add(V::multiply(a.clone(), t.clone())?, b.clone());
        let decided = |value: V, rel: Rel, against: &V| -> Result<Truth, String> {
            match dependence(&value) {
                1 => Ok(Truth::Holds(V::holds(rel, &value, against)?)),
                finer => Ok(Truth::Finer(finer)),
            }
        };
        let both = |f: &Guard<V>, g: &Guard<V>, and: bool| -> Result<Truth, String> {
            match f.at(t)? {
                Truth::Holds(x) if x != and => Ok(Truth::Holds(x)),
                Truth::Holds(_) => g.at(t),
                finer => Ok(finer),
            }
        };
        match self {
            Guard::Linear { rel, a, b } => decided(term(a, b)?, *rel, &int(0)),
            Guard::Remainder { rel, a, b, m, c } => decided(V::modulo(term(a, b)?, m.clone())?, *rel, c),
            Guard::And(f, g) => both(f, g, true),
            Guard::Or(f, g) => both(f, g, false),
            Guard::Not(f) => Ok(match f.at(t)? {
                Truth::Holds(x) => Truth::Holds(!x),
                finer => finer,
            }),
        }
    }
}

/// The modulus a value's dependence on the residue of the generator needs; `1` for a value the
/// same on every class.
fn dependence<V: TowerValue>(v: &V) -> u64 {
    match v.exact() {
        Some(ExactRef::Natural(n)) if n.value.depends().single().is_none() => n.value.depends().modulus(),
        Some(ExactRef::Depends(d)) if d.value.single().is_none() => d.value.modulus(),
        _ => 1,
    }
}

// ── residue classes ───────────────────────────────────────────────────────────────────────────

/// Every infinitesimal `v` involves, its lists' items, a dependence's branches and a provisional
/// answer's value included, each once.
fn infinitesimals_in<V: ValueAlgebra>(v: &V, out: &mut Vec<Arc<Scale>>) {
    let push = |s: &Arc<Scale>, out: &mut Vec<Arc<Scale>>| {
        if !out.iter().any(|t| t.same_infinitesimal(s)) {
            out.push(s.clone());
        }
    };
    if let Shape::List(items) | Shape::Tuple(items) = v.shape() {
        for item in &items {
            infinitesimals_in(item, out);
        }
        return;
    }
    match v.exact() {
        Some(ExactRef::Hyper(h)) => match &h.value {
            HyperForm::Multi(m) | HyperForm::Integer(m) => m.scales().iter().for_each(|s| push(s, out)),
            HyperForm::Nested(g) => g.scales().iter().for_each(|s| push(s, out)),
            _ => push(&h.scale, out),
        },
        Some(ExactRef::Natural(n)) => push(&n.scale, out),
        Some(ExactRef::Depends(d)) => {
            push(&d.scale, out);
            for (_, b) in d.value.branches() {
                infinitesimals_in(&b.0, out);
            }
        }
        Some(ExactRef::Converging(c)) => infinitesimals_in(c.value(), out),
        _ => {}
    }
}

/// The refusal of a supertask stepped by the infinitesimal `clock` whose `what` holds another, for
/// the reason `why`.
fn on_one_infinitesimal<'v, V: ValueAlgebra + 'v>(
    values: impl Iterator<Item = &'v V>,
    clock: &Scale,
    what: &str,
    why: &str,
) -> Result<(), String> {
    let mut found = Vec::new();
    for v in values {
        infinitesimals_in(v, &mut found);
    }
    match found.iter().find(|s| !s.same_infinitesimal(clock)) {
        Some(other) => Err(format!(
            "this supertask steps by {} and its {what} holds {}: {why}",
            clock.name, other.name
        )),
        None => Ok(()),
    }
}

/// The one infinitesimal `values` involve, or the refusal when they involve two.
fn sole_infinitesimal<V: ValueAlgebra>(values: &[&V]) -> Result<Option<Arc<Scale>>, String> {
    let mut found = Vec::new();
    for v in values {
        infinitesimals_in(*v, &mut found);
    }
    match found.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(one.clone())),
        [a, b, ..] => Err(format!(
            "this supertask's values hold both {} and {}; a supertask over one infinitesimal whose values hold another is not resolved",
            a.name, b.name
        )),
    }
}

/// Every generator whose residue `v` reads — a residue-dependent value's and an infinite integer's that
/// depends on the residue — its lists' items, a dependence's branches and a provisional answer's value
/// included, each once.
fn residue_generators<V: ValueAlgebra>(v: &V, out: &mut Vec<Arc<Scale>>) {
    let mut push = |s: &Arc<Scale>| {
        if !out.iter().any(|t| t.same_infinitesimal(s)) {
            out.push(s.clone());
        }
    };
    if let Shape::List(items) | Shape::Tuple(items) = v.shape() {
        for item in &items {
            residue_generators(item, out);
        }
        return;
    }
    match v.exact() {
        Some(ExactRef::Natural(n)) if n.value.depends().single().is_none() => push(&n.scale),
        Some(ExactRef::Depends(d)) => {
            push(&d.scale);
            for (_, b) in d.value.branches() {
                residue_generators(&b.0, out);
            }
        }
        Some(ExactRef::Converging(c)) => residue_generators(c.value(), out),
        _ => {}
    }
}

/// The moduli of the residues of `scale`'s generator that `v` reads, its lists' items and a
/// dependence's branches included.
fn moduli_on<V: ValueAlgebra>(v: &V, scale: &Scale, out: &mut Vec<u64>) {
    if let Shape::List(items) | Shape::Tuple(items) = v.shape() {
        for item in &items {
            moduli_on(item, scale, out);
        }
        return;
    }
    match v.exact() {
        Some(ExactRef::Natural(n)) if n.scale.same_infinitesimal(scale) => out.push(dependence(v)),
        Some(ExactRef::Depends(d)) => {
            if d.scale.same_infinitesimal(scale) {
                out.push(dependence(v));
            }
            for (_, b) in d.value.branches() {
                moduli_on(&b.0, scale, out);
            }
        }
        Some(ExactRef::Converging(c)) => moduli_on(c.value(), scale, out),
        _ => {}
    }
}

/// The residue-dependent values anywhere in `v`, its lists' items included.
fn dependences<V: ValueAlgebra>(v: &V, out: &mut Vec<V>) {
    match v.shape() {
        Shape::List(items) | Shape::Tuple(items) => {
            for item in &items {
                dependences(item, out);
            }
        }
        _ => {
            if matches!(v.exact(), Some(ExactRef::Depends(_))) {
                out.push(v.clone());
            }
        }
    }
}

/// `v` on the class `h` of the generator, its lists' items included ([`on_class`]).
fn pinned<V: ValueAlgebra>(v: &V, h: u64, class: &Arc<Scale>) -> V {
    match v.shape() {
        Shape::List(items) => V::list(items.iter().map(|item| pinned(item, h, class)).collect()),
        _ => on_class(v, h, class),
    }
}

/// An answer read on one class, back on the program's own scale: a hyperreal is a function of the
/// infinitesimal whatever its residue. An infinite integer stays the polynomial it is on its class,
/// which the classes' answers join into one ([`tower::from_depends`]).
fn rescaled<V: ValueAlgebra>(v: &V, scale: &Arc<Scale>) -> V {
    if let Shape::List(items) = v.shape() {
        return V::list(items.iter().map(|item| rescaled(item, scale)).collect());
    }
    match v.exact() {
        Some(ExactRef::Hyper(x)) if x.scale.same_infinitesimal(scale) && !matches!(x.value, HyperForm::Multi(_) | HyperForm::Integer(_) | HyperForm::Nested(_)) => V::hyper(HyperValue {
            value: x.value.clone(),
            scale: scale.clone(),
        }),
        _ => v.clone(),
    }
}

/// The answers of the classes modulo `modulus` as one value: lists of one width item by item, any
/// other answer the residue-dependent value its classes make (one value where they agree).
fn joined<V: ValueAlgebra>(scale: &Arc<Scale>, modulus: u64, answers: Vec<Option<V>>) -> V {
    let lists: Option<Vec<Option<Vec<V>>>> = answers
        .iter()
        .map(|a| match a {
            None => Some(None),
            Some(v) => match v.shape() {
                Shape::List(items) => Some(Some(items)),
                _ => None,
            },
        })
        .collect();
    if let Some(lists) = lists {
        let mut widths = lists.iter().flatten().map(Vec::len);
        if let Some(width) = widths.next().filter(|w| widths.all(|x| x == *w)) {
            return V::list(
                (0..width)
                    .map(|i| joined(scale, modulus, lists.iter().map(|l| l.as_ref().map(|l| l[i].clone())).collect()))
                    .collect(),
            );
        }
    }
    let branches = answers.into_iter().map(|a| a.map(Branch)).collect();
    let value = Depends::from_branches(scale.residues.clone(), modulus, branches).expect("the classes are the type's own");
    tower::from_depends(value, scale)
}

/// `f` over `args` on each residue class of the generator that the residue-dependent values among
/// them read — every value pinned to its class — joined into one answer, which depends on the residue
/// only where the classes' answers differ; `None` when no argument holds a residue-dependent value.
/// A supertask whose every pass adds `0 when ⌈1/dt⌉ is even, 1 when it is odd` ends at `0` on the
/// even classes and `1/dt` on the odd ones.
///
/// Values that read the residues of several generators are read on the classes of one of them, an
/// infinite integer's own among them, and `f` reads the others within each class the same way; the
/// classes' answers join in the one form [`tower::from_depends`] gives them, a dependence on the
/// coarsest — the generator of the largest infinitesimal — whose branches depend on the finer.
fn on_each_class<V: ValueAlgebra>(args: &[&V], f: impl Fn(&[V]) -> Result<V, String>) -> Option<Result<V, String>> {
    let mut found = Vec::new();
    for arg in args {
        dependences(*arg, &mut found);
    }
    let mut generators = Vec::new();
    for arg in args {
        residue_generators(*arg, &mut generators);
    }
    for (i, x) in generators.iter().enumerate() {
        if let Some(y) = generators[i + 1..].iter().find(|y| x.order(y).is_none()) {
            return Some(Err(x.unordered(y)));
        }
    }
    let scale = match generators.as_slice() {
        [first, _, ..] => first.clone(),
        _ => scale_among(&found.iter().collect::<Vec<_>>())?,
    };
    let refuse = |e: ResidueError| e.describe(&scale.generator_text());
    let mut modulus = scale.residues.modulus();
    let mut moduli = Vec::new();
    for arg in args {
        moduli_on(*arg, &scale, &mut moduli);
    }
    for m in moduli {
        modulus = match bounded_lcm(modulus, m) {
            Ok(m) => m,
            Err(e) => return Some(Err(refuse(e))),
        };
    }
    let mut answers: Vec<Option<V>> = Vec::with_capacity(modulus as usize);
    for h in 0..modulus {
        if !scale.residues.admits(h) {
            answers.push(None);
            continue;
        }
        let class = Arc::new(scale.with_residues(
            scale
                .residues
                .and(&ResidueType::congruent(h, modulus))
                .expect("an admitted class is consistent with its type"),
        ));
        let on_class: Vec<V> = args.iter().map(|arg| pinned(*arg, h, &class)).collect();
        match f(&on_class) {
            Ok(answer) => answers.push(Some(rescaled(&answer, &scale))),
            Err(e) => return Some(Err(e)),
        }
    }
    Some(Ok(joined(&scale, modulus, answers)))
}

/// The program's infinitesimal, as the first value on its scale carries it.
fn scale_among<V: TowerValue>(values: &[&V]) -> Option<Arc<Scale>> {
    values.iter().find_map(|v| match v.exact()? {
        ExactRef::Hyper(h) => Some(h.scale.clone()),
        ExactRef::Natural(n) => Some(n.scale.clone()),
        ExactRef::Depends(d) => Some(d.scale.clone()),
        _ => None,
    })
}

/// `v` on the class of the generator `class` describes, `H ≡ h`: an infinite integer by its
/// polynomial there, a residue-dependent value by its branch there, and a hyperreal on the
/// class's own scale.
fn on_class<V: TowerValue>(v: &V, h: u64, class: &Arc<Scale>) -> V {
    match v.exact() {
        Some(ExactRef::Hyper(x)) if x.scale.same_infinitesimal(class) && !matches!(x.value, HyperForm::Multi(_) | HyperForm::Integer(_) | HyperForm::Nested(_)) => V::hyper(HyperValue {
            value: x.value.clone(),
            scale: class.clone(),
        }),
        Some(ExactRef::Natural(n)) if n.scale.same_infinitesimal(class) => {
            let p = n.value.depends().at(h).expect("an admitted class has a branch").clone();
            let value = Hypernatural::from_polynomial(p, &class.residues).expect("a branch is an integer on its class");
            tower::from_natural(value, class)
        }
        Some(ExactRef::Depends(d)) if d.scale.same_infinitesimal(class) => {
            on_class(&d.value.at(h).expect("an admitted class has a branch").0, h, class)
        }
        _ => v.clone(),
    }
}

/// The polynomial in the generator an exit is on its own class.
fn class_polynomial<V: TowerValue>(v: &V) -> Option<Poly> {
    if let Some(n) = v.integer() {
        return Some(Poly::constant(Rational::from_bigint(n)));
    }
    match v.exact()? {
        ExactRef::Natural(n) => n.value.depends().single().cloned(),
        _ => None,
    }
}

/// Where a guard first fails on one residue class of the generator.
enum Exit<V> {
    At(V),
    Never,
    /// A value the class does not pin down: the classes modulo this must be read apart first.
    Finer(u64),
}

/// How many passes `While <guard>:` makes when each pass adds `step` to its counter, from `t0`: the
/// first natural `k` at which the guard fails at `t0 + k·step`. The guard compares terms linear in
/// the counter (the formula [`Guard::read`] takes), so its truth can change only where one of them
/// crosses zero and otherwise repeats with the period of its remainders; its first failure lies at
/// a crossing or within one period after one, and is found by evaluating the guard there alone.
/// Where those points depend on the residue of the generator — `⌊1/(2·dt)⌋` — each class is read on
/// its own scale and the count is the infinite integer the classes' answers make.
///
/// `−1` — not counted, so the loop runs as written — when every number is standard (the loop then
/// ends after finitely many passes, or never, exactly as it is written), when one is not a number,
/// and when a standard step never fails the guard; a refusal when an infinitesimal one never does.
pub fn first_exit<V: ValueAlgebra>(t0: &V, step: &V, guard: &V) -> Result<V, String> {
    let guard = Guard::read(guard)?;
    let mut numbers = vec![t0, step];
    guard.numbers(&mut numbers);
    let countable = |v: &&V| {
        v.is_standard_number()
            || matches!(
                v.exact(),
                Some(
                    ExactRef::Algebraic(_)
                        | ExactRef::Real(_)
                        | ExactRef::Hyper(_)
                        | ExactRef::Natural(_)
                        | ExactRef::Depends(_)
                )
            )
    };
    if !numbers.iter().all(countable) || numbers.iter().all(|v| is_standard(*v)) {
        return Ok(int(-1));
    }
    sole_infinitesimal(&numbers)?;
    let scale = scale_among(&numbers).expect("a value that is not standard has a scale");
    let refuse = |e: ResidueError| e.describe(&scale.generator_text());
    let mut modulus = scale.residues.modulus();
    for v in &numbers {
        modulus = bounded_lcm(modulus, dependence(*v)).map_err(refuse)?;
    }
    'refine: loop {
        let mut exits: Vec<Option<Exit<V>>> = Vec::with_capacity(modulus as usize);
        for h in 0..modulus {
            if !scale.residues.admits(h) {
                exits.push(None);
                continue;
            }
            let class = Arc::new(scale.with_residues(
                scale
                    .residues
                    .and(&ResidueType::congruent(h, modulus))
                    .expect("an admitted class is consistent with its type"),
            ));
            let pin = |v: &V| on_class(v, h, &class);
            let (t0, step) = (pin(t0), pin(step));
            match exit_on_class(&t0, &step, &guard.map(&pin))? {
                Exit::Finer(m) => {
                    let finer = bounded_lcm(modulus, m).map_err(refuse)?;
                    if finer == modulus {
                        return Err(format!(
                            "this loop's guard depends on the residue of {} past the classes that separate it",
                            scale.generator_text()
                        ));
                    }
                    modulus = finer;
                    continue 'refine;
                }
                exit => exits.push(Some(exit)),
            }
        }
        if exits.iter().flatten().any(|e| matches!(e, Exit::Never)) {
            let finite = exits.iter().flatten().all(|e| match e {
                Exit::At(k) => is_standard(k),
                _ => true,
            });
            if is_standard(step) && finite {
                return Ok(int(-1));
            }
            return Err(format!(
                "this loop never ends: its counter starts at {}, steps by {}, and its guard holds at every step",
                t0.display_string(),
                step.display_string()
            ));
        }
        let branches: Vec<Option<Poly>> = exits
            .iter()
            .map(|e| match e {
                Some(Exit::At(k)) => Some(class_polynomial(k).expect("an exit is an integer on its class")),
                _ => None,
            })
            .collect();
        let value = Depends::from_branches(scale.residues.clone(), modulus, branches)
            .expect("the classes are the type's own");
        let count = Hypernatural::from_branches(value).expect("each branch is an integer on its class");
        return Ok(tower::from_natural(count, &scale));
    }
}

/// The first failure of `guard` on one class, every value pinned to it.
fn exit_on_class<V: ValueAlgebra>(t0: &V, step: &V, guard: &Guard<V>) -> Result<Exit<V>, String> {
    let mut starts: Vec<V> = vec![int(0)];
    let mut period: u64 = 1;
    let mut atoms = Vec::new();
    guard.atoms(&mut atoms);
    for atom in atoms {
        let (Guard::Linear { a, b, .. } | Guard::Remainder { a, b, .. }) = atom else {
            unreachable!("an atom is a linear term or a remainder");
        };
        let rate = V::multiply(a.clone(), step.clone())?;
        if let Guard::Remainder { m, .. } = atom {
            let entry = V::list(vec![rate.clone(), m.clone()]);
            let p = remainder_period(&entry)?.ok_or_else(|| {
                format!(
                    "a loop's guard takes a remainder by a whole number of a term that steps by a \
                     rational amount; here the term steps by {} and the remainder is by {}",
                    rate.display_string(),
                    m.display_string()
                )
            })?;
            period = bounded_lcm(period, p).map_err(|_| "a loop's guard repeats with too long a period".to_string())?;
            continue;
        }
        if V::holds(Rel::Eq, &rate, &int(0))? {
            continue;
        }
        let start = V::add(V::multiply(a.clone(), t0.clone())?, b.clone())?;
        let crossing = V::floor(V::exact_divide(V::subtract(int(0), start)?, rate)?)?;
        match dependence(&crossing) {
            1 => starts.push(crossing),
            finer => return Ok(Exit::Finer(finer)),
        }
    }
    let mut ordered: Vec<V> = Vec::with_capacity(starts.len());
    for s in starts {
        if !V::holds(Rel::GtEq, &s, &int(0))? {
            continue;
        }
        let mut at = ordered.len();
        for (i, o) in ordered.iter().enumerate() {
            if V::holds(Rel::LtEq, &s, o)? {
                at = i;
                break;
            }
        }
        if at == ordered.len() || !V::holds(Rel::Eq, &s, &ordered[at])? {
            ordered.insert(at, s);
        }
    }
    let mut examined: u64 = 0;
    let mut next: Option<V> = None;
    for s in ordered {
        let mut k = match next {
            Some(n) if V::holds(Rel::Gt, &n, &s)? => n,
            _ => s.clone(),
        };
        let last = V::add(s, int(period as i64))?;
        while V::holds(Rel::LtEq, &k, &last)? {
            examined += 1;
            if examined > MAX_CANDIDATES {
                return Err("a loop's guard changes at too many points to read where it first fails".to_string());
            }
            let t = V::add(t0.clone(), V::multiply(k.clone(), step.clone())?)?;
            match guard.at(&t)? {
                Truth::Holds(true) => {}
                Truth::Holds(false) => return Ok(Exit::At(k)),
                Truth::Finer(m) => return Ok(Exit::Finer(m)),
            }
            k = V::add(k, int(1))?;
        }
        next = Some(k);
    }
    Ok(Exit::Never)
}

// ── search candidates ─────────────────────────────────────────────────────────────────────────

/// The most candidates one search examines; past it the search is not decided this way.
const MAX_CANDIDATES: u64 = 1 << 20;

/// A standard exact coefficient; `None` for a float (whose rounding the linear reading would not
/// see) or anything nonstandard.
fn exact_standard<V: ValueAlgebra>(v: &V) -> Option<V> {
    match v.shape() {
        Shape::Int(_) | Shape::BigInt(_) | Shape::Rational(_) => Some(v.clone()),
        _ => matches!(v.exact(), Some(ExactRef::Algebraic(_))).then(|| v.clone()),
    }
}

/// The period in `n` of a remainder term: `m` for `n % m`, written as the modulus alone, and
/// `m·q` for `(a·n + b) % m`, written `[a, m]`, with `a = p/q` in lowest terms — `a·(n + m·q)`
/// differs from `a·n` by `p·m`. `None` when it is not a positive whole period.
fn remainder_period<V: ValueAlgebra>(entry: &V) -> Result<Option<u64>, String> {
    let (a, m) = match entry.shape() {
        Shape::List(_) => {
            let pair = items(entry, "a remainder term")?;
            match pair.as_slice() {
                [a, m] => (a.clone(), m.clone()),
                _ => return Ok(None),
            }
        }
        _ => (int(1), entry.clone()),
    };
    let m = match m.shape() {
        Shape::Int(k) if k > 0 => k as u64,
        _ => return Ok(None),
    };
    let denominator = match a.shape() {
        Shape::Int(_) | Shape::BigInt(_) => 1,
        Shape::Rational(r) => match r.denominator().to_u64() {
            Some(q) => q,
            None => return Ok(None),
        },
        _ => return Ok(None),
    };
    Ok(m.checked_mul(denominator))
}

/// The naturals a search over every natural must examine, in increasing order; empty when the
/// predicate's coefficients or periods are not ones this reading decides. See the module
/// documentation.
pub fn search_candidates<V: ValueAlgebra>(atoms: &V, moduli: &V) -> Result<V, String> {
    let undecided = || Ok(V::list(Vec::new()));
    let mut period: u64 = 1;
    for entry in items(moduli, "a search's periods")? {
        let Some(m) = remainder_period(&entry)? else {
            return undecided();
        };
        let l = u128::from(period / gcd(period, m)) * u128::from(m);
        if l > u128::from(MAX_CANDIDATES) {
            return undecided();
        }
        period = l as u64;
    }
    let mut starts: Vec<u64> = vec![0];
    for atom in items(atoms, "a search's linear terms")? {
        let pair = items(&atom, "a linear term")?;
        let (Some(a), Some(b)) = (
            pair.first().and_then(exact_standard),
            pair.get(1).and_then(exact_standard),
        ) else {
            return undecided();
        };
        if V::holds(Rel::Eq, &a, &int(0))? {
            continue;
        }
        let root = V::exact_divide(V::subtract(int(0), b)?, a)?;
        let sides: [fn(V) -> Result<V, String>; 2] = [V::floor, V::ceil];
        for side in sides {
            match side(root.clone())?.shape() {
                Shape::Int(t) if t >= 0 => starts.push(t as u64),
                Shape::Int(_) => {}
                Shape::BigInt(t) if t.is_negative() => {}
                _ => return undecided(),
            }
        }
    }
    starts.sort_unstable();
    starts.dedup();
    let mut out: Vec<u64> = Vec::new();
    for s in starts {
        let from = out.last().map_or(s, |last| s.max(last + 1));
        let to = s.saturating_add(period);
        if to >= from {
            if out.len() as u64 + (to - from + 1) > MAX_CANDIDATES {
                return undecided();
            }
            out.extend(from..=to);
        }
    }
    Ok(V::list(out.into_iter().map(|n| int(n as i64)).collect()))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/supertask/tests.rs"]
mod tests;
