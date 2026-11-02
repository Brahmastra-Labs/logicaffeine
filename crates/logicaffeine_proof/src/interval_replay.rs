//! Independent range-safety checker for analog netlist claims — the trust core of the analog
//! lane's `RangeCertificate`, in the same spirit as this crate's PR/RUP checker: a small,
//! dependency-free replay the certificate producer cannot influence.
//!
//! The compiler (one crate over) claims per-node voltage bounds for a mapped analog dataflow.
//! This module re-derives those bounds from the declared input boxes by DIRECTED-OUTWARD-ROUNDED
//! `f64` interval arithmetic — after every interval operation each endpoint is bumped one ULP
//! outward (`next_down`/`next_up`), so the replayed interval SOUNDLY contains every exactly-
//! computed value regardless of intermediate rounding — and checks, fail-closed:
//!
//!   1. **Honesty** — the replayed interval of every node is contained in the node's CLAIMED
//!      interval (a claim tighter than reality is rejected);
//!   2. **No clipping** — every claimed interval lies within the usable rail half-span;
//!   3. **No starvation** — every node's claimed full scale clears the noise floor by the
//!      demanded dynamic-range ratio (a signal whose whole range drowns in noise is a
//!      gain-scheduling bug, not a small signal).
//!
//! Any NaN or infinity anywhere — inputs, weights, claims, replay — rejects the certificate.
//! No solver, no bignum, no imports beyond `std`. Trust this file, not the producer.

/// An operand of a replayed cell: a plane input, a prior cell's net, or a programmed constant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RSrc {
    In(usize),
    Val(usize),
    Const(f64),
}

/// The neutral cell vocabulary the checker replays. Mirrors (but does not import) the analog
/// lane's tile set; the producer converts its netlist into this form to be judged.
#[derive(Clone, Debug, PartialEq)]
pub enum RCell {
    /// `v = Σ wᵢ·xᵢ` — weighted Kirchhoff sum.
    Sum {
        inputs: Vec<RSrc>,
        weights: Vec<f64>,
    },
    /// `v = a·b` — four-quadrant multiplier.
    Mul { a: RSrc, b: RSrc },
    /// `v = k·a` — programmed constant gain.
    Gain { a: RSrc, k: f64 },
    /// `v = if c < 0 then t else f` — comparator-driven switch. The replay rule is the
    /// decided branch when the condition's box clears zero, else the branch HULL (the
    /// comparator's ambiguity is an accuracy question for the budget, not a range one).
    Select { c: RSrc, t: RSrc, f: RSrc },
    /// `v = eˣ` — monotone image box (translinear).
    Exp { a: RSrc },
    /// `v = ln x` — monotone; a box touching the nonpositive axis is REJECTED, fail-closed.
    Log { a: RSrc },
    /// `v = tanh x` — monotone.
    Tanh { a: RSrc },
    /// `v = √x` — monotone; a box touching zero is REJECTED (unbounded error slope).
    Sqrt { a: RSrc },
    /// `v = a / b` — the divisor's box must CLEAR zero or the claim is rejected.
    Div { a: RSrc, b: RSrc },
}

/// The claim under judgment: declared math-domain input boxes, per-net power-of-two scales,
/// claimed VOLT-domain per-node bounds, and the physical envelope they must respect.
#[derive(Clone, Debug, PartialEq)]
pub struct RangeClaim {
    /// Declared math-domain input bounds, one per plane input.
    pub input_lo: Vec<f64>,
    pub input_hi: Vec<f64>,
    /// Per-cell power-of-two scale exponent: `volt = 2^scale_log2 · math` (exact in f64).
    pub scale_log2: Vec<i32>,
    /// Claimed volt-domain bounds, one per cell.
    pub node_lo_v: Vec<f64>,
    pub node_hi_v: Vec<f64>,
    /// The usable half-span around mid-rail in volts (headroom already applied by the producer;
    /// this checker takes it as the hard wall).
    pub rail_half_span_v: f64,
    /// The noise floor in volts (`0.0` for an ideal target — disables check 3).
    pub noise_floor_v: f64,
    /// Required ratio of a node's full scale to the noise floor (dynamic-range demand).
    pub min_full_scale_over_noise: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Iv {
    lo: f64,
    hi: f64,
}

impl Iv {
    fn finite(&self) -> bool {
        self.lo.is_finite() && self.hi.is_finite() && self.lo <= self.hi
    }
    /// One-ULP outward bump — the directed-rounding step that makes every op sound.
    fn outward(self) -> Iv {
        Iv {
            lo: self.lo.next_down(),
            hi: self.hi.next_up(),
        }
    }
    fn add(self, o: Iv) -> Iv {
        Iv {
            lo: self.lo + o.lo,
            hi: self.hi + o.hi,
        }
        .outward()
    }
    fn scale(self, w: f64) -> Iv {
        let (a, b) = (w * self.lo, w * self.hi);
        Iv {
            lo: a.min(b),
            hi: a.max(b),
        }
        .outward()
    }
    fn mul(self, o: Iv) -> Iv {
        let ps = [
            self.lo * o.lo,
            self.lo * o.hi,
            self.hi * o.lo,
            self.hi * o.hi,
        ];
        let mut lo = ps[0];
        let mut hi = ps[0];
        for p in ps {
            lo = lo.min(p);
            hi = hi.max(p);
        }
        Iv { lo, hi }.outward()
    }
    fn subset_of(self, outer: Iv) -> bool {
        outer.lo <= self.lo && self.hi <= outer.hi
    }
}

/// Replay the claim and judge it. `true` iff every check passes; ANY malformation (arity
/// mismatch, dangling reference, non-finite number, inverted interval) is `false` — fail-closed.
pub fn check_range_claim(cells: &[RCell], claim: &RangeClaim) -> bool {
    let n_in = claim.input_lo.len();
    if claim.input_hi.len() != n_in
        || claim.scale_log2.len() != cells.len()
        || claim.node_lo_v.len() != cells.len()
        || claim.node_hi_v.len() != cells.len()
        || !claim.rail_half_span_v.is_finite()
        || claim.rail_half_span_v <= 0.0
        || !claim.noise_floor_v.is_finite()
        || claim.noise_floor_v < 0.0
        || !claim.min_full_scale_over_noise.is_finite()
        || claim.min_full_scale_over_noise < 1.0
    {
        return false;
    }

    let inputs: Vec<Iv> = (0..n_in)
        .map(|i| Iv {
            lo: claim.input_lo[i],
            hi: claim.input_hi[i],
        })
        .collect();
    if inputs.iter().any(|iv| !iv.finite()) {
        return false;
    }

    let src = |s: &RSrc, vals: &[Iv]| -> Option<Iv> {
        match s {
            RSrc::In(k) => inputs.get(*k).copied(),
            RSrc::Val(v) => vals.get(*v).copied(),
            RSrc::Const(c) => c.is_finite().then_some(Iv { lo: *c, hi: *c }),
        }
    };

    let mut vals: Vec<Iv> = Vec::with_capacity(cells.len());
    for (i, cell) in cells.iter().enumerate() {
        // Math-domain replay of this node.
        let math = match cell {
            RCell::Sum {
                inputs: xs,
                weights,
            } => {
                if xs.len() != weights.len()
                    || xs.is_empty()
                    || weights.iter().any(|w| !w.is_finite())
                {
                    return false;
                }
                let mut acc = Iv { lo: 0.0, hi: 0.0 };
                for (x, w) in xs.iter().zip(weights) {
                    let Some(v) = src(x, &vals) else { return false };
                    acc = acc.add(v.scale(*w));
                }
                acc
            }
            RCell::Mul { a, b } => {
                let (Some(a), Some(b)) = (src(a, &vals), src(b, &vals)) else {
                    return false;
                };
                a.mul(b)
            }
            RCell::Gain { a, k } => {
                if !k.is_finite() {
                    return false;
                }
                let Some(a) = src(a, &vals) else { return false };
                a.scale(*k)
            }
            RCell::Select { c, t, f } => {
                let (Some(c), Some(t), Some(f)) = (src(c, &vals), src(t, &vals), src(f, &vals))
                else {
                    return false;
                };
                if c.hi < 0.0 {
                    t
                } else if c.lo >= 0.0 {
                    f
                } else {
                    Iv {
                        lo: t.lo.min(f.lo),
                        hi: t.hi.max(f.hi),
                    }
                    .outward()
                }
            }
            RCell::Exp { a } => {
                let Some(a) = src(a, &vals) else { return false };
                Iv {
                    lo: a.lo.exp(),
                    hi: a.hi.exp(),
                }
                .outward()
            }
            RCell::Log { a } => {
                let Some(a) = src(a, &vals) else { return false };
                if a.lo <= 0.0 {
                    return false;
                }
                Iv {
                    lo: a.lo.ln(),
                    hi: a.hi.ln(),
                }
                .outward()
            }
            RCell::Tanh { a } => {
                let Some(a) = src(a, &vals) else { return false };
                Iv {
                    lo: a.lo.tanh(),
                    hi: a.hi.tanh(),
                }
                .outward()
            }
            RCell::Sqrt { a } => {
                let Some(a) = src(a, &vals) else { return false };
                if a.lo <= 0.0 {
                    return false;
                }
                Iv {
                    lo: a.lo.sqrt(),
                    hi: a.hi.sqrt(),
                }
                .outward()
            }
            RCell::Div { a, b } => {
                let (Some(a), Some(b)) = (src(a, &vals), src(b, &vals)) else {
                    return false;
                };
                if b.lo <= 0.0 && b.hi >= 0.0 {
                    return false;
                }
                let ps = [a.lo / b.lo, a.lo / b.hi, a.hi / b.lo, a.hi / b.hi];
                let mut lo = ps[0];
                let mut hi = ps[0];
                for p in ps {
                    lo = lo.min(p);
                    hi = hi.max(p);
                }
                Iv { lo, hi }.outward()
            }
        };
        if !math.finite() {
            return false;
        }

        // Volt domain: multiply by the exact power-of-two scale.
        let s = claim.scale_log2[i];
        if !(-1000..=1000).contains(&s) {
            return false;
        }
        let volt = math.scale(f64::powi(2.0, s));
        if !volt.finite() {
            return false;
        }

        let claimed = Iv {
            lo: claim.node_lo_v[i],
            hi: claim.node_hi_v[i],
        };
        if !claimed.finite() {
            return false;
        }
        // 1. Honesty: the replayed bounds fit inside the claim.
        if !volt.subset_of(claimed) {
            return false;
        }
        // 2. No clipping: the claim fits inside the usable rail half-span.
        let wall = Iv {
            lo: -claim.rail_half_span_v,
            hi: claim.rail_half_span_v,
        };
        if !claimed.subset_of(wall) {
            return false;
        }
        // 3. No starvation: the claimed full scale clears the noise floor.
        if claim.noise_floor_v > 0.0 {
            let full_scale = claimed.lo.abs().max(claimed.hi.abs());
            if full_scale < claim.noise_floor_v * claim.min_full_scale_over_noise {
                return false;
            }
        }

        vals.push(math);
    }
    true
}

/// Replay a STREAMED-REDUCTION claim: the same cells run once per frame with input 0 the
/// element box and input 1 the carried accumulator (starting at `init`); the claim's per-node
/// bounds must contain the HULL of every frame's replayed bounds, the claim's input-1 box must
/// contain the whole accumulator trajectory, and every claimed bound must respect the rails and
/// the noise floor. Fail-closed on any malformation, divergence, or a frame count of zero.
pub fn check_reduction_claim(
    cells: &[RCell],
    out: RSrc,
    init: f64,
    frames: usize,
    claim: &RangeClaim,
) -> bool {
    if frames == 0 || !init.is_finite() || claim.input_lo.len() != 2 || claim.input_hi.len() != 2 {
        return false;
    }
    // Static shape/rail/noise judgment via the DAG checker on the FIRST frame's inputs is not
    // sufficient — iterate, hull, then judge the hull against the claim.
    let elem = Iv {
        lo: claim.input_lo[0],
        hi: claim.input_hi[0],
    };
    let acc_claim = Iv {
        lo: claim.input_lo[1],
        hi: claim.input_hi[1],
    };
    if !elem.finite() || !acc_claim.finite() {
        return false;
    }
    let mut acc = Iv { lo: init, hi: init };
    let mut hulls: Vec<Iv> = vec![
        Iv {
            lo: f64::INFINITY,
            hi: f64::NEG_INFINITY
        };
        cells.len()
    ];
    for _ in 0..frames {
        let inputs = [elem, acc];
        let src = |s: &RSrc, vals: &[Iv]| -> Option<Iv> {
            match s {
                RSrc::In(k) => inputs.get(*k).copied(),
                RSrc::Val(v) => vals.get(*v).copied(),
                RSrc::Const(c) => c.is_finite().then_some(Iv { lo: *c, hi: *c }),
            }
        };
        let mut vals: Vec<Iv> = Vec::with_capacity(cells.len());
        for cell in cells {
            let v = match cell {
                RCell::Sum {
                    inputs: xs,
                    weights,
                } => {
                    if xs.len() != weights.len()
                        || xs.is_empty()
                        || weights.iter().any(|w| !w.is_finite())
                    {
                        return false;
                    }
                    let mut a = Iv { lo: 0.0, hi: 0.0 };
                    for (x, w) in xs.iter().zip(weights) {
                        let Some(iv) = src(x, &vals) else {
                            return false;
                        };
                        a = a.add(iv.scale(*w));
                    }
                    a
                }
                RCell::Mul { a, b } => {
                    let (Some(a), Some(b)) = (src(a, &vals), src(b, &vals)) else {
                        return false;
                    };
                    a.mul(b)
                }
                RCell::Gain { a, k } => {
                    if !k.is_finite() {
                        return false;
                    }
                    let Some(a) = src(a, &vals) else { return false };
                    a.scale(*k)
                }
                RCell::Select { c, t, f } => {
                    let (Some(c), Some(t), Some(f)) = (src(c, &vals), src(t, &vals), src(f, &vals))
                    else {
                        return false;
                    };
                    if c.hi < 0.0 {
                        t
                    } else if c.lo >= 0.0 {
                        f
                    } else {
                        Iv {
                            lo: t.lo.min(f.lo),
                            hi: t.hi.max(f.hi),
                        }
                        .outward()
                    }
                }
                RCell::Exp { a } => {
                    let Some(a) = src(a, &vals) else { return false };
                    Iv {
                        lo: a.lo.exp(),
                        hi: a.hi.exp(),
                    }
                    .outward()
                }
                RCell::Log { a } => {
                    let Some(a) = src(a, &vals) else { return false };
                    if a.lo <= 0.0 {
                        return false;
                    }
                    Iv {
                        lo: a.lo.ln(),
                        hi: a.hi.ln(),
                    }
                    .outward()
                }
                RCell::Tanh { a } => {
                    let Some(a) = src(a, &vals) else { return false };
                    Iv {
                        lo: a.lo.tanh(),
                        hi: a.hi.tanh(),
                    }
                    .outward()
                }
                RCell::Sqrt { a } => {
                    let Some(a) = src(a, &vals) else { return false };
                    if a.lo <= 0.0 {
                        return false;
                    }
                    Iv {
                        lo: a.lo.sqrt(),
                        hi: a.hi.sqrt(),
                    }
                    .outward()
                }
                RCell::Div { a, b } => {
                    let (Some(a), Some(b)) = (src(a, &vals), src(b, &vals)) else {
                        return false;
                    };
                    if b.lo <= 0.0 && b.hi >= 0.0 {
                        return false;
                    }
                    let ps = [a.lo / b.lo, a.lo / b.hi, a.hi / b.lo, a.hi / b.hi];
                    let mut lo = ps[0];
                    let mut hi = ps[0];
                    for p in ps {
                        lo = lo.min(p);
                        hi = hi.max(p);
                    }
                    Iv { lo, hi }.outward()
                }
            };
            if !v.finite() {
                return false;
            }
            vals.push(v);
        }
        for (h, v) in hulls.iter_mut().zip(&vals) {
            h.lo = h.lo.min(v.lo);
            h.hi = h.hi.max(v.hi);
        }
        let Some(next) = src(&out, &vals) else {
            return false;
        };
        if !next.finite() {
            return false;
        }
        acc = next;
        // The whole trajectory must live inside the claimed accumulator box.
        if !acc.subset_of(acc_claim) {
            return false;
        }
    }
    // Judge the hulls exactly as the DAG checker judges per-node bounds.
    if hulls.len() != claim.node_lo_v.len()
        || hulls.len() != claim.node_hi_v.len()
        || hulls.len() != claim.scale_log2.len()
    {
        return false;
    }
    let wall = Iv {
        lo: -claim.rail_half_span_v,
        hi: claim.rail_half_span_v,
    };
    if !claim.rail_half_span_v.is_finite() || claim.rail_half_span_v <= 0.0 {
        return false;
    }
    for (i, h) in hulls.iter().enumerate() {
        let h = if h.lo > h.hi {
            Iv { lo: 0.0, hi: 0.0 }
        } else {
            *h
        };
        let s = claim.scale_log2[i];
        if !(-1000..=1000).contains(&s) {
            return false;
        }
        let volt = h.scale(f64::powi(2.0, s));
        let claimed = Iv {
            lo: claim.node_lo_v[i],
            hi: claim.node_hi_v[i],
        };
        if !volt.finite()
            || !claimed.finite()
            || !volt.subset_of(claimed)
            || !claimed.subset_of(wall)
        {
            return false;
        }
        if claim.noise_floor_v > 0.0 {
            let full_scale = claimed.lo.abs().max(claimed.hi.abs());
            if full_scale < claim.noise_floor_v * claim.min_full_scale_over_noise {
                return false;
            }
        }
    }
    true
}

/// Replay a MULTI-STATE INTEGRATOR claim: the cells run once per frame over the held state
/// vector (starting inside the `init` boxes); `outs[s]` gives state `s`'s next value. The
/// claim's input boxes must contain every frame's state vector (the trajectory hull), and the
/// per-node claims must contain the per-frame hulls, inside the rails / above the noise floor.
pub fn check_integrator_claim(
    cells: &[RCell],
    outs: &[RSrc],
    init_lo: &[f64],
    init_hi: &[f64],
    frames: usize,
    claim: &RangeClaim,
) -> bool {
    let n_state = outs.len();
    if frames == 0
        || n_state == 0
        || init_lo.len() != n_state
        || init_hi.len() != n_state
        || claim.input_lo.len() != n_state
        || claim.input_hi.len() != n_state
    {
        return false;
    }
    let mut state: Vec<Iv> = (0..n_state)
        .map(|s| Iv {
            lo: init_lo[s],
            hi: init_hi[s],
        })
        .collect();
    if state.iter().any(|iv| !iv.finite()) {
        return false;
    }
    let state_claim: Vec<Iv> = (0..n_state)
        .map(|s| Iv {
            lo: claim.input_lo[s],
            hi: claim.input_hi[s],
        })
        .collect();
    if state_claim.iter().any(|iv| !iv.finite()) {
        return false;
    }
    // The initial boxes themselves must sit inside the claimed hull.
    if state
        .iter()
        .zip(&state_claim)
        .any(|(s, c)| !s.subset_of(*c))
    {
        return false;
    }
    let mut hulls: Vec<Iv> = vec![
        Iv {
            lo: f64::INFINITY,
            hi: f64::NEG_INFINITY
        };
        cells.len()
    ];
    for _ in 0..frames {
        let src = |s: &RSrc, vals: &[Iv]| -> Option<Iv> {
            match s {
                RSrc::In(k) => state.get(*k).copied(),
                RSrc::Val(v) => vals.get(*v).copied(),
                RSrc::Const(c) => c.is_finite().then_some(Iv { lo: *c, hi: *c }),
            }
        };
        let mut vals: Vec<Iv> = Vec::with_capacity(cells.len());
        for cell in cells {
            let v = match cell {
                RCell::Sum {
                    inputs: xs,
                    weights,
                } => {
                    if xs.len() != weights.len()
                        || xs.is_empty()
                        || weights.iter().any(|w| !w.is_finite())
                    {
                        return false;
                    }
                    let mut a = Iv { lo: 0.0, hi: 0.0 };
                    for (x, w) in xs.iter().zip(weights) {
                        let Some(iv) = src(x, &vals) else {
                            return false;
                        };
                        a = a.add(iv.scale(*w));
                    }
                    a
                }
                RCell::Mul { a, b } => {
                    let (Some(a), Some(b)) = (src(a, &vals), src(b, &vals)) else {
                        return false;
                    };
                    a.mul(b)
                }
                RCell::Gain { a, k } => {
                    if !k.is_finite() {
                        return false;
                    }
                    let Some(a) = src(a, &vals) else { return false };
                    a.scale(*k)
                }
                RCell::Select { c, t, f } => {
                    let (Some(c), Some(t), Some(f)) = (src(c, &vals), src(t, &vals), src(f, &vals))
                    else {
                        return false;
                    };
                    if c.hi < 0.0 {
                        t
                    } else if c.lo >= 0.0 {
                        f
                    } else {
                        Iv {
                            lo: t.lo.min(f.lo),
                            hi: t.hi.max(f.hi),
                        }
                        .outward()
                    }
                }
                RCell::Exp { a } => {
                    let Some(a) = src(a, &vals) else { return false };
                    Iv {
                        lo: a.lo.exp(),
                        hi: a.hi.exp(),
                    }
                    .outward()
                }
                RCell::Log { a } => {
                    let Some(a) = src(a, &vals) else { return false };
                    if a.lo <= 0.0 {
                        return false;
                    }
                    Iv {
                        lo: a.lo.ln(),
                        hi: a.hi.ln(),
                    }
                    .outward()
                }
                RCell::Tanh { a } => {
                    let Some(a) = src(a, &vals) else { return false };
                    Iv {
                        lo: a.lo.tanh(),
                        hi: a.hi.tanh(),
                    }
                    .outward()
                }
                RCell::Sqrt { a } => {
                    let Some(a) = src(a, &vals) else { return false };
                    if a.lo <= 0.0 {
                        return false;
                    }
                    Iv {
                        lo: a.lo.sqrt(),
                        hi: a.hi.sqrt(),
                    }
                    .outward()
                }
                RCell::Div { a, b } => {
                    let (Some(a), Some(b)) = (src(a, &vals), src(b, &vals)) else {
                        return false;
                    };
                    if b.lo <= 0.0 && b.hi >= 0.0 {
                        return false;
                    }
                    let ps = [a.lo / b.lo, a.lo / b.hi, a.hi / b.lo, a.hi / b.hi];
                    let mut lo = ps[0];
                    let mut hi = ps[0];
                    for p in ps {
                        lo = lo.min(p);
                        hi = hi.max(p);
                    }
                    Iv { lo, hi }.outward()
                }
            };
            if !v.finite() {
                return false;
            }
            vals.push(v);
        }
        for (h, v) in hulls.iter_mut().zip(&vals) {
            h.lo = h.lo.min(v.lo);
            h.hi = h.hi.max(v.hi);
        }
        let mut next = Vec::with_capacity(n_state);
        for o in outs {
            let Some(v) = src(o, &vals) else { return false };
            if !v.finite() {
                return false;
            }
            next.push(v);
        }
        state = next;
        if state
            .iter()
            .zip(&state_claim)
            .any(|(s, c)| !s.subset_of(*c))
        {
            return false; // the trajectory left the claimed hull
        }
    }
    if hulls.len() != claim.node_lo_v.len()
        || hulls.len() != claim.node_hi_v.len()
        || hulls.len() != claim.scale_log2.len()
    {
        return false;
    }
    if !claim.rail_half_span_v.is_finite() || claim.rail_half_span_v <= 0.0 {
        return false;
    }
    let wall = Iv {
        lo: -claim.rail_half_span_v,
        hi: claim.rail_half_span_v,
    };
    for (i, h) in hulls.iter().enumerate() {
        let h = if h.lo > h.hi {
            Iv { lo: 0.0, hi: 0.0 }
        } else {
            *h
        };
        let s = claim.scale_log2[i];
        if !(-1000..=1000).contains(&s) {
            return false;
        }
        let volt = h.scale(f64::powi(2.0, s));
        let claimed = Iv {
            lo: claim.node_lo_v[i],
            hi: claim.node_hi_v[i],
        };
        if !volt.finite()
            || !claimed.finite()
            || !volt.subset_of(claimed)
            || !claimed.subset_of(wall)
        {
            return false;
        }
        if claim.noise_floor_v > 0.0 {
            let full_scale = claimed.lo.abs().max(claimed.hi.abs());
            if full_scale < claim.noise_floor_v * claim.min_full_scale_over_noise {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/interval_replay/tests.rs"]
mod tests;
