//! Where each constant of a compiled step comes from.
//!
//! A projection or a table the author declares quantized is read straight out of the checkpoint in the format the
//! checkpoint keeps it — Q8_0, a block format, or a 16-bit float: the rows its binding's view picks, in the order it
//! picks them, copied byte for byte (an MXFP4 row's blocks regrouped, exponents before nibbles, so each sub-block's
//! nibbles are one aligned word) — the program maps the checkpoint and uploads those rows, so a weight is never
//! rewritten, widened or quantized again. Every other constant — a norm weight, a rotary table, a
//! small `f32` projection — the compiler computes once, as [`bind::values`] reads it, and ships beside the program.

use crate::model::bind::{self, Source, View};
use crate::model::gguf::Gguf;
use crate::model::quant::GgmlType;
use crate::model::spec::{ModelSpec, RopeScaling};
use crate::spatial::tensor_parallel::{SliceAxis, TpSlice};

use super::LaneError;

/// `rows` consecutive checkpoint rows, the first at byte `offset` of the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub offset: u64,
    pub rows: usize,
}

/// A quantized matrix read straight from the checkpoint: `rows` rows of `cols` weights in `ty`, the device copy's rows
/// the runs' rows in order. `transposed` when the program declares the matrix the other way round — a projection's
/// rows are its outputs, the columns of the `in × out` matrix the program multiplies by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rows {
    pub tensor: String,
    pub ty: GgmlType,
    pub rows: usize,
    pub cols: usize,
    pub transposed: bool,
    pub runs: Vec<Run>,
}

/// How a constant reaches the device.
#[derive(Clone, Debug, PartialEq)]
pub enum Placement {
    /// Quantized rows straight from the checkpoint.
    Rows(Rows),
    /// Values the compiler computes, in the layout the program declares (`rows × cols`, row-major).
    Values(Vec<f32>),
}

/// The rows of a tensor of `n` rows a view picks, in order, and whether it transposes them — `None` for a view that
/// reorders within a row.
fn picked(view: &View, n: usize) -> Option<(Vec<usize>, bool)> {
    match *view {
        View::Same => Some(((0..n).collect(), false)),
        View::Transposed => Some(((0..n).collect(), true)),
        View::NeoxPairs { head_dim, from, dims, transpose: true } => Some((
            (0..n).map(|r| (r / head_dim) * head_dim + bind::neox_source_from(r % head_dim, from, dims)).collect(),
            true,
        )),
        View::NeoxPairs { transpose: false, .. } => None,
        View::RowRange { first, count, transpose } => Some(((first..first + count).collect(), transpose)),
        View::HeadPart { stride, offset, width, transpose } => {
            Some(((0..n).filter(|r| (offset..offset + width).contains(&(r % stride))).collect(), transpose))
        }
        // Each head's block of rows in place: the rows are the head's outputs, read transposed head by head.
        View::FusedPart { first, count, head_dim, dims, bias: false } => Some((
            (0..count).map(|r| first + (r / head_dim) * head_dim + bind::neox_source_from(r % head_dim, 0, dims)).collect(),
            true,
        )),
        View::FusedPart { bias: true, .. } => None,
        View::HeadBlocks { stride, offset, width } => {
            Some(((0..n).filter(|r| (offset..offset + width).contains(&(r % stride))).collect(), true))
        }
        // Zero rows between its taps: no pick of the tensor's rows.
        View::Dilated { .. } => None,
    }
}

/// Where constant `var` (of `len` elements) of a step authored from `spec` comes from. `q8` when the author declared
/// it a quantized matrix — rows read in place, or values when the checkpoint keeps it in a float format; `positions`
/// the rows of the rotary caches.
pub fn place(
    spec: &ModelSpec,
    g: &mut Gguf,
    var: &str,
    len: usize,
    q8: bool,
    positions: usize,
) -> Result<Placement, LaneError> {
    if let Some(table) = rotary_table(spec, g, var, positions)? {
        if table.len() != len {
            return Err(LaneError::Shape { var: var.to_string(), declared: len, bound: table.len() });
        }
        return Ok(Placement::Values(table));
    }
    let binding = match bind::bind(spec, var) {
        Ok(Source::Weight(b)) => b,
        Ok(Source::State) => return Err(LaneError::Unbound(format!("{var} is carried state, not a constant"))),
        Err(bind::Unbound(v)) => return Err(LaneError::Unbound(v)),
    };
    let info = g.info(&binding.tensor).ok_or_else(|| LaneError::Unbound(binding.tensor.clone()))?.clone();
    let ty = g.tensor_type(&binding.tensor).map_err(|e| LaneError::Io(e.to_string()))?;
    // A matrix the checkpoint keeps in `f32` is read as its values (the f32 kernels take it); every other format — a
    // 16-bit float one too, two bytes a weight, widened in registers — is read as rows in place.
    if q8 && ty != GgmlType::F32 {
        if !super::kernels::reads_in_place(ty) {
            return Err(LaneError::Format {
                tensor: binding.tensor,
                ty: ty.name(),
                why: "a matrix the lane reads in place is Q8_0, a format the block library decodes or a 16-bit float; this one has no decoder",
            });
        }
        let cols = info.dims.first().copied().unwrap_or(1) as usize;
        let n = info.elements() / cols.max(1);
        let Some((picked, transposed)) = picked(&binding.view, n) else {
            return Err(LaneError::Format {
                tensor: binding.tensor,
                ty: ty.name(),
                why: "a view that reorders within a row cannot be read as whole quantized rows",
            });
        };
        if picked.len() * cols != len {
            return Err(LaneError::Shape { var: var.to_string(), declared: len, bound: picked.len() * cols });
        }
        let row_bytes = ty.bytes_for(cols) as u64;
        let base = g.data_start() + info.offset;
        let mut runs: Vec<Run> = Vec::new();
        let mut last: Option<usize> = None;
        for &r in &picked {
            match (runs.last_mut(), last) {
                (Some(run), Some(prev)) if r == prev + 1 => run.rows += 1,
                _ => runs.push(Run { offset: base + r as u64 * row_bytes, rows: 1 }),
            }
            last = Some(r);
        }
        return Ok(Placement::Rows(Rows {
            tensor: binding.tensor,
            ty,
            rows: picked.len(),
            cols,
            transposed,
            runs,
        }));
    }
    let values = bind::values(&binding, g).map_err(|e| LaneError::Io(e.to_string()))?;
    if values.len() != len {
        return Err(LaneError::Shape { var: var.to_string(), declared: len, bound: values.len() });
    }
    Ok(Placement::Values(values.into_iter().map(|v| v as f32).collect()))
}

/// Where a chip-local constant `var` (of `len` elements) of a step split over chips comes from: its `slice` of a full
/// constant of the step written whole. A quantized matrix's slice is read in place where it is whole rows of the
/// checkpoint — a run of outputs — or one run of whole blocks of every row — a run of inputs, each row's blocks copied
/// as a row of its own; any other slice, and every other constant, is read as the full constant's values and cut.
pub fn place_slice(spec: &ModelSpec, g: &mut Gguf, var: &str, len: usize, slice: &TpSlice, q8: bool) -> Result<Placement, LaneError> {
    let (rows, cols) = slice.shape();
    if rows * cols != len {
        return Err(LaneError::Shape { var: var.to_string(), declared: len, bound: rows * cols });
    }
    let binding = match bind::bind(spec, &slice.of) {
        Ok(Source::Weight(b)) => b,
        Ok(Source::State) => return Err(LaneError::Unbound(format!("{var} slices carried state `{}`, not a constant", slice.of))),
        Err(bind::Unbound(v)) => return Err(LaneError::Unbound(v)),
    };
    let info = g.info(&binding.tensor).ok_or_else(|| LaneError::Unbound(binding.tensor.clone()))?.clone();
    let ty = g.tensor_type(&binding.tensor).map_err(|e| LaneError::Io(e.to_string()))?;
    let width = info.dims.first().copied().unwrap_or(1) as usize;
    let picked_rows = picked(&binding.view, info.elements() / width.max(1));
    if q8 && ty != GgmlType::F32 && super::kernels::reads_in_place(ty) {
        if let Some((picked, transposed)) = picked_rows {
            // The slice's axis along the checkpoint's rows, or along each row: a transposed matrix's rows are the
            // program's columns.
            let along_rows = (slice.axis == SliceAxis::Cols) == transposed;
            let base = g.data_start() + info.offset;
            let row_bytes = ty.bytes_for(width) as u64;
            let whole = |n: usize| n % ty.block_len() == 0;
            let placed = if along_rows {
                let chosen: Vec<usize> = slice.ranges.iter().flat_map(|&(start, n)| picked[start..start + n].iter().copied()).collect();
                let mut runs: Vec<Run> = Vec::new();
                let mut last: Option<usize> = None;
                for &r in &chosen {
                    match (runs.last_mut(), last) {
                        (Some(run), Some(prev)) if r == prev + 1 => run.rows += 1,
                        _ => runs.push(Run { offset: base + r as u64 * row_bytes, rows: 1 }),
                    }
                    last = Some(r);
                }
                Some(Rows { tensor: binding.tensor.clone(), ty, rows: chosen.len(), cols: width, transposed, runs })
            } else {
                match slice.ranges[..] {
                    [(start, n)] if whole(start) && whole(n) => {
                        let skip = ty.bytes_for(start) as u64;
                        let runs = picked.iter().map(|&r| Run { offset: base + r as u64 * row_bytes + skip, rows: 1 }).collect();
                        Some(Rows { tensor: binding.tensor.clone(), ty, rows: picked.len(), cols: n, transposed, runs })
                    }
                    _ => None,
                }
            };
            if let Some(rows) = placed {
                return Ok(Placement::Rows(rows));
            }
        }
    }
    let full = bind::values(&binding, g).map_err(|e| LaneError::Io(e.to_string()))?;
    let (fr, fc) = slice.of_shape;
    if full.len() != fr * fc {
        return Err(LaneError::Shape { var: slice.of.clone(), declared: fr * fc, bound: full.len() });
    }
    Ok(Placement::Values((0..rows).flat_map(|r| (0..cols).map(move |c| (r, c))).map(|(r, c)| full[slice.full_index(r, c)] as f32).collect()))
}

/// A rotary table's values when `var` names one ([`bind::RotaryTable`]): a decode step's caches of `positions` rows, or
/// a forward's one row (position 0), of the rotation the name gives.
fn rotary_table(spec: &ModelSpec, g: &mut Gguf, var: &str, positions: usize) -> Result<Option<Vec<f32>>, LaneError> {
    let Some(table) = bind::RotaryTable::named(var) else { return Ok(None) };
    let rows = if table.cached { positions } else { 1 };
    let rope = *bind::rotations(spec)
        .get(table.rotation)
        .ok_or_else(|| LaneError::Unbound(format!("{var}: the model has no rotation {}", table.rotation)))?;
    let factors = match rope.scaling {
        RopeScaling::Factors => Some(g.float_tensor("rope_freqs.weight").map_err(|e| LaneError::Io(e.to_string()))?),
        _ => None,
    };
    let factors: Option<Vec<f64>> = factors.map(|f| f.into_iter().map(f64::from).collect());
    let (c, s) = bind::rope_tables(rope, rows, factors.as_deref());
    Ok(Some((if table.cos { c } else { s }).into_iter().map(|v| v as f32).collect()))
}
