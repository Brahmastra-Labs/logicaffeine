//! Phase 39: Credential Management
//!
//! Stores and retrieves API tokens for the package registry.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Credentials storage format
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Credentials {
    /// Map of registry URL -> token
    #[serde(default)]
    pub registries: HashMap<String, String>,
}

impl Credentials {
    /// Load credentials from the default location
    pub fn load() -> Result<Self, CredentialsError> {
        let path = credentials_path().ok_or(CredentialsError::NoConfigDir)?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)
            .map_err(|e| CredentialsError::Io(e.to_string()))?;

        toml::from_str(&content)
            .map_err(|e| CredentialsError::Parse(e.to_string()))
    }

    /// Save credentials to the default location
    pub fn save(&self) -> Result<(), CredentialsError> {
        let path = credentials_path().ok_or(CredentialsError::NoConfigDir)?;

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| CredentialsError::Io(e.to_string()))?;
        }

        let content = toml::to_string_pretty(self)
            .map_err(|e| CredentialsError::Serialize(e.to_string()))?;

        fs::write(&path, content)
            .map_err(|e| CredentialsError::Io(e.to_string()))?;

        // Set restrictive permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            fs::set_permissions(&path, perms)
                .map_err(|e| CredentialsError::Io(e.to_string()))?;
        }

        Ok(())
    }

    /// Get token for a registry
    pub fn get_token(&self, registry_url: &str) -> Option<&str> {
        self.registries.get(registry_url).map(|s| s.as_str())
    }

    /// Set token for a registry
    pub fn set_token(&mut self, registry_url: &str, token: &str) {
        self.registries.insert(registry_url.to_string(), token.to_string());
    }

    /// Remove token for a registry
    pub fn remove_token(&mut self, registry_url: &str) {
        self.registries.remove(registry_url);
    }
}

/// Get the token for a registry, checking env var first then credentials file
pub fn get_token(registry_url: &str) -> Option<String> {
    // Check LOGOS_TOKEN env var first
    if let Ok(token) = std::env::var("LOGOS_TOKEN") {
        if !token.is_empty() {
            return Some(token);
        }
    }

    // Fall back to credentials file
    Credentials::load()
        .ok()
        .and_then(|c| c.get_token(registry_url).map(String::from))
}

/// Get the path to the credentials file
pub fn credentials_path() -> Option<PathBuf> {
    // Check LOGOS_CREDENTIALS_PATH env var first
    if let Ok(path) = std::env::var("LOGOS_CREDENTIALS_PATH") {
        return Some(PathBuf::from(path));
    }

    // Use standard config directory
    dirs::config_dir().map(|p| p.join("logos").join("credentials.toml"))
}

#[derive(Debug)]
pub enum CredentialsError {
    NoConfigDir,
    Io(String),
    Parse(String),
    Serialize(String),
}

impl std::fmt::Display for CredentialsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoConfigDir => write!(f, "Could not determine config directory"),
            Self::Io(e) => write!(f, "I/O error: {}", e),
            Self::Parse(e) => write!(f, "Failed to parse credentials: {}", e),
            Self::Serialize(e) => write!(f, "Failed to serialize credentials: {}", e),
        }
    }
}

impl std::error::Error for CredentialsError {}
