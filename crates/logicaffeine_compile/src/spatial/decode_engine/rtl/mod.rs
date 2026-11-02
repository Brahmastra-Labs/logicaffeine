//! The decode engine's units as Verilog, each computing exactly what its [`super::numerics`] function computes.

pub mod act_quant;
pub mod array;
pub mod attend;
pub mod column;
pub mod engine;
pub mod fp32;
pub mod gdn;
pub mod lane;
pub mod matvec;
pub mod ram;
pub mod simd;
pub mod stream;
pub mod streaming;
pub mod vector;
pub mod vector_side;
pub mod weights;
