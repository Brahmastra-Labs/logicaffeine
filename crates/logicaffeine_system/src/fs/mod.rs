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

    // ─── NativeVfs: exists, remove, rename ───────────────────────────────

    #[tokio::test]
    async fn test_native_vfs_exists_returns_false_for_missing_file() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        assert!(!vfs.exists("nope.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_native_vfs_exists_returns_true_after_write() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        vfs.write("yes.txt", b"data").await.unwrap();
        assert!(vfs.exists("yes.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_native_vfs_remove_deletes_file() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        vfs.write("doomed.txt", b"bye").await.unwrap();
        assert!(vfs.exists("doomed.txt").await.unwrap());

        vfs.remove("doomed.txt").await.unwrap();
        assert!(!vfs.exists("doomed.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_native_vfs_remove_nonexistent_returns_error() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        let result = vfs.remove("ghost.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_native_vfs_rename_moves_file() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        vfs.write("old.txt", b"content").await.unwrap();
        vfs.rename("old.txt", "new.txt").await.unwrap();

        assert!(!vfs.exists("old.txt").await.unwrap());
        let content = vfs.read_to_string("new.txt").await.unwrap();
        assert_eq!(content, "content");
    }

    #[tokio::test]
    async fn test_native_vfs_create_dir_all() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        vfs.create_dir_all("x/y/z").await.unwrap();
        vfs.write("x/y/z/deep.txt", b"found").await.unwrap();
        let content = vfs.read_to_string("x/y/z/deep.txt").await.unwrap();
        assert_eq!(content, "found");
    }

    // ─── NativeVfs: atomic write semantics ───────────────────────────────

    #[tokio::test]
    async fn test_native_vfs_write_overwrites_existing() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        vfs.write("over.txt", b"first").await.unwrap();
        vfs.write("over.txt", b"second").await.unwrap();
        let content = vfs.read_to_string("over.txt").await.unwrap();
        assert_eq!(content, "second");
    }

    #[tokio::test]
    async fn test_native_vfs_write_empty_file() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        vfs.write("empty.txt", b"").await.unwrap();
        let content = vfs.read("empty.txt").await.unwrap();
        assert!(content.is_empty());
    }

    #[tokio::test]
    async fn test_native_vfs_write_no_leftover_tmp() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        vfs.write("clean.txt", b"data").await.unwrap();

        // No .tmp file should remain after atomic write
        assert!(!vfs.exists("clean.tmp").await.unwrap());
    }

    // ─── NativeVfs: read errors ──────────────────────────────────────────

    #[tokio::test]
    async fn test_native_vfs_read_nonexistent_returns_not_found() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        let result = vfs.read("missing.txt").await;
        assert!(matches!(result, Err(VfsError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_native_vfs_read_to_string_nonexistent_returns_not_found() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        let result = vfs.read_to_string("missing.txt").await;
        assert!(matches!(result, Err(VfsError::NotFound(_))));
    }

    // ─── NativeVfs: append edge cases ────────────────────────────────────

    #[tokio::test]
    async fn test_native_vfs_append_creates_file_if_missing() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        vfs.append("new.txt", b"first").await.unwrap();
        let content = vfs.read_to_string("new.txt").await.unwrap();
        assert_eq!(content, "first");
    }

    #[tokio::test]
    async fn test_native_vfs_append_multiple_sequential() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        for i in 0..10 {
            vfs.append("journal.log", format!("entry-{}\n", i).as_bytes())
                .await
                .unwrap();
        }

        let content = vfs.read_to_string("journal.log").await.unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0], "entry-0");
        assert_eq!(lines[9], "entry-9");
    }

    // ─── NativeVfs: binary data ──────────────────────────────────────────

    #[tokio::test]
    async fn test_native_vfs_read_write_binary() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        let binary: Vec<u8> = (0u8..=255).collect();
        vfs.write("binary.bin", &binary).await.unwrap();
        let read_back = vfs.read("binary.bin").await.unwrap();
        assert_eq!(read_back, binary);
    }

    // ─── NativeVfs: path traversal prevention ────────────────────────────

    #[tokio::test]
    async fn test_native_vfs_resolve_strips_path_traversal() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        // Path traversal attempts should be stripped
        vfs.write("../../../etc/passwd", b"nope").await.unwrap();
        // File should be created inside the base dir, not at /etc/passwd
        assert!(vfs.exists("etc/passwd").await.unwrap());
    }

    #[tokio::test]
    async fn test_native_vfs_resolve_strips_leading_slash() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        vfs.write("/absolute.txt", b"data").await.unwrap();
        assert!(vfs.exists("absolute.txt").await.unwrap());
    }

    // ─── NativeVfs: list_dir edge cases ──────────────────────────────────

    #[tokio::test]
    async fn test_native_vfs_list_dir_empty() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        let entries = vfs.list_dir("").await.unwrap();
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn test_native_vfs_list_dir_nonexistent_returns_error() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        let result = vfs.list_dir("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_native_vfs_list_dir_case_insensitive_sort() {
        let temp = TempDir::new().unwrap();
        let vfs = NativeVfs::new(temp.path());

        vfs.write("Zebra.txt", b"z").await.unwrap();
        vfs.write("apple.txt", b"a").await.unwrap();
        vfs.write("Banana.txt", b"b").await.unwrap();

        let entries = vfs.list_dir("").await.unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "apple.txt");
        assert_eq!(entries[1].name, "Banana.txt");
        assert_eq!(entries[2].name, "Zebra.txt");
    }

    // ─── get_platform_vfs() ──────────────────────────────────────────────

    #[tokio::test]
    async fn test_get_platform_vfs_returns_boxed_vfs() {
        // get_platform_vfs() returns Box<dyn Vfs + Send + Sync> on all platforms.
        // We can't control the root dir, but we can verify it returns a valid object.
        let vfs = get_platform_vfs();
        // Smoke test: exists should work even if path doesn't exist
        let result = vfs.exists("__nonexistent_test_file__").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_dyn_vfs_supports_all_operations() {
        let temp = TempDir::new().unwrap();
        let vfs: Box<dyn Vfs + Send + Sync> = Box::new(NativeVfs::new(temp.path()));

        // write + read
        vfs.write("test.txt", b"hello").await.unwrap();
        assert_eq!(vfs.read("test.txt").await.unwrap(), b"hello");

        // read_to_string
        assert_eq!(vfs.read_to_string("test.txt").await.unwrap(), "hello");

        // exists
        assert!(vfs.exists("test.txt").await.unwrap());
        assert!(!vfs.exists("nope.txt").await.unwrap());

        // append
        vfs.append("test.txt", b" world").await.unwrap();
        assert_eq!(vfs.read_to_string("test.txt").await.unwrap(), "hello world");

        // create_dir_all + nested write
        vfs.create_dir_all("a/b").await.unwrap();
        vfs.write("a/b/nested.txt", b"deep").await.unwrap();
        assert_eq!(vfs.read_to_string("a/b/nested.txt").await.unwrap(), "deep");

        // rename
        vfs.rename("test.txt", "moved.txt").await.unwrap();
        assert!(!vfs.exists("test.txt").await.unwrap());
        assert_eq!(
            vfs.read_to_string("moved.txt").await.unwrap(),
            "hello world"
        );

        // list_dir
        let entries = vfs.list_dir("").await.unwrap();
        assert!(entries.len() >= 2);

        // remove
        vfs.remove("moved.txt").await.unwrap();
        assert!(!vfs.exists("moved.txt").await.unwrap());
    }

    #[tokio::test]
    async fn test_dyn_vfs_can_be_wrapped_in_arc() {
        let temp = TempDir::new().unwrap();
        let vfs: std::sync::Arc<dyn Vfs + Send + Sync> =
            std::sync::Arc::from(Box::new(NativeVfs::new(temp.path())) as Box<dyn Vfs + Send + Sync>);

        // Verify Arc<dyn Vfs> works for Distributed/Persistent mount patterns
        let vfs_clone = vfs.clone();
        vfs.write("arc_test.txt", b"shared").await.unwrap();
        let content = vfs_clone.read_to_string("arc_test.txt").await.unwrap();
        assert_eq!(content, "shared");
    }

    // ─── UringVfs tests (Linux only, gated behind feature) ──────────────

    #[cfg(all(target_os = "linux", feature = "io-uring"))]
    mod uring_tests {
        use super::*;

        #[tokio::test]
        async fn test_uring_vfs_creation() {
            let temp = TempDir::new().unwrap();
            let result = super::super::UringVfs::new(temp.path());
            assert!(result.is_ok(), "UringVfs::new should succeed on supported Linux kernels");
        }

        #[tokio::test]
        async fn test_uring_vfs_read_write() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            vfs.write("test.txt", b"hello world").await.unwrap();
            let content = vfs.read_to_string("test.txt").await.unwrap();
            assert_eq!(content, "hello world");
        }

        #[tokio::test]
        async fn test_uring_vfs_read_bytes() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            let data = b"binary data \x00\x01\x02\xff";
            vfs.write("bin.dat", data).await.unwrap();
            let read_back = vfs.read("bin.dat").await.unwrap();
            assert_eq!(read_back, data);
        }

        #[tokio::test]
        async fn test_uring_vfs_append() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            vfs.append("log.txt", b"line1\n").await.unwrap();
            vfs.append("log.txt", b"line2\n").await.unwrap();

            let content = vfs.read_to_string("log.txt").await.unwrap();
            assert_eq!(content, "line1\nline2\n");
        }

        #[tokio::test]
        async fn test_uring_vfs_append_creates_file() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            vfs.append("new.log", b"first entry\n").await.unwrap();
            let content = vfs.read_to_string("new.log").await.unwrap();
            assert_eq!(content, "first entry\n");
        }

        #[tokio::test]
        async fn test_uring_vfs_append_many_sequential() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            for i in 0..100 {
                vfs.append("journal.log", format!("entry-{}\n", i).as_bytes())
                    .await
                    .unwrap();
            }

            let content = vfs.read_to_string("journal.log").await.unwrap();
            let lines: Vec<&str> = content.lines().collect();
            assert_eq!(lines.len(), 100);
            assert_eq!(lines[0], "entry-0");
            assert_eq!(lines[99], "entry-99");
        }

        #[tokio::test]
        async fn test_uring_vfs_nested_dirs() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            vfs.write("a/b/c/file.txt", b"deep").await.unwrap();
            let content = vfs.read_to_string("a/b/c/file.txt").await.unwrap();
            assert_eq!(content, "deep");
        }

        #[tokio::test]
        async fn test_uring_vfs_exists() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            assert!(!vfs.exists("missing.txt").await.unwrap());
            vfs.write("present.txt", b"here").await.unwrap();
            assert!(vfs.exists("present.txt").await.unwrap());
        }

        #[tokio::test]
        async fn test_uring_vfs_remove() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            vfs.write("doomed.txt", b"bye").await.unwrap();
            assert!(vfs.exists("doomed.txt").await.unwrap());

            vfs.remove("doomed.txt").await.unwrap();
            assert!(!vfs.exists("doomed.txt").await.unwrap());
        }

        #[tokio::test]
        async fn test_uring_vfs_remove_nonexistent_returns_error() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            let result = vfs.remove("ghost.txt").await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_uring_vfs_rename() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            vfs.write("old.txt", b"content").await.unwrap();
            vfs.rename("old.txt", "new.txt").await.unwrap();

            assert!(!vfs.exists("old.txt").await.unwrap());
            let content = vfs.read_to_string("new.txt").await.unwrap();
            assert_eq!(content, "content");
        }

        #[tokio::test]
        async fn test_uring_vfs_create_dir_all() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            vfs.create_dir_all("x/y/z").await.unwrap();
            vfs.write("x/y/z/deep.txt", b"found").await.unwrap();
            let content = vfs.read_to_string("x/y/z/deep.txt").await.unwrap();
            assert_eq!(content, "found");
        }

        #[tokio::test]
        async fn test_uring_vfs_list_dir() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            vfs.write("file1.txt", b"1").await.unwrap();
            vfs.write("file2.txt", b"2").await.unwrap();
            vfs.write("subdir/nested.txt", b"n").await.unwrap();

            let entries = vfs.list_dir("").await.unwrap();
            assert_eq!(entries.len(), 3);
            assert_eq!(entries[0].name, "subdir");
            assert!(entries[0].is_directory);
            assert_eq!(entries[1].name, "file1.txt");
            assert_eq!(entries[2].name, "file2.txt");
        }

        #[tokio::test]
        async fn test_uring_vfs_list_dir_empty() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            let entries = vfs.list_dir("").await.unwrap();
            assert!(entries.is_empty());
        }

        #[tokio::test]
        async fn test_uring_vfs_atomic_write_overwrites() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            vfs.write("over.txt", b"first").await.unwrap();
            vfs.write("over.txt", b"second").await.unwrap();
            let content = vfs.read_to_string("over.txt").await.unwrap();
            assert_eq!(content, "second");
        }

        #[tokio::test]
        async fn test_uring_vfs_atomic_write_no_leftover_tmp() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            vfs.write("clean.txt", b"data").await.unwrap();
            assert!(!vfs.exists("clean.tmp").await.unwrap());
        }

        #[tokio::test]
        async fn test_uring_vfs_write_empty_file() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            vfs.write("empty.txt", b"").await.unwrap();
            let content = vfs.read("empty.txt").await.unwrap();
            assert!(content.is_empty());
        }

        #[tokio::test]
        async fn test_uring_vfs_read_nonexistent_returns_error() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            let result = vfs.read("missing.txt").await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_uring_vfs_binary_roundtrip() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            let binary: Vec<u8> = (0u8..=255).collect();
            vfs.write("binary.bin", &binary).await.unwrap();
            let read_back = vfs.read("binary.bin").await.unwrap();
            assert_eq!(read_back, binary);
        }

        #[tokio::test]
        async fn test_uring_vfs_large_file_roundtrip() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            // 1 MiB file
            let large: Vec<u8> = (0..1_048_576).map(|i| (i % 256) as u8).collect();
            vfs.write("large.bin", &large).await.unwrap();
            let read_back = vfs.read("large.bin").await.unwrap();
            assert_eq!(read_back.len(), large.len());
            assert_eq!(read_back, large);
        }

        #[tokio::test]
        async fn test_uring_vfs_path_traversal_prevention() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            vfs.write("../../../etc/passwd", b"nope").await.unwrap();
            assert!(vfs.exists("etc/passwd").await.unwrap());
        }

        #[tokio::test]
        async fn test_uring_vfs_concurrent_reads() {
            let temp = TempDir::new().unwrap();
            let vfs = std::sync::Arc::new(super::super::UringVfs::new(temp.path()).unwrap());

            // Write several files
            for i in 0..10 {
                vfs.write(&format!("file_{}.txt", i), format!("content_{}", i).as_bytes())
                    .await
                    .unwrap();
            }

            // Read them all concurrently
            let mut handles = Vec::new();
            for i in 0..10 {
                let vfs = vfs.clone();
                handles.push(tokio::spawn(async move {
                    let content = vfs.read_to_string(&format!("file_{}.txt", i)).await.unwrap();
                    assert_eq!(content, format!("content_{}", i));
                }));
            }

            for handle in handles {
                handle.await.unwrap();
            }
        }

        #[tokio::test]
        async fn test_uring_vfs_concurrent_appends() {
            let temp = TempDir::new().unwrap();
            let vfs = std::sync::Arc::new(super::super::UringVfs::new(temp.path()).unwrap());

            // 50 concurrent appends
            let mut handles = Vec::new();
            for i in 0..50 {
                let vfs = vfs.clone();
                handles.push(tokio::spawn(async move {
                    vfs.append("concurrent.log", format!("entry-{}\n", i).as_bytes())
                        .await
                        .unwrap();
                }));
            }

            for handle in handles {
                handle.await.unwrap();
            }

            let content = vfs.read_to_string("concurrent.log").await.unwrap();
            let lines: Vec<&str> = content.lines().collect();
            assert_eq!(lines.len(), 50);
        }

        #[tokio::test]
        async fn test_uring_vfs_worker_shutdown_clean() {
            let temp = TempDir::new().unwrap();

            // Create and immediately drop — should shut down cleanly
            {
                let vfs = super::super::UringVfs::new(temp.path()).unwrap();
                vfs.write("shutdown_test.txt", b"data").await.unwrap();
            }
            // If we reach here without hanging, the worker shut down cleanly.

            // Verify the data persisted
            let vfs = NativeVfs::new(temp.path());
            let content = vfs.read_to_string("shutdown_test.txt").await.unwrap();
            assert_eq!(content, "data");
        }

        #[tokio::test]
        async fn test_uring_vfs_operations_after_many_writes() {
            let temp = TempDir::new().unwrap();
            let vfs = super::super::UringVfs::new(temp.path()).unwrap();

            // Stress test: many sequential writes then reads
            for i in 0..200 {
                let name = format!("file_{}.txt", i);
                let content = format!("content_{}", i);
                vfs.write(&name, content.as_bytes()).await.unwrap();
            }

            for i in 0..200 {
                let name = format!("file_{}.txt", i);
                let expected = format!("content_{}", i);
                let actual = vfs.read_to_string(&name).await.unwrap();
                assert_eq!(actual, expected, "file {} content mismatch", i);
            }
        }

        #[tokio::test]
        async fn test_uring_vfs_matches_native_vfs_behavior() {
            let temp1 = TempDir::new().unwrap();
            let temp2 = TempDir::new().unwrap();
            let uring = super::super::UringVfs::new(temp1.path()).unwrap();
            let native = NativeVfs::new(temp2.path());

            // Both should produce identical results for the same operations
            let ops = vec![
                ("write", "test.txt", b"hello" as &[u8]),
                ("write", "dir/nested.txt", b"nested"),
                ("append", "log.txt", b"line1\n"),
                ("append", "log.txt", b"line2\n"),
            ];

            for (op, path, data) in &ops {
                match *op {
                    "write" => {
                        uring.write(path, data).await.unwrap();
                        native.write(path, data).await.unwrap();
                    }
                    "append" => {
                        uring.append(path, data).await.unwrap();
                        native.append(path, data).await.unwrap();
                    }
                    _ => unreachable!(),
                }
            }

            // Compare results
            for path in &["test.txt", "dir/nested.txt", "log.txt"] {
                let uring_content = uring.read(path).await.unwrap();
                let native_content = native.read(path).await.unwrap();
                assert_eq!(uring_content, native_content, "mismatch for {}", path);
            }

            // Compare list_dir
            let uring_entries = uring.list_dir("").await.unwrap();
            let native_entries = native.list_dir("").await.unwrap();
            assert_eq!(uring_entries.len(), native_entries.len());
            for (u, n) in uring_entries.iter().zip(native_entries.iter()) {
                assert_eq!(u.name, n.name);
                assert_eq!(u.is_directory, n.is_directory);
            }
        }
    }

    // ─── UringVfs fallback test (Linux without io_uring support) ─────────

    #[cfg(all(target_os = "linux", feature = "io-uring"))]
    #[tokio::test]
    async fn test_uring_vfs_fallback_on_unsupported() {
        // get_platform_vfs should always return a working VFS,
        // regardless of whether io_uring is actually available.
        let temp = TempDir::new().unwrap();
        std::env::set_current_dir(temp.path()).unwrap();

        let vfs = get_platform_vfs();
        vfs.write("fallback.txt", b"works").await.unwrap();
        let content = vfs.read_to_string("fallback.txt").await.unwrap();
        assert_eq!(content, "works");
    }
}
