//! IndexedDB-based VFS Fallback for WASM
//!
//! Provides browser persistence using IndexedDB when OPFS is unavailable
//! (e.g., Private Browsing mode). Data persists for the session but may
//! be cleared when Private Browsing ends.
//!
//! # Architecture
//!
//! - Uses a single IndexedDB database "logicaffeine-vfs"
//! - Two object stores:
//!   - "files": stores file content as `{ path, data }` with path as key
//!   - "dirs": stores directory existence as `{ path }` with path as key
//!
//! # Limitations
//!
//! - Slower than OPFS for large files
//! - In Private Browsing, data is cleared when session ends
//! - No true atomic operations

#![cfg(target_arch = "wasm32")]

use super::{DirEntry, Vfs, VfsError, VfsResult};
use async_trait::async_trait;
use js_sys::{Array, Object, Uint8Array};
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{IdbDatabase, IdbRequest, IdbTransaction, IdbTransactionMode};

const DB_NAME: &str = "logicaffeine-vfs";
const DB_VERSION: u32 = 1;
const FILES_STORE: &str = "files";
const DIRS_STORE: &str = "dirs";

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

/// VFS backed by IndexedDB for fallback storage.
///
/// Used when OPFS is unavailable (e.g., Private Browsing mode).
#[derive(Clone)]
pub struct IndexedDbVfs {
    db: Rc<RefCell<Option<IdbDatabase>>>,
}

impl IndexedDbVfs {
    /// Create a new IndexedDB VFS.
    ///
    /// Opens or creates the database with required object stores.
    pub async fn new() -> VfsResult<Self> {
        log("[IndexedDbVfs] Initializing...");

        let window = web_sys::window()
            .ok_or_else(|| VfsError::PermissionDenied("No window object".into()))?;

        let indexed_db = window
            .indexed_db()
            .map_err(|e| VfsError::PermissionDenied(format!("IndexedDB access failed: {:?}", e)))?
            .ok_or_else(|| VfsError::PermissionDenied("IndexedDB not available".into()))?;

        // Open database with version upgrade handler
        let open_request = indexed_db
            .open_with_u32(DB_NAME, DB_VERSION)
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to open IndexedDB: {:?}", e),
            )))?;

        // Handle upgrade needed (create object stores)
        let upgrade_closure = Closure::once(Box::new(move |event: web_sys::IdbVersionChangeEvent| {
            log("[IndexedDbVfs] Upgrade needed, creating object stores...");
            let target = event.target().unwrap();
            let request: IdbRequest = target.unchecked_into();
            let db: IdbDatabase = request.result().unwrap().unchecked_into();

            // Create files store if it doesn't exist
            let store_names = db.object_store_names();
            if !store_names.contains(FILES_STORE) {
                let params = web_sys::IdbObjectStoreParameters::new();
                params.set_key_path(&JsValue::from_str("path"));
                db.create_object_store_with_optional_parameters(FILES_STORE, &params)
                    .expect("Failed to create files store");
                log("[IndexedDbVfs] Created 'files' object store");
            }

            // Create dirs store if it doesn't exist
            if !store_names.contains(DIRS_STORE) {
                let params = web_sys::IdbObjectStoreParameters::new();
                params.set_key_path(&JsValue::from_str("path"));
                db.create_object_store_with_optional_parameters(DIRS_STORE, &params)
                    .expect("Failed to create dirs store");
                log("[IndexedDbVfs] Created 'dirs' object store");
            }
        }) as Box<dyn FnOnce(web_sys::IdbVersionChangeEvent)>);

        open_request.set_onupgradeneeded(Some(upgrade_closure.as_ref().unchecked_ref()));
        upgrade_closure.forget();

        // Wait for database to open using JS Promise
        let db = Self::wait_for_request(&open_request).await?;
        let db: IdbDatabase = db.unchecked_into();

        log("[IndexedDbVfs] Database opened successfully");

        Ok(Self {
            db: Rc::new(RefCell::new(Some(db))),
        })
    }

    /// Wait for an IDB request to complete using JS Promise.
    async fn wait_for_request(request: &IdbRequest) -> VfsResult<JsValue> {
        // Use JS Promise pattern (same as WorkerOpfsVfs)
        let request_clone = request.clone();
        let promise = js_sys::Promise::new(&mut |resolve, reject| {
            let resolve_clone = resolve.clone();
            let success_closure = Closure::once(Box::new(move |_event: web_sys::Event| {
                let _ = resolve_clone.call0(&JsValue::NULL);
            }) as Box<dyn FnOnce(web_sys::Event)>);

            let error_closure = Closure::once(Box::new(move |_event: web_sys::Event| {
                let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("IndexedDB request failed"));
            }) as Box<dyn FnOnce(web_sys::Event)>);

            request_clone.set_onsuccess(Some(success_closure.as_ref().unchecked_ref()));
            request_clone.set_onerror(Some(error_closure.as_ref().unchecked_ref()));

            success_closure.forget();
            error_closure.forget();
        });

        JsFuture::from(promise).await.map_err(|_| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "IndexedDB request failed",
            ))
        })?;

        request.result().map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get result: {:?}", e),
            ))
        })
    }

    /// Wait for an IDB transaction to complete using JS Promise.
    async fn wait_for_transaction(transaction: &IdbTransaction) -> VfsResult<()> {
        let transaction_clone = transaction.clone();
        let promise = js_sys::Promise::new(&mut |resolve, reject| {
            let resolve_clone = resolve.clone();
            let complete_closure = Closure::once(Box::new(move |_event: web_sys::Event| {
                let _ = resolve_clone.call0(&JsValue::NULL);
            }) as Box<dyn FnOnce(web_sys::Event)>);

            let error_closure = Closure::once(Box::new(move |_event: web_sys::Event| {
                let _ = reject.call1(&JsValue::NULL, &JsValue::from_str("Transaction failed"));
            }) as Box<dyn FnOnce(web_sys::Event)>);

            transaction_clone.set_oncomplete(Some(complete_closure.as_ref().unchecked_ref()));
            transaction_clone.set_onerror(Some(error_closure.as_ref().unchecked_ref()));

            complete_closure.forget();
            error_closure.forget();
        });

        JsFuture::from(promise).await.map_err(|_| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Transaction failed",
            ))
        })?;

        Ok(())
    }

    /// Get the database reference.
    fn get_db(&self) -> VfsResult<IdbDatabase> {
        self.db
            .borrow()
            .clone()
            .ok_or_else(|| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Database not initialized",
            )))
    }

    /// Normalize a path (remove leading/trailing slashes, handle empty).
    fn normalize_path(path: &str) -> String {
        let normalized = path.trim_start_matches('/').trim_end_matches('/');
        if normalized.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", normalized)
        }
    }

    /// Get parent path and filename from a path.
    fn split_path(path: &str) -> (String, String) {
        let normalized = Self::normalize_path(path);
        match normalized.rfind('/') {
            Some(0) => ("/".to_string(), normalized[1..].to_string()),
            Some(idx) => (normalized[..idx].to_string(), normalized[idx + 1..].to_string()),
            None => ("/".to_string(), normalized),
        }
    }
}

#[async_trait(?Send)]
impl Vfs for IndexedDbVfs {
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>> {
        let db = self.get_db()?;
        let path = Self::normalize_path(path);

        let transaction = db
            .transaction_with_str_and_mode(FILES_STORE, IdbTransactionMode::Readonly)
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create transaction: {:?}", e),
            )))?;

        let store = transaction.object_store(FILES_STORE).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get object store: {:?}", e),
            ))
        })?;

        let request = store.get(&JsValue::from_str(&path)).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get file: {:?}", e),
            ))
        })?;

        let result = Self::wait_for_request(&request).await?;

        if result.is_undefined() || result.is_null() {
            return Err(VfsError::NotFound(path));
        }

        // Extract data from { path, data } object
        let data = js_sys::Reflect::get(&result, &JsValue::from_str("data"))
            .map_err(|_| VfsError::NotFound(path.clone()))?;

        let uint8_array = Uint8Array::new(&data);
        Ok(uint8_array.to_vec())
    }

    async fn read_to_string(&self, path: &str) -> VfsResult<String> {
        let bytes = self.read(path).await?;
        String::from_utf8(bytes).map_err(|e| VfsError::SerializationError(e.to_string()))
    }

    async fn write(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        let db = self.get_db()?;
        let path = Self::normalize_path(path);

        // Ensure parent directory exists
        let (parent, _) = Self::split_path(&path);
        if parent != "/" {
            self.create_dir_all(&parent).await?;
        }

        let stores = Array::new();
        stores.push(&JsValue::from_str(FILES_STORE));

        let transaction = db
            .transaction_with_str_sequence_and_mode(&stores, IdbTransactionMode::Readwrite)
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create transaction: {:?}", e),
            )))?;

        let store = transaction.object_store(FILES_STORE).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get object store: {:?}", e),
            ))
        })?;

        // Create { path, data } object
        let obj = Object::new();
        js_sys::Reflect::set(&obj, &JsValue::from_str("path"), &JsValue::from_str(&path))
            .map_err(|_| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to set path",
            )))?;

        let data = Uint8Array::new_with_length(contents.len() as u32);
        data.copy_from(contents);
        js_sys::Reflect::set(&obj, &JsValue::from_str("data"), &data)
            .map_err(|_| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to set data",
            )))?;

        let request = store.put(&obj).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to put file: {:?}", e),
            ))
        })?;

        Self::wait_for_request(&request).await?;
        Self::wait_for_transaction(&transaction).await?;

        Ok(())
    }

    async fn append(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        // Read existing content, append, write back
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
        let db = self.get_db()?;
        let path = Self::normalize_path(path);

        // Check files store first
        let transaction = db
            .transaction_with_str_and_mode(FILES_STORE, IdbTransactionMode::Readonly)
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create transaction: {:?}", e),
            )))?;

        let store = transaction.object_store(FILES_STORE).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get object store: {:?}", e),
            ))
        })?;

        let request = store.get(&JsValue::from_str(&path)).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to check file: {:?}", e),
            ))
        })?;

        let result = Self::wait_for_request(&request).await?;
        if !result.is_undefined() && !result.is_null() {
            return Ok(true);
        }

        // Check dirs store
        let transaction = db
            .transaction_with_str_and_mode(DIRS_STORE, IdbTransactionMode::Readonly)
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create transaction: {:?}", e),
            )))?;

        let store = transaction.object_store(DIRS_STORE).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get object store: {:?}", e),
            ))
        })?;

        let request = store.get(&JsValue::from_str(&path)).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to check directory: {:?}", e),
            ))
        })?;

        let result = Self::wait_for_request(&request).await?;
        Ok(!result.is_undefined() && !result.is_null())
    }

    async fn remove(&self, path: &str) -> VfsResult<()> {
        let db = self.get_db()?;
        let path = Self::normalize_path(path);

        // Try to remove from files store
        let transaction = db
            .transaction_with_str_and_mode(FILES_STORE, IdbTransactionMode::Readwrite)
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create transaction: {:?}", e),
            )))?;

        let store = transaction.object_store(FILES_STORE).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get object store: {:?}", e),
            ))
        })?;

        let request = store.delete(&JsValue::from_str(&path)).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to delete file: {:?}", e),
            ))
        })?;

        Self::wait_for_request(&request).await?;
        Self::wait_for_transaction(&transaction).await?;

        Ok(())
    }

    async fn create_dir_all(&self, path: &str) -> VfsResult<()> {
        let db = self.get_db()?;
        let path = Self::normalize_path(path);

        if path == "/" {
            return Ok(());
        }

        // Create all parent directories
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let mut current = String::new();

        let transaction = db
            .transaction_with_str_and_mode(DIRS_STORE, IdbTransactionMode::Readwrite)
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create transaction: {:?}", e),
            )))?;

        let store = transaction.object_store(DIRS_STORE).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get object store: {:?}", e),
            ))
        })?;

        for part in parts {
            if part.is_empty() {
                continue;
            }
            current = format!("{}/{}", current, part);

            let obj = Object::new();
            js_sys::Reflect::set(&obj, &JsValue::from_str("path"), &JsValue::from_str(&current))
                .map_err(|_| VfsError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Failed to set path",
                )))?;

            let request = store.put(&obj).map_err(|e| {
                VfsError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to create directory: {:?}", e),
                ))
            })?;

            Self::wait_for_request(&request).await?;
        }

        Self::wait_for_transaction(&transaction).await?;

        Ok(())
    }

    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        // Read, write, delete
        let content = self.read(from).await?;
        self.write(to, &content).await?;
        self.remove(from).await?;
        Ok(())
    }

    async fn list_dir(&self, path: &str) -> VfsResult<Vec<DirEntry>> {
        let db = self.get_db()?;
        let path = Self::normalize_path(path);
        let prefix = if path == "/" {
            "/".to_string()
        } else {
            format!("{}/", path)
        };

        let mut entries = Vec::new();
        let mut seen_dirs = std::collections::HashSet::new();

        // Get all files with this prefix
        let transaction = db
            .transaction_with_str_and_mode(FILES_STORE, IdbTransactionMode::Readonly)
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create transaction: {:?}", e),
            )))?;

        let store = transaction.object_store(FILES_STORE).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get object store: {:?}", e),
            ))
        })?;

        let request = store.get_all().map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get all files: {:?}", e),
            ))
        })?;

        let result = Self::wait_for_request(&request).await?;
        let files: Array = result.unchecked_into();

        for i in 0..files.length() {
            let item = files.get(i);
            let file_path = js_sys::Reflect::get(&item, &JsValue::from_str("path"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();

            // Check if this file is in our directory
            if file_path.starts_with(&prefix) {
                let relative = &file_path[prefix.len()..];
                if let Some(slash) = relative.find('/') {
                    // It's in a subdirectory
                    let dir_name = &relative[..slash];
                    if !seen_dirs.contains(dir_name) {
                        seen_dirs.insert(dir_name.to_string());
                        entries.push(DirEntry {
                            name: dir_name.to_string(),
                            is_directory: true,
                        });
                    }
                } else if !relative.is_empty() {
                    // It's a direct child file
                    entries.push(DirEntry {
                        name: relative.to_string(),
                        is_directory: false,
                    });
                }
            }
        }

        // Get all directories with this prefix
        let transaction = db
            .transaction_with_str_and_mode(DIRS_STORE, IdbTransactionMode::Readonly)
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create transaction: {:?}", e),
            )))?;

        let store = transaction.object_store(DIRS_STORE).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get object store: {:?}", e),
            ))
        })?;

        let request = store.get_all().map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get all dirs: {:?}", e),
            ))
        })?;

        let result = Self::wait_for_request(&request).await?;
        let dirs: Array = result.unchecked_into();

        for i in 0..dirs.length() {
            let item = dirs.get(i);
            let dir_path = js_sys::Reflect::get(&item, &JsValue::from_str("path"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();

            // Check if this directory is a direct child
            if dir_path.starts_with(&prefix) {
                let relative = &dir_path[prefix.len()..];
                if !relative.contains('/') && !relative.is_empty() && !seen_dirs.contains(relative) {
                    seen_dirs.insert(relative.to_string());
                    entries.push(DirEntry {
                        name: relative.to_string(),
                        is_directory: true,
                    });
                }
            }
        }

        // Sort: directories first, then alphabetically
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
