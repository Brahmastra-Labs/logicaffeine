//! io_uring VFS Implementation
//!
//! `UringVfs` implements the `Vfs` trait using Linux's io_uring for true
//! kernel-async file I/O. All hot-path operations (read, write, append) go
//! through io_uring SQEs on a dedicated worker thread, avoiding tokio's
//! blocking thread pool entirely.
//!
//! # Architecture
//!
//! ```text
//! Tokio task ──► UringVfs::read()
//!                  │
//!                  ├─ send UringCommand via crossbeam ───► Worker thread
//!                  ├─ await tokio::sync::oneshot            ├─ Submit SQE
//!                  │                                        ├─ submit_and_wait()
//!                  │                                        ├─ Process CQE
//!                  └─ receive VfsResult ◄────────────────── └─ Send via oneshot
//! ```

use std::path::PathBuf;

use async_trait::async_trait;
use crossbeam_channel::Sender;
use io_uring::IoUring;
use tokio::sync::oneshot;

use super::uring_worker::UringCommand;
use super::{DirEntry, Vfs, VfsError, VfsResult};

/// Ring size — number of SQE slots. 256 is ample for file I/O workloads.
const RING_SIZE: u32 = 256;

pub struct UringVfs {
    base_dir: PathBuf,
    sender: Sender<UringCommand>,
    worker_handle: Option<std::thread::JoinHandle<()>>,
}

impl UringVfs {
    /// Create a new `UringVfs` rooted at the given directory.
    ///
    /// Spawns a dedicated OS thread running the io_uring worker loop.
    /// Returns `Err` if the kernel doesn't support io_uring (old kernel,
    /// container restrictions, etc.) — caller should fall back to `NativeVfs`.
    pub fn new<P: Into<PathBuf>>(base_dir: P) -> Result<Self, VfsError> {
        let base_dir = base_dir.into();

        // Probe io_uring support by creating a ring.
        let ring = IoUring::new(RING_SIZE).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("io_uring not available: {}", e),
            ))
        })?;

        let (tx, rx) = crossbeam_channel::unbounded();

        let worker_handle = std::thread::Builder::new()
            .name("uring-vfs-worker".into())
            .spawn(move || {
                super::uring_worker::run_worker(ring, rx);
            })
            .map_err(|e| VfsError::IoError(e))?;

        Ok(Self {
            base_dir,
            sender: tx,
            worker_handle: Some(worker_handle),
        })
    }

    fn resolve(&self, path: &str) -> PathBuf {
        let clean = path.trim_start_matches('/').trim_start_matches("../");
        self.base_dir.join(clean)
    }

    fn send(&self, cmd: UringCommand) -> VfsResult<()> {
        self.sender.send(cmd).map_err(|_| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "io_uring worker thread has shut down",
            ))
        })
    }
}

impl Drop for UringVfs {
    fn drop(&mut self) {
        let _ = self.sender.send(UringCommand::Shutdown);
        if let Some(handle) = self.worker_handle.take() {
            let _ = handle.join();
        }
    }
}

#[async_trait]
impl Vfs for UringVfs {
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>> {
        let full_path = self.resolve(path);
        let (tx, rx) = oneshot::channel();
        self.send(UringCommand::Read {
            path: full_path,
            tx,
        })?;
        rx.await.map_err(|_| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "io_uring worker dropped the response",
            ))
        })?
    }

    async fn read_to_string(&self, path: &str) -> VfsResult<String> {
        let full_path = self.resolve(path);
        let (tx, rx) = oneshot::channel();
        self.send(UringCommand::ReadToString {
            path: full_path,
            tx,
        })?;
        rx.await.map_err(|_| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "io_uring worker dropped the response",
            ))
        })?
    }

    async fn write(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        let full_path = self.resolve(path);
        let (tx, rx) = oneshot::channel();
        self.send(UringCommand::Write {
            path: full_path,
            data: contents.to_vec(),
            tx,
        })?;
        rx.await.map_err(|_| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "io_uring worker dropped the response",
            ))
        })?
    }

    async fn append(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        let full_path = self.resolve(path);
        let (tx, rx) = oneshot::channel();
        self.send(UringCommand::Append {
            path: full_path,
            data: contents.to_vec(),
            tx,
        })?;
        rx.await.map_err(|_| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "io_uring worker dropped the response",
            ))
        })?
    }

    async fn exists(&self, path: &str) -> VfsResult<bool> {
        let full_path = self.resolve(path);
        let (tx, rx) = oneshot::channel();
        self.send(UringCommand::Exists {
            path: full_path,
            tx,
        })?;
        rx.await.map_err(|_| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "io_uring worker dropped the response",
            ))
        })?
    }

    async fn remove(&self, path: &str) -> VfsResult<()> {
        let full_path = self.resolve(path);
        let (tx, rx) = oneshot::channel();
        self.send(UringCommand::Remove {
            path: full_path,
            tx,
        })?;
        rx.await.map_err(|_| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "io_uring worker dropped the response",
            ))
        })?
    }

    async fn create_dir_all(&self, path: &str) -> VfsResult<()> {
        let full_path = self.resolve(path);
        let (tx, rx) = oneshot::channel();
        self.send(UringCommand::CreateDirAll {
            path: full_path,
            tx,
        })?;
        rx.await.map_err(|_| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "io_uring worker dropped the response",
            ))
        })?
    }

    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        let from_path = self.resolve(from);
        let to_path = self.resolve(to);
        let (tx, rx) = oneshot::channel();
        self.send(UringCommand::Rename {
            from: from_path,
            to: to_path,
            tx,
        })?;
        rx.await.map_err(|_| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "io_uring worker dropped the response",
            ))
        })?
    }

    async fn list_dir(&self, path: &str) -> VfsResult<Vec<DirEntry>> {
        let full_path = self.resolve(path);
        let (tx, rx) = oneshot::channel();
        self.send(UringCommand::ListDir {
            path: full_path,
            tx,
        })?;
        rx.await.map_err(|_| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "io_uring worker dropped the response",
            ))
        })?
    }
}
