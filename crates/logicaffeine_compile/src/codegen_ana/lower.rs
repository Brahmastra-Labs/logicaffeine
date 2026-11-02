//! Float-function → analog-netlist lowering — the analog lane's Futamura moment.
//!
//! The region unit is a FUNCTION: its float parameters are the plane's DAC inputs, its `Assume`
//! statements the declared input ranges, its return the ADC tap. Extraction reuses the digital
//! fabric's certified if-conversion front-end ([`decision_fabric_from_source`], imported
//! read-only) on a contract-free twin of the source: the `Assume` range checks are the CONTROL
//! PLANE's job (the digital host validates a word before driving the DAC), so they are stripped
//! from the analog dataflow — exactly the mixed-signal architecture, not an elision. The declared
//! ranges themselves are read from the ORIGINAL function body.
//!
//! Every cell the fabric front-end emits is either mapped to an analog tile (`FAddF`/`FSubF` →
//! weighted Kirchhoff [`AnalogCell::Sum`], `FMulF` → [`AnalogCell::GainConst`] when one side is a
//! programmed constant, else the four-quadrant [`AnalogCell::Mul`]; `IntToF` of a constant folds)
//! or refused with a NAMED [`AnaLowerError`] — fail-closed, never a silently-wrong circuit.

use std::collections::BTreeMap;

use crate::ast::stmt::{Stmt, TypeExpr};
use crate::codegen_sva::fabric_rtl::{decision_fabric_from_source, FuncUnit, LoopCell, Src};

use super::cell::{ASrc, AnalogCell, AnalogNetlist};
use super::target::{AnaUnplaceable, AnalogCapability, AnalogTarget};
use super::{assumed_ranges_in, AnaLowerError};

/// Where a fabric SSA val lands in the analog netlist: a real tile's output net, or a
/// compile-time fold (an `IntToF` of a constant).
#[derive(Clone, Copy)]
pub(crate) enum NetRef {
    Cell(usize),
    Const(f64),
}

/// The result of mapping one fabric cell into the analog vocabulary.
pub(crate) enum MappedCell {
    /// A real analog tile.
    Tile(AnalogCell),
    /// A short tile GROUP emitted for one fabric cell (e.g. Select = difference Sum + switch).
    /// Inside the group, `ASrc::Val(PREV_TILE)` refers to the immediately preceding tile;
    /// [`push_tiles`] rewrites it to the real index. The group's result is its LAST tile.
    Tiles(Vec<AnalogCell>),
    /// A compile-time fold (e.g. `IntToF` of a constant) — no tile, just a value.
    Fold(f64),
}

/// The within-group back-reference sentinel for [`MappedCell::Tiles`].
pub(crate) const PREV_TILE: usize = usize::MAX - 1;

/// Push a tile group, rewriting `PREV_TILE` references, returning the LAST tile's index.
pub(crate) fn push_tiles(cells: &mut Vec<AnalogCell>, tiles: Vec<AnalogCell>) -> usize {
    let mut last = 0usize;
    for mut t in tiles {
        let prev = cells.len().wrapping_sub(1);
        let fix = |s: &mut ASrc| {
            if matches!(s, ASrc::Val(v) if *v == PREV_TILE) {
                *s = ASrc::Val(prev);
            }
        };
        match &mut t {
            AnalogCell::Sum { inputs, .. } => inputs.iter_mut().for_each(fix),
            AnalogCell::Mul { a, b } => {
                fix(a);
                fix(b);
            }
            AnalogCell::GainConst { a, .. } => fix(a),
            AnalogCell::Select { c, t, f } => {
                fix(c);
                fix(t);
                fix(f);
            }
            AnalogCell::Exp { a }
            | AnalogCell::Log { a }
            | AnalogCell::Tanh { a }
            | AnalogCell::Sqrt { a } => fix(a),
            AnalogCell::Div { a, b } => {
                fix(a);
                fix(b);
            }
        }
        cells.push(t);
        last = cells.len() - 1;
    }
    last
}

/// Map ONE fabric float cell to an analog tile — the shared vocabulary of the decision-fabric
/// and reduction lanes. `asrc` resolves fabric operands into analog operands (each lane has its
/// own input convention). `MemRead`/heap/state cells are the CALLER's job to intercept (the
/// reduction lane maps the streamed element read); reaching here they refuse.
pub(crate) fn map_float_cell(
    cell: &LoopCell,
    asrc: &dyn Fn(&Src) -> ASrc,
) -> Result<MappedCell, AnaLowerError> {
    Ok(match cell {
        LoopCell::Func { unit: FuncUnit::FAddF, a, b } => {
            MappedCell::Tile(AnalogCell::Sum { inputs: vec![asrc(a), asrc(b)], weights: vec![1.0, 1.0] })
        }
        LoopCell::Func { unit: FuncUnit::FSubF, a, b } => {
            MappedCell::Tile(AnalogCell::Sum { inputs: vec![asrc(a), asrc(b)], weights: vec![1.0, -1.0] })
        }
        LoopCell::Func { unit: FuncUnit::FMulF, a, b } => {
            let (aa, bb) = (asrc(a), asrc(b));
            MappedCell::Tile(match (aa, bb) {
                (ASrc::Const(k), other) | (other, ASrc::Const(k)) => AnalogCell::GainConst { a: other, k },
                _ => AnalogCell::Mul { a: aa, b: bb },
            })
        }
        // INT→FLOAT of a compile-time constant folds to a programmed reference; a RUNTIME
        // integer has no voltage representation and refuses.
        LoopCell::Func { unit: FuncUnit::IntToF, a, .. } => match a {
            Src::Const(c) => MappedCell::Fold(*c as f64),
            _ => {
                return Err(AnaLowerError::UnloweredOp(
                    "int→float promotion of a runtime value (no voltage representation for a digital word mid-plane)".to_string(),
                ))
            }
        },
        LoopCell::Func { unit: FuncUnit::FDivF, a, b } => {
            // O6: the translinear ratio tile — the divisor's certified box must clear zero
            // (the schedule's range rules refuse `DivisorRangeSpansZero` otherwise).
            MappedCell::Tile(AnalogCell::Div { a: asrc(a), b: asrc(b) })
        }
        LoopCell::Func { unit: FuncUnit::FSqrtF, a, .. } => {
            MappedCell::Tile(AnalogCell::Sqrt { a: asrc(a) })
        }
        LoopCell::Func { unit, .. } => {
            return Err(AnaLowerError::UnloweredOp(format!("functional unit {unit:?} has no witnessed analog tile")))
        }
        LoopCell::Arith { op, .. } => {
            return Err(AnaLowerError::UnloweredOp(format!("integer op {op:?} inside an analog region")))
        }
        LoopCell::Select { cmp, ca, cb, t, e } => {
            // Normalize the decision to `d < 0` with `d = x − y`; strict vs inclusive is
            // indistinguishable on the analog plane (the boundary lives inside the
            // comparator's ambiguity band, priced by `CompareBand`). Equality is a
            // measure-zero decision no continuous comparator realizes — refused.
            use crate::codegen_sva::fabric_rtl::CmpKind;
            let (x, y, tt, ff) = match cmp {
                CmpKind::Lt | CmpKind::Le => (asrc(ca), asrc(cb), asrc(t), asrc(e)),
                CmpKind::Gt | CmpKind::Ge => (asrc(cb), asrc(ca), asrc(t), asrc(e)),
                CmpKind::Eq | CmpKind::Ne => {
                    return Err(AnaLowerError::UnloweredOp(
                        "equality select: a continuous comparator has no measure-zero decision"
                            .to_string(),
                    ))
                }
            };
            MappedCell::Tiles(vec![
                AnalogCell::Sum { inputs: vec![x, y], weights: vec![1.0, -1.0] },
                AnalogCell::Select { c: ASrc::Val(PREV_TILE), t: tt, f: ff },
            ])
        }
        LoopCell::MemRead { .. } | LoopCell::HeapRead { .. } | LoopCell::HeapWrite { .. }
        | LoopCell::HeapAlloc { .. } => {
            return Err(AnaLowerError::UnloweredOp("memory/state-plane access inside an analog region".to_string()))
        }
        LoopCell::VectorArith { .. } => {
            return Err(AnaLowerError::UnloweredOp("packed-SIMD op inside an analog region".to_string()))
        }
    })
}

/// Lower `fn_name` in `src` to an analog tile netlist realizable on `target`.
///
/// Refusals (fail-closed, in gate order): no float parameter → [`AnaLowerError::NoAnalogRegion`];
/// a non-float parameter → [`AnaLowerError::UnloweredOp`]; a float parameter without a declared
/// range in the function body → [`AnaLowerError::UnboundedInput`] naming it; an op outside the
/// witnessed tile vocabulary → [`AnaLowerError::UnloweredOp`]; a netlist the target cannot
/// realize → [`AnaLowerError::Unplaceable`].
pub fn analog_netlist_from_source(
    src: &str,
    fn_name: &str,
    target: &AnalogTarget,
) -> Result<AnalogNetlist, AnaLowerError> {
    // ── Gate 1: the function's parameter contract (from the ORIGINAL source). ──
    check_params(src, fn_name)?; // ranges re-fetched by the schedule pass via `region_param_ranges`

    // ── Extract: the certified fabric front-end on the contract-free twin. ──
    let twin = source_without_range_contracts(src);
    let df = decision_fabric_from_source(&twin, fn_name);
    if df.escaped_first_return {
        return Err(AnaLowerError::UnloweredOp(
            "early-return branch shape: a branch continues past the first Return, and a \
             tolerance-verified lane cannot detect a dropped branch — write the conditional \
             as Let/Set so it if-converts"
                .to_string(),
        ));
    }

    // ── Lower: fabric cells → analog tiles, or a named refusal. ──
    let mut cells: Vec<AnalogCell> = Vec::new();
    let mut map: BTreeMap<usize, NetRef> = BTreeMap::new();

    let asrc = |s: &Src, map: &BTreeMap<usize, NetRef>| -> ASrc {
        match s {
            Src::In(k) => ASrc::In(*k),
            Src::Const(bits) => ASrc::Const(f64::from_bits(*bits as u64)),
            Src::Val(v) => match map.get(v) {
                Some(NetRef::Cell(c)) => ASrc::Val(*c),
                Some(NetRef::Const(f)) => ASrc::Const(*f),
                None => ASrc::Val(usize::MAX), // an unmapped val is caught by the refusal below
            },
        }
    };

    for (i, cell) in df.cells.iter().enumerate() {
        let resolve = |s: &Src| asrc(s, &map);
        let mapped = match map_float_cell(cell, &resolve)? {
            MappedCell::Tile(t) => NetRef::Cell(push(&mut cells, t)),
            MappedCell::Tiles(ts) => NetRef::Cell(push_tiles(&mut cells, ts)),
            MappedCell::Fold(v) => NetRef::Const(v),
        };
        map.insert(i, mapped);
    }

    let out = asrc(&df.result, &map);
    if matches!(out, ASrc::Val(usize::MAX)) {
        return Err(AnaLowerError::UnloweredOp(
            "region output references an unlowered value".to_string(),
        ));
    }

    let netlist = AnalogNetlist {
        n_in: df.n_in,
        cells,
        out,
    };
    gate_on_target(&netlist, target)?;
    Ok(netlist)
}

fn push(cells: &mut Vec<AnalogCell>, c: AnalogCell) -> usize {
    cells.push(c);
    cells.len() - 1
}

/// The region's declared per-parameter ranges, IN PARAMETER ORDER, from the function's OWN
/// body — a contract declared in another function does not bound this region (the multi-region
/// programs of the mixed-signal partition made whole-source collection a bug).
pub(crate) fn region_param_ranges(
    src: &str,
    fn_name: &str,
) -> Result<Vec<super::AssumedRange>, AnaLowerError> {
    check_params(src, fn_name)
}

/// Gate 1: every parameter must be a float with a declared range in the function's own body.
/// Returns the ranges in parameter order.
fn check_params(src: &str, fn_name: &str) -> Result<Vec<super::AssumedRange>, AnaLowerError> {
    crate::ui_bridge::with_parsed_program(src, |parsed, interner| {
        let (stmts, _types, _pol) = match parsed {
            Ok(t) => t,
            Err(advice) => {
                return Err(AnaLowerError::UnloweredOp(format!(
                    "parse failed: {advice}"
                )))
            }
        };
        let Some((params, body)) = stmts.iter().find_map(|s| match s {
            Stmt::FunctionDef {
                name, params, body, ..
            } if interner.resolve(*name) == fn_name => Some((params, *body)),
            _ => None,
        }) else {
            return Err(AnaLowerError::UnloweredOp(format!(
                "function `{fn_name}` not found"
            )));
        };

        let is_float = |t: &TypeExpr<'_>| match t {
            TypeExpr::Primitive(s) | TypeExpr::Named(s) => {
                matches!(interner.resolve(*s), "Float" | "Real")
            }
            _ => false,
        };
        if !params.iter().any(|(_, t)| is_float(t)) {
            return Err(AnaLowerError::NoAnalogRegion);
        }
        // The ranges declared in THIS function's body — a contract elsewhere does not bound us.
        let ranges = assumed_ranges_in(body, interner);
        let mut ordered = Vec::with_capacity(params.len());
        for (p, t) in params.iter() {
            let pname = interner.resolve(*p);
            if !is_float(t) {
                return Err(AnaLowerError::UnloweredOp(format!(
                    "non-float parameter `{pname}` (no voltage representation)"
                )));
            }
            let Some(r) = ranges.iter().find(|r| r.var == pname) else {
                return Err(AnaLowerError::UnboundedInput(format!(
                    "parameter `{pname}` has no declared range (`Assume {pname} is between A and B.`)"
                )));
            };
            if r.lo > r.hi {
                return Err(AnaLowerError::ContradictoryContract {
                    var: r.var.clone(),
                    lo: r.lo,
                    hi: r.hi,
                });
            }
            ordered.push(r.clone());
        }
        Ok(ordered)
    })
}

/// The contract-free twin: the `Assume` range checks belong to the digital control plane (the
/// host validates each word BEFORE driving the DAC), so the analog dataflow is extracted without
/// them. The statement is line-shaped in block grammar, so a line filter is exact for the
/// canonical form; a `Require that` stays put (and refuses loudly downstream if it reaches the
/// fabric front-end) — only the range-declaration surface moves planes.
pub(crate) fn source_without_range_contracts(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("Assume "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Gate 3: the target must provide every tile kind, enough multiplier tiles (the scarce
/// resource), enough DAC/ADC channels, and enough array room.
pub(crate) fn gate_on_target(
    netlist: &AnalogNetlist,
    target: &AnalogTarget,
) -> Result<(), AnaLowerError> {
    for cell in &netlist.cells {
        let cap = AnalogCapability::Cell(cell.kind());
        if !target.provides(cap) {
            return Err(AnaLowerError::Unplaceable(
                AnaUnplaceable::MissingCapability {
                    needed: cap,
                    target: target.name,
                },
            ));
        }
    }
    let n_mul = netlist
        .cells
        .iter()
        .filter(|c| matches!(c, super::cell::AnalogCell::Mul { .. }))
        .count();
    if n_mul > 0 && !target.provides(AnalogCapability::MulTiles(n_mul)) {
        return Err(AnaLowerError::Unplaceable(
            AnaUnplaceable::MissingCapability {
                needed: AnalogCapability::MulTiles(n_mul),
                target: target.name,
            },
        ));
    }
    for (cap_needed, _n) in [
        (AnalogCapability::DacChannels(netlist.n_in), netlist.n_in),
        (AnalogCapability::AdcChannels(1), 1),
    ] {
        if !target.provides(cap_needed) {
            return Err(AnaLowerError::Unplaceable(
                AnaUnplaceable::MissingCapability {
                    needed: cap_needed,
                    target: target.name,
                },
            ));
        }
    }
    if netlist.cells.len() > target.rows * target.cols {
        return Err(AnaLowerError::Unplaceable(AnaUnplaceable::TooSmall {
            needed_cells: netlist.cells.len(),
            target_rows: target.rows,
            target_cols: target.cols,
            target: target.name,
        }));
    }
    Ok(())
}
