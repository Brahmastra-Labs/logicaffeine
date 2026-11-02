//! Settled-answer replay — the residual certificate for physics-solved linear systems.
//!
//! The trust covenant: the physical solver (or its behavioral model) is UNTRUSTED by
//! construction — no trajectory, no convergence argument, no producer arithmetic is believed.
//! The claim is neutral data: `A`, `b`, the settled readout `x̂`, and a declared ∞-norm bound
//! `ε` — all exact rationals. [`check_relax_claim`] re-multiplies `r = A·x̂ − b` with THIS
//! crate's own exact arithmetic ([`crate::linalg::matvec`]) and demands `‖r‖∞ ≤ ε`,
//! component by component. Fail-closed: a malformed shape, a negative ε, or an empty system
//! is a rejection, never a pass.
//!
//! The same mathematics will judge the thermal map (`G·ΔT = P`) — one residual checker,
//! two physical planes.

use logicaffeine_base::numeric::Rational;

use crate::linalg::matvec;

/// The neutral claim a settling run emits. Everything exact; nothing trusted.
#[derive(Clone, Debug, PartialEq)]
pub struct RelaxClaim {
    /// The system matrix, row-major, n×n.
    pub a: Vec<Vec<Rational>>,
    /// The right-hand side, length n.
    pub b: Vec<Rational>,
    /// The settled readout, length n.
    pub x_hat: Vec<Rational>,
    /// The declared bound: `‖A·x̂ − b‖∞ ≤ eps`.
    pub eps: Rational,
}

/// The neutral claim a convex-quadratic settle emits: `minimize ½xᵀQx + bᵀx`, the settled
/// readout, a claimed strong-convexity parameter, and a claimed suboptimality bound.
/// Everything exact; nothing trusted.
#[derive(Clone, Debug, PartialEq)]
pub struct MinimizeClaim {
    pub q: Vec<Vec<Rational>>,
    pub b: Vec<Rational>,
    pub x_hat: Vec<Rational>,
    /// Claimed lower bound on Q's spectrum (strong-convexity parameter), > 0.
    pub mu: Rational,
    /// Claimed bound: `f(x̂) − f* ≤ subopt`.
    pub subopt: Rational,
}

/// Judge a minimize claim by THEOREM, not trust: for symmetric Q with spectrum ≥ μ > 0
/// (verified against the exact Gershgorin row bound, which this checker recomputes itself),
/// strong convexity gives `f(x̂) − f* ≤ ‖∇f(x̂)‖² / (2μ)` — the gradient is re-derived with
/// this crate's own arithmetic and the inequality checked exactly. Fail-closed throughout.
pub fn check_minimize_claim(claim: &MinimizeClaim) -> bool {
    let n = claim.q.len();
    if n == 0 || claim.b.len() != n || claim.x_hat.len() != n {
        return false;
    }
    if claim.q.iter().any(|row| row.len() != n) {
        return false;
    }
    if !claim.mu.is_positive() || claim.subopt.is_negative() {
        return false;
    }
    // Symmetry, and the Gershgorin row bound: min_i (q_ii − Σ_{j≠i} |q_ij|) ≥ μ.
    for i in 0..n {
        for j in (i + 1)..n {
            if claim.q[i][j] != claim.q[j][i] {
                return false;
            }
        }
        let off = (0..n)
            .filter(|j| *j != i)
            .fold(Rational::zero(), |acc, j| acc.add(&claim.q[i][j].abs()));
        if claim.q[i][i].sub(&off) < claim.mu {
            return false;
        }
    }
    // ∇f(x̂) = Q·x̂ + b, re-multiplied here; then ‖∇‖² ≤ 2μ·subopt exactly.
    let qx = matvec(&claim.q, &claim.x_hat);
    let mut norm2 = Rational::zero();
    for i in 0..n {
        let g = qx[i].add(&claim.b[i]);
        norm2 = norm2.add(&g.mul(&g));
    }
    let two_mu_subopt = Rational::from_i64(2).mul(&claim.mu).mul(&claim.subopt);
    norm2 <= two_mu_subopt
}

/// The Ising candidate's neutral claim: the instance, the returned spins, the claimed exact
/// energy, and the claimed 1-flip stability verdict. BY DESIGN there is no optimality field
/// — a soft-spin settle returns an attractor, and this type cannot express more.
#[derive(Clone, Debug, PartialEq)]
pub struct IsingClaim {
    /// Symmetric coupling matrix with zero diagonal.
    pub j: Vec<Vec<Rational>>,
    /// External fields.
    pub h: Vec<Rational>,
    /// The candidate spins, each exactly ±1.
    pub spins: Vec<i8>,
    /// Claimed `H(s) = −Σ_{i<j} J_ij·s_i·s_j − Σ h_i·s_i`, exact.
    pub energy: Rational,
    /// Claimed: no single spin flip lowers H.
    pub flip_stable: bool,
}

/// Judge an Ising candidate: recompute the exact energy and the exact 1-flip stability with
/// this crate's own arithmetic and demand agreement with BOTH claims. Fail-closed on shape
/// garbage (asymmetry, nonzero diagonal, non-±1 spins).
pub fn check_ising_claim(claim: &IsingClaim) -> bool {
    let n = claim.j.len();
    if n == 0 || claim.h.len() != n || claim.spins.len() != n {
        return false;
    }
    if claim.j.iter().any(|row| row.len() != n) {
        return false;
    }
    for i in 0..n {
        if !claim.j[i][i].is_zero() {
            return false;
        }
        for jx in (i + 1)..n {
            if claim.j[i][jx] != claim.j[jx][i] {
                return false;
            }
        }
    }
    if claim.spins.iter().any(|s| *s != 1 && *s != -1) {
        return false;
    }
    let s_of = |i: usize| Rational::from_i64(claim.spins[i] as i64);

    // Exact energy.
    let mut energy = Rational::zero();
    for i in 0..n {
        for jx in (i + 1)..n {
            energy = energy.sub(&claim.j[i][jx].mul(&s_of(i)).mul(&s_of(jx)));
        }
        energy = energy.sub(&claim.h[i].mul(&s_of(i)));
    }
    if energy != claim.energy {
        return false;
    }

    // Exact 1-flip stability: flipping spin i changes H by ΔH_i = 2·s_i·(Σ_j J_ij·s_j + h_i);
    // the candidate is flip-stable iff every ΔH_i ≥ 0.
    let mut stable = true;
    for i in 0..n {
        let mut field = claim.h[i].clone();
        for jx in 0..n {
            if jx != i {
                field = field.add(&claim.j[i][jx].mul(&s_of(jx)));
            }
        }
        let delta = Rational::from_i64(2).mul(&s_of(i)).mul(&field);
        if delta.is_negative() {
            stable = false;
            break;
        }
    }
    stable == claim.flip_stable
}

/// THE judge. Recomputes the residual with this crate's own exact multiplication and checks
/// the declared bound. Rejects malformed claims outright.
pub fn check_relax_claim(claim: &RelaxClaim) -> bool {
    let n = claim.a.len();
    if n == 0 || claim.b.len() != n || claim.x_hat.len() != n {
        return false;
    }
    if claim.a.iter().any(|row| row.len() != n) {
        return false;
    }
    if claim.eps.is_negative() {
        return false;
    }
    let ax = matvec(&claim.a, &claim.x_hat);
    ax.iter()
        .zip(&claim.b)
        .all(|(axi, bi)| axi.sub(bi).abs() <= claim.eps)
}
