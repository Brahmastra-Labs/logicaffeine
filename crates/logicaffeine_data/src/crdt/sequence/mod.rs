//! Sequence CRDTs for collaborative lists and text.
//!
//! Provides [`RGA`] and [`YATA`] implementations for conflict-free
//! collaborative editing of ordered sequences.

mod rga;
mod yata;

pub use rga::RGA;
pub use yata::YATA;
