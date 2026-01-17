//! Phase 36/37/39: Project Module System
//!
//! Infrastructure for managing LOGOS projects.
//!
//! This module provides the foundational types for working with LOGOS projects,
//! from manifest parsing through build orchestration to registry publishing.
//!
//! # Submodules
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`manifest`] | Parse and serialize `Largo.toml` manifests |
//! | [`build`][mod@build] | Compile and run LOGOS projects |
//! | [`credentials`] | Store and retrieve API tokens |
//! | [`registry`] | Communicate with the package registry |
//!
//! # Re-exports
//!
//! This module re-exports common types for convenience:
//!
//! - **Manifest**: [`Manifest`], [`ManifestError`]
//! - **Build**: [`build()`], [`run`], [`find_project_root`], [`BuildConfig`], [`BuildResult`], [`BuildError`]
//! - **Credentials**: [`Credentials`], [`get_registry_token`]
//! - **Registry**: [`RegistryClient`], [`create_tarball`], [`is_git_dirty`]
//!
//! # Module Loading
//!
//! The [`Loader`] and [`ModuleSource`] types are re-exported from the compile
//! crate for loading LOGOS modules from various URI schemes.

// Re-export Loader from compile crate (basic file/logos: schemes)
pub use logicaffeine_compile::loader::{Loader, ModuleSource};

// CLI-specific project modules
pub mod manifest;
pub mod build;
pub mod credentials;
pub mod registry;

pub use manifest::{Manifest, ManifestError};
pub use build::{build, find_project_root, run, BuildConfig, BuildError, BuildResult};
pub use credentials::{Credentials, get_token as get_registry_token};
pub use registry::{RegistryClient, create_tarball, is_git_dirty};
