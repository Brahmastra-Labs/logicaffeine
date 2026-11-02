//! The board's own CPU: an RV64IMAC hart with machine and user modes, built to run the LOGOS compiler from the board's
//! DDR2 so the board compiles programs for its fabric engine with no computer attached.
//!
//! [`model`] is the hart's executable specification and [`rvc`] its compressed-instruction decompressor; [`rtl`] is the
//! Verilog held to them. [`soc`] defines the system around the hart — its memory map, device registers and DDR2
//! layout — once, for the model, the RTL and the firmware's C; [`image`] is the boot image the flash carries; [`ddr2`]
//! is the DDR2 behind it, as the controller schedules it.

pub mod ddr2;
pub mod image;
pub mod model;
pub mod rtl;
pub mod rvc;
pub mod soc;
