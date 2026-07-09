//! The Architect's term language — flat, multi-arity, `Copy`.
//!
//! Children are `NodeId`s (e-class references after canonicalization).
//! `Var` carries `(symbol index, version)` — versions partition reads of a
//! mutable name so equality never leaks across a write. `Opaque` is the
//! escape hatch: any expression the e-graph does not model (calls, indexing,
//! collection literals, text) becomes an opaque leaf whose ORIGINAL `&Expr`
//! pointer is held by the converter, so extraction reproduces it verbatim —
//! effects and error behavior preserved by construction.

use super::NodeId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompilerENode {
    Int(i64),
    Bool(bool),
    /// f64 bit pattern. Floats are NEVER arithmetic-rewritten (no identity,
    /// no reassociation — bit-exactness is the contract); the variant exists
    /// so float-valued subtrees still participate in congruence.
    Float(u64),
    /// (symbol index, version)
    Var(u32, u32),
    /// Index into the converter's originals table.
    Opaque(u32),

    Add(NodeId, NodeId),
    Sub(NodeId, NodeId),
    Mul(NodeId, NodeId),
    Div(NodeId, NodeId),
    Mod(NodeId, NodeId),
    Shl(NodeId, NodeId),
    Shr(NodeId, NodeId),
    BitXor(NodeId, NodeId),
    /// Bitwise `&`/`|` on Int (the surface `&`/`|` symbols; also what the
    /// mod-pow2 mask rule synthesizes).
    BitAnd(NodeId, NodeId),
    BitOr(NodeId, NodeId),
    /// Logical `and`/`or`: truthiness in, Bool out, short-circuit. Rules
    /// consult class facts (`is_bool`) before rewriting.
    And(NodeId, NodeId),
    Or(NodeId, NodeId),
    Not(NodeId),

    Eq(NodeId, NodeId),
    Ne(NodeId, NodeId),
    Lt(NodeId, NodeId),
    Le(NodeId, NodeId),
    Gt(NodeId, NodeId),
    Ge(NodeId, NodeId),

    Concat(NodeId, NodeId),
    Len(NodeId),
    Index(NodeId, NodeId),
    /// `copy of xs` — a fresh, unaliased deep copy. Total.
    Copy(NodeId),
    /// `xs sliced from a to b` (1-based, inclusive). NON-total: raises on
    /// out-of-bounds, so rewrites may never delete one without proofs.
    Slice(NodeId, NodeId, NodeId),
    /// `xs contains x` — membership. Total; modeled for congruence/CSE
    /// only (no rewrite touches its semantics).
    Contains(NodeId, NodeId),
}

impl CompilerENode {
    /// Children in evaluation order.
    pub fn children(&self) -> Vec<NodeId> {
        use CompilerENode::*;
        match *self {
            Int(_) | Bool(_) | Float(_) | Var(..) | Opaque(_) => vec![],
            Not(a) | Len(a) | Copy(a) => vec![a],
            Add(a, b) | Sub(a, b) | Mul(a, b) | Div(a, b) | Mod(a, b) | Shl(a, b)
            | Shr(a, b) | BitXor(a, b) | BitAnd(a, b) | BitOr(a, b) | And(a, b) | Or(a, b)
            | Eq(a, b) | Ne(a, b) | Lt(a, b) | Le(a, b) | Gt(a, b) | Ge(a, b)
            | Concat(a, b) | Index(a, b) | Contains(a, b) => {
                vec![a, b]
            }
            Slice(a, b, c) => vec![a, b, c],
        }
    }

    /// Rebuild the node with each child mapped (canonicalization).
    pub fn map_children(self, mut f: impl FnMut(NodeId) -> NodeId) -> Self {
        use CompilerENode::*;
        match self {
            Int(_) | Bool(_) | Float(_) | Var(..) | Opaque(_) => self,
            Not(a) => Not(f(a)),
            Len(a) => Len(f(a)),
            Add(a, b) => Add(f(a), f(b)),
            Sub(a, b) => Sub(f(a), f(b)),
            Mul(a, b) => Mul(f(a), f(b)),
            Div(a, b) => Div(f(a), f(b)),
            Mod(a, b) => Mod(f(a), f(b)),
            Shl(a, b) => Shl(f(a), f(b)),
            Shr(a, b) => Shr(f(a), f(b)),
            BitXor(a, b) => BitXor(f(a), f(b)),
            BitAnd(a, b) => BitAnd(f(a), f(b)),
            BitOr(a, b) => BitOr(f(a), f(b)),
            And(a, b) => And(f(a), f(b)),
            Or(a, b) => Or(f(a), f(b)),
            Eq(a, b) => Eq(f(a), f(b)),
            Ne(a, b) => Ne(f(a), f(b)),
            Lt(a, b) => Lt(f(a), f(b)),
            Le(a, b) => Le(f(a), f(b)),
            Gt(a, b) => Gt(f(a), f(b)),
            Ge(a, b) => Ge(f(a), f(b)),
            Concat(a, b) => Concat(f(a), f(b)),
            Index(a, b) => Index(f(a), f(b)),
            Copy(a) => Copy(f(a)),
            Slice(a, b, c) => Slice(f(a), f(b), f(c)),
            Contains(a, b) => Contains(f(a), f(b)),
        }
    }

    /// Ops whose evaluation can never raise a runtime error, given total
    /// children. Div/Mod (zero divisor), Index/Slice (bounds), and Opaque
    /// (arbitrary effects) are the non-total ones — a rewrite may only
    /// DELETE a subterm whose whole tree is total, or it would erase the
    /// program's error/effect behavior. Len/Copy/Contains raise KIND
    /// errors over non-collections, so they are non-total HERE and earn
    /// totality through class facts in
    /// [`super::CompilerEGraph::provably_total`].
    pub fn op_is_total(&self) -> bool {
        !matches!(
            self,
            CompilerENode::Div(..)
                | CompilerENode::Mod(..)
                | CompilerENode::Index(..)
                | CompilerENode::Slice(..)
                | CompilerENode::Len(..)
                | CompilerENode::Copy(..)
                | CompilerENode::Contains(..)
                | CompilerENode::Opaque(_)
        )
    }
}
