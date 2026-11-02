//! The LOGOS decode engine: a spatial dataflow on one card that holds its sequences' token-rows on chip and
//! streams the weights through them once a pass.
//!
//! [`numerics`] is the engine's arithmetic, bit for bit: what every unit computes, so the hardware is held to it
//! exactly and it is held to the model's own arithmetic within a measured bar. [`model`] runs a checkpoint in it.
//! [`lane`] is the block-scale lane over the integer sums the array actually delivers; [`vector`] is the vector
//! unit's instruction set and the micro-programs it runs; [`program`] is a checkpoint's step as the commands the
//! units run, the list [`model`] interprets; [`machine`] runs it as the jobs the sequencer issues over the card's
//! memory, every byte where it lives; [`twin`] prices a program command by command on the engine's units; [`rtl`]
//! prints the units; [`ofs`] writes the D5005's accelerator build tree around them. [`image`] lays a checkpoint's
//! weights over the card's memory channels in the order a pass reads them; [`physical`] places the rest of a machine's
//! memory over the channels and names the streams of each sequence's state; [`ddr`] prices a pass's streams on those
//! channels.

pub mod capability;
pub mod ddr;
pub mod image;
pub mod lane;
pub mod lower_graph;
pub mod machine;
pub mod model;
pub mod numerics;
pub mod ofs;
pub mod physical;
pub mod program;
pub mod rtl;
pub mod shim;
pub mod twin;
pub mod vector;
