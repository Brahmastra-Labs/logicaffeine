//! Virtual File System Abstraction
//!
//! Provides platform-agnostic async file operations through the [`Vfs`] trait.
//! This enables the same code to work on native platforms and in the browser.
//!
//! # Platform Implementations
//!
//! - **Native** ([`NativeVfs`]): Uses `tokio::fs` with atomic write operations
//! - **WASM** (`OpfsVfs`): Uses the browser's Origin Private File System API
//!
//! # Features
//!
//! Requires the `persistence` feature.
//!
//! # Example
//!
//! ```no_run
//! use logicaffeine_system::fs::{Vfs, NativeVfs};
//! use std::sync::Arc;
//!
//! # fn main() {}
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let vfs: Arc<dyn Vfs + Send + Sync> = Arc::new(NativeVfs::new("/data"));
//!
//! // Write and read files
//! vfs.write("config.json", b"{}").await?;
//! let data = vfs.read("config.json").await?;
//!
//! // Atomic append for journaling
//! vfs.append("log.txt", b"entry\n").await?;
//! # Ok(())
//! # }
//! ```

#[cfg(all(target_os = "linux", feature = "io-uring"))]
mod uring;
#[cfg(all(target_os = "linux", feature = "io-uring"))]
pub(crate) mod uring_worker;

#[cfg(all(target_os = "linux", feature = "io-uring"))]
pub use uring::UringVfs;

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

/// The sealed test world's VFS:
/// an in-memory filesystem that bridges the [`crate::testworld`] thread-local,
/// so `Given a file …` seeds and Vfs-backed `Read`/`Write` statements share
/// ONE source of truth. Installed per-test by the `largo test` runner; there
/// is no real disk behind it (unless the test grants the filesystem
/// capability, which routes it to a real `NativeVfs` instead). Its body only
/// reads and writes the `testworld` thread-local, so it is equally at home
/// behind either `Vfs` shape — the in-browser guide's page test runner drives
/// it through the `?Send` one.
pub struct MemVfs;

impl MemVfs {
    pub fn new() -> Self {
        MemVfs
    }
}

impl Default for MemVfs {
    fn default() -> Self {
        MemVfs::new()
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
impl Vfs for MemVfs {
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>> {
        self.read_to_string(path).await.map(String::into_bytes)
    }

    async fn read_to_string(&self, path: &str) -> VfsResult<String> {
        crate::testworld::with_world(|w| w.files.get(path).cloned())
            .flatten()
            .ok_or_else(|| VfsError::NotFound(path.to_string()))
    }

    async fn write(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        let text = String::from_utf8_lossy(contents).into_owned();
        crate::testworld::seed_file(path.to_string(), text);
        Ok(())
    }

    async fn append(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        let existing = self.read_to_string(path).await.unwrap_or_default();
        let text = existing + &String::from_utf8_lossy(contents);
        crate::testworld::seed_file(path.to_string(), text);
        Ok(())
    }

    async fn exists(&self, path: &str) -> VfsResult<bool> {
        Ok(crate::testworld::with_world(|w| w.files.contains_key(path)).unwrap_or(false))
    }

    async fn remove(&self, path: &str) -> VfsResult<()> {
        crate::testworld::with_world(|w| w.files.remove(path));
        Ok(())
    }

    async fn create_dir_all(&self, _path: &str) -> VfsResult<()> {
        Ok(()) // directories are implicit in the flat in-memory map
    }

    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        let content = self.read_to_string(from).await?;
        crate::testworld::seed_file(to.to_string(), content);
        crate::testworld::with_world(|w| w.files.remove(from));
        Ok(())
    }

    async fn list_dir(&self, path: &str) -> VfsResult<Vec<DirEntry>> {
        let prefix = if path.is_empty() || path == "/" {
            String::new()
        } else {
            format!("{}/", path.trim_end_matches('/'))
        };
        Ok(crate::testworld::with_world(|w| {
            w.files
                .keys()
                .filter_map(|k| k.strip_prefix(&prefix))
                .filter(|rest| !rest.is_empty() && !rest.contains('/'))
                .map(|name| DirEntry {
                    name: name.to_string(),
                    is_directory: false,
                })
                .collect()
        })
        .unwrap_or_default())
    }
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
        tokio::fs::read_to_string(&full_path)
            .await
            .map_err(VfsError::from)
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
        tokio::fs::remove_file(&full_path)
            .await
            .map_err(VfsError::from)
    }

    async fn create_dir_all(&self, path: &str) -> VfsResult<()> {
        let full_path = self.resolve(path);
        tokio::fs::create_dir_all(&full_path)
            .await
            .map_err(VfsError::from)
    }

    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        let from_path = self.resolve(from);
        let to_path = self.resolve(to);
        tokio::fs::rename(&from_path, &to_path)
            .await
            .map_err(VfsError::from)
    }

    async fn list_dir(&self, path: &str) -> VfsResult<Vec<DirEntry>> {
        let full_path = self.resolve(path);
        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&full_path)
            .await
            .map_err(VfsError::from)?;

        while let Some(entry) = read_dir.next_entry().await.map_err(VfsError::from)? {
            let metadata = entry.metadata().await.map_err(VfsError::from)?;
            entries.push(DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_directory: metadata.is_dir(),
            });
        }

        // Sort entries: directories first, then alphabetically
        entries.sort_by(|a, b| match (a.is_directory, b.is_directory) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
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
/// - Linux with `io-uring` feature: Tries `UringVfs` first for kernel-async I/O,
///   falls back to `NativeVfs` if io_uring is unavailable (old kernel, container).
/// - Other native: Returns `NativeVfs` (tokio::fs)
/// - WASM: Returns `WorkerOpfsVfs` backed by a Web Worker
#[cfg(not(target_arch = "wasm32"))]
pub fn get_platform_vfs() -> Box<dyn Vfs + Send + Sync> {
    #[cfg(all(target_os = "linux", feature = "io-uring"))]
    {
        match UringVfs::new(".") {
            Ok(vfs) => return Box::new(vfs),
            Err(_) => {} // Fall through to NativeVfs
        }
    }
    Box::new(NativeVfs::new("."))
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
                    log(&format!(
                        "[VFS] OPFS test failed: {:?}, trying IndexedDB...",
                        e
                    ));
                }
            }
        }
        Err(e) => {
            log(&format!(
                "[VFS] OPFS creation failed: {:?}, trying IndexedDB...",
                e
            ));
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
#[path = "../../../logicaffeine_tests/unit/logicaffeine_system/fs/tests.rs"]
mod tests;
