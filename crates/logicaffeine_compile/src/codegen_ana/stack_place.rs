//! 3D placement over composed analog tiers — vertical interconnect as a COUNTED, refusable
//! resource, and the cross-tier Pelgrom category priced into the budget.
//!
//! The 2D placer's greedy operand adjacency generalizes: sites now span every analog tier,
//! the cost of an operand edge is lateral pitches plus `hop_cost_pitches` per crossed gap,
//! and each crossing CONSUMES one via from each crossed gap's budget — running out is a typed
//! [`AnaUnplaceable::VerticalResourceExhausted`], never an overcommit. The budget composes
//! through the SAME engine as 2D (`nonideal_budget_with`), fed the full separation law: a
//! cross-tier leg pays the flat categorical ceiling
//! ([`Separation::CrossTier`]), which is how the budget itself teaches the placer to keep
//! matched clusters in-tier. `place_stack_blind` is the deliberate straw man the anti-vacuity
//! lock beats three ways (wire, ε, vias).

use super::cell::{ASrc, AnalogCell, AnalogNetlist};
use super::error_budget::{nonideal_budget_with, OutputBudget};
use super::gain::{GainSchedule, RangeCertificate};
use super::nonideal::{leg_operand, separation_sigma, NonIdealityModel, Separation};
use super::stack::{StackTarget, TierFabric};
use super::target::{AnaUnplaceable, AnalogTarget};
use super::AnaLowerError;

/// A physical site in the stack: which STACK tier, and where on its array.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StackSite {
    pub tier: usize,
    pub row: usize,
    pub col: usize,
}

/// Per-cell site assignment plus the per-gap via ledger.
#[derive(Clone, Debug, PartialEq)]
pub struct StackPlacement {
    pub site: Vec<StackSite>,
    /// `vertical_used[g]` = crossings consumed from gap `g`'s budget by operand edges.
    pub vertical_used: Vec<usize>,
}

impl StackPlacement {
    /// The Pelgrom-law separation between two placed cells.
    pub fn separation(&self, a: usize, b: usize) -> Separation {
        let (sa, sb) = (self.site[a], self.site[b]);
        if sa.tier == sb.tier {
            Separation::SameTier {
                pitches: (sa.row.abs_diff(sb.row) + sa.col.abs_diff(sb.col)) as f64,
            }
        } else {
            Separation::CrossTier
        }
    }

    /// Total operand cost: lateral pitches plus `hop_cost_pitches` per crossed gap.
    pub fn wire_cost(&self, netlist: &AnalogNetlist, hop_cost_pitches: f64) -> f64 {
        let mut cost = 0.0;
        for (i, cell) in netlist.cells.iter().enumerate() {
            let mut add = |s: &ASrc| {
                if let ASrc::Val(v) = s {
                    let (a, b) = (self.site[i], self.site[*v]);
                    cost += (a.row.abs_diff(b.row) + a.col.abs_diff(b.col)) as f64
                        + hop_cost_pitches * a.tier.abs_diff(b.tier) as f64;
                }
            };
            match cell {
                AnalogCell::Sum { inputs, .. } => inputs.iter().for_each(&mut add),
                AnalogCell::Mul { a, b } => {
                    add(a);
                    add(b);
                }
                AnalogCell::GainConst { a, .. } => add(a),
                AnalogCell::Select { c, t, f } => {
                    add(c);
                    add(t);
                    add(f);
                }
                AnalogCell::Exp { a }
                | AnalogCell::Log { a }
                | AnalogCell::Tanh { a }
                | AnalogCell::Sqrt { a } => add(a),
                AnalogCell::Div { a, b } => {
                    add(a);
                    add(b);
                }
            }
        }
        cost
    }
}

struct SiteSpace {
    /// Every analog-tier site, tier-major then row-major — the deterministic scan order.
    sites: Vec<StackSite>,
    /// Per-gap via budgets (summed over the gap's classes).
    gap_budget: Vec<usize>,
}

fn site_space(stack: &StackTarget) -> SiteSpace {
    let mut sites = Vec::new();
    for (ti, tier) in stack.tiers.iter().enumerate() {
        if let TierFabric::Analog(a) = &tier.fabric {
            for r in 0..a.rows {
                for c in 0..a.cols {
                    sites.push(StackSite {
                        tier: ti,
                        row: r,
                        col: c,
                    });
                }
            }
        }
    }
    let gap_budget = stack
        .gaps
        .iter()
        .map(|g| g.iter().map(|v| v.count).sum())
        .collect();
    SiteSpace { sites, gap_budget }
}

/// The gaps an edge between stack tiers `a` and `b` crosses.
fn crossed_gaps(a: usize, b: usize) -> std::ops::Range<usize> {
    a.min(b)..a.max(b)
}

fn edge_crossings(
    site: &[StackSite],
    operands: &[usize],
    candidate: StackSite,
    n_gaps: usize,
) -> Vec<usize> {
    let mut per_gap = vec![0usize; n_gaps];
    for o in operands {
        for g in crossed_gaps(candidate.tier, site[*o].tier) {
            per_gap[g] += 1;
        }
    }
    per_gap
}

pub(crate) fn operand_cells_of(cell: &AnalogCell) -> Vec<usize> {
    operand_cells(cell)
}

fn operand_cells(cell: &AnalogCell) -> Vec<usize> {
    let mut out = Vec::new();
    let mut add = |s: &ASrc| {
        if let ASrc::Val(v) = s {
            out.push(*v);
        }
    };
    match cell {
        AnalogCell::Sum { inputs, .. } => inputs.iter().for_each(&mut add),
        AnalogCell::Mul { a, b } => {
            add(a);
            add(b);
        }
        AnalogCell::GainConst { a, .. } => add(a),
        AnalogCell::Select { c, t, f } => {
            add(c);
            add(t);
            add(f);
        }
        AnalogCell::Exp { a }
        | AnalogCell::Log { a }
        | AnalogCell::Tanh { a }
        | AnalogCell::Sqrt { a } => add(a),
        AnalogCell::Div { a, b } => {
            add(a);
            add(b);
        }
    }
    out
}

/// Greedy operand-adjacent 3D placement: seat cells in SSA order at the FEASIBLE free site
/// minimizing lateral-plus-hop cost to the placed operands (tier-major row-major tie-break —
/// deterministic forever). A cell with no feasible site refuses on the blocking gap.
pub fn place_stack(
    netlist: &AnalogNetlist,
    stack: &StackTarget,
    hop_cost_pitches: f64,
) -> Result<StackPlacement, AnaLowerError> {
    if let Err(e) = stack.validate() {
        return Err(AnaLowerError::UnloweredOp(format!(
            "malformed stack: {e:?}"
        )));
    }
    let space = site_space(stack);
    if netlist.cells.len() > space.sites.len() {
        return Err(AnaLowerError::Unplaceable(AnaUnplaceable::StackTooSmall {
            needed_cells: netlist.cells.len(),
            sites: space.sites.len(),
            stack: stack.name,
        }));
    }
    let mut used = vec![false; space.sites.len()];
    let mut vertical_used = vec![0usize; space.gap_budget.len()];
    let mut site: Vec<StackSite> = Vec::with_capacity(netlist.cells.len());
    for cell in &netlist.cells {
        let operands = operand_cells(cell);
        let mut best: Option<(f64, usize, Vec<usize>)> = None;
        let mut blocking_gap: Option<usize> = None;
        for (si, cand) in space.sites.iter().enumerate() {
            if used[si] {
                continue;
            }
            let need = edge_crossings(&site, &operands, *cand, vertical_used.len());
            if let Some(g) =
                (0..need.len()).find(|g| vertical_used[*g] + need[*g] > space.gap_budget[*g])
            {
                if blocking_gap.is_none() {
                    blocking_gap = Some(g);
                }
                continue;
            }
            let d: f64 = operands
                .iter()
                .map(|o| {
                    let s = site[*o];
                    (cand.row.abs_diff(s.row) + cand.col.abs_diff(s.col)) as f64
                        + hop_cost_pitches * cand.tier.abs_diff(s.tier) as f64
                })
                .sum();
            if best.as_ref().map(|(bd, _, _)| d < *bd).unwrap_or(true) {
                best = Some((d, si, need));
            }
        }
        let Some((_, si, need)) = best else {
            let gap = blocking_gap.unwrap_or(0);
            return Err(AnaLowerError::Unplaceable(
                AnaUnplaceable::VerticalResourceExhausted {
                    gap,
                    budget: space.gap_budget.get(gap).copied().unwrap_or(0),
                },
            ));
        };
        used[si] = true;
        for (g, n) in need.iter().enumerate() {
            vertical_used[g] += n;
        }
        site.push(space.sites[si]);
    }
    Ok(StackPlacement {
        site,
        vertical_used,
    })
}

/// The deliberate straw man: max-stride scatter over the tier-major site list, adjacency- and
/// hop-blind (it still refuses rather than overcommit a gap — legality is non-negotiable).
pub fn place_stack_blind(
    netlist: &AnalogNetlist,
    stack: &StackTarget,
    _hop_cost_pitches: f64,
) -> Result<StackPlacement, AnaLowerError> {
    if let Err(e) = stack.validate() {
        return Err(AnaLowerError::UnloweredOp(format!(
            "malformed stack: {e:?}"
        )));
    }
    let space = site_space(stack);
    if netlist.cells.len() > space.sites.len() {
        return Err(AnaLowerError::Unplaceable(AnaUnplaceable::StackTooSmall {
            needed_cells: netlist.cells.len(),
            sites: space.sites.len(),
            stack: stack.name,
        }));
    }
    let total = space.sites.len();
    let stride = (total / 2).max(1) | 1;
    let mut used = vec![false; total];
    let mut cursor = 0usize;
    let mut site = Vec::with_capacity(netlist.cells.len());
    for _ in &netlist.cells {
        while used[cursor] {
            cursor = (cursor + 1) % total;
        }
        used[cursor] = true;
        site.push(space.sites[cursor]);
        cursor = (cursor + stride) % total;
    }
    let mut vertical_used = vec![0usize; space.gap_budget.len()];
    for (i, cell) in netlist.cells.iter().enumerate() {
        for o in operand_cells(cell) {
            for g in crossed_gaps(site[i].tier, site[o].tier) {
                vertical_used[g] += 1;
            }
        }
    }
    if let Some(g) = (0..vertical_used.len()).find(|g| vertical_used[*g] > space.gap_budget[*g]) {
        return Err(AnaLowerError::Unplaceable(
            AnaUnplaceable::VerticalResourceExhausted {
                gap: g,
                budget: space.gap_budget[g],
            },
        ));
    }
    Ok(StackPlacement {
        site,
        vertical_used,
    })
}

/// The thermally-coupled stacked budget: the stacked budget PLUS the drift terms priced
/// against the CERTIFIED thermal map — offset drifts with each tile's absolute rise
/// (`offset_tempco · ΔT`), gain with the tile-to-operand GRADIENT
/// (`gain_tempco · |ΔT_cell − ΔT_operand|`) — matching cares about gradients. One Picard
/// pass, deterministic: placement + power → temperature → ε; hot placement provably costs ε
/// (`thermal_cooking_lock` measures it). The ℚ→f64 bridge rounds outward.
pub fn error_budget_thermal_stacked(
    netlist: &AnalogNetlist,
    cert: &RangeCertificate,
    schedule: &GainSchedule,
    target: &AnalogTarget,
    model: &NonIdealityModel,
    placement: &StackPlacement,
    map: &super::thermal::ThermalMap,
    stack: &StackTarget,
    opts: &super::thermal::ThermalOpts,
) -> OutputBudget {
    use super::error_budget::{bump_add, bump_mul, outward_sum, ErrorSource, ErrorTerm};

    let base = error_budget_nonideal_stacked(netlist, cert, schedule, target, model, placement);
    let s = f64::powi(2.0, schedule.scale_log2);
    let q = &schedule.quantized;
    let bound_of = |src: &ASrc| -> super::gain::MathBound {
        match src {
            ASrc::In(k) => cert.input_bounds[*k],
            ASrc::Val(v) => cert.per_net[*v].math,
            ASrc::Const(c) => super::gain::MathBound { lo: *c, hi: *c },
        }
    };
    // Outward ℚ→f64 bridge: a cell's rise is its coarse thermal node's ΔT, rounded UP.
    let (gr, gc) = opts.analog_grid;
    let tile_dt = |cell: usize| -> f64 {
        let site = placement.site[cell];
        let TierFabric::Analog(a) = &stack.tiers[site.tier].fabric else {
            return 0.0;
        };
        let mut node = 0usize;
        for (ti, tier) in stack.tiers.iter().enumerate() {
            if ti == site.tier {
                let r = (site.row * gr / a.rows.max(1)).min(gr - 1);
                let c = (site.col * gc / a.cols.max(1)).min(gc - 1);
                node += r * gc + c;
                break;
            }
            node += match tier.fabric {
                TierFabric::Analog(_) => gr * gc,
                _ => 1,
            };
        }
        map.claim.delta_t[node].to_f64().next_up()
    };

    let mut gain_d = Vec::with_capacity(q.cells.len());
    let mut off_d = Vec::with_capacity(q.cells.len());
    for (i, cell) in q.cells.iter().enumerate() {
        let dt_i = tile_dt(i);
        let grad_of = |leg: usize| -> f64 {
            match leg_operand(&q.cells, i, leg) {
                Some(v) => (dt_i - tile_dt(v)).abs().next_up(),
                None => 0.0,
            }
        };
        let g = match cell {
            AnalogCell::Sum { inputs, weights } => {
                outward_sum(inputs.iter().zip(weights).enumerate().map(|(l, (x, w))| {
                    bump_mul(
                        model.gain_tempco_per_k * grad_of(l),
                        w.abs() * bound_of(x).max_abs(),
                    )
                }))
            }
            AnalogCell::GainConst { a, k } => bump_mul(
                model.gain_tempco_per_k * grad_of(0),
                k.abs() * bound_of(a).max_abs(),
            ),
            AnalogCell::Mul { a, b } => bump_mul(
                model.gain_tempco_per_k * grad_of(0),
                bound_of(a).max_abs() * bound_of(b).max_abs(),
            ),
            AnalogCell::Select { .. } => 0.0,
            AnalogCell::Exp { a }
            | AnalogCell::Log { a }
            | AnalogCell::Tanh { a }
            | AnalogCell::Sqrt { a } => {
                bump_mul(model.gain_tempco_per_k * grad_of(0), bound_of(a).max_abs())
            }
            AnalogCell::Div { a, b } => bump_mul(
                model.gain_tempco_per_k * grad_of(0),
                bound_of(a).max_abs().max(bound_of(b).max_abs()),
            ),
        };
        gain_d.push(g);
        off_d.push(bump_add(0.0, model.offset_tempco_v_per_k * dt_i / s));
    }

    let mut terms = base.terms;
    let tg = super::error_budget::propagate_injected_pub(q, &bound_of, &gain_d);
    if tg > 0.0 {
        terms.push(ErrorTerm {
            eps_math: tg,
            source: ErrorSource::ThermalGain,
        });
    }
    let to = super::error_budget::propagate_injected_pub(q, &bound_of, &off_d);
    if to > 0.0 {
        terms.push(ErrorTerm {
            eps_math: to,
            source: ErrorSource::ThermalOffset,
        });
    }
    let eps_math = outward_sum(terms.iter().map(|t| t.eps_math));
    OutputBudget { eps_math, terms }
}

/// The stacked physical budget: the SAME composition engine as 2D, fed the full separation
/// law — cross-tier legs pay the categorical ceiling, so hot scattering provably costs ε.
pub fn error_budget_nonideal_stacked(
    netlist: &AnalogNetlist,
    cert: &RangeCertificate,
    schedule: &GainSchedule,
    target: &AnalogTarget,
    model: &NonIdealityModel,
    placement: &StackPlacement,
) -> OutputBudget {
    let cells = schedule.quantized.cells.clone();
    let sigma_of = move |i: usize, l: usize| {
        let sep = leg_operand(&cells, i, l)
            .map(|v| placement.separation(i, v))
            .unwrap_or(Separation::SameTier { pitches: 0.0 });
        separation_sigma(model, sep)
    };
    nonideal_budget_with(netlist, cert, schedule, target, model, &sigma_of)
}
