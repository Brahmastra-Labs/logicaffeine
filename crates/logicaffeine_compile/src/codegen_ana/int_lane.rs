//! The certified-EXACT integer lane — small-range integer dataflow with the ADC as restorer.
//!
//! Integer programs extract as integer ALU cells (`Arith`), and their constants are VALUES
//! (`Src::Const(7)` is seven, not a bit pattern) — the mirror image of the float lane's
//! conventions. The tile vocabulary is unchanged: integer add/sub is the weighted Kirchhoff
//! sum, integer-constant multiply a programmed gain, variable×variable the four-quadrant
//! multiplier. What is NEW is the claim: because every weight and constant is an integer, the
//! region is GRID-PRESERVING, and when the compiler's error budget proves **ε < ½ grid step**
//! ([`super::exactness_gate`]) the rounded readout is EXACTLY the VM's integer — a stronger
//! statement than the float lane's ε-≡, purchased with the grid's slack. Division, remainder,
//! and every bitwise op refuse: quotient grids are unforgiving and bit patterns are not
//! voltages.

use crate::ast::stmt::{Stmt, TypeExpr};
use crate::codegen_sva::fabric_rtl::{decision_fabric_from_source, AluOp, FuncUnit, LoopCell, Src};
use std::collections::BTreeMap;

use super::cell::{ASrc, AnalogCell, AnalogNetlist};
use super::lower::{gate_on_target, source_without_range_contracts, MappedCell, NetRef};
use super::target::AnalogTarget;
use super::{assumed_ranges_in, AnaLowerError};

/// Map ONE integer fabric cell to an analog tile — the int-lane sibling of `map_float_cell`.
/// `asrc` must decode `Src::Const` as an integer VALUE.
pub(crate) fn map_int_cell(
    cell: &LoopCell,
    asrc: &dyn Fn(&Src) -> ASrc,
) -> Result<MappedCell, AnaLowerError> {
    Ok(match cell {
        LoopCell::Arith {
            op: AluOp::Add,
            a,
            b,
        } => MappedCell::Tile(AnalogCell::Sum {
            inputs: vec![asrc(a), asrc(b)],
            weights: vec![1.0, 1.0],
        }),
        LoopCell::Arith {
            op: AluOp::Sub,
            a,
            b,
        } => MappedCell::Tile(AnalogCell::Sum {
            inputs: vec![asrc(a), asrc(b)],
            weights: vec![1.0, -1.0],
        }),
        LoopCell::Arith {
            op: AluOp::Mul,
            a,
            b,
        } => {
            let (aa, bb) = (asrc(a), asrc(b));
            MappedCell::Tile(match (aa, bb) {
                (ASrc::Const(k), other) | (other, ASrc::Const(k)) => {
                    AnalogCell::GainConst { a: other, k }
                }
                _ => AnalogCell::Mul { a: aa, b: bb },
            })
        }
        LoopCell::Arith { op, .. } => {
            return Err(AnaLowerError::UnloweredOp(format!(
                "integer op {op:?} has no voltage semantics (a bit pattern is not a voltage)"
            )))
        }
        LoopCell::Func {
            unit: FuncUnit::DivS | FuncUnit::ModS | FuncUnit::FloorDivS,
            ..
        } => return Err(AnaLowerError::UnloweredOp(
            "integer division/remainder (no exact analog divider — quotient grids are unforgiving)"
                .to_string(),
        )),
        LoopCell::Func { unit, .. } => {
            return Err(AnaLowerError::UnloweredOp(format!(
                "functional unit {unit:?} has no exact integer analog tile"
            )))
        }
        LoopCell::Select { cmp, ca, cb, t, e } => {
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
            return Ok(super::lower::MappedCell::Tiles(vec![
                AnalogCell::Sum {
                    inputs: vec![x, y],
                    weights: vec![1.0, -1.0],
                },
                AnalogCell::Select {
                    c: ASrc::Val(super::lower::PREV_TILE),
                    t: tt,
                    f: ff,
                },
            ]));
        }
        LoopCell::MemRead { .. }
        | LoopCell::HeapRead { .. }
        | LoopCell::HeapWrite { .. }
        | LoopCell::HeapAlloc { .. } => {
            return Err(AnaLowerError::UnloweredOp(
                "memory/state-plane access inside an analog region".to_string(),
            ))
        }
        LoopCell::VectorArith { .. } => {
            return Err(AnaLowerError::UnloweredOp(
                "packed-SIMD op inside an analog region".to_string(),
            ))
        }
    })
}

/// Lower an INTEGER function to an analog tile netlist realizable on `target`. The exactness
/// claim itself is made downstream by [`super::exactness_gate`] over the scheduled budget.
pub fn analog_int_netlist_from_source(
    src: &str,
    fn_name: &str,
    target: &AnalogTarget,
) -> Result<AnalogNetlist, AnaLowerError> {
    check_int_params(src, fn_name)?; // ranges re-fetched by the schedule pass via `int_region_param_ranges`
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

    let mut cells: Vec<AnalogCell> = Vec::new();
    let mut map: BTreeMap<usize, NetRef> = BTreeMap::new();
    let resolve_with = |map: &BTreeMap<usize, NetRef>, s: &Src| -> ASrc {
        match s {
            Src::In(k) => ASrc::In(*k),
            // Integer constants are VALUES — the mirror of the float lane's bit-pattern decode.
            Src::Const(c) => ASrc::Const(*c as f64),
            Src::Val(v) => match map.get(v) {
                Some(NetRef::Cell(c)) => ASrc::Val(*c),
                Some(NetRef::Const(f)) => ASrc::Const(*f),
                None => ASrc::Val(usize::MAX),
            },
        }
    };
    for (i, cell) in df.cells.iter().enumerate() {
        let resolve = |s: &Src| resolve_with(&map, s);
        let mapped = match map_int_cell(cell, &resolve)? {
            MappedCell::Tile(t) => {
                cells.push(t);
                NetRef::Cell(cells.len() - 1)
            }
            MappedCell::Tiles(ts) => NetRef::Cell(super::lower::push_tiles(&mut cells, ts)),
            MappedCell::Fold(v) => NetRef::Const(v),
        };
        map.insert(i, mapped);
    }
    let out = resolve_with(&map, &df.result);
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
    assert_grid_preserving(&netlist)?;
    gate_on_target(&netlist, target)?;
    Ok(netlist)
}

/// The grid-preservation guarantee: every weight and every constant is an integer, so integer
/// inputs produce integer values on every net. An integer program cannot produce anything else
/// through this lane's mapping — this check is the belt that makes the invariant loud.
pub(crate) fn assert_grid_preserving(netlist: &AnalogNetlist) -> Result<(), AnaLowerError> {
    let int_src = |s: &ASrc| match s {
        ASrc::Const(c) => c.fract() == 0.0,
        _ => true,
    };
    for cell in &netlist.cells {
        let ok = match cell {
            AnalogCell::Sum { inputs, weights } => {
                weights.iter().all(|w| w.fract() == 0.0) && inputs.iter().all(int_src)
            }
            AnalogCell::GainConst { a, k } => k.fract() == 0.0 && int_src(a),
            AnalogCell::Select { c, t, f } => int_src(c) && int_src(t) && int_src(f),
            // Translinear outputs are irrational off the trivial points — never grid-preserving.
            AnalogCell::Exp { .. }
            | AnalogCell::Log { .. }
            | AnalogCell::Tanh { .. }
            | AnalogCell::Sqrt { .. }
            | AnalogCell::Div { .. } => false,
            AnalogCell::Mul { a, b } => int_src(a) && int_src(b),
        };
        if !ok {
            return Err(AnaLowerError::UnloweredOp(
                "non-integer weight/constant in an integer region — the grid would not be preserved".to_string(),
            ));
        }
    }
    Ok(())
}

/// The int region's declared per-parameter ranges, in parameter order, from its OWN body.
pub(crate) fn int_region_param_ranges(
    src: &str,
    fn_name: &str,
) -> Result<Vec<super::AssumedRange>, AnaLowerError> {
    check_int_params(src, fn_name)
}

/// Gate 1: every parameter must be an integer with a declared integer range in the function body.
/// Returns the ranges in parameter order.
fn check_int_params(src: &str, fn_name: &str) -> Result<Vec<super::AssumedRange>, AnaLowerError> {
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
        let is_int = |t: &TypeExpr<'_>| {
            matches!(t, TypeExpr::Primitive(s) | TypeExpr::Named(s)
                if matches!(interner.resolve(*s), "Int" | "Nat"))
        };
        if params.is_empty() || !params.iter().any(|(_, t)| is_int(t)) {
            return Err(AnaLowerError::NoAnalogRegion);
        }
        let ranges = assumed_ranges_in(body, interner);
        let mut ordered = Vec::with_capacity(params.len());
        for (p, t) in params.iter() {
            let pname = interner.resolve(*p);
            if !is_int(t) {
                return Err(AnaLowerError::UnloweredOp(format!(
                    "non-integer parameter `{pname}` in the integer lane"
                )));
            }
            let Some(r) = ranges.iter().find(|r| r.var == pname) else {
                return Err(AnaLowerError::UnboundedInput(format!(
                    "parameter `{pname}` has no declared range (`Assume {pname} is between A and B.`)"
                )));
            };
            if r.lo.fract() != 0.0 || r.hi.fract() != 0.0 {
                return Err(AnaLowerError::UnboundedInput(format!(
                    "parameter `{pname}`'s range [{}, {}] is not an integer range",
                    r.lo, r.hi
                )));
            }
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
