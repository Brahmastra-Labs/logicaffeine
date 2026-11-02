use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use lsp_types::{TextDocumentContentChangeEvent, Url};

use crate::document::apply_content_change;

/// How long after the last keystroke analysis runs. A typing burst coalesces
/// into one pass over the final text; a single edit still feels instant
/// because publish follows the window immediately.
pub const DEBOUNCE: Duration = Duration::from_millis(150);

/// The live (pre-analysis) text of every open document, with a generation
/// counter per document.
///
/// The generation guard is the cancellation model: every edit bumps the
/// generation, and an analysis pass only installs+publishes its result if the
/// generation it captured is still current when it finishes. Stale results
/// are dropped on the floor — no locks held, nothing blocks.
pub struct Scheduler {
    entries: DashMap<Url, DocEntry>,
}

struct DocEntry {
    generation: Arc<AtomicU64>,
    text: String,
    version: i32,
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler {
            entries: DashMap::new(),
        }
    }

    /// Register a newly opened document. Returns its starting generation.
    pub fn open(&self, uri: Url, text: String, version: i32) -> u64 {
        let entry = DocEntry {
            generation: Arc::new(AtomicU64::new(0)),
            text,
            version,
        };
        self.entries.insert(uri, entry);
        0
    }

    /// Apply LSP content changes in order and bump the generation.
    /// Returns the new generation, or `None` for an unopened document.
    pub fn apply_changes(
        &self,
        uri: &Url,
        changes: Vec<TextDocumentContentChangeEvent>,
        version: i32,
    ) -> Option<u64> {
        let mut entry = self.entries.get_mut(uri)?;
        for change in changes {
            apply_content_change(&mut entry.text, change.range, &change.text);
        }
        entry.version = version;
        Some(entry.generation.fetch_add(1, Ordering::SeqCst) + 1)
    }

    /// Snapshot the current text/version if `generation` is still current.
    pub fn current_if(&self, uri: &Url, generation: u64) -> Option<(String, i32)> {
        let entry = self.entries.get(uri)?;
        if entry.generation.load(Ordering::SeqCst) != generation {
            return None;
        }
        Some((entry.text.clone(), entry.version))
    }

    /// The live text and version of an open document, regardless of
    /// generation — what a save should check.
    pub fn current_text(&self, uri: &Url) -> Option<(String, i32)> {
        let entry = self.entries.get(uri)?;
        Some((entry.text.clone(), entry.version))
    }

    pub fn is_current(&self, uri: &Url, generation: u64) -> bool {
        self.entries
            .get(uri)
            .map(|entry| entry.generation.load(Ordering::SeqCst) == generation)
            .unwrap_or(false)
    }

    /// Every open document, for whole-server sweeps (config changes).
    pub fn open_uris(&self) -> Vec<Url> {
        self.entries.iter().map(|e| e.key().clone()).collect()
    }

    pub fn close(&self, uri: &Url) {
        self.entries.remove(uri);
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/scheduler/tests.rs"]
mod tests;
