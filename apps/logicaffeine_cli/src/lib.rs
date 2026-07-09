#![doc = include_str!("../README.md")]

pub mod cli;
pub(crate) mod commands;
pub mod compile;
pub mod project;
pub(crate) mod repl;
pub mod ui;

/// Re-export of the LOGOS interface types from the kernel crate.
///
/// Provides access to core types like `World` and `Entity` for
/// integration with the LOGOS runtime.
pub use logicaffeine_kernel::interface;

/// Re-export of analysis utilities from the compile crate.
///
/// Useful for tooling that needs to analyze LOGOS source without
/// performing a full build.
pub use logicaffeine_compile::analysis;

/// Entry point for the CLI.
///
/// Parses command-line arguments and executes the appropriate command.
/// See [`cli::run_cli`] for details.
pub use cli::run_cli;
