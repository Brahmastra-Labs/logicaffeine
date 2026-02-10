//! LOGOS CLI (`largo`)
//!
//! Command-line interface for the LOGOS build system and package registry.
//!
//! This crate provides the `largo` CLI tool for creating, building, and
//! publishing LOGOS projects. It can be used as a library for programmatic
//! access to the build system.
//!
//! # Commands
//!
//! | Command | Description |
//! |---------|-------------|
//! | `largo new` | Create a new project |
//! | `largo init` | Initialize project in current directory |
//! | `largo build` | Compile a LOGOS module to Rust |
//! | `largo run` | Build and execute a module |
//! | `largo check` | Type-check without building |
//! | `largo verify` | Run Z3 static verification |
//! | `largo publish` | Publish package to registry |
//! | `largo login` | Authenticate with registry |
//! | `largo logout` | Remove stored credentials |
//!
//! # Module Structure
//!
//! - [`cli`] - Command-line argument parsing and dispatch
//! - [`compile`] - Re-exports from the compilation pipeline
//! - [`project`] - Project management (manifest, build, registry)
//!   - [`project::manifest`] - `Largo.toml` parsing
//!   - [`project::build`][mod@project::build] - Build orchestration
//!   - [`project::credentials`] - API token storage
//!   - [`project::registry`] - Package registry client
//!
//! # Feature Flags
//!
//! | Feature | Description |
//! |---------|-------------|
//! | `verification` | Enable Z3 static verification (requires Z3 installed) |
//!
//! # Library Usage
//!
//! While `largo` is primarily a CLI tool, the library API can be used
//! for build system integration:
//!
//! ```no_run
//! use logicaffeine_cli::project::{BuildConfig, build, find_project_root};
//! use std::env;
//!
//! let cwd = env::current_dir().unwrap();
//! let root = find_project_root(&cwd).expect("Not in a LOGOS project");
//!
//! let result = build(BuildConfig {
//!     project_dir: root,
//!     release: false,
//!     lib_mode: false,
//!     target: None,
//! })?;
//!
//! println!("Built: {}", result.binary_path.display());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod cli;
pub mod compile;
pub mod project;

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
