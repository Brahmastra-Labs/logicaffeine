//! The rustc flycheck: on save, compile the document through the AOT
//! backend's mapped codegen and run `cargo check` over it, translating every
//! finding back to English with real user-source spans — the borrow checker,
//! speaking LOGOS.
//!
//! Never on the interactive path: saves trigger it, a per-document
//! generation guard drops stale results (a newer save always wins), edits
//! clear findings outright (their positions would lie), and a machine
//! without cargo degrades to interactive-only diagnostics.

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use dashmap::DashMap;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Range, Url};

use logicaffeine_language::token::Span;

use crate::line_index::LineIndex;

/// The diagnostic source label — distinct from interactive `logicaffeine`
/// diagnostics so editors and users can tell the two engines apart.
pub const FLYCHECK_SOURCE: &str = "logicaffeine (rustc)";

/// One translated rustc finding.
pub struct FlycheckFinding {
    pub message: String,
    pub suggestion: Option<String>,
    /// User-source span from the mapped codegen; `None` = whole file.
    pub span: Option<Span>,
}

/// The check engine seam. The real runner shells out to cargo; tests inject
/// mocks so the server's merge/staleness/dedup behavior is provable without
/// a toolchain.
pub trait FlycheckRunner: Send + Sync {
    /// `None` = the toolchain is unavailable (degrade silently);
    /// `Some(findings)` = the check ran.
    fn check(&self, source: &str, workspace_key: &str) -> Option<Vec<FlycheckFinding>>;
}

/// Per-document flycheck state: save generations and the last published
/// findings (already converted to diagnostics against the checked text).
pub struct Flycheck {
    runner: Box<dyn FlycheckRunner>,
    generations: DashMap<Url, AtomicU64>,
    results: DashMap<Url, Vec<Diagnostic>>,
}

impl Flycheck {
    pub fn new(runner: Box<dyn FlycheckRunner>) -> Self {
        Flycheck {
            runner,
            generations: DashMap::new(),
            results: DashMap::new(),
        }
    }

    /// A save happened: invalidate any in-flight run and return the new
    /// generation this one must still hold when it finishes.
    pub fn begin_save(&self, uri: &Url) -> u64 {
        self.generations
            .entry(uri.clone())
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::SeqCst)
            + 1
    }

    pub fn is_current(&self, uri: &Url, generation: u64) -> bool {
        self.generations
            .get(uri)
            .map(|g| g.load(Ordering::SeqCst) == generation)
            .unwrap_or(false)
    }

    /// Edits invalidate findings outright — their positions would lie.
    pub fn clear(&self, uri: &Url) {
        self.results.remove(uri);
        // Bump so an in-flight save result from before the edit is dropped.
        if let Some(generation) = self.generations.get(uri) {
            generation.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn forget(&self, uri: &Url) {
        self.results.remove(uri);
        self.generations.remove(uri);
    }

    /// The last completed run's diagnostics, for merging into any publish.
    pub fn diagnostics_for(&self, uri: &Url) -> Vec<Diagnostic> {
        self.results
            .get(uri)
            .map(|entry| entry.clone())
            .unwrap_or_default()
    }

    /// Run the checker (blocking — call inside `spawn_blocking`).
    pub fn run(&self, source: &str, workspace_key: &str) -> Option<Vec<FlycheckFinding>> {
        self.runner.check(source, workspace_key)
    }

    /// Store a completed run's findings if its generation still holds.
    /// Returns the stored diagnostics, or `None` when the run went stale.
    pub fn complete(
        &self,
        uri: &Url,
        generation: u64,
        findings: Vec<FlycheckFinding>,
        checked_text: &str,
        interactive: &[Diagnostic],
    ) -> Option<Vec<Diagnostic>> {
        if !self.is_current(uri, generation) {
            return None;
        }
        let diagnostics = to_diagnostics(findings, checked_text, interactive);
        self.results.insert(uri.clone(), diagnostics.clone());
        Some(diagnostics)
    }
}

/// Convert findings to LSP diagnostics against the text that was checked,
/// dropping any that overlap an interactive ERROR — rustc's value is what
/// the local checkers MISS, not an echo of what they already said.
fn to_diagnostics(
    findings: Vec<FlycheckFinding>,
    checked_text: &str,
    interactive: &[Diagnostic],
) -> Vec<Diagnostic> {
    let line_index = LineIndex::new(checked_text);
    findings
        .into_iter()
        .map(|finding| {
            let range = match finding.span {
                Some(span) => Range {
                    start: line_index.position(span.start),
                    end: line_index.position(span.end),
                },
                None => Range::default(),
            };
            let mut message = finding.message;
            if let Some(suggestion) = finding.suggestion {
                message.push_str("\n\n");
                message.push_str(&suggestion);
            }
            Diagnostic {
                range,
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some(FLYCHECK_SOURCE.to_string()),
                message,
                ..Default::default()
            }
        })
        .filter(|diagnostic| {
            !interactive.iter().any(|existing| {
                existing.severity == Some(DiagnosticSeverity::ERROR)
                    && ranges_intersect(&existing.range, &diagnostic.range)
            })
        })
        .collect()
}

fn ranges_intersect(a: &Range, b: &Range) -> bool {
    !(a.end < b.start || b.end < a.start)
}

// ---------------------------------------------------------------------------
// The real runner
// ---------------------------------------------------------------------------

/// Shells out to `cargo check` through
/// [`logicaffeine_compile::compile::rustc_check`], one persistent cache
/// directory per workspace (warm incremental runs), single-flight per
/// workspace so two saves never race one cargo target dir.
pub struct CargoFlycheck {
    locks: DashMap<String, std::sync::Arc<Mutex<()>>>,
}

impl CargoFlycheck {
    pub fn new() -> Self {
        CargoFlycheck {
            locks: DashMap::new(),
        }
    }
}

impl Default for CargoFlycheck {
    fn default() -> Self {
        Self::new()
    }
}

fn cargo_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        std::process::Command::new("cargo")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

fn cache_dir_for(workspace_key: &str) -> std::path::PathBuf {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    workspace_key.hash(&mut hasher);
    std::env::temp_dir().join(format!("logicaffeine-flycheck-{:016x}", hasher.finish()))
}

impl FlycheckRunner for CargoFlycheck {
    fn check(&self, source: &str, workspace_key: &str) -> Option<Vec<FlycheckFinding>> {
        if !cargo_available() {
            return None;
        }
        let lock = self
            .locks
            .entry(workspace_key.to_string())
            .or_insert_with(|| std::sync::Arc::new(Mutex::new(())))
            .clone();
        let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        let dir = cache_dir_for(workspace_key);
        match logicaffeine_compile::compile::rustc_check(source, &dir) {
            Ok(errors) => Some(
                errors
                    .into_iter()
                    .map(|e| FlycheckFinding {
                        message: format!("{}\n\n{}", e.title, e.explanation),
                        suggestion: e.suggestion,
                        span: e.logos_span,
                    })
                    .collect(),
            ),
            // The interactive pipeline already reports parse problems.
            Err(logicaffeine_compile::compile::CompileError::Parse(_)) => Some(Vec::new()),
            // Generated code that fails rustc WITHOUT a LOGOS translation is
            // a compiler bug worth surfacing, not hiding.
            Err(logicaffeine_compile::compile::CompileError::Build(stderr)) => {
                let first_error = stderr
                    .lines()
                    .find(|l| l.contains("error"))
                    .unwrap_or("cargo check failed")
                    .to_string();
                Some(vec![FlycheckFinding {
                    message: format!(
                        "The generated Rust failed to compile — likely a LOGOS compiler bug, \
                        not an error in your program.\n\n{first_error}"
                    ),
                    suggestion: None,
                    span: None,
                }])
            }
            Err(other) => {
                log::warn!("flycheck could not run: {other:?}");
                None
            }
        }
    }
}
