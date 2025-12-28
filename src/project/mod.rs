//! Phase 36/37/39: Project Module System
//!
//! Provides infrastructure for multi-file LOGOS projects, including:
//! - Module loading from various URI schemes (file:, logos:, https:)
//! - Caching of loaded modules
//! - Standard library embedding
//! - Project manifests and build orchestration (Phase 37)
//! - Package registry client and credentials (Phase 39)

pub mod loader;
#[cfg(feature = "cli")]
pub mod manifest;
#[cfg(feature = "cli")]
pub mod build;
#[cfg(feature = "cli")]
pub mod credentials;
#[cfg(feature = "cli")]
pub mod registry;

pub use loader::{Loader, ModuleSource};
#[cfg(feature = "cli")]
pub use manifest::{Manifest, ManifestError};
#[cfg(feature = "cli")]
pub use build::{build, find_project_root, run, BuildConfig, BuildError, BuildResult};
#[cfg(feature = "cli")]
pub use credentials::{Credentials, get_token as get_registry_token};
#[cfg(feature = "cli")]
pub use registry::{RegistryClient, create_tarball, is_git_dirty};
