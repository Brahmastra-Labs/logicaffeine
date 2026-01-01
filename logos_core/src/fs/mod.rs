//! Phase 53: Virtual File System Abstraction
//!
//! Provides platform-agnostic async file operations.
//! - Native: tokio::fs with atomic operations
//! - WASM: OPFS (Origin Private File System) via web-sys

#[cfg(target_arch = "wasm32")]
mod opfs;

#[cfg(target_arch = "wasm32")]
pub use opfs::OpfsVfs;

use async_trait::async_trait;
use std::io;

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

/// Error type for VFS operations
#[derive(Debug)]
pub enum VfsError {
    NotFound(String),
    PermissionDenied(String),
    AlreadyExists(String),
    IoError(io::Error),
    SerializationError(String),
    JournalCorrupted(String),
}

impl std::fmt::Display for VfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VfsError::NotFound(s) => write!(f, "Not found: {}", s),
            VfsError::PermissionDenied(s) => write!(f, "Permission denied: {}", s),
            VfsError::AlreadyExists(s) => write!(f, "Already exists: {}", s),
            VfsError::IoError(e) => write!(f, "IO error: {}", e),
            VfsError::SerializationError(s) => write!(f, "Serialization error: {}", s),
            VfsError::JournalCorrupted(s) => write!(f, "Journal corrupted: {}", s),
        }
    }
}

impl std::error::Error for VfsError {}

impl From<io::Error> for VfsError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::NotFound => VfsError::NotFound(e.to_string()),
            io::ErrorKind::PermissionDenied => VfsError::PermissionDenied(e.to_string()),
            io::ErrorKind::AlreadyExists => VfsError::AlreadyExists(e.to_string()),
            _ => VfsError::IoError(e),
        }
    }
}

pub type VfsResult<T> = Result<T, VfsError>;

/// Virtual File System trait for platform-agnostic file operations.
///
/// On native platforms, requires Send+Sync for thread-safe access.
/// On WASM, these bounds are relaxed since JS is single-threaded.
#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
pub trait Vfs: Send + Sync {
    /// Read entire file contents as bytes.
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>>;

    /// Read file contents as UTF-8 string.
    async fn read_to_string(&self, path: &str) -> VfsResult<String>;

    /// Write bytes to file (atomic on native, best-effort on WASM).
    async fn write(&self, path: &str, contents: &[u8]) -> VfsResult<()>;

    /// Append bytes to file (atomic append semantics).
    async fn append(&self, path: &str, contents: &[u8]) -> VfsResult<()>;

    /// Check if file exists.
    async fn exists(&self, path: &str) -> VfsResult<bool>;

    /// Delete a file.
    async fn remove(&self, path: &str) -> VfsResult<()>;

    /// Create directory and all parent directories.
    async fn create_dir_all(&self, path: &str) -> VfsResult<()>;
}

/// WASM version of VFS trait without Send+Sync (JS is single-threaded).
#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
pub trait Vfs {
    /// Read entire file contents as bytes.
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>>;

    /// Read file contents as UTF-8 string.
    async fn read_to_string(&self, path: &str) -> VfsResult<String>;

    /// Write bytes to file (atomic on native, best-effort on WASM).
    async fn write(&self, path: &str, contents: &[u8]) -> VfsResult<()>;

    /// Append bytes to file (atomic append semantics).
    async fn append(&self, path: &str, contents: &[u8]) -> VfsResult<()>;

    /// Check if file exists.
    async fn exists(&self, path: &str) -> VfsResult<bool>;

    /// Delete a file.
    async fn remove(&self, path: &str) -> VfsResult<()>;

    /// Create directory and all parent directories.
    async fn create_dir_all(&self, path: &str) -> VfsResult<()>;
}

/// Native filesystem VFS using tokio::fs.
#[cfg(not(target_arch = "wasm32"))]
pub struct NativeVfs {
    /// Base directory for all operations (sandbox root).
    base_dir: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl NativeVfs {
    /// Create a new NativeVfs rooted at the given directory.
    pub fn new<P: Into<PathBuf>>(base_dir: P) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Resolve a virtual path to an absolute filesystem path.
    fn resolve(&self, path: &str) -> PathBuf {
        // Security: Prevent path traversal attacks
        let clean = path.trim_start_matches('/').trim_start_matches("../");
        self.base_dir.join(clean)
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
impl Vfs for NativeVfs {
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>> {
        let full_path = self.resolve(path);
        tokio::fs::read(&full_path).await.map_err(VfsError::from)
    }

    async fn read_to_string(&self, path: &str) -> VfsResult<String> {
        let full_path = self.resolve(path);
        tokio::fs::read_to_string(&full_path).await.map_err(VfsError::from)
    }

    async fn write(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        let full_path = self.resolve(path);

        // Ensure parent directory exists
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // Atomic write: write to temp file, then rename
        let temp_path = full_path.with_extension("tmp");
        tokio::fs::write(&temp_path, contents).await?;
        tokio::fs::rename(&temp_path, &full_path).await?;

        Ok(())
    }

    async fn append(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        use tokio::io::AsyncWriteExt;

        let full_path = self.resolve(path);

        // Ensure parent directory exists
        if let Some(parent) = full_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&full_path)
            .await?;

        file.write_all(contents).await?;
        file.sync_all().await?;

        Ok(())
    }

    async fn exists(&self, path: &str) -> VfsResult<bool> {
        let full_path = self.resolve(path);
        Ok(full_path.exists())
    }

    async fn remove(&self, path: &str) -> VfsResult<()> {
        let full_path = self.resolve(path);
        tokio::fs::remove_file(&full_path).await.map_err(VfsError::from)
    }

    async fn create_dir_all(&self, path: &str) -> VfsResult<()> {
        let full_path = self.resolve(path);
        tokio::fs::create_dir_all(&full_path).await.map_err(VfsError::from)
    }
}

/// Type alias for platform-specific VFS.
#[cfg(not(target_arch = "wasm32"))]
pub type PlatformVfs = NativeVfs;

#[cfg(target_arch = "wasm32")]
pub type PlatformVfs = OpfsVfs;

/// Get the platform-default VFS instance.
///
/// - Native: Returns NativeVfs rooted at current directory
/// - WASM: Returns OpfsVfs rooted at OPFS root
#[cfg(not(target_arch = "wasm32"))]
pub fn get_platform_vfs() -> NativeVfs {
    NativeVfs::new(".")
}

#[cfg(target_arch = "wasm32")]
pub async fn get_platform_vfs() -> VfsResult<OpfsVfs> {
    OpfsVfs::new().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_native_vfs_read_write() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        vfs.write("test.txt", b"hello world").await.unwrap();
        let content = vfs.read_to_string("test.txt").await.unwrap();

        assert_eq!(content, "hello world");
    }

    #[tokio::test]
    async fn test_native_vfs_append() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        vfs.append("log.txt", b"line1\n").await.unwrap();
        vfs.append("log.txt", b"line2\n").await.unwrap();

        let content = vfs.read_to_string("log.txt").await.unwrap();
        assert_eq!(content, "line1\nline2\n");
    }

    #[tokio::test]
    async fn test_native_vfs_nested_dirs() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        vfs.write("a/b/c/file.txt", b"deep").await.unwrap();
        let content = vfs.read_to_string("a/b/c/file.txt").await.unwrap();

        assert_eq!(content, "deep");
    }
}
