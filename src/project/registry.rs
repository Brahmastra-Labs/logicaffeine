//! Phase 39: Registry Client
//!
//! HTTP client for communicating with the LOGOS package registry.

use std::path::Path;

const DEFAULT_REGISTRY_URL: &str = "https://registry.logicaffeine.com";

/// Registry client for API communication
pub struct RegistryClient {
    base_url: String,
    token: String,
}

impl RegistryClient {
    pub fn new(base_url: &str, token: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    pub fn default_url() -> &'static str {
        DEFAULT_REGISTRY_URL
    }

    /// Validate the authentication token
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

    /// Publish a package to the registry
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

/// Create a tarball from a LOGOS project
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

/// Check if git working directory is dirty
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

#[derive(Debug, serde::Deserialize)]
pub struct UserInfo {
    pub id: String,
    pub login: String,
    pub name: Option<String>,
    pub is_admin: bool,
}

#[derive(Debug, serde::Serialize)]
pub struct PublishMetadata {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub repository: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub keywords: Vec<String>,
    pub entry_point: String,
    pub dependencies: std::collections::HashMap<String, String>,
    pub readme: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct PublishResult {
    pub success: bool,
    pub package: String,
    pub version: String,
    pub sha256: String,
    pub size: u64,
}

// ============== Errors ==============

#[derive(Debug)]
pub enum RegistryError {
    NoToken,
    Unauthorized,
    Forbidden(String),
    VersionExists { name: String, version: String },
    TooLarge,
    Network(String),
    Server { status: u16, message: String },
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

#[derive(Debug)]
pub enum PackageError {
    MissingFile(String),
    Io(String),
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
