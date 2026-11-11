//! A compiled lane form's cut, lifted into the one IR as the shared optimizer reads it: each constant's storage from the
//! form's buffers (checkpoint rows in their block format, or values), the names it reads and writes across steps as its
//! plan took them.

#![allow(dead_code)]

use std::collections::BTreeMap;

use logicaffeine_compile::codegen_cuda::lane::plan::{Fill, StepIo as LaneIo};
use logicaffeine_compile::codegen_cuda::lane::weights::Placement;
use logicaffeine_compile::codegen_cuda::lane::Form;
use logicaffeine_compile::spatial::passes::recognize_stages::recognize_stages;
use logicaffeine_compile::tensor_graph::graph::{SeqIo, StepIo, Storage, TensorGraph};

/// The lane's names across steps as the IR's.
pub fn io_of(io: LaneIo) -> StepIo {
    StepIo {
        token: io.token,
        position: io.position,
        logits: io.logits,
        next: io.next,
        more: io.more.into_iter().map(|q| SeqIo { position: q.position, logits: q.logits, next: q.next }).collect(),
        logits_read: io.logits_read,
    }
}

/// `form`'s cut as the one IR lifts it.
pub fn lifted(form: &Form, what: &str) -> TensorGraph {
    let rp = recognize_stages(&form.source).unwrap_or_else(|e| panic!("{what}: {e:?}"));
    let placed: BTreeMap<String, Storage> = form
        .schedule
        .buffers
        .iter()
        .filter_map(|(n, b)| match &b.fill {
            Fill::Constant(Placement::Rows(r)) => Some((n.clone(), Storage::Rows { format: r.ty, transposed: r.transposed })),
            Fill::Constant(Placement::Values(_)) => Some((n.clone(), Storage::Values)),
            _ => None,
        })
        .collect();
    // The names the form carries across steps as its plan took them: whether its host reads its logits too.
    TensorGraph::lift(&rp, Some(form.identity.clone()), Some(io_of(form.schedule.io.clone())), &|n| placed.get(n).cloned().unwrap_or(Storage::Values))
        .unwrap_or_else(|e| panic!("{what}: {e}"))
}
