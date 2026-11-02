//! The accelerator backends' shared framework: the analog plane, the Plasticine PCU mesh and the SambaNova RDU
//! all run a program the same way, as a host program with device regions.
//!
//! - [`region`]: the functions a device runs, lowered from the host's own optimized program to control-flow
//!   graphs over checked cells, each held to the VM.
//! - [`lane`], [`multiword`], [`int64`], [`softfloat`], [`cells`]: every cell as a routine over the lane
//!   words a device actually has, with no binary64 or 64-bit unit assumed.
//! - [`c_model`]: the executable device model, generated from the regions and their routines, that a host
//!   binary links on a machine with no accelerator.

pub mod c_model;
pub mod cells;
pub mod int64;
pub mod lane;
pub mod multiword;
pub mod region;
pub mod softfloat;
