//! Phase 39: Registry Client
//!
//! HTTP client for communicating with the LOGOS package registry.
//!
//! This module provides the [`RegistryClient`] for authenticated API calls to
//! the package registry, along with supporting types for package metadata and
//! error handling.
//!
//! # Architecture
//!
//! The registry client uses [`ureq`] for HTTP requests with Bearer token
//! authentication. All requests are made over HTTPS to the configured registry
//! URL (defaulting to `registry.logicaffeine.com`).
//!
//! # Example
//!
//! ```no_run
//! use logicaffeine_cli::project::registry::{RegistryClient, PublishMetadata};
//!
//! let client = RegistryClient::new("https://registry.logicaffeine.com", "tok_xxx");
//!
//! // Validate authentication
//! let user = client.validate_token()?;
//! println!("Authenticated as: {}", user.login);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::path::Path;

const DEFAULT_REGISTRY_URL: &str = "https://registry.logicaffeine.com";

/// HTTP client for the LOGOS package registry API.
///
/// Provides authenticated access to registry operations including:
/// - Token validation
/// - Package publishing
///
/// # Authentication
///
/// All API calls require a Bearer token, typically obtained via `largo login`.
/// Tokens are validated against the registry's `/auth/me` endpoint.
///
/// # Example
///
/// ```no_run
/// use logicaffeine_cli::project::registry::RegistryClient;
///
/// let client = RegistryClient::new(
///     RegistryClient::default_url(),
///     "tok_xxxxx"
/// );
///
/// // Verify the token is valid
/// match client.validate_token() {
///     Ok(user) => println!("Logged in as {}", user.login),
///     Err(e) => eprintln!("Auth failed: {}", e),
/// }
/// ```
pub struct RegistryClient {
    /// Base URL of the registry API (without trailing slash).
    base_url: String,
    /// Bearer token for authentication.
    token: String,
}

impl RegistryClient {
    /// Create a new registry client with the given URL and authentication token.
    ///
    /// # Arguments
    ///
    /// * `base_url` - The registry API base URL. Trailing slashes are stripped.
    /// * `token` - Bearer token for authentication.
    ///
    /// # Example
    ///
    /// ```
    /// use logicaffeine_cli::project::registry::RegistryClient;
    ///
    /// let client = RegistryClient::new(
    ///     "https://registry.logicaffeine.com",
    ///     "tok_xxxxx"
    /// );
    /// ```
    pub fn new(base_url: &str, token: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    /// Returns the default registry URL.
    ///
    /// Currently returns `https://registry.logicaffeine.com`.
    pub fn default_url() -> &'static str {
        DEFAULT_REGISTRY_URL
    }

    /// Validate the authentication token by querying the registry.
    ///
    /// Makes a request to `/auth/me` to verify the token is valid and
    /// retrieve the associated user information.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError::Unauthorized`] if the token is invalid or expired.
    /// Returns [`RegistryError::Network`] for connection failures.
    pub fn validate_token(&self) -> Result<UserInfo, RegistryError> {
        let url = format!("{}/auth/me", self.base_url);

        let response = ureq::get(&url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(401, _) => RegistryError::Unauthorized,
                ureq::Error::Status(403, r) => {
                    let msg = r.into_string().unwrap_or_default();
                    RegistryError::Forbidden(msg)
                }
                ureq::Error::Status(code, r) => RegistryError::Server {
                    status: code,
                    message: r.into_string().unwrap_or_default(),
                },
                e => RegistryError::Network(e.to_string()),
            })?;

        let user: UserInfo = response.into_json()
            .map_err(|e| RegistryError::Network(e.to_string()))?;

        Ok(user)
    }

    /// Publish a package to the registry.
    ///
    /// Uploads a package tarball with metadata to the registry's publish endpoint.
    /// The request is sent as multipart form data.
    ///
    /// # Arguments
    ///
    /// * `name` - Package name (must match manifest)
    /// * `version` - Semantic version string
    /// * `tarball` - Gzipped tar archive of the package
    /// * `metadata` - Package metadata for the registry index
    ///
    /// # Errors
    ///
    /// - [`RegistryError::Unauthorized`] - Invalid or missing token
    /// - [`RegistryError::VersionExists`] - This version already published
    /// - [`RegistryError::TooLarge`] - Package exceeds 10MB limit
    /// - [`RegistryError::InvalidPackage`] - Metadata serialization failed
    pub fn publish(
        &self,
        name: &str,
        version: &str,
        tarball: &[u8],
        metadata: &PublishMetadata,
    ) -> Result<PublishResult, RegistryError> {
        use std::io::Read;

        let url = format!("{}/packages/publish", self.base_url);

        // Create multipart form data
        let boundary = format!("----LargoBoundary{}", rand::random::<u64>());

        let metadata_json = serde_json::to_string(metadata)
            .map_err(|e| RegistryError::InvalidPackage(e.to_string()))?;

        let mut body = Vec::new();

        // Add metadata field
        body.extend_from_slice(format!(
            "--{}\r\nContent-Disposition: form-data; name=\"metadata\"\r\n\r\n{}\r\n",
            boundary, metadata_json
        ).as_bytes());

        // Add tarball field
        body.extend_from_slice(format!(
            "--{}\r\nContent-Disposition: form-data; name=\"tarball\"; filename=\"{}-{}.tar.gz\"\r\nContent-Type: application/gzip\r\n\r\n",
            boundary, name, version
        ).as_bytes());
        body.extend_from_slice(tarball);
        body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

        let response = ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Content-Type", &format!("multipart/form-data; boundary={}", boundary))
            .send_bytes(&body)
            .map_err(|e| match e {
                ureq::Error::Status(401, _) => RegistryError::Unauthorized,
                ureq::Error::Status(403, r) => {
                    let msg = r.into_string().unwrap_or_else(|_| "Forbidden".to_string());
                    RegistryError::Forbidden(msg)
                }
                ureq::Error::Status(409, _) => RegistryError::VersionExists {
                    name: name.to_string(),
                    version: version.to_string(),
                },
                ureq::Error::Status(413, _) => RegistryError::TooLarge,
                ureq::Error::Status(code, r) => RegistryError::Server {
                    status: code,
                    message: r.into_string().unwrap_or_default(),
                },
                e => RegistryError::Network(e.to_string()),
            })?;

        let result: PublishResult = response.into_json()
            .map_err(|e| RegistryError::Network(e.to_string()))?;

        Ok(result)
    }
}

/// Create a gzipped tarball from a LOGOS project.
///
/// Packages the project for upload to the registry. The tarball includes:
/// - `Largo.toml` (required)
/// - `src/` directory recursively (required)
/// - `README.md` (if present)
/// - `LICENSE` (if present)
///
/// Hidden files (starting with `.`) and the `target/` directory are excluded.
/// Only `.lg`, `.md`, `.toml`, and `.json` files are included from `src/`.
///
/// # Arguments
///
/// * `project_dir` - Root directory of the LOGOS project
///
/// # Errors
///
/// Returns [`PackageError::MissingFile`] if `Largo.toml` or `src/` is missing.
pub fn create_tarball(project_dir: &Path) -> Result<Vec<u8>, PackageError> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use tar::Builder;
    use std::fs::File;
    use std::io::Write;

    let mut tarball = Vec::new();

    {
        let encoder = GzEncoder::new(&mut tarball, Compression::default());
        let mut builder = Builder::new(encoder);

        // Add Largo.toml
        let manifest_path = project_dir.join("Largo.toml");
        if !manifest_path.exists() {
            return Err(PackageError::MissingFile("Largo.toml".to_string()));
        }
        add_file_to_tar(&mut builder, project_dir, "Largo.toml")?;

        // Add src/ directory recursively
        let src_dir = project_dir.join("src");
        if !src_dir.exists() {
            return Err(PackageError::MissingFile("src/".to_string()));
        }
        add_dir_recursive(&mut builder, project_dir, "src")?;

        // Add README.md if it exists
        if project_dir.join("README.md").exists() {
            add_file_to_tar(&mut builder, project_dir, "README.md")?;
        }

        // Add LICENSE if it exists
        if project_dir.join("LICENSE").exists() {
            add_file_to_tar(&mut builder, project_dir, "LICENSE")?;
        }

        builder.finish()
            .map_err(|e| PackageError::TarError(e.to_string()))?;
    }

    Ok(tarball)
}

fn add_file_to_tar<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    base_dir: &Path,
    rel_path: &str,
) -> Result<(), PackageError> {
    let full_path = base_dir.join(rel_path);
    let content = std::fs::read(&full_path)
        .map_err(|e| PackageError::Io(format!("{}: {}", rel_path, e)))?;

    let mut header = tar::Header::new_gnu();
    header.set_path(rel_path)
        .map_err(|e| PackageError::TarError(e.to_string()))?;
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_mtime(0); // Reproducible builds
    header.set_cksum();

    builder.append(&header, content.as_slice())
        .map_err(|e| PackageError::TarError(e.to_string()))?;

    Ok(())
}

fn add_dir_recursive<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    base_dir: &Path,
    rel_dir: &str,
) -> Result<(), PackageError> {
    let full_dir = base_dir.join(rel_dir);

    for entry in std::fs::read_dir(&full_dir)
        .map_err(|e| PackageError::Io(format!("{}: {}", rel_dir, e)))?
    {
        let entry = entry.map_err(|e| PackageError::Io(e.to_string()))?;
        let path = entry.path();
        let name = entry.file_name();
        let rel_path = format!("{}/{}", rel_dir, name.to_string_lossy());

        // Skip hidden files and target directory
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') || name_str == "target" {
            continue;
        }

        if path.is_dir() {
            add_dir_recursive(builder, base_dir, &rel_path)?;
        } else if path.is_file() {
            // Only include .lg, .md, and common config files
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "lg" | "md" | "toml" | "json") || name_str == "LICENSE" {
                add_file_to_tar(builder, base_dir, &rel_path)?;
            }
        }
    }

    Ok(())
}

/// Check if the git working directory has uncommitted changes.
///
/// Runs `git status --porcelain` and returns `true` if there is any output,
/// indicating uncommitted changes (modified, staged, or untracked files).
///
/// Returns `false` if:
/// - The directory is not a git repository
/// - Git is not available on the system
/// - The working directory is clean
///
/// # Arguments
///
/// * `project_dir` - Directory to check (should contain `.git`)
pub fn is_git_dirty(project_dir: &Path) -> bool {
    use std::process::Command;

    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(project_dir)
        .output();

    match output {
        Ok(out) if out.status.success() => !out.stdout.is_empty(),
        _ => false, // Not a git repo or git not available
    }
}

// ============== Types ==============

/// User information returned from the registry's `/auth/me` endpoint.
///
/// Contains details about the authenticated user, used to confirm
/// successful login and display user information.
#[derive(Debug, serde::Deserialize)]
pub struct UserInfo {
    /// Unique user identifier in the registry.
    pub id: String,
    /// GitHub username (used for login).
    pub login: String,
    /// Display name (may differ from login).
    pub name: Option<String>,
    /// Whether the user has registry admin privileges.
    pub is_admin: bool,
}

/// Metadata submitted when publishing a package.
///
/// This information is stored in the registry index and displayed
/// on the package's registry page.
#[derive(Debug, serde::Serialize)]
pub struct PublishMetadata {
    /// Package name (must match `Largo.toml`).
    pub name: String,
    /// Semantic version string (e.g., "1.0.0").
    pub version: String,
    /// Short description of the package.
    pub description: Option<String>,
    /// URL to the source repository (e.g., GitHub).
    pub repository: Option<String>,
    /// URL to the project homepage or documentation.
    pub homepage: Option<String>,
    /// SPDX license identifier (e.g., "MIT", "Apache-2.0").
    pub license: Option<String>,
    /// Searchable keywords for discovery.
    pub keywords: Vec<String>,
    /// Relative path to the entry point file.
    pub entry_point: String,
    /// Map of dependency names to version requirements.
    pub dependencies: std::collections::HashMap<String, String>,
    /// Full README content (if `README.md` exists).
    pub readme: Option<String>,
}

/// Response from a successful publish operation.
///
/// Returned by the registry after a package is successfully uploaded
/// and indexed.
#[derive(Debug, serde::Deserialize)]
pub struct PublishResult {
    /// Whether the publish succeeded.
    pub success: bool,
    /// The published package name.
    pub package: String,
    /// The published version.
    pub version: String,
    /// SHA-256 hash of the uploaded tarball.
    pub sha256: String,
    /// Size of the tarball in bytes.
    pub size: u64,
}

// ============== Errors ==============

/// Errors that can occur during registry API operations.
///
/// Each variant includes a user-friendly error message with guidance
/// on how to resolve the issue.
#[derive(Debug)]
pub enum RegistryError {
    /// No authentication token was provided or found.
    NoToken,
    /// The provided token is invalid or expired (HTTP 401).
    Unauthorized,
    /// The server rejected the request (HTTP 403).
    Forbidden(String),
    /// The package version already exists in the registry (HTTP 409).
    VersionExists {
        /// Package name.
        name: String,
        /// Version that already exists.
        version: String,
    },
    /// The package tarball exceeds the size limit (HTTP 413).
    TooLarge,
    /// Network or connection error.
    Network(String),
    /// The server returned an unexpected error.
    Server {
        /// HTTP status code.
        status: u16,
        /// Error message from the server.
        message: String,
    },
    /// The package metadata could not be serialized.
    InvalidPackage(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoToken => write!(
                f,
                "No authentication token found.\n\
                 Run 'largo login' or set LOGOS_TOKEN environment variable."
            ),
            Self::Unauthorized => write!(
                f,
                "Authentication failed. Your token may be invalid or expired.\n\
                 Run 'largo login' to get a new token."
            ),
            Self::Forbidden(msg) => write!(f, "Access denied: {}", msg),
            Self::VersionExists { name, version } => write!(
                f,
                "Version {} of package '{}' already exists.\n\
                 Update the version in Largo.toml and try again.",
                version, name
            ),
            Self::TooLarge => write!(f, "Package too large. Maximum size is 10MB."),
            Self::Network(e) => write!(f, "Network error: {}", e),
            Self::Server { status, message } => {
                write!(f, "Registry returned error {}: {}", status, message)
            }
            Self::InvalidPackage(e) => write!(f, "Invalid package: {}", e),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Errors that can occur when creating a package tarball.
#[derive(Debug)]
pub enum PackageError {
    /// A required file is missing from the project.
    MissingFile(String),
    /// A file system operation failed.
    Io(String),
    /// The tar archive could not be created.
    TarError(String),
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingFile(name) => write!(f, "Missing required file: {}", name),
            Self::Io(e) => write!(f, "I/O error: {}", e),
            Self::TarError(e) => write!(f, "Failed to create tarball: {}", e),
        }
    }
}

impl std::error::Error for PackageError {}
