//! The decision core for linear integer arithmetic: Fourier-Motzkin elimination
//! that tracks the non-negative combination it eliminates with, so an
//! unsatisfiable system yields its **Farkas certificate** — the multipliers `λᵢ ≥ 0`
//! on the original constraints such that `Σ λᵢ·constraintᵢ` is a positive constant
//! `≤ 0` (a contradiction). The proof engine reconstructs a kernel proof from those
//! multipliers via `le_mul_nonneg`/`le_add_mono` + the Bool no-confusion
//! discriminator. Pure and self-contained — no kernel, no certificates here.

use std::collections::BTreeMap;

use crate::ProofTerm;

/// A linear expression `Σ cⱼ·xⱼ + constant` over the integers (zero coefficients
/// pruned, so equality is canonical).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinExpr {
    pub coeffs: BTreeMap<String, i64>,
    pub constant: i64,
}

impl LinExpr {
    fn constant(c: i64) -> Self {
        LinExpr {
            coeffs: BTreeMap::new(),
            constant: c,
        }
    }
    fn var(name: &str) -> Self {
        let mut coeffs = BTreeMap::new();
        coeffs.insert(name.to_string(), 1);
        LinExpr {
            coeffs,
            constant: 0,
        }
    }
    fn prune(mut self) -> Self {
        self.coeffs.retain(|_, c| *c != 0);
        self
    }
    fn add(&self, o: &Self) -> Self {
        let mut coeffs = self.coeffs.clone();
        for (k, v) in &o.coeffs {
            *coeffs.entry(k.clone()).or_insert(0) += v;
        }
        LinExpr {
            coeffs,
            constant: self.constant + o.constant,
        }
        .prune()
    }
    fn neg(&self) -> Self {
        LinExpr {
            coeffs: self.coeffs.iter().map(|(k, v)| (k.clone(), -v)).collect(),
            constant: -self.constant,
        }
    }
    pub fn sub(&self, o: &Self) -> Self {
        self.add(&o.neg())
    }
    /// True when this is a bare integer constant (no variables remain).
    pub fn is_const(&self) -> bool {
        self.coeffs.is_empty()
    }
    fn scale(&self, k: i64) -> Self {
        LinExpr {
            coeffs: self
                .coeffs
                .iter()
                .map(|(x, v)| (x.clone(), v * k))
                .collect(),
            constant: self.constant * k,
        }
        .prune()
    }
    fn is_constant(&self) -> bool {
        self.coeffs.is_empty()
    }
    fn coeff(&self, v: &str) -> i64 {
        self.coeffs.get(v).copied().unwrap_or(0)
    }
}

/// Parse a [`ProofTerm`] into a linear expression, or `None` if it is non-linear
/// (e.g. `mul` of two non-constants). Numeric constants are literals; other names
/// are variables; `add`/`sub`/`mul` build the form.
pub fn parse_lin(t: &ProofTerm) -> Option<LinExpr> {
    match t {
        ProofTerm::Constant(s) => Some(match s.parse::<i64>() {
            Ok(n) => LinExpr::constant(n),
            Err(_) => LinExpr::var(s),
        }),
        ProofTerm::Variable(s) | ProofTerm::BoundVarRef(s) => Some(LinExpr::var(s)),
        ProofTerm::Function(name, args) => match (name.as_str(), args.as_slice()) {
            ("add", [a, b]) => Some(parse_lin(a)?.add(&parse_lin(b)?)),
            ("sub", [a, b]) => Some(parse_lin(a)?.sub(&parse_lin(b)?)),
            ("mul", [a, b]) => {
                let (la, lb) = (parse_lin(a)?, parse_lin(b)?);
                if la.is_constant() {
                    Some(lb.scale(la.constant))
                } else if lb.is_constant() {
                    Some(la.scale(lb.constant))
                } else {
                    None
                }
            }
            _ => None,
        },
        ProofTerm::Group(_) => None,
    }
}

/// A row of the elimination: the constraint `e ≤ 0`, tagged with the non-negative
/// combination `prov` (original-constraint index → multiplier) that produced it.
#[derive(Clone, Debug)]
struct Row {
    e: LinExpr,
    prov: BTreeMap<usize, i64>,
}

fn combine_prov(
    a: &BTreeMap<usize, i64>,
    ka: i64,
    b: &BTreeMap<usize, i64>,
    kb: i64,
) -> BTreeMap<usize, i64> {
    let mut out = BTreeMap::new();
    for (i, v) in a {
        *out.entry(*i).or_insert(0) += v * ka;
    }
    for (i, v) in b {
        *out.entry(*i).or_insert(0) += v * kb;
    }
    out.retain(|_, v| *v != 0);
    out
}

/// Given constraints `cᵢ ≤ 0`, find a non-negative integer combination
/// `Σ λᵢ·cᵢ = (positive constant) ≤ 0` — a Farkas refutation. Returns the
/// multipliers `λᵢ`, or `None` if the system is satisfiable over ℚ (so no rational
/// Farkas certificate exists).
pub fn find_farkas(constraints: &[LinExpr]) -> Option<BTreeMap<usize, i64>> {
    let mut rows: Vec<Row> = constraints
        .iter()
        .enumerate()
        .map(|(i, e)| Row {
            e: e.clone(),
            prov: BTreeMap::from([(i, 1)]),
        })
        .collect();

    let contradiction = |rows: &[Row]| -> Option<BTreeMap<usize, i64>> {
        rows.iter()
            .find(|r| r.e.is_constant() && r.e.constant > 0)
            .map(|r| r.prov.clone())
    };
    if let Some(p) = contradiction(&rows) {
        return Some(p);
    }

    // Eliminate variables one at a time (Fourier-Motzkin).
    let mut vars: Vec<String> = rows
        .iter()
        .flat_map(|r| r.e.coeffs.keys().cloned())
        .collect();
    vars.sort();
    vars.dedup();

    for v in vars {
        let (mut pos, mut neg, mut zero) = (Vec::new(), Vec::new(), Vec::new());
        for r in rows {
            match r.e.coeff(&v) {
                c if c > 0 => pos.push(r),
                c if c < 0 => neg.push(r),
                _ => zero.push(r),
            }
        }
        let mut next = zero;
        for p in &pos {
            for n in &neg {
                let (pc, nc) = (p.e.coeff(&v), -n.e.coeff(&v)); // both > 0
                                                                // nc·p + pc·n  cancels v (coeff: nc·pc + pc·(-nc) = 0).
                next.push(Row {
                    e: p.e.scale(nc).add(&n.e.scale(pc)),
                    prov: combine_prov(&p.prov, nc, &n.prov, pc),
                });
            }
        }
        rows = next;
        if let Some(p) = contradiction(&rows) {
            return Some(p);
        }
    }
    contradiction(&rows)
}

/// `Σ multipliers[i]·constraints[i]`. For a valid Farkas certificate this is a
/// bare positive constant (the variables cancel) — that constant is the `d` in the
/// reconstructed contradiction `0 ≤ -d`.
pub fn combine(constraints: &[LinExpr], multipliers: &BTreeMap<usize, i64>) -> LinExpr {
    let mut acc = LinExpr::constant(0);
    for (&i, &m) in multipliers {
        acc = acc.add(&constraints[i].scale(m));
    }
    acc
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/linarith_solve/tests.rs"]
mod tests;
