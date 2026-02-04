//! Safari-Compatible OPFS via Web Worker
//!
//! This module provides OPFS access that works on Safari by using a dedicated
//! Web Worker with `createSyncAccessHandle()`. Safari does not support
//! `FileSystemFileHandle.createWritable()` on the main thread, so we must
//! offload all write operations to a worker.
//!
//! # Architecture
//!
//! ```text
//! Main Thread (WASM/Rust)        Worker Thread (JavaScript)
//! ┌─────────────────────┐        ┌─────────────────────────┐
//! │   WorkerOpfsVfs     │        │   opfs-worker.js        │
//! │   - Vfs trait impl  │◄──────►│   - createSyncAccessHandle │
//! │   - postMessage     │        │   - file handle cache   │
//! └─────────────────────┘        └─────────────────────────┘
//! ```
//!
//! # Browser Support
//!
//! - Safari 15.2+ (desktop and iOS)
//! - Chrome 86+
//! - Firefox 111+
//! - Edge 86+

#![cfg(target_arch = "wasm32")]

use super::{DirEntry, Vfs, VfsError, VfsResult};
use async_trait::async_trait;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

/// Request ID counter for multiplexing worker responses.
thread_local! {
    static REQUEST_ID: RefCell<u32> = const { RefCell::new(0) };
}

fn next_request_id() -> u32 {
    REQUEST_ID.with(|id| {
        let mut id = id.borrow_mut();
        *id = id.wrapping_add(1);
        *id
    })
}

/// Pending request tracking for response routing.
type PendingRequests = Rc<RefCell<HashMap<u32, js_sys::Function>>>;

/// VFS backed by a Web Worker using OPFS with createSyncAccessHandle().
///
/// This implementation works on all modern browsers including Safari, which
/// doesn't support `createWritable()` on the main thread.
#[derive(Clone)]
pub struct WorkerOpfsVfs {
    worker: web_sys::Worker,
    pending: PendingRequests,
}

impl WorkerOpfsVfs {
    /// Create a new WorkerOpfsVfs by spawning the OPFS worker.
    ///
    /// The worker script must be available at `/assets/opfs-worker.js`.
    pub fn new() -> VfsResult<Self> {
        let opts = web_sys::WorkerOptions::new();
        opts.set_type(web_sys::WorkerType::Classic);

        let worker = web_sys::Worker::new_with_options("/assets/opfs-worker.js", &opts)
            .map_err(|e| {
                VfsError::PermissionDenied(format!(
                    "Failed to create OPFS worker: {:?}",
                    e
                ))
            })?;

        let pending: PendingRequests = Rc::new(RefCell::new(HashMap::new()));

        // Set up message handler
        let pending_clone = pending.clone();
        let onmessage = Closure::wrap(Box::new(move |event: web_sys::MessageEvent| {
            let data = event.data();

            // Check for ready signal
            if let Ok(ready) = js_sys::Reflect::get(&data, &JsValue::from_str("ready")) {
                if ready.is_truthy() {
                    log("[WorkerOpfsVfs] Worker ready");
                    return;
                }
            }

            // Extract request ID
            let id = match js_sys::Reflect::get(&data, &JsValue::from_str("id")) {
                Ok(v) => v.as_f64().map(|n| n as u32),
                Err(_) => None,
            };

            if let Some(id) = id {
                let mut pending = pending_clone.borrow_mut();
                if let Some(resolve) = pending.remove(&id) {
                    let _ = resolve.call1(&JsValue::NULL, &data);
                }
            }
        }) as Box<dyn Fn(web_sys::MessageEvent)>);

        worker.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget(); // Leak closure to keep handler alive

        // Set up error handler
        let onerror = Closure::wrap(Box::new(move |event: web_sys::ErrorEvent| {
            log(&format!(
                "[WorkerOpfsVfs] Worker error: {} at {}:{}",
                event.message(),
                event.filename(),
                event.lineno()
            ));
        }) as Box<dyn Fn(web_sys::ErrorEvent)>);

        worker.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        onerror.forget();

        log("[WorkerOpfsVfs] Initialized");

        Ok(Self { worker, pending })
    }

    /// Send a request to the worker and wait for the response.
    async fn send_request(&self, op: &str, args: &JsValue) -> VfsResult<JsValue> {
        let id = next_request_id();

        // Create request object
        let request = js_sys::Object::new();
        js_sys::Reflect::set(&request, &JsValue::from_str("id"), &JsValue::from(id))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set request id: {:?}", e),
            )))?;
        js_sys::Reflect::set(&request, &JsValue::from_str("op"), &JsValue::from_str(op))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set request op: {:?}", e),
            )))?;
        js_sys::Reflect::set(&request, &JsValue::from_str("args"), args)
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set request args: {:?}", e),
            )))?;

        // Create promise for response
        let pending = self.pending.clone();
        let promise = js_sys::Promise::new(&mut |resolve, _reject| {
            pending.borrow_mut().insert(id, resolve);
        });

        // Post message to worker
        self.worker.post_message(&request).map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to post message to worker: {:?}", e),
            ))
        })?;

        // Wait for response
        let result = JsFuture::from(promise).await.map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Worker request failed: {:?}", e),
            ))
        })?;

        // Check for error
        let ok = js_sys::Reflect::get(&result, &JsValue::from_str("ok"))
            .map(|v| v.as_bool().unwrap_or(false))
            .unwrap_or(false);

        if !ok {
            let error = js_sys::Reflect::get(&result, &JsValue::from_str("error"))
                .unwrap_or(JsValue::NULL);
            let name = js_sys::Reflect::get(&error, &JsValue::from_str("name"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_else(|| "Error".into());
            let message = js_sys::Reflect::get(&error, &JsValue::from_str("message"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_else(|| "Unknown error".into());

            return Err(Self::map_js_error(&name, &message));
        }

        Ok(result)
    }

    /// Send a request with a transferable buffer for zero-copy transfer.
    async fn send_request_with_transfer(
        &self,
        op: &str,
        args: &JsValue,
        transfer: &js_sys::Array,
    ) -> VfsResult<JsValue> {
        let id = next_request_id();

        // Create request object
        let request = js_sys::Object::new();
        js_sys::Reflect::set(&request, &JsValue::from_str("id"), &JsValue::from(id))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set request id: {:?}", e),
            )))?;
        js_sys::Reflect::set(&request, &JsValue::from_str("op"), &JsValue::from_str(op))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set request op: {:?}", e),
            )))?;
        js_sys::Reflect::set(&request, &JsValue::from_str("args"), args)
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set request args: {:?}", e),
            )))?;

        // Create promise for response
        let pending = self.pending.clone();
        let promise = js_sys::Promise::new(&mut |resolve, _reject| {
            pending.borrow_mut().insert(id, resolve);
        });

        // Post message with transfer
        self.worker
            .post_message_with_transfer(&request, transfer)
            .map_err(|e| {
                VfsError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to post message to worker: {:?}", e),
                ))
            })?;

        // Wait for response
        let result = JsFuture::from(promise).await.map_err(|e| {
            VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Worker request failed: {:?}", e),
            ))
        })?;

        // Check for error
        let ok = js_sys::Reflect::get(&result, &JsValue::from_str("ok"))
            .map(|v| v.as_bool().unwrap_or(false))
            .unwrap_or(false);

        if !ok {
            let error = js_sys::Reflect::get(&result, &JsValue::from_str("error"))
                .unwrap_or(JsValue::NULL);
            let name = js_sys::Reflect::get(&error, &JsValue::from_str("name"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_else(|| "Error".into());
            let message = js_sys::Reflect::get(&error, &JsValue::from_str("message"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_else(|| "Unknown error".into());

            return Err(Self::map_js_error(&name, &message));
        }

        Ok(result)
    }

    /// Map JavaScript error names to VfsError variants.
    fn map_js_error(name: &str, message: &str) -> VfsError {
        match name {
            "NotFoundError" => VfsError::NotFound(message.to_string()),
            "NotAllowedError" | "SecurityError" => {
                VfsError::PermissionDenied(message.to_string())
            }
            "QuotaExceededError" => VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Quota exceeded: {}", message),
            )),
            "TypeMismatchError" => VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                message.to_string(),
            )),
            _ => VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("{}: {}", name, message),
            )),
        }
    }
}

#[async_trait(?Send)]
impl Vfs for WorkerOpfsVfs {
    async fn read(&self, path: &str) -> VfsResult<Vec<u8>> {
        let args = js_sys::Object::new();
        js_sys::Reflect::set(&args, &JsValue::from_str("path"), &JsValue::from_str(path))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set path: {:?}", e),
            )))?;

        let result = self.send_request("read", &args).await?;

        // Extract data from response
        let data = js_sys::Reflect::get(&result, &JsValue::from_str("data"))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get data from response: {:?}", e),
            )))?;

        let uint8_array = js_sys::Uint8Array::new(&data);
        Ok(uint8_array.to_vec())
    }

    async fn read_to_string(&self, path: &str) -> VfsResult<String> {
        let bytes = self.read(path).await?;
        String::from_utf8(bytes).map_err(|e| VfsError::SerializationError(e.to_string()))
    }

    async fn write(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        let args = js_sys::Object::new();
        js_sys::Reflect::set(&args, &JsValue::from_str("path"), &JsValue::from_str(path))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set path: {:?}", e),
            )))?;

        // Create Uint8Array and transfer buffer for zero-copy
        let data = js_sys::Uint8Array::new_with_length(contents.len() as u32);
        data.copy_from(contents);

        js_sys::Reflect::set(&args, &JsValue::from_str("data"), &data)
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set data: {:?}", e),
            )))?;

        // Transfer the buffer for zero-copy
        let transfer = js_sys::Array::new();
        transfer.push(&data.buffer());

        self.send_request_with_transfer("write", &args, &transfer).await?;
        Ok(())
    }

    async fn append(&self, path: &str, contents: &[u8]) -> VfsResult<()> {
        let args = js_sys::Object::new();
        js_sys::Reflect::set(&args, &JsValue::from_str("path"), &JsValue::from_str(path))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set path: {:?}", e),
            )))?;

        let data = js_sys::Uint8Array::new_with_length(contents.len() as u32);
        data.copy_from(contents);

        js_sys::Reflect::set(&args, &JsValue::from_str("data"), &data)
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set data: {:?}", e),
            )))?;

        let transfer = js_sys::Array::new();
        transfer.push(&data.buffer());

        self.send_request_with_transfer("append", &args, &transfer).await?;
        Ok(())
    }

    async fn exists(&self, path: &str) -> VfsResult<bool> {
        let args = js_sys::Object::new();
        js_sys::Reflect::set(&args, &JsValue::from_str("path"), &JsValue::from_str(path))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set path: {:?}", e),
            )))?;

        let result = self.send_request("exists", &args).await?;

        let exists = js_sys::Reflect::get(&result, &JsValue::from_str("exists"))
            .map(|v| v.as_bool().unwrap_or(false))
            .unwrap_or(false);

        Ok(exists)
    }

    async fn remove(&self, path: &str) -> VfsResult<()> {
        let args = js_sys::Object::new();
        js_sys::Reflect::set(&args, &JsValue::from_str("path"), &JsValue::from_str(path))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set path: {:?}", e),
            )))?;

        self.send_request("remove", &args).await?;
        Ok(())
    }

    async fn create_dir_all(&self, path: &str) -> VfsResult<()> {
        let args = js_sys::Object::new();
        js_sys::Reflect::set(&args, &JsValue::from_str("path"), &JsValue::from_str(path))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set path: {:?}", e),
            )))?;

        self.send_request("createDirAll", &args).await?;
        Ok(())
    }

    async fn rename(&self, from: &str, to: &str) -> VfsResult<()> {
        let args = js_sys::Object::new();
        js_sys::Reflect::set(&args, &JsValue::from_str("from"), &JsValue::from_str(from))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set from: {:?}", e),
            )))?;
        js_sys::Reflect::set(&args, &JsValue::from_str("to"), &JsValue::from_str(to))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set to: {:?}", e),
            )))?;

        self.send_request("rename", &args).await?;
        Ok(())
    }

    async fn list_dir(&self, path: &str) -> VfsResult<Vec<DirEntry>> {
        let args = js_sys::Object::new();
        js_sys::Reflect::set(&args, &JsValue::from_str("path"), &JsValue::from_str(path))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to set path: {:?}", e),
            )))?;

        let result = self.send_request("listDir", &args).await?;

        let entries_js = js_sys::Reflect::get(&result, &JsValue::from_str("entries"))
            .map_err(|e| VfsError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to get entries: {:?}", e),
            )))?;

        let entries_array: js_sys::Array = entries_js.unchecked_into();
        let mut entries = Vec::with_capacity(entries_array.length() as usize);

        for i in 0..entries_array.length() {
            let entry = entries_array.get(i);
            let name = js_sys::Reflect::get(&entry, &JsValue::from_str("name"))
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default();
            let is_directory = js_sys::Reflect::get(&entry, &JsValue::from_str("isDirectory"))
                .map(|v| v.as_bool().unwrap_or(false))
                .unwrap_or(false);

            entries.push(DirEntry { name, is_directory });
        }

        Ok(entries)
    }
}
