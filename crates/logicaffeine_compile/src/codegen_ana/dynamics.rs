//! The relaxation lane — continuous-time state evolution as a compute primitive
//! (`Solve A·x = b` by settling).
//!
//! The four-fabric thesis, mechanized at its first rung: the SAME Sum tiles that compute
//! results in the dataflow lane here generate FORCES — `ẋ = b − A·x` is one weighted-sum tile
//! per state variable, and capacitor state integrates it. Gradient flow on the strongly convex
//! quadratic `½xᵀAx − bᵀx` converges for admissible `A` (symmetric + positive definite;
//! Hirsch–Smale, gradient systems), and the admission gate PROVES membership over exact ℚ —
//! strict diagonal dominance with a positive diagonal (Gershgorin ⇒ SPD for symmetric `A`) or
//! all leading principal minors positive (Sylvester's criterion). Anything else refuses TYPED.
//!
//! The trust covenant: the settle (here, a deterministic explicit-Euler behavioral model with
//! an optional fixed-seed noise injection — the future chip replaces it wholesale) is
//! UNTRUSTED. Only the settled answer counts: the readout is ADC-quantized on the target's
//! converter grid, the exact residual `r = A·x̂ − b` is computed in ℚ, and the emitted
//! [`RelaxClaim`] is judged by the INDEPENDENT `logicaffeine_proof::relax_replay` checker.
//! Settling time is a performance knob, not a correctness one: an under-settled run exceeds
//! the demanded budget and refuses — it cannot emit a certificate it does not meet.

use logicaffeine_base::numeric::{rational_from_f64_exact, Rational};
use logicaffeine_proof::relax_replay::{IsingClaim, MinimizeClaim, RelaxClaim};

use super::cell::{ASrc, AnalogCell};
use super::target::AnalogTarget;
use super::AnaLowerError;

/// A configured dynamical system: one force tile per state variable, built from the EXISTING
/// tile vocabulary (`AnalogCell::Sum` — the tiles stop computing results and start generating
/// the forces that move state).
#[derive(Clone, Debug, PartialEq)]
pub struct DynSystem {
    pub n: usize,
    /// `force[i]` computes `ẋ_i = b_i − Σ_j a_ij·x_j`; state taps are `ASrc::In(j)`, the
    /// constant leg carries `b_i`.
    pub force: Vec<AnalogCell>,
    pub init: Vec<f64>,
}

/// Build the gradient-flow force network for `ẋ = b − A·x`. Purely structural — admission is
/// [`solve_by_settling`]'s job.
pub fn gradient_flow_system(a: &[Vec<f64>], b: &[f64]) -> DynSystem {
    let n = a.len();
    let force = (0..n)
        .map(|i| {
            let mut inputs: Vec<ASrc> = (0..n).map(ASrc::In).collect();
            let mut weights: Vec<f64> = a[i].iter().map(|w| -w).collect();
            inputs.push(ASrc::Const(1.0));
            weights.push(b[i]);
            AnalogCell::Sum { inputs, weights }
        })
        .collect();
    DynSystem {
        n,
        force,
        init: vec![0.0; n],
    }
}

/// The deterministic settle knobs. `eps_required` is the demanded certificate budget — the
/// hard admission line for the emitted claim; `noise_amp`/`seed` inject a fixed-seed
/// disturbance per step (the model's stand-in for the physical noise floor, and R3's future
/// annealing ramp).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SettleOpts {
    pub steps: usize,
    pub rate: f64,
    pub eps_required: f64,
    pub noise_amp: f64,
    pub seed: u64,
}

/// One settled, certified solve.
#[derive(Clone, Debug, PartialEq)]
pub struct RelaxOutcome {
    /// The ADC-quantized readout (math domain).
    pub x_hat: Vec<f64>,
    pub steps: usize,
    /// The neutral exact claim, judged by `logicaffeine_proof::relax_replay::check_relax_claim`.
    pub claim: RelaxClaim,
}

/// `Solve A·x = b` by settling the gradient-flow system, fail-closed end to end.
pub fn solve_by_settling(
    a: &[Vec<f64>],
    b: &[f64],
    target: &AnalogTarget,
    opts: &SettleOpts,
) -> Result<RelaxOutcome, AnaLowerError> {
    let aq = exact_matrix(a)?;
    let n = aq.len();
    if b.len() != n {
        return Err(AnaLowerError::ConvergenceUnprovable {
            reason: format!("shape mismatch: A is {n}×{n} but b has {} entries", b.len()),
        });
    }
    admit(&aq)?;

    // The untrusted settle: explicit Euler over the force network, fixed-seed noise optional.
    let sys = gradient_flow_system(a, b);
    let mut x = sys.init.clone();
    let mut noise_state = opts.seed ^ 0x5E77_1E5A_1755_A17E;
    for _ in 0..opts.steps {
        let forces: Vec<f64> = sys.force.iter().map(|cell| cell.eval(&x, &[])).collect();
        for i in 0..n {
            x[i] += opts.rate * forces[i];
            if opts.noise_amp > 0.0 {
                x[i] += opts.noise_amp * centered_uniform(&mut noise_state);
            }
        }
    }

    // Readout through the converter grid at unit block scale — quantization lands honestly
    // in the residual.
    let half_span = target.half_span_v();
    let adc_lsb = 2.0 * half_span / f64::powi(2.0, i32::from(target.adc_bits));
    let x_hat: Vec<f64> = x.iter().map(|v| (v / adc_lsb).round() * adc_lsb).collect();

    // The exact residual and the claim. Every f64 is finite here or conversion refuses.
    let bq = exact_vec(b)?;
    let xq = exact_vec(&x_hat)?;
    let mut linf = Rational::zero();
    for i in 0..n {
        let mut acc = bq[i].negated();
        for j in 0..n {
            acc = acc.add(&aq[i][j].mul(&xq[j]));
        }
        let mag = acc.abs();
        if mag > linf {
            linf = mag;
        }
    }
    let linf_f = linf.to_f64();
    if !(linf_f <= opts.eps_required) {
        return Err(AnaLowerError::ResidualExceedsBudget {
            linf: linf_f,
            required: opts.eps_required,
        });
    }
    Ok(RelaxOutcome {
        x_hat,
        steps: opts.steps,
        claim: RelaxClaim {
            a: aq,
            b: bq,
            x_hat: xq,
            eps: linf,
        },
    })
}

/// One settled, certified quadratic minimization.
#[derive(Clone, Debug, PartialEq)]
pub struct MinimizeOutcome {
    pub x_hat: Vec<f64>,
    pub steps: usize,
    /// The theorem-checked claim (`proof::relax_replay::check_minimize_claim`).
    pub claim: MinimizeClaim,
    /// f64 view of the exact suboptimality bound.
    pub subopt: f64,
}

/// `minimize ½xᵀQx + bᵀx` by settling `ẋ = −(Q·x + b)` — the SAME force network with the
/// signs flipped (the tiles generate the negative gradient). The admission gate is R1's; the
/// CERTIFICATE is R2's theorem: for spectrum ≥ μ > 0 (exact Gershgorin row bound over ℚ),
/// `f(x̂) − f* ≤ ‖∇f(x̂)‖²/(2μ)` — a gradient residual becomes a distance-to-optimum bound.
/// `Candidate` semantics: the bound is the claim; no optimality field exists to lie with.
pub fn minimize_by_settling(
    q: &[Vec<f64>],
    b: &[f64],
    target: &AnalogTarget,
    opts: &SettleOpts,
) -> Result<MinimizeOutcome, AnaLowerError> {
    let qq = exact_matrix(q)?;
    let n = qq.len();
    if b.len() != n {
        return Err(AnaLowerError::ConvergenceUnprovable {
            reason: format!("shape mismatch: Q is {n}×{n} but b has {} entries", b.len()),
        });
    }
    admit(&qq)?;
    // The certificate needs strict Gershgorin positivity — a Sylvester-only admission has no
    // exact spectral lower bound to certify with.
    let mut mu: Option<Rational> = None;
    for i in 0..n {
        let off = (0..n)
            .filter(|j| *j != i)
            .fold(Rational::zero(), |acc, j| acc.add(&qq[i][j].abs()));
        let row = qq[i][i].sub(&off);
        mu = Some(match mu {
            None => row,
            Some(m) => {
                if row < m {
                    row
                } else {
                    m
                }
            }
        });
    }
    let mu = mu.expect("n > 0");
    if !mu.is_positive() {
        return Err(AnaLowerError::ConvergenceUnprovable {
            reason: "no certified spectral floor: Q is not strictly diagonally dominant, so \
                     the Gershgorin bound gives no positive μ for the suboptimality theorem"
                .to_string(),
        });
    }

    // Settle the negative gradient flow: reuse the linear machinery with b' = −b.
    let neg_b: Vec<f64> = b.iter().map(|x| -x).collect();
    let sys = gradient_flow_system(q, &neg_b);
    let mut x = sys.init.clone();
    let mut noise_state = opts.seed ^ 0x5E77_1E5A_1755_A17E;
    for _ in 0..opts.steps {
        let forces: Vec<f64> = sys.force.iter().map(|cell| cell.eval(&x, &[])).collect();
        for i in 0..n {
            x[i] += opts.rate * forces[i];
            if opts.noise_amp > 0.0 {
                x[i] += opts.noise_amp * centered_uniform(&mut noise_state);
            }
        }
    }
    let half_span = target.half_span_v();
    let adc_lsb = 2.0 * half_span / f64::powi(2.0, i32::from(target.adc_bits));
    let x_hat: Vec<f64> = x.iter().map(|v| (v / adc_lsb).round() * adc_lsb).collect();

    // The exact gradient and the exact bound.
    let bq = exact_vec(b)?;
    let xq = exact_vec(&x_hat)?;
    let mut norm2 = Rational::zero();
    for i in 0..n {
        let mut g = bq[i].clone();
        for j in 0..n {
            g = g.add(&qq[i][j].mul(&xq[j]));
        }
        norm2 = norm2.add(&g.mul(&g));
    }
    let subopt = norm2.div(&Rational::from_i64(2).mul(&mu)).expect("mu > 0");
    let subopt_f = subopt.to_f64();
    if !(subopt_f <= opts.eps_required) {
        return Err(AnaLowerError::ResidualExceedsBudget {
            linf: subopt_f,
            required: opts.eps_required,
        });
    }
    Ok(MinimizeOutcome {
        x_hat,
        steps: opts.steps,
        subopt: subopt_f,
        claim: MinimizeClaim {
            q: qq,
            b: bq,
            x_hat: xq,
            mu,
            subopt,
        },
    })
}

/// One Ising candidate: an attractor of the soft-spin flow, certified for WHAT IT IS —
/// its exact energy and its exact 1-flip stability — and nothing more (the claim type has
/// no optimality field to lie with).
#[derive(Clone, Debug, PartialEq)]
pub struct IsingOutcome {
    pub spins: Vec<i8>,
    pub steps: usize,
    pub claim: IsingClaim,
    /// f64 view of the exact energy.
    pub energy: f64,
    pub flip_stable: bool,
}

/// The R3 attempt: `minimize H(s) = −Σ J_ij·s_i·s_j − Σ h_i·s_i` by SOFT-SPIN relaxation
/// (Hopfield 1984): continuous states `x ∈ [−1,1]ⁿ` follow `ẋ = −x + tanh(β·(J·x + h))`
/// with the annealing schedule as a NOISE-AMPLITUDE RAMP (`opts.noise_amp` decaying linearly
/// to zero — the noise floor as a resource) while β ramps up. The trajectory and the tanh
/// live in the UNTRUSTED model tier; the readout `sign(x̂)` is certified digitally and
/// exactly: recomputed energy and 1-flip stability, judged by
/// `proof::relax_replay::check_ising_claim`. A candidate that is not even 1-flip stable is
/// polished greedily (each flip strictly lowers H — a certified descent, not a search).
pub fn ising_by_soft_spin(
    j: &[Vec<f64>],
    h: &[f64],
    opts: &SettleOpts,
    beta_max: f64,
) -> Result<IsingOutcome, AnaLowerError> {
    let jq = exact_matrix(j)?;
    let n = jq.len();
    if h.len() != n {
        return Err(AnaLowerError::ConvergenceUnprovable {
            reason: format!("shape mismatch: J is {n}×{n} but h has {} entries", h.len()),
        });
    }
    for i in 0..n {
        if !jq[i][i].is_zero() {
            return Err(AnaLowerError::ConvergenceUnprovable {
                reason: format!("nonzero self-coupling J[{i}][{i}] — not an Ising instance"),
            });
        }
        for k in (i + 1)..n {
            if jq[i][k] != jq[k][i] {
                return Err(AnaLowerError::ConvergenceUnprovable {
                    reason: format!("asymmetric coupling at ({i},{k})"),
                });
            }
        }
    }

    // The untrusted soft-spin settle with the annealing ramp.
    let mut x = vec![0.0_f64; n];
    let mut noise_state = opts.seed ^ 0x1516_C0DE_5EED_0151;
    for step in 0..opts.steps {
        let progress = step as f64 / opts.steps.max(1) as f64;
        let beta = beta_max * (0.1 + 0.9 * progress);
        let amp = opts.noise_amp * (1.0 - progress);
        for i in 0..n {
            let mut field = h[i];
            for k in 0..n {
                field += j[i][k] * x[k];
            }
            let dx = -x[i] + (beta * field).tanh();
            x[i] += opts.rate * dx;
            if amp > 0.0 {
                x[i] += amp * centered_uniform(&mut noise_state);
            }
            x[i] = x[i].clamp(-1.0, 1.0);
        }
    }
    let mut spins: Vec<i8> = x.iter().map(|v| if *v < 0.0 { -1 } else { 1 }).collect();

    // Certified greedy polish: flip any spin whose exact ΔH is negative until none is —
    // every step strictly lowers H, so this terminates and lands 1-flip stable.
    let hq = exact_vec(h)?;
    loop {
        let mut flipped = false;
        for i in 0..n {
            let mut field = hq[i].clone();
            for k in 0..n {
                if k != i {
                    field = field.add(&jq[i][k].mul(&Rational::from_i64(spins[k] as i64)));
                }
            }
            let delta = Rational::from_i64(2 * spins[i] as i64).mul(&field);
            if delta.is_negative() {
                spins[i] = -spins[i];
                flipped = true;
            }
        }
        if !flipped {
            break;
        }
    }

    // The exact energy of what we actually return.
    let s_of = |i: usize| Rational::from_i64(spins[i] as i64);
    let mut energy = Rational::zero();
    for i in 0..n {
        for k in (i + 1)..n {
            energy = energy.sub(&jq[i][k].mul(&s_of(i)).mul(&s_of(k)));
        }
        energy = energy.sub(&hq[i].mul(&s_of(i)));
    }
    let energy_f = energy.to_f64();
    let claim = IsingClaim {
        j: jq,
        h: hq,
        spins: spins.clone(),
        energy,
        flip_stable: true,
    };
    Ok(IsingOutcome {
        spins,
        steps: opts.steps,
        claim,
        energy: energy_f,
        flip_stable: true,
    })
}

/// Fold one soft equality constraint `gᵀx = c` into the quadratic as a virtual spring:
/// `f(x) + λ(gᵀx − c)²` ⇒ `Q += 2λ·ggᵀ`, `b −= 2λc·g`. Stiffer springs pull the settled
/// answer closer to the constraint surface — the quadratic-penalty form (primal-dual
/// dynamics are the ledgered future).
pub fn with_penalty(
    q: &[Vec<f64>],
    b: &[f64],
    g: &[f64],
    c: f64,
    lambda: f64,
) -> (Vec<Vec<f64>>, Vec<f64>) {
    let n = q.len();
    let mut q2: Vec<Vec<f64>> = q.iter().map(|r| r.to_vec()).collect();
    let mut b2 = b.to_vec();
    for i in 0..n {
        for j in 0..n {
            q2[i][j] += 2.0 * lambda * g[i] * g[j];
        }
        b2[i] -= 2.0 * lambda * c * g[i];
    }
    (q2, b2)
}

/// The admission gate, over EXACT ℚ: symmetric AND (strictly diagonally dominant with a
/// positive diagonal, OR all leading principal minors positive). Cited theorems: Gershgorin
/// (dominance ⇒ definiteness for symmetric matrices), Sylvester (minors), and gradient-flow
/// convergence on strongly convex quadratics.
fn admit(a: &[Vec<Rational>]) -> Result<(), AnaLowerError> {
    let n = a.len();
    let refuse = |reason: String| Err(AnaLowerError::ConvergenceUnprovable { reason });
    if n == 0 {
        return refuse("empty system".to_string());
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if a[i][j] != a[j][i] {
                return refuse(format!(
                    "A is not symmetric at ({i},{j}) — gradient flow solves ∇(½xᵀAx − bᵀx) = 0, \
                     which is only A·x = b for symmetric A"
                ));
            }
        }
    }
    let dominant = (0..n).all(|i| {
        let off: Rational = (0..n)
            .filter(|j| *j != i)
            .fold(Rational::zero(), |acc, j| acc.add(&a[i][j].abs()));
        a[i][i].is_positive() && a[i][i] > off
    });
    if dominant {
        return Ok(());
    }
    if (1..=n).all(|k| leading_minor_det(a, k).is_positive()) {
        return Ok(());
    }
    refuse(
        "A is symmetric but neither strictly diagonally dominant (positive diagonal) nor \
         Sylvester-positive (all leading principal minors > 0) — convergence unproven"
            .to_string(),
    )
}

/// Determinant of the k×k leading principal minor by exact fraction Gaussian elimination.
fn leading_minor_det(a: &[Vec<Rational>], k: usize) -> Rational {
    let mut m: Vec<Vec<Rational>> = (0..k).map(|i| a[i][..k].to_vec()).collect();
    let mut det = Rational::one();
    for col in 0..k {
        let Some(p) = (col..k).find(|r| !m[*r][col].is_zero()) else {
            return Rational::zero();
        };
        if p != col {
            m.swap(p, col);
            det = det.negated();
        }
        det = det.mul(&m[col][col]);
        let pivot = m[col][col].clone();
        for r in (col + 1)..k {
            let factor = m[r][col].div(&pivot).expect("pivot is nonzero");
            for c in col..k {
                let sub = factor.mul(&m[col][c]);
                m[r][c] = m[r][c].sub(&sub);
            }
        }
    }
    det
}

fn exact_matrix(a: &[Vec<f64>]) -> Result<Vec<Vec<Rational>>, AnaLowerError> {
    let n = a.len();
    a.iter()
        .map(|row| {
            if row.len() != n {
                return Err(AnaLowerError::ConvergenceUnprovable {
                    reason: format!(
                        "ragged matrix: a row has {} entries in an n={n} system",
                        row.len()
                    ),
                });
            }
            exact_vec(row)
        })
        .collect()
}

fn exact_vec(v: &[f64]) -> Result<Vec<Rational>, AnaLowerError> {
    v.iter()
        .map(|x| {
            rational_from_f64_exact(*x).ok_or_else(|| AnaLowerError::ConvergenceUnprovable {
                reason: format!("non-finite entry {x}"),
            })
        })
        .collect()
}

/// SplitMix64-driven uniform in (−1, 1) — deterministic forever under a fixed seed.
fn centered_uniform(state: &mut u64) -> f64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 11) as f64 + 0.5) / (1u64 << 52) as f64 - 1.0
}
