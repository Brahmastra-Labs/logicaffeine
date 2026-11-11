//! What a chip allows: the one interface every chip answers through.
//!
//! The shared optimizer decides a fusion once, for every chip, by dataflow alone; a chip's shim says whether it runs the
//! fused op whole. The answer is data the chip declares, consulted before a rule commits, so a chip never receives a
//! fused op it cannot run:
//!
//!   • **native** — the chip has a primitive that computes the fused op whole;
//!   • **decompose** — the chip runs the fused op as its parts, each of which it allows;
//!   • **refuse** — the chip runs neither, and says why.

use super::fuse::ChipRule;
use super::graph::{Op, QuantFormat, Sym, TensorGraph};
use super::numerics::{Bound, Contract, Keep};
use super::symbols::ClassId;

/// A chip's answer for one op.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Allow {
    Native,
    Decompose,
    Refuse(String),
}

impl Allow {
    pub fn is_native(&self) -> bool {
        matches!(self, Allow::Native)
    }
}

/// What a chip's link does with a class wherever it meets one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    /// A primitive of the chip's own computes it whole; the chip then judges each op's shape ([`Shim::fits`]).
    Native,
    /// The chip runs its parts, each as it runs it alone.
    Decompose,
    /// Neither, and why.
    Refuse(&'static str),
}

/// What a chip allows.
pub trait Shim {
    /// The chip and the configuration it answers for, e.g. `cuda-lane:rtx3060:TwoPassInt8`.
    fn name(&self) -> String;

    /// What the chip's link does with `class`. Every chip answers by a match over every class with no catch-all arm,
    /// so a class added to the registry fails the build until each chip answers it.
    fn supports(&self, class: ClassId) -> Support;

    /// Whether the chip's link builds `op` as it stands — its shapes, its accesses, its destination: `Ok`, or why not.
    fn fits(&self, op: &Op, g: &TensorGraph) -> Result<(), String>;

    /// The quantized form `consumer` reads its operand `operand` in, or `None` for one it reads as its values.
    fn activation(&self, _consumer: &Op, _operand: usize, _g: &TensorGraph) -> Option<QuantFormat> {
        None
    }

    /// The chip's own rules, each linked into the table where its position says.
    fn rules(&self) -> Vec<(Position, ChipRule<'_>)> {
        Vec::new()
    }

    /// The numerics contract of the knob point the chip answers for, or `None` while the chip has not declared it.
    fn numerics(&self) -> Option<Contract>;
}

/// Whether the chip runs `op`: a fused op of a class it links whole that its link builds as it stands (`Native`), one
/// of a class it runs as its parts, every part allowed (`Decompose`), or neither, by name — a refused part names the
/// part and its own reason. A lifted op, a view or a quantize is judged by its shape alone.
pub fn allows(shim: &dyn Shim, op: &Op, g: &TensorGraph) -> Allow {
    let fit = |op: &Op| shim.fits(op, g).map_or_else(Allow::Refuse, |()| Allow::Native);
    let Sym::Fused(f) = &op.sym else { return fit(op) };
    match shim.supports(f.class) {
        Support::Native => fit(op),
        Support::Decompose => {
            for part in &f.parts {
                if let Allow::Refuse(why) = allows(shim, part, g) {
                    return Allow::Refuse(format!("{} runs as its parts, and its {} is refused: {why}", f.class, part.kind()));
                }
            }
            Allow::Decompose
        }
        Support::Refuse(why) => Allow::Refuse(format!("{}: {why}", f.class)),
    }
}

/// Where a chip's own rule runs in the table: before a shared rule, after one, or after them all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    Before(&'static str),
    After(&'static str),
    End,
}

/// The shim that allows everything: the meaning alone, for graph-level tests. It answers no class by name — every class
/// is one it links whole, whatever the registry adds.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReferenceShim;

impl Shim for ReferenceShim {
    fn name(&self) -> String {
        "reference".to_string()
    }

    fn supports(&self, _: ClassId) -> Support {
        Support::Native
    }

    fn fits(&self, _: &Op, _: &TensorGraph) -> Result<(), String> {
        Ok(())
    }

    fn numerics(&self) -> Option<Contract> {
        Some(Contract { point: "the meaning".to_string(), activations: None, sixteen: None, cache: Keep::Exact, state: Keep::Exact, attention: None, fused: Bound::EXACT, accumulator_bits: 64 })
    }
}

/// A chip as if it linked `classes` whole, whatever their shape, and answered everything else as it does: where a rule
/// building those classes would fire on the chip once its link has them.
pub struct Linking<'a> {
    pub base: &'a dyn Shim,
    pub classes: &'a [ClassId],
}

impl Shim for Linking<'_> {
    fn name(&self) -> String {
        format!("{} linking {:?}", self.base.name(), self.classes)
    }

    fn supports(&self, class: ClassId) -> Support {
        if self.classes.contains(&class) {
            Support::Native
        } else {
            self.base.supports(class)
        }
    }

    fn fits(&self, op: &Op, g: &TensorGraph) -> Result<(), String> {
        match &op.sym {
            Sym::Fused(f) if self.classes.contains(&f.class) => Ok(()),
            _ => self.base.fits(op, g),
        }
    }

    fn activation(&self, consumer: &Op, operand: usize, g: &TensorGraph) -> Option<QuantFormat> {
        self.base.activation(consumer, operand, g)
    }

    fn rules(&self) -> Vec<(Position, ChipRule<'_>)> {
        self.base.rules()
    }

    fn numerics(&self) -> Option<Contract> {
        self.base.numerics()
    }
}
