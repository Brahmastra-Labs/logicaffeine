use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use tower_lsp::lsp_types::{SemanticToken, Url};

use crate::document::DocumentState;

/// Global server state: the latest analyzed snapshot of every open document.
///
/// Snapshots are immutable `Arc`s: handlers clone the `Arc` out and drop the
/// map guard immediately, so a guard can never be held across an `.await`.
/// Text mutation and re-analysis live in the [`crate::scheduler`]; this map
/// only ever swaps in completed snapshots.
pub struct ServerState {
    documents: DashMap<Url, Arc<DocumentState>>,
    /// The last semantic-token emission per document, keyed for delta requests.
    semantic_cache: DashMap<Url, (String, Arc<Vec<SemanticToken>>)>,
    next_result_id: AtomicU64,
}

impl ServerState {
    pub fn new() -> Self {
        ServerState {
            documents: DashMap::new(),
            semantic_cache: DashMap::new(),
            next_result_id: AtomicU64::new(1),
        }
    }

    /// The latest snapshot for a document, if it is open and analyzed.
    /// Every open document's latest snapshot — cross-file features consult
    /// LIVE buffers before the disk-backed workspace index.
    pub fn open_documents(&self) -> Vec<(Url, Arc<DocumentState>)> {
        self.documents
            .iter()
            .map(|entry| (entry.key().clone(), Arc::clone(entry.value())))
            .collect()
    }

    pub fn snapshot(&self, uri: &Url) -> Option<Arc<DocumentState>> {
        self.documents.get(uri).map(|entry| Arc::clone(&entry))
    }

    /// Swap in a freshly analyzed snapshot.
    pub fn install_snapshot(&self, uri: Url, document: DocumentState) -> Arc<DocumentState> {
        let snapshot = Arc::new(document);
        self.documents.insert(uri, Arc::clone(&snapshot));
        snapshot
    }

    /// Remember a semantic-token emission and mint its result id.
    pub fn cache_semantic_tokens(&self, uri: Url, data: Vec<SemanticToken>) -> String {
        let id = self
            .next_result_id
            .fetch_add(1, Ordering::Relaxed)
            .to_string();
        self.semantic_cache.insert(uri, (id.clone(), Arc::new(data)));
        id
    }

    /// The previous emission for a document, if any.
    pub fn cached_semantic_tokens(&self, uri: &Url) -> Option<(String, Arc<Vec<SemanticToken>>)> {
        self.semantic_cache
            .get(uri)
            .map(|entry| (entry.0.clone(), Arc::clone(&entry.1)))
    }

    pub fn close_document(&self, uri: &Url) {
        self.documents.remove(uri);
        self.semantic_cache.remove(uri);
    }
}
