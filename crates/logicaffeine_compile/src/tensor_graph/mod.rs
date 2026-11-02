//! The tensor graph — programs as tensor ops, and the transforms over them.
//!
//! [`front`] is the front end every tensor backend enters through: a program's tensor statements lifted, in order,
//! into typed ops. [`graph`] is the one IR those ops form, each tensor with its facts, and [`eval`] what a graph
//! computes, equal to the VM. [`symbols`] registers every fusion class, its kind and an exemplar of its pattern.
//! [`fuse`] is the shared optimizer — one table of rules over the graph, reading [`facts`] and asking each chip's
//! [`shim`] what it runs whole; [`numerics`] gives each product the quantized activation its chip reads. [`link`]
//! resolves a graph against a chip's shim before the chip's own link builds it. [`smt`] states what a graph computes as
//! terms, so a rewrite's meaning is an obligation a solver discharges. [`vjp`] differentiates a tensor program in reverse
//! mode, writing its backward pass as ordinary LOGOS.
pub mod eval;
pub mod facts;
pub mod front;
pub mod fuse;
pub mod graph;
pub mod link;
pub mod numerics;
pub mod shim;
pub mod smt;
pub mod symbols;
pub mod vjp;
