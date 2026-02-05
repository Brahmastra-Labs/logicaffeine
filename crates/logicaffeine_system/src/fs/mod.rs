//! Virtual File System Abstraction
//!
//! Provides platform-agnostic async file operations through the [`Vfs`] trait.
//! This enables the same code to work on native platforms and in the browser.
//!
//! # Platform Implementations
//!
//! - **Native** ([`NativeVfs`]): Uses `tokio::fs` with atomic write operations
//! - **WASM** ([`OpfsVfs`]): Uses the browser's Origin Private File System API
//!
//! # Features
//!
//! Requires the `persistence` feature.
//!
//! # Example
//!
//! ```rust,ignore
//! use logicaffeine_system::fs::{Vfs, NativeVfs};
//! use std::sync::Arc;
//!
//! let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(NativeVfs::new("/data"));
//!
//! // Write and read files
//! vfs.write("config.json", b"{}").await?;
//! let data = vfs.read("config.json").await?;
//!
//! // Atomic append for journaling
//! vfs.append("log.txt", b"entry\n").await?;
//! ```

#[cfg(target_arch = "wasm32")]
mod opfs;

#[cfg(target_arch = "wasm32")]
mod worker_opfs;

#[cfg(target_arch = "wasm32")]
mod indexeddb_vfs;

#[cfg(target_arch = "wasm32")]
pub use opfs::OpfsVfs;

#[cfg(target_arch = "wasm32")]
pub use worker_opfs::WorkerOpfsVfs;

#[cfg(target_arch = "wasm32")]
pub use indexeddb_vfs::IndexedDbVfs;

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

/// A directory entry returned by `list_dir`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    /// Name of the file or directory (not full path).
    pub name: String,
    /// True if this entry is a directory.
    pub is_directory: bool,
}

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

    /// Atomically rename a file (for journal compaction).
    async fn rename(&self, from: &str, to: &str) -> VfsResult<()>;

    /// List entries in a directory.
    async fn list_dir(&self, path: &str) -> VfsResult<Vec<DirEntry>>;
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

    /// Atomically rename a file (for journal compaction).
    async fn rename(&self, from: &str, to: &str) -> VfsResult<()>;

    /// List entries in a directory.
    async fn list_dir(&self, path: &str) -> VfsResult<Vec<DirEntry>>;
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

    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        let from_path = self.resolve(from);
        let to_path = self.resolve(to);
        tokio::fs::rename(&from_path, &to_path).await.map_err(VfsError::from)
    }

    async fn list_dir(&self, path: &str) -> VfsResult<Vec<DirEntry>> {
        let full_path = self.resolve(path);
        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&full_path).await.map_err(VfsError::from)?;

        while let Some(entry) = read_dir.next_entry().await.map_err(VfsError::from)? {
            let metadata = entry.metadata().await.map_err(VfsError::from)?;
            entries.push(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_directory: metadata.is_dir(),
            });
        }

        // Sort entries: directories first, then alphabetically
        entries.sort_by(|a, b| {
            match (a.is_directory, b.is_directory) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            }
        });

        Ok(entries)
    }
}

/// Type alias for platform-specific VFS.
#[cfg(not(target_arch = "wasm32"))]
pub type PlatformVfs = NativeVfs;

#[cfg(target_arch = "wasm32")]
pub type PlatformVfs = WorkerOpfsVfs;

/// Get the platform-default VFS instance.
///
/// - Native: Returns NativeVfs rooted at current directory
/// - WASM: Returns WorkerOpfsVfs backed by a Web Worker (Safari-compatible)
#[cfg(not(target_arch = "wasm32"))]
pub fn get_platform_vfs() -> NativeVfs {
    NativeVfs::new(".")
}

#[cfg(target_arch = "wasm32")]
pub fn get_platform_vfs() -> VfsResult<WorkerOpfsVfs> {
    WorkerOpfsVfs::new()
}

/// Enum wrapping both OPFS and IndexedDB VFS implementations.
///
/// Used for transparent fallback when OPFS is unavailable (e.g., Private Browsing).
#[cfg(target_arch = "wasm32")]
#[derive(Clone)]
pub enum WebVfs {
    /// Primary: OPFS via Web Worker (best performance, largest quota)
    Opfs(WorkerOpfsVfs),
    /// Fallback: IndexedDB (works in Private Browsing, session-scoped)
    IndexedDb(IndexedDbVfs),
}

#[cfg(target_arch = "wasm32")]
impl WebVfs {
    /// Returns true if using the IndexedDB fallback.
    pub fn is_fallback(&self) -> bool {
        matches!(self, WebVfs::IndexedDb(_))
    }

    /// Returns a human-readable name for the current storage backend.
    pub fn backend_name(&self) -> &'static str {
        match self {
            WebVfs::Opfs(_) => "OPFS",
            WebVfs::IndexedDb(_) => "IndexedDB",
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
impl Vfs for WebVfs {
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>> {
        match self {
            WebVfs::Opfs(vfs) => vfs.read(path).await,
            WebVfs::IndexedDb(vfs) => vfs.read(path).await,
        }
    }

    async fn read_to_string(&self, path: &str) -> VfsResult<String> {
        match self {
            WebVfs::Opfs(vfs) => vfs.read_to_string(path).await,
            WebVfs::IndexedDb(vfs) => vfs.read_to_string(path).await,
        }
    }

    async fn write(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        match self {
            WebVfs::Opfs(vfs) => vfs.write(path, contents).await,
            WebVfs::IndexedDb(vfs) => vfs.write(path, contents).await,
        }
    }

    async fn append(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        match self {
            WebVfs::Opfs(vfs) => vfs.append(path, contents).await,
            WebVfs::IndexedDb(vfs) => vfs.append(path, contents).await,
        }
    }

    async fn exists(&self, path: &str) -> VfsResult<bool> {
        match self {
            WebVfs::Opfs(vfs) => vfs.exists(path).await,
            WebVfs::IndexedDb(vfs) => vfs.exists(path).await,
        }
    }

    async fn remove(&self, path: &str) -> VfsResult<()> {
        match self {
            WebVfs::Opfs(vfs) => vfs.remove(path).await,
            WebVfs::IndexedDb(vfs) => vfs.remove(path).await,
        }
    }

    async fn create_dir_all(&self, path: &str) -> VfsResult<()> {
        match self {
            WebVfs::Opfs(vfs) => vfs.create_dir_all(path).await,
            WebVfs::IndexedDb(vfs) => vfs.create_dir_all(path).await,
        }
    }

    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        match self {
            WebVfs::Opfs(vfs) => vfs.rename(from, to).await,
            WebVfs::IndexedDb(vfs) => vfs.rename(from, to).await,
        }
    }

    async fn list_dir(&self, path: &str) -> VfsResult<Vec<DirEntry>> {
        match self {
            WebVfs::Opfs(vfs) => vfs.list_dir(path).await,
            WebVfs::IndexedDb(vfs) => vfs.list_dir(path).await,
        }
    }
}

/// Get platform VFS with automatic fallback.
///
/// Tries OPFS first (best performance), falls back to IndexedDB if OPFS is
/// unavailable (e.g., Private Browsing mode).
///
/// Returns `(WebVfs, is_fallback)` where `is_fallback` is true if using IndexedDB.
#[cfg(target_arch = "wasm32")]
pub async fn get_platform_vfs_with_fallback() -> VfsResult<WebVfs> {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = console)]
        fn log(s: &str);
    }

    log("[VFS] Attempting OPFS initialization...");

    // Try OPFS first
    match WorkerOpfsVfs::new() {
        Ok(opfs) => {
            // Test if OPFS actually works by trying to create a directory
            // This catches Private Browsing mode where OPFS creation succeeds
            // but operations fail
            match opfs.create_dir_all("/").await {
                Ok(_) => {
                    log("[VFS] OPFS initialized successfully");
                    return Ok(WebVfs::Opfs(opfs));
                }
                Err(e) => {
                    log(&format!("[VFS] OPFS test failed: {:?}, trying IndexedDB...", e));
                }
            }
        }
        Err(e) => {
            log(&format!("[VFS] OPFS creation failed: {:?}, trying IndexedDB...", e));
        }
    }

    // Fall back to IndexedDB
    log("[VFS] Falling back to IndexedDB...");
    match IndexedDbVfs::new().await {
        Ok(idb) => {
            log("[VFS] IndexedDB initialized successfully");
            Ok(WebVfs::IndexedDb(idb))
        }
        Err(e) => {
            log(&format!("[VFS] IndexedDB initialization failed: {:?}", e));
            Err(e)
        }
    }
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

    #[tokio::test]
    async fn test_native_vfs_list_dir() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        // Create files and directories
        vfs.write("file1.txt", b"content1").await.unwrap();
        vfs.write("file2.txt", b"content2").await.unwrap();
        vfs.write("subdir/nested.txt", b"nested").await.unwrap();

        // List root directory
        let entries = vfs.list_dir("").await.unwrap();

        // Should have 3 entries: subdir (dir), file1.txt, file2.txt
        assert_eq!(entries.len(), 3);

        // Directory should come first
        assert_eq!(entries[0].name, "subdir");
        assert!(entries[0].is_directory);

        // Files should be alphabetically sorted
        assert_eq!(entries[1].name, "file1.txt");
        assert!(!entries[1].is_directory);
        assert_eq!(entries[2].name, "file2.txt");
        assert!(!entries[2].is_directory);
    }
}
