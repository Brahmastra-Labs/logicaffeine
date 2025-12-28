//! Phase 36/37: Project Module System
//!
//! Provides infrastructure for multi-file LOGOS projects, including:
//! - Module loading from various URI schemes (file:, logos:, https:)
//! - Caching of loaded modules
//! - Standard library embedding
//! - Project manifests and build orchestration (Phase 37)

pub mod loader;
#[cfg(feature = "cli")]
pub mod manifest;
#[cfg(feature = "cli")]
pub mod build;

pub use loader::{Loader, ModuleSource};
#[cfg(feature = "cli")]
pub use manifest::{Manifest, ManifestError};
#[cfg(feature = "cli")]
pub use build::{build, find_project_root, run, BuildConfig, BuildError, BuildResult};
