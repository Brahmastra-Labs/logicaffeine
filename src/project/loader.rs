//! Phase 36: Module Loader
//!
//! Handles resolution and loading of module sources from various URI schemes:
//! - `file:./path.md` - Local filesystem relative to current file
//! - `logos:std` - Built-in standard library (embedded at compile time)
//! - `https://logicaffeine.dev/...` - Remote registry (Phase 37)

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// A loaded module's source content and metadata.
#[derive(Debug, Clone)]
pub struct ModuleSource {
    /// The source content of the module
    pub content: String,
    /// The resolved path (for error reporting and relative resolution)
    pub path: PathBuf,
}

/// Module loader that handles multiple URI schemes.
///
/// Caches loaded modules to prevent duplicate loading and supports
/// cycle detection through the cache.
pub struct Loader {
    /// Cache of loaded modules (URI -> ModuleSource)
    cache: HashMap<String, ModuleSource>,
    /// Root directory of the project (for relative path resolution)
    root_path: PathBuf,
}

impl Loader {
    /// Creates a new Loader with the given root path.
    pub fn new(root_path: PathBuf) -> Self {
        Loader {
            cache: HashMap::new(),
            root_path,
        }
    }

    /// Resolves a URI to a module source.
    ///
    /// Supports:
    /// - `file:./path.md` - Local filesystem (relative to base_path)
    /// - `logos:std` - Built-in standard library
    /// - `logos:core` - Built-in core types
    /// - `https://logicaffeine.dev/...` - Remote registry (returns error for now)
    pub fn resolve(&mut self, base_path: &Path, uri: &str) -> Result<&ModuleSource, String> {
        // Normalize the URI for caching
        let cache_key = self.normalize_uri(base_path, uri)?;

        // Check cache first
        if self.cache.contains_key(&cache_key) {
            return Ok(&self.cache[&cache_key]);
        }

        // Load based on scheme
        let source = if uri.starts_with("file:") {
            self.load_file(base_path, uri)?
        } else if uri.starts_with("logos:") {
            self.load_intrinsic(uri)?
        } else if uri.starts_with("https://") || uri.starts_with("http://") {
            self.load_remote(uri)?
        } else {
            // Default to file: scheme if no scheme provided
            self.load_file(base_path, &format!("file:{}", uri))?
        };

        // Cache and return
        self.cache.insert(cache_key.clone(), source);
        Ok(&self.cache[&cache_key])
    }

    /// Normalizes a URI for consistent caching.
    fn normalize_uri(&self, base_path: &Path, uri: &str) -> Result<String, String> {
        if uri.starts_with("file:") {
            let path_str = uri.trim_start_matches("file:");
            let base_dir = base_path.parent().unwrap_or(&self.root_path);
            let resolved = base_dir.join(path_str);
            Ok(format!("file:{}", resolved.display()))
        } else {
            Ok(uri.to_string())
        }
    }

    /// Loads a module from the local filesystem.
    fn load_file(&self, base_path: &Path, uri: &str) -> Result<ModuleSource, String> {
        let path_str = uri.trim_start_matches("file:");

        // Resolve relative to the base file's directory
        let base_dir = base_path.parent().unwrap_or(&self.root_path);
        let resolved_path = base_dir.join(path_str);

        // Security: Check that we're not escaping the root path
        // (Basic check - a real implementation would canonicalize paths)
        let canonical_root = self.root_path.canonicalize()
            .unwrap_or_else(|_| self.root_path.clone());

        // Read the file
        let content = fs::read_to_string(&resolved_path)
            .map_err(|e| format!("Failed to read '{}': {}", resolved_path.display(), e))?;

        // Check if escaping root (after we know the file exists)
        if let Ok(canonical_path) = resolved_path.canonicalize() {
            if !canonical_path.starts_with(&canonical_root) {
                return Err(format!(
                    "Security: Cannot load '{}' - path escapes project root",
                    uri
                ));
            }
        }

        Ok(ModuleSource {
            content,
            path: resolved_path,
        })
    }

    /// Loads a built-in module (embedded at compile time).
    fn load_intrinsic(&self, uri: &str) -> Result<ModuleSource, String> {
        let name = uri.trim_start_matches("logos:");

        match name {
            "std" => Ok(ModuleSource {
                content: include_str!("../../assets/std/std.md").to_string(),
                path: PathBuf::from("logos:std"),
            }),
            "core" => Ok(ModuleSource {
                content: include_str!("../../assets/std/core.md").to_string(),
                path: PathBuf::from("logos:core"),
            }),
            _ => Err(format!("Unknown intrinsic module: '{}'", uri)),
        }
    }

    /// Loads a module from a remote URL (Phase 37).
    fn load_remote(&self, uri: &str) -> Result<ModuleSource, String> {
        // Phase 37: Implement actual HTTP fetching with caching and lockfile
        // For now, return an error directing users to use local imports
        Err(format!(
            "Remote module loading not yet implemented for '{}'. \
             Use 'logos fetch' to download dependencies locally first. \
             (Full remote support coming in Phase 37)",
            uri
        ))
    }

    /// Checks if a module has already been loaded (for cycle detection).
    pub fn is_loaded(&self, uri: &str) -> bool {
        self.cache.contains_key(uri)
    }

    /// Returns all loaded module URIs (for debugging).
    pub fn loaded_modules(&self) -> Vec<&str> {
        self.cache.keys().map(|s| s.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_file_scheme_resolution() {
        let temp_dir = tempdir().unwrap();
        let geo_path = temp_dir.path().join("geo.md");
        fs::write(&geo_path, "## Definition\nA Point has:\n    an x, which is Int.\n").unwrap();

        let mut loader = Loader::new(temp_dir.path().to_path_buf());
        let result = loader.resolve(&temp_dir.path().join("main.md"), "file:./geo.md");

        assert!(result.is_ok(), "Should resolve file: scheme: {:?}", result);
        assert!(result.unwrap().content.contains("Point"));
    }

    #[test]
    fn test_logos_std_scheme() {
        let mut loader = Loader::new(PathBuf::from("."));
        let result = loader.resolve(&PathBuf::from("main.md"), "logos:std");

        assert!(result.is_ok(), "Should resolve logos:std: {:?}", result);
    }

    #[test]
    fn test_logos_core_scheme() {
        let mut loader = Loader::new(PathBuf::from("."));
        let result = loader.resolve(&PathBuf::from("main.md"), "logos:core");

        assert!(result.is_ok(), "Should resolve logos:core: {:?}", result);
    }

    #[test]
    fn test_unknown_intrinsic() {
        let mut loader = Loader::new(PathBuf::from("."));
        let result = loader.resolve(&PathBuf::from("main.md"), "logos:unknown");

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown intrinsic"));
    }

    #[test]
    fn test_caching() {
        let temp_dir = tempdir().unwrap();
        let geo_path = temp_dir.path().join("geo.md");
        fs::write(&geo_path, "content").unwrap();

        let mut loader = Loader::new(temp_dir.path().to_path_buf());

        // First load
        let _ = loader.resolve(&temp_dir.path().join("main.md"), "file:./geo.md");

        // Should be cached now
        assert!(loader.loaded_modules().len() == 1);
    }

    #[test]
    fn test_missing_file() {
        let temp_dir = tempdir().unwrap();
        let mut loader = Loader::new(temp_dir.path().to_path_buf());

        let result = loader.resolve(&temp_dir.path().join("main.md"), "file:./nonexistent.md");

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to read"));
    }
}
