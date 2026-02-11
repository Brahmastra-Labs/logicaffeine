use dashmap::DashMap;
use tower_lsp::lsp_types::Url;

use crate::document::DocumentState;

/// Global server state, shared across all requests.
///
/// Uses `DashMap` for concurrent access without external locking.
pub struct ServerState {
    pub documents: DashMap<Url, DocumentState>,
}

impl ServerState {
    pub fn new() -> Self {
        ServerState {
            documents: DashMap::new(),
        }
    }

    pub fn open_document(&self, uri: Url, source: String, version: i32) {
        let doc = DocumentState::with_uri(source, version, Some(&uri));
        self.documents.insert(uri, doc);
    }

    pub fn update_document(&self, uri: &Url, source: String, version: i32) {
        if let Some(mut doc) = self.documents.get_mut(uri) {
            doc.update_with_uri(source, version, Some(uri));
        }
    }

    pub fn close_document(&self, uri: &Url) {
        self.documents.remove(uri);
    }
}
