//! Phase 37: Largo.toml Manifest Parser
//!
//! Parses project manifests for LOGOS build configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Project manifest (Largo.toml)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub package: Package,
    #[serde(default)]
    pub dependencies: HashMap<String, DependencySpec>,
}

/// Package metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default = "default_entry")]
    pub entry: String,
}

/// Dependency specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DependencySpec {
    /// Simple version string: "1.0.0" or URI: "logos:std"
    Simple(String),
    /// Detailed dependency: { version = "1.0", path = "../foo" }
    Detailed(DependencyDetail),
}

impl std::fmt::Display for DependencySpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DependencySpec::Simple(s) => write!(f, "{}", s),
            DependencySpec::Detailed(d) => {
                if let Some(v) = &d.version {
                    write!(f, "{}", v)
                } else if let Some(p) = &d.path {
                    write!(f, "path:{}", p)
                } else if let Some(g) = &d.git {
                    write!(f, "git:{}", g)
                } else {
                    write!(f, "*")
                }
            }
        }
    }
}

/// Detailed dependency specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyDetail {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub git: Option<String>,
}

fn default_version() -> String {
    "0.1.0".to_string()
}

fn default_entry() -> String {
    "src/main.lg".to_string()
}

/// Errors that can occur when loading a manifest
#[derive(Debug)]
pub enum ManifestError {
    Io(std::path::PathBuf, String),
    Parse(std::path::PathBuf, String),
    Serialize(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Io(path, e) => write!(f, "Failed to read {}: {}", path.display(), e),
            ManifestError::Parse(path, e) => write!(f, "Failed to parse {}: {}", path.display(), e),
            ManifestError::Serialize(e) => write!(f, "Failed to serialize manifest: {}", e),
        }
    }
}

impl std::error::Error for ManifestError {}

impl Manifest {
    /// Load manifest from a directory (looks for Largo.toml)
    pub fn load(dir: &Path) -> Result<Self, ManifestError> {
        let path = dir.join("Largo.toml");
        let content = fs::read_to_string(&path)
            .map_err(|e| ManifestError::Io(path.clone(), e.to_string()))?;
        toml::from_str(&content).map_err(|e| ManifestError::Parse(path, e.to_string()))
    }

    /// Create a new manifest with default values
    pub fn new(name: &str) -> Self {
        Manifest {
            package: Package {
                name: name.to_string(),
                version: default_version(),
                description: None,
                authors: Vec::new(),
                entry: default_entry(),
            },
            dependencies: HashMap::new(),
        }
    }

    /// Serialize to TOML string
    pub fn to_toml(&self) -> Result<String, ManifestError> {
        toml::to_string_pretty(self).map_err(|e| ManifestError::Serialize(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_manifest() {
        let toml = r#"
[package]
name = "myproject"
"#;
        let manifest: Manifest = toml::from_str(toml).expect("Should parse minimal manifest");
        assert_eq!(manifest.package.name, "myproject");
        assert_eq!(manifest.package.version, "0.1.0"); // default
        assert_eq!(manifest.package.entry, "src/main.lg"); // default
    }

    #[test]
    fn parse_full_manifest() {
        let toml = r#"
[package]
name = "myproject"
version = "1.0.0"
description = "A test project"
entry = "src/app.lg"
authors = ["Test Author"]

[dependencies]
std = "logos:std"
"#;
        let manifest: Manifest = toml::from_str(toml).expect("Should parse full manifest");
        assert_eq!(manifest.package.name, "myproject");
        assert_eq!(manifest.package.version, "1.0.0");
        assert_eq!(manifest.package.entry, "src/app.lg");
        assert!(manifest.package.description.is_some());
        assert_eq!(manifest.package.authors.len(), 1);
    }

    #[test]
    fn create_new_manifest() {
        let manifest = Manifest::new("testproject");
        assert_eq!(manifest.package.name, "testproject");
        let toml = manifest.to_toml().expect("Should serialize");
        assert!(toml.contains("name = \"testproject\""));
    }

    #[test]
    fn parse_path_dependency() {
        let toml = r#"
[package]
name = "with_deps"

[dependencies]
math = { path = "./math" }
"#;
        let manifest: Manifest = toml::from_str(toml).expect("Should parse path deps");
        assert!(!manifest.dependencies.is_empty());
        match &manifest.dependencies["math"] {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.path.as_deref(), Some("./math"));
            }
            _ => panic!("Expected detailed dependency"),
        }
    }
}
