//! End-to-end test harness for the LOGOS language server.
//!
//! Drives the real `tower_lsp::Server` loop over in-memory duplex pipes,
//! speaking the actual LSP wire protocol (`Content-Length` framing + JSON-RPC).
//! Everything a real editor exercises — message framing, capability
//! negotiation, request/response correlation, and server-initiated
//! `textDocument/publishDiagnostics` — is exercised here too.

pub mod error_kinds;
pub mod quickguide;

use std::collections::VecDeque;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};
use tower_lsp::lsp_types::notification::Notification;
use tower_lsp::lsp_types::request::Request;
use tower_lsp::lsp_types::*;
use tower_lsp::{LspService, Server};

use logicaffeine_lsp::server::LogicAffeineServer;

/// Hard ceiling on any single read; a hung server fails loudly, never silently.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Harness {
    writer: WriteHalf<DuplexStream>,
    reader: ReadHalf<DuplexStream>,
    read_buf: Vec<u8>,
    /// Server→client messages received while waiting for something else.
    pending: VecDeque<Value>,
    next_id: i64,
    doc_counter: u32,
    pub init: InitializeResult,
}

impl Harness {
    /// Boot with a workspace folder, wait for background indexing readiness
    /// implicitly via the first request round trip.
    pub async fn start_with_workspace(root: &std::path::Path) -> Self {
        let uri = Url::from_directory_path(root.canonicalize().unwrap()).unwrap();
        Self::start_inner(Some(vec![WorkspaceFolder {
            name: "fixture".to_string(),
            uri,
        }]))
        .await
    }

    /// Boot the server over duplex pipes and complete the
    /// `initialize`/`initialized` handshake.
    pub async fn start() -> Self {
        Self::start_inner(None).await
    }

    /// Boot with a custom-constructed service (e.g. a mock flycheck runner).
    pub async fn start_with_service(
        make: impl FnOnce() -> (LspService<LogicAffeineServer>, tower_lsp::ClientSocket),
    ) -> Self {
        Self::boot(make(), None).await
    }

    async fn start_inner(workspace_folders: Option<Vec<WorkspaceFolder>>) -> Self {
        Self::boot(LspService::new(LogicAffeineServer::new), workspace_folders).await
    }

    async fn boot(
        (service, socket): (LspService<LogicAffeineServer>, tower_lsp::ClientSocket),
        workspace_folders: Option<Vec<WorkspaceFolder>>,
    ) -> Self {
        let (client_side, server_side) = tokio::io::duplex(1024 * 1024);
        let (server_read, server_write) = tokio::io::split(server_side);
        let (client_read, client_write) = tokio::io::split(client_side);
        tokio::spawn(async move {
            Server::new(server_read, server_write, socket)
                .serve(service)
                .await;
        });

        let mut harness = Harness {
            writer: client_write,
            reader: client_read,
            read_buf: Vec::new(),
            pending: VecDeque::new(),
            next_id: 0,
            doc_counter: 0,
            init: InitializeResult::default(),
        };

        let init: InitializeResult = harness
            .request::<request::Initialize>(InitializeParams {
                workspace_folders,
                ..InitializeParams::default()
            })
            .await;
        harness
            .notify::<notification::Initialized>(InitializedParams {})
            .await;
        harness.init = init;
        harness
    }

    /// Send a typed request and await its typed response.
    pub async fn request<R: Request>(&mut self, params: R::Params) -> R::Result {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": R::METHOD,
            "params": params,
        });
        self.send(&msg).await;

        loop {
            let msg = self.read_message().await;
            if msg.get("id").and_then(Value::as_i64) == Some(id) && msg.get("method").is_none() {
                if let Some(err) = msg.get("error") {
                    panic!("server returned error for {}: {err}", R::METHOD);
                }
                let result = msg.get("result").cloned().unwrap_or(Value::Null);
                return serde_json::from_value(result).unwrap_or_else(|e| {
                    panic!("failed to decode {} response: {e}", R::METHOD)
                });
            }
            self.pending.push_back(msg);
        }
    }

    /// Send a typed notification.
    pub async fn notify<N: Notification>(&mut self, params: N::Params) {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": N::METHOD,
            "params": params,
        });
        self.send(&msg).await;
    }

    /// Open a document with a fresh URI and return that URI.
    pub async fn open(&mut self, text: &str) -> Url {
        self.doc_counter += 1;
        let uri = Url::parse(&format!("file:///harness/doc{}.lg", self.doc_counter)).unwrap();
        self.open_at(uri.clone(), text).await;
        uri
    }

    /// Open a document under an explicit URI — how an editor opens a REAL
    /// workspace file (cross-file features key on the file's identity).
    pub async fn open_at(&mut self, uri: Url, text: &str) {
        self.notify::<notification::DidOpenTextDocument>(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri,
                language_id: "logicaffeine".to_string(),
                version: 1,
                text: text.to_string(),
            },
        })
        .await;
    }

    /// Replace the full text of an open document (FULL sync).
    pub async fn change(&mut self, uri: &Url, text: &str, version: i32) {
        self.notify::<notification::DidChangeTextDocument>(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: text.to_string(),
            }],
        })
        .await;
    }

    /// Apply an incremental range edit to an open document.
    pub async fn change_range(&mut self, uri: &Url, range: Range, text: &str, version: i32) {
        self.notify::<notification::DidChangeTextDocument>(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: Some(range),
                range_length: None,
                text: text.to_string(),
            }],
        })
        .await;
    }

    /// Save a document (no text payload — the server keeps the live text).
    pub async fn save(&mut self, uri: &Url) {
        self.notify::<notification::DidSaveTextDocument>(DidSaveTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            text: None,
        })
        .await;
    }

    /// Close a document.
    pub async fn close(&mut self, uri: &Url) {
        self.notify::<notification::DidCloseTextDocument>(DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
        })
        .await;
    }

    /// Remove and return every ALREADY-RECEIVED publish for `uri` without
    /// waiting — the assertion surface for "nothing (more) was published".
    pub fn drain_pending_diagnostics(&mut self, uri: &Url) -> Vec<PublishDiagnosticsParams> {
        let mut drained = Vec::new();
        while let Some(ix) = self.pending.iter().position(|m| is_publish_for(m, uri)) {
            let msg = self.pending.remove(ix).unwrap();
            drained.push(decode_publish(&msg));
        }
        drained
    }

    /// Await the next `textDocument/publishDiagnostics` for `uri`.
    pub async fn recv_diagnostics(&mut self, uri: &Url) -> PublishDiagnosticsParams {
        if let Some(ix) = self.pending.iter().position(|m| is_publish_for(m, uri)) {
            let msg = self.pending.remove(ix).unwrap();
            return decode_publish(&msg);
        }
        loop {
            let msg = self.read_message().await;
            if is_publish_for(&msg, uri) {
                return decode_publish(&msg);
            }
            self.pending.push_back(msg);
        }
    }

    async fn send(&mut self, msg: &Value) {
        let body = serde_json::to_string(msg).unwrap();
        let framed = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        self.writer.write_all(framed.as_bytes()).await.unwrap();
        self.writer.flush().await.unwrap();
    }

    /// Read one framed JSON-RPC message, failing loudly on timeout.
    async fn read_message(&mut self) -> Value {
        tokio::time::timeout(READ_TIMEOUT, self.read_message_inner())
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "timed out waiting for a server message; pending: {:?}",
                    self.pending
                )
            })
    }

    async fn read_message_inner(&mut self) -> Value {
        loop {
            if let Some(msg) = self.try_extract_message() {
                return msg;
            }
            let mut chunk = [0u8; 4096];
            let n = self.reader.read(&mut chunk).await.unwrap();
            assert!(n > 0, "server closed the connection unexpectedly");
            self.read_buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// Pull one complete `Content-Length`-framed message out of the buffer.
    fn try_extract_message(&mut self) -> Option<Value> {
        let header_end = find_subsequence(&self.read_buf, b"\r\n\r\n")?;
        let headers = std::str::from_utf8(&self.read_buf[..header_end]).unwrap();
        let content_length: usize = headers
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length:"))
            .expect("missing Content-Length header")
            .trim()
            .parse()
            .expect("malformed Content-Length");

        let body_start = header_end + 4;
        if self.read_buf.len() < body_start + content_length {
            return None;
        }
        let body = &self.read_buf[body_start..body_start + content_length];
        let msg = serde_json::from_slice(body).expect("malformed JSON-RPC body");
        self.read_buf.drain(..body_start + content_length);
        Some(msg)
    }
}

fn is_publish_for(msg: &Value, uri: &Url) -> bool {
    msg.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics")
        && msg
            .pointer("/params/uri")
            .and_then(Value::as_str)
            .map(|u| u == uri.as_str())
            .unwrap_or(false)
}

fn decode_publish(msg: &Value) -> PublishDiagnosticsParams {
    serde_json::from_value(msg.get("params").cloned().unwrap())
        .expect("malformed publishDiagnostics params")
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
