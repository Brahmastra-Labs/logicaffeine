//! What a composed program was made from, section by section.
//!
//! The whole-program front end lifts a source's tensor statements into ops ([`crate::tensor_graph::graph::TensorGraph`]),
//! the shared optimizer decides every fusion over them for the chip ([`crate::tensor_graph::fuse::Report`]), and the
//! lowering turns the ops it leaves into sections, joining some and bridging layouts between others. A [`Lineage`] keeps
//! all three facts beside the program, in the lifted ops' numbering: which statement each op is, every decision over
//! them, and the ops whose work each section of the composed program holds. A report on a section reads it, never
//! the section's name or the tensors it happens to store.

use std::collections::BTreeSet;

use crate::tensor_graph::fuse::{Decision, Outcome, Report};

/// One op the front end lifted: the top-level statement it is, the tensor it binds and its op.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiftedOp {
    pub stmt_index: usize,
    pub binder: String,
    pub op: String,
}

/// A composed program's lineage: the lifted ops (a decision's `covers` and a section's ops index this list), every
/// decision the shared optimizer made about them on the chip, and, section by section, the lifted ops whose work the
/// section holds — none for a layout bridge, which computes no statement.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lineage {
    pub ops: Vec<LiftedOp>,
    pub fusion: Report,
    pub sections: Vec<BTreeSet<usize>>,
}

impl Lineage {
    /// The sections that hold any op `decision` covers, in order.
    pub fn sections_of(&self, decision: &Decision) -> Vec<usize> {
        self.sections
            .iter()
            .enumerate()
            .filter(|(_, ops)| decision.covers.iter().any(|o| ops.contains(o)))
            .map(|(k, _)| k)
            .collect()
    }

    /// Whether the ops `decision` covers run apart: some section holds part of them and not the rest. An op linked whole
    /// gives every one of its sections all the ops it covers, however many sections it lowers to; ops the lowering runs
    /// one by one — a fired region whose members each store what they make — leave each section with its own.
    pub fn runs_apart(&self, decision: &Decision) -> bool {
        self.sections.iter().any(|ops| {
            let held = decision.covers.iter().filter(|o| ops.contains(o)).count();
            held > 0 && held < decision.covers.len()
        })
    }

    /// The decisions over the ops section `k` holds, by their place in the report: each decision whose covered ops
    /// include one of the section's.
    pub fn decisions_in(&self, k: usize) -> Vec<usize> {
        let ops = &self.sections[k];
        self.fusion
            .decisions
            .iter()
            .enumerate()
            .filter(|(_, d)| d.covers.iter().any(|o| ops.contains(o)))
            .map(|(i, _)| i)
            .collect()
    }
}

/// What became of a decision: `fired` — or `fired, run apart`, where the lowering ran its ops in sections of their own
/// ([`Lineage::runs_apart`]) — or the way it was declined: `refused` by the chip, or `held` by the compile.
pub fn outcome_word(lineage: &Lineage, decision: &Decision) -> &'static str {
    match decision.outcome {
        Outcome::Fired if lineage.runs_apart(decision) => "fired, run apart",
        Outcome::Fired => "fired",
        Outcome::Refused(_) => "refused",
        Outcome::Held => "held",
    }
}
