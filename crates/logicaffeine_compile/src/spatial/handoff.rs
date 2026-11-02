//! What one chip hands another when they share a model's work.
//!
//! A prompt prefilled on one chip and decoded on another — a GPU's prefill handed to an RDU's decode, as SambaNova serves
//! MiniMax, or the reverse — and a model whose layers are split over two chips meet at the same place: the state a
//! decode step carries from one token to the next. Every attention layer's key and value cache, every gated-delta
//! layer's memory and conv rings: the tensors the step reads and writes back in place, less those it assembles whole
//! ([`carried_state`]).
//!
//! ★ THE CONTRACT IS THE AUTHOR'S. Every backend lowers the one program `model::author` writes, and that program
//! declares each carried tensor once, by name and shape. The prefill program declares the same tensors the decode
//! program reads, so the hand-off is those tensors' values, row-major as declared — whatever tiles, pages or arenas a
//! backend lays them out in, it reads and writes this logical order at its edge. [`contract`] reads the declarations
//! off the decode step's layer programs; a bundle's manifest states it, so a host on either side knows what to send;
//! [`Contract::check`] refuses a hand-off that does not match it, by name.

use std::collections::BTreeMap;

use crate::model::program::{written, Cut};
use crate::model::spec::ModelSpec;
use crate::tensor_graph::front::carried_state;
use crate::tensor_graph::graph::{Storage, TensorGraph};

/// One tensor a decode step carries: its name in the authored programs, the layer it belongs to, and its declared shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Carried {
    pub name: String,
    pub layer: usize,
    pub rows: usize,
    pub cols: usize,
}

/// The state a model's decode step carries over a cache of `ctx` positions, layer by layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contract {
    pub ctx: usize,
    /// The width of the hidden state a layer hands the next — what a model split over chips sends at its cut.
    pub hidden: usize,
    pub carried: Vec<Carried>,
}

/// A prompt's state after `position` tokens: each carried tensor's values by name, row-major as declared.
#[derive(Clone, Debug, PartialEq)]
pub struct Handoff {
    pub position: usize,
    pub state: BTreeMap<String, Vec<f64>>,
}

/// Why a hand-off does not meet its contract, or why a model has none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandoffError {
    /// A layer's decode program does not walk.
    Program { layer: usize, why: String },
    /// A carried tensor the hand-off leaves out.
    Missing(String),
    /// A tensor the contract does not carry.
    Unknown(String),
    /// A carried tensor of the wrong size.
    Length { name: String, want: usize, got: usize },
    /// A position whose next token's row the cache does not have: a decode after `position` tokens writes row
    /// `position + 1` of `ctx`.
    Position { position: usize, ctx: usize },
}

impl std::fmt::Display for HandoffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandoffError::Program { layer, why } => write!(f, "layer {layer}'s decode program: {why}"),
            HandoffError::Missing(n) => write!(f, "the hand-off leaves out the carried `{n}`"),
            HandoffError::Unknown(n) => write!(f, "the hand-off carries `{n}`, which no decode step reads"),
            HandoffError::Length { name, want, got } => write!(f, "`{name}` is {want} values in the contract and {got} in the hand-off"),
            HandoffError::Position { position, ctx } => {
                write!(f, "after {position} tokens the next decode writes row {} of a cache of {ctx}", position + 1)
            }
        }
    }
}

/// The carried state of `spec`'s decode step over `ctx` positions: every layer's, in layer order and, within a layer, in
/// the order its program first touches it.
pub fn contract(spec: &ModelSpec, ctx: usize) -> Result<Contract, HandoffError> {
    let mut carried = Vec::new();
    for layer in 0..spec.layers.len() {
        let ((src, _, _), _) = written(spec, Cut::DecodeLayer { layer, ctx });
        let lifted = TensorGraph::lift_source(&src, &|_| Storage::Values).map_err(|e| HandoffError::Program { layer, why: format!("{e:?}") })?;
        let rp = &lifted.program;
        for name in carried_state(rp) {
            let (rows, cols) = rp.shapes.get(&name).copied().ok_or_else(|| HandoffError::Program {
                layer,
                why: format!("carries `{name}`, which it does not declare"),
            })?;
            carried.push(Carried { name, layer, rows, cols });
        }
    }
    Ok(Contract { ctx, hidden: spec.d, carried })
}

impl Contract {
    /// The carried tensors of layer `layer`.
    pub fn of_layer(&self, layer: usize) -> impl Iterator<Item = &Carried> {
        self.carried.iter().filter(move |c| c.layer == layer)
    }

    /// Whether `h` is this contract's: every carried tensor, at its size, nothing else, and a next row in the cache.
    pub fn check(&self, h: &Handoff) -> Result<(), HandoffError> {
        if let Some(name) = h.state.keys().find(|n| !self.carried.iter().any(|c| &c.name == *n)) {
            return Err(HandoffError::Unknown(name.clone()));
        }
        for c in &self.carried {
            let values = h.state.get(&c.name).ok_or_else(|| HandoffError::Missing(c.name.clone()))?;
            if values.len() != c.rows * c.cols {
                return Err(HandoffError::Length { name: c.name.clone(), want: c.rows * c.cols, got: values.len() });
            }
        }
        if h.position >= self.ctx {
            return Err(HandoffError::Position { position: h.position, ctx: self.ctx });
        }
        Ok(())
    }

    /// The hand-off a run leaves after `position` tokens: the carried tensors out of `tensors` (a run's every tensor, by
    /// name), checked.
    pub fn take(&self, position: usize, tensors: &BTreeMap<String, Vec<f64>>) -> Result<Handoff, HandoffError> {
        let state = self
            .carried
            .iter()
            .map(|c| tensors.get(&c.name).map(|v| (c.name.clone(), v.clone())).ok_or_else(|| HandoffError::Missing(c.name.clone())))
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let h = Handoff { position, state };
        self.check(&h)?;
        Ok(h)
    }

    /// The contract as a manifest states it.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ctx": self.ctx,
            "hidden": self.hidden,
            "carried": self.carried.iter().map(|c| serde_json::json!({ "name": c.name, "layer": c.layer, "rows": c.rows, "cols": c.cols })).collect::<Vec<_>>(),
        })
    }
}
