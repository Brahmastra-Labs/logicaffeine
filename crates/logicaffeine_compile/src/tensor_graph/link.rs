//! The link's resolver: every op of a graph answered by a chip's shim, before the chip's own link builds anything.
//!
//! An op the chip links whole resolves to itself; an op of a class the chip runs as its parts resolves to its parts,
//! each resolved in turn; an op the chip runs neither way is unresolved, by its place, its class and the chip's reason.
//! Every refusal of the graph is reported together, and nothing is dropped: a graph resolves only when every op does.

use super::graph::{Op, Sym, TensorGraph};
use super::shim::{allows, Allow, Shim};
use super::symbols::ClassId;

/// An op as the chip runs it: whole, or as its parts.
#[derive(Clone, Debug)]
pub enum Linked<'g> {
    Whole(&'g Op),
    Parts(Vec<Linked<'g>>),
}

impl Linked<'_> {
    /// The ops the chip's link builds, in order.
    pub fn whole(&self) -> Vec<&Op> {
        match self {
            Linked::Whole(op) => vec![op],
            Linked::Parts(parts) => parts.iter().flat_map(Linked::whole).collect(),
        }
    }
}

/// An op the chip runs neither whole nor as its parts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unresolved {
    /// The op's place in the graph.
    pub op: usize,
    /// Its class, for a fused op.
    pub class: Option<ClassId>,
    /// Its kind: the lifted op's, or the class id.
    pub kind: String,
    pub why: String,
}

impl std::fmt::Display for Unresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "op {} ({}): {}", self.op, self.kind, self.why)
    }
}

fn link<'g>(shim: &dyn Shim, op: &'g Op, g: &TensorGraph) -> Result<Linked<'g>, String> {
    match allows(shim, op, g) {
        Allow::Native => Ok(Linked::Whole(op)),
        Allow::Decompose => {
            let Sym::Fused(f) = &op.sym else { unreachable!("only a fused op decomposes") };
            f.parts.iter().map(|p| link(shim, p, g)).collect::<Result<_, _>>().map(Linked::Parts)
        }
        Allow::Refuse(why) => Err(why),
    }
}

/// Every op of `g` as `shim`'s chip runs it, or every op it cannot run.
pub fn resolve<'g>(g: &'g TensorGraph, shim: &dyn Shim) -> Result<Vec<Linked<'g>>, Vec<Unresolved>> {
    let mut linked = Vec::with_capacity(g.ops.len());
    let mut unresolved = Vec::new();
    for (i, op) in g.ops.iter().enumerate() {
        match link(shim, op, g) {
            Ok(l) => linked.push(l),
            Err(why) => unresolved.push(Unresolved {
                op: i,
                class: match &op.sym {
                    Sym::Fused(f) => Some(f.class),
                    _ => None,
                },
                kind: op.kind().to_string(),
                why,
            }),
        }
    }
    if unresolved.is_empty() {
        Ok(linked)
    } else {
        Err(unresolved)
    }
}
