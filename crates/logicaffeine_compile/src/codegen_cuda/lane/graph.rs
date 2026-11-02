//! The step graph: an authored step (a decode step, a prefill chunk) as the ops it runs and the tensors they
//! touch, each tensor given the role a compiled lane needs to place it.
//!
//! The graph is the step lifted into the one IR ([`TensorGraph::lift`]): every top-level tensor statement is one op,
//! and a statement the front end cannot lift — or a write it cannot account for — is refused by name. A tensor's role
//! is the IR's, from how the program uses it:
//!
//!   • a **parameter** is a tensor the program declares and reads as a position or as row indices (the token, a
//!     chunk's tokens, the position): a compiled step takes it at run time;
//!   • **state** is a declared tensor an op writes in place (a KV cache by `Set row`, a convolution ring, a
//!     delta-rule memory), carried from one step to the next;
//!   • a **constant** is any other declared tensor (weights, norm vectors, rotary tables), which the lane's binding
//!     fills;
//!   • an **activation** is what an op computes.

use std::collections::{BTreeMap, BTreeSet};

use crate::spatial::passes::recognize_stages::{RecognizedProgram, TensorOpSpec};
use crate::tensor_graph::graph::{Storage, TensorGraph};

pub use crate::tensor_graph::graph::{GraphError, Role};

/// One tensor of the step: its shape as the program gives it (`rows × cols`), and its role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tensor {
    pub name: String,
    pub rows: usize,
    pub cols: usize,
    pub role: Role,
}

impl Tensor {
    pub fn len(&self) -> usize {
        self.rows * self.cols
    }
}

/// One op of the step, its operands and result by name.
#[derive(Clone, Debug)]
pub struct Op {
    pub spec: TensorOpSpec,
    pub ins: Vec<String>,
    pub out: String,
}

/// An authored step as a graph: its tensors in their roles, its ops, and the step as the one IR holds it, which the
/// shared optimizer reads.
#[derive(Clone, Debug)]
pub struct StepGraph {
    pub tensors: BTreeMap<String, Tensor>,
    pub ops: Vec<Op>,
    pub lifted: TensorGraph,
}

impl StepGraph {
    /// Build the graph of a walked program.
    pub fn build(rp: &RecognizedProgram) -> Result<StepGraph, GraphError> {
        let lifted = TensorGraph::lift(rp, None, None, &|_| Storage::Values)?;
        let tensors = lifted
            .tensors
            .values()
            .map(|t| (t.name.clone(), Tensor { name: t.name.clone(), rows: t.rows, cols: t.cols, role: t.role }))
            .collect();
        let ops = lifted
            .ops
            .iter()
            .map(|o| Op { spec: o.prim().expect("a lifted op is a primitive").clone(), ins: o.ins.clone(), out: o.out.clone() })
            .collect();
        Ok(StepGraph { tensors, ops, lifted })
    }

    /// The tensors of one role, by name.
    pub fn of_role(&self, role: Role) -> impl Iterator<Item = &Tensor> {
        self.tensors.values().filter(move |t| t.role == role)
    }

    /// The ops that read `tensor`, by index.
    pub fn readers(&self, tensor: &str) -> Vec<usize> {
        self.ops.iter().enumerate().filter(|(_, o)| o.ins.iter().any(|x| x == tensor)).map(|(i, _)| i).collect()
    }

    /// The attention caches: every state tensor an attention reads its keys or values from, itself or through the
    /// head slices cut from it (a latent cache's values are its rows' first part).
    pub fn caches(&self) -> BTreeSet<String> {
        let producer: BTreeMap<&str, &Op> = self.ops.iter().map(|o| (o.out.as_str(), o)).collect();
        let mut out = BTreeSet::new();
        for op in self.ops.iter().filter(|o| matches!(o.spec, TensorOpSpec::Attention(_))) {
            for read in &op.ins[1..3] {
                let mut t = read.as_str();
                loop {
                    if self.tensors.get(t).is_some_and(|x| x.role == Role::State) {
                        out.insert(t.to_string());
                        break;
                    }
                    match producer.get(t) {
                        Some(p) if matches!(p.spec, TensorOpSpec::HeadSlice(_)) => t = &p.ins[0],
                        _ => break,
                    }
                }
            }
        }
        out
    }
}
