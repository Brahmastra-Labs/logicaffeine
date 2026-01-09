//! Phase 54: OPFS (Origin Private File System) implementation for WASM.
//!
//! Provides browser persistence using the File System Access API.
//! All paths are relative to the OPFS root (no traversal possible).

#![cfg(target_arch = "wasm32")]

use super::{DirEntry, Vfs, VfsError, VfsResult};
use async_trait::async_trait;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{FileSystemDirectoryHandle, FileSystemFileHandle, FileSystemWritableFileStream};

#[wasm_bindgen]
extern "C" {
    /// Async iterator result from FileSystemDirectoryHandle.entries()
    #[wasm_bindgen(js_name = Object)]
    type AsyncIteratorResult;

    #[wasm_bindgen(method, getter)]
    fn done(this: &AsyncIteratorResult) -> bool;

    #[wasm_bindgen(method, getter)]
    fn value(this: &AsyncIteratorResult) -> JsValue;
}

/// VFS backed by the browser's Origin Private File System.
///
/// OPFS provides a private, sandboxed file system per origin that persists
/// across page reloads. This gives LOGOS the same persistence semantics
/// in the browser as native apps have on disk.
#[derive(Clone)]
pub struct OpfsVfs {
    root: FileSystemDirectoryHandle,
}

impl OpfsVfs {
    /// Create a new OPFS VFS rooted at the origin's private filesystem.
    ///
    /// This requires a secure context (HTTPS or localhost).
    pub async fn new() -> VfsResult<Self> {
        let window = web_sys::window()
            .ok_or_else(|| VfsError::PermissionDenied("No window object".into()))?;
        let navigator = window.navigator();
        let storage = navigator.storage();

        let promise = storage.get_directory();
        let root = JsFuture::from(promise)
            .await
            .map_err(|e| VfsError::PermissionDenied(format!("OPFS access denied: {:?}", e)))?
            .unchecked_into::<FileSystemDirectoryHandle>();

        Ok(Self { root })
    }

    /// Navigate to a directory, optionally creating intermediate directories.
    async fn get_dir(&self, path: &str, create: bool) -> VfsResult<FileSystemDirectoryHandle> {
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            return Ok(self.root.clone());
        }

        let mut current = self.root.clone();
        for segment in path.split('/') {
            if segment.is_empty() || segment == "." {
                continue;
            }
            if segment == ".." {
                // OPFS doesn't allow traversal above root - just skip
                continue;
            }

            let opts = web_sys::FileSystemGetDirectoryOptions::new();
            opts.set_create(create);

            let promise = current.get_directory_handle_with_options(segment, &opts);
            current = JsFuture::from(promise)
                .await
                .map_err(|e| {
                    if !create {
                        VfsError::NotFound(format!("Directory not found: {}", path))
                    } else {
                        VfsError::IoError(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Failed to get directory: {:?}", e),
                        ))
                    }
                })?
                .unchecked_into::<FileSystemDirectoryHandle>();
        }

        Ok(current)
    }

    /// Get file handle at path.
    async fn get_file(&self, path: &str, create: bool) -> VfsResult<FileSystemFileHandle> {
        let path = path.trim_start_matches('/');

        // Split path into parent directory and filename
        let (parent_path, filename) = match path.rfind('/') {
            Some(idx) => (&path[..idx], &path[idx + 1..]),
            None => ("", path),
        };

        // Get parent directory
        let parent = self.get_dir(parent_path, create).await?;

        // Get file handle
        let opts = web_sys::FileSystemGetFileOptions::new();
        opts.set_create(create);

        let promise = parent.get_file_handle_with_options(filename, &opts);
        JsFuture::from(promise)
            .await
            .map(|v| v.unchecked_into::<FileSystemFileHandle>())
            .map_err(|_| VfsError::NotFound(path.into()))
    }

    /// Extract parent path from a file path.
    fn parent_path(path: &str) -> Option<&str> {
        let path = path.trim_start_matches('/');
        path.rfind('/').map(|idx| &path[..idx])
    }
}

#[async_trait(?Send)]
impl Vfs for OpfsVfs {
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>> {
        let file_handle = self.get_file(path, false).await?;

        let promise = file_handle.get_file();
        let file: web_sys::File = JsFuture::from(promise)
            .await
            .map_err(|_| VfsError::NotFound(path.into()))?
            .unchecked_into();

        let promise = file.array_buffer();
        let array_buffer = JsFuture::from(promise)
            .await
            .map_err(|e| {
                VfsError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Read failed: {:?}", e),
                ))
            })?;

        let uint8_array = js_sys::Uint8Array::new(&array_buffer);
        Ok(uint8_array.to_vec())
    }

    async fn read_to_string(&self, path: &str) -> VfsResult<String> {
        let bytes = self.read(path).await?;
        String::from_utf8(bytes).map_err(|e| VfsError::SerializationError(e.to_string()))
    }

    async fn write(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        // Ensure parent directory exists
        if let Some(parent) = Self::parent_path(path) {
            self.create_dir_all(parent).await?;
        }

        let file_handle = self.get_file(path, true).await?;

        // Create writable stream (truncates by default)
        let promise = file_handle.create_writable();
        let writable: FileSystemWritableFileStream = JsFuture::from(promise)
            .await
            .map_err(|e| VfsError::PermissionDenied(format!("Create writable failed: {:?}", e)))?
            .unchecked_into();

        // Write content
        let data = js_sys::Uint8Array::from(contents);
        let promise = writable.write_with_buffer_source(&data)
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Write setup failed: {:?}", e),
            )))?;
        JsFuture::from(promise).await.map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Write failed: {:?}", e),
            ))
        })?;

        // Close stream
        let promise = writable.close();
        JsFuture::from(promise).await.map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Close failed: {:?}", e),
            ))
        })?;

        Ok(())
    }

    async fn append(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        // OPFS doesn't have native append - read existing, concat, write
        let existing = match self.read(path).await {
            Ok(data) => data,
            Err(VfsError::NotFound(_)) => Vec::new(),
            Err(e) => return Err(e),
        };

        let mut combined = existing;
        combined.extend_from_slice(contents);
        self.write(path, &combined).await
    }

    async fn exists(&self, path: &str) -> VfsResult<bool> {
        // First try as a file
        match self.get_file(path, false).await {
            Ok(_) => return Ok(true),
            Err(VfsError::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
        // Then try as a directory
        match self.get_dir(path, false).await {
            Ok(_) => Ok(true),
            Err(VfsError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn remove(&self, path: &str) -> VfsResult<()> {
        let path = path.trim_start_matches('/');

        // Split into parent and filename
        let (parent_path, filename) = match path.rfind('/') {
            Some(idx) => (&path[..idx], &path[idx + 1..]),
            None => ("", path),
        };

        let parent = self.get_dir(parent_path, false).await?;

        let promise = parent.remove_entry(filename);
        JsFuture::from(promise)
            .await
            .map_err(|_| VfsError::NotFound(path.into()))?;

        Ok(())
    }

    async fn create_dir_all(&self, path: &str) -> VfsResult<()> {
        self.get_dir(path, true).await?;
        Ok(())
    }

    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        // OPFS doesn't have native rename - read, write, delete
        let content = self.read(from).await?;
        self.write(to, &content).await?;
        self.remove(from).await?;
        Ok(())
    }

    async fn list_dir(&self, path: &str) -> VfsResult<Vec<DirEntry>> {
        let dir = self.get_dir(path, false).await?;

        // Get async iterator via entries()
        let iterator: js_sys::AsyncIterator = js_sys::Reflect::get(&dir, &JsValue::from_str("entries"))
            .ok()
            .and_then(|f| f.dyn_ref::<js_sys::Function>().cloned())
            .ok_or_else(|| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "entries() not available",
            )))?
            .call0(&dir)
            .map_err(|_| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to call entries()",
            )))?
            .unchecked_into();

        let mut entries = Vec::new();

        loop {
            let next_promise = iterator.next().map_err(|_| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Iterator next() failed",
            )))?;

            let result: AsyncIteratorResult = JsFuture::from(next_promise)
                .await
                .map_err(|_| VfsError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Async iteration failed",
                )))?
                .unchecked_into();

            if result.done() {
                break;
            }

            // value is [name, handle] array
            let pair = result.value();
            let array: js_sys::Array = pair.unchecked_into();
            let name: String = array.get(0).as_string().unwrap_or_default();
            let handle = array.get(1);

            // Check if it's a directory by looking at the 'kind' property
            let kind = js_sys::Reflect::get(&handle, &JsValue::from_str("kind"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();

            entries.push(DirEntry {
                name,
                is_directory: kind == "directory",
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
