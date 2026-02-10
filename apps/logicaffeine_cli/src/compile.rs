//! LOGOS Compilation Pipeline for CLI
//!
//! Re-exports the compilation API from [`logicaffeine_compile`].
//!
//! This module provides access to the core compilation functions without
//! requiring a direct dependency on the compile crate. The key export is
//! [`compile_project`], which transforms LOGOS source into Rust code.
//!
//! # Architecture
//!
//! The compilation pipeline is implemented in the `logicaffeine_compile` crate.
//! This module simply re-exports those types for convenience within the CLI.
//!
//! # Example
//!
//! ```no_run
//! use std::path::Path;
//! use logicaffeine_cli::compile::compile_project;
//!
//! let rust_code = compile_project(Path::new("src/main.lg"))?;
//! println!("Generated {} bytes of Rust", rust_code.rust_code.len());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

// Re-export compile functions and types from the compile crate
pub use logicaffeine_compile::compile::*;
