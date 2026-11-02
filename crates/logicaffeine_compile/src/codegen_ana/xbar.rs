//! Crossbar canonicalization — folding a gain/sum tree into single Kirchhoff columns.
//!
//! `Sum` is linear, so a `GainConst` feeding a `Sum` input is the same node with the weight
//! product (`w·(k·x) = (w·k)·x` in ℝ), and a `Sum` feeding a `Sum` splices its addends with
//! distributed weights. Driven to fixpoint, an unrolled constant-weight dot product collapses to
//! ONE `Sum { inputs, weights }` — one physical current-summing node whose programmed
//! conductances ARE the weight vector: the crossbar column (`I = V·G`).
//!
//! The fold RE-ASSOCIATES float arithmetic, so the folded netlist's ideal evaluation is no
//! longer bit-identical to the VM's op order — it is equivalent in ℝ and compared through
//! `logos_approx_eq` / the error budget (which carries an explicit f64-rounding term). Callers
//! choose: the unfolded netlist for the bit-exact ideal tier, the folded one for the physical
//! crossbar shape. Both evaluate with the same [`super::eval`] entry points.

use super::cell::{ASrc, AnalogCell, AnalogNetlist};

/// Fold to fixpoint, prune, then TREE the columns back down to the physical summing fan-in:
/// a folded column wider than `max_fanin` splits into balanced sub-sums (weights ride on the
/// leaves; the joining nodes take unit weights), because fan-in is a physical resource — the
/// summing node's loading, settling, and noise all grow with it.
pub fn fold_crossbar_for(netlist: &AnalogNetlist, max_fanin: usize) -> AnalogNetlist {
    assert!(max_fanin >= 2, "a summing node needs at least two inputs");
    let folded = fold_crossbar(netlist);
    let mut cells: Vec<AnalogCell> = Vec::new();
    let mut remap: Vec<ASrc> = Vec::with_capacity(folded.cells.len());
    for cell in &folded.cells {
        let placed = match cell {
            AnalogCell::Sum { inputs, weights } => {
                let leaves: Vec<(ASrc, f64)> = inputs
                    .iter()
                    .zip(weights)
                    .map(|(x, w)| (relabel(x, &remap), *w))
                    .collect();
                tree_sum(&leaves, max_fanin, &mut cells)
            }
            AnalogCell::Mul { a, b } => {
                let c = AnalogCell::Mul {
                    a: relabel(a, &remap),
                    b: relabel(b, &remap),
                };
                cells.push(c);
                ASrc::Val(cells.len() - 1)
            }
            AnalogCell::GainConst { a, k } => {
                let c = AnalogCell::GainConst {
                    a: relabel(a, &remap),
                    k: *k,
                };
                cells.push(c);
                ASrc::Val(cells.len() - 1)
            }
            AnalogCell::Select { c, t, f } => {
                let cell = AnalogCell::Select {
                    c: relabel(c, &remap),
                    t: relabel(t, &remap),
                    f: relabel(f, &remap),
                };
                cells.push(cell);
                ASrc::Val(cells.len() - 1)
            }
            AnalogCell::Exp { a } => {
                let cell = AnalogCell::Exp {
                    a: relabel(a, &remap),
                };
                cells.push(cell);
                ASrc::Val(cells.len() - 1)
            }
            AnalogCell::Log { a } => {
                let cell = AnalogCell::Log {
                    a: relabel(a, &remap),
                };
                cells.push(cell);
                ASrc::Val(cells.len() - 1)
            }
            AnalogCell::Tanh { a } => {
                let cell = AnalogCell::Tanh {
                    a: relabel(a, &remap),
                };
                cells.push(cell);
                ASrc::Val(cells.len() - 1)
            }
            AnalogCell::Sqrt { a } => {
                let cell = AnalogCell::Sqrt {
                    a: relabel(a, &remap),
                };
                cells.push(cell);
                ASrc::Val(cells.len() - 1)
            }
            AnalogCell::Div { a, b } => {
                let cell = AnalogCell::Div {
                    a: relabel(a, &remap),
                    b: relabel(b, &remap),
                };
                cells.push(cell);
                ASrc::Val(cells.len() - 1)
            }
        };
        remap.push(placed);
    }
    let out = relabel(&folded.out, &remap);
    AnalogNetlist {
        n_in: folded.n_in,
        cells,
        out,
    }
}

fn relabel(s: &ASrc, remap: &[ASrc]) -> ASrc {
    match s {
        ASrc::Val(v) => remap[*v],
        other => *other,
    }
}

/// Emit a ≤`max_fanin` tree over weighted leaves; joining nodes carry unit weights.
fn tree_sum(leaves: &[(ASrc, f64)], max_fanin: usize, cells: &mut Vec<AnalogCell>) -> ASrc {
    if leaves.len() == 1 {
        let (x, w) = leaves[0];
        cells.push(AnalogCell::Sum {
            inputs: vec![x],
            weights: vec![w],
        });
        return ASrc::Val(cells.len() - 1);
    }
    if leaves.len() <= max_fanin {
        cells.push(AnalogCell::Sum {
            inputs: leaves.iter().map(|(x, _)| *x).collect(),
            weights: leaves.iter().map(|(_, w)| *w).collect(),
        });
        return ASrc::Val(cells.len() - 1);
    }
    let chunk = leaves.len().div_ceil(max_fanin).max(1);
    let subs: Vec<(ASrc, f64)> = leaves
        .chunks(chunk.max(1))
        .map(|c| (tree_sum(c, max_fanin, cells), 1.0))
        .collect();
    tree_sum(&subs, max_fanin, cells)
}

/// Fold gain-into-sum and sum-into-sum to fixpoint, then prune dead tiles. Order-preserving:
/// spliced addends keep their producer's addend order at the consumer's position, so the
/// evaluation order stays deterministic.
pub fn fold_crossbar(netlist: &AnalogNetlist) -> AnalogNetlist {
    let mut cells = netlist.cells.clone();
    loop {
        let mut changed = false;
        for i in 0..cells.len() {
            let AnalogCell::Sum { inputs, weights } = &cells[i] else {
                continue;
            };
            let mut new_inputs: Vec<ASrc> = Vec::with_capacity(inputs.len());
            let mut new_weights: Vec<f64> = Vec::with_capacity(weights.len());
            let mut folded_here = false;
            for (x, w) in inputs.iter().zip(weights) {
                match x {
                    ASrc::Val(j) => match &cells[*j] {
                        AnalogCell::GainConst { a, k } => {
                            new_inputs.push(*a);
                            new_weights.push(w * k);
                            folded_here = true;
                        }
                        AnalogCell::Sum {
                            inputs: is2,
                            weights: ws2,
                        } => {
                            for (x2, w2) in is2.iter().zip(ws2) {
                                new_inputs.push(*x2);
                                new_weights.push(w * w2);
                            }
                            folded_here = true;
                        }
                        _ => {
                            new_inputs.push(*x);
                            new_weights.push(*w);
                        }
                    },
                    _ => {
                        new_inputs.push(*x);
                        new_weights.push(*w);
                    }
                }
            }
            if folded_here {
                cells[i] = AnalogCell::Sum {
                    inputs: new_inputs,
                    weights: new_weights,
                };
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    prune(&AnalogNetlist {
        n_in: netlist.n_in,
        cells,
        out: netlist.out,
    })
}

/// Drop tiles unreachable from the output and compact the SSA numbering (the digital fabric's
/// `prune_dead_cells` discipline, over analog tiles).
fn prune(netlist: &AnalogNetlist) -> AnalogNetlist {
    let mut live = vec![false; netlist.cells.len()];
    let mut stack: Vec<usize> = Vec::new();
    let mark = |s: &ASrc, stack: &mut Vec<usize>| {
        if let ASrc::Val(v) = s {
            stack.push(*v);
        }
    };
    mark(&netlist.out, &mut stack);
    while let Some(v) = stack.pop() {
        if live[v] {
            continue;
        }
        live[v] = true;
        match &netlist.cells[v] {
            AnalogCell::Sum { inputs, .. } => {
                for x in inputs {
                    mark(x, &mut stack);
                }
            }
            AnalogCell::Mul { a, b } => {
                mark(a, &mut stack);
                mark(b, &mut stack);
            }
            AnalogCell::GainConst { a, .. } => mark(a, &mut stack),
            AnalogCell::Select { c, t, f } => {
                mark(c, &mut stack);
                mark(t, &mut stack);
                mark(f, &mut stack);
            }
            AnalogCell::Exp { a }
            | AnalogCell::Log { a }
            | AnalogCell::Tanh { a }
            | AnalogCell::Sqrt { a } => mark(a, &mut stack),
            AnalogCell::Div { a, b } => {
                mark(a, &mut stack);
                mark(b, &mut stack);
            }
        }
    }
    let mut remap = vec![usize::MAX; netlist.cells.len()];
    let mut kept: Vec<AnalogCell> = Vec::new();
    for (i, c) in netlist.cells.iter().enumerate() {
        if live[i] {
            remap[i] = kept.len();
            kept.push(c.clone());
        }
    }
    let rl = |s: &ASrc| match s {
        ASrc::Val(v) => ASrc::Val(remap[*v]),
        other => *other,
    };
    for c in kept.iter_mut() {
        match c {
            AnalogCell::Sum { inputs, .. } => {
                for x in inputs.iter_mut() {
                    *x = rl(x);
                }
            }
            AnalogCell::Mul { a, b } => {
                *a = rl(a);
                *b = rl(b);
            }
            AnalogCell::GainConst { a, .. } => *a = rl(a),
            AnalogCell::Select { c, t, f } => {
                *c = rl(c);
                *t = rl(t);
                *f = rl(f);
            }
            AnalogCell::Exp { a }
            | AnalogCell::Log { a }
            | AnalogCell::Tanh { a }
            | AnalogCell::Sqrt { a } => *a = rl(a),
            AnalogCell::Div { a, b } => {
                *a = rl(a);
                *b = rl(b);
            }
        }
    }
    AnalogNetlist {
        n_in: netlist.n_in,
        cells: kept,
        out: rl(&netlist.out),
    }
}
