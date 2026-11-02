//! Non-ideality-aware placement — operand adjacency as a matching resource.
//!
//! The Pelgrom distance gradient makes a tile's gain error grow with its operand separation,
//! so placement is not cosmetic: a sensitivity-blind scatter of a deep chain accumulates
//! distance-gain error the budget must then carry. The aware placer greedily seats each tile
//! at the free site minimizing pitch distance to its operand tiles (deterministic tie-break:
//! row-major). `place_blind` is the deliberate straw man — maximum-stride scatter — kept
//! public so the anti-vacuity lock can prove the cost function earns its keep.

use super::cell::{ASrc, AnalogCell, AnalogNetlist};
use super::target::{AnaUnplaceable, AnalogTarget};
use super::AnaLowerError;

/// Per-cell site assignment on the target's tile array.
#[derive(Clone, Debug, PartialEq)]
pub struct AnalogPlacement {
    pub site: Vec<(usize, usize)>,
}

impl AnalogPlacement {
    /// Manhattan pitch distance between two placed cells.
    pub fn pitch_distance(&self, a: usize, b: usize) -> f64 {
        let (ar, ac) = self.site[a];
        let (br, bc) = self.site[b];
        (ar.abs_diff(br) + ac.abs_diff(bc)) as f64
    }

    /// Total operand-distance cost (the objective the aware placer minimizes).
    pub fn wire_cost(&self, netlist: &AnalogNetlist) -> f64 {
        let mut cost = 0.0;
        for (i, cell) in netlist.cells.iter().enumerate() {
            let mut add = |s: &ASrc| {
                if let ASrc::Val(v) = s {
                    cost += self.pitch_distance(i, *v);
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

/// Greedy operand-adjacent placement: seat cells in SSA order, each at the free site with the
/// smallest total pitch distance to its already-placed operands (row-major tie-break —
/// deterministic, reproducible).
pub fn place_analog(
    netlist: &AnalogNetlist,
    target: &AnalogTarget,
) -> Result<AnalogPlacement, AnaLowerError> {
    let sites_total = target.rows * target.cols;
    if netlist.cells.len() > sites_total {
        return Err(AnaLowerError::Unplaceable(AnaUnplaceable::TooSmall {
            needed_cells: netlist.cells.len(),
            target_rows: target.rows,
            target_cols: target.cols,
            target: target.name,
        }));
    }
    let mut used = vec![false; sites_total];
    let mut site: Vec<(usize, usize)> = Vec::with_capacity(netlist.cells.len());
    for (i, cell) in netlist.cells.iter().enumerate() {
        let operands: Vec<usize> = operand_cells(cell);
        let mut best: Option<(f64, usize)> = None;
        for s in 0..sites_total {
            if used[s] {
                continue;
            }
            let (r, c) = (s / target.cols, s % target.cols);
            let d: f64 = operands
                .iter()
                .map(|o| {
                    let (or, oc) = site[*o];
                    (r.abs_diff(or) + c.abs_diff(oc)) as f64
                })
                .sum();
            if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, s));
            }
        }
        let (_, s) = best.expect("a free site exists (counted above)");
        used[s] = true;
        site.push((s / target.cols, s % target.cols));
        let _ = i;
    }
    Ok(AnalogPlacement { site })
}

/// The deliberate straw man: maximum-stride scatter (successive cells thrown to opposite
/// corners), used by the anti-vacuity lock to prove aware placement pays.
pub fn place_blind(
    netlist: &AnalogNetlist,
    target: &AnalogTarget,
) -> Result<AnalogPlacement, AnaLowerError> {
    let sites_total = target.rows * target.cols;
    if netlist.cells.len() > sites_total {
        return Err(AnaLowerError::Unplaceable(AnaUnplaceable::TooSmall {
            needed_cells: netlist.cells.len(),
            target_rows: target.rows,
            target_cols: target.cols,
            target: target.name,
        }));
    }
    // Walk sites with a large coprime stride so consecutive cells land far apart.
    let stride = (sites_total / 2).max(1) | 1;
    let mut used = vec![false; sites_total];
    let mut cursor = 0usize;
    let mut site = Vec::with_capacity(netlist.cells.len());
    for _ in &netlist.cells {
        while used[cursor] {
            cursor = (cursor + 1) % sites_total;
        }
        used[cursor] = true;
        site.push((cursor / target.cols, cursor % target.cols));
        cursor = (cursor + stride) % sites_total;
    }
    Ok(AnalogPlacement { site })
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
