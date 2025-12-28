//! Phase 36: Project Module System
//!
//! Provides infrastructure for multi-file LOGOS projects, including:
//! - Module loading from various URI schemes (file:, logos:, https:)
//! - Caching of loaded modules
//! - Standard library embedding

pub mod loader;

pub use loader::{Loader, ModuleSource};
