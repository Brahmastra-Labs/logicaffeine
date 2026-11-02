//! Flows across infinitesimals: the state a loop stepped by one infinitesimal leaves when its affine
//! update holds values of others.
//!
//! **Rates of larger infinitesimals.** `N = T/s + δ` steps of `x ↦ A·x`, `A = I + Σ_{k≥1} u^k·A_k` with
//! `u = s^(1/q)` and every `A_k` a matrix of values of the infinitesimals larger than `s`, leave
//! `exp(N·log A)·x₀` — the recurrences of one infinitesimal's flow ([`crate::germ`]) over those values:
//! `log A = Σ u^k·L_k`, the exponent `C_0 + Σ_{j≥1} u^j·C_j` with `C_j = T·L_{q+j} + δ·L_j`, `C_0 = T·A_q`,
//! and with commuting terms `exp(Σ u^j·C_j) = Σ u^m·E_m`, `E_m = (1/m)·Σ k·C_k·E_{m−k}`. The standard
//! part relative to the larger infinitesimals is `e^{T·A_q}·x₀`, the inner limit. Without commuting
//! terms, the orders below the first nonzero `C_j` are known, and the rest refused by name.
//!
//! **The exponential of a matrix of non-standard rates** ([`matrix_exp`]) is read one infinitesimal at a
//! time: with `A(ε) = Σ ε^j·B_j` in the smallest infinitesimal its entries hold, the block upper-
//! triangular Toeplitz matrix of `B_0, …, B_m` has as exponential the Toeplitz matrix of the first
//! `m + 1` coefficients of `e^{T·A(ε)}` (Van Loan), so each coefficient is the corner block of the
//! exponential of a matrix of values of larger infinitesimals — down to rational matrices, exactly.
//!
//! **Rates of smaller infinitesimals.** `N` steps of `A = I + Σ_{k≥1} w^k·A_k`, `w` a root of an
//! infinitesimal smaller than `s` and the `A_k` values of `s` and the larger ones, leave
//! `exp(N·log A)·x₀`, the series `Σ_m w^m·P_m` with `P_m = Σ_{j ≤ m} (N^j/j!)·[(log A)^j]_m`: every
//! coefficient exact, `N` an ordinary number from the smaller infinitesimal's standpoint. A scalar
//! update not the identity at order `0` is `exp(N·ln A)`, the germs' own exponential and logarithm.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::expansion::{Coefficient, Expansion, ExpansionError, Source};
use crate::matrix_exp::exp_matrix;
use crate::nested::{GermRecipe, NestedGerm};
use crate::number::Number;
use crate::numeric::Rational;
use crate::tower::{self, Op, Scale, TowerValue};

/// A matrix of numbers.
pub(crate) type NMatrix = Vec<Vec<Number>>;

fn text(e: ExpansionError) -> String {
    e.to_string()
}

fn zeros(n: usize) -> NMatrix {
    vec![vec![Number::zero(); n]; n]
}

fn identity(n: usize) -> NMatrix {
    (0..n)
        .map(|i| (0..n).map(|j| if i == j { Number::one() } else { Number::zero() }).collect())
        .collect()
}

fn mat_mul(a: &NMatrix, b: &NMatrix) -> Result<NMatrix, String> {
    let n = a.len();
    let mut out = zeros(n);
    for (i, row) in a.iter().enumerate() {
        for (k, x) in row.iter().enumerate() {
            if x.is_zero().map_err(text)? {
                continue;
            }
            for j in 0..n {
                out[i][j] = out[i][j].add(&x.mul(&b[k][j]).map_err(text)?).map_err(text)?;
            }
        }
    }
    Ok(out)
}

fn mat_add(a: &NMatrix, b: &NMatrix) -> Result<NMatrix, String> {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.iter().zip(y).map(|(p, q)| p.add(q).map_err(text)).collect())
        .collect()
}

fn mat_scale(a: &NMatrix, k: &Number) -> Result<NMatrix, String> {
    a.iter()
        .map(|row| row.iter().map(|v| v.mul(k).map_err(text)).collect())
        .collect()
}

fn is_zero_matrix(a: &NMatrix) -> Result<bool, String> {
    for row in a {
        for v in row {
            if !v.is_zero().map_err(text)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn rational(r: Rational) -> Number {
    <Number as TowerValue>::from_rational(r)
}

fn mat_vec(a: &NMatrix, x: &[Number]) -> Result<Vec<Number>, String> {
    a.iter()
        .map(|row| {
            row.iter().zip(x).try_fold(Number::zero(), |acc, (e, v)| {
                acc.add(&e.mul(v).map_err(text)?).map_err(text)
            })
        })
        .collect()
}

fn lcm(a: u32, b: u32) -> u32 {
    fn gcd(a: u64, b: u64) -> u64 {
        if b == 0 {
            a
        } else {
            gcd(b, a % b)
        }
    }
    let g = gcd(u64::from(a), u64::from(b));
    u32::try_from(u64::from(a) / g * u64::from(b)).expect("a grid of moderate index")
}

/// The smallest infinitesimal any of the numbers involves.
fn smallest_among<'a>(numbers: impl Iterator<Item = &'a Number>) -> Option<Arc<Scale>> {
    let mut smallest: Option<Arc<Scale>> = None;
    for n in numbers {
        for s in tower::number_scales(n) {
            smallest = Some(match smallest {
                None => s,
                Some(t) if s.order(&t) == Some(std::cmp::Ordering::Less) => s,
                Some(t) => t,
            });
        }
    }
    smallest
}

/// The expansions of a matrix's entries at `scale`, on the least grid all lie on, with that grid.
fn expansions(a: &NMatrix, scale: &Arc<Scale>) -> Result<(Vec<Vec<Expansion<Number>>>, u32), String> {
    let mut grid = 1;
    let mut out = Vec::with_capacity(a.len());
    for row in a {
        let mut r = Vec::with_capacity(row.len());
        for v in row {
            let e = tower::number_germ(v, scale)?.expansion.clone();
            grid = lcm(grid, e.ramification());
            r.push(e);
        }
        out.push(r);
    }
    let out = out
        .into_iter()
        .map(|row| row.into_iter().map(|e| e.refined(grid / e.ramification())).collect())
        .collect();
    Ok((out, grid))
}

/// The matrix of order `k` of a matrix of expansions on one grid.
fn term(e: &[Vec<Expansion<Number>>], k: i64) -> Result<NMatrix, String> {
    e.iter()
        .map(|row| row.iter().map(|x| x.coefficient(k).map_err(text)).collect())
        .collect()
}

/// `L_k`, the coefficient of `u^k` in `log A = Σ_{n≥1} (−1)^{n+1}·X^n/n`, `X = Σ_{k≥1} u^k·A_k`.
fn log_term(terms: &dyn Fn(usize) -> Result<NMatrix, String>, n: usize, k: usize) -> Result<NMatrix, String> {
    let mut total = zeros(n);
    let mut power: Vec<NMatrix> = (0..=k).map(|j| if j == 0 { identity(n) } else { zeros(n) }).collect();
    for count in 1..=k {
        let mut next: Vec<NMatrix> = vec![zeros(n); k + 1];
        for (have, p) in power.iter().enumerate() {
            if is_zero_matrix(p)? {
                continue;
            }
            for step in 1..=(k - have) {
                next[have + step] = mat_add(&next[have + step], &mat_mul(p, &terms(step)?)?)?;
            }
        }
        power = next;
        let sign = if count % 2 == 1 { 1 } else { -1 };
        let c = rational(Rational::from_ratio_i64(sign, count as i64).expect("count ≥ 1"));
        total = mat_add(&total, &mat_scale(&power[k], &c)?)?;
    }
    Ok(total)
}

// ── the exponential of a matrix of numbers ───────────────────────────────────────────────────

/// The data of `e^{t·A}`, shared by its entries' expansions.
#[derive(Debug)]
pub(crate) struct MatrixExpSpec {
    pub(crate) a: NMatrix,
    pub(crate) t: Rational,
    scale: Arc<Scale>,
    entries: Vec<Vec<Expansion<Number>>>,
    /// The coefficient matrices of `e^{t·A(ε)}` found so far.
    found: Mutex<Vec<NMatrix>>,
}

impl MatrixExpSpec {
    /// The coefficient of `ε^m` in `e^{t·A(ε)}`: the corner block of the exponential of the block
    /// Toeplitz matrix of `B_0, …, B_m`.
    fn coefficient(&self, m: usize) -> Result<NMatrix, String> {
        if let Some(found) = self.found.lock().expect("unpoisoned").get(m) {
            return Ok(found.clone());
        }
        let n = self.a.len();
        let blocks: Vec<NMatrix> = (0..=m).map(|j| term(&self.entries, j as i64)).collect::<Result<_, _>>()?;
        let size = (m + 1) * n;
        let mut big = vec![vec![Number::zero(); size]; size];
        for bi in 0..=m {
            for bj in bi..=m {
                for i in 0..n {
                    for j in 0..n {
                        big[bi * n + i][bj * n + j] = blocks[bj - bi][i][j].clone();
                    }
                }
            }
        }
        let e = matrix_exp(&big, &self.t)?;
        let mut found = self.found.lock().expect("unpoisoned");
        while found.len() <= m {
            let k = found.len();
            found.push((0..n).map(|i| (0..n).map(|j| e[i][k * n + j].clone()).collect()).collect());
        }
        Ok(found[m].clone())
    }
}

#[derive(Debug)]
struct MatrixExpEntry(Arc<MatrixExpSpec>, usize, usize);

impl Source<Number> for MatrixExpEntry {
    fn coefficient(&self, k: i64) -> Result<Number, ExpansionError> {
        if k < 0 {
            return Ok(Number::zero());
        }
        self.0
            .coefficient(k as usize)
            .map(|m| m[self.1][self.2].clone())
            .map_err(ExpansionError::Coefficient)
    }
}

/// `e^{t·A}` for a square matrix of numbers. See the module documentation; a matrix of standard
/// numbers that are not all rational is refused, and so is one infinite in its smallest infinitesimal.
pub(crate) fn matrix_exp(a: &NMatrix, t: &Rational) -> Result<NMatrix, String> {
    let n = a.len();
    let rationals: Option<Vec<Vec<Rational>>> = a
        .iter()
        .map(|row| row.iter().map(|x| x.standard_rational()).collect())
        .collect();
    if let Some(r) = rationals {
        let e = exp_matrix(&r, t).ok_or_else(|| "the exponential of a matrix that is not square".to_string())?;
        return Ok(e.into_iter().map(|row| row.into_iter().map(tower::from_exact_real).collect()).collect());
    }
    if n == 1 {
        let ta = Number::arith(Op::Mul, rational(t.clone()), a[0][0].clone())?;
        return Ok(vec![vec![ta.exp().map_err(text)?]]);
    }
    let Some(scale) = smallest_among(a.iter().flatten()) else {
        return Err("the exponential of a matrix of irrational standard numbers is not computed".to_string());
    };
    let (entries, grid) = expansions(a, &scale)?;
    for row in &entries {
        for e in row {
            for k in e.floor()..0 {
                if !e.coefficient(k).map_err(text)?.is_zero().map_err(text)? {
                    return Err(format!(
                        "the exponential of a matrix with an entry infinite in {} is beyond every power of it",
                        scale.name
                    ));
                }
            }
        }
    }
    let spec = Arc::new(MatrixExpSpec {
        a: a.clone(),
        t: t.clone(),
        scale: scale.clone(),
        entries,
        found: Mutex::new(Vec::new()),
    });
    Ok((0..n)
        .map(|i| {
            (0..n)
                .map(|j| {
                    let expansion = Expansion::source(Arc::new(MatrixExpEntry(spec.clone(), i, j)), 0, grid);
                    Number::hyper(tower::HyperValue {
                        value: tower::HyperForm::Nested(Arc::new(NestedGerm::from_source(
                            &spec.scale,
                            GermRecipe::MatrixExp(spec.clone(), i, j),
                            expansion,
                        ))),
                        scale: spec.scale.clone(),
                    })
                })
                .collect()
        })
        .collect())
}

// ── a flow whose rates are values of larger infinitesimals ───────────────────────────────────

/// The data of a flow stepped by `scale` with rates of larger infinitesimals, shared by its state's
/// components. See the module documentation.
#[derive(Debug)]
pub(crate) struct FlowSpec {
    pub(crate) a: NMatrix,
    pub(crate) t: Rational,
    pub(crate) delta: Rational,
    pub(crate) start: Vec<Number>,
    scale: Arc<Scale>,
    entries: Vec<Vec<Expansion<Number>>>,
    /// The update's grid: `A = I + Σ_{k≥1} u^k·A_k`, `u = s^(1/grid)`.
    grid: u32,
    starts: Vec<Expansion<Number>>,
    /// The state's grid and lowest order.
    q: u32,
    floor: i64,
    exp_t: NMatrix,
    exponent: Mutex<Vec<NMatrix>>,
    expansion: Mutex<Vec<NMatrix>>,
    state: Mutex<BTreeMap<i64, Vec<Number>>>,
}

impl FlowSpec {
    fn width(&self) -> usize {
        self.a.len()
    }

    fn term(&self, k: usize) -> Result<NMatrix, String> {
        term(&self.entries, k as i64)
    }

    /// `C_j = T·L_{grid+j} + δ·L_j`.
    fn exponent(&self, j: usize) -> Result<NMatrix, String> {
        let mut cache = self.exponent.lock().expect("unpoisoned");
        while cache.len() < j {
            let i = cache.len() + 1;
            let n = self.width();
            let terms = |k: usize| self.term(k);
            let c = mat_add(
                &mat_scale(&log_term(&terms, n, i + self.grid as usize)?, &rational(self.t.clone()))?,
                &mat_scale(&log_term(&terms, n, i)?, &rational(self.delta.clone()))?,
            )?;
            cache.push(c);
        }
        Ok(cache[j - 1].clone())
    }

    /// Whether the exponent's terms through `C_reach` commute, which makes their exponential the
    /// product the recurrence reads.
    fn commuting_through(&self, reach: usize) -> Result<bool, String> {
        let cs: Vec<NMatrix> = (1..=reach).map(|j| self.exponent(j)).collect::<Result<_, _>>()?;
        for (i, a) in cs.iter().enumerate() {
            for b in &cs[i + 1..] {
                let (ab, ba) = (mat_mul(a, b)?, mat_mul(b, a)?);
                let difference: NMatrix = ab
                    .iter()
                    .zip(&ba)
                    .map(|(x, y)| x.iter().zip(y).map(|(p, q)| p.sub(q).map_err(text)).collect())
                    .collect::<Result<_, String>>()?;
                if !is_zero_matrix(&difference)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    fn expansion(&self, m: usize) -> Result<NMatrix, String> {
        let mut cache = self.expansion.lock().expect("unpoisoned");
        if cache.is_empty() {
            cache.push(identity(self.width()));
        }
        while cache.len() <= m {
            let mm = cache.len();
            let mut total = zeros(self.width());
            for k in 1..=mm {
                let c = self.exponent(k)?;
                if is_zero_matrix(&c)? {
                    continue;
                }
                let term = mat_mul(&c, &cache[mm - k])?;
                total = mat_add(&total, &mat_scale(&term, &rational(Rational::from_i64(k as i64)))?)?;
            }
            cache.push(mat_scale(&total, &rational(Rational::from_ratio_i64(1, mm as i64).expect("mm ≥ 1")))?);
        }
        Ok(cache[m].clone())
    }

    /// `s_m = e^{T·A_grid}·Σ_{a·r+b=m} E_a·x₀_b`, in units of `s^(1/q)`.
    fn state(&self, m: i64) -> Result<Vec<Number>, String> {
        if let Some(s) = self.state.lock().expect("unpoisoned").get(&m) {
            return Ok(s.clone());
        }
        let r = i64::from(self.q / self.grid);
        let reach = ((m - self.floor).max(0) / r) as usize;
        let nonzero_exponent = (1..=reach)
            .map(|j| self.exponent(j).and_then(|c| is_zero_matrix(&c).map(|z| !z)))
            .collect::<Result<Vec<bool>, String>>()?;
        if nonzero_exponent.iter().any(|b| *b) && !self.commuting_through(reach)? {
            return Err(format!(
                "the {} term of this flow is not computed exactly: its update matrices do not commute",
                crate::puiseux::power_text(&self.scale.name, &Rational::from_ratio_i64(m, i64::from(self.q)).expect("q > 0"))
            ));
        }
        let n = self.width();
        let mut inner = vec![Number::zero(); n];
        for a in 0..=reach {
            let e = self.expansion(a)?;
            if is_zero_matrix(&e)? {
                continue;
            }
            let start: Vec<Number> = self
                .starts
                .iter()
                .map(|g| g.coefficient(m - a as i64 * r).map_err(text))
                .collect::<Result<_, _>>()?;
            let moved = mat_vec(&e, &start)?;
            inner = inner
                .iter()
                .zip(&moved)
                .map(|(x, y)| x.add(y).map_err(text))
                .collect::<Result<_, _>>()?;
        }
        let s = mat_vec(&self.exp_t, &inner)?;
        self.state.lock().expect("unpoisoned").insert(m, s.clone());
        Ok(s)
    }
}

#[derive(Debug)]
struct FlowComponent(Arc<FlowSpec>, usize);

impl Source<Number> for FlowComponent {
    fn coefficient(&self, k: i64) -> Result<Number, ExpansionError> {
        self.0.state(k).map(|s| s[self.1].clone()).map_err(ExpansionError::Coefficient)
    }
}

/// The state after `N = T/s + δ` steps of `x ↦ A·x`, `A`'s entries values of `scale` and the larger
/// infinitesimals: each component a germ of several infinitesimals at `scale`.
pub(crate) fn flow(scale: &Arc<Scale>, a: &NMatrix, t: Rational, delta: Rational, start: &[Number]) -> Result<Vec<Number>, String> {
    let n = a.len();
    let (entries, grid) = expansions(a, scale)?;
    let at_zero = term(&entries, 0)?;
    let difference: NMatrix = at_zero
        .iter()
        .zip(identity(n))
        .map(|(x, y)| x.iter().zip(&y).map(|(p, q)| p.sub(q).map_err(text)).collect())
        .collect::<Result<_, String>>()?;
    if !is_zero_matrix(&difference)? {
        return Err(format!(
            "the update changes its state at every step by an amount that is no infinitesimal of {}, so this supertask is no flow of it",
            scale.name
        ));
    }
    for k in 1..grid as i64 {
        if !is_zero_matrix(&term(&entries, k)?)? {
            return Err(format!(
                "the update has a term of a fractional order of {} below its first, so this supertask is not a flow",
                scale.name
            ));
        }
    }
    let starts: Vec<Expansion<Number>> = start
        .iter()
        .map(|x| tower::number_germ(x, scale).map(|g| g.expansion.clone()))
        .collect::<Result<_, _>>()?;
    let q = starts.iter().fold(grid, |l, e| lcm(l, e.ramification()));
    let starts: Vec<Expansion<Number>> = starts.into_iter().map(|e| e.refined(q / e.ramification())).collect();
    let floor = starts.iter().map(Expansion::floor).min().unwrap_or(0);
    let exp_t = matrix_exp(&term(&entries, i64::from(grid))?, &t)?;
    let spec = Arc::new(FlowSpec {
        a: a.clone(),
        t,
        delta,
        start: start.to_vec(),
        scale: scale.clone(),
        entries,
        grid,
        starts,
        q,
        floor,
        exp_t,
        exponent: Mutex::new(Vec::new()),
        expansion: Mutex::new(Vec::new()),
        state: Mutex::new(BTreeMap::new()),
    });
    Ok((0..n)
        .map(|i| {
            let expansion = Expansion::source(Arc::new(FlowComponent(spec.clone(), i)), floor, q);
            Number::hyper(tower::HyperValue {
                value: tower::HyperForm::Nested(Arc::new(NestedGerm::from_source(
                    scale,
                    GermRecipe::Flow(spec.clone(), i),
                    expansion,
                ))),
                scale: scale.clone(),
            })
        })
        .collect())
}

// ── a power whose rates are values of a smaller infinitesimal ────────────────────────────────

/// The data of `A^N·x₀` for `A = I + Σ_{k≥1} w^k·A_k`, `w` a root of `scale`. See the module
/// documentation.
#[derive(Debug)]
pub(crate) struct PowerSpec {
    pub(crate) a: NMatrix,
    pub(crate) count: Number,
    pub(crate) start: Vec<Number>,
    entries: Vec<Vec<Expansion<Number>>>,
    starts: Vec<Expansion<Number>>,
    /// `(log A)^j` truncated, by `j` and order.
    logs: Mutex<Vec<Vec<NMatrix>>>,
    found: Mutex<BTreeMap<i64, Vec<Number>>>,
}

impl PowerSpec {
    fn width(&self) -> usize {
        self.a.len()
    }

    /// `[(log A)^j]_k` for every `j ≤ k ≤ m`.
    fn log_powers(&self, m: usize) -> Result<Vec<Vec<NMatrix>>, String> {
        let n = self.width();
        let mut logs = self.logs.lock().expect("unpoisoned");
        if logs.first().is_none_or(|row| row.len() <= m) {
            let terms = |k: usize| term(&self.entries, k as i64);
            let l: Vec<NMatrix> = (0..=m)
                .map(|k| if k == 0 { Ok(zeros(n)) } else { log_term(&terms, n, k) })
                .collect::<Result<_, _>>()?;
            let mut powers: Vec<Vec<NMatrix>> = vec![(0..=m).map(|k| if k == 0 { identity(n) } else { zeros(n) }).collect()];
            for j in 1..=m {
                let previous = &powers[j - 1];
                let mut next = vec![zeros(n); m + 1];
                for (a, p) in previous.iter().enumerate() {
                    if is_zero_matrix(p)? {
                        continue;
                    }
                    for (b, lb) in l.iter().enumerate().skip(1) {
                        if a + b > m {
                            break;
                        }
                        next[a + b] = mat_add(&next[a + b], &mat_mul(p, lb)?)?;
                    }
                }
                powers.push(next);
            }
            *logs = powers;
        }
        Ok(logs.clone())
    }

    /// `P_m = Σ_{j ≤ m} (N^j/j!)·[(log A)^j]_m`.
    fn power(&self, m: usize) -> Result<NMatrix, String> {
        let logs = self.log_powers(m)?;
        let n = self.width();
        let mut total = zeros(n);
        let mut weight = Number::one();
        for (j, by_order) in logs.iter().enumerate().take(m + 1) {
            if j > 0 {
                weight = weight
                    .mul(&self.count)
                    .map_err(text)?
                    .mul(&rational(Rational::from_ratio_i64(1, j as i64).expect("j ≥ 1")))
                    .map_err(text)?;
            }
            total = mat_add(&total, &mat_scale(&by_order[m], &weight)?)?;
        }
        Ok(total)
    }

    /// The state's coefficient of order `m` on the state's grid: `Σ_{a·r+b=m} P_a·x₀_b`.
    fn state(&self, m: i64) -> Result<Vec<Number>, String> {
        if let Some(s) = self.found.lock().expect("unpoisoned").get(&m) {
            return Ok(s.clone());
        }
        let floor = self.starts.iter().map(Expansion::floor).min().unwrap_or(0);
        let reach = (m - floor).max(0) as usize;
        let n = self.width();
        let mut out = vec![Number::zero(); n];
        for a in 0..=reach {
            let p = self.power(a)?;
            if is_zero_matrix(&p)? {
                continue;
            }
            let start: Vec<Number> = self
                .starts
                .iter()
                .map(|g| g.coefficient(m - a as i64).map_err(text))
                .collect::<Result<_, _>>()?;
            let moved = mat_vec(&p, &start)?;
            out = out.iter().zip(&moved).map(|(x, y)| x.add(y).map_err(text)).collect::<Result<_, _>>()?;
        }
        self.found.lock().expect("unpoisoned").insert(m, out.clone());
        Ok(out)
    }
}

/// `A^N·x₀` for a count infinite in `scale` itself — `⌈1/(dt·dx)⌉` steps of `1 + dt·dx` — as
/// `exp(N·log A)`: `A` the identity at order `0` in `scale`, `log A = Σ_{k≥1} L_k·u^k` its series
/// ([`log_term`]), and `N·log A` the product of the two expansions, limited in `scale` whenever the
/// steps the count takes add up to a limited amount. Its exponential is taken at `scale`'s orders as
/// any matrix's ([`MatrixExpSpec`]); each component of the end state keeps the power's recipe, which
/// names the update, the count and the start.
fn logarithmic_power(
    scale: &Arc<Scale>,
    a: &NMatrix,
    count: Number,
    counted: Expansion<Number>,
    start: &[Number],
) -> Result<Vec<Number>, String> {
    let n = a.len();
    let (entries, grid) = expansions(a, scale)?;
    if !is_zero_matrix(&mat_add(&term(&entries, 0)?, &mat_scale(&identity(n), &rational(Rational::from_i64(-1)))?)?)? {
        return Err(format!(
            "the update holds {} and changes its state at every step by more than a value of it, so its power is not computed",
            scale.name
        ));
    }
    let starts: Vec<Expansion<Number>> = start
        .iter()
        .map(|x| tower::number_germ(x, scale).map(|g| g.expansion.clone()))
        .collect::<Result<_, _>>()?;
    let q = starts.iter().fold(lcm(grid, counted.ramification()), |l, e| lcm(l, e.ramification()));
    let entries: Vec<Vec<Expansion<Number>>> = entries
        .into_iter()
        .map(|row| row.into_iter().map(|e| e.refined(q / grid)).collect())
        .collect();
    let counted = counted.refined(q / counted.ramification());
    let logs = Arc::new(LogSpec {
        entries: entries.clone(),
        width: n,
        found: Mutex::new(Vec::new()),
    });
    let scaled: Vec<Vec<Expansion<Number>>> = (0..n)
        .map(|i| {
            (0..n)
                .map(|j| counted.mul(&Expansion::source(Arc::new(LogEntry(logs.clone(), i, j)), 1, q)))
                .collect()
        })
        .collect();
    for row in &scaled {
        for e in row {
            for k in e.floor()..0 {
                if !e.coefficient(k).map_err(text)?.is_zero().map_err(text)? {
                    return Err(format!(
                        "this supertask's step count {} outgrows its steps: the change they add up to is infinite in {}",
                        count.display_string(),
                        scale.name
                    ));
                }
            }
        }
    }
    let exponential = Arc::new(MatrixExpSpec {
        a: a.clone(),
        t: Rational::one(),
        scale: scale.clone(),
        entries: scaled,
        found: Mutex::new(Vec::new()),
    });
    let starts: Vec<Expansion<Number>> = starts.into_iter().map(|e| e.refined(q / e.ramification())).collect();
    let spec = Arc::new(PowerSpec {
        a: a.clone(),
        count,
        start: start.to_vec(),
        entries,
        starts: starts.clone(),
        logs: Mutex::new(Vec::new()),
        found: Mutex::new(BTreeMap::new()),
    });
    Ok((0..n)
        .map(|i| {
            let expansion = (0..n).fold(Expansion::constant(Number::zero()), |sum, j| {
                let entry = Expansion::source(Arc::new(MatrixExpEntry(exponential.clone(), i, j)), 0, q);
                sum.add(&entry.mul(&starts[j]))
            });
            Number::hyper(tower::HyperValue {
                value: tower::HyperForm::Nested(Arc::new(NestedGerm::from_source(
                    scale,
                    GermRecipe::Power(spec.clone(), i),
                    expansion,
                ))),
                scale: scale.clone(),
            })
        })
        .collect())
}

/// The series of `log A` for `A` the identity at order `0`, shared by its entries' expansions.
#[derive(Debug)]
struct LogSpec {
    entries: Vec<Vec<Expansion<Number>>>,
    width: usize,
    /// `L_k` for each order `k` found so far, `L_0 = 0`.
    found: Mutex<Vec<NMatrix>>,
}

impl LogSpec {
    fn order(&self, k: usize) -> Result<NMatrix, String> {
        let mut found = self.found.lock().expect("unpoisoned");
        while found.len() <= k {
            let at = found.len();
            let next = if at == 0 {
                zeros(self.width)
            } else {
                log_term(&|step| term(&self.entries, step as i64), self.width, at)?
            };
            found.push(next);
        }
        Ok(found[k].clone())
    }
}

#[derive(Debug)]
struct LogEntry(Arc<LogSpec>, usize, usize);

impl Source<Number> for LogEntry {
    fn coefficient(&self, k: i64) -> Result<Number, ExpansionError> {
        if k < 1 {
            return Ok(Number::zero());
        }
        self.0.order(k as usize).map(|m| m[self.1][self.2].clone()).map_err(ExpansionError::Coefficient)
    }
}

#[derive(Debug)]
struct PowerComponent(Arc<PowerSpec>, usize);

impl Source<Number> for PowerComponent {
    fn coefficient(&self, k: i64) -> Result<Number, ExpansionError> {
        self.0.state(k).map(|s| s[self.1].clone()).map_err(ExpansionError::Coefficient)
    }
}

/// `A^N·x₀` for `A`'s entries values of the infinitesimal `scale` (smaller than the one `N` counts)
/// and larger ones: exactly, by `exp(N·log A)` in `scale`, when `A` is the identity at order `0`; a
/// scalar update otherwise by `exp(N·ln A)`; a matrix update otherwise refused by name.
pub(crate) fn power(scale: &Arc<Scale>, a: &NMatrix, count: Number, start: &[Number]) -> Result<Vec<Number>, String> {
    let counted = tower::number_germ(&count, scale)?.expansion.clone();
    if counted.floor() < 0 {
        return logarithmic_power(scale, a, count, counted, start);
    }
    let n = a.len();
    let (entries, grid) = expansions(a, scale)?;
    let at_zero = term(&entries, 0)?;
    let difference: NMatrix = at_zero
        .iter()
        .zip(identity(n))
        .map(|(x, y)| x.iter().zip(&y).map(|(p, q)| p.sub(q).map_err(text)).collect())
        .collect::<Result<_, String>>()?;
    if !is_zero_matrix(&difference)? {
        if n == 1 {
            let g = tower::number_germ(&a[0][0], scale)?;
            let logarithm = Arc::new(NestedGerm::ln(&g, &a[0][0].display_string())?);
            let times = Arc::new(NestedGerm::mul(&logarithm, &tower::number_germ(&count, scale)?));
            let grown = Arc::new(NestedGerm::exp(&times, &times.render())?);
            let end = NestedGerm::mul(&grown, &tower::number_germ(&start[0], scale)?);
            return Ok(vec![Number::hyper(tower::HyperValue {
                value: tower::HyperForm::Nested(Arc::new(end)),
                scale: scale.clone(),
            })]);
        }
        return Err(format!(
            "the update holds {} and changes its state at every step by more than a value of it, so its power is not computed",
            scale.name
        ));
    }
    let starts: Vec<Expansion<Number>> = start
        .iter()
        .map(|x| tower::number_germ(x, scale).map(|g| g.expansion.clone()))
        .collect::<Result<_, _>>()?;
    let q = starts.iter().fold(grid, |l, e| lcm(l, e.ramification()));
    let entries: Vec<Vec<Expansion<Number>>> = entries
        .into_iter()
        .map(|row| row.into_iter().map(|e| e.refined(q / grid)).collect())
        .collect();
    let starts: Vec<Expansion<Number>> = starts.into_iter().map(|e| e.refined(q / e.ramification())).collect();
    let floor = starts.iter().map(Expansion::floor).min().unwrap_or(0);
    let spec = Arc::new(PowerSpec {
        a: a.clone(),
        count,
        start: start.to_vec(),
        entries,
        starts,
        logs: Mutex::new(Vec::new()),
        found: Mutex::new(BTreeMap::new()),
    });
    Ok((0..n)
        .map(|i| {
            let expansion = Expansion::source(Arc::new(PowerComponent(spec.clone(), i)), floor, q);
            Number::hyper(tower::HyperValue {
                value: tower::HyperForm::Nested(Arc::new(NestedGerm::from_source(
                    scale,
                    GermRecipe::Power(spec.clone(), i),
                    expansion,
                ))),
                scale: scale.clone(),
            })
        })
        .collect())
}
